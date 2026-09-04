// SPDX-License-Identifier: AGPL-3.0-or-later

//! Universe-wide protocol-19 prepared installation and verified-open bridge.
//!
//! Per-cell and directory heads are staging evidence only. The canonical
//! prepared-install head is written last and is the sole all-cell commit
//! marker. Active boot first obtains a read-only validated capability, then
//! consumes it to open recovery-capable stores only after authorization.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use verse_protocol::{CellKeyV1, protocol_v19::Protocol19CompatibilityTuple};

#[cfg(test)]
use crate::cell_directory::CellAssignmentRecord;
use crate::cell_directory::{CellAssignmentState, CellDirectoryError};
use crate::cell_directory_v3::{
    ActiveDirectoryV3Expectation, DraftCellDirectoryHistoryStoreV3,
    ValidatedProtocol19DirectoryGenesis,
};
use crate::grid_handoff_v2::lifecycle_v2::{
    LifecycleAuthorityOperationKindV2, LifecycleHeadCommitmentV2,
    LifecycleInitializationFailpointV2, LifecycleModeV2, LifecycleRecordV2,
    LifecycleRuntimePreflightV2, LifecycleV2Error, PRODUCTION_QUANTUM_MILLIS,
    PreparedLifecycleAppendV2,
};
use crate::grid_handoff_v2::migration_transform::ValidatedProtocol19MigrationTransform;
use crate::grid_handoff_v2::migration_transform::{
    recover_identity_map_root, recover_production_origin_root,
};
use crate::grid_handoff_v2::store_v21::{
    DraftWorld21Store, DraftWorld21StoreError, PreparedWorld21CellEvidence,
    ValidatedWorld21StagingTarget,
};
use crate::manifest_v5::ValidatedUniverseManifestV5;
use crate::persistence::TrustedClock;
use crate::protocol19_migration::{
    CanonicalProtocol19MigrationReceiptEvidence, MigrationReceiptError,
    ValidatedProtocol19MigrationReceipt, hash_source_directory_archive,
    recover_canonical_migration_receipt,
};

const MAX_BACKGROUND_QUANTA_PER_DISPATCH: u64 = 60;
const MAX_BACKGROUND_DISPATCH_TIME: Duration = Duration::from_millis(250);
const ACTIVATION_DIRECTORY: &str = "protocol-19-activation-v1";
const UNIVERSE_LIFECYCLE_HEAD_FILE: &str = "universe-lifecycle-v1.head.json";
const UNIVERSE_LIFECYCLE_HEAD_SCHEMA_VERSION: u32 = 1;
const UNIVERSE_LIFECYCLE_HEAD_HASH_DOMAIN: &[u8] = b"the-verse/universe-lifecycle-head/v1\0";
const UNIVERSE_LIFECYCLE_GENESIS_ROOT_DOMAIN: &[u8] =
    b"the-verse/universe-lifecycle-genesis-root/v1\0";
const MAX_UNIVERSE_LIFECYCLE_HEAD_BYTES: usize = 16 * 1_024 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Protocol19BackgroundDispatchOutcome {
    pub(crate) mode: &'static str,
    pub(crate) committed_quanta: u64,
    pub(crate) acknowledged_production_sequence: u64,
    pub(crate) next_scheduled_for_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UniverseLifecycleGenesisBindingV1 {
    cell_id: String,
    lifecycle_genesis_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingUniverseLifecycleAppendV1 {
    cell_id: String,
    prior: Option<LifecycleHeadCommitmentV2>,
    record: LifecycleRecordV2,
    next: LifecycleHeadCommitmentV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct UniverseLifecycleHeadV1 {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    universe_id: String,
    manifest_hash: String,
    active_head_hash: String,
    prepared_install_head_hash: String,
    cell_set_root: String,
    lifecycle_genesis_root: String,
    universe_lifecycle_revision: u64,
    cells: BTreeMap<String, LifecycleHeadCommitmentV2>,
    pending_append: Option<PendingUniverseLifecycleAppendV1>,
    previous_head_hash: String,
    head_hash: String,
}

impl UniverseLifecycleHeadV1 {
    fn initial(
        prepared: &Protocol19PreparedInstallHeadV1,
        active_head_hash: &str,
    ) -> Result<Self, Protocol19InstallError> {
        let mut head = Self {
            schema_version: UNIVERSE_LIFECYCLE_HEAD_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            universe_id: prepared.universe_id.clone(),
            manifest_hash: prepared.target_manifest_hash.clone(),
            active_head_hash: active_head_hash.to_owned(),
            prepared_install_head_hash: prepared.head_hash.clone(),
            cell_set_root: prepared.cell_set_root.clone(),
            lifecycle_genesis_root: lifecycle_genesis_root(prepared)?,
            universe_lifecycle_revision: 1,
            cells: BTreeMap::new(),
            pending_append: None,
            previous_head_hash: String::new(),
            head_hash: String::new(),
        };
        head.seal()?;
        Ok(head)
    }

    fn validate_identity(
        &self,
        prepared: &Protocol19PreparedInstallHeadV1,
        active_head_hash: &str,
    ) -> Result<(), Protocol19InstallError> {
        let cells_known = self
            .cells
            .keys()
            .all(|cell_id| prepared.cells.iter().any(|cell| &cell.cell_id == cell_id));
        let pending_known = self.pending_append.as_ref().is_none_or(|pending| {
            prepared
                .cells
                .iter()
                .any(|cell| cell.cell_id == pending.cell_id)
                && self.cells.get(&pending.cell_id) == pending.prior.as_ref()
                && pending.next.last_record_hash() == pending.record.record_hash()
                && pending
                    .prior
                    .as_ref()
                    .map_or(pending.next.entry_count() == 1, |prior| {
                        pending.next.is_direct_successor_of(prior)
                    })
        });
        if self.schema_version != UNIVERSE_LIFECYCLE_HEAD_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.universe_id != prepared.universe_id
            || self.manifest_hash != prepared.target_manifest_hash
            || self.active_head_hash != active_head_hash
            || self.prepared_install_head_hash != prepared.head_hash
            || self.cell_set_root != prepared.cell_set_root
            || self.lifecycle_genesis_root != lifecycle_genesis_root(prepared)?
            || self.universe_lifecycle_revision == 0
            || (self.universe_lifecycle_revision == 1) != self.previous_head_hash.is_empty()
            || !cells_known
            || !pending_known
            || !valid_hash(&self.head_hash)
            || (!self.previous_head_hash.is_empty() && !valid_hash(&self.previous_head_hash))
            || self.head_hash != self.calculate_hash()?
        {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle head is invalid or belongs to another activation".into(),
            ));
        }
        for (cell_id, commitment) in &self.cells {
            let genesis = prepared
                .cells
                .iter()
                .find(|cell| &cell.cell_id == cell_id)
                .expect("known lifecycle cell has prepared evidence");
            commitment
                .validate_identity(&genesis.lifecycle_record_hash, active_head_hash)
                .map_err(Protocol19InstallError::from)?;
        }
        if let Some(pending) = &self.pending_append {
            let genesis = prepared
                .cells
                .iter()
                .find(|cell| cell.cell_id == pending.cell_id)
                .expect("known pending lifecycle cell has prepared evidence");
            pending
                .next
                .validate_identity(&genesis.lifecycle_record_hash, active_head_hash)?;
            pending.record.validate_universe_identity(
                &pending.cell_id,
                &genesis.lifecycle_record_hash,
                active_head_hash,
            )?;
            if let Some(prior) = &pending.prior {
                prior.validate_identity(&genesis.lifecycle_record_hash, active_head_hash)?;
            }
        }
        Ok(())
    }

    fn begin_append(
        &self,
        cell_id: &str,
        prepared: &PreparedLifecycleAppendV2,
    ) -> Result<Self, Protocol19InstallError> {
        if self.pending_append.is_some() {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle already has a pending materialization".into(),
            ));
        }
        let prior = self.cells.get(cell_id).cloned();
        let next = prepared.next_commitment();
        if prior.as_ref().map_or(next.entry_count() != 1, |prior| {
            !next.is_direct_successor_of(prior)
        }) {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle append is not one exact child successor".into(),
            ));
        }
        let mut head = self.successor()?;
        head.pending_append = Some(PendingUniverseLifecycleAppendV1 {
            cell_id: cell_id.to_owned(),
            prior,
            record: prepared.record().clone(),
            next,
        });
        head.seal()?;
        Ok(head)
    }

    fn finish_append(&self) -> Result<Self, Protocol19InstallError> {
        let pending = self.pending_append.as_ref().ok_or_else(|| {
            Protocol19InstallError::Invalid(
                "universe lifecycle has no pending materialization".into(),
            )
        })?;
        let mut head = self.successor()?;
        head.cells
            .insert(pending.cell_id.clone(), pending.next.clone());
        head.pending_append = None;
        head.seal()?;
        Ok(head)
    }

    fn successor(&self) -> Result<Self, Protocol19InstallError> {
        let mut head = self.clone();
        head.universe_lifecycle_revision = self
            .universe_lifecycle_revision
            .checked_add(1)
            .ok_or_else(|| {
                Protocol19InstallError::Invalid("universe lifecycle revision is exhausted".into())
            })?;
        head.previous_head_hash.clone_from(&self.head_hash);
        head.head_hash.clear();
        Ok(head)
    }

    fn seal(&mut self) -> Result<(), Protocol19InstallError> {
        self.head_hash.clear();
        self.head_hash = self.calculate_hash()?;
        Ok(())
    }

    fn calculate_hash(&self) -> Result<String, Protocol19InstallError> {
        let mut material = self.clone();
        material.head_hash.clear();
        hash_json(UNIVERSE_LIFECYCLE_HEAD_HASH_DOMAIN, &material)
    }
}

#[derive(Debug)]
struct UniverseLifecycleStoreV1 {
    path: PathBuf,
    head: UniverseLifecycleHeadV1,
    poisoned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UniverseLifecyclePersistFailpointV1 {
    HeadSyncedBeforeRename,
    HeadRenamedBeforeMemory,
}

impl UniverseLifecycleStoreV1 {
    fn read_if_present(
        universe_root: &Path,
        prepared: &Protocol19PreparedInstallHeadV1,
        active_head_hash: &str,
    ) -> Result<Option<UniverseLifecycleHeadV1>, Protocol19InstallError> {
        let path = universe_root
            .join(ACTIVATION_DIRECTORY)
            .join(UNIVERSE_LIFECYCLE_HEAD_FILE);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(io_error(&path, source)),
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle head is not a regular file".into(),
            ));
        }
        let bytes = read_bounded(&path, MAX_UNIVERSE_LIFECYCLE_HEAD_BYTES)?;
        let head: UniverseLifecycleHeadV1 = serde_json::from_slice(&bytes)
            .map_err(|source| Protocol19InstallError::Json(source.to_string()))?;
        if serde_json::to_vec(&head)
            .map_err(|source| Protocol19InstallError::Json(source.to_string()))?
            != bytes
        {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle head bytes are not canonical".into(),
            ));
        }
        head.validate_identity(prepared, active_head_hash)?;
        Ok(Some(head))
    }

    fn open_or_bootstrap(
        universe_root: &Path,
        prepared: &Protocol19PreparedInstallHeadV1,
        active_head_hash: &str,
    ) -> Result<Self, Protocol19InstallError> {
        let activation_root = universe_root.join(ACTIVATION_DIRECTORY);
        cleanup_universe_lifecycle_temps(&activation_root)?;
        let path = activation_root.join(UNIVERSE_LIFECYCLE_HEAD_FILE);
        let head =
            if let Some(head) = Self::read_if_present(universe_root, prepared, active_head_hash)? {
                head
            } else {
                let head = UniverseLifecycleHeadV1::initial(prepared, active_head_hash)?;
                atomic_write(
                    &path,
                    &serde_json::to_vec(&head)
                        .map_err(|source| Protocol19InstallError::Json(source.to_string()))?,
                )?;
                head
            };
        Ok(Self {
            path,
            head,
            poisoned: false,
        })
    }

    fn persist(
        &mut self,
        next: UniverseLifecycleHeadV1,
        failpoint: Option<UniverseLifecyclePersistFailpointV1>,
    ) -> Result<(), Protocol19InstallError> {
        if self.poisoned {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle write outcome is uncertain; reopen before retry".into(),
            ));
        }
        let persisted = read_bounded(&self.path, MAX_UNIVERSE_LIFECYCLE_HEAD_BYTES)?;
        let expected = serde_json::to_vec(&self.head)
            .map_err(|source| Protocol19InstallError::Json(source.to_string()))?;
        if persisted != expected {
            self.poisoned = true;
            return Err(Protocol19InstallError::Invalid(
                "persisted universe lifecycle head diverged from the locked in-memory tip".into(),
            ));
        }
        let bytes = serde_json::to_vec(&next)
            .map_err(|source| Protocol19InstallError::Json(source.to_string()))?;
        if bytes.len() > MAX_UNIVERSE_LIFECYCLE_HEAD_BYTES {
            return Err(Protocol19InstallError::TooLarge(self.path.clone()));
        }
        if let Err(error) = atomic_write_universe_lifecycle_head(&self.path, &bytes, failpoint) {
            self.poisoned = true;
            return Err(error);
        }
        self.head = next;
        Ok(())
    }

    fn begin_append(
        &mut self,
        cell_id: &str,
        prepared: &PreparedLifecycleAppendV2,
        failpoint: Option<UniverseLifecyclePersistFailpointV1>,
    ) -> Result<(), Protocol19InstallError> {
        let next = self.head.begin_append(cell_id, prepared)?;
        self.persist(next, failpoint)
    }

    fn finish_append(
        &mut self,
        failpoint: Option<UniverseLifecyclePersistFailpointV1>,
    ) -> Result<(), Protocol19InstallError> {
        let next = self.head.finish_append()?;
        self.persist(next, failpoint)
    }
}

fn cleanup_universe_lifecycle_temps(root: &Path) -> Result<(), Protocol19InstallError> {
    let prefix = format!(".{UNIVERSE_LIFECYCLE_HEAD_FILE}.tmp-");
    let mut paths = Vec::new();
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(uuid) = name.strip_prefix(&prefix) else {
            continue;
        };
        if Uuid::parse_str(uuid).is_err()
            || !entry
                .file_type()
                .map_err(|source| io_error(entry.path(), source))?
                .is_file()
        {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle staging debris is malformed".into(),
            ));
        }
        paths.push(entry.path());
    }
    if paths.len() > 64 {
        return Err(Protocol19InstallError::Invalid(
            "universe lifecycle staging debris exceeds its bound".into(),
        ));
    }
    for path in paths {
        fs::remove_file(&path).map_err(|source| io_error(&path, source))?;
    }
    sync_directory(root)
}

fn lifecycle_genesis_root(
    prepared: &Protocol19PreparedInstallHeadV1,
) -> Result<String, Protocol19InstallError> {
    let bindings: Vec<_> = prepared
        .cells
        .iter()
        .map(|cell| UniverseLifecycleGenesisBindingV1 {
            cell_id: cell.cell_id.clone(),
            lifecycle_genesis_hash: cell.lifecycle_record_hash.clone(),
        })
        .collect();
    hash_json(UNIVERSE_LIFECYCLE_GENESIS_ROOT_DOMAIN, &bindings)
}

fn reconcile_universe_lifecycle(
    universe: &mut UniverseLifecycleStoreV1,
    cells: &mut BTreeMap<String, DraftWorld21Store>,
    lifecycle_states: &mut BTreeMap<String, LifecycleRuntimePreflightV2>,
    directory: &DraftCellDirectoryHistoryStoreV3,
    active_head_hash: &str,
    initialization_failpoint: &mut Option<LifecycleInitializationFailpointV2>,
) -> Result<(), Protocol19InstallError> {
    if let Some(pending) = universe.head.pending_append.clone() {
        let state = lifecycle_states.get(&pending.cell_id).ok_or_else(|| {
            Protocol19InstallError::Invalid(
                "universe lifecycle pending append names an unknown cell".into(),
            )
        })?;
        let assignment = directory.current_cell_assignment(&pending.cell_id)?;
        let cell = cells
            .get_mut(&pending.cell_id)
            .expect("preflighted cell exists");
        let materialized = if let Some(preflight_commitment) = state.committed() {
            let current =
                cell.open_or_initialize_lifecycle(active_head_hash, &assignment, directory)?;
            if &current != preflight_commitment {
                return Err(Protocol19InstallError::Invalid(
                    "mutable lifecycle open differs from its preflight commitment".into(),
                ));
            }
            if current == pending.next {
                current
            } else if pending.prior.as_ref() == Some(&current) {
                let prepared = cell.prepare_lifecycle_append(pending.record.clone(), directory)?;
                if prepared.next_commitment() != pending.next {
                    return Err(Protocol19InstallError::Invalid(
                        "pending universe lifecycle successor is not deterministic".into(),
                    ));
                }
                cell.materialize_lifecycle_append(prepared, directory)?;
                cell.lifecycle_commitment()?
            } else {
                return Err(Protocol19InstallError::Invalid(
                    "cell lifecycle tip differs from the universe pending append".into(),
                ));
            }
        } else {
            if pending.prior.is_some() || !state.is_uncommitted_bootstrap() {
                return Err(Protocol19InstallError::Invalid(
                    "committed cell lifecycle history is missing".into(),
                ));
            }
            let prepared = cell.prepare_initial_lifecycle_append(active_head_hash, &assignment)?;
            if prepared.record() != &pending.record || prepared.next_commitment() != pending.next {
                return Err(Protocol19InstallError::Invalid(
                    "pending initial lifecycle append differs from immutable genesis".into(),
                ));
            }
            if let Some(failpoint) = initialization_failpoint.take() {
                cell.initialize_prepared_lifecycle_with_failpoint(
                    active_head_hash,
                    prepared,
                    directory,
                    failpoint,
                )?
            } else {
                cell.initialize_prepared_lifecycle(active_head_hash, prepared, directory)?
            }
        };
        if materialized != pending.next {
            return Err(Protocol19InstallError::Invalid(
                "cell lifecycle did not materialize the universe commitment".into(),
            ));
        }
        universe.finish_append(None)?;
        lifecycle_states.insert(
            pending.cell_id,
            LifecycleRuntimePreflightV2::Committed(materialized),
        );
    }

    let cell_ids: Vec<_> = cells.keys().cloned().collect();
    for cell_id in cell_ids {
        let state = lifecycle_states
            .get(&cell_id)
            .expect("preflight recorded every activated cell");
        let committed = universe.head.cells.get(&cell_id).cloned();
        match (committed, state) {
            (Some(expected), LifecycleRuntimePreflightV2::Committed(preflight_commitment)) => {
                let assignment = directory.current_cell_assignment(&cell_id)?;
                let actual = cells
                    .get_mut(&cell_id)
                    .expect("preflighted cell exists")
                    .open_or_initialize_lifecycle(active_head_hash, &assignment, directory)?;
                if actual != expected || actual != *preflight_commitment {
                    return Err(Protocol19InstallError::Invalid(
                        "cell lifecycle tip differs from the universe commitment".into(),
                    ));
                }
            }
            (Some(_), _) => {
                return Err(Protocol19InstallError::Invalid(
                    "universe-committed cell lifecycle artifacts are missing".into(),
                ));
            }
            (None, LifecycleRuntimePreflightV2::Committed(_)) => {
                return Err(Protocol19InstallError::Invalid(
                    "cell lifecycle exists without universe authorization".into(),
                ));
            }
            (None, LifecycleRuntimePreflightV2::Absent) => {
                let assignment = directory.current_cell_assignment(&cell_id)?;
                let prepared = cells
                    .get(&cell_id)
                    .expect("preflighted cell exists")
                    .prepare_initial_lifecycle_append(active_head_hash, &assignment)?;
                universe.begin_append(&cell_id, &prepared, None)?;
                let cell = cells.get_mut(&cell_id).expect("preflighted cell exists");
                let actual = if let Some(failpoint) = initialization_failpoint.take() {
                    cell.initialize_prepared_lifecycle_with_failpoint(
                        active_head_hash,
                        prepared,
                        directory,
                        failpoint,
                    )?
                } else {
                    cell.initialize_prepared_lifecycle(active_head_hash, prepared, directory)?
                };
                let expected = universe
                    .head
                    .pending_append
                    .as_ref()
                    .expect("begin append persisted pending state")
                    .next
                    .clone();
                if actual != expected {
                    return Err(Protocol19InstallError::Invalid(
                        "initial lifecycle materialization differs from authorization".into(),
                    ));
                }
                universe.finish_append(None)?;
                lifecycle_states.insert(cell_id, LifecycleRuntimePreflightV2::Committed(actual));
            }
            (
                None,
                LifecycleRuntimePreflightV2::EmptyHistoryOnly
                | LifecycleRuntimePreflightV2::EmptyPair,
            ) => {
                return Err(Protocol19InstallError::Invalid(
                    "interrupted lifecycle bootstrap lacks a pending universe authorization".into(),
                ));
            }
        }
    }
    if universe.head.pending_append.is_some() || universe.head.cells.len() != cells.len() {
        return Err(Protocol19InstallError::Invalid(
            "universe lifecycle bootstrap is incomplete".into(),
        ));
    }
    Ok(())
}

fn validate_preflight_lifecycle_commitments(
    universe: Option<&UniverseLifecycleHeadV1>,
    cells: &BTreeMap<String, LifecycleRuntimePreflightV2>,
) -> Result<(), Protocol19InstallError> {
    let Some(universe) = universe else {
        if cells.values().any(|state| !state.is_absent()) {
            return Err(Protocol19InstallError::Invalid(
                "cell lifecycle artifacts exist without a universe lifecycle commitment".into(),
            ));
        }
        return Ok(());
    };
    for (cell_id, actual) in cells {
        let committed = universe.cells.get(cell_id);
        let accepted = if universe
            .pending_append
            .as_ref()
            .is_some_and(|pending| pending.cell_id == *cell_id)
        {
            let pending = universe
                .pending_append
                .as_ref()
                .expect("pending was checked");
            match (pending.prior.as_ref(), actual) {
                (None, state) if state.is_uncommitted_bootstrap() => true,
                (prior, LifecycleRuntimePreflightV2::Committed(actual)) => {
                    Some(actual) == prior || actual == &pending.next
                }
                _ => false,
            }
        } else {
            match (committed, actual) {
                (None, LifecycleRuntimePreflightV2::Absent) => true,
                (Some(expected), LifecycleRuntimePreflightV2::Committed(actual)) => {
                    actual == expected
                }
                _ => false,
            }
        };
        if !accepted {
            return Err(Protocol19InstallError::Invalid(
                "cell lifecycle materialization differs from the universe write-ahead head".into(),
            ));
        }
    }
    if universe
        .cells
        .keys()
        .any(|cell_id| !cells.contains_key(cell_id))
    {
        return Err(Protocol19InstallError::Invalid(
            "universe lifecycle commitment names an unknown cell".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Protocol19LifecycleCoordinatorFailpoint {
    UniverseLifecycleWriteAheadCommitted,
    UniverseLifecycleBeginSyncedBeforeRename,
    UniverseLifecycleBeginRenamedBeforeMemory,
    UniverseLifecycleFinishSyncedBeforeRename,
    UniverseLifecycleFinishRenamedBeforeMemory,
    DirectoryAuthorityCommitted,
    AuthorityFinalized,
    DirectoryReleaseCommitted,
}

const INSTALL_DIRECTORY: &str = "protocol-19-prepared-install-v1";
const INSTALL_LOCK_FILE: &str = "writer.lock";
const INSTALL_HEAD_FILE: &str = "prepared-install-v1.head.json";
const RECEIPT_FILE: &str = "migration-receipt-v1.json";
const SOURCE_DIRECTORY_ARCHIVE_FILE: &str = "source-directory-v2.archive.json";
const IDENTITY_MAP_FILE: &str = "identity-map-v1.json";
const PRODUCTION_ORIGIN_FILE: &str = "production-origin-v1.json";
const TARGET_MANIFEST_FILE: &str = "manifest-v5.json";
const TARGET_DIRECTORY_FILE: &str = "directory-v3.genesis.json";
const TARGET_DIRECTORY_HISTORY_FILE: &str = "directory-v3.genesis-history.json";
const INSTALL_HEAD_SCHEMA_VERSION: u32 = 1;
const INSTALL_HEAD_HASH_DOMAIN: &[u8] = b"the-verse/protocol-19-prepared-install-head/v1\0";
const INSTALL_CELL_SET_HASH_DOMAIN: &[u8] = b"the-verse/protocol-19-prepared-install-cell-set/v1\0";
const MAX_INSTALL_HEAD_BYTES: usize = 4 * 1_024 * 1_024;
const MAX_INSTALL_ARTIFACT_BYTES: usize = 256 * 1_024 * 1_024;
const MAX_INSTALL_CELLS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PreparedInstallMode {
    StagedUnactivated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Protocol19PreparedInstallHeadV1 {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    mode: PreparedInstallMode,
    universe_id: String,
    world_seed: String,
    target_manifest_hash: String,
    source_directory_document_hash: String,
    source_directory_archive_hash: String,
    migration_anchor_hash: String,
    migration_receipt_hash: String,
    target_directory_revision: u64,
    target_directory_document_hash: String,
    target_directory_history_entry_hash: String,
    target_assignment_root: String,
    target_placement_root: String,
    identity_map_root: String,
    production_origin_root: String,
    global_conservation_root: String,
    normalized_gameplay_root: String,
    cell_count: u64,
    cell_set_root: String,
    cells: Vec<PreparedWorld21CellEvidence>,
    head_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Protocol19PreparedInstallSummary {
    pub(crate) compatibility: Protocol19CompatibilityTuple,
    pub(crate) universe_id: String,
    pub(crate) world_seed: u64,
    pub(crate) target_manifest_hash: String,
    pub(crate) migration_anchor_hash: String,
    pub(crate) migration_receipt_hash: String,
    pub(crate) target_directory_document_hash: String,
    pub(crate) target_assignment_root: String,
    pub(crate) target_placement_root: String,
    pub(crate) identity_map_root: String,
    pub(crate) production_origin_root: String,
    pub(crate) global_conservation_root: String,
    pub(crate) normalized_gameplay_root: String,
    pub(crate) cell_count: u64,
    pub(crate) cell_set_root: String,
    pub(crate) prepared_install_head_hash: String,
}

impl Protocol19PreparedInstallHeadV1 {
    fn new(
        receipt: &ValidatedProtocol19MigrationReceipt<'_, '_>,
        directory: &ValidatedProtocol19DirectoryGenesis<'_, '_>,
        cells: Vec<PreparedWorld21CellEvidence>,
    ) -> Result<Self, Protocol19InstallError> {
        let transform = receipt.transform();
        let source = transform.source();
        let cell_count = u64::try_from(cells.len()).map_err(|_| {
            Protocol19InstallError::Invalid("prepared cell count overflowed".into())
        })?;
        let cell_set_root = hash_json(INSTALL_CELL_SET_HASH_DOMAIN, &cells)?;
        let mut head = Self {
            schema_version: INSTALL_HEAD_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            mode: PreparedInstallMode::StagedUnactivated,
            universe_id: source.universe_id().to_owned(),
            world_seed: source.world_seed().to_string(),
            target_manifest_hash: transform.target_manifest_hash().to_owned(),
            source_directory_document_hash: source.directory_document_hash().to_owned(),
            source_directory_archive_hash: receipt.source_directory_archive_hash().to_owned(),
            migration_anchor_hash: receipt.anchor_hash().to_owned(),
            migration_receipt_hash: receipt.receipt_hash().to_owned(),
            target_directory_revision: directory.directory_revision(),
            target_directory_document_hash: directory.document_hash().to_owned(),
            target_directory_history_entry_hash: directory.history_entry_hash().to_owned(),
            target_assignment_root: directory.assignment_root().to_owned(),
            target_placement_root: directory.placement_root().to_owned(),
            identity_map_root: transform.identity_map_root().to_owned(),
            production_origin_root: transform.production_origin_root().to_owned(),
            global_conservation_root: transform.global_conservation_root().to_owned(),
            normalized_gameplay_root: transform.normalized_gameplay_root().to_owned(),
            cell_count,
            cell_set_root,
            cells,
            head_hash: String::new(),
        };
        head.head_hash = head.calculate_hash()?;
        head.validate()?;
        Ok(head)
    }

    fn calculate_hash(&self) -> Result<String, Protocol19InstallError> {
        let mut material = self.clone();
        material.head_hash.clear();
        hash_json(INSTALL_HEAD_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), Protocol19InstallError> {
        let seed = self.world_seed.parse::<u64>().map_err(|_| {
            Protocol19InstallError::Invalid("prepared world seed is not canonical".into())
        })?;
        let manifest = crate::manifest_v5::build_validated_manifest_v5(seed)
            .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
        let ordered = !self.cells.is_empty()
            && self
                .cells
                .windows(2)
                .all(|pair| pair[0].cell_id < pair[1].cell_id);
        let cells_valid = self.cells.iter().all(|cell| {
            crate::celestial::cell_id(&cell.cell_key).is_ok_and(|id| id == cell.cell_id)
                && cell.cell_key.universe_id == self.universe_id
                && cell.migration_receipt_hash == self.migration_receipt_hash
                && all_hashes([
                    &cell.initialization_head_hash,
                    &cell.identity_hash,
                    &cell.lifecycle_record_hash,
                    &cell.snapshot_state_hash,
                    &cell.active_world_hash,
                    &cell.migration_receipt_hash,
                ])
        });
        if self.schema_version != INSTALL_HEAD_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.mode != PreparedInstallMode::StagedUnactivated
            || seed.to_string() != self.world_seed
            || self.universe_id != manifest.universe_id()
            || self.target_manifest_hash != manifest.manifest_hash()
            || self.target_directory_revision != 1
            || usize::try_from(self.cell_count).ok() != Some(self.cells.len())
            || self.cells.len() > MAX_INSTALL_CELLS
            || !ordered
            || !cells_valid
            || self.cell_set_root != hash_json(INSTALL_CELL_SET_HASH_DOMAIN, &self.cells)?
            || !all_hashes([
                &self.target_manifest_hash,
                &self.source_directory_document_hash,
                &self.source_directory_archive_hash,
                &self.migration_anchor_hash,
                &self.migration_receipt_hash,
                &self.target_directory_document_hash,
                &self.target_directory_history_entry_hash,
                &self.target_assignment_root,
                &self.target_placement_root,
                &self.identity_map_root,
                &self.production_origin_root,
                &self.global_conservation_root,
                &self.normalized_gameplay_root,
                &self.cell_set_root,
                &self.head_hash,
            ])
            || self.head_hash != self.calculate_hash()?
        {
            return Err(Protocol19InstallError::Invalid(
                "prepared-install head is not one canonical unactivated universe".into(),
            ));
        }
        Ok(())
    }

    fn encode_canonical(&self) -> Result<Vec<u8>, Protocol19InstallError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|source| Protocol19InstallError::Json(source.to_string()))?;
        if bytes.is_empty() || bytes.len() > MAX_INSTALL_HEAD_BYTES {
            return Err(Protocol19InstallError::TooLarge(PathBuf::from(
                INSTALL_HEAD_FILE,
            )));
        }
        Ok(bytes)
    }

    fn decode_canonical(bytes: &[u8]) -> Result<Self, Protocol19InstallError> {
        if bytes.is_empty() || bytes.len() > MAX_INSTALL_HEAD_BYTES {
            return Err(Protocol19InstallError::TooLarge(PathBuf::from(
                INSTALL_HEAD_FILE,
            )));
        }
        let head = serde_json::from_slice::<Self>(bytes)
            .map_err(|source| Protocol19InstallError::Json(source.to_string()))?;
        head.validate()?;
        if head.encode_canonical()? != bytes {
            return Err(Protocol19InstallError::Invalid(
                "prepared-install head bytes are not canonical".into(),
            ));
        }
        Ok(head)
    }

    fn summary(&self) -> Result<Protocol19PreparedInstallSummary, Protocol19InstallError> {
        let world_seed = self.world_seed.parse::<u64>().map_err(|_| {
            Protocol19InstallError::Invalid("prepared world seed is not canonical".into())
        })?;
        Ok(Protocol19PreparedInstallSummary {
            compatibility: self.compatibility.clone(),
            universe_id: self.universe_id.clone(),
            world_seed,
            target_manifest_hash: self.target_manifest_hash.clone(),
            migration_anchor_hash: self.migration_anchor_hash.clone(),
            migration_receipt_hash: self.migration_receipt_hash.clone(),
            target_directory_document_hash: self.target_directory_document_hash.clone(),
            target_assignment_root: self.target_assignment_root.clone(),
            target_placement_root: self.target_placement_root.clone(),
            identity_map_root: self.identity_map_root.clone(),
            production_origin_root: self.production_origin_root.clone(),
            global_conservation_root: self.global_conservation_root.clone(),
            normalized_gameplay_root: self.normalized_gameplay_root.clone(),
            cell_count: self.cell_count,
            cell_set_root: self.cell_set_root.clone(),
            prepared_install_head_hash: self.head_hash.clone(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Protocol19InstallFailpoint {
    InstallNamespaceSynced,
    ArtifactsSynced,
    DirectoryGenesisSynced,
    CellStaged(usize),
    HeadTempSyncedBeforeRename,
    HeadRenamedBeforeDirectorySync,
    HeadDirectorySyncedBeforeMemory,
}

#[derive(Debug, Error)]
pub(crate) enum Protocol19InstallError {
    #[error("protocol-19 prepared installation is invalid: {0}")]
    Invalid(String),
    #[error("protocol-19 prepared installation JSON is invalid: {0}")]
    Json(String),
    #[error("protocol-19 prepared installation artifact is too large: {0}")]
    TooLarge(PathBuf),
    #[error("protocol-19 prepared installation I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("another protocol-19 prepared-install writer is active")]
    WriterConflict,
    #[error("protocol-19 prepared installation injected failure: {0:?}")]
    Injected(Protocol19InstallFailpoint),
    #[error(transparent)]
    Directory(#[from] CellDirectoryError),
    #[error(transparent)]
    Migration(#[from] MigrationReceiptError),
    #[error(transparent)]
    Cell(#[from] DraftWorld21StoreError),
    #[error(transparent)]
    Lifecycle(#[from] LifecycleV2Error),
}

/// Held proof that the complete target set is durably prepared. The borrow
/// keeps the frozen source and transform alive; no method grants activation.
#[derive(Debug)]
pub(crate) struct PreparedProtocol19World<'migration, 'source> {
    head: Protocol19PreparedInstallHeadV1,
    universe_root: PathBuf,
    _directory: DraftCellDirectoryHistoryStoreV3,
    _cells: Vec<DraftWorld21Store>,
    _install_lock: File,
    _migration: PhantomData<&'migration ValidatedProtocol19MigrationTransform<'source>>,
}

impl PreparedProtocol19World<'_, '_> {
    pub(crate) fn head_hash(&self) -> &str {
        &self.head.head_hash
    }

    pub(crate) fn receipt_hash(&self) -> &str {
        &self.head.migration_receipt_hash
    }

    pub(crate) const fn cell_count(&self) -> u64 {
        self.head.cell_count
    }

    pub(crate) fn summary(
        &self,
    ) -> Result<Protocol19PreparedInstallSummary, Protocol19InstallError> {
        self.head.summary()
    }

    pub(crate) fn universe_root(&self) -> &Path {
        &self.universe_root
    }
}

#[derive(Debug)]
pub(crate) struct ValidatedProtocol19PreparedInstall {
    universe_root: PathBuf,
    install_lock: File,
    head: Protocol19PreparedInstallHeadV1,
    summary: Protocol19PreparedInstallSummary,
    receipt: CanonicalProtocol19MigrationReceiptEvidence,
    manifest: ValidatedUniverseManifestV5,
    directory_document_bytes: Vec<u8>,
    directory_history_bytes: Vec<u8>,
    cell_ids: BTreeSet<String>,
}

impl ValidatedProtocol19PreparedInstall {
    pub(crate) fn summary(&self) -> &Protocol19PreparedInstallSummary {
        &self.summary
    }

    pub(crate) fn receipt(&self) -> &CanonicalProtocol19MigrationReceiptEvidence {
        &self.receipt
    }

    pub(crate) fn open(
        self,
        active_head_hash: &str,
    ) -> Result<OpenedProtocol19PreparedInstall, Protocol19InstallError> {
        self.open_with_lifecycle_initialization_failpoint(active_head_hash, None)
    }

    pub(crate) fn open_with_lifecycle_initialization_failpoint(
        self,
        active_head_hash: &str,
        mut initialization_failpoint: Option<LifecycleInitializationFailpointV2>,
    ) -> Result<OpenedProtocol19PreparedInstall, Protocol19InstallError> {
        for expected in &self.head.cells {
            DraftWorld21Store::preflight_active_file_set(
                self.universe_root.join("cells").join(&expected.cell_id),
            )?;
        }
        let directory = DraftCellDirectoryHistoryStoreV3::open_from_active_head(
            &self.universe_root,
            ActiveDirectoryV3Expectation {
                universe_id: &self.head.universe_id,
                manifest_hash: &self.head.target_manifest_hash,
                revision: self.head.target_directory_revision,
                document_hash: &self.head.target_directory_document_hash,
                history_entry_hash: &self.head.target_directory_history_entry_hash,
                assignment_root: &self.head.target_assignment_root,
                placement_root: &self.head.target_placement_root,
                document_bytes: &self.directory_document_bytes,
                history_entry_bytes: &self.directory_history_bytes,
            },
        )?;

        // Parse and replay every selected cell without repairing any cell
        // artifact. A corrupt canonical-named artifact in a later cell must
        // fail before an earlier cell can initialize or recover lifecycle
        // state.
        let mut lifecycle_commitments = BTreeMap::new();
        for (expected, receipt_cell) in self.head.cells.iter().zip(&self.receipt.target_cells) {
            let commitment = DraftWorld21Store::preflight_active_contents(
                self.universe_root.join("cells").join(&expected.cell_id),
                &self.manifest,
                &self.head.migration_anchor_hash,
                active_head_hash,
                expected,
                &receipt_cell.production_origin_root,
                &receipt_cell.identity_subset_root,
                &directory,
            )?;
            lifecycle_commitments.insert(expected.cell_id.clone(), commitment);
        }
        let preflight_universe_lifecycle = UniverseLifecycleStoreV1::read_if_present(
            &self.universe_root,
            &self.head,
            active_head_hash,
        )?;
        validate_preflight_lifecycle_commitments(
            preflight_universe_lifecycle.as_ref(),
            &lifecycle_commitments,
        )?;
        let mut lifecycle_states = lifecycle_commitments;

        let mut cells = BTreeMap::new();
        for (expected, receipt_cell) in self.head.cells.iter().zip(&self.receipt.target_cells) {
            let cell = DraftWorld21Store::open_from_active_head(
                self.universe_root.join("cells").join(&expected.cell_id),
                &self.manifest,
                &self.head.migration_anchor_hash,
                active_head_hash,
                expected,
                &receipt_cell.production_origin_root,
                &receipt_cell.identity_subset_root,
                &directory,
            )?;
            if cells.insert(expected.cell_id.clone(), cell).is_some() {
                return Err(Protocol19InstallError::Invalid(
                    "activated prepared install contains duplicate cell identity".into(),
                ));
            }
        }
        let mut universe_lifecycle = UniverseLifecycleStoreV1::open_or_bootstrap(
            &self.universe_root,
            &self.head,
            active_head_hash,
        )?;
        reconcile_universe_lifecycle(
            &mut universe_lifecycle,
            &mut cells,
            &mut lifecycle_states,
            &directory,
            active_head_hash,
            &mut initialization_failpoint,
        )?;
        validate_exact_cell_namespace_ids(&self.universe_root, &self.cell_ids)?;
        Ok(OpenedProtocol19PreparedInstall {
            summary: self.summary,
            receipt: self.receipt,
            directory,
            cells,
            universe_lifecycle,
            #[cfg(test)]
            lifecycle_failpoint: None,
            _install_lock: self.install_lock,
        })
    }
}

#[derive(Debug)]
pub(crate) struct OpenedProtocol19PreparedInstall {
    summary: Protocol19PreparedInstallSummary,
    receipt: CanonicalProtocol19MigrationReceiptEvidence,
    directory: DraftCellDirectoryHistoryStoreV3,
    cells: BTreeMap<String, DraftWorld21Store>,
    universe_lifecycle: UniverseLifecycleStoreV1,
    #[cfg(test)]
    lifecycle_failpoint: Option<Protocol19LifecycleCoordinatorFailpoint>,
    _install_lock: File,
}

impl OpenedProtocol19PreparedInstall {
    fn append_committed_lifecycle(
        &mut self,
        cell_id: &str,
        record: LifecycleRecordV2,
    ) -> Result<(), Protocol19InstallError> {
        if self.universe_lifecycle.head.pending_append.is_some() {
            return Err(Protocol19InstallError::Invalid(
                "universe lifecycle has an unfinished materialization; reopen before retry".into(),
            ));
        }
        let prior = self
            .cells
            .get(cell_id)
            .ok_or_else(|| {
                Protocol19InstallError::Invalid(
                    "lifecycle append selected a cell outside the activated set".into(),
                )
            })?
            .lifecycle_commitment()?;
        if self.universe_lifecycle.head.cells.get(cell_id) != Some(&prior) {
            return Err(Protocol19InstallError::Invalid(
                "cell lifecycle tip differs from its universe commitment".into(),
            ));
        }
        let prepared = self
            .cells
            .get_mut(cell_id)
            .expect("activated lifecycle cell exists")
            .prepare_lifecycle_append(record, &self.directory)?;
        #[cfg(test)]
        let begin_failpoint = match self.lifecycle_failpoint {
            Some(
                Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleBeginSyncedBeforeRename,
            ) => {
                self.lifecycle_failpoint = None;
                Some(UniverseLifecyclePersistFailpointV1::HeadSyncedBeforeRename)
            }
            Some(
                Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleBeginRenamedBeforeMemory,
            ) => {
                self.lifecycle_failpoint = None;
                Some(UniverseLifecyclePersistFailpointV1::HeadRenamedBeforeMemory)
            }
            _ => None,
        };
        #[cfg(not(test))]
        let begin_failpoint = None;
        self.universe_lifecycle
            .begin_append(cell_id, &prepared, begin_failpoint)?;
        #[cfg(test)]
        if self.lifecycle_failpoint
            == Some(Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleWriteAheadCommitted)
        {
            self.lifecycle_failpoint = None;
            return Err(Protocol19InstallError::Invalid(
                "injected failure after universe lifecycle write-ahead commit".into(),
            ));
        }
        self.cells
            .get_mut(cell_id)
            .expect("activated lifecycle cell exists")
            .materialize_lifecycle_append(prepared, &self.directory)?;
        let materialized = self
            .cells
            .get(cell_id)
            .expect("activated lifecycle cell exists")
            .lifecycle_commitment()?;
        let expected = self
            .universe_lifecycle
            .head
            .pending_append
            .as_ref()
            .expect("write-ahead lifecycle append remains pending")
            .next
            .clone();
        if materialized != expected {
            return Err(Protocol19InstallError::Invalid(
                "cell lifecycle materialization differs from its write-ahead commitment".into(),
            ));
        }
        #[cfg(test)]
        let finish_failpoint = match self.lifecycle_failpoint {
            Some(
                Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleFinishSyncedBeforeRename,
            ) => {
                self.lifecycle_failpoint = None;
                Some(UniverseLifecyclePersistFailpointV1::HeadSyncedBeforeRename)
            }
            Some(
                Protocol19LifecycleCoordinatorFailpoint::UniverseLifecycleFinishRenamedBeforeMemory,
            ) => {
                self.lifecycle_failpoint = None;
                Some(UniverseLifecyclePersistFailpointV1::HeadRenamedBeforeMemory)
            }
            _ => None,
        };
        #[cfg(not(test))]
        let finish_failpoint = None;
        self.universe_lifecycle.finish_append(finish_failpoint)
    }

    #[cfg(test)]
    pub(crate) fn reseal_pending_lifecycle_outside_state_machine_for_test(
        &mut self,
    ) -> Result<(), Protocol19InstallError> {
        let pending = self
            .universe_lifecycle
            .head
            .pending_append
            .clone()
            .ok_or_else(|| {
                Protocol19InstallError::Invalid(
                    "test requires a pending universe lifecycle append".into(),
                )
            })?;
        let prepared = self
            .cells
            .get(&pending.cell_id)
            .ok_or_else(|| {
                Protocol19InstallError::Invalid(
                    "test pending lifecycle cell is outside the activated set".into(),
                )
            })?
            .prepare_resealed_non_state_machine_lifecycle_successor_for_test(pending.record)?;
        let mut replacement = self.universe_lifecycle.head.clone();
        let replacement_pending = replacement
            .pending_append
            .as_mut()
            .expect("test pending lifecycle append was checked");
        replacement_pending.record = prepared.record().clone();
        replacement_pending.next = prepared.next_commitment();
        replacement.seal()?;
        atomic_write(
            &self.universe_lifecycle.path,
            &serde_json::to_vec(&replacement)
                .map_err(|source| Protocol19InstallError::Json(source.to_string()))?,
        )?;
        self.universe_lifecycle.head = replacement;
        Ok(())
    }

    pub(crate) fn summary(&self) -> &Protocol19PreparedInstallSummary {
        &self.summary
    }

    pub(crate) fn receipt(&self) -> &CanonicalProtocol19MigrationReceiptEvidence {
        &self.receipt
    }

    #[cfg(test)]
    pub(crate) fn cell_assignment(
        &self,
        cell_key: &CellKeyV1,
    ) -> Result<&CellAssignmentRecord, CellDirectoryError> {
        self.directory.assignment(cell_key)
    }

    #[cfg(test)]
    pub(crate) fn claim_cell_authority(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        self.directory
            .claim_cell(cell_key, expected_generation, holder_id)
    }

    #[cfg(test)]
    pub(crate) fn recover_cell_authority(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        self.directory
            .recover_cell(cell_key, expected_generation, holder_id)
    }

    #[cfg(test)]
    pub(crate) fn release_cell_authority(
        &mut self,
        cell_key: &CellKeyV1,
        expected_generation: u64,
        holder_id: &str,
    ) -> Result<CellAssignmentRecord, CellDirectoryError> {
        self.directory
            .release_cell(cell_key, expected_generation, holder_id)
    }

    pub(crate) fn dispatch_background_production(
        &mut self,
        cell_key: &CellKeyV1,
        holder_id: &str,
        clock: &dyn TrustedClock,
    ) -> Result<Protocol19BackgroundDispatchOutcome, Protocol19InstallError> {
        let started = Instant::now();
        let mut committed_quanta = 0u64;
        let cell_id = crate::celestial::cell_id(cell_key)
            .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
        if !self.cells.contains_key(&cell_id) {
            return Err(Protocol19InstallError::Invalid(
                "background dispatch selected a cell outside the activated set".into(),
            ));
        }

        let trusted_now = clock
            .now_unix_ms()
            .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
        self.reconcile_or_acquire_background_authority(cell_key, &cell_id, holder_id, trusted_now)?;
        #[cfg(test)]
        if self.lifecycle_failpoint
            == Some(Protocol19LifecycleCoordinatorFailpoint::AuthorityFinalized)
        {
            self.lifecycle_failpoint = None;
            return Err(Protocol19InstallError::Invalid(
                "injected failure after lifecycle authority finalization".into(),
            ));
        }
        if self
            .cells
            .get(&cell_id)
            .expect("activated cell was checked")
            .lifecycle_record()?
            .observed_mode()
            == LifecycleModeV2::Sleeping
        {
            return Ok(Protocol19BackgroundDispatchOutcome {
                mode: "sleeping",
                committed_quanta: 0,
                acknowledged_production_sequence: self
                    .cells
                    .get(&cell_id)
                    .expect("activated cell was checked")
                    .lifecycle_record()?
                    .acknowledged_sequence(),
                next_scheduled_for_unix_ms: None,
            });
        }

        loop {
            committed_quanta = committed_quanta
                .checked_add(self.reconcile_pending_production(&cell_id, clock)?)
                .ok_or_else(|| {
                    Protocol19InstallError::Invalid("dispatch count exhausted".into())
                })?;
            let trusted_now = clock
                .now_unix_ms()
                .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;

            let next_occurrence = self
                .cells
                .get(&cell_id)
                .expect("activated cell was checked")
                .lifecycle_record()?
                .next_occurrence()
                .cloned();
            let next_occurrence = if let Some(occurrence) = next_occurrence {
                Some(occurrence)
            } else {
                let cell = self
                    .cells
                    .get(&cell_id)
                    .expect("activated cell was checked");
                if cell.background_production_is_runnable()? {
                    let scheduled = trusted_now
                        .checked_add(PRODUCTION_QUANTUM_MILLIS)
                        .ok_or_else(|| {
                            Protocol19InstallError::Invalid(
                                "production schedule time is exhausted".into(),
                            )
                        })?;
                    let occurrence = cell.next_production_occurrence_at(scheduled)?;
                    let assignment = self.directory.current_cell_assignment(&cell_id)?;
                    let frontier = cell.lifecycle_world_frontier()?;
                    let next = cell.lifecycle_record()?.renew_and_set_schedule(
                        &assignment,
                        trusted_now,
                        &frontier,
                        Some(occurrence.clone()),
                    )?;
                    self.append_committed_lifecycle(&cell_id, next)?;
                    Some(occurrence)
                } else {
                    None
                }
            };

            let Some(occurrence) = next_occurrence else {
                self.release_background_authority(cell_key, &cell_id, holder_id, trusted_now)?;
                break;
            };
            if occurrence.scheduled_for_unix_ms > trusted_now
                || committed_quanta >= MAX_BACKGROUND_QUANTA_PER_DISPATCH
                || started.elapsed() >= MAX_BACKGROUND_DISPATCH_TIME
            {
                break;
            }

            let assignment = self.directory.current_cell_assignment(&cell_id)?;
            let frontier = self
                .cells
                .get(&cell_id)
                .expect("activated cell was checked")
                .lifecycle_world_frontier()?;
            let pending = self
                .cells
                .get(&cell_id)
                .expect("activated cell was checked")
                .lifecycle_record()?
                .begin_world_commit(&assignment, trusted_now, &frontier)?;
            self.append_committed_lifecycle(&cell_id, pending)?;

            if started.elapsed() >= MAX_BACKGROUND_DISPATCH_TIME {
                break;
            }

            // The lifecycle fsync above may have taken arbitrarily long. Take
            // a new trusted-time sample and revalidate the lease at the exact
            // world-write boundary rather than reusing the earlier sample.
            let append_trusted_now = clock
                .now_unix_ms()
                .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
            let assignment = self.directory.current_cell_assignment(&cell_id)?;
            let frontier = self
                .cells
                .get(&cell_id)
                .expect("activated cell was checked")
                .lifecycle_world_frontier()?;
            self.cells
                .get(&cell_id)
                .expect("activated cell was checked")
                .lifecycle_record()?
                .require_event_append_authority(&assignment, append_trusted_now, &frontier)?;

            let authority = self.directory.current_cell_authority(&cell_id)?;
            self.cells
                .get_mut(&cell_id)
                .expect("activated cell was checked")
                .append_production_occurrence(occurrence, append_trusted_now, &authority)?;
            committed_quanta = committed_quanta.checked_add(1).ok_or_else(|| {
                Protocol19InstallError::Invalid("dispatch count exhausted".into())
            })?;
        }

        let lifecycle = self
            .cells
            .get(&cell_id)
            .expect("activated cell was checked")
            .lifecycle_record()?;
        Ok(Protocol19BackgroundDispatchOutcome {
            mode: match lifecycle.observed_mode() {
                LifecycleModeV2::Sleeping => "sleeping",
                LifecycleModeV2::Activating => "activating",
                LifecycleModeV2::Background => "background",
                LifecycleModeV2::Draining => "draining",
            },
            committed_quanta,
            acknowledged_production_sequence: lifecycle.acknowledged_sequence(),
            next_scheduled_for_unix_ms: lifecycle
                .next_occurrence()
                .map(|occurrence| occurrence.scheduled_for_unix_ms),
        })
    }

    #[cfg(test)]
    pub(crate) fn set_lifecycle_failpoint_for_test(
        &mut self,
        cell_key: &CellKeyV1,
        failpoint: crate::grid_handoff_v2::lifecycle_v2::LifecycleAppendFailpointV2,
    ) -> Result<(), Protocol19InstallError> {
        let cell_id = crate::celestial::cell_id(cell_key)
            .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
        self.cells
            .get_mut(&cell_id)
            .ok_or_else(|| {
                Protocol19InstallError::Invalid(
                    "lifecycle failpoint selected a cell outside the activated set".into(),
                )
            })?
            .set_lifecycle_failpoint_for_test(failpoint)?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_lifecycle_coordinator_failpoint_for_test(
        &mut self,
        failpoint: Protocol19LifecycleCoordinatorFailpoint,
    ) {
        self.lifecycle_failpoint = Some(failpoint);
    }

    fn reconcile_or_acquire_background_authority(
        &mut self,
        cell_key: &CellKeyV1,
        cell_id: &str,
        holder_id: &str,
        trusted_now: u64,
    ) -> Result<(), Protocol19InstallError> {
        if let Some(operation) = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_record()?
            .authority_operation()
            .cloned()
        {
            self.finish_authority_operation(cell_key, cell_id, &operation, trusted_now)?;
            if operation.holder_id() != holder_id {
                // Crash recovery completes the exact persisted transaction,
                // but this dispatch may never continue under another caller's
                // holder. Acquire or recover the requested holder explicitly.
                return self.reconcile_or_acquire_background_authority(
                    cell_key,
                    cell_id,
                    holder_id,
                    trusted_now,
                );
            }
            return Ok(());
        }

        let current = self.directory.current_cell_assignment(cell_id)?;
        let lifecycle = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_record()?
            .clone();
        let frontier = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_world_frontier()?;
        let kind = match current.assignment().state {
            CellAssignmentState::Sleeping => LifecycleAuthorityOperationKindV2::Claim,
            CellAssignmentState::Assigned
                if lifecycle.holder_id() == current.assignment().holder_id.as_deref()
                    && lifecycle.holder_id() == Some(holder_id)
                    && lifecycle
                        .authority_expires_at_unix_ms()
                        .is_some_and(|expiry| trusted_now < expiry) =>
            {
                if lifecycle.pending_world_commit().is_some() {
                    return Ok(());
                }
                let next_occurrence = lifecycle.next_occurrence().cloned();
                let renewed = lifecycle.renew_and_set_schedule(
                    &current,
                    trusted_now,
                    &frontier,
                    next_occurrence,
                )?;
                self.append_committed_lifecycle(cell_id, renewed)?;
                return Ok(());
            }
            CellAssignmentState::Assigned => LifecycleAuthorityOperationKindV2::Recovery,
            _ => {
                return Err(Protocol19InstallError::Invalid(
                    "cell directory is in a nonterminal authority mode".into(),
                ));
            }
        };
        let request =
            lifecycle.request_authority(&current, kind, holder_id, trusted_now, &frontier)?;
        self.append_committed_lifecycle(cell_id, request.clone())?;
        let operation = request
            .authority_operation()
            .expect("authority request contains its operation")
            .clone();
        self.finish_authority_operation(cell_key, cell_id, &operation, trusted_now)
    }

    fn finish_authority_operation(
        &mut self,
        cell_key: &CellKeyV1,
        cell_id: &str,
        operation: &crate::grid_handoff_v2::lifecycle_v2::LifecycleAuthorityOperationV2,
        trusted_now: u64,
    ) -> Result<(), Protocol19InstallError> {
        self.cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_record()?
            .preflight_authority_finalization(trusted_now)?;
        match operation.kind() {
            LifecycleAuthorityOperationKindV2::Claim => {
                self.directory.claim_cell(
                    cell_key,
                    operation.expected_assignment_generation(),
                    operation.holder_id(),
                )?;
            }
            LifecycleAuthorityOperationKindV2::Recovery => {
                self.directory.recover_cell(
                    cell_key,
                    operation.expected_assignment_generation(),
                    operation.holder_id(),
                )?;
            }
            LifecycleAuthorityOperationKindV2::Release => {
                self.directory.release_cell(
                    cell_key,
                    operation.expected_assignment_generation(),
                    operation.holder_id(),
                )?;
                #[cfg(test)]
                if self.lifecycle_failpoint
                    == Some(Protocol19LifecycleCoordinatorFailpoint::DirectoryReleaseCommitted)
                {
                    self.lifecycle_failpoint = None;
                    return Err(Protocol19InstallError::Invalid(
                        "injected failure after directory release commit".into(),
                    ));
                }
                let sleeping = self.directory.current_cell_assignment(cell_id)?;
                let cell = self.cells.get(cell_id).expect("activated cell was checked");
                let frontier = cell.lifecycle_world_frontier()?;
                let next = cell.lifecycle_record()?.finalize_sleeping(
                    &sleeping,
                    trusted_now,
                    &frontier,
                )?;
                self.append_committed_lifecycle(cell_id, next)?;
                return Ok(());
            }
        }
        #[cfg(test)]
        if self.lifecycle_failpoint
            == Some(Protocol19LifecycleCoordinatorFailpoint::DirectoryAuthorityCommitted)
        {
            self.lifecycle_failpoint = None;
            return Err(Protocol19InstallError::Invalid(
                "injected failure after directory authority commit".into(),
            ));
        }
        let assigned = self.directory.current_cell_assignment(cell_id)?;
        let cell = self.cells.get(cell_id).expect("activated cell was checked");
        let frontier = cell.lifecycle_world_frontier()?;
        let finalized =
            cell.lifecycle_record()?
                .finalize_assigned(&assigned, trusted_now, &frontier)?;
        self.append_committed_lifecycle(cell_id, finalized)?;
        Ok(())
    }

    fn reconcile_pending_production(
        &mut self,
        cell_id: &str,
        clock: &dyn TrustedClock,
    ) -> Result<u64, Protocol19InstallError> {
        let pending = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_record()?
            .pending_world_commit()
            .cloned();
        let Some(pending) = pending else {
            return Ok(0);
        };
        let frontier = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_world_frontier()?;
        let prior = frontier.event_sequence == pending.prior_event_sequence()
            && frontier.event_hash == pending.prior_event_hash()
            && frontier.state_hash == pending.prior_state_hash();
        let committed = frontier.event_sequence
            == pending.prior_event_sequence().checked_add(1).unwrap_or(0)
            && frontier.committed_production_sequence
                == pending.occurrence().production_quantum_sequence
            && frontier.last_scheduled_for_unix_ms == pending.occurrence().scheduled_for_unix_ms;
        if prior {
            let append_trusted_now = clock
                .now_unix_ms()
                .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
            let assignment = self.directory.current_cell_assignment(cell_id)?;
            self.cells
                .get(cell_id)
                .expect("activated cell was checked")
                .lifecycle_record()?
                .require_event_append_authority(&assignment, append_trusted_now, &frontier)?;
            let authority = self.directory.current_cell_authority(cell_id)?;
            self.cells
                .get_mut(cell_id)
                .expect("activated cell was checked")
                .append_production_occurrence(
                    pending.occurrence().clone(),
                    append_trusted_now,
                    &authority,
                )?;
        } else if !committed {
            return Err(Protocol19InstallError::Invalid(
                "pending production occurrence has a conflicting world result".into(),
            ));
        }
        let accepted_trusted_at = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .committed_production_accepted_trusted_at(pending.occurrence())?;
        let trusted_now = clock
            .now_unix_ms()
            .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
        require_production_ack_time(trusted_now, accepted_trusted_at)?;
        let frontier = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_world_frontier()?;
        let runnable = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .background_production_is_runnable()?;
        let next_occurrence = if runnable {
            let scheduled = pending
                .occurrence()
                .scheduled_for_unix_ms
                .checked_add(PRODUCTION_QUANTUM_MILLIS)
                .ok_or_else(|| {
                    Protocol19InstallError::Invalid("production schedule time is exhausted".into())
                })?;
            Some(
                self.cells
                    .get(cell_id)
                    .expect("activated cell was checked")
                    .next_production_occurrence_at(scheduled)?,
            )
        } else {
            None
        };
        let assignment = self.directory.current_cell_assignment(cell_id)?;
        let acknowledged = self
            .cells
            .get(cell_id)
            .expect("activated cell was checked")
            .lifecycle_record()?
            .acknowledge_world_commit(&assignment, trusted_now, &frontier, next_occurrence)?;
        self.append_committed_lifecycle(cell_id, acknowledged)?;
        Ok(u64::from(prior))
    }

    fn release_background_authority(
        &mut self,
        cell_key: &CellKeyV1,
        cell_id: &str,
        holder_id: &str,
        trusted_now: u64,
    ) -> Result<(), Protocol19InstallError> {
        let assignment = self.directory.current_cell_assignment(cell_id)?;
        let cell = self.cells.get(cell_id).expect("activated cell was checked");
        let frontier = cell.lifecycle_world_frontier()?;
        let request = cell.lifecycle_record()?.request_authority(
            &assignment,
            LifecycleAuthorityOperationKindV2::Release,
            holder_id,
            trusted_now,
            &frontier,
        )?;
        self.append_committed_lifecycle(cell_id, request.clone())?;
        self.finish_authority_operation(
            cell_key,
            cell_id,
            request
                .authority_operation()
                .expect("release request contains its operation"),
            trusted_now,
        )
    }
}

fn require_production_ack_time(
    trusted_now_unix_ms: u64,
    accepted_trusted_at_unix_ms: u64,
) -> Result<(), Protocol19InstallError> {
    if trusted_now_unix_ms < accepted_trusted_at_unix_ms {
        return Err(Protocol19InstallError::Invalid(
            "trusted time moved backward after the durable production event".into(),
        ));
    }
    Ok(())
}

pub(crate) fn prepare_or_recover<'migration, 'source>(
    transform: &'migration ValidatedProtocol19MigrationTransform<'source>,
    manifest: &ValidatedUniverseManifestV5,
) -> Result<PreparedProtocol19World<'migration, 'source>, Protocol19InstallError> {
    prepare_or_recover_with_failpoint(transform, manifest, None)
}

pub(crate) fn validate_from_active_head(
    universe_root: impl AsRef<Path>,
    expected_prepared_head_hash: &str,
) -> Result<ValidatedProtocol19PreparedInstall, Protocol19InstallError> {
    let universe_root = universe_root.as_ref();
    let install_root = universe_root.join(INSTALL_DIRECTORY);
    let metadata =
        fs::symlink_metadata(&install_root).map_err(|source| io_error(&install_root, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Protocol19InstallError::Invalid(
            "active head selects a non-directory prepared-install namespace".into(),
        ));
    }
    let install_lock = acquire_install_lock(&install_root, false)?;
    validate_install_file_set(&install_root)?;
    let head = Protocol19PreparedInstallHeadV1::decode_canonical(&read_bounded(
        &install_root.join(INSTALL_HEAD_FILE),
        MAX_INSTALL_HEAD_BYTES,
    )?)?;
    if head.head_hash != expected_prepared_head_hash {
        return Err(Protocol19InstallError::Invalid(
            "prepared-install head differs from the active global head".into(),
        ));
    }
    let summary = head.summary()?;
    let receipt_bytes = read_bounded(&install_root.join(RECEIPT_FILE), MAX_INSTALL_ARTIFACT_BYTES)?;
    let receipt = recover_canonical_migration_receipt(&receipt_bytes)?;
    validate_receipt_against_head(&receipt, &head)?;

    let source_archive = read_bounded(
        &install_root.join(SOURCE_DIRECTORY_ARCHIVE_FILE),
        MAX_INSTALL_ARTIFACT_BYTES,
    )?;
    if hash_source_directory_archive(&source_archive) != receipt.source_directory_archive_hash {
        return Err(Protocol19InstallError::Invalid(
            "source directory archive differs from the active receipt".into(),
        ));
    }

    let cell_ids = head
        .cells
        .iter()
        .map(|cell| cell.cell_id.clone())
        .collect::<BTreeSet<_>>();
    let identity_bytes = read_bounded(
        &install_root.join(IDENTITY_MAP_FILE),
        MAX_INSTALL_ARTIFACT_BYTES,
    )?;
    let identity_root = recover_identity_map_root(&identity_bytes, &head.universe_id, &cell_ids)
        .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
    let production_bytes = read_bounded(
        &install_root.join(PRODUCTION_ORIGIN_FILE),
        MAX_INSTALL_ARTIFACT_BYTES,
    )?;
    let production_root =
        recover_production_origin_root(&production_bytes, &head.universe_id, &cell_ids)
            .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
    if identity_root != head.identity_map_root || production_root != head.production_origin_root {
        return Err(Protocol19InstallError::Invalid(
            "identity or production artifact differs from the active head".into(),
        ));
    }

    let manifest_bytes = read_bounded(
        &install_root.join(TARGET_MANIFEST_FILE),
        MAX_INSTALL_ARTIFACT_BYTES,
    )?;
    let manifest = crate::manifest_v5::decode_manifest_v5(&manifest_bytes, summary.world_seed)
        .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
    if manifest.universe_id() != head.universe_id
        || manifest.manifest_hash() != head.target_manifest_hash
    {
        return Err(Protocol19InstallError::Invalid(
            "manifest artifact differs from the active head".into(),
        ));
    }

    let directory_document_bytes = read_bounded(
        &install_root.join(TARGET_DIRECTORY_FILE),
        MAX_INSTALL_ARTIFACT_BYTES,
    )?;
    let directory_history_bytes = read_bounded(
        &install_root.join(TARGET_DIRECTORY_HISTORY_FILE),
        MAX_INSTALL_ARTIFACT_BYTES,
    )?;
    validate_exact_cell_namespace_ids(universe_root, &cell_ids)?;
    Ok(ValidatedProtocol19PreparedInstall {
        universe_root: universe_root.to_path_buf(),
        install_lock,
        head,
        summary,
        receipt,
        manifest,
        directory_document_bytes,
        directory_history_bytes,
        cell_ids,
    })
}

fn validate_receipt_against_head(
    receipt: &CanonicalProtocol19MigrationReceiptEvidence,
    head: &Protocol19PreparedInstallHeadV1,
) -> Result<(), Protocol19InstallError> {
    if receipt.universe_id != head.universe_id
        || receipt.world_seed.to_string() != head.world_seed
        || receipt.target_manifest_hash != head.target_manifest_hash
        || receipt.migration_anchor_hash != head.migration_anchor_hash
        || receipt.migration_receipt_hash != head.migration_receipt_hash
        || receipt.source_directory_archive_hash != head.source_directory_archive_hash
        || receipt.identity_map_root != head.identity_map_root
        || receipt.production_origin_root != head.production_origin_root
        || receipt.target_directory_revision != head.target_directory_revision
        || receipt.target_directory_document_hash != head.target_directory_document_hash
        || receipt.target_directory_history_entry_hash != head.target_directory_history_entry_hash
        || receipt.target_assignment_root != head.target_assignment_root
        || receipt.target_placement_root != head.target_placement_root
        || receipt.global_conservation_root != head.global_conservation_root
        || receipt.normalized_gameplay_root != head.normalized_gameplay_root
        || receipt.cell_count != head.cell_count
    {
        return Err(Protocol19InstallError::Invalid(
            "migration receipt differs from the prepared-install head".into(),
        ));
    }
    if receipt.target_cells.len() != head.cells.len()
        || receipt
            .target_cells
            .iter()
            .zip(&head.cells)
            .any(|(receipt_cell, prepared_cell)| {
                receipt_cell.cell_key != prepared_cell.cell_key
                    || receipt_cell.cell_id != prepared_cell.cell_id
                    || receipt_cell.migration_anchor_hash != head.migration_anchor_hash
                    || receipt_cell.snapshot_state_hash != prepared_cell.snapshot_state_hash
                    || receipt_cell.active_world_hash != prepared_cell.active_world_hash
                    || receipt_cell.lifecycle_record_hash != prepared_cell.lifecycle_record_hash
                    || receipt_cell.event17_genesis_sequence != receipt_cell.legacy_event_sequence
                    || receipt_cell.event17_predecessor_hash != receipt_cell.legacy_event_head_hash
                    || receipt_cell.event17_journal_entry_count != 0
                    || !receipt_cell.event17_journal_head_hash.is_empty()
                    || prepared_cell.migration_receipt_hash != receipt.migration_receipt_hash
            })
    {
        return Err(Protocol19InstallError::Invalid(
            "migration receipt cell commitments differ from the prepared-install head".into(),
        ));
    }
    Ok(())
}

fn prepare_or_recover_with_failpoint<'migration, 'source>(
    transform: &'migration ValidatedProtocol19MigrationTransform<'source>,
    manifest: &ValidatedUniverseManifestV5,
    mut failpoint: Option<Protocol19InstallFailpoint>,
) -> Result<PreparedProtocol19World<'migration, 'source>, Protocol19InstallError> {
    if manifest.manifest_hash() != transform.target_manifest_hash()
        || manifest.world_seed() != transform.source().world_seed()
        || manifest.universe_id() != transform.source().universe_id()
    {
        return Err(Protocol19InstallError::Invalid(
            "prepared installer manifest differs from its migration transform".into(),
        ));
    }
    let directory_genesis = ValidatedProtocol19DirectoryGenesis::derive(transform)?;
    let receipt = ValidatedProtocol19MigrationReceipt::derive(transform, &directory_genesis)?;
    let universe_root = transform.source().universe_root();
    let install_root = universe_root.join(INSTALL_DIRECTORY);
    create_real_directory(universe_root, &install_root)?;
    let head_path = install_root.join(INSTALL_HEAD_FILE);
    let head_existed_before_lock = head_path
        .try_exists()
        .map_err(|source| io_error(&head_path, source))?;
    let install_lock = acquire_install_lock(&install_root, !head_existed_before_lock)?;
    inject(
        &mut failpoint,
        Protocol19InstallFailpoint::InstallNamespaceSynced,
    )?;
    let committed = head_path
        .try_exists()
        .map_err(|source| io_error(&head_path, source))?;
    if committed {
        validate_install_file_set(&install_root)?;
        validate_artifacts(
            &install_root,
            transform,
            manifest,
            &directory_genesis,
            &receipt,
        )?;
    } else {
        reset_uncommitted(universe_root, &install_root, transform, &directory_genesis)?;
        persist_artifacts(
            &install_root,
            transform,
            manifest,
            &directory_genesis,
            &receipt,
        )?;
        inject(&mut failpoint, Protocol19InstallFailpoint::ArtifactsSynced)?;
    }

    let directory = if committed {
        DraftCellDirectoryHistoryStoreV3::open_genesis(universe_root, &directory_genesis)?
    } else {
        let directory =
            DraftCellDirectoryHistoryStoreV3::stage_genesis(universe_root, &directory_genesis)?;
        directory.validate_genesis_file_set()?;
        inject(
            &mut failpoint,
            Protocol19InstallFailpoint::DirectoryGenesisSynced,
        )?;
        directory
    };
    directory.validate_genesis_file_set()?;

    let mut stores = Vec::with_capacity(transform.cells().len());
    let mut cell_evidence = Vec::with_capacity(transform.cells().len());
    for (index, cell) in transform.cells().iter().enumerate() {
        let target =
            ValidatedWorld21StagingTarget::from_migration_receipt(&receipt, cell, manifest)?;
        let cell_root = universe_root.join("cells").join(cell.cell_id());
        let store = if committed {
            DraftWorld21Store::open_from_migration(&cell_root, &target)?
        } else {
            DraftWorld21Store::stage_from_migration(&cell_root, &target)?
        };
        store.validate_prepared_file_set()?;
        cell_evidence.push(store.prepared_install_evidence());
        stores.push(store);
        inject(
            &mut failpoint,
            Protocol19InstallFailpoint::CellStaged(index),
        )?;
    }
    validate_exact_cell_namespaces(universe_root, transform)?;
    let expected_head =
        Protocol19PreparedInstallHeadV1::new(&receipt, &directory_genesis, cell_evidence)?;
    let head = if committed {
        let bytes = read_bounded(&head_path, MAX_INSTALL_HEAD_BYTES)?;
        let persisted = Protocol19PreparedInstallHeadV1::decode_canonical(&bytes)?;
        if persisted != expected_head {
            return Err(Protocol19InstallError::Invalid(
                "prepared-install head differs from the exact frozen-source migration".into(),
            ));
        }
        persisted
    } else {
        let bytes = expected_head.encode_canonical()?;
        persist_head(&install_root, &bytes, &mut failpoint)?;
        validate_install_file_set(&install_root)?;
        expected_head
    };
    Ok(PreparedProtocol19World {
        head,
        universe_root: universe_root.to_owned(),
        _directory: directory,
        _cells: stores,
        _install_lock: install_lock,
        _migration: PhantomData,
    })
}

fn persist_artifacts(
    install_root: &Path,
    transform: &ValidatedProtocol19MigrationTransform<'_>,
    manifest: &ValidatedUniverseManifestV5,
    directory: &ValidatedProtocol19DirectoryGenesis<'_, '_>,
    receipt: &ValidatedProtocol19MigrationReceipt<'_, '_>,
) -> Result<(), Protocol19InstallError> {
    let manifest_bytes = crate::manifest_v5::encode_manifest_v5(manifest)
        .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
    for (name, bytes) in expected_artifacts(transform, &manifest_bytes, directory, receipt) {
        if bytes.is_empty() || bytes.len() > MAX_INSTALL_ARTIFACT_BYTES {
            return Err(Protocol19InstallError::TooLarge(install_root.join(name)));
        }
        atomic_write(&install_root.join(name), bytes)?;
    }
    sync_directory(install_root)
}

fn validate_artifacts(
    install_root: &Path,
    transform: &ValidatedProtocol19MigrationTransform<'_>,
    manifest: &ValidatedUniverseManifestV5,
    directory: &ValidatedProtocol19DirectoryGenesis<'_, '_>,
    receipt: &ValidatedProtocol19MigrationReceipt<'_, '_>,
) -> Result<(), Protocol19InstallError> {
    let manifest_bytes = crate::manifest_v5::encode_manifest_v5(manifest)
        .map_err(|source| Protocol19InstallError::Invalid(source.to_string()))?;
    for (name, expected) in expected_artifacts(transform, &manifest_bytes, directory, receipt) {
        if read_bounded(&install_root.join(name), MAX_INSTALL_ARTIFACT_BYTES)? != expected {
            return Err(Protocol19InstallError::Invalid(format!(
                "prepared artifact {name} differs from the source-bound migration"
            )));
        }
    }
    Ok(())
}

fn expected_artifacts<'a>(
    transform: &'a ValidatedProtocol19MigrationTransform<'_>,
    manifest_bytes: &'a [u8],
    directory: &'a ValidatedProtocol19DirectoryGenesis<'_, '_>,
    receipt: &'a ValidatedProtocol19MigrationReceipt<'_, '_>,
) -> [(&'static str, &'a [u8]); 7] {
    [
        (RECEIPT_FILE, receipt.bytes()),
        (
            SOURCE_DIRECTORY_ARCHIVE_FILE,
            transform.source().directory_document_bytes(),
        ),
        (IDENTITY_MAP_FILE, transform.identity_map_bytes()),
        (PRODUCTION_ORIGIN_FILE, transform.production_origin_bytes()),
        (TARGET_MANIFEST_FILE, manifest_bytes),
        (TARGET_DIRECTORY_FILE, directory.document_bytes()),
        (
            TARGET_DIRECTORY_HISTORY_FILE,
            directory.history_entry_bytes(),
        ),
    ]
}

fn reset_uncommitted(
    universe_root: &Path,
    install_root: &Path,
    transform: &ValidatedProtocol19MigrationTransform<'_>,
    _directory: &ValidatedProtocol19DirectoryGenesis<'_, '_>,
) -> Result<(), Protocol19InstallError> {
    validate_resettable_install_file_set(install_root)?;
    DraftCellDirectoryHistoryStoreV3::discard_uncommitted_genesis(universe_root)?;
    for cell in transform.cells() {
        DraftWorld21Store::discard_uncommitted_namespace(
            universe_root.join("cells").join(cell.cell_id()),
        )?;
    }
    for entry in fs::read_dir(install_root).map_err(|source| io_error(install_root, source))? {
        let entry = entry.map_err(|source| io_error(install_root, source))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            Protocol19InstallError::Invalid(
                "prepared-install namespace contains non-UTF-8 debris".into(),
            )
        })?;
        if name != INSTALL_LOCK_FILE {
            fs::remove_file(entry.path()).map_err(|source| io_error(entry.path(), source))?;
        }
    }
    sync_directory(install_root)
}

fn validate_resettable_install_file_set(root: &Path) -> Result<(), Protocol19InstallError> {
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            Protocol19InstallError::Invalid(
                "prepared-install namespace contains non-UTF-8 debris".into(),
            )
        })?;
        let recognized = name == INSTALL_LOCK_FILE
            || name == INSTALL_HEAD_FILE
            || artifact_names().contains(&name)
            || artifact_names()
                .iter()
                .any(|artifact| name.starts_with(&format!(".{artifact}.tmp-")))
            || name.starts_with(&format!(".{INSTALL_HEAD_FILE}.tmp-"));
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_file()
            || !recognized
        {
            return Err(Protocol19InstallError::Invalid(
                "prepared-install namespace contains unknown debris".into(),
            ));
        }
    }
    Ok(())
}

fn validate_install_file_set(root: &Path) -> Result<(), Protocol19InstallError> {
    let mut expected = artifact_names().to_vec();
    expected.extend([INSTALL_LOCK_FILE, INSTALL_HEAD_FILE]);
    let mut observed = Vec::new();
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_file()
        {
            return Err(Protocol19InstallError::Invalid(
                "prepared-install namespace contains a non-file artifact".into(),
            ));
        }
        observed.push(entry.file_name().into_string().map_err(|_| {
            Protocol19InstallError::Invalid(
                "prepared-install namespace contains a non-UTF-8 artifact".into(),
            )
        })?);
    }
    expected.sort_unstable();
    observed.sort_unstable();
    if observed != expected {
        return Err(Protocol19InstallError::Invalid(
            "prepared-install namespace is not the exact committed artifact set".into(),
        ));
    }
    Ok(())
}

fn artifact_names() -> [&'static str; 7] {
    [
        RECEIPT_FILE,
        SOURCE_DIRECTORY_ARCHIVE_FILE,
        IDENTITY_MAP_FILE,
        PRODUCTION_ORIGIN_FILE,
        TARGET_MANIFEST_FILE,
        TARGET_DIRECTORY_FILE,
        TARGET_DIRECTORY_HISTORY_FILE,
    ]
}

fn validate_exact_cell_namespaces(
    universe_root: &Path,
    transform: &ValidatedProtocol19MigrationTransform<'_>,
) -> Result<(), Protocol19InstallError> {
    let expected = transform
        .cells()
        .iter()
        .map(|cell| cell.cell_id().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    let cells_root = universe_root.join("cells");
    let mut observed = std::collections::BTreeSet::new();
    for entry in fs::read_dir(&cells_root).map_err(|source| io_error(&cells_root, source))? {
        let entry = entry.map_err(|source| io_error(&cells_root, source))?;
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_dir()
        {
            continue;
        }
        if DraftWorld21Store::namespace_exists(entry.path())? {
            let cell_id = entry.file_name().into_string().map_err(|_| {
                Protocol19InstallError::Invalid(
                    "world-21 cell namespace has a non-UTF-8 route".into(),
                )
            })?;
            observed.insert(cell_id);
        }
    }
    if observed != expected {
        return Err(Protocol19InstallError::Invalid(
            "world-21 namespaces are not the exact migration cell set".into(),
        ));
    }
    Ok(())
}

fn validate_exact_cell_namespace_ids(
    universe_root: &Path,
    expected: &BTreeSet<String>,
) -> Result<(), Protocol19InstallError> {
    let cells_root = universe_root.join("cells");
    let mut observed = BTreeSet::new();
    for entry in fs::read_dir(&cells_root).map_err(|source| io_error(&cells_root, source))? {
        let entry = entry.map_err(|source| io_error(&cells_root, source))?;
        if !entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?
            .is_dir()
        {
            continue;
        }
        if DraftWorld21Store::namespace_exists(entry.path())? {
            observed.insert(entry.file_name().into_string().map_err(|_| {
                Protocol19InstallError::Invalid(
                    "world-21 cell namespace has a non-UTF-8 route".into(),
                )
            })?);
        }
    }
    if &observed != expected {
        return Err(Protocol19InstallError::Invalid(
            "world-21 namespaces are not the exact active-head cell set".into(),
        ));
    }
    Ok(())
}

fn persist_head(
    root: &Path,
    bytes: &[u8],
    failpoint: &mut Option<Protocol19InstallFailpoint>,
) -> Result<(), Protocol19InstallError> {
    let path = root.join(INSTALL_HEAD_FILE);
    let temporary = root.join(format!(".{INSTALL_HEAD_FILE}.tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| io_error(&temporary, source))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error(&temporary, source))?;
        inject(
            failpoint,
            Protocol19InstallFailpoint::HeadTempSyncedBeforeRename,
        )?;
        fs::rename(&temporary, &path).map_err(|source| io_error(&path, source))?;
        inject(
            failpoint,
            Protocol19InstallFailpoint::HeadRenamedBeforeDirectorySync,
        )?;
        sync_directory(root)?;
        inject(
            failpoint,
            Protocol19InstallFailpoint::HeadDirectorySyncedBeforeMemory,
        )
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn inject(
    selected: &mut Option<Protocol19InstallFailpoint>,
    current: Protocol19InstallFailpoint,
) -> Result<(), Protocol19InstallError> {
    if *selected == Some(current) {
        #[cfg(test)]
        if std::env::var_os("VERSE_PROTOCOL19_INSTALL_HARD_EXIT").is_some() {
            std::process::exit(97);
        }
        *selected = None;
        Err(Protocol19InstallError::Injected(current))
    } else {
        Ok(())
    }
}

fn create_real_directory(parent: &Path, path: &Path) -> Result<(), Protocol19InstallError> {
    fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(Protocol19InstallError::Invalid(
            "prepared-install namespace is not a real directory".into(),
        ));
    }
    sync_directory(parent)
}

fn acquire_install_lock(root: &Path, create: bool) -> Result<File, Protocol19InstallError> {
    let path = root.join(INSTALL_LOCK_FILE);
    let file = OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| io_error(&path, source))?;
    match file.try_lock_exclusive() {
        Ok(()) => Ok(file),
        Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
            Err(Protocol19InstallError::WriterConflict)
        }
        Err(source) => Err(io_error(&path, source)),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Protocol19InstallError> {
    let parent = path
        .parent()
        .ok_or_else(|| Protocol19InstallError::Invalid("prepared artifact has no parent".into()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            Protocol19InstallError::Invalid("prepared artifact name is not UTF-8".into())
        })?;
    let temporary = parent.join(format!(".{name}.tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| io_error(&temporary, source))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error(&temporary, source))?;
        fs::rename(&temporary, path).map_err(|source| io_error(path, source))?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn atomic_write_universe_lifecycle_head(
    path: &Path,
    bytes: &[u8],
    failpoint: Option<UniverseLifecyclePersistFailpointV1>,
) -> Result<(), Protocol19InstallError> {
    let parent = path.parent().ok_or_else(|| {
        Protocol19InstallError::Invalid("universe lifecycle head has no parent".into())
    })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            Protocol19InstallError::Invalid("universe lifecycle head is not UTF-8".into())
        })?;
    let temporary = parent.join(format!(".{name}.tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| io_error(&temporary, source))?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| io_error(&temporary, source))?;
        if failpoint == Some(UniverseLifecyclePersistFailpointV1::HeadSyncedBeforeRename) {
            return Err(Protocol19InstallError::Invalid(
                "injected failure before universe lifecycle head replacement".into(),
            ));
        }
        fs::rename(&temporary, path).map_err(|source| io_error(path, source))?;
        sync_directory(parent)?;
        if failpoint == Some(UniverseLifecyclePersistFailpointV1::HeadRenamedBeforeMemory) {
            return Err(Protocol19InstallError::Invalid(
                "injected failure after universe lifecycle head replacement".into(),
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, Protocol19InstallError> {
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if length > u64::try_from(maximum).unwrap_or(u64::MAX) {
        return Err(Protocol19InstallError::TooLarge(path.to_owned()));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    file.take(u64::try_from(maximum).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() > maximum {
        return Err(Protocol19InstallError::TooLarge(path.to_owned()));
    }
    Ok(bytes)
}

fn sync_directory(path: &Path) -> Result<(), Protocol19InstallError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

fn hash_json<T: Serialize>(domain: &[u8], value: &T) -> Result<String, Protocol19InstallError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| Protocol19InstallError::Json(source.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn all_hashes<'a>(values: impl IntoIterator<Item = &'a String>) -> bool {
    values.into_iter().all(|value| {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn io_error(path: impl AsRef<Path>, source: std::io::Error) -> Protocol19InstallError {
    Protocol19InstallError::Io {
        path: path.as_ref().to_owned(),
        source,
    }
}

#[cfg(test)]
pub(crate) fn initialize_frozen_protocol18_fixture_for_test(root: &Path, with_event: bool) {
    let manifest = crate::celestial::universe_manifest(
        8_119,
        crate::WORLD_SCHEMA_VERSION,
        crate::EVENT_SCHEMA_VERSION,
    )
    .expect("source manifest builds");
    let cell_keys = crate::proof_cell_keys().expect("proof cells derive");
    let mut directory = crate::LocalCellDirectory::open(root, &manifest, cell_keys.clone())
        .expect("source directory initializes");
    for (index, cell_key) in cell_keys.into_iter().enumerate() {
        let prior = directory
            .assignment(&cell_key)
            .expect("source assignment exists")
            .clone();
        let holder = format!("prepared-install-fixture-{index}");
        let cell_root = directory
            .cell_store_root(&cell_key)
            .expect("source cell route derives");
        let mut runtime =
            crate::Runtime::open_directory_managed_for_cell(cell_root, 8_119, cell_key.clone(), 1)
                .expect("source cell initializes");
        let assignment = directory
            .claim(
                &cell_key,
                prior.assignment_generation,
                &holder,
                runtime.state().fencing_token,
            )
            .expect("source cell claims");
        if with_event && index == 0 {
            runtime
                .execute_next_for_fixture(&verse_protocol::ClientMessage::SetSuitMode {
                    operation_sequence: 0,
                    operation_id: "prepared-install-source-event".into(),
                    helmet_closed: false,
                    jetpack_enabled: true,
                    magnetic_boots_enabled: false,
                })
                .expect("source event commits");
        }
        assert_eq!(
            runtime
                .drain_to_background_or_sleeping()
                .expect("source cell drains"),
            crate::LifecycleMode::Sleeping
        );
        directory
            .release(&cell_key, assignment.assignment_generation, &holder)
            .expect("source cell releases");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use tempfile::{TempDir, tempdir};
    use verse_protocol::{CellCoordinate, CellKeyV1, SectorCoordinate};

    use super::*;
    use crate::grid_handoff_v2::migration_transform::ValidatedProtocol19MigrationTransform;
    use crate::protocol19_source::ValidatedFrozenProtocol18Source;
    use crate::{
        LifecycleMode, LocalCellDirectory, LocalTwoCellRuntime, MobileAggregateKind, Runtime,
    };

    const TEST_SEED: u64 = 8_119;
    const SUBPROCESS_MODE_ENV: &str = "VERSE_PROTOCOL19_INSTALL_SUBPROCESS_MODE";
    const SUBPROCESS_ROOT_ENV: &str = "VERSE_PROTOCOL19_INSTALL_SUBPROCESS_ROOT";
    const SUBPROCESS_SECOND_CWD_ENV: &str = "VERSE_PROTOCOL19_INSTALL_SECOND_CWD";

    #[test]
    fn production_ack_rejects_time_before_the_persisted_event() {
        assert!(require_production_ack_time(8_000, 8_000).is_ok());
        assert!(require_production_ack_time(7_999, 8_000).is_err());
    }

    fn frozen_fixture(with_event: bool) -> TempDir {
        let root = tempdir().expect("temporary universe root");
        initialize_frozen_fixture(root.path(), with_event);
        root
    }

    fn initialize_frozen_fixture(root: &Path, with_event: bool) {
        initialize_frozen_protocol18_fixture_for_test(root, with_event);
    }

    fn finalized_transfer_fixture() -> TempDir {
        let root = tempdir().expect("temporary universe root");
        let manifest = crate::celestial::universe_manifest(
            TEST_SEED,
            crate::WORLD_SCHEMA_VERSION,
            crate::EVENT_SCHEMA_VERSION,
        )
        .expect("source manifest builds");
        let cells = crate::proof_cell_keys().expect("proof cells derive");
        let mut directory = LocalCellDirectory::open(root.path(), &manifest, cells.clone())
            .expect("source directory initializes");
        let prior = directory
            .assignment(&cells[0])
            .expect("source assignment exists")
            .clone();
        let cell_root = directory
            .cell_store_root(&cells[0])
            .expect("source cell route derives");
        let mut runtime =
            Runtime::open_directory_managed_for_cell(cell_root, TEST_SEED, cells[0].clone(), 600)
                .expect("source cell initializes");
        let holder_id = "migration-boundary-fixture";
        let assignment = directory
            .claim(
                &cells[0],
                prior.assignment_generation,
                holder_id,
                runtime.state().fencing_token,
            )
            .expect("source cell claims");
        runtime
            .configure_migration_boundary_fixture()
            .expect("canonical boundary profile persists");
        assert_eq!(
            runtime
                .drain_to_background_or_sleeping()
                .expect("source cell drains"),
            LifecycleMode::Sleeping
        );
        directory
            .release(&cells[0], assignment.assignment_generation, holder_id)
            .expect("source cell releases");
        drop(runtime);
        drop(directory);

        let mut coordinator =
            LocalTwoCellRuntime::open(root.path(), TEST_SEED, 600, "migration-floor")
                .expect("two-cell coordinator opens");
        let handoff = coordinator
            .handoff_player("player-local")
            .expect("player transfer finalizes");
        assert_eq!(handoff.placement_generation, 2);
        coordinator
            .release_all_for_frozen_fixture()
            .expect("terminal source releases to frozen state");
        root
    }

    fn collect_legacy_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(base: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).expect("legacy directory reads") {
                let entry = entry.expect("legacy entry reads");
                let entry_path = entry.path();
                let name = entry.file_name();
                let name = name.to_str().expect("fixture names are UTF-8");
                if [
                    INSTALL_DIRECTORY,
                    "protocol-19-directory-v3",
                    "protocol-19-world-v21",
                ]
                .contains(&name)
                {
                    continue;
                }
                if entry.file_type().expect("legacy file type reads").is_dir() {
                    visit(base, &entry_path, files);
                } else {
                    files.insert(
                        entry_path
                            .strip_prefix(base)
                            .expect("legacy path is rooted")
                            .to_owned(),
                        fs::read(entry_path).expect("legacy file reads"),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    fn collect_all_files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(base: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).expect("directory reads") {
                let entry = entry.expect("directory entry reads");
                let entry_path = entry.path();
                if entry.file_type().expect("file type reads").is_dir() {
                    visit(base, &entry_path, files);
                } else {
                    files.insert(
                        entry_path
                            .strip_prefix(base)
                            .expect("file path is rooted")
                            .to_owned(),
                        fs::read(entry_path).expect("file reads"),
                    );
                }
            }
        }
        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    fn collect_tree_topology(root: &Path) -> BTreeMap<PathBuf, &'static str> {
        fn visit(base: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, &'static str>) {
            for entry in fs::read_dir(path).expect("directory topology reads") {
                let entry = entry.expect("directory topology entry reads");
                let entry_path = entry.path();
                let relative = entry_path
                    .strip_prefix(base)
                    .expect("topology path is rooted")
                    .to_owned();
                let file_type = entry.file_type().expect("topology file type reads");
                if file_type.is_dir() {
                    entries.insert(relative, "directory");
                    visit(base, &entry_path, entries);
                } else if file_type.is_symlink() {
                    entries.insert(relative, "symlink");
                } else {
                    entries.insert(relative, "file");
                }
            }
        }
        let mut entries = BTreeMap::new();
        visit(root, root, &mut entries);
        entries
    }

    fn failpoint_label(failpoint: Protocol19InstallFailpoint) -> &'static str {
        match failpoint {
            Protocol19InstallFailpoint::InstallNamespaceSynced => "install_namespace_synced",
            Protocol19InstallFailpoint::ArtifactsSynced => "artifacts_synced",
            Protocol19InstallFailpoint::DirectoryGenesisSynced => "directory_genesis_synced",
            Protocol19InstallFailpoint::CellStaged(0) => "cell_staged_0",
            Protocol19InstallFailpoint::CellStaged(1) => "cell_staged_1",
            Protocol19InstallFailpoint::CellStaged(_) => "unsupported_cell_staged",
            Protocol19InstallFailpoint::HeadTempSyncedBeforeRename => {
                "head_temp_synced_before_rename"
            }
            Protocol19InstallFailpoint::HeadRenamedBeforeDirectorySync => {
                "head_renamed_before_directory_sync"
            }
            Protocol19InstallFailpoint::HeadDirectorySyncedBeforeMemory => {
                "head_directory_synced_before_memory"
            }
        }
    }

    fn parse_failpoint_label(label: &str) -> Protocol19InstallFailpoint {
        match label {
            "install_namespace_synced" => Protocol19InstallFailpoint::InstallNamespaceSynced,
            "artifacts_synced" => Protocol19InstallFailpoint::ArtifactsSynced,
            "directory_genesis_synced" => Protocol19InstallFailpoint::DirectoryGenesisSynced,
            "cell_staged_0" => Protocol19InstallFailpoint::CellStaged(0),
            "cell_staged_1" => Protocol19InstallFailpoint::CellStaged(1),
            "head_temp_synced_before_rename" => {
                Protocol19InstallFailpoint::HeadTempSyncedBeforeRename
            }
            "head_renamed_before_directory_sync" => {
                Protocol19InstallFailpoint::HeadRenamedBeforeDirectorySync
            }
            "head_directory_synced_before_memory" => {
                Protocol19InstallFailpoint::HeadDirectorySyncedBeforeMemory
            }
            _ => panic!("unsupported install failpoint label {label}"),
        }
    }

    #[test]
    fn protocol19_install_subprocess_driver() {
        let Some(mode) = std::env::var_os(SUBPROCESS_MODE_ENV) else {
            return;
        };
        let mode = mode.to_string_lossy();
        if mode == "relative_route" {
            let source = ValidatedFrozenProtocol18Source::acquire_existing("universe", TEST_SEED)
                .expect("relative source freezes");
            let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
                .expect("target manifest builds");
            let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
                .expect("target transform derives");
            std::env::set_current_dir(
                std::env::var_os(SUBPROCESS_SECOND_CWD_ENV)
                    .expect("second working directory is configured"),
            )
            .expect("working directory changes");
            drop(prepare_or_recover(&transform, &manifest).expect("relative source prepares"));
            return;
        }
        let root = PathBuf::from(
            std::env::var_os(SUBPROCESS_ROOT_ENV).expect("subprocess root is configured"),
        );
        let source = ValidatedFrozenProtocol18Source::acquire_existing(&root, TEST_SEED)
            .expect("subprocess source freezes");
        let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
            .expect("target manifest builds");
        let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
            .expect("target transform derives");
        let failpoint = parse_failpoint_label(&mode);
        let result = prepare_or_recover_with_failpoint(&transform, &manifest, Some(failpoint));
        panic!("hard-exit failpoint returned instead of terminating: {result:?}");
    }

    #[test]
    fn maximum_cell_head_is_canonical_and_fits_its_wire_bound() {
        let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
            .expect("target manifest builds");
        let hash = "a".repeat(64);
        let mut cells = (0..MAX_INSTALL_CELLS)
            .map(|index| {
                let cell_key = CellKeyV1 {
                    schema_version: 1,
                    universe_id: manifest.universe_id().to_owned(),
                    sector: SectorCoordinate {
                        x: i128::MAX.to_string(),
                        y: i128::MIN.to_string(),
                        z: "0".into(),
                    },
                    cell: CellCoordinate {
                        x: u32::try_from(index % 1_000).expect("x coordinate fits"),
                        y: u32::try_from(index / 1_000).expect("y coordinate fits"),
                        z: 999,
                    },
                };
                let cell_id = crate::celestial::cell_id(&cell_key).expect("cell ID derives");
                PreparedWorld21CellEvidence {
                    cell_key,
                    cell_id,
                    initialization_head_hash: hash.clone(),
                    identity_hash: hash.clone(),
                    lifecycle_record_hash: hash.clone(),
                    snapshot_state_hash: hash.clone(),
                    active_world_hash: hash.clone(),
                    migration_receipt_hash: hash.clone(),
                }
            })
            .collect::<Vec<_>>();
        cells.sort_by(|left, right| left.cell_id.cmp(&right.cell_id));
        let cell_set_root = hash_json(INSTALL_CELL_SET_HASH_DOMAIN, &cells).expect("cells hash");
        let mut head = Protocol19PreparedInstallHeadV1 {
            schema_version: INSTALL_HEAD_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            mode: PreparedInstallMode::StagedUnactivated,
            universe_id: manifest.universe_id().to_owned(),
            world_seed: TEST_SEED.to_string(),
            target_manifest_hash: manifest.manifest_hash().to_owned(),
            source_directory_document_hash: hash.clone(),
            source_directory_archive_hash: hash.clone(),
            migration_anchor_hash: hash.clone(),
            migration_receipt_hash: hash.clone(),
            target_directory_revision: 1,
            target_directory_document_hash: hash.clone(),
            target_directory_history_entry_hash: hash.clone(),
            target_assignment_root: hash.clone(),
            target_placement_root: hash.clone(),
            identity_map_root: hash.clone(),
            production_origin_root: hash.clone(),
            global_conservation_root: hash.clone(),
            normalized_gameplay_root: hash.clone(),
            cell_count: u64::try_from(cells.len()).expect("cell count fits"),
            cell_set_root,
            cells,
            head_hash: String::new(),
        };
        head.head_hash = head.calculate_hash().expect("head hash derives");
        let bytes = head.encode_canonical().expect("maximum head encodes");
        assert!(bytes.len() <= MAX_INSTALL_HEAD_BYTES);
        assert_eq!(
            Protocol19PreparedInstallHeadV1::decode_canonical(&bytes).expect("head decodes"),
            head
        );
    }

    #[test]
    fn prepared_cell_commitments_must_match_the_canonical_receipt() {
        let root = frozen_fixture(true);
        let source = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect("source freezes");
        let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
            .expect("target manifest builds");
        let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
            .expect("target transform derives");
        drop(prepare_or_recover(&transform, &manifest).expect("world prepares"));
        let install_root = root.path().join(INSTALL_DIRECTORY);
        let receipt = recover_canonical_migration_receipt(
            &fs::read(install_root.join(RECEIPT_FILE)).expect("receipt reads"),
        )
        .expect("receipt recovers");
        let mut head = Protocol19PreparedInstallHeadV1::decode_canonical(
            &fs::read(install_root.join(INSTALL_HEAD_FILE)).expect("head reads"),
        )
        .expect("head decodes");
        validate_receipt_against_head(&receipt, &head).expect("original bindings match");

        head.cells[0].snapshot_state_hash = "b".repeat(64);
        head.cell_set_root =
            hash_json(INSTALL_CELL_SET_HASH_DOMAIN, &head.cells).expect("cell set reseals");
        head.head_hash = head.calculate_hash().expect("head reseals");
        head.validate()
            .expect("mutated head remains internally valid");
        assert!(validate_receipt_against_head(&receipt, &head).is_err());
    }

    #[test]
    fn real_two_cell_source_prepares_one_exact_recoverable_world() {
        let root = frozen_fixture(true);
        let before = collect_legacy_files(root.path());
        let source = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect("source freezes");
        let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
            .expect("target manifest builds");
        let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
            .expect("target transform derives");

        let first = prepare_or_recover(&transform, &manifest).expect("world prepares");
        assert_eq!(first.cell_count(), 2);
        assert_eq!(first.head_hash().len(), 64);
        assert_eq!(first.receipt_hash().len(), 64);
        let install_root = root.path().join(INSTALL_DIRECTORY);
        let receipt: serde_json::Value = serde_json::from_slice(
            &fs::read(install_root.join(RECEIPT_FILE)).expect("receipt reads"),
        )
        .expect("receipt parses");
        assert_eq!(
            receipt["source_cells"]
                .as_array()
                .expect("source cells are an array")
                .len(),
            2
        );
        assert_eq!(
            receipt["target_cells"]
                .as_array()
                .expect("target cells are an array")
                .len(),
            2
        );
        for (source_cell, target_cell) in receipt["source_cells"]
            .as_array()
            .expect("source cells exist")
            .iter()
            .zip(
                receipt["target_cells"]
                    .as_array()
                    .expect("target cells exist"),
            )
        {
            assert_eq!(
                target_cell["legacy_event_sequence"],
                source_cell["source_event_sequence"]
            );
            assert_eq!(
                target_cell["legacy_event_head_hash"],
                source_cell["source_event_head_hash"]
            );
            assert_eq!(target_cell["event17_journal_entry_count"], 0);
        }
        let directory: serde_json::Value = serde_json::from_slice(
            &fs::read(install_root.join(TARGET_DIRECTORY_FILE)).expect("directory genesis reads"),
        )
        .expect("directory genesis parses");
        assert_eq!(
            directory["assignments"]
                .as_object()
                .expect("assignments exist")
                .len(),
            2
        );
        assert_eq!(
            directory["placements"]
                .as_object()
                .expect("placements exist")
                .len(),
            3
        );
        assert!(
            directory["transfers"]
                .as_object()
                .expect("transfers exist")
                .is_empty()
        );
        assert_eq!(
            fs::read(install_root.join(SOURCE_DIRECTORY_ARCHIVE_FILE))
                .expect("source archive reads"),
            fs::read(root.path().join("cell-directory.json")).expect("source directory reads")
        );
        assert_eq!(collect_legacy_files(root.path()), before);
        assert!(matches!(
            prepare_or_recover(&transform, &manifest),
            Err(Protocol19InstallError::WriterConflict)
        ));
        let first_head = first.head_hash().to_owned();
        let first_receipt = first.receipt_hash().to_owned();
        drop(first);

        let reopened = prepare_or_recover(&transform, &manifest).expect("world reopens");
        assert_eq!(reopened.head_hash(), first_head);
        assert_eq!(reopened.receipt_hash(), first_receipt);
        assert_eq!(collect_legacy_files(root.path()), before);
    }

    #[test]
    fn finalized_source_transfer_becomes_a_receipt_bound_placement_floor() {
        let root = finalized_transfer_fixture();
        let before = collect_legacy_files(root.path());
        let source = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
            .expect("terminal transfer source freezes");
        assert_eq!(source.terminal_transfer_count(), 1);
        let source_placement = source
            .placements()
            .find(|placement| {
                placement.aggregate_kind == MobileAggregateKind::Player
                    && placement.placement_generation == 2
            })
            .expect("finalized player placement exists")
            .clone();
        let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
            .expect("target manifest builds");
        let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
            .expect("target transform derives");
        let target_id = transform
            .target_aggregate_id(
                source_placement.aggregate_kind,
                &source_placement.cell_id,
                &source_placement.aggregate_id,
            )
            .to_owned();

        drop(prepare_or_recover(&transform, &manifest).expect("world prepares"));
        let directory: serde_json::Value = serde_json::from_slice(
            &fs::read(
                root.path()
                    .join(INSTALL_DIRECTORY)
                    .join(TARGET_DIRECTORY_FILE),
            )
            .expect("directory genesis reads"),
        )
        .expect("directory genesis parses");
        assert_eq!(
            directory["placements"][&target_id]["placement_generation"],
            2
        );
        assert_eq!(
            directory["migration_genesis"]["placement_floors"][&target_id]["placement_generation"],
            2
        );
        assert_eq!(
            directory["migration_genesis"]["source_terminal_transfer_ids"]
                .as_array()
                .expect("source transfer identities are canonical")
                .len(),
            1
        );
        assert_eq!(collect_legacy_files(root.path()), before);
    }

    #[test]
    fn every_universe_boundary_recovers_to_one_exact_prepared_world() {
        for failpoint in [
            Protocol19InstallFailpoint::InstallNamespaceSynced,
            Protocol19InstallFailpoint::ArtifactsSynced,
            Protocol19InstallFailpoint::DirectoryGenesisSynced,
            Protocol19InstallFailpoint::CellStaged(0),
            Protocol19InstallFailpoint::CellStaged(1),
            Protocol19InstallFailpoint::HeadTempSyncedBeforeRename,
            Protocol19InstallFailpoint::HeadRenamedBeforeDirectorySync,
            Protocol19InstallFailpoint::HeadDirectorySyncedBeforeMemory,
        ] {
            let root = frozen_fixture(true);
            let before = collect_legacy_files(root.path());
            let source = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
                .expect("source freezes");
            let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
                .expect("target manifest builds");
            let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
                .expect("target transform derives");
            assert!(matches!(
                prepare_or_recover_with_failpoint(&transform, &manifest, Some(failpoint)),
                Err(Protocol19InstallError::Injected(actual)) if actual == failpoint
            ));
            let globally_visible = root
                .path()
                .join(INSTALL_DIRECTORY)
                .join(INSTALL_HEAD_FILE)
                .exists();
            assert_eq!(
                globally_visible,
                matches!(
                    failpoint,
                    Protocol19InstallFailpoint::HeadRenamedBeforeDirectorySync
                        | Protocol19InstallFailpoint::HeadDirectorySyncedBeforeMemory
                ),
                "{failpoint:?} exposed the wrong global commit state"
            );
            let recovered = prepare_or_recover(&transform, &manifest)
                .unwrap_or_else(|error| panic!("{failpoint:?} must recover: {error}"));
            assert_eq!(recovered.cell_count(), 2);
            assert_eq!(collect_legacy_files(root.path()), before);
        }
    }

    #[test]
    fn abrupt_process_loss_at_every_boundary_recovers_the_exact_world() {
        for failpoint in [
            Protocol19InstallFailpoint::InstallNamespaceSynced,
            Protocol19InstallFailpoint::ArtifactsSynced,
            Protocol19InstallFailpoint::DirectoryGenesisSynced,
            Protocol19InstallFailpoint::CellStaged(0),
            Protocol19InstallFailpoint::CellStaged(1),
            Protocol19InstallFailpoint::HeadTempSyncedBeforeRename,
            Protocol19InstallFailpoint::HeadRenamedBeforeDirectorySync,
            Protocol19InstallFailpoint::HeadDirectorySyncedBeforeMemory,
        ] {
            let root = frozen_fixture(true);
            let before = collect_legacy_files(root.path());
            let output = Command::new(std::env::current_exe().expect("test executable resolves"))
                .arg("--exact")
                .arg("protocol19_install::tests::protocol19_install_subprocess_driver")
                .arg("--nocapture")
                .env(SUBPROCESS_MODE_ENV, failpoint_label(failpoint))
                .env(SUBPROCESS_ROOT_ENV, root.path())
                .env("VERSE_PROTOCOL19_INSTALL_HARD_EXIT", "1")
                .output()
                .expect("hard-exit child launches");
            assert_eq!(
                output.status.code(),
                Some(97),
                "{failpoint:?} child did not terminate at the boundary: {}",
                String::from_utf8_lossy(&output.stderr)
            );

            let source = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
                .expect("source refreezes after process loss");
            let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
                .expect("target manifest builds");
            let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
                .expect("target transform derives");
            let recovered = prepare_or_recover(&transform, &manifest)
                .unwrap_or_else(|error| panic!("{failpoint:?} must recover: {error}"));
            assert_eq!(recovered.cell_count(), 2);
            assert_eq!(collect_legacy_files(root.path()), before);
        }
    }

    #[test]
    fn relative_frozen_source_route_remains_bound_after_working_directory_change() {
        let outer = tempdir().expect("outer fixture root");
        let first_parent = outer.path().join("first");
        let second_parent = outer.path().join("second");
        let first_universe = first_parent.join("universe");
        let second_universe = second_parent.join("universe");
        fs::create_dir_all(&first_universe).expect("first universe root creates");
        fs::create_dir_all(&second_universe).expect("second universe root creates");
        initialize_frozen_fixture(&first_universe, true);
        initialize_frozen_fixture(&second_universe, false);
        let second_before = collect_all_files(&second_universe);
        let second_topology = collect_tree_topology(&second_universe);

        let output = Command::new(std::env::current_exe().expect("test executable resolves"))
            .arg("--exact")
            .arg("protocol19_install::tests::protocol19_install_subprocess_driver")
            .arg("--nocapture")
            .current_dir(&first_parent)
            .env(SUBPROCESS_MODE_ENV, "relative_route")
            .env(SUBPROCESS_SECOND_CWD_ENV, &second_parent)
            .output()
            .expect("relative-route child launches");
        assert!(
            output.status.success(),
            "relative-route child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            first_universe
                .join(INSTALL_DIRECTORY)
                .join(INSTALL_HEAD_FILE)
                .is_file()
        );
        assert!(!second_universe.join(INSTALL_DIRECTORY).exists());
        assert_eq!(collect_all_files(&second_universe), second_before);
        assert_eq!(collect_tree_topology(&second_universe), second_topology);
    }

    #[test]
    fn committed_world_rejects_tamper_without_repairing_any_bytes() {
        for mutation in 0..9 {
            let root = frozen_fixture(true);
            let source = ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED)
                .expect("source freezes");
            let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
                .expect("target manifest builds");
            let transform = ValidatedProtocol19MigrationTransform::derive(&source, &manifest)
                .expect("target transform derives");
            drop(prepare_or_recover(&transform, &manifest).expect("world prepares"));
            match mutation {
                0 => {
                    fs::remove_file(root.path().join(INSTALL_DIRECTORY).join(IDENTITY_MAP_FILE))
                        .expect("artifact removes");
                }
                1 => {
                    let cells = transform.cells();
                    let first = root
                        .path()
                        .join("cells")
                        .join(cells[0].cell_id())
                        .join("protocol-19-world-v21");
                    let second = root
                        .path()
                        .join("cells")
                        .join(cells[1].cell_id())
                        .join("protocol-19-world-v21");
                    let temporary = root.path().join("swapped-world-v21");
                    fs::rename(&first, &temporary).expect("first cell moves");
                    fs::rename(&second, &first).expect("second cell moves");
                    fs::rename(&temporary, &second).expect("first cell completes swap");
                }
                2 => {
                    let foreign = root.path().join("cells").join("foreign-cell");
                    fs::create_dir_all(&foreign).expect("foreign cell creates");
                    let source_namespace = root
                        .path()
                        .join("cells")
                        .join(transform.cells()[0].cell_id())
                        .join("protocol-19-world-v21");
                    copy_directory(&source_namespace, &foreign.join("protocol-19-world-v21"));
                }
                3 => {
                    fs::remove_dir_all(
                        root.path()
                            .join("cells")
                            .join(transform.cells()[0].cell_id())
                            .join("protocol-19-world-v21"),
                    )
                    .expect("committed cell namespace removes");
                }
                4 => {
                    fs::remove_file(
                        root.path()
                            .join("cells")
                            .join(transform.cells()[0].cell_id())
                            .join("protocol-19-world-v21")
                            .join("initialization-v21.head.json"),
                    )
                    .expect("committed cell head removes");
                }
                5 => {
                    let history = root
                        .path()
                        .join("protocol-19-directory-v3")
                        .join("history-v3.ndjson");
                    let mut file = fs::OpenOptions::new()
                        .append(true)
                        .open(&history)
                        .expect("directory history opens");
                    file.write_all(b"{\"torn\":")
                        .and_then(|()| file.sync_all())
                        .expect("trailing partial record persists");
                }
                6 => {
                    let namespace = root
                        .path()
                        .join("cells")
                        .join(transform.cells()[0].cell_id())
                        .join("protocol-19-world-v21");
                    fs::remove_file(namespace.join("initialization-v21.head.json"))
                        .expect("committed cell head removes");
                    fs::write(namespace.join("foreign-debris"), b"must remain")
                        .expect("foreign debris writes");
                }
                7 => {
                    fs::remove_file(root.path().join(INSTALL_DIRECTORY).join(INSTALL_LOCK_FILE))
                        .expect("global install lock removes");
                }
                8 => {
                    fs::remove_file(
                        root.path()
                            .join("protocol-19-directory-v3")
                            .join("writer.lock"),
                    )
                    .expect("directory writer lock removes");
                }
                _ => unreachable!(),
            }
            let mutated = collect_all_files(root.path());
            let mutated_topology = collect_tree_topology(root.path());
            let legacy = collect_legacy_files(root.path());
            assert!(prepare_or_recover(&transform, &manifest).is_err());
            assert_eq!(
                collect_all_files(root.path()),
                mutated,
                "committed mutation {mutation} was silently repaired"
            );
            assert_eq!(
                collect_tree_topology(root.path()),
                mutated_topology,
                "committed mutation {mutation} changed filesystem topology"
            );
            assert_eq!(collect_legacy_files(root.path()), legacy);
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_cell_container_cannot_redirect_migration_writes() {
        use std::os::unix::fs::symlink;

        let root = frozen_fixture(true);
        let external = tempdir().expect("external directory");
        let external_cells = external.path().join("redirected-cells");
        fs::rename(root.path().join("cells"), &external_cells)
            .expect("cell container moves outside universe");
        symlink(&external_cells, root.path().join("cells")).expect("cell symlink creates");
        let before_bytes = collect_all_files(external.path());
        let before_topology = collect_tree_topology(external.path());

        assert!(
            ValidatedFrozenProtocol18Source::acquire_existing(root.path(), TEST_SEED).is_err(),
            "frozen source must reject an escaped cell container"
        );
        assert_eq!(collect_all_files(external.path()), before_bytes);
        assert_eq!(collect_tree_topology(external.path()), before_topology);
        assert!(!external_cells.join(INSTALL_DIRECTORY).exists());
        for cell in crate::proof_cell_keys().expect("proof cells derive") {
            let cell_id = crate::celestial::cell_id(&cell).expect("cell ID derives");
            assert!(
                !external_cells
                    .join(cell_id)
                    .join("protocol-19-world-v21")
                    .exists()
            );
        }
    }

    #[test]
    fn complete_foreign_prepared_world_cannot_reseal_another_frozen_source() {
        let original_root = frozen_fixture(true);
        let original_source =
            ValidatedFrozenProtocol18Source::acquire_existing(original_root.path(), TEST_SEED)
                .expect("original source freezes");
        let manifest = crate::manifest_v5::build_validated_manifest_v5(TEST_SEED)
            .expect("target manifest builds");
        let original_transform =
            ValidatedProtocol19MigrationTransform::derive(&original_source, &manifest)
                .expect("original transform derives");
        drop(prepare_or_recover(&original_transform, &manifest).expect("original target prepares"));

        let foreign_root = frozen_fixture(false);
        let foreign_source =
            ValidatedFrozenProtocol18Source::acquire_existing(foreign_root.path(), TEST_SEED)
                .expect("foreign source freezes");
        let foreign_transform =
            ValidatedProtocol19MigrationTransform::derive(&foreign_source, &manifest)
                .expect("foreign transform derives");
        drop(prepare_or_recover(&foreign_transform, &manifest).expect("foreign target prepares"));

        replace_directory(
            &foreign_root.path().join(INSTALL_DIRECTORY),
            &original_root.path().join(INSTALL_DIRECTORY),
        );
        replace_directory(
            &foreign_root.path().join("protocol-19-directory-v3"),
            &original_root.path().join("protocol-19-directory-v3"),
        );
        for cell in original_transform.cells() {
            replace_directory(
                &foreign_root
                    .path()
                    .join("cells")
                    .join(cell.cell_id())
                    .join("protocol-19-world-v21"),
                &original_root
                    .path()
                    .join("cells")
                    .join(cell.cell_id())
                    .join("protocol-19-world-v21"),
            );
        }
        assert!(prepare_or_recover(&original_transform, &manifest).is_err());
    }

    fn replace_directory(source: &Path, destination: &Path) {
        fs::remove_dir_all(destination).expect("old target directory removes");
        copy_directory(source, destination);
    }

    fn copy_directory(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).expect("copy destination creates");
        for entry in fs::read_dir(source).expect("copy source reads") {
            let entry = entry.expect("copy entry reads");
            let destination = destination.join(entry.file_name());
            if entry.file_type().expect("copy type reads").is_dir() {
                copy_directory(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), destination).expect("copy file writes");
            }
        }
    }
}
