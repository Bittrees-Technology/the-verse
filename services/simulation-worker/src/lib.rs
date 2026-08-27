// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{
        HeaderValue, Method, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{Html, IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{error, info, warn};
#[cfg(test)]
use verse_protocol::ProjectedMotionSnapshot;
use verse_protocol::{
    CELESTIAL_REGISTRY_SCHEMA_VERSION, CelestialRegistrySnapshot, ClientAuthentication,
    ClientMessage, INTEREST_SCHEMA_VERSION, IntentReceipt, InterestSnapshot, MotionSnapshot,
    PROJECTION_SCHEMA_VERSION, PROTOCOL_VERSION, ProjectedWorldSnapshot, ServerMessage,
    SessionRole, UNIVERSE_MANIFEST_SCHEMA_VERSION, UniverseManifestSnapshot, WorldSnapshot,
};
use verse_simulation::{
    AdvanceImpact, EVENT_SCHEMA_VERSION, IntentError, InterestEntityIdentity,
    InterestProjectionState, ProjectedInterestFrame, ProjectionError, ProjectionSource, Runtime,
    RuntimeError, WORLD_SCHEMA_VERSION, WorldState, registry_snapshot, universe_manifest,
};

const COMMAND_CENTER_HTML: &str = include_str!("../../../apps/web-command-center/index.html");
const COMMAND_CENTER_JS: &str = include_str!("../../../apps/web-command-center/app.js");
const COMMAND_CENTER_CSS: &str = include_str!("../../../apps/web-command-center/styles.css");
const VERIFIER_WORKER_JS: &str =
    include_str!("../../../apps/web-command-center/verifier-worker.js");
const VERIFIER_WORKER_CORE_JS: &str =
    include_str!("../../../apps/web-command-center/verifier-worker-core.js");
const REPLICATION_PERIOD: Duration = Duration::from_nanos(16_666_667);
const DYNAMIC_CACHE_CONTROL: &str = "no-store";
const MAX_CLIENT_NAME_BYTES: usize = 128;
const MAX_SERVER_MESSAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SERIALIZATION_TIME: Duration = Duration::from_millis(500);
const SERVER_WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const UNACKNOWLEDGED_FRAME_TIMEOUT: Duration = Duration::from_secs(30);
const RECOVERY_WINDOW: Duration = Duration::from_secs(10);
const MAX_RECOVERIES_PER_WINDOW: u8 = 4;
const MAX_CONCURRENT_CONNECTIONS: usize = 1_024;
const MAX_CONCURRENT_HTTP_PROJECTIONS: usize = 1;
const HTTP_PROJECTION_MIN_REFRESH: Duration = Duration::from_millis(250);
#[cfg(not(test))]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(test)]
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplicationKind {
    Structural,
    Motion,
}

#[derive(Debug, Clone, Default)]
struct ReplicationFeed {
    latest_structural_sequence: Option<u64>,
    latest_motion_sequence: Option<u64>,
}

impl ReplicationFeed {
    fn publish(&mut self, kind: ReplicationKind, sequence: u64) -> bool {
        match kind {
            ReplicationKind::Structural => {
                if self
                    .latest_structural_sequence
                    .is_some_and(|current| current >= sequence)
                {
                    return false;
                }
                self.latest_structural_sequence = Some(sequence);
                if self
                    .latest_motion_sequence
                    .is_some_and(|current| current <= sequence)
                {
                    self.latest_motion_sequence = None;
                }
                true
            }
            ReplicationKind::Motion => {
                if self
                    .latest_structural_sequence
                    .is_some_and(|current| current >= sequence)
                    || self
                        .latest_motion_sequence
                        .is_some_and(|current| current >= sequence)
                {
                    return false;
                }
                self.latest_motion_sequence = Some(sequence);
                true
            }
        }
    }

    fn next_after(&self, cursor: ReplicationCursor) -> Option<PendingReplication> {
        if let Some(sequence) = self.latest_structural_sequence
            && sequence > cursor.full_snapshot_sequence
        {
            // A cursor can be ahead of the retained structural marker after
            // receiving motion. Re-project the current complete state instead
            // of rolling the connection back or losing structural changes.
            return Some(PendingReplication::Structural);
        }
        self.latest_motion_sequence
            .filter(|sequence| *sequence > cursor.event_sequence)
            .map(|_| PendingReplication::Motion)
    }

    #[cfg(test)]
    fn retained_update_count(&self) -> usize {
        usize::from(self.latest_structural_sequence.is_some())
            + usize::from(self.latest_motion_sequence.is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReplicationCursor {
    event_sequence: u64,
    full_snapshot_sequence: u64,
}

impl ReplicationCursor {
    fn after_initial_snapshot(event_sequence: u64) -> Self {
        Self {
            event_sequence,
            full_snapshot_sequence: event_sequence,
        }
    }

    fn record(&mut self, message: &ServerMessage) {
        let sequence = replication_event_sequence(message)
            .expect("only state replication messages advance the cursor");
        debug_assert!(sequence >= self.event_sequence);
        self.event_sequence = self.event_sequence.max(sequence);
        if matches!(
            message,
            ServerMessage::InterestBaseline { .. }
                | ServerMessage::InterestDelta { .. }
                | ServerMessage::Snapshot { .. }
        ) {
            self.full_snapshot_sequence = self.full_snapshot_sequence.max(sequence);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PendingReplication {
    Structural,
    Motion,
}

fn replication_event_sequence(message: &ServerMessage) -> Option<u64> {
    match message {
        ServerMessage::InterestBaseline { baseline } => Some(baseline.event_sequence),
        ServerMessage::InterestDelta { delta } => Some(delta.event_sequence),
        ServerMessage::Snapshot { snapshot } => Some(snapshot.event_sequence),
        ServerMessage::MotionState { motion } => Some(motion.event_sequence),
        _ => None,
    }
}

fn replication_interest(message: &ServerMessage) -> Option<&InterestSnapshot> {
    match message {
        ServerMessage::InterestBaseline { baseline } => Some(&baseline.interest),
        ServerMessage::InterestDelta { delta } => Some(&delta.interest),
        ServerMessage::Snapshot { snapshot } => Some(&snapshot.interest),
        ServerMessage::MotionState { motion } => Some(&motion.interest),
        _ => None,
    }
}

#[derive(Debug)]
pub struct AppState {
    runtime: Mutex<Runtime>,
    updates: watch::Sender<ReplicationFeed>,
    connected_players: Mutex<BTreeSet<String>>,
    session_admission: Arc<Semaphore>,
    http_projection_admission: Arc<Semaphore>,
    public_world_cache: Mutex<Option<CachedPublicWorld>>,
    projection_revision: Mutex<Arc<ProjectionRevision>>,
    registry: CelestialRegistrySnapshot,
    universe_manifest: UniverseManifestSnapshot,
}

#[derive(Debug, Clone)]
struct CachedPublicWorld {
    event_sequence: u64,
    generated_at: Instant,
    encoded: Arc<str>,
}

#[derive(Debug)]
struct ProjectionRevision {
    world: WorldState,
    source: OnceLock<Result<Arc<ProjectionSource>, String>>,
}

impl ProjectionRevision {
    fn new(world: WorldState) -> Self {
        Self {
            world,
            source: OnceLock::new(),
        }
    }

    fn source(&self) -> Result<Arc<ProjectionSource>, ProjectionError> {
        self.source
            .get_or_init(|| {
                self.world
                    .projection_source()
                    .map(Arc::new)
                    .map_err(|source| source.to_string())
            })
            .as_ref()
            .cloned()
            .map_err(|source| ProjectionError::InvalidCanonicalSnapshot(source.clone()))
    }
}

impl AppState {
    pub fn new(runtime: Runtime) -> Arc<Self> {
        let world_seed = runtime.state().world_seed;
        let registry = registry_snapshot(world_seed)
            .expect("the runtime's validated celestial registry remains available");
        let universe_manifest =
            universe_manifest(world_seed, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
                .expect("the runtime's validated universe manifest remains available");
        let projection_revision = Arc::new(ProjectionRevision::new(runtime.state().clone()));
        let (updates, _) = watch::channel(ReplicationFeed::default());
        Arc::new(Self {
            runtime: Mutex::new(runtime),
            updates,
            connected_players: Mutex::new(BTreeSet::new()),
            session_admission: Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS)),
            http_projection_admission: Arc::new(Semaphore::new(MAX_CONCURRENT_HTTP_PROJECTIONS)),
            public_world_cache: Mutex::new(None),
            projection_revision: Mutex::new(projection_revision),
            registry,
            universe_manifest,
        })
    }

    pub fn snapshot(&self) -> WorldSnapshot {
        self.runtime.lock().snapshot()
    }

    pub fn motion_snapshot(&self) -> MotionSnapshot {
        self.runtime.lock().motion_snapshot()
    }

    fn projected_snapshot(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<ProjectedWorldSnapshot, ProjectionError> {
        self.runtime
            .lock()
            .state()
            .project_world_snapshot(actor_player_id)
    }

    #[cfg(test)]
    fn projected_motion_snapshot(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<ProjectedMotionSnapshot, ProjectionError> {
        self.runtime
            .lock()
            .state()
            .project_motion_snapshot(actor_player_id)
    }

    fn projected_interest_frame(
        &self,
        projection: &mut InterestProjectionState,
    ) -> Result<ProjectedInterestFrame, ProjectionError> {
        let revision = self.projection_revision.lock().clone();
        let source = revision.source()?;
        source.project_interest_frame(projection, &BTreeMap::<InterestEntityIdentity, _>::new())
    }

    fn bounded_public_world_json(&self) -> Result<Arc<str>, String> {
        let current_sequence = self.runtime.lock().state().event_sequence;
        if let Some(cached) = self.public_world_cache.lock().as_ref()
            && (cached.event_sequence == current_sequence
                || cached.generated_at.elapsed() < HTTP_PROJECTION_MIN_REFRESH)
        {
            return Ok(cached.encoded.clone());
        }

        let snapshot = self
            .projected_snapshot(None)
            .map_err(|source| source.to_string())?;
        let event_sequence = snapshot.event_sequence;
        let encoded = encode_bounded_json(&snapshot).map_err(|source| source.to_string())?;
        let encoded = Arc::<str>::from(encoded);
        *self.public_world_cache.lock() = Some(CachedPublicWorld {
            event_sequence,
            generated_at: Instant::now(),
            encoded: encoded.clone(),
        });
        Ok(encoded)
    }

    pub fn persist_snapshot(&self) -> Result<(), RuntimeError> {
        self.runtime.lock().persist_snapshot()
    }

    pub fn is_halted(&self) -> bool {
        self.runtime.lock().is_halted()
    }

    pub fn advance(&self, delta_millis: u16) -> Result<bool, RuntimeError> {
        let mut runtime = self.runtime.lock();
        let outcome = runtime.advance_with_outcome(delta_millis)?;
        let update_kind = match outcome.impact {
            AdvanceImpact::None => None,
            AdvanceImpact::Motion => Some(ReplicationKind::Motion),
            AdvanceImpact::Structural => Some(ReplicationKind::Structural),
        };
        if let Some(update_kind) = update_kind {
            self.publish_projection_revision(&runtime);
            self.publish_update(update_kind, runtime.state().event_sequence);
        }
        Ok(outcome.changed())
    }

    fn execute_as(
        &self,
        actor_player_id: &str,
        intent: &ClientMessage,
    ) -> Result<IntentReceipt, RuntimeError> {
        let mut runtime = self.runtime.lock();
        let before_event_sequence = runtime.state().event_sequence;
        let receipt = runtime.execute_as(actor_player_id, intent)?;
        if runtime.state().event_sequence == before_event_sequence {
            return Ok(receipt);
        }
        let update_kind = if matches!(intent, ClientMessage::SetPlayerControl { .. }) {
            ReplicationKind::Motion
        } else {
            ReplicationKind::Structural
        };
        // Keep mutation and publication in the same runtime critical section so
        // every subscriber observes structural and motion state in event order.
        self.publish_projection_revision(&runtime);
        self.publish_update(update_kind, runtime.state().event_sequence);
        Ok(receipt)
    }

    fn publish_projection_revision(&self, runtime: &Runtime) {
        *self.projection_revision.lock() =
            Arc::new(ProjectionRevision::new(runtime.state().clone()));
    }

    fn publish_update(&self, kind: ReplicationKind, event_sequence: u64) {
        self.updates
            .send_if_modified(|feed| feed.publish(kind, event_sequence));
    }

    fn claim_player(&self, player_id: &str) -> bool {
        self.connected_players.lock().insert(player_id.to_owned())
    }

    fn release_player(&self, player_id: &str) {
        self.connected_players.lock().remove(player_id);
    }

    fn registry_message(&self) -> ServerMessage {
        ServerMessage::Registry {
            registry: Box::new(self.registry.clone()),
            universe_manifest: Box::new(self.universe_manifest.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SessionBinding {
    Spectator,
    Player(String),
}

impl SessionBinding {
    fn role(&self) -> SessionRole {
        match self {
            Self::Spectator => SessionRole::Spectator,
            Self::Player(player_id) => SessionRole::Player {
                player_id: player_id.clone(),
            },
        }
    }

    fn player_id(&self) -> Option<&str> {
        match self {
            Self::Spectator => None,
            Self::Player(player_id) => Some(player_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InterestFrontier {
    session_epoch: String,
    interest_epoch: u64,
    baseline_id: String,
    delta_sequence: u64,
    view_hash: String,
}

#[derive(Debug, Clone)]
struct PendingInterestFrame {
    projection: InterestProjectionState,
    frontier: InterestFrontier,
    sent_at: tokio::time::Instant,
    recovery: bool,
}

#[derive(Debug)]
struct SessionTransport {
    acknowledged_projection: InterestProjectionState,
    acknowledged_frontier: Option<InterestFrontier>,
    pending: Option<PendingInterestFrame>,
    superseded_frontier: Option<InterestFrontier>,
    recovery_window_started_at: tokio::time::Instant,
    recoveries_in_window: u8,
}

impl SessionTransport {
    fn new(binding: &SessionBinding) -> Self {
        let session_epoch = uuid::Uuid::new_v4().to_string();
        let acknowledged_projection = match binding {
            SessionBinding::Spectator => {
                InterestProjectionState::public_origin_spectator(session_epoch)
            }
            SessionBinding::Player(player_id) => {
                InterestProjectionState::bound_player(session_epoch, player_id.clone())
            }
        };
        Self {
            acknowledged_projection,
            acknowledged_frontier: None,
            pending: None,
            superseded_frontier: None,
            recovery_window_started_at: tokio::time::Instant::now(),
            recoveries_in_window: 0,
        }
    }

    fn awaiting_acknowledgement(&self) -> bool {
        self.pending.is_some()
    }

    fn pending_timed_out(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.sent_at.elapsed() >= UNACKNOWLEDGED_FRAME_TIMEOUT)
    }

    fn recovery_is_pending(&self) -> bool {
        self.pending
            .as_ref()
            .is_some_and(|pending| pending.recovery)
    }

    fn permit_recovery(&mut self) -> bool {
        if self.recovery_window_started_at.elapsed() >= RECOVERY_WINDOW {
            self.recovery_window_started_at = tokio::time::Instant::now();
            self.recoveries_in_window = 0;
        }
        if self.recoveries_in_window >= MAX_RECOVERIES_PER_WINDOW {
            return false;
        }
        self.recoveries_in_window += 1;
        true
    }

    fn acknowledge(
        &mut self,
        session_epoch: &str,
        interest_epoch: u64,
        baseline_id: &str,
        delta_sequence: u64,
        view_hash: &str,
    ) -> bool {
        if let Some(pending) = self.pending.as_ref()
            && pending.frontier.matches_acknowledgement(
                session_epoch,
                interest_epoch,
                baseline_id,
                delta_sequence,
                view_hash,
            )
        {
            let pending = self.pending.take().expect("the matched frame exists");
            self.acknowledged_projection = pending.projection;
            self.acknowledged_frontier = Some(pending.frontier);
            self.superseded_frontier = None;
            return true;
        }
        self.acknowledged_frontier.as_ref().is_some_and(|frontier| {
            frontier.matches_acknowledgement(
                session_epoch,
                interest_epoch,
                baseline_id,
                delta_sequence,
                view_hash,
            )
        })
    }

    fn matches_superseded_acknowledgement(
        &self,
        session_epoch: &str,
        interest_epoch: u64,
        baseline_id: &str,
        delta_sequence: u64,
        view_hash: &str,
    ) -> bool {
        self.superseded_frontier.as_ref().is_some_and(|frontier| {
            frontier.matches_acknowledgement(
                session_epoch,
                interest_epoch,
                baseline_id,
                delta_sequence,
                view_hash,
            )
        })
    }

    fn stage(
        &mut self,
        state: &AppState,
        fresh_baseline: bool,
    ) -> Result<ServerMessage, ProjectionError> {
        if fresh_baseline {
            self.superseded_frontier = self
                .pending
                .as_ref()
                .map(|pending| pending.frontier.clone());
        }
        let mut candidate = if fresh_baseline {
            self.pending.as_ref().map_or_else(
                || self.acknowledged_projection.clone(),
                |pending| pending.projection.clone(),
            )
        } else {
            self.acknowledged_projection.clone()
        };
        if fresh_baseline {
            candidate.fresh_baseline()?;
        }
        let message = match state.projected_interest_frame(&mut candidate)? {
            ProjectedInterestFrame::Baseline(baseline) => ServerMessage::InterestBaseline {
                baseline: Box::new(baseline),
            },
            ProjectedInterestFrame::Delta(delta) => ServerMessage::InterestDelta {
                delta: Box::new(delta),
            },
        };
        let interest = replication_interest(&message)
            .expect("an interest frame always contains an interest frontier");
        self.pending = Some(PendingInterestFrame {
            projection: candidate,
            frontier: InterestFrontier::from_interest(interest),
            sent_at: tokio::time::Instant::now(),
            recovery: fresh_baseline,
        });
        Ok(message)
    }
}

impl InterestFrontier {
    fn from_interest(interest: &InterestSnapshot) -> Self {
        Self {
            session_epoch: interest.session_epoch.clone(),
            interest_epoch: interest.interest_epoch,
            baseline_id: interest.baseline_id.clone(),
            delta_sequence: interest.delta_sequence,
            view_hash: interest.view_hash.clone(),
        }
    }

    fn matches_acknowledgement(
        &self,
        session_epoch: &str,
        interest_epoch: u64,
        baseline_id: &str,
        delta_sequence: u64,
        view_hash: &str,
    ) -> bool {
        self.session_epoch == session_epoch
            && self.interest_epoch == interest_epoch
            && self.baseline_id == baseline_id
            && self.delta_sequence == delta_sequence
            && self.view_hash == view_hash
    }
}

#[derive(Debug, Serialize)]
struct StatusDocument {
    service: &'static str,
    protocol_version: u32,
    content_manifest_version: String,
    universe_id: String,
    cell_id: String,
    event_sequence: u64,
    simulation_tick: u64,
    fencing_token: u64,
    world_hash: String,
    conservation_valid: bool,
    authoritative_halted: bool,
}

pub fn router(state: Arc<AppState>) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(HeaderValue::from_static("http://localhost:3000"))
        .allow_methods([Method::GET])
        .allow_headers([]);
    Router::new()
        .route("/", get(command_center))
        .route("/app.js", get(command_center_javascript))
        .route("/styles.css", get(command_center_styles))
        .route("/verifier-worker.js", get(verifier_worker_javascript))
        .route(
            "/verifier-worker-core.js",
            get(verifier_worker_core_javascript),
        )
        .route(
            "/generated/verse_interest_verifier.js",
            get(generated_verifier_javascript),
        )
        .route(
            "/generated/verse_interest_verifier_bg.wasm",
            get(generated_verifier_wasm),
        )
        .route("/healthz", get(health))
        .route("/api/v1/status", get(status))
        .route("/api/v1/registry", get(registry))
        .route("/api/v1/world", get(world))
        .route("/ws", get(websocket_upgrade))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(false)),
        )
        .layer(cors)
        .with_state(state)
}

async fn command_center() -> Response {
    no_store(Html(COMMAND_CENTER_HTML).into_response())
}

async fn command_center_javascript() -> Response {
    no_store(
        (
            [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
            COMMAND_CENTER_JS,
        )
            .into_response(),
    )
}

async fn command_center_styles() -> Response {
    no_store(
        (
            [(CONTENT_TYPE, "text/css; charset=utf-8")],
            COMMAND_CENTER_CSS,
        )
            .into_response(),
    )
}

async fn verifier_worker_javascript() -> Response {
    no_store(
        (
            [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
            VERIFIER_WORKER_JS,
        )
            .into_response(),
    )
}

async fn verifier_worker_core_javascript() -> Response {
    no_store(
        (
            [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
            VERIFIER_WORKER_CORE_JS,
        )
            .into_response(),
    )
}

async fn generated_verifier_javascript() -> Response {
    generated_browser_asset(
        "verse_interest_verifier.js",
        "text/javascript; charset=utf-8",
    )
    .await
}

async fn generated_verifier_wasm() -> Response {
    generated_browser_asset("verse_interest_verifier_bg.wasm", "application/wasm").await
}

async fn generated_browser_asset(file_name: &str, content_type: &'static str) -> Response {
    let directory = std::env::var_os("VERSE_BROWSER_VERIFIER_ASSET_DIR").map_or_else(
        || {
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/web-command-center/generated")
        },
        std::path::PathBuf::from,
    );
    match tokio::fs::read(directory.join(file_name)).await {
        Ok(bytes) => no_store(([(CONTENT_TYPE, content_type)], bytes).into_response()),
        Err(_) => no_store(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                [(CONTENT_TYPE, content_type)],
                "browser verifier asset is unavailable; run tools/ci/build-browser-verifier.sh",
            )
                .into_response(),
        ),
    }
}

async fn health(State(state): State<Arc<AppState>>) -> StatusCode {
    if state.is_halted() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::NO_CONTENT
    }
}

fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        CACHE_CONTROL,
        HeaderValue::from_static(DYNAMIC_CACHE_CONTROL),
    );
    response
}

async fn status(State(state): State<Arc<AppState>>) -> Response {
    let snapshot = state.snapshot();
    no_store(
        Json(StatusDocument {
            service: "verse-simulation-worker",
            protocol_version: PROTOCOL_VERSION,
            content_manifest_version: snapshot.content_manifest_version,
            universe_id: snapshot.universe_id,
            cell_id: snapshot.cell_id,
            event_sequence: snapshot.event_sequence,
            simulation_tick: snapshot.simulation_tick,
            fencing_token: snapshot.fencing_token,
            world_hash: snapshot.world_hash,
            conservation_valid: snapshot.conservation.valid,
            authoritative_halted: state.is_halted(),
        })
        .into_response(),
    )
}

async fn world(State(state): State<Arc<AppState>>) -> Response {
    let Ok(_permit) = state.http_projection_admission.clone().try_acquire_owned() else {
        return no_store(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "public world projection capacity is currently full",
            )
                .into_response(),
        );
    };
    match state.bounded_public_world_json() {
        Ok(encoded) => no_store(
            (
                [(CONTENT_TYPE, "application/json; charset=utf-8")],
                encoded.as_ref().to_owned(),
            )
                .into_response(),
        ),
        Err(_) => no_store(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "public world projection is unavailable",
            )
                .into_response(),
        ),
    }
}

async fn registry(State(state): State<Arc<AppState>>) -> Response {
    no_store(Json(state.registry_message()).into_response())
}

async fn websocket_upgrade(
    upgrade: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    let Ok(permit) = state.session_admission.clone().try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "the bounded session capacity is currently full",
        )
            .into_response();
    };
    upgrade
        .max_message_size(8 * 1024 * 1024)
        .on_upgrade(move |socket| websocket_session(socket, state, permit))
}

async fn websocket_session(socket: WebSocket, state: Arc<AppState>, _permit: OwnedSemaphorePermit) {
    let (mut sender, mut receiver) = socket.split();
    let Some((client_name, binding)) = complete_handshake(&mut receiver, &mut sender, &state).await
    else {
        return;
    };
    info!(%client_name, role = ?binding, "client completed protocol handshake");

    // Subscribe before projecting so a mutation between projection and delivery
    // is retained as a canonical marker and cannot be missed by this session.
    let mut updates = state.updates.subscribe();
    let mut transport = SessionTransport::new(&binding);
    let Ok(initial_message) = transport.stage(&state, false) else {
        send_projection_failure(&mut sender).await;
        if let Some(player_id) = binding.player_id() {
            state.release_player(player_id);
        }
        return;
    };
    let initial_sequence = replication_event_sequence(&initial_message)
        .expect("the initial projected snapshot has an event sequence");

    if send_server_message(
        &mut sender,
        &ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            world_schema_version: state.universe_manifest.world_schema_version,
            event_schema_version: state.universe_manifest.event_schema_version,
            content_schema_version: state.universe_manifest.content_schema_version,
            content_manifest_version: state.universe_manifest.content_manifest_version.clone(),
            celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            universe_manifest_schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            server_name: "The Verse local universe".into(),
            session_role: binding.role(),
        },
    )
    .await
    .is_err()
    {
        if let Some(player_id) = binding.player_id() {
            state.release_player(player_id);
        }
        return;
    }

    if send_server_message(&mut sender, &state.registry_message())
        .await
        .is_err()
    {
        if let Some(player_id) = binding.player_id() {
            state.release_player(player_id);
        }
        return;
    }

    let mut replication_cursor = ReplicationCursor::after_initial_snapshot(initial_sequence);
    if send_server_message(&mut sender, &initial_message)
        .await
        .is_err()
    {
        if let Some(player_id) = binding.player_id() {
            state.release_player(player_id);
        }
        return;
    }
    let mut replication_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + REPLICATION_PERIOD,
        REPLICATION_PERIOD,
    );
    replication_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            client_message = receiver.next() => {
                match client_message {
                    Some(Ok(message)) => {
                        if !handle_client_message(
                            message,
                            &state,
                            &binding,
                            &mut sender,
                            &mut replication_cursor,
                            &mut transport,
                        ).await {
                            break;
                        }
                    }
                    Some(Err(source)) => {
                        warn!(%source, "websocket receive failed");
                        break;
                    }
                    None => break,
                }
            }
            _ = replication_interval.tick() => {
                if transport.pending_timed_out() {
                    if transport.recovery_is_pending() || !transport.permit_recovery() {
                        send_fatal_and_close(
                            &mut sender,
                            "interest_ack_timeout",
                            "the client did not acknowledge the recovery baseline in time",
                        ).await;
                        break;
                    }
                    let Ok(message) = transport.stage(&state, true) else {
                        send_projection_failure(&mut sender).await;
                        break;
                    };
                    if send_server_message(&mut sender, &message).await.is_err() {
                        break;
                    }
                    replication_cursor.record(&message);
                    continue;
                }
                if transport.awaiting_acknowledgement() {
                    continue;
                }
                let pending = updates.borrow_and_update().next_after(replication_cursor);
                let Some(_pending) = pending else {
                    continue;
                };
                let Ok(message) = transport.stage(&state, false) else {
                    send_projection_failure(&mut sender).await;
                    break;
                };
                if send_server_message(&mut sender, &message).await.is_err() {
                    break;
                }
                replication_cursor.record(&message);
            }
        }
    }

    if let Some(player_id) = binding.player_id() {
        state.release_player(player_id);
    }
}

#[cfg(test)]
fn projected_snapshot_message(
    state: &AppState,
    binding: &SessionBinding,
) -> Result<ServerMessage, ProjectionError> {
    state
        .projected_snapshot(binding.player_id())
        .map(|snapshot| ServerMessage::Snapshot {
            snapshot: Box::new(snapshot),
        })
}

#[cfg(test)]
fn projected_replication_message(
    state: &AppState,
    binding: &SessionBinding,
    pending: PendingReplication,
) -> Result<ServerMessage, ProjectionError> {
    match pending {
        PendingReplication::Structural => projected_snapshot_message(state, binding),
        PendingReplication::Motion => {
            state
                .projected_motion_snapshot(binding.player_id())
                .map(|motion| ServerMessage::MotionState {
                    motion: Box::new(motion),
                })
        }
    }
}

async fn complete_handshake(
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    state: &Arc<AppState>,
) -> Option<(String, SessionBinding)> {
    let deadline = tokio::time::Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let message = match tokio::time::timeout_at(deadline, receiver.next()).await {
            Err(_) => {
                send_fatal_and_close(
                    sender,
                    "protocol_handshake_timeout",
                    "send one compatible hello before the fixed handshake deadline",
                )
                .await;
                return None;
            }
            Ok(message) => match message {
                Some(Ok(message)) => message,
                Some(Err(source)) => {
                    warn!(%source, "websocket handshake receive failed");
                    return None;
                }
                None => return None,
            },
        };
        match message {
            Message::Text(text) => match serde_json::from_str::<ClientMessage>(&text) {
                Ok(ClientMessage::Hello {
                    protocol_version,
                    client_name,
                    authentication,
                }) if protocol_version == PROTOCOL_VERSION => {
                    if !valid_client_name(&client_name) {
                        send_fatal_and_close(
                            sender,
                            "invalid_client_name",
                            "client name must contain 1-128 printable ASCII bytes",
                        )
                        .await;
                        return None;
                    }
                    let binding = match authentication {
                        ClientAuthentication::Spectator => SessionBinding::Spectator,
                        ClientAuthentication::LocalDevelopment { player_id } => {
                            if !state
                                .snapshot()
                                .players
                                .iter()
                                .any(|player| player.player_id == player_id)
                            {
                                send_fatal_and_close(
                                    sender,
                                    "actor_not_present",
                                    "the requested local-development player is not present in this cell",
                                )
                                .await;
                                return None;
                            }
                            if !state.claim_player(&player_id) {
                                send_fatal_and_close(
                                    sender,
                                    "player_already_connected",
                                    "the requested player already has an active gameplay session",
                                )
                                .await;
                                return None;
                            }
                            SessionBinding::Player(player_id)
                        }
                    };
                    return Some((client_name, binding));
                }
                Ok(ClientMessage::Hello {
                    protocol_version, ..
                }) => {
                    send_fatal_and_close(
                        sender,
                        "protocol_version_mismatch",
                        format!(
                            "server requires protocol {PROTOCOL_VERSION}, client sent {protocol_version}"
                        ),
                    )
                    .await;
                    return None;
                }
                Ok(_) => {
                    send_fatal_and_close(
                        sender,
                        "protocol_handshake_required",
                        "send a compatible hello before requesting state or submitting intents",
                    )
                    .await;
                    return None;
                }
                Err(source) => {
                    send_fatal_and_close(sender, "invalid_handshake", source.to_string()).await;
                    return None;
                }
            },
            Message::Ping(payload) => {
                if !matches!(
                    tokio::time::timeout(SERVER_WRITE_TIMEOUT, sender.send(Message::Pong(payload)))
                        .await,
                    Ok(Ok(()))
                ) {
                    return None;
                }
            }
            Message::Pong(_) => {}
            Message::Close(_) => return None,
            Message::Binary(_) => {
                send_fatal_and_close(
                    sender,
                    "invalid_handshake",
                    "the protocol handshake must be a JSON text message",
                )
                .await;
                return None;
            }
        }
    }
}

fn valid_client_name(client_name: &str) -> bool {
    !client_name.is_empty()
        && client_name.len() <= MAX_CLIENT_NAME_BYTES
        && client_name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
}

async fn handle_client_message(
    message: Message,
    state: &Arc<AppState>,
    binding: &SessionBinding,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    replication_cursor: &mut ReplicationCursor,
    transport: &mut SessionTransport,
) -> bool {
    let Message::Text(text) = message else {
        return !matches!(message, Message::Close(_));
    };
    let parsed = match serde_json::from_str::<ClientMessage>(&text) {
        Ok(message) => message,
        Err(source) => {
            return send_server_message(
                sender,
                &ServerMessage::IntentRejected {
                    operation_sequence: None,
                    operation_id: None,
                    code: "invalid_json".into(),
                    message: source.to_string(),
                },
            )
            .await
            .is_ok();
        }
    };

    match parsed {
        ClientMessage::Hello { .. } => {
            send_fatal_and_close(
                sender,
                "protocol_handshake_already_complete",
                "a connection may complete the protocol handshake only once",
            )
            .await;
            false
        }
        ClientMessage::RequestSnapshot => {
            if transport.recovery_is_pending() {
                return true;
            }
            if !transport.permit_recovery() {
                send_fatal_and_close(
                    sender,
                    "interest_recovery_rate_limited",
                    "the client requested repeated interest recovery before convergence",
                )
                .await;
                return false;
            }
            let Ok(message) = transport.stage(state, true) else {
                send_projection_failure(sender).await;
                return false;
            };
            if send_server_message(sender, &message).await.is_ok() {
                replication_cursor.record(&message);
                true
            } else {
                false
            }
        }
        ClientMessage::AcknowledgeInterest {
            session_epoch,
            interest_epoch,
            baseline_id,
            delta_sequence,
            view_hash,
        } => {
            if transport.acknowledge(
                &session_epoch,
                interest_epoch,
                &baseline_id,
                delta_sequence,
                &view_hash,
            ) {
                return true;
            }
            if transport.recovery_is_pending()
                && transport.matches_superseded_acknowledgement(
                    &session_epoch,
                    interest_epoch,
                    &baseline_id,
                    delta_sequence,
                    &view_hash,
                )
            {
                return true;
            }
            if transport.recovery_is_pending() || !transport.permit_recovery() {
                send_fatal_and_close(
                    sender,
                    "interest_recovery_rate_limited",
                    "the client repeated an invalid interest acknowledgement before convergence",
                )
                .await;
                return false;
            }
            let Ok(message) = transport.stage(state, true) else {
                send_projection_failure(sender).await;
                return false;
            };
            if send_server_message(sender, &message).await.is_ok() {
                replication_cursor.record(&message);
                true
            } else {
                false
            }
        }
        intent => {
            let Some(actor_player_id) = binding.player_id() else {
                return send_server_message(
                    sender,
                    &ServerMessage::IntentRejected {
                        operation_sequence: intent.operation_sequence(),
                        operation_id: intent.operation_id().map(str::to_owned),
                        code: "spectator_read_only".into(),
                        message: "spectator sessions cannot submit gameplay mutations".into(),
                    },
                )
                .await
                .is_ok();
            };
            let result = state.execute_as(actor_player_id, &intent);
            match result {
                Ok(receipt) => {
                    if send_server_message(sender, &ServerMessage::IntentAccepted { receipt })
                        .await
                        .is_err()
                    {
                        return false;
                    }
                    true
                }
                Err(RuntimeError::Intent(source)) => {
                    send_intent_error(
                        sender,
                        intent.operation_sequence(),
                        intent.operation_id(),
                        source,
                    )
                    .await
                }
                Err(RuntimeError::CanonicalInvariant(source)) => {
                    error!(%source, "canonical world invariant failed while processing intent");
                    send_fatal_and_close(
                        sender,
                        "canonical_state_invalid",
                        "authoritative state validation failed; writes are stopped",
                    )
                    .await;
                    false
                }
                Err(RuntimeError::Persistence(source)) => {
                    error!(%source, "persistence failure while processing intent");
                    send_fatal_and_close(
                        sender,
                        "persistence_failure",
                        "authoritative persistence failed; writes are stopped",
                    )
                    .await;
                    false
                }
                Err(RuntimeError::Physics(source)) => {
                    error!(%source, "physics failure while processing intent");
                    send_fatal_and_close(
                        sender,
                        "physics_failure",
                        "authoritative physics rejected the operation",
                    )
                    .await;
                    false
                }
                Err(RuntimeError::Halted) => {
                    send_fatal_and_close(
                        sender,
                        "authoritative_writes_halted",
                        "the universe is in fail-closed recovery mode",
                    )
                    .await;
                    false
                }
            }
        }
    }
}

async fn send_intent_error(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    operation_sequence: Option<u64>,
    operation_id: Option<&str>,
    source: IntentError,
) -> bool {
    let message = source.message();
    send_server_message(
        sender,
        &ServerMessage::IntentRejected {
            operation_sequence,
            operation_id: operation_id.map(ToOwned::to_owned),
            code: source.code().into(),
            message,
        },
    )
    .await
    .is_ok()
}

async fn send_server_message(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &ServerMessage,
) -> Result<(), axum::Error> {
    let text = encode_server_message(message)?;
    tokio::time::timeout(
        SERVER_WRITE_TIMEOUT,
        sender.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| {
        axum::Error::new(io::Error::new(
            io::ErrorKind::TimedOut,
            "server message write exceeded the bounded deadline",
        ))
    })?
}

fn encode_server_message(message: &ServerMessage) -> Result<String, axum::Error> {
    encode_bounded_json(message).map_err(axum::Error::new)
}

fn encode_bounded_json<T: Serialize>(value: &T) -> Result<String, io::Error> {
    let started_at = std::time::Instant::now();
    let text = serde_json::to_string(value).map_err(io::Error::other)?;
    if started_at.elapsed() > MAX_SERIALIZATION_TIME {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "server message serialization exceeded the bounded deadline",
        ));
    }
    if text.len() > MAX_SERVER_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "server message exceeded the bounded byte budget",
        ));
    }
    Ok(text)
}

async fn send_fatal_and_close(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    code: &str,
    message: impl Into<String>,
) {
    let _ = send_server_message(
        sender,
        &ServerMessage::Fatal {
            code: code.into(),
            message: message.into(),
        },
    )
    .await;
    let _ = tokio::time::timeout(SERVER_WRITE_TIMEOUT, sender.send(Message::Close(None))).await;
}

async fn send_projection_failure(sender: &mut futures_util::stream::SplitSink<WebSocket, Message>) {
    send_fatal_and_close(
        sender,
        "projection_unavailable",
        "authorized state projection is unavailable",
    )
    .await;
}

pub fn internal_error(source: impl std::fmt::Display) -> impl IntoResponse {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("authoritative service failure: {source}"),
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use futures_util::{SinkExt as _, StreamExt as _};
    use http::Request;
    use tempfile::tempdir;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message as ClientWebSocketMessage;
    use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};
    use tower::ServiceExt;
    use verse_protocol::{BlockKind, ProductionRecipeKind, ResourceKind, Vec3};
    use verse_simulation::Store;

    use super::*;

    fn test_app() -> Router {
        router(test_state())
    }

    fn test_state() -> Arc<AppState> {
        let directory = tempdir().expect("tempdir").keep();
        let mut runtime = Runtime::open(directory, 99, 20).expect("runtime");
        runtime
            .admit_development_player("player-remote")
            .expect("remote development player admits");
        AppState::new(runtime)
    }

    fn production_test_state() -> Arc<AppState> {
        let directory = tempdir().expect("tempdir").keep();
        {
            let mut store = Store::open(&directory, 199).expect("fixture store opens");
            let mut world = store.load_world().expect("fixture world loads");
            let position = Vec3::new(900.0, -990.0, -3_800.0);
            let address = world
                .address_for_active_position(position)
                .expect("fixture position has a canonical address");
            world.player.position = position;
            world.player.address = address;
            world
                .inventories
                .get_mut("inventory-cargo-industry-starter")
                .expect("starter industry cargo exists")
                .contents
                .ore = 2;
            world.ledger.genesis_ore += 2;
            assert!(world.conservation().valid);
            store
                .save_snapshot(&world)
                .expect("production fixture persists");
        }
        AppState::new(Runtime::open(directory, 199, 20).expect("runtime"))
    }

    type TestSocket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

    async fn connect_test_socket() -> (TestSocket, Arc<AppState>, tokio::task::JoinHandle<()>) {
        let state = test_state();
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test listener binds");
        let address = listener.local_addr().expect("listener has address");
        let server_state = state.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, router(server_state))
                .await
                .expect("test server runs");
        });
        let (socket, _) = connect_async(format!("ws://{address}/ws"))
            .await
            .expect("test websocket connects");
        (socket, state, server)
    }

    async fn receive_wire_json(socket: &mut TestSocket) -> serde_json::Value {
        loop {
            let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .expect("server message arrives before timeout")
                .expect("websocket remains open")
                .expect("websocket message is valid");
            if let ClientWebSocketMessage::Text(text) = message {
                return serde_json::from_str(&text).expect("server message is valid JSON");
            }
        }
    }

    async fn receive_wire_message(socket: &mut TestSocket) -> ServerMessage {
        serde_json::from_value(receive_wire_json(socket).await)
            .expect("server message is valid protocol JSON")
    }

    async fn acknowledge_interest(socket: &mut TestSocket, interest: &InterestSnapshot) {
        send_client_message(
            socket,
            &ClientMessage::AcknowledgeInterest {
                session_epoch: interest.session_epoch.clone(),
                interest_epoch: interest.interest_epoch,
                baseline_id: interest.baseline_id.clone(),
                delta_sequence: interest.delta_sequence,
                view_hash: interest.view_hash.clone(),
            },
        )
        .await;
    }

    async fn receive_server_message(socket: &mut TestSocket) -> ServerMessage {
        loop {
            let message = receive_wire_message(socket).await;
            match message {
                ServerMessage::Registry { .. } => {}
                ServerMessage::InterestBaseline { baseline } => {
                    acknowledge_interest(socket, &baseline.interest).await;
                    return ServerMessage::Snapshot { snapshot: baseline };
                }
                ServerMessage::InterestDelta { delta } => {
                    acknowledge_interest(socket, &delta.interest).await;
                    return ServerMessage::InterestDelta { delta };
                }
                other => return other,
            }
        }
    }

    fn interest_delta_as_legacy_snapshot(
        delta: &verse_protocol::ProjectedInterestDelta,
    ) -> serde_json::Value {
        let mut players = Vec::new();
        let mut grids = Vec::new();
        let mut voxel_chunks = Vec::new();
        let mut death_drops = Vec::new();
        for projected in delta
            .interest
            .entered
            .iter()
            .chain(&delta.interest.replaced)
        {
            let value = serde_json::to_value(&projected.payload).expect("payload serializes");
            match value["entity_kind"].as_str() {
                Some("player") => players.push(value["value"].clone()),
                Some("grid") => grids.push(value["value"].clone()),
                Some("voxel_chunk") => voxel_chunks.push(value["value"].clone()),
                Some("death_drop") => death_drops.push(value["value"].clone()),
                _ => panic!("interest payload uses a known entity kind"),
            }
        }
        let mut snapshot = serde_json::json!({
            "projection_schema_version": delta.projection_schema_version,
            "schema_version": delta.schema_version,
            "content_manifest_version": delta.content_manifest_version,
            "universe_id": delta.universe_id,
            "cell_id": delta.cell_id,
            "universe_manifest_hash": delta.universe_manifest_hash,
            "celestial_registry_hash": delta.celestial_registry_hash,
            "cell_address": delta.cell_address,
            "gravity_body_id": delta.gravity_body_id,
            "voxel_body_id": delta.voxel_body_id,
            "event_sequence": delta.event_sequence,
            "simulation_tick": delta.simulation_tick,
            "world_hash": delta.world_hash,
            "players": players,
            "environment": delta.environment,
            "voxel_chunks": voxel_chunks,
            "grids": grids,
            "death_drops": death_drops,
            "conservation_valid": delta.conservation_valid,
            "interest": delta.interest,
            "actor_private": delta.actor_private,
        });
        if snapshot["actor_private"].is_null() {
            snapshot
                .as_object_mut()
                .expect("snapshot is an object")
                .remove("actor_private");
        }
        snapshot
    }

    async fn receive_server_json(socket: &mut TestSocket) -> serde_json::Value {
        loop {
            let message = receive_wire_message(socket).await;
            match message {
                ServerMessage::Registry { .. } => {}
                ServerMessage::InterestBaseline { baseline } => {
                    acknowledge_interest(socket, &baseline.interest).await;
                    return serde_json::json!({ "type": "snapshot", "snapshot": baseline });
                }
                ServerMessage::InterestDelta { delta } => {
                    acknowledge_interest(socket, &delta.interest).await;
                    return serde_json::json!({
                        "type": "snapshot",
                        "snapshot": interest_delta_as_legacy_snapshot(&delta),
                    });
                }
                other => return serde_json::to_value(other).expect("server message serializes"),
            }
        }
    }

    async fn receive_json_type(
        socket: &mut TestSocket,
        expected_type: &str,
        forbid_receipt: bool,
    ) -> serde_json::Value {
        for _ in 0..16 {
            let message = receive_server_json(socket).await;
            if forbid_receipt {
                assert!(
                    message["type"] != "intent_accepted" && message["type"] != "intent_rejected",
                    "another session must never receive an actor's intent result: {message}"
                );
            }
            if message["type"] == expected_type {
                return message;
            }
        }
        panic!("expected {expected_type} did not arrive within the bounded queue");
    }

    async fn assert_no_server_message(socket: &mut TestSocket) {
        let received = tokio::time::timeout(REPLICATION_PERIOD * 4, socket.next()).await;
        assert!(
            received.is_err(),
            "session unexpectedly received a server message: {received:?}"
        );
    }

    async fn receive_until(
        socket: &mut TestSocket,
        predicate: impl Fn(&ServerMessage) -> bool,
    ) -> ServerMessage {
        for _ in 0..16 {
            let message = receive_server_message(socket).await;
            if predicate(&message) {
                return message;
            }
        }
        panic!("expected server message did not arrive within the bounded queue");
    }

    async fn send_client_message(socket: &mut TestSocket, message: &ClientMessage) {
        socket
            .send(ClientWebSocketMessage::Text(
                serde_json::to_string(message)
                    .expect("client message serializes")
                    .into(),
            ))
            .await
            .expect("client message sends");
    }

    fn server_address(socket: &TestSocket) -> std::net::SocketAddr {
        match socket.get_ref() {
            MaybeTlsStream::Plain(stream) => stream.peer_addr().expect("server address"),
            _ => panic!("test connection is plain WebSocket"),
        }
    }

    async fn connect_additional(socket: &TestSocket) -> TestSocket {
        let (additional, _) = connect_async(format!("ws://{}/ws", server_address(socket)))
            .await
            .expect("additional test websocket connects");
        additional
    }

    async fn wait_for_player_release(state: &AppState, player_id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.connected_players.lock().contains(player_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("closed gameplay session releases its player binding");
    }

    async fn complete_session(socket: &mut TestSocket, hello: &ClientMessage) -> serde_json::Value {
        send_client_message(socket, hello).await;
        assert!(matches!(
            receive_wire_message(socket).await,
            ServerMessage::Welcome { .. }
        ));
        assert!(matches!(
            receive_wire_message(socket).await,
            ServerMessage::Registry { .. }
        ));
        let ServerMessage::InterestBaseline { baseline } = receive_wire_message(socket).await
        else {
            panic!("protocol-16 session must begin with an interest baseline");
        };
        acknowledge_interest(socket, &baseline.interest).await;
        serde_json::json!({ "type": "snapshot", "snapshot": baseline })
    }

    fn assert_public_fields_are_redacted(snapshot: &serde_json::Value) {
        assert!(snapshot.get("inventories").is_none());
        assert!(snapshot.get("conservation").is_none());
        assert!(snapshot.get("committed_operation_sequence").is_none());
        for drop in snapshot["death_drops"]
            .as_array()
            .expect("public death-drop list")
        {
            for private_field in [
                "owner_player_id",
                "inventory_id",
                "source_death_id",
                "cause",
                "contents",
            ] {
                assert!(
                    drop.get(private_field).is_none(),
                    "public death drop leaked {private_field}"
                );
            }
        }
        for player in snapshot["players"]
            .as_array()
            .expect("public player roster")
        {
            for private_field in [
                "inventory_id",
                "experience",
                "career",
                "suit_oxygen_milli",
                "movement_epoch",
                "last_received_input_sequence",
                "control_linear_input",
                "committed_operation_sequence",
            ] {
                assert!(
                    player.get(private_field).is_none(),
                    "public player leaked {private_field}"
                );
            }
        }
        for grid in snapshot["grids"].as_array().expect("public grid list") {
            assert!(grid.get("mass_kg").is_none());
            for block in grid["blocks"].as_array().expect("public block list") {
                assert!(block.get("inventory_id").is_none());
            }
        }
    }

    fn assert_snapshot_audience(message: &serde_json::Value, expected_actor: Option<&str>) {
        assert_eq!(message["type"], "snapshot");
        let snapshot = &message["snapshot"];
        assert_public_fields_are_redacted(snapshot);
        let encoded = serde_json::to_string(message).expect("message serializes");
        match expected_actor {
            None => {
                assert!(snapshot.get("actor_private").is_none());
                assert!(!encoded.contains("committed_operation_sequence"));
                for secret in [
                    "inventory-player-local",
                    "inventory-player-remote",
                    "inventory-cargo-industry-starter",
                    "inventory-cargo-starter",
                    "inventory-drop-player-local",
                    "death-player-local",
                ] {
                    assert!(!encoded.contains(secret), "spectator leaked {secret}");
                }
            }
            Some(player_id) => {
                let private = snapshot
                    .get("actor_private")
                    .expect("player projection contains actor-private state");
                assert_eq!(private["player"]["player_id"], player_id);
                assert!(private["committed_operation_sequence"].is_u64());
                assert_eq!(
                    private["player"]["inventory_id"],
                    format!("inventory-{player_id}")
                );
                if player_id == "player-local" {
                    assert!(!encoded.contains("inventory-player-remote"));
                } else {
                    for secret in [
                        "inventory-player-local",
                        "inventory-cargo-industry-starter",
                        "inventory-cargo-starter",
                        "inventory-drop-player-local",
                        "death-player-local",
                    ] {
                        assert!(!encoded.contains(secret), "foreign actor leaked {secret}");
                    }
                }
            }
        }
    }

    async fn assert_intent_rejected(
        socket: &mut TestSocket,
        intent: ClientMessage,
        expected_code: &str,
    ) {
        let operation_id = intent
            .operation_id()
            .expect("test mutation has an operation ID")
            .to_owned();
        let operation_sequence = intent
            .operation_sequence()
            .expect("test mutation has an operation sequence");
        send_client_message(socket, &intent).await;
        let response = receive_until(socket, |message| {
            matches!(
                message,
                ServerMessage::IntentRejected {
                    operation_sequence: Some(candidate_sequence),
                    operation_id: Some(candidate),
                    ..
                } if *candidate_sequence == operation_sequence && candidate == &operation_id
            )
        })
        .await;
        assert!(
            matches!(
                &response,
            ServerMessage::IntentRejected {
                operation_sequence: Some(candidate_sequence),
                operation_id: Some(candidate),
                code,
                ..
            } if *candidate_sequence == operation_sequence
                && candidate == &operation_id
                && code == expected_code
            ),
            "{operation_id} must fail closed with {expected_code}; received {response:?}"
        );
    }

    async fn receive_intent_accepted(
        socket: &mut TestSocket,
        operation_sequence: u64,
        operation_id: &str,
    ) -> IntentReceipt {
        let message = receive_until(socket, |message| {
            matches!(
                message,
                ServerMessage::IntentAccepted { receipt }
                    if receipt.operation_sequence == operation_sequence
                        && receipt.operation_id == operation_id
            )
        })
        .await;
        let ServerMessage::IntentAccepted { receipt } = message else {
            unreachable!("receive_until matched an accepted receipt")
        };
        receipt
    }

    fn player_hello(client_name: &str, player_id: &str) -> ClientMessage {
        ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: client_name.into(),
            authentication: ClientAuthentication::LocalDevelopment {
                player_id: player_id.into(),
            },
        }
    }

    fn local_player_hello(client_name: &str) -> ClientMessage {
        player_hello(client_name, "player-local")
    }

    fn suit_mode_intent(
        operation_sequence: u64,
        operation_id: &str,
        helmet_closed: bool,
    ) -> ClientMessage {
        ClientMessage::SetSuitMode {
            operation_sequence,
            operation_id: operation_id.into(),
            helmet_closed,
            jetpack_enabled: true,
            magnetic_boots_enabled: false,
        }
    }

    fn actor_player(snapshot: &ProjectedWorldSnapshot) -> &verse_protocol::PlayerSnapshot {
        &snapshot
            .actor_private
            .as_ref()
            .expect("authenticated player snapshot has an actor-private view")
            .player
    }

    struct ActorDeltaView {
        movement_epoch: u64,
        last_received_input_sequence: u64,
        last_processed_input_sequence: u64,
        linear_velocity: verse_protocol::Vec3,
        locomotion: verse_protocol::PlayerLocomotionSnapshot,
        jetpack_enabled: bool,
    }

    fn actor_delta(delta: &verse_protocol::ProjectedInterestDelta) -> ActorDeltaView {
        let player = if let Some(private) = &delta.actor_private {
            &private.player
        } else {
            let motion = delta
                .actor_private_motion
                .as_ref()
                .expect("authenticated delta has private structure or private motion");
            return ActorDeltaView {
                movement_epoch: motion.movement_epoch,
                last_received_input_sequence: motion.last_received_input_sequence,
                last_processed_input_sequence: motion.last_processed_input_sequence,
                linear_velocity: motion.linear_velocity,
                locomotion: motion.locomotion.clone(),
                jetpack_enabled: motion.jetpack_enabled,
            };
        };
        ActorDeltaView {
            movement_epoch: player.movement_epoch,
            last_received_input_sequence: player.last_received_input_sequence,
            last_processed_input_sequence: player.last_processed_input_sequence,
            linear_velocity: player.linear_velocity,
            locomotion: player.locomotion.clone(),
            jetpack_enabled: player.jetpack_enabled,
        }
    }

    async fn assert_socket_closes(socket: &mut TestSocket) {
        let next = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("server closes incompatible connection promptly");
        assert!(matches!(
            next,
            None | Some(Ok(ClientWebSocketMessage::Close(_)))
        ));
    }

    #[test]
    fn latest_state_feed_coalesces_a_slow_consumers_motion_backlog() {
        let mut feed = ReplicationFeed::default();
        for event_sequence in 1..=4_096 {
            assert!(feed.publish(ReplicationKind::Motion, event_sequence));
        }

        assert_eq!(feed.retained_update_count(), 1);
        let cursor = ReplicationCursor::after_initial_snapshot(0);
        let Some(PendingReplication::Motion) = feed.next_after(cursor) else {
            panic!("the slow consumer receives the latest coalesced motion state");
        };
        assert_eq!(feed.latest_motion_sequence, Some(4_096));
    }

    #[test]
    fn session_projection_does_not_hold_or_wait_for_the_authoritative_runtime_lock() {
        let state = test_state();
        let projecting_state = state.clone();
        let runtime_guard = state.runtime.lock();
        let (result_sender, result_receiver) = std::sync::mpsc::channel();
        let projection = std::thread::spawn(move || {
            let mut cursor = InterestProjectionState::public_origin_spectator("lock-proof");
            let result = projecting_state.projected_interest_frame(&mut cursor);
            result_sender.send(result).expect("projection result sends");
        });

        let result = result_receiver.recv_timeout(Duration::from_secs(1));
        drop(runtime_guard);
        projection.join().expect("projection thread joins");
        assert!(matches!(
            result.expect("projection completes while the runtime lock is held"),
            Ok(ProjectedInterestFrame::Baseline(_))
        ));
    }

    #[test]
    fn outbound_messages_and_interest_recovery_are_strictly_bounded() {
        let normal = ServerMessage::Fatal {
            code: "bounded".into(),
            message: "small".into(),
        };
        assert!(encode_server_message(&normal).is_ok());
        let oversized = ServerMessage::Fatal {
            code: "bounded".into(),
            message: "x".repeat(MAX_SERVER_MESSAGE_BYTES),
        };
        assert!(encode_server_message(&oversized).is_err());

        let state = test_state();
        let mut transport = SessionTransport::new(&SessionBinding::Spectator);
        let first = transport
            .stage(&state, false)
            .expect("initial bounded baseline stages");
        let first_frontier = replication_interest(&first)
            .map(InterestFrontier::from_interest)
            .expect("initial baseline has a frontier");
        assert!(!transport.pending_timed_out());
        transport.pending.as_mut().expect("pending frame").sent_at =
            tokio::time::Instant::now() - UNACKNOWLEDGED_FRAME_TIMEOUT;
        assert!(transport.pending_timed_out());
        assert!(!transport.recovery_is_pending());
        assert!(transport.permit_recovery());
        let recovery = transport
            .stage(&state, true)
            .expect("a timed-out ordinary frame rebases once");
        let recovery_frontier = replication_interest(&recovery)
            .map(InterestFrontier::from_interest)
            .expect("recovery baseline has a frontier");
        assert_eq!(
            recovery_frontier.interest_epoch,
            first_frontier.interest_epoch + 1
        );
        assert_ne!(recovery_frontier.baseline_id, first_frontier.baseline_id);
        assert!(transport.recovery_is_pending());
        assert!(transport.matches_superseded_acknowledgement(
            &first_frontier.session_epoch,
            first_frontier.interest_epoch,
            &first_frontier.baseline_id,
            first_frontier.delta_sequence,
            &first_frontier.view_hash,
        ));
        transport
            .pending
            .as_mut()
            .expect("recovery pending")
            .sent_at = tokio::time::Instant::now() - UNACKNOWLEDGED_FRAME_TIMEOUT;
        assert!(transport.pending_timed_out() && transport.recovery_is_pending());

        let mut bounded = SessionTransport::new(&SessionBinding::Spectator);
        for _ in 0..MAX_RECOVERIES_PER_WINDOW {
            assert!(bounded.permit_recovery());
        }
        assert!(!bounded.permit_recovery());
    }

    #[test]
    fn public_http_projection_is_bounded_cached_and_rate_constrained() {
        let state = test_state();
        let first = state
            .bounded_public_world_json()
            .expect("initial public HTTP projection");
        assert!(first.len() <= MAX_SERVER_MESSAGE_BYTES);
        let same = state
            .bounded_public_world_json()
            .expect("same revision uses cache");
        assert!(Arc::ptr_eq(&first, &same));

        let before_snapshot = state.snapshot();
        let before = before_snapshot.event_sequence;
        state
            .execute_as(
                "player-local",
                &ClientMessage::SetSuitMode {
                    operation_sequence: 1,
                    operation_id: "http-cache-refresh".into(),
                    helmet_closed: !before_snapshot.player.helmet_closed,
                    jetpack_enabled: before_snapshot.player.jetpack_enabled,
                    magnetic_boots_enabled: before_snapshot
                        .player
                        .locomotion
                        .magnetic_boots_enabled,
                },
            )
            .expect("structural world change commits");
        let after = state.snapshot().event_sequence;
        assert!(after > before);
        let rate_limited = state
            .bounded_public_world_json()
            .expect("fresh cache is served during the refresh floor");
        assert!(Arc::ptr_eq(&first, &rate_limited));

        state
            .public_world_cache
            .lock()
            .as_mut()
            .expect("cache exists")
            .generated_at = Instant::now()
            .checked_sub(HTTP_PROJECTION_MIN_REFRESH)
            .expect("test refresh floor fits monotonic time");
        let refreshed = state
            .bounded_public_world_json()
            .expect("stale cache refreshes once");
        assert!(!Arc::ptr_eq(&first, &refreshed));
        let refreshed_value: serde_json::Value =
            serde_json::from_str(&refreshed).expect("cached response is JSON");
        assert_eq!(refreshed_value["event_sequence"], after);
    }

    #[test]
    fn idempotent_retry_does_not_schedule_another_complete_snapshot() {
        let state = test_state();
        let mut observer = state.updates.subscribe();
        let player = state.snapshot().player;
        let intent = ClientMessage::SetSuitMode {
            operation_sequence: 1,
            operation_id: "idempotent-structural-retry".into(),
            helmet_closed: !player.helmet_closed,
            jetpack_enabled: player.jetpack_enabled,
            magnetic_boots_enabled: player.locomotion.magnetic_boots_enabled,
        };

        let first = state
            .execute_as("player-local", &intent)
            .expect("the structural operation commits");
        assert!(observer.has_changed().expect("the feed remains open"));
        assert_eq!(
            observer.borrow_and_update().latest_structural_sequence,
            Some(first.event_sequence)
        );

        let retry = state
            .execute_as("player-local", &intent)
            .expect("the idempotent retry returns its receipt");
        assert_eq!(retry, first);
        assert!(!observer.has_changed().expect("the feed remains open"));
    }

    #[test]
    fn production_progress_publishes_structural_without_life_support_change() {
        let state = production_test_state();
        let before = state
            .projected_snapshot(Some("player-local"))
            .expect("private projection succeeds");
        let before_oxygen = actor_player(&before).suit_oxygen_milli;
        assert!(before.environment.breathable);

        let mut observer = state.updates.subscribe();
        let queued = state
            .execute_as(
                "player-local",
                &ClientMessage::QueueProduction {
                    operation_sequence: 1,
                    operation_id: "replicate-production-progress".into(),
                    machine_block_id: "block-refinery".into(),
                    recipe: ProductionRecipeKind::Refining,
                    batches: 1,
                    source_inventory_id: "inventory-cargo-industry-starter".into(),
                    destination_inventory_id: "inventory-cargo-industry-starter".into(),
                },
            )
            .expect("production job queues");
        assert_eq!(
            observer.borrow_and_update().latest_structural_sequence,
            Some(queued.event_sequence)
        );

        for _ in 0..3 {
            assert!(!state.advance(250).expect("partial production tick"));
            assert!(!observer.has_changed().expect("the feed remains open"));
        }
        assert!(state.advance(250).expect("production second advances"));
        assert!(observer.has_changed().expect("the feed remains open"));
        let feed = observer.borrow_and_update().clone();
        let after = state
            .projected_snapshot(Some("player-local"))
            .expect("private projection succeeds");
        let private = after
            .actor_private
            .as_ref()
            .expect("player receives actor-private state");
        let canonical_progress =
            state.runtime.lock().state().production_queues["block-refinery"][0].progress_ticks;

        assert_eq!(actor_player(&after).suit_oxygen_milli, before_oxygen);
        assert_eq!(canonical_progress, 60);
        assert!(
            private.production_queues.is_empty(),
            "remote production details stay outside the actor's interest view"
        );
        assert_eq!(feed.latest_structural_sequence, Some(after.event_sequence));
        assert_eq!(feed.latest_motion_sequence, None);
        assert!(after.event_sequence > queued.event_sequence);
    }

    #[test]
    fn structural_state_precedes_newer_motion_and_is_never_coalesced_away() {
        let mut feed = ReplicationFeed::default();
        // Reverse publication exercises the fail-safe ordering independently of
        // the runtime lock that normally serializes these updates.
        assert!(feed.publish(ReplicationKind::Motion, 9));
        assert!(feed.publish(ReplicationKind::Structural, 7));
        assert_eq!(feed.retained_update_count(), 2);

        let mut cursor = ReplicationCursor::after_initial_snapshot(0);
        let Some(PendingReplication::Structural) = feed.next_after(cursor) else {
            panic!("the required complete snapshot is selected first");
        };
        cursor.record(&ServerMessage::Snapshot {
            snapshot: Box::new(
                test_state()
                    .projected_snapshot(None)
                    .expect("spectator projection"),
            ),
        });
        cursor.event_sequence = 7;
        cursor.full_snapshot_sequence = 7;

        let Some(PendingReplication::Motion) = feed.next_after(cursor) else {
            panic!("the newer motion state follows the structural snapshot");
        };
        cursor.event_sequence = 9;
        assert!(feed.next_after(cursor).is_none());
        assert_eq!(cursor.event_sequence, 9);
        assert_eq!(cursor.full_snapshot_sequence, 7);
    }

    #[test]
    fn replication_cursor_requests_a_fresh_snapshot_instead_of_rolling_back() {
        let state = test_state();
        let mut feed = ReplicationFeed::default();
        assert!(feed.publish(ReplicationKind::Structural, 10));
        let cursor = ReplicationCursor {
            event_sequence: 12,
            full_snapshot_sequence: 4,
        };

        assert!(matches!(
            feed.next_after(cursor),
            Some(PendingReplication::Structural)
        ));
        let spectator_refresh = projected_replication_message(
            &state,
            &SessionBinding::Spectator,
            PendingReplication::Structural,
        )
        .expect("spectator refresh projects");
        let local_refresh = projected_replication_message(
            &state,
            &SessionBinding::Player("player-local".into()),
            PendingReplication::Structural,
        )
        .expect("actor refresh projects");
        assert_snapshot_audience(
            &serde_json::to_value(spectator_refresh).expect("spectator refresh serializes"),
            None,
        );
        assert_snapshot_audience(
            &serde_json::to_value(local_refresh).expect("local refresh serializes"),
            Some("player-local"),
        );
    }

    #[test]
    fn runtime_bursts_retain_at_most_one_structural_and_one_motion_update() {
        let state = test_state();
        let mut slow_consumer = state.updates.subscribe();
        let initial = state.snapshot();
        let mut cursor = ReplicationCursor::after_initial_snapshot(initial.event_sequence);
        let player = initial
            .players
            .iter()
            .find(|player| player.player_id == "player-local")
            .expect("local player is present");
        let mut published_steps = 0;
        for input_sequence in 1..=8 {
            state
                .execute_as(
                    "player-local",
                    &ClientMessage::SetPlayerControl {
                        operation_sequence: input_sequence,
                        operation_id: format!("slow-consumer-burst-{input_sequence}"),
                        movement_epoch: player.movement_epoch,
                        input_sequence,
                        linear_input: verse_protocol::Vec3::new(0.0, 0.0, -1.0),
                        angular_input: verse_protocol::Vec3::ZERO,
                        boost: false,
                        jump: false,
                        dampeners: true,
                    },
                )
                .expect("control renews authoritative movement");
            for _ in 0..16 {
                published_steps += usize::from(
                    state
                        .advance(17)
                        .expect("authoritative time advances without failure"),
                );
            }
        }
        assert!(
            published_steps > 64,
            "the burst exceeds the removed queue; observed {published_steps} published updates"
        );

        let authoritative = state.snapshot();
        let feed = slow_consumer.borrow_and_update().clone();
        assert!((1..=2).contains(&feed.retained_update_count()));
        let mut delivered = Vec::new();
        for _ in 0..2 {
            let Some(pending) = feed.next_after(cursor) else {
                break;
            };
            let message = Arc::new(
                projected_replication_message(&state, &SessionBinding::Spectator, pending)
                    .expect("spectator replication projection succeeds"),
            );
            cursor.record(&message);
            delivered.push(message);
        }

        assert!(!delivered.is_empty());
        assert!(delivered.len() <= 2);
        let final_message = delivered.last().expect("a final replication exists");
        assert_eq!(
            replication_event_sequence(final_message),
            Some(authoritative.event_sequence)
        );
        let final_hash = match final_message.as_ref() {
            ServerMessage::Snapshot { snapshot } => &snapshot.world_hash,
            ServerMessage::MotionState { motion } => &motion.world_hash,
            _ => panic!("the final coalesced message is state replication"),
        };
        assert_eq!(final_hash, &authoritative.world_hash);
        assert_eq!(cursor.event_sequence, authoritative.event_sequence);
    }

    #[test]
    fn per_connection_replication_rate_is_capped_at_sixty_hertz() {
        assert!(REPLICATION_PERIOD >= Duration::from_nanos(1_000_000_000 / 60));
    }

    #[tokio::test]
    async fn silent_and_ping_only_handshakes_expire_and_release_admission() {
        let (mut silent, state, server) = connect_test_socket().await;
        assert_eq!(
            state.session_admission.available_permits(),
            MAX_CONCURRENT_CONNECTIONS - 1
        );
        assert!(matches!(
            receive_wire_message(&mut silent).await,
            ServerMessage::Fatal { ref code, .. } if code == "protocol_handshake_timeout"
        ));
        assert_socket_closes(&mut silent).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.session_admission.available_permits() != MAX_CONCURRENT_CONNECTIONS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("silent handshake releases its admission permit");
        server.abort();

        let (mut ping_only, state, server) = connect_test_socket().await;
        ping_only
            .send(ClientWebSocketMessage::Ping(vec![1].into()))
            .await
            .expect("first ping sends");
        tokio::time::sleep(HANDSHAKE_TIMEOUT / 2).await;
        ping_only
            .send(ClientWebSocketMessage::Ping(vec![2].into()))
            .await
            .expect("second ping sends");
        assert!(matches!(
            receive_wire_message(&mut ping_only).await,
            ServerMessage::Fatal { ref code, .. } if code == "protocol_handshake_timeout"
        ));
        assert_socket_closes(&mut ping_only).await;
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.session_admission.available_permits() != MAX_CONCURRENT_CONNECTIONS {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("ping-only handshake releases its admission permit");
        server.abort();
    }

    #[tokio::test]
    async fn websocket_requires_hello_before_snapshot_or_mutation() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
                operation_sequence: 1,
                operation_id: "pre-handshake-mutation".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::ZERO,
                angular_input: verse_protocol::Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Fatal { ref code, .. } if code == "protocol_handshake_required"
        ));
        assert_eq!(state.snapshot().event_sequence, 0);
        assert_socket_closes(&mut socket).await;
        server.abort();
    }

    #[tokio::test]
    async fn websocket_rejects_and_closes_an_incompatible_protocol() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(
            &mut socket,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION - 1,
                client_name: "obsolete-client".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Fatal { ref code, .. } if code == "protocol_version_mismatch"
        ));
        assert_eq!(state.snapshot().event_sequence, 0);
        assert_socket_closes(&mut socket).await;
        server.abort();
    }

    #[tokio::test]
    async fn websocket_sends_snapshot_only_after_a_compatible_hello() {
        let (mut socket, _state, server) = connect_test_socket().await;
        send_client_message(&mut socket, &local_player_hello("compatible-test-client")).await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome {
                protocol_version: PROTOCOL_VERSION,
                session_role: SessionRole::Player { .. },
                ..
            }
        ));
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Snapshot { .. }
        ));
        socket.close(None).await.expect("test socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn protocol16_orders_full_compatibility_registry_then_interest_baseline() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(
            &mut socket,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "interest-ordering".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;

        let welcome = receive_wire_message(&mut socket).await;
        assert!(matches!(
            welcome,
            ServerMessage::Welcome {
                protocol_version: PROTOCOL_VERSION,
                projection_schema_version: PROJECTION_SCHEMA_VERSION,
                world_schema_version: WORLD_SCHEMA_VERSION,
                event_schema_version: EVENT_SCHEMA_VERSION,
                content_schema_version: 11,
                ref content_manifest_version,
                celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
                universe_manifest_schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
                interest_schema_version: INTEREST_SCHEMA_VERSION,
                session_role: SessionRole::Spectator,
                ..
            } if content_manifest_version == "p1.5.0"
        ));
        let ServerMessage::Registry {
            registry,
            universe_manifest,
        } = receive_wire_message(&mut socket).await
        else {
            panic!("registry must precede cell state");
        };
        let ServerMessage::InterestBaseline { baseline } = receive_wire_message(&mut socket).await
        else {
            panic!("first cell state must be an interest baseline");
        };
        assert_eq!(baseline.interest.interest_epoch, 1);
        assert_eq!(baseline.interest.delta_sequence, 0);
        assert_eq!(
            baseline.interest.observer_class,
            verse_protocol::InterestObserverClass::PublicOriginSpectator
        );
        assert_eq!(baseline.interest.registry_hash, registry.registry_hash);
        assert_eq!(
            baseline.interest.universe_manifest_hash,
            universe_manifest.manifest_hash
        );
        assert_eq!(baseline.world_hash, state.snapshot().world_hash);
        assert!(uuid::Uuid::parse_str(&baseline.interest.session_epoch).is_ok());

        socket.close(None).await.expect("test socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn one_wrong_ack_rebases_and_a_repeat_closes_without_world_mutation() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(
            &mut socket,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "adversarial-interest-ack".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        assert!(matches!(
            receive_wire_message(&mut socket).await,
            ServerMessage::Welcome { .. }
        ));
        assert!(matches!(
            receive_wire_message(&mut socket).await,
            ServerMessage::Registry { .. }
        ));
        let ServerMessage::InterestBaseline { baseline: first } =
            receive_wire_message(&mut socket).await
        else {
            panic!("initial baseline");
        };
        let before = state.snapshot();

        let mut wrong_hash = first.interest.clone();
        wrong_hash.view_hash = "0".repeat(64);
        acknowledge_interest(&mut socket, &wrong_hash).await;
        let ServerMessage::InterestBaseline { baseline: second } =
            receive_wire_message(&mut socket).await
        else {
            panic!("wrong hash must produce one fresh baseline");
        };
        assert_eq!(second.interest.interest_epoch, 2);
        assert_ne!(second.interest.baseline_id, first.interest.baseline_id);

        acknowledge_interest(&mut socket, &first.interest).await;
        assert_no_server_message(&mut socket).await;
        acknowledge_interest(&mut socket, &wrong_hash).await;
        assert!(matches!(
            receive_wire_message(&mut socket).await,
            ServerMessage::Fatal { ref code, .. }
                if code == "interest_recovery_rate_limited"
        ));
        assert_socket_closes(&mut socket).await;

        let after = state.snapshot();
        assert_eq!(after.event_sequence, before.event_sequence);
        assert_eq!(after.world_hash, before.world_hash);
        assert_eq!(after.simulation_tick, before.simulation_tick);

        server.abort();
    }

    #[tokio::test]
    async fn reconnect_uses_a_new_opaque_session_epoch_and_baseline() {
        let (mut first, _state, server) = connect_test_socket().await;
        let initial = complete_session(
            &mut first,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "session-epoch-first".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        let first_epoch = initial["snapshot"]["interest"]["session_epoch"]
            .as_str()
            .expect("session epoch")
            .to_owned();
        let first_baseline = initial["snapshot"]["interest"]["baseline_id"]
            .as_str()
            .expect("baseline ID")
            .to_owned();
        let address = server_address(&first);
        first.close(None).await.expect("first socket closes");

        let (mut second, _) = connect_async(format!("ws://{address}/ws"))
            .await
            .expect("reconnected test websocket connects");
        let reconnected = complete_session(
            &mut second,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "session-epoch-second".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        assert_eq!(reconnected["snapshot"]["interest"]["interest_epoch"], 1);
        assert_ne!(
            reconnected["snapshot"]["interest"]["session_epoch"],
            first_epoch
        );
        assert_ne!(
            reconnected["snapshot"]["interest"]["baseline_id"],
            first_baseline
        );

        second.close(None).await.expect("second socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn spectator_session_can_observe_but_cannot_mutate() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(
            &mut socket,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "spectator-test-client".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome {
                session_role: SessionRole::Spectator,
                ..
            }
        ));
        let ServerMessage::Snapshot { snapshot } = receive_server_message(&mut socket).await else {
            panic!("spectator receives the public authoritative snapshot");
        };
        let before_hash = snapshot.world_hash;

        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
                operation_sequence: 1,
                operation_id: "spectator-spoof-1".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::new(0.0, 0.0, -1.0),
                angular_input: verse_protocol::Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::IntentRejected { ref code, .. } if code == "spectator_read_only"
        ));
        assert_eq!(state.snapshot().event_sequence, 0);
        assert_eq!(state.snapshot().world_hash, before_hash);
        socket.close(None).await.expect("test socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn handshake_rejects_unknown_and_concurrently_connected_players() {
        let (mut first, _state, server) = connect_test_socket().await;
        send_client_message(&mut first, &local_player_hello("first-player-session")).await;
        assert!(matches!(
            receive_server_message(&mut first).await,
            ServerMessage::Welcome { .. }
        ));
        assert!(matches!(
            receive_server_message(&mut first).await,
            ServerMessage::Snapshot { .. }
        ));

        let address = match first.get_ref() {
            MaybeTlsStream::Plain(stream) => stream.peer_addr().expect("server address"),
            _ => panic!("test connection is plain WebSocket"),
        };
        let (mut duplicate, _) = connect_async(format!("ws://{address}/ws"))
            .await
            .expect("second test websocket connects");
        send_client_message(
            &mut duplicate,
            &local_player_hello("duplicate-player-session"),
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut duplicate).await,
            ServerMessage::Fatal { ref code, .. } if code == "player_already_connected"
        ));
        assert_socket_closes(&mut duplicate).await;

        first.close(None).await.expect("first session closes");
        server.abort();

        let (mut unknown, _state, unknown_server) = connect_test_socket().await;
        send_client_message(
            &mut unknown,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "unknown-player-session".into(),
                authentication: ClientAuthentication::LocalDevelopment {
                    player_id: "player-not-admitted".into(),
                },
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut unknown).await,
            ServerMessage::Fatal { ref code, .. } if code == "actor_not_present"
        ));
        assert_socket_closes(&mut unknown).await;
        unknown_server.abort();
    }

    #[tokio::test]
    async fn two_player_sockets_bind_and_advance_independent_control_frontiers() {
        let (mut local, state, server) = connect_test_socket().await;
        send_client_message(&mut local, &local_player_hello("local-two-player-test")).await;
        assert!(matches!(
            receive_server_message(&mut local).await,
            ServerMessage::Welcome {
                session_role: SessionRole::Player { ref player_id },
                ..
            } if player_id == "player-local"
        ));
        let ServerMessage::Snapshot {
            snapshot: local_snapshot,
        } = receive_server_message(&mut local).await
        else {
            panic!("local player receives the shared initial snapshot");
        };

        let address = match local.get_ref() {
            MaybeTlsStream::Plain(stream) => stream.peer_addr().expect("server address"),
            _ => panic!("test connection is plain WebSocket"),
        };
        let (mut remote, _) = connect_async(format!("ws://{address}/ws"))
            .await
            .expect("remote test websocket connects");
        send_client_message(
            &mut remote,
            &player_hello("remote-two-player-test", "player-remote"),
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut remote).await,
            ServerMessage::Welcome {
                session_role: SessionRole::Player { ref player_id },
                ..
            } if player_id == "player-remote"
        ));
        let ServerMessage::Snapshot {
            snapshot: remote_snapshot,
        } = receive_server_message(&mut remote).await
        else {
            panic!("remote player receives the shared initial snapshot");
        };
        assert_eq!(local_snapshot.world_hash, remote_snapshot.world_hash);
        assert_eq!(
            local_snapshot
                .players
                .iter()
                .map(|player| player.player_id.as_str())
                .collect::<Vec<_>>(),
            vec!["player-local", "player-remote"]
        );

        let shared_operation_id = "two-socket-shared-operation";
        let local_player = actor_player(&local_snapshot);
        send_client_message(
            &mut local,
            &ClientMessage::SetPlayerControl {
                operation_sequence: 1,
                operation_id: shared_operation_id.into(),
                movement_epoch: local_player.movement_epoch,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::new(1.0, 0.0, 0.0),
                angular_input: verse_protocol::Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            },
        )
        .await;
        assert!(matches!(
            receive_until(&mut local, |message| matches!(
                message,
                ServerMessage::IntentAccepted { .. }
            ))
            .await,
            ServerMessage::IntentAccepted { .. }
        ));

        let remote_player = actor_player(&remote_snapshot);
        send_client_message(
            &mut remote,
            &ClientMessage::SetPlayerControl {
                operation_sequence: 1,
                operation_id: shared_operation_id.into(),
                movement_epoch: remote_player.movement_epoch,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::new(-1.0, 0.0, 0.0),
                angular_input: verse_protocol::Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            },
        )
        .await;
        assert!(matches!(
            receive_until(&mut remote, |message| matches!(
                message,
                ServerMessage::IntentAccepted { .. }
            ))
            .await,
            ServerMessage::IntentAccepted { .. }
        ));

        let shared = state.snapshot();
        assert_eq!(shared.event_sequence, 2);
        assert!(shared.players.iter().all(|player| {
            player.last_received_input_sequence == 1 && player.last_processed_input_sequence == 0
        }));
        for socket in [&mut local, &mut remote] {
            send_client_message(socket, &ClientMessage::RequestSnapshot).await;
        }
        let ServerMessage::Snapshot {
            snapshot: local_final,
        } = receive_until(&mut local, |message| {
            matches!(message, ServerMessage::Snapshot { .. })
        })
        .await
        else {
            unreachable!();
        };
        let ServerMessage::Snapshot {
            snapshot: remote_final,
        } = receive_until(&mut remote, |message| {
            matches!(message, ServerMessage::Snapshot { .. })
        })
        .await
        else {
            unreachable!();
        };
        assert_eq!(local_final.world_hash, remote_final.world_hash);
        assert_eq!(local_final.world_hash, shared.world_hash);
        assert_eq!(
            local_final
                .actor_private
                .as_ref()
                .expect("local private frontier")
                .committed_operation_sequence,
            1
        );
        assert_eq!(
            remote_final
                .actor_private
                .as_ref()
                .expect("remote private frontier")
                .committed_operation_sequence,
            1
        );

        local.close(None).await.expect("local socket closes");
        remote.close(None).await.expect("remote socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn remote_socket_cannot_operate_another_players_industry_or_grid() {
        let (mut remote, state, server) = connect_test_socket().await;
        send_client_message(
            &mut remote,
            &player_hello("remote-authority-test", "player-remote"),
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut remote).await,
            ServerMessage::Welcome {
                session_role: SessionRole::Player { ref player_id },
                ..
            } if player_id == "player-remote"
        ));
        let ServerMessage::Snapshot { snapshot } = receive_server_message(&mut remote).await else {
            panic!("remote player receives its projected initial snapshot");
        };
        assert_eq!(actor_player(&snapshot).player_id, "player-remote");
        let before = state.snapshot();
        let primary = before
            .players
            .iter()
            .find(|player| player.player_id == "player-local")
            .expect("primary actor is present");
        let remote_player = before
            .players
            .iter()
            .find(|player| player.player_id == "player-remote")
            .expect("remote actor is present");
        let starter_grid = before.grids.first().expect("starter grid is present");
        assert_eq!(starter_grid.owner_player_id, "player-local");
        let starter_block = starter_grid
            .blocks
            .first()
            .expect("starter block is present");
        let cargo_inventory_id = starter_grid
            .blocks
            .iter()
            .find_map(|block| block.inventory_id.as_deref())
            .expect("starter grid has canonical cargo");

        let denied_inventory_intents = [
            (
                ClientMessage::RefineOre {
                    operation_sequence: 1,
                    operation_id: "deny-remote-refine-primary".into(),
                    inventory_id: primary.inventory_id.clone(),
                    batches: 1,
                },
                "physical_machine_required",
            ),
            (
                ClientMessage::CraftComponent {
                    operation_sequence: 1,
                    operation_id: "deny-remote-craft-primary".into(),
                    inventory_id: primary.inventory_id.clone(),
                    quantity: 1,
                },
                "physical_machine_required",
            ),
            (
                ClientMessage::QueueProduction {
                    operation_sequence: 1,
                    operation_id: "deny-remote-queue-primary".into(),
                    machine_block_id: "block-refinery".into(),
                    recipe: ProductionRecipeKind::Refining,
                    batches: 1,
                    source_inventory_id: cargo_inventory_id.into(),
                    destination_inventory_id: cargo_inventory_id.into(),
                },
                "grid_access_denied",
            ),
            (
                ClientMessage::TransferInventory {
                    operation_sequence: 1,
                    operation_id: "deny-remote-withdraw-primary".into(),
                    source_inventory_id: primary.inventory_id.clone(),
                    destination_inventory_id: remote_player.inventory_id.clone(),
                    resource: ResourceKind::Component,
                    quantity: 1,
                },
                "inventory_access_denied",
            ),
            (
                ClientMessage::TransferInventory {
                    operation_sequence: 1,
                    operation_id: "deny-remote-deposit-primary".into(),
                    source_inventory_id: remote_player.inventory_id.clone(),
                    destination_inventory_id: primary.inventory_id.clone(),
                    resource: ResourceKind::Ore,
                    quantity: 1,
                },
                "inventory_access_denied",
            ),
            (
                ClientMessage::TransferInventory {
                    operation_sequence: 1,
                    operation_id: "deny-remote-withdraw-cargo".into(),
                    source_inventory_id: cargo_inventory_id.into(),
                    destination_inventory_id: remote_player.inventory_id.clone(),
                    resource: ResourceKind::Component,
                    quantity: 1,
                },
                "inventory_access_denied",
            ),
            (
                ClientMessage::TransferInventory {
                    operation_sequence: 1,
                    operation_id: "deny-remote-deposit-cargo".into(),
                    source_inventory_id: remote_player.inventory_id.clone(),
                    destination_inventory_id: cargo_inventory_id.into(),
                    resource: ResourceKind::Ore,
                    quantity: 1,
                },
                "inventory_access_denied",
            ),
        ];
        for (intent, expected_code) in denied_inventory_intents {
            assert_intent_rejected(&mut remote, intent, expected_code).await;
        }

        let denied_grid_intents = [
            ClientMessage::BuildBlock {
                operation_sequence: 1,
                operation_id: "deny-remote-build-primary-grid".into(),
                grid_id: starter_grid.grid_id.clone(),
                coordinate: starter_block.coordinate,
                kind: BlockKind::Structural,
                orientation: 0,
            },
            ClientMessage::WeldBlock {
                operation_sequence: 1,
                operation_id: "deny-remote-weld-primary-grid".into(),
                grid_id: starter_grid.grid_id.clone(),
                block_id: starter_block.block_id.clone(),
            },
            ClientMessage::SetGridControl {
                operation_sequence: 1,
                operation_id: "deny-remote-control-primary-grid".into(),
                grid_id: starter_grid.grid_id.clone(),
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                dampeners: true,
            },
            ClientMessage::ToggleGridAnchor {
                operation_sequence: 1,
                operation_id: "deny-remote-anchor-primary-grid".into(),
                grid_id: starter_grid.grid_id.clone(),
            },
        ];
        for intent in denied_grid_intents {
            assert_intent_rejected(&mut remote, intent, "grid_access_denied").await;
        }

        let after = state.snapshot();
        assert_eq!(after.event_sequence, before.event_sequence);
        assert_eq!(after.world_hash, before.world_hash);
        assert_eq!(after.inventories, before.inventories);
        assert_eq!(after.grids, before.grids);
        assert_eq!(after.conservation, before.conservation);

        remote.close(None).await.expect("remote socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn character_control_acknowledges_with_lightweight_atomic_motion_state() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(&mut socket, &local_player_hello("motion-state-test-client")).await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome { .. }
        ));
        let ServerMessage::Snapshot { snapshot } = receive_server_message(&mut socket).await else {
            panic!("compatible handshake must receive a full snapshot");
        };
        let movement_epoch = actor_player(&snapshot).movement_epoch;
        let operation_id = "player-control-1-1";
        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
                operation_sequence: 1,
                operation_id: operation_id.into(),
                movement_epoch,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::new(0.0, 0.0, -1.0),
                angular_input: verse_protocol::Vec3::new(0.0, 0.0, 0.5),
                boost: false,
                jump: false,
                dampeners: true,
            },
        )
        .await;
        let ServerMessage::IntentAccepted { receipt } = receive_server_message(&mut socket).await
        else {
            panic!("accepted character control must receive a receipt");
        };
        assert_eq!(receipt.operation_id, operation_id);
        let ServerMessage::InterestDelta { delta } = receive_server_message(&mut socket).await
        else {
            panic!("character control must publish a contiguous interest delta");
        };
        assert_eq!(delta.event_sequence, receipt.event_sequence);
        assert_eq!(actor_delta(&delta).last_received_input_sequence, 1);
        assert_eq!(actor_delta(&delta).last_processed_input_sequence, 0);
        assert_eq!(actor_delta(&delta).movement_epoch, movement_epoch);
        assert_eq!(delta.world_hash, state.snapshot().world_hash);
        assert_eq!(delta.interest.delta_sequence, 1);
        assert_eq!(
            delta.interest.previous_view_hash.as_deref(),
            Some(snapshot.interest.view_hash.as_str())
        );
        assert_eq!(delta.interest.baseline_id, snapshot.interest.baseline_id);
        let first_delta_hash = delta.interest.view_hash.clone();
        assert!(state.advance(17).expect("authoritative physics advances"));
        let ServerMessage::InterestDelta { delta } = receive_server_message(&mut socket).await
        else {
            panic!("consumed character control must publish an authoritative interest delta");
        };
        assert_eq!(actor_delta(&delta).last_received_input_sequence, 1);
        assert_eq!(actor_delta(&delta).last_processed_input_sequence, 1);
        assert!(actor_delta(&delta).linear_velocity.magnitude() > 0.0);
        assert_eq!(delta.interest.delta_sequence, 2);
        assert_eq!(
            delta.interest.previous_view_hash.as_deref(),
            Some(first_delta_hash.as_str())
        );
        socket.close(None).await.expect("test socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn websocket_carries_magnetic_preference_and_jump_through_authoritative_motion() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(
            &mut socket,
            &local_player_hello("p0.10-locomotion-test-client"),
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::Welcome { .. }
        ));
        let ServerMessage::Snapshot { snapshot } = receive_server_message(&mut socket).await else {
            panic!("compatible handshake must receive a full snapshot");
        };
        let player = actor_player(&snapshot);

        send_client_message(
            &mut socket,
            &ClientMessage::SetSuitMode {
                operation_sequence: 1,
                operation_id: "arm-boots-and-release-jetpack".into(),
                helmet_closed: player.helmet_closed,
                jetpack_enabled: false,
                magnetic_boots_enabled: true,
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::IntentAccepted { .. }
        ));
        let ServerMessage::InterestDelta { delta } = receive_server_message(&mut socket).await
        else {
            panic!("suit-mode intent must publish an authoritative interest delta");
        };
        let player = actor_delta(&delta);
        assert!(!player.jetpack_enabled);
        assert!(player.locomotion.magnetic_boots_enabled);
        assert_eq!(
            player.locomotion.kind,
            verse_protocol::LocomotionKind::Airborne
        );

        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
                operation_sequence: 2,
                operation_id: "airborne-jump-edge".into(),
                movement_epoch: player.movement_epoch,
                input_sequence: 1,
                linear_input: verse_protocol::Vec3::ZERO,
                angular_input: verse_protocol::Vec3::ZERO,
                boost: false,
                jump: true,
                dampeners: true,
            },
        )
        .await;
        assert!(matches!(
            receive_server_message(&mut socket).await,
            ServerMessage::IntentAccepted { .. }
        ));
        let ServerMessage::InterestDelta { delta } = receive_server_message(&mut socket).await
        else {
            panic!("jump input receipt must publish an interest delta");
        };
        assert_eq!(actor_delta(&delta).last_received_input_sequence, 1);
        assert_eq!(actor_delta(&delta).last_processed_input_sequence, 0);
        assert!(actor_delta(&delta).locomotion.magnetic_boots_enabled);
        assert!(state.advance(17).expect("authoritative jump edge advances"));
        let ServerMessage::InterestDelta { delta } = receive_server_message(&mut socket).await
        else {
            panic!("processed jump edge must publish authoritative interest");
        };
        assert_eq!(actor_delta(&delta).last_processed_input_sequence, 1);
        assert!(actor_delta(&delta).locomotion.jump_held);
        assert!(actor_delta(&delta).locomotion.magnetic_boots_enabled);
        assert_eq!(
            actor_delta(&delta).locomotion.kind,
            verse_protocol::LocomotionKind::Airborne
        );

        socket.close(None).await.expect("test socket closes");
        server.abort();
    }

    #[tokio::test]
    async fn http_world_is_always_a_no_store_spectator_projection_despite_spoofing() {
        for spoofed in [false, true] {
            let mut request = Request::builder().uri(if spoofed {
                "/api/v1/world?player_id=player-local&authentication=local_development"
            } else {
                "/api/v1/world"
            });
            if spoofed {
                request = request
                    .header("authorization", "Bearer forged-player-local")
                    .header("cookie", "player_id=player-local; role=player")
                    .header("origin", "http://localhost:3000")
                    .header("x-player-id", "player-local")
                    .header("x-forwarded-user", "player-local");
            }
            let response = test_app()
                .oneshot(request.body(Body::empty()).expect("request"))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers().get(CACHE_CONTROL),
                Some(&HeaderValue::from_static(DYNAMIC_CACHE_CONTROL))
            );
            let body = to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body");
            let snapshot: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
            assert_snapshot_audience(
                &serde_json::json!({ "type": "snapshot", "snapshot": snapshot }),
                None,
            );
        }
    }

    #[tokio::test]
    async fn registry_endpoint_is_public_complete_and_never_cacheable() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/registry")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CACHE_CONTROL),
            Some(&HeaderValue::from_static(DYNAMIC_CACHE_CONTROL))
        );
        let body = to_bytes(response.into_body(), 8 * 1024 * 1024)
            .await
            .expect("body");
        let message: ServerMessage = serde_json::from_slice(&body).expect("protocol registry");
        let ServerMessage::Registry {
            registry,
            universe_manifest,
        } = message
        else {
            panic!("registry endpoint must return the protocol registry document");
        };
        assert_eq!(registry.schema_version, CELESTIAL_REGISTRY_SCHEMA_VERSION);
        assert_eq!(
            universe_manifest.schema_version,
            UNIVERSE_MANIFEST_SCHEMA_VERSION
        );
        assert_eq!(
            registry.registry_hash,
            universe_manifest.celestial_registry_hash
        );
        assert!(!registry.bodies.is_empty());
    }

    #[tokio::test]
    async fn initial_and_requested_snapshots_are_session_private_and_convergent() {
        let (mut local, state, server) = connect_test_socket().await;
        let mut remote = connect_additional(&local).await;
        let mut spectator = connect_additional(&local).await;

        let local_initial =
            complete_session(&mut local, &local_player_hello("projection-local-initial")).await;
        let remote_initial = complete_session(
            &mut remote,
            &player_hello("projection-remote-initial", "player-remote"),
        )
        .await;
        let spectator_initial = complete_session(
            &mut spectator,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "projection-spectator-initial".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;

        assert_snapshot_audience(&local_initial, Some("player-local"));
        assert_snapshot_audience(&remote_initial, Some("player-remote"));
        assert_snapshot_audience(&spectator_initial, None);
        assert_ne!(
            local_initial["snapshot"]["interest"]["view_hash"],
            remote_initial["snapshot"]["interest"]["view_hash"]
        );
        assert_ne!(
            local_initial["snapshot"]["interest"]["view_hash"],
            spectator_initial["snapshot"]["interest"]["view_hash"]
        );
        assert_eq!(
            local_initial["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        assert_eq!(
            remote_initial["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        for candidate in [&remote_initial, &spectator_initial] {
            assert_eq!(
                candidate["snapshot"]["event_sequence"],
                local_initial["snapshot"]["event_sequence"]
            );
            assert_eq!(
                candidate["snapshot"]["world_hash"],
                local_initial["snapshot"]["world_hash"]
            );
        }
        assert_eq!(
            local_initial["snapshot"]["world_hash"],
            state.snapshot().world_hash
        );

        for socket in [&mut local, &mut remote, &mut spectator] {
            send_client_message(socket, &ClientMessage::RequestSnapshot).await;
        }
        let local_requested = receive_json_type(&mut local, "snapshot", false).await;
        let remote_requested = receive_json_type(&mut remote, "snapshot", false).await;
        let spectator_requested = receive_json_type(&mut spectator, "snapshot", false).await;
        assert_snapshot_audience(&local_requested, Some("player-local"));
        assert_snapshot_audience(&remote_requested, Some("player-remote"));
        assert_snapshot_audience(&spectator_requested, None);
        assert_eq!(
            local_requested["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        assert_eq!(
            remote_requested["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        for candidate in [&remote_requested, &spectator_requested] {
            assert_eq!(
                candidate["snapshot"]["event_sequence"],
                local_requested["snapshot"]["event_sequence"]
            );
            assert_eq!(
                candidate["snapshot"]["world_hash"],
                local_requested["snapshot"]["world_hash"]
            );
        }

        local.close(None).await.expect("local closes");
        remote.close(None).await.expect("remote closes");
        spectator.close(None).await.expect("spectator closes");
        server.abort();
    }

    #[tokio::test]
    async fn websocket_exact_retry_conflicts_and_diagnostic_id_reuse_are_safe() {
        let (mut local, state, server) = connect_test_socket().await;
        let mut spectator = connect_additional(&local).await;
        let local_initial =
            complete_session(&mut local, &local_player_hello("idempotency-local")).await;
        let spectator_initial = complete_session(
            &mut spectator,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "idempotency-spectator".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        assert_eq!(
            local_initial["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        assert_snapshot_audience(&spectator_initial, None);

        let first = suit_mode_intent(1, "reusable-diagnostic-id", false);
        send_client_message(&mut local, &first).await;
        let first_receipt = receive_intent_accepted(&mut local, 1, "reusable-diagnostic-id").await;
        let local_update = receive_json_type(&mut local, "snapshot", false).await;
        let spectator_update = receive_json_type(&mut spectator, "snapshot", true).await;
        assert_eq!(
            local_update["snapshot"]["actor_private"]["committed_operation_sequence"],
            1
        );
        assert_snapshot_audience(&spectator_update, None);
        let committed = state.snapshot();

        send_client_message(&mut local, &first).await;
        let retry_receipt = receive_intent_accepted(&mut local, 1, "reusable-diagnostic-id").await;
        assert_eq!(retry_receipt, first_receipt);
        assert_eq!(state.snapshot().event_sequence, committed.event_sequence);
        assert_eq!(state.snapshot().world_hash, committed.world_hash);
        assert_no_server_message(&mut local).await;
        assert_no_server_message(&mut spectator).await;

        assert_intent_rejected(
            &mut local,
            suit_mode_intent(1, "reusable-diagnostic-id", true),
            "operation_conflict",
        )
        .await;
        assert_intent_rejected(
            &mut local,
            suit_mode_intent(1, "changed-diagnostic-id", false),
            "operation_conflict",
        )
        .await;
        assert_eq!(state.snapshot().event_sequence, committed.event_sequence);
        assert_eq!(state.snapshot().world_hash, committed.world_hash);
        assert_no_server_message(&mut spectator).await;

        let second = suit_mode_intent(2, "reusable-diagnostic-id", true);
        send_client_message(&mut local, &second).await;
        let second_receipt = receive_intent_accepted(&mut local, 2, "reusable-diagnostic-id").await;
        assert!(second_receipt.event_sequence > first_receipt.event_sequence);
        let local_update = receive_json_type(&mut local, "snapshot", false).await;
        let spectator_update = receive_json_type(&mut spectator, "snapshot", true).await;
        assert_eq!(
            local_update["snapshot"]["actor_private"]["committed_operation_sequence"],
            2
        );
        assert_snapshot_audience(&spectator_update, None);
        assert_eq!(
            local_update["snapshot"]["world_hash"],
            spectator_update["snapshot"]["world_hash"]
        );

        local.close(None).await.expect("local closes");
        spectator.close(None).await.expect("spectator closes");
        server.abort();
    }

    #[tokio::test]
    async fn invalid_gap_and_gameplay_rejection_leave_sequence_reusable() {
        let (mut local, state, server) = connect_test_socket().await;
        let initial = complete_session(
            &mut local,
            &local_player_hello("reusable-rejected-frontier"),
        )
        .await;
        assert_eq!(
            initial["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        let before = state.snapshot();

        assert_intent_rejected(
            &mut local,
            suit_mode_intent(0, "invalid-zero", false),
            "operation_sequence_invalid",
        )
        .await;
        assert_intent_rejected(
            &mut local,
            suit_mode_intent(2, "invalid-gap", false),
            "operation_sequence_gap",
        )
        .await;
        assert_intent_rejected(
            &mut local,
            suit_mode_intent(1, "rejected-no-change", true),
            "suit_mode_no_change",
        )
        .await;
        assert_eq!(state.snapshot().event_sequence, before.event_sequence);
        assert_eq!(state.snapshot().world_hash, before.world_hash);

        let corrected = suit_mode_intent(1, "corrected-one", false);
        send_client_message(&mut local, &corrected).await;
        let receipt = receive_intent_accepted(&mut local, 1, "corrected-one").await;
        assert_eq!(receipt.event_sequence, before.event_sequence + 1);
        let update = receive_json_type(&mut local, "snapshot", false).await;
        assert_eq!(
            update["snapshot"]["actor_private"]["committed_operation_sequence"],
            1
        );

        local.close(None).await.expect("local closes");
        server.abort();
    }

    #[tokio::test]
    async fn compacted_retry_is_denied_and_actor_frontiers_remain_private() {
        let (mut local, state, server) = connect_test_socket().await;
        complete_session(&mut local, &local_player_hello("compaction-local")).await;

        for operation_sequence in 1..=129 {
            let operation_id = format!("compacted-{operation_sequence}");
            let intent = suit_mode_intent(
                operation_sequence,
                &operation_id,
                operation_sequence % 2 == 0,
            );
            send_client_message(&mut local, &intent).await;
            receive_intent_accepted(&mut local, operation_sequence, &operation_id).await;
        }
        send_client_message(&mut local, &ClientMessage::RequestSnapshot).await;
        let local_snapshot = receive_until(&mut local, |message| {
            matches!(
                message,
                ServerMessage::Snapshot { snapshot }
                    if snapshot
                        .actor_private
                        .as_ref()
                        .is_some_and(|private| private.committed_operation_sequence == 129)
            )
        })
        .await;
        let ServerMessage::Snapshot {
            snapshot: local_snapshot,
        } = local_snapshot
        else {
            unreachable!("snapshot predicate matched")
        };
        assert_eq!(local_snapshot.event_sequence, 129);

        let mut remote = connect_additional(&local).await;
        let mut spectator = connect_additional(&local).await;
        let remote_snapshot = complete_session(
            &mut remote,
            &player_hello("compaction-remote", "player-remote"),
        )
        .await;
        let spectator_snapshot = complete_session(
            &mut spectator,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "compaction-spectator".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        assert_eq!(
            remote_snapshot["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        assert_snapshot_audience(&spectator_snapshot, None);
        for candidate in [&remote_snapshot, &spectator_snapshot] {
            assert_eq!(candidate["snapshot"]["event_sequence"], 129);
            assert_eq!(
                candidate["snapshot"]["world_hash"],
                local_snapshot.world_hash
            );
        }

        local.close(None).await.expect("first local session closes");
        tokio::time::sleep(Duration::from_millis(25)).await;
        let mut local = connect_additional(&remote).await;
        let local_reconnect = complete_session(
            &mut local,
            &local_player_hello("compaction-local-reconnect"),
        )
        .await;
        assert_eq!(
            local_reconnect["snapshot"]["actor_private"]["committed_operation_sequence"],
            129
        );
        assert_eq!(local_reconnect["snapshot"]["event_sequence"], 129);
        assert_eq!(
            local_reconnect["snapshot"]["world_hash"],
            local_snapshot.world_hash
        );

        assert_intent_rejected(
            &mut local,
            suit_mode_intent(1, "compacted-1", false),
            "operation_already_committed",
        )
        .await;
        assert_eq!(state.snapshot().event_sequence, 129);
        assert_no_server_message(&mut remote).await;
        assert_no_server_message(&mut spectator).await;

        local.close(None).await.expect("local closes");
        remote.close(None).await.expect("remote closes");
        spectator.close(None).await.expect("spectator closes");
        server.abort();
    }

    #[tokio::test]
    async fn live_structural_and_motion_updates_project_per_session_without_receipt_leaks() {
        let (mut local, state, server) = connect_test_socket().await;
        let mut remote = connect_additional(&local).await;
        let mut spectator = connect_additional(&local).await;
        let local_initial = complete_session(&mut local, &local_player_hello("live-local")).await;
        let _remote_initial =
            complete_session(&mut remote, &player_hello("live-remote", "player-remote")).await;
        let _spectator_initial = complete_session(
            &mut spectator,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "live-spectator".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;

        send_client_message(
            &mut local,
            &ClientMessage::TransferInventory {
                operation_sequence: 1,
                operation_id: "private-cargo-transfer".into(),
                source_inventory_id: "inventory-player-local".into(),
                destination_inventory_id: "inventory-cargo-starter".into(),
                resource: ResourceKind::Component,
                quantity: 7,
            },
        )
        .await;
        assert!(matches!(
            receive_until(&mut local, |message| matches!(message, ServerMessage::IntentAccepted { receipt } if receipt.operation_id == "private-cargo-transfer")).await,
            ServerMessage::IntentAccepted { .. }
        ));
        let local_structural = receive_json_type(&mut local, "snapshot", false).await;
        let remote_structural = receive_json_type(&mut remote, "snapshot", true).await;
        let spectator_structural = receive_json_type(&mut spectator, "snapshot", true).await;
        assert_snapshot_audience(&local_structural, Some("player-local"));
        assert_public_fields_are_redacted(&remote_structural["snapshot"]);
        assert!(remote_structural["snapshot"].get("actor_private").is_none());
        assert_snapshot_audience(&spectator_structural, None);
        assert_eq!(
            local_structural["snapshot"]["actor_private"]["committed_operation_sequence"],
            1
        );
        let local_inventories = local_structural["snapshot"]["actor_private"]["inventories"]
            .as_array()
            .expect("local private inventories");
        let inventory_components = |inventory_id: &str| {
            local_inventories
                .iter()
                .find(|inventory| inventory["inventory_id"] == inventory_id)
                .unwrap_or_else(|| panic!("missing private inventory {inventory_id}"))["contents"]
                ["components"]
                .as_u64()
                .expect("component quantity")
        };
        assert_eq!(inventory_components("inventory-player-local"), 17);
        assert_eq!(inventory_components("inventory-cargo-starter"), 7);
        let authoritative = state.snapshot();
        for candidate in [&local_structural, &remote_structural, &spectator_structural] {
            assert_eq!(
                candidate["snapshot"]["event_sequence"],
                authoritative.event_sequence
            );
            assert_eq!(
                candidate["snapshot"]["world_hash"],
                authoritative.world_hash
            );
        }

        send_client_message(
            &mut local,
            &ClientMessage::SetPlayerControl {
                operation_sequence: 2,
                operation_id: "private-motion-control".into(),
                movement_epoch:
                    local_initial["snapshot"]["actor_private"]["player"]["movement_epoch"]
                        .as_u64()
                        .expect("movement epoch"),
                input_sequence: 1,
                linear_input: Vec3::new(0.0, 0.0, -1.0),
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            },
        )
        .await;
        assert!(matches!(
            receive_until(&mut local, |message| matches!(message, ServerMessage::IntentAccepted { receipt } if receipt.operation_id == "private-motion-control")).await,
            ServerMessage::IntentAccepted { .. }
        ));
        let local_motion = receive_json_type(&mut local, "snapshot", false).await;
        let remote_motion = receive_json_type(&mut remote, "snapshot", true).await;
        let spectator_motion = receive_json_type(&mut spectator, "snapshot", true).await;
        assert_snapshot_audience(&local_motion, Some("player-local"));
        assert_public_fields_are_redacted(&remote_motion["snapshot"]);
        assert!(remote_motion["snapshot"].get("actor_private").is_none());
        assert_snapshot_audience(&spectator_motion, None);
        let authoritative = state.snapshot();
        for candidate in [&local_motion, &remote_motion, &spectator_motion] {
            assert_eq!(
                candidate["snapshot"]["event_sequence"],
                authoritative.event_sequence
            );
            assert_eq!(
                candidate["snapshot"]["world_hash"],
                authoritative.world_hash
            );
        }

        for socket in [&mut local, &mut remote, &mut spectator] {
            send_client_message(socket, &ClientMessage::RequestSnapshot).await;
        }
        let local_final = receive_json_type(&mut local, "snapshot", false).await;
        let remote_final = receive_json_type(&mut remote, "snapshot", true).await;
        let spectator_final = receive_json_type(&mut spectator, "snapshot", true).await;
        assert_eq!(
            local_final["snapshot"]["actor_private"]["committed_operation_sequence"],
            2
        );
        assert_eq!(
            remote_final["snapshot"]["actor_private"]["committed_operation_sequence"],
            0
        );
        assert_snapshot_audience(&spectator_final, None);

        local.close(None).await.expect("local closes");
        remote.close(None).await.expect("remote closes");
        spectator.close(None).await.expect("spectator closes");
        server.abort();
    }

    #[tokio::test]
    async fn death_drop_and_cargo_remain_private_across_sessions_and_reconnect() {
        let (mut local, state, server) = connect_test_socket().await;
        state
            .execute_as(
                "player-local",
                &ClientMessage::SetSuitMode {
                    operation_sequence: 1,
                    operation_id: "prepare-private-drop".into(),
                    helmet_closed: false,
                    jetpack_enabled: true,
                    magnetic_boots_enabled: false,
                },
            )
            .expect("helmet opens");
        for _ in 0..100 {
            state.advance(250).expect("vacuum life support advances");
        }
        let canonical = state.snapshot();
        assert_eq!(canonical.death_drops.len(), 1);
        let drop_inventory_id = canonical.death_drops[0].inventory_id.clone();
        let drop_id = canonical.death_drops[0].drop_id.clone();

        let mut remote = connect_additional(&local).await;
        let mut spectator = connect_additional(&local).await;
        let local_initial = complete_session(&mut local, &local_player_hello("drop-local")).await;
        let remote_initial =
            complete_session(&mut remote, &player_hello("drop-remote", "player-remote")).await;
        let spectator_initial = complete_session(
            &mut spectator,
            &ClientMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
                client_name: "drop-spectator".into(),
                authentication: ClientAuthentication::Spectator,
            },
        )
        .await;
        assert_snapshot_audience(&local_initial, Some("player-local"));
        assert_snapshot_audience(&remote_initial, Some("player-remote"));
        assert_snapshot_audience(&spectator_initial, None);
        let private = &local_initial["snapshot"]["actor_private"];
        assert_eq!(private["death_drops"][0]["drop_id"], drop_id);
        assert!(
            private["inventories"]
                .as_array()
                .expect("private inventories")
                .iter()
                .any(|inventory| inventory["inventory_id"] == drop_inventory_id)
        );
        for foreign in [&remote_initial, &spectator_initial] {
            let encoded = serde_json::to_string(foreign).expect("message serializes");
            assert!(
                foreign["snapshot"]["death_drops"]
                    .as_array()
                    .expect("public death drops")
                    .iter()
                    .any(|drop| drop["drop_id"] == drop_id),
                "a nearby salvage marker is public"
            );
            assert!(!encoded.contains(&drop_inventory_id));
        }

        local.close(None).await.expect("local closes");
        wait_for_player_release(&state, "player-local").await;
        let mut reconnected = connect_additional(&remote).await;
        let reconnect_snapshot = complete_session(
            &mut reconnected,
            &local_player_hello("drop-local-reconnect"),
        )
        .await;
        assert_snapshot_audience(&reconnect_snapshot, Some("player-local"));
        assert_eq!(
            reconnect_snapshot["snapshot"]["actor_private"]["death_drops"][0]["drop_id"],
            drop_id
        );

        reconnected
            .close(None)
            .await
            .expect("reconnected local closes");
        remote.close(None).await.expect("remote closes");
        spectator.close(None).await.expect("spectator closes");
        server.abort();
    }

    #[test]
    fn invalid_projection_audience_fails_without_a_canonical_fallback() {
        let state = test_state();
        let binding = SessionBinding::Player("player-not-admitted".into());
        assert!(projected_snapshot_message(&state, &binding).is_err());
        assert!(
            projected_replication_message(&state, &binding, PendingReplication::Motion).is_err()
        );
    }

    #[test]
    fn client_names_are_bounded_before_they_reach_connection_logging() {
        assert!(valid_client_name("native-client"));
        assert!(!valid_client_name(""));
        assert!(!valid_client_name(&"x".repeat(MAX_CLIENT_NAME_BYTES + 1)));
        assert!(!valid_client_name("line\nbreak"));
        assert!(!valid_client_name("non-ascii-é"));
    }

    #[tokio::test]
    async fn status_reports_conserved_genesis() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(CACHE_CONTROL),
            Some(&HeaderValue::from_static(DYNAMIC_CACHE_CONTROL))
        );
        let body = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
        assert_eq!(json["conservation_valid"], true);
        assert_eq!(json["protocol_version"], PROTOCOL_VERSION);
    }

    #[tokio::test]
    async fn health_endpoint_is_ready() {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn life_support_progress_is_independent_of_spoofable_client_names() {
        let directory = tempdir().expect("tempdir");
        let state = AppState::new(Runtime::open(directory.path(), 99, 20).expect("runtime"));
        for _ in 0..8 {
            state.advance(250).expect("authoritative tick");
        }
        assert_eq!(state.snapshot().player.suit_oxygen_milli, 990);
    }

    #[tokio::test]
    async fn command_center_assets_are_served_by_the_game_server() {
        for (uri, expected_content_type) in [
            ("/", "text/html"),
            ("/app.js", "text/javascript"),
            ("/styles.css", "text/css"),
            ("/verifier-worker.js", "text/javascript"),
            ("/verifier-worker-core.js", "text/javascript"),
        ] {
            let response = test_app()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            assert!(
                response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.starts_with(expected_content_type))
            );
            assert_eq!(
                response.headers().get(CACHE_CONTROL),
                Some(&HeaderValue::from_static("no-store"))
            );
        }
    }

    #[tokio::test]
    async fn generated_browser_verifier_routes_are_typed_and_never_cached() {
        for (uri, expected_content_type) in [
            ("/generated/verse_interest_verifier.js", "text/javascript"),
            (
                "/generated/verse_interest_verifier_bg.wasm",
                "application/wasm",
            ),
        ] {
            let response = test_app()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::OK);
            assert!(
                response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|value| value.starts_with(expected_content_type))
            );
            assert_eq!(
                response.headers().get(CACHE_CONTROL),
                Some(&HeaderValue::from_static("no-store"))
            );
        }
    }
}
