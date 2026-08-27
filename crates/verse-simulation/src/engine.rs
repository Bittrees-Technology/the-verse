// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use thiserror::Error;
use verse_physics::{
    BodyControl, BodySpec, BoxColliderSpec, CapsuleCast, CapsuleColliderSpec, MotionQuality,
    PhysicsError, Pose as PhysicsPose, Quat as PhysicsQuat, Scene, SceneConfig, SphereColliderSpec,
    Vec3 as PhysicsVec3,
};
use verse_protocol::{
    BlockKind, ClientMessage, IVec3, IntentReceipt, InventoryContents, InventoryDomain,
    LocomotionKind, LocomotionSupportSnapshot, MotionSnapshot, PlayerDeathCause, PlayerLifeState,
    PlayerLocomotionSnapshot, Quat, ResourceKind, Vec3, WorldSnapshot,
};

use crate::content;
use crate::event::{
    CanonicalEvent, EVENT_SCHEMA_NAME, EVENT_SCHEMA_VERSION, EventPayload, PhysicsBodyOutcome,
    PhysicsContactOutcome, PhysicsContactPhase, PlayerPhysicsOutcome,
};
use crate::model::{
    Block, CARGO_INVENTORY_CAPACITY_LITERS, ContactPairKey, DeathDrop, Grid, InventoryRecord,
    PLANET_CENTER, PLANET_SURFACE_RADIUS_M, PLAYER_INVENTORY_ID, Player, PlayerControlFrame,
    WorldState, radial_up,
};
use crate::persistence::{PersistenceError, Store};

const PLAYER_BODY_ID: &str = "player-body-player-local";
const PLAYER_COLLIDER_ID: &str = "player-collider-player-local";
const PLANET_BODY_ID: &str = "planet-body-khepri-prime";
const PLANET_COLLIDER_ID: &str = "planet-collider-khepri-prime";
const MINING_RANGE: f64 = 8.5;
const HAND_TOOL_RANGE: f64 = 9.0;
const MAX_GRID_CONTROL_INPUT: f64 = 1.0;
const CONTROL_INPUT_EPSILON: f64 = 1.0e-9;
const MAX_GRID_BLOCKS_P0: usize = 2_048;
const MAX_PENDING_PLAYER_CONTROL_FRAMES: usize = 64;
const PLAYER_POSITION_CORRECTION_BUDGET_M_PER_STEP: f64 = 0.25;
const PLAYER_ROTATION_SLOP_RADIANS_PER_STEP: f64 = 0.000_1;
const REPLAY_QUANTIZATION_SLOP: f64 = 0.000_004;
#[cfg(test)]
const REPLAY_CONTACT_SLOP_M: f64 = 0.15;
const PHYSICS_SPECULATIVE_DISTANCE_M: f64 = 0.02;
const PHYSICS_CONTACT_POINT_SLOP_M: f64 = 0.001;
const PLAYER_PLANET_PENETRATION_LIMIT_M: f64 = 0.28;
const PLAYER_BOX_PENETRATION_LIMIT_M: f64 = 0.85;
const CHARACTER_INERTIA_MULTIPLIER: f64 = 12.0;

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
}

#[derive(Debug)]
pub struct Runtime {
    store: Store,
    state: WorldState,
    snapshot_every: u64,
    events_since_snapshot: u64,
    life_support_elapsed_millis: u32,
    physics_step_phase: u64,
    physics: Scene,
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
        let mut store = Store::open(data_directory, requested_seed)?;
        let mut state = store.load_world()?;
        state.fencing_token = store.fencing_token();

        let mut physics = Scene::new(physics_scene_config())?;
        physics.rebuild(&physics_body_specs(&state))?;
        let physics_step_phase = state.physics_step_phase;
        let mut runtime = Self {
            store,
            state,
            snapshot_every: snapshot_every.max(1),
            events_since_snapshot: 0,
            life_support_elapsed_millis: 0,
            physics_step_phase,
            physics,
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

    pub const fn state(&self) -> &WorldState {
        &self.state
    }

    #[cfg(test)]
    pub(crate) fn relocate_player_for_test(&mut self, position: Vec3) {
        self.state.player.position = position;
        self.state.player.orientation = Quat::IDENTITY;
        self.state.player.linear_velocity = Vec3::ZERO;
        self.state.player.angular_velocity = Vec3::ZERO;
        self.state.player.surface_contact = false;
        self.physics
            .rebuild(&physics_body_specs(&self.state))
            .expect("test relocation must produce a valid physics scene");
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
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        let operation_id = message.operation_id().ok_or_else(|| {
            IntentError::rejected(
                "not_a_mutating_intent",
                "hello and snapshot requests are handled by the network service",
            )
        })?;

        if let Some(receipt) = self.state.processed_operations.get(operation_id) {
            return Ok(receipt.clone());
        }

        let event = self.state.prepare_client_event(message)?;
        let mut next_state = self.state.clone();
        next_state.apply_event(&event)?;
        if let EventPayload::VoxelMined { coordinate, .. } = &event.payload {
            let chunk = voxel_collision_chunk_coordinate(*coordinate);
            let body_id = voxel_collision_chunk_body_id(chunk);
            self.physics.replace_body(
                &body_id,
                voxel_collision_chunk_body_spec(&next_state, chunk),
            )?;
            #[cfg(test)]
            {
                self.physics_chunk_replacements += 1;
            }
        } else if event_changes_physics_scene(&event.payload) {
            self.physics.rebuild(&physics_body_specs(&next_state))?;
            #[cfg(test)]
            {
                self.physics_full_rebuilds += 1;
            }
        }
        if let Err(source) = self.store.append_event(&event) {
            self.halted = true;
            return Err(source.into());
        }
        self.state = next_state;
        self.after_event()?;

        self.state
            .processed_operations
            .get(operation_id)
            .cloned()
            .ok_or_else(|| {
                IntentError::rejected(
                    "receipt_missing",
                    "accepted operation did not produce a durable receipt",
                )
                .into()
            })
    }

    pub fn advance(&mut self, delta_millis: u16) -> Result<bool, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        let moving_grid = self.state.grids.values().any(|grid| {
            !grid.anchored
                && (grid.linear_velocity.magnitude() > f64::EPSILON
                    || grid.angular_velocity.magnitude() > f64::EPSILON
                    || grid.control_linear_input.magnitude() > f64::EPSILON
                    || grid.control_angular_input.magnitude() > f64::EPSILON)
        });
        let player_physics_active = matches!(self.state.player.life_state, PlayerLifeState::Alive)
            && (self.state.player.linear_velocity.magnitude() > f64::EPSILON
                || self.state.player.angular_velocity.magnitude() > f64::EPSILON
                || self.state.player.control_linear_input.magnitude() > f64::EPSILON
                || self.state.player.control_angular_input.magnitude() > f64::EPSILON
                || !self.state.player.pending_control_frames.is_empty()
                || self.state.player.boost
                || self.state.simulation_tick
                    < self.state.player.control_expires_at_simulation_tick
                || !self.state.player.dampeners
                || !self.state.player.jetpack_enabled);
        let physics_active = moving_grid || player_physics_active;
        let delta_millis = delta_millis.clamp(1, 250);
        let mut changed = false;
        if physics_active {
            let fixed_step_hz = content::manifest().physics.fixed_step_hz;
            self.physics_step_phase = self
                .physics_step_phase
                .saturating_add(u64::from(delta_millis) * 1_000_000 * u64::from(fixed_step_hz));
            let step_count = (self.physics_step_phase / 1_000_000_000).min(15);
            if step_count > 0 {
                self.physics_step_phase -= step_count * 1_000_000_000;
                let mut body_states = match self.physics.body_states() {
                    Ok(bodies) => bodies,
                    Err(source) => {
                        self.halted = true;
                        return Err(source.into());
                    }
                };
                let mut output = None;
                let mut contacts = Vec::new();
                let mut active_contacts = self.state.active_contact_pairs.clone();
                let mut scheduled_player = self.state.player.clone();
                for substep_index in 0..step_count {
                    let substep_simulation_tick =
                        self.state.simulation_tick.saturating_add(substep_index);
                    advance_player_control_for_substep(
                        &mut scheduled_player,
                        substep_simulation_tick,
                    );
                    let player_jump = classify_player_locomotion_for_substep(
                        &self.state,
                        &self.physics,
                        &mut scheduled_player,
                        &body_states,
                        substep_simulation_tick,
                    )?;
                    let controls = physics_controls(
                        &self.state,
                        &scheduled_player,
                        &body_states,
                        substep_simulation_tick,
                        player_jump,
                    );
                    let step = match self.physics.step(&controls) {
                        Ok(step) => step,
                        Err(source) => {
                            self.halted = true;
                            return Err(source.into());
                        }
                    };
                    if let (Some(prior), Some(result)) = (
                        body_states
                            .iter()
                            .find(|body| body.body_id == PLAYER_BODY_ID),
                        step.bodies
                            .iter()
                            .find(|body| body.body_id == PLAYER_BODY_ID),
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
                    contacts.extend(step.contacts.iter().map(|contact| {
                        let key = contact_pair_key(contact);
                        let phase = if active_contacts.contains(&key) {
                            PhysicsContactPhase::Persisted
                        } else {
                            PhysicsContactPhase::Began
                        };
                        physics_contact_outcome(&self.state, contact, substep_index, phase)
                    }));
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
                        .map(physics_body_outcome)
                        .collect(),
                    player: output
                        .bodies
                        .iter()
                        .find(|body| body.body_id == PLAYER_BODY_ID)
                        .map(|body| {
                            player_physics_outcome(
                                &scheduled_player,
                                body,
                                active_contacts.iter().any(contact_key_involves_player),
                                self.state.simulation_tick.saturating_add(step_count),
                            )
                        }),
                    contacts,
                    active_contacts_after: active_contacts.into_iter().collect(),
                };
                if let Err(source) = self.commit_system_event(payload) {
                    self.halted = true;
                    return Err(source);
                }
                if let Err(source) = self.physics.rebuild(&physics_body_specs(&self.state)) {
                    self.halted = true;
                    return Err(source.into());
                }
                #[cfg(test)]
                {
                    self.physics_full_rebuilds += 1;
                }
                changed = true;
            }
        }

        let player_life_support_active =
            matches!(self.state.player.life_state, PlayerLifeState::Alive);
        if player_life_support_active {
            self.life_support_elapsed_millis = self
                .life_support_elapsed_millis
                .saturating_add(u32::from(delta_millis));
        } else if !matches!(self.state.player.life_state, PlayerLifeState::Alive) {
            self.life_support_elapsed_millis = 0;
        }
        if player_life_support_active && self.life_support_elapsed_millis >= 1_000 {
            let elapsed_seconds = self.life_support_elapsed_millis / 1_000;
            self.life_support_elapsed_millis %= 1_000;
            for _ in 0..elapsed_seconds {
                let Some(payload) = self.state.life_support_payload_after_one_second()? else {
                    continue;
                };
                self.commit_system_event(payload)?;
                changed = true;
                if !matches!(self.state.player.life_state, PlayerLifeState::Alive) {
                    break;
                }
            }
        }
        Ok(changed)
    }

    fn commit_system_event(&mut self, payload: EventPayload) -> Result<(), RuntimeError> {
        let event = self.state.prepare_system_event(payload);
        let mut next_state = self.state.clone();
        next_state.apply_event(&event)?;
        if event_changes_physics_scene(&event.payload) {
            self.physics.rebuild(&physics_body_specs(&next_state))?;
            #[cfg(test)]
            {
                self.physics_full_rebuilds += 1;
            }
        }
        if let Err(source) = self.store.append_event(&event) {
            self.halted = true;
            return Err(source.into());
        }
        self.state = next_state;
        self.after_event()?;
        Ok(())
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

    fn after_event(&mut self) -> Result<(), RuntimeError> {
        self.events_since_snapshot += 1;
        if self.events_since_snapshot >= self.snapshot_every {
            self.persist_snapshot()?;
        }
        Ok(())
    }
}

impl WorldState {
    fn next_suit_oxygen_after_one_second(&self) -> Result<u16, IntentError> {
        if !matches!(self.player.life_state, PlayerLifeState::Alive)
            || self.player.suit_oxygen_milli == 0
        {
            return Err(IntentError::rejected(
                "player_not_alive",
                "only an alive player with remaining oxygen has a life-support transition",
            ));
        }
        let environment = self.environment_at(self.player.position);
        let survival = &content::manifest().survival;
        let per_second_delta = if !self.player.helmet_closed && environment.breathable {
            survival.open_breathable_delta_milli_per_second
        } else if !self.player.helmet_closed {
            survival.open_vacuum_delta_milli_per_second
        } else if environment.breathable {
            survival.sealed_breathable_delta_milli_per_second
        } else {
            survival.sealed_vacuum_delta_milli_per_second
        };
        Ok(u16::try_from(
            (i32::from(self.player.suit_oxygen_milli) + i32::from(per_second_delta))
                .clamp(0, i32::from(survival.suit_oxygen_capacity_milli)),
        )
        .expect("clamped suit oxygen always fits u16"))
    }

    fn life_support_payload_after_one_second(&self) -> Result<Option<EventPayload>, IntentError> {
        let previous_oxygen_milli = self.player.suit_oxygen_milli;
        let new_oxygen_milli = self.next_suit_oxygen_after_one_second()?;
        if new_oxygen_milli == previous_oxygen_milli {
            return Ok(None);
        }
        if new_oxygen_milli == 0 {
            return self.oxygen_incapacitation_payload().map(Some);
        }
        Ok(Some(EventPayload::SuitOxygenChanged {
            previous_oxygen_milli,
            new_oxygen_milli,
        }))
    }

    fn oxygen_incapacitation_payload(&self) -> Result<EventPayload, IntentError> {
        if !matches!(self.player.life_state, PlayerLifeState::Alive)
            || self.player.suit_oxygen_milli == 0
        {
            return Err(IntentError::rejected(
                "player_not_alive",
                "only an alive player with remaining oxygen can become incapacitated",
            ));
        }
        if self.next_suit_oxygen_after_one_second()? != 0 {
            return Err(IntentError::rejected(
                "oxygen_not_depleted",
                "the authoritative one-second life-support transition does not reach zero",
            ));
        }
        let event_sequence = self.event_sequence + 1;
        let death_id = format!("death-{}-{event_sequence}", self.player.player_id);
        let inventory = self.inventory(&self.player.inventory_id)?;
        if inventory.domain
            != (InventoryDomain::Player {
                player_id: self.player.player_id.clone(),
            })
        {
            return Err(IntentError::rejected(
                "player_inventory_domain_invalid",
                "the player inventory does not belong to the authoritative player",
            ));
        }
        let has_carried_inventory = inventory.contents != InventoryContents::default();
        let (dropped_inventory, death_drop) = if has_carried_inventory {
            let drop_id = format!("drop-{}-{event_sequence}", self.player.player_id);
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
                    },
                    contents: inventory.contents.clone(),
                    capacity_liters: inventory.capacity_liters,
                }),
                Some(DeathDrop {
                    drop_id,
                    death_id: death_id.clone(),
                    inventory_id,
                    owner_player_id: self.player.player_id.clone(),
                    position: self.player.position,
                    created_event_sequence: event_sequence,
                    cause: PlayerDeathCause::OxygenDepleted,
                }),
            )
        } else {
            (None, None)
        };
        Ok(EventPayload::PlayerIncapacitated {
            death_id,
            cause: PlayerDeathCause::OxygenDepleted,
            position: self.player.position,
            previous_oxygen_milli: self.player.suit_oxygen_milli,
            dropped_inventory,
            death_drop,
        })
    }

    fn player_respawn_payload(&self) -> Result<EventPayload, IntentError> {
        let PlayerLifeState::Incapacitated { death_id, .. } = &self.player.life_state else {
            return Err(IntentError::rejected(
                "player_already_alive",
                "the player is already alive",
            ));
        };
        let survival = &content::manifest().survival;
        if self.inventory(&self.player.inventory_id)?.contents != InventoryContents::default() {
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
            position,
            suit_oxygen_milli: survival.respawn_oxygen_milli,
            helmet_closed: survival.respawn_helmet_closed,
            jetpack_enabled: survival.respawn_jetpack_enabled,
            magnetic_boots_enabled: false,
        })
    }

    fn proof_recovery_position_is_clear(&self, position: Vec3) -> bool {
        let planet_distance = Vec3::new(
            position.x - PLANET_CENTER.x,
            position.y - PLANET_CENTER.y,
            position.z - PLANET_CENTER.z,
        )
        .magnitude();
        planet_distance
            >= PLANET_SURFACE_RADIUS_M + content::manifest().character.collision_radius_m + 0.001
            && !self.player_movement_hits_voxel(position, position)
            && !self.player_movement_hits_grid(position, position)
    }

    pub fn prepare_client_event(
        &self,
        message: &ClientMessage,
    ) -> Result<CanonicalEvent, IntentError> {
        let operation_id = message.operation_id().ok_or_else(|| {
            IntentError::rejected("not_a_mutating_intent", "message has no operation ID")
        })?;
        if operation_id.trim().is_empty() || operation_id.len() > 128 {
            return Err(IntentError::rejected(
                "invalid_operation_id",
                "operation ID must contain between 1 and 128 characters",
            ));
        }
        if !matches!(self.player.life_state, PlayerLifeState::Alive)
            && !matches!(message, ClientMessage::RespawnPlayer { .. })
        {
            return Err(IntentError::rejected(
                "player_incapacitated",
                "life support has failed; request recovery before performing work",
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
                if *movement_epoch != self.player.movement_epoch {
                    return Err(IntentError::rejected(
                        "movement_epoch_stale",
                        "character control does not match the current movement epoch",
                    ));
                }
                if *input_sequence <= self.player.last_received_input_sequence {
                    return Err(IntentError::rejected(
                        "movement_input_out_of_order",
                        "character control sequence must advance monotonically",
                    ));
                }
                let lease_queue_limit =
                    usize::try_from(content::manifest().character.control_lease_ticks)
                        .unwrap_or(usize::MAX)
                        .min(MAX_PENDING_PLAYER_CONTROL_FRAMES);
                if self.player.pending_control_frames.len() >= lease_queue_limit {
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
                if self.player.helmet_closed == *helmet_closed
                    && self.player.jetpack_enabled == *jetpack_enabled
                    && self.player.locomotion.magnetic_boots_enabled == *magnetic_boots_enabled
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
            ClientMessage::RespawnPlayer { .. } => self.player_respawn_payload()?,
            ClientMessage::MineVoxel { coordinate, .. } => {
                let material = self.voxels.material(*coordinate).ok_or_else(|| {
                    IntentError::rejected("voxel_missing", "target voxel is already empty")
                })?;
                let voxel_position = Vec3::new(
                    f64::from(coordinate.x),
                    f64::from(coordinate.y),
                    f64::from(coordinate.z),
                );
                if self.player.position.squared_distance(voxel_position)
                    > MINING_RANGE * MINING_RANGE
                {
                    return Err(IntentError::rejected(
                        "voxel_out_of_range",
                        "target voxel is beyond the mining tool range",
                    ));
                }
                let ore_yield = content::voxel(material).ore_yield;
                if !self
                    .inventory(&self.player.inventory_id)?
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
                    inventory_id: self.player.inventory_id.clone(),
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
                self.ensure_inventory_functional(inventory_id)?;
                let inventory = self.inventory(inventory_id)?;
                if inventory.contents.ore < ore_required {
                    return Err(IntentError::rejected(
                        "insufficient_ore",
                        format!("refining requires {ore_required} ore"),
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
                let refined_output = quantity.saturating_mul(
                    content::manifest()
                        .recipes
                        .component_crafting
                        .component_output,
                );
                let used_after = inventory
                    .used_liters()
                    .saturating_sub(refined_required.saturating_mul(
                        crate::model::resource_unit_volume_liters(ResourceKind::RefinedMaterial),
                    ))
                    .saturating_add(refined_output.saturating_mul(
                        crate::model::resource_unit_volume_liters(ResourceKind::Component),
                    ));
                if used_after > inventory.capacity_liters {
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
                self.ensure_hand_tool_range(grid, *coordinate, "block_out_of_range")?;
                if self.player_intersects_grid_coordinate(grid, *coordinate) {
                    return Err(IntentError::rejected(
                        "block_intersects_player",
                        "a block frame cannot be created around the living player collider",
                    ));
                }
                let player_inventory = self.inventory(PLAYER_INVENTORY_ID)?;
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
                    block,
                }
            }
            ClientMessage::WeldBlock {
                grid_id, block_id, ..
            } => {
                let grid = self.grid(grid_id)?;
                let block = grid.blocks.get(block_id).ok_or_else(|| {
                    IntentError::rejected("block_missing", "weld target does not exist")
                })?;
                self.ensure_hand_tool_range(grid, block.coordinate, "block_out_of_range")?;
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
                }
            }
            ClientMessage::DamageBlock {
                grid_id, block_id, ..
            } => {
                let grid = self.grid(grid_id)?;
                let block = grid.blocks.get(block_id).ok_or_else(|| {
                    IntentError::rejected(
                        "block_missing",
                        "target block does not exist on the grid",
                    )
                })?;
                self.ensure_hand_tool_range(grid, block.coordinate, "block_out_of_range")?;
                EventPayload::BlockDamaged {
                    grid_id: grid_id.clone(),
                    block_id: block_id.clone(),
                    damage: 35,
                }
            }
            ClientMessage::Hello { .. } | ClientMessage::RequestSnapshot => {
                return Err(IntentError::rejected(
                    "not_a_mutating_intent",
                    "message is handled by the network service",
                ));
            }
        };

        Ok(self.new_event(
            "player-local",
            "human",
            Some(operation_id.to_owned()),
            payload,
        ))
    }

    pub fn prepare_system_event(&self, payload: EventPayload) -> CanonicalEvent {
        self.new_event("simulation-worker", "system", None, payload)
    }

    fn new_event(
        &self,
        actor_profile_id: &str,
        actor_type: &str,
        operation_id: Option<String>,
        payload: EventPayload,
    ) -> CanonicalEvent {
        CanonicalEvent::new(
            self.event_sequence + 1,
            self.content_manifest_version.clone(),
            self.universe_id.clone(),
            self.cell_id.clone(),
            self.fencing_token,
            actor_profile_id,
            actor_type,
            operation_id,
            self.last_event_hash.clone(),
            payload,
        )
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
        if let Some(operation_id) = &event.operation_id
            && self.processed_operations.contains_key(operation_id)
        {
            return Err(IntentError::rejected(
                "replay_operation_duplicate",
                "event operation ID was already committed",
            ));
        }
        match &event.payload {
            EventPayload::PlayerControlSet { .. }
                if event.actor_profile_id != self.player.player_id
                    || event.actor_type != "human"
                    || event.operation_id.as_deref().is_none_or(str::is_empty) =>
            {
                return Err(IntentError::rejected(
                    "replay_player_control_envelope_invalid",
                    "character control requires the authoritative player actor and an operation ID",
                ));
            }
            EventPayload::PhysicsStepCommitted { .. }
                if event.actor_profile_id != "simulation-worker"
                    || event.actor_type != "system"
                    || event.operation_id.is_some() =>
            {
                return Err(IntentError::rejected(
                    "replay_physics_envelope_invalid",
                    "physics outcomes require the system actor and no operation ID",
                ));
            }
            EventPayload::SuitOxygenChanged { .. } | EventPayload::PlayerIncapacitated { .. }
                if event.actor_profile_id != "simulation-worker"
                    || event.actor_type != "system"
                    || event.operation_id.is_some() =>
            {
                return Err(IntentError::rejected(
                    "replay_lifecycle_envelope_invalid",
                    "automatic life-support events require the system actor and no operation ID",
                ));
            }
            EventPayload::PlayerRespawned { .. }
                if event.actor_profile_id != "player-local"
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
        if !matches!(self.player.life_state, PlayerLifeState::Alive)
            && !matches!(
                &event.payload,
                EventPayload::PlayerRespawned { .. } | EventPayload::PhysicsStepCommitted { .. }
            )
        {
            return Err(IntentError::rejected(
                "replay_player_incapacitated",
                "incapacitated players cannot commit gameplay events before recovery",
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
                ensure_bounded_control(*linear_input, "replayed character linear control")?;
                ensure_bounded_control(*angular_input, "replayed character angular control")?;
                if *movement_epoch != self.player.movement_epoch
                    || *input_sequence <= self.player.last_received_input_sequence
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
                if self.player.pending_control_frames.len() >= lease_queue_limit {
                    return Err(IntentError::rejected(
                        "replay_player_control_backpressure_invalid",
                        "character control event exceeds the canonical pending-frame bound",
                    ));
                }
                self.player
                    .pending_control_frames
                    .push_back(PlayerControlFrame {
                        input_sequence: *input_sequence,
                        linear_input: *linear_input,
                        angular_input: *angular_input,
                        boost: *boost,
                        dampeners: *dampeners,
                        jump: *jump,
                        expires_at_simulation_tick: *expires_at_simulation_tick,
                    });
                self.player.last_received_input_sequence = *input_sequence;
            }
            EventPayload::SuitModeChanged {
                helmet_closed,
                jetpack_enabled,
                magnetic_boots_enabled,
            } => {
                self.player.helmet_closed = *helmet_closed;
                self.player.jetpack_enabled = *jetpack_enabled;
                self.player.locomotion.magnetic_boots_enabled = *magnetic_boots_enabled;
                if *jetpack_enabled {
                    self.player.locomotion.kind = LocomotionKind::Eva;
                    self.player.locomotion.support = None;
                } else if matches!(self.player.locomotion.kind, LocomotionKind::Eva) {
                    self.player.locomotion.kind = LocomotionKind::Airborne;
                }
            }
            EventPayload::SuitOxygenChanged {
                new_oxygen_milli, ..
            } => {
                let expected = self.life_support_payload_after_one_second()?;
                if expected.as_ref() != Some(&event.payload) {
                    return Err(IntentError::rejected(
                        "replay_suit_oxygen_invalid",
                        "life-support event is not the exact authoritative one-second outcome",
                    ));
                }
                self.player.suit_oxygen_milli = *new_oxygen_milli;
            }
            EventPayload::PlayerIncapacitated { .. } => {
                let expected = self.oxygen_incapacitation_payload()?;
                if expected != event.payload {
                    return Err(IntentError::rejected(
                        "replay_player_incapacitation_invalid",
                        "incapacitation does not match authoritative life support or inventory",
                    ));
                }
                let EventPayload::PlayerIncapacitated {
                    death_id,
                    cause,
                    dropped_inventory,
                    death_drop,
                    ..
                } = expected
                else {
                    unreachable!("incapacitation preparation returns incapacitation payload");
                };
                self.inventory_mut(&self.player.inventory_id.clone())?
                    .contents = InventoryContents::default();
                if let (Some(inventory), Some(drop)) = (dropped_inventory, death_drop) {
                    self.inventories
                        .insert(inventory.inventory_id.clone(), inventory);
                    self.death_drops.insert(drop.drop_id.clone(), drop);
                }
                self.player.suit_oxygen_milli = 0;
                self.player.jetpack_enabled = false;
                self.player.linear_velocity = Vec3::ZERO;
                self.player.angular_velocity = Vec3::ZERO;
                self.player.surface_contact = false;
                self.player.locomotion = reset_locomotion(
                    self.player.position,
                    LocomotionKind::Airborne,
                    false,
                    self.simulation_tick,
                );
                self.player.control_linear_input = Vec3::ZERO;
                self.player.control_angular_input = Vec3::ZERO;
                self.player.boost = false;
                self.player.dampeners = true;
                self.player.jump = false;
                self.player.control_expires_at_simulation_tick = self.simulation_tick;
                self.player.pending_control_frames.clear();
                self.player.life_state = PlayerLifeState::Incapacitated { death_id, cause };
                self.active_contact_pairs
                    .retain(|pair| !contact_key_involves_player(pair));
                for grid in self.grids.values_mut() {
                    grid.control_linear_input = Vec3::ZERO;
                    grid.control_angular_input = Vec3::ZERO;
                    grid.dampeners = true;
                }
            }
            EventPayload::PlayerRespawned {
                position,
                suit_oxygen_milli,
                helmet_closed,
                jetpack_enabled,
                magnetic_boots_enabled,
                ..
            } => {
                let expected = self.player_respawn_payload()?;
                if expected != event.payload {
                    return Err(IntentError::rejected(
                        "replay_player_respawn_invalid",
                        "respawn does not match the server-selected recovery outcome",
                    ));
                }
                self.player.position = *position;
                self.player.orientation = Quat::IDENTITY;
                self.player.linear_velocity = Vec3::ZERO;
                self.player.angular_velocity = Vec3::ZERO;
                self.player.surface_contact = false;
                self.player.locomotion = reset_locomotion(
                    *position,
                    if *jetpack_enabled {
                        LocomotionKind::Eva
                    } else {
                        LocomotionKind::Airborne
                    },
                    *magnetic_boots_enabled,
                    self.simulation_tick,
                );
                self.player.movement_epoch = self.player.movement_epoch.saturating_add(1);
                self.player.last_received_input_sequence = 0;
                self.player.last_processed_input_sequence = 0;
                self.player.pending_control_frames.clear();
                self.player.control_linear_input = Vec3::ZERO;
                self.player.control_angular_input = Vec3::ZERO;
                self.player.boost = false;
                self.player.dampeners = true;
                self.player.jump = false;
                self.player.control_expires_at_simulation_tick = self.simulation_tick;
                self.player.suit_oxygen_milli = *suit_oxygen_milli;
                self.player.helmet_closed = *helmet_closed;
                self.player.jetpack_enabled = *jetpack_enabled;
                self.player.life_state = PlayerLifeState::Alive;
                self.active_contact_pairs
                    .retain(|pair| !contact_key_involves_player(pair));
            }
            EventPayload::VoxelMined {
                coordinate,
                material,
                ore_yield,
                inventory_id,
            } => {
                let removed = self.voxels.remove(*coordinate).ok_or_else(|| {
                    IntentError::rejected("replay_voxel_missing", "event target voxel is missing")
                })?;
                if removed != *material {
                    return Err(IntentError::rejected(
                        "replay_material_mismatch",
                        "event material does not match voxel material",
                    ));
                }
                self.inventory_mut(inventory_id)?.contents.ore += ore_yield;
                self.ledger.mined_ore += ore_yield;
                let body_id =
                    voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(*coordinate));
                let collider_id = voxel_collision_collider_id(*coordinate);
                self.active_contact_pairs.retain(|pair| {
                    !((pair.body_a == body_id && pair.collider_a == collider_id)
                        || (pair.body_b == body_id && pair.collider_b == collider_id))
                });
            }
            EventPayload::OreRefined {
                inventory_id,
                batches,
            } => {
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
                self.ensure_inventory_functional(inventory_id)?;
                if self.inventory(inventory_id)?.contents.ore < ore_required {
                    return Err(IntentError::rejected(
                        "replay_refining_inventory_invalid",
                        "refining event exceeds the authoritative ore inventory",
                    ));
                }
                let inventory = self.inventory_mut(inventory_id)?;
                inventory.contents.ore -= ore_required;
                inventory.contents.refined_material += batches * recipe.refined_output;
                self.ledger.refine_batches += batches;
            }
            EventPayload::ComponentCrafted {
                inventory_id,
                quantity,
            } => {
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
                self.ensure_inventory_functional(inventory_id)?;
                if self.inventory(inventory_id)?.contents.refined_material < refined_required {
                    return Err(IntentError::rejected(
                        "replay_crafting_inventory_invalid",
                        "crafting event exceeds the authoritative refined inventory",
                    ));
                }
                let inventory = self.inventory_mut(inventory_id)?;
                inventory.contents.refined_material -= refined_required;
                inventory.contents.components += quantity * recipe.component_output;
                self.ledger.crafted_components += quantity;
            }
            EventPayload::InventoryTransferred {
                source_inventory_id,
                destination_inventory_id,
                resource,
                quantity,
            } => {
                if source_inventory_id == destination_inventory_id || *quantity == 0 {
                    return Err(IntentError::rejected(
                        "replay_inventory_transfer_invalid",
                        "inventory transfer must use distinct inventories and a positive quantity",
                    ));
                }
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
                mutate_resource(
                    &mut self.inventory_mut(source_inventory_id)?.contents,
                    *resource,
                    |amount| *amount -= quantity,
                );
                mutate_resource(
                    &mut self.inventory_mut(destination_inventory_id)?.contents,
                    *resource,
                    |amount| *amount += quantity,
                );
            }
            EventPayload::BlockBuilt { grid_id, block } => {
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
                self.ensure_hand_tool_range(
                    grid,
                    block.coordinate,
                    "replay_construction_out_of_range",
                )?;
                if self.player_intersects_grid_coordinate(grid, block.coordinate) {
                    return Err(IntentError::rejected(
                        "replay_construction_intersects_player",
                        "placed frame intersects the authoritative player collider",
                    ));
                }
                if self.inventory(PLAYER_INVENTORY_ID)?.contents.components < block.component_cost {
                    return Err(IntentError::rejected(
                        "replay_construction_components_invalid",
                        "placed frame exceeds the authoritative component inventory",
                    ));
                }
                self.inventory_mut(PLAYER_INVENTORY_ID)?.contents.components -=
                    block.component_cost;
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
                let grid = self.grid_mut(grid_id)?;
                grid.control_linear_input = *linear_input;
                grid.control_angular_input = *angular_input;
                grid.dampeners = *dampeners;
            }
            EventPayload::GridAnchorSet { grid_id, anchored } => {
                let grid = self.grid_mut(grid_id)?;
                grid.anchored = *anchored;
                if *anchored {
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
            } => self.apply_damage(grid_id, block_id, *damage, event.event_sequence)?,
            EventPayload::PhysicsStepCommitted {
                fixed_step_hz,
                step_count,
                remaining_step_phase,
                bodies,
                player,
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
                let player_alive = matches!(self.player.life_state, PlayerLifeState::Alive);
                if player.is_some() != player_alive {
                    return Err(IntentError::rejected(
                        "replay_player_physics_presence_invalid",
                        "physics outcome must contain the player exactly while alive",
                    ));
                }
                let mut scheduled_player = self.player.clone();
                if player_alive {
                    for substep_index in 0..u64::from(*step_count) {
                        advance_player_control_for_substep(
                            &mut scheduled_player,
                            self.simulation_tick.saturating_add(substep_index),
                        );
                    }
                }
                if let Some(player) = player {
                    if player.player_id != self.player.player_id {
                        return Err(IntentError::rejected(
                            "replay_player_physics_identity_invalid",
                            "physics outcome identifies the wrong player",
                        ));
                    }
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
                        &self.player,
                        player,
                        *step_count,
                        &physics_limits,
                    )?;
                    let resulting_tick =
                        self.simulation_tick.saturating_add(u64::from(*step_count));
                    validate_player_locomotion_outcome(
                        self,
                        &scheduled_player,
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
                        && (body.position != grid.position
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
                    if contact_key_involves_player(&key) {
                        let player = player
                            .as_ref()
                            .expect("validated living player outcome exists for a player contact");
                        if !self.player_contact_is_spatially_plausible(
                            contact,
                            player,
                            bodies,
                            *step_count,
                            &physics_limits,
                        ) {
                            return Err(IntentError::rejected(
                                "replay_player_contact_spatially_invalid",
                                "player contact must lie on the plausible swept player and counterpart geometry",
                            ));
                        }
                    }
                    contacts_by_substep[usize::from(contact.substep_index)].push((key, contact));
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
                if let Some(player) = player {
                    let expected_surface_contact = active.iter().any(contact_key_involves_player)
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
                    grid.position = body.position;
                    grid.orientation = body.orientation;
                    grid.linear_velocity = body.linear_velocity;
                    grid.angular_velocity = body.angular_velocity;
                }
                if let Some(player) = player {
                    self.player.position = player.position;
                    self.player.orientation = player.orientation;
                    self.player.linear_velocity = player.linear_velocity;
                    self.player.angular_velocity = player.angular_velocity;
                    self.player.surface_contact = player.surface_contact;
                    self.player.locomotion = player.locomotion.clone();
                    self.player.last_processed_input_sequence =
                        scheduled_player.last_processed_input_sequence;
                    self.player.pending_control_frames = scheduled_player.pending_control_frames;
                    self.player.control_linear_input = player.control_linear_input;
                    self.player.control_angular_input = player.control_angular_input;
                    self.player.boost = player.boost;
                    self.player.dampeners = player.dampeners;
                    self.player.jump = player.jump;
                    self.player.control_expires_at_simulation_tick =
                        player.control_expires_at_simulation_tick;
                }
                self.active_contact_pairs = active;
                self.physics_step_phase = u64::from(*remaining_step_phase);
                self.simulation_tick = self.simulation_tick.saturating_add(u64::from(*step_count));
            }
        }

        self.player.experience = self
            .player
            .experience
            .saturating_add(event.payload.experience_reward());
        match &event.payload {
            EventPayload::VoxelMined { .. } => self.player.career.voxels_mined += 1,
            EventPayload::OreRefined { batches, .. } => {
                self.player.career.refining_batches += batches;
            }
            EventPayload::ComponentCrafted { quantity, .. } => {
                self.player.career.components_crafted += quantity;
            }
            EventPayload::BlockWelded {
                completed_construction: true,
                ..
            } => self.player.career.blocks_built += 1,
            EventPayload::GridAnchorSet { anchored: true, .. } => {
                self.player.career.anchors_engaged += 1;
            }
            EventPayload::PlayerControlSet { .. }
            | EventPayload::SuitModeChanged { .. }
            | EventPayload::SuitOxygenChanged { .. }
            | EventPayload::PlayerIncapacitated { .. }
            | EventPayload::PlayerRespawned { .. }
            | EventPayload::InventoryTransferred { .. }
            | EventPayload::BlockBuilt { .. }
            | EventPayload::BlockWelded { .. }
            | EventPayload::GridControlSet { .. }
            | EventPayload::GridAnchorSet {
                anchored: false, ..
            }
            | EventPayload::BlockDamaged { .. }
            | EventPayload::PhysicsStepCommitted { .. } => {}
        }

        self.event_sequence = event.event_sequence;
        self.last_event_hash.clone_from(&event.event_hash);
        if let Some(operation_id) = &event.operation_id {
            let (code, message) = event.payload.receipt();
            self.processed_operations.insert(
                operation_id.clone(),
                IntentReceipt {
                    operation_id: operation_id.clone(),
                    event_sequence: event.event_sequence,
                    code: code.into(),
                    message,
                },
            );
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
            };
        }
        self.split_disconnected_grid(grid_id, event_sequence)?;
        Ok(())
    }

    fn split_disconnected_grid(
        &mut self,
        grid_id: &str,
        event_sequence: u64,
    ) -> Result<(), IntentError> {
        let original = self.grids.remove(grid_id).ok_or_else(|| {
            IntentError::rejected("replay_grid_missing", "grid split target is missing")
        })?;
        if original.blocks.is_empty() {
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

        for (index, component) in components.into_iter().enumerate() {
            let new_grid_id = if index == primary_index {
                original.grid_id.clone()
            } else {
                format!("{}-split-{event_sequence}-{index}", original.grid_id)
            };
            let blocks = component
                .iter()
                .map(|coordinate| {
                    let block_id = &by_coordinate[coordinate];
                    (block_id.clone(), original.blocks[block_id].clone())
                })
                .collect();
            let mut grid = Grid {
                grid_id: new_grid_id.clone(),
                position: original.position,
                orientation: original.orientation,
                linear_velocity: original.linear_velocity,
                angular_velocity: original.angular_velocity,
                control_linear_input: original.control_linear_input,
                control_angular_input: original.control_angular_input,
                dampeners: original.dampeners,
                anchored: original.anchored,
                blocks,
            };
            grid.anchored = grid.anchored && grid.anchor_touches(&self.voxels);
            self.grids.insert(new_grid_id, grid);
        }
        Ok(())
    }

    fn physics_collider_exists(&self, body_id: &str, collider_id: &str) -> bool {
        if body_id == PLAYER_BODY_ID {
            return matches!(self.player.life_state, PlayerLifeState::Alive)
                && collider_id == PLAYER_COLLIDER_ID;
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

    fn grid_mut(&mut self, grid_id: &str) -> Result<&mut Grid, IntentError> {
        self.grids.get_mut(grid_id).ok_or_else(|| {
            IntentError::rejected("grid_missing", format!("grid {grid_id} does not exist"))
        })
    }

    fn ensure_hand_tool_range(
        &self,
        grid: &Grid,
        coordinate: verse_protocol::IVec3,
        code: &str,
    ) -> Result<(), IntentError> {
        let world = grid.world_coordinate(coordinate);
        let position = Vec3::new(f64::from(world.x), f64::from(world.y), f64::from(world.z));
        if self.player.position.squared_distance(position) > HAND_TOOL_RANGE * HAND_TOOL_RANGE {
            return Err(IntentError::rejected(
                code,
                "the targeted block coordinate is beyond the hand-tool range",
            ));
        }
        Ok(())
    }

    fn player_intersects_voxel(&self, position: Vec3) -> bool {
        let extent = content::manifest().character.collision_radius_m + 0.5;
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
                        && sphere_intersects_unit_cube(position, coordinate)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn player_movement_hits_voxel(&self, start: Vec3, end: Vec3) -> bool {
        movement_samples(start, end)
            .into_iter()
            .any(|position| self.player_intersects_voxel(position))
    }

    fn player_intersects_grid(&self, position: Vec3) -> bool {
        self.grids.values().any(|grid| {
            let relative = Vec3::new(
                position.x - grid.position.x,
                position.y - grid.position.y,
                position.z - grid.position.z,
            );
            let local_player = grid.orientation.conjugate().rotate(relative);
            grid.blocks
                .values()
                .any(|block| sphere_intersects_unit_cube(local_player, block.coordinate))
        })
    }

    fn player_intersects_grid_coordinate(&self, grid: &Grid, coordinate: IVec3) -> bool {
        let relative = self.player.position - grid.position;
        let local_player = grid.orientation.conjugate().rotate(relative);
        sphere_intersects_unit_cube(local_player, coordinate)
    }

    fn player_movement_hits_grid(&self, start: Vec3, end: Vec3) -> bool {
        movement_samples(start, end)
            .into_iter()
            .any(|position| self.player_intersects_grid(position))
    }

    fn player_contact_is_spatially_plausible(
        &self,
        contact: &PhysicsContactOutcome,
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
        let surface_slack = 0.5 * contact.penetration_m.max(PHYSICS_SPECULATIVE_DISTANCE_M)
            + PHYSICS_CONTACT_POINT_SLOP_M
            + REPLAY_QUANTIZATION_SLOP;
        let capsule_half_height = character_capsule_half_height();
        if point_capsule_axis_distance(
            contact.point,
            self.player.position,
            self.player.orientation,
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

        let (other_body, other_collider) = if contact.body_a_id == PLAYER_BODY_ID {
            (&contact.body_b_id, &contact.collider_b_id)
        } else if contact.body_b_id == PLAYER_BODY_ID {
            (&contact.body_a_id, &contact.collider_a_id)
        } else {
            return false;
        };
        if other_body == PLAYER_BODY_ID {
            return false;
        }
        if other_body == PLANET_BODY_ID {
            return other_collider == PLANET_COLLIDER_ID
                && contact.penetration_m <= PLAYER_PLANET_PENETRATION_LIMIT_M
                && ((contact.point - PLANET_CENTER).magnitude() - PLANET_SURFACE_RADIUS_M).abs()
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
    let planet_distance = (outcome.position - PLANET_CENTER).magnitude();
    if planet_distance
        < PLANET_SURFACE_RADIUS_M + radial_capsule_extent - PLAYER_PLANET_PENETRATION_LIMIT_M
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
        if support.body_id == PLAYER_BODY_ID
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
            | EventPayload::GridControlSet { .. }
            | EventPayload::PhysicsStepCommitted { .. }
    )
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
        PhysicsPose::new(to_physics_vec3(PLANET_CENTER), PhysicsQuat::IDENTITY),
        Vec::new(),
    );
    planet.sphere_colliders.push(SphereColliderSpec {
        collider_id: PLANET_COLLIDER_ID.into(),
        local_pose: PhysicsPose::IDENTITY,
        radius: PLANET_SURFACE_RADIUS_M as f32,
        density_kg_per_m3: 5_500.0,
    });
    planet.friction = physics.friction;
    planet.restitution = physics.restitution;
    bodies.push(planet);
    if matches!(state.player.life_state, PlayerLifeState::Alive) {
        let character = &content::manifest().character;
        let radius = character.collision_radius_m;
        let half_height_of_cylinder = (character.standing_height_m - 2.0 * radius) * 0.5;
        let volume = std::f64::consts::PI * radius.powi(2) * (2.0 * half_height_of_cylinder)
            + 4.0 / 3.0 * std::f64::consts::PI * radius.powi(3);
        let mut player = BodySpec::dynamic(
            PLAYER_BODY_ID,
            PhysicsPose::new(
                to_physics_vec3(state.player.position),
                to_physics_quat(state.player.orientation),
            ),
            Vec::new(),
        );
        player.capsule_colliders.push(CapsuleColliderSpec {
            collider_id: PLAYER_COLLIDER_ID.into(),
            local_pose: PhysicsPose::IDENTITY,
            radius: radius as f32,
            half_height_of_cylinder: half_height_of_cylinder as f32,
            density_kg_per_m3: (character.mass_kg / volume) as f32,
        });
        player.linear_velocity = to_physics_vec3(state.player.linear_velocity);
        player.angular_velocity = to_physics_vec3(state.player.angular_velocity);
        player.friction = physics.friction;
        player.restitution = physics.restitution;
        player.allow_sleeping = false;
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
    let Some(body) = body_states
        .iter()
        .find(|body| body.body_id == PLAYER_BODY_ID)
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
        ignore_body_id: Some(PLAYER_BODY_ID.into()),
    })?;
    let prior_had_support = player.locomotion.support.is_some();
    let mut accepted_support = None;
    let mut support_kind = LocomotionKind::Airborne;
    let mut support_up = gravity_up;
    if let Some(hit) = hit {
        let surface_normal = normalized_or(from_physics_vec3(hit.surface_normal), probe_up);
        let slope_cosine = (character.walkable_slope_degrees.to_radians()).cos();
        let slope_dot = dot(surface_normal, gravity_up);
        let gravity_walkable = environment.gravity_m_s2 > 0.05 && slope_dot >= slope_cosine;
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
                to_physics_vec3(PLANET_CENTER),
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
) -> Vec<BodyControl> {
    let physics = &content::manifest().physics;
    let mut controls = state
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
                grid.orientation.rotate(grid.control_linear_input) * physics.control_force_newtons
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
        .collect::<Vec<_>>();

    if matches!(player.life_state, PlayerLifeState::Alive) {
        let character = &content::manifest().character;
        let body_state = body_states
            .iter()
            .find(|body| body.body_id == PLAYER_BODY_ID);
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
                let gravity_into_support = dot(gravity, up).min(0.0);
                acceleration = acceleration + gravity - up * gravity_into_support;
            }
        }
        if let Some(jump) = player_jump {
            let delta_seconds = f64::from(physics.fixed_delta_seconds);
            let relative_normal_speed = dot(linear_velocity - jump.support_velocity, jump.up);
            acceleration = acceleration
                + jump.up
                    * ((character.jump_speed_m_s - relative_normal_speed) / delta_seconds).max(0.0);
        }
        let world_angular_input = if player.jetpack_enabled {
            orientation.rotate(angular_input)
        } else {
            normalized_or(player.locomotion.up, radial_up(position)) * angular_input.y
        };
        let angular_acceleration = if dampeners {
            let target = world_angular_input * character.maximum_angular_speed_radians_per_second;
            ((target - angular_velocity) * (1.0 / f64::from(physics.fixed_delta_seconds))).clamped(
                if world_angular_input.magnitude() > CONTROL_INPUT_EPSILON {
                    character.angular_acceleration_radians_per_second_squared
                } else {
                    character.angular_dampener_acceleration_radians_per_second_squared
                },
            )
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
            body_id: PLAYER_BODY_ID.into(),
            force_newtons: to_physics_vec3(acceleration * character.mass_kg),
            torque_newton_meters: to_physics_vec3(orientation.rotate(local_torque)),
        });
    }
    controls
}

fn physics_body_outcome(body: &verse_physics::BodyState) -> PhysicsBodyOutcome {
    let limits = physics_scene_config();
    PhysicsBodyOutcome {
        grid_id: body.body_id.clone(),
        position: from_physics_vec3(body.pose.position),
        orientation: from_physics_quat(body.pose.rotation),
        linear_velocity: from_physics_vec3(body.linear_velocity)
            .clamped(f64::from(limits.max_linear_velocity_mps)),
        angular_velocity: from_physics_vec3(body.angular_velocity)
            .clamped(f64::from(limits.max_angular_velocity_radians_per_second)),
    }
}

fn player_physics_outcome(
    player: &Player,
    body: &verse_physics::BodyState,
    surface_contact: bool,
    resulting_simulation_tick: u64,
) -> PlayerPhysicsOutcome {
    let limits = physics_scene_config();
    let lease_active = resulting_simulation_tick < player.control_expires_at_simulation_tick;
    PlayerPhysicsOutcome {
        player_id: player.player_id.clone(),
        position: from_physics_vec3(body.pose.position),
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
    }
}

fn physics_contact_outcome(
    state: &WorldState,
    contact: &verse_physics::ContactRecord,
    substep_index: u8,
    phase: PhysicsContactPhase,
) -> PhysicsContactOutcome {
    PhysicsContactOutcome {
        substep_index,
        body_a_id: contact.body_a_id.clone(),
        collider_a_id: contact.collider_a_id.clone(),
        body_b_id: contact.body_b_id.clone(),
        collider_b_id: contact.collider_b_id.clone(),
        point: from_physics_vec3(contact.point),
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
    }
}

fn contact_pair_key(contact: &verse_physics::ContactRecord) -> ContactPairKey {
    ContactPairKey {
        body_a: contact.body_a_id.clone(),
        collider_a: contact.collider_a_id.clone(),
        body_b: contact.body_b_id.clone(),
        collider_b: contact.collider_b_id.clone(),
    }
}

fn contact_key_involves_player(contact: &ContactPairKey) -> bool {
    contact.body_a == PLAYER_BODY_ID || contact.body_b == PLAYER_BODY_ID
}

fn reduced_translational_contact_mass_grams(
    state: &WorldState,
    left_body: &str,
    right_body: &str,
) -> u64 {
    fn mass(state: &WorldState, body_id: &str) -> Option<u64> {
        if body_id == PLAYER_BODY_ID && matches!(state.player.life_state, PlayerLifeState::Alive) {
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

fn sphere_intersects_unit_cube(center: Vec3, cube: IVec3) -> bool {
    let closest_x = center
        .x
        .clamp(f64::from(cube.x) - 0.5, f64::from(cube.x) + 0.5);
    let closest_y = center
        .y
        .clamp(f64::from(cube.y) - 0.5, f64::from(cube.y) + 0.5);
    let closest_z = center
        .z
        .clamp(f64::from(cube.z) - 0.5, f64::from(cube.z) + 0.5);
    let radius = content::manifest().character.collision_radius_m;
    center.squared_distance(Vec3::new(closest_x, closest_y, closest_z)) <= radius * radius + 1.0e-9
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
    if value.magnitude() > MAX_GRID_CONTROL_INPUT + CONTROL_INPUT_EPSILON {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use proptest::prelude::*;
    use tempfile::tempdir;
    use verse_protocol::IVec3;

    use super::*;
    use crate::model::{STARTER_GRID_ID, VoxelField};
    use crate::persistence::AppendFailpoint;

    fn runtime() -> Runtime {
        Runtime::open(tempdir().expect("tempdir").keep(), 42, 5).expect("runtime opens")
    }

    fn move_player_near_grid(runtime: &mut Runtime) {
        runtime.state.player.position = Vec3::new(10.0, 1.0, 3.0);
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("test setup rebuilds player near the grid");
    }

    fn stationary_player_outcome(
        state: &WorldState,
        step_count: u8,
    ) -> Option<PlayerPhysicsOutcome> {
        matches!(state.player.life_state, PlayerLifeState::Alive).then(|| {
            let resulting_tick = state.simulation_tick + u64::from(step_count);
            let lease_active = resulting_tick < state.player.control_expires_at_simulation_tick;
            PlayerPhysicsOutcome {
                player_id: state.player.player_id.clone(),
                position: state.player.position,
                orientation: state.player.orientation,
                linear_velocity: state.player.linear_velocity,
                angular_velocity: state.player.angular_velocity,
                locomotion: state.player.locomotion.clone(),
                surface_contact: false,
                control_linear_input: if lease_active {
                    state.player.control_linear_input
                } else {
                    Vec3::ZERO
                },
                control_angular_input: if lease_active {
                    state.player.control_angular_input
                } else {
                    Vec3::ZERO
                },
                boost: state.player.boost && lease_active,
                jump: state.player.jump && lease_active,
                dampeners: state.player.dampeners || !lease_active,
                control_expires_at_simulation_tick: state.player.control_expires_at_simulation_tick,
            }
        })
    }

    fn reachable_voxel(runtime: &Runtime) -> IVec3 {
        runtime
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
                runtime.state().player.position.squared_distance(position)
                    <= MINING_RANGE * MINING_RANGE
            })
            .expect("reachable voxel exists")
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
            runtime
                .execute(&ClientMessage::WeldBlock {
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
        runtime
            .state
            .inventories
            .retain(|inventory_id, _| inventory_id == PLAYER_INVENTORY_ID);
        runtime.state.ledger.genesis_installed_components = runtime
            .state
            .grids
            .values()
            .flat_map(|grid| grid.blocks.values())
            .map(|block| block.component_cost)
            .sum();
        runtime.state.ledger.destroyed_components = 0;
        assert!(runtime.state.conservation().valid);
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("fixture physics rebuilds");
        runtime
            .persist_snapshot()
            .expect("fixture snapshot persists");
    }

    #[test]
    fn replay_rejects_an_incompatible_event_schema_even_with_a_valid_hash() {
        let runtime = runtime();
        let mut state = runtime.state().clone();
        let mut event = state.prepare_system_event(EventPayload::SuitOxygenChanged {
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
            player: stationary_player_outcome(state, 1),
            contacts: vec![PhysicsContactOutcome {
                substep_index: 0,
                body_a_id: STARTER_GRID_ID.into(),
                collider_a_id: "block-core".into(),
                body_b_id: voxel_body.clone(),
                collider_b_id: voxel_collider,
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
            player: stationary_player_outcome(&state, 1),
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
            body.position.x += 1.0;
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
            player: stationary_player_outcome(state, 1),
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
        let EventPayload::PhysicsStepCommitted { player, .. } = &mut missing else {
            unreachable!();
        };
        *player = None;
        reject(missing, "replay_player_physics_presence_invalid");

        let mut wrong_id = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut wrong_id
        else {
            unreachable!();
        };
        player.player_id.push_str("-forged");
        reject(wrong_id, "replay_player_physics_identity_invalid");

        let mut non_finite = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut non_finite
        else {
            unreachable!();
        };
        player.position.x = f64::NAN;
        reject(non_finite, "invalid_vector");

        let mut zero_rotation = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut zero_rotation
        else {
            unreachable!();
        };
        player.orientation = Quat::new(0.0, 0.0, 0.0, 0.0);
        reject(zero_rotation, "replay_player_physics_rotation_invalid");

        let mut over_speed = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut over_speed
        else {
            unreachable!();
        };
        player.linear_velocity = Vec3::new(32.001, 0.0, 0.0);
        reject(over_speed, "replay_player_physics_velocity_invalid");

        let mut teleported = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut teleported
        else {
            unreachable!();
        };
        player.position.x += 10.0;
        reject(teleported, "replay_player_physics_translation_invalid");

        let mut spun = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut spun
        else {
            unreachable!();
        };
        player.orientation = Quat::new(0.0, 0.0, 1.0, 0.0);
        reject(spun, "replay_player_physics_rotation_continuity_invalid");

        let mut impossible_contact = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            contacts,
            active_contacts_after,
            ..
        } = &mut impossible_contact
        else {
            unreachable!();
        };
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
            player: Some(player),
            contacts,
            active_contacts_after,
            ..
        } = &mut wrong_voxel_geometry
        else {
            unreachable!();
        };
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
            player: Some(player),
            contacts,
            active_contacts_after,
            ..
        } = &mut wrong_grid_geometry
        else {
            unreachable!();
        };
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
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut wrong_control
        else {
            unreachable!();
        };
        player.boost = true;
        reject(wrong_control, "replay_player_physics_control_invalid");

        let mut wrong_locomotion = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut wrong_locomotion
        else {
            unreachable!();
        };
        player.locomotion.kind = LocomotionKind::Grounded;
        reject(wrong_locomotion, "replay_player_locomotion_invalid");

        let mut wrong_jump = canonical.clone();
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut wrong_jump
        else {
            unreachable!();
        };
        player.jump = true;
        reject(wrong_jump, "replay_player_physics_control_invalid");

        let mut wrong_contact = canonical;
        let EventPayload::PhysicsStepCommitted {
            player: Some(player),
            ..
        } = &mut wrong_contact
        else {
            unreachable!();
        };
        player.surface_contact = true;
        reject(wrong_contact, "replay_player_surface_contact_invalid");
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
                position: grid.position,
                orientation: grid.orientation,
                linear_velocity: grid.linear_velocity,
                angular_velocity: grid.angular_velocity,
            })
            .collect::<Vec<_>>();
        bodies
            .iter_mut()
            .find(|body| body.grid_id == STARTER_GRID_ID)
            .expect("starter grid outcome exists")
            .position
            .x += 10.0;
        let event = state.prepare_system_event(EventPayload::PhysicsStepCommitted {
            fixed_step_hz: content::manifest().physics.fixed_step_hz,
            step_count: 1,
            remaining_step_phase: 0,
            bodies,
            player: stationary_player_outcome(state, 1),
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
        let runtime = runtime();
        let mut state = runtime.state().clone();
        let before_hash = state.state_hash();
        let max_health = state.grids[STARTER_GRID_ID].blocks["block-core"].max_health();
        let event = state.prepare_system_event(EventPayload::BlockWelded {
            grid_id: STARTER_GRID_ID.into(),
            block_id: "block-core".into(),
            previous_health: max_health,
            new_health: max_health,
            max_health,
            completed_construction: false,
        });

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
            .prepare_client_event(&ClientMessage::BuildBlock {
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
        let target = reachable_voxel(&runtime);
        let body_id = voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(target));
        let collider_id = voxel_collision_collider_id(target);
        assert!(runtime.physics.contains_collider(&body_id, &collider_id));
        let intent = ClientMessage::MineVoxel {
            operation_id: "mine-once".into(),
            coordinate: target,
        };
        let first = runtime.execute(&intent).expect("first mine accepted");
        assert_eq!(runtime.physics_chunk_replacements, 1);
        assert_eq!(runtime.physics_full_rebuilds, 0);
        assert!(!runtime.physics.contains_collider(&body_id, &collider_id));
        let hash_after_first = runtime.state().state_hash();
        let second = runtime.execute(&intent).expect("retry accepted");
        assert_eq!(first, second);
        assert_eq!(hash_after_first, runtime.state().state_hash());
        assert_eq!(runtime.physics_chunk_replacements, 1);
        assert_eq!(runtime.physics_full_rebuilds, 0);
        assert!(runtime.state().conservation().valid);
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
                state.voxels.occupied.iter().any(|other| {
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
        state.player.position = Vec3::new(
            f64::from(target.x),
            f64::from(target.y),
            f64::from(target.z),
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
            .prepare_client_event(&ClientMessage::MineVoxel {
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
            .execute(&ClientMessage::SetGridControl {
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

        runtime
            .execute(&ClientMessage::MineVoxel {
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
        runtime.state.player.position = Vec3::new(0.0, 0.0, 3.0);
        runtime.persist_snapshot().expect("player pose persists");
        assert!(runtime.state().grids["anchored-grid"].anchor_touches(&runtime.state().voxels));
        let before_hash = runtime.state().state_hash();
        let before_fingerprint = runtime.physics.body_collider_fingerprint();
        let before_journal =
            fs::read(directory.path().join("events.ndjson")).expect("journal reads");

        let result = runtime.execute(&ClientMessage::MineVoxel {
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
            runtime.physics.body_collider_fingerprint(),
            before_fingerprint
        );
        assert_eq!(runtime.physics_chunk_replacements, 0);
        assert_eq!(runtime.physics_full_rebuilds, 0);
        assert!(
            !runtime
                .state()
                .processed_operations
                .contains_key("mine-final-anchor-support")
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
            operation_id: "transfer-components".into(),
            source_inventory_id: PLAYER_INVENTORY_ID.into(),
            destination_inventory_id: cargo_id.clone(),
            resource: ResourceKind::Component,
            quantity: 4,
        };
        runtime.execute(&intent).expect("transfer accepted");
        runtime.execute(&intent).expect("retry returns receipt");
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
                .execute(&ClientMessage::TransferInventory {
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
        let result = runtime.execute(&ClientMessage::SetPlayerControl {
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
            let result = runtime.execute(&ClientMessage::SetPlayerControl {
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

        let result = runtime.execute(&ClientMessage::SetPlayerControl {
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
            .execute(&accepted_control)
            .expect("bounded in-order control is accepted");
        let retry_receipt = runtime
            .execute(&accepted_control)
            .expect("same operation retry returns its durable receipt");
        assert_eq!(retry_receipt, first_receipt);
        assert_eq!(runtime.state().event_sequence, 1);
        let result = runtime.execute(&ClientMessage::SetPlayerControl {
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

    #[test]
    fn one_frame_press_and_release_are_consumed_on_successive_fixed_steps() {
        let mut runtime = runtime();
        let initial_orientation = runtime.state().player.orientation;
        for (input_sequence, angular_input) in [(1, Vec3::new(0.0, 0.0, 1.0)), (2, Vec3::ZERO)] {
            runtime
                .execute(&ClientMessage::SetPlayerControl {
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
                        .execute(&ClientMessage::SetPlayerControl {
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
                    runtime.execute(&ClientMessage::SetPlayerControl {
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
                .execute(&ClientMessage::SetPlayerControl {
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
        let result = runtime.execute(&ClientMessage::SetPlayerControl {
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
            .execute(&ClientMessage::SetPlayerControl {
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
                .execute(&ClientMessage::SetPlayerControl {
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
            .execute(&ClientMessage::SetPlayerControl {
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
                        .execute(&ClientMessage::SetPlayerControl {
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
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("high-inertia player fixture rebuilds");
        runtime
            .execute(&ClientMessage::SetPlayerControl {
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
            .execute(&ClientMessage::SetPlayerControl {
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
            .execute(&ClientMessage::SetPlayerControl {
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
            PLANET_CENTER + Vec3::new(PLANET_SURFACE_RADIUS_M + 100.0, 0.0, 0.0),
            PLANET_CENTER + Vec3::new(0.0, PLANET_SURFACE_RADIUS_M + 100.0, 0.0),
            PLANET_CENTER + Vec3::new(0.0, 0.0, PLANET_SURFACE_RADIUS_M + 100.0),
        ] {
            let mut state = runtime().state().clone();
            state.player.position = position;
            state.player.jetpack_enabled = true;
            state.player.dampeners = false;
            state.player.control_expires_at_simulation_tick = 1;
            let controls = physics_controls(&state, &state.player, &[], 0, None);
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
            PLANET_CENTER.x,
            PLANET_CENTER.y + PLANET_SURFACE_RADIUS_M + 10.0,
            PLANET_CENTER.z,
        ));
        runtime.state.player.suit_oxygen_milli = 900;
        runtime
            .execute(&ClientMessage::SetSuitMode {
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
            PLANET_CENTER.x,
            PLANET_CENTER.y + PLANET_SURFACE_RADIUS_M + 10.0,
            PLANET_CENTER.z,
        );

        let apply = |mut state: WorldState, payload: EventPayload| {
            let event = state.prepare_system_event(payload);
            let result = state.apply_event(&event);
            (state, result)
        };

        let mut state = runtime().state().clone();
        state.player.position = vacuum;
        state.player.helmet_closed = false;
        let (_, impossible) = apply(
            state.clone(),
            EventPayload::SuitOxygenChanged {
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
                previous_oxygen_milli: 1_000,
                new_oxygen_milli: 960,
            },
        );
        accepted.expect("open-vacuum exact delta applies");
        assert_eq!(exact_vacuum.player.suit_oxygen_milli, 960);

        let mut state = runtime().state().clone();
        state.player.position = breathable;
        state.player.helmet_closed = false;
        state.player.suit_oxygen_milli = 900;
        let (_, impossible) = apply(
            state.clone(),
            EventPayload::SuitOxygenChanged {
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
                previous_oxygen_milli: 900,
                new_oxygen_milli: 925,
            },
        );
        accepted.expect("open-breathable exact delta applies");
        assert_eq!(exact_breathable.player.suit_oxygen_milli, 925);

        let mut full_oxygen = runtime().state().clone();
        full_oxygen.player.position = vacuum;
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
                operation_id: "dead-suit".into(),
                helmet_closed: false,
                jetpack_enabled: false,
                magnetic_boots_enabled: false,
            },
            ClientMessage::MineVoxel {
                operation_id: "dead-mine".into(),
                coordinate: IVec3::ZERO,
            },
            ClientMessage::RefineOre {
                operation_id: "dead-refine".into(),
                inventory_id: PLAYER_INVENTORY_ID.into(),
                batches: 1,
            },
            ClientMessage::CraftComponent {
                operation_id: "dead-craft".into(),
                inventory_id: PLAYER_INVENTORY_ID.into(),
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_id: "dead-transfer".into(),
                source_inventory_id: drop_inventory_id.clone(),
                destination_inventory_id: PLAYER_INVENTORY_ID.into(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
            ClientMessage::BuildBlock {
                operation_id: "dead-build".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(0, 1, 0),
                kind: BlockKind::Structural,
                orientation: 0,
            },
            ClientMessage::WeldBlock {
                operation_id: "dead-weld".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: "block-core".into(),
            },
            ClientMessage::SetGridControl {
                operation_id: "dead-control".into(),
                grid_id: STARTER_GRID_ID.into(),
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                dampeners: false,
            },
            ClientMessage::ToggleGridAnchor {
                operation_id: "dead-anchor".into(),
                grid_id: STARTER_GRID_ID.into(),
            },
            ClientMessage::DamageBlock {
                operation_id: "dead-damage".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: "block-core".into(),
            },
        ];
        let dead_hash = runtime.state().state_hash();
        for message in blocked_messages {
            assert!(matches!(
                runtime.execute(&message),
                Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                    if code == "player_incapacitated"
            ));
            assert_eq!(runtime.state().state_hash(), dead_hash);
        }

        let respawn = ClientMessage::RespawnPlayer {
            operation_id: "recover-once".into(),
        };
        let first = runtime.execute(&respawn).expect("recovery accepted");
        let recovered_hash = runtime.state().state_hash();
        let second = runtime.execute(&respawn).expect("recovery retry accepted");
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
            ClientMessage::RefineOre {
                operation_id: "drop-refine".into(),
                inventory_id: drop_inventory_id.clone(),
                batches: 1,
            },
            ClientMessage::CraftComponent {
                operation_id: "drop-craft".into(),
                inventory_id: drop_inventory_id.clone(),
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_id: "drop-transfer-source".into(),
                source_inventory_id: drop_inventory_id.clone(),
                destination_inventory_id: PLAYER_INVENTORY_ID.into(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_id: "drop-transfer-destination".into(),
                source_inventory_id: PLAYER_INVENTORY_ID.into(),
                destination_inventory_id: drop_inventory_id.clone(),
                resource: ResourceKind::Component,
                quantity: 1,
            },
        ];
        for intent in sealed_intents {
            assert!(matches!(
                runtime.execute(&intent),
                Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                    if code == "dropped_inventory_sealed"
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
            let event = runtime.state().new_event(
                "player-local",
                "human",
                Some(format!("forged-drop-operation-{index}")),
                payload,
            );
            let mut candidate = runtime.state().clone();
            let before = candidate.state_hash();
            assert!(matches!(
                candidate.apply_event(&event),
                Err(IntentError::Rejected { ref code, .. }) if code == "dropped_inventory_sealed"
            ));
            assert_eq!(candidate.state_hash(), before);
        }
    }

    #[test]
    fn empty_inventory_oxygen_death_does_not_create_an_empty_drop() {
        let mut runtime = runtime();
        runtime
            .execute(&ClientMessage::TransferInventory {
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
        assert_eq!(runtime.state().inventories.len(), 2);
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
        runtime.life_support_elapsed_millis = 999;

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
        let EventPayload::PlayerIncapacitated { position, .. } = &mut wrong_position else {
            unreachable!();
        };
        position.x += 0.5;
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
        assert_eq!(grid.control_linear_input, Vec3::ZERO);
        assert_eq!(grid.control_angular_input, Vec3::ZERO);
        assert!(grid.dampeners);
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
                runtime.life_support_elapsed_millis = 999;
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
                    runtime.execute(&ClientMessage::RespawnPlayer {
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
            let event = dead.new_event(
                "player-local",
                "human",
                Some("tampered-respawn".into()),
                payload,
            );
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
        let EventPayload::PlayerRespawned { position, .. } = &mut wrong_position else {
            unreachable!();
        };
        position.x += 1.0;
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
        state
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid")
            .position = primary;
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

        let event = state.new_event(
            "player-local",
            "human",
            Some("blocked-primary-recovery".into()),
            payload,
        );
        state.apply_event(&event).expect("fallback respawn applies");
        assert_eq!(state.player.position, position);
        assert_eq!(state.player.life_state, PlayerLifeState::Alive);
    }

    #[test]
    fn respawn_clearance_uses_the_content_radius_at_voxel_and_grid_edges() {
        let radius = content::manifest().character.collision_radius_m;

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
        grid.position = Vec3::new(100.0, 100.0, 100.0);
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
            "player-local",
            "human",
            Some("forged-death".into()),
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
        let respawn_event =
            state.new_event("player-local", "human", Some(operation_id.into()), respawn);
        state
            .apply_event(&respawn_event)
            .expect("human respawn with operation applies");
        let duplicate = state.new_event(
            "player-local",
            "human",
            Some(operation_id.into()),
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
            Err(IntentError::Rejected { ref code, .. }) if code == "replay_operation_duplicate"
        ));
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
                            .execute(&ClientMessage::RespawnPlayer {
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
        let result = runtime.execute(&ClientMessage::TransferInventory {
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
            operation_id: "idempotent-build".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::Structural,
            orientation: 2,
        };
        let first = runtime.execute(&intent).expect("build accepted");
        let second = runtime.execute(&intent).expect("retry accepted");
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
            .execute(&ClientMessage::BuildBlock {
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
            operation_id: "weld-once".into(),
            grid_id: STARTER_GRID_ID.into(),
            block_id: block_id.clone(),
        };
        let first_receipt = runtime.execute(&first_weld).expect("first weld accepted");
        let retry_receipt = runtime.execute(&first_weld).expect("weld retry accepted");
        assert_eq!(first_receipt, retry_receipt);
        assert_eq!(
            runtime.state().grids[STARTER_GRID_ID].blocks[&block_id].health,
            50
        );
        runtime
            .execute(&ClientMessage::WeldBlock {
                operation_id: "weld-middle".into(),
                grid_id: STARTER_GRID_ID.into(),
                block_id: block_id.clone(),
            })
            .expect("middle weld accepted");
        let final_weld = ClientMessage::WeldBlock {
            operation_id: "weld-final".into(),
            grid_id: STARTER_GRID_ID.into(),
            block_id: block_id.clone(),
        };
        let final_receipt = runtime.execute(&final_weld).expect("final weld accepted");
        let final_retry = runtime
            .execute(&final_weld)
            .expect("final weld retry accepted");
        assert_eq!(final_receipt, final_retry);
        assert!(
            runtime.state().grids[STARTER_GRID_ID]
                .block_at(IVec3::new(0, 1, 0))
                .expect("completed block exists")
                .construction_complete
        );
        assert_eq!(runtime.state().player.career.blocks_built, 1);
        assert_eq!(runtime.state().player.experience, 37);
        let sequence = runtime.state().event_sequence;
        let completed = runtime.execute(&ClientMessage::WeldBlock {
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
            .execute(&ClientMessage::BuildBlock {
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
        let sealed = runtime.execute(&ClientMessage::TransferInventory {
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
            .execute(&ClientMessage::TransferInventory {
                operation_id: "transfer-into-complete-cargo".into(),
                source_inventory_id: PLAYER_INVENTORY_ID.into(),
                destination_inventory_id: cargo_inventory_id.clone(),
                resource: ResourceKind::Component,
                quantity: 1,
            })
            .expect("completed cargo accepts inventory");
        runtime
            .execute(&ClientMessage::TransferInventory {
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

            runtime
                .execute(&ClientMessage::DamageBlock {
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
            assert_eq!(runtime.state().player.experience, 3);

            let mut weld_index = 0_u32;
            while runtime.state().grids[STARTER_GRID_ID].blocks[&block_id].health
                < original.max_health()
            {
                runtime
                    .execute(&ClientMessage::WeldBlock {
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
            assert_eq!(runtime.state().player.experience, 15);
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
        let remote = runtime.execute(&ClientMessage::BuildBlock {
            operation_id: "remote-frame".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::Structural,
            orientation: 0,
        });
        assert!(matches!(
            remote,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "block_out_of_range"
        ));
        move_player_near_grid(&mut runtime);
        let candidate = IVec3::new(0, 1, 0);
        runtime.state.player.position =
            runtime.state().grids[STARTER_GRID_ID].world_position(candidate);
        let overlap = runtime.execute(&ClientMessage::BuildBlock {
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
        let invalid = runtime.execute(&ClientMessage::BuildBlock {
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
        let mut state = runtime().state().clone();
        state.player.position = Vec3::new(10.0, 1.0, 3.0);
        let coordinate = IVec3::new(0, 1, 0);
        let canonical = state
            .prepare_client_event(&ClientMessage::BuildBlock {
                operation_id: "prepared-clear-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate,
                kind: BlockKind::Structural,
                orientation: 0,
            })
            .expect("clear construction event prepares");
        state.player.position = state.grids[STARTER_GRID_ID].world_position(coordinate);
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
        move_player_near_grid(&mut runtime);
        runtime
            .execute(&ClientMessage::BuildBlock {
                operation_id: "place-anchor-frame".into(),
                grid_id: STARTER_GRID_ID.into(),
                coordinate: IVec3::new(-2, 0, 0),
                kind: BlockKind::Anchor,
                orientation: 3,
            })
            .expect("anchor frame placement accepted");
        let unfinished = runtime.execute(&ClientMessage::ToggleGridAnchor {
            operation_id: "engage-unfinished-anchor".into(),
            grid_id: STARTER_GRID_ID.into(),
        });
        assert!(matches!(
            unfinished,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "anchor_not_touching_voxel"
        ));
        weld_to_completion(&mut runtime, IVec3::new(-2, 0, 0), "weld-anchor");
        runtime
            .execute(&ClientMessage::ToggleGridAnchor {
                operation_id: "engage-complete-anchor".into(),
                grid_id: STARTER_GRID_ID.into(),
            })
            .expect("complete anchor engages");
        assert!(runtime.state().grids[STARTER_GRID_ID].anchored);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn unfinished_power_block_does_not_join_the_network() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let baseline = runtime.state().grids[STARTER_GRID_ID].power().produced;
        runtime
            .execute(&ClientMessage::BuildBlock {
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
        let lock_path = directory.path().join("writer.lock");
        let mut lease: serde_json::Value =
            serde_json::from_slice(&fs::read(&lock_path).expect("lease reads"))
                .expect("lease parses");
        lease["fencing_token"] = serde_json::json!(9_999);
        fs::write(
            &lock_path,
            serde_json::to_vec_pretty(&lease).expect("lease serializes"),
        )
        .expect("replacement lease writes");

        let result = runtime.execute(&ClientMessage::SetPlayerControl {
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
            runtime.execute(&ClientMessage::SetPlayerControl {
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
        runtime.state.player.position = start;
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("test setup rebuilds the rotated grid scene");

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
        runtime.state.player.position = PLANET_CENTER
            + Vec3::new(
                0.0,
                PLANET_SURFACE_RADIUS_M + standing_half_height + 2.0,
                0.0,
            );
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("planet landing fixture rebuilds");

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
            let distance = (runtime.state().player.position - PLANET_CENTER).magnitude();
            let gap = distance - PLANET_SURFACE_RADIUS_M - standing_half_height;
            minimum_gap = minimum_gap.min(gap);
            maximum_gap = maximum_gap.max(gap);
        }
        assert!(minimum_gap > -REPLAY_CONTACT_SLOP_M);
        assert!(maximum_gap - minimum_gap < 0.05);
        assert!(runtime.state().player.linear_velocity.magnitude() < 0.25);
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
    fn grounded_capsule_walks_sprints_and_brakes_in_the_surface_tangent_frame() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        runtime.state.player.position = PLANET_CENTER
            + Vec3::new(
                0.0,
                PLANET_SURFACE_RADIUS_M + standing_half_height + 0.02,
                0.0,
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
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("grounded walking fixture rebuilds");
        runtime.advance(17).expect("support is classified");
        assert_eq!(
            runtime.state().player.locomotion.kind,
            LocomotionKind::Grounded
        );

        let initial_x = runtime.state().player.position.x;
        runtime
            .execute(&ClientMessage::SetPlayerControl {
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
            .execute(&ClientMessage::SetPlayerControl {
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
            .execute(&ClientMessage::SetPlayerControl {
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
        runtime.state.player.position = PLANET_CENTER
            + Vec3::new(
                0.0,
                PLANET_SURFACE_RADIUS_M + standing_half_height + 0.02,
                0.0,
            );
        runtime.state.player.linear_velocity = Vec3::ZERO;
        runtime.state.player.jetpack_enabled = false;
        runtime.state.player.locomotion = reset_locomotion(
            runtime.state.player.position,
            LocomotionKind::Airborne,
            false,
            runtime.state.simulation_tick,
        );
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("jump fixture rebuilds");
        runtime.advance(17).expect("support is classified");

        runtime
            .execute(&ClientMessage::SetPlayerControl {
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
            .execute(&ClientMessage::SetPlayerControl {
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
        runtime.state.player.position = Vec3::new(11.0, 1.42, 0.0);
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
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("moving support fixture rebuilds");

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
    fn below_ccd_threshold_planet_step_stays_inside_the_replay_penetration_budget() {
        let mut runtime = runtime();
        let standing_half_height = content::manifest().character.standing_height_m * 0.5;
        runtime.state.player.position = PLANET_CENTER
            + Vec3::new(
                0.0,
                PLANET_SURFACE_RADIUS_M + standing_half_height + 0.10,
                0.0,
            );
        runtime.state.player.linear_velocity = Vec3::new(0.0, -12.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("near-surface landing fixture rebuilds");

        runtime
            .advance(17)
            .expect("near-threshold planet step remains replay-valid");
        let distance = (runtime.state().player.position - PLANET_CENTER).magnitude();
        assert!(
            distance
                >= PLANET_SURFACE_RADIUS_M + standing_half_height
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
        runtime.state.player.position = Vec3::new(
            f64::from(surface.x),
            f64::from(surface.y) + 0.5 + radius + 2.0,
            f64::from(surface.z),
        );
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("voxel collision fixture rebuilds");

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
        runtime.state.player.position = clear_start;
        runtime.state.player.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
        runtime.state.player.surface_contact = false;
        runtime.state.active_contact_pairs.clear();
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("nearby clear voxel fixture rebuilds");
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
        grid.anchored = true;
        let block_position = grid.world_position(IVec3::ZERO);
        runtime.state.player.position = block_position + Vec3::new(0.0, 0.5 + radius + 2.0, 0.0);
        runtime.state.player.linear_velocity = Vec3::new(0.0, -24.0, 0.0);
        runtime.state.player.jetpack_enabled = false;
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("axis-aligned grid fixture rebuilds");

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
        runtime.state.player.position = clear_start;
        runtime.state.player.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
        runtime.state.player.surface_contact = false;
        runtime.state.active_contact_pairs.clear();
        runtime
            .physics
            .rebuild(&physics_body_specs(&runtime.state))
            .expect("nearby clear grid fixture rebuilds");
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
                .execute(&ClientMessage::SetGridControl {
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
                .execute(&ClientMessage::SetGridControl {
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
            before_target = reachable_voxel(&runtime);
            let body_id =
                voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(before_target));
            let collider_id = voxel_collision_collider_id(before_target);
            prior_hash = runtime.state().state_hash();
            prior_fingerprint = runtime.physics.body_collider_fingerprint();
            assert_eq!(
                prior_fingerprint,
                expected_physics_fingerprint(runtime.state())
            );
            runtime
                .store
                .set_append_failpoint(AppendFailpoint::BeforeWrite);
            assert!(matches!(
                runtime.execute(&ClientMessage::MineVoxel {
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
            assert!(!runtime.physics.contains_collider(&body_id, &collider_id));
        }
        let recovered = Runtime::open(before_directory.path(), 109, 100)
            .expect("before-write mining failure recovers");
        assert_eq!(recovered.state().state_hash(), prior_hash);
        assert!(recovered.state().voxels.occupied.contains(&before_target));
        assert_eq!(
            recovered.physics.body_collider_fingerprint(),
            prior_fingerprint
        );

        let after_directory = tempdir().expect("tempdir");
        let expected_durable_state;
        let after_target;
        {
            let mut runtime =
                Runtime::open(after_directory.path(), 113, 100).expect("runtime opens");
            after_target = reachable_voxel(&runtime);
            let body_id =
                voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(after_target));
            let collider_id = voxel_collision_collider_id(after_target);
            let prior_state = runtime.state().clone();
            runtime
                .store
                .set_append_failpoint(AppendFailpoint::AfterSync);
            assert!(matches!(
                runtime.execute(&ClientMessage::MineVoxel {
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
            assert!(!runtime.physics.contains_collider(&body_id, &collider_id));

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
            recovered.physics.body_collider_fingerprint(),
            expected_physics_fingerprint(&expected_durable_state)
        );
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn disconnected_damage_splits_grid_without_duplicating_blocks() {
        let mut runtime = runtime();
        move_player_near_grid(&mut runtime);
        let build = ClientMessage::BuildBlock {
            operation_id: "build-bridge".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::DamageTest,
            orientation: 0,
        };
        runtime.execute(&build).expect("bridge block built");
        let build_top = ClientMessage::BuildBlock {
            operation_id: "build-top".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 2, 0),
            kind: BlockKind::Structural,
            orientation: 0,
        };
        runtime.execute(&build_top).expect("top block built");
        weld_to_completion(&mut runtime, IVec3::new(0, 1, 0), "weld-bridge");
        weld_to_completion(&mut runtime, IVec3::new(0, 2, 0), "weld-top");
        let bridge_id = runtime.state().grids[STARTER_GRID_ID]
            .block_at(IVec3::new(0, 1, 0))
            .expect("bridge block")
            .block_id
            .clone();
        for index in 0..2 {
            runtime
                .execute(&ClientMessage::DamageBlock {
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
        assert_eq!(runtime.state().grids.len(), 2);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn authoritative_grid_controls_cannot_drive_through_asteroid_voxels() {
        let mut runtime = runtime();
        runtime
            .execute(&ClientMessage::SetGridControl {
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
                .execute(&ClientMessage::SetGridControl {
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
        let cargo_id = heavy
            .state()
            .inventories
            .keys()
            .find(|inventory_id| inventory_id.contains("cargo"))
            .cloned()
            .expect("starter cargo exists");
        heavy
            .execute(&ClientMessage::TransferInventory {
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
            .execute(&ClientMessage::SetGridControl {
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
            runtime
                .physics
                .rebuild(&physics_body_specs(&runtime.state))
                .expect("cargo-bearing anchor fixture physics rebuilds");
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
                runtime.execute(&ClientMessage::SetGridControl {
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
                .execute(&ClientMessage::ToggleGridAnchor {
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
                .execute(&ClientMessage::SetGridControl {
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
            .execute(&ClientMessage::SetGridControl {
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
            .execute(&ClientMessage::SetGridControl {
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
            .execute(&ClientMessage::SetGridControl {
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
