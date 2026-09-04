// SPDX-License-Identifier: AGPL-3.0-or-later

//! Activated protocol-19 lifecycle-v2 journal and scheduling state.
//!
//! The immutable migration lifecycle remains receipt material. This module
//! owns only the runtime successor chain and never grants directory authority.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use verse_protocol::{CellKeyV1, protocol_v19::Protocol19CompatibilityTuple};

use crate::cell_directory::CellAssignmentState;
use crate::cell_directory_v3::{
    DraftCellDirectoryHistoryStoreV3, ValidatedCurrentCellAssignmentV3,
};
use crate::event::{PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION, ProductionScheduleOccurrence};
use crate::model::WorldState;
use crate::protocol19_migration::Protocol19TargetLifecycleGenesisV2;

pub(crate) const LIFECYCLE_HISTORY_FILE: &str = "lifecycle-v2.ndjson";
pub(crate) const LIFECYCLE_HEAD_FILE: &str = "lifecycle-v2.head.json";
const LIFECYCLE_RECORD_SCHEMA_VERSION: u32 = 2;
const LIFECYCLE_HISTORY_ENTRY_SCHEMA_VERSION: u32 = 1;
const LIFECYCLE_HEAD_SCHEMA_VERSION: u32 = 1;
const LIFECYCLE_RECORD_HASH_DOMAIN: &[u8] = b"the-verse/cell-lifecycle-record/v2\0";
const LIFECYCLE_HISTORY_HASH_DOMAIN: &[u8] = b"the-verse/cell-lifecycle-history/v2\0";
const LIFECYCLE_HEAD_HASH_DOMAIN: &[u8] = b"the-verse/cell-lifecycle-head/v2\0";
const MAX_LIFECYCLE_RECORD_BYTES: usize = 256 * 1_024;
const MAX_LIFECYCLE_HEAD_BYTES: usize = 64 * 1_024;
const MAX_LIFECYCLE_HISTORY_BYTES: u64 = 512 * 1_024 * 1_024;
pub(crate) const MAX_STALE_LIFECYCLE_HEAD_TEMPS: usize = 64;
pub(crate) const AUTHORITY_DURATION_MILLIS: u64 = 15_000;
pub(crate) const EVENT_APPEND_LEASE_MARGIN_MILLIS: u64 = 5_000;
pub(crate) const PRODUCTION_QUANTUM_MILLIS: u64 = 1_000;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LifecycleAppendFailpointV2 {
    JournalSyncedBeforeHead,
    HeadRenamedBeforeMemory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LifecycleInitializationFailpointV2 {
    EmptyHistoryCreated,
    EmptyHeadCommitted,
    InitialJournalSyncedBeforeHead,
    InitialHeadRenamedBeforeMemory,
}

#[derive(Debug, Error)]
pub(crate) enum LifecycleV2Error {
    #[error("lifecycle-v2 I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid lifecycle-v2 state: {0}")]
    Invalid(String),
    #[error("lifecycle-v2 arithmetic exhausted: {0}")]
    Exhausted(&'static str),
    #[error("lifecycle-v2 write outcome is uncertain; reopen before retry")]
    Poisoned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LifecycleRuntimePreflightV2 {
    Absent,
    EmptyHistoryOnly,
    EmptyPair,
    Committed(LifecycleHeadCommitmentV2),
}

impl LifecycleRuntimePreflightV2 {
    pub(crate) fn committed(&self) -> Option<&LifecycleHeadCommitmentV2> {
        match self {
            Self::Committed(commitment) => Some(commitment),
            Self::Absent | Self::EmptyHistoryOnly | Self::EmptyPair => None,
        }
    }

    pub(crate) fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    pub(crate) fn is_uncommitted_bootstrap(&self) -> bool {
        matches!(
            self,
            Self::Absent | Self::EmptyHistoryOnly | Self::EmptyPair
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleModeV2 {
    Sleeping,
    Activating,
    Background,
    Draining,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LifecycleAuthorityOperationKindV2 {
    Claim,
    Recovery,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LifecycleAuthorityOperationV2 {
    kind: LifecycleAuthorityOperationKindV2,
    operation_id: String,
    expected_assignment_generation: u64,
    holder_id: String,
    requested_at_unix_ms: u64,
}

impl LifecycleAuthorityOperationV2 {
    pub(crate) fn kind(&self) -> LifecycleAuthorityOperationKindV2 {
        self.kind
    }

    pub(crate) fn expected_assignment_generation(&self) -> u64 {
        self.expected_assignment_generation
    }

    pub(crate) fn holder_id(&self) -> &str {
        &self.holder_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PendingProductionCommitV2 {
    occurrence: ProductionScheduleOccurrence,
    prior_event_sequence: u64,
    prior_event_hash: String,
    prior_state_hash: String,
}

impl PendingProductionCommitV2 {
    pub(crate) fn occurrence(&self) -> &ProductionScheduleOccurrence {
        &self.occurrence
    }

    pub(crate) fn prior_event_sequence(&self) -> u64 {
        self.prior_event_sequence
    }

    pub(crate) fn prior_event_hash(&self) -> &str {
        &self.prior_event_hash
    }

    pub(crate) fn prior_state_hash(&self) -> &str {
        &self.prior_state_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LifecycleWorldFrontierV2 {
    pub(crate) event_sequence: u64,
    pub(crate) event_hash: String,
    pub(crate) state_hash: String,
    pub(crate) active_world_hash: String,
    pub(crate) celestial_registry_hash: String,
    pub(crate) production_lifecycle_generation: u64,
    pub(crate) committed_production_sequence: u64,
    pub(crate) last_scheduled_for_unix_ms: u64,
}

impl LifecycleWorldFrontierV2 {
    pub(crate) fn from_world(
        world: &WorldState,
        state_hash: &str,
        active_world_hash: String,
    ) -> Self {
        Self {
            event_sequence: world.event_sequence,
            event_hash: world.last_event_hash.clone(),
            state_hash: state_hash.to_owned(),
            active_world_hash,
            celestial_registry_hash: world.celestial_registry_hash.clone(),
            production_lifecycle_generation: world.production_clock.lifecycle_generation,
            committed_production_sequence: world.production_clock.last_committed_quantum_sequence,
            last_scheduled_for_unix_ms: world.production_clock.last_scheduled_for_unix_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LifecycleRecordV2 {
    schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    universe_id: String,
    cell_key: CellKeyV1,
    cell_id: String,
    manifest_hash: String,
    celestial_registry_hash: String,
    migration_anchor_hash: String,
    lifecycle_genesis_hash: String,
    active_head_hash: String,
    lifecycle_revision: u64,
    desired_mode: LifecycleModeV2,
    observed_mode: LifecycleModeV2,
    directory_revision: u64,
    directory_document_hash: String,
    assignment_generation: u64,
    authority_fencing_token: u64,
    holder_id: Option<String>,
    authority_acquired_at_unix_ms: Option<u64>,
    authority_renewed_at_unix_ms: Option<u64>,
    authority_expires_at_unix_ms: Option<u64>,
    last_trusted_unix_ms: u64,
    activation_cutoff_unix_ms: Option<u64>,
    last_world_event_sequence: u64,
    last_world_event_hash: String,
    last_world_state_hash: String,
    last_active_world_hash: String,
    production_lifecycle_generation: u64,
    acknowledged_production_sequence: u64,
    last_committed_production_scheduled_for_unix_ms: u64,
    next_production_occurrence: Option<ProductionScheduleOccurrence>,
    authority_operation: Option<LifecycleAuthorityOperationV2>,
    pending_world_commit: Option<PendingProductionCommitV2>,
    previous_record_hash: String,
    record_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LifecycleHeadCommitmentV2 {
    lifecycle_genesis_hash: String,
    active_head_hash: String,
    entry_count: u64,
    journal_bytes: u64,
    last_entry_hash: String,
    last_record_hash: String,
    head_hash: String,
}

impl LifecycleHeadCommitmentV2 {
    pub(crate) fn entry_count(&self) -> u64 {
        self.entry_count
    }

    pub(crate) fn last_record_hash(&self) -> &str {
        &self.last_record_hash
    }

    pub(crate) fn validate_identity(
        &self,
        lifecycle_genesis_hash: &str,
        active_head_hash: &str,
    ) -> Result<(), LifecycleV2Error> {
        let head = LifecycleHeadV2 {
            schema_version: LIFECYCLE_HEAD_SCHEMA_VERSION,
            lifecycle_genesis_hash: self.lifecycle_genesis_hash.clone(),
            active_head_hash: self.active_head_hash.clone(),
            entry_count: self.entry_count,
            journal_bytes: self.journal_bytes,
            last_entry_hash: self.last_entry_hash.clone(),
            last_record_hash: self.last_record_hash.clone(),
            head_hash: self.head_hash.clone(),
        };
        if self.lifecycle_genesis_hash != lifecycle_genesis_hash
            || self.active_head_hash != active_head_hash
            || self.entry_count == 0
            || self.journal_bytes == 0
            || !valid_hash(&self.last_entry_hash)
            || !valid_hash(&self.last_record_hash)
            || !valid_hash(&self.head_hash)
            || head.validate().is_err()
        {
            return Err(invalid("lifecycle head commitment identity is invalid"));
        }
        Ok(())
    }

    pub(crate) fn is_direct_successor_of(&self, prior: &Self) -> bool {
        self.lifecycle_genesis_hash == prior.lifecycle_genesis_hash
            && self.active_head_hash == prior.active_head_hash
            && self.entry_count == prior.entry_count.checked_add(1).unwrap_or(0)
            && self.journal_bytes > prior.journal_bytes
            && self.last_entry_hash != prior.last_entry_hash
            && self.last_record_hash != prior.last_record_hash
            && self.head_hash != prior.head_hash
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedLifecycleAppendV2 {
    record: LifecycleRecordV2,
    entry: LifecycleHistoryEntryV2,
    line: Vec<u8>,
    next_head: LifecycleHeadV2,
}

impl PreparedLifecycleAppendV2 {
    pub(crate) fn record(&self) -> &LifecycleRecordV2 {
        &self.record
    }

    pub(crate) fn next_commitment(&self) -> LifecycleHeadCommitmentV2 {
        LifecycleHeadCommitmentV2::from(&self.next_head)
    }
}

impl LifecycleRecordV2 {
    pub(crate) fn record_hash(&self) -> &str {
        &self.record_hash
    }

    pub(crate) fn validate_universe_identity(
        &self,
        cell_id: &str,
        lifecycle_genesis_hash: &str,
        active_head_hash: &str,
    ) -> Result<(), LifecycleV2Error> {
        self.validate()?;
        if self.cell_id != cell_id
            || self.lifecycle_genesis_hash != lifecycle_genesis_hash
            || self.active_head_hash != active_head_hash
        {
            return Err(invalid(
                "lifecycle record belongs to another universe commitment",
            ));
        }
        Ok(())
    }

    fn initial(
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<Self, LifecycleV2Error> {
        let record = assignment.assignment();
        let genesis_event_frontier_matches = frontier.event_sequence
            == genesis.legacy_event_sequence()
            && frontier.event_hash == genesis.legacy_event_head_hash();
        if record.state != CellAssignmentState::Sleeping
            || record.holder_id.is_some()
            || assignment.directory_revision() != genesis.target_directory_revision()
            || assignment.directory_document_hash() != genesis.target_directory_document_hash()
            || record.assignment_generation != genesis.assignment_generation()
            || record.authority_fencing_token != genesis.authority_fencing_token()
            || record.cell_key != *genesis.cell_key()
            || record.cell_id != genesis.cell_id()
            || assignment.universe_manifest_hash() != genesis.manifest_hash()
            || frontier.state_hash != genesis.snapshot_state_hash()
            || frontier.active_world_hash != genesis.active_world_hash()
            || !genesis_event_frontier_matches
            || frontier.committed_production_sequence != genesis.acknowledged_production_sequence()
            || (frontier.committed_production_sequence == 0
                && frontier.last_scheduled_for_unix_ms != 0)
        {
            return Err(invalid(
                "runtime lifecycle genesis differs from its immutable directory or world frontier",
            ));
        }
        let mut next = Self {
            schema_version: LIFECYCLE_RECORD_SCHEMA_VERSION,
            compatibility: Protocol19CompatibilityTuple::canonical(),
            universe_id: assignment.universe_id().to_owned(),
            cell_key: record.cell_key.clone(),
            cell_id: record.cell_id.clone(),
            manifest_hash: assignment.universe_manifest_hash().to_owned(),
            celestial_registry_hash: frontier.celestial_registry_hash.clone(),
            migration_anchor_hash: genesis.migration_anchor_hash().to_owned(),
            lifecycle_genesis_hash: genesis.record_hash().to_owned(),
            active_head_hash: active_head_hash.to_owned(),
            lifecycle_revision: 2,
            desired_mode: LifecycleModeV2::Sleeping,
            observed_mode: LifecycleModeV2::Sleeping,
            directory_revision: assignment.directory_revision(),
            directory_document_hash: assignment.directory_document_hash().to_owned(),
            assignment_generation: record.assignment_generation,
            authority_fencing_token: record.authority_fencing_token,
            holder_id: None,
            authority_acquired_at_unix_ms: None,
            authority_renewed_at_unix_ms: None,
            authority_expires_at_unix_ms: None,
            last_trusted_unix_ms: genesis.trusted_cutoff_unix_ms(),
            activation_cutoff_unix_ms: None,
            last_world_event_sequence: frontier.event_sequence,
            last_world_event_hash: frontier.event_hash.clone(),
            last_world_state_hash: frontier.state_hash.clone(),
            last_active_world_hash: frontier.active_world_hash.clone(),
            production_lifecycle_generation: frontier.production_lifecycle_generation,
            acknowledged_production_sequence: frontier.committed_production_sequence,
            last_committed_production_scheduled_for_unix_ms: frontier.last_scheduled_for_unix_ms,
            next_production_occurrence: None,
            authority_operation: None,
            pending_world_commit: None,
            previous_record_hash: genesis.record_hash().to_owned(),
            record_hash: String::new(),
        };
        next.seal()?;
        Ok(next)
    }

    fn successor(&self, trusted_now_unix_ms: u64) -> Result<Self, LifecycleV2Error> {
        require_monotonic_trusted_time(trusted_now_unix_ms, self.last_trusted_unix_ms)?;
        let mut next = self.clone();
        next.lifecycle_revision = self
            .lifecycle_revision
            .checked_add(1)
            .ok_or(LifecycleV2Error::Exhausted("lifecycle revision"))?;
        next.last_trusted_unix_ms = trusted_now_unix_ms;
        next.previous_record_hash.clone_from(&self.record_hash);
        next.record_hash.clear();
        Ok(next)
    }

    pub(crate) fn request_authority(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        kind: LifecycleAuthorityOperationKindV2,
        holder_id: &str,
        trusted_now_unix_ms: u64,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<Self, LifecycleV2Error> {
        if let (LifecycleAuthorityOperationKindV2::Recovery, Some(pending)) =
            (kind, self.pending_world_commit.as_ref())
        {
            self.require_assignment_identity(assignment)?;
            let prior = frontier.event_sequence == pending.prior_event_sequence
                && frontier.event_hash == pending.prior_event_hash
                && frontier.state_hash == pending.prior_state_hash;
            let successor = frontier.event_sequence
                == pending.prior_event_sequence.checked_add(1).unwrap_or(0)
                && frontier.production_lifecycle_generation
                    == pending.occurrence.lifecycle_generation
                && frontier.committed_production_sequence
                    == pending.occurrence.production_quantum_sequence
                && frontier.last_scheduled_for_unix_ms == pending.occurrence.scheduled_for_unix_ms;
            if !(prior || successor) {
                return Err(invalid(
                    "pending production recovery found a conflicting world frontier",
                ));
            }
        } else {
            self.require_exact_frontiers(assignment, frontier)?;
        }
        if self.authority_operation.is_some()
            || (self.pending_world_commit.is_some()
                && kind != LifecycleAuthorityOperationKindV2::Recovery)
        {
            return Err(invalid("another lifecycle transaction is already pending"));
        }
        let expected_state = match kind {
            LifecycleAuthorityOperationKindV2::Claim => CellAssignmentState::Sleeping,
            LifecycleAuthorityOperationKindV2::Recovery
            | LifecycleAuthorityOperationKindV2::Release => CellAssignmentState::Assigned,
        };
        if assignment.assignment().state != expected_state || holder_id.trim().is_empty() {
            return Err(invalid(
                "authority request does not match the current assignment",
            ));
        }
        if kind == LifecycleAuthorityOperationKindV2::Release
            && assignment.assignment().holder_id.as_deref() != Some(holder_id)
        {
            return Err(invalid("release request holder is not current"));
        }
        let operation_id = format!(
            "lifecycle-{}-{}-{}",
            match kind {
                LifecycleAuthorityOperationKindV2::Claim => "claim",
                LifecycleAuthorityOperationKindV2::Recovery => "recover",
                LifecycleAuthorityOperationKindV2::Release => "release",
            },
            assignment.assignment().assignment_generation,
            holder_id
        );
        if operation_id.len() > 192
            || !operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        {
            return Err(invalid("lifecycle operation identity is not stable"));
        }
        let mut next = self.successor(trusted_now_unix_ms)?;
        next.desired_mode = if kind == LifecycleAuthorityOperationKindV2::Release {
            LifecycleModeV2::Sleeping
        } else {
            LifecycleModeV2::Background
        };
        next.observed_mode = if kind == LifecycleAuthorityOperationKindV2::Release {
            LifecycleModeV2::Draining
        } else {
            self.observed_mode
        };
        next.authority_operation = Some(LifecycleAuthorityOperationV2 {
            kind,
            operation_id,
            expected_assignment_generation: assignment.assignment().assignment_generation,
            holder_id: holder_id.to_owned(),
            requested_at_unix_ms: trusted_now_unix_ms,
        });
        next.seal()?;
        Ok(next)
    }

    pub(crate) fn finalize_assigned(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        trusted_now_unix_ms: u64,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<Self, LifecycleV2Error> {
        let operation = self
            .authority_operation
            .as_ref()
            .ok_or_else(|| invalid("assigned finalization has no pending authority request"))?;
        if !matches!(
            operation.kind,
            LifecycleAuthorityOperationKindV2::Claim | LifecycleAuthorityOperationKindV2::Recovery
        ) || assignment.assignment().state != CellAssignmentState::Assigned
            || assignment.assignment().holder_id.as_deref() != Some(&operation.holder_id)
            || assignment.assignment().assignment_generation
                != operation
                    .expected_assignment_generation
                    .checked_add(1)
                    .ok_or(LifecycleV2Error::Exhausted("assignment generation"))?
        {
            return Err(invalid(
                "directory successor does not match the pending authority request",
            ));
        }
        let exact_prior_frontier = self.require_world_frontier(frontier).is_ok();
        self.require_world_or_pending_successor(frontier)?;
        let mut next = self.successor(trusted_now_unix_ms)?;
        next.observed_mode = LifecycleModeV2::Background;
        next.directory_revision = assignment.directory_revision();
        assignment
            .directory_document_hash()
            .clone_into(&mut next.directory_document_hash);
        next.assignment_generation = assignment.assignment().assignment_generation;
        next.authority_fencing_token = assignment.assignment().authority_fencing_token;
        next.holder_id
            .clone_from(&assignment.assignment().holder_id);
        next.authority_acquired_at_unix_ms = Some(trusted_now_unix_ms);
        next.authority_renewed_at_unix_ms = Some(trusted_now_unix_ms);
        next.authority_expires_at_unix_ms = Some(
            trusted_now_unix_ms
                .checked_add(AUTHORITY_DURATION_MILLIS)
                .ok_or(LifecycleV2Error::Exhausted("authority expiry"))?,
        );
        next.authority_operation = None;
        // Authority recovery may finish after the pending world event already
        // committed. Keep the pending transaction bound to its prior frontier;
        // only acknowledgement may advance lifecycle to that successor.
        if exact_prior_frontier {
            next.install_frontier(frontier);
        }
        next.seal()?;
        Ok(next)
    }

    pub(crate) fn renew_and_set_schedule(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        trusted_now_unix_ms: u64,
        frontier: &LifecycleWorldFrontierV2,
        next_occurrence: Option<ProductionScheduleOccurrence>,
    ) -> Result<Self, LifecycleV2Error> {
        self.require_live_authority(assignment, trusted_now_unix_ms)?;
        self.require_world_frontier(frontier)?;
        if let Some(occurrence) = &next_occurrence {
            validate_occurrence(occurrence, self, frontier)?;
        }
        let mut next = self.successor(trusted_now_unix_ms)?;
        next.directory_revision = assignment.directory_revision();
        assignment
            .directory_document_hash()
            .clone_into(&mut next.directory_document_hash);
        next.authority_renewed_at_unix_ms = Some(trusted_now_unix_ms);
        next.authority_expires_at_unix_ms = Some(
            trusted_now_unix_ms
                .checked_add(AUTHORITY_DURATION_MILLIS)
                .ok_or(LifecycleV2Error::Exhausted("authority expiry"))?,
        );
        next.next_production_occurrence = next_occurrence;
        next.install_frontier(frontier);
        next.seal()?;
        Ok(next)
    }

    pub(crate) fn begin_world_commit(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        trusted_now_unix_ms: u64,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<Self, LifecycleV2Error> {
        self.require_live_authority(assignment, trusted_now_unix_ms)?;
        self.require_world_frontier(frontier)?;
        let occurrence = self
            .next_production_occurrence
            .clone()
            .ok_or_else(|| invalid("production dispatch has no armed occurrence"))?;
        if occurrence.scheduled_for_unix_ms > trusted_now_unix_ms {
            return Err(invalid("production occurrence is not due"));
        }
        if self.pending_world_commit.is_some() {
            return Err(invalid("a production event is already pending"));
        }
        let mut next = self.successor(trusted_now_unix_ms)?;
        next.pending_world_commit = Some(PendingProductionCommitV2 {
            occurrence,
            prior_event_sequence: frontier.event_sequence,
            prior_event_hash: frontier.event_hash.clone(),
            prior_state_hash: frontier.state_hash.clone(),
        });
        next.seal()?;
        Ok(next)
    }

    pub(crate) fn acknowledge_world_commit(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        trusted_now_unix_ms: u64,
        frontier: &LifecycleWorldFrontierV2,
        next_occurrence: Option<ProductionScheduleOccurrence>,
    ) -> Result<Self, LifecycleV2Error> {
        self.require_live_authority(assignment, trusted_now_unix_ms)?;
        let pending = self
            .pending_world_commit
            .as_ref()
            .ok_or_else(|| invalid("production acknowledgement has no pending event"))?;
        if frontier.event_sequence != pending.prior_event_sequence.checked_add(1).unwrap_or(0)
            || frontier.committed_production_sequence
                != pending.occurrence.production_quantum_sequence
            || frontier.last_scheduled_for_unix_ms != pending.occurrence.scheduled_for_unix_ms
        {
            return Err(invalid(
                "world frontier is not the exact pending production successor",
            ));
        }
        let mut next = self.successor(trusted_now_unix_ms)?;
        next.pending_world_commit = None;
        next.acknowledged_production_sequence = frontier.committed_production_sequence;
        next.next_production_occurrence = next_occurrence;
        next.install_frontier(frontier);
        next.seal()?;
        Ok(next)
    }

    pub(crate) fn finalize_sleeping(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        trusted_now_unix_ms: u64,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<Self, LifecycleV2Error> {
        let operation = self
            .authority_operation
            .as_ref()
            .ok_or_else(|| invalid("sleeping finalization has no pending release"))?;
        if operation.kind != LifecycleAuthorityOperationKindV2::Release
            || assignment.assignment().state != CellAssignmentState::Sleeping
            || assignment.assignment().holder_id.is_some()
            || assignment.assignment().assignment_generation
                != operation.expected_assignment_generation
        {
            return Err(invalid(
                "sleeping directory tip does not match the pending release",
            ));
        }
        self.require_world_frontier(frontier)?;
        let mut next = self.successor(trusted_now_unix_ms)?;
        next.observed_mode = LifecycleModeV2::Sleeping;
        next.directory_revision = assignment.directory_revision();
        assignment
            .directory_document_hash()
            .clone_into(&mut next.directory_document_hash);
        next.assignment_generation = assignment.assignment().assignment_generation;
        next.authority_fencing_token = assignment.assignment().authority_fencing_token;
        next.holder_id = None;
        next.authority_acquired_at_unix_ms = None;
        next.authority_renewed_at_unix_ms = None;
        next.authority_expires_at_unix_ms = None;
        next.authority_operation = None;
        next.pending_world_commit = None;
        next.install_frontier(frontier);
        next.seal()?;
        Ok(next)
    }

    fn require_exact_frontiers(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<(), LifecycleV2Error> {
        self.require_world_frontier(frontier)?;
        let record = assignment.assignment();
        if assignment.universe_id() != self.universe_id
            || assignment.universe_manifest_hash() != self.manifest_hash
            || record.cell_key != self.cell_key
            || record.cell_id != self.cell_id
            || record.assignment_generation != self.assignment_generation
            || record.authority_fencing_token != self.authority_fencing_token
        {
            return Err(invalid("lifecycle and directory frontiers differ"));
        }
        Ok(())
    }

    fn require_world_frontier(
        &self,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<(), LifecycleV2Error> {
        if frontier.event_sequence != self.last_world_event_sequence
            || frontier.event_hash != self.last_world_event_hash
            || frontier.state_hash != self.last_world_state_hash
            || frontier.active_world_hash != self.last_active_world_hash
            || frontier.celestial_registry_hash != self.celestial_registry_hash
            || frontier.production_lifecycle_generation != self.production_lifecycle_generation
            || frontier.committed_production_sequence != self.acknowledged_production_sequence
            || frontier.last_scheduled_for_unix_ms
                != self.last_committed_production_scheduled_for_unix_ms
        {
            return Err(invalid("lifecycle and world frontiers differ"));
        }
        Ok(())
    }

    fn require_world_or_pending_successor(
        &self,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<(), LifecycleV2Error> {
        if self.require_world_frontier(frontier).is_ok() {
            return Ok(());
        }
        let pending = self.pending_world_commit.as_ref().ok_or_else(|| {
            invalid("lifecycle and world frontiers differ without a pending commit")
        })?;
        if frontier.event_sequence != pending.prior_event_sequence.checked_add(1).unwrap_or(0)
            || frontier.production_lifecycle_generation != pending.occurrence.lifecycle_generation
            || frontier.celestial_registry_hash != self.celestial_registry_hash
            || frontier.committed_production_sequence
                != pending.occurrence.production_quantum_sequence
            || frontier.last_scheduled_for_unix_ms != pending.occurrence.scheduled_for_unix_ms
        {
            return Err(invalid(
                "lifecycle pending commit has a conflicting world successor",
            ));
        }
        Ok(())
    }

    fn require_live_authority(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        trusted_now_unix_ms: u64,
    ) -> Result<(), LifecycleV2Error> {
        self.require_assignment_identity(assignment)?;
        if self.observed_mode != LifecycleModeV2::Background
            || self.holder_id.as_deref() != assignment.assignment().holder_id.as_deref()
            || self.authority_operation.is_some()
            || self
                .authority_expires_at_unix_ms
                .is_none_or(|expiry| trusted_now_unix_ms >= expiry)
        {
            return Err(invalid("lifecycle authority is not live"));
        }
        Ok(())
    }

    pub(crate) fn require_event_append_authority(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        trusted_now_unix_ms: u64,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<(), LifecycleV2Error> {
        require_monotonic_trusted_time(trusted_now_unix_ms, self.last_trusted_unix_ms)?;
        self.require_live_authority(assignment, trusted_now_unix_ms)?;
        require_event_append_lease_margin(
            trusted_now_unix_ms,
            self.authority_expires_at_unix_ms
                .expect("live lifecycle authority has an expiry"),
        )?;
        self.require_world_frontier(frontier)?;
        if self.pending_world_commit.is_none() {
            return Err(invalid(
                "production event append has no pending lifecycle commit",
            ));
        }
        Ok(())
    }

    pub(crate) fn preflight_authority_finalization(
        &self,
        trusted_now_unix_ms: u64,
    ) -> Result<(), LifecycleV2Error> {
        let operation = self
            .authority_operation
            .as_ref()
            .ok_or_else(|| invalid("authority finalization has no pending operation"))?;
        self.successor(trusted_now_unix_ms)?;
        if matches!(
            operation.kind,
            LifecycleAuthorityOperationKindV2::Claim | LifecycleAuthorityOperationKindV2::Recovery
        ) {
            trusted_now_unix_ms
                .checked_add(AUTHORITY_DURATION_MILLIS)
                .ok_or(LifecycleV2Error::Exhausted("authority expiry"))?;
        }
        Ok(())
    }

    fn require_assignment_identity(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
    ) -> Result<(), LifecycleV2Error> {
        let record = assignment.assignment();
        if record.state != CellAssignmentState::Assigned
            || assignment.universe_id() != self.universe_id
            || assignment.universe_manifest_hash() != self.manifest_hash
            || record.cell_key != self.cell_key
            || record.cell_id != self.cell_id
            || record.assignment_generation != self.assignment_generation
            || record.authority_fencing_token != self.authority_fencing_token
        {
            return Err(invalid(
                "current assignment does not match lifecycle authority",
            ));
        }
        Ok(())
    }

    fn install_frontier(&mut self, frontier: &LifecycleWorldFrontierV2) {
        self.last_world_event_sequence = frontier.event_sequence;
        self.last_world_event_hash.clone_from(&frontier.event_hash);
        self.last_world_state_hash.clone_from(&frontier.state_hash);
        self.last_active_world_hash
            .clone_from(&frontier.active_world_hash);
        self.production_lifecycle_generation = frontier.production_lifecycle_generation;
        self.last_committed_production_scheduled_for_unix_ms = frontier.last_scheduled_for_unix_ms;
    }

    fn world_frontier(&self) -> LifecycleWorldFrontierV2 {
        LifecycleWorldFrontierV2 {
            event_sequence: self.last_world_event_sequence,
            event_hash: self.last_world_event_hash.clone(),
            state_hash: self.last_world_state_hash.clone(),
            active_world_hash: self.last_active_world_hash.clone(),
            celestial_registry_hash: self.celestial_registry_hash.clone(),
            production_lifecycle_generation: self.production_lifecycle_generation,
            committed_production_sequence: self.acknowledged_production_sequence,
            last_scheduled_for_unix_ms: self.last_committed_production_scheduled_for_unix_ms,
        }
    }

    fn validate_directory_binding(
        &self,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<(), LifecycleV2Error> {
        let assignment = directory_history
            .historical_cell_assignment(
                self.directory_revision,
                &self.directory_document_hash,
                &self.cell_id,
            )
            .map_err(|source| invalid(source.to_string()))?;
        let record = assignment.assignment();
        let assigned_binding =
            record.state == CellAssignmentState::Assigned && record.holder_id == self.holder_id;
        let sleeping_binding = record.state == CellAssignmentState::Sleeping
            && record.holder_id.is_none()
            && self.holder_id.is_none();
        if assignment.universe_id() != self.universe_id
            || assignment.universe_manifest_hash() != self.manifest_hash
            || record.cell_key != self.cell_key
            || record.cell_id != self.cell_id
            || record.assignment_generation != self.assignment_generation
            || record.authority_fencing_token != self.authority_fencing_token
            || !(assigned_binding || sleeping_binding)
        {
            return Err(invalid(
                "lifecycle record is not authenticated by its historical directory assignment",
            ));
        }
        Ok(())
    }

    fn validate_exact_successor(
        &self,
        prior: &Self,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<(), LifecycleV2Error> {
        let assignment = directory_history
            .historical_cell_assignment(
                self.directory_revision,
                &self.directory_document_hash,
                &self.cell_id,
            )
            .map_err(|source| invalid(source.to_string()))?;
        let prior_frontier = prior.world_frontier();
        let expected = match (&prior.authority_operation, &self.authority_operation) {
            (None, Some(operation)) => prior.request_authority(
                &assignment,
                operation.kind,
                &operation.holder_id,
                self.last_trusted_unix_ms,
                &prior_frontier,
            )?,
            (Some(operation), None) => match operation.kind {
                LifecycleAuthorityOperationKindV2::Claim
                | LifecycleAuthorityOperationKindV2::Recovery => prior.finalize_assigned(
                    &assignment,
                    self.last_trusted_unix_ms,
                    &prior_frontier,
                )?,
                LifecycleAuthorityOperationKindV2::Release => prior.finalize_sleeping(
                    &assignment,
                    self.last_trusted_unix_ms,
                    &prior_frontier,
                )?,
            },
            (None, None) => match (
                prior.pending_world_commit.as_ref(),
                self.pending_world_commit.as_ref(),
            ) {
                (None, Some(_)) => prior.begin_world_commit(
                    &assignment,
                    self.last_trusted_unix_ms,
                    &prior_frontier,
                )?,
                (Some(_), None) => prior.acknowledge_world_commit(
                    &assignment,
                    self.last_trusted_unix_ms,
                    &self.world_frontier(),
                    self.next_production_occurrence.clone(),
                )?,
                (None, None) => prior.renew_and_set_schedule(
                    &assignment,
                    self.last_trusted_unix_ms,
                    &prior_frontier,
                    self.next_production_occurrence.clone(),
                )?,
                (Some(_), Some(_)) => {
                    return Err(invalid(
                        "pending production transaction cannot change without acknowledgement",
                    ));
                }
            },
            (Some(_), Some(_)) => {
                return Err(invalid(
                    "pending authority transaction cannot be replaced in place",
                ));
            }
        };
        if *self != expected {
            return Err(invalid(
                "lifecycle history contains a successor not produced by the state machine",
            ));
        }
        Ok(())
    }

    fn seal(&mut self) -> Result<(), LifecycleV2Error> {
        self.record_hash.clear();
        self.record_hash = self.calculate_hash()?;
        self.validate()
    }

    fn calculate_hash(&self) -> Result<String, LifecycleV2Error> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_json(LIFECYCLE_RECORD_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), LifecycleV2Error> {
        let live = self.observed_mode != LifecycleModeV2::Sleeping;
        let authority_times = self.authority_acquired_at_unix_ms.is_some()
            && self.authority_renewed_at_unix_ms.is_some()
            && self.authority_expires_at_unix_ms.is_some();
        let empty_event =
            self.last_world_event_sequence == 0 && self.last_world_event_hash.is_empty();
        let populated_event =
            self.last_world_event_sequence > 0 && valid_hash(&self.last_world_event_hash);
        if self.schema_version != LIFECYCLE_RECORD_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.universe_id.trim().is_empty()
            || self.cell_key.universe_id != self.universe_id
            || !valid_hash(&self.cell_id)
            || !valid_hash(&self.manifest_hash)
            || !valid_hash(&self.celestial_registry_hash)
            || !valid_hash(&self.migration_anchor_hash)
            || !valid_hash(&self.lifecycle_genesis_hash)
            || !valid_hash(&self.active_head_hash)
            || self.lifecycle_revision < 2
            || self.directory_revision == 0
            || !valid_hash(&self.directory_document_hash)
            || self.assignment_generation == 0
            || self.authority_fencing_token == 0
            || self.last_trusted_unix_ms == 0
            || !(empty_event || populated_event)
            || !valid_hash(&self.last_world_state_hash)
            || !valid_hash(&self.last_active_world_hash)
            || self.production_lifecycle_generation == 0
            || self.acknowledged_production_sequence > self.last_world_event_sequence
            || !valid_hash(&self.previous_record_hash)
            || !valid_hash(&self.record_hash)
            || live != self.holder_id.is_some()
            || live != authority_times
            || self.authority_expires_at_unix_ms.is_some_and(|expiry| {
                expiry <= self.authority_renewed_at_unix_ms.unwrap_or(u64::MAX)
            })
            || self.authority_renewed_at_unix_ms.is_some_and(|renewed| {
                renewed < self.authority_acquired_at_unix_ms.unwrap_or(renewed)
                    || renewed > self.last_trusted_unix_ms
            })
            || self.pending_world_commit.is_some() && self.next_production_occurrence.is_none()
            || self.record_hash != self.calculate_hash()?
        {
            return Err(invalid("lifecycle record is not canonical"));
        }
        if let Some(occurrence) = &self.next_production_occurrence {
            validate_occurrence(
                occurrence,
                self,
                &LifecycleWorldFrontierV2 {
                    event_sequence: self.last_world_event_sequence,
                    event_hash: self.last_world_event_hash.clone(),
                    state_hash: self.last_world_state_hash.clone(),
                    active_world_hash: self.last_active_world_hash.clone(),
                    celestial_registry_hash: self.celestial_registry_hash.clone(),
                    production_lifecycle_generation: self.production_lifecycle_generation,
                    committed_production_sequence: self.acknowledged_production_sequence,
                    last_scheduled_for_unix_ms: self
                        .last_committed_production_scheduled_for_unix_ms,
                },
            )?;
        }
        if let Some(pending) = &self.pending_world_commit
            && (self.next_production_occurrence.as_ref() != Some(&pending.occurrence)
                || pending.prior_event_sequence != self.last_world_event_sequence
                || pending.prior_event_hash != self.last_world_event_hash
                || pending.prior_state_hash != self.last_world_state_hash)
        {
            return Err(invalid(
                "pending production commit does not bind the lifecycle frontier",
            ));
        }
        let lifecycle_shape_is_valid = match (&self.authority_operation, self.observed_mode) {
            (None, LifecycleModeV2::Sleeping) => self.desired_mode == LifecycleModeV2::Sleeping,
            (None, LifecycleModeV2::Background) => self.desired_mode == LifecycleModeV2::Background,
            (Some(operation), LifecycleModeV2::Sleeping) => {
                operation.kind == LifecycleAuthorityOperationKindV2::Claim
                    && self.desired_mode == LifecycleModeV2::Background
                    && operation.expected_assignment_generation == self.assignment_generation
            }
            (Some(operation), LifecycleModeV2::Background) => {
                operation.kind == LifecycleAuthorityOperationKindV2::Recovery
                    && self.desired_mode == LifecycleModeV2::Background
                    && operation.expected_assignment_generation == self.assignment_generation
            }
            (Some(operation), LifecycleModeV2::Draining) => {
                operation.kind == LifecycleAuthorityOperationKindV2::Release
                    && self.desired_mode == LifecycleModeV2::Sleeping
                    && operation.expected_assignment_generation == self.assignment_generation
                    && self.holder_id.as_deref() == Some(operation.holder_id.as_str())
            }
            _ => false,
        };
        if !lifecycle_shape_is_valid {
            return Err(invalid("lifecycle mode and authority operation disagree"));
        }
        if let Some(operation) = &self.authority_operation {
            let expected_operation_id = format!(
                "lifecycle-{}-{}-{}",
                match operation.kind {
                    LifecycleAuthorityOperationKindV2::Claim => "claim",
                    LifecycleAuthorityOperationKindV2::Recovery => "recover",
                    LifecycleAuthorityOperationKindV2::Release => "release",
                },
                operation.expected_assignment_generation,
                operation.holder_id
            );
            if operation.operation_id != expected_operation_id
                || operation.requested_at_unix_ms != self.last_trusted_unix_ms
                || operation.holder_id.trim().is_empty()
            {
                return Err(invalid("lifecycle authority operation is not canonical"));
            }
        }
        Ok(())
    }

    pub(crate) fn observed_mode(&self) -> LifecycleModeV2 {
        self.observed_mode
    }

    pub(crate) fn authority_operation(&self) -> Option<&LifecycleAuthorityOperationV2> {
        self.authority_operation.as_ref()
    }

    pub(crate) fn pending_world_commit(&self) -> Option<&PendingProductionCommitV2> {
        self.pending_world_commit.as_ref()
    }

    pub(crate) fn next_occurrence(&self) -> Option<&ProductionScheduleOccurrence> {
        self.next_production_occurrence.as_ref()
    }

    pub(crate) fn holder_id(&self) -> Option<&str> {
        self.holder_id.as_deref()
    }

    pub(crate) fn authority_expires_at_unix_ms(&self) -> Option<u64> {
        self.authority_expires_at_unix_ms
    }

    pub(crate) fn acknowledged_sequence(&self) -> u64 {
        self.acknowledged_production_sequence
    }

    fn validate_recovery_view(
        &self,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        frontier: &LifecycleWorldFrontierV2,
    ) -> Result<(), LifecycleV2Error> {
        let directory = assignment.assignment();
        if assignment.universe_id() != self.universe_id
            || assignment.universe_manifest_hash() != self.manifest_hash
            || directory.cell_key != self.cell_key
            || directory.cell_id != self.cell_id
        {
            return Err(invalid(
                "recovered lifecycle belongs to another directory assignment",
            ));
        }
        self.require_world_or_pending_successor(frontier)?;
        match self.authority_operation.as_ref() {
            None if self.observed_mode == LifecycleModeV2::Sleeping => {
                if directory.state != CellAssignmentState::Sleeping
                    || directory.holder_id.is_some()
                    || directory.assignment_generation != self.assignment_generation
                    || directory.authority_fencing_token != self.authority_fencing_token
                {
                    return Err(invalid(
                        "sleeping lifecycle does not match the sleeping directory tip",
                    ));
                }
            }
            None => {
                self.require_assignment_identity(assignment)?;
                if directory.holder_id.as_deref() != self.holder_id.as_deref() {
                    return Err(invalid(
                        "assigned lifecycle holder differs from the directory tip",
                    ));
                }
            }
            Some(operation)
                if matches!(
                    operation.kind,
                    LifecycleAuthorityOperationKindV2::Claim
                        | LifecycleAuthorityOperationKindV2::Recovery
                ) =>
            {
                let predecessor_state =
                    if operation.kind == LifecycleAuthorityOperationKindV2::Claim {
                        CellAssignmentState::Sleeping
                    } else {
                        CellAssignmentState::Assigned
                    };
                let is_predecessor = directory.state == predecessor_state
                    && directory.assignment_generation == operation.expected_assignment_generation
                    && directory.authority_fencing_token == self.authority_fencing_token;
                let is_successor = directory.state == CellAssignmentState::Assigned
                    && directory.assignment_generation
                        == operation
                            .expected_assignment_generation
                            .checked_add(1)
                            .unwrap_or(0)
                    && directory.authority_fencing_token
                        == self.authority_fencing_token.checked_add(1).unwrap_or(0)
                    && directory.holder_id.as_deref() == Some(&operation.holder_id);
                if !(is_predecessor || is_successor) {
                    return Err(invalid(
                        "pending authority request has no exact directory frontier",
                    ));
                }
            }
            Some(operation) => {
                let is_predecessor = directory.state == CellAssignmentState::Assigned
                    && directory.assignment_generation == operation.expected_assignment_generation
                    && directory.authority_fencing_token == self.authority_fencing_token
                    && directory.holder_id.as_deref() == Some(&operation.holder_id);
                let is_successor = directory.state == CellAssignmentState::Sleeping
                    && directory.assignment_generation == operation.expected_assignment_generation
                    && directory.authority_fencing_token == self.authority_fencing_token
                    && directory.holder_id.is_none();
                if !(is_predecessor || is_successor) {
                    return Err(invalid("pending release has no exact directory frontier"));
                }
            }
        }
        Ok(())
    }
}

fn require_event_append_lease_margin(
    trusted_now_unix_ms: u64,
    authority_expires_at_unix_ms: u64,
) -> Result<(), LifecycleV2Error> {
    let safe_through = trusted_now_unix_ms
        .checked_add(EVENT_APPEND_LEASE_MARGIN_MILLIS)
        .ok_or(LifecycleV2Error::Exhausted(
            "event append lease safety margin",
        ))?;
    if safe_through > authority_expires_at_unix_ms {
        return Err(invalid(
            "lifecycle authority lacks the event append lease safety margin",
        ));
    }
    Ok(())
}

fn require_monotonic_trusted_time(
    trusted_now_unix_ms: u64,
    last_trusted_unix_ms: u64,
) -> Result<(), LifecycleV2Error> {
    if trusted_now_unix_ms < last_trusted_unix_ms {
        return Err(invalid("trusted lifecycle time moved backward"));
    }
    Ok(())
}

fn validate_occurrence(
    occurrence: &ProductionScheduleOccurrence,
    lifecycle: &LifecycleRecordV2,
    frontier: &LifecycleWorldFrontierV2,
) -> Result<(), LifecycleV2Error> {
    if occurrence.schema_version != PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
        || occurrence.universe_id != lifecycle.universe_id
        || occurrence.cell_id != lifecycle.cell_id
        || occurrence.lifecycle_generation != frontier.production_lifecycle_generation
        || occurrence.production_quantum_sequence
            != frontier
                .committed_production_sequence
                .checked_add(1)
                .ok_or(LifecycleV2Error::Exhausted("production sequence"))?
        || occurrence.scheduled_for_unix_ms == 0
        || occurrence.universe_manifest_hash != lifecycle.manifest_hash
        || occurrence.celestial_registry_hash != lifecycle.celestial_registry_hash
        || (lifecycle.next_production_occurrence.is_some()
            && frontier.committed_production_sequence > 0
            && occurrence.scheduled_for_unix_ms
                != frontier
                    .last_scheduled_for_unix_ms
                    .checked_add(PRODUCTION_QUANTUM_MILLIS)
                    .ok_or(LifecycleV2Error::Exhausted("production schedule time"))?)
    {
        return Err(invalid(
            "production occurrence is not the exact next cursor",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleHistoryEntryV2 {
    schema_version: u32,
    record: LifecycleRecordV2,
    previous_entry_hash: String,
    entry_hash: String,
}

impl LifecycleHistoryEntryV2 {
    fn new(
        record: LifecycleRecordV2,
        previous_entry_hash: String,
    ) -> Result<Self, LifecycleV2Error> {
        let mut entry = Self {
            schema_version: LIFECYCLE_HISTORY_ENTRY_SCHEMA_VERSION,
            record,
            previous_entry_hash,
            entry_hash: String::new(),
        };
        entry.entry_hash = entry.calculate_hash()?;
        entry.validate()?;
        Ok(entry)
    }

    fn calculate_hash(&self) -> Result<String, LifecycleV2Error> {
        let mut material = self.clone();
        material.entry_hash.clear();
        hash_json(LIFECYCLE_HISTORY_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), LifecycleV2Error> {
        self.record.validate()?;
        if self.schema_version != LIFECYCLE_HISTORY_ENTRY_SCHEMA_VERSION
            || (!self.previous_entry_hash.is_empty() && !valid_hash(&self.previous_entry_hash))
            || !valid_hash(&self.entry_hash)
            || self.entry_hash != self.calculate_hash()?
        {
            return Err(invalid("lifecycle history entry is invalid"));
        }
        Ok(())
    }

    fn canonical_line(&self) -> Result<Vec<u8>, LifecycleV2Error> {
        let mut bytes = serde_json::to_vec(self).map_err(|source| invalid(source.to_string()))?;
        if bytes.len() >= MAX_LIFECYCLE_RECORD_BYTES {
            return Err(invalid("lifecycle record exceeds its size bound"));
        }
        bytes.push(b'\n');
        Ok(bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LifecycleHeadV2 {
    schema_version: u32,
    lifecycle_genesis_hash: String,
    active_head_hash: String,
    entry_count: u64,
    journal_bytes: u64,
    last_entry_hash: String,
    last_record_hash: String,
    head_hash: String,
}

impl LifecycleHeadV2 {
    fn empty(genesis_hash: &str, active_head_hash: &str) -> Result<Self, LifecycleV2Error> {
        let mut head = Self {
            schema_version: LIFECYCLE_HEAD_SCHEMA_VERSION,
            lifecycle_genesis_hash: genesis_hash.to_owned(),
            active_head_hash: active_head_hash.to_owned(),
            entry_count: 0,
            journal_bytes: 0,
            last_entry_hash: String::new(),
            last_record_hash: genesis_hash.to_owned(),
            head_hash: String::new(),
        };
        head.seal()?;
        Ok(head)
    }

    fn successor(
        &self,
        entry: &LifecycleHistoryEntryV2,
        line_len: usize,
    ) -> Result<Self, LifecycleV2Error> {
        let mut head = Self {
            schema_version: LIFECYCLE_HEAD_SCHEMA_VERSION,
            lifecycle_genesis_hash: self.lifecycle_genesis_hash.clone(),
            active_head_hash: self.active_head_hash.clone(),
            entry_count: self
                .entry_count
                .checked_add(1)
                .ok_or(LifecycleV2Error::Exhausted("history count"))?,
            journal_bytes: self
                .journal_bytes
                .checked_add(
                    u64::try_from(line_len)
                        .map_err(|_| LifecycleV2Error::Exhausted("history length"))?,
                )
                .ok_or(LifecycleV2Error::Exhausted("history length"))?,
            last_entry_hash: entry.entry_hash.clone(),
            last_record_hash: entry.record.record_hash.clone(),
            head_hash: String::new(),
        };
        head.seal()?;
        Ok(head)
    }

    fn seal(&mut self) -> Result<(), LifecycleV2Error> {
        self.head_hash.clear();
        self.head_hash = self.calculate_hash()?;
        self.validate()
    }

    fn calculate_hash(&self) -> Result<String, LifecycleV2Error> {
        let mut material = self.clone();
        material.head_hash.clear();
        hash_json(LIFECYCLE_HEAD_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), LifecycleV2Error> {
        let empty = self.entry_count == 0
            && self.journal_bytes == 0
            && self.last_entry_hash.is_empty()
            && self.last_record_hash == self.lifecycle_genesis_hash;
        let populated = self.entry_count > 0
            && self.journal_bytes > 0
            && valid_hash(&self.last_entry_hash)
            && valid_hash(&self.last_record_hash);
        if self.schema_version != LIFECYCLE_HEAD_SCHEMA_VERSION
            || !valid_hash(&self.lifecycle_genesis_hash)
            || !valid_hash(&self.active_head_hash)
            || !(empty || populated)
            || !valid_hash(&self.head_hash)
            || self.head_hash != self.calculate_hash()?
        {
            return Err(invalid("lifecycle head is invalid"));
        }
        Ok(())
    }
}

impl From<&LifecycleHeadV2> for LifecycleHeadCommitmentV2 {
    fn from(head: &LifecycleHeadV2) -> Self {
        Self {
            lifecycle_genesis_hash: head.lifecycle_genesis_hash.clone(),
            active_head_hash: head.active_head_hash.clone(),
            entry_count: head.entry_count,
            journal_bytes: head.journal_bytes,
            last_entry_hash: head.last_entry_hash.clone(),
            last_record_hash: head.last_record_hash.clone(),
            head_hash: head.head_hash.clone(),
        }
    }
}

#[derive(Debug)]
pub(crate) struct LifecycleStoreV2 {
    root: PathBuf,
    head: LifecycleHeadV2,
    current: Option<LifecycleRecordV2>,
    poisoned: bool,
    #[cfg(test)]
    failpoint: Option<LifecycleAppendFailpointV2>,
}

impl LifecycleStoreV2 {
    pub(crate) fn expected_initial_record(
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        initial_frontier: &LifecycleWorldFrontierV2,
    ) -> Result<LifecycleRecordV2, LifecycleV2Error> {
        LifecycleRecordV2::initial(genesis, active_head_hash, assignment, initial_frontier)
    }

    pub(crate) fn prepare_initial_append(
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        initial_frontier: &LifecycleWorldFrontierV2,
    ) -> Result<PreparedLifecycleAppendV2, LifecycleV2Error> {
        let head = LifecycleHeadV2::empty(genesis.record_hash(), active_head_hash)?;
        let record =
            LifecycleRecordV2::initial(genesis, active_head_hash, assignment, initial_frontier)?;
        let entry = LifecycleHistoryEntryV2::new(record.clone(), String::new())?;
        let line = entry.canonical_line()?;
        let next_head = head.successor(&entry, line.len())?;
        Ok(PreparedLifecycleAppendV2 {
            record,
            entry,
            line,
            next_head,
        })
    }

    pub(crate) fn initialize_prepared(
        root: impl AsRef<Path>,
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        prepared: PreparedLifecycleAppendV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<Self, LifecycleV2Error> {
        Self::initialize_prepared_inner(
            root,
            genesis,
            active_head_hash,
            prepared,
            directory_history,
            None,
        )
    }

    pub(crate) fn initialize_prepared_with_failpoint(
        root: impl AsRef<Path>,
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        prepared: PreparedLifecycleAppendV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
        failpoint: LifecycleInitializationFailpointV2,
    ) -> Result<Self, LifecycleV2Error> {
        Self::initialize_prepared_inner(
            root,
            genesis,
            active_head_hash,
            prepared,
            directory_history,
            Some(failpoint),
        )
    }

    fn initialize_prepared_inner(
        root: impl AsRef<Path>,
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        prepared: PreparedLifecycleAppendV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
        failpoint: Option<LifecycleInitializationFailpointV2>,
    ) -> Result<Self, LifecycleV2Error> {
        let root = root.as_ref().to_path_buf();
        let history_path = root.join(LIFECYCLE_HISTORY_FILE);
        let head_path = root.join(LIFECYCLE_HEAD_FILE);
        let empty = LifecycleHeadV2::empty(genesis.record_hash(), active_head_hash)?;
        let history_exists = regular_file_exists(&history_path)?;
        let head_exists = regular_file_exists(&head_path)?;
        let initial_frontier = prepared.record.world_frontier();
        let mut store = match (history_exists, head_exists) {
            (false, false) => {
                create_synced_empty(&history_path)?;
                if failpoint == Some(LifecycleInitializationFailpointV2::EmptyHistoryCreated) {
                    return Err(invalid(
                        "injected failure after empty lifecycle history creation",
                    ));
                }
                atomic_write(&head_path, &encode_canonical(&empty)?)?;
                sync_directory(&root)?;
                if failpoint == Some(LifecycleInitializationFailpointV2::EmptyHeadCommitted) {
                    return Err(invalid(
                        "injected failure after empty lifecycle head commit",
                    ));
                }
                Self {
                    root,
                    head: empty,
                    current: None,
                    poisoned: false,
                    #[cfg(test)]
                    failpoint: None,
                }
            }
            (true, false) => {
                if fs::metadata(&history_path)
                    .map_err(|source| io(&history_path, source))?
                    .len()
                    != 0
                {
                    return Err(invalid(
                        "lifecycle bootstrap history exists without its empty head",
                    ));
                }
                atomic_write(&head_path, &encode_canonical(&empty)?)?;
                sync_directory(&root)?;
                if failpoint == Some(LifecycleInitializationFailpointV2::EmptyHeadCommitted) {
                    return Err(invalid(
                        "injected failure after empty lifecycle head commit",
                    ));
                }
                Self {
                    root,
                    head: empty,
                    current: None,
                    poisoned: false,
                    #[cfg(test)]
                    failpoint: None,
                }
            }
            (true, true) => {
                let recovered = Self::recover(
                    &root,
                    genesis,
                    active_head_hash,
                    &initial_frontier,
                    directory_history,
                    true,
                )?;
                if recovered.current.is_some() {
                    if recovered.commitment() == prepared.next_commitment() {
                        return Ok(recovered);
                    }
                    return Err(invalid(
                        "existing lifecycle bootstrap differs from its authorization",
                    ));
                }
                if recovered.head != empty {
                    return Err(invalid(
                        "empty lifecycle bootstrap head differs from immutable genesis",
                    ));
                }
                recovered
            }
            (false, true) => {
                return Err(invalid(
                    "lifecycle bootstrap head exists without its history",
                ));
            }
        };
        let checked = store.prepare_append(prepared.record.clone(), directory_history)?;
        if checked.next_commitment() != prepared.next_commitment() {
            return Err(invalid(
                "authorized initial lifecycle append is not deterministic",
            ));
        }
        #[cfg(test)]
        match failpoint {
            Some(LifecycleInitializationFailpointV2::InitialJournalSyncedBeforeHead) => {
                store.failpoint = Some(LifecycleAppendFailpointV2::JournalSyncedBeforeHead);
            }
            Some(LifecycleInitializationFailpointV2::InitialHeadRenamedBeforeMemory) => {
                store.failpoint = Some(LifecycleAppendFailpointV2::HeadRenamedBeforeMemory);
            }
            _ => {}
        }
        #[cfg(not(test))]
        let _ = failpoint;
        store.materialize_prepared(prepared, directory_history)?;
        Ok(store)
    }

    pub(crate) fn commitment(&self) -> LifecycleHeadCommitmentV2 {
        LifecycleHeadCommitmentV2::from(&self.head)
    }

    pub(crate) fn open_or_initialize(
        root: impl AsRef<Path>,
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        initial_frontier: &LifecycleWorldFrontierV2,
        frontier: &LifecycleWorldFrontierV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<Self, LifecycleV2Error> {
        let root = root.as_ref().to_path_buf();
        let history_path = root.join(LIFECYCLE_HISTORY_FILE);
        let head_path = root.join(LIFECYCLE_HEAD_FILE);
        let history_exists = regular_file_exists(&history_path)?;
        let head_exists = regular_file_exists(&head_path)?;
        if head_exists != history_exists {
            return Err(invalid("lifecycle runtime artifact set is incomplete"));
        }
        if !history_exists {
            return Err(invalid(
                "lifecycle runtime has not been authorized by the universe coordinator",
            ));
        }
        let store = Self::recover(
            &root,
            genesis,
            active_head_hash,
            initial_frontier,
            directory_history,
            true,
        )?;
        if store.head.entry_count == 0 {
            return Err(invalid("authorized lifecycle runtime is empty"));
        }
        store.validate_identity(genesis, active_head_hash)?;
        store
            .current()
            .validate_recovery_view(assignment, frontier)?;
        Ok(store)
    }

    fn recover(
        root: &Path,
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        initial_frontier: &LifecycleWorldFrontierV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
        repair: bool,
    ) -> Result<Self, LifecycleV2Error> {
        let head_path = root.join(LIFECYCLE_HEAD_FILE);
        let head_bytes = read_bounded(&head_path, MAX_LIFECYCLE_HEAD_BYTES)?;
        let mut head: LifecycleHeadV2 = serde_json::from_slice(&head_bytes)
            .map_err(|source| invalid(format!("lifecycle head JSON is invalid: {source}")))?;
        if encode_canonical(&head)? != head_bytes {
            return Err(invalid("lifecycle head bytes are not canonical"));
        }
        head.validate()?;
        if head.lifecycle_genesis_hash != genesis.record_hash()
            || head.active_head_hash != active_head_hash
        {
            return Err(invalid("lifecycle head belongs to another activated world"));
        }
        let history_path = root.join(LIFECYCLE_HISTORY_FILE);
        let metadata = fs::metadata(&history_path).map_err(|source| io(&history_path, source))?;
        if metadata.len() > MAX_LIFECYCLE_HISTORY_BYTES {
            return Err(invalid("lifecycle history exceeds its size bound"));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        File::open(&history_path)
            .and_then(|mut file| file.read_to_end(&mut bytes))
            .map_err(|source| io(&history_path, source))?;
        let complete_len = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let mut entries = Vec::new();
        for line in bytes[..complete_len].split_inclusive(|byte| *byte == b'\n') {
            if line.len() > MAX_LIFECYCLE_RECORD_BYTES {
                return Err(invalid("lifecycle history line exceeds its size bound"));
            }
            let entry: LifecycleHistoryEntryV2 = serde_json::from_slice(&line[..line.len() - 1])
                .map_err(|source| {
                    invalid(format!("lifecycle history JSON is invalid: {source}"))
                })?;
            if entry.canonical_line()? != line {
                return Err(invalid("lifecycle history bytes are not canonical"));
            }
            entry.validate()?;
            let expected_previous_entry = entries
                .last()
                .map_or("", |prior: &LifecycleHistoryEntryV2| {
                    prior.entry_hash.as_str()
                });
            let expected_previous_record = entries.last().map_or(genesis.record_hash(), |prior| {
                prior.record.record_hash.as_str()
            });
            let expected_revision = u64::try_from(entries.len())
                .ok()
                .and_then(|count| count.checked_add(2))
                .ok_or(LifecycleV2Error::Exhausted("lifecycle revision"))?;
            if entry.previous_entry_hash != expected_previous_entry
                || entry.record.previous_record_hash != expected_previous_record
                || entry.record.lifecycle_revision != expected_revision
            {
                return Err(invalid("lifecycle history chain is discontinuous"));
            }
            entries.push(entry);
        }
        if let Some(first) = entries.first() {
            let assignment = directory_history
                .historical_cell_assignment(
                    genesis.target_directory_revision(),
                    genesis.target_directory_document_hash(),
                    genesis.cell_id(),
                )
                .map_err(|source| invalid(source.to_string()))?;
            let expected = LifecycleRecordV2::initial(
                genesis,
                active_head_hash,
                &assignment,
                initial_frontier,
            )?;
            if first.record != expected {
                return Err(invalid(
                    "lifecycle history does not begin at the immutable genesis frontier",
                ));
            }
            first.record.validate_directory_binding(directory_history)?;
            for pair in entries.windows(2) {
                pair[1]
                    .record
                    .validate_directory_binding(directory_history)?;
                pair[1]
                    .record
                    .validate_exact_successor(&pair[0].record, directory_history)?;
            }
        }
        let head_count = usize::try_from(head.entry_count)
            .map_err(|_| LifecycleV2Error::Exhausted("history count"))?;
        if head_count > entries.len() || entries.len() > head_count.saturating_add(1) {
            return Err(invalid("lifecycle head and history length disagree"));
        }
        let head_bytes_len = usize::try_from(head.journal_bytes)
            .map_err(|_| LifecycleV2Error::Exhausted("history length"))?;
        let prefix_len = entries
            .iter()
            .take(head_count)
            .try_fold(0usize, |sum, entry| {
                sum.checked_add(entry.canonical_line().map(|line| line.len())?)
                    .ok_or(LifecycleV2Error::Exhausted("history length"))
            })?;
        let prefix_entry = head_count
            .checked_sub(1)
            .and_then(|index| entries.get(index));
        if head_bytes_len != prefix_len
            || head.last_entry_hash != prefix_entry.map_or("", |entry| entry.entry_hash.as_str())
            || head.last_record_hash
                != prefix_entry.map_or(genesis.record_hash(), |entry| {
                    entry.record.record_hash.as_str()
                })
        {
            return Err(invalid("lifecycle head does not bind its journal prefix"));
        }
        if entries.len() == head_count + 1 {
            let entry = entries.last().expect("one complete successor exists");
            head = head.successor(entry, entry.canonical_line()?.len())?;
            if repair {
                atomic_write(&head_path, &encode_canonical(&head)?)?;
                sync_directory(root)?;
            }
        }
        let committed_len = usize::try_from(head.journal_bytes)
            .map_err(|_| LifecycleV2Error::Exhausted("history length"))?;
        if bytes.len() != committed_len {
            if complete_len != committed_len {
                return Err(invalid(
                    "lifecycle history has an uncommitted complete suffix",
                ));
            }
            if repair {
                let file = OpenOptions::new()
                    .write(true)
                    .open(&history_path)
                    .map_err(|source| io(&history_path, source))?;
                file.set_len(head.journal_bytes)
                    .map_err(|source| io(&history_path, source))?;
                file.sync_all()
                    .map_err(|source| io(&history_path, source))?;
            }
        }
        if repair {
            cleanup_stale_head_temps(root)?;
        }
        let current = entries
            .get(
                usize::try_from(head.entry_count)
                    .unwrap_or(0)
                    .saturating_sub(1),
            )
            .map(|entry| entry.record.clone());
        Ok(Self {
            root: root.to_path_buf(),
            head,
            current,
            poisoned: false,
            #[cfg(test)]
            failpoint: None,
        })
    }

    pub(crate) fn preflight_existing(
        root: impl AsRef<Path>,
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
        assignment: &ValidatedCurrentCellAssignmentV3<'_>,
        initial_frontier: &LifecycleWorldFrontierV2,
        frontier: &LifecycleWorldFrontierV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<LifecycleRuntimePreflightV2, LifecycleV2Error> {
        let root = root.as_ref();
        let history_path = root.join(LIFECYCLE_HISTORY_FILE);
        let history_exists = regular_file_exists(&history_path)?;
        let head_exists = regular_file_exists(&root.join(LIFECYCLE_HEAD_FILE))?;
        match (history_exists, head_exists) {
            (false, false) => return Ok(LifecycleRuntimePreflightV2::Absent),
            (true, false) => {
                if fs::metadata(&history_path)
                    .map_err(|source| io(&history_path, source))?
                    .len()
                    == 0
                {
                    return Ok(LifecycleRuntimePreflightV2::EmptyHistoryOnly);
                }
                return Err(invalid(
                    "lifecycle history exists without its empty bootstrap head",
                ));
            }
            (false, true) => {
                return Err(invalid("lifecycle head exists without its history"));
            }
            (true, true) => {}
        }
        let store = Self::recover(
            root,
            genesis,
            active_head_hash,
            initial_frontier,
            directory_history,
            false,
        )?;
        if store.current.is_none() {
            return Ok(LifecycleRuntimePreflightV2::EmptyPair);
        }
        store.validate_identity(genesis, active_head_hash)?;
        store
            .current()
            .validate_recovery_view(assignment, frontier)?;
        Ok(LifecycleRuntimePreflightV2::Committed(store.commitment()))
    }

    fn validate_identity(
        &self,
        genesis: &Protocol19TargetLifecycleGenesisV2,
        active_head_hash: &str,
    ) -> Result<(), LifecycleV2Error> {
        let current = self
            .current
            .as_ref()
            .ok_or_else(|| invalid("lifecycle history is empty"))?;
        if current.lifecycle_genesis_hash != genesis.record_hash()
            || current.active_head_hash != active_head_hash
            || current.cell_key != *genesis.cell_key()
            || current.cell_id != genesis.cell_id()
            || current.manifest_hash != genesis.manifest_hash()
            || current.migration_anchor_hash != genesis.migration_anchor_hash()
        {
            return Err(invalid(
                "lifecycle record identity differs from immutable genesis",
            ));
        }
        Ok(())
    }

    pub(crate) fn current(&self) -> &LifecycleRecordV2 {
        self.current
            .as_ref()
            .expect("opened lifecycle store has an initial record")
    }

    pub(crate) fn prepare_append(
        &mut self,
        record: LifecycleRecordV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<PreparedLifecycleAppendV2, LifecycleV2Error> {
        if self.poisoned {
            return Err(LifecycleV2Error::Poisoned);
        }
        let head_path = self.root.join(LIFECYCLE_HEAD_FILE);
        let persisted_head = read_bounded(&head_path, MAX_LIFECYCLE_HEAD_BYTES)?;
        if persisted_head != encode_canonical(&self.head)? {
            self.poisoned = true;
            return Err(invalid(
                "persisted lifecycle head diverged from the locked in-memory tip",
            ));
        }
        let history_path = self.root.join(LIFECYCLE_HISTORY_FILE);
        let persisted_history_length = fs::metadata(&history_path)
            .map_err(|source| io(&history_path, source))?
            .len();
        if persisted_history_length != self.head.journal_bytes {
            self.poisoned = true;
            return Err(invalid(
                "persisted lifecycle history diverged from the locked in-memory tip",
            ));
        }
        record.validate()?;
        record.validate_directory_binding(directory_history)?;
        if let Some(current) = &self.current {
            record.validate_exact_successor(current, directory_history)?;
        }
        let expected_previous_record = self
            .current
            .as_ref()
            .map_or(self.head.lifecycle_genesis_hash.as_str(), |current| {
                current.record_hash.as_str()
            });
        let expected_revision = self.current.as_ref().map_or(2, |current| {
            current.lifecycle_revision.checked_add(1).unwrap_or(0)
        });
        if record.previous_record_hash != expected_previous_record
            || record.lifecycle_revision != expected_revision
        {
            return Err(invalid("lifecycle append is not the exact successor"));
        }
        let entry = LifecycleHistoryEntryV2::new(record, self.head.last_entry_hash.clone())?;
        let line = entry.canonical_line()?;
        let next_head = self.head.successor(&entry, line.len())?;
        if next_head.journal_bytes > MAX_LIFECYCLE_HISTORY_BYTES {
            return Err(invalid("lifecycle history exceeds its size bound"));
        }
        Ok(PreparedLifecycleAppendV2 {
            record: entry.record.clone(),
            entry,
            line,
            next_head,
        })
    }

    #[cfg(test)]
    pub(crate) fn prepare_resealed_non_state_machine_successor_for_test(
        &self,
        mut record: LifecycleRecordV2,
    ) -> Result<PreparedLifecycleAppendV2, LifecycleV2Error> {
        record.activation_cutoff_unix_ms = Some(record.last_trusted_unix_ms);
        record.seal()?;
        let entry =
            LifecycleHistoryEntryV2::new(record.clone(), self.head.last_entry_hash.clone())?;
        let line = entry.canonical_line()?;
        let next_head = self.head.successor(&entry, line.len())?;
        Ok(PreparedLifecycleAppendV2 {
            record,
            entry,
            line,
            next_head,
        })
    }

    pub(crate) fn materialize_prepared(
        &mut self,
        prepared: PreparedLifecycleAppendV2,
        directory_history: &DraftCellDirectoryHistoryStoreV3,
    ) -> Result<(), LifecycleV2Error> {
        if self.poisoned {
            return Err(LifecycleV2Error::Poisoned);
        }
        let current_commitment = self.commitment();
        let checked = self.prepare_append(prepared.record.clone(), directory_history)?;
        if checked.next_commitment() != prepared.next_commitment()
            || checked.line != prepared.line
            || checked.entry != prepared.entry
            || current_commitment != self.commitment()
        {
            return Err(invalid(
                "prepared lifecycle append differs from the locked current tip",
            ));
        }
        let PreparedLifecycleAppendV2 {
            entry,
            line,
            next_head,
            ..
        } = prepared;
        let history_path = self.root.join(LIFECYCLE_HISTORY_FILE);
        let head_path = self.root.join(LIFECYCLE_HEAD_FILE);
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .append(true)
                .open(&history_path)
                .map_err(|source| io(&history_path, source))?;
            file.write_all(&line)
                .map_err(|source| io(&history_path, source))?;
            file.sync_all()
                .map_err(|source| io(&history_path, source))?;
            #[cfg(test)]
            if self.failpoint == Some(LifecycleAppendFailpointV2::JournalSyncedBeforeHead) {
                self.failpoint = None;
                return Err(invalid(
                    "injected failure after lifecycle journal synchronization",
                ));
            }
            atomic_write(&head_path, &encode_canonical(&next_head)?)?;
            #[cfg(test)]
            if self.failpoint == Some(LifecycleAppendFailpointV2::HeadRenamedBeforeMemory) {
                self.failpoint = None;
                return Err(invalid("injected failure after lifecycle head replacement"));
            }
            sync_directory(&self.root)
        })();
        if let Err(error) = write_result {
            self.poisoned = true;
            return Err(error);
        }
        self.current = Some(entry.record);
        self.head = next_head;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_failpoint(&mut self, failpoint: LifecycleAppendFailpointV2) {
        self.failpoint = Some(failpoint);
    }
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, LifecycleV2Error> {
    serde_json::to_vec(value).map_err(|source| invalid(source.to_string()))
}

fn hash_json<T: Serialize>(domain: &[u8], value: &T) -> Result<String, LifecycleV2Error> {
    let bytes = encode_canonical(value)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn invalid(message: impl Into<String>) -> LifecycleV2Error {
    LifecycleV2Error::Invalid(message.into())
}

fn io(path: &Path, source: std::io::Error) -> LifecycleV2Error {
    LifecycleV2Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn regular_file_exists(path: &Path) -> Result<bool, LifecycleV2Error> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => Ok(true),
        Ok(_) => Err(invalid("lifecycle artifact is not a regular file")),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(io(path, source)),
    }
}

fn read_bounded(path: &Path, maximum: usize) -> Result<Vec<u8>, LifecycleV2Error> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io(path, source))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > maximum as u64 {
        return Err(invalid("lifecycle artifact type or size is invalid"));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    File::open(path)
        .and_then(|mut file| file.read_to_end(&mut bytes))
        .map_err(|source| io(path, source))?;
    Ok(bytes)
}

fn create_synced_empty(path: &Path) -> Result<(), LifecycleV2Error> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| io(path, source))?;
    file.sync_all().map_err(|source| io(path, source))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), LifecycleV2Error> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("lifecycle path has no parent"))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid("lifecycle path is not UTF-8"))?;
    let temporary = parent.join(format!(".{name}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|source| io(&temporary, source))?;
        file.write_all(bytes)
            .map_err(|source| io(&temporary, source))?;
        file.sync_all().map_err(|source| io(&temporary, source))?;
        fs::rename(&temporary, path).map_err(|source| io(path, source))?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(crate) fn is_lifecycle_head_temp(name: &str) -> bool {
    let prefix = format!(".{LIFECYCLE_HEAD_FILE}.");
    name.strip_prefix(&prefix)
        .and_then(|suffix| suffix.strip_suffix(".tmp"))
        .is_some_and(|candidate| Uuid::parse_str(candidate).is_ok())
}

fn cleanup_stale_head_temps(root: &Path) -> Result<(), LifecycleV2Error> {
    let mut temps = Vec::new();
    for entry in fs::read_dir(root).map_err(|source| io(root, source))? {
        let entry = entry.map_err(|source| io(root, source))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_lifecycle_head_temp(name) {
            if !entry
                .file_type()
                .map_err(|source| io(&entry.path(), source))?
                .is_file()
            {
                return Err(invalid("lifecycle head temporary is not a regular file"));
            }
            temps.push(entry.path());
        }
    }
    if temps.len() > MAX_STALE_LIFECYCLE_HEAD_TEMPS {
        return Err(invalid("too many stale lifecycle head temporaries"));
    }
    for path in temps {
        fs::remove_file(&path).map_err(|source| io(&path, source))?;
    }
    sync_directory(root)
}

fn sync_directory(path: &Path) -> Result<(), LifecycleV2Error> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| io(path, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_append_requires_the_full_lease_safety_margin() {
        assert!(require_event_append_lease_margin(10_000, 15_000).is_ok());
        assert!(require_event_append_lease_margin(10_001, 15_000).is_err());
        assert!(require_event_append_lease_margin(u64::MAX, u64::MAX).is_err());
    }

    #[test]
    fn event_append_rejects_time_before_the_durable_lifecycle_frontier() {
        assert!(require_monotonic_trusted_time(10_000, 10_000).is_ok());
        assert!(require_monotonic_trusted_time(9_999, 10_000).is_err());
    }
}
