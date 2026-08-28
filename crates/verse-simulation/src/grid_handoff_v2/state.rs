// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant draft-world-21 lock, quarantine, and pre-commit abort staging.
//!
//! The envelope keeps aggregate authority beside the active world-20 payload
//! without adding fields to that published schema. It is private, in-memory,
//! and unreachable from `Runtime`, `Store`, and the production directory.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::production::{DraftProductionJobOriginV2, validate_production_job_origins};
use super::{
    BundledPlacementMember, BundledPlacementPlan, CellKeyV1, ContactPairKey,
    DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION, DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
    DraftGridClosureError, DraftGridClosurePackageV2, DraftGridTransferContextV2,
    InventoryContents, MAX_DRAFT_GRID_BLOCKS, MAX_DRAFT_GRID_CARGO_INVENTORIES,
    MAX_DRAFT_GRID_CONTACTS, MAX_DRAFT_GRID_MEMBERS, MAX_DRAFT_GRID_PRODUCTION_JOBS,
    MAX_DRAFT_GRID_PRODUCTION_QUEUES, MobileAggregateKind, WorldState, celestial,
    extract_draft_grid_closure_from_validated_world, hash_json, player_body_id_v2,
    valid_blake3_hex, valid_stable_id, validate_adjacent_cells, validate_destination_conflicts,
};
use crate::cell_directory::TransferPhase;
use crate::cell_directory_v3::ValidatedGridTransferAuthorityV3;
use crate::model::{TransferConservationWitness, TransferWitnessDirection};

const DRAFT_GRID_CELL_STATE_SCHEMA_VERSION: u32 = 21;
const MAX_DRAFT_GRID_CELL_STATE_BYTES: usize = 32 * 1_024 * 1_024;
const DRAFT_CELL_STATE_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-cell-state/v21\0";
const QUARANTINE_RECEIPT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-quarantine-receipt/v2\0";
const ABORT_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-abort-witness/v2\0";
const ABORT_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-abort-event/v2\0";
const EXPORT_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-witness/v2\0";
const EXPORT_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-event/v2\0";
const EXPORT_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-proof/v2\0";
const EXPORT_RECORD_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-record/v2\0";
const DRAFT_ACTIVE_WORLD_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-active-world/v21\0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridTransferBindingV2 {
    schema_version: u32,
    receipt_schema_version: u32,
    aggregate_kind: MobileAggregateKind,
    transfer_id: String,
    root_aggregate_id: String,
    package_hash: String,
    closure_root: String,
    conservation_root: String,
    member_root: String,
    source_cell_key: CellKeyV1,
    source_cell_id: String,
    destination_cell_key: CellKeyV1,
    destination_cell_id: String,
    source_assignment_generation: u64,
    source_fencing_token: u64,
    destination_assignment_generation: u64,
    destination_fencing_token: u64,
    members: Vec<BundledPlacementMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftFrozenClosureIdsV2 {
    grid_id: String,
    block_ids: Vec<String>,
    inventory_ids: Vec<String>,
    machine_block_ids: Vec<String>,
    job_ids: Vec<String>,
    player_ids: Vec<String>,
    operation_actor_ids: Vec<String>,
    internal_contacts: Vec<ContactPairKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftAggregateTransferLockV2 {
    binding: DraftGridTransferBindingV2,
    frozen: DraftFrozenClosureIdsV2,
    source_event_sequence: u64,
    source_event_hash: String,
    source_base_world_hash: String,
    prepared_at_simulation_tick: u64,
    production_job_origins: BTreeMap<String, DraftProductionJobOriginV2>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridTransferQuarantineReceiptV2 {
    schema_version: u32,
    transfer_id: String,
    root_aggregate_id: String,
    package_hash: String,
    closure_root: String,
    conservation_root: String,
    member_root: String,
    destination_cell_id: String,
    destination_assignment_generation: u64,
    destination_fencing_token: u64,
    destination_event_sequence: u64,
    destination_base_world_hash: String,
    destination_draft_world_hash: String,
    quarantined_at_unix_ms: u64,
    receipt_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftAggregateTransferReservationV2 {
    binding: DraftGridTransferBindingV2,
    frozen: DraftFrozenClosureIdsV2,
    receipt_hash: String,
    destination_event_sequence: u64,
    destination_base_world_hash: String,
    destination_draft_world_hash: String,
    quarantined_at_unix_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DraftGridTransferAbortSideV2 {
    Source,
    Destination,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridTransferAbortWitnessV2 {
    schema_version: u32,
    binding: DraftGridTransferBindingV2,
    side: DraftGridTransferAbortSideV2,
    removed_authority: bool,
    quarantine_receipt_hash: Option<String>,
    cell_id: String,
    assignment_generation: u64,
    historical_fencing_token: u64,
    live_fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: String,
    cleanup_event_sequence: u64,
    cleanup_event_hash: String,
    base_world_hash: String,
    prior_draft_world_hash: String,
    resulting_draft_world_hash: String,
    aborted_at_unix_ms: u64,
    witness_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridTransferLedgerVectorV2 {
    ore: u64,
    refined_material: u64,
    components: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridExportRecordV2 {
    schema_version: u32,
    binding: DraftGridTransferBindingV2,
    frozen: DraftFrozenClosureIdsV2,
    quarantine_receipt_hash: String,
    source_assignment_generation: u64,
    historical_fencing_token: u64,
    live_fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: String,
    export_event_sequence: u64,
    export_event_hash: String,
    prior_draft_world_hash: String,
    resulting_active_world_hash: String,
    ledger_vector: DraftGridTransferLedgerVectorV2,
    conservation_witness: TransferConservationWitness,
    exported_at_unix_ms: u64,
    mutation_witness_hash: String,
    proof_hash: String,
    record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DraftGridExportProofV2 {
    pub(crate) transfer_id: String,
    pub(crate) root_aggregate_id: String,
    pub(crate) member_root: String,
    pub(crate) package_hash: String,
    pub(crate) source_cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) fencing_token: u64,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) resulting_active_world_hash: String,
    pub(crate) quarantine_receipt_hash: String,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
    ledger_vector: DraftGridTransferLedgerVectorV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DraftGridAbortCleanupProofV2 {
    pub(crate) side: DraftGridTransferAbortSideV2,
    pub(crate) transfer_id: String,
    pub(crate) member_root: String,
    pub(crate) package_hash: String,
    pub(crate) cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) fencing_token: u64,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) resulting_draft_world_hash: String,
    pub(crate) quarantine_receipt_hash: Option<String>,
    pub(crate) abort_witness_hash: String,
    pub(crate) removed_authority: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftGridTransferCellStateV2 {
    schema_version: u32,
    base: WorldState,
    production_job_origins: BTreeMap<String, DraftProductionJobOriginV2>,
    aggregate_locks: BTreeMap<String, DraftAggregateTransferLockV2>,
    aggregate_reservations: BTreeMap<String, DraftAggregateTransferReservationV2>,
    committed_exports: BTreeMap<String, DraftGridExportRecordV2>,
    abort_witnesses: BTreeMap<String, DraftGridTransferAbortWitnessV2>,
    state_hash: String,
}

#[derive(Serialize)]
struct DraftActiveWorldHashMaterialV2<'a> {
    schema_version: u32,
    base: &'a WorldState,
    production_job_origins: &'a BTreeMap<String, DraftProductionJobOriginV2>,
    aggregate_locks: &'a BTreeMap<String, DraftAggregateTransferLockV2>,
    aggregate_reservations: &'a BTreeMap<String, DraftAggregateTransferReservationV2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DraftGridDirectoryProofKindV2 {
    SourcePrepare,
    DestinationQuarantine,
    SourceExport,
    DestinationImport,
    DestinationActivation,
    SourceFinalization,
    SourceAbort,
    DestinationAbort,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftGridDirectoryAuthorityV2 {
    binding: DraftGridTransferBindingV2,
    phase: TransferPhase,
    quarantine_receipt_hash: Option<String>,
    source_export_proof_hash: Option<String>,
    proofs: BTreeSet<DraftGridDirectoryProofKindV2>,
    live_source_assignment_generation: u64,
    live_source_fencing_token: u64,
    live_destination_assignment_generation: u64,
    live_destination_fencing_token: u64,
}

impl DraftGridTransferBindingV2 {
    fn from_package(package: &DraftGridClosurePackageV2) -> Self {
        Self {
            schema_version: package.schema_version,
            receipt_schema_version: package.receipt_schema_version,
            aggregate_kind: package.aggregate_kind,
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            package_hash: package.package_hash.clone(),
            closure_root: package.closure_root.clone(),
            conservation_root: package.conservation_root.clone(),
            member_root: package.member_root.clone(),
            source_cell_key: package.source_cell_key.clone(),
            source_cell_id: package.source_cell_id.clone(),
            destination_cell_key: package.destination_cell_key.clone(),
            destination_cell_id: package.destination_cell_id.clone(),
            source_assignment_generation: package.source_assignment_generation,
            source_fencing_token: package.source_fencing_token,
            destination_assignment_generation: package.destination_assignment_generation,
            destination_fencing_token: package.destination_fencing_token,
            members: package.members.clone(),
        }
    }

    fn from_validated_authority(authority: &ValidatedGridTransferAuthorityV3) -> Self {
        Self {
            schema_version: authority.package_schema_version(),
            receipt_schema_version: authority.receipt_schema_version(),
            aggregate_kind: authority.aggregate_kind(),
            transfer_id: authority.transfer_id().to_owned(),
            root_aggregate_id: authority.root_aggregate_id().to_owned(),
            package_hash: authority.package_hash().to_owned(),
            closure_root: authority.closure_root().to_owned(),
            conservation_root: authority.conservation_root().to_owned(),
            member_root: authority.member_root().to_owned(),
            source_cell_key: authority.source_cell_key().clone(),
            source_cell_id: authority.source_cell_id().to_owned(),
            destination_cell_key: authority.destination_cell_key().clone(),
            destination_cell_id: authority.destination_cell_id().to_owned(),
            source_assignment_generation: authority.source_assignment_generation(),
            source_fencing_token: authority.source_fencing_token(),
            destination_assignment_generation: authority.destination_assignment_generation(),
            destination_fencing_token: authority.destination_fencing_token(),
            members: authority.members().to_vec(),
        }
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        validate_adjacent_cells(&self.source_cell_key, &self.destination_cell_key)?;
        if self.schema_version != DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION
            || self.receipt_schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || self.aggregate_kind != MobileAggregateKind::Grid
            || !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.closure_root)
            || !valid_blake3_hex(&self.conservation_root)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.source_cell_id)
            || !valid_blake3_hex(&self.destination_cell_id)
            || celestial::cell_id(&self.source_cell_key).as_ref() != Ok(&self.source_cell_id)
            || celestial::cell_id(&self.destination_cell_key).as_ref()
                != Ok(&self.destination_cell_id)
            || self.source_cell_id == self.destination_cell_id
            || self.source_assignment_generation == 0
            || self.source_fencing_token == 0
            || self.destination_assignment_generation == 0
            || self.destination_fencing_token == 0
        {
            return Err(DraftGridClosureError::Invalid(
                "draft grid-transfer authority binding is invalid".into(),
            ));
        }
        let root_members = self
            .members
            .iter()
            .filter(|member| member.aggregate_id == self.root_aggregate_id)
            .count();
        if self.members.is_empty()
            || self.members.len() > MAX_DRAFT_GRID_MEMBERS
            || root_members != 1
            || self
                .members
                .iter()
                .find(|member| member.aggregate_id == self.root_aggregate_id)
                .is_none_or(|member| member.aggregate_kind != MobileAggregateKind::Grid)
        {
            return Err(DraftGridClosureError::Invalid(
                "draft authority members omit the unique grid root".into(),
            ));
        }
        let mut prior: Option<&str> = None;
        for member in &self.members {
            if prior.is_some_and(|prior| prior >= member.aggregate_id.as_str())
                || !valid_stable_id(&member.aggregate_id)
                || member.prior_placement_generation == 0
                || member.prior_placement_generation.checked_add(1)
                    != Some(member.resulting_placement_generation)
                || (member.aggregate_id != self.root_aggregate_id
                    && member.aggregate_kind != MobileAggregateKind::Player)
            {
                return Err(DraftGridClosureError::Invalid(
                    "draft authority members are not canonical ordered generation advances".into(),
                ));
            }
            prior = Some(&member.aggregate_id);
        }
        Ok(())
    }
}

impl DraftGridDirectoryAuthorityV2 {
    fn from_validated_v3(authority: &ValidatedGridTransferAuthorityV3) -> Self {
        let proofs = [
            (
                authority.source_prepare_proven(),
                DraftGridDirectoryProofKindV2::SourcePrepare,
            ),
            (
                authority.destination_quarantine_proven(),
                DraftGridDirectoryProofKindV2::DestinationQuarantine,
            ),
            (
                authority.source_export_proven(),
                DraftGridDirectoryProofKindV2::SourceExport,
            ),
            (
                authority.destination_import_proven(),
                DraftGridDirectoryProofKindV2::DestinationImport,
            ),
            (
                authority.destination_activation_proven(),
                DraftGridDirectoryProofKindV2::DestinationActivation,
            ),
            (
                authority.source_finalization_proven(),
                DraftGridDirectoryProofKindV2::SourceFinalization,
            ),
            (
                authority.source_abort_proven(),
                DraftGridDirectoryProofKindV2::SourceAbort,
            ),
            (
                authority.destination_abort_proven(),
                DraftGridDirectoryProofKindV2::DestinationAbort,
            ),
        ]
        .into_iter()
        .filter_map(|(present, proof)| present.then_some(proof))
        .collect();
        Self {
            binding: DraftGridTransferBindingV2::from_validated_authority(authority),
            phase: authority.phase(),
            quarantine_receipt_hash: authority.quarantine_receipt_hash().map(str::to_owned),
            source_export_proof_hash: authority
                .source_export_proof()
                .and_then(|proof| proof.export_proof_hash())
                .map(str::to_owned),
            proofs,
            live_source_assignment_generation: authority.live_source_assignment_generation(),
            live_source_fencing_token: authority.live_source_fencing_token(),
            live_destination_assignment_generation: authority
                .live_destination_assignment_generation(),
            live_destination_fencing_token: authority.live_destination_fencing_token(),
        }
    }

    #[cfg(test)]
    fn for_package(package: &DraftGridClosurePackageV2, phase: TransferPhase) -> Self {
        Self {
            binding: DraftGridTransferBindingV2::from_package(package),
            phase,
            quarantine_receipt_hash: None,
            source_export_proof_hash: None,
            proofs: BTreeSet::new(),
            live_source_assignment_generation: package.source_assignment_generation,
            live_source_fencing_token: package.source_fencing_token,
            live_destination_assignment_generation: package.destination_assignment_generation,
            live_destination_fencing_token: package.destination_fencing_token,
        }
    }

    fn has_proof(&self, kind: DraftGridDirectoryProofKindV2) -> bool {
        self.proofs.contains(&kind)
    }

    fn validate_phase_matrix(&self) -> Result<(), DraftGridClosureError> {
        let has_receipt = self.quarantine_receipt_hash.is_some();
        let has_prepare = self.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare);
        let has_quarantine = self.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine);
        let has_export = self.has_proof(DraftGridDirectoryProofKindV2::SourceExport);
        let has_import = self.has_proof(DraftGridDirectoryProofKindV2::DestinationImport);
        let has_activation = self.has_proof(DraftGridDirectoryProofKindV2::DestinationActivation);
        let has_finalization = self.has_proof(DraftGridDirectoryProofKindV2::SourceFinalization);
        let has_source_abort = self.has_proof(DraftGridDirectoryProofKindV2::SourceAbort);
        let has_destination_abort = self.has_proof(DraftGridDirectoryProofKindV2::DestinationAbort);
        let valid = match self.phase {
            TransferPhase::Prepared => {
                !has_receipt
                    && !has_quarantine
                    && !has_export
                    && !has_import
                    && !has_activation
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Quarantined => {
                has_receipt
                    && has_prepare
                    && has_quarantine
                    && !has_export
                    && !has_import
                    && !has_activation
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Committed => {
                has_receipt
                    && has_prepare
                    && has_quarantine
                    && !has_import
                    && !has_activation
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Imported => {
                has_receipt
                    && has_prepare
                    && has_quarantine
                    && has_export
                    && has_import
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Finalized => {
                has_receipt
                    && has_prepare
                    && has_quarantine
                    && has_export
                    && has_import
                    && has_activation
                    && has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Aborting => {
                !has_export
                    && !has_import
                    && !has_activation
                    && !has_finalization
                    && (!has_quarantine || has_prepare)
            }
            TransferPhase::Aborted => {
                !has_export
                    && !has_import
                    && !has_activation
                    && !has_finalization
                    && has_source_abort
                    && has_destination_abort
            }
        } && (has_receipt == has_quarantine)
            && (!has_quarantine || has_prepare)
            && (has_export == self.source_export_proof_hash.is_some())
            && (!has_import || has_export)
            && (!has_activation || has_import)
            && (!has_finalization || has_activation);
        if !valid {
            return Err(DraftGridClosureError::Invalid(
                "directory authority phase and validated proof matrix disagree".into(),
            ));
        }
        Ok(())
    }

    fn validate_package(
        &self,
        package: &DraftGridClosurePackageV2,
    ) -> Result<(), DraftGridClosureError> {
        self.binding.validate()?;
        self.validate_phase_matrix()?;
        if self.binding != DraftGridTransferBindingV2::from_package(package)
            || self.live_source_assignment_generation < self.binding.source_assignment_generation
            || self.live_source_fencing_token < self.binding.source_fencing_token
            || self.live_destination_assignment_generation
                < self.binding.destination_assignment_generation
            || self.live_destination_fencing_token < self.binding.destination_fencing_token
            || self
                .quarantine_receipt_hash
                .as_ref()
                .is_some_and(|hash| !valid_blake3_hex(hash))
            || self
                .source_export_proof_hash
                .as_ref()
                .is_some_and(|hash| !valid_blake3_hex(hash))
        {
            return Err(DraftGridClosureError::Invalid(
                "directory authority does not bind the exact grid package".into(),
            ));
        }
        Ok(())
    }
}

impl DraftFrozenClosureIdsV2 {
    fn from_package(package: &DraftGridClosurePackageV2) -> Self {
        Self {
            grid_id: package.grid.grid_id.clone(),
            block_ids: package.grid.blocks.keys().cloned().collect(),
            inventory_ids: package
                .cargo_inventories
                .keys()
                .chain(
                    package
                        .players
                        .values()
                        .map(|player| &player.inventory.inventory_id),
                )
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            machine_block_ids: package.production_queues.keys().cloned().collect(),
            job_ids: package
                .production_queues
                .values()
                .flatten()
                .map(|job| job.job_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            player_ids: package.players.keys().cloned().collect(),
            operation_actor_ids: package
                .players
                .iter()
                .filter(|(_, player)| player.operation_history.is_some())
                .map(|(player_id, _)| player_id.clone())
                .collect(),
            internal_contacts: package.active_internal_contacts.iter().cloned().collect(),
        }
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        if !valid_stable_id(&self.grid_id)
            || !strict_ordered_ids(&self.block_ids)
            || !strict_ordered_ids(&self.inventory_ids)
            || !strict_ordered_ids(&self.machine_block_ids)
            || !strict_ordered_ids(&self.job_ids)
            || !strict_ordered_ids(&self.player_ids)
            || (!self.operation_actor_ids.is_empty()
                && !strict_ordered_ids(&self.operation_actor_ids))
            || self.block_ids.is_empty()
            || self.block_ids.len() > MAX_DRAFT_GRID_BLOCKS
            || self.inventory_ids.len() > MAX_DRAFT_GRID_CARGO_INVENTORIES + MAX_DRAFT_GRID_MEMBERS
            || self.machine_block_ids.len() > MAX_DRAFT_GRID_PRODUCTION_QUEUES
            || self.job_ids.len() > MAX_DRAFT_GRID_PRODUCTION_JOBS
            || self.player_ids.is_empty()
            || self.player_ids.len() + 1 > MAX_DRAFT_GRID_MEMBERS
            || self.operation_actor_ids.len() > self.player_ids.len()
            || self.internal_contacts.len() > MAX_DRAFT_GRID_CONTACTS
            || self
                .operation_actor_ids
                .iter()
                .any(|actor| self.player_ids.binary_search(actor).is_err())
            || self
                .machine_block_ids
                .iter()
                .any(|machine| self.block_ids.binary_search(machine).is_err())
            || self
                .internal_contacts
                .windows(2)
                .any(|pair| pair[0] >= pair[1])
        {
            return Err(DraftGridClosureError::Invalid(
                "frozen grid-closure identities are not exact ordered bounded sets".into(),
            ));
        }
        Ok(())
    }

    fn validate_package(
        &self,
        package: &DraftGridClosurePackageV2,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        if self != &Self::from_package(package)
            || self.block_ids.len() as u64 != package.conservation.block_count
            || self.job_ids.len() as u64 != package.conservation.production_job_count
            || self.player_ids.len() as u64 != package.conservation.player_count
            || self.internal_contacts.len() as u64 != package.conservation.internal_contact_count
        {
            return Err(DraftGridClosureError::Invalid(
                "frozen identities do not match the committed package closure".into(),
            ));
        }
        Ok(())
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.grid_id == other.grid_id
            || sorted_intersects(&self.block_ids, &other.block_ids)
            || sorted_intersects(&self.inventory_ids, &other.inventory_ids)
            || sorted_intersects(&self.machine_block_ids, &other.machine_block_ids)
            || sorted_intersects(&self.job_ids, &other.job_ids)
            || sorted_intersects(&self.player_ids, &other.player_ids)
            || sorted_intersects(&self.operation_actor_ids, &other.operation_actor_ids)
            || sorted_intersects(&self.internal_contacts, &other.internal_contacts)
    }

    fn contains_subject(&self, subject_id: &str) -> bool {
        self.grid_id == subject_id
            || self
                .block_ids
                .binary_search_by(|id| id.as_str().cmp(subject_id))
                .is_ok()
            || self
                .inventory_ids
                .binary_search_by(|id| id.as_str().cmp(subject_id))
                .is_ok()
            || self
                .machine_block_ids
                .binary_search_by(|id| id.as_str().cmp(subject_id))
                .is_ok()
            || self
                .job_ids
                .binary_search_by(|id| id.as_str().cmp(subject_id))
                .is_ok()
            || self
                .player_ids
                .binary_search_by(|id| id.as_str().cmp(subject_id))
                .is_ok()
            || self
                .operation_actor_ids
                .binary_search_by(|id| id.as_str().cmp(subject_id))
                .is_ok()
    }
}

impl DraftAggregateTransferLockV2 {
    fn from_package(package: &DraftGridClosurePackageV2) -> Self {
        Self {
            binding: DraftGridTransferBindingV2::from_package(package),
            frozen: DraftFrozenClosureIdsV2::from_package(package),
            source_event_sequence: package.source_event_sequence,
            source_event_hash: package.source_event_hash.clone(),
            source_base_world_hash: package.source_world_hash.clone(),
            prepared_at_simulation_tick: package.prepared_at_simulation_tick,
            production_job_origins: package.production_job_origins.clone(),
        }
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.binding.validate()?;
        self.frozen.validate()?;
        if self.binding.root_aggregate_id != self.frozen.grid_id
            || !valid_blake3_hex(&self.source_base_world_hash)
            || (self.source_event_sequence == 0 && !self.source_event_hash.is_empty())
            || (self.source_event_sequence > 0 && !valid_blake3_hex(&self.source_event_hash))
        {
            return Err(DraftGridClosureError::Invalid(
                "aggregate lock frontier or root binding is invalid".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridTransferQuarantineReceiptV2 {
    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.receipt_hash.clear();
        hash_json(QUARANTINE_RECEIPT_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.closure_root)
            || !valid_blake3_hex(&self.conservation_root)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.destination_cell_id)
            || self.destination_assignment_generation == 0
            || self.destination_fencing_token == 0
            || !valid_blake3_hex(&self.destination_base_world_hash)
            || !valid_blake3_hex(&self.destination_draft_world_hash)
            || self.quarantined_at_unix_ms == 0
            || !valid_blake3_hex(&self.receipt_hash)
            || self.receipt_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "grid quarantine receipt identity, authority, or hash is invalid".into(),
            ));
        }
        Ok(())
    }
}

impl DraftAggregateTransferReservationV2 {
    fn from_receipt(
        package: &DraftGridClosurePackageV2,
        receipt: &DraftGridTransferQuarantineReceiptV2,
    ) -> Self {
        Self {
            binding: DraftGridTransferBindingV2::from_package(package),
            frozen: DraftFrozenClosureIdsV2::from_package(package),
            receipt_hash: receipt.receipt_hash.clone(),
            destination_event_sequence: receipt.destination_event_sequence,
            destination_base_world_hash: receipt.destination_base_world_hash.clone(),
            destination_draft_world_hash: receipt.destination_draft_world_hash.clone(),
            quarantined_at_unix_ms: receipt.quarantined_at_unix_ms,
        }
    }

    fn receipt(&self) -> DraftGridTransferQuarantineReceiptV2 {
        DraftGridTransferQuarantineReceiptV2 {
            schema_version: self.binding.receipt_schema_version,
            transfer_id: self.binding.transfer_id.clone(),
            root_aggregate_id: self.binding.root_aggregate_id.clone(),
            package_hash: self.binding.package_hash.clone(),
            closure_root: self.binding.closure_root.clone(),
            conservation_root: self.binding.conservation_root.clone(),
            member_root: self.binding.member_root.clone(),
            destination_cell_id: self.binding.destination_cell_id.clone(),
            destination_assignment_generation: self.binding.destination_assignment_generation,
            destination_fencing_token: self.binding.destination_fencing_token,
            destination_event_sequence: self.destination_event_sequence,
            destination_base_world_hash: self.destination_base_world_hash.clone(),
            destination_draft_world_hash: self.destination_draft_world_hash.clone(),
            quarantined_at_unix_ms: self.quarantined_at_unix_ms,
            receipt_hash: self.receipt_hash.clone(),
        }
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.binding.validate()?;
        self.frozen.validate()?;
        if self.binding.root_aggregate_id != self.frozen.grid_id
            || self.quarantined_at_unix_ms == 0
            || !valid_blake3_hex(&self.destination_base_world_hash)
            || !valid_blake3_hex(&self.destination_draft_world_hash)
        {
            return Err(DraftGridClosureError::Invalid(
                "aggregate reservation authority or closure is invalid".into(),
            ));
        }
        self.receipt().validate()
    }
}

impl DraftGridTransferLedgerVectorV2 {
    fn from_package(package: &DraftGridClosurePackageV2) -> Result<Self, DraftGridClosureError> {
        Ok(Self {
            ore: package.conservation.transferable_contents.ore,
            refined_material: package.conservation.transferable_contents.refined_material,
            components: package
                .conservation
                .transferable_contents
                .components
                .checked_add(package.conservation.installed_components)
                .ok_or_else(|| {
                    DraftGridClosureError::Unsupported(
                        "grid-transfer installed component total overflowed".into(),
                    )
                })?,
        })
    }

    fn as_contents(self) -> InventoryContents {
        InventoryContents {
            ore: self.ore,
            refined_material: self.refined_material,
            components: self.components,
        }
    }
}

impl DraftGridExportRecordV2 {
    #[allow(clippy::too_many_arguments)]
    fn new(
        prior_state: &DraftGridTransferCellStateV2,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
        ledger_vector: DraftGridTransferLedgerVectorV2,
        conservation_witness: TransferConservationWitness,
        exported_at_unix_ms: u64,
    ) -> Result<Self, DraftGridClosureError> {
        let quarantine_receipt_hash =
            authority.quarantine_receipt_hash.clone().ok_or_else(|| {
                DraftGridClosureError::Invalid(
                    "source export lacks a committed quarantine receipt".into(),
                )
            })?;
        let mut record = Self {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            binding: DraftGridTransferBindingV2::from_package(package),
            frozen: DraftFrozenClosureIdsV2::from_package(package),
            quarantine_receipt_hash,
            source_assignment_generation: authority.live_source_assignment_generation,
            historical_fencing_token: package.source_fencing_token,
            live_fencing_token: authority.live_source_fencing_token,
            prior_event_sequence: prior_state.base.event_sequence,
            prior_event_hash: prior_state.base.last_event_hash.clone(),
            export_event_sequence: prior_state.base.event_sequence.checked_add(1).ok_or_else(
                || {
                    DraftGridClosureError::Unsupported(
                        "source-export event sequence exhausted".into(),
                    )
                },
            )?,
            export_event_hash: String::new(),
            prior_draft_world_hash: prior_state.state_hash.clone(),
            resulting_active_world_hash: String::new(),
            ledger_vector,
            conservation_witness,
            exported_at_unix_ms,
            mutation_witness_hash: String::new(),
            proof_hash: String::new(),
            record_hash: String::new(),
        };
        record.mutation_witness_hash = record.calculate_mutation_hash()?;
        record.export_event_hash = record.proof().calculate_event_hash()?;
        Ok(record)
    }

    fn seal_resulting_active_world_hash(
        &mut self,
        resulting_state: &DraftGridTransferCellStateV2,
    ) -> Result<(), DraftGridClosureError> {
        self.resulting_active_world_hash = resulting_state.calculate_active_world_hash()?;
        self.proof_hash = self.proof().calculate_hash()?;
        self.record_hash = self.calculate_hash()?;
        self.validate()
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.export_event_hash.clear();
        material.resulting_active_world_hash.clear();
        material.mutation_witness_hash.clear();
        material.proof_hash.clear();
        material.record_hash.clear();
        hash_json(EXPORT_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_json(EXPORT_RECORD_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.binding.validate()?;
        self.frozen.validate()?;
        let expected_contents = self.ledger_vector.as_contents();
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || self.binding.root_aggregate_id != self.frozen.grid_id
            || !valid_blake3_hex(&self.quarantine_receipt_hash)
            || self.source_assignment_generation < self.binding.source_assignment_generation
            || self.historical_fencing_token != self.binding.source_fencing_token
            || self.live_fencing_token < self.historical_fencing_token
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.export_event_sequence)
            || !valid_blake3_hex(&self.export_event_hash)
            || !valid_blake3_hex(&self.prior_draft_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || self.conservation_witness.transfer_id != self.binding.transfer_id
            || self.conservation_witness.package_hash != self.binding.package_hash
            || self.conservation_witness.counterparty_cell_id != self.binding.destination_cell_id
            || self.conservation_witness.direction != TransferWitnessDirection::Export
            || self.conservation_witness.contents != expected_contents
            || self.exported_at_unix_ms == 0
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || !valid_blake3_hex(&self.proof_hash)
            || !valid_blake3_hex(&self.record_hash)
            || self.record_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "grid export record identity, frontier, conservation, or hash is invalid".into(),
            ));
        }
        self.proof()
            .validate()
            .map_err(DraftGridClosureError::Invalid)
    }

    fn proof(&self) -> DraftGridExportProofV2 {
        DraftGridExportProofV2 {
            transfer_id: self.binding.transfer_id.clone(),
            root_aggregate_id: self.binding.root_aggregate_id.clone(),
            member_root: self.binding.member_root.clone(),
            package_hash: self.binding.package_hash.clone(),
            source_cell_id: self.binding.source_cell_id.clone(),
            assignment_generation: self.source_assignment_generation,
            fencing_token: self.live_fencing_token,
            event_sequence: self.export_event_sequence,
            event_hash: self.export_event_hash.clone(),
            resulting_active_world_hash: self.resulting_active_world_hash.clone(),
            quarantine_receipt_hash: self.quarantine_receipt_hash.clone(),
            mutation_witness_hash: self.mutation_witness_hash.clone(),
            proof_hash: self.proof_hash.clone(),
            ledger_vector: self.ledger_vector,
        }
    }

    fn validate_request(
        &self,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        if self.binding != DraftGridTransferBindingV2::from_package(package)
            || authority.quarantine_receipt_hash.as_deref()
                != Some(self.quarantine_receipt_hash.as_str())
        {
            return Err(DraftGridClosureError::Changed(
                "source-export retry changed its package or quarantine receipt".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridExportProofV2 {
    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.event_hash.clear();
        material.resulting_active_world_hash.clear();
        material.proof_hash.clear();
        hash_json(EXPORT_EVENT_HASH_DOMAIN, &material)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(EXPORT_PROOF_HASH_DOMAIN, &material)
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.source_cell_id)
            || self.assignment_generation == 0
            || self.fencing_token == 0
            || self.event_sequence == 0
            || !valid_blake3_hex(&self.event_hash)
            || self.event_hash
                != self
                    .calculate_event_hash()
                    .map_err(|source| source.to_string())?
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.quarantine_receipt_hash)
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash().map_err(|source| source.to_string())?
        {
            return Err("grid source-export proof is not canonical fenced material".into());
        }
        Ok(())
    }
}

impl DraftGridTransferAbortWitnessV2 {
    #[allow(clippy::too_many_arguments)]
    fn new(
        prior_state: &DraftGridTransferCellStateV2,
        resulting_state: &DraftGridTransferCellStateV2,
        package: &DraftGridClosurePackageV2,
        side: DraftGridTransferAbortSideV2,
        removed_authority: bool,
        authority: &DraftGridDirectoryAuthorityV2,
        aborted_at_unix_ms: u64,
    ) -> Result<Self, DraftGridClosureError> {
        let (cell_id, assignment_generation, historical_fencing_token, live_fencing_token) =
            match side {
                DraftGridTransferAbortSideV2::Source => (
                    package.source_cell_id.clone(),
                    authority.live_source_assignment_generation,
                    package.source_fencing_token,
                    authority.live_source_fencing_token,
                ),
                DraftGridTransferAbortSideV2::Destination => (
                    package.destination_cell_id.clone(),
                    authority.live_destination_assignment_generation,
                    package.destination_fencing_token,
                    authority.live_destination_fencing_token,
                ),
            };
        let mut witness = Self {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            binding: DraftGridTransferBindingV2::from_package(package),
            side,
            removed_authority,
            quarantine_receipt_hash: authority.quarantine_receipt_hash.clone(),
            cell_id,
            assignment_generation,
            historical_fencing_token,
            live_fencing_token,
            prior_event_sequence: prior_state.base.event_sequence,
            prior_event_hash: prior_state.base.last_event_hash.clone(),
            cleanup_event_sequence: prior_state.base.event_sequence.checked_add(1).ok_or_else(
                || {
                    DraftGridClosureError::Unsupported(
                        "abort cleanup event sequence exhausted".into(),
                    )
                },
            )?,
            cleanup_event_hash: String::new(),
            base_world_hash: prior_state.base.state_hash(),
            prior_draft_world_hash: prior_state.state_hash.clone(),
            resulting_draft_world_hash: resulting_state.calculate_active_world_hash()?,
            aborted_at_unix_ms,
            witness_hash: String::new(),
        };
        witness.witness_hash = witness.calculate_hash()?;
        witness.cleanup_event_hash = witness.cleanup_proof().calculate_event_hash()?;
        witness.validate()?;
        Ok(witness)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.cleanup_event_hash.clear();
        material.witness_hash.clear();
        hash_json(ABORT_WITNESS_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.binding.validate()?;
        let (expected_cell_id, minimum_generation, historical_fencing_token) = match self.side {
            DraftGridTransferAbortSideV2::Source => (
                &self.binding.source_cell_id,
                self.binding.source_assignment_generation,
                self.binding.source_fencing_token,
            ),
            DraftGridTransferAbortSideV2::Destination => (
                &self.binding.destination_cell_id,
                self.binding.destination_assignment_generation,
                self.binding.destination_fencing_token,
            ),
        };
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || &self.cell_id != expected_cell_id
            || self.assignment_generation < minimum_generation
            || self.historical_fencing_token != historical_fencing_token
            || self.live_fencing_token < self.historical_fencing_token
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.cleanup_event_sequence)
            || !valid_blake3_hex(&self.cleanup_event_hash)
            || !valid_blake3_hex(&self.base_world_hash)
            || !valid_blake3_hex(&self.prior_draft_world_hash)
            || !valid_blake3_hex(&self.resulting_draft_world_hash)
            || self.aborted_at_unix_ms == 0
            || self
                .quarantine_receipt_hash
                .as_ref()
                .is_some_and(|hash| !valid_blake3_hex(hash))
            || !valid_blake3_hex(&self.witness_hash)
            || self.witness_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "grid abort witness identity, frontier, or hash is invalid".into(),
            ));
        }
        self.cleanup_proof()
            .validate()
            .map_err(DraftGridClosureError::Invalid)?;
        Ok(())
    }

    fn cleanup_proof(&self) -> DraftGridAbortCleanupProofV2 {
        DraftGridAbortCleanupProofV2 {
            side: self.side,
            transfer_id: self.binding.transfer_id.clone(),
            member_root: self.binding.member_root.clone(),
            package_hash: self.binding.package_hash.clone(),
            cell_id: self.cell_id.clone(),
            assignment_generation: self.assignment_generation,
            fencing_token: self.live_fencing_token,
            event_sequence: self.cleanup_event_sequence,
            event_hash: self.cleanup_event_hash.clone(),
            resulting_draft_world_hash: self.resulting_draft_world_hash.clone(),
            quarantine_receipt_hash: self.quarantine_receipt_hash.clone(),
            abort_witness_hash: self.witness_hash.clone(),
            removed_authority: self.removed_authority,
        }
    }

    fn validate_request(
        &self,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
        side: DraftGridTransferAbortSideV2,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        if self.binding != DraftGridTransferBindingV2::from_package(package)
            || self.side != side
            || self.quarantine_receipt_hash != authority.quarantine_receipt_hash
        {
            return Err(DraftGridClosureError::Changed(
                "abort retry changed its package, side, or quarantine authority".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridAbortCleanupProofV2 {
    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.event_hash.clear();
        hash_json(ABORT_EVENT_HASH_DOMAIN, &material)
    }

    #[cfg(test)]
    pub(crate) fn seal_event_hash(&mut self) -> Result<(), String> {
        self.event_hash.clear();
        self.event_hash = self
            .calculate_event_hash()
            .map_err(|source| source.to_string())?;
        self.validate()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !valid_stable_id(&self.transfer_id)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.cell_id)
            || self.assignment_generation == 0
            || self.fencing_token == 0
            || self.event_sequence == 0
            || !valid_blake3_hex(&self.event_hash)
            || self.event_hash
                != self
                    .calculate_event_hash()
                    .map_err(|source| source.to_string())?
            || !valid_blake3_hex(&self.resulting_draft_world_hash)
            || self
                .quarantine_receipt_hash
                .as_ref()
                .is_some_and(|hash| !valid_blake3_hex(hash))
            || !valid_blake3_hex(&self.abort_witness_hash)
        {
            return Err("grid abort cleanup proof is not canonical fenced material".into());
        }
        Ok(())
    }
}

impl DraftGridTransferCellStateV2 {
    fn new(base: WorldState) -> Result<Self, DraftGridClosureError> {
        Self::new_with_production_origins(base, BTreeMap::new())
    }

    pub(super) fn new_with_production_origins(
        base: WorldState,
        production_job_origins: BTreeMap<String, DraftProductionJobOriginV2>,
    ) -> Result<Self, DraftGridClosureError> {
        let mut state = Self {
            schema_version: DRAFT_GRID_CELL_STATE_SCHEMA_VERSION,
            base,
            production_job_origins,
            aggregate_locks: BTreeMap::new(),
            aggregate_reservations: BTreeMap::new(),
            committed_exports: BTreeMap::new(),
            abort_witnesses: BTreeMap::new(),
            state_hash: String::new(),
        };
        state.seal()?;
        Ok(state)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.state_hash.clear();
        hash_json(DRAFT_CELL_STATE_HASH_DOMAIN, &material)
    }

    fn calculate_active_world_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            DRAFT_ACTIVE_WORLD_HASH_DOMAIN,
            &DraftActiveWorldHashMaterialV2 {
                schema_version: self.schema_version,
                base: &self.base,
                production_job_origins: &self.production_job_origins,
                aggregate_locks: &self.aggregate_locks,
                aggregate_reservations: &self.aggregate_reservations,
            },
        )
    }

    fn seal(&mut self) -> Result<(), DraftGridClosureError> {
        self.state_hash.clear();
        self.state_hash = self.calculate_hash()?;
        self.validate()
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.base
            .validate_player_roster_with_job_frontier(|job| {
                self.production_job_origins
                    .get(&job.job_id)
                    .is_some_and(|origin| {
                        origin.frontier_is_valid_in_cell(
                            &self.base.cell_id,
                            self.base.event_sequence,
                            job,
                        )
                    })
            })
            .map_err(DraftGridClosureError::Invalid)?;
        validate_production_job_origins(
            &self.base.universe_id,
            &self.base.cell_id,
            self.base.event_sequence,
            &self.base.production_queues,
            &self.production_job_origins,
        )?;
        if self.schema_version != DRAFT_GRID_CELL_STATE_SCHEMA_VERSION
            || !self.base.conservation().valid
            || self.aggregate_locks.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.aggregate_reservations.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.committed_exports.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.abort_witnesses.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || !valid_blake3_hex(&self.state_hash)
            || self.state_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "draft grid-transfer cell envelope or base conservation is invalid".into(),
            ));
        }

        let mut transfer_ids = BTreeSet::new();
        let mut frozen_sets = Vec::new();
        for (root_id, lock) in &self.aggregate_locks {
            lock.validate()?;
            if root_id != &lock.binding.root_aggregate_id
                || lock.binding.source_cell_id != self.base.cell_id
                || self.base.fencing_token < lock.binding.source_fencing_token
                || !transfer_ids.insert(lock.binding.transfer_id.as_str())
                || !source_lock_matches(&self.base, lock)
                || active_v1_transfer_conflicts(&self.base, &lock.binding.transfer_id, &lock.frozen)
            {
                return Err(DraftGridClosureError::Invalid(
                    "aggregate lock does not bind one exact resident source closure".into(),
                ));
            }
            frozen_sets.push(&lock.frozen);
        }
        for (transfer_id, reservation) in &self.aggregate_reservations {
            reservation.validate()?;
            if transfer_id != &reservation.binding.transfer_id
                || reservation.binding.destination_cell_id != self.base.cell_id
                || self.base.fencing_token < reservation.binding.destination_fencing_token
                || !transfer_ids.insert(transfer_id)
                || !frozen_closure_is_absent(&self.base, &reservation.frozen)
                || active_v1_transfer_conflicts(
                    &self.base,
                    &reservation.binding.transfer_id,
                    &reservation.frozen,
                )
            {
                return Err(DraftGridClosureError::Invalid(
                    "aggregate reservation does not bind one absent destination closure".into(),
                ));
            }
            frozen_sets.push(&reservation.frozen);
        }
        for (transfer_id, export) in &self.committed_exports {
            export.validate()?;
            export
                .proof()
                .validate()
                .map_err(DraftGridClosureError::Invalid)?;
            if transfer_id != &export.binding.transfer_id
                || export.binding.source_cell_id != self.base.cell_id
                || self.base.fencing_token < export.live_fencing_token
                || self.base.event_sequence < export.export_event_sequence
                || (self.base.event_sequence == export.export_event_sequence
                    && self.base.last_event_hash != export.export_event_hash)
                || !transfer_ids.insert(transfer_id)
                || self.base.transfer_witnesses.get(transfer_id)
                    != Some(&export.conservation_witness)
                || self
                    .aggregate_locks
                    .contains_key(&export.binding.root_aggregate_id)
                || !frozen_closure_is_absent(&self.base, &export.frozen)
                || export
                    .frozen
                    .job_ids
                    .iter()
                    .any(|job_id| self.production_job_origins.contains_key(job_id))
                || self
                    .base
                    .player_transfer_locks
                    .values()
                    .any(|lock| lock.transfer_id == *transfer_id)
                || self
                    .base
                    .player_transfer_reservations
                    .values()
                    .any(|reservation| reservation.transfer_id == *transfer_id)
            {
                return Err(DraftGridClosureError::Invalid(
                    "committed export does not bind one exact absent source closure and ledger witness"
                        .into(),
                ));
            }
        }
        for (transfer_id, witness) in &self.abort_witnesses {
            witness.validate()?;
            witness
                .cleanup_proof()
                .validate()
                .map_err(DraftGridClosureError::Invalid)?;
            if transfer_id != &witness.binding.transfer_id
                || witness.cell_id != self.base.cell_id
                || self.base.fencing_token < witness.live_fencing_token
                || !transfer_ids.insert(transfer_id)
                || active_v1_transfer_id_conflicts(&self.base, &witness.binding.transfer_id)
            {
                return Err(DraftGridClosureError::Invalid(
                    "abort witness does not bind one exact historical cell cleanup".into(),
                ));
            }
        }
        for (index, frozen) in frozen_sets.iter().enumerate() {
            if frozen_sets
                .iter()
                .skip(index + 1)
                .any(|other| frozen.overlaps(other))
            {
                return Err(DraftGridClosureError::Invalid(
                    "two aggregate transfers overlap one frozen subject".into(),
                ));
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|source| {
            DraftGridClosureError::Invalid(format!("draft cell state cannot encode: {source}"))
        })?;
        if bytes.len() > MAX_DRAFT_GRID_CELL_STATE_BYTES {
            return Err(DraftGridClosureError::TooLarge);
        }
        Ok(())
    }

    pub(super) fn encode_canonical(&self) -> Result<Vec<u8>, DraftGridClosureError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|source| {
            DraftGridClosureError::Invalid(format!("draft cell state cannot encode: {source}"))
        })
    }

    pub(super) fn decode_canonical(bytes: &[u8]) -> Result<Self, DraftGridClosureError> {
        if bytes.len() > MAX_DRAFT_GRID_CELL_STATE_BYTES {
            return Err(DraftGridClosureError::TooLarge);
        }
        let mut state = serde_json::from_slice::<Self>(bytes).map_err(|source| {
            DraftGridClosureError::Invalid(format!("draft cell state JSON is invalid: {source}"))
        })?;
        state
            .base
            .hydrate_spatial_poses()
            .map_err(DraftGridClosureError::Invalid)?;
        state.validate()?;
        let canonical = serde_json::to_vec(&state).map_err(|source| {
            DraftGridClosureError::Invalid(format!("draft cell state cannot re-encode: {source}"))
        })?;
        if canonical != bytes {
            return Err(DraftGridClosureError::Invalid(
                "draft cell state bytes are not the exact canonical encoding".into(),
            ));
        }
        Ok(state)
    }

    fn locked_transfer_for_subject(&self, subject_id: &str) -> Option<&str> {
        self.aggregate_locks.values().find_map(|lock| {
            lock.frozen
                .contains_subject(subject_id)
                .then_some(lock.binding.transfer_id.as_str())
        })
    }

    pub(super) fn capture_grid_closure(
        &self,
        grid_id: &str,
        context: &DraftGridTransferContextV2,
    ) -> Result<DraftGridClosurePackageV2, DraftGridClosureError> {
        self.validate()?;
        let grid =
            self.base.grids.get(grid_id).ok_or_else(|| {
                DraftGridClosureError::Invalid("source grid is not resident".into())
            })?;
        let job_ids = grid
            .blocks
            .keys()
            .filter_map(|block_id| self.base.production_queues.get(block_id))
            .flatten()
            .map(|job| job.job_id.as_str())
            .collect::<BTreeSet<_>>();
        let derived_origins = self
            .production_job_origins
            .iter()
            .filter(|(job_id, _)| job_ids.contains(job_id.as_str()))
            .map(|(job_id, origin)| (job_id.clone(), origin.clone()))
            .collect();
        let mut derived_context = context.clone();
        derived_context.production_job_origins = derived_origins;
        extract_draft_grid_closure_from_validated_world(&self.base, grid_id, &derived_context)
    }
}

const MAX_DRAFT_TRANSFERS_PER_CELL: usize = 1_024;

fn strict_ordered_ids(ids: &[String]) -> bool {
    ids.iter().all(|id| valid_stable_id(id)) && ids.windows(2).all(|pair| pair[0] < pair[1])
}

fn sorted_intersects<T: Ord>(left: &[T], right: &[T]) -> bool {
    let mut left_index = 0;
    let mut right_index = 0;
    while left_index < left.len() && right_index < right.len() {
        match left[left_index].cmp(&right[right_index]) {
            std::cmp::Ordering::Less => left_index += 1,
            std::cmp::Ordering::Greater => right_index += 1,
            std::cmp::Ordering::Equal => return true,
        }
    }
    false
}

fn frozen_closure_is_present(world: &WorldState, frozen: &DraftFrozenClosureIdsV2) -> bool {
    let Some(grid) = world.grids.get(&frozen.grid_id) else {
        return false;
    };
    if grid.blocks.keys().ne(frozen.block_ids.iter())
        || frozen
            .inventory_ids
            .iter()
            .any(|inventory_id| !world.inventories.contains_key(inventory_id))
        || frozen
            .player_ids
            .iter()
            .any(|player_id| world.player.get(player_id).is_none())
        || frozen
            .operation_actor_ids
            .iter()
            .any(|player_id| !world.processed_operations.contains_key(player_id))
        || frozen
            .internal_contacts
            .iter()
            .any(|contact| !world.active_contact_pairs.contains(contact))
    {
        return false;
    }
    let machine_ids = grid
        .blocks
        .keys()
        .filter(|block_id| world.production_queues.contains_key(*block_id))
        .cloned()
        .collect::<Vec<_>>();
    if machine_ids != frozen.machine_block_ids {
        return false;
    }
    let job_ids = frozen
        .machine_block_ids
        .iter()
        .filter_map(|machine| world.production_queues.get(machine))
        .flatten()
        .map(|job| job.job_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if job_ids != frozen.job_ids {
        return false;
    }
    let closure_bodies = std::iter::once(frozen.grid_id.clone())
        .chain(
            frozen
                .player_ids
                .iter()
                .map(|player| player_body_id_v2(player)),
        )
        .collect::<BTreeSet<_>>();
    let current_contacts = world
        .active_contact_pairs
        .iter()
        .filter(|contact| {
            closure_bodies.contains(&contact.body_a) || closure_bodies.contains(&contact.body_b)
        })
        .cloned()
        .collect::<Vec<_>>();
    current_contacts == frozen.internal_contacts
}

fn locked_closure_matches(world: &WorldState, lock: &DraftAggregateTransferLockV2) -> bool {
    if !frozen_closure_is_present(world, &lock.frozen) {
        return false;
    }
    let context = DraftGridTransferContextV2 {
        transfer_id: lock.binding.transfer_id.clone(),
        source_assignment_generation: lock.binding.source_assignment_generation,
        destination_assignment_generation: lock.binding.destination_assignment_generation,
        source_fencing_token: lock.binding.source_fencing_token,
        destination_fencing_token: lock.binding.destination_fencing_token,
        placement: BundledPlacementPlan {
            root_aggregate_id: lock.binding.root_aggregate_id.clone(),
            source_cell_key: lock.binding.source_cell_key.clone(),
            source_cell_id: lock.binding.source_cell_id.clone(),
            destination_cell_key: lock.binding.destination_cell_key.clone(),
            destination_cell_id: lock.binding.destination_cell_id.clone(),
            members: lock.binding.members.clone(),
            member_root: lock.binding.member_root.clone(),
        },
        production_job_origins: lock.production_job_origins.clone(),
    };
    let mut live_context = context;
    live_context.source_fencing_token = world.fencing_token;
    extract_draft_grid_closure_from_validated_world(
        world,
        &lock.binding.root_aggregate_id,
        &live_context,
    )
    .is_ok_and(|current| {
        current.closure_root == lock.binding.closure_root
            && current.conservation_root == lock.binding.conservation_root
            && DraftFrozenClosureIdsV2::from_package(&current) == lock.frozen
    })
}

fn source_lock_matches(world: &WorldState, lock: &DraftAggregateTransferLockV2) -> bool {
    lock.source_event_sequence == world.event_sequence
        && lock.source_event_hash == world.last_event_hash
        && lock.source_base_world_hash == world.state_hash()
        && lock.prepared_at_simulation_tick == world.simulation_tick
        && locked_closure_matches(world, lock)
}

fn frozen_closure_is_absent(world: &WorldState, frozen: &DraftFrozenClosureIdsV2) -> bool {
    !world.grids.contains_key(&frozen.grid_id)
        && frozen
            .block_ids
            .iter()
            .all(|block_id| world.block_grid(block_id).is_none())
        && frozen
            .inventory_ids
            .iter()
            .all(|inventory_id| !world.inventories.contains_key(inventory_id))
        && frozen
            .machine_block_ids
            .iter()
            .all(|machine| !world.production_queues.contains_key(machine))
        && frozen.job_ids.iter().all(|job_id| {
            world
                .production_queues
                .values()
                .flatten()
                .all(|job| &job.job_id != job_id)
        })
        && frozen
            .player_ids
            .iter()
            .all(|player_id| world.player.get(player_id).is_none())
        && frozen
            .operation_actor_ids
            .iter()
            .all(|player_id| !world.processed_operations.contains_key(player_id))
        && frozen
            .internal_contacts
            .iter()
            .all(|contact| !world.active_contact_pairs.contains(contact))
}

fn active_v1_transfer_conflicts(
    world: &WorldState,
    transfer_id: &str,
    frozen: &DraftFrozenClosureIdsV2,
) -> bool {
    world.transfer_witnesses.contains_key(transfer_id)
        || world.player_transfer_locks.iter().any(|(player_id, lock)| {
            lock.transfer_id == transfer_id || frozen.player_ids.binary_search(player_id).is_ok()
        })
        || world
            .player_transfer_reservations
            .values()
            .any(|reservation| {
                reservation.transfer_id == transfer_id
                    || frozen
                        .player_ids
                        .binary_search(&reservation.player_id)
                        .is_ok()
                    || frozen
                        .inventory_ids
                        .binary_search(&reservation.inventory_id)
                        .is_ok()
            })
}

fn active_v1_transfer_id_conflicts(world: &WorldState, transfer_id: &str) -> bool {
    world.transfer_witnesses.contains_key(transfer_id)
        || world
            .player_transfer_locks
            .values()
            .any(|lock| lock.transfer_id == transfer_id)
        || world
            .player_transfer_reservations
            .values()
            .any(|reservation| reservation.transfer_id == transfer_id)
}

fn grid_transfer_witness(
    package: &DraftGridClosurePackageV2,
    direction: TransferWitnessDirection,
    ledger_vector: DraftGridTransferLedgerVectorV2,
) -> TransferConservationWitness {
    TransferConservationWitness {
        transfer_id: package.transfer_id.clone(),
        package_hash: package.package_hash.clone(),
        counterparty_cell_id: match direction {
            TransferWitnessDirection::Import => package.source_cell_id.clone(),
            TransferWitnessDirection::Export => package.destination_cell_id.clone(),
        },
        direction,
        contents: ledger_vector.as_contents(),
    }
}

fn insert_grid_transfer_witness(
    world: &mut WorldState,
    witness: TransferConservationWitness,
) -> Result<(), DraftGridClosureError> {
    if world.transfer_witnesses.contains_key(&witness.transfer_id) {
        return Err(DraftGridClosureError::Changed(
            "grid transfer witness already exists without its committed record".into(),
        ));
    }
    let contents = &witness.contents;
    let ledger = &mut world.ledger;
    match witness.direction {
        TransferWitnessDirection::Import => {
            ledger.transfer_imported_ore = ledger
                .transfer_imported_ore
                .checked_add(contents.ore)
                .ok_or_else(|| {
                DraftGridClosureError::Unsupported("grid import ore ledger overflowed".into())
            })?;
            ledger.transfer_imported_refined = ledger
                .transfer_imported_refined
                .checked_add(contents.refined_material)
                .ok_or_else(|| {
                    DraftGridClosureError::Unsupported(
                        "grid import refined ledger overflowed".into(),
                    )
                })?;
            ledger.transfer_imported_components = ledger
                .transfer_imported_components
                .checked_add(contents.components)
                .ok_or_else(|| {
                    DraftGridClosureError::Unsupported(
                        "grid import component ledger overflowed".into(),
                    )
                })?;
        }
        TransferWitnessDirection::Export => {
            ledger.transfer_exported_ore = ledger
                .transfer_exported_ore
                .checked_add(contents.ore)
                .ok_or_else(|| {
                DraftGridClosureError::Unsupported("grid export ore ledger overflowed".into())
            })?;
            ledger.transfer_exported_refined = ledger
                .transfer_exported_refined
                .checked_add(contents.refined_material)
                .ok_or_else(|| {
                    DraftGridClosureError::Unsupported(
                        "grid export refined ledger overflowed".into(),
                    )
                })?;
            ledger.transfer_exported_components = ledger
                .transfer_exported_components
                .checked_add(contents.components)
                .ok_or_else(|| {
                    DraftGridClosureError::Unsupported(
                        "grid export component ledger overflowed".into(),
                    )
                })?;
        }
    }
    world
        .transfer_witnesses
        .insert(witness.transfer_id.clone(), witness);
    Ok(())
}

fn stage_prepared_grid_lock_v2(
    state: &DraftGridTransferCellStateV2,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<DraftGridTransferCellStateV2, DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if !matches!(
        authority.phase,
        TransferPhase::Prepared | TransferPhase::Quarantined
    ) || (authority.phase == TransferPhase::Quarantined
        && (!authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
            || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
            || authority.quarantine_receipt_hash.is_none()))
    {
        return Err(DraftGridClosureError::Invalid(
            "directory authority is not in a lockable precommit phase".into(),
        ));
    }
    if state.base.cell_id != package.source_cell_id
        || state.base.fencing_token != authority.live_source_fencing_token
        || package
            .production_job_origins
            .iter()
            .any(|(job_id, origin)| state.production_job_origins.get(job_id) != Some(origin))
    {
        return Err(DraftGridClosureError::Invalid(
            "source draft cell does not own the package fence and authoritative job origins".into(),
        ));
    }
    let expected = DraftAggregateTransferLockV2::from_package(package);
    if let Some(existing) = state.aggregate_locks.get(&package.root_aggregate_id) {
        if existing == &expected {
            return Ok(state.clone());
        }
        return Err(DraftGridClosureError::Changed(
            "grid root is already locked by different transfer material".into(),
        ));
    }
    if !source_lock_matches(&state.base, &expected) {
        return Err(DraftGridClosureError::Changed(
            "source closure changed before the aggregate lock became durable".into(),
        ));
    }
    if state.aggregate_locks.values().any(|lock| {
        lock.binding.transfer_id == package.transfer_id || lock.frozen.overlaps(&expected.frozen)
    }) || state.aggregate_reservations.values().any(|reservation| {
        reservation.binding.transfer_id == package.transfer_id
            || reservation.frozen.overlaps(&expected.frozen)
    }) || state.committed_exports.contains_key(&package.transfer_id)
        || state.abort_witnesses.contains_key(&package.transfer_id)
    {
        return Err(DraftGridClosureError::Changed(
            "another aggregate transfer already freezes a closure subject".into(),
        ));
    }
    let mut next = state.clone();
    next.aggregate_locks
        .insert(package.root_aggregate_id.clone(), expected);
    next.seal()?;
    Ok(next)
}

fn stage_grid_quarantine_v2(
    state: &DraftGridTransferCellStateV2,
    trusted_now_unix_ms: u64,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<
    (
        DraftGridTransferCellStateV2,
        DraftGridTransferQuarantineReceiptV2,
    ),
    DraftGridClosureError,
> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if state.base.cell_id != package.destination_cell_id
        || state.base.fencing_token != authority.live_destination_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "destination draft cell does not own the live directory fence".into(),
        ));
    }
    if let Some(existing) = state.aggregate_reservations.get(&package.transfer_id) {
        let receipt = existing.receipt();
        if existing.binding != DraftGridTransferBindingV2::from_package(package)
            || existing.frozen != DraftFrozenClosureIdsV2::from_package(package)
        {
            return Err(DraftGridClosureError::Changed(
                "quarantine retry changed immutable package or closure material".into(),
            ));
        }
        receipt.validate()?;
        let phase_matches = match authority.phase {
            TransferPhase::Prepared => {
                authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
                    && !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
                    && authority.quarantine_receipt_hash.is_none()
            }
            TransferPhase::Quarantined => {
                authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
                    && authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
                    && authority.quarantine_receipt_hash.as_deref()
                        == Some(receipt.receipt_hash.as_str())
            }
            _ => false,
        };
        if !phase_matches {
            return Err(DraftGridClosureError::Invalid(
                "quarantine retry lacks the matching durable directory authority".into(),
            ));
        }
        return Ok((state.clone(), receipt));
    }
    if authority.phase != TransferPhase::Prepared
        || !authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || authority.quarantine_receipt_hash.is_some()
        || trusted_now_unix_ms == 0
    {
        return Err(DraftGridClosureError::Invalid(
            "directory authority is not awaiting first destination quarantine".into(),
        ));
    }
    validate_destination_conflicts(&state.base, package)?;
    let frozen = DraftFrozenClosureIdsV2::from_package(package);
    if state.aggregate_locks.values().any(|lock| {
        lock.binding.transfer_id == package.transfer_id || lock.frozen.overlaps(&frozen)
    }) || state.aggregate_reservations.values().any(|reservation| {
        reservation.binding.transfer_id == package.transfer_id
            || reservation.frozen.overlaps(&frozen)
    }) || state.committed_exports.contains_key(&package.transfer_id)
        || state.abort_witnesses.contains_key(&package.transfer_id)
    {
        return Err(DraftGridClosureError::Changed(
            "destination has an overlapping aggregate lock or reservation".into(),
        ));
    }
    let mut receipt = DraftGridTransferQuarantineReceiptV2 {
        schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
        transfer_id: package.transfer_id.clone(),
        root_aggregate_id: package.root_aggregate_id.clone(),
        package_hash: package.package_hash.clone(),
        closure_root: package.closure_root.clone(),
        conservation_root: package.conservation_root.clone(),
        member_root: package.member_root.clone(),
        destination_cell_id: package.destination_cell_id.clone(),
        destination_assignment_generation: package.destination_assignment_generation,
        destination_fencing_token: package.destination_fencing_token,
        destination_event_sequence: state.base.event_sequence,
        destination_base_world_hash: state.base.state_hash(),
        destination_draft_world_hash: state.state_hash.clone(),
        quarantined_at_unix_ms: trusted_now_unix_ms,
        receipt_hash: String::new(),
    };
    receipt.receipt_hash = receipt.calculate_hash()?;
    receipt.validate()?;
    let reservation = DraftAggregateTransferReservationV2::from_receipt(package, &receipt);
    reservation.validate()?;
    let mut next = state.clone();
    next.aggregate_reservations
        .insert(package.transfer_id.clone(), reservation);
    next.seal()?;
    Ok((next, receipt))
}

fn stage_committed_grid_export_v2(
    state: &DraftGridTransferCellStateV2,
    trusted_now_unix_ms: u64,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<(DraftGridTransferCellStateV2, DraftGridExportProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if !authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || authority.quarantine_receipt_hash.is_none()
        || state.base.cell_id != package.source_cell_id
        || state.base.fencing_token != authority.live_source_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "source export lacks exact committed directory and live-fence authority".into(),
        ));
    }
    if let Some(existing) = state.committed_exports.get(&package.transfer_id) {
        existing.validate_request(package, authority)?;
        let proof = existing.proof();
        let directory_proof_matches =
            authority.source_export_proof_hash.as_deref() == Some(proof.proof_hash.as_str());
        let retry_phase_matches = match authority.phase {
            TransferPhase::Committed => {
                authority.source_export_proof_hash.is_none() || directory_proof_matches
            }
            TransferPhase::Imported | TransferPhase::Finalized => directory_proof_matches,
            _ => false,
        };
        if state
            .aggregate_locks
            .contains_key(&package.root_aggregate_id)
            || !frozen_closure_is_absent(&state.base, &existing.frozen)
            || existing
                .frozen
                .job_ids
                .iter()
                .any(|job_id| state.production_job_origins.contains_key(job_id))
            || state.base.transfer_witnesses.get(&package.transfer_id)
                != Some(&existing.conservation_witness)
            || !retry_phase_matches
        {
            return Err(DraftGridClosureError::Changed(
                "source-export retry conflicts with its durable record".into(),
            ));
        }
        return Ok((state.clone(), proof));
    }
    if authority.phase != TransferPhase::Committed
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourceExport)
        || authority.source_export_proof_hash.is_some()
        || trusted_now_unix_ms == 0
        || state
            .aggregate_reservations
            .contains_key(&package.transfer_id)
        || state.abort_witnesses.contains_key(&package.transfer_id)
        || active_v1_transfer_id_conflicts(&state.base, &package.transfer_id)
    {
        return Err(DraftGridClosureError::Invalid(
            "source export is not at a clean committed boundary".into(),
        ));
    }
    let expected_lock = DraftAggregateTransferLockV2::from_package(package);
    if state.aggregate_locks.get(&package.root_aggregate_id) != Some(&expected_lock)
        || !source_lock_matches(&state.base, &expected_lock)
    {
        return Err(DraftGridClosureError::Changed(
            "source export no longer matches the exact frozen package closure".into(),
        ));
    }

    let ledger_vector = DraftGridTransferLedgerVectorV2::from_package(package)?;
    let conservation_witness =
        grid_transfer_witness(package, TransferWitnessDirection::Export, ledger_vector);
    let mut next = state.clone();

    for contact in &package.active_internal_contacts {
        if !next.base.active_contact_pairs.remove(contact) {
            return Err(DraftGridClosureError::Changed(
                "source export lost an internal contact from the frozen closure".into(),
            ));
        }
    }
    for (machine_id, expected_queue) in &package.production_queues {
        if next.base.production_queues.remove(machine_id).as_ref() != Some(expected_queue) {
            return Err(DraftGridClosureError::Changed(
                "source export lost or changed a frozen production queue".into(),
            ));
        }
    }
    for (job_id, expected_origin) in &package.production_job_origins {
        if next.production_job_origins.remove(job_id).as_ref() != Some(expected_origin) {
            return Err(DraftGridClosureError::Changed(
                "source export lost or changed production provenance".into(),
            ));
        }
    }
    for (inventory_id, expected_inventory) in &package.cargo_inventories {
        if next.base.inventories.remove(inventory_id).as_ref() != Some(expected_inventory) {
            return Err(DraftGridClosureError::Changed(
                "source export lost or changed a frozen cargo inventory".into(),
            ));
        }
    }
    for (player_id, packaged_player) in &package.players {
        if next
            .base
            .inventories
            .remove(&packaged_player.inventory.inventory_id)
            .as_ref()
            != Some(&packaged_player.inventory)
            || next.base.processed_operations.remove(player_id).as_ref()
                != packaged_player.operation_history.as_ref()
            || next.base.player.by_id.remove(player_id).as_ref()
                != Some(&packaged_player.source_player)
        {
            return Err(DraftGridClosureError::Changed(
                "source export lost or changed a frozen rider closure".into(),
            ));
        }
    }
    if next.base.grids.remove(&package.root_aggregate_id).as_ref() != Some(&package.grid)
        || next
            .aggregate_locks
            .remove(&package.root_aggregate_id)
            .as_ref()
            != Some(&expected_lock)
    {
        return Err(DraftGridClosureError::Changed(
            "source export lost or changed its frozen grid root".into(),
        ));
    }
    if package
        .players
        .contains_key(&next.base.player.primary_player_id)
    {
        next.base.player.primary_player_id = next
            .base
            .player
            .by_id
            .keys()
            .next()
            .cloned()
            .unwrap_or_default();
    }
    insert_grid_transfer_witness(&mut next.base, conservation_witness.clone())?;
    if !next.base.conservation().valid {
        return Err(DraftGridClosureError::Invalid(
            "source export does not conserve its exact transfer vector".into(),
        ));
    }
    let mut record = DraftGridExportRecordV2::new(
        state,
        package,
        authority,
        ledger_vector,
        conservation_witness,
        trusted_now_unix_ms,
    )?;
    next.base.event_sequence = record.export_event_sequence;
    next.base
        .last_event_hash
        .clone_from(&record.export_event_hash);
    record.seal_resulting_active_world_hash(&next)?;
    let proof = record.proof();
    next.committed_exports
        .insert(package.transfer_id.clone(), record);
    next.seal()?;
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    Ok((next, proof))
}

fn stage_aborted_grid_cleanup_v2(
    state: &DraftGridTransferCellStateV2,
    trusted_now_unix_ms: u64,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<(DraftGridTransferCellStateV2, DraftGridAbortCleanupProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    let side = if state.base.cell_id == package.source_cell_id {
        DraftGridTransferAbortSideV2::Source
    } else if state.base.cell_id == package.destination_cell_id {
        DraftGridTransferAbortSideV2::Destination
    } else {
        return Err(DraftGridClosureError::Invalid(
            "abort cleanup was presented to an unrelated cell".into(),
        ));
    };
    let live_fencing_token = match side {
        DraftGridTransferAbortSideV2::Source => authority.live_source_fencing_token,
        DraftGridTransferAbortSideV2::Destination => authority.live_destination_fencing_token,
    };
    if state.base.fencing_token != live_fencing_token {
        return Err(DraftGridClosureError::Invalid(
            "abort cleanup caller does not own the live directory fence".into(),
        ));
    }
    if let Some(existing) = state.abort_witnesses.get(&package.transfer_id) {
        if !matches!(
            authority.phase,
            TransferPhase::Aborting | TransferPhase::Aborted
        ) {
            return Err(DraftGridClosureError::Invalid(
                "abort witness retry lacks terminal directory authority".into(),
            ));
        }
        existing.validate_request(package, authority, side)?;
        let proof = existing.cleanup_proof();
        proof.validate().map_err(DraftGridClosureError::Invalid)?;
        return Ok((state.clone(), proof));
    }
    if authority.phase != TransferPhase::Aborting || trusted_now_unix_ms == 0 {
        return Err(DraftGridClosureError::Invalid(
            "only an aborting precommit transfer may clean cell authority".into(),
        ));
    }
    let mut next = state.clone();
    let removed_authority = match side {
        DraftGridTransferAbortSideV2::Source => {
            if authority.has_proof(DraftGridDirectoryProofKindV2::SourceAbort) {
                return Err(DraftGridClosureError::Changed(
                    "directory retains a source-abort proof absent from cell state".into(),
                ));
            }
            if state
                .aggregate_locks
                .get(&package.root_aggregate_id)
                .is_some_and(|existing| {
                    existing != &DraftAggregateTransferLockV2::from_package(package)
                })
            {
                return Err(DraftGridClosureError::Changed(
                    "source abort does not match the exact aggregate lock".into(),
                ));
            }
            let removed = next
                .aggregate_locks
                .remove(&package.root_aggregate_id)
                .is_some();
            if !removed && authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare) {
                return Err(DraftGridClosureError::Changed(
                    "directory proves a source lock that is absent from cell state".into(),
                ));
            }
            removed
        }
        DraftGridTransferAbortSideV2::Destination => {
            if authority.has_proof(DraftGridDirectoryProofKindV2::DestinationAbort) {
                return Err(DraftGridClosureError::Changed(
                    "directory retains a destination-abort proof absent from cell state".into(),
                ));
            }
            if !frozen_closure_is_absent(
                &state.base,
                &DraftFrozenClosureIdsV2::from_package(package),
            ) {
                return Err(DraftGridClosureError::Changed(
                    "destination abort cannot remove imported closure subjects".into(),
                ));
            }
            if state
                .aggregate_reservations
                .get(&package.transfer_id)
                .is_some_and(|existing| {
                    existing.binding != DraftGridTransferBindingV2::from_package(package)
                        || existing.frozen != DraftFrozenClosureIdsV2::from_package(package)
                        || authority.quarantine_receipt_hash.as_deref()
                            != Some(existing.receipt_hash.as_str())
                })
            {
                return Err(DraftGridClosureError::Changed(
                    "destination abort does not match the exact quarantine reservation".into(),
                ));
            }
            let removed = next
                .aggregate_reservations
                .remove(&package.transfer_id)
                .is_some();
            if !removed && authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
            {
                return Err(DraftGridClosureError::Changed(
                    "directory proves a destination reservation that is absent from cell state"
                        .into(),
                ));
            }
            removed
        }
    };
    let witness = DraftGridTransferAbortWitnessV2::new(
        state,
        &next,
        package,
        side,
        removed_authority,
        authority,
        trusted_now_unix_ms,
    )?;
    next.abort_witnesses
        .insert(package.transfer_id.clone(), witness.clone());
    next.seal()?;
    let proof = witness.cleanup_proof();
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    Ok((next, proof))
}

fn context_from_package(package: &DraftGridClosurePackageV2) -> DraftGridTransferContextV2 {
    DraftGridTransferContextV2 {
        transfer_id: package.transfer_id.clone(),
        source_assignment_generation: package.source_assignment_generation,
        destination_assignment_generation: package.destination_assignment_generation,
        source_fencing_token: package.source_fencing_token,
        destination_fencing_token: package.destination_fencing_token,
        placement: BundledPlacementPlan {
            root_aggregate_id: package.root_aggregate_id.clone(),
            source_cell_key: package.source_cell_key.clone(),
            source_cell_id: package.source_cell_id.clone(),
            destination_cell_key: package.destination_cell_key.clone(),
            destination_cell_id: package.destination_cell_id.clone(),
            members: package.members.clone(),
            member_root: package.member_root.clone(),
        },
        production_job_origins: package.production_job_origins.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::package_fixture;
    use super::*;

    fn destination_state(package: &DraftGridClosurePackageV2) -> DraftGridTransferCellStateV2 {
        let mut destination = WorldState::genesis_for_cell(801, &package.destination_cell_key)
            .expect("destination world derives");
        destination.fencing_token = package.destination_fencing_token;
        DraftGridTransferCellStateV2::new(destination).expect("draft destination seals")
    }

    fn committed_source(
        source: WorldState,
        package: &DraftGridClosurePackageV2,
    ) -> (DraftGridTransferCellStateV2, DraftGridDirectoryAuthorityV2) {
        let source = DraftGridTransferCellStateV2::new(source).expect("source envelope seals");
        let prepared = DraftGridDirectoryAuthorityV2::for_package(package, TransferPhase::Prepared);
        let locked =
            stage_prepared_grid_lock_v2(&source, package, &prepared).expect("source closure locks");
        let mut quarantine_authority = prepared.clone();
        quarantine_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
        let (_, receipt) = stage_grid_quarantine_v2(
            &destination_state(package),
            1_800_000_000_000,
            package,
            &quarantine_authority,
        )
        .expect("destination quarantine derives receipt");
        let mut committed = quarantine_authority;
        committed.phase = TransferPhase::Committed;
        committed
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationQuarantine);
        committed.quarantine_receipt_hash = Some(receipt.receipt_hash);
        (locked, committed)
    }

    #[test]
    fn prepared_lock_freezes_every_subject_family_and_retries_exactly() {
        let (source, _, package) = package_fixture();
        let state = DraftGridTransferCellStateV2::new(source.clone()).expect("draft source seals");
        let authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked =
            stage_prepared_grid_lock_v2(&state, &package, &authority).expect("closure locks");
        assert_eq!(locked.base, source);
        assert_ne!(locked.state_hash, state.state_hash);
        assert_eq!(locked.aggregate_locks.len(), 1);
        for subject in [
            package.root_aggregate_id.as_str(),
            "block-core",
            package
                .cargo_inventories
                .keys()
                .next()
                .expect("cargo exists"),
            "player-local",
            package.players["player-local"]
                .inventory
                .inventory_id
                .as_str(),
        ] {
            assert_eq!(
                locked.locked_transfer_for_subject(subject),
                Some(package.transfer_id.as_str())
            );
        }
        let retry = stage_prepared_grid_lock_v2(&locked, &package, &authority)
            .expect("exact lock retry succeeds");
        assert_eq!(retry, locked);
    }

    #[test]
    fn locked_closure_tamper_fails_envelope_validation() {
        let (source, _, package) = package_fixture();
        let state = DraftGridTransferCellStateV2::new(source).expect("source seals");
        let authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked =
            stage_prepared_grid_lock_v2(&state, &package, &authority).expect("closure locks");

        let mut control_tamper = locked.clone();
        control_tamper
            .base
            .grids
            .get_mut(&package.root_aggregate_id)
            .expect("grid exists")
            .control_linear_input
            .x += 0.25;
        assert!(control_tamper.seal().is_err());

        let mut cargo_tamper = locked.clone();
        cargo_tamper
            .base
            .inventories
            .get_mut(
                package
                    .cargo_inventories
                    .keys()
                    .next()
                    .expect("cargo exists"),
            )
            .expect("cargo remains present")
            .contents
            .ore += 1;
        assert!(cargo_tamper.seal().is_err());

        let mut rider_tamper = locked;
        rider_tamper
            .base
            .player
            .get_mut("player-local")
            .expect("rider exists")
            .experience += 1;
        assert!(rider_tamper.seal().is_err());
    }

    #[test]
    fn quarantine_receipt_and_reservation_are_exact_and_canonical() {
        let (_, _, package) = package_fixture();
        let state = destination_state(&package);
        let mut authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
        let (reserved, receipt) =
            stage_grid_quarantine_v2(&state, 1_800_000_000_000, &package, &authority)
                .expect("destination quarantines");
        receipt.validate().expect("receipt validates");
        assert_eq!(reserved.base, state.base);
        assert_ne!(reserved.state_hash, state.state_hash);
        assert_eq!(reserved.aggregate_reservations.len(), 1);
        assert_eq!(receipt.destination_draft_world_hash, state.state_hash);
        assert_eq!(
            receipt.receipt_hash,
            "9dc3e8495ac5f8ca3eaffa73ee4f6f77efef2a6226f586dee9c80a0cd6364007"
        );

        let (retry_state, retry_receipt) =
            stage_grid_quarantine_v2(&reserved, 1_800_000_000_001, &package, &authority)
                .expect("exact quarantine retry succeeds");
        assert_eq!(retry_state, reserved);
        assert_eq!(retry_receipt, receipt);

        let mut quarantined_authority = authority.clone();
        quarantined_authority.phase = TransferPhase::Quarantined;
        quarantined_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationQuarantine);
        quarantined_authority.quarantine_receipt_hash = Some(receipt.receipt_hash.clone());
        let (restart_state, restart_receipt) = stage_grid_quarantine_v2(
            &reserved,
            1_800_000_100_000,
            &package,
            &quarantined_authority,
        )
        .expect("post-directory-commit quarantine retry succeeds");
        assert_eq!(restart_state, reserved);
        assert_eq!(restart_receipt, receipt);

        let mut wrong_receipt = quarantined_authority;
        wrong_receipt.quarantine_receipt_hash = Some("ab".repeat(32));
        assert!(
            stage_grid_quarantine_v2(&reserved, 1_800_000_100_001, &package, &wrong_receipt,)
                .is_err()
        );

        let bytes = reserved.encode_canonical().expect("state encodes");
        let decoded = DraftGridTransferCellStateV2::decode_canonical(&bytes)
            .expect("canonical state decodes");
        assert_eq!(decoded, reserved);
        let mut noncanonical = vec![b' '];
        noncanonical.extend(bytes);
        assert!(DraftGridTransferCellStateV2::decode_canonical(&noncanonical).is_err());
    }

    #[test]
    fn precommit_abort_removes_only_exact_lock_and_reservation() {
        let (source, _, package) = package_fixture();
        let source_state = DraftGridTransferCellStateV2::new(source).expect("source seals");
        let prepared =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked =
            stage_prepared_grid_lock_v2(&source_state, &package, &prepared).expect("source locks");

        let destination = destination_state(&package);
        let mut quarantine_authority = prepared.clone();
        quarantine_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
        let (reserved, receipt) = stage_grid_quarantine_v2(
            &destination,
            1_800_000_000_000,
            &package,
            &quarantine_authority,
        )
        .expect("destination reserves");

        let mut abort_authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Aborting);
        abort_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
        abort_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationQuarantine);
        abort_authority.quarantine_receipt_hash = Some(receipt.receipt_hash.clone());
        let (source_clean, source_witness) =
            stage_aborted_grid_cleanup_v2(&locked, 1_800_000_010_000, &package, &abort_authority)
                .expect("source unlocks");
        let (destination_clean, destination_witness) =
            stage_aborted_grid_cleanup_v2(&reserved, 1_800_000_010_001, &package, &abort_authority)
                .expect("destination unreserves");
        assert_eq!(source_clean.base, source_state.base);
        assert_eq!(destination_clean.base, destination.base);
        assert!(source_clean.aggregate_locks.is_empty());
        assert!(destination_clean.aggregate_reservations.is_empty());
        assert!(source_witness.removed_authority);
        assert!(destination_witness.removed_authority);
        let (source_retry, source_retry_witness) = stage_aborted_grid_cleanup_v2(
            &source_clean,
            1_800_000_020_000,
            &package,
            &abort_authority,
        )
        .expect("source cleanup retry succeeds");
        assert_eq!(source_retry, source_clean);
        assert_eq!(source_retry_witness, source_witness);
        let (destination_retry, destination_retry_witness) = stage_aborted_grid_cleanup_v2(
            &destination_clean,
            1_800_000_020_001,
            &package,
            &abort_authority,
        )
        .expect("destination cleanup retry succeeds");
        assert_eq!(destination_retry, destination_clean);
        assert_eq!(destination_retry_witness, destination_witness);
    }

    #[test]
    fn successor_fence_recovers_lock_and_old_worker_is_rejected() {
        let (source, _, package) = package_fixture();
        let state = DraftGridTransferCellStateV2::new(source).expect("source seals");
        let prepared =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let mut locked =
            stage_prepared_grid_lock_v2(&state, &package, &prepared).expect("source locks");

        locked.base.fencing_token += 1;
        locked.seal().expect("successor fence preserves closure");
        assert!(stage_prepared_grid_lock_v2(&locked, &package, &prepared).is_err());

        let mut successor = prepared;
        successor.live_source_assignment_generation += 1;
        successor.live_source_fencing_token += 1;
        let recovered = stage_prepared_grid_lock_v2(&locked, &package, &successor)
            .expect("successor recovers exact lock");
        assert_eq!(recovered, locked);

        successor.phase = TransferPhase::Aborting;
        let (clean, witness) =
            stage_aborted_grid_cleanup_v2(&recovered, 1_800_000_030_000, &package, &successor)
                .expect("successor aborts exact lock");
        assert!(clean.aggregate_locks.is_empty());
        assert_eq!(witness.fencing_token, locked.base.fencing_token);
    }

    #[test]
    fn committed_export_removes_the_whole_closure_and_conserves_installed_components_once() {
        let (source, _, package) = package_fixture();
        let initial_ledger = source.ledger.clone();
        let initial_clock = source.production_clock.clone();
        let initial_event_sequence = source.event_sequence;
        let (locked, committed) = committed_source(source, &package);
        let expected_vector =
            DraftGridTransferLedgerVectorV2::from_package(&package).expect("vector derives");

        let (exported, proof) =
            stage_committed_grid_export_v2(&locked, 1_800_000_010_000, &package, &committed)
                .expect("exact closure exports");
        proof.validate().expect("source-export proof validates");
        let mut substituted_proof = proof.clone();
        substituted_proof.resulting_active_world_hash = "ab".repeat(32);
        assert!(substituted_proof.validate().is_err());
        assert_eq!(proof.ledger_vector, expected_vector);
        assert_eq!(
            expected_vector.components,
            package.conservation.transferable_contents.components
                + package.conservation.installed_components
        );
        assert!(exported.aggregate_locks.is_empty());
        assert_eq!(exported.committed_exports.len(), 1);
        assert!(frozen_closure_is_absent(
            &exported.base,
            &DraftFrozenClosureIdsV2::from_package(&package)
        ));
        assert!(
            package
                .production_job_origins
                .keys()
                .all(|job_id| !exported.production_job_origins.contains_key(job_id))
        );
        assert_eq!(exported.base.production_clock, initial_clock);
        assert_eq!(
            exported.base.event_sequence,
            initial_event_sequence
                .checked_add(1)
                .expect("frontier advances")
        );
        assert_eq!(exported.base.last_event_hash, proof.event_hash);
        assert_eq!(
            exported.base.ledger.transfer_exported_ore,
            initial_ledger.transfer_exported_ore + expected_vector.ore
        );
        assert_eq!(
            exported.base.ledger.transfer_exported_refined,
            initial_ledger.transfer_exported_refined + expected_vector.refined_material
        );
        assert_eq!(
            exported.base.ledger.transfer_exported_components,
            initial_ledger.transfer_exported_components + expected_vector.components
        );
        assert!(exported.base.conservation().valid);

        let (retry, retry_proof) =
            stage_committed_grid_export_v2(&exported, 1_800_000_020_000, &package, &committed)
                .expect("exact export retry returns its proof");
        assert_eq!(retry, exported);
        assert_eq!(retry_proof, proof);

        let mut directory_proven = committed.clone();
        directory_proven
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceExport);
        directory_proven.source_export_proof_hash = Some(proof.proof_hash.clone());
        let (_, committed_proof_retry) = stage_committed_grid_export_v2(
            &exported,
            1_800_000_025_000,
            &package,
            &directory_proven,
        )
        .expect("directory-proven committed retry succeeds");
        assert_eq!(committed_proof_retry, proof);

        let mut imported = directory_proven.clone();
        imported.phase = TransferPhase::Imported;
        imported
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationImport);
        let (_, imported_proof_retry) =
            stage_committed_grid_export_v2(&exported, 1_800_000_026_000, &package, &imported)
                .expect("imported directory phase retrieves the historical export proof");
        assert_eq!(imported_proof_retry, proof);

        let mut finalized = imported.clone();
        finalized.phase = TransferPhase::Finalized;
        finalized
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationActivation);
        finalized
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceFinalization);
        let (_, finalized_proof_retry) =
            stage_committed_grid_export_v2(&exported, 1_800_000_027_000, &package, &finalized)
                .expect("finalized directory phase retrieves the historical export proof");
        assert_eq!(finalized_proof_retry, proof);

        let mut changed_directory_proof = imported;
        changed_directory_proof.source_export_proof_hash = Some("ab".repeat(32));
        assert!(
            stage_committed_grid_export_v2(
                &exported,
                1_800_000_028_000,
                &package,
                &changed_directory_proof,
            )
            .is_err()
        );
        let mut missing_directory_proof = changed_directory_proof;
        missing_directory_proof.source_export_proof_hash = None;
        assert!(
            stage_committed_grid_export_v2(
                &exported,
                1_800_000_029_000,
                &package,
                &missing_directory_proof,
            )
            .is_err()
        );

        let mut progressed = exported.clone();
        progressed.base.simulation_tick += 1;
        progressed.seal().expect("exported source may progress");
        let (progressed_retry, progressed_proof) =
            stage_committed_grid_export_v2(&progressed, 1_800_000_030_000, &package, &committed)
                .expect("historical export proof survives unrelated world progress");
        assert_eq!(progressed_retry, progressed);
        assert_eq!(progressed_proof, proof);

        let bytes = exported.encode_canonical().expect("exported state encodes");
        assert_eq!(
            DraftGridTransferCellStateV2::decode_canonical(&bytes).expect("exported state decodes"),
            exported
        );
    }

    #[test]
    fn export_record_rejects_partial_state_ledger_tamper_and_vector_overflow() {
        let (source, _, package) = package_fixture();
        let (locked, committed) = committed_source(source, &package);
        let (exported, _) =
            stage_committed_grid_export_v2(&locked, 1_800_000_030_000, &package, &committed)
                .expect("exact closure exports");

        let mut partial = exported.clone();
        partial
            .base
            .grids
            .insert(package.grid.grid_id.clone(), package.grid.clone());
        assert!(partial.seal().is_err());

        let mut ledger_tamper = exported.clone();
        ledger_tamper.base.ledger.transfer_exported_components += 1;
        assert!(ledger_tamper.seal().is_err());

        let mut witness_tamper = exported.clone();
        witness_tamper
            .base
            .transfer_witnesses
            .get_mut(&package.transfer_id)
            .expect("witness exists")
            .contents
            .components += 1;
        assert!(witness_tamper.seal().is_err());

        let mut result_tamper = exported.clone();
        result_tamper
            .committed_exports
            .get_mut(&package.transfer_id)
            .expect("export record exists")
            .resulting_active_world_hash = "ab".repeat(32);
        assert!(result_tamper.seal().is_err());

        let mut overflowing = package;
        overflowing.conservation.transferable_contents.components = u64::MAX;
        overflowing.conservation.installed_components = 1;
        assert!(DraftGridTransferLedgerVectorV2::from_package(&overflowing).is_err());
    }

    #[test]
    fn abort_noop_still_persists_proof_and_retry_survives_world_progress() {
        let (source, _, package) = package_fixture();
        let source = DraftGridTransferCellStateV2::new(source).expect("source seals");
        let destination = destination_state(&package);
        let aborting =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Aborting);

        let (source_proven, source_witness) =
            stage_aborted_grid_cleanup_v2(&source, 1_800_000_040_000, &package, &aborting)
                .expect("empty source cleanup is witnessed");
        let (destination_proven, destination_witness) =
            stage_aborted_grid_cleanup_v2(&destination, 1_800_000_040_001, &package, &aborting)
                .expect("empty destination cleanup is witnessed");
        assert!(!source_witness.removed_authority);
        assert!(!destination_witness.removed_authority);
        assert!(source_witness.event_sequence > 0);
        assert!(destination_witness.event_sequence > 0);
        assert!(valid_blake3_hex(&source_witness.resulting_draft_world_hash));
        assert_ne!(source_proven.state_hash, source.state_hash);
        assert_ne!(destination_proven.state_hash, destination.state_hash);

        let mut progressed = source_proven;
        progressed.base.simulation_tick += 1;
        progressed.seal().expect("unlocked world may progress");
        let (retry, retry_witness) =
            stage_aborted_grid_cleanup_v2(&progressed, 1_800_000_050_000, &package, &aborting)
                .expect("historical cleanup witness remains retryable");
        assert_eq!(retry, progressed);
        assert_eq!(retry_witness, source_witness);
    }

    #[test]
    fn impossible_phase_matrix_and_wrong_live_fence_fail_closed() {
        let (_, _, package) = package_fixture();
        let destination = destination_state(&package);

        let mut impossible =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        impossible
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
        impossible
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationQuarantine);
        impossible.quarantine_receipt_hash = Some("ab".repeat(32));
        assert!(
            stage_grid_quarantine_v2(&destination, 1_800_000_000_000, &package, &impossible,)
                .is_err()
        );

        let mut orphaned_destination_proof = impossible;
        orphaned_destination_proof.phase = TransferPhase::Aborting;
        orphaned_destination_proof
            .proofs
            .remove(&DraftGridDirectoryProofKindV2::SourcePrepare);
        assert!(
            stage_aborted_grid_cleanup_v2(
                &destination,
                1_800_000_000_000,
                &package,
                &orphaned_destination_proof,
            )
            .is_err()
        );

        let mut wrong_live_fence =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        wrong_live_fence
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
        wrong_live_fence.live_destination_fencing_token += 1;
        assert!(
            stage_grid_quarantine_v2(&destination, 1_800_000_000_000, &package, &wrong_live_fence,)
                .is_err()
        );
    }

    #[test]
    fn committed_or_substituted_authority_cannot_abort_or_quarantine() {
        let (_, _, package) = package_fixture();
        let destination = destination_state(&package);
        let committed =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Committed);
        assert!(
            stage_aborted_grid_cleanup_v2(&destination, 1_800_000_000_000, &package, &committed,)
                .is_err()
        );

        let mut wrong =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        wrong
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
        wrong.binding.destination_fencing_token += 1;
        assert!(
            stage_grid_quarantine_v2(&destination, 1_800_000_000_000, &package, &wrong,).is_err()
        );
    }
}
