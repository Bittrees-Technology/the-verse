// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant draft-world-21 lock, quarantine, and pre-commit abort staging.
//!
//! The envelope keeps aggregate authority beside the active world-20 payload
//! without adding fields to that published schema. It is private, in-memory,
//! and unreachable from `Runtime`, `Store`, and the production directory.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[cfg(test)]
use super::event_v17::DraftCanonicalGridEventV17;
use super::event_v17::{DraftGridEventPayloadV17, ValidatedDraftGridEventContextV17};
use super::production::{
    DraftImportedProductionEligibilityV2, DraftImportedProductionOccurrenceControlsV2,
    DraftProductionImportAuthorityV2, DraftProductionJobOriginV2,
    DraftProductionMachineControlKindV2, DraftProductionMachineControlV2,
    derive_imported_production_eligibilities, derive_imported_production_occurrence_controls,
    imported_production_eligibility_map_root, validate_production_job_origins,
};
use super::{
    BundledPlacementMember, BundledPlacementPlan, CellKeyV1, ContactPairKey,
    DRAFT_GRID_TRANSFER_PACKAGE_SCHEMA_VERSION, DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
    DraftGridClosureError, DraftGridClosurePackageV2, DraftGridTransferContextV2,
    InventoryContents, MAX_DRAFT_GRID_BLOCKS, MAX_DRAFT_GRID_CARGO_INVENTORIES,
    MAX_DRAFT_GRID_CONTACTS, MAX_DRAFT_GRID_MEMBERS, MAX_DRAFT_GRID_PRODUCTION_JOBS,
    MAX_DRAFT_GRID_PRODUCTION_QUEUES, MobileAggregateKind, WorldState, celestial,
    extract_draft_grid_closure_from_validated_world, hash_json, player_body_id_v2,
    valid_blake3_hex, valid_stable_id, validate_adjacent_cells,
    validate_destination_conflicts_in_validated_world_v21,
};
use crate::cell_directory::TransferPhase;
use crate::cell_directory_v3::{DirectoryPhaseProofV3, ValidatedGridTransferAuthorityV3};
use crate::event::ProductionMachineOutcome;
use crate::model::{Ledger, TransferConservationWitness, TransferWitnessDirection};
use verse_protocol::CareerSnapshot;

const DRAFT_GRID_CELL_STATE_SCHEMA_VERSION: u32 = 21;
const MAX_DRAFT_GRID_CELL_STATE_BYTES: usize = 32 * 1_024 * 1_024;
const DRAFT_CELL_STATE_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-cell-state/v21\0";
const PREPARE_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-prepare-witness/v2\0";
const PREPARE_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-prepare-event/v2\0";
const PREPARE_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-prepare-proof/v2\0";
const QUARANTINE_RECEIPT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-quarantine-receipt/v2\0";
const QUARANTINE_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-quarantine-witness/v2\0";
const QUARANTINE_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-quarantine-event/v2\0";
const QUARANTINE_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-quarantine-proof/v2\0";
const ABORT_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-abort-witness/v2\0";
const ABORT_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-abort-event/v2\0";
const ABORT_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-abort-proof/v2\0";
const EXPORT_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-witness/v2\0";
const EXPORT_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-event/v2\0";
const EXPORT_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-proof/v2\0";
const EXPORT_RECORD_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-export-record/v2\0";
const IMPORT_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-import-witness/v2\0";
const IMPORT_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-import-event/v2\0";
const IMPORT_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-import-proof/v2\0";
const IMPORT_RECORD_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-import-record/v2\0";
const ACTIVATION_WITNESS_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-activation-witness/v2\0";
const ACTIVATION_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-activation-event/v2\0";
const ACTIVATION_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-activation-proof/v2\0";
const ACTIVATION_RECORD_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-activation-record/v2\0";
const FINALIZATION_WITNESS_HASH_DOMAIN: &[u8] =
    b"the-verse/grid-transfer-finalization-witness/v2\0";
const FINALIZATION_EVENT_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-finalization-event/v2\0";
const FINALIZATION_PROOF_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-finalization-proof/v2\0";
const FINALIZATION_RECORD_HASH_DOMAIN: &[u8] = b"the-verse/grid-transfer-finalization-record/v2\0";
const PRODUCTION_RELEASE_WITNESS_HASH_DOMAIN: &[u8] =
    b"the-verse/grid-transfer-production-release-witness/v2\0";
const PRODUCTION_RELEASE_EVENT_HASH_DOMAIN: &[u8] =
    b"the-verse/grid-transfer-production-release-event/v2\0";
const PRODUCTION_RELEASE_PROOF_HASH_DOMAIN: &[u8] =
    b"the-verse/grid-transfer-production-release-proof/v2\0";
const PRODUCTION_RELEASE_RECORD_HASH_DOMAIN: &[u8] =
    b"the-verse/grid-transfer-production-release-record/v2\0";
const PRODUCTION_RELEASE_OUTCOMES_ROOT_DOMAIN: &[u8] =
    b"the-verse/grid-transfer-production-release-outcomes/v2\0";
const PRODUCTION_OCCURRENCE_HISTORY_ENTRY_DOMAIN: &[u8] =
    b"the-verse/grid-transfer-production-occurrence-history/v2\0";
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
    prepare_event_sequence: u64,
    prepare_event_hash: String,
    prepare_event_payload_hash: String,
    prepare_mutation_witness_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftGridPrepareProofV2 {
    pub(crate) transfer_id: String,
    pub(crate) root_aggregate_id: String,
    pub(crate) member_root: String,
    pub(crate) package_hash: String,
    pub(crate) source_cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) fencing_token: u64,
    pub(crate) prior_event_sequence: u64,
    pub(crate) prior_event_hash: String,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) event_payload_hash: String,
    pub(crate) prior_active_world_hash: String,
    pub(crate) resulting_active_world_hash: String,
    pub(crate) prepared_at_simulation_tick: u64,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftGridTransferQuarantineReceiptV2 {
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
    quarantine_event_sequence: u64,
    quarantine_event_hash: String,
    quarantine_event_payload_hash: String,
    quarantine_mutation_witness_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftGridQuarantineProofV2 {
    pub(crate) transfer_id: String,
    pub(crate) root_aggregate_id: String,
    pub(crate) member_root: String,
    pub(crate) package_hash: String,
    pub(crate) destination_cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) fencing_token: u64,
    pub(crate) prior_event_sequence: u64,
    pub(crate) prior_event_hash: String,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) event_payload_hash: String,
    pub(crate) prior_active_world_hash: String,
    pub(crate) resulting_active_world_hash: String,
    pub(crate) quarantine_receipt_hash: String,
    pub(crate) quarantined_at_unix_ms: u64,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
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
    cleanup_event_payload_hash: String,
    base_world_hash: String,
    prior_draft_world_hash: String,
    resulting_draft_world_hash: String,
    cleanup_simulation_tick: u64,
    aborted_at_unix_ms: u64,
    removed_lock: Option<DraftAggregateTransferLockV2>,
    removed_reservation: Option<DraftAggregateTransferReservationV2>,
    mutation_witness_hash: String,
    cleanup_proof_hash: String,
    witness_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftGridTransferLedgerVectorV2 {
    pub(crate) ore: u64,
    pub(crate) refined_material: u64,
    pub(crate) components: u64,
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

/// Active destination authority after exact materialization and before the
/// directory proves activation. Subjects named by this record remain frozen
/// even though their canonical payloads are resident in the destination base.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftPendingGridImportV2 {
    schema_version: u32,
    reservation: DraftAggregateTransferReservationV2,
    source_export_proof: DraftGridExportProofV2,
    destination_assignment_generation: u64,
    historical_fencing_token: u64,
    live_fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: String,
    import_event_sequence: u64,
    import_event_hash: String,
    prior_draft_world_hash: String,
    ledger_vector: DraftGridTransferLedgerVectorV2,
    conservation_witness: TransferConservationWitness,
    production_eligibility_root: String,
    destination_production_lifecycle_generation: u64,
    imported_at_unix_ms: u64,
    mutation_witness_hash: String,
}

/// Historical import evidence. It intentionally does not participate in the
/// active-world hash, so later activation and ordinary gameplay may advance
/// without invalidating the exact import result that the directory retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridImportRecordV2 {
    schema_version: u32,
    pending: DraftPendingGridImportV2,
    resulting_active_world_hash: String,
    proof_hash: String,
    record_hash: String,
}

/// Historical proof that the destination released the imported closure for
/// ordinary gameplay. Imported production eligibility remains independently
/// held until its exact one-second boundary is processed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridActivationRecordV2 {
    schema_version: u32,
    pending: DraftPendingGridImportV2,
    destination_assignment_generation: u64,
    live_fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: String,
    activation_event_sequence: u64,
    activation_event_hash: String,
    prior_active_world_hash: String,
    resulting_active_world_hash: String,
    destination_import_proof_hash: String,
    activated_at_unix_ms: u64,
    mutation_witness_hash: String,
    proof_hash: String,
    record_hash: String,
}

/// Historical source proof that the directory-authenticated destination
/// import and activation were durable before the absent source export was
/// finalized. The export tombstone and conservation witness remain retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridFinalizationRecordV2 {
    schema_version: u32,
    binding: DraftGridTransferBindingV2,
    frozen: DraftFrozenClosureIdsV2,
    conservation_witness: TransferConservationWitness,
    source_assignment_generation: u64,
    live_fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: String,
    finalization_event_sequence: u64,
    finalization_event_hash: String,
    prior_active_world_hash: String,
    resulting_active_world_hash: String,
    source_export_proof_hash: String,
    source_exported_at_unix_ms: u64,
    destination_import_proof_hash: String,
    imported_at_unix_ms: u64,
    destination_activation_proof_hash: String,
    activated_at_unix_ms: u64,
    destination_import_proof: DraftGridImportProofV2,
    destination_activation_proof: DraftGridActivationProofV2,
    finalized_at_unix_ms: u64,
    mutation_witness_hash: String,
    proof_hash: String,
    record_hash: String,
}

/// Compact active source tombstone. It prevents a finalized export from ever
/// being unlocked or reintroduced even if historical proof records are moved
/// to colder retention later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftGridFinalizationTombstoneV2 {
    schema_version: u32,
    binding: DraftGridTransferBindingV2,
    frozen: DraftFrozenClosureIdsV2,
    conservation_witness: TransferConservationWitness,
    source_assignment_generation: u64,
    live_fencing_token: u64,
    finalization_event_sequence: u64,
    finalization_event_hash: String,
    source_export_proof_hash: String,
    destination_import_proof_hash: String,
    destination_activation_proof_hash: String,
    finalized_at_unix_ms: u64,
    mutation_witness_hash: String,
}

/// Historical proof that one exact destination production occurrence removed
/// every imported machine hold whose re-arm boundary was due and applied the
/// ordinary whole-cell production outcome exactly once. Its append-only history
/// link keeps earlier pauses and releases committed by the active world.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftImportedProductionReleaseRecordV2 {
    schema_version: u32,
    controls: DraftImportedProductionOccurrenceControlsV2,
    outcomes: Vec<DraftProductionMachineOccurrenceOutcomeV2>,
    released_eligibilities: BTreeMap<String, DraftImportedProductionEligibilityV2>,
    prior_production_job_origins: BTreeMap<String, DraftProductionJobOriginV2>,
    prior_production_queues: BTreeMap<String, std::collections::VecDeque<super::ProductionJob>>,
    prior_destination_inventory_contents: BTreeMap<String, InventoryContents>,
    prior_ledger: Ledger,
    prior_owners: BTreeMap<String, DraftProductionOwnerSnapshotV2>,
    prior_event_sequence: u64,
    prior_event_hash: String,
    release_event_sequence: u64,
    release_event_hash: String,
    prior_active_world_hash: String,
    resulting_active_world_hash: String,
    prior_production_quantum_sequence: u64,
    prior_production_scheduled_for_unix_ms: u64,
    prior_history_count: u64,
    prior_history_head: String,
    resulting_history_count: u64,
    resulting_history_head: String,
    history_entry_hash: String,
    live_fencing_token: u64,
    accepted_trusted_at_unix_ms: u64,
    mutation_witness_hash: String,
    proof_hash: String,
    record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftProductionMachineOccurrenceOutcomeV2 {
    control: DraftProductionMachineControlV2,
    ordinary_outcome: Option<ProductionMachineOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftProductionOwnerSnapshotV2 {
    experience: u64,
    career: CareerSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub(crate) exported_at_unix_ms: u64,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
    pub(crate) ledger_vector: DraftGridTransferLedgerVectorV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftGridImportProofV2 {
    pub(crate) transfer_id: String,
    pub(crate) root_aggregate_id: String,
    pub(crate) member_root: String,
    pub(crate) package_hash: String,
    pub(crate) destination_cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) fencing_token: u64,
    pub(crate) prior_event_sequence: u64,
    pub(crate) prior_event_hash: String,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) prior_draft_world_hash: String,
    pub(crate) resulting_active_world_hash: String,
    pub(crate) quarantine_receipt_hash: String,
    pub(crate) quarantined_at_unix_ms: u64,
    pub(crate) source_export_proof_hash: String,
    pub(crate) source_exported_at_unix_ms: u64,
    pub(crate) imported_at_unix_ms: u64,
    pub(crate) destination_production_lifecycle_generation: u64,
    pub(crate) production_eligibility_root: String,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
    pub(crate) ledger_vector: DraftGridTransferLedgerVectorV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftGridActivationProofV2 {
    pub(crate) transfer_id: String,
    pub(crate) root_aggregate_id: String,
    pub(crate) member_root: String,
    pub(crate) package_hash: String,
    pub(crate) destination_cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) fencing_token: u64,
    pub(crate) prior_event_sequence: u64,
    pub(crate) prior_event_hash: String,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) prior_active_world_hash: String,
    pub(crate) resulting_active_world_hash: String,
    pub(crate) quarantine_receipt_hash: String,
    pub(crate) destination_import_proof_hash: String,
    pub(crate) imported_at_unix_ms: u64,
    pub(crate) activated_at_unix_ms: u64,
    pub(crate) production_eligibility_root: String,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftGridFinalizationProofV2 {
    pub(crate) transfer_id: String,
    pub(crate) root_aggregate_id: String,
    pub(crate) member_root: String,
    pub(crate) package_hash: String,
    pub(crate) source_cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) fencing_token: u64,
    pub(crate) prior_event_sequence: u64,
    pub(crate) prior_event_hash: String,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) prior_active_world_hash: String,
    pub(crate) resulting_active_world_hash: String,
    pub(crate) source_export_proof_hash: String,
    pub(crate) source_exported_at_unix_ms: u64,
    pub(crate) destination_import_proof_hash: String,
    pub(crate) imported_at_unix_ms: u64,
    pub(crate) destination_activation_proof_hash: String,
    pub(crate) activated_at_unix_ms: u64,
    pub(crate) finalized_at_unix_ms: u64,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct DraftImportedProductionReleaseProofV2 {
    pub(crate) controls_root: String,
    pub(crate) occurrence: crate::event::ProductionScheduleOccurrence,
    pub(crate) released_eligibility_root: String,
    pub(crate) outcomes_root: String,
    pub(crate) prior_event_sequence: u64,
    pub(crate) prior_event_hash: String,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) prior_active_world_hash: String,
    pub(crate) resulting_active_world_hash: String,
    pub(crate) prior_production_quantum_sequence: u64,
    pub(crate) prior_production_scheduled_for_unix_ms: u64,
    pub(crate) prior_history_count: u64,
    pub(crate) prior_history_head: String,
    pub(crate) resulting_history_count: u64,
    pub(crate) resulting_history_head: String,
    pub(crate) history_entry_hash: String,
    pub(crate) live_fencing_token: u64,
    pub(crate) accepted_trusted_at_unix_ms: u64,
    pub(crate) mutation_witness_hash: String,
    pub(crate) proof_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub(crate) event_payload_hash: String,
    pub(crate) prior_event_sequence: u64,
    pub(crate) prior_event_hash: String,
    pub(crate) prior_draft_world_hash: String,
    pub(crate) resulting_draft_world_hash: String,
    pub(crate) trusted_time_unix_ms: u64,
    pub(crate) mutation_witness_hash: String,
    pub(crate) quarantine_receipt_hash: Option<String>,
    pub(crate) abort_witness_hash: String,
    pub(crate) removed_authority: bool,
    pub(crate) proof_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftGridTransferCellStateV2 {
    schema_version: u32,
    base: WorldState,
    production_job_origins: BTreeMap<String, DraftProductionJobOriginV2>,
    aggregate_locks: BTreeMap<String, DraftAggregateTransferLockV2>,
    aggregate_reservations: BTreeMap<String, DraftAggregateTransferReservationV2>,
    pending_imports: BTreeMap<String, DraftPendingGridImportV2>,
    imported_production_eligibilities: BTreeMap<String, DraftImportedProductionEligibilityV2>,
    committed_prepares: BTreeMap<String, DraftGridPrepareProofV2>,
    committed_quarantines: BTreeMap<String, DraftGridQuarantineProofV2>,
    committed_exports: BTreeMap<String, DraftGridExportRecordV2>,
    committed_imports: BTreeMap<String, DraftGridImportRecordV2>,
    committed_activations: BTreeMap<String, DraftGridActivationRecordV2>,
    source_finalization_tombstones: BTreeMap<String, DraftGridFinalizationTombstoneV2>,
    committed_finalizations: BTreeMap<String, DraftGridFinalizationRecordV2>,
    committed_production_releases: BTreeMap<String, DraftImportedProductionReleaseRecordV2>,
    production_occurrence_history_count: u64,
    production_occurrence_history_head: String,
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
    pending_imports: &'a BTreeMap<String, DraftPendingGridImportV2>,
    imported_production_eligibilities: &'a BTreeMap<String, DraftImportedProductionEligibilityV2>,
    source_finalization_tombstones: &'a BTreeMap<String, DraftGridFinalizationTombstoneV2>,
    production_occurrence_history_count: u64,
    production_occurrence_history_head: &'a str,
}

#[derive(Serialize)]
struct DraftGridPrepareEventHashMaterialV2<'a> {
    transfer_id: &'a str,
    root_aggregate_id: &'a str,
    member_root: &'a str,
    package_hash: &'a str,
    source_cell_id: &'a str,
    assignment_generation: u64,
    fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: &'a str,
    event_sequence: u64,
    event_payload_hash: &'a str,
    prepared_at_simulation_tick: u64,
    mutation_witness_hash: &'a str,
}

#[derive(Serialize)]
struct DraftGridQuarantineEventHashMaterialV2<'a> {
    transfer_id: &'a str,
    root_aggregate_id: &'a str,
    member_root: &'a str,
    package_hash: &'a str,
    destination_cell_id: &'a str,
    assignment_generation: u64,
    fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: &'a str,
    event_sequence: u64,
    quarantine_receipt_hash: &'a str,
    quarantined_at_unix_ms: u64,
    mutation_witness_hash: &'a str,
}

#[derive(Serialize)]
struct DraftGridImportEventHashMaterialV2<'a> {
    transfer_id: &'a str,
    root_aggregate_id: &'a str,
    member_root: &'a str,
    package_hash: &'a str,
    destination_cell_id: &'a str,
    assignment_generation: u64,
    fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: &'a str,
    event_sequence: u64,
    quarantine_receipt_hash: &'a str,
    quarantined_at_unix_ms: u64,
    source_export_proof_hash: &'a str,
    source_exported_at_unix_ms: u64,
    imported_at_unix_ms: u64,
    destination_production_lifecycle_generation: u64,
    mutation_witness_hash: &'a str,
}

#[derive(Serialize)]
struct DraftGridActivationEventHashMaterialV2<'a> {
    transfer_id: &'a str,
    root_aggregate_id: &'a str,
    member_root: &'a str,
    package_hash: &'a str,
    destination_cell_id: &'a str,
    assignment_generation: u64,
    fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: &'a str,
    event_sequence: u64,
    quarantine_receipt_hash: &'a str,
    destination_import_proof_hash: &'a str,
    imported_at_unix_ms: u64,
    activated_at_unix_ms: u64,
    production_eligibility_root: &'a str,
    mutation_witness_hash: &'a str,
}

#[derive(Serialize)]
struct DraftGridFinalizationEventHashMaterialV2<'a> {
    transfer_id: &'a str,
    root_aggregate_id: &'a str,
    member_root: &'a str,
    package_hash: &'a str,
    source_cell_id: &'a str,
    assignment_generation: u64,
    fencing_token: u64,
    prior_event_sequence: u64,
    prior_event_hash: &'a str,
    event_sequence: u64,
    source_export_proof_hash: &'a str,
    source_exported_at_unix_ms: u64,
    destination_import_proof_hash: &'a str,
    imported_at_unix_ms: u64,
    destination_activation_proof_hash: &'a str,
    activated_at_unix_ms: u64,
    finalized_at_unix_ms: u64,
    mutation_witness_hash: &'a str,
}

#[derive(Serialize)]
struct DraftImportedProductionReleaseEventHashMaterialV2<'a> {
    controls_root: &'a str,
    occurrence: &'a crate::event::ProductionScheduleOccurrence,
    released_eligibility_root: &'a str,
    outcomes_root: &'a str,
    prior_event_sequence: u64,
    prior_event_hash: &'a str,
    event_sequence: u64,
    prior_production_quantum_sequence: u64,
    prior_production_scheduled_for_unix_ms: u64,
    prior_history_count: u64,
    prior_history_head: &'a str,
    resulting_history_count: u64,
    live_fencing_token: u64,
    accepted_trusted_at_unix_ms: u64,
    mutation_witness_hash: &'a str,
}

#[derive(Serialize)]
struct DraftProductionOccurrenceHistoryEntryMaterialV2<'a> {
    prior_history_count: u64,
    prior_history_head: &'a str,
    resulting_history_count: u64,
    controls_root: &'a str,
    event_hash: &'a str,
    mutation_witness_hash: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftGridDirectoryAuthorityV2 {
    directory_revision: u64,
    directory_document_hash: String,
    binding: DraftGridTransferBindingV2,
    phase: TransferPhase,
    quarantine_receipt_hash: Option<String>,
    source_prepare_proof: Option<DraftGridPrepareProofV2>,
    destination_quarantine_proof: Option<DraftGridQuarantineProofV2>,
    source_export_proof_hash: Option<String>,
    source_exported_at_unix_ms: Option<u64>,
    source_export_proof: Option<DraftGridExportProofV2>,
    destination_import_proof: Option<DraftGridImportProofV2>,
    destination_activation_proof: Option<DraftGridActivationProofV2>,
    source_finalization_proof: Option<DraftGridFinalizationProofV2>,
    source_abort_proof: Option<DraftGridAbortCleanupProofV2>,
    destination_abort_proof: Option<DraftGridAbortCleanupProofV2>,
    proofs: BTreeSet<DraftGridDirectoryProofKindV2>,
    source_fencing_history: Vec<(u64, u64)>,
    destination_fencing_history: Vec<(u64, u64)>,
    live_source_assignment_generation: u64,
    live_source_fencing_token: u64,
    live_destination_assignment_generation: u64,
    live_destination_fencing_token: u64,
}

/// Sealed import boundary built only by the destination state transaction.
/// Production scheduling may read it but cannot construct scalar authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedDraftGridImportBoundaryV2 {
    destination_assignment_generation: u64,
    destination_fencing_token: u64,
    import_event_sequence: u64,
    import_event_hash: String,
    trusted_import_unix_ms: u64,
    destination_production_lifecycle_generation: u64,
}

impl ValidatedDraftGridImportBoundaryV2 {
    pub(super) fn destination_assignment_generation(&self) -> u64 {
        self.destination_assignment_generation
    }

    pub(super) fn destination_fencing_token(&self) -> u64 {
        self.destination_fencing_token
    }

    pub(super) fn import_event_sequence(&self) -> u64 {
        self.import_event_sequence
    }

    pub(super) fn import_event_hash(&self) -> &str {
        &self.import_event_hash
    }

    pub(super) fn trusted_import_unix_ms(&self) -> u64 {
        self.trusted_import_unix_ms
    }

    pub(super) fn destination_production_lifecycle_generation(&self) -> u64 {
        self.destination_production_lifecycle_generation
    }
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
    #[cfg(test)]
    fn test_fencing_history(live_generation: u64, live_fencing_token: u64) -> Vec<(u64, u64)> {
        let first_fence = live_fencing_token
            .checked_sub(live_generation.saturating_sub(1))
            .expect("test fencing token covers every assignment generation");
        (1..=live_generation)
            .map(|generation| (generation, first_fence + generation - 1))
            .collect()
    }

    pub(crate) fn from_validated_v3(authority: &ValidatedGridTransferAuthorityV3) -> Self {
        let source_prepare_proof = authority.source_prepare_cell_proof();
        let destination_quarantine_proof = authority.destination_quarantine_cell_proof();
        let source_export_proof = authority.source_export_cell_proof();
        let destination_import_proof = authority.destination_import_cell_proof();
        let destination_activation_proof = authority.destination_activation_cell_proof();
        let source_finalization_proof = authority.source_finalization_cell_proof();
        let source_abort_proof = authority.source_abort_cell_proof();
        let destination_abort_proof = authority.destination_abort_cell_proof();
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
            directory_revision: authority.directory_revision(),
            directory_document_hash: authority.directory_document_hash().to_owned(),
            binding: DraftGridTransferBindingV2::from_validated_authority(authority),
            phase: authority.phase(),
            quarantine_receipt_hash: authority.quarantine_receipt_hash().map(str::to_owned),
            source_prepare_proof,
            destination_quarantine_proof,
            source_export_proof_hash: authority
                .source_export_proof()
                .and_then(|proof| proof.export_proof_hash())
                .map(str::to_owned),
            source_exported_at_unix_ms: authority
                .source_export_proof()
                .and_then(DirectoryPhaseProofV3::trusted_time_unix_ms),
            source_export_proof,
            destination_import_proof,
            destination_activation_proof,
            source_finalization_proof,
            source_abort_proof,
            destination_abort_proof,
            proofs,
            source_fencing_history: authority
                .source_fencing_history()
                .iter()
                .map(|(&generation, &fence)| (generation, fence))
                .collect(),
            destination_fencing_history: authority
                .destination_fencing_history()
                .iter()
                .map(|(&generation, &fence)| (generation, fence))
                .collect(),
            live_source_assignment_generation: authority.live_source_assignment_generation(),
            live_source_fencing_token: authority.live_source_fencing_token(),
            live_destination_assignment_generation: authority
                .live_destination_assignment_generation(),
            live_destination_fencing_token: authority.live_destination_fencing_token(),
        }
    }

    #[cfg(test)]
    pub(super) fn for_package(package: &DraftGridClosurePackageV2, phase: TransferPhase) -> Self {
        Self {
            directory_revision: 1,
            directory_document_hash: blake3::hash(package.package_hash.as_bytes())
                .to_hex()
                .to_string(),
            binding: DraftGridTransferBindingV2::from_package(package),
            phase,
            quarantine_receipt_hash: None,
            source_prepare_proof: None,
            destination_quarantine_proof: None,
            source_export_proof_hash: None,
            source_exported_at_unix_ms: None,
            source_export_proof: None,
            destination_import_proof: None,
            destination_activation_proof: None,
            source_finalization_proof: None,
            source_abort_proof: None,
            destination_abort_proof: None,
            proofs: BTreeSet::new(),
            source_fencing_history: Self::test_fencing_history(
                package.source_assignment_generation,
                package.source_fencing_token,
            ),
            destination_fencing_history: Self::test_fencing_history(
                package.destination_assignment_generation,
                package.destination_fencing_token,
            ),
            live_source_assignment_generation: package.source_assignment_generation,
            live_source_fencing_token: package.source_fencing_token,
            live_destination_assignment_generation: package.destination_assignment_generation,
            live_destination_fencing_token: package.destination_fencing_token,
        }
    }

    #[cfg(test)]
    pub(super) fn advance_test_source_authority(&mut self) {
        self.live_source_assignment_generation += 1;
        self.live_source_fencing_token += 1;
        self.source_fencing_history.push((
            self.live_source_assignment_generation,
            self.live_source_fencing_token,
        ));
        self.advance_test_directory_revision();
    }

    #[cfg(test)]
    fn advance_test_destination_authority(&mut self) {
        self.live_destination_assignment_generation += 1;
        self.live_destination_fencing_token += 1;
        self.destination_fencing_history.push((
            self.live_destination_assignment_generation,
            self.live_destination_fencing_token,
        ));
        self.advance_test_directory_revision();
    }

    #[cfg(test)]
    fn advance_test_directory_revision(&mut self) {
        self.directory_revision += 1;
        self.directory_document_hash = blake3::hash(
            format!(
                "{}:{}:{}:{}:{}",
                self.directory_revision,
                self.live_source_assignment_generation,
                self.live_source_fencing_token,
                self.live_destination_assignment_generation,
                self.live_destination_fencing_token
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
    }

    #[cfg(test)]
    fn record_test_source_prepare(
        &mut self,
        state: &DraftGridTransferCellStateV2,
        transfer_id: &str,
    ) {
        self.source_prepare_proof = Some(
            state
                .committed_prepares
                .get(transfer_id)
                .expect("test source prepare proof exists")
                .clone(),
        );
        self.proofs
            .insert(DraftGridDirectoryProofKindV2::SourcePrepare);
    }

    #[cfg(test)]
    fn record_test_destination_quarantine(
        &mut self,
        state: &DraftGridTransferCellStateV2,
        transfer_id: &str,
    ) {
        let proof = state
            .committed_quarantines
            .get(transfer_id)
            .expect("test destination quarantine proof exists")
            .clone();
        self.quarantine_receipt_hash = Some(proof.quarantine_receipt_hash.clone());
        self.destination_quarantine_proof = Some(proof);
        self.proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationQuarantine);
    }

    #[cfg(test)]
    fn record_test_abort(&mut self, proof: &DraftGridAbortCleanupProofV2) {
        let (slot, kind) = match proof.side {
            DraftGridTransferAbortSideV2::Source => (
                &mut self.source_abort_proof,
                DraftGridDirectoryProofKindV2::SourceAbort,
            ),
            DraftGridTransferAbortSideV2::Destination => (
                &mut self.destination_abort_proof,
                DraftGridDirectoryProofKindV2::DestinationAbort,
            ),
        };
        *slot = Some(proof.clone());
        self.proofs.insert(kind);
    }

    fn has_proof(&self, kind: DraftGridDirectoryProofKindV2) -> bool {
        self.proofs.contains(&kind)
    }

    fn fencing_history_valid(
        history: &[(u64, u64)],
        live_generation: u64,
        live_fencing_token: u64,
    ) -> bool {
        if history.len() != usize::try_from(live_generation).unwrap_or(usize::MAX)
            || history.last().copied() != Some((live_generation, live_fencing_token))
        {
            return false;
        }
        let mut prior_fence = 0;
        for (index, &(generation, fence)) in history.iter().enumerate() {
            if generation != u64::try_from(index).unwrap_or(u64::MAX) + 1 || fence <= prior_fence {
                return false;
            }
            prior_fence = fence;
        }
        true
    }

    fn fencing_token_at(history: &[(u64, u64)], generation: u64) -> Option<u64> {
        let index = usize::try_from(generation.checked_sub(1)?).ok()?;
        history
            .get(index)
            .filter(|(stored_generation, _)| *stored_generation == generation)
            .map(|(_, fence)| *fence)
    }

    #[cfg(test)]
    pub(crate) fn has_valid_phase_matrix(&self) -> bool {
        self.validate_phase_matrix().is_ok()
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
            && (has_prepare == self.source_prepare_proof.is_some())
            && (has_quarantine == self.destination_quarantine_proof.is_some())
            && (!has_quarantine || has_prepare)
            && (has_export == self.source_export_proof_hash.is_some())
            && (has_export == self.source_exported_at_unix_ms.is_some())
            && (has_export == self.source_export_proof.is_some())
            && (has_import == self.destination_import_proof.is_some())
            && (has_activation == self.destination_activation_proof.is_some())
            && (has_finalization == self.source_finalization_proof.is_some())
            && (has_source_abort == self.source_abort_proof.is_some())
            && (has_destination_abort == self.destination_abort_proof.is_some())
            && self.source_export_proof.as_ref().is_none_or(|proof| {
                self.source_export_proof_hash.as_deref() == Some(proof.proof_hash.as_str())
                    && self.source_exported_at_unix_ms == Some(proof.exported_at_unix_ms)
            })
            && self.destination_import_proof.as_ref().is_none_or(|proof| {
                self.quarantine_receipt_hash.as_deref()
                    == Some(proof.quarantine_receipt_hash.as_str())
                    && self.source_export_proof_hash.as_deref()
                        == Some(proof.source_export_proof_hash.as_str())
                    && self.source_exported_at_unix_ms == Some(proof.source_exported_at_unix_ms)
            })
            && self
                .destination_activation_proof
                .as_ref()
                .is_none_or(|proof| {
                    self.quarantine_receipt_hash.as_deref()
                        == Some(proof.quarantine_receipt_hash.as_str())
                        && self
                            .destination_import_proof
                            .as_ref()
                            .is_some_and(|import| {
                                import.proof_hash == proof.destination_import_proof_hash
                            })
                })
            && self.source_finalization_proof.as_ref().is_none_or(|proof| {
                self.source_export_proof.as_ref().is_some_and(|export| {
                    export.proof_hash == proof.source_export_proof_hash
                        && export.exported_at_unix_ms == proof.source_exported_at_unix_ms
                }) && self
                    .destination_import_proof
                    .as_ref()
                    .is_some_and(|import| {
                        import.proof_hash == proof.destination_import_proof_hash
                            && import.imported_at_unix_ms == proof.imported_at_unix_ms
                    })
                    && self
                        .destination_activation_proof
                        .as_ref()
                        .is_some_and(|activation| {
                            activation.proof_hash == proof.destination_activation_proof_hash
                                && activation.activated_at_unix_ms == proof.activated_at_unix_ms
                        })
            })
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

    pub(super) fn validate_package(
        &self,
        package: &DraftGridClosurePackageV2,
    ) -> Result<(), DraftGridClosureError> {
        self.binding.validate()?;
        self.validate_phase_matrix()?;
        let expected_ledger_vector = DraftGridTransferLedgerVectorV2::from_package(package)?;
        if self.directory_revision == 0
            || !valid_blake3_hex(&self.directory_document_hash)
            || !Self::fencing_history_valid(
                &self.source_fencing_history,
                self.live_source_assignment_generation,
                self.live_source_fencing_token,
            )
            || !Self::fencing_history_valid(
                &self.destination_fencing_history,
                self.live_destination_assignment_generation,
                self.live_destination_fencing_token,
            )
            || Self::fencing_token_at(
                &self.source_fencing_history,
                package.source_assignment_generation,
            ) != Some(package.source_fencing_token)
            || Self::fencing_token_at(
                &self.destination_fencing_history,
                package.destination_assignment_generation,
            ) != Some(package.destination_fencing_token)
            || self.binding != DraftGridTransferBindingV2::from_package(package)
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
            || self.source_exported_at_unix_ms == Some(0)
            || self.source_prepare_proof.as_ref().is_some_and(|proof| {
                proof.validate().is_err()
                    || proof.transfer_id != package.transfer_id
                    || proof.root_aggregate_id != package.root_aggregate_id
                    || proof.member_root != package.member_root
                    || proof.package_hash != package.package_hash
                    || proof.source_cell_id != package.source_cell_id
                    || Self::fencing_token_at(
                        &self.source_fencing_history,
                        proof.assignment_generation,
                    ) != Some(proof.fencing_token)
                    || proof.prepared_at_simulation_tick != package.prepared_at_simulation_tick
            })
            || self
                .destination_quarantine_proof
                .as_ref()
                .is_some_and(|proof| {
                    proof.validate().is_err()
                        || proof.transfer_id != package.transfer_id
                        || proof.root_aggregate_id != package.root_aggregate_id
                        || proof.member_root != package.member_root
                        || proof.package_hash != package.package_hash
                        || proof.destination_cell_id != package.destination_cell_id
                        || Self::fencing_token_at(
                            &self.destination_fencing_history,
                            proof.assignment_generation,
                        ) != Some(proof.fencing_token)
                        || self.quarantine_receipt_hash.as_deref()
                            != Some(proof.quarantine_receipt_hash.as_str())
                })
            || self.source_export_proof.as_ref().is_some_and(|proof| {
                proof.validate().is_err()
                    || proof.transfer_id != package.transfer_id
                    || proof.root_aggregate_id != package.root_aggregate_id
                    || proof.member_root != package.member_root
                    || proof.package_hash != package.package_hash
                    || proof.source_cell_id != package.source_cell_id
                    || Self::fencing_token_at(
                        &self.source_fencing_history,
                        proof.assignment_generation,
                    ) != Some(proof.fencing_token)
                    || self.quarantine_receipt_hash.as_deref()
                        != Some(proof.quarantine_receipt_hash.as_str())
                    || proof.ledger_vector != expected_ledger_vector
            })
            || self.destination_import_proof.as_ref().is_some_and(|proof| {
                proof.validate().is_err()
                    || proof.transfer_id != package.transfer_id
                    || proof.root_aggregate_id != package.root_aggregate_id
                    || proof.member_root != package.member_root
                    || proof.package_hash != package.package_hash
                    || proof.destination_cell_id != package.destination_cell_id
                    || Self::fencing_token_at(
                        &self.destination_fencing_history,
                        proof.assignment_generation,
                    ) != Some(proof.fencing_token)
                    || self.quarantine_receipt_hash.as_deref()
                        != Some(proof.quarantine_receipt_hash.as_str())
                    || self.source_export_proof_hash.as_deref()
                        != Some(proof.source_export_proof_hash.as_str())
                    || self.source_exported_at_unix_ms != Some(proof.source_exported_at_unix_ms)
                    || proof.ledger_vector != expected_ledger_vector
            })
            || self
                .destination_activation_proof
                .as_ref()
                .is_some_and(|proof| {
                    proof.validate().is_err()
                        || proof.transfer_id != package.transfer_id
                        || proof.root_aggregate_id != package.root_aggregate_id
                        || proof.member_root != package.member_root
                        || proof.package_hash != package.package_hash
                        || proof.destination_cell_id != package.destination_cell_id
                        || Self::fencing_token_at(
                            &self.destination_fencing_history,
                            proof.assignment_generation,
                        ) != Some(proof.fencing_token)
                        || self.quarantine_receipt_hash.as_deref()
                            != Some(proof.quarantine_receipt_hash.as_str())
                        || self.destination_import_proof.as_ref().is_none_or(|import| {
                            import.proof_hash != proof.destination_import_proof_hash
                                || import.imported_at_unix_ms != proof.imported_at_unix_ms
                                || import.production_eligibility_root
                                    != proof.production_eligibility_root
                        })
                })
            || self
                .source_finalization_proof
                .as_ref()
                .is_some_and(|proof| {
                    proof.validate().is_err()
                        || proof.transfer_id != package.transfer_id
                        || proof.root_aggregate_id != package.root_aggregate_id
                        || proof.member_root != package.member_root
                        || proof.package_hash != package.package_hash
                        || proof.source_cell_id != package.source_cell_id
                        || Self::fencing_token_at(
                            &self.source_fencing_history,
                            proof.assignment_generation,
                        ) != Some(proof.fencing_token)
                })
            || self.source_abort_proof.as_ref().is_some_and(|proof| {
                proof.validate().is_err()
                    || proof.side != DraftGridTransferAbortSideV2::Source
                    || proof.transfer_id != package.transfer_id
                    || proof.member_root != package.member_root
                    || proof.package_hash != package.package_hash
                    || proof.cell_id != package.source_cell_id
                    || Self::fencing_token_at(
                        &self.source_fencing_history,
                        proof.assignment_generation,
                    ) != Some(proof.fencing_token)
                    || proof
                        .quarantine_receipt_hash
                        .as_ref()
                        .is_some_and(|receipt| {
                            self.quarantine_receipt_hash.as_ref() != Some(receipt)
                        })
            })
            || self.destination_abort_proof.as_ref().is_some_and(|proof| {
                proof.validate().is_err()
                    || proof.side != DraftGridTransferAbortSideV2::Destination
                    || proof.transfer_id != package.transfer_id
                    || proof.member_root != package.member_root
                    || proof.package_hash != package.package_hash
                    || proof.cell_id != package.destination_cell_id
                    || Self::fencing_token_at(
                        &self.destination_fencing_history,
                        proof.assignment_generation,
                    ) != Some(proof.fencing_token)
                    || proof.quarantine_receipt_hash != self.quarantine_receipt_hash
            })
        {
            return Err(DraftGridClosureError::Invalid(
                "directory authority does not bind the exact grid package".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn phase(&self) -> TransferPhase {
        self.phase
    }

    pub(super) fn source_fencing_token(&self) -> u64 {
        self.live_source_fencing_token
    }

    pub(super) fn destination_fencing_token(&self) -> u64 {
        self.live_destination_fencing_token
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
    fn from_package(package: &DraftGridClosurePackageV2, proof: &DraftGridPrepareProofV2) -> Self {
        Self {
            binding: DraftGridTransferBindingV2::from_package(package),
            frozen: DraftFrozenClosureIdsV2::from_package(package),
            source_event_sequence: package.source_event_sequence,
            source_event_hash: package.source_event_hash.clone(),
            source_base_world_hash: package.source_world_hash.clone(),
            prepared_at_simulation_tick: package.prepared_at_simulation_tick,
            production_job_origins: package.production_job_origins.clone(),
            prepare_event_sequence: proof.event_sequence,
            prepare_event_hash: proof.event_hash.clone(),
            prepare_event_payload_hash: proof.event_payload_hash.clone(),
            prepare_mutation_witness_hash: proof.mutation_witness_hash.clone(),
        }
    }

    fn matches_package(&self, package: &DraftGridClosurePackageV2) -> bool {
        self.binding == DraftGridTransferBindingV2::from_package(package)
            && self.frozen == DraftFrozenClosureIdsV2::from_package(package)
            && self.source_event_sequence == package.source_event_sequence
            && self.source_event_hash == package.source_event_hash
            && self.source_base_world_hash == package.source_world_hash
            && self.prepared_at_simulation_tick == package.prepared_at_simulation_tick
            && self.production_job_origins == package.production_job_origins
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.binding.validate()?;
        self.frozen.validate()?;
        if self.binding.root_aggregate_id != self.frozen.grid_id
            || !valid_blake3_hex(&self.source_base_world_hash)
            || (self.source_event_sequence == 0 && !self.source_event_hash.is_empty())
            || (self.source_event_sequence > 0 && !valid_blake3_hex(&self.source_event_hash))
            || self.source_event_sequence.checked_add(1) != Some(self.prepare_event_sequence)
            || !valid_blake3_hex(&self.prepare_event_hash)
            || !valid_blake3_hex(&self.prepare_event_payload_hash)
            || !valid_blake3_hex(&self.prepare_mutation_witness_hash)
        {
            return Err(DraftGridClosureError::Invalid(
                "aggregate lock frontier or root binding is invalid".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridPrepareProofV2 {
    fn new(
        state: &DraftGridTransferCellStateV2,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
        event: &ValidatedDraftGridEventContextV17,
    ) -> Result<Self, DraftGridClosureError> {
        let mut proof = Self {
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            member_root: package.member_root.clone(),
            package_hash: package.package_hash.clone(),
            source_cell_id: package.source_cell_id.clone(),
            assignment_generation: authority.live_source_assignment_generation,
            fencing_token: authority.live_source_fencing_token,
            prior_event_sequence: state.base.event_sequence,
            prior_event_hash: state.base.last_event_hash.clone(),
            event_sequence: event.event_sequence,
            event_hash: event.event_hash.clone(),
            event_payload_hash: event.event_payload_hash.clone(),
            prior_active_world_hash: state.calculate_active_world_hash()?,
            resulting_active_world_hash: String::new(),
            prepared_at_simulation_tick: package.prepared_at_simulation_tick,
            mutation_witness_hash: String::new(),
            proof_hash: String::new(),
        };
        proof.mutation_witness_hash = proof.calculate_mutation_hash()?;
        Ok(proof)
    }

    fn seal_result(
        &mut self,
        state: &DraftGridTransferCellStateV2,
    ) -> Result<(), DraftGridClosureError> {
        self.resulting_active_world_hash = state.calculate_active_world_hash()?;
        self.proof_hash = self.calculate_hash()?;
        self.validate()
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.event_hash.clear();
        material.resulting_active_world_hash.clear();
        material.mutation_witness_hash.clear();
        material.proof_hash.clear();
        hash_json(PREPARE_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(PREPARE_PROOF_HASH_DOMAIN, &material)
    }

    #[cfg(test)]
    pub(crate) fn seal_hashes_for_test(&mut self) -> Result<(), String> {
        self.mutation_witness_hash = self
            .calculate_mutation_hash()
            .map_err(|source| source.to_string())?;
        self.proof_hash = self.calculate_hash().map_err(|source| source.to_string())?;
        self.validate().map_err(|source| source.to_string())
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        if !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.source_cell_id)
            || self.assignment_generation == 0
            || self.fencing_token == 0
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.event_sequence)
            || !valid_blake3_hex(&self.event_hash)
            || !valid_blake3_hex(&self.event_payload_hash)
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "grid prepare proof is not canonical event-17 material".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_for_directory(&self) -> Result<(), String> {
        self.validate().map_err(|source| source.to_string())
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
        proof: &DraftGridQuarantineProofV2,
    ) -> Self {
        Self {
            binding: DraftGridTransferBindingV2::from_package(package),
            frozen: DraftFrozenClosureIdsV2::from_package(package),
            receipt_hash: receipt.receipt_hash.clone(),
            destination_event_sequence: receipt.destination_event_sequence,
            destination_base_world_hash: receipt.destination_base_world_hash.clone(),
            destination_draft_world_hash: receipt.destination_draft_world_hash.clone(),
            quarantined_at_unix_ms: receipt.quarantined_at_unix_ms,
            quarantine_event_sequence: proof.event_sequence,
            quarantine_event_hash: proof.event_hash.clone(),
            quarantine_event_payload_hash: proof.event_payload_hash.clone(),
            quarantine_mutation_witness_hash: proof.mutation_witness_hash.clone(),
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
            || self.destination_event_sequence.checked_add(1)
                != Some(self.quarantine_event_sequence)
            || !valid_blake3_hex(&self.quarantine_event_hash)
            || !valid_blake3_hex(&self.quarantine_event_payload_hash)
            || !valid_blake3_hex(&self.quarantine_mutation_witness_hash)
        {
            return Err(DraftGridClosureError::Invalid(
                "aggregate reservation authority or closure is invalid".into(),
            ));
        }
        self.receipt().validate()
    }
}

impl DraftGridQuarantineProofV2 {
    fn new(
        state: &DraftGridTransferCellStateV2,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
        event: &ValidatedDraftGridEventContextV17,
        receipt: &DraftGridTransferQuarantineReceiptV2,
    ) -> Result<Self, DraftGridClosureError> {
        let mut proof = Self {
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            member_root: package.member_root.clone(),
            package_hash: package.package_hash.clone(),
            destination_cell_id: package.destination_cell_id.clone(),
            assignment_generation: authority.live_destination_assignment_generation,
            fencing_token: authority.live_destination_fencing_token,
            prior_event_sequence: state.base.event_sequence,
            prior_event_hash: state.base.last_event_hash.clone(),
            event_sequence: event.event_sequence,
            event_hash: event.event_hash.clone(),
            event_payload_hash: event.event_payload_hash.clone(),
            prior_active_world_hash: state.calculate_active_world_hash()?,
            resulting_active_world_hash: String::new(),
            quarantine_receipt_hash: receipt.receipt_hash.clone(),
            quarantined_at_unix_ms: event.occurred_at_unix_ms,
            mutation_witness_hash: String::new(),
            proof_hash: String::new(),
        };
        proof.mutation_witness_hash = proof.calculate_mutation_hash()?;
        Ok(proof)
    }

    fn seal_result(
        &mut self,
        state: &DraftGridTransferCellStateV2,
    ) -> Result<(), DraftGridClosureError> {
        self.resulting_active_world_hash = state.calculate_active_world_hash()?;
        self.proof_hash = self.calculate_hash()?;
        self.validate()
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.event_hash.clear();
        material.resulting_active_world_hash.clear();
        material.mutation_witness_hash.clear();
        material.proof_hash.clear();
        hash_json(QUARANTINE_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(QUARANTINE_PROOF_HASH_DOMAIN, &material)
    }

    #[cfg(test)]
    pub(crate) fn seal_hashes_for_test(&mut self) -> Result<(), String> {
        self.mutation_witness_hash = self
            .calculate_mutation_hash()
            .map_err(|source| source.to_string())?;
        self.proof_hash = self.calculate_hash().map_err(|source| source.to_string())?;
        self.validate().map_err(|source| source.to_string())
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        if !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.destination_cell_id)
            || self.assignment_generation == 0
            || self.fencing_token == 0
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.event_sequence)
            || !valid_blake3_hex(&self.event_hash)
            || !valid_blake3_hex(&self.event_payload_hash)
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.quarantine_receipt_hash)
            || self.quarantined_at_unix_ms == 0
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "grid quarantine proof is not canonical event-17 material".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_for_directory(&self) -> Result<(), String> {
        self.validate().map_err(|source| source.to_string())
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
            exported_at_unix_ms: self.exported_at_unix_ms,
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

    #[cfg(test)]
    pub(crate) fn seal_hashes_for_test(&mut self) -> Result<(), String> {
        self.event_hash.clear();
        self.proof_hash.clear();
        self.event_hash = self
            .calculate_event_hash()
            .map_err(|source| source.to_string())?;
        self.proof_hash = self.calculate_hash().map_err(|source| source.to_string())?;
        self.validate()
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
            || self.exported_at_unix_ms == 0
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash().map_err(|source| source.to_string())?
        {
            return Err("grid source-export proof is not canonical fenced material".into());
        }
        Ok(())
    }
}

impl DraftPendingGridImportV2 {
    fn import_boundary(&self) -> ValidatedDraftGridImportBoundaryV2 {
        ValidatedDraftGridImportBoundaryV2 {
            destination_assignment_generation: self.destination_assignment_generation,
            destination_fencing_token: self.live_fencing_token,
            import_event_sequence: self.import_event_sequence,
            import_event_hash: self.import_event_hash.clone(),
            trusted_import_unix_ms: self.imported_at_unix_ms,
            destination_production_lifecycle_generation: self
                .destination_production_lifecycle_generation,
        }
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.import_event_hash.clear();
        material.production_eligibility_root.clear();
        material.mutation_witness_hash.clear();
        hash_json(IMPORT_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            IMPORT_EVENT_HASH_DOMAIN,
            &DraftGridImportEventHashMaterialV2 {
                transfer_id: &self.reservation.binding.transfer_id,
                root_aggregate_id: &self.reservation.binding.root_aggregate_id,
                member_root: &self.reservation.binding.member_root,
                package_hash: &self.reservation.binding.package_hash,
                destination_cell_id: &self.reservation.binding.destination_cell_id,
                assignment_generation: self.destination_assignment_generation,
                fencing_token: self.live_fencing_token,
                prior_event_sequence: self.prior_event_sequence,
                prior_event_hash: &self.prior_event_hash,
                event_sequence: self.import_event_sequence,
                quarantine_receipt_hash: &self.reservation.receipt_hash,
                quarantined_at_unix_ms: self.reservation.quarantined_at_unix_ms,
                source_export_proof_hash: &self.source_export_proof.proof_hash,
                source_exported_at_unix_ms: self.source_export_proof.exported_at_unix_ms,
                imported_at_unix_ms: self.imported_at_unix_ms,
                destination_production_lifecycle_generation: self
                    .destination_production_lifecycle_generation,
                mutation_witness_hash: &self.mutation_witness_hash,
            },
        )
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.reservation.validate()?;
        self.source_export_proof
            .validate()
            .map_err(DraftGridClosureError::Invalid)?;
        let expected_contents = self.ledger_vector.as_contents();
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || self.reservation.binding.root_aggregate_id != self.reservation.frozen.grid_id
            || self.source_export_proof.transfer_id != self.reservation.binding.transfer_id
            || self.source_export_proof.root_aggregate_id
                != self.reservation.binding.root_aggregate_id
            || self.source_export_proof.member_root != self.reservation.binding.member_root
            || self.source_export_proof.package_hash != self.reservation.binding.package_hash
            || self.source_export_proof.source_cell_id != self.reservation.binding.source_cell_id
            || self.source_export_proof.assignment_generation
                < self.reservation.binding.source_assignment_generation
            || self.source_export_proof.fencing_token
                < self.reservation.binding.source_fencing_token
            || self.source_export_proof.quarantine_receipt_hash != self.reservation.receipt_hash
            || self.source_export_proof.ledger_vector != self.ledger_vector
            || self.source_export_proof.exported_at_unix_ms
                < self.reservation.quarantined_at_unix_ms
            || self.destination_assignment_generation
                < self.reservation.binding.destination_assignment_generation
            || self.historical_fencing_token != self.reservation.binding.destination_fencing_token
            || self.live_fencing_token < self.historical_fencing_token
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.import_event_sequence)
            || !valid_blake3_hex(&self.import_event_hash)
            || !valid_blake3_hex(&self.prior_draft_world_hash)
            || self.conservation_witness.transfer_id != self.reservation.binding.transfer_id
            || self.conservation_witness.package_hash != self.reservation.binding.package_hash
            || self.conservation_witness.counterparty_cell_id
                != self.reservation.binding.source_cell_id
            || self.conservation_witness.direction != TransferWitnessDirection::Import
            || self.conservation_witness.contents != expected_contents
            || !valid_blake3_hex(&self.production_eligibility_root)
            || self.destination_production_lifecycle_generation == 0
            || self.imported_at_unix_ms < self.reservation.quarantined_at_unix_ms
            || self.imported_at_unix_ms < self.source_export_proof.exported_at_unix_ms
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || self.import_event_hash != self.calculate_event_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "pending grid import identity, frontier, conservation, or hash is invalid".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridImportRecordV2 {
    fn new(
        pending: DraftPendingGridImportV2,
        resulting_active_world_hash: String,
    ) -> Result<Self, DraftGridClosureError> {
        let mut record = Self {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            pending,
            resulting_active_world_hash,
            proof_hash: String::new(),
            record_hash: String::new(),
        };
        record.proof_hash = record.proof().calculate_hash()?;
        record.record_hash = record.calculate_hash()?;
        record.validate()?;
        Ok(record)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_json(IMPORT_RECORD_HASH_DOMAIN, &material)
    }

    fn proof(&self) -> DraftGridImportProofV2 {
        DraftGridImportProofV2 {
            transfer_id: self.pending.reservation.binding.transfer_id.clone(),
            root_aggregate_id: self.pending.reservation.binding.root_aggregate_id.clone(),
            member_root: self.pending.reservation.binding.member_root.clone(),
            package_hash: self.pending.reservation.binding.package_hash.clone(),
            destination_cell_id: self.pending.reservation.binding.destination_cell_id.clone(),
            assignment_generation: self.pending.destination_assignment_generation,
            fencing_token: self.pending.live_fencing_token,
            prior_event_sequence: self.pending.prior_event_sequence,
            prior_event_hash: self.pending.prior_event_hash.clone(),
            event_sequence: self.pending.import_event_sequence,
            event_hash: self.pending.import_event_hash.clone(),
            prior_draft_world_hash: self.pending.prior_draft_world_hash.clone(),
            resulting_active_world_hash: self.resulting_active_world_hash.clone(),
            quarantine_receipt_hash: self.pending.reservation.receipt_hash.clone(),
            quarantined_at_unix_ms: self.pending.reservation.quarantined_at_unix_ms,
            source_export_proof_hash: self.pending.source_export_proof.proof_hash.clone(),
            source_exported_at_unix_ms: self.pending.source_export_proof.exported_at_unix_ms,
            imported_at_unix_ms: self.pending.imported_at_unix_ms,
            destination_production_lifecycle_generation: self
                .pending
                .destination_production_lifecycle_generation,
            production_eligibility_root: self.pending.production_eligibility_root.clone(),
            mutation_witness_hash: self.pending.mutation_witness_hash.clone(),
            proof_hash: self.proof_hash.clone(),
            ledger_vector: self.pending.ledger_vector,
        }
    }

    fn validate(&self) -> Result<(), DraftGridClosureError> {
        self.pending.validate()?;
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || !valid_blake3_hex(&self.record_hash)
            || self.record_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "grid import record result, proof, or hash is invalid".into(),
            ));
        }
        self.proof()
            .validate()
            .map_err(DraftGridClosureError::Invalid)
    }

    fn validate_request(
        &self,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        if self.pending.reservation.binding != DraftGridTransferBindingV2::from_package(package)
            || self.pending.reservation.frozen != DraftFrozenClosureIdsV2::from_package(package)
            || authority.quarantine_receipt_hash.as_deref()
                != Some(self.pending.reservation.receipt_hash.as_str())
            || authority.source_export_proof.as_ref() != Some(&self.pending.source_export_proof)
        {
            return Err(DraftGridClosureError::Changed(
                "destination-import retry changed its package, receipt, or source proof".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridImportProofV2 {
    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            IMPORT_EVENT_HASH_DOMAIN,
            &DraftGridImportEventHashMaterialV2 {
                transfer_id: &self.transfer_id,
                root_aggregate_id: &self.root_aggregate_id,
                member_root: &self.member_root,
                package_hash: &self.package_hash,
                destination_cell_id: &self.destination_cell_id,
                assignment_generation: self.assignment_generation,
                fencing_token: self.fencing_token,
                prior_event_sequence: self.prior_event_sequence,
                prior_event_hash: &self.prior_event_hash,
                event_sequence: self.event_sequence,
                quarantine_receipt_hash: &self.quarantine_receipt_hash,
                quarantined_at_unix_ms: self.quarantined_at_unix_ms,
                source_export_proof_hash: &self.source_export_proof_hash,
                source_exported_at_unix_ms: self.source_exported_at_unix_ms,
                imported_at_unix_ms: self.imported_at_unix_ms,
                destination_production_lifecycle_generation: self
                    .destination_production_lifecycle_generation,
                mutation_witness_hash: &self.mutation_witness_hash,
            },
        )
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(IMPORT_PROOF_HASH_DOMAIN, &material)
    }

    #[cfg(test)]
    pub(crate) fn seal_hashes_for_test(&mut self) -> Result<(), String> {
        self.event_hash = self
            .calculate_event_hash()
            .map_err(|source| source.to_string())?;
        self.proof_hash = self.calculate_hash().map_err(|source| source.to_string())?;
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.destination_cell_id)
            || self.assignment_generation == 0
            || self.fencing_token == 0
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.event_sequence)
            || !valid_blake3_hex(&self.event_hash)
            || self.event_hash
                != self
                    .calculate_event_hash()
                    .map_err(|source| source.to_string())?
            || !valid_blake3_hex(&self.prior_draft_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.quarantine_receipt_hash)
            || self.quarantined_at_unix_ms == 0
            || !valid_blake3_hex(&self.source_export_proof_hash)
            || self.source_exported_at_unix_ms == 0
            || self.imported_at_unix_ms < self.quarantined_at_unix_ms
            || self.imported_at_unix_ms < self.source_exported_at_unix_ms
            || self.destination_production_lifecycle_generation == 0
            || !valid_blake3_hex(&self.production_eligibility_root)
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash().map_err(|source| source.to_string())?
        {
            return Err("grid destination-import proof is not canonical fenced material".into());
        }
        Ok(())
    }
}

impl DraftGridActivationRecordV2 {
    #[allow(clippy::too_many_arguments)]
    fn new(
        pending: DraftPendingGridImportV2,
        destination_assignment_generation: u64,
        live_fencing_token: u64,
        prior_event_sequence: u64,
        prior_event_hash: String,
        prior_active_world_hash: String,
        destination_import_proof_hash: String,
        activated_at_unix_ms: u64,
    ) -> Result<Self, DraftGridClosureError> {
        let activation_event_sequence = prior_event_sequence.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported(
                "destination activation event sequence exhausted".into(),
            )
        })?;
        let mut record = Self {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            pending,
            destination_assignment_generation,
            live_fencing_token,
            prior_event_sequence,
            prior_event_hash,
            activation_event_sequence,
            activation_event_hash: String::new(),
            prior_active_world_hash,
            resulting_active_world_hash: String::new(),
            destination_import_proof_hash,
            activated_at_unix_ms,
            mutation_witness_hash: String::new(),
            proof_hash: String::new(),
            record_hash: String::new(),
        };
        record.mutation_witness_hash = record.calculate_mutation_hash()?;
        record.activation_event_hash = record.proof().calculate_event_hash()?;
        Ok(record)
    }

    fn seal_resulting_active_world_hash(
        &mut self,
        state: &DraftGridTransferCellStateV2,
    ) -> Result<(), DraftGridClosureError> {
        self.resulting_active_world_hash = state.calculate_active_world_hash()?;
        self.proof_hash = self.proof().calculate_hash()?;
        self.record_hash = self.calculate_hash()?;
        Ok(())
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.activation_event_hash.clear();
        material.resulting_active_world_hash.clear();
        material.mutation_witness_hash.clear();
        material.proof_hash.clear();
        material.record_hash.clear();
        hash_json(ACTIVATION_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_json(ACTIVATION_RECORD_HASH_DOMAIN, &material)
    }

    fn proof(&self) -> DraftGridActivationProofV2 {
        DraftGridActivationProofV2 {
            transfer_id: self.pending.reservation.binding.transfer_id.clone(),
            root_aggregate_id: self.pending.reservation.binding.root_aggregate_id.clone(),
            member_root: self.pending.reservation.binding.member_root.clone(),
            package_hash: self.pending.reservation.binding.package_hash.clone(),
            destination_cell_id: self.pending.reservation.binding.destination_cell_id.clone(),
            assignment_generation: self.destination_assignment_generation,
            fencing_token: self.live_fencing_token,
            prior_event_sequence: self.prior_event_sequence,
            prior_event_hash: self.prior_event_hash.clone(),
            event_sequence: self.activation_event_sequence,
            event_hash: self.activation_event_hash.clone(),
            prior_active_world_hash: self.prior_active_world_hash.clone(),
            resulting_active_world_hash: self.resulting_active_world_hash.clone(),
            quarantine_receipt_hash: self.pending.reservation.receipt_hash.clone(),
            destination_import_proof_hash: self.destination_import_proof_hash.clone(),
            imported_at_unix_ms: self.pending.imported_at_unix_ms,
            activated_at_unix_ms: self.activated_at_unix_ms,
            production_eligibility_root: self.pending.production_eligibility_root.clone(),
            mutation_witness_hash: self.mutation_witness_hash.clone(),
            proof_hash: self.proof_hash.clone(),
        }
    }

    fn validate_with_import(
        &self,
        import: &DraftGridImportRecordV2,
    ) -> Result<(), DraftGridClosureError> {
        self.pending.validate()?;
        let proof = self.proof();
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || import.pending != self.pending
            || self.destination_import_proof_hash != import.proof_hash
            || self.destination_assignment_generation
                < self.pending.destination_assignment_generation
            || self.live_fencing_token < self.pending.live_fencing_token
            || self.prior_event_sequence < self.pending.import_event_sequence
            || (self.prior_event_sequence == self.pending.import_event_sequence
                && (self.prior_event_hash != self.pending.import_event_hash
                    || self.prior_active_world_hash != import.resulting_active_world_hash))
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.activation_event_sequence)
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || self.activated_at_unix_ms < self.pending.imported_at_unix_ms
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || !valid_blake3_hex(&self.activation_event_hash)
            || self.activation_event_hash != proof.calculate_event_hash()?
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != proof.calculate_hash()?
            || !valid_blake3_hex(&self.record_hash)
            || self.record_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "grid activation record identity, frontier, or hash is invalid".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridActivationProofV2 {
    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            ACTIVATION_EVENT_HASH_DOMAIN,
            &DraftGridActivationEventHashMaterialV2 {
                transfer_id: &self.transfer_id,
                root_aggregate_id: &self.root_aggregate_id,
                member_root: &self.member_root,
                package_hash: &self.package_hash,
                destination_cell_id: &self.destination_cell_id,
                assignment_generation: self.assignment_generation,
                fencing_token: self.fencing_token,
                prior_event_sequence: self.prior_event_sequence,
                prior_event_hash: &self.prior_event_hash,
                event_sequence: self.event_sequence,
                quarantine_receipt_hash: &self.quarantine_receipt_hash,
                destination_import_proof_hash: &self.destination_import_proof_hash,
                imported_at_unix_ms: self.imported_at_unix_ms,
                activated_at_unix_ms: self.activated_at_unix_ms,
                production_eligibility_root: &self.production_eligibility_root,
                mutation_witness_hash: &self.mutation_witness_hash,
            },
        )
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(ACTIVATION_PROOF_HASH_DOMAIN, &material)
    }

    #[cfg(test)]
    pub(crate) fn seal_hashes_for_test(&mut self) -> Result<(), String> {
        self.event_hash = self
            .calculate_event_hash()
            .map_err(|source| source.to_string())?;
        self.proof_hash = self.calculate_hash().map_err(|source| source.to_string())?;
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.destination_cell_id)
            || self.assignment_generation == 0
            || self.fencing_token == 0
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.event_sequence)
            || !valid_blake3_hex(&self.event_hash)
            || self.event_hash
                != self
                    .calculate_event_hash()
                    .map_err(|source| source.to_string())?
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.quarantine_receipt_hash)
            || !valid_blake3_hex(&self.destination_import_proof_hash)
            || self.imported_at_unix_ms == 0
            || self.activated_at_unix_ms < self.imported_at_unix_ms
            || !valid_blake3_hex(&self.production_eligibility_root)
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash().map_err(|source| source.to_string())?
        {
            return Err(
                "grid destination-activation proof is not canonical fenced material".into(),
            );
        }
        Ok(())
    }
}

impl DraftGridFinalizationRecordV2 {
    fn new(
        state: &DraftGridTransferCellStateV2,
        export: &DraftGridExportRecordV2,
        authority: &DraftGridDirectoryAuthorityV2,
        finalized_at_unix_ms: u64,
    ) -> Result<Self, DraftGridClosureError> {
        let export_proof = export.proof();
        let import_proof = authority.destination_import_proof.as_ref().ok_or_else(|| {
            DraftGridClosureError::Invalid(
                "source finalization lacks the directory-retained destination import proof".into(),
            )
        })?;
        let activation_proof =
            authority
                .destination_activation_proof
                .as_ref()
                .ok_or_else(|| {
                    DraftGridClosureError::Invalid(
                    "source finalization lacks the directory-retained destination activation proof"
                        .into(),
                )
                })?;
        let finalization_event_sequence =
            state.base.event_sequence.checked_add(1).ok_or_else(|| {
                DraftGridClosureError::Unsupported(
                    "source finalization event sequence exhausted".into(),
                )
            })?;
        let mut record = Self {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            binding: export.binding.clone(),
            frozen: export.frozen.clone(),
            conservation_witness: export.conservation_witness.clone(),
            source_assignment_generation: authority.live_source_assignment_generation,
            live_fencing_token: authority.live_source_fencing_token,
            prior_event_sequence: state.base.event_sequence,
            prior_event_hash: state.base.last_event_hash.clone(),
            finalization_event_sequence,
            finalization_event_hash: String::new(),
            prior_active_world_hash: state.calculate_active_world_hash()?,
            resulting_active_world_hash: String::new(),
            source_export_proof_hash: export_proof.proof_hash,
            source_exported_at_unix_ms: export_proof.exported_at_unix_ms,
            destination_import_proof_hash: import_proof.proof_hash.clone(),
            imported_at_unix_ms: import_proof.imported_at_unix_ms,
            destination_activation_proof_hash: activation_proof.proof_hash.clone(),
            activated_at_unix_ms: activation_proof.activated_at_unix_ms,
            destination_import_proof: import_proof.clone(),
            destination_activation_proof: activation_proof.clone(),
            finalized_at_unix_ms,
            mutation_witness_hash: String::new(),
            proof_hash: String::new(),
            record_hash: String::new(),
        };
        record.mutation_witness_hash = record.calculate_mutation_hash()?;
        record.finalization_event_hash = record.proof().calculate_event_hash()?;
        record.validate_with_export(export)?;
        Ok(record)
    }

    fn seal_resulting_active_world_hash(
        &mut self,
        state: &DraftGridTransferCellStateV2,
        export: &DraftGridExportRecordV2,
    ) -> Result<(), DraftGridClosureError> {
        self.resulting_active_world_hash = state.calculate_active_world_hash()?;
        self.proof_hash = self.proof().calculate_hash()?;
        self.record_hash = self.calculate_hash()?;
        self.validate_with_export(export)
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.finalization_event_hash.clear();
        material.resulting_active_world_hash.clear();
        material.mutation_witness_hash.clear();
        material.proof_hash.clear();
        material.record_hash.clear();
        hash_json(FINALIZATION_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_json(FINALIZATION_RECORD_HASH_DOMAIN, &material)
    }

    fn proof(&self) -> DraftGridFinalizationProofV2 {
        DraftGridFinalizationProofV2 {
            transfer_id: self.binding.transfer_id.clone(),
            root_aggregate_id: self.binding.root_aggregate_id.clone(),
            member_root: self.binding.member_root.clone(),
            package_hash: self.binding.package_hash.clone(),
            source_cell_id: self.binding.source_cell_id.clone(),
            assignment_generation: self.source_assignment_generation,
            fencing_token: self.live_fencing_token,
            prior_event_sequence: self.prior_event_sequence,
            prior_event_hash: self.prior_event_hash.clone(),
            event_sequence: self.finalization_event_sequence,
            event_hash: self.finalization_event_hash.clone(),
            prior_active_world_hash: self.prior_active_world_hash.clone(),
            resulting_active_world_hash: self.resulting_active_world_hash.clone(),
            source_export_proof_hash: self.source_export_proof_hash.clone(),
            source_exported_at_unix_ms: self.source_exported_at_unix_ms,
            destination_import_proof_hash: self.destination_import_proof_hash.clone(),
            imported_at_unix_ms: self.imported_at_unix_ms,
            destination_activation_proof_hash: self.destination_activation_proof_hash.clone(),
            activated_at_unix_ms: self.activated_at_unix_ms,
            finalized_at_unix_ms: self.finalized_at_unix_ms,
            mutation_witness_hash: self.mutation_witness_hash.clone(),
            proof_hash: self.proof_hash.clone(),
        }
    }

    fn tombstone(&self) -> DraftGridFinalizationTombstoneV2 {
        DraftGridFinalizationTombstoneV2 {
            schema_version: self.schema_version,
            binding: self.binding.clone(),
            frozen: self.frozen.clone(),
            conservation_witness: self.conservation_witness.clone(),
            source_assignment_generation: self.source_assignment_generation,
            live_fencing_token: self.live_fencing_token,
            finalization_event_sequence: self.finalization_event_sequence,
            finalization_event_hash: self.finalization_event_hash.clone(),
            source_export_proof_hash: self.source_export_proof_hash.clone(),
            destination_import_proof_hash: self.destination_import_proof_hash.clone(),
            destination_activation_proof_hash: self.destination_activation_proof_hash.clone(),
            finalized_at_unix_ms: self.finalized_at_unix_ms,
            mutation_witness_hash: self.mutation_witness_hash.clone(),
        }
    }

    fn validate_with_export(
        &self,
        export: &DraftGridExportRecordV2,
    ) -> Result<(), DraftGridClosureError> {
        let proof = self.proof();
        let export_proof = export.proof();
        let import = &self.destination_import_proof;
        let activation = &self.destination_activation_proof;
        let import_binding_matches = import.transfer_id == self.binding.transfer_id
            && import.root_aggregate_id == self.binding.root_aggregate_id
            && import.member_root == self.binding.member_root
            && import.package_hash == self.binding.package_hash
            && import.destination_cell_id == self.binding.destination_cell_id
            && import.assignment_generation >= self.binding.destination_assignment_generation
            && import.fencing_token >= self.binding.destination_fencing_token
            && import.quarantine_receipt_hash == export.quarantine_receipt_hash
            && import.source_export_proof_hash == self.source_export_proof_hash
            && import.source_exported_at_unix_ms == self.source_exported_at_unix_ms
            && import.imported_at_unix_ms == self.imported_at_unix_ms
            && import.ledger_vector == export.ledger_vector;
        let activation_binding_matches = activation.transfer_id == import.transfer_id
            && activation.root_aggregate_id == import.root_aggregate_id
            && activation.member_root == import.member_root
            && activation.package_hash == import.package_hash
            && activation.destination_cell_id == import.destination_cell_id
            && activation.assignment_generation >= import.assignment_generation
            && activation.fencing_token >= import.fencing_token
            && activation.prior_event_sequence >= import.event_sequence
            && (activation.prior_event_sequence != import.event_sequence
                || (activation.prior_event_hash == import.event_hash
                    && activation.prior_active_world_hash == import.resulting_active_world_hash))
            && activation.quarantine_receipt_hash == import.quarantine_receipt_hash
            && activation.destination_import_proof_hash == import.proof_hash
            && activation.imported_at_unix_ms == import.imported_at_unix_ms
            && activation.production_eligibility_root == import.production_eligibility_root
            && activation.activated_at_unix_ms == self.activated_at_unix_ms;
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || self.binding != export.binding
            || self.frozen != export.frozen
            || self.conservation_witness != export.conservation_witness
            || self.source_assignment_generation < export.source_assignment_generation
            || self.live_fencing_token < export.live_fencing_token
            || self.prior_event_sequence < export.export_event_sequence
            || (self.prior_event_sequence == export.export_event_sequence
                && (self.prior_event_hash != export.export_event_hash
                    || self.prior_active_world_hash != export.resulting_active_world_hash))
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.finalization_event_sequence)
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || (!self.resulting_active_world_hash.is_empty()
                && !valid_blake3_hex(&self.resulting_active_world_hash))
            || self.source_export_proof_hash != export_proof.proof_hash
            || self.source_exported_at_unix_ms != export_proof.exported_at_unix_ms
            || !valid_blake3_hex(&self.destination_import_proof_hash)
            || !valid_blake3_hex(&self.destination_activation_proof_hash)
            || import.validate().is_err()
            || activation.validate().is_err()
            || import.proof_hash != self.destination_import_proof_hash
            || activation.proof_hash != self.destination_activation_proof_hash
            || !import_binding_matches
            || !activation_binding_matches
            || self.imported_at_unix_ms < self.source_exported_at_unix_ms
            || self.activated_at_unix_ms < self.imported_at_unix_ms
            || self.finalized_at_unix_ms < self.activated_at_unix_ms
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || !valid_blake3_hex(&self.finalization_event_hash)
            || self.finalization_event_hash != proof.calculate_event_hash()?
            || (!self.proof_hash.is_empty() && self.proof_hash != proof.calculate_hash()?)
            || (!self.record_hash.is_empty() && self.record_hash != self.calculate_hash()?)
        {
            return Err(DraftGridClosureError::Invalid(
                "grid source-finalization record identity, frontier, or proof chain is invalid"
                    .into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridFinalizationProofV2 {
    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            FINALIZATION_EVENT_HASH_DOMAIN,
            &DraftGridFinalizationEventHashMaterialV2 {
                transfer_id: &self.transfer_id,
                root_aggregate_id: &self.root_aggregate_id,
                member_root: &self.member_root,
                package_hash: &self.package_hash,
                source_cell_id: &self.source_cell_id,
                assignment_generation: self.assignment_generation,
                fencing_token: self.fencing_token,
                prior_event_sequence: self.prior_event_sequence,
                prior_event_hash: &self.prior_event_hash,
                event_sequence: self.event_sequence,
                source_export_proof_hash: &self.source_export_proof_hash,
                source_exported_at_unix_ms: self.source_exported_at_unix_ms,
                destination_import_proof_hash: &self.destination_import_proof_hash,
                imported_at_unix_ms: self.imported_at_unix_ms,
                destination_activation_proof_hash: &self.destination_activation_proof_hash,
                activated_at_unix_ms: self.activated_at_unix_ms,
                finalized_at_unix_ms: self.finalized_at_unix_ms,
                mutation_witness_hash: &self.mutation_witness_hash,
            },
        )
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(FINALIZATION_PROOF_HASH_DOMAIN, &material)
    }

    #[cfg(test)]
    pub(crate) fn seal_hashes_for_test(&mut self) -> Result<(), String> {
        self.event_hash = self
            .calculate_event_hash()
            .map_err(|source| source.to_string())?;
        self.proof_hash = self.calculate_hash().map_err(|source| source.to_string())?;
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if !valid_stable_id(&self.transfer_id)
            || !valid_stable_id(&self.root_aggregate_id)
            || !valid_blake3_hex(&self.member_root)
            || !valid_blake3_hex(&self.package_hash)
            || !valid_blake3_hex(&self.source_cell_id)
            || self.assignment_generation == 0
            || self.fencing_token == 0
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.event_sequence)
            || !valid_blake3_hex(&self.event_hash)
            || self.event_hash
                != self
                    .calculate_event_hash()
                    .map_err(|source| source.to_string())?
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.source_export_proof_hash)
            || self.source_exported_at_unix_ms == 0
            || !valid_blake3_hex(&self.destination_import_proof_hash)
            || self.imported_at_unix_ms < self.source_exported_at_unix_ms
            || !valid_blake3_hex(&self.destination_activation_proof_hash)
            || self.activated_at_unix_ms < self.imported_at_unix_ms
            || self.finalized_at_unix_ms < self.activated_at_unix_ms
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash().map_err(|source| source.to_string())?
        {
            return Err("grid source-finalization proof is not canonical fenced material".into());
        }
        Ok(())
    }
}

impl DraftImportedProductionReleaseRecordV2 {
    fn new(
        state: &DraftGridTransferCellStateV2,
        controls: DraftImportedProductionOccurrenceControlsV2,
        outcomes: Vec<DraftProductionMachineOccurrenceOutcomeV2>,
        released_eligibilities: BTreeMap<String, DraftImportedProductionEligibilityV2>,
        accepted_trusted_at_unix_ms: u64,
    ) -> Result<Self, DraftGridClosureError> {
        controls.validate_for_world(&state.base, &state.imported_production_eligibilities)?;
        let release_event_sequence = state.base.event_sequence.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported(
                "production eligibility release event sequence exhausted".into(),
            )
        })?;
        let prior_destination_inventory_contents = outcomes
            .iter()
            .filter_map(|outcome| outcome.ordinary_outcome.as_ref())
            .map(|outcome| {
                state
                    .base
                    .inventories
                    .get(&outcome.destination_inventory_id)
                    .map(|inventory| {
                        (
                            outcome.destination_inventory_id.clone(),
                            inventory.contents.clone(),
                        )
                    })
                    .ok_or_else(|| {
                        DraftGridClosureError::Invalid(
                            "production release outcome lost its destination inventory".into(),
                        )
                    })
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;
        let prior_owners = outcomes
            .iter()
            .filter_map(|outcome| outcome.ordinary_outcome.as_ref())
            .map(|outcome| {
                let job = state
                    .base
                    .production_queues
                    .get(&outcome.machine_block_id)
                    .and_then(|queue| queue.front())
                    .ok_or_else(|| {
                        DraftGridClosureError::Invalid(
                            "production release outcome lost its queue head".into(),
                        )
                    })?;
                let owner = state.base.player.get(&job.owner_player_id).ok_or_else(|| {
                    DraftGridClosureError::Invalid(
                        "production release outcome lost its queue owner".into(),
                    )
                })?;
                Ok((
                    job.owner_player_id.clone(),
                    DraftProductionOwnerSnapshotV2 {
                        experience: owner.experience,
                        career: owner.career.clone(),
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, DraftGridClosureError>>()?;
        let resulting_history_count = state
            .production_occurrence_history_count
            .checked_add(1)
            .ok_or_else(|| {
                DraftGridClosureError::Unsupported(
                    "production occurrence history count exhausted".into(),
                )
            })?;
        let mut record = Self {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            controls,
            outcomes,
            released_eligibilities,
            prior_production_job_origins: state.production_job_origins.clone(),
            prior_production_queues: state.base.production_queues.clone(),
            prior_destination_inventory_contents,
            prior_ledger: state.base.ledger.clone(),
            prior_owners,
            prior_event_sequence: state.base.event_sequence,
            prior_event_hash: state.base.last_event_hash.clone(),
            release_event_sequence,
            release_event_hash: String::new(),
            prior_active_world_hash: state.calculate_active_world_hash()?,
            resulting_active_world_hash: String::new(),
            prior_production_quantum_sequence: state
                .base
                .production_clock
                .last_committed_quantum_sequence,
            prior_production_scheduled_for_unix_ms: state
                .base
                .production_clock
                .last_scheduled_for_unix_ms,
            prior_history_count: state.production_occurrence_history_count,
            prior_history_head: state.production_occurrence_history_head.clone(),
            resulting_history_count,
            resulting_history_head: String::new(),
            history_entry_hash: String::new(),
            live_fencing_token: state.base.fencing_token,
            accepted_trusted_at_unix_ms,
            mutation_witness_hash: String::new(),
            proof_hash: String::new(),
            record_hash: String::new(),
        };
        record.mutation_witness_hash = record.calculate_mutation_hash()?;
        record.release_event_hash = record.proof()?.calculate_event_hash()?;
        record.history_entry_hash = record.calculate_history_entry_hash()?;
        record
            .resulting_history_head
            .clone_from(&record.history_entry_hash);
        record.validate_static()?;
        Ok(record)
    }

    fn seal_resulting_active_world_hash(
        &mut self,
        state: &DraftGridTransferCellStateV2,
    ) -> Result<(), DraftGridClosureError> {
        self.resulting_active_world_hash = state.calculate_active_world_hash()?;
        self.proof_hash = self.proof()?.calculate_hash()?;
        self.record_hash = self.calculate_hash()?;
        self.validate_static()
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.release_event_hash.clear();
        material.resulting_active_world_hash.clear();
        material.resulting_history_head.clear();
        material.history_entry_hash.clear();
        material.mutation_witness_hash.clear();
        material.proof_hash.clear();
        material.record_hash.clear();
        hash_json(PRODUCTION_RELEASE_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_history_entry_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            PRODUCTION_OCCURRENCE_HISTORY_ENTRY_DOMAIN,
            &DraftProductionOccurrenceHistoryEntryMaterialV2 {
                prior_history_count: self.prior_history_count,
                prior_history_head: &self.prior_history_head,
                resulting_history_count: self.resulting_history_count,
                controls_root: self.controls.controls_root(),
                event_hash: &self.release_event_hash,
                mutation_witness_hash: &self.mutation_witness_hash,
            },
        )
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_json(PRODUCTION_RELEASE_RECORD_HASH_DOMAIN, &material)
    }

    fn proof(&self) -> Result<DraftImportedProductionReleaseProofV2, DraftGridClosureError> {
        Ok(DraftImportedProductionReleaseProofV2 {
            controls_root: self.controls.controls_root().to_owned(),
            occurrence: self.controls.occurrence().clone(),
            released_eligibility_root: imported_production_eligibility_map_root(
                &self.released_eligibilities,
            )?,
            outcomes_root: hash_json(PRODUCTION_RELEASE_OUTCOMES_ROOT_DOMAIN, &self.outcomes)?,
            prior_event_sequence: self.prior_event_sequence,
            prior_event_hash: self.prior_event_hash.clone(),
            event_sequence: self.release_event_sequence,
            event_hash: self.release_event_hash.clone(),
            prior_active_world_hash: self.prior_active_world_hash.clone(),
            resulting_active_world_hash: self.resulting_active_world_hash.clone(),
            prior_production_quantum_sequence: self.prior_production_quantum_sequence,
            prior_production_scheduled_for_unix_ms: self.prior_production_scheduled_for_unix_ms,
            prior_history_count: self.prior_history_count,
            prior_history_head: self.prior_history_head.clone(),
            resulting_history_count: self.resulting_history_count,
            resulting_history_head: self.resulting_history_head.clone(),
            history_entry_hash: self.history_entry_hash.clone(),
            live_fencing_token: self.live_fencing_token,
            accepted_trusted_at_unix_ms: self.accepted_trusted_at_unix_ms,
            mutation_witness_hash: self.mutation_witness_hash.clone(),
            proof_hash: self.proof_hash.clone(),
        })
    }

    fn validate_static(&self) -> Result<(), DraftGridClosureError> {
        self.controls.validate_canonical()?;
        let proof = self.proof()?;
        let occurrence = self.controls.occurrence();
        let expected_releases = self
            .controls
            .machines()
            .iter()
            .filter(|control| {
                control.kind() == DraftProductionMachineControlKindV2::ReleaseAndEvaluate
            })
            .map(|control| {
                (
                    control.machine_block_id(),
                    control.eligibility_hash().unwrap_or_default(),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let released_match_controls =
            self.released_eligibilities
                .iter()
                .all(|(machine_id, eligibility)| {
                    expected_releases.get(machine_id.as_str()).copied()
                        == Some(eligibility.eligibility_hash())
                        && eligibility.machine_block_id() == machine_id
                        && eligibility.eligible_at_unix_ms() <= occurrence.scheduled_for_unix_ms
                })
                && expected_releases.len() == self.released_eligibilities.len();
        let outcomes_match_controls = self.outcomes.len() == self.controls.machines().len()
            && self
                .outcomes
                .iter()
                .zip(self.controls.machines())
                .all(|(outcome, control)| {
                    &outcome.control == control
                        && match control.kind() {
                            DraftProductionMachineControlKindV2::TransferPaused => {
                                outcome.ordinary_outcome.is_none()
                            }
                            DraftProductionMachineControlKindV2::Evaluate
                            | DraftProductionMachineControlKindV2::ReleaseAndEvaluate => {
                                outcome.ordinary_outcome.as_ref().is_some_and(|ordinary| {
                                    ordinary.grid_id == control.grid_id()
                                        && ordinary.machine_block_id == control.machine_block_id()
                                })
                            }
                        }
                });
        let occurrence_is_next = self.prior_production_quantum_sequence.checked_add(1)
            == Some(occurrence.production_quantum_sequence);
        let occurrence_time_is_next = if self.prior_production_quantum_sequence == 0 {
            occurrence.scheduled_for_unix_ms > 0
        } else {
            self.prior_production_scheduled_for_unix_ms
                .checked_add(1_000)
                .is_some_and(|earliest| occurrence.scheduled_for_unix_ms >= earliest)
        };
        if self.schema_version != DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION
            || !released_match_controls
            || !outcomes_match_controls
            || !occurrence_is_next
            || !occurrence_time_is_next
            || (self.prior_history_count == 0 && !self.prior_history_head.is_empty())
            || (self.prior_history_count > 0 && !valid_blake3_hex(&self.prior_history_head))
            || self.prior_history_count.checked_add(1) != Some(self.resulting_history_count)
            || !valid_blake3_hex(&self.history_entry_hash)
            || self.history_entry_hash != self.calculate_history_entry_hash()?
            || self.resulting_history_head != self.history_entry_hash
            || self.live_fencing_token == 0
            || self.accepted_trusted_at_unix_ms < occurrence.scheduled_for_unix_ms
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.release_event_sequence)
            || !valid_blake3_hex(&self.release_event_hash)
            || self.release_event_hash != proof.calculate_event_hash()?
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || (!self.resulting_active_world_hash.is_empty()
                && !valid_blake3_hex(&self.resulting_active_world_hash))
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || (!self.proof_hash.is_empty() && self.proof_hash != proof.calculate_hash()?)
            || (!self.record_hash.is_empty() && self.record_hash != self.calculate_hash()?)
        {
            return Err(DraftGridClosureError::Invalid(
                "imported production release record identity, occurrence, or hash is invalid"
                    .into(),
            ));
        }
        for eligibility in self.released_eligibilities.values() {
            eligibility.validate_release_occurrence(occurrence)?;
        }
        Ok(())
    }
}

impl DraftImportedProductionReleaseProofV2 {
    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            PRODUCTION_RELEASE_EVENT_HASH_DOMAIN,
            &DraftImportedProductionReleaseEventHashMaterialV2 {
                controls_root: &self.controls_root,
                occurrence: &self.occurrence,
                released_eligibility_root: &self.released_eligibility_root,
                outcomes_root: &self.outcomes_root,
                prior_event_sequence: self.prior_event_sequence,
                prior_event_hash: &self.prior_event_hash,
                event_sequence: self.event_sequence,
                prior_production_quantum_sequence: self.prior_production_quantum_sequence,
                prior_production_scheduled_for_unix_ms: self.prior_production_scheduled_for_unix_ms,
                prior_history_count: self.prior_history_count,
                prior_history_head: &self.prior_history_head,
                resulting_history_count: self.resulting_history_count,
                live_fencing_token: self.live_fencing_token,
                accepted_trusted_at_unix_ms: self.accepted_trusted_at_unix_ms,
                mutation_witness_hash: &self.mutation_witness_hash,
            },
        )
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(PRODUCTION_RELEASE_PROOF_HASH_DOMAIN, &material)
    }

    fn calculate_history_entry_hash(&self) -> Result<String, DraftGridClosureError> {
        hash_json(
            PRODUCTION_OCCURRENCE_HISTORY_ENTRY_DOMAIN,
            &DraftProductionOccurrenceHistoryEntryMaterialV2 {
                prior_history_count: self.prior_history_count,
                prior_history_head: &self.prior_history_head,
                resulting_history_count: self.resulting_history_count,
                controls_root: &self.controls_root,
                event_hash: &self.event_hash,
                mutation_witness_hash: &self.mutation_witness_hash,
            },
        )
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        let occurrence = &self.occurrence;
        if !valid_blake3_hex(&self.controls_root)
            || occurrence.schema_version
                != crate::event::PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
            || occurrence.universe_id.trim().is_empty()
            || !valid_blake3_hex(&occurrence.cell_id)
            || occurrence.lifecycle_generation == 0
            || occurrence.production_quantum_sequence == 0
            || occurrence.scheduled_for_unix_ms == 0
            || !valid_blake3_hex(&occurrence.universe_manifest_hash)
            || !valid_blake3_hex(&occurrence.celestial_registry_hash)
            || !valid_blake3_hex(&self.released_eligibility_root)
            || !valid_blake3_hex(&self.outcomes_root)
            || (self.prior_history_count == 0 && !self.prior_history_head.is_empty())
            || (self.prior_history_count > 0 && !valid_blake3_hex(&self.prior_history_head))
            || self.prior_history_count.checked_add(1) != Some(self.resulting_history_count)
            || !valid_blake3_hex(&self.history_entry_hash)
            || self.history_entry_hash
                != self
                    .calculate_history_entry_hash()
                    .map_err(|source| source.to_string())?
            || self.resulting_history_head != self.history_entry_hash
            || self.live_fencing_token == 0
            || self.accepted_trusted_at_unix_ms < occurrence.scheduled_for_unix_ms
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.event_sequence)
            || !valid_blake3_hex(&self.event_hash)
            || self.event_hash
                != self
                    .calculate_event_hash()
                    .map_err(|source| source.to_string())?
            || !valid_blake3_hex(&self.prior_active_world_hash)
            || !valid_blake3_hex(&self.resulting_active_world_hash)
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash().map_err(|source| source.to_string())?
        {
            return Err(
                "imported production release proof is not canonical occurrence material".into(),
            );
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
        removed_lock: Option<DraftAggregateTransferLockV2>,
        removed_reservation: Option<DraftAggregateTransferReservationV2>,
        authority: &DraftGridDirectoryAuthorityV2,
        event: &ValidatedDraftGridEventContextV17,
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
        let quarantine_receipt_hash = removed_reservation
            .as_ref()
            .map(|reservation| reservation.receipt_hash.clone())
            .or_else(|| authority.quarantine_receipt_hash.clone());
        let mut witness = Self {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            binding: DraftGridTransferBindingV2::from_package(package),
            side,
            removed_authority: removed_lock.is_some() || removed_reservation.is_some(),
            quarantine_receipt_hash,
            cell_id,
            assignment_generation,
            historical_fencing_token,
            live_fencing_token,
            prior_event_sequence: prior_state.base.event_sequence,
            prior_event_hash: prior_state.base.last_event_hash.clone(),
            cleanup_event_sequence: event.event_sequence,
            cleanup_event_hash: event.event_hash.clone(),
            cleanup_event_payload_hash: event.event_payload_hash.clone(),
            base_world_hash: prior_state.base.state_hash(),
            prior_draft_world_hash: prior_state.state_hash.clone(),
            resulting_draft_world_hash: resulting_state.calculate_active_world_hash()?,
            cleanup_simulation_tick: resulting_state.base.simulation_tick,
            aborted_at_unix_ms: event.occurred_at_unix_ms,
            removed_lock,
            removed_reservation,
            mutation_witness_hash: String::new(),
            cleanup_proof_hash: String::new(),
            witness_hash: String::new(),
        };
        witness.mutation_witness_hash = witness.calculate_mutation_hash()?;
        witness.witness_hash = witness.calculate_hash()?;
        let mut cleanup = witness.cleanup_proof();
        cleanup.proof_hash = cleanup.calculate_hash()?;
        witness.cleanup_proof_hash = cleanup.proof_hash;
        witness.validate()?;
        Ok(witness)
    }

    fn calculate_mutation_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.cleanup_event_hash.clear();
        material.resulting_draft_world_hash.clear();
        material.mutation_witness_hash.clear();
        material.cleanup_proof_hash.clear();
        material.witness_hash.clear();
        hash_json(ABORT_WITNESS_HASH_DOMAIN, &material)
    }

    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.cleanup_proof_hash.clear();
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
            || !valid_blake3_hex(&self.cleanup_event_payload_hash)
            || !valid_blake3_hex(&self.base_world_hash)
            || !valid_blake3_hex(&self.prior_draft_world_hash)
            || !valid_blake3_hex(&self.resulting_draft_world_hash)
            || self.aborted_at_unix_ms == 0
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self.mutation_witness_hash != self.calculate_mutation_hash()?
            || !valid_blake3_hex(&self.cleanup_proof_hash)
            || self
                .quarantine_receipt_hash
                .as_ref()
                .is_some_and(|hash| !valid_blake3_hex(hash))
            || !valid_blake3_hex(&self.witness_hash)
            || self.witness_hash != self.calculate_hash()?
            || self.removed_authority
                != (self.removed_lock.is_some() || self.removed_reservation.is_some())
            || match self.side {
                DraftGridTransferAbortSideV2::Source => {
                    self.removed_reservation.is_some()
                        || self.removed_lock.as_ref().is_some_and(|lock| {
                            lock.binding != self.binding
                                || lock.binding.root_aggregate_id != self.binding.root_aggregate_id
                        })
                }
                DraftGridTransferAbortSideV2::Destination => {
                    self.removed_lock.is_some()
                        || self
                            .removed_reservation
                            .as_ref()
                            .is_some_and(|reservation| {
                                reservation.binding != self.binding
                                    || self.quarantine_receipt_hash.as_deref()
                                        != Some(reservation.receipt_hash.as_str())
                            })
                }
            }
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
            event_payload_hash: self.cleanup_event_payload_hash.clone(),
            prior_event_sequence: self.prior_event_sequence,
            prior_event_hash: self.prior_event_hash.clone(),
            prior_draft_world_hash: self.prior_draft_world_hash.clone(),
            resulting_draft_world_hash: self.resulting_draft_world_hash.clone(),
            trusted_time_unix_ms: self.aborted_at_unix_ms,
            mutation_witness_hash: self.mutation_witness_hash.clone(),
            quarantine_receipt_hash: self.quarantine_receipt_hash.clone(),
            abort_witness_hash: self.witness_hash.clone(),
            removed_authority: self.removed_authority,
            proof_hash: self.cleanup_proof_hash.clone(),
        }
    }

    fn validate_request(
        &self,
        package: &DraftGridClosurePackageV2,
        authority: &DraftGridDirectoryAuthorityV2,
        side: DraftGridTransferAbortSideV2,
    ) -> Result<(), DraftGridClosureError> {
        self.validate()?;
        let receipt_matches = match side {
            DraftGridTransferAbortSideV2::Source => self
                .quarantine_receipt_hash
                .as_ref()
                .is_none_or(|receipt| authority.quarantine_receipt_hash.as_ref() == Some(receipt)),
            DraftGridTransferAbortSideV2::Destination => {
                self.quarantine_receipt_hash == authority.quarantine_receipt_hash
            }
        };
        if self.binding != DraftGridTransferBindingV2::from_package(package)
            || self.side != side
            || !receipt_matches
        {
            return Err(DraftGridClosureError::Changed(
                "abort retry changed its package, side, or quarantine authority".into(),
            ));
        }
        Ok(())
    }
}

impl DraftGridAbortCleanupProofV2 {
    fn calculate_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.proof_hash.clear();
        hash_json(ABORT_PROOF_HASH_DOMAIN, &material)
    }

    fn calculate_event_hash(&self) -> Result<String, DraftGridClosureError> {
        let mut material = self.clone();
        material.event_hash.clear();
        hash_json(ABORT_EVENT_HASH_DOMAIN, &material)
    }

    #[cfg(test)]
    pub(crate) fn seal_event_hash(&mut self) -> Result<(), String> {
        self.event_hash.clear();
        self.proof_hash.clear();
        self.event_hash = self
            .calculate_event_hash()
            .map_err(|source| source.to_string())?;
        self.proof_hash = self.calculate_hash().map_err(|source| source.to_string())?;
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
            || !valid_blake3_hex(&self.event_payload_hash)
            || (self.prior_event_sequence == 0 && !self.prior_event_hash.is_empty())
            || (self.prior_event_sequence > 0 && !valid_blake3_hex(&self.prior_event_hash))
            || self.prior_event_sequence.checked_add(1) != Some(self.event_sequence)
            || !valid_blake3_hex(&self.prior_draft_world_hash)
            || !valid_blake3_hex(&self.resulting_draft_world_hash)
            || self.trusted_time_unix_ms == 0
            || !valid_blake3_hex(&self.mutation_witness_hash)
            || self
                .quarantine_receipt_hash
                .as_ref()
                .is_some_and(|hash| !valid_blake3_hex(hash))
            || !valid_blake3_hex(&self.abort_witness_hash)
            || !valid_blake3_hex(&self.proof_hash)
            || self.proof_hash != self.calculate_hash().map_err(|source| source.to_string())?
        {
            return Err("grid abort cleanup proof is not canonical fenced material".into());
        }
        Ok(())
    }
}

impl DraftGridTransferCellStateV2 {
    pub(super) fn base(&self) -> &WorldState {
        &self.base
    }

    #[cfg(test)]
    pub(super) fn advance_test_fence(&mut self) -> Result<(), DraftGridClosureError> {
        self.base.fencing_token = self.base.fencing_token.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported("test successor fence exhausted".into())
        })?;
        self.seal()
    }

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
            pending_imports: BTreeMap::new(),
            imported_production_eligibilities: BTreeMap::new(),
            committed_prepares: BTreeMap::new(),
            committed_quarantines: BTreeMap::new(),
            committed_exports: BTreeMap::new(),
            committed_imports: BTreeMap::new(),
            committed_activations: BTreeMap::new(),
            source_finalization_tombstones: BTreeMap::new(),
            committed_finalizations: BTreeMap::new(),
            committed_production_releases: BTreeMap::new(),
            production_occurrence_history_count: 0,
            production_occurrence_history_head: String::new(),
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
        // The fencing token is an operational lease, not gameplay state. A
        // successor worker must be able to verify the same active-world root
        // while presenting a newer directory fence.
        let mut canonical_base = self.base.clone();
        canonical_base.fencing_token = 0;
        hash_json(
            DRAFT_ACTIVE_WORLD_HASH_DOMAIN,
            &DraftActiveWorldHashMaterialV2 {
                schema_version: self.schema_version,
                base: &canonical_base,
                production_job_origins: &self.production_job_origins,
                aggregate_locks: &self.aggregate_locks,
                aggregate_reservations: &self.aggregate_reservations,
                pending_imports: &self.pending_imports,
                imported_production_eligibilities: &self.imported_production_eligibilities,
                source_finalization_tombstones: &self.source_finalization_tombstones,
                production_occurrence_history_count: self.production_occurrence_history_count,
                production_occurrence_history_head: &self.production_occurrence_history_head,
            },
        )
    }

    fn seal(&mut self) -> Result<(), DraftGridClosureError> {
        self.state_hash.clear();
        self.state_hash = self.calculate_hash()?;
        self.validate()
    }

    fn original_eligibilities_for_transfer(
        &self,
        transfer_id: &str,
    ) -> Result<BTreeMap<String, DraftImportedProductionEligibilityV2>, DraftGridClosureError> {
        let mut records = self
            .imported_production_eligibilities
            .iter()
            .filter(|(_, eligibility)| eligibility.transfer_id() == transfer_id)
            .map(|(machine_id, eligibility)| (machine_id.clone(), eligibility.clone()))
            .collect::<BTreeMap<_, _>>();
        for release in self.committed_production_releases.values() {
            for (machine_id, eligibility) in &release.released_eligibilities {
                if eligibility.transfer_id() == transfer_id
                    && records
                        .insert(machine_id.clone(), eligibility.clone())
                        .is_some()
                {
                    return Err(DraftGridClosureError::Invalid(
                        "import eligibility is both live and historically released".into(),
                    ));
                }
            }
        }
        Ok(records)
    }

    fn production_origin_is_active_or_released(&self, transfer_id: &str, job_id: &str) -> bool {
        self.production_job_origins.contains_key(job_id)
            || self.committed_production_releases.values().any(|release| {
                release.released_eligibilities.values().any(|eligibility| {
                    eligibility.transfer_id() == transfer_id && eligibility.contains_job_id(job_id)
                }) && release.prior_production_job_origins.contains_key(job_id)
            })
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
            || self.pending_imports.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.committed_prepares.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.committed_quarantines.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.committed_activations.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.source_finalization_tombstones.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.committed_finalizations.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.source_finalization_tombstones.len() != self.committed_finalizations.len()
            || self.committed_production_releases.len() > MAX_DRAFT_PRODUCTION_OCCURRENCES_PER_CELL
            || usize::try_from(self.production_occurrence_history_count).ok()
                != Some(self.committed_production_releases.len())
            || (self.production_occurrence_history_count == 0
                && !self.production_occurrence_history_head.is_empty())
            || (self.production_occurrence_history_count > 0
                && !valid_blake3_hex(&self.production_occurrence_history_head))
            || self.imported_production_eligibilities.len()
                > MAX_DRAFT_IMPORTED_PRODUCTION_ELIGIBILITIES_PER_CELL
            || self.committed_exports.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.committed_imports.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || self.abort_witnesses.len() > MAX_DRAFT_TRANSFERS_PER_CELL
            || draft_transfer_count(self) > MAX_DRAFT_TRANSFERS_PER_CELL
            || !valid_blake3_hex(&self.state_hash)
            || self.state_hash != self.calculate_hash()?
        {
            return Err(DraftGridClosureError::Invalid(
                "draft grid-transfer cell envelope or base conservation is invalid".into(),
            ));
        }

        let mut transfer_ids = BTreeSet::new();
        let mut frozen_sets = Vec::new();
        for (transfer_id, proof) in &self.committed_prepares {
            proof.validate()?;
            let at_prepare_frontier = self.base.event_sequence == proof.event_sequence;
            let prior_matches = if at_prepare_frontier {
                let mut prior = self.clone();
                prior.committed_prepares.remove(transfer_id);
                prior.aggregate_locks.remove(&proof.root_aggregate_id);
                prior.base.event_sequence = proof.prior_event_sequence;
                prior
                    .base
                    .last_event_hash
                    .clone_from(&proof.prior_event_hash);
                prior.base.fencing_token = proof.fencing_token;
                prior.calculate_active_world_hash()? == proof.prior_active_world_hash
            } else {
                true
            };
            if transfer_id != &proof.transfer_id
                || proof.source_cell_id != self.base.cell_id
                || self.base.fencing_token < proof.fencing_token
                || self.base.event_sequence < proof.event_sequence
                || (at_prepare_frontier && self.base.last_event_hash != proof.event_hash)
                || (at_prepare_frontier
                    && self.calculate_active_world_hash()? != proof.resulting_active_world_hash)
                || !prior_matches
            {
                return Err(DraftGridClosureError::Invalid(
                    "historical grid prepare proof does not bind one event frontier".into(),
                ));
            }
        }
        for (transfer_id, proof) in &self.committed_quarantines {
            proof.validate()?;
            let reservation = self.aggregate_reservations.get(transfer_id);
            let at_quarantine_frontier = self.base.event_sequence == proof.event_sequence;
            let prior_matches = if at_quarantine_frontier {
                let mut prior = self.clone();
                prior.committed_quarantines.remove(transfer_id);
                prior.aggregate_reservations.remove(transfer_id);
                prior.base.event_sequence = proof.prior_event_sequence;
                prior
                    .base
                    .last_event_hash
                    .clone_from(&proof.prior_event_hash);
                let receipt = reservation.map(DraftAggregateTransferReservationV2::receipt);
                prior.calculate_active_world_hash()? == proof.prior_active_world_hash
                    && receipt.as_ref().is_some_and(|receipt| {
                        receipt.receipt_hash == proof.quarantine_receipt_hash
                            && receipt.destination_base_world_hash == prior.base.state_hash()
                            && prior.calculate_hash().ok().as_ref()
                                == Some(&receipt.destination_draft_world_hash)
                    })
            } else {
                true
            };
            if transfer_id != &proof.transfer_id
                || proof.destination_cell_id != self.base.cell_id
                || self.base.fencing_token < proof.fencing_token
                || self.base.event_sequence < proof.event_sequence
                || (at_quarantine_frontier && self.base.last_event_hash != proof.event_hash)
                || (at_quarantine_frontier
                    && self.calculate_active_world_hash()? != proof.resulting_active_world_hash)
                || !prior_matches
            {
                return Err(DraftGridClosureError::Invalid(
                    "historical grid quarantine proof does not bind one event frontier".into(),
                ));
            }
        }
        for (root_id, lock) in &self.aggregate_locks {
            lock.validate()?;
            let prepare = self.committed_prepares.get(&lock.binding.transfer_id);
            if root_id != &lock.binding.root_aggregate_id
                || lock.binding.source_cell_id != self.base.cell_id
                || self.base.fencing_token < lock.binding.source_fencing_token
                || prepare.is_none_or(|proof| {
                    proof.root_aggregate_id != lock.binding.root_aggregate_id
                        || proof.member_root != lock.binding.member_root
                        || proof.package_hash != lock.binding.package_hash
                        || proof.event_sequence != lock.prepare_event_sequence
                        || proof.event_hash != lock.prepare_event_hash
                        || proof.event_payload_hash != lock.prepare_event_payload_hash
                        || proof.mutation_witness_hash != lock.prepare_mutation_witness_hash
                })
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
            let quarantine = self.committed_quarantines.get(transfer_id);
            if transfer_id != &reservation.binding.transfer_id
                || reservation.binding.destination_cell_id != self.base.cell_id
                || self.base.fencing_token < reservation.binding.destination_fencing_token
                || quarantine.is_none_or(|proof| {
                    proof.root_aggregate_id != reservation.binding.root_aggregate_id
                        || proof.member_root != reservation.binding.member_root
                        || proof.package_hash != reservation.binding.package_hash
                        || proof.quarantine_receipt_hash != reservation.receipt_hash
                        || proof.event_sequence != reservation.quarantine_event_sequence
                        || proof.event_hash != reservation.quarantine_event_hash
                        || proof.event_payload_hash != reservation.quarantine_event_payload_hash
                        || proof.mutation_witness_hash
                            != reservation.quarantine_mutation_witness_hash
                        || proof.quarantined_at_unix_ms != reservation.quarantined_at_unix_ms
                })
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
        for (transfer_id, pending) in &self.pending_imports {
            pending.validate()?;
            let eligibilities = self.original_eligibilities_for_transfer(transfer_id)?;
            let every_eligibility_is_live =
                eligibilities.iter().all(|(machine_id, eligibility)| {
                    self.imported_production_eligibilities.get(machine_id) == Some(eligibility)
                });
            if transfer_id != &pending.reservation.binding.transfer_id
                || pending.reservation.binding.destination_cell_id != self.base.cell_id
                || self.base.fencing_token < pending.live_fencing_token
                || self.base.production_clock.lifecycle_generation
                    != pending.destination_production_lifecycle_generation
                || self.base.event_sequence < pending.import_event_sequence
                || (self.base.event_sequence == pending.import_event_sequence
                    && self.base.last_event_hash != pending.import_event_hash)
                || !transfer_ids.insert(transfer_id)
                || !(if every_eligibility_is_live {
                    frozen_closure_is_present(&self.base, &pending.reservation.frozen)
                } else {
                    frozen_closure_subjects_are_present(&self.base, &pending.reservation.frozen)
                })
                || pending.reservation.frozen.job_ids.iter().any(|job_id| {
                    !self.production_origin_is_active_or_released(transfer_id, job_id)
                })
                || self.base.transfer_witnesses.get(transfer_id)
                    != Some(&pending.conservation_witness)
                || self
                    .aggregate_locks
                    .contains_key(&pending.reservation.binding.root_aggregate_id)
                || self.aggregate_reservations.contains_key(transfer_id)
                || self.committed_exports.contains_key(transfer_id)
                || self.committed_activations.contains_key(transfer_id)
                || self.abort_witnesses.contains_key(transfer_id)
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
                || self
                    .committed_imports
                    .get(transfer_id)
                    .is_none_or(|record| record.pending != *pending)
                || eligibilities
                    .keys()
                    .ne(pending.reservation.frozen.machine_block_ids.iter())
                || imported_production_eligibility_map_root(&eligibilities)?
                    != pending.production_eligibility_root
            {
                return Err(DraftGridClosureError::Invalid(
                    "pending import does not freeze one exact resident destination closure".into(),
                ));
            }
            let import_boundary = pending.import_boundary();
            for (machine_id, eligibility) in &eligibilities {
                if self.imported_production_eligibilities.get(machine_id) != Some(eligibility) {
                    eligibility.validate_persisted_import_boundary(
                        transfer_id,
                        &pending.reservation.binding.package_hash,
                        &import_boundary,
                    )?;
                    continue;
                }
                let queue = self.base.production_queues.get(machine_id).ok_or_else(|| {
                    DraftGridClosureError::Invalid(
                        "pending import eligibility lost its resident machine queue".into(),
                    )
                })?;
                eligibility.validate_persisted_in_world(&self.base, queue)?;
                eligibility.validate_persisted_import_boundary(
                    transfer_id,
                    &pending.reservation.binding.package_hash,
                    &import_boundary,
                )?;
                if eligibility.package_hash() != pending.reservation.binding.package_hash {
                    return Err(DraftGridClosureError::Invalid(
                        "pending import eligibility changed its package binding".into(),
                    ));
                }
            }
            frozen_sets.push(&pending.reservation.frozen);
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
        for (transfer_id, finalization) in &self.committed_finalizations {
            let export = self.committed_exports.get(transfer_id).ok_or_else(|| {
                DraftGridClosureError::Invalid(
                    "source finalization has no historical committed export".into(),
                )
            })?;
            finalization.validate_with_export(export)?;
            let proof = finalization.proof();
            proof.validate().map_err(DraftGridClosureError::Invalid)?;
            if self.source_finalization_tombstones.get(transfer_id)
                != Some(&finalization.tombstone())
            {
                return Err(DraftGridClosureError::Invalid(
                    "source finalization active tombstone and historical proof disagree".into(),
                ));
            }
            let at_finalization_frontier =
                self.base.event_sequence == finalization.finalization_event_sequence;
            let prior_active_world_matches = if at_finalization_frontier {
                let mut prior = self.clone();
                prior.source_finalization_tombstones.remove(transfer_id);
                prior.committed_finalizations.remove(transfer_id);
                prior.base.event_sequence = finalization.prior_event_sequence;
                prior
                    .base
                    .last_event_hash
                    .clone_from(&finalization.prior_event_hash);
                prior.calculate_active_world_hash()? == finalization.prior_active_world_hash
            } else {
                true
            };
            if transfer_id != &finalization.binding.transfer_id
                || finalization.binding.source_cell_id != self.base.cell_id
                || self.base.fencing_token < finalization.live_fencing_token
                || self.base.event_sequence < finalization.finalization_event_sequence
                || (at_finalization_frontier
                    && self.base.last_event_hash != finalization.finalization_event_hash)
                || (at_finalization_frontier
                    && self.calculate_active_world_hash()?
                        != finalization.resulting_active_world_hash)
                || !prior_active_world_matches
                || self.base.transfer_witnesses.get(transfer_id)
                    != Some(&finalization.conservation_witness)
                || self
                    .aggregate_locks
                    .contains_key(&finalization.binding.root_aggregate_id)
                || self.aggregate_reservations.contains_key(transfer_id)
                || self.pending_imports.contains_key(transfer_id)
                || self.committed_imports.contains_key(transfer_id)
                || self.committed_activations.contains_key(transfer_id)
                || self.abort_witnesses.contains_key(transfer_id)
                || !frozen_closure_is_absent(&self.base, &finalization.frozen)
                || finalization
                    .frozen
                    .job_ids
                    .iter()
                    .any(|job_id| self.production_job_origins.contains_key(job_id))
            {
                return Err(DraftGridClosureError::Invalid(
                    "source finalization does not retain one exact absent export tombstone".into(),
                ));
            }
        }
        for (transfer_id, import) in &self.committed_imports {
            import.validate()?;
            let pending_matches = self
                .pending_imports
                .get(transfer_id)
                .is_some_and(|pending| pending == &import.pending);
            let activation_matches = self
                .committed_activations
                .get(transfer_id)
                .is_some_and(|activation| activation.pending == import.pending);
            if transfer_id != &import.pending.reservation.binding.transfer_id
                || import.pending.reservation.binding.destination_cell_id != self.base.cell_id
                || self.base.fencing_token < import.pending.live_fencing_token
                || self.base.event_sequence < import.pending.import_event_sequence
                || (self.base.event_sequence == import.pending.import_event_sequence
                    && self.base.last_event_hash != import.pending.import_event_hash)
                || (self.base.event_sequence == import.pending.import_event_sequence
                    && self.calculate_active_world_hash()? != import.resulting_active_world_hash)
                || self.base.transfer_witnesses.get(transfer_id)
                    != Some(&import.pending.conservation_witness)
                || pending_matches == activation_matches
                || self
                    .aggregate_locks
                    .values()
                    .any(|lock| lock.binding.transfer_id == *transfer_id)
                || self.aggregate_reservations.contains_key(transfer_id)
                || self.committed_exports.contains_key(transfer_id)
                || self.abort_witnesses.contains_key(transfer_id)
            {
                return Err(DraftGridClosureError::Invalid(
                    "committed import does not bind one exact historical destination result".into(),
                ));
            }
        }
        for (transfer_id, activation) in &self.committed_activations {
            let import = self.committed_imports.get(transfer_id).ok_or_else(|| {
                DraftGridClosureError::Invalid(
                    "committed activation has no historical destination import".into(),
                )
            })?;
            activation.validate_with_import(import)?;
            let import_boundary = activation.pending.import_boundary();
            let at_activation_frontier =
                self.base.event_sequence == activation.activation_event_sequence;
            let prior_active_world_matches = if at_activation_frontier {
                let mut prior = self.clone();
                prior.committed_activations.remove(transfer_id);
                prior
                    .pending_imports
                    .insert(transfer_id.clone(), activation.pending.clone());
                prior.base.event_sequence = activation.prior_event_sequence;
                prior
                    .base
                    .last_event_hash
                    .clone_from(&activation.prior_event_hash);
                prior.calculate_active_world_hash()? == activation.prior_active_world_hash
            } else {
                true
            };
            let eligibilities = self.original_eligibilities_for_transfer(transfer_id)?;
            let every_eligibility_is_live =
                eligibilities.iter().all(|(machine_id, eligibility)| {
                    self.imported_production_eligibilities.get(machine_id) == Some(eligibility)
                });
            if transfer_id != &activation.pending.reservation.binding.transfer_id
                || activation.pending.reservation.binding.destination_cell_id != self.base.cell_id
                || self.base.fencing_token < activation.live_fencing_token
                || self.base.event_sequence < activation.activation_event_sequence
                || (at_activation_frontier
                    && self.base.last_event_hash != activation.activation_event_hash)
                || (at_activation_frontier
                    && self.calculate_active_world_hash()?
                        != activation.resulting_active_world_hash)
                || !prior_active_world_matches
                || self.pending_imports.contains_key(transfer_id)
                || (at_activation_frontier
                    && !(if every_eligibility_is_live {
                        frozen_closure_is_present(
                            &self.base,
                            &activation.pending.reservation.frozen,
                        )
                    } else {
                        frozen_closure_subjects_are_present(
                            &self.base,
                            &activation.pending.reservation.frozen,
                        )
                    }))
                || self.base.transfer_witnesses.get(transfer_id)
                    != Some(&activation.pending.conservation_witness)
                || self
                    .aggregate_locks
                    .values()
                    .any(|lock| lock.binding.transfer_id == *transfer_id)
                || self.aggregate_reservations.contains_key(transfer_id)
                || self.committed_exports.contains_key(transfer_id)
                || self.abort_witnesses.contains_key(transfer_id)
                || eligibilities.keys().ne(activation
                    .pending
                    .reservation
                    .frozen
                    .machine_block_ids
                    .iter())
                || imported_production_eligibility_map_root(&eligibilities)?
                    != activation.pending.production_eligibility_root
            {
                return Err(DraftGridClosureError::Invalid(
                    "committed activation does not release one exact imported closure".into(),
                ));
            }
            for (machine_id, eligibility) in &eligibilities {
                if self.imported_production_eligibilities.get(machine_id) != Some(eligibility) {
                    eligibility.validate_persisted_import_boundary(
                        transfer_id,
                        &activation.pending.reservation.binding.package_hash,
                        &import_boundary,
                    )?;
                    continue;
                }
                let queue = self.base.production_queues.get(machine_id).ok_or_else(|| {
                    DraftGridClosureError::Invalid(
                        "activated import eligibility lost its resident machine queue".into(),
                    )
                })?;
                eligibility.validate_persisted_in_world(&self.base, queue)?;
                eligibility.validate_persisted_import_boundary(
                    transfer_id,
                    &activation.pending.reservation.binding.package_hash,
                    &import_boundary,
                )?;
            }
        }
        let mut production_occurrence_history = self
            .committed_production_releases
            .values()
            .collect::<Vec<_>>();
        production_occurrence_history.sort_by_key(|release| release.resulting_history_count);
        let mut expected_history_count = 0_u64;
        let mut expected_history_head = String::new();
        let mut production_occurrence_keys = BTreeSet::new();
        for release in production_occurrence_history {
            let occurrence = release.controls.occurrence();
            if release.prior_history_count != expected_history_count
                || release.prior_history_head != expected_history_head
                || release.resulting_history_count != expected_history_count.saturating_add(1)
                || release.resulting_history_head != release.history_entry_hash
                || !production_occurrence_keys.insert((
                    occurrence.lifecycle_generation,
                    occurrence.production_quantum_sequence,
                ))
            {
                return Err(DraftGridClosureError::Invalid(
                    "production occurrence history is not one canonical append-only chain".into(),
                ));
            }
            expected_history_count = release.resulting_history_count;
            expected_history_head.clone_from(&release.resulting_history_head);
        }
        if expected_history_count != self.production_occurrence_history_count
            || expected_history_head != self.production_occurrence_history_head
        {
            return Err(DraftGridClosureError::Invalid(
                "production occurrence history head does not match its retained chain".into(),
            ));
        }

        let mut released_eligibility_hashes = BTreeSet::new();
        for (controls_root, release) in &self.committed_production_releases {
            release.validate_static()?;
            let proof = release.proof()?;
            proof.validate().map_err(DraftGridClosureError::Invalid)?;
            if controls_root != release.controls.controls_root()
                || release.controls.occurrence().universe_id != self.base.universe_id
                || release.controls.occurrence().cell_id != self.base.cell_id
                || release.controls.occurrence().lifecycle_generation
                    != self.base.production_clock.lifecycle_generation
                || release.controls.occurrence().universe_manifest_hash
                    != self.base.universe_manifest_hash
                || release.controls.occurrence().celestial_registry_hash
                    != self.base.celestial_registry_hash
                || self.base.event_sequence < release.release_event_sequence
                || self.base.fencing_token < release.live_fencing_token
                || release.released_eligibilities.values().any(|eligibility| {
                    !released_eligibility_hashes.insert(eligibility.eligibility_hash().to_owned())
                        || self
                            .imported_production_eligibilities
                            .get(eligibility.machine_block_id())
                            == Some(eligibility)
                })
            {
                return Err(DraftGridClosureError::Invalid(
                    "historical imported production occurrence is conflicting or misplaced".into(),
                ));
            }
            for eligibility in release.released_eligibilities.values() {
                let import = self
                    .committed_imports
                    .get(eligibility.transfer_id())
                    .ok_or_else(|| {
                        DraftGridClosureError::Invalid(
                            "released eligibility has no committed destination import".into(),
                        )
                    })?;
                eligibility.validate_persisted_import_boundary(
                    eligibility.transfer_id(),
                    &import.pending.reservation.binding.package_hash,
                    &import.pending.import_boundary(),
                )?;
            }
            if self.base.event_sequence == release.release_event_sequence {
                if self.base.last_event_hash != release.release_event_hash
                    || self.calculate_active_world_hash()? != release.resulting_active_world_hash
                {
                    return Err(DraftGridClosureError::Invalid(
                        "production release frontier changed its event or resulting world".into(),
                    ));
                }
                let mut prior = self.clone();
                prior.committed_production_releases.remove(controls_root);
                prior.base.event_sequence = release.prior_event_sequence;
                prior
                    .base
                    .last_event_hash
                    .clone_from(&release.prior_event_hash);
                prior.base.production_clock.last_committed_quantum_sequence =
                    release.prior_production_quantum_sequence;
                prior.base.production_clock.last_scheduled_for_unix_ms =
                    release.prior_production_scheduled_for_unix_ms;
                prior.production_occurrence_history_count = release.prior_history_count;
                prior
                    .production_occurrence_history_head
                    .clone_from(&release.prior_history_head);
                prior.production_job_origins = release.prior_production_job_origins.clone();
                prior.base.production_queues = release.prior_production_queues.clone();
                prior.base.ledger = release.prior_ledger.clone();
                for (inventory_id, contents) in &release.prior_destination_inventory_contents {
                    prior
                        .base
                        .inventories
                        .get_mut(inventory_id)
                        .ok_or_else(|| {
                            DraftGridClosureError::Invalid(
                                "release predecessor lost a destination inventory".into(),
                            )
                        })?
                        .contents
                        .clone_from(contents);
                }
                for (player_id, snapshot) in &release.prior_owners {
                    let player = prior.base.player.get_mut(player_id).ok_or_else(|| {
                        DraftGridClosureError::Invalid(
                            "release predecessor lost a production owner".into(),
                        )
                    })?;
                    player.experience = snapshot.experience;
                    player.career.clone_from(&snapshot.career);
                }
                for (machine_id, eligibility) in &release.released_eligibilities {
                    if prior
                        .imported_production_eligibilities
                        .insert(machine_id.clone(), eligibility.clone())
                        .is_some()
                    {
                        return Err(DraftGridClosureError::Invalid(
                            "release predecessor duplicated an imported eligibility".into(),
                        ));
                    }
                }
                if prior.calculate_active_world_hash()? != release.prior_active_world_hash {
                    return Err(DraftGridClosureError::Invalid(
                        "production release cannot reconstruct its exact predecessor world".into(),
                    ));
                }
                release
                    .controls
                    .validate_for_world(&prior.base, &prior.imported_production_eligibilities)?;
                let (mut replayed_world, replayed_outcomes) =
                    plan_imported_production_occurrence_v2(&prior.base, &release.controls)?;
                if replayed_outcomes != release.outcomes {
                    return Err(DraftGridClosureError::Invalid(
                        "production release outcomes do not replay from their predecessor".into(),
                    ));
                }
                replayed_world.event_sequence = release.release_event_sequence;
                replayed_world
                    .last_event_hash
                    .clone_from(&release.release_event_hash);
                let mut replayed = prior;
                replayed.base = replayed_world;
                let remaining_job_ids = replayed
                    .base
                    .production_queues
                    .values()
                    .flatten()
                    .map(|job| job.job_id.as_str())
                    .collect::<BTreeSet<_>>();
                replayed
                    .production_job_origins
                    .retain(|job_id, _| remaining_job_ids.contains(job_id.as_str()));
                for machine_id in release.released_eligibilities.keys() {
                    replayed
                        .imported_production_eligibilities
                        .remove(machine_id);
                }
                replayed.production_occurrence_history_count = release.resulting_history_count;
                replayed
                    .production_occurrence_history_head
                    .clone_from(&release.resulting_history_head);
                if replayed.calculate_active_world_hash()? != release.resulting_active_world_hash
                    || replayed.base != self.base
                    || replayed.imported_production_eligibilities
                        != self.imported_production_eligibilities
                    || replayed.production_job_origins != self.production_job_origins
                {
                    return Err(DraftGridClosureError::Invalid(
                        "production release did not commit one exact whole-cell occurrence".into(),
                    ));
                }
            }
        }
        if released_eligibility_hashes.len() + self.imported_production_eligibilities.len()
            > MAX_DRAFT_IMPORTED_PRODUCTION_HISTORY_PER_CELL
        {
            return Err(DraftGridClosureError::TooLarge);
        }
        for (machine_id, eligibility) in &self.imported_production_eligibilities {
            eligibility.validate()?;
            let import = self
                .committed_imports
                .get(eligibility.transfer_id())
                .ok_or_else(|| {
                    DraftGridClosureError::Invalid(
                        "import eligibility has no committed destination import".into(),
                    )
                })?;
            let queue = self.base.production_queues.get(machine_id).ok_or_else(|| {
                DraftGridClosureError::Invalid(
                    "import eligibility has no resident machine queue".into(),
                )
            })?;
            if machine_id != eligibility.machine_block_id()
                || eligibility.package_hash() != import.pending.reservation.binding.package_hash
                || import
                    .pending
                    .reservation
                    .frozen
                    .machine_block_ids
                    .binary_search(machine_id)
                    .is_err()
            {
                return Err(DraftGridClosureError::Invalid(
                    "import eligibility is outside its committed closure".into(),
                ));
            }
            eligibility.validate_persisted_in_world(&self.base, queue)?;
        }
        for (transfer_id, witness) in &self.abort_witnesses {
            witness.validate()?;
            witness
                .cleanup_proof()
                .validate()
                .map_err(DraftGridClosureError::Invalid)?;
            let at_abort_frontier = self.base.event_sequence == witness.cleanup_event_sequence
                && self.base.simulation_tick == witness.cleanup_simulation_tick;
            let prior_matches = if at_abort_frontier {
                let mut prior = self.clone();
                prior.abort_witnesses.remove(transfer_id);
                if let Some(lock) = &witness.removed_lock {
                    prior
                        .aggregate_locks
                        .insert(lock.binding.root_aggregate_id.clone(), lock.clone());
                }
                if let Some(reservation) = &witness.removed_reservation {
                    prior
                        .aggregate_reservations
                        .insert(reservation.binding.transfer_id.clone(), reservation.clone());
                }
                prior.base.event_sequence = witness.prior_event_sequence;
                prior
                    .base
                    .last_event_hash
                    .clone_from(&witness.prior_event_hash);
                prior.base.fencing_token = witness.live_fencing_token;
                prior.base.state_hash() == witness.base_world_hash
                    && prior.calculate_hash()? == witness.prior_draft_world_hash
            } else {
                true
            };
            if transfer_id != &witness.binding.transfer_id
                || witness.cell_id != self.base.cell_id
                || self.base.fencing_token < witness.live_fencing_token
                || self.base.simulation_tick < witness.cleanup_simulation_tick
                || self.base.event_sequence < witness.cleanup_event_sequence
                || (at_abort_frontier && self.base.last_event_hash != witness.cleanup_event_hash)
                || (at_abort_frontier
                    && self.calculate_active_world_hash()? != witness.resulting_draft_world_hash)
                || !prior_matches
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
        self.aggregate_locks
            .values()
            .find_map(|lock| {
                lock.frozen
                    .contains_subject(subject_id)
                    .then_some(lock.binding.transfer_id.as_str())
            })
            .or_else(|| {
                self.pending_imports.values().find_map(|pending| {
                    pending
                        .reservation
                        .frozen
                        .contains_subject(subject_id)
                        .then_some(pending.reservation.binding.transfer_id.as_str())
                })
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
        if grid.blocks.keys().any(|block_id| {
            self.imported_production_eligibilities
                .contains_key(block_id)
        }) {
            return Err(DraftGridClosureError::Invalid(
                "grid cannot hand off while a destination-bound production hold is live".into(),
            ));
        }
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
const MAX_DRAFT_IMPORTED_PRODUCTION_ELIGIBILITIES_PER_CELL: usize =
    MAX_DRAFT_GRID_PRODUCTION_QUEUES;
const MAX_DRAFT_PRODUCTION_OCCURRENCES_PER_CELL: usize = 2_048;
const MAX_DRAFT_IMPORTED_PRODUCTION_HISTORY_PER_CELL: usize = 8_192;

fn draft_transfer_count(state: &DraftGridTransferCellStateV2) -> usize {
    state
        .aggregate_locks
        .values()
        .map(|lock| lock.binding.transfer_id.as_str())
        .chain(state.aggregate_reservations.keys().map(String::as_str))
        .chain(state.pending_imports.keys().map(String::as_str))
        .chain(state.committed_prepares.keys().map(String::as_str))
        .chain(state.committed_quarantines.keys().map(String::as_str))
        .chain(state.committed_exports.keys().map(String::as_str))
        .chain(state.committed_imports.keys().map(String::as_str))
        .chain(state.committed_activations.keys().map(String::as_str))
        .chain(
            state
                .source_finalization_tombstones
                .keys()
                .map(String::as_str),
        )
        .chain(state.committed_finalizations.keys().map(String::as_str))
        .chain(state.abort_witnesses.keys().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .len()
}

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
    if !frozen_closure_subjects_are_present(world, frozen) {
        return false;
    }
    let grid = &world.grids[&frozen.grid_id];
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
    job_ids == frozen.job_ids
}

fn frozen_closure_subjects_are_present(
    world: &WorldState,
    frozen: &DraftFrozenClosureIdsV2,
) -> bool {
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
    let at_prepare_frontier = world.event_sequence == lock.prepare_event_sequence;
    let capture_world_matches = if at_prepare_frontier {
        let mut capture = world.clone();
        capture.event_sequence = lock.source_event_sequence;
        capture.last_event_hash.clone_from(&lock.source_event_hash);
        lock.source_base_world_hash == capture.state_hash()
    } else {
        true
    };
    world.event_sequence >= lock.prepare_event_sequence
        && (!at_prepare_frontier || world.last_event_hash == lock.prepare_event_hash)
        && capture_world_matches
        && lock.prepared_at_simulation_tick == world.simulation_tick
        && locked_closure_matches(world, lock)
}

fn source_capture_matches(world: &WorldState, lock: &DraftAggregateTransferLockV2) -> bool {
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

pub(super) fn stage_prepared_grid_event_v17(
    state: &DraftGridTransferCellStateV2,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
    event: &ValidatedDraftGridEventContextV17,
) -> Result<(DraftGridTransferCellStateV2, DraftGridPrepareProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    event.require_payload(&DraftGridEventPayloadV17::GridTransferPrepared {
        package: package.clone(),
        authority: authority.clone(),
    })?;
    if authority.phase != TransferPhase::Prepared
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || authority.quarantine_receipt_hash.is_some()
    {
        return Err(DraftGridClosureError::Invalid(
            "directory authority is not awaiting the first source prepare event".into(),
        ));
    }
    if state.base.cell_id != package.source_cell_id
        || state.base.fencing_token != authority.live_source_fencing_token
        || package.production_queues.keys().any(|machine_id| {
            state
                .imported_production_eligibilities
                .contains_key(machine_id)
        })
        || package
            .production_job_origins
            .iter()
            .any(|(job_id, origin)| state.production_job_origins.get(job_id) != Some(origin))
    {
        return Err(DraftGridClosureError::Invalid(
            "source draft cell does not own the package fence and authoritative job origins".into(),
        ));
    }
    if let Some(existing) = state.aggregate_locks.get(&package.root_aggregate_id) {
        let _ = existing;
        return Err(DraftGridClosureError::Changed(
            "source prepare apply requires its exact predecessor; reconcile committed state separately"
                .into(),
        ));
    }
    if event.event_sequence != state.base.event_sequence.checked_add(1).unwrap_or(0)
        || event.previous_event_hash != state.base.last_event_hash
        || event.authority_fencing_token != state.base.fencing_token
        || event.authority_fencing_token != authority.live_source_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "grid prepare event-17 context does not follow the live source frontier".into(),
        ));
    }
    let mut proof = DraftGridPrepareProofV2::new(state, package, authority, event)?;
    let expected = DraftAggregateTransferLockV2::from_package(package, &proof);
    if !source_capture_matches(&state.base, &expected) {
        return Err(DraftGridClosureError::Changed(
            "source closure changed before the aggregate lock became durable".into(),
        ));
    }
    if state.aggregate_locks.values().any(|lock| {
        lock.binding.transfer_id == package.transfer_id || lock.frozen.overlaps(&expected.frozen)
    }) || state.aggregate_reservations.values().any(|reservation| {
        reservation.binding.transfer_id == package.transfer_id
            || reservation.frozen.overlaps(&expected.frozen)
    }) || state.committed_prepares.contains_key(&package.transfer_id)
        || state
            .committed_quarantines
            .contains_key(&package.transfer_id)
        || state.committed_exports.contains_key(&package.transfer_id)
        || state.abort_witnesses.contains_key(&package.transfer_id)
    {
        return Err(DraftGridClosureError::Changed(
            "another aggregate transfer already freezes a closure subject".into(),
        ));
    }
    let mut next = state.clone();
    next.aggregate_locks
        .insert(package.root_aggregate_id.clone(), expected);
    next.base.event_sequence = event.event_sequence;
    next.base.last_event_hash.clone_from(&event.event_hash);
    proof.seal_result(&next)?;
    next.committed_prepares
        .insert(package.transfer_id.clone(), proof.clone());
    next.seal()?;
    Ok((next, proof))
}

pub(super) fn reconcile_prepared_grid_v2(
    state: &DraftGridTransferCellStateV2,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<(DraftGridTransferCellStateV2, DraftGridPrepareProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if state.base.cell_id != package.source_cell_id
        || state.base.fencing_token != authority.live_source_fencing_token
        || !matches!(
            authority.phase,
            TransferPhase::Prepared | TransferPhase::Quarantined | TransferPhase::Committed
        )
    {
        return Err(DraftGridClosureError::Invalid(
            "source prepare reconciliation lacks current source authority".into(),
        ));
    }
    if matches!(
        authority.phase,
        TransferPhase::Quarantined | TransferPhase::Committed
    ) && (!authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || authority.quarantine_receipt_hash.is_none())
    {
        return Err(DraftGridClosureError::Invalid(
            "later prepare reconciliation lacks durable precommit directory proofs".into(),
        ));
    }
    let lock = state
        .aggregate_locks
        .get(&package.root_aggregate_id)
        .filter(|lock| lock.matches_package(package))
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "source prepare reconciliation cannot find the exact resident lock".into(),
            )
        })?;
    let proof = state
        .committed_prepares
        .get(&package.transfer_id)
        .filter(|proof| {
            proof.root_aggregate_id == lock.binding.root_aggregate_id
                && proof.member_root == lock.binding.member_root
                && proof.package_hash == lock.binding.package_hash
                && proof.event_sequence == lock.prepare_event_sequence
                && proof.event_hash == lock.prepare_event_hash
                && proof.event_payload_hash == lock.prepare_event_payload_hash
                && proof.mutation_witness_hash == lock.prepare_mutation_witness_hash
        })
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "source prepare reconciliation cannot find the exact canonical proof".into(),
            )
        })?;
    proof.validate()?;
    if authority
        .source_prepare_proof
        .as_ref()
        .is_some_and(|directory_proof| directory_proof != proof)
    {
        return Err(DraftGridClosureError::Changed(
            "source prepare reconciliation conflicts with the directory's canonical proof".into(),
        ));
    }
    Ok((state.clone(), proof.clone()))
}

#[cfg(test)]
fn stage_prepared_grid_lock_v2(
    state: &DraftGridTransferCellStateV2,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<DraftGridTransferCellStateV2, DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if state.base.fencing_token != authority.live_source_fencing_token {
        return Err(DraftGridClosureError::Invalid(
            "test prepare caller does not own the live source fence".into(),
        ));
    }
    if state
        .aggregate_locks
        .contains_key(&package.root_aggregate_id)
    {
        return reconcile_prepared_grid_v2(state, package, authority).map(|(state, _)| state);
    }
    let event = DraftCanonicalGridEventV17::new_system(
        state,
        format!("prepare-{}", package.transfer_id),
        1_800_000_000_000,
        DraftGridEventPayloadV17::GridTransferPrepared {
            package: package.clone(),
            authority: authority.clone(),
        },
    )?;
    let context = event.validate_for_state(state)?;
    stage_prepared_grid_event_v17(state, package, authority, &context).map(|(next, _)| next)
}

pub(super) fn stage_grid_quarantine_event_v17(
    state: &DraftGridTransferCellStateV2,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
    event: &ValidatedDraftGridEventContextV17,
) -> Result<
    (
        DraftGridTransferCellStateV2,
        DraftGridTransferQuarantineReceiptV2,
        DraftGridQuarantineProofV2,
    ),
    DraftGridClosureError,
> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    event.require_payload(&DraftGridEventPayloadV17::GridTransferQuarantined {
        package: package.clone(),
        authority: authority.clone(),
    })?;
    if state.base.cell_id != package.destination_cell_id
        || state.base.fencing_token != authority.live_destination_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "destination draft cell does not own the live directory fence".into(),
        ));
    }
    if let Some(existing) = state.aggregate_reservations.get(&package.transfer_id) {
        let _ = existing;
        return Err(DraftGridClosureError::Changed(
            "destination quarantine apply requires its exact predecessor; reconcile committed state separately"
                .into(),
        ));
    }
    if authority.phase != TransferPhase::Prepared
        || !authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || authority.quarantine_receipt_hash.is_some()
        || event.occurred_at_unix_ms == 0
    {
        return Err(DraftGridClosureError::Invalid(
            "directory authority is not awaiting first destination quarantine".into(),
        ));
    }
    if event.event_sequence != state.base.event_sequence.checked_add(1).unwrap_or(0)
        || event.previous_event_hash != state.base.last_event_hash
        || event.authority_fencing_token != state.base.fencing_token
        || event.authority_fencing_token != authority.live_destination_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "grid quarantine event-17 context does not follow the live destination frontier".into(),
        ));
    }
    validate_destination_conflicts_in_validated_world_v21(
        &state.base,
        package,
        authority.live_destination_fencing_token,
    )?;
    let frozen = DraftFrozenClosureIdsV2::from_package(package);
    if state.aggregate_locks.values().any(|lock| {
        lock.binding.transfer_id == package.transfer_id || lock.frozen.overlaps(&frozen)
    }) || state.aggregate_reservations.values().any(|reservation| {
        reservation.binding.transfer_id == package.transfer_id
            || reservation.frozen.overlaps(&frozen)
    }) || state.committed_prepares.contains_key(&package.transfer_id)
        || state
            .committed_quarantines
            .contains_key(&package.transfer_id)
        || state.committed_exports.contains_key(&package.transfer_id)
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
        quarantined_at_unix_ms: event.occurred_at_unix_ms,
        receipt_hash: String::new(),
    };
    receipt.receipt_hash = receipt.calculate_hash()?;
    receipt.validate()?;
    let mut proof = DraftGridQuarantineProofV2::new(state, package, authority, event, &receipt)?;
    let reservation = DraftAggregateTransferReservationV2::from_receipt(package, &receipt, &proof);
    reservation.validate()?;
    let mut next = state.clone();
    next.aggregate_reservations
        .insert(package.transfer_id.clone(), reservation);
    next.base.event_sequence = event.event_sequence;
    next.base.last_event_hash.clone_from(&event.event_hash);
    proof.seal_result(&next)?;
    next.committed_quarantines
        .insert(package.transfer_id.clone(), proof.clone());
    next.seal()?;
    Ok((next, receipt, proof))
}

pub(super) fn reconcile_quarantined_grid_v2(
    state: &DraftGridTransferCellStateV2,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<
    (
        DraftGridTransferCellStateV2,
        DraftGridTransferQuarantineReceiptV2,
        DraftGridQuarantineProofV2,
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
            "quarantine reconciliation lacks current destination authority".into(),
        ));
    }
    let reservation = state
        .aggregate_reservations
        .get(&package.transfer_id)
        .filter(|reservation| {
            reservation.binding == DraftGridTransferBindingV2::from_package(package)
                && reservation.frozen == DraftFrozenClosureIdsV2::from_package(package)
        })
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "quarantine reconciliation cannot find the exact reservation".into(),
            )
        })?;
    let receipt = reservation.receipt();
    let proof = state
        .committed_quarantines
        .get(&package.transfer_id)
        .filter(|proof| {
            proof.quarantine_receipt_hash == receipt.receipt_hash
                && proof.event_sequence == reservation.quarantine_event_sequence
                && proof.event_hash == reservation.quarantine_event_hash
                && proof.event_payload_hash == reservation.quarantine_event_payload_hash
                && proof.mutation_witness_hash == reservation.quarantine_mutation_witness_hash
        })
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "quarantine reconciliation cannot find the exact canonical proof".into(),
            )
        })?;
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
        TransferPhase::Aborting => {
            authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
                && (!authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
                    || (authority.quarantine_receipt_hash.as_deref()
                        == Some(receipt.receipt_hash.as_str())
                        && authority.destination_quarantine_proof.as_ref() == Some(proof)))
        }
        _ => false,
    };
    if !phase_matches {
        return Err(DraftGridClosureError::Invalid(
            "quarantine reconciliation lacks matching directory authority".into(),
        ));
    }
    receipt.validate()?;
    proof.validate()?;
    if authority
        .destination_quarantine_proof
        .as_ref()
        .is_some_and(|directory_proof| directory_proof != proof)
    {
        return Err(DraftGridClosureError::Changed(
            "quarantine reconciliation conflicts with the directory's canonical proof".into(),
        ));
    }
    Ok((state.clone(), receipt, proof.clone()))
}

#[cfg(test)]
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
    if state
        .aggregate_reservations
        .contains_key(&package.transfer_id)
    {
        return reconcile_quarantined_grid_v2(state, package, authority)
            .map(|(state, receipt, _)| (state, receipt));
    }
    let event = DraftCanonicalGridEventV17::new_system(
        state,
        format!("quarantine-{}", package.transfer_id),
        trusted_now_unix_ms,
        DraftGridEventPayloadV17::GridTransferQuarantined {
            package: package.clone(),
            authority: authority.clone(),
        },
    )?;
    let context = event.validate_for_state(state)?;
    stage_grid_quarantine_event_v17(state, package, authority, &context)
        .map(|(next, receipt, _)| (next, receipt))
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
    let expected_lock = state
        .aggregate_locks
        .get(&package.root_aggregate_id)
        .cloned()
        .filter(|lock| lock.matches_package(package))
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "source export no longer matches the exact frozen package closure".into(),
            )
        })?;
    if !source_lock_matches(&state.base, &expected_lock) {
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

fn stage_committed_grid_import_v2(
    state: &DraftGridTransferCellStateV2,
    trusted_now_unix_ms: u64,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<(DraftGridTransferCellStateV2, DraftGridImportProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if !authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::SourceExport)
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourceAbort)
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationAbort)
        || state.base.cell_id != package.destination_cell_id
        || state.base.fencing_token != authority.live_destination_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "destination import lacks exact committed directory and live-fence authority".into(),
        ));
    }
    let source_export_proof = authority.source_export_proof.clone().ok_or_else(|| {
        DraftGridClosureError::Invalid(
            "destination import lacks the authenticated source-export proof".into(),
        )
    })?;
    if let Some(existing) = state.committed_imports.get(&package.transfer_id) {
        let existing_proof = existing.proof();
        let directory_retry_matches = match authority.phase {
            TransferPhase::Committed => {
                !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationImport)
                    && !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationActivation)
                    && !authority.has_proof(DraftGridDirectoryProofKindV2::SourceFinalization)
                    && authority.destination_import_proof.is_none()
            }
            TransferPhase::Imported | TransferPhase::Finalized => {
                authority.destination_import_proof.as_ref() == Some(&existing_proof)
            }
            _ => false,
        };
        if !directory_retry_matches {
            return Err(DraftGridClosureError::Changed(
                "destination-import retry conflicts with durable directory import evidence".into(),
            ));
        }
        existing.validate_request(package, authority)?;
        let pending_matches =
            state.pending_imports.get(&package.transfer_id) == Some(&existing.pending);
        let activation_matches = state
            .committed_activations
            .get(&package.transfer_id)
            .is_some_and(|activation| activation.pending == existing.pending);
        if pending_matches == activation_matches
            || state.base.transfer_witnesses.get(&package.transfer_id)
                != Some(&existing.pending.conservation_witness)
        {
            return Err(DraftGridClosureError::Changed(
                "destination-import retry conflicts with its durable active authority".into(),
            ));
        }
        return Ok((state.clone(), existing_proof));
    }
    if authority.phase != TransferPhase::Committed
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationImport)
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationActivation)
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourceFinalization)
        || authority.destination_import_proof.is_some()
    {
        return Err(DraftGridClosureError::Invalid(
            "first destination import requires the exact pre-import directory boundary".into(),
        ));
    }
    if trusted_now_unix_ms == 0
        || state.pending_imports.contains_key(&package.transfer_id)
        || state.committed_exports.contains_key(&package.transfer_id)
        || state.abort_witnesses.contains_key(&package.transfer_id)
    {
        return Err(DraftGridClosureError::Invalid(
            "destination import is not at a clean committed boundary".into(),
        ));
    }
    let reservation = state
        .aggregate_reservations
        .get(&package.transfer_id)
        .cloned()
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "destination import lost its exact quarantine reservation".into(),
            )
        })?;
    let expected_binding = DraftGridTransferBindingV2::from_package(package);
    let expected_frozen = DraftFrozenClosureIdsV2::from_package(package);
    if reservation.binding != expected_binding
        || reservation.frozen != expected_frozen
        || authority.quarantine_receipt_hash.as_deref() != Some(reservation.receipt_hash.as_str())
        || source_export_proof.quarantine_receipt_hash != reservation.receipt_hash
        || trusted_now_unix_ms < reservation.quarantined_at_unix_ms
        || trusted_now_unix_ms < source_export_proof.exported_at_unix_ms
    {
        return Err(DraftGridClosureError::Changed(
            "destination import changed its reservation, receipt, export proof, or trusted time"
                .into(),
        ));
    }
    validate_destination_conflicts_in_validated_world_v21(
        &state.base,
        package,
        authority.live_destination_fencing_token,
    )?;
    if state.aggregate_locks.values().any(|lock| {
        lock.binding.transfer_id == package.transfer_id || lock.frozen.overlaps(&expected_frozen)
    }) || state
        .aggregate_reservations
        .iter()
        .any(|(transfer_id, other)| {
            transfer_id != &package.transfer_id && other.frozen.overlaps(&expected_frozen)
        })
        || state.pending_imports.values().any(|pending| {
            pending.reservation.binding.transfer_id == package.transfer_id
                || pending.reservation.frozen.overlaps(&expected_frozen)
        })
    {
        return Err(DraftGridClosureError::Changed(
            "destination import overlaps another active aggregate transfer".into(),
        ));
    }

    let ledger_vector = DraftGridTransferLedgerVectorV2::from_package(package)?;
    if source_export_proof.ledger_vector != ledger_vector {
        return Err(DraftGridClosureError::Changed(
            "destination import ledger vector differs from the authenticated source export".into(),
        ));
    }
    let conservation_witness =
        grid_transfer_witness(package, TransferWitnessDirection::Import, ledger_vector);
    let mut pending = DraftPendingGridImportV2 {
        schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
        reservation: reservation.clone(),
        source_export_proof,
        destination_assignment_generation: authority.live_destination_assignment_generation,
        historical_fencing_token: package.destination_fencing_token,
        live_fencing_token: authority.live_destination_fencing_token,
        prior_event_sequence: state.base.event_sequence,
        prior_event_hash: state.base.last_event_hash.clone(),
        import_event_sequence: state.base.event_sequence.checked_add(1).ok_or_else(|| {
            DraftGridClosureError::Unsupported("destination-import event sequence exhausted".into())
        })?,
        import_event_hash: String::new(),
        prior_draft_world_hash: state.state_hash.clone(),
        ledger_vector,
        conservation_witness,
        production_eligibility_root: imported_production_eligibility_map_root(&BTreeMap::new())?,
        destination_production_lifecycle_generation: state
            .base
            .production_clock
            .lifecycle_generation,
        imported_at_unix_ms: trusted_now_unix_ms,
        mutation_witness_hash: String::new(),
    };
    pending.mutation_witness_hash = pending.calculate_mutation_hash()?;
    pending.import_event_hash = pending.calculate_event_hash()?;
    let production_authority = DraftProductionImportAuthorityV2::from_committed_import(
        package,
        &pending.import_boundary(),
    )?;
    let eligibilities = derive_imported_production_eligibilities(package, &production_authority)?;
    pending.production_eligibility_root = imported_production_eligibility_map_root(&eligibilities)?;
    pending.validate()?;

    let mut next = state.clone();
    if next
        .aggregate_reservations
        .remove(&package.transfer_id)
        .as_ref()
        != Some(&reservation)
    {
        return Err(DraftGridClosureError::Changed(
            "destination import could not consume its exact reservation".into(),
        ));
    }
    let destination_origin = celestial::cell_address_from_key(&package.destination_cell_key)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    let mut grid = package.grid.clone();
    grid.position = celestial::local_position_from_address(&destination_origin, &grid.address)
        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
    if next.base.grids.insert(grid.grid_id.clone(), grid).is_some() {
        return Err(DraftGridClosureError::Changed(
            "destination import overwrote a resident grid".into(),
        ));
    }
    for (inventory_id, inventory) in &package.cargo_inventories {
        if next
            .base
            .inventories
            .insert(inventory_id.clone(), inventory.clone())
            .is_some()
        {
            return Err(DraftGridClosureError::Changed(
                "destination import overwrote a cargo inventory".into(),
            ));
        }
    }
    for (machine_id, queue) in &package.production_queues {
        if next
            .base
            .production_queues
            .insert(machine_id.clone(), queue.clone())
            .is_some()
        {
            return Err(DraftGridClosureError::Changed(
                "destination import overwrote a production queue".into(),
            ));
        }
    }
    for (job_id, origin) in &package.production_job_origins {
        if next
            .production_job_origins
            .insert(job_id.clone(), origin.clone())
            .is_some()
        {
            return Err(DraftGridClosureError::Changed(
                "destination import overwrote production provenance".into(),
            ));
        }
    }
    for (player_id, player) in &package.players {
        if next
            .base
            .inventories
            .insert(
                player.inventory.inventory_id.clone(),
                player.inventory.clone(),
            )
            .is_some()
            || next
                .base
                .player
                .by_id
                .insert(player_id.clone(), player.destination_player.clone())
                .is_some()
        {
            return Err(DraftGridClosureError::Changed(
                "destination import overwrote a rider or rider inventory".into(),
            ));
        }
        if let Some(history) = &player.operation_history
            && next
                .base
                .processed_operations
                .insert(player_id.clone(), history.clone())
                .is_some()
        {
            return Err(DraftGridClosureError::Changed(
                "destination import overwrote a rider operation history".into(),
            ));
        }
    }
    if next.base.player.primary_player_id.is_empty() {
        next.base
            .player
            .primary_player_id
            .clone_from(&package.grid.owner_player_id);
    }
    for contact in &package.active_internal_contacts {
        if !next.base.active_contact_pairs.insert(contact.clone()) {
            return Err(DraftGridClosureError::Changed(
                "destination import duplicated an internal contact".into(),
            ));
        }
    }
    insert_grid_transfer_witness(&mut next.base, pending.conservation_witness.clone())?;
    if !next.base.conservation().valid {
        return Err(DraftGridClosureError::Invalid(
            "destination import does not conserve its exact transfer vector".into(),
        ));
    }
    next.base.event_sequence = pending.import_event_sequence;
    next.base
        .last_event_hash
        .clone_from(&pending.import_event_hash);
    for (machine_id, eligibility) in eligibilities {
        if next
            .imported_production_eligibilities
            .insert(machine_id, eligibility)
            .is_some()
        {
            return Err(DraftGridClosureError::Changed(
                "destination import overwrote a production eligibility".into(),
            ));
        }
    }
    if next
        .pending_imports
        .insert(package.transfer_id.clone(), pending.clone())
        .is_some()
    {
        return Err(DraftGridClosureError::Changed(
            "destination import overwrote a pending activation lock".into(),
        ));
    }
    let resulting_active_world_hash = next.calculate_active_world_hash()?;
    let record = DraftGridImportRecordV2::new(pending, resulting_active_world_hash)?;
    let proof = record.proof();
    if next
        .committed_imports
        .insert(package.transfer_id.clone(), record)
        .is_some()
    {
        return Err(DraftGridClosureError::Changed(
            "destination import overwrote historical import evidence".into(),
        ));
    }
    next.seal()?;
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    Ok((next, proof))
}

fn stage_imported_grid_activation_v2(
    state: &DraftGridTransferCellStateV2,
    trusted_now_unix_ms: u64,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<(DraftGridTransferCellStateV2, DraftGridActivationProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if !authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::SourceExport)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationImport)
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourceAbort)
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationAbort)
        || state.base.cell_id != package.destination_cell_id
        || state.base.fencing_token != authority.live_destination_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "destination activation lacks exact imported directory and live-fence authority".into(),
        ));
    }
    let import = state
        .committed_imports
        .get(&package.transfer_id)
        .cloned()
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "destination activation lost its historical import evidence".into(),
            )
        })?;
    let import_proof = import.proof();
    if authority.destination_import_proof.as_ref() != Some(&import_proof) {
        return Err(DraftGridClosureError::Changed(
            "destination activation changed its directory-retained import proof".into(),
        ));
    }
    if let Some(existing) = state.committed_activations.get(&package.transfer_id) {
        existing.validate_with_import(&import)?;
        let existing_proof = existing.proof();
        let directory_retry_matches = match authority.phase {
            TransferPhase::Imported => {
                if authority.has_proof(DraftGridDirectoryProofKindV2::DestinationActivation) {
                    authority.destination_activation_proof.as_ref() == Some(&existing_proof)
                } else {
                    authority.destination_activation_proof.is_none()
                }
            }
            TransferPhase::Finalized => {
                authority.destination_activation_proof.as_ref() == Some(&existing_proof)
            }
            _ => false,
        };
        if !directory_retry_matches
            || state.pending_imports.contains_key(&package.transfer_id)
            || state.base.transfer_witnesses.get(&package.transfer_id)
                != Some(&existing.pending.conservation_witness)
        {
            return Err(DraftGridClosureError::Changed(
                "destination-activation retry conflicts with durable cell or directory evidence"
                    .into(),
            ));
        }
        return Ok((state.clone(), existing_proof));
    }
    if authority.phase != TransferPhase::Imported
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationActivation)
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourceFinalization)
        || authority.destination_activation_proof.is_some()
        || trusted_now_unix_ms < import.pending.imported_at_unix_ms
    {
        return Err(DraftGridClosureError::Invalid(
            "first destination activation requires the exact pre-activation imported boundary"
                .into(),
        ));
    }
    let pending = state
        .pending_imports
        .get(&package.transfer_id)
        .cloned()
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "destination activation lost its pending imported closure".into(),
            )
        })?;
    if pending != import.pending
        || pending.reservation.binding != DraftGridTransferBindingV2::from_package(package)
        || pending.reservation.frozen != DraftFrozenClosureIdsV2::from_package(package)
    {
        return Err(DraftGridClosureError::Changed(
            "destination activation changed its exact imported closure".into(),
        ));
    }

    let mut record = DraftGridActivationRecordV2::new(
        pending.clone(),
        authority.live_destination_assignment_generation,
        authority.live_destination_fencing_token,
        state.base.event_sequence,
        state.base.last_event_hash.clone(),
        state.calculate_active_world_hash()?,
        import.proof_hash.clone(),
        trusted_now_unix_ms,
    )?;
    let mut next = state.clone();
    if next.pending_imports.remove(&package.transfer_id).as_ref() != Some(&pending) {
        return Err(DraftGridClosureError::Changed(
            "destination activation could not release its exact pending lock".into(),
        ));
    }
    next.base.event_sequence = record.activation_event_sequence;
    next.base
        .last_event_hash
        .clone_from(&record.activation_event_hash);
    record.seal_resulting_active_world_hash(&next)?;
    record.validate_with_import(&import)?;
    let proof = record.proof();
    if next
        .committed_activations
        .insert(package.transfer_id.clone(), record)
        .is_some()
    {
        return Err(DraftGridClosureError::Changed(
            "destination activation overwrote historical activation evidence".into(),
        ));
    }
    next.seal()?;
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    Ok((next, proof))
}

fn stage_finalized_grid_source_v2(
    state: &DraftGridTransferCellStateV2,
    trusted_now_unix_ms: u64,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<(DraftGridTransferCellStateV2, DraftGridFinalizationProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if !authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::SourceExport)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationImport)
        || !authority.has_proof(DraftGridDirectoryProofKindV2::DestinationActivation)
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourceAbort)
        || authority.has_proof(DraftGridDirectoryProofKindV2::DestinationAbort)
        || state.base.cell_id != package.source_cell_id
        || state.base.fencing_token != authority.live_source_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "source finalization lacks exact activated directory and live-fence authority".into(),
        ));
    }
    let export = state
        .committed_exports
        .get(&package.transfer_id)
        .cloned()
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "source finalization lost its historical committed export".into(),
            )
        })?;
    export.validate_request(package, authority)?;
    let export_proof = export.proof();
    if authority.source_export_proof.as_ref() != Some(&export_proof) {
        return Err(DraftGridClosureError::Changed(
            "source finalization changed its directory-retained export proof".into(),
        ));
    }
    if let Some(existing) = state.committed_finalizations.get(&package.transfer_id) {
        existing.validate_with_export(&export)?;
        let existing_proof = existing.proof();
        let directory_retry_matches = match authority.phase {
            TransferPhase::Imported => {
                !authority.has_proof(DraftGridDirectoryProofKindV2::SourceFinalization)
                    && authority.source_finalization_proof.is_none()
            }
            TransferPhase::Finalized => {
                authority.source_finalization_proof.as_ref() == Some(&existing_proof)
            }
            _ => false,
        };
        if !directory_retry_matches
            || state.base.transfer_witnesses.get(&package.transfer_id)
                != Some(&existing.conservation_witness)
            || !frozen_closure_is_absent(&state.base, &existing.frozen)
        {
            return Err(DraftGridClosureError::Changed(
                "source-finalization retry conflicts with durable cell or directory evidence"
                    .into(),
            ));
        }
        return Ok((state.clone(), existing_proof));
    }
    let activation = authority
        .destination_activation_proof
        .as_ref()
        .ok_or_else(|| {
            DraftGridClosureError::Invalid(
                "first source finalization lacks destination activation evidence".into(),
            )
        })?;
    if authority.phase != TransferPhase::Imported
        || authority.has_proof(DraftGridDirectoryProofKindV2::SourceFinalization)
        || authority.source_finalization_proof.is_some()
        || trusted_now_unix_ms < activation.activated_at_unix_ms
        || state
            .aggregate_locks
            .contains_key(&package.root_aggregate_id)
        || state
            .aggregate_reservations
            .contains_key(&package.transfer_id)
        || state.pending_imports.contains_key(&package.transfer_id)
        || state.committed_imports.contains_key(&package.transfer_id)
        || state
            .committed_activations
            .contains_key(&package.transfer_id)
        || state.abort_witnesses.contains_key(&package.transfer_id)
        || !frozen_closure_is_absent(&state.base, &export.frozen)
    {
        return Err(DraftGridClosureError::Invalid(
            "first source finalization requires one clean post-activation export tombstone".into(),
        ));
    }

    let mut record =
        DraftGridFinalizationRecordV2::new(state, &export, authority, trusted_now_unix_ms)?;
    let mut next = state.clone();
    next.base.event_sequence = record.finalization_event_sequence;
    next.base
        .last_event_hash
        .clone_from(&record.finalization_event_hash);
    if next
        .source_finalization_tombstones
        .insert(package.transfer_id.clone(), record.tombstone())
        .is_some()
    {
        return Err(DraftGridClosureError::Changed(
            "source finalization overwrote an active audit tombstone".into(),
        ));
    }
    record.seal_resulting_active_world_hash(&next, &export)?;
    let proof = record.proof();
    if next
        .committed_finalizations
        .insert(package.transfer_id.clone(), record)
        .is_some()
    {
        return Err(DraftGridClosureError::Changed(
            "source finalization overwrote historical finalization evidence".into(),
        ));
    }
    next.seal()?;
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    Ok((next, proof))
}

fn plan_imported_production_occurrence_v2(
    world: &WorldState,
    controls: &DraftImportedProductionOccurrenceControlsV2,
) -> Result<(WorldState, Vec<DraftProductionMachineOccurrenceOutcomeV2>), DraftGridClosureError> {
    let mut planning = world.clone();
    let mut outcomes = Vec::with_capacity(controls.machines().len());
    for control in controls.machines() {
        let ordinary_outcome = match control.kind() {
            DraftProductionMachineControlKindV2::TransferPaused => None,
            DraftProductionMachineControlKindV2::Evaluate
            | DraftProductionMachineControlKindV2::ReleaseAndEvaluate => {
                let outcome = planning
                    .production_machine_outcome_after_one_second(control.machine_block_id())
                    .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
                if outcome.changes_state() {
                    planning
                        .apply_production_machine_outcome(&outcome)
                        .map_err(|source| DraftGridClosureError::Invalid(source.to_string()))?;
                }
                Some(outcome)
            }
        };
        outcomes.push(DraftProductionMachineOccurrenceOutcomeV2 {
            control: control.clone(),
            ordinary_outcome,
        });
    }
    planning.production_clock.last_committed_quantum_sequence =
        controls.occurrence().production_quantum_sequence;
    planning.production_clock.last_scheduled_for_unix_ms =
        controls.occurrence().scheduled_for_unix_ms;
    Ok((planning, outcomes))
}

fn stage_imported_production_occurrence_v2(
    state: &DraftGridTransferCellStateV2,
    accepted_trusted_now_unix_ms: u64,
    occurrence: crate::event::ProductionScheduleOccurrence,
) -> Result<
    (
        DraftGridTransferCellStateV2,
        DraftImportedProductionOccurrenceControlsV2,
        DraftImportedProductionReleaseProofV2,
    ),
    DraftGridClosureError,
> {
    state.validate()?;
    if accepted_trusted_now_unix_ms < occurrence.scheduled_for_unix_ms {
        return Err(DraftGridClosureError::Invalid(
            "production occurrence was delivered before its trusted scheduled time".into(),
        ));
    }
    if let Some(existing) = state
        .committed_production_releases
        .values()
        .find(|record| record.controls.occurrence() == &occurrence)
    {
        existing.validate_static()?;
        let proof = existing.proof()?;
        proof.validate().map_err(DraftGridClosureError::Invalid)?;
        return Ok((state.clone(), existing.controls.clone(), proof));
    }
    if state.imported_production_eligibilities.is_empty() {
        return Err(DraftGridClosureError::Invalid(
            "imported production staging requires at least one live eligibility".into(),
        ));
    }
    let controls = derive_imported_production_occurrence_controls(
        &state.base,
        &state.imported_production_eligibilities,
        occurrence,
    )?;
    let (mut resulting_world, outcomes) =
        plan_imported_production_occurrence_v2(&state.base, &controls)?;
    let released_eligibilities = controls
        .machines()
        .iter()
        .filter(|control| control.kind() == DraftProductionMachineControlKindV2::ReleaseAndEvaluate)
        .map(|control| {
            state
                .imported_production_eligibilities
                .get(control.machine_block_id())
                .cloned()
                .map(|eligibility| (control.machine_block_id().to_owned(), eligibility))
                .ok_or_else(|| {
                    DraftGridClosureError::Changed(
                        "due imported production eligibility disappeared before commit".into(),
                    )
                })
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut record = DraftImportedProductionReleaseRecordV2::new(
        state,
        controls.clone(),
        outcomes,
        released_eligibilities.clone(),
        accepted_trusted_now_unix_ms,
    )?;
    resulting_world.event_sequence = record.release_event_sequence;
    resulting_world
        .last_event_hash
        .clone_from(&record.release_event_hash);
    let mut next = state.clone();
    next.base = resulting_world;
    next.production_occurrence_history_count = record.resulting_history_count;
    next.production_occurrence_history_head
        .clone_from(&record.resulting_history_head);
    let remaining_job_ids = next
        .base
        .production_queues
        .values()
        .flatten()
        .map(|job| job.job_id.as_str())
        .collect::<BTreeSet<_>>();
    next.production_job_origins
        .retain(|job_id, _| remaining_job_ids.contains(job_id.as_str()));
    for (machine_id, eligibility) in &released_eligibilities {
        if next
            .imported_production_eligibilities
            .remove(machine_id)
            .as_ref()
            != Some(eligibility)
        {
            return Err(DraftGridClosureError::Changed(
                "production occurrence could not consume its exact due eligibility".into(),
            ));
        }
    }
    record.seal_resulting_active_world_hash(&next)?;
    let proof = record.proof()?;
    if next
        .committed_production_releases
        .insert(controls.controls_root().to_owned(), record)
        .is_some()
    {
        return Err(DraftGridClosureError::Changed(
            "production occurrence overwrote historical gate evidence".into(),
        ));
    }
    next.seal()?;
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    Ok((next, controls, proof))
}

pub(super) fn stage_aborted_grid_cleanup_event_v17(
    state: &DraftGridTransferCellStateV2,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
    event: &ValidatedDraftGridEventContextV17,
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
    event.require_payload(&DraftGridEventPayloadV17::GridTransferAborted {
        package: package.clone(),
        authority: authority.clone(),
        side,
    })?;
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
        let _ = existing;
        return Err(DraftGridClosureError::Changed(
            "abort cleanup apply requires its exact predecessor; reconcile committed state separately"
                .into(),
        ));
    }
    if authority.phase != TransferPhase::Aborting || event.occurred_at_unix_ms == 0 {
        return Err(DraftGridClosureError::Invalid(
            "only an aborting precommit transfer may clean cell authority".into(),
        ));
    }
    if event.event_sequence != state.base.event_sequence.checked_add(1).unwrap_or(0)
        || event.previous_event_hash != state.base.last_event_hash
        || event.authority_fencing_token != state.base.fencing_token
        || event.authority_fencing_token != live_fencing_token
    {
        return Err(DraftGridClosureError::Invalid(
            "grid abort event-17 context does not follow the live cell frontier".into(),
        ));
    }
    let mut next = state.clone();
    let (removed_lock, removed_reservation) = match side {
        DraftGridTransferAbortSideV2::Source => {
            if authority.has_proof(DraftGridDirectoryProofKindV2::SourceAbort) {
                return Err(DraftGridClosureError::Changed(
                    "directory retains a source-abort proof absent from cell state".into(),
                ));
            }
            if state
                .aggregate_locks
                .get(&package.root_aggregate_id)
                .is_some_and(|existing| !existing.matches_package(package))
            {
                return Err(DraftGridClosureError::Changed(
                    "source abort does not match the exact aggregate lock".into(),
                ));
            }
            let removed = next.aggregate_locks.remove(&package.root_aggregate_id);
            if removed.is_none()
                && authority.has_proof(DraftGridDirectoryProofKindV2::SourcePrepare)
            {
                return Err(DraftGridClosureError::Changed(
                    "directory proves a source lock that is absent from cell state".into(),
                ));
            }
            (removed, None)
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
            let removed = next.aggregate_reservations.remove(&package.transfer_id);
            if removed.is_none()
                && authority.has_proof(DraftGridDirectoryProofKindV2::DestinationQuarantine)
            {
                return Err(DraftGridClosureError::Changed(
                    "directory proves a destination reservation that is absent from cell state"
                        .into(),
                ));
            }
            (None, removed)
        }
    };
    next.base.event_sequence = event.event_sequence;
    next.base.last_event_hash.clone_from(&event.event_hash);
    let witness = DraftGridTransferAbortWitnessV2::new(
        state,
        &next,
        package,
        side,
        removed_lock,
        removed_reservation,
        authority,
        event,
    )?;
    next.abort_witnesses
        .insert(package.transfer_id.clone(), witness.clone());
    next.seal()?;
    let proof = witness.cleanup_proof();
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    Ok((next, proof))
}

pub(super) fn reconcile_aborted_grid_cleanup_v2(
    state: &DraftGridTransferCellStateV2,
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
            "abort reconciliation was presented to an unrelated cell".into(),
        ));
    };
    let live_fence = match side {
        DraftGridTransferAbortSideV2::Source => authority.live_source_fencing_token,
        DraftGridTransferAbortSideV2::Destination => authority.live_destination_fencing_token,
    };
    if state.base.fencing_token != live_fence
        || !matches!(
            authority.phase,
            TransferPhase::Aborting | TransferPhase::Aborted
        )
    {
        return Err(DraftGridClosureError::Invalid(
            "abort reconciliation lacks current terminal cell authority".into(),
        ));
    }
    let witness = state
        .abort_witnesses
        .get(&package.transfer_id)
        .ok_or_else(|| {
            DraftGridClosureError::Changed(
                "abort reconciliation cannot find its canonical cleanup witness".into(),
            )
        })?;
    witness.validate_request(package, authority, side)?;
    let proof = witness.cleanup_proof();
    proof.validate().map_err(DraftGridClosureError::Invalid)?;
    let directory_proof = match side {
        DraftGridTransferAbortSideV2::Source => authority.source_abort_proof.as_ref(),
        DraftGridTransferAbortSideV2::Destination => authority.destination_abort_proof.as_ref(),
    };
    if directory_proof.is_some_and(|directory_proof| directory_proof != &proof) {
        return Err(DraftGridClosureError::Changed(
            "abort reconciliation conflicts with the directory's canonical cleanup proof".into(),
        ));
    }
    Ok((state.clone(), proof))
}

#[cfg(test)]
fn stage_aborted_grid_cleanup_v2(
    state: &DraftGridTransferCellStateV2,
    trusted_now_unix_ms: u64,
    package: &DraftGridClosurePackageV2,
    authority: &DraftGridDirectoryAuthorityV2,
) -> Result<(DraftGridTransferCellStateV2, DraftGridAbortCleanupProofV2), DraftGridClosureError> {
    state.validate()?;
    package.validate_wire()?;
    authority.validate_package(package)?;
    if state.abort_witnesses.contains_key(&package.transfer_id) {
        return reconcile_aborted_grid_cleanup_v2(state, package, authority);
    }
    let side = if state.base.cell_id == package.source_cell_id {
        DraftGridTransferAbortSideV2::Source
    } else if state.base.cell_id == package.destination_cell_id {
        DraftGridTransferAbortSideV2::Destination
    } else {
        return Err(DraftGridClosureError::Invalid(
            "abort cleanup was presented to an unrelated cell".into(),
        ));
    };
    let event = DraftCanonicalGridEventV17::new_system(
        state,
        format!("abort-{}-{side:?}", package.transfer_id).to_ascii_lowercase(),
        trusted_now_unix_ms,
        DraftGridEventPayloadV17::GridTransferAborted {
            package: package.clone(),
            authority: authority.clone(),
            side,
        },
    )?;
    let context = event.validate_for_state(state)?;
    stage_aborted_grid_cleanup_event_v17(state, package, authority, &context)
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
    use std::collections::VecDeque;

    use super::super::tests::package_fixture;
    use super::*;
    use crate::content;
    use crate::model::{Block, ProductionJob, production_recipe_quantities};
    use verse_protocol::{BlockKind, ProductionRecipeKind};

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
        quarantine_authority.record_test_source_prepare(&locked, &package.transfer_id);
        let (reserved, _) = stage_grid_quarantine_v2(
            &destination_state(package),
            1_800_000_000_000,
            package,
            &quarantine_authority,
        )
        .expect("destination quarantine derives receipt");
        let mut committed = quarantine_authority;
        committed.phase = TransferPhase::Committed;
        committed.record_test_destination_quarantine(&reserved, &package.transfer_id);
        (locked, committed)
    }

    fn production_package_fixture() -> (WorldState, DraftGridClosurePackageV2) {
        let (mut source, context, _) = package_fixture();
        let grid = source
            .grids
            .get_mut(&context.placement.root_aggregate_id)
            .expect("fixture grid exists");
        let prior_component_cost = grid.blocks["block-core"].component_cost;
        let replacement = Block::new(
            "block-core",
            grid.blocks["block-core"].coordinate,
            BlockKind::Refinery,
        );
        let replacement_component_cost = replacement.component_cost;
        grid.blocks.insert("block-core".into(), replacement);
        source.ledger.genesis_installed_components = source
            .ledger
            .genesis_installed_components
            .checked_sub(prior_component_cost)
            .and_then(|value| value.checked_add(replacement_component_cost))
            .expect("fixture installed components remain bounded");
        source.event_sequence = 1;
        source.last_event_hash = "11".repeat(32);
        let (reserved_inputs, _, duration_ticks) =
            production_recipe_quantities(ProductionRecipeKind::Refining, 1)
                .expect("refining recipe derives");
        source.ledger.genesis_ore = source
            .ledger
            .genesis_ore
            .checked_add(reserved_inputs.ore)
            .expect("fixture ore remains bounded");
        let cargo_inventory_id =
            source.grids[&context.placement.root_aggregate_id].blocks["block-cargo"]
                .inventory_id
                .clone()
                .expect("fixture cargo inventory exists");
        let (job_id, job_origin) = DraftProductionJobOriginV2::new(
            &source.universe_id,
            &source.cell_id,
            source.event_sequence,
            0,
        )
        .expect("canonical job origin derives");
        source.production_queues.insert(
            "block-core".into(),
            VecDeque::from([ProductionJob {
                job_id: job_id.clone(),
                operation_id: "operation-import-restart".into(),
                owner_player_id: "player-local".into(),
                machine_block_id: "block-core".into(),
                recipe: ProductionRecipeKind::Refining,
                content_manifest_version: source.content_manifest_version.clone(),
                batches: 1,
                source_inventory_id: cargo_inventory_id.clone(),
                destination_inventory_id: cargo_inventory_id,
                progress_ticks: duration_ticks
                    .checked_sub(u64::from(content::manifest().physics.fixed_step_hz))
                    .expect("fixture job has at least one production quantum remaining"),
                duration_ticks,
                reserved_inputs,
                pending_outputs: InventoryContents::default(),
                queued_event_sequence: source.event_sequence,
            }]),
        );
        let origins = BTreeMap::from([(job_id, job_origin)]);
        let authoritative =
            DraftGridTransferCellStateV2::new_with_production_origins(source.clone(), origins)
                .expect("production source state seals");
        let package = authoritative
            .capture_grid_closure(&context.placement.root_aggregate_id, &context)
            .expect("production package captures");
        (source, package)
    }

    fn synthetic_import_proof(
        package: &DraftGridClosurePackageV2,
        source_export: &DraftGridExportProofV2,
    ) -> DraftGridImportProofV2 {
        let mut proof = DraftGridImportProofV2 {
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            member_root: package.member_root.clone(),
            package_hash: package.package_hash.clone(),
            destination_cell_id: package.destination_cell_id.clone(),
            assignment_generation: package.destination_assignment_generation,
            fencing_token: package.destination_fencing_token,
            prior_event_sequence: 7,
            prior_event_hash: blake3::hash(b"synthetic import prior event")
                .to_hex()
                .to_string(),
            event_sequence: 8,
            event_hash: String::new(),
            prior_draft_world_hash: blake3::hash(b"synthetic import prior draft world")
                .to_hex()
                .to_string(),
            resulting_active_world_hash: blake3::hash(b"synthetic import active world")
                .to_hex()
                .to_string(),
            quarantine_receipt_hash: source_export.quarantine_receipt_hash.clone(),
            quarantined_at_unix_ms: source_export.exported_at_unix_ms - 1,
            source_export_proof_hash: source_export.proof_hash.clone(),
            source_exported_at_unix_ms: source_export.exported_at_unix_ms,
            imported_at_unix_ms: source_export.exported_at_unix_ms + 1,
            destination_production_lifecycle_generation: 1,
            production_eligibility_root: blake3::hash(b"synthetic production eligibility")
                .to_hex()
                .to_string(),
            mutation_witness_hash: blake3::hash(b"synthetic import mutation")
                .to_hex()
                .to_string(),
            proof_hash: String::new(),
            ledger_vector: DraftGridTransferLedgerVectorV2::from_package(package)
                .expect("synthetic import ledger derives"),
        };
        proof
            .seal_hashes_for_test()
            .expect("synthetic import proof seals");
        proof
    }

    fn synthetic_activation_proof(
        package: &DraftGridClosurePackageV2,
        import: &DraftGridImportProofV2,
    ) -> DraftGridActivationProofV2 {
        let mut proof = DraftGridActivationProofV2 {
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            member_root: package.member_root.clone(),
            package_hash: package.package_hash.clone(),
            destination_cell_id: package.destination_cell_id.clone(),
            assignment_generation: package.destination_assignment_generation,
            fencing_token: package.destination_fencing_token,
            prior_event_sequence: import.event_sequence,
            prior_event_hash: import.event_hash.clone(),
            event_sequence: import.event_sequence + 1,
            event_hash: String::new(),
            prior_active_world_hash: import.resulting_active_world_hash.clone(),
            resulting_active_world_hash: blake3::hash(b"synthetic activation active world")
                .to_hex()
                .to_string(),
            quarantine_receipt_hash: import.quarantine_receipt_hash.clone(),
            destination_import_proof_hash: import.proof_hash.clone(),
            imported_at_unix_ms: import.imported_at_unix_ms,
            activated_at_unix_ms: import.imported_at_unix_ms + 1,
            production_eligibility_root: import.production_eligibility_root.clone(),
            mutation_witness_hash: blake3::hash(b"synthetic activation mutation")
                .to_hex()
                .to_string(),
            proof_hash: String::new(),
        };
        proof
            .seal_hashes_for_test()
            .expect("synthetic activation proof seals");
        proof
    }

    fn synthetic_finalization_proof(
        package: &DraftGridClosurePackageV2,
        export: &DraftGridExportProofV2,
        import: &DraftGridImportProofV2,
        activation: &DraftGridActivationProofV2,
    ) -> DraftGridFinalizationProofV2 {
        let mut proof = DraftGridFinalizationProofV2 {
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            member_root: package.member_root.clone(),
            package_hash: package.package_hash.clone(),
            source_cell_id: package.source_cell_id.clone(),
            assignment_generation: package.source_assignment_generation,
            fencing_token: package.source_fencing_token,
            prior_event_sequence: export.event_sequence,
            prior_event_hash: export.event_hash.clone(),
            event_sequence: export.event_sequence + 1,
            event_hash: String::new(),
            prior_active_world_hash: export.resulting_active_world_hash.clone(),
            resulting_active_world_hash: blake3::hash(b"synthetic finalization active world")
                .to_hex()
                .to_string(),
            source_export_proof_hash: export.proof_hash.clone(),
            source_exported_at_unix_ms: export.exported_at_unix_ms,
            destination_import_proof_hash: import.proof_hash.clone(),
            imported_at_unix_ms: import.imported_at_unix_ms,
            destination_activation_proof_hash: activation.proof_hash.clone(),
            activated_at_unix_ms: activation.activated_at_unix_ms,
            finalized_at_unix_ms: activation.activated_at_unix_ms + 1,
            mutation_witness_hash: blake3::hash(b"synthetic finalization mutation")
                .to_hex()
                .to_string(),
            proof_hash: String::new(),
        };
        proof
            .seal_hashes_for_test()
            .expect("synthetic finalization proof seals");
        proof
    }

    fn source_finalization_fixture() -> (
        DraftGridTransferCellStateV2,
        DraftGridClosurePackageV2,
        DraftGridDirectoryAuthorityV2,
    ) {
        let (source, package) = production_package_fixture();
        let source_state = DraftGridTransferCellStateV2::new_with_production_origins(
            source,
            package.production_job_origins.clone(),
        )
        .expect("source seals");
        let prepared =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked = stage_prepared_grid_lock_v2(&source_state, &package, &prepared)
            .expect("source closure locks");
        let destination = destination_state(&package);
        let mut authority = prepared;
        authority.record_test_source_prepare(&locked, &package.transfer_id);
        let (reserved, _) =
            stage_grid_quarantine_v2(&destination, 1_800_000_000_000, &package, &authority)
                .expect("destination reservation seals");
        authority.phase = TransferPhase::Committed;
        authority.record_test_destination_quarantine(&reserved, &package.transfer_id);
        let (exported, export_proof) =
            stage_committed_grid_export_v2(&locked, 1_800_000_010_000, &package, &authority)
                .expect("source export commits");
        authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceExport);
        authority.source_export_proof_hash = Some(export_proof.proof_hash.clone());
        authority.source_exported_at_unix_ms = Some(export_proof.exported_at_unix_ms);
        authority.source_export_proof = Some(export_proof);
        let (imported, import_proof) =
            stage_committed_grid_import_v2(&reserved, 1_800_000_020_000, &package, &authority)
                .expect("destination import commits");
        authority.phase = TransferPhase::Imported;
        authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationImport);
        authority.destination_import_proof = Some(import_proof);
        let (_, activation_proof) =
            stage_imported_grid_activation_v2(&imported, 1_800_000_020_001, &package, &authority)
                .expect("destination activation commits");
        authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationActivation);
        authority.destination_activation_proof = Some(activation_proof);
        (exported, package, authority)
    }

    fn import_record_fixture() -> (
        DraftGridTransferCellStateV2,
        DraftGridClosurePackageV2,
        DraftPendingGridImportV2,
        DraftGridImportRecordV2,
        DraftGridDirectoryAuthorityV2,
    ) {
        let (source, package) = production_package_fixture();
        let source_state = DraftGridTransferCellStateV2::new_with_production_origins(
            source,
            package.production_job_origins.clone(),
        )
        .expect("source seals");
        let prepared =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked = stage_prepared_grid_lock_v2(&source_state, &package, &prepared)
            .expect("source closure locks");
        let destination = destination_state(&package);
        let mut quarantine_authority = prepared.clone();
        quarantine_authority.record_test_source_prepare(&locked, &package.transfer_id);
        let (reserved, _) = stage_grid_quarantine_v2(
            &destination,
            1_800_000_000_000,
            &package,
            &quarantine_authority,
        )
        .expect("destination reservation seals");
        let mut committed = quarantine_authority;
        committed.phase = TransferPhase::Committed;
        committed.record_test_destination_quarantine(&reserved, &package.transfer_id);
        let (_, source_export_proof) =
            stage_committed_grid_export_v2(&locked, 1_800_000_010_000, &package, &committed)
                .expect("source export proof seals");
        committed
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceExport);
        committed.source_export_proof_hash = Some(source_export_proof.proof_hash.clone());
        committed.source_exported_at_unix_ms = Some(source_export_proof.exported_at_unix_ms);
        committed.source_export_proof = Some(source_export_proof.clone());
        let ledger_vector =
            DraftGridTransferLedgerVectorV2::from_package(&package).expect("ledger vector derives");
        let mut pending = DraftPendingGridImportV2 {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            reservation: reserved.aggregate_reservations[&package.transfer_id].clone(),
            source_export_proof,
            destination_assignment_generation: package.destination_assignment_generation,
            historical_fencing_token: package.destination_fencing_token,
            live_fencing_token: package.destination_fencing_token,
            prior_event_sequence: reserved.base.event_sequence,
            prior_event_hash: reserved.base.last_event_hash.clone(),
            import_event_sequence: reserved
                .base
                .event_sequence
                .checked_add(1)
                .expect("event frontier advances"),
            import_event_hash: String::new(),
            prior_draft_world_hash: reserved.state_hash.clone(),
            ledger_vector,
            conservation_witness: grid_transfer_witness(
                &package,
                TransferWitnessDirection::Import,
                ledger_vector,
            ),
            production_eligibility_root: imported_production_eligibility_map_root(&BTreeMap::new())
                .expect("empty eligibility root hashes"),
            destination_production_lifecycle_generation: reserved
                .base
                .production_clock
                .lifecycle_generation,
            imported_at_unix_ms: 1_800_000_020_000,
            mutation_witness_hash: String::new(),
        };
        pending.mutation_witness_hash = pending
            .calculate_mutation_hash()
            .expect("import mutation hashes");
        pending.import_event_hash = pending.calculate_event_hash().expect("import event hashes");
        pending.validate().expect("pending import validates");
        let mut record = DraftGridImportRecordV2 {
            schema_version: DRAFT_GRID_TRANSFER_RECEIPT_SCHEMA_VERSION,
            pending: pending.clone(),
            resulting_active_world_hash: "ab".repeat(32),
            proof_hash: String::new(),
            record_hash: String::new(),
        };
        record.proof_hash = record
            .proof()
            .calculate_hash()
            .expect("import proof hashes");
        record.record_hash = record.calculate_hash().expect("import record hashes");
        record.validate().expect("import record validates");
        (reserved, package, pending, record, committed)
    }

    fn materialized_import_fixture() -> (
        DraftGridTransferCellStateV2,
        DraftGridClosurePackageV2,
        DraftGridDirectoryAuthorityV2,
    ) {
        let (reserved, package, _, _, mut authority) = import_record_fixture();
        let (imported, proof) =
            stage_committed_grid_import_v2(&reserved, 1_800_000_020_000, &package, &authority)
                .expect("exact destination import commits");
        proof.validate().expect("import proof validates");
        authority.phase = TransferPhase::Imported;
        authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationImport);
        authority.destination_import_proof = Some(proof);
        (imported, package, authority)
    }

    fn materialized_import_state_fixture() -> DraftGridTransferCellStateV2 {
        materialized_import_fixture().0
    }

    fn next_occurrence(
        state: &DraftGridTransferCellStateV2,
        scheduled_for_unix_ms: u64,
    ) -> crate::event::ProductionScheduleOccurrence {
        crate::event::ProductionScheduleOccurrence {
            schema_version: crate::event::PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            universe_id: state.base.universe_id.clone(),
            cell_id: state.base.cell_id.clone(),
            lifecycle_generation: state.base.production_clock.lifecycle_generation,
            production_quantum_sequence: state
                .base
                .production_clock
                .last_committed_quantum_sequence
                .checked_add(1)
                .expect("fixture occurrence sequence advances"),
            scheduled_for_unix_ms,
            universe_manifest_hash: state.base.universe_manifest_hash.clone(),
            celestial_registry_hash: state.base.celestial_registry_hash.clone(),
        }
    }

    #[test]
    fn dormant_import_models_are_restart_verifiable_and_acyclic() {
        let (reserved, package, pending, record, _) = import_record_fixture();
        let proof = record.proof();
        proof.validate().expect("typed import proof validates");
        assert_eq!(proof.transfer_id, package.transfer_id);
        assert_eq!(proof.ledger_vector, pending.ledger_vector);

        let bytes = serde_json::to_vec(&record).expect("import record encodes");
        let decoded = serde_json::from_slice::<DraftGridImportRecordV2>(&bytes)
            .expect("import record decodes");
        assert_eq!(decoded, record);
        let mut unknown = serde_json::to_value(&record).expect("record becomes JSON");
        unknown
            .as_object_mut()
            .expect("record JSON is an object")
            .insert("unknown".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<DraftGridImportRecordV2>(unknown).is_err());

        let mut changed_result = record.clone();
        changed_result.resulting_active_world_hash = "cd".repeat(32);
        let changed_proof = changed_result.proof();
        assert_eq!(changed_proof.event_hash, proof.event_hash);
        assert_ne!(
            changed_proof
                .calculate_hash()
                .expect("changed proof hashes"),
            proof.proof_hash
        );

        let mut changed_eligibility = pending.clone();
        changed_eligibility.production_eligibility_root = "ef".repeat(32);
        assert_eq!(
            changed_eligibility
                .calculate_event_hash()
                .expect("post-event eligibility root does not rehash the event"),
            pending.import_event_hash
        );
        let mut substituted_source = pending.clone();
        substituted_source.source_export_proof.proof_hash = "12".repeat(32);
        substituted_source.mutation_witness_hash = substituted_source
            .calculate_mutation_hash()
            .expect("outer mutation can be resealed");
        substituted_source.import_event_hash = substituted_source
            .calculate_event_hash()
            .expect("outer event can be resealed");
        assert!(substituted_source.validate().is_err());
        let mut downgraded_source = pending.clone();
        downgraded_source.source_export_proof.assignment_generation = 1;
        downgraded_source
            .source_export_proof
            .seal_hashes_for_test()
            .expect("standalone downgraded source proof reseals");
        downgraded_source.mutation_witness_hash = downgraded_source
            .calculate_mutation_hash()
            .expect("downgraded outer mutation reseals");
        downgraded_source.import_event_hash = downgraded_source
            .calculate_event_hash()
            .expect("downgraded outer event reseals");
        assert!(downgraded_source.validate().is_err());
        let mut early_import = pending.clone();
        early_import.imported_at_unix_ms = early_import
            .source_export_proof
            .exported_at_unix_ms
            .checked_sub(1)
            .expect("fixture export time is positive");
        early_import.mutation_witness_hash = early_import
            .calculate_mutation_hash()
            .expect("early mutation hashes");
        early_import.import_event_hash = early_import
            .calculate_event_hash()
            .expect("early event hashes");
        assert!(early_import.validate().is_err());

        let empty_active_hash = reserved
            .calculate_active_world_hash()
            .expect("empty destination active hash derives");
        let mut with_pending = reserved.clone();
        with_pending
            .pending_imports
            .insert(package.transfer_id.clone(), pending);
        assert_ne!(
            with_pending
                .calculate_active_world_hash()
                .expect("pending active hash derives"),
            empty_active_hash
        );
        let mut with_history = reserved.clone();
        let prior_full_hash = with_history
            .calculate_hash()
            .expect("prior full hash derives");
        with_history
            .committed_imports
            .insert(package.transfer_id.clone(), record);
        assert_eq!(
            with_history
                .calculate_active_world_hash()
                .expect("historical active hash derives"),
            empty_active_hash
        );
        assert_ne!(
            with_history
                .calculate_hash()
                .expect("historical full hash derives"),
            prior_full_hash
        );
    }

    #[test]
    fn dormant_materialized_import_restart_keeps_activation_and_production_holds() {
        let state = materialized_import_state_fixture();
        let bytes = state.encode_canonical().expect("imported state encodes");
        assert_eq!(
            DraftGridTransferCellStateV2::decode_canonical(&bytes)
                .expect("imported state restarts"),
            state
        );
        let transfer_id = state
            .committed_imports
            .keys()
            .next()
            .expect("import exists")
            .clone();

        let mut missing_pending = state.clone();
        missing_pending.pending_imports.remove(&transfer_id);
        missing_pending.base.event_sequence += 1;
        missing_pending.base.last_event_hash = "45".repeat(32);
        assert!(missing_pending.seal().is_err());

        let mut missing_eligibility = state.clone();
        missing_eligibility
            .imported_production_eligibilities
            .pop_first();
        assert!(missing_eligibility.seal().is_err());

        let mut backdated = state;
        let (machine_id, eligibility) = backdated
            .imported_production_eligibilities
            .first_key_value()
            .map(|(machine_id, eligibility)| (machine_id.clone(), eligibility.clone()))
            .expect("fixture carries a machine eligibility");
        let backdated_eligibility =
            eligibility.resealed_with_trusted_import_unix_ms_for_test(1_800_000_019_999);
        backdated
            .imported_production_eligibilities
            .insert(machine_id, backdated_eligibility);
        let pending = backdated
            .pending_imports
            .get_mut(&transfer_id)
            .expect("pending import exists");
        pending.production_eligibility_root =
            imported_production_eligibility_map_root(&backdated.imported_production_eligibilities)
                .expect("backdated map root reseals");
        let import = backdated
            .committed_imports
            .get_mut(&transfer_id)
            .expect("import record exists");
        import.pending = pending.clone();
        import.proof_hash = import.proof().calculate_hash().expect("proof reseals");
        import.record_hash = import.calculate_hash().expect("record reseals");
        assert!(backdated.seal().is_err());
    }

    #[test]
    fn source_finalization_is_one_audit_only_event_with_exact_crash_retries() {
        let (exported, package, authority) = source_finalization_fixture();
        let prior_base = exported.base.clone();
        let prior_origins = exported.production_job_origins.clone();
        let export = exported.committed_exports[&package.transfer_id].clone();
        let activation = authority
            .destination_activation_proof
            .as_ref()
            .expect("fixture has destination activation proof");

        let mut before_activation = authority.clone();
        before_activation
            .proofs
            .remove(&DraftGridDirectoryProofKindV2::DestinationActivation);
        before_activation.destination_activation_proof = None;
        assert!(
            stage_finalized_grid_source_v2(
                &exported,
                activation.activated_at_unix_ms,
                &package,
                &before_activation,
            )
            .is_err()
        );
        assert!(
            stage_finalized_grid_source_v2(
                &exported,
                activation.activated_at_unix_ms - 1,
                &package,
                &authority,
            )
            .is_err()
        );

        let (finalized, proof) = stage_finalized_grid_source_v2(
            &exported,
            activation.activated_at_unix_ms,
            &package,
            &authority,
        )
        .expect("source finalization commits after destination activation");
        proof
            .validate()
            .expect("source finalization proof validates");
        assert_eq!(finalized.base.event_sequence, prior_base.event_sequence + 1);
        assert_eq!(finalized.base.last_event_hash, proof.event_hash);
        let mut expected_base = prior_base;
        expected_base.event_sequence = finalized.base.event_sequence;
        expected_base.last_event_hash.clone_from(&proof.event_hash);
        assert_eq!(finalized.base, expected_base);
        assert_eq!(finalized.production_job_origins, prior_origins);
        assert_eq!(finalized.committed_exports[&package.transfer_id], export);
        assert_eq!(finalized.source_finalization_tombstones.len(), 1);
        assert_eq!(finalized.committed_finalizations.len(), 1);
        assert_eq!(
            DraftGridTransferCellStateV2::decode_canonical(
                &finalized
                    .encode_canonical()
                    .expect("finalized state encodes")
            )
            .expect("finalized state restarts"),
            finalized
        );

        let (cell_first_retry, cell_first_proof) = stage_finalized_grid_source_v2(
            &finalized,
            activation.activated_at_unix_ms + 10_000,
            &package,
            &authority,
        )
        .expect("source-committed directory-pending retry is exact");
        assert_eq!(cell_first_retry, finalized);
        assert_eq!(cell_first_proof, proof);

        let mut directory_finalized = authority.clone();
        directory_finalized.phase = TransferPhase::Finalized;
        directory_finalized
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceFinalization);
        directory_finalized.source_finalization_proof = Some(proof.clone());
        let (directory_retry, directory_retry_proof) = stage_finalized_grid_source_v2(
            &finalized,
            activation.activated_at_unix_ms + 20_000,
            &package,
            &directory_finalized,
        )
        .expect("directory-finalized retry is exact");
        assert_eq!(directory_retry, finalized);
        assert_eq!(directory_retry_proof, proof);
        assert!(
            stage_finalized_grid_source_v2(
                &exported,
                activation.activated_at_unix_ms + 20_000,
                &package,
                &directory_finalized,
            )
            .is_err(),
            "directory-finalized state cannot synthesize a missing local event"
        );

        let mut missing_tombstone = finalized.clone();
        missing_tombstone
            .source_finalization_tombstones
            .remove(&package.transfer_id);
        assert!(missing_tombstone.seal().is_err());
        let mut missing_history = finalized.clone();
        missing_history
            .committed_finalizations
            .remove(&package.transfer_id);
        assert!(missing_history.seal().is_err());

        let mut changed_activation_body = finalized.clone();
        changed_activation_body
            .committed_finalizations
            .get_mut(&package.transfer_id)
            .expect("finalization history exists")
            .destination_activation_proof
            .destination_import_proof_hash = blake3::hash(b"substituted destination import proof")
            .to_hex()
            .to_string();
        assert!(
            changed_activation_body.seal().is_err(),
            "restart rejects a changed retained activation proof body"
        );

        let mut substituted_dependency_chain = finalized.clone();
        let mut changed_record = substituted_dependency_chain
            .committed_finalizations
            .remove(&package.transfer_id)
            .expect("finalization history exists");
        let changed_destination = blake3::hash(b"substituted destination cell")
            .to_hex()
            .to_string();
        changed_record
            .destination_import_proof
            .destination_cell_id
            .clone_from(&changed_destination);
        changed_record
            .destination_import_proof
            .seal_hashes_for_test()
            .expect("substituted import proof reseals");
        changed_record
            .destination_activation_proof
            .destination_cell_id
            .clone_from(&changed_destination);
        changed_record
            .destination_activation_proof
            .destination_import_proof_hash
            .clone_from(&changed_record.destination_import_proof.proof_hash);
        if changed_record
            .destination_activation_proof
            .prior_event_sequence
            == changed_record.destination_import_proof.event_sequence
        {
            changed_record
                .destination_activation_proof
                .prior_event_hash
                .clone_from(&changed_record.destination_import_proof.event_hash);
        }
        changed_record
            .destination_activation_proof
            .seal_hashes_for_test()
            .expect("substituted activation proof reseals");
        changed_record
            .destination_import_proof_hash
            .clone_from(&changed_record.destination_import_proof.proof_hash);
        changed_record
            .destination_activation_proof_hash
            .clone_from(&changed_record.destination_activation_proof.proof_hash);
        changed_record.mutation_witness_hash = changed_record
            .calculate_mutation_hash()
            .expect("substituted finalization mutation reseals");
        changed_record.finalization_event_hash = changed_record
            .proof()
            .calculate_event_hash()
            .expect("substituted finalization event reseals");
        substituted_dependency_chain
            .base
            .last_event_hash
            .clone_from(&changed_record.finalization_event_hash);
        substituted_dependency_chain
            .source_finalization_tombstones
            .insert(package.transfer_id.clone(), changed_record.tombstone());
        changed_record.resulting_active_world_hash = substituted_dependency_chain
            .calculate_active_world_hash()
            .expect("substituted active world reseals");
        changed_record.proof_hash = changed_record
            .proof()
            .calculate_hash()
            .expect("substituted finalization proof reseals");
        changed_record.record_hash = changed_record
            .calculate_hash()
            .expect("substituted finalization history reseals");
        substituted_dependency_chain
            .committed_finalizations
            .insert(package.transfer_id.clone(), changed_record);
        substituted_dependency_chain.state_hash = substituted_dependency_chain
            .calculate_hash()
            .expect("substituted state envelope reseals");
        assert!(
            substituted_dependency_chain.validate().is_err(),
            "restart rejects a fully resealed proof chain for another destination"
        );

        let mut successor = finalized.clone();
        successor.base.fencing_token += 1;
        successor.base.event_sequence += 1;
        successor.base.last_event_hash = blake3::hash(b"source finalization successor event")
            .to_hex()
            .to_string();
        successor.seal().expect("successor source state seals");
        directory_finalized.advance_test_source_authority();
        let (successor_retry, successor_proof) = stage_finalized_grid_source_v2(
            &successor,
            activation.activated_at_unix_ms + 30_000,
            &package,
            &directory_finalized,
        )
        .expect("successor retries the exact historical finalization");
        assert_eq!(successor_retry, successor);
        assert_eq!(successor_proof, proof);
    }

    #[test]
    fn imported_activation_releases_only_gameplay_lock_and_retries_through_finalization() {
        let (imported, package, authority) = materialized_import_fixture();
        let prior_clock = imported.base.production_clock.clone();
        let prior_queues = imported.base.production_queues.clone();
        let prior_grid = imported.base.grids[&package.root_aggregate_id].clone();
        assert_eq!(
            imported.locked_transfer_for_subject(&package.root_aggregate_id),
            Some(package.transfer_id.as_str())
        );

        let (activated, proof) =
            stage_imported_grid_activation_v2(&imported, 1_800_000_020_001, &package, &authority)
                .expect("destination activation commits");
        proof.validate().expect("activation proof validates");
        assert!(activated.pending_imports.is_empty());
        assert_eq!(activated.committed_activations.len(), 1);
        assert_eq!(
            activated.base.event_sequence,
            imported.base.event_sequence + 1
        );
        assert_eq!(activated.base.last_event_hash, proof.event_hash);
        assert_eq!(activated.base.production_clock, prior_clock);
        assert_eq!(activated.base.production_queues, prior_queues);
        assert_eq!(activated.base.grids[&package.root_aggregate_id], prior_grid);
        assert_eq!(
            activated.imported_production_eligibilities,
            imported.imported_production_eligibilities
        );
        assert_eq!(
            activated.locked_transfer_for_subject(&package.root_aggregate_id),
            None
        );
        assert_eq!(
            DraftGridTransferCellStateV2::decode_canonical(
                &activated
                    .encode_canonical()
                    .expect("activated state encodes"),
            )
            .expect("activated state restarts"),
            activated
        );

        let (cell_first_retry, cell_first_proof) =
            stage_imported_grid_activation_v2(&activated, 1_800_000_020_002, &package, &authority)
                .expect("cell-first activation retry is exact");
        assert_eq!(cell_first_retry, activated);
        assert_eq!(cell_first_proof, proof);

        let mut directory_proven = authority.clone();
        directory_proven
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationActivation);
        directory_proven.destination_activation_proof = Some(proof.clone());
        let (directory_retry, directory_retry_proof) = stage_imported_grid_activation_v2(
            &activated,
            1_800_000_020_003,
            &package,
            &directory_proven,
        )
        .expect("directory-proven activation retry is exact");
        assert_eq!(directory_retry, activated);
        assert_eq!(directory_retry_proof, proof);

        let synthetic_finalization = synthetic_finalization_proof(
            &package,
            directory_proven
                .source_export_proof
                .as_ref()
                .expect("directory retains export proof"),
            directory_proven
                .destination_import_proof
                .as_ref()
                .expect("directory retains import proof"),
            &proof,
        );
        let mut finalized = directory_proven.clone();
        finalized.phase = TransferPhase::Finalized;
        finalized
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceFinalization);
        finalized.source_finalization_proof = Some(synthetic_finalization);
        let (finalized_retry, finalized_retry_proof) =
            stage_imported_grid_activation_v2(&activated, 1_800_000_020_004, &package, &finalized)
                .expect("finalized activation retry is exact");
        assert_eq!(finalized_retry, activated);
        assert_eq!(finalized_retry_proof, proof);

        let mut changed_directory = directory_proven;
        changed_directory
            .destination_activation_proof
            .as_mut()
            .expect("activation proof exists")
            .resulting_active_world_hash = "ab".repeat(32);
        assert!(
            stage_imported_grid_activation_v2(
                &activated,
                1_800_000_020_005,
                &package,
                &changed_directory,
            )
            .is_err()
        );

        let mut queue_body_tamper = activated.clone();
        queue_body_tamper
            .base
            .production_queues
            .values_mut()
            .next()
            .and_then(|queue| queue.front_mut())
            .expect("imported production job exists")
            .progress_ticks += 1;
        assert!(queue_body_tamper.seal().is_err());

        let mut prior_hash_tamper = activated.clone();
        let mut activation = prior_hash_tamper
            .committed_activations
            .remove(&package.transfer_id)
            .expect("activation record exists");
        activation.prior_active_world_hash = "ac".repeat(32);
        activation.mutation_witness_hash = activation
            .calculate_mutation_hash()
            .expect("tampered mutation reseals");
        activation.activation_event_hash = activation
            .proof()
            .calculate_event_hash()
            .expect("tampered event reseals");
        prior_hash_tamper
            .base
            .last_event_hash
            .clone_from(&activation.activation_event_hash);
        activation.resulting_active_world_hash = prior_hash_tamper
            .calculate_active_world_hash()
            .expect("tampered result derives");
        activation.proof_hash = activation
            .proof()
            .calculate_hash()
            .expect("tampered proof reseals");
        activation.record_hash = activation
            .calculate_hash()
            .expect("tampered record reseals");
        prior_hash_tamper
            .committed_activations
            .insert(package.transfer_id.clone(), activation);
        assert!(prior_hash_tamper.seal().is_err());

        let mut forked_frontier = activated.clone();
        let mut activation = forked_frontier
            .committed_activations
            .remove(&package.transfer_id)
            .expect("activation record exists");
        activation.prior_event_hash = blake3::hash(b"forked import predecessor")
            .to_hex()
            .to_string();
        let mut claimed_prior = forked_frontier.clone();
        claimed_prior
            .pending_imports
            .insert(package.transfer_id.clone(), activation.pending.clone());
        claimed_prior.base.event_sequence = activation.prior_event_sequence;
        claimed_prior
            .base
            .last_event_hash
            .clone_from(&activation.prior_event_hash);
        activation.prior_active_world_hash = claimed_prior
            .calculate_active_world_hash()
            .expect("forked prior world derives");
        activation.mutation_witness_hash = activation
            .calculate_mutation_hash()
            .expect("forked mutation reseals");
        activation.activation_event_hash = activation
            .proof()
            .calculate_event_hash()
            .expect("forked event reseals");
        forked_frontier
            .base
            .last_event_hash
            .clone_from(&activation.activation_event_hash);
        activation.resulting_active_world_hash = forked_frontier
            .calculate_active_world_hash()
            .expect("forked result derives");
        activation.proof_hash = activation
            .proof()
            .calculate_hash()
            .expect("forked proof reseals");
        activation.record_hash = activation.calculate_hash().expect("forked record reseals");
        forked_frontier
            .committed_activations
            .insert(package.transfer_id.clone(), activation);
        assert!(forked_frontier.seal().is_err());

        let next_cell = celestial::neighbor_cell_key(&package.destination_cell_key, [1, 0, 0])
            .expect("next destination cell derives");
        let next_members = package
            .members
            .iter()
            .map(|member| BundledPlacementMember {
                aggregate_id: member.aggregate_id.clone(),
                aggregate_kind: member.aggregate_kind,
                prior_placement_generation: member.resulting_placement_generation,
                resulting_placement_generation: member
                    .resulting_placement_generation
                    .checked_add(1)
                    .expect("next placement generation advances"),
            })
            .collect();
        let next_plan = BundledPlacementPlan::new(
            package.root_aggregate_id.clone(),
            package.destination_cell_key.clone(),
            next_cell.clone(),
            next_members,
        )
        .expect("next placement plan derives");
        let current_origin = celestial::cell_address_from_key(&package.destination_cell_key)
            .expect("current destination origin derives");
        let crossed_address =
            celestial::cell_address_from_key(&next_cell).expect("crossed address derives");
        let crossed_local_position =
            celestial::local_position_from_address(&current_origin, &crossed_address)
                .expect("crossed local position derives");
        let mut transfer_ready = activated.clone();
        transfer_ready.base.event_sequence += 1;
        transfer_ready.base.last_event_hash = blake3::hash(b"later transfer boundary crossing")
            .to_hex()
            .to_string();
        let grid = transfer_ready
            .base
            .grids
            .get_mut(&package.root_aggregate_id)
            .expect("activated grid exists");
        grid.address = crossed_address.clone();
        grid.position = crossed_local_position;
        for player_id in package.players.keys() {
            let player = transfer_ready
                .base
                .player
                .get_mut(player_id)
                .expect("activated rider exists");
            player.address = crossed_address.clone();
            player.position = crossed_local_position;
        }
        transfer_ready
            .seal()
            .expect("activated closure may reach a later transfer boundary");
        let next_context = DraftGridTransferContextV2 {
            transfer_id: "transfer-grid-v2-next".into(),
            source_assignment_generation: package.destination_assignment_generation,
            destination_assignment_generation: package.destination_assignment_generation + 1,
            source_fencing_token: transfer_ready.base.fencing_token,
            destination_fencing_token: transfer_ready.base.fencing_token + 1,
            placement: next_plan,
            production_job_origins: BTreeMap::new(),
        };
        assert!(
            transfer_ready
                .capture_grid_closure(&package.root_aggregate_id, &next_context)
                .is_err(),
            "a destination-bound production hold cannot leak into another handoff"
        );
        let eligible_at = transfer_ready
            .imported_production_eligibilities
            .values()
            .next()
            .expect("activated fixture still has a production hold")
            .eligible_at_unix_ms();
        let occurrence = next_occurrence(&transfer_ready, eligible_at);
        let (mut transfer_ready, _, _) = stage_imported_production_occurrence_v2(
            &transfer_ready,
            occurrence.scheduled_for_unix_ms,
            occurrence,
        )
        .expect("eligible production releases before the next handoff");
        transfer_ready.base.event_sequence += 1;
        transfer_ready.base.last_event_hash = blake3::hash(b"post-release transfer boundary")
            .to_hex()
            .to_string();
        transfer_ready
            .seal()
            .expect("a later boundary may follow the production occurrence");
        let next_package = transfer_ready
            .capture_grid_closure(&package.root_aggregate_id, &next_context)
            .expect("released grid can form a later transfer under a new ID");
        let next_authority =
            DraftGridDirectoryAuthorityV2::for_package(&next_package, TransferPhase::Prepared);
        let relocked = stage_prepared_grid_lock_v2(&transfer_ready, &next_package, &next_authority)
            .expect("historical activation does not blacklist the grid root");
        assert_eq!(
            relocked.locked_transfer_for_subject(&package.root_aggregate_id),
            Some("transfer-grid-v2-next")
        );

        let mut later_gameplay = activated;
        later_gameplay.base.event_sequence += 1;
        later_gameplay.base.last_event_hash = blake3::hash(b"ordinary post-activation motion")
            .to_hex()
            .to_string();
        later_gameplay
            .base
            .grids
            .get_mut(&package.root_aggregate_id)
            .expect("activated grid exists")
            .linear_velocity
            .x += 0.25;
        later_gameplay
            .seal()
            .expect("ordinary gameplay may advance after activation");

        let mut successor_state = later_gameplay;
        successor_state.base.fencing_token += 1;
        successor_state.base.event_sequence += 1;
        successor_state.base.last_event_hash = blake3::hash(b"activation successor fence")
            .to_hex()
            .to_string();
        successor_state
            .seal()
            .expect("successor fence preserves historical activation");
        let mut successor_authority = finalized;
        successor_authority.advance_test_destination_authority();
        let (successor_retry, successor_proof) = stage_imported_grid_activation_v2(
            &successor_state,
            1_800_000_020_006,
            &package,
            &successor_authority,
        )
        .expect("successor retries historical activation under its live fence");
        assert_eq!(successor_retry, successor_state);
        assert_eq!(successor_proof, proof);
    }

    #[test]
    fn imported_production_gate_pauses_then_releases_inside_exact_whole_cell_occurrence() {
        let (imported, package, authority) = materialized_import_fixture();
        let (machine_id, eligibility) = imported
            .imported_production_eligibilities
            .first_key_value()
            .map(|(machine_id, eligibility)| (machine_id.clone(), eligibility.clone()))
            .expect("fixture carries one imported machine hold");
        let initial_queue = imported.base.production_queues[&machine_id].clone();
        let initial_clock = imported.base.production_clock.clone();
        let initial_ledger = imported.base.ledger.clone();
        let completed_job_id = initial_queue[0].job_id.clone();
        let eligible_at = eligibility.eligible_at_unix_ms();

        let early_occurrence = next_occurrence(&imported, eligible_at - 1);
        assert!(
            stage_imported_production_occurrence_v2(
                &imported,
                early_occurrence.scheduled_for_unix_ms - 1,
                early_occurrence.clone(),
            )
            .is_err()
        );
        let (paused, paused_controls, paused_proof) = stage_imported_production_occurrence_v2(
            &imported,
            early_occurrence.scheduled_for_unix_ms,
            early_occurrence.clone(),
        )
        .expect("pre-boundary occurrence commits an explicit transfer pause");
        paused_proof.validate().expect("paused proof validates");
        assert_eq!(paused.base.production_queues[&machine_id], initial_queue);
        assert_eq!(
            paused.imported_production_eligibilities[&machine_id],
            eligibility
        );
        assert_eq!(
            paused.base.production_clock.last_committed_quantum_sequence,
            initial_clock.last_committed_quantum_sequence + 1
        );
        assert_eq!(
            paused_controls.machines()[0].kind(),
            DraftProductionMachineControlKindV2::TransferPaused
        );
        assert!(
            paused.committed_production_releases[paused_controls.controls_root()].outcomes[0]
                .ordinary_outcome
                .is_none()
        );
        assert_eq!(
            DraftGridTransferCellStateV2::decode_canonical(
                &paused.encode_canonical().expect("paused state encodes")
            )
            .expect("paused state restarts"),
            paused
        );
        let (paused_retry, paused_retry_controls, paused_retry_proof) =
            stage_imported_production_occurrence_v2(
                &paused,
                early_occurrence.scheduled_for_unix_ms + 10,
                early_occurrence.clone(),
            )
            .expect("paused occurrence redelivery is exact");
        assert_eq!(paused_retry, paused);
        assert_eq!(paused_retry_controls, paused_controls);
        assert_eq!(paused_retry_proof, paused_proof);

        let due_occurrence = next_occurrence(&paused, eligible_at + 999);
        let (released, release_controls, release_proof) = stage_imported_production_occurrence_v2(
            &paused,
            due_occurrence.scheduled_for_unix_ms,
            due_occurrence.clone(),
        )
        .expect("first eligible occurrence releases and evaluates exactly once");
        release_proof.validate().expect("release proof validates");
        let mut substituted_history_head = release_proof.clone();
        substituted_history_head.history_entry_hash = "ab".repeat(32);
        substituted_history_head
            .resulting_history_head
            .clone_from(&substituted_history_head.history_entry_hash);
        substituted_history_head.proof_hash = substituted_history_head
            .calculate_hash()
            .expect("substituted proof reseals");
        assert!(
            substituted_history_head.validate().is_err(),
            "a resealed proof cannot substitute an unbound occurrence-history head"
        );
        assert_eq!(
            release_controls.machines()[0].kind(),
            DraftProductionMachineControlKindV2::ReleaseAndEvaluate
        );
        assert!(released.imported_production_eligibilities.is_empty());
        assert!(!released.base.production_queues.contains_key(&machine_id));
        assert!(
            !released
                .production_job_origins
                .contains_key(&completed_job_id)
        );
        assert_eq!(
            released.base.ledger.refine_batches,
            initial_ledger.refine_batches + 1
        );
        assert!(
            released.committed_production_releases[release_controls.controls_root()].outcomes[0]
                .ordinary_outcome
                .is_some()
        );
        assert_eq!(
            released
                .base
                .production_clock
                .last_committed_quantum_sequence,
            initial_clock.last_committed_quantum_sequence + 2
        );
        assert_eq!(
            released.locked_transfer_for_subject(&package.root_aggregate_id),
            Some(package.transfer_id.as_str())
        );
        assert_eq!(
            DraftGridTransferCellStateV2::decode_canonical(
                &released.encode_canonical().expect("released state encodes")
            )
            .expect("released state restarts"),
            released
        );
        let (released_retry, released_retry_controls, released_retry_proof) =
            stage_imported_production_occurrence_v2(
                &released,
                due_occurrence.scheduled_for_unix_ms + 10,
                due_occurrence.clone(),
            )
            .expect("released occurrence redelivery is exact");
        assert_eq!(released_retry, released);
        assert_eq!(released_retry_controls, release_controls);
        assert_eq!(released_retry_proof, release_proof);

        let (historical_pause_retry, historical_pause_controls, historical_pause_proof) =
            stage_imported_production_occurrence_v2(
                &released,
                due_occurrence.scheduled_for_unix_ms + 11,
                early_occurrence,
            )
            .expect("an older paused occurrence remains exactly retryable");
        assert_eq!(historical_pause_retry, released);
        assert_eq!(historical_pause_controls, paused_controls);
        assert_eq!(historical_pause_proof, paused_proof);

        let mut deleted_historical_pause = released.clone();
        deleted_historical_pause
            .committed_production_releases
            .remove(paused_controls.controls_root());
        assert!(
            deleted_historical_pause.seal().is_err(),
            "the active history head detects deletion behind the current frontier"
        );

        let (activated, _) =
            stage_imported_grid_activation_v2(&released, eligible_at + 2_000, &package, &authority)
                .expect("production may re-arm before the independent gameplay activation");
        assert!(activated.pending_imports.is_empty());
        assert!(activated.imported_production_eligibilities.is_empty());
        activated
            .validate()
            .expect("post-release activation validates");
    }

    #[test]
    fn imported_production_occurrence_rejects_queue_and_release_history_tamper() {
        let imported = materialized_import_state_fixture();
        let eligibility = imported
            .imported_production_eligibilities
            .first_key_value()
            .map(|(_, eligibility)| eligibility.clone())
            .expect("fixture carries one imported machine hold");
        let occurrence = next_occurrence(&imported, eligibility.eligible_at_unix_ms());

        let mut queue_tamper = imported.clone();
        queue_tamper
            .base
            .production_queues
            .values_mut()
            .next()
            .and_then(|queue| queue.front_mut())
            .expect("fixture queue exists")
            .progress_ticks += 1;
        assert!(
            stage_imported_production_occurrence_v2(
                &queue_tamper,
                occurrence.scheduled_for_unix_ms,
                occurrence.clone(),
            )
            .is_err()
        );

        let (released, controls, _) = stage_imported_production_occurrence_v2(
            &imported,
            occurrence.scheduled_for_unix_ms,
            occurrence,
        )
        .expect("eligible occurrence commits");
        let mut resurrected = released.clone();
        resurrected.imported_production_eligibilities.insert(
            eligibility.machine_block_id().to_owned(),
            eligibility.clone(),
        );
        assert!(resurrected.seal().is_err());

        let mut missing_history = released.clone();
        missing_history
            .committed_production_releases
            .remove(controls.controls_root());
        assert!(missing_history.seal().is_err());

        let mut outcome_tamper = released;
        outcome_tamper
            .committed_production_releases
            .get_mut(controls.controls_root())
            .expect("release history exists")
            .outcomes[0]
            .ordinary_outcome
            .as_mut()
            .expect("release has ordinary outcome")
            .new_progress_ticks += 1;
        assert!(outcome_tamper.seal().is_err());
    }

    #[test]
    fn committed_import_materializes_exact_closure_without_ticking_production() {
        let (reserved, package, _, _, authority) = import_record_fixture();
        let initial_ledger = reserved.base.ledger.clone();
        let initial_clock = reserved.base.production_clock.clone();
        let initial_event_sequence = reserved.base.event_sequence;
        let expected_vector =
            DraftGridTransferLedgerVectorV2::from_package(&package).expect("vector derives");

        let (imported, proof) =
            stage_committed_grid_import_v2(&reserved, 1_800_000_020_000, &package, &authority)
                .expect("destination import commits");
        assert!(imported.aggregate_reservations.is_empty());
        assert_eq!(imported.pending_imports.len(), 1);
        assert_eq!(imported.committed_imports.len(), 1);
        assert_eq!(
            imported.imported_production_eligibilities.len(),
            package.production_queues.len()
        );
        let destination_origin = celestial::cell_address_from_key(&package.destination_cell_key)
            .expect("destination origin derives");
        let expected_grid_position =
            celestial::local_position_from_address(&destination_origin, &package.grid.address)
                .expect("grid destination pose derives");
        let resident_grid = &imported.base.grids[&package.grid.grid_id];
        assert_eq!(resident_grid.address, package.grid.address);
        assert_eq!(resident_grid.position, expected_grid_position);
        assert_eq!(resident_grid.orientation, package.grid.orientation);
        assert_eq!(resident_grid.linear_velocity, package.grid.linear_velocity);
        assert_eq!(
            resident_grid.angular_velocity,
            package.grid.angular_velocity
        );
        for (player_id, player) in &package.players {
            assert_eq!(
                imported.base.player.get(player_id),
                Some(&player.destination_player)
            );
            assert_eq!(
                imported
                    .base
                    .inventories
                    .get(&player.inventory.inventory_id),
                Some(&player.inventory)
            );
            assert_eq!(
                imported.base.processed_operations.get(player_id),
                player.operation_history.as_ref()
            );
        }
        assert_eq!(
            imported.base.player.primary_player_id,
            package.grid.owner_player_id
        );
        assert_eq!(imported.base.production_queues, package.production_queues);
        assert_eq!(
            imported.production_job_origins,
            package.production_job_origins
        );
        assert_eq!(imported.base.production_clock, initial_clock);
        assert_eq!(imported.base.event_sequence, initial_event_sequence + 1);
        assert_eq!(imported.base.last_event_hash, proof.event_hash);
        assert_eq!(proof.ledger_vector, expected_vector);
        assert_eq!(
            imported.base.ledger.transfer_imported_ore,
            initial_ledger.transfer_imported_ore + expected_vector.ore
        );
        assert_eq!(
            imported.base.ledger.transfer_imported_refined,
            initial_ledger.transfer_imported_refined + expected_vector.refined_material
        );
        assert_eq!(
            imported.base.ledger.transfer_imported_components,
            initial_ledger.transfer_imported_components + expected_vector.components
        );
        assert_eq!(
            proof.resulting_active_world_hash,
            imported
                .calculate_active_world_hash()
                .expect("active world hash derives")
        );
        assert!(imported.base.conservation().valid);

        let (retry, retry_proof) =
            stage_committed_grid_import_v2(&imported, 1_800_000_030_000, &package, &authority)
                .expect("exact import retry returns the durable result");
        assert_eq!(retry, imported);
        assert_eq!(retry_proof, proof);

        let mut imported_authority = authority.clone();
        imported_authority.phase = TransferPhase::Imported;
        imported_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationImport);
        imported_authority.destination_import_proof = Some(proof.clone());
        let (late_retry, late_retry_proof) = stage_committed_grid_import_v2(
            &imported,
            1_800_000_040_000,
            &package,
            &imported_authority,
        )
        .expect("directory-imported retry returns the authenticated historical result");
        assert_eq!(late_retry, imported);
        assert_eq!(late_retry_proof, proof);

        let mut finalized_authority = imported_authority.clone();
        finalized_authority.phase = TransferPhase::Finalized;
        finalized_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationActivation);
        let activation_proof = synthetic_activation_proof(&package, &proof);
        finalized_authority.destination_activation_proof = Some(activation_proof.clone());
        finalized_authority
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceFinalization);
        finalized_authority.source_finalization_proof = Some(synthetic_finalization_proof(
            &package,
            finalized_authority
                .source_export_proof
                .as_ref()
                .expect("finalized authority retains its export proof"),
            &proof,
            &activation_proof,
        ));
        let (finalized_retry, finalized_retry_proof) = stage_committed_grid_import_v2(
            &imported,
            1_800_000_050_000,
            &package,
            &finalized_authority,
        )
        .expect("directory-finalized retry returns the authenticated historical result");
        assert_eq!(finalized_retry, imported);
        assert_eq!(finalized_retry_proof, proof);

        let mut substituted_import_authority = imported_authority;
        substituted_import_authority
            .destination_import_proof
            .as_mut()
            .expect("directory import proof exists")
            .resulting_active_world_hash = "ab".repeat(32);
        assert!(
            stage_committed_grid_import_v2(
                &imported,
                1_800_000_060_000,
                &package,
                &substituted_import_authority,
            )
            .is_err()
        );

        let prior = reserved.clone();
        assert!(
            stage_committed_grid_import_v2(&reserved, 1_800_000_009_999, &package, &authority,)
                .is_err()
        );
        assert_eq!(reserved, prior);
    }

    #[test]
    fn prepared_lock_freezes_every_subject_family_and_retries_exactly() {
        let (source, _, package) = package_fixture();
        let state = DraftGridTransferCellStateV2::new(source.clone()).expect("draft source seals");
        let authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked =
            stage_prepared_grid_lock_v2(&state, &package, &authority).expect("closure locks");
        let mut expected_base = source;
        expected_base.event_sequence = locked.base.event_sequence;
        expected_base
            .last_event_hash
            .clone_from(&locked.base.last_event_hash);
        assert_eq!(locked.base, expected_base);
        assert_eq!(locked.base.event_sequence, state.base.event_sequence + 1);
        assert_eq!(locked.committed_prepares.len(), 1);
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
        let (source, _, package) = package_fixture();
        let source = DraftGridTransferCellStateV2::new(source).expect("source seals");
        let state = destination_state(&package);
        let mut authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked = stage_prepared_grid_lock_v2(&source, &package, &authority)
            .expect("source prepare proof seals");
        authority.record_test_source_prepare(&locked, &package.transfer_id);
        let (reserved, receipt) =
            stage_grid_quarantine_v2(&state, 1_800_000_000_000, &package, &authority)
                .expect("destination quarantines");
        receipt.validate().expect("receipt validates");
        let mut expected_base = state.base.clone();
        expected_base.event_sequence = reserved.base.event_sequence;
        expected_base
            .last_event_hash
            .clone_from(&reserved.base.last_event_hash);
        assert_eq!(reserved.base, expected_base);
        assert_eq!(reserved.base.event_sequence, state.base.event_sequence + 1);
        assert_ne!(reserved.state_hash, state.state_hash);
        assert_eq!(reserved.aggregate_reservations.len(), 1);
        assert_eq!(reserved.committed_quarantines.len(), 1);
        assert_eq!(receipt.destination_draft_world_hash, state.state_hash);
        assert_eq!(
            receipt.receipt_hash,
            "121b1b5809105965f6ef9ae2d914c27c850dd0cccfb071aa47b5edbe661652f2"
        );

        let (retry_state, retry_receipt) =
            stage_grid_quarantine_v2(&reserved, 1_800_000_000_001, &package, &authority)
                .expect("exact quarantine retry succeeds");
        assert_eq!(retry_state, reserved);
        assert_eq!(retry_receipt, receipt);

        let mut quarantined_authority = authority.clone();
        quarantined_authority.phase = TransferPhase::Quarantined;
        quarantined_authority.record_test_destination_quarantine(&reserved, &package.transfer_id);
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
    fn successor_destination_fence_can_create_the_first_quarantine() {
        let (source, _, package) = package_fixture();
        let source = DraftGridTransferCellStateV2::new(source).expect("source seals");
        let mut destination = destination_state(&package);
        destination.base.fencing_token = destination
            .base
            .fencing_token
            .checked_add(1)
            .expect("successor fence advances");
        destination.seal().expect("successor destination seals");
        let mut authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked = stage_prepared_grid_lock_v2(&source, &package, &authority)
            .expect("source prepare proof seals");
        authority.record_test_source_prepare(&locked, &package.transfer_id);
        authority.advance_test_destination_authority();
        let (reserved, receipt) =
            stage_grid_quarantine_v2(&destination, 1_800_000_000_000, &package, &authority)
                .expect("successor creates the first exact quarantine");
        assert_eq!(
            receipt.destination_fencing_token,
            package.destination_fencing_token
        );
        assert_eq!(
            reserved.base.fencing_token,
            authority.live_destination_fencing_token
        );
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
        quarantine_authority.record_test_source_prepare(&locked, &package.transfer_id);
        let (reserved, receipt) = stage_grid_quarantine_v2(
            &destination,
            1_800_000_000_000,
            &package,
            &quarantine_authority,
        )
        .expect("destination reserves");

        let mut directory_aborted_before_quarantine =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Aborting);
        directory_aborted_before_quarantine
            .record_test_source_prepare(&locked, &package.transfer_id);
        let (reopened_reservation, late_receipt, late_quarantine_proof) =
            reconcile_quarantined_grid_v2(
                &DraftGridTransferCellStateV2::decode_canonical(
                    &reserved.encode_canonical().expect("reservation encodes"),
                )
                .expect("reservation reopens"),
                &package,
                &directory_aborted_before_quarantine,
            )
            .expect("aborting directory can recover the exact cell-first quarantine proof");
        assert_eq!(reopened_reservation, reserved);
        assert_eq!(late_receipt, receipt);
        assert_eq!(
            late_quarantine_proof,
            reserved.committed_quarantines[&package.transfer_id]
        );

        let mut abort_authority =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Aborting);
        abort_authority.record_test_source_prepare(&locked, &package.transfer_id);
        abort_authority.record_test_destination_quarantine(&reserved, &package.transfer_id);
        assert_eq!(
            abort_authority.quarantine_receipt_hash.as_deref(),
            Some(receipt.receipt_hash.as_str())
        );
        let (source_clean, source_witness) =
            stage_aborted_grid_cleanup_v2(&locked, 1_800_000_010_000, &package, &abort_authority)
                .expect("source unlocks");
        let mut source_proven_authority = abort_authority.clone();
        source_proven_authority.record_test_abort(&source_witness);
        let (destination_clean, destination_witness) = stage_aborted_grid_cleanup_v2(
            &reserved,
            1_800_000_010_001,
            &package,
            &source_proven_authority,
        )
        .expect("destination unreserves");
        let mut aborted_authority = source_proven_authority;
        aborted_authority.record_test_abort(&destination_witness);
        aborted_authority.phase = TransferPhase::Aborted;
        let mut expected_source_base = source_state.base.clone();
        expected_source_base.event_sequence = source_clean.base.event_sequence;
        expected_source_base
            .last_event_hash
            .clone_from(&source_clean.base.last_event_hash);
        assert_eq!(source_clean.base, expected_source_base);
        assert_eq!(
            source_clean.base.event_sequence,
            source_state.base.event_sequence + 2
        );
        let mut expected_destination_base = destination.base.clone();
        expected_destination_base.event_sequence = destination_clean.base.event_sequence;
        expected_destination_base
            .last_event_hash
            .clone_from(&destination_clean.base.last_event_hash);
        assert_eq!(destination_clean.base, expected_destination_base);
        assert_eq!(
            destination_clean.base.event_sequence,
            destination.base.event_sequence + 2
        );
        assert!(source_clean.aggregate_locks.is_empty());
        assert!(destination_clean.aggregate_reservations.is_empty());
        assert!(source_witness.removed_authority);
        assert!(destination_witness.removed_authority);
        let (source_retry, source_retry_witness) = stage_aborted_grid_cleanup_v2(
            &source_clean,
            1_800_000_020_000,
            &package,
            &aborted_authority,
        )
        .expect("source cleanup retry succeeds");
        assert_eq!(source_retry, source_clean);
        assert_eq!(source_retry_witness, source_witness);
        let (destination_retry, destination_retry_witness) = stage_aborted_grid_cleanup_v2(
            &destination_clean,
            1_800_000_020_001,
            &package,
            &aborted_authority,
        )
        .expect("destination cleanup retry succeeds");
        assert_eq!(destination_retry, destination_clean);
        assert_eq!(destination_retry_witness, destination_witness);

        let mut substituted = aborted_authority;
        substituted
            .source_abort_proof
            .as_mut()
            .expect("source abort proof is retained")
            .proof_hash = "ab".repeat(32);
        assert!(reconcile_aborted_grid_cleanup_v2(&source_clean, &package, &substituted).is_err());
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
        successor.advance_test_source_authority();
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
                .checked_add(2)
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
        directory_proven.source_exported_at_unix_ms = Some(proof.exported_at_unix_ms);
        directory_proven.source_export_proof = Some(proof.clone());
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
        imported.destination_import_proof = Some(synthetic_import_proof(&package, &proof));
        let (_, imported_proof_retry) =
            stage_committed_grid_export_v2(&exported, 1_800_000_026_000, &package, &imported)
                .expect("imported directory phase retrieves the historical export proof");
        assert_eq!(imported_proof_retry, proof);

        let mut finalized = imported.clone();
        finalized.phase = TransferPhase::Finalized;
        finalized
            .proofs
            .insert(DraftGridDirectoryProofKindV2::DestinationActivation);
        let imported_proof = imported
            .destination_import_proof
            .as_ref()
            .expect("imported authority carries its typed import proof")
            .clone();
        finalized.destination_activation_proof =
            Some(synthetic_activation_proof(&package, &imported_proof));
        finalized
            .proofs
            .insert(DraftGridDirectoryProofKindV2::SourceFinalization);
        finalized.source_finalization_proof = Some(synthetic_finalization_proof(
            &package,
            &proof,
            &imported_proof,
            finalized
                .destination_activation_proof
                .as_ref()
                .expect("finalized authority carries activation proof"),
        ));
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
    fn directory_fencing_history_rejects_a_resealed_generation_fence_cross_pair() {
        let (source, _, package) = package_fixture();
        assert!(package.source_assignment_generation > 1);
        let source = DraftGridTransferCellStateV2::new(source).expect("source seals");
        let prepared =
            DraftGridDirectoryAuthorityV2::for_package(&package, TransferPhase::Prepared);
        let locked = stage_prepared_grid_lock_v2(&source, &package, &prepared)
            .expect("source prepare proof seals");
        let mut proven = prepared;
        proven.record_test_source_prepare(&locked, &package.transfer_id);

        let mut cross_paired = proven;
        let proof = cross_paired
            .source_prepare_proof
            .as_mut()
            .expect("source prepare proof is retained");
        proof.assignment_generation -= 1;
        proof
            .seal_hashes_for_test()
            .expect("cross-paired proof is internally self-consistent");
        assert!(
            cross_paired.validate_package(&package).is_err(),
            "directory provenance rejects a generation paired with another generation's fence"
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
