// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant staged/recovery slice of the isolated world-21 Store.
//!
//! The active Store never reads this namespace. The production-compiled staging
//! seam requires a source-bound, receipt-derived non-Serde target capability
//! and participates in the universe-wide prepared-install transaction, but has
//! no worker or activation entry point.

mod event_journal;

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use verse_protocol::{CellKeyV1, protocol_v19::Protocol19CompatibilityTuple};

use super::event_v17::{
    DraftCanonicalGridEventV17, DraftGridEventPayloadV17, DraftProductionAuthorityClaimV17,
    ValidatedCurrentGridEventAuthorityV17,
};
use super::lifecycle_v2::{
    LIFECYCLE_HEAD_FILE, LIFECYCLE_HISTORY_FILE, LifecycleInitializationFailpointV2,
    LifecycleRecordV2, LifecycleRuntimePreflightV2, LifecycleStoreV2, LifecycleV2Error,
    LifecycleWorldFrontierV2, MAX_STALE_LIFECYCLE_HEAD_TEMPS, is_lifecycle_head_temp,
};
use super::state::DraftGridTransferCellStateV2;
#[cfg(test)]
use super::state::ValidatedDraftGridTransferCellStateV21;
use crate::cell_directory::CellDirectoryError;
#[cfg(test)]
use crate::protocol19_migration::Protocol19World21StagingEvidence;
use crate::protocol19_migration::{
    Protocol19TargetLifecycleGenesisV2, Protocol19World21StagingCommitment,
    ValidatedProtocol19MigrationReceipt,
};

const STORE_DIRECTORY: &str = "protocol-19-world-v21";
const INITIALIZATION_HEAD_FILE: &str = "initialization-v21.head.json";
const IDENTITY_FILE: &str = "identity-v19.json";
const MANIFEST_FILE: &str = "manifest-v5.json";
const LIFECYCLE_GENESIS_FILE: &str = "lifecycle-v2.genesis.json";
const SNAPSHOT_FILE: &str = "snapshot-v21.json";
const EVENT_JOURNAL_FILE: &str = "events-v17.ndjson";
const WRITER_LOCK_FILE: &str = "writer-v21.lock";
const STORE_IDENTITY_SCHEMA_VERSION: u32 = 1;
const STORE_IDENTITY_HASH_DOMAIN: &[u8] = b"the-verse/world-21-store-identity/v1\0";
const INITIALIZATION_HEAD_SCHEMA_VERSION: u32 = 1;
const INITIALIZATION_HEAD_HASH_DOMAIN: &[u8] = b"the-verse/world-21-store-initialization-head/v1\0";
const MAX_INITIALIZATION_HEAD_BYTES: usize = 64 * 1_024;
const MAX_IDENTITY_BYTES: usize = 64 * 1_024;
const MAX_MANIFEST_BYTES: usize = 64 * 1_024;
const MAX_SNAPSHOT_BYTES: usize = 256 * 1_024 * 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWorld21StoreIdentity {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    universe_id: String,
    world_seed: String,
    cell_key: CellKeyV1,
    cell_id: String,
    manifest_hash: String,
    migration_anchor_hash: String,
    migration_receipt_hash: String,
    lifecycle_record_hash: String,
    snapshot_state_hash: String,
    active_world_hash: String,
    production_origin_root: String,
    identity_subset_root: String,
    legacy_event_schema_version: u32,
    legacy_event_sequence: u64,
    legacy_event_head_hash: String,
    identity_hash: String,
}

impl DraftWorld21StoreIdentity {
    fn new(
        state: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        expected_cell_key: &CellKeyV1,
        staging: &Protocol19World21StagingCommitment,
    ) -> Result<Self, DraftWorld21StoreError> {
        let base = state.base();
        let lifecycle = staging.lifecycle();
        let mut identity = Self {
            schema_version: STORE_IDENTITY_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            universe_id: manifest.universe_id().to_owned(),
            world_seed: manifest.world_seed().to_string(),
            cell_key: expected_cell_key.clone(),
            cell_id: base.cell_id.clone(),
            manifest_hash: manifest.manifest_hash().to_owned(),
            migration_anchor_hash: lifecycle.migration_anchor_hash().to_owned(),
            migration_receipt_hash: staging.migration_receipt_hash().to_owned(),
            lifecycle_record_hash: lifecycle.record_hash().to_owned(),
            snapshot_state_hash: state.state_hash().to_owned(),
            active_world_hash: state
                .calculate_active_world_hash()
                .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?,
            production_origin_root: staging.production_origin_root().to_owned(),
            identity_subset_root: staging.identity_subset_root().to_owned(),
            legacy_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
            legacy_event_sequence: base.event_sequence,
            legacy_event_head_hash: base.last_event_hash.clone(),
            identity_hash: String::new(),
        };
        identity.identity_hash = identity.calculate_hash()?;
        identity.validate(state, manifest, expected_cell_key, lifecycle)?;
        Ok(identity)
    }

    fn calculate_hash(&self) -> Result<String, DraftWorld21StoreError> {
        let mut material = self.clone();
        material.identity_hash.clear();
        hash_json(STORE_IDENTITY_HASH_DOMAIN, &material)
    }

    fn validate(
        &self,
        state: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        expected_cell_key: &CellKeyV1,
        lifecycle: &Protocol19TargetLifecycleGenesisV2,
    ) -> Result<(), DraftWorld21StoreError> {
        let base = state.base();
        crate::celestial::validate_cell_key(expected_cell_key)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let state_cell_key = crate::celestial::cell_key_from_address(&base.cell_address)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let expected_cell_id = crate::celestial::cell_id(expected_cell_key)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let empty_frontier =
            self.legacy_event_sequence == 0 && self.legacy_event_head_hash.is_empty();
        let populated_frontier =
            self.legacy_event_sequence > 0 && valid_hash(&self.legacy_event_head_hash);
        if self.schema_version != STORE_IDENTITY_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.universe_id != manifest.universe_id()
            || self.universe_id != base.universe_id
            || self.world_seed != manifest.world_seed().to_string()
            || self.world_seed != base.world_seed.to_string()
            || &self.cell_key != expected_cell_key
            || self.cell_key != state_cell_key
            || self.cell_key.universe_id != self.universe_id
            || self.cell_id != expected_cell_id
            || self.cell_id != base.cell_id
            || self.manifest_hash != manifest.manifest_hash()
            || self.manifest_hash != base.universe_manifest_hash
            || self.migration_anchor_hash != lifecycle.migration_anchor_hash()
            || !valid_hash(&self.migration_anchor_hash)
            || !valid_hash(&self.migration_receipt_hash)
            || self.lifecycle_record_hash != lifecycle.record_hash()
            || !valid_hash(&self.lifecycle_record_hash)
            || self.snapshot_state_hash != state.state_hash()
            || !valid_hash(&self.snapshot_state_hash)
            || self.active_world_hash
                != state
                    .calculate_active_world_hash()
                    .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?
            || self.active_world_hash != lifecycle.active_world_hash()
            || !valid_hash(&self.active_world_hash)
            || self.production_origin_root != lifecycle.production_origin_root()
            || !valid_hash(&self.production_origin_root)
            || self.identity_subset_root != lifecycle.identity_subset_root()
            || !valid_hash(&self.identity_subset_root)
            || self.legacy_event_schema_version != crate::event::EVENT_SCHEMA_VERSION
            || self.legacy_event_sequence != base.event_sequence
            || self.legacy_event_head_hash != base.last_event_hash
            || lifecycle.manifest_hash() != self.manifest_hash
            || lifecycle.cell_key() != expected_cell_key
            || lifecycle.cell_id() != self.cell_id
            || lifecycle.snapshot_state_hash() != self.snapshot_state_hash
            || lifecycle.authority_fencing_token() != base.fencing_token
            || lifecycle.legacy_event_sequence() != self.legacy_event_sequence
            || lifecycle.legacy_event_head_hash() != self.legacy_event_head_hash
            || !(empty_frontier || populated_frontier)
            || !valid_hash(&self.identity_hash)
            || self.identity_hash != self.calculate_hash()?
        {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store identity does not match its manifest, snapshot, and legacy frontier"
                    .into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWorld21InitializationHead {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    identity_hash: String,
    manifest_hash: String,
    migration_anchor_hash: String,
    migration_receipt_hash: String,
    lifecycle_record_hash: String,
    snapshot_state_hash: String,
    event_journal_byte_length: u64,
    head_hash: String,
}

impl DraftWorld21InitializationHead {
    fn new(identity: &DraftWorld21StoreIdentity) -> Result<Self, DraftWorld21StoreError> {
        let mut head = Self {
            schema_version: INITIALIZATION_HEAD_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            identity_hash: identity.identity_hash.clone(),
            manifest_hash: identity.manifest_hash.clone(),
            migration_anchor_hash: identity.migration_anchor_hash.clone(),
            migration_receipt_hash: identity.migration_receipt_hash.clone(),
            lifecycle_record_hash: identity.lifecycle_record_hash.clone(),
            snapshot_state_hash: identity.snapshot_state_hash.clone(),
            event_journal_byte_length: 0,
            head_hash: String::new(),
        };
        head.head_hash = head.calculate_hash()?;
        head.validate(identity, 0)?;
        Ok(head)
    }

    fn calculate_hash(&self) -> Result<String, DraftWorld21StoreError> {
        let mut material = self.clone();
        material.head_hash.clear();
        hash_json(INITIALIZATION_HEAD_HASH_DOMAIN, &material)
    }

    fn validate(
        &self,
        identity: &DraftWorld21StoreIdentity,
        event_journal_byte_length: u64,
    ) -> Result<(), DraftWorld21StoreError> {
        if self.schema_version != INITIALIZATION_HEAD_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.identity_hash != identity.identity_hash
            || self.manifest_hash != identity.manifest_hash
            || self.migration_anchor_hash != identity.migration_anchor_hash
            || self.migration_receipt_hash != identity.migration_receipt_hash
            || self.lifecycle_record_hash != identity.lifecycle_record_hash
            || self.snapshot_state_hash != identity.snapshot_state_hash
            || self.event_journal_byte_length != event_journal_byte_length
            || self.event_journal_byte_length != 0
            || !valid_hash(&self.identity_hash)
            || !valid_hash(&self.manifest_hash)
            || !valid_hash(&self.migration_anchor_hash)
            || !valid_hash(&self.migration_receipt_hash)
            || !valid_hash(&self.lifecycle_record_hash)
            || !valid_hash(&self.snapshot_state_hash)
            || !valid_hash(&self.head_hash)
            || self.head_hash != self.calculate_hash()?
        {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store initialization head does not match its authority files".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DraftWorld21InitializationFailpoint {
    NamespaceDirectorySynced,
    IdentitySynced,
    ManifestSynced,
    LifecycleGenesisSynced,
    SnapshotSynced,
    EventJournalSynced,
    EventBoundaryJournalSynced,
    EventHeadSynced,
    HeadTempSyncedBeforeRename,
    HeadRenamedBeforeDirectorySync,
    HeadDirectorySyncedBeforeMemory,
}

#[derive(Debug, Error)]
pub(crate) enum DraftWorld21StoreError {
    #[error("world-21 Store is invalid: {0}")]
    Invalid(String),
    #[error("world-21 Store file is too large: {0}")]
    TooLarge(PathBuf),
    #[error("world-21 Store I/O failed at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("world-21 Store already has a writer")]
    WriterConflict,
    #[error("world-21 Store injected initialization failure: {0:?}")]
    Injected(DraftWorld21InitializationFailpoint),
    #[error("world-21 Store injected append failure: {0}")]
    InjectedAppend(&'static str),
    #[error(transparent)]
    Lifecycle(#[from] LifecycleV2Error),
    #[error(transparent)]
    Directory(#[from] CellDirectoryError),
}

/// Non-Serde target capability that couples one validated world-21 state to
/// the exact cell commitment in a canonical migration receipt. Staging this
/// target remains dormant and cannot activate protocol 19.
#[derive(Debug)]
pub(crate) struct ValidatedWorld21StagingTarget<'state, 'manifest> {
    state: &'state DraftGridTransferCellStateV2,
    manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
    commitment: Protocol19World21StagingCommitment,
}

impl<'state, 'manifest> ValidatedWorld21StagingTarget<'state, 'manifest> {
    pub(crate) fn from_migration_receipt(
        receipt: &ValidatedProtocol19MigrationReceipt<'_, '_>,
        cell: &'state super::migration_transform::Protocol19TransformedCell,
        manifest: &'manifest crate::manifest_v5::ValidatedUniverseManifestV5,
    ) -> Result<Self, DraftWorld21StoreError> {
        let state = cell
            .validate(manifest)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let commitment = receipt
            .bind_cell(cell)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        Ok(Self {
            state: state.state(),
            manifest: state.manifest(),
            commitment,
        })
    }

    #[cfg(test)]
    pub(crate) fn from_receipt(
        receipt_bytes: &[u8],
        state: &ValidatedDraftGridTransferCellStateV21<'state, 'manifest>,
        expected_cell_key: &CellKeyV1,
    ) -> Result<Self, DraftWorld21StoreError> {
        let base = state.state().base();
        let state_cell_key = crate::celestial::cell_key_from_address(&base.cell_address)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        if &state_cell_key != expected_cell_key {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 staging route does not match the validated target state".into(),
            ));
        }
        let active_world_hash = state
            .state()
            .calculate_active_world_hash()
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let evidence = Protocol19World21StagingEvidence {
            manifest_hash: state.manifest_hash(),
            universe_id: &base.universe_id,
            world_seed: base.world_seed,
            cell_key: expected_cell_key,
            cell_id: &base.cell_id,
            authority_fencing_token: base.fencing_token,
            snapshot_state_hash: state.state().state_hash(),
            active_world_hash: &active_world_hash,
            legacy_event_sequence: base.event_sequence,
            legacy_event_head_hash: &base.last_event_hash,
        };
        let commitment =
            crate::protocol19_migration::bind_world21_staging_target(receipt_bytes, &evidence)
                .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        Ok(Self {
            state: state.state(),
            manifest: state.manifest(),
            commitment,
        })
    }

    #[cfg(test)]
    fn for_test(
        state: &ValidatedDraftGridTransferCellStateV21<'state, 'manifest>,
        expected_cell_key: &CellKeyV1,
        migration_anchor_hash: &str,
    ) -> Result<Self, DraftWorld21StoreError> {
        let base = state.state().base();
        let active_world_hash = state
            .state()
            .calculate_active_world_hash()
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let evidence = Protocol19World21StagingEvidence {
            manifest_hash: state.manifest_hash(),
            universe_id: &base.universe_id,
            world_seed: base.world_seed,
            cell_key: expected_cell_key,
            cell_id: &base.cell_id,
            authority_fencing_token: base.fencing_token,
            snapshot_state_hash: state.state().state_hash(),
            active_world_hash: &active_world_hash,
            legacy_event_sequence: base.event_sequence,
            legacy_event_head_hash: &base.last_event_hash,
        };
        let commitment = crate::protocol19_migration::test_world21_staging_commitment(
            &evidence,
            migration_anchor_hash,
        );
        Ok(Self {
            state: state.state(),
            manifest: state.manifest(),
            commitment,
        })
    }
}

#[derive(Debug)]
pub(crate) struct DraftWorld21Store {
    root: PathBuf,
    initialization_head: DraftWorld21InitializationHead,
    identity: DraftWorld21StoreIdentity,
    lifecycle: Protocol19TargetLifecycleGenesisV2,
    manifest: crate::manifest_v5::ValidatedUniverseManifestV5,
    state: DraftGridTransferCellStateV2,
    lifecycle_initial_frontier: LifecycleWorldFrontierV2,
    events: event_journal::DraftWorld21EventJournal,
    lifecycle_runtime: Option<LifecycleStoreV2>,
    _writer_lock: File,
}

struct RecoveredImmutableWorld21 {
    initialization_head: DraftWorld21InitializationHead,
    identity: DraftWorld21StoreIdentity,
    lifecycle: Protocol19TargetLifecycleGenesisV2,
    manifest: crate::manifest_v5::ValidatedUniverseManifestV5,
    snapshot: DraftGridTransferCellStateV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PreparedWorld21CellEvidence {
    pub(crate) cell_key: CellKeyV1,
    pub(crate) cell_id: String,
    pub(crate) initialization_head_hash: String,
    pub(crate) identity_hash: String,
    pub(crate) lifecycle_record_hash: String,
    pub(crate) snapshot_state_hash: String,
    pub(crate) active_world_hash: String,
    pub(crate) migration_receipt_hash: String,
}

impl DraftWorld21Store {
    pub(crate) fn preflight_active_file_set(
        base_root: impl AsRef<Path>,
    ) -> Result<(), DraftWorld21StoreError> {
        let base_root = base_root.as_ref();
        validate_real_cell_root(base_root)?;
        let root = base_root.join(STORE_DIRECTORY);
        let metadata = fs::symlink_metadata(&root).map_err(|source| io_error(&root, source))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(DraftWorld21StoreError::Invalid(
                "activated world-21 namespace is not a real directory".into(),
            ));
        }
        validate_active_file_set(&root)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn preflight_active_contents(
        base_root: impl AsRef<Path>,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        migration_anchor_hash: &str,
        active_head_hash: &str,
        expected: &PreparedWorld21CellEvidence,
        expected_production_origin_root: &str,
        expected_identity_subset_root: &str,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
    ) -> Result<LifecycleRuntimePreflightV2, DraftWorld21StoreError> {
        let base_root = base_root.as_ref();
        validate_real_cell_root(base_root)?;
        let root = base_root.join(STORE_DIRECTORY);
        validate_active_file_set(&root)?;
        let _writer_lock = acquire_existing_writer_lock(&root)?;
        let immutable =
            recover_immutable(&root, manifest, &expected.cell_key, migration_anchor_hash)?;
        let state = event_journal::DraftWorld21EventJournal::preflight_with_history(
            &root,
            &immutable.identity,
            &immutable.initialization_head,
            &immutable.snapshot,
            &immutable.manifest,
            directory_history,
        )?;
        let initial_frontier = LifecycleWorldFrontierV2::from_world(
            immutable.snapshot.base(),
            immutable.snapshot.state_hash(),
            immutable.identity.active_world_hash.clone(),
        );
        let frontier = lifecycle_world_frontier_from_state(&state)?;
        let assignment = directory_history.current_cell_assignment(&expected.cell_id)?;
        let lifecycle_commitment = LifecycleStoreV2::preflight_existing(
            &root,
            &immutable.lifecycle,
            active_head_hash,
            &assignment,
            &initial_frontier,
            &frontier,
            directory_history,
        )?;
        if lifecycle_commitment.committed().is_none() {
            if frontier != initial_frontier {
                return Err(DraftWorld21StoreError::Invalid(
                    "an uninitialized lifecycle is not at its immutable world genesis".into(),
                ));
            }
            LifecycleStoreV2::expected_initial_record(
                &immutable.lifecycle,
                active_head_hash,
                &assignment,
                &initial_frontier,
            )?;
        }
        let evidence = PreparedWorld21CellEvidence {
            cell_key: immutable.identity.cell_key.clone(),
            cell_id: immutable.identity.cell_id.clone(),
            initialization_head_hash: immutable.initialization_head.head_hash.clone(),
            identity_hash: immutable.identity.identity_hash.clone(),
            lifecycle_record_hash: immutable.lifecycle.record_hash().to_owned(),
            snapshot_state_hash: immutable.identity.snapshot_state_hash.clone(),
            active_world_hash: immutable.identity.active_world_hash.clone(),
            migration_receipt_hash: immutable.identity.migration_receipt_hash.clone(),
        };
        if evidence != *expected
            || immutable.identity.production_origin_root != expected_production_origin_root
            || immutable.identity.identity_subset_root != expected_identity_subset_root
            || state.state_hash().is_empty()
        {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store differs from its active-head or receipt commitment".into(),
            ));
        }
        Ok(lifecycle_commitment)
    }

    pub(crate) fn stage_from_migration(
        root: impl AsRef<Path>,
        target: &ValidatedWorld21StagingTarget<'_, '_>,
    ) -> Result<Self, DraftWorld21StoreError> {
        Self::stage_from_migration_with_failpoint(root, target, None)
    }

    pub(crate) fn open_from_migration(
        base_root: impl AsRef<Path>,
        target: &ValidatedWorld21StagingTarget<'_, '_>,
    ) -> Result<Self, DraftWorld21StoreError> {
        let base_root = base_root.as_ref();
        validate_real_cell_root(base_root)?;
        let root = base_root.join(STORE_DIRECTORY);
        let metadata = fs::symlink_metadata(&root).map_err(|source| io_error(&root, source))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(DraftWorld21StoreError::Invalid(
                "committed world-21 namespace is not a real directory".into(),
            ));
        }
        let writer_lock = acquire_existing_writer_lock(&root)?;
        let initialization_head = root.join(INITIALIZATION_HEAD_FILE);
        if !initialization_head
            .try_exists()
            .map_err(|source| io_error(&initialization_head, source))?
        {
            return Err(DraftWorld21StoreError::Invalid(
                "committed world-21 Store has no initialization head".into(),
            ));
        }
        let lifecycle = target.commitment.lifecycle();
        Self::recover_locked(
            root,
            writer_lock,
            target.manifest,
            lifecycle.cell_key(),
            lifecycle.migration_anchor_hash(),
            Some(&target.commitment),
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_from_active_head(
        base_root: impl AsRef<Path>,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        migration_anchor_hash: &str,
        active_head_hash: &str,
        expected: &PreparedWorld21CellEvidence,
        expected_production_origin_root: &str,
        expected_identity_subset_root: &str,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
    ) -> Result<Self, DraftWorld21StoreError> {
        let base_root = base_root.as_ref();
        validate_real_cell_root(base_root)?;
        let root = base_root.join(STORE_DIRECTORY);
        let metadata = fs::symlink_metadata(&root).map_err(|source| io_error(&root, source))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(DraftWorld21StoreError::Invalid(
                "activated world-21 namespace is not a real directory".into(),
            ));
        }
        validate_active_file_set(&root)?;
        let writer_lock = acquire_existing_writer_lock(&root)?;
        let store = Self::recover_locked(
            root,
            writer_lock,
            manifest,
            &expected.cell_key,
            migration_anchor_hash,
            None,
            Some(directory_history),
        )?;
        if store.prepared_install_evidence() != *expected {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store differs from the cell selected by the active head".into(),
            ));
        }
        if store.identity.production_origin_root != expected_production_origin_root
            || store.identity.identity_subset_root != expected_identity_subset_root
        {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store identity differs from its canonical migration receipt".into(),
            ));
        }
        let _ = active_head_hash;
        validate_active_file_set(&store.root)?;
        Ok(store)
    }

    #[cfg(test)]
    fn create_for_test(
        root: impl AsRef<Path>,
        state: &ValidatedDraftGridTransferCellStateV21<'_, '_>,
        expected_cell_key: &CellKeyV1,
        migration_anchor_hash: &str,
    ) -> Result<Self, DraftWorld21StoreError> {
        let target = ValidatedWorld21StagingTarget::for_test(
            state,
            expected_cell_key,
            migration_anchor_hash,
        )?;
        Self::stage_from_migration(root, &target)
    }

    #[cfg(test)]
    fn create_for_test_with_failpoint(
        root: impl AsRef<Path>,
        state: &ValidatedDraftGridTransferCellStateV21<'_, '_>,
        expected_cell_key: &CellKeyV1,
        migration_anchor_hash: &str,
        failpoint: Option<DraftWorld21InitializationFailpoint>,
    ) -> Result<Self, DraftWorld21StoreError> {
        let target = ValidatedWorld21StagingTarget::for_test(
            state,
            expected_cell_key,
            migration_anchor_hash,
        )?;
        Self::stage_from_migration_with_failpoint(root, &target, failpoint)
    }

    fn stage_from_migration_with_failpoint(
        root: impl AsRef<Path>,
        target: &ValidatedWorld21StagingTarget<'_, '_>,
        mut failpoint: Option<DraftWorld21InitializationFailpoint>,
    ) -> Result<Self, DraftWorld21StoreError> {
        let state = target.state;
        let manifest = target.manifest;
        let lifecycle = target.commitment.lifecycle();
        let expected_cell_key = lifecycle.cell_key();
        let migration_anchor_hash = lifecycle.migration_anchor_hash();
        let base_root = root.as_ref();
        validate_real_cell_root(base_root)?;
        let root = base_root.join(STORE_DIRECTORY);
        fs::create_dir_all(&root).map_err(|source| io_error(&root, source))?;
        sync_directory(base_root)?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::NamespaceDirectorySynced,
        )?;
        let writer_lock = acquire_writer_lock(&root)?;
        let initialization_head_path = root.join(INITIALIZATION_HEAD_FILE);
        if initialization_head_path
            .try_exists()
            .map_err(|source| io_error(&initialization_head_path, source))?
        {
            return Self::recover_locked(
                root,
                writer_lock,
                manifest,
                expected_cell_key,
                migration_anchor_hash,
                Some(&target.commitment),
                None,
            );
        }
        reset_incomplete_initialization(&root)?;
        let identity =
            DraftWorld21StoreIdentity::new(state, manifest, expected_cell_key, &target.commitment)?;
        let initialization_head = DraftWorld21InitializationHead::new(&identity)?;
        let identity_bytes = serde_json::to_vec(&identity)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let initialization_head_bytes = serde_json::to_vec(&initialization_head)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let manifest_bytes = crate::manifest_v5::encode_manifest_v5(manifest)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let lifecycle_bytes = lifecycle
            .encode_canonical()
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let validated_state = state
            .validate_world_v21(manifest)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let snapshot_bytes =
            DraftGridTransferCellStateV2::encode_world_v21_canonical(&validated_state)
                .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        atomic_write(&root.join(IDENTITY_FILE), &identity_bytes)?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::IdentitySynced,
        )?;
        atomic_write(&root.join(MANIFEST_FILE), &manifest_bytes)?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::ManifestSynced,
        )?;
        atomic_write(&root.join(LIFECYCLE_GENESIS_FILE), &lifecycle_bytes)?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::LifecycleGenesisSynced,
        )?;
        atomic_write(&root.join(SNAPSHOT_FILE), &snapshot_bytes)?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::SnapshotSynced,
        )?;
        create_synced_empty(&root.join(EVENT_JOURNAL_FILE))?;
        sync_directory(&root)?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::EventJournalSynced,
        )?;
        create_synced_empty(&root.join(event_journal::EVENT_BOUNDARY_FILE))?;
        sync_directory(&root)?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::EventBoundaryJournalSynced,
        )?;
        event_journal::DraftWorld21EventJournal::initialize_head_file(
            &root,
            &identity,
            &initialization_head,
        )?;
        inject_initialization_failure(
            &mut failpoint,
            DraftWorld21InitializationFailpoint::EventHeadSynced,
        )?;
        persist_initialization_head(&root, &initialization_head_bytes, &mut failpoint)?;
        Self::recover_locked(
            root,
            writer_lock,
            manifest,
            expected_cell_key,
            migration_anchor_hash,
            Some(&target.commitment),
            None,
        )
    }

    fn open_for_test(
        root: impl AsRef<Path>,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        expected_cell_key: &CellKeyV1,
        migration_anchor_hash: &str,
    ) -> Result<Self, DraftWorld21StoreError> {
        let root = root.as_ref().join(STORE_DIRECTORY);
        let writer_lock = acquire_writer_lock(&root)?;
        Self::recover_locked(
            root,
            writer_lock,
            manifest,
            expected_cell_key,
            migration_anchor_hash,
            None,
            None,
        )
    }

    fn open_with_event_replay_for_test(
        root: impl AsRef<Path>,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        expected_cell_key: &CellKeyV1,
        migration_anchor_hash: &str,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
    ) -> Result<Self, DraftWorld21StoreError> {
        let root = root.as_ref().join(STORE_DIRECTORY);
        let writer_lock = acquire_writer_lock(&root)?;
        Self::recover_locked(
            root,
            writer_lock,
            manifest,
            expected_cell_key,
            migration_anchor_hash,
            None,
            Some(directory_history),
        )
    }

    fn recover_locked(
        root: PathBuf,
        writer_lock: File,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        expected_cell_key: &CellKeyV1,
        migration_anchor_hash: &str,
        expected_staging: Option<&Protocol19World21StagingCommitment>,
        directory_history: Option<&crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3>,
    ) -> Result<Self, DraftWorld21StoreError> {
        if !valid_hash(migration_anchor_hash) {
            return Err(DraftWorld21StoreError::Invalid(
                "migration anchor hash is not canonical BLAKE3 text".into(),
            ));
        }
        let initialization_head_bytes = read_bounded(
            &root.join(INITIALIZATION_HEAD_FILE),
            MAX_INITIALIZATION_HEAD_BYTES,
        )?;
        let initialization_head =
            serde_json::from_slice::<DraftWorld21InitializationHead>(&initialization_head_bytes)
                .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let canonical_initialization_head = serde_json::to_vec(&initialization_head)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        if canonical_initialization_head != initialization_head_bytes {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store initialization head bytes are not canonical".into(),
            ));
        }
        let manifest_bytes = read_bounded(&root.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)?;
        let reopened_manifest =
            crate::manifest_v5::decode_manifest_v5(&manifest_bytes, manifest.world_seed())
                .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        if reopened_manifest.document() != manifest.document() {
            return Err(DraftWorld21StoreError::Invalid(
                "persisted manifest 5 differs from the expected capability".into(),
            ));
        }
        let lifecycle_bytes = read_bounded(
            &root.join(LIFECYCLE_GENESIS_FILE),
            crate::protocol19_migration::MAX_TARGET_LIFECYCLE_BYTES,
        )?;
        let lifecycle =
            crate::protocol19_migration::decode_target_lifecycle_genesis(&lifecycle_bytes)
                .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        if lifecycle.migration_anchor_hash() != migration_anchor_hash
            || lifecycle.manifest_hash() != reopened_manifest.manifest_hash()
            || lifecycle.cell_key() != expected_cell_key
        {
            return Err(DraftWorld21StoreError::Invalid(
                "persisted lifecycle genesis differs from the routed migration target".into(),
            ));
        }
        let snapshot_bytes = read_bounded(&root.join(SNAPSHOT_FILE), MAX_SNAPSHOT_BYTES)?;
        let state = DraftGridTransferCellStateV2::decode_world_v21_canonical(
            &snapshot_bytes,
            &reopened_manifest,
        )
        .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let identity_bytes = read_bounded(&root.join(IDENTITY_FILE), MAX_IDENTITY_BYTES)?;
        let identity = serde_json::from_slice::<DraftWorld21StoreIdentity>(&identity_bytes)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let canonical_identity = serde_json::to_vec(&identity)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        if canonical_identity != identity_bytes {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store identity bytes are not canonical".into(),
            ));
        }
        identity.validate(&state, &reopened_manifest, expected_cell_key, &lifecycle)?;
        initialization_head.validate(&identity, 0)?;
        if let Some(expected) = expected_staging
            && (lifecycle != *expected.lifecycle()
                || identity.migration_receipt_hash != expected.migration_receipt_hash()
                || identity.production_origin_root != expected.production_origin_root()
                || identity.identity_subset_root != expected.identity_subset_root())
        {
            return Err(DraftWorld21StoreError::Invalid(
                "existing world-21 Store belongs to another migration target".into(),
            ));
        }
        let lifecycle_initial_frontier = LifecycleWorldFrontierV2::from_world(
            state.base(),
            state.state_hash(),
            identity.active_world_hash.clone(),
        );
        let (events, state) = match directory_history {
            Some(directory_history) => {
                event_journal::DraftWorld21EventJournal::recover_with_history(
                    &root,
                    &identity,
                    &initialization_head,
                    &state,
                    &reopened_manifest,
                    directory_history,
                )?
            }
            None => (
                event_journal::DraftWorld21EventJournal::recover_empty(
                    &root,
                    &identity,
                    &initialization_head,
                    &state,
                )?,
                state,
            ),
        };
        Ok(Self {
            root,
            initialization_head,
            identity,
            lifecycle,
            manifest: reopened_manifest,
            state,
            lifecycle_initial_frontier,
            events,
            lifecycle_runtime: None,
            _writer_lock: writer_lock,
        })
    }

    fn append_live_event_for_test(
        &mut self,
        event: &super::event_v17::DraftCanonicalGridEventV17,
        authority: &super::event_v17::ValidatedCurrentGridEventAuthorityV17<'_, '_>,
    ) -> Result<super::dispatcher_v17::DraftGridEventProofV17, DraftWorld21StoreError> {
        let application = self
            .events
            .append_live(&self.state, &self.manifest, event, authority)?;
        self.state = application.next_state;
        Ok(application.proof)
    }

    fn set_append_failpoint_for_test(
        &mut self,
        failpoint: event_journal::DraftWorld21AppendFailpoint,
    ) {
        self.events.set_failpoint(failpoint);
    }

    fn committed_event17_count(&self) -> u64 {
        self.events.committed_event_count()
    }

    fn state(&self) -> &DraftGridTransferCellStateV2 {
        &self.state
    }

    fn identity(&self) -> &DraftWorld21StoreIdentity {
        &self.identity
    }

    fn initialization_head(&self) -> &DraftWorld21InitializationHead {
        &self.initialization_head
    }

    fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn lifecycle_world_frontier(
        &self,
    ) -> Result<LifecycleWorldFrontierV2, DraftWorld21StoreError> {
        lifecycle_world_frontier_from_state(&self.state)
    }

    pub(crate) fn expected_initial_lifecycle_record(
        &self,
        active_head_hash: &str,
        assignment: &crate::cell_directory_v3::ValidatedCurrentCellAssignmentV3<'_>,
    ) -> Result<crate::grid_handoff_v2::lifecycle_v2::LifecycleRecordV2, DraftWorld21StoreError>
    {
        LifecycleStoreV2::expected_initial_record(
            &self.lifecycle,
            active_head_hash,
            assignment,
            &self.lifecycle_initial_frontier,
        )
        .map_err(Into::into)
    }

    pub(crate) fn prepare_initial_lifecycle_append(
        &self,
        active_head_hash: &str,
        assignment: &crate::cell_directory_v3::ValidatedCurrentCellAssignmentV3<'_>,
    ) -> Result<
        crate::grid_handoff_v2::lifecycle_v2::PreparedLifecycleAppendV2,
        DraftWorld21StoreError,
    > {
        LifecycleStoreV2::prepare_initial_append(
            &self.lifecycle,
            active_head_hash,
            assignment,
            &self.lifecycle_initial_frontier,
        )
        .map_err(Into::into)
    }

    pub(crate) fn initialize_prepared_lifecycle(
        &mut self,
        active_head_hash: &str,
        prepared: crate::grid_handoff_v2::lifecycle_v2::PreparedLifecycleAppendV2,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
    ) -> Result<
        crate::grid_handoff_v2::lifecycle_v2::LifecycleHeadCommitmentV2,
        DraftWorld21StoreError,
    > {
        let runtime = LifecycleStoreV2::initialize_prepared(
            &self.root,
            &self.lifecycle,
            active_head_hash,
            prepared,
            directory_history,
        )?;
        let commitment = runtime.commitment();
        self.lifecycle_runtime = Some(runtime);
        validate_active_file_set(&self.root)?;
        Ok(commitment)
    }

    pub(crate) fn initialize_prepared_lifecycle_with_failpoint(
        &mut self,
        active_head_hash: &str,
        prepared: crate::grid_handoff_v2::lifecycle_v2::PreparedLifecycleAppendV2,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
        failpoint: LifecycleInitializationFailpointV2,
    ) -> Result<
        crate::grid_handoff_v2::lifecycle_v2::LifecycleHeadCommitmentV2,
        DraftWorld21StoreError,
    > {
        let runtime = LifecycleStoreV2::initialize_prepared_with_failpoint(
            &self.root,
            &self.lifecycle,
            active_head_hash,
            prepared,
            directory_history,
            failpoint,
        )?;
        let commitment = runtime.commitment();
        self.lifecycle_runtime = Some(runtime);
        validate_active_file_set(&self.root)?;
        Ok(commitment)
    }

    pub(crate) fn open_or_initialize_lifecycle(
        &mut self,
        active_head_hash: &str,
        assignment: &crate::cell_directory_v3::ValidatedCurrentCellAssignmentV3<'_>,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
    ) -> Result<
        crate::grid_handoff_v2::lifecycle_v2::LifecycleHeadCommitmentV2,
        DraftWorld21StoreError,
    > {
        let frontier = self.lifecycle_world_frontier()?;
        let runtime = LifecycleStoreV2::open_or_initialize(
            &self.root,
            &self.lifecycle,
            active_head_hash,
            assignment,
            &self.lifecycle_initial_frontier,
            &frontier,
            directory_history,
        )?;
        let commitment = runtime.commitment();
        self.lifecycle_runtime = Some(runtime);
        validate_active_file_set(&self.root)?;
        Ok(commitment)
    }

    pub(crate) fn lifecycle_commitment(
        &self,
    ) -> Result<
        crate::grid_handoff_v2::lifecycle_v2::LifecycleHeadCommitmentV2,
        DraftWorld21StoreError,
    > {
        Ok(self
            .lifecycle_runtime
            .as_ref()
            .ok_or_else(|| DraftWorld21StoreError::Invalid("lifecycle runtime is not open".into()))?
            .commitment())
    }

    pub(crate) fn prepare_lifecycle_append(
        &mut self,
        record: crate::grid_handoff_v2::lifecycle_v2::LifecycleRecordV2,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
    ) -> Result<
        crate::grid_handoff_v2::lifecycle_v2::PreparedLifecycleAppendV2,
        DraftWorld21StoreError,
    > {
        self.lifecycle_runtime
            .as_mut()
            .ok_or_else(|| DraftWorld21StoreError::Invalid("lifecycle runtime is not open".into()))?
            .prepare_append(record, directory_history)
            .map_err(Into::into)
    }

    #[cfg(test)]
    pub(crate) fn prepare_resealed_non_state_machine_lifecycle_successor_for_test(
        &self,
        record: crate::grid_handoff_v2::lifecycle_v2::LifecycleRecordV2,
    ) -> Result<
        crate::grid_handoff_v2::lifecycle_v2::PreparedLifecycleAppendV2,
        DraftWorld21StoreError,
    > {
        self.lifecycle_runtime
            .as_ref()
            .ok_or_else(|| DraftWorld21StoreError::Invalid("lifecycle runtime is not open".into()))?
            .prepare_resealed_non_state_machine_successor_for_test(record)
            .map_err(Into::into)
    }

    pub(crate) fn materialize_lifecycle_append(
        &mut self,
        prepared: crate::grid_handoff_v2::lifecycle_v2::PreparedLifecycleAppendV2,
        directory_history: &crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3,
    ) -> Result<(), DraftWorld21StoreError> {
        self.lifecycle_runtime
            .as_mut()
            .ok_or_else(|| DraftWorld21StoreError::Invalid("lifecycle runtime is not open".into()))?
            .materialize_prepared(prepared, directory_history)
            .map_err(Into::into)
    }

    pub(crate) fn lifecycle_record(&self) -> Result<&LifecycleRecordV2, DraftWorld21StoreError> {
        self.lifecycle_runtime
            .as_ref()
            .map(LifecycleStoreV2::current)
            .ok_or_else(|| {
                DraftWorld21StoreError::Invalid(
                    "world-21 Store has no activated lifecycle runtime".into(),
                )
            })
    }

    #[cfg(test)]
    pub(crate) fn set_lifecycle_failpoint_for_test(
        &mut self,
        failpoint: super::lifecycle_v2::LifecycleAppendFailpointV2,
    ) -> Result<(), DraftWorld21StoreError> {
        self.lifecycle_runtime
            .as_mut()
            .ok_or_else(|| {
                DraftWorld21StoreError::Invalid(
                    "world-21 Store has no activated lifecycle runtime".into(),
                )
            })?
            .set_failpoint(failpoint);
        Ok(())
    }

    pub(crate) fn background_production_is_runnable(&self) -> Result<bool, DraftWorld21StoreError> {
        self.state
            .base()
            .background_production_is_runnable()
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))
    }

    pub(crate) fn next_production_occurrence_at(
        &self,
        scheduled_for_unix_ms: u64,
    ) -> Result<crate::event::ProductionScheduleOccurrence, DraftWorld21StoreError> {
        self.state
            .base()
            .next_production_occurrence_at(scheduled_for_unix_ms)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))
    }

    pub(crate) fn append_production_occurrence(
        &mut self,
        occurrence: crate::event::ProductionScheduleOccurrence,
        accepted_trusted_at_unix_ms: u64,
        authority: &crate::cell_directory_v3::ValidatedCurrentCellAuthorityV3<'_>,
    ) -> Result<(), DraftWorld21StoreError> {
        let live_authority = ValidatedCurrentGridEventAuthorityV17::Production(authority);
        let event = DraftCanonicalGridEventV17::new_live_world_v21_system_for_store(
            &self.state,
            &self.manifest,
            format!(
                "production-{}-{}",
                occurrence.lifecycle_generation, occurrence.production_quantum_sequence
            ),
            accepted_trusted_at_unix_ms,
            DraftGridEventPayloadV17::ProductionQuantumCommitted {
                occurrence,
                accepted_trusted_at_unix_ms,
                authority: DraftProductionAuthorityClaimV17::from_validated(authority.validated()),
            },
            &live_authority,
        )
        .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        let application =
            self.events
                .append_live(&self.state, &self.manifest, &event, &live_authority)?;
        self.state = application.next_state;
        drop(application.proof);
        Ok(())
    }

    pub(crate) fn committed_production_accepted_trusted_at(
        &self,
        occurrence: &crate::event::ProductionScheduleOccurrence,
    ) -> Result<u64, DraftWorld21StoreError> {
        let (_, _, proof) =
            super::state::reconcile_imported_production_occurrence_v2(&self.state, occurrence)
                .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
        Ok(proof.accepted_trusted_at_unix_ms)
    }

    pub(crate) fn prepared_install_evidence(&self) -> PreparedWorld21CellEvidence {
        PreparedWorld21CellEvidence {
            cell_key: self.identity.cell_key.clone(),
            cell_id: self.identity.cell_id.clone(),
            initialization_head_hash: self.initialization_head.head_hash.clone(),
            identity_hash: self.identity.identity_hash.clone(),
            lifecycle_record_hash: self.lifecycle.record_hash().to_owned(),
            snapshot_state_hash: self.identity.snapshot_state_hash.clone(),
            active_world_hash: self.identity.active_world_hash.clone(),
            migration_receipt_hash: self.identity.migration_receipt_hash.clone(),
        }
    }

    pub(crate) fn validate_prepared_file_set(&self) -> Result<(), DraftWorld21StoreError> {
        let expected = [
            INITIALIZATION_HEAD_FILE,
            IDENTITY_FILE,
            MANIFEST_FILE,
            LIFECYCLE_GENESIS_FILE,
            SNAPSHOT_FILE,
            EVENT_JOURNAL_FILE,
            event_journal::EVENT_BOUNDARY_FILE,
            event_journal::EVENT_HEAD_FILE,
            WRITER_LOCK_FILE,
        ];
        for entry in fs::read_dir(&self.root).map_err(|source| io_error(&self.root, source))? {
            let entry = entry.map_err(|source| io_error(&self.root, source))?;
            let name = entry.file_name();
            let name = name.to_str().ok_or_else(|| {
                DraftWorld21StoreError::Invalid(
                    "world-21 Store contains a non-UTF-8 artifact".into(),
                )
            })?;
            if !entry
                .file_type()
                .map_err(|source| io_error(entry.path(), source))?
                .is_file()
                || !expected.contains(&name)
            {
                return Err(DraftWorld21StoreError::Invalid(
                    "world-21 Store contains an unexpected artifact".into(),
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn discard_uncommitted_namespace(
        base_root: impl AsRef<Path>,
    ) -> Result<(), DraftWorld21StoreError> {
        let root = base_root.as_ref().join(STORE_DIRECTORY);
        let metadata = match fs::symlink_metadata(&root) {
            Ok(metadata) => metadata,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(source) => return Err(io_error(&root, source)),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 namespace is not a real directory".into(),
            ));
        }
        let writer_lock = acquire_writer_lock(&root)?;
        fs::remove_dir_all(&root).map_err(|source| io_error(&root, source))?;
        drop(writer_lock);
        sync_directory(base_root.as_ref())
    }

    pub(crate) fn namespace_exists(
        base_root: impl AsRef<Path>,
    ) -> Result<bool, DraftWorld21StoreError> {
        let root = base_root.as_ref().join(STORE_DIRECTORY);
        match fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(true),
            Ok(_) => Err(DraftWorld21StoreError::Invalid(
                "world-21 namespace is not a real directory".into(),
            )),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(io_error(&root, source)),
        }
    }
}

fn lifecycle_world_frontier_from_state(
    state: &DraftGridTransferCellStateV2,
) -> Result<LifecycleWorldFrontierV2, DraftWorld21StoreError> {
    Ok(LifecycleWorldFrontierV2::from_world(
        state.base(),
        state.state_hash(),
        state
            .calculate_active_world_hash()
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?,
    ))
}

fn recover_immutable(
    root: &Path,
    manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
    expected_cell_key: &CellKeyV1,
    migration_anchor_hash: &str,
) -> Result<RecoveredImmutableWorld21, DraftWorld21StoreError> {
    if !valid_hash(migration_anchor_hash) {
        return Err(DraftWorld21StoreError::Invalid(
            "migration anchor hash is not canonical BLAKE3 text".into(),
        ));
    }
    let initialization_head_bytes = read_bounded(
        &root.join(INITIALIZATION_HEAD_FILE),
        MAX_INITIALIZATION_HEAD_BYTES,
    )?;
    let initialization_head =
        serde_json::from_slice::<DraftWorld21InitializationHead>(&initialization_head_bytes)
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
    if serde_json::to_vec(&initialization_head)
        .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?
        != initialization_head_bytes
    {
        return Err(DraftWorld21StoreError::Invalid(
            "world-21 Store initialization head bytes are not canonical".into(),
        ));
    }
    let manifest_bytes = read_bounded(&root.join(MANIFEST_FILE), MAX_MANIFEST_BYTES)?;
    let reopened_manifest =
        crate::manifest_v5::decode_manifest_v5(&manifest_bytes, manifest.world_seed())
            .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
    if reopened_manifest.document() != manifest.document() {
        return Err(DraftWorld21StoreError::Invalid(
            "persisted manifest 5 differs from the expected capability".into(),
        ));
    }
    let lifecycle_bytes = read_bounded(
        &root.join(LIFECYCLE_GENESIS_FILE),
        crate::protocol19_migration::MAX_TARGET_LIFECYCLE_BYTES,
    )?;
    let lifecycle = crate::protocol19_migration::decode_target_lifecycle_genesis(&lifecycle_bytes)
        .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
    if lifecycle.migration_anchor_hash() != migration_anchor_hash
        || lifecycle.manifest_hash() != reopened_manifest.manifest_hash()
        || lifecycle.cell_key() != expected_cell_key
    {
        return Err(DraftWorld21StoreError::Invalid(
            "persisted lifecycle genesis differs from the routed migration target".into(),
        ));
    }
    let snapshot_bytes = read_bounded(&root.join(SNAPSHOT_FILE), MAX_SNAPSHOT_BYTES)?;
    let snapshot = DraftGridTransferCellStateV2::decode_world_v21_canonical(
        &snapshot_bytes,
        &reopened_manifest,
    )
    .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
    let identity_bytes = read_bounded(&root.join(IDENTITY_FILE), MAX_IDENTITY_BYTES)?;
    let identity = serde_json::from_slice::<DraftWorld21StoreIdentity>(&identity_bytes)
        .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
    if serde_json::to_vec(&identity)
        .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?
        != identity_bytes
    {
        return Err(DraftWorld21StoreError::Invalid(
            "world-21 Store identity bytes are not canonical".into(),
        ));
    }
    identity.validate(&snapshot, &reopened_manifest, expected_cell_key, &lifecycle)?;
    initialization_head.validate(&identity, 0)?;
    Ok(RecoveredImmutableWorld21 {
        initialization_head,
        identity,
        lifecycle,
        manifest: reopened_manifest,
        snapshot,
    })
}

fn validate_active_file_set(root: &Path) -> Result<(), DraftWorld21StoreError> {
    let prepared = [
        INITIALIZATION_HEAD_FILE,
        IDENTITY_FILE,
        MANIFEST_FILE,
        LIFECYCLE_GENESIS_FILE,
        SNAPSHOT_FILE,
        EVENT_JOURNAL_FILE,
        event_journal::EVENT_BOUNDARY_FILE,
        event_journal::EVENT_HEAD_FILE,
        WRITER_LOCK_FILE,
    ];
    let mut lifecycle_head = false;
    let mut lifecycle_history = false;
    let mut lifecycle_temps = 0usize;
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let file_type = entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            DraftWorld21StoreError::Invalid("world-21 Store contains a non-UTF-8 artifact".into())
        })?;
        if !file_type.is_file() {
            return Err(DraftWorld21StoreError::Invalid(
                "world-21 Store contains a non-regular artifact".into(),
            ));
        }
        if prepared.contains(&name) {
            continue;
        }
        if name == LIFECYCLE_HEAD_FILE {
            lifecycle_head = true;
            continue;
        }
        if name == LIFECYCLE_HISTORY_FILE {
            lifecycle_history = true;
            continue;
        }
        if is_lifecycle_head_temp(name) {
            lifecycle_temps += 1;
            continue;
        }
        return Err(DraftWorld21StoreError::Invalid(
            "world-21 Store contains an unexpected active artifact".into(),
        ));
    }
    if lifecycle_temps > MAX_STALE_LIFECYCLE_HEAD_TEMPS
        || lifecycle_head && !lifecycle_history
        || lifecycle_history
            && !lifecycle_head
            && fs::metadata(root.join(LIFECYCLE_HISTORY_FILE))
                .map_err(|source| io_error(root.join(LIFECYCLE_HISTORY_FILE), source))?
                .len()
                != 0
    {
        return Err(DraftWorld21StoreError::Invalid(
            "world-21 Store lifecycle runtime artifact set is incomplete".into(),
        ));
    }
    Ok(())
}

fn validate_real_cell_root(path: &Path) -> Result<(), DraftWorld21StoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(DraftWorld21StoreError::Invalid(
            "world-21 Store requires an existing real per-cell root directory".into(),
        ));
    }
    Ok(())
}

fn reset_incomplete_initialization(root: &Path) -> Result<(), DraftWorld21StoreError> {
    for file in [
        IDENTITY_FILE,
        MANIFEST_FILE,
        LIFECYCLE_GENESIS_FILE,
        SNAPSHOT_FILE,
        EVENT_JOURNAL_FILE,
        event_journal::EVENT_BOUNDARY_FILE,
        event_journal::EVENT_HEAD_FILE,
    ] {
        let path = root.join(file);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(io_error(&path, source)),
        }
    }
    for entry in fs::read_dir(root).map_err(|source| io_error(root, source))? {
        let entry = entry.map_err(|source| io_error(root, source))?;
        let file_type = entry
            .file_type()
            .map_err(|source| io_error(entry.path(), source))?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let is_initialization_temp = [
            INITIALIZATION_HEAD_FILE,
            IDENTITY_FILE,
            MANIFEST_FILE,
            LIFECYCLE_GENESIS_FILE,
            SNAPSHOT_FILE,
            event_journal::EVENT_HEAD_FILE,
        ]
        .iter()
        .any(|authority| file_name.starts_with(&format!(".{authority}.tmp-")));
        if file_type.is_file() && is_initialization_temp {
            fs::remove_file(entry.path()).map_err(|source| io_error(entry.path(), source))?;
        }
    }
    sync_directory(root)
}

fn persist_initialization_head(
    root: &Path,
    bytes: &[u8],
    failpoint: &mut Option<DraftWorld21InitializationFailpoint>,
) -> Result<(), DraftWorld21StoreError> {
    let path = root.join(INITIALIZATION_HEAD_FILE);
    let temporary = root.join(format!(
        ".{INITIALIZATION_HEAD_FILE}.tmp-{}",
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| io_error(&temporary, source))?;
        file.write_all(bytes)
            .map_err(|source| io_error(&temporary, source))?;
        file.sync_all()
            .map_err(|source| io_error(&temporary, source))?;
        inject_initialization_failure(
            failpoint,
            DraftWorld21InitializationFailpoint::HeadTempSyncedBeforeRename,
        )?;
        fs::rename(&temporary, &path).map_err(|source| io_error(&path, source))?;
        inject_initialization_failure(
            failpoint,
            DraftWorld21InitializationFailpoint::HeadRenamedBeforeDirectorySync,
        )?;
        sync_directory(root)?;
        inject_initialization_failure(
            failpoint,
            DraftWorld21InitializationFailpoint::HeadDirectorySyncedBeforeMemory,
        )
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn inject_initialization_failure(
    selected: &mut Option<DraftWorld21InitializationFailpoint>,
    current: DraftWorld21InitializationFailpoint,
) -> Result<(), DraftWorld21StoreError> {
    if *selected == Some(current) {
        *selected = None;
        Err(DraftWorld21StoreError::Injected(current))
    } else {
        Ok(())
    }
}

fn acquire_writer_lock(root: &Path) -> Result<File, DraftWorld21StoreError> {
    let path = root.join(WRITER_LOCK_FILE);
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| io_error(&path, source))?;
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
            return Err(DraftWorld21StoreError::WriterConflict);
        }
        Err(source) => return Err(io_error(&path, source)),
    }
    Ok(file)
}

fn acquire_existing_writer_lock(root: &Path) -> Result<File, DraftWorld21StoreError> {
    let path = root.join(WRITER_LOCK_FILE);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| io_error(&path, source))?;
    match file.try_lock_exclusive() {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
            return Err(DraftWorld21StoreError::WriterConflict);
        }
        Err(source) => return Err(io_error(&path, source)),
    }
    Ok(file)
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, DraftWorld21StoreError> {
    let file = File::open(path).map_err(|source| io_error(path, source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if length > u64::try_from(maximum).unwrap_or(u64::MAX) {
        return Err(DraftWorld21StoreError::TooLarge(path.to_owned()));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
    file.take(u64::try_from(maximum).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(path, source))?;
    if bytes.len() > maximum {
        return Err(DraftWorld21StoreError::TooLarge(path.to_owned()));
    }
    Ok(bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DraftWorld21StoreError> {
    let parent = path.parent().ok_or_else(|| {
        DraftWorld21StoreError::Invalid("authority file has no parent directory".into())
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            DraftWorld21StoreError::Invalid("authority filename is not canonical UTF-8".into())
        })?;
    let temporary = parent.join(format!(".{file_name}.tmp-{}", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|source| io_error(&temporary, source))?;
        file.write_all(bytes)
            .map_err(|source| io_error(&temporary, source))?;
        file.sync_all()
            .map_err(|source| io_error(&temporary, source))?;
        fs::rename(&temporary, path).map_err(|source| io_error(path, source))?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_synced_empty(path: &Path) -> Result<(), DraftWorld21StoreError> {
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    file.sync_all().map_err(|source| io_error(path, source))
}

fn sync_directory(path: &Path) -> Result<(), DraftWorld21StoreError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error(path, source))
}

fn hash_json<T: Serialize>(domain: &[u8], value: &T) -> Result<String, DraftWorld21StoreError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| DraftWorld21StoreError::Invalid(source.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn io_error(path: impl AsRef<Path>, source: std::io::Error) -> DraftWorld21StoreError {
    DraftWorld21StoreError::Io {
        path: path.as_ref().to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::super::state::DraftGridTransferCellStateV2;
    use super::super::tests::package_v3_directory_fixture;
    use super::*;
    use crate::cell_directory_v3::{
        DraftCellDirectoryHistoryStoreV3, DraftDirectoryV3AuthorityHarness,
        DraftDirectoryV3AuthoritySeed,
    };
    use crate::grid_handoff_v2::event_v17::{
        DraftCanonicalGridEventV17, DraftGridEventPayloadV17, ValidatedCurrentGridEventAuthorityV17,
    };
    use crate::grid_handoff_v2::state::DraftGridDirectoryAuthorityV2;
    use crate::model::WorldState;
    use tempfile::tempdir;

    fn manifest_state() -> (
        crate::manifest_v5::ValidatedUniverseManifestV5,
        DraftGridTransferCellStateV2,
    ) {
        let manifest = crate::manifest_v5::build_validated_manifest_v5(801)
            .expect("manifest-5 capability builds");
        let (source, _, package) = package_v3_directory_fixture();
        let mut state = DraftGridTransferCellStateV2::new_with_production_origins(
            source,
            package.production_job_origins,
        )
        .expect("draft state seals");
        state
            .rebind_test_manifest_v5(&manifest)
            .expect("state binds manifest 5");
        (manifest, state)
    }

    fn migration_anchor() -> String {
        blake3::hash(b"world-21 recovery store migration anchor")
            .to_hex()
            .to_string()
    }

    fn state_cell_key(state: &DraftGridTransferCellStateV2) -> CellKeyV1 {
        crate::celestial::cell_key_from_address(&state.base().cell_address)
            .expect("state cell key derives")
    }

    fn canonical_receipt_for_state(
        state: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        cell_key: &CellKeyV1,
    ) -> Vec<u8> {
        let active_world_hash = state
            .calculate_active_world_hash()
            .expect("active world hashes");
        crate::protocol19_migration::test_canonical_receipt_bytes_for_world21_target(
            &Protocol19World21StagingEvidence {
                manifest_hash: manifest.manifest_hash(),
                universe_id: &state.base().universe_id,
                world_seed: state.base().world_seed,
                cell_key,
                cell_id: &state.base().cell_id,
                authority_fencing_token: state.base().fencing_token,
                snapshot_state_hash: state.state_hash(),
                active_world_hash: &active_world_hash,
                legacy_event_sequence: state.base().event_sequence,
                legacy_event_head_hash: &state.base().last_event_hash,
            },
        )
    }

    fn manifest_event_fixture(
        directory_root: &Path,
    ) -> (
        crate::manifest_v5::ValidatedUniverseManifestV5,
        DraftGridTransferCellStateV2,
        super::super::DraftGridClosurePackageV2,
        DraftCellDirectoryHistoryStoreV3,
    ) {
        manifest_event_fixture_with_successor_fence(directory_root, false)
    }

    fn manifest_event_fixture_with_successor_fence(
        directory_root: &Path,
        successor_fence: bool,
    ) -> (
        crate::manifest_v5::ValidatedUniverseManifestV5,
        DraftGridTransferCellStateV2,
        super::super::DraftGridClosurePackageV2,
        DraftCellDirectoryHistoryStoreV3,
    ) {
        let manifest = crate::manifest_v5::build_validated_manifest_v5(801)
            .expect("manifest-5 capability builds");
        let (source, context, legacy_package) = package_v3_directory_fixture();
        let mut manifest_source = source.clone();
        manifest_source.universe_manifest_hash = manifest.manifest_hash().to_owned();
        let package = super::super::extract_draft_grid_closure_from_validated_world(
            &manifest_source,
            &legacy_package.root_aggregate_id,
            &context,
        )
        .expect("manifest-5 package recaptures its exact source body");
        package
            .validate_manifest_v5(&manifest)
            .expect("recaptured package binds manifest 5");
        let mut state = DraftGridTransferCellStateV2::new_with_production_origins(
            source,
            package.production_job_origins.clone(),
        )
        .expect("event source state seals");
        state
            .rebind_test_manifest_v5(&manifest)
            .expect("event source binds manifest 5");
        let mut directory = DraftDirectoryV3AuthorityHarness::new(DraftDirectoryV3AuthoritySeed {
            universe_id: package.universe_id.clone(),
            universe_manifest_hash: package.universe_manifest_hash.clone(),
            transfer_id: package.transfer_id.clone(),
            root_aggregate_id: package.root_aggregate_id.clone(),
            source_cell_key: package.source_cell_key.clone(),
            destination_cell_key: package.destination_cell_key.clone(),
            source_assignment_generation: package.source_assignment_generation,
            source_fencing_token: package.source_fencing_token,
            destination_assignment_generation: package.destination_assignment_generation,
            destination_fencing_token: package.destination_fencing_token,
            package_schema_version: package.schema_version,
            receipt_schema_version: package.receipt_schema_version,
            closure_root: package.closure_root.clone(),
            conservation_root: package.conservation_root.clone(),
            package_hash: package.package_hash.clone(),
            members: package.members.clone(),
            member_root: package.member_root.clone(),
        })
        .expect("manifest-5 directory authority builds");
        directory
            .prepare()
            .expect("directory prepares event authority");
        if successor_fence {
            directory
                .advance_cell_authority(&package.source_cell_id)
                .expect("directory advances the source fence");
        }
        let history = directory
            .persist_history(directory_root)
            .expect("directory history persists");
        (manifest, state, package, history)
    }

    fn live_prepare_event(
        state: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        package: &super::super::DraftGridClosurePackageV2,
        directory: &DraftCellDirectoryHistoryStoreV3,
        event_id: &str,
    ) -> DraftCanonicalGridEventV17 {
        let capability = directory
            .current_grid_authority(&package.transfer_id)
            .expect("current grid authority resolves");
        DraftCanonicalGridEventV17::new_live_world_v21_system_for_store(
            state,
            manifest,
            event_id,
            1_800_000_040_000,
            DraftGridEventPayloadV17::GridTransferPrepared {
                package: package.clone(),
                authority: DraftGridDirectoryAuthorityV2::from_validated_v3(capability.validated()),
            },
            &ValidatedCurrentGridEventAuthorityV17::Grid(&capability),
        )
        .expect("live prepare event seals")
    }

    #[test]
    fn recovery_only_store_reopens_exact_manifest_snapshot_and_frontier() {
        let root = tempdir().expect("temporary root");
        let (manifest, state) = manifest_state();
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let cell_key = state_cell_key(&state);
        let store = DraftWorld21Store::create_for_test(
            root.path(),
            &capability,
            &cell_key,
            &migration_anchor(),
        )
        .expect("world-21 Store creates");
        assert_eq!(store.state(), &state);
        assert_eq!(
            store.identity().legacy_event_sequence,
            state.base().event_sequence
        );
        assert_eq!(
            store.identity().legacy_event_head_hash,
            state.base().last_event_hash
        );
        assert!(valid_hash(&store.initialization_head().head_hash));
        assert!(store.root().ends_with(STORE_DIRECTORY));
        assert!(matches!(
            DraftWorld21Store::open_for_test(
                root.path(),
                &manifest,
                &cell_key,
                &migration_anchor(),
            ),
            Err(DraftWorld21StoreError::WriterConflict)
        ));
        drop(store);

        let reopened = DraftWorld21Store::open_for_test(
            root.path(),
            &manifest,
            &cell_key,
            &migration_anchor(),
        )
        .expect("world-21 Store reopens");
        assert_eq!(reopened.state(), &state);
        assert_eq!(reopened.identity().manifest_hash, manifest.manifest_hash());
    }

    #[test]
    fn canonical_receipt_stages_and_reopens_its_exact_lifecycle_bound_cell() {
        let root = tempdir().expect("temporary root");
        let (manifest, state) = manifest_state();
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let cell_key = state_cell_key(&state);
        let receipt_bytes = canonical_receipt_for_state(&state, &manifest, &cell_key);
        let target =
            ValidatedWorld21StagingTarget::from_receipt(&receipt_bytes, &capability, &cell_key)
                .expect("receipt binds exact staging target");
        let store = DraftWorld21Store::stage_from_migration(root.path(), &target)
            .expect("receipt-bound Store stages");
        let migration_anchor_hash = store.identity().migration_anchor_hash.clone();
        assert_eq!(
            store.lifecycle.record_hash(),
            store.identity().lifecycle_record_hash
        );
        assert_eq!(
            fs::read(store.root().join(LIFECYCLE_GENESIS_FILE)).expect("lifecycle genesis reads"),
            store
                .lifecycle
                .encode_canonical()
                .expect("lifecycle genesis encodes")
        );
        drop(store);

        let reopened = DraftWorld21Store::open_for_test(
            root.path(),
            &manifest,
            &cell_key,
            &migration_anchor_hash,
        )
        .expect("receipt-bound Store reopens");
        assert_eq!(reopened.state(), &state);

        let wrong_key =
            crate::celestial::neighbor_cell_key(&cell_key, [1, 0, 0]).expect("wrong route derives");
        assert!(
            ValidatedWorld21StagingTarget::from_receipt(&receipt_bytes, &capability, &wrong_key,)
                .is_err()
        );
    }

    #[test]
    fn recovery_only_store_fails_closed_on_every_authority_file_tamper() {
        for target in [
            INITIALIZATION_HEAD_FILE,
            IDENTITY_FILE,
            MANIFEST_FILE,
            LIFECYCLE_GENESIS_FILE,
            SNAPSHOT_FILE,
            EVENT_JOURNAL_FILE,
            event_journal::EVENT_BOUNDARY_FILE,
            event_journal::EVENT_HEAD_FILE,
        ] {
            let root = tempdir().expect("temporary root");
            let (manifest, state) = manifest_state();
            let capability = state
                .validate_world_v21(&manifest)
                .expect("state capability mints");
            let cell_key = state_cell_key(&state);
            let store = DraftWorld21Store::create_for_test(
                root.path(),
                &capability,
                &cell_key,
                &migration_anchor(),
            )
            .expect("world-21 Store creates");
            let path = store.root().join(target);
            drop(store);
            let mut bytes = fs::read(&path).expect("authority file reads");
            if bytes.is_empty() {
                bytes.push(b'x');
            } else {
                bytes[0] ^= 1;
            }
            fs::write(&path, bytes).expect("authority file tampers");
            assert!(
                DraftWorld21Store::open_for_test(
                    root.path(),
                    &manifest,
                    &cell_key,
                    &migration_anchor(),
                )
                .is_err(),
                "tampered {target} must fail closed"
            );
        }

        let root = tempdir().expect("temporary root");
        let (manifest, state) = manifest_state();
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let cell_key = state_cell_key(&state);
        let store = DraftWorld21Store::create_for_test(
            root.path(),
            &capability,
            &cell_key,
            &migration_anchor(),
        )
        .expect("world-21 Store creates");
        drop(store);
        let wrong_manifest =
            crate::manifest_v5::build_validated_manifest_v5(802).expect("other manifest builds");
        let wrong_anchor = blake3::hash(b"wrong anchor").to_hex().to_string();
        assert!(
            DraftWorld21Store::open_for_test(
                root.path(),
                &wrong_manifest,
                &cell_key,
                &migration_anchor(),
            )
            .is_err()
        );
        assert!(
            DraftWorld21Store::open_for_test(root.path(), &manifest, &cell_key, &wrong_anchor,)
                .is_err()
        );
    }

    #[test]
    fn initialization_failpoints_recover_only_precommit_or_complete_authority() {
        for (failpoint, initialization_is_committed) in [
            (
                DraftWorld21InitializationFailpoint::NamespaceDirectorySynced,
                false,
            ),
            (DraftWorld21InitializationFailpoint::IdentitySynced, false),
            (DraftWorld21InitializationFailpoint::ManifestSynced, false),
            (
                DraftWorld21InitializationFailpoint::LifecycleGenesisSynced,
                false,
            ),
            (DraftWorld21InitializationFailpoint::SnapshotSynced, false),
            (
                DraftWorld21InitializationFailpoint::EventJournalSynced,
                false,
            ),
            (
                DraftWorld21InitializationFailpoint::EventBoundaryJournalSynced,
                false,
            ),
            (DraftWorld21InitializationFailpoint::EventHeadSynced, false),
            (
                DraftWorld21InitializationFailpoint::HeadTempSyncedBeforeRename,
                false,
            ),
            (
                DraftWorld21InitializationFailpoint::HeadRenamedBeforeDirectorySync,
                true,
            ),
            (
                DraftWorld21InitializationFailpoint::HeadDirectorySyncedBeforeMemory,
                true,
            ),
        ] {
            let root = tempdir().expect("temporary root");
            let (manifest, state) = manifest_state();
            let capability = state
                .validate_world_v21(&manifest)
                .expect("state capability mints");
            let cell_key = state_cell_key(&state);
            let error = DraftWorld21Store::create_for_test_with_failpoint(
                root.path(),
                &capability,
                &cell_key,
                &migration_anchor(),
                Some(failpoint),
            )
            .expect_err("initialization failure injects");
            assert!(
                matches!(error, DraftWorld21StoreError::Injected(actual) if actual == failpoint)
            );

            let reopen = DraftWorld21Store::open_for_test(
                root.path(),
                &manifest,
                &cell_key,
                &migration_anchor(),
            );
            if initialization_is_committed {
                reopen.expect("committed initialization reopens");
            } else {
                assert!(reopen.is_err(), "precommit initialization cannot reopen");
                DraftWorld21Store::create_for_test(
                    root.path(),
                    &capability,
                    &cell_key,
                    &migration_anchor(),
                )
                .expect("precommit debris is replaced exactly");
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn uncertain_initialization_head_metadata_never_authorizes_cleanup() {
        use std::os::unix::fs::symlink;

        let root = tempdir().expect("temporary root");
        let namespace = root.path().join(STORE_DIRECTORY);
        fs::create_dir(&namespace).expect("namespace creates");
        let sentinel = b"preexisting authority must survive";
        fs::write(namespace.join(IDENTITY_FILE), sentinel).expect("sentinel writes");
        symlink(
            INITIALIZATION_HEAD_FILE,
            namespace.join(INITIALIZATION_HEAD_FILE),
        )
        .expect("self-referential head symlink creates");
        let (manifest, state) = manifest_state();
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let cell_key = state_cell_key(&state);

        assert!(matches!(
            DraftWorld21Store::create_for_test(
                root.path(),
                &capability,
                &cell_key,
                &migration_anchor(),
            ),
            Err(DraftWorld21StoreError::Io { .. })
        ));
        assert_eq!(
            fs::read(namespace.join(IDENTITY_FILE)).expect("sentinel remains"),
            sentinel
        );
    }

    #[test]
    fn initialization_requires_an_existing_per_cell_root() {
        let parent = tempdir().expect("temporary parent");
        let missing_root = parent.path().join("missing-cell-root");
        let (manifest, state) = manifest_state();
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let cell_key = state_cell_key(&state);

        assert!(matches!(
            DraftWorld21Store::create_for_test(
                &missing_root,
                &capability,
                &cell_key,
                &migration_anchor(),
            ),
            Err(DraftWorld21StoreError::Io { .. })
        ));
        assert!(!missing_root.exists());
    }

    #[test]
    fn recovery_rejects_two_complete_cell_namespaces_when_swapped() {
        let source_root = tempdir().expect("source root");
        let destination_root = tempdir().expect("destination root");
        let (manifest, source_state) = manifest_state();
        let source_cell_key = state_cell_key(&source_state);
        let (_, context, _) = package_v3_directory_fixture();
        let destination_cell_key = context.placement.destination_cell_key;
        let mut destination_base = WorldState::genesis_for_cell(801, &destination_cell_key)
            .expect("destination genesis builds");
        destination_base.fencing_token = 1;
        let mut destination_state = DraftGridTransferCellStateV2::new_with_production_origins(
            destination_base,
            std::collections::BTreeMap::default(),
        )
        .expect("destination state seals");
        destination_state
            .rebind_test_manifest_v5(&manifest)
            .expect("destination state binds manifest 5");
        let source_capability = source_state
            .validate_world_v21(&manifest)
            .expect("source capability mints");
        let destination_capability = destination_state
            .validate_world_v21(&manifest)
            .expect("destination capability mints");
        let source_store = DraftWorld21Store::create_for_test(
            source_root.path(),
            &source_capability,
            &source_cell_key,
            &migration_anchor(),
        )
        .expect("source Store creates");
        let destination_store = DraftWorld21Store::create_for_test(
            destination_root.path(),
            &destination_capability,
            &destination_cell_key,
            &migration_anchor(),
        )
        .expect("destination Store creates");
        drop(source_store);
        drop(destination_store);

        let source_namespace = source_root.path().join(STORE_DIRECTORY);
        let destination_namespace = destination_root.path().join(STORE_DIRECTORY);
        let held_namespace = source_root.path().join("held-world-v21");
        fs::rename(&source_namespace, &held_namespace).expect("source namespace moves aside");
        fs::rename(&destination_namespace, &source_namespace)
            .expect("destination namespace moves to source route");
        fs::rename(&held_namespace, &destination_namespace)
            .expect("source namespace moves to destination route");

        assert!(
            DraftWorld21Store::open_for_test(
                source_root.path(),
                &manifest,
                &source_cell_key,
                &migration_anchor(),
            )
            .is_err()
        );
        assert!(
            DraftWorld21Store::open_for_test(
                destination_root.path(),
                &manifest,
                &destination_cell_key,
                &migration_anchor(),
            )
            .is_err()
        );
    }

    #[test]
    fn event17_append_replays_exact_state_and_requires_directory_history() {
        let root = tempdir().expect("temporary Store root");
        let directory_root = tempdir().expect("temporary directory root");
        let (manifest, state, package, directory) = manifest_event_fixture(directory_root.path());
        let cell_key = state_cell_key(&state);
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let mut store = DraftWorld21Store::create_for_test(
            root.path(),
            &capability,
            &cell_key,
            &migration_anchor(),
        )
        .expect("world-21 Store creates");
        let genesis_active_world_hash = store.identity().active_world_hash.clone();
        let event = live_prepare_event(
            store.state(),
            &manifest,
            &package,
            &directory,
            "store-prepare-1",
        );
        assert_eq!(event.event_sequence(), state.base().event_sequence + 1);
        assert_eq!(event.previous_event_hash(), state.base().last_event_hash);
        let current = directory
            .current_grid_authority(&package.transfer_id)
            .expect("current authority remains locked");
        let proof = store
            .append_live_event_for_test(
                &event,
                &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
            )
            .expect("event append commits");
        assert!(valid_hash(event_journal::proof_hash(&proof)));
        assert_eq!(store.committed_event17_count(), 1);
        let expected_state = store.state().clone();
        let expected_active_world_hash = expected_state
            .calculate_active_world_hash()
            .expect("event successor active-world hash derives");
        assert_ne!(expected_active_world_hash, genesis_active_world_hash);
        drop(current);
        drop(store);

        assert!(
            DraftWorld21Store::open_for_test(
                root.path(),
                &manifest,
                &cell_key,
                &migration_anchor(),
            )
            .is_err(),
            "the recovery-only constructor must not silently trust a nonempty journal"
        );
        let reopened = DraftWorld21Store::open_with_event_replay_for_test(
            root.path(),
            &manifest,
            &cell_key,
            &migration_anchor(),
            &directory,
        )
        .expect("manifest-aware event replay reopens");
        assert_eq!(reopened.state(), &expected_state);
        assert_eq!(reopened.committed_event17_count(), 1);
        assert_eq!(
            lifecycle_world_frontier_from_state(reopened.state())
                .expect("replayed lifecycle frontier derives")
                .active_world_hash,
            expected_active_world_hash,
            "replayed lifecycle preflight must not reuse the immutable genesis world hash"
        );
    }

    #[test]
    fn event17_append_rebinds_and_replays_a_valid_successor_fence() {
        let root = tempdir().expect("temporary Store root");
        let directory_root = tempdir().expect("temporary directory root");
        let (manifest, state, package, directory) =
            manifest_event_fixture_with_successor_fence(directory_root.path(), true);
        let cell_key = state_cell_key(&state);
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let mut store = DraftWorld21Store::create_for_test(
            root.path(),
            &capability,
            &cell_key,
            &migration_anchor(),
        )
        .expect("world-21 Store creates");
        let event = live_prepare_event(
            store.state(),
            &manifest,
            &package,
            &directory,
            "store-successor-fence",
        );
        let current = directory
            .current_grid_authority(&package.transfer_id)
            .expect("successor authority remains current");
        assert!(current.validated().live_source_fencing_token() > state.base().fencing_token);
        store
            .append_live_event_for_test(
                &event,
                &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
            )
            .expect("successor-fence append commits");
        let expected = store.state().clone();
        assert_eq!(
            expected.base().fencing_token,
            current.validated().live_source_fencing_token()
        );
        drop(current);
        drop(store);
        let reopened = DraftWorld21Store::open_with_event_replay_for_test(
            root.path(),
            &manifest,
            &cell_key,
            &migration_anchor(),
            &directory,
        )
        .expect("successor-fence history replays");
        assert_eq!(reopened.state(), &expected);
    }

    #[test]
    fn event17_append_failpoints_recover_only_prior_or_exact_successor() {
        use event_journal::DraftWorld21AppendFailpoint as Failpoint;

        for (failpoint, successor_is_visible) in [
            (Failpoint::PendingHeadTempSyncedBeforeRename, false),
            (Failpoint::PendingHeadRenamedBeforeDirectorySync, false),
            (Failpoint::PendingHeadDirectorySyncedBeforeEvent, false),
            (Failpoint::PendingHeadSynced, false),
            (Failpoint::PartialEventWritten, false),
            (Failpoint::EventLineWrittenBeforeSync, true),
            (Failpoint::EventSyncedBeforeCommitHead, true),
            (Failpoint::PartialBoundaryWritten, true),
            (Failpoint::BoundaryLineWrittenBeforeSync, true),
            (Failpoint::BoundarySyncedBeforeCommitHead, true),
            (Failpoint::CommitHeadTempSyncedBeforeRename, true),
            (Failpoint::CommitHeadRenamedBeforeDirectorySync, true),
            (Failpoint::CommitHeadDirectorySyncedBeforeMemory, true),
        ] {
            let root = tempdir().expect("temporary Store root");
            let directory_root = tempdir().expect("temporary directory root");
            let (manifest, state, package, directory) =
                manifest_event_fixture(directory_root.path());
            let cell_key = state_cell_key(&state);
            let capability = state
                .validate_world_v21(&manifest)
                .expect("state capability mints");
            let mut store = DraftWorld21Store::create_for_test(
                root.path(),
                &capability,
                &cell_key,
                &migration_anchor(),
            )
            .expect("world-21 Store creates");
            let event = live_prepare_event(
                store.state(),
                &manifest,
                &package,
                &directory,
                &format!("store-failpoint-{failpoint:?}"),
            );
            let current = directory
                .current_grid_authority(&package.transfer_id)
                .expect("current authority remains locked");
            store.set_append_failpoint_for_test(failpoint);
            assert!(matches!(
                store.append_live_event_for_test(
                    &event,
                    &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
                ),
                Err(DraftWorld21StoreError::InjectedAppend(_))
            ));
            assert!(
                store
                    .append_live_event_for_test(
                        &event,
                        &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
                    )
                    .is_err(),
                "an uncertain {failpoint:?} outcome must poison the old writer"
            );
            drop(current);
            drop(store);

            let reopened = DraftWorld21Store::open_with_event_replay_for_test(
                root.path(),
                &manifest,
                &cell_key,
                &migration_anchor(),
                &directory,
            )
            .unwrap_or_else(|error| panic!("{failpoint:?} must recover: {error}"));
            let expected_count = u64::from(successor_is_visible);
            assert_eq!(reopened.committed_event17_count(), expected_count);
            assert_eq!(
                reopened.state().base().event_sequence,
                state.base().event_sequence + expected_count,
                "{failpoint:?} recovered the wrong frontier"
            );
            if successor_is_visible {
                assert_eq!(reopened.state().base().last_event_hash, event.event_hash());
            } else {
                assert_eq!(reopened.state(), &state);
            }
        }
    }

    #[test]
    fn event17_precommit_failpoint_keeps_the_writer_retryable() {
        let root = tempdir().expect("temporary Store root");
        let directory_root = tempdir().expect("temporary directory root");
        let (manifest, state, package, directory) = manifest_event_fixture(directory_root.path());
        let cell_key = state_cell_key(&state);
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let mut store = DraftWorld21Store::create_for_test(
            root.path(),
            &capability,
            &cell_key,
            &migration_anchor(),
        )
        .expect("world-21 Store creates");
        let event = live_prepare_event(
            store.state(),
            &manifest,
            &package,
            &directory,
            "store-precommit",
        );
        let current = directory
            .current_grid_authority(&package.transfer_id)
            .expect("current authority remains locked");
        store.set_append_failpoint_for_test(
            event_journal::DraftWorld21AppendFailpoint::BeforePendingHeadWrite,
        );
        assert!(matches!(
            store.append_live_event_for_test(
                &event,
                &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
            ),
            Err(DraftWorld21StoreError::InjectedAppend(_))
        ));
        assert_eq!(store.committed_event17_count(), 0);
        store
            .append_live_event_for_test(
                &event,
                &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
            )
            .expect("a failure before any durable write remains retryable");
        assert_eq!(store.committed_event17_count(), 1);
    }

    #[test]
    fn event17_recovery_never_truncates_beyond_a_sealed_pending_range() {
        use event_journal::DraftWorld21AppendFailpoint as Failpoint;

        for (failpoint, target, pending_end_field) in [
            (
                Failpoint::PartialEventWritten,
                EVENT_JOURNAL_FILE,
                "journal_end",
            ),
            (
                Failpoint::PartialBoundaryWritten,
                event_journal::EVENT_BOUNDARY_FILE,
                "boundary_end",
            ),
        ] {
            let root = tempdir().expect("temporary Store root");
            let directory_root = tempdir().expect("temporary directory root");
            let (manifest, state, package, directory) =
                manifest_event_fixture(directory_root.path());
            let cell_key = state_cell_key(&state);
            let capability = state
                .validate_world_v21(&manifest)
                .expect("state capability mints");
            let mut store = DraftWorld21Store::create_for_test(
                root.path(),
                &capability,
                &cell_key,
                &migration_anchor(),
            )
            .expect("world-21 Store creates");
            let event = live_prepare_event(
                store.state(),
                &manifest,
                &package,
                &directory,
                &format!("store-overlong-{failpoint:?}"),
            );
            let current = directory
                .current_grid_authority(&package.transfer_id)
                .expect("current authority remains locked");
            store.set_append_failpoint_for_test(failpoint);
            assert!(matches!(
                store.append_live_event_for_test(
                    &event,
                    &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
                ),
                Err(DraftWorld21StoreError::InjectedAppend(_))
            ));
            let namespace = store.root().to_owned();
            drop(current);
            drop(store);

            let head = serde_json::from_slice::<serde_json::Value>(
                &fs::read(namespace.join(event_journal::EVENT_HEAD_FILE))
                    .expect("pending head reads"),
            )
            .expect("pending head decodes");
            let sealed_end = head["pending_event"][pending_end_field]
                .as_u64()
                .expect("pending range end exists");
            let path = namespace.join(target);
            let partial_length = fs::metadata(&path).expect("partial suffix exists").len();
            assert!(partial_length < sealed_end);
            let extension = usize::try_from(sealed_end - partial_length)
                .expect("sealed test range fits memory");
            OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("partial authority opens")
                .write_all(&vec![b'x'; extension])
                .expect("overlong authority reaches the sealed end without a newline");
            let overlong_length = fs::metadata(&path).expect("overlong suffix exists").len();
            assert_eq!(overlong_length, sealed_end);

            assert!(
                DraftWorld21Store::open_with_event_replay_for_test(
                    root.path(),
                    &manifest,
                    &cell_key,
                    &migration_anchor(),
                    &directory,
                )
                .is_err(),
                "{failpoint:?} data outside the possible crash range must fail closed"
            );
            assert_eq!(
                fs::metadata(&path)
                    .expect("failed recovery preserves evidence")
                    .len(),
                overlong_length,
                "failed recovery must not truncate suspicious evidence"
            );
        }
    }

    #[test]
    fn resealed_event17_head_cannot_substitute_its_manifest_identity() {
        let root = tempdir().expect("temporary Store root");
        let (manifest, state) = manifest_state();
        let cell_key = state_cell_key(&state);
        let capability = state
            .validate_world_v21(&manifest)
            .expect("state capability mints");
        let store = DraftWorld21Store::create_for_test(
            root.path(),
            &capability,
            &cell_key,
            &migration_anchor(),
        )
        .expect("world-21 Store creates");
        let namespace = store.root().to_owned();
        drop(store);
        event_journal::substitute_head_manifest_for_test(&namespace, "ab".repeat(32))
            .expect("test substitution reseals the head");
        assert!(
            DraftWorld21Store::open_for_test(
                root.path(),
                &manifest,
                &cell_key,
                &migration_anchor(),
            )
            .is_err(),
            "a syntactically valid but substituted head manifest must fail closed"
        );
    }

    #[test]
    fn committed_event17_authority_and_unpinned_suffix_tamper_fail_closed() {
        for target in [
            EVENT_JOURNAL_FILE,
            event_journal::EVENT_BOUNDARY_FILE,
            event_journal::EVENT_HEAD_FILE,
        ] {
            let root = tempdir().expect("temporary Store root");
            let directory_root = tempdir().expect("temporary directory root");
            let (manifest, state, package, directory) =
                manifest_event_fixture(directory_root.path());
            let cell_key = state_cell_key(&state);
            let capability = state
                .validate_world_v21(&manifest)
                .expect("state capability mints");
            let mut store = DraftWorld21Store::create_for_test(
                root.path(),
                &capability,
                &cell_key,
                &migration_anchor(),
            )
            .expect("world-21 Store creates");
            let event = live_prepare_event(
                store.state(),
                &manifest,
                &package,
                &directory,
                "store-tamper",
            );
            let current = directory
                .current_grid_authority(&package.transfer_id)
                .expect("current authority remains locked");
            store
                .append_live_event_for_test(
                    &event,
                    &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
                )
                .expect("event append commits");
            let path = store.root().join(target);
            drop(current);
            drop(store);
            let mut bytes = fs::read(&path).expect("committed authority reads");
            bytes[0] ^= 1;
            fs::write(&path, bytes).expect("committed authority tampers");
            assert!(
                DraftWorld21Store::open_with_event_replay_for_test(
                    root.path(),
                    &manifest,
                    &cell_key,
                    &migration_anchor(),
                    &directory,
                )
                .is_err(),
                "tampered committed {target} must fail closed"
            );
        }

        for target in [EVENT_JOURNAL_FILE, event_journal::EVENT_BOUNDARY_FILE] {
            let root = tempdir().expect("temporary Store root");
            let directory_root = tempdir().expect("temporary directory root");
            let (manifest, state, package, directory) =
                manifest_event_fixture(directory_root.path());
            let cell_key = state_cell_key(&state);
            let capability = state
                .validate_world_v21(&manifest)
                .expect("state capability mints");
            let mut store = DraftWorld21Store::create_for_test(
                root.path(),
                &capability,
                &cell_key,
                &migration_anchor(),
            )
            .expect("world-21 Store creates");
            let event = live_prepare_event(
                store.state(),
                &manifest,
                &package,
                &directory,
                "store-suffix",
            );
            let current = directory
                .current_grid_authority(&package.transfer_id)
                .expect("current authority remains locked");
            store
                .append_live_event_for_test(
                    &event,
                    &ValidatedCurrentGridEventAuthorityV17::Grid(&current),
                )
                .expect("event append commits");
            let path = store.root().join(target);
            drop(current);
            drop(store);
            OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("authority opens for suffix")
                .write_all(b"\n")
                .expect("unpinned suffix writes");
            assert!(
                DraftWorld21Store::open_with_event_replay_for_test(
                    root.path(),
                    &manifest,
                    &cell_key,
                    &migration_anchor(),
                    &directory,
                )
                .is_err(),
                "unpinned {target} suffix must fail closed"
            );
        }
    }
}
