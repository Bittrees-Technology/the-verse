// SPDX-License-Identifier: AGPL-3.0-or-later

//! Offline, read-only protocol-18 source validation for the protocol-19
//! migration. This module proves only the frozen legacy side. It does not mint
//! a migration receipt, stage target state, or grant activation authority.

use std::collections::BTreeMap;
use std::path::Path;

use thiserror::Error;
use verse_protocol::CellKeyV1;

use crate::cell_directory::{
    CellAssignmentRecord, CellDirectoryError, CellTransferAbortProof,
    CellTransferFinalizationProof, CellTransferImportProof, CellTransferPrepareProof,
    CellTransferQuarantineProof, FrozenProtocol18Directory, TransferAbortRole,
};
use crate::persistence::{
    DurableTransferBoundary, FrozenProtocol18Cell, PersistenceError, TransferBoundaryKind,
};
use crate::{EVENT_SCHEMA_VERSION, WORLD_SCHEMA_VERSION, celestial};

#[derive(Debug, Error)]
pub(crate) enum FrozenProtocol18SourceError {
    #[error(transparent)]
    Directory(#[from] CellDirectoryError),
    #[error(transparent)]
    Cell(#[from] PersistenceError),
    #[error("frozen protocol-18 source invariant failed: {0}")]
    Invalid(String),
}

#[derive(Debug)]
pub(crate) struct FrozenProtocol18CellEvidence {
    assignment_generation: u64,
    fencing_history_root: String,
    cell: FrozenProtocol18Cell,
}

impl FrozenProtocol18CellEvidence {
    pub(crate) fn state(&self) -> &crate::WorldState {
        self.cell.state()
    }

    pub(crate) fn cell_key(&self) -> &CellKeyV1 {
        self.cell.cell_key()
    }

    pub(crate) fn cell_id(&self) -> &str {
        self.cell.cell_id()
    }

    pub(crate) const fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    pub(crate) const fn authority_fencing_token(&self) -> u64 {
        self.cell.authority_fencing_token()
    }

    pub(crate) fn fencing_history_root(&self) -> &str {
        &self.fencing_history_root
    }

    pub(crate) fn world_state_hash(&self) -> &str {
        self.cell.world_state_hash()
    }

    pub(crate) fn snapshot_document_hash(&self) -> &str {
        self.cell.snapshot_document_hash()
    }

    pub(crate) const fn event_sequence(&self) -> u64 {
        self.cell.event_sequence()
    }

    pub(crate) fn event_head_hash(&self) -> &str {
        self.cell.event_head_hash()
    }

    pub(crate) const fn event_archive_entry_count(&self) -> u64 {
        self.cell.event_archive_entry_count()
    }

    pub(crate) fn event_archive_root(&self) -> &str {
        self.cell.event_archive_root()
    }

    pub(crate) const fn lifecycle_revision(&self) -> u64 {
        self.cell.lifecycle_revision()
    }

    pub(crate) fn lifecycle_record_hash(&self) -> &str {
        self.cell.lifecycle_record_hash()
    }

    pub(crate) const fn transfer_boundary_entry_count(&self) -> u64 {
        self.cell.transfer_boundary_entry_count()
    }

    pub(crate) fn transfer_boundary_head_hash(&self) -> &str {
        self.cell.transfer_boundary_head_hash()
    }

    pub(crate) fn transfer_boundary_archive_root(&self) -> &str {
        self.cell.transfer_boundary_archive_root()
    }

    pub(crate) const fn acknowledged_production_sequence(&self) -> u64 {
        self.cell.acknowledged_production_sequence()
    }

    pub(crate) fn next_production_occurrence_root(&self) -> &str {
        self.cell.next_production_occurrence_root()
    }

    pub(crate) const fn last_trusted_unix_ms(&self) -> u64 {
        self.cell.last_trusted_unix_ms()
    }
}

/// Non-Serde, non-cloneable source capability. Its lifetime owns the existing
/// directory-v2 lock and every existing protocol-18 cell writer lock.
#[derive(Debug)]
pub(crate) struct ValidatedFrozenProtocol18Source {
    world_seed: u64,
    universe_id: String,
    source_manifest_hash: String,
    directory: FrozenProtocol18Directory,
    cells: Vec<FrozenProtocol18CellEvidence>,
}

impl ValidatedFrozenProtocol18Source {
    pub(crate) fn acquire_existing(
        root: impl AsRef<Path>,
        world_seed: u64,
    ) -> Result<Self, FrozenProtocol18SourceError> {
        let root = root.as_ref();
        let manifest =
            celestial::universe_manifest(world_seed, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
                .map_err(|source| FrozenProtocol18SourceError::Invalid(source.to_string()))?;
        let proof_cells = crate::cell_directory::proof_cell_keys()?;
        let directory =
            FrozenProtocol18Directory::lock_existing(root, &manifest, proof_cells.clone())?;
        let assignments = directory.assignments().cloned().collect::<Vec<_>>();
        if assignments.len() != proof_cells.len()
            || assignments
                .windows(2)
                .any(|pair| pair[0].cell_id >= pair[1].cell_id)
        {
            return Err(FrozenProtocol18SourceError::Invalid(
                "directory cells are not one canonical ordered proof set".into(),
            ));
        }

        let mut cells = Vec::with_capacity(assignments.len());
        for assignment in &assignments {
            let cell = FrozenProtocol18Cell::lock_existing(
                directory.cell_store_root(assignment),
                world_seed,
                assignment.cell_key.clone(),
                assignment.authority_fencing_token,
            )?;
            validate_cell_fences(assignment, &cell)?;
            cells.push(FrozenProtocol18CellEvidence {
                assignment_generation: assignment.assignment_generation,
                fencing_history_root: directory.fencing_history_root(assignment)?,
                cell,
            });
        }
        validate_directory_transfer_evidence(&directory, &cells)?;

        Ok(Self {
            world_seed,
            universe_id: manifest.universe_id,
            source_manifest_hash: manifest.manifest_hash,
            directory,
            cells,
        })
    }

    pub(crate) const fn world_seed(&self) -> u64 {
        self.world_seed
    }

    pub(crate) fn universe_id(&self) -> &str {
        &self.universe_id
    }

    pub(crate) fn source_manifest_hash(&self) -> &str {
        &self.source_manifest_hash
    }

    pub(crate) const fn directory_revision(&self) -> u64 {
        self.directory.directory_revision()
    }

    pub(crate) fn directory_document_hash(&self) -> &str {
        self.directory.document_hash()
    }

    pub(crate) fn terminal_transfer_count(&self) -> u64 {
        self.directory.terminal_transfer_count()
    }

    pub(crate) fn terminal_transfer_root(&self) -> &str {
        self.directory.terminal_transfer_root()
    }

    pub(crate) fn assignment_root(&self) -> &str {
        self.directory.assignment_root()
    }

    pub(crate) fn placement_root(&self) -> &str {
        self.directory.placement_root()
    }

    pub(crate) fn cells(&self) -> &[FrozenProtocol18CellEvidence] {
        &self.cells
    }
}

fn validate_cell_fences(
    assignment: &CellAssignmentRecord,
    cell: &FrozenProtocol18Cell,
) -> Result<(), FrozenProtocol18SourceError> {
    if cell.cell_key() != &assignment.cell_key
        || cell.cell_id() != assignment.cell_id
        || cell.authority_fencing_token() != assignment.authority_fencing_token
    {
        return Err(FrozenProtocol18SourceError::Invalid(format!(
            "cell {} differs from its frozen directory assignment",
            assignment.cell_id
        )));
    }
    for event in cell.events() {
        if !assignment
            .fencing_history
            .values()
            .any(|issued| *issued == event.authority_fencing_token)
        {
            return Err(FrozenProtocol18SourceError::Invalid(format!(
                "cell {} event {} uses a fence absent from directory history",
                assignment.cell_id, event.event_sequence
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct BoundaryEvidence {
    transfer_id: String,
    package_hash: String,
    receipt_hash: Option<String>,
    cell_id: String,
    kind: TransferBoundaryKind,
    fencing_token: u64,
    event_sequence: u64,
    event_hash: String,
    world_hash: String,
}

fn observed_boundary(boundary: &DurableTransferBoundary) -> BoundaryEvidence {
    BoundaryEvidence {
        transfer_id: boundary.transfer_id.clone(),
        package_hash: boundary.package_hash.clone(),
        receipt_hash: boundary.receipt_hash.clone(),
        cell_id: boundary.cell_id.clone(),
        kind: boundary.kind,
        fencing_token: boundary.authority_fencing_token,
        event_sequence: boundary.event_sequence,
        event_hash: boundary.event_hash.clone(),
        world_hash: boundary.resulting_world_hash.clone(),
    }
}

fn prepare_boundary(proof: &CellTransferPrepareProof) -> BoundaryEvidence {
    BoundaryEvidence {
        transfer_id: proof.transfer_id.clone(),
        package_hash: proof.package_hash.clone(),
        receipt_hash: None,
        cell_id: proof.source_cell_id.clone(),
        kind: TransferBoundaryKind::Prepare,
        fencing_token: proof.source_fencing_token,
        event_sequence: proof.source_event_sequence,
        event_hash: proof.source_event_hash.clone(),
        world_hash: proof.source_world_hash.clone(),
    }
}

fn quarantine_boundary(proof: &CellTransferQuarantineProof) -> BoundaryEvidence {
    BoundaryEvidence {
        transfer_id: proof.transfer_id.clone(),
        package_hash: proof.package_hash.clone(),
        receipt_hash: Some(proof.quarantine_receipt_hash.clone()),
        cell_id: proof.destination_cell_id.clone(),
        kind: TransferBoundaryKind::Quarantine,
        fencing_token: proof.destination_fencing_token,
        event_sequence: proof.destination_event_sequence,
        event_hash: proof.destination_event_hash.clone(),
        world_hash: proof.destination_world_hash.clone(),
    }
}

fn import_boundary(proof: &CellTransferImportProof) -> BoundaryEvidence {
    BoundaryEvidence {
        transfer_id: proof.transfer_id.clone(),
        package_hash: proof.package_hash.clone(),
        receipt_hash: Some(proof.quarantine_receipt_hash.clone()),
        cell_id: proof.destination_cell_id.clone(),
        kind: TransferBoundaryKind::Import,
        fencing_token: proof.destination_fencing_token,
        event_sequence: proof.destination_event_sequence,
        event_hash: proof.destination_event_hash.clone(),
        world_hash: proof.destination_world_hash.clone(),
    }
}

fn finalization_boundary(proof: &CellTransferFinalizationProof) -> BoundaryEvidence {
    BoundaryEvidence {
        transfer_id: proof.transfer_id.clone(),
        package_hash: proof.package_hash.clone(),
        receipt_hash: None,
        cell_id: proof.source_cell_id.clone(),
        kind: TransferBoundaryKind::Export,
        fencing_token: proof.source_fencing_token,
        event_sequence: proof.source_event_sequence,
        event_hash: proof.source_event_hash.clone(),
        world_hash: proof.source_world_hash.clone(),
    }
}

fn abort_boundary(proof: &CellTransferAbortProof) -> BoundaryEvidence {
    BoundaryEvidence {
        transfer_id: proof.transfer_id.clone(),
        package_hash: proof.package_hash.clone(),
        receipt_hash: None,
        cell_id: proof.cell_id.clone(),
        kind: match proof.role {
            TransferAbortRole::Source => TransferBoundaryKind::AbortSource,
            TransferAbortRole::Destination => TransferBoundaryKind::AbortDestination,
        },
        fencing_token: proof.fencing_token,
        event_sequence: proof.event_sequence,
        event_hash: proof.event_hash.clone(),
        world_hash: proof.world_hash.clone(),
    }
}

fn insert_expected(
    expected: &mut BTreeMap<(String, TransferBoundaryKind), BoundaryEvidence>,
    evidence: BoundaryEvidence,
) -> Result<(), FrozenProtocol18SourceError> {
    let key = (evidence.transfer_id.clone(), evidence.kind);
    if expected.insert(key, evidence).is_some() {
        return Err(FrozenProtocol18SourceError::Invalid(
            "directory repeats one transfer proof boundary".into(),
        ));
    }
    Ok(())
}

fn validate_directory_transfer_evidence(
    directory: &FrozenProtocol18Directory,
    cells: &[FrozenProtocol18CellEvidence],
) -> Result<(), FrozenProtocol18SourceError> {
    let mut expected = BTreeMap::new();
    for transfer in directory.transfers() {
        if let Some(proof) = &transfer.source_prepare_proof {
            insert_expected(&mut expected, prepare_boundary(proof))?;
        }
        if let Some(proof) = &transfer.destination_quarantine_proof {
            insert_expected(&mut expected, quarantine_boundary(proof))?;
        }
        if let Some(proof) = &transfer.import_proof {
            insert_expected(&mut expected, import_boundary(proof))?;
        }
        if let Some(proof) = &transfer.finalization_proof {
            insert_expected(&mut expected, finalization_boundary(proof))?;
        }
        if let Some(proof) = &transfer.source_abort_proof {
            insert_expected(&mut expected, abort_boundary(proof))?;
        }
        if let Some(proof) = &transfer.destination_abort_proof {
            insert_expected(&mut expected, abort_boundary(proof))?;
        }
    }

    let mut observed = BTreeMap::new();
    for cell in cells {
        for boundary in cell.cell.transfer_boundaries() {
            let evidence = observed_boundary(boundary);
            let key = (evidence.transfer_id.clone(), evidence.kind);
            if observed.insert(key, evidence).is_some() {
                return Err(FrozenProtocol18SourceError::Invalid(
                    "cell archives repeat one directory transfer boundary".into(),
                ));
            }
        }
    }
    if expected != observed {
        return Err(FrozenProtocol18SourceError::Invalid(
            "directory transfer proofs differ from exact cell event boundaries".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs::{self, OpenOptions};
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    use fs2::FileExt;
    use serde_json::Value;
    use tempfile::{TempDir, tempdir};
    use verse_protocol::ClientMessage;

    use super::*;
    use crate::{LifecycleMode, LocalCellDirectory, Runtime};

    const TEST_SEED: u64 = 8_119;

    fn frozen_fixture_with_profile(with_event: bool, surface_profile: bool) -> TempDir {
        let root = tempdir().expect("temporary universe root");
        let manifest = celestial::universe_manifest(
            TEST_SEED,
            crate::WORLD_SCHEMA_VERSION,
            crate::EVENT_SCHEMA_VERSION,
        )
        .expect("manifest builds");
        let cell_keys = crate::proof_cell_keys().expect("proof cells build");
        let mut directory = LocalCellDirectory::open(root.path(), &manifest, cell_keys.clone())
            .expect("directory initializes");
        for (index, cell_key) in cell_keys.into_iter().enumerate() {
            let prior = directory
                .assignment(&cell_key)
                .expect("assignment exists")
                .clone();
            let holder_id = format!("freeze-fixture-{index}");
            let cell_root = directory
                .cell_store_root(&cell_key)
                .expect("cell root derives");
            let mut runtime = Runtime::open_directory_managed_for_cell(
                &cell_root,
                TEST_SEED,
                cell_key.clone(),
                1,
            )
            .expect("cell runtime opens");
            let assigned = directory
                .claim(
                    &cell_key,
                    prior.assignment_generation,
                    &holder_id,
                    runtime.state().fencing_token,
                )
                .expect("cell assignment claims");
            if surface_profile && index == 0 {
                assert!(
                    runtime
                        .configure_earth_start_playtest()
                        .expect("canonical surface profile configures")
                );
            }
            if with_event && index == 0 {
                runtime
                    .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                        operation_sequence: 0,
                        operation_id: "frozen-source-event".into(),
                        helmet_closed: surface_profile,
                        jetpack_enabled: surface_profile,
                        magnetic_boots_enabled: false,
                    })
                    .expect("canonical event commits");
            }
            assert_eq!(
                runtime
                    .drain_to_background_or_sleeping()
                    .expect("cell drains"),
                LifecycleMode::Sleeping
            );
            directory
                .release(&cell_key, assigned.assignment_generation, &holder_id)
                .expect("directory assignment releases");
        }
        drop(directory);
        root
    }

    fn frozen_fixture(with_event: bool) -> TempDir {
        frozen_fixture_with_profile(with_event, false)
    }

    fn stale_mid_journal_fixture() -> TempDir {
        let root = tempdir().expect("temporary universe root");
        let manifest = celestial::universe_manifest(
            TEST_SEED,
            crate::WORLD_SCHEMA_VERSION,
            crate::EVENT_SCHEMA_VERSION,
        )
        .expect("manifest builds");
        let cell_keys = crate::proof_cell_keys().expect("proof cells build");
        let mut directory = LocalCellDirectory::open(root.path(), &manifest, cell_keys.clone())
            .expect("directory initializes");

        let origin_key = cell_keys[0].clone();
        let origin_root = directory
            .cell_store_root(&origin_key)
            .expect("origin root derives");
        let first_assignment = directory
            .assignment(&origin_key)
            .expect("origin assignment exists")
            .clone();
        let mut first_runtime = Runtime::open_directory_managed_for_cell(
            &origin_root,
            TEST_SEED,
            origin_key.clone(),
            1,
        )
        .expect("first origin runtime opens");
        let first_holder = "stale-fence-first";
        let first_claim = directory
            .claim(
                &origin_key,
                first_assignment.assignment_generation,
                first_holder,
                first_runtime.state().fencing_token,
            )
            .expect("first origin claim commits");
        first_runtime
            .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                operation_sequence: 0,
                operation_id: "stale-fence-event-one".into(),
                helmet_closed: false,
                jetpack_enabled: false,
                magnetic_boots_enabled: false,
            })
            .expect("first canonical event commits");
        let snapshot_path = origin_root.join("world-snapshot.json");
        let stale_snapshot = fs::read(&snapshot_path).expect("mid-journal snapshot reads");
        assert_eq!(
            first_runtime
                .drain_to_background_or_sleeping()
                .expect("first origin runtime drains"),
            LifecycleMode::Sleeping
        );
        directory
            .release(&origin_key, first_claim.assignment_generation, first_holder)
            .expect("first origin assignment releases");
        drop(first_runtime);

        let second_assignment = directory
            .assignment(&origin_key)
            .expect("second origin assignment exists")
            .clone();
        let mut second_runtime = Runtime::open_directory_managed_for_cell(
            &origin_root,
            TEST_SEED,
            origin_key.clone(),
            1,
        )
        .expect("second origin runtime opens");
        let second_holder = "stale-fence-second";
        let second_claim = directory
            .claim(
                &origin_key,
                second_assignment.assignment_generation,
                second_holder,
                second_runtime.state().fencing_token,
            )
            .expect("second origin claim commits");
        second_runtime
            .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                operation_sequence: 0,
                operation_id: "stale-fence-event-two".into(),
                helmet_closed: true,
                jetpack_enabled: true,
                magnetic_boots_enabled: false,
            })
            .expect("second canonical event commits");
        assert_eq!(
            second_runtime
                .drain_to_background_or_sleeping()
                .expect("second origin runtime drains"),
            LifecycleMode::Sleeping
        );
        directory
            .release(
                &origin_key,
                second_claim.assignment_generation,
                second_holder,
            )
            .expect("second origin assignment releases");
        drop(second_runtime);
        fs::write(&snapshot_path, stale_snapshot).expect("stale snapshot substitutes frontier");
        crate::persistence::set_snapshot_fencing_token_for_test(&snapshot_path, 0)
            .expect("stale snapshot fence rewrites canonically");

        let neighbor_key = cell_keys[1].clone();
        let neighbor_assignment = directory
            .assignment(&neighbor_key)
            .expect("neighbor assignment exists")
            .clone();
        let neighbor_root = directory
            .cell_store_root(&neighbor_key)
            .expect("neighbor root derives");
        let mut neighbor_runtime = Runtime::open_directory_managed_for_cell(
            neighbor_root,
            TEST_SEED,
            neighbor_key.clone(),
            1,
        )
        .expect("neighbor runtime opens");
        let neighbor_holder = "stale-fence-neighbor";
        let neighbor_claim = directory
            .claim(
                &neighbor_key,
                neighbor_assignment.assignment_generation,
                neighbor_holder,
                neighbor_runtime.state().fencing_token,
            )
            .expect("neighbor claim commits");
        assert_eq!(
            neighbor_runtime
                .drain_to_background_or_sleeping()
                .expect("neighbor runtime drains"),
            LifecycleMode::Sleeping
        );
        directory
            .release(
                &neighbor_key,
                neighbor_claim.assignment_generation,
                neighbor_holder,
            )
            .expect("neighbor assignment releases");
        drop(neighbor_runtime);
        drop(directory);
        root
    }

    fn collect_files(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, (Vec<u8>, SystemTime)>) {
            let mut entries = fs::read_dir(path)
                .expect("directory reads")
                .collect::<Result<Vec<_>, _>>()
                .expect("entries read");
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                let path = entry.path();
                let metadata = entry.metadata().expect("metadata reads");
                if metadata.is_dir() {
                    visit(root, &path, files);
                } else if metadata.is_file() {
                    files.insert(
                        path.strip_prefix(root)
                            .expect("path is below root")
                            .to_path_buf(),
                        (
                            fs::read(&path).expect("file reads"),
                            metadata.modified().expect("modified time reads"),
                        ),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    fn summary(source: &ValidatedFrozenProtocol18Source) -> Vec<String> {
        let mut values = vec![
            source.world_seed().to_string(),
            source.universe_id().to_owned(),
            source.source_manifest_hash().to_owned(),
            source.directory_revision().to_string(),
            source.directory_document_hash().to_owned(),
            source.terminal_transfer_count().to_string(),
            source.terminal_transfer_root().to_owned(),
            source.assignment_root().to_owned(),
            source.placement_root().to_owned(),
        ];
        for cell in source.cells() {
            values.extend([
                cell.cell_id().to_owned(),
                cell.assignment_generation().to_string(),
                cell.authority_fencing_token().to_string(),
                cell.fencing_history_root().to_owned(),
                cell.world_state_hash().to_owned(),
                cell.snapshot_document_hash().to_owned(),
                cell.event_sequence().to_string(),
                cell.event_head_hash().to_owned(),
                cell.event_archive_entry_count().to_string(),
                cell.event_archive_root().to_owned(),
                cell.lifecycle_revision().to_string(),
                cell.lifecycle_record_hash().to_owned(),
                cell.transfer_boundary_entry_count().to_string(),
                cell.transfer_boundary_head_hash().to_owned(),
                cell.transfer_boundary_archive_root().to_owned(),
                cell.acknowledged_production_sequence().to_string(),
                cell.next_production_occurrence_root().to_owned(),
                cell.last_trusted_unix_ms().to_string(),
                cell.state().state_hash(),
                serde_json::to_string(cell.cell_key()).expect("cell key encodes"),
            ]);
        }
        values
    }

    fn cell_root(root: &Path, index: usize) -> PathBuf {
        let cell_key = crate::proof_cell_keys().expect("proof cells build")[index].clone();
        root.join("cells")
            .join(celestial::cell_id(&cell_key).expect("cell ID derives"))
    }

    fn rewrite_pretty(path: &Path, mutate: impl FnOnce(&mut Value)) {
        let mut value: Value =
            serde_json::from_slice(&fs::read(path).expect("artifact reads")).expect("JSON parses");
        mutate(&mut value);
        fs::write(
            path,
            serde_json::to_vec_pretty(&value).expect("pretty JSON encodes"),
        )
        .expect("artifact rewrites");
    }

    #[test]
    fn real_frozen_two_cell_source_is_read_only_and_repeatable() {
        let root = frozen_fixture(true);
        let before = collect_files(root.path());
        let first = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect("frozen source validates");
        assert_eq!(first.cells().len(), 2);
        assert_eq!(first.cells()[0].event_archive_entry_count(), 1);
        assert_eq!(first.cells()[1].event_archive_entry_count(), 0);
        let first_summary = summary(&first);
        assert_eq!(collect_files(root.path()), before);

        assert!(matches!(
            ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED),
            Err(FrozenProtocol18SourceError::Directory(
                CellDirectoryError::WriterAlreadyActive(_)
            ))
        ));
        drop(first);

        let second = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect("frozen source reacquires");
        assert_eq!(summary(&second), first_summary);
        assert_eq!(collect_files(root.path()), before);
    }

    #[test]
    fn canonical_surface_profile_is_replayed_but_resealed_variants_fail() {
        let root = frozen_fixture_with_profile(true, true);
        ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect("canonical surface-profile source validates");

        let snapshot_path = cell_root(root.path(), 0).join("world-snapshot.json");
        let lifecycle_path = cell_root(root.path(), 0).join("cell-lifecycle.json");
        let mut snapshot: Value =
            serde_json::from_slice(&fs::read(&snapshot_path).expect("surface snapshot reads"))
                .expect("surface snapshot parses");
        let mut state: crate::WorldState =
            serde_json::from_value(snapshot["state"].clone()).expect("surface state parses");
        state.ledger.genesis_installed_components = state
            .ledger
            .genesis_installed_components
            .checked_add(1)
            .expect("test ledger has capacity");
        let forged_hash = state.state_hash();
        snapshot["state"] = serde_json::to_value(state).expect("forged state encodes");
        snapshot["state_hash"] = serde_json::json!(forged_hash);
        fs::write(
            &snapshot_path,
            serde_json::to_vec_pretty(&snapshot).expect("forged snapshot encodes"),
        )
        .expect("forged snapshot writes");
        rewrite_pretty(&lifecycle_path, |lifecycle| {
            lifecycle["last_world_state_hash"] = serde_json::json!(forged_hash);
        });

        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
    }

    #[test]
    fn busy_cell_and_late_failure_release_every_earlier_lock() {
        let root = frozen_fixture(false);
        let first_cell_root = cell_root(root.path(), 0);
        let writer_path = first_cell_root.join("writer.lock");
        let held = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&writer_path)
            .expect("writer lock opens");
        held.try_lock_exclusive().expect("test owns cell lock");
        assert!(matches!(
            ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED),
            Err(FrozenProtocol18SourceError::Cell(
                PersistenceError::WriterAlreadyActive(_)
            ))
        ));
        FileExt::unlock(&held).expect("test cell lock releases");
        drop(held);

        let second_snapshot = cell_root(root.path(), 1).join("world-snapshot.json");
        let canonical = fs::read(&second_snapshot).expect("snapshot reads");
        let mut noncanonical = canonical.clone();
        noncanonical.push(b' ');
        fs::write(&second_snapshot, &noncanonical).expect("snapshot corrupts");
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        assert_eq!(
            fs::read(&second_snapshot).expect("snapshot remains readable"),
            noncanonical
        );
        fs::write(&second_snapshot, canonical).expect("snapshot restores");

        ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect("all locks were released after late failure");
    }

    #[test]
    fn strict_archives_and_pending_lifecycle_reject_without_recovery() {
        let root = frozen_fixture(true);
        let events_path = cell_root(root.path(), 0).join("events.ndjson");
        let canonical_events = fs::read(&events_path).expect("events read");
        let mut torn_events = canonical_events.clone();
        torn_events.pop();
        fs::write(&events_path, &torn_events).expect("event tail tears");
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        assert_eq!(fs::read(&events_path).expect("events reread"), torn_events);
        fs::write(&events_path, canonical_events).expect("events restore");

        let canonical_events = fs::read(&events_path).expect("events read again");
        let mut whitespace_alias = vec![b' '];
        whitespace_alias.extend_from_slice(&canonical_events);
        fs::write(&events_path, &whitespace_alias).expect("event alias writes");
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        assert_eq!(
            fs::read(&events_path).expect("event alias rereads"),
            whitespace_alias
        );
        fs::write(&events_path, canonical_events).expect("events restore again");

        let boundaries_path = cell_root(root.path(), 0).join("transfer-boundaries.ndjson");
        fs::write(&boundaries_path, b" ").expect("boundary tail corrupts");
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        assert_eq!(fs::read(&boundaries_path).expect("boundaries reread"), b" ");
        fs::write(&boundaries_path, b"").expect("boundaries restore");

        let lifecycle_path = cell_root(root.path(), 0).join("cell-lifecycle.json");
        let canonical_lifecycle = fs::read(&lifecycle_path).expect("lifecycle reads");
        rewrite_pretty(&lifecycle_path, |lifecycle| {
            lifecycle["pending_world_commit"] = serde_json::json!({
                "event_sequence": 2,
                "event_hash": "1".repeat(64),
                "occurred_at_unix_ms": 1,
                "prior_next_occurrence": null,
                "resulting_next_occurrence": null
            });
        });
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        let pending = fs::read(&lifecycle_path).expect("pending lifecycle remains");
        assert_ne!(pending, canonical_lifecycle);
    }

    #[test]
    fn oversized_directory_and_zero_trusted_time_fail_without_mutation() {
        let root = frozen_fixture(false);
        let directory_path = root.path().join("cell-directory.json");
        let canonical_directory = fs::read(&directory_path).expect("directory reads");
        let oversized_length = 64_u64 * 1_024 * 1_024 + 1;
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&directory_path)
            .expect("directory opens for oversize fixture")
            .set_len(oversized_length)
            .expect("directory expands beyond the frozen bound");
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        assert_eq!(
            fs::metadata(&directory_path)
                .expect("oversized directory remains")
                .len(),
            oversized_length
        );
        fs::write(&directory_path, canonical_directory).expect("directory restores");

        let lifecycle_path = cell_root(root.path(), 0).join("cell-lifecycle.json");
        rewrite_pretty(&lifecycle_path, |lifecycle| {
            lifecycle["last_trusted_unix_ms"] = serde_json::json!(0);
            lifecycle["updated_at_unix_ms"] = serde_json::json!(0);
        });
        let zero_time = fs::read(&lifecycle_path).expect("zero-time lifecycle reads");
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        assert_eq!(
            fs::read(&lifecycle_path).expect("zero-time lifecycle remains"),
            zero_time
        );
    }

    #[test]
    fn stale_snapshot_fence_cannot_hide_a_prefix_rollback() {
        let root = stale_mid_journal_fixture();
        let error = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect_err("stale snapshot fence must fail");
        assert!(
            matches!(
                error,
                FrozenProtocol18SourceError::Cell(
                    PersistenceError::HistoricalFenceSnapshotMismatch { .. }
                )
            ),
            "unexpected stale-fence error: {error:?}"
        );
    }

    #[test]
    fn non_sleeping_and_frontier_substitution_fail_closed() {
        let root = frozen_fixture(true);
        let directory_path = root.path().join("cell-directory.json");
        let canonical_directory = fs::read(&directory_path).expect("directory reads");
        rewrite_pretty(&directory_path, |directory| {
            let assignments = directory["assignments"]
                .as_object_mut()
                .expect("assignments object");
            let first = assignments.values_mut().next().expect("first assignment");
            first["state"] = serde_json::json!("assigned");
            first["holder_id"] = serde_json::json!("unexpected-holder");
        });
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        fs::write(&directory_path, canonical_directory).expect("directory restores");

        let canonical_directory = fs::read(&directory_path).expect("directory rereads");
        rewrite_pretty(&directory_path, |directory| {
            let assignments = directory["assignments"]
                .as_object_mut()
                .expect("assignments object");
            let first = assignments.values_mut().next().expect("first assignment");
            first["authority_fencing_token"] = serde_json::json!(2);
            first["fencing_history"]["1"] = serde_json::json!(2);
        });
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        fs::write(&directory_path, canonical_directory).expect("directory restores again");

        let lifecycle_path = cell_root(root.path(), 0).join("cell-lifecycle.json");
        rewrite_pretty(&lifecycle_path, |lifecycle| {
            lifecycle["last_world_state_hash"] = serde_json::json!("2".repeat(64));
        });
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
    }

    #[test]
    fn swapped_cell_snapshot_and_unknown_field_reject() {
        let root = frozen_fixture(false);
        let first_path = cell_root(root.path(), 0).join("world-snapshot.json");
        let second_path = cell_root(root.path(), 1).join("world-snapshot.json");
        let first = fs::read(&first_path).expect("first snapshot reads");
        let second = fs::read(&second_path).expect("second snapshot reads");
        fs::write(&first_path, &second).expect("second snapshot substitutes first");
        fs::write(&second_path, &first).expect("first snapshot substitutes second");
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
        fs::write(&first_path, &first).expect("first snapshot restores");
        fs::write(&second_path, &second).expect("second snapshot restores");

        rewrite_pretty(&first_path, |snapshot| {
            snapshot["unknown_migration_claim"] = serde_json::json!(true);
        });
        assert!(ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err());
    }
}
