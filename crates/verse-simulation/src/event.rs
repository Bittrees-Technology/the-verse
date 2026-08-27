// SPDX-License-Identifier: AGPL-3.0-or-later

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use verse_protocol::{IVec3, PlayerDeathCause, Quat, ResourceKind, Vec3, VoxelMaterial};

use crate::model::{Block, ContactPairKey, DeathDrop, InventoryRecord};

pub const EVENT_SCHEMA_NAME: &str = "verse.world_event";
pub const EVENT_SCHEMA_VERSION: u32 = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum EventPayload {
    PlayerControlSet {
        movement_epoch: u64,
        input_sequence: u64,
        linear_input: Vec3,
        angular_input: Vec3,
        boost: bool,
        dampeners: bool,
        expires_at_simulation_tick: u64,
    },
    SuitModeChanged {
        helmet_closed: bool,
        jetpack_enabled: bool,
    },
    SuitOxygenChanged {
        previous_oxygen_milli: u16,
        new_oxygen_milli: u16,
    },
    PlayerIncapacitated {
        death_id: String,
        cause: PlayerDeathCause,
        position: Vec3,
        previous_oxygen_milli: u16,
        dropped_inventory: Option<InventoryRecord>,
        death_drop: Option<DeathDrop>,
    },
    PlayerRespawned {
        death_id: String,
        position: Vec3,
        suit_oxygen_milli: u16,
        helmet_closed: bool,
        jetpack_enabled: bool,
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
    BlockWelded {
        grid_id: String,
        block_id: String,
        previous_health: u16,
        new_health: u16,
        max_health: u16,
        completed_construction: bool,
    },
    GridControlSet {
        grid_id: String,
        linear_input: Vec3,
        angular_input: Vec3,
        dampeners: bool,
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
    PhysicsStepCommitted {
        fixed_step_hz: u16,
        step_count: u8,
        remaining_step_phase: u32,
        bodies: Vec<PhysicsBodyOutcome>,
        player: Option<PlayerPhysicsOutcome>,
        contacts: Vec<PhysicsContactOutcome>,
        active_contacts_after: Vec<ContactPairKey>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerPhysicsOutcome {
    pub player_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub surface_contact: bool,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub control_expires_at_simulation_tick: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsBodyOutcome {
    pub grid_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsContactOutcome {
    pub substep_index: u8,
    pub body_a_id: String,
    pub collider_a_id: String,
    pub body_b_id: String,
    pub collider_b_id: String,
    pub point: Vec3,
    pub normal: Vec3,
    pub penetration_m: f64,
    pub closing_speed_mm_per_second: u64,
    pub estimated_normal_impulse_millinewton_seconds: u64,
    pub reduced_translational_mass_grams: u64,
    pub phase: PhysicsContactPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsContactPhase {
    Began,
    Persisted,
}

impl EventPayload {
    pub fn experience_reward(&self) -> u64 {
        match self {
            Self::VoxelMined { ore_yield, .. } => ore_yield * 5,
            Self::OreRefined { batches, .. } => batches * 12,
            Self::ComponentCrafted { quantity, .. } => quantity * 18,
            Self::InventoryTransferred { .. } => 2,
            Self::BlockBuilt { .. } => 5,
            Self::BlockWelded {
                completed_construction,
                ..
            } => {
                if *completed_construction {
                    20
                } else {
                    6
                }
            }
            Self::GridAnchorSet { anchored: true, .. } => 40,
            Self::BlockDamaged { .. } => 3,
            Self::PlayerControlSet { .. }
            | Self::SuitModeChanged { .. }
            | Self::SuitOxygenChanged { .. }
            | Self::PlayerIncapacitated { .. }
            | Self::PlayerRespawned { .. }
            | Self::GridControlSet { .. }
            | Self::GridAnchorSet {
                anchored: false, ..
            }
            | Self::PhysicsStepCommitted { .. } => 0,
        }
    }

    pub fn receipt(&self) -> (&'static str, String) {
        match self {
            Self::PlayerControlSet { input_sequence, .. } => (
                "player_control_set",
                format!("Character control {input_sequence} accepted"),
            ),
            Self::SuitModeChanged {
                helmet_closed,
                jetpack_enabled,
            } => (
                "suit_mode_changed",
                format!(
                    "Helmet {} — jetpack {}",
                    if *helmet_closed { "sealed" } else { "open" },
                    if *jetpack_enabled {
                        "online"
                    } else {
                        "offline"
                    }
                ),
            ),
            Self::SuitOxygenChanged {
                new_oxygen_milli, ..
            } => (
                "suit_oxygen_changed",
                format!("Suit oxygen {}%", u32::from(*new_oxygen_milli) / 10),
            ),
            Self::PlayerIncapacitated { .. } => (
                "player_incapacitated",
                "Life support failed — recovery required".into(),
            ),
            Self::PlayerRespawned { .. } => (
                "player_respawned",
                "Recovery complete — suit inventory is empty".into(),
            ),
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
            Self::BlockBuilt { block, .. } => (
                "block_frame_placed",
                format!("Placed {:?} frame", block.kind),
            ),
            Self::BlockWelded {
                new_health,
                max_health,
                ..
            } => {
                let percent = u32::from(*new_health) * 100 / u32::from(*max_health);
                (
                    "block_welded",
                    format!("Welded block to {percent}% integrity"),
                )
            }
            Self::GridControlSet { .. } => ("grid_control_set", "Grid control accepted".into()),
            Self::GridAnchorSet { anchored, .. } => (
                "grid_anchor_set",
                if *anchored {
                    "Grid anchored".into()
                } else {
                    "Grid released".into()
                },
            ),
            Self::BlockDamaged { .. } => ("block_damaged", "Damage applied".into()),
            Self::PhysicsStepCommitted { .. } => {
                ("physics_step_committed", "Physics step committed".into())
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
            schema_name: EVENT_SCHEMA_NAME.into(),
            schema_version: EVENT_SCHEMA_VERSION,
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
            "p0.9.0",
            "universe",
            "cell",
            9,
            "player",
            "human",
            Some("op-1".into()),
            "",
            EventPayload::PlayerControlSet {
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::new(1.0, 0.0, 0.0),
                angular_input: Vec3::ZERO,
                boost: false,
                dampeners: true,
                expires_at_simulation_tick: 18,
            },
        );
        assert!(event.hash_is_valid());
        event.payload = EventPayload::PlayerControlSet {
            movement_epoch: 1,
            input_sequence: 1,
            linear_input: Vec3::new(0.0, 1.0, 0.0),
            angular_input: Vec3::ZERO,
            boost: false,
            dampeners: true,
            expires_at_simulation_tick: 18,
        };
        assert!(!event.hash_is_valid());
    }
}
