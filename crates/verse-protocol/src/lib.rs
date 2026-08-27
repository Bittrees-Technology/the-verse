// SPDX-License-Identifier: Apache-2.0

//! Public, versioned protocol types shared by The Verse clients and server.
//!
//! P0 deliberately uses JSON over WebSocket so the native prototype, browser
//! tools, and test harnesses can inspect every authoritative message. A later
//! binary replication protocol must preserve these semantic contracts.

use serde::{Deserialize, Serialize};

/// The only protocol version accepted by this build.
pub const PROTOCOL_VERSION: u32 = 14;

/// The actor-aware public/private projection contract carried by protocol 13.
pub const PROJECTION_SCHEMA_VERSION: u32 = 1;
pub const INTENT_FINGERPRINT_SCHEMA_VERSION: u32 = 1;

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
    Player {
        player_id: String,
    },
    Cargo {
        block_id: String,
    },
    Dropped {
        reason: String,
        owner_player_id: String,
    },
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocomotionKind {
    Eva,
    Airborne,
    Grounded,
    Magnetic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocomotionSupportSnapshot {
    pub body_id: String,
    pub collider_id: String,
    pub local_anchor: Vec3,
    pub local_normal: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerLocomotionSnapshot {
    pub kind: LocomotionKind,
    pub up: Vec3,
    pub view_pitch_radians: f64,
    pub support: Option<LocomotionSupportSnapshot>,
    pub jump_held: bool,
    pub jump_buffer_expires_at_simulation_tick: u64,
    pub support_grace_expires_at_simulation_tick: u64,
    pub magnetic_boots_enabled: bool,
    pub magnetic_reattach_after_simulation_tick: u64,
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
    pub locomotion: PlayerLocomotionSnapshot,
    pub movement_epoch: u64,
    pub last_received_input_sequence: u64,
    pub last_processed_input_sequence: u64,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub jump: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentSnapshot>,
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
    pub owner_player_id: String,
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
    pub locomotion: PlayerLocomotionSnapshot,
    pub movement_epoch: u64,
    pub last_received_input_sequence: u64,
    pub last_processed_input_sequence: u64,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub jump: bool,
    pub control_expires_at_simulation_tick: u64,
    pub jetpack_enabled: bool,
    pub life_state: PlayerLifeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentSnapshot>,
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
    /// Deterministic canonical roster. `player` remains the primary-player
    /// compatibility view during the P1.0 client migration.
    #[serde(default)]
    pub players: Vec<PlayerMotionSnapshot>,
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
    /// Deterministic canonical roster. `player` remains the primary-player
    /// compatibility view during the P1.0 client migration.
    #[serde(default)]
    pub players: Vec<PlayerSnapshot>,
    pub environment: EnvironmentSnapshot,
    pub voxels: Vec<VoxelSnapshot>,
    pub grids: Vec<GridSnapshot>,
    pub inventories: Vec<InventorySnapshot>,
    pub death_drops: Vec<DeathDropSnapshot>,
    pub conservation: ConservationSnapshot,
}

/// Public life-state information needed to render another player without
/// exposing the protected death identity or cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicPlayerLifeState {
    Alive,
    Incapacitated,
}

/// The player state visible to every authenticated player and spectator.
/// Control values, input frontiers, inventories, progression, oxygen, and
/// private suit state intentionally exist only in the actor-private view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicPlayerSnapshot {
    pub player_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub surface_contact: bool,
    pub locomotion_kind: LocomotionKind,
    pub life_state: PublicPlayerLifeState,
    pub helmet_closed: bool,
    pub jetpack_enabled: bool,
}

/// A public block view. Cargo inventory identity is an authority edge and is
/// therefore absent even when the block itself is visible.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicBlockSnapshot {
    pub block_id: String,
    pub coordinate: IVec3,
    pub kind: BlockKind,
    pub orientation: u8,
    pub health: u16,
    pub max_health: u16,
    pub construction_complete: bool,
}

/// A public grid view. Cargo-inclusive mass is owner-private because it leaks
/// protected inventory contents through deterministic mass differences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicGridSnapshot {
    pub grid_id: String,
    pub owner_player_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub anchored: bool,
    pub power: PowerSnapshot,
    pub blocks: Vec<PublicBlockSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnedGridMassSnapshot {
    pub grid_id: String,
    pub mass_kg: f64,
}

/// The exact canonical data bound to one authenticated player. Inventory and
/// death-drop vectors contain only records whose strict durable owner resolves
/// to the same actor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorPrivateSnapshot {
    pub player: PlayerSnapshot,
    pub committed_operation_sequence: u64,
    pub inventories: Vec<InventorySnapshot>,
    pub death_drops: Vec<DeathDropSnapshot>,
    pub owned_grid_masses: Vec<OwnedGridMassSnapshot>,
}

/// Actor-aware wire projection of a canonical [`WorldSnapshot`].
///
/// The canonical `event_sequence`, `simulation_tick`, and `world_hash` remain
/// exact so clients can reconcile authoritative state. Consequently traffic
/// timing and hash changes can still reveal that hidden state changed; this
/// schema protects field values, not timing or aggregate-hash side channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectedWorldSnapshot {
    pub projection_schema_version: u32,
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub universe_id: String,
    pub cell_id: String,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub fencing_token: u64,
    pub world_hash: String,
    pub players: Vec<PublicPlayerSnapshot>,
    pub environment: EnvironmentSnapshot,
    pub voxels: Vec<VoxelSnapshot>,
    pub grids: Vec<PublicGridSnapshot>,
    pub conservation_valid: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_private: Option<ActorPrivateSnapshot>,
}

/// Public high-rate motion for a player. Exact locomotion, input frontiers,
/// controls, and suit state are confined to the bound actor's private motion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicPlayerMotionSnapshot {
    pub player_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub surface_contact: bool,
    pub locomotion_kind: LocomotionKind,
    pub life_state: PublicPlayerLifeState,
    pub jetpack_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicGridMotionSnapshot {
    pub grid_id: String,
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

/// Actor-aware wire projection of canonical high-rate motion. It preserves the
/// canonical hash and timing, with the same residual side-channel documented
/// for [`ProjectedWorldSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectedMotionSnapshot {
    pub projection_schema_version: u32,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub world_hash: String,
    pub players: Vec<PublicPlayerMotionSnapshot>,
    pub grids: Vec<PublicGridMotionSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_private: Option<PlayerMotionSnapshot>,
}

/// Authentication material is interpreted only by the connection boundary and
/// never becomes canonical simulation state. The local-development mode is
/// permitted only on a loopback-bound worker; production profiles will use a
/// short-lived session credential issued after passkey authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientAuthentication {
    Spectator,
    LocalDevelopment { player_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionRole {
    Spectator,
    Player { player_id: String },
}

/// Commands sent by all P0 clients. Every mutating command carries a contiguous
/// actor-local operation sequence for durable idempotency plus a diagnostic ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        protocol_version: u32,
        client_name: String,
        authentication: ClientAuthentication,
    },
    RequestSnapshot,
    SetPlayerControl {
        operation_sequence: u64,
        operation_id: String,
        movement_epoch: u64,
        input_sequence: u64,
        linear_input: Vec3,
        angular_input: Vec3,
        boost: bool,
        dampeners: bool,
        jump: bool,
    },
    SetSuitMode {
        operation_sequence: u64,
        operation_id: String,
        helmet_closed: bool,
        jetpack_enabled: bool,
        magnetic_boots_enabled: bool,
    },
    RespawnPlayer {
        operation_sequence: u64,
        operation_id: String,
    },
    MineVoxel {
        operation_sequence: u64,
        operation_id: String,
        coordinate: IVec3,
    },
    RefineOre {
        operation_sequence: u64,
        operation_id: String,
        inventory_id: String,
        batches: u64,
    },
    CraftComponent {
        operation_sequence: u64,
        operation_id: String,
        inventory_id: String,
        quantity: u64,
    },
    TransferInventory {
        operation_sequence: u64,
        operation_id: String,
        source_inventory_id: String,
        destination_inventory_id: String,
        resource: ResourceKind,
        quantity: u64,
    },
    BuildBlock {
        operation_sequence: u64,
        operation_id: String,
        grid_id: String,
        coordinate: IVec3,
        kind: BlockKind,
        orientation: u8,
    },
    WeldBlock {
        operation_sequence: u64,
        operation_id: String,
        grid_id: String,
        block_id: String,
    },
    SetGridControl {
        operation_sequence: u64,
        operation_id: String,
        grid_id: String,
        linear_input: Vec3,
        angular_input: Vec3,
        dampeners: bool,
    },
    ToggleGridAnchor {
        operation_sequence: u64,
        operation_id: String,
        grid_id: String,
    },
    DamageBlock {
        operation_sequence: u64,
        operation_id: String,
        grid_id: String,
        block_id: String,
    },
}

impl ClientMessage {
    pub fn set_operation_sequence(&mut self, sequence: u64) -> bool {
        match self {
            Self::Hello { .. } | Self::RequestSnapshot => false,
            Self::SetPlayerControl {
                operation_sequence, ..
            }
            | Self::SetSuitMode {
                operation_sequence, ..
            }
            | Self::RespawnPlayer {
                operation_sequence, ..
            }
            | Self::MineVoxel {
                operation_sequence, ..
            }
            | Self::RefineOre {
                operation_sequence, ..
            }
            | Self::CraftComponent {
                operation_sequence, ..
            }
            | Self::TransferInventory {
                operation_sequence, ..
            }
            | Self::BuildBlock {
                operation_sequence, ..
            }
            | Self::WeldBlock {
                operation_sequence, ..
            }
            | Self::SetGridControl {
                operation_sequence, ..
            }
            | Self::ToggleGridAnchor {
                operation_sequence, ..
            }
            | Self::DamageBlock {
                operation_sequence, ..
            } => {
                *operation_sequence = sequence;
                true
            }
        }
    }

    pub fn operation_sequence(&self) -> Option<u64> {
        match self {
            Self::Hello { .. } | Self::RequestSnapshot => None,
            Self::SetPlayerControl {
                operation_sequence, ..
            }
            | Self::SetSuitMode {
                operation_sequence, ..
            }
            | Self::RespawnPlayer {
                operation_sequence, ..
            }
            | Self::MineVoxel {
                operation_sequence, ..
            }
            | Self::RefineOre {
                operation_sequence, ..
            }
            | Self::CraftComponent {
                operation_sequence, ..
            }
            | Self::TransferInventory {
                operation_sequence, ..
            }
            | Self::BuildBlock {
                operation_sequence, ..
            }
            | Self::WeldBlock {
                operation_sequence, ..
            }
            | Self::SetGridControl {
                operation_sequence, ..
            }
            | Self::ToggleGridAnchor {
                operation_sequence, ..
            }
            | Self::DamageBlock {
                operation_sequence, ..
            } => Some(*operation_sequence),
        }
    }

    pub fn operation_id(&self) -> Option<&str> {
        match self {
            Self::Hello { .. } | Self::RequestSnapshot => None,
            Self::SetPlayerControl { operation_id, .. }
            | Self::SetSuitMode { operation_id, .. }
            | Self::RespawnPlayer { operation_id, .. }
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
    pub operation_sequence: u64,
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
        session_role: SessionRole,
    },
    Snapshot {
        snapshot: Box<ProjectedWorldSnapshot>,
    },
    MotionState {
        motion: Box<ProjectedMotionSnapshot>,
    },
    IntentAccepted {
        receipt: IntentReceipt,
    },
    IntentRejected {
        operation_sequence: Option<u64>,
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
            operation_sequence: 1,
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
            operation_sequence: 2,
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
            operation_sequence: 3,
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
            operation_sequence: 4,
            operation_id: "suit-1".into(),
            helmet_closed: false,
            jetpack_enabled: true,
            magnetic_boots_enabled: true,
        })
        .expect("suit mode intent serializes");
        assert_eq!(value["type"], "set_suit_mode");
        assert_eq!(value["helmet_closed"], false);
        assert_eq!(value["jetpack_enabled"], true);
        assert_eq!(value["magnetic_boots_enabled"], true);
    }

    #[test]
    fn protocol_v12_character_control_contains_jump_but_no_transform_or_time() {
        let message = ClientMessage::SetPlayerControl {
            operation_sequence: 5,
            operation_id: "player-control-3-41".into(),
            movement_epoch: 3,
            input_sequence: 41,
            linear_input: Vec3::new(0.0, 0.0, -1.0),
            angular_input: Vec3::new(0.0, 0.0, 0.5),
            boost: true,
            dampeners: false,
            jump: true,
        };
        let value = serde_json::to_value(&message).expect("control serializes");
        assert_eq!(value["type"], "set_player_control");
        assert_eq!(value["movement_epoch"], 3);
        assert_eq!(value["input_sequence"], 41);
        assert_eq!(value["linear_input"]["z"], -1.0);
        assert_eq!(value["angular_input"]["z"], 0.5);
        assert_eq!(value["jump"], true);
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
    fn protocol_v14_preserves_tagged_life_state_and_death_cause() {
        assert_eq!(PROTOCOL_VERSION, 14);
        assert_eq!(PROJECTION_SCHEMA_VERSION, 1);
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
    fn protocol_v12_snapshot_exposes_locomotion_input_oxygen_and_death_drops() {
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
            locomotion: PlayerLocomotionSnapshot {
                kind: LocomotionKind::Airborne,
                up: Vec3::new(0.0, 1.0, 0.0),
                view_pitch_radians: 0.0,
                support: None,
                jump_held: false,
                jump_buffer_expires_at_simulation_tick: 0,
                support_grace_expires_at_simulation_tick: 0,
                magnetic_boots_enabled: true,
                magnetic_reattach_after_simulation_tick: 0,
            },
            movement_epoch: 1,
            last_received_input_sequence: 0,
            last_processed_input_sequence: 0,
            control_linear_input: Vec3::ZERO,
            control_angular_input: Vec3::ZERO,
            boost: false,
            dampeners: true,
            jump: false,
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
            environment: Some(EnvironmentSnapshot {
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
            }),
        };
        let world = WorldSnapshot {
            schema_version: 15,
            content_manifest_version: "p1.1.0".into(),
            universe_id: "the-verse-local".into(),
            cell_id: "cell-origin".into(),
            event_sequence: 42,
            simulation_tick: 0,
            fencing_token: 1,
            world_hash: "hash".into(),
            players: vec![player.clone()],
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
        assert_eq!(value["player"]["environment"]["altitude_m"], 3_000.0);
        assert_eq!(value["players"][0]["player_id"], "player-local");
        assert_eq!(value["death_drops"][0]["drop_id"], death_drop.drop_id);
        assert_eq!(value["death_drops"][0]["cause"]["kind"], "oxygen_depleted");
        assert_eq!(
            serde_json::from_value::<WorldSnapshot>(value.clone())
                .expect("world snapshot deserializes"),
            world
        );
        let mut legacy = value;
        legacy["player"]
            .as_object_mut()
            .expect("legacy primary is an object")
            .remove("environment");
        legacy["players"][0]
            .as_object_mut()
            .expect("legacy roster player is an object")
            .remove("environment");
        let legacy = serde_json::from_value::<WorldSnapshot>(legacy)
            .expect("pre-environment player snapshots remain compatible");
        assert_eq!(legacy.player.environment, None);
        assert_eq!(legacy.players[0].environment, None);
        assert_eq!(legacy.environment, world.environment);
    }

    #[test]
    fn protocol_v12_hello_separates_authentication_from_gameplay_intents() {
        let hello = ClientMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
            client_name: "native-test".into(),
            authentication: ClientAuthentication::LocalDevelopment {
                player_id: "player-local".into(),
            },
        };
        let value = serde_json::to_value(&hello).expect("hello serializes");
        assert_eq!(value["type"], "hello");
        assert_eq!(value["authentication"]["kind"], "local_development");
        assert_eq!(value["authentication"]["player_id"], "player-local");
        assert!(hello.operation_id().is_none());

        let welcome = ServerMessage::Welcome {
            protocol_version: PROTOCOL_VERSION,
            server_name: "test".into(),
            session_role: SessionRole::Player {
                player_id: "player-local".into(),
            },
        };
        let value = serde_json::to_value(welcome).expect("welcome serializes");
        assert_eq!(value["session_role"]["kind"], "player");
        assert_eq!(value["session_role"]["player_id"], "player-local");
    }

    #[test]
    fn protocol_v12_exposes_grid_ownership_and_owner_preserving_drops() {
        let grid = GridSnapshot {
            grid_id: "grid-starter".into(),
            owner_player_id: "player-local".into(),
            position: Vec3::ZERO,
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            mass_kg: 1.0,
            anchored: false,
            power: PowerSnapshot::default(),
            blocks: Vec::new(),
        };
        let value = serde_json::to_value(&grid).expect("grid snapshot serializes");
        assert_eq!(value["owner_player_id"], "player-local");
        assert_eq!(
            serde_json::from_value::<GridSnapshot>(value).expect("grid snapshot deserializes"),
            grid
        );

        let dropped = InventoryDomain::Dropped {
            reason: "cargo_block_destroyed".into(),
            owner_player_id: "player-local".into(),
        };
        let value = serde_json::to_value(&dropped).expect("drop domain serializes");
        assert_eq!(value["kind"], "dropped");
        assert_eq!(value["owner_player_id"], "player-local");
        assert_eq!(
            serde_json::from_value::<InventoryDomain>(value).expect("drop domain deserializes"),
            dropped
        );
    }

    #[test]
    fn protocol_v13_spectator_projection_omits_actor_private_sections() {
        let snapshot = ProjectedWorldSnapshot {
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            schema_version: 15,
            content_manifest_version: "p1.1.0".into(),
            universe_id: "the-verse-local".into(),
            cell_id: "cell-origin".into(),
            event_sequence: 8,
            simulation_tick: 13,
            fencing_token: 2,
            world_hash: "canonical-hash".into(),
            players: vec![PublicPlayerSnapshot {
                player_id: "player-local".into(),
                position: Vec3::ZERO,
                orientation: Quat::IDENTITY,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                surface_contact: false,
                locomotion_kind: LocomotionKind::Eva,
                life_state: PublicPlayerLifeState::Alive,
                helmet_closed: true,
                jetpack_enabled: true,
            }],
            environment: EnvironmentSnapshot {
                celestial_body_id: "khepri-prime".into(),
                celestial_body_name: "Khepri Prime".into(),
                planet_center: Vec3::ZERO,
                surface_radius_m: 1_200.0,
                altitude_m: 0.0,
                gravity: Vec3::new(0.0, -6.2, 0.0),
                gravity_m_s2: 6.2,
                atmosphere_density: 1.0,
                oxygen_fraction: 0.21,
                breathable: true,
            },
            voxels: Vec::new(),
            grids: Vec::new(),
            conservation_valid: true,
            actor_private: None,
        };
        let message = ServerMessage::Snapshot {
            snapshot: Box::new(snapshot.clone()),
        };
        let value = serde_json::to_value(&message).expect("projected message serializes");
        assert_eq!(value["type"], "snapshot");
        assert_eq!(value["snapshot"]["world_hash"], "canonical-hash");
        assert!(value["snapshot"].get("actor_private").is_none());
        assert!(value["snapshot"].get("inventories").is_none());
        assert!(value["snapshot"].get("death_drops").is_none());
        assert_eq!(
            serde_json::from_value::<ServerMessage>(value).expect("message deserializes"),
            message
        );
    }

    #[test]
    fn respawn_message_carries_sequence_and_diagnostic_id() {
        let message = ClientMessage::RespawnPlayer {
            operation_sequence: 6,
            operation_id: "respawn-1".into(),
        };
        let value = serde_json::to_value(&message).expect("respawn intent serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "type": "respawn_player",
                "operation_sequence": 6,
                "operation_id": "respawn-1"
            })
        );
        assert_eq!(message.operation_sequence(), Some(6));
        assert_eq!(message.operation_id(), Some("respawn-1"));
        assert_eq!(
            serde_json::from_value::<ClientMessage>(value).expect("respawn intent deserializes"),
            message
        );
    }

    #[test]
    fn grid_control_carries_inputs_instead_of_client_owned_velocity() {
        let value = serde_json::to_value(ClientMessage::SetGridControl {
            operation_sequence: 7,
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

    #[test]
    fn protocol_v14_sequences_every_mutating_variant_and_echoes_results() {
        let messages = vec![
            ClientMessage::SetPlayerControl {
                operation_sequence: 1,
                operation_id: "control".into(),
                movement_epoch: 1,
                input_sequence: 1,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                dampeners: true,
                jump: false,
            },
            ClientMessage::SetSuitMode {
                operation_sequence: 2,
                operation_id: "suit".into(),
                helmet_closed: true,
                jetpack_enabled: true,
                magnetic_boots_enabled: false,
            },
            ClientMessage::RespawnPlayer {
                operation_sequence: 3,
                operation_id: "respawn".into(),
            },
            ClientMessage::MineVoxel {
                operation_sequence: 4,
                operation_id: "mine".into(),
                coordinate: IVec3::ZERO,
            },
            ClientMessage::RefineOre {
                operation_sequence: 5,
                operation_id: "refine".into(),
                inventory_id: "inventory".into(),
                batches: 1,
            },
            ClientMessage::CraftComponent {
                operation_sequence: 6,
                operation_id: "craft".into(),
                inventory_id: "inventory".into(),
                quantity: 1,
            },
            ClientMessage::TransferInventory {
                operation_sequence: 7,
                operation_id: "transfer".into(),
                source_inventory_id: "source".into(),
                destination_inventory_id: "destination".into(),
                resource: ResourceKind::Ore,
                quantity: 1,
            },
            ClientMessage::BuildBlock {
                operation_sequence: 8,
                operation_id: "build".into(),
                grid_id: "grid".into(),
                coordinate: IVec3::ZERO,
                kind: BlockKind::Structural,
                orientation: 0,
            },
            ClientMessage::WeldBlock {
                operation_sequence: 9,
                operation_id: "weld".into(),
                grid_id: "grid".into(),
                block_id: "block".into(),
            },
            ClientMessage::SetGridControl {
                operation_sequence: 10,
                operation_id: "grid-control".into(),
                grid_id: "grid".into(),
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                dampeners: true,
            },
            ClientMessage::ToggleGridAnchor {
                operation_sequence: 11,
                operation_id: "anchor".into(),
                grid_id: "grid".into(),
            },
            ClientMessage::DamageBlock {
                operation_sequence: 12,
                operation_id: "damage".into(),
                grid_id: "grid".into(),
                block_id: "block".into(),
            },
        ];
        for (index, message) in messages.iter().enumerate() {
            let expected = u64::try_from(index + 1).expect("fixture sequence fits");
            assert_eq!(message.operation_sequence(), Some(expected));
            assert_eq!(
                serde_json::to_value(message).expect("mutation serializes")["operation_sequence"],
                expected
            );
            let mut renumbered = message.clone();
            assert!(renumbered.set_operation_sequence(expected + 100));
            assert_eq!(renumbered.operation_sequence(), Some(expected + 100));
        }

        let receipt = IntentReceipt {
            operation_sequence: 12,
            operation_id: "damage".into(),
            event_sequence: 44,
            code: "block_damaged".into(),
            message: "Damage applied".into(),
        };
        let value = serde_json::to_value(ServerMessage::IntentAccepted { receipt })
            .expect("accepted result serializes");
        assert_eq!(value["receipt"]["operation_sequence"], 12);
        let value = serde_json::to_value(ServerMessage::IntentRejected {
            operation_sequence: Some(13),
            operation_id: Some("rejected".into()),
            code: "operation_sequence_gap".into(),
            message: "resynchronize".into(),
        })
        .expect("rejected result serializes");
        assert_eq!(value["operation_sequence"], 13);

        let mut request = ClientMessage::RequestSnapshot;
        assert!(!request.set_operation_sequence(1));
        assert_eq!(request.operation_sequence(), None);
    }
}
