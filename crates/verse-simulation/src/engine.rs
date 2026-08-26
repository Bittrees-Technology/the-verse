// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use thiserror::Error;
use verse_protocol::{
    BlockKind, ClientMessage, IntentReceipt, InventoryContents, InventoryDomain, ResourceKind,
    Vec3, WorldSnapshot,
};

use crate::content;
use crate::event::{CanonicalEvent, EventPayload};
use crate::model::{Block, Grid, InventoryRecord, PLAYER_INVENTORY_ID, WorldState};
use crate::persistence::{PersistenceError, Store};

const MAX_PLAYER_MOVE_STEP: f64 = 3.0;
const MINING_RANGE: f64 = 8.5;
const MAX_GRID_SPEED: f64 = 8.0;
const MAX_GRID_ANGULAR_SPEED: f64 = 1.5;
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
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    #[error(transparent)]
    Intent(#[from] IntentError),
    #[error("authoritative writes are halted after a persistence failure")]
    Halted,
}

#[derive(Debug)]
pub struct Runtime {
    store: Store,
    state: WorldState,
    snapshot_every: u64,
    events_since_snapshot: u64,
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

        let mut runtime = Self {
            store,
            state,
            snapshot_every: snapshot_every.max(1),
            events_since_snapshot: 0,
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
        let moving = self.state.grids.values().any(|grid| {
            !grid.anchored
                && (grid.linear_velocity.magnitude() > f64::EPSILON
                    || grid.angular_velocity.abs() > f64::EPSILON)
        });
        if !moving {
            return Ok(false);
        }

        let event = self
            .state
            .prepare_system_event(EventPayload::SimulationAdvanced {
                delta_millis: delta_millis.clamp(1, 250),
            });
        let mut next_state = self.state.clone();
        next_state.apply_event(&event)?;
        if let Err(source) = self.store.append_event(&event) {
            self.halted = true;
            return Err(source.into());
        }
        self.state = next_state;
        self.after_event()?;
        Ok(true)
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
                if self.player.position.squared_distance(*position)
                    > MAX_PLAYER_MOVE_STEP * MAX_PLAYER_MOVE_STEP
                {
                    return Err(IntentError::rejected(
                        "movement_too_large",
                        "authoritative player movement exceeded the per-intent limit",
                    ));
                }
                EventPayload::PlayerMoved {
                    position: *position,
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
                EventPayload::VoxelMined {
                    coordinate: *coordinate,
                    material,
                    ore_yield: content::voxel(material).ore_yield,
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
                let source = self.inventory(source_inventory_id)?;
                self.inventory(destination_inventory_id)?;
                if source.contents.amount(*resource) < *quantity {
                    return Err(IntentError::rejected(
                        "insufficient_inventory",
                        "source inventory does not contain the requested quantity",
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
                ..
            } => {
                let grid = self.grid(grid_id)?;
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
                if *kind == BlockKind::Cargo {
                    block.inventory_id = Some(format!("inventory-{block_id}"));
                }
                EventPayload::BlockBuilt {
                    grid_id: grid_id.clone(),
                    block,
                }
            }
            ClientMessage::SetGridMotion {
                grid_id,
                linear_velocity,
                angular_velocity,
                ..
            } => {
                ensure_finite(*linear_velocity, "grid velocity")?;
                if !angular_velocity.is_finite() {
                    return Err(IntentError::rejected(
                        "invalid_motion",
                        "grid angular velocity must be finite",
                    ));
                }
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
                if linear_velocity.magnitude() > MAX_GRID_SPEED
                    || angular_velocity.abs() > MAX_GRID_ANGULAR_SPEED
                {
                    return Err(IntentError::rejected(
                        "motion_limit_exceeded",
                        "requested grid motion exceeds the P0 safety budget",
                    ));
                }
                EventPayload::GridMotionSet {
                    grid_id: grid_id.clone(),
                    linear_velocity: *linear_velocity,
                    angular_velocity: *angular_velocity,
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
                if !grid.blocks.contains_key(block_id) {
                    return Err(IntentError::rejected(
                        "block_missing",
                        "target block does not exist on the grid",
                    ));
                }
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
                let ore_required = batches * recipe.ore_input;
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
                let inventory = self.inventory_mut(inventory_id)?;
                inventory.contents.refined_material -= quantity * recipe.refined_input;
                inventory.contents.components += quantity * recipe.component_output;
                self.ledger.crafted_components += quantity;
            }
            EventPayload::InventoryTransferred {
                source_inventory_id,
                destination_inventory_id,
                resource,
                quantity,
            } => {
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
                        },
                    );
                }
                self.grid_mut(grid_id)?
                    .blocks
                    .insert(block.block_id.clone(), block.clone());
                self.ledger.built_blocks += 1;
            }
            EventPayload::GridMotionSet {
                grid_id,
                linear_velocity,
                angular_velocity,
            } => {
                let grid = self.grid_mut(grid_id)?;
                grid.linear_velocity = *linear_velocity;
                grid.angular_velocity = *angular_velocity;
            }
            EventPayload::GridAnchorSet { grid_id, anchored } => {
                let grid = self.grid_mut(grid_id)?;
                grid.anchored = *anchored;
                if *anchored {
                    grid.linear_velocity = Vec3::ZERO;
                    grid.angular_velocity = 0.0;
                }
            }
            EventPayload::BlockDamaged {
                grid_id,
                block_id,
                damage,
            } => self.apply_damage(grid_id, block_id, *damage, event.event_sequence)?,
            EventPayload::SimulationAdvanced { delta_millis } => {
                self.simulation_tick += 1;
                let delta_seconds = f64::from(*delta_millis) / 1_000.0;
                for grid in self.grids.values_mut().filter(|grid| !grid.anchored) {
                    grid.position = grid.position + grid.linear_velocity * delta_seconds;
                    grid.yaw_radians = (grid.yaw_radians + grid.angular_velocity * delta_seconds)
                        .rem_euclid(std::f64::consts::TAU);
                }
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
            EventPayload::BlockBuilt { .. } => self.player.career.blocks_built += 1,
            EventPayload::GridAnchorSet { anchored: true, .. } => {
                self.player.career.anchors_engaged += 1;
            }
            EventPayload::PlayerMoved { .. }
            | EventPayload::InventoryTransferred { .. }
            | EventPayload::GridMotionSet { .. }
            | EventPayload::GridAnchorSet {
                anchored: false, ..
            }
            | EventPayload::BlockDamaged { .. }
            | EventPayload::SimulationAdvanced { .. } => {}
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
                yaw_radians: original.yaw_radians,
                linear_velocity: original.linear_velocity,
                angular_velocity: original.angular_velocity,
                anchored: original.anchored,
                blocks,
            };
            grid.anchored = grid.anchored && grid.anchor_touches(&self.voxels);
            self.grids.insert(new_grid_id, grid);
        }
        Ok(())
    }

    fn inventory(&self, inventory_id: &str) -> Result<&InventoryRecord, IntentError> {
        self.inventories.get(inventory_id).ok_or_else(|| {
            IntentError::rejected(
                "inventory_missing",
                format!("inventory {inventory_id} does not exist"),
            )
        })
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
    use crate::model::STARTER_GRID_ID;

    fn runtime() -> Runtime {
        Runtime::open(tempdir().expect("tempdir").keep(), 42, 5).expect("runtime opens")
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
    fn build_retry_does_not_consume_a_second_component() {
        let mut runtime = runtime();
        let intent = ClientMessage::BuildBlock {
            operation_id: "idempotent-build".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::Structural,
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
    fn disconnected_damage_splits_grid_without_duplicating_blocks() {
        let mut runtime = runtime();
        let build = ClientMessage::BuildBlock {
            operation_id: "build-bridge".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 1, 0),
            kind: BlockKind::DamageTest,
        };
        runtime.execute(&build).expect("bridge block built");
        let build_top = ClientMessage::BuildBlock {
            operation_id: "build-top".into(),
            grid_id: STARTER_GRID_ID.into(),
            coordinate: IVec3::new(0, 2, 0),
            kind: BlockKind::Structural,
        };
        runtime.execute(&build_top).expect("top block built");
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
}
