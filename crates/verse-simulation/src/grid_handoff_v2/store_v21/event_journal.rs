// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant event-17 journal, lifecycle head, and replay transaction.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use verse_protocol::protocol_v19::Protocol19CompatibilityTuple;

use super::{
    DraftWorld21InitializationHead, DraftWorld21StoreError, DraftWorld21StoreIdentity,
    atomic_write, hash_json, io_error, read_bounded, sync_directory, valid_hash,
};
use crate::cell_directory_v3::DraftCellDirectoryHistoryStoreV3;
use crate::grid_handoff_v2::dispatcher_v17::{
    DraftGridEventApplicationV17, DraftGridEventProofV17, apply_manifest_bound_event_v17,
};
use crate::grid_handoff_v2::event_v17::{
    DraftCanonicalGridEventV17, DraftGridEventAuthorityLookupV17, DraftGridEventPayloadV17,
    MAX_DRAFT_GRID_EVENT_BYTES, ValidatedCurrentGridEventAuthorityV17,
    ValidatedDraftGridEventAuthorityV17, ValidatedManifestBoundGridEventAuthorityV17,
};
use crate::grid_handoff_v2::state::DraftGridTransferCellStateV2;

pub(super) const EVENT_HEAD_FILE: &str = "events-v17.head.json";
pub(super) const EVENT_BOUNDARY_FILE: &str = "event-boundaries-v17.ndjson";
const EVENT_HEAD_SCHEMA_VERSION: u32 = 1;
const EVENT_BOUNDARY_SCHEMA_VERSION: u32 = 1;
const EVENT_HEAD_HASH_DOMAIN: &[u8] = b"the-verse/world-21-event-head/v1\0";
const EVENT_RECORD_HASH_DOMAIN: &[u8] = b"the-verse/world-21-event-record/v1\0";
const EVENT_BOUNDARY_HASH_DOMAIN: &[u8] = b"the-verse/world-21-event-boundary/v1\0";
const MAX_EVENT_HEAD_BYTES: usize = 64 * 1_024;
const MAX_EVENT_RECORD_BYTES: usize = MAX_DRAFT_GRID_EVENT_BYTES + 1;
const MAX_EVENT_JOURNAL_BYTES: u64 = 4 * 1_024 * 1_024 * 1_024;
const MAX_EVENT_BOUNDARY_RECORD_BYTES: usize = 64 * 1_024;
const MAX_EVENT_BOUNDARY_JOURNAL_BYTES: u64 = 256 * 1_024 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DraftWorld21EventKindV17 {
    GridTransferPrepared,
    GridTransferQuarantined,
    GridTransferExported,
    GridTransferImported,
    GridTransferActivated,
    GridTransferFinalized,
    GridTransferAborted,
    ProductionQuantumCommitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWorld21EventBoundaryV17 {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    event_kind: DraftWorld21EventKindV17,
    event_sequence: u64,
    event_hash: String,
    event_payload_hash: String,
    directory_revision: u64,
    directory_document_hash: String,
    cell_id: String,
    transfer_id: Option<String>,
    package_hash: Option<String>,
    proof_hash: String,
    resulting_state_hash: String,
    previous_boundary_hash: String,
    boundary_hash: String,
}

impl DraftWorld21EventBoundaryV17 {
    fn new(
        event: &DraftCanonicalGridEventV17,
        application: &DraftGridEventApplicationV17,
        previous_boundary_hash: String,
    ) -> Result<Self, DraftWorld21StoreError> {
        let (event_kind, transfer_id, package_hash) = match event.payload() {
            DraftGridEventPayloadV17::GridTransferPrepared { package, .. } => (
                DraftWorld21EventKindV17::GridTransferPrepared,
                Some(package.transfer_id.clone()),
                Some(package.package_hash.clone()),
            ),
            DraftGridEventPayloadV17::GridTransferQuarantined { package, .. } => (
                DraftWorld21EventKindV17::GridTransferQuarantined,
                Some(package.transfer_id.clone()),
                Some(package.package_hash.clone()),
            ),
            DraftGridEventPayloadV17::GridTransferExported { package, .. } => (
                DraftWorld21EventKindV17::GridTransferExported,
                Some(package.transfer_id.clone()),
                Some(package.package_hash.clone()),
            ),
            DraftGridEventPayloadV17::GridTransferImported { package, .. } => (
                DraftWorld21EventKindV17::GridTransferImported,
                Some(package.transfer_id.clone()),
                Some(package.package_hash.clone()),
            ),
            DraftGridEventPayloadV17::GridTransferActivated { package, .. } => (
                DraftWorld21EventKindV17::GridTransferActivated,
                Some(package.transfer_id.clone()),
                Some(package.package_hash.clone()),
            ),
            DraftGridEventPayloadV17::GridTransferFinalized { package, .. } => (
                DraftWorld21EventKindV17::GridTransferFinalized,
                Some(package.transfer_id.clone()),
                Some(package.package_hash.clone()),
            ),
            DraftGridEventPayloadV17::GridTransferAborted { package, .. } => (
                DraftWorld21EventKindV17::GridTransferAborted,
                Some(package.transfer_id.clone()),
                Some(package.package_hash.clone()),
            ),
            DraftGridEventPayloadV17::ProductionQuantumCommitted { .. } => (
                DraftWorld21EventKindV17::ProductionQuantumCommitted,
                None,
                None,
            ),
        };
        let (directory_revision, directory_document_hash) = match event.authority_lookup() {
            DraftGridEventAuthorityLookupV17::Grid {
                directory_revision,
                directory_document_hash,
                ..
            }
            | DraftGridEventAuthorityLookupV17::Production {
                directory_revision,
                directory_document_hash,
                ..
            } => (directory_revision, directory_document_hash.to_owned()),
        };
        let mut boundary = Self {
            schema_version: EVENT_BOUNDARY_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            event_kind,
            event_sequence: event.event_sequence(),
            event_hash: event.event_hash().to_owned(),
            event_payload_hash: event.event_payload_hash().to_owned(),
            directory_revision,
            directory_document_hash,
            cell_id: event.cell_id().to_owned(),
            transfer_id,
            package_hash,
            proof_hash: proof_hash(&application.proof).to_owned(),
            resulting_state_hash: application.next_state.state_hash().to_owned(),
            previous_boundary_hash,
            boundary_hash: String::new(),
        };
        boundary.boundary_hash = boundary.calculate_hash()?;
        boundary.validate()?;
        Ok(boundary)
    }

    fn calculate_hash(&self) -> Result<String, DraftWorld21StoreError> {
        let mut material = self.clone();
        material.boundary_hash.clear();
        hash_json(EVENT_BOUNDARY_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), DraftWorld21StoreError> {
        let grid_event = !matches!(
            self.event_kind,
            DraftWorld21EventKindV17::ProductionQuantumCommitted
        );
        if self.schema_version != EVENT_BOUNDARY_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.event_sequence == 0
            || !valid_hash(&self.event_hash)
            || !valid_hash(&self.event_payload_hash)
            || self.directory_revision == 0
            || !valid_hash(&self.directory_document_hash)
            || !valid_hash(&self.cell_id)
            || grid_event != self.transfer_id.is_some()
            || grid_event != self.package_hash.is_some()
            || self.transfer_id.as_deref().is_some_and(str::is_empty)
            || self
                .package_hash
                .as_deref()
                .is_some_and(|hash| !valid_hash(hash))
            || !valid_hash(&self.proof_hash)
            || !valid_hash(&self.resulting_state_hash)
            || (!self.previous_boundary_hash.is_empty()
                && !valid_hash(&self.previous_boundary_hash))
            || !valid_hash(&self.boundary_hash)
            || self.boundary_hash != self.calculate_hash()?
        {
            return Err(invalid("event-17 boundary is invalid"));
        }
        Ok(())
    }

    fn encode_record(&self) -> Result<Vec<u8>, DraftWorld21StoreError> {
        let mut bytes = serde_json::to_vec(self).map_err(|source| invalid(source.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_EVENT_BOUNDARY_RECORD_BYTES {
            return Err(invalid("event-17 boundary exceeds its byte bound"));
        }
        Ok(bytes)
    }

    fn decode_record(bytes: &[u8]) -> Result<Self, DraftWorld21StoreError> {
        let boundary =
            serde_json::from_slice::<Self>(bytes).map_err(|source| invalid(source.to_string()))?;
        let canonical =
            serde_json::to_vec(&boundary).map_err(|source| invalid(source.to_string()))?;
        if canonical != bytes {
            return Err(invalid("event-17 boundary bytes are not canonical"));
        }
        boundary.validate()?;
        Ok(boundary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWorld21PendingEventV17 {
    event_sequence: u64,
    previous_event_hash: String,
    event_hash: String,
    event_payload_hash: String,
    resulting_state_hash: String,
    journal_start: u64,
    journal_end: u64,
    record_hash: String,
    proof_hash: String,
    boundary_start: u64,
    boundary_end: u64,
    boundary_hash: String,
}

#[derive(Debug, Clone, Copy)]
struct DraftWorld21PendingRangesV17 {
    journal_start: u64,
    journal_end: u64,
    boundary_start: u64,
    boundary_end: u64,
}

impl DraftWorld21PendingEventV17 {
    fn new(
        event: &DraftCanonicalGridEventV17,
        resulting_state: &DraftGridTransferCellStateV2,
        journal_start: u64,
        record: &[u8],
        boundary: &DraftWorld21EventBoundaryV17,
        boundary_start: u64,
        boundary_record: &[u8],
    ) -> Result<Self, DraftWorld21StoreError> {
        let record_length = u64::try_from(record.len()).map_err(|_| {
            DraftWorld21StoreError::Invalid("event-17 record length does not fit u64".into())
        })?;
        let journal_end = journal_start.checked_add(record_length).ok_or_else(|| {
            DraftWorld21StoreError::Invalid("event-17 journal length overflowed".into())
        })?;
        let boundary_record_length = u64::try_from(boundary_record.len()).map_err(|_| {
            DraftWorld21StoreError::Invalid("event-17 boundary length does not fit u64".into())
        })?;
        let boundary_end = boundary_start
            .checked_add(boundary_record_length)
            .ok_or_else(|| invalid("event-17 boundary journal length overflowed"))?;
        Ok(Self {
            event_sequence: event.event_sequence(),
            previous_event_hash: event.previous_event_hash().to_owned(),
            event_hash: event.event_hash().to_owned(),
            event_payload_hash: event.event_payload_hash().to_owned(),
            resulting_state_hash: resulting_state.state_hash().to_owned(),
            journal_start,
            journal_end,
            record_hash: event_record_hash(record),
            proof_hash: boundary.proof_hash.clone(),
            boundary_start,
            boundary_end,
            boundary_hash: boundary.boundary_hash.clone(),
        })
    }

    fn validate_against(
        &self,
        head: &DraftWorld21EventHeadV17,
    ) -> Result<(), DraftWorld21StoreError> {
        let expected_sequence = head
            .committed_event_sequence
            .checked_add(1)
            .ok_or_else(|| {
                DraftWorld21StoreError::Invalid("event-17 sequence is exhausted".into())
            })?;
        let record_length = self
            .journal_end
            .checked_sub(self.journal_start)
            .ok_or_else(|| {
                DraftWorld21StoreError::Invalid("pending event-17 journal range is reversed".into())
            })?;
        let boundary_length = self
            .boundary_end
            .checked_sub(self.boundary_start)
            .ok_or_else(|| invalid("pending event-17 boundary range is reversed"))?;
        if self.event_sequence != expected_sequence
            || self.previous_event_hash != head.committed_event_hash
            || !valid_frontier(self.event_sequence, &self.event_hash)
            || !valid_hash(&self.event_payload_hash)
            || !valid_hash(&self.resulting_state_hash)
            || self.journal_start != head.journal_byte_length
            || record_length == 0
            || record_length > u64::try_from(MAX_EVENT_RECORD_BYTES).unwrap_or(u64::MAX)
            || self.journal_end > MAX_EVENT_JOURNAL_BYTES
            || !valid_hash(&self.record_hash)
            || !valid_hash(&self.proof_hash)
            || self.boundary_start != head.boundary_byte_length
            || boundary_length == 0
            || boundary_length > u64::try_from(MAX_EVENT_BOUNDARY_RECORD_BYTES).unwrap_or(u64::MAX)
            || self.boundary_end > MAX_EVENT_BOUNDARY_JOURNAL_BYTES
            || !valid_hash(&self.boundary_hash)
        {
            return Err(invalid(
                "pending event-17 does not follow the committed journal frontier",
            ));
        }
        Ok(())
    }

    fn matches(
        &self,
        event: &DraftCanonicalGridEventV17,
        resulting_state: &DraftGridTransferCellStateV2,
        record: &[u8],
        boundary: &DraftWorld21EventBoundaryV17,
        ranges: DraftWorld21PendingRangesV17,
    ) -> bool {
        self.event_sequence == event.event_sequence()
            && self.previous_event_hash == event.previous_event_hash()
            && self.event_hash == event.event_hash()
            && self.event_payload_hash == event.event_payload_hash()
            && self.resulting_state_hash == resulting_state.state_hash()
            && self.journal_start == ranges.journal_start
            && self.journal_end == ranges.journal_end
            && self.record_hash == event_record_hash(record)
            && self.proof_hash == boundary.proof_hash
            && self.boundary_start == ranges.boundary_start
            && self.boundary_end == ranges.boundary_end
            && self.boundary_hash == boundary.boundary_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftWorld21EventHeadV17 {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    identity_hash: String,
    initialization_head_hash: String,
    manifest_hash: String,
    cell_id: String,
    legacy_event_schema_version: u32,
    legacy_event_sequence: u64,
    legacy_event_head_hash: String,
    committed_event_count: u64,
    committed_event_sequence: u64,
    committed_event_hash: String,
    committed_state_hash: String,
    journal_byte_length: u64,
    boundary_count: u64,
    boundary_head_hash: String,
    boundary_byte_length: u64,
    pending_event: Option<DraftWorld21PendingEventV17>,
    head_hash: String,
}

impl DraftWorld21EventHeadV17 {
    fn new(
        identity: &DraftWorld21StoreIdentity,
        initialization: &DraftWorld21InitializationHead,
    ) -> Result<Self, DraftWorld21StoreError> {
        let mut head = Self {
            schema_version: EVENT_HEAD_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            identity_hash: identity.identity_hash.clone(),
            initialization_head_hash: initialization.head_hash.clone(),
            manifest_hash: identity.manifest_hash.clone(),
            cell_id: identity.cell_id.clone(),
            legacy_event_schema_version: identity.legacy_event_schema_version,
            legacy_event_sequence: identity.legacy_event_sequence,
            legacy_event_head_hash: identity.legacy_event_head_hash.clone(),
            committed_event_count: 0,
            committed_event_sequence: identity.legacy_event_sequence,
            committed_event_hash: identity.legacy_event_head_hash.clone(),
            committed_state_hash: identity.snapshot_state_hash.clone(),
            journal_byte_length: 0,
            boundary_count: 0,
            boundary_head_hash: String::new(),
            boundary_byte_length: 0,
            pending_event: None,
            head_hash: String::new(),
        };
        head.reseal()?;
        head.validate(identity, initialization)?;
        Ok(head)
    }

    fn calculate_hash(&self) -> Result<String, DraftWorld21StoreError> {
        let mut material = self.clone();
        material.head_hash.clear();
        hash_json(EVENT_HEAD_HASH_DOMAIN, &material)
    }

    fn reseal(&mut self) -> Result<(), DraftWorld21StoreError> {
        self.head_hash = self.calculate_hash()?;
        Ok(())
    }

    fn validate(
        &self,
        identity: &DraftWorld21StoreIdentity,
        initialization: &DraftWorld21InitializationHead,
    ) -> Result<(), DraftWorld21StoreError> {
        let expected_sequence = self
            .legacy_event_sequence
            .checked_add(self.committed_event_count)
            .ok_or_else(|| invalid("event-17 committed sequence overflowed"))?;
        let empty = self.committed_event_count == 0
            && self.committed_event_sequence == self.legacy_event_sequence
            && self.committed_event_hash == self.legacy_event_head_hash
            && self.committed_state_hash == identity.snapshot_state_hash
            && self.journal_byte_length == 0;
        let populated = self.committed_event_count > 0
            && self.committed_event_sequence > self.legacy_event_sequence
            && valid_frontier(self.committed_event_sequence, &self.committed_event_hash)
            && valid_hash(&self.committed_state_hash)
            && self.journal_byte_length > 0;
        if self.schema_version != EVENT_HEAD_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.identity_hash != identity.identity_hash
            || self.initialization_head_hash != initialization.head_hash
            || self.manifest_hash != identity.manifest_hash
            || self.cell_id != identity.cell_id
            || self.legacy_event_schema_version != identity.legacy_event_schema_version
            || self.legacy_event_sequence != identity.legacy_event_sequence
            || self.legacy_event_head_hash != identity.legacy_event_head_hash
            || self.committed_event_sequence != expected_sequence
            || self.journal_byte_length > MAX_EVENT_JOURNAL_BYTES
            || self.boundary_count != self.committed_event_count
            || self.boundary_byte_length > MAX_EVENT_BOUNDARY_JOURNAL_BYTES
            || (self.boundary_count == 0
                && (!self.boundary_head_hash.is_empty() || self.boundary_byte_length != 0))
            || (self.boundary_count > 0
                && (!valid_hash(&self.boundary_head_hash) || self.boundary_byte_length == 0))
            || !(empty || populated)
            || !valid_hash(&self.identity_hash)
            || !valid_hash(&self.initialization_head_hash)
            || !valid_hash(&self.manifest_hash)
            || !valid_hash(&self.cell_id)
            || !valid_hash(&self.head_hash)
            || self.head_hash != self.calculate_hash()?
        {
            return Err(invalid(
                "event-17 head does not match the initialized world-21 Store",
            ));
        }
        if let Some(pending) = &self.pending_event {
            pending.validate_against(self)?;
        }
        Ok(())
    }

    fn stage_pending(
        &self,
        pending: DraftWorld21PendingEventV17,
    ) -> Result<Self, DraftWorld21StoreError> {
        if self.pending_event.is_some() {
            return Err(invalid("event-17 head already has a pending append"));
        }
        pending.validate_against(self)?;
        let mut staged = self.clone();
        staged.pending_event = Some(pending);
        staged.reseal()?;
        Ok(staged)
    }

    fn rollback_pending(&self) -> Result<Self, DraftWorld21StoreError> {
        let mut rolled_back = self.clone();
        rolled_back.pending_event = None;
        rolled_back.reseal()?;
        Ok(rolled_back)
    }

    fn commit_pending(&self) -> Result<Self, DraftWorld21StoreError> {
        let pending = self
            .pending_event
            .as_ref()
            .ok_or_else(|| invalid("event-17 head has no pending append to commit"))?;
        pending.validate_against(self)?;
        let mut committed = self.clone();
        committed.committed_event_count = committed
            .committed_event_count
            .checked_add(1)
            .ok_or_else(|| invalid("event-17 committed count overflowed"))?;
        committed.committed_event_sequence = pending.event_sequence;
        committed
            .committed_event_hash
            .clone_from(&pending.event_hash);
        committed
            .committed_state_hash
            .clone_from(&pending.resulting_state_hash);
        committed.journal_byte_length = pending.journal_end;
        committed.boundary_count = committed
            .boundary_count
            .checked_add(1)
            .ok_or_else(|| invalid("event-17 boundary count overflowed"))?;
        committed
            .boundary_head_hash
            .clone_from(&pending.boundary_hash);
        committed.boundary_byte_length = pending.boundary_end;
        committed.pending_event = None;
        committed.reseal()?;
        Ok(committed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DraftWorld21AppendFailpoint {
    BeforePendingHeadWrite,
    PendingHeadTempSyncedBeforeRename,
    PendingHeadRenamedBeforeDirectorySync,
    PendingHeadDirectorySyncedBeforeEvent,
    PendingHeadSynced,
    PartialEventWritten,
    EventLineWrittenBeforeSync,
    EventSyncedBeforeCommitHead,
    PartialBoundaryWritten,
    BoundaryLineWrittenBeforeSync,
    BoundarySyncedBeforeCommitHead,
    CommitHeadTempSyncedBeforeRename,
    CommitHeadRenamedBeforeDirectorySync,
    CommitHeadDirectorySyncedBeforeMemory,
}

#[derive(Debug)]
pub(super) struct DraftWorld21EventJournal {
    root: PathBuf,
    head: DraftWorld21EventHeadV17,
    journal_file: File,
    boundary_file: File,
    poisoned: bool,
    failpoint: Option<DraftWorld21AppendFailpoint>,
}

impl DraftWorld21EventJournal {
    pub(super) fn initialize_head_file(
        root: &Path,
        identity: &DraftWorld21StoreIdentity,
        initialization: &DraftWorld21InitializationHead,
    ) -> Result<(), DraftWorld21StoreError> {
        let head = DraftWorld21EventHeadV17::new(identity, initialization)?;
        persist_head_atomic(root, &head)
    }

    pub(super) fn recover_empty(
        root: &Path,
        identity: &DraftWorld21StoreIdentity,
        initialization: &DraftWorld21InitializationHead,
        snapshot: &DraftGridTransferCellStateV2,
    ) -> Result<Self, DraftWorld21StoreError> {
        let head = read_head(root, identity, initialization)?;
        let journal_path = root.join(super::EVENT_JOURNAL_FILE);
        let journal_length = fs::metadata(&journal_path)
            .map_err(|source| io_error(&journal_path, source))?
            .len();
        let boundary_path = root.join(EVENT_BOUNDARY_FILE);
        let boundary_length = fs::metadata(&boundary_path)
            .map_err(|source| io_error(&boundary_path, source))?
            .len();
        if head.committed_event_count != 0
            || head.pending_event.is_some()
            || journal_length != 0
            || boundary_length != 0
            || head.committed_state_hash != snapshot.state_hash()
        {
            return Err(invalid(
                "recovery-only world-21 Store refuses a nonempty event-17 frontier",
            ));
        }
        Self::open_append(root, head)
    }

    pub(super) fn recover_with_history(
        root: &Path,
        identity: &DraftWorld21StoreIdentity,
        initialization: &DraftWorld21InitializationHead,
        snapshot: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<(Self, DraftGridTransferCellStateV2), DraftWorld21StoreError> {
        let head = read_head(root, identity, initialization)?;
        let (head, state) = replay(
            root,
            head,
            snapshot,
            manifest,
            directory_history,
            identity,
            initialization,
        )?;
        Ok((Self::open_append(root, head)?, state))
    }

    fn open_append(
        root: &Path,
        head: DraftWorld21EventHeadV17,
    ) -> Result<Self, DraftWorld21StoreError> {
        let journal_path = root.join(super::EVENT_JOURNAL_FILE);
        let journal_file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&journal_path)
            .map_err(|source| io_error(&journal_path, source))?;
        let boundary_path = root.join(EVENT_BOUNDARY_FILE);
        let boundary_file = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&boundary_path)
            .map_err(|source| io_error(&boundary_path, source))?;
        Ok(Self {
            root: root.to_owned(),
            head,
            journal_file,
            boundary_file,
            poisoned: false,
            failpoint: None,
        })
    }

    pub(super) fn append_live(
        &mut self,
        state: &DraftGridTransferCellStateV2,
        manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
        event: &DraftCanonicalGridEventV17,
        authority: &ValidatedCurrentGridEventAuthorityV17<'_, '_>,
    ) -> Result<DraftGridEventApplicationV17, DraftWorld21StoreError> {
        if self.poisoned {
            return Err(invalid(
                "event-17 write outcome is uncertain; reopen before retry",
            ));
        }
        if self.head.pending_event.is_some()
            || self.head.committed_event_sequence != state.base().event_sequence
            || self.head.committed_event_hash != state.base().last_event_hash
            || self.head.committed_state_hash != state.state_hash()
        {
            return Err(invalid(
                "event-17 in-memory state and durable head frontiers disagree",
            ));
        }
        let application = apply_live_manifest_bound(state, manifest, event, authority)?;
        let mut record = event
            .encode_canonical()
            .map_err(|source| invalid(source.to_string()))?;
        record.push(b'\n');
        if record.len() > MAX_EVENT_RECORD_BYTES {
            return Err(invalid("event-17 record exceeds its byte bound"));
        }
        let boundary = DraftWorld21EventBoundaryV17::new(
            event,
            &application,
            self.head.boundary_head_hash.clone(),
        )?;
        let boundary_record = boundary.encode_record()?;
        let journal_path = self.root.join(super::EVENT_JOURNAL_FILE);
        let journal_length = self
            .journal_file
            .metadata()
            .map_err(|source| io_error(&journal_path, source))?
            .len();
        if journal_length != self.head.journal_byte_length {
            return Err(invalid(
                "event-17 journal and in-memory head frontiers disagree",
            ));
        }
        let boundary_path = self.root.join(EVENT_BOUNDARY_FILE);
        let boundary_length = self
            .boundary_file
            .metadata()
            .map_err(|source| io_error(&boundary_path, source))?
            .len();
        if boundary_length != self.head.boundary_byte_length {
            return Err(invalid(
                "event-17 boundary journal and in-memory head frontiers disagree",
            ));
        }
        let pending = DraftWorld21PendingEventV17::new(
            event,
            &application.next_state,
            journal_length,
            &record,
            &boundary,
            boundary_length,
            &boundary_record,
        )?;
        let pending_head = self.head.stage_pending(pending)?;

        if self.consume_failpoint(DraftWorld21AppendFailpoint::BeforePendingHeadWrite) {
            return Err(injected("before pending event-17 head write"));
        }
        if let Err(error) = self.persist_pending_head(&pending_head) {
            self.poisoned = true;
            return Err(error);
        }
        if self.consume_failpoint(DraftWorld21AppendFailpoint::PendingHeadSynced) {
            self.poisoned = true;
            return Err(injected("after pending event-17 head sync"));
        }
        if self.consume_failpoint(DraftWorld21AppendFailpoint::PartialEventWritten) {
            let partial_length = record.len().div_ceil(2).max(1);
            let result = self
                .journal_file
                .write_all(&record[..partial_length])
                .and_then(|()| self.journal_file.sync_data());
            self.poisoned = true;
            result.map_err(|source| io_error(&journal_path, source))?;
            return Err(injected("after partial event-17 journal write"));
        }
        if let Err(source) = self.journal_file.write_all(&record) {
            self.poisoned = true;
            return Err(io_error(&journal_path, source));
        }
        if self.consume_failpoint(DraftWorld21AppendFailpoint::EventLineWrittenBeforeSync) {
            self.poisoned = true;
            return Err(injected("after event-17 line write before sync"));
        }
        if let Err(source) = self.journal_file.sync_data() {
            self.poisoned = true;
            return Err(io_error(&journal_path, source));
        }
        if self.consume_failpoint(DraftWorld21AppendFailpoint::EventSyncedBeforeCommitHead) {
            self.poisoned = true;
            return Err(injected("after event-17 sync before boundary"));
        }
        if self.consume_failpoint(DraftWorld21AppendFailpoint::PartialBoundaryWritten) {
            let partial_length = boundary_record.len().div_ceil(2).max(1);
            let result = self
                .boundary_file
                .write_all(&boundary_record[..partial_length])
                .and_then(|()| self.boundary_file.sync_data());
            self.poisoned = true;
            result.map_err(|source| io_error(&boundary_path, source))?;
            return Err(injected("after partial event-17 boundary write"));
        }
        if let Err(source) = self.boundary_file.write_all(&boundary_record) {
            self.poisoned = true;
            return Err(io_error(&boundary_path, source));
        }
        if self.consume_failpoint(DraftWorld21AppendFailpoint::BoundaryLineWrittenBeforeSync) {
            self.poisoned = true;
            return Err(injected("after event-17 boundary write before sync"));
        }
        if let Err(source) = self.boundary_file.sync_data() {
            self.poisoned = true;
            return Err(io_error(&boundary_path, source));
        }
        if self.consume_failpoint(DraftWorld21AppendFailpoint::BoundarySyncedBeforeCommitHead) {
            self.poisoned = true;
            return Err(injected(
                "after event-17 boundary sync before committed head",
            ));
        }

        let committed_head = match pending_head.commit_pending() {
            Ok(head) => head,
            Err(error) => {
                self.poisoned = true;
                return Err(error);
            }
        };
        if let Err(error) = self.persist_committed_head(&committed_head) {
            self.poisoned = true;
            return Err(error);
        }
        self.head = committed_head;
        Ok(application)
    }

    fn persist_pending_head(
        &mut self,
        head: &DraftWorld21EventHeadV17,
    ) -> Result<(), DraftWorld21StoreError> {
        let path = self.root.join(EVENT_HEAD_FILE);
        let bytes = serde_json::to_vec(head).map_err(|source| invalid(source.to_string()))?;
        if bytes.len() > MAX_EVENT_HEAD_BYTES {
            return Err(invalid("pending event-17 head exceeds its byte bound"));
        }
        let temporary = self
            .root
            .join(format!(".{EVENT_HEAD_FILE}.tmp-{}", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|source| io_error(&temporary, source))?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|source| io_error(&temporary, source))?;
            if self
                .consume_failpoint(DraftWorld21AppendFailpoint::PendingHeadTempSyncedBeforeRename)
            {
                return Err(injected("before pending event-17 head rename"));
            }
            fs::rename(&temporary, &path).map_err(|source| io_error(&path, source))?;
            if self.consume_failpoint(
                DraftWorld21AppendFailpoint::PendingHeadRenamedBeforeDirectorySync,
            ) {
                return Err(injected("after pending event-17 head rename"));
            }
            sync_directory(&self.root)?;
            if self.consume_failpoint(
                DraftWorld21AppendFailpoint::PendingHeadDirectorySyncedBeforeEvent,
            ) {
                return Err(injected("after pending event-17 head directory sync"));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn persist_committed_head(
        &mut self,
        head: &DraftWorld21EventHeadV17,
    ) -> Result<(), DraftWorld21StoreError> {
        let path = self.root.join(EVENT_HEAD_FILE);
        let bytes = serde_json::to_vec(head).map_err(|source| invalid(source.to_string()))?;
        if bytes.len() > MAX_EVENT_HEAD_BYTES {
            return Err(invalid("committed event-17 head exceeds its byte bound"));
        }
        let temporary = self
            .root
            .join(format!(".{EVENT_HEAD_FILE}.tmp-{}", Uuid::new_v4()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|source| io_error(&temporary, source))?;
            file.write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|source| io_error(&temporary, source))?;
            if self.consume_failpoint(DraftWorld21AppendFailpoint::CommitHeadTempSyncedBeforeRename)
            {
                return Err(injected("before committed event-17 head rename"));
            }
            fs::rename(&temporary, &path).map_err(|source| io_error(&path, source))?;
            if self.consume_failpoint(
                DraftWorld21AppendFailpoint::CommitHeadRenamedBeforeDirectorySync,
            ) {
                return Err(injected("after committed event-17 head rename"));
            }
            sync_directory(&self.root)?;
            if self.consume_failpoint(
                DraftWorld21AppendFailpoint::CommitHeadDirectorySyncedBeforeMemory,
            ) {
                return Err(injected("after committed event-17 head directory sync"));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    pub(super) fn set_failpoint(&mut self, failpoint: DraftWorld21AppendFailpoint) {
        self.failpoint = Some(failpoint);
    }

    fn consume_failpoint(&mut self, failpoint: DraftWorld21AppendFailpoint) -> bool {
        if self.failpoint == Some(failpoint) {
            self.failpoint = None;
            true
        } else {
            false
        }
    }

    pub(super) fn committed_event_count(&self) -> u64 {
        self.head.committed_event_count
    }
}

fn read_head(
    root: &Path,
    identity: &DraftWorld21StoreIdentity,
    initialization: &DraftWorld21InitializationHead,
) -> Result<DraftWorld21EventHeadV17, DraftWorld21StoreError> {
    let path = root.join(EVENT_HEAD_FILE);
    let bytes = read_bounded(&path, MAX_EVENT_HEAD_BYTES)?;
    let head = serde_json::from_slice::<DraftWorld21EventHeadV17>(&bytes)
        .map_err(|source| invalid(source.to_string()))?;
    let canonical = serde_json::to_vec(&head).map_err(|source| invalid(source.to_string()))?;
    if canonical != bytes {
        return Err(invalid("event-17 head bytes are not canonical"));
    }
    head.validate(identity, initialization)?;
    Ok(head)
}

fn persist_head_atomic(
    root: &Path,
    head: &DraftWorld21EventHeadV17,
) -> Result<(), DraftWorld21StoreError> {
    let bytes = serde_json::to_vec(head).map_err(|source| invalid(source.to_string()))?;
    if bytes.len() > MAX_EVENT_HEAD_BYTES {
        return Err(invalid("event-17 head exceeds its byte bound"));
    }
    atomic_write(&root.join(EVENT_HEAD_FILE), &bytes)
}

fn replay(
    root: &Path,
    mut head: DraftWorld21EventHeadV17,
    snapshot: &DraftGridTransferCellStateV2,
    manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
    directory_history: &DraftCellDirectoryHistoryStoreV3,
    identity: &DraftWorld21StoreIdentity,
    initialization: &DraftWorld21InitializationHead,
) -> Result<(DraftWorld21EventHeadV17, DraftGridTransferCellStateV2), DraftWorld21StoreError> {
    let journal_path = root.join(super::EVENT_JOURNAL_FILE);
    let file_length = fs::metadata(&journal_path)
        .map_err(|source| io_error(&journal_path, source))?
        .len();
    if file_length > MAX_EVENT_JOURNAL_BYTES || file_length < head.journal_byte_length {
        return Err(invalid(
            "event-17 journal length is outside its committed bounds",
        ));
    }
    let boundary_path = root.join(EVENT_BOUNDARY_FILE);
    let boundary_file_length = fs::metadata(&boundary_path)
        .map_err(|source| io_error(&boundary_path, source))?
        .len();
    if boundary_file_length > MAX_EVENT_BOUNDARY_JOURNAL_BYTES
        || boundary_file_length < head.boundary_byte_length
    {
        return Err(invalid(
            "event-17 boundary journal length is outside its committed bounds",
        ));
    }
    let read_file = File::open(&journal_path).map_err(|source| io_error(&journal_path, source))?;
    let mut reader = BufReader::new(read_file);
    let boundary_read_file =
        File::open(&boundary_path).map_err(|source| io_error(&boundary_path, source))?;
    let mut boundary_reader = BufReader::new(boundary_read_file);
    let mut offset = 0_u64;
    let mut boundary_offset = 0_u64;
    let mut replayed_count = 0_u64;
    let mut observed_boundary_head = String::new();
    let mut state = snapshot.clone();

    while offset < head.journal_byte_length {
        let (record, terminated) =
            read_record_bounded(&mut reader, &journal_path, MAX_DRAFT_GRID_EVENT_BYTES)?
                .ok_or_else(|| invalid("event-17 journal is shorter than its committed head"))?;
        if !terminated {
            return Err(invalid(
                "event-17 committed prefix ends in an unterminated record",
            ));
        }
        let end = offset
            .checked_add(u64::try_from(record.len() + 1).unwrap_or(u64::MAX))
            .ok_or_else(|| invalid("event-17 replay offset overflowed"))?;
        if end > head.journal_byte_length {
            return Err(invalid(
                "event-17 committed head is not on a record boundary",
            ));
        }
        let event = DraftCanonicalGridEventV17::decode_canonical(&record)
            .map_err(|source| invalid(source.to_string()))?;
        let application =
            apply_historical_manifest_bound(&state, manifest, &event, directory_history)?;
        let expected_boundary = DraftWorld21EventBoundaryV17::new(
            &event,
            &application,
            observed_boundary_head.clone(),
        )?;
        let (boundary_record, boundary_terminated) = read_record_bounded(
            &mut boundary_reader,
            &boundary_path,
            MAX_EVENT_BOUNDARY_RECORD_BYTES - 1,
        )?
        .ok_or_else(|| invalid("event-17 boundary journal is shorter than its committed head"))?;
        if !boundary_terminated {
            return Err(invalid(
                "event-17 committed boundary prefix ends in an unterminated record",
            ));
        }
        let boundary_end = boundary_offset
            .checked_add(u64::try_from(boundary_record.len() + 1).unwrap_or(u64::MAX))
            .ok_or_else(|| invalid("event-17 boundary replay offset overflowed"))?;
        if boundary_end > head.boundary_byte_length {
            return Err(invalid(
                "event-17 committed boundary head is not on a record boundary",
            ));
        }
        let persisted_boundary = DraftWorld21EventBoundaryV17::decode_record(&boundary_record)?;
        if persisted_boundary != expected_boundary {
            return Err(invalid(
                "event-17 boundary does not match its replay-derived proof",
            ));
        }
        observed_boundary_head = persisted_boundary.boundary_hash;
        boundary_offset = boundary_end;
        state = application.next_state;
        replayed_count = replayed_count
            .checked_add(1)
            .ok_or_else(|| invalid("event-17 replay count overflowed"))?;
        offset = end;
    }
    if replayed_count != head.committed_event_count
        || state.base().event_sequence != head.committed_event_sequence
        || state.base().last_event_hash != head.committed_event_hash
        || state.state_hash() != head.committed_state_hash
        || boundary_offset != head.boundary_byte_length
        || replayed_count != head.boundary_count
        || observed_boundary_head != head.boundary_head_hash
    {
        return Err(invalid(
            "event-17 committed head does not match replayed canonical state",
        ));
    }

    let suffix = read_record_bounded(&mut reader, &journal_path, MAX_DRAFT_GRID_EVENT_BYTES)?;
    match suffix {
        None => {
            if read_record_bounded(
                &mut boundary_reader,
                &boundary_path,
                MAX_EVENT_BOUNDARY_RECORD_BYTES - 1,
            )?
            .is_some()
            {
                return Err(invalid(
                    "event-17 boundary journal advances without its pending event",
                ));
            }
            if head.pending_event.is_some() {
                head = head.rollback_pending()?;
                head.validate(identity, initialization)?;
                persist_head_atomic(root, &head)?;
            }
        }
        Some((_record, false)) => {
            let pending = head
                .pending_event
                .as_ref()
                .ok_or_else(|| invalid("event-17 journal has an unpinned partial suffix"))?;
            if file_length <= head.journal_byte_length
                || file_length >= pending.journal_end
                || boundary_file_length != head.boundary_byte_length
            {
                return Err(invalid("event-17 journal has an unpinned partial suffix"));
            }
            if read_record_bounded(
                &mut boundary_reader,
                &boundary_path,
                MAX_EVENT_BOUNDARY_RECORD_BYTES - 1,
            )?
            .is_some()
            {
                return Err(invalid(
                    "event-17 boundary advances while its pending event is partial",
                ));
            }
            truncate_journal(&journal_path, head.journal_byte_length)?;
            head = head.rollback_pending()?;
            head.validate(identity, initialization)?;
            persist_head_atomic(root, &head)?;
        }
        Some((record, true)) => {
            let pending = head.pending_event.as_ref().ok_or_else(|| {
                invalid("event-17 journal has a complete record beyond its committed head")
            })?;
            let end = offset
                .checked_add(u64::try_from(record.len() + 1).unwrap_or(u64::MAX))
                .ok_or_else(|| invalid("event-17 pending replay offset overflowed"))?;
            let event = DraftCanonicalGridEventV17::decode_canonical(&record)
                .map_err(|source| invalid(source.to_string()))?;
            let application =
                apply_historical_manifest_bound(&state, manifest, &event, directory_history)?;
            let mut encoded_record = record.clone();
            encoded_record.push(b'\n');
            let expected_boundary = DraftWorld21EventBoundaryV17::new(
                &event,
                &application,
                head.boundary_head_hash.clone(),
            )?;
            let expected_boundary_record = expected_boundary.encode_record()?;
            let boundary_end = boundary_offset
                .checked_add(u64::try_from(expected_boundary_record.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| invalid("event-17 pending boundary offset overflowed"))?;
            if !pending.matches(
                &event,
                &application.next_state,
                &encoded_record,
                &expected_boundary,
                DraftWorld21PendingRangesV17 {
                    journal_start: offset,
                    journal_end: end,
                    boundary_start: boundary_offset,
                    boundary_end,
                },
            ) {
                return Err(invalid(
                    "event-17 durable suffix does not equal the pending append",
                ));
            }
            if read_record_bounded(&mut reader, &journal_path, MAX_DRAFT_GRID_EVENT_BYTES)?
                .is_some()
            {
                return Err(invalid(
                    "event-17 journal has records beyond its one pending append",
                ));
            }
            match read_record_bounded(
                &mut boundary_reader,
                &boundary_path,
                MAX_EVENT_BOUNDARY_RECORD_BYTES - 1,
            )? {
                None => append_boundary_record(
                    &boundary_path,
                    head.boundary_byte_length,
                    &expected_boundary_record,
                )?,
                Some((_boundary_record, false)) => {
                    if boundary_file_length <= head.boundary_byte_length
                        || boundary_file_length >= pending.boundary_end
                    {
                        return Err(invalid(
                            "event-17 pending boundary suffix exceeds its sealed range",
                        ));
                    }
                    truncate_journal(&boundary_path, head.boundary_byte_length)?;
                    append_boundary_record(
                        &boundary_path,
                        head.boundary_byte_length,
                        &expected_boundary_record,
                    )?;
                }
                Some((boundary_record, true)) => {
                    let persisted = DraftWorld21EventBoundaryV17::decode_record(&boundary_record)?;
                    if persisted != expected_boundary
                        || read_record_bounded(
                            &mut boundary_reader,
                            &boundary_path,
                            MAX_EVENT_BOUNDARY_RECORD_BYTES - 1,
                        )?
                        .is_some()
                    {
                        return Err(invalid(
                            "event-17 boundary suffix does not equal its pending proof",
                        ));
                    }
                }
            }
            head = head.commit_pending()?;
            head.validate(identity, initialization)?;
            persist_head_atomic(root, &head)?;
            state = application.next_state;
        }
    }
    Ok((head, state))
}

fn apply_live_manifest_bound(
    state: &DraftGridTransferCellStateV2,
    manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
    event: &DraftCanonicalGridEventV17,
    authority: &ValidatedCurrentGridEventAuthorityV17<'_, '_>,
) -> Result<DraftGridEventApplicationV17, DraftWorld21StoreError> {
    let application = match authority {
        ValidatedCurrentGridEventAuthorityV17::Grid(current) => {
            let validated = (*current).validated();
            let rebound = event
                .rebind_world_v21_for_state(
                    state,
                    manifest,
                    ValidatedDraftGridEventAuthorityV17::Grid(validated),
                )
                .map_err(|source| invalid(source.to_string()))?;
            let state_capability = rebound
                .validate_world_v21(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let bound = validated
                .bind_manifest_v5(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let event_capability = event
                .validate_world_v21(
                    &state_capability,
                    ValidatedManifestBoundGridEventAuthorityV17::Grid(&bound),
                )
                .map_err(|source| invalid(source.to_string()))?;
            apply_manifest_bound_event_v17(&event_capability)
        }
        ValidatedCurrentGridEventAuthorityV17::Production(current) => {
            let validated = (*current).validated();
            let rebound = event
                .rebind_world_v21_for_state(
                    state,
                    manifest,
                    ValidatedDraftGridEventAuthorityV17::Production(validated),
                )
                .map_err(|source| invalid(source.to_string()))?;
            let state_capability = rebound
                .validate_world_v21(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let bound = validated
                .bind_manifest_v5(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let event_capability = event
                .validate_world_v21(
                    &state_capability,
                    ValidatedManifestBoundGridEventAuthorityV17::Production(&bound),
                )
                .map_err(|source| invalid(source.to_string()))?;
            apply_manifest_bound_event_v17(&event_capability)
        }
    }
    .map_err(|source| invalid(source.to_string()))?;
    application
        .next_state
        .validate_world_v21(manifest)
        .map_err(|source| invalid(source.to_string()))?;
    Ok(application)
}

fn apply_historical_manifest_bound(
    state: &DraftGridTransferCellStateV2,
    manifest: &crate::manifest_v5::ValidatedUniverseManifestV5,
    event: &DraftCanonicalGridEventV17,
    directory_history: &DraftCellDirectoryHistoryStoreV3,
) -> Result<DraftGridEventApplicationV17, DraftWorld21StoreError> {
    let application = match event.authority_lookup() {
        DraftGridEventAuthorityLookupV17::Grid {
            directory_revision,
            directory_document_hash,
            transfer_id,
        } => {
            let validated = directory_history
                .resolve_historical_grid_authority(
                    directory_revision,
                    directory_document_hash,
                    transfer_id,
                )
                .map_err(|source| invalid(source.to_string()))?;
            let rebound = event
                .rebind_world_v21_for_state(
                    state,
                    manifest,
                    ValidatedDraftGridEventAuthorityV17::Grid(&validated),
                )
                .map_err(|source| invalid(source.to_string()))?;
            let state_capability = rebound
                .validate_world_v21(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let bound = validated
                .bind_manifest_v5(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let event_capability = event
                .validate_world_v21(
                    &state_capability,
                    ValidatedManifestBoundGridEventAuthorityV17::Grid(&bound),
                )
                .map_err(|source| invalid(source.to_string()))?;
            apply_manifest_bound_event_v17(&event_capability)
        }
        DraftGridEventAuthorityLookupV17::Production {
            directory_revision,
            directory_document_hash,
            cell_id,
        } => {
            let validated = directory_history
                .resolve_historical_cell_authority(
                    directory_revision,
                    directory_document_hash,
                    cell_id,
                )
                .map_err(|source| invalid(source.to_string()))?;
            let rebound = event
                .rebind_world_v21_for_state(
                    state,
                    manifest,
                    ValidatedDraftGridEventAuthorityV17::Production(&validated),
                )
                .map_err(|source| invalid(source.to_string()))?;
            let state_capability = rebound
                .validate_world_v21(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let bound = validated
                .bind_manifest_v5(manifest)
                .map_err(|source| invalid(source.to_string()))?;
            let event_capability = event
                .validate_world_v21(
                    &state_capability,
                    ValidatedManifestBoundGridEventAuthorityV17::Production(&bound),
                )
                .map_err(|source| invalid(source.to_string()))?;
            apply_manifest_bound_event_v17(&event_capability)
        }
    }
    .map_err(|source| invalid(source.to_string()))?;
    application
        .next_state
        .validate_world_v21(manifest)
        .map_err(|source| invalid(source.to_string()))?;
    Ok(application)
}

fn read_record_bounded(
    reader: &mut BufReader<File>,
    path: &Path,
    maximum: usize,
) -> Result<Option<(Vec<u8>, bool)>, DraftWorld21StoreError> {
    let mut record = Vec::new();
    loop {
        let available = reader.fill_buf().map_err(|source| io_error(path, source))?;
        if available.is_empty() {
            return if record.is_empty() {
                Ok(None)
            } else {
                Ok(Some((record, false)))
            };
        }
        if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
            if record.len().saturating_add(newline) > maximum {
                return Err(invalid("event-17 journal record exceeds its byte bound"));
            }
            record.extend_from_slice(&available[..newline]);
            reader.consume(newline + 1);
            return Ok(Some((record, true)));
        }
        if record.len().saturating_add(available.len()) > maximum {
            return Err(invalid("event-17 journal record exceeds its byte bound"));
        }
        let consumed = available.len();
        record.extend_from_slice(available);
        reader.consume(consumed);
    }
}

fn append_boundary_record(
    path: &Path,
    expected_length: u64,
    record: &[u8],
) -> Result<(), DraftWorld21StoreError> {
    let mut file = OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    let found = file
        .metadata()
        .map_err(|source| io_error(path, source))?
        .len();
    if found != expected_length {
        return Err(invalid(
            "event-17 boundary backfill compare-and-swap is stale",
        ));
    }
    file.write_all(record)
        .and_then(|()| file.sync_data())
        .map_err(|source| io_error(path, source))
}

fn truncate_journal(path: &Path, length: u64) -> Result<(), DraftWorld21StoreError> {
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|source| io_error(path, source))?;
    file.set_len(length)
        .and_then(|()| file.sync_all())
        .map_err(|source| io_error(path, source))
}

fn event_record_hash(record: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(EVENT_RECORD_HASH_DOMAIN);
    hasher.update(record);
    hasher.finalize().to_hex().to_string()
}

pub(super) fn proof_hash(proof: &DraftGridEventProofV17) -> &str {
    match proof {
        DraftGridEventProofV17::Prepared(proof) => &proof.proof_hash,
        DraftGridEventProofV17::Quarantined { proof, .. } => &proof.proof_hash,
        DraftGridEventProofV17::Exported(proof) => &proof.proof_hash,
        DraftGridEventProofV17::Imported(proof) => &proof.proof_hash,
        DraftGridEventProofV17::Activated(proof) => &proof.proof_hash,
        DraftGridEventProofV17::Finalized(proof) => &proof.proof_hash,
        DraftGridEventProofV17::Aborted(proof) => &proof.proof_hash,
        DraftGridEventProofV17::Production { proof, .. } => &proof.proof_hash,
    }
}

#[cfg(test)]
pub(super) fn substitute_head_manifest_for_test(
    root: &Path,
    manifest_hash: String,
) -> Result<(), DraftWorld21StoreError> {
    let path = root.join(EVENT_HEAD_FILE);
    let bytes = read_bounded(&path, MAX_EVENT_HEAD_BYTES)?;
    let mut head = serde_json::from_slice::<DraftWorld21EventHeadV17>(&bytes)
        .map_err(|source| invalid(source.to_string()))?;
    head.manifest_hash = manifest_hash;
    head.reseal()?;
    persist_head_atomic(root, &head)
}

fn valid_frontier(sequence: u64, hash: &str) -> bool {
    (sequence == 0 && hash.is_empty()) || (sequence > 0 && valid_hash(hash))
}

fn invalid(message: impl Into<String>) -> DraftWorld21StoreError {
    DraftWorld21StoreError::Invalid(message.into())
}

fn injected(message: &'static str) -> DraftWorld21StoreError {
    DraftWorld21StoreError::InjectedAppend(message)
}
