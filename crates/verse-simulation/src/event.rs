// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use verse_protocol::{IVec3, ResourceKind, Vec3, VoxelMaterial};

use crate::model::Block;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum EventPayload {
    PlayerMoved {
        position: Vec3,
    },
    VoxelMined {
        coordinate: IVec3,
        material: VoxelMaterial,
        ore_yield: u64,
        inventory_id: String,
    },
    OreRefined {
        inventory_id: String,
        batches: u64,
    },
    ComponentCrafted {
        inventory_id: String,
        quantity: u64,
    },
    InventoryTransferred {
        source_inventory_id: String,
        destination_inventory_id: String,
        resource: ResourceKind,
        quantity: u64,
    },
    BlockBuilt {
        grid_id: String,
        block: Block,
    },
    GridMotionSet {
        grid_id: String,
        linear_velocity: Vec3,
        angular_velocity: f64,
    },
    GridAnchorSet {
        grid_id: String,
        anchored: bool,
    },
    BlockDamaged {
        grid_id: String,
        block_id: String,
        damage: u16,
    },
    SimulationAdvanced {
        delta_millis: u16,
    },
}

impl EventPayload {
    pub fn experience_reward(&self) -> u64 {
        match self {
            Self::VoxelMined { ore_yield, .. } => ore_yield * 5,
            Self::OreRefined { batches, .. } => batches * 12,
            Self::ComponentCrafted { quantity, .. } => quantity * 18,
            Self::InventoryTransferred { .. } => 2,
            Self::BlockBuilt { .. } => 25,
            Self::GridAnchorSet { anchored: true, .. } => 40,
            Self::BlockDamaged { .. } => 3,
            Self::PlayerMoved { .. }
            | Self::GridMotionSet { .. }
            | Self::GridAnchorSet {
                anchored: false, ..
            }
            | Self::SimulationAdvanced { .. } => 0,
        }
    }

    pub fn receipt(&self) -> (&'static str, String) {
        match self {
            Self::PlayerMoved { .. } => ("player_moved", "Position accepted".into()),
            Self::VoxelMined { ore_yield, .. } => ("voxel_mined", format!("Mined {ore_yield} ore")),
            Self::OreRefined { batches, .. } => (
                "ore_refined",
                format!("Refined {batches} batch(es) into {batches} material"),
            ),
            Self::ComponentCrafted { quantity, .. } => (
                "component_crafted",
                format!("Crafted {quantity} component(s)"),
            ),
            Self::InventoryTransferred {
                resource, quantity, ..
            } => (
                "inventory_transferred",
                format!("Transferred {quantity} {resource:?}"),
            ),
            Self::BlockBuilt { block, .. } => {
                ("block_built", format!("Built {:?} block", block.kind))
            }
            Self::GridMotionSet { .. } => ("grid_motion_set", "Grid motion accepted".into()),
            Self::GridAnchorSet { anchored, .. } => (
                "grid_anchor_set",
                if *anchored {
                    "Grid anchored".into()
                } else {
                    "Grid released".into()
                },
            ),
            Self::BlockDamaged { .. } => ("block_damaged", "Damage applied".into()),
            Self::SimulationAdvanced { .. } => {
                ("simulation_advanced", "Simulation advanced".into())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalEvent {
    pub schema_name: String,
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub event_id: String,
    pub event_sequence: u64,
    pub occurred_at_unix_ms: u64,
    pub universe_id: String,
    pub cell_id: String,
    pub authority_fencing_token: u64,
    pub actor_profile_id: String,
    pub actor_type: String,
    pub operation_id: Option<String>,
    pub previous_event_hash: String,
    pub payload: EventPayload,
    pub event_hash: String,
}

#[derive(Serialize)]
struct EventHashMaterial<'a> {
    schema_name: &'a str,
    schema_version: u32,
    content_manifest_version: &'a str,
    event_id: &'a str,
    event_sequence: u64,
    occurred_at_unix_ms: u64,
    universe_id: &'a str,
    cell_id: &'a str,
    authority_fencing_token: u64,
    actor_profile_id: &'a str,
    actor_type: &'a str,
    operation_id: &'a Option<String>,
    previous_event_hash: &'a str,
    payload: &'a EventPayload,
}

impl CanonicalEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_sequence: u64,
        content_manifest_version: impl Into<String>,
        universe_id: impl Into<String>,
        cell_id: impl Into<String>,
        authority_fencing_token: u64,
        actor_profile_id: impl Into<String>,
        actor_type: impl Into<String>,
        operation_id: Option<String>,
        previous_event_hash: impl Into<String>,
        payload: EventPayload,
    ) -> Self {
        let occurred_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        let mut event = Self {
            schema_name: "verse.world_event".into(),
            schema_version: 1,
            content_manifest_version: content_manifest_version.into(),
            event_id: Uuid::new_v4().to_string(),
            event_sequence,
            occurred_at_unix_ms,
            universe_id: universe_id.into(),
            cell_id: cell_id.into(),
            authority_fencing_token,
            actor_profile_id: actor_profile_id.into(),
            actor_type: actor_type.into(),
            operation_id,
            previous_event_hash: previous_event_hash.into(),
            payload,
            event_hash: String::new(),
        };
        event.event_hash = event.calculate_hash();
        event
    }

    pub fn calculate_hash(&self) -> String {
        let material = EventHashMaterial {
            schema_name: &self.schema_name,
            schema_version: self.schema_version,
            content_manifest_version: &self.content_manifest_version,
            event_id: &self.event_id,
            event_sequence: self.event_sequence,
            occurred_at_unix_ms: self.occurred_at_unix_ms,
            universe_id: &self.universe_id,
            cell_id: &self.cell_id,
            authority_fencing_token: self.authority_fencing_token,
            actor_profile_id: &self.actor_profile_id,
            actor_type: &self.actor_type,
            operation_id: &self.operation_id,
            previous_event_hash: &self.previous_event_hash,
            payload: &self.payload,
        };
        let bytes = serde_json::to_vec(&material).expect("event hash material serializes");
        blake3::hash(&bytes).to_hex().to_string()
    }

    pub fn hash_is_valid(&self) -> bool {
        self.event_hash == self.calculate_hash()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_hash_detects_payload_tampering() {
        let mut event = CanonicalEvent::new(
            1,
            "p0.3.0",
            "universe",
            "cell",
            9,
            "player",
            "human",
            Some("op-1".into()),
            "",
            EventPayload::PlayerMoved {
                position: Vec3::new(1.0, 2.0, 3.0),
            },
        );
        assert!(event.hash_is_valid());
        event.payload = EventPayload::PlayerMoved {
            position: Vec3::new(9.0, 2.0, 3.0),
        };
        assert!(!event.hash_is_valid());
    }
}
