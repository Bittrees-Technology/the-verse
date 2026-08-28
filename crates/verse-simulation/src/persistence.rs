// SPDX-License-Identifier: AGPL-3.0-or-later

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use verse_protocol::{CellKeyV1, UniverseManifestSnapshot};

use crate::event::{
    CanonicalEvent, EVENT_SCHEMA_NAME, EVENT_SCHEMA_VERSION, ProductionScheduleOccurrence,
};
use crate::model::{WORLD_SCHEMA_VERSION, WorldState};
use crate::{celestial, content};

const MANIFEST_FILE: &str = "universe-manifest.json";
const SNAPSHOT_FILE: &str = "world-snapshot.json";
const JOURNAL_FILE: &str = "events.ndjson";
const LOCK_FILE: &str = "writer.lock";
const LIFECYCLE_FILE: &str = "cell-lifecycle.json";
pub const LIFECYCLE_CONTROL_SCHEMA_VERSION: u32 = verse_protocol::LIFECYCLE_CONTROL_SCHEMA_VERSION;
pub const LEASE_DURATION_MILLIS: u64 = 15_000;
pub const LEASE_RENEWAL_INTERVAL_MILLIS: u64 = 5_000;
pub const LEASE_WRITE_SAFETY_MARGIN_MILLIS: u64 = 5_000;
pub const TRUSTED_CLOCK_ROLLBACK_TOLERANCE_MILLIS: u64 = 1_000;

pub trait TrustedClock: std::fmt::Debug + Send + Sync {
    fn now_unix_ms(&self) -> Result<u64, PersistenceError>;
}

#[derive(Debug, Default)]
pub struct SystemTrustedClock;

impl TrustedClock for SystemTrustedClock {
    fn now_unix_ms(&self) -> Result<u64, PersistenceError> {
        unix_millis()
    }
}

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
    #[error("cell identity is invalid: {0}")]
    InvalidCellIdentity(String),
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
    #[error("writer fencing token is exhausted")]
    FencingTokenExhausted,
    #[error("writer lease metadata no longer identifies the current holder")]
    LeaseOwnershipChanged,
    #[error("durable lifecycle control is missing for an existing world")]
    MissingLifecycleControl,
    #[error("durable lifecycle control is invalid: {0}")]
    InvalidLifecycleControl(String),
    #[error("cell store has released mutation authority")]
    StoreReleased,
    #[error("writer lease expired at {expires_at_unix_ms}; trusted time is {now_unix_ms}")]
    LeaseExpired {
        expires_at_unix_ms: u64,
        now_unix_ms: u64,
    },
    #[error(
        "writer lease has only {remaining_millis} ms remaining; at least {required_millis} ms is required"
    )]
    LeaseSafetyMarginInsufficient {
        remaining_millis: u64,
        required_millis: u64,
    },
    #[error("trusted time moved backward from {previous_unix_ms} to {now_unix_ms}")]
    TrustedClockRollback {
        previous_unix_ms: u64,
        now_unix_ms: u64,
    },
    #[error("trusted time is unavailable or cannot be represented")]
    TrustedClockUnavailable,
    #[error("live fencing token {live} is not newer than recovered token {recovered}")]
    LiveFenceNotNewer { live: u64, recovered: u64 },
    #[error("durable lifecycle world frontier does not match recovered canonical state")]
    LifecycleWorldFrontierMismatch,
    #[error("journal fencing token is invalid at line {line}: previous {previous}, found {found}")]
    InvalidHistoricalFence {
        line: usize,
        previous: u64,
        found: u64,
    },
    #[error(
        "journal fencing token {found} at line {line} is incompatible with snapshot fence {snapshot}"
    )]
    HistoricalFenceSnapshotMismatch {
        line: usize,
        snapshot: u64,
        found: u64,
    },
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
    authority_fencing_token: u64,
    #[serde(default)]
    content_manifest_version: Option<String>,
    #[serde(default)]
    universe_id: Option<String>,
    #[serde(default)]
    cell_id: Option<String>,
    #[serde(default)]
    universe_manifest_hash: Option<String>,
    #[serde(default)]
    celestial_registry_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleMode {
    Sleeping,
    Activating,
    Background,
    Active,
    Draining,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellLifecycleStatus {
    pub lifecycle_revision: u64,
    pub desired_mode: LifecycleMode,
    pub observed_mode: LifecycleMode,
    pub fencing_token: u64,
    pub expires_at_unix_ms: Option<u64>,
    pub last_world_event_sequence: u64,
    pub next_production_occurrence: Option<ProductionScheduleOccurrence>,
    pub acknowledged_production_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingWorldCommit {
    event_sequence: u64,
    event_hash: String,
    occurred_at_unix_ms: u64,
    prior_next_occurrence: Option<ProductionScheduleOccurrence>,
    resulting_next_occurrence: Option<ProductionScheduleOccurrence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CellLifecycleRecord {
    schema_version: u32,
    universe_id: String,
    cell_id: String,
    universe_manifest_hash: String,
    celestial_registry_hash: String,
    lifecycle_revision: u64,
    desired_mode: LifecycleMode,
    observed_mode: LifecycleMode,
    fencing_token: u64,
    holder_id: Option<String>,
    acquired_at_unix_ms: Option<u64>,
    renewed_at_unix_ms: Option<u64>,
    expires_at_unix_ms: Option<u64>,
    activation_cutoff_unix_ms: Option<u64>,
    last_trusted_unix_ms: u64,
    last_world_event_sequence: u64,
    last_world_event_hash: String,
    last_world_state_hash: String,
    next_production_occurrence: Option<ProductionScheduleOccurrence>,
    acknowledged_production_sequence: u64,
    pending_world_commit: Option<PendingWorldCommit>,
    updated_at_unix_ms: u64,
}

#[derive(Debug)]
pub struct Store {
    root: PathBuf,
    lock_file: File,
    journal_file: File,
    fencing_token: u64,
    lifecycle: CellLifecycleRecord,
    last_trusted_unix_ms: u64,
    world_seed: u64,
    cell_key: CellKeyV1,
    cell_id: String,
    universe_manifest: UniverseManifestSnapshot,
    clock: Arc<dyn TrustedClock>,
    write_enabled: bool,
    recovered_observed_mode: Option<LifecycleMode>,
    #[cfg(test)]
    append_failpoint: Option<AppendFailpoint>,
}

impl Store {
    pub fn open(root: impl AsRef<Path>, requested_seed: u64) -> Result<Self, PersistenceError> {
        Self::open_with_clock(root, requested_seed, Arc::new(SystemTrustedClock))
    }

    pub fn open_with_clock(
        root: impl AsRef<Path>,
        requested_seed: u64,
        clock: Arc<dyn TrustedClock>,
    ) -> Result<Self, PersistenceError> {
        Self::open_for_cell_with_clock(root, requested_seed, celestial::cell_origin_key(), clock)
    }

    pub fn open_for_cell(
        root: impl AsRef<Path>,
        requested_seed: u64,
        cell_key: CellKeyV1,
    ) -> Result<Self, PersistenceError> {
        Self::open_for_cell_with_clock(root, requested_seed, cell_key, Arc::new(SystemTrustedClock))
    }

    pub fn open_for_cell_with_clock(
        root: impl AsRef<Path>,
        requested_seed: u64,
        cell_key: CellKeyV1,
        clock: Arc<dyn TrustedClock>,
    ) -> Result<Self, PersistenceError> {
        celestial::validate_cell_key(&cell_key)
            .map_err(|source| PersistenceError::InvalidCellIdentity(source.to_string()))?;
        let cell_id = celestial::cell_id(&cell_key)
            .map_err(|source| PersistenceError::InvalidCellIdentity(source.to_string()))?;
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|source| io_error(&root, source))?;

        let lock_path = root.join(LOCK_FILE);
        let lock_file = OpenOptions::new()
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

        let manifest_path = root.join(MANIFEST_FILE);
        let runtime_manifest = celestial::universe_manifest(
            requested_seed,
            WORLD_SCHEMA_VERSION,
            EVENT_SCHEMA_VERSION,
        )
        .map_err(|source| PersistenceError::InvalidRuntimeUniverseManifest(source.to_string()))?;
        if cell_key.universe_id != runtime_manifest.universe_id {
            return Err(PersistenceError::InvalidCellIdentity(
                "cell key belongs to a different universe".into(),
            ));
        }
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

        let lifecycle_path = root.join(LIFECYCLE_FILE);
        let previous_lifecycle = if lifecycle_path.exists() {
            let lifecycle = read_json::<CellLifecycleRecord>(&lifecycle_path)?;
            validate_prior_lifecycle(&lifecycle, &runtime_manifest, &cell_id)?;
            Some(lifecycle)
        } else {
            let existing_snapshot = root.join(SNAPSHOT_FILE).exists();
            let existing_journal = root
                .join(JOURNAL_FILE)
                .metadata()
                .is_ok_and(|metadata| metadata.len() > 0);
            if existing_snapshot || existing_journal {
                return Err(PersistenceError::MissingLifecycleControl);
            }
            None
        };
        let recovered_observed_mode = previous_lifecycle
            .as_ref()
            .map(|record| record.observed_mode);
        let recovered_fence = recover_persisted_fencing_frontier(&root)?;
        let previous_token = previous_lifecycle
            .as_ref()
            .map_or(recovered_fence, |lifecycle| {
                lifecycle.fencing_token.max(recovered_fence)
            });
        let lifecycle_revision = previous_lifecycle.as_ref().map_or(Ok(1), |lifecycle| {
            lifecycle.lifecycle_revision.checked_add(1).ok_or(
                PersistenceError::InvalidLifecycleControl("lifecycle revision is exhausted".into()),
            )
        })?;
        let fencing_token = previous_token
            .checked_add(1)
            .ok_or(PersistenceError::FencingTokenExhausted)?;
        let durable_last_trusted_unix_ms = previous_lifecycle
            .as_ref()
            .map_or(0, |lifecycle| lifecycle.last_trusted_unix_ms);
        let acquired_at_unix_ms =
            accept_trusted_time(clock.now_unix_ms()?, durable_last_trusted_unix_ms)?;
        let expires_at_unix_ms = acquired_at_unix_ms
            .checked_add(LEASE_DURATION_MILLIS)
            .ok_or(PersistenceError::TrustedClockUnavailable)?;
        let holder_id = Uuid::new_v4().to_string();
        let activation_cutoff_unix_ms = previous_lifecycle
            .as_ref()
            .filter(|record| record.observed_mode == LifecycleMode::Activating)
            .and_then(|record| record.activation_cutoff_unix_ms)
            .or(Some(acquired_at_unix_ms));
        let lifecycle = CellLifecycleRecord {
            schema_version: LIFECYCLE_CONTROL_SCHEMA_VERSION,
            universe_id: runtime_manifest.universe_id.clone(),
            cell_id: cell_id.clone(),
            universe_manifest_hash: runtime_manifest.manifest_hash.clone(),
            celestial_registry_hash: runtime_manifest.celestial_registry_hash.clone(),
            lifecycle_revision,
            desired_mode: LifecycleMode::Active,
            observed_mode: LifecycleMode::Activating,
            fencing_token,
            holder_id: Some(holder_id),
            acquired_at_unix_ms: Some(acquired_at_unix_ms),
            renewed_at_unix_ms: Some(acquired_at_unix_ms),
            expires_at_unix_ms: Some(expires_at_unix_ms),
            activation_cutoff_unix_ms,
            last_trusted_unix_ms: acquired_at_unix_ms,
            last_world_event_sequence: previous_lifecycle
                .as_ref()
                .map_or(0, |record| record.last_world_event_sequence),
            last_world_event_hash: previous_lifecycle
                .as_ref()
                .map_or_else(String::new, |record| record.last_world_event_hash.clone()),
            last_world_state_hash: previous_lifecycle
                .as_ref()
                .map_or_else(String::new, |record| record.last_world_state_hash.clone()),
            next_production_occurrence: previous_lifecycle
                .as_ref()
                .and_then(|record| record.next_production_occurrence.clone()),
            acknowledged_production_sequence: previous_lifecycle
                .as_ref()
                .map_or(0, |record| record.acknowledged_production_sequence),
            pending_world_commit: previous_lifecycle
                .as_ref()
                .and_then(|record| record.pending_world_commit.clone()),
            updated_at_unix_ms: acquired_at_unix_ms,
        };
        write_json_atomic(&lifecycle_path, &lifecycle)?;

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
            lifecycle,
            last_trusted_unix_ms: acquired_at_unix_ms,
            world_seed: requested_seed,
            cell_key,
            cell_id,
            universe_manifest: runtime_manifest,
            clock,
            write_enabled: true,
            recovered_observed_mode,
            #[cfg(test)]
            append_failpoint: None,
        })
    }

    pub const fn fencing_token(&self) -> u64 {
        self.fencing_token
    }

    pub const fn cell_key(&self) -> &CellKeyV1 {
        &self.cell_key
    }

    pub fn cell_id(&self) -> &str {
        &self.cell_id
    }

    pub(crate) fn root_path(&self) -> &Path {
        &self.root
    }

    pub(crate) fn clock(&self) -> Arc<dyn TrustedClock> {
        Arc::clone(&self.clock)
    }

    pub const fn lifecycle_mode(&self) -> LifecycleMode {
        self.lifecycle.observed_mode
    }

    pub fn next_production_occurrence(&self) -> Option<&ProductionScheduleOccurrence> {
        self.lifecycle.next_production_occurrence.as_ref()
    }

    pub const fn activation_cutoff_unix_ms(&self) -> Option<u64> {
        self.lifecycle.activation_cutoff_unix_ms
    }

    pub fn lifecycle_status(&self) -> CellLifecycleStatus {
        CellLifecycleStatus {
            lifecycle_revision: self.lifecycle.lifecycle_revision,
            desired_mode: self.lifecycle.desired_mode,
            observed_mode: self.lifecycle.observed_mode,
            fencing_token: self.lifecycle.fencing_token,
            expires_at_unix_ms: self.lifecycle.expires_at_unix_ms,
            last_world_event_sequence: self.lifecycle.last_world_event_sequence,
            next_production_occurrence: self.lifecycle.next_production_occurrence.clone(),
            acknowledged_production_sequence: self.lifecycle.acknowledged_production_sequence,
        }
    }

    pub fn accepted_trusted_time(&mut self) -> Result<u64, PersistenceError> {
        self.trusted_unix_millis()
    }

    pub fn renew_lease(&mut self) -> Result<(), PersistenceError> {
        let found = self.read_current_lease()?;
        let now_unix_ms = self.trusted_unix_millis()?;
        self.validate_live_lease(&found, now_unix_ms)?;
        let renewed_at_unix_ms = self
            .lifecycle
            .renewed_at_unix_ms
            .ok_or(PersistenceError::LeaseOwnershipChanged)?;
        let expires_at_unix_ms = self
            .lifecycle
            .expires_at_unix_ms
            .ok_or(PersistenceError::LeaseOwnershipChanged)?;
        let renewal_due =
            now_unix_ms.saturating_sub(renewed_at_unix_ms) >= LEASE_RENEWAL_INTERVAL_MILLIS;
        let write_margin_low =
            expires_at_unix_ms.saturating_sub(now_unix_ms) < LEASE_WRITE_SAFETY_MARGIN_MILLIS;
        if !renewal_due && !write_margin_low {
            return Ok(());
        }
        let expires_at_unix_ms = now_unix_ms
            .checked_add(LEASE_DURATION_MILLIS)
            .ok_or(PersistenceError::TrustedClockUnavailable)?;
        let renewed = CellLifecycleRecord {
            renewed_at_unix_ms: Some(now_unix_ms),
            expires_at_unix_ms: Some(expires_at_unix_ms),
            last_trusted_unix_ms: now_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            ..found
        };
        write_json_atomic(&self.root.join(LIFECYCLE_FILE), &renewed)?;
        self.lifecycle = renewed;
        self.last_trusted_unix_ms = now_unix_ms;
        Ok(())
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
            let state = WorldState::genesis_for_cell(self.world_seed, &self.cell_key)
                .map_err(PersistenceError::InvalidCellIdentity)?;
            if !self.world_binding_matches(&state) {
                return Err(PersistenceError::WorldUniverseBindingMismatch);
            }
            state
        };
        let snapshot_event_sequence = state.event_sequence;
        let snapshot_fence = state.fencing_token;

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

        let mut previous_historical_fence = 0_u64;
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
            if header.authority_fencing_token == 0
                || header.authority_fencing_token < previous_historical_fence
            {
                return Err(PersistenceError::InvalidHistoricalFence {
                    line: index + 1,
                    previous: previous_historical_fence,
                    found: header.authority_fencing_token,
                });
            }
            previous_historical_fence = header.authority_fencing_token;
            if (header.event_sequence <= snapshot_event_sequence
                && header.authority_fencing_token > snapshot_fence)
                || (header.event_sequence > snapshot_event_sequence
                    && header.authority_fencing_token < snapshot_fence)
            {
                return Err(PersistenceError::HistoricalFenceSnapshotMismatch {
                    line: index + 1,
                    snapshot: snapshot_fence,
                    found: header.authority_fencing_token,
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
        let recovered_fence = state.fencing_token.max(previous_historical_fence);
        if self.fencing_token <= recovered_fence {
            return Err(PersistenceError::LiveFenceNotNewer {
                live: self.fencing_token,
                recovered: recovered_fence,
            });
        }
        self.reconcile_lifecycle_with_world(&state)?;
        Ok(state)
    }

    pub fn publish_active(&mut self, state: &WorldState) -> Result<(), PersistenceError> {
        self.verify_live_lease_for_write()?;
        if self.lifecycle.observed_mode == LifecycleMode::Active
            && self.lifecycle.desired_mode == LifecycleMode::Active
        {
            return Ok(());
        }
        let now_unix_ms = self.trusted_unix_millis()?;
        let mut active = self.lifecycle.clone();
        active.lifecycle_revision = active.lifecycle_revision.checked_add(1).ok_or_else(|| {
            PersistenceError::InvalidLifecycleControl("lifecycle revision is exhausted".into())
        })?;
        active.desired_mode = LifecycleMode::Active;
        active.observed_mode = LifecycleMode::Active;
        active.activation_cutoff_unix_ms = None;
        active.last_world_event_sequence = state.event_sequence;
        active
            .last_world_event_hash
            .clone_from(&state.last_event_hash);
        active.last_world_state_hash = state.state_hash();
        active.last_trusted_unix_ms = now_unix_ms;
        active.updated_at_unix_ms = now_unix_ms;
        self.persist_lifecycle(active)
    }

    pub fn transition_mode(
        &mut self,
        desired_mode: LifecycleMode,
        observed_mode: LifecycleMode,
        state: &WorldState,
    ) -> Result<(), PersistenceError> {
        self.verify_live_lease_for_write()?;
        if self.lifecycle.desired_mode == desired_mode
            && self.lifecycle.observed_mode == observed_mode
        {
            return Ok(());
        }
        if !valid_lifecycle_transition(self.lifecycle.observed_mode, observed_mode) {
            return Err(PersistenceError::InvalidLifecycleControl(format!(
                "invalid transition from {:?} to {:?}",
                self.lifecycle.observed_mode, observed_mode
            )));
        }
        let now_unix_ms = self.trusted_unix_millis()?;
        let mut transitioned = self.lifecycle.clone();
        transitioned.lifecycle_revision = transitioned
            .lifecycle_revision
            .checked_add(1)
            .ok_or_else(|| {
                PersistenceError::InvalidLifecycleControl("lifecycle revision is exhausted".into())
            })?;
        transitioned.desired_mode = desired_mode;
        transitioned.observed_mode = observed_mode;
        transitioned.activation_cutoff_unix_ms =
            (observed_mode == LifecycleMode::Activating).then_some(now_unix_ms);
        transitioned.last_world_event_sequence = state.event_sequence;
        transitioned
            .last_world_event_hash
            .clone_from(&state.last_event_hash);
        transitioned.last_world_state_hash = state.state_hash();
        transitioned.last_trusted_unix_ms = now_unix_ms;
        transitioned.updated_at_unix_ms = now_unix_ms;
        self.persist_lifecycle(transitioned)
    }

    pub fn release_to_sleeping(&mut self, state: &WorldState) -> Result<(), PersistenceError> {
        if !matches!(
            self.lifecycle.observed_mode,
            LifecycleMode::Draining | LifecycleMode::Background
        ) {
            return Err(PersistenceError::InvalidLifecycleControl(
                "only a draining or background cell may release to sleeping".into(),
            ));
        }
        self.save_snapshot(state)?;
        let now_unix_ms = self.trusted_unix_millis()?;
        let mut sleeping = self.lifecycle.clone();
        sleeping.lifecycle_revision =
            sleeping.lifecycle_revision.checked_add(1).ok_or_else(|| {
                PersistenceError::InvalidLifecycleControl("lifecycle revision is exhausted".into())
            })?;
        sleeping.desired_mode = LifecycleMode::Sleeping;
        sleeping.observed_mode = LifecycleMode::Sleeping;
        sleeping.holder_id = None;
        sleeping.acquired_at_unix_ms = None;
        sleeping.renewed_at_unix_ms = None;
        sleeping.expires_at_unix_ms = None;
        sleeping.activation_cutoff_unix_ms = None;
        sleeping.last_world_event_sequence = state.event_sequence;
        sleeping
            .last_world_event_hash
            .clone_from(&state.last_event_hash);
        sleeping.last_world_state_hash = state.state_hash();
        sleeping.last_trusted_unix_ms = now_unix_ms;
        sleeping.updated_at_unix_ms = now_unix_ms;
        write_json_atomic(&self.root.join(LIFECYCLE_FILE), &sleeping)?;
        self.lifecycle = sleeping;
        FileExt::unlock(&self.lock_file)
            .map_err(|source| io_error(self.root.join(LOCK_FILE), source))?;
        self.write_enabled = false;
        Ok(())
    }

    pub fn restore_recovered_host_mode(
        &mut self,
        state: &WorldState,
    ) -> Result<LifecycleMode, PersistenceError> {
        match self.recovered_observed_mode {
            Some(LifecycleMode::Sleeping) => {
                if self.lifecycle.observed_mode != LifecycleMode::Activating {
                    return Err(PersistenceError::InvalidLifecycleControl(
                        "sleeping recovery must hold activating authority".into(),
                    ));
                }
                self.save_snapshot(state)?;
                let now_unix_ms = self.trusted_unix_millis()?;
                let mut sleeping = self.lifecycle.clone();
                sleeping.lifecycle_revision =
                    sleeping.lifecycle_revision.checked_add(1).ok_or_else(|| {
                        PersistenceError::InvalidLifecycleControl(
                            "lifecycle revision is exhausted".into(),
                        )
                    })?;
                sleeping.desired_mode = LifecycleMode::Sleeping;
                sleeping.observed_mode = LifecycleMode::Sleeping;
                sleeping.holder_id = None;
                sleeping.acquired_at_unix_ms = None;
                sleeping.renewed_at_unix_ms = None;
                sleeping.expires_at_unix_ms = None;
                sleeping.activation_cutoff_unix_ms = None;
                sleeping.last_world_event_sequence = state.event_sequence;
                sleeping
                    .last_world_event_hash
                    .clone_from(&state.last_event_hash);
                sleeping.last_world_state_hash = state.state_hash();
                sleeping.last_trusted_unix_ms = now_unix_ms;
                sleeping.updated_at_unix_ms = now_unix_ms;
                write_json_atomic(&self.root.join(LIFECYCLE_FILE), &sleeping)?;
                self.lifecycle = sleeping;
                FileExt::unlock(&self.lock_file)
                    .map_err(|source| io_error(self.root.join(LOCK_FILE), source))?;
                self.write_enabled = false;
                Ok(LifecycleMode::Sleeping)
            }
            Some(LifecycleMode::Background) => {
                self.verify_live_lease_for_write()?;
                let now_unix_ms = self.trusted_unix_millis()?;
                let mut background = self.lifecycle.clone();
                background.lifecycle_revision = background
                    .lifecycle_revision
                    .checked_add(1)
                    .ok_or_else(|| {
                        PersistenceError::InvalidLifecycleControl(
                            "lifecycle revision is exhausted".into(),
                        )
                    })?;
                background.desired_mode = LifecycleMode::Background;
                background.observed_mode = LifecycleMode::Background;
                background.activation_cutoff_unix_ms = None;
                background.last_world_event_sequence = state.event_sequence;
                background
                    .last_world_event_hash
                    .clone_from(&state.last_event_hash);
                background.last_world_state_hash = state.state_hash();
                background.last_trusted_unix_ms = now_unix_ms;
                background.updated_at_unix_ms = now_unix_ms;
                self.persist_lifecycle(background)?;
                Ok(LifecycleMode::Background)
            }
            Some(LifecycleMode::Draining) => {
                let runnable = state
                    .background_production_is_runnable()
                    .map_err(|source| {
                        PersistenceError::InvalidLifecycleControl(source.to_string())
                    })?;
                self.recovered_observed_mode = Some(if runnable {
                    LifecycleMode::Background
                } else {
                    LifecycleMode::Sleeping
                });
                self.restore_recovered_host_mode(state)
            }
            Some(LifecycleMode::Activating | LifecycleMode::Active) | None => {
                Ok(LifecycleMode::Activating)
            }
        }
    }

    pub fn commit_world_event(
        &mut self,
        event: &CanonicalEvent,
        resulting_state: &WorldState,
        resulting_next_occurrence: Option<ProductionScheduleOccurrence>,
    ) -> Result<(), PersistenceError> {
        self.verify_live_lease_for_write()?;
        if event.authority_fencing_token != self.fencing_token {
            return Err(PersistenceError::FencingTokenChanged {
                expected: self.fencing_token,
                found: event.authority_fencing_token,
            });
        }
        if resulting_state.event_sequence != event.event_sequence
            || resulting_state.last_event_hash != event.event_hash
            || resulting_state.fencing_token != self.fencing_token
        {
            return Err(PersistenceError::LifecycleWorldFrontierMismatch);
        }
        if let Some(occurrence) = &resulting_next_occurrence {
            validate_occurrence_binding(occurrence, &self.lifecycle)?;
        }
        let now_unix_ms = self.trusted_unix_millis()?;
        let pending = PendingWorldCommit {
            event_sequence: event.event_sequence,
            event_hash: event.event_hash.clone(),
            occurred_at_unix_ms: event.occurred_at_unix_ms,
            prior_next_occurrence: self.lifecycle.next_production_occurrence.clone(),
            resulting_next_occurrence: resulting_next_occurrence.clone(),
        };
        let mut staged = self.lifecycle.clone();
        staged.pending_world_commit = Some(pending);
        staged.last_trusted_unix_ms = now_unix_ms;
        staged.updated_at_unix_ms = now_unix_ms;
        self.persist_lifecycle(staged)?;

        self.append_event(event)?;

        let now_unix_ms = self.trusted_unix_millis()?;
        let mut committed = self.lifecycle.clone();
        committed.last_world_event_sequence = event.event_sequence;
        committed
            .last_world_event_hash
            .clone_from(&event.event_hash);
        committed.last_world_state_hash = resulting_state.state_hash();
        committed.next_production_occurrence = resulting_next_occurrence;
        committed.pending_world_commit = None;
        committed.last_trusted_unix_ms = now_unix_ms;
        committed.updated_at_unix_ms = now_unix_ms;
        self.persist_lifecycle(committed)
    }

    pub fn acknowledge_production_sequence(
        &mut self,
        state: &WorldState,
    ) -> Result<(), PersistenceError> {
        self.verify_live_lease_for_write()?;
        let production_quantum_sequence = state.production_clock.last_committed_quantum_sequence;
        if production_quantum_sequence <= self.lifecycle.acknowledged_production_sequence {
            return Ok(());
        }
        if state.event_sequence != self.lifecycle.last_world_event_sequence
            || state.last_event_hash != self.lifecycle.last_world_event_hash
            || state.state_hash() != self.lifecycle.last_world_state_hash
        {
            return Err(PersistenceError::LifecycleWorldFrontierMismatch);
        }
        let now_unix_ms = self.trusted_unix_millis()?;
        let mut acknowledged = self.lifecycle.clone();
        acknowledged.acknowledged_production_sequence = production_quantum_sequence;
        acknowledged.last_trusted_unix_ms = now_unix_ms;
        acknowledged.updated_at_unix_ms = now_unix_ms;
        self.persist_lifecycle(acknowledged)
    }

    pub fn append_event(&mut self, event: &CanonicalEvent) -> Result<(), PersistenceError> {
        self.verify_live_lease_for_write()?;
        if event.authority_fencing_token != self.fencing_token {
            return Err(PersistenceError::FencingTokenChanged {
                expected: self.fencing_token,
                found: event.authority_fencing_token,
            });
        }
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
        self.verify_live_lease_for_write()?;
        if state.fencing_token != self.fencing_token {
            return Err(PersistenceError::FencingTokenChanged {
                expected: self.fencing_token,
                found: state.fencing_token,
            });
        }
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
        write_json_atomic(&self.root.join(SNAPSHOT_FILE), &snapshot)?;
        let now_unix_ms = self.trusted_unix_millis()?;
        let mut lifecycle = self.lifecycle.clone();
        lifecycle.last_world_event_sequence = state.event_sequence;
        lifecycle
            .last_world_event_hash
            .clone_from(&state.last_event_hash);
        lifecycle.last_world_state_hash = state.state_hash();
        lifecycle.last_trusted_unix_ms = now_unix_ms;
        lifecycle.updated_at_unix_ms = now_unix_ms;
        self.persist_lifecycle(lifecycle)
    }

    fn world_binding_matches(&self, state: &WorldState) -> bool {
        state.world_seed == self.world_seed
            && state.universe_id == self.universe_manifest.universe_id
            && state.content_manifest_version == self.universe_manifest.content_manifest_version
            && state.universe_manifest_hash == self.universe_manifest.manifest_hash
            && state.celestial_registry_hash == self.universe_manifest.celestial_registry_hash
            && state.cell_id == self.cell_id
            && celestial::cell_key_from_address(&state.cell_address)
                .is_ok_and(|cell_key| cell_key == self.cell_key)
    }

    fn event_header_binding_matches(&self, header: &EventHeader) -> bool {
        header.content_manifest_version.as_deref()
            == Some(self.universe_manifest.content_manifest_version.as_str())
            && header.universe_id.as_deref() == Some(self.universe_manifest.universe_id.as_str())
            && header.cell_id.as_deref() == Some(self.cell_id.as_str())
            && header.universe_manifest_hash.as_deref()
                == Some(self.universe_manifest.manifest_hash.as_str())
            && header.celestial_registry_hash.as_deref()
                == Some(self.universe_manifest.celestial_registry_hash.as_str())
    }

    fn event_binding_matches(&self, event: &CanonicalEvent) -> bool {
        event.content_manifest_version == self.universe_manifest.content_manifest_version
            && event.universe_id == self.universe_manifest.universe_id
            && event.cell_id == self.cell_id
            && event.universe_manifest_hash == self.universe_manifest.manifest_hash
            && event.celestial_registry_hash == self.universe_manifest.celestial_registry_hash
    }

    fn verify_live_lease_for_write(&mut self) -> Result<(), PersistenceError> {
        if !self.write_enabled {
            return Err(PersistenceError::StoreReleased);
        }
        let mut remaining_millis = 0;
        for _ in 0..2 {
            self.renew_lease()?;
            let now_unix_ms = self.trusted_unix_millis()?;
            let found = self.read_current_lease()?;
            self.validate_live_lease(&found, now_unix_ms)?;
            remaining_millis = found
                .expires_at_unix_ms
                .ok_or(PersistenceError::LeaseOwnershipChanged)?
                .saturating_sub(now_unix_ms);
            if remaining_millis >= LEASE_WRITE_SAFETY_MARGIN_MILLIS {
                return Ok(());
            }
        }
        Err(PersistenceError::LeaseSafetyMarginInsufficient {
            remaining_millis,
            required_millis: LEASE_WRITE_SAFETY_MARGIN_MILLIS,
        })
    }

    fn read_current_lease(&self) -> Result<CellLifecycleRecord, PersistenceError> {
        let lifecycle_path = self.root.join(LIFECYCLE_FILE);
        if !lifecycle_path.exists() {
            return Err(PersistenceError::LeaseOwnershipChanged);
        }
        read_json(&lifecycle_path)
    }

    fn validate_live_lease(
        &self,
        found: &CellLifecycleRecord,
        now_unix_ms: u64,
    ) -> Result<(), PersistenceError> {
        if found.fencing_token != self.fencing_token {
            return Err(PersistenceError::FencingTokenChanged {
                expected: self.fencing_token,
                found: found.fencing_token,
            });
        }
        if found.schema_version != LIFECYCLE_CONTROL_SCHEMA_VERSION || found != &self.lifecycle {
            return Err(PersistenceError::LeaseOwnershipChanged);
        }
        let expires_at_unix_ms = found
            .expires_at_unix_ms
            .ok_or(PersistenceError::LeaseOwnershipChanged)?;
        if now_unix_ms >= expires_at_unix_ms {
            return Err(PersistenceError::LeaseExpired {
                expires_at_unix_ms,
                now_unix_ms,
            });
        }
        Ok(())
    }

    fn trusted_unix_millis(&mut self) -> Result<u64, PersistenceError> {
        let accepted = accept_trusted_time(self.clock.now_unix_ms()?, self.last_trusted_unix_ms)?;
        self.last_trusted_unix_ms = accepted;
        Ok(accepted)
    }

    fn reconcile_lifecycle_with_world(
        &mut self,
        state: &WorldState,
    ) -> Result<(), PersistenceError> {
        let found = self.read_current_lease()?;
        let now_unix_ms = self.trusted_unix_millis()?;
        self.validate_live_lease(&found, now_unix_ms)?;
        let mut reconciled = found;

        if let Some(pending) = reconciled.pending_world_commit.take() {
            if state.event_sequence < pending.event_sequence {
                reconciled.next_production_occurrence = pending.prior_next_occurrence;
            } else if state.event_sequence == pending.event_sequence
                && state.last_event_hash == pending.event_hash
            {
                reconciled.next_production_occurrence = pending.resulting_next_occurrence;
            } else {
                return Err(PersistenceError::LifecycleWorldFrontierMismatch);
            }
        }
        if reconciled.last_world_event_sequence > state.event_sequence {
            return Err(PersistenceError::LifecycleWorldFrontierMismatch);
        }
        if reconciled.last_world_event_sequence == state.event_sequence
            && ((!reconciled.last_world_event_hash.is_empty()
                && reconciled.last_world_event_hash != state.last_event_hash)
                || (!reconciled.last_world_state_hash.is_empty()
                    && reconciled.last_world_state_hash != state.state_hash()))
        {
            return Err(PersistenceError::LifecycleWorldFrontierMismatch);
        }

        let committed_sequence = state.production_clock.last_committed_quantum_sequence;
        if reconciled.acknowledged_production_sequence > committed_sequence {
            return Err(PersistenceError::InvalidLifecycleControl(
                "production acknowledgement is ahead of canonical world state".into(),
            ));
        }
        let runnable = state
            .background_production_is_runnable()
            .map_err(|source| PersistenceError::InvalidLifecycleControl(source.to_string()))?;
        match &reconciled.next_production_occurrence {
            Some(occurrence) => {
                let expected_sequence = committed_sequence.checked_add(1).ok_or_else(|| {
                    PersistenceError::InvalidLifecycleControl(
                        "production occurrence sequence is exhausted".into(),
                    )
                })?;
                let expected_time = if committed_sequence == 0 {
                    None
                } else {
                    Some(
                        state
                            .production_clock
                            .last_scheduled_for_unix_ms
                            .checked_add(1_000)
                            .ok_or_else(|| {
                                PersistenceError::InvalidLifecycleControl(
                                    "production schedule time is exhausted".into(),
                                )
                            })?,
                    )
                };
                if !runnable
                    || occurrence.lifecycle_generation
                        != state.production_clock.lifecycle_generation
                    || occurrence.production_quantum_sequence != expected_sequence
                    || expected_time
                        .is_some_and(|scheduled| occurrence.scheduled_for_unix_ms != scheduled)
                {
                    return Err(PersistenceError::InvalidLifecycleControl(
                        "next occurrence does not match canonical production state".into(),
                    ));
                }
            }
            None if runnable => {
                let scheduled_for_unix_ms = now_unix_ms
                    .checked_add(1_000)
                    .ok_or(PersistenceError::TrustedClockUnavailable)?;
                reconciled.next_production_occurrence = Some(
                    state
                        .next_production_occurrence_at(scheduled_for_unix_ms)
                        .map_err(|source| {
                            PersistenceError::InvalidLifecycleControl(source.to_string())
                        })?,
                );
            }
            None => {}
        }
        reconciled.acknowledged_production_sequence = committed_sequence;
        reconciled.last_world_event_sequence = state.event_sequence;
        reconciled
            .last_world_event_hash
            .clone_from(&state.last_event_hash);
        reconciled.last_world_state_hash = state.state_hash();
        reconciled.last_trusted_unix_ms = now_unix_ms;
        reconciled.updated_at_unix_ms = now_unix_ms;
        self.persist_lifecycle(reconciled)
    }

    fn persist_lifecycle(
        &mut self,
        lifecycle: CellLifecycleRecord,
    ) -> Result<(), PersistenceError> {
        write_json_atomic(&self.root.join(LIFECYCLE_FILE), &lifecycle)?;
        self.lifecycle = lifecycle;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_append_failpoint(&mut self, failpoint: AppendFailpoint) {
        self.append_failpoint = Some(failpoint);
    }

    #[cfg(test)]
    pub(crate) fn install_next_production_occurrence_for_test(
        &mut self,
        occurrence: ProductionScheduleOccurrence,
    ) -> Result<(), PersistenceError> {
        validate_occurrence_binding(&occurrence, &self.lifecycle)?;
        let expected_sequence = self
            .lifecycle
            .acknowledged_production_sequence
            .checked_add(1)
            .ok_or_else(|| {
                PersistenceError::InvalidLifecycleControl(
                    "acknowledged production sequence is exhausted".into(),
                )
            })?;
        if occurrence.production_quantum_sequence != expected_sequence {
            return Err(PersistenceError::InvalidLifecycleControl(
                "test production occurrence does not follow the acknowledged frontier".into(),
            ));
        }
        let mut lifecycle = self.lifecycle.clone();
        lifecycle.next_production_occurrence = Some(occurrence);
        self.persist_lifecycle(lifecycle)
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
        if self.write_enabled {
            let _ = FileExt::unlock(&self.lock_file);
        }
    }
}

fn valid_lifecycle_transition(from: LifecycleMode, to: LifecycleMode) -> bool {
    match from {
        LifecycleMode::Sleeping => {
            matches!(to, LifecycleMode::Activating | LifecycleMode::Background)
        }
        LifecycleMode::Background => {
            matches!(to, LifecycleMode::Activating | LifecycleMode::Sleeping)
        }
        LifecycleMode::Activating => to == LifecycleMode::Active,
        LifecycleMode::Active => to == LifecycleMode::Draining,
        LifecycleMode::Draining => {
            matches!(to, LifecycleMode::Background | LifecycleMode::Sleeping)
        }
    }
}

fn validate_prior_lifecycle(
    lifecycle: &CellLifecycleRecord,
    manifest: &UniverseManifestSnapshot,
    expected_cell_id: &str,
) -> Result<(), PersistenceError> {
    if lifecycle.schema_version != LIFECYCLE_CONTROL_SCHEMA_VERSION {
        return Err(PersistenceError::InvalidLifecycleControl(format!(
            "schema {} is unsupported; expected {}",
            lifecycle.schema_version, LIFECYCLE_CONTROL_SCHEMA_VERSION
        )));
    }
    if lifecycle.universe_id != manifest.universe_id
        || lifecycle.cell_id != expected_cell_id
        || lifecycle.universe_manifest_hash != manifest.manifest_hash
        || lifecycle.celestial_registry_hash != manifest.celestial_registry_hash
    {
        return Err(PersistenceError::InvalidLifecycleControl(
            "universe, cell, or manifest binding mismatch".into(),
        ));
    }
    if lifecycle.lifecycle_revision == 0 {
        return Err(PersistenceError::InvalidLifecycleControl(
            "lifecycle revision must be positive".into(),
        ));
    }
    match lifecycle.observed_mode {
        LifecycleMode::Sleeping => {
            if lifecycle.holder_id.is_some()
                || lifecycle.acquired_at_unix_ms.is_some()
                || lifecycle.renewed_at_unix_ms.is_some()
                || lifecycle.expires_at_unix_ms.is_some()
            {
                return Err(PersistenceError::InvalidLifecycleControl(
                    "sleeping lifecycle cannot retain a live holder or lease times".into(),
                ));
            }
            if lifecycle.activation_cutoff_unix_ms.is_some() {
                return Err(PersistenceError::InvalidLifecycleControl(
                    "sleeping lifecycle cannot retain an activation cut-off".into(),
                ));
            }
        }
        LifecycleMode::Activating
        | LifecycleMode::Background
        | LifecycleMode::Active
        | LifecycleMode::Draining => {
            let holder_id = lifecycle.holder_id.as_deref().unwrap_or_default();
            let (Some(acquired), Some(renewed), Some(expires)) = (
                lifecycle.acquired_at_unix_ms,
                lifecycle.renewed_at_unix_ms,
                lifecycle.expires_at_unix_ms,
            ) else {
                return Err(PersistenceError::InvalidLifecycleControl(
                    "live lifecycle requires complete lease times".into(),
                ));
            };
            if lifecycle.fencing_token == 0
                || holder_id.is_empty()
                || acquired > renewed
                || renewed > lifecycle.last_trusted_unix_ms
                || lifecycle.last_trusted_unix_ms >= expires
            {
                return Err(PersistenceError::InvalidLifecycleControl(
                    "live lifecycle holder, fence, or timestamps are not canonical".into(),
                ));
            }
            match (lifecycle.observed_mode, lifecycle.activation_cutoff_unix_ms) {
                (LifecycleMode::Activating, Some(cutoff))
                    if cutoff <= lifecycle.last_trusted_unix_ms => {}
                (LifecycleMode::Activating, _) => {
                    return Err(PersistenceError::InvalidLifecycleControl(
                        "activating lifecycle requires a trusted wake cut-off".into(),
                    ));
                }
                (_, None) => {}
                (_, Some(_)) => {
                    return Err(PersistenceError::InvalidLifecycleControl(
                        "only an activating lifecycle may retain a wake cut-off".into(),
                    ));
                }
            }
        }
    }
    if lifecycle.updated_at_unix_ms < lifecycle.last_trusted_unix_ms {
        return Err(PersistenceError::InvalidLifecycleControl(
            "lifecycle update time precedes trusted time".into(),
        ));
    }
    if lifecycle.last_world_event_sequence > 0
        && (lifecycle.last_world_event_hash.len() != 64
            || lifecycle.last_world_state_hash.len() != 64)
    {
        return Err(PersistenceError::InvalidLifecycleControl(
            "nonempty world frontier requires canonical hashes".into(),
        ));
    }
    if let Some(occurrence) = &lifecycle.next_production_occurrence {
        validate_occurrence_binding(occurrence, lifecycle)?;
        if occurrence.production_quantum_sequence <= lifecycle.acknowledged_production_sequence {
            return Err(PersistenceError::InvalidLifecycleControl(
                "next production occurrence does not follow the acknowledged frontier".into(),
            ));
        }
    }
    if let Some(pending) = &lifecycle.pending_world_commit {
        if pending.event_sequence <= lifecycle.last_world_event_sequence
            || pending.event_hash.len() != 64
            || pending.occurred_at_unix_ms == 0
        {
            return Err(PersistenceError::InvalidLifecycleControl(
                "pending world commit does not follow the durable world frontier".into(),
            ));
        }
        if let Some(occurrence) = &pending.prior_next_occurrence {
            validate_occurrence_binding(occurrence, lifecycle)?;
        }
        if let Some(occurrence) = &pending.resulting_next_occurrence {
            validate_occurrence_binding(occurrence, lifecycle)?;
        }
    }
    Ok(())
}

fn validate_occurrence_binding(
    occurrence: &ProductionScheduleOccurrence,
    lifecycle: &CellLifecycleRecord,
) -> Result<(), PersistenceError> {
    if occurrence.schema_version != crate::event::PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
        || occurrence.universe_id != lifecycle.universe_id
        || occurrence.cell_id != lifecycle.cell_id
        || occurrence.universe_manifest_hash != lifecycle.universe_manifest_hash
        || occurrence.celestial_registry_hash != lifecycle.celestial_registry_hash
        || occurrence.lifecycle_generation == 0
        || occurrence.production_quantum_sequence == 0
        || occurrence.scheduled_for_unix_ms == 0
    {
        return Err(PersistenceError::InvalidLifecycleControl(
            "production occurrence binding is invalid".into(),
        ));
    }
    Ok(())
}

fn recover_persisted_fencing_frontier(root: &Path) -> Result<u64, PersistenceError> {
    let snapshot_path = root.join(SNAPSHOT_FILE);
    let (snapshot_event_sequence, snapshot_fence) = if snapshot_path.exists() {
        let header: SnapshotHeader = read_json(&snapshot_path)?;
        if header.schema_version != WORLD_SCHEMA_VERSION {
            return Err(PersistenceError::SnapshotSchema {
                found: header.schema_version,
                expected: WORLD_SCHEMA_VERSION,
            });
        }
        let snapshot: SnapshotDocument = read_json(&snapshot_path)?;
        (snapshot.state.event_sequence, snapshot.state.fencing_token)
    } else {
        (0, 0)
    };

    let journal_path = root.join(JOURNAL_FILE);
    let journal_bytes = match fs::read(&journal_path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(source) => return Err(io_error(&journal_path, source)),
    };
    let committed_length = if journal_bytes.last() == Some(&b'\n') {
        journal_bytes.len()
    } else {
        journal_bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |position| position + 1)
    };
    let journal = std::str::from_utf8(&journal_bytes[..committed_length]).map_err(|source| {
        let line = std::str::from_utf8(&journal_bytes[..source.valid_up_to()])
            .expect("UTF-8 error reports a valid prefix")
            .matches('\n')
            .count()
            + 1;
        PersistenceError::CorruptJournal {
            line,
            message: source.to_string(),
        }
    })?;
    let mut historical_fence = 0_u64;
    for (index, line) in journal.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let header: EventHeader =
            serde_json::from_str(line).map_err(|source| PersistenceError::CorruptJournal {
                line: index + 1,
                message: source.to_string(),
            })?;
        if header.authority_fencing_token == 0 || header.authority_fencing_token < historical_fence
        {
            return Err(PersistenceError::InvalidHistoricalFence {
                line: index + 1,
                previous: historical_fence,
                found: header.authority_fencing_token,
            });
        }
        if (header.event_sequence <= snapshot_event_sequence
            && header.authority_fencing_token > snapshot_fence)
            || (header.event_sequence > snapshot_event_sequence
                && header.authority_fencing_token < snapshot_fence)
        {
            return Err(PersistenceError::HistoricalFenceSnapshotMismatch {
                line: index + 1,
                snapshot: snapshot_fence,
                found: header.authority_fencing_token,
            });
        }
        historical_fence = header.authority_fencing_token;
    }
    Ok(snapshot_fence.max(historical_fence))
}

fn accept_trusted_time(sample: u64, previous: u64) -> Result<u64, PersistenceError> {
    let rollback_floor = previous.saturating_sub(TRUSTED_CLOCK_ROLLBACK_TOLERANCE_MILLIS);
    if sample < rollback_floor {
        return Err(PersistenceError::TrustedClockRollback {
            previous_unix_ms: previous,
            now_unix_ms: sample,
        });
    }
    Ok(sample.max(previous))
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

fn unix_millis() -> Result<u64, PersistenceError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| PersistenceError::TrustedClockUnavailable)?
        .as_millis()
        .try_into()
        .map_err(|_| PersistenceError::TrustedClockUnavailable)
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::sync::atomic::{AtomicU64, Ordering};

    use tempfile::tempdir;
    use verse_protocol::{BlockKind, ClientMessage, IVec3, Vec3};

    use super::*;
    use crate::Runtime;
    use crate::event::EventPayload;
    use crate::model::{STARTER_GRID_ID, WorldState};

    #[derive(Debug)]
    struct ManualTrustedClock {
        now_unix_ms: AtomicU64,
    }

    impl ManualTrustedClock {
        const fn new(now_unix_ms: u64) -> Self {
            Self {
                now_unix_ms: AtomicU64::new(now_unix_ms),
            }
        }

        fn set(&self, now_unix_ms: u64) {
            self.now_unix_ms.store(now_unix_ms, Ordering::SeqCst);
        }
    }

    impl TrustedClock for ManualTrustedClock {
        fn now_unix_ms(&self) -> Result<u64, PersistenceError> {
            Ok(self.now_unix_ms.load(Ordering::SeqCst))
        }
    }

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
    fn append_and_snapshot_require_the_exact_live_fencing_token() {
        let directory = tempdir().expect("tempdir");
        let mut store = Store::open(directory.path(), 12).expect("store opens");
        let mut world = WorldState::genesis(12);
        world.fencing_token = store.fencing_token();
        let mut event = world.prepare_system_event(EventPayload::SuitOxygenChanged {
            player_id: "player-local".into(),
            previous_oxygen_milli: 1_000,
            new_oxygen_milli: 995,
        });
        event.authority_fencing_token += 1;
        event.event_hash = event.calculate_hash();
        assert!(matches!(
            store.append_event(&event),
            Err(PersistenceError::FencingTokenChanged { expected, found })
                if expected == store.fencing_token() && found == event.authority_fencing_token
        ));

        world.fencing_token = 0;
        assert!(matches!(
            store.save_snapshot(&world),
            Err(PersistenceError::FencingTokenChanged { expected, found: 0 })
                if expected == store.fencing_token()
        ));
    }

    #[test]
    fn fencing_token_exhaustion_fails_closed() {
        let directory = tempdir().expect("tempdir");
        drop(Store::open(directory.path(), 14).expect("initial store opens"));
        let lifecycle_path = directory.path().join(LIFECYCLE_FILE);
        let mut lease: CellLifecycleRecord = read_json(&lifecycle_path).expect("lease reads");
        lease.fencing_token = u64::MAX;
        write_json_atomic(&lifecycle_path, &lease).expect("lease updates");

        assert!(matches!(
            Store::open(directory.path(), 14),
            Err(PersistenceError::FencingTokenExhausted)
        ));
    }

    #[test]
    fn renewal_updates_the_durable_deadline_and_expiry_fences_writes() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(100_000));
        let mut store =
            Store::open_with_clock(directory.path(), 15, clock.clone()).expect("store opens");
        clock.set(100_000 + LEASE_RENEWAL_INTERVAL_MILLIS);
        store.renew_lease().expect("lease renews");
        assert_eq!(store.lifecycle.renewed_at_unix_ms, Some(105_000));
        assert_eq!(
            store.lifecycle.expires_at_unix_ms,
            Some(105_000 + LEASE_DURATION_MILLIS)
        );
        let lifecycle_path = directory.path().join(LIFECYCLE_FILE);
        let persisted: CellLifecycleRecord =
            read_json(&lifecycle_path).expect("renewed lease reads");
        assert_eq!(
            persisted.renewed_at_unix_ms,
            store.lifecycle.renewed_at_unix_ms
        );
        assert_eq!(
            persisted.expires_at_unix_ms,
            store.lifecycle.expires_at_unix_ms
        );
        assert_eq!(persisted.last_trusted_unix_ms, 105_000);

        clock.set(
            store
                .lifecycle
                .expires_at_unix_ms
                .expect("live lease has expiry"),
        );
        assert!(matches!(
            store.renew_lease(),
            Err(PersistenceError::LeaseExpired { .. })
        ));
    }

    #[test]
    fn trusted_clock_clamps_small_rollback_and_rejects_large_rollback() {
        let directory = tempdir().expect("tempdir");
        let clock = Arc::new(ManualTrustedClock::new(200_000));
        let mut store =
            Store::open_with_clock(directory.path(), 24, clock.clone()).expect("store opens");

        clock.set(199_500);
        store.renew_lease().expect("small rollback clamps");
        assert_eq!(store.last_trusted_unix_ms, 200_000);

        clock.set(198_999);
        assert!(matches!(
            store.renew_lease(),
            Err(PersistenceError::TrustedClockRollback {
                previous_unix_ms: 200_000,
                now_unix_ms: 198_999
            })
        ));
    }

    #[test]
    fn missing_lifecycle_control_for_existing_world_fails_closed() {
        let directory = tempdir().expect("tempdir");
        let mut runtime = Runtime::open(directory.path(), 25, 100).expect("runtime opens");
        runtime.persist_snapshot().expect("snapshot persists");
        drop(runtime);
        fs::remove_file(directory.path().join(LIFECYCLE_FILE)).expect("lifecycle removed");

        assert!(matches!(
            Store::open(directory.path(), 25),
            Err(PersistenceError::MissingLifecycleControl)
        ));
    }

    #[test]
    fn replacement_fence_exceeds_every_journal_header() {
        let directory = tempdir().expect("tempdir");
        {
            let mut runtime = Runtime::open(directory.path(), 26, 100).expect("runtime opens");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                    operation_sequence: 0,
                    operation_id: "historical-max".into(),
                    helmet_closed: false,
                    jetpack_enabled: false,
                    magnetic_boots_enabled: false,
                })
                .expect("event commits");
        }
        let journal_path = directory.path().join(JOURNAL_FILE);
        let journal = fs::read_to_string(&journal_path).expect("journal reads");
        let mut event: CanonicalEvent = serde_json::from_str(journal.trim()).expect("event parses");
        event.authority_fencing_token = 9;
        event.event_hash = event.calculate_hash();
        fs::write(
            &journal_path,
            format!(
                "{}\n",
                serde_json::to_string(&event).expect("event serializes")
            ),
        )
        .expect("journal updates");
        let lifecycle_path = directory.path().join(LIFECYCLE_FILE);
        let mut lifecycle: CellLifecycleRecord =
            read_json(&lifecycle_path).expect("lifecycle reads");
        lifecycle.last_world_event_sequence = 0;
        lifecycle.last_world_event_hash.clear();
        lifecycle.last_world_state_hash.clear();
        write_json_atomic(&lifecycle_path, &lifecycle).expect("lagging lifecycle persists");

        let mut replacement = Store::open(directory.path(), 26).expect("replacement opens");
        assert_eq!(replacement.fencing_token(), 10);
        replacement.load_world().expect("journal replays");
    }

    #[test]
    fn recovery_rejects_a_decreasing_historical_fence() {
        let directory = tempdir().expect("tempdir");
        drop(Runtime::open(directory.path(), 16, 100).expect("initial runtime opens"));
        {
            let mut runtime = Runtime::open(directory.path(), 16, 100).expect("second lease opens");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                    operation_sequence: 0,
                    operation_id: "fence-two".into(),
                    helmet_closed: false,
                    jetpack_enabled: false,
                    magnetic_boots_enabled: false,
                })
                .expect("first event commits");
        }
        {
            let mut runtime = Runtime::open(directory.path(), 16, 100).expect("third lease opens");
            runtime
                .execute_next_for_fixture(&ClientMessage::SetSuitMode {
                    operation_sequence: 0,
                    operation_id: "fence-three".into(),
                    helmet_closed: true,
                    jetpack_enabled: false,
                    magnetic_boots_enabled: false,
                })
                .expect("second event commits");
        }
        let journal_path = directory.path().join(JOURNAL_FILE);
        let journal = fs::read_to_string(&journal_path).expect("journal reads");
        let mut events = journal
            .lines()
            .map(|line| serde_json::from_str::<CanonicalEvent>(line).expect("event parses"))
            .collect::<Vec<_>>();
        assert_eq!(events.len(), 2);
        assert!(events[1].authority_fencing_token > events[0].authority_fencing_token);
        events[1].authority_fencing_token = 1;
        events[1].event_hash = events[1].calculate_hash();
        fs::write(
            &journal_path,
            format!(
                "{}\n{}\n",
                serde_json::to_string(&events[0]).expect("first event serializes"),
                serde_json::to_string(&events[1]).expect("second event serializes")
            ),
        )
        .expect("tampered journal writes");

        assert!(matches!(
            Store::open(directory.path(), 16),
            Err(PersistenceError::InvalidHistoricalFence {
                line: 2,
                previous: 2,
                found: 1
            })
        ));
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
        assert_eq!(stored.schema_version, 3);
        assert_eq!(stored.event_schema_version, 15);
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
            let mut world = WorldState::genesis(21);
            world.fencing_token = store.fencing_token();
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
