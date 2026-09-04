// SPDX-License-Identifier: AGPL-3.0-or-later

//! Cell-directory-v3 model and active protocol-19 history store.
//!
//! The crate-private activated path opens only the signed-genesis history and
//! advances authority through its single-writer journal. The protocol-18
//! directory remains isolated while lifecycle-v2 scheduling is completed.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use verse_protocol::CellKeyV1;

use crate::cell_directory::{
    AggregatePlacementRecord, AggregatePlacementState, BundledPlacementMember,
    BundledPlacementPlan, BundledPlacementTransition, CellAssignmentRecord, CellAssignmentState,
    CellDirectoryError, MobileAggregateKind, TransferPhase, stage_bundled_placement_transition,
};
use crate::grid_handoff_v2::DraftGridCompatibilityTupleV19;
#[cfg(test)]
use crate::grid_handoff_v2::state::DraftGridDirectoryAuthorityV2;
use crate::grid_handoff_v2::state::{
    DraftGridAbortCleanupProofV2, DraftGridActivationProofV2, DraftGridExportProofV2,
    DraftGridFinalizationProofV2, DraftGridImportProofV2, DraftGridPrepareProofV2,
    DraftGridQuarantineProofV2, DraftGridTransferAbortSideV2, DraftGridTransferLedgerVectorV2,
};
use crate::{celestial, model::valid_blake3_hex};

const DRAFT_CELL_DIRECTORY_V3_SCHEMA_VERSION: u32 = 3;
const DRAFT_AGGREGATE_TRANSFER_PACKAGE_SCHEMA_VERSION: u32 = 2;
const DRAFT_AGGREGATE_TRANSFER_RECEIPT_SCHEMA_VERSION: u32 = 2;
const DOCUMENT_HASH_DOMAIN: &[u8] = b"the-verse/cell-directory-document/v3\0";
const MAX_DRAFT_DIRECTORY_V3_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_DRAFT_PLACEMENTS: usize = 8_192;
const MAX_DRAFT_TRANSFERS: usize = 1_024;
const MAX_DRAFT_TRANSFER_MEMBERSHIPS: usize = 8_192;
const DRAFT_DIRECTORY_HISTORY_SCHEMA_VERSION: u32 = 1;
const DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY: &str = "protocol-19-directory-v3";
const DRAFT_DIRECTORY_HISTORY_LOCK_FILE: &str = "writer.lock";
const DRAFT_DIRECTORY_HISTORY_HEAD_FILE: &str = "head-v3.json";
const DRAFT_DIRECTORY_HISTORY_FILE: &str = "history-v3.ndjson";
const MAX_STALE_DIRECTORY_HEAD_TEMPS: usize = 64;
const DRAFT_DIRECTORY_HISTORY_ENTRY_HASH_DOMAIN: &[u8] =
    b"the-verse/cell-directory-history-entry/v3\0";
const MIGRATION_ASSIGNMENT_ROOT_DOMAIN: &[u8] =
    b"the-verse/protocol-19-directory-genesis-assignments/v1\0";
const MIGRATION_PLACEMENT_ROOT_DOMAIN: &[u8] =
    b"the-verse/protocol-19-directory-genesis-placements/v1\0";
const MIGRATION_GENESIS_RECORD_HASH_DOMAIN: &[u8] =
    b"the-verse/protocol-19-directory-genesis-record/v1\0";
const MAX_DRAFT_DIRECTORY_HISTORY_BYTES: u64 = 512 * 1_024 * 1_024;
const MAX_DRAFT_DIRECTORY_HISTORY_LINE_BYTES: usize = MAX_DRAFT_DIRECTORY_V3_BYTES + 4_096;
const MAX_DRAFT_DIRECTORY_HISTORY_HEAD_BYTES: u64 = 16 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryBundleV3 {
    package_schema_version: u32,
    receipt_schema_version: u32,
    aggregate_kind: MobileAggregateKind,
    closure_root: String,
    conservation_root: String,
    package_hash: String,
    members: Vec<BundledPlacementMember>,
    member_root: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DirectoryPhaseProofKindV3 {
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
pub(super) struct DirectoryPhaseProofV3 {
    kind: DirectoryPhaseProofKindV3,
    transfer_id: String,
    member_root: String,
    package_hash: String,
    cell_id: String,
    assignment_generation: u64,
    fencing_token: u64,
    event_sequence: u64,
    event_hash: String,
    event_payload_hash: Option<String>,
    world_hash: String,
    quarantine_receipt_hash: Option<String>,
    export_proof_hash: Option<String>,
    prepare_proof_hash: Option<String>,
    quarantine_proof_hash: Option<String>,
    import_proof_hash: Option<String>,
    activation_proof_hash: Option<String>,
    finalization_proof_hash: Option<String>,
    destination_import_proof_hash: Option<String>,
    prior_event_sequence: Option<u64>,
    prior_event_hash: Option<String>,
    prior_draft_world_hash: Option<String>,
    prior_active_world_hash: Option<String>,
    quarantined_at_unix_ms: Option<u64>,
    imported_at_unix_ms: Option<u64>,
    destination_activated_at_unix_ms: Option<u64>,
    source_export_proof_hash: Option<String>,
    source_exported_at_unix_ms: Option<u64>,
    destination_production_lifecycle_generation: Option<u64>,
    production_eligibility_root: Option<String>,
    mutation_witness_hash: Option<String>,
    ledger_vector: Option<DraftGridTransferLedgerVectorV2>,
    trusted_time_unix_ms: Option<u64>,
    prepared_at_simulation_tick: Option<u64>,
    abort_witness_hash: Option<String>,
    abort_proof_hash: Option<String>,
    resulting_draft_world_hash: Option<String>,
    abort_removed_authority: Option<bool>,
}

impl DirectoryPhaseProofV3 {
    pub(super) fn cell_id(&self) -> &str {
        &self.cell_id
    }

    pub(super) fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    pub(super) fn fencing_token(&self) -> u64 {
        self.fencing_token
    }

    pub(super) fn event_sequence(&self) -> u64 {
        self.event_sequence
    }

    pub(super) fn event_hash(&self) -> &str {
        &self.event_hash
    }

    pub(super) fn world_hash(&self) -> &str {
        &self.world_hash
    }

    pub(super) fn quarantine_receipt_hash(&self) -> Option<&str> {
        self.quarantine_receipt_hash.as_deref()
    }

    pub(super) fn export_proof_hash(&self) -> Option<&str> {
        self.export_proof_hash.as_deref()
    }

    pub(super) fn trusted_time_unix_ms(&self) -> Option<u64> {
        self.trusted_time_unix_ms
    }

    fn source_prepare_cell_proof(
        &self,
        root_aggregate_id: &str,
    ) -> Option<DraftGridPrepareProofV2> {
        let proof = DraftGridPrepareProofV2 {
            transfer_id: self.transfer_id.clone(),
            root_aggregate_id: root_aggregate_id.to_owned(),
            member_root: self.member_root.clone(),
            package_hash: self.package_hash.clone(),
            source_cell_id: self.cell_id.clone(),
            assignment_generation: self.assignment_generation,
            fencing_token: self.fencing_token,
            prior_event_sequence: self.prior_event_sequence?,
            prior_event_hash: self.prior_event_hash.clone()?,
            event_sequence: self.event_sequence,
            event_hash: self.event_hash.clone(),
            event_payload_hash: self.event_payload_hash.clone()?,
            prior_active_world_hash: self.prior_active_world_hash.clone()?,
            resulting_active_world_hash: self.world_hash.clone(),
            prepared_at_simulation_tick: self.prepared_at_simulation_tick?,
            mutation_witness_hash: self.mutation_witness_hash.clone()?,
            proof_hash: self.prepare_proof_hash.clone()?,
        };
        proof.validate_for_directory().is_ok().then_some(proof)
    }

    fn destination_quarantine_cell_proof(
        &self,
        root_aggregate_id: &str,
    ) -> Option<DraftGridQuarantineProofV2> {
        let proof = DraftGridQuarantineProofV2 {
            transfer_id: self.transfer_id.clone(),
            root_aggregate_id: root_aggregate_id.to_owned(),
            member_root: self.member_root.clone(),
            package_hash: self.package_hash.clone(),
            destination_cell_id: self.cell_id.clone(),
            assignment_generation: self.assignment_generation,
            fencing_token: self.fencing_token,
            prior_event_sequence: self.prior_event_sequence?,
            prior_event_hash: self.prior_event_hash.clone()?,
            event_sequence: self.event_sequence,
            event_hash: self.event_hash.clone(),
            event_payload_hash: self.event_payload_hash.clone()?,
            prior_active_world_hash: self.prior_active_world_hash.clone()?,
            resulting_active_world_hash: self.world_hash.clone(),
            quarantine_receipt_hash: self.quarantine_receipt_hash.clone()?,
            quarantined_at_unix_ms: self.quarantined_at_unix_ms?,
            mutation_witness_hash: self.mutation_witness_hash.clone()?,
            proof_hash: self.quarantine_proof_hash.clone()?,
        };
        proof.validate_for_directory().is_ok().then_some(proof)
    }

    fn destination_import_cell_proof(
        &self,
        root_aggregate_id: &str,
    ) -> Option<DraftGridImportProofV2> {
        let import = DraftGridImportProofV2 {
            transfer_id: self.transfer_id.clone(),
            root_aggregate_id: root_aggregate_id.to_owned(),
            member_root: self.member_root.clone(),
            package_hash: self.package_hash.clone(),
            destination_cell_id: self.cell_id.clone(),
            assignment_generation: self.assignment_generation,
            fencing_token: self.fencing_token,
            prior_event_sequence: self.prior_event_sequence?,
            prior_event_hash: self.prior_event_hash.clone()?,
            event_sequence: self.event_sequence,
            event_hash: self.event_hash.clone(),
            event_payload_hash: self.event_payload_hash.clone()?,
            prior_draft_world_hash: self.prior_draft_world_hash.clone()?,
            resulting_active_world_hash: self.world_hash.clone(),
            quarantine_receipt_hash: self.quarantine_receipt_hash.clone()?,
            quarantined_at_unix_ms: self.quarantined_at_unix_ms?,
            source_export_proof_hash: self.source_export_proof_hash.clone()?,
            source_exported_at_unix_ms: self.source_exported_at_unix_ms?,
            imported_at_unix_ms: self.trusted_time_unix_ms?,
            destination_production_lifecycle_generation: self
                .destination_production_lifecycle_generation?,
            production_eligibility_root: self.production_eligibility_root.clone()?,
            mutation_witness_hash: self.mutation_witness_hash.clone()?,
            proof_hash: self.import_proof_hash.clone()?,
            ledger_vector: self.ledger_vector?,
        };
        import.validate().is_ok().then_some(import)
    }

    fn destination_activation_cell_proof(
        &self,
        root_aggregate_id: &str,
    ) -> Option<DraftGridActivationProofV2> {
        let activation = DraftGridActivationProofV2 {
            transfer_id: self.transfer_id.clone(),
            root_aggregate_id: root_aggregate_id.to_owned(),
            member_root: self.member_root.clone(),
            package_hash: self.package_hash.clone(),
            destination_cell_id: self.cell_id.clone(),
            assignment_generation: self.assignment_generation,
            fencing_token: self.fencing_token,
            prior_event_sequence: self.prior_event_sequence?,
            prior_event_hash: self.prior_event_hash.clone()?,
            event_sequence: self.event_sequence,
            event_hash: self.event_hash.clone(),
            event_payload_hash: self.event_payload_hash.clone()?,
            prior_active_world_hash: self.prior_active_world_hash.clone()?,
            resulting_active_world_hash: self.world_hash.clone(),
            quarantine_receipt_hash: self.quarantine_receipt_hash.clone()?,
            destination_import_proof_hash: self.destination_import_proof_hash.clone()?,
            imported_at_unix_ms: self.imported_at_unix_ms?,
            activated_at_unix_ms: self.trusted_time_unix_ms?,
            production_eligibility_root: self.production_eligibility_root.clone()?,
            mutation_witness_hash: self.mutation_witness_hash.clone()?,
            proof_hash: self.activation_proof_hash.clone()?,
        };
        activation.validate().is_ok().then_some(activation)
    }

    fn source_finalization_cell_proof(
        &self,
        root_aggregate_id: &str,
    ) -> Option<DraftGridFinalizationProofV2> {
        let finalization = DraftGridFinalizationProofV2 {
            transfer_id: self.transfer_id.clone(),
            root_aggregate_id: root_aggregate_id.to_owned(),
            member_root: self.member_root.clone(),
            package_hash: self.package_hash.clone(),
            source_cell_id: self.cell_id.clone(),
            assignment_generation: self.assignment_generation,
            fencing_token: self.fencing_token,
            prior_event_sequence: self.prior_event_sequence?,
            prior_event_hash: self.prior_event_hash.clone()?,
            event_sequence: self.event_sequence,
            event_hash: self.event_hash.clone(),
            event_payload_hash: self.event_payload_hash.clone()?,
            prior_active_world_hash: self.prior_active_world_hash.clone()?,
            resulting_active_world_hash: self.world_hash.clone(),
            source_export_proof_hash: self.source_export_proof_hash.clone()?,
            source_exported_at_unix_ms: self.source_exported_at_unix_ms?,
            destination_import_proof_hash: self.destination_import_proof_hash.clone()?,
            imported_at_unix_ms: self.imported_at_unix_ms?,
            destination_activation_proof_hash: self.activation_proof_hash.clone()?,
            activated_at_unix_ms: self.destination_activated_at_unix_ms?,
            finalized_at_unix_ms: self.trusted_time_unix_ms?,
            mutation_witness_hash: self.mutation_witness_hash.clone()?,
            proof_hash: self.finalization_proof_hash.clone()?,
        };
        finalization.validate().is_ok().then_some(finalization)
    }

    fn abort_cell_proof(
        &self,
        expected_side: DraftGridTransferAbortSideV2,
    ) -> Option<DraftGridAbortCleanupProofV2> {
        let proof = DraftGridAbortCleanupProofV2 {
            side: expected_side,
            transfer_id: self.transfer_id.clone(),
            member_root: self.member_root.clone(),
            package_hash: self.package_hash.clone(),
            cell_id: self.cell_id.clone(),
            assignment_generation: self.assignment_generation,
            fencing_token: self.fencing_token,
            event_sequence: self.event_sequence,
            event_hash: self.event_hash.clone(),
            event_payload_hash: self.event_payload_hash.clone()?,
            prior_event_sequence: self.prior_event_sequence?,
            prior_event_hash: self.prior_event_hash.clone()?,
            prior_draft_world_hash: self.prior_draft_world_hash.clone()?,
            resulting_draft_world_hash: self.resulting_draft_world_hash.clone()?,
            trusted_time_unix_ms: self.trusted_time_unix_ms?,
            mutation_witness_hash: self.mutation_witness_hash.clone()?,
            quarantine_receipt_hash: self.quarantine_receipt_hash.clone(),
            abort_witness_hash: self.abort_witness_hash.clone()?,
            removed_authority: self.abort_removed_authority?,
            proof_hash: self.abort_proof_hash.clone()?,
        };
        proof.validate().is_ok().then_some(proof)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellTransferRecordV3 {
    transfer_id: String,
    root_aggregate_id: String,
    source_cell_key: CellKeyV1,
    source_cell_id: String,
    destination_cell_key: CellKeyV1,
    destination_cell_id: String,
    source_assignment_generation: u64,
    source_fencing_token: u64,
    destination_assignment_generation: u64,
    destination_fencing_token: u64,
    bundle: DirectoryBundleV3,
    quarantine_receipt_hash: Option<String>,
    source_prepare_proof: Option<DirectoryPhaseProofV3>,
    destination_quarantine_proof: Option<DirectoryPhaseProofV3>,
    source_export_proof: Option<DirectoryPhaseProofV3>,
    import_proof: Option<DirectoryPhaseProofV3>,
    destination_activation_proof: Option<DirectoryPhaseProofV3>,
    finalization_proof: Option<DirectoryPhaseProofV3>,
    source_abort_proof: Option<DirectoryPhaseProofV3>,
    destination_abort_proof: Option<DirectoryPhaseProofV3>,
    phase: TransferPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MigrationPlacementFloorV1 {
    aggregate_id: String,
    aggregate_kind: MobileAggregateKind,
    cell_key: CellKeyV1,
    cell_id: String,
    placement_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryMigrationGenesisV1 {
    schema_version: u32,
    source_directory_revision: u64,
    source_directory_document_hash: String,
    source_terminal_transfer_count: u64,
    source_terminal_transfer_root: String,
    source_terminal_transfer_ids: BTreeSet<String>,
    source_assignment_root: String,
    source_placement_root: String,
    target_assignment_root: String,
    target_placement_root: String,
    identity_map_root: String,
    production_origin_root: String,
    normalized_gameplay_root: String,
    placement_floors: BTreeMap<String, MigrationPlacementFloorV1>,
    record_hash: String,
}

impl DirectoryMigrationGenesisV1 {
    fn new(
        transform: &crate::grid_handoff_v2::migration_transform::ValidatedProtocol19MigrationTransform<'_>,
        assignments: &BTreeMap<String, CellAssignmentRecord>,
        placements: &BTreeMap<String, AggregatePlacementRecord>,
    ) -> Result<Self, CellDirectoryError> {
        let source = transform.source();
        let source_terminal_transfer_ids = source
            .transfers()
            .map(|transfer| transfer.transfer_id.clone())
            .collect::<BTreeSet<_>>();
        let placement_floors = placements
            .iter()
            .filter(|(_, placement)| placement.placement_generation > 1)
            .map(|(aggregate_id, placement)| {
                (
                    aggregate_id.clone(),
                    MigrationPlacementFloorV1 {
                        aggregate_id: aggregate_id.clone(),
                        aggregate_kind: placement.aggregate_kind,
                        cell_key: placement.cell_key.clone(),
                        cell_id: placement.cell_id.clone(),
                        placement_generation: placement.placement_generation,
                    },
                )
            })
            .collect();
        let mut record = Self {
            schema_version: 1,
            source_directory_revision: source.directory_revision(),
            source_directory_document_hash: source.directory_document_hash().to_owned(),
            source_terminal_transfer_count: source.terminal_transfer_count(),
            source_terminal_transfer_root: source.terminal_transfer_root().to_owned(),
            source_terminal_transfer_ids,
            source_assignment_root: source.assignment_root().to_owned(),
            source_placement_root: source.placement_root().to_owned(),
            target_assignment_root: hash_directory_genesis(
                MIGRATION_ASSIGNMENT_ROOT_DOMAIN,
                assignments,
            )?,
            target_placement_root: hash_directory_genesis(
                MIGRATION_PLACEMENT_ROOT_DOMAIN,
                placements,
            )?,
            identity_map_root: transform.identity_map_root().to_owned(),
            production_origin_root: transform.production_origin_root().to_owned(),
            normalized_gameplay_root: transform.normalized_gameplay_root().to_owned(),
            placement_floors,
            record_hash: String::new(),
        };
        record.record_hash = record.calculate_hash()?;
        record.validate()?;
        Ok(record)
    }

    fn calculate_hash(&self) -> Result<String, CellDirectoryError> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_directory_genesis(MIGRATION_GENESIS_RECORD_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), CellDirectoryError> {
        if self.schema_version != 1
            || self.source_directory_revision == 0
            || ![
                &self.source_directory_document_hash,
                &self.source_terminal_transfer_root,
                &self.source_assignment_root,
                &self.source_placement_root,
                &self.target_assignment_root,
                &self.target_placement_root,
                &self.identity_map_root,
                &self.production_origin_root,
                &self.normalized_gameplay_root,
                &self.record_hash,
            ]
            .into_iter()
            .all(|hash| valid_blake3_hex(hash))
            || self.record_hash != self.calculate_hash()?
            || usize::try_from(self.source_terminal_transfer_count).ok()
                != Some(self.source_terminal_transfer_ids.len())
            || self
                .source_terminal_transfer_ids
                .iter()
                .any(|transfer_id| validate_stable_id(transfer_id, "transfer").is_err())
            || self.placement_floors.iter().any(|(aggregate_id, floor)| {
                floor.aggregate_id != *aggregate_id
                    || floor.placement_generation <= 1
                    || floor.cell_id.is_empty()
                    || floor.cell_key.universe_id.is_empty()
            })
        {
            return Err(invalid("directory-v3 migration genesis record is invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellDirectoryDocumentV3 {
    schema_version: u32,
    universe_id: String,
    universe_manifest_hash: String,
    directory_revision: u64,
    assignments: BTreeMap<String, CellAssignmentRecord>,
    placements: BTreeMap<String, AggregatePlacementRecord>,
    transfers: BTreeMap<String, CellTransferRecordV3>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    migration_genesis: Option<DirectoryMigrationGenesisV1>,
    document_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftDirectoryHistoryEntryV3 {
    history_schema_version: u32,
    previous_entry_hash: String,
    document: CellDirectoryDocumentV3,
    entry_hash: String,
}

impl DraftDirectoryHistoryEntryV3 {
    fn new(
        previous_entry_hash: String,
        document: CellDirectoryDocumentV3,
    ) -> Result<Self, CellDirectoryError> {
        document.validate()?;
        let mut entry = Self {
            history_schema_version: DRAFT_DIRECTORY_HISTORY_SCHEMA_VERSION,
            previous_entry_hash,
            document,
            entry_hash: String::new(),
        };
        entry.entry_hash = entry.calculate_hash()?;
        entry.validate_self()?;
        Ok(entry)
    }

    fn calculate_hash(&self) -> Result<String, CellDirectoryError> {
        let mut material = self.clone();
        material.entry_hash.clear();
        let bytes = serde_json::to_vec(&material).map_err(|source| {
            invalid(format!(
                "v3 directory history hash material cannot be encoded: {source}"
            ))
        })?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(DRAFT_DIRECTORY_HISTORY_ENTRY_HASH_DOMAIN);
        hasher.update(&bytes);
        Ok(hasher.finalize().to_hex().to_string())
    }

    fn validate_self(&self) -> Result<(), CellDirectoryError> {
        self.document.validate()?;
        if self.history_schema_version != DRAFT_DIRECTORY_HISTORY_SCHEMA_VERSION
            || (!self.previous_entry_hash.is_empty()
                && !valid_blake3_hex(&self.previous_entry_hash))
            || !valid_blake3_hex(&self.entry_hash)
            || self.entry_hash != self.calculate_hash()?
        {
            return Err(invalid(
                "v3 directory history entry schema or hash is invalid",
            ));
        }
        Ok(())
    }

    fn encode_canonical(&self) -> Result<Vec<u8>, CellDirectoryError> {
        self.validate_self()?;
        let bytes = serde_json::to_vec(self).map_err(|source| {
            invalid(format!(
                "v3 directory history entry cannot be encoded: {source}"
            ))
        })?;
        if bytes.len() > MAX_DRAFT_DIRECTORY_HISTORY_LINE_BYTES {
            return Err(invalid("v3 directory history entry exceeds its byte bound"));
        }
        Ok(bytes)
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, CellDirectoryError> {
        if bytes.is_empty() || bytes.len() > MAX_DRAFT_DIRECTORY_HISTORY_LINE_BYTES {
            return Err(invalid("v3 directory history entry exceeds its byte bound"));
        }
        let entry = serde_json::from_slice::<Self>(bytes).map_err(|source| {
            invalid(format!(
                "v3 directory history entry JSON is invalid: {source}"
            ))
        })?;
        entry.validate_self()?;
        if entry.encode_canonical()? != bytes {
            return Err(invalid(
                "v3 directory history entry bytes are not canonical",
            ));
        }
        Ok(entry)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftDirectoryHistoryHeadV3 {
    history_schema_version: u32,
    compatibility: DraftGridCompatibilityTupleV19,
    universe_id: String,
    universe_manifest_hash: String,
    entry_count: u64,
    directory_revision: u64,
    document_hash: String,
    entry_hash: String,
    journal_byte_length: u64,
}

impl DraftDirectoryHistoryHeadV3 {
    fn empty(document: &CellDirectoryDocumentV3) -> Self {
        Self {
            history_schema_version: DRAFT_DIRECTORY_HISTORY_SCHEMA_VERSION,
            compatibility: DraftGridCompatibilityTupleV19::canonical(),
            universe_id: document.universe_id.clone(),
            universe_manifest_hash: document.universe_manifest_hash.clone(),
            entry_count: 0,
            directory_revision: 0,
            document_hash: String::new(),
            entry_hash: String::new(),
            journal_byte_length: 0,
        }
    }

    fn from_tip(
        prior: &Self,
        entry: &DraftDirectoryHistoryEntryV3,
        journal_byte_length: u64,
    ) -> Result<Self, CellDirectoryError> {
        Ok(Self {
            history_schema_version: DRAFT_DIRECTORY_HISTORY_SCHEMA_VERSION,
            compatibility: DraftGridCompatibilityTupleV19::canonical(),
            universe_id: prior.universe_id.clone(),
            universe_manifest_hash: prior.universe_manifest_hash.clone(),
            entry_count: prior
                .entry_count
                .checked_add(1)
                .ok_or_else(|| invalid("v3 directory history entry count exhausted"))?,
            directory_revision: entry.document.directory_revision,
            document_hash: entry.document.document_hash.clone(),
            entry_hash: entry.entry_hash.clone(),
            journal_byte_length,
        })
    }

    fn validate_identity(
        &self,
        expected_universe_id: &str,
        expected_manifest_hash: &str,
    ) -> Result<(), CellDirectoryError> {
        let empty_tip = self.entry_count == 0
            && self.directory_revision == 0
            && self.document_hash.is_empty()
            && self.entry_hash.is_empty()
            && self.journal_byte_length == 0;
        let populated_tip = self.entry_count > 0
            && self.directory_revision > 0
            && valid_blake3_hex(&self.document_hash)
            && valid_blake3_hex(&self.entry_hash)
            && self.journal_byte_length > 0;
        if self.history_schema_version != DRAFT_DIRECTORY_HISTORY_SCHEMA_VERSION
            || self.compatibility != DraftGridCompatibilityTupleV19::canonical()
            || self.universe_id != expected_universe_id
            || self.universe_manifest_hash != expected_manifest_hash
            || !valid_blake3_hex(&self.universe_manifest_hash)
            || !(empty_tip || populated_tip)
        {
            return Err(invalid(
                "v3 directory history head identity or compatibility is invalid",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftDirectoryHistoryLocationV3 {
    offset: u64,
    line_length: usize,
    document_hash: String,
    entry_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CellAuthorityTransitionV3 {
    ClaimSleeping,
    RecoverAssigned,
}

/// Non-Serde proof of the exact directory-v3 genesis derived from the frozen
/// protocol-18 directory and its validated world-21 transform.
#[derive(Debug)]
pub(crate) struct ValidatedProtocol19DirectoryGenesis<'migration, 'source> {
    transform: &'migration crate::grid_handoff_v2::migration_transform::ValidatedProtocol19MigrationTransform<'source>,
    document: CellDirectoryDocumentV3,
    history_entry: DraftDirectoryHistoryEntryV3,
    document_bytes: Vec<u8>,
    history_entry_bytes: Vec<u8>,
    assignment_root: String,
    placement_root: String,
}

impl<'migration, 'source> ValidatedProtocol19DirectoryGenesis<'migration, 'source> {
    pub(crate) fn derive(
        transform: &'migration crate::grid_handoff_v2::migration_transform::ValidatedProtocol19MigrationTransform<'source>,
    ) -> Result<Self, CellDirectoryError> {
        let source = transform.source();
        let assignments = source
            .assignments()
            .cloned()
            .map(|assignment| (assignment.cell_id.clone(), assignment))
            .collect::<BTreeMap<_, _>>();
        let mut placements = BTreeMap::new();
        for source_placement in source.placements() {
            if source_placement.state != AggregatePlacementState::Resident
                || source_placement.active_transfer_id.is_some()
            {
                return Err(invalid(
                    "frozen source placement is not terminal and resident",
                ));
            }
            let aggregate_id = transform
                .target_aggregate_id(
                    source_placement.aggregate_kind,
                    &source_placement.cell_id,
                    &source_placement.aggregate_id,
                )
                .to_owned();
            let target_cell = transform
                .cells()
                .binary_search_by(|cell| cell.cell_id().cmp(&source_placement.cell_id))
                .ok()
                .and_then(|index| transform.cells().get(index))
                .ok_or_else(|| invalid("source placement references an absent target cell"))?;
            if !target_cell.contains_aggregate(source_placement.aggregate_kind, &aggregate_id) {
                return Err(invalid(
                    "target placement is absent from its transformed cell",
                ));
            }
            let target_placement = AggregatePlacementRecord {
                aggregate_id: aggregate_id.clone(),
                aggregate_kind: source_placement.aggregate_kind,
                cell_key: source_placement.cell_key.clone(),
                cell_id: source_placement.cell_id.clone(),
                placement_generation: source_placement.placement_generation,
                state: AggregatePlacementState::Resident,
                active_transfer_id: None,
            };
            if placements.insert(aggregate_id, target_placement).is_some() {
                return Err(invalid(
                    "two frozen placements map to one target aggregate identity",
                ));
            }
        }
        for cell in transform.cells() {
            for (aggregate_kind, aggregate_id) in cell.resident_aggregates() {
                if let Some(existing) = placements.get(&aggregate_id) {
                    if existing.aggregate_kind != aggregate_kind
                        || existing.cell_key != *cell.cell_key()
                        || existing.cell_id != cell.cell_id()
                    {
                        return Err(invalid(
                            "target aggregate has a conflicting directory placement",
                        ));
                    }
                    continue;
                }
                let placement = AggregatePlacementRecord {
                    aggregate_id: aggregate_id.clone(),
                    aggregate_kind,
                    cell_key: cell.cell_key().clone(),
                    cell_id: cell.cell_id().to_owned(),
                    placement_generation: 1,
                    state: AggregatePlacementState::Resident,
                    active_transfer_id: None,
                };
                placements.insert(aggregate_id, placement);
            }
        }
        let migration_genesis =
            DirectoryMigrationGenesisV1::new(transform, &assignments, &placements)?;
        let mut document = CellDirectoryDocumentV3 {
            schema_version: DRAFT_CELL_DIRECTORY_V3_SCHEMA_VERSION,
            universe_id: source.universe_id().to_owned(),
            universe_manifest_hash: transform.target_manifest_hash().to_owned(),
            directory_revision: 1,
            assignments,
            placements,
            transfers: BTreeMap::new(),
            migration_genesis: Some(migration_genesis),
            document_hash: String::new(),
        };
        document.seal()?;
        let history_entry = DraftDirectoryHistoryEntryV3::new(String::new(), document.clone())?;
        let document_bytes = serde_json::to_vec(&document)
            .map_err(|source| invalid(format!("directory genesis cannot encode: {source}")))?;
        if document_bytes.len() > MAX_DRAFT_DIRECTORY_V3_BYTES {
            return Err(invalid("directory genesis exceeds its byte bound"));
        }
        let history_entry_bytes = history_entry.encode_canonical()?;
        let assignment_root =
            hash_directory_genesis(MIGRATION_ASSIGNMENT_ROOT_DOMAIN, &document.assignments)?;
        let placement_root =
            hash_directory_genesis(MIGRATION_PLACEMENT_ROOT_DOMAIN, &document.placements)?;
        Ok(Self {
            transform,
            document,
            history_entry,
            document_bytes,
            history_entry_bytes,
            assignment_root,
            placement_root,
        })
    }

    pub(crate) const fn directory_revision(&self) -> u64 {
        self.document.directory_revision
    }

    pub(crate) fn document_hash(&self) -> &str {
        &self.document.document_hash
    }

    pub(crate) fn history_entry_hash(&self) -> &str {
        &self.history_entry.entry_hash
    }

    pub(crate) fn assignment_root(&self) -> &str {
        &self.assignment_root
    }

    pub(crate) fn placement_root(&self) -> &str {
        &self.placement_root
    }

    pub(crate) fn document_bytes(&self) -> &[u8] {
        &self.document_bytes
    }

    pub(crate) fn history_entry_bytes(&self) -> &[u8] {
        &self.history_entry_bytes
    }

    pub(crate) fn validate_for_transform(
        &self,
        transform: &'migration crate::grid_handoff_v2::migration_transform::ValidatedProtocol19MigrationTransform<'source>,
    ) -> Result<(), CellDirectoryError> {
        let expected = Self::derive(transform)?;
        if !std::ptr::eq(self.transform, transform)
            || self.document != expected.document
            || self.history_entry != expected.history_entry
            || self.document_bytes != expected.document_bytes
            || self.history_entry_bytes != expected.history_entry_bytes
            || self.assignment_root != expected.assignment_root
            || self.placement_root != expected.placement_root
        {
            return Err(invalid(
                "directory-v3 genesis belongs to another migration transform",
            ));
        }
        Ok(())
    }
}

fn hash_directory_genesis<T: Serialize>(
    domain: &[u8],
    value: &T,
) -> Result<String, CellDirectoryError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| invalid(format!("directory genesis root cannot encode: {source}")))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DraftDirectoryHistoryFailpointV3 {
    BeforeJournalWrite,
    AfterPartialJournalWrite,
    AfterJournalLineWriteBeforeSync,
    AfterJournalSyncBeforeHead,
    AfterHeadTempSyncBeforeRename,
    AfterHeadRenameBeforeDirectorySync,
    AfterHeadDirectorySyncBeforeMemory,
}

#[cfg(test)]
pub(crate) struct DraftDirectoryV3AuthoritySeed {
    pub(crate) universe_id: String,
    pub(crate) universe_manifest_hash: String,
    pub(crate) transfer_id: String,
    pub(crate) root_aggregate_id: String,
    pub(crate) source_cell_key: CellKeyV1,
    pub(crate) destination_cell_key: CellKeyV1,
    pub(crate) source_assignment_generation: u64,
    pub(crate) source_fencing_token: u64,
    pub(crate) destination_assignment_generation: u64,
    pub(crate) destination_fencing_token: u64,
    pub(crate) package_schema_version: u32,
    pub(crate) receipt_schema_version: u32,
    pub(crate) closure_root: String,
    pub(crate) conservation_root: String,
    pub(crate) package_hash: String,
    pub(crate) members: Vec<BundledPlacementMember>,
    pub(crate) member_root: String,
}

#[cfg(test)]
pub(crate) struct DraftDirectoryV3AuthorityHarness {
    document: CellDirectoryDocumentV3,
    requested: CellTransferRecordV3,
    history: Vec<CellDirectoryDocumentV3>,
}

/// Isolated protocol-19 history store. Activated construction requires the
/// exact signed genesis and retains its writer lock for the store lifetime.
#[derive(Debug)]
pub(super) struct DraftCellDirectoryHistoryStoreV3 {
    root: PathBuf,
    lock_file: File,
    history_file: File,
    head: DraftDirectoryHistoryHeadV3,
    current: Option<CellDirectoryDocumentV3>,
    index: BTreeMap<u64, DraftDirectoryHistoryLocationV3>,
    poisoned: bool,
    #[cfg(test)]
    failpoint: Option<DraftDirectoryHistoryFailpointV3>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ActiveDirectoryV3Expectation<'a> {
    pub(crate) universe_id: &'a str,
    pub(crate) manifest_hash: &'a str,
    pub(crate) revision: u64,
    pub(crate) document_hash: &'a str,
    pub(crate) history_entry_hash: &'a str,
    pub(crate) assignment_root: &'a str,
    pub(crate) placement_root: &'a str,
    pub(crate) document_bytes: &'a [u8],
    pub(crate) history_entry_bytes: &'a [u8],
}

impl DraftCellDirectoryHistoryStoreV3 {
    fn open_or_initialize(
        base_root: impl AsRef<Path>,
        initial: CellDirectoryDocumentV3,
    ) -> Result<Self, CellDirectoryError> {
        initial.validate()?;
        let base_root = base_root.as_ref();
        fs::create_dir_all(base_root).map_err(|source| io_error_v3(base_root, source))?;
        let root = base_root.join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        fs::create_dir_all(&root).map_err(|source| io_error_v3(&root, source))?;
        File::open(base_root)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error_v3(base_root, source))?;
        let lock_file = lock_history_writer_v3(&root, true)?;
        let head_path = root.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        let history_path = root.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let history_file = match OpenOptions::new()
            .read(true)
            .append(true)
            .open(&history_path)
        {
            Ok(file) => file,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                let file = OpenOptions::new()
                    .create_new(true)
                    .read(true)
                    .append(true)
                    .open(&history_path)
                    .map_err(|source| io_error_v3(&history_path, source))?;
                file.sync_all()
                    .map_err(|source| io_error_v3(&history_path, source))?;
                File::open(&root)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|source| io_error_v3(&root, source))?;
                file
            }
            Err(source) => return Err(io_error_v3(&history_path, source)),
        };
        let history_is_empty = history_file
            .metadata()
            .map_err(|source| io_error_v3(&history_path, source))?
            .len()
            == 0;
        let head = if head_path.exists() {
            let head = read_history_head_v3(&head_path)?;
            head.validate_identity(&initial.universe_id, &initial.universe_manifest_hash)?;
            head
        } else if history_is_empty {
            DraftDirectoryHistoryHeadV3::empty(&initial)
        } else {
            return Err(invalid(
                "v3 directory history has records but no durable head",
            ));
        };
        let head_existed = head_path.exists();
        let mut store = Self {
            root,
            lock_file,
            history_file,
            head,
            current: None,
            index: BTreeMap::new(),
            poisoned: false,
            #[cfg(test)]
            failpoint: None,
        };
        if !head_existed {
            let empty_head = store.head.clone();
            store.persist_head(&empty_head)?;
        }
        store.recover()?;
        if store.current.is_none() {
            store.append_document(None, None, initial)?;
        } else if store.index.first_key_value().map(|(revision, _)| *revision)
            != Some(initial.directory_revision)
            || store.resolve_document(initial.directory_revision, &initial.document_hash)?
                != initial
        {
            return Err(invalid(
                "v3 directory history genesis differs from initialization input",
            ));
        }
        Ok(store)
    }

    fn open(
        base_root: impl AsRef<Path>,
        expected_universe_id: &str,
        expected_manifest_hash: &str,
    ) -> Result<Self, CellDirectoryError> {
        let root = base_root
            .as_ref()
            .join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        if !root.is_dir() {
            return Err(invalid("v3 directory history does not exist"));
        }
        let lock_file = lock_history_writer_v3(&root, false)?;
        let head_path = root.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        let history_path = root.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let head = read_history_head_v3(&head_path)?;
        head.validate_identity(expected_universe_id, expected_manifest_hash)?;
        let history_file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&history_path)
            .map_err(|source| io_error_v3(&history_path, source))?;
        let mut store = Self {
            root,
            lock_file,
            history_file,
            head,
            current: None,
            index: BTreeMap::new(),
            poisoned: false,
            #[cfg(test)]
            failpoint: None,
        };
        store.recover()?;
        store.current()?;
        Ok(store)
    }

    fn current(&self) -> Result<&CellDirectoryDocumentV3, CellDirectoryError> {
        self.current
            .as_ref()
            .ok_or_else(|| invalid("v3 directory history has no genesis document"))
    }

    pub(crate) fn stage_genesis(
        base_root: impl AsRef<Path>,
        genesis: &ValidatedProtocol19DirectoryGenesis<'_, '_>,
    ) -> Result<Self, CellDirectoryError> {
        Self::open_or_initialize(base_root, genesis.document.clone())
    }

    pub(crate) fn open_genesis(
        base_root: impl AsRef<Path>,
        genesis: &ValidatedProtocol19DirectoryGenesis<'_, '_>,
    ) -> Result<Self, CellDirectoryError> {
        let root = base_root
            .as_ref()
            .join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        let metadata = fs::symlink_metadata(&root).map_err(|source| io_error_v3(&root, source))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid("v3 directory namespace is not a real directory"));
        }
        let lock_file = lock_history_writer_v3(&root, false)?;
        let head_path = root.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        let history_path = root.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let head = read_history_head_v3(&head_path)?;
        head.validate_identity(
            &genesis.document.universe_id,
            &genesis.document.universe_manifest_hash,
        )?;
        let mut expected_history = genesis.history_entry_bytes.clone();
        expected_history.push(b'\n');
        let history_length = fs::metadata(&history_path)
            .map_err(|source| io_error_v3(&history_path, source))?
            .len();
        if history_length
            != u64::try_from(expected_history.len())
                .map_err(|_| invalid("v3 directory genesis length overflowed"))?
            || history_length > MAX_DRAFT_DIRECTORY_HISTORY_BYTES
            || fs::read(&history_path).map_err(|source| io_error_v3(&history_path, source))?
                != expected_history
        {
            return Err(invalid(
                "persisted directory-v3 genesis differs from the migration commitment",
            ));
        }
        let expected_head = DraftDirectoryHistoryHeadV3::from_tip(
            &DraftDirectoryHistoryHeadV3::empty(&genesis.document),
            &genesis.history_entry,
            history_length,
        )?;
        if head != expected_head {
            return Err(invalid(
                "persisted directory-v3 head differs from the migration commitment",
            ));
        }
        let history_file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&history_path)
            .map_err(|source| io_error_v3(&history_path, source))?;
        let index = BTreeMap::from([(
            genesis.document.directory_revision,
            DraftDirectoryHistoryLocationV3 {
                offset: 0,
                line_length: genesis.history_entry_bytes.len(),
                document_hash: genesis.document.document_hash.clone(),
                entry_hash: genesis.history_entry.entry_hash.clone(),
            },
        )]);
        Ok(Self {
            root,
            lock_file,
            history_file,
            head,
            current: Some(genesis.document.clone()),
            index,
            poisoned: false,
            #[cfg(test)]
            failpoint: None,
        })
    }

    pub(crate) fn open_from_active_head(
        base_root: impl AsRef<Path>,
        expected: ActiveDirectoryV3Expectation<'_>,
    ) -> Result<Self, CellDirectoryError> {
        let base_root = base_root.as_ref();
        let root = base_root.join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        let metadata = fs::symlink_metadata(&root).map_err(|source| io_error_v3(&root, source))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid(
                "active head selects a non-directory directory-v3 namespace",
            ));
        }
        let history_path = root.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let history_entry =
            DraftDirectoryHistoryEntryV3::decode_canonical(expected.history_entry_bytes)?;
        let genesis = history_entry.document.clone();
        genesis.validate()?;
        let canonical_document = serde_json::to_vec(&genesis)
            .map_err(|source| invalid(format!("directory-v3 document cannot encode: {source}")))?;
        let assignment_root =
            hash_directory_genesis(MIGRATION_ASSIGNMENT_ROOT_DOMAIN, &genesis.assignments)?;
        let placement_root =
            hash_directory_genesis(MIGRATION_PLACEMENT_ROOT_DOMAIN, &genesis.placements)?;
        if !history_entry.previous_entry_hash.is_empty()
            || genesis.directory_revision != expected.revision
            || genesis.document_hash != expected.document_hash
            || assignment_root != expected.assignment_root
            || placement_root != expected.placement_root
            || canonical_document != expected.document_bytes
            || history_entry.document != genesis
            || history_entry.entry_hash != expected.history_entry_hash
        {
            return Err(invalid(
                "directory-v3 genesis differs from the exact active-head commitment",
            ));
        }

        // The signed activation head anchors revision one, not a permanently
        // frozen directory tip. Prove the durable prefix before recovery is
        // allowed to truncate or advance either artifact, then recover the
        // complete hash-chained successor history.
        let lock_file = lock_history_writer_v3(&root, false)?;
        let head_path = root.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        let head = read_history_head_v3(&head_path)?;
        head.validate_identity(expected.universe_id, expected.manifest_hash)?;
        let mut expected_prefix = expected.history_entry_bytes.to_vec();
        expected_prefix.push(b'\n');
        let expected_prefix_length = u64::try_from(expected_prefix.len())
            .map_err(|_| invalid("active directory-v3 genesis length overflowed"))?;
        let history_length = fs::metadata(&history_path)
            .map_err(|source| io_error_v3(&history_path, source))?
            .len();
        if history_length > MAX_DRAFT_DIRECTORY_HISTORY_BYTES
            || head.entry_count == 0
            || head.journal_byte_length < expected_prefix_length
            || history_length < head.journal_byte_length
        {
            return Err(invalid(
                "active directory-v3 head does not pin the selected genesis prefix",
            ));
        }
        let history_file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&history_path)
            .map_err(|source| io_error_v3(&history_path, source))?;
        let mut persisted_prefix = vec![0; expected_prefix.len()];
        File::open(&history_path)
            .and_then(|mut file| file.read_exact(&mut persisted_prefix))
            .map_err(|source| io_error_v3(&history_path, source))?;
        if persisted_prefix != expected_prefix {
            return Err(invalid(
                "directory-v3 history does not begin with the exact active-head genesis",
            ));
        }
        remove_stale_history_head_temps_v3(&root)?;
        let mut store = Self {
            root,
            lock_file,
            history_file,
            head,
            current: None,
            index: BTreeMap::new(),
            poisoned: false,
            #[cfg(test)]
            failpoint: None,
        };
        store.recover()?;
        store.current()?;
        let location = store.index.get(&expected.revision).ok_or_else(|| {
            invalid("active directory-v3 history has no selected genesis revision")
        })?;
        let first_revision = store.index.first_key_value().map(|(revision, _)| *revision);
        let persisted_genesis =
            store.resolve_document(expected.revision, expected.document_hash)?;
        let mut persisted_entry = vec![0; location.line_length];
        File::open(&history_path)
            .and_then(|mut file| {
                file.seek(SeekFrom::Start(location.offset))?;
                file.read_exact(&mut persisted_entry)
            })
            .map_err(|source| io_error_v3(&history_path, source))?;
        if first_revision != Some(expected.revision)
            || location.offset != 0
            || location.entry_hash != expected.history_entry_hash
            || location.document_hash != expected.document_hash
            || persisted_entry != expected.history_entry_bytes
            || persisted_genesis != genesis
        {
            return Err(invalid(
                "directory-v3 history does not descend from the exact active-head genesis",
            ));
        }
        store.validate_genesis_file_set()?;
        Ok(store)
    }

    pub(crate) fn assignment(
        &self,
        cell_key: &CellKeyV1,
    ) -> Result<&CellAssignmentRecord, CellDirectoryError> {
        let cell_id = celestial::cell_id(cell_key).map_err(|source| invalid(source.to_string()))?;
        self.current()?
            .assignments
            .get(&cell_id)
            .ok_or(CellDirectoryError::UnknownCell(cell_id))
    }

    /// Claims one sleeping cell under the already-held directory and cell
    /// writer locks. The successor generation and fence are derived from the
    /// durable tip; callers cannot choose either authority value.
    pub(crate) fn claim_cell(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        self.transition_cell_authority(
            cell_key,
            expected_generation,
            holder_id,
            CellAuthorityTransitionV3::ClaimSleeping,
        )
    }

    /// Replaces an assigned holder after exclusive writer-lock acquisition.
    /// A replacement always advances both the generation and fencing token.
    pub(crate) fn recover_cell(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        self.transition_cell_authority(
            cell_key,
            expected_generation,
            holder_id,
            CellAuthorityTransitionV3::RecoverAssigned,
        )
    }

    pub(crate) fn release_cell(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        validate_stable_id(holder_id, "assignment holder")?;
        let cell_id = celestial::cell_id(cell_key).map_err(|source| invalid(source.to_string()))?;
        let current = self.current()?.clone();
        let assignment = current
            .assignments
            .get(&cell_id)
            .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.clone()))?;
        if assignment.state == CellAssignmentState::Sleeping
            && assignment.assignment_generation == expected_generation
            && assignment.holder_id.is_none()
        {
            if self
                .release_transition_predecessor(&cell_id, expected_generation)?
                .is_some_and(|prior| prior.holder_id.as_deref() == Some(holder_id))
            {
                return Ok(assignment.clone());
            }
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "the sleeping generation was not released by this holder".into(),
            });
        }
        if assignment.state != CellAssignmentState::Assigned
            || assignment.assignment_generation != expected_generation
            || assignment.holder_id.as_deref() != Some(holder_id)
        {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "generation, state, or holder no longer matches".into(),
            });
        }
        if current.transfers.values().any(|transfer| {
            !matches!(
                transfer.phase,
                TransferPhase::Finalized | TransferPhase::Aborted
            ) && (transfer.source_cell_id == assignment.cell_id
                || transfer.destination_cell_id == assignment.cell_id)
        }) {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "cell assignment is pinned by a nonterminal transfer".into(),
            });
        }
        let mut next = current.clone();
        let released = next
            .assignments
            .get_mut(&assignment.cell_id)
            .expect("validated assignment exists in cloned directory");
        released.state = CellAssignmentState::Sleeping;
        released.holder_id = None;
        let next = finish_v3_transaction(&current, next)?;
        self.commit(current.directory_revision, &current.document_hash, next)?;
        Ok(self
            .current()?
            .assignments
            .get(&assignment.cell_id)
            .expect("committed assignment exists")
            .clone())
    }

    fn transition_cell_authority(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
        transition: CellAuthorityTransitionV3,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        validate_stable_id(holder_id, "assignment holder")?;
        let cell_id = celestial::cell_id(cell_key).map_err(|source| invalid(source.to_string()))?;
        let current = self.current()?.clone();
        let assignment = current
            .assignments
            .get(&cell_id)
            .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.clone()))?;
        let resulting_generation = expected_generation
            .checked_add(1)
            .ok_or_else(|| CellDirectoryError::AssignmentGenerationExhausted(cell_id.clone()))?;

        // Exact redelivery after an uncertain commit is a no-op. The history
        // proves that this authority is precisely the one successor derived
        // from the caller's expected generation.
        let expected_predecessor_state = match transition {
            CellAuthorityTransitionV3::ClaimSleeping => CellAssignmentState::Sleeping,
            CellAuthorityTransitionV3::RecoverAssigned => CellAssignmentState::Assigned,
        };
        if assignment.state == CellAssignmentState::Assigned
            && assignment.assignment_generation == resulting_generation
            && assignment.holder_id.as_deref() == Some(holder_id)
            && assignment
                .fencing_history
                .get(&expected_generation)
                .and_then(|prior| prior.checked_add(1))
                == Some(assignment.authority_fencing_token)
        {
            if self
                .authority_transition_predecessor(
                    &cell_id,
                    expected_generation,
                    resulting_generation,
                )?
                .map(|prior| prior.state)
                == Some(expected_predecessor_state)
            {
                return Ok(assignment.clone());
            }
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "the committed successor belongs to another authority transition".into(),
            });
        }

        if assignment.assignment_generation != expected_generation
            || assignment.state != expected_predecessor_state
            || (expected_predecessor_state == CellAssignmentState::Sleeping
                && assignment.holder_id.is_some())
        {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: match transition {
                    CellAuthorityTransitionV3::ClaimSleeping => {
                        "expected the current sleeping generation without a holder"
                    }
                    CellAuthorityTransitionV3::RecoverAssigned => {
                        "only the current assigned generation may be recovered"
                    }
                }
                .into(),
            });
        }
        let next_fence = assignment
            .authority_fencing_token
            .checked_add(1)
            .ok_or_else(|| invalid("cell authority fencing token is exhausted"))?;
        let mut next = current.clone();
        let claimed = next
            .assignments
            .get_mut(&assignment.cell_id)
            .expect("validated assignment exists in cloned directory");
        claimed.assignment_generation = resulting_generation;
        claimed.authority_fencing_token = next_fence;
        claimed
            .fencing_history
            .insert(resulting_generation, next_fence);
        claimed.state = CellAssignmentState::Assigned;
        claimed.holder_id = Some(holder_id.to_owned());
        let next = finish_v3_transaction(&current, next)?;
        self.commit(current.directory_revision, &current.document_hash, next)?;
        Ok(self
            .current()?
            .assignments
            .get(&assignment.cell_id)
            .expect("committed assignment exists")
            .clone())
    }

    fn authority_transition_predecessor(
        &self,
        cell_id: &str,
        expected_generation: u64,
        resulting_generation: u64,
    ) -> Result<Option<CellAssignmentRecord>, CellDirectoryError> {
        let mut prior_assignment: Option<CellAssignmentRecord> = None;
        for (&revision, location) in &self.index {
            let document = self.resolve_document(revision, &location.document_hash)?;
            let assignment = document
                .assignments
                .get(cell_id)
                .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.to_owned()))?;
            if assignment.assignment_generation == resulting_generation {
                return Ok(prior_assignment.and_then(|prior| {
                    (prior.assignment_generation == expected_generation).then_some(prior)
                }));
            }
            if assignment.assignment_generation > resulting_generation {
                return Ok(None);
            }
            prior_assignment = Some(assignment.clone());
        }
        Ok(None)
    }

    fn release_transition_predecessor(
        &self,
        cell_id: &str,
        generation: u64,
    ) -> Result<Option<CellAssignmentRecord>, CellDirectoryError> {
        let mut prior_assignment: Option<CellAssignmentRecord> = None;
        for (&revision, location) in &self.index {
            let document = self.resolve_document(revision, &location.document_hash)?;
            let assignment = document
                .assignments
                .get(cell_id)
                .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.to_owned()))?;
            if assignment.assignment_generation == generation
                && assignment.state == CellAssignmentState::Sleeping
                && prior_assignment.as_ref().is_some_and(|prior| {
                    prior.assignment_generation == generation
                        && prior.state == CellAssignmentState::Assigned
                })
            {
                return Ok(prior_assignment);
            }
            if assignment.assignment_generation > generation {
                return Ok(None);
            }
            prior_assignment = Some(assignment.clone());
        }
        Ok(None)
    }

    pub(crate) fn validate_genesis_file_set(&self) -> Result<(), CellDirectoryError> {
        let expected = BTreeSet::from([
            DRAFT_DIRECTORY_HISTORY_LOCK_FILE,
            DRAFT_DIRECTORY_HISTORY_HEAD_FILE,
            DRAFT_DIRECTORY_HISTORY_FILE,
        ]);
        for entry in fs::read_dir(&self.root).map_err(|source| io_error_v3(&self.root, source))? {
            let entry = entry.map_err(|source| io_error_v3(&self.root, source))?;
            let name = entry.file_name();
            let name = name
                .to_str()
                .ok_or_else(|| invalid("v3 directory contains a non-UTF-8 artifact"))?;
            if !entry
                .file_type()
                .map_err(|source| io_error_v3(entry.path(), source))?
                .is_file()
                || !expected.contains(name)
            {
                return Err(invalid(
                    "v3 directory genesis contains an unexpected artifact",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn discard_uncommitted_genesis(
        base_root: impl AsRef<Path>,
    ) -> Result<(), CellDirectoryError> {
        let root = base_root
            .as_ref()
            .join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(io_error_v3(&root, source)),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(invalid("v3 directory namespace is not a real directory"));
        }
        let lock_file = lock_history_writer_v3(&root, true)?;
        fs::remove_dir_all(&root).map_err(|source| io_error_v3(&root, source))?;
        drop(lock_file);
        File::open(base_root.as_ref())
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error_v3(base_root.as_ref(), source))
    }

    fn commit(
        &mut self,
        expected_revision: u64,
        expected_document_hash: &str,
        next: CellDirectoryDocumentV3,
    ) -> Result<(), CellDirectoryError> {
        self.append_document(Some(expected_revision), Some(expected_document_hash), next)
    }

    fn resolve_document(
        &self,
        directory_revision: u64,
        document_hash: &str,
    ) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
        let location = self.index.get(&directory_revision).ok_or_else(|| {
            invalid(format!(
                "v3 directory history has no revision {directory_revision}"
            ))
        })?;
        if location.document_hash != document_hash {
            return Err(invalid(format!(
                "v3 directory history revision {directory_revision} has a different document hash"
            )));
        }
        let path = self.root.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let mut file = File::open(&path).map_err(|source| io_error_v3(&path, source))?;
        file.seek(SeekFrom::Start(location.offset))
            .map_err(|source| io_error_v3(&path, source))?;
        let mut bytes = vec![0; location.line_length];
        file.read_exact(&mut bytes)
            .map_err(|source| io_error_v3(&path, source))?;
        let entry = DraftDirectoryHistoryEntryV3::decode_canonical(&bytes)?;
        if entry.document.directory_revision != directory_revision
            || entry.document.document_hash != location.document_hash
            || entry.entry_hash != location.entry_hash
        {
            return Err(invalid(
                "v3 directory history record changed after validated open",
            ));
        }
        Ok(entry.document)
    }

    pub(super) fn resolve_historical_grid_authority(
        &self,
        directory_revision: u64,
        document_hash: &str,
        transfer_id: &str,
    ) -> Result<ValidatedGridTransferAuthorityV3, CellDirectoryError> {
        self.resolve_document(directory_revision, document_hash)?
            .validated_grid_transfer_authority(transfer_id)
    }

    pub(super) fn resolve_historical_cell_authority(
        &self,
        directory_revision: u64,
        document_hash: &str,
        cell_id: &str,
    ) -> Result<ValidatedCellAuthorityV3, CellDirectoryError> {
        self.resolve_document(directory_revision, document_hash)?
            .validated_cell_authority(cell_id)
    }

    /// Borrows the locked directory store so the current head cannot advance
    /// while a live grid capability is in use. There is deliberately no
    /// historical-to-current conversion.
    pub(super) fn current_grid_authority(
        &self,
        transfer_id: &str,
    ) -> Result<ValidatedCurrentGridAuthorityV3<'_>, CellDirectoryError> {
        Ok(ValidatedCurrentGridAuthorityV3 {
            authority: self
                .current()?
                .validated_grid_transfer_authority(transfer_id)?,
            _store_guard: PhantomData,
        })
    }

    /// Borrows the locked directory store so the current head cannot advance
    /// while a live cell capability is in use.
    pub(super) fn current_cell_authority(
        &self,
        cell_id: &str,
    ) -> Result<ValidatedCurrentCellAuthorityV3<'_>, CellDirectoryError> {
        Ok(ValidatedCurrentCellAuthorityV3 {
            authority: self.current()?.validated_cell_authority(cell_id)?,
            _store_guard: PhantomData,
        })
    }

    fn append_document(
        &mut self,
        expected_revision: Option<u64>,
        expected_document_hash: Option<&str>,
        next: CellDirectoryDocumentV3,
    ) -> Result<(), CellDirectoryError> {
        if self.poisoned {
            return Err(invalid(
                "v3 directory history write outcome is uncertain; reopen before retry",
            ));
        }
        next.validate()?;
        match self.current.as_ref() {
            None => {
                if expected_revision.is_some()
                    || expected_document_hash.is_some()
                    || self.head.entry_count != 0
                    || !self.index.is_empty()
                {
                    return Err(invalid("v3 directory history genesis CAS is invalid"));
                }
            }
            Some(current) => {
                if expected_revision != Some(current.directory_revision)
                    || expected_document_hash != Some(current.document_hash.as_str())
                {
                    return Err(invalid("v3 directory history compare-and-swap is stale"));
                }
                if next == *current {
                    return Ok(());
                }
                if next.directory_revision
                    != current
                        .directory_revision
                        .checked_add(1)
                        .ok_or(CellDirectoryError::DirectoryRevisionExhausted)?
                    || next.universe_id != current.universe_id
                    || next.universe_manifest_hash != current.universe_manifest_hash
                    || next.migration_genesis != current.migration_genesis
                {
                    return Err(invalid(
                        "v3 directory history successor revision or identity is invalid",
                    ));
                }
            }
        }
        if next.universe_id != self.head.universe_id
            || next.universe_manifest_hash != self.head.universe_manifest_hash
        {
            return Err(invalid(
                "v3 directory history document disagrees with the pinned identity",
            ));
        }

        #[cfg(test)]
        if self.consume_failpoint(DraftDirectoryHistoryFailpointV3::BeforeJournalWrite) {
            return Err(invalid("injected failure before v3 history journal write"));
        }

        let previous_entry_hash = if self.head.entry_count == 0 {
            String::new()
        } else {
            self.head.entry_hash.clone()
        };
        let entry = DraftDirectoryHistoryEntryV3::new(previous_entry_hash, next)?;
        let line = entry.encode_canonical()?;
        let offset = self
            .history_file
            .metadata()
            .map_err(|source| io_error_v3(self.root.join(DRAFT_DIRECTORY_HISTORY_FILE), source))?
            .len();
        if offset != self.head.journal_byte_length {
            return Err(invalid(
                "v3 directory history journal and in-memory head diverged",
            ));
        }
        let record_length = line
            .len()
            .checked_add(1)
            .ok_or_else(|| invalid("v3 directory history record length overflowed"))?;
        let resulting_length = offset
            .checked_add(
                u64::try_from(record_length)
                    .map_err(|_| invalid("v3 directory history record is too large"))?,
            )
            .ok_or_else(|| invalid("v3 directory history byte length overflowed"))?;
        if resulting_length > MAX_DRAFT_DIRECTORY_HISTORY_BYTES {
            return Err(invalid("v3 directory history exceeds its byte bound"));
        }

        #[cfg(test)]
        if self.consume_failpoint(DraftDirectoryHistoryFailpointV3::AfterPartialJournalWrite) {
            let partial_length = line.len().div_ceil(2).max(1);
            let result = self
                .history_file
                .write_all(&line[..partial_length])
                .and_then(|()| self.history_file.sync_data());
            self.poisoned = true;
            result.map_err(|source| {
                io_error_v3(self.root.join(DRAFT_DIRECTORY_HISTORY_FILE), source)
            })?;
            return Err(invalid("injected partial v3 history journal write"));
        }

        if let Err(source) = self
            .history_file
            .write_all(&line)
            .and_then(|()| self.history_file.write_all(b"\n"))
        {
            self.poisoned = true;
            return Err(io_error_v3(
                self.root.join(DRAFT_DIRECTORY_HISTORY_FILE),
                source,
            ));
        }
        #[cfg(test)]
        if self.consume_failpoint(DraftDirectoryHistoryFailpointV3::AfterJournalLineWriteBeforeSync)
        {
            self.poisoned = true;
            return Err(invalid(
                "injected failure after v3 history journal line write",
            ));
        }
        if let Err(source) = self.history_file.sync_data() {
            self.poisoned = true;
            return Err(io_error_v3(
                self.root.join(DRAFT_DIRECTORY_HISTORY_FILE),
                source,
            ));
        }
        #[cfg(test)]
        if self.consume_failpoint(DraftDirectoryHistoryFailpointV3::AfterJournalSyncBeforeHead) {
            self.poisoned = true;
            return Err(invalid("injected failure after v3 history journal sync"));
        }

        let next_head =
            DraftDirectoryHistoryHeadV3::from_tip(&self.head, &entry, resulting_length)?;
        if let Err(error) = self.persist_head(&next_head) {
            self.poisoned = true;
            return Err(error);
        }
        self.index.insert(
            entry.document.directory_revision,
            DraftDirectoryHistoryLocationV3 {
                offset,
                line_length: line.len(),
                document_hash: entry.document.document_hash.clone(),
                entry_hash: entry.entry_hash.clone(),
            },
        );
        self.current = Some(entry.document);
        self.head = next_head;
        Ok(())
    }

    fn recover(&mut self) -> Result<(), CellDirectoryError> {
        let history_path = self.root.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let file_length = fs::metadata(&history_path)
            .map_err(|source| io_error_v3(&history_path, source))?
            .len();
        if file_length > MAX_DRAFT_DIRECTORY_HISTORY_BYTES {
            return Err(invalid("v3 directory history exceeds its byte bound"));
        }
        if file_length < self.head.journal_byte_length {
            return Err(invalid(
                "v3 directory history is shorter than its pinned head",
            ));
        }

        let read_file =
            File::open(&history_path).map_err(|source| io_error_v3(&history_path, source))?;
        let mut reader = BufReader::new(read_file);
        let mut offset = 0_u64;
        let mut prior_revision: Option<u64> = None;
        let mut prior_entry_hash = String::new();
        let mut derived_head = DraftDirectoryHistoryHeadV3 {
            history_schema_version: DRAFT_DIRECTORY_HISTORY_SCHEMA_VERSION,
            compatibility: DraftGridCompatibilityTupleV19::canonical(),
            universe_id: self.head.universe_id.clone(),
            universe_manifest_hash: self.head.universe_manifest_hash.clone(),
            entry_count: 0,
            directory_revision: 0,
            document_hash: String::new(),
            entry_hash: String::new(),
            journal_byte_length: 0,
        };
        let mut index = BTreeMap::new();
        let mut current = None;
        let mut head_matched = self.head.entry_count == 0 && self.head.journal_byte_length == 0;
        let mut pinned_migration_genesis = None;

        loop {
            let start = offset;
            let mut record = Vec::new();
            let read = (&mut reader)
                .take(
                    u64::try_from(MAX_DRAFT_DIRECTORY_HISTORY_LINE_BYTES)
                        .expect("history line bound fits u64")
                        + 2,
                )
                .read_until(b'\n', &mut record)
                .map_err(|source| io_error_v3(&history_path, source))?;
            if read == 0 {
                break;
            }
            if read > MAX_DRAFT_DIRECTORY_HISTORY_LINE_BYTES + 1 {
                return Err(invalid("v3 directory history line exceeds its byte bound"));
            }
            if record.last() != Some(&b'\n') {
                if start < self.head.journal_byte_length {
                    return Err(invalid(
                        "v3 directory history has a torn record inside its pinned prefix",
                    ));
                }
                self.history_file
                    .set_len(start)
                    .and_then(|()| self.history_file.sync_data())
                    .map_err(|source| io_error_v3(&history_path, source))?;
                break;
            }
            record.pop();
            let entry = DraftDirectoryHistoryEntryV3::decode_canonical(&record)?;
            if entry.previous_entry_hash != prior_entry_hash
                || entry.document.universe_id != self.head.universe_id
                || entry.document.universe_manifest_hash != self.head.universe_manifest_hash
                || (prior_revision.is_none()
                    && entry.document.migration_genesis.is_some()
                    && entry.document.directory_revision != 1)
                || prior_revision.is_some_and(|revision| {
                    revision.checked_add(1) != Some(entry.document.directory_revision)
                })
            {
                return Err(invalid(
                    "v3 directory history chain, revision, or identity is invalid",
                ));
            }
            if let Some(genesis) = &pinned_migration_genesis {
                if genesis != &entry.document.migration_genesis {
                    return Err(invalid(
                        "v3 directory history changed its migration genesis",
                    ));
                }
            } else {
                pinned_migration_genesis = Some(entry.document.migration_genesis.clone());
            }
            offset = start
                .checked_add(
                    u64::try_from(read)
                        .map_err(|_| invalid("v3 directory history offset overflowed"))?,
                )
                .ok_or_else(|| invalid("v3 directory history offset overflowed"))?;
            derived_head = DraftDirectoryHistoryHeadV3::from_tip(&derived_head, &entry, offset)?;
            if offset == self.head.journal_byte_length {
                if derived_head != self.head {
                    return Err(invalid(
                        "v3 directory history head does not identify its exact record",
                    ));
                }
                head_matched = true;
            } else if start < self.head.journal_byte_length
                && offset > self.head.journal_byte_length
            {
                return Err(invalid(
                    "v3 directory history head is not on a record boundary",
                ));
            }
            index.insert(
                entry.document.directory_revision,
                DraftDirectoryHistoryLocationV3 {
                    offset: start,
                    line_length: record.len(),
                    document_hash: entry.document.document_hash.clone(),
                    entry_hash: entry.entry_hash.clone(),
                },
            );
            prior_revision = Some(entry.document.directory_revision);
            prior_entry_hash.clone_from(&entry.entry_hash);
            current = Some(entry.document);
        }
        if !head_matched {
            return Err(invalid(
                "v3 directory history cannot resolve its pinned head",
            ));
        }
        if derived_head != self.head {
            self.persist_head(&derived_head)?;
        }
        self.head = derived_head;
        self.current = current;
        self.index = index;
        self.poisoned = false;
        Ok(())
    }

    fn persist_head(
        &mut self,
        head: &DraftDirectoryHistoryHeadV3,
    ) -> Result<(), CellDirectoryError> {
        head.validate_identity(&self.head.universe_id, &self.head.universe_manifest_hash)?;
        let head_path = self.root.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        let bytes = serde_json::to_vec_pretty(head).map_err(|source| CellDirectoryError::Json {
            path: head_path.clone(),
            source,
        })?;
        if bytes.is_empty()
            || u64::try_from(bytes.len()).unwrap_or(u64::MAX)
                > MAX_DRAFT_DIRECTORY_HISTORY_HEAD_BYTES
        {
            return Err(invalid("v3 directory history head exceeds its byte bound"));
        }
        let temp_path = self.root.join(format!(
            ".{DRAFT_DIRECTORY_HISTORY_HEAD_FILE}.tmp-{}-{}",
            std::process::id(),
            Uuid::new_v4()
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp_path)
                .map_err(|source| io_error_v3(&temp_path, source))?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|source| io_error_v3(&temp_path, source))?;
            #[cfg(test)]
            if self
                .consume_failpoint(DraftDirectoryHistoryFailpointV3::AfterHeadTempSyncBeforeRename)
            {
                return Err(invalid("injected failure before v3 history head rename"));
            }
            fs::rename(&temp_path, &head_path).map_err(|source| io_error_v3(&head_path, source))?;
            #[cfg(test)]
            if self.consume_failpoint(
                DraftDirectoryHistoryFailpointV3::AfterHeadRenameBeforeDirectorySync,
            ) {
                return Err(invalid("injected failure after v3 history head rename"));
            }
            File::open(&self.root)
                .and_then(|directory| directory.sync_all())
                .map_err(|source| io_error_v3(&self.root, source))?;
            #[cfg(test)]
            if self.consume_failpoint(
                DraftDirectoryHistoryFailpointV3::AfterHeadDirectorySyncBeforeMemory,
            ) {
                return Err(invalid(
                    "injected failure after v3 history head directory sync",
                ));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp_path);
        }
        result
    }

    #[cfg(test)]
    fn set_failpoint(&mut self, failpoint: DraftDirectoryHistoryFailpointV3) {
        self.failpoint = Some(failpoint);
    }

    #[cfg(test)]
    fn consume_failpoint(&mut self, failpoint: DraftDirectoryHistoryFailpointV3) -> bool {
        if self.failpoint == Some(failpoint) {
            self.failpoint = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
impl DraftDirectoryV3AuthorityHarness {
    pub(crate) fn new(seed: DraftDirectoryV3AuthoritySeed) -> Result<Self, CellDirectoryError> {
        fn assignment(
            cell_key: CellKeyV1,
            generation: u64,
            fence: u64,
            holder: &str,
        ) -> Result<CellAssignmentRecord, CellDirectoryError> {
            let first_fence = fence
                .checked_sub(generation.saturating_sub(1))
                .ok_or_else(|| invalid("test v3 authority seed has impossible fence history"))?;
            let cell_id =
                celestial::cell_id(&cell_key).map_err(|source| invalid(source.to_string()))?;
            Ok(CellAssignmentRecord {
                cell_key,
                cell_id,
                assignment_generation: generation,
                authority_fencing_token: fence,
                fencing_history: (1..=generation)
                    .map(|generation| (generation, first_fence + generation - 1))
                    .collect(),
                state: CellAssignmentState::Assigned,
                holder_id: Some(holder.into()),
            })
        }

        let source_cell_id = celestial::cell_id(&seed.source_cell_key)
            .map_err(|source| invalid(source.to_string()))?;
        let destination_cell_id = celestial::cell_id(&seed.destination_cell_key)
            .map_err(|source| invalid(source.to_string()))?;
        let source_assignment = assignment(
            seed.source_cell_key.clone(),
            seed.source_assignment_generation,
            seed.source_fencing_token,
            "test-source-holder",
        )?;
        let destination_assignment = assignment(
            seed.destination_cell_key.clone(),
            seed.destination_assignment_generation,
            seed.destination_fencing_token,
            "test-destination-holder",
        )?;
        let placements = seed
            .members
            .iter()
            .map(|member| {
                (
                    member.aggregate_id.clone(),
                    AggregatePlacementRecord {
                        aggregate_id: member.aggregate_id.clone(),
                        aggregate_kind: member.aggregate_kind,
                        cell_key: seed.source_cell_key.clone(),
                        cell_id: source_cell_id.clone(),
                        placement_generation: member.prior_placement_generation,
                        state: AggregatePlacementState::Resident,
                        active_transfer_id: None,
                    },
                )
            })
            .collect();
        let requested = CellTransferRecordV3 {
            transfer_id: seed.transfer_id,
            root_aggregate_id: seed.root_aggregate_id,
            source_cell_key: seed.source_cell_key,
            source_cell_id,
            destination_cell_key: seed.destination_cell_key,
            destination_cell_id,
            source_assignment_generation: seed.source_assignment_generation,
            source_fencing_token: seed.source_fencing_token,
            destination_assignment_generation: seed.destination_assignment_generation,
            destination_fencing_token: seed.destination_fencing_token,
            bundle: DirectoryBundleV3 {
                package_schema_version: seed.package_schema_version,
                receipt_schema_version: seed.receipt_schema_version,
                aggregate_kind: MobileAggregateKind::Grid,
                closure_root: seed.closure_root,
                conservation_root: seed.conservation_root,
                package_hash: seed.package_hash,
                members: seed.members,
                member_root: seed.member_root,
            },
            quarantine_receipt_hash: None,
            source_prepare_proof: None,
            destination_quarantine_proof: None,
            source_export_proof: None,
            import_proof: None,
            destination_activation_proof: None,
            finalization_proof: None,
            source_abort_proof: None,
            destination_abort_proof: None,
            phase: TransferPhase::Prepared,
        };
        let mut document = CellDirectoryDocumentV3 {
            schema_version: DRAFT_CELL_DIRECTORY_V3_SCHEMA_VERSION,
            universe_id: seed.universe_id,
            universe_manifest_hash: seed.universe_manifest_hash,
            directory_revision: 1,
            assignments: BTreeMap::from([
                (source_assignment.cell_id.clone(), source_assignment),
                (
                    destination_assignment.cell_id.clone(),
                    destination_assignment,
                ),
            ]),
            placements,
            transfers: BTreeMap::new(),
            migration_genesis: None,
            document_hash: String::new(),
        };
        document.seal()?;
        Ok(Self {
            history: vec![document.clone()],
            document,
            requested,
        })
    }

    fn retain_current(&mut self) {
        self.history.push(self.document.clone());
    }

    pub(crate) fn prepare(&mut self) -> Result<(), CellDirectoryError> {
        self.document = stage_v3_prepare(&self.document, &self.requested)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn authority(&self) -> Result<ValidatedGridTransferAuthorityV3, CellDirectoryError> {
        self.document
            .validated_grid_transfer_authority(&self.requested.transfer_id)
    }

    pub(crate) fn cell_authority(
        &self,
        cell_id: &str,
    ) -> Result<ValidatedCellAuthorityV3, CellDirectoryError> {
        self.document.validated_cell_authority(cell_id)
    }

    pub(crate) fn advance_cell_authority(
        &mut self,
        cell_id: &str,
    ) -> Result<(), CellDirectoryError> {
        let mut next = self.document.clone();
        let assignment = next
            .assignments
            .get_mut(cell_id)
            .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.to_owned()))?;
        assignment.assignment_generation = assignment
            .assignment_generation
            .checked_add(1)
            .ok_or_else(|| CellDirectoryError::AssignmentGenerationExhausted(cell_id.into()))?;
        assignment.authority_fencing_token = assignment
            .authority_fencing_token
            .checked_add(1)
            .ok_or_else(|| invalid("test v3 authority fence exhausted"))?;
        assignment.fencing_history.insert(
            assignment.assignment_generation,
            assignment.authority_fencing_token,
        );
        self.document = finish_v3_transaction(&self.document, next)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn record_prepare(
        &mut self,
        proof: &DraftGridPrepareProofV2,
    ) -> Result<(), CellDirectoryError> {
        self.document =
            stage_v3_source_prepared(&self.document, &self.requested.transfer_id, proof)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn record_quarantine(
        &mut self,
        proof: &DraftGridQuarantineProofV2,
    ) -> Result<(), CellDirectoryError> {
        self.document = stage_v3_quarantine(&self.document, &self.requested.transfer_id, proof)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn commit_placement(&mut self) -> Result<(), CellDirectoryError> {
        self.document = stage_v3_commit(
            &self.document,
            &self.requested.transfer_id,
            &self.requested.bundle.member_root,
        )?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn record_export(
        &mut self,
        proof: &DraftGridExportProofV2,
    ) -> Result<(), CellDirectoryError> {
        self.document =
            stage_v3_source_exported(&self.document, &self.requested.transfer_id, proof)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn record_import(
        &mut self,
        proof: &DraftGridImportProofV2,
    ) -> Result<(), CellDirectoryError> {
        self.document =
            stage_v3_destination_imported(&self.document, &self.requested.transfer_id, proof)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn record_activation(
        &mut self,
        proof: &DraftGridActivationProofV2,
    ) -> Result<(), CellDirectoryError> {
        self.document =
            stage_v3_destination_activated(&self.document, &self.requested.transfer_id, proof)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn record_finalization(
        &mut self,
        proof: &DraftGridFinalizationProofV2,
    ) -> Result<(), CellDirectoryError> {
        self.document =
            stage_v3_source_finalized(&self.document, &self.requested.transfer_id, proof)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn request_abort(&mut self) -> Result<(), CellDirectoryError> {
        self.document = stage_v3_request_abort(&self.document, &self.requested.transfer_id)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn record_abort(
        &mut self,
        proof: &DraftGridAbortCleanupProofV2,
    ) -> Result<(), CellDirectoryError> {
        self.document = stage_v3_abort_cleanup(&self.document, &self.requested.transfer_id, proof)?;
        self.retain_current();
        Ok(())
    }

    pub(crate) fn persist_history(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<DraftCellDirectoryHistoryStoreV3, CellDirectoryError> {
        let mut store = DraftCellDirectoryHistoryStoreV3::open_or_initialize(
            root,
            self.history
                .first()
                .expect("test authority history has genesis")
                .clone(),
        )?;
        for pair in self.history.windows(2) {
            store.commit(
                pair[0].directory_revision,
                &pair[0].document_hash,
                pair[1].clone(),
            )?;
        }
        Ok(store)
    }
}

fn remove_stale_history_head_temps_v3(root: &Path) -> Result<(), CellDirectoryError> {
    let expected = BTreeSet::from([
        DRAFT_DIRECTORY_HISTORY_LOCK_FILE,
        DRAFT_DIRECTORY_HISTORY_HEAD_FILE,
        DRAFT_DIRECTORY_HISTORY_FILE,
    ]);
    let temp_prefix = format!(".{DRAFT_DIRECTORY_HISTORY_HEAD_FILE}.tmp-");
    let mut stale_temps = Vec::new();
    for entry in fs::read_dir(root).map_err(|source| io_error_v3(root, source))? {
        let entry = entry.map_err(|source| io_error_v3(root, source))?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| invalid("v3 directory contains a non-UTF-8 artifact"))?;
        let file_type = entry
            .file_type()
            .map_err(|source| io_error_v3(entry.path(), source))?;
        if expected.contains(name) {
            if !file_type.is_file() {
                return Err(invalid(
                    "v3 directory authority artifact is not a regular file",
                ));
            }
            continue;
        }
        let recognized_temp = name
            .strip_prefix(&temp_prefix)
            .and_then(|suffix| suffix.split_once('-'))
            .is_some_and(|(pid, uuid)| {
                pid.parse::<u32>()
                    .is_ok_and(|value| value > 0 && value.to_string() == pid)
                    && Uuid::parse_str(uuid).is_ok_and(|value| value.to_string() == uuid)
            });
        if !recognized_temp || !file_type.is_file() {
            return Err(invalid(
                "v3 directory contains an unexpected authority artifact",
            ));
        }
        stale_temps.push(entry.path());
    }
    if stale_temps.len() > MAX_STALE_DIRECTORY_HEAD_TEMPS {
        return Err(invalid(
            "v3 directory contains too many stale head temporary files",
        ));
    }
    for temp_path in &stale_temps {
        fs::remove_file(temp_path).map_err(|source| io_error_v3(temp_path, source))?;
    }
    if !stale_temps.is_empty() {
        File::open(root)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error_v3(root, source))?;
    }
    Ok(())
}

fn lock_history_writer_v3(root: &Path, create: bool) -> Result<File, CellDirectoryError> {
    let lock_path = root.join(DRAFT_DIRECTORY_HISTORY_LOCK_FILE);
    let lock_file = OpenOptions::new()
        .create(create)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| io_error_v3(&lock_path, source))?;
    FileExt::try_lock_exclusive(&lock_file).map_err(|source| {
        if source.kind() == std::io::ErrorKind::WouldBlock {
            CellDirectoryError::WriterAlreadyActive(root.to_path_buf())
        } else {
            io_error_v3(&lock_path, source)
        }
    })?;
    Ok(lock_file)
}

fn read_history_head_v3(path: &Path) -> Result<DraftDirectoryHistoryHeadV3, CellDirectoryError> {
    let length = fs::metadata(path)
        .map_err(|source| io_error_v3(path, source))?
        .len();
    if length == 0 || length > MAX_DRAFT_DIRECTORY_HISTORY_HEAD_BYTES {
        return Err(invalid("v3 directory history head exceeds its byte bound"));
    }
    let bytes = fs::read(path).map_err(|source| io_error_v3(path, source))?;
    let head = serde_json::from_slice::<DraftDirectoryHistoryHeadV3>(&bytes).map_err(|source| {
        CellDirectoryError::Json {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let canonical =
        serde_json::to_vec_pretty(&head).map_err(|source| CellDirectoryError::Json {
            path: path.to_path_buf(),
            source,
        })?;
    if canonical != bytes {
        return Err(invalid("v3 directory history head bytes are not canonical"));
    }
    Ok(head)
}

fn io_error_v3(path: impl AsRef<Path>, source: std::io::Error) -> CellDirectoryError {
    CellDirectoryError::Io {
        path: path.as_ref().to_path_buf(),
        source,
    }
}

/// Non-serializable capability for one exact assigned cell in one exact,
/// fully validated historical directory document. Serialized event claims are
/// compared with this capability; they can never construct it themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedCellAuthorityV3 {
    directory_revision: u64,
    directory_document_hash: String,
    universe_id: String,
    universe_manifest_hash: String,
    assignment: CellAssignmentRecord,
}

impl ValidatedCellAuthorityV3 {
    fn try_from_document_assignment(
        directory_revision: u64,
        directory_document_hash: &str,
        universe_id: &str,
        universe_manifest_hash: &str,
        assignment: &CellAssignmentRecord,
    ) -> Result<Self, CellDirectoryError> {
        if assignment.state != CellAssignmentState::Assigned
            || assignment.holder_id.as_deref().is_none_or(str::is_empty)
        {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id: assignment.cell_id.clone(),
                reason: "v3 historical cell is not assigned to a live authority holder".into(),
            });
        }
        Ok(Self {
            directory_revision,
            directory_document_hash: directory_document_hash.to_owned(),
            universe_id: universe_id.to_owned(),
            universe_manifest_hash: universe_manifest_hash.to_owned(),
            assignment: assignment.clone(),
        })
    }

    pub(super) fn directory_revision(&self) -> u64 {
        self.directory_revision
    }

    pub(super) fn directory_document_hash(&self) -> &str {
        &self.directory_document_hash
    }

    pub(super) fn universe_id(&self) -> &str {
        &self.universe_id
    }

    pub(super) fn universe_manifest_hash(&self) -> &str {
        &self.universe_manifest_hash
    }

    pub(super) fn bind_manifest_v5<'authority, 'manifest>(
        &'authority self,
        manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
    ) -> Result<ValidatedManifestBoundCellAuthorityV3<'authority, 'manifest>, CellDirectoryError>
    {
        if self.universe_id != manifest.universe_id()
            || self.universe_manifest_hash != manifest.manifest_hash()
        {
            return Err(invalid(
                "directory-v3 cell authority does not match the validated manifest-5 identity",
            ));
        }
        Ok(ValidatedManifestBoundCellAuthorityV3 {
            authority: self,
            manifest,
        })
    }

    pub(super) fn cell_key(&self) -> &CellKeyV1 {
        &self.assignment.cell_key
    }

    pub(super) fn cell_id(&self) -> &str {
        &self.assignment.cell_id
    }

    pub(super) fn assignment_generation(&self) -> u64 {
        self.assignment.assignment_generation
    }

    pub(super) fn fencing_token(&self) -> u64 {
        self.assignment.authority_fencing_token
    }

    pub(super) fn holder_id(&self) -> &str {
        self.assignment
            .holder_id
            .as_deref()
            .expect("validated current cell authority has a holder")
    }

    pub(super) fn fencing_history(&self) -> &BTreeMap<u64, u64> {
        &self.assignment.fencing_history
    }
}

/// Read-only capability produced only after the complete dormant directory-v3
/// document (including assignments, fencing history, and phase proofs) passes
/// validation. Grid handoff staging consumes this view instead of reconstructing
/// directory authority from caller-supplied booleans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedGridTransferAuthorityV3 {
    directory_revision: u64,
    directory_document_hash: String,
    universe_id: String,
    universe_manifest_hash: String,
    record: CellTransferRecordV3,
    source_assignment: CellAssignmentRecord,
    destination_assignment: CellAssignmentRecord,
}

/// Non-Serde borrow proving that historical or current grid authority belongs
/// to the complete manifest-5 universe rather than an arbitrary hash string.
#[derive(Debug)]
pub(super) struct ValidatedManifestBoundGridAuthorityV3<'authority, 'manifest> {
    authority: &'authority ValidatedGridTransferAuthorityV3,
    manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
}

impl<'authority> ValidatedManifestBoundGridAuthorityV3<'authority, '_> {
    pub(super) fn authority(&self) -> &'authority ValidatedGridTransferAuthorityV3 {
        self.authority
    }

    pub(super) fn manifest_hash(&self) -> &str {
        self.manifest.manifest_hash()
    }
}

/// Manifest-bound equivalent for production and other whole-cell authority.
#[derive(Debug)]
pub(super) struct ValidatedManifestBoundCellAuthorityV3<'authority, 'manifest> {
    authority: &'authority ValidatedCellAuthorityV3,
    manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
}

impl<'authority> ValidatedManifestBoundCellAuthorityV3<'authority, '_> {
    pub(super) fn authority(&self) -> &'authority ValidatedCellAuthorityV3 {
        self.authority
    }

    pub(super) fn manifest_hash(&self) -> &str {
        self.manifest.manifest_hash()
    }
}

/// Non-Serde live capability for the exact current directory head. Its store
/// lifetime prevents the writer from committing a successor while it is held.
#[derive(Debug)]
pub(super) struct ValidatedCurrentGridAuthorityV3<'store> {
    authority: ValidatedGridTransferAuthorityV3,
    _store_guard: PhantomData<&'store DraftCellDirectoryHistoryStoreV3>,
}

impl ValidatedCurrentGridAuthorityV3<'_> {
    pub(super) fn validated(&self) -> &ValidatedGridTransferAuthorityV3 {
        &self.authority
    }
}

/// Non-Serde live capability for one assigned cell at the exact current
/// directory head. Historical cell evidence cannot construct this type.
#[derive(Debug)]
pub(super) struct ValidatedCurrentCellAuthorityV3<'store> {
    authority: ValidatedCellAuthorityV3,
    _store_guard: PhantomData<&'store DraftCellDirectoryHistoryStoreV3>,
}

impl ValidatedCurrentCellAuthorityV3<'_> {
    pub(super) fn validated(&self) -> &ValidatedCellAuthorityV3 {
        &self.authority
    }
}

impl ValidatedGridTransferAuthorityV3 {
    pub(super) fn directory_revision(&self) -> u64 {
        self.directory_revision
    }

    pub(super) fn directory_document_hash(&self) -> &str {
        &self.directory_document_hash
    }

    pub(super) fn universe_id(&self) -> &str {
        &self.universe_id
    }

    pub(super) fn universe_manifest_hash(&self) -> &str {
        &self.universe_manifest_hash
    }

    pub(super) fn bind_manifest_v5<'authority, 'manifest>(
        &'authority self,
        manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
    ) -> Result<ValidatedManifestBoundGridAuthorityV3<'authority, 'manifest>, CellDirectoryError>
    {
        if self.universe_id != manifest.universe_id()
            || self.universe_manifest_hash != manifest.manifest_hash()
        {
            return Err(invalid(
                "directory-v3 grid authority does not match the validated manifest-5 identity",
            ));
        }
        Ok(ValidatedManifestBoundGridAuthorityV3 {
            authority: self,
            manifest,
        })
    }

    pub(super) fn source_cell_authority(
        &self,
    ) -> Result<ValidatedCellAuthorityV3, CellDirectoryError> {
        ValidatedCellAuthorityV3::try_from_document_assignment(
            self.directory_revision,
            &self.directory_document_hash,
            &self.universe_id,
            &self.universe_manifest_hash,
            &self.source_assignment,
        )
    }

    pub(super) fn destination_cell_authority(
        &self,
    ) -> Result<ValidatedCellAuthorityV3, CellDirectoryError> {
        ValidatedCellAuthorityV3::try_from_document_assignment(
            self.directory_revision,
            &self.directory_document_hash,
            &self.universe_id,
            &self.universe_manifest_hash,
            &self.destination_assignment,
        )
    }

    pub(super) fn package_schema_version(&self) -> u32 {
        self.record.bundle.package_schema_version
    }

    pub(super) fn receipt_schema_version(&self) -> u32 {
        self.record.bundle.receipt_schema_version
    }

    pub(super) fn aggregate_kind(&self) -> MobileAggregateKind {
        self.record.bundle.aggregate_kind
    }

    pub(super) fn transfer_id(&self) -> &str {
        &self.record.transfer_id
    }

    pub(super) fn root_aggregate_id(&self) -> &str {
        &self.record.root_aggregate_id
    }

    pub(super) fn package_hash(&self) -> &str {
        &self.record.bundle.package_hash
    }

    pub(super) fn closure_root(&self) -> &str {
        &self.record.bundle.closure_root
    }

    pub(super) fn conservation_root(&self) -> &str {
        &self.record.bundle.conservation_root
    }

    pub(super) fn member_root(&self) -> &str {
        &self.record.bundle.member_root
    }

    pub(super) fn source_cell_key(&self) -> &CellKeyV1 {
        &self.record.source_cell_key
    }

    pub(super) fn source_cell_id(&self) -> &str {
        &self.record.source_cell_id
    }

    pub(super) fn destination_cell_key(&self) -> &CellKeyV1 {
        &self.record.destination_cell_key
    }

    pub(super) fn destination_cell_id(&self) -> &str {
        &self.record.destination_cell_id
    }

    pub(super) fn source_assignment_generation(&self) -> u64 {
        self.record.source_assignment_generation
    }

    pub(super) fn source_fencing_token(&self) -> u64 {
        self.record.source_fencing_token
    }

    pub(super) fn destination_assignment_generation(&self) -> u64 {
        self.record.destination_assignment_generation
    }

    pub(super) fn destination_fencing_token(&self) -> u64 {
        self.record.destination_fencing_token
    }

    pub(super) fn members(&self) -> &[BundledPlacementMember] {
        &self.record.bundle.members
    }

    pub(super) fn phase(&self) -> TransferPhase {
        self.record.phase
    }

    pub(super) fn quarantine_receipt_hash(&self) -> Option<&str> {
        self.record.quarantine_receipt_hash.as_deref()
    }

    pub(super) fn source_prepare_proven(&self) -> bool {
        self.record.source_prepare_proof.is_some()
    }

    pub(super) fn source_prepare_proof(&self) -> Option<&DirectoryPhaseProofV3> {
        self.record.source_prepare_proof.as_ref()
    }

    pub(super) fn source_prepare_cell_proof(&self) -> Option<DraftGridPrepareProofV2> {
        self.record
            .source_prepare_proof
            .as_ref()?
            .source_prepare_cell_proof(&self.record.root_aggregate_id)
    }

    pub(super) fn destination_quarantine_proven(&self) -> bool {
        self.record.destination_quarantine_proof.is_some()
    }

    pub(super) fn destination_quarantine_proof(&self) -> Option<&DirectoryPhaseProofV3> {
        self.record.destination_quarantine_proof.as_ref()
    }

    pub(super) fn destination_quarantine_cell_proof(&self) -> Option<DraftGridQuarantineProofV2> {
        self.record
            .destination_quarantine_proof
            .as_ref()?
            .destination_quarantine_cell_proof(&self.record.root_aggregate_id)
    }

    pub(super) fn source_export_proven(&self) -> bool {
        self.record.source_export_proof.is_some()
    }

    pub(super) fn source_export_proof(&self) -> Option<&DirectoryPhaseProofV3> {
        self.record.source_export_proof.as_ref()
    }

    pub(super) fn source_export_cell_proof(&self) -> Option<DraftGridExportProofV2> {
        let proof = self.record.source_export_proof.as_ref()?;
        let export = DraftGridExportProofV2 {
            transfer_id: proof.transfer_id.clone(),
            root_aggregate_id: self.record.root_aggregate_id.clone(),
            member_root: proof.member_root.clone(),
            package_hash: proof.package_hash.clone(),
            source_cell_id: proof.cell_id.clone(),
            assignment_generation: proof.assignment_generation,
            fencing_token: proof.fencing_token,
            prior_event_sequence: proof.prior_event_sequence?,
            prior_event_hash: proof.prior_event_hash.clone()?,
            event_sequence: proof.event_sequence,
            event_hash: proof.event_hash.clone(),
            event_payload_hash: proof.event_payload_hash.clone()?,
            prior_draft_world_hash: proof.prior_draft_world_hash.clone()?,
            resulting_active_world_hash: proof.world_hash.clone(),
            quarantine_receipt_hash: proof.quarantine_receipt_hash.clone()?,
            exported_at_unix_ms: proof.trusted_time_unix_ms?,
            mutation_witness_hash: proof.mutation_witness_hash.clone()?,
            proof_hash: proof.export_proof_hash.clone()?,
            ledger_vector: proof.ledger_vector?,
        };
        export.validate().is_ok().then_some(export)
    }

    pub(super) fn destination_import_proven(&self) -> bool {
        self.record.import_proof.is_some()
    }

    pub(super) fn destination_import_proof(&self) -> Option<&DirectoryPhaseProofV3> {
        self.record.import_proof.as_ref()
    }

    pub(super) fn destination_import_cell_proof(&self) -> Option<DraftGridImportProofV2> {
        let proof = self.record.import_proof.as_ref()?;
        proof.destination_import_cell_proof(&self.record.root_aggregate_id)
    }

    pub(super) fn destination_activation_proven(&self) -> bool {
        self.record.destination_activation_proof.is_some()
    }

    pub(super) fn destination_activation_proof(&self) -> Option<&DirectoryPhaseProofV3> {
        self.record.destination_activation_proof.as_ref()
    }

    pub(super) fn destination_activation_cell_proof(&self) -> Option<DraftGridActivationProofV2> {
        let proof = self.record.destination_activation_proof.as_ref()?;
        proof.destination_activation_cell_proof(&self.record.root_aggregate_id)
    }

    pub(super) fn source_finalization_proven(&self) -> bool {
        self.record.finalization_proof.is_some()
    }

    pub(super) fn source_finalization_proof(&self) -> Option<&DirectoryPhaseProofV3> {
        self.record.finalization_proof.as_ref()
    }

    pub(super) fn source_finalization_cell_proof(&self) -> Option<DraftGridFinalizationProofV2> {
        let proof = self.record.finalization_proof.as_ref()?;
        proof.source_finalization_cell_proof(&self.record.root_aggregate_id)
    }

    pub(super) fn source_abort_proven(&self) -> bool {
        self.record.source_abort_proof.is_some()
    }

    pub(super) fn source_abort_cell_proof(&self) -> Option<DraftGridAbortCleanupProofV2> {
        self.record
            .source_abort_proof
            .as_ref()?
            .abort_cell_proof(DraftGridTransferAbortSideV2::Source)
    }

    pub(super) fn destination_abort_proven(&self) -> bool {
        self.record.destination_abort_proof.is_some()
    }

    pub(super) fn destination_abort_cell_proof(&self) -> Option<DraftGridAbortCleanupProofV2> {
        self.record
            .destination_abort_proof
            .as_ref()?
            .abort_cell_proof(DraftGridTransferAbortSideV2::Destination)
    }

    pub(super) fn live_source_assignment_generation(&self) -> u64 {
        self.source_assignment.assignment_generation
    }

    pub(super) fn source_fencing_history(&self) -> &BTreeMap<u64, u64> {
        &self.source_assignment.fencing_history
    }

    pub(super) fn live_source_fencing_token(&self) -> u64 {
        self.source_assignment.authority_fencing_token
    }

    pub(super) fn live_source_holder_id(&self) -> Option<&str> {
        self.source_assignment.holder_id.as_deref()
    }

    pub(super) fn live_destination_assignment_generation(&self) -> u64 {
        self.destination_assignment.assignment_generation
    }

    pub(super) fn destination_fencing_history(&self) -> &BTreeMap<u64, u64> {
        &self.destination_assignment.fencing_history
    }

    pub(super) fn live_destination_fencing_token(&self) -> u64 {
        self.destination_assignment.authority_fencing_token
    }

    pub(super) fn live_destination_holder_id(&self) -> Option<&str> {
        self.destination_assignment.holder_id.as_deref()
    }
}

impl CellTransferRecordV3 {
    fn immutable_material_matches(&self, other: &Self) -> bool {
        self.transfer_id == other.transfer_id
            && self.root_aggregate_id == other.root_aggregate_id
            && self.source_cell_key == other.source_cell_key
            && self.source_cell_id == other.source_cell_id
            && self.destination_cell_key == other.destination_cell_key
            && self.destination_cell_id == other.destination_cell_id
            && self.source_assignment_generation == other.source_assignment_generation
            && self.source_fencing_token == other.source_fencing_token
            && self.destination_assignment_generation == other.destination_assignment_generation
            && self.destination_fencing_token == other.destination_fencing_token
            && self.bundle == other.bundle
    }

    fn bundled_plan(&self) -> Result<BundledPlacementPlan, CellDirectoryError> {
        let plan = BundledPlacementPlan {
            root_aggregate_id: self.root_aggregate_id.clone(),
            source_cell_key: self.source_cell_key.clone(),
            source_cell_id: self.source_cell_id.clone(),
            destination_cell_key: self.destination_cell_key.clone(),
            destination_cell_id: self.destination_cell_id.clone(),
            members: self.bundle.members.clone(),
            member_root: self.bundle.member_root.clone(),
        };
        plan.validate()?;
        Ok(plan)
    }

    fn validate_identity(
        &self,
        transfer_id: &str,
    ) -> Result<BundledPlacementPlan, CellDirectoryError> {
        validate_stable_id(transfer_id, "transfer")?;
        validate_stable_id(&self.root_aggregate_id, "root aggregate")?;
        validate_hash(&self.bundle.closure_root, "closure root")?;
        validate_hash(&self.bundle.conservation_root, "conservation root")?;
        validate_hash(&self.bundle.package_hash, "package")?;
        validate_hash(&self.bundle.member_root, "member root")?;
        if self.transfer_id != transfer_id
            || self.bundle.package_schema_version != DRAFT_AGGREGATE_TRANSFER_PACKAGE_SCHEMA_VERSION
            || self.bundle.receipt_schema_version != DRAFT_AGGREGATE_TRANSFER_RECEIPT_SCHEMA_VERSION
            || self.bundle.aggregate_kind != MobileAggregateKind::Grid
            || self.source_assignment_generation == 0
            || self.source_fencing_token == 0
            || self.destination_assignment_generation == 0
            || self.destination_fencing_token == 0
        {
            return Err(invalid(format!(
                "v3 transfer {transfer_id} has invalid immutable identity, schema, or authority"
            )));
        }
        if let Some(receipt_hash) = &self.quarantine_receipt_hash {
            validate_hash(receipt_hash, "quarantine receipt")?;
        }
        if (self.phase == TransferPhase::Prepared && self.quarantine_receipt_hash.is_some())
            || (matches!(
                self.phase,
                TransferPhase::Quarantined
                    | TransferPhase::Committed
                    | TransferPhase::Imported
                    | TransferPhase::Finalized
            ) && self.quarantine_receipt_hash.is_none())
        {
            return Err(invalid(format!(
                "v3 transfer {transfer_id} phase and receipt binding disagree"
            )));
        }
        let plan = self.bundled_plan()?;
        let root = plan
            .members
            .iter()
            .find(|member| member.aggregate_id == plan.root_aggregate_id)
            .ok_or_else(|| invalid(format!("v3 transfer {transfer_id} omits its root")))?;
        if root.aggregate_kind != self.bundle.aggregate_kind {
            return Err(invalid(format!(
                "v3 transfer {transfer_id} root alias and member kind disagree"
            )));
        }
        Ok(plan)
    }

    fn validate_phase_proofs(
        &self,
        source: &CellAssignmentRecord,
        destination: &CellAssignmentRecord,
    ) -> Result<(), CellDirectoryError> {
        self.validate_phase_proof(
            self.source_prepare_proof.as_ref(),
            DirectoryPhaseProofKindV3::SourcePrepare,
            &self.source_cell_id,
            self.source_assignment_generation,
            source,
            false,
        )?;
        self.validate_phase_proof(
            self.destination_quarantine_proof.as_ref(),
            DirectoryPhaseProofKindV3::DestinationQuarantine,
            &self.destination_cell_id,
            self.destination_assignment_generation,
            destination,
            true,
        )?;
        self.validate_phase_proof(
            self.source_export_proof.as_ref(),
            DirectoryPhaseProofKindV3::SourceExport,
            &self.source_cell_id,
            self.source_assignment_generation,
            source,
            true,
        )?;
        self.validate_phase_proof(
            self.import_proof.as_ref(),
            DirectoryPhaseProofKindV3::DestinationImport,
            &self.destination_cell_id,
            self.destination_assignment_generation,
            destination,
            true,
        )?;
        self.validate_phase_proof(
            self.destination_activation_proof.as_ref(),
            DirectoryPhaseProofKindV3::DestinationActivation,
            &self.destination_cell_id,
            self.destination_assignment_generation,
            destination,
            true,
        )?;
        self.validate_phase_proof(
            self.finalization_proof.as_ref(),
            DirectoryPhaseProofKindV3::SourceFinalization,
            &self.source_cell_id,
            self.source_assignment_generation,
            source,
            false,
        )?;
        self.validate_phase_proof(
            self.source_abort_proof.as_ref(),
            DirectoryPhaseProofKindV3::SourceAbort,
            &self.source_cell_id,
            self.source_assignment_generation,
            source,
            true,
        )?;
        self.validate_phase_proof(
            self.destination_abort_proof.as_ref(),
            DirectoryPhaseProofKindV3::DestinationAbort,
            &self.destination_cell_id,
            self.destination_assignment_generation,
            destination,
            true,
        )?;
        Self::validate_proof_order(
            self.source_prepare_proof.as_ref(),
            self.source_export_proof.as_ref(),
            "source prepare and export",
        )?;
        Self::validate_proof_order(
            self.source_export_proof.as_ref(),
            self.finalization_proof.as_ref(),
            "source export and finalization",
        )?;
        Self::validate_proof_order(
            self.destination_quarantine_proof.as_ref(),
            self.import_proof.as_ref(),
            "destination quarantine and import",
        )?;
        Self::validate_proof_order(
            self.import_proof.as_ref(),
            self.destination_activation_proof.as_ref(),
            "destination import and activation",
        )?;
        Self::validate_proof_order(
            self.source_prepare_proof.as_ref(),
            self.source_abort_proof.as_ref(),
            "source prepare and abort cleanup",
        )?;
        Self::validate_proof_order(
            self.destination_quarantine_proof.as_ref(),
            self.destination_abort_proof.as_ref(),
            "destination quarantine and abort cleanup",
        )?;

        let has_prepare = self.source_prepare_proof.is_some();
        let has_quarantine = self.destination_quarantine_proof.is_some();
        let has_export = self.source_export_proof.is_some();
        let has_import = self.import_proof.is_some();
        let has_activation = self.destination_activation_proof.is_some();
        let has_finalization = self.finalization_proof.is_some();
        let has_source_abort = self.source_abort_proof.is_some();
        let has_destination_abort = self.destination_abort_proof.is_some();
        let phase_valid = match self.phase {
            TransferPhase::Prepared => {
                self.quarantine_receipt_hash.is_none()
                    && !has_quarantine
                    && !has_export
                    && !has_import
                    && !has_activation
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Quarantined => {
                self.quarantine_receipt_hash.is_some()
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
                self.quarantine_receipt_hash.is_some()
                    && has_prepare
                    && has_quarantine
                    && !has_import
                    && !has_activation
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Imported => {
                self.quarantine_receipt_hash.is_some()
                    && has_prepare
                    && has_quarantine
                    && has_export
                    && has_import
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Finalized => {
                self.quarantine_receipt_hash.is_some()
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
                    && !has_activation
                    && !has_import
                    && !has_finalization
                    && (self.quarantine_receipt_hash.is_some() == has_quarantine)
                    && (!has_quarantine || has_prepare)
            }
            TransferPhase::Aborted => {
                !has_export
                    && !has_activation
                    && !has_import
                    && !has_finalization
                    && has_source_abort
                    && has_destination_abort
                    && (self.quarantine_receipt_hash.is_some() == has_quarantine)
                    && (!has_quarantine || has_prepare)
            }
        };
        if !phase_valid {
            return Err(invalid(format!(
                "v3 transfer {} phase and durable proof matrix disagree",
                self.transfer_id
            )));
        }
        Ok(())
    }

    fn validate_proof_order(
        earlier: Option<&DirectoryPhaseProofV3>,
        later: Option<&DirectoryPhaseProofV3>,
        phase_pair: &str,
    ) -> Result<(), CellDirectoryError> {
        let Some((earlier, later)) = earlier.zip(later) else {
            return Ok(());
        };
        if later.cell_id != earlier.cell_id
            || later.assignment_generation < earlier.assignment_generation
            || later.event_sequence <= earlier.event_sequence
        {
            return Err(invalid(format!(
                "v3 transfer proof order is invalid for {phase_pair}"
            )));
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_phase_proof(
        &self,
        proof: Option<&DirectoryPhaseProofV3>,
        expected_kind: DirectoryPhaseProofKindV3,
        expected_cell_id: &str,
        minimum_generation: u64,
        assignment: &CellAssignmentRecord,
        binds_receipt: bool,
    ) -> Result<(), CellDirectoryError> {
        let Some(proof) = proof else {
            return Ok(());
        };
        validate_hash(&proof.event_hash, "phase event")?;
        validate_hash(&proof.world_hash, "phase world")?;
        if let Some(hash) = &proof.event_payload_hash {
            validate_hash(hash, "phase event payload")?;
        }
        if let Some(hash) = &proof.prepare_proof_hash {
            validate_hash(hash, "source-prepare proof")?;
        }
        if let Some(hash) = &proof.quarantine_proof_hash {
            validate_hash(hash, "destination-quarantine proof")?;
        }
        if let Some(hash) = &proof.abort_witness_hash {
            validate_hash(hash, "abort witness")?;
        }
        if let Some(hash) = &proof.abort_proof_hash {
            validate_hash(hash, "abort cleanup proof")?;
        }
        if let Some(hash) = &proof.export_proof_hash {
            validate_hash(hash, "source-export proof")?;
        }
        if let Some(hash) = &proof.import_proof_hash {
            validate_hash(hash, "destination-import proof")?;
        }
        if let Some(hash) = &proof.activation_proof_hash {
            validate_hash(hash, "destination-activation proof")?;
        }
        if let Some(hash) = &proof.finalization_proof_hash {
            validate_hash(hash, "source-finalization proof")?;
        }
        if let Some(hash) = &proof.destination_import_proof_hash {
            validate_hash(hash, "activation destination-import proof")?;
        }
        if let Some(hash) = &proof.prior_event_hash
            && !hash.is_empty()
        {
            validate_hash(hash, "prior phase event")?;
        }
        if let Some(hash) = &proof.prior_draft_world_hash {
            validate_hash(hash, "prior draft world")?;
        }
        if let Some(hash) = &proof.prior_active_world_hash {
            validate_hash(hash, "prior active world")?;
        }
        if let Some(hash) = &proof.source_export_proof_hash {
            validate_hash(hash, "import source-export proof")?;
        }
        if let Some(hash) = &proof.production_eligibility_root {
            validate_hash(hash, "import production eligibility")?;
        }
        if let Some(hash) = &proof.mutation_witness_hash {
            validate_hash(hash, "source-export mutation witness")?;
        }
        if let Some(hash) = &proof.resulting_draft_world_hash {
            validate_hash(hash, "resulting draft world")?;
        }
        let is_abort = matches!(
            expected_kind,
            DirectoryPhaseProofKindV3::SourceAbort | DirectoryPhaseProofKindV3::DestinationAbort
        );
        let prepare_binding_valid = if expected_kind == DirectoryPhaseProofKindV3::SourcePrepare {
            proof
                .source_prepare_cell_proof(&self.root_aggregate_id)
                .is_some()
        } else {
            proof.prepare_proof_hash.is_none() && proof.prepared_at_simulation_tick.is_none()
        };
        let quarantine_binding_valid =
            if expected_kind == DirectoryPhaseProofKindV3::DestinationQuarantine {
                proof
                    .destination_quarantine_cell_proof(&self.root_aggregate_id)
                    .is_some()
            } else {
                proof.quarantine_proof_hash.is_none()
            };
        let export_binding_valid = if expected_kind == DirectoryPhaseProofKindV3::SourceExport {
            proof
                .export_proof_hash
                .as_ref()
                .zip(proof.mutation_witness_hash.as_ref())
                .zip(proof.ledger_vector)
                .is_some_and(|((proof_hash, mutation_witness_hash), ledger_vector)| {
                    DraftGridExportProofV2 {
                        transfer_id: proof.transfer_id.clone(),
                        root_aggregate_id: self.root_aggregate_id.clone(),
                        member_root: proof.member_root.clone(),
                        package_hash: proof.package_hash.clone(),
                        source_cell_id: proof.cell_id.clone(),
                        assignment_generation: proof.assignment_generation,
                        fencing_token: proof.fencing_token,
                        prior_event_sequence: proof.prior_event_sequence.unwrap_or_default(),
                        prior_event_hash: proof.prior_event_hash.clone().unwrap_or_default(),
                        event_sequence: proof.event_sequence,
                        event_hash: proof.event_hash.clone(),
                        event_payload_hash: proof.event_payload_hash.clone().unwrap_or_default(),
                        prior_draft_world_hash: proof
                            .prior_draft_world_hash
                            .clone()
                            .unwrap_or_default(),
                        resulting_active_world_hash: proof.world_hash.clone(),
                        quarantine_receipt_hash: proof
                            .quarantine_receipt_hash
                            .clone()
                            .unwrap_or_default(),
                        exported_at_unix_ms: proof.trusted_time_unix_ms.unwrap_or_default(),
                        mutation_witness_hash: mutation_witness_hash.clone(),
                        proof_hash: proof_hash.clone(),
                        ledger_vector,
                    }
                    .validate()
                    .is_ok()
                })
        } else {
            proof.export_proof_hash.is_none()
        };
        let import_binding_valid = if expected_kind == DirectoryPhaseProofKindV3::DestinationImport
        {
            let typed_import = match (
                proof.import_proof_hash.as_ref(),
                proof.prior_event_sequence,
                proof.prior_event_hash.as_ref(),
                proof.prior_draft_world_hash.as_ref(),
                proof.quarantined_at_unix_ms,
                proof.source_export_proof_hash.as_ref(),
                proof.source_exported_at_unix_ms,
                proof.destination_production_lifecycle_generation,
                proof.production_eligibility_root.as_ref(),
                proof.mutation_witness_hash.as_ref(),
                proof.ledger_vector,
                proof.trusted_time_unix_ms,
            ) {
                (
                    Some(import_proof_hash),
                    Some(prior_event_sequence),
                    Some(prior_event_hash),
                    Some(prior_draft_world_hash),
                    Some(quarantined_at_unix_ms),
                    Some(source_export_proof_hash),
                    Some(source_exported_at_unix_ms),
                    Some(destination_production_lifecycle_generation),
                    Some(production_eligibility_root),
                    Some(mutation_witness_hash),
                    Some(ledger_vector),
                    Some(imported_at_unix_ms),
                ) => Some(DraftGridImportProofV2 {
                    transfer_id: proof.transfer_id.clone(),
                    root_aggregate_id: self.root_aggregate_id.clone(),
                    member_root: proof.member_root.clone(),
                    package_hash: proof.package_hash.clone(),
                    destination_cell_id: proof.cell_id.clone(),
                    assignment_generation: proof.assignment_generation,
                    fencing_token: proof.fencing_token,
                    prior_event_sequence,
                    prior_event_hash: prior_event_hash.clone(),
                    event_sequence: proof.event_sequence,
                    event_hash: proof.event_hash.clone(),
                    event_payload_hash: proof.event_payload_hash.clone().unwrap_or_default(),
                    prior_draft_world_hash: prior_draft_world_hash.clone(),
                    resulting_active_world_hash: proof.world_hash.clone(),
                    quarantine_receipt_hash: proof
                        .quarantine_receipt_hash
                        .clone()
                        .unwrap_or_default(),
                    quarantined_at_unix_ms,
                    source_export_proof_hash: source_export_proof_hash.clone(),
                    source_exported_at_unix_ms,
                    imported_at_unix_ms,
                    destination_production_lifecycle_generation,
                    production_eligibility_root: production_eligibility_root.clone(),
                    mutation_witness_hash: mutation_witness_hash.clone(),
                    proof_hash: import_proof_hash.clone(),
                    ledger_vector,
                }),
                _ => None,
            };
            typed_import.is_some_and(|import| import.validate().is_ok())
                && self
                    .source_export_proof
                    .as_ref()
                    .is_some_and(|source_export| {
                        proof.source_export_proof_hash.as_deref()
                            == source_export.export_proof_hash.as_deref()
                            && proof.source_exported_at_unix_ms
                                == source_export.trusted_time_unix_ms
                    })
        } else if expected_kind == DirectoryPhaseProofKindV3::SourceFinalization {
            proof.import_proof_hash.is_none()
                && proof.prior_draft_world_hash.is_none()
                && proof.quarantined_at_unix_ms.is_none()
                && proof.source_export_proof_hash.is_some()
                && proof.source_exported_at_unix_ms.is_some()
                && proof.destination_production_lifecycle_generation.is_none()
        } else if expected_kind == DirectoryPhaseProofKindV3::DestinationQuarantine {
            proof.import_proof_hash.is_none()
                && proof.prior_draft_world_hash.is_none()
                && proof.quarantined_at_unix_ms.is_some()
                && proof.source_export_proof_hash.is_none()
                && proof.source_exported_at_unix_ms.is_none()
                && proof.destination_production_lifecycle_generation.is_none()
        } else if is_abort || expected_kind == DirectoryPhaseProofKindV3::SourceExport {
            proof.import_proof_hash.is_none()
                && proof.prior_draft_world_hash.is_some()
                && proof.quarantined_at_unix_ms.is_none()
                && proof.source_export_proof_hash.is_none()
                && proof.source_exported_at_unix_ms.is_none()
                && proof.destination_production_lifecycle_generation.is_none()
        } else {
            proof.import_proof_hash.is_none()
                && proof.prior_draft_world_hash.is_none()
                && proof.quarantined_at_unix_ms.is_none()
                && proof.source_export_proof_hash.is_none()
                && proof.source_exported_at_unix_ms.is_none()
                && proof.destination_production_lifecycle_generation.is_none()
        };
        let activation_binding_valid = if expected_kind
            == DirectoryPhaseProofKindV3::DestinationActivation
        {
            let typed_activation = match (
                proof.activation_proof_hash.as_ref(),
                proof.destination_import_proof_hash.as_ref(),
                proof.prior_event_sequence,
                proof.prior_event_hash.as_ref(),
                proof.prior_active_world_hash.as_ref(),
                proof.imported_at_unix_ms,
                proof.production_eligibility_root.as_ref(),
                proof.mutation_witness_hash.as_ref(),
                proof.trusted_time_unix_ms,
            ) {
                (
                    Some(activation_proof_hash),
                    Some(destination_import_proof_hash),
                    Some(prior_event_sequence),
                    Some(prior_event_hash),
                    Some(prior_active_world_hash),
                    Some(imported_at_unix_ms),
                    Some(production_eligibility_root),
                    Some(mutation_witness_hash),
                    Some(activated_at_unix_ms),
                ) => Some(DraftGridActivationProofV2 {
                    transfer_id: proof.transfer_id.clone(),
                    root_aggregate_id: self.root_aggregate_id.clone(),
                    member_root: proof.member_root.clone(),
                    package_hash: proof.package_hash.clone(),
                    destination_cell_id: proof.cell_id.clone(),
                    assignment_generation: proof.assignment_generation,
                    fencing_token: proof.fencing_token,
                    prior_event_sequence,
                    prior_event_hash: prior_event_hash.clone(),
                    event_sequence: proof.event_sequence,
                    event_hash: proof.event_hash.clone(),
                    event_payload_hash: proof.event_payload_hash.clone().unwrap_or_default(),
                    prior_active_world_hash: prior_active_world_hash.clone(),
                    resulting_active_world_hash: proof.world_hash.clone(),
                    quarantine_receipt_hash: proof
                        .quarantine_receipt_hash
                        .clone()
                        .unwrap_or_default(),
                    destination_import_proof_hash: destination_import_proof_hash.clone(),
                    imported_at_unix_ms,
                    activated_at_unix_ms,
                    production_eligibility_root: production_eligibility_root.clone(),
                    mutation_witness_hash: mutation_witness_hash.clone(),
                    proof_hash: activation_proof_hash.clone(),
                }),
                _ => None,
            };
            typed_activation.is_some_and(|activation| activation.validate().is_ok())
                && self.import_proof.as_ref().is_some_and(|import| {
                    proof.destination_import_proof_hash.as_deref()
                        == import.import_proof_hash.as_deref()
                        && proof.imported_at_unix_ms == import.trusted_time_unix_ms
                        && proof.production_eligibility_root == import.production_eligibility_root
                        && proof.assignment_generation >= import.assignment_generation
                        && proof.fencing_token >= import.fencing_token
                        && proof
                            .prior_event_sequence
                            .is_some_and(|sequence| sequence >= import.event_sequence)
                        && (proof.prior_event_sequence != Some(import.event_sequence)
                            || (proof.prior_event_hash.as_deref()
                                == Some(import.event_hash.as_str())
                                && proof.prior_active_world_hash.as_deref()
                                    == Some(import.world_hash.as_str())))
                })
        } else if expected_kind == DirectoryPhaseProofKindV3::SourceFinalization {
            proof.activation_proof_hash.is_some()
                && proof.destination_import_proof_hash.is_some()
                && proof.prior_active_world_hash.is_some()
                && proof.imported_at_unix_ms.is_some()
        } else if matches!(
            expected_kind,
            DirectoryPhaseProofKindV3::SourcePrepare
                | DirectoryPhaseProofKindV3::DestinationQuarantine
        ) {
            proof.activation_proof_hash.is_none()
                && proof.destination_import_proof_hash.is_none()
                && proof.prior_active_world_hash.is_some()
                && proof.imported_at_unix_ms.is_none()
        } else {
            proof.activation_proof_hash.is_none()
                && proof.destination_import_proof_hash.is_none()
                && proof.prior_active_world_hash.is_none()
                && proof.imported_at_unix_ms.is_none()
        };
        let finalization_binding_valid = if expected_kind
            == DirectoryPhaseProofKindV3::SourceFinalization
        {
            proof
                .source_finalization_cell_proof(&self.root_aggregate_id)
                .is_some_and(|finalization| {
                    finalization.validate().is_ok()
                        && self.source_export_proof.as_ref().is_some_and(|export| {
                            export.export_proof_hash.as_deref()
                                == Some(finalization.source_export_proof_hash.as_str())
                                && export.trusted_time_unix_ms
                                    == Some(finalization.source_exported_at_unix_ms)
                        })
                        && self.import_proof.as_ref().is_some_and(|import| {
                            import.import_proof_hash.as_deref()
                                == Some(finalization.destination_import_proof_hash.as_str())
                                && import.trusted_time_unix_ms
                                    == Some(finalization.imported_at_unix_ms)
                        })
                        && self
                            .destination_activation_proof
                            .as_ref()
                            .is_some_and(|activation| {
                                activation.activation_proof_hash.as_deref()
                                    == Some(finalization.destination_activation_proof_hash.as_str())
                                    && activation.trusted_time_unix_ms
                                        == Some(finalization.activated_at_unix_ms)
                            })
                })
        } else {
            proof.finalization_proof_hash.is_none()
                && proof.destination_activated_at_unix_ms.is_none()
        };
        let has_event_predecessor = matches!(
            expected_kind,
            DirectoryPhaseProofKindV3::SourcePrepare
                | DirectoryPhaseProofKindV3::DestinationQuarantine
                | DirectoryPhaseProofKindV3::SourceExport
                | DirectoryPhaseProofKindV3::DestinationImport
                | DirectoryPhaseProofKindV3::DestinationActivation
                | DirectoryPhaseProofKindV3::SourceFinalization
                | DirectoryPhaseProofKindV3::SourceAbort
                | DirectoryPhaseProofKindV3::DestinationAbort
        );
        let event_predecessor_valid = has_event_predecessor
            == (proof.prior_event_sequence.is_some() && proof.prior_event_hash.is_some());
        let production_root_valid = matches!(
            expected_kind,
            DirectoryPhaseProofKindV3::DestinationImport
                | DirectoryPhaseProofKindV3::DestinationActivation
        ) == proof.production_eligibility_root.is_some();
        let shared_handoff_binding_valid = match expected_kind {
            DirectoryPhaseProofKindV3::SourcePrepare => {
                proof.mutation_witness_hash.is_some()
                    && proof.trusted_time_unix_ms.is_none()
                    && proof.prepared_at_simulation_tick.is_some()
            }
            DirectoryPhaseProofKindV3::DestinationQuarantine => {
                proof.mutation_witness_hash.is_some()
                    && proof.trusted_time_unix_ms.is_none()
                    && proof.prepared_at_simulation_tick.is_none()
            }
            DirectoryPhaseProofKindV3::SourceExport
            | DirectoryPhaseProofKindV3::DestinationImport
            | DirectoryPhaseProofKindV3::DestinationActivation
            | DirectoryPhaseProofKindV3::SourceFinalization
            | DirectoryPhaseProofKindV3::SourceAbort
            | DirectoryPhaseProofKindV3::DestinationAbort => {
                proof.mutation_witness_hash.is_some()
                    && proof.trusted_time_unix_ms.is_some()
                    && proof.prepared_at_simulation_tick.is_none()
            }
        };
        let event_payload_binding_valid = proof.event_payload_hash.is_some();
        let ledger_binding_valid = matches!(
            expected_kind,
            DirectoryPhaseProofKindV3::SourceExport | DirectoryPhaseProofKindV3::DestinationImport
        ) == proof.ledger_vector.is_some();
        let abort_binding_valid = if is_abort {
            proof.resulting_draft_world_hash.as_deref() == Some(proof.world_hash.as_str())
                && proof
                    .abort_witness_hash
                    .as_ref()
                    .zip(proof.abort_removed_authority)
                    .is_some_and(|(witness_hash, removed_authority)| {
                        DraftGridAbortCleanupProofV2 {
                            side: match expected_kind {
                                DirectoryPhaseProofKindV3::SourceAbort => {
                                    DraftGridTransferAbortSideV2::Source
                                }
                                DirectoryPhaseProofKindV3::DestinationAbort => {
                                    DraftGridTransferAbortSideV2::Destination
                                }
                                _ => unreachable!("abort role was classified"),
                            },
                            transfer_id: proof.transfer_id.clone(),
                            member_root: proof.member_root.clone(),
                            package_hash: proof.package_hash.clone(),
                            cell_id: proof.cell_id.clone(),
                            assignment_generation: proof.assignment_generation,
                            fencing_token: proof.fencing_token,
                            event_sequence: proof.event_sequence,
                            event_hash: proof.event_hash.clone(),
                            event_payload_hash: proof
                                .event_payload_hash
                                .clone()
                                .unwrap_or_default(),
                            prior_event_sequence: proof.prior_event_sequence.unwrap_or_default(),
                            prior_event_hash: proof.prior_event_hash.clone().unwrap_or_default(),
                            prior_draft_world_hash: proof
                                .prior_draft_world_hash
                                .clone()
                                .unwrap_or_default(),
                            resulting_draft_world_hash: proof.world_hash.clone(),
                            trusted_time_unix_ms: proof.trusted_time_unix_ms.unwrap_or_default(),
                            mutation_witness_hash: proof
                                .mutation_witness_hash
                                .clone()
                                .unwrap_or_default(),
                            quarantine_receipt_hash: proof.quarantine_receipt_hash.clone(),
                            abort_witness_hash: witness_hash.clone(),
                            removed_authority,
                            proof_hash: proof.abort_proof_hash.clone().unwrap_or_default(),
                        }
                        .validate()
                        .is_ok()
                    })
        } else {
            proof.abort_witness_hash.is_none()
                && proof.abort_proof_hash.is_none()
                && proof.resulting_draft_world_hash.is_none()
                && proof.abort_removed_authority.is_none()
        };
        let receipt_binding_valid = match expected_kind {
            DirectoryPhaseProofKindV3::SourceAbort => proof
                .quarantine_receipt_hash
                .as_ref()
                .is_none_or(|receipt| self.quarantine_receipt_hash.as_ref() == Some(receipt)),
            _ if binds_receipt => proof.quarantine_receipt_hash == self.quarantine_receipt_hash,
            _ => proof.quarantine_receipt_hash.is_none(),
        };
        if proof.kind != expected_kind
            || proof.transfer_id != self.transfer_id
            || proof.member_root != self.bundle.member_root
            || proof.package_hash != self.bundle.package_hash
            || proof.cell_id != expected_cell_id
            || proof.assignment_generation < minimum_generation
            || assignment
                .fencing_history
                .get(&proof.assignment_generation)
                .copied()
                != Some(proof.fencing_token)
            || proof.event_sequence == 0
            || !prepare_binding_valid
            || !quarantine_binding_valid
            || !export_binding_valid
            || !import_binding_valid
            || !activation_binding_valid
            || !finalization_binding_valid
            || !event_predecessor_valid
            || !production_root_valid
            || !shared_handoff_binding_valid
            || !event_payload_binding_valid
            || !ledger_binding_valid
            || !abort_binding_valid
            || !receipt_binding_valid
        {
            return Err(invalid(format!(
                "v3 transfer {} contains an invalid phase proof",
                self.transfer_id
            )));
        }
        Ok(())
    }
}

impl CellDirectoryDocumentV3 {
    pub(super) fn validated_cell_authority(
        &self,
        cell_id: &str,
    ) -> Result<ValidatedCellAuthorityV3, CellDirectoryError> {
        self.validate()?;
        let assignment = self
            .assignments
            .get(cell_id)
            .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.to_owned()))?;
        ValidatedCellAuthorityV3::try_from_document_assignment(
            self.directory_revision,
            &self.document_hash,
            &self.universe_id,
            &self.universe_manifest_hash,
            assignment,
        )
    }

    pub(super) fn validated_grid_transfer_authority(
        &self,
        transfer_id: &str,
    ) -> Result<ValidatedGridTransferAuthorityV3, CellDirectoryError> {
        self.validate()?;
        let record = v3_transfer(self, transfer_id)?.clone();
        Ok(ValidatedGridTransferAuthorityV3 {
            directory_revision: self.directory_revision,
            directory_document_hash: self.document_hash.clone(),
            universe_id: self.universe_id.clone(),
            universe_manifest_hash: self.universe_manifest_hash.clone(),
            source_assignment: self
                .assignments
                .get(&record.source_cell_id)
                .expect("validated source assignment exists")
                .clone(),
            destination_assignment: self
                .assignments
                .get(&record.destination_cell_id)
                .expect("validated destination assignment exists")
                .clone(),
            record,
        })
    }

    fn calculate_hash(&self) -> Result<String, CellDirectoryError> {
        let mut material = self.clone();
        material.document_hash.clear();
        let bytes = serde_json::to_vec(&material).map_err(|source| {
            invalid(format!(
                "v3 directory hash material cannot be encoded: {source}"
            ))
        })?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(DOCUMENT_HASH_DOMAIN);
        hasher.update(&bytes);
        Ok(hasher.finalize().to_hex().to_string())
    }

    fn seal(&mut self) -> Result<(), CellDirectoryError> {
        self.document_hash = self.calculate_hash()?;
        self.validate()
    }

    fn validate(&self) -> Result<(), CellDirectoryError> {
        let encoded_len = serde_json::to_vec(self)
            .map_err(|source| invalid(format!("v3 directory cannot be measured: {source}")))?
            .len();
        if self.schema_version != DRAFT_CELL_DIRECTORY_V3_SCHEMA_VERSION
            || self.universe_id.trim().is_empty()
            || !valid_blake3_hex(&self.universe_manifest_hash)
            || self.directory_revision == 0
            || self.assignments.len() != 2
            || self.placements.len() > MAX_DRAFT_PLACEMENTS
            || self.transfers.len() > MAX_DRAFT_TRANSFERS
            || encoded_len > MAX_DRAFT_DIRECTORY_V3_BYTES
            || !valid_blake3_hex(&self.document_hash)
            || self.document_hash != self.calculate_hash()?
        {
            return Err(invalid(
                "v3 directory schema, universe, manifest, revision, or hash is invalid",
            ));
        }
        self.validate_assignments()?;
        self.validate_migration_genesis()?;
        self.validate_placements()?;
        self.validate_transfers()?;
        Ok(())
    }

    fn validate_migration_genesis(&self) -> Result<(), CellDirectoryError> {
        let Some(genesis) = &self.migration_genesis else {
            return Ok(());
        };
        genesis.validate()?;
        if self.directory_revision == 1 {
            if genesis.target_assignment_root
                != hash_directory_genesis(MIGRATION_ASSIGNMENT_ROOT_DOMAIN, &self.assignments)?
                || genesis.target_placement_root
                    != hash_directory_genesis(MIGRATION_PLACEMENT_ROOT_DOMAIN, &self.placements)?
            {
                return Err(invalid(
                    "directory-v3 migration genesis roots differ from revision one",
                ));
            }
            let expected_floors = self
                .placements
                .iter()
                .filter(|(_, placement)| placement.placement_generation > 1)
                .map(|(aggregate_id, placement)| {
                    (
                        aggregate_id.clone(),
                        MigrationPlacementFloorV1 {
                            aggregate_id: aggregate_id.clone(),
                            aggregate_kind: placement.aggregate_kind,
                            cell_key: placement.cell_key.clone(),
                            cell_id: placement.cell_id.clone(),
                            placement_generation: placement.placement_generation,
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>();
            if genesis.placement_floors != expected_floors {
                return Err(invalid(
                    "directory-v3 migration placement floors are incomplete",
                ));
            }
        }
        for (aggregate_id, floor) in &genesis.placement_floors {
            let placement = self.placements.get(aggregate_id).ok_or_else(|| {
                invalid(format!(
                    "directory-v3 migration floor {aggregate_id} has no current placement"
                ))
            })?;
            let assignment = self.assignments.get(&floor.cell_id).ok_or_else(|| {
                invalid(format!(
                    "directory-v3 migration floor {aggregate_id} has no genesis cell"
                ))
            })?;
            if floor.cell_key != assignment.cell_key
                || floor.aggregate_kind != placement.aggregate_kind
                || placement.placement_generation < floor.placement_generation
                || (placement.placement_generation == floor.placement_generation
                    && (placement.cell_id != floor.cell_id || placement.cell_key != floor.cell_key))
            {
                return Err(invalid(format!(
                    "directory-v3 migration floor {aggregate_id} disagrees with placement history"
                )));
            }
        }
        Ok(())
    }

    fn validate_assignments(&self) -> Result<(), CellDirectoryError> {
        if self.assignments.is_empty() {
            return Err(invalid("v3 directory has no cell assignments"));
        }
        for (cell_id, assignment) in &self.assignments {
            celestial::validate_cell_key(&assignment.cell_key)
                .map_err(|source| invalid(source.to_string()))?;
            let derived = celestial::cell_id(&assignment.cell_key)
                .map_err(|source| invalid(source.to_string()))?;
            if cell_id != &derived
                || assignment.cell_id != *cell_id
                || assignment.cell_key.universe_id != self.universe_id
            {
                return Err(invalid(format!(
                    "v3 assignment {cell_id} has inconsistent cell identity"
                )));
            }
            match assignment.state {
                CellAssignmentState::Sleeping => {
                    if assignment.holder_id.is_some() {
                        return Err(invalid(format!(
                            "v3 sleeping assignment {cell_id} retains authority"
                        )));
                    }
                }
                CellAssignmentState::Assigned => {
                    if assignment.assignment_generation == 0
                        || assignment.authority_fencing_token == 0
                        || assignment.holder_id.as_deref().is_none_or(str::is_empty)
                    {
                        return Err(invalid(format!(
                            "v3 assigned cell {cell_id} has invalid authority history"
                        )));
                    }
                }
                CellAssignmentState::Claiming | CellAssignmentState::Releasing => {
                    return Err(invalid(format!(
                        "v3 cell {cell_id} retained an incomplete authority transition"
                    )));
                }
            }
            if assignment.fencing_history.len()
                != usize::try_from(assignment.assignment_generation).unwrap_or(usize::MAX)
                || assignment
                    .fencing_history
                    .get(&assignment.assignment_generation)
                    .copied()
                    .unwrap_or(0)
                    != assignment.authority_fencing_token
            {
                return Err(invalid(format!(
                    "v3 cell {cell_id} has an incomplete authority history"
                )));
            }
            let mut prior_fence = 0;
            for generation in 1..=assignment.assignment_generation {
                let fence = assignment
                    .fencing_history
                    .get(&generation)
                    .copied()
                    .ok_or_else(|| {
                        invalid(format!("v3 assigned cell {cell_id} has a fence gap"))
                    })?;
                if fence <= prior_fence {
                    return Err(invalid(format!(
                        "v3 assigned cell {cell_id} has a non-increasing fence"
                    )));
                }
                prior_fence = fence;
            }
        }
        Ok(())
    }

    fn validate_placements(&self) -> Result<(), CellDirectoryError> {
        for (aggregate_id, placement) in &self.placements {
            validate_stable_id(aggregate_id, "aggregate")?;
            let assignment = self.assignments.get(&placement.cell_id).ok_or_else(|| {
                invalid(format!(
                    "v3 placement {aggregate_id} references an unknown cell"
                ))
            })?;
            if placement.aggregate_id != *aggregate_id
                || placement.placement_generation == 0
                || placement.cell_key != assignment.cell_key
            {
                return Err(invalid(format!(
                    "v3 placement {aggregate_id} has invalid identity or generation"
                )));
            }
            match placement.state {
                AggregatePlacementState::Resident if placement.active_transfer_id.is_none() => {}
                AggregatePlacementState::Preparing | AggregatePlacementState::InTransit
                    if placement.active_transfer_id.is_some() => {}
                _ => {
                    return Err(invalid(format!(
                        "v3 placement {aggregate_id} state and transfer binding disagree"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_transfers(&self) -> Result<(), CellDirectoryError> {
        let mut total_memberships = 0usize;
        let mut plans = BTreeMap::new();
        let mut advance_index = BTreeMap::new();
        for (transfer_id, transfer) in &self.transfers {
            if self
                .migration_genesis
                .as_ref()
                .is_some_and(|genesis| genesis.source_terminal_transfer_ids.contains(transfer_id))
            {
                return Err(invalid(format!(
                    "v3 transfer {transfer_id} reuses a frozen source transfer identity"
                )));
            }
            let plan = transfer.validate_identity(transfer_id)?;
            if let Some(genesis) = &self.migration_genesis {
                for member in &plan.members {
                    if genesis
                        .placement_floors
                        .get(&member.aggregate_id)
                        .is_some_and(|floor| {
                            member.prior_placement_generation < floor.placement_generation
                        })
                    {
                        return Err(invalid(format!(
                            "v3 transfer {transfer_id} precedes a receipt-bound migration floor"
                        )));
                    }
                }
            }
            total_memberships = total_memberships
                .checked_add(plan.members.len())
                .ok_or_else(|| invalid("v3 transfer membership count overflowed"))?;
            if total_memberships > MAX_DRAFT_TRANSFER_MEMBERSHIPS {
                return Err(invalid(
                    "v3 directory exceeds its transfer membership bound",
                ));
            }
            if matches!(
                transfer.phase,
                TransferPhase::Committed | TransferPhase::Imported | TransferPhase::Finalized
            ) {
                for member in &plan.members {
                    let key = (
                        member.aggregate_id.clone(),
                        member.prior_placement_generation,
                        transfer.source_cell_id.clone(),
                    );
                    let value = (
                        member.resulting_placement_generation,
                        transfer.destination_cell_id.clone(),
                    );
                    if advance_index.insert(key, value).is_some() {
                        return Err(invalid(format!(
                            "v3 transfer history is ambiguous for member {}",
                            member.aggregate_id
                        )));
                    }
                }
            }
            plans.insert(transfer_id.clone(), plan);
        }

        for (transfer_id, transfer) in &self.transfers {
            let plan = plans
                .get(transfer_id)
                .expect("validated v3 transfer plan is indexed");
            let source = self
                .assignments
                .get(&transfer.source_cell_id)
                .ok_or_else(|| {
                    invalid(format!("v3 transfer {transfer_id} source cell is unknown"))
                })?;
            let destination = self
                .assignments
                .get(&transfer.destination_cell_id)
                .ok_or_else(|| {
                    invalid(format!(
                        "v3 transfer {transfer_id} destination cell is unknown"
                    ))
                })?;
            if source.cell_key != transfer.source_cell_key
                || destination.cell_key != transfer.destination_cell_key
                || source
                    .fencing_history
                    .get(&transfer.source_assignment_generation)
                    .copied()
                    != Some(transfer.source_fencing_token)
                || destination
                    .fencing_history
                    .get(&transfer.destination_assignment_generation)
                    .copied()
                    != Some(transfer.destination_fencing_token)
                || !are_face_neighbors(&transfer.source_cell_key, &transfer.destination_cell_key)
                || (!matches!(
                    transfer.phase,
                    TransferPhase::Finalized | TransferPhase::Aborted
                ) && (source.state != CellAssignmentState::Assigned
                    || destination.state != CellAssignmentState::Assigned))
            {
                return Err(invalid(format!(
                    "v3 transfer {transfer_id} cell route or historical authority is invalid"
                )));
            }
            transfer.validate_phase_proofs(source, destination)?;
            for member in &plan.members {
                let placement = self.placements.get(&member.aggregate_id).ok_or_else(|| {
                    invalid(format!(
                        "v3 transfer {transfer_id} member {} has no placement",
                        member.aggregate_id
                    ))
                })?;
                if placement.aggregate_kind != member.aggregate_kind
                    || !member_matches_phase(placement, member, transfer, transfer_id)
                {
                    return Err(invalid(format!(
                        "v3 transfer {transfer_id} member {} disagrees with its phase",
                        member.aggregate_id
                    )));
                }
                let terminal_start = match transfer.phase {
                    TransferPhase::Imported | TransferPhase::Finalized
                        if placement.placement_generation
                            > member.resulting_placement_generation =>
                    {
                        Some((
                            member.resulting_placement_generation,
                            transfer.destination_cell_id.as_str(),
                        ))
                    }
                    TransferPhase::Aborted
                        if placement.placement_generation > member.prior_placement_generation =>
                    {
                        Some((
                            member.prior_placement_generation,
                            transfer.source_cell_id.as_str(),
                        ))
                    }
                    _ => None,
                };
                if let Some((generation, cell_id)) = terminal_start {
                    Self::validate_later_member_history(
                        &member.aggregate_id,
                        generation,
                        cell_id,
                        placement,
                        &advance_index,
                    )?;
                }
            }
            if self.placements.iter().any(|(aggregate_id, placement)| {
                placement.active_transfer_id.as_deref() == Some(transfer_id)
                    && !plan
                        .members
                        .iter()
                        .any(|member| member.aggregate_id == *aggregate_id)
            }) {
                return Err(invalid(format!(
                    "v3 transfer {transfer_id} is active on a nonmember placement"
                )));
            }
        }

        for (aggregate_id, placement) in &self.placements {
            if placement.placement_generation > 1 {
                if let Some(floor) = self
                    .migration_genesis
                    .as_ref()
                    .and_then(|genesis| genesis.placement_floors.get(aggregate_id))
                {
                    Self::validate_later_member_history(
                        aggregate_id,
                        floor.placement_generation,
                        &floor.cell_id,
                        placement,
                        &advance_index,
                    )?;
                } else {
                    let mut origins =
                        advance_index
                            .keys()
                            .filter(|(candidate_id, generation, _)| {
                                candidate_id == aggregate_id && *generation == 1
                            });
                    let (_, _, origin_cell_id) = origins.next().ok_or_else(|| {
                        invalid(format!(
                            "v3 placement {aggregate_id} has no durable generation-one origin"
                        ))
                    })?;
                    if origins.next().is_some() {
                        return Err(invalid(format!(
                            "v3 placement {aggregate_id} has ambiguous generation-one history"
                        )));
                    }
                    Self::validate_later_member_history(
                        aggregate_id,
                        1,
                        origin_cell_id,
                        placement,
                        &advance_index,
                    )?;
                }
            }
            if let Some(transfer_id) = &placement.active_transfer_id {
                let transfer = self.transfers.get(transfer_id).ok_or_else(|| {
                    invalid(format!(
                        "v3 active placement {aggregate_id} references an unknown transfer"
                    ))
                })?;
                if !transfer
                    .bundle
                    .members
                    .iter()
                    .any(|member| member.aggregate_id == *aggregate_id)
                {
                    return Err(invalid(format!(
                        "v3 active placement {aggregate_id} is absent from its transfer"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_later_member_history(
        aggregate_id: &str,
        mut generation: u64,
        cell_id: &str,
        placement: &AggregatePlacementRecord,
        advance_index: &BTreeMap<(String, u64, String), (u64, String)>,
    ) -> Result<(), CellDirectoryError> {
        let mut current_cell_id = cell_id.to_owned();
        while generation < placement.placement_generation {
            let key = (aggregate_id.to_owned(), generation, current_cell_id.clone());
            let (next_generation, next_cell_id) = advance_index.get(&key).ok_or_else(|| {
                invalid(format!(
                    "v3 placement {aggregate_id} skips a durable transfer generation"
                ))
            })?;
            if *next_generation != generation.saturating_add(1) {
                return Err(invalid(format!(
                    "v3 placement {aggregate_id} has a non-contiguous transfer history"
                )));
            }
            generation = *next_generation;
            current_cell_id.clone_from(next_cell_id);
        }
        if generation != placement.placement_generation || current_cell_id != placement.cell_id {
            return Err(invalid(format!(
                "v3 placement {aggregate_id} history does not reach its current cell"
            )));
        }
        Ok(())
    }
}

fn stage_v3_prepare(
    document: &CellDirectoryDocumentV3,
    requested: &CellTransferRecordV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    if requested.phase != TransferPhase::Prepared
        || requested.quarantine_receipt_hash.is_some()
        || requested.source_prepare_proof.is_some()
        || requested.destination_quarantine_proof.is_some()
        || requested.source_export_proof.is_some()
        || requested.import_proof.is_some()
        || requested.destination_activation_proof.is_some()
        || requested.finalization_proof.is_some()
        || requested.source_abort_proof.is_some()
        || requested.destination_abort_proof.is_some()
    {
        return Err(conflict(
            &requested.transfer_id,
            "v3 prepare request contains post-prepare evidence",
        ));
    }
    let plan = requested.validate_identity(&requested.transfer_id)?;
    let source = document
        .assignments
        .get(&requested.source_cell_id)
        .ok_or_else(|| invalid("v3 prepare source assignment is unknown"))?;
    let destination = document
        .assignments
        .get(&requested.destination_cell_id)
        .ok_or_else(|| invalid("v3 prepare destination assignment is unknown"))?;
    requested.validate_phase_proofs(source, destination)?;
    if let Some(existing) = document.transfers.get(&requested.transfer_id) {
        if existing.immutable_material_matches(requested) {
            return Ok(document.clone());
        }
        return Err(conflict(
            &requested.transfer_id,
            "v3 transfer ID is already bound to different immutable material",
        ));
    }
    if source.cell_key != requested.source_cell_key
        || destination.cell_key != requested.destination_cell_key
        || source.state != CellAssignmentState::Assigned
        || destination.state != CellAssignmentState::Assigned
        || source.assignment_generation != requested.source_assignment_generation
        || source.authority_fencing_token != requested.source_fencing_token
        || destination.assignment_generation != requested.destination_assignment_generation
        || destination.authority_fencing_token != requested.destination_fencing_token
        || !are_face_neighbors(&requested.source_cell_key, &requested.destination_cell_key)
    {
        return Err(conflict(
            &requested.transfer_id,
            "v3 prepare route does not match current cell authority",
        ));
    }

    let placements = stage_bundled_placement_transition(
        &document.placements,
        &plan,
        &requested.transfer_id,
        &requested.bundle.member_root,
        BundledPlacementTransition::Prepare,
    )?;
    let mut next = document.clone();
    next.placements = placements;
    next.transfers
        .insert(requested.transfer_id.clone(), requested.clone());
    finish_v3_transaction(document, next)
}

fn stage_v3_source_prepared(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    prepare: &DraftGridPrepareProofV2,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    prepare
        .validate_for_directory()
        .map_err(|source| invalid(format!("grid source-prepare proof is invalid: {source}")))?;
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?;
    if prepare.root_aggregate_id != current.root_aggregate_id {
        return Err(conflict(
            transfer_id,
            "v3 source-prepare proof changed its root aggregate",
        ));
    }
    let proof = DirectoryPhaseProofV3 {
        kind: DirectoryPhaseProofKindV3::SourcePrepare,
        transfer_id: prepare.transfer_id.clone(),
        member_root: prepare.member_root.clone(),
        package_hash: prepare.package_hash.clone(),
        cell_id: prepare.source_cell_id.clone(),
        assignment_generation: prepare.assignment_generation,
        fencing_token: prepare.fencing_token,
        event_sequence: prepare.event_sequence,
        event_hash: prepare.event_hash.clone(),
        event_payload_hash: Some(prepare.event_payload_hash.clone()),
        world_hash: prepare.resulting_active_world_hash.clone(),
        quarantine_receipt_hash: None,
        export_proof_hash: None,
        prepare_proof_hash: Some(prepare.proof_hash.clone()),
        quarantine_proof_hash: None,
        import_proof_hash: None,
        activation_proof_hash: None,
        finalization_proof_hash: None,
        destination_import_proof_hash: None,
        prior_event_sequence: Some(prepare.prior_event_sequence),
        prior_event_hash: Some(prepare.prior_event_hash.clone()),
        prior_draft_world_hash: None,
        prior_active_world_hash: Some(prepare.prior_active_world_hash.clone()),
        quarantined_at_unix_ms: None,
        imported_at_unix_ms: None,
        destination_activated_at_unix_ms: None,
        source_export_proof_hash: None,
        source_exported_at_unix_ms: None,
        destination_production_lifecycle_generation: None,
        production_eligibility_root: None,
        mutation_witness_hash: Some(prepare.mutation_witness_hash.clone()),
        ledger_vector: None,
        trusted_time_unix_ms: None,
        prepared_at_simulation_tick: Some(prepare.prepared_at_simulation_tick),
        abort_witness_hash: None,
        abort_proof_hash: None,
        resulting_draft_world_hash: None,
        abort_removed_authority: None,
    };
    apply_v3_source_prepare_proof(document, transfer_id, &proof)
}

fn apply_v3_source_prepare_proof(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    proof: &DirectoryPhaseProofV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if let Some(existing) = &current.source_prepare_proof {
        if existing == proof {
            return Ok(document.clone());
        }
        return Err(conflict(
            transfer_id,
            "v3 source-prepare retry changed its durable proof",
        ));
    }
    if current.phase != TransferPhase::Prepared {
        return Err(conflict(
            transfer_id,
            "v3 source-prepare proof arrived after prepare phase",
        ));
    }
    let mut next = document.clone();
    next.transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists")
        .source_prepare_proof = Some(proof.clone());
    finish_v3_transaction(document, next)
}

fn stage_v3_quarantine(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    quarantine: &DraftGridQuarantineProofV2,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    quarantine.validate_for_directory().map_err(|source| {
        invalid(format!(
            "grid destination-quarantine proof is invalid: {source}"
        ))
    })?;
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?;
    if quarantine.root_aggregate_id != current.root_aggregate_id {
        return Err(conflict(
            transfer_id,
            "v3 destination-quarantine proof changed its root aggregate",
        ));
    }
    let proof = DirectoryPhaseProofV3 {
        kind: DirectoryPhaseProofKindV3::DestinationQuarantine,
        transfer_id: quarantine.transfer_id.clone(),
        member_root: quarantine.member_root.clone(),
        package_hash: quarantine.package_hash.clone(),
        cell_id: quarantine.destination_cell_id.clone(),
        assignment_generation: quarantine.assignment_generation,
        fencing_token: quarantine.fencing_token,
        event_sequence: quarantine.event_sequence,
        event_hash: quarantine.event_hash.clone(),
        event_payload_hash: Some(quarantine.event_payload_hash.clone()),
        world_hash: quarantine.resulting_active_world_hash.clone(),
        quarantine_receipt_hash: Some(quarantine.quarantine_receipt_hash.clone()),
        export_proof_hash: None,
        prepare_proof_hash: None,
        quarantine_proof_hash: Some(quarantine.proof_hash.clone()),
        import_proof_hash: None,
        activation_proof_hash: None,
        finalization_proof_hash: None,
        destination_import_proof_hash: None,
        prior_event_sequence: Some(quarantine.prior_event_sequence),
        prior_event_hash: Some(quarantine.prior_event_hash.clone()),
        prior_draft_world_hash: None,
        prior_active_world_hash: Some(quarantine.prior_active_world_hash.clone()),
        quarantined_at_unix_ms: Some(quarantine.quarantined_at_unix_ms),
        imported_at_unix_ms: None,
        destination_activated_at_unix_ms: None,
        source_export_proof_hash: None,
        source_exported_at_unix_ms: None,
        destination_production_lifecycle_generation: None,
        production_eligibility_root: None,
        mutation_witness_hash: Some(quarantine.mutation_witness_hash.clone()),
        ledger_vector: None,
        trusted_time_unix_ms: None,
        prepared_at_simulation_tick: None,
        abort_witness_hash: None,
        abort_proof_hash: None,
        resulting_draft_world_hash: None,
        abort_removed_authority: None,
    };
    apply_v3_quarantine_proof(document, transfer_id, &proof)
}

fn apply_v3_quarantine_proof(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    proof: &DirectoryPhaseProofV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let receipt_hash = proof.quarantine_receipt_hash.as_deref().ok_or_else(|| {
        invalid("typed destination-quarantine proof omits its receipt commitment")
    })?;
    validate_hash(receipt_hash, "quarantine receipt")?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if current.quarantine_receipt_hash.is_some() || current.destination_quarantine_proof.is_some() {
        if current.quarantine_receipt_hash.as_deref() == Some(receipt_hash)
            && current.destination_quarantine_proof.as_ref() == Some(proof)
        {
            return Ok(document.clone());
        }
        return Err(conflict(
            transfer_id,
            "v3 quarantine retry changed its receipt or durable proof",
        ));
    }
    if !matches!(
        current.phase,
        TransferPhase::Prepared | TransferPhase::Aborting
    ) || current.source_prepare_proof.is_none()
        || current.destination_abort_proof.is_some()
    {
        return Err(conflict(
            transfer_id,
            "v3 transfer is not awaiting its first quarantine proof",
        ));
    }
    let mut next = document.clone();
    let transfer = next
        .transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists");
    transfer.quarantine_receipt_hash = Some(receipt_hash.to_owned());
    transfer.destination_quarantine_proof = Some(proof.clone());
    if transfer.phase == TransferPhase::Prepared {
        transfer.phase = TransferPhase::Quarantined;
    }
    finish_v3_transaction(document, next)
}

fn stage_v3_commit(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    expected_member_root: &str,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    validate_hash(expected_member_root, "member root")?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if current.bundle.member_root != expected_member_root {
        return Err(conflict(
            transfer_id,
            "v3 commit member root changed after prepare",
        ));
    }
    if matches!(
        current.phase,
        TransferPhase::Committed | TransferPhase::Imported | TransferPhase::Finalized
    ) {
        return Ok(document.clone());
    }
    if current.phase != TransferPhase::Quarantined
        || current.source_prepare_proof.is_none()
        || current.destination_quarantine_proof.is_none()
    {
        return Err(conflict(
            transfer_id,
            "v3 transfer is not quarantined with complete durable evidence",
        ));
    }
    let plan = current.bundled_plan()?;
    let placements = stage_bundled_placement_transition(
        &document.placements,
        &plan,
        transfer_id,
        &current.bundle.member_root,
        BundledPlacementTransition::Commit,
    )?;
    let mut next = document.clone();
    next.placements = placements;
    next.transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists")
        .phase = TransferPhase::Committed;
    finish_v3_transaction(document, next)
}

fn stage_v3_destination_imported(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    import: &DraftGridImportProofV2,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    import.validate().map_err(|source| {
        invalid(format!(
            "grid destination-import proof is invalid: {source}"
        ))
    })?;
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?;
    if import.root_aggregate_id != current.root_aggregate_id {
        return Err(conflict(
            transfer_id,
            "v3 destination-import proof changed its root aggregate",
        ));
    }
    let proof = DirectoryPhaseProofV3 {
        kind: DirectoryPhaseProofKindV3::DestinationImport,
        transfer_id: import.transfer_id.clone(),
        member_root: import.member_root.clone(),
        package_hash: import.package_hash.clone(),
        cell_id: import.destination_cell_id.clone(),
        assignment_generation: import.assignment_generation,
        fencing_token: import.fencing_token,
        event_sequence: import.event_sequence,
        event_hash: import.event_hash.clone(),
        event_payload_hash: Some(import.event_payload_hash.clone()),
        world_hash: import.resulting_active_world_hash.clone(),
        quarantine_receipt_hash: Some(import.quarantine_receipt_hash.clone()),
        export_proof_hash: None,
        prepare_proof_hash: None,
        quarantine_proof_hash: None,
        import_proof_hash: Some(import.proof_hash.clone()),
        activation_proof_hash: None,
        finalization_proof_hash: None,
        destination_import_proof_hash: None,
        prior_event_sequence: Some(import.prior_event_sequence),
        prior_event_hash: Some(import.prior_event_hash.clone()),
        prior_draft_world_hash: Some(import.prior_draft_world_hash.clone()),
        prior_active_world_hash: None,
        quarantined_at_unix_ms: Some(import.quarantined_at_unix_ms),
        imported_at_unix_ms: None,
        destination_activated_at_unix_ms: None,
        source_export_proof_hash: Some(import.source_export_proof_hash.clone()),
        source_exported_at_unix_ms: Some(import.source_exported_at_unix_ms),
        destination_production_lifecycle_generation: Some(
            import.destination_production_lifecycle_generation,
        ),
        production_eligibility_root: Some(import.production_eligibility_root.clone()),
        mutation_witness_hash: Some(import.mutation_witness_hash.clone()),
        ledger_vector: Some(import.ledger_vector),
        trusted_time_unix_ms: Some(import.imported_at_unix_ms),
        prepared_at_simulation_tick: None,
        abort_witness_hash: None,
        abort_proof_hash: None,
        resulting_draft_world_hash: None,
        abort_removed_authority: None,
    };
    apply_v3_import_proof(document, transfer_id, &proof)
}

fn apply_v3_import_proof(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    proof: &DirectoryPhaseProofV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if let Some(existing) = &current.import_proof {
        if existing == proof
            && matches!(
                current.phase,
                TransferPhase::Imported | TransferPhase::Finalized
            )
        {
            return Ok(document.clone());
        }
        return Err(conflict(
            transfer_id,
            "v3 import retry changed its durable proof",
        ));
    }
    if current.phase != TransferPhase::Committed || current.source_export_proof.is_none() {
        return Err(conflict(
            transfer_id,
            "v3 transfer is not committed with durable source-export evidence",
        ));
    }
    let plan = current.bundled_plan()?;
    let placements = stage_bundled_placement_transition(
        &document.placements,
        &plan,
        transfer_id,
        &current.bundle.member_root,
        BundledPlacementTransition::Import,
    )?;
    let mut next = document.clone();
    next.placements = placements;
    let transfer = next
        .transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists");
    transfer.import_proof = Some(proof.clone());
    transfer.phase = TransferPhase::Imported;
    finish_v3_transaction(document, next)
}

fn stage_v3_source_exported(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    export: &DraftGridExportProofV2,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    export
        .validate()
        .map_err(|source| invalid(format!("grid source-export proof is invalid: {source}")))?;
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?;
    if export.root_aggregate_id != current.root_aggregate_id {
        return Err(conflict(
            transfer_id,
            "v3 source-export proof changed its root aggregate",
        ));
    }
    let proof = DirectoryPhaseProofV3 {
        kind: DirectoryPhaseProofKindV3::SourceExport,
        transfer_id: export.transfer_id.clone(),
        member_root: export.member_root.clone(),
        package_hash: export.package_hash.clone(),
        cell_id: export.source_cell_id.clone(),
        assignment_generation: export.assignment_generation,
        fencing_token: export.fencing_token,
        event_sequence: export.event_sequence,
        event_hash: export.event_hash.clone(),
        event_payload_hash: Some(export.event_payload_hash.clone()),
        world_hash: export.resulting_active_world_hash.clone(),
        quarantine_receipt_hash: Some(export.quarantine_receipt_hash.clone()),
        export_proof_hash: Some(export.proof_hash.clone()),
        prepare_proof_hash: None,
        quarantine_proof_hash: None,
        import_proof_hash: None,
        activation_proof_hash: None,
        finalization_proof_hash: None,
        destination_import_proof_hash: None,
        prior_event_sequence: Some(export.prior_event_sequence),
        prior_event_hash: Some(export.prior_event_hash.clone()),
        prior_draft_world_hash: Some(export.prior_draft_world_hash.clone()),
        prior_active_world_hash: None,
        quarantined_at_unix_ms: None,
        imported_at_unix_ms: None,
        destination_activated_at_unix_ms: None,
        source_export_proof_hash: None,
        source_exported_at_unix_ms: None,
        destination_production_lifecycle_generation: None,
        production_eligibility_root: None,
        mutation_witness_hash: Some(export.mutation_witness_hash.clone()),
        ledger_vector: Some(export.ledger_vector),
        trusted_time_unix_ms: Some(export.exported_at_unix_ms),
        prepared_at_simulation_tick: None,
        abort_witness_hash: None,
        abort_proof_hash: None,
        resulting_draft_world_hash: None,
        abort_removed_authority: None,
    };
    apply_v3_source_export_proof(document, transfer_id, &proof)
}

fn apply_v3_source_export_proof(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    proof: &DirectoryPhaseProofV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if let Some(existing) = &current.source_export_proof {
        if existing == proof
            && matches!(
                current.phase,
                TransferPhase::Committed | TransferPhase::Imported | TransferPhase::Finalized
            )
        {
            return Ok(document.clone());
        }
        return Err(conflict(
            transfer_id,
            "v3 source-export retry changed its durable proof",
        ));
    }
    if current.phase != TransferPhase::Committed || current.import_proof.is_some() {
        return Err(conflict(
            transfer_id,
            "v3 source export requires a committed transfer before destination import",
        ));
    }
    let mut next = document.clone();
    next.transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists")
        .source_export_proof = Some(proof.clone());
    finish_v3_transaction(document, next)
}

fn stage_v3_destination_activated(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    activation: &DraftGridActivationProofV2,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    activation.validate().map_err(|source| {
        invalid(format!(
            "grid destination-activation proof is invalid: {source}"
        ))
    })?;
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?;
    if activation.root_aggregate_id != current.root_aggregate_id {
        return Err(conflict(
            transfer_id,
            "v3 destination-activation proof changed its root aggregate",
        ));
    }
    let proof = DirectoryPhaseProofV3 {
        kind: DirectoryPhaseProofKindV3::DestinationActivation,
        transfer_id: activation.transfer_id.clone(),
        member_root: activation.member_root.clone(),
        package_hash: activation.package_hash.clone(),
        cell_id: activation.destination_cell_id.clone(),
        assignment_generation: activation.assignment_generation,
        fencing_token: activation.fencing_token,
        event_sequence: activation.event_sequence,
        event_hash: activation.event_hash.clone(),
        event_payload_hash: Some(activation.event_payload_hash.clone()),
        world_hash: activation.resulting_active_world_hash.clone(),
        quarantine_receipt_hash: Some(activation.quarantine_receipt_hash.clone()),
        export_proof_hash: None,
        prepare_proof_hash: None,
        quarantine_proof_hash: None,
        import_proof_hash: None,
        activation_proof_hash: Some(activation.proof_hash.clone()),
        finalization_proof_hash: None,
        destination_import_proof_hash: Some(activation.destination_import_proof_hash.clone()),
        prior_event_sequence: Some(activation.prior_event_sequence),
        prior_event_hash: Some(activation.prior_event_hash.clone()),
        prior_draft_world_hash: None,
        prior_active_world_hash: Some(activation.prior_active_world_hash.clone()),
        quarantined_at_unix_ms: None,
        imported_at_unix_ms: Some(activation.imported_at_unix_ms),
        destination_activated_at_unix_ms: None,
        source_export_proof_hash: None,
        source_exported_at_unix_ms: None,
        destination_production_lifecycle_generation: None,
        production_eligibility_root: Some(activation.production_eligibility_root.clone()),
        mutation_witness_hash: Some(activation.mutation_witness_hash.clone()),
        ledger_vector: None,
        trusted_time_unix_ms: Some(activation.activated_at_unix_ms),
        prepared_at_simulation_tick: None,
        abort_witness_hash: None,
        abort_proof_hash: None,
        resulting_draft_world_hash: None,
        abort_removed_authority: None,
    };
    apply_v3_destination_activation_proof(document, transfer_id, &proof)
}

fn apply_v3_destination_activation_proof(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    proof: &DirectoryPhaseProofV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if let Some(existing) = &current.destination_activation_proof {
        if existing == proof
            && matches!(
                current.phase,
                TransferPhase::Imported | TransferPhase::Finalized
            )
        {
            return Ok(document.clone());
        }
        return Err(conflict(
            transfer_id,
            "v3 destination-activation retry changed its durable proof",
        ));
    }
    if current.phase != TransferPhase::Imported || current.import_proof.is_none() {
        return Err(conflict(
            transfer_id,
            "v3 destination activation requires a durably imported transfer",
        ));
    }
    let mut next = document.clone();
    next.transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists")
        .destination_activation_proof = Some(proof.clone());
    finish_v3_transaction(document, next)
}

fn stage_v3_source_finalized(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    finalization: &DraftGridFinalizationProofV2,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    finalization.validate().map_err(|source| {
        invalid(format!(
            "grid source-finalization proof is invalid: {source}"
        ))
    })?;
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?;
    if finalization.root_aggregate_id != current.root_aggregate_id {
        return Err(conflict(
            transfer_id,
            "v3 source-finalization proof changed its root aggregate",
        ));
    }
    let proof = DirectoryPhaseProofV3 {
        kind: DirectoryPhaseProofKindV3::SourceFinalization,
        transfer_id: finalization.transfer_id.clone(),
        member_root: finalization.member_root.clone(),
        package_hash: finalization.package_hash.clone(),
        cell_id: finalization.source_cell_id.clone(),
        assignment_generation: finalization.assignment_generation,
        fencing_token: finalization.fencing_token,
        event_sequence: finalization.event_sequence,
        event_hash: finalization.event_hash.clone(),
        event_payload_hash: Some(finalization.event_payload_hash.clone()),
        world_hash: finalization.resulting_active_world_hash.clone(),
        quarantine_receipt_hash: None,
        export_proof_hash: None,
        prepare_proof_hash: None,
        quarantine_proof_hash: None,
        import_proof_hash: None,
        activation_proof_hash: Some(finalization.destination_activation_proof_hash.clone()),
        finalization_proof_hash: Some(finalization.proof_hash.clone()),
        destination_import_proof_hash: Some(finalization.destination_import_proof_hash.clone()),
        prior_event_sequence: Some(finalization.prior_event_sequence),
        prior_event_hash: Some(finalization.prior_event_hash.clone()),
        prior_draft_world_hash: None,
        prior_active_world_hash: Some(finalization.prior_active_world_hash.clone()),
        quarantined_at_unix_ms: None,
        imported_at_unix_ms: Some(finalization.imported_at_unix_ms),
        destination_activated_at_unix_ms: Some(finalization.activated_at_unix_ms),
        source_export_proof_hash: Some(finalization.source_export_proof_hash.clone()),
        source_exported_at_unix_ms: Some(finalization.source_exported_at_unix_ms),
        destination_production_lifecycle_generation: None,
        production_eligibility_root: None,
        mutation_witness_hash: Some(finalization.mutation_witness_hash.clone()),
        ledger_vector: None,
        trusted_time_unix_ms: Some(finalization.finalized_at_unix_ms),
        prepared_at_simulation_tick: None,
        abort_witness_hash: None,
        abort_proof_hash: None,
        resulting_draft_world_hash: None,
        abort_removed_authority: None,
    };
    apply_v3_source_finalization_proof(document, transfer_id, &proof)
}

fn apply_v3_source_finalization_proof(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    proof: &DirectoryPhaseProofV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if let Some(existing) = &current.finalization_proof {
        if existing == proof && current.phase == TransferPhase::Finalized {
            return Ok(document.clone());
        }
        return Err(conflict(
            transfer_id,
            "v3 finalization retry changed its durable proof",
        ));
    }
    if current.phase != TransferPhase::Imported
        || current.import_proof.is_none()
        || current.destination_activation_proof.is_none()
    {
        return Err(conflict(
            transfer_id,
            "v3 transfer is not imported with durable destination-activation evidence",
        ));
    }
    let mut next = document.clone();
    let transfer = next
        .transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists");
    transfer.finalization_proof = Some(proof.clone());
    transfer.phase = TransferPhase::Finalized;
    finish_v3_transaction(document, next)
}

fn stage_v3_request_abort(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?;
    if matches!(
        current.phase,
        TransferPhase::Aborting | TransferPhase::Aborted
    ) {
        return Ok(document.clone());
    }
    if !matches!(
        current.phase,
        TransferPhase::Prepared | TransferPhase::Quarantined
    ) {
        return Err(conflict(
            transfer_id,
            "v3 committed transfer cannot abort to its source",
        ));
    }
    let mut next = document.clone();
    next.transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists")
        .phase = TransferPhase::Aborting;
    finish_v3_transaction(document, next)
}

fn stage_v3_abort_cleanup(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    cleanup: &DraftGridAbortCleanupProofV2,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    cleanup
        .validate()
        .map_err(|source| invalid(format!("grid abort cleanup proof is invalid: {source}")))?;
    let proof = DirectoryPhaseProofV3 {
        kind: match cleanup.side {
            DraftGridTransferAbortSideV2::Source => DirectoryPhaseProofKindV3::SourceAbort,
            DraftGridTransferAbortSideV2::Destination => {
                DirectoryPhaseProofKindV3::DestinationAbort
            }
        },
        transfer_id: cleanup.transfer_id.clone(),
        member_root: cleanup.member_root.clone(),
        package_hash: cleanup.package_hash.clone(),
        cell_id: cleanup.cell_id.clone(),
        assignment_generation: cleanup.assignment_generation,
        fencing_token: cleanup.fencing_token,
        event_sequence: cleanup.event_sequence,
        event_hash: cleanup.event_hash.clone(),
        event_payload_hash: Some(cleanup.event_payload_hash.clone()),
        world_hash: cleanup.resulting_draft_world_hash.clone(),
        quarantine_receipt_hash: cleanup.quarantine_receipt_hash.clone(),
        export_proof_hash: None,
        prepare_proof_hash: None,
        quarantine_proof_hash: None,
        import_proof_hash: None,
        activation_proof_hash: None,
        finalization_proof_hash: None,
        destination_import_proof_hash: None,
        prior_event_sequence: Some(cleanup.prior_event_sequence),
        prior_event_hash: Some(cleanup.prior_event_hash.clone()),
        prior_draft_world_hash: Some(cleanup.prior_draft_world_hash.clone()),
        prior_active_world_hash: None,
        quarantined_at_unix_ms: None,
        imported_at_unix_ms: None,
        destination_activated_at_unix_ms: None,
        source_export_proof_hash: None,
        source_exported_at_unix_ms: None,
        destination_production_lifecycle_generation: None,
        production_eligibility_root: None,
        mutation_witness_hash: Some(cleanup.mutation_witness_hash.clone()),
        ledger_vector: None,
        trusted_time_unix_ms: Some(cleanup.trusted_time_unix_ms),
        prepared_at_simulation_tick: None,
        abort_witness_hash: Some(cleanup.abort_witness_hash.clone()),
        abort_proof_hash: Some(cleanup.proof_hash.clone()),
        resulting_draft_world_hash: Some(cleanup.resulting_draft_world_hash.clone()),
        abort_removed_authority: Some(cleanup.removed_authority),
    };
    apply_v3_abort_cleanup_proof(document, transfer_id, &proof)
}

fn apply_v3_abort_cleanup_proof(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
    proof: &DirectoryPhaseProofV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?.clone();
    let existing = match proof.kind {
        DirectoryPhaseProofKindV3::SourceAbort => current.source_abort_proof.as_ref(),
        DirectoryPhaseProofKindV3::DestinationAbort => current.destination_abort_proof.as_ref(),
        _ => {
            return Err(conflict(
                transfer_id,
                "v3 abort cleanup proof has a non-abort role",
            ));
        }
    };
    if let Some(existing) = existing {
        if existing == proof
            && matches!(
                current.phase,
                TransferPhase::Aborting | TransferPhase::Aborted
            )
        {
            return Ok(document.clone());
        }
        return Err(conflict(
            transfer_id,
            "v3 abort cleanup retry changed its durable proof",
        ));
    }
    if current.phase != TransferPhase::Aborting {
        return Err(conflict(
            transfer_id,
            "v3 abort cleanup requires an aborting transfer",
        ));
    }
    let mut next = document.clone();
    let transfer = next
        .transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists");
    match proof.kind {
        DirectoryPhaseProofKindV3::SourceAbort => {
            transfer.source_abort_proof = Some(proof.clone());
        }
        DirectoryPhaseProofKindV3::DestinationAbort => {
            transfer.destination_abort_proof = Some(proof.clone());
        }
        _ => unreachable!("abort proof role was validated"),
    }
    finish_v3_transaction(document, next)
}

fn stage_v3_finalize_abort(
    document: &CellDirectoryDocumentV3,
    transfer_id: &str,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    document.validate()?;
    let current = v3_transfer(document, transfer_id)?.clone();
    if current.phase == TransferPhase::Aborted {
        return Ok(document.clone());
    }
    if current.phase != TransferPhase::Aborting
        || current.source_abort_proof.is_none()
        || current.destination_abort_proof.is_none()
    {
        return Err(conflict(
            transfer_id,
            "v3 abort cannot finalize before both durable cell cleanups",
        ));
    }
    let plan = current.bundled_plan()?;
    let placements = stage_bundled_placement_transition(
        &document.placements,
        &plan,
        transfer_id,
        &current.bundle.member_root,
        BundledPlacementTransition::Abort,
    )?;
    let mut next = document.clone();
    next.placements = placements;
    next.transfers
        .get_mut(transfer_id)
        .expect("validated v3 transfer exists")
        .phase = TransferPhase::Aborted;
    finish_v3_transaction(document, next)
}

fn finish_v3_transaction(
    prior: &CellDirectoryDocumentV3,
    mut next: CellDirectoryDocumentV3,
) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    next.directory_revision = prior
        .directory_revision
        .checked_add(1)
        .ok_or(CellDirectoryError::DirectoryRevisionExhausted)?;
    next.document_hash.clear();
    next.seal()?;
    Ok(next)
}

fn v3_transfer<'a>(
    document: &'a CellDirectoryDocumentV3,
    transfer_id: &str,
) -> Result<&'a CellTransferRecordV3, CellDirectoryError> {
    document
        .transfers
        .get(transfer_id)
        .ok_or_else(|| CellDirectoryError::UnknownTransfer(transfer_id.to_owned()))
}

fn conflict(transfer_id: &str, reason: impl Into<String>) -> CellDirectoryError {
    CellDirectoryError::TransferConflict {
        transfer_id: transfer_id.to_owned(),
        reason: reason.into(),
    }
}

fn member_matches_phase(
    placement: &AggregatePlacementRecord,
    member: &BundledPlacementMember,
    transfer: &CellTransferRecordV3,
    transfer_id: &str,
) -> bool {
    match transfer.phase {
        TransferPhase::Prepared | TransferPhase::Quarantined | TransferPhase::Aborting => {
            placement.state == AggregatePlacementState::Preparing
                && placement.cell_key == transfer.source_cell_key
                && placement.cell_id == transfer.source_cell_id
                && placement.placement_generation == member.prior_placement_generation
                && placement.active_transfer_id.as_deref() == Some(transfer_id)
        }
        TransferPhase::Committed => {
            placement.state == AggregatePlacementState::InTransit
                && placement.cell_key == transfer.destination_cell_key
                && placement.cell_id == transfer.destination_cell_id
                && placement.placement_generation == member.resulting_placement_generation
                && placement.active_transfer_id.as_deref() == Some(transfer_id)
        }
        TransferPhase::Imported | TransferPhase::Finalized => {
            placement.placement_generation > member.resulting_placement_generation
                || (placement.state == AggregatePlacementState::Resident
                    && placement.cell_key == transfer.destination_cell_key
                    && placement.cell_id == transfer.destination_cell_id
                    && placement.placement_generation == member.resulting_placement_generation
                    && placement.active_transfer_id.is_none())
        }
        TransferPhase::Aborted => {
            placement.placement_generation > member.prior_placement_generation
                || (placement.state == AggregatePlacementState::Resident
                    && placement.cell_key == transfer.source_cell_key
                    && placement.cell_id == transfer.source_cell_id
                    && placement.placement_generation == member.prior_placement_generation
                    && placement.active_transfer_id.is_none())
        }
    }
}

fn are_face_neighbors(source: &CellKeyV1, destination: &CellKeyV1) -> bool {
    for offset in [
        [-1, 0, 0],
        [1, 0, 0],
        [0, -1, 0],
        [0, 1, 0],
        [0, 0, -1],
        [0, 0, 1],
    ] {
        if celestial::neighbor_cell_key(source, offset)
            .is_ok_and(|neighbor| &neighbor == destination)
        {
            return true;
        }
    }
    false
}

fn encode_v3(document: &CellDirectoryDocumentV3) -> Result<Vec<u8>, CellDirectoryError> {
    document.validate()?;
    serde_json::to_vec(document)
        .map_err(|source| invalid(format!("v3 directory cannot be encoded: {source}")))
}

fn decode_v3(bytes: &[u8]) -> Result<CellDirectoryDocumentV3, CellDirectoryError> {
    if bytes.len() > MAX_DRAFT_DIRECTORY_V3_BYTES {
        return Err(invalid("v3 directory exceeds its encoded byte bound"));
    }
    let document = serde_json::from_slice::<CellDirectoryDocumentV3>(bytes)
        .map_err(|source| invalid(format!("v3 directory JSON is invalid: {source}")))?;
    document.validate()?;
    Ok(document)
}

fn validate_stable_id(value: &str, kind: &str) -> Result<(), CellDirectoryError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(invalid(format!(
            "v3 {kind} ID is not bounded canonical text"
        )));
    }
    Ok(())
}

fn validate_hash(value: &str, kind: &str) -> Result<(), CellDirectoryError> {
    if !valid_blake3_hex(value) {
        return Err(invalid(format!(
            "v3 {kind} hash is not canonical BLAKE3 text"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> CellDirectoryError {
    CellDirectoryError::InvalidDirectory(message.into())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::cell_directory::{LocalCellDirectory, proof_cell_keys, write_json_atomic};
    use crate::{EVENT_SCHEMA_VERSION, WORLD_SCHEMA_VERSION, universe_manifest};

    fn assigned(cell_key: CellKeyV1, holder: &str, fence: u64) -> CellAssignmentRecord {
        let cell_id = celestial::cell_id(&cell_key).expect("cell ID derives");
        CellAssignmentRecord {
            cell_key,
            cell_id,
            assignment_generation: 1,
            authority_fencing_token: fence,
            fencing_history: BTreeMap::from([(1, fence)]),
            state: CellAssignmentState::Assigned,
            holder_id: Some(holder.into()),
        }
    }

    fn placement(
        id: &str,
        kind: MobileAggregateKind,
        cell_key: &CellKeyV1,
        transfer_id: &str,
    ) -> AggregatePlacementRecord {
        AggregatePlacementRecord {
            aggregate_id: id.into(),
            aggregate_kind: kind,
            cell_key: cell_key.clone(),
            cell_id: celestial::cell_id(cell_key).expect("cell ID derives"),
            placement_generation: 1,
            state: AggregatePlacementState::Preparing,
            active_transfer_id: Some(transfer_id.into()),
        }
    }

    fn prepared_document() -> CellDirectoryDocumentV3 {
        let manifest = universe_manifest(811, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
            .expect("manifest builds");
        let [source, destination] = proof_cell_keys().expect("proof cells derive");
        let source_assignment = assigned(source.clone(), "worker-source", 5);
        let destination_assignment = assigned(destination.clone(), "worker-destination", 9);
        let members = vec![
            BundledPlacementMember {
                aggregate_id: "grid-v3-proof".into(),
                aggregate_kind: MobileAggregateKind::Grid,
                prior_placement_generation: 1,
                resulting_placement_generation: 2,
            },
            BundledPlacementMember {
                aggregate_id: "player-v3-owner".into(),
                aggregate_kind: MobileAggregateKind::Player,
                prior_placement_generation: 1,
                resulting_placement_generation: 2,
            },
            BundledPlacementMember {
                aggregate_id: "player-v3-rider".into(),
                aggregate_kind: MobileAggregateKind::Player,
                prior_placement_generation: 1,
                resulting_placement_generation: 2,
            },
        ];
        let plan = BundledPlacementPlan::new(
            "grid-v3-proof",
            source.clone(),
            destination.clone(),
            members.clone(),
        )
        .expect("plan builds");
        let transfer_id = "transfer-grid-v3-proof";
        let transfer = CellTransferRecordV3 {
            transfer_id: transfer_id.into(),
            root_aggregate_id: plan.root_aggregate_id.clone(),
            source_cell_key: source.clone(),
            source_cell_id: plan.source_cell_id,
            destination_cell_key: destination,
            destination_cell_id: plan.destination_cell_id,
            source_assignment_generation: 1,
            source_fencing_token: 5,
            destination_assignment_generation: 1,
            destination_fencing_token: 9,
            bundle: DirectoryBundleV3 {
                package_schema_version: DRAFT_AGGREGATE_TRANSFER_PACKAGE_SCHEMA_VERSION,
                receipt_schema_version: DRAFT_AGGREGATE_TRANSFER_RECEIPT_SCHEMA_VERSION,
                aggregate_kind: MobileAggregateKind::Grid,
                closure_root: blake3::hash(b"grid closure").to_hex().to_string(),
                conservation_root: blake3::hash(b"grid conservation").to_hex().to_string(),
                package_hash: blake3::hash(b"grid package").to_hex().to_string(),
                members,
                member_root: plan.member_root,
            },
            quarantine_receipt_hash: None,
            source_prepare_proof: None,
            destination_quarantine_proof: None,
            source_export_proof: None,
            import_proof: None,
            destination_activation_proof: None,
            finalization_proof: None,
            source_abort_proof: None,
            destination_abort_proof: None,
            phase: TransferPhase::Prepared,
        };
        let placements = transfer
            .bundle
            .members
            .iter()
            .map(|member| {
                (
                    member.aggregate_id.clone(),
                    placement(
                        &member.aggregate_id,
                        member.aggregate_kind,
                        &source,
                        transfer_id,
                    ),
                )
            })
            .collect();
        let assignments = BTreeMap::from([
            (source_assignment.cell_id.clone(), source_assignment),
            (
                destination_assignment.cell_id.clone(),
                destination_assignment,
            ),
        ]);
        let mut document = CellDirectoryDocumentV3 {
            schema_version: DRAFT_CELL_DIRECTORY_V3_SCHEMA_VERSION,
            universe_id: manifest.universe_id,
            universe_manifest_hash: manifest.manifest_hash,
            directory_revision: 7,
            assignments,
            placements,
            transfers: BTreeMap::from([(transfer_id.into(), transfer)]),
            migration_genesis: None,
            document_hash: String::new(),
        };
        document.seal().expect("draft v3 document seals");
        document
    }

    fn initial_document_and_request() -> (CellDirectoryDocumentV3, CellTransferRecordV3) {
        let prepared = prepared_document();
        let requested = prepared.transfers["transfer-grid-v3-proof"].clone();
        let mut initial = prepared;
        initial.directory_revision -= 1;
        initial.transfers.clear();
        for placement in initial.placements.values_mut() {
            placement.state = AggregatePlacementState::Resident;
            placement.active_transfer_id = None;
        }
        initial.seal().expect("initial v3 document seals");
        (initial, requested)
    }

    fn migrated_floor_document() -> (CellDirectoryDocumentV3, CellTransferRecordV3) {
        let (mut document, requested) = initial_document_and_request();
        document.directory_revision = 1;
        for placement in document.placements.values_mut() {
            placement.placement_generation = 2;
        }
        let placement_floors = document
            .placements
            .iter()
            .map(|(aggregate_id, placement)| {
                (
                    aggregate_id.clone(),
                    MigrationPlacementFloorV1 {
                        aggregate_id: aggregate_id.clone(),
                        aggregate_kind: placement.aggregate_kind,
                        cell_key: placement.cell_key.clone(),
                        cell_id: placement.cell_id.clone(),
                        placement_generation: placement.placement_generation,
                    },
                )
            })
            .collect();
        let target_assignment_root =
            hash_directory_genesis(MIGRATION_ASSIGNMENT_ROOT_DOMAIN, &document.assignments)
                .expect("assignment root derives");
        let target_placement_root =
            hash_directory_genesis(MIGRATION_PLACEMENT_ROOT_DOMAIN, &document.placements)
                .expect("placement root derives");
        let hash = blake3::hash(b"migration-floor-fixture")
            .to_hex()
            .to_string();
        let mut genesis = DirectoryMigrationGenesisV1 {
            schema_version: 1,
            source_directory_revision: 9,
            source_directory_document_hash: hash.clone(),
            source_terminal_transfer_count: 0,
            source_terminal_transfer_root: hash.clone(),
            source_terminal_transfer_ids: BTreeSet::new(),
            source_assignment_root: hash.clone(),
            source_placement_root: hash.clone(),
            target_assignment_root,
            target_placement_root,
            identity_map_root: hash.clone(),
            production_origin_root: hash.clone(),
            normalized_gameplay_root: hash,
            placement_floors,
            record_hash: String::new(),
        };
        genesis.record_hash = genesis.calculate_hash().expect("genesis hash derives");
        document.migration_genesis = Some(genesis);
        document.seal().expect("migrated floor document seals");
        (document, requested)
    }

    fn open_active_history(
        root: &std::path::Path,
        genesis: &CellDirectoryDocumentV3,
    ) -> Result<DraftCellDirectoryHistoryStoreV3, CellDirectoryError> {
        let history_entry = DraftDirectoryHistoryEntryV3::new(String::new(), genesis.clone())?;
        let document_bytes = serde_json::to_vec(genesis)
            .map_err(|source| invalid(format!("test genesis cannot encode: {source}")))?;
        let history_entry_bytes = history_entry.encode_canonical()?;
        let assignment_root =
            hash_directory_genesis(MIGRATION_ASSIGNMENT_ROOT_DOMAIN, &genesis.assignments)?;
        let placement_root =
            hash_directory_genesis(MIGRATION_PLACEMENT_ROOT_DOMAIN, &genesis.placements)?;
        DraftCellDirectoryHistoryStoreV3::open_from_active_head(
            root,
            ActiveDirectoryV3Expectation {
                universe_id: &genesis.universe_id,
                manifest_hash: &genesis.universe_manifest_hash,
                revision: genesis.directory_revision,
                document_hash: &genesis.document_hash,
                history_entry_hash: &history_entry.entry_hash,
                assignment_root: &assignment_root,
                placement_root: &placement_root,
                document_bytes: &document_bytes,
                history_entry_bytes: &history_entry_bytes,
            },
        )
    }

    fn phase_proof(
        transfer: &CellTransferRecordV3,
        kind: DirectoryPhaseProofKindV3,
    ) -> DirectoryPhaseProofV3 {
        let (cell_id, assignment_generation, fencing_token, binds_receipt, event_sequence, label) =
            match kind {
                DirectoryPhaseProofKindV3::SourcePrepare => (
                    transfer.source_cell_id.clone(),
                    transfer.source_assignment_generation,
                    transfer.source_fencing_token,
                    false,
                    41,
                    b"source-prepare".as_slice(),
                ),
                DirectoryPhaseProofKindV3::DestinationQuarantine => (
                    transfer.destination_cell_id.clone(),
                    transfer.destination_assignment_generation,
                    transfer.destination_fencing_token,
                    true,
                    51,
                    b"destination-quarantine".as_slice(),
                ),
                DirectoryPhaseProofKindV3::SourceExport => (
                    transfer.source_cell_id.clone(),
                    transfer.source_assignment_generation,
                    transfer.source_fencing_token,
                    true,
                    42,
                    b"source-export".as_slice(),
                ),
                DirectoryPhaseProofKindV3::DestinationImport => (
                    transfer.destination_cell_id.clone(),
                    transfer.destination_assignment_generation,
                    transfer.destination_fencing_token,
                    true,
                    52,
                    b"destination-import".as_slice(),
                ),
                DirectoryPhaseProofKindV3::DestinationActivation => (
                    transfer.destination_cell_id.clone(),
                    transfer.destination_assignment_generation,
                    transfer.destination_fencing_token,
                    true,
                    53,
                    b"destination-activation".as_slice(),
                ),
                DirectoryPhaseProofKindV3::SourceFinalization => (
                    transfer.source_cell_id.clone(),
                    transfer.source_assignment_generation,
                    transfer.source_fencing_token,
                    false,
                    43,
                    b"source-finalization".as_slice(),
                ),
                DirectoryPhaseProofKindV3::SourceAbort => (
                    transfer.source_cell_id.clone(),
                    transfer.source_assignment_generation,
                    transfer.source_fencing_token,
                    true,
                    42,
                    b"source-abort".as_slice(),
                ),
                DirectoryPhaseProofKindV3::DestinationAbort => (
                    transfer.destination_cell_id.clone(),
                    transfer.destination_assignment_generation,
                    transfer.destination_fencing_token,
                    true,
                    52,
                    b"destination-abort".as_slice(),
                ),
            };
        let world_hash = blake3::hash(&[label, b"-world"].concat())
            .to_hex()
            .to_string();
        let is_abort = matches!(
            kind,
            DirectoryPhaseProofKindV3::SourceAbort | DirectoryPhaseProofKindV3::DestinationAbort
        );
        let mut proof = DirectoryPhaseProofV3 {
            kind,
            transfer_id: transfer.transfer_id.clone(),
            member_root: transfer.bundle.member_root.clone(),
            package_hash: transfer.bundle.package_hash.clone(),
            cell_id,
            assignment_generation,
            fencing_token,
            event_sequence,
            event_hash: blake3::hash(label).to_hex().to_string(),
            event_payload_hash: None,
            world_hash: world_hash.clone(),
            quarantine_receipt_hash: if binds_receipt {
                transfer.quarantine_receipt_hash.clone()
            } else {
                None
            },
            export_proof_hash: None,
            prepare_proof_hash: None,
            quarantine_proof_hash: None,
            import_proof_hash: None,
            activation_proof_hash: None,
            finalization_proof_hash: None,
            destination_import_proof_hash: None,
            prior_event_sequence: None,
            prior_event_hash: None,
            prior_draft_world_hash: None,
            prior_active_world_hash: None,
            quarantined_at_unix_ms: None,
            imported_at_unix_ms: None,
            destination_activated_at_unix_ms: None,
            source_export_proof_hash: None,
            source_exported_at_unix_ms: None,
            destination_production_lifecycle_generation: None,
            production_eligibility_root: None,
            mutation_witness_hash: None,
            ledger_vector: None,
            trusted_time_unix_ms: None,
            prepared_at_simulation_tick: None,
            abort_witness_hash: is_abort.then(|| {
                blake3::hash(&[label, b"-witness"].concat())
                    .to_hex()
                    .to_string()
            }),
            abort_proof_hash: None,
            resulting_draft_world_hash: is_abort.then_some(world_hash),
            abort_removed_authority: is_abort.then_some(true),
        };
        if kind == DirectoryPhaseProofKindV3::SourcePrepare {
            let mut prepare = DraftGridPrepareProofV2 {
                transfer_id: proof.transfer_id.clone(),
                root_aggregate_id: transfer.root_aggregate_id.clone(),
                member_root: proof.member_root.clone(),
                package_hash: proof.package_hash.clone(),
                source_cell_id: proof.cell_id.clone(),
                assignment_generation: proof.assignment_generation,
                fencing_token: proof.fencing_token,
                prior_event_sequence: proof.event_sequence - 1,
                prior_event_hash: blake3::hash(b"directory prepare prior event")
                    .to_hex()
                    .to_string(),
                event_sequence: proof.event_sequence,
                event_hash: proof.event_hash.clone(),
                event_payload_hash: blake3::hash(b"directory prepare event payload")
                    .to_hex()
                    .to_string(),
                prior_active_world_hash: blake3::hash(b"directory prepare prior active world")
                    .to_hex()
                    .to_string(),
                resulting_active_world_hash: proof.world_hash.clone(),
                prepared_at_simulation_tick: 17,
                mutation_witness_hash: String::new(),
                proof_hash: String::new(),
            };
            prepare
                .seal_hashes_for_test()
                .expect("source-prepare proof seals");
            proof.event_payload_hash = Some(prepare.event_payload_hash);
            proof.prepare_proof_hash = Some(prepare.proof_hash);
            proof.prior_event_sequence = Some(prepare.prior_event_sequence);
            proof.prior_event_hash = Some(prepare.prior_event_hash);
            proof.prior_active_world_hash = Some(prepare.prior_active_world_hash);
            proof.mutation_witness_hash = Some(prepare.mutation_witness_hash);
            proof.prepared_at_simulation_tick = Some(prepare.prepared_at_simulation_tick);
        }
        if kind == DirectoryPhaseProofKindV3::DestinationQuarantine {
            let mut quarantine = DraftGridQuarantineProofV2 {
                transfer_id: proof.transfer_id.clone(),
                root_aggregate_id: transfer.root_aggregate_id.clone(),
                member_root: proof.member_root.clone(),
                package_hash: proof.package_hash.clone(),
                destination_cell_id: proof.cell_id.clone(),
                assignment_generation: proof.assignment_generation,
                fencing_token: proof.fencing_token,
                prior_event_sequence: proof.event_sequence - 1,
                prior_event_hash: blake3::hash(b"directory quarantine prior event")
                    .to_hex()
                    .to_string(),
                event_sequence: proof.event_sequence,
                event_hash: proof.event_hash.clone(),
                event_payload_hash: blake3::hash(b"directory quarantine event payload")
                    .to_hex()
                    .to_string(),
                prior_active_world_hash: blake3::hash(b"directory quarantine prior active world")
                    .to_hex()
                    .to_string(),
                resulting_active_world_hash: proof.world_hash.clone(),
                quarantine_receipt_hash: proof
                    .quarantine_receipt_hash
                    .clone()
                    .expect("quarantine proof binds receipt"),
                quarantined_at_unix_ms: 1_800_000_000_051,
                mutation_witness_hash: String::new(),
                proof_hash: String::new(),
            };
            quarantine
                .seal_hashes_for_test()
                .expect("destination-quarantine proof seals");
            proof.event_payload_hash = Some(quarantine.event_payload_hash);
            proof.quarantine_proof_hash = Some(quarantine.proof_hash);
            proof.prior_event_sequence = Some(quarantine.prior_event_sequence);
            proof.prior_event_hash = Some(quarantine.prior_event_hash);
            proof.prior_active_world_hash = Some(quarantine.prior_active_world_hash);
            proof.quarantined_at_unix_ms = Some(quarantine.quarantined_at_unix_ms);
            proof.mutation_witness_hash = Some(quarantine.mutation_witness_hash);
        }
        if kind == DirectoryPhaseProofKindV3::SourceExport {
            let ledger_vector = DraftGridTransferLedgerVectorV2 {
                ore: 3,
                refined_material: 5,
                components: 7,
            };
            let mut export = DraftGridExportProofV2 {
                transfer_id: proof.transfer_id.clone(),
                root_aggregate_id: transfer.root_aggregate_id.clone(),
                member_root: proof.member_root.clone(),
                package_hash: proof.package_hash.clone(),
                source_cell_id: proof.cell_id.clone(),
                assignment_generation: proof.assignment_generation,
                fencing_token: proof.fencing_token,
                prior_event_sequence: proof.event_sequence - 1,
                prior_event_hash: blake3::hash(b"source export prior event")
                    .to_hex()
                    .to_string(),
                event_sequence: proof.event_sequence,
                event_hash: String::new(),
                event_payload_hash: blake3::hash(b"source export event payload")
                    .to_hex()
                    .to_string(),
                prior_draft_world_hash: blake3::hash(b"source export prior draft world")
                    .to_hex()
                    .to_string(),
                resulting_active_world_hash: proof.world_hash.clone(),
                quarantine_receipt_hash: proof
                    .quarantine_receipt_hash
                    .clone()
                    .expect("source export binds quarantine receipt"),
                exported_at_unix_ms: 1_800_000_000_000,
                mutation_witness_hash: blake3::hash(b"source export mutation").to_hex().to_string(),
                proof_hash: String::new(),
                ledger_vector,
            };
            export
                .seal_hashes_for_test()
                .expect("source export proof seals");
            proof.event_hash = export.event_hash;
            proof.event_payload_hash = Some(export.event_payload_hash);
            proof.prior_event_sequence = Some(export.prior_event_sequence);
            proof.prior_event_hash = Some(export.prior_event_hash);
            proof.prior_draft_world_hash = Some(export.prior_draft_world_hash);
            proof.export_proof_hash = Some(export.proof_hash);
            proof.mutation_witness_hash = Some(export.mutation_witness_hash);
            proof.ledger_vector = Some(ledger_vector);
            proof.trusted_time_unix_ms = Some(export.exported_at_unix_ms);
        }
        if kind == DirectoryPhaseProofKindV3::DestinationImport {
            let ledger_vector = DraftGridTransferLedgerVectorV2 {
                ore: 3,
                refined_material: 5,
                components: 7,
            };
            let source_export = transfer.source_export_proof.as_ref();
            let source_export_proof_hash = source_export
                .and_then(|source_export| source_export.export_proof_hash.clone())
                .unwrap_or_else(|| blake3::hash(b"source export proof").to_hex().to_string());
            let source_exported_at_unix_ms = source_export
                .and_then(|source_export| source_export.trusted_time_unix_ms)
                .unwrap_or(1_800_000_000_000);
            let mut import = DraftGridImportProofV2 {
                transfer_id: proof.transfer_id.clone(),
                root_aggregate_id: transfer.root_aggregate_id.clone(),
                member_root: proof.member_root.clone(),
                package_hash: proof.package_hash.clone(),
                destination_cell_id: proof.cell_id.clone(),
                assignment_generation: proof.assignment_generation,
                fencing_token: proof.fencing_token,
                prior_event_sequence: proof.event_sequence - 1,
                prior_event_hash: blake3::hash(b"destination import prior event")
                    .to_hex()
                    .to_string(),
                event_sequence: proof.event_sequence,
                event_hash: String::new(),
                event_payload_hash: blake3::hash(b"destination import event payload")
                    .to_hex()
                    .to_string(),
                prior_draft_world_hash: blake3::hash(b"destination import prior draft world")
                    .to_hex()
                    .to_string(),
                resulting_active_world_hash: proof.world_hash.clone(),
                quarantine_receipt_hash: proof.quarantine_receipt_hash.clone().unwrap_or_else(
                    || {
                        blake3::hash(b"missing premature quarantine receipt")
                            .to_hex()
                            .to_string()
                    },
                ),
                quarantined_at_unix_ms: 1_799_999_999_000,
                source_export_proof_hash,
                source_exported_at_unix_ms,
                imported_at_unix_ms: 1_800_000_001_000,
                destination_production_lifecycle_generation: 1,
                production_eligibility_root: blake3::hash(b"production eligibility")
                    .to_hex()
                    .to_string(),
                mutation_witness_hash: blake3::hash(b"destination import mutation")
                    .to_hex()
                    .to_string(),
                proof_hash: String::new(),
                ledger_vector,
            };
            import
                .seal_hashes_for_test()
                .expect("destination import proof seals");
            proof.event_hash = import.event_hash;
            proof.event_payload_hash = Some(import.event_payload_hash);
            proof.import_proof_hash = Some(import.proof_hash);
            proof.prior_event_sequence = Some(import.prior_event_sequence);
            proof.prior_event_hash = Some(import.prior_event_hash);
            proof.prior_draft_world_hash = Some(import.prior_draft_world_hash);
            proof.quarantined_at_unix_ms = Some(import.quarantined_at_unix_ms);
            proof.source_export_proof_hash = Some(import.source_export_proof_hash);
            proof.source_exported_at_unix_ms = Some(import.source_exported_at_unix_ms);
            proof.destination_production_lifecycle_generation =
                Some(import.destination_production_lifecycle_generation);
            proof.production_eligibility_root = Some(import.production_eligibility_root);
            proof.mutation_witness_hash = Some(import.mutation_witness_hash);
            proof.ledger_vector = Some(ledger_vector);
            proof.trusted_time_unix_ms = Some(import.imported_at_unix_ms);
        }
        if kind == DirectoryPhaseProofKindV3::DestinationActivation {
            let destination_import = transfer.import_proof.as_ref();
            let destination_import_proof_hash = destination_import
                .and_then(|destination_import| destination_import.import_proof_hash.clone())
                .unwrap_or_else(|| {
                    blake3::hash(b"destination import proof")
                        .to_hex()
                        .to_string()
                });
            let imported_at_unix_ms = destination_import
                .and_then(|destination_import| destination_import.trusted_time_unix_ms)
                .unwrap_or(1_800_000_001_000);
            let prior_event_hash = destination_import.map_or_else(
                || {
                    blake3::hash(b"destination activation prior event")
                        .to_hex()
                        .to_string()
                },
                |destination_import| destination_import.event_hash.clone(),
            );
            let prior_active_world_hash = destination_import.map_or_else(
                || {
                    blake3::hash(b"destination activation prior world")
                        .to_hex()
                        .to_string()
                },
                |destination_import| destination_import.world_hash.clone(),
            );
            let production_eligibility_root = destination_import
                .and_then(|destination_import| {
                    destination_import.production_eligibility_root.clone()
                })
                .unwrap_or_else(|| blake3::hash(b"production eligibility").to_hex().to_string());
            let mut activation = DraftGridActivationProofV2 {
                transfer_id: proof.transfer_id.clone(),
                root_aggregate_id: transfer.root_aggregate_id.clone(),
                member_root: proof.member_root.clone(),
                package_hash: proof.package_hash.clone(),
                destination_cell_id: proof.cell_id.clone(),
                assignment_generation: proof.assignment_generation,
                fencing_token: proof.fencing_token,
                prior_event_sequence: proof.event_sequence - 1,
                prior_event_hash,
                event_sequence: proof.event_sequence,
                event_hash: String::new(),
                event_payload_hash: blake3::hash(b"destination activation event payload")
                    .to_hex()
                    .to_string(),
                prior_active_world_hash,
                resulting_active_world_hash: proof.world_hash.clone(),
                quarantine_receipt_hash: proof.quarantine_receipt_hash.clone().unwrap_or_else(
                    || {
                        blake3::hash(b"missing premature quarantine receipt")
                            .to_hex()
                            .to_string()
                    },
                ),
                destination_import_proof_hash,
                imported_at_unix_ms,
                activated_at_unix_ms: imported_at_unix_ms + 1_000,
                production_eligibility_root,
                mutation_witness_hash: blake3::hash(b"destination activation mutation")
                    .to_hex()
                    .to_string(),
                proof_hash: String::new(),
            };
            activation
                .seal_hashes_for_test()
                .expect("destination activation proof seals");
            proof.event_hash = activation.event_hash;
            proof.event_payload_hash = Some(activation.event_payload_hash);
            proof.activation_proof_hash = Some(activation.proof_hash);
            proof.destination_import_proof_hash = Some(activation.destination_import_proof_hash);
            proof.prior_event_sequence = Some(activation.prior_event_sequence);
            proof.prior_event_hash = Some(activation.prior_event_hash);
            proof.prior_active_world_hash = Some(activation.prior_active_world_hash);
            proof.imported_at_unix_ms = Some(activation.imported_at_unix_ms);
            proof.production_eligibility_root = Some(activation.production_eligibility_root);
            proof.mutation_witness_hash = Some(activation.mutation_witness_hash);
            proof.trusted_time_unix_ms = Some(activation.activated_at_unix_ms);
        }
        if kind == DirectoryPhaseProofKindV3::SourceFinalization {
            let source_export = transfer.source_export_proof.as_ref();
            let destination_import = transfer.import_proof.as_ref();
            let destination_activation = transfer.destination_activation_proof.as_ref();
            let source_export_proof_hash = source_export
                .and_then(|export| export.export_proof_hash.clone())
                .unwrap_or_else(|| blake3::hash(b"source export proof").to_hex().to_string());
            let source_exported_at_unix_ms = source_export
                .and_then(|export| export.trusted_time_unix_ms)
                .unwrap_or(1_800_000_000_000);
            let destination_import_proof_hash = destination_import
                .and_then(|import| import.import_proof_hash.clone())
                .unwrap_or_else(|| {
                    blake3::hash(b"destination import proof")
                        .to_hex()
                        .to_string()
                });
            let imported_at_unix_ms = destination_import
                .and_then(|import| import.trusted_time_unix_ms)
                .unwrap_or(1_800_000_001_000);
            let destination_activation_proof_hash = destination_activation
                .and_then(|activation| activation.activation_proof_hash.clone())
                .unwrap_or_else(|| {
                    blake3::hash(b"destination activation proof")
                        .to_hex()
                        .to_string()
                });
            let activated_at_unix_ms = destination_activation
                .and_then(|activation| activation.trusted_time_unix_ms)
                .unwrap_or(imported_at_unix_ms + 1_000);
            let mut finalization = DraftGridFinalizationProofV2 {
                transfer_id: proof.transfer_id.clone(),
                root_aggregate_id: transfer.root_aggregate_id.clone(),
                member_root: proof.member_root.clone(),
                package_hash: proof.package_hash.clone(),
                source_cell_id: proof.cell_id.clone(),
                assignment_generation: proof.assignment_generation,
                fencing_token: proof.fencing_token,
                prior_event_sequence: proof.event_sequence - 1,
                prior_event_hash: source_export.map_or_else(
                    || {
                        blake3::hash(b"source finalization prior event")
                            .to_hex()
                            .to_string()
                    },
                    |export| export.event_hash.clone(),
                ),
                event_sequence: proof.event_sequence,
                event_hash: String::new(),
                event_payload_hash: blake3::hash(b"source finalization event payload")
                    .to_hex()
                    .to_string(),
                prior_active_world_hash: source_export.map_or_else(
                    || {
                        blake3::hash(b"source finalization prior world")
                            .to_hex()
                            .to_string()
                    },
                    |export| export.world_hash.clone(),
                ),
                resulting_active_world_hash: proof.world_hash.clone(),
                source_export_proof_hash,
                source_exported_at_unix_ms,
                destination_import_proof_hash,
                imported_at_unix_ms,
                destination_activation_proof_hash,
                activated_at_unix_ms,
                finalized_at_unix_ms: activated_at_unix_ms + 1_000,
                mutation_witness_hash: blake3::hash(b"source finalization mutation")
                    .to_hex()
                    .to_string(),
                proof_hash: String::new(),
            };
            finalization
                .seal_hashes_for_test()
                .expect("source finalization proof seals");
            proof.event_hash = finalization.event_hash;
            proof.event_payload_hash = Some(finalization.event_payload_hash);
            proof.activation_proof_hash = Some(finalization.destination_activation_proof_hash);
            proof.finalization_proof_hash = Some(finalization.proof_hash);
            proof.destination_import_proof_hash = Some(finalization.destination_import_proof_hash);
            proof.prior_event_sequence = Some(finalization.prior_event_sequence);
            proof.prior_event_hash = Some(finalization.prior_event_hash);
            proof.prior_active_world_hash = Some(finalization.prior_active_world_hash);
            proof.imported_at_unix_ms = Some(finalization.imported_at_unix_ms);
            proof.destination_activated_at_unix_ms = Some(finalization.activated_at_unix_ms);
            proof.source_export_proof_hash = Some(finalization.source_export_proof_hash);
            proof.source_exported_at_unix_ms = Some(finalization.source_exported_at_unix_ms);
            proof.mutation_witness_hash = Some(finalization.mutation_witness_hash);
            proof.trusted_time_unix_ms = Some(finalization.finalized_at_unix_ms);
        }
        if is_abort {
            proof.event_payload_hash = Some(
                blake3::hash(&[label, b"-payload"].concat())
                    .to_hex()
                    .to_string(),
            );
            proof.prior_event_sequence = Some(proof.event_sequence - 1);
            proof.prior_event_hash = Some(
                blake3::hash(&[label, b"-prior-event"].concat())
                    .to_hex()
                    .to_string(),
            );
            proof.prior_draft_world_hash = Some(
                blake3::hash(&[label, b"-prior-draft"].concat())
                    .to_hex()
                    .to_string(),
            );
            proof.mutation_witness_hash = Some(
                blake3::hash(&[label, b"-mutation"].concat())
                    .to_hex()
                    .to_string(),
            );
            proof.trusted_time_unix_ms = Some(1_800_000_000_000 + proof.event_sequence);
            let mut cleanup = DraftGridAbortCleanupProofV2 {
                side: match kind {
                    DirectoryPhaseProofKindV3::SourceAbort => DraftGridTransferAbortSideV2::Source,
                    DirectoryPhaseProofKindV3::DestinationAbort => {
                        DraftGridTransferAbortSideV2::Destination
                    }
                    _ => unreachable!("abort role was classified"),
                },
                transfer_id: proof.transfer_id.clone(),
                member_root: proof.member_root.clone(),
                package_hash: proof.package_hash.clone(),
                cell_id: proof.cell_id.clone(),
                assignment_generation: proof.assignment_generation,
                fencing_token: proof.fencing_token,
                event_sequence: proof.event_sequence,
                event_hash: proof.event_hash.clone(),
                event_payload_hash: proof
                    .event_payload_hash
                    .clone()
                    .expect("abort payload hash exists"),
                prior_event_sequence: proof
                    .prior_event_sequence
                    .expect("abort prior sequence exists"),
                prior_event_hash: proof
                    .prior_event_hash
                    .clone()
                    .expect("abort prior event exists"),
                prior_draft_world_hash: proof
                    .prior_draft_world_hash
                    .clone()
                    .expect("abort prior draft exists"),
                resulting_draft_world_hash: proof.world_hash.clone(),
                trusted_time_unix_ms: proof
                    .trusted_time_unix_ms
                    .expect("abort trusted time exists"),
                mutation_witness_hash: proof
                    .mutation_witness_hash
                    .clone()
                    .expect("abort mutation exists"),
                quarantine_receipt_hash: proof.quarantine_receipt_hash.clone(),
                abort_witness_hash: proof
                    .abort_witness_hash
                    .clone()
                    .expect("abort witness hash exists"),
                removed_authority: proof
                    .abort_removed_authority
                    .expect("abort removal flag exists"),
                proof_hash: String::new(),
            };
            cleanup
                .seal_event_hash()
                .expect("directory abort proof seals");
            proof.event_hash = cleanup.event_hash;
            proof.abort_proof_hash = Some(cleanup.proof_hash);
        }
        proof
    }

    fn abort_cleanup_proof(
        transfer: &CellTransferRecordV3,
        side: DraftGridTransferAbortSideV2,
        removed_authority: bool,
    ) -> DraftGridAbortCleanupProofV2 {
        let kind = match side {
            DraftGridTransferAbortSideV2::Source => DirectoryPhaseProofKindV3::SourceAbort,
            DraftGridTransferAbortSideV2::Destination => {
                DirectoryPhaseProofKindV3::DestinationAbort
            }
        };
        let proof = phase_proof(transfer, kind);
        let mut cleanup = DraftGridAbortCleanupProofV2 {
            side,
            transfer_id: proof.transfer_id,
            member_root: proof.member_root,
            package_hash: proof.package_hash,
            cell_id: proof.cell_id,
            assignment_generation: proof.assignment_generation,
            fencing_token: proof.fencing_token,
            event_sequence: proof.event_sequence,
            event_hash: proof.event_hash,
            event_payload_hash: proof
                .event_payload_hash
                .expect("abort proof has payload hash"),
            prior_event_sequence: proof
                .prior_event_sequence
                .expect("abort proof has prior sequence"),
            prior_event_hash: proof.prior_event_hash.expect("abort proof has prior event"),
            prior_draft_world_hash: proof
                .prior_draft_world_hash
                .expect("abort proof has prior draft"),
            resulting_draft_world_hash: proof
                .resulting_draft_world_hash
                .expect("abort proof has resulting world"),
            trusted_time_unix_ms: proof
                .trusted_time_unix_ms
                .expect("abort proof has trusted time"),
            mutation_witness_hash: proof
                .mutation_witness_hash
                .expect("abort proof has mutation"),
            quarantine_receipt_hash: proof.quarantine_receipt_hash,
            abort_witness_hash: proof
                .abort_witness_hash
                .expect("abort proof has witness hash"),
            removed_authority,
            proof_hash: proof
                .abort_proof_hash
                .expect("abort proof has typed proof hash"),
        };
        cleanup
            .seal_event_hash()
            .expect("abort cleanup proof seals");
        cleanup
    }

    fn complete_directory_history() -> Vec<CellDirectoryDocumentV3> {
        let (initial, requested) = initial_document_and_request();
        let prepared = stage_v3_prepare(&initial, &requested).expect("bundle prepares");
        let prepare_proof = phase_proof(
            &prepared.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourcePrepare,
        )
        .source_prepare_cell_proof(&requested.root_aggregate_id)
        .expect("typed prepare proof reconstructs");
        let source_prepared =
            stage_v3_source_prepared(&prepared, "transfer-grid-v3-proof", &prepare_proof)
                .expect("source prepare proof commits");

        let mut quarantine_material = source_prepared.transfers["transfer-grid-v3-proof"].clone();
        quarantine_material.quarantine_receipt_hash =
            Some(blake3::hash(b"history receipt").to_hex().to_string());
        let quarantine_proof = phase_proof(
            &quarantine_material,
            DirectoryPhaseProofKindV3::DestinationQuarantine,
        )
        .destination_quarantine_cell_proof(&requested.root_aggregate_id)
        .expect("typed quarantine proof reconstructs");
        let quarantined = stage_v3_quarantine(
            &source_prepared,
            "transfer-grid-v3-proof",
            &quarantine_proof,
        )
        .expect("quarantine commits");
        let committed = stage_v3_commit(
            &quarantined,
            "transfer-grid-v3-proof",
            &requested.bundle.member_root,
        )
        .expect("placement commits");

        let export_proof = phase_proof(
            &committed.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourceExport,
        );
        let exported =
            apply_v3_source_export_proof(&committed, "transfer-grid-v3-proof", &export_proof)
                .expect("export proof commits");
        let import_proof = phase_proof(
            &exported.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::DestinationImport,
        )
        .destination_import_cell_proof(&requested.root_aggregate_id)
        .expect("typed import proof reconstructs");
        let imported =
            stage_v3_destination_imported(&exported, "transfer-grid-v3-proof", &import_proof)
                .expect("import proof commits");
        let activation_proof = phase_proof(
            &imported.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::DestinationActivation,
        )
        .destination_activation_cell_proof(&requested.root_aggregate_id)
        .expect("typed activation proof reconstructs");
        let activated =
            stage_v3_destination_activated(&imported, "transfer-grid-v3-proof", &activation_proof)
                .expect("activation proof commits");
        let finalization_proof = phase_proof(
            &activated.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourceFinalization,
        )
        .source_finalization_cell_proof(&requested.root_aggregate_id)
        .expect("typed finalization proof reconstructs");
        let finalized =
            stage_v3_source_finalized(&activated, "transfer-grid-v3-proof", &finalization_proof)
                .expect("finalization proof commits");

        vec![
            initial,
            prepared,
            source_prepared,
            quarantined,
            committed,
            exported,
            imported,
            activated,
            finalized,
        ]
    }

    fn finalized_document() -> CellDirectoryDocumentV3 {
        let mut document = prepared_document();
        let transfer = document
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists");
        transfer.phase = TransferPhase::Finalized;
        transfer.quarantine_receipt_hash = Some(blake3::hash(b"grid receipt").to_hex().to_string());
        let proof_material = transfer.clone();
        transfer.source_prepare_proof = Some(phase_proof(
            &proof_material,
            DirectoryPhaseProofKindV3::SourcePrepare,
        ));
        transfer.destination_quarantine_proof = Some(phase_proof(
            &proof_material,
            DirectoryPhaseProofKindV3::DestinationQuarantine,
        ));
        transfer.source_export_proof = Some(phase_proof(
            &proof_material,
            DirectoryPhaseProofKindV3::SourceExport,
        ));
        let import_material = transfer.clone();
        transfer.import_proof = Some(phase_proof(
            &import_material,
            DirectoryPhaseProofKindV3::DestinationImport,
        ));
        let activation_material = transfer.clone();
        transfer.destination_activation_proof = Some(phase_proof(
            &activation_material,
            DirectoryPhaseProofKindV3::DestinationActivation,
        ));
        let finalization_material = transfer.clone();
        transfer.finalization_proof = Some(phase_proof(
            &finalization_material,
            DirectoryPhaseProofKindV3::SourceFinalization,
        ));
        for member in &transfer.bundle.members {
            let placement = document
                .placements
                .get_mut(&member.aggregate_id)
                .expect("member placement exists");
            placement
                .cell_key
                .clone_from(&transfer.destination_cell_key);
            placement.cell_id.clone_from(&transfer.destination_cell_id);
            placement.placement_generation = member.resulting_placement_generation;
            placement.state = AggregatePlacementState::Resident;
            placement.active_transfer_id = None;
        }
        document.seal().expect("finalized v3 document seals");
        document
    }

    fn aborted_document() -> CellDirectoryDocumentV3 {
        let mut document = prepared_document();
        let transfer = document
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists");
        transfer.phase = TransferPhase::Aborted;
        let proof_material = transfer.clone();
        transfer.source_abort_proof = Some(phase_proof(
            &proof_material,
            DirectoryPhaseProofKindV3::SourceAbort,
        ));
        transfer.destination_abort_proof = Some(phase_proof(
            &proof_material,
            DirectoryPhaseProofKindV3::DestinationAbort,
        ));
        for member in &transfer.bundle.members {
            let placement = document
                .placements
                .get_mut(&member.aggregate_id)
                .expect("member placement exists");
            placement.state = AggregatePlacementState::Resident;
            placement.active_transfer_id = None;
        }
        document.seal().expect("aborted v3 document seals");
        document
    }

    fn two_finalized_transfers_document() -> CellDirectoryDocumentV3 {
        let mut document = finalized_document();
        let first = document.transfers["transfer-grid-v3-proof"].clone();
        let members = first
            .bundle
            .members
            .iter()
            .map(|member| BundledPlacementMember {
                aggregate_id: member.aggregate_id.clone(),
                aggregate_kind: member.aggregate_kind,
                prior_placement_generation: 2,
                resulting_placement_generation: 3,
            })
            .collect::<Vec<_>>();
        let plan = BundledPlacementPlan::new(
            first.root_aggregate_id.clone(),
            first.destination_cell_key.clone(),
            first.source_cell_key.clone(),
            members.clone(),
        )
        .expect("return plan builds");
        let mut second = CellTransferRecordV3 {
            transfer_id: "transfer-grid-v3-return".into(),
            root_aggregate_id: first.root_aggregate_id,
            source_cell_key: plan.source_cell_key.clone(),
            source_cell_id: plan.source_cell_id,
            destination_cell_key: plan.destination_cell_key.clone(),
            destination_cell_id: plan.destination_cell_id,
            source_assignment_generation: 1,
            source_fencing_token: 9,
            destination_assignment_generation: 1,
            destination_fencing_token: 5,
            bundle: DirectoryBundleV3 {
                package_schema_version: DRAFT_AGGREGATE_TRANSFER_PACKAGE_SCHEMA_VERSION,
                receipt_schema_version: DRAFT_AGGREGATE_TRANSFER_RECEIPT_SCHEMA_VERSION,
                aggregate_kind: MobileAggregateKind::Grid,
                closure_root: blake3::hash(b"return closure").to_hex().to_string(),
                conservation_root: blake3::hash(b"return conservation").to_hex().to_string(),
                package_hash: blake3::hash(b"return package").to_hex().to_string(),
                members,
                member_root: plan.member_root,
            },
            quarantine_receipt_hash: Some(blake3::hash(b"return receipt").to_hex().to_string()),
            source_prepare_proof: None,
            destination_quarantine_proof: None,
            source_export_proof: None,
            import_proof: None,
            destination_activation_proof: None,
            finalization_proof: None,
            source_abort_proof: None,
            destination_abort_proof: None,
            phase: TransferPhase::Finalized,
        };
        second.source_prepare_proof = Some(phase_proof(
            &second,
            DirectoryPhaseProofKindV3::SourcePrepare,
        ));
        second.destination_quarantine_proof = Some(phase_proof(
            &second,
            DirectoryPhaseProofKindV3::DestinationQuarantine,
        ));
        second.source_export_proof = Some(phase_proof(
            &second,
            DirectoryPhaseProofKindV3::SourceExport,
        ));
        second.import_proof = Some(phase_proof(
            &second,
            DirectoryPhaseProofKindV3::DestinationImport,
        ));
        second.destination_activation_proof = Some(phase_proof(
            &second,
            DirectoryPhaseProofKindV3::DestinationActivation,
        ));
        second.finalization_proof = Some(phase_proof(
            &second,
            DirectoryPhaseProofKindV3::SourceFinalization,
        ));
        for member in &second.bundle.members {
            let placement = document
                .placements
                .get_mut(&member.aggregate_id)
                .expect("member placement exists");
            placement.cell_key.clone_from(&second.destination_cell_key);
            placement.cell_id.clone_from(&second.destination_cell_id);
            placement.placement_generation = member.resulting_placement_generation;
        }
        document
            .transfers
            .insert(second.transfer_id.clone(), second);
        document.seal().expect("two-transfer history seals");
        document
    }

    #[test]
    fn dormant_v3_codec_round_trips_complete_bundle() {
        let document = prepared_document();
        assert_eq!(
            document.document_hash,
            "19e1bdf1e3f3545fd93990d3184986ed6b77bde0bac45d25f6403e12c5b8baef"
        );
        let bytes = encode_v3(&document).expect("v3 encodes");
        let decoded = decode_v3(&bytes).expect("v3 decodes");
        assert_eq!(decoded, document);
        let transfer = &decoded.transfers["transfer-grid-v3-proof"];
        assert_eq!(transfer.bundle.members.len(), 3);
        assert_eq!(
            transfer.bundle.member_root,
            transfer.bundled_plan().unwrap().member_root
        );
    }

    #[test]
    fn validated_grid_authority_view_carries_exact_phase_and_live_fences() {
        let prepared = prepared_document();
        let view = prepared
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("validated authority view derives");
        assert_eq!(view.phase(), TransferPhase::Prepared);
        assert_eq!(view.live_source_assignment_generation(), 1);
        assert_eq!(view.live_source_fencing_token(), 5);
        assert_eq!(view.live_destination_assignment_generation(), 1);
        assert_eq!(view.live_destination_fencing_token(), 9);
        assert!(!view.source_prepare_proven());
        assert!(!view.destination_quarantine_proven());
        assert!(!view.source_export_proven());
        assert!(!view.destination_import_proven());
        assert!(!view.destination_activation_proven());
        assert!(!view.source_finalization_proven());

        let prepare_proof = phase_proof(
            &prepared.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourcePrepare,
        );
        let prepare_cell_proof = prepare_proof
            .source_prepare_cell_proof(
                &prepared.transfers["transfer-grid-v3-proof"].root_aggregate_id,
            )
            .expect("typed source-prepare proof reconstructs");
        let source_prepared =
            stage_v3_source_prepared(&prepared, "transfer-grid-v3-proof", &prepare_cell_proof)
                .expect("source proof commits");
        let receipt_hash = blake3::hash(b"authority view receipt").to_hex().to_string();
        let mut proof_material = source_prepared.transfers["transfer-grid-v3-proof"].clone();
        proof_material.quarantine_receipt_hash = Some(receipt_hash.clone());
        let quarantine_proof = phase_proof(
            &proof_material,
            DirectoryPhaseProofKindV3::DestinationQuarantine,
        );
        let quarantine_cell_proof = quarantine_proof
            .destination_quarantine_cell_proof(&proof_material.root_aggregate_id)
            .expect("typed destination-quarantine proof reconstructs");
        let quarantined = stage_v3_quarantine(
            &source_prepared,
            "transfer-grid-v3-proof",
            &quarantine_cell_proof,
        )
        .expect("quarantine commits");
        let view = quarantined
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("quarantined authority view derives");
        assert_eq!(view.phase(), TransferPhase::Quarantined);
        assert_eq!(view.quarantine_receipt_hash(), Some(receipt_hash.as_str()));
        assert!(view.source_prepare_proven());
        assert!(view.destination_quarantine_proven());
        assert!(!view.source_export_proven());
        assert!(!view.destination_import_proven());
        assert!(!view.destination_activation_proven());
        assert!(!view.source_finalization_proven());
        assert!(!view.source_abort_proven());
        assert!(!view.destination_abort_proven());
        let cell_authority = DraftGridDirectoryAuthorityV2::from_validated_v3(&view);
        assert!(
            cell_authority.has_valid_phase_matrix(),
            "directory-to-cell authority keeps the exact typed proof matrix"
        );
        let authority_bytes =
            serde_json::to_vec(&cell_authority).expect("cell authority bridge encodes");
        let reopened_authority =
            serde_json::from_slice::<DraftGridDirectoryAuthorityV2>(&authority_bytes)
                .expect("cell authority bridge reopens");
        assert_eq!(reopened_authority, cell_authority);
        assert!(
            reopened_authority.has_valid_phase_matrix(),
            "reopened cell authority retains its proof matrix"
        );

        let finalized = finalized_document();
        let view = finalized
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("finalized authority view derives");
        assert!(view.source_export_proven());
        assert!(view.destination_import_proven());
        assert!(view.destination_activation_proven());
        assert!(view.source_finalization_proven());
        let export = view
            .source_export_proof()
            .expect("source export view exists");
        assert_eq!(export.cell_id(), view.source_cell_id());
        assert_eq!(export.assignment_generation(), 1);
        assert_eq!(export.fencing_token(), 5);
        assert_eq!(export.event_sequence(), 42);
        assert_eq!(
            export.quarantine_receipt_hash(),
            view.quarantine_receipt_hash()
        );
        assert!(valid_blake3_hex(export.event_hash()));
        assert!(valid_blake3_hex(export.world_hash()));
        assert!(export.export_proof_hash().is_some_and(valid_blake3_hex));
        let typed_export = view
            .source_export_cell_proof()
            .expect("typed source export proof reconstructs");
        typed_export
            .validate()
            .expect("typed source export proof validates");
        assert_eq!(
            typed_export.proof_hash,
            export.export_proof_hash().expect("proof hash exists")
        );
        let import = view
            .destination_import_proof()
            .expect("destination import view exists");
        let typed_import = view
            .destination_import_cell_proof()
            .expect("typed destination import proof reconstructs");
        typed_import
            .validate()
            .expect("typed destination import proof validates");

        assert_eq!(
            typed_import.proof_hash,
            import
                .import_proof_hash
                .as_deref()
                .expect("directory import proof hash exists")
        );
        let activation = view
            .destination_activation_proof()
            .expect("destination activation view exists");
        let typed_activation = view
            .destination_activation_cell_proof()
            .expect("typed destination activation proof reconstructs");
        typed_activation
            .validate()
            .expect("typed destination activation proof validates");
        assert_eq!(
            typed_activation.proof_hash,
            activation
                .activation_proof_hash
                .as_deref()
                .expect("directory activation proof hash exists")
        );
        assert!(activation.event_sequence() > import.event_sequence());
        assert!(view.source_finalization_proof().is_some());

        let aborted = aborted_document();
        let view = aborted
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("aborted authority view derives");
        let source_abort = view
            .source_abort_cell_proof()
            .expect("typed source-abort proof reconstructs");
        let destination_abort = view
            .destination_abort_cell_proof()
            .expect("typed destination-abort proof reconstructs");
        source_abort
            .validate()
            .expect("typed source-abort proof validates");
        destination_abort
            .validate()
            .expect("typed destination-abort proof validates");
        assert_eq!(source_abort.side, DraftGridTransferAbortSideV2::Source);
        assert_eq!(
            destination_abort.side,
            DraftGridTransferAbortSideV2::Destination
        );
    }

    #[test]
    fn directory_authorities_require_complete_manifest5_identity() {
        let manifest = crate::manifest_v5::build_validated_manifest_v5(801)
            .expect("manifest-5 capability builds");
        let other_manifest = crate::manifest_v5::build_validated_manifest_v5(802)
            .expect("other manifest-5 capability builds");
        let active_bound = prepared_document();
        let active_authority = active_bound
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("active-bound authority derives");
        assert!(active_authority.bind_manifest_v5(&manifest).is_err());

        let mut document = active_bound;
        document.universe_manifest_hash = manifest.manifest_hash().to_owned();
        document.seal().expect("manifest-5 directory seals");
        let grid_authority = document
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("grid authority derives");
        let bound_grid = grid_authority
            .bind_manifest_v5(&manifest)
            .expect("grid authority binds manifest 5");
        assert_eq!(bound_grid.authority(), &grid_authority);
        assert_eq!(bound_grid.manifest_hash(), manifest.manifest_hash());
        assert!(grid_authority.bind_manifest_v5(&other_manifest).is_err());

        let source_cell = grid_authority
            .source_cell_authority()
            .expect("source cell authority derives");
        let bound_cell = source_cell
            .bind_manifest_v5(&manifest)
            .expect("cell authority binds manifest 5");
        assert_eq!(bound_cell.authority(), &source_cell);
        assert_eq!(bound_cell.manifest_hash(), manifest.manifest_hash());
        assert!(source_cell.bind_manifest_v5(&other_manifest).is_err());

        let mut arbitrary_root = document;
        arbitrary_root.universe_manifest_hash = "ab".repeat(32);
        arbitrary_root
            .seal()
            .expect("syntactically valid arbitrary root seals");
        let arbitrary = arbitrary_root
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("arbitrary-root authority derives");
        assert!(arbitrary.bind_manifest_v5(&manifest).is_err());
    }

    #[test]
    fn validated_cell_authority_is_exact_assigned_historical_capability() {
        let root = tempdir().expect("temporary directory");
        let (initial, _) = initial_document_and_request();
        let source_id = initial
            .assignments
            .keys()
            .next()
            .expect("source assignment exists")
            .clone();
        let universe_id = initial.universe_id.clone();
        let manifest_hash = initial.universe_manifest_hash.clone();
        let mut successor = initial.clone();
        successor.directory_revision += 1;
        let assignment = successor
            .assignments
            .get_mut(&source_id)
            .expect("source assignment exists");
        assignment.assignment_generation += 1;
        assignment.authority_fencing_token += 1;
        assignment.fencing_history.insert(
            assignment.assignment_generation,
            assignment.authority_fencing_token,
        );
        successor.seal().expect("successor authority seals");

        let mut store =
            DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), initial.clone())
                .expect("history initializes");
        store
            .commit(
                initial.directory_revision,
                &initial.document_hash,
                successor.clone(),
            )
            .expect("successor authority commits");
        let old = store
            .resolve_historical_cell_authority(
                initial.directory_revision,
                &initial.document_hash,
                &source_id,
            )
            .expect("old authority resolves exactly");
        let live = store
            .resolve_historical_cell_authority(
                successor.directory_revision,
                &successor.document_hash,
                &source_id,
            )
            .expect("successor authority resolves exactly");
        assert_eq!(old.directory_revision(), initial.directory_revision);
        assert_eq!(old.directory_document_hash(), initial.document_hash);
        assert_eq!(old.universe_id(), universe_id);
        assert_eq!(old.universe_manifest_hash(), manifest_hash);
        assert_eq!(old.cell_id(), source_id);
        assert_eq!(old.cell_key(), &initial.assignments[&source_id].cell_key);
        assert_eq!(old.assignment_generation(), 1);
        assert_eq!(
            old.fencing_token(),
            initial.assignments[&source_id].authority_fencing_token
        );
        assert_eq!(live.assignment_generation(), 2);
        assert_eq!(live.fencing_token(), old.fencing_token() + 1);
        assert_eq!(live.fencing_history().get(&1), Some(&old.fencing_token()));
        let current = store
            .current_cell_authority(&source_id)
            .expect("current-head cell authority resolves");
        assert_eq!(
            current.validated().directory_revision(),
            successor.directory_revision
        );
        assert_eq!(
            current.validated().directory_document_hash(),
            successor.document_hash
        );
        assert_eq!(
            current.validated().holder_id(),
            successor.assignments[&source_id]
                .holder_id
                .as_deref()
                .expect("successor assignment has a holder")
        );
        drop(current);

        let mut sleeping = successor;
        let assignment = sleeping
            .assignments
            .get_mut(&source_id)
            .expect("source assignment exists");
        assignment.state = CellAssignmentState::Sleeping;
        assignment.holder_id = None;
        sleeping.seal().expect("sleeping document seals");
        assert!(sleeping.validated_cell_authority(&source_id).is_err());

        let mut finalized_document = finalized_document();
        let finalized_source_id = finalized_document.transfers["transfer-grid-v3-proof"]
            .source_cell_id
            .clone();
        let finalized_source = finalized_document
            .assignments
            .get_mut(&finalized_source_id)
            .expect("finalized source assignment exists");
        finalized_source.state = CellAssignmentState::Sleeping;
        finalized_source.holder_id = None;
        finalized_document
            .seal()
            .expect("terminal finalized document permits a sleeping source");
        let finalized = finalized_document
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("finalized transfer authority resolves");
        assert!(finalized.source_cell_authority().is_err());
        finalized
            .destination_cell_authority()
            .expect("finalized destination remains assigned");

        let mut aborted_document = aborted_document();
        let aborted_destination_id = aborted_document.transfers["transfer-grid-v3-proof"]
            .destination_cell_id
            .clone();
        let aborted_destination = aborted_document
            .assignments
            .get_mut(&aborted_destination_id)
            .expect("aborted destination assignment exists");
        aborted_destination.state = CellAssignmentState::Sleeping;
        aborted_destination.holder_id = None;
        aborted_document
            .seal()
            .expect("terminal aborted document permits a sleeping destination");
        let aborted = aborted_document
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("aborted transfer authority resolves");
        aborted
            .source_cell_authority()
            .expect("aborted source remains assigned");
        assert!(aborted.destination_cell_authority().is_err());
    }

    #[test]
    fn dormant_v3_transactions_advance_one_atomic_bundle_and_revision_per_phase() {
        let (initial, requested) = initial_document_and_request();
        let prepared = stage_v3_prepare(&initial, &requested).expect("bundle prepares");
        assert_eq!(prepared, prepared_document());
        assert_eq!(
            stage_v3_prepare(&prepared, &requested).expect("prepare retry is exact"),
            prepared
        );

        let prepare_proof = phase_proof(
            &prepared.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourcePrepare,
        );
        let prepare_cell_proof = prepare_proof
            .source_prepare_cell_proof(
                &prepared.transfers["transfer-grid-v3-proof"].root_aggregate_id,
            )
            .expect("typed source-prepare proof reconstructs");
        let source_prepared =
            stage_v3_source_prepared(&prepared, "transfer-grid-v3-proof", &prepare_cell_proof)
                .expect("source proof commits");

        let receipt_hash = blake3::hash(b"transaction receipt").to_hex().to_string();
        let mut quarantine_material = source_prepared.transfers["transfer-grid-v3-proof"].clone();
        quarantine_material.quarantine_receipt_hash = Some(receipt_hash.clone());
        let quarantine_proof = phase_proof(
            &quarantine_material,
            DirectoryPhaseProofKindV3::DestinationQuarantine,
        );
        let quarantine_cell_proof = quarantine_proof
            .destination_quarantine_cell_proof(&quarantine_material.root_aggregate_id)
            .expect("typed destination-quarantine proof reconstructs");
        let quarantined = stage_v3_quarantine(
            &source_prepared,
            "transfer-grid-v3-proof",
            &quarantine_cell_proof,
        )
        .expect("quarantine commits");
        let premature_export_proof = phase_proof(
            &quarantined.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourceExport,
        );
        assert!(
            apply_v3_source_export_proof(
                &quarantined,
                "transfer-grid-v3-proof",
                &premature_export_proof,
            )
            .is_err()
        );

        let committed = stage_v3_commit(
            &quarantined,
            "transfer-grid-v3-proof",
            &requested.bundle.member_root,
        )
        .expect("bundle placement commits");
        assert_eq!(
            stage_v3_commit(
                &committed,
                "transfer-grid-v3-proof",
                &requested.bundle.member_root,
            )
            .expect("commit retry is exact"),
            committed
        );
        for member in &requested.bundle.members {
            let placement = &committed.placements[&member.aggregate_id];
            assert_eq!(placement.state, AggregatePlacementState::InTransit);
            assert_eq!(placement.cell_id, requested.destination_cell_id);
            assert_eq!(
                placement.placement_generation,
                member.resulting_placement_generation
            );
        }

        let import_proof = phase_proof(
            &committed.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::DestinationImport,
        );
        assert!(
            apply_v3_import_proof(&committed, "transfer-grid-v3-proof", &import_proof).is_err()
        );
        let premature_activation_proof = phase_proof(
            &committed.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::DestinationActivation,
        );
        assert!(
            apply_v3_destination_activation_proof(
                &committed,
                "transfer-grid-v3-proof",
                &premature_activation_proof,
            )
            .is_err()
        );

        let export_proof = phase_proof(
            &committed.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourceExport,
        );
        let exported =
            apply_v3_source_export_proof(&committed, "transfer-grid-v3-proof", &export_proof)
                .expect("source export proof commits");
        assert_eq!(
            apply_v3_source_export_proof(&exported, "transfer-grid-v3-proof", &export_proof)
                .expect("source export retry is exact"),
            exported
        );
        let mut changed_export_proof = export_proof.clone();
        changed_export_proof.world_hash = blake3::hash(b"changed source export world")
            .to_hex()
            .to_string();
        assert!(
            apply_v3_source_export_proof(
                &exported,
                "transfer-grid-v3-proof",
                &changed_export_proof,
            )
            .is_err()
        );

        let import_proof = phase_proof(
            &exported.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::DestinationImport,
        );
        let typed_import = import_proof
            .destination_import_cell_proof(&requested.root_aggregate_id)
            .expect("typed import proof reconstructs");
        let imported =
            stage_v3_destination_imported(&exported, "transfer-grid-v3-proof", &typed_import)
                .expect("bundle import commits");
        assert_eq!(
            stage_v3_destination_imported(&imported, "transfer-grid-v3-proof", &typed_import)
                .expect("import retry is exact"),
            imported
        );
        for member in &requested.bundle.members {
            let placement = &imported.placements[&member.aggregate_id];
            assert_eq!(placement.state, AggregatePlacementState::Resident);
            assert!(placement.active_transfer_id.is_none());
        }

        let finalization_proof = phase_proof(
            &imported.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourceFinalization,
        );
        let premature_typed_finalization = finalization_proof
            .source_finalization_cell_proof(&requested.root_aggregate_id)
            .expect("premature typed finalization proof reconstructs");
        assert!(
            stage_v3_source_finalized(
                &imported,
                "transfer-grid-v3-proof",
                &premature_typed_finalization,
            )
            .is_err()
        );

        let activation_proof = phase_proof(
            &imported.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::DestinationActivation,
        );
        let typed_activation = activation_proof
            .destination_activation_cell_proof(&requested.root_aggregate_id)
            .expect("typed activation proof reconstructs");
        let mut forked_activation = typed_activation.clone();
        forked_activation.prior_event_hash = blake3::hash(b"forked directory import event")
            .to_hex()
            .to_string();
        forked_activation.prior_active_world_hash = blake3::hash(b"forked directory import world")
            .to_hex()
            .to_string();
        forked_activation
            .seal_hashes_for_test()
            .expect("forked activation proof reseals");
        assert!(
            stage_v3_destination_activated(
                &imported,
                "transfer-grid-v3-proof",
                &forked_activation,
            )
            .is_err()
        );
        let activated =
            stage_v3_destination_activated(&imported, "transfer-grid-v3-proof", &typed_activation)
                .expect("destination activation proof commits");
        assert_eq!(
            stage_v3_destination_activated(
                &activated,
                "transfer-grid-v3-proof",
                &typed_activation,
            )
            .expect("destination activation retry is exact"),
            activated
        );
        let mut changed_activation_proof = activation_proof.clone();
        changed_activation_proof.event_sequence += 1;
        assert!(
            apply_v3_destination_activation_proof(
                &activated,
                "transfer-grid-v3-proof",
                &changed_activation_proof,
            )
            .is_err()
        );

        let finalization_proof = phase_proof(
            &activated.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourceFinalization,
        );
        let typed_finalization = finalization_proof
            .source_finalization_cell_proof(&requested.root_aggregate_id)
            .expect("typed finalization proof reconstructs");
        let mut substituted_activation = typed_finalization.clone();
        substituted_activation.destination_activation_proof_hash =
            blake3::hash(b"substituted destination activation proof")
                .to_hex()
                .to_string();
        substituted_activation
            .seal_hashes_for_test()
            .expect("substituted finalization proof reseals");
        assert!(
            stage_v3_source_finalized(
                &activated,
                "transfer-grid-v3-proof",
                &substituted_activation,
            )
            .is_err(),
            "finalization cannot substitute a different destination activation"
        );
        let finalized =
            stage_v3_source_finalized(&activated, "transfer-grid-v3-proof", &typed_finalization)
                .expect("bundle finalizes");
        assert_eq!(finalized.directory_revision, initial.directory_revision + 8);
        assert_eq!(
            apply_v3_source_export_proof(&finalized, "transfer-grid-v3-proof", &export_proof)
                .expect("late source-export retry is exact"),
            finalized
        );
        assert_eq!(
            stage_v3_destination_imported(&finalized, "transfer-grid-v3-proof", &typed_import)
                .expect("late import retry is exact"),
            finalized
        );
        assert_eq!(
            stage_v3_destination_activated(
                &finalized,
                "transfer-grid-v3-proof",
                &typed_activation,
            )
            .expect("late destination-activation retry is exact"),
            finalized
        );
        assert_eq!(
            stage_v3_source_finalized(&finalized, "transfer-grid-v3-proof", &typed_finalization,)
                .expect("finalization retry is exact"),
            finalized
        );
        assert_eq!(
            decode_v3(&encode_v3(&finalized).expect("finalized document encodes"))
                .expect("finalized document reopens"),
            finalized
        );
    }

    #[test]
    fn dormant_v3_transactions_abort_every_member_only_after_both_cleanups() {
        let (initial, requested) = initial_document_and_request();
        let prepared = stage_v3_prepare(&initial, &requested).expect("bundle prepares");
        let aborting =
            stage_v3_request_abort(&prepared, "transfer-grid-v3-proof").expect("abort begins");
        assert!(stage_v3_finalize_abort(&aborting, "transfer-grid-v3-proof").is_err());

        let source_proof = abort_cleanup_proof(
            &aborting.transfers["transfer-grid-v3-proof"],
            DraftGridTransferAbortSideV2::Source,
            false,
        );
        let source_clean =
            stage_v3_abort_cleanup(&aborting, "transfer-grid-v3-proof", &source_proof)
                .expect("source cleanup commits");
        let source_clean = decode_v3(
            &encode_v3(&source_clean).expect("source-clean directory encodes for restart"),
        )
        .expect("source-clean directory reopens after restart");
        assert_eq!(
            stage_v3_abort_cleanup(&source_clean, "transfer-grid-v3-proof", &source_proof)
                .expect("source cleanup retry is exact"),
            source_clean
        );
        let mut changed_witness = source_proof.clone();
        changed_witness.abort_witness_hash = "ab".repeat(32);
        assert!(
            stage_v3_abort_cleanup(&source_clean, "transfer-grid-v3-proof", &changed_witness,)
                .is_err()
        );
        let mut changed_world = source_proof.clone();
        changed_world.resulting_draft_world_hash = "cd".repeat(32);
        assert!(
            stage_v3_abort_cleanup(&source_clean, "transfer-grid-v3-proof", &changed_world,)
                .is_err()
        );
        let mut zero_frontier = source_proof.clone();
        zero_frontier.event_sequence = 0;
        assert!(
            stage_v3_abort_cleanup(&source_clean, "transfer-grid-v3-proof", &zero_frontier,)
                .is_err()
        );
        let mut changed_proof_hash = source_proof.clone();
        changed_proof_hash.proof_hash = "ef".repeat(32);
        assert!(
            stage_v3_abort_cleanup(&source_clean, "transfer-grid-v3-proof", &changed_proof_hash,)
                .is_err()
        );
        let destination_proof = abort_cleanup_proof(
            &source_clean.transfers["transfer-grid-v3-proof"],
            DraftGridTransferAbortSideV2::Destination,
            false,
        );
        let both_clean =
            stage_v3_abort_cleanup(&source_clean, "transfer-grid-v3-proof", &destination_proof)
                .expect("destination cleanup commits");
        let aborted = stage_v3_finalize_abort(&both_clean, "transfer-grid-v3-proof")
            .expect("abort finalizes");
        let aborted = decode_v3(&encode_v3(&aborted).expect("aborted directory encodes"))
            .expect("aborted directory reopens");
        for member in &requested.bundle.members {
            assert_eq!(
                aborted.placements[&member.aggregate_id],
                initial.placements[&member.aggregate_id]
            );
        }
        assert_eq!(
            stage_v3_finalize_abort(&aborted, "transfer-grid-v3-proof")
                .expect("abort retry is exact"),
            aborted
        );
        let authority = aborted
            .validated_grid_transfer_authority("transfer-grid-v3-proof")
            .expect("aborted authority reopens");
        assert_eq!(
            authority
                .source_abort_cell_proof()
                .expect("source abort proof survives restart"),
            source_proof
        );
        assert_eq!(
            authority
                .destination_abort_cell_proof()
                .expect("destination abort proof survives restart"),
            destination_proof
        );
    }

    #[test]
    fn aborting_directory_adopts_a_late_durable_quarantine_before_cleanup() {
        let (initial, requested) = initial_document_and_request();
        let prepared = stage_v3_prepare(&initial, &requested).expect("bundle prepares");
        let prepare = phase_proof(
            &prepared.transfers["transfer-grid-v3-proof"],
            DirectoryPhaseProofKindV3::SourcePrepare,
        )
        .source_prepare_cell_proof(&requested.root_aggregate_id)
        .expect("typed prepare proof reconstructs");
        let source_prepared =
            stage_v3_source_prepared(&prepared, "transfer-grid-v3-proof", &prepare)
                .expect("source prepare proof commits");

        let receipt = blake3::hash(b"late quarantine receipt")
            .to_hex()
            .to_string();
        let mut quarantine_material = source_prepared.transfers["transfer-grid-v3-proof"].clone();
        quarantine_material.quarantine_receipt_hash = Some(receipt.clone());
        let late_quarantine = phase_proof(
            &quarantine_material,
            DirectoryPhaseProofKindV3::DestinationQuarantine,
        )
        .destination_quarantine_cell_proof(&requested.root_aggregate_id)
        .expect("late typed quarantine proof reconstructs");

        let aborting = stage_v3_request_abort(&source_prepared, "transfer-grid-v3-proof")
            .expect("directory requests abort before seeing quarantine");
        let source_cleanup = abort_cleanup_proof(
            &aborting.transfers["transfer-grid-v3-proof"],
            DraftGridTransferAbortSideV2::Source,
            true,
        );
        let source_clean =
            stage_v3_abort_cleanup(&aborting, "transfer-grid-v3-proof", &source_cleanup)
                .expect("source cleanup commits before late quarantine is adopted");
        let source_clean =
            decode_v3(&encode_v3(&source_clean).expect("source-clean aborting directory encodes"))
                .expect("source-clean aborting directory reopens");

        let adopted =
            stage_v3_quarantine(&source_clean, "transfer-grid-v3-proof", &late_quarantine)
                .expect("aborting directory adopts exact cell-first quarantine");
        assert_eq!(
            adopted.transfers["transfer-grid-v3-proof"].phase,
            TransferPhase::Aborting
        );
        assert_eq!(
            adopted.transfers["transfer-grid-v3-proof"]
                .quarantine_receipt_hash
                .as_deref(),
            Some(receipt.as_str())
        );
        assert_eq!(
            adopted.transfers["transfer-grid-v3-proof"]
                .source_abort_proof
                .as_ref()
                .and_then(|proof| proof.quarantine_receipt_hash.as_deref()),
            None,
            "an earlier source cleanup remains valid when the destination receipt arrives later"
        );

        let destination_cleanup = abort_cleanup_proof(
            &adopted.transfers["transfer-grid-v3-proof"],
            DraftGridTransferAbortSideV2::Destination,
            true,
        );
        let both_clean =
            stage_v3_abort_cleanup(&adopted, "transfer-grid-v3-proof", &destination_cleanup)
                .expect("destination reservation cleanup commits");
        let aborted = stage_v3_finalize_abort(&both_clean, "transfer-grid-v3-proof")
            .expect("late-quarantine abort finalizes without a stranded reservation");
        assert_eq!(
            aborted.transfers["transfer-grid-v3-proof"].phase,
            TransferPhase::Aborted
        );
    }

    #[test]
    fn dormant_v3_transaction_conflicts_leave_the_prior_document_unchanged() {
        let (initial, mut requested) = initial_document_and_request();
        for member in &mut requested.bundle.members {
            member.prior_placement_generation = 2;
            member.resulting_placement_generation = 3;
        }
        let substituted = BundledPlacementPlan::new(
            requested.root_aggregate_id.clone(),
            requested.source_cell_key.clone(),
            requested.destination_cell_key.clone(),
            requested.bundle.members.clone(),
        )
        .expect("substituted plan is syntactically valid");
        requested.bundle.member_root = substituted.member_root;
        assert!(stage_v3_prepare(&initial, &requested).is_err());
        initial.validate().expect("prior document remains valid");

        let (_, requested) = initial_document_and_request();
        let prepared = stage_v3_prepare(&initial, &requested).expect("bundle prepares");
        assert!(
            apply_v3_import_proof(
                &prepared,
                "transfer-grid-v3-proof",
                &phase_proof(
                    &prepared.transfers["transfer-grid-v3-proof"],
                    DirectoryPhaseProofKindV3::DestinationImport,
                ),
            )
            .is_err()
        );
        assert!(stage_v3_commit(&prepared, "transfer-grid-v3-proof", &"0".repeat(64),).is_err());
        prepared
            .validate()
            .expect("failed transitions leave prepared document valid");
    }

    #[test]
    fn dormant_v3_codec_rejects_unknown_fields_versions_and_hash_tamper() {
        let document = prepared_document();
        let bytes = encode_v3(&document).expect("v3 encodes");
        let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON parses");
        value["unexpected"] = serde_json::json!(true);
        assert!(decode_v3(&serde_json::to_vec(&value).unwrap()).is_err());
        assert!(decode_v3(&vec![b' '; MAX_DRAFT_DIRECTORY_V3_BYTES + 1]).is_err());

        let mut oversized = prepared_document();
        oversized.universe_id = "u".repeat(MAX_DRAFT_DIRECTORY_V3_BYTES);
        oversized.document_hash = oversized.calculate_hash().unwrap();
        assert!(encode_v3(&oversized).is_err());

        let mut wrong_version = document.clone();
        wrong_version.schema_version = 2;
        wrong_version.document_hash = wrong_version.calculate_hash().unwrap();
        assert!(decode_v3(&serde_json::to_vec(&wrong_version).unwrap()).is_err());

        let mut tampered = document;
        tampered
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .unwrap()
            .bundle
            .members
            .pop();
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn dormant_v3_validator_rejects_partial_or_aliased_placement_sets() {
        let mut partial = prepared_document();
        partial.placements.get_mut("player-v3-rider").unwrap().state =
            AggregatePlacementState::Resident;
        partial.document_hash = partial.calculate_hash().unwrap();
        assert!(partial.validate().is_err());

        let mut aliased = prepared_document();
        aliased.placements.insert(
            "player-unrelated".into(),
            placement(
                "player-unrelated",
                MobileAggregateKind::Player,
                &aliased.transfers["transfer-grid-v3-proof"].source_cell_key,
                "transfer-grid-v3-proof",
            ),
        );
        aliased.document_hash = aliased.calculate_hash().unwrap();
        assert!(aliased.validate().is_err());
    }

    #[test]
    fn dormant_v3_terminal_history_rejects_an_unproved_generation_jump() {
        let mut document = finalized_document();
        document
            .placements
            .get_mut("player-v3-rider")
            .expect("rider exists")
            .placement_generation = 3;
        document.document_hash = document.calculate_hash().unwrap();
        assert!(document.validate().is_err());
    }

    #[test]
    fn dormant_v3_terminal_history_accepts_one_chain_and_rejects_ambiguity() {
        let mut document = two_finalized_transfers_document();
        document.validate().expect("durable return chain validates");

        let mut duplicate = document.transfers["transfer-grid-v3-return"].clone();
        duplicate.transfer_id = "transfer-grid-v3-return-alias".into();
        let duplicate_id = duplicate.transfer_id.clone();
        for proof in [
            duplicate.source_prepare_proof.as_mut(),
            duplicate.destination_quarantine_proof.as_mut(),
            duplicate.source_export_proof.as_mut(),
            duplicate.import_proof.as_mut(),
            duplicate.destination_activation_proof.as_mut(),
            duplicate.finalization_proof.as_mut(),
        ]
        .into_iter()
        .flatten()
        {
            proof.transfer_id.clone_from(&duplicate_id);
        }
        document
            .transfers
            .insert(duplicate.transfer_id.clone(), duplicate);
        document.document_hash = document.calculate_hash().unwrap();
        assert!(document.validate().is_err());
    }

    #[test]
    fn dormant_v3_terminal_phases_require_exact_cell_proofs() {
        let mut missing_export = finalized_document();
        missing_export
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .source_export_proof = None;
        missing_export.document_hash = missing_export.calculate_hash().unwrap();
        assert!(missing_export.validate().is_err());

        let mut missing_export_witness = finalized_document();
        missing_export_witness
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .source_export_proof
            .as_mut()
            .expect("source export exists")
            .export_proof_hash = None;
        missing_export_witness.document_hash = missing_export_witness.calculate_hash().unwrap();
        assert!(missing_export_witness.validate().is_err());

        let mut substituted_export_world = finalized_document();
        substituted_export_world
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .source_export_proof
            .as_mut()
            .expect("source export exists")
            .world_hash = "ab".repeat(32);
        substituted_export_world.document_hash = substituted_export_world.calculate_hash().unwrap();
        assert!(substituted_export_world.validate().is_err());

        let mut missing_export_mutation = finalized_document();
        missing_export_mutation
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .source_export_proof
            .as_mut()
            .expect("source export exists")
            .mutation_witness_hash = None;
        missing_export_mutation.document_hash = missing_export_mutation.calculate_hash().unwrap();
        assert!(missing_export_mutation.validate().is_err());

        let mut finalized = finalized_document();
        finalized
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .import_proof = None;
        finalized.document_hash = finalized.calculate_hash().unwrap();
        assert!(finalized.validate().is_err());

        let mut missing_activation = finalized_document();
        missing_activation
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .destination_activation_proof = None;
        missing_activation.document_hash = missing_activation.calculate_hash().unwrap();
        assert!(missing_activation.validate().is_err());

        let mut wrong_fence = finalized_document();
        wrong_fence
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .finalization_proof
            .as_mut()
            .expect("finalization proof exists")
            .fencing_token = 9;
        wrong_fence.document_hash = wrong_fence.calculate_hash().unwrap();
        assert!(wrong_fence.validate().is_err());

        let mut aborted = aborted_document();
        aborted
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .destination_abort_proof = None;
        aborted.document_hash = aborted.calculate_hash().unwrap();
        assert!(aborted.validate().is_err());
    }

    #[test]
    fn dormant_v3_phase_proof_frontiers_advance_in_each_cell() {
        let mut stale_source_event = finalized_document();
        let prepare_sequence = stale_source_event.transfers["transfer-grid-v3-proof"]
            .source_prepare_proof
            .as_ref()
            .expect("source prepare exists")
            .event_sequence;
        stale_source_event
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .source_export_proof
            .as_mut()
            .expect("source export exists")
            .event_sequence = prepare_sequence;
        stale_source_event.document_hash = stale_source_event.calculate_hash().unwrap();
        assert!(stale_source_event.validate().is_err());

        let mut stale_destination_event = finalized_document();
        let import_sequence = stale_destination_event.transfers["transfer-grid-v3-proof"]
            .import_proof
            .as_ref()
            .expect("destination import exists")
            .event_sequence;
        stale_destination_event
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .destination_activation_proof
            .as_mut()
            .expect("destination activation exists")
            .event_sequence = import_sequence;
        stale_destination_event.document_hash = stale_destination_event.calculate_hash().unwrap();
        assert!(stale_destination_event.validate().is_err());

        let mut generation_regression = finalized_document();
        let source_cell_id = generation_regression.transfers["transfer-grid-v3-proof"]
            .source_cell_id
            .clone();
        let source = generation_regression
            .assignments
            .get_mut(&source_cell_id)
            .expect("source assignment exists");
        source.assignment_generation = 2;
        source.authority_fencing_token = 11;
        source.fencing_history.insert(2, 11);
        let export = generation_regression
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .source_export_proof
            .as_mut()
            .expect("source export exists");
        export.assignment_generation = 2;
        export.fencing_token = 11;
        generation_regression.document_hash = generation_regression.calculate_hash().unwrap();
        assert!(generation_regression.validate().is_err());
    }

    #[test]
    fn dormant_v3_proofs_accept_a_historically_fenced_successor() {
        let mut document = finalized_document();
        let transfer = &document.transfers["transfer-grid-v3-proof"];
        let source_cell_id = transfer.source_cell_id.clone();
        let root_aggregate_id = transfer.root_aggregate_id.clone();
        let mut typed_finalization = transfer
            .finalization_proof
            .as_ref()
            .and_then(|proof| proof.source_finalization_cell_proof(&root_aggregate_id))
            .expect("typed finalization proof reconstructs");
        typed_finalization.assignment_generation = 2;
        typed_finalization.fencing_token = 11;
        typed_finalization
            .seal_hashes_for_test()
            .expect("successor finalization proof reseals");
        let source = document
            .assignments
            .get_mut(&source_cell_id)
            .expect("source assignment exists");
        source.assignment_generation = 2;
        source.authority_fencing_token = 11;
        source.fencing_history.insert(2, 11);
        let proof = document
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .finalization_proof
            .as_mut()
            .expect("finalization proof exists");
        proof.assignment_generation = 2;
        proof.fencing_token = 11;
        proof.event_hash.clone_from(&typed_finalization.event_hash);
        proof.finalization_proof_hash = Some(typed_finalization.proof_hash);
        document.document_hash = document.calculate_hash().unwrap();
        document
            .validate()
            .expect("successor proof resolves against durable fence history");
    }

    #[test]
    fn dormant_v3_route_is_one_face_even_across_a_sector_boundary() {
        let [origin, _] = proof_cell_keys().expect("proof cells derive");
        assert!(!are_face_neighbors(&origin, &origin));
        let diagonal =
            celestial::neighbor_cell_key(&origin, [1, 1, 0]).expect("diagonal cell derives");
        assert!(!are_face_neighbors(&origin, &diagonal));
        let mut other_universe =
            celestial::neighbor_cell_key(&origin, [1, 0, 0]).expect("neighbor derives");
        other_universe.universe_id = "other-universe".into();
        assert!(!are_face_neighbors(&origin, &other_universe));

        let mut boundary = origin;
        boundary.cell.x = celestial::CELLS_PER_SECTOR_AXIS - 1;
        let across_sector = celestial::neighbor_cell_key(&boundary, [1, 0, 0])
            .expect("cross-sector face neighbor derives");
        assert_ne!(boundary.sector.x, across_sector.sector.x);
        assert!(are_face_neighbors(&boundary, &across_sector));
    }

    #[test]
    fn v3_member_ids_are_unique_in_the_complete_document() {
        let document = prepared_document();
        let transfer = &document.transfers["transfer-grid-v3-proof"];
        let ids = transfer
            .bundle
            .members
            .iter()
            .map(|member| member.aggregate_id.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), transfer.bundle.members.len());
    }

    #[test]
    fn dormant_v3_persists_atomically_without_touching_the_live_filename() {
        let root = tempdir().expect("temporary directory");
        let path = root.path().join("draft-cell-directory-v3.json");
        let document = prepared_document();
        write_json_atomic(&path, &document).expect("draft v3 persists atomically");
        assert!(!root.path().join("cell-directory.json").exists());
        let reopened = decode_v3(&fs::read(&path).expect("draft v3 reads"))
            .expect("draft v3 reopens through its strict codec");
        assert_eq!(reopened, document);
    }

    #[test]
    fn dormant_v3_initialization_resumes_exact_empty_crash_states() {
        enum EmptyCrashState {
            HistoryOnly,
            EmptyHead,
        }

        for crash_state in [EmptyCrashState::HistoryOnly, EmptyCrashState::EmptyHead] {
            let root = tempdir().expect("temporary directory");
            let initial = complete_directory_history()[0].clone();
            let isolated = root.path().join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
            fs::create_dir_all(&isolated).expect("isolated history directory creates");
            File::create(isolated.join(DRAFT_DIRECTORY_HISTORY_FILE))
                .expect("empty history file creates");
            if matches!(crash_state, EmptyCrashState::EmptyHead) {
                let head = DraftDirectoryHistoryHeadV3::empty(&initial);
                fs::write(
                    isolated.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE),
                    serde_json::to_vec_pretty(&head).expect("empty head encodes"),
                )
                .expect("empty head writes");
                assert!(
                    DraftCellDirectoryHistoryStoreV3::open(
                        root.path(),
                        &initial.universe_id,
                        &initial.universe_manifest_hash,
                    )
                    .is_err(),
                    "ordinary reopen never exposes a store without genesis"
                );
            }

            let store =
                DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), initial.clone())
                    .expect("empty initialization crash resumes");
            assert_eq!(store.current().expect("genesis exists"), &initial);
            assert_eq!(store.head.entry_count, 1);
            drop(store);
            let reopened = DraftCellDirectoryHistoryStoreV3::open(
                root.path(),
                &initial.universe_id,
                &initial.universe_manifest_hash,
            )
            .expect("resumed history reopens normally");
            assert_eq!(reopened.current().expect("genesis exists"), &initial);
        }
    }

    #[test]
    fn dormant_v3_history_resolves_every_exact_authority_after_later_commits() {
        let root = tempdir().expect("temporary directory");
        let documents = complete_directory_history();
        let universe_id = documents[0].universe_id.clone();
        let manifest_hash = documents[0].universe_manifest_hash.clone();
        let mut store =
            DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), documents[0].clone())
                .expect("history initializes");
        assert!(!root.path().join("cell-directory.json").exists());
        assert!(!root.path().join("cell-directory.lock").exists());
        assert!(matches!(
            DraftCellDirectoryHistoryStoreV3::open(root.path(), &universe_id, &manifest_hash),
            Err(CellDirectoryError::WriterAlreadyActive(_))
        ));

        for pair in documents.windows(2) {
            store
                .commit(
                    pair[0].directory_revision,
                    &pair[0].document_hash,
                    pair[1].clone(),
                )
                .expect("history successor commits");
        }
        assert_eq!(
            store.current().expect("tip exists"),
            documents.last().unwrap()
        );
        for document in documents.iter().skip(1) {
            let authority = store
                .resolve_historical_grid_authority(
                    document.directory_revision,
                    &document.document_hash,
                    "transfer-grid-v3-proof",
                )
                .expect("exact historical authority resolves");
            assert_eq!(authority.directory_revision(), document.directory_revision);
            assert_eq!(authority.directory_document_hash(), document.document_hash);
            assert_eq!(
                authority.phase(),
                document.transfers["transfer-grid-v3-proof"].phase
            );
        }
        let current = store
            .current_grid_authority("transfer-grid-v3-proof")
            .expect("current-head grid authority resolves");
        assert_eq!(
            current.validated().directory_revision(),
            documents.last().unwrap().directory_revision
        );
        assert!(current.validated().live_destination_holder_id().is_some());
        drop(current);
        assert!(
            store
                .resolve_historical_grid_authority(
                    documents[1].directory_revision,
                    &documents[2].document_hash,
                    "transfer-grid-v3-proof",
                )
                .is_err(),
            "a different valid historical document hash cannot substitute"
        );

        let expected_count = store.head.entry_count;
        let tip = store.current().expect("tip exists").clone();
        store
            .commit(tip.directory_revision, &tip.document_hash, tip.clone())
            .expect("exact retry is a no-op");
        assert_eq!(store.head.entry_count, expected_count);
        drop(store);

        let reopened =
            DraftCellDirectoryHistoryStoreV3::open(root.path(), &universe_id, &manifest_hash)
                .expect("history reopens");
        assert_eq!(reopened.current().expect("tip exists"), &tip);
        assert_eq!(reopened.head.entry_count, documents.len() as u64);
    }

    #[test]
    fn migration_floor_rejects_pre_genesis_transfer_history() {
        let (mut document, requested) = migrated_floor_document();
        document
            .transfers
            .insert(requested.transfer_id.clone(), requested);
        document.document_hash.clear();
        document.document_hash = document.calculate_hash().expect("document hash derives");
        let error = document
            .validate()
            .expect_err("pre-migration transfer history must reject");
        assert!(
            error
                .to_string()
                .contains("precedes a receipt-bound migration floor")
        );
    }

    #[test]
    fn history_recovery_cannot_reseal_or_replace_migration_genesis() {
        let root = tempdir().expect("temporary directory");
        let (initial, _) = migrated_floor_document();
        let mut successor = initial.clone();
        successor.directory_revision = 2;
        successor.seal().expect("successor seals");
        let mut store =
            DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), initial.clone())
                .expect("history initializes");
        store
            .commit(
                initial.directory_revision,
                &initial.document_hash,
                successor.clone(),
            )
            .expect("valid successor commits");
        drop(store);

        let mut malicious = successor;
        let genesis = malicious
            .migration_genesis
            .as_mut()
            .expect("migration genesis exists");
        genesis.source_directory_document_hash = blake3::hash(b"resealed source directory")
            .to_hex()
            .to_string();
        genesis.record_hash = genesis.calculate_hash().expect("record hash reseals");
        malicious.seal().expect("malicious successor self-seals");

        let first = DraftDirectoryHistoryEntryV3::new(String::new(), initial.clone())
            .expect("first history entry derives");
        let second = DraftDirectoryHistoryEntryV3::new(first.entry_hash.clone(), malicious)
            .expect("second history entry derives");
        let mut history = first.encode_canonical().expect("first history encodes");
        history.push(b'\n');
        let first_length = u64::try_from(history.len()).expect("first length fits");
        history.extend(second.encode_canonical().expect("second history encodes"));
        history.push(b'\n');
        let mut head = DraftDirectoryHistoryHeadV3::empty(&initial);
        head = DraftDirectoryHistoryHeadV3::from_tip(&head, &first, first_length)
            .expect("first head derives");
        head = DraftDirectoryHistoryHeadV3::from_tip(
            &head,
            &second,
            u64::try_from(history.len()).expect("history length fits"),
        )
        .expect("second head derives");
        let isolated = root.path().join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        fs::write(isolated.join(DRAFT_DIRECTORY_HISTORY_FILE), history)
            .expect("resealed history writes");
        fs::write(
            isolated.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE),
            serde_json::to_vec_pretty(&head).expect("head encodes"),
        )
        .expect("resealed head writes");

        let error = DraftCellDirectoryHistoryStoreV3::open(
            root.path(),
            &initial.universe_id,
            &initial.universe_manifest_hash,
        )
        .expect_err("history must reject replaced migration genesis");
        assert!(error.to_string().contains("changed its migration genesis"));
    }

    #[test]
    fn dormant_v3_history_failpoints_recover_prior_or_complete_successor() {
        let documents = complete_directory_history();
        for (failpoint, successor_is_durable) in [
            (DraftDirectoryHistoryFailpointV3::BeforeJournalWrite, false),
            (
                DraftDirectoryHistoryFailpointV3::AfterPartialJournalWrite,
                false,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterJournalLineWriteBeforeSync,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterJournalSyncBeforeHead,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterHeadTempSyncBeforeRename,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterHeadRenameBeforeDirectorySync,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterHeadDirectorySyncBeforeMemory,
                true,
            ),
        ] {
            let root = tempdir().expect("temporary directory");
            let initial = documents[0].clone();
            let successor = documents[1].clone();
            let universe_id = initial.universe_id.clone();
            let manifest_hash = initial.universe_manifest_hash.clone();
            let mut store =
                DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), initial.clone())
                    .expect("history initializes");
            store.set_failpoint(failpoint);
            assert!(
                store
                    .commit(
                        initial.directory_revision,
                        &initial.document_hash,
                        successor.clone(),
                    )
                    .is_err()
            );
            drop(store);

            let mut reopened =
                DraftCellDirectoryHistoryStoreV3::open(root.path(), &universe_id, &manifest_hash)
                    .expect("history recovers");
            let expected = if successor_is_durable {
                &successor
            } else {
                &initial
            };
            assert_eq!(reopened.current().expect("tip exists"), expected);
            if successor_is_durable {
                reopened
                    .commit(
                        successor.directory_revision,
                        &successor.document_hash,
                        successor.clone(),
                    )
                    .expect("durable ambiguous commit reconciles as an exact retry");
            } else {
                reopened
                    .commit(
                        initial.directory_revision,
                        &initial.document_hash,
                        successor.clone(),
                    )
                    .expect("prior state remains appendable");
            }
            assert_eq!(reopened.head.entry_count, 2);
            let history = fs::read_to_string(
                root.path()
                    .join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY)
                    .join(DRAFT_DIRECTORY_HISTORY_FILE),
            )
            .expect("history reads");
            assert_eq!(history.lines().count(), 2);
        }
    }

    fn authority_failpoints() -> [(DraftDirectoryHistoryFailpointV3, bool); 7] {
        [
            (DraftDirectoryHistoryFailpointV3::BeforeJournalWrite, false),
            (
                DraftDirectoryHistoryFailpointV3::AfterPartialJournalWrite,
                false,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterJournalLineWriteBeforeSync,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterJournalSyncBeforeHead,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterHeadTempSyncBeforeRename,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterHeadRenameBeforeDirectorySync,
                true,
            ),
            (
                DraftDirectoryHistoryFailpointV3::AfterHeadDirectorySyncBeforeMemory,
                true,
            ),
        ]
    }

    #[test]
    fn active_v3_claim_failpoints_recover_prior_or_exact_successor() {
        for (failpoint, claim_is_durable) in authority_failpoints() {
            let root = tempdir().expect("temporary directory");
            let (genesis, _) = migrated_floor_document();
            let assignment = genesis
                .assignments
                .values()
                .next()
                .expect("genesis assignment exists")
                .clone();
            let holder = assignment
                .holder_id
                .as_deref()
                .expect("genesis assignment is held");
            let mut store =
                DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis.clone())
                    .expect("history initializes");
            store
                .release_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    holder,
                )
                .expect("assignment releases before claim");
            store.set_failpoint(failpoint);
            assert!(
                store
                    .claim_cell(
                        &assignment.cell_key,
                        assignment.assignment_generation,
                        "worker-claim-successor",
                    )
                    .is_err(),
                "the injected write boundary must be observable"
            );
            drop(store);

            let mut reopened = open_active_history(root.path(), &genesis)
                .expect("signed genesis accepts recovered successors");
            let observed = reopened
                .assignment(&assignment.cell_key)
                .expect("assignment resolves after recovery");
            assert_eq!(
                observed.state,
                if claim_is_durable {
                    CellAssignmentState::Assigned
                } else {
                    CellAssignmentState::Sleeping
                }
            );
            let committed = reopened
                .claim_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    "worker-claim-successor",
                )
                .expect("claim retry resolves exactly once");
            assert_eq!(committed.state, CellAssignmentState::Assigned);
            assert_eq!(
                committed.assignment_generation,
                assignment.assignment_generation + 1
            );
            assert_eq!(reopened.head.entry_count, 3);
            let history = fs::read_to_string(
                root.path()
                    .join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY)
                    .join(DRAFT_DIRECTORY_HISTORY_FILE),
            )
            .expect("active history reads");
            assert_eq!(history.lines().count(), 3);
        }
    }

    #[test]
    fn active_v3_recovery_failpoints_recover_prior_or_exact_successor() {
        for (failpoint, recovery_is_durable) in authority_failpoints() {
            let root = tempdir().expect("temporary directory");
            let (genesis, _) = migrated_floor_document();
            let assignment = genesis
                .assignments
                .values()
                .next()
                .expect("genesis assignment exists")
                .clone();
            let mut store =
                DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis.clone())
                    .expect("history initializes");
            store.set_failpoint(failpoint);
            assert!(
                store
                    .recover_cell(
                        &assignment.cell_key,
                        assignment.assignment_generation,
                        "worker-recovery-successor",
                    )
                    .is_err(),
                "the injected recovery boundary must be observable"
            );
            drop(store);

            let mut reopened = open_active_history(root.path(), &genesis)
                .expect("signed genesis accepts recovered authority history");
            let observed = reopened
                .assignment(&assignment.cell_key)
                .expect("assignment resolves after recovery");
            assert_eq!(
                observed.assignment_generation,
                assignment.assignment_generation + u64::from(recovery_is_durable)
            );
            assert_eq!(
                observed.holder_id.as_deref(),
                if recovery_is_durable {
                    Some("worker-recovery-successor")
                } else {
                    assignment.holder_id.as_deref()
                }
            );
            let committed = reopened
                .recover_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    "worker-recovery-successor",
                )
                .expect("recovery retry resolves exactly once");
            assert_eq!(
                committed.assignment_generation,
                assignment.assignment_generation + 1
            );
            assert_eq!(
                committed.authority_fencing_token,
                assignment.authority_fencing_token + 1
            );
            assert_eq!(reopened.head.entry_count, 2);
        }
    }

    #[test]
    fn active_v3_release_failpoints_recover_prior_or_exact_successor() {
        for (failpoint, release_is_durable) in authority_failpoints() {
            let root = tempdir().expect("temporary directory");
            let (genesis, _) = migrated_floor_document();
            let assignment = genesis
                .assignments
                .values()
                .next()
                .expect("genesis assignment exists")
                .clone();
            let holder = assignment
                .holder_id
                .as_deref()
                .expect("genesis assignment is held");
            let mut store =
                DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis.clone())
                    .expect("history initializes");
            store.set_failpoint(failpoint);
            assert!(
                store
                    .release_cell(
                        &assignment.cell_key,
                        assignment.assignment_generation,
                        holder,
                    )
                    .is_err(),
                "the injected release boundary must be observable"
            );
            drop(store);

            let mut reopened = open_active_history(root.path(), &genesis)
                .expect("signed genesis accepts recovered release history");
            let observed = reopened
                .assignment(&assignment.cell_key)
                .expect("assignment resolves after recovery");
            assert_eq!(
                observed.state,
                if release_is_durable {
                    CellAssignmentState::Sleeping
                } else {
                    CellAssignmentState::Assigned
                }
            );
            let committed = reopened
                .release_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    holder,
                )
                .expect("release retry resolves exactly once");
            assert_eq!(committed.state, CellAssignmentState::Sleeping);
            assert_eq!(
                committed.assignment_generation,
                assignment.assignment_generation
            );
            assert_eq!(
                committed.authority_fencing_token,
                assignment.authority_fencing_token
            );
            assert_eq!(reopened.head.entry_count, 2);
        }
    }

    #[test]
    fn active_v3_recovers_a_synced_successor_with_an_orphaned_head_temp() {
        let root = tempdir().expect("temporary directory");
        let (genesis, _) = migrated_floor_document();
        let assignment = genesis
            .assignments
            .values()
            .next()
            .expect("genesis assignment exists")
            .clone();
        let holder = assignment
            .holder_id
            .as_deref()
            .expect("genesis assignment is held");
        let mut store =
            DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis.clone())
                .expect("history initializes");
        let directory_root = root.path().join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        let head_path = directory_root.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        let prior_head = fs::read(&head_path).expect("prior head reads");
        let released = store
            .release_cell(
                &assignment.cell_key,
                assignment.assignment_generation,
                holder,
            )
            .expect("successor commits before crash simulation");
        let successor_head = fs::read(&head_path).expect("successor head reads");
        let orphan_path = directory_root.join(format!(
            ".{DRAFT_DIRECTORY_HISTORY_HEAD_FILE}.tmp-4242-{}",
            Uuid::new_v4()
        ));
        fs::write(&orphan_path, &successor_head).expect("synced head temp persists");
        fs::write(&head_path, prior_head).expect("durable head remains at predecessor");
        drop(store);

        let reopened = open_active_history(root.path(), &genesis)
            .expect("active recovery removes only the recognized temp and adopts successor");
        assert_eq!(
            reopened
                .assignment(&assignment.cell_key)
                .expect("released assignment resolves"),
            &released
        );
        assert!(!orphan_path.exists());
        assert_eq!(
            fs::read(&head_path).expect("recovered head reads"),
            successor_head
        );
    }

    #[test]
    fn active_v3_unknown_artifact_prevents_head_temp_cleanup() {
        let root = tempdir().expect("temporary directory");
        let (genesis, _) = migrated_floor_document();
        let store =
            DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis.clone())
                .expect("history initializes");
        drop(store);
        let directory_root = root.path().join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        let orphan_path = directory_root.join(format!(
            ".{DRAFT_DIRECTORY_HISTORY_HEAD_FILE}.tmp-4242-{}",
            Uuid::new_v4()
        ));
        let unknown_path = directory_root.join("unexpected-authority-artifact");
        fs::write(&orphan_path, b"recognized stale temp").expect("head temp writes");
        fs::write(&unknown_path, b"unknown").expect("unknown artifact writes");
        let head_path = directory_root.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        let history_path = directory_root.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let head_before = fs::read(&head_path).expect("head snapshots");
        let history_before = fs::read(&history_path).expect("history snapshots");

        assert!(open_active_history(root.path(), &genesis).is_err());
        assert!(orphan_path.exists(), "tamper failure performs no cleanup");
        assert_eq!(fs::read(&head_path).expect("head rereads"), head_before);
        assert_eq!(
            fs::read(&history_path).expect("history rereads"),
            history_before
        );
    }

    #[test]
    fn active_v3_retries_cannot_alias_claim_and_recovery() {
        let root = tempdir().expect("temporary directory");
        let (genesis, _) = migrated_floor_document();
        let assignment = genesis
            .assignments
            .values()
            .next()
            .expect("genesis assignment exists")
            .clone();
        let mut store = DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis)
            .expect("history initializes");
        let recovered = store
            .recover_cell(
                &assignment.cell_key,
                assignment.assignment_generation,
                "worker-recovered",
            )
            .expect("assigned authority recovers");
        assert_eq!(
            store
                .recover_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    "worker-recovered",
                )
                .expect("the exact recovery retry remains idempotent"),
            recovered
        );
        assert!(
            store
                .recover_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    "worker-impostor",
                )
                .is_err(),
            "another holder cannot alias a recovery retry"
        );
        assert!(
            store
                .claim_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    "worker-recovered",
                )
                .is_err(),
            "a recovery successor cannot masquerade as a claim retry"
        );
        let released = store
            .release_cell(
                &assignment.cell_key,
                recovered.assignment_generation,
                "worker-recovered",
            )
            .expect("recovered authority releases");
        assert_eq!(
            store
                .release_cell(
                    &assignment.cell_key,
                    recovered.assignment_generation,
                    "worker-recovered",
                )
                .expect("the exact release retry remains idempotent"),
            released
        );
        assert!(
            store
                .release_cell(
                    &assignment.cell_key,
                    recovered.assignment_generation,
                    "worker-impostor",
                )
                .is_err(),
            "another holder cannot alias a release retry"
        );
        let claimed = store
            .claim_cell(
                &assignment.cell_key,
                recovered.assignment_generation,
                "worker-claimed",
            )
            .expect("sleeping authority claims");
        assert!(
            store
                .recover_cell(
                    &assignment.cell_key,
                    recovered.assignment_generation,
                    "worker-claimed",
                )
                .is_err(),
            "a claim successor cannot masquerade as a recovery retry"
        );
        assert_eq!(
            store
                .claim_cell(
                    &assignment.cell_key,
                    recovered.assignment_generation,
                    "worker-claimed",
                )
                .expect("the exact claim retry remains idempotent"),
            claimed
        );
    }

    #[test]
    fn active_v3_authority_counters_fail_closed_at_exhaustion() {
        for exhaust_generation in [false, true] {
            let root = tempdir().expect("temporary directory");
            let (mut genesis, _) = initial_document_and_request();
            let assignment = genesis
                .assignments
                .values_mut()
                .next()
                .expect("genesis assignment exists");
            if !exhaust_generation {
                assignment.authority_fencing_token = u64::MAX;
                assignment.fencing_history = BTreeMap::from([(
                    assignment.assignment_generation,
                    assignment.authority_fencing_token,
                )]);
            }
            let cell_key = assignment.cell_key.clone();
            let generation = if exhaust_generation {
                u64::MAX
            } else {
                assignment.assignment_generation
            };
            genesis.seal().expect("exhausted counter fixture seals");
            let mut store =
                DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis.clone())
                    .expect("history initializes");
            let error = store
                .recover_cell(&cell_key, generation, "worker-after-exhaustion")
                .expect_err("an exhausted authority counter cannot advance");
            assert!(
                error.to_string().contains("exhausted"),
                "the counter failure remains explicit: {error}"
            );
            assert_eq!(store.current().expect("tip remains"), &genesis);
            assert_eq!(store.head.entry_count, 1);
        }
    }

    #[test]
    fn active_v3_release_rejects_a_nonterminal_transfer_pin() {
        let root = tempdir().expect("temporary directory");
        let genesis = prepared_document();
        let transfer = &genesis.transfers["transfer-grid-v3-proof"];
        let pinned_cell_ids = [
            transfer.source_cell_id.clone(),
            transfer.destination_cell_id.clone(),
        ];
        let mut store = DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis)
            .expect("history initializes");
        let revision = store.current().expect("tip exists").directory_revision;
        for cell_id in pinned_cell_ids {
            let assignment = store
                .current()
                .expect("tip exists")
                .assignments
                .get(&cell_id)
                .expect("pinned assignment exists")
                .clone();
            let error = store
                .release_cell(
                    &assignment.cell_key,
                    assignment.assignment_generation,
                    assignment.holder_id.as_deref().expect("assignment is held"),
                )
                .expect_err("nonterminal transfer pins both cells");
            assert!(error.to_string().contains("nonterminal transfer"));
        }
        assert_eq!(
            store
                .current()
                .expect("rejected release preserves tip")
                .directory_revision,
            revision
        );
    }

    #[test]
    fn active_v3_rejects_a_foreign_genesis_before_recovery_writes() {
        let root = tempdir().expect("temporary directory");
        let (genesis, _) = initial_document_and_request();
        let store =
            DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), genesis.clone())
                .expect("history initializes");
        drop(store);

        let mut foreign = genesis.clone();
        foreign
            .assignments
            .values_mut()
            .next()
            .expect("foreign assignment exists")
            .holder_id = Some("worker-forged".into());
        foreign.seal().expect("foreign genesis self-seals");
        let foreign_entry = DraftDirectoryHistoryEntryV3::new(String::new(), foreign.clone())
            .expect("foreign history entry derives");
        let mut foreign_history = foreign_entry
            .encode_canonical()
            .expect("foreign history encodes");
        foreign_history.push(b'\n');
        let foreign_head = DraftDirectoryHistoryHeadV3::from_tip(
            &DraftDirectoryHistoryHeadV3::empty(&foreign),
            &foreign_entry,
            u64::try_from(foreign_history.len()).expect("history length fits"),
        )
        .expect("foreign head derives");
        let isolated = root.path().join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
        let history_path = isolated.join(DRAFT_DIRECTORY_HISTORY_FILE);
        let head_path = isolated.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
        fs::write(&history_path, &foreign_history).expect("foreign history installs");
        fs::write(
            &head_path,
            serde_json::to_vec_pretty(&foreign_head).expect("foreign head encodes"),
        )
        .expect("foreign head installs");
        let history_before = fs::read(&history_path).expect("history snapshots");
        let head_before = fs::read(&head_path).expect("head snapshots");

        let error = open_active_history(root.path(), &genesis)
            .expect_err("signed activation rejects a self-consistent foreign root");
        assert!(error.to_string().contains("exact active-head genesis"));
        assert_eq!(
            fs::read(&history_path).expect("rejected history rereads"),
            history_before
        );
        assert_eq!(
            fs::read(&head_path).expect("rejected head rereads"),
            head_before
        );
    }

    #[test]
    fn dormant_v3_history_rejects_stale_cas_gaps_and_revision_forks() {
        let root = tempdir().expect("temporary directory");
        let documents = complete_directory_history();
        let initial = documents[0].clone();
        let prepared = documents[1].clone();
        let mut store =
            DraftCellDirectoryHistoryStoreV3::open_or_initialize(root.path(), initial.clone())
                .expect("history initializes");

        assert!(
            store
                .commit(
                    initial.directory_revision + 1,
                    &initial.document_hash,
                    prepared.clone(),
                )
                .is_err()
        );
        let mut gap = prepared.clone();
        gap.directory_revision += 1;
        gap.seal().expect("gap document seals independently");
        assert!(
            store
                .commit(initial.directory_revision, &initial.document_hash, gap)
                .is_err()
        );
        store
            .commit(
                initial.directory_revision,
                &initial.document_hash,
                prepared.clone(),
            )
            .expect("canonical successor commits");

        let mut fork = prepared.clone();
        let source_id = fork.transfers["transfer-grid-v3-proof"]
            .source_cell_id
            .clone();
        fork.assignments
            .get_mut(&source_id)
            .expect("source assignment exists")
            .holder_id = Some("worker-source-fork".into());
        fork.seal().expect("same-revision fork seals independently");
        assert_ne!(fork.document_hash, prepared.document_hash);
        assert!(
            store
                .commit(prepared.directory_revision, &prepared.document_hash, fork)
                .is_err()
        );
        assert!(
            store
                .commit(
                    initial.directory_revision,
                    &initial.document_hash,
                    documents[2].clone(),
                )
                .is_err(),
            "a stale predecessor cannot append after the tip advanced"
        );
        let count = store.head.entry_count;
        store
            .commit(
                prepared.directory_revision,
                &prepared.document_hash,
                prepared.clone(),
            )
            .expect("exact current retry is a no-op");
        assert_eq!(store.head.entry_count, count);
        assert_eq!(store.current().expect("tip exists"), &prepared);
    }

    #[test]
    fn dormant_v3_history_fails_closed_on_chain_head_and_suffix_tampering() {
        enum Tamper {
            RehashMiddlePredecessor,
            DeleteMiddle,
            DeleteHeadedSuffix,
            CompleteGarbageSuffix,
            OversizedCompleteSuffix,
            MissingHead,
            NonCanonicalHead,
            OversizedHead,
        }

        for tamper in [
            Tamper::RehashMiddlePredecessor,
            Tamper::DeleteMiddle,
            Tamper::DeleteHeadedSuffix,
            Tamper::CompleteGarbageSuffix,
            Tamper::OversizedCompleteSuffix,
            Tamper::MissingHead,
            Tamper::NonCanonicalHead,
            Tamper::OversizedHead,
        ] {
            let root = tempdir().expect("temporary directory");
            let documents = complete_directory_history();
            let universe_id = documents[0].universe_id.clone();
            let manifest_hash = documents[0].universe_manifest_hash.clone();
            let mut store = DraftCellDirectoryHistoryStoreV3::open_or_initialize(
                root.path(),
                documents[0].clone(),
            )
            .expect("history initializes");
            for pair in documents[..3].windows(2) {
                store
                    .commit(
                        pair[0].directory_revision,
                        &pair[0].document_hash,
                        pair[1].clone(),
                    )
                    .expect("history successor commits");
            }
            drop(store);

            let isolated = root.path().join(DRAFT_DIRECTORY_HISTORY_SUBDIRECTORY);
            let history_path = isolated.join(DRAFT_DIRECTORY_HISTORY_FILE);
            let head_path = isolated.join(DRAFT_DIRECTORY_HISTORY_HEAD_FILE);
            match tamper {
                Tamper::RehashMiddlePredecessor => {
                    let text = fs::read_to_string(&history_path).expect("history reads");
                    let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
                    let mut middle =
                        serde_json::from_str::<DraftDirectoryHistoryEntryV3>(&lines[1])
                            .expect("middle entry parses");
                    middle.previous_entry_hash = blake3::hash(b"substituted predecessor")
                        .to_hex()
                        .to_string();
                    middle.entry_hash = middle.calculate_hash().expect("entry rehashes");
                    lines[1] = String::from_utf8(middle.encode_canonical().expect("entry encodes"))
                        .expect("entry remains UTF-8");
                    fs::write(&history_path, format!("{}\n", lines.join("\n")))
                        .expect("tampered chain writes");
                }
                Tamper::DeleteMiddle => {
                    let text = fs::read_to_string(&history_path).expect("history reads");
                    let lines = text.lines().collect::<Vec<_>>();
                    fs::write(&history_path, format!("{}\n{}\n", lines[0], lines[2]))
                        .expect("shortened chain writes");
                }
                Tamper::DeleteHeadedSuffix => {
                    let text = fs::read_to_string(&history_path).expect("history reads");
                    let lines = text.lines().collect::<Vec<_>>();
                    fs::write(&history_path, format!("{}\n{}\n", lines[0], lines[1]))
                        .expect("headed suffix deletes");
                }
                Tamper::CompleteGarbageSuffix => {
                    OpenOptions::new()
                        .append(true)
                        .open(&history_path)
                        .expect("history opens")
                        .write_all(b"{\"complete\":\"garbage\"}\n")
                        .expect("garbage suffix writes");
                }
                Tamper::OversizedCompleteSuffix => {
                    let mut file = OpenOptions::new()
                        .append(true)
                        .open(&history_path)
                        .expect("history opens");
                    file.write_all(&vec![b' '; MAX_DRAFT_DIRECTORY_HISTORY_LINE_BYTES + 1])
                        .and_then(|()| file.write_all(b"\n"))
                        .expect("oversized complete suffix writes");
                }
                Tamper::MissingHead => {
                    fs::remove_file(&head_path).expect("head removes");
                }
                Tamper::NonCanonicalHead => {
                    let head = read_history_head_v3(&head_path).expect("head reads");
                    fs::write(
                        &head_path,
                        serde_json::to_vec(&head).expect("compact head encodes"),
                    )
                    .expect("noncanonical head writes");
                }
                Tamper::OversizedHead => {
                    fs::write(
                        &head_path,
                        vec![b' '; MAX_DRAFT_DIRECTORY_HISTORY_HEAD_BYTES as usize + 1],
                    )
                    .expect("oversized head writes");
                }
            }
            assert!(
                DraftCellDirectoryHistoryStoreV3::open(root.path(), &universe_id, &manifest_hash,)
                    .is_err(),
                "tampered history must fail closed"
            );
        }
    }

    #[test]
    fn live_v2_and_dormant_v3_codecs_reject_each_others_documents() {
        assert_eq!(verse_protocol::PROTOCOL_VERSION, 18);
        assert_eq!(verse_protocol::CELL_DIRECTORY_SCHEMA_VERSION, 2);
        assert_eq!(verse_protocol::TRANSFER_PACKAGE_SCHEMA_VERSION, 1);

        let root = tempdir().expect("temporary directory");
        let manifest = universe_manifest(811, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
            .expect("manifest builds");
        let cells = proof_cell_keys().expect("proof cells derive");
        let path = root.path().join("cell-directory.json");

        let directory = LocalCellDirectory::open(root.path(), &manifest, cells.clone())
            .expect("live v2 directory opens");
        let v2_bytes = fs::read(&path).expect("v2 directory reads");
        assert!(decode_v3(&v2_bytes).is_err());
        drop(directory);
        let reopened = LocalCellDirectory::open(root.path(), &manifest, cells.clone())
            .expect("live v2 directory reopens");
        assert_eq!(fs::read(&path).expect("reopened v2 reads"), v2_bytes);
        drop(reopened);

        write_json_atomic(&path, &prepared_document()).expect("draft v3 fixture persists");
        assert!(LocalCellDirectory::open(root.path(), &manifest, cells).is_err());
    }
}
