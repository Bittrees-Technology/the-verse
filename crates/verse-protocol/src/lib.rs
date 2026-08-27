// SPDX-License-Identifier: Apache-2.0

//! Public, versioned protocol types shared by The Verse clients and server.
//!
//! P0 deliberately uses JSON over WebSocket so the native prototype, browser
//! tools, and test harnesses can inspect every authoritative message. A later
//! binary replication protocol must preserve these semantic contracts.

use serde::{Deserialize, Serialize};

/// The only protocol version accepted by this P0 build.
pub const PROTOCOL_VERSION: u32 = 9;

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

impl std::ops::Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

/// A normalized local-space rotation. Components use the same ordering as
/// Godot and Jolt (`x`, `y`, `z`, `w`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
        w: 1.0,
    };

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }

    pub fn rotate(self, vector: Vec3) -> Vec3 {
        let x = f64::from(self.x);
        let y = f64::from(self.y);
        let z = f64::from(self.z);
        let w = f64::from(self.w);
        let tx = 2.0 * (y * vector.z - z * vector.y);
        let ty = 2.0 * (z * vector.x - x * vector.z);
        let tz = 2.0 * (x * vector.y - y * vector.x);
        Vec3::new(
            vector.x + w * tx + (y * tz - z * ty),
            vector.y + w * ty + (z * tx - x * tz),
            vector.z + w * tz + (x * ty - y * tx),
        )
    }

    #[must_use]
    pub const fn conjugate(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, self.w)
    }
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
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
    pub capacity_liters: u64,
    pub used_liters: u64,
    pub mass_grams: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlayerDeathCause {
    OxygenDepleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlayerLifeState {
    Alive,
    Incapacitated {
        death_id: String,
        cause: PlayerDeathCause,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeathDropSnapshot {
    pub drop_id: String,
    pub death_id: String,
    pub inventory_id: String,
    pub owner_player_id: String,
    pub position: Vec3,
    pub created_event_sequence: u64,
    pub cause: PlayerDeathCause,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct PlayerSnapshot {
    pub player_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub surface_contact: bool,
    pub movement_epoch: u64,
    pub last_received_input_sequence: u64,
    pub last_processed_input_sequence: u64,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub control_expires_at_simulation_tick: u64,
    pub inventory_id: String,
    pub experience: u64,
    pub level: u32,
    pub next_level_experience: u64,
    pub career: CareerSnapshot,
    pub life_state: PlayerLifeState,
    pub suit_oxygen_milli: u16,
    pub critical_oxygen_milli: u16,
    pub helmet_closed: bool,
    pub jetpack_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentSnapshot {
    pub celestial_body_id: String,
    pub celestial_body_name: String,
    pub planet_center: Vec3,
    pub surface_radius_m: f64,
    pub altitude_m: f64,
    pub gravity: Vec3,
    pub gravity_m_s2: f64,
    pub atmosphere_density: f64,
    pub oxygen_fraction: f64,
    pub breathable: bool,
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
    pub orientation: u8,
    pub health: u16,
    pub max_health: u16,
    pub construction_complete: bool,
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
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub mass_kg: f64,
    pub anchored: bool,
    pub power: PowerSnapshot,
    pub blocks: Vec<BlockSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct PlayerMotionSnapshot {
    pub player_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub surface_contact: bool,
    pub movement_epoch: u64,
    pub last_received_input_sequence: u64,
    pub last_processed_input_sequence: u64,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub control_expires_at_simulation_tick: u64,
    pub jetpack_enabled: bool,
    pub life_state: PlayerLifeState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridMotionSnapshot {
    pub grid_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionSnapshot {
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub world_hash: String,
    pub player: PlayerMotionSnapshot,
    pub grids: Vec<GridMotionSnapshot>,
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
    pub environment: EnvironmentSnapshot,
    pub voxels: Vec<VoxelSnapshot>,
    pub grids: Vec<GridSnapshot>,
    pub inventories: Vec<InventorySnapshot>,
    pub death_drops: Vec<DeathDropSnapshot>,
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
    SetPlayerControl {
        operation_id: String,
        movement_epoch: u64,
        input_sequence: u64,
        linear_input: Vec3,
        angular_input: Vec3,
        boost: bool,
        dampeners: bool,
    },
    SetSuitMode {
        operation_id: String,
        helmet_closed: bool,
        jetpack_enabled: bool,
    },
    RespawnPlayer {
        operation_id: String,
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
        orientation: u8,
    },
    WeldBlock {
        operation_id: String,
        grid_id: String,
        block_id: String,
    },
    SetGridControl {
        operation_id: String,
        grid_id: String,
        linear_input: Vec3,
        angular_input: Vec3,
        dampeners: bool,
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
            Self::SetPlayerControl { operation_id, .. }
            | Self::SetSuitMode { operation_id, .. }
            | Self::RespawnPlayer { operation_id }
            | Self::MineVoxel { operation_id, .. }
            | Self::RefineOre { operation_id, .. }
            | Self::CraftComponent { operation_id, .. }
            | Self::TransferInventory { operation_id, .. }
            | Self::BuildBlock { operation_id, .. }
            | Self::WeldBlock { operation_id, .. }
            | Self::SetGridControl { operation_id, .. }
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
    MotionState {
        motion: Box<MotionSnapshot>,
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

    #[test]
    fn construction_messages_preserve_orientation_and_weld_identity() {
        let frame = serde_json::to_value(ClientMessage::BuildBlock {
            operation_id: "frame-1".into(),
            grid_id: "grid".into(),
            coordinate: IVec3::new(4, 5, 6),
            kind: BlockKind::Drill,
            orientation: 3,
        })
        .expect("frame intent serializes");
        assert_eq!(frame["type"], "build_block");
        assert_eq!(frame["orientation"], 3);

        let weld = serde_json::to_value(ClientMessage::WeldBlock {
            operation_id: "weld-1".into(),
            grid_id: "grid".into(),
            block_id: "block-9".into(),
        })
        .expect("weld intent serializes");
        assert_eq!(weld["type"], "weld_block");
        assert_eq!(weld["block_id"], "block-9");
    }

    #[test]
    fn protocol_requires_explicit_construction_completion_state() {
        let block = BlockSnapshot {
            block_id: "block-frame".into(),
            coordinate: IVec3::new(1, 2, 3),
            kind: BlockKind::Structural,
            orientation: 0,
            health: 65,
            max_health: 100,
            construction_complete: true,
            inventory_id: None,
        };
        let mut value = serde_json::to_value(&block).expect("block snapshot serializes");
        assert_eq!(value["construction_complete"], true);
        value
            .as_object_mut()
            .expect("block snapshot is an object")
            .remove("construction_complete");
        assert!(serde_json::from_value::<BlockSnapshot>(value).is_err());
    }

    #[test]
    fn suit_mode_message_is_explicit_and_idempotent() {
        let value = serde_json::to_value(ClientMessage::SetSuitMode {
            operation_id: "suit-1".into(),
            helmet_closed: false,
            jetpack_enabled: true,
        })
        .expect("suit mode intent serializes");
        assert_eq!(value["type"], "set_suit_mode");
        assert_eq!(value["helmet_closed"], false);
        assert_eq!(value["jetpack_enabled"], true);
    }

    #[test]
    fn protocol_v9_character_control_contains_inputs_but_no_transform_or_time() {
        let message = ClientMessage::SetPlayerControl {
            operation_id: "player-control-3-41".into(),
            movement_epoch: 3,
            input_sequence: 41,
            linear_input: Vec3::new(0.0, 0.0, -1.0),
            angular_input: Vec3::new(0.0, 0.0, 0.5),
            boost: true,
            dampeners: false,
        };
        let value = serde_json::to_value(&message).expect("control serializes");
        assert_eq!(value["type"], "set_player_control");
        assert_eq!(value["movement_epoch"], 3);
        assert_eq!(value["input_sequence"], 41);
        assert_eq!(value["linear_input"]["z"], -1.0);
        assert_eq!(value["angular_input"]["z"], 0.5);
        for forbidden in [
            "position",
            "orientation",
            "linear_velocity",
            "angular_velocity",
            "surface_contact",
            "delta",
            "delta_time",
        ] {
            assert!(
                value.get(forbidden).is_none(),
                "forbidden field {forbidden}"
            );
        }
        assert_eq!(
            serde_json::from_value::<ClientMessage>(value).expect("control deserializes"),
            message
        );
    }

    #[test]
    fn protocol_v9_exposes_tagged_life_state_and_death_cause() {
        assert_eq!(PROTOCOL_VERSION, 9);
        let life_state = PlayerLifeState::Incapacitated {
            death_id: "death-player-local-42".into(),
            cause: PlayerDeathCause::OxygenDepleted,
        };
        let value = serde_json::to_value(&life_state).expect("life state serializes");
        assert_eq!(value["kind"], "incapacitated");
        assert_eq!(value["death_id"], "death-player-local-42");
        assert_eq!(value["cause"]["kind"], "oxygen_depleted");
        assert_eq!(
            serde_json::from_value::<PlayerLifeState>(value).expect("life state deserializes"),
            life_state
        );

        let alive = serde_json::to_value(PlayerLifeState::Alive).expect("alive state serializes");
        assert_eq!(alive, serde_json::json!({ "kind": "alive" }));
    }

    #[test]
    fn protocol_v9_snapshot_exposes_motion_input_oxygen_and_death_drops() {
        let death_drop = DeathDropSnapshot {
            drop_id: "drop-player-local-42".into(),
            death_id: "death-player-local-42".into(),
            inventory_id: "inventory-drop-player-local-42".into(),
            owner_player_id: "player-local".into(),
            position: Vec3::new(1.0, 2.0, 3.0),
            created_event_sequence: 42,
            cause: PlayerDeathCause::OxygenDepleted,
        };
        let player = PlayerSnapshot {
            player_id: "player-local".into(),
            position: death_drop.position,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            surface_contact: false,
            movement_epoch: 1,
            last_received_input_sequence: 0,
            last_processed_input_sequence: 0,
            control_linear_input: Vec3::ZERO,
            control_angular_input: Vec3::ZERO,
            boost: false,
            dampeners: true,
            control_expires_at_simulation_tick: 0,
            inventory_id: "inventory-player-local".into(),
            experience: 0,
            level: 1,
            next_level_experience: 100,
            career: CareerSnapshot::default(),
            life_state: PlayerLifeState::Incapacitated {
                death_id: "death-player-local-42".into(),
                cause: PlayerDeathCause::OxygenDepleted,
            },
            suit_oxygen_milli: 0,
            critical_oxygen_milli: 100,
            helmet_closed: true,
            jetpack_enabled: false,
        };
        let world = WorldSnapshot {
            schema_version: 11,
            content_manifest_version: "p0.9.0".into(),
            universe_id: "the-verse-local".into(),
            cell_id: "cell-origin".into(),
            event_sequence: 42,
            simulation_tick: 0,
            fencing_token: 1,
            world_hash: "hash".into(),
            player,
            environment: EnvironmentSnapshot {
                celestial_body_id: "khepri-prime".into(),
                celestial_body_name: "Khepri Prime".into(),
                planet_center: Vec3::ZERO,
                surface_radius_m: 1_200.0,
                altitude_m: 3_000.0,
                gravity: Vec3::ZERO,
                gravity_m_s2: 0.0,
                atmosphere_density: 0.0,
                oxygen_fraction: 0.0,
                breathable: false,
            },
            voxels: Vec::new(),
            grids: Vec::new(),
            inventories: Vec::new(),
            death_drops: vec![death_drop.clone()],
            conservation: ConservationSnapshot::default(),
        };
        let value = serde_json::to_value(&world).expect("world snapshot serializes");
        assert_eq!(value["player"]["critical_oxygen_milli"], 100);
        assert_eq!(value["death_drops"][0]["drop_id"], death_drop.drop_id);
        assert_eq!(value["death_drops"][0]["cause"]["kind"], "oxygen_depleted");
        assert_eq!(
            serde_json::from_value::<WorldSnapshot>(value).expect("world snapshot deserializes"),
            world
        );
    }

    #[test]
    fn respawn_message_carries_only_an_operation_id() {
        let message = ClientMessage::RespawnPlayer {
            operation_id: "respawn-1".into(),
        };
        let value = serde_json::to_value(&message).expect("respawn intent serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "type": "respawn_player",
                "operation_id": "respawn-1"
            })
        );
        assert_eq!(message.operation_id(), Some("respawn-1"));
        assert_eq!(
            serde_json::from_value::<ClientMessage>(value).expect("respawn intent deserializes"),
            message
        );
    }

    #[test]
    fn grid_control_carries_inputs_instead_of_client_owned_velocity() {
        let value = serde_json::to_value(ClientMessage::SetGridControl {
            operation_id: "control-1".into(),
            grid_id: "grid-starter".into(),
            linear_input: Vec3::new(0.0, 0.0, 0.75),
            angular_input: Vec3::new(0.0, 0.2, 0.0),
            dampeners: true,
        })
        .expect("grid control serializes");
        assert_eq!(value["type"], "set_grid_control");
        assert!(value.get("linear_velocity").is_none());
        assert!(value.get("position").is_none());
        assert_eq!(value["dampeners"], true);
    }
}
