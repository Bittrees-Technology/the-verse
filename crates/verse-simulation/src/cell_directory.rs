// SPDX-License-Identifier: AGPL-3.0-or-later

//! Durable local cell assignment directory for the bounded P1.7 proof.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use verse_protocol::{CellKeyV1, UniverseManifestSnapshot};

use crate::celestial;

pub const CELL_DIRECTORY_SCHEMA_VERSION: u32 = verse_protocol::CELL_DIRECTORY_SCHEMA_VERSION;

const DIRECTORY_FILE: &str = "cell-directory.json";
const DIRECTORY_LOCK_FILE: &str = "cell-directory.lock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellAssignmentState {
    Sleeping,
    Claiming,
    Assigned,
    Releasing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellAssignmentRecord {
    pub cell_key: CellKeyV1,
    pub cell_id: String,
    pub assignment_generation: u64,
    pub authority_fencing_token: u64,
    pub fencing_history: BTreeMap<u64, u64>,
    pub state: CellAssignmentState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MobileAggregateKind {
    Player,
    Grid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregatePlacementState {
    Resident,
    Preparing,
    InTransit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AggregatePlacementRecord {
    pub aggregate_id: String,
    pub aggregate_kind: MobileAggregateKind,
    pub cell_key: CellKeyV1,
    pub cell_id: String,
    pub placement_generation: u64,
    pub state: AggregatePlacementState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_transfer_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct BundledPlacementMember {
    pub aggregate_id: String,
    pub aggregate_kind: MobileAggregateKind,
    pub prior_placement_generation: u64,
    pub resulting_placement_generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(not(test), allow(dead_code))]
pub struct BundledPlacementPlan {
    pub root_aggregate_id: String,
    pub source_cell_key: CellKeyV1,
    pub source_cell_id: String,
    pub destination_cell_key: CellKeyV1,
    pub destination_cell_id: String,
    pub members: Vec<BundledPlacementMember>,
    pub member_root: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum BundledPlacementTransition {
    Prepare,
    Commit,
    Import,
    Abort,
}

#[derive(Serialize)]
#[cfg_attr(not(test), allow(dead_code))]
struct BundledPlacementRootMaterial<'a> {
    root_aggregate_id: &'a str,
    source_cell_key: &'a CellKeyV1,
    source_cell_id: &'a str,
    destination_cell_key: &'a CellKeyV1,
    destination_cell_id: &'a str,
    members: &'a [BundledPlacementMember],
}

#[cfg_attr(not(test), allow(dead_code))]
impl BundledPlacementPlan {
    pub fn new(
        root_aggregate_id: impl Into<String>,
        source_cell_key: CellKeyV1,
        destination_cell_key: CellKeyV1,
        members: Vec<BundledPlacementMember>,
    ) -> Result<Self, CellDirectoryError> {
        let root_aggregate_id = root_aggregate_id.into();
        let source_cell_id = celestial::cell_id(&source_cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        let destination_cell_id = celestial::cell_id(&destination_cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        let mut plan = Self {
            root_aggregate_id,
            source_cell_key,
            source_cell_id,
            destination_cell_key,
            destination_cell_id,
            members,
            member_root: String::new(),
        };
        plan.member_root = plan.calculate_member_root()?;
        plan.validate()?;
        Ok(plan)
    }

    pub fn calculate_member_root(&self) -> Result<String, CellDirectoryError> {
        let material = BundledPlacementRootMaterial {
            root_aggregate_id: &self.root_aggregate_id,
            source_cell_key: &self.source_cell_key,
            source_cell_id: &self.source_cell_id,
            destination_cell_key: &self.destination_cell_key,
            destination_cell_id: &self.destination_cell_id,
            members: &self.members,
        };
        let bytes = serde_json::to_vec(&material).map_err(|source| {
            CellDirectoryError::InvalidDirectory(format!(
                "bundled placement root material cannot be encoded: {source}"
            ))
        })?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"the-verse/bundled-placement-members/v1\0");
        hasher.update(&bytes);
        Ok(hasher.finalize().to_hex().to_string())
    }

    pub fn validate(&self) -> Result<(), CellDirectoryError> {
        validate_stable_id(&self.root_aggregate_id, "root aggregate")?;
        celestial::validate_cell_key(&self.source_cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        celestial::validate_cell_key(&self.destination_cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        if self.source_cell_key == self.destination_cell_key
            || self.source_cell_key.universe_id != self.destination_cell_key.universe_id
            || celestial::cell_id(&self.source_cell_key)
                .is_ok_and(|cell_id| cell_id != self.source_cell_id)
            || celestial::cell_id(&self.destination_cell_key)
                .is_ok_and(|cell_id| cell_id != self.destination_cell_id)
            || self.members.is_empty()
            || self.members.len() > 128
            || self.member_root != self.calculate_member_root()?
        {
            return Err(CellDirectoryError::InvalidDirectory(
                "bundled placement cells, members, or root are invalid".into(),
            ));
        }

        let mut saw_root = false;
        let mut prior_id: Option<&str> = None;
        for member in &self.members {
            validate_stable_id(&member.aggregate_id, "bundle member")?;
            if prior_id.is_some_and(|prior| prior >= member.aggregate_id.as_str())
                || member.prior_placement_generation == 0
                || member.prior_placement_generation.checked_add(1)
                    != Some(member.resulting_placement_generation)
            {
                return Err(CellDirectoryError::InvalidDirectory(
                    "bundled placement members must be unique, ordered, and advance once".into(),
                ));
            }
            if member.aggregate_id == self.root_aggregate_id {
                if member.aggregate_kind != MobileAggregateKind::Grid || saw_root {
                    return Err(CellDirectoryError::InvalidDirectory(
                        "bundled placement root must identify exactly one grid".into(),
                    ));
                }
                saw_root = true;
            } else if member.aggregate_kind != MobileAggregateKind::Player {
                return Err(CellDirectoryError::InvalidDirectory(
                    "the bounded grid bundle permits only player rider members".into(),
                ));
            }
            prior_id = Some(&member.aggregate_id);
        }
        if !saw_root {
            return Err(CellDirectoryError::InvalidDirectory(
                "bundled placement members omit the grid root".into(),
            ));
        }
        Ok(())
    }
}

/// Pure phase staging for directory-v3 integration. This is not a standalone
/// authority API: callers must supply the member root from the durable transfer
/// record and install the resulting member changes with that record in one
/// current-document commit.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn stage_bundled_placement_transition(
    placements: &BTreeMap<String, AggregatePlacementRecord>,
    plan: &BundledPlacementPlan,
    transfer_id: &str,
    durable_member_root: &str,
    transition: BundledPlacementTransition,
) -> Result<BTreeMap<String, AggregatePlacementRecord>, CellDirectoryError> {
    plan.validate()?;
    validate_stable_id(transfer_id, "transfer")?;
    validate_hash(durable_member_root, "bundled placement member root")?;
    if durable_member_root != plan.member_root {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer_id.to_owned(),
            reason: "placement plan does not match the durable member root".into(),
        });
    }
    if placements.iter().any(|(aggregate_id, placement)| {
        placement.active_transfer_id.as_deref() == Some(transfer_id)
            && !plan
                .members
                .iter()
                .any(|member| member.aggregate_id == *aggregate_id)
    }) {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer_id.to_owned(),
            reason: "transfer ID is active on a nonmember placement".into(),
        });
    }

    for member in &plan.members {
        let placement = placements
            .get(&member.aggregate_id)
            .ok_or_else(|| CellDirectoryError::UnknownAggregate(member.aggregate_id.clone()))?;
        let common_valid = placement.aggregate_id == member.aggregate_id
            && placement.aggregate_kind == member.aggregate_kind;
        let phase_valid = match transition {
            BundledPlacementTransition::Prepare => {
                placement.cell_key == plan.source_cell_key
                    && placement.cell_id == plan.source_cell_id
                    && placement.placement_generation == member.prior_placement_generation
                    && placement.state == AggregatePlacementState::Resident
                    && placement.active_transfer_id.is_none()
            }
            BundledPlacementTransition::Commit | BundledPlacementTransition::Abort => {
                placement.cell_key == plan.source_cell_key
                    && placement.cell_id == plan.source_cell_id
                    && placement.placement_generation == member.prior_placement_generation
                    && placement.state == AggregatePlacementState::Preparing
                    && placement.active_transfer_id.as_deref() == Some(transfer_id)
            }
            BundledPlacementTransition::Import => {
                placement.cell_key == plan.destination_cell_key
                    && placement.cell_id == plan.destination_cell_id
                    && placement.placement_generation == member.resulting_placement_generation
                    && placement.state == AggregatePlacementState::InTransit
                    && placement.active_transfer_id.as_deref() == Some(transfer_id)
            }
        };
        if !common_valid || !phase_valid {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: format!(
                    "bundle member {} is stale or in the wrong placement phase",
                    member.aggregate_id
                ),
            });
        }
    }

    let mut staged = placements.clone();
    for member in &plan.members {
        let placement = staged
            .get_mut(&member.aggregate_id)
            .expect("validated bundle member exists in cloned placements");
        match transition {
            BundledPlacementTransition::Prepare => {
                placement.state = AggregatePlacementState::Preparing;
                placement.active_transfer_id = Some(transfer_id.to_owned());
            }
            BundledPlacementTransition::Commit => {
                placement.cell_key.clone_from(&plan.destination_cell_key);
                placement.cell_id.clone_from(&plan.destination_cell_id);
                placement.placement_generation = member.resulting_placement_generation;
                placement.state = AggregatePlacementState::InTransit;
            }
            BundledPlacementTransition::Import | BundledPlacementTransition::Abort => {
                placement.state = AggregatePlacementState::Resident;
                placement.active_transfer_id = None;
            }
        }
    }
    Ok(staged)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferPhase {
    Prepared,
    Quarantined,
    Committed,
    Imported,
    Finalized,
    Aborting,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferAbortRole {
    Source,
    Destination,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTransferRecord {
    pub transfer_id: String,
    pub aggregate_id: String,
    pub aggregate_kind: MobileAggregateKind,
    pub source_cell_key: CellKeyV1,
    pub source_cell_id: String,
    pub destination_cell_key: CellKeyV1,
    pub destination_cell_id: String,
    pub source_assignment_generation: u64,
    pub destination_assignment_generation: u64,
    pub prior_placement_generation: u64,
    pub resulting_placement_generation: u64,
    pub package_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantine_receipt_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_prepare_proof: Option<CellTransferPrepareProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_quarantine_proof: Option<CellTransferQuarantineProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_proof: Option<CellTransferImportProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finalization_proof: Option<CellTransferFinalizationProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_abort_proof: Option<CellTransferAbortProof>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_abort_proof: Option<CellTransferAbortProof>,
    pub phase: TransferPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTransferAbortProof {
    pub(crate) transfer_id: String,
    pub(crate) package_hash: String,
    pub(crate) cell_id: String,
    pub(crate) assignment_generation: u64,
    pub(crate) role: TransferAbortRole,
    pub(crate) fencing_token: u64,
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) world_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTransferImportProof {
    pub(crate) transfer_id: String,
    pub(crate) package_hash: String,
    pub(crate) quarantine_receipt_hash: String,
    pub(crate) destination_cell_id: String,
    pub(crate) destination_assignment_generation: u64,
    pub(crate) resulting_placement_generation: u64,
    pub(crate) destination_fencing_token: u64,
    pub(crate) destination_event_sequence: u64,
    pub(crate) destination_event_hash: String,
    pub(crate) destination_world_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTransferFinalizationProof {
    pub(crate) transfer_id: String,
    pub(crate) package_hash: String,
    pub(crate) source_cell_id: String,
    pub(crate) source_assignment_generation: u64,
    pub(crate) resulting_placement_generation: u64,
    pub(crate) source_fencing_token: u64,
    pub(crate) source_event_sequence: u64,
    pub(crate) source_event_hash: String,
    pub(crate) source_world_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTransferPrepareProof {
    pub(crate) transfer_id: String,
    pub(crate) package_hash: String,
    pub(crate) source_cell_id: String,
    pub(crate) source_assignment_generation: u64,
    pub(crate) prior_placement_generation: u64,
    pub(crate) source_fencing_token: u64,
    pub(crate) source_event_sequence: u64,
    pub(crate) source_event_hash: String,
    pub(crate) source_world_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellTransferQuarantineProof {
    pub(crate) transfer_id: String,
    pub(crate) package_hash: String,
    pub(crate) quarantine_receipt_hash: String,
    pub(crate) destination_cell_id: String,
    pub(crate) destination_assignment_generation: u64,
    pub(crate) resulting_placement_generation: u64,
    pub(crate) destination_fencing_token: u64,
    pub(crate) destination_event_sequence: u64,
    pub(crate) destination_event_hash: String,
    pub(crate) destination_world_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellDirectoryDocument {
    schema_version: u32,
    universe_id: String,
    universe_manifest_hash: String,
    directory_revision: u64,
    assignments: BTreeMap<String, CellAssignmentRecord>,
    placements: BTreeMap<String, AggregatePlacementRecord>,
    transfers: BTreeMap<String, CellTransferRecord>,
}

#[derive(Debug, Error)]
pub enum CellDirectoryError {
    #[error("cell directory I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cell directory JSON is invalid at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("another universe-directory writer already owns {0}")]
    WriterAlreadyActive(PathBuf),
    #[error("cell directory invariant failed: {0}")]
    InvalidDirectory(String),
    #[error("cell directory has no assignment for {0}")]
    UnknownCell(String),
    #[error("cell assignment compare-and-swap conflict for {cell_id}: {reason}")]
    AssignmentConflict { cell_id: String, reason: String },
    #[error("cell assignment generation is exhausted for {0}")]
    AssignmentGenerationExhausted(String),
    #[error("cell transfer compare-and-swap conflict for {transfer_id}: {reason}")]
    TransferConflict { transfer_id: String, reason: String },
    #[error("cell directory has no placement for {0}")]
    UnknownAggregate(String),
    #[error("cell directory has no transfer for {0}")]
    UnknownTransfer(String),
    #[error("aggregate placement generation is exhausted for {0}")]
    PlacementGenerationExhausted(String),
    #[error("cell directory revision is exhausted")]
    DirectoryRevisionExhausted,
}

#[derive(Debug)]
pub struct LocalCellDirectory {
    root: PathBuf,
    lock_file: File,
    document: CellDirectoryDocument,
}

impl LocalCellDirectory {
    pub(crate) fn open(
        root: impl AsRef<Path>,
        universe_manifest: &UniverseManifestSnapshot,
        proof_cells: impl IntoIterator<Item = CellKeyV1>,
    ) -> Result<Self, CellDirectoryError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|source| io_error(&root, source))?;
        let lock_path = root.join(DIRECTORY_LOCK_FILE);
        let lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| io_error(&lock_path, source))?;
        FileExt::try_lock_exclusive(&lock_file).map_err(|source| {
            if source.kind() == std::io::ErrorKind::WouldBlock {
                CellDirectoryError::WriterAlreadyActive(root.clone())
            } else {
                io_error(&lock_path, source)
            }
        })?;

        let expected_assignments = canonical_assignments(universe_manifest, proof_cells)?;
        let directory_path = root.join(DIRECTORY_FILE);
        let document = if directory_path.exists() {
            let bytes =
                fs::read(&directory_path).map_err(|source| io_error(&directory_path, source))?;
            let document =
                serde_json::from_slice::<CellDirectoryDocument>(&bytes).map_err(|source| {
                    CellDirectoryError::Json {
                        path: directory_path.clone(),
                        source,
                    }
                })?;
            validate_document(&document, universe_manifest, &expected_assignments)?;
            document
        } else {
            let document = CellDirectoryDocument {
                schema_version: CELL_DIRECTORY_SCHEMA_VERSION,
                universe_id: universe_manifest.universe_id.clone(),
                universe_manifest_hash: universe_manifest.manifest_hash.clone(),
                directory_revision: 1,
                assignments: expected_assignments,
                placements: BTreeMap::new(),
                transfers: BTreeMap::new(),
            };
            write_json_atomic(&directory_path, &document)?;
            document
        };

        Ok(Self {
            root,
            lock_file,
            document,
        })
    }

    pub const fn directory_revision(&self) -> u64 {
        self.document.directory_revision
    }

    pub fn assignment(
        &self,
        cell_key: &CellKeyV1,
    ) -> Result<&CellAssignmentRecord, CellDirectoryError> {
        let cell_id = celestial::cell_id(cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        self.document
            .assignments
            .get(&cell_id)
            .ok_or(CellDirectoryError::UnknownCell(cell_id))
    }

    pub(crate) fn claim(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
        authority_fencing_token: u64,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        validate_holder(holder_id)?;
        if authority_fencing_token == 0 {
            return Err(CellDirectoryError::InvalidDirectory(
                "assigned cell authority fence must be positive".into(),
            ));
        }
        let cell_id = celestial::cell_id(cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        let mut next_document = self.document.clone();
        let assignment = next_document
            .assignments
            .get_mut(&cell_id)
            .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.clone()))?;
        if assignment.assignment_generation != expected_generation
            || assignment.state != CellAssignmentState::Sleeping
            || assignment.holder_id.is_some()
        {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "expected the current sleeping generation without a holder".into(),
            });
        }
        assignment.assignment_generation = assignment
            .assignment_generation
            .checked_add(1)
            .ok_or_else(|| {
                CellDirectoryError::AssignmentGenerationExhausted(assignment.cell_id.clone())
            })?;
        assignment.state = CellAssignmentState::Assigned;
        assignment.holder_id = Some(holder_id.to_owned());
        if authority_fencing_token <= assignment.authority_fencing_token {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "new cell authority fence must exceed its durable predecessor".into(),
            });
        }
        assignment.authority_fencing_token = authority_fencing_token;
        assignment
            .fencing_history
            .insert(assignment.assignment_generation, authority_fencing_token);
        let result = assignment.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    #[allow(dead_code)]
    pub(crate) fn release(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        validate_holder(holder_id)?;
        let cell_id = celestial::cell_id(cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        let mut next_document = self.document.clone();
        let assignment = next_document
            .assignments
            .get_mut(&cell_id)
            .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.clone()))?;
        if assignment.state == CellAssignmentState::Sleeping
            && assignment.assignment_generation == expected_generation
            && assignment.holder_id.is_none()
        {
            return Ok(assignment.clone());
        }
        if assignment.assignment_generation != expected_generation
            || assignment.state != CellAssignmentState::Assigned
            || assignment.holder_id.as_deref() != Some(holder_id)
        {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "generation, state, or holder no longer matches".into(),
            });
        }
        if self.document.transfers.values().any(|transfer| {
            !matches!(
                transfer.phase,
                TransferPhase::Finalized | TransferPhase::Aborted
            ) && (transfer.source_cell_id == cell_id || transfer.destination_cell_id == cell_id)
        }) {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "cell assignment is pinned by a nonterminal transfer".into(),
            });
        }
        assignment.state = CellAssignmentState::Sleeping;
        assignment.holder_id = None;
        let result = assignment.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn recover_assignment(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
        authority_fencing_token: u64,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        validate_holder(holder_id)?;
        let cell_id = celestial::cell_id(cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        let mut next_document = self.document.clone();
        let assignment = next_document
            .assignments
            .get_mut(&cell_id)
            .ok_or_else(|| CellDirectoryError::UnknownCell(cell_id.clone()))?;
        if assignment.assignment_generation != expected_generation
            || assignment.state != CellAssignmentState::Assigned
        {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "only the current assigned generation may be recovered".into(),
            });
        }
        assignment.assignment_generation = assignment
            .assignment_generation
            .checked_add(1)
            .ok_or_else(|| {
                CellDirectoryError::AssignmentGenerationExhausted(assignment.cell_id.clone())
            })?;
        if authority_fencing_token <= assignment.authority_fencing_token {
            return Err(CellDirectoryError::AssignmentConflict {
                cell_id,
                reason: "replacement cell authority fence must strictly advance".into(),
            });
        }
        assignment.authority_fencing_token = authority_fencing_token;
        assignment
            .fencing_history
            .insert(assignment.assignment_generation, authority_fencing_token);
        assignment.holder_id = Some(holder_id.to_owned());
        let result = assignment.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub fn placement(
        &self,
        aggregate_id: &str,
    ) -> Result<&AggregatePlacementRecord, CellDirectoryError> {
        self.document
            .placements
            .get(aggregate_id)
            .ok_or_else(|| CellDirectoryError::UnknownAggregate(aggregate_id.to_owned()))
    }

    pub fn transfer(&self, transfer_id: &str) -> Result<&CellTransferRecord, CellDirectoryError> {
        self.document
            .transfers
            .get(transfer_id)
            .ok_or_else(|| CellDirectoryError::UnknownTransfer(transfer_id.to_owned()))
    }

    pub fn transfer_records(&self) -> Vec<CellTransferRecord> {
        self.document.transfers.values().cloned().collect()
    }

    pub(crate) fn register_placement(
        &mut self,
        aggregate_id: &str,
        aggregate_kind: MobileAggregateKind,
        cell_key: &CellKeyV1,
    ) -> Result<AggregatePlacementRecord, CellDirectoryError> {
        validate_stable_id(aggregate_id, "aggregate")?;
        let assignment = self.assignment(cell_key)?.clone();
        let requested = AggregatePlacementRecord {
            aggregate_id: aggregate_id.to_owned(),
            aggregate_kind,
            cell_key: cell_key.clone(),
            cell_id: assignment.cell_id,
            placement_generation: 1,
            state: AggregatePlacementState::Resident,
            active_transfer_id: None,
        };
        if let Some(existing) = self.document.placements.get(aggregate_id) {
            if existing == &requested {
                return Ok(existing.clone());
            }
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: aggregate_id.to_owned(),
                reason: "aggregate already has a different canonical placement".into(),
            });
        }
        let mut next_document = self.document.clone();
        next_document
            .placements
            .insert(aggregate_id.to_owned(), requested.clone());
        self.commit_document(next_document)?;
        Ok(requested)
    }

    pub(crate) fn prepare_transfer(
        &mut self,
        aggregate_id: &str,
        expected_placement_generation: u64,
        transfer_id: &str,
        destination_cell_key: &CellKeyV1,
        package_hash: &str,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        validate_stable_id(aggregate_id, "aggregate")?;
        validate_stable_id(transfer_id, "transfer")?;
        validate_hash(package_hash, "transfer package")?;
        if let Some(existing) = self.document.transfers.get(transfer_id) {
            if existing.aggregate_id == aggregate_id
                && existing.destination_cell_key == *destination_cell_key
                && existing.prior_placement_generation == expected_placement_generation
                && existing.package_hash == package_hash
            {
                return Ok(existing.clone());
            }
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "transfer ID is already bound to different immutable material".into(),
            });
        }
        let placement = self.placement(aggregate_id)?.clone();
        let source_assignment = self.assignment(&placement.cell_key)?.clone();
        let destination_assignment = self.assignment(destination_cell_key)?.clone();
        if source_assignment.state != CellAssignmentState::Assigned
            || destination_assignment.state != CellAssignmentState::Assigned
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "source and destination cells must both have current assignments".into(),
            });
        }
        if placement.state != AggregatePlacementState::Resident
            || placement.active_transfer_id.is_some()
            || placement.placement_generation != expected_placement_generation
            || placement.cell_id == destination_assignment.cell_id
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "aggregate is not resident at the expected source generation".into(),
            });
        }
        let resulting_placement_generation = placement
            .placement_generation
            .checked_add(1)
            .ok_or_else(|| {
                CellDirectoryError::PlacementGenerationExhausted(aggregate_id.to_owned())
            })?;
        let requested = CellTransferRecord {
            transfer_id: transfer_id.to_owned(),
            aggregate_id: aggregate_id.to_owned(),
            aggregate_kind: placement.aggregate_kind,
            source_cell_key: placement.cell_key.clone(),
            source_cell_id: placement.cell_id.clone(),
            destination_cell_key: destination_cell_key.clone(),
            destination_cell_id: destination_assignment.cell_id,
            source_assignment_generation: source_assignment.assignment_generation,
            destination_assignment_generation: destination_assignment.assignment_generation,
            prior_placement_generation: placement.placement_generation,
            resulting_placement_generation,
            package_hash: package_hash.to_owned(),
            quarantine_receipt_hash: None,
            source_prepare_proof: None,
            destination_quarantine_proof: None,
            import_proof: None,
            finalization_proof: None,
            source_abort_proof: None,
            destination_abort_proof: None,
            phase: TransferPhase::Prepared,
        };
        let mut next_document = self.document.clone();
        let next_placement = next_document
            .placements
            .get_mut(aggregate_id)
            .expect("validated placement exists in cloned document");
        next_placement.state = AggregatePlacementState::Preparing;
        next_placement.active_transfer_id = Some(transfer_id.to_owned());
        next_document
            .transfers
            .insert(transfer_id.to_owned(), requested.clone());
        self.commit_document(next_document)?;
        Ok(requested)
    }

    pub(crate) fn record_source_prepared(
        &mut self,
        transfer_id: &str,
        proof: &CellTransferPrepareProof,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let mut proof = proof.clone();
        let current = self.transfer(transfer_id)?.clone();
        proof.source_assignment_generation = resolve_proof_generation(
            &current,
            proof.source_assignment_generation,
            proof.source_fencing_token,
            self.assignment(&current.source_cell_key)?,
        )?;
        validate_prepare_proof(&current, &proof)?;
        if let Some(existing) = &current.source_prepare_proof {
            if existing == &proof {
                return Ok(current);
            }
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "source prepare retry does not match the durable event proof".into(),
            });
        }
        if current.phase != TransferPhase::Prepared {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "source prepare proof arrived after the quarantine transition".into(),
            });
        }
        validate_historical_proof_authority(
            &current,
            proof.source_assignment_generation,
            proof.source_fencing_token,
            self.assignment(&current.source_cell_key)?,
        )?;
        let mut next_document = self.document.clone();
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.source_prepare_proof = Some(proof);
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn record_quarantine(
        &mut self,
        transfer_id: &str,
        package_hash: &str,
        receipt_hash: &str,
        proof: &CellTransferQuarantineProof,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let mut proof = proof.clone();
        validate_hash(package_hash, "transfer package")?;
        validate_hash(receipt_hash, "quarantine receipt")?;
        let current = self.transfer(transfer_id)?.clone();
        if current.package_hash != package_hash {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "quarantine package hash does not match prepare".into(),
            });
        }
        proof.destination_assignment_generation = resolve_proof_generation(
            &current,
            proof.destination_assignment_generation,
            proof.destination_fencing_token,
            self.assignment(&current.destination_cell_key)?,
        )?;
        if proof.quarantine_receipt_hash != receipt_hash {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "quarantine proof does not bind the submitted receipt hash".into(),
            });
        }
        if current.phase == TransferPhase::Quarantined
            && current.quarantine_receipt_hash.as_deref() == Some(receipt_hash)
        {
            if current.destination_quarantine_proof.as_ref() == Some(&proof) {
                return Ok(current);
            }
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "quarantine retry does not match the durable event proof".into(),
            });
        }
        if current.phase != TransferPhase::Prepared
            || current.quarantine_receipt_hash.is_some()
            || current.source_prepare_proof.is_none()
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "transfer is not awaiting its first quarantine receipt".into(),
            });
        }
        validate_quarantine_proof(&current, &proof)?;
        validate_historical_proof_authority(
            &current,
            proof.destination_assignment_generation,
            proof.destination_fencing_token,
            self.assignment(&current.destination_cell_key)?,
        )?;
        let mut next_document = self.document.clone();
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Quarantined;
        transfer.quarantine_receipt_hash = Some(receipt_hash.to_owned());
        transfer.destination_quarantine_proof = Some(proof);
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn commit_transfer(
        &mut self,
        transfer_id: &str,
        expected_prior_placement_generation: u64,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let current = self.transfer(transfer_id)?.clone();
        if matches!(
            current.phase,
            TransferPhase::Committed | TransferPhase::Imported | TransferPhase::Finalized
        ) && current.prior_placement_generation == expected_prior_placement_generation
        {
            return Ok(current);
        }
        if current.phase != TransferPhase::Quarantined
            || current.prior_placement_generation != expected_prior_placement_generation
            || current.quarantine_receipt_hash.is_none()
            || current.source_prepare_proof.is_none()
            || current.destination_quarantine_proof.is_none()
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "transfer is not quarantined at the expected placement generation".into(),
            });
        }
        let source_assignment = self.assignment(&current.source_cell_key)?;
        let destination_assignment = self.assignment(&current.destination_cell_key)?;
        if source_assignment.state != CellAssignmentState::Assigned
            || destination_assignment.state != CellAssignmentState::Assigned
            || source_assignment.assignment_generation < current.source_assignment_generation
            || destination_assignment.assignment_generation
                < current.destination_assignment_generation
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "source or destination assignment changed before commit".into(),
            });
        }
        let placement = self.placement(&current.aggregate_id)?;
        if placement.state != AggregatePlacementState::Preparing
            || placement.cell_id != current.source_cell_id
            || placement.placement_generation != current.prior_placement_generation
            || placement.active_transfer_id.as_deref() != Some(transfer_id)
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "source placement no longer matches prepared transfer".into(),
            });
        }

        let mut next_document = self.document.clone();
        let next_placement = next_document
            .placements
            .get_mut(&current.aggregate_id)
            .expect("validated placement exists in cloned document");
        next_placement.cell_key = current.destination_cell_key.clone();
        next_placement
            .cell_id
            .clone_from(&current.destination_cell_id);
        next_placement.placement_generation = current.resulting_placement_generation;
        next_placement.state = AggregatePlacementState::InTransit;
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Committed;
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn record_imported(
        &mut self,
        transfer_id: &str,
        proof: &CellTransferImportProof,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let mut proof = proof.clone();
        let current = self.transfer(transfer_id)?.clone();
        proof.destination_assignment_generation = resolve_proof_generation(
            &current,
            proof.destination_assignment_generation,
            proof.destination_fencing_token,
            self.assignment(&current.destination_cell_key)?,
        )?;
        if matches!(
            current.phase,
            TransferPhase::Imported | TransferPhase::Finalized
        ) {
            if current.import_proof.as_ref() == Some(&proof) {
                return Ok(current);
            }
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "import retry does not match the durable destination proof".into(),
            });
        }
        let placement = self.placement(&current.aggregate_id)?;
        if current.phase != TransferPhase::Committed
            || placement.state != AggregatePlacementState::InTransit
            || placement.cell_id != current.destination_cell_id
            || placement.placement_generation != current.resulting_placement_generation
            || placement.active_transfer_id.as_deref() != Some(transfer_id)
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "committed destination placement is not ready for import".into(),
            });
        }
        validate_import_proof(&current, &proof)?;
        validate_historical_proof_authority(
            &current,
            proof.destination_assignment_generation,
            proof.destination_fencing_token,
            self.assignment(&current.destination_cell_key)?,
        )?;
        let mut next_document = self.document.clone();
        let next_placement = next_document
            .placements
            .get_mut(&current.aggregate_id)
            .expect("validated placement exists in cloned document");
        next_placement.state = AggregatePlacementState::Resident;
        next_placement.active_transfer_id = None;
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Imported;
        transfer.import_proof = Some(proof);
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn finalize_transfer(
        &mut self,
        transfer_id: &str,
        proof: &CellTransferFinalizationProof,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let mut proof = proof.clone();
        let current = self.transfer(transfer_id)?.clone();
        proof.source_assignment_generation = resolve_proof_generation(
            &current,
            proof.source_assignment_generation,
            proof.source_fencing_token,
            self.assignment(&current.source_cell_key)?,
        )?;
        if current.phase == TransferPhase::Finalized {
            if current.finalization_proof.as_ref() == Some(&proof) {
                return Ok(current);
            }
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "finalization retry does not match the durable source proof".into(),
            });
        }
        if current.phase != TransferPhase::Imported {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "only an imported transfer may finalize".into(),
            });
        }
        let placement = self.placement(&current.aggregate_id)?;
        if placement.state != AggregatePlacementState::Resident
            || placement.cell_id != current.destination_cell_id
            || placement.placement_generation != current.resulting_placement_generation
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "destination residency does not match imported transfer".into(),
            });
        }
        validate_finalization_proof(&current, &proof)?;
        validate_historical_proof_authority(
            &current,
            proof.source_assignment_generation,
            proof.source_fencing_token,
            self.assignment(&current.source_cell_key)?,
        )?;
        let mut next_document = self.document.clone();
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Finalized;
        transfer.finalization_proof = Some(proof);
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn request_abort(
        &mut self,
        transfer_id: &str,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let current = self.transfer(transfer_id)?.clone();
        if matches!(
            current.phase,
            TransferPhase::Aborting | TransferPhase::Aborted
        ) {
            return Ok(current);
        }
        if !matches!(
            current.phase,
            TransferPhase::Prepared | TransferPhase::Quarantined
        ) {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "a committed transfer cannot abort to the source".into(),
            });
        }
        let placement = self.placement(&current.aggregate_id)?;
        if placement.state != AggregatePlacementState::Preparing
            || placement.cell_id != current.source_cell_id
            || placement.placement_generation != current.prior_placement_generation
            || placement.active_transfer_id.as_deref() != Some(transfer_id)
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "source placement no longer matches the abortable prepare".into(),
            });
        }
        let mut next_document = self.document.clone();
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Aborting;
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn record_abort_cleanup(
        &mut self,
        transfer_id: &str,
        proof: &CellTransferAbortProof,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let mut proof = proof.clone();
        let current = self.transfer(transfer_id)?.clone();
        if current.phase != TransferPhase::Aborting {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "abort cleanup proof requires an aborting transfer".into(),
            });
        }
        let cell_key = match proof.role {
            TransferAbortRole::Source => &current.source_cell_key,
            TransferAbortRole::Destination => &current.destination_cell_key,
        };
        proof.assignment_generation = resolve_proof_generation(
            &current,
            proof.assignment_generation,
            proof.fencing_token,
            self.assignment(cell_key)?,
        )?;
        validate_abort_proof(&current, &proof)?;
        let (existing, cell_key) = match proof.role {
            TransferAbortRole::Source => (
                current.source_abort_proof.as_ref(),
                &current.source_cell_key,
            ),
            TransferAbortRole::Destination => (
                current.destination_abort_proof.as_ref(),
                &current.destination_cell_key,
            ),
        };
        if let Some(existing) = existing {
            if existing == &proof {
                return Ok(current);
            }
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "abort cleanup retry does not match its durable cell proof".into(),
            });
        }
        validate_historical_proof_authority(
            &current,
            proof.assignment_generation,
            proof.fencing_token,
            self.assignment(cell_key)?,
        )?;
        let mut next_document = self.document.clone();
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        match proof.role {
            TransferAbortRole::Source => transfer.source_abort_proof = Some(proof),
            TransferAbortRole::Destination => {
                transfer.destination_abort_proof = Some(proof);
            }
        }
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn finalize_abort(
        &mut self,
        transfer_id: &str,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let current = self.transfer(transfer_id)?.clone();
        if current.phase == TransferPhase::Aborted {
            return Ok(current);
        }
        if current.phase != TransferPhase::Aborting
            || current.source_abort_proof.is_none()
            || current.destination_abort_proof.is_none()
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "abort cannot finish before every durable cell cleanup".into(),
            });
        }
        let placement = self.placement(&current.aggregate_id)?;
        if placement.state != AggregatePlacementState::Preparing
            || placement.cell_id != current.source_cell_id
            || placement.placement_generation != current.prior_placement_generation
            || placement.active_transfer_id.as_deref() != Some(transfer_id)
        {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "source placement no longer matches the aborting transfer".into(),
            });
        }
        let mut next_document = self.document.clone();
        let next_placement = next_document
            .placements
            .get_mut(&current.aggregate_id)
            .expect("validated placement exists in cloned document");
        next_placement.state = AggregatePlacementState::Resident;
        next_placement.active_transfer_id = None;
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Aborted;
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub(crate) fn cell_store_root(
        &self,
        cell_key: &CellKeyV1,
    ) -> Result<PathBuf, CellDirectoryError> {
        let assignment = self.assignment(cell_key)?;
        Ok(self.root.join("cells").join(&assignment.cell_id))
    }

    fn commit_document(
        &mut self,
        mut next_document: CellDirectoryDocument,
    ) -> Result<(), CellDirectoryError> {
        next_document.directory_revision = self
            .document
            .directory_revision
            .checked_add(1)
            .ok_or(CellDirectoryError::DirectoryRevisionExhausted)?;
        write_json_atomic(&self.root.join(DIRECTORY_FILE), &next_document)?;
        self.document = next_document;
        Ok(())
    }
}

impl Drop for LocalCellDirectory {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock_file);
    }
}

pub fn proof_cell_keys() -> Result<[CellKeyV1; 2], CellDirectoryError> {
    let origin = celestial::cell_origin_key();
    let east = celestial::neighbor_cell_key(&origin, [1, 0, 0])
        .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
    Ok([origin, east])
}

fn canonical_assignments(
    universe_manifest: &UniverseManifestSnapshot,
    proof_cells: impl IntoIterator<Item = CellKeyV1>,
) -> Result<BTreeMap<String, CellAssignmentRecord>, CellDirectoryError> {
    let mut assignments = BTreeMap::new();
    for cell_key in proof_cells {
        celestial::validate_cell_key(&cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        if cell_key.universe_id != universe_manifest.universe_id {
            return Err(CellDirectoryError::InvalidDirectory(
                "proof cell belongs to a different universe".into(),
            ));
        }
        let cell_id = celestial::cell_id(&cell_key)
            .map_err(|source| CellDirectoryError::InvalidDirectory(source.to_string()))?;
        let record = CellAssignmentRecord {
            cell_key,
            cell_id: cell_id.clone(),
            assignment_generation: 0,
            authority_fencing_token: 0,
            fencing_history: BTreeMap::new(),
            state: CellAssignmentState::Sleeping,
            holder_id: None,
        };
        if assignments.insert(cell_id, record).is_some() {
            return Err(CellDirectoryError::InvalidDirectory(
                "proof cells contain a duplicate canonical identity".into(),
            ));
        }
    }
    if assignments.len() != 2 {
        return Err(CellDirectoryError::InvalidDirectory(
            "P1.7 proof requires exactly two canonical cells".into(),
        ));
    }
    Ok(assignments)
}

fn validate_document(
    document: &CellDirectoryDocument,
    universe_manifest: &UniverseManifestSnapshot,
    expected_assignments: &BTreeMap<String, CellAssignmentRecord>,
) -> Result<(), CellDirectoryError> {
    if document.schema_version != CELL_DIRECTORY_SCHEMA_VERSION
        || document.universe_id != universe_manifest.universe_id
        || document.universe_manifest_hash != universe_manifest.manifest_hash
        || document.directory_revision == 0
        || document.assignments.len() != expected_assignments.len()
    {
        return Err(CellDirectoryError::InvalidDirectory(
            "schema, universe, manifest, revision, or proof-cell count mismatch".into(),
        ));
    }
    for (cell_id, expected) in expected_assignments {
        let stored = document.assignments.get(cell_id).ok_or_else(|| {
            CellDirectoryError::InvalidDirectory(format!(
                "directory is missing proof cell {cell_id}"
            ))
        })?;
        if stored.cell_id != *cell_id || stored.cell_key != expected.cell_key {
            return Err(CellDirectoryError::InvalidDirectory(format!(
                "directory cell identity mismatch for {cell_id}"
            )));
        }
        match stored.state {
            CellAssignmentState::Sleeping => {
                if stored.holder_id.is_some() {
                    return Err(CellDirectoryError::InvalidDirectory(format!(
                        "sleeping cell {cell_id} retains a holder"
                    )));
                }
            }
            CellAssignmentState::Assigned => {
                if stored.assignment_generation == 0
                    || stored.authority_fencing_token == 0
                    || stored.holder_id.as_deref().is_none_or(str::is_empty)
                {
                    return Err(CellDirectoryError::InvalidDirectory(format!(
                        "assigned cell {cell_id} lacks a generation or holder"
                    )));
                }
            }
            CellAssignmentState::Claiming | CellAssignmentState::Releasing => {
                return Err(CellDirectoryError::InvalidDirectory(format!(
                    "cell {cell_id} retained an incomplete assignment transition"
                )));
            }
        }
        if stored.fencing_history.len()
            != usize::try_from(stored.assignment_generation).unwrap_or(usize::MAX)
            || stored
                .fencing_history
                .iter()
                .enumerate()
                .any(|(index, (generation, fence))| {
                    *generation != u64::try_from(index + 1).unwrap_or(u64::MAX)
                        || *fence == 0
                        || (index > 0
                            && stored
                                .fencing_history
                                .get(&(generation - 1))
                                .is_some_and(|previous| previous >= fence))
                })
            || stored
                .fencing_history
                .get(&stored.assignment_generation)
                .copied()
                .unwrap_or(0)
                != stored.authority_fencing_token
        {
            return Err(CellDirectoryError::InvalidDirectory(format!(
                "cell {cell_id} assignment-to-fence history is invalid"
            )));
        }
    }
    for (aggregate_id, placement) in &document.placements {
        validate_stable_id(aggregate_id, "aggregate")?;
        if placement.aggregate_id != *aggregate_id || placement.placement_generation == 0 {
            return Err(CellDirectoryError::InvalidDirectory(format!(
                "aggregate placement identity or generation is invalid for {aggregate_id}"
            )));
        }
        let assignment = document
            .assignments
            .get(&placement.cell_id)
            .ok_or_else(|| {
                CellDirectoryError::InvalidDirectory(format!(
                    "aggregate {aggregate_id} references an unknown cell"
                ))
            })?;
        if assignment.cell_key != placement.cell_key {
            return Err(CellDirectoryError::InvalidDirectory(format!(
                "aggregate {aggregate_id} cell key and ID disagree"
            )));
        }
        match placement.state {
            AggregatePlacementState::Resident if placement.active_transfer_id.is_none() => {}
            AggregatePlacementState::Preparing | AggregatePlacementState::InTransit
                if placement.active_transfer_id.is_some() => {}
            _ => {
                return Err(CellDirectoryError::InvalidDirectory(format!(
                    "aggregate {aggregate_id} placement state and transfer binding disagree"
                )));
            }
        }
    }
    for (transfer_id, transfer) in &document.transfers {
        validate_stable_id(transfer_id, "transfer")?;
        validate_stable_id(&transfer.aggregate_id, "aggregate")?;
        validate_hash(&transfer.package_hash, "transfer package")?;
        if transfer.transfer_id != *transfer_id
            || transfer.source_cell_id == transfer.destination_cell_id
            || transfer.source_assignment_generation == 0
            || transfer.destination_assignment_generation == 0
            || transfer.prior_placement_generation == 0
            || transfer.prior_placement_generation.checked_add(1)
                != Some(transfer.resulting_placement_generation)
        {
            return Err(CellDirectoryError::InvalidDirectory(format!(
                "transfer {transfer_id} immutable identity or generation is invalid"
            )));
        }
        let source = document
            .assignments
            .get(&transfer.source_cell_id)
            .ok_or_else(|| {
                CellDirectoryError::InvalidDirectory(format!(
                    "transfer {transfer_id} source cell is unknown"
                ))
            })?;
        let destination = document
            .assignments
            .get(&transfer.destination_cell_id)
            .ok_or_else(|| {
                CellDirectoryError::InvalidDirectory(format!(
                    "transfer {transfer_id} destination cell is unknown"
                ))
            })?;
        let placement = document
            .placements
            .get(&transfer.aggregate_id)
            .ok_or_else(|| {
                CellDirectoryError::InvalidDirectory(format!(
                    "transfer {transfer_id} aggregate placement is unknown"
                ))
            })?;
        if source.cell_key != transfer.source_cell_key
            || destination.cell_key != transfer.destination_cell_key
            || placement.aggregate_kind != transfer.aggregate_kind
        {
            return Err(CellDirectoryError::InvalidDirectory(format!(
                "transfer {transfer_id} cell or aggregate identity is inconsistent"
            )));
        }
        if let Some(receipt_hash) = &transfer.quarantine_receipt_hash {
            validate_hash(receipt_hash, "quarantine receipt")?;
        }
        if let Some(proof) = &transfer.source_prepare_proof {
            validate_prepare_proof(transfer, proof)?;
            validate_historical_proof_authority(
                transfer,
                proof.source_assignment_generation,
                proof.source_fencing_token,
                source,
            )?;
        }
        if let Some(proof) = &transfer.destination_quarantine_proof {
            validate_quarantine_proof(transfer, proof)?;
            validate_historical_proof_authority(
                transfer,
                proof.destination_assignment_generation,
                proof.destination_fencing_token,
                destination,
            )?;
            if transfer.quarantine_receipt_hash.as_deref()
                != Some(proof.quarantine_receipt_hash.as_str())
            {
                return Err(CellDirectoryError::InvalidDirectory(format!(
                    "transfer {transfer_id} quarantine proof and receipt disagree"
                )));
            }
        }
        if let Some(proof) = &transfer.import_proof {
            validate_import_proof(transfer, proof)?;
            validate_historical_proof_authority(
                transfer,
                proof.destination_assignment_generation,
                proof.destination_fencing_token,
                destination,
            )?;
        }
        if let Some(proof) = &transfer.finalization_proof {
            validate_finalization_proof(transfer, proof)?;
            validate_historical_proof_authority(
                transfer,
                proof.source_assignment_generation,
                proof.source_fencing_token,
                source,
            )?;
        }
        if let Some(proof) = &transfer.source_abort_proof {
            validate_abort_proof(transfer, proof)?;
            validate_historical_proof_authority(
                transfer,
                proof.assignment_generation,
                proof.fencing_token,
                source,
            )?;
            if proof.role != TransferAbortRole::Source {
                return Err(CellDirectoryError::InvalidDirectory(format!(
                    "transfer {transfer_id} source abort proof has the wrong role"
                )));
            }
        }
        if let Some(proof) = &transfer.destination_abort_proof {
            validate_abort_proof(transfer, proof)?;
            validate_historical_proof_authority(
                transfer,
                proof.assignment_generation,
                proof.fencing_token,
                destination,
            )?;
            if proof.role != TransferAbortRole::Destination {
                return Err(CellDirectoryError::InvalidDirectory(format!(
                    "transfer {transfer_id} destination abort proof has the wrong role"
                )));
            }
        }
        let placement_matches = match transfer.phase {
            TransferPhase::Prepared | TransferPhase::Quarantined | TransferPhase::Aborting => {
                placement.state == AggregatePlacementState::Preparing
                    && placement.cell_id == transfer.source_cell_id
                    && placement.placement_generation == transfer.prior_placement_generation
                    && placement.active_transfer_id.as_deref() == Some(transfer_id)
            }
            TransferPhase::Committed => {
                placement.state == AggregatePlacementState::InTransit
                    && placement.cell_id == transfer.destination_cell_id
                    && placement.placement_generation == transfer.resulting_placement_generation
                    && placement.active_transfer_id.as_deref() == Some(transfer_id)
            }
            TransferPhase::Imported | TransferPhase::Finalized => {
                placement.placement_generation > transfer.resulting_placement_generation
                    || (placement.placement_generation == transfer.resulting_placement_generation
                        && placement.cell_id == transfer.destination_cell_id)
            }
            TransferPhase::Aborted => {
                placement.placement_generation > transfer.prior_placement_generation
                    || (placement.placement_generation == transfer.prior_placement_generation
                        && placement.cell_id == transfer.source_cell_id)
            }
        };
        if !placement_matches
            || (transfer.phase == TransferPhase::Prepared
                && transfer.quarantine_receipt_hash.is_some())
            || (matches!(
                transfer.phase,
                TransferPhase::Quarantined
                    | TransferPhase::Committed
                    | TransferPhase::Imported
                    | TransferPhase::Finalized
            ) && transfer.quarantine_receipt_hash.is_none())
            || (transfer.source_prepare_proof.is_none()
                && !matches!(
                    transfer.phase,
                    TransferPhase::Prepared | TransferPhase::Aborted
                ))
            || (transfer.destination_quarantine_proof.is_some()
                != transfer.quarantine_receipt_hash.is_some())
            || (matches!(
                transfer.phase,
                TransferPhase::Quarantined
                    | TransferPhase::Committed
                    | TransferPhase::Imported
                    | TransferPhase::Finalized
            ) && transfer.destination_quarantine_proof.is_none())
            || (matches!(
                transfer.phase,
                TransferPhase::Prepared
                    | TransferPhase::Quarantined
                    | TransferPhase::Committed
                    | TransferPhase::Aborting
                    | TransferPhase::Aborted
            ) && (transfer.import_proof.is_some() || transfer.finalization_proof.is_some()))
            || (transfer.phase == TransferPhase::Imported
                && (transfer.import_proof.is_none() || transfer.finalization_proof.is_some()))
            || (transfer.phase == TransferPhase::Finalized
                && (transfer.import_proof.is_none() || transfer.finalization_proof.is_none()))
            || (!matches!(
                transfer.phase,
                TransferPhase::Aborting | TransferPhase::Aborted
            ) && (transfer.source_abort_proof.is_some()
                || transfer.destination_abort_proof.is_some()))
            || (transfer.phase == TransferPhase::Aborted
                && (transfer.source_abort_proof.is_none()
                    || transfer.destination_abort_proof.is_none()))
        {
            return Err(CellDirectoryError::InvalidDirectory(format!(
                "transfer {transfer_id} phase and placement state disagree"
            )));
        }
    }
    Ok(())
}

fn validate_stable_id(value: &str, kind: &str) -> Result<(), CellDirectoryError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CellDirectoryError::InvalidDirectory(format!(
            "{kind} ID is not bounded canonical text"
        )));
    }
    Ok(())
}

fn validate_hash(value: &str, kind: &str) -> Result<(), CellDirectoryError> {
    if !crate::model::valid_blake3_hex(value) {
        return Err(CellDirectoryError::InvalidDirectory(format!(
            "{kind} hash is not canonical BLAKE3 text"
        )));
    }
    Ok(())
}

fn validate_import_proof(
    transfer: &CellTransferRecord,
    proof: &CellTransferImportProof,
) -> Result<(), CellDirectoryError> {
    validate_hash(&proof.destination_event_hash, "destination import event")?;
    validate_hash(&proof.destination_world_hash, "destination import world")?;
    if proof.transfer_id != transfer.transfer_id
        || proof.package_hash != transfer.package_hash
        || transfer.quarantine_receipt_hash.as_deref()
            != Some(proof.quarantine_receipt_hash.as_str())
        || proof.destination_cell_id != transfer.destination_cell_id
        || proof.destination_assignment_generation < transfer.destination_assignment_generation
        || proof.resulting_placement_generation != transfer.resulting_placement_generation
        || proof.destination_fencing_token == 0
        || proof.destination_event_sequence == 0
    {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer.transfer_id.clone(),
            reason: "destination import proof does not bind the committed transfer".into(),
        });
    }
    Ok(())
}

fn validate_finalization_proof(
    transfer: &CellTransferRecord,
    proof: &CellTransferFinalizationProof,
) -> Result<(), CellDirectoryError> {
    validate_hash(&proof.source_event_hash, "source finalization event")?;
    validate_hash(&proof.source_world_hash, "source finalization world")?;
    if proof.transfer_id != transfer.transfer_id
        || proof.package_hash != transfer.package_hash
        || proof.source_cell_id != transfer.source_cell_id
        || proof.source_assignment_generation < transfer.source_assignment_generation
        || proof.resulting_placement_generation != transfer.resulting_placement_generation
        || proof.source_fencing_token == 0
        || proof.source_event_sequence == 0
    {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer.transfer_id.clone(),
            reason: "source finalization proof does not bind the committed transfer".into(),
        });
    }
    Ok(())
}

fn validate_prepare_proof(
    transfer: &CellTransferRecord,
    proof: &CellTransferPrepareProof,
) -> Result<(), CellDirectoryError> {
    validate_hash(&proof.source_event_hash, "source prepare event")?;
    validate_hash(&proof.source_world_hash, "source prepare world")?;
    if proof.transfer_id != transfer.transfer_id
        || proof.package_hash != transfer.package_hash
        || proof.source_cell_id != transfer.source_cell_id
        || proof.source_assignment_generation < transfer.source_assignment_generation
        || proof.prior_placement_generation != transfer.prior_placement_generation
        || proof.source_fencing_token == 0
        || proof.source_event_sequence == 0
    {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer.transfer_id.clone(),
            reason: "source prepare proof does not bind the prepared transfer".into(),
        });
    }
    Ok(())
}

fn validate_quarantine_proof(
    transfer: &CellTransferRecord,
    proof: &CellTransferQuarantineProof,
) -> Result<(), CellDirectoryError> {
    validate_hash(
        &proof.destination_event_hash,
        "destination quarantine event",
    )?;
    validate_hash(
        &proof.destination_world_hash,
        "destination quarantine world",
    )?;
    if proof.transfer_id != transfer.transfer_id
        || proof.package_hash != transfer.package_hash
        || transfer
            .quarantine_receipt_hash
            .as_deref()
            .is_some_and(|receipt| receipt != proof.quarantine_receipt_hash)
        || proof.destination_cell_id != transfer.destination_cell_id
        || proof.destination_assignment_generation < transfer.destination_assignment_generation
        || proof.resulting_placement_generation != transfer.resulting_placement_generation
        || proof.destination_fencing_token == 0
        || proof.destination_event_sequence == 0
    {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer.transfer_id.clone(),
            reason: "destination quarantine proof does not bind the prepared transfer".into(),
        });
    }
    Ok(())
}

fn resolve_proof_generation(
    transfer: &CellTransferRecord,
    claimed_generation: u64,
    fencing_token: u64,
    assignment: &CellAssignmentRecord,
) -> Result<u64, CellDirectoryError> {
    let generations = assignment
        .fencing_history
        .iter()
        .filter_map(|(generation, fence)| (*fence == fencing_token).then_some(*generation))
        .collect::<Vec<_>>();
    if assignment.state != CellAssignmentState::Assigned || generations.len() != 1 {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer.transfer_id.clone(),
            reason: "cell event fence does not resolve to one durable assignment generation".into(),
        });
    }
    let generation = generations[0];
    if claimed_generation != 0 && claimed_generation != generation {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer.transfer_id.clone(),
            reason: "cell event proof claims the wrong historical assignment generation".into(),
        });
    }
    Ok(generation)
}

fn validate_historical_proof_authority(
    transfer: &CellTransferRecord,
    assignment_generation: u64,
    fencing_token: u64,
    assignment: &CellAssignmentRecord,
) -> Result<(), CellDirectoryError> {
    if assignment
        .fencing_history
        .get(&assignment_generation)
        .copied()
        != Some(fencing_token)
    {
        return Err(CellDirectoryError::InvalidDirectory(format!(
            "transfer {} proof generation is not bound to its historical cell fence",
            transfer.transfer_id
        )));
    }
    Ok(())
}

fn validate_abort_proof(
    transfer: &CellTransferRecord,
    proof: &CellTransferAbortProof,
) -> Result<(), CellDirectoryError> {
    validate_hash(&proof.event_hash, "abort cleanup event")?;
    validate_hash(&proof.world_hash, "abort cleanup world")?;
    let (expected_cell_id, minimum_generation) = match proof.role {
        TransferAbortRole::Source => (
            transfer.source_cell_id.as_str(),
            transfer.source_assignment_generation,
        ),
        TransferAbortRole::Destination => (
            transfer.destination_cell_id.as_str(),
            transfer.destination_assignment_generation,
        ),
    };
    if proof.transfer_id != transfer.transfer_id
        || proof.package_hash != transfer.package_hash
        || proof.cell_id != expected_cell_id
        || proof.assignment_generation < minimum_generation
        || proof.fencing_token == 0
        || proof.event_sequence == 0
    {
        return Err(CellDirectoryError::TransferConflict {
            transfer_id: transfer.transfer_id.clone(),
            reason: "abort cleanup proof does not bind the transfer and cell role".into(),
        });
    }
    Ok(())
}

fn validate_holder(holder_id: &str) -> Result<(), CellDirectoryError> {
    validate_stable_id(holder_id, "assignment holder")
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), CellDirectoryError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|source| CellDirectoryError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cell-directory"),
        std::process::id(),
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|source| io_error(&temp_path, source))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error(&temp_path, source))?;
        fs::rename(&temp_path, path).map_err(|source| io_error(path, source))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error(parent, source))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn io_error(path: impl AsRef<Path>, source: std::io::Error) -> CellDirectoryError {
    CellDirectoryError::Io {
        path: path.as_ref().to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::tempdir;

    use super::*;
    use crate::{
        EVENT_SCHEMA_VERSION, LifecycleMode, Store, WORLD_SCHEMA_VERSION, universe_manifest,
    };

    fn manifest(seed: u64) -> UniverseManifestSnapshot {
        universe_manifest(seed, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
            .expect("test manifest builds")
    }

    fn bundled_placement(
        aggregate_id: &str,
        aggregate_kind: MobileAggregateKind,
        cell_key: &CellKeyV1,
    ) -> AggregatePlacementRecord {
        AggregatePlacementRecord {
            aggregate_id: aggregate_id.into(),
            aggregate_kind,
            cell_key: cell_key.clone(),
            cell_id: celestial::cell_id(cell_key).expect("test cell ID derives"),
            placement_generation: 1,
            state: AggregatePlacementState::Resident,
            active_transfer_id: None,
        }
    }

    fn bundled_plan() -> BundledPlacementPlan {
        let [source, destination] = proof_cell_keys().expect("proof cells derive");
        BundledPlacementPlan::new(
            "grid-transfer-proof",
            source,
            destination,
            vec![
                BundledPlacementMember {
                    aggregate_id: "grid-transfer-proof".into(),
                    aggregate_kind: MobileAggregateKind::Grid,
                    prior_placement_generation: 1,
                    resulting_placement_generation: 2,
                },
                BundledPlacementMember {
                    aggregate_id: "player-transfer-owner".into(),
                    aggregate_kind: MobileAggregateKind::Player,
                    prior_placement_generation: 1,
                    resulting_placement_generation: 2,
                },
                BundledPlacementMember {
                    aggregate_id: "player-transfer-rider".into(),
                    aggregate_kind: MobileAggregateKind::Player,
                    prior_placement_generation: 1,
                    resulting_placement_generation: 2,
                },
            ],
        )
        .expect("bundled placement plan is canonical")
    }

    #[test]
    fn bundled_placement_member_root_has_a_stable_golden_vector() {
        assert_eq!(
            bundled_plan().member_root,
            "2f5eb27786bc9230e48f5b1b946f985270a14066132a7ebfda0f6a0aeb7f3eb9"
        );
    }

    #[test]
    fn bundled_grid_and_rider_placements_advance_atomically() {
        let plan = bundled_plan();
        let unrelated_id = "player-unrelated";
        let mut placements = BTreeMap::from([
            (
                plan.root_aggregate_id.clone(),
                bundled_placement(
                    &plan.root_aggregate_id,
                    MobileAggregateKind::Grid,
                    &plan.source_cell_key,
                ),
            ),
            (
                plan.members[1].aggregate_id.clone(),
                bundled_placement(
                    &plan.members[1].aggregate_id,
                    MobileAggregateKind::Player,
                    &plan.source_cell_key,
                ),
            ),
            (
                plan.members[2].aggregate_id.clone(),
                bundled_placement(
                    &plan.members[2].aggregate_id,
                    MobileAggregateKind::Player,
                    &plan.source_cell_key,
                ),
            ),
            (
                unrelated_id.into(),
                bundled_placement(
                    unrelated_id,
                    MobileAggregateKind::Player,
                    &plan.source_cell_key,
                ),
            ),
        ]);
        let unrelated = placements[unrelated_id].clone();

        for transition in [
            BundledPlacementTransition::Prepare,
            BundledPlacementTransition::Commit,
            BundledPlacementTransition::Import,
        ] {
            let advanced = stage_bundled_placement_transition(
                &placements,
                &plan,
                "grid-bundle-transfer-1",
                &plan.member_root,
                transition,
            )
            .expect("whole bundle advances");
            placements = advanced;
        }

        for member in &plan.members {
            let placement = &placements[&member.aggregate_id];
            assert_eq!(placement.cell_key, plan.destination_cell_key);
            assert_eq!(placement.cell_id, plan.destination_cell_id);
            assert_eq!(
                placement.placement_generation,
                member.resulting_placement_generation
            );
            assert_eq!(placement.state, AggregatePlacementState::Resident);
            assert_eq!(placement.active_transfer_id, None);
        }
        assert_eq!(placements[unrelated_id], unrelated);
    }

    #[test]
    fn bundled_placement_conflict_changes_no_member() {
        let plan = bundled_plan();
        let mut placements = plan
            .members
            .iter()
            .map(|member| {
                (
                    member.aggregate_id.clone(),
                    bundled_placement(
                        &member.aggregate_id,
                        member.aggregate_kind,
                        &plan.source_cell_key,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        placements
            .get_mut("player-transfer-rider")
            .expect("rider placement exists")
            .placement_generation = 2;
        let prior = placements.clone();

        assert!(matches!(
            stage_bundled_placement_transition(
                &placements,
                &plan,
                "grid-bundle-transfer-2",
                &plan.member_root,
                BundledPlacementTransition::Prepare,
            ),
            Err(CellDirectoryError::TransferConflict { .. })
        ));
        assert_eq!(placements, prior);
    }

    #[test]
    fn bundled_placement_rejects_a_partially_advanced_member_set() {
        let plan = bundled_plan();
        let placements = plan
            .members
            .iter()
            .map(|member| {
                (
                    member.aggregate_id.clone(),
                    bundled_placement(
                        &member.aggregate_id,
                        member.aggregate_kind,
                        &plan.source_cell_key,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut partial = placements.clone();
        let grid = partial
            .get_mut(&plan.root_aggregate_id)
            .expect("grid placement exists");
        grid.state = AggregatePlacementState::Preparing;
        grid.active_transfer_id = Some("grid-bundle-transfer-partial".into());

        assert!(matches!(
            stage_bundled_placement_transition(
                &partial,
                &plan,
                "grid-bundle-transfer-partial",
                &plan.member_root,
                BundledPlacementTransition::Prepare,
            ),
            Err(CellDirectoryError::TransferConflict { .. })
        ));
        assert_eq!(
            placements[&plan.root_aggregate_id].state,
            AggregatePlacementState::Resident
        );

        let mut aliased = placements;
        let mut unrelated = bundled_placement(
            "player-unrelated",
            MobileAggregateKind::Player,
            &plan.source_cell_key,
        );
        unrelated.active_transfer_id = Some("grid-bundle-transfer-partial".into());
        unrelated.state = AggregatePlacementState::Preparing;
        aliased.insert(unrelated.aggregate_id.clone(), unrelated);
        assert!(
            stage_bundled_placement_transition(
                &aliased,
                &plan,
                "grid-bundle-transfer-partial",
                &plan.member_root,
                BundledPlacementTransition::Prepare,
            )
            .is_err()
        );
    }

    #[test]
    fn bundled_placement_abort_restores_every_source_member() {
        let plan = bundled_plan();
        let placements = plan
            .members
            .iter()
            .map(|member| {
                (
                    member.aggregate_id.clone(),
                    bundled_placement(
                        &member.aggregate_id,
                        member.aggregate_kind,
                        &plan.source_cell_key,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let prepared = stage_bundled_placement_transition(
            &placements,
            &plan,
            "grid-bundle-transfer-3",
            &plan.member_root,
            BundledPlacementTransition::Prepare,
        )
        .expect("bundle prepares");
        let aborted = stage_bundled_placement_transition(
            &prepared,
            &plan,
            "grid-bundle-transfer-3",
            &plan.member_root,
            BundledPlacementTransition::Abort,
        )
        .expect("bundle aborts");
        assert_eq!(aborted, placements);
    }

    #[test]
    fn bundled_placement_rejects_wrong_transfer_id_and_plan_substitution() {
        let plan = bundled_plan();
        let placements = plan
            .members
            .iter()
            .map(|member| {
                (
                    member.aggregate_id.clone(),
                    bundled_placement(
                        &member.aggregate_id,
                        member.aggregate_kind,
                        &plan.source_cell_key,
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert!(
            stage_bundled_placement_transition(
                &placements,
                &plan,
                "grid-bundle-never-started",
                &plan.member_root,
                BundledPlacementTransition::Abort,
            )
            .is_err()
        );

        let prepared = stage_bundled_placement_transition(
            &placements,
            &plan,
            "grid-bundle-transfer-bound",
            &plan.member_root,
            BundledPlacementTransition::Prepare,
        )
        .expect("bundle prepares");
        assert!(
            stage_bundled_placement_transition(
                &prepared,
                &plan,
                "grid-bundle-wrong-id",
                &plan.member_root,
                BundledPlacementTransition::Commit,
            )
            .is_err()
        );

        let alternate_destination = celestial::neighbor_cell_key(&plan.source_cell_key, [-1, 0, 0])
            .expect("alternate neighbor derives");
        let substituted = BundledPlacementPlan::new(
            plan.root_aggregate_id.clone(),
            plan.source_cell_key.clone(),
            alternate_destination,
            plan.members.clone(),
        )
        .expect("alternate plan is syntactically canonical");
        assert_ne!(substituted.member_root, plan.member_root);
        assert!(
            stage_bundled_placement_transition(
                &prepared,
                &substituted,
                "grid-bundle-transfer-bound",
                &plan.member_root,
                BundledPlacementTransition::Commit,
            )
            .is_err()
        );

        let committed = stage_bundled_placement_transition(
            &prepared,
            &plan,
            "grid-bundle-transfer-bound",
            &plan.member_root,
            BundledPlacementTransition::Commit,
        )
        .expect("bundle commits");
        assert!(
            stage_bundled_placement_transition(
                &committed,
                &plan,
                "grid-bundle-wrong-id",
                &plan.member_root,
                BundledPlacementTransition::Import,
            )
            .is_err()
        );
    }

    #[test]
    fn bundled_placement_plan_rejects_missing_root_and_alias_order() {
        let [source, destination] = proof_cell_keys().expect("proof cells derive");
        let rider = BundledPlacementMember {
            aggregate_id: "player-transfer-rider".into(),
            aggregate_kind: MobileAggregateKind::Player,
            prior_placement_generation: 1,
            resulting_placement_generation: 2,
        };
        assert!(
            BundledPlacementPlan::new(
                "grid-transfer-proof",
                source.clone(),
                destination.clone(),
                vec![rider.clone()],
            )
            .is_err()
        );
        let grid = BundledPlacementMember {
            aggregate_id: "grid-transfer-proof".into(),
            aggregate_kind: MobileAggregateKind::Grid,
            prior_placement_generation: 1,
            resulting_placement_generation: 2,
        };
        assert!(
            BundledPlacementPlan::new(
                "grid-transfer-proof",
                source.clone(),
                destination.clone(),
                vec![rider, grid.clone()],
            )
            .is_err()
        );

        let mut other_universe = destination.clone();
        other_universe.universe_id = "other-universe".into();
        assert!(
            BundledPlacementPlan::new(
                "grid-transfer-proof",
                source.clone(),
                other_universe,
                vec![grid.clone()],
            )
            .is_err()
        );
        let mut tampered =
            BundledPlacementPlan::new("grid-transfer-proof", source, destination, vec![grid])
                .expect("root-only grid plan is valid");
        tampered.member_root = "0".repeat(64);
        assert!(tampered.validate().is_err());
    }

    fn import_proof(transfer: &CellTransferRecord, receipt_hash: &str) -> CellTransferImportProof {
        CellTransferImportProof {
            transfer_id: transfer.transfer_id.clone(),
            package_hash: transfer.package_hash.clone(),
            quarantine_receipt_hash: receipt_hash.to_owned(),
            destination_cell_id: transfer.destination_cell_id.clone(),
            destination_assignment_generation: transfer.destination_assignment_generation,
            resulting_placement_generation: transfer.resulting_placement_generation,
            destination_fencing_token: 9,
            destination_event_sequence: 41,
            destination_event_hash: blake3::hash(b"destination-import-event")
                .to_hex()
                .to_string(),
            destination_world_hash: blake3::hash(b"destination-import-world")
                .to_hex()
                .to_string(),
        }
    }

    fn finalization_proof(transfer: &CellTransferRecord) -> CellTransferFinalizationProof {
        CellTransferFinalizationProof {
            transfer_id: transfer.transfer_id.clone(),
            package_hash: transfer.package_hash.clone(),
            source_cell_id: transfer.source_cell_id.clone(),
            source_assignment_generation: transfer.source_assignment_generation,
            resulting_placement_generation: transfer.resulting_placement_generation,
            source_fencing_token: 5,
            source_event_sequence: 43,
            source_event_hash: blake3::hash(b"source-finalization-event")
                .to_hex()
                .to_string(),
            source_world_hash: blake3::hash(b"source-finalization-world")
                .to_hex()
                .to_string(),
        }
    }

    fn prepare_proof(transfer: &CellTransferRecord) -> CellTransferPrepareProof {
        CellTransferPrepareProof {
            transfer_id: transfer.transfer_id.clone(),
            package_hash: transfer.package_hash.clone(),
            source_cell_id: transfer.source_cell_id.clone(),
            source_assignment_generation: transfer.source_assignment_generation,
            prior_placement_generation: transfer.prior_placement_generation,
            source_fencing_token: 5,
            source_event_sequence: 37,
            source_event_hash: blake3::hash(b"source-prepare-event").to_hex().to_string(),
            source_world_hash: blake3::hash(b"source-prepare-world").to_hex().to_string(),
        }
    }

    fn quarantine_proof(
        transfer: &CellTransferRecord,
        receipt_hash: &str,
    ) -> CellTransferQuarantineProof {
        CellTransferQuarantineProof {
            transfer_id: transfer.transfer_id.clone(),
            package_hash: transfer.package_hash.clone(),
            quarantine_receipt_hash: receipt_hash.to_owned(),
            destination_cell_id: transfer.destination_cell_id.clone(),
            destination_assignment_generation: transfer.destination_assignment_generation,
            resulting_placement_generation: transfer.resulting_placement_generation,
            destination_fencing_token: 9,
            destination_event_sequence: 39,
            destination_event_hash: blake3::hash(b"destination-quarantine-event")
                .to_hex()
                .to_string(),
            destination_world_hash: blake3::hash(b"destination-quarantine-world")
                .to_hex()
                .to_string(),
        }
    }

    #[test]
    fn two_cell_assignments_persist_distinct_roots_and_generations() {
        let directory_root = tempdir().expect("temporary directory");
        let manifest = manifest(701);
        let [origin, east] = proof_cell_keys().expect("proof keys build");
        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("directory opens");
        assert_eq!(directory.directory_revision(), 1);
        assert_ne!(
            directory.cell_store_root(&origin).expect("origin root"),
            directory.cell_store_root(&east).expect("east root")
        );

        let claimed = directory
            .claim(&origin, 0, "worker-origin-a", 1)
            .expect("origin claim commits");
        assert_eq!(claimed.assignment_generation, 1);
        assert_eq!(claimed.state, CellAssignmentState::Assigned);
        assert_eq!(claimed.holder_id.as_deref(), Some("worker-origin-a"));
        assert!(directory.claim(&origin, 0, "worker-origin-b", 2).is_err());

        let released = directory
            .release(&origin, 1, "worker-origin-a")
            .expect("origin release commits");
        assert_eq!(released.state, CellAssignmentState::Sleeping);
        assert_eq!(
            directory
                .release(&origin, 1, "worker-origin-a")
                .expect("release retry reconciles"),
            released
        );
        drop(directory);

        let mut reopened =
            LocalCellDirectory::open(directory_root.path(), &manifest, [origin.clone(), east])
                .expect("directory reopens");
        assert_eq!(
            reopened.assignment(&origin).expect("origin exists"),
            &released
        );
        let replacement = reopened
            .claim(&origin, 1, "worker-origin-b", 2)
            .expect("replacement claim advances generation");
        assert_eq!(replacement.assignment_generation, 2);
    }

    #[test]
    fn directory_excludes_a_second_writer_and_rejects_stale_material() {
        let directory_root = tempdir().expect("temporary directory");
        let active_manifest = manifest(702);
        let cells = proof_cell_keys().expect("proof keys build");
        let directory =
            LocalCellDirectory::open(directory_root.path(), &active_manifest, cells.clone())
                .expect("first directory writer opens");
        assert!(matches!(
            LocalCellDirectory::open(directory_root.path(), &active_manifest, cells.clone()),
            Err(CellDirectoryError::WriterAlreadyActive(_))
        ));
        drop(directory);

        let other_manifest = manifest(703);
        assert!(matches!(
            LocalCellDirectory::open(directory_root.path(), &other_manifest, cells),
            Err(CellDirectoryError::InvalidDirectory(_))
        ));
    }

    #[test]
    fn directory_rejects_unknown_fields_and_incomplete_transitions() {
        let directory_root = tempdir().expect("temporary directory");
        let manifest = manifest(704);
        let cells = proof_cell_keys().expect("proof keys build");
        drop(
            LocalCellDirectory::open(directory_root.path(), &manifest, cells.clone())
                .expect("directory opens"),
        );
        let path = directory_root.path().join(DIRECTORY_FILE);
        let mut document =
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).expect("directory reads"))
                .expect("directory JSON parses");
        document
            .as_object_mut()
            .expect("directory is an object")
            .insert("unexpected".into(), serde_json::json!(true));
        let mut file = File::create(&path).expect("directory opens for tamper");
        file.write_all(
            &serde_json::to_vec_pretty(&document).expect("tampered directory serializes"),
        )
        .expect("tampered directory writes");
        file.sync_all().expect("tampered directory syncs");

        assert!(matches!(
            LocalCellDirectory::open(directory_root.path(), &manifest, cells),
            Err(CellDirectoryError::Json { .. })
        ));
    }

    #[test]
    fn placement_generation_and_transfer_commit_survive_every_reopen() {
        let directory_root = tempdir().expect("temporary directory");
        let manifest = manifest(706);
        let [origin, east] = proof_cell_keys().expect("proof keys build");
        let package_hash = blake3::hash(b"player-transfer-package")
            .to_hex()
            .to_string();
        let receipt_hash = blake3::hash(b"destination-quarantine-receipt")
            .to_hex()
            .to_string();

        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("directory opens");
        directory
            .claim(&origin, 0, "worker-origin", 5)
            .expect("source assignment commits");
        directory
            .claim(&east, 0, "worker-east", 9)
            .expect("destination assignment commits");
        directory
            .register_placement("player-local", MobileAggregateKind::Player, &origin)
            .expect("initial placement commits");
        let prepared = directory
            .prepare_transfer("player-local", 1, "transfer-player-1", &east, &package_hash)
            .expect("prepare commits");
        assert_eq!(prepared.phase, TransferPhase::Prepared);
        assert_eq!(prepared.prior_placement_generation, 1);
        assert_eq!(prepared.resulting_placement_generation, 2);
        assert!(directory.commit_transfer("transfer-player-1", 1).is_err());
        assert!(directory.release(&origin, 1, "worker-origin").is_err());
        assert_eq!(
            directory
                .prepare_transfer("player-local", 1, "transfer-player-1", &east, &package_hash)
                .expect("prepare retry reconciles"),
            prepared
        );
        assert!(
            directory
                .prepare_transfer(
                    "player-local",
                    1,
                    "transfer-player-1",
                    &east,
                    &"0".repeat(64),
                )
                .is_err()
        );
        drop(directory);

        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("prepared directory reopens");
        let prepare_proof = prepare_proof(
            directory
                .transfer("transfer-player-1")
                .expect("prepared transfer exists"),
        );
        let mut forged_prepare = prepare_proof.clone();
        forged_prepare.source_event_sequence = 0;
        assert!(
            directory
                .record_source_prepared("transfer-player-1", &forged_prepare)
                .is_err()
        );
        directory
            .record_source_prepared("transfer-player-1", &prepare_proof)
            .expect("source prepare proof commits");
        assert!(directory.commit_transfer("transfer-player-1", 1).is_err());
        let quarantine_proof = quarantine_proof(
            directory
                .transfer("transfer-player-1")
                .expect("prepared transfer remains"),
            &receipt_hash,
        );
        let quarantined = directory
            .record_quarantine(
                "transfer-player-1",
                &package_hash,
                &receipt_hash,
                &quarantine_proof,
            )
            .expect("quarantine commits");
        assert_eq!(quarantined.phase, TransferPhase::Quarantined);
        drop(directory);

        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("quarantined directory reopens");
        let committed = directory
            .commit_transfer("transfer-player-1", 1)
            .expect("directory CAS commits");
        assert_eq!(committed.phase, TransferPhase::Committed);
        let placement = directory
            .placement("player-local")
            .expect("placement exists");
        assert_eq!(placement.cell_key, east);
        assert_eq!(placement.placement_generation, 2);
        assert_eq!(placement.state, AggregatePlacementState::InTransit);
        assert!(directory.request_abort("transfer-player-1").is_err());
        drop(directory);

        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("committed directory reopens");
        let import_proof = import_proof(
            directory
                .transfer("transfer-player-1")
                .expect("committed transfer exists"),
            &receipt_hash,
        );
        let mut forged_import = import_proof.clone();
        forged_import.destination_fencing_token = 0;
        assert!(
            directory
                .record_imported("transfer-player-1", &forged_import)
                .is_err()
        );
        let imported = directory
            .record_imported("transfer-player-1", &import_proof)
            .expect("import commits");
        assert_eq!(imported.phase, TransferPhase::Imported);
        assert_eq!(
            directory
                .record_imported("transfer-player-1", &import_proof)
                .expect("import retry reconciles"),
            imported
        );
        drop(directory);

        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("imported directory reopens");
        let finalization_proof = finalization_proof(
            directory
                .transfer("transfer-player-1")
                .expect("imported transfer exists"),
        );
        let mut forged_finalization = finalization_proof.clone();
        forged_finalization.source_event_sequence = 0;
        assert!(
            directory
                .finalize_transfer("transfer-player-1", &forged_finalization)
                .is_err()
        );
        let finalized = directory
            .finalize_transfer("transfer-player-1", &finalization_proof)
            .expect("finalization commits");
        assert_eq!(finalized.phase, TransferPhase::Finalized);
        let placement = directory
            .placement("player-local")
            .expect("placement exists");
        assert_eq!(placement.cell_key, east);
        assert_eq!(placement.placement_generation, 2);
        assert_eq!(placement.state, AggregatePlacementState::Resident);
        assert!(placement.active_transfer_id.is_none());

        directory
            .recover_assignment(&origin, 1, "worker-origin-successor", 6)
            .expect("source successor binds a second generation and fence");
        drop(directory);
        let path = directory_root.path().join(DIRECTORY_FILE);
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("directory reads"))
                .expect("directory parses");
        document["transfers"]["transfer-player-1"]["finalization_proof"]["source_assignment_generation"] =
            serde_json::json!(2);
        fs::write(
            &path,
            serde_json::to_vec_pretty(&document).expect("tampered directory serializes"),
        )
        .expect("tampered directory writes");
        assert!(matches!(
            LocalCellDirectory::open(directory_root.path(), &manifest, [origin, east]),
            Err(CellDirectoryError::InvalidDirectory(_))
        ));
    }

    #[test]
    fn precommit_abort_remains_pinned_until_both_cell_cleanups_are_proved() {
        let directory_root = tempdir().expect("temporary directory");
        let manifest = manifest(707);
        let [origin, east] = proof_cell_keys().expect("proof keys build");
        let package_hash = blake3::hash(b"abortable-package").to_hex().to_string();
        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("directory opens");
        directory
            .claim(&origin, 0, "worker-origin", 5)
            .expect("source assignment commits");
        directory
            .claim(&east, 0, "worker-east", 9)
            .expect("destination assignment commits");
        directory
            .register_placement("grid-mobile", MobileAggregateKind::Grid, &origin)
            .expect("placement commits");
        directory
            .prepare_transfer(
                "grid-mobile",
                1,
                "transfer-grid-abort",
                &east,
                &package_hash,
            )
            .expect("prepare commits");
        let aborting = directory
            .request_abort("transfer-grid-abort")
            .expect("precommit abort begins");
        assert_eq!(aborting.phase, TransferPhase::Aborting);
        assert!(directory.finalize_abort("transfer-grid-abort").is_err());
        assert!(directory.release(&origin, 1, "worker-origin").is_err());
        let placement = directory
            .placement("grid-mobile")
            .expect("placement exists");
        assert_eq!(placement.cell_key, origin);
        assert_eq!(placement.placement_generation, 1);
        assert_eq!(placement.state, AggregatePlacementState::Preparing);
    }

    #[test]
    fn proof_cells_materialize_independent_fenced_lifecycle_roots() {
        let directory_root = tempdir().expect("temporary directory");
        let manifest = manifest(705);
        let [origin, east] = proof_cell_keys().expect("proof keys build");
        let directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("directory opens");
        let origin_root = directory
            .cell_store_root(&origin)
            .expect("origin root derives");
        let east_root = directory.cell_store_root(&east).expect("east root derives");

        let mut origin_store =
            Store::open_for_cell(&origin_root, 705, origin.clone()).expect("origin store opens");
        let mut origin_world = origin_store.load_world().expect("origin world loads");
        origin_world.fencing_token = origin_store.fencing_token();
        origin_store
            .save_snapshot(&origin_world)
            .expect("origin snapshot persists");

        let mut east_store =
            Store::open_for_cell(&east_root, 705, east.clone()).expect("east store opens");
        let mut east_world = east_store.load_world().expect("east world loads");
        east_world.fencing_token = east_store.fencing_token();
        east_store
            .save_snapshot(&east_world)
            .expect("east snapshot persists");

        assert_ne!(origin_store.cell_id(), east_store.cell_id());
        assert_eq!(origin_store.cell_key(), &origin);
        assert_eq!(east_store.cell_key(), &east);
        assert!(!origin_world.player.by_id.is_empty());
        assert!(east_world.player.by_id.is_empty());
        assert!(east_world.grids.is_empty());
        assert!(east_world.voxels.occupied.is_empty());

        east_store
            .publish_active(&east_world)
            .expect("empty destination activates for controlled materialization");
        east_store
            .transition_mode(
                LifecycleMode::Sleeping,
                LifecycleMode::Draining,
                &east_world,
            )
            .expect("empty destination drains");
        east_store
            .release_to_sleeping(&east_world)
            .expect("empty destination sleeps");
        assert_eq!(east_store.lifecycle_mode(), LifecycleMode::Sleeping);
    }
}
