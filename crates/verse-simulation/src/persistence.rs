// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::content;
use crate::event::CanonicalEvent;
use crate::model::{WORLD_SCHEMA_VERSION, WorldState};

const MANIFEST_FILE: &str = "universe-manifest.json";
const SNAPSHOT_FILE: &str = "world-snapshot.json";
const JOURNAL_FILE: &str = "events.ndjson";
const LOCK_FILE: &str = "writer.lock";

#[derive(Debug, Error)]
pub enum PersistenceError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("another simulation worker already owns {0}")]
    WriterAlreadyActive(PathBuf),
    #[error("invalid JSON at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("world seed mismatch: stored {stored}, requested {requested}")]
    SeedMismatch { stored: u64, requested: u64 },
    #[error("content manifest mismatch: stored {stored}, runtime {runtime}")]
    ContentManifestMismatch { stored: String, runtime: String },
    #[error("snapshot schema {found} is unsupported; expected {expected}")]
    SnapshotSchema { found: u32, expected: u32 },
    #[error("snapshot content hash is invalid")]
    SnapshotHashMismatch,
    #[error("journal line {line} is corrupt: {message}")]
    CorruptJournal { line: usize, message: String },
    #[error("writer fencing token changed from {expected} to {found}")]
    FencingTokenChanged { expected: u64, found: u64 },
    #[error("journal replay rejected event {event_sequence}: {message}")]
    Replay {
        event_sequence: u64,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UniverseManifest {
    schema_version: u32,
    universe_id: String,
    world_seed: u64,
    content_manifest_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnapshotDocument {
    schema_version: u32,
    state_hash: String,
    event_sequence: u64,
    last_event_hash: String,
    state: WorldState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LeaseMetadata {
    fencing_token: u64,
    worker_id: String,
    acquired_at_unix_ms: u64,
}

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    lock_file: File,
    journal_file: File,
    fencing_token: u64,
    world_seed: u64,
}

impl Store {
    pub fn open(root: impl AsRef<Path>, requested_seed: u64) -> Result<Self, PersistenceError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|source| io_error(&root, source))?;

        let lock_path = root.join(LOCK_FILE);
        let mut lock_file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|source| io_error(&lock_path, source))?;
        FileExt::try_lock_exclusive(&lock_file).map_err(|source| {
            if source.kind() == std::io::ErrorKind::WouldBlock {
                PersistenceError::WriterAlreadyActive(root.clone())
            } else {
                io_error(&lock_path, source)
            }
        })?;

        let previous_token =
            read_lease(&mut lock_file, &lock_path)?.map_or(0, |lease| lease.fencing_token);
        let fencing_token = previous_token
            .checked_add(1)
            .unwrap_or(previous_token.saturating_add(1));
        let lease = LeaseMetadata {
            fencing_token,
            worker_id: Uuid::new_v4().to_string(),
            acquired_at_unix_ms: unix_millis(),
        };
        write_json_to_file(&mut lock_file, &lock_path, &lease)?;

        let manifest_path = root.join(MANIFEST_FILE);
        let world_seed = if manifest_path.exists() {
            let manifest: UniverseManifest = read_json(&manifest_path)?;
            if manifest.world_seed != requested_seed {
                return Err(PersistenceError::SeedMismatch {
                    stored: manifest.world_seed,
                    requested: requested_seed,
                });
            }
            let runtime_content = &content::manifest().manifest_version;
            if manifest.content_manifest_version != *runtime_content {
                return Err(PersistenceError::ContentManifestMismatch {
                    stored: manifest.content_manifest_version,
                    runtime: runtime_content.clone(),
                });
            }
            manifest.world_seed
        } else {
            let manifest = UniverseManifest {
                schema_version: 1,
                universe_id: "the-verse-local".into(),
                world_seed: requested_seed,
                content_manifest_version: content::manifest().manifest_version.clone(),
            };
            write_json_atomic(&manifest_path, &manifest)?;
            requested_seed
        };

        let journal_path = root.join(JOURNAL_FILE);
        let journal_file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&journal_path)
            .map_err(|source| io_error(&journal_path, source))?;

        Ok(Self {
            root,
            lock_file,
            journal_file,
            fencing_token,
            world_seed,
        })
    }

    pub const fn fencing_token(&self) -> u64 {
        self.fencing_token
    }

    pub fn load_world(&mut self) -> Result<WorldState, PersistenceError> {
        let snapshot_path = self.root.join(SNAPSHOT_FILE);
        let mut state = if snapshot_path.exists() {
            let snapshot: SnapshotDocument = read_json(&snapshot_path)?;
            if snapshot.schema_version != WORLD_SCHEMA_VERSION {
                return Err(PersistenceError::SnapshotSchema {
                    found: snapshot.schema_version,
                    expected: WORLD_SCHEMA_VERSION,
                });
            }
            if snapshot.state_hash != snapshot.state.state_hash()
                || snapshot.event_sequence != snapshot.state.event_sequence
                || snapshot.last_event_hash != snapshot.state.last_event_hash
            {
                return Err(PersistenceError::SnapshotHashMismatch);
            }
            snapshot.state
        } else {
            WorldState::genesis(self.world_seed)
        };

        let journal_path = self.root.join(JOURNAL_FILE);
        let mut journal = String::new();
        File::open(&journal_path)
            .and_then(|mut file| file.read_to_string(&mut journal))
            .map_err(|source| io_error(&journal_path, source))?;

        for (index, line) in journal.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event: CanonicalEvent =
                serde_json::from_str(line).map_err(|source| PersistenceError::CorruptJournal {
                    line: index + 1,
                    message: source.to_string(),
                })?;
            if event.event_sequence <= state.event_sequence {
                continue;
            }
            state
                .apply_event(&event)
                .map_err(|source| PersistenceError::Replay {
                    event_sequence: event.event_sequence,
                    message: source.to_string(),
                })?;
        }
        Ok(state)
    }

    pub fn append_event(&mut self, event: &CanonicalEvent) -> Result<(), PersistenceError> {
        self.verify_fencing_token()?;
        let journal_path = self.root.join(JOURNAL_FILE);
        let bytes = serde_json::to_vec(event).map_err(|source| PersistenceError::Json {
            path: journal_path.clone(),
            source,
        })?;
        self.journal_file
            .write_all(&bytes)
            .and_then(|()| self.journal_file.write_all(b"\n"))
            .and_then(|()| self.journal_file.sync_data())
            .map_err(|source| io_error(&journal_path, source))
    }

    pub fn save_snapshot(&mut self, state: &WorldState) -> Result<(), PersistenceError> {
        self.verify_fencing_token()?;
        let snapshot = SnapshotDocument {
            schema_version: WORLD_SCHEMA_VERSION,
            state_hash: state.state_hash(),
            event_sequence: state.event_sequence,
            last_event_hash: state.last_event_hash.clone(),
            state: state.clone(),
        };
        write_json_atomic(&self.root.join(SNAPSHOT_FILE), &snapshot)
    }

    fn verify_fencing_token(&mut self) -> Result<(), PersistenceError> {
        let lock_path = self.root.join(LOCK_FILE);
        let found =
            read_lease(&mut self.lock_file, &lock_path)?.map_or(0, |lease| lease.fencing_token);
        if found == self.fencing_token {
            Ok(())
        } else {
            Err(PersistenceError::FencingTokenChanged {
                expected: self.fencing_token,
                found,
            })
        }
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.lock_file);
    }
}

fn read_lease(file: &mut File, path: &Path) -> Result<Option<LeaseMetadata>, PersistenceError> {
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error(path, source))?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|source| io_error(path, source))?;
    if text.trim().is_empty() {
        Ok(None)
    } else {
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| PersistenceError::Json {
                path: path.to_path_buf(),
                source,
            })
    }
}

fn write_json_to_file<T: Serialize>(
    file: &mut File,
    path: &Path,
    value: &T,
) -> Result<(), PersistenceError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|source| PersistenceError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.set_len(0))
        .and_then(|()| file.write_all(&bytes))
        .and_then(|()| file.sync_data())
        .map_err(|source| io_error(path, source))
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, PersistenceError> {
    let bytes = fs::read(path).map_err(|source| io_error(path, source))?;
    serde_json::from_slice(&bytes).map_err(|source| PersistenceError::Json {
        path: path.to_path_buf(),
        source,
    })
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), PersistenceError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|source| PersistenceError::Json {
        path: path.to_path_buf(),
        source,
    })?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let temp_path = parent.join(format!(
        ".{}.tmp-{}-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("snapshot"),
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
        fs::rename(&temp_path, path).map_err(|source| io_error(path, source))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn io_error(path: impl AsRef<Path>, source: std::io::Error) -> PersistenceError {
    PersistenceError::Io {
        path: path.as_ref().to_path_buf(),
        source,
    }
}

fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use tempfile::tempdir;
    use verse_protocol::{BlockKind, ClientMessage, IVec3, Vec3};

    use super::*;
    use crate::Runtime;

    #[test]
    fn second_writer_is_rejected_and_fencing_token_advances() {
        let directory = tempdir().expect("tempdir");
        let first = Store::open(directory.path(), 11).expect("first writer");
        assert!(matches!(
            Store::open(directory.path(), 11),
            Err(PersistenceError::WriterAlreadyActive(_))
        ));
        let first_token = first.fencing_token();
        drop(first);
        let second = Store::open(directory.path(), 11).expect("replacement writer");
        assert!(second.fencing_token() > first_token);
    }

    #[test]
    fn snapshot_and_journal_recover_identical_world_hash() {
        let directory = tempdir().expect("tempdir");
        let target;
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 19, 100).expect("runtime starts");
            target = runtime
                .state()
                .voxels
                .occupied
                .iter()
                .copied()
                .find(|coord| {
                    let dx = f64::from(coord.x) - runtime.state().player.position.x;
                    let dy = f64::from(coord.y) - runtime.state().player.position.y;
                    let dz = f64::from(coord.z) - runtime.state().player.position.z;
                    dx.mul_add(dx, dy.mul_add(dy, dz * dz)) <= 8.5 * 8.5
                })
                .expect("reachable voxel");
            runtime
                .execute(&ClientMessage::MineVoxel {
                    operation_id: "durable-mine".into(),
                    coordinate: target,
                })
                .expect("mine succeeds");
            expected_hash = runtime.state().state_hash();
        }

        let recovered = Runtime::open(directory.path(), 19, 100).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert!(!recovered.state().voxels.occupied.contains(&target));
    }

    #[test]
    fn construction_integrity_and_orientation_recover_exactly() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 29, 100).expect("runtime starts");
            for (index, position) in [
                Vec3::new(11.0, 3.5, 7.5),
                Vec3::new(10.5, 2.5, 5.0),
                Vec3::new(10.0, 1.0, 3.0),
            ]
            .into_iter()
            .enumerate()
            {
                runtime
                    .execute(&ClientMessage::MovePlayer {
                        operation_id: format!("recovery-move-{index}"),
                        position,
                    })
                    .expect("player approaches construction range");
            }
            runtime
                .execute(&ClientMessage::BuildBlock {
                    operation_id: "recovery-frame".into(),
                    grid_id: "grid-starter".into(),
                    coordinate: IVec3::new(0, 1, 0),
                    kind: BlockKind::Structural,
                    orientation: 2,
                })
                .expect("construction frame placed");
            let block_id = runtime.state().grids["grid-starter"]
                .block_at(IVec3::new(0, 1, 0))
                .expect("frame exists")
                .block_id
                .clone();
            runtime
                .execute(&ClientMessage::WeldBlock {
                    operation_id: "recovery-weld".into(),
                    grid_id: "grid-starter".into(),
                    block_id,
                })
                .expect("one weld stage accepted");
            runtime.persist_snapshot().expect("snapshot persists");
            expected_hash = runtime.state().state_hash();
        }

        let recovered = Runtime::open(directory.path(), 29, 100).expect("runtime recovers");
        let block = recovered.state().grids["grid-starter"]
            .block_at(IVec3::new(0, 1, 0))
            .expect("construction frame recovers");
        assert_eq!(block.orientation, 2);
        assert_eq!(block.health, 50);
        assert_eq!(block.max_health(), 100);
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn suit_environment_and_inventory_metrics_recover_exactly() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 37, 100).expect("runtime starts");
            runtime
                .execute(&ClientMessage::SetSuitMode {
                    operation_id: "persistent-suit-mode".into(),
                    helmet_closed: false,
                    jetpack_enabled: false,
                })
                .expect("suit mode accepted");
            runtime.persist_snapshot().expect("snapshot persists");
            expected_hash = runtime.state().state_hash();
        }

        let recovered = Runtime::open(directory.path(), 37, 100).expect("runtime recovers");
        let snapshot = recovered.snapshot();
        assert!(!snapshot.player.helmet_closed);
        assert!(!snapshot.player.jetpack_enabled);
        assert_eq!(snapshot.player.suit_oxygen_milli, 1_000);
        assert!(!snapshot.environment.breathable);
        assert!(snapshot.environment.altitude_m > 3_000.0);
        let suit = snapshot
            .inventories
            .iter()
            .find(|inventory| inventory.inventory_id == "inventory-player-local")
            .expect("suit inventory snapshot");
        assert_eq!(suit.used_liters, 528);
        assert_eq!(suit.mass_grams, 115_200);
        assert_eq!(recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn corrupt_journal_is_detected() {
        let directory = tempdir().expect("tempdir");
        {
            let _runtime = Runtime::open(directory.path(), 23, 100).expect("runtime starts");
        }
        let journal_path = directory.path().join(JOURNAL_FILE);
        OpenOptions::new()
            .append(true)
            .open(&journal_path)
            .expect("journal opens")
            .write_all(b"{not-json}\n")
            .expect("corruption written");
        assert!(matches!(
            Runtime::open(directory.path(), 23, 100),
            Err(crate::RuntimeError::Persistence(
                PersistenceError::CorruptJournal { .. }
            ))
        ));
    }

    #[test]
    fn tampered_snapshot_is_detected_before_replay() {
        let directory = tempdir().expect("tempdir");
        {
            let mut runtime = Runtime::open(directory.path(), 31, 100).expect("runtime starts");
            runtime.persist_snapshot().expect("snapshot persisted");
        }
        let snapshot_path = directory.path().join(SNAPSHOT_FILE);
        let mut snapshot: serde_json::Value =
            read_json(&snapshot_path).expect("snapshot JSON reads");
        snapshot["state"]["player"]["position"]["x"] = serde_json::json!(999.0);
        fs::write(
            &snapshot_path,
            serde_json::to_vec_pretty(&snapshot).expect("snapshot serializes"),
        )
        .expect("tampered snapshot writes");

        assert!(matches!(
            Runtime::open(directory.path(), 31, 100),
            Err(crate::RuntimeError::Persistence(
                PersistenceError::SnapshotHashMismatch
            ))
        ));
    }

    #[test]
    fn changed_content_manifest_is_rejected() {
        let directory = tempdir().expect("tempdir");
        drop(Store::open(directory.path(), 41).expect("store"));
        let manifest_path = directory.path().join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            read_json(&manifest_path).expect("manifest JSON reads");
        manifest["content_manifest_version"] = serde_json::json!("foreign-rules-v9");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
        )
        .expect("changed manifest writes");

        assert!(matches!(
            Store::open(directory.path(), 41),
            Err(PersistenceError::ContentManifestMismatch { .. })
        ));
    }

    #[test]
    fn seed_change_is_rejected_for_existing_universe() {
        let directory = tempdir().expect("tempdir");
        drop(Store::open(directory.path(), 5).expect("store"));
        assert!(matches!(
            Store::open(directory.path(), 6),
            Err(PersistenceError::SeedMismatch {
                stored: 5,
                requested: 6
            })
        ));
    }

    #[test]
    fn coordinate_type_remains_json_compatible() {
        let coordinate = IVec3::new(1, -2, 3);
        assert_eq!(
            serde_json::to_string(&coordinate).expect("coordinate serializes"),
            r#"{"x":1,"y":-2,"z":3}"#
        );
    }
}
