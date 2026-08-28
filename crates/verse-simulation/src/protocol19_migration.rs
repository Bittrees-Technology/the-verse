// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant canonical protocol-18 to protocol-19 migration receipt.
//!
//! This codec is not an install authority. It commits an offline, terminal
//! protocol-18 universe and its world-21 genesis without rewriting event-16
//! history. A future installer must additionally prove live locks, archives,
//! signatures, and every referenced root before it can mint an activation
//! permit.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verse_protocol::CellKeyV1;
use verse_protocol::protocol_v19::Protocol19CompatibilityTuple;

use crate::{celestial, content};

const MIGRATION_SCHEMA_VERSION: u32 = 1;
const MIGRATION_KIND: &str =
    "protocol18-world20-event16-directory2-to-protocol19-world21-event17-manifest5";
const MIGRATION_ANCHOR_HASH_DOMAIN: &[u8] = b"the-verse/protocol-18-to-19-migration-anchor/v1\0";
const MIGRATION_RECEIPT_HASH_DOMAIN: &[u8] = b"the-verse/protocol-18-to-19-migration-receipt/v1\0";
const SOURCE_CELL_SET_HASH_DOMAIN: &[u8] = b"the-verse/protocol-18-terminal-cell-set/v1\0";
const TARGET_CELL_SET_HASH_DOMAIN: &[u8] = b"the-verse/protocol-19-genesis-cell-set/v1\0";
const MAX_MIGRATION_RECEIPT_BYTES: usize = 16 * 1_024 * 1_024;
const MAX_MIGRATION_CELLS: usize = 4_096;

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

#[derive(Debug, Clone, PartialEq, Eq, Error)]
enum MigrationReceiptError {
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
        {
            return Err(MigrationReceiptError::Invalid(
                "source and target cells do not form one ordered event-era bridge".into(),
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
            trusted_cutoff_unix_ms: 1_800_000_000_000,
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
                target_lifecycle_record_hash: hash(&format!("target-lifecycle-{index}")),
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
            target_cells_root: hash_target_cells(&target_cells).expect("target cells hash"),
            target_cells,
            target_global_conservation_root: conservation_root,
            target_normalized_gameplay_root: normalized_gameplay_root,
            receipt_hash: String::new(),
        };
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
            "71652ca3db8c41d9321d71d4ac6c4b82340b5da52ae82a77a51d4f1355d67aae"
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
}
