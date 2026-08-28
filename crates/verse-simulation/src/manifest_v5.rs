// SPDX-License-Identifier: AGPL-3.0-or-later

//! Dormant manifest-5 builder and opaque simulation-wide validation capability.
//!
//! The active manifest generator remains schema 4. This module has no runtime
//! caller and cannot activate protocol 19; it only establishes the exact
//! identity object the future world-21 Store must require.

use serde::Serialize;
use thiserror::Error;
use verse_protocol::protocol_v19::{
    Protocol19CompatibilityTuple, UNIVERSE_MANIFEST_SCHEMA_VERSION, UniverseManifestSnapshotV5,
};

use crate::{celestial, content};

const MANIFEST_HASH_DOMAIN_V5: &[u8] = b"the-verse/universe-manifest/v5\0";
const MAX_MANIFEST_V5_BYTES: usize = 64 * 1_024;
const FRONTIER_POLICY_VERSION_V5: &str = "closed-proof-frontier-v1";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(super) enum DraftManifestV5Error {
    #[error("manifest-5 material is invalid: {0}")]
    Invalid(String),
    #[error("manifest-5 JSON is invalid: {0}")]
    Json(String),
    #[error("manifest-5 exceeds its byte bound")]
    TooLarge,
}

/// Non-Serde capability proving the complete manifest-5 document was rebuilt
/// and validated against this binary's immutable content and registry rules.
#[derive(Debug)]
pub(crate) struct ValidatedUniverseManifestV5 {
    document: UniverseManifestSnapshotV5,
}

impl ValidatedUniverseManifestV5 {
    pub(crate) fn document(&self) -> &UniverseManifestSnapshotV5 {
        &self.document
    }

    pub(crate) fn manifest_hash(&self) -> &str {
        &self.document.manifest_hash
    }

    pub(crate) fn universe_id(&self) -> &str {
        &self.document.universe_id
    }

    pub(crate) fn world_seed(&self) -> u64 {
        self.document
            .world_seed
            .parse()
            .expect("validated manifest-5 seed is canonical u64")
    }
}

#[derive(Serialize)]
struct ManifestHashMaterialV5<'a> {
    schema_version: u32,
    compatibility: &'a Protocol19CompatibilityTuple,
    universe_id: &'a str,
    world_seed: &'a str,
    address_schema_version: u32,
    sector_edge_um: u64,
    cell_edge_um: u64,
    cells_per_sector_axis: u32,
    generation_rule_version: &'a str,
    frontier_policy_version: &'a str,
    celestial_registry_hash: &'a str,
    content_hash: &'a str,
    lifecycle_policy_hash: &'a str,
}

pub(super) fn build_validated_manifest_v5(
    world_seed: u64,
) -> Result<ValidatedUniverseManifestV5, DraftManifestV5Error> {
    let registry = celestial::registry_snapshot(world_seed)
        .map_err(|source| DraftManifestV5Error::Invalid(source.to_string()))?;
    let mut document = UniverseManifestSnapshotV5 {
        schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
        manifest_hash: String::new(),
        compatibility: Protocol19CompatibilityTuple::canonical(),
        universe_id: registry.universe_id.clone(),
        world_seed: world_seed.to_string(),
        address_schema_version: celestial::ADDRESS_SCHEMA_VERSION,
        sector_edge_um: celestial::SECTOR_EDGE_UM,
        cell_edge_um: celestial::CELL_EDGE_UM,
        cells_per_sector_axis: celestial::CELLS_PER_SECTOR_AXIS,
        generation_rule_version: registry.generation_rule_version,
        frontier_policy_version: FRONTIER_POLICY_VERSION_V5.into(),
        celestial_registry_hash: registry.registry_hash,
        content_hash: content::manifest_hash().into(),
        lifecycle_policy_hash: celestial::lifecycle_policy_hash()
            .map_err(|source| DraftManifestV5Error::Invalid(source.to_string()))?,
    };
    document.manifest_hash = calculate_manifest_hash_v5(&document)?;
    validate_manifest_v5(&document, world_seed)?;
    Ok(ValidatedUniverseManifestV5 { document })
}

pub(super) fn encode_manifest_v5(
    capability: &ValidatedUniverseManifestV5,
) -> Result<Vec<u8>, DraftManifestV5Error> {
    validate_manifest_v5(capability.document(), capability.world_seed())?;
    let bytes = serde_json::to_vec(capability.document())
        .map_err(|source| DraftManifestV5Error::Json(source.to_string()))?;
    if bytes.len() > MAX_MANIFEST_V5_BYTES {
        return Err(DraftManifestV5Error::TooLarge);
    }
    Ok(bytes)
}

pub(super) fn decode_manifest_v5(
    bytes: &[u8],
    expected_world_seed: u64,
) -> Result<ValidatedUniverseManifestV5, DraftManifestV5Error> {
    if bytes.is_empty() || bytes.len() > MAX_MANIFEST_V5_BYTES {
        return Err(DraftManifestV5Error::TooLarge);
    }
    let document = serde_json::from_slice::<UniverseManifestSnapshotV5>(bytes)
        .map_err(|source| DraftManifestV5Error::Json(source.to_string()))?;
    validate_manifest_v5(&document, expected_world_seed)?;
    let canonical = serde_json::to_vec(&document)
        .map_err(|source| DraftManifestV5Error::Json(source.to_string()))?;
    if canonical != bytes {
        return Err(DraftManifestV5Error::Invalid(
            "bytes are not the exact canonical encoding".into(),
        ));
    }
    Ok(ValidatedUniverseManifestV5 { document })
}

fn calculate_manifest_hash_v5(
    document: &UniverseManifestSnapshotV5,
) -> Result<String, DraftManifestV5Error> {
    let material = ManifestHashMaterialV5 {
        schema_version: document.schema_version,
        compatibility: &document.compatibility,
        universe_id: &document.universe_id,
        world_seed: &document.world_seed,
        address_schema_version: document.address_schema_version,
        sector_edge_um: document.sector_edge_um,
        cell_edge_um: document.cell_edge_um,
        cells_per_sector_axis: document.cells_per_sector_axis,
        generation_rule_version: &document.generation_rule_version,
        frontier_policy_version: &document.frontier_policy_version,
        celestial_registry_hash: &document.celestial_registry_hash,
        content_hash: &document.content_hash,
        lifecycle_policy_hash: &document.lifecycle_policy_hash,
    };
    let bytes = serde_json::to_vec(&material)
        .map_err(|source| DraftManifestV5Error::Json(source.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(MANIFEST_HASH_DOMAIN_V5);
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn validate_manifest_v5(
    document: &UniverseManifestSnapshotV5,
    expected_world_seed: u64,
) -> Result<(), DraftManifestV5Error> {
    let registry = celestial::registry_snapshot(expected_world_seed)
        .map_err(|source| DraftManifestV5Error::Invalid(source.to_string()))?;
    let content = content::manifest();
    let lifecycle_policy_hash = celestial::lifecycle_policy_hash()
        .map_err(|source| DraftManifestV5Error::Invalid(source.to_string()))?;
    if document.schema_version != UNIVERSE_MANIFEST_SCHEMA_VERSION
        || document.compatibility != Protocol19CompatibilityTuple::canonical()
        || document.universe_id != registry.universe_id
        || document.world_seed != expected_world_seed.to_string()
        || document.address_schema_version != celestial::ADDRESS_SCHEMA_VERSION
        || document.sector_edge_um != celestial::SECTOR_EDGE_UM
        || document.cell_edge_um != celestial::CELL_EDGE_UM
        || document.cells_per_sector_axis != celestial::CELLS_PER_SECTOR_AXIS
        || document.generation_rule_version != registry.generation_rule_version
        || document.frontier_policy_version != FRONTIER_POLICY_VERSION_V5
        || document.celestial_registry_hash != registry.registry_hash
        || document.compatibility.content_schema_version != content.schema_version
        || document.compatibility.content_manifest_version != content.manifest_version
        || document.content_hash != content::manifest_hash()
        || document.lifecycle_policy_hash != lifecycle_policy_hash
        || !valid_blake3_hex(&document.manifest_hash)
        || !valid_blake3_hex(&document.celestial_registry_hash)
        || !valid_blake3_hex(&document.content_hash)
        || !valid_blake3_hex(&document.lifecycle_policy_hash)
        || document.manifest_hash != calculate_manifest_hash_v5(document)?
    {
        return Err(DraftManifestV5Error::Invalid(
            "document does not match the canonical protocol-19 universe".into(),
        ));
    }
    Ok(())
}

fn valid_blake3_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reseal(document: &mut UniverseManifestSnapshotV5) {
        document.manifest_hash = calculate_manifest_hash_v5(document).expect("manifest rehashes");
    }

    #[test]
    fn manifest_v5_is_deterministic_domain_separated_and_canonical() {
        let validated = build_validated_manifest_v5(801).expect("manifest builds");
        let bytes = encode_manifest_v5(&validated).expect("manifest encodes");
        let reopened = decode_manifest_v5(&bytes, 801).expect("manifest reopens");
        assert_eq!(reopened.document(), validated.document());
        assert_eq!(reopened.world_seed(), 801);
        assert_eq!(reopened.universe_id(), validated.universe_id());
        assert_eq!(
            reopened.manifest_hash(),
            "ef2397a896cfd81e22e5cb341ea7649c511a5ff6cd0143709f40404f823decc2"
        );
        assert_eq!(
            reopened.manifest_hash(),
            build_validated_manifest_v5(801)
                .expect("manifest rebuilds")
                .manifest_hash()
        );
        let active = celestial::universe_manifest(
            801,
            crate::model::WORLD_SCHEMA_VERSION,
            crate::event::EVENT_SCHEMA_VERSION,
        )
        .expect("active manifest builds");
        assert_ne!(reopened.manifest_hash(), active.manifest_hash);

        let mut whitespace = vec![b' '];
        whitespace.extend_from_slice(&bytes);
        assert!(decode_manifest_v5(&whitespace, 801).is_err());
        let mut unknown: serde_json::Value = serde_json::from_slice(&bytes).expect("JSON parses");
        unknown["unknown"] = serde_json::json!(true);
        assert!(
            decode_manifest_v5(&serde_json::to_vec(&unknown).expect("JSON encodes"), 801).is_err()
        );
    }

    #[test]
    fn manifest_v5_rejects_every_rehashed_tuple_substitution() {
        let original = build_validated_manifest_v5(801)
            .expect("manifest builds")
            .document;
        macro_rules! reject_increment {
            ($field:ident) => {{
                let mut changed = original.clone();
                changed.compatibility.$field += 1;
                reseal(&mut changed);
                assert!(validate_manifest_v5(&changed, 801).is_err());
            }};
        }
        reject_increment!(protocol_version);
        reject_increment!(projection_schema_version);
        reject_increment!(world_schema_version);
        reject_increment!(event_schema_version);
        reject_increment!(content_schema_version);
        reject_increment!(celestial_registry_schema_version);
        reject_increment!(universe_manifest_schema_version);
        reject_increment!(interest_schema_version);
        reject_increment!(operation_fingerprint_schema_version);
        reject_increment!(lifecycle_control_schema_version);
        reject_increment!(production_occurrence_schema_version);
        reject_increment!(cell_key_schema_version);
        reject_increment!(directory_schema_version);
        reject_increment!(transfer_package_schema_version);

        let mut changed = original;
        changed.compatibility.content_manifest_version = "p1.5.0-substituted".into();
        reseal(&mut changed);
        assert!(validate_manifest_v5(&changed, 801).is_err());
    }

    #[test]
    fn manifest_v5_rejects_rehashed_identity_root_and_dimension_substitution() {
        let original = build_validated_manifest_v5(801)
            .expect("manifest builds")
            .document;
        for mutate in [
            |value: &mut UniverseManifestSnapshotV5| value.universe_id.push_str("-other"),
            |value: &mut UniverseManifestSnapshotV5| value.world_seed = "802".into(),
            |value: &mut UniverseManifestSnapshotV5| value.address_schema_version += 1,
            |value: &mut UniverseManifestSnapshotV5| value.sector_edge_um += 1,
            |value: &mut UniverseManifestSnapshotV5| value.cell_edge_um += 1,
            |value: &mut UniverseManifestSnapshotV5| value.cells_per_sector_axis += 1,
            |value: &mut UniverseManifestSnapshotV5| {
                value.generation_rule_version.push_str("-other");
            },
            |value: &mut UniverseManifestSnapshotV5| {
                value.frontier_policy_version.push_str("-other");
            },
            |value: &mut UniverseManifestSnapshotV5| {
                value.celestial_registry_hash = "ab".repeat(32);
            },
            |value: &mut UniverseManifestSnapshotV5| value.content_hash = "ab".repeat(32),
            |value: &mut UniverseManifestSnapshotV5| value.lifecycle_policy_hash = "ab".repeat(32),
        ] {
            let mut changed = original.clone();
            mutate(&mut changed);
            reseal(&mut changed);
            assert!(validate_manifest_v5(&changed, 801).is_err());
        }
    }

    #[test]
    fn manifest_v4_and_hybrid_v4_with_world21_event17_never_cross_decode() {
        let active = celestial::universe_manifest(
            801,
            crate::model::WORLD_SCHEMA_VERSION,
            crate::event::EVENT_SCHEMA_VERSION,
        )
        .expect("active manifest builds");
        assert!(
            decode_manifest_v5(&serde_json::to_vec(&active).expect("v4 encodes"), 801).is_err()
        );

        let hybrid = celestial::universe_manifest(801, 21, 17).expect("hybrid v4 builds");
        assert_eq!(hybrid.schema_version, 4);
        assert_eq!(hybrid.projection_schema_version, 4);
        assert_eq!(hybrid.interest_schema_version, 2);
        assert_eq!(hybrid.cell_directory_schema_version, 2);
        assert_eq!(hybrid.transfer_package_schema_version, 1);
        assert!(
            decode_manifest_v5(&serde_json::to_vec(&hybrid).expect("hybrid encodes"), 801).is_err()
        );
    }

    #[test]
    fn gameplay_body_identity_is_contextual_and_never_cross_validates() {
        let manifest = build_validated_manifest_v5(801).expect("manifest builds");
        let other_manifest = build_validated_manifest_v5(802).expect("other manifest builds");
        let active = crate::model::WorldState::genesis(801);
        assert!(active.validate_player_roster().is_ok());
        assert!(
            active
                .validate_world_v21_gameplay_body_with_job_frontier(&manifest, |job| {
                    job.queued_event_sequence <= active.event_sequence
                })
                .is_err(),
            "manifest-4 authority cannot be treated as a world-21 gameplay body"
        );

        let mut body = active;
        body.universe_manifest_hash = manifest.manifest_hash().to_owned();
        assert!(body.validate_player_roster().is_err());
        assert!(
            body.validate_world_v21_gameplay_body_with_job_frontier(&manifest, |job| {
                job.queued_event_sequence <= body.event_sequence
            })
            .is_ok()
        );
        assert!(
            body.validate_world_v21_gameplay_body_with_job_frontier(&other_manifest, |job| {
                job.queued_event_sequence <= body.event_sequence
            })
            .is_err(),
            "a valid capability for another universe cannot authorize this body"
        );
    }
}
