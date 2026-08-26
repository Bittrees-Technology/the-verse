// SPDX-License-Identifier: Apache-2.0

//! Public, versioned protocol types shared by The Verse clients and server.
//!
//! P0 deliberately uses JSON over WebSocket so the native prototype, browser
//! tools, and test harnesses can inspect every authoritative message. A later
//! binary replication protocol must preserve these semantic contracts.

use serde::{Deserialize, Serialize};

/// The only protocol version accepted by this P0 build.
pub const PROTOCOL_VERSION: u32 = 2;

/// A stable integer voxel or block coordinate.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct IVec3 {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl IVec3 {
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };

    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    pub fn squared_distance(self, other: Self) -> i64 {
        let dx = i64::from(self.x - other.x);
        let dy = i64::from(self.y - other.y);
        let dz = i64::from(self.z - other.z);
        dx * dx + dy * dy + dz * dz
    }

    pub fn manhattan_distance(self, other: Self) -> i32 {
        (self.x - other.x).abs() + (self.y - other.y).abs() + (self.z - other.z).abs()
    }

    pub fn neighbors(self) -> [Self; 6] {
        [
            Self::new(self.x + 1, self.y, self.z),
            Self::new(self.x - 1, self.y, self.z),
            Self::new(self.x, self.y + 1, self.z),
            Self::new(self.x, self.y - 1, self.z),
            Self::new(self.x, self.y, self.z + 1),
            Self::new(self.x, self.y, self.z - 1),
        ]
    }
}

/// A local floating-point coordinate used for rendering and active-cell motion.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn squared_distance(self, other: Self) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx.mul_add(dx, dy.mul_add(dy, dz * dz))
    }

    pub fn magnitude(self) -> f64 {
        self.squared_distance(Self::ZERO).sqrt()
    }

    #[must_use]
    pub fn clamped(self, max: f64) -> Self {
        let magnitude = self.magnitude();
        if magnitude <= max || magnitude == 0.0 {
            self
        } else {
            let scale = max / magnitude;
            Self::new(self.x * scale, self.y * scale, self.z * scale)
        }
    }
}

impl std::ops::Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl std::ops::Mul<f64> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoxelMaterial {
    Rock,
    FerriteOre,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Ore,
    RefinedMaterial,
    Component,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Structural,
    ControlCore,
    PowerSource,
    Battery,
    Cargo,
    Drill,
    Anchor,
    DamageTest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoxelSnapshot {
    pub coordinate: IVec3,
    pub material: VoxelMaterial,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryContents {
    pub ore: u64,
    pub refined_material: u64,
    pub components: u64,
}

impl InventoryContents {
    pub const fn amount(&self, resource: ResourceKind) -> u64 {
        match resource {
            ResourceKind::Ore => self.ore,
            ResourceKind::RefinedMaterial => self.refined_material,
            ResourceKind::Component => self.components,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InventoryDomain {
    Player { player_id: String },
    Cargo { block_id: String },
    Dropped { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventorySnapshot {
    pub inventory_id: String,
    pub domain: InventoryDomain,
    pub contents: InventoryContents,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub player_id: String,
    pub position: Vec3,
    pub inventory_id: String,
    pub experience: u64,
    pub level: u32,
    pub next_level_experience: u64,
    pub career: CareerSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CareerSnapshot {
    pub voxels_mined: u64,
    pub refining_batches: u64,
    pub components_crafted: u64,
    pub blocks_built: u64,
    pub anchors_engaged: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockSnapshot {
    pub block_id: String,
    pub coordinate: IVec3,
    pub kind: BlockKind,
    pub health: u16,
    pub inventory_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PowerSnapshot {
    pub produced: f64,
    pub required: f64,
    pub stored: f64,
    pub online: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridSnapshot {
    pub grid_id: String,
    pub position: Vec3,
    pub yaw_radians: f64,
    pub linear_velocity: Vec3,
    pub angular_velocity: f64,
    pub anchored: bool,
    pub power: PowerSnapshot,
    pub blocks: Vec<BlockSnapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConservationSnapshot {
    pub ore_sources: u64,
    pub ore_live: u64,
    pub ore_consumed: u64,
    pub refined_sources: u64,
    pub refined_live: u64,
    pub refined_consumed: u64,
    pub component_sources: u64,
    pub components_live: u64,
    pub components_installed_or_destroyed: u64,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub universe_id: String,
    pub cell_id: String,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub fencing_token: u64,
    pub world_hash: String,
    pub player: PlayerSnapshot,
    pub voxels: Vec<VoxelSnapshot>,
    pub grids: Vec<GridSnapshot>,
    pub inventories: Vec<InventorySnapshot>,
    pub conservation: ConservationSnapshot,
}

/// Commands sent by all P0 clients. Every mutating command carries an operation
/// ID so retrying a packet returns the original result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        protocol_version: u32,
        client_name: String,
    },
    RequestSnapshot,
    MovePlayer {
        operation_id: String,
        position: Vec3,
    },
    MineVoxel {
        operation_id: String,
        coordinate: IVec3,
    },
    RefineOre {
        operation_id: String,
        inventory_id: String,
        batches: u64,
    },
    CraftComponent {
        operation_id: String,
        inventory_id: String,
        quantity: u64,
    },
    TransferInventory {
        operation_id: String,
        source_inventory_id: String,
        destination_inventory_id: String,
        resource: ResourceKind,
        quantity: u64,
    },
    BuildBlock {
        operation_id: String,
        grid_id: String,
        coordinate: IVec3,
        kind: BlockKind,
    },
    SetGridMotion {
        operation_id: String,
        grid_id: String,
        linear_velocity: Vec3,
        angular_velocity: f64,
    },
    ToggleGridAnchor {
        operation_id: String,
        grid_id: String,
    },
    DamageBlock {
        operation_id: String,
        grid_id: String,
        block_id: String,
    },
}

impl ClientMessage {
    pub fn operation_id(&self) -> Option<&str> {
        match self {
            Self::Hello { .. } | Self::RequestSnapshot => None,
            Self::MovePlayer { operation_id, .. }
            | Self::MineVoxel { operation_id, .. }
            | Self::RefineOre { operation_id, .. }
            | Self::CraftComponent { operation_id, .. }
            | Self::TransferInventory { operation_id, .. }
            | Self::BuildBlock { operation_id, .. }
            | Self::SetGridMotion { operation_id, .. }
            | Self::ToggleGridAnchor { operation_id, .. }
            | Self::DamageBlock { operation_id, .. } => Some(operation_id),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentReceipt {
    pub operation_id: String,
    pub event_sequence: u64,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        protocol_version: u32,
        server_name: String,
    },
    Snapshot {
        snapshot: Box<WorldSnapshot>,
    },
    IntentAccepted {
        receipt: IntentReceipt,
    },
    IntentRejected {
        operation_id: Option<String>,
        code: String,
        message: String,
    },
    Fatal {
        code: String,
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_messages_use_stable_tagged_json() {
        let message = ClientMessage::MineVoxel {
            operation_id: "op-1".into(),
            coordinate: IVec3::new(1, 2, 3),
        };
        let value = serde_json::to_value(message).expect("message serializes");
        assert_eq!(value["type"], "mine_voxel");
        assert_eq!(value["coordinate"]["z"], 3);
    }

    #[test]
    fn vector_clamping_preserves_direction() {
        let vector = Vec3::new(6.0, 0.0, 8.0).clamped(5.0);
        assert!((vector.x - 3.0).abs() < f64::EPSILON);
        assert!((vector.z - 4.0).abs() < f64::EPSILON);
    }
}
