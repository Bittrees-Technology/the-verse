// SPDX-License-Identifier: Apache-2.0

//! Dormant protocol-19 wire declarations.
//!
//! These types do not change the active protocol-18 constants or messages.
//! They give the grid-closure implementation one exact compatibility tuple to
//! validate before a future coordinated activation.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 19;
pub const PROJECTION_SCHEMA_VERSION: u32 = 5;
pub const WORLD_SCHEMA_VERSION: u32 = 21;
pub const EVENT_SCHEMA_VERSION: u32 = 17;
pub const CONTENT_SCHEMA_VERSION: u32 = 11;
pub const CONTENT_MANIFEST_VERSION: &str = "p1.5.0";
pub const CELESTIAL_REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const UNIVERSE_MANIFEST_SCHEMA_VERSION: u32 = 5;
pub const INTEREST_SCHEMA_VERSION: u32 = 3;
pub const INTENT_FINGERPRINT_SCHEMA_VERSION: u32 = 2;
pub const LIFECYCLE_CONTROL_SCHEMA_VERSION: u32 = 2;
pub const PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION: u32 = 1;
pub const CELL_KEY_SCHEMA_VERSION: u32 = 1;
pub const CELL_DIRECTORY_SCHEMA_VERSION: u32 = 3;
pub const TRANSFER_PACKAGE_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Protocol19CompatibilityTuple {
    pub protocol_version: u32,
    pub projection_schema_version: u32,
    pub world_schema_version: u32,
    pub event_schema_version: u32,
    pub content_schema_version: u32,
    pub content_manifest_version: String,
    pub celestial_registry_schema_version: u32,
    pub universe_manifest_schema_version: u32,
    pub interest_schema_version: u32,
    pub operation_fingerprint_schema_version: u32,
    pub lifecycle_control_schema_version: u32,
    pub production_occurrence_schema_version: u32,
    pub cell_key_schema_version: u32,
    pub directory_schema_version: u32,
    pub transfer_package_schema_version: u32,
}

impl Protocol19CompatibilityTuple {
    #[must_use]
    pub fn canonical() -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            world_schema_version: WORLD_SCHEMA_VERSION,
            event_schema_version: EVENT_SCHEMA_VERSION,
            content_schema_version: CONTENT_SCHEMA_VERSION,
            content_manifest_version: CONTENT_MANIFEST_VERSION.into(),
            celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            universe_manifest_schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            operation_fingerprint_schema_version: INTENT_FINGERPRINT_SCHEMA_VERSION,
            lifecycle_control_schema_version: LIFECYCLE_CONTROL_SCHEMA_VERSION,
            production_occurrence_schema_version: PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            cell_key_schema_version: CELL_KEY_SCHEMA_VERSION,
            directory_schema_version: CELL_DIRECTORY_SCHEMA_VERSION,
            transfer_package_schema_version: TRANSFER_PACKAGE_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseManifestSnapshotV5 {
    pub schema_version: u32,
    pub manifest_hash: String,
    pub compatibility: Protocol19CompatibilityTuple,
    pub universe_id: String,
    pub world_seed: String,
    pub address_schema_version: u32,
    pub sector_edge_um: u64,
    pub cell_edge_um: u64,
    pub cells_per_sector_axis: u32,
    pub generation_rule_version: String,
    pub frontier_policy_version: String,
    pub celestial_registry_hash: String,
    pub content_hash: String,
    pub lifecycle_policy_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dormant_tuple_does_not_change_the_active_protocol() {
        assert_eq!(crate::PROTOCOL_VERSION, 18);
        assert_eq!(crate::UNIVERSE_MANIFEST_SCHEMA_VERSION, 4);
        assert_eq!(crate::CELL_DIRECTORY_SCHEMA_VERSION, 2);
        assert_eq!(crate::TRANSFER_PACKAGE_SCHEMA_VERSION, 1);

        let tuple = Protocol19CompatibilityTuple::canonical();
        assert_eq!(tuple.protocol_version, 19);
        assert_eq!(tuple.universe_manifest_schema_version, 5);
        assert_eq!(tuple.content_manifest_version, "p1.5.0");
        assert_eq!(tuple.directory_schema_version, 3);
        assert_eq!(tuple.transfer_package_schema_version, 2);
    }
}
