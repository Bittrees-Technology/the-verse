// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant canonical protocol-18 to protocol-19 migration receipt.
//!
//! The source-bound capability in this module commits an offline, terminal
//! protocol-18 universe and its world-21 genesis without rewriting event-16
//! history. It authorizes only dormant staging. Signatures and coordinated
//! activation remain separate future gates.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verse_protocol::CellKeyV1;
use verse_protocol::protocol_v19::Protocol19CompatibilityTuple;

use crate::cell_directory_v3::ValidatedProtocol19DirectoryGenesis;
use crate::grid_handoff_v2::migration_transform::{
    Protocol19TransformedCell, ValidatedProtocol19MigrationTransform,
};
use crate::{celestial, content};

const MIGRATION_SCHEMA_VERSION: u32 = 1;
const MIGRATION_KIND: &str =
    "protocol18-world20-event16-directory2-to-protocol19-world21-event17-manifest5";
const MIGRATION_ANCHOR_HASH_DOMAIN: &[u8] = b"the-verse/protocol-18-to-19-migration-anchor/v1\0";
const MIGRATION_RECEIPT_HASH_DOMAIN: &[u8] = b"the-verse/protocol-18-to-19-migration-receipt/v1\0";
const SOURCE_CELL_SET_HASH_DOMAIN: &[u8] = b"the-verse/protocol-18-terminal-cell-set/v1\0";
const TARGET_CELL_SET_HASH_DOMAIN: &[u8] = b"the-verse/protocol-19-genesis-cell-set/v1\0";
const TARGET_LIFECYCLE_HASH_DOMAIN: &[u8] = b"the-verse/protocol-19-target-lifecycle-genesis/v2\0";
const SOURCE_DIRECTORY_ARCHIVE_HASH_DOMAIN: &[u8] = b"the-verse/protocol-18-directory-archive/v1\0";
const MAX_MIGRATION_RECEIPT_BYTES: usize = 16 * 1_024 * 1_024;
pub(crate) const MAX_TARGET_LIFECYCLE_BYTES: usize = 64 * 1_024;
const MAX_MIGRATION_CELLS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Protocol19TargetLifecycleMode {
    StagedUnactivated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_field_names)]
struct Protocol18CompatibilityTuple {
    protocol_version: u32,
    projection_schema_version: u32,
    world_schema_version: u32,
    event_schema_version: u32,
    content_schema_version: u32,
    content_manifest_version: String,
    celestial_registry_schema_version: u32,
    universe_manifest_schema_version: u32,
    interest_schema_version: u32,
    operation_fingerprint_schema_version: u32,
    lifecycle_control_schema_version: u32,
    production_occurrence_schema_version: u32,
    cell_key_schema_version: u32,
    directory_schema_version: u32,
    transfer_package_schema_version: u32,
}

impl Protocol18CompatibilityTuple {
    fn canonical() -> Self {
        Self {
            protocol_version: verse_protocol::PROTOCOL_VERSION,
            projection_schema_version: verse_protocol::PROJECTION_SCHEMA_VERSION,
            world_schema_version: crate::model::WORLD_SCHEMA_VERSION,
            event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
            content_schema_version: content::manifest().schema_version,
            content_manifest_version: content::manifest().manifest_version.clone(),
            celestial_registry_schema_version: verse_protocol::CELESTIAL_REGISTRY_SCHEMA_VERSION,
            universe_manifest_schema_version: verse_protocol::UNIVERSE_MANIFEST_SCHEMA_VERSION,
            interest_schema_version: verse_protocol::INTEREST_SCHEMA_VERSION,
            operation_fingerprint_schema_version: verse_protocol::INTENT_FINGERPRINT_SCHEMA_VERSION,
            lifecycle_control_schema_version: verse_protocol::LIFECYCLE_CONTROL_SCHEMA_VERSION,
            production_occurrence_schema_version:
                verse_protocol::PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            cell_key_schema_version: verse_protocol::CELL_KEY_SCHEMA_VERSION,
            directory_schema_version: verse_protocol::CELL_DIRECTORY_SCHEMA_VERSION,
            transfer_package_schema_version: verse_protocol::TRANSFER_PACKAGE_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalCellV18Commitment {
    cell_key: CellKeyV1,
    cell_id: String,
    assignment_generation: u64,
    authority_fencing_token: u64,
    fencing_history_root: String,
    source_world_state_hash: String,
    source_snapshot_document_hash: String,
    source_event_schema_version: u32,
    source_event_sequence: u64,
    source_event_head_hash: String,
    source_event_archive_entry_count: u64,
    source_event_archive_root: String,
    source_lifecycle_revision: u64,
    source_lifecycle_record_hash: String,
    source_transfer_boundary_entry_count: u64,
    source_transfer_boundary_head_hash: String,
    source_transfer_boundary_archive_root: String,
    acknowledged_production_sequence: u64,
    next_production_occurrence_root: String,
    last_trusted_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct World21GenesisCommitment {
    cell_key: CellKeyV1,
    cell_id: String,
    migration_anchor_hash: String,
    target_world_state_hash: String,
    target_active_world_hash: String,
    target_lifecycle_record_hash: String,
    legacy_event_schema_version: u32,
    legacy_event_sequence: u64,
    legacy_event_head_hash: String,
    event17_genesis_sequence: u64,
    event17_predecessor_hash: String,
    event17_journal_entry_count: u64,
    event17_journal_head_hash: String,
    production_origin_root: String,
    identity_subset_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Protocol18To19MigrationAnchorV1 {
    schema_version: u32,
    migration_kind: String,
    source_compatibility: Protocol18CompatibilityTuple,
    target_compatibility: Protocol19CompatibilityTuple,
    universe_id: String,
    world_seed: String,
    trusted_cutoff_unix_ms: u64,
    source_manifest_hash: String,
    target_manifest_hash: String,
    source_directory_revision: u64,
    source_directory_document_hash: String,
    source_terminal_transfer_count: u64,
    source_terminal_transfer_root: String,
    source_assignment_root: String,
    source_placement_root: String,
    source_cells_root: String,
    source_cell_count: u64,
    identity_map_entry_count: u64,
    identity_map_root: String,
    production_origin_count: u64,
    production_origin_root: String,
    source_global_conservation_root: String,
    normalized_gameplay_root: String,
    anchor_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Protocol18To19MigrationReceiptV1 {
    schema_version: u32,
    anchor: Protocol18To19MigrationAnchorV1,
    source_directory_archive_hash: String,
    source_cells: Vec<TerminalCellV18Commitment>,
    identity_map_blob_hash: String,
    production_origin_blob_hash: String,
    target_directory_revision: u64,
    target_directory_document_hash: String,
    target_directory_history_entry_hash: String,
    target_assignment_root: String,
    target_placement_root: String,
    target_cells_root: String,
    target_cells: Vec<World21GenesisCommitment>,
    target_global_conservation_root: String,
    target_normalized_gameplay_root: String,
    receipt_hash: String,
}

/// Immutable lifecycle genesis for one staged world-21 cell. This is not a
/// lease and grants no runtime authority: the future protocol-19 scheduler
/// must replace the staged mode through the universe-wide activation gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Protocol19TargetLifecycleGenesisV2 {
    record_schema_version: u32,
    lifecycle_control_schema_version: u32,
    compatibility: Protocol19CompatibilityTuple,
    mode: Protocol19TargetLifecycleMode,
    universe_id: String,
    world_seed: String,
    cell_key: CellKeyV1,
    cell_id: String,
    manifest_hash: String,
    migration_anchor_hash: String,
    target_directory_revision: u64,
    target_directory_document_hash: String,
    assignment_generation: u64,
    authority_fencing_token: u64,
    lifecycle_revision: u64,
    trusted_cutoff_unix_ms: u64,
    snapshot_state_hash: String,
    active_world_hash: String,
    legacy_event_schema_version: u32,
    legacy_event_sequence: u64,
    legacy_event_head_hash: String,
    event17_genesis_sequence: u64,
    event17_predecessor_hash: String,
    event17_journal_entry_count: u64,
    event17_journal_head_hash: String,
    acknowledged_production_sequence: u64,
    next_production_occurrence_root: String,
    production_origin_root: String,
    identity_subset_root: String,
    record_hash: String,
}

impl Protocol19TargetLifecycleGenesisV2 {
    fn new(
        anchor: &Protocol18To19MigrationAnchorV1,
        target_directory_revision: u64,
        target_directory_document_hash: &str,
        source: &TerminalCellV18Commitment,
        target: &World21GenesisCommitment,
    ) -> Result<Self, MigrationReceiptError> {
        let mut record = Self {
            record_schema_version: 1,
            lifecycle_control_schema_version: anchor
                .target_compatibility
                .lifecycle_control_schema_version,
            compatibility: anchor.target_compatibility.clone(),
            mode: Protocol19TargetLifecycleMode::StagedUnactivated,
            universe_id: anchor.universe_id.clone(),
            world_seed: anchor.world_seed.clone(),
            cell_key: target.cell_key.clone(),
            cell_id: target.cell_id.clone(),
            manifest_hash: anchor.target_manifest_hash.clone(),
            migration_anchor_hash: anchor.anchor_hash.clone(),
            target_directory_revision,
            target_directory_document_hash: target_directory_document_hash.to_owned(),
            assignment_generation: source.assignment_generation,
            authority_fencing_token: source.authority_fencing_token,
            lifecycle_revision: 1,
            trusted_cutoff_unix_ms: anchor.trusted_cutoff_unix_ms,
            snapshot_state_hash: target.target_world_state_hash.clone(),
            active_world_hash: target.target_active_world_hash.clone(),
            legacy_event_schema_version: target.legacy_event_schema_version,
            legacy_event_sequence: target.legacy_event_sequence,
            legacy_event_head_hash: target.legacy_event_head_hash.clone(),
            event17_genesis_sequence: target.event17_genesis_sequence,
            event17_predecessor_hash: target.event17_predecessor_hash.clone(),
            event17_journal_entry_count: target.event17_journal_entry_count,
            event17_journal_head_hash: target.event17_journal_head_hash.clone(),
            acknowledged_production_sequence: source.acknowledged_production_sequence,
            next_production_occurrence_root: source.next_production_occurrence_root.clone(),
            production_origin_root: target.production_origin_root.clone(),
            identity_subset_root: target.identity_subset_root.clone(),
            record_hash: String::new(),
        };
        record.record_hash = record.calculate_hash()?;
        record.validate()?;
        Ok(record)
    }

    fn calculate_hash(&self) -> Result<String, MigrationReceiptError> {
        let mut material = self.clone();
        material.record_hash.clear();
        hash_json(TARGET_LIFECYCLE_HASH_DOMAIN, &material)
    }

    fn validate(&self) -> Result<(), MigrationReceiptError> {
        let seed = self.world_seed.parse::<u64>().map_err(|_| {
            MigrationReceiptError::Invalid("target lifecycle seed is invalid".into())
        })?;
        let manifest = crate::manifest_v5::build_validated_manifest_v5(seed)
            .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
        let expected_cell_id = celestial::cell_id(&self.cell_key)
            .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
        let empty_legacy =
            self.legacy_event_sequence == 0 && self.legacy_event_head_hash.is_empty();
        let populated_legacy =
            self.legacy_event_sequence > 0 && valid_hash(&self.legacy_event_head_hash);
        if self.record_schema_version != 1
            || self.lifecycle_control_schema_version
                != verse_protocol::protocol_v19::LIFECYCLE_CONTROL_SCHEMA_VERSION
            || self.compatibility != Protocol19CompatibilityTuple::canonical()
            || self.mode != Protocol19TargetLifecycleMode::StagedUnactivated
            || seed.to_string() != self.world_seed
            || self.universe_id != manifest.universe_id()
            || self.cell_key.universe_id != self.universe_id
            || self.cell_id != expected_cell_id
            || self.manifest_hash != manifest.manifest_hash()
            || self.target_directory_revision == 0
            || self.assignment_generation == 0
            || self.authority_fencing_token == 0
            || self.lifecycle_revision != 1
            || self.trusted_cutoff_unix_ms == 0
            || self.legacy_event_schema_version != crate::event::EVENT_SCHEMA_VERSION
            || self.event17_genesis_sequence != self.legacy_event_sequence
            || self.event17_predecessor_hash != self.legacy_event_head_hash
            || self.event17_journal_entry_count != 0
            || !self.event17_journal_head_hash.is_empty()
            || !(empty_legacy || populated_legacy)
            || !all_hashes([
                &self.manifest_hash,
                &self.migration_anchor_hash,
                &self.target_directory_document_hash,
                &self.snapshot_state_hash,
                &self.active_world_hash,
                &self.next_production_occurrence_root,
                &self.production_origin_root,
                &self.identity_subset_root,
                &self.record_hash,
            ])
            || self.record_hash != self.calculate_hash()?
        {
            return Err(MigrationReceiptError::Invalid(
                "target lifecycle genesis is not canonical staged protocol-19 material".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn encode_canonical(&self) -> Result<Vec<u8>, MigrationReceiptError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|source| MigrationReceiptError::Json(source.to_string()))?;
        if bytes.is_empty() || bytes.len() > MAX_TARGET_LIFECYCLE_BYTES {
            return Err(MigrationReceiptError::TooLarge);
        }
        Ok(bytes)
    }

    pub(crate) fn record_hash(&self) -> &str {
        &self.record_hash
    }

    pub(crate) fn manifest_hash(&self) -> &str {
        &self.manifest_hash
    }

    pub(crate) fn migration_anchor_hash(&self) -> &str {
        &self.migration_anchor_hash
    }

    pub(crate) fn cell_key(&self) -> &CellKeyV1 {
        &self.cell_key
    }

    pub(crate) fn cell_id(&self) -> &str {
        &self.cell_id
    }

    pub(crate) fn snapshot_state_hash(&self) -> &str {
        &self.snapshot_state_hash
    }

    pub(crate) fn active_world_hash(&self) -> &str {
        &self.active_world_hash
    }

    pub(crate) const fn authority_fencing_token(&self) -> u64 {
        self.authority_fencing_token
    }

    pub(crate) const fn assignment_generation(&self) -> u64 {
        self.assignment_generation
    }

    pub(crate) const fn trusted_cutoff_unix_ms(&self) -> u64 {
        self.trusted_cutoff_unix_ms
    }

    pub(crate) const fn target_directory_revision(&self) -> u64 {
        self.target_directory_revision
    }

    pub(crate) fn target_directory_document_hash(&self) -> &str {
        &self.target_directory_document_hash
    }

    pub(crate) const fn acknowledged_production_sequence(&self) -> u64 {
        self.acknowledged_production_sequence
    }

    pub(crate) const fn legacy_event_sequence(&self) -> u64 {
        self.legacy_event_sequence
    }

    pub(crate) fn legacy_event_head_hash(&self) -> &str {
        &self.legacy_event_head_hash
    }

    pub(crate) fn production_origin_root(&self) -> &str {
        &self.production_origin_root
    }

    pub(crate) fn identity_subset_root(&self) -> &str {
        &self.identity_subset_root
    }
}

pub(crate) fn decode_target_lifecycle_genesis(
    bytes: &[u8],
) -> Result<Protocol19TargetLifecycleGenesisV2, MigrationReceiptError> {
    if bytes.is_empty() || bytes.len() > MAX_TARGET_LIFECYCLE_BYTES {
        return Err(MigrationReceiptError::TooLarge);
    }
    let record = serde_json::from_slice::<Protocol19TargetLifecycleGenesisV2>(bytes)
        .map_err(|source| MigrationReceiptError::Json(source.to_string()))?;
    record.validate()?;
    if record.encode_canonical()? != bytes {
        return Err(MigrationReceiptError::Invalid(
            "target lifecycle bytes are not the exact canonical encoding".into(),
        ));
    }
    Ok(record)
}

/// Canonical syntax and internal-consistency result. This is deliberately not
/// an installation or activation capability.
#[derive(Debug)]
struct CanonicalProtocol18To19MigrationReceipt {
    document: Protocol18To19MigrationReceiptV1,
}

impl CanonicalProtocol18To19MigrationReceipt {
    fn document(&self) -> &Protocol18To19MigrationReceiptV1 {
        &self.document
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalProtocol19TargetCellEvidence {
    pub(crate) cell_key: CellKeyV1,
    pub(crate) cell_id: String,
    pub(crate) migration_anchor_hash: String,
    pub(crate) snapshot_state_hash: String,
    pub(crate) active_world_hash: String,
    pub(crate) lifecycle_record_hash: String,
    pub(crate) production_origin_root: String,
    pub(crate) identity_subset_root: String,
    pub(crate) legacy_event_sequence: u64,
    pub(crate) legacy_event_head_hash: String,
    pub(crate) event17_genesis_sequence: u64,
    pub(crate) event17_predecessor_hash: String,
    pub(crate) event17_journal_entry_count: u64,
    pub(crate) event17_journal_head_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CanonicalProtocol19MigrationReceiptEvidence {
    pub(crate) universe_id: String,
    pub(crate) world_seed: u64,
    pub(crate) trusted_cutoff_unix_ms: u64,
    pub(crate) target_manifest_hash: String,
    pub(crate) migration_anchor_hash: String,
    pub(crate) migration_receipt_hash: String,
    pub(crate) source_directory_archive_hash: String,
    pub(crate) identity_map_root: String,
    pub(crate) production_origin_root: String,
    pub(crate) target_directory_revision: u64,
    pub(crate) target_directory_document_hash: String,
    pub(crate) target_directory_history_entry_hash: String,
    pub(crate) target_assignment_root: String,
    pub(crate) target_placement_root: String,
    pub(crate) target_cells_root: String,
    pub(crate) global_conservation_root: String,
    pub(crate) normalized_gameplay_root: String,
    pub(crate) cell_count: u64,
    pub(crate) target_cells: Vec<CanonicalProtocol19TargetCellEvidence>,
}

pub(crate) fn recover_canonical_migration_receipt(
    bytes: &[u8],
) -> Result<CanonicalProtocol19MigrationReceiptEvidence, MigrationReceiptError> {
    let receipt = decode_canonical(bytes)?;
    let document = receipt.document();
    let world_seed = document.anchor.world_seed.parse::<u64>().map_err(|_| {
        MigrationReceiptError::Invalid("migration receipt seed is not canonical".into())
    })?;
    Ok(CanonicalProtocol19MigrationReceiptEvidence {
        universe_id: document.anchor.universe_id.clone(),
        world_seed,
        trusted_cutoff_unix_ms: document.anchor.trusted_cutoff_unix_ms,
        target_manifest_hash: document.anchor.target_manifest_hash.clone(),
        migration_anchor_hash: document.anchor.anchor_hash.clone(),
        migration_receipt_hash: document.receipt_hash.clone(),
        source_directory_archive_hash: document.source_directory_archive_hash.clone(),
        identity_map_root: document.identity_map_blob_hash.clone(),
        production_origin_root: document.production_origin_blob_hash.clone(),
        target_directory_revision: document.target_directory_revision,
        target_directory_document_hash: document.target_directory_document_hash.clone(),
        target_directory_history_entry_hash: document.target_directory_history_entry_hash.clone(),
        target_assignment_root: document.target_assignment_root.clone(),
        target_placement_root: document.target_placement_root.clone(),
        target_cells_root: document.target_cells_root.clone(),
        global_conservation_root: document.target_global_conservation_root.clone(),
        normalized_gameplay_root: document.target_normalized_gameplay_root.clone(),
        cell_count: u64::try_from(document.target_cells.len()).map_err(|_| {
            MigrationReceiptError::Invalid("migration receipt cell count overflowed".into())
        })?,
        target_cells: document
            .target_cells
            .iter()
            .map(|cell| CanonicalProtocol19TargetCellEvidence {
                cell_key: cell.cell_key.clone(),
                cell_id: cell.cell_id.clone(),
                migration_anchor_hash: cell.migration_anchor_hash.clone(),
                snapshot_state_hash: cell.target_world_state_hash.clone(),
                active_world_hash: cell.target_active_world_hash.clone(),
                lifecycle_record_hash: cell.target_lifecycle_record_hash.clone(),
                production_origin_root: cell.production_origin_root.clone(),
                identity_subset_root: cell.identity_subset_root.clone(),
                legacy_event_sequence: cell.legacy_event_sequence,
                legacy_event_head_hash: cell.legacy_event_head_hash.clone(),
                event17_genesis_sequence: cell.event17_genesis_sequence,
                event17_predecessor_hash: cell.event17_predecessor_hash.clone(),
                event17_journal_entry_count: cell.event17_journal_entry_count,
                event17_journal_head_hash: cell.event17_journal_head_hash.clone(),
            })
            .collect(),
    })
}

pub(crate) fn hash_source_directory_archive(bytes: &[u8]) -> String {
    hash_bytes(SOURCE_DIRECTORY_ARCHIVE_HASH_DOMAIN, bytes)
}

/// Non-Serde source-bound receipt capability. It can only be derived while the
/// frozen source locks are held by the validated transform, and it borrows the
/// exact directory-v3 genesis used to construct the receipt.
#[derive(Debug)]
pub(crate) struct ValidatedProtocol19MigrationReceipt<'migration, 'source> {
    transform: &'migration ValidatedProtocol19MigrationTransform<'source>,
    receipt: CanonicalProtocol18To19MigrationReceipt,
    bytes: Vec<u8>,
}

impl<'migration, 'source> ValidatedProtocol19MigrationReceipt<'migration, 'source> {
    pub(crate) fn derive(
        transform: &'migration ValidatedProtocol19MigrationTransform<'source>,
        directory: &ValidatedProtocol19DirectoryGenesis<'migration, 'source>,
    ) -> Result<Self, MigrationReceiptError> {
        let source = transform.source();
        directory
            .validate_for_transform(transform)
            .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
        if directory.directory_revision() != 1
            || transform.target_manifest_hash().is_empty()
            || transform.cells().len() != source.cells().len()
        {
            return Err(MigrationReceiptError::Invalid(
                "migration capabilities do not form one complete target universe".into(),
            ));
        }
        let source_cells = source
            .cells()
            .iter()
            .map(|cell| TerminalCellV18Commitment {
                cell_key: cell.cell_key().clone(),
                cell_id: cell.cell_id().to_owned(),
                assignment_generation: cell.assignment_generation(),
                authority_fencing_token: cell.authority_fencing_token(),
                fencing_history_root: cell.fencing_history_root().to_owned(),
                source_world_state_hash: cell.world_state_hash().to_owned(),
                source_snapshot_document_hash: cell.snapshot_document_hash().to_owned(),
                source_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
                source_event_sequence: cell.event_sequence(),
                source_event_head_hash: cell.event_head_hash().to_owned(),
                source_event_archive_entry_count: cell.event_archive_entry_count(),
                source_event_archive_root: cell.event_archive_root().to_owned(),
                source_lifecycle_revision: cell.lifecycle_revision(),
                source_lifecycle_record_hash: cell.lifecycle_record_hash().to_owned(),
                source_transfer_boundary_entry_count: cell.transfer_boundary_entry_count(),
                source_transfer_boundary_head_hash: cell.transfer_boundary_head_hash().to_owned(),
                source_transfer_boundary_archive_root: cell
                    .transfer_boundary_archive_root()
                    .to_owned(),
                acknowledged_production_sequence: cell.acknowledged_production_sequence(),
                next_production_occurrence_root: cell.next_production_occurrence_root().to_owned(),
                last_trusted_unix_ms: cell.last_trusted_unix_ms(),
            })
            .collect::<Vec<_>>();
        if source_cells
            .iter()
            .zip(transform.cells())
            .any(|(source, target)| {
                source.cell_id != target.cell_id() || source.cell_key != *target.cell_key()
            })
        {
            return Err(MigrationReceiptError::Invalid(
                "source and transformed target cell order differs".into(),
            ));
        }
        let trusted_cutoff_unix_ms = source_cells
            .iter()
            .map(|cell| cell.last_trusted_unix_ms)
            .max()
            .ok_or_else(|| {
                MigrationReceiptError::Invalid("migration has no source cells".into())
            })?;
        let source_cells_root = hash_source_cells(&source_cells)?;
        let mut anchor = Protocol18To19MigrationAnchorV1 {
            schema_version: MIGRATION_SCHEMA_VERSION,
            migration_kind: MIGRATION_KIND.into(),
            source_compatibility: Protocol18CompatibilityTuple::canonical(),
            target_compatibility: Protocol19CompatibilityTuple::canonical(),
            universe_id: source.universe_id().to_owned(),
            world_seed: source.world_seed().to_string(),
            trusted_cutoff_unix_ms,
            source_manifest_hash: source.source_manifest_hash().to_owned(),
            target_manifest_hash: transform.target_manifest_hash().to_owned(),
            source_directory_revision: source.directory_revision(),
            source_directory_document_hash: source.directory_document_hash().to_owned(),
            source_terminal_transfer_count: source.terminal_transfer_count(),
            source_terminal_transfer_root: source.terminal_transfer_root().to_owned(),
            source_assignment_root: source.assignment_root().to_owned(),
            source_placement_root: source.placement_root().to_owned(),
            source_cells_root,
            source_cell_count: u64::try_from(source_cells.len()).map_err(|_| {
                MigrationReceiptError::Invalid("migration cell count overflowed".into())
            })?,
            identity_map_entry_count: transform.identity_map_entry_count(),
            identity_map_root: transform.identity_map_root().to_owned(),
            production_origin_count: transform.production_origin_count(),
            production_origin_root: transform.production_origin_root().to_owned(),
            source_global_conservation_root: transform.global_conservation_root().to_owned(),
            normalized_gameplay_root: transform.normalized_gameplay_root().to_owned(),
            anchor_hash: String::new(),
        };
        anchor.anchor_hash = calculate_anchor_hash(&anchor)?;
        let mut target_cells = transform
            .cells()
            .iter()
            .map(|cell| {
                Ok(World21GenesisCommitment {
                    cell_key: cell.cell_key().clone(),
                    cell_id: cell.cell_id().to_owned(),
                    migration_anchor_hash: anchor.anchor_hash.clone(),
                    target_world_state_hash: cell.world_state_hash().to_owned(),
                    target_active_world_hash: cell.active_world_hash()?,
                    target_lifecycle_record_hash: String::new(),
                    legacy_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
                    legacy_event_sequence: cell.event_sequence(),
                    legacy_event_head_hash: cell.event_head_hash().to_owned(),
                    event17_genesis_sequence: cell.event_sequence(),
                    event17_predecessor_hash: cell.event_head_hash().to_owned(),
                    event17_journal_entry_count: 0,
                    event17_journal_head_hash: String::new(),
                    production_origin_root: cell.production_origin_root().to_owned(),
                    identity_subset_root: cell.identity_subset_root().to_owned(),
                })
            })
            .collect::<Result<
                Vec<_>,
                crate::grid_handoff_v2::migration_transform::Protocol19MigrationTransformError,
            >>()
            .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
        for (index, source_cell) in source_cells.iter().enumerate() {
            let lifecycle = Protocol19TargetLifecycleGenesisV2::new(
                &anchor,
                directory.directory_revision(),
                directory.document_hash(),
                source_cell,
                &target_cells[index],
            )?;
            lifecycle
                .record_hash()
                .clone_into(&mut target_cells[index].target_lifecycle_record_hash);
        }
        let mut document = Protocol18To19MigrationReceiptV1 {
            schema_version: MIGRATION_SCHEMA_VERSION,
            anchor,
            source_directory_archive_hash: hash_bytes(
                SOURCE_DIRECTORY_ARCHIVE_HASH_DOMAIN,
                source.directory_document_bytes(),
            ),
            source_cells,
            identity_map_blob_hash: transform.identity_map_root().to_owned(),
            production_origin_blob_hash: transform.production_origin_root().to_owned(),
            target_directory_revision: directory.directory_revision(),
            target_directory_document_hash: directory.document_hash().to_owned(),
            target_directory_history_entry_hash: directory.history_entry_hash().to_owned(),
            target_assignment_root: directory.assignment_root().to_owned(),
            target_placement_root: directory.placement_root().to_owned(),
            target_cells_root: hash_target_cells(&target_cells)?,
            target_cells,
            target_global_conservation_root: transform.global_conservation_root().to_owned(),
            target_normalized_gameplay_root: transform.normalized_gameplay_root().to_owned(),
            receipt_hash: String::new(),
        };
        document.receipt_hash = calculate_receipt_hash(&document)?;
        let receipt = CanonicalProtocol18To19MigrationReceipt { document };
        let bytes = encode_canonical(&receipt)?;
        let decoded = decode_canonical(&bytes)?;
        if decoded.document() != receipt.document() {
            return Err(MigrationReceiptError::Invalid(
                "canonical migration receipt failed deterministic self-validation".into(),
            ));
        }
        Ok(Self {
            transform,
            receipt,
            bytes,
        })
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn anchor_hash(&self) -> &str {
        &self.receipt.document.anchor.anchor_hash
    }

    pub(crate) fn receipt_hash(&self) -> &str {
        &self.receipt.document.receipt_hash
    }

    pub(crate) fn source_directory_archive_hash(&self) -> &str {
        &self.receipt.document.source_directory_archive_hash
    }

    pub(crate) fn transform(&self) -> &ValidatedProtocol19MigrationTransform<'source> {
        self.transform
    }

    pub(crate) fn bind_cell(
        &self,
        cell: &Protocol19TransformedCell,
    ) -> Result<Protocol19World21StagingCommitment, MigrationReceiptError> {
        if !self
            .transform
            .cells()
            .iter()
            .any(|candidate| std::ptr::eq(candidate, cell))
        {
            return Err(MigrationReceiptError::Invalid(
                "transformed cell belongs to another migration capability".into(),
            ));
        }
        let source = self.transform.source();
        let active_world_hash = cell
            .active_world_hash()
            .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
        let source_cell = source
            .cells()
            .iter()
            .find(|source_cell| source_cell.cell_id() == cell.cell_id())
            .ok_or_else(|| {
                MigrationReceiptError::Invalid("target cell has no frozen source".into())
            })?;
        let evidence = Protocol19World21StagingEvidence {
            manifest_hash: self.transform.target_manifest_hash(),
            universe_id: source.universe_id(),
            world_seed: source.world_seed(),
            cell_key: cell.cell_key(),
            cell_id: cell.cell_id(),
            authority_fencing_token: cell.authority_fencing_token(),
            snapshot_state_hash: cell.world_state_hash(),
            active_world_hash: &active_world_hash,
            legacy_event_sequence: cell.event_sequence(),
            legacy_event_head_hash: cell.event_head_hash(),
        };
        let commitment = bind_world21_staging_target(&self.bytes, &evidence)?;
        if commitment.lifecycle().assignment_generation() != source_cell.assignment_generation()
            || commitment.production_origin_root() != cell.production_origin_root()
            || commitment.identity_subset_root() != cell.identity_subset_root()
        {
            return Err(MigrationReceiptError::Invalid(
                "migration receipt does not bind the exact source and transform cell".into(),
            ));
        }
        Ok(commitment)
    }
}

/// Exact target material supplied by the already validated world-21 state
/// capability. Scalar claims from the receipt are compared with this evidence;
/// they never create it.
pub(crate) struct Protocol19World21StagingEvidence<'a> {
    pub(crate) manifest_hash: &'a str,
    pub(crate) universe_id: &'a str,
    pub(crate) world_seed: u64,
    pub(crate) cell_key: &'a CellKeyV1,
    pub(crate) cell_id: &'a str,
    pub(crate) authority_fencing_token: u64,
    pub(crate) snapshot_state_hash: &'a str,
    pub(crate) active_world_hash: &'a str,
    pub(crate) legacy_event_sequence: u64,
    pub(crate) legacy_event_head_hash: &'a str,
}

/// Non-Serde proof that one canonical migration receipt identifies the exact
/// validated world-21 snapshot to be staged. It does not prove the source
/// archives or grant activation authority.
#[derive(Debug)]
pub(crate) struct Protocol19World21StagingCommitment {
    lifecycle: Protocol19TargetLifecycleGenesisV2,
    migration_receipt_hash: String,
    production_origin_root: String,
    identity_subset_root: String,
}

impl Protocol19World21StagingCommitment {
    pub(crate) fn lifecycle(&self) -> &Protocol19TargetLifecycleGenesisV2 {
        &self.lifecycle
    }

    pub(crate) fn migration_receipt_hash(&self) -> &str {
        &self.migration_receipt_hash
    }

    pub(crate) fn production_origin_root(&self) -> &str {
        &self.production_origin_root
    }

    pub(crate) fn identity_subset_root(&self) -> &str {
        &self.identity_subset_root
    }
}

pub(crate) fn bind_world21_staging_target(
    receipt_bytes: &[u8],
    evidence: &Protocol19World21StagingEvidence<'_>,
) -> Result<Protocol19World21StagingCommitment, MigrationReceiptError> {
    let receipt = decode_canonical(receipt_bytes)?;
    let document = receipt.document();
    let seed_text = evidence.world_seed.to_string();
    let index = document
        .target_cells
        .binary_search_by(|target| target.cell_id.as_str().cmp(evidence.cell_id))
        .map_err(|_| {
            MigrationReceiptError::Invalid(
                "validated world-21 cell is absent from the migration receipt".into(),
            )
        })?;
    let source = &document.source_cells[index];
    let target = &document.target_cells[index];
    if document.anchor.universe_id != evidence.universe_id
        || document.anchor.world_seed != seed_text
        || document.anchor.target_manifest_hash != evidence.manifest_hash
        || &target.cell_key != evidence.cell_key
        || target.cell_id != evidence.cell_id
        || source.cell_key != *evidence.cell_key
        || source.cell_id != evidence.cell_id
        || source.authority_fencing_token != evidence.authority_fencing_token
        || target.target_world_state_hash != evidence.snapshot_state_hash
        || target.target_active_world_hash != evidence.active_world_hash
        || target.legacy_event_sequence != evidence.legacy_event_sequence
        || target.legacy_event_head_hash != evidence.legacy_event_head_hash
        || target.event17_genesis_sequence != evidence.legacy_event_sequence
        || target.event17_predecessor_hash != evidence.legacy_event_head_hash
        || target.event17_journal_entry_count != 0
        || !target.event17_journal_head_hash.is_empty()
    {
        return Err(MigrationReceiptError::Invalid(
            "migration receipt does not identify the exact validated world-21 staging target"
                .into(),
        ));
    }
    let lifecycle = Protocol19TargetLifecycleGenesisV2::new(
        &document.anchor,
        document.target_directory_revision,
        &document.target_directory_document_hash,
        source,
        target,
    )?;
    if lifecycle.record_hash() != target.target_lifecycle_record_hash {
        return Err(MigrationReceiptError::Invalid(
            "migration receipt target lifecycle commitment changed".into(),
        ));
    }
    Ok(Protocol19World21StagingCommitment {
        lifecycle,
        migration_receipt_hash: document.receipt_hash.clone(),
        production_origin_root: target.production_origin_root.clone(),
        identity_subset_root: target.identity_subset_root.clone(),
    })
}

#[cfg(test)]
pub(crate) fn test_world21_staging_commitment(
    evidence: &Protocol19World21StagingEvidence<'_>,
    migration_anchor_hash: &str,
) -> Protocol19World21StagingCommitment {
    fn test_hash(label: &str) -> String {
        blake3::hash(label.as_bytes()).to_hex().to_string()
    }

    let source_manifest = celestial::universe_manifest(
        evidence.world_seed,
        crate::model::WORLD_SCHEMA_VERSION,
        crate::event::EVENT_SCHEMA_VERSION,
    )
    .expect("test source manifest derives");
    let mut anchor = Protocol18To19MigrationAnchorV1 {
        schema_version: MIGRATION_SCHEMA_VERSION,
        migration_kind: MIGRATION_KIND.into(),
        source_compatibility: Protocol18CompatibilityTuple::canonical(),
        target_compatibility: Protocol19CompatibilityTuple::canonical(),
        universe_id: evidence.universe_id.to_owned(),
        world_seed: evidence.world_seed.to_string(),
        trusted_cutoff_unix_ms: 1_800_000_000_000,
        source_manifest_hash: source_manifest.manifest_hash,
        target_manifest_hash: evidence.manifest_hash.to_owned(),
        source_directory_revision: 1,
        source_directory_document_hash: test_hash("test-source-directory"),
        source_terminal_transfer_count: 0,
        source_terminal_transfer_root: test_hash("test-terminal-transfers"),
        source_assignment_root: test_hash("test-source-assignments"),
        source_placement_root: test_hash("test-source-placements"),
        source_cells_root: test_hash("test-source-cells"),
        source_cell_count: 1,
        identity_map_entry_count: 0,
        identity_map_root: test_hash("test-identities"),
        production_origin_count: 0,
        production_origin_root: test_hash("test-production-origins"),
        source_global_conservation_root: test_hash("test-conservation"),
        normalized_gameplay_root: test_hash("test-gameplay"),
        anchor_hash: migration_anchor_hash.to_owned(),
    };
    // The caller supplies the anchor in legacy Store tests. Retain it exactly;
    // those tests exercise Store durability rather than receipt validation.
    if anchor.anchor_hash.is_empty() {
        anchor.anchor_hash = calculate_anchor_hash(&anchor).expect("test anchor hashes");
    }
    let source = TerminalCellV18Commitment {
        cell_key: evidence.cell_key.clone(),
        cell_id: evidence.cell_id.to_owned(),
        assignment_generation: 1,
        authority_fencing_token: evidence.authority_fencing_token,
        fencing_history_root: test_hash("test-fencing-history"),
        source_world_state_hash: test_hash("test-source-world"),
        source_snapshot_document_hash: test_hash("test-source-snapshot"),
        source_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
        source_event_sequence: evidence.legacy_event_sequence,
        source_event_head_hash: evidence.legacy_event_head_hash.to_owned(),
        source_event_archive_entry_count: evidence.legacy_event_sequence,
        source_event_archive_root: test_hash("test-event-archive"),
        source_lifecycle_revision: 1,
        source_lifecycle_record_hash: test_hash("test-source-lifecycle"),
        source_transfer_boundary_entry_count: 0,
        source_transfer_boundary_head_hash: String::new(),
        source_transfer_boundary_archive_root: String::new(),
        acknowledged_production_sequence: 0,
        next_production_occurrence_root: test_hash("test-next-production"),
        last_trusted_unix_ms: anchor.trusted_cutoff_unix_ms,
    };
    let target = World21GenesisCommitment {
        cell_key: evidence.cell_key.clone(),
        cell_id: evidence.cell_id.to_owned(),
        migration_anchor_hash: anchor.anchor_hash.clone(),
        target_world_state_hash: evidence.snapshot_state_hash.to_owned(),
        target_active_world_hash: evidence.active_world_hash.to_owned(),
        target_lifecycle_record_hash: String::new(),
        legacy_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
        legacy_event_sequence: evidence.legacy_event_sequence,
        legacy_event_head_hash: evidence.legacy_event_head_hash.to_owned(),
        event17_genesis_sequence: evidence.legacy_event_sequence,
        event17_predecessor_hash: evidence.legacy_event_head_hash.to_owned(),
        event17_journal_entry_count: 0,
        event17_journal_head_hash: String::new(),
        production_origin_root: test_hash("test-cell-production"),
        identity_subset_root: test_hash("test-cell-identities"),
    };
    let lifecycle = Protocol19TargetLifecycleGenesisV2::new(
        &anchor,
        1,
        &test_hash("test-target-directory"),
        &source,
        &target,
    )
    .expect("test lifecycle derives");
    Protocol19World21StagingCommitment {
        lifecycle,
        migration_receipt_hash: test_hash("test-migration-receipt"),
        production_origin_root: target.production_origin_root,
        identity_subset_root: target.identity_subset_root,
    }
}

#[cfg(test)]
pub(crate) fn test_canonical_receipt_bytes_for_world21_target(
    evidence: &Protocol19World21StagingEvidence<'_>,
) -> Vec<u8> {
    fn test_hash(label: &str) -> String {
        blake3::hash(label.as_bytes()).to_hex().to_string()
    }

    let source_manifest = celestial::universe_manifest(
        evidence.world_seed,
        crate::model::WORLD_SCHEMA_VERSION,
        crate::event::EVENT_SCHEMA_VERSION,
    )
    .expect("test source manifest derives");
    let event_archive_root = if evidence.legacy_event_sequence > 0 {
        test_hash("canonical-test-event-archive")
    } else {
        String::new()
    };
    let source = TerminalCellV18Commitment {
        cell_key: evidence.cell_key.clone(),
        cell_id: evidence.cell_id.to_owned(),
        assignment_generation: 1,
        authority_fencing_token: evidence.authority_fencing_token,
        fencing_history_root: test_hash("canonical-test-fencing-history"),
        source_world_state_hash: test_hash("canonical-test-source-world"),
        source_snapshot_document_hash: test_hash("canonical-test-source-snapshot"),
        source_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
        source_event_sequence: evidence.legacy_event_sequence,
        source_event_head_hash: evidence.legacy_event_head_hash.to_owned(),
        source_event_archive_entry_count: evidence.legacy_event_sequence,
        source_event_archive_root: event_archive_root,
        source_lifecycle_revision: 1,
        source_lifecycle_record_hash: test_hash("canonical-test-source-lifecycle"),
        source_transfer_boundary_entry_count: 0,
        source_transfer_boundary_head_hash: String::new(),
        source_transfer_boundary_archive_root: String::new(),
        acknowledged_production_sequence: 0,
        next_production_occurrence_root: test_hash("canonical-test-next-production"),
        last_trusted_unix_ms: 1_800_000_000_000,
    };
    let source_cells = vec![source];
    let identity_map_root = test_hash("canonical-test-identities");
    let production_origin_root = test_hash("canonical-test-production-origins");
    let conservation_root = test_hash("canonical-test-conservation");
    let gameplay_root = test_hash("canonical-test-gameplay");
    let mut anchor = Protocol18To19MigrationAnchorV1 {
        schema_version: MIGRATION_SCHEMA_VERSION,
        migration_kind: MIGRATION_KIND.into(),
        source_compatibility: Protocol18CompatibilityTuple::canonical(),
        target_compatibility: Protocol19CompatibilityTuple::canonical(),
        universe_id: evidence.universe_id.to_owned(),
        world_seed: evidence.world_seed.to_string(),
        trusted_cutoff_unix_ms: 1_800_000_000_000,
        source_manifest_hash: source_manifest.manifest_hash,
        target_manifest_hash: evidence.manifest_hash.to_owned(),
        source_directory_revision: 1,
        source_directory_document_hash: test_hash("canonical-test-source-directory"),
        source_terminal_transfer_count: 0,
        source_terminal_transfer_root: test_hash("canonical-test-terminal-transfers"),
        source_assignment_root: test_hash("canonical-test-source-assignments"),
        source_placement_root: test_hash("canonical-test-source-placements"),
        source_cells_root: hash_source_cells(&source_cells).expect("source cells hash"),
        source_cell_count: 1,
        identity_map_entry_count: 0,
        identity_map_root: identity_map_root.clone(),
        production_origin_count: 0,
        production_origin_root: production_origin_root.clone(),
        source_global_conservation_root: conservation_root.clone(),
        normalized_gameplay_root: gameplay_root.clone(),
        anchor_hash: String::new(),
    };
    anchor.anchor_hash = calculate_anchor_hash(&anchor).expect("canonical test anchor hashes");
    let target = World21GenesisCommitment {
        cell_key: evidence.cell_key.clone(),
        cell_id: evidence.cell_id.to_owned(),
        migration_anchor_hash: anchor.anchor_hash.clone(),
        target_world_state_hash: evidence.snapshot_state_hash.to_owned(),
        target_active_world_hash: evidence.active_world_hash.to_owned(),
        target_lifecycle_record_hash: String::new(),
        legacy_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
        legacy_event_sequence: evidence.legacy_event_sequence,
        legacy_event_head_hash: evidence.legacy_event_head_hash.to_owned(),
        event17_genesis_sequence: evidence.legacy_event_sequence,
        event17_predecessor_hash: evidence.legacy_event_head_hash.to_owned(),
        event17_journal_entry_count: 0,
        event17_journal_head_hash: String::new(),
        production_origin_root: test_hash("canonical-test-cell-production"),
        identity_subset_root: test_hash("canonical-test-cell-identities"),
    };
    let mut document = Protocol18To19MigrationReceiptV1 {
        schema_version: MIGRATION_SCHEMA_VERSION,
        anchor,
        source_directory_archive_hash: test_hash("canonical-test-directory-archive"),
        source_cells,
        identity_map_blob_hash: identity_map_root,
        production_origin_blob_hash: production_origin_root,
        target_directory_revision: 1,
        target_directory_document_hash: test_hash("canonical-test-target-directory"),
        target_directory_history_entry_hash: test_hash("canonical-test-target-history"),
        target_assignment_root: test_hash("canonical-test-target-assignments"),
        target_placement_root: test_hash("canonical-test-target-placements"),
        target_cells_root: String::new(),
        target_cells: vec![target],
        target_global_conservation_root: conservation_root,
        target_normalized_gameplay_root: gameplay_root,
        receipt_hash: String::new(),
    };
    document.target_cells[0].target_lifecycle_record_hash =
        Protocol19TargetLifecycleGenesisV2::new(
            &document.anchor,
            document.target_directory_revision,
            &document.target_directory_document_hash,
            &document.source_cells[0],
            &document.target_cells[0],
        )
        .expect("canonical test lifecycle derives")
        .record_hash()
        .to_owned();
    document.target_cells_root =
        hash_target_cells(&document.target_cells).expect("target cells hash");
    document.receipt_hash = calculate_receipt_hash(&document).expect("receipt hashes");
    encode_canonical(&CanonicalProtocol18To19MigrationReceipt { document })
        .expect("canonical test receipt encodes")
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum MigrationReceiptError {
    #[error("migration receipt is invalid: {0}")]
    Invalid(String),
    #[error("migration receipt JSON is invalid: {0}")]
    Json(String),
    #[error("migration receipt exceeds its byte bound")]
    TooLarge,
}

fn encode_canonical(
    receipt: &CanonicalProtocol18To19MigrationReceipt,
) -> Result<Vec<u8>, MigrationReceiptError> {
    validate_receipt(receipt.document())?;
    let bytes = serde_json::to_vec(receipt.document())
        .map_err(|source| MigrationReceiptError::Json(source.to_string()))?;
    if bytes.len() > MAX_MIGRATION_RECEIPT_BYTES {
        return Err(MigrationReceiptError::TooLarge);
    }
    Ok(bytes)
}

fn decode_canonical(
    bytes: &[u8],
) -> Result<CanonicalProtocol18To19MigrationReceipt, MigrationReceiptError> {
    if bytes.is_empty() || bytes.len() > MAX_MIGRATION_RECEIPT_BYTES {
        return Err(MigrationReceiptError::TooLarge);
    }
    let document = serde_json::from_slice::<Protocol18To19MigrationReceiptV1>(bytes)
        .map_err(|source| MigrationReceiptError::Json(source.to_string()))?;
    validate_receipt(&document)?;
    let canonical = serde_json::to_vec(&document)
        .map_err(|source| MigrationReceiptError::Json(source.to_string()))?;
    if canonical != bytes {
        return Err(MigrationReceiptError::Invalid(
            "bytes are not the exact canonical encoding".into(),
        ));
    }
    Ok(CanonicalProtocol18To19MigrationReceipt { document })
}

fn validate_receipt(
    receipt: &Protocol18To19MigrationReceiptV1,
) -> Result<(), MigrationReceiptError> {
    validate_anchor(&receipt.anchor)?;
    if receipt.schema_version != MIGRATION_SCHEMA_VERSION
        || receipt.target_directory_revision != 1
        || receipt.source_cells.is_empty()
        || receipt.source_cells.len() > MAX_MIGRATION_CELLS
        || receipt.source_cells.len() != receipt.target_cells.len()
        || usize::try_from(receipt.anchor.source_cell_count).ok()
            != Some(receipt.source_cells.len())
        || !all_hashes([
            &receipt.source_directory_archive_hash,
            &receipt.identity_map_blob_hash,
            &receipt.production_origin_blob_hash,
            &receipt.target_directory_document_hash,
            &receipt.target_directory_history_entry_hash,
            &receipt.target_assignment_root,
            &receipt.target_placement_root,
            &receipt.target_cells_root,
            &receipt.target_global_conservation_root,
            &receipt.target_normalized_gameplay_root,
            &receipt.receipt_hash,
        ])
        || receipt.anchor.identity_map_root != receipt.identity_map_blob_hash
        || receipt.anchor.production_origin_root != receipt.production_origin_blob_hash
        || receipt.anchor.source_global_conservation_root != receipt.target_global_conservation_root
        || receipt.anchor.normalized_gameplay_root != receipt.target_normalized_gameplay_root
        || receipt.anchor.source_cells_root != hash_source_cells(&receipt.source_cells)?
        || receipt.target_cells_root != hash_target_cells(&receipt.target_cells)?
    {
        return Err(MigrationReceiptError::Invalid(
            "receipt header, roots, counts, or conservation bridge is invalid".into(),
        ));
    }

    let mut prior_source = None;
    let mut prior_target = None;
    for (source, target) in receipt.source_cells.iter().zip(&receipt.target_cells) {
        validate_source_cell(source, &receipt.anchor.universe_id)?;
        validate_target_cell(target, &receipt.anchor)?;
        if source.last_trusted_unix_ms > receipt.anchor.trusted_cutoff_unix_ms {
            return Err(MigrationReceiptError::Invalid(
                "source trusted time exceeds the migration cutoff".into(),
            ));
        }
        let expected_lifecycle = Protocol19TargetLifecycleGenesisV2::new(
            &receipt.anchor,
            receipt.target_directory_revision,
            &receipt.target_directory_document_hash,
            source,
            target,
        )?;
        if prior_source
            .as_ref()
            .is_some_and(|prior| prior >= &source.cell_id)
            || prior_target
                .as_ref()
                .is_some_and(|prior| prior >= &target.cell_id)
            || source.cell_key != target.cell_key
            || source.cell_id != target.cell_id
            || source.source_event_schema_version != crate::event::EVENT_SCHEMA_VERSION
            || target.legacy_event_schema_version != crate::event::EVENT_SCHEMA_VERSION
            || target.legacy_event_sequence != source.source_event_sequence
            || target.legacy_event_head_hash != source.source_event_head_hash
            || target.event17_genesis_sequence != source.source_event_sequence
            || target.event17_predecessor_hash != source.source_event_head_hash
            || target.target_lifecycle_record_hash != expected_lifecycle.record_hash()
        {
            return Err(MigrationReceiptError::Invalid(
                "source and target cells do not form one ordered event-era and lifecycle bridge"
                    .into(),
            ));
        }
        prior_source = Some(source.cell_id.clone());
        prior_target = Some(target.cell_id.clone());
    }
    if receipt.receipt_hash != calculate_receipt_hash(receipt)? {
        return Err(MigrationReceiptError::Invalid(
            "receipt hash does not commit the complete migration".into(),
        ));
    }
    Ok(())
}

fn validate_anchor(anchor: &Protocol18To19MigrationAnchorV1) -> Result<(), MigrationReceiptError> {
    let seed = anchor
        .world_seed
        .parse::<u64>()
        .map_err(|_| MigrationReceiptError::Invalid("world seed is not canonical u64".into()))?;
    let registry = celestial::registry_snapshot(seed)
        .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
    let source_manifest = celestial::universe_manifest(
        seed,
        crate::model::WORLD_SCHEMA_VERSION,
        crate::event::EVENT_SCHEMA_VERSION,
    )
    .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
    let target_manifest = crate::manifest_v5::build_validated_manifest_v5(seed)
        .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
    if seed.to_string() != anchor.world_seed
        || anchor.schema_version != MIGRATION_SCHEMA_VERSION
        || anchor.migration_kind != MIGRATION_KIND
        || anchor.source_compatibility != Protocol18CompatibilityTuple::canonical()
        || anchor.target_compatibility != Protocol19CompatibilityTuple::canonical()
        || anchor.universe_id != registry.universe_id
        || anchor.source_manifest_hash != source_manifest.manifest_hash
        || anchor.target_manifest_hash != target_manifest.manifest_hash()
        || anchor.trusted_cutoff_unix_ms == 0
        || anchor.source_directory_revision == 0
        || anchor.source_cell_count == 0
        || anchor.source_manifest_hash == anchor.target_manifest_hash
        || !all_hashes([
            &anchor.source_manifest_hash,
            &anchor.target_manifest_hash,
            &anchor.source_directory_document_hash,
            &anchor.source_terminal_transfer_root,
            &anchor.source_assignment_root,
            &anchor.source_placement_root,
            &anchor.source_cells_root,
            &anchor.identity_map_root,
            &anchor.production_origin_root,
            &anchor.source_global_conservation_root,
            &anchor.normalized_gameplay_root,
            &anchor.anchor_hash,
        ])
        || anchor.anchor_hash != calculate_anchor_hash(anchor)?
    {
        return Err(MigrationReceiptError::Invalid(
            "migration anchor is not canonical protocol-18 to protocol-19 material".into(),
        ));
    }
    Ok(())
}

fn validate_source_cell(
    cell: &TerminalCellV18Commitment,
    universe_id: &str,
) -> Result<(), MigrationReceiptError> {
    let expected_cell_id = celestial::cell_id(&cell.cell_key)
        .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
    let empty_event_frontier = cell.source_event_sequence == 0
        && cell.source_event_head_hash.is_empty()
        && cell.source_event_archive_entry_count == 0
        && cell.source_event_archive_root.is_empty();
    let populated_event_frontier = cell.source_event_sequence > 0
        && valid_hash(&cell.source_event_head_hash)
        && cell.source_event_archive_entry_count > 0
        && valid_hash(&cell.source_event_archive_root);
    let empty_boundary_frontier = cell.source_transfer_boundary_entry_count == 0
        && cell.source_transfer_boundary_head_hash.is_empty()
        && cell.source_transfer_boundary_archive_root.is_empty();
    let populated_boundary_frontier = cell.source_transfer_boundary_entry_count > 0
        && valid_hash(&cell.source_transfer_boundary_head_hash)
        && valid_hash(&cell.source_transfer_boundary_archive_root);
    if cell.cell_key.universe_id != universe_id
        || cell.cell_id != expected_cell_id
        || cell.assignment_generation == 0
        || cell.authority_fencing_token == 0
        || cell.source_event_schema_version != crate::event::EVENT_SCHEMA_VERSION
        || cell.source_lifecycle_revision == 0
        || cell.last_trusted_unix_ms == 0
        || !(empty_event_frontier || populated_event_frontier)
        || !(empty_boundary_frontier || populated_boundary_frontier)
        || !all_hashes([
            &cell.fencing_history_root,
            &cell.source_world_state_hash,
            &cell.source_snapshot_document_hash,
            &cell.source_lifecycle_record_hash,
            &cell.next_production_occurrence_root,
        ])
    {
        return Err(MigrationReceiptError::Invalid(
            "terminal protocol-18 cell commitment is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_target_cell(
    cell: &World21GenesisCommitment,
    anchor: &Protocol18To19MigrationAnchorV1,
) -> Result<(), MigrationReceiptError> {
    let expected_cell_id = celestial::cell_id(&cell.cell_key)
        .map_err(|source| MigrationReceiptError::Invalid(source.to_string()))?;
    let empty_legacy = cell.legacy_event_sequence == 0
        && cell.legacy_event_head_hash.is_empty()
        && cell.event17_predecessor_hash.is_empty();
    let populated_legacy = cell.legacy_event_sequence > 0
        && valid_hash(&cell.legacy_event_head_hash)
        && cell.event17_predecessor_hash == cell.legacy_event_head_hash;
    if cell.cell_key.universe_id != anchor.universe_id
        || cell.cell_id != expected_cell_id
        || cell.migration_anchor_hash != anchor.anchor_hash
        || cell.legacy_event_schema_version != crate::event::EVENT_SCHEMA_VERSION
        || cell.event17_genesis_sequence != cell.legacy_event_sequence
        || cell.event17_journal_entry_count != 0
        || !cell.event17_journal_head_hash.is_empty()
        || !(empty_legacy || populated_legacy)
        || !all_hashes([
            &cell.migration_anchor_hash,
            &cell.target_world_state_hash,
            &cell.target_active_world_hash,
            &cell.target_lifecycle_record_hash,
            &cell.production_origin_root,
            &cell.identity_subset_root,
        ])
    {
        return Err(MigrationReceiptError::Invalid(
            "world-21 genesis cell commitment is invalid".into(),
        ));
    }
    Ok(())
}

fn calculate_anchor_hash(
    anchor: &Protocol18To19MigrationAnchorV1,
) -> Result<String, MigrationReceiptError> {
    let mut material = anchor.clone();
    material.anchor_hash.clear();
    hash_json(MIGRATION_ANCHOR_HASH_DOMAIN, &material)
}

fn calculate_receipt_hash(
    receipt: &Protocol18To19MigrationReceiptV1,
) -> Result<String, MigrationReceiptError> {
    let mut material = receipt.clone();
    material.receipt_hash.clear();
    hash_json(MIGRATION_RECEIPT_HASH_DOMAIN, &material)
}

fn hash_source_cells(cells: &[TerminalCellV18Commitment]) -> Result<String, MigrationReceiptError> {
    hash_json(SOURCE_CELL_SET_HASH_DOMAIN, &cells)
}

fn hash_target_cells(cells: &[World21GenesisCommitment]) -> Result<String, MigrationReceiptError> {
    hash_json(TARGET_CELL_SET_HASH_DOMAIN, &cells)
}

fn hash_json<T: Serialize>(domain: &[u8], value: &T) -> Result<String, MigrationReceiptError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|source| MigrationReceiptError::Json(source.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn hash_bytes(domain: &[u8], bytes: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().to_hex().to_string()
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn all_hashes<'a>(values: impl IntoIterator<Item = &'a String>) -> bool {
    values.into_iter().all(|value| valid_hash(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(label: &str) -> String {
        blake3::hash(label.as_bytes()).to_hex().to_string()
    }

    fn fixture() -> CanonicalProtocol18To19MigrationReceipt {
        let seed = 801;
        let source_manifest = celestial::universe_manifest(
            seed,
            crate::model::WORLD_SCHEMA_VERSION,
            crate::event::EVENT_SCHEMA_VERSION,
        )
        .expect("source manifest builds");
        let target_manifest =
            crate::manifest_v5::build_validated_manifest_v5(seed).expect("target manifest builds");
        let source_key = celestial::cell_origin_key();
        let target_key =
            celestial::neighbor_cell_key(&source_key, [1, 0, 0]).expect("neighbor derives");
        let mut source_cells = [source_key, target_key]
            .into_iter()
            .enumerate()
            .map(|(index, cell_key)| TerminalCellV18Commitment {
                cell_id: celestial::cell_id(&cell_key).expect("cell ID derives"),
                cell_key,
                assignment_generation: u64::try_from(index + 1).expect("small generation"),
                authority_fencing_token: u64::try_from(index + 11).expect("small fence"),
                fencing_history_root: hash(&format!("fencing-{index}")),
                source_world_state_hash: hash(&format!("source-world-{index}")),
                source_snapshot_document_hash: hash(&format!("snapshot-{index}")),
                source_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
                source_event_sequence: u64::try_from(index + 41).expect("small event sequence"),
                source_event_head_hash: hash(&format!("event-head-{index}")),
                source_event_archive_entry_count: u64::try_from(index + 41)
                    .expect("small archive count"),
                source_event_archive_root: hash(&format!("event-archive-{index}")),
                source_lifecycle_revision: u64::try_from(index + 4)
                    .expect("small lifecycle revision"),
                source_lifecycle_record_hash: hash(&format!("lifecycle-{index}")),
                source_transfer_boundary_entry_count: 1,
                source_transfer_boundary_head_hash: hash(&format!("boundary-head-{index}")),
                source_transfer_boundary_archive_root: hash(&format!("boundary-root-{index}")),
                acknowledged_production_sequence: u64::try_from(index)
                    .expect("small production sequence"),
                next_production_occurrence_root: hash(&format!("next-production-{index}")),
                last_trusted_unix_ms: 1_800_000_000_000
                    + u64::try_from(index).expect("small time offset"),
            })
            .collect::<Vec<_>>();
        source_cells.sort_by(|left, right| left.cell_id.cmp(&right.cell_id));
        let source_cells_root = hash_source_cells(&source_cells).expect("source cells hash");
        let conservation_root = hash("global-conservation");
        let normalized_gameplay_root = hash("normalized-gameplay");
        let identity_map_root = hash("identity-map");
        let production_origin_root = hash("production-origins");
        let mut anchor = Protocol18To19MigrationAnchorV1 {
            schema_version: MIGRATION_SCHEMA_VERSION,
            migration_kind: MIGRATION_KIND.into(),
            source_compatibility: Protocol18CompatibilityTuple::canonical(),
            target_compatibility: Protocol19CompatibilityTuple::canonical(),
            universe_id: source_manifest.universe_id,
            world_seed: seed.to_string(),
            trusted_cutoff_unix_ms: 1_800_000_000_010,
            source_manifest_hash: source_manifest.manifest_hash,
            target_manifest_hash: target_manifest.manifest_hash().to_owned(),
            source_directory_revision: 9,
            source_directory_document_hash: hash("source-directory"),
            source_terminal_transfer_count: 1,
            source_terminal_transfer_root: hash("source-terminal-transfers"),
            source_assignment_root: hash("source-assignments"),
            source_placement_root: hash("source-placements"),
            source_cells_root,
            source_cell_count: u64::try_from(source_cells.len()).expect("small cell count"),
            identity_map_entry_count: 3,
            identity_map_root: identity_map_root.clone(),
            production_origin_count: 2,
            production_origin_root: production_origin_root.clone(),
            source_global_conservation_root: conservation_root.clone(),
            normalized_gameplay_root: normalized_gameplay_root.clone(),
            anchor_hash: String::new(),
        };
        anchor.anchor_hash = calculate_anchor_hash(&anchor).expect("anchor hashes");
        let target_cells = source_cells
            .iter()
            .enumerate()
            .map(|(index, source)| World21GenesisCommitment {
                cell_key: source.cell_key.clone(),
                cell_id: source.cell_id.clone(),
                migration_anchor_hash: anchor.anchor_hash.clone(),
                target_world_state_hash: hash(&format!("target-world-{index}")),
                target_active_world_hash: hash(&format!("target-active-{index}")),
                target_lifecycle_record_hash: String::new(),
                legacy_event_schema_version: crate::event::EVENT_SCHEMA_VERSION,
                legacy_event_sequence: source.source_event_sequence,
                legacy_event_head_hash: source.source_event_head_hash.clone(),
                event17_genesis_sequence: source.source_event_sequence,
                event17_predecessor_hash: source.source_event_head_hash.clone(),
                event17_journal_entry_count: 0,
                event17_journal_head_hash: String::new(),
                production_origin_root: hash(&format!("cell-production-{index}")),
                identity_subset_root: hash(&format!("cell-identities-{index}")),
            })
            .collect::<Vec<_>>();
        let mut document = Protocol18To19MigrationReceiptV1 {
            schema_version: MIGRATION_SCHEMA_VERSION,
            anchor,
            source_directory_archive_hash: hash("source-directory-archive"),
            source_cells,
            identity_map_blob_hash: identity_map_root,
            production_origin_blob_hash: production_origin_root,
            target_directory_revision: 1,
            target_directory_document_hash: hash("target-directory"),
            target_directory_history_entry_hash: hash("target-directory-history"),
            target_assignment_root: hash("target-assignments"),
            target_placement_root: hash("target-placements"),
            target_cells_root: String::new(),
            target_cells,
            target_global_conservation_root: conservation_root,
            target_normalized_gameplay_root: normalized_gameplay_root,
            receipt_hash: String::new(),
        };
        for (source, target) in document.source_cells.iter().zip(&mut document.target_cells) {
            target.target_lifecycle_record_hash = Protocol19TargetLifecycleGenesisV2::new(
                &document.anchor,
                document.target_directory_revision,
                &document.target_directory_document_hash,
                source,
                target,
            )
            .expect("target lifecycle derives")
            .record_hash()
            .to_owned();
        }
        document.target_cells_root =
            hash_target_cells(&document.target_cells).expect("target cells hash");
        document.receipt_hash = calculate_receipt_hash(&document).expect("receipt hashes");
        validate_receipt(&document).expect("fixture validates");
        CanonicalProtocol18To19MigrationReceipt { document }
    }

    fn reseal(receipt: &mut Protocol18To19MigrationReceiptV1) {
        receipt.anchor.source_cells_root =
            hash_source_cells(&receipt.source_cells).expect("source cells hash");
        receipt.anchor.anchor_hash = calculate_anchor_hash(&receipt.anchor).expect("anchor hashes");
        for target in &mut receipt.target_cells {
            target.migration_anchor_hash = receipt.anchor.anchor_hash.clone();
        }
        receipt.target_cells_root =
            hash_target_cells(&receipt.target_cells).expect("target cells hash");
        receipt.receipt_hash = calculate_receipt_hash(receipt).expect("receipt hashes");
    }

    #[test]
    fn migration_receipt_is_canonical_and_preserves_event16_frontiers() {
        let receipt = fixture();
        let bytes = encode_canonical(&receipt).expect("receipt encodes");
        let reopened = decode_canonical(&bytes).expect("receipt reopens");
        assert_eq!(reopened.document(), receipt.document());
        assert_eq!(
            reopened.document().receipt_hash,
            "a8ba3ba76658dddd74c5b4d0458706322b7ecb9fa90d69513eaf71f132fcaa34"
        );
        for (source, target) in reopened
            .document()
            .source_cells
            .iter()
            .zip(&reopened.document().target_cells)
        {
            assert_eq!(target.legacy_event_sequence, source.source_event_sequence);
            assert_eq!(target.legacy_event_head_hash, source.source_event_head_hash);
            assert_eq!(
                target.event17_genesis_sequence,
                source.source_event_sequence
            );
            assert_eq!(
                target.event17_predecessor_hash,
                source.source_event_head_hash
            );
            assert_eq!(target.event17_journal_entry_count, 0);
        }

        let mut whitespace = vec![b' '];
        whitespace.extend_from_slice(&bytes);
        assert!(decode_canonical(&whitespace).is_err());
        let mut unknown = serde_json::from_slice::<serde_json::Value>(&bytes).expect("JSON parses");
        unknown["unknown"] = serde_json::json!(true);
        assert!(decode_canonical(&serde_json::to_vec(&unknown).expect("JSON encodes")).is_err());
    }

    #[test]
    fn target_lifecycle_genesis_is_canonical_and_receipt_derived() {
        let receipt = fixture();
        let document = receipt.document();
        let lifecycle = Protocol19TargetLifecycleGenesisV2::new(
            &document.anchor,
            document.target_directory_revision,
            &document.target_directory_document_hash,
            &document.source_cells[0],
            &document.target_cells[0],
        )
        .expect("target lifecycle derives");
        assert_eq!(
            lifecycle.record_hash(),
            document.target_cells[0].target_lifecycle_record_hash
        );
        let bytes = lifecycle.encode_canonical().expect("lifecycle encodes");
        assert_eq!(
            decode_target_lifecycle_genesis(&bytes).expect("lifecycle reopens"),
            lifecycle
        );

        let mut whitespace = vec![b' '];
        whitespace.extend_from_slice(&bytes);
        assert!(decode_target_lifecycle_genesis(&whitespace).is_err());
        let mut tampered =
            serde_json::from_slice::<serde_json::Value>(&bytes).expect("JSON parses");
        tampered["authority_fencing_token"] = serde_json::json!(99);
        assert!(
            decode_target_lifecycle_genesis(
                &serde_json::to_vec(&tampered).expect("tampered JSON encodes")
            )
            .is_err()
        );
    }

    #[test]
    fn staging_commitment_binds_one_exact_receipt_cell_without_activation_authority() {
        let receipt = fixture();
        let bytes = encode_canonical(&receipt).expect("receipt encodes");
        let document = receipt.document();
        let source = &document.source_cells[0];
        let target = &document.target_cells[0];
        let evidence = Protocol19World21StagingEvidence {
            manifest_hash: &document.anchor.target_manifest_hash,
            universe_id: &document.anchor.universe_id,
            world_seed: document.anchor.world_seed.parse().expect("seed parses"),
            cell_key: &target.cell_key,
            cell_id: &target.cell_id,
            authority_fencing_token: source.authority_fencing_token,
            snapshot_state_hash: &target.target_world_state_hash,
            active_world_hash: &target.target_active_world_hash,
            legacy_event_sequence: target.legacy_event_sequence,
            legacy_event_head_hash: &target.legacy_event_head_hash,
        };
        let staged =
            bind_world21_staging_target(&bytes, &evidence).expect("exact staging target binds");
        assert_eq!(
            staged.lifecycle().record_hash(),
            target.target_lifecycle_record_hash
        );
        assert_eq!(staged.migration_receipt_hash(), document.receipt_hash);
        assert_eq!(
            staged.production_origin_root(),
            target.production_origin_root
        );
        assert_eq!(staged.identity_subset_root(), target.identity_subset_root);

        let wrong_active = hash("wrong-active-world");
        let wrong = Protocol19World21StagingEvidence {
            active_world_hash: &wrong_active,
            ..evidence
        };
        assert!(bind_world21_staging_target(&bytes, &wrong).is_err());
    }

    #[test]
    fn migration_receipt_rejects_resealed_frontier_and_conservation_substitution() {
        let mut wrong_frontier = fixture().document;
        wrong_frontier.target_cells[0].legacy_event_sequence += 1;
        reseal(&mut wrong_frontier);
        assert!(validate_receipt(&wrong_frontier).is_err());

        let mut reset_frontier = fixture().document;
        reset_frontier.target_cells[0].legacy_event_sequence = 0;
        reset_frontier.target_cells[0]
            .legacy_event_head_hash
            .clear();
        reset_frontier.target_cells[0].event17_genesis_sequence = 0;
        reset_frontier.target_cells[0]
            .event17_predecessor_hash
            .clear();
        reseal(&mut reset_frontier);
        assert!(validate_receipt(&reset_frontier).is_err());

        let mut changed_conservation = fixture().document;
        changed_conservation.target_global_conservation_root = hash("invented-conservation");
        reseal(&mut changed_conservation);
        assert!(validate_receipt(&changed_conservation).is_err());

        let mut changed_tuple = fixture().document;
        changed_tuple
            .anchor
            .target_compatibility
            .event_schema_version += 1;
        reseal(&mut changed_tuple);
        assert!(validate_receipt(&changed_tuple).is_err());

        let mut changed_manifests = fixture().document;
        changed_manifests.anchor.source_manifest_hash = hash("invented-source-manifest");
        changed_manifests.anchor.target_manifest_hash = hash("invented-target-manifest");
        reseal(&mut changed_manifests);
        assert!(validate_receipt(&changed_manifests).is_err());
    }

    #[test]
    fn migration_receipt_rejects_source_trusted_time_after_resealed_cutoff() {
        let mut regressed_cutoff = fixture().document;
        regressed_cutoff.anchor.trusted_cutoff_unix_ms =
            regressed_cutoff.source_cells[0].last_trusted_unix_ms - 1;
        reseal(&mut regressed_cutoff);

        assert!(matches!(
            validate_receipt(&regressed_cutoff),
            Err(MigrationReceiptError::Invalid(message))
                if message == "source trusted time exceeds the migration cutoff"
        ));
    }
}
