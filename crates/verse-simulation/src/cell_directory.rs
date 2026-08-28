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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferPhase {
    Prepared,
    Quarantined,
    Committed,
    Imported,
    Finalized,
    Aborted,
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
    pub phase: TransferPhase,
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
    pub fn open(
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

    pub fn claim(
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
        let result = assignment.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub fn release(
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
        assignment.state = CellAssignmentState::Sleeping;
        assignment.holder_id = None;
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

    pub fn register_placement(
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

    pub fn prepare_transfer(
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

    pub fn record_quarantine(
        &mut self,
        transfer_id: &str,
        package_hash: &str,
        receipt_hash: &str,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        validate_hash(package_hash, "transfer package")?;
        validate_hash(receipt_hash, "quarantine receipt")?;
        let current = self.transfer(transfer_id)?.clone();
        if current.package_hash != package_hash {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "quarantine package hash does not match prepare".into(),
            });
        }
        if current.phase == TransferPhase::Quarantined
            && current.quarantine_receipt_hash.as_deref() == Some(receipt_hash)
        {
            return Ok(current);
        }
        if current.phase != TransferPhase::Prepared || current.quarantine_receipt_hash.is_some() {
            return Err(CellDirectoryError::TransferConflict {
                transfer_id: transfer_id.to_owned(),
                reason: "transfer is not awaiting its first quarantine receipt".into(),
            });
        }
        let mut next_document = self.document.clone();
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Quarantined;
        transfer.quarantine_receipt_hash = Some(receipt_hash.to_owned());
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub fn commit_transfer(
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
            || source_assignment.assignment_generation != current.source_assignment_generation
            || destination_assignment.assignment_generation
                != current.destination_assignment_generation
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

    pub fn record_imported(
        &mut self,
        transfer_id: &str,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let current = self.transfer(transfer_id)?.clone();
        if matches!(
            current.phase,
            TransferPhase::Imported | TransferPhase::Finalized
        ) {
            return Ok(current);
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
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub fn finalize_transfer(
        &mut self,
        transfer_id: &str,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let current = self.transfer(transfer_id)?.clone();
        if current.phase == TransferPhase::Finalized {
            return Ok(current);
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
        let mut next_document = self.document.clone();
        let transfer = next_document
            .transfers
            .get_mut(transfer_id)
            .expect("validated transfer exists in cloned document");
        transfer.phase = TransferPhase::Finalized;
        let result = transfer.clone();
        self.commit_document(next_document)?;
        Ok(result)
    }

    pub fn abort_transfer(
        &mut self,
        transfer_id: &str,
    ) -> Result<CellTransferRecord, CellDirectoryError> {
        let current = self.transfer(transfer_id)?.clone();
        if current.phase == TransferPhase::Aborted {
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

    pub fn cell_store_root(&self, cell_key: &CellKeyV1) -> Result<PathBuf, CellDirectoryError> {
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
        let placement_matches = match transfer.phase {
            TransferPhase::Prepared | TransferPhase::Quarantined => {
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
            .claim(&origin, 0, "worker-origin-a")
            .expect("origin claim commits");
        assert_eq!(claimed.assignment_generation, 1);
        assert_eq!(claimed.state, CellAssignmentState::Assigned);
        assert_eq!(claimed.holder_id.as_deref(), Some("worker-origin-a"));
        assert!(directory.claim(&origin, 0, "worker-origin-b").is_err());

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
            .claim(&origin, 1, "worker-origin-b")
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
            .claim(&origin, 0, "worker-origin")
            .expect("source assignment commits");
        directory
            .claim(&east, 0, "worker-east")
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
        let quarantined = directory
            .record_quarantine("transfer-player-1", &package_hash, &receipt_hash)
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
        assert!(directory.abort_transfer("transfer-player-1").is_err());
        drop(directory);

        let mut directory = LocalCellDirectory::open(
            directory_root.path(),
            &manifest,
            [origin.clone(), east.clone()],
        )
        .expect("committed directory reopens");
        let imported = directory
            .record_imported("transfer-player-1")
            .expect("import commits");
        assert_eq!(imported.phase, TransferPhase::Imported);
        assert_eq!(
            directory
                .record_imported("transfer-player-1")
                .expect("import retry reconciles"),
            imported
        );
        drop(directory);

        let mut directory =
            LocalCellDirectory::open(directory_root.path(), &manifest, [origin, east.clone()])
                .expect("imported directory reopens");
        let finalized = directory
            .finalize_transfer("transfer-player-1")
            .expect("finalization commits");
        assert_eq!(finalized.phase, TransferPhase::Finalized);
        let placement = directory
            .placement("player-local")
            .expect("placement exists");
        assert_eq!(placement.cell_key, east);
        assert_eq!(placement.placement_generation, 2);
        assert_eq!(placement.state, AggregatePlacementState::Resident);
        assert!(placement.active_transfer_id.is_none());
    }

    #[test]
    fn only_precommit_transfer_can_abort_back_to_source() {
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
            .claim(&origin, 0, "worker-origin")
            .expect("source assignment commits");
        directory
            .claim(&east, 0, "worker-east")
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
        let aborted = directory
            .abort_transfer("transfer-grid-abort")
            .expect("precommit abort succeeds");
        assert_eq!(aborted.phase, TransferPhase::Aborted);
        let placement = directory
            .placement("grid-mobile")
            .expect("placement exists");
        assert_eq!(placement.cell_key, origin);
        assert_eq!(placement.placement_generation, 1);
        assert_eq!(placement.state, AggregatePlacementState::Resident);
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
