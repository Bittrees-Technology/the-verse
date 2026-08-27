// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

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
use tokio::sync::watch;
use tower_http::{
    cors::CorsLayer,
    trace::{DefaultMakeSpan, TraceLayer},
};
use tracing::{error, info, warn};
use verse_protocol::{
    ClientAuthentication, ClientMessage, IntentReceipt, MotionSnapshot, PROTOCOL_VERSION,
    ProjectedMotionSnapshot, ProjectedWorldSnapshot, ServerMessage, SessionRole, WorldSnapshot,
};
use verse_simulation::{IntentError, ProjectionError, Runtime, RuntimeError};

const COMMAND_CENTER_HTML: &str = include_str!("../../../apps/web-command-center/index.html");
const COMMAND_CENTER_JS: &str = include_str!("../../../apps/web-command-center/app.js");
const COMMAND_CENTER_CSS: &str = include_str!("../../../apps/web-command-center/styles.css");
const REPLICATION_PERIOD: Duration = Duration::from_nanos(16_666_667);
const DYNAMIC_CACHE_CONTROL: &str = "no-store";
const MAX_CLIENT_NAME_BYTES: usize = 128;

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
        if matches!(message, ServerMessage::Snapshot { .. }) {
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
        ServerMessage::Snapshot { snapshot } => Some(snapshot.event_sequence),
        ServerMessage::MotionState { motion } => Some(motion.event_sequence),
        _ => None,
    }
}

#[derive(Debug)]
pub struct AppState {
    runtime: Mutex<Runtime>,
    updates: watch::Sender<ReplicationFeed>,
    connected_players: Mutex<BTreeSet<String>>,
}

impl AppState {
    pub fn new(runtime: Runtime) -> Arc<Self> {
        let (updates, _) = watch::channel(ReplicationFeed::default());
        Arc::new(Self {
            runtime: Mutex::new(runtime),
            updates,
            connected_players: Mutex::new(BTreeSet::new()),
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

    fn projected_motion_snapshot(
        &self,
        actor_player_id: Option<&str>,
    ) -> Result<ProjectedMotionSnapshot, ProjectionError> {
        self.runtime
            .lock()
            .state()
            .project_motion_snapshot(actor_player_id)
    }

    pub fn persist_snapshot(&self) -> Result<(), RuntimeError> {
        self.runtime.lock().persist_snapshot()
    }

    pub fn is_halted(&self) -> bool {
        self.runtime.lock().is_halted()
    }

    pub fn advance(&self, delta_millis: u16) -> Result<bool, RuntimeError> {
        let mut runtime = self.runtime.lock();
        let before_lifecycle = runtime
            .state()
            .player
            .iter()
            .map(|(player_id, player)| {
                (
                    player_id.clone(),
                    player.life_state.clone(),
                    player.suit_oxygen_milli,
                )
            })
            .collect::<Vec<_>>();
        let changed = runtime.advance(delta_millis)?;
        if changed {
            let lifecycle_changed = runtime
                .state()
                .player
                .iter()
                .map(|(player_id, player)| {
                    (
                        player_id.clone(),
                        player.life_state.clone(),
                        player.suit_oxygen_milli,
                    )
                })
                .ne(before_lifecycle);
            let update_kind = if lifecycle_changed {
                ReplicationKind::Structural
            } else {
                ReplicationKind::Motion
            };
            self.publish_update(update_kind, runtime.state().event_sequence);
        }
        Ok(changed)
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
        self.publish_update(update_kind, runtime.state().event_sequence);
        Ok(receipt)
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
        .route("/healthz", get(health))
        .route("/api/v1/status", get(status))
        .route("/api/v1/world", get(world))
        .route("/ws", get(websocket_upgrade))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(false)),
        )
        .layer(cors)
        .with_state(state)
}

async fn command_center() -> Html<&'static str> {
    Html(COMMAND_CENTER_HTML)
}

async fn command_center_javascript() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/javascript; charset=utf-8")],
        COMMAND_CENTER_JS,
    )
}

async fn command_center_styles() -> impl IntoResponse {
    (
        [(CONTENT_TYPE, "text/css; charset=utf-8")],
        COMMAND_CENTER_CSS,
    )
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
    match state.projected_snapshot(None) {
        Ok(snapshot) => no_store(Json(snapshot).into_response()),
        Err(_) => no_store(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "public world projection is unavailable",
            )
                .into_response(),
        ),
    }
}

async fn websocket_upgrade(
    upgrade: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> Response {
    upgrade
        .max_message_size(8 * 1024 * 1024)
        .on_upgrade(move |socket| websocket_session(socket, state))
}

async fn websocket_session(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let Some((client_name, binding)) = complete_handshake(&mut receiver, &mut sender, &state).await
    else {
        return;
    };
    info!(%client_name, role = ?binding, "client completed protocol handshake");

    // Subscribe before projecting so a mutation between projection and delivery
    // is retained as a canonical marker and cannot be missed by this session.
    let mut updates = state.updates.subscribe();
    let Ok(initial_message) = projected_snapshot_message(&state, &binding) else {
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
                let pending = updates.borrow_and_update().next_after(replication_cursor);
                let Some(pending) = pending else {
                    continue;
                };
                let Ok(message) = projected_replication_message(&state, &binding, pending) else {
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
    loop {
        let message = match receiver.next().await {
            Some(Ok(message)) => message,
            Some(Err(source)) => {
                warn!(%source, "websocket handshake receive failed");
                return None;
            }
            None => return None,
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
                if sender.send(Message::Pong(payload)).await.is_err() {
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
            let Ok(message) = projected_snapshot_message(state, binding) else {
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
                    send_intent_error(sender, intent.operation_id(), source).await
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
    operation_id: Option<&str>,
    source: IntentError,
) -> bool {
    let message = source.message();
    send_server_message(
        sender,
        &ServerMessage::IntentRejected {
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
    let text = serde_json::to_string(message).map_err(axum::Error::new)?;
    sender.send(Message::Text(text.into())).await
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
    let _ = sender.send(Message::Close(None)).await;
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
    use verse_protocol::{BlockKind, ResourceKind, Vec3};

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

    async fn receive_server_message(socket: &mut TestSocket) -> ServerMessage {
        serde_json::from_value(receive_server_json(socket).await)
            .expect("server message is valid protocol JSON")
    }

    async fn receive_server_json(socket: &mut TestSocket) -> serde_json::Value {
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

    async fn receive_json_type(
        socket: &mut TestSocket,
        expected_type: &str,
        forbid_receipt: bool,
    ) -> serde_json::Value {
        for _ in 0..16 {
            let message = receive_server_json(socket).await;
            if forbid_receipt {
                assert_ne!(
                    message["type"], "intent_accepted",
                    "another session must never receive an actor's intent receipt"
                );
            }
            if message["type"] == expected_type {
                return message;
            }
        }
        panic!("expected {expected_type} did not arrive within the bounded queue");
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

    async fn complete_session(socket: &mut TestSocket, hello: &ClientMessage) -> serde_json::Value {
        send_client_message(socket, hello).await;
        assert!(matches!(
            receive_server_message(socket).await,
            ServerMessage::Welcome { .. }
        ));
        receive_json_type(socket, "snapshot", false).await
    }

    fn assert_public_fields_are_redacted(snapshot: &serde_json::Value) {
        assert!(snapshot.get("inventories").is_none());
        assert!(snapshot.get("death_drops").is_none());
        assert!(snapshot.get("conservation").is_none());
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
                for secret in [
                    "inventory-player-local",
                    "inventory-player-remote",
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
                assert_eq!(
                    private["player"]["inventory_id"],
                    format!("inventory-{player_id}")
                );
                if player_id == "player-local" {
                    assert!(!encoded.contains("inventory-player-remote"));
                } else {
                    for secret in [
                        "inventory-player-local",
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

    fn assert_motion_audience(message: &serde_json::Value, expected_actor: Option<&str>) {
        assert_eq!(message["type"], "motion_state");
        let motion = &message["motion"];
        for player in motion["players"].as_array().expect("public motion roster") {
            for private_field in [
                "movement_epoch",
                "last_received_input_sequence",
                "last_processed_input_sequence",
                "control_linear_input",
                "locomotion",
            ] {
                assert!(
                    player.get(private_field).is_none(),
                    "public motion leaked {private_field}"
                );
            }
        }
        match expected_actor {
            None => assert!(motion.get("actor_private").is_none()),
            Some(player_id) => assert_eq!(motion["actor_private"]["player_id"], player_id),
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
        send_client_message(socket, &intent).await;
        let response = receive_server_message(socket).await;
        assert!(
            matches!(
                &response,
            ServerMessage::IntentRejected {
                operation_id: Some(candidate),
                code,
                ..
            } if candidate == &operation_id && code == expected_code
            ),
            "{operation_id} must fail closed with {expected_code}; received {response:?}"
        );
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

    fn actor_player(snapshot: &ProjectedWorldSnapshot) -> &verse_protocol::PlayerSnapshot {
        &snapshot
            .actor_private
            .as_ref()
            .expect("authenticated player snapshot has an actor-private view")
            .player
    }

    fn actor_motion(motion: &ProjectedMotionSnapshot) -> &verse_protocol::PlayerMotionSnapshot {
        motion
            .actor_private
            .as_ref()
            .expect("authenticated player motion has an actor-private view")
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
    fn idempotent_retry_does_not_schedule_another_complete_snapshot() {
        let state = test_state();
        let mut observer = state.updates.subscribe();
        let player = state.snapshot().player;
        let intent = ClientMessage::SetSuitMode {
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
    async fn websocket_requires_hello_before_snapshot_or_mutation() {
        let (mut socket, state, server) = connect_test_socket().await;
        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
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
            ClientMessage::RefineOre {
                operation_id: "deny-remote-refine-primary".into(),
                inventory_id: primary.inventory_id.clone(),
                batches: 1,
            },
            ClientMessage::CraftComponent {
                operation_id: "deny-remote-craft-primary".into(),
                inventory_id: primary.inventory_id.clone(),
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_id: "deny-remote-withdraw-primary".into(),
                source_inventory_id: primary.inventory_id.clone(),
                destination_inventory_id: remote_player.inventory_id.clone(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_id: "deny-remote-deposit-primary".into(),
                source_inventory_id: remote_player.inventory_id.clone(),
                destination_inventory_id: primary.inventory_id.clone(),
                resource: ResourceKind::Ore,
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_id: "deny-remote-withdraw-cargo".into(),
                source_inventory_id: cargo_inventory_id.into(),
                destination_inventory_id: remote_player.inventory_id.clone(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_id: "deny-remote-deposit-cargo".into(),
                source_inventory_id: remote_player.inventory_id.clone(),
                destination_inventory_id: cargo_inventory_id.into(),
                resource: ResourceKind::Ore,
                quantity: 1,
            },
        ];
        for intent in denied_inventory_intents {
            assert_intent_rejected(&mut remote, intent, "inventory_access_denied").await;
        }

        let denied_grid_intents = [
            ClientMessage::BuildBlock {
                operation_id: "deny-remote-build-primary-grid".into(),
                grid_id: starter_grid.grid_id.clone(),
                coordinate: starter_block.coordinate,
                kind: BlockKind::Structural,
                orientation: 0,
            },
            ClientMessage::WeldBlock {
                operation_id: "deny-remote-weld-primary-grid".into(),
                grid_id: starter_grid.grid_id.clone(),
                block_id: starter_block.block_id.clone(),
            },
            ClientMessage::SetGridControl {
                operation_id: "deny-remote-control-primary-grid".into(),
                grid_id: starter_grid.grid_id.clone(),
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                dampeners: true,
            },
            ClientMessage::ToggleGridAnchor {
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
        let ServerMessage::MotionState { motion } = receive_server_message(&mut socket).await
        else {
            panic!("character control must publish lightweight motion state");
        };
        assert_eq!(motion.event_sequence, receipt.event_sequence);
        assert_eq!(actor_motion(&motion).last_received_input_sequence, 1);
        assert_eq!(actor_motion(&motion).last_processed_input_sequence, 0);
        assert_eq!(actor_motion(&motion).movement_epoch, movement_epoch);
        assert_eq!(motion.grids.len(), snapshot.grids.len());
        assert_eq!(motion.world_hash, state.snapshot().world_hash);
        assert!(state.advance(17).expect("authoritative physics advances"));
        let ServerMessage::MotionState { motion } = receive_server_message(&mut socket).await
        else {
            panic!("consumed character control must publish authoritative motion state");
        };
        assert_eq!(actor_motion(&motion).last_received_input_sequence, 1);
        assert_eq!(actor_motion(&motion).last_processed_input_sequence, 1);
        assert!(actor_motion(&motion).linear_velocity.magnitude() > 0.0);
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
        let ServerMessage::Snapshot { snapshot } = receive_server_message(&mut socket).await else {
            panic!("suit-mode intent must publish a complete authoritative snapshot");
        };
        let player = actor_player(&snapshot);
        assert!(!player.jetpack_enabled);
        assert!(player.locomotion.magnetic_boots_enabled);
        assert_eq!(
            player.locomotion.kind,
            verse_protocol::LocomotionKind::Airborne
        );

        send_client_message(
            &mut socket,
            &ClientMessage::SetPlayerControl {
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
        let ServerMessage::MotionState { motion } = receive_server_message(&mut socket).await
        else {
            panic!("jump input receipt must publish lightweight authoritative motion");
        };
        assert_eq!(actor_motion(&motion).last_received_input_sequence, 1);
        assert_eq!(actor_motion(&motion).last_processed_input_sequence, 0);
        assert!(actor_motion(&motion).locomotion.magnetic_boots_enabled);
        assert!(state.advance(17).expect("authoritative jump edge advances"));
        let ServerMessage::MotionState { motion } = receive_server_message(&mut socket).await
        else {
            panic!("processed jump edge must publish authoritative motion");
        };
        assert_eq!(actor_motion(&motion).last_processed_input_sequence, 1);
        assert!(actor_motion(&motion).locomotion.jump_held);
        assert!(actor_motion(&motion).locomotion.magnetic_boots_enabled);
        assert_eq!(
            actor_motion(&motion).locomotion.kind,
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
        assert_snapshot_audience(&remote_structural, Some("player-remote"));
        assert_snapshot_audience(&spectator_structural, None);
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
        let local_motion = receive_json_type(&mut local, "motion_state", false).await;
        let remote_motion = receive_json_type(&mut remote, "motion_state", true).await;
        let spectator_motion = receive_json_type(&mut spectator, "motion_state", true).await;
        assert_motion_audience(&local_motion, Some("player-local"));
        assert_motion_audience(&remote_motion, Some("player-remote"));
        assert_motion_audience(&spectator_motion, None);
        let authoritative = state.snapshot();
        for candidate in [&local_motion, &remote_motion, &spectator_motion] {
            assert_eq!(
                candidate["motion"]["event_sequence"],
                authoritative.event_sequence
            );
            assert_eq!(candidate["motion"]["world_hash"], authoritative.world_hash);
        }

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
            assert!(!encoded.contains(&drop_id));
            assert!(!encoded.contains(&drop_inventory_id));
        }

        local.close(None).await.expect("local closes");
        tokio::time::sleep(Duration::from_millis(25)).await;
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
        }
    }
}
