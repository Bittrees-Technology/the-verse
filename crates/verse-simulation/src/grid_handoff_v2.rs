// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant package-v2 model for ADR-0024 grid-closure handoff.
//!
//! This module is intentionally private and has no persistence or runtime
//! entry point. The active protocol-18/package-v1 path remains EVA-only until
//! the complete ADR-0024 compatibility tuple activates atomically.

#[allow(dead_code)]
mod dispatcher_v17;
#[allow(dead_code)]
mod event_v17;
#[allow(dead_code)]
mod production;
#[allow(dead_code)]
pub(crate) mod state;
#[allow(dead_code)]
mod store_v21;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verse_protocol::{CellKeyV1, InventoryContents, InventoryDomain, LocomotionKind, Vec3};

use crate::cell_directory::{BundledPlacementMember, BundledPlacementPlan, MobileAggregateKind};
use crate::engine::{
    PLANET_BODY_ID, voxel_collision_chunk_body_id, voxel_collision_chunk_coordinate,
};
use crate::model::{
    ActorOperationHistory, ContactPairKey, Grid, InventoryRecord,
    PROCESSED_OPERATION_RECORD_BYTES_LIMIT, PROCESSED_OPERATION_RETAINED_BYTES_LIMIT,
    PROCESSED_OPERATION_RETENTION_LIMIT, Player, ProductionJob, WorldState,
    production_recipe_quantities, resource_unit_mass_grams, resource_unit_volume_liters,
    valid_blake3_hex, valid_player_id,
};
use crate::{celestial, content};
use production::{DraftProductionJobOriginV2, validate_production_job_origins};

const DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION: u32 = 2;
const DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION: u32 = 2;
const MAX_DRAFT_GRID_TRANSFER_PACKAGE_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_DRAFT_GRID_BLOCKS: usize = 8_192;
const MAX_DRAFT_GRID_CARGO_INVENTORIES: usize = 8_192;
const MAX_DRAFT_GRID_PRODUCTION_QUEUES: usize = 8_192;
const MAX_DRAFT_GRID_PRODUCTION_JOBS: usize = 65_536;
const MAX_DRAFT_GRID_CONTACTS: usize = 65_536;
const MAX_DRAFT_GRID_MEMBERS: usize = 128;
const MICROMETRES_PER_METRE: f64 = 1_000_000.0;

const CLOSURE_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-closure/v2\0";
const CONSERVATION_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-conservation/v2\0";
const PACKAGE_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-package/v2\0";

/// One shared dormant tuple prevents event, directory, manifest, and package
/// codecs from drifting while active protocol 18 remains unchanged.
pub(super) type DraftGridCompatibilityTupleV19 =
    verse_protocol::protocol_v19::Protocol19CompatibilityTuple;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
enum DraftGridClosureError {
    #[error("grid closure package is invalid: {0}")]
    Invalid(String),
    #[error("grid closure is unsupported by the bounded handoff: {0}")]
    Unsupported(String),
    #[error("grid closure changed after capture: {0}")]
    Changed(String),
    #[error("grid closure package exceeds the {MAX_DRAFT_GRID_TRANSFER_PACKAGE_BYTES}-byte bound")]
    TooLarge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftGridTransferContextV2 {
    transfer_id: String,
    source_assignment_generation: u64,
    destination_assignment_generation: u64,
    source_fencing_token: u64,
    destination_fencing_token: u64,
    placement: BundledPlacementPlan,
    production_job_origins: BTreeMap<String, DraftProductionJobOriginV2>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftClosurePlayerV2 {
    source_player: Player,
    destination_player: Player,
    inventory: InventoryRecord,
    operation_history: Option<ActorOperationHistory>,
    is_owner: bool,
    is_supported_rider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridClosureConservationV2 {
    cargo_contents: InventoryContents,
    rider_contents: InventoryContents,
    reserved_inputs: InventoryContents,
    pending_outputs: InventoryContents,
    transferable_contents: InventoryContents,
    installed_components: u64,
    cargo_capacity_liters: u64,
    rider_capacity_liters: u64,
    cargo_used_liters: u64,
    rider_used_liters: u64,
    escrow_volume_liters: u64,
    total_resource_volume_liters: u64,
    cargo_mass_grams: u64,
    rider_mass_grams: u64,
    escrow_mass_grams: u64,
    installed_component_mass_grams: u64,
    grid_mass_grams: u64,
    closure_mass_grams: u64,
    block_count: u64,
    cargo_inventory_count: u64,
    production_queue_count: u64,
    production_job_count: u64,
    player_count: u64,
    supported_rider_count: u64,
    operation_history_count: u64,
    internal_contact_count: u64,
    placement_member_count: u64,
    subject_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridClosurePackageV2 {
    schema_version: u32,
    receipt_schema_version: u32,
    transfer_id: String,
    aggregate_kind: MobileAggregateKind,
    root_aggregate_id: String,
    source_cell_key: CellKeyV1,
    source_cell_id: String,
    destination_cell_key: CellKeyV1,
    destination_cell_id: String,
    source_assignment_generation: u64,
    destination_assignment_generation: u64,
    source_fencing_token: u64,
    destination_fencing_token: u64,
    members: Vec<BundledPlacementMember>,
    member_root: String,
    universe_id: String,
    universe_manifest_hash: String,
    celestial_registry_hash: String,
    content_manifest_version: String,
    source_event_sequence: u64,
    source_event_hash: String,
    source_world_hash: String,
    prepared_at_simulation_tick: u64,
    grid: Grid,
    cargo_inventories: BTreeMap<String, InventoryRecord>,
    production_queues: BTreeMap<String, VecDeque<ProductionJob>>,
    production_job_origins: BTreeMap<String, DraftProductionJobOriginV2>,
    players: BTreeMap<String, DraftClosurePlayerV2>,
    active_internal_contacts: BTreeSet<ContactPairKey>,
    conservation: DraftGridClosureConservationV2,
    closure_root: String,
    conservation_root: String,
    package_hash: String,
}

/// Borrow proving that one exact package-v2 document is bound to the complete
/// validated manifest-5 identity, not merely to caller-supplied hash strings.
#[derive(Debug)]
struct ValidatedDraftGridClosurePackageV2<'package, 'manifest> {
    package: &'package DraftGridClosurePackageV2,
    manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
}

impl ValidatedDraftGridClosurePackageV2<'_, '_> {
    fn package(&self) -> &DraftGridClosurePackageV2 {
        self.package
    }

    fn manifest_hash(&self) -> &str {
        self.manifest.manifest_hash()
    }
}

#[derive(Serialize)]
struct ClosureHashMaterial<'a> {
    root_aggregate_id: &'a str,
    member_root: &'a str,
    grid: &'a Grid,
    cargo_inventories: &'a BTreeMap<String, InventoryRecord>,
    production_queues: &'a BTreeMap<String, VecDeque<ProductionJob>>,
    production_job_origins: &'a BTreeMap<String, DraftProductionJobOriginV2>,
    players: &'a BTreeMap<String, DraftClosurePlayerV2>,
    active_internal_contacts: &'a BTreeSet<ContactPairKey>,
}

impl DraftGridClosurePackageV2 {
    fn hydrate_spatial_poses(&mut self) -> Result<(), DraftGridClosureError> {
        let source_origin = celestial::cell_address_from_key(&self.source_cell_key)
            .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
        let destination_origin = celestial::cell_address_from_key(&self.destination_cell_key)
            .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
        self.grid.position =
            celestial::local_position_from_address(&source_origin, &self.grid.address)
                .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
        for player in self.players.values_mut() {
            player.source_player.position = celestial::local_position_from_address(
                &source_origin,
                &player.source_player.address,
            )
            .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
            player.destination_player.position = celestial::local_position_from_address(
                &destination_origin,
                &player.destination_player.address,
            )
            .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
        }
        Ok(())
    }

    fn calculate_closure_root(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            CLOSURE_HASH_DOMAIN,
            &ClosureHashMaterial {
                root_aggregate_id: &self.root_aggregate_id,
                member_root: &self.member_root,
                grid: &self.grid,
                cargo_inventories: &self.cargo_inventories,
                production_queues: &self.production_queues,
                production_job_origins: &self.production_job_origins,
                players: &self.players,
                active_internal_contacts: &self.active_internal_contacts,
            },
        )
    }

    fn calculate_conservation_root(&self) -> Result<String, DraftGridClosureError> {
        hash_json(CONSERVATION_HASH_DOMAIN, &self.conservation)
    }

    fn calculate_package_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.package_hash.clear();
        hash_json(PACKAGE_HASH_DOMAIN, &material)
    }

    fn encode_canonical(&self) -> Result<Vec<u8>, DraftGridClosureError> {
        self.validate_wire()?;
        let bytes = serde_json::to_vec(self).map_err(|source| {
            DraftGridClosureError::Invalid(format!("package cannot be encoded: {source}"))
        })?;
        if bytes.len() > MAX_DRAFT_GRID_TRANSFER_PACKAGE_BYTES {
            return Err(DraftGridClosureError::TooLarge);
        }
        Ok(bytes)
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, DraftGridClosureError> {
        if bytes.len() > MAX_DRAFT_GRID_TRANSFER_PACKAGE_BYTES {
            return Err(DraftGridClosureError::TooLarge);
        }
        let mut package = serde_json::from_slice::<Self>(bytes).map_err(|source| {
            DraftGridClosureError::Invalid(format!("package JSON is invalid: {source}"))
        })?;
        package.hydrate_spatial_poses()?;
        package.validate_wire()?;
        let canonical = serde_json::to_vec(&package).map_err(|source| {
            DraftGridClosureError::Invalid(format!("package cannot be re-encoded: {source}"))
        })?;
        if canonical != bytes {
            return Err(DraftGridClosureError::Invalid(
                "package bytes are not the exact canonical encoding".into(),
            ));
        }
        Ok(package)
    }

    fn validate_wire(&self) -> Result<(), DraftGridClosureError> {
        let placement = BundledPlacementPlan {
            root_aggregate_id: self.root_aggregate_id.clone(),
            source_cell_key: self.source_cell_key.clone(),
            source_cell_id: self.source_cell_id.clone(),
            destination_cell_key: self.destination_cell_key.clone(),
            destination_cell_id: self.destination_cell_id.clone(),
            members: self.members.clone(),
            member_root: self.member_root.clone(),
        };
        placement.validate().map_err(|source| {
            DraftGridClosureError::Invalid(format!("placement bundle is invalid: {source}"))
        })?;
        validate_adjacent_cells(&self.source_cell_key, &self.destination_cell_key)?;
        if self.schema_version != DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION
            || self.receipt_schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || self.aggregate_kind != MobileAggregateKind::Grid
            || !valid_stable_id(&self.transfer_id)
            || self.source_assignment_generation == 0
            || self.destination_assignment_generation == 0
            || self.source_fencing_token == 0
            || self.destination_fencing_token == 0
            || self.universe_id != self.source_cell_key.universe_id
            || self.universe_id != self.destination_cell_key.universe_id
            || !valid_blake3_hex(&self.universe_manifest_hash)
            || !valid_blake3_hex(&self.celestial_registry_hash)
            || self.content_manifest_version.trim().is_empty()
            || !valid_blake3_hex(&self.source_world_hash)
            || (self.source_event_sequence == 0 && !self.source_event_hash.is_empty())
            || (self.source_event_sequence > 0 && !valid_blake3_hex(&self.source_event_hash))
        {
            return Err(DraftGridClosureError::Invalid(
                "package identity, authority, frontier, or trust roots are invalid".into(),
            ));
        }
        if self.grid.grid_id != self.root_aggregate_id || self.grid.anchored {
            return Err(DraftGridClosureError::Unsupported(
                "the package root must be one ordinary unanchored grid".into(),
            ));
        }
        validate_resource_bounds(self)?;
        validate_package_grid(self)?;
        validate_package_players(self)?;
        validate_package_cargo(self)?;
        validate_inventory_uniqueness(self)?;
        validate_package_queues(self)?;
        validate_production_job_origins(
            &self.universe_id,
            &self.source_cell_id,
            self.source_event_sequence,
            &self.production_queues,
            &self.production_job_origins,
        )?;
        validate_package_contacts(self)?;
        validate_destination_containment(self)?;

        let expected_conservation = calculate_conservation(
            &self.grid,
            &self.cargo_inventories,
            &self.production_queues,
            &self.players,
            &self.active_internal_contacts,
            &self.members,
        )?;
        if self.conservation != expected_conservation
            || self.closure_root != self.calculate_closure_root()?
            || self.conservation_root != self.calculate_conservation_root()?
            || self.package_hash != self.calculate_package_hash()?
            || !valid_blake3_hex(&self.closure_root)
            || !valid_blake3_hex(&self.conservation_root)
            || !valid_blake3_hex(&self.package_hash)
        {
            return Err(DraftGridClosureError::Invalid(
                "closure, conservation, or package commitment does not match the exact contents"
                    .into(),
            ));
        }
        Ok(())
    }

    fn validate_manifest_v5<'package, 'manifest>(
        &'package self,
        manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
    ) -> Result<ValidatedDraftGridClosurePackageV2<'package, 'manifest>, DraftGridClosureError>
    {
        self.validate_wire()?;
        let document = manifest.document();
        if self.universe_id != manifest.universe_id()
            || self.universe_manifest_hash != manifest.manifest_hash()
            || self.celestial_registry_hash != document.celestial_registry_hash
            || self.content_manifest_version != document.compatibility.content_manifest_version
            || self.schema_version != document.compatibility.transfer_package_schema_version
            || self.receipt_schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
        {
            return Err(DraftGridClosureError::Invalid(
                "package-v2 does not match the complete validated manifest-5 identity".into(),
            ));
        }
        Ok(ValidatedDraftGridClosurePackageV2 {
            package: self,
            manifest,
        })
    }
}

fn validate_package_grid(package: &DraftGridClosurePackageV2) -> Result<(), DraftGridClosureError> {
    validate_grid_address_in_destination(&package.grid, &package.destination_cell_key)?;
    let source_origin = celestial::cell_address_from_key(&package.source_cell_key)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    if !valid_stable_id(&package.grid.grid_id)
        || !valid_stable_id(&package.grid.owner_player_id)
        || !celestial::local_position_from_address(&source_origin, &package.grid.address)
            .is_ok_and(|position| position == package.grid.position)
        || !finite_vec(package.grid.position)
        || !finite_vec(package.grid.linear_velocity)
        || !finite_vec(package.grid.angular_velocity)
        || !finite_vec(package.grid.control_linear_input)
        || !finite_vec(package.grid.control_angular_input)
        || !normalized_quat(package.grid.orientation)
    {
        return Err(DraftGridClosureError::Invalid(
            "grid identity, canonical pose, motion, or controls are invalid".into(),
        ));
    }
    let mut coordinates = BTreeSet::new();
    for (block_id, block) in &package.grid.blocks {
        let definition = content::block(block.kind);
        if block_id != &block.block_id
            || !valid_stable_id(block_id)
            || !coordinates.insert(block.coordinate)
            || block.orientation > 3
            || block.component_cost != definition.component_cost
            || block.health == 0
            || block.health > definition.max_health
            || matches!(block.kind, verse_protocol::BlockKind::Cargo)
                != block.inventory_id.is_some()
        {
            return Err(DraftGridClosureError::Invalid(
                "block identity, topology, orientation, integrity, or cargo linkage is invalid"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn extract_draft_grid_closure(
    source: &WorldState,
    grid_id: &str,
    context: &DraftGridTransferContextV2,
) -> Result<DraftGridClosurePackageV2, DraftGridClosureError> {
    source
        .validate_player_roster()
        .map_err(DraftGridClosureError::Invalid)?;
    extract_draft_grid_closure_from_validated_world(source, grid_id, context)
}

fn extract_draft_grid_closure_from_validated_world(
    source: &WorldState,
    grid_id: &str,
    context: &DraftGridTransferContextV2,
) -> Result<DraftGridClosurePackageV2, DraftGridClosureError> {
    context.placement.validate().map_err(|source| {
        DraftGridClosureError::Invalid(format!("placement bundle is invalid: {source}"))
    })?;
    validate_adjacent_cells(
        &context.placement.source_cell_key,
        &context.placement.destination_cell_key,
    )?;
    let source_key = celestial::cell_key_from_address(&source.cell_address)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    if !valid_stable_id(&context.transfer_id)
        || context.source_assignment_generation == 0
        || context.destination_assignment_generation == 0
        || context.source_fencing_token == 0
        || context.destination_fencing_token == 0
        || source_key != context.placement.source_cell_key
        || source.cell_id != context.placement.source_cell_id
        || source.fencing_token != context.source_fencing_token
        || grid_id != context.placement.root_aggregate_id
    {
        return Err(DraftGridClosureError::Invalid(
            "source cell, fence, assignment, transfer, or root context is stale".into(),
        ));
    }
    let grid = source
        .grids
        .get(grid_id)
        .ok_or_else(|| DraftGridClosureError::Invalid("source grid is not resident".into()))?
        .clone();
    if grid.anchored {
        return Err(DraftGridClosureError::Unsupported(
            "anchored grids cannot enter the bounded handoff".into(),
        ));
    }
    if grid.blocks.is_empty() {
        return Err(DraftGridClosureError::Unsupported(
            "a grid closure must retain at least one block".into(),
        ));
    }

    let mut cargo_inventories = BTreeMap::new();
    for block in grid.blocks.values() {
        if let Some(inventory_id) = &block.inventory_id {
            let inventory = source.inventories.get(inventory_id).ok_or_else(|| {
                DraftGridClosureError::Invalid(format!("cargo inventory {inventory_id} is missing"))
            })?;
            if inventory.domain
                != (InventoryDomain::Cargo {
                    block_id: block.block_id.clone(),
                })
            {
                return Err(DraftGridClosureError::Invalid(
                    "cargo block and inventory backlink do not agree".into(),
                ));
            }
            cargo_inventories.insert(inventory_id.clone(), inventory.clone());
        }
    }

    let mut production_queues = BTreeMap::new();
    for block_id in grid.blocks.keys() {
        if let Some(queue) = source.production_queues.get(block_id) {
            production_queues.insert(block_id.clone(), queue.clone());
        }
    }

    let owner = source.player.get(&grid.owner_player_id).ok_or_else(|| {
        DraftGridClosureError::Unsupported(
            "the grid owner must be resident so ownership and rewards move atomically".into(),
        )
    })?;
    if owner
        .locomotion
        .support
        .as_ref()
        .is_some_and(|support| support.body_id != grid_id)
    {
        return Err(DraftGridClosureError::Unsupported(
            "the grid owner has a support edge outside the transfer closure".into(),
        ));
    }
    let mut player_ids = BTreeSet::from([owner.player_id.clone()]);
    for (player_id, player) in source.player.iter() {
        let supported = player_is_supported_by_grid(player, grid_id);
        if player
            .locomotion
            .support
            .as_ref()
            .is_some_and(|support| support.body_id == grid_id)
            && !supported
        {
            return Err(DraftGridClosureError::Unsupported(
                "a grid support edge has an unsupported locomotion mode".into(),
            ));
        }
        if supported {
            player_ids.insert(player_id.clone());
        }
    }
    if !production_queues.is_empty() && !player_is_supported_by_grid(owner, grid_id) {
        return Err(DraftGridClosureError::Unsupported(
            "a queue-bearing grid requires its owner to ride the transferring closure".into(),
        ));
    }

    let mut players = BTreeMap::new();
    for player_id in player_ids {
        let source_player = source
            .player
            .get(&player_id)
            .expect("derived closure player remains present")
            .clone();
        let inventory = source
            .inventories
            .get(&source_player.inventory_id)
            .ok_or_else(|| {
                DraftGridClosureError::Invalid(format!(
                    "carried inventory {} is missing",
                    source_player.inventory_id
                ))
            })?
            .clone();
        let destination_player =
            destination_player_v2(&source_player, &context.placement.destination_cell_key)?;
        players.insert(
            player_id.clone(),
            DraftClosurePlayerV2 {
                source_player,
                destination_player,
                inventory,
                operation_history: source.processed_operations.get(&player_id).cloned(),
                is_owner: player_id == grid.owner_player_id,
                is_supported_rider: source
                    .player
                    .get(&player_id)
                    .is_some_and(|player| player_is_supported_by_grid(player, grid_id)),
            },
        );
    }

    let inventory_ids = cargo_inventories
        .keys()
        .chain(
            players
                .values()
                .map(|player| &player.inventory.inventory_id),
        )
        .cloned()
        .collect::<BTreeSet<_>>();
    for queue in production_queues.values() {
        for job in queue {
            if !inventory_ids.contains(&job.source_inventory_id)
                || !inventory_ids.contains(&job.destination_inventory_id)
            {
                return Err(DraftGridClosureError::Unsupported(
                    "a production queue has an inventory edge outside the transfer closure".into(),
                ));
            }
        }
    }
    let closure_player_ids = players.keys().cloned().collect::<BTreeSet<_>>();
    let closure_job_ids = production_queues
        .values()
        .flatten()
        .map(|job| &job.job_id)
        .collect::<BTreeSet<_>>();
    for (machine_block_id, queue) in &source.production_queues {
        if grid.blocks.contains_key(machine_block_id) {
            continue;
        }
        for job in queue {
            if inventory_ids.contains(&job.source_inventory_id)
                || inventory_ids.contains(&job.destination_inventory_id)
                || closure_player_ids.contains(&job.owner_player_id)
                || closure_job_ids.contains(&job.job_id)
            {
                return Err(DraftGridClosureError::Unsupported(
                    "an excluded production queue retains an edge into the transfer closure".into(),
                ));
            }
        }
    }
    if source
        .player_transfer_locks
        .keys()
        .any(|player_id| closure_player_ids.contains(player_id))
        || source
            .player_transfer_locks
            .values()
            .any(|lock| lock.transfer_id == context.transfer_id)
        || source
            .player_transfer_reservations
            .values()
            .any(|reservation| {
                reservation.transfer_id == context.transfer_id
                    || closure_player_ids.contains(&reservation.player_id)
                    || inventory_ids.contains(&reservation.inventory_id)
            })
        || source.transfer_witnesses.contains_key(&context.transfer_id)
    {
        return Err(DraftGridClosureError::Unsupported(
            "the closure or transfer ID already participates in another durable handoff".into(),
        ));
    }

    let active_internal_contacts =
        derive_internal_contacts(source, grid_id, players.keys().map(String::as_str))?;
    validate_context_members(context, grid_id, players.keys().map(String::as_str))?;

    let conservation = calculate_conservation(
        &grid,
        &cargo_inventories,
        &production_queues,
        &players,
        &active_internal_contacts,
        &context.placement.members,
    )?;
    let mut package = DraftGridClosurePackageV2 {
        schema_version: DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION,
        receipt_schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
        transfer_id: context.transfer_id.clone(),
        aggregate_kind: MobileAggregateKind::Grid,
        root_aggregate_id: grid_id.to_owned(),
        source_cell_key: context.placement.source_cell_key.clone(),
        source_cell_id: context.placement.source_cell_id.clone(),
        destination_cell_key: context.placement.destination_cell_key.clone(),
        destination_cell_id: context.placement.destination_cell_id.clone(),
        source_assignment_generation: context.source_assignment_generation,
        destination_assignment_generation: context.destination_assignment_generation,
        source_fencing_token: context.source_fencing_token,
        destination_fencing_token: context.destination_fencing_token,
        members: context.placement.members.clone(),
        member_root: context.placement.member_root.clone(),
        universe_id: source.universe_id.clone(),
        universe_manifest_hash: source.universe_manifest_hash.clone(),
        celestial_registry_hash: source.celestial_registry_hash.clone(),
        content_manifest_version: source.content_manifest_version.clone(),
        source_event_sequence: source.event_sequence,
        source_event_hash: source.last_event_hash.clone(),
        source_world_hash: source.state_hash(),
        prepared_at_simulation_tick: source.simulation_tick,
        grid,
        cargo_inventories,
        production_queues,
        production_job_origins: context.production_job_origins.clone(),
        players,
        active_internal_contacts,
        conservation,
        closure_root: String::new(),
        conservation_root: String::new(),
        package_hash: String::new(),
    };
    package.closure_root = package.calculate_closure_root()?;
    package.conservation_root = package.calculate_conservation_root()?;
    package.package_hash = package.calculate_package_hash()?;
    package.validate_wire()?;
    if serde_json::to_vec(&package)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?
        .len()
        > MAX_DRAFT_GRID_TRANSFER_PACKAGE_BYTES
    {
        return Err(DraftGridClosureError::TooLarge);
    }
    Ok(package)
}

fn validate_source_closure(
    source: &WorldState,
    context: &DraftGridTransferContextV2,
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    package.validate_wire()?;
    let current = extract_draft_grid_closure(source, &package.root_aggregate_id, context)?;
    if current != *package {
        return Err(DraftGridClosureError::Changed(
            "the authoritative source no longer matches the captured package".into(),
        ));
    }
    Ok(())
}

fn validate_context_members<'a>(
    context: &DraftGridTransferContextV2,
    grid_id: &str,
    player_ids: impl Iterator<Item = &'a str>,
) -> Result<(), DraftGridClosureError> {
    let mut expected = BTreeMap::from([(grid_id.to_owned(), MobileAggregateKind::Grid)]);
    for player_id in player_ids {
        expected.insert(player_id.to_owned(), MobileAggregateKind::Player);
    }
    let actual = context
        .placement
        .members
        .iter()
        .map(|member| (member.aggregate_id.clone(), member.aggregate_kind))
        .collect::<BTreeMap<_, _>>();
    if expected != actual || expected.len() != context.placement.members.len() {
        return Err(DraftGridClosureError::Invalid(
            "placement members do not exactly match the server-derived grid and riders".into(),
        ));
    }
    Ok(())
}

fn validate_package_players(
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let member_kinds = package
        .members
        .iter()
        .map(|member| (member.aggregate_id.as_str(), member.aggregate_kind))
        .collect::<BTreeMap<_, _>>();
    if member_kinds.get(package.root_aggregate_id.as_str()) != Some(&MobileAggregateKind::Grid)
        || package.players.is_empty()
    {
        return Err(DraftGridClosureError::Invalid(
            "the closure must contain its grid root and at least the resident owner".into(),
        ));
    }
    let source_origin = celestial::cell_address_from_key(&package.source_cell_key)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    let mut owner_count = 0_usize;
    for (player_id, player) in &package.players {
        if !valid_player_id(player_id)
            || player_id != &player.source_player.player_id
            || player_id != &player.destination_player.player_id
            || member_kinds.get(player_id.as_str()) != Some(&MobileAggregateKind::Player)
            || player.inventory.inventory_id != player.source_player.inventory_id
            || player.inventory.domain
                != (InventoryDomain::Player {
                    player_id: player_id.clone(),
                })
            || player.inventory.capacity_liters == 0
            || checked_inventory_used_liters(&player.inventory)? > player.inventory.capacity_liters
            || player.is_owner != (*player_id == package.grid.owner_player_id)
            || player.is_supported_rider
                != player_is_supported_by_grid(&player.source_player, &package.grid.grid_id)
            || player.destination_player
                != destination_player_v2(&player.source_player, &package.destination_cell_key)?
            || !celestial::local_position_from_address(
                &source_origin,
                &player.source_player.address,
            )
            .is_ok_and(|position| position == player.source_player.position)
            || !finite_player(&player.source_player)
            || (!player.is_owner && !player.is_supported_rider)
            || player
                .source_player
                .locomotion
                .support
                .as_ref()
                .is_some_and(|support| support.body_id != package.grid.grid_id)
        {
            return Err(DraftGridClosureError::Invalid(
                "player identity, inventory, role, support, or destination frontier is invalid"
                    .into(),
            ));
        }
        if player.is_owner {
            owner_count += 1;
        }
        validate_operation_history(
            player_id,
            player.operation_history.as_ref(),
            &package.source_cell_id,
            package.source_event_sequence,
        )?;
        if player.is_supported_rider {
            let support = player
                .source_player
                .locomotion
                .support
                .as_ref()
                .expect("supported-rider predicate requires support");
            if !package.grid.blocks.contains_key(&support.collider_id) {
                return Err(DraftGridClosureError::Unsupported(
                    "a rider support collider is not a block in the transferred grid".into(),
                ));
            }
        }
    }
    if owner_count != 1
        || (!package.production_queues.is_empty()
            && !package
                .players
                .get(&package.grid.owner_player_id)
                .is_some_and(|player| player.is_supported_rider))
        || member_kinds.len() != package.players.len() + 1
    {
        return Err(DraftGridClosureError::Unsupported(
            "owner, rider, queue, or placement membership closure is incomplete".into(),
        ));
    }
    Ok(())
}

fn finite_player(player: &Player) -> bool {
    finite_vec(player.position)
        && finite_vec(player.linear_velocity)
        && finite_vec(player.angular_velocity)
        && finite_vec(player.control_linear_input)
        && finite_vec(player.control_angular_input)
        && finite_vec(player.locomotion.up)
        && player.locomotion.view_pitch_radians.is_finite()
        && normalized_quat(player.orientation)
        && player.locomotion.support.as_ref().is_none_or(|support| {
            valid_stable_id(&support.body_id)
                && valid_stable_id(&support.collider_id)
                && finite_vec(support.local_anchor)
                && finite_vec(support.local_normal)
        })
        && player.pending_control_frames.iter().all(|frame| {
            finite_vec(frame.linear_input)
                && finite_vec(frame.angular_input)
                && frame.input_sequence > player.last_processed_input_sequence
                && frame.input_sequence <= player.last_received_input_sequence
        })
}

fn validate_operation_history(
    player_id: &str,
    history: Option<&ActorOperationHistory>,
    source_cell_id: &str,
    source_event_sequence: u64,
) -> Result<(), DraftGridClosureError> {
    let Some(history) = history else {
        return Ok(());
    };
    let retained_bytes = serde_json::to_vec(&history.retained)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?
        .len();
    if history.committed_through == 0
        || history.compacted_through > history.committed_through
        || history.retained.len() > PROCESSED_OPERATION_RETENTION_LIMIT
        || retained_bytes > PROCESSED_OPERATION_RETAINED_BYTES_LIMIT
        || (history.compacted_through == 0 && !history.compacted_history_hash.is_empty())
        || (history.compacted_through > 0 && !valid_blake3_hex(&history.compacted_history_hash))
    {
        return Err(DraftGridClosureError::Invalid(
            "operation history frontier or compacted commitment is invalid".into(),
        ));
    }
    let mut prior = history.compacted_through;
    let mut last_event_by_cell = BTreeMap::new();
    for (sequence, record) in &history.retained {
        let expected = prior.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported("operation sequence is exhausted".into())
        })?;
        if *sequence != expected
            || !valid_stable_id(&record.operation_id)
            || !valid_blake3_hex(&record.intent_fingerprint)
            || !valid_blake3_hex(&record.receipt_origin_cell_id)
            || serde_json::to_vec(record)
                .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?
                .len()
                > PROCESSED_OPERATION_RECORD_BYTES_LIMIT
            || record.receipt.operation_sequence != *sequence
            || record.receipt.operation_id != record.operation_id
            || record.receipt.event_sequence == 0
            || record.receipt.code.trim().is_empty()
            || (record.receipt_origin_cell_id == source_cell_id
                && record.receipt.event_sequence > source_event_sequence)
            || last_event_by_cell
                .get(&record.receipt_origin_cell_id)
                .is_some_and(|prior| record.receipt.event_sequence <= *prior)
        {
            return Err(DraftGridClosureError::Invalid(format!(
                "operation history for {player_id} is not a canonical contiguous frontier"
            )));
        }
        prior = *sequence;
        last_event_by_cell.insert(
            record.receipt_origin_cell_id.clone(),
            record.receipt.event_sequence,
        );
    }
    if prior != history.committed_through {
        return Err(DraftGridClosureError::Invalid(
            "operation history retained suffix does not reach its committed frontier".into(),
        ));
    }
    Ok(())
}

fn finite_vec(value: Vec3) -> bool {
    value.x.is_finite() && value.y.is_finite() && value.z.is_finite()
}

fn normalized_quat(orientation: verse_protocol::Quat) -> bool {
    let length_squared = f64::from(orientation.x).mul_add(
        f64::from(orientation.x),
        f64::from(orientation.y).mul_add(
            f64::from(orientation.y),
            f64::from(orientation.z).mul_add(
                f64::from(orientation.z),
                f64::from(orientation.w) * f64::from(orientation.w),
            ),
        ),
    );
    orientation.is_finite() && (length_squared - 1.0).abs() <= 1.0e-3
}

fn validate_package_cargo(
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let mut expected = BTreeMap::new();
    for block in package.grid.blocks.values() {
        if let Some(inventory_id) = &block.inventory_id
            && expected
                .insert(inventory_id.clone(), block.block_id.clone())
                .is_some()
        {
            return Err(DraftGridClosureError::Invalid(
                "two cargo blocks cannot share one inventory".into(),
            ));
        }
    }
    if expected.len() != package.cargo_inventories.len() {
        return Err(DraftGridClosureError::Invalid(
            "cargo closure has a missing or extra inventory".into(),
        ));
    }
    for (inventory_id, block_id) in expected {
        let inventory = package
            .cargo_inventories
            .get(&inventory_id)
            .ok_or_else(|| {
                DraftGridClosureError::Invalid("cargo inventory closure is incomplete".into())
            })?;
        if inventory.inventory_id != inventory_id
            || inventory.domain != (InventoryDomain::Cargo { block_id })
            || inventory.capacity_liters == 0
            || checked_inventory_used_liters(inventory)? > inventory.capacity_liters
        {
            return Err(DraftGridClosureError::Invalid(
                "cargo inventory identity, backlink, contents, or capacity is invalid".into(),
            ));
        }
    }
    Ok(())
}

fn validate_inventory_uniqueness(
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let mut inventory_ids = package
        .cargo_inventories
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    for player in package.players.values() {
        if !inventory_ids.insert(player.inventory.inventory_id.clone()) {
            return Err(DraftGridClosureError::Invalid(
                "cargo and rider inventory identities must be globally unique".into(),
            ));
        }
    }
    let expected_count = package
        .cargo_inventories
        .len()
        .checked_add(package.players.len())
        .ok_or_else(|| {
            DraftGridClosureError::Unsupported("inventory subject count overflowed".into())
        })?;
    if inventory_ids.len() != expected_count {
        return Err(DraftGridClosureError::Invalid(
            "inventory identity cardinality does not match the closure".into(),
        ));
    }
    Ok(())
}

fn validate_package_queues(
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let inventory_ids = package
        .cargo_inventories
        .keys()
        .chain(
            package
                .players
                .values()
                .map(|player| &player.inventory.inventory_id),
        )
        .collect::<BTreeSet<_>>();
    let mut job_ids = BTreeSet::new();
    for (machine_block_id, queue) in &package.production_queues {
        let machine = package.grid.blocks.get(machine_block_id).ok_or_else(|| {
            DraftGridClosureError::Unsupported(
                "a transferred production queue is not on the transferred grid".into(),
            )
        })?;
        if queue.is_empty()
            || queue.len() > content::manifest().production.queue_limit_per_machine
            || !machine.is_complete()
            || !matches!(
                machine.kind,
                verse_protocol::BlockKind::Refinery | verse_protocol::BlockKind::Assembler
            )
        {
            return Err(DraftGridClosureError::Invalid(
                "production queue machine, size, or state is invalid".into(),
            ));
        }
        for (queue_index, job) in queue.iter().enumerate() {
            let (expected_inputs, expected_outputs, expected_duration) =
                production_recipe_quantities(job.recipe, job.batches).ok_or_else(|| {
                    DraftGridClosureError::Invalid(
                        "production recipe quantities overflowed or have zero batches".into(),
                    )
                })?;
            let escrow_valid = if job.progress_ticks < job.duration_ticks {
                job.reserved_inputs == expected_inputs
                    && job.pending_outputs == InventoryContents::default()
            } else {
                job.progress_ticks == job.duration_ticks
                    && job.reserved_inputs == InventoryContents::default()
                    && job.pending_outputs == expected_outputs
            };
            if job.machine_block_id != *machine_block_id
                || job.owner_player_id != package.grid.owner_player_id
                || job.content_manifest_version != package.content_manifest_version
                || !valid_stable_id(&job.job_id)
                || !valid_stable_id(&job.operation_id)
                || !job_ids.insert(job.job_id.as_str())
                || !inventory_ids.contains(&job.source_inventory_id)
                || !inventory_ids.contains(&job.destination_inventory_id)
                || closure_inventory_owner(package, &job.source_inventory_id)
                    != Some(package.grid.owner_player_id.as_str())
                || closure_inventory_owner(package, &job.destination_inventory_id)
                    != Some(package.grid.owner_player_id.as_str())
                || !content::machine_supports_recipe(machine.kind, job.recipe)
                || job.duration_ticks != expected_duration
                || job.progress_ticks > job.duration_ticks
                || (queue_index > 0 && job.progress_ticks != 0)
                || job.queued_event_sequence == 0
                || !escrow_valid
            {
                return Err(DraftGridClosureError::Unsupported(
                    "production identity, owner, or inventory endpoint leaves the closure".into(),
                ));
            }
        }
    }
    Ok(())
}

fn closure_inventory_owner<'a>(
    package: &'a DraftGridClosurePackageV2,
    inventory_id: &str,
) -> Option<&'a str> {
    if package.cargo_inventories.contains_key(inventory_id) {
        return Some(&package.grid.owner_player_id);
    }
    package
        .players
        .values()
        .find(|player| player.inventory.inventory_id == inventory_id)
        .map(|player| player.source_player.player_id.as_str())
}

fn validate_package_contacts(
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let mut bodies = BTreeMap::from([(
        package.grid.grid_id.clone(),
        package.grid.blocks.keys().cloned().collect::<BTreeSet<_>>(),
    )]);
    for player_id in package.players.keys() {
        bodies.insert(
            player_body_id_v2(player_id),
            BTreeSet::from([player_collider_id_v2(player_id)]),
        );
    }
    for contact in &package.active_internal_contacts {
        if (&contact.body_b, &contact.collider_b) < (&contact.body_a, &contact.collider_a)
            || !bodies
                .get(&contact.body_a)
                .is_some_and(|colliders| colliders.contains(&contact.collider_a))
            || !bodies
                .get(&contact.body_b)
                .is_some_and(|colliders| colliders.contains(&contact.collider_b))
        {
            return Err(DraftGridClosureError::Invalid(
                "internal contacts must use canonical closure body and collider identities".into(),
            ));
        }
    }
    Ok(())
}

fn validate_resource_bounds(
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let jobs = package
        .production_queues
        .values()
        .try_fold(0_usize, |total, queue| total.checked_add(queue.len()))
        .ok_or_else(|| {
            DraftGridClosureError::Unsupported("production job count overflowed".into())
        })?;
    if package.members.is_empty()
        || package.members.len() > MAX_DRAFT_GRID_MEMBERS
        || package.grid.blocks.is_empty()
        || package.grid.blocks.len() > MAX_DRAFT_GRID_BLOCKS
        || package.cargo_inventories.len() > MAX_DRAFT_GRID_CARGO_INVENTORIES
        || package.production_queues.len() > MAX_DRAFT_GRID_PRODUCTION_QUEUES
        || jobs > MAX_DRAFT_GRID_PRODUCTION_JOBS
        || package.active_internal_contacts.len() > MAX_DRAFT_GRID_CONTACTS
    {
        return Err(DraftGridClosureError::Unsupported(
            "the closure exceeds a bounded handoff execution budget".into(),
        ));
    }
    Ok(())
}

fn derive_internal_contacts<'a>(
    source: &WorldState,
    grid_id: &str,
    player_ids: impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<ContactPairKey>, DraftGridClosureError> {
    let mut closure_bodies = BTreeSet::from([grid_id.to_owned()]);
    closure_bodies.extend(player_ids.map(player_body_id_v2));
    let mut contacts = BTreeSet::new();
    for contact in &source.active_contact_pairs {
        let a_inside = closure_bodies.contains(&contact.body_a);
        let b_inside = closure_bodies.contains(&contact.body_b);
        if a_inside ^ b_inside {
            return Err(DraftGridClosureError::Unsupported(
                "the grid closure has an active external physics contact".into(),
            ));
        }
        if a_inside && b_inside {
            contacts.insert(contact.clone());
        }
    }
    Ok(contacts)
}

fn player_is_supported_by_grid(player: &Player, grid_id: &str) -> bool {
    matches!(
        player.locomotion.kind,
        LocomotionKind::Grounded | LocomotionKind::Magnetic
    ) && player
        .locomotion
        .support
        .as_ref()
        .is_some_and(|support| support.body_id == grid_id)
}

fn player_body_id_v2(player_id: &str) -> String {
    format!("player-body-{player_id}")
}

fn player_collider_id_v2(player_id: &str) -> String {
    format!("player-collider-{player_id}")
}

fn validate_adjacent_cells(
    source: &CellKeyV1,
    destination: &CellKeyV1,
) -> Result<(), DraftGridClosureError> {
    celestial::validate_cell_key(source)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    celestial::validate_cell_key(destination)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    if source.universe_id != destination.universe_id {
        return Err(DraftGridClosureError::Invalid(
            "source and destination cells belong to different universes".into(),
        ));
    }
    let adjacent = [
        [1, 0, 0],
        [-1, 0, 0],
        [0, 1, 0],
        [0, -1, 0],
        [0, 0, 1],
        [0, 0, -1],
    ]
    .into_iter()
    .any(|offset| celestial::neighbor_cell_key(source, offset).as_ref() == Ok(destination));
    if !adjacent {
        return Err(DraftGridClosureError::Unsupported(
            "the bounded proof transfers only across one shared cell face".into(),
        ));
    }
    Ok(())
}

fn validate_grid_address_in_destination(
    grid: &Grid,
    destination: &CellKeyV1,
) -> Result<(), DraftGridClosureError> {
    let address_cell = celestial::cell_key_from_address(&grid.address)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    if &address_cell != destination {
        return Err(DraftGridClosureError::Unsupported(
            "the grid canonical reference point has not entered the destination cell".into(),
        ));
    }
    Ok(())
}

fn destination_player_v2(
    source: &Player,
    destination_cell_key: &CellKeyV1,
) -> Result<Player, DraftGridClosureError> {
    let destination_origin = celestial::cell_address_from_key(destination_cell_key)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    let mut destination = source.clone();
    destination.position =
        celestial::local_position_from_address(&destination_origin, &destination.address)
            .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    destination.movement_epoch = destination.movement_epoch.checked_add(1).ok_or_else(|| {
        DraftGridClosureError::Unsupported("a rider movement epoch is exhausted".into())
    })?;
    destination.last_processed_input_sequence = destination.last_received_input_sequence;
    destination.pending_control_frames.clear();
    destination.control_linear_input = Vec3::ZERO;
    destination.control_angular_input = Vec3::ZERO;
    destination.boost = false;
    destination.jump = false;
    destination.control_expires_at_simulation_tick = 0;
    Ok(destination)
}

fn validate_destination_containment(
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let destination_origin = celestial::cell_address_from_key(&package.destination_cell_key)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    let grid_position =
        celestial::local_position_from_address(&destination_origin, &package.grid.address)
            .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    for block in package.grid.blocks.values() {
        for x in [-0.5, 0.5] {
            for y in [-0.5, 0.5] {
                for z in [-0.5, 0.5] {
                    let local = Vec3::new(
                        f64::from(block.coordinate.x) + x,
                        f64::from(block.coordinate.y) + y,
                        f64::from(block.coordinate.z) + z,
                    );
                    let corner = grid_position + package.grid.orientation.rotate(local);
                    if !point_inside_cell_outward(corner)? {
                        return Err(DraftGridClosureError::Unsupported(
                            "a rotated block collider crosses the destination cell boundary".into(),
                        ));
                    }
                }
            }
        }
    }

    let character = &content::manifest().character;
    let radius = character.collision_radius_m;
    let half_height = (character.standing_height_m - 2.0 * radius) * 0.5;
    if !radius.is_finite() || !half_height.is_finite() || radius <= 0.0 || half_height < 0.0 {
        return Err(DraftGridClosureError::Invalid(
            "character capsule dimensions are invalid".into(),
        ));
    }
    for player in package.players.values() {
        let center = celestial::local_position_from_address(
            &destination_origin,
            &player.source_player.address,
        )
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
        let axis = player
            .source_player
            .orientation
            .rotate(Vec3::new(0.0, 1.0, 0.0));
        let extent = Vec3::new(
            axis.x.abs() * half_height + radius,
            axis.y.abs() * half_height + radius,
            axis.z.abs() * half_height + radius,
        );
        if !axis_is_contained_outward(center.x - extent.x, center.x + extent.x)?
            || !axis_is_contained_outward(center.y - extent.y, center.y + extent.y)?
            || !axis_is_contained_outward(center.z - extent.z, center.z + extent.z)?
        {
            return Err(DraftGridClosureError::Unsupported(
                "a rider capsule crosses the destination cell boundary".into(),
            ));
        }
    }
    Ok(())
}

fn point_inside_cell_outward(point: Vec3) -> Result<bool, DraftGridClosureError> {
    Ok(axis_is_contained_outward(point.x, point.x)?
        && axis_is_contained_outward(point.y, point.y)?
        && axis_is_contained_outward(point.z, point.z)?)
}

fn axis_is_contained_outward(lower: f64, upper: f64) -> Result<bool, DraftGridClosureError> {
    if !lower.is_finite() || !upper.is_finite() || lower > upper {
        return Err(DraftGridClosureError::Invalid(
            "collider bounds are non-finite or inverted".into(),
        ));
    }
    let canonical_minimum = outward_micrometres(lower, f64::floor)?;
    let canonical_maximum = outward_micrometres(upper, f64::ceil)?;
    let half_cell_um = i128::from(celestial::CELL_EDGE_UM / 2);
    Ok(canonical_minimum >= -half_cell_um && canonical_maximum < half_cell_um)
}

fn outward_micrometres(meters: f64, round: fn(f64) -> f64) -> Result<i128, DraftGridClosureError> {
    let scaled = meters * MICROMETRES_PER_METRE;
    if !scaled.is_finite() || scaled < i128::MIN as f64 || scaled > i128::MAX as f64 {
        return Err(DraftGridClosureError::Unsupported(
            "collider bounds cannot be represented in canonical micrometres".into(),
        ));
    }
    Ok(round(scaled) as i128)
}

fn validate_destination_conflicts(
    destination: &WorldState,
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    package.validate_wire()?;
    destination
        .validate_player_roster()
        .map_err(DraftGridClosureError::Invalid)?;
    let destination_key = celestial::cell_key_from_address(&destination.cell_address)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    if destination_key != package.destination_cell_key
        || destination.cell_id != package.destination_cell_id
        || destination.universe_id != package.universe_id
        || destination.universe_manifest_hash != package.universe_manifest_hash
        || destination.celestial_registry_hash != package.celestial_registry_hash
        || destination.content_manifest_version != package.content_manifest_version
        || destination.fencing_token != package.destination_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "destination cell, fence, or trust roots do not match the package".into(),
        ));
    }
    validate_destination_identity_conflicts(destination, package)
}

/// Conflict-only world-21 destination gate.
///
/// The caller must first validate the package, the origin-aware draft cell
/// envelope, and the directory authority. This helper deliberately neither
/// re-runs world-20's local-only production frontier nor pins the destination
/// to the package's historical fence.
fn validate_destination_conflicts_in_validated_world_v21(
    destination: &WorldState,
    package: &DraftGridClosurePackageV2,
    live_destination_fencing_token: u64,
) -> Result<(), DraftGridClosureError> {
    let destination_key = celestial::cell_key_from_address(&destination.cell_address)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    if destination_key != package.destination_cell_key
        || destination.cell_id != package.destination_cell_id
        || destination.universe_id != package.universe_id
        || destination.universe_manifest_hash != package.universe_manifest_hash
        || destination.celestial_registry_hash != package.celestial_registry_hash
        || destination.content_manifest_version != package.content_manifest_version
        || live_destination_fencing_token < package.destination_fencing_token
        || destination.fencing_token != live_destination_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "validated world-21 destination identity, live fence, or trust roots disagree".into(),
        ));
    }
    validate_destination_identity_conflicts(destination, package)
}

fn validate_destination_identity_conflicts(
    destination: &WorldState,
    package: &DraftGridClosurePackageV2,
) -> Result<(), DraftGridClosureError> {
    let block_ids = package.grid.blocks.keys().collect::<BTreeSet<_>>();
    let inventory_ids = package
        .cargo_inventories
        .keys()
        .chain(
            package
                .players
                .values()
                .map(|player| &player.inventory.inventory_id),
        )
        .collect::<BTreeSet<_>>();
    let job_ids = package
        .production_queues
        .values()
        .flatten()
        .map(|job| &job.job_id)
        .collect::<BTreeSet<_>>();
    let destination_job_ids = destination
        .production_queues
        .values()
        .flatten()
        .map(|job| &job.job_id)
        .collect::<BTreeSet<_>>();
    let closure_body_ids = std::iter::once(package.grid.grid_id.clone())
        .chain(
            package
                .players
                .keys()
                .map(|player_id| player_body_id_v2(player_id)),
        )
        .collect::<BTreeSet<_>>();
    let resident_player_body_ids = destination
        .player
        .iter()
        .map(|(player_id, _)| player_body_id_v2(player_id))
        .collect::<BTreeSet<_>>();
    let resident_voxel_body_ids = destination
        .voxels
        .occupied
        .iter()
        .copied()
        .map(voxel_collision_chunk_coordinate)
        .map(voxel_collision_chunk_body_id)
        .collect::<BTreeSet<_>>();
    if destination
        .grids
        .keys()
        .any(|grid_id| closure_body_ids.contains(grid_id))
        || resident_player_body_ids.contains(&package.grid.grid_id)
        || resident_voxel_body_ids.contains(&package.grid.grid_id)
        || package.grid.grid_id == PLANET_BODY_ID
        || destination
            .grids
            .values()
            .flat_map(|grid| grid.blocks.keys())
            .any(|block_id| block_ids.contains(block_id))
        || destination
            .inventories
            .keys()
            .any(|inventory_id| inventory_ids.contains(inventory_id))
        || destination_job_ids
            .iter()
            .any(|job_id| job_ids.contains(job_id))
        || package.players.keys().any(|player_id| {
            destination.player.get(player_id).is_some()
                || destination.processed_operations.contains_key(player_id)
                || destination.player_transfer_locks.contains_key(player_id)
        })
        || destination
            .player_transfer_locks
            .values()
            .any(|lock| lock.transfer_id == package.transfer_id)
        || destination
            .player_transfer_reservations
            .values()
            .any(|reservation| {
                reservation.transfer_id == package.transfer_id
                    || package.players.contains_key(&reservation.player_id)
                    || inventory_ids.contains(&reservation.inventory_id)
            })
        || destination
            .transfer_witnesses
            .contains_key(&package.transfer_id)
        || destination.active_contact_pairs.iter().any(|contact| {
            closure_body_ids.contains(&contact.body_a) || closure_body_ids.contains(&contact.body_b)
        })
    {
        return Err(DraftGridClosureError::Unsupported(
            "destination already contains a closure subject or conflicting reservation".into(),
        ));
    }
    Ok(())
}

fn calculate_conservation(
    grid: &Grid,
    cargo_inventories: &BTreeMap<String, InventoryRecord>,
    production_queues: &BTreeMap<String, VecDeque<ProductionJob>>,
    players: &BTreeMap<String, DraftClosurePlayerV2>,
    active_internal_contacts: &BTreeSet<ContactPairKey>,
    members: &[BundledPlacementMember],
) -> Result<DraftGridClosureConservationV2, DraftGridClosureError> {
    let cargo_contents = sum_contents(cargo_inventories.values().map(|value| &value.contents))?;
    let rider_contents = sum_contents(players.values().map(|value| &value.inventory.contents))?;
    let reserved_inputs = sum_contents(
        production_queues
            .values()
            .flatten()
            .map(|job| &job.reserved_inputs),
    )?;
    let pending_outputs = sum_contents(
        production_queues
            .values()
            .flatten()
            .map(|job| &job.pending_outputs),
    )?;
    let inventory_contents = checked_add_contents(&cargo_contents, &rider_contents)?;
    let escrow_contents = checked_add_contents(&reserved_inputs, &pending_outputs)?;
    let transferable_contents = checked_add_contents(&inventory_contents, &escrow_contents)?;
    let installed_components = grid.blocks.values().try_fold(0_u64, |total, block| {
        checked_add(total, block.component_cost, "installed component total")
    })?;

    let cargo_capacity_liters = sum_u64(
        cargo_inventories
            .values()
            .map(|value| value.capacity_liters),
        "cargo capacity",
    )?;
    let rider_capacity_liters = sum_u64(
        players
            .values()
            .map(|value| value.inventory.capacity_liters),
        "rider capacity",
    )?;
    let cargo_used_liters = contents_volume_liters(&cargo_contents)?;
    let rider_used_liters = contents_volume_liters(&rider_contents)?;
    let escrow_volume_liters = contents_volume_liters(&escrow_contents)?;
    let installed_component_volume_liters = checked_mul(
        installed_components,
        resource_unit_volume_liters(verse_protocol::ResourceKind::Component),
        "installed component volume",
    )?;
    let total_resource_volume_liters = sum_u64(
        [
            cargo_used_liters,
            rider_used_liters,
            escrow_volume_liters,
            installed_component_volume_liters,
        ]
        .into_iter(),
        "total closure resource volume",
    )?;

    let cargo_mass_grams = contents_mass_grams_checked(&cargo_contents)?;
    let rider_mass_grams = contents_mass_grams_checked(&rider_contents)?;
    let escrow_mass_grams = contents_mass_grams_checked(&escrow_contents)?;
    let installed_component_mass_grams = checked_mul(
        installed_components,
        resource_unit_mass_grams(verse_protocol::ResourceKind::Component),
        "installed component mass",
    )?;
    let block_mass_grams = grid.blocks.values().try_fold(0_u64, |total, block| {
        let definition = content::block(block.kind);
        let max_health = u64::from(block.max_health());
        let effective_health = u64::from(block.health).max(max_health.div_ceil(10));
        let weighted = u128::from(definition.mass_grams)
            .checked_mul(u128::from(effective_health))
            .ok_or_else(|| {
                DraftGridClosureError::Unsupported("block integrity mass overflowed".into())
            })?
            / u128::from(max_health);
        let weighted = u64::try_from(weighted).map_err(|_| {
            DraftGridClosureError::Unsupported("block integrity mass exceeds u64".into())
        })?;
        checked_add(total, weighted, "grid block mass")
    })?;
    let grid_mass_grams = sum_u64(
        [block_mass_grams, cargo_mass_grams, escrow_mass_grams].into_iter(),
        "grid physical mass",
    )?;
    let closure_mass_grams =
        checked_add(grid_mass_grams, rider_mass_grams, "closure physical mass")?;

    let block_count = len_u64(grid.blocks.len(), "block count")?;
    let cargo_inventory_count = len_u64(cargo_inventories.len(), "cargo inventory count")?;
    let production_queue_count = len_u64(production_queues.len(), "production queue count")?;
    let production_job_count = len_u64(
        production_queues
            .values()
            .try_fold(0_usize, |total, queue| total.checked_add(queue.len()))
            .ok_or_else(|| {
                DraftGridClosureError::Unsupported("production job count overflowed".into())
            })?,
        "production job count",
    )?;
    let player_count = len_u64(players.len(), "player count")?;
    let supported_rider_count = len_u64(
        players
            .values()
            .filter(|player| player.is_supported_rider)
            .count(),
        "supported rider count",
    )?;
    let operation_history_count = len_u64(
        players
            .values()
            .filter(|player| player.operation_history.is_some())
            .count(),
        "operation history count",
    )?;
    let internal_contact_count = len_u64(active_internal_contacts.len(), "contact count")?;
    let placement_member_count = len_u64(members.len(), "placement member count")?;
    let subject_count = sum_u64(
        [
            1,
            block_count,
            cargo_inventory_count,
            production_job_count,
            player_count,
            player_count,
        ]
        .into_iter(),
        "closure subject count",
    )?;

    Ok(DraftGridClosureConservationV2 {
        cargo_contents,
        rider_contents,
        reserved_inputs,
        pending_outputs,
        transferable_contents,
        installed_components,
        cargo_capacity_liters,
        rider_capacity_liters,
        cargo_used_liters,
        rider_used_liters,
        escrow_volume_liters,
        total_resource_volume_liters,
        cargo_mass_grams,
        rider_mass_grams,
        escrow_mass_grams,
        installed_component_mass_grams,
        grid_mass_grams,
        closure_mass_grams,
        block_count,
        cargo_inventory_count,
        production_queue_count,
        production_job_count,
        player_count,
        supported_rider_count,
        operation_history_count,
        internal_contact_count,
        placement_member_count,
        subject_count,
    })
}

fn sum_contents<'a>(
    mut contents: impl Iterator<Item = &'a InventoryContents>,
) -> Result<InventoryContents, DraftGridClosureError> {
    contents.try_fold(InventoryContents::default(), |total, next| {
        checked_add_contents(&total, next)
    })
}

fn checked_add_contents(
    left: &InventoryContents,
    right: &InventoryContents,
) -> Result<InventoryContents, DraftGridClosureError> {
    Ok(InventoryContents {
        ore: checked_add(left.ore, right.ore, "ore total")?,
        refined_material: checked_add(
            left.refined_material,
            right.refined_material,
            "refined-material total",
        )?,
        components: checked_add(left.components, right.components, "component total")?,
    })
}

fn checked_inventory_used_liters(
    inventory: &InventoryRecord,
) -> Result<u64, DraftGridClosureError> {
    contents_volume_liters(&inventory.contents)
}

fn contents_volume_liters(contents: &InventoryContents) -> Result<u64, DraftGridClosureError> {
    weighted_contents(contents, resource_unit_volume_liters, "resource volume")
}

fn contents_mass_grams_checked(contents: &InventoryContents) -> Result<u64, DraftGridClosureError> {
    weighted_contents(contents, resource_unit_mass_grams, "resource mass")
}

fn weighted_contents(
    contents: &InventoryContents,
    unit: fn(verse_protocol::ResourceKind) -> u64,
    label: &str,
) -> Result<u64, DraftGridClosureError> {
    sum_u64(
        [
            checked_mul(contents.ore, unit(verse_protocol::ResourceKind::Ore), label)?,
            checked_mul(
                contents.refined_material,
                unit(verse_protocol::ResourceKind::RefinedMaterial),
                label,
            )?,
            checked_mul(
                contents.components,
                unit(verse_protocol::ResourceKind::Component),
                label,
            )?,
        ]
        .into_iter(),
        label,
    )
}

fn sum_u64(
    mut values: impl Iterator<Item = u64>,
    label: &str,
) -> Result<u64, DraftGridClosureError> {
    values.try_fold(0_u64, |total, value| checked_add(total, value, label))
}

fn checked_add(left: u64, right: u64, label: &str) -> Result<u64, DraftGridClosureError> {
    left.checked_add(right).ok_or_else(|| {
        DraftGridClosureError::Unsupported(format!("{label} overflowed the proof budget"))
    })
}

fn checked_mul(left: u64, right: u64, label: &str) -> Result<u64, DraftGridClosureError> {
    left.checked_mul(right).ok_or_else(|| {
        DraftGridClosureError::Unsupported(format!("{label} overflowed the proof budget"))
    })
}

fn len_u64(value: usize, label: &str) -> Result<u64, DraftGridClosureError> {
    u64::try_from(value).map_err(|_| {
        DraftGridClosureError::Unsupported(format!("{label} exceeds the proof budget"))
    })
}

fn valid_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn hash_json<T: Serialize>(domain: &[u8], value: &T) -> Result<String, DraftGridClosureError> {
    let bytes = serde_json::to_vec(value).map_err(|source| {
        DraftGridClosureError::Invalid(format!("hash material cannot be encoded: {source}"))
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        PlayerTransferLock, PlayerTransferReservation, ProcessedOperationRecord, STARTER_GRID_ID,
        STARTER_INDUSTRY_GRID_ID, TransferConservationWitness, TransferWitnessDirection,
    };
    use verse_protocol::{
        IVec3, IntentReceipt, LocomotionSupportSnapshot, ProductionRecipeKind, Quat, ResourceKind,
    };

    pub(super) fn crossing_fixture() -> (WorldState, DraftGridTransferContextV2) {
        let mut source = WorldState::genesis(801);
        source.fencing_token = 11;
        let source_key = celestial::cell_origin_key();
        let destination_key =
            celestial::neighbor_cell_key(&source_key, [1, 0, 0]).expect("destination cell derives");
        let grid_address = celestial::address_from_origin_offset_um(
            &source.cell_address,
            [i128::from(celestial::CELL_EDGE_UM / 2) + 5_000_000, 0, 0],
        )
        .expect("crossing grid address canonicalizes");
        let player_address = celestial::address_from_origin_offset_um(
            &source.cell_address,
            [
                i128::from(celestial::CELL_EDGE_UM / 2) + 5_000_000,
                2_000_000,
                0,
            ],
        )
        .expect("crossing rider address canonicalizes");
        {
            let grid = source
                .grids
                .get_mut(STARTER_GRID_ID)
                .expect("starter grid exists");
            grid.address = grid_address;
            grid.position =
                celestial::local_position_from_address(&source.cell_address, &grid.address)
                    .expect("source grid pose hydrates");
            grid.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
            grid.control_linear_input = Vec3::new(1.0, 0.0, 0.0);
        }
        {
            let player = source.player.get_mut("player-local").expect("owner exists");
            player.address = player_address;
            player.position =
                celestial::local_position_from_address(&source.cell_address, &player.address)
                    .expect("source player pose hydrates");
            player.surface_contact = true;
            player.locomotion.kind = LocomotionKind::Magnetic;
            player.locomotion.support = Some(LocomotionSupportSnapshot {
                body_id: STARTER_GRID_ID.into(),
                collider_id: "block-core".into(),
                local_anchor: Vec3::new(0.0, 0.5, 0.0),
                local_normal: Vec3::new(0.0, 1.0, 0.0),
            });
            player.locomotion.magnetic_boots_enabled = true;
            player.last_received_input_sequence = 9;
            player.last_processed_input_sequence = 8;
            player.control_linear_input = Vec3::new(0.5, 0.0, 0.0);
            player.boost = true;
            player.jump = true;
            player.control_expires_at_simulation_tick = 40;
        }
        let placement = BundledPlacementPlan::new(
            STARTER_GRID_ID,
            source_key,
            destination_key,
            vec![
                BundledPlacementMember {
                    aggregate_id: STARTER_GRID_ID.into(),
                    aggregate_kind: MobileAggregateKind::Grid,
                    prior_placement_generation: 4,
                    resulting_placement_generation: 5,
                },
                BundledPlacementMember {
                    aggregate_id: "player-local".into(),
                    aggregate_kind: MobileAggregateKind::Player,
                    prior_placement_generation: 7,
                    resulting_placement_generation: 8,
                },
            ],
        )
        .expect("placement plan is valid");
        let context = DraftGridTransferContextV2 {
            transfer_id: "transfer-grid-closure-1".into(),
            source_assignment_generation: 3,
            destination_assignment_generation: 5,
            source_fencing_token: 11,
            destination_fencing_token: 13,
            placement,
            production_job_origins: BTreeMap::new(),
        };
        (source, context)
    }

    pub(super) fn package_fixture() -> (
        WorldState,
        DraftGridTransferContextV2,
        DraftGridClosurePackageV2,
    ) {
        let (source, context) = crossing_fixture();
        let package = extract_draft_grid_closure(&source, STARTER_GRID_ID, &context)
            .expect("grid closure extracts");
        (source, context, package)
    }

    pub(super) fn package_v3_directory_fixture() -> (
        WorldState,
        DraftGridTransferContextV2,
        DraftGridClosurePackageV2,
    ) {
        let (source, mut context) = crossing_fixture();
        for member in &mut context.placement.members {
            member.prior_placement_generation = 1;
            member.resulting_placement_generation = 2;
        }
        context.placement.member_root = context
            .placement
            .calculate_member_root()
            .expect("v3 fixture member root derives");
        let package = extract_draft_grid_closure(&source, STARTER_GRID_ID, &context)
            .expect("v3 directory grid closure extracts");
        (source, context, package)
    }

    fn reseal_package(package: &mut DraftGridClosurePackageV2) {
        let placement = BundledPlacementPlan::new(
            package.root_aggregate_id.clone(),
            package.source_cell_key.clone(),
            package.destination_cell_key.clone(),
            package.members.clone(),
        )
        .expect("tampered test placement remains structurally valid");
        package.member_root = placement.member_root;
        package.conservation = calculate_conservation(
            &package.grid,
            &package.cargo_inventories,
            &package.production_queues,
            &package.players,
            &package.active_internal_contacts,
            &package.members,
        )
        .expect("tampered test conservation derives");
        package.closure_root = package
            .calculate_closure_root()
            .expect("closure root derives");
        package.conservation_root = package
            .calculate_conservation_root()
            .expect("conservation root derives");
        package.package_hash = package
            .calculate_package_hash()
            .expect("package hash derives");
    }

    #[test]
    fn canonical_package_round_trips_and_keeps_active_v1_unchanged() {
        let (_, _, package) = package_fixture();
        let bytes = package.encode_canonical().expect("package encodes");
        let decoded =
            DraftGridClosurePackageV2::decode_canonical(&bytes).expect("canonical package decodes");
        assert_eq!(decoded, package);
        assert_eq!(package.schema_version, 2);
        assert_eq!(verse_protocol::TRANSFER_PACKAGE_SCHEMA_VERSION, 1);
        assert_eq!(verse_protocol::CELL_DIRECTORY_SCHEMA_VERSION, 2);
        assert_eq!(package.players.len(), 1);
        assert_eq!(package.cargo_inventories.len(), 1);
        assert_eq!(package.conservation.block_count, 25);
        assert_eq!(package.conservation.placement_member_count, 2);
        assert_eq!(
            package.package_hash,
            "dc06fd2d41b50671dca5189c905e40a9ec364aa0e35413a4c2de22d569862826"
        );
    }

    #[test]
    fn package_v2_requires_complete_manifest5_identity_before_persistence() {
        let (_, _, active_bound) = package_fixture();
        let manifest = crate::manifest_v5::build_validated_manifest_v5(801)
            .expect("manifest-5 capability builds");
        let other_manifest = crate::manifest_v5::build_validated_manifest_v5(802)
            .expect("other manifest-5 capability builds");
        assert!(active_bound.validate_manifest_v5(&manifest).is_err());

        let mut package = active_bound;
        package.universe_manifest_hash = manifest.manifest_hash().to_owned();
        package.package_hash = package
            .calculate_package_hash()
            .expect("manifest-5 package rehashes");
        let validated = package
            .validate_manifest_v5(&manifest)
            .expect("package binds the complete manifest-5 identity");
        assert_eq!(validated.package(), &package);
        assert_eq!(validated.manifest_hash(), manifest.manifest_hash());
        assert!(package.validate_manifest_v5(&other_manifest).is_err());

        let mut substituted_registry = package;
        substituted_registry.celestial_registry_hash = "ab".repeat(32);
        substituted_registry.package_hash = substituted_registry
            .calculate_package_hash()
            .expect("substituted registry package rehashes");
        assert!(substituted_registry.validate_wire().is_ok());
        assert!(
            substituted_registry
                .validate_manifest_v5(&manifest)
                .is_err(),
            "syntactically valid raw trust roots cannot replace manifest-5 authority"
        );
    }

    #[test]
    fn canonical_decoder_rejects_whitespace_unknown_fields_and_tampering() {
        let (_, _, package) = package_fixture();
        let bytes = package.encode_canonical().expect("package encodes");
        let mut whitespace = vec![b' '];
        whitespace.extend_from_slice(&bytes);
        assert!(DraftGridClosurePackageV2::decode_canonical(&whitespace).is_err());

        let mut value = serde_json::to_value(&package).expect("package becomes JSON");
        value["grid"]["blocks"]["block-core"]["unknown_authority"] = serde_json::json!(true);
        let unknown = serde_json::to_vec(&value).expect("tampered JSON encodes");
        assert!(DraftGridClosurePackageV2::decode_canonical(&unknown).is_err());

        let mut tampered = package.clone();
        tampered.grid.linear_velocity.x += 1.0;
        assert!(tampered.validate_wire().is_err());
    }

    #[test]
    fn destination_rebase_preserves_ship_and_support_but_resets_rider_controls() {
        let (_, _, package) = package_fixture();
        let rider = &package.players["player-local"];
        assert_eq!(
            rider.source_player.address,
            rider.destination_player.address
        );
        assert_eq!(
            rider.destination_player.movement_epoch,
            rider.source_player.movement_epoch + 1
        );
        assert_eq!(
            rider.destination_player.last_processed_input_sequence,
            rider.source_player.last_received_input_sequence
        );
        assert_eq!(
            rider.destination_player.locomotion,
            rider.source_player.locomotion
        );
        assert_eq!(
            rider.destination_player.linear_velocity,
            rider.source_player.linear_velocity
        );
        assert!(rider.destination_player.pending_control_frames.is_empty());
        assert_eq!(rider.destination_player.control_linear_input, Vec3::ZERO);
        assert!(!rider.destination_player.boost);
        assert!(!rider.destination_player.jump);
        assert_eq!(package.grid.linear_velocity, Vec3::new(4.0, 0.0, 0.0));
        assert_eq!(package.grid.control_linear_input, Vec3::new(1.0, 0.0, 0.0));
    }

    #[test]
    fn changed_source_and_incomplete_member_set_fail_closed() {
        let (source, context, package) = package_fixture();
        let mut changed = source.clone();
        changed
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("grid exists")
            .linear_velocity
            .x += 1.0;
        assert!(matches!(
            validate_source_closure(&changed, &context, &package),
            Err(DraftGridClosureError::Changed(_))
        ));

        let mut incomplete = context.clone();
        incomplete.placement.members.pop();
        incomplete.placement.member_root = incomplete
            .placement
            .calculate_member_root()
            .expect("root recalculates");
        assert!(extract_draft_grid_closure(&source, STARTER_GRID_ID, &incomplete).is_err());
    }

    #[test]
    fn wire_validation_rejects_a_resealed_unrelated_player() {
        let (_, _, mut package) = package_fixture();
        let mut outsider = package.players["player-local"].clone();
        outsider.source_player.player_id = "player-outsider".into();
        outsider.destination_player.player_id = "player-outsider".into();
        outsider.source_player.inventory_id = "inventory-player-outsider".into();
        outsider.destination_player.inventory_id = "inventory-player-outsider".into();
        outsider.inventory.inventory_id = "inventory-player-outsider".into();
        outsider.inventory.domain = InventoryDomain::Player {
            player_id: "player-outsider".into(),
        };
        outsider.source_player.locomotion.kind = LocomotionKind::Eva;
        outsider.source_player.locomotion.support = None;
        outsider.destination_player =
            destination_player_v2(&outsider.source_player, &package.destination_cell_key)
                .expect("outsider destination derives");
        outsider.is_owner = false;
        outsider.is_supported_rider = false;
        package.players.insert("player-outsider".into(), outsider);
        package.members.push(BundledPlacementMember {
            aggregate_id: "player-outsider".into(),
            aggregate_kind: MobileAggregateKind::Player,
            prior_placement_generation: 10,
            resulting_placement_generation: 11,
        });
        reseal_package(&mut package);
        assert!(matches!(
            package.validate_wire(),
            Err(DraftGridClosureError::Invalid(_))
        ));
    }

    #[test]
    fn wire_validation_rejects_cargo_and_rider_inventory_aliasing() {
        let (_, _, mut package) = package_fixture();
        let cargo_id = package
            .cargo_inventories
            .keys()
            .next()
            .expect("cargo exists")
            .clone();
        let rider = package
            .players
            .get_mut("player-local")
            .expect("rider exists");
        rider.source_player.inventory_id = cargo_id.clone();
        rider.destination_player.inventory_id = cargo_id.clone();
        rider.inventory.inventory_id = cargo_id;
        rider.inventory.domain = InventoryDomain::Player {
            player_id: "player-local".into(),
        };
        reseal_package(&mut package);
        assert!(matches!(
            package.validate_wire(),
            Err(DraftGridClosureError::Invalid(_))
        ));
    }

    #[test]
    fn owner_support_outside_the_grid_is_rejected() {
        let (mut source, context) = crossing_fixture();
        let owner = source.player.get_mut("player-local").expect("owner exists");
        owner.locomotion.kind = LocomotionKind::Grounded;
        owner.locomotion.support = Some(LocomotionSupportSnapshot {
            body_id: STARTER_INDUSTRY_GRID_ID.into(),
            collider_id: "block-industry-core".into(),
            local_anchor: Vec3::new(1.0, 1.5, 0.0),
            local_normal: Vec3::new(0.0, 1.0, 0.0),
        });
        assert!(matches!(
            extract_draft_grid_closure(&source, STARTER_GRID_ID, &context),
            Err(DraftGridClosureError::Unsupported(_))
        ));
    }

    #[test]
    fn excluded_production_queue_cannot_retain_a_closure_owner_edge() {
        let (mut source, context) = crossing_fixture();
        let (reserved_inputs, _, duration_ticks) =
            production_recipe_quantities(ProductionRecipeKind::Refining, 1)
                .expect("recipe derives");
        source.event_sequence = 1;
        source.last_event_hash = "22".repeat(32);
        source.production_queues.insert(
            "block-refinery".into(),
            VecDeque::from([ProductionJob {
                job_id: "job-external-owner-edge".into(),
                operation_id: "operation-external-owner-edge".into(),
                owner_player_id: "player-local".into(),
                machine_block_id: "block-refinery".into(),
                recipe: ProductionRecipeKind::Refining,
                content_manifest_version: source.content_manifest_version.clone(),
                batches: 1,
                source_inventory_id: "inventory-cargo-industry-starter".into(),
                destination_inventory_id: "inventory-cargo-industry-starter".into(),
                progress_ticks: 0,
                duration_ticks,
                reserved_inputs,
                pending_outputs: InventoryContents::default(),
                queued_event_sequence: 1,
            }]),
        );
        assert!(matches!(
            extract_draft_grid_closure(&source, STARTER_GRID_ID, &context),
            Err(DraftGridClosureError::Unsupported(_))
        ));
    }

    #[test]
    fn operation_history_limits_match_the_active_world_guardrails() {
        let (_, _, package) = package_fixture();
        let mut retained = BTreeMap::new();
        for sequence in
            1..=u64::try_from(PROCESSED_OPERATION_RETENTION_LIMIT + 1).expect("test bound fits u64")
        {
            let operation_id = format!("operation-{sequence}");
            retained.insert(
                sequence,
                ProcessedOperationRecord {
                    operation_id: operation_id.clone(),
                    intent_fingerprint: "33".repeat(32),
                    receipt_origin_cell_id: package.source_cell_id.clone(),
                    receipt: IntentReceipt {
                        operation_sequence: sequence,
                        operation_id,
                        event_sequence: sequence,
                        code: "ok".into(),
                        message: "bounded".into(),
                    },
                },
            );
        }
        let history = ActorOperationHistory {
            committed_through: u64::try_from(PROCESSED_OPERATION_RETENTION_LIMIT + 1)
                .expect("test bound fits u64"),
            compacted_through: 0,
            compacted_history_hash: String::new(),
            retained,
        };
        assert!(matches!(
            validate_operation_history(
                "player-local",
                Some(&history),
                &package.source_cell_id,
                u64::try_from(PROCESSED_OPERATION_RETENTION_LIMIT + 1)
                    .expect("test bound fits u64"),
            ),
            Err(DraftGridClosureError::Invalid(_))
        ));
    }

    #[test]
    fn anchored_and_externally_contacting_grids_are_rejected_without_mutation() {
        let (source, context) = crossing_fixture();
        let mut anchored = source.clone();
        anchored
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("grid exists")
            .anchored = true;
        let anchored_before = anchored.clone();
        assert!(extract_draft_grid_closure(&anchored, STARTER_GRID_ID, &context).is_err());
        assert_eq!(anchored, anchored_before);

        let mut contacting = source;
        contacting.active_contact_pairs.insert(ContactPairKey {
            body_a: STARTER_GRID_ID.into(),
            collider_a: "block-core".into(),
            body_b: "zz-external-body".into(),
            collider_b: "zz-external-collider".into(),
        });
        let contacting_before = contacting.clone();
        assert!(matches!(
            extract_draft_grid_closure(&contacting, STARTER_GRID_ID, &context),
            Err(DraftGridClosureError::Unsupported(_))
        ));
        assert_eq!(contacting, contacting_before);
    }

    #[test]
    fn internal_contacts_are_captured_exactly_and_order_is_canonical() {
        let (mut source, context) = crossing_fixture();
        let internal = ContactPairKey {
            body_a: STARTER_GRID_ID.into(),
            collider_a: "block-core".into(),
            body_b: player_body_id_v2("player-local"),
            collider_b: player_collider_id_v2("player-local"),
        };
        source.active_contact_pairs.insert(internal.clone());
        let package = extract_draft_grid_closure(&source, STARTER_GRID_ID, &context)
            .expect("internal contact transfers");
        assert_eq!(package.active_internal_contacts, BTreeSet::from([internal]));
        package.validate_wire().expect("internal contact validates");
    }

    #[test]
    fn every_closure_family_is_committed_and_btree_order_is_stable() {
        let (_, _, package) = package_fixture();
        let original_root = package.closure_root.clone();

        let mut grid_tamper = package.clone();
        grid_tamper.grid.angular_velocity.z += 0.25;
        assert_ne!(
            grid_tamper.calculate_closure_root().expect("root derives"),
            original_root
        );

        let mut cargo_tamper = package.clone();
        cargo_tamper
            .cargo_inventories
            .values_mut()
            .next()
            .expect("cargo exists")
            .contents
            .ore += 1;
        assert_ne!(
            cargo_tamper.calculate_closure_root().expect("root derives"),
            original_root
        );

        let mut rider_tamper = package.clone();
        rider_tamper
            .players
            .get_mut("player-local")
            .expect("rider exists")
            .source_player
            .experience += 1;
        assert_ne!(
            rider_tamper.calculate_closure_root().expect("root derives"),
            original_root
        );

        let reversed_grid = package
            .grid
            .blocks
            .iter()
            .rev()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>();
        let mut reordered = package.clone();
        reordered.grid.blocks = reversed_grid;
        assert_eq!(
            reordered.calculate_closure_root().expect("root derives"),
            original_root
        );
    }

    #[test]
    fn containment_checks_rotated_block_corners_and_full_rider_capsule() {
        let (_, _, mut package) = package_fixture();
        let destination_origin = celestial::cell_address_from_key(&package.destination_cell_key)
            .expect("origin derives");
        package.grid.blocks = BTreeMap::from([(
            "edge-block".into(),
            crate::model::Block::new(
                "edge-block",
                IVec3::ZERO,
                verse_protocol::BlockKind::Structural,
            ),
        )]);
        package.grid.orientation = Quat::IDENTITY;
        package.grid.address = celestial::address_from_local_position(
            &destination_origin,
            Vec3::new(9_999.5, 0.0, 0.0),
        )
        .expect("edge address canonicalizes");
        assert!(matches!(
            validate_destination_containment(&package),
            Err(DraftGridClosureError::Unsupported(_))
        ));

        package.grid.address = celestial::address_from_local_position(
            &destination_origin,
            Vec3::new(9_999.499_998, 0.0, 0.0),
        )
        .expect("contained address canonicalizes");
        assert!(validate_destination_containment(&package).is_ok());

        let radius = content::manifest().character.collision_radius_m;
        let rider = package
            .players
            .get_mut("player-local")
            .expect("rider exists");
        rider.source_player.orientation = Quat::IDENTITY;
        rider.source_player.address = celestial::address_from_local_position(
            &destination_origin,
            Vec3::new(10_000.0 - radius, 0.0, 0.0),
        )
        .expect("capsule edge address canonicalizes");
        assert!(matches!(
            validate_destination_containment(&package),
            Err(DraftGridClosureError::Unsupported(_))
        ));
    }

    #[test]
    fn conservation_uses_checked_arithmetic() {
        let (_, _, mut package) = package_fixture();
        package
            .cargo_inventories
            .values_mut()
            .next()
            .expect("cargo exists")
            .contents
            .ore = u64::MAX;
        assert!(matches!(
            calculate_conservation(
                &package.grid,
                &package.cargo_inventories,
                &package.production_queues,
                &package.players,
                &package.active_internal_contacts,
                &package.members,
            ),
            Err(DraftGridClosureError::Unsupported(_))
        ));
    }

    #[test]
    fn destination_conflicts_are_detected_before_import() {
        let (_, _, package) = package_fixture();
        let mut destination = WorldState::genesis_for_cell(801, &package.destination_cell_key)
            .expect("destination world derives");
        destination.fencing_token = package.destination_fencing_token;
        validate_destination_conflicts(&destination, &package)
            .expect("empty destination accepts the closure identities");
        let mut colliding_grid = package.grid.clone();
        colliding_grid.position = celestial::local_position_from_address(
            &destination.cell_address,
            &colliding_grid.address,
        )
        .expect("colliding destination grid pose hydrates");
        destination
            .inventories
            .extend(package.cargo_inventories.clone());
        destination
            .grids
            .insert(colliding_grid.grid_id.clone(), colliding_grid);
        assert!(matches!(
            validate_destination_conflicts(&destination, &package),
            Err(DraftGridClosureError::Unsupported(_))
        ));
    }

    #[test]
    fn world21_destination_conflicts_use_live_fence_and_reject_touching_contacts() {
        let (_, _, package) = package_fixture();
        let mut destination = WorldState::genesis_for_cell(801, &package.destination_cell_key)
            .expect("destination world derives");
        let live_fence = package
            .destination_fencing_token
            .checked_add(1)
            .expect("fixture fence advances");
        destination.fencing_token = live_fence;
        assert!(validate_destination_conflicts(&destination, &package).is_err());
        validate_destination_conflicts_in_validated_world_v21(&destination, &package, live_fence)
            .expect("validated successor fence accepts an empty destination");
        assert!(
            validate_destination_conflicts_in_validated_world_v21(
                &destination,
                &package,
                package.destination_fencing_token,
            )
            .is_err()
        );

        destination.active_contact_pairs.insert(ContactPairKey {
            body_a: package.grid.grid_id.clone(),
            collider_a: "stale-grid-collider".into(),
            body_b: "resident-body".into(),
            collider_b: "resident-collider".into(),
        });
        assert!(matches!(
            validate_destination_conflicts_in_validated_world_v21(
                &destination,
                &package,
                live_fence,
            ),
            Err(DraftGridClosureError::Unsupported(_))
        ));

        let mut grid_body_collision =
            WorldState::genesis_for_cell(801, &package.destination_cell_key)
                .expect("destination world derives");
        let mut resident_grid = package.grid.clone();
        resident_grid.grid_id = player_body_id_v2("player-local");
        resident_grid.blocks.clear();
        grid_body_collision
            .grids
            .insert(resident_grid.grid_id.clone(), resident_grid);
        assert!(validate_destination_identity_conflicts(&grid_body_collision, &package).is_err());

        let mut player_body_collision =
            WorldState::genesis_for_cell(801, &package.destination_cell_key)
                .expect("destination world derives");
        let mut resident_player = package.players["player-local"].destination_player.clone();
        resident_player.player_id = "resident".into();
        player_body_collision
            .player
            .by_id
            .insert("resident".into(), resident_player);
        let mut colliding_package = package;
        colliding_package.grid.grid_id = player_body_id_v2("resident");
        assert!(
            validate_destination_identity_conflicts(&player_body_collision, &colliding_package)
                .is_err()
        );

        let origin_destination = WorldState::genesis(801);
        let occupied = origin_destination
            .voxels
            .occupied
            .iter()
            .next()
            .copied()
            .expect("origin fixture has occupied voxels");
        let mut voxel_body_package = colliding_package.clone();
        voxel_body_package.grid.grid_id =
            voxel_collision_chunk_body_id(voxel_collision_chunk_coordinate(occupied));
        assert!(
            validate_destination_identity_conflicts(&origin_destination, &voxel_body_package)
                .is_err()
        );
        let mut planet_body_package = colliding_package;
        planet_body_package.grid.grid_id = PLANET_BODY_ID.into();
        assert!(
            validate_destination_identity_conflicts(&origin_destination, &planet_body_package)
                .is_err()
        );
    }

    #[test]
    fn source_rejects_existing_lock_reservation_and_witness_authority() {
        let (source, context) = crossing_fixture();
        let mut locked = source.clone();
        locked.player_transfer_locks.insert(
            "player-local".into(),
            PlayerTransferLock {
                transfer_id: "other-player-transfer".into(),
                package_hash: "44".repeat(32),
                destination_cell_id: context.placement.destination_cell_id.clone(),
                prior_placement_generation: 2,
                resulting_placement_generation: 3,
            },
        );
        assert!(matches!(
            extract_draft_grid_closure(&locked, STARTER_GRID_ID, &context),
            Err(DraftGridClosureError::Unsupported(_))
        ));

        let mut reserved = source.clone();
        reserved.player_transfer_reservations.insert(
            context.transfer_id.clone(),
            PlayerTransferReservation {
                transfer_id: context.transfer_id.clone(),
                package_hash: "55".repeat(32),
                receipt_hash: "66".repeat(32),
                source_cell_id: context.placement.destination_cell_id.clone(),
                destination_cell_id: context.placement.source_cell_id.clone(),
                player_id: "player-absent".into(),
                inventory_id: "inventory-absent".into(),
                destination_assignment_generation: 1,
                destination_fencing_token: 1,
                destination_event_sequence: 0,
                destination_world_hash: "77".repeat(32),
                prior_placement_generation: 1,
                resulting_placement_generation: 2,
            },
        );
        assert!(matches!(
            extract_draft_grid_closure(&reserved, STARTER_GRID_ID, &context),
            Err(DraftGridClosureError::Unsupported(_))
        ));

        let mut witnessed = source;
        witnessed.transfer_witnesses.insert(
            context.transfer_id.clone(),
            TransferConservationWitness {
                transfer_id: context.transfer_id.clone(),
                package_hash: "88".repeat(32),
                counterparty_cell_id: context.placement.destination_cell_id.clone(),
                direction: TransferWitnessDirection::Export,
                contents: InventoryContents::default(),
            },
        );
        assert!(matches!(
            extract_draft_grid_closure(&witnessed, STARTER_GRID_ID, &context),
            Err(DraftGridClosureError::Unsupported(_))
        ));
    }

    #[test]
    fn destination_rejects_every_existing_use_of_the_transfer_id() {
        let (_, _, package) = package_fixture();
        let empty_destination = WorldState::genesis_for_cell(801, &package.destination_cell_key)
            .expect("destination derives");

        let mut witnessed = empty_destination.clone();
        witnessed.fencing_token = package.destination_fencing_token;
        witnessed.transfer_witnesses.insert(
            package.transfer_id.clone(),
            TransferConservationWitness {
                transfer_id: package.transfer_id.clone(),
                package_hash: "99".repeat(32),
                counterparty_cell_id: package.source_cell_id.clone(),
                direction: TransferWitnessDirection::Import,
                contents: InventoryContents::default(),
            },
        );
        assert!(matches!(
            validate_destination_conflicts(&witnessed, &package),
            Err(DraftGridClosureError::Unsupported(_))
        ));

        let mut reserved = empty_destination.clone();
        reserved.fencing_token = package.destination_fencing_token;
        reserved.player_transfer_reservations.insert(
            package.transfer_id.clone(),
            PlayerTransferReservation {
                transfer_id: package.transfer_id.clone(),
                package_hash: "aa".repeat(32),
                receipt_hash: "bb".repeat(32),
                source_cell_id: package.source_cell_id.clone(),
                destination_cell_id: package.destination_cell_id.clone(),
                player_id: "player-absent".into(),
                inventory_id: "inventory-absent".into(),
                destination_assignment_generation: 1,
                destination_fencing_token: package.destination_fencing_token,
                destination_event_sequence: 0,
                destination_world_hash: "cc".repeat(32),
                prior_placement_generation: 1,
                resulting_placement_generation: 2,
            },
        );
        assert!(matches!(
            validate_destination_conflicts(&reserved, &package),
            Err(DraftGridClosureError::Unsupported(_))
        ));

        let mut locked = empty_destination;
        locked.fencing_token = package.destination_fencing_token;
        let mut remote_player = package.players["player-local"].destination_player.clone();
        remote_player.player_id = "player-remote".into();
        remote_player.inventory_id = "inventory-player-remote".into();
        remote_player.locomotion.kind = LocomotionKind::Eva;
        remote_player.locomotion.support = None;
        remote_player.locomotion.magnetic_boots_enabled = false;
        remote_player.surface_contact = false;
        let remote_inventory = InventoryRecord {
            inventory_id: remote_player.inventory_id.clone(),
            domain: InventoryDomain::Player {
                player_id: remote_player.player_id.clone(),
            },
            contents: InventoryContents::default(),
            capacity_liters: 100,
        };
        locked.player.primary_player_id = remote_player.player_id.clone();
        locked
            .player
            .by_id
            .insert(remote_player.player_id.clone(), remote_player);
        locked
            .inventories
            .insert(remote_inventory.inventory_id.clone(), remote_inventory);
        locked.player_transfer_locks.insert(
            "player-remote".into(),
            PlayerTransferLock {
                transfer_id: package.transfer_id.clone(),
                package_hash: "dd".repeat(32),
                destination_cell_id: package.source_cell_id.clone(),
                prior_placement_generation: 1,
                resulting_placement_generation: 2,
            },
        );
        assert!(matches!(
            validate_destination_conflicts(&locked, &package),
            Err(DraftGridClosureError::Unsupported(_))
        ));
    }

    #[test]
    fn queue_bearing_grid_requires_supported_owner_and_preserves_fifo_escrow() {
        let (mut source, _) = crossing_fixture();
        let source_key = celestial::cell_origin_key();
        let destination_key =
            celestial::neighbor_cell_key(&source_key, [1, 0, 0]).expect("destination derives");
        let address = celestial::address_from_origin_offset_um(
            &source.cell_address,
            [i128::from(celestial::CELL_EDGE_UM / 2) + 5_000_000, 0, 0],
        )
        .expect("industry address canonicalizes");
        let industry = source
            .grids
            .get_mut(STARTER_INDUSTRY_GRID_ID)
            .expect("industry grid exists");
        industry.address = address;
        industry.position =
            celestial::local_position_from_address(&source.cell_address, &industry.address)
                .expect("industry pose hydrates");
        let (reserved_inputs, _, duration_ticks) =
            production_recipe_quantities(ProductionRecipeKind::Refining, 1)
                .expect("recipe quantities derive");
        source.ledger.genesis_ore = source
            .ledger
            .genesis_ore
            .checked_add(reserved_inputs.ore)
            .expect("fixture genesis ore remains bounded");
        source.event_sequence = 1;
        source.last_event_hash = "11".repeat(32);
        let (job_id, job_origin) = DraftProductionJobOriginV2::new(
            &source.universe_id,
            &source.cell_id,
            source.event_sequence,
            0,
        )
        .expect("canonical production job identity derives");
        source.production_queues.insert(
            "block-refinery".into(),
            VecDeque::from([ProductionJob {
                job_id: job_id.clone(),
                operation_id: "operation-grid-handoff".into(),
                owner_player_id: "player-local".into(),
                machine_block_id: "block-refinery".into(),
                recipe: ProductionRecipeKind::Refining,
                content_manifest_version: source.content_manifest_version.clone(),
                batches: 1,
                source_inventory_id: "inventory-cargo-industry-starter".into(),
                destination_inventory_id: "inventory-cargo-industry-starter".into(),
                progress_ticks: 0,
                duration_ticks,
                reserved_inputs,
                pending_outputs: InventoryContents::default(),
                queued_event_sequence: 1,
            }]),
        );
        let placement = BundledPlacementPlan::new(
            STARTER_INDUSTRY_GRID_ID,
            source_key,
            destination_key,
            vec![
                BundledPlacementMember {
                    aggregate_id: STARTER_INDUSTRY_GRID_ID.into(),
                    aggregate_kind: MobileAggregateKind::Grid,
                    prior_placement_generation: 4,
                    resulting_placement_generation: 5,
                },
                BundledPlacementMember {
                    aggregate_id: "player-local".into(),
                    aggregate_kind: MobileAggregateKind::Player,
                    prior_placement_generation: 7,
                    resulting_placement_generation: 8,
                },
            ],
        )
        .expect("placement derives");
        let context = DraftGridTransferContextV2 {
            transfer_id: "transfer-grid-production".into(),
            source_assignment_generation: 3,
            destination_assignment_generation: 5,
            source_fencing_token: 11,
            destination_fencing_token: 13,
            placement,
            production_job_origins: BTreeMap::from([(job_id.clone(), job_origin)]),
        };
        let unsupported = extract_draft_grid_closure(&source, STARTER_INDUSTRY_GRID_ID, &context);
        assert!(
            matches!(unsupported, Err(DraftGridClosureError::Unsupported(_))),
            "unexpected queue-owner result: {unsupported:?}"
        );

        let player = source.player.get_mut("player-local").expect("owner exists");
        player.locomotion.kind = LocomotionKind::Grounded;
        player.locomotion.support = Some(LocomotionSupportSnapshot {
            body_id: STARTER_INDUSTRY_GRID_ID.into(),
            collider_id: "block-industry-core".into(),
            local_anchor: Vec3::new(1.0, 1.5, 0.0),
            local_normal: Vec3::new(0.0, 1.0, 0.0),
        });
        let player_address = celestial::address_from_origin_offset_um(
            &source.cell_address,
            [
                i128::from(celestial::CELL_EDGE_UM / 2) + 5_000_000,
                2_000_000,
                0,
            ],
        )
        .expect("owner address canonicalizes");
        player.address = player_address;
        player.position =
            celestial::local_position_from_address(&source.cell_address, &player.address)
                .expect("owner pose hydrates");
        let authoritative_state = state::DraftGridTransferCellStateV2::new_with_production_origins(
            source.clone(),
            context.production_job_origins.clone(),
        )
        .expect("draft world persists authoritative job origins");
        let mut caller_context = context.clone();
        caller_context.production_job_origins.clear();
        let package = authoritative_state
            .capture_grid_closure(STARTER_INDUSTRY_GRID_ID, &caller_context)
            .expect("supported owner and authoritative queue transfer together");
        assert_eq!(package.production_queues["block-refinery"].len(), 1);
        assert!(package.production_job_origins.contains_key(&job_id));
        assert_eq!(package.conservation.production_job_count, 1);
        assert_eq!(
            package.conservation.reserved_inputs.ore,
            content::manifest().recipes.refining.ore_input
        );
        assert_eq!(
            package.conservation.escrow_mass_grams,
            content::manifest().recipes.refining.ore_input
                * resource_unit_mass_grams(ResourceKind::Ore)
        );

        let mut missing_origin = package.clone();
        missing_origin.production_job_origins.clear();
        reseal_package(&mut missing_origin);
        assert!(matches!(
            missing_origin.validate_wire(),
            Err(DraftGridClosureError::Invalid(_))
        ));

        let mut substituted_origin = package.clone();
        let origin = substituted_origin
            .production_job_origins
            .remove(&job_id)
            .expect("job origin exists");
        substituted_origin
            .production_job_origins
            .insert("production-job-substituted".into(), origin);
        reseal_package(&mut substituted_origin);
        assert!(matches!(
            substituted_origin.validate_wire(),
            Err(DraftGridClosureError::Invalid(_))
        ));

        let mut multihop = package.clone();
        let foreign_cell_id = "ab".repeat(32);
        let (foreign_job_id, foreign_origin) =
            DraftProductionJobOriginV2::new(&multihop.universe_id, &foreign_cell_id, 100, 0)
                .expect("foreign canonical job derives");
        let foreign_job = multihop
            .production_queues
            .get_mut("block-refinery")
            .expect("queue exists")
            .front_mut()
            .expect("queue head exists");
        foreign_job.job_id = foreign_job_id.clone();
        foreign_job.operation_id = "operation-foreign-grid-handoff".into();
        foreign_job.queued_event_sequence = 100;
        multihop.production_job_origins =
            BTreeMap::from([(foreign_job_id.clone(), foreign_origin.clone())]);
        reseal_package(&mut multihop);
        assert!(
            multihop.validate_wire().is_ok(),
            "an origin-qualified job may cross a quieter intermediate cell"
        );

        let mut intermediate_world = source.clone();
        let intermediate_job = intermediate_world
            .production_queues
            .get_mut("block-refinery")
            .expect("intermediate queue exists")
            .front_mut()
            .expect("intermediate job exists");
        intermediate_job.job_id = foreign_job_id.clone();
        intermediate_job.operation_id = "operation-foreign-grid-handoff".into();
        intermediate_job.queued_event_sequence = 100;
        let foreign_origins = BTreeMap::from([(foreign_job_id.clone(), foreign_origin.clone())]);
        let intermediate_state = state::DraftGridTransferCellStateV2::new_with_production_origins(
            intermediate_world.clone(),
            foreign_origins.clone(),
        )
        .expect("world21 accepts an origin-qualified foreign frontier");
        let intermediate_bytes = intermediate_state
            .encode_canonical()
            .expect("intermediate world encodes");
        let reopened_intermediate =
            state::DraftGridTransferCellStateV2::decode_canonical(&intermediate_bytes)
                .expect("intermediate world reopens");
        assert_eq!(reopened_intermediate, intermediate_state);
        let mut second_hop_context = context.clone();
        second_hop_context.transfer_id = "transfer-grid-production-second-hop".into();
        second_hop_context.production_job_origins.clear();
        let second_hop_package = reopened_intermediate
            .capture_grid_closure(STARTER_INDUSTRY_GRID_ID, &second_hop_context)
            .expect("authoritative second-hop package captures");
        assert_eq!(second_hop_package.production_job_origins, foreign_origins);
        assert_eq!(
            second_hop_package.production_queues["block-refinery"][0].queued_event_sequence,
            100
        );
        assert!(
            state::DraftGridTransferCellStateV2::new_with_production_origins(
                intermediate_world.clone(),
                BTreeMap::new(),
            )
            .is_err()
        );

        let (local_future_job_id, local_future_origin) = DraftProductionJobOriginV2::new(
            &intermediate_world.universe_id,
            &intermediate_world.cell_id,
            100,
            0,
        )
        .expect("local future job identity derives");
        let local_future_job = intermediate_world
            .production_queues
            .get_mut("block-refinery")
            .expect("local future queue exists")
            .front_mut()
            .expect("local future job exists");
        local_future_job.job_id = local_future_job_id.clone();
        assert!(
            state::DraftGridTransferCellStateV2::new_with_production_origins(
                intermediate_world,
                BTreeMap::from([(local_future_job_id, local_future_origin)]),
            )
            .is_err(),
            "a local job cannot claim an uncommitted future event"
        );

        let mut two_job_package = package.clone();
        let (second_job_id, second_origin) = DraftProductionJobOriginV2::new(
            &two_job_package.universe_id,
            &two_job_package.source_cell_id,
            two_job_package.source_event_sequence,
            1,
        )
        .expect("second canonical job derives");
        let mut second_job = two_job_package.production_queues["block-refinery"]
            .front()
            .expect("queue head exists")
            .clone();
        second_job.job_id = second_job_id.clone();
        second_job.operation_id = "operation-grid-handoff-second".into();
        two_job_package
            .production_queues
            .get_mut("block-refinery")
            .expect("queue exists")
            .push_back(second_job);
        two_job_package
            .production_job_origins
            .insert(second_job_id.clone(), second_origin);
        reseal_package(&mut two_job_package);
        two_job_package
            .validate_wire()
            .expect("two-job package validates");
        let import_authority = production::DraftProductionImportAuthorityV2::new(
            &two_job_package,
            two_job_package.destination_assignment_generation,
            two_job_package.destination_fencing_token,
            7,
            "cd".repeat(32),
            1_800_000_000_000,
            3,
        )
        .expect("import authority validates");
        let eligibilities = production::derive_imported_production_eligibilities(
            &two_job_package,
            &import_authority,
        )
        .expect("exact eligibility map derives");
        production::validate_imported_production_eligibilities(
            &two_job_package,
            &import_authority,
            &eligibilities,
        )
        .expect("exact eligibility map validates");

        let mut reordered = eligibilities.clone();
        let record = reordered.get("block-refinery").expect("eligibility exists");
        let tampered =
            record.resealed_with_ordered_job_ids_for_test(vec![second_job_id, job_id.clone()]);
        reordered.insert("block-refinery".into(), tampered);
        assert!(
            production::validate_imported_production_eligibilities(
                &two_job_package,
                &import_authority,
                &reordered,
            )
            .is_err()
        );

        let mut missing = eligibilities.clone();
        missing.clear();
        assert!(
            production::validate_imported_production_eligibilities(
                &two_job_package,
                &import_authority,
                &missing,
            )
            .is_err()
        );

        let backdated = production::DraftProductionImportAuthorityV2::new(
            &two_job_package,
            two_job_package.destination_assignment_generation,
            two_job_package.destination_fencing_token,
            7,
            "cd".repeat(32),
            1_799_999_999_000,
            3,
        )
        .expect("alternate authority is internally valid");
        assert!(
            production::validate_imported_production_eligibilities(
                &two_job_package,
                &backdated,
                &eligibilities,
            )
            .is_err()
        );
    }
}
