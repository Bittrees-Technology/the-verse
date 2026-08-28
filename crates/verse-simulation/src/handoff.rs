// SPDX-License-Identifier: AGPL-3.0-or-later

//! Content-addressed P1.7 player handoff packages and destination quarantine.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verse_protocol::{CellKeyV1, InventoryContents, InventoryDomain, LocomotionKind, Vec3};

use crate::celestial;
use crate::model::{ActorOperationHistory, InventoryRecord, Player, WorldState, valid_blake3_hex};

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
    Ok(receipt)
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
    use crate::{EVENT_SCHEMA_VERSION, neighbor_cell_key};

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
}
