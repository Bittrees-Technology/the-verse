// SPDX-License-Identifier: Apache-2.0

//! Public, versioned protocol types shared by The Verse clients and server.
//!
//! P0 deliberately uses JSON over WebSocket so the native prototype, browser
//! tools, and test harnesses can inspect every authoritative message. A later
//! binary replication protocol must preserve these semantic contracts.

use serde::{Deserialize, Serialize};

pub mod protocol_v19;

/// The only protocol version accepted by this build.
pub const PROTOCOL_VERSION: u32 = 18;

/// The transfer-aware, actor-scoped projection contract carried by protocol 18.
pub const PROJECTION_SCHEMA_VERSION: u32 = 4;
pub const CELESTIAL_REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const UNIVERSE_MANIFEST_SCHEMA_VERSION: u32 = 4;
pub const INTEREST_SCHEMA_VERSION: u32 = 2;
pub const INTENT_FINGERPRINT_SCHEMA_VERSION: u32 = 2;
pub const LIFECYCLE_CONTROL_SCHEMA_VERSION: u32 = 2;
pub const PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION: u32 = 1;
pub const CELL_KEY_SCHEMA_VERSION: u32 = 1;
pub const CELL_DIRECTORY_SCHEMA_VERSION: u32 = 2;
pub const TRANSFER_PACKAGE_SCHEMA_VERSION: u32 = 1;

/// An exact bounded local coordinate in integer micrometres.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct I64Vec3 {
    pub x: i64,
    pub y: i64,
    pub z: i64,
}

impl I64Vec3 {
    pub const ZERO: Self = Self { x: 0, y: 0, z: 0 };

    pub const fn new(x: i64, y: i64, z: i64) -> Self {
        Self { x, y, z }
    }
}

/// Sector axes are strings on JSON surfaces so signed 128-bit coordinates
/// never pass through a JavaScript number.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorCoordinate {
    pub x: String,
    pub y: String,
    pub z: String,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
pub struct CellCoordinate {
    pub x: u32,
    pub y: u32,
    pub z: u32,
}

/// Canonical universe address schema 1. `local_um` is normalized into the
/// cell-centred half-open interval selected by the universe manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseAddress {
    pub universe_id: String,
    pub sector: SectorCoordinate,
    pub cell: CellCoordinate,
    pub local_um: I64Vec3,
}

/// Stable execution-cell identity without a cell-local position. Routing,
/// assignment, and persistence use this canonical key rather than a worker
/// name, display alias, or filesystem path.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CellKeyV1 {
    pub schema_version: u32,
    pub universe_id: String,
    pub sector: SectorCoordinate,
    pub cell: CellCoordinate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CelestialBodyKind {
    Planet,
    Moon,
    Asteroid,
    AsteroidField,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CelestialScaleClass {
    Proof,
    Production,
}

/// Immutable public registry record. Physical/environment quantities are
/// integer encoded so the registry hash is independent of floating-point
/// serialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelestialBodySnapshot {
    pub body_id: String,
    pub display_name: String,
    pub kind: CelestialBodyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_body_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field_id: Option<String>,
    pub center: UniverseAddress,
    pub surface_radius_um: u64,
    pub exclusion_radius_um: u64,
    pub fixed_orientation_microradians: I64Vec3,
    pub surface_gravity_millimetres_per_second_squared: u64,
    pub atmosphere_height_um: u64,
    pub oxygen_parts_per_million: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voxel_field_id: Option<String>,
    pub geometry_definition_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voxel_definition_id: Option<String>,
    pub material_definition_id: String,
    pub gravity_definition_id: String,
    pub atmosphere_definition_id: String,
    pub resource_definition_id: String,
    pub visual_descriptor_id: String,
    pub scale_class: CelestialScaleClass,
    pub generation_seed: String,
    pub generation_rule_version: String,
    pub materialized_registry_version: u64,
    pub content_manifest_version: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CelestialRegistrySnapshot {
    pub schema_version: u32,
    pub registry_hash: String,
    pub license: String,
    pub universe_id: String,
    pub generation_rule_version: String,
    pub minimum_fixed_body_surface_gap_um: u64,
    pub bodies: Vec<CelestialBodySnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UniverseManifestSnapshot {
    pub schema_version: u32,
    pub manifest_hash: String,
    pub universe_id: String,
    pub world_seed: String,
    pub address_schema_version: u32,
    pub sector_edge_um: u64,
    pub cell_edge_um: u64,
    pub cells_per_sector_axis: u32,
    pub generation_rule_version: String,
    pub frontier_policy_version: String,
    pub celestial_registry_schema_version: u32,
    pub celestial_registry_hash: String,
    pub content_schema_version: u32,
    pub content_manifest_version: String,
    pub content_hash: String,
    pub world_schema_version: u32,
    pub event_schema_version: u32,
    pub projection_schema_version: u32,
    pub interest_schema_version: u32,
    pub operation_fingerprint_schema_version: u32,
    pub cell_key_schema_version: u32,
    pub cell_directory_schema_version: u32,
    pub transfer_package_schema_version: u32,
    pub lifecycle_control_schema_version: u32,
    pub production_schedule_occurrence_schema_version: u32,
    pub lifecycle_policy_hash: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterestObserverClass {
    BoundPlayer,
    PublicOriginSpectator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterestFrameKind {
    Baseline,
    Delta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterestEntityKind {
    Player,
    Grid,
    VoxelChunk,
    DeathDrop,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterestEntityRef {
    pub entity_id: String,
    pub kind: InterestEntityKind,
    pub projected_revision: u64,
}

/// Complete audience-safe entity value used for first entry, re-entry, and
/// absolute replacement. Payload variants contain no actor-private overlay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "entity_kind", content = "value", rename_all = "snake_case")]
pub enum InterestEntityPayload {
    Player(PublicPlayerSnapshot),
    Grid(PublicGridSnapshot),
    VoxelChunk(PublicVoxelChunkSnapshot),
    DeathDrop(PublicDeathDropSnapshot),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterestEntityProjection {
    pub entity_id: String,
    pub kind: InterestEntityKind,
    pub projected_revision: u64,
    pub component_schema_version: u32,
    pub payload: InterestEntityPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterestRemovalReason {
    OutOfInterest,
    Destroyed,
    Transferred,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterestRemoval {
    pub entity_id: String,
    pub kind: InterestEntityKind,
    pub reason: InterestRemovalReason,
}

/// One-time private proof that a destination baseline is the continuation of
/// a committed cross-cell placement rather than an unrelated cell snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterestTransferLink {
    pub transfer_id: String,
    pub destination_cell_key: CellKeyV1,
    pub placement_generation: u64,
}

/// Connection-local replication frontier. The global commitment is retained
/// as a documented timing/hash side channel; `view_hash` is the convergence
/// commitment for the audience-safe subset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterestSnapshot {
    pub schema_version: u32,
    pub frame_kind: InterestFrameKind,
    pub session_epoch: String,
    pub interest_epoch: u64,
    pub baseline_id: String,
    pub delta_sequence: u64,
    pub observer_class: InterestObserverClass,
    pub cell_address: UniverseAddress,
    pub local_origin_address: UniverseAddress,
    pub registry_hash: String,
    pub universe_manifest_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_link: Option<InterestTransferLink>,
    pub canonical_event_sequence: u64,
    pub canonical_tick: u64,
    pub canonical_world_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_view_hash: Option<String>,
    pub view_hash: String,
    /// Complete first-entry or re-entry projections in canonical entity order.
    pub entered: Vec<InterestEntityProjection>,
    /// Complete absolute replacements for already-visible entities. These are
    /// never arithmetic patches and do not depend on hidden intermediate state.
    pub replaced: Vec<InterestEntityProjection>,
    pub removed: Vec<InterestRemoval>,
}

/// A stable integer voxel or block coordinate.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    Conveyor,
    Refinery,
    Assembler,
}

/// A manifest-registered physical production transformation. Clients select a
/// recipe kind; the authoritative server resolves all quantities, loss, power,
/// and duration from the active content manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionRecipeKind {
    Refining,
    Component,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProductionJobStatus {
    Queued,
    Running,
    PausedPower,
    PausedRoute,
    OutputBlocked,
}

/// Actor-private view of one canonical physical-production job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionJobSnapshot {
    pub job_id: String,
    pub owner_player_id: String,
    pub machine_block_id: String,
    pub recipe: ProductionRecipeKind,
    pub batches: u64,
    pub source_inventory_id: String,
    pub destination_inventory_id: String,
    pub progress_ticks: u64,
    pub duration_ticks: u64,
    pub status: ProductionJobStatus,
    pub reserved_inputs: InventoryContents,
    pub pending_outputs: InventoryContents,
}

/// Actor-private FIFO for one production machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionQueueSnapshot {
    pub machine_block_id: String,
    pub jobs: Vec<ProductionJobSnapshot>,
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
    pub address: UniverseAddress,
    /// Derived active-cell convenience pose. Canonical snapshot JSON carries
    /// only `address`; projections may derive a bounded renderer position.
    #[serde(skip, default)]
    pub position: Vec3,
    pub created_event_sequence: u64,
    pub cause: PlayerDeathCause,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct PlayerSnapshot {
    pub player_id: String,
    pub address: UniverseAddress,
    /// Derived active-cell convenience pose; never canonical snapshot bytes.
    #[serde(skip, default)]
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
#[serde(deny_unknown_fields)]
pub struct EnvironmentSnapshot {
    pub celestial_body_id: String,
    pub celestial_body_name: String,
    pub celestial_scale_class: CelestialScaleClass,
    pub nearest_body_id: String,
    pub nearest_body_name: String,
    pub planet_center: Vec3,
    pub surface_radius_m: f64,
    pub distance_to_center_m: f64,
    pub distance_to_surface_m: f64,
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
#[serde(deny_unknown_fields)]
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
    pub address: UniverseAddress,
    /// Derived active-cell convenience pose; never canonical snapshot bytes.
    #[serde(skip, default)]
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
    pub address: UniverseAddress,
    /// Derived active-cell convenience pose; never canonical motion bytes.
    #[serde(skip, default)]
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
    pub address: UniverseAddress,
    /// Derived active-cell convenience pose; never canonical motion bytes.
    #[serde(skip, default)]
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionSnapshot {
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub cell_address: UniverseAddress,
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
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub cell_address: UniverseAddress,
    pub gravity_body_id: String,
    pub voxel_body_id: String,
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
#[serde(deny_unknown_fields)]
pub struct PublicPlayerSnapshot {
    pub player_id: String,
    pub address: UniverseAddress,
    /// Renderer-only convenience pose derived from `address`.
    #[serde(skip, default)]
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
#[serde(deny_unknown_fields)]
pub struct PublicBlockSnapshot {
    pub block_id: String,
    pub coordinate: IVec3,
    pub kind: BlockKind,
    pub orientation: u8,
    pub health: u16,
    pub max_health: u16,
    pub construction_complete: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_state: Option<PublicMachineState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicMachineState {
    Idle,
    Operating,
    Paused,
    Blocked,
}

/// A public grid view. Cargo-inclusive mass is owner-private because it leaks
/// protected inventory contents through deterministic mass differences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicGridSnapshot {
    pub grid_id: String,
    pub owner_player_id: String,
    pub address: UniverseAddress,
    /// Renderer-only convenience pose derived from `address`.
    #[serde(skip, default)]
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub anchored: bool,
    pub power: PowerSnapshot,
    pub blocks: Vec<PublicBlockSnapshot>,
}

/// Public salvage marker. Ownership, source death, inventory identity, cause,
/// and contents remain actor-private authority records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicDeathDropSnapshot {
    pub drop_id: String,
    pub address: UniverseAddress,
}

/// Complete authorized replacement for one body-local voxel chunk. Individual
/// voxels remain integer body-local coordinates; clients never infer chunk
/// identity or revision from total visible voxel count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicVoxelChunkSnapshot {
    pub chunk_id: String,
    pub body_id: String,
    pub revision: u64,
    pub voxels: Vec<VoxelSnapshot>,
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
    pub production_queues: Vec<ProductionQueueSnapshot>,
}

/// Actor-aware wire projection of a canonical [`WorldSnapshot`].
///
/// The canonical `event_sequence`, `simulation_tick`, and `world_hash` remain
/// exact so clients can reconcile authoritative state. Consequently traffic
/// timing and hash changes can still reveal that hidden state changed; this
/// schema protects field values, not timing or aggregate-hash side channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedWorldSnapshot {
    pub projection_schema_version: u32,
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub universe_id: String,
    pub cell_id: String,
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub cell_address: UniverseAddress,
    pub gravity_body_id: String,
    pub voxel_body_id: String,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub fencing_token: u64,
    pub world_hash: String,
    pub players: Vec<PublicPlayerSnapshot>,
    pub environment: EnvironmentSnapshot,
    pub voxel_chunks: Vec<PublicVoxelChunkSnapshot>,
    pub grids: Vec<PublicGridSnapshot>,
    pub death_drops: Vec<PublicDeathDropSnapshot>,
    pub conservation_valid: bool,
    pub interest: InterestSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_private: Option<ActorPrivateSnapshot>,
}

/// A contiguous cumulative delta from one acknowledged interest view to the
/// newest audience-safe view. Public enters/replacements/removals live in
/// `interest`; optional private values are complete actor-bound replacements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedInterestDelta {
    pub projection_schema_version: u32,
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub universe_id: String,
    pub cell_id: String,
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub cell_address: UniverseAddress,
    pub gravity_body_id: String,
    pub voxel_body_id: String,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub world_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conservation_valid: Option<bool>,
    pub interest: InterestSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_private: Option<ActorPrivateSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_private_motion: Option<PlayerMotionSnapshot>,
}

/// Public high-rate motion for a player. Exact locomotion, input frontiers,
/// controls, and suit state are confined to the bound actor's private motion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicPlayerMotionSnapshot {
    pub player_id: String,
    pub address: UniverseAddress,
    /// Renderer-only convenience pose derived from `address`.
    #[serde(skip, default)]
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
    pub address: UniverseAddress,
    /// Renderer-only convenience pose derived from `address`.
    #[serde(skip, default)]
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
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub world_hash: String,
    pub players: Vec<PublicPlayerMotionSnapshot>,
    pub grids: Vec<PublicGridMotionSnapshot>,
    pub interest: InterestSnapshot,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoffPhase {
    Preparing,
    Importing,
    VerifyingDestination,
}

/// Private, bounded gateway presentation state for one immutable handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffStatus {
    pub transfer_id: String,
    pub phase: HandoffPhase,
    pub destination_cell_key: CellKeyV1,
    pub placement_generation: u64,
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
    AcknowledgeInterest {
        session_epoch: String,
        interest_epoch: u64,
        baseline_id: String,
        delta_sequence: u64,
        view_hash: String,
    },
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
    QueueProduction {
        operation_sequence: u64,
        operation_id: String,
        machine_block_id: String,
        recipe: ProductionRecipeKind,
        batches: u64,
        source_inventory_id: String,
        destination_inventory_id: String,
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
            Self::Hello { .. } | Self::RequestSnapshot | Self::AcknowledgeInterest { .. } => false,
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
            | Self::QueueProduction {
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
            Self::Hello { .. } | Self::RequestSnapshot | Self::AcknowledgeInterest { .. } => None,
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
            | Self::QueueProduction {
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
            Self::Hello { .. } | Self::RequestSnapshot | Self::AcknowledgeInterest { .. } => None,
            Self::SetPlayerControl { operation_id, .. }
            | Self::SetSuitMode { operation_id, .. }
            | Self::RespawnPlayer { operation_id, .. }
            | Self::MineVoxel { operation_id, .. }
            | Self::RefineOre { operation_id, .. }
            | Self::CraftComponent { operation_id, .. }
            | Self::QueueProduction { operation_id, .. }
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
        projection_schema_version: u32,
        world_schema_version: u32,
        event_schema_version: u32,
        content_schema_version: u32,
        content_manifest_version: String,
        celestial_registry_schema_version: u32,
        universe_manifest_schema_version: u32,
        interest_schema_version: u32,
        server_name: String,
        session_role: SessionRole,
    },
    Registry {
        registry: Box<CelestialRegistrySnapshot>,
        universe_manifest: Box<UniverseManifestSnapshot>,
    },
    InterestBaseline {
        baseline: Box<ProjectedWorldSnapshot>,
    },
    InterestDelta {
        delta: Box<ProjectedInterestDelta>,
    },
    Handoff {
        handoff: HandoffStatus,
    },
    /// Legacy compatibility shape. Protocol-18 official clients must use
    /// `interest_baseline` and `interest_delta` and workers must never mix the
    /// two state-stream families within one session.
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

    fn test_address() -> UniverseAddress {
        UniverseAddress {
            universe_id: "the-verse-local".into(),
            sector: SectorCoordinate {
                x: "0".into(),
                y: "0".into(),
                z: "0".into(),
            },
            cell: CellCoordinate {
                x: 500,
                y: 500,
                z: 500,
            },
            local_um: I64Vec3::ZERO,
        }
    }

    #[test]
    fn exact_address_json_rejects_unknown_fields_at_every_level() {
        let mut top = serde_json::to_value(test_address()).expect("address serializes");
        top.as_object_mut()
            .expect("address is an object")
            .insert("unexpected".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<UniverseAddress>(top).is_err());

        let mut nested = serde_json::to_value(test_address()).expect("address serializes");
        nested["sector"]
            .as_object_mut()
            .expect("sector is an object")
            .insert("unexpected".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<UniverseAddress>(nested).is_err());
    }

    #[test]
    fn protocol_v18_registry_manifest_and_interest_reject_unknown_fields() {
        let body = CelestialBodySnapshot {
            body_id: "body-test".into(),
            display_name: "Body Test".into(),
            kind: CelestialBodyKind::Asteroid,
            parent_body_id: None,
            field_id: Some("field-test".into()),
            center: test_address(),
            surface_radius_um: 1_000_000,
            exclusion_radius_um: 2_000_000,
            fixed_orientation_microradians: I64Vec3::ZERO,
            surface_gravity_millimetres_per_second_squared: 0,
            atmosphere_height_um: 0,
            oxygen_parts_per_million: 0,
            voxel_field_id: Some("voxel-test".into()),
            geometry_definition_id: "geometry-test".into(),
            voxel_definition_id: Some("voxel-definition-test".into()),
            material_definition_id: "material-test".into(),
            gravity_definition_id: "gravity-test".into(),
            atmosphere_definition_id: "atmosphere-test".into(),
            resource_definition_id: "resource-test".into(),
            visual_descriptor_id: "visual-test".into(),
            scale_class: CelestialScaleClass::Proof,
            generation_seed: "1".into(),
            generation_rule_version: "rule-1".into(),
            materialized_registry_version: 1,
            content_manifest_version: "p1.5.0".into(),
            content_hash: "content-hash".into(),
        };
        let registry = CelestialRegistrySnapshot {
            schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            registry_hash: "registry-hash".into(),
            license: "CC-BY-SA-4.0".into(),
            universe_id: "the-verse-local".into(),
            generation_rule_version: "rule-1".into(),
            minimum_fixed_body_surface_gap_um: 1,
            bodies: vec![body],
        };
        let mut registry_value = serde_json::to_value(registry).expect("registry serializes");
        registry_value["bodies"][0]
            .as_object_mut()
            .expect("body object")
            .insert("unexpected".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<CelestialRegistrySnapshot>(registry_value).is_err());

        let manifest = UniverseManifestSnapshot {
            schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            manifest_hash: "manifest-hash".into(),
            universe_id: "the-verse-local".into(),
            world_seed: "1".into(),
            address_schema_version: 1,
            sector_edge_um: 20_000_000_000_000,
            cell_edge_um: 20_000_000_000,
            cells_per_sector_axis: 1_000,
            generation_rule_version: "rule-1".into(),
            frontier_policy_version: "frontier-1".into(),
            celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            celestial_registry_hash: "registry-hash".into(),
            content_schema_version: 11,
            content_manifest_version: "p1.5.0".into(),
            content_hash: "content-hash".into(),
            world_schema_version: 20,
            event_schema_version: 16,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
            operation_fingerprint_schema_version: INTENT_FINGERPRINT_SCHEMA_VERSION,
            cell_key_schema_version: CELL_KEY_SCHEMA_VERSION,
            cell_directory_schema_version: CELL_DIRECTORY_SCHEMA_VERSION,
            transfer_package_schema_version: TRANSFER_PACKAGE_SCHEMA_VERSION,
            lifecycle_control_schema_version: LIFECYCLE_CONTROL_SCHEMA_VERSION,
            production_schedule_occurrence_schema_version:
                PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            lifecycle_policy_hash: "lifecycle-policy-hash".into(),
        };
        let mut manifest_value = serde_json::to_value(manifest).expect("manifest serializes");
        manifest_value
            .as_object_mut()
            .expect("manifest object")
            .insert("unexpected".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<UniverseManifestSnapshot>(manifest_value).is_err());

        let mut interest_value =
            serde_json::to_value(test_interest()).expect("interest serializes");
        interest_value
            .as_object_mut()
            .expect("interest object")
            .insert("unexpected".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<InterestSnapshot>(interest_value).is_err());

        let handoff = ServerMessage::Handoff {
            handoff: HandoffStatus {
                transfer_id: "transfer-proof".into(),
                phase: HandoffPhase::VerifyingDestination,
                destination_cell_key: CellKeyV1 {
                    schema_version: CELL_KEY_SCHEMA_VERSION,
                    universe_id: "the-verse-local".into(),
                    sector: test_address().sector,
                    cell: test_address().cell,
                },
                placement_generation: 2,
            },
        };
        let mut handoff_value = serde_json::to_value(handoff).expect("handoff serializes");
        assert_eq!(handoff_value["type"], "handoff");
        assert_eq!(handoff_value["handoff"]["phase"], "verifying_destination");
        handoff_value["handoff"]
            .as_object_mut()
            .expect("handoff object")
            .insert("unexpected".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<ServerMessage>(handoff_value).is_err());
    }

    fn test_environment(altitude_m: f64) -> EnvironmentSnapshot {
        EnvironmentSnapshot {
            celestial_body_id: "khepri-prime".into(),
            celestial_body_name: "Khepri Prime".into(),
            celestial_scale_class: CelestialScaleClass::Proof,
            nearest_body_id: "origin-asteroid".into(),
            nearest_body_name: "Origin Asteroid".into(),
            planet_center: Vec3::ZERO,
            surface_radius_m: 1_200.0,
            distance_to_center_m: 1_200.0 + altitude_m,
            distance_to_surface_m: altitude_m,
            altitude_m,
            gravity: Vec3::ZERO,
            gravity_m_s2: 0.0,
            atmosphere_density: 0.0,
            oxygen_fraction: 0.0,
            breathable: false,
        }
    }

    fn test_interest() -> InterestSnapshot {
        InterestSnapshot {
            schema_version: INTEREST_SCHEMA_VERSION,
            frame_kind: InterestFrameKind::Baseline,
            session_epoch: "session-test".into(),
            interest_epoch: 1,
            baseline_id: "baseline-test".into(),
            delta_sequence: 0,
            observer_class: InterestObserverClass::PublicOriginSpectator,
            cell_address: test_address(),
            local_origin_address: test_address(),
            registry_hash: "registry-hash".into(),
            universe_manifest_hash: "manifest-hash".into(),
            transfer_link: None,
            canonical_event_sequence: 8,
            canonical_tick: 13,
            canonical_world_hash: "canonical-hash".into(),
            previous_view_hash: None,
            view_hash: "view-hash".into(),
            entered: Vec::new(),
            replaced: Vec::new(),
            removed: Vec::new(),
        }
    }

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
    fn protocol_v18_preserves_tagged_life_state_and_death_cause() {
        assert_eq!(PROTOCOL_VERSION, 18);
        assert_eq!(PROJECTION_SCHEMA_VERSION, 4);
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
            address: test_address(),
            position: Vec3::new(1.0, 2.0, 3.0),
            created_event_sequence: 42,
            cause: PlayerDeathCause::OxygenDepleted,
        };
        let player = PlayerSnapshot {
            player_id: "player-local".into(),
            address: test_address(),
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
            environment: Some(test_environment(3_000.0)),
        };
        let world = WorldSnapshot {
            schema_version: 15,
            content_manifest_version: "p1.1.0".into(),
            universe_id: "the-verse-local".into(),
            cell_id: "cell-origin".into(),
            universe_manifest_hash: "manifest-hash".into(),
            celestial_registry_hash: "registry-hash".into(),
            cell_address: test_address(),
            gravity_body_id: "khepri-prime".into(),
            voxel_body_id: "origin-asteroid".into(),
            event_sequence: 42,
            simulation_tick: 0,
            fencing_token: 1,
            world_hash: "hash".into(),
            players: vec![player.clone()],
            player,
            environment: test_environment(3_000.0),
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
        assert!(value["player"].get("position").is_none());
        assert!(value["players"][0].get("position").is_none());
        assert!(value["death_drops"][0].get("position").is_none());
        let decoded = serde_json::from_value::<WorldSnapshot>(value.clone())
            .expect("world snapshot deserializes");
        assert_eq!(decoded.player.address, world.player.address);
        assert_eq!(decoded.players[0].address, world.players[0].address);
        assert_eq!(decoded.death_drops[0].address, death_drop.address);
        assert_eq!(
            serde_json::to_value(decoded).expect("decoded snapshot serializes canonically"),
            value
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
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            world_schema_version: 18,
            event_schema_version: 14,
            content_schema_version: 11,
            content_manifest_version: "p1.5.0".into(),
            celestial_registry_schema_version: CELESTIAL_REGISTRY_SCHEMA_VERSION,
            universe_manifest_schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            interest_schema_version: INTEREST_SCHEMA_VERSION,
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
            address: test_address(),
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
            universe_manifest_hash: "manifest-hash".into(),
            celestial_registry_hash: "registry-hash".into(),
            cell_address: test_address(),
            gravity_body_id: "khepri-prime".into(),
            voxel_body_id: "origin-asteroid".into(),
            event_sequence: 8,
            simulation_tick: 13,
            fencing_token: 2,
            world_hash: "canonical-hash".into(),
            players: vec![PublicPlayerSnapshot {
                player_id: "player-local".into(),
                address: test_address(),
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
            environment: test_environment(0.0),
            voxel_chunks: Vec::new(),
            grids: Vec::new(),
            death_drops: Vec::new(),
            conservation_valid: true,
            interest: test_interest(),
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
        assert_eq!(value["snapshot"]["death_drops"], serde_json::json!([]));
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
    fn protocol_v15_exposes_actor_private_production_and_queue_intent() {
        let job = ProductionJobSnapshot {
            job_id: "job-player-local-7".into(),
            owner_player_id: "player-local".into(),
            machine_block_id: "block-refinery".into(),
            recipe: ProductionRecipeKind::Refining,
            batches: 2,
            source_inventory_id: "inventory-cargo-input".into(),
            destination_inventory_id: "inventory-cargo-output".into(),
            progress_ticks: 60,
            duration_ticks: 240,
            status: ProductionJobStatus::Running,
            reserved_inputs: InventoryContents {
                ore: 4,
                refined_material: 0,
                components: 0,
            },
            pending_outputs: InventoryContents::default(),
        };
        let queue = ProductionQueueSnapshot {
            machine_block_id: job.machine_block_id.clone(),
            jobs: vec![job.clone()],
        };
        let value = serde_json::to_value(&queue).expect("production queue serializes");
        assert_eq!(value["machine_block_id"], "block-refinery");
        assert_eq!(value["jobs"][0]["recipe"], "refining");
        assert_eq!(value["jobs"][0]["status"], "running");
        assert_eq!(value["jobs"][0]["reserved_inputs"]["ore"], 4);
        assert_eq!(
            serde_json::from_value::<ProductionQueueSnapshot>(value)
                .expect("production queue deserializes"),
            queue
        );

        let intent = ClientMessage::QueueProduction {
            operation_sequence: 7,
            operation_id: "queue-production-7".into(),
            machine_block_id: "block-refinery".into(),
            recipe: ProductionRecipeKind::Refining,
            batches: 2,
            source_inventory_id: "inventory-cargo-input".into(),
            destination_inventory_id: "inventory-cargo-output".into(),
        };
        let value = serde_json::to_value(&intent).expect("queue intent serializes");
        assert_eq!(value["type"], "queue_production");
        assert_eq!(value["recipe"], "refining");
        assert_eq!(value["batches"], 2);
        assert_eq!(intent.operation_sequence(), Some(7));
        assert_eq!(intent.operation_id(), Some("queue-production-7"));
        assert_eq!(
            serde_json::from_value::<ClientMessage>(value).expect("queue intent deserializes"),
            intent
        );
    }

    #[test]
    fn protocol_v15_sequences_every_mutating_variant_and_echoes_results() {
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
            ClientMessage::QueueProduction {
                operation_sequence: 7,
                operation_id: "production".into(),
                machine_block_id: "machine".into(),
                recipe: ProductionRecipeKind::Component,
                batches: 1,
                source_inventory_id: "source".into(),
                destination_inventory_id: "destination".into(),
            },
            ClientMessage::TransferInventory {
                operation_sequence: 8,
                operation_id: "transfer".into(),
                source_inventory_id: "source".into(),
                destination_inventory_id: "destination".into(),
                resource: ResourceKind::Ore,
                quantity: 1,
            },
            ClientMessage::BuildBlock {
                operation_sequence: 9,
                operation_id: "build".into(),
                grid_id: "grid".into(),
                coordinate: IVec3::ZERO,
                kind: BlockKind::Structural,
                orientation: 0,
            },
            ClientMessage::WeldBlock {
                operation_sequence: 10,
                operation_id: "weld".into(),
                grid_id: "grid".into(),
                block_id: "block".into(),
            },
            ClientMessage::SetGridControl {
                operation_sequence: 11,
                operation_id: "grid-control".into(),
                grid_id: "grid".into(),
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                dampeners: true,
            },
            ClientMessage::ToggleGridAnchor {
                operation_sequence: 12,
                operation_id: "anchor".into(),
                grid_id: "grid".into(),
            },
            ClientMessage::DamageBlock {
                operation_sequence: 13,
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
            operation_sequence: 13,
            operation_id: "damage".into(),
            event_sequence: 44,
            code: "block_damaged".into(),
            message: "Damage applied".into(),
        };
        let value = serde_json::to_value(ServerMessage::IntentAccepted { receipt })
            .expect("accepted result serializes");
        assert_eq!(value["receipt"]["operation_sequence"], 13);
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
