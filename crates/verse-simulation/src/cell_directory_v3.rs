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
    BundledPlacementPlan, CellAssignmentRecord, CellAssignmentState, CellDirectoryError,
    MobileAggregateKind, TransferPhase,
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
    DestinationImport,
    SourceFinalization,
    SourceAbort,
    DestinationAbort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryPhaseProofV3 {
    kind: DirectoryPhaseProofKindV3,
    transfer_id: String,
    member_root: String,
    package_hash: String,
    cell_id: String,
    assignment_generation: u64,
    fencing_token: u64,
    event_sequence: u64,
    event_hash: String,
    world_hash: String,
    quarantine_receipt_hash: Option<String>,
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
    import_proof: Option<DirectoryPhaseProofV3>,
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

impl CellTransferRecordV3 {
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
            self.import_proof.as_ref(),
            DirectoryPhaseProofKindV3::DestinationImport,
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
            false,
        )?;
        self.validate_phase_proof(
            self.destination_abort_proof.as_ref(),
            DirectoryPhaseProofKindV3::DestinationAbort,
            &self.destination_cell_id,
            self.destination_assignment_generation,
            destination,
            false,
        )?;

        let has_prepare = self.source_prepare_proof.is_some();
        let has_quarantine = self.destination_quarantine_proof.is_some();
        let has_import = self.import_proof.is_some();
        let has_finalization = self.finalization_proof.is_some();
        let has_source_abort = self.source_abort_proof.is_some();
        let has_destination_abort = self.destination_abort_proof.is_some();
        let phase_valid = match self.phase {
            TransferPhase::Prepared => {
                self.quarantine_receipt_hash.is_none()
                    && !has_quarantine
                    && !has_import
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Quarantined | TransferPhase::Committed => {
                self.quarantine_receipt_hash.is_some()
                    && has_prepare
                    && has_quarantine
                    && !has_import
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Imported => {
                self.quarantine_receipt_hash.is_some()
                    && has_prepare
                    && has_quarantine
                    && has_import
                    && !has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Finalized => {
                self.quarantine_receipt_hash.is_some()
                    && has_prepare
                    && has_quarantine
                    && has_import
                    && has_finalization
                    && !has_source_abort
                    && !has_destination_abort
            }
            TransferPhase::Aborting => {
                !has_import
                    && !has_finalization
                    && (self.quarantine_receipt_hash.is_some() == has_quarantine)
                    && (!has_quarantine || has_prepare)
            }
            TransferPhase::Aborted => {
                !has_import
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
            || (binds_receipt && proof.quarantine_receipt_hash != self.quarantine_receipt_hash)
            || (!binds_receipt && proof.quarantine_receipt_hash.is_some())
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
            import_proof: None,
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

    fn phase_proof(
        transfer: &CellTransferRecordV3,
        kind: DirectoryPhaseProofKindV3,
    ) -> DirectoryPhaseProofV3 {
        let (cell_id, assignment_generation, fencing_token, binds_receipt, label) = match kind {
            DirectoryPhaseProofKindV3::SourcePrepare => (
                transfer.source_cell_id.clone(),
                transfer.source_assignment_generation,
                transfer.source_fencing_token,
                false,
                b"source-prepare".as_slice(),
            ),
            DirectoryPhaseProofKindV3::DestinationQuarantine => (
                transfer.destination_cell_id.clone(),
                transfer.destination_assignment_generation,
                transfer.destination_fencing_token,
                true,
                b"destination-quarantine".as_slice(),
            ),
            DirectoryPhaseProofKindV3::DestinationImport => (
                transfer.destination_cell_id.clone(),
                transfer.destination_assignment_generation,
                transfer.destination_fencing_token,
                true,
                b"destination-import".as_slice(),
            ),
            DirectoryPhaseProofKindV3::SourceFinalization => (
                transfer.source_cell_id.clone(),
                transfer.source_assignment_generation,
                transfer.source_fencing_token,
                false,
                b"source-finalization".as_slice(),
            ),
            DirectoryPhaseProofKindV3::SourceAbort => (
                transfer.source_cell_id.clone(),
                transfer.source_assignment_generation,
                transfer.source_fencing_token,
                false,
                b"source-abort".as_slice(),
            ),
            DirectoryPhaseProofKindV3::DestinationAbort => (
                transfer.destination_cell_id.clone(),
                transfer.destination_assignment_generation,
                transfer.destination_fencing_token,
                false,
                b"destination-abort".as_slice(),
            ),
        };
        DirectoryPhaseProofV3 {
            kind,
            transfer_id: transfer.transfer_id.clone(),
            member_root: transfer.bundle.member_root.clone(),
            package_hash: transfer.bundle.package_hash.clone(),
            cell_id,
            assignment_generation,
            fencing_token,
            event_sequence: 41,
            event_hash: blake3::hash(label).to_hex().to_string(),
            world_hash: blake3::hash(&[label, b"-world"].concat())
                .to_hex()
                .to_string(),
            quarantine_receipt_hash: if binds_receipt {
                transfer.quarantine_receipt_hash.clone()
            } else {
                None
            },
        }
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
        transfer.import_proof = Some(phase_proof(
            &proof_material,
            DirectoryPhaseProofKindV3::DestinationImport,
        ));
        transfer.finalization_proof = Some(phase_proof(
            &proof_material,
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
            import_proof: None,
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
        second.import_proof = Some(phase_proof(
            &second,
            DirectoryPhaseProofKindV3::DestinationImport,
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
            "e24a1b61e0684391985fb9700af8061c167cdb943ff67593146e119bd7682528"
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
            duplicate.import_proof.as_mut(),
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
        let mut finalized = finalized_document();
        finalized
            .transfers
            .get_mut("transfer-grid-v3-proof")
            .expect("transfer exists")
            .import_proof = None;
        finalized.document_hash = finalized.calculate_hash().unwrap();
        assert!(finalized.validate().is_err());

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
    fn dormant_v3_proofs_accept_a_historically_fenced_successor() {
        let mut document = finalized_document();
        let source_cell_id = document.transfers["transfer-grid-v3-proof"]
            .source_cell_id
            .clone();
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
