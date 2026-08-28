// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use thiserror::Error;
use verse_physics::{
    BodyCollisionClass, BodyControl, BodySpec, BoxColliderSpec, CapsuleCast, CapsuleColliderSpec,
    MotionQuality, PhysicsError, Pose as PhysicsPose, Quat as PhysicsQuat, Scene, SceneConfig,
    SphereColliderSpec, Vec3 as PhysicsVec3,
};
use verse_protocol::{
    BlockKind, CareerSnapshot, ClientMessage, INTENT_FINGERPRINT_SCHEMA_VERSION, IVec3,
    IntentReceipt, InventoryContents, InventoryDomain, LocomotionKind, LocomotionSupportSnapshot,
    MotionSnapshot, PROTOCOL_VERSION, PlayerDeathCause, PlayerLifeState, PlayerLocomotionSnapshot,
    ProductionRecipeKind, Quat, ResourceKind, Vec3, WorldSnapshot,
};

use crate::content;
use crate::event::{
    CanonicalEvent, EVENT_SCHEMA_NAME, EVENT_SCHEMA_VERSION, EventPayload,
    PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION, PhysicsBodyOutcome, PhysicsContactOutcome,
    PhysicsContactPhase, PlayerPhysicsOutcome, ProductionMachineOutcome,
    ProductionMachineOutcomeKind, ProductionScheduleOccurrence,
};
use crate::handoff::{
    stage_aborted_eva_unlock, stage_committed_eva_export, stage_committed_eva_import,
    stage_eva_player_quarantine, stage_prepared_eva_lock,
};
#[cfg(test)]
use crate::model::PLAYER_INVENTORY_ID;
use crate::model::{
    Block, CARGO_INVENTORY_CAPACITY_LITERS, ContactPairKey, DeathDrop, Grid, InventoryRecord,
    Player, PlayerControlFrame, ProcessedOperationRecord, ProductionJob, WORLD_SCHEMA_VERSION,
    WorldState, inventory_can_add_contents, planet_center, planet_surface_radius_m,
    production_recipe_quantities, radial_up, valid_blake3_hex,
};
use crate::persistence::{
    CellLifecycleStatus, PersistenceError, Store, SystemTrustedClock, TrustedClock,
};
#[cfg(test)]
use crate::targeting::{TOOL_SURFACE_RANGE_M, ToolHit};
use crate::targeting::{ToolTarget, closest_tool_hit};

pub const MAX_BACKGROUND_QUEUE_BEARING_MACHINES: usize = 256;
pub const MAX_PRODUCTION_CATCH_UP_QUANTA: usize = 60;
pub const MAX_PRODUCTION_CATCH_UP_MILLIS: u128 = 250;

#[cfg(test)]
const PLAYER_BODY_ID: &str = "player-body-player-local";
#[cfg(test)]
const PLAYER_COLLIDER_ID: &str = "player-collider-player-local";
const PLANET_BODY_ID: &str = "planet-body-khepri-prime";
const PLANET_COLLIDER_ID: &str = "planet-collider-khepri-prime";
const MAX_GRID_CONTROL_INPUT: f64 = 1.0;
const CONTROL_INPUT_EPSILON: f64 = 1.0e-9;
// Godot's standard Vector3 uses float32 components. Normalizing in float32 and
// reconstructing those components as JSON float64 can put the calculated
// magnitude a few ULPs above one even though the source vector was valid.
const CONTROL_INPUT_SOURCE_PRECISION_EPSILON: f64 = 8.0 * f32::EPSILON as f64;
const MAX_GRID_BLOCKS_P0: usize = 2_048;
const MAX_PENDING_PLAYER_CONTROL_FRAMES: usize = 64;
const PLAYER_POSITION_CORRECTION_BUDGET_M_PER_STEP: f64 = 0.55;
const PLAYER_ROTATION_SLOP_RADIANS_PER_STEP: f64 = 0.000_1;
const REPLAY_QUANTIZATION_SLOP: f64 = 0.000_004;
#[cfg(test)]
const REPLAY_CONTACT_SLOP_M: f64 = 0.15;
const PHYSICS_MINIMUM_SPECULATIVE_DISTANCE_M: f64 = 0.02;
const PHYSICS_CONTACT_POINT_SLOP_M: f64 = 0.001;
const PLAYER_PLANET_PENETRATION_LIMIT_M: f64 = 0.28;
const PLAYER_BOX_PENETRATION_LIMIT_M: f64 = 0.85;
const CHARACTER_INERTIA_MULTIPLIER: f64 = 12.0;

#[derive(Serialize)]
struct IntentFingerprintMaterial<'a> {
    domain: &'static str,
    protocol_version: u32,
    fingerprint_schema_version: u32,
    world_schema_version: u32,
    event_schema_version: u32,
    universe_id: &'a str,
    actor_player_id: &'a str,
    message: &'a serde_json::Value,
}

#[derive(Debug, Clone)]
struct OperationEventMetadata {
    operation_id: String,
    operation_sequence: u64,
    intent_fingerprint: String,
}

fn player_body_id(player_id: &str) -> String {
    format!("player-body-{player_id}")
}

fn player_collider_id(player_id: &str) -> String {
    format!("player-collider-{player_id}")
}

fn client_message_floats_are_finite(message: &ClientMessage) -> bool {
    let finite = |value: Vec3| value.x.is_finite() && value.y.is_finite() && value.z.is_finite();
    match message {
        ClientMessage::SetPlayerControl {
            linear_input,
            angular_input,
            ..
        }
        | ClientMessage::SetGridControl {
            linear_input,
            angular_input,
            ..
        } => finite(*linear_input) && finite(*angular_input),
        ClientMessage::Hello { .. }
        | ClientMessage::RequestSnapshot
        | ClientMessage::AcknowledgeInterest { .. }
        | ClientMessage::SetSuitMode { .. }
        | ClientMessage::RespawnPlayer { .. }
        | ClientMessage::MineVoxel { .. }
        | ClientMessage::RefineOre { .. }
        | ClientMessage::CraftComponent { .. }
        | ClientMessage::QueueProduction { .. }
        | ClientMessage::TransferInventory { .. }
        | ClientMessage::BuildBlock { .. }
        | ClientMessage::WeldBlock { .. }
        | ClientMessage::ToggleGridAnchor { .. }
        | ClientMessage::DamageBlock { .. } => true,
    }
}

fn normalize_json_signed_zero(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number)
            if number
                .as_f64()
                .is_some_and(|value| value == 0.0 && value.is_sign_negative()) =>
        {
            *number =
                serde_json::Number::from_f64(0.0).expect("finite canonical zero is a JSON number");
        }
        serde_json::Value::Array(values) => {
            for value in values {
                normalize_json_signed_zero(value);
            }
        }
        serde_json::Value::Object(map) => {
            for value in map.values_mut() {
                normalize_json_signed_zero(value);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IntentError {
    #[error("{code}: {message}")]
    Rejected { code: String, message: String },
    #[error("event sequence mismatch: expected {expected}, received {received}")]
    SequenceMismatch { expected: u64, received: u64 },
    #[error("event previous hash does not match authoritative state")]
    PreviousHashMismatch,
    #[error("event hash is invalid")]
    InvalidEventHash,
    #[error("event belongs to a different universe or cell")]
    WrongAuthority,
    #[error("event content manifest does not match authoritative state")]
    ContentManifestMismatch,
    #[error("conservation invariant failed after event {event_sequence}")]
    ConservationViolation { event_sequence: u64 },
}

impl IntentError {
    fn rejected(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Rejected {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn code(&self) -> &str {
        match self {
            Self::Rejected { code, .. } => code,
            Self::SequenceMismatch { .. } => "event_sequence_mismatch",
            Self::PreviousHashMismatch => "event_previous_hash_mismatch",
            Self::InvalidEventHash => "event_hash_invalid",
            Self::WrongAuthority => "event_wrong_authority",
            Self::ContentManifestMismatch => "event_content_manifest_mismatch",
            Self::ConservationViolation { .. } => "conservation_violation",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Rejected { message, .. } => message.clone(),
            _ => self.to_string(),
        }
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Intent(#[from] IntentError),
    #[error(transparent)]
    Physics(#[from] PhysicsError),
    #[error("authoritative writes are halted after a persistence failure")]
    Halted,
    #[error("cell lifecycle mode {mode:?} does not permit this operation")]
    LifecycleUnavailable {
        mode: crate::persistence::LifecycleMode,
    },
    #[error("canonical world invariant failed: {0}")]
    CanonicalInvariant(String),
}

/// The strongest network-visible state class committed while advancing
/// authoritative time. Structural state subsumes motion because a complete
/// snapshot must precede any later lightweight motion state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AdvanceImpact {
    #[default]
    None,
    Motion,
    Structural,
}

impl AdvanceImpact {
    #[must_use]
    pub const fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Structural, _) | (_, Self::Structural) => Self::Structural,
            (Self::Motion, _) | (_, Self::Motion) => Self::Motion,
            (Self::None, Self::None) => Self::None,
        }
    }
}

/// Result of one elapsed-time advance. Runtime callers that only need the
/// historical changed/unchanged contract may continue using [`Runtime::advance`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AdvanceOutcome {
    pub impact: AdvanceImpact,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProductionDispatchOutcome {
    pub committed_quanta: usize,
    pub backlog_remaining: bool,
}

#[derive(Debug, Clone)]
pub struct RuntimeOpenConfig {
    data_directory: PathBuf,
    requested_seed: u64,
    snapshot_every: u64,
    clock: Arc<dyn TrustedClock>,
}

impl AdvanceOutcome {
    #[must_use]
    pub const fn changed(self) -> bool {
        !matches!(self.impact, AdvanceImpact::None)
    }

    fn record(&mut self, impact: AdvanceImpact) {
        self.impact = self.impact.combine(impact);
    }
}

#[derive(Debug)]
pub struct Runtime {
    store: Store,
    state: WorldState,
    snapshot_every: u64,
    events_since_snapshot: u64,
    life_support_elapsed_millis_by_player: BTreeMap<String, u32>,
    physics_step_phase: u64,
    physics: Option<Scene>,
    halted: bool,
    #[cfg(test)]
    physics_full_rebuilds: u64,
    #[cfg(test)]
    physics_chunk_replacements: u64,
}

impl Runtime {
    pub fn open(
        data_directory: impl AsRef<Path>,
        requested_seed: u64,
        snapshot_every: u64,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_clock(
            data_directory,
            requested_seed,
            snapshot_every,
            Arc::new(SystemTrustedClock),
        )
    }

    pub fn open_hosted(
        data_directory: impl AsRef<Path>,
        requested_seed: u64,
        snapshot_every: u64,
    ) -> Result<Self, RuntimeError> {
        Self::open_hosted_with_clock(
            data_directory,
            requested_seed,
            snapshot_every,
            Arc::new(SystemTrustedClock),
        )
    }

    pub fn open_hosted_with_clock(
        data_directory: impl AsRef<Path>,
        requested_seed: u64,
        snapshot_every: u64,
        clock: Arc<dyn TrustedClock>,
    ) -> Result<Self, RuntimeError> {
        let mut runtime = Self::open_for_activation_with_clock(
            data_directory,
            requested_seed,
            snapshot_every,
            clock,
        )?;
        runtime.store.restore_recovered_host_mode(&runtime.state)?;
        Ok(runtime)
    }

    pub fn open_with_clock(
        data_directory: impl AsRef<Path>,
        requested_seed: u64,
        snapshot_every: u64,
        clock: Arc<dyn TrustedClock>,
    ) -> Result<Self, RuntimeError> {
        let mut runtime = Self::open_for_activation_with_clock(
            data_directory,
            requested_seed,
            snapshot_every,
            clock,
        )?;
        while !runtime.activation_step()? {}
        Ok(runtime)
    }

    pub fn open_for_activation(config: &RuntimeOpenConfig) -> Result<Self, RuntimeError> {
        Self::open_for_activation_with_clock(
            &config.data_directory,
            config.requested_seed,
            config.snapshot_every,
            Arc::clone(&config.clock),
        )
    }

    fn open_for_activation_with_clock(
        data_directory: impl AsRef<Path>,
        requested_seed: u64,
        snapshot_every: u64,
        clock: Arc<dyn TrustedClock>,
    ) -> Result<Self, RuntimeError> {
        let mut store = Store::open_with_clock(data_directory, requested_seed, clock)?;
        let mut state = store.load_world()?;
        state.fencing_token = store.fencing_token();

        let physics_step_phase = state.physics_step_phase;
        let life_support_elapsed_millis_by_player = state
            .player
            .by_id
            .keys()
            .map(|player_id| (player_id.clone(), 0))
            .collect();
        let mut runtime = Self {
            store,
            state,
            snapshot_every: snapshot_every.max(1),
            events_since_snapshot: 0,
            life_support_elapsed_millis_by_player,
            physics_step_phase,
            physics: None,
            halted: false,
            #[cfg(test)]
            physics_full_rebuilds: 0,
            #[cfg(test)]
            physics_chunk_replacements: 0,
        };
        if runtime.state.event_sequence == 0 {
            runtime.store.save_snapshot(&runtime.state)?;
        }
        Ok(runtime)
    }

    fn initialize_physics(&mut self) -> Result<(), RuntimeError> {
        if self.physics.is_some() {
            return Ok(());
        }
        let mut physics = Scene::new(physics_scene_config())?;
        physics.rebuild(&physics_body_specs(&self.state))?;
        self.physics = Some(physics);
        Ok(())
    }

    #[cfg(test)]
    fn physics(&self) -> &Scene {
        self.physics
            .as_ref()
            .expect("active test runtime has an initialized physics scene")
    }

    #[cfg(test)]
    fn physics_mut(&mut self) -> &mut Scene {
        self.physics
            .as_mut()
            .expect("active test runtime has an initialized physics scene")
    }

    #[cfg(test)]
    fn rebuild_physics_for_test(&mut self) {
        let body_specs = physics_body_specs(&self.state);
        self.physics_mut()
            .rebuild(&body_specs)
            .expect("test fixture must produce a valid physics scene");
    }

    pub fn open_config(&self) -> RuntimeOpenConfig {
        RuntimeOpenConfig {
            data_directory: self.store.root_path().to_path_buf(),
            requested_seed: self.state.world_seed,
            snapshot_every: self.snapshot_every,
            clock: self.store.clock(),
        }
    }

    pub const fn state(&self) -> &WorldState {
        &self.state
    }

    pub const fn physics_scene_is_initialized(&self) -> bool {
        self.physics.is_some()
    }

    pub fn next_production_occurrence(&self) -> Option<&ProductionScheduleOccurrence> {
        self.store.next_production_occurrence()
    }

    pub fn lifecycle_status(&self) -> CellLifecycleStatus {
        self.store.lifecycle_status()
    }

    /// Adds a deterministic loopback-development actor before the first
    /// canonical event. This is a server-authorized fixture, never a gameplay
    /// hello side effect, and cannot rewrite an active universe history.
    pub fn admit_development_player(&mut self, player_id: &str) -> Result<bool, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        if let Err(message) = self.state.validate_player_roster() {
            self.halted = true;
            return Err(RuntimeError::CanonicalInvariant(message));
        }
        if self.state.player.by_id.contains_key(player_id) {
            return Ok(false);
        }
        if self.state.event_sequence != 0 {
            return Err(IntentError::rejected(
                "development_admission_requires_fresh_world",
                "development players can be pre-admitted only before the first canonical event",
            )
            .into());
        }

        let mut next_state = self.state.clone();
        let inventory_id = format!("inventory-{player_id}");
        if next_state.inventories.contains_key(&inventory_id) {
            return Err(IntentError::rejected(
                "development_admission_inventory_conflict",
                "development player inventory identity already exists",
            )
            .into());
        }
        let mut player = next_state.player.primary().clone();
        player_id.clone_into(&mut player.player_id);
        let roster_offset =
            u32::try_from(next_state.player.by_id.len()).map_or(f64::from(u32::MAX), f64::from);
        // Keep the loopback co-op fixture inside the starter asteroid's hand-
        // tool envelope while giving each suit a visibly distinct spawn. The
        // character collision layer already prevents these nearby capsules
        // from pushing or becoming locomotion support for one another.
        player.position.y += 1.5 * roster_offset;
        player.address = next_state
            .address_for_active_position(player.position)
            .map_err(|message| {
                IntentError::rejected("development_admission_address_invalid", message)
            })?;
        player.orientation = Quat::IDENTITY;
        player.linear_velocity = Vec3::ZERO;
        player.angular_velocity = Vec3::ZERO;
        player.surface_contact = false;
        player.locomotion.kind = LocomotionKind::Eva;
        player.locomotion.up = radial_up(player.position);
        player.locomotion.view_pitch_radians = 0.0;
        player.locomotion.support = None;
        player.locomotion.jump_held = false;
        player.locomotion.jump_buffer_expires_at_simulation_tick = 0;
        player.locomotion.support_grace_expires_at_simulation_tick = 0;
        player.locomotion.magnetic_boots_enabled = false;
        player.locomotion.magnetic_reattach_after_simulation_tick = 0;
        player.movement_epoch = 1;
        player.last_received_input_sequence = 0;
        player.last_processed_input_sequence = 0;
        player.pending_control_frames.clear();
        player.control_linear_input = Vec3::ZERO;
        player.control_angular_input = Vec3::ZERO;
        player.boost = false;
        player.dampeners = true;
        player.jump = false;
        player.control_expires_at_simulation_tick = 0;
        player.inventory_id.clone_from(&inventory_id);
        player.experience = 0;
        player.career = CareerSnapshot::default();
        player.suit_oxygen_milli = 1_000;
        player.helmet_closed = true;
        player.jetpack_enabled = true;
        player.life_state = PlayerLifeState::Alive;
        next_state.player.by_id.insert(player_id.to_owned(), player);
        next_state.inventories.insert(
            inventory_id.clone(),
            InventoryRecord {
                inventory_id,
                domain: InventoryDomain::Player {
                    player_id: player_id.to_owned(),
                },
                contents: InventoryContents::default(),
                capacity_liters: crate::model::PLAYER_INVENTORY_CAPACITY_LITERS,
            },
        );
        next_state
            .validate_player_roster()
            .map_err(|message| IntentError::rejected("development_admission_invalid", message))?;
        if !next_state.conservation().valid {
            return Err(IntentError::ConservationViolation {
                event_sequence: next_state.event_sequence,
            }
            .into());
        }
        let next_physics =
            if self.store.lifecycle_mode() == crate::persistence::LifecycleMode::Active {
                let mut physics = Scene::new(physics_scene_config())?;
                physics.rebuild(&physics_body_specs(&next_state))?;
                Some(physics)
            } else {
                None
            };
        self.store.save_snapshot(&next_state)?;
        self.state = next_state;
        self.physics = next_physics;
        self.life_support_elapsed_millis_by_player
            .insert(player_id.to_owned(), 0);
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn relocate_player_for_test(&mut self, position: Vec3) {
        self.state.player.address = self
            .state
            .address_for_active_position(position)
            .expect("test relocation has an exact active-cell address");
        self.state.player.position = self
            .state
            .active_position_for_address(&self.state.player.address)
            .expect("test relocation address hydrates an exact pose");
        self.state.player.orientation = Quat::IDENTITY;
        self.state.player.linear_velocity = Vec3::ZERO;
        self.state.player.angular_velocity = Vec3::ZERO;
        self.state.player.surface_contact = false;
        self.rebuild_physics_for_test();
    }

    #[cfg(test)]
    pub(crate) fn aim_player_for_test(&mut self, target: Vec3, outward_face: Vec3) {
        let face = outward_face * outward_face.magnitude().recip();
        let eye = target + face * 4.0;
        let forward = (target - eye) * (target - eye).magnitude().recip();
        let dot = -forward.z;
        let orientation = if dot < -1.0 + 1.0e-9 {
            Quat::new(0.0, 1.0, 0.0, 0.0)
        } else {
            let x = forward.y;
            let y = -forward.x;
            let w = 1.0 + dot;
            let inverse_length = x.mul_add(x, y.mul_add(y, w * w)).sqrt().recip();
            Quat::new(
                (x * inverse_length) as f32,
                (y * inverse_length) as f32,
                0.0,
                (w * inverse_length) as f32,
            )
        };
        let eye_offset = content::manifest().character.eye_height_m
            - content::manifest().character.standing_height_m * 0.5;
        let position = eye - Vec3::new(0.0, eye_offset, 0.0);
        self.state.player.address = self
            .state
            .address_for_active_position(position)
            .expect("test aim has an exact active-cell address");
        self.state.player.position = self
            .state
            .active_position_for_address(&self.state.player.address)
            .expect("test aim address hydrates an exact pose");
        self.state.player.orientation = orientation;
        self.state.player.linear_velocity = Vec3::ZERO;
        self.state.player.angular_velocity = Vec3::ZERO;
        self.state.player.surface_contact = false;
        self.state.player.locomotion.kind = LocomotionKind::Airborne;
        self.state.player.locomotion.up = Vec3::new(0.0, 1.0, 0.0);
        self.state.player.locomotion.view_pitch_radians = 0.0;
        self.rebuild_physics_for_test();
    }

    pub const fn is_halted(&self) -> bool {
        self.halted
    }

    pub fn snapshot(&self) -> WorldSnapshot {
        self.state.snapshot()
    }

    pub fn motion_snapshot(&self) -> MotionSnapshot {
        self.state.motion_snapshot()
    }

    pub fn execute(&mut self, message: &ClientMessage) -> Result<IntentReceipt, RuntimeError> {
        let actor_player_id = self.state.player.player_id.clone();
        self.execute_as(&actor_player_id, message)
    }

    /// Execute one client intent for the player identity already authenticated
    /// by the connection boundary. The actor is deliberately separate from the
    /// client payload so changing JSON fields cannot select another player.
    pub fn execute_as(
        &mut self,
        actor_player_id: &str,
        message: &ClientMessage,
    ) -> Result<IntentReceipt, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        if self.store.lifecycle_mode() != crate::persistence::LifecycleMode::Active {
            return Err(RuntimeError::LifecycleUnavailable {
                mode: self.store.lifecycle_mode(),
            });
        }
        if let Err(message) = self.state.validate_player_roster() {
            self.halted = true;
            return Err(RuntimeError::CanonicalInvariant(message));
        }
        if !self.state.player.by_id.contains_key(actor_player_id) {
            return Err(IntentError::rejected(
                "actor_not_present",
                "the authenticated player is not present in this simulation cell",
            )
            .into());
        }
        if self
            .state
            .player_transfer_locks
            .contains_key(actor_player_id)
        {
            return Err(IntentError::rejected(
                "actor_transfer_locked",
                "the player is in an authoritative cross-cell handoff",
            )
            .into());
        }
        let operation_id = message.operation_id().ok_or_else(|| {
            IntentError::rejected(
                "not_a_mutating_intent",
                "hello and snapshot requests are handled by the network service",
            )
        })?;
        if operation_id.trim().is_empty() || operation_id.len() > 128 {
            return Err(IntentError::rejected(
                "invalid_operation_id",
                "operation ID must contain between 1 and 128 characters",
            )
            .into());
        }
        let operation_sequence = message.operation_sequence().ok_or_else(|| {
            IntentError::rejected(
                "not_a_mutating_intent",
                "hello and snapshot requests have no operation sequence",
            )
        })?;
        let intent_fingerprint = self
            .state
            .client_intent_fingerprint(actor_player_id, message)?;
        if let Some(receipt) = self.state.validate_operation_attempt(
            actor_player_id,
            operation_sequence,
            &intent_fingerprint,
        )? {
            return Ok(receipt);
        }

        let mut event = self
            .state
            .prepare_client_event_as(actor_player_id, message)?;
        let occurred_at_unix_ms = match self.store.accepted_trusted_time() {
            Ok(now) => now,
            Err(source) => {
                self.halted = true;
                return Err(source.into());
            }
        };
        event.retime_and_rehash(occurred_at_unix_ms);
        let prior_production_runnable = self.state.background_production_is_runnable()?;
        let mut next_state = self.state.clone();
        next_state.apply_event(&event)?;
        let resulting_next_occurrence = self.resulting_next_production_occurrence(
            prior_production_runnable,
            &next_state,
            &event,
        )?;
        let lifecycle_mode = self.store.lifecycle_mode();
        let physics = self
            .physics
            .as_mut()
            .ok_or(RuntimeError::LifecycleUnavailable {
                mode: lifecycle_mode,
            })?;
        if let EventPayload::VoxelMined { coordinate, .. } = &event.payload {
            let chunk = voxel_collision_chunk_coordinate(*coordinate);
            let body_id = voxel_collision_chunk_body_id(chunk);
            physics.replace_body(
                &body_id,
                voxel_collision_chunk_body_spec(&next_state, chunk),
            )?;
            #[cfg(test)]
            {
                self.physics_chunk_replacements += 1;
            }
        } else if event_changes_physics_scene(&event.payload) {
            physics.rebuild(&physics_body_specs(&next_state))?;
            #[cfg(test)]
            {
                self.physics_full_rebuilds += 1;
            }
        }
        if let Err(source) =
            self.store
                .commit_world_event(&event, &next_state, resulting_next_occurrence)
        {
            self.halted = true;
            return Err(source.into());
        }
        self.state = next_state;
        self.after_event()?;

        self.state
            .processed_operation_record(actor_player_id, operation_sequence)
            .map(|record| record.receipt.clone())
            .ok_or_else(|| {
                IntentError::rejected(
                    "receipt_missing",
                    "accepted operation did not produce a durable receipt",
                )
                .into()
            })
    }

    #[cfg(test)]
    pub(crate) fn execute_next_for_fixture(
        &mut self,
        message: &ClientMessage,
    ) -> Result<IntentReceipt, RuntimeError> {
        let actor_player_id = self.state.player.player_id.clone();
        self.execute_next_as_for_fixture(&actor_player_id, message)
    }

    #[cfg(test)]
    pub(crate) fn execute_next_as_for_fixture(
        &mut self,
        actor_player_id: &str,
        message: &ClientMessage,
    ) -> Result<IntentReceipt, RuntimeError> {
        let mut sequenced = message.clone();
        if sequenced.operation_sequence() == Some(0) {
            let retained_sequence = sequenced.operation_id().and_then(|operation_id| {
                self.state
                    .processed_operations
                    .get(actor_player_id)
                    .and_then(|history| {
                        history
                            .retained
                            .values()
                            .find(|record| record.operation_id == operation_id)
                    })
                    .map(|record| record.receipt.operation_sequence)
            });
            let sequence = retained_sequence.unwrap_or_else(|| {
                self.state
                    .last_operation_sequence(actor_player_id)
                    .checked_add(1)
                    .expect("fixture operation sequence remains available")
            });
            assert!(sequenced.set_operation_sequence(sequence));
        }
        self.execute_as(actor_player_id, &sequenced)
    }

    pub fn advance(&mut self, delta_millis: u16) -> Result<bool, RuntimeError> {
        self.advance_with_outcome(delta_millis)
            .map(AdvanceOutcome::changed)
    }

    pub fn advance_with_outcome(
        &mut self,
        delta_millis: u16,
    ) -> Result<AdvanceOutcome, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        if self.store.lifecycle_mode() != crate::persistence::LifecycleMode::Active {
            return Err(RuntimeError::LifecycleUnavailable {
                mode: self.store.lifecycle_mode(),
            });
        }
        if let Err(source) = self.store.renew_lease() {
            self.halted = true;
            return Err(source.into());
        }
        let moving_grid = self.state.grids.values().any(|grid| {
            !grid.anchored
                && (grid.linear_velocity.magnitude() > f64::EPSILON
                    || grid.angular_velocity.magnitude() > f64::EPSILON
                    || grid.control_linear_input.magnitude() > f64::EPSILON
                    || grid.control_angular_input.magnitude() > f64::EPSILON)
        });
        let player_physics_active = self.state.player.iter().any(|(player_id, player)| {
            !self.state.player_transfer_locks.contains_key(player_id)
                && matches!(player.life_state, PlayerLifeState::Alive)
                && (player.linear_velocity.magnitude() > f64::EPSILON
                    || player.angular_velocity.magnitude() > f64::EPSILON
                    || player.control_linear_input.magnitude() > f64::EPSILON
                    || player.control_angular_input.magnitude() > f64::EPSILON
                    || !player.pending_control_frames.is_empty()
                    || player.boost
                    || self.state.simulation_tick < player.control_expires_at_simulation_tick
                    || !player.dampeners
                    || !player.jetpack_enabled)
        });
        let physics_active = moving_grid || player_physics_active;
        let delta_millis = delta_millis.clamp(1, 250);
        let mut outcome = AdvanceOutcome::default();
        if physics_active {
            let fixed_step_hz = content::manifest().physics.fixed_step_hz;
            self.physics_step_phase = self
                .physics_step_phase
                .saturating_add(u64::from(delta_millis) * 1_000_000 * u64::from(fixed_step_hz));
            let step_count = (self.physics_step_phase / 1_000_000_000).min(15);
            if step_count > 0 {
                self.physics_step_phase -= step_count * 1_000_000_000;
                let lifecycle_mode = self.store.lifecycle_mode();
                let physics = self
                    .physics
                    .as_mut()
                    .ok_or(RuntimeError::LifecycleUnavailable {
                        mode: lifecycle_mode,
                    })?;
                let mut body_states = match physics.body_states() {
                    Ok(bodies) => bodies,
                    Err(source) => {
                        self.halted = true;
                        return Err(source.into());
                    }
                };
                let mut output = None;
                let mut contacts = Vec::new();
                let mut active_contacts = self.state.active_contact_pairs.clone();
                let mut scheduled_players = self.state.player.by_id.clone();
                scheduled_players.retain(|player_id, _| {
                    !self.state.player_transfer_locks.contains_key(player_id)
                });
                for substep_index in 0..step_count {
                    let substep_simulation_tick =
                        self.state.simulation_tick.saturating_add(substep_index);
                    let mut player_jumps = BTreeMap::new();
                    for (player_id, scheduled_player) in &mut scheduled_players {
                        advance_player_control_for_substep(
                            scheduled_player,
                            substep_simulation_tick,
                        );
                        adjust_grounded_capsule_for_substep(
                            &self.state,
                            physics,
                            scheduled_player,
                            &mut body_states,
                            substep_simulation_tick,
                        )?;
                        let jump = classify_player_locomotion_for_substep(
                            &self.state,
                            &*physics,
                            scheduled_player,
                            &body_states,
                            substep_simulation_tick,
                        )?;
                        player_jumps.insert(player_id.clone(), jump);
                    }
                    let mut controls = Vec::new();
                    for (index, (player_id, scheduled_player)) in
                        scheduled_players.iter().enumerate()
                    {
                        controls.extend(physics_controls(
                            &self.state,
                            scheduled_player,
                            &body_states,
                            substep_simulation_tick,
                            player_jumps.get(player_id).copied().flatten(),
                            index == 0,
                        ));
                    }
                    let step = match physics.step(&controls) {
                        Ok(step) => step,
                        Err(source) => {
                            self.halted = true;
                            return Err(source.into());
                        }
                    };
                    for scheduled_player in scheduled_players.values() {
                        let player_body_id = player_body_id(&scheduled_player.player_id);
                        if let (Some(prior), Some(result)) = (
                            body_states
                                .iter()
                                .find(|body| body.body_id == player_body_id),
                            step.bodies
                                .iter()
                                .find(|body| body.body_id == player_body_id),
                        ) && let Err(source) = ensure_player_fixed_step_envelope(
                            from_physics_vec3(prior.pose.position),
                            from_physics_quat(prior.pose.rotation),
                            from_physics_vec3(result.pose.position),
                            from_physics_quat(result.pose.rotation),
                            &physics_scene_config(),
                        ) {
                            self.halted = true;
                            return Err(source.into());
                        }
                    }
                    for grid in self.state.grids.values().filter(|grid| !grid.anchored) {
                        if let (Some(prior), Some(result)) = (
                            body_states.iter().find(|body| body.body_id == grid.grid_id),
                            step.bodies.iter().find(|body| body.body_id == grid.grid_id),
                        ) && let Err(source) = ensure_dynamic_body_fixed_step_envelope(
                            from_physics_vec3(prior.pose.position),
                            from_physics_quat(prior.pose.rotation),
                            from_physics_vec3(result.pose.position),
                            from_physics_quat(result.pose.rotation),
                            grid_local_center_of_mass(&self.state, grid),
                            &physics_scene_config(),
                        ) {
                            self.halted = true;
                            return Err(source.into());
                        }
                    }
                    let substep_index =
                        u8::try_from(substep_index).expect("bounded physics substep index fits u8");
                    let current_contacts = step
                        .contacts
                        .iter()
                        .map(contact_pair_key)
                        .collect::<BTreeSet<_>>();
                    contacts.extend(
                        step.contacts
                            .iter()
                            .map(|contact| {
                                let key = contact_pair_key(contact);
                                let phase = if active_contacts.contains(&key) {
                                    PhysicsContactPhase::Persisted
                                } else {
                                    PhysicsContactPhase::Began
                                };
                                physics_contact_outcome(&self.state, contact, substep_index, phase)
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                    active_contacts = current_contacts;
                    body_states.clone_from(&step.bodies);
                    output = Some(step);
                }
                let output = output.expect("positive physics step count produces output");
                let payload = EventPayload::PhysicsStepCommitted {
                    fixed_step_hz,
                    step_count: u8::try_from(step_count)
                        .expect("bounded physics step count fits u8"),
                    remaining_step_phase: u32::try_from(self.physics_step_phase)
                        .expect("substep phase fits u32"),
                    bodies: output
                        .bodies
                        .iter()
                        .filter(|body| self.state.grids.contains_key(&body.body_id))
                        .map(|body| physics_body_outcome(&self.state, body))
                        .collect::<Result<Vec<_>, _>>()?,
                    players: scheduled_players
                        .values()
                        .filter_map(|scheduled_player| {
                            let player_body_id = player_body_id(&scheduled_player.player_id);
                            output
                                .bodies
                                .iter()
                                .find(|body| body.body_id == player_body_id)
                                .map(|body| {
                                    player_physics_outcome(
                                        &self.state,
                                        scheduled_player,
                                        body,
                                        active_contacts.iter().any(|contact| {
                                            contact_key_involves_player_id(
                                                contact,
                                                &scheduled_player.player_id,
                                            )
                                        }),
                                        self.state.simulation_tick.saturating_add(step_count),
                                    )
                                })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    contacts,
                    active_contacts_after: active_contacts.into_iter().collect(),
                };
                if let Err(source) = self.commit_system_event(payload) {
                    self.halted = true;
                    return Err(source);
                }
                let lifecycle_mode = self.store.lifecycle_mode();
                let physics = self
                    .physics
                    .as_mut()
                    .ok_or(RuntimeError::LifecycleUnavailable {
                        mode: lifecycle_mode,
                    })?;
                if let Err(source) = physics.rebuild(&physics_body_specs(&self.state)) {
                    self.halted = true;
                    return Err(source.into());
                }
                #[cfg(test)]
                {
                    self.physics_full_rebuilds += 1;
                }
                outcome.record(AdvanceImpact::Motion);
            }
        }

        let player_ids = self
            .state
            .player
            .by_id
            .keys()
            .filter(|player_id| !self.state.player_transfer_locks.contains_key(*player_id))
            .cloned()
            .collect::<Vec<_>>();
        let mut elapsed_seconds_by_player = BTreeMap::new();
        for player_id in &player_ids {
            let player = self
                .state
                .player
                .get(player_id)
                .expect("roster identity collected from canonical state");
            let elapsed_millis = self
                .life_support_elapsed_millis_by_player
                .entry(player_id.clone())
                .or_default();
            if matches!(player.life_state, PlayerLifeState::Alive) {
                *elapsed_millis = elapsed_millis.saturating_add(u32::from(delta_millis));
                elapsed_seconds_by_player.insert(player_id.clone(), *elapsed_millis / 1_000);
                *elapsed_millis %= 1_000;
            } else {
                *elapsed_millis = 0;
            }
        }
        self.life_support_elapsed_millis_by_player
            .retain(|player_id, _| {
                self.state.player.by_id.contains_key(player_id)
                    && !self.state.player_transfer_locks.contains_key(player_id)
            });

        let max_elapsed_seconds = elapsed_seconds_by_player
            .values()
            .copied()
            .max()
            .unwrap_or_default();
        // Each elapsed life-support second is a deterministic round. Players
        // within the round are scheduled in canonical roster order so two
        // simultaneous transitions always produce the same event chain.
        for second_index in 0..max_elapsed_seconds {
            for player_id in &player_ids {
                if elapsed_seconds_by_player
                    .get(player_id)
                    .copied()
                    .unwrap_or_default()
                    <= second_index
                    || !matches!(
                        self.state
                            .player
                            .get(player_id)
                            .expect("scheduled player remains in canonical roster")
                            .life_state,
                        PlayerLifeState::Alive
                    )
                {
                    continue;
                }
                let Some(payload) = self
                    .state
                    .life_support_payload_after_one_second_for(player_id)?
                else {
                    continue;
                };
                self.commit_system_event(payload)?;
                outcome.record(AdvanceImpact::Structural);
            }
        }

        if self.advance_due_production()?.committed_quanta > 0 {
            outcome.record(AdvanceImpact::Structural);
        }
        Ok(outcome)
    }

    fn commit_system_event(&mut self, payload: EventPayload) -> Result<(), RuntimeError> {
        let mut event = self.state.prepare_system_event(payload);
        let occurred_at_unix_ms = match self.store.accepted_trusted_time() {
            Ok(now) => now,
            Err(source) => {
                self.halted = true;
                return Err(source.into());
            }
        };
        event.retime_and_rehash(occurred_at_unix_ms);
        self.commit_prepared_system_event(&event)
    }

    fn commit_production_quantum(&mut self, payload: EventPayload) -> Result<(), RuntimeError> {
        let event = self.state.prepare_production_quantum_event(payload)?;
        self.commit_prepared_system_event(&event)
    }

    fn commit_prepared_system_event(&mut self, event: &CanonicalEvent) -> Result<(), RuntimeError> {
        let prior_production_runnable = self.state.background_production_is_runnable()?;
        let mut next_state = self.state.clone();
        next_state.apply_event(event)?;
        let resulting_next_occurrence = self.resulting_next_production_occurrence(
            prior_production_runnable,
            &next_state,
            event,
        )?;
        if event_changes_physics_scene(&event.payload) {
            let lifecycle_mode = self.store.lifecycle_mode();
            self.physics
                .as_mut()
                .ok_or(RuntimeError::LifecycleUnavailable {
                    mode: lifecycle_mode,
                })?
                .rebuild(&physics_body_specs(&next_state))?;
            #[cfg(test)]
            {
                self.physics_full_rebuilds += 1;
            }
        }
        if let Err(source) =
            self.store
                .commit_world_event(event, &next_state, resulting_next_occurrence)
        {
            self.halted = true;
            return Err(source.into());
        }
        self.state = next_state;
        if matches!(
            &event.payload,
            EventPayload::ProductionQuantumCommitted { .. }
        ) && let Err(source) = self.store.acknowledge_production_sequence(&self.state)
        {
            self.halted = true;
            return Err(source.into());
        }
        self.after_event()?;
        Ok(())
    }

    fn resulting_next_production_occurrence(
        &self,
        prior_runnable: bool,
        resulting_state: &WorldState,
        event: &CanonicalEvent,
    ) -> Result<Option<ProductionScheduleOccurrence>, RuntimeError> {
        let resulting_runnable = resulting_state.background_production_is_runnable()?;
        if !resulting_runnable {
            return Ok(None);
        }
        if let EventPayload::ProductionQuantumCommitted { occurrence, .. } = &event.payload {
            let scheduled_for_unix_ms = occurrence
                .scheduled_for_unix_ms
                .checked_add(1_000)
                .ok_or_else(|| {
                    IntentError::rejected(
                        "production_clock_exhausted",
                        "production scheduled time is exhausted",
                    )
                })?;
            return resulting_state
                .next_production_occurrence_at(scheduled_for_unix_ms)
                .map(Some)
                .map_err(Into::into);
        }
        if prior_runnable {
            return self
                .store
                .next_production_occurrence()
                .cloned()
                .map(Some)
                .ok_or_else(|| {
                    IntentError::rejected(
                        "production_schedule_missing",
                        "runnable production has no durable next occurrence",
                    )
                    .into()
                });
        }
        let scheduled_from_trusted_boundary = event
            .occurred_at_unix_ms
            .checked_add(1_000)
            .ok_or_else(|| {
                IntentError::rejected(
                    "production_clock_exhausted",
                    "production scheduled time is exhausted",
                )
            })?;
        let scheduled_after_committed_cursor = resulting_state
            .production_clock
            .last_scheduled_for_unix_ms
            .checked_add(1_000)
            .ok_or_else(|| {
                IntentError::rejected(
                    "production_clock_exhausted",
                    "production scheduled time is exhausted",
                )
            })?;
        let scheduled_for_unix_ms =
            scheduled_from_trusted_boundary.max(scheduled_after_committed_cursor);
        resulting_state
            .next_production_occurrence_at(scheduled_for_unix_ms)
            .map(Some)
            .map_err(Into::into)
    }

    pub fn persist_snapshot(&mut self) -> Result<(), RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        if let Err(source) = self.store.save_snapshot(&self.state) {
            self.halted = true;
            return Err(source.into());
        }
        self.events_since_snapshot = 0;
        Ok(())
    }

    pub fn renew_lease(&mut self) -> Result<(), RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        if let Err(source) = self.store.renew_lease() {
            self.halted = true;
            return Err(source.into());
        }
        Ok(())
    }

    /// Commits one scheduler-delivered production occurrence without stepping
    /// physics, life support, controls, damage, or replication state. Exact
    /// redelivery of the already committed frontier is an idempotent no-op.
    pub fn advance_background_production_occurrence(
        &mut self,
        occurrence: ProductionScheduleOccurrence,
    ) -> Result<bool, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        if !matches!(
            self.store.lifecycle_mode(),
            crate::persistence::LifecycleMode::Active
                | crate::persistence::LifecycleMode::Background
                | crate::persistence::LifecycleMode::Activating
        ) {
            return Err(RuntimeError::LifecycleUnavailable {
                mode: self.store.lifecycle_mode(),
            });
        }
        if self.state.production_occurrence_is_committed(&occurrence) {
            if let Err(source) = self.store.acknowledge_production_sequence(&self.state) {
                self.halted = true;
                return Err(source.into());
            }
            return Ok(false);
        }
        if self.store.next_production_occurrence() != Some(&occurrence) {
            return Err(IntentError::rejected(
                "production_occurrence_delivery_conflict",
                "scheduler delivery does not match the durable next production occurrence",
            )
            .into());
        }
        let payload = self.state.production_quantum_payload(occurrence)?;
        self.commit_production_quantum(payload)?;
        Ok(true)
    }

    pub fn advance_due_production(&mut self) -> Result<ProductionDispatchOutcome, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        let now_unix_ms = match self.store.accepted_trusted_time() {
            Ok(now) => now,
            Err(source) => {
                self.halted = true;
                return Err(source.into());
            }
        };
        self.advance_due_production_through(now_unix_ms)
    }

    fn advance_due_production_through(
        &mut self,
        cutoff_unix_ms: u64,
    ) -> Result<ProductionDispatchOutcome, RuntimeError> {
        let started_at = std::time::Instant::now();
        let mut committed_quanta = 0;
        while committed_quanta < MAX_PRODUCTION_CATCH_UP_QUANTA {
            let Some(occurrence) = self.store.next_production_occurrence().cloned() else {
                break;
            };
            if occurrence.scheduled_for_unix_ms > cutoff_unix_ms {
                break;
            }
            if self.advance_background_production_occurrence(occurrence)? {
                committed_quanta += 1;
            }
            if started_at.elapsed().as_millis() >= MAX_PRODUCTION_CATCH_UP_MILLIS {
                break;
            }
        }
        let backlog_remaining = self
            .store
            .next_production_occurrence()
            .is_some_and(|occurrence| occurrence.scheduled_for_unix_ms <= cutoff_unix_ms);
        Ok(ProductionDispatchOutcome {
            committed_quanta,
            backlog_remaining,
        })
    }

    pub fn production_wait_millis(&mut self) -> Result<Option<u64>, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        let now_unix_ms = self.store.accepted_trusted_time()?;
        Ok(self
            .store
            .next_production_occurrence()
            .map(|occurrence| occurrence.scheduled_for_unix_ms.saturating_sub(now_unix_ms)))
    }

    pub fn background_dispatch_step(
        &mut self,
    ) -> Result<crate::persistence::LifecycleMode, RuntimeError> {
        if self.store.lifecycle_mode() != crate::persistence::LifecycleMode::Background {
            return Err(RuntimeError::LifecycleUnavailable {
                mode: self.store.lifecycle_mode(),
            });
        }
        self.advance_due_production()?;
        if self.store.next_production_occurrence().is_none() {
            self.store.release_to_sleeping(&self.state)?;
            return Ok(crate::persistence::LifecycleMode::Sleeping);
        }
        Ok(crate::persistence::LifecycleMode::Background)
    }

    pub fn drain_to_background_or_sleeping(
        &mut self,
    ) -> Result<crate::persistence::LifecycleMode, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        if self.store.lifecycle_mode() != crate::persistence::LifecycleMode::Active {
            return Err(RuntimeError::LifecycleUnavailable {
                mode: self.store.lifecycle_mode(),
            });
        }
        let production_runnable = self.state.background_production_is_runnable()?;
        let desired_mode = if production_runnable {
            crate::persistence::LifecycleMode::Background
        } else {
            crate::persistence::LifecycleMode::Sleeping
        };
        self.store.transition_mode(
            desired_mode,
            crate::persistence::LifecycleMode::Draining,
            &self.state,
        )?;
        self.persist_snapshot()?;
        if production_runnable {
            self.store.transition_mode(
                crate::persistence::LifecycleMode::Background,
                crate::persistence::LifecycleMode::Background,
                &self.state,
            )?;
            self.physics = None;
            Ok(crate::persistence::LifecycleMode::Background)
        } else {
            self.store.release_to_sleeping(&self.state)?;
            self.physics = None;
            Ok(crate::persistence::LifecycleMode::Sleeping)
        }
    }

    pub fn activation_step(&mut self) -> Result<bool, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        match self.store.lifecycle_mode() {
            crate::persistence::LifecycleMode::Active => return Ok(true),
            crate::persistence::LifecycleMode::Background => self.store.transition_mode(
                crate::persistence::LifecycleMode::Active,
                crate::persistence::LifecycleMode::Activating,
                &self.state,
            )?,
            crate::persistence::LifecycleMode::Activating => {}
            mode => return Err(RuntimeError::LifecycleUnavailable { mode }),
        }
        let cutoff_unix_ms = self.store.activation_cutoff_unix_ms().ok_or_else(|| {
            RuntimeError::Persistence(PersistenceError::InvalidLifecycleControl(
                "activating lifecycle has no durable wake cut-off".into(),
            ))
        })?;
        let dispatch = self.advance_due_production_through(cutoff_unix_ms)?;
        if dispatch.backlog_remaining {
            return Ok(false);
        }
        self.persist_snapshot()?;
        self.initialize_physics()?;
        self.store.publish_active(&self.state)?;
        Ok(true)
    }

    fn after_event(&mut self) -> Result<(), RuntimeError> {
        self.events_since_snapshot += 1;
        if self.events_since_snapshot >= self.snapshot_every {
            self.persist_snapshot()?;
        }
        Ok(())
    }
}

impl WorldState {
    fn production_occurrence_is_committed(
        &self,
        occurrence: &ProductionScheduleOccurrence,
    ) -> bool {
        occurrence.schema_version == PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
            && occurrence.universe_id == self.universe_id
            && occurrence.cell_id == self.cell_id
            && occurrence.lifecycle_generation == self.production_clock.lifecycle_generation
            && occurrence.production_quantum_sequence
                == self.production_clock.last_committed_quantum_sequence
            && occurrence.scheduled_for_unix_ms == self.production_clock.last_scheduled_for_unix_ms
            && occurrence.universe_manifest_hash == self.universe_manifest_hash
            && occurrence.celestial_registry_hash == self.celestial_registry_hash
    }

    pub fn background_production_is_runnable(&self) -> Result<bool, IntentError> {
        if self.production_queues.len() > MAX_BACKGROUND_QUEUE_BEARING_MACHINES {
            return Err(IntentError::rejected(
                "background_machine_budget_exceeded",
                format!(
                    "background production supports at most {MAX_BACKGROUND_QUEUE_BEARING_MACHINES} queue-bearing machines"
                ),
            ));
        }
        let mut scheduled = self
            .production_queues
            .keys()
            .map(|machine_block_id| {
                self.block_grid(machine_block_id)
                    .map(|(grid, _)| (grid.grid_id.as_str(), machine_block_id.as_str()))
                    .ok_or_else(|| {
                        IntentError::rejected(
                            "production_machine_missing",
                            "a queued production machine is missing from canonical state",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        scheduled.sort_unstable();
        for (_, machine_block_id) in scheduled {
            if self
                .production_machine_outcome_after_one_second(machine_block_id)?
                .changes_state()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn next_active_production_occurrence(
        &self,
    ) -> Result<ProductionScheduleOccurrence, IntentError> {
        let production_quantum_sequence = self
            .production_clock
            .last_committed_quantum_sequence
            .checked_add(1)
            .ok_or_else(|| {
                IntentError::rejected(
                    "production_occurrence_exhausted",
                    "production occurrence sequence is exhausted",
                )
            })?;
        let scheduled_for_unix_ms = if self.production_clock.last_scheduled_for_unix_ms == 0 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| {
                    IntentError::rejected(
                        "production_clock_invalid",
                        "system time is before the Unix epoch",
                    )
                })?
                .as_millis()
                .try_into()
                .map_err(|_| {
                    IntentError::rejected(
                        "production_clock_invalid",
                        "system time cannot be represented by the production clock",
                    )
                })?
        } else {
            self.production_clock
                .last_scheduled_for_unix_ms
                .checked_add(1_000)
                .ok_or_else(|| {
                    IntentError::rejected(
                        "production_clock_exhausted",
                        "production scheduled time is exhausted",
                    )
                })?
        };
        Ok(ProductionScheduleOccurrence {
            schema_version: PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            universe_id: self.universe_id.clone(),
            cell_id: self.cell_id.clone(),
            lifecycle_generation: self.production_clock.lifecycle_generation,
            production_quantum_sequence,
            scheduled_for_unix_ms,
            universe_manifest_hash: self.universe_manifest_hash.clone(),
            celestial_registry_hash: self.celestial_registry_hash.clone(),
        })
    }

    pub(crate) fn next_production_occurrence_at(
        &self,
        scheduled_for_unix_ms: u64,
    ) -> Result<ProductionScheduleOccurrence, IntentError> {
        let mut occurrence = self.next_active_production_occurrence()?;
        occurrence.scheduled_for_unix_ms = scheduled_for_unix_ms;
        Ok(occurrence)
    }

    fn production_quantum_payload(
        &self,
        occurrence: ProductionScheduleOccurrence,
    ) -> Result<EventPayload, IntentError> {
        self.validate_next_production_occurrence(&occurrence)?;
        let mut scheduled = self
            .production_queues
            .keys()
            .map(|machine_block_id| {
                self.block_grid(machine_block_id)
                    .map(|(grid, _)| (grid.grid_id.clone(), machine_block_id.clone()))
                    .ok_or_else(|| {
                        IntentError::rejected(
                            "production_machine_missing",
                            "a queued production machine is missing from canonical state",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        scheduled.sort();

        let mut planning_state = self.clone();
        let mut outcomes = Vec::with_capacity(scheduled.len());
        for (_, machine_block_id) in scheduled {
            let outcome =
                planning_state.production_machine_outcome_after_one_second(&machine_block_id)?;
            if outcome.changes_state() {
                planning_state.apply_production_machine_outcome(&outcome)?;
            }
            outcomes.push(outcome);
        }
        Ok(EventPayload::ProductionQuantumCommitted {
            occurrence,
            elapsed_ticks: u64::from(content::manifest().physics.fixed_step_hz),
            outcomes,
        })
    }

    fn validate_next_production_occurrence(
        &self,
        occurrence: &ProductionScheduleOccurrence,
    ) -> Result<(), IntentError> {
        let expected_sequence = self
            .production_clock
            .last_committed_quantum_sequence
            .checked_add(1)
            .ok_or_else(|| {
                IntentError::rejected(
                    "production_occurrence_exhausted",
                    "production occurrence sequence is exhausted",
                )
            })?;
        let time_is_valid = if self.production_clock.last_committed_quantum_sequence == 0 {
            occurrence.scheduled_for_unix_ms > 0
        } else {
            self.production_clock
                .last_scheduled_for_unix_ms
                .checked_add(1_000)
                .is_some_and(|earliest| occurrence.scheduled_for_unix_ms >= earliest)
        };
        if occurrence.schema_version != PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
            || occurrence.universe_id != self.universe_id
            || occurrence.cell_id != self.cell_id
            || occurrence.lifecycle_generation != self.production_clock.lifecycle_generation
            || occurrence.production_quantum_sequence != expected_sequence
            || occurrence.universe_manifest_hash != self.universe_manifest_hash
            || occurrence.celestial_registry_hash != self.celestial_registry_hash
            || !time_is_valid
        {
            return Err(IntentError::rejected(
                "production_occurrence_invalid",
                "production occurrence is not the exact next trusted cell quantum",
            ));
        }
        Ok(())
    }

    fn production_machine_outcome_after_one_second(
        &self,
        machine_block_id: &str,
    ) -> Result<ProductionMachineOutcome, IntentError> {
        let Some(job) = self
            .production_queues
            .get(machine_block_id)
            .and_then(|queue| queue.front())
        else {
            return Err(IntentError::rejected(
                "production_job_missing",
                "a scheduled production machine has no queue head",
            ));
        };
        let Some((grid, machine)) = self.block_grid(machine_block_id) else {
            return Err(IntentError::rejected(
                "production_machine_missing",
                "a queued production machine is missing from canonical state",
            ));
        };
        let base = |kind, new_progress_ticks, outputs| ProductionMachineOutcome {
            grid_id: grid.grid_id.clone(),
            machine_block_id: machine_block_id.to_owned(),
            job_id: job.job_id.clone(),
            kind,
            previous_progress_ticks: job.progress_ticks,
            new_progress_ticks,
            destination_inventory_id: job.destination_inventory_id.clone(),
            outputs,
        };
        if !machine.is_complete() || !content::machine_supports_recipe(machine.kind, job.recipe) {
            return Ok(base(
                ProductionMachineOutcomeKind::PausedMachine,
                job.progress_ticks,
                InventoryContents::default(),
            ));
        }
        if !grid.power().online {
            return Ok(base(
                ProductionMachineOutcomeKind::PausedPower,
                job.progress_ticks,
                InventoryContents::default(),
            ));
        }
        if !self.production_route_exists(machine_block_id, &job.source_inventory_id)
            || !self.production_route_exists(machine_block_id, &job.destination_inventory_id)
        {
            return Ok(base(
                ProductionMachineOutcomeKind::PausedRoute,
                job.progress_ticks,
                InventoryContents::default(),
            ));
        }

        if job.progress_ticks == job.duration_ticks {
            let destination = self.inventory(&job.destination_inventory_id)?;
            if !inventory_can_add_contents(destination, &job.pending_outputs) {
                return Ok(base(
                    ProductionMachineOutcomeKind::OutputBlocked,
                    job.progress_ticks,
                    job.pending_outputs.clone(),
                ));
            }
            return Ok(base(
                ProductionMachineOutcomeKind::OutputDelivered,
                job.progress_ticks,
                job.pending_outputs.clone(),
            ));
        }

        let elapsed_ticks = u64::from(content::manifest().physics.fixed_step_hz);
        let new_progress_ticks = job
            .progress_ticks
            .saturating_add(elapsed_ticks)
            .min(job.duration_ticks);
        if new_progress_ticks != job.duration_ticks {
            return Ok(base(
                ProductionMachineOutcomeKind::Advanced,
                new_progress_ticks,
                InventoryContents::default(),
            ));
        }
        let (_, outputs, _) =
            production_recipe_quantities(job.recipe, job.batches).ok_or_else(|| {
                IntentError::rejected(
                    "production_job_invalid",
                    "queued production quantities no longer match registered content",
                )
            })?;
        let kind =
            if inventory_can_add_contents(self.inventory(&job.destination_inventory_id)?, &outputs)
            {
                ProductionMachineOutcomeKind::CompletedAndDelivered
            } else {
                ProductionMachineOutcomeKind::Completed
            };
        Ok(base(kind, new_progress_ticks, outputs))
    }

    fn apply_production_machine_outcome(
        &mut self,
        outcome: &ProductionMachineOutcome,
    ) -> Result<(), IntentError> {
        match outcome.kind {
            ProductionMachineOutcomeKind::PausedPower
            | ProductionMachineOutcomeKind::PausedRoute
            | ProductionMachineOutcomeKind::PausedMachine
            | ProductionMachineOutcomeKind::OutputBlocked => Ok(()),
            ProductionMachineOutcomeKind::Advanced => {
                self.production_queues
                    .get_mut(&outcome.machine_block_id)
                    .and_then(|queue| queue.front_mut())
                    .expect("validated production queue head exists")
                    .progress_ticks = outcome.new_progress_ticks;
                Ok(())
            }
            ProductionMachineOutcomeKind::Completed
            | ProductionMachineOutcomeKind::CompletedAndDelivered => {
                let job = self.production_queues[&outcome.machine_block_id]
                    .front()
                    .expect("validated production completion has a queue head")
                    .clone();
                match job.recipe {
                    ProductionRecipeKind::Refining => {
                        self.ledger.refine_batches = self
                            .ledger
                            .refine_batches
                            .checked_add(job.batches)
                            .ok_or_else(|| {
                                IntentError::rejected(
                                    "replay_production_ledger_invalid",
                                    "refining completion overflows the canonical ledger",
                                )
                            })?;
                    }
                    ProductionRecipeKind::Component => {
                        self.ledger.crafted_components = self
                            .ledger
                            .crafted_components
                            .checked_add(job.batches)
                            .ok_or_else(|| {
                                IntentError::rejected(
                                    "replay_production_ledger_invalid",
                                    "component completion overflows the canonical ledger",
                                )
                            })?;
                    }
                }
                let (experience_reward, refining_batches, components_crafted) = match job.recipe {
                    ProductionRecipeKind::Refining => (
                        job.batches
                            .saturating_mul(content::manifest().experience_rewards.refining_batch),
                        job.batches,
                        0,
                    ),
                    ProductionRecipeKind::Component => (
                        job.batches.saturating_mul(
                            content::manifest().experience_rewards.crafted_component,
                        ),
                        0,
                        job.batches,
                    ),
                };
                let owner = self.player.get_mut(&job.owner_player_id).ok_or_else(|| {
                    IntentError::rejected(
                        "replay_production_owner_missing",
                        "production completion owner is not present in the canonical roster",
                    )
                })?;
                owner.experience = owner.experience.saturating_add(experience_reward);
                owner.career.refining_batches = owner
                    .career
                    .refining_batches
                    .saturating_add(refining_batches);
                owner.career.components_crafted = owner
                    .career
                    .components_crafted
                    .saturating_add(components_crafted);

                if matches!(
                    outcome.kind,
                    ProductionMachineOutcomeKind::CompletedAndDelivered
                ) {
                    add_contents(
                        &mut self
                            .inventory_mut(&outcome.destination_inventory_id)?
                            .contents,
                        &outcome.outputs,
                    )?;
                    self.pop_production_queue_head(&outcome.machine_block_id);
                } else {
                    let head = self
                        .production_queues
                        .get_mut(&outcome.machine_block_id)
                        .and_then(|queue| queue.front_mut())
                        .expect("validated production queue head exists");
                    head.progress_ticks = outcome.new_progress_ticks;
                    head.reserved_inputs = InventoryContents::default();
                    head.pending_outputs.clone_from(&outcome.outputs);
                }
                Ok(())
            }
            ProductionMachineOutcomeKind::OutputDelivered => {
                add_contents(
                    &mut self
                        .inventory_mut(&outcome.destination_inventory_id)?
                        .contents,
                    &outcome.outputs,
                )?;
                self.pop_production_queue_head(&outcome.machine_block_id);
                Ok(())
            }
        }
    }

    fn pop_production_queue_head(&mut self, machine_block_id: &str) {
        let queue = self
            .production_queues
            .get_mut(machine_block_id)
            .expect("validated production queue exists");
        queue.pop_front();
        if queue.is_empty() {
            self.production_queues.remove(machine_block_id);
        }
    }

    fn next_suit_oxygen_after_one_second_for(&self, player_id: &str) -> Result<u16, IntentError> {
        let player = self.player.get(player_id).ok_or_else(|| {
            IntentError::rejected(
                "lifecycle_player_missing",
                "life support target is not present in the canonical roster",
            )
        })?;
        if !matches!(player.life_state, PlayerLifeState::Alive) || player.suit_oxygen_milli == 0 {
            return Err(IntentError::rejected(
                "player_not_alive",
                "only an alive player with remaining oxygen has a life-support transition",
            ));
        }
        let environment = self.environment_at(player.position);
        let survival = &content::manifest().survival;
        let per_second_delta = if !player.helmet_closed && environment.breathable {
            survival.open_breathable_delta_milli_per_second
        } else if !player.helmet_closed {
            survival.open_vacuum_delta_milli_per_second
        } else if environment.breathable {
            survival.sealed_breathable_delta_milli_per_second
        } else {
            survival.sealed_vacuum_delta_milli_per_second
        };
        Ok(u16::try_from(
            (i32::from(player.suit_oxygen_milli) + i32::from(per_second_delta))
                .clamp(0, i32::from(survival.suit_oxygen_capacity_milli)),
        )
        .expect("clamped suit oxygen always fits u16"))
    }

    fn life_support_payload_after_one_second_for(
        &self,
        player_id: &str,
    ) -> Result<Option<EventPayload>, IntentError> {
        let player = self.player.get(player_id).ok_or_else(|| {
            IntentError::rejected(
                "lifecycle_player_missing",
                "life support target is not present in the canonical roster",
            )
        })?;
        let previous_oxygen_milli = player.suit_oxygen_milli;
        let new_oxygen_milli = self.next_suit_oxygen_after_one_second_for(player_id)?;
        if new_oxygen_milli == previous_oxygen_milli {
            return Ok(None);
        }
        if new_oxygen_milli == 0 {
            return self.oxygen_incapacitation_payload_for(player_id).map(Some);
        }
        Ok(Some(EventPayload::SuitOxygenChanged {
            player_id: player_id.to_owned(),
            previous_oxygen_milli,
            new_oxygen_milli,
        }))
    }

    #[cfg(test)]
    fn oxygen_incapacitation_payload(&self) -> Result<EventPayload, IntentError> {
        self.oxygen_incapacitation_payload_for(&self.player.player_id)
    }

    fn oxygen_incapacitation_payload_for(
        &self,
        player_id: &str,
    ) -> Result<EventPayload, IntentError> {
        let player = self.player.get(player_id).ok_or_else(|| {
            IntentError::rejected(
                "lifecycle_player_missing",
                "life support target is not present in the canonical roster",
            )
        })?;
        if !matches!(player.life_state, PlayerLifeState::Alive) || player.suit_oxygen_milli == 0 {
            return Err(IntentError::rejected(
                "player_not_alive",
                "only an alive player with remaining oxygen can become incapacitated",
            ));
        }
        if self.next_suit_oxygen_after_one_second_for(player_id)? != 0 {
            return Err(IntentError::rejected(
                "oxygen_not_depleted",
                "the authoritative one-second life-support transition does not reach zero",
            ));
        }
        let event_sequence = self.event_sequence + 1;
        let death_id = format!("death-{}-{event_sequence}", player.player_id);
        let inventory = self.inventory(&player.inventory_id)?;
        if inventory.domain
            != (InventoryDomain::Player {
                player_id: player.player_id.clone(),
            })
        {
            return Err(IntentError::rejected(
                "player_inventory_domain_invalid",
                "the player inventory does not belong to the authoritative player",
            ));
        }
        let has_carried_inventory = inventory.contents != InventoryContents::default();
        let (dropped_inventory, death_drop) = if has_carried_inventory {
            let drop_id = format!("drop-{}-{event_sequence}", player.player_id);
            let inventory_id = format!("inventory-{drop_id}");
            if self.death_drops.contains_key(&drop_id)
                || self.inventories.contains_key(&inventory_id)
            {
                return Err(IntentError::rejected(
                    "death_drop_identity_conflict",
                    "the deterministic death-drop identity is already in use",
                ));
            }
            (
                Some(InventoryRecord {
                    inventory_id: inventory_id.clone(),
                    domain: InventoryDomain::Dropped {
                        reason: "player_oxygen_depleted".into(),
                        owner_player_id: player.player_id.clone(),
                    },
                    contents: inventory.contents.clone(),
                    capacity_liters: inventory.capacity_liters,
                }),
                Some(DeathDrop {
                    drop_id,
                    death_id: death_id.clone(),
                    inventory_id,
                    owner_player_id: player.player_id.clone(),
                    address: player.address.clone(),
                    position: player.position,
                    created_event_sequence: event_sequence,
                    cause: PlayerDeathCause::OxygenDepleted,
                }),
            )
        } else {
            (None, None)
        };
        Ok(EventPayload::PlayerIncapacitated {
            player_id: player.player_id.clone(),
            death_id,
            cause: PlayerDeathCause::OxygenDepleted,
            address: player.address.clone(),
            position: player.position,
            previous_oxygen_milli: player.suit_oxygen_milli,
            dropped_inventory,
            death_drop,
        })
    }

    #[cfg(test)]
    fn player_respawn_payload(&self) -> Result<EventPayload, IntentError> {
        self.player_respawn_payload_for(&self.player.player_id)
    }

    fn player_respawn_payload_for(&self, player_id: &str) -> Result<EventPayload, IntentError> {
        let player = self.player.get(player_id).ok_or_else(|| {
            IntentError::rejected(
                "lifecycle_player_missing",
                "respawn target is not present in the canonical roster",
            )
        })?;
        let PlayerLifeState::Incapacitated { death_id, .. } = &player.life_state else {
            return Err(IntentError::rejected(
                "player_already_alive",
                "the player is already alive",
            ));
        };
        let survival = &content::manifest().survival;
        if self.inventory(&player.inventory_id)?.contents != InventoryContents::default() {
            return Err(IntentError::rejected(
                "respawn_inventory_not_empty",
                "recovery requires the carried inventory to remain in its death drop",
            ));
        }
        let position = (0..=2_048)
            .map(|step| {
                survival.proof_recovery_position + Vec3::new(0.0, f64::from(step) * 2.0, 0.0)
            })
            .find(|position| self.proof_recovery_position_is_clear(*position))
            .ok_or_else(|| {
                IntentError::rejected(
                    "proof_recovery_region_exhausted",
                    "the deterministic proof recovery corridor has no clear point",
                )
            })?;
        Ok(EventPayload::PlayerRespawned {
            death_id: death_id.clone(),
            address: self
                .address_for_active_position(position)
                .map_err(|message| IntentError::rejected("respawn_address_invalid", message))?,
            position,
            suit_oxygen_milli: survival.respawn_oxygen_milli,
            helmet_closed: survival.respawn_helmet_closed,
            jetpack_enabled: survival.respawn_jetpack_enabled,
            magnetic_boots_enabled: false,
        })
    }

    fn proof_recovery_position_is_clear(&self, position: Vec3) -> bool {
        let orientation = Quat::IDENTITY;
        let character = &content::manifest().character;
        let planet_axis_distance = point_capsule_axis_distance(
            planet_center(),
            position,
            orientation,
            character_capsule_half_height(),
        );
        planet_axis_distance >= planet_surface_radius_m() + character.collision_radius_m + 0.001
            && !self.player_movement_hits_voxel(position, position, orientation)
            && !self.player_movement_hits_grid(position, position, orientation)
    }

    pub fn client_intent_fingerprint(
        &self,
        actor_player_id: &str,
        message: &ClientMessage,
    ) -> Result<String, IntentError> {
        if message.operation_sequence().is_none() || message.operation_id().is_none() {
            return Err(IntentError::rejected(
                "not_a_mutating_intent",
                "only mutating client messages have intent fingerprints",
            ));
        }
        if !client_message_floats_are_finite(message) {
            return Err(IntentError::rejected(
                "invalid_vector",
                "client intent floating-point fields must be finite",
            ));
        }
        let mut canonical_message = serde_json::to_value(message).map_err(|_| {
            IntentError::rejected(
                "intent_fingerprint_invalid",
                "client intent cannot be represented by the canonical fingerprint schema",
            )
        })?;
        normalize_json_signed_zero(&mut canonical_message);
        let material = IntentFingerprintMaterial {
            domain: "the-verse-client-intent-v2",
            protocol_version: PROTOCOL_VERSION,
            fingerprint_schema_version: INTENT_FINGERPRINT_SCHEMA_VERSION,
            world_schema_version: WORLD_SCHEMA_VERSION,
            event_schema_version: EVENT_SCHEMA_VERSION,
            universe_id: &self.universe_id,
            actor_player_id,
            message: &canonical_message,
        };
        let bytes = serde_json::to_vec(&material).map_err(|_| {
            IntentError::rejected(
                "intent_fingerprint_invalid",
                "client intent cannot be represented by the canonical fingerprint schema",
            )
        })?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }

    fn validate_operation_attempt(
        &self,
        actor_player_id: &str,
        operation_sequence: u64,
        intent_fingerprint: &str,
    ) -> Result<Option<IntentReceipt>, IntentError> {
        if operation_sequence == 0 {
            return Err(IntentError::rejected(
                "operation_sequence_invalid",
                "operation sequence must be positive",
            ));
        }
        let Some(history) = self.processed_operations.get(actor_player_id) else {
            if operation_sequence == 1 {
                return Ok(None);
            }
            return Err(IntentError::rejected(
                "operation_sequence_gap",
                "operation sequence must begin at one for this actor",
            ));
        };
        if operation_sequence <= history.compacted_through {
            return Err(IntentError::rejected(
                "operation_already_committed",
                "operation committed but its historical receipt has been compacted",
            ));
        }
        if let Some(record) = history.retained.get(&operation_sequence) {
            if record.intent_fingerprint == intent_fingerprint {
                return Ok(Some(record.receipt.clone()));
            }
            return Err(IntentError::rejected(
                "operation_conflict",
                "operation sequence is already bound to a different client intent",
            ));
        }
        if operation_sequence <= history.committed_through {
            return Err(IntentError::rejected(
                "operation_history_invalid",
                "canonical operation history is missing a committed retained record",
            ));
        }
        let expected = history.committed_through.checked_add(1).ok_or_else(|| {
            IntentError::rejected(
                "operation_sequence_exhausted",
                "the actor operation sequence has reached its maximum value",
            )
        })?;
        if operation_sequence != expected {
            return Err(IntentError::rejected(
                "operation_sequence_gap",
                format!("operation sequence must be the next committed value {expected}"),
            ));
        }
        Ok(None)
    }

    fn client_message_for_human_event(
        event: &CanonicalEvent,
    ) -> Result<ClientMessage, IntentError> {
        let operation_sequence = event
            .operation_sequence
            .expect("validated human event has an operation sequence");
        let operation_id = event
            .operation_id
            .as_ref()
            .expect("validated human event has an operation ID")
            .clone();
        let message = match &event.payload {
            EventPayload::PlayerControlSet {
                movement_epoch,
                input_sequence,
                linear_input,
                angular_input,
                boost,
                dampeners,
                jump,
                ..
            } => ClientMessage::SetPlayerControl {
                operation_sequence,
                operation_id,
                movement_epoch: *movement_epoch,
                input_sequence: *input_sequence,
                linear_input: *linear_input,
                angular_input: *angular_input,
                boost: *boost,
                dampeners: *dampeners,
                jump: *jump,
            },
            EventPayload::SuitModeChanged {
                helmet_closed,
                jetpack_enabled,
                magnetic_boots_enabled,
            } => ClientMessage::SetSuitMode {
                operation_sequence,
                operation_id,
                helmet_closed: *helmet_closed,
                jetpack_enabled: *jetpack_enabled,
                magnetic_boots_enabled: *magnetic_boots_enabled,
            },
            EventPayload::PlayerRespawned { .. } => ClientMessage::RespawnPlayer {
                operation_sequence,
                operation_id,
            },
            EventPayload::VoxelMined { coordinate, .. } => ClientMessage::MineVoxel {
                operation_sequence,
                operation_id,
                coordinate: *coordinate,
            },
            EventPayload::OreRefined {
                inventory_id,
                batches,
            } => ClientMessage::RefineOre {
                operation_sequence,
                operation_id,
                inventory_id: inventory_id.clone(),
                batches: *batches,
            },
            EventPayload::ComponentCrafted {
                inventory_id,
                quantity,
            } => ClientMessage::CraftComponent {
                operation_sequence,
                operation_id,
                inventory_id: inventory_id.clone(),
                quantity: *quantity,
            },
            EventPayload::ProductionQueued { job } => ClientMessage::QueueProduction {
                operation_sequence,
                operation_id,
                machine_block_id: job.machine_block_id.clone(),
                recipe: job.recipe,
                batches: job.batches,
                source_inventory_id: job.source_inventory_id.clone(),
                destination_inventory_id: job.destination_inventory_id.clone(),
            },
            EventPayload::InventoryTransferred {
                source_inventory_id,
                destination_inventory_id,
                resource,
                quantity,
            } => ClientMessage::TransferInventory {
                operation_sequence,
                operation_id,
                source_inventory_id: source_inventory_id.clone(),
                destination_inventory_id: destination_inventory_id.clone(),
                resource: *resource,
                quantity: *quantity,
            },
            EventPayload::BlockBuilt { grid_id, block, .. } => ClientMessage::BuildBlock {
                operation_sequence,
                operation_id,
                grid_id: grid_id.clone(),
                coordinate: block.coordinate,
                kind: block.kind,
                orientation: block.orientation,
            },
            EventPayload::BlockWelded {
                grid_id, block_id, ..
            } => ClientMessage::WeldBlock {
                operation_sequence,
                operation_id,
                grid_id: grid_id.clone(),
                block_id: block_id.clone(),
            },
            EventPayload::GridControlSet {
                grid_id,
                linear_input,
                angular_input,
                dampeners,
            } => ClientMessage::SetGridControl {
                operation_sequence,
                operation_id,
                grid_id: grid_id.clone(),
                linear_input: *linear_input,
                angular_input: *angular_input,
                dampeners: *dampeners,
            },
            EventPayload::GridAnchorSet { grid_id, .. } => ClientMessage::ToggleGridAnchor {
                operation_sequence,
                operation_id,
                grid_id: grid_id.clone(),
            },
            EventPayload::BlockDamaged {
                grid_id, block_id, ..
            } => ClientMessage::DamageBlock {
                operation_sequence,
                operation_id,
                grid_id: grid_id.clone(),
                block_id: block_id.clone(),
            },
            EventPayload::SuitOxygenChanged { .. } | EventPayload::PlayerIncapacitated { .. } => {
                return Err(IntentError::rejected(
                    "replay_lifecycle_envelope_invalid",
                    "automatic life-support payload cannot use a human client envelope",
                ));
            }
            EventPayload::PhysicsStepCommitted { .. } => {
                return Err(IntentError::rejected(
                    "replay_physics_envelope_invalid",
                    "physics payload cannot use a human client envelope",
                ));
            }
            EventPayload::ProductionQuantumCommitted { .. } => {
                return Err(IntentError::rejected(
                    "replay_production_envelope_invalid",
                    "automatic production payload cannot use a human client envelope",
                ));
            }
            EventPayload::PlayerTransferPrepared { .. }
            | EventPayload::PlayerTransferQuarantined { .. }
            | EventPayload::PlayerTransferAborted { .. }
            | EventPayload::PlayerTransferExported { .. }
            | EventPayload::PlayerTransferImported { .. } => {
                return Err(IntentError::rejected(
                    "replay_transfer_envelope_invalid",
                    "cross-cell transfer payload cannot use a human client envelope",
                ));
            }
        };
        Ok(message)
    }

    pub fn prepare_client_event(
        &self,
        message: &ClientMessage,
    ) -> Result<CanonicalEvent, IntentError> {
        self.prepare_client_event_as(&self.player.player_id, message)
    }

    #[cfg(test)]
    fn prepare_next_client_event_for_fixture(
        &self,
        message: &ClientMessage,
    ) -> Result<CanonicalEvent, IntentError> {
        self.prepare_next_client_event_as_for_fixture(&self.player.player_id, message)
    }

    #[cfg(test)]
    fn prepare_next_client_event_as_for_fixture(
        &self,
        actor_player_id: &str,
        message: &ClientMessage,
    ) -> Result<CanonicalEvent, IntentError> {
        let mut sequenced = message.clone();
        if sequenced.operation_sequence() == Some(0) {
            let sequence = self
                .last_operation_sequence(actor_player_id)
                .checked_add(1)
                .expect("fixture operation sequence remains available");
            assert!(sequenced.set_operation_sequence(sequence));
        }
        self.prepare_client_event_as(actor_player_id, &sequenced)
    }

    pub fn prepare_client_event_as(
        &self,
        actor_player_id: &str,
        message: &ClientMessage,
    ) -> Result<CanonicalEvent, IntentError> {
        let actor = self.player.get(actor_player_id).ok_or_else(|| {
            IntentError::rejected(
                "actor_not_present",
                "the authenticated player is not present in this simulation cell",
            )
        })?;
        if self.player_transfer_locks.contains_key(actor_player_id) {
            return Err(IntentError::rejected(
                "actor_transfer_locked",
                "the player is in an authoritative cross-cell handoff",
            ));
        }
        let operation_id = message.operation_id().ok_or_else(|| {
            IntentError::rejected("not_a_mutating_intent", "message has no operation ID")
        })?;
        let operation_sequence = message.operation_sequence().ok_or_else(|| {
            IntentError::rejected("not_a_mutating_intent", "message has no operation sequence")
        })?;
        if operation_id.trim().is_empty() || operation_id.len() > 128 {
            return Err(IntentError::rejected(
                "invalid_operation_id",
                "operation ID must contain between 1 and 128 characters",
            ));
        }
        let intent_fingerprint = self.client_intent_fingerprint(actor_player_id, message)?;
        if self
            .validate_operation_attempt(actor_player_id, operation_sequence, &intent_fingerprint)?
            .is_some()
        {
            return Err(IntentError::rejected(
                "operation_already_committed",
                "the retained operation has already committed; execute through the runtime to recover its receipt",
            ));
        }
        if !matches!(actor.life_state, PlayerLifeState::Alive)
            && !matches!(message, ClientMessage::RespawnPlayer { .. })
        {
            return Err(IntentError::rejected(
                "player_incapacitated",
                "life support has failed; request recovery before performing work",
            ));
        }
        if matches!(
            message,
            ClientMessage::RefineOre { .. } | ClientMessage::CraftComponent { .. }
        ) {
            return Err(IntentError::rejected(
                "physical_machine_required",
                "protocol 15 production must be queued on a connected physical machine",
            ));
        }

        let payload = match message {
            ClientMessage::SetPlayerControl {
                movement_epoch,
                input_sequence,
                linear_input,
                angular_input,
                boost,
                dampeners,
                jump,
                ..
            } => {
                ensure_bounded_control(*linear_input, "character linear control")?;
                ensure_bounded_control(*angular_input, "character angular control")?;
                if *movement_epoch != actor.movement_epoch {
                    return Err(IntentError::rejected(
                        "movement_epoch_stale",
                        "character control does not match the current movement epoch",
                    ));
                }
                if *input_sequence <= actor.last_received_input_sequence {
                    return Err(IntentError::rejected(
                        "movement_input_out_of_order",
                        "character control sequence must advance monotonically",
                    ));
                }
                let lease_queue_limit =
                    usize::try_from(content::manifest().character.control_lease_ticks)
                        .unwrap_or(usize::MAX)
                        .min(MAX_PENDING_PLAYER_CONTROL_FRAMES);
                if actor.pending_control_frames.len() >= lease_queue_limit {
                    return Err(IntentError::rejected(
                        "movement_input_backpressure",
                        "character control queue is full; wait for an authoritative physics acknowledgement",
                    ));
                }
                EventPayload::PlayerControlSet {
                    movement_epoch: *movement_epoch,
                    input_sequence: *input_sequence,
                    linear_input: *linear_input,
                    angular_input: *angular_input,
                    boost: *boost,
                    dampeners: *dampeners,
                    jump: *jump,
                    expires_at_simulation_tick: self
                        .simulation_tick
                        .saturating_add(content::manifest().character.control_lease_ticks),
                }
            }
            ClientMessage::SetSuitMode {
                helmet_closed,
                jetpack_enabled,
                magnetic_boots_enabled,
                ..
            } => {
                if actor.helmet_closed == *helmet_closed
                    && actor.jetpack_enabled == *jetpack_enabled
                    && actor.locomotion.magnetic_boots_enabled == *magnetic_boots_enabled
                {
                    return Err(IntentError::rejected(
                        "suit_mode_no_change",
                        "helmet and jetpack already match the requested state",
                    ));
                }
                EventPayload::SuitModeChanged {
                    helmet_closed: *helmet_closed,
                    jetpack_enabled: *jetpack_enabled,
                    magnetic_boots_enabled: *magnetic_boots_enabled,
                }
            }
            ClientMessage::RespawnPlayer { .. } => {
                self.player_respawn_payload_for(actor_player_id)?
            }
            ClientMessage::MineVoxel { coordinate, .. } => {
                let material = self.voxels.material(*coordinate).ok_or_else(|| {
                    IntentError::rejected("voxel_missing", "target voxel is already empty")
                })?;
                self.ensure_voxel_tool_target(
                    actor,
                    *coordinate,
                    "voxel_not_targeted",
                    "the requested voxel is not the authenticated actor's closest visible tool target",
                )?;
                let ore_yield = content::voxel(material).ore_yield;
                if !self
                    .inventory(&actor.inventory_id)?
                    .can_add(ResourceKind::Ore, ore_yield)
                {
                    return Err(IntentError::rejected(
                        "inventory_capacity_exceeded",
                        "the suit inventory has no volume for the mined ore",
                    ));
                }
                if self.grids.values().any(|grid| {
                    grid.anchored
                        && grid.anchor_touches(&self.voxels)
                        && !grid.anchor_touches_after_removal(&self.voxels, Some(*coordinate))
                }) {
                    return Err(IntentError::rejected(
                        "voxel_supports_anchor",
                        "release the anchored grid before mining its final voxel support",
                    ));
                }
                EventPayload::VoxelMined {
                    coordinate: *coordinate,
                    material,
                    ore_yield,
                    inventory_id: actor.inventory_id.clone(),
                }
            }
            ClientMessage::RefineOre {
                inventory_id,
                batches,
                ..
            } => {
                if *batches == 0 {
                    return Err(IntentError::rejected(
                        "invalid_quantity",
                        "refining requires at least one batch",
                    ));
                }
                let ore_required = batches
                    .checked_mul(content::manifest().recipes.refining.ore_input)
                    .ok_or_else(|| {
                        IntentError::rejected("quantity_overflow", "refining quantity is too large")
                    })?;
                self.ensure_actor_owns_inventory(actor_player_id, inventory_id)?;
                self.ensure_inventory_functional(inventory_id)?;
                let inventory = self.inventory(inventory_id)?;
                if inventory.contents.ore < ore_required {
                    return Err(IntentError::rejected(
                        "insufficient_ore",
                        format!("refining requires {ore_required} ore"),
                    ));
                }
                let refined_output = batches
                    .checked_mul(content::manifest().recipes.refining.refined_output)
                    .ok_or_else(|| {
                        IntentError::rejected("quantity_overflow", "refining output is too large")
                    })?;
                let mut projected = inventory.clone();
                projected.contents.ore -= ore_required;
                projected.contents.refined_material = projected
                    .contents
                    .refined_material
                    .checked_add(refined_output)
                    .ok_or_else(|| {
                        IntentError::rejected("quantity_overflow", "refined inventory overflowed")
                    })?;
                if projected.used_liters() > projected.capacity_liters {
                    return Err(IntentError::rejected(
                        "inventory_capacity_exceeded",
                        "the inventory has no volume for the refined output",
                    ));
                }
                EventPayload::OreRefined {
                    inventory_id: inventory_id.clone(),
                    batches: *batches,
                }
            }
            ClientMessage::CraftComponent {
                inventory_id,
                quantity,
                ..
            } => {
                if *quantity == 0 {
                    return Err(IntentError::rejected(
                        "invalid_quantity",
                        "crafting requires at least one component",
                    ));
                }
                self.ensure_actor_owns_inventory(actor_player_id, inventory_id)?;
                self.ensure_inventory_functional(inventory_id)?;
                let inventory = self.inventory(inventory_id)?;
                let refined_required = quantity
                    .checked_mul(content::manifest().recipes.component_crafting.refined_input)
                    .ok_or_else(|| {
                        IntentError::rejected("quantity_overflow", "crafting quantity is too large")
                    })?;
                if inventory.contents.refined_material < refined_required {
                    return Err(IntentError::rejected(
                        "insufficient_refined_material",
                        format!("crafting requires {refined_required} refined material"),
                    ));
                }
                let component_output = quantity
                    .checked_mul(
                        content::manifest()
                            .recipes
                            .component_crafting
                            .component_output,
                    )
                    .ok_or_else(|| {
                        IntentError::rejected("quantity_overflow", "crafting output is too large")
                    })?;
                let mut projected = inventory.clone();
                projected.contents.refined_material -= refined_required;
                projected.contents.components = projected
                    .contents
                    .components
                    .checked_add(component_output)
                    .ok_or_else(|| {
                        IntentError::rejected("quantity_overflow", "component inventory overflowed")
                    })?;
                if projected.used_liters() > projected.capacity_liters {
                    return Err(IntentError::rejected(
                        "inventory_capacity_exceeded",
                        "the inventory has no volume for the fabricated component",
                    ));
                }
                EventPayload::ComponentCrafted {
                    inventory_id: inventory_id.clone(),
                    quantity: *quantity,
                }
            }
            ClientMessage::QueueProduction {
                machine_block_id,
                recipe,
                batches,
                source_inventory_id,
                destination_inventory_id,
                ..
            } => {
                let (grid, machine) = self.block_grid(machine_block_id).ok_or_else(|| {
                    IntentError::rejected(
                        "production_machine_missing",
                        "the selected production machine does not exist",
                    )
                })?;
                self.ensure_actor_owns_grid(actor_player_id, &grid.grid_id)?;
                if !machine.is_complete() {
                    return Err(IntentError::rejected(
                        "production_machine_incomplete",
                        "finish welding the selected production machine before queueing work",
                    ));
                }
                if !content::machine_supports_recipe(machine.kind, *recipe) {
                    return Err(IntentError::rejected(
                        "production_recipe_mismatch",
                        "the selected recipe is not registered for this machine",
                    ));
                }
                if self
                    .production_queues
                    .get(machine_block_id)
                    .map_or(0, VecDeque::len)
                    >= content::manifest().production.queue_limit_per_machine
                {
                    return Err(IntentError::rejected(
                        "production_queue_full",
                        "this machine already has the maximum 32 queued jobs",
                    ));
                }
                for inventory_id in [source_inventory_id, destination_inventory_id] {
                    self.ensure_actor_owns_inventory(actor_player_id, inventory_id)?;
                    if self.cargo_block_for_inventory(inventory_id).is_none() {
                        return Err(IntentError::rejected(
                            "production_cargo_required",
                            "production source and destination must be completed cargo inventories",
                        ));
                    }
                    if !self.production_route_exists(machine_block_id, inventory_id) {
                        return Err(IntentError::rejected(
                            "production_route_missing",
                            "the machine and both cargo endpoints require one completed same-grid conveyor route",
                        ));
                    }
                }
                let (reserved_inputs, _outputs, duration_ticks) =
                    production_recipe_quantities(*recipe, *batches).ok_or_else(|| {
                        IntentError::rejected(
                            "production_quantity_invalid",
                            "production batches must be positive and fit canonical bounds",
                        )
                    })?;
                let source = self.inventory(source_inventory_id)?;
                if source.contents.ore < reserved_inputs.ore
                    || source.contents.refined_material < reserved_inputs.refined_material
                    || source.contents.components < reserved_inputs.components
                {
                    return Err(IntentError::rejected(
                        "production_input_insufficient",
                        "source cargo does not contain the recipe's registered input",
                    ));
                }
                EventPayload::ProductionQueued {
                    job: ProductionJob {
                        job_id: format!("production-job-{}", self.event_sequence + 1),
                        operation_id: operation_id.to_owned(),
                        owner_player_id: actor_player_id.to_owned(),
                        machine_block_id: machine_block_id.clone(),
                        recipe: *recipe,
                        content_manifest_version: self.content_manifest_version.clone(),
                        batches: *batches,
                        source_inventory_id: source_inventory_id.clone(),
                        destination_inventory_id: destination_inventory_id.clone(),
                        progress_ticks: 0,
                        duration_ticks,
                        reserved_inputs,
                        pending_outputs: InventoryContents::default(),
                        queued_event_sequence: self.event_sequence + 1,
                    },
                }
            }
            ClientMessage::TransferInventory {
                source_inventory_id,
                destination_inventory_id,
                resource,
                quantity,
                ..
            } => {
                if source_inventory_id == destination_inventory_id {
                    return Err(IntentError::rejected(
                        "same_inventory",
                        "source and destination inventories must differ",
                    ));
                }
                if *quantity == 0 {
                    return Err(IntentError::rejected(
                        "invalid_quantity",
                        "transfer quantity must be positive",
                    ));
                }
                self.ensure_actor_owns_inventory(actor_player_id, source_inventory_id)?;
                self.ensure_actor_owns_inventory(actor_player_id, destination_inventory_id)?;
                self.ensure_inventory_functional(source_inventory_id)?;
                self.ensure_inventory_functional(destination_inventory_id)?;
                let source = self.inventory(source_inventory_id)?;
                self.inventory(destination_inventory_id)?;
                if source.contents.amount(*resource) < *quantity {
                    return Err(IntentError::rejected(
                        "insufficient_inventory",
                        "source inventory does not contain the requested quantity",
                    ));
                }
                if !self
                    .inventory(destination_inventory_id)?
                    .can_add(*resource, *quantity)
                {
                    return Err(IntentError::rejected(
                        "inventory_capacity_exceeded",
                        "destination inventory capacity would be exceeded",
                    ));
                }
                EventPayload::InventoryTransferred {
                    source_inventory_id: source_inventory_id.clone(),
                    destination_inventory_id: destination_inventory_id.clone(),
                    resource: *resource,
                    quantity: *quantity,
                }
            }
            ClientMessage::BuildBlock {
                grid_id,
                coordinate,
                kind,
                orientation,
                ..
            } => {
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                let grid = self.grid(grid_id)?;
                if *orientation > 3 {
                    return Err(IntentError::rejected(
                        "invalid_block_orientation",
                        "block orientation must be a quarter turn from 0 through 3",
                    ));
                }
                if grid.blocks.len() >= MAX_GRID_BLOCKS_P0 {
                    return Err(IntentError::rejected(
                        "p0_grid_budget_reached",
                        "this proof build limits one active grid to 2048 blocks",
                    ));
                }
                if grid.block_at(*coordinate).is_some() {
                    return Err(IntentError::rejected(
                        "block_position_occupied",
                        "another block already occupies that grid coordinate",
                    ));
                }
                if !grid.blocks.is_empty()
                    && !grid
                        .blocks
                        .values()
                        .any(|block| block.coordinate.manhattan_distance(*coordinate) == 1)
                {
                    return Err(IntentError::rejected(
                        "block_not_connected",
                        "new blocks must share a face with the target grid",
                    ));
                }
                self.ensure_build_tool_target(
                    actor,
                    grid_id,
                    *coordinate,
                    "build_face_not_targeted",
                    "the requested frame is not on the exact visible face targeted by the authenticated actor",
                )?;
                if self.player_intersects_grid_coordinate(grid, *coordinate) {
                    return Err(IntentError::rejected(
                        "block_intersects_player",
                        "a block frame cannot be created around the living player collider",
                    ));
                }
                let player_inventory = self.inventory(&actor.inventory_id)?;
                let component_cost = content::block(*kind).component_cost;
                if player_inventory.contents.components < component_cost {
                    return Err(IntentError::rejected(
                        "insufficient_components",
                        format!("building requires {component_cost} component(s)"),
                    ));
                }

                let block_id = format!("block-{}", self.event_sequence + 1);
                let mut block = Block::new(block_id.clone(), *coordinate, *kind);
                block.orientation = *orientation;
                block.health = block.max_health().div_ceil(4);
                block.construction_complete = false;
                if *kind == BlockKind::Cargo {
                    block.inventory_id = Some(format!("inventory-{block_id}"));
                }
                EventPayload::BlockBuilt {
                    grid_id: grid_id.clone(),
                    component_inventory_id: actor.inventory_id.clone(),
                    block,
                }
            }
            ClientMessage::WeldBlock {
                grid_id, block_id, ..
            } => {
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                let grid = self.grid(grid_id)?;
                let block = grid.blocks.get(block_id).ok_or_else(|| {
                    IntentError::rejected("block_missing", "weld target does not exist")
                })?;
                self.ensure_block_tool_target(
                    actor,
                    grid_id,
                    block_id,
                    "block_not_targeted",
                    "the requested weld block is not the authenticated actor's closest visible tool target",
                )?;
                let max_health = block.max_health();
                if block.health >= max_health {
                    return Err(IntentError::rejected(
                        "block_already_complete",
                        "the targeted block is already at full integrity",
                    ));
                }
                let new_health = block
                    .health
                    .saturating_add(max_health.div_ceil(4))
                    .min(max_health);
                EventPayload::BlockWelded {
                    grid_id: grid_id.clone(),
                    block_id: block_id.clone(),
                    previous_health: block.health,
                    new_health,
                    max_health,
                    completed_construction: !block.construction_complete
                        && new_health == max_health,
                }
            }
            ClientMessage::SetGridControl {
                grid_id,
                linear_input,
                angular_input,
                dampeners,
                ..
            } => {
                ensure_finite(*linear_input, "grid linear control")?;
                ensure_finite(*angular_input, "grid angular control")?;
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                let grid = self.grid(grid_id)?;
                if grid.anchored {
                    return Err(IntentError::rejected(
                        "grid_is_anchored",
                        "release the grid anchor before applying motion",
                    ));
                }
                if !grid.power().online {
                    return Err(IntentError::rejected(
                        "grid_unpowered",
                        "the control grid requires power",
                    ));
                }
                if linear_input.magnitude() > MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_EPSILON
                    || angular_input.magnitude() > MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_EPSILON
                {
                    return Err(IntentError::rejected(
                        "control_limit_exceeded",
                        "requested grid controls must be normalized",
                    ));
                }
                EventPayload::GridControlSet {
                    grid_id: grid_id.clone(),
                    linear_input: *linear_input,
                    angular_input: *angular_input,
                    dampeners: *dampeners,
                }
            }
            ClientMessage::ToggleGridAnchor { grid_id, .. } => {
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                let grid = self.grid(grid_id)?;
                let anchored = !grid.anchored;
                if anchored {
                    if !grid.power().online {
                        return Err(IntentError::rejected(
                            "grid_unpowered",
                            "anchoring requires an online power network",
                        ));
                    }
                    if !grid.anchor_touches(&self.voxels) {
                        return Err(IntentError::rejected(
                            "anchor_not_touching_voxel",
                            "an anchor block must touch asteroid voxels",
                        ));
                    }
                }
                EventPayload::GridAnchorSet {
                    grid_id: grid_id.clone(),
                    anchored,
                    reward_credited: anchored && grid.anchor_reward_eligible,
                }
            }
            ClientMessage::DamageBlock {
                grid_id, block_id, ..
            } => {
                let grid = self.grid(grid_id)?;
                let _block = grid.blocks.get(block_id).ok_or_else(|| {
                    IntentError::rejected(
                        "block_missing",
                        "target block does not exist on the grid",
                    )
                })?;
                self.ensure_block_tool_target(
                    actor,
                    grid_id,
                    block_id,
                    "block_not_targeted",
                    "the requested damage block is not the authenticated actor's closest visible tool target",
                )?;
                EventPayload::BlockDamaged {
                    grid_id: grid_id.clone(),
                    block_id: block_id.clone(),
                    damage: 35,
                }
            }
            ClientMessage::Hello { .. }
            | ClientMessage::RequestSnapshot
            | ClientMessage::AcknowledgeInterest { .. } => {
                return Err(IntentError::rejected(
                    "not_a_mutating_intent",
                    "message is handled by the network service",
                ));
            }
        };

        Ok(self.new_event(
            Some(actor_player_id),
            "human",
            Some(OperationEventMetadata {
                operation_id: operation_id.to_owned(),
                operation_sequence,
                intent_fingerprint,
            }),
            payload,
        ))
    }

    pub fn prepare_system_event(&self, payload: EventPayload) -> CanonicalEvent {
        self.new_event(None, "system", None, payload)
    }

    fn prepare_production_quantum_event(
        &self,
        payload: EventPayload,
    ) -> Result<CanonicalEvent, IntentError> {
        let EventPayload::ProductionQuantumCommitted { occurrence, .. } = &payload else {
            return Err(IntentError::rejected(
                "production_payload_invalid",
                "production commit requires a whole-cell quantum payload",
            ));
        };
        self.validate_next_production_occurrence(occurrence)?;
        let event_id = production_occurrence_event_id(occurrence);
        let occurred_at_unix_ms = occurrence.scheduled_for_unix_ms;
        let mut event = self.new_event(None, "system", None, payload);
        event.event_id = event_id;
        event.occurred_at_unix_ms = occurred_at_unix_ms;
        event.event_hash = event.calculate_hash();
        Ok(event)
    }

    fn new_event(
        &self,
        actor_player_id: Option<&str>,
        actor_type: &str,
        operation: Option<OperationEventMetadata>,
        payload: EventPayload,
    ) -> CanonicalEvent {
        let (operation_id, operation_sequence, intent_fingerprint) =
            operation.map_or((None, None, None), |operation| {
                (
                    Some(operation.operation_id),
                    Some(operation.operation_sequence),
                    Some(operation.intent_fingerprint),
                )
            });
        CanonicalEvent::new(
            self.event_sequence + 1,
            self.content_manifest_version.clone(),
            self.universe_manifest_hash.clone(),
            self.celestial_registry_hash.clone(),
            self.universe_id.clone(),
            self.cell_id.clone(),
            self.fencing_token.max(1),
            actor_player_id.map(str::to_owned),
            actor_type,
            operation_id,
            operation_sequence,
            intent_fingerprint,
            self.last_event_hash.clone(),
            payload,
        )
    }

    #[cfg(test)]
    fn new_test_human_event(
        &self,
        actor_player_id: &str,
        operation_id: impl Into<String>,
        payload: EventPayload,
    ) -> CanonicalEvent {
        let operation_sequence = self
            .last_operation_sequence(actor_player_id)
            .checked_add(1)
            .expect("test operation sequence remains available");
        self.new_test_human_event_at(actor_player_id, operation_sequence, operation_id, payload)
    }

    #[cfg(test)]
    fn new_test_human_event_at(
        &self,
        actor_player_id: &str,
        operation_sequence: u64,
        operation_id: impl Into<String>,
        payload: EventPayload,
    ) -> CanonicalEvent {
        let mut event = self.new_event(
            Some(actor_player_id),
            "human",
            Some(OperationEventMetadata {
                operation_id: operation_id.into(),
                operation_sequence,
                intent_fingerprint: "0".repeat(64),
            }),
            payload,
        );
        let message = Self::client_message_for_human_event(&event)
            .expect("test human payload reconstructs a typed client message");
        event.intent_fingerprint = Some(
            self.client_intent_fingerprint(actor_player_id, &message)
                .expect("test human intent fingerprints"),
        );
        event.event_hash = event.calculate_hash();
        event
    }

    pub fn apply_event(&mut self, event: &CanonicalEvent) -> Result<(), IntentError> {
        if event.schema_name != EVENT_SCHEMA_NAME || event.schema_version != EVENT_SCHEMA_VERSION {
            return Err(IntentError::rejected(
                "event_schema_mismatch",
                format!("event requires {EVENT_SCHEMA_NAME} schema {EVENT_SCHEMA_VERSION}"),
            ));
        }
        let expected = self.event_sequence + 1;
        if event.event_sequence != expected {
            return Err(IntentError::SequenceMismatch {
                expected,
                received: event.event_sequence,
            });
        }
        if event.previous_event_hash != self.last_event_hash {
            return Err(IntentError::PreviousHashMismatch);
        }
        if !event.hash_is_valid() {
            return Err(IntentError::InvalidEventHash);
        }
        if event.universe_id != self.universe_id || event.cell_id != self.cell_id {
            return Err(IntentError::WrongAuthority);
        }
        if event.content_manifest_version != self.content_manifest_version {
            return Err(IntentError::ContentManifestMismatch);
        }
        if event.universe_manifest_hash != self.universe_manifest_hash
            || event.celestial_registry_hash != self.celestial_registry_hash
        {
            return Err(IntentError::rejected(
                "event_universe_binding_mismatch",
                "event was produced under a different universe manifest or celestial registry",
            ));
        }
        let mut hydrated_event = event.clone();
        hydrated_event
            .payload
            .hydrate_spatial_poses(&self.cell_address)
            .map_err(|message| IntentError::rejected("event_spatial_address_invalid", message))?;
        let event = &hydrated_event;
        match event.actor_type.as_str() {
            "human"
                if event
                    .actor_player_id
                    .as_deref()
                    .is_none_or(|player_id| !self.player.by_id.contains_key(player_id))
                    || event.operation_id.as_deref().is_none_or(|operation_id| {
                        operation_id.trim().is_empty() || operation_id.len() > 128
                    })
                    || event
                        .operation_sequence
                        .is_none_or(|sequence| sequence == 0)
                    || event
                        .intent_fingerprint
                        .as_deref()
                        .is_none_or(|fingerprint| !valid_blake3_hex(fingerprint)) =>
            {
                return Err(IntentError::rejected(
                    "replay_actor_envelope_invalid",
                    "human events require one present canonical player actor, operation ID, positive operation sequence, and typed intent fingerprint",
                ));
            }
            "system"
                if event.actor_player_id.is_some()
                    || event.operation_id.is_some()
                    || event.operation_sequence.is_some()
                    || event.intent_fingerprint.is_some() =>
            {
                return Err(IntentError::rejected(
                    "replay_actor_envelope_invalid",
                    "system events cannot carry any client operation metadata",
                ));
            }
            "human" | "system" => {}
            _ => {
                return Err(IntentError::rejected(
                    "replay_actor_envelope_invalid",
                    "event actor type must be human or system",
                ));
            }
        }
        if event.actor_type == "human" {
            let actor_player_id = event
                .actor_player_id
                .as_deref()
                .expect("validated human event has an actor");
            let operation_sequence = event
                .operation_sequence
                .expect("validated human event has an operation sequence");
            let intent_fingerprint = event
                .intent_fingerprint
                .as_deref()
                .expect("validated human event has an intent fingerprint");
            if self
                .validate_operation_attempt(
                    actor_player_id,
                    operation_sequence,
                    intent_fingerprint,
                )?
                .is_some()
            {
                return Err(IntentError::rejected(
                    "replay_operation_duplicate",
                    "event operation sequence was already committed",
                ));
            }
            let reconstructed = Self::client_message_for_human_event(event)?;
            let expected_fingerprint =
                self.client_intent_fingerprint(actor_player_id, &reconstructed)?;
            if expected_fingerprint != intent_fingerprint {
                return Err(IntentError::rejected(
                    "replay_intent_fingerprint_mismatch",
                    "event intent fingerprint does not bind its typed client request",
                ));
            }
        }
        if event
            .actor_player_id
            .as_ref()
            .is_some_and(|player_id| self.player_transfer_locks.contains_key(player_id))
        {
            return Err(IntentError::rejected(
                "replay_actor_transfer_locked",
                "a transfer-locked player cannot commit gameplay mutations",
            ));
        }
        match &event.payload {
            EventPayload::SuitOxygenChanged { player_id, .. }
            | EventPayload::PlayerIncapacitated { player_id, .. }
                if self.player_transfer_locks.contains_key(player_id) =>
            {
                return Err(IntentError::rejected(
                    "replay_actor_transfer_locked",
                    "life support cannot mutate a transfer-locked player",
                ));
            }
            EventPayload::PhysicsStepCommitted { players, .. }
                if players
                    .iter()
                    .any(|player| self.player_transfer_locks.contains_key(&player.player_id)) =>
            {
                return Err(IntentError::rejected(
                    "replay_actor_transfer_locked",
                    "physics cannot mutate a transfer-locked player",
                ));
            }
            _ => {}
        }
        match &event.payload {
            EventPayload::PlayerControlSet { .. }
                if event.actor_type != "human"
                    || event.operation_id.as_deref().is_none_or(str::is_empty) =>
            {
                return Err(IntentError::rejected(
                    "replay_player_control_envelope_invalid",
                    "character control requires the authoritative player actor and an operation ID",
                ));
            }
            EventPayload::VoxelMined { .. }
                if event.actor_type != "human"
                    || event.actor_player_id.is_none()
                    || event.operation_id.as_deref().is_none_or(str::is_empty) =>
            {
                return Err(IntentError::rejected(
                    "replay_mining_envelope_invalid",
                    "voxel mining requires the authoritative player actor and an operation ID",
                ));
            }
            EventPayload::OreRefined { .. } | EventPayload::ComponentCrafted { .. } => {
                return Err(IntentError::rejected(
                    "replay_physical_machine_required",
                    "event schema 13 rejects direct inventory refining and crafting",
                ));
            }
            EventPayload::ProductionQueued { .. }
            | EventPayload::InventoryTransferred { .. }
            | EventPayload::BlockBuilt { .. }
            | EventPayload::BlockWelded { .. }
            | EventPayload::GridControlSet { .. }
            | EventPayload::GridAnchorSet { .. }
            | EventPayload::BlockDamaged { .. }
                if event.actor_type != "human"
                    || event.actor_player_id.is_none()
                    || event.operation_id.as_deref().is_none_or(str::is_empty) =>
            {
                return Err(IntentError::rejected(
                    "replay_hand_tool_envelope_invalid",
                    "industry, construction, grid control, anchoring, and hand-tool damage require an authenticated player actor and operation ID",
                ));
            }
            EventPayload::ProductionQuantumCommitted { .. }
                if event.actor_player_id.is_some()
                    || event.actor_type != "system"
                    || event.operation_id.is_some() =>
            {
                return Err(IntentError::rejected(
                    "replay_production_envelope_invalid",
                    "production quanta require the system actor and no operation ID",
                ));
            }
            EventPayload::PhysicsStepCommitted { .. }
                if event.actor_player_id.is_some()
                    || event.actor_type != "system"
                    || event.operation_id.is_some() =>
            {
                return Err(IntentError::rejected(
                    "replay_physics_envelope_invalid",
                    "physics outcomes require the system actor and no operation ID",
                ));
            }
            EventPayload::SuitModeChanged { .. }
                if event.actor_player_id.is_none()
                    || event.actor_type != "human"
                    || event.operation_id.as_deref().is_none_or(str::is_empty) =>
            {
                return Err(IntentError::rejected(
                    "replay_lifecycle_envelope_invalid",
                    "suit mode requires the authenticated player actor and an operation ID",
                ));
            }
            EventPayload::SuitOxygenChanged { player_id, .. }
            | EventPayload::PlayerIncapacitated { player_id, .. }
                if !self.player.by_id.contains_key(player_id)
                    || event.actor_player_id.is_some()
                    || event.actor_type != "system"
                    || event.operation_id.is_some() =>
            {
                return Err(IntentError::rejected(
                    "replay_lifecycle_envelope_invalid",
                    "automatic life-support events require the system actor and no operation ID",
                ));
            }
            EventPayload::PlayerRespawned { .. }
                if event.actor_player_id.is_none()
                    || event.actor_type != "human"
                    || event.operation_id.as_deref().is_none_or(str::is_empty) =>
            {
                return Err(IntentError::rejected(
                    "replay_lifecycle_envelope_invalid",
                    "respawn requires the authoritative player actor and an operation ID",
                ));
            }
            _ => {}
        }
        if let Some(actor_player_id) = event.actor_player_id.as_deref()
            && !matches!(
                self.player
                    .get(actor_player_id)
                    .expect("validated human actor is present")
                    .life_state,
                PlayerLifeState::Alive
            )
            && !matches!(&event.payload, EventPayload::PlayerRespawned { .. })
        {
            return Err(IntentError::rejected(
                "replay_player_incapacitated",
                "incapacitated players cannot commit gameplay events before recovery",
            ));
        }
        if event.authority_fencing_token == 0 || event.authority_fencing_token < self.fencing_token
        {
            return Err(IntentError::rejected(
                "event_fencing_token_invalid",
                "event fencing token must be positive and nondecreasing",
            ));
        }

        match &event.payload {
            EventPayload::PlayerControlSet {
                movement_epoch,
                input_sequence,
                linear_input,
                angular_input,
                boost,
                dampeners,
                jump,
                expires_at_simulation_tick,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated player control has a human actor");
                let actor = self
                    .player
                    .get_mut(actor_player_id)
                    .expect("validated player control actor is present");
                ensure_bounded_control(*linear_input, "replayed character linear control")?;
                ensure_bounded_control(*angular_input, "replayed character angular control")?;
                if *movement_epoch != actor.movement_epoch
                    || *input_sequence <= actor.last_received_input_sequence
                    || *expires_at_simulation_tick
                        != self
                            .simulation_tick
                            .saturating_add(content::manifest().character.control_lease_ticks)
                {
                    return Err(IntentError::rejected(
                        "replay_player_control_invalid",
                        "character control epoch, sequence, or lease is not canonical",
                    ));
                }
                let lease_queue_limit =
                    usize::try_from(content::manifest().character.control_lease_ticks)
                        .unwrap_or(usize::MAX)
                        .min(MAX_PENDING_PLAYER_CONTROL_FRAMES);
                if actor.pending_control_frames.len() >= lease_queue_limit {
                    return Err(IntentError::rejected(
                        "replay_player_control_backpressure_invalid",
                        "character control event exceeds the canonical pending-frame bound",
                    ));
                }
                actor.pending_control_frames.push_back(PlayerControlFrame {
                    input_sequence: *input_sequence,
                    linear_input: *linear_input,
                    angular_input: *angular_input,
                    boost: *boost,
                    dampeners: *dampeners,
                    jump: *jump,
                    expires_at_simulation_tick: *expires_at_simulation_tick,
                });
                actor.last_received_input_sequence = *input_sequence;
            }
            EventPayload::SuitModeChanged {
                helmet_closed,
                jetpack_enabled,
                magnetic_boots_enabled,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated suit mode has a human actor");
                let player = self
                    .player
                    .get_mut(actor_player_id)
                    .expect("validated suit-mode actor is present");
                player.helmet_closed = *helmet_closed;
                player.jetpack_enabled = *jetpack_enabled;
                player.locomotion.magnetic_boots_enabled = *magnetic_boots_enabled;
                if *jetpack_enabled {
                    player.locomotion.kind = LocomotionKind::Eva;
                    player.locomotion.support = None;
                } else if matches!(player.locomotion.kind, LocomotionKind::Eva) {
                    player.locomotion.kind = LocomotionKind::Airborne;
                }
            }
            EventPayload::SuitOxygenChanged {
                player_id,
                new_oxygen_milli,
                ..
            } => {
                let expected = self.life_support_payload_after_one_second_for(player_id)?;
                if expected.as_ref() != Some(&event.payload) {
                    return Err(IntentError::rejected(
                        "replay_suit_oxygen_invalid",
                        "life-support event is not the exact authoritative one-second outcome",
                    ));
                }
                self.player
                    .get_mut(player_id)
                    .expect("validated lifecycle target is present")
                    .suit_oxygen_milli = *new_oxygen_milli;
            }
            EventPayload::PlayerIncapacitated { player_id, .. } => {
                let expected = self.oxygen_incapacitation_payload_for(player_id)?;
                if expected != event.payload {
                    return Err(IntentError::rejected(
                        "replay_player_incapacitation_invalid",
                        "incapacitation does not match authoritative life support or inventory",
                    ));
                }
                let EventPayload::PlayerIncapacitated {
                    player_id,
                    death_id,
                    cause,
                    dropped_inventory,
                    death_drop,
                    ..
                } = expected
                else {
                    unreachable!("incapacitation preparation returns incapacitation payload");
                };
                let inventory_id = self
                    .player
                    .get(&player_id)
                    .expect("validated incapacitation target is present")
                    .inventory_id
                    .clone();
                self.inventory_mut(&inventory_id)?.contents = InventoryContents::default();
                if let (Some(inventory), Some(drop)) = (dropped_inventory, death_drop) {
                    self.inventories
                        .insert(inventory.inventory_id.clone(), inventory);
                    self.death_drops.insert(drop.drop_id.clone(), drop);
                }
                let player = self
                    .player
                    .get_mut(&player_id)
                    .expect("validated incapacitation target is present");
                player.suit_oxygen_milli = 0;
                player.jetpack_enabled = false;
                player.linear_velocity = Vec3::ZERO;
                player.angular_velocity = Vec3::ZERO;
                player.surface_contact = false;
                player.locomotion = reset_locomotion(
                    player.position,
                    LocomotionKind::Airborne,
                    false,
                    self.simulation_tick,
                );
                player.control_linear_input = Vec3::ZERO;
                player.control_angular_input = Vec3::ZERO;
                player.boost = false;
                player.dampeners = true;
                player.jump = false;
                player.control_expires_at_simulation_tick = self.simulation_tick;
                player.pending_control_frames.clear();
                player.life_state = PlayerLifeState::Incapacitated { death_id, cause };
                self.active_contact_pairs
                    .retain(|pair| !contact_key_involves_player_id(pair, &player_id));
            }
            EventPayload::PlayerRespawned {
                address,
                position,
                suit_oxygen_milli,
                helmet_closed,
                jetpack_enabled,
                magnetic_boots_enabled,
                ..
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated respawn has a human actor");
                let expected = self.player_respawn_payload_for(actor_player_id)?;
                if expected != event.payload {
                    return Err(IntentError::rejected(
                        "replay_player_respawn_invalid",
                        "respawn does not match the server-selected recovery outcome",
                    ));
                }
                let player = self
                    .player
                    .get_mut(actor_player_id)
                    .expect("validated respawn actor is present");
                player.address = address.clone();
                player.position = *position;
                player.orientation = Quat::IDENTITY;
                player.linear_velocity = Vec3::ZERO;
                player.angular_velocity = Vec3::ZERO;
                player.surface_contact = false;
                player.locomotion = reset_locomotion(
                    *position,
                    if *jetpack_enabled {
                        LocomotionKind::Eva
                    } else {
                        LocomotionKind::Airborne
                    },
                    *magnetic_boots_enabled,
                    self.simulation_tick,
                );
                player.movement_epoch = player.movement_epoch.saturating_add(1);
                player.last_received_input_sequence = 0;
                player.last_processed_input_sequence = 0;
                player.pending_control_frames.clear();
                player.control_linear_input = Vec3::ZERO;
                player.control_angular_input = Vec3::ZERO;
                player.boost = false;
                player.dampeners = true;
                player.jump = false;
                player.control_expires_at_simulation_tick = self.simulation_tick;
                player.suit_oxygen_milli = *suit_oxygen_milli;
                player.helmet_closed = *helmet_closed;
                player.jetpack_enabled = *jetpack_enabled;
                player.life_state = PlayerLifeState::Alive;
                self.active_contact_pairs
                    .retain(|pair| !contact_key_involves_player_id(pair, actor_player_id));
            }
            EventPayload::VoxelMined {
                coordinate,
                material,
                ore_yield,
                inventory_id,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated mining event has a human actor");
                let actor = self
                    .player
                    .get(actor_player_id)
                    .expect("validated mining actor is present");
                if inventory_id != &actor.inventory_id {
                    return Err(IntentError::rejected(
                        "replay_mining_actor_inventory_invalid",
                        "mined ore must be credited to the authenticated actor's carried inventory",
                    ));
                }
                let canonical_material = self.voxels.material(*coordinate).ok_or_else(|| {
                    IntentError::rejected("replay_voxel_missing", "event target voxel is missing")
                })?;
                if canonical_material != *material {
                    return Err(IntentError::rejected(
                        "replay_material_mismatch",
                        "event material does not match voxel material",
                    ));
                }
                self.ensure_voxel_tool_target(
                    actor,
                    *coordinate,
                    "replay_mining_target_invalid",
                    "mining event does not match the authenticated actor's closest visible tool target",
                )?;
                let canonical_ore_yield = content::voxel(canonical_material).ore_yield;
                if *ore_yield != canonical_ore_yield {
                    return Err(IntentError::rejected(
                        "replay_mining_yield_invalid",
                        "mining event yield does not match the canonical voxel material",
                    ));
                }
                if !self
                    .inventory(inventory_id)?
                    .can_add(ResourceKind::Ore, canonical_ore_yield)
                {
                    return Err(IntentError::rejected(
                        "replay_mining_inventory_capacity_invalid",
                        "mining event exceeds the authenticated actor's carried inventory capacity",
                    ));
                }
                if self.grids.values().any(|grid| {
                    grid.anchored
                        && grid.anchor_touches(&self.voxels)
                        && !grid.anchor_touches_after_removal(&self.voxels, Some(*coordinate))
                }) {
                    return Err(IntentError::rejected(
                        "replay_mining_anchor_support_invalid",
                        "mining event would remove the final voxel support from an anchored grid",
                    ));
                }

                self.voxels
                    .remove(*coordinate)
                    .expect("validated mining target remains present");
                self.inventory_mut(inventory_id)?.contents.ore += canonical_ore_yield;
                self.ledger.mined_ore += canonical_ore_yield;
                let body_id =
                    voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(*coordinate));
                let collider_id = voxel_collision_collider_id(*coordinate);
                self.active_contact_pairs.retain(|pair| {
                    !((pair.body_a == body_id && pair.collider_a == collider_id)
                        || (pair.body_b == body_id && pair.collider_b == collider_id))
                });
            }
            EventPayload::ProductionQueued { job } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated production enqueue has a human actor");
                let operation_id = event
                    .operation_id
                    .as_deref()
                    .expect("validated production enqueue has an operation ID");
                let (grid, machine) = self.block_grid(&job.machine_block_id).ok_or_else(|| {
                    IntentError::rejected(
                        "replay_production_machine_missing",
                        "queued production machine is not present",
                    )
                })?;
                if job.job_id != format!("production-job-{}", event.event_sequence)
                    || job.queued_event_sequence != event.event_sequence
                    || job.operation_id != operation_id
                    || job.owner_player_id != actor_player_id
                    || grid.owner_player_id != actor_player_id
                    || !machine.is_complete()
                    || !content::machine_supports_recipe(machine.kind, job.recipe)
                    || job.content_manifest_version != self.content_manifest_version
                    || job.progress_ticks != 0
                    || job.pending_outputs != InventoryContents::default()
                    || self
                        .production_queues
                        .values()
                        .flatten()
                        .any(|prior| prior.job_id == job.job_id)
                {
                    return Err(IntentError::rejected(
                        "replay_production_job_invalid",
                        "queued production identity, owner, machine, manifest, or initial state is not canonical",
                    ));
                }
                if self
                    .production_queues
                    .get(&job.machine_block_id)
                    .map_or(0, VecDeque::len)
                    >= content::manifest().production.queue_limit_per_machine
                {
                    return Err(IntentError::rejected(
                        "replay_production_queue_full",
                        "queued production exceeds the canonical machine queue bound",
                    ));
                }
                let (expected_inputs, _, expected_duration) =
                    production_recipe_quantities(job.recipe, job.batches).ok_or_else(|| {
                        IntentError::rejected(
                            "replay_production_quantity_invalid",
                            "queued production quantities overflow registered bounds",
                        )
                    })?;
                if job.reserved_inputs != expected_inputs || job.duration_ticks != expected_duration
                {
                    return Err(IntentError::rejected(
                        "replay_production_recipe_invalid",
                        "queued production escrow or duration does not match registered content",
                    ));
                }
                for inventory_id in [&job.source_inventory_id, &job.destination_inventory_id] {
                    self.ensure_actor_owns_inventory(actor_player_id, inventory_id)?;
                    if self.cargo_block_for_inventory(inventory_id).is_none()
                        || !self.production_route_exists(&job.machine_block_id, inventory_id)
                    {
                        return Err(IntentError::rejected(
                            "replay_production_route_invalid",
                            "queued production endpoints do not share a completed conveyor route",
                        ));
                    }
                }
                let mut source = self.inventory(&job.source_inventory_id)?.clone();
                subtract_contents(&mut source.contents, &job.reserved_inputs).map_err(|()| {
                    IntentError::rejected(
                        "replay_production_input_invalid",
                        "queued production exceeds its source cargo contents",
                    )
                })?;
                self.inventory_mut(&job.source_inventory_id)?.contents = source.contents;
                self.production_queues
                    .entry(job.machine_block_id.clone())
                    .or_default()
                    .push_back(job.clone());
            }
            EventPayload::ProductionQuantumCommitted {
                occurrence,
                elapsed_ticks,
                outcomes,
            } => {
                if *elapsed_ticks != u64::from(content::manifest().physics.fixed_step_hz)
                    || event.event_id != production_occurrence_event_id(occurrence)
                    || event.occurred_at_unix_ms != occurrence.scheduled_for_unix_ms
                {
                    return Err(IntentError::rejected(
                        "replay_production_quantum_envelope_invalid",
                        "production quantum identity, time, or elapsed ticks are invalid",
                    ));
                }
                let expected = self.production_quantum_payload(occurrence.clone())?;
                if expected != event.payload {
                    return Err(IntentError::rejected(
                        "replay_production_quantum_invalid",
                        "production quantum does not match the complete authoritative one-second outcome",
                    ));
                }
                for outcome in outcomes {
                    if outcome.changes_state() {
                        self.apply_production_machine_outcome(outcome)?;
                    }
                }
                self.production_clock.last_committed_quantum_sequence =
                    occurrence.production_quantum_sequence;
                self.production_clock.last_scheduled_for_unix_ms = occurrence.scheduled_for_unix_ms;
            }
            EventPayload::OreRefined {
                inventory_id,
                batches,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated refining event has a human actor");
                let recipe = &content::manifest().recipes.refining;
                if *batches == 0 {
                    return Err(IntentError::rejected(
                        "replay_refining_quantity_invalid",
                        "refining event must contain at least one batch",
                    ));
                }
                let ore_required = batches.checked_mul(recipe.ore_input).ok_or_else(|| {
                    IntentError::rejected(
                        "replay_refining_quantity_invalid",
                        "refining event quantity overflowed",
                    )
                })?;
                let refined_output =
                    batches.checked_mul(recipe.refined_output).ok_or_else(|| {
                        IntentError::rejected(
                            "replay_refining_quantity_invalid",
                            "refining event output overflowed",
                        )
                    })?;
                self.ensure_actor_owns_inventory(actor_player_id, inventory_id)?;
                self.ensure_inventory_functional(inventory_id)?;
                let mut projected = self.inventory(inventory_id)?.clone();
                if projected.contents.ore < ore_required {
                    return Err(IntentError::rejected(
                        "replay_refining_inventory_invalid",
                        "refining event exceeds the authoritative ore inventory",
                    ));
                }
                projected.contents.ore -= ore_required;
                projected.contents.refined_material = projected
                    .contents
                    .refined_material
                    .checked_add(refined_output)
                    .ok_or_else(|| {
                        IntentError::rejected(
                            "replay_refining_inventory_invalid",
                            "refining event overflows the authoritative inventory",
                        )
                    })?;
                if projected.used_liters() > projected.capacity_liters {
                    return Err(IntentError::rejected(
                        "replay_refining_inventory_invalid",
                        "refining event exceeds authoritative inventory capacity",
                    ));
                }
                let next_batches = self
                    .ledger
                    .refine_batches
                    .checked_add(*batches)
                    .ok_or_else(|| {
                        IntentError::rejected(
                            "replay_refining_ledger_invalid",
                            "refining event overflows the canonical ledger",
                        )
                    })?;
                self.inventory_mut(inventory_id)?.contents = projected.contents;
                self.ledger.refine_batches = next_batches;
            }
            EventPayload::ComponentCrafted {
                inventory_id,
                quantity,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated crafting event has a human actor");
                let recipe = &content::manifest().recipes.component_crafting;
                if *quantity == 0 {
                    return Err(IntentError::rejected(
                        "replay_crafting_quantity_invalid",
                        "crafting event must contain at least one component",
                    ));
                }
                let refined_required =
                    quantity.checked_mul(recipe.refined_input).ok_or_else(|| {
                        IntentError::rejected(
                            "replay_crafting_quantity_invalid",
                            "crafting event quantity overflowed",
                        )
                    })?;
                let component_output =
                    quantity
                        .checked_mul(recipe.component_output)
                        .ok_or_else(|| {
                            IntentError::rejected(
                                "replay_crafting_quantity_invalid",
                                "crafting event output overflowed",
                            )
                        })?;
                self.ensure_actor_owns_inventory(actor_player_id, inventory_id)?;
                self.ensure_inventory_functional(inventory_id)?;
                let mut projected = self.inventory(inventory_id)?.clone();
                if projected.contents.refined_material < refined_required {
                    return Err(IntentError::rejected(
                        "replay_crafting_inventory_invalid",
                        "crafting event exceeds the authoritative refined inventory",
                    ));
                }
                projected.contents.refined_material -= refined_required;
                projected.contents.components = projected
                    .contents
                    .components
                    .checked_add(component_output)
                    .ok_or_else(|| {
                        IntentError::rejected(
                            "replay_crafting_inventory_invalid",
                            "crafting event overflows the authoritative inventory",
                        )
                    })?;
                if projected.used_liters() > projected.capacity_liters {
                    return Err(IntentError::rejected(
                        "replay_crafting_inventory_invalid",
                        "crafting event exceeds authoritative inventory capacity",
                    ));
                }
                let next_crafted = self
                    .ledger
                    .crafted_components
                    .checked_add(*quantity)
                    .ok_or_else(|| {
                        IntentError::rejected(
                            "replay_crafting_ledger_invalid",
                            "crafting event overflows the canonical ledger",
                        )
                    })?;
                self.inventory_mut(inventory_id)?.contents = projected.contents;
                self.ledger.crafted_components = next_crafted;
            }
            EventPayload::InventoryTransferred {
                source_inventory_id,
                destination_inventory_id,
                resource,
                quantity,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated transfer event has a human actor");
                if source_inventory_id == destination_inventory_id || *quantity == 0 {
                    return Err(IntentError::rejected(
                        "replay_inventory_transfer_invalid",
                        "inventory transfer must use distinct inventories and a positive quantity",
                    ));
                }
                self.ensure_actor_owns_inventory(actor_player_id, source_inventory_id)?;
                self.ensure_actor_owns_inventory(actor_player_id, destination_inventory_id)?;
                self.ensure_inventory_functional(source_inventory_id)?;
                self.ensure_inventory_functional(destination_inventory_id)?;
                if self
                    .inventory(source_inventory_id)?
                    .contents
                    .amount(*resource)
                    < *quantity
                    || !self
                        .inventory(destination_inventory_id)?
                        .can_add(*resource, *quantity)
                {
                    return Err(IntentError::rejected(
                        "replay_inventory_transfer_invalid",
                        "inventory transfer exceeds authoritative contents or capacity",
                    ));
                }
                let mut source_contents = self.inventory(source_inventory_id)?.contents.clone();
                let mut destination_contents =
                    self.inventory(destination_inventory_id)?.contents.clone();
                mutate_resource(&mut source_contents, *resource, |amount| {
                    *amount -= quantity;
                });
                let destination_amount = destination_contents
                    .amount(*resource)
                    .checked_add(*quantity)
                    .ok_or_else(|| {
                        IntentError::rejected(
                            "replay_inventory_transfer_invalid",
                            "inventory transfer overflows the destination quantity",
                        )
                    })?;
                mutate_resource(&mut destination_contents, *resource, |amount| {
                    *amount = destination_amount;
                });
                self.inventory_mut(source_inventory_id)?.contents = source_contents;
                self.inventory_mut(destination_inventory_id)?.contents = destination_contents;
            }
            EventPayload::BlockBuilt {
                grid_id,
                component_inventory_id,
                block,
            } => {
                let definition = content::block(block.kind);
                let expected_block_id = format!("block-{}", event.event_sequence);
                let expected_inventory_id = (block.kind == BlockKind::Cargo)
                    .then(|| format!("inventory-{}", block.block_id));
                if block.orientation > 3
                    || block.health != block.max_health().div_ceil(4)
                    || block.construction_complete
                    || block.component_cost != definition.component_cost
                    || block.block_id != expected_block_id
                    || block.inventory_id != expected_inventory_id
                {
                    return Err(IntentError::rejected(
                        "replay_construction_frame_invalid",
                        "placed frame does not match canonical identity, cost, linkage, orientation, or integrity",
                    ));
                }
                if self
                    .grids
                    .values()
                    .any(|grid| grid.blocks.contains_key(&block.block_id))
                    || block
                        .inventory_id
                        .as_ref()
                        .is_some_and(|inventory_id| self.inventories.contains_key(inventory_id))
                {
                    return Err(IntentError::rejected(
                        "replay_construction_identity_duplicate",
                        "placed frame reuses an authoritative block or inventory identity",
                    ));
                }
                let grid = self.grid(grid_id)?;
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated construction event has a human actor");
                let actor = self
                    .player
                    .get(actor_player_id)
                    .expect("validated construction actor is present");
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                if component_inventory_id != &actor.inventory_id {
                    return Err(IntentError::rejected(
                        "replay_construction_inventory_invalid",
                        "construction must debit the authenticated actor's carried inventory",
                    ));
                }
                self.ensure_actor_owns_inventory(actor_player_id, component_inventory_id)?;
                if grid.blocks.len() >= MAX_GRID_BLOCKS_P0
                    || grid.block_at(block.coordinate).is_some()
                    || (!grid.blocks.is_empty()
                        && !grid.blocks.values().any(|existing| {
                            existing.coordinate.manhattan_distance(block.coordinate) == 1
                        }))
                {
                    return Err(IntentError::rejected(
                        "replay_construction_location_invalid",
                        "placed frame exceeds the grid budget or is not at a free face-connected coordinate",
                    ));
                }
                self.ensure_build_tool_target(
                    actor,
                    grid_id,
                    block.coordinate,
                    "replay_construction_target_invalid",
                    "construction event is not on the exact closest visible face targeted by its authenticated actor",
                )?;
                if self.player_intersects_grid_coordinate(grid, block.coordinate) {
                    return Err(IntentError::rejected(
                        "replay_construction_intersects_player",
                        "placed frame intersects the authoritative player collider",
                    ));
                }
                if self.inventory(component_inventory_id)?.contents.components
                    < block.component_cost
                {
                    return Err(IntentError::rejected(
                        "replay_construction_components_invalid",
                        "placed frame exceeds the authoritative component inventory",
                    ));
                }
                self.inventory_mut(component_inventory_id)?
                    .contents
                    .components -= block.component_cost;
                if let Some(inventory_id) = &block.inventory_id {
                    self.inventories.insert(
                        inventory_id.clone(),
                        InventoryRecord {
                            inventory_id: inventory_id.clone(),
                            domain: InventoryDomain::Cargo {
                                block_id: block.block_id.clone(),
                            },
                            contents: InventoryContents::default(),
                            capacity_liters: CARGO_INVENTORY_CAPACITY_LITERS,
                        },
                    );
                }
                self.grid_mut(grid_id)?
                    .blocks
                    .insert(block.block_id.clone(), block.clone());
                self.ledger.built_blocks += 1;
            }
            EventPayload::BlockWelded {
                grid_id,
                block_id,
                previous_health,
                new_health,
                max_health,
                completed_construction,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated weld event has a human actor");
                let actor = self
                    .player
                    .get(actor_player_id)
                    .expect("validated weld actor is present");
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                let grid = self.grid(grid_id)?;
                let _targeted_block = grid.blocks.get(block_id).ok_or_else(|| {
                    IntentError::rejected("replay_block_missing", "weld target is missing")
                })?;
                self.ensure_block_tool_target(
                    actor,
                    grid_id,
                    block_id,
                    "replay_weld_target_invalid",
                    "weld event does not match the authenticated actor's closest visible tool target",
                )?;
                let block = self
                    .grid_mut(grid_id)?
                    .blocks
                    .get_mut(block_id)
                    .ok_or_else(|| {
                        IntentError::rejected("replay_block_missing", "weld target is missing")
                    })?;
                if block.health != *previous_health || block.max_health() != *max_health {
                    return Err(IntentError::rejected(
                        "replay_integrity_mismatch",
                        "weld event does not match the block integrity state",
                    ));
                }
                if *previous_health >= *max_health {
                    return Err(IntentError::rejected(
                        "replay_weld_no_change",
                        "weld event cannot target a block already at full integrity",
                    ));
                }
                let expected_health = previous_health
                    .saturating_add(max_health.div_ceil(4))
                    .min(*max_health);
                if *new_health != expected_health {
                    return Err(IntentError::rejected(
                        "replay_weld_increment_invalid",
                        "weld event does not match the canonical integrity increment",
                    ));
                }
                let expected_completion =
                    !block.construction_complete && *new_health == *max_health;
                if *completed_construction != expected_completion {
                    return Err(IntentError::rejected(
                        "replay_construction_completion_invalid",
                        "weld event completion does not match construction state",
                    ));
                }
                block.health = *new_health;
                block.construction_complete |= *completed_construction;
            }
            EventPayload::GridControlSet {
                grid_id,
                linear_input,
                angular_input,
                dampeners,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated grid control event has a human actor");
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                ensure_finite(*linear_input, "replayed grid linear control")?;
                ensure_finite(*angular_input, "replayed grid angular control")?;
                let grid = self.grid(grid_id)?;
                if grid.anchored
                    || !grid.power().online
                    || linear_input.magnitude() > MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_EPSILON
                    || angular_input.magnitude() > MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_EPSILON
                {
                    return Err(IntentError::rejected(
                        "replay_grid_control_invalid",
                        "grid control requires an owned, powered, released grid and normalized finite inputs",
                    ));
                }
                let grid = self.grid_mut(grid_id)?;
                grid.control_linear_input = *linear_input;
                grid.control_angular_input = *angular_input;
                grid.dampeners = *dampeners;
            }
            EventPayload::GridAnchorSet {
                grid_id,
                anchored,
                reward_credited,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated anchor event has a human actor");
                self.ensure_actor_owns_grid(actor_player_id, grid_id)?;
                let grid = self.grid(grid_id)?;
                let expected_reward = *anchored && grid.anchor_reward_eligible;
                if *anchored == grid.anchored
                    || (*anchored && (!grid.power().online || !grid.anchor_touches(&self.voxels)))
                    || *reward_credited != expected_reward
                {
                    return Err(IntentError::rejected(
                        "replay_grid_anchor_invalid",
                        "anchor event must be an authorized toggle with canonical power, contact, and reward state",
                    ));
                }
                let grid = self.grid_mut(grid_id)?;
                grid.anchored = *anchored;
                if *anchored {
                    if *reward_credited {
                        grid.anchor_reward_eligible = false;
                    }
                    grid.linear_velocity = Vec3::ZERO;
                    grid.angular_velocity = Vec3::ZERO;
                    grid.control_linear_input = Vec3::ZERO;
                    grid.control_angular_input = Vec3::ZERO;
                }
            }
            EventPayload::BlockDamaged {
                grid_id,
                block_id,
                damage,
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated damage event has a human actor");
                let actor = self
                    .player
                    .get(actor_player_id)
                    .expect("validated damage actor is present");
                if *damage != 35 {
                    return Err(IntentError::rejected(
                        "replay_damage_amount_invalid",
                        "hand-tool damage must match the canonical amount",
                    ));
                }
                let grid = self.grid(grid_id)?;
                let _block = grid.blocks.get(block_id).ok_or_else(|| {
                    IntentError::rejected("replay_block_missing", "damage target is missing")
                })?;
                self.ensure_block_tool_target(
                    actor,
                    grid_id,
                    block_id,
                    "replay_damage_target_invalid",
                    "damage event does not match the authenticated actor's closest visible tool target",
                )?;
                self.apply_damage(grid_id, block_id, *damage, event.event_sequence)?;
            }
            EventPayload::PhysicsStepCommitted {
                fixed_step_hz,
                step_count,
                remaining_step_phase,
                bodies,
                players,
                contacts,
                active_contacts_after,
            } => {
                let expected_fixed_step_hz = content::manifest().physics.fixed_step_hz;
                if *fixed_step_hz != expected_fixed_step_hz
                    || *step_count == 0
                    || *step_count > 15
                    || *remaining_step_phase >= 1_000_000_000
                {
                    return Err(IntentError::rejected(
                        "replay_physics_timing_invalid",
                        "physics timing must use the configured fixed step, a bounded positive step count, and a substep remainder",
                    ));
                }
                if bodies.len() != self.grids.len() {
                    return Err(IntentError::rejected(
                        "replay_physics_body_count_invalid",
                        "physics outcome must contain every authoritative grid exactly once",
                    ));
                }
                let physics_limits = physics_scene_config();
                let living_player_count = self
                    .player
                    .iter()
                    .filter(|(player_id, player)| {
                        !self.player_transfer_locks.contains_key(*player_id)
                            && matches!(player.life_state, PlayerLifeState::Alive)
                    })
                    .count();
                if players.len() != living_player_count {
                    return Err(IntentError::rejected(
                        "replay_player_physics_presence_invalid",
                        "physics outcome must contain every living player exactly once",
                    ));
                }
                let mut scheduled_players = self.player.by_id.clone();
                scheduled_players
                    .retain(|player_id, _| !self.player_transfer_locks.contains_key(player_id));
                for scheduled_player in scheduled_players
                    .values_mut()
                    .filter(|player| matches!(player.life_state, PlayerLifeState::Alive))
                {
                    for substep_index in 0..u64::from(*step_count) {
                        advance_player_control_for_substep(
                            scheduled_player,
                            self.simulation_tick.saturating_add(substep_index),
                        );
                    }
                }
                let mut seen_players = BTreeSet::new();
                for player in players {
                    if !seen_players.insert(player.player_id.as_str()) {
                        return Err(IntentError::rejected(
                            "replay_player_physics_duplicate",
                            "physics outcome contains a duplicate player",
                        ));
                    }
                    let Some(prior_player) = self.player.get(&player.player_id) else {
                        return Err(IntentError::rejected(
                            "replay_player_physics_identity_invalid",
                            "physics outcome identifies a player outside this cell",
                        ));
                    };
                    if !matches!(prior_player.life_state, PlayerLifeState::Alive) {
                        return Err(IntentError::rejected(
                            "replay_player_physics_presence_invalid",
                            "physics outcome cannot contain an incapacitated player",
                        ));
                    }
                    let scheduled_player = scheduled_players
                        .get(&player.player_id)
                        .expect("validated player is present in the scheduled roster");
                    ensure_finite(player.position, "replayed player position")?;
                    ensure_finite(player.linear_velocity, "replayed player velocity")?;
                    ensure_finite(player.angular_velocity, "replayed player angular velocity")?;
                    let orientation_length_squared = f64::from(player.orientation.x).mul_add(
                        f64::from(player.orientation.x),
                        f64::from(player.orientation.y).mul_add(
                            f64::from(player.orientation.y),
                            f64::from(player.orientation.z).mul_add(
                                f64::from(player.orientation.z),
                                f64::from(player.orientation.w) * f64::from(player.orientation.w),
                            ),
                        ),
                    );
                    if !player.orientation.is_finite()
                        || (orientation_length_squared - 1.0).abs() > 1.0e-3
                    {
                        return Err(IntentError::rejected(
                            "replay_player_physics_rotation_invalid",
                            "player physics rotation must be finite and unit length",
                        ));
                    }
                    if player.linear_velocity.magnitude()
                        > f64::from(physics_limits.max_linear_velocity_mps)
                        || player.angular_velocity.magnitude()
                            > f64::from(physics_limits.max_angular_velocity_radians_per_second)
                    {
                        return Err(IntentError::rejected(
                            "replay_player_physics_velocity_invalid",
                            "player physics velocity exceeds the authoritative solver limit",
                        ));
                    }
                    ensure_player_motion_continuity(
                        prior_player,
                        player,
                        *step_count,
                        &physics_limits,
                    )?;
                    let resulting_tick =
                        self.simulation_tick.saturating_add(u64::from(*step_count));
                    validate_player_locomotion_outcome(
                        self,
                        scheduled_player,
                        player,
                        resulting_tick,
                    )?;
                    let lease_active =
                        resulting_tick < scheduled_player.control_expires_at_simulation_tick;
                    let expected_linear = if lease_active {
                        scheduled_player.control_linear_input
                    } else {
                        Vec3::ZERO
                    };
                    let expected_angular = if lease_active {
                        scheduled_player.control_angular_input
                    } else {
                        Vec3::ZERO
                    };
                    if player.control_linear_input != expected_linear
                        || player.control_angular_input != expected_angular
                        || player.boost != (scheduled_player.boost && lease_active)
                        || player.jump != (scheduled_player.jump && lease_active)
                        || player.dampeners != (scheduled_player.dampeners || !lease_active)
                        || player.control_expires_at_simulation_tick
                            != scheduled_player.control_expires_at_simulation_tick
                    {
                        return Err(IntentError::rejected(
                            "replay_player_physics_control_invalid",
                            "player physics control and lease outcome is not canonical",
                        ));
                    }
                }
                let canonical_player_ids = self
                    .player
                    .iter()
                    .filter(|(_, player)| matches!(player.life_state, PlayerLifeState::Alive))
                    .map(|(player_id, _)| player_id.as_str());
                if !players
                    .iter()
                    .map(|player| player.player_id.as_str())
                    .eq(canonical_player_ids)
                {
                    return Err(IntentError::rejected(
                        "replay_player_physics_order_invalid",
                        "physics player outcomes must use canonical player-ID order",
                    ));
                }
                let mut seen = BTreeSet::new();
                for body in bodies {
                    if !seen.insert(body.grid_id.as_str()) {
                        return Err(IntentError::rejected(
                            "replay_physics_body_duplicate",
                            "physics outcome contains a duplicate grid",
                        ));
                    }
                    ensure_finite(body.position, "replayed grid position")?;
                    ensure_finite(body.linear_velocity, "replayed grid velocity")?;
                    ensure_finite(body.angular_velocity, "replayed grid angular velocity")?;
                    let orientation_length_squared = f64::from(body.orientation.x).mul_add(
                        f64::from(body.orientation.x),
                        f64::from(body.orientation.y).mul_add(
                            f64::from(body.orientation.y),
                            f64::from(body.orientation.z).mul_add(
                                f64::from(body.orientation.z),
                                f64::from(body.orientation.w) * f64::from(body.orientation.w),
                            ),
                        ),
                    );
                    if !body.orientation.is_finite()
                        || (orientation_length_squared - 1.0).abs() > 1.0e-3
                    {
                        return Err(IntentError::rejected(
                            "replay_physics_rotation_invalid",
                            "physics outcome rotation must be finite and unit length",
                        ));
                    }
                    if body.linear_velocity.magnitude()
                        > f64::from(physics_limits.max_linear_velocity_mps)
                        || body.angular_velocity.magnitude()
                            > f64::from(physics_limits.max_angular_velocity_radians_per_second)
                    {
                        return Err(IntentError::rejected(
                            "replay_physics_body_velocity_invalid",
                            "physics outcome velocity exceeds the authoritative solver limit",
                        ));
                    }
                    let Some(grid) = self.grids.get(&body.grid_id) else {
                        return Err(IntentError::rejected(
                            "replay_physics_body_identity_invalid",
                            "physics outcome must identify an authoritative grid",
                        ));
                    };
                    let orientation_dot = f64::from(body.orientation.x)
                        * f64::from(grid.orientation.x)
                        + f64::from(body.orientation.y) * f64::from(grid.orientation.y)
                        + f64::from(body.orientation.z) * f64::from(grid.orientation.z)
                        + f64::from(body.orientation.w) * f64::from(grid.orientation.w);
                    if grid.anchored
                        && (body.address != grid.address
                            || (orientation_dot.abs() - 1.0).abs() > 1.0e-3
                            || body.linear_velocity != Vec3::ZERO
                            || body.angular_velocity != Vec3::ZERO)
                    {
                        return Err(IntentError::rejected(
                            "replay_physics_anchored_body_invalid",
                            "anchored grid pose must remain unchanged and its velocity must remain zero",
                        ));
                    }
                    if !grid.anchored {
                        ensure_dynamic_body_motion_continuity(
                            grid.position,
                            grid.orientation,
                            body.position,
                            body.orientation,
                            *step_count,
                            grid_local_center_of_mass(self, grid),
                            &physics_limits,
                        )?;
                    }
                }
                if bodies
                    .windows(2)
                    .any(|pair| pair[0].grid_id.as_str() >= pair[1].grid_id.as_str())
                {
                    return Err(IntentError::rejected(
                        "replay_physics_body_order_invalid",
                        "physics grid outcomes must use canonical grid-ID order",
                    ));
                }
                let mut contacts_by_substep = vec![Vec::new(); usize::from(*step_count)];
                for contact in contacts {
                    if contact.substep_index >= *step_count {
                        return Err(IntentError::rejected(
                            "replay_physics_contact_substep_invalid",
                            "physics contact substep must refer to a committed solver step",
                        ));
                    }
                    ensure_finite(contact.point, "replayed contact point")?;
                    ensure_finite(contact.normal, "replayed contact normal")?;
                    let normal_length_squared = contact.normal.x * contact.normal.x
                        + contact.normal.y * contact.normal.y
                        + contact.normal.z * contact.normal.z;
                    if (normal_length_squared - 1.0).abs() > 0.000_01 {
                        return Err(IntentError::rejected(
                            "replay_physics_contact_normal_invalid",
                            "physics contact normal must have unit length",
                        ));
                    }
                    if !contact.penetration_m.is_finite() || contact.penetration_m < 0.0 {
                        return Err(IntentError::rejected(
                            "replay_physics_contact_invalid",
                            "physics contact values must be finite and non-negative",
                        ));
                    }
                    let key = ContactPairKey {
                        body_a: contact.body_a_id.clone(),
                        collider_a: contact.collider_a_id.clone(),
                        body_b: contact.body_b_id.clone(),
                        collider_b: contact.collider_b_id.clone(),
                    };
                    if (&key.body_b, &key.collider_b) < (&key.body_a, &key.collider_a)
                        || !self.physics_collider_exists(&key.body_a, &key.collider_a)
                        || !self.physics_collider_exists(&key.body_b, &key.collider_b)
                    {
                        return Err(IntentError::rejected(
                            "replay_physics_contact_identity_invalid",
                            "physics contact identities must be canonical live colliders",
                        ));
                    }
                    if contact.reduced_translational_mass_grams
                        != reduced_translational_contact_mass_grams(
                            self,
                            &contact.body_a_id,
                            &contact.body_b_id,
                        )
                    {
                        return Err(IntentError::rejected(
                            "replay_physics_contact_mass_invalid",
                            "physics contact reduced translational mass does not match canonical content",
                        ));
                    }
                    let left_player = player_for_body_id(self, &key.body_a);
                    let right_player = player_for_body_id(self, &key.body_b);
                    if left_player.is_some() && right_player.is_some() {
                        return Err(IntentError::rejected(
                            "replay_character_contact_forbidden",
                            "character collision layers do not produce character-to-character contacts",
                        ));
                    }
                    if let Some(contact_player_id) = player_id_for_contact(self, &key) {
                        let player = players
                            .iter()
                            .find(|player| player.player_id == contact_player_id)
                            .expect("validated living player outcome exists for a player contact");
                        let prior_player = self
                            .player
                            .get(contact_player_id)
                            .expect("contact player is present in the canonical roster");
                        if !self.player_contact_is_spatially_plausible(
                            contact,
                            prior_player,
                            player,
                            bodies,
                            *step_count,
                            &physics_limits,
                        ) {
                            return Err(IntentError::rejected(
                                "replay_player_contact_spatially_invalid",
                                format!(
                                    "player contact must lie on the plausible swept player and counterpart geometry: substep={} pair={}/{}:{}/{} point=({:.6},{:.6},{:.6}) penetration_m={:.6} closing_speed_mm_per_second={}",
                                    contact.substep_index,
                                    contact.body_a_id,
                                    contact.collider_a_id,
                                    contact.body_b_id,
                                    contact.collider_b_id,
                                    contact.point.x,
                                    contact.point.y,
                                    contact.point.z,
                                    contact.penetration_m,
                                    contact.closing_speed_mm_per_second,
                                ),
                            ));
                        }
                    }
                    contacts_by_substep[usize::from(contact.substep_index)].push((key, contact));
                }
                if contacts.windows(2).any(|pair| {
                    (
                        pair[0].substep_index,
                        pair[0].body_a_id.as_str(),
                        pair[0].collider_a_id.as_str(),
                        pair[0].body_b_id.as_str(),
                        pair[0].collider_b_id.as_str(),
                    ) >= (
                        pair[1].substep_index,
                        pair[1].body_a_id.as_str(),
                        pair[1].collider_a_id.as_str(),
                        pair[1].body_b_id.as_str(),
                        pair[1].collider_b_id.as_str(),
                    )
                }) {
                    return Err(IntentError::rejected(
                        "replay_physics_contact_order_invalid",
                        "physics contacts must use canonical substep and collider-pair order",
                    ));
                }
                let mut active = self.active_contact_pairs.clone();
                for substep in contacts_by_substep {
                    let mut current = BTreeSet::new();
                    for (key, contact) in substep {
                        if !current.insert(key.clone()) {
                            return Err(IntentError::rejected(
                                "replay_physics_contact_duplicate",
                                "physics outcome contains a duplicate contact pair",
                            ));
                        }
                        let expected_phase = if active.contains(&key) {
                            PhysicsContactPhase::Persisted
                        } else {
                            PhysicsContactPhase::Began
                        };
                        if contact.phase != expected_phase {
                            return Err(IntentError::rejected(
                                "replay_physics_contact_phase_invalid",
                                "physics contact lifecycle does not match canonical state",
                            ));
                        }
                    }
                    active = current;
                }
                if active_contacts_after.as_slice()
                    != active.iter().cloned().collect::<Vec<_>>().as_slice()
                {
                    return Err(IntentError::rejected(
                        "replay_physics_active_contacts_invalid",
                        "physics active-contact outcome does not match the final substep",
                    ));
                }
                for player in players {
                    let expected_surface_contact = active
                        .iter()
                        .any(|contact| contact_key_involves_player_id(contact, &player.player_id))
                        || player.locomotion.support.is_some();
                    if player.surface_contact != expected_surface_contact {
                        return Err(IntentError::rejected(
                            "replay_player_surface_contact_invalid",
                            "player surface contact must match the final authoritative contact set",
                        ));
                    }
                }
                for body in bodies {
                    let grid = self
                        .grids
                        .get_mut(&body.grid_id)
                        .expect("validated physics body identifies a live grid");
                    grid.address = body.address.clone();
                    grid.position = body.position;
                    grid.orientation = body.orientation;
                    grid.linear_velocity = body.linear_velocity;
                    grid.angular_velocity = body.angular_velocity;
                }
                for player in players {
                    let scheduled_player = scheduled_players
                        .remove(&player.player_id)
                        .expect("validated physics outcome has scheduled state");
                    let canonical_player = self
                        .player
                        .get_mut(&player.player_id)
                        .expect("validated physics outcome has canonical state");
                    canonical_player.address = player.address.clone();
                    canonical_player.position = player.position;
                    canonical_player.orientation = player.orientation;
                    canonical_player.linear_velocity = player.linear_velocity;
                    canonical_player.angular_velocity = player.angular_velocity;
                    canonical_player.surface_contact = player.surface_contact;
                    canonical_player.locomotion = player.locomotion.clone();
                    canonical_player.last_processed_input_sequence =
                        scheduled_player.last_processed_input_sequence;
                    canonical_player.pending_control_frames =
                        scheduled_player.pending_control_frames;
                    canonical_player.control_linear_input = player.control_linear_input;
                    canonical_player.control_angular_input = player.control_angular_input;
                    canonical_player.boost = player.boost;
                    canonical_player.dampeners = player.dampeners;
                    canonical_player.jump = player.jump;
                    canonical_player.control_expires_at_simulation_tick =
                        player.control_expires_at_simulation_tick;
                }
                self.active_contact_pairs = active;
                self.physics_step_phase = u64::from(*remaining_step_phase);
                self.simulation_tick = self.simulation_tick.saturating_add(u64::from(*step_count));
            }
            EventPayload::PlayerTransferPrepared {
                package,
                directory_transfer,
            } => {
                *self = stage_prepared_eva_lock(self, package, directory_transfer).map_err(
                    |source| {
                        IntentError::rejected("replay_transfer_prepare_invalid", source.to_string())
                    },
                )?;
            }
            EventPayload::PlayerTransferQuarantined { package, receipt } => {
                let (staged, expected_receipt) =
                    stage_eva_player_quarantine(self, receipt.destination_fencing_token, package)
                        .map_err(|source| {
                        IntentError::rejected(
                            "replay_transfer_quarantine_invalid",
                            source.to_string(),
                        )
                    })?;
                if &expected_receipt != receipt {
                    return Err(IntentError::rejected(
                        "replay_transfer_quarantine_invalid",
                        "quarantine event does not contain the canonical receipt",
                    ));
                }
                *self = staged;
            }
            EventPayload::PlayerTransferAborted {
                package,
                directory_transfer,
            } => {
                *self = stage_aborted_eva_unlock(self, package, directory_transfer).map_err(
                    |source| {
                        IntentError::rejected("replay_transfer_abort_invalid", source.to_string())
                    },
                )?;
            }
            EventPayload::PlayerTransferExported {
                package,
                directory_transfer,
            } => {
                *self = stage_committed_eva_export(self, package, directory_transfer).map_err(
                    |source| {
                        IntentError::rejected("replay_transfer_export_invalid", source.to_string())
                    },
                )?;
            }
            EventPayload::PlayerTransferImported {
                package,
                receipt,
                directory_transfer,
            } => {
                *self = stage_committed_eva_import(self, package, receipt, directory_transfer)
                    .map_err(|source| {
                        IntentError::rejected("replay_transfer_import_invalid", source.to_string())
                    })?;
            }
        }

        let experience_reward = event.payload.experience_reward();
        if experience_reward > 0 {
            let actor_player_id = event
                .actor_player_id
                .as_deref()
                .expect("reward-bearing events have a validated human actor");
            let actor = self
                .player
                .get_mut(actor_player_id)
                .expect("validated reward actor is present");
            actor.experience = actor.experience.saturating_add(experience_reward);
        }
        match &event.payload {
            EventPayload::VoxelMined { .. } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated mining event has a human actor");
                self.player
                    .get_mut(actor_player_id)
                    .expect("validated mining actor is present")
                    .career
                    .voxels_mined += 1;
            }
            EventPayload::OreRefined { batches, .. } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated refining event has a human actor");
                self.player
                    .get_mut(actor_player_id)
                    .expect("validated refining actor is present")
                    .career
                    .refining_batches += batches;
            }
            EventPayload::ComponentCrafted { quantity, .. } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated crafting event has a human actor");
                self.player
                    .get_mut(actor_player_id)
                    .expect("validated crafting actor is present")
                    .career
                    .components_crafted += quantity;
            }
            EventPayload::BlockWelded {
                completed_construction: true,
                ..
            } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated weld event has a human actor");
                self.player
                    .get_mut(actor_player_id)
                    .expect("validated weld actor is present")
                    .career
                    .blocks_built += 1;
            }
            EventPayload::GridAnchorSet { anchored: true, .. } => {
                let actor_player_id = event
                    .actor_player_id
                    .as_deref()
                    .expect("validated anchor event has a human actor");
                self.player
                    .get_mut(actor_player_id)
                    .expect("validated anchor actor is present")
                    .career
                    .anchors_engaged += 1;
            }
            EventPayload::PlayerControlSet { .. }
            | EventPayload::SuitModeChanged { .. }
            | EventPayload::SuitOxygenChanged { .. }
            | EventPayload::PlayerIncapacitated { .. }
            | EventPayload::PlayerRespawned { .. }
            | EventPayload::ProductionQueued { .. }
            | EventPayload::ProductionQuantumCommitted { .. }
            | EventPayload::InventoryTransferred { .. }
            | EventPayload::BlockBuilt { .. }
            | EventPayload::BlockWelded { .. }
            | EventPayload::GridControlSet { .. }
            | EventPayload::GridAnchorSet {
                anchored: false, ..
            }
            | EventPayload::BlockDamaged { .. }
            | EventPayload::PhysicsStepCommitted { .. }
            | EventPayload::PlayerTransferPrepared { .. }
            | EventPayload::PlayerTransferQuarantined { .. }
            | EventPayload::PlayerTransferAborted { .. }
            | EventPayload::PlayerTransferExported { .. }
            | EventPayload::PlayerTransferImported { .. } => {}
        }

        self.event_sequence = event.event_sequence;
        self.fencing_token = event.authority_fencing_token;
        self.last_event_hash.clone_from(&event.event_hash);
        if let (
            Some(actor_player_id),
            Some(operation_id),
            Some(operation_sequence),
            Some(intent_fingerprint),
        ) = (
            &event.actor_player_id,
            &event.operation_id,
            event.operation_sequence,
            &event.intent_fingerprint,
        ) {
            let (code, message) = event.payload.receipt();
            self.record_processed_operation(
                actor_player_id,
                ProcessedOperationRecord {
                    operation_id: operation_id.clone(),
                    intent_fingerprint: intent_fingerprint.clone(),
                    receipt_origin_cell_id: self.cell_id.clone(),
                    receipt: IntentReceipt {
                        operation_sequence,
                        operation_id: operation_id.clone(),
                        event_sequence: event.event_sequence,
                        code: code.into(),
                        message,
                    },
                },
            )
            .map_err(|message| {
                IntentError::rejected("replay_operation_history_invalid", message)
            })?;
        }

        if !self.conservation().valid {
            return Err(IntentError::ConservationViolation {
                event_sequence: event.event_sequence,
            });
        }
        Ok(())
    }

    fn apply_damage(
        &mut self,
        grid_id: &str,
        block_id: &str,
        damage: u16,
        event_sequence: u64,
    ) -> Result<(), IntentError> {
        let grid_owner_player_id = self.grid(grid_id)?.owner_player_id.clone();
        let removed = {
            let grid = self.grid_mut(grid_id)?;
            let block = grid.blocks.get_mut(block_id).ok_or_else(|| {
                IntentError::rejected("replay_block_missing", "damage target is missing")
            })?;
            if block.health > damage {
                block.health -= damage;
                None
            } else {
                grid.blocks.remove(block_id)
            }
        };

        let Some(removed) = removed else {
            return Ok(());
        };
        self.ledger.destroyed_blocks += 1;
        self.ledger.destroyed_components += removed.component_cost;
        if let Some(inventory_id) = &removed.inventory_id
            && let Some(inventory) = self.inventories.get_mut(inventory_id)
        {
            inventory.domain = InventoryDomain::Dropped {
                reason: "cargo_block_destroyed".into(),
                owner_player_id: grid_owner_player_id.clone(),
            };
        }
        if let Some(queue) = self.production_queues.remove(block_id) {
            let mut contents = InventoryContents::default();
            for job in queue {
                add_contents(&mut contents, &job.reserved_inputs)?;
                add_contents(&mut contents, &job.pending_outputs)?;
            }
            let inventory_id = format!("inventory-production-drop-{event_sequence}-{block_id}");
            if self.inventories.contains_key(&inventory_id) {
                return Err(IntentError::rejected(
                    "replay_production_drop_identity_duplicate",
                    "deterministic machine-destruction drop identity is already in use",
                ));
            }
            self.inventories.insert(
                inventory_id.clone(),
                InventoryRecord {
                    inventory_id,
                    domain: InventoryDomain::Dropped {
                        reason: "production_machine_destroyed".into(),
                        owner_player_id: grid_owner_player_id,
                    },
                    contents,
                    capacity_liters: u64::MAX,
                },
            );
        }
        self.split_disconnected_grid(grid_id, event_sequence)?;
        Ok(())
    }

    fn split_disconnected_grid(
        &mut self,
        grid_id: &str,
        event_sequence: u64,
    ) -> Result<(), IntentError> {
        let original = self.grids.get(grid_id).cloned().ok_or_else(|| {
            IntentError::rejected("replay_grid_missing", "grid split target is missing")
        })?;
        if original.blocks.is_empty() {
            self.grids.remove(grid_id);
            return Ok(());
        }

        let by_coordinate = original
            .blocks
            .values()
            .map(|block| (block.coordinate, block.block_id.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut remaining = by_coordinate.keys().copied().collect::<BTreeSet<_>>();
        let mut components = Vec::new();

        while let Some(start) = remaining.iter().next().copied() {
            let mut queue = VecDeque::from([start]);
            let mut component = BTreeSet::new();
            remaining.remove(&start);
            while let Some(coordinate) = queue.pop_front() {
                component.insert(coordinate);
                for neighbor in coordinate.neighbors() {
                    if remaining.remove(&neighbor) {
                        queue.push_back(neighbor);
                    }
                }
            }
            components.push(component);
        }

        components.sort_by_key(|component| component.iter().next().copied());
        let primary_index = components
            .iter()
            .position(|component| {
                component.iter().any(|coordinate| {
                    let block_id = &by_coordinate[coordinate];
                    original.blocks[block_id].kind == BlockKind::ControlCore
                })
            })
            .unwrap_or(0);

        let split_grid_ids = components
            .iter()
            .enumerate()
            .map(|(index, _)| {
                if index == primary_index {
                    original.grid_id.clone()
                } else {
                    format!("{}-split-{event_sequence}-{index}", original.grid_id)
                }
            })
            .collect::<Vec<_>>();
        if split_grid_ids
            .iter()
            .enumerate()
            .any(|(index, split_id)| index != primary_index && self.grids.contains_key(split_id))
        {
            return Err(IntentError::rejected(
                "replay_grid_split_identity_duplicate",
                "deterministic grid split identity is already in use",
            ));
        }

        let did_split = components.len() > 1;
        self.grids.remove(grid_id);

        for (index, component) in components.into_iter().enumerate() {
            let new_grid_id = split_grid_ids[index].clone();
            let blocks = component
                .iter()
                .map(|coordinate| {
                    let block_id = &by_coordinate[coordinate];
                    (block_id.clone(), original.blocks[block_id].clone())
                })
                .collect();
            let mut grid = Grid {
                grid_id: new_grid_id.clone(),
                owner_player_id: original.owner_player_id.clone(),
                anchor_reward_eligible: original.anchor_reward_eligible && index == primary_index,
                address: original.address.clone(),
                position: original.position,
                orientation: original.orientation,
                linear_velocity: original.linear_velocity,
                angular_velocity: original.angular_velocity,
                control_linear_input: if did_split {
                    Vec3::ZERO
                } else {
                    original.control_linear_input
                },
                control_angular_input: if did_split {
                    Vec3::ZERO
                } else {
                    original.control_angular_input
                },
                dampeners: original.dampeners || did_split,
                anchored: original.anchored,
                blocks,
            };
            grid.anchored = grid.anchored && grid.anchor_touches(&self.voxels);
            self.grids.insert(new_grid_id, grid);
        }
        Ok(())
    }

    fn physics_collider_exists(&self, body_id: &str, collider_id: &str) -> bool {
        if let Some(player) = player_for_body_id(self, body_id) {
            return matches!(player.life_state, PlayerLifeState::Alive)
                && collider_id == player_collider_id(&player.player_id);
        }
        if body_id == PLANET_BODY_ID {
            return collider_id == PLANET_COLLIDER_ID;
        }
        if let Some(grid) = self.grids.get(body_id) {
            return grid.blocks.contains_key(collider_id);
        }
        self.voxels.occupied.iter().any(|coordinate| {
            voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(*coordinate)) == body_id
                && voxel_collision_collider_id(*coordinate) == collider_id
        })
    }

    fn inventory(&self, inventory_id: &str) -> Result<&InventoryRecord, IntentError> {
        self.inventories.get(inventory_id).ok_or_else(|| {
            IntentError::rejected(
                "inventory_missing",
                format!("inventory {inventory_id} does not exist"),
            )
        })
    }

    fn ensure_inventory_functional(&self, inventory_id: &str) -> Result<(), IntentError> {
        let inventory = self.inventory(inventory_id)?;
        let block_id = match &inventory.domain {
            InventoryDomain::Player { .. } => return Ok(()),
            InventoryDomain::Dropped { .. } => {
                return Err(IntentError::rejected(
                    "dropped_inventory_sealed",
                    "dropped inventory requires an explicit recovery or salvage action",
                ));
            }
            InventoryDomain::Cargo { block_id } => block_id,
        };
        let block = self
            .grids
            .values()
            .find_map(|grid| grid.blocks.get(block_id))
            .ok_or_else(|| {
                IntentError::rejected(
                    "inventory_owner_missing",
                    format!("cargo inventory {inventory_id} has no live owner block"),
                )
            })?;
        if block.construction_complete {
            Ok(())
        } else {
            Err(IntentError::rejected(
                "inventory_block_incomplete",
                "cargo inventory remains sealed until its block finishes construction",
            ))
        }
    }

    /// Resolve inventory authority from canonical ownership rather than from
    /// client-selected IDs. Carried inventory belongs to its linked player;
    /// cargo inherits the owner of the one grid containing its live block.
    /// Dropped inventory deliberately has no generic-use authority.
    fn ensure_actor_owns_inventory(
        &self,
        actor_player_id: &str,
        inventory_id: &str,
    ) -> Result<(), IntentError> {
        let inventory = self.inventory(inventory_id)?;
        if matches!(inventory.domain, InventoryDomain::Dropped { .. }) {
            return Err(IntentError::rejected(
                "dropped_inventory_sealed",
                "dropped inventory requires an explicit recovery or salvage action",
            ));
        }
        let owner_player_id = self
            .inventory_owner_player_id(inventory_id)
            .map_err(|message| IntentError::rejected("inventory_authority_invalid", message))?;
        if owner_player_id != actor_player_id {
            return Err(IntentError::rejected(
                "inventory_access_denied",
                "the authenticated player cannot access the selected inventory",
            ));
        }
        match &inventory.domain {
            InventoryDomain::Player { player_id } => {
                let actor = self.player.get(actor_player_id).ok_or_else(|| {
                    IntentError::rejected(
                        "actor_not_present",
                        "the authenticated inventory actor is not present",
                    )
                })?;
                if player_id == actor_player_id && actor.inventory_id == inventory_id {
                    Ok(())
                } else {
                    Err(IntentError::rejected(
                        "inventory_access_denied",
                        "the authenticated player cannot access the selected carried inventory",
                    ))
                }
            }
            InventoryDomain::Cargo { .. } => Ok(()),
            InventoryDomain::Dropped { .. } => unreachable!("dropped inventories reject above"),
        }
    }

    fn inventory_mut(&mut self, inventory_id: &str) -> Result<&mut InventoryRecord, IntentError> {
        self.inventories.get_mut(inventory_id).ok_or_else(|| {
            IntentError::rejected(
                "inventory_missing",
                format!("inventory {inventory_id} does not exist"),
            )
        })
    }

    fn grid(&self, grid_id: &str) -> Result<&Grid, IntentError> {
        self.grids.get(grid_id).ok_or_else(|| {
            IntentError::rejected("grid_missing", format!("grid {grid_id} does not exist"))
        })
    }

    fn ensure_actor_owns_grid(
        &self,
        actor_player_id: &str,
        grid_id: &str,
    ) -> Result<(), IntentError> {
        if self.grid(grid_id)?.owner_player_id == actor_player_id {
            Ok(())
        } else {
            Err(IntentError::rejected(
                "grid_access_denied",
                "the authenticated player cannot perform this action on the selected grid",
            ))
        }
    }

    fn grid_mut(&mut self, grid_id: &str) -> Result<&mut Grid, IntentError> {
        self.grids.get_mut(grid_id).ok_or_else(|| {
            IntentError::rejected("grid_missing", format!("grid {grid_id} does not exist"))
        })
    }

    fn ensure_voxel_tool_target(
        &self,
        actor: &Player,
        coordinate: IVec3,
        code: &str,
        message: &str,
    ) -> Result<(), IntentError> {
        let hit = closest_tool_hit(actor, &self.voxels, &self.grids);
        if matches!(
            hit,
            Some(ref hit)
                if hit.local_face.is_some()
                    && hit.target == ToolTarget::Voxel { coordinate }
        ) {
            Ok(())
        } else {
            Err(IntentError::rejected(code, message))
        }
    }

    fn ensure_block_tool_target(
        &self,
        actor: &Player,
        grid_id: &str,
        block_id: &str,
        code: &str,
        message: &str,
    ) -> Result<(), IntentError> {
        let hit = closest_tool_hit(actor, &self.voxels, &self.grids);
        if matches!(
            hit,
            Some(ref hit)
                if hit.local_face.is_some()
                    && matches!(
                        &hit.target,
                        ToolTarget::Block {
                            grid_id: targeted_grid_id,
                            block_id: targeted_block_id,
                            ..
                        } if targeted_grid_id == grid_id && targeted_block_id == block_id
                    )
        ) {
            Ok(())
        } else {
            Err(IntentError::rejected(code, message))
        }
    }

    fn ensure_build_tool_target(
        &self,
        actor: &Player,
        grid_id: &str,
        coordinate: IVec3,
        code: &str,
        message: &str,
    ) -> Result<(), IntentError> {
        let Some(hit) = closest_tool_hit(actor, &self.voxels, &self.grids) else {
            return Err(IntentError::rejected(code, message));
        };
        let Some(face) = hit.local_face else {
            return Err(IntentError::rejected(code, message));
        };
        let ToolTarget::Block {
            grid_id: targeted_grid_id,
            coordinate: targeted_coordinate,
            ..
        } = hit.target
        else {
            return Err(IntentError::rejected(code, message));
        };
        let expected = IVec3::new(
            targeted_coordinate.x.saturating_add(face.x),
            targeted_coordinate.y.saturating_add(face.y),
            targeted_coordinate.z.saturating_add(face.z),
        );
        if targeted_grid_id == grid_id && expected == coordinate {
            Ok(())
        } else {
            Err(IntentError::rejected(code, message))
        }
    }

    fn player_intersects_voxel(&self, position: Vec3, orientation: Quat) -> bool {
        let character = &content::manifest().character;
        let extent = character_capsule_half_height() + character.collision_radius_m + 0.5;
        let minimum = IVec3::new(
            (position.x - extent).floor() as i32,
            (position.y - extent).floor() as i32,
            (position.z - extent).floor() as i32,
        );
        let maximum = IVec3::new(
            (position.x + extent).ceil() as i32,
            (position.y + extent).ceil() as i32,
            (position.z + extent).ceil() as i32,
        );
        for x in minimum.x..=maximum.x {
            for y in minimum.y..=maximum.y {
                for z in minimum.z..=maximum.z {
                    let coordinate = IVec3::new(x, y, z);
                    if self.voxels.material(coordinate).is_some()
                        && capsule_intersects_unit_cube(position, orientation, coordinate)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn player_movement_hits_voxel(&self, start: Vec3, end: Vec3, orientation: Quat) -> bool {
        movement_samples(start, end)
            .into_iter()
            .any(|position| self.player_intersects_voxel(position, orientation))
    }

    fn player_intersects_grid(&self, position: Vec3, orientation: Quat) -> bool {
        let world_axis = orientation.rotate(Vec3::new(0.0, 1.0, 0.0));
        self.grids.values().any(|grid| {
            let relative = Vec3::new(
                position.x - grid.position.x,
                position.y - grid.position.y,
                position.z - grid.position.z,
            );
            let local_player = grid.orientation.conjugate().rotate(relative);
            let local_axis = grid.orientation.conjugate().rotate(world_axis);
            grid.blocks.values().any(|block| {
                capsule_axis_intersects_unit_cube(local_player, local_axis, block.coordinate)
            })
        })
    }

    fn player_intersects_grid_coordinate(&self, grid: &Grid, coordinate: IVec3) -> bool {
        self.player.iter().any(|(_, player)| {
            if !matches!(player.life_state, PlayerLifeState::Alive) {
                return false;
            }
            let relative = player.position - grid.position;
            let local_player = grid.orientation.conjugate().rotate(relative);
            let world_axis = player.orientation.rotate(Vec3::new(0.0, 1.0, 0.0));
            let local_axis = grid.orientation.conjugate().rotate(world_axis);
            capsule_axis_intersects_unit_cube(local_player, local_axis, coordinate)
        })
    }

    fn player_movement_hits_grid(&self, start: Vec3, end: Vec3, orientation: Quat) -> bool {
        movement_samples(start, end)
            .into_iter()
            .any(|position| self.player_intersects_grid(position, orientation))
    }

    fn player_contact_is_spatially_plausible(
        &self,
        contact: &PhysicsContactOutcome,
        prior_player: &Player,
        player: &PlayerPhysicsOutcome,
        bodies: &[PhysicsBodyOutcome],
        step_count: u8,
        limits: &SceneConfig,
    ) -> bool {
        let radius = content::manifest().character.collision_radius_m;
        let fixed_delta_seconds = f64::from(content::manifest().physics.fixed_delta_seconds);
        let completed_steps = f64::from(contact.substep_index) + 1.0;
        let per_step_reach = f64::from(limits.max_linear_velocity_mps) * fixed_delta_seconds
            + PLAYER_POSITION_CORRECTION_BUDGET_M_PER_STEP
            + REPLAY_QUANTIZATION_SLOP;
        // Jolt's linear-cast motion quality may report a speculative manifold
        // anywhere along the configured fixed-step velocity sweep. The stored
        // contact point is the midpoint between the two manifold surfaces, so
        // replay must allow half of that bounded separation. The event keeps
        // overlap as a non-negative penetration value and therefore cannot
        // recover the native manifold's negative speculative depth directly.
        let maximum_speculative_separation = (f64::from(limits.max_linear_velocity_mps)
            * fixed_delta_seconds)
            .max(PHYSICS_MINIMUM_SPECULATIVE_DISTANCE_M);
        let surface_slack = 0.5 * contact.penetration_m.max(maximum_speculative_separation)
            + PHYSICS_CONTACT_POINT_SLOP_M
            + REPLAY_QUANTIZATION_SLOP;
        let capsule_half_height = character_capsule_half_height();
        if point_capsule_axis_distance(
            contact.point,
            prior_player.position,
            prior_player.orientation,
            capsule_half_height,
        ) > completed_steps * per_step_reach + radius + surface_slack
        {
            return false;
        }
        if contact.substep_index + 1 == step_count
            && point_capsule_axis_distance(
                contact.point,
                player.position,
                player.orientation,
                capsule_half_height,
            ) > radius + per_step_reach + surface_slack
        {
            return false;
        }

        let player_body_id = player_body_id(&prior_player.player_id);
        let (other_body, other_collider) = if contact.body_a_id == player_body_id {
            (&contact.body_b_id, &contact.collider_b_id)
        } else if contact.body_b_id == player_body_id {
            (&contact.body_a_id, &contact.collider_a_id)
        } else {
            return false;
        };
        if player_for_body_id(self, other_body).is_some() {
            return false;
        }
        if other_body == PLANET_BODY_ID {
            return other_collider == PLANET_COLLIDER_ID
                && contact.penetration_m <= PLAYER_PLANET_PENETRATION_LIMIT_M
                && ((contact.point - planet_center()).magnitude() - planet_surface_radius_m())
                    .abs()
                    <= surface_slack;
        }
        if let Some(grid) = self.grids.get(other_body) {
            let Some(block) = grid.blocks.get(other_collider) else {
                return false;
            };
            if contact.penetration_m > PLAYER_BOX_PENETRATION_LIMIT_M {
                return false;
            }
            let local_center = Vec3::new(
                f64::from(block.coordinate.x),
                f64::from(block.coordinate.y),
                f64::from(block.coordinate.z),
            );
            if grid.anchored {
                let local_point = grid
                    .orientation
                    .conjugate()
                    .rotate(contact.point - grid.position);
                return point_unit_cube_distance(local_point, block.coordinate) <= surface_slack;
            }
            if !bodies.iter().any(|body| body.grid_id == *other_body) {
                return false;
            }
            let prior_center = grid.position + grid.orientation.rotate(local_center);
            let half_diagonal = 3.0_f64.sqrt() * 0.5;
            let angular_reach = (completed_steps
                * (f64::from(limits.max_angular_velocity_radians_per_second)
                    * fixed_delta_seconds
                    + PLAYER_ROTATION_SLOP_RADIANS_PER_STEP))
                .min(std::f64::consts::PI);
            let center_of_mass = grid_local_center_of_mass(self, grid);
            let orbit_radius = (local_center - center_of_mass).magnitude() + half_diagonal;
            let grid_point_reach = completed_steps * per_step_reach
                + half_diagonal
                + 2.0 * orbit_radius * (angular_reach * 0.5).sin();
            return contact.point.squared_distance(prior_center).sqrt()
                <= grid_point_reach + surface_slack;
        }
        let Some(coordinate) = self.voxels.occupied.iter().find(|coordinate| {
            voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(**coordinate))
                == *other_body
                && voxel_collision_collider_id(**coordinate) == *other_collider
        }) else {
            return false;
        };
        contact.penetration_m <= PLAYER_BOX_PENETRATION_LIMIT_M
            && point_unit_cube_distance(contact.point, *coordinate) <= surface_slack
    }
}

fn ensure_player_motion_continuity(
    prior: &crate::model::Player,
    outcome: &PlayerPhysicsOutcome,
    step_count: u8,
    limits: &SceneConfig,
) -> Result<(), IntentError> {
    let fixed_delta_seconds = f64::from(content::manifest().physics.fixed_delta_seconds);
    let maximum_translation = f64::from(step_count)
        * (f64::from(limits.max_linear_velocity_mps) * fixed_delta_seconds
            + PLAYER_POSITION_CORRECTION_BUDGET_M_PER_STEP)
        + REPLAY_QUANTIZATION_SLOP;
    if prior.position.squared_distance(outcome.position) > maximum_translation * maximum_translation
    {
        return Err(IntentError::rejected(
            "replay_player_physics_translation_invalid",
            "player physics translation exceeds the conservative fixed-step envelope",
        ));
    }
    let angular_displacement =
        quaternion_angular_displacement(prior.orientation, outcome.orientation);
    let maximum_rotation = f64::from(step_count)
        * (f64::from(limits.max_angular_velocity_radians_per_second) * fixed_delta_seconds
            + PLAYER_ROTATION_SLOP_RADIANS_PER_STEP)
        + REPLAY_QUANTIZATION_SLOP;
    if angular_displacement > maximum_rotation {
        return Err(IntentError::rejected(
            "replay_player_physics_rotation_continuity_invalid",
            "player physics rotation exceeds the conservative fixed-step envelope",
        ));
    }
    let radius = content::manifest().character.collision_radius_m;
    let capsule_axis = outcome.orientation.rotate(Vec3::new(0.0, 1.0, 0.0));
    let outcome_up = radial_up(outcome.position);
    let radial_capsule_extent = radius
        + character_capsule_half_height()
            * (capsule_axis.x * outcome_up.x
                + capsule_axis.y * outcome_up.y
                + capsule_axis.z * outcome_up.z)
                .abs();
    let planet_distance = (outcome.position - planet_center()).magnitude();
    if planet_distance
        < planet_surface_radius_m() + radial_capsule_extent - PLAYER_PLANET_PENETRATION_LIMIT_M
    {
        return Err(IntentError::rejected(
            "replay_player_planet_penetration_invalid",
            "player physics outcome penetrates beyond the planet contact tolerance",
        ));
    }
    Ok(())
}

fn validate_player_locomotion_outcome(
    state: &WorldState,
    scheduled_player: &Player,
    outcome: &PlayerPhysicsOutcome,
    resulting_tick: u64,
) -> Result<(), IntentError> {
    let locomotion = &outcome.locomotion;
    ensure_finite(locomotion.up, "replayed locomotion up")?;
    let up_length = locomotion.up.magnitude();
    let maximum_pitch = content::manifest()
        .character
        .maximum_view_pitch_degrees
        .to_radians();
    if (up_length - 1.0).abs() > 1.0e-5
        || !locomotion.view_pitch_radians.is_finite()
        || locomotion.view_pitch_radians.abs() > maximum_pitch + 1.0e-9
        || locomotion.magnetic_boots_enabled != scheduled_player.locomotion.magnetic_boots_enabled
        || locomotion.jump_held != scheduled_player.locomotion.jump_held
        || locomotion.jump_buffer_expires_at_simulation_tick
            > scheduled_player
                .locomotion
                .jump_buffer_expires_at_simulation_tick
        || locomotion.support_grace_expires_at_simulation_tick
            > resulting_tick.saturating_add(content::manifest().character.coyote_ticks)
        || locomotion.magnetic_reattach_after_simulation_tick
            > resulting_tick.saturating_add(
                content::manifest()
                    .character
                    .magnetic_reattach_lockout_ticks,
            )
    {
        return Err(IntentError::rejected(
            "replay_player_locomotion_invalid",
            "player locomotion orientation, view, input edge, or timing state is not canonical",
        ));
    }
    if scheduled_player.jetpack_enabled {
        if !matches!(locomotion.kind, LocomotionKind::Eva) || locomotion.support.is_some() {
            return Err(IntentError::rejected(
                "replay_player_locomotion_invalid",
                "jetpack locomotion must be EVA and unsupported",
            ));
        }
        return Ok(());
    }
    if matches!(locomotion.kind, LocomotionKind::Eva) {
        return Err(IntentError::rejected(
            "replay_player_locomotion_invalid",
            "EVA locomotion requires the authoritative jetpack mode",
        ));
    }
    let support_required = matches!(
        locomotion.kind,
        LocomotionKind::Grounded | LocomotionKind::Magnetic
    );
    if support_required != locomotion.support.is_some()
        || matches!(locomotion.kind, LocomotionKind::Magnetic) && !locomotion.magnetic_boots_enabled
    {
        return Err(IntentError::rejected(
            "replay_player_locomotion_invalid",
            "grounded and magnetic states require a valid support and magnetic mode requires armed boots",
        ));
    }
    if matches!(locomotion.kind, LocomotionKind::Grounded)
        && dot(locomotion.up, radial_up(outcome.position)) < 0.999_99
    {
        return Err(IntentError::rejected(
            "replay_player_locomotion_invalid",
            "grounded locomotion up must follow the canonical radial gravity frame",
        ));
    }
    if let Some(support) = &locomotion.support {
        ensure_finite(support.local_anchor, "replayed support anchor")?;
        ensure_finite(support.local_normal, "replayed support normal")?;
        if player_for_body_id(state, &support.body_id).is_some()
            || !state.physics_collider_exists(&support.body_id, &support.collider_id)
            || (support.local_normal.magnitude() - 1.0).abs() > 1.0e-5
            || support.local_anchor.magnitude() > 1.0e7
        {
            return Err(IntentError::rejected(
                "replay_player_locomotion_invalid",
                "locomotion support must identify one live non-player collider with finite local geometry",
            ));
        }
    }
    Ok(())
}

fn ensure_player_fixed_step_envelope(
    prior_position: Vec3,
    prior_orientation: Quat,
    outcome_position: Vec3,
    outcome_orientation: Quat,
    limits: &SceneConfig,
) -> Result<(), IntentError> {
    let fixed_delta_seconds = f64::from(content::manifest().physics.fixed_delta_seconds);
    let maximum_translation = f64::from(limits.max_linear_velocity_mps) * fixed_delta_seconds
        + PLAYER_POSITION_CORRECTION_BUDGET_M_PER_STEP
        + REPLAY_QUANTIZATION_SLOP;
    if prior_position.squared_distance(outcome_position) > maximum_translation * maximum_translation
    {
        return Err(IntentError::rejected(
            "physics_player_translation_envelope_invalid",
            "native player result exceeds the per-step movement safety envelope",
        ));
    }
    let maximum_rotation = f64::from(limits.max_angular_velocity_radians_per_second)
        * fixed_delta_seconds
        + PLAYER_ROTATION_SLOP_RADIANS_PER_STEP
        + REPLAY_QUANTIZATION_SLOP;
    if quaternion_angular_displacement(prior_orientation, outcome_orientation) > maximum_rotation {
        return Err(IntentError::rejected(
            "physics_player_rotation_envelope_invalid",
            "native player result exceeds the per-step rotation safety envelope",
        ));
    }
    Ok(())
}

fn ensure_dynamic_body_motion_continuity(
    prior_position: Vec3,
    prior_orientation: Quat,
    outcome_position: Vec3,
    outcome_orientation: Quat,
    step_count: u8,
    local_center_of_mass: Vec3,
    limits: &SceneConfig,
) -> Result<(), IntentError> {
    let fixed_delta_seconds = f64::from(content::manifest().physics.fixed_delta_seconds);
    let maximum_translation = f64::from(step_count)
        * (f64::from(limits.max_linear_velocity_mps) * fixed_delta_seconds
            + PLAYER_POSITION_CORRECTION_BUDGET_M_PER_STEP
            + REPLAY_QUANTIZATION_SLOP);
    let prior_center_of_mass = prior_position + prior_orientation.rotate(local_center_of_mass);
    let outcome_center_of_mass =
        outcome_position + outcome_orientation.rotate(local_center_of_mass);
    if prior_center_of_mass.squared_distance(outcome_center_of_mass)
        > maximum_translation * maximum_translation
    {
        return Err(IntentError::rejected(
            "replay_physics_body_translation_invalid",
            "dynamic body translation exceeds the server-enforced fixed-step envelope",
        ));
    }
    let maximum_rotation = f64::from(step_count)
        * (f64::from(limits.max_angular_velocity_radians_per_second) * fixed_delta_seconds
            + PLAYER_ROTATION_SLOP_RADIANS_PER_STEP);
    if quaternion_angular_displacement(prior_orientation, outcome_orientation) > maximum_rotation {
        return Err(IntentError::rejected(
            "replay_physics_body_rotation_continuity_invalid",
            "dynamic body rotation exceeds the server-enforced fixed-step envelope",
        ));
    }
    Ok(())
}

fn ensure_dynamic_body_fixed_step_envelope(
    prior_position: Vec3,
    prior_orientation: Quat,
    outcome_position: Vec3,
    outcome_orientation: Quat,
    local_center_of_mass: Vec3,
    limits: &SceneConfig,
) -> Result<(), IntentError> {
    ensure_dynamic_body_motion_continuity(
        prior_position,
        prior_orientation,
        outcome_position,
        outcome_orientation,
        1,
        local_center_of_mass,
        limits,
    )
    .map_err(|_| {
        IntentError::rejected(
            "physics_dynamic_body_envelope_invalid",
            "native dynamic-body result exceeds the per-step safety envelope",
        )
    })
}

fn grid_local_center_of_mass(state: &WorldState, grid: &Grid) -> Vec3 {
    let mut weighted = Vec3::ZERO;
    let mut total_mass = 0.0;
    for block in grid.blocks.values() {
        let definition = content::block(block.kind);
        let integrity = f64::from(block.health) / f64::from(block.max_health());
        let inventory_mass = block
            .inventory_id
            .as_ref()
            .and_then(|inventory_id| state.inventories.get(inventory_id))
            .map_or(0.0, |inventory| inventory.mass_grams() as f64 / 1_000.0);
        let mass =
            (definition.mass_grams as f64 / 1_000.0 * integrity.max(0.1) + inventory_mass).max(1.0);
        let center = Vec3::new(
            f64::from(block.coordinate.x),
            f64::from(block.coordinate.y),
            f64::from(block.coordinate.z),
        );
        weighted = weighted + center * mass;
        total_mass += mass;
    }
    if total_mass <= f64::EPSILON {
        Vec3::ZERO
    } else {
        weighted * total_mass.recip()
    }
}

fn quaternion_angular_displacement(left: Quat, right: Quat) -> f64 {
    let dot = f64::from(left.x) * f64::from(right.x)
        + f64::from(left.y) * f64::from(right.y)
        + f64::from(left.z) * f64::from(right.z)
        + f64::from(left.w) * f64::from(right.w);
    2.0 * dot.abs().clamp(0.0, 1.0).acos()
}

fn point_unit_cube_distance(point: Vec3, coordinate: IVec3) -> f64 {
    let closest = Vec3::new(
        point
            .x
            .clamp(f64::from(coordinate.x) - 0.5, f64::from(coordinate.x) + 0.5),
        point
            .y
            .clamp(f64::from(coordinate.y) - 0.5, f64::from(coordinate.y) + 0.5),
        point
            .z
            .clamp(f64::from(coordinate.z) - 0.5, f64::from(coordinate.z) + 0.5),
    );
    point.squared_distance(closest).sqrt()
}

fn character_capsule_half_height() -> f64 {
    let character = &content::manifest().character;
    (character.standing_height_m - 2.0 * character.collision_radius_m) * 0.5
}

fn character_capsule_inertia(mass_kg: f64) -> (f64, f64) {
    let radius = content::manifest().character.collision_radius_m;
    let half_cylinder = character_capsule_half_height();
    let cylinder_length = 2.0 * half_cylinder;
    let cylinder_volume = std::f64::consts::PI * radius.powi(2) * cylinder_length;
    let sphere_volume = 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3);
    let cylinder_mass = mass_kg * cylinder_volume / (cylinder_volume + sphere_volume);
    let sphere_mass = mass_kg - cylinder_mass;
    let axis_inertia = 0.5 * cylinder_mass * radius.powi(2) + 0.4 * sphere_mass * radius.powi(2);
    let cylinder_perpendicular_inertia =
        cylinder_mass * (3.0 * radius.powi(2) + cylinder_length.powi(2)) / 12.0;
    let hemisphere_mass = sphere_mass * 0.5;
    let hemisphere_center_offset = half_cylinder + 3.0 * radius / 8.0;
    let hemisphere_centroid_inertia = hemisphere_mass * 83.0 / 320.0 * radius.powi(2);
    let perpendicular_inertia = cylinder_perpendicular_inertia
        + 2.0 * (hemisphere_centroid_inertia + hemisphere_mass * hemisphere_center_offset.powi(2));
    (axis_inertia, perpendicular_inertia)
}

fn point_capsule_axis_distance(
    point: Vec3,
    center: Vec3,
    orientation: Quat,
    half_height: f64,
) -> f64 {
    let axis = orientation.rotate(Vec3::new(0.0, 1.0, 0.0));
    let start = center - axis * half_height;
    let end = center + axis * half_height;
    let segment = end - start;
    let to_point = point - start;
    let length_squared = segment.x * segment.x + segment.y * segment.y + segment.z * segment.z;
    let projection = if length_squared > f64::EPSILON {
        ((to_point.x * segment.x + to_point.y * segment.y + to_point.z * segment.z)
            / length_squared)
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.squared_distance(start + segment * projection).sqrt()
}

fn dot(left: Vec3, right: Vec3) -> f64 {
    left.x * right.x + left.y * right.y + left.z * right.z
}

fn cross(left: Vec3, right: Vec3) -> Vec3 {
    Vec3::new(
        left.y * right.z - left.z * right.y,
        left.z * right.x - left.x * right.z,
        left.x * right.y - left.y * right.x,
    )
}

fn normalized_or(value: Vec3, fallback: Vec3) -> Vec3 {
    let magnitude = value.magnitude();
    if magnitude > 1.0e-9 && magnitude.is_finite() {
        value * (1.0 / magnitude)
    } else {
        let fallback_magnitude = fallback.magnitude();
        if fallback_magnitude > 1.0e-9 && fallback_magnitude.is_finite() {
            fallback * (1.0 / fallback_magnitude)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        }
    }
}

fn canonical_vec3(value: Vec3) -> Vec3 {
    Vec3::new(
        canonical_f64(value.x),
        canonical_f64(value.y),
        canonical_f64(value.z),
    )
}

fn canonical_f64(value: f64) -> f64 {
    let quantized = quantize_f64(value);
    if quantized == 0.0 { 0.0 } else { quantized }
}

fn movement_samples(start: Vec3, end: Vec3) -> Vec<Vec3> {
    const SAMPLE_SPACING_M: f64 = 0.2;
    let delta = end - start;
    let distance = delta.magnitude();
    let mut steps = 1_u32;
    while f64::from(steps) * SAMPLE_SPACING_M < distance {
        steps += 1;
    }
    (1..=steps)
        .map(|step| start + delta * (f64::from(step) / f64::from(steps)))
        .collect()
}

fn physics_scene_config() -> SceneConfig {
    SceneConfig {
        fixed_delta_seconds: content::manifest().physics.fixed_delta_seconds,
        collision_substeps: content::manifest().physics.collision_substeps,
        max_colliders_per_body: 8_192,
        max_linear_velocity_mps: 32.0,
        max_angular_velocity_radians_per_second: 8.0,
        ..SceneConfig::default()
    }
}

fn event_changes_physics_scene(payload: &EventPayload) -> bool {
    !matches!(
        payload,
        EventPayload::PlayerControlSet { .. }
            | EventPayload::SuitModeChanged { .. }
            | EventPayload::SuitOxygenChanged { .. }
            | EventPayload::ProductionQueued { .. }
            | EventPayload::ProductionQuantumCommitted { .. }
            | EventPayload::GridControlSet { .. }
            | EventPayload::PhysicsStepCommitted { .. }
    )
}

fn production_occurrence_event_id(occurrence: &ProductionScheduleOccurrence) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"the-verse/production-occurrence/v1\0");
    hasher.update(
        &serde_json::to_vec(occurrence).expect("production occurrence serialization cannot fail"),
    );
    hasher.finalize().to_hex().to_string()
}

fn voxel_collision_chunk_edge_cells() -> i32 {
    i32::from(content::manifest().physics.voxel_collision_chunk_edge_cells)
}

fn voxel_collision_chunk_cell_count() -> usize {
    usize::from(content::manifest().physics.voxel_collision_chunk_edge_cells).pow(3)
}

fn voxel_collision_chunk_coordinate(coordinate: IVec3) -> IVec3 {
    let edge = voxel_collision_chunk_edge_cells();
    IVec3::new(
        coordinate.x.div_euclid(edge),
        coordinate.y.div_euclid(edge),
        coordinate.z.div_euclid(edge),
    )
}

fn voxel_collision_chunk_origin(chunk: IVec3) -> IVec3 {
    let edge = voxel_collision_chunk_edge_cells();
    IVec3::new(chunk.x * edge, chunk.y * edge, chunk.z * edge)
}

fn voxel_collision_chunk_body_id(chunk: IVec3) -> String {
    format!(
        "voxel-chunk-{x}-{y}-{z}",
        x = chunk.x,
        y = chunk.y,
        z = chunk.z
    )
}

fn voxel_collision_chunk_coordinates(chunk: IVec3) -> Vec<IVec3> {
    let origin = voxel_collision_chunk_origin(chunk);
    let edge = voxel_collision_chunk_edge_cells();
    let mut coordinates = Vec::with_capacity(voxel_collision_chunk_cell_count());
    for local_x in 0..edge {
        for local_y in 0..edge {
            for local_z in 0..edge {
                coordinates.push(IVec3::new(
                    origin.x + local_x,
                    origin.y + local_y,
                    origin.z + local_z,
                ));
            }
        }
    }
    coordinates
}

fn voxel_collision_collider_id(coordinate: IVec3) -> String {
    format!(
        "voxel-{x}-{y}-{z}",
        x = coordinate.x,
        y = coordinate.y,
        z = coordinate.z
    )
}

fn voxel_collision_chunk_body_spec(state: &WorldState, chunk: IVec3) -> Option<BodySpec> {
    let origin = voxel_collision_chunk_origin(chunk);
    let mut colliders = Vec::with_capacity(voxel_collision_chunk_cell_count());
    for coordinate in voxel_collision_chunk_coordinates(chunk) {
        if state.voxels.occupied.contains(&coordinate) {
            colliders.push(BoxColliderSpec {
                collider_id: voxel_collision_collider_id(coordinate),
                local_pose: PhysicsPose::new(
                    PhysicsVec3::new(
                        f64::from(coordinate.x - origin.x),
                        f64::from(coordinate.y - origin.y),
                        f64::from(coordinate.z - origin.z),
                    ),
                    PhysicsQuat::IDENTITY,
                ),
                half_extents: PhysicsVec3::new(0.5, 0.5, 0.5),
                density_kg_per_m3: 2_600.0,
            });
        }
    }
    if colliders.is_empty() {
        return None;
    }
    let physics = &content::manifest().physics;
    let mut body = BodySpec::static_body(
        voxel_collision_chunk_body_id(chunk),
        PhysicsPose::new(
            PhysicsVec3::new(
                f64::from(origin.x),
                f64::from(origin.y),
                f64::from(origin.z),
            ),
            PhysicsQuat::IDENTITY,
        ),
        colliders,
    );
    body.friction = physics.friction;
    body.restitution = physics.restitution;
    Some(body)
}

fn voxel_collision_body_specs(state: &WorldState) -> Vec<BodySpec> {
    let chunks = state
        .voxels
        .occupied
        .iter()
        .copied()
        .map(voxel_collision_chunk_coordinate)
        .collect::<BTreeSet<_>>();
    chunks
        .into_iter()
        .filter_map(|chunk| voxel_collision_chunk_body_spec(state, chunk))
        .collect()
}

fn physics_body_specs(state: &WorldState) -> Vec<BodySpec> {
    let physics = &content::manifest().physics;
    let mut bodies = voxel_collision_body_specs(state);
    let mut planet = BodySpec::static_body(
        PLANET_BODY_ID,
        PhysicsPose::new(to_physics_vec3(planet_center()), PhysicsQuat::IDENTITY),
        Vec::new(),
    );
    planet.sphere_colliders.push(SphereColliderSpec {
        collider_id: PLANET_COLLIDER_ID.into(),
        local_pose: PhysicsPose::IDENTITY,
        radius: planet_surface_radius_m() as f32,
        density_kg_per_m3: 5_500.0,
    });
    planet.friction = physics.friction;
    planet.restitution = physics.restitution;
    bodies.push(planet);
    for (_, canonical_player) in state.player.iter().filter(|(player_id, player)| {
        !state.player_transfer_locks.contains_key(*player_id)
            && matches!(player.life_state, PlayerLifeState::Alive)
    }) {
        let character = &content::manifest().character;
        let radius = character.collision_radius_m;
        let half_height_of_cylinder = (character.standing_height_m - 2.0 * radius) * 0.5;
        let volume = std::f64::consts::PI * radius.powi(2) * (2.0 * half_height_of_cylinder)
            + 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3);
        let mut player = BodySpec::dynamic(
            player_body_id(&canonical_player.player_id),
            PhysicsPose::new(
                to_physics_vec3(canonical_player.position),
                to_physics_quat(canonical_player.orientation),
            ),
            Vec::new(),
        );
        player.capsule_colliders.push(CapsuleColliderSpec {
            collider_id: player_collider_id(&canonical_player.player_id),
            local_pose: PhysicsPose::IDENTITY,
            radius: radius as f32,
            half_height_of_cylinder: half_height_of_cylinder as f32,
            density_kg_per_m3: (character.mass_kg / volume) as f32,
        });
        player.linear_velocity = to_physics_vec3(canonical_player.linear_velocity);
        player.angular_velocity = to_physics_vec3(canonical_player.angular_velocity);
        player.friction = physics.friction;
        player.restitution = physics.restitution;
        player.allow_sleeping = false;
        player.collision_class = BodyCollisionClass::Character;
        player.inertia_multiplier = CHARACTER_INERTIA_MULTIPLIER as f32;
        player.motion_quality = MotionQuality::LinearCast;
        bodies.push(player);
    }
    bodies.reserve(state.grids.len());
    bodies.extend(state.grids.values().map(|grid| {
        let colliders = grid
            .blocks
            .values()
            .map(|block| {
                let definition = content::block(block.kind);
                let integrity = f32::from(block.health) / f32::from(block.max_health());
                let inventory_mass = block
                    .inventory_id
                    .as_ref()
                    .and_then(|inventory_id| state.inventories.get(inventory_id))
                    .map_or(0.0, |inventory| inventory.mass_grams() as f32 / 1_000.0);
                BoxColliderSpec {
                    collider_id: block.block_id.clone(),
                    local_pose: PhysicsPose::new(
                        PhysicsVec3::new(
                            f64::from(block.coordinate.x),
                            f64::from(block.coordinate.y),
                            f64::from(block.coordinate.z),
                        ),
                        PhysicsQuat::IDENTITY,
                    ),
                    half_extents: PhysicsVec3::new(0.5, 0.5, 0.5),
                    density_kg_per_m3: (definition.mass_grams as f32 / 1_000.0
                        * integrity.max(0.1)
                        + inventory_mass)
                        .max(1.0),
                }
            })
            .collect();
        let pose = PhysicsPose::new(
            to_physics_vec3(grid.position),
            to_physics_quat(grid.orientation),
        );
        let mut body = if grid.anchored {
            BodySpec::static_body(grid.grid_id.clone(), pose, colliders)
        } else {
            BodySpec::dynamic(grid.grid_id.clone(), pose, colliders)
        };
        body.linear_velocity = to_physics_vec3(grid.linear_velocity);
        body.angular_velocity = to_physics_vec3(grid.angular_velocity);
        body.friction = physics.friction;
        body.restitution = physics.restitution;
        body
    }));
    bodies
}

fn advance_player_control_for_substep(player: &mut Player, simulation_tick: u64) {
    while player
        .pending_control_frames
        .front()
        .is_some_and(|frame| simulation_tick >= frame.expires_at_simulation_tick)
    {
        let expired = player
            .pending_control_frames
            .pop_front()
            .expect("checked pending character control exists");
        player.last_processed_input_sequence = expired.input_sequence;
    }

    if let Some(frame) = player.pending_control_frames.pop_front() {
        let jump_pressed = frame.jump && !player.locomotion.jump_held;
        player.last_processed_input_sequence = frame.input_sequence;
        player.control_linear_input = frame.linear_input;
        player.control_angular_input = frame.angular_input;
        player.boost = frame.boost;
        player.dampeners = frame.dampeners;
        player.jump = frame.jump;
        player.locomotion.jump_held = frame.jump;
        if jump_pressed {
            player.locomotion.jump_buffer_expires_at_simulation_tick =
                simulation_tick.saturating_add(content::manifest().character.jump_buffer_ticks);
        }
        player.control_expires_at_simulation_tick = frame.expires_at_simulation_tick;
    }

    if simulation_tick >= player.control_expires_at_simulation_tick {
        player.control_linear_input = Vec3::ZERO;
        player.control_angular_input = Vec3::ZERO;
        player.boost = false;
        player.dampeners = true;
        player.jump = false;
        player.locomotion.jump_held = false;
    }
}

fn reset_locomotion(
    position: Vec3,
    kind: LocomotionKind,
    magnetic_boots_enabled: bool,
    simulation_tick: u64,
) -> PlayerLocomotionSnapshot {
    PlayerLocomotionSnapshot {
        kind,
        up: radial_up(position),
        view_pitch_radians: 0.0,
        support: None,
        jump_held: false,
        jump_buffer_expires_at_simulation_tick: simulation_tick,
        support_grace_expires_at_simulation_tick: simulation_tick,
        magnetic_boots_enabled,
        magnetic_reattach_after_simulation_tick: simulation_tick,
    }
}

#[derive(Debug, Clone, Copy)]
struct PlayerJumpLaunch {
    up: Vec3,
    support_velocity: Vec3,
}

fn classify_player_locomotion_for_substep(
    state: &WorldState,
    physics_scene: &Scene,
    player: &mut Player,
    body_states: &[verse_physics::BodyState],
    simulation_tick: u64,
) -> Result<Option<PlayerJumpLaunch>, PhysicsError> {
    if !matches!(player.life_state, PlayerLifeState::Alive) {
        return Ok(None);
    }
    let player_body_id = player_body_id(&player.player_id);
    let Some(body) = body_states
        .iter()
        .find(|body| body.body_id == player_body_id)
    else {
        return Ok(None);
    };
    let position = from_physics_vec3(body.pose.position);
    if player.jetpack_enabled {
        player.locomotion.kind = LocomotionKind::Eva;
        player.locomotion.support = None;
        player.locomotion.up = canonical_vec3(radial_up(position));
        return Ok(None);
    }

    let character = &content::manifest().character;
    let maximum_pitch = character.maximum_view_pitch_degrees.to_radians();
    player.locomotion.view_pitch_radians = canonical_f64(
        (player.locomotion.view_pitch_radians
            + player.control_angular_input.x
                * character.maximum_angular_speed_radians_per_second
                * f64::from(content::manifest().physics.fixed_delta_seconds))
        .clamp(-maximum_pitch, maximum_pitch),
    );
    let body_orientation = from_physics_quat(body.pose.rotation);
    let body_up = normalized_or(
        body_orientation.rotate(Vec3::new(0.0, 1.0, 0.0)),
        player.locomotion.up,
    );
    let environment = state.environment_at(position);
    let gravity_up = radial_up(position);
    let magnetic_probe = player.locomotion.magnetic_boots_enabled
        && simulation_tick >= player.locomotion.magnetic_reattach_after_simulation_tick;
    let probe_up = if matches!(player.locomotion.kind, LocomotionKind::Magnetic) || magnetic_probe {
        body_up
    } else {
        gravity_up
    };
    let probe_distance = if magnetic_probe {
        character
            .support_probe_distance_m
            .max(character.magnetic_probe_distance_m)
    } else {
        character.support_probe_distance_m
    };
    let hit = physics_scene.cast_capsule(&CapsuleCast {
        pose: body.pose,
        radius: character.collision_radius_m as f32,
        half_height_of_cylinder: character_capsule_half_height() as f32,
        displacement: to_physics_vec3(probe_up * -probe_distance),
        collision_class: BodyCollisionClass::Character,
        ignore_body_id: Some(player_body_id),
    })?;
    let prior_had_support = player.locomotion.support.is_some();
    let mut accepted_support = None;
    let mut support_kind = LocomotionKind::Airborne;
    let mut support_up = gravity_up;
    if let Some(hit) = hit {
        let surface_normal = normalized_or(from_physics_vec3(hit.surface_normal), probe_up);
        let same_grounded_support = matches!(player.locomotion.kind, LocomotionKind::Grounded)
            && player.locomotion.support.as_ref().is_some_and(|support| {
                support.body_id == hit.body_id && support.collider_id == hit.collider_id
            });
        let gravity_walkable = probe_distance * hit.fraction <= character.ground_snap_m + 0.002
            && gravity_support_is_walkable(
                environment.gravity_m_s2,
                surface_normal,
                gravity_up,
                same_grounded_support,
            );
        let relative_normal_speed = support_relative_normal_speed(
            state,
            body_states,
            &hit.body_id,
            from_physics_vec3(hit.point_on_body),
            from_physics_vec3(body.linear_velocity),
            surface_normal,
        );
        let same_magnetic_support = matches!(player.locomotion.kind, LocomotionKind::Magnetic)
            && player.locomotion.support.as_ref().is_some_and(|support| {
                support.body_id == hit.body_id && support.collider_id == hit.collider_id
            });
        let magnetic_walkable = magnetic_probe
            && magnetic_support_is_eligible(state, &hit.body_id, &hit.collider_id)
            && (same_magnetic_support
                || relative_normal_speed <= character.magnetic_catch_speed_m_s);
        if gravity_walkable || magnetic_walkable {
            support_kind = if gravity_walkable {
                LocomotionKind::Grounded
            } else {
                LocomotionKind::Magnetic
            };
            support_up = if gravity_walkable {
                gravity_up
            } else {
                surface_normal
            };
            accepted_support = Some(locomotion_support_snapshot(
                state,
                body_states,
                &hit.body_id,
                &hit.collider_id,
                from_physics_vec3(hit.point_on_body),
                surface_normal,
            ));
        }
    }

    let jump_requested = player.jump
        && player.locomotion.jump_buffer_expires_at_simulation_tick >= simulation_tick
        && (accepted_support.is_some()
            || player.locomotion.support_grace_expires_at_simulation_tick >= simulation_tick);
    if jump_requested {
        let support_velocity = accepted_support.as_ref().map_or(Vec3::ZERO, |support| {
            let anchor = support_world_anchor(state, body_states, support);
            support_point_velocity(state, body_states, &support.body_id, anchor)
        });
        if matches!(player.locomotion.kind, LocomotionKind::Magnetic) {
            player.locomotion.magnetic_reattach_after_simulation_tick =
                simulation_tick.saturating_add(character.magnetic_reattach_lockout_ticks);
        }
        player.locomotion.kind = LocomotionKind::Airborne;
        player.locomotion.support = None;
        player.locomotion.up = canonical_vec3(support_up);
        player.locomotion.jump_buffer_expires_at_simulation_tick = simulation_tick;
        player.locomotion.support_grace_expires_at_simulation_tick = simulation_tick;
        return Ok(Some(PlayerJumpLaunch {
            up: support_up,
            support_velocity,
        }));
    }

    if let Some(support) = accepted_support {
        player.locomotion.kind = support_kind;
        player.locomotion.support = Some(support);
        player.locomotion.up = canonical_vec3(support_up);
        player.locomotion.support_grace_expires_at_simulation_tick =
            simulation_tick.saturating_add(character.coyote_ticks);
    } else {
        if prior_had_support {
            player.locomotion.support_grace_expires_at_simulation_tick =
                simulation_tick.saturating_add(character.coyote_ticks);
        }
        player.locomotion.kind = LocomotionKind::Airborne;
        player.locomotion.support = None;
        player.locomotion.up = canonical_vec3(gravity_up);
    }
    Ok(None)
}

fn adjust_grounded_capsule_for_substep(
    state: &WorldState,
    physics_scene: &mut Scene,
    player: &Player,
    body_states: &mut [verse_physics::BodyState],
    simulation_tick: u64,
) -> Result<(), PhysicsError> {
    if !matches!(player.life_state, PlayerLifeState::Alive)
        || player.jetpack_enabled
        || !matches!(player.locomotion.kind, LocomotionKind::Grounded)
    {
        return Ok(());
    }
    let player_body_id = player_body_id(&player.player_id);
    let Some(body_index) = body_states
        .iter()
        .position(|body| body.body_id == player_body_id)
    else {
        return Ok(());
    };
    let character = &content::manifest().character;
    let up = normalized_or(
        player.locomotion.up,
        radial_up(from_physics_vec3(body_states[body_index].pose.position)),
    );
    let lease_active = simulation_tick < player.control_expires_at_simulation_tick;
    let local_walk_input = if lease_active {
        Vec3::new(
            player.control_linear_input.x,
            0.0,
            player.control_linear_input.z,
        )
    } else {
        Vec3::ZERO
    };
    if local_walk_input.magnitude() > CONTROL_INPUT_EPSILON {
        let body_pose = body_states[body_index].pose;
        let orientation = from_physics_quat(body_pose.rotation);
        let raw_forward = orientation.rotate(local_walk_input);
        let tangent_forward = raw_forward - up * dot(raw_forward, up);
        if tangent_forward.magnitude() > CONTROL_INPUT_EPSILON {
            let selected_speed = if player.boost {
                character.sprint_speed_m_s
            } else {
                character.walk_speed_m_s
            };
            let forward_displacement = tangent_forward
                * (1.0 / tangent_forward.magnitude())
                * local_walk_input.magnitude().min(1.0)
                * selected_speed
                * f64::from(content::manifest().physics.fixed_delta_seconds);
            if let Some(step_translation) = grounded_step_translation(
                physics_scene,
                &player_body_id,
                body_pose,
                up,
                forward_displacement,
            )? {
                physics_scene
                    .translate_dynamic_body(&player_body_id, to_physics_vec3(step_translation))?;
                body_states[body_index].pose.position =
                    body_states[body_index].pose.position + to_physics_vec3(step_translation);
            }
        }
    }

    let body_pose = body_states[body_index].pose;
    let snap_displacement = up * -character.ground_snap_m;
    if let Some(hit) = physics_scene.cast_capsule(&CapsuleCast {
        pose: body_pose,
        radius: character.collision_radius_m as f32,
        half_height_of_cylinder: character_capsule_half_height() as f32,
        displacement: to_physics_vec3(snap_displacement),
        collision_class: BodyCollisionClass::Character,
        ignore_body_id: Some(player_body_id.clone()),
    })? {
        let surface_normal = normalized_or(from_physics_vec3(hit.surface_normal), up);
        let same_support = player.locomotion.support.as_ref().is_some_and(|support| {
            support.body_id == hit.body_id && support.collider_id == hit.collider_id
        });
        let environment = state.environment_at(from_physics_vec3(body_pose.position));
        let support_velocity = support_point_velocity(
            state,
            body_states,
            &hit.body_id,
            from_physics_vec3(hit.point_on_body),
        );
        let separating_speed = dot(
            from_physics_vec3(body_states[body_index].linear_velocity) - support_velocity,
            surface_normal,
        );
        if gravity_support_is_walkable(
            environment.gravity_m_s2,
            surface_normal,
            radial_up(from_physics_vec3(body_pose.position)),
            same_support,
        ) && separating_speed <= 0.05
        {
            const SNAP_SKIN_M: f64 = 0.002;
            let snap_distance = (character.ground_snap_m * hit.fraction - SNAP_SKIN_M).max(0.0);
            if snap_distance > f64::EPSILON {
                let translation = up * -snap_distance;
                physics_scene
                    .translate_dynamic_body(&player_body_id, to_physics_vec3(translation))?;
                body_states[body_index].pose.position =
                    body_states[body_index].pose.position + to_physics_vec3(translation);
            }
        }
    }
    Ok(())
}

fn grounded_step_translation(
    physics_scene: &Scene,
    player_body_id: &str,
    body_pose: PhysicsPose,
    up: Vec3,
    forward_displacement: Vec3,
) -> Result<Option<Vec3>, PhysicsError> {
    const STEP_SKIN_M: f64 = 0.006;
    let character = &content::manifest().character;
    let capsule = |pose: PhysicsPose, displacement: Vec3| CapsuleCast {
        pose,
        radius: character.collision_radius_m as f32,
        half_height_of_cylinder: character_capsule_half_height() as f32,
        displacement: to_physics_vec3(displacement),
        collision_class: BodyCollisionClass::Character,
        ignore_body_id: Some(player_body_id.into()),
    };
    let obstruction_pose = PhysicsPose::new(
        body_pose.position + to_physics_vec3(up * STEP_SKIN_M),
        body_pose.rotation,
    );
    let Some(obstruction) =
        physics_scene.cast_capsule(&capsule(obstruction_pose, forward_displacement))?
    else {
        return Ok(None);
    };
    let obstruction_normal = normalized_or(from_physics_vec3(obstruction.surface_normal), up);
    if dot(obstruction_normal, up) >= character.walkable_slope_degrees.to_radians().cos() {
        return Ok(None);
    }

    let raised_forward_pose = PhysicsPose::new(
        body_pose.position + to_physics_vec3(up * character.step_height_m + forward_displacement),
        body_pose.rotation,
    );
    let landing_distance = character.step_height_m + character.ground_snap_m;
    let Some(landing) =
        physics_scene.cast_capsule(&capsule(raised_forward_pose, up * -landing_distance))?
    else {
        return Ok(None);
    };
    let landing_normal = normalized_or(from_physics_vec3(landing.surface_normal), up);
    if dot(landing_normal, up) < character.walkable_slope_degrees.to_radians().cos() {
        return Ok(None);
    }
    let landing_lift = character.step_height_m - landing_distance * landing.fraction;
    if landing_lift <= STEP_SKIN_M || landing_lift > character.step_height_m {
        return Ok(None);
    }
    let vertical_displacement = up * (landing_lift + STEP_SKIN_M).min(character.step_height_m);
    if physics_scene
        .cast_capsule(&capsule(body_pose, vertical_displacement))?
        .is_some()
    {
        return Ok(None);
    }
    let lifted_pose = PhysicsPose::new(
        body_pose.position + to_physics_vec3(vertical_displacement),
        body_pose.rotation,
    );
    if physics_scene
        .cast_capsule(&capsule(lifted_pose, forward_displacement))?
        .is_some()
    {
        return Ok(None);
    }
    Ok(Some(vertical_displacement + forward_displacement))
}

fn gravity_support_is_walkable(
    gravity_m_s2: f64,
    surface_normal: Vec3,
    gravity_up: Vec3,
    retaining_same_support: bool,
) -> bool {
    if gravity_m_s2 <= 0.05 {
        return false;
    }
    let character = &content::manifest().character;
    let exit_margin = if retaining_same_support {
        character.slope_exit_hysteresis_degrees
    } else {
        0.0
    };
    let maximum_slope_degrees = (character.walkable_slope_degrees + exit_margin).min(89.0);
    dot(surface_normal, gravity_up) >= maximum_slope_degrees.to_radians().cos()
}

fn magnetic_support_is_eligible(state: &WorldState, body_id: &str, collider_id: &str) -> bool {
    state
        .grids
        .get(body_id)
        .and_then(|grid| grid.blocks.get(collider_id))
        .is_some_and(Block::is_complete)
}

fn locomotion_support_snapshot(
    state: &WorldState,
    body_states: &[verse_physics::BodyState],
    body_id: &str,
    collider_id: &str,
    world_anchor: Vec3,
    world_normal: Vec3,
) -> LocomotionSupportSnapshot {
    let pose = support_body_pose(state, body_states, body_id);
    let (local_anchor, local_normal) = pose.map_or((world_anchor, world_normal), |pose| {
        let orientation = from_physics_quat(pose.rotation);
        (
            orientation
                .conjugate()
                .rotate(world_anchor - from_physics_vec3(pose.position)),
            normalized_or(orientation.conjugate().rotate(world_normal), world_normal),
        )
    });
    LocomotionSupportSnapshot {
        body_id: body_id.into(),
        collider_id: collider_id.into(),
        local_anchor: canonical_vec3(local_anchor),
        local_normal: canonical_vec3(local_normal),
    }
}

fn support_body_pose(
    state: &WorldState,
    body_states: &[verse_physics::BodyState],
    body_id: &str,
) -> Option<PhysicsPose> {
    body_states
        .iter()
        .find(|body| body.body_id == body_id)
        .map(|body| body.pose)
        .or_else(|| {
            state.grids.get(body_id).map(|grid| {
                PhysicsPose::new(
                    to_physics_vec3(grid.position),
                    to_physics_quat(grid.orientation),
                )
            })
        })
        .or_else(|| {
            (body_id == PLANET_BODY_ID).then_some(PhysicsPose::new(
                to_physics_vec3(planet_center()),
                PhysicsQuat::IDENTITY,
            ))
        })
}

fn support_relative_normal_speed(
    state: &WorldState,
    body_states: &[verse_physics::BodyState],
    body_id: &str,
    world_anchor: Vec3,
    player_velocity: Vec3,
    surface_normal: Vec3,
) -> f64 {
    let support_velocity = support_point_velocity(state, body_states, body_id, world_anchor);
    dot(player_velocity - support_velocity, surface_normal).abs()
}

fn support_point_velocity(
    state: &WorldState,
    body_states: &[verse_physics::BodyState],
    body_id: &str,
    world_anchor: Vec3,
) -> Vec3 {
    let Some(body) = body_states.iter().find(|body| body.body_id == body_id) else {
        return Vec3::ZERO;
    };
    let linear_velocity = from_physics_vec3(body.linear_velocity);
    let angular_velocity = from_physics_vec3(body.angular_velocity);
    let pose_position = from_physics_vec3(body.pose.position);
    let center_of_mass = state.grids.get(body_id).map_or(pose_position, |grid| {
        pose_position
            + from_physics_quat(body.pose.rotation).rotate(grid_local_center_of_mass(state, grid))
    });
    linear_velocity + cross(angular_velocity, world_anchor - center_of_mass)
}

fn support_world_anchor(
    state: &WorldState,
    body_states: &[verse_physics::BodyState],
    support: &LocomotionSupportSnapshot,
) -> Vec3 {
    support_body_pose(state, body_states, &support.body_id).map_or(support.local_anchor, |pose| {
        from_physics_vec3(pose.position)
            + from_physics_quat(pose.rotation).rotate(support.local_anchor)
    })
}

fn physics_controls(
    state: &WorldState,
    player: &Player,
    body_states: &[verse_physics::BodyState],
    simulation_tick: u64,
    player_jump: Option<PlayerJumpLaunch>,
    include_grid_controls: bool,
) -> Vec<BodyControl> {
    let physics = &content::manifest().physics;
    let mut controls = if include_grid_controls {
        state
            .grids
            .values()
            .filter(|grid| !grid.anchored)
            .map(|grid| {
                let body_state = body_states.iter().find(|body| body.body_id == grid.grid_id);
                let linear_velocity = body_state.map_or(grid.linear_velocity, |body| {
                    from_physics_vec3(body.linear_velocity)
                });
                let angular_velocity = body_state.map_or(grid.angular_velocity, |body| {
                    from_physics_vec3(body.angular_velocity)
                });
                let online = grid.power().online;
                let user_force = if online {
                    grid.orientation.rotate(grid.control_linear_input)
                        * physics.control_force_newtons
                } else {
                    Vec3::ZERO
                };
                let user_torque = if online {
                    grid.orientation.rotate(grid.control_angular_input)
                        * physics.control_torque_newton_meters
                } else {
                    Vec3::ZERO
                };
                let dampener_force = if online && grid.dampeners {
                    linear_velocity * -physics.linear_dampener_newtons_per_mps
                } else {
                    Vec3::ZERO
                };
                let dampener_torque = if online && grid.dampeners {
                    angular_velocity * -physics.angular_dampener_newton_meters_per_radian
                } else {
                    Vec3::ZERO
                };
                BodyControl {
                    body_id: grid.grid_id.clone(),
                    force_newtons: to_physics_vec3(user_force + dampener_force),
                    torque_newton_meters: to_physics_vec3(user_torque + dampener_torque),
                }
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    if matches!(player.life_state, PlayerLifeState::Alive) {
        let character = &content::manifest().character;
        let player_body_id = player_body_id(&player.player_id);
        let body_state = body_states
            .iter()
            .find(|body| body.body_id == player_body_id);
        let position = body_state.map_or(player.position, |body| {
            from_physics_vec3(body.pose.position)
        });
        let orientation = body_state.map_or(player.orientation, |body| {
            from_physics_quat(body.pose.rotation)
        });
        let linear_velocity = body_state.map_or(player.linear_velocity, |body| {
            from_physics_vec3(body.linear_velocity)
        });
        let angular_velocity = body_state.map_or(player.angular_velocity, |body| {
            from_physics_vec3(body.angular_velocity)
        });
        let lease_active = simulation_tick < player.control_expires_at_simulation_tick;
        let linear_input = if lease_active {
            player.control_linear_input
        } else {
            Vec3::ZERO
        };
        let angular_input = if lease_active {
            player.control_angular_input
        } else {
            Vec3::ZERO
        };
        let dampeners = player.dampeners || !lease_active;
        let boost = player.boost && lease_active;
        let gravity = state.environment_at(position).gravity;
        let mut acceleration = gravity;
        if player.jetpack_enabled {
            let world_input = orientation.rotate(linear_input);
            if dampeners {
                let maximum_speed = if boost {
                    character.boost_maximum_speed_m_s
                } else {
                    character.maximum_speed_m_s
                };
                let target_velocity = world_input * maximum_speed;
                let delta_seconds = f64::from(physics.fixed_delta_seconds);
                let maximum_acceleration = if world_input.magnitude() > CONTROL_INPUT_EPSILON {
                    if boost {
                        character.boost_acceleration_m_s2
                    } else {
                        character.thrust_acceleration_m_s2
                    }
                } else {
                    character.linear_dampener_acceleration_m_s2
                };
                acceleration = ((target_velocity - linear_velocity) * (1.0 / delta_seconds))
                    .clamped(maximum_acceleration);
            } else {
                let thrust = if boost {
                    character.boost_acceleration_m_s2
                } else {
                    character.thrust_acceleration_m_s2
                };
                let delta_seconds = f64::from(physics.fixed_delta_seconds);
                let selected_maximum_speed = if boost {
                    character.boost_maximum_speed_m_s
                } else {
                    character.maximum_speed_m_s
                };
                let gravity_velocity = linear_velocity + gravity * delta_seconds;
                let bounded_velocity = if world_input.magnitude() > CONTROL_INPUT_EPSILON {
                    let ceiling = gravity_velocity.magnitude().max(selected_maximum_speed);
                    (gravity_velocity + world_input * thrust * delta_seconds).clamped(ceiling)
                } else {
                    gravity_velocity
                };
                acceleration = (bounded_velocity - linear_velocity) * (1.0 / delta_seconds);
            }
        } else if matches!(
            player.locomotion.kind,
            LocomotionKind::Grounded | LocomotionKind::Magnetic
        ) {
            let up = normalized_or(player.locomotion.up, radial_up(position));
            let local_walk_input = Vec3::new(linear_input.x, 0.0, linear_input.z);
            let raw_world_direction = orientation.rotate(local_walk_input);
            let tangent_direction = raw_world_direction - up * dot(raw_world_direction, up);
            let tangent_input = if tangent_direction.magnitude() > CONTROL_INPUT_EPSILON {
                tangent_direction
                    * (1.0 / tangent_direction.magnitude())
                    * local_walk_input.magnitude().min(1.0)
            } else {
                Vec3::ZERO
            };
            let support_velocity =
                player
                    .locomotion
                    .support
                    .as_ref()
                    .map_or(Vec3::ZERO, |support| {
                        let anchor = support_world_anchor(state, body_states, support);
                        support_point_velocity(state, body_states, &support.body_id, anchor)
                    });
            let relative_velocity = linear_velocity - support_velocity;
            let relative_tangent_velocity = relative_velocity - up * dot(relative_velocity, up);
            let selected_speed = if boost {
                character.sprint_speed_m_s
            } else {
                character.walk_speed_m_s
            };
            let target_tangent_velocity = tangent_input * selected_speed;
            let motor_limit = if tangent_input.magnitude() > CONTROL_INPUT_EPSILON {
                character.ground_acceleration_m_s2
            } else {
                character.ground_braking_m_s2
            };
            let delta_seconds = f64::from(physics.fixed_delta_seconds);
            let tangent_acceleration = ((target_tangent_velocity - relative_tangent_velocity)
                * (1.0 / delta_seconds))
                .clamped(motor_limit);
            acceleration = tangent_acceleration;
            if matches!(player.locomotion.kind, LocomotionKind::Magnetic) {
                acceleration = acceleration - up * character.magnetic_adhesion_acceleration_m_s2;
            } else {
                let relative_normal_speed = dot(relative_velocity, up);
                let normal_acceleration = (-relative_normal_speed / delta_seconds).clamp(
                    -character.ground_braking_m_s2,
                    character.ground_braking_m_s2,
                );
                acceleration = acceleration + up * normal_acceleration;
            }
        }
        if let Some(jump) = player_jump {
            let delta_seconds = f64::from(physics.fixed_delta_seconds);
            let relative_normal_speed = dot(linear_velocity - jump.support_velocity, jump.up);
            acceleration = acceleration
                + jump.up
                    * ((character.jump_speed_m_s - relative_normal_speed) / delta_seconds).max(0.0);
        }
        let angular_acceleration = if player.jetpack_enabled {
            let world_angular_input = orientation.rotate(angular_input);
            if dampeners {
                let target =
                    world_angular_input * character.maximum_angular_speed_radians_per_second;
                ((target - angular_velocity) * (1.0 / f64::from(physics.fixed_delta_seconds)))
                    .clamped(if world_angular_input.magnitude() > CONTROL_INPUT_EPSILON {
                        character.angular_acceleration_radians_per_second_squared
                    } else {
                        character.angular_dampener_acceleration_radians_per_second_squared
                    })
            } else {
                let delta_seconds = f64::from(physics.fixed_delta_seconds);
                let bounded_velocity = if world_angular_input.magnitude() > CONTROL_INPUT_EPSILON {
                    let ceiling = angular_velocity
                        .magnitude()
                        .max(character.maximum_angular_speed_radians_per_second);
                    (angular_velocity
                        + world_angular_input
                            * character.angular_acceleration_radians_per_second_squared
                            * delta_seconds)
                        .clamped(ceiling)
                } else {
                    angular_velocity
                };
                (bounded_velocity - angular_velocity) * (1.0 / delta_seconds)
            }
        } else {
            let desired_up = normalized_or(player.locomotion.up, radial_up(position));
            let current_up =
                normalized_or(orientation.rotate(Vec3::new(0.0, 1.0, 0.0)), desired_up);
            let mut upright_axis = cross(current_up, desired_up);
            if upright_axis.magnitude() <= CONTROL_INPUT_EPSILON
                && dot(current_up, desired_up) < 0.0
            {
                upright_axis = normalized_or(
                    orientation.rotate(Vec3::new(1.0, 0.0, 0.0)),
                    Vec3::new(1.0, 0.0, 0.0),
                );
            }
            let target =
                (desired_up * angular_input.y * character.maximum_angular_speed_radians_per_second
                    + upright_axis * character.maximum_angular_speed_radians_per_second)
                    .clamped(character.maximum_angular_speed_radians_per_second);
            ((target - angular_velocity) * (1.0 / f64::from(physics.fixed_delta_seconds)))
                .clamped(character.upright_alignment_acceleration_radians_per_second_squared)
        };
        let (capsule_axis_inertia, capsule_perpendicular_inertia) =
            character_capsule_inertia(character.mass_kg);
        let local_angular_acceleration = orientation.conjugate().rotate(angular_acceleration);
        let local_torque = Vec3::new(
            local_angular_acceleration.x * capsule_perpendicular_inertia,
            local_angular_acceleration.y * capsule_axis_inertia,
            local_angular_acceleration.z * capsule_perpendicular_inertia,
        ) * CHARACTER_INERTIA_MULTIPLIER;
        controls.push(BodyControl {
            body_id: player_body_id,
            force_newtons: to_physics_vec3(acceleration * character.mass_kg),
            torque_newton_meters: to_physics_vec3(orientation.rotate(local_torque)),
        });
    }
    controls
}

fn quantized_event_position(
    state: &WorldState,
    position: Vec3,
) -> Result<(verse_protocol::UniverseAddress, Vec3), IntentError> {
    let address = state
        .address_for_active_position(position)
        .map_err(|message| IntentError::rejected("physics_position_address_invalid", message))?;
    let position = state
        .active_position_for_address(&address)
        .map_err(|message| IntentError::rejected("physics_position_hydration_invalid", message))?;
    Ok((address, position))
}

fn physics_body_outcome(
    state: &WorldState,
    body: &verse_physics::BodyState,
) -> Result<PhysicsBodyOutcome, IntentError> {
    let limits = physics_scene_config();
    let (address, position) =
        quantized_event_position(state, from_physics_vec3(body.pose.position))?;
    Ok(PhysicsBodyOutcome {
        grid_id: body.body_id.clone(),
        address,
        position,
        orientation: from_physics_quat(body.pose.rotation),
        linear_velocity: from_physics_vec3(body.linear_velocity)
            .clamped(f64::from(limits.max_linear_velocity_mps)),
        angular_velocity: from_physics_vec3(body.angular_velocity)
            .clamped(f64::from(limits.max_angular_velocity_radians_per_second)),
    })
}

fn player_physics_outcome(
    state: &WorldState,
    player: &Player,
    body: &verse_physics::BodyState,
    surface_contact: bool,
    resulting_simulation_tick: u64,
) -> Result<PlayerPhysicsOutcome, IntentError> {
    let limits = physics_scene_config();
    let lease_active = resulting_simulation_tick < player.control_expires_at_simulation_tick;
    let (address, position) =
        quantized_event_position(state, from_physics_vec3(body.pose.position))?;
    Ok(PlayerPhysicsOutcome {
        player_id: player.player_id.clone(),
        address,
        position,
        orientation: from_physics_quat(body.pose.rotation),
        linear_velocity: from_physics_vec3(body.linear_velocity)
            .clamped(f64::from(limits.max_linear_velocity_mps)),
        angular_velocity: from_physics_vec3(body.angular_velocity)
            .clamped(f64::from(limits.max_angular_velocity_radians_per_second)),
        surface_contact: surface_contact || player.locomotion.support.is_some(),
        locomotion: player.locomotion.clone(),
        control_linear_input: if lease_active {
            player.control_linear_input
        } else {
            Vec3::ZERO
        },
        control_angular_input: if lease_active {
            player.control_angular_input
        } else {
            Vec3::ZERO
        },
        boost: player.boost && lease_active,
        dampeners: player.dampeners || !lease_active,
        jump: player.jump && lease_active,
        control_expires_at_simulation_tick: player.control_expires_at_simulation_tick,
    })
}

fn physics_contact_outcome(
    state: &WorldState,
    contact: &verse_physics::ContactRecord,
    substep_index: u8,
    phase: PhysicsContactPhase,
) -> Result<PhysicsContactOutcome, IntentError> {
    let (point_address, point) = quantized_event_position(state, from_physics_vec3(contact.point))?;
    Ok(PhysicsContactOutcome {
        substep_index,
        body_a_id: contact.body_a_id.clone(),
        collider_a_id: contact.collider_a_id.clone(),
        body_b_id: contact.body_b_id.clone(),
        collider_b_id: contact.collider_b_id.clone(),
        point_address,
        point,
        normal: from_physics_vec3(contact.normal),
        penetration_m: quantize_f64(contact.penetration_m),
        closing_speed_mm_per_second: quantize_nonnegative_u64(contact.impact_speed_mps, 1_000.0),
        estimated_normal_impulse_millinewton_seconds: quantize_nonnegative_u64(
            contact.estimated_normal_impulse_ns,
            1_000.0,
        ),
        reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
            state,
            &contact.body_a_id,
            &contact.body_b_id,
        ),
        phase,
    })
}

fn contact_pair_key(contact: &verse_physics::ContactRecord) -> ContactPairKey {
    ContactPairKey {
        body_a: contact.body_a_id.clone(),
        collider_a: contact.collider_a_id.clone(),
        body_b: contact.body_b_id.clone(),
        collider_b: contact.collider_b_id.clone(),
    }
}

#[cfg(test)]
fn contact_key_involves_player(contact: &ContactPairKey) -> bool {
    contact.body_a == PLAYER_BODY_ID || contact.body_b == PLAYER_BODY_ID
}

fn contact_key_involves_player_id(contact: &ContactPairKey, player_id: &str) -> bool {
    let body_id = player_body_id(player_id);
    contact.body_a == body_id || contact.body_b == body_id
}

fn player_for_body_id<'a>(state: &'a WorldState, body_id: &str) -> Option<&'a Player> {
    state
        .player
        .iter()
        .find_map(|(player_id, player)| (player_body_id(player_id) == body_id).then_some(player))
}

fn player_id_for_contact<'a>(state: &'a WorldState, contact: &ContactPairKey) -> Option<&'a str> {
    let left = player_for_body_id(state, &contact.body_a).map(|player| player.player_id.as_str());
    let right = player_for_body_id(state, &contact.body_b).map(|player| player.player_id.as_str());
    match (left, right) {
        (Some(player_id), None) | (None, Some(player_id)) => Some(player_id),
        _ => None,
    }
}

fn reduced_translational_contact_mass_grams(
    state: &WorldState,
    left_body: &str,
    right_body: &str,
) -> u64 {
    fn mass(state: &WorldState, body_id: &str) -> Option<u64> {
        if player_for_body_id(state, body_id)
            .is_some_and(|player| matches!(player.life_state, PlayerLifeState::Alive))
        {
            #[allow(clippy::cast_sign_loss)]
            return Some((content::manifest().character.mass_kg * 1_000.0).round() as u64);
        }
        let grid = state.grids.get(body_id)?;
        (!grid.anchored).then(|| state.grid_mass_grams(grid))
    }

    match (mass(state, left_body), mass(state, right_body)) {
        (Some(left), Some(right)) => {
            let numerator = u128::from(left) * u128::from(right);
            let denominator = u128::from(left).saturating_add(u128::from(right));
            u64::try_from(numerator.checked_div(denominator).unwrap_or(0)).unwrap_or(u64::MAX)
        }
        (Some(dynamic), None) | (None, Some(dynamic)) => dynamic,
        (None, None) => 0,
    }
}

fn to_physics_vec3(value: Vec3) -> PhysicsVec3 {
    PhysicsVec3::new(value.x, value.y, value.z)
}

fn from_physics_vec3(value: PhysicsVec3) -> Vec3 {
    Vec3::new(
        quantize_f64(value.x),
        quantize_f64(value.y),
        quantize_f64(value.z),
    )
}

fn to_physics_quat(value: Quat) -> PhysicsQuat {
    PhysicsQuat::new(value.x, value.y, value.z, value.w)
}

fn from_physics_quat(value: PhysicsQuat) -> Quat {
    let quantized = Quat::new(
        quantize_f32(value.x),
        quantize_f32(value.y),
        quantize_f32(value.z),
        quantize_f32(value.w),
    );
    normalize_quat(quantized)
}

fn normalize_quat(value: Quat) -> Quat {
    let length = value.x.mul_add(
        value.x,
        value
            .y
            .mul_add(value.y, value.z.mul_add(value.z, value.w * value.w)),
    );
    if length <= 1.0e-12 || !length.is_finite() {
        Quat::IDENTITY
    } else {
        let inverse = length.sqrt().recip();
        Quat::new(
            value.x * inverse,
            value.y * inverse,
            value.z * inverse,
            value.w * inverse,
        )
    }
}

fn quantize_f64(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn quantize_nonnegative_u64(value: f64, scale: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        0
    } else {
        // The explicit finite-positive check and clamp establish the cast's
        // complete u64 range before conversion.
        #[allow(clippy::cast_sign_loss)]
        {
            (value * scale).round().clamp(0.0, u64::MAX as f64) as u64
        }
    }
}

fn quantize_f32(value: f32) -> f32 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn capsule_intersects_unit_cube(center: Vec3, orientation: Quat, cube: IVec3) -> bool {
    capsule_axis_intersects_unit_cube(center, orientation.rotate(Vec3::new(0.0, 1.0, 0.0)), cube)
}

fn capsule_axis_intersects_unit_cube(center: Vec3, axis: Vec3, cube: IVec3) -> bool {
    let axis = normalized_or(axis, Vec3::new(0.0, 1.0, 0.0));
    let half_height = character_capsule_half_height();
    let start = center - axis * half_height;
    let end = center + axis * half_height;
    let radius = content::manifest().character.collision_radius_m;
    segment_unit_cube_distance_squared(start, end, cube) <= radius * radius + 1.0e-9
}

fn segment_unit_cube_distance_squared(start: Vec3, end: Vec3, cube: IVec3) -> f64 {
    let minimum = Vec3::new(
        f64::from(cube.x) - 0.5,
        f64::from(cube.y) - 0.5,
        f64::from(cube.z) - 0.5,
    );
    let maximum = Vec3::new(
        f64::from(cube.x) + 0.5,
        f64::from(cube.y) + 0.5,
        f64::from(cube.z) + 0.5,
    );
    let delta = end - start;
    let mut breakpoints = vec![0.0, 1.0];
    for (origin, direction, lower, upper) in [
        (start.x, delta.x, minimum.x, maximum.x),
        (start.y, delta.y, minimum.y, maximum.y),
        (start.z, delta.z, minimum.z, maximum.z),
    ] {
        if direction.abs() > f64::EPSILON {
            for boundary in [lower, upper] {
                let parameter = (boundary - origin) / direction;
                if parameter > 0.0 && parameter < 1.0 {
                    breakpoints.push(parameter);
                }
            }
        }
    }
    breakpoints.sort_by(f64::total_cmp);
    breakpoints.dedup_by(|left, right| (*left - *right).abs() <= f64::EPSILON);

    let point_distance_squared = |parameter: f64| {
        let point = start + delta * parameter;
        let closest = Vec3::new(
            point.x.clamp(minimum.x, maximum.x),
            point.y.clamp(minimum.y, maximum.y),
            point.z.clamp(minimum.z, maximum.z),
        );
        point.squared_distance(closest)
    };
    let mut minimum_distance_squared = point_distance_squared(0.0).min(point_distance_squared(1.0));
    for interval in breakpoints.windows(2) {
        let lower_parameter = interval[0];
        let upper_parameter = interval[1];
        let midpoint = (lower_parameter + upper_parameter) * 0.5;
        let midpoint_point = start + delta * midpoint;
        let mut quadratic = 0.0;
        let mut linear = 0.0;
        for (origin, direction, sample, lower, upper) in [
            (start.x, delta.x, midpoint_point.x, minimum.x, maximum.x),
            (start.y, delta.y, midpoint_point.y, minimum.y, maximum.y),
            (start.z, delta.z, midpoint_point.z, minimum.z, maximum.z),
        ] {
            let boundary = if sample < lower {
                Some(lower)
            } else if sample > upper {
                Some(upper)
            } else {
                None
            };
            if let Some(boundary) = boundary {
                quadratic += direction * direction;
                linear += (origin - boundary) * direction;
            }
        }
        let candidate = if quadratic > f64::EPSILON {
            (-linear / quadratic).clamp(lower_parameter, upper_parameter)
        } else {
            midpoint
        };
        minimum_distance_squared = minimum_distance_squared
            .min(point_distance_squared(lower_parameter))
            .min(point_distance_squared(upper_parameter))
            .min(point_distance_squared(candidate));
    }
    minimum_distance_squared
}

fn ensure_finite(value: Vec3, label: &str) -> Result<(), IntentError> {
    if value.x.is_finite() && value.y.is_finite() && value.z.is_finite() {
        Ok(())
    } else {
        Err(IntentError::rejected(
            "invalid_vector",
            format!("{label} must contain only finite numbers"),
        ))
    }
}

fn ensure_bounded_control(value: Vec3, label: &str) -> Result<(), IntentError> {
    ensure_finite(value, label)?;
    if value.magnitude() > MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_SOURCE_PRECISION_EPSILON {
        Err(IntentError::rejected(
            "control_input_out_of_range",
            format!("{label} magnitude must not exceed one"),
        ))
    } else {
        Ok(())
    }
}

fn mutate_resource(
    contents: &mut InventoryContents,
    resource: ResourceKind,
    mutate: impl FnOnce(&mut u64),
) {
    match resource {
        ResourceKind::Ore => mutate(&mut contents.ore),
        ResourceKind::RefinedMaterial => mutate(&mut contents.refined_material),
        ResourceKind::Component => mutate(&mut contents.components),
    }
}

fn subtract_contents(
    contents: &mut InventoryContents,
    removed: &InventoryContents,
) -> Result<(), ()> {
    contents.ore = contents.ore.checked_sub(removed.ore).ok_or(())?;
    contents.refined_material = contents
        .refined_material
        .checked_sub(removed.refined_material)
        .ok_or(())?;
    contents.components = contents
        .components
        .checked_sub(removed.components)
        .ok_or(())?;
    Ok(())
}

fn add_contents(
    contents: &mut InventoryContents,
    added: &InventoryContents,
) -> Result<(), IntentError> {
    contents.ore = contents.ore.checked_add(added.ore).ok_or_else(|| {
        IntentError::rejected("production_output_overflow", "ore output overflowed cargo")
    })?;
    contents.refined_material = contents
        .refined_material
        .checked_add(added.refined_material)
        .ok_or_else(|| {
            IntentError::rejected(
                "production_output_overflow",
                "refined material output overflowed cargo",
            )
        })?;
    contents.components = contents
        .components
        .checked_add(added.components)
        .ok_or_else(|| {
            IntentError::rejected(
                "production_output_overflow",
                "component output overflowed cargo",
            )
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use proptest::prelude::*;
    use tempfile::tempdir;
    use verse_protocol::{IVec3, UniverseAddress};

    use super::*;
    use crate::celestial;
    use crate::model::{
        ActorOperationHistory, PROCESSED_OPERATION_RETENTION_LIMIT, STARTER_GRID_ID,
        STARTER_INDUSTRY_CARGO_INVENTORY_ID, STARTER_INDUSTRY_GRID_ID, VoxelField,
    };
    use crate::persistence::AppendFailpoint;

    #[derive(Debug)]
    struct ManualTrustedClock(AtomicU64);

    impl ManualTrustedClock {
        const fn new(now_unix_ms: u64) -> Self {
            Self(AtomicU64::new(now_unix_ms))
        }

        fn set(&self, now_unix_ms: u64) {
            self.0.store(now_unix_ms, Ordering::SeqCst);
        }
    }

    impl TrustedClock for ManualTrustedClock {
        fn now_unix_ms(&self) -> Result<u64, PersistenceError> {
            Ok(self.0.load(Ordering::SeqCst))
        }
    }

    fn runtime() -> Runtime {
        Runtime::open(tempdir().expect("tempdir").keep(), 42, 5).expect("runtime opens")
    }

    fn exact_test_address(position: Vec3) -> UniverseAddress {
        celestial::address_from_local_position(&celestial::cell_origin_address(), position)
            .expect("test position has a canonical exact address")
    }

    fn set_test_player_position(player: &mut Player, position: Vec3) {
        player.address = exact_test_address(position);
        player.position = celestial::local_position_from_address(
            &celestial::cell_origin_address(),
            &player.address,
        )
        .expect("test player address hydrates an exact pose");
    }

    fn set_test_grid_position(grid: &mut Grid, position: Vec3) {
        grid.address = exact_test_address(position);
        grid.position = celestial::local_position_from_address(
            &celestial::cell_origin_address(),
            &grid.address,
        )
        .expect("test grid address hydrates an exact pose");
    }

    fn seed_industry_cargo(runtime: &mut Runtime, contents: InventoryContents) {
        let ore = contents.ore;
        let refined_material = contents.refined_material;
        let components = contents.components;
        runtime
            .state
            .inventories
            .get_mut(STARTER_INDUSTRY_CARGO_INVENTORY_ID)
            .expect("starter industry cargo exists")
            .contents = contents;
        runtime.state.ledger.genesis_ore = runtime.state.ledger.genesis_ore.saturating_add(ore);
        runtime.state.ledger.genesis_refined = runtime
            .state
            .ledger
            .genesis_refined
            .saturating_add(refined_material);
        runtime.state.ledger.genesis_components = runtime
            .state
            .ledger
            .genesis_components
            .saturating_add(components);
        assert!(runtime.state.conservation().valid);
        runtime
            .persist_snapshot()
            .expect("industry fixture persists");
    }

    fn production_intent(
        operation_id: &str,
        machine_block_id: &str,
        recipe: ProductionRecipeKind,
        batches: u64,
    ) -> ClientMessage {
        ClientMessage::QueueProduction {
            operation_sequence: 0,
            operation_id: operation_id.into(),
            machine_block_id: machine_block_id.into(),
            recipe,
            batches,
            source_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
            destination_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
        }
    }

    fn advance_whole_seconds(runtime: &mut Runtime, seconds: usize) {
        for _ in 0..seconds {
            if runtime.next_production_occurrence().is_none() {
                if !runtime
                    .state()
                    .background_production_is_runnable()
                    .expect("fixture production state is valid")
                {
                    continue;
                }
                let scheduled_for_unix_ms = runtime
                    .state()
                    .production_clock
                    .last_scheduled_for_unix_ms
                    .checked_add(1_000)
                    .expect("fixture production clock has capacity");
                let occurrence = runtime
                    .state()
                    .next_production_occurrence_at(scheduled_for_unix_ms)
                    .expect("fixture occurrence follows canonical production state");
                runtime
                    .store
                    .install_next_production_occurrence_for_test(occurrence)
                    .expect("fixture occurrence persists");
            }
            let occurrence = runtime
                .next_production_occurrence()
                .cloned()
                .expect("fixture occurrence is durably scheduled");
            if let Err(source) =
                runtime.advance_background_production_occurrence(occurrence.clone())
            {
                panic!(
                    "authoritative second advances: {source}; occurrence={occurrence:?}; clock={:?}",
                    runtime.state().production_clock
                );
            }
        }
    }

    fn two_machine_production_runtime(directory: &Path) -> Runtime {
        let mut runtime = Runtime::open(directory, 499, 100).expect("runtime opens");
        seed_industry_cargo(
            &mut runtime,
            InventoryContents {
                ore: 2,
                refined_material: 1,
                ..InventoryContents::default()
            },
        );
        runtime
            .execute_next_for_fixture(&production_intent(
                "atomic-refine",
                "block-refinery",
                ProductionRecipeKind::Refining,
                1,
            ))
            .expect("refinery queue accepts");
        runtime
            .execute_next_for_fixture(&production_intent(
                "atomic-component",
                "block-assembler",
                ProductionRecipeKind::Component,
                1,
            ))
            .expect("assembler queue accepts");
        runtime
    }

    fn last_journal_event(directory: &Path) -> CanonicalEvent {
        let journal =
            fs::read_to_string(directory.join("events.ndjson")).expect("production journal reads");
        serde_json::from_str(
            journal
                .lines()
                .last()
                .expect("production journal contains an event"),
        )
        .expect("production event parses")
    }

    #[test]
    fn whole_cell_production_quantum_is_one_ordered_atomic_event() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = two_machine_production_runtime(directory.path());
        let before_sequence = runtime.state().event_sequence;
        let occurrence = runtime
            .next_production_occurrence()
            .cloned()
            .expect("next occurrence is durably scheduled");

        assert!(
            runtime
                .advance_background_production_occurrence(occurrence)
                .expect("whole-cell quantum commits")
        );

        assert_eq!(runtime.state().event_sequence, before_sequence + 1);
        assert_eq!(
            runtime
                .state()
                .production_queues
                .values()
                .map(|queue| queue.front().expect("queue head").progress_ticks)
                .collect::<Vec<_>>(),
            vec![60, 60]
        );
        assert_eq!(
            runtime
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            1
        );
        let event = last_journal_event(directory.path());
        let EventPayload::ProductionQuantumCommitted {
            occurrence,
            elapsed_ticks,
            outcomes,
        } = event.payload
        else {
            panic!("last event must be the atomic production quantum");
        };
        assert_eq!(event.event_id, production_occurrence_event_id(&occurrence));
        assert_eq!(event.occurred_at_unix_ms, occurrence.scheduled_for_unix_ms);
        assert_eq!(occurrence.production_quantum_sequence, 1);
        assert_eq!(elapsed_ticks, 60);
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].grid_id, STARTER_INDUSTRY_GRID_ID);
        assert_eq!(outcomes[0].machine_block_id, "block-assembler");
        assert_eq!(outcomes[1].grid_id, STARTER_INDUSTRY_GRID_ID);
        assert_eq!(outcomes[1].machine_block_id, "block-refinery");
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.kind == ProductionMachineOutcomeKind::Advanced)
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn production_quantum_replay_rejects_any_outcome_tamper_before_mutation() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = two_machine_production_runtime(directory.path());
        let prior = runtime.state().clone();
        let occurrence = runtime
            .next_production_occurrence()
            .cloned()
            .expect("next occurrence is durably scheduled");
        runtime
            .advance_background_production_occurrence(occurrence)
            .expect("whole-cell quantum commits");
        let mut event = last_journal_event(directory.path());
        let EventPayload::ProductionQuantumCommitted { outcomes, .. } = &mut event.payload else {
            panic!("last event must be a production quantum");
        };
        outcomes[0].new_progress_ticks += 1;
        event.event_hash = event.calculate_hash();

        let mut replay = prior.clone();
        assert!(matches!(
            replay.apply_event(&event),
            Err(IntentError::Rejected { code, .. }) if code == "replay_production_quantum_invalid"
        ));
        assert_eq!(replay, prior);
    }

    #[test]
    fn background_occurrence_is_idempotent_and_changes_no_physics_or_life_state() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = two_machine_production_runtime(directory.path());
        let occurrence = runtime
            .next_production_occurrence()
            .cloned()
            .expect("next occurrence is durably scheduled");
        let prior_sequence = runtime.state().event_sequence;
        let prior_tick = runtime.state().simulation_tick;
        let prior_phase = runtime.state().physics_step_phase;
        let prior_players = runtime.state().player.clone();

        assert!(
            runtime
                .advance_background_production_occurrence(occurrence.clone())
                .expect("background occurrence commits")
        );
        assert_eq!(runtime.state().event_sequence, prior_sequence + 1);
        assert_eq!(runtime.state().simulation_tick, prior_tick);
        assert_eq!(runtime.state().physics_step_phase, prior_phase);
        assert_eq!(runtime.state().player, prior_players);
        let committed_hash = runtime.state().state_hash();

        assert!(
            !runtime
                .advance_background_production_occurrence(occurrence)
                .expect("duplicate occurrence reconciles")
        );
        assert_eq!(runtime.state().event_sequence, prior_sequence + 1);
        assert_eq!(runtime.state().state_hash(), committed_hash);
    }

    #[test]
    fn production_quantum_failpoints_recover_none_or_the_complete_vector() {
        for (failpoint, durable) in [
            (AppendFailpoint::BeforeWrite, false),
            (AppendFailpoint::AfterSync, true),
        ] {
            let directory = tempdir().expect("tempdir");
            let mut runtime = two_machine_production_runtime(directory.path());
            let occurrence = runtime
                .next_production_occurrence()
                .cloned()
                .expect("next occurrence is durably scheduled");
            let prior_hash = runtime.state().state_hash();
            let prior_sequence = runtime.state().event_sequence;
            runtime.store.set_append_failpoint(failpoint);

            assert!(matches!(
                runtime.advance_background_production_occurrence(occurrence),
                Err(RuntimeError::Persistence(
                    PersistenceError::InjectedFailure(_)
                ))
            ));
            assert!(runtime.is_halted());
            assert_eq!(runtime.state().state_hash(), prior_hash);
            assert_eq!(runtime.state().event_sequence, prior_sequence);
            drop(runtime);

            let recovered =
                Runtime::open(directory.path(), 499, 100).expect("failed quantum recovers");
            let progress = recovered
                .state()
                .production_queues
                .values()
                .map(|queue| queue.front().expect("queue head").progress_ticks)
                .collect::<Vec<_>>();
            if durable {
                assert_eq!(recovered.state().event_sequence, prior_sequence + 1);
                assert_eq!(progress, vec![60, 60]);
                assert_eq!(
                    recovered
                        .state()
                        .production_clock
                        .last_committed_quantum_sequence,
                    1
                );
            } else {
                assert_eq!(recovered.state().event_sequence, prior_sequence);
                assert_eq!(progress, vec![0, 0]);
                assert_eq!(
                    recovered
                        .state()
                        .production_clock
                        .last_committed_quantum_sequence,
                    0
                );
            }
            assert!(recovered.state().conservation().valid);
        }
    }

    #[test]
    fn durable_schedule_preserves_the_remaining_subsecond_across_restart() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(1_000_000));
        {
            let mut runtime = Runtime::open_with_clock(directory.path(), 601, 100, clock.clone())
                .expect("runtime opens");
            seed_industry_cargo(
                &mut runtime,
                InventoryContents {
                    ore: 2,
                    ..InventoryContents::default()
                },
            );
            runtime
                .execute_next_for_fixture(&production_intent(
                    "durable-partial-second",
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    1,
                ))
                .expect("job queues");
            assert_eq!(
                runtime
                    .next_production_occurrence()
                    .expect("occurrence arms")
                    .scheduled_for_unix_ms,
                1_001_000
            );
        }

        clock.set(1_000_750);
        let mut recovered = Runtime::open_with_clock(directory.path(), 601, 100, clock.clone())
            .expect("runtime recovers at 750 ms");
        assert_eq!(
            recovered
                .advance_due_production()
                .expect("early dispatch is inert")
                .committed_quanta,
            0
        );
        clock.set(1_000_999);
        assert_eq!(
            recovered
                .advance_due_production()
                .expect("999 ms dispatch is inert")
                .committed_quanta,
            0
        );
        clock.set(1_001_000);
        assert_eq!(
            recovered
                .advance_due_production()
                .expect("exact due dispatch commits")
                .committed_quanta,
            1
        );
        assert_eq!(
            recovered.state().production_queues["block-refinery"][0].progress_ticks,
            60
        );
    }

    #[test]
    fn forward_jump_respects_catch_up_budgets_and_retains_exact_continuation() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(2_000_000));
        let open_config;
        {
            let mut runtime = Runtime::open_with_clock(directory.path(), 602, 100, clock.clone())
                .expect("runtime opens");
            seed_industry_cargo(
                &mut runtime,
                InventoryContents {
                    ore: 200,
                    ..InventoryContents::default()
                },
            );
            runtime
                .execute_next_for_fixture(&production_intent(
                    "bounded-forward-jump",
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    100,
                ))
                .expect("long job queues");
            open_config = runtime.open_config();
        }

        clock.set(2_100_000);
        let mut recovered = Runtime::open_for_activation(&open_config)
            .expect("replacement runtime opens after forward jump");
        let first = recovered
            .advance_due_production()
            .expect("first catch-up dispatch succeeds");
        assert!((1..=MAX_PRODUCTION_CATCH_UP_QUANTA).contains(&first.committed_quanta));
        assert!(first.backlog_remaining);
        let mut committed = first.committed_quanta;
        let mut backlog_remaining = first.backlog_remaining;
        let mut dispatches = 1;
        while backlog_remaining {
            let continuation = recovered
                .advance_due_production()
                .expect("continuation dispatch succeeds");
            assert!((1..=MAX_PRODUCTION_CATCH_UP_QUANTA).contains(&continuation.committed_quanta));
            committed += continuation.committed_quanta;
            backlog_remaining = continuation.backlog_remaining;
            dispatches += 1;
            assert!(dispatches <= 100, "bounded continuation must make progress");
        }
        assert_eq!(committed, 100);
        assert_eq!(
            recovered
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            100
        );
        assert_eq!(
            recovered.state().production_queues["block-refinery"][0].progress_ticks,
            6_000
        );
    }

    #[test]
    fn drain_releases_an_idle_cell_and_a_successor_reactivates_with_a_new_fence() {
        let directory = tempdir().expect("tempdir");
        let first_fence;
        {
            let mut runtime = Runtime::open(directory.path(), 603, 100).expect("runtime opens");
            first_fence = runtime.lifecycle_status().fencing_token;
            assert_eq!(
                runtime
                    .drain_to_background_or_sleeping()
                    .expect("idle drain succeeds"),
                crate::persistence::LifecycleMode::Sleeping
            );
            let sleeping = runtime.lifecycle_status();
            assert_eq!(
                sleeping.observed_mode,
                crate::persistence::LifecycleMode::Sleeping
            );
            assert!(sleeping.expires_at_unix_ms.is_none());
            assert!(!runtime.physics_scene_is_initialized());
            assert!(matches!(
                runtime.advance(16),
                Err(RuntimeError::LifecycleUnavailable {
                    mode: crate::persistence::LifecycleMode::Sleeping
                })
            ));
        }

        let successor = Runtime::open(directory.path(), 603, 100).expect("successor activates");
        assert_eq!(
            successor.lifecycle_status().observed_mode,
            crate::persistence::LifecycleMode::Active
        );
        assert!(successor.physics_scene_is_initialized());
        assert!(successor.lifecycle_status().fencing_token > first_fence);
    }

    #[test]
    fn background_mode_runs_only_due_production_then_reactivates() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(3_000_000));
        let mut runtime = Runtime::open_with_clock(directory.path(), 604, 100, clock.clone())
            .expect("runtime opens");
        seed_industry_cargo(
            &mut runtime,
            InventoryContents {
                ore: 2,
                ..InventoryContents::default()
            },
        );
        runtime
            .execute_next_for_fixture(&production_intent(
                "background-drain",
                "block-refinery",
                ProductionRecipeKind::Refining,
                1,
            ))
            .expect("job queues");
        let simulation_tick = runtime.state().simulation_tick;
        let player_oxygen = runtime.state().player.suit_oxygen_milli;
        assert_eq!(
            runtime
                .drain_to_background_or_sleeping()
                .expect("runnable drain succeeds"),
            crate::persistence::LifecycleMode::Background
        );
        assert!(!runtime.physics_scene_is_initialized());
        assert!(matches!(
            runtime.advance(250),
            Err(RuntimeError::LifecycleUnavailable {
                mode: crate::persistence::LifecycleMode::Background
            })
        ));

        clock.set(3_001_000);
        assert_eq!(
            runtime
                .advance_due_production()
                .expect("background production advances")
                .committed_quanta,
            1
        );
        assert_eq!(runtime.state().simulation_tick, simulation_tick);
        assert_eq!(runtime.state().player.suit_oxygen_milli, player_oxygen);
        assert!(runtime.activation_step().expect("activation completes"));
        assert_eq!(
            runtime.lifecycle_status().observed_mode,
            crate::persistence::LifecycleMode::Active
        );
        assert!(runtime.physics_scene_is_initialized());
    }

    #[test]
    fn idle_production_rearms_from_the_new_trusted_boundary_without_cursor_conflict() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(3_500_000));
        let mut runtime = Runtime::open_with_clock(directory.path(), 608, 100, clock.clone())
            .expect("runtime opens");
        seed_industry_cargo(
            &mut runtime,
            InventoryContents {
                ore: 2,
                ..InventoryContents::default()
            },
        );
        runtime
            .execute_next_for_fixture(&production_intent(
                "idle-rearm-refining",
                "block-refinery",
                ProductionRecipeKind::Refining,
                1,
            ))
            .expect("refining queues");
        for due in [3_501_000, 3_502_000] {
            clock.set(due);
            assert_eq!(
                runtime
                    .advance_due_production()
                    .expect("refining quantum commits")
                    .committed_quanta,
                1
            );
        }
        assert!(runtime.next_production_occurrence().is_none());
        assert_eq!(
            runtime
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            2
        );

        clock.set(3_502_125);
        runtime
            .execute_next_for_fixture(&production_intent(
                "idle-rearm-assembly",
                "block-assembler",
                ProductionRecipeKind::Component,
                1,
            ))
            .expect("assembly queues after the idle gap");
        let rearmed = runtime
            .next_production_occurrence()
            .cloned()
            .expect("new runnable boundary rearms production");
        assert_eq!(rearmed.production_quantum_sequence, 3);
        assert_eq!(rearmed.scheduled_for_unix_ms, 3_503_125);

        let before_conflict = runtime.state().state_hash();
        let mut conflicting = rearmed.clone();
        conflicting.scheduled_for_unix_ms += 1;
        assert!(matches!(
            runtime.advance_background_production_occurrence(conflicting),
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "production_occurrence_delivery_conflict"
        ));
        assert_eq!(runtime.state().state_hash(), before_conflict);

        clock.set(rearmed.scheduled_for_unix_ms);
        assert_eq!(
            runtime
                .advance_due_production()
                .expect("rearmed occurrence commits")
                .committed_quanta,
            1
        );
        assert_eq!(
            runtime
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            3
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn activation_catches_up_only_through_its_durable_wake_cutoff() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(4_000_000));
        let mut runtime = Runtime::open_with_clock(directory.path(), 605, 100, clock.clone())
            .expect("runtime opens");
        seed_industry_cargo(
            &mut runtime,
            InventoryContents {
                ore: 200,
                ..InventoryContents::default()
            },
        );
        runtime
            .execute_next_for_fixture(&production_intent(
                "activation-cutoff",
                "block-refinery",
                ProductionRecipeKind::Refining,
                100,
            ))
            .expect("long job queues");
        assert_eq!(
            runtime
                .drain_to_background_or_sleeping()
                .expect("runnable cell drains"),
            crate::persistence::LifecycleMode::Background
        );
        let open_config = runtime.open_config();
        drop(runtime);

        clock.set(4_070_000);
        let mut runtime = Runtime::open_for_activation(&open_config)
            .expect("replacement acquires the background cell for activation");
        assert!(!runtime.activation_step().expect("bounded catch-up yields"));
        assert_eq!(
            runtime.lifecycle_status().observed_mode,
            crate::persistence::LifecycleMode::Activating
        );
        assert!(!runtime.physics_scene_is_initialized());
        assert!(
            (1..=u64::try_from(MAX_PRODUCTION_CATCH_UP_QUANTA).expect("catch-up budget fits u64"))
                .contains(
                    &runtime
                        .state()
                        .production_clock
                        .last_committed_quantum_sequence
                )
        );
        let first_activation_frontier = runtime
            .state()
            .production_clock
            .last_committed_quantum_sequence;
        drop(runtime);

        clock.set(4_075_000);
        let mut runtime =
            Runtime::open_hosted_with_clock(directory.path(), 605, 100, clock.clone())
                .expect("activation crash recovery preserves the original cut-off");
        assert_eq!(
            runtime
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            first_activation_frontier
        );
        let mut activated = false;
        for _ in 0..100 {
            activated = runtime
                .activation_step()
                .expect("cut-off catch-up continues");
            if activated {
                break;
            }
        }
        assert!(
            activated,
            "bounded activation eventually reaches its cut-off"
        );
        assert_eq!(
            runtime
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            70
        );
        assert_eq!(
            runtime
                .next_production_occurrence()
                .expect("post-cutoff work remains scheduled")
                .scheduled_for_unix_ms,
            4_071_000
        );
        assert!(runtime.physics_scene_is_initialized());
    }

    #[test]
    fn hosted_restart_restores_sleeping_without_physics_or_writer_ownership() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(5_000_000));
        let mut runtime = Runtime::open_with_clock(directory.path(), 606, 100, clock.clone())
            .expect("runtime opens");
        assert_eq!(
            runtime
                .drain_to_background_or_sleeping()
                .expect("idle cell sleeps"),
            crate::persistence::LifecycleMode::Sleeping
        );
        drop(runtime);

        let sleeping_host =
            Runtime::open_hosted_with_clock(directory.path(), 606, 100, clock.clone())
                .expect("host recovers the durable sleeping mode");
        assert_eq!(
            sleeping_host.lifecycle_status().observed_mode,
            crate::persistence::LifecycleMode::Sleeping
        );
        assert!(!sleeping_host.physics_scene_is_initialized());

        let wake_config = sleeping_host.open_config();
        let waking = Runtime::open_for_activation(&wake_config)
            .expect("sleeping host retains no exclusive writer ownership");
        assert_eq!(
            waking.lifecycle_status().observed_mode,
            crate::persistence::LifecycleMode::Activating
        );
        assert!(!waking.physics_scene_is_initialized());
    }

    #[cfg(unix)]
    #[test]
    fn cross_process_hard_kill_replays_exactly_one_background_output() {
        const CHILD_FLAG: &str = "VERSE_P16_CROSS_PROCESS_CHILD";
        const ROOT_ENV: &str = "VERSE_P16_CROSS_PROCESS_ROOT";
        const READY_ENV: &str = "VERSE_P16_CROSS_PROCESS_READY";
        const SEED: u64 = 607;
        const START_UNIX_MS: u64 = 6_000_000;

        if std::env::var_os(CHILD_FLAG).is_some() {
            let root = std::path::PathBuf::from(
                std::env::var_os(ROOT_ENV).expect("child receives the universe root"),
            );
            let ready = std::path::PathBuf::from(
                std::env::var_os(READY_ENV).expect("child receives the readiness marker"),
            );
            let clock = Arc::new(ManualTrustedClock::new(START_UNIX_MS));
            let mut runtime = Runtime::open_with_clock(&root, SEED, 100, clock.clone())
                .expect("child runtime opens");
            seed_industry_cargo(
                &mut runtime,
                InventoryContents {
                    ore: 2,
                    ..InventoryContents::default()
                },
            );
            runtime
                .execute_next_for_fixture(&production_intent(
                    "cross-process-background",
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    1,
                ))
                .expect("child queues refining");
            let job = runtime
                .state
                .production_queues
                .get_mut("block-refinery")
                .and_then(|queue| queue.front_mut())
                .expect("child refining job exists");
            job.progress_ticks = job
                .duration_ticks
                .checked_sub(u64::from(content::manifest().physics.fixed_step_hz))
                .expect("fixture duration exceeds one quantum");
            runtime
                .persist_snapshot()
                .expect("near-complete child job persists");
            assert_eq!(
                runtime
                    .drain_to_background_or_sleeping()
                    .expect("child drains to background"),
                crate::persistence::LifecycleMode::Background
            );
            clock.set(START_UNIX_MS + 1_000);
            assert_eq!(
                runtime
                    .advance_due_production()
                    .expect("child commits one due quantum")
                    .committed_quanta,
                1
            );
            assert_eq!(
                runtime.state.inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                    .contents
                    .refined_material,
                1
            );
            fs::write(&ready, b"journal-synced").expect("child publishes readiness marker");
            loop {
                std::thread::park();
            }
        }

        let directory = tempdir().expect("tempdir");
        let ready = directory.path().join("background-event-synced");
        let current_executable =
            std::env::current_exe().expect("test executable path remains available");
        let mut child = std::process::Command::new(current_executable)
            .arg("--exact")
            .arg("engine::tests::cross_process_hard_kill_replays_exactly_one_background_output")
            .arg("--nocapture")
            .env(CHILD_FLAG, "1")
            .env(ROOT_ENV, directory.path())
            .env(READY_ENV, &ready)
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("cross-process fixture starts");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !ready.exists() {
            if let Some(status) = child.try_wait().expect("child status remains readable") {
                panic!("cross-process fixture exited before the hard-kill boundary: {status}");
            }
            assert!(
                std::time::Instant::now() < deadline,
                "cross-process fixture did not reach the hard-kill boundary"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let stop_status = std::process::Command::new("kill")
            .arg("-STOP")
            .arg(child.id().to_string())
            .status()
            .expect("the local host provides POSIX process signalling");
        assert!(stop_status.success());
        let blocked_clock = Arc::new(ManualTrustedClock::new(START_UNIX_MS + 20_000));
        assert!(matches!(
            Runtime::open_hosted_with_clock(directory.path(), SEED, 100, blocked_clock,),
            Err(RuntimeError::Persistence(
                PersistenceError::WriterAlreadyActive(_)
            ))
        ));
        child
            .kill()
            .expect("hard-kill terminates the writer process");
        let status = child.wait().expect("hard-killed child is reaped");
        assert!(!status.success());

        let clock = Arc::new(ManualTrustedClock::new(START_UNIX_MS + 1_000));
        let mut recovered =
            Runtime::open_hosted_with_clock(directory.path(), SEED, 100, clock.clone())
                .expect("successor replays the synced event without a graceful snapshot");
        assert_eq!(
            recovered.lifecycle_status().observed_mode,
            crate::persistence::LifecycleMode::Background
        );
        assert!(!recovered.physics_scene_is_initialized());
        assert_eq!(
            recovered
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            1
        );
        assert_eq!(
            recovered.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                .contents
                .refined_material,
            1
        );
        assert!(recovered.state().conservation().valid);
        assert_eq!(
            recovered
                .background_dispatch_step()
                .expect("reconciled background cell releases"),
            crate::persistence::LifecycleMode::Sleeping
        );

        let wake_config = recovered.open_config();
        drop(recovered);
        let mut active = Runtime::open_for_activation(&wake_config)
            .expect("successor reacquires the sleeping cell");
        assert!(active.activation_step().expect("successor activates"));
        assert_eq!(
            active.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                .contents
                .refined_material,
            1
        );
        assert_eq!(
            active
                .state()
                .production_clock
                .last_committed_quantum_sequence,
            1
        );
    }

    #[test]
    fn advance_outcome_classifies_motion_and_life_support() {
        let mut motion_runtime = runtime();
        let player = motion_runtime.state().player.primary();
        let movement_epoch = player.movement_epoch;
        let input_sequence = player.last_received_input_sequence + 1;
        motion_runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "advance-impact-motion".into(),
                movement_epoch,
                input_sequence,
                linear_input: Vec3::new(0.0, 0.0, -1.0),
                angular_input: Vec3::ZERO,
                boost: false,
                dampeners: true,
                jump: false,
            })
            .expect("physics control commits");
        assert_eq!(
            motion_runtime
                .advance_with_outcome(17)
                .expect("physics advances")
                .impact,
            AdvanceImpact::Motion
        );

        let mut life_support_runtime = runtime();
        for _ in 0..3 {
            assert_eq!(
                life_support_runtime
                    .advance_with_outcome(250)
                    .expect("partial life-support interval advances")
                    .impact,
                AdvanceImpact::None
            );
        }
        assert_eq!(
            life_support_runtime
                .advance_with_outcome(250)
                .expect("life-support transition advances")
                .impact,
            AdvanceImpact::Structural
        );
        assert_eq!(
            AdvanceImpact::Motion.combine(AdvanceImpact::Structural),
            AdvanceImpact::Structural
        );
    }

    #[test]
    fn physical_industry_reserves_advances_completes_and_recovers_exactly() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 501, 100).expect("runtime opens");
            runtime
                .admit_development_player("player-remote")
                .expect("projection actor admits");
            seed_industry_cargo(
                &mut runtime,
                InventoryContents {
                    ore: 2,
                    ..InventoryContents::default()
                },
            );
            let refine = production_intent(
                "physical-refine-1",
                "block-refinery",
                ProductionRecipeKind::Refining,
                1,
            );
            let first = runtime
                .execute_next_for_fixture(&refine)
                .expect("connected refinery enqueue accepts");
            let retry = runtime
                .execute_next_for_fixture(&refine)
                .expect("exact enqueue retry returns its durable receipt");
            assert_eq!(first, retry);
            assert_eq!(
                runtime.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                    .contents
                    .ore,
                0
            );
            assert_eq!(runtime.state().production_queues["block-refinery"].len(), 1);
            assert_eq!(runtime.state().ledger.refine_batches, 0);
            assert_eq!(runtime.state().player.career.refining_batches, 0);
            assert!(runtime.state().conservation().valid);

            let local = runtime
                .state()
                .project_world_snapshot(Some("player-local"))
                .expect("local production projection");
            assert_eq!(
                local
                    .actor_private
                    .expect("local private state")
                    .production_queues
                    .len(),
                1
            );
            let remote = runtime
                .state()
                .project_world_snapshot(Some("player-remote"))
                .expect("remote production projection");
            assert!(
                remote
                    .actor_private
                    .expect("remote private state")
                    .production_queues
                    .is_empty()
            );
            assert!(
                runtime
                    .state()
                    .project_world_snapshot(None)
                    .expect("spectator projection")
                    .actor_private
                    .is_none()
            );

            advance_whole_seconds(&mut runtime, 1);
            let refining = runtime.state().production_queues["block-refinery"]
                .front()
                .expect("refining remains queued");
            assert_eq!(refining.progress_ticks, 60);
            assert_eq!(refining.duration_ticks, 120);
            assert_eq!(runtime.state().ledger.refine_batches, 0);
            assert_eq!(
                runtime.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                    .contents
                    .refined_material,
                0
            );

            advance_whole_seconds(&mut runtime, 1);
            assert!(
                !runtime
                    .state()
                    .production_queues
                    .contains_key("block-refinery")
            );
            assert_eq!(runtime.state().ledger.refine_batches, 1);
            assert_eq!(runtime.state().player.career.refining_batches, 1);
            assert_eq!(
                runtime.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                    .contents
                    .refined_material,
                1
            );

            runtime
                .execute_next_for_fixture(&production_intent(
                    "physical-component-1",
                    "block-assembler",
                    ProductionRecipeKind::Component,
                    1,
                ))
                .expect("connected assembler enqueue accepts");
            advance_whole_seconds(&mut runtime, 1);
            assert_eq!(
                runtime.state().production_queues["block-assembler"]
                    .front()
                    .expect("assembler remains queued")
                    .progress_ticks,
                60
            );
            advance_whole_seconds(&mut runtime, 1);
            assert!(
                !runtime
                    .state()
                    .production_queues
                    .contains_key("block-assembler")
            );
            assert_eq!(runtime.state().ledger.crafted_components, 1);
            assert_eq!(runtime.state().player.career.components_crafted, 1);
            assert_eq!(
                runtime.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID].contents,
                InventoryContents {
                    components: 1,
                    ..InventoryContents::default()
                }
            );
            assert!(runtime.state().conservation().valid);
            runtime
                .persist_snapshot()
                .expect("completed industry persists");
            expected_hash = runtime.state().state_hash();
        }
        let recovered = Runtime::open(directory.path(), 501, 100).expect("industry recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert_eq!(
            recovered.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                .contents
                .components,
            1
        );
    }

    #[test]
    fn production_pauses_for_power_and_route_and_delivers_blocked_output_once() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 502, 100).expect("runtime opens");
        seed_industry_cargo(
            &mut runtime,
            InventoryContents {
                ore: 2,
                ..InventoryContents::default()
            },
        );
        runtime
            .execute_next_for_fixture(&production_intent(
                "paused-refine-1",
                "block-refinery",
                ProductionRecipeKind::Refining,
                1,
            ))
            .expect("refinery enqueue accepts");

        runtime
            .state
            .grids
            .get_mut(STARTER_INDUSTRY_GRID_ID)
            .expect("industry grid")
            .blocks
            .get_mut("block-industry-power")
            .expect("industry power block")
            .construction_complete = false;
        advance_whole_seconds(&mut runtime, 1);
        assert_eq!(
            runtime.state().production_queues["block-refinery"][0].progress_ticks,
            0
        );
        assert_eq!(
            runtime.state().production_job_status("block-refinery", 0),
            verse_protocol::ProductionJobStatus::PausedPower
        );

        runtime
            .state
            .grids
            .get_mut(STARTER_INDUSTRY_GRID_ID)
            .expect("industry grid")
            .blocks
            .get_mut("block-industry-power")
            .expect("industry power block")
            .construction_complete = true;
        runtime
            .state
            .grids
            .get_mut(STARTER_INDUSTRY_GRID_ID)
            .expect("industry grid")
            .blocks
            .get_mut("block-conveyor")
            .expect("industry conveyor")
            .construction_complete = false;
        advance_whole_seconds(&mut runtime, 1);
        assert_eq!(
            runtime.state().production_queues["block-refinery"][0].progress_ticks,
            0
        );
        assert_eq!(
            runtime.state().production_job_status("block-refinery", 0),
            verse_protocol::ProductionJobStatus::PausedRoute
        );

        runtime
            .state
            .grids
            .get_mut(STARTER_INDUSTRY_GRID_ID)
            .expect("industry grid")
            .blocks
            .get_mut("block-conveyor")
            .expect("industry conveyor")
            .construction_complete = true;
        runtime
            .state
            .inventories
            .get_mut(STARTER_INDUSTRY_CARGO_INVENTORY_ID)
            .expect("industry cargo")
            .capacity_liters = 1;
        advance_whole_seconds(&mut runtime, 2);
        let blocked = runtime.state().production_queues["block-refinery"]
            .front()
            .expect("blocked output remains in machine escrow");
        assert_eq!(blocked.progress_ticks, blocked.duration_ticks);
        assert_eq!(blocked.reserved_inputs, InventoryContents::default());
        assert_eq!(blocked.pending_outputs.refined_material, 1);
        assert_eq!(runtime.state().ledger.refine_batches, 1);
        assert_eq!(runtime.state().player.career.refining_batches, 1);
        assert_eq!(
            runtime.state().production_job_status("block-refinery", 0),
            verse_protocol::ProductionJobStatus::OutputBlocked
        );
        let experience_after_completion = runtime.state().player.experience;
        assert!(runtime.state().conservation().valid);
        runtime.persist_snapshot().expect("blocked output persists");
        let blocked_hash = runtime.state().state_hash();
        drop(runtime);

        let mut recovered =
            Runtime::open(directory.path(), 502, 100).expect("blocked job recovers");
        assert_eq!(recovered.state().state_hash(), blocked_hash);
        recovered
            .state
            .inventories
            .get_mut(STARTER_INDUSTRY_CARGO_INVENTORY_ID)
            .expect("industry cargo")
            .capacity_liters = CARGO_INVENTORY_CAPACITY_LITERS;
        advance_whole_seconds(&mut recovered, 1);
        assert!(
            !recovered
                .state()
                .production_queues
                .contains_key("block-refinery")
        );
        assert_eq!(
            recovered.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                .contents
                .refined_material,
            1
        );
        assert_eq!(
            recovered.state().player.experience,
            experience_after_completion
        );
        assert_eq!(recovered.state().ledger.refine_batches, 1);
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn destroying_a_production_machine_drops_every_escrowed_asset_once() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 503, 100).expect("runtime opens");
            seed_industry_cargo(
                &mut runtime,
                InventoryContents {
                    ore: 4,
                    ..InventoryContents::default()
                },
            );
            runtime
                .execute_next_for_fixture(&production_intent(
                    "destroyed-machine-refine",
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    2,
                ))
                .expect("refinery work reserves input");
            assert_eq!(
                runtime.state().production_queues["block-refinery"][0]
                    .reserved_inputs
                    .ore,
                4
            );
            aim_player_at_block(&mut runtime, STARTER_INDUSTRY_GRID_ID, "block-refinery");
            runtime.persist_snapshot().expect("damage pose persists");
            for strike in 0..6 {
                runtime
                    .execute_next_for_fixture(&ClientMessage::DamageBlock {
                        operation_sequence: 0,
                        operation_id: format!("destroy-refinery-{strike}"),
                        grid_id: STARTER_INDUSTRY_GRID_ID.into(),
                        block_id: "block-refinery".into(),
                    })
                    .expect("authoritative damage applies");
            }
            assert!(runtime.state().block_grid("block-refinery").is_none());
            assert!(
                !runtime
                    .state()
                    .production_queues
                    .contains_key("block-refinery")
            );
            let drops = runtime
                .state()
                .inventories
                .values()
                .filter(|inventory| {
                    matches!(
                        &inventory.domain,
                        InventoryDomain::Dropped { reason, .. }
                            if reason == "production_machine_destroyed"
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(drops.len(), 1);
            assert_eq!(
                drops[0].contents,
                InventoryContents {
                    ore: 4,
                    ..InventoryContents::default()
                }
            );
            assert_eq!(runtime.state().ledger.refine_batches, 0);
            assert!(runtime.state().conservation().valid);
            expected_hash = runtime.state().state_hash();
        }
        let recovered = Runtime::open(directory.path(), 503, 100).expect("drop recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert_eq!(
            recovered
                .state()
                .inventories
                .values()
                .filter(|inventory| matches!(
                    &inventory.domain,
                    InventoryDomain::Dropped { reason, .. }
                        if reason == "production_machine_destroyed"
                ))
                .count(),
            1
        );
    }

    #[test]
    fn grid_split_retains_machine_queue_once_and_pauses_the_broken_route() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        let progress_before_split;
        {
            let mut runtime = Runtime::open(directory.path(), 504, 100).expect("runtime opens");
            seed_industry_cargo(
                &mut runtime,
                InventoryContents {
                    ore: 4,
                    ..InventoryContents::default()
                },
            );
            runtime
                .execute_next_for_fixture(&production_intent(
                    "split-machine-refine",
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    2,
                ))
                .expect("refinery work reserves input");
            advance_whole_seconds(&mut runtime, 1);
            progress_before_split =
                runtime.state().production_queues["block-refinery"][0].progress_ticks;

            aim_player_at_block(&mut runtime, STARTER_INDUSTRY_GRID_ID, "block-conveyor");
            runtime.persist_snapshot().expect("damage pose persists");
            for strike in 0..3 {
                runtime
                    .execute_next_for_fixture(&ClientMessage::DamageBlock {
                        operation_sequence: 0,
                        operation_id: format!("split-industry-conveyor-{strike}"),
                        grid_id: STARTER_INDUSTRY_GRID_ID.into(),
                        block_id: "block-conveyor".into(),
                    })
                    .expect("authoritative conveyor damage applies");
            }

            assert!(runtime.state().block_grid("block-conveyor").is_none());
            let (machine_grid, _) = runtime
                .state()
                .block_grid("block-refinery")
                .expect("refinery survives on one split fragment");
            assert_ne!(machine_grid.grid_id, STARTER_INDUSTRY_GRID_ID);
            assert_eq!(runtime.state().production_queues["block-refinery"].len(), 1);
            assert_eq!(
                runtime.state().production_queues["block-refinery"][0].progress_ticks,
                progress_before_split
            );
            assert_eq!(
                runtime.state().production_job_status("block-refinery", 0),
                verse_protocol::ProductionJobStatus::PausedRoute
            );
            assert_eq!(
                runtime
                    .state()
                    .inventories
                    .values()
                    .filter(|inventory| matches!(
                        &inventory.domain,
                        InventoryDomain::Dropped { reason, .. }
                            if reason == "production_machine_destroyed"
                    ))
                    .count(),
                0
            );

            advance_whole_seconds(&mut runtime, 2);
            assert_eq!(
                runtime.state().production_queues["block-refinery"][0].progress_ticks,
                progress_before_split
            );
            assert!(runtime.state().conservation().valid);
            runtime.persist_snapshot().expect("split queue persists");
            expected_hash = runtime.state().state_hash();
        }

        let recovered = Runtime::open(directory.path(), 504, 100).expect("split queue recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert_eq!(
            recovered.state().production_queues["block-refinery"][0].progress_ticks,
            progress_before_split
        );
        assert_eq!(
            recovered.state().production_job_status("block-refinery", 0),
            verse_protocol::ProductionJobStatus::PausedRoute
        );
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn production_queue_bound_rejects_the_thirty_third_job_without_reserving_input() {
        let mut runtime = runtime();
        seed_industry_cargo(
            &mut runtime,
            InventoryContents {
                ore: 66,
                ..InventoryContents::default()
            },
        );
        for index in 0..content::manifest().production.queue_limit_per_machine {
            runtime
                .execute_next_for_fixture(&production_intent(
                    &format!("bounded-refinery-{index}"),
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    1,
                ))
                .expect("job within the queue bound accepts");
        }
        let queue = &runtime.state().production_queues["block-refinery"];
        assert_eq!(
            queue.len(),
            content::manifest().production.queue_limit_per_machine
        );
        assert!(
            queue
                .iter()
                .map(|job| job.queued_event_sequence)
                .is_sorted()
        );
        assert_eq!(
            runtime.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                .contents
                .ore,
            2
        );

        let before_hash = runtime.state().state_hash();
        let before_sequence = runtime.state().event_sequence;
        let error = runtime
            .execute_next_for_fixture(&production_intent(
                "bounded-refinery-overflow",
                "block-refinery",
                ProductionRecipeKind::Refining,
                1,
            ))
            .expect_err("the thirty-third job rejects");
        assert!(matches!(
            error,
            RuntimeError::Intent(IntentError::Rejected { ref code, .. })
                if code == "production_queue_full"
        ));
        assert_eq!(runtime.state().state_hash(), before_hash);
        assert_eq!(runtime.state().event_sequence, before_sequence);
        assert_eq!(
            runtime.state().inventories[STARTER_INDUSTRY_CARGO_INVENTORY_ID]
                .contents
                .ore,
            2
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn production_admission_rejects_shortcuts_wrong_authority_and_broken_routes_without_mutation() {
        let mut runtime = runtime();
        runtime
            .admit_development_player("player-remote")
            .expect("remote actor admits");
        seed_industry_cargo(
            &mut runtime,
            InventoryContents {
                ore: 2,
                ..InventoryContents::default()
            },
        );

        let cases = [
            (
                "player-local",
                ClientMessage::RefineOre {
                    operation_sequence: 0,
                    operation_id: "legacy-pocket-refine".into(),
                    inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                    batches: 1,
                },
                "physical_machine_required",
            ),
            (
                "player-remote",
                production_intent(
                    "foreign-machine",
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    1,
                ),
                "grid_access_denied",
            ),
            (
                "player-local",
                production_intent(
                    "wrong-recipe",
                    "block-refinery",
                    ProductionRecipeKind::Component,
                    1,
                ),
                "production_recipe_mismatch",
            ),
            (
                "player-local",
                ClientMessage::QueueProduction {
                    operation_sequence: 0,
                    operation_id: "suit-endpoint".into(),
                    machine_block_id: "block-refinery".into(),
                    recipe: ProductionRecipeKind::Refining,
                    batches: 1,
                    source_inventory_id: PLAYER_INVENTORY_ID.into(),
                    destination_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                },
                "production_cargo_required",
            ),
            (
                "player-local",
                ClientMessage::QueueProduction {
                    operation_sequence: 0,
                    operation_id: "cross-grid-route".into(),
                    machine_block_id: "block-refinery".into(),
                    recipe: ProductionRecipeKind::Refining,
                    batches: 1,
                    source_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                    destination_inventory_id: "inventory-cargo-starter".into(),
                },
                "production_route_missing",
            ),
            (
                "player-local",
                production_intent(
                    "zero-batches",
                    "block-refinery",
                    ProductionRecipeKind::Refining,
                    0,
                ),
                "production_quantity_invalid",
            ),
        ];
        for (actor, message, expected_code) in cases {
            let before_hash = runtime.state().state_hash();
            let before_sequence = runtime.state().event_sequence;
            let error = runtime
                .execute_next_as_for_fixture(actor, &message)
                .expect_err("invalid production admission rejects");
            assert!(matches!(
                error,
                RuntimeError::Intent(IntentError::Rejected { ref code, .. })
                    if code == expected_code
            ));
            assert_eq!(runtime.state().state_hash(), before_hash);
            assert_eq!(runtime.state().event_sequence, before_sequence);
        }
        assert!(runtime.state().production_queues.is_empty());

        runtime
            .state
            .grids
            .get_mut(STARTER_INDUSTRY_GRID_ID)
            .expect("industry grid")
            .blocks
            .get_mut("block-conveyor")
            .expect("industry conveyor")
            .construction_complete = false;
        let before_hash = runtime.state().state_hash();
        let before_sequence = runtime.state().event_sequence;
        let error = runtime
            .execute_next_for_fixture(&production_intent(
                "unfinished-conveyor-route",
                "block-refinery",
                ProductionRecipeKind::Refining,
                1,
            ))
            .expect_err("an unfinished conveyor cannot route production");
        assert!(matches!(
            error,
            RuntimeError::Intent(IntentError::Rejected { ref code, .. })
                if code == "production_route_missing"
        ));
        assert_eq!(runtime.state().state_hash(), before_hash);
        assert_eq!(runtime.state().event_sequence, before_sequence);
        assert!(runtime.state().production_queues.is_empty());
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn authenticated_actor_is_separate_from_the_client_intent() {
        let mut runtime = runtime();
        let before_hash = runtime.state().state_hash();
        let before_sequence = runtime.state().event_sequence;
        let intent = ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "actor-isolation-1".into(),
            movement_epoch: runtime.state().player.movement_epoch,
            input_sequence: 1,
            linear_input: Vec3::new(0.0, 0.0, -1.0),
            angular_input: Vec3::ZERO,
            boost: false,
            dampeners: true,
            jump: false,
        };

        let rejected = runtime
            .execute_next_as_for_fixture("another-player", &intent)
            .expect_err("an unbound actor cannot control the canonical player");
        assert!(matches!(
            rejected,
            RuntimeError::Intent(IntentError::Rejected { ref code, .. })
                if code == "actor_not_present"
        ));
        assert_eq!(runtime.state().event_sequence, before_sequence);
        assert_eq!(runtime.state().state_hash(), before_hash);

        let receipt = runtime
            .execute_next_as_for_fixture("player-local", &intent)
            .expect("the bound actor can submit its own control");
        assert_eq!(runtime.state().event_sequence, before_sequence + 1);
        assert_eq!(runtime.state().player.last_received_input_sequence, 1);
        assert_eq!(receipt.event_sequence, before_sequence + 1);
        assert_eq!(
            runtime
                .state()
                .processed_operation("player-local", "actor-isolation-1")
                .expect("actor-scoped receipt exists"),
            &receipt
        );
    }

    fn sequenced_suit_message(
        operation_sequence: u64,
        operation_id: impl Into<String>,
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

    fn rejected_code(error: RuntimeError) -> String {
        match error {
            RuntimeError::Intent(IntentError::Rejected { code, .. }) => code,
            other => panic!("expected an intent rejection, received {other}"),
        }
    }

    #[test]
    fn operation_sequences_detect_conflicts_gaps_and_reuse_rejected_frontier() {
        let mut runtime = runtime();
        let first = sequenced_suit_message(1, "sequence-one", false);
        let first_receipt = runtime.execute(&first).expect("sequence one commits");
        let committed_hash = runtime.state().state_hash();
        assert_eq!(first_receipt.operation_sequence, 1);

        assert_eq!(
            runtime
                .execute(&first)
                .expect("an exact retry is idempotent"),
            first_receipt
        );
        assert_eq!(runtime.state().state_hash(), committed_hash);

        assert_eq!(
            rejected_code(
                runtime
                    .execute(&sequenced_suit_message(1, "changed-id", false))
                    .expect_err("a changed diagnostic ID changes the typed intent")
            ),
            "operation_conflict"
        );
        assert_eq!(
            rejected_code(
                runtime
                    .execute(&sequenced_suit_message(1, "sequence-one", true))
                    .expect_err("a changed payload conflicts at the committed sequence")
            ),
            "operation_conflict"
        );
        assert_eq!(runtime.state().state_hash(), committed_hash);

        assert_eq!(
            rejected_code(
                runtime
                    .execute(&sequenced_suit_message(0, "zero", true))
                    .expect_err("zero is never a client operation sequence")
            ),
            "operation_sequence_invalid"
        );
        assert_eq!(
            rejected_code(
                runtime
                    .execute(&sequenced_suit_message(3, "gap", true))
                    .expect_err("a client cannot skip sequence two")
            ),
            "operation_sequence_gap"
        );

        let rejected_second = sequenced_suit_message(2, "rejected-two", false);
        assert_eq!(
            rejected_code(
                runtime
                    .execute(&rejected_second)
                    .expect_err("an unchanged suit mode is rejected")
            ),
            "suit_mode_no_change"
        );
        assert_eq!(runtime.state().last_operation_sequence("player-local"), 1);
        assert_eq!(runtime.state().state_hash(), committed_hash);

        let corrected = sequenced_suit_message(2, "corrected-two", true);
        let corrected_receipt = runtime
            .execute(&corrected)
            .expect("the rejected frontier can be reused with corrected intent");
        assert_eq!(corrected_receipt.operation_sequence, 2);
        assert_eq!(runtime.state().last_operation_sequence("player-local"), 2);
    }

    #[test]
    fn operation_fingerprints_are_actor_scoped_and_canonicalize_signed_zero() {
        let mut runtime = runtime();
        runtime
            .admit_development_player("player-remote")
            .expect("second actor is pre-admitted");

        let shared = sequenced_suit_message(1, "shared-diagnostic", false);
        let local = runtime
            .execute(&shared)
            .expect("local sequence one commits");
        let remote = runtime
            .execute_as("player-remote", &shared)
            .expect("remote has an independent sequence one");
        assert_eq!(local.operation_sequence, 1);
        assert_eq!(remote.operation_sequence, 1);
        assert_ne!(local.event_sequence, remote.event_sequence);
        assert_eq!(runtime.state().last_operation_sequence("player-local"), 1);
        assert_eq!(runtime.state().last_operation_sequence("player-remote"), 1);

        let positive_zero = ClientMessage::SetPlayerControl {
            operation_sequence: 2,
            operation_id: "zero-control".into(),
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(0.0, 0.0, 0.0),
            angular_input: Vec3::new(0.0, 0.0, 0.0),
            boost: false,
            dampeners: true,
            jump: false,
        };
        let negative_zero = ClientMessage::SetPlayerControl {
            operation_sequence: 2,
            operation_id: "zero-control".into(),
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(-0.0, 0.0, -0.0),
            angular_input: Vec3::new(0.0, -0.0, 0.0),
            boost: false,
            dampeners: true,
            jump: false,
        };
        assert_eq!(
            runtime
                .state()
                .client_intent_fingerprint("player-local", &positive_zero)
                .expect("positive zero fingerprints"),
            runtime
                .state()
                .client_intent_fingerprint("player-local", &negative_zero)
                .expect("negative zero fingerprints canonically")
        );

        let non_finite = ClientMessage::SetPlayerControl {
            operation_sequence: 2,
            operation_id: "zero-control".into(),
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(f64::NAN, 0.0, 0.0),
            angular_input: Vec3::ZERO,
            boost: false,
            dampeners: true,
            jump: false,
        };
        assert_eq!(
            runtime
                .state()
                .client_intent_fingerprint("player-local", &non_finite)
                .expect_err("non-finite intent data is not fingerprintable")
                .code(),
            "invalid_vector"
        );
    }

    #[test]
    fn lost_receipt_and_next_operation_survive_a_lower_frontier_destination() {
        use crate::cell_directory::{CellTransferRecord, MobileAggregateKind, TransferPhase};
        use crate::handoff::{
            PlayerTransferContext, prepare_eva_player_transfer, stage_committed_eva_import,
            stage_eva_player_quarantine,
        };

        let mut source = WorldState::genesis(8_011);
        source.fencing_token = 11;
        let first = sequenced_suit_message(1, "source-operation", false);
        let first_event = source
            .prepare_client_event_as("player-local", &first)
            .expect("source operation prepares");
        source
            .apply_event(&first_event)
            .expect("source operation commits");
        let first_receipt = source
            .processed_operation_record("player-local", 1)
            .expect("source receipt is retained")
            .receipt
            .clone();

        let source_key = celestial::cell_origin_key();
        let destination_key =
            celestial::neighbor_cell_key(&source_key, [1, 0, 0]).expect("destination cell derives");
        let boundary_address = celestial::address_from_origin_offset_um(
            &source.cell_address,
            [i128::from(celestial::CELL_EDGE_UM / 2), 0, 0],
        )
        .expect("boundary address canonicalizes");
        let boundary_position =
            celestial::local_position_from_address(&source.cell_address, &boundary_address)
                .expect("boundary position hydrates");
        let player = source
            .player
            .get_mut("player-local")
            .expect("source player exists");
        player.address = boundary_address;
        player.position = boundary_position;
        player.locomotion.kind = LocomotionKind::Eva;
        player.locomotion.support = None;
        player.locomotion.magnetic_boots_enabled = false;
        source
            .validate_player_roster()
            .expect("source operation history remains canonical at the boundary");

        let mut destination =
            WorldState::genesis_for_cell(8_011, &destination_key).expect("destination cell builds");
        destination.fencing_token = 17;
        assert!(
            destination.event_sequence < source.event_sequence,
            "proof exercises unrelated cell journal frontiers"
        );
        assert_eq!(
            source
                .client_intent_fingerprint("player-local", &first)
                .expect("source fingerprint derives"),
            destination
                .client_intent_fingerprint("player-local", &first)
                .expect("destination fingerprint derives"),
            "routing must not change one canonical client operation"
        );

        let context = PlayerTransferContext {
            transfer_id: "transfer-operation-frontier".into(),
            source_cell_key: source_key,
            destination_cell_key: destination_key,
            source_assignment_generation: 3,
            destination_assignment_generation: 5,
            source_fencing_token: 11,
            prior_placement_generation: 7,
            resulting_placement_generation: 8,
        };
        let package = prepare_eva_player_transfer(&source, "player-local", &context)
            .expect("player package carries source operation history");
        let (reserved_destination, receipt) =
            stage_eva_player_quarantine(&destination, destination.fencing_token, &package)
                .expect("destination reserves the package");
        let committed = CellTransferRecord {
            transfer_id: package.transfer_id.clone(),
            aggregate_id: package.aggregate_id.clone(),
            aggregate_kind: MobileAggregateKind::Player,
            source_cell_key: package.source_cell_key.clone(),
            source_cell_id: package.source_cell_id.clone(),
            destination_cell_key: package.destination_cell_key.clone(),
            destination_cell_id: package.destination_cell_id.clone(),
            source_assignment_generation: package.source_assignment_generation,
            destination_assignment_generation: package.destination_assignment_generation,
            prior_placement_generation: package.prior_placement_generation,
            resulting_placement_generation: package.resulting_placement_generation,
            package_hash: package.package_hash.clone(),
            quarantine_receipt_hash: Some(receipt.receipt_hash.clone()),
            phase: TransferPhase::Committed,
        };
        let mut imported =
            stage_committed_eva_import(&reserved_destination, &package, &receipt, &committed)
                .expect("committed player imports");
        let first_fingerprint = imported
            .client_intent_fingerprint("player-local", &first)
            .expect("imported retry fingerprints");
        assert_eq!(
            imported
                .validate_operation_attempt("player-local", 1, &first_fingerprint)
                .expect("lost source response reconciles"),
            Some(first_receipt.clone())
        );

        let second = sequenced_suit_message(2, "destination-operation", true);
        let second_event = imported
            .prepare_client_event_as("player-local", &second)
            .expect("next operation prepares on the destination");
        imported
            .apply_event(&second_event)
            .expect("next operation commits on the lower destination frontier");
        let second_record = imported
            .processed_operation_record("player-local", 2)
            .expect("destination receipt is retained");
        assert_eq!(second_record.receipt_origin_cell_id, imported.cell_id);
        assert_eq!(second_record.receipt.operation_sequence, 2);
        assert_eq!(second_record.receipt.event_sequence, 1);
        assert_eq!(
            imported
                .processed_operation_record("player-local", 1)
                .expect("source receipt remains retained")
                .receipt,
            first_receipt
        );
        imported
            .validate_player_roster()
            .expect("cross-cell operation history remains canonical");
    }

    #[test]
    fn operation_compaction_and_exact_retry_survive_restart() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        let expected_compaction_hash: String;
        let final_message = sequenced_suit_message(129, "bounded-129", false);
        let final_receipt;
        {
            let mut runtime =
                Runtime::open(directory.path(), 143, 1).expect("runtime opens for compaction");
            let mut last_receipt = None;
            for operation_sequence in 1..=129 {
                let message = sequenced_suit_message(
                    operation_sequence,
                    format!("bounded-{operation_sequence}"),
                    operation_sequence % 2 == 0,
                );
                last_receipt = Some(runtime.execute(&message).expect("operation commits"));
            }
            final_receipt = last_receipt.expect("the bounded campaign is nonempty");
            let history = &runtime.state().processed_operations["player-local"];
            assert_eq!(history.committed_through, 129);
            assert_eq!(history.compacted_through, 1);
            assert_eq!(history.retained.len(), PROCESSED_OPERATION_RETENTION_LIMIT);
            assert_eq!(
                history.retained.first_key_value().map(|(key, _)| *key),
                Some(2)
            );
            assert!(valid_blake3_hex(&history.compacted_history_hash));
            expected_compaction_hash = history.compacted_history_hash.clone();
            expected_hash = runtime.state().state_hash();
        }

        let mut recovered =
            Runtime::open(directory.path(), 143, 1).expect("compacted runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        let history = &recovered.state().processed_operations["player-local"];
        assert_eq!(history.compacted_history_hash, expected_compaction_hash);
        assert_eq!(history.committed_through, 129);
        assert_eq!(history.compacted_through, 1);

        assert_eq!(
            rejected_code(
                recovered
                    .execute(&sequenced_suit_message(1, "bounded-1", false))
                    .expect_err("a compacted retry cannot reconstruct its exact receipt")
            ),
            "operation_already_committed"
        );
        assert_eq!(
            recovered
                .execute(&final_message)
                .expect("a retained exact retry returns its durable receipt"),
            final_receipt
        );
        assert_eq!(recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn operation_frontier_recovers_across_append_failpoints() {
        for (failpoint, durable) in [
            (AppendFailpoint::BeforeWrite, false),
            (AppendFailpoint::AfterSync, true),
        ] {
            let directory = tempdir().expect("tempdir");
            let message = sequenced_suit_message(1, format!("failpoint-{durable}"), false);
            {
                let mut runtime = Runtime::open(directory.path(), 149, 100).expect("runtime opens");
                runtime.store.set_append_failpoint(failpoint);
                assert!(matches!(
                    runtime.execute(&message),
                    Err(RuntimeError::Persistence(
                        PersistenceError::InjectedFailure(_)
                    ))
                ));
                assert!(runtime.is_halted());
                assert_eq!(runtime.state().last_operation_sequence("player-local"), 0);
            }

            let mut recovered =
                Runtime::open(directory.path(), 149, 100).expect("runtime recovers");
            assert_eq!(
                recovered.state().last_operation_sequence("player-local"),
                u64::from(durable)
            );
            let receipt = recovered
                .execute(&message)
                .expect("the same operation is accepted or retried exactly");
            assert_eq!(receipt.operation_sequence, 1);
            assert_eq!(recovered.state().last_operation_sequence("player-local"), 1);
            assert_eq!(recovered.state().event_sequence, 1);
        }
    }

    #[test]
    fn malformed_operation_history_halts_before_intent_processing() {
        let mut runtime = runtime();
        runtime.state.processed_operations.insert(
            "player-local".into(),
            ActorOperationHistory {
                committed_through: 2,
                compacted_through: 0,
                compacted_history_hash: String::new(),
                retained: BTreeMap::new(),
            },
        );
        assert!(matches!(
            runtime.execute(&sequenced_suit_message(3, "must-not-run", false)),
            Err(RuntimeError::CanonicalInvariant(_))
        ));
        assert!(runtime.is_halted());
        assert_eq!(runtime.state().event_sequence, 0);
    }

    #[test]
    fn replay_rejects_trim_empty_operation_id_before_any_state_mutation() {
        let state = WorldState::genesis(151);
        let mut event = state
            .prepare_client_event(&sequenced_suit_message(1, "canonical-id", false))
            .expect("canonical suit event prepares");
        let whitespace_message = sequenced_suit_message(1, " \t ", false);
        event.operation_id = Some(" \t ".into());
        event.intent_fingerprint = Some(
            state
                .client_intent_fingerprint("player-local", &whitespace_message)
                .expect("typed whitespace operation ID fingerprints"),
        );
        event.event_hash = event.calculate_hash();

        let mut candidate = state.clone();
        let before = candidate.clone();
        let before_bytes = serde_json::to_vec(&candidate).expect("canonical state serializes");
        let error = candidate
            .apply_event(&event)
            .expect_err("trim-empty replay operation ID rejects");
        assert_eq!(error.code(), "replay_actor_envelope_invalid");
        assert_eq!(candidate, before);
        assert_eq!(
            serde_json::to_vec(&candidate).expect("rejected state serializes"),
            before_bytes
        );
    }

    fn move_player_near_grid(runtime: &mut Runtime) {
        aim_player_for_build(runtime, STARTER_GRID_ID, IVec3::new(0, 1, 0));
    }

    fn copy_primary_tool_pose_to(runtime: &mut Runtime, player_id: &str) {
        let primary = runtime.state.player.primary().clone();
        let player = runtime
            .state
            .player
            .get_mut(player_id)
            .expect("tool-pose target player exists");
        player.address = primary.address;
        player.position = primary.position;
        player.orientation = primary.orientation;
        player.linear_velocity = primary.linear_velocity;
        player.angular_velocity = primary.angular_velocity;
        player.surface_contact = primary.surface_contact;
        player.locomotion = primary.locomotion;
        runtime.rebuild_physics_for_test();
    }

    fn normalized(value: Vec3) -> Vec3 {
        value * value.magnitude().recip()
    }

    fn orientation_from_forward(forward: Vec3) -> Quat {
        let forward = normalized(forward);
        // Shortest rotation from Godot's local forward (-Z) to the target.
        let dot = -forward.z;
        if dot < -1.0 + 1.0e-9 {
            return Quat::new(0.0, 1.0, 0.0, 0.0);
        }
        let x = forward.y;
        let y = -forward.x;
        let w = 1.0 + dot;
        let inverse_length = x.mul_add(x, y.mul_add(y, w * w)).sqrt().recip();
        Quat::new(
            (x * inverse_length) as f32,
            (y * inverse_length) as f32,
            0.0,
            (w * inverse_length) as f32,
        )
    }

    fn aim_player_from_face(player: &mut Player, target: Vec3, outward_face: Vec3) {
        let eye_offset = content::manifest().character.eye_height_m
            - content::manifest().character.standing_height_m * 0.5;
        let eye = target + normalized(outward_face) * 4.0;
        set_test_player_position(player, eye - Vec3::new(0.0, eye_offset, 0.0));
        player.orientation = orientation_from_forward(target - eye);
        player.linear_velocity = Vec3::ZERO;
        player.angular_velocity = Vec3::ZERO;
        player.locomotion.kind = LocomotionKind::Airborne;
        player.locomotion.up = Vec3::new(0.0, 1.0, 0.0);
        player.locomotion.view_pitch_radians = 0.0;
    }

    fn aim_player_for_build(runtime: &mut Runtime, grid_id: &str, coordinate: IVec3) {
        let grid = runtime.state.grids[grid_id].clone();
        let mut aimed = None;
        for existing in coordinate
            .neighbors()
            .into_iter()
            .filter(|neighbor| grid.block_at(*neighbor).is_some())
        {
            let local_face = IVec3::new(
                coordinate.x - existing.x,
                coordinate.y - existing.y,
                coordinate.z - existing.z,
            );
            let world_face = grid.orientation.rotate(Vec3::new(
                f64::from(local_face.x),
                f64::from(local_face.y),
                f64::from(local_face.z),
            ));
            let mut candidate = runtime.state.player.primary().clone();
            aim_player_from_face(&mut candidate, grid.world_position(existing), world_face);
            let hit = closest_tool_hit(&candidate, &runtime.state.voxels, &runtime.state.grids);
            if matches!(
                hit,
                Some(ToolHit {
                    target: ToolTarget::Block {
                        ref grid_id,
                        coordinate: targeted,
                        ..
                    },
                    local_face: Some(face),
                    ..
                }) if grid_id == &grid.grid_id
                    && targeted == existing
                    && IVec3::new(
                        targeted.x + face.x,
                        targeted.y + face.y,
                        targeted.z + face.z,
                    ) == coordinate
            ) {
                aimed = Some(candidate);
                break;
            }
        }
        *runtime.state.player.primary_mut() =
            aimed.expect("build fixture has one visible face-connected existing block");
        runtime.rebuild_physics_for_test();
    }

    fn exposed_voxel_face(voxels: &VoxelField, coordinate: IVec3) -> Option<IVec3> {
        coordinate
            .neighbors()
            .into_iter()
            .find(|neighbor| !voxels.occupied.contains(neighbor))
            .map(|neighbor| {
                IVec3::new(
                    neighbor.x - coordinate.x,
                    neighbor.y - coordinate.y,
                    neighbor.z - coordinate.z,
                )
            })
    }

    fn aim_player_at_voxel(runtime: &mut Runtime, player_id: &str, coordinate: IVec3) {
        let target = Vec3::new(
            f64::from(coordinate.x),
            f64::from(coordinate.y),
            f64::from(coordinate.z),
        );
        let baseline = runtime
            .state
            .player
            .get(player_id)
            .expect("aimed fixture actor exists")
            .clone();
        let mut aimed = None;
        for neighbor in coordinate
            .neighbors()
            .into_iter()
            .filter(|neighbor| !runtime.state.voxels.occupied.contains(neighbor))
        {
            let face = IVec3::new(
                neighbor.x - coordinate.x,
                neighbor.y - coordinate.y,
                neighbor.z - coordinate.z,
            );
            let mut candidate = baseline.clone();
            aim_player_from_face(
                &mut candidate,
                target,
                Vec3::new(f64::from(face.x), f64::from(face.y), f64::from(face.z)),
            );
            if matches!(
                closest_tool_hit(&candidate, &runtime.state.voxels, &runtime.state.grids),
                Some(ToolHit {
                    target: ToolTarget::Voxel { coordinate: targeted },
                    local_face: Some(_),
                    ..
                }) if targeted == coordinate
            ) {
                aimed = Some(candidate);
                break;
            }
        }
        *runtime
            .state
            .player
            .get_mut(player_id)
            .expect("aimed fixture actor exists") =
            aimed.expect("mining fixture voxel has a visible exposed face");
        runtime.rebuild_physics_for_test();
    }

    fn aim_player_at_block(runtime: &mut Runtime, grid_id: &str, block_id: &str) {
        let grid = runtime.state.grids[grid_id].clone();
        let block = grid.blocks[block_id].clone();
        let baseline = runtime.state.player.primary().clone();
        let mut aimed = None;
        for neighbor in block
            .coordinate
            .neighbors()
            .into_iter()
            .filter(|neighbor| grid.block_at(*neighbor).is_none())
        {
            let face = Vec3::new(
                f64::from(neighbor.x - block.coordinate.x),
                f64::from(neighbor.y - block.coordinate.y),
                f64::from(neighbor.z - block.coordinate.z),
            );
            let mut candidate = baseline.clone();
            aim_player_from_face(
                &mut candidate,
                grid.world_position(block.coordinate),
                grid.orientation.rotate(face),
            );
            if matches!(
                closest_tool_hit(&candidate, &runtime.state.voxels, &runtime.state.grids),
                Some(ToolHit {
                    target: ToolTarget::Block {
                        ref grid_id,
                        ref block_id,
                        ..
                    },
                    local_face: Some(_),
                    ..
                }) if grid_id == &grid.grid_id && block_id == &block.block_id
            ) {
                aimed = Some(candidate);
                break;
            }
        }
        *runtime.state.player.primary_mut() =
            aimed.expect("hand-tool fixture block has one visible exposed face");
        runtime.rebuild_physics_for_test();
    }

    fn aim_player_at_block_preserving_locomotion(
        runtime: &mut Runtime,
        grid_id: &str,
        block_id: &str,
    ) {
        let preserved_locomotion = runtime.state.player.locomotion.clone();
        aim_player_at_block(runtime, grid_id, block_id);
        runtime.state.player.locomotion = preserved_locomotion;
        runtime.state.player.locomotion.view_pitch_radians = 0.0;
    }

    fn restore_player_pose_after_tool_fixture(runtime: &mut Runtime, prior: &Player) {
        runtime.state.player.address = prior.address.clone();
        runtime.state.player.position = prior.position;
        runtime.state.player.orientation = prior.orientation;
        runtime.state.player.linear_velocity = prior.linear_velocity;
        runtime.state.player.angular_velocity = prior.angular_velocity;
        runtime.state.player.surface_contact = prior.surface_contact;
        runtime.rebuild_physics_for_test();
    }

    fn add_remote_player(runtime: &mut Runtime) {
        runtime
            .admit_development_player("player-remote")
            .expect("remote development player admits");
        runtime
            .state
            .player
            .get_mut("player-remote")
            .expect("remote development player exists")
            .linear_velocity = Vec3::new(0.25, 0.0, 0.0);
        runtime.rebuild_physics_for_test();
    }

    #[test]
    fn operation_ids_and_control_frontiers_are_scoped_per_player() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 68, 1_000).expect("runtime opens");
        add_remote_player(&mut runtime);
        runtime
            .persist_snapshot()
            .expect("two-player baseline persists");
        let control = |player: &Player, linear_input: Vec3| ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "shared-operation-id".into(),
            movement_epoch: player.movement_epoch,
            input_sequence: 1,
            linear_input,
            angular_input: Vec3::ZERO,
            boost: false,
            dampeners: true,
            jump: false,
        };
        let local_intent = control(
            runtime
                .state
                .player
                .get("player-local")
                .expect("local exists"),
            Vec3::new(1.0, 0.0, 0.0),
        );
        let remote_intent = control(
            runtime
                .state
                .player
                .get("player-remote")
                .expect("remote exists"),
            Vec3::new(-1.0, 0.0, 0.0),
        );

        let local_receipt = runtime
            .execute_next_as_for_fixture("player-local", &local_intent)
            .expect("local control commits");
        let remote_receipt = runtime
            .execute_next_as_for_fixture("player-remote", &remote_intent)
            .expect("remote control with the same operation ID commits independently");
        assert_ne!(local_receipt.event_sequence, remote_receipt.event_sequence);
        assert_eq!(
            runtime
                .state
                .player
                .get("player-local")
                .unwrap()
                .last_received_input_sequence,
            1
        );
        assert_eq!(
            runtime
                .state
                .player
                .get("player-remote")
                .unwrap()
                .last_received_input_sequence,
            1
        );
        assert_eq!(
            runtime
                .state
                .processed_operation("player-local", "shared-operation-id"),
            Some(&local_receipt)
        );
        assert_eq!(
            runtime
                .state
                .processed_operation("player-remote", "shared-operation-id"),
            Some(&remote_receipt)
        );
        let accepted_hash = runtime.state().state_hash();
        let accepted_sequence = runtime.state().event_sequence;
        assert_eq!(
            runtime
                .execute_next_as_for_fixture("player-local", &local_intent)
                .expect("local retry is idempotent"),
            local_receipt
        );
        assert_eq!(
            runtime
                .execute_next_as_for_fixture("player-remote", &remote_intent)
                .expect("remote retry is idempotent"),
            remote_receipt
        );
        assert_eq!(runtime.state().state_hash(), accepted_hash);
        assert_eq!(runtime.state().event_sequence, accepted_sequence);

        let primary_mode_before = (
            runtime.state().player.helmet_closed,
            runtime.state().player.jetpack_enabled,
            runtime.state().player.locomotion.magnetic_boots_enabled,
        );
        let remote_suit_receipt = runtime
            .execute_next_as_for_fixture(
                "player-remote",
                &ClientMessage::SetSuitMode {
                    operation_sequence: 0,
                    operation_id: "remote-suit-disabled".into(),
                    helmet_closed: false,
                    jetpack_enabled: true,
                    magnetic_boots_enabled: false,
                },
            )
            .expect("secondary suit mode targets its authenticated actor");
        assert_eq!(
            (
                runtime.state().player.helmet_closed,
                runtime.state().player.jetpack_enabled,
                runtime.state().player.locomotion.magnetic_boots_enabled,
            ),
            primary_mode_before
        );
        let remote = runtime.state().player.get("player-remote").unwrap();
        assert!(!remote.helmet_closed);
        assert!(remote.jetpack_enabled);
        assert!(!remote.locomotion.magnetic_boots_enabled);
        let lifecycle_hash = runtime.state().state_hash();
        let lifecycle_sequence = runtime.state().event_sequence;
        assert_eq!(
            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::SetSuitMode {
                        operation_sequence: 0,
                        operation_id: "remote-suit-disabled".into(),
                        helmet_closed: false,
                        jetpack_enabled: true,
                        magnetic_boots_enabled: false,
                    },
                )
                .expect("secondary suit retry is actor-scoped and idempotent"),
            remote_suit_receipt
        );
        assert_eq!(runtime.state().state_hash(), lifecycle_hash);
        assert_eq!(runtime.state().event_sequence, lifecycle_sequence);

        drop(runtime);
        let recovered = Runtime::open(directory.path(), 68, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), lifecycle_hash);
        assert_eq!(
            recovered
                .state()
                .processed_operation("player-remote", "shared-operation-id"),
            Some(&remote_receipt)
        );
    }

    #[test]
    fn actor_owned_industry_and_engineering_are_isolated_and_recover_exactly() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime =
                Runtime::open(directory.path(), 169, 5).expect("authority runtime opens");
            runtime
                .admit_development_player("player-remote")
                .expect("secondary actor admits");
            runtime
                .state
                .grids
                .get_mut(STARTER_GRID_ID)
                .expect("starter grid exists")
                .owner_player_id = "player-remote".into();
            runtime
                .state
                .grids
                .get_mut(STARTER_INDUSTRY_GRID_ID)
                .expect("starter industry grid exists")
                .owner_player_id = "player-remote".into();
            runtime
                .state
                .inventories
                .get_mut(PLAYER_INVENTORY_ID)
                .expect("primary inventory exists")
                .contents
                .components -= 4;
            runtime
                .state
                .inventories
                .get_mut("inventory-player-remote")
                .expect("secondary inventory exists")
                .contents = InventoryContents {
                ore: 2,
                refined_material: 1,
                components: 4,
            };
            runtime.state.ledger.genesis_ore += 2;
            runtime.state.ledger.genesis_refined += 1;
            assert!(runtime.state().conservation().valid);
            runtime
                .persist_snapshot()
                .expect("owned-grid authority fixture persists");

            let denied = [
                (
                    ClientMessage::RefineOre {
                        operation_sequence: 0,
                        operation_id: "remote-refine-primary".into(),
                        inventory_id: PLAYER_INVENTORY_ID.into(),
                        batches: 1,
                    },
                    "physical_machine_required",
                ),
                (
                    ClientMessage::CraftComponent {
                        operation_sequence: 0,
                        operation_id: "remote-craft-primary".into(),
                        inventory_id: PLAYER_INVENTORY_ID.into(),
                        quantity: 1,
                    },
                    "physical_machine_required",
                ),
                (
                    ClientMessage::TransferInventory {
                        operation_sequence: 0,
                        operation_id: "remote-transfer-from-primary".into(),
                        source_inventory_id: PLAYER_INVENTORY_ID.into(),
                        destination_inventory_id: "inventory-player-remote".into(),
                        resource: ResourceKind::Component,
                        quantity: 1,
                    },
                    "inventory_access_denied",
                ),
                (
                    ClientMessage::TransferInventory {
                        operation_sequence: 0,
                        operation_id: "remote-transfer-to-primary".into(),
                        source_inventory_id: "inventory-player-remote".into(),
                        destination_inventory_id: PLAYER_INVENTORY_ID.into(),
                        resource: ResourceKind::Component,
                        quantity: 1,
                    },
                    "inventory_access_denied",
                ),
            ];
            for (message, expected_code) in denied {
                let before = runtime.state().state_hash();
                let error = runtime
                    .execute_next_as_for_fixture("player-remote", &message)
                    .expect_err("cross-owner inventory intent rejects");
                assert!(matches!(
                    error,
                    RuntimeError::Intent(IntentError::Rejected { ref code, .. })
                        if code == expected_code
                ));
                assert_eq!(runtime.state().state_hash(), before);
            }

            let foreign_grid_intents = [
                ClientMessage::BuildBlock {
                    operation_sequence: 0,
                    operation_id: "primary-build-foreign".into(),
                    grid_id: STARTER_GRID_ID.into(),
                    coordinate: IVec3::new(0, 1, 0),
                    kind: BlockKind::Structural,
                    orientation: 0,
                },
                ClientMessage::WeldBlock {
                    operation_sequence: 0,
                    operation_id: "primary-weld-foreign".into(),
                    grid_id: STARTER_GRID_ID.into(),
                    block_id: "block-core".into(),
                },
                ClientMessage::SetGridControl {
                    operation_sequence: 0,
                    operation_id: "primary-control-foreign".into(),
                    grid_id: STARTER_GRID_ID.into(),
                    linear_input: Vec3::new(0.25, 0.0, 0.0),
                    angular_input: Vec3::ZERO,
                    dampeners: true,
                },
                ClientMessage::ToggleGridAnchor {
                    operation_sequence: 0,
                    operation_id: "primary-anchor-foreign".into(),
                    grid_id: STARTER_GRID_ID.into(),
                },
            ];
            for message in foreign_grid_intents {
                let before = runtime.state().state_hash();
                let error = runtime
                    .execute_next_as_for_fixture("player-local", &message)
                    .expect_err("foreign constructive grid intent rejects");
                assert!(matches!(
                    error,
                    RuntimeError::Intent(IntentError::Rejected { ref code, .. })
                        if code == "grid_access_denied"
                ));
                assert_eq!(runtime.state().state_hash(), before);
            }

            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::TransferInventory {
                        operation_sequence: 0,
                        operation_id: "remote-haul-ore-to-cargo".into(),
                        source_inventory_id: "inventory-player-remote".into(),
                        destination_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                        resource: ResourceKind::Ore,
                        quantity: 2,
                    },
                )
                .expect("secondary actor hauls ore into its production cargo");
            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::QueueProduction {
                        operation_sequence: 0,
                        operation_id: "remote-refine-owned".into(),
                        machine_block_id: "block-refinery".into(),
                        recipe: ProductionRecipeKind::Refining,
                        batches: 1,
                        source_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                        destination_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                    },
                )
                .expect("secondary actor queues its connected refinery");
            advance_whole_seconds(&mut runtime, 2);
            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::QueueProduction {
                        operation_sequence: 0,
                        operation_id: "remote-craft-owned".into(),
                        machine_block_id: "block-assembler".into(),
                        recipe: ProductionRecipeKind::Component,
                        batches: 1,
                        source_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                        destination_inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                    },
                )
                .expect("secondary actor queues its connected assembler");
            advance_whole_seconds(&mut runtime, 2);
            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::TransferInventory {
                        operation_sequence: 0,
                        operation_id: "remote-transfer-to-owned-cargo".into(),
                        source_inventory_id: "inventory-player-remote".into(),
                        destination_inventory_id: "inventory-cargo-starter".into(),
                        resource: ResourceKind::Component,
                        quantity: 1,
                    },
                )
                .expect("secondary actor deposits into its completed cargo");
            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::TransferInventory {
                        operation_sequence: 0,
                        operation_id: "remote-transfer-from-owned-cargo".into(),
                        source_inventory_id: "inventory-cargo-starter".into(),
                        destination_inventory_id: "inventory-player-remote".into(),
                        resource: ResourceKind::Component,
                        quantity: 1,
                    },
                )
                .expect("secondary actor withdraws from its completed cargo");
            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::SetGridControl {
                        operation_sequence: 0,
                        operation_id: "remote-control-owned-grid".into(),
                        grid_id: STARTER_GRID_ID.into(),
                        linear_input: Vec3::new(0.25, 0.0, 0.0),
                        angular_input: Vec3::ZERO,
                        dampeners: true,
                    },
                )
                .expect("secondary owner controls its powered released grid");

            aim_player_for_build(&mut runtime, STARTER_GRID_ID, IVec3::new(0, 1, 0));
            copy_primary_tool_pose_to(&mut runtime, "player-remote");
            runtime
                .execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::BuildBlock {
                        operation_sequence: 0,
                        operation_id: "remote-build-owned-grid".into(),
                        grid_id: STARTER_GRID_ID.into(),
                        coordinate: IVec3::new(0, 1, 0),
                        kind: BlockKind::Structural,
                        orientation: 0,
                    },
                )
                .expect("secondary owner places a frame using its carried components");
            let frame_id = runtime.state().grids[STARTER_GRID_ID]
                .block_at(IVec3::new(0, 1, 0))
                .expect("secondary frame exists")
                .block_id
                .clone();
            for stage in 0..3 {
                aim_player_at_block(&mut runtime, STARTER_GRID_ID, &frame_id);
                copy_primary_tool_pose_to(&mut runtime, "player-remote");
                runtime
                    .execute_next_as_for_fixture(
                        "player-remote",
                        &ClientMessage::WeldBlock {
                            operation_sequence: 0,
                            operation_id: format!("remote-weld-owned-grid-{stage}"),
                            grid_id: STARTER_GRID_ID.into(),
                            block_id: frame_id.clone(),
                        },
                    )
                    .expect("secondary owner welds its frame");
            }

            let local_experience_before = runtime.state().player.primary().experience;
            aim_player_at_block(&mut runtime, STARTER_GRID_ID, "block-deck-e");
            runtime
                .execute_next_as_for_fixture(
                    "player-local",
                    &ClientMessage::DamageBlock {
                        operation_sequence: 0,
                        operation_id: "primary-damage-foreign-grid".into(),
                        grid_id: STARTER_GRID_ID.into(),
                        block_id: "block-deck-e".into(),
                    },
                )
                .expect("non-owner PvP damage remains legal in the unsecured cell");

            let primary = runtime.state().player.primary();
            let remote = runtime
                .state()
                .player
                .get("player-remote")
                .expect("secondary actor remains present");
            assert_eq!(primary.experience, local_experience_before);
            assert_eq!(primary.career, CareerSnapshot::default());
            assert_eq!(remote.experience, 55);
            assert_eq!(remote.career.refining_batches, 1);
            assert_eq!(remote.career.components_crafted, 1);
            assert_eq!(remote.career.blocks_built, 1);
            assert_eq!(
                runtime.state().grids[STARTER_GRID_ID].owner_player_id,
                "player-remote"
            );
            assert!(runtime.state().conservation().valid);
            runtime
                .persist_snapshot()
                .expect("actor-owned progression snapshot persists");
            expected_hash = runtime.state().state_hash();
        }

        let recovered =
            Runtime::open(directory.path(), 169, 5).expect("actor-owned progression recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert_eq!(
            recovered.state().grids[STARTER_GRID_ID].owner_player_id,
            "player-remote"
        );
    }

    fn stationary_player_outcomes(state: &WorldState, step_count: u8) -> Vec<PlayerPhysicsOutcome> {
        state
            .player
            .iter()
            .filter(|(_, player)| matches!(player.life_state, PlayerLifeState::Alive))
            .map(|(_, player)| {
                let resulting_tick = state.simulation_tick + u64::from(step_count);
                let lease_active = resulting_tick < player.control_expires_at_simulation_tick;
                PlayerPhysicsOutcome {
                    player_id: player.player_id.clone(),
                    address: player.address.clone(),
                    position: player.position,
                    orientation: player.orientation,
                    linear_velocity: player.linear_velocity,
                    angular_velocity: player.angular_velocity,
                    locomotion: player.locomotion.clone(),
                    surface_contact: false,
                    control_linear_input: if lease_active {
                        player.control_linear_input
                    } else {
                        Vec3::ZERO
                    },
                    control_angular_input: if lease_active {
                        player.control_angular_input
                    } else {
                        Vec3::ZERO
                    },
                    boost: player.boost && lease_active,
                    jump: player.jump && lease_active,
                    dampeners: player.dampeners || !lease_active,
                    control_expires_at_simulation_tick: player.control_expires_at_simulation_tick,
                }
            })
            .collect()
    }

    fn reachable_voxel(runtime: &mut Runtime) -> IVec3 {
        let coordinate = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coordinate| exposed_voxel_face(&runtime.state.voxels, *coordinate).is_some())
            .expect("exposed voxel exists");
        aim_player_at_voxel(runtime, "player-local", coordinate);
        coordinate
    }

    #[test]
    fn hand_tool_prepare_rejects_nonvisible_targets_and_wrong_build_face_without_mutation() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let assert_rejected_unchanged =
            |runtime: &mut Runtime, message: ClientMessage, expected_code: &str| {
                let before_hash = runtime.state().state_hash();
                let before_sequence = runtime.state().event_sequence;
                let result = runtime.execute_next_for_fixture(&message);
                assert!(matches!(
                    result,
                    Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                        if code == expected_code
                ));
                assert_eq!(runtime.state().state_hash(), before_hash);
                assert_eq!(runtime.state().event_sequence, before_sequence);
            };

        assert_rejected_unchanged(
            &mut runtime,
            ClientMessage::DamageBlock {
                operation_sequence: 0,
                operation_id: "occluded-damage-prepare".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: "block-deck-e".into(),
            },
            "block_not_targeted",
        );
        assert_rejected_unchanged(
            &mut runtime,
            ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "wrong-build-face-prepare".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, -1, 0),
                kind: BlockKind::Structural,
                orientation: 0,
            },
            "build_face_not_targeted",
        );

        let visible = reachable_voxel(&mut runtime);
        let different = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coordinate| *coordinate != visible)
            .expect("asteroid has another voxel");
        assert_rejected_unchanged(
            &mut runtime,
            ClientMessage::MineVoxel {
                operation_sequence: 0,
                operation_id: "occluded-mining-prepare".into(),
                coordinate: different,
            },
            "voxel_not_targeted",
        );
    }

    #[test]
    fn mining_prepare_accepts_exactly_nine_meter_surface_range_and_rejects_beyond_it() {
        let mut state = runtime().state().clone();
        state.voxels = VoxelField {
            occupied: BTreeSet::from([IVec3::ZERO]),
            ferrite_ore: BTreeSet::new(),
        };
        state.grids.clear();
        let eye_offset = content::manifest().character.eye_height_m
            - content::manifest().character.standing_height_m * 0.5;
        set_test_player_position(&mut state.player, Vec3::new(0.0, -eye_offset, 9.5));
        state.player.orientation = Quat::IDENTITY;
        state.player.locomotion.kind = LocomotionKind::Airborne;
        state.player.locomotion.up = Vec3::new(0.0, 1.0, 0.0);
        state.player.locomotion.view_pitch_radians = 0.0;
        state
            .prepare_next_client_event_for_fixture(&ClientMessage::MineVoxel {
                operation_sequence: 0,
                operation_id: "exact-nine-meter-surface".into(),
                coordinate: IVec3::ZERO,
            })
            .expect("a surface exactly nine meters from the eye is targetable");

        let beyond_range = state.player.position + Vec3::new(0.0, 0.0, 0.000_001);
        set_test_player_position(&mut state.player, beyond_range);
        let before_hash = state.state_hash();
        let error = state
            .prepare_next_client_event_for_fixture(&ClientMessage::MineVoxel {
                operation_sequence: 0,
                operation_id: "beyond-nine-meter-surface".into(),
                coordinate: IVec3::ZERO,
            })
            .expect_err("a surface beyond the inclusive range rejects");
        assert_eq!(error.code(), "voxel_not_targeted");
        assert_eq!(state.state_hash(), before_hash);
    }

    #[test]
    fn hand_tool_replay_rejects_target_and_actor_substitution_before_mutation() {
        let mut runtime = runtime();
        runtime
            .admit_development_player("player-remote")
            .expect("secondary fixture actor admits");
        move_player_near_grid(&mut runtime);
        aim_player_at_block(&mut runtime, STARTER_GRID_ID, "block-core");
        let state = runtime.state().clone();

        let damage = state
            .prepare_next_client_event_for_fixture(&ClientMessage::DamageBlock {
                operation_sequence: 0,
                operation_id: "canonical-targeted-damage".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: "block-core".into(),
            })
            .expect("visible damage event prepares");
        let reject = |event: &CanonicalEvent, expected_code: &str, state: &WorldState| {
            let mut candidate = state.clone();
            let before_hash = candidate.state_hash();
            let error = candidate
                .apply_event(event)
                .expect_err("forged tool replay rejects");
            assert_eq!(error.code(), expected_code);
            assert_eq!(candidate.state_hash(), before_hash);
        };

        let mut wrong_damage_target = damage.clone();
        let EventPayload::BlockDamaged { block_id, .. } = &mut wrong_damage_target.payload else {
            unreachable!();
        };
        *block_id = "block-deck-e".into();
        wrong_damage_target.event_hash = wrong_damage_target.calculate_hash();
        reject(
            &wrong_damage_target,
            "replay_intent_fingerprint_mismatch",
            &state,
        );

        let mut wrong_damage_amount = damage.clone();
        let EventPayload::BlockDamaged {
            damage: forged_damage,
            ..
        } = &mut wrong_damage_amount.payload
        else {
            unreachable!();
        };
        *forged_damage = 0;
        wrong_damage_amount.event_hash = wrong_damage_amount.calculate_hash();
        reject(&wrong_damage_amount, "replay_damage_amount_invalid", &state);

        let mut wrong_damage_actor = damage;
        wrong_damage_actor.actor_player_id = Some("player-remote".into());
        wrong_damage_actor.event_hash = wrong_damage_actor.calculate_hash();
        reject(
            &wrong_damage_actor,
            "replay_intent_fingerprint_mismatch",
            &state,
        );

        let mut build_runtime = runtime;
        move_player_near_grid(&mut build_runtime);
        let build_state = build_runtime.state().clone();
        let build = build_state
            .prepare_next_client_event_for_fixture(&ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "canonical-targeted-build".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, 1, 0),
                kind: BlockKind::Structural,
                orientation: 0,
            })
            .expect("visible construction event prepares");
        let mut wrong_build_face = build.clone();
        let EventPayload::BlockBuilt { block, .. } = &mut wrong_build_face.payload else {
            unreachable!();
        };
        block.coordinate = IVec3::new(0, -1, 0);
        wrong_build_face.event_hash = wrong_build_face.calculate_hash();
        reject(
            &wrong_build_face,
            "replay_intent_fingerprint_mismatch",
            &build_state,
        );

        let mut wrong_build_actor = build.clone();
        wrong_build_actor.actor_player_id = Some("player-remote".into());
        wrong_build_actor.event_hash = wrong_build_actor.calculate_hash();
        reject(
            &wrong_build_actor,
            "replay_intent_fingerprint_mismatch",
            &build_state,
        );

        let mut post_build_state = build_state;
        post_build_state
            .apply_event(&build)
            .expect("canonical visible frame replays");
        let frame_id = post_build_state.grids[STARTER_GRID_ID]
            .block_at(IVec3::new(0, 1, 0))
            .expect("frame exists")
            .block_id
            .clone();
        let weld = post_build_state
            .prepare_next_client_event_for_fixture(&ClientMessage::WeldBlock {
                operation_sequence: 0,
                operation_id: "canonical-targeted-weld".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: frame_id,
            })
            .expect("visible weld event prepares");
        let mut wrong_weld_target = weld;
        let EventPayload::BlockWelded { block_id, .. } = &mut wrong_weld_target.payload else {
            unreachable!();
        };
        *block_id = "block-core".into();
        wrong_weld_target.event_hash = wrong_weld_target.calculate_hash();
        reject(
            &wrong_weld_target,
            "replay_intent_fingerprint_mismatch",
            &post_build_state,
        );
    }

    fn expected_physics_fingerprint(state: &WorldState) -> Vec<(String, Vec<String>)> {
        let mut fingerprint = physics_body_specs(state)
            .into_iter()
            .map(|spec| {
                let mut colliders = spec
                    .colliders
                    .into_iter()
                    .map(|collider| collider.collider_id)
                    .chain(
                        spec.sphere_colliders
                            .into_iter()
                            .map(|collider| collider.collider_id),
                    )
                    .chain(
                        spec.capsule_colliders
                            .into_iter()
                            .map(|collider| collider.collider_id),
                    )
                    .collect::<Vec<_>>();
                colliders.sort();
                (spec.body_id, colliders)
            })
            .collect::<Vec<_>>();
        fingerprint.sort_by(|left, right| left.0.cmp(&right.0));
        fingerprint
    }

    fn weld_to_completion(runtime: &mut Runtime, coordinate: IVec3, prefix: &str) {
        loop {
            let block = runtime.state().grids[STARTER_GRID_ID]
                .block_at(coordinate)
                .expect("weld target exists")
                .clone();
            if block.is_complete() {
                break;
            }
            aim_player_at_block(runtime, STARTER_GRID_ID, &block.block_id);
            runtime
                .execute_next_for_fixture(&ClientMessage::WeldBlock {
                    operation_sequence: 0,
                    operation_id: format!("{prefix}-{}", block.health),
                    grid_id: STARTER_GRID_ID.into(),
                    block_id: block.block_id,
                })
                .expect("weld stage accepted");
        }
    }

    fn test_grid(
        grid_id: &str,
        position: Vec3,
        linear_velocity: Vec3,
        blocks: impl IntoIterator<Item = Block>,
    ) -> Grid {
        Grid {
            grid_id: grid_id.into(),
            owner_player_id: "player-local".into(),
            anchor_reward_eligible: true,
            address: celestial::address_from_local_position(
                &celestial::cell_origin_address(),
                position,
            )
            .expect("test grid position has a canonical address"),
            position,
            orientation: Quat::IDENTITY,
            linear_velocity,
            angular_velocity: Vec3::ZERO,
            control_linear_input: Vec3::ZERO,
            control_angular_input: Vec3::ZERO,
            dampeners: false,
            anchored: false,
            blocks: blocks
                .into_iter()
                .map(|block| (block.block_id.clone(), block))
                .collect(),
        }
    }

    fn replace_with_physics_fixture(
        runtime: &mut Runtime,
        grids: impl IntoIterator<Item = Grid>,
        voxels: VoxelField,
    ) {
        runtime.state.grids = grids
            .into_iter()
            .map(|grid| (grid.grid_id.clone(), grid))
            .collect();
        runtime.state.voxels = voxels;
        runtime.state.active_contact_pairs.clear();
        runtime.state.simulation_tick = 0;
        runtime.state.physics_step_phase = 0;
        runtime.physics_step_phase = 0;
        let cargo_links = runtime
            .state
            .grids
            .values()
            .flat_map(|grid| grid.blocks.values())
            .filter_map(|block| {
                block
                    .inventory_id
                    .as_ref()
                    .map(|inventory_id| (inventory_id.clone(), block.block_id.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        runtime.state.inventories.retain(|inventory_id, inventory| {
            matches!(inventory.domain, InventoryDomain::Player { .. })
                || cargo_links.contains_key(inventory_id)
        });
        for (inventory_id, block_id) in cargo_links {
            runtime
                .state
                .inventories
                .entry(inventory_id.clone())
                .or_insert_with(|| InventoryRecord {
                    inventory_id,
                    domain: InventoryDomain::Cargo { block_id },
                    contents: InventoryContents::default(),
                    capacity_liters: CARGO_INVENTORY_CAPACITY_LITERS,
                });
        }
        runtime.state.ledger.genesis_installed_components = runtime
            .state
            .grids
            .values()
            .flat_map(|grid| grid.blocks.values())
            .map(|block| block.component_cost)
            .sum();
        runtime.state.ledger.destroyed_components = 0;
        assert!(runtime.state.conservation().valid);
        runtime.rebuild_physics_for_test();
        runtime
            .persist_snapshot()
            .expect("fixture snapshot persists");
    }

    #[test]
    fn replay_rejects_an_incompatible_event_schema_even_with_a_valid_hash() {
        let runtime = runtime();
        let mut state = runtime.state().clone();
        let mut event = state.prepare_system_event(EventPayload::SuitOxygenChanged {
            player_id: "player-local".into(),
            previous_oxygen_milli: 1_000,
            new_oxygen_milli: 995,
        });
        event.schema_version = EVENT_SCHEMA_VERSION - 1;
        event.event_hash = event.calculate_hash();

        let result = state.apply_event(&event);
        assert!(matches!(
            result,
            Err(IntentError::Rejected { ref code, .. }) if code == "event_schema_mismatch"
        ));
        assert_eq!(state.event_sequence, 0);
    }

    #[test]
    fn replay_rejects_tampered_contact_mass_and_lifecycle() {
        let runtime = runtime();
        let state = runtime.state();
        let voxel = state.voxels.occupied.iter().next().expect("voxel exists");
        let voxel_body = voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(*voxel));
        let voxel_collider = voxel_collision_collider_id(*voxel);
        let key = ContactPairKey {
            body_a: STARTER_GRID_ID.into(),
            collider_a: "block-core".into(),
            body_b: voxel_body.clone(),
            collider_b: voxel_collider.clone(),
        };
        let bodies = state
            .grids
            .values()
            .map(|grid| PhysicsBodyOutcome {
                grid_id: grid.grid_id.clone(),
                address: grid.address.clone(),
                position: grid.position,
                orientation: grid.orientation,
                linear_velocity: grid.linear_velocity,
                angular_velocity: grid.angular_velocity,
            })
            .collect::<Vec<_>>();
        let payload = EventPayload::PhysicsStepCommitted {
            fixed_step_hz: content::manifest().physics.fixed_step_hz,
            step_count: 1,
            remaining_step_phase: 0,
            bodies,
            players: stationary_player_outcomes(state, 1),
            contacts: vec![PhysicsContactOutcome {
                substep_index: 0,
                body_a_id: STARTER_GRID_ID.into(),
                collider_a_id: "block-core".into(),
                body_b_id: voxel_body.clone(),
                collider_b_id: voxel_collider,
                point_address: state
                    .address_for_active_position(Vec3::ZERO)
                    .expect("contact point has a canonical address"),
                point: Vec3::ZERO,
                normal: Vec3::new(-1.0, 0.0, 0.0),
                penetration_m: 0.01,
                closing_speed_mm_per_second: 1_000,
                estimated_normal_impulse_millinewton_seconds: 5_000,
                reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
                    state,
                    STARTER_GRID_ID,
                    &voxel_body,
                ),
                phase: PhysicsContactPhase::Began,
            }],
            active_contacts_after: vec![key],
        };

        let mut wrong_mass = state.prepare_system_event(payload.clone());
        if let EventPayload::PhysicsStepCommitted { contacts, .. } = &mut wrong_mass.payload {
            contacts[0].reduced_translational_mass_grams += 1;
        }
        wrong_mass.event_hash = wrong_mass.calculate_hash();
        let error = state
            .clone()
            .apply_event(&wrong_mass)
            .expect_err("tampered reduced translational mass is rejected");
        assert_eq!(error.code(), "replay_physics_contact_mass_invalid");

        let mut wrong_normal = state.prepare_system_event(payload.clone());
        if let EventPayload::PhysicsStepCommitted { contacts, .. } = &mut wrong_normal.payload {
            contacts[0].normal = Vec3::ZERO;
        }
        wrong_normal.event_hash = wrong_normal.calculate_hash();
        let error = state
            .clone()
            .apply_event(&wrong_normal)
            .expect_err("tampered contact normal is rejected");
        assert_eq!(error.code(), "replay_physics_contact_normal_invalid");

        let mut wrong_phase = state.prepare_system_event(payload);
        if let EventPayload::PhysicsStepCommitted { contacts, .. } = &mut wrong_phase.payload {
            contacts[0].phase = PhysicsContactPhase::Persisted;
        }
        wrong_phase.event_hash = wrong_phase.calculate_hash();
        let error = state
            .clone()
            .apply_event(&wrong_phase)
            .expect_err("tampered contact phase is rejected");
        assert_eq!(error.code(), "replay_physics_contact_phase_invalid");
    }

    #[test]
    fn replay_rejects_unrebuildable_body_outcomes_before_mutation() {
        let runtime = runtime();
        let mut state = runtime.state().clone();
        state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists")
            .anchored = true;
        assert!(state.grids[STARTER_GRID_ID].anchored);
        let bodies = state
            .grids
            .values()
            .map(|grid| PhysicsBodyOutcome {
                grid_id: grid.grid_id.clone(),
                address: grid.address.clone(),
                position: grid.position,
                orientation: grid.orientation,
                linear_velocity: grid.linear_velocity,
                angular_velocity: grid.angular_velocity,
            })
            .collect::<Vec<_>>();
        let payload = EventPayload::PhysicsStepCommitted {
            fixed_step_hz: content::manifest().physics.fixed_step_hz,
            step_count: 1,
            remaining_step_phase: 0,
            bodies,
            players: stationary_player_outcomes(&state, 1),
            contacts: Vec::new(),
            active_contacts_after: Vec::new(),
        };
        let before_hash = state.state_hash();

        let mut zero_rotation = state.prepare_system_event(payload.clone());
        if let EventPayload::PhysicsStepCommitted { bodies, .. } = &mut zero_rotation.payload {
            bodies[0].orientation = Quat::new(0.0, 0.0, 0.0, 0.0);
        }
        zero_rotation.event_hash = zero_rotation.calculate_hash();
        let mut replay = state.clone();
        let error = replay
            .apply_event(&zero_rotation)
            .expect_err("zero body rotation is rejected");
        assert_eq!(error.code(), "replay_physics_rotation_invalid");
        assert_eq!(replay.state_hash(), before_hash);

        let mut over_speed = state.prepare_system_event(payload.clone());
        if let EventPayload::PhysicsStepCommitted { bodies, .. } = &mut over_speed.payload {
            bodies[0].linear_velocity = Vec3::new(32.001, 0.0, 0.0);
        }
        over_speed.event_hash = over_speed.calculate_hash();
        let mut replay = state.clone();
        let error = replay
            .apply_event(&over_speed)
            .expect_err("over-limit body velocity is rejected");
        assert_eq!(error.code(), "replay_physics_body_velocity_invalid");
        assert_eq!(replay.state_hash(), before_hash);

        let mut moved_anchor = state.prepare_system_event(payload);
        if let EventPayload::PhysicsStepCommitted { bodies, .. } = &mut moved_anchor.payload {
            let body = bodies
                .iter_mut()
                .find(|body| body.grid_id == STARTER_GRID_ID)
                .expect("starter grid outcome exists");
            body.address = exact_test_address(body.position + Vec3::new(1.0, 0.0, 0.0));
        }
        moved_anchor.event_hash = moved_anchor.calculate_hash();
        let mut replay = state.clone();
        let error = replay
            .apply_event(&moved_anchor)
            .expect_err("anchored grid movement is rejected");
        assert_eq!(error.code(), "replay_physics_anchored_body_invalid");
        assert_eq!(replay.state_hash(), before_hash);
    }

    #[test]
    fn replay_rejects_tampered_player_physics_outcomes_before_mutation() {
        let runtime = runtime();
        let state = runtime.state();
        let bodies = state
            .grids
            .values()
            .map(|grid| PhysicsBodyOutcome {
                grid_id: grid.grid_id.clone(),
                address: grid.address.clone(),
                position: grid.position,
                orientation: grid.orientation,
                linear_velocity: grid.linear_velocity,
                angular_velocity: grid.angular_velocity,
            })
            .collect::<Vec<_>>();
        let canonical = EventPayload::PhysicsStepCommitted {
            fixed_step_hz: content::manifest().physics.fixed_step_hz,
            step_count: 1,
            remaining_step_phase: 0,
            bodies,
            players: stationary_player_outcomes(state, 1),
            contacts: Vec::new(),
            active_contacts_after: Vec::new(),
        };
        let before_hash = state.state_hash();
        let reject = |payload: EventPayload, expected_code: &str| {
            let event = state.prepare_system_event(payload);
            let mut replay = state.clone();
            let error = replay
                .apply_event(&event)
                .expect_err("tampered player physics must reject");
            assert_eq!(error.code(), expected_code);
            assert_eq!(replay.state_hash(), before_hash);
        };

        let mut missing = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut missing else {
            unreachable!();
        };
        players.clear();
        reject(missing, "replay_player_physics_presence_invalid");

        let mut wrong_id = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut wrong_id else {
            unreachable!();
        };
        let player = &mut players[0];
        player.player_id.push_str("-forged");
        reject(wrong_id, "replay_player_physics_identity_invalid");

        let mut noncanonical_address = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut noncanonical_address else {
            unreachable!();
        };
        players[0].address.local_um.x =
            i64::try_from(celestial::CELL_EDGE_UM / 2).expect("half-cell fits i64");
        reject(noncanonical_address, "event_spatial_address_invalid");

        let mut wrong_universe_address = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut wrong_universe_address else {
            unreachable!();
        };
        players[0].address.universe_id = "another-universe".into();
        reject(wrong_universe_address, "event_spatial_address_invalid");

        let mut non_finite = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut non_finite else {
            unreachable!();
        };
        let player = &mut players[0];
        player.linear_velocity.x = f64::NAN;
        reject(non_finite, "invalid_vector");

        let mut zero_rotation = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut zero_rotation else {
            unreachable!();
        };
        let player = &mut players[0];
        player.orientation = Quat::new(0.0, 0.0, 0.0, 0.0);
        reject(zero_rotation, "replay_player_physics_rotation_invalid");

        let mut over_speed = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut over_speed else {
            unreachable!();
        };
        let player = &mut players[0];
        player.linear_velocity = Vec3::new(32.001, 0.0, 0.0);
        reject(over_speed, "replay_player_physics_velocity_invalid");

        let mut teleported = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut teleported else {
            unreachable!();
        };
        let player = &mut players[0];
        player.address = exact_test_address(player.position + Vec3::new(10.0, 0.0, 0.0));
        reject(teleported, "replay_player_physics_translation_invalid");

        let mut spun = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut spun else {
            unreachable!();
        };
        let player = &mut players[0];
        player.orientation = Quat::new(0.0, 0.0, 1.0, 0.0);
        reject(spun, "replay_player_physics_rotation_continuity_invalid");

        let mut impossible_contact = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            players,
            contacts,
            active_contacts_after,
            ..
        } = &mut impossible_contact
        else {
            unreachable!();
        };
        let player = &mut players[0];
        let key = ContactPairKey {
            body_a: PLANET_BODY_ID.into(),
            collider_a: PLANET_COLLIDER_ID.into(),
            body_b: PLAYER_BODY_ID.into(),
            collider_b: PLAYER_COLLIDER_ID.into(),
        };
        player.surface_contact = true;
        contacts.push(PhysicsContactOutcome {
            substep_index: 0,
            body_a_id: key.body_a.clone(),
            collider_a_id: key.collider_a.clone(),
            body_b_id: key.body_b.clone(),
            collider_b_id: key.collider_b.clone(),
            point_address: state
                .address_for_active_position(Vec3::ZERO)
                .expect("contact point has a canonical address"),
            point: Vec3::ZERO,
            normal: Vec3::new(1.0, 0.0, 0.0),
            penetration_m: 0.0,
            closing_speed_mm_per_second: 0,
            estimated_normal_impulse_millinewton_seconds: 0,
            reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
                state,
                PLANET_BODY_ID,
                PLAYER_BODY_ID,
            ),
            phase: PhysicsContactPhase::Began,
        });
        active_contacts_after.push(key);
        reject(
            impossible_contact,
            "replay_player_contact_spatially_invalid",
        );

        let voxel = *state.voxels.occupied.iter().next().expect("voxel exists");
        let voxel_body = voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(voxel));
        let voxel_collider = voxel_collision_collider_id(voxel);
        let mut wrong_voxel_geometry = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            players,
            contacts,
            active_contacts_after,
            ..
        } = &mut wrong_voxel_geometry
        else {
            unreachable!();
        };
        let player = &mut players[0];
        let key = ContactPairKey {
            body_a: PLAYER_BODY_ID.into(),
            collider_a: PLAYER_COLLIDER_ID.into(),
            body_b: voxel_body,
            collider_b: voxel_collider,
        };
        player.surface_contact = true;
        contacts.push(PhysicsContactOutcome {
            substep_index: 0,
            body_a_id: key.body_a.clone(),
            collider_a_id: key.collider_a.clone(),
            body_b_id: key.body_b.clone(),
            collider_b_id: key.collider_b.clone(),
            point_address: state.player.address.clone(),
            point: state.player.position,
            normal: Vec3::new(1.0, 0.0, 0.0),
            penetration_m: 0.0,
            closing_speed_mm_per_second: 0,
            estimated_normal_impulse_millinewton_seconds: 0,
            reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
                state,
                PLAYER_BODY_ID,
                &key.body_b,
            ),
            phase: PhysicsContactPhase::Began,
        });
        active_contacts_after.push(key);
        reject(
            wrong_voxel_geometry,
            "replay_player_contact_spatially_invalid",
        );

        let mut wrong_grid_geometry = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            players,
            contacts,
            active_contacts_after,
            ..
        } = &mut wrong_grid_geometry
        else {
            unreachable!();
        };
        let player = &mut players[0];
        let key = ContactPairKey {
            body_a: STARTER_GRID_ID.into(),
            collider_a: "block-core".into(),
            body_b: PLAYER_BODY_ID.into(),
            collider_b: PLAYER_COLLIDER_ID.into(),
        };
        player.surface_contact = true;
        contacts.push(PhysicsContactOutcome {
            substep_index: 0,
            body_a_id: key.body_a.clone(),
            collider_a_id: key.collider_a.clone(),
            body_b_id: key.body_b.clone(),
            collider_b_id: key.collider_b.clone(),
            point_address: state.player.address.clone(),
            point: state.player.position,
            normal: Vec3::new(1.0, 0.0, 0.0),
            penetration_m: 0.0,
            closing_speed_mm_per_second: 0,
            estimated_normal_impulse_millinewton_seconds: 0,
            reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
                state,
                STARTER_GRID_ID,
                PLAYER_BODY_ID,
            ),
            phase: PhysicsContactPhase::Began,
        });
        active_contacts_after.push(key);
        reject(
            wrong_grid_geometry,
            "replay_player_contact_spatially_invalid",
        );

        let mut wrong_control = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut wrong_control else {
            unreachable!();
        };
        let player = &mut players[0];
        player.boost = true;
        reject(wrong_control, "replay_player_physics_control_invalid");

        let mut wrong_locomotion = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut wrong_locomotion else {
            unreachable!();
        };
        let player = &mut players[0];
        player.locomotion.kind = LocomotionKind::Grounded;
        reject(wrong_locomotion, "replay_player_locomotion_invalid");

        let mut wrong_jump = canonical.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut wrong_jump else {
            unreachable!();
        };
        let player = &mut players[0];
        player.jump = true;
        reject(wrong_jump, "replay_player_physics_control_invalid");

        let mut wrong_contact = canonical;
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut wrong_contact else {
            unreachable!();
        };
        let player = &mut players[0];
        player.surface_contact = true;
        reject(wrong_contact, "replay_player_surface_contact_invalid");
    }

    #[test]
    fn replay_accepts_bounded_linear_cast_voxel_contact_midpoint() {
        let runtime = runtime();
        let state = runtime.state();
        let coordinate = IVec3::new(6, 6, 1);
        assert!(
            state.voxels.occupied.contains(&coordinate),
            "regression fixture voxel remains present"
        );
        let mut prior_player = state.player.primary().clone();
        let contact_point = Vec3::new(6.500_020, 5.946_797, 1.593_079);
        set_test_player_position(&mut prior_player, contact_point);
        let mut player = stationary_player_outcomes(state, 1)
            .into_iter()
            .find(|candidate| candidate.player_id == prior_player.player_id)
            .expect("primary player outcome exists");
        player.position = prior_player.position;
        player.address = prior_player.address.clone();
        player.orientation = prior_player.orientation;
        let voxel_body =
            voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(coordinate));
        let contact = PhysicsContactOutcome {
            substep_index: 0,
            body_a_id: player_body_id(&prior_player.player_id),
            collider_a_id: player_collider_id(&prior_player.player_id),
            body_b_id: voxel_body.clone(),
            collider_b_id: voxel_collision_collider_id(coordinate),
            point_address: exact_test_address(contact_point),
            point: contact_point,
            normal: Vec3::new(1.0, 0.0, 0.0),
            penetration_m: 0.0,
            closing_speed_mm_per_second: 69,
            estimated_normal_impulse_millinewton_seconds: 0,
            reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
                state,
                &player_body_id(&prior_player.player_id),
                &voxel_body,
            ),
            phase: PhysicsContactPhase::Began,
        };

        assert!(state.player_contact_is_spatially_plausible(
            &contact,
            &prior_player,
            &player,
            &[],
            1,
            &physics_scene_config(),
        ));

        let implausible_point = Vec3::new(contact_point.x, contact_point.y, 2.1);
        let mut implausible_player = prior_player.clone();
        set_test_player_position(&mut implausible_player, implausible_point);
        let mut implausible_outcome = player;
        implausible_outcome.position = implausible_player.position;
        implausible_outcome.address = implausible_player.address.clone();
        let mut implausible_contact = contact;
        implausible_contact.point = implausible_point;
        implausible_contact.point_address = exact_test_address(implausible_point);
        assert!(!state.player_contact_is_spatially_plausible(
            &implausible_contact,
            &implausible_player,
            &implausible_outcome,
            &[],
            1,
            &physics_scene_config(),
        ));
    }

    #[test]
    fn replay_rejects_character_to_character_contacts_before_mutation() {
        let runtime = runtime();
        let mut state = runtime.state().clone();
        let mut second_player = state.player.primary().clone();
        second_player.player_id = "player-remote".into();
        second_player.inventory_id = "inventory-player-remote".into();
        let second_position = second_player.position + Vec3::new(4.0, 0.0, 0.0);
        set_test_player_position(&mut second_player, second_position);
        state
            .player
            .by_id
            .insert(second_player.player_id.clone(), second_player);
        let before_hash = state.state_hash();
        let local_body = player_body_id("player-local");
        let local_collider = player_collider_id("player-local");
        let remote_body = player_body_id("player-remote");
        let remote_collider = player_collider_id("player-remote");
        let contact_key = ContactPairKey {
            body_a: local_body.clone(),
            collider_a: local_collider.clone(),
            body_b: remote_body.clone(),
            collider_b: remote_collider.clone(),
        };
        let event = state.prepare_system_event(EventPayload::PhysicsStepCommitted {
            fixed_step_hz: content::manifest().physics.fixed_step_hz,
            step_count: 1,
            remaining_step_phase: 0,
            bodies: state
                .grids
                .values()
                .map(|grid| PhysicsBodyOutcome {
                    grid_id: grid.grid_id.clone(),
                    address: grid.address.clone(),
                    position: grid.position,
                    orientation: grid.orientation,
                    linear_velocity: grid.linear_velocity,
                    angular_velocity: grid.angular_velocity,
                })
                .collect(),
            players: stationary_player_outcomes(&state, 1),
            contacts: vec![PhysicsContactOutcome {
                substep_index: 0,
                body_a_id: local_body.clone(),
                collider_a_id: local_collider,
                body_b_id: remote_body.clone(),
                collider_b_id: remote_collider,
                point_address: state.player.address.clone(),
                point: state.player.position,
                normal: Vec3::new(1.0, 0.0, 0.0),
                penetration_m: 0.0,
                closing_speed_mm_per_second: 0,
                estimated_normal_impulse_millinewton_seconds: 0,
                reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
                    &state,
                    &local_body,
                    &remote_body,
                ),
                phase: PhysicsContactPhase::Began,
            }],
            active_contacts_after: vec![contact_key],
        });
        let mut replay = state.clone();

        let error = replay
            .apply_event(&event)
            .expect_err("character-to-character contacts must not enter canonical replay");

        assert_eq!(error.code(), "replay_character_contact_forbidden");
        assert_eq!(replay.state_hash(), before_hash);
    }

    #[test]
    fn two_player_physics_commits_in_order_and_recovers_exactly() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 67, 1_000).expect("runtime opens");
        add_remote_player(&mut runtime);
        runtime
            .persist_snapshot()
            .expect("two-player baseline persists");
        let prior_state = runtime.state.clone();

        assert!(runtime.advance(17).expect("shared physics step commits"));
        assert!(!runtime.is_halted());
        let committed_hash = runtime.state().state_hash();
        let journal = fs::read_to_string(directory.path().join("events.ndjson"))
            .expect("shared physics journal reads");
        let event: CanonicalEvent = serde_json::from_str(
            journal
                .lines()
                .last()
                .expect("shared physics event is durable"),
        )
        .expect("shared physics event decodes");
        let EventPayload::PhysicsStepCommitted { players, .. } = &event.payload else {
            panic!("shared step must commit a physics outcome");
        };
        assert_eq!(
            players
                .iter()
                .map(|player| player.player_id.as_str())
                .collect::<Vec<_>>(),
            vec!["player-local", "player-remote"]
        );

        let mut reversed = event.clone();
        let EventPayload::PhysicsStepCommitted { players, .. } = &mut reversed.payload else {
            unreachable!();
        };
        players.reverse();
        reversed.event_hash = reversed.calculate_hash();
        let mut replay = prior_state.clone();
        let error = replay
            .apply_event(&reversed)
            .expect_err("noncanonical player outcome order rejects");
        assert_eq!(error.code(), "replay_player_physics_order_invalid");
        assert_eq!(replay.state_hash(), prior_state.state_hash());

        drop(runtime);
        let recovered = Runtime::open(directory.path(), 67, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), committed_hash);
        assert_eq!(recovered.snapshot().players.len(), 2);
    }

    #[test]
    fn replay_rejects_dynamic_grid_teleport_before_mutation() {
        let runtime = runtime();
        let state = runtime.state();
        let before_hash = state.state_hash();
        let mut bodies = state
            .grids
            .values()
            .map(|grid| PhysicsBodyOutcome {
                grid_id: grid.grid_id.clone(),
                address: grid.address.clone(),
                position: grid.position,
                orientation: grid.orientation,
                linear_velocity: grid.linear_velocity,
                angular_velocity: grid.angular_velocity,
            })
            .collect::<Vec<_>>();
        let body = bodies
            .iter_mut()
            .find(|body| body.grid_id == STARTER_GRID_ID)
            .expect("starter grid outcome exists");
        body.address = exact_test_address(body.position + Vec3::new(10.0, 0.0, 0.0));
        let event = state.prepare_system_event(EventPayload::PhysicsStepCommitted {
            fixed_step_hz: content::manifest().physics.fixed_step_hz,
            step_count: 1,
            remaining_step_phase: 0,
            bodies,
            players: stationary_player_outcomes(state, 1),
            contacts: Vec::new(),
            active_contacts_after: Vec::new(),
        });
        let mut replay = state.clone();
        let error = replay
            .apply_event(&event)
            .expect_err("dynamic-grid teleport rejects");
        assert_eq!(error.code(), "replay_physics_body_translation_invalid");
        assert_eq!(replay.state_hash(), before_hash);
    }

    #[test]
    fn replay_rejects_a_full_integrity_no_op_weld_without_awarding_experience() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        aim_player_at_block(&mut runtime, STARTER_GRID_ID, "block-core");
        let mut state = runtime.state().clone();
        let before_hash = state.state_hash();
        let max_health = state.grids[STARTER_GRID_ID].blocks["block-core"].max_health();
        let event = state.new_test_human_event(
            "player-local",
            "replayed-full-weld",
            EventPayload::BlockWelded {
                grid_id: STARTER_GRID_ID.into(),
                block_id: "block-core".into(),
                previous_health: max_health,
                new_health: max_health,
                max_health,
                completed_construction: false,
            },
        );

        let error = state
            .apply_event(&event)
            .expect_err("a no-op weld must not replay");
        assert_eq!(error.code(), "replay_weld_no_change");
        assert_eq!(state.state_hash(), before_hash);
        assert_eq!(state.player.experience, 0);
    }

    #[test]
    fn replay_rejects_noncanonical_construction_fields_before_mutation() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let state = runtime.state().clone();
        let canonical = state
            .prepare_next_client_event_for_fixture(&ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "canonical-replay-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, 1, 0),
                kind: BlockKind::Cargo,
                orientation: 1,
            })
            .expect("canonical frame event prepares");

        for (label, mutate) in [
            (
                "component cost",
                (|block: &mut Block| block.component_cost = 0) as fn(&mut Block),
            ),
            (
                "cargo linkage",
                (|block: &mut Block| block.inventory_id = None) as fn(&mut Block),
            ),
            (
                "occupied coordinate",
                (|block: &mut Block| block.coordinate = IVec3::ZERO) as fn(&mut Block),
            ),
        ] {
            let mut event = canonical.clone();
            let EventPayload::BlockBuilt { block, .. } = &mut event.payload else {
                unreachable!("prepared construction event has block payload");
            };
            mutate(block);
            event.event_hash = event.calculate_hash();
            let mut candidate = state.clone();
            let before_hash = candidate.state_hash();
            assert!(
                candidate.apply_event(&event).is_err(),
                "tampered {label} must be rejected"
            );
            assert_eq!(candidate.state_hash(), before_hash);
        }

        let mut insufficient = state.clone();
        insufficient
            .inventories
            .get_mut(PLAYER_INVENTORY_ID)
            .expect("player inventory")
            .contents
            .components = 0;
        let before_hash = insufficient.state_hash();
        let error = insufficient
            .apply_event(&canonical)
            .expect_err("construction cannot underflow components during replay");
        assert_eq!(error.code(), "replay_construction_components_invalid");
        assert_eq!(insufficient.state_hash(), before_hash);
    }

    #[test]
    fn mining_retry_is_idempotent() {
        let mut runtime = runtime();
        let target = reachable_voxel(&mut runtime);
        let body_id = voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(target));
        let collider_id = voxel_collision_collider_id(target);
        assert!(runtime.physics().contains_collider(&body_id, &collider_id));
        let intent = ClientMessage::MineVoxel {
            operation_sequence: 0,
            operation_id: "mine-once".into(),
            coordinate: target,
        };
        let first = runtime
            .execute_next_for_fixture(&intent)
            .expect("first mine accepted");
        assert_eq!(runtime.physics_chunk_replacements, 1);
        assert_eq!(runtime.physics_full_rebuilds, 0);
        assert!(!runtime.physics().contains_collider(&body_id, &collider_id));
        let hash_after_first = runtime.state().state_hash();
        let second = runtime
            .execute_next_for_fixture(&intent)
            .expect("retry accepted");
        assert_eq!(first, second);
        assert_eq!(hash_after_first, runtime.state().state_hash());
        assert_eq!(runtime.physics_chunk_replacements, 1);
        assert_eq!(runtime.physics_full_rebuilds, 0);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn authenticated_secondary_mining_credits_only_its_actor_and_recovers_exactly() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 141, 1_000).expect("runtime opens");
        runtime
            .admit_development_player("player-remote")
            .expect("remote development player admits");
        let primary_position = runtime
            .state()
            .player
            .get("player-local")
            .expect("primary player exists")
            .position;
        let target = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coordinate| {
                let position = Vec3::new(
                    f64::from(coordinate.x),
                    f64::from(coordinate.y),
                    f64::from(coordinate.z),
                );
                primary_position.squared_distance(position)
                    > TOOL_SURFACE_RANGE_M * TOOL_SURFACE_RANGE_M
                    && exposed_voxel_face(&runtime.state.voxels, *coordinate).is_some()
            })
            .expect("asteroid has a voxel outside primary mining range");
        aim_player_at_voxel(&mut runtime, "player-remote", target);
        runtime
            .persist_snapshot()
            .expect("remote mining baseline persists");

        let material = runtime
            .state()
            .voxels
            .material(target)
            .expect("target voxel exists");
        let ore_yield = content::voxel(material).ore_yield;
        let primary_before = runtime
            .state()
            .player
            .get("player-local")
            .expect("primary player exists")
            .clone();
        let primary_inventory_before = runtime.state().inventories[&primary_before.inventory_id]
            .contents
            .clone();
        let remote_before = runtime
            .state()
            .player
            .get("player-remote")
            .expect("remote player exists")
            .clone();
        let intent = ClientMessage::MineVoxel {
            operation_sequence: 0,
            operation_id: "shared-mining-operation".into(),
            coordinate: target,
        };

        let receipt = runtime
            .execute_next_as_for_fixture("player-remote", &intent)
            .expect("authenticated remote actor mines reachable voxel");
        assert_eq!(receipt.code, "voxel_mined");
        assert_eq!(runtime.state().voxels.material(target), None);
        assert_eq!(
            runtime.state().inventories[&remote_before.inventory_id]
                .contents
                .ore,
            ore_yield
        );
        let remote_after = runtime
            .state()
            .player
            .get("player-remote")
            .expect("remote player remains present")
            .clone();
        assert_eq!(
            remote_after.experience,
            remote_before.experience + ore_yield * 5
        );
        assert_eq!(
            remote_after.career.voxels_mined,
            remote_before.career.voxels_mined + 1
        );
        assert_eq!(
            runtime
                .state()
                .player
                .get("player-local")
                .expect("primary player remains present"),
            &primary_before
        );
        assert_eq!(
            runtime.state().inventories[&primary_before.inventory_id].contents,
            primary_inventory_before
        );
        assert_eq!(
            runtime
                .state()
                .processed_operation("player-remote", "shared-mining-operation"),
            Some(&receipt)
        );
        assert!(
            runtime
                .state()
                .processed_operation("player-local", "shared-mining-operation")
                .is_none()
        );
        let journal = fs::read_to_string(directory.path().join("events.ndjson"))
            .expect("mining journal reads");
        let event: CanonicalEvent = serde_json::from_str(
            journal
                .lines()
                .last()
                .expect("mining event is durably journaled"),
        )
        .expect("mining event parses");
        assert_eq!(event.actor_player_id.as_deref(), Some("player-remote"));
        assert!(matches!(
            event.payload,
            EventPayload::VoxelMined { ref inventory_id, .. }
                if inventory_id == &remote_before.inventory_id
        ));
        let accepted_hash = runtime.state().state_hash();
        assert_eq!(
            runtime
                .execute_next_as_for_fixture("player-remote", &intent)
                .expect("remote mining retry is idempotent"),
            receipt
        );
        assert_eq!(runtime.state().state_hash(), accepted_hash);
        assert!(runtime.state().conservation().valid);

        drop(runtime);
        let recovered = Runtime::open(directory.path(), 141, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), accepted_hash);
        assert_eq!(
            recovered
                .state()
                .player
                .get("player-remote")
                .expect("remote actor recovers"),
            &remote_after
        );
        assert_eq!(
            recovered
                .state()
                .player
                .get("player-local")
                .expect("primary actor recovers"),
            &primary_before
        );
    }

    #[test]
    fn secondary_mining_rejections_are_exactly_non_mutating() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 142, 1_000).expect("runtime opens");
        runtime
            .admit_development_player("player-remote")
            .expect("remote development player admits");
        let out_of_range_target = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .copied()
            .max_by(|left, right| {
                let remote_position = runtime
                    .state()
                    .player
                    .get("player-remote")
                    .expect("remote exists")
                    .position;
                let distance = |coordinate: &IVec3| {
                    remote_position.squared_distance(Vec3::new(
                        f64::from(coordinate.x),
                        f64::from(coordinate.y),
                        f64::from(coordinate.z),
                    ))
                };
                distance(left).total_cmp(&distance(right))
            })
            .expect("asteroid has voxels");
        let assert_unchanged =
            |runtime: &mut Runtime, operation_id: &str, coordinate: IVec3, expected_code: &str| {
                let before_hash = runtime.state().state_hash();
                let before_sequence = runtime.state().event_sequence;
                let before_fingerprint = runtime.physics().body_collider_fingerprint();
                let before_journal = fs::read(directory.path().join("events.ndjson"))
                    .expect("journal reads before rejection");
                let result = runtime.execute_next_as_for_fixture(
                    "player-remote",
                    &ClientMessage::MineVoxel {
                        operation_sequence: 0,
                        operation_id: operation_id.into(),
                        coordinate,
                    },
                );
                assert!(matches!(
                    result,
                    Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                        if code == expected_code
                ));
                assert_eq!(runtime.state().state_hash(), before_hash);
                assert_eq!(runtime.state().event_sequence, before_sequence);
                assert_eq!(
                    runtime.physics().body_collider_fingerprint(),
                    before_fingerprint
                );
                assert_eq!(
                    fs::read(directory.path().join("events.ndjson"))
                        .expect("journal reads after rejection"),
                    before_journal
                );
                assert!(
                    runtime
                        .state()
                        .processed_operation("player-remote", operation_id)
                        .is_none()
                );
            };
        assert_unchanged(
            &mut runtime,
            "remote-mine-out-of-range",
            out_of_range_target,
            "voxel_not_targeted",
        );

        let reachable_target = *runtime
            .state()
            .voxels
            .occupied
            .iter()
            .next()
            .expect("asteroid has a reachable fixture voxel");
        let reachable_position = Vec3::new(
            f64::from(reachable_target.x),
            f64::from(reachable_target.y),
            f64::from(reachable_target.z),
        );
        let remote = runtime
            .state
            .player
            .get_mut("player-remote")
            .expect("remote player exists");
        set_test_player_position(remote, reachable_position + Vec3::new(0.0, 3.0, 0.0));
        remote.life_state = PlayerLifeState::Incapacitated {
            death_id: "remote-test-death".into(),
            cause: PlayerDeathCause::OxygenDepleted,
        };
        runtime.rebuild_physics_for_test();
        runtime
            .persist_snapshot()
            .expect("incapacitated mining baseline persists");
        assert_unchanged(
            &mut runtime,
            "remote-mine-incapacitated",
            reachable_target,
            "player_incapacitated",
        );
    }

    #[test]
    fn replay_rejects_cross_actor_mining_inventory_spoof_without_mutation() {
        let mut runtime = runtime();
        runtime
            .admit_development_player("player-remote")
            .expect("remote development player admits");
        let primary_position = runtime.state().player.primary().position;
        let target = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coordinate| {
                primary_position.squared_distance(Vec3::new(
                    f64::from(coordinate.x),
                    f64::from(coordinate.y),
                    f64::from(coordinate.z),
                )) > TOOL_SURFACE_RANGE_M * TOOL_SURFACE_RANGE_M
                    && exposed_voxel_face(&runtime.state.voxels, *coordinate).is_some()
            })
            .expect("asteroid has a fixture voxel outside primary mining range");
        aim_player_at_voxel(&mut runtime, "player-remote", target);
        let canonical_event = runtime
            .state()
            .prepare_next_client_event_as_for_fixture(
                "player-remote",
                &ClientMessage::MineVoxel {
                    operation_sequence: 0,
                    operation_id: "forged-mining-owner".into(),
                    coordinate: target,
                },
            )
            .expect("canonical remote mining event prepares");
        let primary_inventory_id = runtime.state().player.primary().inventory_id.clone();
        let assert_replay_rejected_without_mutation =
            |event: &CanonicalEvent, expected_code: &str| {
                let mut candidate = runtime.state().clone();
                let before_hash = candidate.state_hash();
                let result = candidate.apply_event(event);
                assert!(matches!(
                    result,
                    Err(IntentError::Rejected { ref code, .. }) if code == expected_code
                ));
                assert_eq!(candidate.state_hash(), before_hash);
                assert!(candidate.voxels.material(target).is_some());
                assert_eq!(candidate.inventories[&primary_inventory_id].contents.ore, 0);
                assert_eq!(
                    candidate.inventories["inventory-player-remote"]
                        .contents
                        .ore,
                    0
                );
            };

        let mut event = canonical_event.clone();
        let EventPayload::VoxelMined { inventory_id, .. } = &mut event.payload else {
            unreachable!("mining intent prepares a mining event")
        };
        inventory_id.clone_from(&primary_inventory_id);
        event.event_hash = event.calculate_hash();
        assert_replay_rejected_without_mutation(&event, "replay_mining_actor_inventory_invalid");

        let mut event = canonical_event.clone();
        event.actor_player_id = Some("player-local".into());
        let EventPayload::VoxelMined { inventory_id, .. } = &mut event.payload else {
            unreachable!("mining intent prepares a mining event")
        };
        inventory_id.clone_from(&primary_inventory_id);
        event.event_hash = event.calculate_hash();
        assert_replay_rejected_without_mutation(&event, "replay_intent_fingerprint_mismatch");

        let mut event = canonical_event;
        let EventPayload::VoxelMined { ore_yield, .. } = &mut event.payload else {
            unreachable!("mining intent prepares a mining event")
        };
        *ore_yield = ore_yield.saturating_add(1);
        event.event_hash = event.calculate_hash();
        assert_replay_rejected_without_mutation(&event, "replay_mining_yield_invalid");
    }

    #[test]
    fn voxel_collision_chunks_use_euclidean_ownership_and_stable_local_leaves() {
        let cases = [(-9, -2), (-8, -1), (-1, -1), (0, 0), (7, 0), (8, 1)];
        for (coordinate, expected_chunk) in cases {
            assert_eq!(
                voxel_collision_chunk_coordinate(IVec3::new(coordinate, 0, 0)).x,
                expected_chunk
            );
        }

        let enumerated_chunk = IVec3::new(-2, 3, -4);
        let enumerated = voxel_collision_chunk_coordinates(enumerated_chunk);
        assert_eq!(enumerated.len(), 8 * 8 * 8);
        assert_eq!(
            enumerated.iter().copied().collect::<BTreeSet<_>>().len(),
            8 * 8 * 8
        );
        assert!(enumerated
            .iter()
            .all(|coordinate| voxel_collision_chunk_coordinate(*coordinate) == enumerated_chunk));
        assert!(!enumerated.contains(&IVec3::new(8_000, 8_000, 8_000)));

        let mut state = runtime().state().clone();
        state.voxels = VoxelField {
            occupied: BTreeSet::from([
                IVec3::new(-9, 0, 0),
                IVec3::new(-8, 0, 0),
                IVec3::new(-1, 0, 0),
                IVec3::new(0, 0, 0),
                IVec3::new(7, 7, 7),
                IVec3::new(8, 0, 0),
            ]),
            ferrite_ore: BTreeSet::new(),
        };
        let specs = voxel_collision_body_specs(&state);
        assert_eq!(specs.len(), 4);
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.body_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "voxel-chunk--2-0-0",
                "voxel-chunk--1-0-0",
                "voxel-chunk-0-0-0",
                "voxel-chunk-1-0-0"
            ]
        );
        for spec in specs {
            assert!(spec.colliders.len() <= 8 * 8 * 8);
            for collider in spec.colliders {
                assert!((0.0..8.0).contains(&collider.local_pose.position.x));
                assert!((0.0..8.0).contains(&collider.local_pose.position.y));
                assert!((0.0..8.0).contains(&collider.local_pose.position.z));
                assert!(collider.collider_id.starts_with("voxel-"));
            }
        }
    }

    #[test]
    fn mining_replay_prunes_removed_collider_on_either_pair_side_only() {
        let mut state = runtime().state().clone();
        let target = state
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coordinate| {
                let chunk = voxel_collision_chunk_coordinate(*coordinate);
                exposed_voxel_face(&state.voxels, *coordinate).is_some()
                    && state.voxels.occupied.iter().any(|other| {
                        *other != *coordinate && voxel_collision_chunk_coordinate(*other) == chunk
                    })
            })
            .expect("target has a surviving chunk neighbor");
        let survivor = state
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coordinate| {
                *coordinate != target
                    && voxel_collision_chunk_coordinate(*coordinate)
                        == voxel_collision_chunk_coordinate(target)
            })
            .expect("surviving collider exists");
        let target_position = Vec3::new(
            f64::from(target.x),
            f64::from(target.y),
            f64::from(target.z),
        );
        let face = exposed_voxel_face(&state.voxels, target).unwrap();
        aim_player_from_face(
            state.player.primary_mut(),
            target_position,
            Vec3::new(f64::from(face.x), f64::from(face.y), f64::from(face.z)),
        );
        let body_id = voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(target));
        let collider_id = voxel_collision_collider_id(target);
        let removed_on_right = ContactPairKey {
            body_a: "aa-grid".into(),
            collider_a: "aa-block".into(),
            body_b: body_id.clone(),
            collider_b: collider_id.clone(),
        };
        let removed_on_left = ContactPairKey {
            body_a: body_id.clone(),
            collider_a: collider_id,
            body_b: "zz-grid".into(),
            collider_b: "zz-block".into(),
        };
        let survivor_pair = ContactPairKey {
            body_a: "aa-grid".into(),
            collider_a: "aa-block".into(),
            body_b: body_id,
            collider_b: voxel_collision_collider_id(survivor),
        };
        state.active_contact_pairs = BTreeSet::from([
            removed_on_right.clone(),
            removed_on_left.clone(),
            survivor_pair.clone(),
        ]);
        let event = state
            .prepare_next_client_event_for_fixture(&ClientMessage::MineVoxel {
                operation_sequence: 0,
                operation_id: "prune-mined-contact".into(),
                coordinate: target,
            })
            .expect("mining event prepares");
        let mut replay = state.clone();
        replay.apply_event(&event).expect("mining event replays");

        assert!(!replay.active_contact_pairs.contains(&removed_on_right));
        assert!(!replay.active_contact_pairs.contains(&removed_on_left));
        assert_eq!(replay.active_contact_pairs, BTreeSet::from([survivor_pair]));
    }

    #[test]
    fn dirty_chunk_replacement_preserves_surviving_canonical_contact_lifecycle() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 127, 1_000).expect("runtime opens");
        let target = IVec3::ZERO;
        let survivor = IVec3::new(1, 0, 0);
        let resting = test_grid(
            "resting-grid",
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::ZERO,
            [
                Block::new("resting-battery", IVec3::ZERO, BlockKind::Battery),
                Block::new("resting-armor", IVec3::new(1, 0, 0), BlockKind::Structural),
            ],
        );
        replace_with_physics_fixture(
            &mut runtime,
            [resting],
            VoxelField {
                occupied: BTreeSet::from([target, survivor]),
                ferrite_ore: BTreeSet::new(),
            },
        );
        runtime.relocate_player_for_test(Vec3::new(0.0, 3.0, 0.0));
        runtime.persist_snapshot().expect("player pose persists");
        runtime
            .execute_next_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "settle-on-two-voxels".into(),
                grid_id: "resting-grid".into(),
                linear_input: Vec3::new(0.0, -0.2, 0.0),
                angular_input: Vec3::ZERO,
                dampeners: true,
            })
            .expect("settling control is accepted");
        let target_collider = voxel_collision_collider_id(target);
        let survivor_collider = voxel_collision_collider_id(survivor);
        let contacted_both = (0..120).any(|_| {
            runtime.advance(17).expect("contact physics advances");
            let touches_target = runtime.state().active_contact_pairs.iter().any(|pair| {
                pair.collider_a == target_collider || pair.collider_b == target_collider
            });
            let touches_survivor = runtime.state().active_contact_pairs.iter().any(|pair| {
                pair.collider_a == survivor_collider || pair.collider_b == survivor_collider
            });
            touches_target && touches_survivor
        });
        assert!(contacted_both, "grid must contact both terrain leaves");
        let surviving_pair = runtime
            .state()
            .active_contact_pairs
            .iter()
            .find(|pair| {
                pair.collider_a == survivor_collider || pair.collider_b == survivor_collider
            })
            .cloned()
            .expect("surviving pair is active");

        aim_player_at_voxel(&mut runtime, "player-local", target);

        runtime
            .execute_next_for_fixture(&ClientMessage::MineVoxel {
                operation_sequence: 0,
                operation_id: "mine-one-contact-leaf".into(),
                coordinate: target,
            })
            .expect("contacted voxel is mined");
        assert!(!runtime.state().active_contact_pairs.iter().any(|pair| {
            pair.collider_a == target_collider || pair.collider_b == target_collider
        }));
        assert!(
            runtime
                .state()
                .active_contact_pairs
                .contains(&surviving_pair)
        );
        assert_eq!(runtime.physics_chunk_replacements, 1);

        runtime.advance(17).expect("post-edit physics advances");
        let journal = fs::read_to_string(directory.path().join("events.ndjson"))
            .expect("post-edit journal reads");
        let event = journal
            .lines()
            .rev()
            .map(|line| serde_json::from_str::<CanonicalEvent>(line).expect("event parses"))
            .find(|event| matches!(event.payload, EventPayload::PhysicsStepCommitted { .. }))
            .expect("post-edit physics event exists");
        let EventPayload::PhysicsStepCommitted {
            contacts,
            active_contacts_after,
            ..
        } = event.payload
        else {
            unreachable!("filtered event is a physics commit")
        };
        assert!(contacts.iter().any(|contact| {
            (contact.collider_a_id == survivor_collider
                || contact.collider_b_id == survivor_collider)
                && contact.phase == PhysicsContactPhase::Persisted
        }));
        assert!(!contacts.iter().any(|contact| {
            contact.collider_a_id == target_collider || contact.collider_b_id == target_collider
        }));
        assert!(active_contacts_after.contains(&surviving_pair));
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn mining_final_anchor_support_is_rejected_without_any_mutation() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 107, 100).expect("runtime opens");
        let target = IVec3::ZERO;
        let mut anchored = test_grid(
            "anchored-grid",
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::ZERO,
            [
                Block::new("anchor-block", IVec3::ZERO, BlockKind::Anchor),
                Block::new("anchor-battery", IVec3::new(1, 0, 0), BlockKind::Battery),
            ],
        );
        anchored.anchored = true;
        replace_with_physics_fixture(
            &mut runtime,
            [anchored],
            VoxelField {
                occupied: BTreeSet::from([target]),
                ferrite_ore: BTreeSet::new(),
            },
        );
        aim_player_at_voxel(&mut runtime, "player-local", target);
        runtime.persist_snapshot().expect("player pose persists");
        assert!(runtime.state().grids["anchored-grid"].anchor_touches(&runtime.state().voxels));
        let before_hash = runtime.state().state_hash();
        let before_fingerprint = runtime.physics().body_collider_fingerprint();
        let before_journal =
            fs::read(directory.path().join("events.ndjson")).expect("journal reads");

        let result = runtime.execute_next_for_fixture(&ClientMessage::MineVoxel {
            operation_sequence: 0,
            operation_id: "mine-final-anchor-support".into(),
            coordinate: target,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "voxel_supports_anchor"
        ));
        assert_eq!(runtime.state().state_hash(), before_hash);
        assert_eq!(
            runtime.physics().body_collider_fingerprint(),
            before_fingerprint
        );
        assert_eq!(runtime.physics_chunk_replacements, 0);
        assert_eq!(runtime.physics_full_rebuilds, 0);
        assert!(
            runtime
                .state()
                .processed_operation("player-local", "mine-final-anchor-support")
                .is_none()
        );
        assert_eq!(
            fs::read(directory.path().join("events.ndjson")).expect("journal rereads"),
            before_journal
        );
    }

    #[test]
    fn transfer_retry_neither_duplicates_nor_loses_assets() {
        let mut runtime = runtime();
        let cargo_id = runtime
            .state()
            .inventories
            .keys()
            .find(|id| id.contains("cargo"))
            .cloned()
            .expect("cargo inventory");
        let intent = ClientMessage::TransferInventory {
            operation_sequence: 0,
            operation_id: "transfer-components".into(),
            source_inventory_id: PLAYER_INVENTORY_ID.into(),
            destination_inventory_id: cargo_id.clone(),
            resource: ResourceKind::Component,
            quantity: 4,
        };
        runtime
            .execute_next_for_fixture(&intent)
            .expect("transfer accepted");
        runtime
            .execute_next_for_fixture(&intent)
            .expect("retry returns receipt");
        assert_eq!(
            runtime.state().inventories[PLAYER_INVENTORY_ID]
                .contents
                .components,
            20
        );
        assert_eq!(
            runtime.state().inventories[&cargo_id].contents.components,
            4
        );
        assert!(runtime.state().conservation().valid);
    }

    proptest! {
        #[test]
        fn component_transfers_conserve_every_valid_quantity(quantity in 1_u64..=24) {
            let mut runtime = runtime();
            let cargo_id = runtime
                .state()
                .inventories
                .keys()
                .find(|id| id.contains("cargo"))
                .cloned()
                .expect("cargo inventory");
            runtime
                .execute_next_for_fixture(&ClientMessage::TransferInventory {
                    operation_sequence: 0,
                    operation_id: format!("property-transfer-{quantity}"),
                    source_inventory_id: PLAYER_INVENTORY_ID.into(),
                    destination_inventory_id: cargo_id.clone(),
                    resource: ResourceKind::Component,
                    quantity,
                })
                .expect("valid transfer is accepted");

            prop_assert_eq!(
                runtime.state().inventories[PLAYER_INVENTORY_ID]
                    .contents
                    .components,
                24 - quantity
            );
            prop_assert_eq!(
                runtime.state().inventories[&cargo_id].contents.components,
                quantity
            );
            prop_assert!(runtime.state().conservation().valid);
        }
    }

    #[test]
    fn character_control_rejects_wrong_epoch_out_of_order_and_unbounded_input() {
        let mut runtime = runtime();
        let result = runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "wrong-epoch".into(),
            movement_epoch: 0,
            input_sequence: 1,
            linear_input: Vec3::ZERO,
            angular_input: Vec3::ZERO,
            boost: false,
            jump: false,
            dampeners: true,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_epoch_stale"
        ));
        assert_eq!(runtime.state().event_sequence, 0);

        for (operation_id, linear_input, angular_input) in [
            (
                "nonfinite-linear",
                Vec3::new(f64::NAN, 0.0, 0.0),
                Vec3::ZERO,
            ),
            (
                "nonfinite-angular",
                Vec3::ZERO,
                Vec3::new(0.0, f64::INFINITY, 0.0),
            ),
        ] {
            let result = runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: operation_id.into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input,
                angular_input,
                boost: false,
                jump: false,
                dampeners: true,
            });
            assert!(matches!(
                result,
                Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                    if code == "invalid_vector"
            ));
            assert_eq!(runtime.state().event_sequence, 0);
        }

        let result = runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "unbounded-control".into(),
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(1.01, 0.0, 0.0),
            angular_input: Vec3::ZERO,
            boost: false,
            jump: false,
            dampeners: true,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "control_input_out_of_range"
        ));
        assert_eq!(runtime.state().event_sequence, 0);

        let accepted_control = ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "control-1".into(),
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(0.0, 0.0, -1.0),
            angular_input: Vec3::new(0.0, 0.0, 0.5),
            boost: true,
            jump: false,
            dampeners: false,
        };
        let first_receipt = runtime
            .execute_next_for_fixture(&accepted_control)
            .expect("bounded in-order control is accepted");
        let retry_receipt = runtime
            .execute_next_for_fixture(&accepted_control)
            .expect("same operation retry returns its durable receipt");
        assert_eq!(retry_receipt, first_receipt);
        assert_eq!(runtime.state().event_sequence, 1);
        let result = runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "control-reordered".into(),
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::ZERO,
            angular_input: Vec3::ZERO,
            boost: false,
            jump: false,
            dampeners: true,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_input_out_of_order"
        ));
        assert_eq!(runtime.state().event_sequence, 1);
        assert_eq!(runtime.state().player.last_received_input_sequence, 1);
        assert_eq!(runtime.state().player.last_processed_input_sequence, 0);
        assert_eq!(runtime.state().player.control_linear_input, Vec3::ZERO);
        assert_eq!(runtime.state().player.pending_control_frames.len(), 1);
        assert!(runtime.state().conservation().valid);
    }

    proptest! {
        #[test]
        fn godot_float32_mouse_and_roll_vectors_survive_json_reconstruction(
            mouse_x in -32_768_i32..=32_767,
            mouse_y in -32_768_i32..=32_767,
            roll in -1_i8..=1,
        ) {
            // This mirrors Godot's standard float32 Vector3 limit_length(1.0)
            // before serde reconstructs the JSON components as float64.
            let mut x = -(mouse_y as f32) * 0.12_f32;
            let mut y = -(mouse_x as f32) * 0.12_f32;
            let mut z = f32::from(roll);
            let magnitude = (x * x + y * y + z * z).sqrt();
            if magnitude > 1.0 {
                let scale = 1.0_f32 / magnitude;
                x *= scale;
                y *= scale;
                z *= scale;
            }
            let reconstructed = Vec3::new(f64::from(x), f64::from(y), f64::from(z));

            prop_assert!(
                ensure_bounded_control(reconstructed, "Godot reconstructed control").is_ok(),
                "float32-valid source magnitude {} was rejected",
                reconstructed.magnitude(),
            );
            prop_assert!(
                reconstructed.magnitude()
                    <= MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_SOURCE_PRECISION_EPSILON
            );
        }
    }

    #[test]
    fn control_precision_tolerance_does_not_admit_materially_unbounded_input() {
        let source_precision_boundary = Vec3::new(
            MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_SOURCE_PRECISION_EPSILON,
            0.0,
            0.0,
        );
        assert!(ensure_bounded_control(source_precision_boundary, "boundary control").is_ok());
        assert!(matches!(
            ensure_bounded_control(Vec3::new(1.000_01, 0.0, 0.0), "materially unbounded"),
            Err(IntentError::Rejected { ref code, .. })
                if code == "control_input_out_of_range"
        ));
    }

    #[test]
    fn one_frame_press_and_release_are_consumed_on_successive_fixed_steps() {
        let mut runtime = runtime();
        let initial_orientation = runtime.state().player.orientation;
        for (input_sequence, angular_input) in [(1, Vec3::new(0.0, 0.0, 1.0)), (2, Vec3::ZERO)] {
            runtime
                .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                    operation_sequence: 0,
                    operation_id: format!("tap-{input_sequence}"),
                    movement_epoch: 1,
                    input_sequence,
                    linear_input: Vec3::ZERO,
                    angular_input,
                    boost: false,
                    jump: false,
                    dampeners: true,
                })
                .expect("tap transition is durably accepted");
        }

        assert_eq!(runtime.state().player.last_received_input_sequence, 2);
        assert_eq!(runtime.state().player.last_processed_input_sequence, 0);
        assert_eq!(runtime.state().player.pending_control_frames.len(), 2);

        runtime.advance(17).expect("press substep commits");
        assert_eq!(runtime.state().simulation_tick, 1);
        assert_eq!(runtime.state().player.last_processed_input_sequence, 1);
        assert_eq!(runtime.state().player.pending_control_frames.len(), 1);
        assert_eq!(
            runtime.state().player.control_angular_input,
            Vec3::new(0.0, 0.0, 1.0)
        );
        assert!(runtime.state().player.angular_velocity.z > 0.0);
        assert_ne!(runtime.state().player.orientation, initial_orientation);

        runtime.advance(17).expect("release substep commits");
        assert_eq!(runtime.state().simulation_tick, 2);
        assert_eq!(runtime.state().player.last_processed_input_sequence, 2);
        assert!(runtime.state().player.pending_control_frames.is_empty());
        assert_eq!(runtime.state().player.control_angular_input, Vec3::ZERO);
    }

    #[test]
    fn pending_character_controls_recover_exactly_from_snapshot_and_journal() {
        for snapshot_every in [1, 100] {
            let directory = tempdir().expect("tempdir");
            let expected_hash;
            {
                let mut runtime =
                    Runtime::open(directory.path(), 132, snapshot_every).expect("runtime opens");
                for (input_sequence, angular_input) in
                    [(1, Vec3::new(0.0, 0.0, 1.0)), (2, Vec3::ZERO)]
                {
                    runtime
                        .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                            operation_sequence: 0,
                            operation_id: format!("restart-tap-{snapshot_every}-{input_sequence}"),
                            movement_epoch: 1,
                            input_sequence,
                            linear_input: Vec3::ZERO,
                            angular_input,
                            boost: false,
                            jump: false,
                            dampeners: true,
                        })
                        .expect("queued transition commits");
                }
                expected_hash = runtime.state().state_hash();
            }

            let mut recovered =
                Runtime::open(directory.path(), 132, snapshot_every).expect("runtime recovers");
            assert_eq!(recovered.state().state_hash(), expected_hash);
            assert_eq!(recovered.state().player.last_received_input_sequence, 2);
            assert_eq!(recovered.state().player.last_processed_input_sequence, 0);
            assert_eq!(recovered.state().player.pending_control_frames.len(), 2);
            recovered.advance(17).expect("recovered press commits");
            assert_eq!(recovered.state().player.last_processed_input_sequence, 1);
            assert_eq!(recovered.state().player.pending_control_frames.len(), 1);
            assert!(recovered.state().player.angular_velocity.z > 0.0);
        }
    }

    #[test]
    fn character_control_commit_failpoints_recover_prior_or_durable_queue() {
        for (failpoint, durable) in [
            (AppendFailpoint::BeforeWrite, false),
            (AppendFailpoint::AfterSync, true),
        ] {
            let directory = tempdir().expect("tempdir");
            {
                let mut runtime = Runtime::open(directory.path(), 133, 100).expect("runtime opens");
                runtime.store.set_append_failpoint(failpoint);
                assert!(matches!(
                    runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                        operation_sequence: 0,
                        operation_id: format!("queued-failpoint-{durable}"),
                        movement_epoch: 1,
                        input_sequence: 1,
                        linear_input: Vec3::ZERO,
                        angular_input: Vec3::new(0.0, 0.0, 1.0),
                        boost: false,
                        jump: false,
                        dampeners: true,
                    }),
                    Err(RuntimeError::Persistence(
                        PersistenceError::InjectedFailure(_)
                    ))
                ));
                assert!(runtime.is_halted());
                assert_eq!(runtime.state().player.last_received_input_sequence, 0);
                assert!(runtime.state().player.pending_control_frames.is_empty());
            }

            let recovered = Runtime::open(directory.path(), 133, 100).expect("runtime recovers");
            assert_eq!(recovered.state().event_sequence, u64::from(durable));
            assert_eq!(
                recovered.state().player.last_received_input_sequence,
                u64::from(durable)
            );
            assert_eq!(
                recovered.state().player.pending_control_frames.len(),
                usize::from(durable)
            );
            assert_eq!(recovered.state().player.last_processed_input_sequence, 0);
        }
    }

    #[test]
    fn character_control_queue_applies_backpressure_without_mutation() {
        let mut runtime = runtime();
        let queue_limit = usize::try_from(content::manifest().character.control_lease_ticks)
            .unwrap_or(usize::MAX)
            .min(MAX_PENDING_PLAYER_CONTROL_FRAMES);
        for offset in 0..queue_limit {
            let input_sequence = u64::try_from(offset + 1).expect("queue bound fits u64");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                    operation_sequence: 0,
                    operation_id: format!("queued-{input_sequence}"),
                    movement_epoch: 1,
                    input_sequence,
                    linear_input: Vec3::ZERO,
                    angular_input: Vec3::new(0.0, 0.0, 1.0),
                    boost: false,
                    jump: false,
                    dampeners: true,
                })
                .expect("bounded queue entry commits");
        }
        let before = runtime.state().state_hash();
        let rejected_sequence = u64::try_from(queue_limit + 1).expect("queue bound fits u64");
        let result = runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "queue-overflow".into(),
            movement_epoch: 1,
            input_sequence: rejected_sequence,
            linear_input: Vec3::ZERO,
            angular_input: Vec3::ZERO,
            boost: false,
            jump: false,
            dampeners: true,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_input_backpressure"
        ));
        assert_eq!(runtime.state().state_hash(), before);
        assert_eq!(
            runtime.state().player.pending_control_frames.len(),
            queue_limit
        );
    }

    #[test]
    fn expired_queued_control_is_acked_without_reviving_stale_motion() {
        let mut runtime = runtime();
        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "expired-front".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::new(0.0, 0.0, 1.0),
                boost: true,
                jump: false,
                dampeners: false,
            })
            .expect("stale fixture control commits");
        runtime.state.player.pending_control_frames[0].expires_at_simulation_tick = 0;
        runtime.advance(17).expect("expired frame is retired");
        assert_eq!(runtime.state().player.last_processed_input_sequence, 1);
        assert!(runtime.state().player.pending_control_frames.is_empty());
        assert_eq!(runtime.state().player.control_linear_input, Vec3::ZERO);
        assert_eq!(runtime.state().player.control_angular_input, Vec3::ZERO);
        assert!(!runtime.state().player.boost);
        assert!(runtime.state().player.dampeners);
    }

    #[test]
    fn disconnected_control_lease_expires_safely_across_restart() {
        let directory = tempdir().expect("tempdir");
        {
            let mut runtime = Runtime::open(directory.path(), 131, 1).expect("runtime opens");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                    operation_sequence: 0,
                    operation_id: "disconnect-held-control".into(),
                    movement_epoch: 1,
                    input_sequence: 1,
                    linear_input: Vec3::new(1.0, 0.0, 0.0),
                    angular_input: Vec3::new(0.0, 0.0, 1.0),
                    boost: true,
                    jump: false,
                    dampeners: false,
                })
                .expect("control is durable before disconnect");
            runtime
                .advance(100)
                .expect("control advances before disconnect");
            assert_eq!(runtime.state().simulation_tick, 6);
            assert_eq!(
                runtime.state().player.control_expires_at_simulation_tick,
                18
            );
        }

        let mut recovered = Runtime::open(directory.path(), 131, 1).expect("runtime restarts");
        assert_eq!(recovered.state().simulation_tick, 6);
        assert_ne!(recovered.state().player.control_linear_input, Vec3::ZERO);
        recovered
            .advance(200)
            .expect("unrefreshed disconnected lease reaches its durable boundary");
        assert_eq!(recovered.state().simulation_tick, 18);
        assert_eq!(recovered.state().player.control_linear_input, Vec3::ZERO);
        assert_eq!(recovered.state().player.control_angular_input, Vec3::ZERO);
        assert!(!recovered.state().player.boost);
        assert!(recovered.state().player.dampeners);
    }

    #[test]
    fn authoritative_character_control_drives_eva_rotation_and_expires_safely() {
        let mut runtime = runtime();
        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "eva-control-1-1".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::new(0.0, 0.0, 1.0),
                boost: false,
                jump: false,
                dampeners: false,
            })
            .expect("bounded EVA control is accepted");
        let initial_position = runtime.state().player.position;

        runtime
            .advance(100)
            .expect("six authoritative substeps commit");

        assert_eq!(runtime.state().simulation_tick, 6);
        assert!(runtime.state().player.position.x > initial_position.x);
        assert!(runtime.state().player.linear_velocity.x > 0.0);
        assert!(runtime.state().player.angular_velocity.z > 0.0);
        assert_ne!(runtime.state().player.orientation, Quat::IDENTITY);
        assert_eq!(runtime.state().player.last_processed_input_sequence, 1);
        assert_eq!(
            runtime.state().player.control_expires_at_simulation_tick,
            18
        );

        runtime
            .advance(200)
            .expect("lease boundary commits another twelve substeps");

        assert_eq!(runtime.state().simulation_tick, 18);
        assert_eq!(runtime.state().player.control_linear_input, Vec3::ZERO);
        assert_eq!(runtime.state().player.control_angular_input, Vec3::ZERO);
        assert!(!runtime.state().player.boost);
        assert!(runtime.state().player.dampeners);
    }

    #[test]
    fn refreshed_character_controls_respect_content_speed_caps_with_both_dampener_modes() {
        let character = &content::manifest().character;
        for dampeners in [true, false] {
            for boost in [false, true] {
                let mut runtime = runtime();
                for input_sequence in 1_u64..=24 {
                    runtime
                        .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                            operation_sequence: 0,
                            operation_id: format!("held-{dampeners}-{boost}-{input_sequence}"),
                            movement_epoch: runtime.state().player.movement_epoch,
                            input_sequence,
                            linear_input: Vec3::new(1.0, 0.0, 0.0),
                            angular_input: Vec3::new(0.0, 0.0, 1.0),
                            boost,
                            jump: false,
                            dampeners,
                        })
                        .expect("refreshed held control is accepted");
                    runtime.advance(100).expect("held control advances");
                }
                let expected_linear = if boost {
                    character.boost_maximum_speed_m_s
                } else {
                    character.maximum_speed_m_s
                };
                assert!(
                    runtime.state().player.linear_velocity.magnitude() <= expected_linear + 0.001,
                    "dampeners={dampeners} boost={boost}: {:?}",
                    runtime.state().player.linear_velocity
                );
                assert!(
                    runtime.state().player.angular_velocity.magnitude()
                        <= character.maximum_angular_speed_radians_per_second + 0.001,
                    "dampeners={dampeners} boost={boost}: {:?}",
                    runtime.state().player.angular_velocity
                );
            }
        }
    }

    #[test]
    fn dampeners_off_preserve_boost_and_collision_inertia_without_adding_more_speed() {
        let mut runtime = runtime();
        runtime.state.player.linear_velocity = Vec3::new(20.0, 0.0, 0.0);
        runtime.state.player.angular_velocity = Vec3::new(0.0, 0.0, 3.0);
        runtime.rebuild_physics_for_test();
        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "coast-after-boost".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: false,
            })
            .expect("neutral inertial control is accepted");
        runtime.advance(17).expect("inertial step commits");
        assert!(runtime.state().player.linear_velocity.x > 19.9);
        assert!(runtime.state().player.angular_velocity.z > 2.9);

        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "normal-thrust-above-normal-cap".into(),
                movement_epoch: 1,
                input_sequence: 2,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::new(0.0, 0.0, 1.0),
                boost: false,
                jump: false,
                dampeners: false,
            })
            .expect("held control above the normal tier is accepted");
        let speed_before = runtime.state().player.linear_velocity.magnitude();
        let spin_before = runtime.state().player.angular_velocity.magnitude();
        runtime
            .advance(17)
            .expect("bounded inertial thrust commits");
        assert!(runtime.state().player.linear_velocity.magnitude() <= speed_before + 0.001);
        assert!(runtime.state().player.angular_velocity.magnitude() <= spin_before + 0.001);
    }

    #[test]
    fn quantized_eva_motion_matches_the_cross_platform_golden_fixture() {
        let mut runtime = runtime();
        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "cross-platform-motion-golden".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::new(0.25, -0.5, 0.75),
                angular_input: Vec3::new(0.3, -0.2, 0.4),
                boost: true,
                jump: false,
                dampeners: true,
            })
            .expect("golden control is accepted");
        runtime
            .advance(250)
            .expect("fifteen golden substeps commit");
        let player = &runtime.state().player;
        let tolerance = 0.000_05;
        assert!(
            player
                .position
                .squared_distance(Vec3::new(12.173_758, 4.151_999, 10.511_918))
                .sqrt()
                <= tolerance
        );
        assert!(
            player
                .linear_velocity
                .squared_distance(Vec3::new(1.358_482, -2.721_69, 3.926_687))
                .sqrt()
                <= tolerance
        );
        assert!(
            player
                .angular_velocity
                .squared_distance(Vec3::new(0.692_12, -0.461_254, 0.922_32))
                .sqrt()
                <= tolerance
        );
        let expected_orientation = Quat::new(0.044_743, -0.029_817, 0.059_621, 0.996_772);
        assert!((player.orientation.x - expected_orientation.x).abs() <= tolerance as f32);
        assert!((player.orientation.y - expected_orientation.y).abs() <= tolerance as f32);
        assert!((player.orientation.z - expected_orientation.z).abs() <= tolerance as f32);
        assert!((player.orientation.w - expected_orientation.w).abs() <= tolerance as f32);
        assert_eq!(runtime.state().simulation_tick, 15);
    }

    #[test]
    fn character_force_uses_radial_gravity_from_the_current_authoritative_position() {
        let character_mass = content::manifest().character.mass_kg;
        for position in [
            planet_center() + Vec3::new(planet_surface_radius_m() + 100.0, 0.0, 0.0),
            planet_center() + Vec3::new(0.0, planet_surface_radius_m() + 100.0, 0.0),
            planet_center() + Vec3::new(0.0, 0.0, planet_surface_radius_m() + 100.0),
        ] {
            let mut state = runtime().state().clone();
            set_test_player_position(&mut state.player, position);
            state.player.jetpack_enabled = true;
            state.player.dampeners = false;
            state.player.control_expires_at_simulation_tick = 1;
            let controls = physics_controls(&state, &state.player, &[], 0, None, true);
            let player = controls
                .iter()
                .find(|control| control.body_id == PLAYER_BODY_ID)
                .expect("living player receives a physics control");
            let expected = state.environment_at(position).gravity * character_mass;
            assert!((player.force_newtons.x - expected.x).abs() < 1.0e-6);
            assert!((player.force_newtons.y - expected.y).abs() < 1.0e-6);
            assert!((player.force_newtons.z - expected.z).abs() < 1.0e-6);
        }
    }

    #[test]
    fn suit_modes_and_environment_drive_authoritative_oxygen() {
        let mut runtime = runtime();
        runtime.relocate_player_for_test(Vec3::new(
            planet_center().x,
            planet_center().y + planet_surface_radius_m() + 10.0,
            planet_center().z,
        ));
        runtime.state.player.suit_oxygen_milli = 900;
        runtime
            .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                operation_sequence: 0,
                operation_id: "open-helmet".into(),
                helmet_closed: false,
                jetpack_enabled: true,
                magnetic_boots_enabled: false,
            })
            .expect("helmet opens in breathable atmosphere");
        assert!(!runtime.advance(250).expect("partial life support tick"));
        assert!(!runtime.advance(250).expect("partial life support tick"));
        assert!(!runtime.advance(250).expect("partial life support tick"));
        assert!(runtime.advance(250).expect("life support tick"));
        assert_eq!(runtime.state().player.suit_oxygen_milli, 925);

        runtime.relocate_player_for_test(Vec3::ZERO);
        for _ in 0..4 {
            runtime.advance(250).expect("vacuum life support tick");
        }
        assert_eq!(runtime.state().player.suit_oxygen_milli, 885);
    }

    #[test]
    fn oxygen_replay_accepts_only_the_exact_authoritative_one_second_outcome() {
        let vacuum = Vec3::new(100.0, 100.0, 100.0);
        let breathable = Vec3::new(
            planet_center().x,
            planet_center().y + planet_surface_radius_m() + 10.0,
            planet_center().z,
        );

        let apply = |mut state: WorldState, payload: EventPayload| {
            let event = state.prepare_system_event(payload);
            let result = state.apply_event(&event);
            (state, result)
        };

        let mut state = runtime().state().clone();
        set_test_player_position(&mut state.player, vacuum);
        state.player.helmet_closed = false;
        let (_, impossible) = apply(
            state.clone(),
            EventPayload::SuitOxygenChanged {
                player_id: "player-local".into(),
                previous_oxygen_milli: 1_000,
                new_oxygen_milli: 1,
            },
        );
        assert!(matches!(
            impossible,
            Err(IntentError::Rejected { ref code, .. }) if code == "replay_suit_oxygen_invalid"
        ));
        let (exact_vacuum, accepted) = apply(
            state,
            EventPayload::SuitOxygenChanged {
                player_id: "player-local".into(),
                previous_oxygen_milli: 1_000,
                new_oxygen_milli: 960,
            },
        );
        accepted.expect("open-vacuum exact delta applies");
        assert_eq!(exact_vacuum.player.suit_oxygen_milli, 960);

        let mut state = runtime().state().clone();
        set_test_player_position(&mut state.player, breathable);
        state.player.helmet_closed = false;
        state.player.suit_oxygen_milli = 900;
        let (_, impossible) = apply(
            state.clone(),
            EventPayload::SuitOxygenChanged {
                player_id: "player-local".into(),
                previous_oxygen_milli: 900,
                new_oxygen_milli: 860,
            },
        );
        assert!(matches!(
            impossible,
            Err(IntentError::Rejected { ref code, .. }) if code == "replay_suit_oxygen_invalid"
        ));
        let (exact_breathable, accepted) = apply(
            state,
            EventPayload::SuitOxygenChanged {
                player_id: "player-local".into(),
                previous_oxygen_milli: 900,
                new_oxygen_milli: 925,
            },
        );
        accepted.expect("open-breathable exact delta applies");
        assert_eq!(exact_breathable.player.suit_oxygen_milli, 925);

        let mut full_oxygen = runtime().state().clone();
        set_test_player_position(&mut full_oxygen.player, vacuum);
        let mut terminal = full_oxygen.clone();
        terminal.player.suit_oxygen_milli = 5;
        let impossible_death = terminal
            .oxygen_incapacitation_payload()
            .expect("terminal payload prepares");
        let (_, rejected) = apply(full_oxygen, impossible_death);
        assert!(matches!(
            rejected,
            Err(IntentError::Rejected { ref code, .. }) if code == "oxygen_not_depleted"
        ));

        let exact_death = terminal
            .oxygen_incapacitation_payload()
            .expect("sealed-vacuum terminal payload prepares");
        let (dead, accepted) = apply(terminal, exact_death);
        accepted.expect("sealed-vacuum five-to-zero death applies");
        assert!(matches!(
            dead.player.life_state,
            PlayerLifeState::Incapacitated { .. }
        ));
    }

    #[test]
    fn life_support_schedules_every_player_in_canonical_order_and_recovers_exactly() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 143, 1_000).expect("runtime opens");
        runtime
            .admit_development_player("player-remote")
            .expect("secondary player admits");
        for player in runtime.state.player.by_id.values_mut() {
            set_test_player_position(player, Vec3::new(100.0, 100.0, 100.0));
            player.helmet_closed = true;
            player.suit_oxygen_milli = 1_000;
            player.linear_velocity = Vec3::ZERO;
        }
        runtime.rebuild_physics_for_test();
        runtime.persist_snapshot().expect("fixture persists");

        for _ in 0..3 {
            assert!(
                !runtime
                    .advance(250)
                    .expect("partial life-support second advances")
            );
        }
        assert!(
            runtime
                .advance(250)
                .expect("one life-support second advances")
        );
        assert_eq!(
            runtime
                .state()
                .player
                .get("player-local")
                .unwrap()
                .suit_oxygen_milli,
            995
        );
        assert_eq!(
            runtime
                .state()
                .player
                .get("player-remote")
                .unwrap()
                .suit_oxygen_milli,
            995
        );
        let journal =
            fs::read_to_string(directory.path().join("events.ndjson")).expect("journal reads");
        let lifecycle_targets = journal
            .lines()
            .map(|line| serde_json::from_str::<CanonicalEvent>(line).expect("event parses"))
            .filter_map(|event| match event.payload {
                EventPayload::SuitOxygenChanged { player_id, .. } => Some(player_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            lifecycle_targets,
            vec!["player-local".to_owned(), "player-remote".to_owned()]
        );
        let expected_hash = runtime.state().state_hash();
        let expected_sequence = runtime.state().event_sequence;

        drop(runtime);
        let recovered = Runtime::open(directory.path(), 143, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert_eq!(recovered.state().event_sequence, expected_sequence);
    }

    #[test]
    fn secondary_oxygen_death_isolates_inventory_player_grid_and_contact_state() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 144, 1_000).expect("runtime opens");
        runtime
            .admit_development_player("player-remote")
            .expect("secondary player admits");
        let remote_inventory_id = runtime
            .state
            .player
            .get("player-remote")
            .unwrap()
            .inventory_id
            .clone();
        runtime
            .state
            .inventories
            .get_mut(PLAYER_INVENTORY_ID)
            .unwrap()
            .contents
            .components -= 3;
        runtime
            .state
            .inventories
            .get_mut(&remote_inventory_id)
            .unwrap()
            .contents
            .components = 3;
        let remote = runtime.state.player.get_mut("player-remote").unwrap();
        remote.suit_oxygen_milli = 5;
        remote.linear_velocity = Vec3::new(3.0, 2.0, 1.0);
        remote.angular_velocity = Vec3::new(0.5, 0.25, 0.125);
        remote.control_linear_input = Vec3::new(1.0, 0.0, 0.0);
        remote.control_angular_input = Vec3::new(0.0, 1.0, 0.0);
        remote.boost = true;
        remote.dampeners = false;
        let local_contact = ContactPairKey {
            body_a: player_body_id("player-local"),
            collider_a: player_collider_id("player-local"),
            body_b: STARTER_GRID_ID.into(),
            collider_b: "block-core".into(),
        };
        let remote_contact = ContactPairKey {
            body_a: player_body_id("player-remote"),
            collider_a: player_collider_id("player-remote"),
            body_b: STARTER_GRID_ID.into(),
            collider_b: "block-power".into(),
        };
        runtime.state.active_contact_pairs =
            BTreeSet::from([local_contact.clone(), remote_contact]);
        let grid = runtime.state.grids.get_mut(STARTER_GRID_ID).unwrap();
        grid.control_linear_input = Vec3::new(0.25, 0.5, 0.75);
        grid.control_angular_input = Vec3::new(0.1, 0.2, 0.3);
        grid.dampeners = false;
        let primary_before = runtime.state.player.primary().clone();
        let grid_before = runtime.state.grids[STARTER_GRID_ID].clone();
        runtime.persist_snapshot().expect("fixture persists");
        runtime
            .life_support_elapsed_millis_by_player
            .insert("player-remote".into(), 999);

        assert!(runtime.advance(1).expect("secondary death commits"));
        assert_eq!(runtime.state().player.primary(), &primary_before);
        assert_eq!(runtime.state().grids[STARTER_GRID_ID], grid_before);
        assert_eq!(
            runtime.state().active_contact_pairs,
            BTreeSet::from([local_contact])
        );
        let remote = runtime.state().player.get("player-remote").unwrap();
        assert!(matches!(
            remote.life_state,
            PlayerLifeState::Incapacitated { .. }
        ));
        assert_eq!(remote.suit_oxygen_milli, 0);
        assert_eq!(remote.linear_velocity, Vec3::ZERO);
        assert_eq!(remote.angular_velocity, Vec3::ZERO);
        assert_eq!(
            runtime.state().inventories[&remote_inventory_id].contents,
            InventoryContents::default()
        );
        let death_drop = runtime
            .state()
            .death_drops
            .values()
            .next()
            .expect("secondary carried inventory drops");
        assert_eq!(death_drop.owner_player_id, "player-remote");
        assert_eq!(
            runtime.state().inventories[&death_drop.inventory_id]
                .contents
                .components,
            3
        );
        assert!(runtime.state().conservation().valid);
        let expected_hash = runtime.state().state_hash();

        drop(runtime);
        let recovered = Runtime::open(directory.path(), 144, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn simultaneous_player_deaths_are_unique_ordered_and_recover_exactly() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 145, 1_000).expect("runtime opens");
        runtime
            .admit_development_player("player-remote")
            .expect("secondary player admits");
        for (player_id, player) in &mut runtime.state.player.by_id {
            player.suit_oxygen_milli = 5;
            runtime
                .life_support_elapsed_millis_by_player
                .insert(player_id.clone(), 999);
        }
        runtime.persist_snapshot().expect("fixture persists");

        assert!(runtime.advance(1).expect("simultaneous deaths commit"));
        let local = runtime.state().player.get("player-local").unwrap();
        let remote = runtime.state().player.get("player-remote").unwrap();
        let PlayerLifeState::Incapacitated {
            death_id: local_death,
            ..
        } = &local.life_state
        else {
            panic!("local player must be incapacitated");
        };
        let PlayerLifeState::Incapacitated {
            death_id: remote_death,
            ..
        } = &remote.life_state
        else {
            panic!("remote player must be incapacitated");
        };
        assert_ne!(local_death, remote_death);
        assert_eq!(local_death, "death-player-local-1");
        assert_eq!(remote_death, "death-player-remote-2");
        let journal =
            fs::read_to_string(directory.path().join("events.ndjson")).expect("journal reads");
        let targets = journal
            .lines()
            .map(|line| serde_json::from_str::<CanonicalEvent>(line).expect("event parses"))
            .filter_map(|event| match event.payload {
                EventPayload::PlayerIncapacitated { player_id, .. } => Some(player_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(targets, vec!["player-local", "player-remote"]);
        let expected_hash = runtime.state().state_hash();

        drop(runtime);
        let recovered = Runtime::open(directory.path(), 145, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn secondary_respawn_is_actor_scoped_idempotent_and_exactly_recoverable() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 146, 1_000).expect("runtime opens");
        runtime
            .admit_development_player("player-remote")
            .expect("secondary player admits");
        runtime
            .state
            .player
            .get_mut("player-remote")
            .unwrap()
            .suit_oxygen_milli = 5;
        runtime.persist_snapshot().expect("fixture persists");
        runtime
            .life_support_elapsed_millis_by_player
            .insert("player-remote".into(), 999);
        runtime.advance(1).expect("secondary death commits");
        let primary_before = runtime.state().player.primary().clone();
        let respawn = ClientMessage::RespawnPlayer {
            operation_sequence: 0,
            operation_id: "remote-respawn".into(),
        };

        let first = runtime
            .execute_next_as_for_fixture("player-remote", &respawn)
            .expect("secondary recovery commits");
        let expected_hash = runtime.state().state_hash();
        let expected_sequence = runtime.state().event_sequence;
        assert_eq!(runtime.state().player.primary(), &primary_before);
        let remote = runtime.state().player.get("player-remote").unwrap();
        assert_eq!(remote.life_state, PlayerLifeState::Alive);
        assert_eq!(remote.movement_epoch, 2);
        assert_eq!(
            runtime
                .execute_next_as_for_fixture("player-remote", &respawn)
                .expect("secondary recovery retry is idempotent"),
            first
        );
        assert_eq!(runtime.state().state_hash(), expected_hash);
        assert_eq!(runtime.state().event_sequence, expected_sequence);
        assert!(
            runtime
                .state()
                .processed_operation("player-local", "remote-respawn")
                .is_none()
        );
        assert_eq!(
            runtime
                .state()
                .processed_operation("player-remote", "remote-respawn"),
            Some(&first)
        );

        drop(runtime);
        let recovered = Runtime::open(directory.path(), 146, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn replay_rejects_malformed_lifecycle_targets_without_mutation() {
        let mut runtime = runtime();
        runtime
            .admit_development_player("player-remote")
            .expect("secondary player admits");
        let base = runtime.state().clone();
        let reject = |state: &WorldState, event: CanonicalEvent, expected_code: &str| {
            let mut candidate = state.clone();
            let before = candidate.state_hash();
            assert!(matches!(
                candidate.apply_event(&event),
                Err(IntentError::Rejected { ref code, .. }) if code == expected_code
            ));
            assert_eq!(candidate.state_hash(), before);
        };

        reject(
            &base,
            base.prepare_system_event(EventPayload::SuitOxygenChanged {
                player_id: "player-ghost".into(),
                previous_oxygen_milli: 1_000,
                new_oxygen_milli: 995,
            }),
            "replay_lifecycle_envelope_invalid",
        );
        reject(
            &base,
            base.new_event(
                None,
                "system",
                None,
                EventPayload::SuitModeChanged {
                    helmet_closed: false,
                    jetpack_enabled: true,
                    magnetic_boots_enabled: false,
                },
            ),
            "replay_lifecycle_envelope_invalid",
        );
        let mut dead = base.clone();
        dead.player
            .get_mut("player-remote")
            .unwrap()
            .suit_oxygen_milli = 5;
        let death = dead
            .oxygen_incapacitation_payload_for("player-remote")
            .expect("secondary death prepares");
        let death_event = dead.prepare_system_event(death);
        dead.apply_event(&death_event)
            .expect("secondary death applies");
        let remote_respawn = dead
            .player_respawn_payload_for("player-remote")
            .expect("secondary respawn prepares");
        reject(
            &dead,
            dead.new_test_human_event(
                "player-local",
                "forged-cross-player-respawn",
                remote_respawn,
            ),
            "player_already_alive",
        );
    }

    #[test]
    fn secondary_death_commit_failpoints_recover_prior_or_durable_actor_state() {
        for (failpoint, durable) in [
            (AppendFailpoint::BeforeWrite, false),
            (AppendFailpoint::AfterSync, true),
        ] {
            let directory = tempdir().expect("tempdir");
            let before_state;
            {
                let mut runtime =
                    Runtime::open(directory.path(), 147, 1_000).expect("runtime opens");
                runtime
                    .admit_development_player("player-remote")
                    .expect("secondary player admits");
                runtime
                    .state
                    .player
                    .get_mut("player-remote")
                    .unwrap()
                    .suit_oxygen_milli = 5;
                runtime.persist_snapshot().expect("fixture persists");
                before_state = runtime.state().clone();
                runtime
                    .life_support_elapsed_millis_by_player
                    .insert("player-remote".into(), 999);
                runtime.store.set_append_failpoint(failpoint);
                assert!(matches!(
                    runtime.advance(1),
                    Err(RuntimeError::Persistence(
                        PersistenceError::InjectedFailure(_)
                    ))
                ));
                assert!(runtime.is_halted());
            }

            let recovered = Runtime::open(directory.path(), 147, 1_000).expect("runtime recovers");
            let expected_hash = if durable {
                let journal = fs::read_to_string(directory.path().join("events.ndjson"))
                    .expect("durable journal reads");
                let event = serde_json::from_str::<CanonicalEvent>(
                    journal.lines().next().expect("durable death event exists"),
                )
                .expect("durable death event parses");
                let mut expected = before_state.clone();
                expected.apply_event(&event).expect("durable death replays");
                expected.state_hash()
            } else {
                before_state.state_hash()
            };
            assert_eq!(recovered.state().state_hash(), expected_hash);
            assert_eq!(
                matches!(
                    recovered
                        .state()
                        .player
                        .get("player-remote")
                        .unwrap()
                        .life_state,
                    PlayerLifeState::Incapacitated { .. }
                ),
                durable
            );
        }
    }

    #[test]
    fn oxygen_death_moves_inventory_once_gates_work_and_respawns_for_free() {
        let mut runtime = runtime();
        runtime
            .state
            .inventories
            .get_mut(PLAYER_INVENTORY_ID)
            .expect("player inventory")
            .contents = InventoryContents {
            ore: 4,
            refined_material: 3,
            components: 24,
        };
        runtime.state.ledger.genesis_ore = 4;
        runtime.state.ledger.genesis_refined = 3;
        runtime.state.player.suit_oxygen_milli = 5;
        let death_position = runtime.state().player.position;
        let experience_before = runtime.state().player.experience;
        let career_before = runtime.state().player.career.clone();
        let ledger_before = runtime.state().ledger.clone();
        let carried_before = runtime.state().inventories[PLAYER_INVENTORY_ID]
            .contents
            .clone();

        for _ in 0..3 {
            assert!(!runtime.advance(250).expect("partial oxygen second"));
        }
        assert!(runtime.advance(250).expect("terminal oxygen second"));
        let death_sequence = runtime.state().event_sequence;
        assert_eq!(runtime.state().player.suit_oxygen_milli, 0);
        assert!(!runtime.state().player.jetpack_enabled);
        let PlayerLifeState::Incapacitated { death_id, cause } = &runtime.state().player.life_state
        else {
            panic!("zero oxygen must incapacitate the player");
        };
        assert_eq!(death_id, &format!("death-player-local-{death_sequence}"));
        assert_eq!(*cause, PlayerDeathCause::OxygenDepleted);
        assert_eq!(
            runtime.state().inventories[PLAYER_INVENTORY_ID].contents,
            InventoryContents::default()
        );
        assert_eq!(runtime.state().death_drops.len(), 1);
        let drop = &runtime.state().death_drops[&format!("drop-player-local-{death_sequence}")];
        assert_eq!(drop.position, death_position);
        assert_eq!(
            drop.death_id,
            format!("death-player-local-{death_sequence}")
        );
        assert_eq!(drop.owner_player_id, "player-local");
        assert_eq!(drop.created_event_sequence, death_sequence);
        let drop_inventory_id = drop.inventory_id.clone();
        assert_eq!(
            runtime.state().inventories[&drop_inventory_id].contents,
            carried_before
        );
        assert_eq!(runtime.state().player.experience, experience_before);
        assert_eq!(runtime.state().player.career, career_before);
        assert_eq!(runtime.state().ledger, ledger_before);
        assert!(runtime.state().conservation().valid);

        for _ in 0..8 {
            assert!(!runtime.advance(250).expect("incapacitated tick is inert"));
        }
        assert_eq!(runtime.state().event_sequence, death_sequence);
        assert_eq!(runtime.state().death_drops.len(), 1);

        let blocked_messages = [
            ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "dead-control".into(),
                movement_epoch: runtime.state().player.movement_epoch,
                input_sequence: 1,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            },
            ClientMessage::SetSuitMode {
                operation_sequence: 0,
                operation_id: "dead-suit".into(),
                helmet_closed: false,
                jetpack_enabled: false,
                magnetic_boots_enabled: false,
            },
            ClientMessage::MineVoxel {
                operation_sequence: 0,
                operation_id: "dead-mine".into(),
                coordinate: IVec3::ZERO,
            },
            ClientMessage::RefineOre {
                operation_sequence: 0,
                operation_id: "dead-refine".into(),
                inventory_id: PLAYER_INVENTORY_ID.into(),
                batches: 1,
            },
            ClientMessage::CraftComponent {
                operation_sequence: 0,
                operation_id: "dead-craft".into(),
                inventory_id: PLAYER_INVENTORY_ID.into(),
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_sequence: 0,
                operation_id: "dead-transfer".into(),
                source_inventory_id: drop_inventory_id.clone(),
                destination_inventory_id: PLAYER_INVENTORY_ID.into(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
            ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "dead-build".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, 1, 0),
                kind: BlockKind::Structural,
                orientation: 0,
            },
            ClientMessage::WeldBlock {
                operation_sequence: 0,
                operation_id: "dead-weld".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: "block-core".into(),
            },
            ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "dead-control".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                dampeners: false,
            },
            ClientMessage::ToggleGridAnchor {
                operation_sequence: 0,
                operation_id: "dead-anchor".into(),
                grid_id: STARTER_GRID_ID.into(),
            },
            ClientMessage::DamageBlock {
                operation_sequence: 0,
                operation_id: "dead-damage".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: "block-core".into(),
            },
        ];
        let dead_hash = runtime.state().state_hash();
        for message in blocked_messages {
            assert!(matches!(
                runtime.execute_next_for_fixture(&message),
                Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                    if code == "player_incapacitated"
            ));
            assert_eq!(runtime.state().state_hash(), dead_hash);
        }

        let respawn = ClientMessage::RespawnPlayer {
            operation_sequence: 0,
            operation_id: "recover-once".into(),
        };
        let first = runtime
            .execute_next_for_fixture(&respawn)
            .expect("recovery accepted");
        let recovered_hash = runtime.state().state_hash();
        let second = runtime
            .execute_next_for_fixture(&respawn)
            .expect("recovery retry accepted");
        assert_eq!(first, second);
        assert_eq!(runtime.state().state_hash(), recovered_hash);
        assert_eq!(runtime.state().event_sequence, death_sequence + 1);
        assert_eq!(runtime.state().player.life_state, PlayerLifeState::Alive);
        assert_eq!(runtime.state().player.movement_epoch, 2);
        assert_eq!(runtime.state().player.last_received_input_sequence, 0);
        assert_eq!(runtime.state().player.last_processed_input_sequence, 0);
        assert!(runtime.state().player.pending_control_frames.is_empty());
        assert_eq!(runtime.state().player.linear_velocity, Vec3::ZERO);
        assert_eq!(runtime.state().player.angular_velocity, Vec3::ZERO);
        assert_eq!(
            runtime.state().player.position,
            content::manifest().survival.proof_recovery_position
        );
        assert_eq!(
            runtime.state().player.suit_oxygen_milli,
            content::manifest().survival.respawn_oxygen_milli
        );
        assert_eq!(
            runtime.state().inventories[PLAYER_INVENTORY_ID].contents,
            InventoryContents::default()
        );
        assert_eq!(runtime.state().death_drops.len(), 1);
        assert_eq!(runtime.state().player.experience, experience_before);
        assert_eq!(runtime.state().player.career, career_before);
        assert_eq!(runtime.state().ledger, ledger_before);
        assert!(runtime.state().conservation().valid);

        let sealed_intents = [
            (
                ClientMessage::RefineOre {
                    operation_sequence: 0,
                    operation_id: "drop-refine".into(),
                    inventory_id: drop_inventory_id.clone(),
                    batches: 1,
                },
                "physical_machine_required",
            ),
            (
                ClientMessage::CraftComponent {
                    operation_sequence: 0,
                    operation_id: "drop-craft".into(),
                    inventory_id: drop_inventory_id.clone(),
                    quantity: 1,
                },
                "physical_machine_required",
            ),
            (
                ClientMessage::TransferInventory {
                    operation_sequence: 0,
                    operation_id: "drop-transfer-source".into(),
                    source_inventory_id: drop_inventory_id.clone(),
                    destination_inventory_id: PLAYER_INVENTORY_ID.into(),
                    resource: ResourceKind::Component,
                    quantity: 1,
                },
                "dropped_inventory_sealed",
            ),
            (
                ClientMessage::TransferInventory {
                    operation_sequence: 0,
                    operation_id: "drop-transfer-destination".into(),
                    source_inventory_id: PLAYER_INVENTORY_ID.into(),
                    destination_inventory_id: drop_inventory_id.clone(),
                    resource: ResourceKind::Component,
                    quantity: 1,
                },
                "dropped_inventory_sealed",
            ),
        ];
        for (intent, expected_code) in sealed_intents {
            assert!(matches!(
                runtime.execute_next_for_fixture(&intent),
                Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                    if code == expected_code
            ));
        }

        let replay_payloads = [
            EventPayload::OreRefined {
                inventory_id: drop_inventory_id.clone(),
                batches: 1,
            },
            EventPayload::ComponentCrafted {
                inventory_id: drop_inventory_id.clone(),
                quantity: 1,
            },
            EventPayload::InventoryTransferred {
                source_inventory_id: drop_inventory_id.clone(),
                destination_inventory_id: PLAYER_INVENTORY_ID.into(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
            EventPayload::InventoryTransferred {
                source_inventory_id: PLAYER_INVENTORY_ID.into(),
                destination_inventory_id: drop_inventory_id,
                resource: ResourceKind::Component,
                quantity: 1,
            },
        ];
        for (index, payload) in replay_payloads.into_iter().enumerate() {
            let expected_code = if matches!(
                payload,
                EventPayload::OreRefined { .. } | EventPayload::ComponentCrafted { .. }
            ) {
                "replay_physical_machine_required"
            } else {
                "dropped_inventory_sealed"
            };
            let event = runtime.state().new_test_human_event(
                "player-local",
                format!("forged-drop-operation-{index}"),
                payload,
            );
            let mut candidate = runtime.state().clone();
            let before = candidate.state_hash();
            assert!(matches!(
                candidate.apply_event(&event),
                Err(IntentError::Rejected { ref code, .. }) if code == expected_code
            ));
            assert_eq!(candidate.state_hash(), before);
        }
    }

    #[test]
    fn empty_inventory_oxygen_death_does_not_create_an_empty_drop() {
        let mut runtime = runtime();
        runtime
            .execute_next_for_fixture(&ClientMessage::TransferInventory {
                operation_sequence: 0,
                operation_id: "stow-before-death".into(),
                source_inventory_id: PLAYER_INVENTORY_ID.into(),
                destination_inventory_id: "inventory-cargo-starter".into(),
                resource: ResourceKind::Component,
                quantity: 24,
            })
            .expect("carried inventory stows");
        runtime.state.player.suit_oxygen_milli = 5;
        for _ in 0..4 {
            runtime.advance(250).expect("oxygen advances");
        }
        assert!(matches!(
            runtime.state().player.life_state,
            PlayerLifeState::Incapacitated { .. }
        ));
        assert!(runtime.state().death_drops.is_empty());
        assert_eq!(runtime.state().inventories.len(), 3);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn oxygen_death_mid_motion_clears_player_kinematics_controls_and_contacts() {
        let mut runtime = runtime();
        runtime.state.player.suit_oxygen_milli = 5;
        runtime.state.player.linear_velocity = Vec3::new(3.0, -2.0, 1.0);
        runtime.state.player.angular_velocity = Vec3::new(0.5, 0.25, -0.75);
        runtime.state.player.control_linear_input = Vec3::new(1.0, 0.0, 0.0);
        runtime.state.player.control_angular_input = Vec3::new(0.0, 0.0, 1.0);
        runtime.state.player.boost = true;
        runtime.state.player.dampeners = false;
        runtime.state.player.surface_contact = true;
        runtime.state.player.control_expires_at_simulation_tick =
            content::manifest().character.control_lease_ticks;
        runtime.state.active_contact_pairs.insert(ContactPairKey {
            body_a: PLAYER_BODY_ID.into(),
            collider_a: PLAYER_COLLIDER_ID.into(),
            body_b: STARTER_GRID_ID.into(),
            collider_b: "block-core".into(),
        });
        runtime
            .life_support_elapsed_millis_by_player
            .insert("player-local".into(), 999);

        runtime
            .advance(1)
            .expect("terminal life-support millisecond incapacitates the moving player");

        assert!(matches!(
            runtime.state().player.life_state,
            PlayerLifeState::Incapacitated { .. }
        ));
        assert_eq!(runtime.state().player.linear_velocity, Vec3::ZERO);
        assert_eq!(runtime.state().player.angular_velocity, Vec3::ZERO);
        assert_eq!(runtime.state().player.control_linear_input, Vec3::ZERO);
        assert_eq!(runtime.state().player.control_angular_input, Vec3::ZERO);
        assert!(!runtime.state().player.boost);
        assert!(runtime.state().player.dampeners);
        assert!(!runtime.state().player.surface_contact);
        assert_eq!(
            runtime.state().player.control_expires_at_simulation_tick,
            runtime.state().simulation_tick
        );
        assert!(
            runtime
                .state()
                .active_contact_pairs
                .iter()
                .all(|pair| !contact_key_involves_player(pair))
        );
    }

    #[test]
    fn incapacitation_replay_rejects_tampering_and_clears_latched_controls() {
        let mut state = runtime().state().clone();
        state.player.suit_oxygen_milli = 5;
        state.player.last_received_input_sequence = 1;
        state
            .player
            .pending_control_frames
            .push_back(PlayerControlFrame {
                input_sequence: 1,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::new(0.0, 0.0, 1.0),
                boost: true,
                jump: false,
                dampeners: false,
                expires_at_simulation_tick: 18,
            });
        state
            .grids
            .get_mut(STARTER_GRID_ID)
            .unwrap()
            .control_linear_input = Vec3::new(1.0, 0.0, 0.0);
        state
            .grids
            .get_mut(STARTER_GRID_ID)
            .unwrap()
            .control_angular_input = Vec3::new(0.0, 1.0, 0.0);
        state.grids.get_mut(STARTER_GRID_ID).unwrap().dampeners = false;
        let canonical = state
            .oxygen_incapacitation_payload()
            .expect("incapacitation prepares");
        let reject = |payload: EventPayload| {
            let event = state.prepare_system_event(payload);
            let mut candidate = state.clone();
            let before = candidate.state_hash();
            assert!(matches!(
                candidate.apply_event(&event),
                Err(IntentError::Rejected { ref code, .. })
                    if code == "replay_player_incapacitation_invalid"
            ));
            assert_eq!(candidate.state_hash(), before);
        };

        let mut wrong_death = canonical.clone();
        let EventPayload::PlayerIncapacitated { death_id, .. } = &mut wrong_death else {
            unreachable!();
        };
        death_id.push_str("-forged");
        reject(wrong_death);

        let mut wrong_position = canonical.clone();
        let EventPayload::PlayerIncapacitated {
            address, position, ..
        } = &mut wrong_position
        else {
            unreachable!();
        };
        *address = exact_test_address(*position + Vec3::new(0.5, 0.0, 0.0));
        reject(wrong_position);

        let mut wrong_previous_oxygen = canonical.clone();
        let EventPayload::PlayerIncapacitated {
            previous_oxygen_milli,
            ..
        } = &mut wrong_previous_oxygen
        else {
            unreachable!();
        };
        *previous_oxygen_milli += 1;
        reject(wrong_previous_oxygen);

        let mut wrong_contents = canonical.clone();
        let EventPayload::PlayerIncapacitated {
            dropped_inventory, ..
        } = &mut wrong_contents
        else {
            unreachable!();
        };
        dropped_inventory
            .as_mut()
            .expect("nonempty carried inventory creates drop")
            .contents
            .components -= 1;
        reject(wrong_contents);

        let mut wrong_capacity = canonical.clone();
        let EventPayload::PlayerIncapacitated {
            dropped_inventory, ..
        } = &mut wrong_capacity
        else {
            unreachable!();
        };
        dropped_inventory
            .as_mut()
            .expect("drop exists")
            .capacity_liters += 1;
        reject(wrong_capacity);

        let mut wrong_dropped_inventory_id = canonical.clone();
        let EventPayload::PlayerIncapacitated {
            dropped_inventory, ..
        } = &mut wrong_dropped_inventory_id
        else {
            unreachable!();
        };
        dropped_inventory
            .as_mut()
            .expect("drop exists")
            .inventory_id
            .push_str("-forged");
        reject(wrong_dropped_inventory_id);

        let mut wrong_linked_inventory_id = canonical.clone();
        let EventPayload::PlayerIncapacitated { death_drop, .. } = &mut wrong_linked_inventory_id
        else {
            unreachable!();
        };
        death_drop
            .as_mut()
            .expect("drop metadata exists")
            .inventory_id
            .push_str("-forged");
        reject(wrong_linked_inventory_id);

        let mut wrong_owner = canonical.clone();
        let EventPayload::PlayerIncapacitated { death_drop, .. } = &mut wrong_owner else {
            unreachable!();
        };
        death_drop
            .as_mut()
            .expect("drop metadata exists")
            .owner_player_id = "attacker".into();
        reject(wrong_owner);

        let mut wrong_drop_death_id = canonical.clone();
        let EventPayload::PlayerIncapacitated { death_drop, .. } = &mut wrong_drop_death_id else {
            unreachable!();
        };
        death_drop
            .as_mut()
            .expect("drop metadata exists")
            .death_id
            .push_str("-forged");
        reject(wrong_drop_death_id);

        let event = state.prepare_system_event(canonical);
        state.apply_event(&event).expect("canonical death applies");
        let grid = &state.grids[STARTER_GRID_ID];
        assert_eq!(grid.control_linear_input, Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(grid.control_angular_input, Vec3::new(0.0, 1.0, 0.0));
        assert!(!grid.dampeners);
        assert!(state.player.pending_control_frames.is_empty());
        assert!(state.conservation().valid);
    }

    #[test]
    fn oxygen_death_commit_failpoints_recover_the_complete_prior_or_durable_drop() {
        for (failpoint, durable) in [
            (AppendFailpoint::BeforeWrite, false),
            (AppendFailpoint::AfterSync, true),
        ] {
            let directory = tempdir().expect("tempdir");
            let before_state;
            {
                let mut runtime = Runtime::open(directory.path(), 91, 100).expect("runtime opens");
                runtime.state.player.suit_oxygen_milli = 5;
                runtime
                    .persist_snapshot()
                    .expect("low-oxygen state persists");
                before_state = runtime.state().clone();
                runtime
                    .life_support_elapsed_millis_by_player
                    .insert("player-local".into(), 999);
                runtime.store.set_append_failpoint(failpoint);
                assert!(matches!(
                    runtime.advance(1),
                    Err(RuntimeError::Persistence(
                        PersistenceError::InjectedFailure(_)
                    ))
                ));
                assert!(runtime.is_halted());
                assert_eq!(runtime.state().state_hash(), before_state.state_hash());
                assert_eq!(runtime.state().player.life_state, PlayerLifeState::Alive);
                assert!(runtime.state().death_drops.is_empty());
            }

            let recovered =
                Runtime::open(directory.path(), 91, 100).expect("runtime recovers after failure");
            if durable {
                let journal = fs::read_to_string(directory.path().join("events.ndjson"))
                    .expect("journal reads");
                let event: CanonicalEvent = serde_json::from_str(
                    journal.lines().last().expect("durable death event exists"),
                )
                .expect("death event parses");
                let mut expected = before_state.clone();
                expected.apply_event(&event).expect("durable death replays");
                assert_eq!(recovered.state().state_hash(), expected.state_hash());
                assert!(matches!(
                    recovered.state().player.life_state,
                    PlayerLifeState::Incapacitated { .. }
                ));
                assert_eq!(recovered.state().death_drops.len(), 1);
            } else {
                assert_eq!(recovered.state().state_hash(), before_state.state_hash());
                assert_eq!(recovered.state().player.life_state, PlayerLifeState::Alive);
                assert!(recovered.state().death_drops.is_empty());
            }
            assert!(recovered.state().conservation().valid);
        }
    }

    #[test]
    fn respawn_commit_failpoints_recover_the_complete_dead_or_alive_state() {
        for (failpoint, durable) in [
            (AppendFailpoint::BeforeWrite, false),
            (AppendFailpoint::AfterSync, true),
        ] {
            let directory = tempdir().expect("tempdir");
            let dead_state;
            {
                let mut runtime = Runtime::open(directory.path(), 92, 100).expect("runtime opens");
                runtime.state.player.suit_oxygen_milli = 5;
                runtime
                    .persist_snapshot()
                    .expect("low-oxygen state persists");
                for _ in 0..4 {
                    runtime.advance(250).expect("oxygen death commits");
                }
                dead_state = runtime.state().clone();
                runtime.persist_snapshot().expect("dead state persists");
                runtime.store.set_append_failpoint(failpoint);
                assert!(matches!(
                    runtime.execute_next_for_fixture(&ClientMessage::RespawnPlayer {
                        operation_sequence: 0,
                        operation_id: "recover-after-failure".into(),
                    }),
                    Err(RuntimeError::Persistence(
                        PersistenceError::InjectedFailure(_)
                    ))
                ));
                assert!(runtime.is_halted());
                assert_eq!(runtime.state().state_hash(), dead_state.state_hash());
            }

            let recovered =
                Runtime::open(directory.path(), 92, 100).expect("runtime recovers after failure");
            if durable {
                let journal = fs::read_to_string(directory.path().join("events.ndjson"))
                    .expect("journal reads");
                let event: CanonicalEvent = serde_json::from_str(
                    journal
                        .lines()
                        .last()
                        .expect("durable respawn event exists"),
                )
                .expect("respawn event parses");
                let mut expected = dead_state.clone();
                expected
                    .apply_event(&event)
                    .expect("durable respawn replays");
                assert_eq!(recovered.state().state_hash(), expected.state_hash());
                assert_eq!(recovered.state().player.life_state, PlayerLifeState::Alive);
            } else {
                assert_eq!(recovered.state().state_hash(), dead_state.state_hash());
                assert!(matches!(
                    recovered.state().player.life_state,
                    PlayerLifeState::Incapacitated { .. }
                ));
            }
            assert_eq!(recovered.state().death_drops.len(), 1);
            assert!(recovered.state().conservation().valid);
        }
    }

    #[test]
    fn respawn_replay_rejects_client_selected_outcomes_before_mutation() {
        let mut dead = runtime().state().clone();
        dead.player.suit_oxygen_milli = 5;
        let death = dead
            .oxygen_incapacitation_payload()
            .expect("death prepares");
        let death_event = dead.prepare_system_event(death);
        dead.apply_event(&death_event).expect("death applies");
        let canonical = dead.player_respawn_payload().expect("respawn prepares");
        let reject = |payload: EventPayload| {
            let event = dead.new_test_human_event("player-local", "tampered-respawn", payload);
            let mut candidate = dead.clone();
            let before = candidate.state_hash();
            assert!(matches!(
                candidate.apply_event(&event),
                Err(IntentError::Rejected { ref code, .. })
                    if code == "replay_player_respawn_invalid"
            ));
            assert_eq!(candidate.state_hash(), before);
        };

        let mut wrong_position = canonical.clone();
        let EventPayload::PlayerRespawned {
            address, position, ..
        } = &mut wrong_position
        else {
            unreachable!();
        };
        *address = exact_test_address(*position + Vec3::new(1.0, 0.0, 0.0));
        reject(wrong_position);

        let mut wrong_oxygen = canonical.clone();
        let EventPayload::PlayerRespawned {
            suit_oxygen_milli, ..
        } = &mut wrong_oxygen
        else {
            unreachable!();
        };
        *suit_oxygen_milli -= 1;
        reject(wrong_oxygen);

        let mut wrong_mode = canonical.clone();
        let EventPayload::PlayerRespawned { helmet_closed, .. } = &mut wrong_mode else {
            unreachable!();
        };
        *helmet_closed = !*helmet_closed;
        reject(wrong_mode);

        let mut wrong_death = canonical;
        let EventPayload::PlayerRespawned { death_id, .. } = &mut wrong_death else {
            unreachable!();
        };
        death_id.push_str("-forged");
        reject(wrong_death);
    }

    #[test]
    fn respawn_uses_a_deterministic_clear_fallback_when_the_primary_point_is_blocked() {
        let mut state = runtime().state().clone();
        state.player.suit_oxygen_milli = 5;
        let death = state
            .oxygen_incapacitation_payload()
            .expect("death prepares");
        let death_event = state.prepare_system_event(death);
        state.apply_event(&death_event).expect("death applies");

        let primary = content::manifest().survival.proof_recovery_position;
        set_test_grid_position(
            state.grids.get_mut(STARTER_GRID_ID).expect("starter grid"),
            primary,
        );
        assert!(!state.proof_recovery_position_is_clear(primary));

        let payload = state.player_respawn_payload().expect("fallback exists");
        let EventPayload::PlayerRespawned { position, .. } = payload.clone() else {
            unreachable!();
        };
        assert_ne!(position, primary);
        assert!(state.proof_recovery_position_is_clear(position));
        assert!((position.x - primary.x).abs() <= f64::EPSILON);
        assert!((position.z - primary.z).abs() <= f64::EPSILON);
        assert!(position.y > primary.y);

        let event = state.new_test_human_event("player-local", "blocked-primary-recovery", payload);
        state.apply_event(&event).expect("fallback respawn applies");
        assert_eq!(state.player.position, position);
        assert_eq!(state.player.life_state, PlayerLifeState::Alive);
    }

    #[test]
    fn respawn_and_construction_clearance_use_the_complete_standing_capsule() {
        let radius = content::manifest().character.collision_radius_m;
        let half_height = character_capsule_half_height();

        let lower_cap_touch = Vec3::new(0.0, -0.5 - half_height - radius, 0.0);
        assert!(capsule_intersects_unit_cube(
            lower_cap_touch,
            Quat::IDENTITY,
            IVec3::new(0, 0, 0),
        ));
        assert!(!capsule_intersects_unit_cube(
            lower_cap_touch + Vec3::new(0.0, -0.001, 0.0),
            Quat::IDENTITY,
            IVec3::new(0, 0, 0),
        ));

        let mut planet_state = runtime().state().clone();
        planet_state.grids.clear();
        planet_state.voxels.occupied.clear();
        planet_state.voxels.ferrite_ore.clear();
        let planet_touch =
            planet_center() + Vec3::new(0.0, planet_surface_radius_m() + half_height + radius, 0.0);
        assert!(!planet_state.proof_recovery_position_is_clear(planet_touch));
        assert!(
            planet_state
                .proof_recovery_position_is_clear(planet_touch + Vec3::new(0.0, 0.001, 0.0))
        );

        let mut voxel_state = runtime().state().clone();
        voxel_state.grids.clear();
        let voxel = *voxel_state
            .voxels
            .occupied
            .iter()
            .max_by_key(|coordinate| coordinate.x)
            .expect("asteroid boundary voxel exists");
        let voxel_touch = Vec3::new(
            f64::from(voxel.x) + 0.5 + radius,
            f64::from(voxel.y),
            f64::from(voxel.z),
        );
        assert!(!voxel_state.proof_recovery_position_is_clear(voxel_touch));
        assert!(
            voxel_state.proof_recovery_position_is_clear(voxel_touch + Vec3::new(0.001, 0.0, 0.0))
        );

        let mut grid_state = runtime().state().clone();
        grid_state.voxels.occupied.clear();
        grid_state.voxels.ferrite_ore.clear();
        let grid = grid_state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists");
        set_test_grid_position(grid, Vec3::new(100.0, 100.0, 100.0));
        grid.orientation = Quat::IDENTITY;
        let grid_touch = grid.position + Vec3::new(2.5 + radius, 0.0, 0.0);
        assert!(!grid_state.proof_recovery_position_is_clear(grid_touch));
        assert!(
            grid_state.proof_recovery_position_is_clear(grid_touch + Vec3::new(0.001, 0.0, 0.0))
        );
    }

    #[test]
    fn lifecycle_replay_requires_canonical_actor_and_operation_envelopes() {
        let mut state = runtime().state().clone();
        state.player.suit_oxygen_milli = 5;
        let death = state
            .oxygen_incapacitation_payload()
            .expect("death prepares");
        let forged_death = state.new_event(
            Some("player-local"),
            "human",
            Some(OperationEventMetadata {
                operation_id: "forged-death".into(),
                operation_sequence: 1,
                intent_fingerprint: "0".repeat(64),
            }),
            death.clone(),
        );
        assert!(matches!(
            state.clone().apply_event(&forged_death),
            Err(IntentError::Rejected { ref code, .. })
                if code == "replay_lifecycle_envelope_invalid"
        ));

        let death_event = state.prepare_system_event(death);
        state
            .apply_event(&death_event)
            .expect("system death applies");
        let respawn = state.player_respawn_payload().expect("respawn prepares");
        let forged_respawn = state.prepare_system_event(respawn.clone());
        assert!(matches!(
            state.clone().apply_event(&forged_respawn),
            Err(IntentError::Rejected { ref code, .. })
                if code == "replay_lifecycle_envelope_invalid"
        ));

        let operation_id = "canonical-recovery";
        let respawn_event = state.new_test_human_event("player-local", operation_id, respawn);
        state
            .apply_event(&respawn_event)
            .expect("human respawn with operation applies");
        let duplicate = state.new_test_human_event_at(
            "player-local",
            1,
            operation_id,
            EventPayload::PlayerControlSet {
                movement_epoch: state.player.movement_epoch,
                input_sequence: 1,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
                expires_at_simulation_tick: state.simulation_tick
                    + content::manifest().character.control_lease_ticks,
            },
        );
        assert!(matches!(
            state.apply_event(&duplicate),
            Err(IntentError::Rejected { ref code, .. }) if code == "operation_conflict"
        ));
    }

    #[test]
    fn industry_and_grid_replay_require_authenticated_human_envelopes() {
        let state = WorldState::genesis(171);
        let payloads = [
            EventPayload::OreRefined {
                inventory_id: PLAYER_INVENTORY_ID.into(),
                batches: 1,
            },
            EventPayload::ComponentCrafted {
                inventory_id: PLAYER_INVENTORY_ID.into(),
                quantity: 1,
            },
            EventPayload::InventoryTransferred {
                source_inventory_id: PLAYER_INVENTORY_ID.into(),
                destination_inventory_id: "inventory-cargo-starter".into(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
            EventPayload::GridControlSet {
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                dampeners: true,
            },
            EventPayload::GridAnchorSet {
                grid_id: STARTER_GRID_ID.into(),
                anchored: true,
                reward_credited: true,
            },
        ];

        for payload in payloads {
            let expected_code = if matches!(
                payload,
                EventPayload::OreRefined { .. } | EventPayload::ComponentCrafted { .. }
            ) {
                "replay_physical_machine_required"
            } else {
                "replay_hand_tool_envelope_invalid"
            };
            let event = state.prepare_system_event(payload);
            let mut candidate = state.clone();
            let before = candidate.state_hash();
            let error = candidate
                .apply_event(&event)
                .expect_err("client work with a system envelope rejects");
            assert_eq!(error.code(), expected_code);
            assert_eq!(candidate.state_hash(), before);
        }
    }

    #[test]
    fn snapshot_and_journal_restarts_preserve_dead_and_post_respawn_states() {
        for respawned in [false, true] {
            for snapshot_target in [false, true] {
                let directory = tempdir().expect("tempdir");
                let expected_hash;
                {
                    let mut runtime =
                        Runtime::open(directory.path(), 93, 100).expect("runtime opens");
                    runtime.state.player.suit_oxygen_milli = 5;
                    runtime.persist_snapshot().expect("baseline persists");
                    for _ in 0..4 {
                        runtime.advance(250).expect("oxygen death commits");
                    }
                    if respawned {
                        runtime
                            .execute_next_for_fixture(&ClientMessage::RespawnPlayer {
                                operation_sequence: 0,
                                operation_id: "matrix-recovery".into(),
                            })
                            .expect("respawn commits");
                    }
                    expected_hash = runtime.state().state_hash();
                    if snapshot_target {
                        runtime
                            .persist_snapshot()
                            .expect("target snapshot persists");
                    }
                }

                let recovered =
                    Runtime::open(directory.path(), 93, 100).expect("target state reopens");
                assert_eq!(recovered.state().state_hash(), expected_hash);
                assert_eq!(
                    recovered.state().player.life_state == PlayerLifeState::Alive,
                    respawned
                );
                assert_eq!(recovered.state().death_drops.len(), 1);
                assert!(recovered.state().conservation().valid);
            }
        }
    }

    #[test]
    fn destination_volume_is_enforced_before_transfer() {
        let mut runtime = runtime();
        let cargo_id = runtime
            .state()
            .inventories
            .keys()
            .find(|id| id.contains("cargo"))
            .cloned()
            .expect("cargo inventory");
        runtime
            .state
            .inventories
            .get_mut(&cargo_id)
            .expect("cargo")
            .capacity_liters = 21;
        let result = runtime.execute_next_for_fixture(&ClientMessage::TransferInventory {
            operation_sequence: 0,
            operation_id: "overfill-cargo".into(),
            source_inventory_id: PLAYER_INVENTORY_ID.into(),
            destination_inventory_id: cargo_id,
            resource: ResourceKind::Component,
            quantity: 1,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "inventory_capacity_exceeded"
        ));
        assert_eq!(
            runtime.state().inventories[PLAYER_INVENTORY_ID]
                .contents
                .components,
            24
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn build_retry_does_not_consume_a_second_component() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let intent = ClientMessage::BuildBlock {
            operation_sequence: 0,
            operation_id: "idempotent-build".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::Structural,
            orientation: 2,
        };
        let first = runtime
            .execute_next_for_fixture(&intent)
            .expect("build accepted");
        let second = runtime
            .execute_next_for_fixture(&intent)
            .expect("retry accepted");
        assert_eq!(first, second);
        assert_eq!(
            runtime.state().inventories[PLAYER_INVENTORY_ID]
                .contents
                .components,
            23
        );
        assert_eq!(runtime.state().grids[STARTER_GRID_ID].blocks.len(), 26);
        let frame = runtime.state().grids[STARTER_GRID_ID]
            .block_at(IVec3::new(0, 1, 0))
            .expect("construction frame exists");
        assert_eq!(frame.health, 25);
        assert_eq!(frame.orientation, 2);
        assert!(!frame.construction_complete);
        assert_eq!(runtime.state().player.career.blocks_built, 0);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn welding_is_staged_idempotent_and_counts_completion_once() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        runtime
            .execute_next_for_fixture(&ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "place-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, 1, 0),
                kind: BlockKind::Structural,
                orientation: 1,
            })
            .expect("frame placement accepted");
        let block_id = runtime.state().grids[STARTER_GRID_ID]
            .block_at(IVec3::new(0, 1, 0))
            .expect("frame exists")
            .block_id
            .clone();
        let first_weld = ClientMessage::WeldBlock {
            operation_sequence: 0,
            operation_id: "weld-once".into(),
            grid_id: STARTER_GRID_ID.into(),
            block_id: block_id.clone(),
        };
        let first_receipt = runtime
            .execute_next_for_fixture(&first_weld)
            .expect("first weld accepted");
        let retry_receipt = runtime
            .execute_next_for_fixture(&first_weld)
            .expect("weld retry accepted");
        assert_eq!(first_receipt, retry_receipt);
        assert_eq!(
            runtime.state().grids[STARTER_GRID_ID].blocks[&block_id].health,
            50
        );
        runtime
            .execute_next_for_fixture(&ClientMessage::WeldBlock {
                operation_sequence: 0,
                operation_id: "weld-middle".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: block_id.clone(),
            })
            .expect("middle weld accepted");
        let final_weld = ClientMessage::WeldBlock {
            operation_sequence: 0,
            operation_id: "weld-final".into(),
            grid_id: STARTER_GRID_ID.into(),
            block_id: block_id.clone(),
        };
        let final_receipt = runtime
            .execute_next_for_fixture(&final_weld)
            .expect("final weld accepted");
        let final_retry = runtime
            .execute_next_for_fixture(&final_weld)
            .expect("final weld retry accepted");
        assert_eq!(final_receipt, final_retry);
        assert!(
            runtime.state().grids[STARTER_GRID_ID]
                .block_at(IVec3::new(0, 1, 0))
                .expect("completed block exists")
                .construction_complete
        );
        assert_eq!(runtime.state().player.career.blocks_built, 1);
        assert_eq!(runtime.state().player.experience, 25);
        let sequence = runtime.state().event_sequence;
        let completed = runtime.execute_next_for_fixture(&ClientMessage::WeldBlock {
            operation_sequence: 0,
            operation_id: "over-weld".into(),
            grid_id: STARTER_GRID_ID.into(),
            block_id,
        });
        assert!(matches!(
            completed,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "block_already_complete"
        ));
        assert_eq!(runtime.state().event_sequence, sequence);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn unfinished_cargo_inventory_is_sealed_until_final_weld() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        runtime
            .execute_next_for_fixture(&ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "place-cargo-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, 1, 0),
                kind: BlockKind::Cargo,
                orientation: 0,
            })
            .expect("cargo frame placement accepted");
        let cargo_block = runtime.state().grids[STARTER_GRID_ID]
            .block_at(IVec3::new(0, 1, 0))
            .expect("cargo frame exists")
            .clone();
        let cargo_inventory_id = cargo_block
            .inventory_id
            .clone()
            .expect("cargo identity exists");
        let sealed = runtime.execute_next_for_fixture(&ClientMessage::TransferInventory {
            operation_sequence: 0,
            operation_id: "transfer-into-sealed-cargo".into(),
            source_inventory_id: PLAYER_INVENTORY_ID.into(),
            destination_inventory_id: cargo_inventory_id.clone(),
            resource: ResourceKind::Component,
            quantity: 1,
        });
        assert!(matches!(
            sealed,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "inventory_block_incomplete"
        ));
        assert_eq!(
            runtime.state().inventories[&cargo_inventory_id]
                .contents
                .components,
            0
        );

        weld_to_completion(&mut runtime, IVec3::new(0, 1, 0), "complete-cargo");
        runtime
            .execute_next_for_fixture(&ClientMessage::TransferInventory {
                operation_sequence: 0,
                operation_id: "transfer-into-complete-cargo".into(),
                source_inventory_id: PLAYER_INVENTORY_ID.into(),
                destination_inventory_id: cargo_inventory_id.clone(),
                resource: ResourceKind::Component,
                quantity: 1,
            })
            .expect("completed cargo accepts inventory");
        runtime
            .execute_next_for_fixture(&ClientMessage::TransferInventory {
                operation_sequence: 0,
                operation_id: "transfer-out-of-complete-cargo".into(),
                source_inventory_id: cargo_inventory_id.clone(),
                destination_inventory_id: PLAYER_INVENTORY_ID.into(),
                resource: ResourceKind::Component,
                quantity: 1,
            })
            .expect("completed cargo releases inventory");
        assert_eq!(
            runtime.state().inventories[&cargo_inventory_id]
                .contents
                .components,
            0
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn repairing_completed_damage_preserves_armor_state_and_build_credit() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 43, 100).expect("runtime starts");
            move_player_near_grid(&mut runtime);
            let block_id = "block-deck-e".to_owned();
            let original = runtime.state().grids[STARTER_GRID_ID].blocks[&block_id].clone();
            assert!(original.construction_complete);
            assert_eq!(runtime.state().player.career.blocks_built, 0);

            aim_player_at_block(&mut runtime, STARTER_GRID_ID, &block_id);

            runtime
                .execute_next_for_fixture(&ClientMessage::DamageBlock {
                    operation_sequence: 0,
                    operation_id: "damage-completed-armor".into(),
                    grid_id: STARTER_GRID_ID.into(),
                    block_id: block_id.clone(),
                })
                .expect("completed armor can be damaged");
            let damaged = &runtime.state().grids[STARTER_GRID_ID].blocks[&block_id];
            assert_eq!(damaged.health, original.health - 35);
            assert!(damaged.construction_complete);
            let snapshot = runtime.state().snapshot();
            let damaged_snapshot = snapshot
                .grids
                .iter()
                .flat_map(|grid| &grid.blocks)
                .find(|block| block.block_id == block_id)
                .expect("damaged block appears in the snapshot");
            assert!(damaged_snapshot.construction_complete);
            assert_eq!(runtime.state().player.career.blocks_built, 0);
            assert_eq!(runtime.state().player.experience, 0);

            let mut weld_index = 0_u32;
            while runtime.state().grids[STARTER_GRID_ID].blocks[&block_id].health
                < original.max_health()
            {
                aim_player_at_block(&mut runtime, STARTER_GRID_ID, &block_id);
                runtime
                    .execute_next_for_fixture(&ClientMessage::WeldBlock {
                        operation_sequence: 0,
                        operation_id: format!("repair-completed-armor-{weld_index}"),
                        grid_id: STARTER_GRID_ID.into(),
                        block_id: block_id.clone(),
                    })
                    .expect("completed armor can be repaired");
                weld_index += 1;
            }
            let repaired = &runtime.state().grids[STARTER_GRID_ID].blocks[&block_id];
            assert_eq!(repaired.health, repaired.max_health());
            assert!(repaired.construction_complete);
            assert_eq!(runtime.state().player.career.blocks_built, 0);
            assert_eq!(runtime.state().player.experience, 0);
            assert!(runtime.state().conservation().valid);
            runtime.persist_snapshot().expect("snapshot persists");
            expected_hash = runtime.state().state_hash();
        }

        let recovered = Runtime::open(directory.path(), 43, 100).expect("runtime recovers");
        let repaired = &recovered.state().grids[STARTER_GRID_ID].blocks["block-deck-e"];
        assert_eq!(repaired.health, repaired.max_health());
        assert!(repaired.construction_complete);
        assert_eq!(recovered.state().player.career.blocks_built, 0);
        assert_eq!(recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn construction_rejects_remote_and_invalid_frames() {
        let mut runtime = runtime();
        let remote = runtime.execute_next_for_fixture(&ClientMessage::BuildBlock {
            operation_sequence: 0,
            operation_id: "remote-frame".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::Structural,
            orientation: 0,
        });
        assert!(matches!(
            remote,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "build_face_not_targeted"
        ));
        move_player_near_grid(&mut runtime);
        let candidate = IVec3::new(0, 1, 0);
        let candidate_position = runtime.state().grids[STARTER_GRID_ID].world_position(candidate);
        set_test_player_position(&mut runtime.state.player, candidate_position);
        let overlap = runtime.execute_next_for_fixture(&ClientMessage::BuildBlock {
            operation_sequence: 0,
            operation_id: "overlapping-frame".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: candidate,
            kind: BlockKind::Structural,
            orientation: 0,
        });
        assert!(matches!(
            overlap,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "block_intersects_player"
        ));
        let invalid = runtime.execute_next_for_fixture(&ClientMessage::BuildBlock {
            operation_sequence: 0,
            operation_id: "invalid-orientation".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::Structural,
            orientation: 4,
        });
        assert!(matches!(
            invalid,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "invalid_block_orientation"
        ));
        assert_eq!(runtime.state().grids[STARTER_GRID_ID].blocks.len(), 25);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn construction_replay_rejects_a_frame_around_the_player_before_mutation() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let mut state = runtime.state().clone();
        let coordinate = IVec3::new(0, 1, 0);
        let canonical = state
            .prepare_next_client_event_for_fixture(&ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "prepared-clear-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate,
                kind: BlockKind::Structural,
                orientation: 0,
            })
            .expect("clear construction event prepares");
        let occupied_position = state.grids[STARTER_GRID_ID].world_position(coordinate);
        set_test_player_position(&mut state.player, occupied_position);
        let before = state.state_hash();
        let error = state
            .apply_event(&canonical)
            .expect_err("replay cannot create a collider around the player");
        assert_eq!(error.code(), "replay_construction_intersects_player");
        assert_eq!(state.state_hash(), before);
    }

    #[test]
    fn unfinished_anchor_cannot_lock_the_grid() {
        let mut runtime = runtime();
        let anchor_coordinate = IVec3::new(-2, 1, -1);
        aim_player_for_build(&mut runtime, STARTER_GRID_ID, anchor_coordinate);
        runtime
            .execute_next_for_fixture(&ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "place-anchor-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: anchor_coordinate,
                kind: BlockKind::Anchor,
                orientation: 3,
            })
            .expect("anchor frame placement accepted");
        let unfinished = runtime.execute_next_for_fixture(&ClientMessage::ToggleGridAnchor {
            operation_sequence: 0,
            operation_id: "engage-unfinished-anchor".into(),
            grid_id: STARTER_GRID_ID.into(),
        });
        assert!(matches!(
            unfinished,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "anchor_not_touching_voxel"
        ));
        weld_to_completion(&mut runtime, anchor_coordinate, "weld-anchor");
        let experience_before_anchor = runtime.state().player.experience;
        let mut forged_anchor = runtime
            .state()
            .prepare_next_client_event_for_fixture(&ClientMessage::ToggleGridAnchor {
                operation_sequence: 0,
                operation_id: "forged-anchor-reward".into(),
                grid_id: STARTER_GRID_ID.into(),
            })
            .expect("canonical rewarded anchor event prepares");
        let EventPayload::GridAnchorSet {
            reward_credited, ..
        } = &mut forged_anchor.payload
        else {
            unreachable!();
        };
        *reward_credited = false;
        forged_anchor.event_hash = forged_anchor.calculate_hash();
        let mut forged_candidate = runtime.state().clone();
        let forged_before = forged_candidate.state_hash();
        let forged_error = forged_candidate
            .apply_event(&forged_anchor)
            .expect_err("forged one-time anchor reward decision rejects");
        assert_eq!(forged_error.code(), "replay_grid_anchor_invalid");
        assert_eq!(forged_candidate.state_hash(), forged_before);
        runtime
            .execute_next_for_fixture(&ClientMessage::ToggleGridAnchor {
                operation_sequence: 0,
                operation_id: "engage-complete-anchor".into(),
                grid_id: STARTER_GRID_ID.into(),
            })
            .expect("complete anchor engages");
        assert!(runtime.state().grids[STARTER_GRID_ID].anchored);
        assert!(!runtime.state().grids[STARTER_GRID_ID].anchor_reward_eligible);
        assert_eq!(
            runtime.state().player.experience,
            experience_before_anchor + 40
        );
        runtime
            .execute_next_for_fixture(&ClientMessage::ToggleGridAnchor {
                operation_sequence: 0,
                operation_id: "release-complete-anchor".into(),
                grid_id: STARTER_GRID_ID.into(),
            })
            .expect("complete anchor releases");
        runtime
            .execute_next_for_fixture(&ClientMessage::ToggleGridAnchor {
                operation_sequence: 0,
                operation_id: "reengage-complete-anchor".into(),
                grid_id: STARTER_GRID_ID.into(),
            })
            .expect("complete anchor reengages without another reward");
        assert_eq!(
            runtime.state().player.experience,
            experience_before_anchor + 40
        );
        assert_eq!(runtime.state().player.career.anchors_engaged, 2);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn unfinished_power_block_does_not_join_the_network() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let baseline = runtime.state().grids[STARTER_GRID_ID].power().produced;
        runtime
            .execute_next_for_fixture(&ClientMessage::BuildBlock {
                operation_sequence: 0,
                operation_id: "place-power-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, 1, 0),
                kind: BlockKind::PowerSource,
                orientation: 1,
            })
            .expect("power frame placement accepted");
        assert!(
            (runtime.state().grids[STARTER_GRID_ID].power().produced - baseline).abs()
                < f64::EPSILON
        );
        weld_to_completion(&mut runtime, IVec3::new(0, 1, 0), "weld-power");
        let expected = baseline + content::block(BlockKind::PowerSource).power_production;
        assert!(
            (runtime.state().grids[STARTER_GRID_ID].power().produced - expected).abs()
                < f64::EPSILON
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn lost_writer_authority_halts_without_mutating_world() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 73, 100).expect("runtime opens");
        let lock_path = directory.path().join("cell-lifecycle.json");
        let mut lease: serde_json::Value =
            serde_json::from_slice(&fs::read(&lock_path).expect("lease reads"))
                .expect("lease parses");
        lease["fencing_token"] = serde_json::json!(9_999);
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&lease).expect("lease serializes"),
        )
        .expect("replacement lease writes");

        let result = runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
            operation_sequence: 0,
            operation_id: "stale-writer-control".into(),
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(1.0, 0.0, 0.0),
            angular_input: Vec3::ZERO,
            boost: false,
            jump: false,
            dampeners: true,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Persistence(
                PersistenceError::FencingTokenChanged { .. }
            ))
        ));
        assert_eq!(runtime.state().event_sequence, 0);
        assert_eq!(runtime.state().player.position, Vec3::new(12.0, 4.5, 10.0));
        assert!(matches!(
            runtime.execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "halted-control".into(),
                movement_epoch: 1,
                input_sequence: 2,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            }),
            Err(RuntimeError::Halted)
        ));
        assert!(
            fs::read_to_string(directory.path().join("events.ndjson"))
                .expect("journal reads")
                .is_empty()
        );
    }

    #[test]
    fn authoritative_player_ccd_contacts_a_rotated_grid() {
        let mut runtime = runtime();
        let half_sqrt = std::f32::consts::FRAC_1_SQRT_2;
        let grid = runtime
            .state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists");
        grid.orientation = Quat::new(0.0, half_sqrt, 0.0, half_sqrt);
        grid.anchored = true;
        let drill_position =
            runtime.state.grids[STARTER_GRID_ID].world_position(IVec3::new(2, 0, 0));
        let start = drill_position + Vec3::new(0.0, 1.2, 0.0);
        set_test_player_position(&mut runtime.state.player, start);
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime.rebuild_physics_for_test();

        let mut contacted = false;
        for _ in 0..30 {
            runtime
                .advance(17)
                .expect("authoritative player step commits");
            contacted |= runtime.state().player.surface_contact;
            if contacted {
                break;
            }
        }

        assert!(runtime.state().player.position.y > drill_position.y + 0.6);
        assert!(runtime.state().player.linear_velocity.y > -24.0);
        assert!(contacted);
        assert!(runtime.state().active_contact_pairs.iter().any(|pair| {
            contact_key_involves_player(pair)
                && (pair.body_a == STARTER_GRID_ID || pair.body_b == STARTER_GRID_ID)
        }));
    }

    #[test]
    fn authoritative_player_lands_on_planet_with_stable_canonical_contact() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        let start = planet_center()
            + Vec3::new(
                0.0,
                planet_surface_radius_m() + standing_half_height + 2.0,
                0.0,
            );
        set_test_player_position(&mut runtime.state.player, start);
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime.rebuild_physics_for_test();

        let mut contacted = false;
        for _ in 0..60 {
            runtime.advance(17).expect("planet landing commits");
            contacted |= runtime.state().player.surface_contact;
        }
        assert!(contacted);
        assert!(runtime.state().active_contact_pairs.iter().any(|pair| {
            contact_key_involves_player(pair)
                && (pair.body_a == PLANET_BODY_ID || pair.body_b == PLANET_BODY_ID)
        }));

        let mut minimum_gap = f64::MAX;
        let mut maximum_gap = f64::MIN;
        for _ in 0..30 {
            runtime
                .advance(17)
                .expect("landed player remains simulated");
            let distance = (runtime.state().player.position - planet_center()).magnitude();
            let gap = distance - planet_surface_radius_m() - standing_half_height;
            minimum_gap = minimum_gap.min(gap);
            maximum_gap = maximum_gap.max(gap);
        }
        assert!(minimum_gap > -REPLAY_CONTACT_SLOP_M);
        assert!(maximum_gap - minimum_gap < 0.05);
        assert!(
            runtime.state().player.linear_velocity.magnitude() < 0.25,
            "settled velocity={:?}, position={:?}, locomotion={:?}",
            runtime.state().player.linear_velocity,
            runtime.state().player.position,
            runtime.state().player.locomotion.kind,
        );
        assert!(runtime.state().player.surface_contact);
        assert_eq!(
            runtime.state().player.locomotion.kind,
            LocomotionKind::Grounded
        );
        let support = runtime
            .state()
            .player
            .locomotion
            .support
            .as_ref()
            .expect("settled capsule retains authoritative support");
        assert_eq!(support.body_id, PLANET_BODY_ID);
        assert_eq!(support.collider_id, PLANET_COLLIDER_ID);
    }

    #[test]
    fn grounded_slope_hysteresis_enters_at_fifty_and_exits_after_fifty_two_degrees() {
        let slope_normal = |degrees: f64| {
            let radians = degrees.to_radians();
            Vec3::new(radians.sin(), radians.cos(), 0.0)
        };
        let gravity_up = Vec3::new(0.0, 1.0, 0.0);

        assert!(gravity_support_is_walkable(
            9.81,
            slope_normal(50.0),
            gravity_up,
            false
        ));
        assert!(!gravity_support_is_walkable(
            9.81,
            slope_normal(50.1),
            gravity_up,
            false
        ));
        assert!(gravity_support_is_walkable(
            9.81,
            slope_normal(51.9),
            gravity_up,
            true
        ));
        assert!(!gravity_support_is_walkable(
            9.81,
            slope_normal(52.1),
            gravity_up,
            true
        ));
        assert!(!gravity_support_is_walkable(
            0.0, gravity_up, gravity_up, true
        ));
    }

    #[test]
    fn magnetic_support_accepts_only_completed_grid_block_colliders() {
        let mut state = runtime().state().clone();
        let completed_block_id = state.grids[STARTER_GRID_ID]
            .blocks
            .values()
            .find(|block| block.is_complete())
            .expect("starter grid has a completed block")
            .block_id
            .clone();
        assert!(magnetic_support_is_eligible(
            &state,
            STARTER_GRID_ID,
            &completed_block_id,
        ));
        assert!(!magnetic_support_is_eligible(
            &state,
            PLANET_BODY_ID,
            PLANET_COLLIDER_ID,
        ));
        assert!(!magnetic_support_is_eligible(
            &state,
            "voxel-chunk-0-0-0",
            "voxel-0-0-0",
        ));

        let block = state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists")
            .blocks
            .get_mut(&completed_block_id)
            .expect("completed block exists");
        block.construction_complete = false;
        block.health = block.max_health().saturating_sub(1);
        assert!(!magnetic_support_is_eligible(
            &state,
            STARTER_GRID_ID,
            &completed_block_id,
        ));
    }

    #[test]
    fn grounded_step_solver_climbs_a_clear_thirty_centimeter_ledge() {
        let character = &content::manifest().character;
        let mut scene = Scene::new(SceneConfig {
            max_body_translation_m: 0.6,
            ..SceneConfig::default()
        })
        .expect("step scene initializes");
        let floor = BodySpec::static_body(
            "floor",
            PhysicsPose::new(PhysicsVec3::new(0.0, -0.1, 0.0), PhysicsQuat::IDENTITY),
            vec![BoxColliderSpec {
                collider_id: "floor-panel".into(),
                half_extents: PhysicsVec3::new(5.0, 0.1, 5.0),
                ..BoxColliderSpec::unit_cube("ignored")
            }],
        );
        let step = BodySpec::static_body(
            "step",
            PhysicsPose::new(PhysicsVec3::new(0.88, 0.15, 0.0), PhysicsQuat::IDENTITY),
            vec![BoxColliderSpec {
                collider_id: "step-panel".into(),
                half_extents: PhysicsVec3::new(0.5, 0.15, 2.0),
                ..BoxColliderSpec::unit_cube("ignored")
            }],
        );
        let mut player = BodySpec::dynamic(
            PLAYER_BODY_ID,
            PhysicsPose::new(PhysicsVec3::new(0.0, 0.9, 0.0), PhysicsQuat::IDENTITY),
            Vec::new(),
        );
        player.capsule_colliders.push(CapsuleColliderSpec::new(
            PLAYER_COLLIDER_ID,
            character.collision_radius_m as f32,
            character_capsule_half_height() as f32,
        ));
        scene
            .rebuild(&[floor, step, player])
            .expect("step scene builds");

        let translation = grounded_step_translation(
            &scene,
            PLAYER_BODY_ID,
            PhysicsPose::new(PhysicsVec3::new(0.0, 0.9, 0.0), PhysicsQuat::IDENTITY),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.125, 0.0, 0.0),
        )
        .expect("step queries succeed")
        .expect("clear ledge produces a bounded step");

        assert!((translation.x - 0.125).abs() < 1.0e-9);
        assert!(
            (0.15..=character.step_height_m).contains(&translation.y),
            "translation={translation:?}"
        );
        assert!(translation.magnitude() < 0.5);

        scene
            .replace_body(
                "step",
                Some(BodySpec::static_body(
                    "step",
                    PhysicsPose::new(PhysicsVec3::new(0.88, 0.35, 0.0), PhysicsQuat::IDENTITY),
                    vec![BoxColliderSpec {
                        collider_id: "step-panel".into(),
                        half_extents: PhysicsVec3::new(0.5, 0.35, 2.0),
                        ..BoxColliderSpec::unit_cube("ignored")
                    }],
                )),
            )
            .expect("tall obstacle replaces");
        assert!(
            grounded_step_translation(
                &scene,
                PLAYER_BODY_ID,
                PhysicsPose::new(PhysicsVec3::new(0.0, 0.9, 0.0), PhysicsQuat::IDENTITY),
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.125, 0.0, 0.0),
            )
            .expect("tall obstacle queries succeed")
            .is_none(),
            "a seventy centimeter obstacle must exceed the configured step height"
        );
    }

    #[test]
    fn grounded_snap_closes_a_small_support_gap_without_changing_velocity() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        let start = planet_center()
            + Vec3::new(
                0.0,
                planet_surface_radius_m() + standing_half_height + 0.15,
                0.0,
            );
        set_test_player_position(&mut runtime.state.player, start);
        runtime.state.player.linear_velocity = Vec3::new(1.0, 0.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Grounded,
            false,
            runtime.state.simulation_tick,
        );
        runtime.state.player.locomotion.support = Some(LocomotionSupportSnapshot {
            body_id: PLANET_BODY_ID.into(),
            collider_id: PLANET_COLLIDER_ID.into(),
            local_anchor: Vec3::new(0.0, planet_surface_radius_m(), 0.0),
            local_normal: Vec3::new(0.0, 1.0, 0.0),
        });
        runtime.rebuild_physics_for_test();
        let mut body_states = runtime
            .physics_mut()
            .body_states()
            .expect("body states extract");
        let before = body_states
            .iter()
            .find(|body| body.body_id == PLAYER_BODY_ID)
            .expect("player body exists")
            .clone();

        {
            let Runtime { state, physics, .. } = &mut runtime;
            adjust_grounded_capsule_for_substep(
                &*state,
                physics
                    .as_mut()
                    .expect("active test runtime has an initialized physics scene"),
                &state.player,
                &mut body_states,
                state.simulation_tick,
            )
            .expect("ground snap applies");
        }
        let after = body_states
            .iter()
            .find(|body| body.body_id == PLAYER_BODY_ID)
            .expect("player body remains");

        let downward_translation = before.pose.position.y - after.pose.position.y;
        assert!((0.12..=0.16).contains(&downward_translation));
        assert_eq!(after.linear_velocity, before.linear_velocity);
    }

    #[test]
    fn grounded_capsule_aligns_its_physical_up_to_radial_planet_gravity() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        let start = planet_center()
            + Vec3::new(
                planet_surface_radius_m() + standing_half_height + 0.02,
                0.0,
                0.0,
            );
        set_test_player_position(&mut runtime.state.player, start);
        let initial_angle = -65.0_f64.to_radians();
        runtime.state.player.orientation = Quat::new(
            0.0,
            0.0,
            (initial_angle * 0.5).sin() as f32,
            (initial_angle * 0.5).cos() as f32,
        );
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime.state.player.angular_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            false,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();

        let desired_up = Vec3::new(1.0, 0.0, 0.0);
        let initial_up = runtime
            .state
            .player
            .orientation
            .rotate(Vec3::new(0.0, 1.0, 0.0));
        let initial_alignment = dot(initial_up, desired_up);
        for _ in 0..8 {
            runtime.advance(250).expect("upright controller advances");
        }
        let final_up = runtime
            .state
            .player
            .orientation
            .rotate(Vec3::new(0.0, 1.0, 0.0));
        let final_alignment = dot(final_up, desired_up);

        assert_eq!(
            runtime.state.player.locomotion.kind,
            LocomotionKind::Grounded
        );
        assert!(
            final_alignment > initial_alignment + 0.07,
            "radial alignment did not improve enough: initial={initial_alignment}, final={final_alignment}, position={:?}, locomotion_up={:?}, angular_velocity={:?}",
            runtime.state.player.position,
            runtime.state.player.locomotion.up,
            runtime.state.player.angular_velocity,
        );
        assert!(final_alignment > 0.98, "final alignment={final_alignment}");
    }

    #[test]
    fn grounded_capsule_classifies_upright_on_all_six_planet_axes() {
        let half_sqrt = std::f32::consts::FRAC_1_SQRT_2;
        let fixtures = [
            (Vec3::new(0.0, 1.0, 0.0), Quat::IDENTITY),
            (Vec3::new(0.0, -1.0, 0.0), Quat::new(1.0, 0.0, 0.0, 0.0)),
            (
                Vec3::new(1.0, 0.0, 0.0),
                Quat::new(0.0, 0.0, -half_sqrt, half_sqrt),
            ),
            (
                Vec3::new(-1.0, 0.0, 0.0),
                Quat::new(0.0, 0.0, half_sqrt, half_sqrt),
            ),
            (
                Vec3::new(0.0, 0.0, 1.0),
                Quat::new(half_sqrt, 0.0, 0.0, half_sqrt),
            ),
            (
                Vec3::new(0.0, 0.0, -1.0),
                Quat::new(-half_sqrt, 0.0, 0.0, half_sqrt),
            ),
        ];
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;

        for (axis, orientation) in fixtures {
            let mut runtime = runtime();
            let start =
                planet_center() + axis * (planet_surface_radius_m() + standing_half_height + 0.02);
            set_test_player_position(&mut runtime.state.player, start);
            runtime.state.player.orientation = orientation;
            runtime.state.player.linear_velocity = Vec3::ZERO;
            runtime.state.player.angular_velocity = Vec3::ZERO;
            runtime.state.player.jetpack_enabled = false;
            runtime.state.player.locomotion = reset_locomotion(
                runtime.state.player.position,
                LocomotionKind::Airborne,
                false,
                runtime.state.simulation_tick,
            );
            runtime.rebuild_physics_for_test();
            runtime.advance(17).expect("planet-axis support classifies");

            assert_eq!(
                runtime.state.player.locomotion.kind,
                LocomotionKind::Grounded,
                "axis={axis:?}"
            );
            let physical_up = runtime
                .state
                .player
                .orientation
                .rotate(Vec3::new(0.0, 1.0, 0.0));
            assert!(
                dot(physical_up, axis) > 0.999,
                "axis={axis:?}, up={physical_up:?}"
            );
        }
    }

    #[test]
    fn grounded_walk_crosses_the_planet_pole_neighborhood_without_an_orientation_flip() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        let start = planet_center()
            + Vec3::new(
                0.0,
                planet_surface_radius_m() + standing_half_height + 0.02,
                0.0,
            );
        set_test_player_position(&mut runtime.state.player, start);
        runtime.state.player.orientation = Quat::IDENTITY;
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime.state.player.angular_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            false,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();
        runtime.advance(17).expect("pole support classifies");
        let initial_position = runtime.state.player.position;
        let mut previous_orientation = runtime.state.player.orientation;

        for sequence in 1..=12 {
            runtime
                .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                    operation_sequence: 0,
                    operation_id: format!("pole-walk-{sequence}"),
                    movement_epoch: runtime.state.player.movement_epoch,
                    input_sequence: sequence,
                    linear_input: Vec3::new(1.0, 0.0, 0.0),
                    angular_input: Vec3::ZERO,
                    boost: false,
                    jump: false,
                    dampeners: true,
                })
                .expect("pole walk input refreshes");
            runtime.advance(250).expect("pole walk advances");
            let orientation = runtime.state.player.orientation;
            let orientation_dot = f64::from(previous_orientation.x) * f64::from(orientation.x)
                + f64::from(previous_orientation.y) * f64::from(orientation.y)
                + f64::from(previous_orientation.z) * f64::from(orientation.z)
                + f64::from(previous_orientation.w) * f64::from(orientation.w);
            let radial = radial_up(runtime.state.player.position);
            let physical_up = orientation.rotate(Vec3::new(0.0, 1.0, 0.0));

            assert_eq!(
                runtime.state.player.locomotion.kind,
                LocomotionKind::Grounded
            );
            assert!(
                orientation_dot.abs() > 0.98,
                "orientation discontinuity at sequence {sequence}: dot={orientation_dot}"
            );
            assert!(
                dot(physical_up, radial) > 0.98,
                "radial upright drift at sequence {sequence}"
            );
            previous_orientation = orientation;
        }
        assert!(runtime.state.player.position.x > initial_position.x + 8.0);
    }

    #[test]
    fn grounded_capsule_walks_sprints_and_brakes_in_the_surface_tangent_frame() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        let start = planet_center()
            + Vec3::new(
                0.0,
                planet_surface_radius_m() + standing_half_height + 0.02,
                0.0,
            );
        set_test_player_position(&mut runtime.state.player, start);
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime.state.player.angular_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            false,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();
        runtime.advance(17).expect("support is classified");
        assert_eq!(
            runtime.state().player.locomotion.kind,
            LocomotionKind::Grounded
        );

        let initial_x = runtime.state().player.position.x;
        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "ground-walk".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            })
            .expect("walk input commits");
        runtime.advance(200).expect("walk advances");
        let walk_speed = runtime.state().player.linear_velocity.x.abs();
        assert!(runtime.state().player.position.x > initial_x + 0.05);
        assert!(walk_speed > 1.0);
        assert!(walk_speed <= content::manifest().character.walk_speed_m_s + 0.1);

        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "ground-sprint".into(),
                movement_epoch: 1,
                input_sequence: 2,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                boost: true,
                jump: false,
                dampeners: true,
            })
            .expect("sprint input commits");
        runtime.advance(250).expect("sprint advances");
        let sprint_speed = runtime.state().player.linear_velocity.x.abs();
        assert!(sprint_speed > walk_speed);
        assert!(sprint_speed <= content::manifest().character.sprint_speed_m_s + 0.1);

        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "ground-brake".into(),
                movement_epoch: 1,
                input_sequence: 3,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                jump: false,
                dampeners: true,
            })
            .expect("brake input commits");
        runtime.advance(250).expect("ground braking advances");
        assert!(runtime.state().player.linear_velocity.x.abs() < sprint_speed * 0.35);
        assert_eq!(
            runtime.state().player.locomotion.kind,
            LocomotionKind::Grounded
        );
    }

    #[test]
    fn grounded_jump_is_edge_triggered_and_inherits_support_motion() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        let start = planet_center()
            + Vec3::new(
                0.0,
                planet_surface_radius_m() + standing_half_height + 0.02,
                0.0,
            );
        set_test_player_position(&mut runtime.state.player, start);
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            false,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();
        runtime.advance(17).expect("support is classified");

        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "ground-jump-press".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                jump: true,
                dampeners: true,
            })
            .expect("jump press commits");
        runtime.advance(17).expect("jump launches");
        assert_eq!(
            runtime.state().player.locomotion.kind,
            LocomotionKind::Airborne
        );
        assert!(runtime.state().player.locomotion.support.is_none());
        assert!(runtime.state().player.linear_velocity.y > 4.0);
        let first_launch_speed = runtime.state().player.linear_velocity.y;

        runtime
            .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                operation_sequence: 0,
                operation_id: "ground-jump-held".into(),
                movement_epoch: 1,
                input_sequence: 2,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                jump: true,
                dampeners: true,
            })
            .expect("held jump frame commits");
        runtime.advance(17).expect("held jump advances once");
        assert!(runtime.state().player.linear_velocity.y < first_launch_speed);
    }

    #[test]
    fn magnetic_capsule_tracks_a_powered_moving_grid_in_support_space() {
        let mut runtime = runtime();
        {
            let grid = runtime
                .state
                .grids
                .get_mut(STARTER_GRID_ID)
                .expect("starter grid exists");
            grid.linear_velocity = Vec3::new(2.0, 0.0, 0.0);
            grid.angular_velocity = Vec3::ZERO;
            grid.dampeners = false;
        }
        set_test_player_position(&mut runtime.state.player, Vec3::new(11.0, 1.42, 0.0));
        runtime.state.player.orientation = Quat::IDENTITY;
        runtime.state.player.linear_velocity = Vec3::new(2.0, 0.0, 0.0);
        runtime.state.player.angular_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            true,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();

        runtime.advance(17).expect("magnetic support is classified");
        assert_eq!(
            runtime.state().player.locomotion.kind,
            LocomotionKind::Magnetic
        );
        assert_eq!(
            runtime
                .state()
                .player
                .locomotion
                .support
                .as_ref()
                .expect("magnetic support exists")
                .body_id,
            STARTER_GRID_ID
        );
        runtime.advance(200).expect("moving support advances");
        let grid_speed = runtime.state().grids[STARTER_GRID_ID].linear_velocity.x;
        let relative_speed = runtime.state().player.linear_velocity.x - grid_speed;
        assert!(
            relative_speed.abs() < 0.35,
            "relative speed {relative_speed}"
        );
        assert_eq!(
            runtime.state().player.locomotion.kind,
            LocomotionKind::Magnetic
        );
    }

    #[test]
    fn magnetic_capsule_retains_a_local_anchor_on_a_rotating_grid() {
        let mut runtime = runtime();
        {
            let grid = runtime
                .state
                .grids
                .get_mut(STARTER_GRID_ID)
                .expect("starter grid exists");
            grid.linear_velocity = Vec3::ZERO;
            grid.angular_velocity = Vec3::new(0.0, 0.4, 0.0);
            grid.dampeners = false;
        }
        set_test_player_position(&mut runtime.state.player, Vec3::new(11.0, 1.42, 1.0));
        runtime.state.player.orientation = Quat::IDENTITY;
        runtime.state.player.linear_velocity = Vec3::new(0.4, 0.0, 0.0);
        runtime.state.player.angular_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            true,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();

        runtime.advance(17).expect("rotating support classifies");
        assert_eq!(
            runtime.state.player.locomotion.kind,
            LocomotionKind::Magnetic
        );
        let initial_anchor = runtime
            .state
            .player
            .locomotion
            .support
            .as_ref()
            .expect("initial rotating support exists")
            .local_anchor;
        for _ in 0..3 {
            runtime.advance(250).expect("rotating support advances");
        }
        let final_support = runtime
            .state
            .player
            .locomotion
            .support
            .as_ref()
            .expect("rotating support remains bound");
        let anchor_drift = final_support
            .local_anchor
            .squared_distance(initial_anchor)
            .sqrt();

        assert_eq!(
            runtime.state.player.locomotion.kind,
            LocomotionKind::Magnetic
        );
        assert_eq!(final_support.body_id, STARTER_GRID_ID);
        assert!(anchor_drift < 0.2, "local anchor drift={anchor_drift}");
    }

    #[test]
    fn destroying_the_bound_magnetic_block_detaches_without_teleporting() {
        let mut runtime = runtime();
        set_test_player_position(&mut runtime.state.player, Vec3::new(11.0, 1.42, 0.0));
        runtime.state.player.orientation = Quat::IDENTITY;
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime.state.player.angular_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            true,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();
        runtime.advance(17).expect("magnetic support classifies");
        let support = runtime
            .state
            .player
            .locomotion
            .support
            .as_ref()
            .expect("magnetic support exists")
            .clone();
        assert_eq!(support.body_id, STARTER_GRID_ID);
        assert_eq!(support.collider_id, "block-core");
        let position_before_damage = runtime.state.player.position;

        let hit_count = runtime.state.grids[STARTER_GRID_ID].blocks[&support.collider_id]
            .health
            .div_ceil(35);
        let supported_player = runtime.state.player.primary().clone();
        for index in 0..hit_count {
            aim_player_at_block_preserving_locomotion(
                &mut runtime,
                STARTER_GRID_ID,
                &support.collider_id,
            );
            runtime
                .execute_next_for_fixture(&ClientMessage::DamageBlock {
                    operation_sequence: 0,
                    operation_id: format!("destroy-magnetic-support-{index}"),
                    grid_id: STARTER_GRID_ID.into(),
                    block_id: support.collider_id.clone(),
                })
                .expect("support damage commits");
            restore_player_pose_after_tool_fixture(&mut runtime, &supported_player);
        }
        assert!(!runtime.state.grids.values().any(|grid| {
            grid.blocks
                .values()
                .any(|block| block.block_id == support.collider_id)
        }));
        runtime.advance(17).expect("destroyed support detaches");

        assert_eq!(
            runtime.state.player.locomotion.kind,
            LocomotionKind::Airborne
        );
        assert!(runtime.state.player.locomotion.support.is_none());
        assert!(
            runtime
                .state
                .player
                .position
                .squared_distance(position_before_damage)
                .sqrt()
                < 0.1,
            "support destruction must not teleport the player"
        );
        assert!(runtime.state.conservation().valid);
    }

    #[test]
    fn magnetic_support_rebinds_by_collider_identity_after_a_grid_split() {
        let mut runtime = runtime();
        replace_with_physics_fixture(
            &mut runtime,
            [test_grid(
                "split-grid",
                Vec3::ZERO,
                Vec3::ZERO,
                [
                    Block::new("split-core", IVec3::new(0, 0, 0), BlockKind::ControlCore),
                    Block::new("split-bridge", IVec3::new(1, 0, 0), BlockKind::Structural),
                    Block::new("split-support", IVec3::new(2, 0, 0), BlockKind::Structural),
                ],
            )],
            VoxelField {
                occupied: BTreeSet::new(),
                ferrite_ore: BTreeSet::new(),
            },
        );
        set_test_player_position(&mut runtime.state.player, Vec3::new(2.0, 1.42, 0.0));
        runtime.state.player.orientation = Quat::IDENTITY;
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime.state.player.angular_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            true,
            runtime.state.simulation_tick,
        );
        runtime.rebuild_physics_for_test();
        runtime.advance(17).expect("split support classifies");
        let initial_support = runtime
            .state
            .player
            .locomotion
            .support
            .as_ref()
            .expect("initial split support exists")
            .clone();
        assert_eq!(initial_support.body_id, "split-grid");
        assert_eq!(initial_support.collider_id, "split-support");
        let position_before_split = runtime.state.player.position;

        let bridge_health = runtime.state.grids["split-grid"].blocks["split-bridge"].health;
        let supported_player = runtime.state.player.primary().clone();
        for index in 0..bridge_health.div_ceil(35) {
            aim_player_at_block_preserving_locomotion(&mut runtime, "split-grid", "split-bridge");
            runtime
                .execute_next_for_fixture(&ClientMessage::DamageBlock {
                    operation_sequence: 0,
                    operation_id: format!("split-support-bridge-{index}"),
                    grid_id: "split-grid".into(),
                    block_id: "split-bridge".into(),
                })
                .expect("bridge damage commits");
            restore_player_pose_after_tool_fixture(&mut runtime, &supported_player);
        }
        let split_grid_id = runtime
            .state
            .grids
            .iter()
            .find_map(|(grid_id, grid)| {
                grid.blocks
                    .contains_key("split-support")
                    .then_some(grid_id.clone())
            })
            .expect("support collider survives on one split body");
        assert_ne!(split_grid_id, initial_support.body_id);
        runtime.advance(17).expect("split support reclassifies");
        let rebound = runtime
            .state
            .player
            .locomotion
            .support
            .as_ref()
            .expect("split support rebinds");

        assert_eq!(
            runtime.state.player.locomotion.kind,
            LocomotionKind::Magnetic
        );
        assert_eq!(rebound.body_id, split_grid_id);
        assert_eq!(rebound.collider_id, initial_support.collider_id);
        assert!(
            runtime
                .state
                .player
                .position
                .squared_distance(position_before_split)
                .sqrt()
                < 0.1,
            "grid split must not teleport the bound player"
        );
        assert!(runtime.state.conservation().valid);
    }

    #[test]
    fn below_ccd_threshold_planet_step_stays_inside_the_replay_penetration_budget() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        let start = planet_center()
            + Vec3::new(
                0.0,
                planet_surface_radius_m() + standing_half_height + 0.10,
                0.0,
            );
        set_test_player_position(&mut runtime.state.player, start);
        runtime.state.player.linear_velocity = Vec3::new(0.0, -12.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime.rebuild_physics_for_test();

        runtime
            .advance(17)
            .expect("near-threshold planet step remains replay-valid");
        let distance = (runtime.state().player.position - planet_center()).magnitude();
        assert!(
            distance
                >= planet_surface_radius_m() + standing_half_height
                    - PLAYER_PLANET_PENETRATION_LIMIT_M
        );
    }

    #[test]
    fn authoritative_player_ccd_contacts_voxel_while_nearby_clear_motion_passes() {
        let mut runtime = runtime();
        let surface = *runtime
            .state()
            .voxels
            .occupied
            .iter()
            .max_by_key(|coordinate| coordinate.y)
            .expect("asteroid surface voxel exists");
        let radius = content::manifest().character.collision_radius_m;
        let collision_start = Vec3::new(
            f64::from(surface.x),
            f64::from(surface.y) + 0.5 + radius + 2.0,
            f64::from(surface.z),
        );
        set_test_player_position(&mut runtime.state.player, collision_start);
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime.rebuild_physics_for_test();

        let mut contacted = false;
        for _ in 0..30 {
            runtime.advance(17).expect("voxel collision commits");
            contacted |= runtime.state().player.surface_contact;
            if contacted {
                break;
            }
        }
        assert!(contacted);
        assert!(runtime.state().active_contact_pairs.iter().any(|pair| {
            contact_key_involves_player(pair)
                && (pair.collider_a == voxel_collision_collider_id(surface)
                    || pair.collider_b == voxel_collision_collider_id(surface))
        }));
        assert!(
            runtime.state().player.position.y
                >= f64::from(surface.y) + 0.5 + radius - REPLAY_CONTACT_SLOP_M
        );

        let clear_start = Vec3::new(
            f64::from(surface.x),
            f64::from(surface.y) + 0.5 + radius + 0.25,
            f64::from(surface.z),
        );
        set_test_player_position(&mut runtime.state.player, clear_start);
        runtime.state.player.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
        runtime.state.player.surface_contact = false;
        runtime.state.active_contact_pairs.clear();
        runtime.rebuild_physics_for_test();
        runtime.advance(100).expect("nearby clear motion commits");
        assert!(runtime.state().player.position.x > clear_start.x + 0.2);
    }

    #[test]
    fn authoritative_player_ccd_contacts_axis_aligned_grid_while_clear_motion_passes() {
        let mut runtime = runtime();
        let radius = content::manifest().character.collision_radius_m;
        let grid = runtime
            .state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists");
        let block_position = grid.world_position(IVec3::ZERO);
        set_test_player_position(
            &mut runtime.state.player,
            block_position + Vec3::new(0.0, 0.5 + radius + 2.0, 0.0),
        );
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime.rebuild_physics_for_test();

        let mut contacted = false;
        for _ in 0..30 {
            runtime.advance(17).expect("grid collision commits");
            contacted |= runtime.state().player.surface_contact;
            if contacted {
                break;
            }
        }
        assert!(contacted);
        assert!(runtime.state().active_contact_pairs.iter().any(|pair| {
            contact_key_involves_player(pair)
                && (pair.body_a == STARTER_GRID_ID || pair.body_b == STARTER_GRID_ID)
        }));

        let clear_start = block_position + Vec3::new(0.0, 0.5 + radius + 0.25, 3.0);
        set_test_player_position(&mut runtime.state.player, clear_start);
        runtime.state.player.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
        runtime.state.player.surface_contact = false;
        runtime.state.active_contact_pairs.clear();
        runtime.rebuild_physics_for_test();
        runtime
            .advance(100)
            .expect("nearby clear grid motion commits");
        assert!(runtime.state().player.position.x > clear_start.x + 0.2);
    }

    #[test]
    fn off_center_rotating_compound_uses_center_of_mass_motion_envelopes() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 137, 1).expect("runtime opens");
            let mut grid = test_grid(
                "off-center-grid",
                Vec3::new(100.0, 100.0, 100.0),
                Vec3::ZERO,
                [
                    Block::new("off-center-core", IVec3::ZERO, BlockKind::ControlCore),
                    Block::new(
                        "off-center-tip",
                        IVec3::new(100, 0, 0),
                        BlockKind::Structural,
                    ),
                ],
            );
            grid.angular_velocity = Vec3::new(0.0, 0.0, 3.0);
            replace_with_physics_fixture(
                &mut runtime,
                [grid],
                VoxelField {
                    occupied: BTreeSet::new(),
                    ferrite_ore: BTreeSet::new(),
                },
            );
            let prior_origin = runtime.state().grids["off-center-grid"].position;

            runtime
                .advance(17)
                .expect("valid off-center rotation stays inside the COM envelope");

            let rotated = &runtime.state().grids["off-center-grid"];
            assert!(rotated.position.squared_distance(prior_origin).sqrt() > 0.8);
            assert!(rotated.angular_velocity.z > 2.9);
            expected_hash = runtime.state().state_hash();
        }
        let recovered = Runtime::open(directory.path(), 137, 1).expect("fixture recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn physics_commit_failpoints_recover_the_complete_prior_or_durable_tick() {
        let before_directory = tempdir().expect("tempdir");
        let prior_hash;
        let prior_sequence;
        {
            let mut runtime = Runtime::open(before_directory.path(), 79, 100)
                .expect("runtime starts for pre-write failure");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetGridControl {
                    operation_sequence: 0,
                    operation_id: "pre-write-control".into(),
                    grid_id: STARTER_GRID_ID.into(),
                    linear_input: Vec3::new(0.0, 0.0, -1.0),
                    angular_input: Vec3::ZERO,
                    dampeners: false,
                })
                .expect("control is durable");
            prior_hash = runtime.state().state_hash();
            prior_sequence = runtime.state().event_sequence;
            runtime
                .store
                .set_append_failpoint(AppendFailpoint::BeforeWrite);
            assert!(matches!(
                runtime.advance(17),
                Err(RuntimeError::Persistence(
                    PersistenceError::InjectedFailure("before journal write")
                ))
            ));
            assert!(runtime.is_halted());
            assert_eq!(runtime.state().state_hash(), prior_hash);
            assert_eq!(runtime.state().event_sequence, prior_sequence);
            assert_eq!(runtime.state().simulation_tick, 0);
        }
        let recovered =
            Runtime::open(before_directory.path(), 79, 100).expect("pre-write failure recovers");
        assert_eq!(recovered.state().state_hash(), prior_hash);
        assert_eq!(recovered.state().event_sequence, prior_sequence);
        assert_eq!(recovered.state().simulation_tick, 0);

        let after_directory = tempdir().expect("tempdir");
        let durable_sequence;
        let before_durable_hash;
        let expected_durable_state;
        {
            let mut runtime = Runtime::open(after_directory.path(), 83, 100)
                .expect("runtime starts for post-sync failure");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetGridControl {
                    operation_sequence: 0,
                    operation_id: "post-sync-control".into(),
                    grid_id: STARTER_GRID_ID.into(),
                    linear_input: Vec3::new(0.0, 0.0, -1.0),
                    angular_input: Vec3::ZERO,
                    dampeners: false,
                })
                .expect("control is durable");
            durable_sequence = runtime.state().event_sequence + 1;
            before_durable_hash = runtime.state().state_hash();
            runtime
                .store
                .set_append_failpoint(AppendFailpoint::AfterSync);
            assert!(matches!(
                runtime.advance(17),
                Err(RuntimeError::Persistence(
                    PersistenceError::InjectedFailure("after journal sync")
                ))
            ));
            assert!(runtime.is_halted());
            assert_eq!(runtime.state().event_sequence + 1, durable_sequence);
            assert_eq!(runtime.state().simulation_tick, 0);
            assert_eq!(runtime.state().state_hash(), before_durable_hash);

            let journal = fs::read_to_string(after_directory.path().join("events.ndjson"))
                .expect("durable journal reads after injected sync failure");
            let durable_event = serde_json::from_str::<CanonicalEvent>(
                journal.lines().last().expect("durable event exists"),
            )
            .expect("durable event parses");
            let mut expected = runtime.state().clone();
            expected
                .apply_event(&durable_event)
                .expect("durable physics event applies to the complete prior state");
            expected_durable_state = expected;
        }
        let mut recovered =
            Runtime::open(after_directory.path(), 83, 100).expect("post-sync failure recovers");
        assert_eq!(recovered.state().event_sequence, durable_sequence);
        assert_eq!(recovered.state().simulation_tick, 1);
        assert_ne!(recovered.state().state_hash(), before_durable_hash);
        assert_eq!(
            recovered.state().state_hash(),
            expected_durable_state.state_hash(),
            "recovery must expose the exact complete state represented by the synced event"
        );
        let mut recovered_without_new_lease = recovered.state().clone();
        recovered_without_new_lease.fencing_token = expected_durable_state.fencing_token;
        assert_eq!(recovered_without_new_lease, expected_durable_state);
        assert!(recovered.advance(17).expect("recovered solver resumes"));
        assert_eq!(recovered.state().simulation_tick, 2);
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn mining_commit_failpoints_recover_matching_world_and_collision_fingerprints() {
        let before_directory = tempdir().expect("tempdir");
        let prior_hash;
        let prior_fingerprint;
        let before_target;
        {
            let mut runtime =
                Runtime::open(before_directory.path(), 109, 100).expect("runtime opens");
            before_target = reachable_voxel(&mut runtime);
            runtime
                .persist_snapshot()
                .expect("aimed mining baseline persists");
            let body_id =
                voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(before_target));
            let collider_id = voxel_collision_collider_id(before_target);
            prior_hash = runtime.state().state_hash();
            prior_fingerprint = runtime.physics().body_collider_fingerprint();
            assert_eq!(
                prior_fingerprint,
                expected_physics_fingerprint(runtime.state())
            );
            runtime
                .store
                .set_append_failpoint(AppendFailpoint::BeforeWrite);
            assert!(matches!(
                runtime.execute_next_for_fixture(&ClientMessage::MineVoxel {
                    operation_sequence: 0,
                    operation_id: "mine-before-write".into(),
                    coordinate: before_target,
                }),
                Err(RuntimeError::Persistence(
                    PersistenceError::InjectedFailure("before journal write")
                ))
            ));
            assert!(runtime.is_halted());
            assert_eq!(runtime.state().state_hash(), prior_hash);
            assert!(runtime.state().voxels.occupied.contains(&before_target));
            assert!(!runtime.physics().contains_collider(&body_id, &collider_id));
        }
        let recovered = Runtime::open(before_directory.path(), 109, 100)
            .expect("before-write mining failure recovers");
        assert_eq!(recovered.state().state_hash(), prior_hash);
        assert!(recovered.state().voxels.occupied.contains(&before_target));
        assert_eq!(
            recovered.physics().body_collider_fingerprint(),
            prior_fingerprint
        );

        let after_directory = tempdir().expect("tempdir");
        let expected_durable_state;
        let after_target;
        {
            let mut runtime =
                Runtime::open(after_directory.path(), 113, 100).expect("runtime opens");
            after_target = reachable_voxel(&mut runtime);
            runtime
                .persist_snapshot()
                .expect("aimed mining baseline persists");
            let body_id =
                voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(after_target));
            let collider_id = voxel_collision_collider_id(after_target);
            let prior_state = runtime.state().clone();
            runtime
                .store
                .set_append_failpoint(AppendFailpoint::AfterSync);
            assert!(matches!(
                runtime.execute_next_for_fixture(&ClientMessage::MineVoxel {
                    operation_sequence: 0,
                    operation_id: "mine-after-sync".into(),
                    coordinate: after_target,
                }),
                Err(RuntimeError::Persistence(
                    PersistenceError::InjectedFailure("after journal sync")
                ))
            ));
            assert!(runtime.is_halted());
            assert_eq!(runtime.state().state_hash(), prior_state.state_hash());
            assert!(runtime.state().voxels.occupied.contains(&after_target));
            assert!(!runtime.physics().contains_collider(&body_id, &collider_id));

            let journal = fs::read_to_string(after_directory.path().join("events.ndjson"))
                .expect("synced mining journal reads");
            let durable_event = serde_json::from_str::<CanonicalEvent>(
                journal.lines().last().expect("durable mining event exists"),
            )
            .expect("durable mining event parses");
            let mut expected = prior_state;
            expected
                .apply_event(&durable_event)
                .expect("durable mining event applies");
            expected_durable_state = expected;
        }
        let recovered = Runtime::open(after_directory.path(), 113, 100)
            .expect("after-sync mining failure recovers");
        assert_eq!(
            recovered.state().state_hash(),
            expected_durable_state.state_hash()
        );
        assert!(!recovered.state().voxels.occupied.contains(&after_target));
        assert_eq!(
            recovered.physics().body_collider_fingerprint(),
            expected_physics_fingerprint(&expected_durable_state)
        );
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn disconnected_damage_splits_grid_without_duplicating_blocks() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let build = ClientMessage::BuildBlock {
            operation_sequence: 0,
            operation_id: "build-bridge".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::DamageTest,
            orientation: 0,
        };
        runtime
            .execute_next_for_fixture(&build)
            .expect("bridge block built");
        let build_top = ClientMessage::BuildBlock {
            operation_sequence: 0,
            operation_id: "build-top".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 2, 0),
            kind: BlockKind::Structural,
            orientation: 0,
        };
        runtime
            .execute_next_for_fixture(&build_top)
            .expect("top block built");
        weld_to_completion(&mut runtime, IVec3::new(0, 1, 0), "weld-bridge");
        weld_to_completion(&mut runtime, IVec3::new(0, 2, 0), "weld-top");
        runtime
            .execute_next_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "control-before-split".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(0.25, 0.0, 0.0),
                angular_input: Vec3::new(0.0, 0.1, 0.0),
                dampeners: false,
            })
            .expect("owned grid control engages before structural split");
        let bridge_id = runtime.state().grids[STARTER_GRID_ID]
            .block_at(IVec3::new(0, 1, 0))
            .expect("bridge block")
            .block_id
            .clone();
        for index in 0..2 {
            aim_player_at_block(&mut runtime, STARTER_GRID_ID, &bridge_id);
            runtime
                .execute_next_for_fixture(&ClientMessage::DamageBlock {
                    operation_sequence: 0,
                    operation_id: format!("damage-{index}"),
                    grid_id: STARTER_GRID_ID.into(),
                    block_id: bridge_id.clone(),
                })
                .expect("damage accepted");
        }
        let block_ids = runtime
            .state()
            .grids
            .values()
            .flat_map(|grid| grid.blocks.keys().cloned())
            .collect::<Vec<_>>();
        assert_eq!(
            block_ids.len(),
            block_ids.iter().collect::<BTreeSet<_>>().len()
        );
        assert_eq!(runtime.state().grids.len(), 3);
        assert!(runtime.state().grids.values().all(|grid| {
            grid.owner_player_id == "player-local"
                && grid.control_linear_input == Vec3::ZERO
                && grid.control_angular_input == Vec3::ZERO
                && grid.dampeners
        }));
        assert_eq!(
            runtime
                .state()
                .grids
                .values()
                .filter(|grid| grid.anchor_reward_eligible)
                .count(),
            1
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn authoritative_grid_controls_cannot_drive_through_asteroid_voxels() {
        let mut runtime = runtime();
        runtime
            .execute_next_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "drive-into-asteroid".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(-1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                dampeners: false,
            })
            .expect("bounded powered control accepted");

        for _ in 0..24 {
            runtime.advance(17).expect("authoritative physics step");
        }

        let grid = &runtime.state().grids[STARTER_GRID_ID];
        assert!(
            grid.position.x >= 10.95,
            "contact solver allowed the starter grid to enter the asteroid: {}",
            grid.position.x
        );
        assert!(grid.linear_velocity.x > -0.25);
        assert!(runtime.state().simulation_tick >= 24);
    }

    #[test]
    fn grid_control_replay_revalidates_bounds_power_and_anchor_state() {
        let state = WorldState::genesis(172);
        let valid = state
            .prepare_next_client_event_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "canonical-grid-control".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(0.25, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                dampeners: true,
            })
            .expect("canonical grid control prepares");

        let mut oversized = valid.clone();
        let EventPayload::GridControlSet { linear_input, .. } = &mut oversized.payload else {
            unreachable!();
        };
        *linear_input = Vec3::new(1.25, 0.0, 0.0);
        oversized.event_hash = oversized.calculate_hash();

        let mut anchored_state = state.clone();
        anchored_state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists")
            .anchored = true;
        let mut unpowered_state = state.clone();
        for block in unpowered_state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists")
            .blocks
            .values_mut()
        {
            if matches!(block.kind, BlockKind::PowerSource | BlockKind::Battery) {
                block.construction_complete = false;
            }
        }

        for (mut candidate, event, expected_code) in [
            (
                state.clone(),
                oversized,
                "replay_intent_fingerprint_mismatch",
            ),
            (anchored_state, valid.clone(), "replay_grid_control_invalid"),
            (unpowered_state, valid, "replay_grid_control_invalid"),
        ] {
            let before = candidate.state_hash();
            let error = candidate
                .apply_event(&event)
                .expect_err("forged grid control replay rejects");
            assert_eq!(error.code(), expected_code);
            assert_eq!(candidate.state_hash(), before);
        }
    }

    #[test]
    fn runtime_equal_mass_grid_collision_commits_and_recovers_within_tolerance() {
        const MOMENTUM_TOLERANCE_KG_MPS: f64 = 1.0;

        let directory = tempdir().expect("tempdir");
        let expected_hash;
        let expected_sequence;
        let expected_tick;
        let expected_contacts;
        {
            let mut runtime =
                Runtime::open(directory.path(), 89, 100).expect("collision fixture runtime opens");
            let alpha = test_grid(
                "alpha-grid",
                Vec3::new(-2.0, 0.0, 0.0),
                Vec3::new(4.0, 0.0, 0.0),
                [Block::new(
                    "alpha-armor",
                    IVec3::ZERO,
                    BlockKind::Structural,
                )],
            );
            let zeta = test_grid(
                "zeta-grid",
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(-4.0, 0.0, 0.0),
                [Block::new("zeta-armor", IVec3::ZERO, BlockKind::Structural)],
            );
            replace_with_physics_fixture(
                &mut runtime,
                [zeta, alpha],
                VoxelField {
                    occupied: BTreeSet::new(),
                    ferrite_ore: BTreeSet::new(),
                },
            );
            let initial_mass_grams = runtime
                .state()
                .grid_mass_grams(&runtime.state().grids["alpha-grid"]);
            assert_eq!(
                initial_mass_grams,
                runtime
                    .state()
                    .grid_mass_grams(&runtime.state().grids["zeta-grid"])
            );
            let initial_mass = runtime
                .state()
                .grid_mass_kg(&runtime.state().grids["alpha-grid"]);

            let contacted = (0..90).any(|_| {
                runtime.advance(17).expect("runtime physics advances");
                runtime
                    .state()
                    .active_contact_pairs
                    .iter()
                    .any(|pair| pair.body_a == "alpha-grid" && pair.body_b == "zeta-grid")
            });
            assert!(
                contacted,
                "authoritative grids must produce a canonical contact"
            );
            let alpha = &runtime.state().grids["alpha-grid"];
            let zeta = &runtime.state().grids["zeta-grid"];
            assert!(
                alpha.position.x < zeta.position.x,
                "grids cannot pass through"
            );
            assert!(alpha.linear_velocity.x < 0.0, "alpha grid must recoil");
            assert!(zeta.linear_velocity.x > 0.0, "zeta grid must recoil");
            let total_momentum =
                alpha.linear_velocity * initial_mass + zeta.linear_velocity * initial_mass;
            assert!(
                total_momentum.magnitude() <= MOMENTUM_TOLERANCE_KG_MPS,
                "committed vector momentum error exceeded {MOMENTUM_TOLERANCE_KG_MPS} kg m/s: {total_momentum:?}"
            );
            assert!(runtime.state().conservation().valid);
            expected_hash = runtime.state().state_hash();
            expected_sequence = runtime.state().event_sequence;
            expected_tick = runtime.state().simulation_tick;
            expected_contacts = runtime.state().active_contact_pairs.clone();
        }

        let recovered =
            Runtime::open(directory.path(), 89, 100).expect("committed collision runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert_eq!(recovered.state().event_sequence, expected_sequence);
        assert_eq!(recovered.state().simulation_tick, expected_tick);
        assert_eq!(recovered.state().active_contact_pairs, expected_contacts);
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn runtime_cargo_mass_reduces_acceleration_under_the_same_force() {
        fn accelerate(runtime: &mut Runtime, operation_prefix: &str) -> f64 {
            runtime
                .execute_next_for_fixture(&ClientMessage::SetGridControl {
                    operation_sequence: 0,
                    operation_id: format!("{operation_prefix}-control"),
                    grid_id: STARTER_GRID_ID.into(),
                    linear_input: Vec3::new(1.0, 0.0, 0.0),
                    angular_input: Vec3::ZERO,
                    dampeners: false,
                })
                .expect("powered control is accepted");
            for _ in 0..10 {
                runtime.advance(17).expect("runtime physics advances");
            }
            runtime.state().grids[STARTER_GRID_ID].linear_velocity.x
        }

        let light_directory = tempdir().expect("tempdir");
        let mut light = Runtime::open(light_directory.path(), 97, 100).expect("light runtime");
        let light_mass = light
            .state()
            .grid_mass_grams(&light.state().grids[STARTER_GRID_ID]);
        let light_velocity = accelerate(&mut light, "light");

        let heavy_directory = tempdir().expect("tempdir");
        let mut heavy = Runtime::open(heavy_directory.path(), 97, 100).expect("heavy runtime");
        let cargo_id = "inventory-cargo-starter".to_owned();
        heavy
            .execute_next_for_fixture(&ClientMessage::TransferInventory {
                operation_sequence: 0,
                operation_id: "load-physical-cargo".into(),
                source_inventory_id: PLAYER_INVENTORY_ID.into(),
                destination_inventory_id: cargo_id.clone(),
                resource: ResourceKind::Component,
                quantity: 24,
            })
            .expect("cargo transfer is accepted");
        let cargo_mass = heavy.state().inventories[&cargo_id].mass_grams();
        let heavy_mass = heavy
            .state()
            .grid_mass_grams(&heavy.state().grids[STARTER_GRID_ID]);
        assert_eq!(heavy_mass - light_mass, cargo_mass);
        let heavy_velocity = accelerate(&mut heavy, "heavy");

        assert!(light_velocity > 0.0);
        assert!(heavy_velocity > 0.0);
        assert!(
            heavy_velocity < light_velocity,
            "loaded grid must accelerate less: loaded={heavy_velocity} empty={light_velocity}"
        );
        assert!(light.state().conservation().valid);
        assert!(heavy.state().conservation().valid);
    }

    #[test]
    fn runtime_powered_resting_contact_stays_within_the_published_interval_bounds() {
        const SETTLE_TICKS: u64 = 120;
        const OBSERVATION_TICKS: u64 = 120;

        let directory = tempdir().expect("tempdir");
        let mut runtime =
            Runtime::open(directory.path(), 101, 1_000).expect("resting fixture runtime opens");
        let mut floor = test_grid(
            "floor-grid",
            Vec3::new(0.0, -1.0, 0.0),
            Vec3::ZERO,
            [
                Block::new("floor-anchor", IVec3::ZERO, BlockKind::Anchor),
                Block::new("floor-battery", IVec3::new(1, 0, 0), BlockKind::Battery),
            ],
        );
        floor.anchored = true;
        let resting = test_grid(
            "resting-grid",
            Vec3::ZERO,
            Vec3::ZERO,
            [Block::new(
                "resting-battery",
                IVec3::ZERO,
                BlockKind::Battery,
            )],
        );
        replace_with_physics_fixture(
            &mut runtime,
            [resting, floor],
            VoxelField {
                occupied: BTreeSet::from([IVec3::new(0, -2, 0)]),
                ferrite_ore: BTreeSet::new(),
            },
        );
        assert!(runtime.state().grids["floor-grid"].anchor_touches(&runtime.state().voxels));
        assert!(runtime.state().grids["floor-grid"].power().online);
        runtime
            .execute_next_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "press-grid-onto-floor".into(),
                grid_id: "resting-grid".into(),
                linear_input: Vec3::new(0.0, -0.2, 0.0),
                angular_input: Vec3::ZERO,
                dampeners: true,
            })
            .expect("powered settling force is accepted");

        let target_tick = SETTLE_TICKS + OBSERVATION_TICKS;
        let mut observation_samples = 0_u64;
        let mut observation_contact_samples = 0_u64;
        let mut resting_origin = None;
        let mut maximum_translation_drift: f64 = 0.0;
        let mut maximum_linear_speed: f64 = 0.0;
        let mut maximum_angular_speed: f64 = 0.0;
        while runtime.state().simulation_tick < target_tick {
            runtime.advance(17).expect("resting physics advances");
            if runtime.state().simulation_tick > SETTLE_TICKS {
                observation_samples += 1;
                if runtime
                    .state()
                    .active_contact_pairs
                    .iter()
                    .any(|pair| pair.body_a == "floor-grid" && pair.body_b == "resting-grid")
                {
                    observation_contact_samples += 1;
                }
                let grid = &runtime.state().grids["resting-grid"];
                let origin = *resting_origin.get_or_insert(grid.position);
                maximum_translation_drift =
                    maximum_translation_drift.max(grid.position.squared_distance(origin).sqrt());
                maximum_linear_speed = maximum_linear_speed.max(grid.linear_velocity.magnitude());
                maximum_angular_speed =
                    maximum_angular_speed.max(grid.angular_velocity.magnitude());
            }
        }
        assert_eq!(
            observation_contact_samples, observation_samples,
            "resting grid must retain canonical floor contact throughout observation"
        );
        assert!(observation_samples > 0);
        assert!(
            maximum_translation_drift <= 1.0e-4,
            "two-second committed translation drift exceeded 0.1 mm: {maximum_translation_drift}"
        );
        assert!(
            maximum_linear_speed <= 1.0e-3,
            "committed resting speed exceeded 1 mm/s: {maximum_linear_speed}"
        );
        assert!(
            maximum_angular_speed <= 1.0e-3,
            "committed resting angular speed exceeded 0.001 rad/s: {maximum_angular_speed}"
        );
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn runtime_valid_anchor_survives_impact_then_unanchors_without_asset_change() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime =
                Runtime::open(directory.path(), 103, 1_000).expect("anchor fixture runtime opens");
            let mut anchored = test_grid(
                "anchored-grid",
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::ZERO,
                [
                    Block::new("anchor-block", IVec3::ZERO, BlockKind::Anchor),
                    Block::new("anchor-battery", IVec3::new(1, 0, 0), BlockKind::Battery),
                    {
                        let mut cargo =
                            Block::new("anchor-cargo", IVec3::new(0, 1, 0), BlockKind::Cargo);
                        cargo.inventory_id = Some("inventory-anchor-cargo".into());
                        cargo
                    },
                ],
            );
            anchored.anchored = true;
            let striker = test_grid(
                "striker-grid",
                Vec3::new(4.0, 0.0, 0.0),
                Vec3::new(-8.0, 0.0, 0.0),
                [Block::new(
                    "striker-armor",
                    IVec3::ZERO,
                    BlockKind::Structural,
                )],
            );
            replace_with_physics_fixture(
                &mut runtime,
                [striker, anchored],
                VoxelField {
                    occupied: BTreeSet::from([IVec3::ZERO]),
                    ferrite_ore: BTreeSet::new(),
                },
            );
            runtime
                .state
                .inventories
                .get_mut(PLAYER_INVENTORY_ID)
                .expect("player inventory exists")
                .contents
                .components -= 4;
            runtime.state.inventories.insert(
                "inventory-anchor-cargo".into(),
                InventoryRecord {
                    inventory_id: "inventory-anchor-cargo".into(),
                    domain: InventoryDomain::Cargo {
                        block_id: "anchor-cargo".into(),
                    },
                    contents: InventoryContents {
                        ore: 0,
                        refined_material: 0,
                        components: 4,
                    },
                    capacity_liters: CARGO_INVENTORY_CAPACITY_LITERS,
                },
            );
            assert!(runtime.state().conservation().valid);
            runtime.rebuild_physics_for_test();
            runtime
                .persist_snapshot()
                .expect("cargo-bearing anchor fixture snapshot persists");
            assert!(runtime.state().grids["anchored-grid"].anchor_touches(&runtime.state().voxels));
            assert!(runtime.state().grids["anchored-grid"].power().online);
            assert_eq!(
                runtime.state().inventories["inventory-anchor-cargo"]
                    .contents
                    .components,
                4
            );
            assert_eq!(
                runtime.state().grids["anchored-grid"].blocks["anchor-cargo"].inventory_id,
                Some("inventory-anchor-cargo".into())
            );
            let before_rejected_control = runtime.state().state_hash();
            assert!(matches!(
                runtime.execute_next_for_fixture(&ClientMessage::SetGridControl {
                    operation_sequence: 0,
                    operation_id: "reject-anchored-control".into(),
                    grid_id: "anchored-grid".into(),
                    linear_input: Vec3::new(1.0, 0.0, 0.0),
                    angular_input: Vec3::ZERO,
                    dampeners: false,
                }),
                Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                    if code == "grid_is_anchored"
            ));
            assert_eq!(runtime.state().state_hash(), before_rejected_control);
            let initial_pose = (
                runtime.state().grids["anchored-grid"].position,
                runtime.state().grids["anchored-grid"].orientation,
            );
            let mut observed_impact = false;
            for _ in 0..120 {
                runtime.advance(17).expect("impact physics advances");
                observed_impact |=
                    runtime.state().active_contact_pairs.iter().any(|pair| {
                        pair.body_a == "anchored-grid" && pair.body_b == "striker-grid"
                    });
                let anchor = &runtime.state().grids["anchored-grid"];
                assert_eq!((anchor.position, anchor.orientation), initial_pose);
                assert_eq!(anchor.linear_velocity, Vec3::ZERO);
                assert_eq!(anchor.angular_velocity, Vec3::ZERO);
            }
            assert!(observed_impact, "striker must contact the anchored grid");

            let blocks_before = runtime.state().grids["anchored-grid"].blocks.clone();
            let inventories_before = runtime.state().inventories.clone();
            let mass_before = runtime
                .state()
                .grid_mass_grams(&runtime.state().grids["anchored-grid"]);
            runtime
                .execute_next_for_fixture(&ClientMessage::ToggleGridAnchor {
                    operation_sequence: 0,
                    operation_id: "release-final-anchor".into(),
                    grid_id: "anchored-grid".into(),
                })
                .expect("last anchor releases");
            let released = &runtime.state().grids["anchored-grid"];
            assert!(!released.anchored);
            assert_eq!((released.position, released.orientation), initial_pose);
            assert_eq!(released.blocks, blocks_before);
            assert_eq!(runtime.state().inventories, inventories_before);
            assert_eq!(
                runtime.state().inventories["inventory-anchor-cargo"]
                    .contents
                    .components,
                4
            );
            assert_eq!(
                released.blocks["anchor-cargo"].inventory_id,
                Some("inventory-anchor-cargo".into())
            );
            assert_eq!(runtime.state().grid_mass_grams(released), mass_before);
            assert!(runtime.state().conservation().valid);

            runtime
                .execute_next_for_fixture(&ClientMessage::SetGridControl {
                    operation_sequence: 0,
                    operation_id: "move-released-anchor".into(),
                    grid_id: "anchored-grid".into(),
                    linear_input: Vec3::new(1.0, 0.0, 0.0),
                    angular_input: Vec3::ZERO,
                    dampeners: false,
                })
                .expect("released powered grid accepts control");
            for _ in 0..10 {
                runtime.advance(17).expect("released grid physics advances");
            }
            assert!(runtime.state().grids["anchored-grid"].position.x > initial_pose.0.x);
            expected_hash = runtime.state().state_hash();
        }
        let recovered =
            Runtime::open(directory.path(), 103, 1_000).expect("released anchor runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert!(!recovered.state().grids["anchored-grid"].anchored);
        assert_eq!(
            recovered.state().inventories["inventory-anchor-cargo"]
                .contents
                .components,
            4
        );
        assert_eq!(
            recovered.state().grids["anchored-grid"].blocks["anchor-cargo"].inventory_id,
            Some("inventory-anchor-cargo".into())
        );
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn canonical_contact_lifecycle_and_estimate_survive_scene_rebuild_and_restart() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 42, 1).expect("runtime opens");
        runtime
            .execute_next_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "contact-lifecycle-thrust".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(-1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                dampeners: false,
            })
            .expect("control accepted");

        let mut first_pairs = BTreeSet::new();
        for _ in 0..120 {
            runtime.advance(17).expect("physics advances");
            if !runtime.state().active_contact_pairs.is_empty() {
                first_pairs = runtime.state().active_contact_pairs.clone();
                break;
            }
        }
        assert!(!first_pairs.is_empty(), "grid must reach the asteroid");

        let events = fs::read_to_string(directory.path().join("events.ndjson"))
            .expect("journal reads")
            .lines()
            .map(|line| serde_json::from_str::<CanonicalEvent>(line).expect("event parses"))
            .collect::<Vec<_>>();
        let first_contact = events
            .iter()
            .rev()
            .find_map(|event| match &event.payload {
                EventPayload::PhysicsStepCommitted { contacts, .. } if !contacts.is_empty() => {
                    Some(contacts)
                }
                _ => None,
            })
            .expect("contact commit exists");
        assert!(
            first_contact
                .iter()
                .any(|contact| contact.phase == PhysicsContactPhase::Began)
        );
        assert!(first_contact.iter().any(|contact| {
            contact.closing_speed_mm_per_second > 0
                && contact.estimated_normal_impulse_millinewton_seconds > 0
                && contact.reduced_translational_mass_grams > 0
        }));

        let mut persisted = false;
        for _ in 0..12 {
            runtime.advance(17).expect("continued contact advances");
            let events =
                fs::read_to_string(directory.path().join("events.ndjson")).expect("journal reads");
            let event: CanonicalEvent =
                serde_json::from_str(events.lines().last().expect("latest event exists"))
                    .expect("latest event parses");
            if let EventPayload::PhysicsStepCommitted { contacts, .. } = event.payload
                && contacts.iter().any(|contact| {
                    contact.phase == PhysicsContactPhase::Persisted
                        && first_pairs.contains(&ContactPairKey {
                            body_a: contact.body_a_id.clone(),
                            collider_a: contact.collider_a_id.clone(),
                            body_b: contact.body_b_id.clone(),
                            collider_b: contact.collider_b_id.clone(),
                        })
                })
            {
                persisted = true;
                break;
            }
        }
        assert!(persisted, "scene rebuild must not recreate canonical onset");

        let before_restart = runtime.state().active_contact_pairs.clone();
        drop(runtime);
        let recovered = Runtime::open(directory.path(), 42, 1).expect("runtime recovers");
        assert_eq!(recovered.state().active_contact_pairs, before_restart);
    }

    #[test]
    fn committed_physics_outcome_recovers_without_resimulation() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 7, 1_000).expect("runtime opens");
        runtime
            .execute_next_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "recovery-thrust".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(0.0, 0.0, 0.5),
                angular_input: Vec3::new(0.0, 0.15, 0.0),
                dampeners: true,
            })
            .expect("control accepted");
        for _ in 0..8 {
            runtime.advance(17).expect("physics advances");
        }
        let expected_hash = runtime.state().state_hash();
        let expected_pose = runtime.state().grids[STARTER_GRID_ID].clone();
        drop(runtime);

        let recovered = Runtime::open(directory.path(), 7, 1_000).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        let recovered_grid = &recovered.state().grids[STARTER_GRID_ID];
        assert_eq!(recovered_grid.position, expected_pose.position);
        assert_eq!(recovered_grid.orientation, expected_pose.orientation);
        assert_eq!(
            recovered_grid.linear_velocity,
            expected_pose.linear_velocity
        );
        assert_eq!(
            recovered_grid.angular_velocity,
            expected_pose.angular_velocity
        );
    }

    #[test]
    fn fixed_step_tick_and_fractional_phase_survive_recovery() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 7, 1).expect("runtime opens");
        runtime
            .execute_next_for_fixture(&ClientMessage::SetGridControl {
                operation_sequence: 0,
                operation_id: "timing-thrust".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(0.0, 0.0, 0.25),
                angular_input: Vec3::ZERO,
                dampeners: true,
            })
            .expect("control accepted");

        for _ in 0..10 {
            runtime.advance(100).expect("physics batch advances");
        }
        assert_eq!(runtime.state().simulation_tick, 60);
        assert_eq!(runtime.state().physics_step_phase, 0);

        runtime.advance(17).expect("fractional batch advances");
        assert_eq!(runtime.state().simulation_tick, 61);
        assert_eq!(runtime.state().physics_step_phase, 20_000_000);
        drop(runtime);

        let mut recovered = Runtime::open(directory.path(), 7, 1).expect("runtime recovers");
        assert_eq!(recovered.state().simulation_tick, 61);
        assert_eq!(recovered.state().physics_step_phase, 20_000_000);
        assert!(!recovered.advance(16).expect("substep phase accumulates"));
        assert!(
            recovered
                .advance(1)
                .expect("recovered phase completes a step")
        );
        assert_eq!(recovered.state().simulation_tick, 62);
        assert_eq!(recovered.state().physics_step_phase, 40_000_000);
    }
}
