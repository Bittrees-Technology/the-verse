// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use thiserror::Error;
use verse_physics::{
    BodyControl, BodySpec, BoxColliderSpec, PhysicsError, Pose as PhysicsPose, Quat as PhysicsQuat,
    Scene, SceneConfig, Vec3 as PhysicsVec3,
};
use verse_protocol::{
    BlockKind, ClientMessage, IVec3, IntentReceipt, InventoryContents, InventoryDomain, Quat,
    ResourceKind, Vec3, WorldSnapshot,
};

use crate::content;
use crate::event::{
    CanonicalEvent, EVENT_SCHEMA_NAME, EVENT_SCHEMA_VERSION, EventPayload, PhysicsBodyOutcome,
    PhysicsContactOutcome, PhysicsContactPhase,
};
use crate::model::{
    Block, CARGO_INVENTORY_CAPACITY_LITERS, ContactPairKey, Grid, InventoryRecord, PLANET_CENTER,
    PLANET_SURFACE_RADIUS_M, PLAYER_INVENTORY_ID, WorldState,
};
use crate::persistence::{PersistenceError, Store};

const MAX_PLAYER_MOVE_STEP: f64 = 3.0;
const PLAYER_COLLISION_RADIUS: f64 = 0.32;
const MINING_RANGE: f64 = 8.5;
const HAND_TOOL_RANGE: f64 = 9.0;
const MAX_GRID_CONTROL_INPUT: f64 = 1.0;
const CONTROL_INPUT_EPSILON: f64 = 1.0e-9;
const MAX_GRID_BLOCKS_P0: usize = 2_048;

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
        };
        if runtime.state.event_sequence == 0 {
            runtime.store.save_snapshot(&runtime.state)?;
        }
        Ok(runtime)
    }

    pub const fn state(&self) -> &WorldState {
        &self.state
    }

    pub const fn is_halted(&self) -> bool {
        self.halted
    }

    pub fn snapshot(&self) -> WorldSnapshot {
        self.state.snapshot()
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
        if event_changes_physics_scene(&event.payload) {
            self.physics.rebuild(&physics_body_specs(&next_state))?;
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
        self.advance_with_player_presence(delta_millis, true)
    }

    pub fn advance_with_player_presence(
        &mut self,
        delta_millis: u16,
        player_active: bool,
    ) -> Result<bool, RuntimeError> {
        if self.halted {
            return Err(RuntimeError::Halted);
        }
        let moving = self.state.grids.values().any(|grid| {
            !grid.anchored
                && (grid.linear_velocity.magnitude() > f64::EPSILON
                    || grid.angular_velocity.magnitude() > f64::EPSILON
                    || grid.control_linear_input.magnitude() > f64::EPSILON
                    || grid.control_angular_input.magnitude() > f64::EPSILON)
        });
        let delta_millis = delta_millis.clamp(1, 250);
        let mut changed = false;
        if moving {
            let fixed_step_hz = content::manifest().physics.fixed_step_hz;
            self.physics_step_phase = self
                .physics_step_phase
                .saturating_add(u64::from(delta_millis) * 1_000_000 * u64::from(fixed_step_hz));
            let step_count = (self.physics_step_phase / 1_000_000_000).min(15);
            if step_count > 0 {
                self.physics_step_phase -= step_count * 1_000_000_000;
                let controls = physics_controls(&self.state);
                let mut output = None;
                let mut contacts = Vec::new();
                let mut active_contacts = self.state.active_contact_pairs.clone();
                for substep_index in 0..step_count {
                    let step = match self.physics.step(&controls) {
                        Ok(step) => step,
                        Err(source) => {
                            self.halted = true;
                            return Err(source.into());
                        }
                    };
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
                changed = true;
            }
        }

        if player_active {
            self.life_support_elapsed_millis = self
                .life_support_elapsed_millis
                .saturating_add(u32::from(delta_millis));
        }
        if player_active && self.life_support_elapsed_millis >= 1_000 {
            let elapsed_seconds = self.life_support_elapsed_millis / 1_000;
            self.life_support_elapsed_millis %= 1_000;
            let previous_oxygen_milli = self.state.player.suit_oxygen_milli;
            let environment = self.state.environment_at(self.state.player.position);
            let per_second_delta = if !self.state.player.helmet_closed && environment.breathable {
                25_i32
            } else if !self.state.player.helmet_closed {
                -40_i32
            } else if environment.breathable {
                0_i32
            } else {
                -5_i32
            };
            let new_oxygen_milli = u16::try_from(
                (i32::from(previous_oxygen_milli)
                    + per_second_delta * i32::try_from(elapsed_seconds).unwrap_or(i32::MAX))
                .clamp(0, 1_000),
            )
            .expect("clamped suit oxygen always fits u16");
            if new_oxygen_milli != previous_oxygen_milli {
                self.commit_system_event(EventPayload::SuitOxygenChanged {
                    previous_oxygen_milli,
                    new_oxygen_milli,
                })?;
                changed = true;
            }
        }
        Ok(changed)
    }

    fn commit_system_event(&mut self, payload: EventPayload) -> Result<(), RuntimeError> {
        let event = self.state.prepare_system_event(payload);
        let mut next_state = self.state.clone();
        next_state.apply_event(&event)?;
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

        let payload = match message {
            ClientMessage::MovePlayer { position, .. } => {
                ensure_finite(*position, "player position")?;
                let distance_squared = self.player.position.squared_distance(*position);
                if distance_squared <= 0.000_001 {
                    return Err(IntentError::rejected(
                        "movement_no_change",
                        "stationary movement intents are not journaled",
                    ));
                }
                if distance_squared > MAX_PLAYER_MOVE_STEP * MAX_PLAYER_MOVE_STEP {
                    return Err(IntentError::rejected(
                        "movement_too_large",
                        "authoritative player movement exceeded the per-intent limit",
                    ));
                }
                let planet_distance = Vec3::new(
                    position.x - PLANET_CENTER.x,
                    position.y - PLANET_CENTER.y,
                    position.z - PLANET_CENTER.z,
                )
                .magnitude();
                if planet_distance < PLANET_SURFACE_RADIUS_M + 0.45 {
                    return Err(IntentError::rejected(
                        "movement_below_planet_surface",
                        "player movement cannot pass through the planetary surface",
                    ));
                }
                if self.player_movement_hits_voxel(self.player.position, *position) {
                    return Err(IntentError::rejected(
                        "movement_hits_voxel",
                        "player movement cannot enter authoritative asteroid material",
                    ));
                }
                if self.player_movement_hits_grid(self.player.position, *position) {
                    return Err(IntentError::rejected(
                        "movement_hits_grid",
                        "player movement cannot enter an authoritative grid block",
                    ));
                }
                EventPayload::PlayerMoved {
                    position: *position,
                }
            }
            ClientMessage::SetSuitMode {
                helmet_closed,
                jetpack_enabled,
                ..
            } => {
                if self.player.helmet_closed == *helmet_closed
                    && self.player.jetpack_enabled == *jetpack_enabled
                {
                    return Err(IntentError::rejected(
                        "suit_mode_no_change",
                        "helmet and jetpack already match the requested state",
                    ));
                }
                EventPayload::SuitModeChanged {
                    helmet_closed: *helmet_closed,
                    jetpack_enabled: *jetpack_enabled,
                }
            }
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

        match &event.payload {
            EventPayload::PlayerMoved { position } => self.player.position = *position,
            EventPayload::SuitModeChanged {
                helmet_closed,
                jetpack_enabled,
            } => {
                self.player.helmet_closed = *helmet_closed;
                self.player.jetpack_enabled = *jetpack_enabled;
            }
            EventPayload::SuitOxygenChanged {
                previous_oxygen_milli,
                new_oxygen_milli,
            } => {
                if self.player.suit_oxygen_milli != *previous_oxygen_milli
                    || *new_oxygen_milli > 1_000
                {
                    return Err(IntentError::rejected(
                        "replay_suit_oxygen_invalid",
                        "life-support event does not match the authoritative suit state",
                    ));
                }
                self.player.suit_oxygen_milli = *new_oxygen_milli;
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
            EventPayload::PlayerMoved { .. }
            | EventPayload::SuitModeChanged { .. }
            | EventPayload::SuitOxygenChanged { .. }
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
        if let Some(grid) = self.grids.get(body_id) {
            return grid.blocks.contains_key(collider_id);
        }
        body_id == "voxel-field-origin"
            && self.voxels.occupied.iter().any(|coordinate| {
                format!(
                    "voxel-{x}-{y}-{z}",
                    x = coordinate.x,
                    y = coordinate.y,
                    z = coordinate.z
                ) == collider_id
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
        let InventoryDomain::Cargo { block_id } = &inventory.domain else {
            return Ok(());
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
        let extent = PLAYER_COLLISION_RADIUS + 0.5;
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

    fn player_movement_hits_grid(&self, start: Vec3, end: Vec3) -> bool {
        movement_samples(start, end)
            .into_iter()
            .any(|position| self.player_intersects_grid(position))
    }
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
        EventPayload::PlayerMoved { .. }
            | EventPayload::SuitModeChanged { .. }
            | EventPayload::SuitOxygenChanged { .. }
            | EventPayload::GridControlSet { .. }
            | EventPayload::PhysicsStepCommitted { .. }
    )
}

fn physics_body_specs(state: &WorldState) -> Vec<BodySpec> {
    let physics = &content::manifest().physics;
    let mut bodies = Vec::with_capacity(state.grids.len() + 1);
    if !state.voxels.occupied.is_empty() {
        let colliders = state
            .voxels
            .occupied
            .iter()
            .map(|coordinate| BoxColliderSpec {
                collider_id: format!(
                    "voxel-{x}-{y}-{z}",
                    x = coordinate.x,
                    y = coordinate.y,
                    z = coordinate.z
                ),
                local_pose: PhysicsPose::new(
                    PhysicsVec3::new(
                        f64::from(coordinate.x),
                        f64::from(coordinate.y),
                        f64::from(coordinate.z),
                    ),
                    PhysicsQuat::IDENTITY,
                ),
                half_extents: PhysicsVec3::new(0.5, 0.5, 0.5),
                density_kg_per_m3: 2_600.0,
            })
            .collect();
        let mut asteroid =
            BodySpec::static_body("voxel-field-origin", PhysicsPose::IDENTITY, colliders);
        asteroid.friction = physics.friction;
        asteroid.restitution = physics.restitution;
        bodies.push(asteroid);
    }
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

fn physics_controls(state: &WorldState) -> Vec<BodyControl> {
    let physics = &content::manifest().physics;
    state
        .grids
        .values()
        .filter(|grid| !grid.anchored)
        .map(|grid| {
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
                grid.linear_velocity * -physics.linear_dampener_newtons_per_mps
            } else {
                Vec3::ZERO
            };
            let dampener_torque = if online && grid.dampeners {
                grid.angular_velocity * -physics.angular_dampener_newton_meters_per_radian
            } else {
                Vec3::ZERO
            };
            BodyControl {
                body_id: grid.grid_id.clone(),
                force_newtons: to_physics_vec3(user_force + dampener_force),
                torque_newton_meters: to_physics_vec3(user_torque + dampener_torque),
            }
        })
        .collect()
}

fn physics_body_outcome(body: &verse_physics::BodyState) -> PhysicsBodyOutcome {
    PhysicsBodyOutcome {
        grid_id: body.body_id.clone(),
        position: from_physics_vec3(body.pose.position),
        orientation: from_physics_quat(body.pose.rotation),
        linear_velocity: from_physics_vec3(body.linear_velocity),
        angular_velocity: from_physics_vec3(body.angular_velocity),
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

fn reduced_translational_contact_mass_grams(
    state: &WorldState,
    left_body: &str,
    right_body: &str,
) -> u64 {
    fn mass(state: &WorldState, body_id: &str) -> Option<u64> {
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
    center.squared_distance(Vec3::new(closest_x, closest_y, closest_z))
        <= PLAYER_COLLISION_RADIUS * PLAYER_COLLISION_RADIUS
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
        for (index, position) in [
            Vec3::new(11.0, 3.5, 7.5),
            Vec3::new(10.5, 2.5, 5.0),
            Vec3::new(10.0, 1.0, 3.0),
        ]
        .into_iter()
        .enumerate()
        {
            runtime
                .execute(&ClientMessage::MovePlayer {
                    operation_id: format!("approach-grid-{index}"),
                    position,
                })
                .expect("bounded movement approaches the grid");
        }
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
        let voxel_collider = format!("voxel-{}-{}-{}", voxel.x, voxel.y, voxel.z);
        let key = ContactPairKey {
            body_a: STARTER_GRID_ID.into(),
            collider_a: "block-core".into(),
            body_b: "voxel-field-origin".into(),
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
            contacts: vec![PhysicsContactOutcome {
                substep_index: 0,
                body_a_id: STARTER_GRID_ID.into(),
                collider_a_id: "block-core".into(),
                body_b_id: "voxel-field-origin".into(),
                collider_b_id: voxel_collider,
                point: Vec3::ZERO,
                normal: Vec3::new(-1.0, 0.0, 0.0),
                penetration_m: 0.01,
                closing_speed_mm_per_second: 1_000,
                estimated_normal_impulse_millinewton_seconds: 5_000,
                reduced_translational_mass_grams: reduced_translational_contact_mass_grams(
                    state,
                    STARTER_GRID_ID,
                    "voxel-field-origin",
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
            bodies[0].linear_velocity = Vec3::new(32.000_001, 0.0, 0.0);
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
        let target = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coord| {
                let pos = Vec3::new(f64::from(coord.x), f64::from(coord.y), f64::from(coord.z));
                runtime.state().player.position.squared_distance(pos) <= MINING_RANGE * MINING_RANGE
            })
            .expect("reachable voxel");
        let intent = ClientMessage::MineVoxel {
            operation_id: "mine-once".into(),
            coordinate: target,
        };
        let first = runtime.execute(&intent).expect("first mine accepted");
        let hash_after_first = runtime.state().state_hash();
        let second = runtime.execute(&intent).expect("retry accepted");
        assert_eq!(first, second);
        assert_eq!(hash_after_first, runtime.state().state_hash());
        assert!(runtime.state().conservation().valid);
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
    fn untrusted_player_cannot_teleport() {
        let mut runtime = runtime();
        let result = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "attempt-teleport".into(),
            position: Vec3::new(1_000.0, 1_000.0, 1_000.0),
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_too_large"
        ));
        assert_eq!(runtime.state().event_sequence, 0);
        assert!(runtime.state().conservation().valid);
    }

    #[test]
    fn untrusted_player_cannot_phase_into_authoritative_voxels() {
        let mut runtime = runtime();
        let target = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .next()
            .copied()
            .expect("asteroid voxel");
        runtime.state.player.position = Vec3::new(
            f64::from(target.x),
            f64::from(target.y),
            f64::from(target.z) + 2.5,
        );
        let result = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "attempt-voxel-phase".into(),
            position: Vec3::new(
                f64::from(target.x),
                f64::from(target.y),
                f64::from(target.z),
            ),
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_hits_voxel"
        ));
        assert_eq!(runtime.state().event_sequence, 0);
    }

    #[test]
    fn untrusted_player_cannot_tunnel_through_a_voxel_between_valid_endpoints() {
        let mut runtime = runtime();
        runtime.state.player.position = Vec3::new(-1.2, 0.0, 0.0);
        runtime.state.voxels.occupied = BTreeSet::from([IVec3::ZERO]);
        runtime.state.voxels.ferrite_ore.clear();

        let result = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "tunnel-through-voxel".into(),
            position: Vec3::new(1.2, 0.0, 0.0),
        });

        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_hits_voxel"
        ));
        assert_eq!(runtime.state().event_sequence, 0);
    }

    #[test]
    fn untrusted_player_cannot_tunnel_through_a_grid_between_valid_endpoints() {
        let mut runtime = runtime();
        runtime.state.player.position = Vec3::new(11.0, 1.2, 0.0);

        let result = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "tunnel-through-grid".into(),
            position: Vec3::new(11.0, -1.2, 0.0),
        });

        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_hits_grid"
        ));
        assert_eq!(runtime.state().event_sequence, 0);
        assert_eq!(runtime.state().player.position, Vec3::new(11.0, 1.2, 0.0));
    }

    #[test]
    fn stationary_movement_does_not_create_an_event() {
        let mut runtime = runtime();
        let result = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "stationary-spam".into(),
            position: runtime.state().player.position,
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_no_change"
        ));
        assert_eq!(runtime.state().event_sequence, 0);
        assert!(runtime.state().processed_operations.is_empty());
    }

    #[test]
    fn planetary_surface_rejects_underground_movement() {
        let mut runtime = runtime();
        runtime.state.player.position = Vec3::new(
            PLANET_CENTER.x,
            PLANET_CENTER.y + PLANET_SURFACE_RADIUS_M + 0.7,
            PLANET_CENTER.z,
        );
        let result = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "walk-through-ground".into(),
            position: Vec3::new(
                PLANET_CENTER.x,
                PLANET_CENTER.y + PLANET_SURFACE_RADIUS_M + 0.2,
                PLANET_CENTER.z,
            ),
        });
        assert!(matches!(
            result,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_below_planet_surface"
        ));
        assert_eq!(runtime.state().event_sequence, 0);
    }

    #[test]
    fn suit_modes_and_environment_drive_authoritative_oxygen() {
        let mut runtime = runtime();
        runtime.state.player.position = Vec3::new(
            PLANET_CENTER.x,
            PLANET_CENTER.y + PLANET_SURFACE_RADIUS_M + 10.0,
            PLANET_CENTER.z,
        );
        runtime.state.player.suit_oxygen_milli = 900;
        runtime
            .execute(&ClientMessage::SetSuitMode {
                operation_id: "open-helmet".into(),
                helmet_closed: false,
                jetpack_enabled: true,
            })
            .expect("helmet opens in breathable atmosphere");
        assert!(!runtime.advance(250).expect("life support tick"));
        assert!(!runtime.advance(250).expect("life support tick"));
        assert!(!runtime.advance(250).expect("life support tick"));
        assert!(runtime.advance(250).expect("life support tick"));
        assert_eq!(runtime.state().player.suit_oxygen_milli, 925);

        runtime.state.player.position = Vec3::ZERO;
        for _ in 0..4 {
            runtime.advance(250).expect("vacuum life support tick");
        }
        assert_eq!(runtime.state().player.suit_oxygen_milli, 885);
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

        let result = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "stale-writer-move".into(),
            position: Vec3::new(12.5, 4.5, 10.0),
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
            runtime.execute(&ClientMessage::MovePlayer {
                operation_id: "halted-move".into(),
                position: Vec3::new(12.5, 4.5, 10.0),
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
    fn swept_player_collision_tracks_a_rotated_grid_without_blocking_clear_motion() {
        let mut runtime = runtime();
        let half_sqrt = std::f32::consts::FRAC_1_SQRT_2;
        runtime
            .state
            .grids
            .get_mut(STARTER_GRID_ID)
            .unwrap()
            .orientation = Quat::new(0.0, half_sqrt, 0.0, half_sqrt);
        let drill_position =
            runtime.state.grids[STARTER_GRID_ID].world_position(IVec3::new(2, 0, 0));
        let start = drill_position + Vec3::new(0.0, 1.2, 0.0);
        runtime.state.player.position = start;
        let before_hash = runtime.state().state_hash();

        let blocked = runtime.execute(&ClientMessage::MovePlayer {
            operation_id: "cross-rotated-grid".into(),
            position: drill_position + Vec3::new(0.0, -1.2, 0.0),
        });
        assert!(matches!(
            blocked,
            Err(RuntimeError::Intent(IntentError::Rejected { ref code, .. }))
                if code == "movement_hits_grid"
        ));
        assert_eq!(runtime.state().state_hash(), before_hash);
        assert_eq!(runtime.state().player.position, start);

        runtime
            .execute(&ClientMessage::MovePlayer {
                operation_id: "clear-above-rotated-grid".into(),
                position: start + Vec3::new(0.0, 0.1, 0.0),
            })
            .expect("nearby clear motion remains available");
        assert_eq!(
            runtime.state().player.position,
            start + Vec3::new(0.0, 0.1, 0.0)
        );
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
