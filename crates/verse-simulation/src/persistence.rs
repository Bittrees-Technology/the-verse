// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use verse_protocol::UniverseManifestSnapshot;

use crate::event::{CanonicalEvent, EVENT_SCHEMA_NAME, EVENT_SCHEMA_VERSION};
use crate::model::{WORLD_SCHEMA_VERSION, WorldState};
use crate::{celestial, content};

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
    #[error("runtime universe manifest is invalid: {0}")]
    InvalidRuntimeUniverseManifest(String),
    #[error("universe manifest mismatch: stored hash {stored_hash}, runtime hash {runtime_hash}")]
    UniverseManifestMismatch {
        stored_hash: String,
        runtime_hash: String,
    },
    #[error("world state does not match the opened universe manifest")]
    WorldUniverseBindingMismatch,
    #[error("event universe binding mismatch at {context}")]
    EventUniverseBindingMismatch { context: String },
    #[error("snapshot schema {found} is unsupported; expected {expected}")]
    SnapshotSchema { found: u32, expected: u32 },
    #[error("snapshot content hash is invalid")]
    SnapshotHashMismatch,
    #[error("snapshot player roster is invalid: {0}")]
    InvalidPlayerRoster(String),
    #[error("journal line {line} is corrupt: {message}")]
    CorruptJournal { line: usize, message: String },
    #[error(
        "journal line {line} uses event schema {found_name} v{found_version}; expected {expected_name} v{expected_version}"
    )]
    EventSchema {
        line: usize,
        found_name: String,
        found_version: u32,
        expected_name: &'static str,
        expected_version: u32,
    },
    #[error("writer fencing token changed from {expected} to {found}")]
    FencingTokenChanged { expected: u64, found: u64 },
    #[error("journal replay rejected event {event_sequence}: {message}")]
    Replay {
        event_sequence: u64,
        message: String,
    },
    #[cfg(test)]
    #[error("injected persistence failure at {0}")]
    InjectedFailure(&'static str),
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AppendFailpoint {
    BeforeWrite,
    AfterSync,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotDocument {
    schema_version: u32,
    state_hash: String,
    event_sequence: u64,
    last_event_hash: String,
    state: WorldState,
}

#[derive(Debug, Deserialize)]
struct SnapshotHeader {
    schema_version: u32,
}

#[derive(Debug, Deserialize)]
struct EventHeader {
    schema_name: String,
    schema_version: u32,
    event_sequence: u64,
    #[serde(default)]
    content_manifest_version: Option<String>,
    #[serde(default)]
    universe_id: Option<String>,
    #[serde(default)]
    universe_manifest_hash: Option<String>,
    #[serde(default)]
    celestial_registry_hash: Option<String>,
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
    universe_manifest: UniverseManifestSnapshot,
    #[cfg(test)]
    append_failpoint: Option<AppendFailpoint>,
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
        let runtime_manifest = celestial::universe_manifest(
            requested_seed,
            WORLD_SCHEMA_VERSION,
            EVENT_SCHEMA_VERSION,
        )
        .map_err(|source| PersistenceError::InvalidRuntimeUniverseManifest(source.to_string()))?;
        if manifest_path.exists() {
            let stored_value: serde_json::Value = read_json(&manifest_path)?;
            let stored: UniverseManifestSnapshot = serde_json::from_value(stored_value.clone())
                .map_err(|source| PersistenceError::Json {
                    path: manifest_path.clone(),
                    source,
                })?;
            let requested_seed_text = requested_seed.to_string();
            if stored.world_seed != requested_seed_text {
                let stored_seed = stored.world_seed.parse::<u64>().map_err(|_| {
                    PersistenceError::UniverseManifestMismatch {
                        stored_hash: stored.manifest_hash.clone(),
                        runtime_hash: runtime_manifest.manifest_hash.clone(),
                    }
                })?;
                return Err(PersistenceError::SeedMismatch {
                    stored: stored_seed,
                    requested: requested_seed,
                });
            }
            let runtime_content = &content::manifest().manifest_version;
            if stored.content_manifest_version != *runtime_content {
                return Err(PersistenceError::ContentManifestMismatch {
                    stored: stored.content_manifest_version,
                    runtime: runtime_content.clone(),
                });
            }
            let runtime_value = serde_json::to_value(&runtime_manifest)
                .expect("protocol universe manifest serializes");
            if stored != runtime_manifest || stored_value != runtime_value {
                return Err(PersistenceError::UniverseManifestMismatch {
                    stored_hash: stored.manifest_hash,
                    runtime_hash: runtime_manifest.manifest_hash.clone(),
                });
            }
        } else {
            write_json_atomic(&manifest_path, &runtime_manifest)?;
        }

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
            world_seed: requested_seed,
            universe_manifest: runtime_manifest,
            #[cfg(test)]
            append_failpoint: None,
        })
    }

    pub const fn fencing_token(&self) -> u64 {
        self.fencing_token
    }

    pub fn load_world(&mut self) -> Result<WorldState, PersistenceError> {
        let snapshot_path = self.root.join(SNAPSHOT_FILE);
        let mut state = if snapshot_path.exists() {
            let header: SnapshotHeader = read_json(&snapshot_path)?;
            if header.schema_version != WORLD_SCHEMA_VERSION {
                return Err(PersistenceError::SnapshotSchema {
                    found: header.schema_version,
                    expected: WORLD_SCHEMA_VERSION,
                });
            }
            let mut snapshot: SnapshotDocument = read_json(&snapshot_path)?;
            snapshot
                .state
                .hydrate_spatial_poses()
                .map_err(PersistenceError::InvalidPlayerRoster)?;
            if !self.world_binding_matches(&snapshot.state) {
                return Err(PersistenceError::WorldUniverseBindingMismatch);
            }
            snapshot
                .state
                .validate_player_roster()
                .map_err(PersistenceError::InvalidPlayerRoster)?;
            if snapshot.state_hash != snapshot.state.state_hash()
                || snapshot.event_sequence != snapshot.state.event_sequence
                || snapshot.last_event_hash != snapshot.state.last_event_hash
            {
                return Err(PersistenceError::SnapshotHashMismatch);
            }
            snapshot.state
        } else {
            let state = WorldState::genesis(self.world_seed);
            if !self.world_binding_matches(&state) {
                return Err(PersistenceError::WorldUniverseBindingMismatch);
            }
            state
        };

        let journal_path = self.root.join(JOURNAL_FILE);
        let mut journal_bytes = Vec::new();
        File::open(&journal_path)
            .and_then(|mut file| file.read_to_end(&mut journal_bytes))
            .map_err(|source| io_error(&journal_path, source))?;
        let committed_length = if journal_bytes.last() == Some(&b'\n') {
            journal_bytes.len()
        } else {
            journal_bytes
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map_or(0, |position| position + 1)
        };
        if committed_length != journal_bytes.len() {
            self.journal_file
                .set_len(u64::try_from(committed_length).expect("journal length fits u64"))
                .and_then(|()| self.journal_file.sync_data())
                .map_err(|source| io_error(&journal_path, source))?;
            journal_bytes.truncate(committed_length);
        }
        let journal = String::from_utf8(journal_bytes).map_err(|source| {
            let valid_up_to = source.utf8_error().valid_up_to();
            let line = std::str::from_utf8(&source.as_bytes()[..valid_up_to])
                .expect("String::from_utf8 reports a valid UTF-8 prefix")
                .matches('\n')
                .count()
                + 1;
            PersistenceError::CorruptJournal {
                line,
                message: source.to_string(),
            }
        })?;

        for (index, line) in journal.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let header: EventHeader =
                serde_json::from_str(line).map_err(|source| PersistenceError::CorruptJournal {
                    line: index + 1,
                    message: source.to_string(),
                })?;
            if header.schema_name != EVENT_SCHEMA_NAME
                || header.schema_version != EVENT_SCHEMA_VERSION
            {
                return Err(PersistenceError::EventSchema {
                    line: index + 1,
                    found_name: header.schema_name,
                    found_version: header.schema_version,
                    expected_name: EVENT_SCHEMA_NAME,
                    expected_version: EVENT_SCHEMA_VERSION,
                });
            }
            if !self.event_header_binding_matches(&header) {
                return Err(PersistenceError::EventUniverseBindingMismatch {
                    context: format!("journal line {}", index + 1),
                });
            }
            if header.event_sequence <= state.event_sequence {
                continue;
            }
            let event: CanonicalEvent =
                serde_json::from_str(line).map_err(|source| PersistenceError::CorruptJournal {
                    line: index + 1,
                    message: source.to_string(),
                })?;
            state
                .apply_event(&event)
                .map_err(|source| PersistenceError::Replay {
                    event_sequence: event.event_sequence,
                    message: source.to_string(),
                })?;
        }
        state
            .validate_player_roster()
            .map_err(PersistenceError::InvalidPlayerRoster)?;
        if !self.world_binding_matches(&state) {
            return Err(PersistenceError::WorldUniverseBindingMismatch);
        }
        Ok(state)
    }

    pub fn append_event(&mut self, event: &CanonicalEvent) -> Result<(), PersistenceError> {
        self.verify_fencing_token()?;
        if !self.event_binding_matches(event) {
            return Err(PersistenceError::EventUniverseBindingMismatch {
                context: "append".into(),
            });
        }
        #[cfg(test)]
        if self.consume_append_failpoint(AppendFailpoint::BeforeWrite) {
            return Err(PersistenceError::InjectedFailure("before journal write"));
        }
        let journal_path = self.root.join(JOURNAL_FILE);
        let bytes = serde_json::to_vec(event).map_err(|source| PersistenceError::Json {
            path: journal_path.clone(),
            source,
        })?;
        self.journal_file
            .write_all(&bytes)
            .and_then(|()| self.journal_file.write_all(b"\n"))
            .and_then(|()| self.journal_file.sync_data())
            .map_err(|source| io_error(&journal_path, source))?;
        #[cfg(test)]
        if self.consume_append_failpoint(AppendFailpoint::AfterSync) {
            return Err(PersistenceError::InjectedFailure("after journal sync"));
        }
        Ok(())
    }

    pub fn save_snapshot(&mut self, state: &WorldState) -> Result<(), PersistenceError> {
        self.verify_fencing_token()?;
        if !self.world_binding_matches(state) {
            return Err(PersistenceError::WorldUniverseBindingMismatch);
        }
        state
            .validate_player_roster()
            .map_err(PersistenceError::InvalidPlayerRoster)?;
        let snapshot = SnapshotDocument {
            schema_version: WORLD_SCHEMA_VERSION,
            state_hash: state.state_hash(),
            event_sequence: state.event_sequence,
            last_event_hash: state.last_event_hash.clone(),
            state: state.clone(),
        };
        write_json_atomic(&self.root.join(SNAPSHOT_FILE), &snapshot)
    }

    fn world_binding_matches(&self, state: &WorldState) -> bool {
        state.world_seed == self.world_seed
            && state.universe_id == self.universe_manifest.universe_id
            && state.content_manifest_version == self.universe_manifest.content_manifest_version
            && state.universe_manifest_hash == self.universe_manifest.manifest_hash
            && state.celestial_registry_hash == self.universe_manifest.celestial_registry_hash
    }

    fn event_header_binding_matches(&self, header: &EventHeader) -> bool {
        header.content_manifest_version.as_deref()
            == Some(self.universe_manifest.content_manifest_version.as_str())
            && header.universe_id.as_deref() == Some(self.universe_manifest.universe_id.as_str())
            && header.universe_manifest_hash.as_deref()
                == Some(self.universe_manifest.manifest_hash.as_str())
            && header.celestial_registry_hash.as_deref()
                == Some(self.universe_manifest.celestial_registry_hash.as_str())
    }

    fn event_binding_matches(&self, event: &CanonicalEvent) -> bool {
        event.content_manifest_version == self.universe_manifest.content_manifest_version
            && event.universe_id == self.universe_manifest.universe_id
            && event.universe_manifest_hash == self.universe_manifest.manifest_hash
            && event.celestial_registry_hash == self.universe_manifest.celestial_registry_hash
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

    #[cfg(test)]
    pub(crate) fn set_append_failpoint(&mut self, failpoint: AppendFailpoint) {
        self.append_failpoint = Some(failpoint);
    }

    #[cfg(test)]
    fn consume_append_failpoint(&mut self, failpoint: AppendFailpoint) -> bool {
        if self.append_failpoint == Some(failpoint) {
            self.append_failpoint = None;
            true
        } else {
            false
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
    use crate::event::EventPayload;
    use crate::model::{STARTER_GRID_ID, WorldState};

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
    fn persisted_universe_manifest_matches_the_protocol_snapshot_exactly() {
        let directory = tempdir().expect("tempdir");
        drop(Store::open(directory.path(), 13).expect("store opens"));
        let stored: UniverseManifestSnapshot =
            read_json(&directory.path().join(MANIFEST_FILE)).expect("manifest reads");
        let expected = celestial::universe_manifest(13, WORLD_SCHEMA_VERSION, EVENT_SCHEMA_VERSION)
            .expect("runtime universe manifest is valid");
        assert_eq!(stored, expected);
        assert_eq!(stored.schema_version, 2);
        assert_eq!(stored.event_schema_version, 14);
    }

    #[test]
    fn store_open_rejects_any_exact_manifest_or_hash_substitution() {
        for (field, replacement) in [
            ("celestial_registry_hash", serde_json::json!("0".repeat(64))),
            ("manifest_hash", serde_json::json!("0".repeat(64))),
            (
                "frontier_policy_version",
                serde_json::json!("tampered-frontier-policy"),
            ),
            ("unexpected_field", serde_json::json!("must fail closed")),
        ] {
            let directory = tempdir().expect("tempdir");
            drop(Store::open(directory.path(), 17).expect("store opens"));
            let manifest_path = directory.path().join(MANIFEST_FILE);
            let mut manifest: serde_json::Value =
                read_json(&manifest_path).expect("manifest JSON reads");
            manifest[field] = replacement;
            fs::write(
                &manifest_path,
                serde_json::to_vec_pretty(&manifest).expect("tampered manifest serializes"),
            )
            .expect("tampered manifest writes");

            let result = Store::open(directory.path(), 17);
            if field == "unexpected_field" {
                assert!(matches!(
                    result,
                    Err(PersistenceError::Json { ref path, .. }) if path == &manifest_path
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(PersistenceError::UniverseManifestMismatch { .. })
                ));
            }
        }
    }

    #[test]
    fn legacy_private_universe_manifest_is_rejected_fail_closed() {
        let directory = tempdir().expect("tempdir");
        drop(Store::open(directory.path(), 17).expect("store opens"));
        let manifest_path = directory.path().join(MANIFEST_FILE);
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 1,
                "universe_id": "the-verse-local",
                "world_seed": 17,
                "content_manifest_version": "p1.4.0",
            }))
            .expect("legacy manifest serializes"),
        )
        .expect("legacy manifest writes");

        assert!(matches!(
            Store::open(directory.path(), 17),
            Err(PersistenceError::Json { ref path, .. }) if path == &manifest_path
        ));
    }

    #[test]
    fn append_and_replay_reject_wrong_event_universe_bindings() {
        let directory = tempdir().expect("tempdir");
        let mut store = Store::open(directory.path(), 18).expect("store opens");
        let manifest = store.universe_manifest.clone();
        let mut event = CanonicalEvent::new(
            1,
            manifest.content_manifest_version.clone(),
            manifest.manifest_hash.clone(),
            manifest.celestial_registry_hash.clone(),
            manifest.universe_id.clone(),
            "cell-origin",
            store.fencing_token(),
            None,
            "system",
            None,
            None,
            None,
            "",
            EventPayload::SuitOxygenChanged {
                player_id: "player-local".into(),
                previous_oxygen_milli: 1_000,
                new_oxygen_milli: 995,
            },
        );
        event.celestial_registry_hash = "0".repeat(64);
        event.event_hash = event.calculate_hash();
        assert!(matches!(
            store.append_event(&event),
            Err(PersistenceError::EventUniverseBindingMismatch { ref context })
                if context == "append"
        ));
        drop(store);

        fs::write(
            directory.path().join(JOURNAL_FILE),
            format!(
                "{}\n",
                serde_json::to_string(&event).expect("event serializes")
            ),
        )
        .expect("mismatched journal writes");
        let mut reopened = Store::open(directory.path(), 18).expect("manifest remains valid");
        assert!(matches!(
            reopened.load_world(),
            Err(PersistenceError::EventUniverseBindingMismatch { ref context })
                if context == "journal line 1"
        ));
    }

    #[test]
    fn snapshot_recovery_rejects_wrong_registry_binding_before_hash_replay() {
        let directory = tempdir().expect("tempdir");
        {
            let mut store = Store::open(directory.path(), 21).expect("store opens");
            let world = WorldState::genesis(21);
            store.save_snapshot(&world).expect("snapshot persists");
        }
        let snapshot_path = directory.path().join(SNAPSHOT_FILE);
        let mut snapshot: SnapshotDocument =
            read_json(&snapshot_path).expect("snapshot document reads");
        snapshot.state.celestial_registry_hash = "0".repeat(64);
        snapshot.state_hash = snapshot.state.state_hash();
        write_json_atomic(&snapshot_path, &snapshot).expect("tampered snapshot writes");

        let mut reopened = Store::open(directory.path(), 21).expect("manifest remains valid");
        assert!(matches!(
            reopened.load_world(),
            Err(PersistenceError::WorldUniverseBindingMismatch)
        ));
    }

    #[test]
    fn snapshot_and_journal_recover_identical_world_hash() {
        let directory = tempdir().expect("tempdir");
        let target;
        let expected_hash;
        let expected_player_address;
        let expected_player_position;
        let expected_grid_addresses;
        {
            let mut runtime = Runtime::open(directory.path(), 19, 100).expect("runtime starts");
            target = runtime
                .state()
                .voxels
                .occupied
                .iter()
                .copied()
                .max_by_key(|coordinate| coordinate.z)
                .expect("asteroid has a visible positive-Z surface voxel");
            runtime.aim_player_for_test(
                Vec3::new(
                    f64::from(target.x),
                    f64::from(target.y),
                    f64::from(target.z),
                ),
                Vec3::new(0.0, 0.0, 1.0),
            );
            runtime
                .persist_snapshot()
                .expect("aimed mining baseline persists");
            runtime
                .execute_next_for_fixture(&ClientMessage::MineVoxel {
                    operation_sequence: 0,
                    operation_id: "durable-mine".into(),
                    coordinate: target,
                })
                .expect("mine succeeds");
            runtime
                .advance(17)
                .expect("one exact-address physics outcome commits");
            expected_hash = runtime.state().state_hash();
            expected_player_address = runtime.state().player.address.clone();
            expected_player_position = runtime.state().player.position;
            expected_grid_addresses = runtime
                .state()
                .grids
                .iter()
                .map(|(grid_id, grid)| (grid_id.clone(), grid.address.clone()))
                .collect::<Vec<_>>();
        }

        let recovered = Runtime::open(directory.path(), 19, 100).expect("runtime recovers");
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert_eq!(recovered.state().player.address, expected_player_address);
        assert_eq!(recovered.state().player.position, expected_player_position);
        assert_eq!(
            recovered
                .state()
                .grids
                .iter()
                .map(|(grid_id, grid)| (grid_id.clone(), grid.address.clone()))
                .collect::<Vec<_>>(),
            expected_grid_addresses
        );
        assert!(!recovered.state().voxels.occupied.contains(&target));
    }

    #[test]
    fn construction_integrity_and_orientation_recover_exactly() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 29, 100).expect("runtime starts");
            let core = runtime.state().grids[STARTER_GRID_ID].world_position(IVec3::ZERO);
            runtime.aim_player_for_test(core, Vec3::new(0.0, 1.0, 0.0));
            runtime
                .persist_snapshot()
                .expect("aimed build baseline persists");
            runtime
                .execute_next_for_fixture(&ClientMessage::BuildBlock {
                    operation_sequence: 0,
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
                .execute_next_for_fixture(&ClientMessage::WeldBlock {
                    operation_sequence: 0,
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
        assert!(!block.construction_complete);
        assert_eq!(recovered.state().state_hash(), expected_hash);
        assert!(recovered.state().conservation().valid);
    }

    #[test]
    fn completed_construction_and_career_recover_from_journal_and_snapshot() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 30, 100).expect("runtime starts");
            let core = runtime.state().grids[STARTER_GRID_ID].world_position(IVec3::ZERO);
            runtime.aim_player_for_test(core, Vec3::new(0.0, 1.0, 0.0));
            runtime
                .persist_snapshot()
                .expect("durable aimed baseline persists");
            runtime
                .execute_next_for_fixture(&ClientMessage::BuildBlock {
                    operation_sequence: 0,
                    operation_id: "completed-recovery-frame".into(),
                    grid_id: STARTER_GRID_ID.into(),
                    coordinate: IVec3::new(0, 1, 0),
                    kind: BlockKind::Structural,
                    orientation: 3,
                })
                .expect("construction frame placed");
            let block_id = runtime.state().grids[STARTER_GRID_ID]
                .block_at(IVec3::new(0, 1, 0))
                .expect("frame exists")
                .block_id
                .clone();
            for stage in 0..3 {
                runtime
                    .execute_next_for_fixture(&ClientMessage::WeldBlock {
                        operation_sequence: 0,
                        operation_id: format!("completed-recovery-weld-{stage}"),
                        grid_id: STARTER_GRID_ID.into(),
                        block_id: block_id.clone(),
                    })
                    .expect("weld accepted");
            }
            assert!(runtime.state().grids[STARTER_GRID_ID].blocks[&block_id].construction_complete);
            assert_eq!(runtime.state().player.career.blocks_built, 1);
            assert_eq!(runtime.state().player.experience, 25);
            expected_hash = runtime.state().state_hash();
        }

        {
            let mut journal_recovered =
                Runtime::open(directory.path(), 30, 100).expect("journal recovers");
            let block = journal_recovered.state().grids[STARTER_GRID_ID]
                .block_at(IVec3::new(0, 1, 0))
                .expect("completed block recovers from journal");
            assert!(block.construction_complete);
            assert_eq!(journal_recovered.state().player.career.blocks_built, 1);
            assert_eq!(journal_recovered.state().state_hash(), expected_hash);
            journal_recovered
                .persist_snapshot()
                .expect("completed state snapshot persists");
        }

        let snapshot_recovered =
            Runtime::open(directory.path(), 30, 100).expect("snapshot recovers");
        let block = snapshot_recovered.state().grids[STARTER_GRID_ID]
            .block_at(IVec3::new(0, 1, 0))
            .expect("completed block recovers from snapshot");
        assert!(block.construction_complete);
        assert_eq!(snapshot_recovered.state().player.career.blocks_built, 1);
        assert_eq!(snapshot_recovered.state().state_hash(), expected_hash);
    }

    #[test]
    fn old_snapshot_schema_is_rejected_before_new_fields_are_deserialized() {
        let directory = tempdir().expect("tempdir");
        {
            let mut runtime = Runtime::open(directory.path(), 32, 100).expect("runtime starts");
            runtime.persist_snapshot().expect("snapshot persists");
        }
        let snapshot_path = directory.path().join(SNAPSHOT_FILE);
        let mut snapshot: serde_json::Value = read_json(&snapshot_path).expect("snapshot reads");
        snapshot["schema_version"] = serde_json::json!(WORLD_SCHEMA_VERSION - 1);
        snapshot["state"]
            .as_object_mut()
            .expect("state is an object")
            .remove("active_contact_pairs");
        for grid in snapshot["state"]["grids"]
            .as_object_mut()
            .expect("grids are an object")
            .values_mut()
        {
            for block in grid["blocks"]
                .as_object_mut()
                .expect("blocks are an object")
                .values_mut()
            {
                block
                    .as_object_mut()
                    .expect("block is an object")
                    .remove("construction_complete");
            }
        }
        fs::write(
            &snapshot_path,
            serde_json::to_vec_pretty(&snapshot).expect("old snapshot serializes"),
        )
        .expect("old snapshot fixture writes");

        assert!(matches!(
            Runtime::open(directory.path(), 32, 100),
            Err(crate::RuntimeError::Persistence(
                PersistenceError::SnapshotSchema {
                    found,
                    expected: WORLD_SCHEMA_VERSION,
                }
            )) if found == WORLD_SCHEMA_VERSION - 1
        ));
    }

    #[test]
    fn old_event_schema_is_rejected_before_new_payload_fields_are_deserialized() {
        let directory = tempdir().expect("tempdir");
        {
            let _runtime = Runtime::open(directory.path(), 33, 100).expect("runtime starts");
        }
        let state = WorldState::genesis(33);
        let max_health = state.grids[STARTER_GRID_ID].blocks["block-core"].max_health();
        let event = state.prepare_system_event(EventPayload::BlockWelded {
            grid_id: STARTER_GRID_ID.into(),
            block_id: "block-core".into(),
            previous_health: max_health - 1,
            new_health: max_health,
            max_health,
            completed_construction: false,
        });
        let mut fixture = serde_json::to_value(event).expect("event serializes");
        fixture["schema_version"] = serde_json::json!(EVENT_SCHEMA_VERSION - 1);
        fixture["payload"]
            .as_object_mut()
            .expect("payload is an object")
            .remove("completed_construction");
        fs::write(
            directory.path().join(JOURNAL_FILE),
            format!(
                "{}\n",
                serde_json::to_string(&fixture).expect("old event serializes")
            ),
        )
        .expect("old event fixture writes");

        assert!(matches!(
            Runtime::open(directory.path(), 33, 100),
            Err(crate::RuntimeError::Persistence(
                PersistenceError::EventSchema {
                    found_version,
                    expected_version: EVENT_SCHEMA_VERSION,
                    ..
                }
            )) if found_version == EVENT_SCHEMA_VERSION - 1
        ));
    }

    #[test]
    fn suit_environment_and_inventory_metrics_recover_exactly() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 37, 100).expect("runtime starts");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                    operation_sequence: 0,
                    operation_id: "persistent-suit-mode".into(),
                    helmet_closed: false,
                    jetpack_enabled: false,
                    magnetic_boots_enabled: false,
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
    fn unterminated_final_journal_record_recovers_prior_state_and_is_truncated() {
        let directory = tempdir().expect("tempdir");
        let expected_hash;
        {
            let mut runtime = Runtime::open(directory.path(), 47, 100).expect("runtime starts");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                    operation_sequence: 0,
                    operation_id: "committed-before-torn-tail".into(),
                    movement_epoch: 1,
                    input_sequence: 1,
                    linear_input: Vec3::new(1.0, 0.0, 0.0),
                    angular_input: Vec3::ZERO,
                    boost: false,
                    jump: false,
                    dampeners: true,
                })
                .expect("committed character control");
            expected_hash = runtime.state().state_hash();
        }
        let journal_path = directory.path().join(JOURNAL_FILE);
        OpenOptions::new()
            .append(true)
            .open(&journal_path)
            .expect("journal opens")
            .write_all(br#"{"schema_name":"verse.world_event""#)
            .expect("torn tail written");

        {
            let mut recovered =
                Runtime::open(directory.path(), 47, 100).expect("prior state recovers");
            assert_eq!(recovered.state().state_hash(), expected_hash);
            recovered
                .execute_next_for_fixture(&ClientMessage::SetPlayerControl {
                    operation_sequence: 0,
                    operation_id: "committed-after-torn-tail".into(),
                    movement_epoch: 1,
                    input_sequence: 2,
                    linear_input: Vec3::ZERO,
                    angular_input: Vec3::ZERO,
                    boost: false,
                    jump: false,
                    dampeners: true,
                })
                .expect("journal remains appendable after truncation");
        }

        let journal = fs::read(&journal_path).expect("journal reads");
        assert_eq!(journal.last(), Some(&b'\n'));
        let journal_text = String::from_utf8(journal).expect("journal remains UTF-8");
        assert_eq!(journal_text.lines().count(), 2);
        assert!(
            journal_text
                .lines()
                .all(|line| { serde_json::from_str::<CanonicalEvent>(line).is_ok() })
        );
        let recovered = Runtime::open(directory.path(), 47, 100).expect("second recovery works");
        assert_eq!(recovered.state().event_sequence, 2);
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
        let local_x = snapshot["state"]["players"]["by_id"]["player-local"]["address"]["local_um"]
            ["x"]
            .as_i64()
            .expect("player local x is an integer");
        snapshot["state"]["players"]["by_id"]["player-local"]["address"]["local_um"]["x"] =
            serde_json::json!(local_x + 1);
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
    fn malformed_player_roster_is_rejected_before_hashing() {
        let directory = tempdir().expect("tempdir");
        {
            let mut runtime = Runtime::open(directory.path(), 48, 100).expect("runtime starts");
            runtime.persist_snapshot().expect("snapshot persisted");
        }
        let snapshot_path = directory.path().join(SNAPSHOT_FILE);
        let mut snapshot: serde_json::Value = read_json(&snapshot_path).expect("snapshot reads");
        snapshot["state"]["players"]["primary_player_id"] = serde_json::json!("missing-player");
        fs::write(
            &snapshot_path,
            serde_json::to_vec_pretty(&snapshot).expect("snapshot serializes"),
        )
        .expect("malformed roster writes");

        assert!(matches!(
            Runtime::open(directory.path(), 48, 100),
            Err(crate::RuntimeError::Persistence(
                PersistenceError::InvalidPlayerRoster(_)
            ))
        ));
    }

    #[test]
    fn incompatible_content_manifests_are_rejected_before_replay() {
        let runtime_version = content::manifest().manifest_version.clone();
        for stored_version in ["p0.8.0", "p0.9.0", "p0.10.0"] {
            let directory = tempdir().expect("tempdir");
            drop(Store::open(directory.path(), 41).expect("store"));
            let manifest_path = directory.path().join(MANIFEST_FILE);
            let mut manifest: serde_json::Value =
                read_json(&manifest_path).expect("manifest JSON reads");
            manifest["content_manifest_version"] = serde_json::json!(stored_version);
            fs::write(
                &manifest_path,
                serde_json::to_vec_pretty(&manifest).expect("manifest serializes"),
            )
            .expect("changed manifest writes");

            assert!(matches!(
                Store::open(directory.path(), 41),
                Err(PersistenceError::ContentManifestMismatch { stored, runtime })
                    if stored == stored_version && runtime == runtime_version
            ));
        }
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
