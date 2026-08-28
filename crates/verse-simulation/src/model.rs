// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ops::{Deref, DerefMut};

use serde::{Deserialize, Serialize};
use verse_protocol::{
    BlockKind, BlockSnapshot, CareerSnapshot, CellKeyV1, ConservationSnapshot, DeathDropSnapshot,
    EnvironmentSnapshot, GridMotionSnapshot, GridSnapshot, IVec3, IntentReceipt, InventoryContents,
    InventoryDomain, InventorySnapshot, LocomotionKind, MotionSnapshot, PlayerDeathCause,
    PlayerLifeState, PlayerLocomotionSnapshot, PlayerMotionSnapshot, PlayerSnapshot, PowerSnapshot,
    ProductionJobStatus, ProductionRecipeKind, Quat, ResourceKind, UniverseAddress, Vec3,
    VoxelMaterial, VoxelSnapshot, WorldSnapshot,
};

use crate::{celestial, content};

pub const WORLD_SCHEMA_VERSION: u32 = 19;
pub const PROCESSED_OPERATION_RETENTION_LIMIT: usize = 128;
pub const PROCESSED_OPERATION_RETAINED_BYTES_LIMIT: usize = 131_072;
pub const PROCESSED_OPERATION_RECORD_BYTES_LIMIT: usize = 4_096;
pub const PLAYER_INVENTORY_ID: &str = "inventory-player-local";
pub const STARTER_GRID_ID: &str = "grid-starter";
pub const STARTER_INDUSTRY_GRID_ID: &str = "grid-industry-starter";
pub const STARTER_INDUSTRY_CARGO_INVENTORY_ID: &str = "inventory-cargo-industry-starter";
pub const PLAYER_INVENTORY_CAPACITY_LITERS: u64 = 1_200;
pub const CARGO_INVENTORY_CAPACITY_LITERS: u64 = 8_000;

pub fn planet_center() -> Vec3 {
    celestial::body_center_m(celestial::GRAVITY_BODY_ID)
}

pub fn planet_surface_radius_m() -> f64 {
    celestial::body_surface_radius_m(celestial::GRAVITY_BODY_ID)
}

pub fn planet_atmosphere_height_m() -> f64 {
    celestial::body_atmosphere_height_m(celestial::GRAVITY_BODY_ID)
}

pub fn planet_surface_gravity_m_s2() -> f64 {
    celestial::body_surface_gravity_m_s2(celestial::GRAVITY_BODY_ID)
}

pub fn valid_player_id(player_id: &str) -> bool {
    !player_id.is_empty()
        && player_id.len() <= 128
        && player_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

pub fn valid_blake3_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub fn radial_up(position: Vec3) -> Vec3 {
    let radial = position - planet_center();
    let magnitude = radial.magnitude();
    if magnitude > 1.0e-9 && magnitude.is_finite() {
        radial * (1.0 / magnitude)
    } else {
        Vec3::new(0.0, 1.0, 0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoxelField {
    pub occupied: BTreeSet<IVec3>,
    pub ferrite_ore: BTreeSet<IVec3>,
}

impl VoxelField {
    pub fn procedural_asteroid(seed: u64, radius: i32) -> Self {
        let mut occupied = BTreeSet::new();
        let mut ferrite_ore = BTreeSet::new();
        let radius_squared = i64::from(radius) * i64::from(radius);
        let extent = radius + 2;

        for x in -extent..=extent {
            for y in -extent..=extent {
                for z in -extent..=extent {
                    let coordinate = IVec3::new(x, y, z);
                    let distance_squared = coordinate.squared_distance(IVec3::ZERO);
                    let shape_noise =
                        fixed_value_noise(seed ^ 0x6A09_E667_F3BC_C909, coordinate) + 512;
                    let surface_radius_fixed = i64::from(radius) * 256 + shape_noise * 3 / 8;
                    let outside_irregular_surface =
                        distance_squared * 65_536 > surface_radius_fixed * surface_radius_fixed;
                    if distance_squared > radius_squared && outside_irregular_surface {
                        continue;
                    }
                    occupied.insert(coordinate);
                    if fixed_value_noise(seed ^ 0xBB67_AE85_84CA_A73B, coordinate) > 280 {
                        ferrite_ore.insert(coordinate);
                    }
                }
            }
        }

        Self {
            occupied,
            ferrite_ore,
        }
    }

    pub fn material(&self, coordinate: IVec3) -> Option<VoxelMaterial> {
        if !self.occupied.contains(&coordinate) {
            None
        } else if self.ferrite_ore.contains(&coordinate) {
            Some(VoxelMaterial::FerriteOre)
        } else {
            Some(VoxelMaterial::Rock)
        }
    }

    pub fn remove(&mut self, coordinate: IVec3) -> Option<VoxelMaterial> {
        let material = self.material(coordinate)?;
        self.occupied.remove(&coordinate);
        self.ferrite_ore.remove(&coordinate);
        Some(material)
    }

    pub fn snapshot(&self) -> Vec<VoxelSnapshot> {
        self.occupied
            .iter()
            .map(|coordinate| VoxelSnapshot {
                coordinate: *coordinate,
                material: if self.ferrite_ore.contains(coordinate) {
                    VoxelMaterial::FerriteOre
                } else {
                    VoxelMaterial::Rock
                },
            })
            .collect()
    }
}

fn deterministic_material_hash(seed: u64, coordinate: IVec3) -> u64 {
    let mut value = seed ^ 0x9E37_79B9_7F4A_7C15;
    for part in [coordinate.x, coordinate.y, coordinate.z] {
        value ^= i64::from(part)
            .cast_unsigned()
            .wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = value.rotate_left(27).wrapping_mul(0x94D0_49BB_1331_11EB);
    }
    value ^ (value >> 31)
}

fn fixed_value_noise(seed: u64, coordinate: IVec3) -> i64 {
    const CELL_SIZE: i32 = 4;
    let cell = IVec3::new(
        coordinate.x.div_euclid(CELL_SIZE),
        coordinate.y.div_euclid(CELL_SIZE),
        coordinate.z.div_euclid(CELL_SIZE),
    );
    let fraction = IVec3::new(
        coordinate.x.rem_euclid(CELL_SIZE),
        coordinate.y.rem_euclid(CELL_SIZE),
        coordinate.z.rem_euclid(CELL_SIZE),
    );
    let sample = |offset: IVec3| {
        let lattice = IVec3::new(cell.x + offset.x, cell.y + offset.y, cell.z + offset.z);
        i64::try_from(deterministic_material_hash(seed, lattice) & 1_023)
            .expect("ten-bit shape noise always fits i64")
            - 512
    };
    let lower_front = fixed_lerp(sample(IVec3::ZERO), sample(IVec3::new(1, 0, 0)), fraction.x);
    let upper_front = fixed_lerp(
        sample(IVec3::new(0, 1, 0)),
        sample(IVec3::new(1, 1, 0)),
        fraction.x,
    );
    let lower_back = fixed_lerp(
        sample(IVec3::new(0, 0, 1)),
        sample(IVec3::new(1, 0, 1)),
        fraction.x,
    );
    let upper_back = fixed_lerp(
        sample(IVec3::new(0, 1, 1)),
        sample(IVec3::new(1, 1, 1)),
        fraction.x,
    );
    let front = fixed_lerp(lower_front, upper_front, fraction.y);
    let back = fixed_lerp(lower_back, upper_back, fraction.y);
    fixed_lerp(front, back, fraction.z)
}

fn fixed_lerp(left: i64, right: i64, fraction: i32) -> i64 {
    const CELL_SIZE: i64 = 4;
    (left * (CELL_SIZE - i64::from(fraction)) + right * i64::from(fraction)) / CELL_SIZE
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct Player {
    pub player_id: String,
    pub address: UniverseAddress,
    /// Bounded active-cell physics pose derived from `address`.
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
    pub pending_control_frames: VecDeque<PlayerControlFrame>,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub jump: bool,
    pub control_expires_at_simulation_tick: u64,
    pub inventory_id: String,
    pub experience: u64,
    pub career: CareerSnapshot,
    pub suit_oxygen_milli: u16,
    pub helmet_closed: bool,
    pub jetpack_enabled: bool,
    pub life_state: PlayerLifeState,
}

/// Canonically ordered player ownership. Dereferencing intentionally exposes
/// the primary P0 pilot while P1 systems migrate one subsystem at a time to
/// explicit actor lookup; serialized state already has one roster source of
/// truth and cannot duplicate the primary player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerRoster {
    pub primary_player_id: String,
    pub by_id: BTreeMap<String, Player>,
}

impl PlayerRoster {
    pub fn empty() -> Self {
        Self {
            primary_player_id: String::new(),
            by_id: BTreeMap::new(),
        }
    }

    pub fn from_primary(player: Player) -> Self {
        let primary_player_id = player.player_id.clone();
        Self {
            primary_player_id: primary_player_id.clone(),
            by_id: BTreeMap::from([(primary_player_id, player)]),
        }
    }

    pub fn get(&self, player_id: &str) -> Option<&Player> {
        self.by_id.get(player_id)
    }

    pub fn get_mut(&mut self, player_id: &str) -> Option<&mut Player> {
        self.by_id.get_mut(player_id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Player)> {
        self.by_id.iter()
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.primary_player_id.is_empty() && self.by_id.is_empty() {
            return Ok(());
        }
        if self.primary_player_id.trim().is_empty()
            || !self.by_id.contains_key(&self.primary_player_id)
        {
            return Err("player roster must identify one present primary player");
        }
        if self
            .by_id
            .iter()
            .any(|(player_id, player)| player_id != &player.player_id)
        {
            return Err("player roster keys must match canonical player IDs");
        }
        Ok(())
    }

    pub fn primary(&self) -> &Player {
        self.by_id
            .get(&self.primary_player_id)
            .expect("canonical player roster contains its primary player")
    }

    pub fn primary_mut(&mut self) -> &mut Player {
        self.by_id
            .get_mut(&self.primary_player_id)
            .expect("canonical player roster contains its primary player")
    }
}

impl Deref for PlayerRoster {
    type Target = Player;

    fn deref(&self) -> &Self::Target {
        self.primary()
    }
}

impl DerefMut for PlayerRoster {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.primary_mut()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct PlayerControlFrame {
    pub input_sequence: u64,
    pub linear_input: Vec3,
    pub angular_input: Vec3,
    pub boost: bool,
    pub dampeners: bool,
    pub jump: bool,
    pub expires_at_simulation_tick: u64,
}

impl Player {
    pub fn level(&self) -> u32 {
        let mut level = 1_u32;
        let mut threshold = 100_u64;
        while self.experience >= threshold && level < 100 {
            level += 1;
            threshold += u64::from(level) * 100;
        }
        level
    }

    pub fn next_level_experience(&self) -> u64 {
        (1..=self.level()).map(|level| u64::from(level) * 100).sum()
    }

    fn snapshot(&self, environment: EnvironmentSnapshot) -> PlayerSnapshot {
        PlayerSnapshot {
            player_id: self.player_id.clone(),
            address: self.address.clone(),
            position: self.position,
            orientation: self.orientation,
            linear_velocity: self.linear_velocity,
            angular_velocity: self.angular_velocity,
            surface_contact: self.surface_contact,
            locomotion: self.locomotion.clone(),
            movement_epoch: self.movement_epoch,
            last_received_input_sequence: self.last_received_input_sequence,
            last_processed_input_sequence: self.last_processed_input_sequence,
            control_linear_input: self.control_linear_input,
            control_angular_input: self.control_angular_input,
            boost: self.boost,
            dampeners: self.dampeners,
            jump: self.jump,
            control_expires_at_simulation_tick: self.control_expires_at_simulation_tick,
            inventory_id: self.inventory_id.clone(),
            experience: self.experience,
            level: self.level(),
            next_level_experience: self.next_level_experience(),
            career: self.career.clone(),
            suit_oxygen_milli: self.suit_oxygen_milli,
            helmet_closed: self.helmet_closed,
            jetpack_enabled: self.jetpack_enabled,
            life_state: self.life_state.clone(),
            critical_oxygen_milli: content::manifest().survival.critical_oxygen_milli,
            environment: Some(environment),
        }
    }

    fn motion_snapshot(&self, environment: EnvironmentSnapshot) -> PlayerMotionSnapshot {
        PlayerMotionSnapshot {
            player_id: self.player_id.clone(),
            address: self.address.clone(),
            position: self.position,
            orientation: self.orientation,
            linear_velocity: self.linear_velocity,
            angular_velocity: self.angular_velocity,
            surface_contact: self.surface_contact,
            locomotion: self.locomotion.clone(),
            movement_epoch: self.movement_epoch,
            last_received_input_sequence: self.last_received_input_sequence,
            last_processed_input_sequence: self.last_processed_input_sequence,
            control_linear_input: self.control_linear_input,
            control_angular_input: self.control_angular_input,
            boost: self.boost,
            dampeners: self.dampeners,
            jump: self.jump,
            control_expires_at_simulation_tick: self.control_expires_at_simulation_tick,
            jetpack_enabled: self.jetpack_enabled,
            life_state: self.life_state.clone(),
            environment: Some(environment),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryRecord {
    pub inventory_id: String,
    pub domain: InventoryDomain,
    pub contents: InventoryContents,
    pub capacity_liters: u64,
}

/// Canonical machine work. Inputs leave ordinary inventory at enqueue and
/// remain here until the registered transformation completes. Completed output
/// remains here only while its destination is unavailable or full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionJob {
    pub job_id: String,
    pub operation_id: String,
    pub owner_player_id: String,
    pub machine_block_id: String,
    pub recipe: ProductionRecipeKind,
    pub content_manifest_version: String,
    pub batches: u64,
    pub source_inventory_id: String,
    pub destination_inventory_id: String,
    pub progress_ticks: u64,
    pub duration_ticks: u64,
    pub reserved_inputs: InventoryContents,
    pub pending_outputs: InventoryContents,
    pub queued_event_sequence: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProductionClock {
    pub lifecycle_generation: u64,
    pub last_committed_quantum_sequence: u64,
    pub last_scheduled_for_unix_ms: u64,
}

impl Default for ProductionClock {
    fn default() -> Self {
        Self {
            lifecycle_generation: 1,
            last_committed_quantum_sequence: 0,
            last_scheduled_for_unix_ms: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeathDrop {
    pub drop_id: String,
    pub death_id: String,
    pub inventory_id: String,
    pub owner_player_id: String,
    pub address: UniverseAddress,
    /// Bounded active-cell presentation pose derived from `address`.
    #[serde(skip, default)]
    pub position: Vec3,
    pub created_event_sequence: u64,
    pub cause: PlayerDeathCause,
}

impl InventoryRecord {
    pub fn used_liters(&self) -> u64 {
        self.contents
            .ore
            .saturating_mul(resource_unit_volume_liters(ResourceKind::Ore))
            .saturating_add(
                self.contents
                    .refined_material
                    .saturating_mul(resource_unit_volume_liters(ResourceKind::RefinedMaterial)),
            )
            .saturating_add(
                self.contents
                    .components
                    .saturating_mul(resource_unit_volume_liters(ResourceKind::Component)),
            )
    }

    pub fn mass_grams(&self) -> u64 {
        self.contents
            .ore
            .saturating_mul(resource_unit_mass_grams(ResourceKind::Ore))
            .saturating_add(
                self.contents
                    .refined_material
                    .saturating_mul(resource_unit_mass_grams(ResourceKind::RefinedMaterial)),
            )
            .saturating_add(
                self.contents
                    .components
                    .saturating_mul(resource_unit_mass_grams(ResourceKind::Component)),
            )
    }

    pub fn can_add(&self, resource: ResourceKind, quantity: u64) -> bool {
        self.used_liters()
            .saturating_add(resource_unit_volume_liters(resource).saturating_mul(quantity))
            <= self.capacity_liters
    }
}

pub const fn resource_unit_volume_liters(resource: ResourceKind) -> u64 {
    match resource {
        ResourceKind::Ore => 37,
        ResourceKind::RefinedMaterial => 15,
        ResourceKind::Component => 22,
    }
}

pub const fn resource_unit_mass_grams(resource: ResourceKind) -> u64 {
    match resource {
        ResourceKind::Ore => 3_500,
        ResourceKind::RefinedMaterial => 2_400,
        ResourceKind::Component => 4_800,
    }
}

pub fn production_recipe_quantities(
    recipe: ProductionRecipeKind,
    batches: u64,
) -> Option<(InventoryContents, InventoryContents, u64)> {
    if batches == 0 {
        return None;
    }
    match recipe {
        ProductionRecipeKind::Refining => {
            let definition = &content::manifest().recipes.refining;
            Some((
                InventoryContents {
                    ore: batches.checked_mul(definition.ore_input)?,
                    ..InventoryContents::default()
                },
                InventoryContents {
                    refined_material: batches.checked_mul(definition.refined_output)?,
                    ..InventoryContents::default()
                },
                batches.checked_mul(definition.duration_ticks_per_batch)?,
            ))
        }
        ProductionRecipeKind::Component => {
            let definition = &content::manifest().recipes.component_crafting;
            Some((
                InventoryContents {
                    refined_material: batches.checked_mul(definition.refined_input)?,
                    ..InventoryContents::default()
                },
                InventoryContents {
                    components: batches.checked_mul(definition.component_output)?,
                    ..InventoryContents::default()
                },
                batches.checked_mul(definition.duration_ticks_per_batch)?,
            ))
        }
    }
}

pub fn contents_mass_grams(contents: &InventoryContents) -> u64 {
    contents
        .ore
        .saturating_mul(resource_unit_mass_grams(ResourceKind::Ore))
        .saturating_add(
            contents
                .refined_material
                .saturating_mul(resource_unit_mass_grams(ResourceKind::RefinedMaterial)),
        )
        .saturating_add(
            contents
                .components
                .saturating_mul(resource_unit_mass_grams(ResourceKind::Component)),
        )
}

pub fn inventory_can_add_contents(
    inventory: &InventoryRecord,
    contents: &InventoryContents,
) -> bool {
    let added_liters = contents
        .ore
        .checked_mul(resource_unit_volume_liters(ResourceKind::Ore))
        .and_then(|value| {
            contents
                .refined_material
                .checked_mul(resource_unit_volume_liters(ResourceKind::RefinedMaterial))
                .and_then(|refined| value.checked_add(refined))
        })
        .and_then(|value| {
            contents
                .components
                .checked_mul(resource_unit_volume_liters(ResourceKind::Component))
                .and_then(|components| value.checked_add(components))
        });
    added_liters
        .and_then(|added| inventory.used_liters().checked_add(added))
        .is_some_and(|used| used <= inventory.capacity_liters)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub block_id: String,
    pub coordinate: IVec3,
    pub kind: BlockKind,
    pub orientation: u8,
    pub health: u16,
    pub construction_complete: bool,
    pub component_cost: u64,
    pub inventory_id: Option<String>,
}

impl Block {
    pub fn new(block_id: impl Into<String>, coordinate: IVec3, kind: BlockKind) -> Self {
        let definition = content::block(kind);
        Self {
            block_id: block_id.into(),
            coordinate,
            kind,
            orientation: 0,
            health: definition.max_health,
            construction_complete: true,
            component_cost: definition.component_cost,
            inventory_id: None,
        }
    }

    pub fn max_health(&self) -> u16 {
        content::block(self.kind).max_health
    }

    pub fn is_complete(&self) -> bool {
        self.construction_complete
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    pub grid_id: String,
    pub owner_player_id: String,
    /// One non-duplicable opportunity for the grid lineage to award its first
    /// successful anchor engagement. A split may preserve this only on its
    /// deterministic primary fragment.
    pub anchor_reward_eligible: bool,
    pub address: UniverseAddress,
    /// Bounded active-cell physics pose derived from `address`.
    #[serde(skip, default)]
    pub position: Vec3,
    pub orientation: Quat,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub control_linear_input: Vec3,
    pub control_angular_input: Vec3,
    pub dampeners: bool,
    pub anchored: bool,
    pub blocks: BTreeMap<String, Block>,
}

impl Grid {
    pub fn block_at(&self, coordinate: IVec3) -> Option<&Block> {
        self.blocks
            .values()
            .find(|block| block.coordinate == coordinate)
    }

    pub fn power(&self) -> PowerSnapshot {
        let produced = self
            .blocks
            .values()
            .filter(|block| block.is_complete())
            .map(|block| content::block(block.kind).power_production)
            .sum::<f64>();
        let required = self
            .blocks
            .values()
            .filter(|block| block.is_complete())
            .map(|block| {
                if block.kind == BlockKind::Anchor && !self.anchored {
                    0.0
                } else {
                    content::block(block.kind).power_requirement
                }
            })
            .sum::<f64>();
        let stored = self
            .blocks
            .values()
            .filter(|block| block.is_complete())
            .map(|block| content::block(block.kind).stored_power)
            .sum::<f64>();

        PowerSnapshot {
            produced,
            required,
            stored,
            online: produced + stored > 0.0 && produced + stored >= required,
        }
    }

    pub fn world_position(&self, local: IVec3) -> Vec3 {
        self.position
            + self.orientation.rotate(Vec3::new(
                f64::from(local.x),
                f64::from(local.y),
                f64::from(local.z),
            ))
    }

    pub fn world_coordinate(&self, local: IVec3) -> IVec3 {
        let position = self.world_position(local);
        IVec3::new(
            position.x.round() as i32,
            position.y.round() as i32,
            position.z.round() as i32,
        )
    }

    pub fn anchor_touches(&self, voxels: &VoxelField) -> bool {
        self.anchor_touches_after_removal(voxels, None)
    }

    pub fn anchor_touches_after_removal(
        &self,
        voxels: &VoxelField,
        removed: Option<IVec3>,
    ) -> bool {
        self.blocks.values().any(|block| {
            if block.kind != BlockKind::Anchor || !block.is_complete() {
                return false;
            }
            let world = self.world_coordinate(block.coordinate);
            let remains_occupied = |coordinate: &IVec3| {
                Some(*coordinate) != removed && voxels.occupied.contains(coordinate)
            };
            remains_occupied(&world) || world.neighbors().iter().any(remains_occupied)
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    pub genesis_ore: u64,
    pub genesis_refined: u64,
    pub genesis_components: u64,
    pub genesis_installed_components: u64,
    pub mined_ore: u64,
    pub refine_batches: u64,
    pub crafted_components: u64,
    pub built_blocks: u64,
    pub destroyed_blocks: u64,
    pub destroyed_components: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ContactPairKey {
    pub body_a: String,
    pub collider_a: String,
    pub body_b: String,
    pub collider_b: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessedOperationRecord {
    pub operation_id: String,
    pub intent_fingerprint: String,
    pub receipt: IntentReceipt,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorOperationHistory {
    pub committed_through: u64,
    pub compacted_through: u64,
    pub compacted_history_hash: String,
    pub retained: BTreeMap<u64, ProcessedOperationRecord>,
}

impl ActorOperationHistory {
    pub const fn last_sequence(&self) -> u64 {
        self.committed_through
    }
}

#[derive(Serialize)]
struct OperationCompactionMaterial<'a> {
    domain: &'static str,
    prior_hash: &'a str,
    operation_sequence: u64,
    intent_fingerprint: &'a str,
    receipt: &'a IntentReceipt,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldState {
    pub schema_version: u32,
    pub content_manifest_version: String,
    pub universe_id: String,
    pub cell_id: String,
    pub universe_manifest_hash: String,
    pub celestial_registry_hash: String,
    pub cell_address: UniverseAddress,
    pub gravity_body_id: String,
    pub voxel_body_id: String,
    pub world_seed: u64,
    pub event_sequence: u64,
    pub simulation_tick: u64,
    pub physics_step_phase: u64,
    pub active_contact_pairs: BTreeSet<ContactPairKey>,
    pub fencing_token: u64,
    pub last_event_hash: String,
    #[serde(rename = "players")]
    pub player: PlayerRoster,
    pub voxels: VoxelField,
    pub grids: BTreeMap<String, Grid>,
    pub inventories: BTreeMap<String, InventoryRecord>,
    pub production_queues: BTreeMap<String, VecDeque<ProductionJob>>,
    pub production_clock: ProductionClock,
    pub death_drops: BTreeMap<String, DeathDrop>,
    pub ledger: Ledger,
    pub processed_operations: BTreeMap<String, ActorOperationHistory>,
}

impl WorldState {
    pub fn address_for_active_position(&self, position: Vec3) -> Result<UniverseAddress, String> {
        celestial::address_from_local_position(&self.cell_address, position)
            .map_err(|source| format!("active-cell position cannot be canonicalized: {source}"))
    }

    pub fn active_position_for_address(&self, address: &UniverseAddress) -> Result<Vec3, String> {
        celestial::local_position_from_address(&self.cell_address, address)
            .map_err(|source| format!("canonical address cannot enter active physics: {source}"))
    }

    /// Restores disposable active-cell poses after persistence deserialization.
    /// Exact addresses remain the sole persisted spatial authority.
    pub fn hydrate_spatial_poses(&mut self) -> Result<(), String> {
        celestial::validate_universe_address(&self.cell_address, &self.universe_id)
            .map_err(|source| format!("world cell address is invalid: {source}"))?;
        let origin = self.cell_address.clone();
        for player in self.player.by_id.values_mut() {
            player.position = celestial::local_position_from_address(&origin, &player.address)
                .map_err(|source| {
                    format!(
                        "player {} address cannot be hydrated: {source}",
                        player.player_id
                    )
                })?;
        }
        for grid in self.grids.values_mut() {
            grid.position = celestial::local_position_from_address(&origin, &grid.address)
                .map_err(|source| {
                    format!("grid {} address cannot be hydrated: {source}", grid.grid_id)
                })?;
        }
        for drop in self.death_drops.values_mut() {
            drop.position = celestial::local_position_from_address(&origin, &drop.address)
                .map_err(|source| {
                    format!(
                        "death drop {} address cannot be hydrated: {source}",
                        drop.drop_id
                    )
                })?;
        }
        Ok(())
    }

    pub fn processed_operation_record(
        &self,
        actor_player_id: &str,
        operation_sequence: u64,
    ) -> Option<&ProcessedOperationRecord> {
        self.processed_operations
            .get(actor_player_id)
            .and_then(|history| history.retained.get(&operation_sequence))
    }

    /// Diagnostic compatibility lookup over the bounded retained suffix.
    /// Ordering and idempotency authority always use operation sequence.
    pub fn processed_operation(
        &self,
        actor_player_id: &str,
        operation_id: &str,
    ) -> Option<&IntentReceipt> {
        self.processed_operations
            .get(actor_player_id)?
            .retained
            .values()
            .find(|record| record.operation_id == operation_id)
            .map(|record| &record.receipt)
    }

    pub fn last_operation_sequence(&self, actor_player_id: &str) -> u64 {
        self.processed_operations
            .get(actor_player_id)
            .map_or(0, ActorOperationHistory::last_sequence)
    }

    pub fn record_processed_operation(
        &mut self,
        actor_player_id: &str,
        record: ProcessedOperationRecord,
    ) -> Result<(), String> {
        if record.operation_id.trim().is_empty()
            || record.operation_id.len() > 128
            || !valid_blake3_hex(&record.intent_fingerprint)
            || record.receipt.operation_id != record.operation_id
            || record.receipt.event_sequence == 0
            || record.receipt.event_sequence < record.receipt.operation_sequence
            || record.receipt.event_sequence > self.event_sequence
            || record.receipt.code.trim().is_empty()
        {
            return Err("processed operation record identity is invalid".into());
        }
        let history = self
            .processed_operations
            .entry(actor_player_id.to_owned())
            .or_default();
        let expected = history
            .committed_through
            .checked_add(1)
            .ok_or_else(|| "operation sequence space is exhausted".to_owned())?;
        if record.receipt.operation_sequence != expected {
            return Err(format!(
                "operation sequence {} does not match expected {expected}",
                record.receipt.operation_sequence
            ));
        }
        if history
            .retained
            .last_key_value()
            .is_some_and(|(_, prior)| prior.receipt.event_sequence >= record.receipt.event_sequence)
        {
            return Err(
                "processed operation receipt event sequences must advance monotonically".into(),
            );
        }
        history.committed_through = expected;
        history.retained.insert(expected, record);
        while operation_history_crosses_retention_bound(history) {
            let (&sequence, compacted) = history
                .retained
                .first_key_value()
                .expect("an over-limit operation history is nonempty");
            let material = OperationCompactionMaterial {
                domain: "the-verse-operation-compaction-v1",
                prior_hash: &history.compacted_history_hash,
                operation_sequence: sequence,
                intent_fingerprint: &compacted.intent_fingerprint,
                receipt: &compacted.receipt,
            };
            let bytes = serde_json::to_vec(&material)
                .expect("canonical operation compaction material serializes");
            history.compacted_history_hash = blake3::hash(&bytes).to_hex().to_string();
            history.retained.remove(&sequence);
            history.compacted_through = sequence;
        }
        Ok(())
    }

    pub fn validate_player_roster(&self) -> Result<(), String> {
        if self.schema_version != WORLD_SCHEMA_VERSION {
            return Err(format!(
                "world schema {} does not match required schema {WORLD_SCHEMA_VERSION}",
                self.schema_version
            ));
        }
        if self.content_manifest_version != content::manifest().manifest_version {
            return Err("world content manifest does not match the active rules".into());
        }
        if self.production_clock.lifecycle_generation == 0
            || (self.production_clock.last_committed_quantum_sequence == 0
                && self.production_clock.last_scheduled_for_unix_ms != 0)
            || (self.production_clock.last_committed_quantum_sequence > 0
                && self.production_clock.last_scheduled_for_unix_ms == 0)
        {
            return Err("world production clock frontier is invalid".into());
        }
        let registry = celestial::registry_snapshot(self.world_seed)
            .map_err(|source| format!("world celestial registry is invalid: {source}"))?;
        let universe_manifest = celestial::universe_manifest(
            self.world_seed,
            WORLD_SCHEMA_VERSION,
            crate::event::EVENT_SCHEMA_VERSION,
        )
        .map_err(|source| format!("world universe manifest is invalid: {source}"))?;
        let cell_key = celestial::cell_key_from_address(&self.cell_address)
            .map_err(|source| format!("world cell key is invalid: {source}"))?;
        let expected_cell_id = celestial::cell_id(&cell_key)
            .map_err(|source| format!("world cell ID is invalid: {source}"))?;
        let origin_cell = cell_key == celestial::cell_origin_key();
        let body_binding_valid = if origin_cell {
            self.gravity_body_id == celestial::GRAVITY_BODY_ID
                && self.voxel_body_id == celestial::VOXEL_BODY_ID
                && registry
                    .bodies
                    .iter()
                    .any(|body| body.body_id == self.gravity_body_id)
                && registry
                    .bodies
                    .iter()
                    .any(|body| body.body_id == self.voxel_body_id && body.voxel_field_id.is_some())
        } else {
            self.gravity_body_id.is_empty()
                && self.voxel_body_id.is_empty()
                && self.voxels.occupied.is_empty()
                && self.voxels.ferrite_ore.is_empty()
        };
        if self.universe_id != registry.universe_id
            || self.universe_manifest_hash != universe_manifest.manifest_hash
            || self.celestial_registry_hash != registry.registry_hash
            || self.cell_id != expected_cell_id
            || !body_binding_valid
        {
            return Err(
                "world identity must match the immutable universe manifest and celestial registry"
                    .into(),
            );
        }
        self.player.validate().map_err(str::to_owned)?;
        let mut inventory_ids = BTreeSet::new();
        let finite_vec =
            |value: Vec3| value.x.is_finite() && value.y.is_finite() && value.z.is_finite();
        for (player_id, player) in self.player.iter() {
            if !valid_player_id(player_id) {
                return Err(
                    "player IDs must be 1-128 ASCII letters, numbers, dots, hyphens, or underscores"
                        .into(),
                );
            }
            if self.grids.contains_key(&format!("player-body-{player_id}")) {
                return Err("a player physics body ID collides with a canonical grid ID".into());
            }
            let orientation_length_squared = f64::from(player.orientation.x).mul_add(
                f64::from(player.orientation.x),
                f64::from(player.orientation.y).mul_add(
                    f64::from(player.orientation.y),
                    f64::from(player.orientation.z).mul_add(
                        f64::from(player.orientation.z),
                        f64::from(player.orientation.w) * f64::from(player.orientation.w),
                    ),
                ),
            );
            if !finite_vec(player.position)
                || celestial::validate_universe_address(&player.address, &self.universe_id).is_err()
                || !celestial::local_position_from_address(&self.cell_address, &player.address)
                    .is_ok_and(|position| position == player.position)
                || !finite_vec(player.linear_velocity)
                || !finite_vec(player.angular_velocity)
                || !player.orientation.is_finite()
                || (orientation_length_squared - 1.0).abs() > 1.0e-3
                || !finite_vec(player.locomotion.up)
                || !player.locomotion.view_pitch_radians.is_finite()
            {
                return Err(
                    "player kinematics and locomotion must be finite and normalized".into(),
                );
            }
            if !inventory_ids.insert(player.inventory_id.as_str()) {
                return Err("each player must own a unique carried inventory".into());
            }
            let inventory = self
                .inventories
                .get(&player.inventory_id)
                .ok_or_else(|| "each player inventory must exist in canonical state".to_owned())?;
            if inventory.inventory_id != player.inventory_id
                || inventory.domain
                    != (InventoryDomain::Player {
                        player_id: player_id.clone(),
                    })
                || inventory.capacity_liters == 0
                || inventory.used_liters() > inventory.capacity_liters
            {
                return Err(
                    "each player inventory must have matching identity, ownership, and capacity"
                        .into(),
                );
            }
        }
        self.validate_authority_graph()?;
        Ok(())
    }

    /// Validates every durable ownership edge independently of active session
    /// state. Grid and drop owners are permitted to be offline, but their IDs,
    /// inventory linkage, and asset identities remain canonical.
    pub fn validate_authority_graph(&self) -> Result<(), String> {
        let finite_vec =
            |value: Vec3| value.x.is_finite() && value.y.is_finite() && value.z.is_finite();
        let normalized_quat = |orientation: Quat| {
            let length_squared = f64::from(orientation.x).mul_add(
                f64::from(orientation.x),
                f64::from(orientation.y).mul_add(
                    f64::from(orientation.y),
                    f64::from(orientation.z).mul_add(
                        f64::from(orientation.z),
                        f64::from(orientation.w) * f64::from(orientation.w),
                    ),
                ),
            );
            orientation.is_finite() && (length_squared - 1.0).abs() <= 1.0e-3
        };

        let mut global_block_ids = BTreeSet::new();
        let mut cargo_inventory_by_block = BTreeMap::new();
        for (grid_id, grid) in &self.grids {
            if grid_id != &grid.grid_id || grid_id.trim().is_empty() {
                return Err("grid map keys must match nonempty canonical grid IDs".into());
            }
            if !valid_player_id(&grid.owner_player_id) {
                return Err("every grid must retain one syntactically valid player owner".into());
            }
            if !finite_vec(grid.position)
                || celestial::validate_universe_address(&grid.address, &self.universe_id).is_err()
                || !celestial::local_position_from_address(&self.cell_address, &grid.address)
                    .is_ok_and(|position| position == grid.position)
                || !finite_vec(grid.linear_velocity)
                || !finite_vec(grid.angular_velocity)
                || !finite_vec(grid.control_linear_input)
                || !finite_vec(grid.control_angular_input)
                || !normalized_quat(grid.orientation)
            {
                return Err("grid kinematics and controls must be finite and normalized".into());
            }
            if grid.anchored && !grid.anchor_touches(&self.voxels) {
                return Err(
                    "an anchored grid must retain a completed voxel-touching anchor".into(),
                );
            }

            let mut coordinates = BTreeSet::new();
            for (block_id, block) in &grid.blocks {
                if block_id != &block.block_id || block_id.trim().is_empty() {
                    return Err("block map keys must match nonempty canonical block IDs".into());
                }
                if !global_block_ids.insert(block_id.as_str()) {
                    return Err("block IDs must be globally unique across all grids".into());
                }
                if !coordinates.insert(block.coordinate) {
                    return Err("a grid cannot contain two blocks at one coordinate".into());
                }
                let definition = content::block(block.kind);
                if block.orientation > 3
                    || block.component_cost != definition.component_cost
                    || block.health == 0
                    || block.health > definition.max_health
                {
                    return Err(
                        "blocks must retain canonical orientation, cost, and positive integrity"
                            .into(),
                    );
                }
                match (block.kind, &block.inventory_id) {
                    (BlockKind::Cargo, Some(inventory_id)) => {
                        if cargo_inventory_by_block
                            .insert(block_id.clone(), inventory_id.clone())
                            .is_some()
                        {
                            return Err("a cargo block may own only one inventory".into());
                        }
                    }
                    (BlockKind::Cargo, None) => {
                        return Err("every cargo block must retain its inventory identity".into());
                    }
                    (_, Some(_)) => {
                        return Err("only cargo blocks may reference an inventory".into());
                    }
                    (_, None) => {}
                }
            }
        }

        for (inventory_id, inventory) in &self.inventories {
            if inventory_id != &inventory.inventory_id
                || inventory_id.trim().is_empty()
                || inventory.capacity_liters == 0
                || inventory.used_liters() > inventory.capacity_liters
            {
                return Err(
                    "inventory keys, identities, capacity, and contents must remain canonical"
                        .into(),
                );
            }
            match &inventory.domain {
                InventoryDomain::Player { player_id } => {
                    let player = self.player.get(player_id).ok_or_else(|| {
                        "player inventory domains must reference a canonical player".to_owned()
                    })?;
                    if player.inventory_id != *inventory_id {
                        return Err(
                            "player inventory domains must match the player's carried inventory"
                                .into(),
                        );
                    }
                }
                InventoryDomain::Cargo { block_id } => {
                    if cargo_inventory_by_block.get(block_id) != Some(inventory_id) {
                        return Err(
                            "cargo inventories require one bidirectional live cargo-block link"
                                .into(),
                        );
                    }
                }
                InventoryDomain::Dropped {
                    reason,
                    owner_player_id,
                } => {
                    if reason.trim().is_empty() || !valid_player_id(owner_player_id) {
                        return Err(
                            "dropped inventories must retain a reason and valid prior owner".into(),
                        );
                    }
                }
            }
        }

        for (block_id, inventory_id) in &cargo_inventory_by_block {
            let inventory = self.inventories.get(inventory_id).ok_or_else(|| {
                "every cargo block must reference one existing inventory".to_owned()
            })?;
            if inventory.domain
                != (InventoryDomain::Cargo {
                    block_id: block_id.clone(),
                })
            {
                return Err("cargo block and inventory ownership links must agree".into());
            }
        }

        let mut production_job_ids = BTreeSet::new();
        for (machine_block_id, queue) in &self.production_queues {
            if queue.is_empty()
                || queue.len() > content::manifest().production.queue_limit_per_machine
            {
                return Err(
                    "production queues must contain between one and the registered maximum jobs"
                        .into(),
                );
            }
            let (machine_grid, machine) = self.block_grid(machine_block_id).ok_or_else(|| {
                "every production queue must reference one live machine".to_owned()
            })?;
            if !machine.is_complete()
                || !matches!(machine.kind, BlockKind::Refinery | BlockKind::Assembler)
            {
                return Err("production queues require a completed refinery or assembler".into());
            }

            for (queue_index, job) in queue.iter().enumerate() {
                if job.job_id.trim().is_empty()
                    || !production_job_ids.insert(job.job_id.as_str())
                    || job.operation_id.trim().is_empty()
                    || job.owner_player_id != machine_grid.owner_player_id
                    || job.machine_block_id != *machine_block_id
                    || job.content_manifest_version != self.content_manifest_version
                    || !content::machine_supports_recipe(machine.kind, job.recipe)
                    || job.batches == 0
                    || job.queued_event_sequence == 0
                    || job.queued_event_sequence > self.event_sequence
                    || job.progress_ticks > job.duration_ticks
                    || (queue_index > 0 && job.progress_ticks != 0)
                {
                    return Err("production job identity, ownership, recipe, order, and progress must remain canonical".into());
                }

                let (expected_inputs, expected_outputs, expected_duration) =
                    production_recipe_quantities(job.recipe, job.batches)
                        .ok_or_else(|| "production job quantities overflowed".to_owned())?;
                if job.duration_ticks != expected_duration {
                    return Err(
                        "production job duration must match its pinned registered recipe".into(),
                    );
                }
                if job.progress_ticks < job.duration_ticks {
                    if job.reserved_inputs != expected_inputs
                        || job.pending_outputs != InventoryContents::default()
                    {
                        return Err(
                            "unfinished production jobs must retain exact reserved input only"
                                .into(),
                        );
                    }
                } else if job.reserved_inputs != InventoryContents::default()
                    || job.pending_outputs != expected_outputs
                {
                    return Err(
                        "completed blocked jobs must retain exact pending output only".into(),
                    );
                }

                for inventory_id in [&job.source_inventory_id, &job.destination_inventory_id] {
                    if self.inventory_owner_player_id(inventory_id)? != job.owner_player_id {
                        return Err(
                            "production endpoints must preserve the machine owner's authority"
                                .into(),
                        );
                    }
                }
            }
        }

        let mut dropped_inventory_ids = BTreeSet::new();
        for (drop_id, drop) in &self.death_drops {
            if drop_id != &drop.drop_id
                || drop_id.trim().is_empty()
                || drop.death_id.trim().is_empty()
                || !valid_player_id(&drop.owner_player_id)
                || !finite_vec(drop.position)
                || celestial::validate_universe_address(&drop.address, &self.universe_id).is_err()
                || !celestial::local_position_from_address(&self.cell_address, &drop.address)
                    .is_ok_and(|position| position == drop.position)
                || drop.created_event_sequence > self.event_sequence
                || !dropped_inventory_ids.insert(drop.inventory_id.as_str())
            {
                return Err(
                    "death-drop identity, owner, position, and sequence must be valid".into(),
                );
            }
            let inventory = self.inventories.get(&drop.inventory_id).ok_or_else(|| {
                "every death drop must reference one existing sealed inventory".to_owned()
            })?;
            match &inventory.domain {
                InventoryDomain::Dropped {
                    owner_player_id, ..
                } if owner_player_id == &drop.owner_player_id => {}
                _ => {
                    return Err(
                        "death-drop inventory domain must preserve the death-drop owner".into(),
                    );
                }
            }
        }

        for (actor_player_id, history) in &self.processed_operations {
            if !valid_player_id(actor_player_id) || self.player.get(actor_player_id).is_none() {
                return Err("operation namespaces require present canonical player actors".into());
            }
            let retained_bytes = processed_operation_retained_bytes(&history.retained);
            if history.committed_through == 0
                || history.compacted_through > history.committed_through
                || history.committed_through > self.event_sequence
                || history.retained.len() > PROCESSED_OPERATION_RETENTION_LIMIT
                || retained_bytes > PROCESSED_OPERATION_RETAINED_BYTES_LIMIT
                || (history.compacted_through == 0 && !history.compacted_history_hash.is_empty())
                || (history.compacted_through > 0
                    && !valid_blake3_hex(&history.compacted_history_hash))
            {
                return Err("operation history bounds and commitment must remain canonical".into());
            }
            let mut last_seen = history.compacted_through;
            let mut last_receipt_event_sequence = 0;
            for (operation_sequence, record) in &history.retained {
                let expected_sequence = last_seen
                    .checked_add(1)
                    .ok_or_else(|| "operation history cannot advance beyond u64::MAX".to_owned())?;
                if *operation_sequence != expected_sequence
                    || record.operation_id.trim().is_empty()
                    || record.operation_id.len() > 128
                    || !valid_blake3_hex(&record.intent_fingerprint)
                    || processed_operation_record_bytes(record)
                        > PROCESSED_OPERATION_RECORD_BYTES_LIMIT
                    || record.receipt.operation_sequence != *operation_sequence
                    || record.receipt.operation_id != record.operation_id
                    || record.receipt.event_sequence == 0
                    || record.receipt.event_sequence < *operation_sequence
                    || record.receipt.event_sequence <= last_receipt_event_sequence
                    || record.receipt.event_sequence > self.event_sequence
                    || record.receipt.code.trim().is_empty()
                {
                    return Err("processed operations must form one bounded contiguous suffix with canonical receipts and fingerprints".into());
                }
                last_seen = *operation_sequence;
                last_receipt_event_sequence = record.receipt.event_sequence;
            }
            if last_seen != history.committed_through {
                return Err(
                    "retained operation history must cover the contiguous uncompacted suffix"
                        .into(),
                );
            }
        }
        Ok(())
    }

    /// Resolves the durable economic owner without caching cargo ownership in
    /// two mutable places. Dropped inventories preserve the owner captured at
    /// the moment their live player or grid linkage ended.
    pub fn inventory_owner_player_id(&self, inventory_id: &str) -> Result<&str, String> {
        let inventory = self
            .inventories
            .get(inventory_id)
            .ok_or_else(|| format!("inventory {inventory_id} does not exist"))?;
        match &inventory.domain {
            InventoryDomain::Player { player_id } => Ok(player_id),
            InventoryDomain::Dropped {
                owner_player_id, ..
            } => Ok(owner_player_id),
            InventoryDomain::Cargo { block_id } => {
                let mut owner = None;
                for grid in self.grids.values() {
                    if let Some(block) = grid.blocks.get(block_id) {
                        if block.inventory_id.as_deref() != Some(inventory_id) {
                            return Err(format!(
                                "cargo block {block_id} does not link back to inventory {inventory_id}"
                            ));
                        }
                        if owner.is_some() {
                            return Err(format!(
                                "cargo block {block_id} is linked from multiple grids"
                            ));
                        }
                        owner = Some(grid.owner_player_id.as_str());
                    }
                }
                owner.ok_or_else(|| format!("cargo inventory {inventory_id} has no live owner"))
            }
        }
    }

    pub fn block_grid(&self, block_id: &str) -> Option<(&Grid, &Block)> {
        self.grids
            .values()
            .find_map(|grid| grid.blocks.get(block_id).map(|block| (grid, block)))
    }

    pub fn cargo_block_for_inventory(&self, inventory_id: &str) -> Option<(&Grid, &Block)> {
        let inventory = self.inventories.get(inventory_id)?;
        let InventoryDomain::Cargo { block_id } = &inventory.domain else {
            return None;
        };
        let (grid, block) = self.block_grid(block_id)?;
        (block.kind == BlockKind::Cargo
            && block.is_complete()
            && block.inventory_id.as_deref() == Some(inventory_id))
        .then_some((grid, block))
    }

    /// Resolves the canonical completed full-face conveyor graph. World-space
    /// proximity, diagonals, corners, and separate grids never connect.
    pub fn production_route_exists(&self, machine_block_id: &str, inventory_id: &str) -> bool {
        let Some((machine_grid, machine)) = self.block_grid(machine_block_id) else {
            return false;
        };
        let Some((cargo_grid, cargo)) = self.cargo_block_for_inventory(inventory_id) else {
            return false;
        };
        if machine_grid.grid_id != cargo_grid.grid_id
            || !machine.is_complete()
            || content::block(machine.kind).conveyor_ports == 0
        {
            return false;
        }

        let mut frontier = VecDeque::from([machine.block_id.as_str()]);
        let mut visited = BTreeSet::from([machine.block_id.as_str()]);
        while let Some(block_id) = frontier.pop_front() {
            if block_id == cargo.block_id {
                return true;
            }
            let block = &machine_grid.blocks[block_id];
            for neighbor in machine_grid.blocks.values() {
                if !neighbor.is_complete()
                    || content::block(neighbor.kind).conveyor_ports == 0
                    || block.coordinate.manhattan_distance(neighbor.coordinate) != 1
                    || !visited.insert(neighbor.block_id.as_str())
                {
                    continue;
                }
                frontier.push_back(neighbor.block_id.as_str());
            }
        }
        false
    }

    pub fn production_job_status(
        &self,
        machine_block_id: &str,
        queue_index: usize,
    ) -> ProductionJobStatus {
        let Some(queue) = self.production_queues.get(machine_block_id) else {
            return ProductionJobStatus::PausedRoute;
        };
        let Some(job) = queue.get(queue_index) else {
            return ProductionJobStatus::PausedRoute;
        };
        if queue_index > 0 {
            return ProductionJobStatus::Queued;
        }
        let routes_valid = self.production_route_exists(machine_block_id, &job.source_inventory_id)
            && self.production_route_exists(machine_block_id, &job.destination_inventory_id);
        if !routes_valid {
            return ProductionJobStatus::PausedRoute;
        }
        let Some((grid, _)) = self.block_grid(machine_block_id) else {
            return ProductionJobStatus::PausedRoute;
        };
        if !grid.power().online {
            return ProductionJobStatus::PausedPower;
        }
        if job.progress_ticks == job.duration_ticks
            && self
                .inventories
                .get(&job.destination_inventory_id)
                .is_none_or(|inventory| {
                    !inventory_can_add_contents(inventory, &job.pending_outputs)
                })
        {
            return ProductionJobStatus::OutputBlocked;
        }
        ProductionJobStatus::Running
    }

    pub fn genesis(seed: u64) -> Self {
        let registry = celestial::registry_snapshot(seed)
            .expect("the embedded celestial registry is valid for the world seed");
        let universe_manifest = celestial::universe_manifest(
            seed,
            WORLD_SCHEMA_VERSION,
            crate::event::EVENT_SCHEMA_VERSION,
        )
        .expect("the embedded universe manifest is valid for the world seed");
        let cell_address = celestial::cell_origin_address();
        let canonical_address = |position| {
            celestial::address_from_local_position(&cell_address, position)
                .expect("genesis position fits the active-cell address")
        };
        let player_position = Vec3::new(12.0, 4.5, 10.0);
        let player_inventory = InventoryRecord {
            inventory_id: PLAYER_INVENTORY_ID.into(),
            domain: InventoryDomain::Player {
                player_id: "player-local".into(),
            },
            contents: InventoryContents {
                ore: 0,
                refined_material: 0,
                components: 24,
            },
            capacity_liters: PLAYER_INVENTORY_CAPACITY_LITERS,
        };
        let cargo_inventory_id = "inventory-cargo-starter".to_owned();
        let cargo_inventory = InventoryRecord {
            inventory_id: cargo_inventory_id.clone(),
            domain: InventoryDomain::Cargo {
                block_id: "block-cargo".into(),
            },
            contents: InventoryContents::default(),
            capacity_liters: CARGO_INVENTORY_CAPACITY_LITERS,
        };

        let mut blocks = BTreeMap::new();
        blocks.insert(
            "block-core".into(),
            Block::new("block-core", IVec3::ZERO, BlockKind::ControlCore),
        );
        blocks.insert(
            "block-power".into(),
            Block::new("block-power", IVec3::new(1, 0, 0), BlockKind::PowerSource),
        );
        let mut cargo_block = Block::new("block-cargo", IVec3::new(-1, 0, 0), BlockKind::Cargo);
        cargo_block.inventory_id = Some(cargo_inventory_id.clone());
        blocks.insert(cargo_block.block_id.clone(), cargo_block);
        for (block_id, coordinate) in [
            ("block-deck-a", IVec3::new(-1, 0, -1)),
            ("block-deck-b", IVec3::new(0, 0, -1)),
            ("block-deck-c", IVec3::new(1, 0, -1)),
            ("block-deck-d", IVec3::new(-1, 0, 1)),
            ("block-deck-e", IVec3::new(0, 0, 1)),
            ("block-deck-f", IVec3::new(1, 0, 1)),
            ("block-bow-a", IVec3::new(-2, 0, -2)),
            ("block-bow-b", IVec3::new(-1, 0, -2)),
            ("block-bow-c", IVec3::new(0, 0, -2)),
            ("block-bow-d", IVec3::new(1, 0, -2)),
            ("block-bow-e", IVec3::new(2, 0, -2)),
            ("block-stern-a", IVec3::new(-2, 0, 2)),
            ("block-stern-b", IVec3::new(-1, 0, 2)),
            ("block-stern-c", IVec3::new(0, 0, 2)),
            ("block-stern-d", IVec3::new(1, 0, 2)),
            ("block-stern-e", IVec3::new(2, 0, 2)),
            ("block-port-a", IVec3::new(-2, 0, -1)),
            ("block-port-b", IVec3::new(-2, 0, 1)),
            ("block-starboard-a", IVec3::new(2, 0, -1)),
            ("block-starboard-b", IVec3::new(2, 0, 1)),
        ] {
            blocks.insert(
                block_id.into(),
                Block::new(block_id, coordinate, BlockKind::Structural),
            );
        }
        blocks.insert(
            "block-battery".into(),
            Block::new("block-battery", IVec3::new(1, 1, 0), BlockKind::Battery),
        );
        blocks.insert(
            "block-drill".into(),
            Block::new("block-drill", IVec3::new(2, 0, 0), BlockKind::Drill),
        );

        let grid = Grid {
            grid_id: STARTER_GRID_ID.into(),
            owner_player_id: "player-local".into(),
            anchor_reward_eligible: true,
            address: canonical_address(Vec3::new(11.0, 0.0, 0.0)),
            position: Vec3::new(11.0, 0.0, 0.0),
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            control_linear_input: Vec3::ZERO,
            control_angular_input: Vec3::ZERO,
            dampeners: true,
            anchored: false,
            blocks,
        };

        let industry_cargo_inventory = InventoryRecord {
            inventory_id: STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
            domain: InventoryDomain::Cargo {
                block_id: "block-industry-cargo".into(),
            },
            contents: InventoryContents::default(),
            capacity_liters: CARGO_INVENTORY_CAPACITY_LITERS,
        };
        let mut industry_blocks = BTreeMap::new();
        industry_blocks.insert(
            "block-industry-core".into(),
            Block::new(
                "block-industry-core",
                IVec3::new(1, 1, 0),
                BlockKind::ControlCore,
            ),
        );
        industry_blocks.insert(
            "block-industry-power".into(),
            Block::new(
                "block-industry-power",
                IVec3::new(0, 1, 0),
                BlockKind::PowerSource,
            ),
        );
        let mut industry_cargo = Block::new(
            "block-industry-cargo",
            IVec3::new(1, 0, 0),
            BlockKind::Cargo,
        );
        industry_cargo.inventory_id = Some(STARTER_INDUSTRY_CARGO_INVENTORY_ID.into());
        industry_blocks.insert(industry_cargo.block_id.clone(), industry_cargo);
        industry_blocks.insert(
            "block-conveyor".into(),
            Block::new("block-conveyor", IVec3::new(2, 0, 0), BlockKind::Conveyor),
        );
        industry_blocks.insert(
            "block-refinery".into(),
            Block::new("block-refinery", IVec3::new(3, 0, 0), BlockKind::Refinery),
        );
        industry_blocks.insert(
            "block-assembler".into(),
            Block::new("block-assembler", IVec3::new(4, 0, 0), BlockKind::Assembler),
        );
        let industry_grid = Grid {
            grid_id: STARTER_INDUSTRY_GRID_ID.into(),
            owner_player_id: "player-local".into(),
            anchor_reward_eligible: false,
            address: canonical_address(Vec3::new(60.0, 0.0, 0.0)),
            position: Vec3::new(60.0, 0.0, 0.0),
            orientation: Quat::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            control_linear_input: Vec3::ZERO,
            control_angular_input: Vec3::ZERO,
            dampeners: true,
            anchored: false,
            blocks: industry_blocks,
        };
        let player_address = canonical_address(player_position);

        Self {
            schema_version: WORLD_SCHEMA_VERSION,
            content_manifest_version: content::manifest().manifest_version.clone(),
            universe_id: registry.universe_id,
            cell_id: celestial::cell_id(&celestial::cell_origin_key())
                .expect("embedded origin cell ID is valid"),
            universe_manifest_hash: universe_manifest.manifest_hash,
            celestial_registry_hash: registry.registry_hash,
            cell_address,
            gravity_body_id: celestial::GRAVITY_BODY_ID.into(),
            voxel_body_id: celestial::VOXEL_BODY_ID.into(),
            world_seed: seed,
            event_sequence: 0,
            simulation_tick: 0,
            physics_step_phase: 0,
            active_contact_pairs: BTreeSet::new(),
            fencing_token: 0,
            last_event_hash: String::new(),
            player: PlayerRoster::from_primary(Player {
                player_id: "player-local".into(),
                address: player_address,
                position: player_position,
                orientation: Quat::IDENTITY,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
                surface_contact: false,
                locomotion: PlayerLocomotionSnapshot {
                    kind: LocomotionKind::Eva,
                    up: radial_up(player_position),
                    view_pitch_radians: 0.0,
                    support: None,
                    jump_held: false,
                    jump_buffer_expires_at_simulation_tick: 0,
                    support_grace_expires_at_simulation_tick: 0,
                    magnetic_boots_enabled: false,
                    magnetic_reattach_after_simulation_tick: 0,
                },
                movement_epoch: 1,
                last_received_input_sequence: 0,
                last_processed_input_sequence: 0,
                pending_control_frames: VecDeque::new(),
                control_linear_input: Vec3::ZERO,
                control_angular_input: Vec3::ZERO,
                boost: false,
                dampeners: true,
                jump: false,
                control_expires_at_simulation_tick: 0,
                inventory_id: PLAYER_INVENTORY_ID.into(),
                experience: 0,
                career: CareerSnapshot::default(),
                suit_oxygen_milli: 1_000,
                helmet_closed: true,
                jetpack_enabled: true,
                life_state: PlayerLifeState::Alive,
            }),
            voxels: VoxelField::procedural_asteroid(seed, 8),
            grids: BTreeMap::from([
                (STARTER_GRID_ID.into(), grid),
                (STARTER_INDUSTRY_GRID_ID.into(), industry_grid),
            ]),
            inventories: BTreeMap::from([
                (PLAYER_INVENTORY_ID.into(), player_inventory),
                (cargo_inventory_id, cargo_inventory),
                (
                    STARTER_INDUSTRY_CARGO_INVENTORY_ID.into(),
                    industry_cargo_inventory,
                ),
            ]),
            production_queues: BTreeMap::new(),
            production_clock: ProductionClock::default(),
            death_drops: BTreeMap::new(),
            ledger: Ledger {
                genesis_components: 24,
                genesis_installed_components: 37,
                ..Ledger::default()
            },
            processed_operations: BTreeMap::new(),
        }
    }

    pub fn genesis_for_cell(seed: u64, cell_key: &CellKeyV1) -> Result<Self, String> {
        celestial::validate_cell_key(cell_key)
            .map_err(|source| format!("cell key is invalid: {source}"))?;
        if cell_key == &celestial::cell_origin_key() {
            return Ok(Self::genesis(seed));
        }

        let registry = celestial::registry_snapshot(seed)
            .map_err(|source| format!("celestial registry is invalid: {source}"))?;
        if cell_key.universe_id != registry.universe_id {
            return Err("cell key belongs to a different universe".into());
        }
        let universe_manifest = celestial::universe_manifest(
            seed,
            WORLD_SCHEMA_VERSION,
            crate::event::EVENT_SCHEMA_VERSION,
        )
        .map_err(|source| format!("universe manifest is invalid: {source}"))?;
        let state = Self {
            schema_version: WORLD_SCHEMA_VERSION,
            content_manifest_version: content::manifest().manifest_version.clone(),
            universe_id: registry.universe_id,
            cell_id: celestial::cell_id(cell_key)
                .map_err(|source| format!("cell ID is invalid: {source}"))?,
            universe_manifest_hash: universe_manifest.manifest_hash,
            celestial_registry_hash: registry.registry_hash,
            cell_address: celestial::cell_address_from_key(cell_key)
                .map_err(|source| format!("cell address is invalid: {source}"))?,
            gravity_body_id: String::new(),
            voxel_body_id: String::new(),
            world_seed: seed,
            event_sequence: 0,
            simulation_tick: 0,
            physics_step_phase: 0,
            active_contact_pairs: BTreeSet::new(),
            fencing_token: 0,
            last_event_hash: String::new(),
            player: PlayerRoster::empty(),
            voxels: VoxelField {
                occupied: BTreeSet::new(),
                ferrite_ore: BTreeSet::new(),
            },
            grids: BTreeMap::new(),
            inventories: BTreeMap::new(),
            production_queues: BTreeMap::new(),
            production_clock: ProductionClock::default(),
            death_drops: BTreeMap::new(),
            ledger: Ledger::default(),
            processed_operations: BTreeMap::new(),
        };
        state.validate_player_roster()?;
        Ok(state)
    }

    pub fn state_hash(&self) -> String {
        // A replacement worker must acquire a newer fencing token, but that
        // operational lease change does not alter the canonical world. Keeping
        // it out of the aggregate hash lets recovery prove gameplay identity.
        let mut canonical = self.clone();
        canonical.fencing_token = 0;
        let bytes = serde_json::to_vec(&canonical).expect("WorldState serialization cannot fail");
        blake3::hash(&bytes).to_hex().to_string()
    }

    pub fn conservation(&self) -> ConservationSnapshot {
        let (mut ore_live, mut refined_live, mut components_live) = self.inventories.values().fold(
            (0_u64, 0_u64, 0_u64),
            |(ore, refined, components), inventory| {
                (
                    ore + inventory.contents.ore,
                    refined + inventory.contents.refined_material,
                    components + inventory.contents.components,
                )
            },
        );
        for job in self.production_queues.values().flatten() {
            ore_live = ore_live
                .saturating_add(job.reserved_inputs.ore)
                .saturating_add(job.pending_outputs.ore);
            refined_live = refined_live
                .saturating_add(job.reserved_inputs.refined_material)
                .saturating_add(job.pending_outputs.refined_material);
            components_live = components_live
                .saturating_add(job.reserved_inputs.components)
                .saturating_add(job.pending_outputs.components);
        }
        let live_blocks = self
            .grids
            .values()
            .flat_map(|grid| grid.blocks.values())
            .map(|block| block.component_cost)
            .sum::<u64>();
        let recipes = &content::manifest().recipes;
        let ore_sources = self.ledger.genesis_ore + self.ledger.mined_ore;
        let ore_consumed = self.ledger.refine_batches * recipes.refining.ore_input;
        let refined_sources = self.ledger.genesis_refined
            + self.ledger.refine_batches * recipes.refining.refined_output;
        let refined_consumed =
            self.ledger.crafted_components * recipes.component_crafting.refined_input;
        let component_sources = self.ledger.genesis_components
            + self.ledger.genesis_installed_components
            + self.ledger.crafted_components * recipes.component_crafting.component_output;
        let components_installed_or_destroyed = live_blocks + self.ledger.destroyed_components;

        ConservationSnapshot {
            ore_sources,
            ore_live,
            ore_consumed,
            refined_sources,
            refined_live,
            refined_consumed,
            component_sources,
            components_live,
            components_installed_or_destroyed,
            valid: ore_sources == ore_live + ore_consumed
                && refined_sources == refined_live + refined_consumed
                && component_sources == components_live + components_installed_or_destroyed,
        }
    }

    pub fn grid_mass_grams(&self, grid: &Grid) -> u64 {
        let block_mass = grid
            .blocks
            .values()
            .map(|block| {
                let definition = content::block(block.kind);
                let max_health = u64::from(block.max_health());
                let minimum_health = max_health.div_ceil(10);
                let effective_health = u64::from(block.health).max(minimum_health);
                u128::from(definition.mass_grams) * u128::from(effective_health)
                    / u128::from(max_health)
            })
            .sum::<u128>();
        let inventory_mass = grid
            .blocks
            .values()
            .filter_map(|block| block.inventory_id.as_ref())
            .filter_map(|inventory_id| self.inventories.get(inventory_id))
            .map(|inventory| u128::from(inventory.mass_grams()))
            .sum::<u128>();
        let production_mass = grid
            .blocks
            .keys()
            .filter_map(|block_id| self.production_queues.get(block_id))
            .flatten()
            .map(|job| {
                u128::from(contents_mass_grams(&job.reserved_inputs))
                    + u128::from(contents_mass_grams(&job.pending_outputs))
            })
            .sum::<u128>();
        u64::try_from(
            block_mass
                .saturating_add(inventory_mass)
                .saturating_add(production_mass),
        )
        .unwrap_or(u64::MAX)
    }

    pub fn grid_mass_kg(&self, grid: &Grid) -> f64 {
        self.grid_mass_grams(grid) as f64 / 1_000.0
    }

    pub fn snapshot(&self) -> WorldSnapshot {
        let mut grids = self
            .grids
            .values()
            .map(|grid| GridSnapshot {
                grid_id: grid.grid_id.clone(),
                owner_player_id: grid.owner_player_id.clone(),
                address: grid.address.clone(),
                position: grid.position,
                orientation: grid.orientation,
                linear_velocity: grid.linear_velocity,
                angular_velocity: grid.angular_velocity,
                mass_kg: self.grid_mass_kg(grid),
                anchored: grid.anchored,
                power: grid.power(),
                blocks: grid
                    .blocks
                    .values()
                    .map(|block| BlockSnapshot {
                        block_id: block.block_id.clone(),
                        coordinate: block.coordinate,
                        kind: block.kind,
                        orientation: block.orientation,
                        health: block.health,
                        max_health: block.max_health(),
                        construction_complete: block.construction_complete,
                        inventory_id: block.inventory_id.clone(),
                    })
                    .collect(),
            })
            .collect::<Vec<_>>();
        grids.sort_by(|left, right| left.grid_id.cmp(&right.grid_id));
        let primary_environment = self.environment_at(self.player.position);

        WorldSnapshot {
            schema_version: self.schema_version,
            content_manifest_version: self.content_manifest_version.clone(),
            universe_id: self.universe_id.clone(),
            cell_id: self.cell_id.clone(),
            universe_manifest_hash: self.universe_manifest_hash.clone(),
            celestial_registry_hash: self.celestial_registry_hash.clone(),
            cell_address: self.cell_address.clone(),
            gravity_body_id: self.gravity_body_id.clone(),
            voxel_body_id: self.voxel_body_id.clone(),
            event_sequence: self.event_sequence,
            simulation_tick: self.simulation_tick,
            fencing_token: self.fencing_token,
            world_hash: self.state_hash(),
            player: self.player.snapshot(primary_environment.clone()),
            players: self
                .player
                .iter()
                .map(|(_, player)| player.snapshot(self.environment_at(player.position)))
                .collect(),
            environment: primary_environment,
            voxels: self.voxels.snapshot(),
            grids,
            inventories: self
                .inventories
                .values()
                .map(|inventory| InventorySnapshot {
                    inventory_id: inventory.inventory_id.clone(),
                    domain: inventory.domain.clone(),
                    contents: inventory.contents.clone(),
                    capacity_liters: inventory.capacity_liters,
                    used_liters: inventory.used_liters(),
                    mass_grams: inventory.mass_grams(),
                })
                .collect(),
            death_drops: self
                .death_drops
                .values()
                .map(|drop| DeathDropSnapshot {
                    drop_id: drop.drop_id.clone(),
                    death_id: drop.death_id.clone(),
                    inventory_id: drop.inventory_id.clone(),
                    owner_player_id: drop.owner_player_id.clone(),
                    address: drop.address.clone(),
                    position: drop.position,
                    created_event_sequence: drop.created_event_sequence,
                    cause: drop.cause,
                })
                .collect(),
            conservation: self.conservation(),
        }
    }

    pub fn motion_snapshot(&self) -> MotionSnapshot {
        let primary_environment = self.environment_at(self.player.position);
        MotionSnapshot {
            universe_manifest_hash: self.universe_manifest_hash.clone(),
            celestial_registry_hash: self.celestial_registry_hash.clone(),
            cell_address: self.cell_address.clone(),
            event_sequence: self.event_sequence,
            simulation_tick: self.simulation_tick,
            world_hash: self.state_hash(),
            player: self.player.motion_snapshot(primary_environment),
            players: self
                .player
                .iter()
                .map(|(_, player)| player.motion_snapshot(self.environment_at(player.position)))
                .collect(),
            grids: self
                .grids
                .values()
                .map(|grid| GridMotionSnapshot {
                    grid_id: grid.grid_id.clone(),
                    address: grid.address.clone(),
                    position: grid.position,
                    orientation: grid.orientation,
                    linear_velocity: grid.linear_velocity,
                    angular_velocity: grid.angular_velocity,
                })
                .collect(),
        }
    }

    pub fn environment_at(&self, position: Vec3) -> EnvironmentSnapshot {
        let gravity_body = celestial::body_snapshot(self.world_seed, &self.gravity_body_id);
        let registry = celestial::registry_snapshot(self.world_seed)
            .expect("the world-bound celestial registry remains valid");
        let nearest_body = registry
            .bodies
            .iter()
            .filter(|body| body.kind != verse_protocol::CelestialBodyKind::AsteroidField)
            .min_by(|left, right| {
                let left_center = celestial::body_center_m(&left.body_id);
                let right_center = celestial::body_center_m(&right.body_id);
                let left_distance = (position - left_center).magnitude()
                    - left.surface_radius_um as f64 / 1_000_000.0;
                let right_distance = (position - right_center).magnitude()
                    - right.surface_radius_um as f64 / 1_000_000.0;
                left_distance.total_cmp(&right_distance)
            })
            .expect("the registry always contains the proof bodies");
        let radial = Vec3::new(
            position.x - planet_center().x,
            position.y - planet_center().y,
            position.z - planet_center().z,
        );
        let distance = radial.magnitude().max(1.0);
        let altitude_m = (distance - planet_surface_radius_m()).max(0.0);
        let gravity_m_s2 = (planet_surface_gravity_m_s2()
            * (planet_surface_radius_m() / distance).powi(2))
        .min(planet_surface_gravity_m_s2() * 1.25);
        let gravity = Vec3::new(
            -radial.x / distance * gravity_m_s2,
            -radial.y / distance * gravity_m_s2,
            -radial.z / distance * gravity_m_s2,
        );
        let atmosphere_density = (1.0 - altitude_m / planet_atmosphere_height_m()).clamp(0.0, 1.0);
        let oxygen_fraction = if atmosphere_density > 0.0 {
            f64::from(gravity_body.oxygen_parts_per_million) / 1_000_000.0
        } else {
            0.0
        };

        EnvironmentSnapshot {
            celestial_body_id: gravity_body.body_id,
            celestial_body_name: gravity_body.display_name,
            celestial_scale_class: gravity_body.scale_class,
            nearest_body_id: nearest_body.body_id.clone(),
            nearest_body_name: nearest_body.display_name.clone(),
            planet_center: planet_center(),
            surface_radius_m: planet_surface_radius_m(),
            distance_to_center_m: distance,
            distance_to_surface_m: distance - planet_surface_radius_m(),
            altitude_m,
            gravity,
            gravity_m_s2,
            atmosphere_density,
            oxygen_fraction,
            breathable: atmosphere_density >= 0.35 && oxygen_fraction >= 0.18,
        }
    }
}

fn processed_operation_record_bytes(record: &ProcessedOperationRecord) -> usize {
    serde_json::to_vec(record)
        .expect("canonical processed operation record serializes")
        .len()
}

/// Canonical byte budget for the retained suffix. Serializing the complete map
/// includes operation-sequence keys and collection delimiters as well as every
/// record, so validation and compaction measure exactly the same representation.
fn processed_operation_retained_bytes(retained: &BTreeMap<u64, ProcessedOperationRecord>) -> usize {
    serde_json::to_vec(retained)
        .expect("canonical processed operation retained map serializes")
        .len()
}

fn operation_history_crosses_retention_bound(history: &ActorOperationHistory) -> bool {
    history.retained.len() > PROCESSED_OPERATION_RETENTION_LIMIT
        || processed_operation_retained_bytes(&history.retained)
            > PROCESSED_OPERATION_RETAINED_BYTES_LIMIT
        || history.retained.values().any(|record| {
            processed_operation_record_bytes(record) > PROCESSED_OPERATION_RECORD_BYTES_LIMIT
        })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn genesis_binds_the_exact_registry_manifest_and_active_cell() {
        let world = WorldState::genesis(127);
        let registry = celestial::registry_snapshot(127).expect("registry builds");
        let manifest = celestial::universe_manifest(
            127,
            WORLD_SCHEMA_VERSION,
            crate::event::EVENT_SCHEMA_VERSION,
        )
        .expect("universe manifest builds");

        assert_eq!(world.universe_manifest_hash, manifest.manifest_hash);
        assert_eq!(world.celestial_registry_hash, registry.registry_hash);
        assert_eq!(world.cell_address, celestial::cell_origin_address());
        assert_eq!(
            world.cell_id,
            celestial::cell_id(&celestial::cell_origin_key()).expect("origin cell ID derives")
        );
        assert_eq!(world.gravity_body_id, celestial::GRAVITY_BODY_ID);
        assert_eq!(world.voxel_body_id, celestial::VOXEL_BODY_ID);
        assert!(world.validate_player_roster().is_ok());
    }

    #[test]
    fn adjacent_cell_genesis_is_empty_canonical_and_conserved() {
        let origin = celestial::cell_origin_key();
        let east =
            celestial::neighbor_cell_key(&origin, [1, 0, 0]).expect("adjacent proof cell derives");
        let world = WorldState::genesis_for_cell(128, &east).expect("empty cell builds");

        assert_eq!(
            world.cell_id,
            celestial::cell_id(&east).expect("east cell ID derives")
        );
        assert_eq!(
            celestial::cell_key_from_address(&world.cell_address).expect("world key derives"),
            east
        );
        assert!(world.player.by_id.is_empty());
        assert!(world.grids.is_empty());
        assert!(world.inventories.is_empty());
        assert!(world.voxels.occupied.is_empty());
        assert!(world.voxels.ferrite_ore.is_empty());
        assert!(world.gravity_body_id.is_empty());
        assert!(world.voxel_body_id.is_empty());
        assert!(world.conservation().valid);
        assert!(world.validate_player_roster().is_ok());
    }

    fn world_with_exact_death_drop(seed: u64) -> WorldState {
        let mut world = WorldState::genesis(seed);
        world.event_sequence = 1;
        let inventory_id = "inventory-drop-spatial-test".to_owned();
        world.inventories.insert(
            inventory_id.clone(),
            InventoryRecord {
                inventory_id: inventory_id.clone(),
                domain: InventoryDomain::Dropped {
                    reason: "spatial_persistence_test".into(),
                    owner_player_id: world.player.player_id.clone(),
                },
                contents: InventoryContents::default(),
                capacity_liters: PLAYER_INVENTORY_CAPACITY_LITERS,
            },
        );
        world.death_drops.insert(
            "drop-spatial-test".into(),
            DeathDrop {
                drop_id: "drop-spatial-test".into(),
                death_id: "death-spatial-test".into(),
                inventory_id,
                owner_player_id: world.player.player_id.clone(),
                address: world.player.address.clone(),
                position: world.player.position,
                created_event_sequence: 1,
                cause: PlayerDeathCause::OxygenDepleted,
            },
        );
        world
    }

    #[test]
    fn canonical_world_json_persists_exact_addresses_but_not_derived_poses() {
        let world = world_with_exact_death_drop(128);
        assert!(world.validate_player_roster().is_ok());
        let expected_hash = world.state_hash();
        let value = serde_json::to_value(&world).expect("world serializes");
        let player = &value["players"]["by_id"]["player-local"];
        let grid = &value["grids"][STARTER_GRID_ID];
        let drop = &value["death_drops"]["drop-spatial-test"];
        for spatial in [player, grid, drop] {
            assert!(spatial.get("address").is_some());
            assert!(spatial.get("position").is_none());
        }

        let mut decoded = serde_json::from_value::<WorldState>(value.clone())
            .expect("exact world JSON deserializes");
        assert_eq!(decoded.player.position, Vec3::ZERO);
        assert_eq!(decoded.grids[STARTER_GRID_ID].position, Vec3::ZERO);
        assert_eq!(
            decoded.death_drops["drop-spatial-test"].position,
            Vec3::ZERO
        );
        assert_eq!(decoded.state_hash(), expected_hash);
        decoded
            .hydrate_spatial_poses()
            .expect("exact addresses hydrate bounded poses");
        assert_eq!(decoded.player.address, world.player.address);
        assert_eq!(decoded.player.position, world.player.position);
        assert_eq!(
            decoded.grids[STARTER_GRID_ID].address,
            world.grids[STARTER_GRID_ID].address
        );
        assert_eq!(
            decoded.death_drops["drop-spatial-test"].address,
            world.death_drops["drop-spatial-test"].address
        );
        assert_eq!(decoded.state_hash(), expected_hash);
        assert!(decoded.validate_player_roster().is_ok());
    }

    #[test]
    fn exact_spatial_persistence_rejects_unknown_and_stale_inputs() {
        let world = WorldState::genesis(129);
        let mut unknown_world = serde_json::to_value(&world).expect("world serializes");
        unknown_world
            .as_object_mut()
            .expect("world is an object")
            .insert("unexpected_world_field".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<WorldState>(unknown_world).is_err());

        let mut unknown_player = serde_json::to_value(&world).expect("world serializes");
        unknown_player["players"]["by_id"]["player-local"]
            .as_object_mut()
            .expect("player is an object")
            .insert("unexpected_spatial_field".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<WorldState>(unknown_player).is_err());

        let mut stale_pose = world.clone();
        stale_pose.player.position.x += 0.000_000_4;
        assert!(stale_pose.validate_player_roster().is_err());

        let mut stale_address = world;
        stale_address
            .grids
            .get_mut(STARTER_GRID_ID)
            .expect("starter grid exists")
            .address
            .local_um
            .x += 1;
        assert!(stale_address.validate_player_roster().is_err());

        let mut wrong_universe = WorldState::genesis(129);
        wrong_universe.player.address.universe_id = "another-universe".into();
        assert!(wrong_universe.validate_player_roster().is_err());

        let mut noncanonical = WorldState::genesis(129);
        noncanonical.player.address.local_um.x =
            i64::try_from(celestial::CELL_EDGE_UM / 2).expect("half-cell fits i64");
        assert!(noncanonical.validate_player_roster().is_err());

        let mut unsafe_distance = WorldState::genesis(129);
        unsafe_distance.player.address.sector.x = i128::MAX.to_string();
        assert!(unsafe_distance.hydrate_spatial_poses().is_err());
    }

    #[test]
    fn environment_uses_registered_gravity_and_nearest_body_identity() {
        let world = WorldState::genesis(131);
        let environment = world.environment_at(Vec3::new(12.0, 4.5, 10.0));

        assert_eq!(environment.celestial_body_id, celestial::GRAVITY_BODY_ID);
        assert_eq!(environment.celestial_body_name, "Khepri Prime");
        assert_eq!(
            environment.celestial_scale_class,
            verse_protocol::CelestialScaleClass::Proof
        );
        assert_eq!(environment.nearest_body_id, celestial::VOXEL_BODY_ID);
        assert_eq!(environment.nearest_body_name, "Origin Ferrite Asteroid");
        assert!((environment.surface_radius_m - 1_200.0).abs() <= f64::EPSILON);
        assert!(environment.distance_to_center_m > environment.surface_radius_m);
        assert!(
            (environment.distance_to_surface_m
                - (environment.distance_to_center_m - environment.surface_radius_m))
                .abs()
                <= f64::EPSILON
        );
        assert!(!environment.breathable);
    }

    #[test]
    fn validation_fails_closed_on_registry_or_manifest_substitution() {
        let mut registry_substitution = WorldState::genesis(137);
        registry_substitution.celestial_registry_hash = "0".repeat(64);
        assert!(
            registry_substitution
                .validate_player_roster()
                .expect_err("registry substitution fails closed")
                .contains("immutable universe manifest")
        );

        let mut manifest_substitution = WorldState::genesis(139);
        manifest_substitution.universe_manifest_hash = "f".repeat(64);
        assert!(manifest_substitution.validate_player_roster().is_err());

        let mut body_substitution = WorldState::genesis(149);
        body_substitution.voxel_body_id = celestial::GRAVITY_BODY_ID.into();
        assert!(body_substitution.validate_player_roster().is_err());
    }

    fn synthetic_operation_record(operation_sequence: u64) -> ProcessedOperationRecord {
        let operation_id = format!("synthetic-{operation_sequence}");
        ProcessedOperationRecord {
            operation_id: operation_id.clone(),
            intent_fingerprint: blake3::hash(operation_id.as_bytes()).to_hex().to_string(),
            receipt: IntentReceipt {
                operation_sequence,
                operation_id,
                event_sequence: operation_sequence,
                code: "synthetic_committed".into(),
                message: "synthetic bounded-history receipt".into(),
            },
        }
    }

    fn synthetic_operation_record_with_size(
        operation_sequence: u64,
        target_bytes: usize,
    ) -> ProcessedOperationRecord {
        let mut record = synthetic_operation_record(operation_sequence);
        record.receipt.message.clear();
        let base_bytes = processed_operation_record_bytes(&record);
        assert!(target_bytes >= base_bytes);
        record.receipt.message = "x".repeat(target_bytes - base_bytes);
        assert_eq!(processed_operation_record_bytes(&record), target_bytes);
        record
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]

        #[test]
        fn operation_compaction_preserves_a_deterministic_bounded_suffix(
            operation_count in 129_u16..400,
        ) {
            let operation_count = u64::from(operation_count);
            let mut first = WorldState::genesis(97);
            let mut second = first.clone();
            first.event_sequence = operation_count;
            second.event_sequence = operation_count;

            for operation_sequence in 1..=operation_count {
                first
                    .record_processed_operation(
                        "player-local",
                        synthetic_operation_record(operation_sequence),
                    )
                    .expect("synthetic operation records contiguously");
                second
                    .record_processed_operation(
                        "player-local",
                        synthetic_operation_record(operation_sequence),
                    )
                    .expect("the duplicate campaign records contiguously");
            }

            let first_history = &first.processed_operations["player-local"];
            let second_history = &second.processed_operations["player-local"];
            prop_assert_eq!(first_history, second_history);
            prop_assert_eq!(first_history.committed_through, operation_count);
            prop_assert!(first_history.retained.len() <= PROCESSED_OPERATION_RETENTION_LIMIT);
            prop_assert!(
                processed_operation_retained_bytes(&first_history.retained)
                    <= PROCESSED_OPERATION_RETAINED_BYTES_LIMIT
            );
            let all_records_bounded = first_history.retained.values().all(|record| {
                processed_operation_record_bytes(record)
                    <= PROCESSED_OPERATION_RECORD_BYTES_LIMIT
            });
            prop_assert!(all_records_bounded);
            prop_assert_eq!(
                first_history.compacted_through
                    + u64::try_from(first_history.retained.len()).expect("retained bound fits u64"),
                operation_count
            );
            prop_assert!(first.validate_player_roster().is_ok());
        }
    }

    #[test]
    fn operation_history_rejects_impossible_global_and_receipt_frontiers() {
        let mut impossible_compacted = WorldState::genesis(103);
        impossible_compacted.event_sequence = 1;
        impossible_compacted.processed_operations.insert(
            "player-local".into(),
            ActorOperationHistory {
                committed_through: 100,
                compacted_through: 100,
                compacted_history_hash: "0".repeat(64),
                retained: BTreeMap::new(),
            },
        );
        assert!(
            impossible_compacted
                .validate_player_roster()
                .expect_err("an actor frontier cannot exceed the global event frontier")
                .contains("operation history bounds")
        );

        let mut first = synthetic_operation_record(1);
        first.receipt.event_sequence = 3;
        let mut second = synthetic_operation_record(2);
        second.receipt.event_sequence = 2;
        let mut out_of_order = WorldState::genesis(107);
        out_of_order.event_sequence = 3;
        out_of_order.processed_operations.insert(
            "player-local".into(),
            ActorOperationHistory {
                committed_through: 2,
                compacted_through: 0,
                compacted_history_hash: String::new(),
                retained: BTreeMap::from([(1, first), (2, second)]),
            },
        );
        assert!(
            out_of_order
                .validate_player_roster()
                .expect_err("retained receipt events must advance with actor operations")
                .contains("canonical receipts")
        );

        let mut zero_receipt = WorldState::genesis(109);
        zero_receipt.event_sequence = 1;
        let mut record = synthetic_operation_record(1);
        record.receipt.event_sequence = 0;
        zero_receipt.processed_operations.insert(
            "player-local".into(),
            ActorOperationHistory {
                committed_through: 1,
                compacted_through: 0,
                compacted_history_hash: String::new(),
                retained: BTreeMap::from([(1, record)]),
            },
        );
        assert!(zero_receipt.validate_player_roster().is_err());
    }

    #[test]
    fn retained_history_byte_budget_includes_sequence_keys_and_delimiters() {
        const RECORD_COUNT: u64 = 32;
        const RECORD_BYTES: usize = 4_091;

        let candidate = (1..=RECORD_COUNT)
            .map(|sequence| {
                (
                    sequence,
                    synthetic_operation_record_with_size(sequence, RECORD_BYTES),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let value_bytes = candidate
            .values()
            .map(processed_operation_record_bytes)
            .sum::<usize>();
        assert!(value_bytes <= PROCESSED_OPERATION_RETAINED_BYTES_LIMIT);
        assert!(
            processed_operation_retained_bytes(&candidate)
                > PROCESSED_OPERATION_RETAINED_BYTES_LIMIT,
            "map keys and delimiters must count toward the canonical byte budget"
        );

        let mut world = WorldState::genesis(113);
        world.event_sequence = RECORD_COUNT;
        for record in candidate.into_values() {
            world
                .record_processed_operation("player-local", record)
                .expect("boundary records commit contiguously");
        }
        let history = &world.processed_operations["player-local"];
        assert!(history.compacted_through > 0);
        assert!(
            processed_operation_retained_bytes(&history.retained)
                <= PROCESSED_OPERATION_RETAINED_BYTES_LIMIT
        );
        assert!(world.validate_player_roster().is_ok());
    }

    #[test]
    fn oversized_operation_record_is_committed_only_into_the_rolling_hash() {
        let mut world = WorldState::genesis(101);
        world.event_sequence = 1;
        let operation_id = "oversized-record".to_owned();
        world
            .record_processed_operation(
                "player-local",
                ProcessedOperationRecord {
                    operation_id: operation_id.clone(),
                    intent_fingerprint: blake3::hash(b"oversized").to_hex().to_string(),
                    receipt: IntentReceipt {
                        operation_sequence: 1,
                        operation_id,
                        event_sequence: 1,
                        code: "oversized_committed".into(),
                        message: "x".repeat(PROCESSED_OPERATION_RECORD_BYTES_LIMIT + 1),
                    },
                },
            )
            .expect("a large durable receipt is immediately compacted");

        let history = &world.processed_operations["player-local"];
        assert_eq!(history.committed_through, 1);
        assert_eq!(history.compacted_through, 1);
        assert!(history.retained.is_empty());
        assert!(valid_blake3_hex(&history.compacted_history_hash));
        assert!(world.validate_player_roster().is_ok());
    }

    #[test]
    fn procedural_asteroid_is_deterministic() {
        let asteroid = VoxelField::procedural_asteroid(42, 8);
        assert_eq!(asteroid, VoxelField::procedural_asteroid(42, 8));
        assert_ne!(
            asteroid.ferrite_ore,
            VoxelField::procedural_asteroid(43, 8).ferrite_ore
        );
        assert!(
            asteroid
                .occupied
                .iter()
                .any(|coordinate| coordinate.squared_distance(IVec3::ZERO) > 64),
            "the visual-realism generator must produce material outside the base sphere"
        );
        assert!(
            asteroid.ferrite_ore.iter().any(|coordinate| {
                [
                    IVec3::new(1, 0, 0),
                    IVec3::new(0, 1, 0),
                    IVec3::new(0, 0, 1),
                ]
                .iter()
                .any(|offset| {
                    asteroid.ferrite_ore.contains(&IVec3::new(
                        coordinate.x + offset.x,
                        coordinate.y + offset.y,
                        coordinate.z + offset.z,
                    ))
                })
            }),
            "ferrite should form readable deposits rather than salt-and-pepper noise"
        );
    }

    #[test]
    fn genesis_exposes_orbital_separation_and_physical_inventory_metrics() {
        let world = WorldState::genesis(42);
        let environment = world.environment_at(world.player.position);
        assert!(!environment.breathable);
        assert!(environment.gravity_m_s2 > 0.3);
        assert!(environment.gravity_m_s2 < 1.0);
        assert!(environment.altitude_m > 3_000.0);
        assert!(environment.atmosphere_density <= f64::EPSILON);
        assert!(
            environment.altitude_m > planet_surface_radius_m() * 2.5,
            "the asteroid origin must be visibly and physically separated from the planet"
        );

        let near_surface = world.environment_at(Vec3::new(
            planet_center().x,
            planet_center().y + planet_surface_radius_m() + 10.0,
            planet_center().z,
        ));
        assert!(near_surface.breathable);
        assert!(near_surface.gravity_m_s2 > 5.5);
        assert!(near_surface.atmosphere_density > 0.9);

        let suit = &world.inventories[PLAYER_INVENTORY_ID];
        assert_eq!(suit.used_liters(), 24 * 22);
        assert_eq!(suit.mass_grams(), 24 * 4_800);
        assert!(suit.used_liters() < suit.capacity_liters);
        assert!(suit.can_add(ResourceKind::Ore, 1));
    }

    #[test]
    fn genesis_is_conserved_and_playable() {
        let world = WorldState::genesis(7);
        assert!(world.conservation().valid);
        assert!(world.validate_player_roster().is_ok());
        assert_eq!(world.grids[STARTER_GRID_ID].blocks.len(), 25);
        assert_eq!(world.grids[STARTER_GRID_ID].owner_player_id, "player-local");
        assert!(world.grids[STARTER_GRID_ID].anchor_reward_eligible);
        assert_eq!(
            world
                .inventory_owner_player_id("inventory-cargo-starter")
                .unwrap(),
            "player-local"
        );
        assert!(world.grids[STARTER_GRID_ID].power().online);
        assert!(world.voxels.occupied.len() > 1_000);
        assert!(world.voxels.occupied.contains(&IVec3::new(8, 0, 0)));
        assert!(world.player.validate().is_ok());
        assert_eq!(world.player.iter().count(), 1);
        let snapshot = world.snapshot();
        assert_eq!(snapshot.players.len(), 1);
        assert_eq!(snapshot.players[0], snapshot.player);
        assert_eq!(snapshot.grids[0].owner_player_id, "player-local");
        let persisted = serde_json::to_value(&world).expect("world serializes");
        assert!(persisted.get("players").is_some());
        assert!(persisted.get("player").is_none());
    }

    #[test]
    fn cargo_owner_is_derived_from_its_containing_grid() {
        let mut world = WorldState::genesis(7);
        world
            .grids
            .get_mut(STARTER_GRID_ID)
            .unwrap()
            .owner_player_id = "player-remote".into();

        assert_eq!(
            world
                .inventory_owner_player_id("inventory-cargo-starter")
                .unwrap(),
            "player-remote"
        );
        assert!(world.validate_authority_graph().is_ok());
    }

    #[test]
    fn authority_graph_rejects_duplicate_blocks_and_broken_cargo_links() {
        let mut duplicate = WorldState::genesis(7);
        let block = duplicate.grids[STARTER_GRID_ID].blocks["block-core"].clone();
        let mut second = duplicate.grids[STARTER_GRID_ID].clone();
        second.grid_id = "grid-duplicate".into();
        second.position = Vec3::new(100.0, 0.0, 0.0);
        second.address = duplicate
            .address_for_active_position(second.position)
            .expect("duplicate grid position canonicalizes");
        second.blocks = BTreeMap::from([(block.block_id.clone(), block)]);
        duplicate.grids.insert(second.grid_id.clone(), second);
        assert_eq!(
            duplicate.validate_authority_graph().unwrap_err(),
            "block IDs must be globally unique across all grids"
        );

        let mut broken = WorldState::genesis(7);
        broken
            .inventories
            .get_mut("inventory-cargo-starter")
            .unwrap()
            .domain = InventoryDomain::Cargo {
            block_id: "block-missing".into(),
        };
        assert_eq!(
            broken.validate_authority_graph().unwrap_err(),
            "cargo inventories require one bidirectional live cargo-block link"
        );
    }

    #[test]
    fn dropped_inventories_retain_a_valid_prior_owner() {
        let mut world = WorldState::genesis(7);
        world.inventories.insert(
            "inventory-destroyed-cargo".into(),
            InventoryRecord {
                inventory_id: "inventory-destroyed-cargo".into(),
                domain: InventoryDomain::Dropped {
                    reason: "cargo_block_destroyed".into(),
                    owner_player_id: "player-remote".into(),
                },
                contents: InventoryContents::default(),
                capacity_liters: CARGO_INVENTORY_CAPACITY_LITERS,
            },
        );
        assert_eq!(
            world
                .inventory_owner_player_id("inventory-destroyed-cargo")
                .unwrap(),
            "player-remote"
        );
        assert!(world.validate_authority_graph().is_ok());

        let InventoryDomain::Dropped {
            owner_player_id, ..
        } = &mut world
            .inventories
            .get_mut("inventory-destroyed-cargo")
            .unwrap()
            .domain
        else {
            unreachable!();
        };
        owner_player_id.clear();
        assert_eq!(
            world.validate_authority_graph().unwrap_err(),
            "dropped inventories must retain a reason and valid prior owner"
        );
    }
}
