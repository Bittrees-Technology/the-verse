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

pub const CELL_DIRECTORY_SCHEMA_VERSION: u32 = 1;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellDirectoryDocument {
    schema_version: u32,
    universe_id: String,
    universe_manifest_hash: String,
    directory_revision: u64,
    assignments: BTreeMap<String, CellAssignmentRecord>,
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
    Ok(())
}

fn validate_holder(holder_id: &str) -> Result<(), CellDirectoryError> {
    if holder_id.is_empty()
        || holder_id.len() > 128
        || !holder_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(CellDirectoryError::InvalidDirectory(
            "assignment holder ID is not bounded canonical text".into(),
        ));
    }
    Ok(())
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
    use crate::{EVENT_SCHEMA_VERSION, WORLD_SCHEMA_VERSION, universe_manifest};

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
}
