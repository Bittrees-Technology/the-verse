// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant cell-directory-v3 wire model for the protocol-19 activation.
//!
//! Nothing in this module opens or writes the production directory. Keeping
//! this codec private preserves the protocol-18/directory-v2 compatibility
//! boundary while the complete grid-closure tuple is implemented and tested.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use verse_protocol::CellKeyV1;

use crate::cell_directory::{
    AggregatePlacementRecord, AggregatePlacementState, BundledPlacementMember,
    BundledPlacementPlan, BundledPlacementTransition, CellAssignmentRecord, CellAssignmentState,
    CellDirectoryError, MobileAggregateKind, TransferPhase, stage_bundled_placement_transition,
};
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
struct CellDirectoryDocumentV3 {
    schema_version: u32,
    universe_id: String,
    universe_manifest_hash: String,
    directory_revision: u64,
    assignments: BTreeMap<String, CellAssignmentRecord>,
    placements: BTreeMap<String, AggregatePlacementRecord>,
    transfers: BTreeMap<String, CellTransferRecordV3>,
    document_hash: String,
}

/// Read-only capability produced only after the complete dormant directory-v3
/// document (including assignments, fencing history, and phase proofs) passes
/// validation. Grid handoff staging consumes this view instead of reconstructing
/// directory authority from caller-supplied booleans.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ValidatedGridTransferAuthorityV3 {
    directory_revision: u64,
    directory_document_hash: String,
    record: CellTransferRecordV3,
    source_assignment: CellAssignmentRecord,
    destination_assignment: CellAssignmentRecord,
}

impl ValidatedGridTransferAuthorityV3 {
    pub(super) fn directory_revision(&self) -> u64 {
        self.directory_revision
    }

    pub(super) fn directory_document_hash(&self) -> &str {
        &self.directory_document_hash
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

    pub(super) fn live_destination_assignment_generation(&self) -> u64 {
        self.destination_assignment.assignment_generation
    }

    pub(super) fn destination_fencing_history(&self) -> &BTreeMap<u64, u64> {
        &self.destination_assignment.fencing_history
    }

    pub(super) fn live_destination_fencing_token(&self) -> u64 {
        self.destination_assignment.authority_fencing_token
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
        } else if is_abort {
            proof.import_proof_hash.is_none()
                && proof.prior_draft_world_hash.is_some()
                && proof.quarantined_at_unix_ms.is_none()
                && proof.source_export_proof_hash.is_none()
                && proof.source_exported_at_unix_ms.is_none()
                && proof.destination_production_lifecycle_generation.is_none()
        } else if expected_kind == DirectoryPhaseProofKindV3::SourceExport {
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
    pub(super) fn validated_grid_transfer_authority(
        &self,
        transfer_id: &str,
    ) -> Result<ValidatedGridTransferAuthorityV3, CellDirectoryError> {
        self.validate()?;
        let record = v3_transfer(self, transfer_id)?.clone();
        Ok(ValidatedGridTransferAuthorityV3 {
            directory_revision: self.directory_revision,
            directory_document_hash: self.document_hash.clone(),
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
        self.validate_placements()?;
        self.validate_transfers()?;
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
            let plan = transfer.validate_identity(transfer_id)?;
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
                let mut origins = advance_index
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
