// SPDX-License-Identifier: AGPL-3.0-or-later

//! Content-addressed P1.7 player handoff packages and destination quarantine.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verse_protocol::{CellKeyV1, InventoryContents, InventoryDomain, LocomotionKind, Vec3};

use crate::celestial;
use crate::cell_directory::{CellTransferRecord, MobileAggregateKind, TransferPhase};
use crate::model::{
    ActorOperationHistory, InventoryRecord, Player, TransferConservationWitness,
    TransferWitnessDirection, WorldState, valid_blake3_hex,
};

pub const TRANSFER_PACKAGE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerTransferContext {
    pub transfer_id: String,
    pub source_cell_key: CellKeyV1,
    pub destination_cell_key: CellKeyV1,
    pub source_assignment_generation: u64,
    pub destination_assignment_generation: u64,
    pub source_fencing_token: u64,
    pub prior_placement_generation: u64,
    pub resulting_placement_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerTransferConservation {
    pub inventory_contents: InventoryContents,
    pub inventory_capacity_liters: u64,
    pub inventory_used_liters: u64,
    pub inventory_mass_grams: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerTransferPackage {
    pub schema_version: u32,
    pub transfer_id: String,
    pub aggregate_id: String,
    pub source_cell_key: CellKeyV1,
    pub source_cell_id: String,
    pub destination_cell_key: CellKeyV1,
    pub destination_cell_id: String,
    pub source_assignment_generation: u64,
    pub destination_assignment_generation: u64,
    pub source_fencing_token: u64,
    pub prior_placement_generation: u64,
    pub resulting_placement_generation: u64,
    pub universe_id: String,
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub content_manifest_version: String,
    pub source_event_sequence: u64,
    pub source_event_hash: String,
    pub source_world_hash: String,
    pub prepared_at_simulation_tick: u64,
    pub source_player: Player,
    pub destination_player: Player,
    pub inventory: InventoryRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_history: Option<ActorOperationHistory>,
    pub conservation: PlayerTransferConservation,
    pub package_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerTransferQuarantineReceipt {
    pub schema_version: u32,
    pub transfer_id: String,
    pub package_hash: String,
    pub destination_cell_id: String,
    pub destination_assignment_generation: u64,
    pub destination_fencing_token: u64,
    pub destination_event_sequence: u64,
    pub destination_world_hash: String,
    pub receipt_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HandoffError {
    #[error("player handoff package is invalid: {0}")]
    InvalidPackage(String),
    #[error("destination quarantine rejected the package: {0}")]
    QuarantineRejected(String),
    #[error("committed player handoff rejected the state transition: {0}")]
    CommittedStateRejected(String),
}

impl PlayerTransferPackage {
    pub fn calculate_hash(&self) -> String {
        let mut material = self.clone();
        material.package_hash.clear();
        let bytes = serde_json::to_vec(&material)
            .expect("player transfer package hash material always serializes");
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"the-verse/player-transfer-package/v1\0");
        hasher.update(&bytes);
        hasher.finalize().to_hex().to_string()
    }

    pub fn hash_is_valid(&self) -> bool {
        valid_blake3_hex(&self.package_hash) && self.package_hash == self.calculate_hash()
    }

    pub fn validate(&self) -> Result<(), HandoffError> {
        celestial::validate_cell_key(&self.source_cell_key)
            .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
        celestial::validate_cell_key(&self.destination_cell_key)
            .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
        let source_cell_id = celestial::cell_id(&self.source_cell_key)
            .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
        let destination_cell_id = celestial::cell_id(&self.destination_cell_key)
            .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
        if self.schema_version != TRANSFER_PACKAGE_SCHEMA_VERSION
            || !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.aggregate_id)
            || self.aggregate_id != self.source_player.player_id
            || self.aggregate_id != self.destination_player.player_id
            || self.source_cell_key == self.destination_cell_key
            || self.source_cell_id != source_cell_id
            || self.destination_cell_id != destination_cell_id
            || self.universe_id != self.source_cell_key.universe_id
            || self.universe_id != self.destination_cell_key.universe_id
            || self.source_assignment_generation == 0
            || self.destination_assignment_generation == 0
            || self.source_fencing_token == 0
            || self.prior_placement_generation == 0
            || self.prior_placement_generation.checked_add(1)
                != Some(self.resulting_placement_generation)
            || !valid_blake3_hex(&self.universe_manifest_hash)
            || !valid_blake3_hex(&self.celestial_registry_hash)
            || !valid_blake3_hex(&self.source_world_hash)
            || (self.source_event_sequence == 0 && !self.source_event_hash.is_empty())
            || (self.source_event_sequence > 0 && !valid_blake3_hex(&self.source_event_hash))
            || self.content_manifest_version.trim().is_empty()
        {
            return Err(HandoffError::InvalidPackage(
                "identity, generation, frontier, or trust-root binding is invalid".into(),
            ));
        }
        if self.source_player.locomotion.kind != LocomotionKind::Eva
            || self.source_player.locomotion.support.is_some()
            || self.source_player.locomotion.magnetic_boots_enabled
        {
            return Err(HandoffError::InvalidPackage(
                "P1.7 player proof requires independent EVA without support or magnetic binding"
                    .into(),
            ));
        }
        let destination_address_key = celestial::cell_key_from_address(&self.source_player.address)
            .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
        if destination_address_key != self.destination_cell_key {
            return Err(HandoffError::InvalidPackage(
                "player canonical address does not belong to the destination cell".into(),
            ));
        }
        let expected_destination =
            destination_player(&self.source_player, &self.destination_cell_key)?;
        if self.destination_player != expected_destination {
            return Err(HandoffError::InvalidPackage(
                "destination player is not the exact rebased source state".into(),
            ));
        }
        if self.inventory.inventory_id != self.source_player.inventory_id
            || self.inventory.domain
                != (InventoryDomain::Player {
                    player_id: self.aggregate_id.clone(),
                })
            || self.inventory.capacity_liters == 0
            || self.inventory.used_liters() > self.inventory.capacity_liters
        {
            return Err(HandoffError::InvalidPackage(
                "carried inventory identity, ownership, or capacity is invalid".into(),
            ));
        }
        let expected_conservation = PlayerTransferConservation {
            inventory_contents: self.inventory.contents.clone(),
            inventory_capacity_liters: self.inventory.capacity_liters,
            inventory_used_liters: self.inventory.used_liters(),
            inventory_mass_grams: self.inventory.mass_grams(),
        };
        if self.conservation != expected_conservation || !self.hash_is_valid() {
            return Err(HandoffError::InvalidPackage(
                "conservation vector or content-addressed package hash is invalid".into(),
            ));
        }
        Ok(())
    }
}

impl PlayerTransferQuarantineReceipt {
    pub fn calculate_hash(&self) -> String {
        let mut material = self.clone();
        material.receipt_hash.clear();
        let bytes = serde_json::to_vec(&material)
            .expect("player quarantine receipt hash material always serializes");
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"the-verse/player-transfer-quarantine-receipt/v1\0");
        hasher.update(&bytes);
        hasher.finalize().to_hex().to_string()
    }

    pub fn hash_is_valid(&self) -> bool {
        valid_blake3_hex(&self.receipt_hash) && self.receipt_hash == self.calculate_hash()
    }

    pub fn validate(&self) -> Result<(), HandoffError> {
        if self.schema_version != TRANSFER_PACKAGE_SCHEMA_VERSION
            || !valid_stable_id(&self.transfer_id)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.destination_cell_id)
            || self.destination_assignment_generation == 0
            || self.destination_fencing_token == 0
            || !valid_blake3_hex(&self.destination_world_hash)
            || !self.hash_is_valid()
        {
            return Err(HandoffError::QuarantineRejected(
                "receipt identity, generation, fence, frontier, or hash is invalid".into(),
            ));
        }
        Ok(())
    }
}

pub fn prepare_eva_player_transfer(
    source: &WorldState,
    actor_player_id: &str,
    context: &PlayerTransferContext,
) -> Result<PlayerTransferPackage, HandoffError> {
    source
        .validate_player_roster()
        .map_err(HandoffError::InvalidPackage)?;
    let source_cell_key = celestial::cell_key_from_address(&source.cell_address)
        .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
    let source_cell_id = celestial::cell_id(&source_cell_key)
        .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
    if source_cell_key != context.source_cell_key
        || source.cell_id != source_cell_id
        || source.fencing_token != context.source_fencing_token
        || context.source_assignment_generation == 0
        || context.destination_assignment_generation == 0
        || context.prior_placement_generation == 0
        || context.prior_placement_generation.checked_add(1)
            != Some(context.resulting_placement_generation)
    {
        return Err(HandoffError::InvalidPackage(
            "source cell, fence, assignment, or placement context is stale".into(),
        ));
    }
    let source_player = source
        .player
        .get(actor_player_id)
        .ok_or_else(|| HandoffError::InvalidPackage("source player is not resident".into()))?
        .clone();
    let inventory = source
        .inventories
        .get(&source_player.inventory_id)
        .ok_or_else(|| HandoffError::InvalidPackage("carried inventory is missing".into()))?
        .clone();
    let conservation = PlayerTransferConservation {
        inventory_contents: inventory.contents.clone(),
        inventory_capacity_liters: inventory.capacity_liters,
        inventory_used_liters: inventory.used_liters(),
        inventory_mass_grams: inventory.mass_grams(),
    };
    let destination_player = destination_player(&source_player, &context.destination_cell_key)?;
    let mut package = PlayerTransferPackage {
        schema_version: TRANSFER_PACKAGE_SCHEMA_VERSION,
        transfer_id: context.transfer_id.clone(),
        aggregate_id: actor_player_id.to_owned(),
        source_cell_key,
        source_cell_id,
        destination_cell_key: context.destination_cell_key.clone(),
        destination_cell_id: celestial::cell_id(&context.destination_cell_key)
            .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?,
        source_assignment_generation: context.source_assignment_generation,
        destination_assignment_generation: context.destination_assignment_generation,
        source_fencing_token: context.source_fencing_token,
        prior_placement_generation: context.prior_placement_generation,
        resulting_placement_generation: context.resulting_placement_generation,
        universe_id: source.universe_id.clone(),
        universe_manifest_hash: source.universe_manifest_hash.clone(),
        celestial_registry_hash: source.celestial_registry_hash.clone(),
        content_manifest_version: source.content_manifest_version.clone(),
        source_event_sequence: source.event_sequence,
        source_event_hash: source.last_event_hash.clone(),
        source_world_hash: source.state_hash(),
        prepared_at_simulation_tick: source.simulation_tick,
        source_player,
        destination_player,
        inventory,
        operation_history: source.processed_operations.get(actor_player_id).cloned(),
        conservation,
        package_hash: String::new(),
    };
    package.package_hash = package.calculate_hash();
    package.validate()?;
    Ok(package)
}

pub fn quarantine_eva_player_transfer(
    destination: &WorldState,
    destination_fencing_token: u64,
    package: &PlayerTransferPackage,
) -> Result<PlayerTransferQuarantineReceipt, HandoffError> {
    package.validate()?;
    destination
        .validate_player_roster()
        .map_err(HandoffError::QuarantineRejected)?;
    let destination_key = celestial::cell_key_from_address(&destination.cell_address)
        .map_err(|source| HandoffError::QuarantineRejected(source.to_string()))?;
    if destination_key != package.destination_cell_key
        || destination.cell_id != package.destination_cell_id
        || destination.universe_id != package.universe_id
        || destination.universe_manifest_hash != package.universe_manifest_hash
        || destination.celestial_registry_hash != package.celestial_registry_hash
        || destination.content_manifest_version != package.content_manifest_version
        || destination.fencing_token != destination_fencing_token
        || destination_fencing_token == 0
        || destination.player.get(&package.aggregate_id).is_some()
        || destination
            .inventories
            .contains_key(&package.inventory.inventory_id)
        || destination
            .processed_operations
            .contains_key(&package.aggregate_id)
    {
        return Err(HandoffError::QuarantineRejected(
            "destination identity, fence, roots, or subject absence is invalid".into(),
        ));
    }
    let mut receipt = PlayerTransferQuarantineReceipt {
        schema_version: TRANSFER_PACKAGE_SCHEMA_VERSION,
        transfer_id: package.transfer_id.clone(),
        package_hash: package.package_hash.clone(),
        destination_cell_id: destination.cell_id.clone(),
        destination_assignment_generation: package.destination_assignment_generation,
        destination_fencing_token,
        destination_event_sequence: destination.event_sequence,
        destination_world_hash: destination.state_hash(),
        receipt_hash: String::new(),
    };
    receipt.receipt_hash = receipt.calculate_hash();
    receipt.validate()?;
    Ok(receipt)
}

pub fn stage_committed_eva_export(
    source: &WorldState,
    package: &PlayerTransferPackage,
    transfer: &CellTransferRecord,
) -> Result<WorldState, HandoffError> {
    package.validate()?;
    validate_committed_transfer(package, transfer)?;
    let witness = transfer_witness(package, TransferWitnessDirection::Export);
    if let Some(existing) = source.transfer_witnesses.get(&package.transfer_id) {
        if existing != &witness
            || source.player.get(&package.aggregate_id).is_some()
            || source
                .inventories
                .contains_key(&package.inventory.inventory_id)
            || source
                .processed_operations
                .contains_key(&package.aggregate_id)
        {
            return Err(HandoffError::CommittedStateRejected(
                "source retry conflicts with the durable export witness".into(),
            ));
        }
        validate_staged_world(source)?;
        return Ok(source.clone());
    }
    if source.state_hash() != package.source_world_hash
        || source.cell_id != package.source_cell_id
        || source.fencing_token != package.source_fencing_token
        || source.player.get(&package.aggregate_id) != Some(&package.source_player)
        || source.inventories.get(&package.inventory.inventory_id) != Some(&package.inventory)
        || source.processed_operations.get(&package.aggregate_id)
            != package.operation_history.as_ref()
    {
        return Err(HandoffError::CommittedStateRejected(
            "source world no longer matches the exact prepared package".into(),
        ));
    }

    let mut staged = source.clone();
    staged.player.by_id.remove(&package.aggregate_id);
    staged.inventories.remove(&package.inventory.inventory_id);
    staged.processed_operations.remove(&package.aggregate_id);
    if staged.player.primary_player_id == package.aggregate_id {
        staged.player.primary_player_id = staged
            .player
            .by_id
            .keys()
            .next()
            .cloned()
            .unwrap_or_default();
    }
    add_transfer_witness(&mut staged, witness)?;
    validate_staged_world(&staged)?;
    Ok(staged)
}

pub fn stage_committed_eva_import(
    destination: &WorldState,
    package: &PlayerTransferPackage,
    receipt: &PlayerTransferQuarantineReceipt,
    transfer: &CellTransferRecord,
) -> Result<WorldState, HandoffError> {
    package.validate()?;
    receipt.validate()?;
    validate_committed_transfer(package, transfer)?;
    if receipt.transfer_id != package.transfer_id
        || receipt.package_hash != package.package_hash
        || receipt.destination_cell_id != package.destination_cell_id
        || receipt.destination_assignment_generation != package.destination_assignment_generation
        || transfer.quarantine_receipt_hash.as_deref() != Some(&receipt.receipt_hash)
    {
        return Err(HandoffError::CommittedStateRejected(
            "quarantine receipt is not bound to the committed transfer".into(),
        ));
    }
    let witness = transfer_witness(package, TransferWitnessDirection::Import);
    if let Some(existing) = destination.transfer_witnesses.get(&package.transfer_id) {
        if existing != &witness
            || destination.player.get(&package.aggregate_id) != Some(&package.destination_player)
            || destination.inventories.get(&package.inventory.inventory_id)
                != Some(&package.inventory)
            || destination.processed_operations.get(&package.aggregate_id)
                != package.operation_history.as_ref()
        {
            return Err(HandoffError::CommittedStateRejected(
                "destination retry conflicts with the durable import witness".into(),
            ));
        }
        validate_staged_world(destination)?;
        return Ok(destination.clone());
    }
    if destination.state_hash() != receipt.destination_world_hash
        || destination.cell_id != package.destination_cell_id
        || destination.fencing_token != receipt.destination_fencing_token
        || destination.event_sequence != receipt.destination_event_sequence
        || destination.player.get(&package.aggregate_id).is_some()
        || destination
            .inventories
            .contains_key(&package.inventory.inventory_id)
        || destination
            .processed_operations
            .contains_key(&package.aggregate_id)
    {
        return Err(HandoffError::CommittedStateRejected(
            "destination world no longer matches the exact quarantine boundary".into(),
        ));
    }

    let mut staged = destination.clone();
    if staged.player.by_id.is_empty() {
        staged
            .player
            .primary_player_id
            .clone_from(&package.aggregate_id);
    }
    staged.player.by_id.insert(
        package.aggregate_id.clone(),
        package.destination_player.clone(),
    );
    staged.inventories.insert(
        package.inventory.inventory_id.clone(),
        package.inventory.clone(),
    );
    if let Some(history) = &package.operation_history {
        staged
            .processed_operations
            .insert(package.aggregate_id.clone(), history.clone());
    }
    add_transfer_witness(&mut staged, witness)?;
    validate_staged_world(&staged)?;
    Ok(staged)
}

fn validate_committed_transfer(
    package: &PlayerTransferPackage,
    transfer: &CellTransferRecord,
) -> Result<(), HandoffError> {
    if !matches!(
        transfer.phase,
        TransferPhase::Committed | TransferPhase::Imported | TransferPhase::Finalized
    ) || transfer.aggregate_kind != MobileAggregateKind::Player
        || transfer.transfer_id != package.transfer_id
        || transfer.aggregate_id != package.aggregate_id
        || transfer.source_cell_key != package.source_cell_key
        || transfer.source_cell_id != package.source_cell_id
        || transfer.destination_cell_key != package.destination_cell_key
        || transfer.destination_cell_id != package.destination_cell_id
        || transfer.source_assignment_generation != package.source_assignment_generation
        || transfer.destination_assignment_generation != package.destination_assignment_generation
        || transfer.prior_placement_generation != package.prior_placement_generation
        || transfer.resulting_placement_generation != package.resulting_placement_generation
        || transfer.package_hash != package.package_hash
        || transfer.quarantine_receipt_hash.is_none()
    {
        return Err(HandoffError::CommittedStateRejected(
            "directory commit does not match the immutable player package".into(),
        ));
    }
    Ok(())
}

fn transfer_witness(
    package: &PlayerTransferPackage,
    direction: TransferWitnessDirection,
) -> TransferConservationWitness {
    TransferConservationWitness {
        transfer_id: package.transfer_id.clone(),
        package_hash: package.package_hash.clone(),
        counterparty_cell_id: match direction {
            TransferWitnessDirection::Import => package.source_cell_id.clone(),
            TransferWitnessDirection::Export => package.destination_cell_id.clone(),
        },
        direction,
        contents: package.conservation.inventory_contents.clone(),
    }
}

fn add_transfer_witness(
    world: &mut WorldState,
    witness: TransferConservationWitness,
) -> Result<(), HandoffError> {
    let contents = &witness.contents;
    let ledger = &mut world.ledger;
    match witness.direction {
        TransferWitnessDirection::Import => {
            ledger.transfer_imported_ore = checked_add(ledger.transfer_imported_ore, contents.ore)?;
            ledger.transfer_imported_refined =
                checked_add(ledger.transfer_imported_refined, contents.refined_material)?;
            ledger.transfer_imported_components =
                checked_add(ledger.transfer_imported_components, contents.components)?;
        }
        TransferWitnessDirection::Export => {
            ledger.transfer_exported_ore = checked_add(ledger.transfer_exported_ore, contents.ore)?;
            ledger.transfer_exported_refined =
                checked_add(ledger.transfer_exported_refined, contents.refined_material)?;
            ledger.transfer_exported_components =
                checked_add(ledger.transfer_exported_components, contents.components)?;
        }
    }
    if world
        .transfer_witnesses
        .insert(witness.transfer_id.clone(), witness)
        .is_some()
    {
        return Err(HandoffError::CommittedStateRejected(
            "transfer witness was already present".into(),
        ));
    }
    Ok(())
}

fn checked_add(left: u64, right: u64) -> Result<u64, HandoffError> {
    left.checked_add(right).ok_or_else(|| {
        HandoffError::CommittedStateRejected("transfer conservation counter overflowed".into())
    })
}

fn validate_staged_world(world: &WorldState) -> Result<(), HandoffError> {
    world
        .validate_player_roster()
        .map_err(HandoffError::CommittedStateRejected)?;
    if !world.conservation().valid {
        return Err(HandoffError::CommittedStateRejected(
            "cell conservation failed across the transfer boundary".into(),
        ));
    }
    Ok(())
}

fn destination_player(
    source_player: &Player,
    destination_cell_key: &CellKeyV1,
) -> Result<Player, HandoffError> {
    let mut destination = source_player.clone();
    let destination_origin = celestial::cell_address_from_key(destination_cell_key)
        .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
    destination.position =
        celestial::local_position_from_address(&destination_origin, &destination.address)
            .map_err(|source| HandoffError::InvalidPackage(source.to_string()))?;
    destination.movement_epoch = destination
        .movement_epoch
        .checked_add(1)
        .ok_or_else(|| HandoffError::InvalidPackage("movement epoch is exhausted".into()))?;
    destination.last_processed_input_sequence = destination.last_received_input_sequence;
    destination.pending_control_frames.clear();
    destination.control_linear_input = Vec3::ZERO;
    destination.control_angular_input = Vec3::ZERO;
    destination.boost = false;
    destination.jump = false;
    destination.control_expires_at_simulation_tick = 0;
    Ok(destination)
}

fn valid_stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WORLD_SCHEMA_VERSION;
    use crate::{
        EVENT_SCHEMA_VERSION, LocalCellDirectory, MobileAggregateKind, neighbor_cell_key,
        proof_cell_keys, universe_manifest,
    };
    use tempfile::tempdir;

    fn crossing_fixture() -> (WorldState, WorldState, PlayerTransferContext) {
        let mut source = WorldState::genesis(801);
        source.fencing_token = 11;
        let source_key = celestial::cell_origin_key();
        let destination_key =
            neighbor_cell_key(&source_key, [1, 0, 0]).expect("destination cell derives");
        let boundary_address = celestial::address_from_origin_offset_um(
            &source.cell_address,
            [i128::from(celestial::CELL_EDGE_UM / 2), 0, 0],
        )
        .expect("boundary address canonicalizes");
        let boundary_position =
            celestial::local_position_from_address(&source.cell_address, &boundary_address)
                .expect("source boundary position hydrates");
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
            .expect("crossing source remains canonical");

        let mut destination =
            WorldState::genesis_for_cell(801, &destination_key).expect("destination builds");
        destination.fencing_token = 17;
        assert_eq!(
            destination.schema_version, WORLD_SCHEMA_VERSION,
            "fixture uses the current world schema"
        );
        assert_eq!(EVENT_SCHEMA_VERSION, crate::event::EVENT_SCHEMA_VERSION);
        let context = PlayerTransferContext {
            transfer_id: "transfer-eva-1".into(),
            source_cell_key: source_key,
            destination_cell_key: destination_key,
            source_assignment_generation: 3,
            destination_assignment_generation: 5,
            source_fencing_token: 11,
            prior_placement_generation: 7,
            resulting_placement_generation: 8,
        };
        (source, destination, context)
    }

    #[test]
    fn eva_package_rebases_exact_state_and_conserves_carried_inventory() {
        let (source, destination, context) = crossing_fixture();
        let package = prepare_eva_player_transfer(&source, "player-local", &context)
            .expect("EVA package prepares");
        assert!(package.hash_is_valid());
        assert!((package.source_player.position.x - 10_000.0).abs() < f64::EPSILON);
        assert!((package.destination_player.position.x + 10_000.0).abs() < f64::EPSILON);
        assert_eq!(
            package.destination_player.movement_epoch,
            package.source_player.movement_epoch + 1
        );
        assert_eq!(
            package.conservation.inventory_contents,
            package.inventory.contents
        );
        assert_eq!(
            package.conservation.inventory_mass_grams,
            package.inventory.mass_grams()
        );

        let receipt =
            quarantine_eva_player_transfer(&destination, destination.fencing_token, &package)
                .expect("destination quarantines exact package");
        assert!(receipt.hash_is_valid());
        assert_eq!(receipt.package_hash, package.package_hash);
    }

    #[test]
    fn package_and_quarantine_reject_tamper_stale_fence_and_duplicate_subjects() {
        let (source, mut destination, context) = crossing_fixture();
        let package = prepare_eva_player_transfer(&source, "player-local", &context)
            .expect("EVA package prepares");

        let mut tampered = package.clone();
        tampered.inventory.contents.components += 1;
        assert!(tampered.validate().is_err());

        assert!(
            quarantine_eva_player_transfer(&destination, destination.fencing_token + 1, &package,)
                .is_err()
        );

        destination.player =
            crate::model::PlayerRoster::from_primary(package.destination_player.clone());
        destination.inventories.insert(
            package.inventory.inventory_id.clone(),
            package.inventory.clone(),
        );
        assert!(
            quarantine_eva_player_transfer(&destination, destination.fencing_token, &package,)
                .is_err()
        );
    }

    #[test]
    fn package_requires_server_derived_destination_crossing_and_independent_eva() {
        let (mut source, _, context) = crossing_fixture();
        source
            .player
            .get_mut("player-local")
            .expect("source player exists")
            .address = source.cell_address.clone();
        assert!(prepare_eva_player_transfer(&source, "player-local", &context).is_err());

        let (mut source, _, context) = crossing_fixture();
        source
            .player
            .get_mut("player-local")
            .expect("source player exists")
            .locomotion
            .magnetic_boots_enabled = true;
        assert!(prepare_eva_player_transfer(&source, "player-local", &context).is_err());
    }

    #[test]
    fn committed_directory_handoff_moves_player_and_cargo_exactly_once() {
        let (source, destination, mut context) = crossing_fixture();
        context.source_assignment_generation = 1;
        context.destination_assignment_generation = 1;
        context.prior_placement_generation = 1;
        context.resulting_placement_generation = 2;
        let package = prepare_eva_player_transfer(&source, "player-local", &context)
            .expect("EVA package prepares");
        assert!(package.conservation.inventory_contents.components > 0);

        let directory_root = tempdir().expect("temporary directory");
        let manifest = universe_manifest(801, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
            .expect("test manifest builds");
        let [origin, east] = proof_cell_keys().expect("proof cells build");
        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("directory opens");
        directory
            .claim(&origin, 0, "worker-origin")
            .expect("source assignment commits");
        directory
            .claim(&east, 0, "worker-east")
            .expect("destination assignment commits");
        directory
            .register_placement("player-local", MobileAggregateKind::Player, &origin)
            .expect("initial placement commits");
        directory
            .prepare_transfer(
                "player-local",
                1,
                &package.transfer_id,
                &east,
                &package.package_hash,
            )
            .expect("directory prepare commits");
        let receipt =
            quarantine_eva_player_transfer(&destination, destination.fencing_token, &package)
                .expect("destination quarantine succeeds");
        directory
            .record_quarantine(
                &package.transfer_id,
                &package.package_hash,
                &receipt.receipt_hash,
            )
            .expect("quarantine receipt commits");
        let committed = directory
            .commit_transfer(&package.transfer_id, 1)
            .expect("placement CAS commits");

        let exported = stage_committed_eva_export(&source, &package, &committed)
            .expect("source export stages atomically");
        let imported = stage_committed_eva_import(&destination, &package, &receipt, &committed)
            .expect("destination import stages atomically");
        assert!(exported.player.get("player-local").is_none());
        assert!(
            !exported
                .inventories
                .contains_key(&package.inventory.inventory_id)
        );
        assert_eq!(
            imported.player.get("player-local"),
            Some(&package.destination_player)
        );
        assert_eq!(
            imported.inventories.get(&package.inventory.inventory_id),
            Some(&package.inventory)
        );
        assert!(exported.conservation().valid);
        assert!(imported.conservation().valid);
        assert_eq!(
            exported
                .transfer_witnesses
                .get(&package.transfer_id)
                .expect("source export witness")
                .direction,
            TransferWitnessDirection::Export
        );
        assert_eq!(
            imported
                .transfer_witnesses
                .get(&package.transfer_id)
                .expect("destination import witness")
                .direction,
            TransferWitnessDirection::Import
        );

        assert_eq!(
            stage_committed_eva_export(&exported, &package, &committed)
                .expect("source export retry converges"),
            exported
        );
        assert_eq!(
            stage_committed_eva_import(&imported, &package, &receipt, &committed)
                .expect("destination import retry converges"),
            imported
        );

        let imported_record = directory
            .record_imported(&package.transfer_id)
            .expect("directory records destination import");
        let finalized = directory
            .finalize_transfer(&package.transfer_id)
            .expect("directory finalizes handoff");
        assert_eq!(imported_record.phase, TransferPhase::Imported);
        assert_eq!(finalized.phase, TransferPhase::Finalized);
        assert_eq!(
            stage_committed_eva_export(&exported, &package, &finalized)
                .expect("finalized source retry converges"),
            exported
        );
        assert_eq!(
            stage_committed_eva_import(&imported, &package, &receipt, &finalized)
                .expect("finalized destination retry converges"),
            imported
        );
    }

    #[test]
    fn world_materialization_rejects_precommit_and_conflicting_retries() {
        let (source, destination, context) = crossing_fixture();
        let package = prepare_eva_player_transfer(&source, "player-local", &context)
            .expect("EVA package prepares");
        let receipt =
            quarantine_eva_player_transfer(&destination, destination.fencing_token, &package)
                .expect("destination quarantine succeeds");
        let mut transfer = CellTransferRecord {
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
            phase: TransferPhase::Quarantined,
        };
        assert!(stage_committed_eva_export(&source, &package, &transfer).is_err());
        assert!(stage_committed_eva_import(&destination, &package, &receipt, &transfer).is_err());

        transfer.phase = TransferPhase::Committed;
        let mut exported =
            stage_committed_eva_export(&source, &package, &transfer).expect("export succeeds");
        exported
            .transfer_witnesses
            .get_mut(&package.transfer_id)
            .expect("witness exists")
            .package_hash = "0".repeat(64);
        assert!(stage_committed_eva_export(&exported, &package, &transfer).is_err());

        let mut stale_destination = destination.clone();
        stale_destination.simulation_tick += 1;
        assert!(
            stage_committed_eva_import(&stale_destination, &package, &receipt, &transfer).is_err()
        );
    }
}
