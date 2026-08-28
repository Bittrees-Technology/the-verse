// SPDX-License-Identifier: AGPL-3.0-or-later

//! Universe-unique canonical identities for subjects created by cell events.

use serde::Serialize;
use thiserror::Error;

pub const SUBJECT_ID_SCHEMA_VERSION: u32 = 1;

const SUBJECT_ID_DOMAIN: &[u8] = b"the-verse/canonical-subject-id/v1\0";
const MAX_UNIVERSE_ID_BYTES: usize = 96;
const MAX_KIND_BYTES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SubjectIdError {
    #[error("canonical subject identity material is invalid: {0}")]
    InvalidMaterial(&'static str),
    #[error("canonical subject identity material cannot be encoded: {0}")]
    Encoding(String),
}

#[derive(Serialize)]
struct SubjectIdMaterial<'a> {
    schema_version: u32,
    universe_id: &'a str,
    creator_cell_id: &'a str,
    event_sequence: u64,
    entity_kind: &'a str,
    ordinal: u32,
}

/// Derives one stable identity whose namespace cannot collide at equal local
/// event sequences in different cells.
pub fn canonical_subject_id(
    universe_id: &str,
    creator_cell_id: &str,
    event_sequence: u64,
    entity_kind: &str,
    ordinal: u32,
) -> Result<String, SubjectIdError> {
    if universe_id.is_empty()
        || universe_id.len() > MAX_UNIVERSE_ID_BYTES
        || !universe_id.bytes().all(valid_universe_id_byte)
    {
        return Err(SubjectIdError::InvalidMaterial(
            "universe ID must be bounded canonical ASCII text",
        ));
    }
    if !is_blake3_hex(creator_cell_id) {
        return Err(SubjectIdError::InvalidMaterial(
            "creator cell ID must be lowercase BLAKE3 hex",
        ));
    }
    if event_sequence == 0 {
        return Err(SubjectIdError::InvalidMaterial(
            "event sequence must be positive",
        ));
    }
    if entity_kind.is_empty()
        || entity_kind.len() > MAX_KIND_BYTES
        || !entity_kind
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(SubjectIdError::InvalidMaterial(
            "entity kind must be bounded lowercase kebab-case ASCII",
        ));
    }

    let material = SubjectIdMaterial {
        schema_version: SUBJECT_ID_SCHEMA_VERSION,
        universe_id,
        creator_cell_id,
        event_sequence,
        entity_kind,
        ordinal,
    };
    let bytes = serde_json::to_vec(&material)
        .map_err(|source| SubjectIdError::Encoding(source.to_string()))?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(SUBJECT_ID_DOMAIN);
    hasher.update(&bytes);
    Ok(format!("{entity_kind}-{}", hasher.finalize().to_hex()))
}

fn valid_universe_id_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')
}

fn is_blake3_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UNIVERSE: &str = "the-verse-proof-universe";
    const ORIGIN_CELL: &str = "5110e8ef07316dc5fc8cd48210915d3e879779c67dc3e11a9da0402656c76d17";
    const EAST_CELL: &str = "e24242afc42c71a9629093e0c82b1779e306e92c52804ebc105ef373fa5a8f4d";

    #[test]
    fn same_material_is_stable_and_cross_cell_material_is_unique() {
        let origin = canonical_subject_id(UNIVERSE, ORIGIN_CELL, 41, "block", 0)
            .expect("origin identity derives");
        let repeated = canonical_subject_id(UNIVERSE, ORIGIN_CELL, 41, "block", 0)
            .expect("same identity derives");
        let east = canonical_subject_id(UNIVERSE, EAST_CELL, 41, "block", 0)
            .expect("east identity derives");

        assert_eq!(origin, repeated);
        assert_ne!(origin, east);
        assert_eq!(
            origin,
            "block-40c460d27bb9579d731084f1d425f06a17a9bbdb440eaff15a17747e5649b691"
        );
    }

    #[test]
    fn every_identity_axis_changes_the_result() {
        let baseline =
            canonical_subject_id(UNIVERSE, ORIGIN_CELL, 41, "block", 0).expect("baseline derives");
        for candidate in [
            canonical_subject_id("another-universe", ORIGIN_CELL, 41, "block", 0),
            canonical_subject_id(UNIVERSE, EAST_CELL, 41, "block", 0),
            canonical_subject_id(UNIVERSE, ORIGIN_CELL, 42, "block", 0),
            canonical_subject_id(UNIVERSE, ORIGIN_CELL, 41, "production-job", 0),
            canonical_subject_id(UNIVERSE, ORIGIN_CELL, 41, "block", 1),
        ] {
            assert_ne!(baseline, candidate.expect("variant derives"));
        }
    }

    #[test]
    fn invalid_or_ambiguous_material_fails_closed() {
        for result in [
            canonical_subject_id("", ORIGIN_CELL, 1, "block", 0),
            canonical_subject_id(UNIVERSE, "A", 1, "block", 0),
            canonical_subject_id(UNIVERSE, ORIGIN_CELL, 0, "block", 0),
            canonical_subject_id(UNIVERSE, ORIGIN_CELL, 1, "Block", 0),
            canonical_subject_id(UNIVERSE, ORIGIN_CELL, 1, "block_grid", 0),
        ] {
            assert!(result.is_err());
        }
    }
}
