// SPDX-License-Identifier: Apache-2.0

//! Clean-room verifier for protocol-18 interest baselines and deltas.
//!
//! The verifier consumes original UTF-8 JSON bytes. All connection and view
//! transitions are staged and require an opaque one-use token to commit.

mod canonical;
mod error;
mod registry;
mod strict_json;
#[cfg(any(test, all(feature = "browser-wasm", target_arch = "wasm32")))]
mod wasm_browser;

#[cfg(all(feature = "browser-wasm", target_arch = "wasm32"))]
pub use wasm_browser::BrowserInterestVerifier;

use std::collections::{BTreeMap, BTreeSet};

pub use error::{ErrorCode, VerifyError};
use serde::Serialize;
use verse_protocol::{
    ActorPrivateSnapshot, BlockKind, CELESTIAL_REGISTRY_SCHEMA_VERSION, CELL_KEY_SCHEMA_VERSION,
    CelestialRegistrySnapshot, CellKeyV1, EnvironmentSnapshot, HandoffPhase, HandoffStatus,
    I64Vec3, INTEREST_SCHEMA_VERSION, InterestEntityKind, InterestEntityPayload,
    InterestEntityProjection, InterestFrameKind, InterestObserverClass, InterestSnapshot,
    InterestTransferLink, InventoryDomain, InventorySnapshot, PROJECTION_SCHEMA_VERSION,
    PROTOCOL_VERSION, PlayerMotionSnapshot, ProductionRecipeKind, ProjectedInterestDelta,
    ProjectedWorldSnapshot, PublicBlockSnapshot, PublicDeathDropSnapshot, PublicGridSnapshot,
    PublicPlayerSnapshot, PublicVoxelChunkSnapshot, ServerMessage, SessionRole,
    UNIVERSE_MANIFEST_SCHEMA_VERSION, UniverseAddress, UniverseManifestSnapshot,
};

use crate::error::Result;

/// Configurable, deterministic denial-of-service limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceLimits {
    pub max_frame_bytes: usize,
    pub max_json_depth: usize,
    pub max_json_values: usize,
    pub max_collection_len: usize,
    pub max_string_bytes: usize,
    pub max_total_string_bytes: usize,
    pub max_entities: usize,
    pub max_blocks_per_grid: usize,
    pub max_voxels_per_chunk: usize,
    pub max_private_records: usize,
    /// Maximum immutable celestial records accepted in one registry frame.
    pub max_registry_bodies: usize,
    /// Maximum pairwise separation checks accepted for one registry frame.
    pub max_registry_pair_comparisons: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 8 * 1024 * 1024,
            max_json_depth: 64,
            max_json_values: 500_000,
            max_collection_len: 250_000,
            max_string_bytes: 256 * 1024,
            max_total_string_bytes: 4 * 1024 * 1024,
            max_entities: 50_000,
            max_blocks_per_grid: 100_000,
            max_voxels_per_chunk: 250_000,
            max_private_records: 100_000,
            max_registry_bodies: 512,
            max_registry_pair_comparisons: 130_816,
        }
    }
}

/// Client-selected compatibility and authority boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifierConfig {
    pub expected_role: SessionRole,
    pub world_schema_version: u32,
    pub event_schema_version: u32,
    pub content_schema_version: u32,
    pub content_manifest_version: String,
    pub expected_content_hash: String,
    pub expected_universe_id: String,
    pub expected_celestial_registry_hash: String,
    pub expected_universe_manifest_hash: String,
    pub limits: ResourceLimits,
}

impl VerifierConfig {
    #[allow(clippy::too_many_arguments)] // Every immutable trust root is intentionally explicit.
    pub fn new(
        expected_role: SessionRole,
        world_schema_version: u32,
        event_schema_version: u32,
        content_schema_version: u32,
        content_manifest_version: impl Into<String>,
        expected_content_hash: impl Into<String>,
        expected_universe_id: impl Into<String>,
        expected_celestial_registry_hash: impl Into<String>,
        expected_universe_manifest_hash: impl Into<String>,
    ) -> Self {
        Self {
            expected_role,
            world_schema_version,
            event_schema_version,
            content_schema_version,
            content_manifest_version: content_manifest_version.into(),
            expected_content_hash: expected_content_hash.into(),
            expected_universe_id: expected_universe_id.into(),
            expected_celestial_registry_hash: expected_celestial_registry_hash.into(),
            expected_universe_manifest_hash: expected_universe_manifest_hash.into(),
            limits: ResourceLimits::default(),
        }
    }
}

/// The kind of successfully staged transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageKind {
    Welcome,
    Registry,
    Handoff,
    Baseline,
    Delta,
    IntentAccepted,
    IntentRejected,
    Fatal,
}

/// Opaque, one-use capability for a pending transition.
#[derive(Debug, PartialEq, Eq)]
pub struct StageToken {
    generation: u64,
    sequence: u64,
}

/// Presentation-safe summary of a verified complete interest view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedView {
    pub session_epoch: String,
    pub interest_epoch: u64,
    pub baseline_id: String,
    pub delta_sequence: u64,
    pub view_hash: String,
    pub entity_count: usize,
    pub has_actor_private: bool,
}

/// Result of committing a valid pending transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitOutcome {
    pub kind: StageKind,
    /// Exact compact `ClientMessage::AcknowledgeInterest` JSON, when applicable.
    pub acknowledgement_json: Option<String>,
    pub view: Option<VerifiedView>,
}

#[derive(Debug, Clone)]
struct WelcomeBinding {
    world_schema_version: u32,
    event_schema_version: u32,
    content_schema_version: u32,
    content_manifest_version: String,
    role: SessionRole,
}

type RegistryBinding = registry::ValidatedRegistry;

#[derive(Debug, Clone)]
struct ViewState {
    content_manifest_version: String,
    universe_id: String,
    cell_id: String,
    universe_manifest_hash: String,
    celestial_registry_hash: String,
    cell_address: UniverseAddress,
    local_origin: UniverseAddress,
    gravity_body_id: String,
    voxel_body_id: String,
    observer_class: InterestObserverClass,
    session_epoch: String,
    interest_epoch: u64,
    baseline_id: String,
    delta_sequence: u64,
    canonical_event_sequence: u64,
    canonical_tick: u64,
    entities: Vec<InterestEntityProjection>,
    environment: EnvironmentSnapshot,
    conservation_valid: bool,
    actor_private: Option<ActorPrivateSnapshot>,
    transfer_link: Option<InterestTransferLink>,
    view_hash: String,
}

#[derive(Debug, Clone)]
struct HandoffBinding {
    status: HandoffStatus,
    session_epoch: String,
    prior_interest_epoch: u64,
}

impl ViewState {
    fn summary(&self) -> VerifiedView {
        VerifiedView {
            session_epoch: self.session_epoch.clone(),
            interest_epoch: self.interest_epoch,
            baseline_id: self.baseline_id.clone(),
            delta_sequence: self.delta_sequence,
            view_hash: self.view_hash.clone(),
            entity_count: self.entities.len(),
            has_actor_private: self.actor_private.is_some(),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CommittedState {
    welcome: Option<WelcomeBinding>,
    registry: Option<RegistryBinding>,
    view: Option<ViewState>,
    handoff: Option<HandoffBinding>,
}

#[derive(Debug, Clone)]
struct PendingState {
    token_generation: u64,
    token_sequence: u64,
    kind: StageKind,
    next: CommittedState,
    message: ServerMessage,
    sanitized_json: String,
}

/// Stateful verifier for one connection.
#[derive(Debug)]
pub struct InterestVerifier {
    config: VerifierConfig,
    committed: CommittedState,
    pending: Option<PendingState>,
    generation: u64,
    next_sequence: u64,
}

impl InterestVerifier {
    pub fn new(config: VerifierConfig) -> Result<Self> {
        validate_limits(&config.limits)?;
        if config.content_manifest_version.is_empty() {
            return Err(VerifyError::new(
                ErrorCode::IncompatibleWelcome,
                "configured content manifest version must be nonempty",
            ));
        }
        registry::validate_expected_commitments(
            &config.expected_universe_id,
            &config.expected_content_hash,
            &config.expected_celestial_registry_hash,
            &config.expected_universe_manifest_hash,
        )?;
        Ok(Self {
            config,
            committed: CommittedState::default(),
            pending: None,
            generation: 1,
            next_sequence: 1,
        })
    }

    /// Parse, validate, and hash one original wire frame without committing it.
    pub fn stage(&mut self, raw_json: &[u8]) -> Result<StageToken> {
        let message: ServerMessage = strict_json::parse_exact(raw_json, &self.config.limits)?;
        let handoff_supersedes_pending = matches!(
            &message,
            ServerMessage::Handoff {
                handoff: HandoffStatus {
                    phase: HandoffPhase::Preparing,
                    ..
                }
            }
        );
        if self.pending.is_some() && !handoff_supersedes_pending {
            return Err(VerifyError::new(
                ErrorCode::PendingStage,
                "a transition is already pending",
            ));
        }
        if self.pending.take().is_some() {
            // A source frame staged immediately before the authoritative
            // prepare boundary must never remain committable afterward.
            self.advance_generation();
        }
        let presentation_message = message.clone();
        let sanitized_json = serde_json::to_string(&presentation_message).map_err(|error| {
            VerifyError::new(
                ErrorCode::Serialization,
                format!("sanitized message serialization: {error}"),
            )
        })?;
        let mut next = self.committed.clone();
        let kind = match message {
            ServerMessage::Snapshot { .. } | ServerMessage::MotionState { .. } => {
                return Err(VerifyError::new(
                    ErrorCode::LegacyMessage,
                    "protocol-15 snapshot and motion_state messages are forbidden",
                ));
            }
            ServerMessage::Welcome {
                protocol_version,
                projection_schema_version,
                world_schema_version,
                event_schema_version,
                content_schema_version,
                content_manifest_version,
                celestial_registry_schema_version,
                universe_manifest_schema_version,
                interest_schema_version,
                session_role,
                ..
            } => {
                if self.committed.welcome.is_some() {
                    return Err(unexpected("welcome"));
                }
                self.validate_welcome(
                    protocol_version,
                    projection_schema_version,
                    world_schema_version,
                    event_schema_version,
                    content_schema_version,
                    &content_manifest_version,
                    celestial_registry_schema_version,
                    universe_manifest_schema_version,
                    interest_schema_version,
                    &session_role,
                )?;
                next.welcome = Some(WelcomeBinding {
                    world_schema_version,
                    event_schema_version,
                    content_schema_version,
                    content_manifest_version,
                    role: session_role,
                });
                StageKind::Welcome
            }
            ServerMessage::Registry {
                registry,
                universe_manifest,
            } => {
                let welcome = self
                    .committed
                    .welcome
                    .as_ref()
                    .ok_or_else(|| unexpected("registry"))?;
                if self.committed.registry.is_some() {
                    return Err(unexpected("registry"));
                }
                let binding =
                    validate_registry(welcome, &self.config, &registry, &universe_manifest)?;
                next.registry = Some(binding);
                StageKind::Registry
            }
            ServerMessage::Handoff { handoff } => {
                let welcome = self
                    .committed
                    .welcome
                    .as_ref()
                    .ok_or_else(|| unexpected("handoff"))?;
                let binding = self
                    .committed
                    .registry
                    .as_ref()
                    .ok_or_else(|| unexpected("handoff"))?;
                if !matches!(welcome.role, SessionRole::Player { .. }) {
                    return Err(unexpected("handoff"));
                }
                if let Some(current) = &self.committed.handoff {
                    validate_handoff_progress(current, &handoff)?;
                    next.handoff = Some(HandoffBinding {
                        status: handoff,
                        session_epoch: current.session_epoch.clone(),
                        prior_interest_epoch: current.prior_interest_epoch,
                    });
                } else {
                    let view = self
                        .committed
                        .view
                        .as_ref()
                        .ok_or_else(|| unexpected("handoff"))?;
                    validate_initial_handoff(&handoff, view, binding)?;
                    next.handoff = Some(HandoffBinding {
                        status: handoff,
                        session_epoch: view.session_epoch.clone(),
                        prior_interest_epoch: view.interest_epoch,
                    });
                }
                // The old cell stops being presentable as soon as preparation
                // is committed. No source delta or receipt can revive it.
                next.view = None;
                StageKind::Handoff
            }
            ServerMessage::InterestBaseline { baseline } => {
                let welcome = self
                    .committed
                    .welcome
                    .as_ref()
                    .ok_or_else(|| unexpected("baseline"))?;
                let binding = self
                    .committed
                    .registry
                    .as_ref()
                    .ok_or_else(|| unexpected("baseline"))?;
                let candidate = self.validate_baseline(welcome, binding, &baseline)?;
                if let Some(handoff) = &self.committed.handoff {
                    validate_destination_baseline(handoff, &candidate)?;
                    next.handoff = None;
                } else if candidate.transfer_link.is_some() {
                    return Err(VerifyError::new(
                        ErrorCode::InvalidBaseline,
                        "a transfer-linked baseline requires a committed handoff",
                    ));
                } else if let Some(current) = &self.committed.view {
                    validate_recovery_baseline(current, &candidate)?;
                }
                next.view = Some(candidate);
                StageKind::Baseline
            }
            ServerMessage::InterestDelta { delta } => {
                let welcome = self
                    .committed
                    .welcome
                    .as_ref()
                    .ok_or_else(|| unexpected("delta"))?;
                let binding = self
                    .committed
                    .registry
                    .as_ref()
                    .ok_or_else(|| unexpected("delta"))?;
                let current = self
                    .committed
                    .view
                    .as_ref()
                    .ok_or_else(|| unexpected("delta"))?;
                next.view = Some(self.validate_delta(welcome, binding, current, &delta)?);
                StageKind::Delta
            }
            ServerMessage::IntentAccepted { .. } => {
                if self.committed.view.is_none() {
                    return Err(unexpected("intent receipt"));
                }
                StageKind::IntentAccepted
            }
            ServerMessage::IntentRejected { .. } => {
                if self.committed.view.is_none() {
                    return Err(unexpected("intent rejection"));
                }
                StageKind::IntentRejected
            }
            ServerMessage::Fatal { .. } => StageKind::Fatal,
        };

        let token = StageToken {
            generation: self.generation,
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        self.pending = Some(PendingState {
            token_generation: token.generation,
            token_sequence: token.sequence,
            kind,
            next,
            message: presentation_message,
            sanitized_json,
        });
        Ok(token)
    }

    /// Return the complete staged view summary, if the pending frame has one.
    pub fn pending_view(&self) -> Option<VerifiedView> {
        self.pending
            .as_ref()
            .and_then(|pending| pending.next.view.as_ref())
            .map(ViewState::summary)
    }

    /// Typed, sanitized message associated with the pending transition.
    pub fn pending_message(&self) -> Option<&ServerMessage> {
        self.pending.as_ref().map(|pending| &pending.message)
    }

    /// Compact JSON reserialized from the typed pending message.
    pub fn pending_sanitized_json(&self) -> Option<&str> {
        self.pending
            .as_ref()
            .map(|pending| pending.sanitized_json.as_str())
    }

    pub fn pending_kind(&self) -> Option<StageKind> {
        self.pending.as_ref().map(|pending| pending.kind)
    }

    pub fn committed_view(&self) -> Option<VerifiedView> {
        self.committed.view.as_ref().map(ViewState::summary)
    }

    /// Atomically install the pending state and generate its verifier-owned ACK.
    #[allow(clippy::needless_pass_by_value)] // Consuming the token enforces one-use at the API boundary.
    pub fn commit(&mut self, token: StageToken) -> Result<CommitOutcome> {
        let StageToken {
            generation,
            sequence,
        } = token;
        let matches = self.pending.as_ref().is_some_and(|pending| {
            pending.token_generation == generation && pending.token_sequence == sequence
        });
        if !matches {
            return Err(VerifyError::new(
                ErrorCode::InvalidStageToken,
                "stage token is not valid for the pending verifier generation",
            ));
        }
        let acknowledgement_json = {
            let pending = self.pending.as_ref().expect("pending token was checked");
            if matches!(pending.kind, StageKind::Baseline | StageKind::Delta) {
                let view = pending
                    .next
                    .view
                    .as_ref()
                    .expect("baseline and delta install a view");
                let message = verse_protocol::ClientMessage::AcknowledgeInterest {
                    session_epoch: view.session_epoch.clone(),
                    interest_epoch: view.interest_epoch,
                    baseline_id: view.baseline_id.clone(),
                    delta_sequence: view.delta_sequence,
                    view_hash: view.view_hash.clone(),
                };
                Some(serde_json::to_string(&message).map_err(|error| {
                    VerifyError::new(
                        ErrorCode::Serialization,
                        format!("acknowledgement serialization: {error}"),
                    )
                })?)
            } else {
                None
            }
        };
        let pending = self.pending.take().expect("pending token was checked");
        self.committed = pending.next;
        self.advance_generation();
        let view = self.committed.view.as_ref().map(ViewState::summary);
        Ok(CommitOutcome {
            kind: pending.kind,
            acknowledgement_json,
            view,
        })
    }

    /// Discard a correctly identified pending transition without changing state.
    #[allow(clippy::needless_pass_by_value)] // Consuming the token enforces one-use at the API boundary.
    pub fn discard(&mut self, token: StageToken) -> Result<()> {
        let StageToken {
            generation,
            sequence,
        } = token;
        let matches = self.pending.as_ref().is_some_and(|pending| {
            pending.token_generation == generation && pending.token_sequence == sequence
        });
        if !matches {
            return Err(VerifyError::new(
                ErrorCode::InvalidStageToken,
                "stage token is not valid for the pending verifier generation",
            ));
        }
        self.pending = None;
        self.advance_generation();
        Ok(())
    }

    /// Clear all negotiated and view state, invalidating every prior token.
    pub fn reset(&mut self) {
        self.committed = CommittedState::default();
        self.pending = None;
        self.advance_generation();
    }

    fn advance_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1).max(1);
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_welcome(
        &self,
        protocol: u32,
        projection: u32,
        world: u32,
        event: u32,
        content: u32,
        content_manifest: &str,
        registry: u32,
        manifest: u32,
        interest: u32,
        role: &SessionRole,
    ) -> Result<()> {
        let compatible = protocol == PROTOCOL_VERSION
            && projection == PROJECTION_SCHEMA_VERSION
            && world == self.config.world_schema_version
            && event == self.config.event_schema_version
            && content == self.config.content_schema_version
            && registry == CELESTIAL_REGISTRY_SCHEMA_VERSION
            && manifest == UNIVERSE_MANIFEST_SCHEMA_VERSION
            && interest == INTEREST_SCHEMA_VERSION
            && role == &self.config.expected_role
            && content_manifest == self.config.content_manifest_version;
        if !compatible {
            return Err(VerifyError::new(
                ErrorCode::IncompatibleWelcome,
                "welcome protocol, schema, manifest, or role is incompatible",
            ));
        }
        Ok(())
    }

    fn validate_baseline(
        &self,
        welcome: &WelcomeBinding,
        binding: &RegistryBinding,
        baseline: &ProjectedWorldSnapshot,
    ) -> Result<ViewState> {
        validate_outer_baseline(welcome, binding, baseline)?;
        validate_interest_header(&baseline.interest, InterestFrameKind::Baseline, binding)?;
        repeated_baseline_headers(baseline)?;
        validate_transfer_link(
            baseline.interest.transfer_link.as_ref(),
            &baseline.cell_id,
            &baseline.interest.cell_address,
            &welcome.role,
            binding,
        )?;
        validate_role(
            welcome,
            baseline.interest.observer_class,
            baseline.actor_private.as_ref(),
            None,
        )?;
        if matches!(welcome.role, SessionRole::Player { .. }) && baseline.actor_private.is_none() {
            return Err(VerifyError::new(
                ErrorCode::InvalidBaseline,
                "a player baseline requires complete actor-private state",
            ));
        }

        let interest = &baseline.interest;
        if interest.delta_sequence != 0
            || interest.previous_view_hash.is_some()
            || interest.session_epoch.is_empty()
            || interest.baseline_id.is_empty()
            || !interest.replaced.is_empty()
            || !interest.removed.is_empty()
        {
            return Err(VerifyError::new(
                ErrorCode::InvalidBaseline,
                "baseline frontier or operation shape is invalid",
            ));
        }
        validate_entity_vector(&interest.entered, &self.config.limits, binding)?;
        validate_baseline_payload_arrays(baseline, &interest.entered)?;
        if let Some(private) = &baseline.actor_private {
            validate_actor_private(private, &self.config.limits, binding)?;
        }
        validate_private_linkage(
            &welcome.role,
            baseline.actor_private.as_ref(),
            &interest.entered,
        )?;

        let mut state = ViewState {
            content_manifest_version: baseline.content_manifest_version.clone(),
            universe_id: baseline.universe_id.clone(),
            cell_id: baseline.cell_id.clone(),
            universe_manifest_hash: baseline.universe_manifest_hash.clone(),
            celestial_registry_hash: baseline.celestial_registry_hash.clone(),
            cell_address: baseline.cell_address.clone(),
            local_origin: interest.local_origin_address.clone(),
            gravity_body_id: baseline.gravity_body_id.clone(),
            voxel_body_id: baseline.voxel_body_id.clone(),
            observer_class: interest.observer_class,
            session_epoch: interest.session_epoch.clone(),
            interest_epoch: interest.interest_epoch,
            baseline_id: interest.baseline_id.clone(),
            delta_sequence: 0,
            canonical_event_sequence: interest.canonical_event_sequence,
            canonical_tick: interest.canonical_tick,
            entities: interest.entered.clone(),
            environment: baseline.environment.clone(),
            conservation_valid: baseline.conservation_valid,
            actor_private: baseline.actor_private.clone(),
            transfer_link: interest.transfer_link.clone(),
            view_hash: String::new(),
        };
        state.view_hash = hash_view(&state)?;
        require_wire_hash(&interest.view_hash, &state.view_hash)?;
        Ok(state)
    }

    fn validate_delta(
        &self,
        welcome: &WelcomeBinding,
        binding: &RegistryBinding,
        current: &ViewState,
        delta: &ProjectedInterestDelta,
    ) -> Result<ViewState> {
        validate_outer_delta(welcome, binding, delta)?;
        validate_interest_header(&delta.interest, InterestFrameKind::Delta, binding)?;
        repeated_delta_headers(delta)?;
        validate_role(
            welcome,
            delta.interest.observer_class,
            delta.actor_private.as_ref(),
            delta.actor_private_motion.as_ref(),
        )?;
        validate_delta_frontier(current, &delta.interest)?;
        if delta.interest.transfer_link.is_some() {
            return Err(VerifyError::new(
                ErrorCode::InvalidDelta,
                "transfer linkage is valid only on a complete destination baseline",
            ));
        }
        if delta.cell_id != current.cell_id
            || delta.gravity_body_id != current.gravity_body_id
            || delta.voxel_body_id != current.voxel_body_id
        {
            return Err(VerifyError::new(
                ErrorCode::InvalidDelta,
                "delta cell or body binding differs from the committed view",
            ));
        }
        if delta.actor_private.is_some() && delta.actor_private_motion.is_some() {
            return Err(VerifyError::new(
                ErrorCode::InvalidDelta,
                "complete actor-private replacement and private motion are mutually exclusive",
            ));
        }

        validate_entity_vector(&delta.interest.entered, &self.config.limits, binding)?;
        validate_entity_vector(&delta.interest.replaced, &self.config.limits, binding)?;
        validate_removals(&delta.interest.removed, &self.config.limits)?;
        validate_disjoint_operations(&delta.interest, &self.config.limits)?;

        let mut entities: BTreeMap<EntityKey, InterestEntityProjection> = current
            .entities
            .iter()
            .cloned()
            .map(|entity| (entity_key(&entity), entity))
            .collect();
        for entity in &delta.interest.entered {
            let key = entity_key(entity);
            if entities.insert(key, entity.clone()).is_some() {
                return Err(VerifyError::new(
                    ErrorCode::InvalidDelta,
                    "enter names an existing identity",
                ));
            }
        }
        for replacement in &delta.interest.replaced {
            let key = entity_key(replacement);
            let Some(existing) = entities.get(&key) else {
                return Err(VerifyError::new(
                    ErrorCode::InvalidDelta,
                    "replacement names an absent identity",
                ));
            };
            if replacement.projected_revision <= existing.projected_revision {
                return Err(VerifyError::new(
                    ErrorCode::InvalidDelta,
                    "replacement projected revision did not increase",
                ));
            }
            entities.insert(key, replacement.clone());
        }
        for removal in &delta.interest.removed {
            if entities
                .remove(&(removal.entity_id.clone(), removal.kind))
                .is_none()
            {
                return Err(VerifyError::new(
                    ErrorCode::InvalidDelta,
                    "removal names an absent identity",
                ));
            }
        }
        if entities.len() > self.config.limits.max_entities {
            return Err(VerifyError::new(
                ErrorCode::ResourceLimit,
                "resulting complete entity set exceeds max_entities",
            ));
        }

        let mut actor_private = current.actor_private.clone();
        if let Some(replacement) = &delta.actor_private {
            validate_actor_private(replacement, &self.config.limits, binding)?;
            actor_private = Some(replacement.clone());
        } else if let Some(motion) = &delta.actor_private_motion {
            validate_private_motion(motion, binding)?;
            let Some(private) = actor_private.as_mut() else {
                return Err(VerifyError::new(
                    ErrorCode::InvalidDelta,
                    "private motion requires committed actor-private state",
                ));
            };
            if private.player.player_id != motion.player_id {
                return Err(VerifyError::new(
                    ErrorCode::InvalidDelta,
                    "private motion identity differs from committed private player",
                ));
            }
            apply_private_motion(private, motion);
        }

        let entities: Vec<_> = entities.into_values().collect();
        validate_private_linkage(&welcome.role, actor_private.as_ref(), &entities)?;

        let mut state = current.clone();
        state.delta_sequence = delta.interest.delta_sequence;
        state.canonical_event_sequence = delta.interest.canonical_event_sequence;
        state.canonical_tick = delta.interest.canonical_tick;
        state.local_origin = delta.interest.local_origin_address.clone();
        state.entities = entities;
        state.environment = delta
            .environment
            .clone()
            .unwrap_or_else(|| current.environment.clone());
        state.conservation_valid = delta
            .conservation_valid
            .unwrap_or(current.conservation_valid);
        state.actor_private = actor_private;
        state.transfer_link = None;
        state.view_hash.clear();
        state.view_hash = hash_view(&state)?;
        require_wire_hash(&delta.interest.view_hash, &state.view_hash)?;
        Ok(state)
    }
}

type EntityKey = (String, InterestEntityKind);

fn unexpected(message: &str) -> VerifyError {
    VerifyError::new(
        ErrorCode::UnexpectedMessage,
        format!("{message} is not valid in the current verifier state"),
    )
}

fn validate_limits(limits: &ResourceLimits) -> Result<()> {
    let values = [
        limits.max_frame_bytes,
        limits.max_json_depth,
        limits.max_json_values,
        limits.max_collection_len,
        limits.max_string_bytes,
        limits.max_total_string_bytes,
        limits.max_entities,
        limits.max_blocks_per_grid,
        limits.max_voxels_per_chunk,
        limits.max_private_records,
        limits.max_registry_bodies,
        limits.max_registry_pair_comparisons,
    ];
    if values.contains(&0) || limits.max_json_depth > 128 {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "all resource limits must be nonzero and max_json_depth must not exceed 128",
        ));
    }
    Ok(())
}

fn validate_registry(
    welcome: &WelcomeBinding,
    config: &VerifierConfig,
    registry: &CelestialRegistrySnapshot,
    manifest: &UniverseManifestSnapshot,
) -> Result<RegistryBinding> {
    registry::validate_documents(
        welcome.world_schema_version,
        welcome.event_schema_version,
        welcome.content_schema_version,
        &welcome.content_manifest_version,
        &config.expected_content_hash,
        &config.expected_universe_id,
        &config.expected_celestial_registry_hash,
        &config.expected_universe_manifest_hash,
        config.limits.max_registry_bodies,
        config.limits.max_registry_pair_comparisons,
        registry,
        manifest,
    )
}

fn validate_outer_baseline(
    welcome: &WelcomeBinding,
    binding: &RegistryBinding,
    baseline: &ProjectedWorldSnapshot,
) -> Result<()> {
    let valid = baseline.projection_schema_version == PROJECTION_SCHEMA_VERSION
        && baseline.schema_version == welcome.world_schema_version
        && baseline.content_manifest_version == welcome.content_manifest_version
        && baseline.universe_id == binding.universe_id
        && baseline.universe_manifest_hash == binding.universe_manifest_hash
        && baseline.celestial_registry_hash == binding.registry_hash
        && baseline.cell_address.universe_id == binding.universe_id;
    if !valid {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "baseline outer header disagrees with the established binding",
        ));
    }
    binding.validate_address(&baseline.cell_address, "baseline cell address")?;
    validate_cell_body_bindings(
        binding,
        &baseline.gravity_body_id,
        &baseline.voxel_body_id,
        "baseline",
    )?;
    binding.validate_environment(&baseline.environment, "baseline environment")?;
    Ok(())
}

fn validate_outer_delta(
    welcome: &WelcomeBinding,
    binding: &RegistryBinding,
    delta: &ProjectedInterestDelta,
) -> Result<()> {
    let valid = delta.projection_schema_version == PROJECTION_SCHEMA_VERSION
        && delta.schema_version == welcome.world_schema_version
        && delta.content_manifest_version == welcome.content_manifest_version
        && delta.universe_id == binding.universe_id
        && delta.universe_manifest_hash == binding.universe_manifest_hash
        && delta.celestial_registry_hash == binding.registry_hash
        && delta.cell_address.universe_id == binding.universe_id;
    if !valid {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "delta outer header disagrees with the established binding",
        ));
    }
    binding.validate_address(&delta.cell_address, "delta cell address")?;
    validate_cell_body_bindings(
        binding,
        &delta.gravity_body_id,
        &delta.voxel_body_id,
        "delta",
    )?;
    if let Some(environment) = &delta.environment {
        binding.validate_environment(environment, "delta environment")?;
    }
    Ok(())
}

fn validate_cell_body_bindings(
    binding: &RegistryBinding,
    gravity_body_id: &str,
    voxel_body_id: &str,
    label: &str,
) -> Result<()> {
    match (gravity_body_id.is_empty(), voxel_body_id.is_empty()) {
        (true, true) => Ok(()),
        (false, false) => {
            binding.require_body(gravity_body_id, &format!("{label} gravity_body_id"))?;
            binding.require_body(voxel_body_id, &format!("{label} voxel_body_id"))
        }
        _ => Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            format!("{label} gravity and voxel body bindings must both be present or both absent"),
        )),
    }
}

fn validate_interest_header(
    interest: &InterestSnapshot,
    kind: InterestFrameKind,
    binding: &RegistryBinding,
) -> Result<()> {
    let valid = interest.schema_version == INTEREST_SCHEMA_VERSION
        && interest.frame_kind == kind
        && interest.registry_hash == binding.registry_hash
        && interest.universe_manifest_hash == binding.universe_manifest_hash
        && interest.cell_address.universe_id == binding.universe_id
        && interest.local_origin_address.universe_id == binding.universe_id;
    if !valid {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "interest header disagrees with frame kind or established binding",
        ));
    }
    binding.validate_address(&interest.cell_address, "interest cell address")?;
    binding.validate_address(
        &interest.local_origin_address,
        "interest local-origin address",
    )?;
    Ok(())
}

fn validate_transfer_link(
    link: Option<&InterestTransferLink>,
    destination_cell_id: &str,
    destination_cell_address: &UniverseAddress,
    role: &SessionRole,
    binding: &RegistryBinding,
) -> Result<()> {
    let Some(link) = link else {
        return Ok(());
    };
    let valid_identifier = !link.transfer_id.is_empty()
        && link.transfer_id.len() <= 128
        && link
            .transfer_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if !matches!(role, SessionRole::Player { .. })
        || !valid_identifier
        || link.placement_generation == 0
        || link.destination_cell_key.schema_version != CELL_KEY_SCHEMA_VERSION
    {
        return Err(VerifyError::new(
            ErrorCode::InvalidBaseline,
            "transfer link role, identity, schema, or placement generation is invalid",
        ));
    }
    let address = UniverseAddress {
        universe_id: link.destination_cell_key.universe_id.clone(),
        sector: link.destination_cell_key.sector.clone(),
        cell: link.destination_cell_key.cell,
        local_um: I64Vec3::ZERO,
    };
    binding.validate_address(&address, "transfer destination cell key")?;
    if &address != destination_cell_address {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "transfer link does not name the baseline destination cell address",
        ));
    }
    let canonical = canonical::fixed_json(&link.destination_cell_key)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"the-verse/cell-key/v1\0");
    hasher.update(&canonical);
    if hasher.finalize().to_hex().as_str() != destination_cell_id {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "transfer link does not name the baseline destination cell identity",
        ));
    }
    Ok(())
}

fn validate_initial_handoff(
    status: &HandoffStatus,
    source: &ViewState,
    binding: &RegistryBinding,
) -> Result<()> {
    if status.phase != HandoffPhase::Preparing {
        return Err(VerifyError::new(
            ErrorCode::UnexpectedMessage,
            "the first handoff status must be preparing",
        ));
    }
    validate_handoff_material(status, binding)?;
    let destination_cell_id = cell_id_for_key(&status.destination_cell_key)?;
    if destination_cell_id == source.cell_id {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "handoff destination must differ from the committed source cell",
        ));
    }
    Ok(())
}

fn validate_handoff_progress(current: &HandoffBinding, candidate: &HandoffStatus) -> Result<()> {
    let same_material = candidate.transfer_id == current.status.transfer_id
        && candidate.destination_cell_key == current.status.destination_cell_key
        && candidate.placement_generation == current.status.placement_generation;
    let valid_phase = candidate.phase == current.status.phase
        || matches!(
            (current.status.phase, candidate.phase),
            (HandoffPhase::Preparing, HandoffPhase::Importing)
                | (HandoffPhase::Importing, HandoffPhase::VerifyingDestination)
        );
    if !same_material || !valid_phase {
        return Err(VerifyError::new(
            ErrorCode::FrontierMismatch,
            "handoff identity changed or phase did not advance monotonically",
        ));
    }
    Ok(())
}

fn validate_destination_baseline(handoff: &HandoffBinding, candidate: &ViewState) -> Result<()> {
    let Some(link) = candidate.transfer_link.as_ref() else {
        return Err(VerifyError::new(
            ErrorCode::InvalidBaseline,
            "handoff completion requires a transfer-linked destination baseline",
        ));
    };
    let expected_interest_epoch = handoff
        .prior_interest_epoch
        .checked_add(1)
        .ok_or_else(|| VerifyError::new(ErrorCode::FrontierMismatch, "interest epoch overflow"))?;
    let valid = handoff.status.phase == HandoffPhase::VerifyingDestination
        && link.transfer_id == handoff.status.transfer_id
        && link.destination_cell_key == handoff.status.destination_cell_key
        && link.placement_generation == handoff.status.placement_generation
        && candidate.session_epoch == handoff.session_epoch
        && candidate.interest_epoch == expected_interest_epoch;
    if !valid {
        return Err(VerifyError::new(
            ErrorCode::FrontierMismatch,
            "destination baseline does not complete the committed handoff frontier",
        ));
    }
    Ok(())
}

fn validate_handoff_material(status: &HandoffStatus, binding: &RegistryBinding) -> Result<()> {
    let valid_identifier = !status.transfer_id.is_empty()
        && status.transfer_id.len() <= 128
        && status
            .transfer_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'));
    if !valid_identifier
        || status.placement_generation == 0
        || status.destination_cell_key.schema_version != CELL_KEY_SCHEMA_VERSION
    {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "handoff identity, cell-key schema, or placement generation is invalid",
        ));
    }
    let address = UniverseAddress {
        universe_id: status.destination_cell_key.universe_id.clone(),
        sector: status.destination_cell_key.sector.clone(),
        cell: status.destination_cell_key.cell,
        local_um: I64Vec3::ZERO,
    };
    binding.validate_address(&address, "handoff destination cell key")?;
    cell_id_for_key(&status.destination_cell_key)?;
    Ok(())
}

fn cell_id_for_key(key: &CellKeyV1) -> Result<String> {
    let canonical = canonical::fixed_json(key)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"the-verse/cell-key/v1\0");
    hasher.update(&canonical);
    Ok(hasher.finalize().to_hex().to_string())
}

fn repeated_baseline_headers(baseline: &ProjectedWorldSnapshot) -> Result<()> {
    let interest = &baseline.interest;
    if baseline.cell_address != interest.cell_address
        || baseline.event_sequence != interest.canonical_event_sequence
        || baseline.simulation_tick != interest.canonical_tick
        || baseline.world_hash != interest.canonical_world_hash
    {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "baseline outer and interest headers disagree",
        ));
    }
    Ok(())
}

fn repeated_delta_headers(delta: &ProjectedInterestDelta) -> Result<()> {
    let interest = &delta.interest;
    if delta.cell_address != interest.cell_address
        || delta.event_sequence != interest.canonical_event_sequence
        || delta.simulation_tick != interest.canonical_tick
        || delta.world_hash != interest.canonical_world_hash
    {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "delta outer and interest headers disagree",
        ));
    }
    Ok(())
}

fn validate_role(
    welcome: &WelcomeBinding,
    observer: InterestObserverClass,
    private: Option<&ActorPrivateSnapshot>,
    motion: Option<&PlayerMotionSnapshot>,
) -> Result<()> {
    match &welcome.role {
        SessionRole::Spectator => {
            if observer != InterestObserverClass::PublicOriginSpectator
                || private.is_some()
                || motion.is_some()
            {
                return Err(VerifyError::new(
                    ErrorCode::BindingMismatch,
                    "spectator observer or private state is invalid",
                ));
            }
        }
        SessionRole::Player { player_id } => {
            if observer != InterestObserverClass::BoundPlayer
                || private.is_some_and(|value| value.player.player_id != *player_id)
                || motion.is_some_and(|value| value.player_id != *player_id)
            {
                return Err(VerifyError::new(
                    ErrorCode::BindingMismatch,
                    "bound player observer or private identity is invalid",
                ));
            }
        }
    }
    Ok(())
}

fn validate_delta_frontier(current: &ViewState, interest: &InterestSnapshot) -> Result<()> {
    let next = current
        .delta_sequence
        .checked_add(1)
        .ok_or_else(|| VerifyError::new(ErrorCode::FrontierMismatch, "delta sequence overflow"))?;
    let valid = interest.session_epoch == current.session_epoch
        && interest.interest_epoch == current.interest_epoch
        && interest.baseline_id == current.baseline_id
        && interest.delta_sequence == next
        && interest.previous_view_hash.as_deref() == Some(current.view_hash.as_str())
        && interest.observer_class == current.observer_class
        && interest.cell_address == current.cell_address
        && interest.canonical_event_sequence >= current.canonical_event_sequence
        && interest.canonical_tick >= current.canonical_tick;
    if !valid {
        return Err(VerifyError::new(
            ErrorCode::FrontierMismatch,
            "delta frontier does not extend the committed view",
        ));
    }
    Ok(())
}

fn validate_recovery_baseline(current: &ViewState, candidate: &ViewState) -> Result<()> {
    let valid = candidate.session_epoch == current.session_epoch
        && candidate.observer_class == current.observer_class
        && candidate.interest_epoch > current.interest_epoch
        && candidate.baseline_id != current.baseline_id
        && candidate.canonical_event_sequence >= current.canonical_event_sequence
        && candidate.canonical_tick >= current.canonical_tick;
    if !valid {
        return Err(VerifyError::new(
            ErrorCode::FrontierMismatch,
            "recovery baseline must advance the epoch with a new baseline ID in the same session",
        ));
    }
    Ok(())
}

fn entity_key(entity: &InterestEntityProjection) -> EntityKey {
    (entity.entity_id.clone(), entity.kind)
}

fn validate_entity_vector(
    entities: &[InterestEntityProjection],
    limits: &ResourceLimits,
    binding: &RegistryBinding,
) -> Result<()> {
    if entities.len() > limits.max_entities {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "entity vector exceeds max_entities",
        ));
    }
    ensure_strictly_sorted(entities, entity_key, "entity vector")?;
    for entity in entities {
        if entity.component_schema_version != PROJECTION_SCHEMA_VERSION {
            return Err(VerifyError::new(
                ErrorCode::InvalidEntity,
                "entity component schema version does not match the projection schema",
            ));
        }
        match (&entity.kind, &entity.payload) {
            (InterestEntityKind::Player, InterestEntityPayload::Player(value))
                if entity.entity_id == value.player_id =>
            {
                binding.validate_address(&value.address, "public player address")?;
            }
            (InterestEntityKind::Grid, InterestEntityPayload::Grid(value))
                if entity.entity_id == value.grid_id =>
            {
                validate_grid(value, limits, binding)?;
            }
            (InterestEntityKind::VoxelChunk, InterestEntityPayload::VoxelChunk(value))
                if entity.entity_id == value.chunk_id =>
            {
                validate_voxel_chunk(value, limits, binding)?;
            }
            (InterestEntityKind::DeathDrop, InterestEntityPayload::DeathDrop(value))
                if entity.entity_id == value.drop_id =>
            {
                binding.validate_address(&value.address, "public death-drop address")?;
            }
            _ => {
                return Err(VerifyError::new(
                    ErrorCode::InvalidEntity,
                    "entity kind, payload variant, and payload identity disagree",
                ));
            }
        }
    }
    Ok(())
}

fn validate_removals(
    removals: &[verse_protocol::InterestRemoval],
    limits: &ResourceLimits,
) -> Result<()> {
    if removals.len() > limits.max_entities {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "removal vector exceeds max_entities",
        ));
    }
    ensure_strictly_sorted(
        removals,
        |value| (value.entity_id.clone(), value.kind),
        "removal vector",
    )
}

fn validate_grid(
    grid: &PublicGridSnapshot,
    limits: &ResourceLimits,
    binding: &RegistryBinding,
) -> Result<()> {
    binding.validate_address(&grid.address, "public grid address")?;
    if grid.blocks.len() > limits.max_blocks_per_grid {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "grid blocks exceed max_blocks_per_grid",
        ));
    }
    ensure_strictly_sorted(&grid.blocks, |block| block.block_id.clone(), "grid blocks")
}

fn validate_voxel_chunk(
    chunk: &PublicVoxelChunkSnapshot,
    limits: &ResourceLimits,
    binding: &RegistryBinding,
) -> Result<()> {
    binding.require_body(&chunk.body_id, "voxel chunk body_id")?;
    if chunk.voxels.len() > limits.max_voxels_per_chunk {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "voxel chunk exceeds max_voxels_per_chunk",
        ));
    }
    ensure_strictly_sorted(
        &chunk.voxels,
        |voxel| (voxel.coordinate.x, voxel.coordinate.y, voxel.coordinate.z),
        "voxel coordinates",
    )
}

fn validate_actor_private(
    private: &ActorPrivateSnapshot,
    limits: &ResourceLimits,
    binding: &RegistryBinding,
) -> Result<()> {
    binding.validate_address(&private.player.address, "private player address")?;
    if let Some(environment) = &private.player.environment {
        binding.validate_environment(environment, "private player environment")?;
    }
    for drop in &private.death_drops {
        binding.validate_address(&drop.address, "private death-drop address")?;
    }
    let count = private
        .inventories
        .len()
        .saturating_add(private.death_drops.len())
        .saturating_add(private.owned_grid_masses.len())
        .saturating_add(private.production_queues.len())
        .saturating_add(
            private
                .production_queues
                .iter()
                .map(|queue| queue.jobs.len())
                .sum::<usize>(),
        );
    if count > limits.max_private_records {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "actor-private collections exceed max_private_records",
        ));
    }
    ensure_strictly_sorted(
        &private.inventories,
        |value| value.inventory_id.clone(),
        "private inventories",
    )?;
    ensure_strictly_sorted(
        &private.death_drops,
        |value| value.drop_id.clone(),
        "private death drops",
    )?;
    ensure_strictly_sorted(
        &private.owned_grid_masses,
        |value| value.grid_id.clone(),
        "owned grid masses",
    )?;
    ensure_strictly_sorted(
        &private.production_queues,
        |value| value.machine_block_id.clone(),
        "production queues",
    )
}

fn validate_private_motion(motion: &PlayerMotionSnapshot, binding: &RegistryBinding) -> Result<()> {
    binding.validate_address(&motion.address, "private player motion address")?;
    if let Some(environment) = &motion.environment {
        binding.validate_environment(environment, "private player motion environment")?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct PublicBlockLink<'a> {
    grid: &'a PublicGridSnapshot,
    block: &'a PublicBlockSnapshot,
}

struct PublicLinkIndex<'a> {
    players: BTreeMap<&'a str, &'a PublicPlayerSnapshot>,
    grids: BTreeMap<&'a str, &'a PublicGridSnapshot>,
    drops: BTreeMap<&'a str, &'a PublicDeathDropSnapshot>,
    blocks: BTreeMap<&'a str, Vec<PublicBlockLink<'a>>>,
}

fn validate_private_linkage(
    role: &SessionRole,
    private: Option<&ActorPrivateSnapshot>,
    entities: &[InterestEntityProjection],
) -> Result<()> {
    let (actor_id, private) = match (role, private) {
        (SessionRole::Player { player_id }, Some(private)) => (player_id.as_str(), private),
        (SessionRole::Player { .. }, None) => {
            return Err(invalid_private_linkage(
                "bound-player view has no actor-private overlay",
            ));
        }
        (SessionRole::Spectator, None) => return Ok(()),
        (SessionRole::Spectator, Some(_)) => {
            return Err(invalid_private_linkage(
                "spectator view contains actor-private state",
            ));
        }
    };
    if private.player.player_id != actor_id {
        return Err(invalid_private_linkage(
            "private player identity differs from the bound actor",
        ));
    }

    let public = build_public_link_index(entities);
    let public_player = public.players.get(actor_id).ok_or_else(|| {
        invalid_private_linkage("bound private player has no visible public player")
    })?;
    if public_player.address != private.player.address {
        return Err(invalid_private_linkage(
            "bound public and private player addresses disagree",
        ));
    }

    let inventories: BTreeMap<&str, &InventorySnapshot> = private
        .inventories
        .iter()
        .map(|inventory| (inventory.inventory_id.as_str(), inventory))
        .collect();
    let mut carried_inventory_count = 0_usize;
    let mut cargo_blocks = BTreeSet::new();
    let mut dropped_inventory_ids = BTreeSet::new();
    for inventory in &private.inventories {
        match &inventory.domain {
            InventoryDomain::Player { player_id } => {
                if player_id != actor_id || inventory.inventory_id != private.player.inventory_id {
                    return Err(invalid_private_linkage(
                        "carried inventory identity or player domain differs from the bound actor",
                    ));
                }
                carried_inventory_count = carried_inventory_count.saturating_add(1);
            }
            InventoryDomain::Cargo { block_id } => {
                if !cargo_blocks.insert(block_id.as_str()) {
                    return Err(invalid_private_linkage(
                        "more than one private inventory names the same cargo block",
                    ));
                }
                let link = require_unique_public_block(&public, block_id, "cargo inventory")?;
                if link.grid.owner_player_id != actor_id || link.block.kind != BlockKind::Cargo {
                    return Err(invalid_private_linkage(
                        "cargo inventory does not resolve to one visible actor-owned cargo block",
                    ));
                }
            }
            InventoryDomain::Dropped {
                owner_player_id, ..
            } => {
                if owner_player_id != actor_id {
                    return Err(invalid_private_linkage(
                        "dropped inventory owner differs from the bound actor",
                    ));
                }
                dropped_inventory_ids.insert(inventory.inventory_id.as_str());
            }
        }
    }
    if carried_inventory_count != 1
        || !inventories.contains_key(private.player.inventory_id.as_str())
    {
        return Err(invalid_private_linkage(
            "private player must resolve exactly one carried player inventory",
        ));
    }

    let mut linked_drop_inventories = BTreeSet::new();
    for drop in &private.death_drops {
        if drop.owner_player_id != actor_id {
            return Err(invalid_private_linkage(
                "private death-drop owner differs from the bound actor",
            ));
        }
        let public_drop = public.drops.get(drop.drop_id.as_str()).ok_or_else(|| {
            invalid_private_linkage("private death drop has no visible public death drop")
        })?;
        if public_drop.address != drop.address {
            return Err(invalid_private_linkage(
                "public and private death-drop addresses disagree",
            ));
        }
        let inventory = inventories
            .get(drop.inventory_id.as_str())
            .ok_or_else(|| invalid_private_linkage("private death drop inventory is absent"))?;
        if !matches!(
            &inventory.domain,
            InventoryDomain::Dropped { owner_player_id, .. } if owner_player_id == actor_id
        ) {
            return Err(invalid_private_linkage(
                "private death drop does not resolve to an actor-owned dropped inventory",
            ));
        }
        if !linked_drop_inventories.insert(drop.inventory_id.as_str()) {
            return Err(invalid_private_linkage(
                "more than one private death drop names the same inventory",
            ));
        }
    }
    if dropped_inventory_ids != linked_drop_inventories {
        return Err(invalid_private_linkage(
            "actor-private dropped inventories and death drops are not one-to-one",
        ));
    }

    for mass in &private.owned_grid_masses {
        let grid = public.grids.get(mass.grid_id.as_str()).ok_or_else(|| {
            invalid_private_linkage("private grid mass has no visible public grid")
        })?;
        if grid.owner_player_id != actor_id {
            return Err(invalid_private_linkage(
                "private grid mass resolves to a grid owned by another actor",
            ));
        }
    }

    for queue in &private.production_queues {
        let machine =
            require_unique_public_block(&public, &queue.machine_block_id, "production queue")?;
        if machine.grid.owner_player_id != actor_id
            || !matches!(
                machine.block.kind,
                BlockKind::Refinery | BlockKind::Assembler
            )
        {
            return Err(invalid_private_linkage(
                "production queue does not resolve to one visible actor-owned machine",
            ));
        }
        let mut job_ids = BTreeSet::new();
        for job in &queue.jobs {
            if !job_ids.insert(job.job_id.as_str())
                || job.owner_player_id != actor_id
                || job.machine_block_id != queue.machine_block_id
            {
                return Err(invalid_private_linkage(
                    "production job identity, owner, or machine differs from its queue",
                ));
            }
            let recipe_matches_machine = matches!(
                (job.recipe, machine.block.kind),
                (ProductionRecipeKind::Refining, BlockKind::Refinery)
                    | (ProductionRecipeKind::Component, BlockKind::Assembler)
            );
            if !recipe_matches_machine {
                return Err(invalid_private_linkage(
                    "production job recipe does not match its visible machine",
                ));
            }
            if !inventories.contains_key(job.source_inventory_id.as_str())
                || !inventories.contains_key(job.destination_inventory_id.as_str())
            {
                return Err(invalid_private_linkage(
                    "production job endpoint inventory is absent from actor-private state",
                ));
            }
        }
    }
    Ok(())
}

fn build_public_link_index(entities: &[InterestEntityProjection]) -> PublicLinkIndex<'_> {
    let mut index = PublicLinkIndex {
        players: BTreeMap::new(),
        grids: BTreeMap::new(),
        drops: BTreeMap::new(),
        blocks: BTreeMap::new(),
    };
    for entity in entities {
        match &entity.payload {
            InterestEntityPayload::Player(player) => {
                index.players.insert(player.player_id.as_str(), player);
            }
            InterestEntityPayload::Grid(grid) => {
                index.grids.insert(grid.grid_id.as_str(), grid);
                for block in &grid.blocks {
                    index
                        .blocks
                        .entry(block.block_id.as_str())
                        .or_default()
                        .push(PublicBlockLink { grid, block });
                }
            }
            InterestEntityPayload::DeathDrop(drop) => {
                index.drops.insert(drop.drop_id.as_str(), drop);
            }
            InterestEntityPayload::VoxelChunk(_) => {}
        }
    }
    index
}

fn require_unique_public_block<'a>(
    index: &'a PublicLinkIndex<'a>,
    block_id: &str,
    label: &str,
) -> Result<PublicBlockLink<'a>> {
    let links = index.blocks.get(block_id).ok_or_else(|| {
        invalid_private_linkage(format!("{label} block is absent from the public view"))
    })?;
    if links.len() != 1 {
        return Err(invalid_private_linkage(format!(
            "{label} block identity is ambiguous in the public view"
        )));
    }
    Ok(links[0])
}

fn invalid_private_linkage(detail: impl Into<String>) -> VerifyError {
    VerifyError::new(ErrorCode::InvalidPrivateLinkage, detail)
}

fn ensure_strictly_sorted<T, K, F>(values: &[T], key: F, label: &str) -> Result<()>
where
    K: Ord,
    F: Fn(&T) -> K,
{
    if values
        .windows(2)
        .any(|window| key(&window[0]) >= key(&window[1]))
    {
        return Err(VerifyError::new(
            ErrorCode::NonCanonicalOrder,
            format!("{label} is not strictly canonical"),
        ));
    }
    Ok(())
}

fn validate_baseline_payload_arrays(
    baseline: &ProjectedWorldSnapshot,
    entities: &[InterestEntityProjection],
) -> Result<()> {
    let mut players = Vec::<PublicPlayerSnapshot>::new();
    let mut grids = Vec::<PublicGridSnapshot>::new();
    let mut chunks = Vec::<PublicVoxelChunkSnapshot>::new();
    let mut drops = Vec::<PublicDeathDropSnapshot>::new();
    for entity in entities {
        match &entity.payload {
            InterestEntityPayload::Player(value) => players.push(value.clone()),
            InterestEntityPayload::Grid(value) => grids.push(value.clone()),
            InterestEntityPayload::VoxelChunk(value) => chunks.push(value.clone()),
            InterestEntityPayload::DeathDrop(value) => drops.push(value.clone()),
        }
    }
    if baseline.players != players
        || baseline.grids != grids
        || baseline.voxel_chunks != chunks
        || baseline.death_drops != drops
    {
        return Err(VerifyError::new(
            ErrorCode::InvalidBaseline,
            "baseline public arrays are not the exact complete interest payload set",
        ));
    }
    Ok(())
}

fn validate_disjoint_operations(
    interest: &InterestSnapshot,
    limits: &ResourceLimits,
) -> Result<()> {
    let operation_count = interest
        .entered
        .len()
        .saturating_add(interest.replaced.len())
        .saturating_add(interest.removed.len());
    if operation_count > limits.max_entities {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "combined delta operations exceed max_entities",
        ));
    }
    let mut identities = BTreeSet::new();
    for key in interest
        .entered
        .iter()
        .chain(&interest.replaced)
        .map(entity_key)
        .chain(
            interest
                .removed
                .iter()
                .map(|value| (value.entity_id.clone(), value.kind)),
        )
    {
        if !identities.insert(key) {
            return Err(VerifyError::new(
                ErrorCode::InvalidDelta,
                "one delta mentions an identity in more than one operation",
            ));
        }
    }
    Ok(())
}

fn apply_private_motion(private: &mut ActorPrivateSnapshot, motion: &PlayerMotionSnapshot) {
    let player = &mut private.player;
    player.address = motion.address.clone();
    player.orientation = motion.orientation;
    player.linear_velocity = motion.linear_velocity;
    player.angular_velocity = motion.angular_velocity;
    player.surface_contact = motion.surface_contact;
    player.locomotion = motion.locomotion.clone();
    player.movement_epoch = motion.movement_epoch;
    player.last_received_input_sequence = motion.last_received_input_sequence;
    player.last_processed_input_sequence = motion.last_processed_input_sequence;
    player.control_linear_input = motion.control_linear_input;
    player.control_angular_input = motion.control_angular_input;
    player.boost = motion.boost;
    player.dampeners = motion.dampeners;
    player.jump = motion.jump;
    player.control_expires_at_simulation_tick = motion.control_expires_at_simulation_tick;
    player.jetpack_enabled = motion.jetpack_enabled;
    player.life_state = motion.life_state.clone();
    player.environment.clone_from(&motion.environment);
}

#[derive(Serialize)]
struct HashMaterial<'a> {
    projection_schema_version: u32,
    interest_schema_version: u32,
    content_manifest_version: &'a str,
    universe_id: &'a str,
    cell_id: &'a str,
    universe_manifest_hash: &'a str,
    celestial_registry_hash: &'a str,
    cell_address: &'a UniverseAddress,
    local_origin: &'a UniverseAddress,
    gravity_body_id: &'a str,
    voxel_body_id: &'a str,
    observer_class: InterestObserverClass,
    session_epoch: &'a str,
    interest_epoch: u64,
    baseline_id: &'a str,
    delta_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    transfer_link: Option<&'a InterestTransferLink>,
    entities: &'a [InterestEntityProjection],
    environment: &'a EnvironmentSnapshot,
    conservation_valid: bool,
    actor_private: &'a Option<ActorPrivateSnapshot>,
}

fn hash_view(view: &ViewState) -> Result<String> {
    canonical::digest(&HashMaterial {
        projection_schema_version: PROJECTION_SCHEMA_VERSION,
        interest_schema_version: INTEREST_SCHEMA_VERSION,
        content_manifest_version: &view.content_manifest_version,
        universe_id: &view.universe_id,
        cell_id: &view.cell_id,
        universe_manifest_hash: &view.universe_manifest_hash,
        celestial_registry_hash: &view.celestial_registry_hash,
        cell_address: &view.cell_address,
        local_origin: &view.local_origin,
        gravity_body_id: &view.gravity_body_id,
        voxel_body_id: &view.voxel_body_id,
        observer_class: view.observer_class,
        session_epoch: &view.session_epoch,
        interest_epoch: view.interest_epoch,
        baseline_id: &view.baseline_id,
        delta_sequence: view.delta_sequence,
        transfer_link: view.transfer_link.as_ref(),
        entities: &view.entities,
        environment: &view.environment,
        conservation_valid: view.conservation_valid,
        actor_private: &view.actor_private,
    })
}

fn require_wire_hash(wire: &str, computed: &str) -> Result<()> {
    let valid_shape = wire.len() == 64
        && wire
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    if !valid_shape || wire.as_bytes() != computed.as_bytes() {
        return Err(VerifyError::new(
            ErrorCode::HashMismatch,
            "wire view_hash is not the exact lowercase computed digest",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use verse_protocol::{
        CareerSnapshot, CelestialBodyKind, CelestialBodySnapshot, CelestialRegistrySnapshot,
        CelestialScaleClass, CellCoordinate, DeathDropSnapshot, I64Vec3, InterestSnapshot,
        InventoryContents, InventorySnapshot, LocomotionKind, OwnedGridMassSnapshot,
        PlayerDeathCause, PlayerLifeState, PlayerLocomotionSnapshot, PlayerSnapshot, PowerSnapshot,
        ProductionJobSnapshot, ProductionJobStatus, ProductionQueueSnapshot, PublicMachineState,
        PublicPlayerLifeState, Quat, SectorCoordinate, Vec3,
    };

    use super::*;

    const WORLD_SCHEMA: u32 = 11;
    const EVENT_SCHEMA: u32 = 12;
    const CONTENT_SCHEMA: u32 = 13;
    const CONTENT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn address() -> UniverseAddress {
        UniverseAddress {
            universe_id: "universe-test".into(),
            sector: SectorCoordinate {
                x: "0".into(),
                y: "2".into(),
                z: "3".into(),
            },
            cell: CellCoordinate { x: 4, y: 5, z: 6 },
            local_um: I64Vec3 { x: -7, y: 8, z: 9 },
        }
    }

    fn environment() -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            celestial_body_id: "body-a".into(),
            celestial_body_name: "Body A".into(),
            celestial_scale_class: CelestialScaleClass::Proof,
            nearest_body_id: "body-a".into(),
            nearest_body_name: "Body A".into(),
            planet_center: Vec3::new(1.25, -2.5, 0.000_000_5),
            surface_radius_m: 3.0,
            distance_to_center_m: 4.0,
            distance_to_surface_m: 1.0,
            altitude_m: -0.000_000_5,
            gravity: Vec3::new(0.0, -9.81, -0.0),
            gravity_m_s2: 9.81,
            atmosphere_density: 0.5,
            oxygen_fraction: 0.21,
            breathable: true,
        }
    }

    fn private_snapshot() -> ActorPrivateSnapshot {
        ActorPrivateSnapshot {
            player: PlayerSnapshot {
                player_id: "player-a".into(),
                address: address(),
                position: Vec3::ZERO,
                orientation: Quat::IDENTITY,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                surface_contact: false,
                locomotion: PlayerLocomotionSnapshot {
                    kind: LocomotionKind::Eva,
                    up: Vec3::new(0.0, 1.0, 0.0),
                    view_pitch_radians: 0.0,
                    support: None,
                    jump_held: false,
                    jump_buffer_expires_at_simulation_tick: 0,
                    support_grace_expires_at_simulation_tick: 0,
                    magnetic_boots_enabled: false,
                    magnetic_reattach_after_simulation_tick: 0,
                },
                movement_epoch: 1,
                last_received_input_sequence: 0,
                last_processed_input_sequence: 0,
                control_linear_input: Vec3::ZERO,
                control_angular_input: Vec3::ZERO,
                boost: false,
                dampeners: true,
                jump: false,
                control_expires_at_simulation_tick: 0,
                inventory_id: "inventory-player-a".into(),
                experience: 0,
                level: 1,
                next_level_experience: 100,
                career: CareerSnapshot::default(),
                life_state: PlayerLifeState::Alive,
                suit_oxygen_milli: 1_000,
                critical_oxygen_milli: 100,
                helmet_closed: true,
                jetpack_enabled: true,
                environment: Some(environment()),
            },
            committed_operation_sequence: 0,
            inventories: Vec::new(),
            death_drops: vec![DeathDropSnapshot {
                drop_id: "drop-a".into(),
                death_id: "death-a".into(),
                inventory_id: "inventory-drop-a".into(),
                owner_player_id: "player-a".into(),
                address: address(),
                position: Vec3::ZERO,
                created_event_sequence: 1,
                cause: PlayerDeathCause::OxygenDepleted,
            }],
            owned_grid_masses: Vec::new(),
            production_queues: Vec::new(),
        }
    }

    fn private_motion() -> PlayerMotionSnapshot {
        let player = private_snapshot().player;
        PlayerMotionSnapshot {
            player_id: player.player_id,
            address: player.address,
            position: Vec3::ZERO,
            orientation: player.orientation,
            linear_velocity: player.linear_velocity,
            angular_velocity: player.angular_velocity,
            surface_contact: player.surface_contact,
            locomotion: player.locomotion,
            movement_epoch: player.movement_epoch,
            last_received_input_sequence: player.last_received_input_sequence,
            last_processed_input_sequence: player.last_processed_input_sequence,
            control_linear_input: player.control_linear_input,
            control_angular_input: player.control_angular_input,
            boost: player.boost,
            dampeners: player.dampeners,
            jump: player.jump,
            control_expires_at_simulation_tick: player.control_expires_at_simulation_tick,
            jetpack_enabled: player.jetpack_enabled,
            life_state: player.life_state,
            environment: player.environment,
        }
    }

    fn linked_public_entities() -> Vec<InterestEntityProjection> {
        let drop = PublicDeathDropSnapshot {
            drop_id: "drop-a".into(),
            address: address(),
        };
        let grid = PublicGridSnapshot {
            grid_id: "grid-a".into(),
            owner_player_id: "player-a".into(),
            address: address(),
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            anchored: false,
            power: PowerSnapshot::default(),
            blocks: vec![
                PublicBlockSnapshot {
                    block_id: "cargo-block".into(),
                    coordinate: verse_protocol::IVec3::ZERO,
                    kind: BlockKind::Cargo,
                    orientation: 0,
                    health: 100,
                    max_health: 100,
                    construction_complete: true,
                    machine_state: None,
                },
                PublicBlockSnapshot {
                    block_id: "refinery-block".into(),
                    coordinate: verse_protocol::IVec3::new(1, 0, 0),
                    kind: BlockKind::Refinery,
                    orientation: 0,
                    health: 100,
                    max_health: 100,
                    construction_complete: true,
                    machine_state: Some(PublicMachineState::Idle),
                },
            ],
        };
        let player = PublicPlayerSnapshot {
            player_id: "player-a".into(),
            address: address(),
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            surface_contact: false,
            locomotion_kind: LocomotionKind::Eva,
            life_state: PublicPlayerLifeState::Alive,
            helmet_closed: true,
            jetpack_enabled: true,
        };
        vec![
            InterestEntityProjection {
                entity_id: "drop-a".into(),
                kind: InterestEntityKind::DeathDrop,
                projected_revision: 1,
                component_schema_version: PROJECTION_SCHEMA_VERSION,
                payload: InterestEntityPayload::DeathDrop(drop),
            },
            InterestEntityProjection {
                entity_id: "grid-a".into(),
                kind: InterestEntityKind::Grid,
                projected_revision: 1,
                component_schema_version: PROJECTION_SCHEMA_VERSION,
                payload: InterestEntityPayload::Grid(grid),
            },
            InterestEntityProjection {
                entity_id: "player-a".into(),
                kind: InterestEntityKind::Player,
                projected_revision: 1,
                component_schema_version: PROJECTION_SCHEMA_VERSION,
                payload: InterestEntityPayload::Player(player),
            },
        ]
    }

    fn linked_private_snapshot() -> ActorPrivateSnapshot {
        let mut private = private_snapshot();
        private.player.inventory_id = "inventory-player".into();
        private.inventories = vec![
            InventorySnapshot {
                inventory_id: "inventory-cargo".into(),
                domain: InventoryDomain::Cargo {
                    block_id: "cargo-block".into(),
                },
                contents: InventoryContents::default(),
                capacity_liters: 100,
                used_liters: 0,
                mass_grams: 0,
            },
            InventorySnapshot {
                inventory_id: "inventory-drop".into(),
                domain: InventoryDomain::Dropped {
                    reason: "player_death".into(),
                    owner_player_id: "player-a".into(),
                },
                contents: InventoryContents::default(),
                capacity_liters: 100,
                used_liters: 0,
                mass_grams: 0,
            },
            InventorySnapshot {
                inventory_id: "inventory-player".into(),
                domain: InventoryDomain::Player {
                    player_id: "player-a".into(),
                },
                contents: InventoryContents::default(),
                capacity_liters: 100,
                used_liters: 0,
                mass_grams: 0,
            },
        ];
        private.death_drops[0].inventory_id = "inventory-drop".into();
        private.owned_grid_masses = vec![OwnedGridMassSnapshot {
            grid_id: "grid-a".into(),
            mass_kg: 1_000.0,
        }];
        private.production_queues = vec![ProductionQueueSnapshot {
            machine_block_id: "refinery-block".into(),
            jobs: vec![ProductionJobSnapshot {
                job_id: "job-a".into(),
                owner_player_id: "player-a".into(),
                machine_block_id: "refinery-block".into(),
                recipe: ProductionRecipeKind::Refining,
                batches: 1,
                source_inventory_id: "inventory-cargo".into(),
                destination_inventory_id: "inventory-player".into(),
                progress_ticks: 0,
                duration_ticks: 100,
                status: ProductionJobStatus::Queued,
                reserved_inputs: InventoryContents::default(),
                pending_outputs: InventoryContents::default(),
            }],
        }];
        private
    }

    fn player_role() -> SessionRole {
        SessionRole::Player {
            player_id: "player-a".into(),
        }
    }

    fn linkage_error(
        private: &ActorPrivateSnapshot,
        entities: &[InterestEntityProjection],
    ) -> ErrorCode {
        validate_private_linkage(&player_role(), Some(private), entities)
            .expect_err("invalid linkage is rejected")
            .code()
    }

    fn linked_player_baseline() -> ServerMessage {
        let entities = linked_public_entities();
        let private = linked_private_snapshot();
        let mut state = view_state(0);
        state.observer_class = InterestObserverClass::BoundPlayer;
        state.entities.clone_from(&entities);
        state.actor_private = Some(private.clone());
        state.view_hash = hash_view(&state).expect("linked player fixture hashes");

        let mut message = baseline();
        let ServerMessage::InterestBaseline { baseline } = &mut message else {
            unreachable!();
        };
        baseline.players = entities
            .iter()
            .filter_map(|entity| match &entity.payload {
                InterestEntityPayload::Player(player) => Some(player.clone()),
                _ => None,
            })
            .collect();
        baseline.grids = entities
            .iter()
            .filter_map(|entity| match &entity.payload {
                InterestEntityPayload::Grid(grid) => Some(grid.clone()),
                _ => None,
            })
            .collect();
        baseline.death_drops = entities
            .iter()
            .filter_map(|entity| match &entity.payload {
                InterestEntityPayload::DeathDrop(drop) => Some(drop.clone()),
                _ => None,
            })
            .collect();
        baseline.interest.observer_class = InterestObserverClass::BoundPlayer;
        baseline.interest.entered = entities;
        baseline.interest.view_hash = state.view_hash;
        baseline.actor_private = Some(private);
        message
    }

    fn config() -> VerifierConfig {
        let (registry_hash, manifest_hash) = binding_hashes();
        VerifierConfig::new(
            SessionRole::Spectator,
            WORLD_SCHEMA,
            EVENT_SCHEMA,
            CONTENT_SCHEMA,
            "content-v1",
            CONTENT_HASH,
            "universe-test",
            registry_hash,
            manifest_hash,
        )
    }

    fn welcome() -> ServerMessage {
        ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            world_schema_version: WORLD_SCHEMA,
            event_schema_version: EVENT_SCHEMA,
            content_schema_version: CONTENT_SCHEMA,
            content_manifest_version: "content-v1".into(),
            celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            universe_manifest_schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            server_name: "test".into(),
            session_role: SessionRole::Spectator,
        }
    }

    fn registry() -> ServerMessage {
        let body = CelestialBodySnapshot {
            body_id: "body-a".into(),
            display_name: "Body A".into(),
            kind: CelestialBodyKind::Asteroid,
            parent_body_id: None,
            field_id: None,
            center: address(),
            surface_radius_um: 10,
            exclusion_radius_um: 10,
            fixed_orientation_microradians: I64Vec3::ZERO,
            surface_gravity_millimetres_per_second_squared: 0,
            atmosphere_height_um: 0,
            oxygen_parts_per_million: 0,
            voxel_field_id: Some("voxel-a".into()),
            geometry_definition_id: "geometry-a".into(),
            voxel_definition_id: Some("voxel-definition-a".into()),
            material_definition_id: "material-a".into(),
            gravity_definition_id: "gravity-a".into(),
            atmosphere_definition_id: "atmosphere-a".into(),
            resource_definition_id: "resource-a".into(),
            visual_descriptor_id: "visual-a".into(),
            scale_class: CelestialScaleClass::Proof,
            generation_seed: "seed-a".into(),
            generation_rule_version: "generation-v1".into(),
            materialized_registry_version: 1,
            content_manifest_version: "content-v1".into(),
            content_hash: CONTENT_HASH.into(),
        };
        let mut registry = CelestialRegistrySnapshot {
            schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            registry_hash: String::new(),
            license: "CC-BY-SA-4.0".into(),
            universe_id: "universe-test".into(),
            generation_rule_version: "generation-v1".into(),
            minimum_fixed_body_surface_gap_um: 1,
            bodies: vec![body],
        };
        registry.registry_hash =
            registry::registry_hash(&registry).expect("registry fixture hashes");
        let mut manifest = UniverseManifestSnapshot {
            schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            manifest_hash: String::new(),
            universe_id: "universe-test".into(),
            world_seed: "seed".into(),
            address_schema_version: 1,
            sector_edge_um: 1_000,
            cell_edge_um: 100,
            cells_per_sector_axis: 10,
            generation_rule_version: "generation-v1".into(),
            frontier_policy_version: "frontier-v1".into(),
            celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            celestial_registry_hash: registry.registry_hash.clone(),
            content_schema_version: CONTENT_SCHEMA,
            content_manifest_version: "content-v1".into(),
            content_hash: CONTENT_HASH.into(),
            world_schema_version: WORLD_SCHEMA,
            event_schema_version: EVENT_SCHEMA,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            operation_fingerprint_schema_version: verse_protocol::INTENT_FINGERPRINT_SCHEMA_VERSION,
            cell_key_schema_version: verse_protocol::CELL_KEY_SCHEMA_VERSION,
            cell_directory_schema_version: verse_protocol::CELL_DIRECTORY_SCHEMA_VERSION,
            transfer_package_schema_version: verse_protocol::TRANSFER_PACKAGE_SCHEMA_VERSION,
            lifecycle_control_schema_version: verse_protocol::LIFECYCLE_CONTROL_SCHEMA_VERSION,
            production_schedule_occurrence_schema_version:
                verse_protocol::PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            lifecycle_policy_hash: CONTENT_HASH.into(),
        };
        manifest.manifest_hash =
            registry::manifest_hash(&manifest).expect("manifest fixture hashes");
        ServerMessage::Registry {
            registry: Box::new(registry),
            universe_manifest: Box::new(manifest),
        }
    }

    fn binding_hashes() -> (String, String) {
        let ServerMessage::Registry {
            registry,
            universe_manifest,
        } = registry()
        else {
            unreachable!();
        };
        (registry.registry_hash, universe_manifest.manifest_hash)
    }

    fn registry_binding() -> RegistryBinding {
        let ServerMessage::Registry {
            registry,
            universe_manifest,
        } = registry()
        else {
            unreachable!();
        };
        registry::validate_documents(
            WORLD_SCHEMA,
            EVENT_SCHEMA,
            CONTENT_SCHEMA,
            "content-v1",
            &universe_manifest.content_hash,
            &universe_manifest.universe_id,
            &registry.registry_hash,
            &universe_manifest.manifest_hash,
            512,
            130_816,
            &registry,
            &universe_manifest,
        )
        .expect("registry fixture validates")
    }

    fn interest(kind: InterestFrameKind, sequence: u64) -> InterestSnapshot {
        let (registry_hash, manifest_hash) = binding_hashes();
        InterestSnapshot {
            schema_version: INTEREST_SCHEMA_VERSION,
            frame_kind: kind,
            session_epoch: "session-1".into(),
            interest_epoch: 41,
            baseline_id: "baseline-1".into(),
            delta_sequence: sequence,
            observer_class: InterestObserverClass::PublicOriginSpectator,
            cell_address: address(),
            local_origin_address: address(),
            registry_hash,
            universe_manifest_hash: manifest_hash,
            transfer_link: None,
            canonical_event_sequence: 50 + sequence,
            canonical_tick: 60 + sequence,
            canonical_world_hash: format!("world-{sequence}"),
            previous_view_hash: None,
            view_hash: String::new(),
            entered: Vec::new(),
            replaced: Vec::new(),
            removed: Vec::new(),
        }
    }

    fn view_state(sequence: u64) -> ViewState {
        let (registry_hash, manifest_hash) = binding_hashes();
        let mut state = ViewState {
            content_manifest_version: "content-v1".into(),
            universe_id: "universe-test".into(),
            cell_id: "cell-a".into(),
            universe_manifest_hash: manifest_hash,
            celestial_registry_hash: registry_hash,
            cell_address: address(),
            local_origin: address(),
            gravity_body_id: "body-a".into(),
            voxel_body_id: "body-a".into(),
            observer_class: InterestObserverClass::PublicOriginSpectator,
            session_epoch: "session-1".into(),
            interest_epoch: 41,
            baseline_id: "baseline-1".into(),
            delta_sequence: sequence,
            canonical_event_sequence: 50 + sequence,
            canonical_tick: 60 + sequence,
            entities: Vec::new(),
            environment: environment(),
            conservation_valid: true,
            actor_private: None,
            transfer_link: None,
            view_hash: String::new(),
        };
        state.view_hash = hash_view(&state).expect("fixture hashes");
        state
    }

    fn baseline() -> ServerMessage {
        let state = view_state(0);
        let mut frontier = interest(InterestFrameKind::Baseline, 0);
        frontier.view_hash = state.view_hash;
        let (registry_hash, manifest_hash) = binding_hashes();
        ServerMessage::InterestBaseline {
            baseline: Box::new(ProjectedWorldSnapshot {
                projection_schema_version: PROJECTION_SCHEMA_VERSION,
                schema_version: WORLD_SCHEMA,
                content_manifest_version: "content-v1".into(),
                universe_id: "universe-test".into(),
                cell_id: "cell-a".into(),
                universe_manifest_hash: manifest_hash,
                celestial_registry_hash: registry_hash,
                cell_address: address(),
                gravity_body_id: "body-a".into(),
                voxel_body_id: "body-a".into(),
                event_sequence: 50,
                simulation_tick: 60,
                fencing_token: 999,
                world_hash: "world-0".into(),
                players: Vec::new(),
                environment: environment(),
                voxel_chunks: Vec::new(),
                grids: Vec::new(),
                death_drops: Vec::new(),
                conservation_valid: true,
                interest: frontier,
                actor_private: None,
            }),
        }
    }

    fn delta(previous: &str) -> ServerMessage {
        let state = view_state(1);
        let mut frontier = interest(InterestFrameKind::Delta, 1);
        frontier.previous_view_hash = Some(previous.into());
        frontier.view_hash = state.view_hash;
        let (registry_hash, manifest_hash) = binding_hashes();
        ServerMessage::InterestDelta {
            delta: Box::new(ProjectedInterestDelta {
                projection_schema_version: PROJECTION_SCHEMA_VERSION,
                schema_version: WORLD_SCHEMA,
                content_manifest_version: "content-v1".into(),
                universe_id: "universe-test".into(),
                cell_id: "cell-a".into(),
                universe_manifest_hash: manifest_hash,
                celestial_registry_hash: registry_hash,
                cell_address: address(),
                gravity_body_id: "body-a".into(),
                voxel_body_id: "body-a".into(),
                event_sequence: 51,
                simulation_tick: 61,
                world_hash: "world-1".into(),
                environment: None,
                conservation_valid: None,
                interest: frontier,
                actor_private: None,
                actor_private_motion: None,
            }),
        }
    }

    fn rebased_delta(previous: &str) -> ServerMessage {
        let mut origin = address();
        origin.local_um.x = -50;
        let mut state = view_state(1);
        state.local_origin = origin.clone();
        state.view_hash = hash_view(&state).expect("fixture hashes");
        let mut message = delta(previous);
        let ServerMessage::InterestDelta { delta } = &mut message else {
            unreachable!();
        };
        delta.interest.local_origin_address = origin;
        delta.interest.view_hash = state.view_hash;
        message
    }

    fn recovery_baseline() -> ServerMessage {
        let mut state = view_state(0);
        state.interest_epoch = 42;
        state.baseline_id = "baseline-2".into();
        state.view_hash = hash_view(&state).expect("fixture hashes");
        let mut message = baseline();
        let ServerMessage::InterestBaseline { baseline } = &mut message else {
            unreachable!();
        };
        baseline.interest.interest_epoch = 42;
        baseline.interest.baseline_id = "baseline-2".into();
        baseline.interest.view_hash = state.view_hash;
        message
    }

    fn baseline_with_drop() -> ServerMessage {
        let drop = PublicDeathDropSnapshot {
            drop_id: "drop-a".into(),
            address: address(),
        };
        let entity = InterestEntityProjection {
            entity_id: "drop-a".into(),
            kind: InterestEntityKind::DeathDrop,
            projected_revision: 1,
            component_schema_version: PROJECTION_SCHEMA_VERSION,
            payload: InterestEntityPayload::DeathDrop(drop.clone()),
        };
        let mut state = view_state(0);
        state.entities.push(entity.clone());
        state.view_hash = hash_view(&state).expect("fixture hashes");
        let mut message = baseline();
        let ServerMessage::InterestBaseline { baseline } = &mut message else {
            unreachable!();
        };
        baseline.death_drops.push(drop);
        baseline.interest.entered.push(entity);
        baseline.interest.view_hash = state.view_hash;
        message
    }

    fn bytes(message: &ServerMessage) -> Vec<u8> {
        serde_json::to_vec(message).expect("fixture serializes")
    }

    fn commit_message(verifier: &mut InterestVerifier, message: &ServerMessage) -> CommitOutcome {
        let token = verifier.stage(&bytes(message)).expect("fixture stages");
        verifier.commit(token).expect("fixture commits")
    }

    fn ready_verifier() -> InterestVerifier {
        let mut verifier = InterestVerifier::new(config()).expect("config is valid");
        assert_eq!(
            commit_message(&mut verifier, &welcome()).kind,
            StageKind::Welcome
        );
        assert_eq!(
            commit_message(&mut verifier, &registry()).kind,
            StageKind::Registry
        );
        verifier
    }

    #[test]
    fn rejects_duplicate_and_unknown_fields_before_transition() {
        let mut verifier = InterestVerifier::new(config()).expect("config is valid");
        let raw = br#"{"type":"welcome","protocol_version":17,"protocol_version":17,"projection_schema_version":3,"world_schema_version":11,"event_schema_version":12,"content_schema_version":13,"content_manifest_version":"content-v1","celestial_registry_schema_version":1,"universe_manifest_schema_version":3,"interest_schema_version":1,"server_name":"test","session_role":{"kind":"spectator"}}"#;
        assert_eq!(
            verifier
                .stage(raw)
                .expect_err("duplicate is rejected")
                .code(),
            ErrorCode::DuplicateKey
        );

        let mut value = serde_json::to_value(welcome()).expect("fixture serializes");
        value
            .as_object_mut()
            .expect("message object")
            .insert("unexpected".into(), serde_json::json!(true));
        assert_eq!(
            verifier
                .stage(&serde_json::to_vec(&value).expect("value serializes"))
                .expect_err("unknown is rejected")
                .code(),
            ErrorCode::UnknownField
        );
    }

    #[test]
    fn rejects_renderer_only_fields_and_noncanonical_entities() {
        let mut verifier = ready_verifier();
        let mut value = serde_json::to_value(baseline_with_drop()).expect("fixture serializes");
        value["baseline"]["interest"]["entered"][0]["payload"]["value"]
            .as_object_mut()
            .expect("death-drop payload object")
            .insert(
                "position".into(),
                serde_json::json!({"x": 0, "y": 0, "z": 0}),
            );
        assert_eq!(
            verifier
                .stage(&serde_json::to_vec(&value).expect("value serializes"))
                .expect_err("serde-skipped renderer field is rejected")
                .code(),
            ErrorCode::UnknownField
        );
        let token = verifier
            .stage(&bytes(&baseline_with_drop()))
            .expect("canonical entity baseline stages");
        verifier.discard(token).expect("entity baseline discards");

        let make_drop = |id: &str| InterestEntityProjection {
            entity_id: id.into(),
            kind: InterestEntityKind::DeathDrop,
            projected_revision: 1,
            component_schema_version: PROJECTION_SCHEMA_VERSION,
            payload: InterestEntityPayload::DeathDrop(PublicDeathDropSnapshot {
                drop_id: id.into(),
                address: address(),
            }),
        };
        let unordered = vec![make_drop("z"), make_drop("a")];
        let binding = registry_binding();
        assert_eq!(
            validate_entity_vector(&unordered, &ResourceLimits::default(), &binding)
                .expect_err("unordered entities are rejected")
                .code(),
            ErrorCode::NonCanonicalOrder
        );
        let mut wrong_identity = make_drop("a");
        let InterestEntityPayload::DeathDrop(value) = &mut wrong_identity.payload else {
            unreachable!();
        };
        value.drop_id = "other".into();
        assert_eq!(
            validate_entity_vector(&[wrong_identity], &ResourceLimits::default(), &binding)
                .expect_err("payload identity mismatch is rejected")
                .code(),
            ErrorCode::InvalidEntity
        );
    }

    #[test]
    fn recursively_rejects_noncanonical_addresses_and_unknown_body_references() {
        let binding = registry_binding();
        let limits = ResourceLimits::default();
        let invalid_address = || {
            let mut value = address();
            value.sector.x = "-0".into();
            value
        };

        let public_player = InterestEntityProjection {
            entity_id: "player-a".into(),
            kind: InterestEntityKind::Player,
            projected_revision: 1,
            component_schema_version: PROJECTION_SCHEMA_VERSION,
            payload: InterestEntityPayload::Player(PublicPlayerSnapshot {
                player_id: "player-a".into(),
                address: invalid_address(),
                position: Vec3::ZERO,
                orientation: Quat::IDENTITY,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                surface_contact: false,
                locomotion_kind: LocomotionKind::Eva,
                life_state: PublicPlayerLifeState::Alive,
                helmet_closed: true,
                jetpack_enabled: true,
            }),
        };
        assert_eq!(
            validate_entity_vector(&[public_player], &limits, &binding)
                .expect_err("public player address is independently rejected")
                .code(),
            ErrorCode::InvalidAddress
        );

        let public_grid = InterestEntityProjection {
            entity_id: "grid-a".into(),
            kind: InterestEntityKind::Grid,
            projected_revision: 1,
            component_schema_version: PROJECTION_SCHEMA_VERSION,
            payload: InterestEntityPayload::Grid(PublicGridSnapshot {
                grid_id: "grid-a".into(),
                owner_player_id: "player-a".into(),
                address: invalid_address(),
                position: Vec3::ZERO,
                orientation: Quat::IDENTITY,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                anchored: false,
                power: PowerSnapshot::default(),
                blocks: Vec::new(),
            }),
        };
        assert_eq!(
            validate_entity_vector(&[public_grid], &limits, &binding)
                .expect_err("public grid address is independently rejected")
                .code(),
            ErrorCode::InvalidAddress
        );

        let public_drop = InterestEntityProjection {
            entity_id: "drop-a".into(),
            kind: InterestEntityKind::DeathDrop,
            projected_revision: 1,
            component_schema_version: PROJECTION_SCHEMA_VERSION,
            payload: InterestEntityPayload::DeathDrop(PublicDeathDropSnapshot {
                drop_id: "drop-a".into(),
                address: invalid_address(),
            }),
        };
        assert_eq!(
            validate_entity_vector(&[public_drop], &limits, &binding)
                .expect_err("public drop address is independently rejected")
                .code(),
            ErrorCode::InvalidAddress
        );

        let unknown_chunk = InterestEntityProjection {
            entity_id: "chunk-a".into(),
            kind: InterestEntityKind::VoxelChunk,
            projected_revision: 1,
            component_schema_version: PROJECTION_SCHEMA_VERSION,
            payload: InterestEntityPayload::VoxelChunk(PublicVoxelChunkSnapshot {
                chunk_id: "chunk-a".into(),
                body_id: "unknown-body".into(),
                revision: 1,
                voxels: Vec::new(),
            }),
        };
        assert_eq!(
            validate_entity_vector(&[unknown_chunk], &limits, &binding)
                .expect_err("voxel chunk body reference is independently rejected")
                .code(),
            ErrorCode::BindingMismatch
        );

        let mut private = private_snapshot();
        private.player.address = invalid_address();
        assert_eq!(
            validate_actor_private(&private, &limits, &binding)
                .expect_err("private player address is independently rejected")
                .code(),
            ErrorCode::InvalidAddress
        );
        let mut private = private_snapshot();
        private.death_drops[0].address = invalid_address();
        assert_eq!(
            validate_actor_private(&private, &limits, &binding)
                .expect_err("private drop address is independently rejected")
                .code(),
            ErrorCode::InvalidAddress
        );
        let mut motion = private_motion();
        motion.address.cell.x = 10;
        assert_eq!(
            validate_private_motion(&motion, &binding)
                .expect_err("private motion address is independently rejected")
                .code(),
            ErrorCode::InvalidAddress
        );

        let mut environment = environment();
        environment.nearest_body_id = "unknown-body".into();
        assert_eq!(
            binding
                .validate_environment(&environment, "test environment")
                .expect_err("unknown environment reference is rejected")
                .code(),
            ErrorCode::BindingMismatch
        );

        let mut verifier = ready_verifier();
        let mut invalid_header = baseline();
        let ServerMessage::InterestBaseline { baseline: snapshot } = &mut invalid_header else {
            unreachable!();
        };
        snapshot.interest.local_origin_address = invalid_address();
        assert_eq!(
            verifier
                .stage(&bytes(&invalid_header))
                .expect_err("invalid local origin is rejected before hashing")
                .code(),
            ErrorCode::InvalidAddress
        );
        let mut unknown_header_body = baseline();
        let ServerMessage::InterestBaseline { baseline: snapshot } = &mut unknown_header_body
        else {
            unreachable!();
        };
        snapshot.gravity_body_id = "unknown-body".into();
        assert_eq!(
            verifier
                .stage(&bytes(&unknown_header_body))
                .expect_err("unknown header body is rejected before hashing")
                .code(),
            ErrorCode::BindingMismatch
        );

        let mut verifier = InterestVerifier::new(config()).expect("config is valid");
        commit_message(&mut verifier, &welcome());
        let mut invalid_registry = registry();
        let ServerMessage::Registry { registry, .. } = &mut invalid_registry else {
            unreachable!();
        };
        registry.bodies[0].center = invalid_address();
        assert_eq!(
            verifier
                .stage(&bytes(&invalid_registry))
                .expect_err("invalid registry center is rejected before commitment")
                .code(),
            ErrorCode::InvalidAddress
        );
    }

    #[test]
    fn actor_private_linkage_accepts_complete_actor_owned_structure() {
        let entities = linked_public_entities();
        let private = linked_private_snapshot();
        assert!(validate_private_linkage(&player_role(), Some(&private), &entities).is_ok());
        assert!(validate_private_linkage(&SessionRole::Spectator, None, &entities).is_ok());
    }

    #[test]
    fn actor_private_player_and_carried_inventory_must_bind_exactly() {
        let entities = linked_public_entities();
        let mut private = linked_private_snapshot();
        private.player.player_id = "other-player".into();
        assert_eq!(
            linkage_error(&private, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut no_public_player = entities.clone();
        no_public_player.retain(|entity| entity.kind != InterestEntityKind::Player);
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &no_public_player),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_public_address = entities.clone();
        let InterestEntityPayload::Player(player) = &mut wrong_public_address[2].payload else {
            unreachable!();
        };
        player.address.local_um.x += 1;
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &wrong_public_address),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_carried_owner = linked_private_snapshot();
        wrong_carried_owner.inventories[2].domain = InventoryDomain::Player {
            player_id: "other-player".into(),
        };
        assert_eq!(
            linkage_error(&wrong_carried_owner, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_carried_id = linked_private_snapshot();
        wrong_carried_id.player.inventory_id = "inventory-cargo".into();
        assert_eq!(
            linkage_error(&wrong_carried_id, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut missing_carried = linked_private_snapshot();
        missing_carried.inventories.pop();
        assert_eq!(
            linkage_error(&missing_carried, &entities),
            ErrorCode::InvalidPrivateLinkage
        );
    }

    #[test]
    fn actor_private_cargo_requires_one_visible_actor_owned_cargo_block() {
        let entities = linked_public_entities();
        let mut missing = linked_private_snapshot();
        missing.inventories[0].domain = InventoryDomain::Cargo {
            block_id: "missing-block".into(),
        };
        assert_eq!(
            linkage_error(&missing, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut foreign = entities.clone();
        let InterestEntityPayload::Grid(grid) = &mut foreign[1].payload else {
            unreachable!();
        };
        grid.owner_player_id = "other-player".into();
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &foreign),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_kind = entities.clone();
        let InterestEntityPayload::Grid(grid) = &mut wrong_kind[1].payload else {
            unreachable!();
        };
        grid.blocks[0].kind = BlockKind::Structural;
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &wrong_kind),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut ambiguous = entities.clone();
        let InterestEntityPayload::Grid(grid) = &mut ambiguous[1].payload else {
            unreachable!();
        };
        grid.blocks.push(grid.blocks[0].clone());
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &ambiguous),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut duplicate_inventory = linked_private_snapshot();
        let mut duplicate = duplicate_inventory.inventories[0].clone();
        duplicate.inventory_id = "inventory-cargo-second".into();
        duplicate_inventory.inventories.insert(1, duplicate);
        assert_eq!(
            linkage_error(&duplicate_inventory, &entities),
            ErrorCode::InvalidPrivateLinkage
        );
    }

    #[test]
    fn actor_private_drops_require_actor_inventory_and_visible_public_drop() {
        let entities = linked_public_entities();
        let mut wrong_owner = linked_private_snapshot();
        wrong_owner.death_drops[0].owner_player_id = "other-player".into();
        assert_eq!(
            linkage_error(&wrong_owner, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut no_public_drop = entities.clone();
        no_public_drop.retain(|entity| entity.kind != InterestEntityKind::DeathDrop);
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &no_public_drop),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_public_address = entities.clone();
        let InterestEntityPayload::DeathDrop(drop) = &mut wrong_public_address[0].payload else {
            unreachable!();
        };
        drop.address.local_um.x += 1;
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &wrong_public_address),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut missing_inventory = linked_private_snapshot();
        missing_inventory.death_drops[0].inventory_id = "missing-inventory".into();
        assert_eq!(
            linkage_error(&missing_inventory, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_inventory_owner = linked_private_snapshot();
        wrong_inventory_owner.inventories[1].domain = InventoryDomain::Dropped {
            reason: "player_death".into(),
            owner_player_id: "other-player".into(),
        };
        assert_eq!(
            linkage_error(&wrong_inventory_owner, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut orphan_inventory = linked_private_snapshot();
        orphan_inventory.death_drops.clear();
        assert_eq!(
            linkage_error(&orphan_inventory, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut duplicate_drop_inventory = linked_private_snapshot();
        let mut second = duplicate_drop_inventory.death_drops[0].clone();
        second.drop_id = "drop-b".into();
        duplicate_drop_inventory.death_drops.push(second);
        let mut duplicate_public = entities.clone();
        duplicate_public.insert(
            1,
            InterestEntityProjection {
                entity_id: "drop-b".into(),
                kind: InterestEntityKind::DeathDrop,
                projected_revision: 1,
                component_schema_version: PROJECTION_SCHEMA_VERSION,
                payload: InterestEntityPayload::DeathDrop(PublicDeathDropSnapshot {
                    drop_id: "drop-b".into(),
                    address: address(),
                }),
            },
        );
        assert_eq!(
            linkage_error(&duplicate_drop_inventory, &duplicate_public),
            ErrorCode::InvalidPrivateLinkage
        );
    }

    #[test]
    fn actor_private_masses_queues_and_jobs_require_visible_owned_authority() {
        let entities = linked_public_entities();
        let mut missing_mass_grid = linked_private_snapshot();
        missing_mass_grid.owned_grid_masses[0].grid_id = "missing-grid".into();
        assert_eq!(
            linkage_error(&missing_mass_grid, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut foreign_grid = entities.clone();
        let InterestEntityPayload::Grid(grid) = &mut foreign_grid[1].payload else {
            unreachable!();
        };
        grid.owner_player_id = "other-player".into();
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &foreign_grid),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut missing_machine = linked_private_snapshot();
        missing_machine.production_queues[0].machine_block_id = "missing-machine".into();
        assert_eq!(
            linkage_error(&missing_machine, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut non_machine = entities.clone();
        let InterestEntityPayload::Grid(grid) = &mut non_machine[1].payload else {
            unreachable!();
        };
        grid.blocks[1].kind = BlockKind::Structural;
        assert_eq!(
            linkage_error(&linked_private_snapshot(), &non_machine),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_job_owner = linked_private_snapshot();
        wrong_job_owner.production_queues[0].jobs[0].owner_player_id = "other-player".into();
        assert_eq!(
            linkage_error(&wrong_job_owner, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_job_machine = linked_private_snapshot();
        wrong_job_machine.production_queues[0].jobs[0].machine_block_id = "other-machine".into();
        assert_eq!(
            linkage_error(&wrong_job_machine, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        let mut wrong_recipe = linked_private_snapshot();
        wrong_recipe.production_queues[0].jobs[0].recipe = ProductionRecipeKind::Component;
        assert_eq!(
            linkage_error(&wrong_recipe, &entities),
            ErrorCode::InvalidPrivateLinkage
        );

        for endpoint in ["source", "destination"] {
            let mut missing_endpoint = linked_private_snapshot();
            let job = &mut missing_endpoint.production_queues[0].jobs[0];
            if endpoint == "source" {
                job.source_inventory_id = "missing-inventory".into();
            } else {
                job.destination_inventory_id = "missing-inventory".into();
            }
            assert_eq!(
                linkage_error(&missing_endpoint, &entities),
                ErrorCode::InvalidPrivateLinkage
            );
        }

        let mut duplicate_job = linked_private_snapshot();
        let second = duplicate_job.production_queues[0].jobs[0].clone();
        duplicate_job.production_queues[0].jobs.push(second);
        assert_eq!(
            linkage_error(&duplicate_job, &entities),
            ErrorCode::InvalidPrivateLinkage
        );
    }

    #[test]
    fn retained_private_overlay_is_revalidated_after_public_removal() {
        let mut config = VerifierConfig::new(
            player_role(),
            WORLD_SCHEMA,
            EVENT_SCHEMA,
            CONTENT_SCHEMA,
            "content-v1",
            CONTENT_HASH,
            "universe-test",
            binding_hashes().0,
            binding_hashes().1,
        );
        config.limits = ResourceLimits::default();
        let mut verifier = InterestVerifier::new(config).expect("player verifier config is valid");
        let mut player_welcome = welcome();
        let ServerMessage::Welcome { session_role, .. } = &mut player_welcome else {
            unreachable!();
        };
        *session_role = player_role();
        commit_message(&mut verifier, &player_welcome);
        commit_message(&mut verifier, &registry());
        commit_message(&mut verifier, &linked_player_baseline());
        let previous = verifier
            .committed_view()
            .expect("linked baseline commits")
            .view_hash;

        let mut removal = delta(&previous);
        let ServerMessage::InterestDelta { delta } = &mut removal else {
            unreachable!();
        };
        delta.interest.observer_class = InterestObserverClass::BoundPlayer;
        delta.interest.removed = vec![verse_protocol::InterestRemoval {
            entity_id: "grid-a".into(),
            kind: InterestEntityKind::Grid,
            reason: verse_protocol::InterestRemovalReason::OutOfInterest,
        }];
        delta.interest.view_hash = "0".repeat(64);
        assert_eq!(
            verifier
                .stage(&bytes(&removal))
                .expect_err("retained private links are invalid after grid removal")
                .code(),
            ErrorCode::InvalidPrivateLinkage
        );
        assert_eq!(
            verifier
                .committed_view()
                .expect("prior committed view is retained")
                .delta_sequence,
            0
        );
    }

    #[test]
    fn baseline_and_delta_are_staged_and_acknowledged_from_committed_state() {
        let mut verifier = ready_verifier();
        let baseline_token = verifier
            .stage(&bytes(&baseline()))
            .expect("baseline stages");
        let staged = verifier.pending_view().expect("pending view exists");
        assert_eq!(staged.delta_sequence, 0);
        assert!(verifier.committed_view().is_none());
        let baseline_commit = verifier.commit(baseline_token).expect("baseline commits");
        assert_eq!(baseline_commit.kind, StageKind::Baseline);
        assert_eq!(
            baseline_commit.acknowledgement_json.as_deref(),
            Some(concat!(
                "{\"type\":\"acknowledge_interest\",\"session_epoch\":\"session-1\",",
                "\"interest_epoch\":41,\"baseline_id\":\"baseline-1\",\"delta_sequence\":0,",
                "\"view_hash\":\"5e73febfbce403ff0da41746233e02d27aaf0bdb7b0b9a1a6196aedd6995204d\"}"
            ))
        );

        let baseline_hash = verifier.committed_view().expect("view exists").view_hash;
        let mut bad_delta = delta(&baseline_hash);
        let ServerMessage::InterestDelta { delta: value } = &mut bad_delta else {
            unreachable!();
        };
        value.interest.view_hash = "0".repeat(64);
        assert_eq!(
            verifier
                .stage(&bytes(&bad_delta))
                .expect_err("bad digest is rejected")
                .code(),
            ErrorCode::HashMismatch
        );
        assert_eq!(
            verifier
                .committed_view()
                .expect("view remains")
                .delta_sequence,
            0
        );

        let delta_token = verifier
            .stage(&bytes(&delta(&baseline_hash)))
            .expect("delta stages");
        assert_eq!(
            verifier
                .committed_view()
                .expect("committed view remains")
                .delta_sequence,
            0
        );
        let outcome = verifier.commit(delta_token).expect("delta commits");
        assert_eq!(outcome.view.expect("view summary").delta_sequence, 1);
    }

    #[test]
    fn stages_typed_receipts_and_fatal_messages_without_changing_view_state() {
        let mut verifier = ready_verifier();
        commit_message(&mut verifier, &baseline());
        let committed = verifier.committed_view().expect("baseline view");
        let receipt = ServerMessage::IntentAccepted {
            receipt: verse_protocol::IntentReceipt {
                operation_sequence: 7,
                operation_id: "operation-7".into(),
                event_sequence: 8,
                code: "accepted".into(),
                message: "done".into(),
            },
        };
        let token = verifier.stage(&bytes(&receipt)).expect("receipt stages");
        assert_eq!(verifier.pending_kind(), Some(StageKind::IntentAccepted));
        assert_eq!(verifier.pending_message(), Some(&receipt));
        let expected_json =
            String::from_utf8(bytes(&receipt)).expect("serialized receipt is UTF-8");
        assert_eq!(
            verifier.pending_sanitized_json(),
            Some(expected_json.as_str())
        );
        let outcome = verifier.commit(token).expect("receipt commits");
        assert!(outcome.acknowledgement_json.is_none());
        assert_eq!(verifier.committed_view(), Some(committed));

        let fatal = ServerMessage::Fatal {
            code: "closed".into(),
            message: "session closed".into(),
        };
        let token = verifier.stage(&bytes(&fatal)).expect("fatal stages");
        assert_eq!(verifier.pending_kind(), Some(StageKind::Fatal));
        verifier.commit(token).expect("fatal passthrough commits");
    }

    #[test]
    fn accepts_recovery_baseline_and_local_origin_rebase_but_rejects_regression() {
        let mut verifier = ready_verifier();
        commit_message(&mut verifier, &baseline());
        let old_hash = verifier.committed_view().expect("baseline view").view_hash;

        let token = verifier
            .stage(&bytes(&rebased_delta(&old_hash)))
            .expect("local-origin rebase delta stages");
        verifier.commit(token).expect("rebased delta commits");

        verifier.reset();
        commit_message(&mut verifier, &welcome());
        commit_message(&mut verifier, &registry());
        commit_message(&mut verifier, &baseline());
        let mut regressed_recovery = recovery_baseline();
        let ServerMessage::InterestBaseline { baseline: recovery } = &mut regressed_recovery else {
            unreachable!();
        };
        recovery.event_sequence = 49;
        recovery.simulation_tick = 59;
        recovery.interest.canonical_event_sequence = 49;
        recovery.interest.canonical_tick = 59;
        assert_eq!(
            verifier
                .stage(&bytes(&regressed_recovery))
                .expect_err("recovery frontier regression is rejected")
                .code(),
            ErrorCode::FrontierMismatch
        );
        let token = verifier
            .stage(&bytes(&recovery_baseline()))
            .expect("recovery baseline stages");
        let outcome = verifier.commit(token).expect("recovery baseline commits");
        let view = outcome.view.expect("recovery view");
        assert_eq!(view.interest_epoch, 42);
        assert_eq!(view.baseline_id, "baseline-2");

        verifier.reset();
        commit_message(&mut verifier, &welcome());
        commit_message(&mut verifier, &registry());
        commit_message(&mut verifier, &baseline());
        let baseline_hash = verifier.committed_view().expect("baseline view").view_hash;
        let mut regressed = delta(&baseline_hash);
        let ServerMessage::InterestDelta { delta } = &mut regressed else {
            unreachable!();
        };
        delta.event_sequence = 49;
        delta.simulation_tick = 59;
        delta.interest.canonical_event_sequence = 49;
        delta.interest.canonical_tick = 59;
        assert_eq!(
            verifier
                .stage(&bytes(&regressed))
                .expect_err("frontier regression is rejected")
                .code(),
            ErrorCode::FrontierMismatch
        );
    }

    #[test]
    fn pins_content_manifest_and_requires_player_private_baseline() {
        let mut verifier = InterestVerifier::new(config()).expect("config is valid");
        let mut incompatible = welcome();
        let ServerMessage::Welcome {
            content_manifest_version,
            ..
        } = &mut incompatible
        else {
            unreachable!();
        };
        *content_manifest_version = "other-content".into();
        assert_eq!(
            verifier
                .stage(&bytes(&incompatible))
                .expect_err("manifest mismatch is rejected")
                .code(),
            ErrorCode::IncompatibleWelcome
        );

        let mut player_config = VerifierConfig::new(
            SessionRole::Player {
                player_id: "player-a".into(),
            },
            WORLD_SCHEMA,
            EVENT_SCHEMA,
            CONTENT_SCHEMA,
            "content-v1",
            CONTENT_HASH,
            "universe-test",
            binding_hashes().0,
            binding_hashes().1,
        );
        player_config.limits = ResourceLimits::default();
        let mut player_verifier = InterestVerifier::new(player_config).expect("config is valid");
        let mut player_welcome = welcome();
        let ServerMessage::Welcome { session_role, .. } = &mut player_welcome else {
            unreachable!();
        };
        *session_role = SessionRole::Player {
            player_id: "player-a".into(),
        };
        commit_message(&mut player_verifier, &player_welcome);
        commit_message(&mut player_verifier, &registry());
        let mut player_baseline = baseline();
        let ServerMessage::InterestBaseline { baseline } = &mut player_baseline else {
            unreachable!();
        };
        baseline.interest.observer_class = InterestObserverClass::BoundPlayer;
        assert_eq!(
            player_verifier
                .stage(&bytes(&player_baseline))
                .expect_err("player baseline without private state is rejected")
                .code(),
            ErrorCode::InvalidBaseline
        );
    }

    #[test]
    fn enforces_entity_limit_on_resulting_complete_delta_map() {
        let mut bounded_config = config();
        bounded_config.limits.max_entities = 1;
        let mut verifier = InterestVerifier::new(bounded_config).expect("config is valid");
        commit_message(&mut verifier, &welcome());
        commit_message(&mut verifier, &registry());
        commit_message(&mut verifier, &baseline_with_drop());
        let previous = verifier.committed_view().expect("baseline view").view_hash;
        let mut message = delta(&previous);
        let ServerMessage::InterestDelta { delta } = &mut message else {
            unreachable!();
        };
        delta.interest.entered.push(InterestEntityProjection {
            entity_id: "drop-b".into(),
            kind: InterestEntityKind::DeathDrop,
            projected_revision: 1,
            component_schema_version: PROJECTION_SCHEMA_VERSION,
            payload: InterestEntityPayload::DeathDrop(PublicDeathDropSnapshot {
                drop_id: "drop-b".into(),
                address: address(),
            }),
        });
        assert_eq!(
            verifier
                .stage(&bytes(&message))
                .expect_err("resulting two-entity view exceeds the one-entity bound")
                .code(),
            ErrorCode::ResourceLimit
        );
        assert_eq!(
            verifier
                .committed_view()
                .expect("view remains")
                .entity_count,
            1
        );
    }

    #[test]
    fn discard_and_reset_invalidate_tokens_without_ack_or_state_change() {
        let mut verifier = ready_verifier();
        let token = verifier
            .stage(&bytes(&baseline()))
            .expect("baseline stages");
        verifier
            .discard(token)
            .expect("pending transition discards");
        assert!(verifier.committed_view().is_none());

        let token = verifier
            .stage(&bytes(&baseline()))
            .expect("baseline stages again");
        verifier.reset();
        assert_eq!(
            verifier
                .commit(token)
                .expect_err("reset invalidates token")
                .code(),
            ErrorCode::InvalidStageToken
        );
        assert!(verifier.committed_view().is_none());
    }

    #[test]
    fn enforces_resource_bounds_and_rejects_legacy_messages() {
        let mut tiny = config();
        tiny.limits.max_frame_bytes = 8;
        let mut verifier = InterestVerifier::new(tiny).expect("limits are valid");
        assert_eq!(
            verifier
                .stage(&bytes(&welcome()))
                .expect_err("oversize frame is rejected")
                .code(),
            ErrorCode::FrameTooLarge
        );

        let mut verifier = ready_verifier();
        let ServerMessage::InterestBaseline { baseline } = baseline() else {
            unreachable!();
        };
        let legacy = ServerMessage::Snapshot { snapshot: baseline };
        assert_eq!(
            verifier
                .stage(&bytes(&legacy))
                .expect_err("legacy snapshot is rejected")
                .code(),
            ErrorCode::LegacyMessage
        );
    }

    #[test]
    fn empty_spectator_view_hash_is_frozen() {
        assert_eq!(
            view_state(0).view_hash,
            "5e73febfbce403ff0da41746233e02d27aaf0bdb7b0b9a1a6196aedd6995204d"
        );
    }
}
