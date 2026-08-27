// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::Deserialize;
use verse_protocol::{
    BlockKind, I64Vec3, InterestEntityKind, ProductionRecipeKind, Vec3, VoxelMaterial,
};

const P0_CONTENT: &str = include_str!("../../../content/definitions/p0-content.json");

#[derive(Debug, Clone, Deserialize)]
pub struct ContentManifest {
    pub schema_version: u32,
    pub manifest_version: String,
    pub license: String,
    pub voxel_materials: Vec<VoxelDefinition>,
    pub blocks: Vec<BlockDefinition>,
    pub recipes: Recipes,
    pub production: ProductionDefinition,
    pub physics: PhysicsDefinition,
    pub character: CharacterDefinition,
    pub survival: SurvivalDefinition,
    pub celestial: CelestialDefinition,
    pub interest: InterestDefinition,
    pub experience_rewards: ExperienceRewards,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoxelDefinition {
    pub material: VoxelMaterial,
    pub ore_yield: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockDefinition {
    pub kind: BlockKind,
    pub conveyor_ports: u8,
    pub max_health: u16,
    pub power_production: f64,
    pub power_requirement: f64,
    pub stored_power: f64,
    pub component_cost: u64,
    pub mass_grams: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhysicsDefinition {
    pub fixed_step_hz: u16,
    pub fixed_delta_seconds: f32,
    pub collision_substeps: i32,
    pub voxel_collision_chunk_edge_cells: u16,
    pub control_force_newtons: f64,
    pub control_torque_newton_meters: f64,
    pub linear_dampener_newtons_per_mps: f64,
    pub angular_dampener_newton_meters_per_radian: f64,
    pub friction: f32,
    pub restitution: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CharacterDefinition {
    pub mass_kg: f64,
    pub collision_radius_m: f64,
    pub standing_height_m: f64,
    pub eye_height_m: f64,
    pub control_lease_ticks: u64,
    pub thrust_acceleration_m_s2: f64,
    pub boost_acceleration_m_s2: f64,
    pub linear_dampener_acceleration_m_s2: f64,
    pub angular_acceleration_radians_per_second_squared: f64,
    pub angular_dampener_acceleration_radians_per_second_squared: f64,
    pub maximum_speed_m_s: f64,
    pub boost_maximum_speed_m_s: f64,
    pub maximum_angular_speed_radians_per_second: f64,
    pub maximum_view_pitch_degrees: f64,
    pub upright_alignment_acceleration_radians_per_second_squared: f64,
    pub walk_speed_m_s: f64,
    pub sprint_speed_m_s: f64,
    pub ground_acceleration_m_s2: f64,
    pub ground_braking_m_s2: f64,
    pub jump_speed_m_s: f64,
    pub walkable_slope_degrees: f64,
    pub slope_exit_hysteresis_degrees: f64,
    pub step_height_m: f64,
    pub ground_snap_m: f64,
    pub support_probe_distance_m: f64,
    pub jump_buffer_ticks: u64,
    pub coyote_ticks: u64,
    pub magnetic_probe_distance_m: f64,
    pub magnetic_catch_speed_m_s: f64,
    pub magnetic_adhesion_acceleration_m_s2: f64,
    pub magnetic_reattach_lockout_ticks: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SurvivalDefinition {
    pub suit_oxygen_capacity_milli: u16,
    pub critical_oxygen_milli: u16,
    pub open_breathable_delta_milli_per_second: i16,
    pub open_vacuum_delta_milli_per_second: i16,
    pub sealed_breathable_delta_milli_per_second: i16,
    pub sealed_vacuum_delta_milli_per_second: i16,
    pub respawn_oxygen_milli: u16,
    pub respawn_helmet_closed: bool,
    pub respawn_jetpack_enabled: bool,
    pub proof_recovery_position: Vec3,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterestDefinition {
    pub spatial_bucket_edge_m: u32,
    pub enter_radius_m: u32,
    pub exit_radius_m: u32,
    pub exit_consecutive_ticks: u16,
    pub maximum_visible_entities: usize,
    pub selected_context_margin_m: u32,
    pub maximum_selected_context_entities: usize,
    pub public_spectator_anchor_um: I64Vec3,
    pub entity_bands: Vec<InterestBandDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InterestBandDefinition {
    pub kind: InterestEntityKind,
    pub enter_radius_m: u32,
    pub exit_radius_m: u32,
    pub update_interval_ticks: u16,
    pub maximum_entities: usize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CelestialDefinition {
    pub minimum_fixed_body_surface_gap_um: u64,
    pub geometry_definition_ids: Vec<String>,
    pub voxel_definition_ids: Vec<String>,
    pub material_definition_ids: Vec<String>,
    pub gravity_definition_ids: Vec<String>,
    pub atmosphere_definition_ids: Vec<String>,
    pub resource_definition_ids: Vec<String>,
}

impl CelestialDefinition {
    pub fn contains_geometry(&self, value: &str) -> bool {
        self.geometry_definition_ids
            .iter()
            .any(|item| item == value)
    }

    pub fn contains_voxel(&self, value: &str) -> bool {
        self.voxel_definition_ids.iter().any(|item| item == value)
    }

    pub fn contains_material(&self, value: &str) -> bool {
        self.material_definition_ids
            .iter()
            .any(|item| item == value)
    }

    pub fn contains_gravity(&self, value: &str) -> bool {
        self.gravity_definition_ids.iter().any(|item| item == value)
    }

    pub fn contains_atmosphere(&self, value: &str) -> bool {
        self.atmosphere_definition_ids
            .iter()
            .any(|item| item == value)
    }

    pub fn contains_resource(&self, value: &str) -> bool {
        self.resource_definition_ids
            .iter()
            .any(|item| item == value)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExperienceRewards {
    pub mined_ore_unit: u64,
    pub refining_batch: u64,
    pub crafted_component: u64,
    pub frame_placed: u64,
    pub construction_completed: u64,
    pub weld_progress_or_repair: u64,
    pub inventory_transfer: u64,
    pub first_anchor_engagement: u64,
    pub block_damage: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Recipes {
    pub refining: RefiningRecipe,
    pub component_crafting: ComponentRecipe,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefiningRecipe {
    pub ore_input: u64,
    pub refined_output: u64,
    pub defined_loss: u64,
    pub duration_ticks_per_batch: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentRecipe {
    pub refined_input: u64,
    pub component_output: u64,
    pub duration_ticks_per_batch: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProductionDefinition {
    pub scheduler_interval_millis: u64,
    pub queue_limit_per_machine: usize,
}

const ALL_CONVEYOR_PORTS: u8 = 0b11_1111;
const P1_4_BLOCK_KINDS: [BlockKind; 11] = [
    BlockKind::Structural,
    BlockKind::ControlCore,
    BlockKind::PowerSource,
    BlockKind::Battery,
    BlockKind::Cargo,
    BlockKind::Drill,
    BlockKind::Anchor,
    BlockKind::DamageTest,
    BlockKind::Conveyor,
    BlockKind::Refinery,
    BlockKind::Assembler,
];

pub const fn production_machine_kind(recipe: ProductionRecipeKind) -> BlockKind {
    match recipe {
        ProductionRecipeKind::Refining => BlockKind::Refinery,
        ProductionRecipeKind::Component => BlockKind::Assembler,
    }
}

pub fn machine_supports_recipe(machine_kind: BlockKind, recipe: ProductionRecipeKind) -> bool {
    machine_kind == production_machine_kind(recipe)
}

fn validate_production(content: &ContentManifest) -> Result<(), &'static str> {
    if content.production.scheduler_interval_millis != 1_000 {
        return Err("P1.4 production scheduler interval must be exactly one second");
    }
    if content.production.queue_limit_per_machine != 32 {
        return Err("P1.4 production queues must contain at most 32 jobs");
    }
    if content.recipes.refining.duration_ticks_per_batch == 0
        || content.recipes.component_crafting.duration_ticks_per_batch == 0
    {
        return Err("production recipe durations must be positive");
    }

    if P1_4_BLOCK_KINDS.iter().any(|kind| {
        content
            .blocks
            .iter()
            .filter(|definition| definition.kind == *kind)
            .count()
            != 1
    }) {
        return Err("every P1.4 block kind must have exactly one definition");
    }

    for definition in &content.blocks {
        if definition.conveyor_ports & !ALL_CONVEYOR_PORTS != 0 {
            return Err("block conveyor ports may use only the six canonical face bits");
        }
        let expected_ports = match definition.kind {
            BlockKind::Cargo | BlockKind::Conveyor | BlockKind::Refinery | BlockKind::Assembler => {
                ALL_CONVEYOR_PORTS
            }
            BlockKind::Structural
            | BlockKind::ControlCore
            | BlockKind::PowerSource
            | BlockKind::Battery
            | BlockKind::Drill
            | BlockKind::Anchor
            | BlockKind::DamageTest => 0,
        };
        if definition.conveyor_ports != expected_ports {
            return Err("block conveyor ports do not match the P1.4 topology contract");
        }
    }

    if !machine_supports_recipe(BlockKind::Refinery, ProductionRecipeKind::Refining)
        || !machine_supports_recipe(BlockKind::Assembler, ProductionRecipeKind::Component)
        || machine_supports_recipe(BlockKind::Refinery, ProductionRecipeKind::Component)
        || machine_supports_recipe(BlockKind::Assembler, ProductionRecipeKind::Refining)
    {
        return Err("production recipes must resolve to exactly one matching machine kind");
    }

    Ok(())
}

fn validate_voxel_collision_chunk_edge_cells(edge_cells: u16) -> Result<(), &'static str> {
    if edge_cells == 8 {
        Ok(())
    } else {
        Err("P0.7 collision chunks must be 8×8×8 cells")
    }
}

fn validate_character(definition: &CharacterDefinition) -> Result<(), &'static str> {
    if !definition.mass_kg.is_finite() || definition.mass_kg <= 0.0 {
        return Err("character mass must be finite and positive");
    }
    if !definition.collision_radius_m.is_finite() || definition.collision_radius_m <= 0.0 {
        return Err("character collision radius must be finite and positive");
    }
    if !definition.standing_height_m.is_finite()
        || definition.standing_height_m <= 2.0 * definition.collision_radius_m
        || !definition.eye_height_m.is_finite()
        || definition.eye_height_m <= definition.collision_radius_m
        || definition.eye_height_m >= definition.standing_height_m
    {
        return Err("character standing and eye heights must fit the capsule");
    }
    if definition.control_lease_ticks == 0 {
        return Err("character control lease must be positive");
    }
    if !definition.thrust_acceleration_m_s2.is_finite()
        || definition.thrust_acceleration_m_s2 <= 0.0
    {
        return Err("character thrust acceleration must be finite and positive");
    }
    if !definition.boost_acceleration_m_s2.is_finite()
        || definition.boost_acceleration_m_s2 <= definition.thrust_acceleration_m_s2
    {
        return Err("character boost acceleration must be finite and exceed thrust acceleration");
    }
    if !definition.linear_dampener_acceleration_m_s2.is_finite()
        || definition.linear_dampener_acceleration_m_s2 <= 0.0
    {
        return Err("character linear dampener acceleration must be finite and positive");
    }
    if !definition
        .angular_acceleration_radians_per_second_squared
        .is_finite()
        || definition.angular_acceleration_radians_per_second_squared <= 0.0
    {
        return Err("character angular acceleration must be finite and positive");
    }
    if !definition
        .angular_dampener_acceleration_radians_per_second_squared
        .is_finite()
        || definition.angular_dampener_acceleration_radians_per_second_squared <= 0.0
    {
        return Err("character angular dampener acceleration must be finite and positive");
    }
    if !definition.maximum_speed_m_s.is_finite() || definition.maximum_speed_m_s <= 0.0 {
        return Err("character maximum speed must be finite and positive");
    }
    if !definition.boost_maximum_speed_m_s.is_finite()
        || definition.boost_maximum_speed_m_s <= definition.maximum_speed_m_s
    {
        return Err("character boost maximum speed must be finite and exceed maximum speed");
    }
    if !definition
        .maximum_angular_speed_radians_per_second
        .is_finite()
        || definition.maximum_angular_speed_radians_per_second <= 0.0
    {
        return Err("character maximum angular speed must be finite and positive");
    }
    if !definition.maximum_view_pitch_degrees.is_finite()
        || !(1.0..89.0).contains(&definition.maximum_view_pitch_degrees)
    {
        return Err("character view pitch limit must be finite and between 1 and 89 degrees");
    }
    if !definition
        .upright_alignment_acceleration_radians_per_second_squared
        .is_finite()
        || definition.upright_alignment_acceleration_radians_per_second_squared <= 0.0
    {
        return Err("character upright alignment acceleration must be finite and positive");
    }
    if !definition.walk_speed_m_s.is_finite()
        || definition.walk_speed_m_s <= 0.0
        || !definition.sprint_speed_m_s.is_finite()
        || definition.sprint_speed_m_s <= definition.walk_speed_m_s
    {
        return Err("character walk and sprint speeds must be finite, positive, and ordered");
    }
    for value in [
        definition.ground_acceleration_m_s2,
        definition.ground_braking_m_s2,
        definition.jump_speed_m_s,
        definition.magnetic_catch_speed_m_s,
        definition.magnetic_adhesion_acceleration_m_s2,
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(
                "character locomotion accelerations and speeds must be finite and positive",
            );
        }
    }
    if !definition.walkable_slope_degrees.is_finite()
        || !(1.0..89.0).contains(&definition.walkable_slope_degrees)
        || !definition.slope_exit_hysteresis_degrees.is_finite()
        || definition.slope_exit_hysteresis_degrees <= 0.0
        || definition.slope_exit_hysteresis_degrees >= definition.walkable_slope_degrees
    {
        return Err("character slope and hysteresis angles must be finite and ordered");
    }
    if !definition.step_height_m.is_finite()
        || definition.step_height_m <= 0.0
        || definition.step_height_m >= definition.standing_height_m
        || !definition.ground_snap_m.is_finite()
        || definition.ground_snap_m <= 0.0
        || definition.ground_snap_m > definition.step_height_m
        || !definition.support_probe_distance_m.is_finite()
        || definition.support_probe_distance_m < definition.ground_snap_m
        || definition.support_probe_distance_m > definition.step_height_m
        || !definition.magnetic_probe_distance_m.is_finite()
        || definition.magnetic_probe_distance_m <= 0.0
        || definition.magnetic_probe_distance_m > definition.step_height_m
    {
        return Err("character step, snap, and support probes must be finite and ordered");
    }
    if definition.jump_buffer_ticks == 0
        || definition.jump_buffer_ticks > definition.control_lease_ticks
        || definition.coyote_ticks == 0
        || definition.coyote_ticks > definition.control_lease_ticks
        || definition.magnetic_reattach_lockout_ticks == 0
    {
        return Err("character locomotion tick windows must be positive and bounded");
    }
    Ok(())
}

fn validate_survival(definition: &SurvivalDefinition) -> Result<(), &'static str> {
    let capacity = definition.suit_oxygen_capacity_milli;
    if capacity == 0 {
        return Err("suit oxygen capacity must be positive");
    }
    if definition.critical_oxygen_milli == 0 || definition.critical_oxygen_milli >= capacity {
        return Err("critical oxygen must be positive and below suit capacity");
    }
    if definition.respawn_oxygen_milli <= definition.critical_oxygen_milli
        || definition.respawn_oxygen_milli > capacity
    {
        return Err("respawn oxygen must be above critical and at most suit capacity");
    }
    if !definition.proof_recovery_position.x.is_finite()
        || !definition.proof_recovery_position.y.is_finite()
        || !definition.proof_recovery_position.z.is_finite()
    {
        return Err("proof recovery position must be finite");
    }

    let capacity = i32::from(capacity);
    let open_breathable = i32::from(definition.open_breathable_delta_milli_per_second);
    let open_vacuum = i32::from(definition.open_vacuum_delta_milli_per_second);
    let sealed_breathable = i32::from(definition.sealed_breathable_delta_milli_per_second);
    let sealed_vacuum = i32::from(definition.sealed_vacuum_delta_milli_per_second);
    if !(1..=capacity).contains(&open_breathable) {
        return Err("open-helmet breathable oxygen rate must be positive and bounded by capacity");
    }
    if !(-capacity..=-1).contains(&open_vacuum) {
        return Err("open-helmet vacuum oxygen rate must be negative and bounded by capacity");
    }
    if sealed_breathable != 0 {
        return Err("sealed-helmet breathable oxygen rate must be zero");
    }
    if !(-capacity..=-1).contains(&sealed_vacuum) {
        return Err("sealed-helmet vacuum oxygen rate must be negative and bounded by capacity");
    }
    if sealed_vacuum <= open_vacuum {
        return Err("sealed-helmet vacuum oxygen loss must be slower than open-helmet loss");
    }
    Ok(())
}

fn validate_interest(definition: &InterestDefinition) -> Result<(), &'static str> {
    if definition.spatial_bucket_edge_m != 256 {
        return Err("P1.5 spatial buckets must have a 256 metre edge");
    }
    if definition.enter_radius_m != 2_000 || definition.exit_radius_m != 2_250 {
        return Err("P1.5 interest radii must be the versioned 2,000/2,250 metre proof policy");
    }
    if definition.exit_consecutive_ticks != 2 {
        return Err("P1.5 interest exit hysteresis must require two committed evaluations");
    }
    if definition.maximum_visible_entities != 4_096 {
        return Err("P1.5 interest views must contain at most 4,096 entities");
    }
    if definition.selected_context_margin_m != 250
        || definition.maximum_selected_context_entities != 64
        || definition.public_spectator_anchor_um != I64Vec3::ZERO
    {
        return Err("P1.5 selected-context and spectator-anchor policy must remain pinned");
    }
    let expected_kinds = [
        InterestEntityKind::Player,
        InterestEntityKind::Grid,
        InterestEntityKind::VoxelChunk,
        InterestEntityKind::DeathDrop,
    ];
    if definition.entity_bands.len() != expected_kinds.len()
        || !definition
            .entity_bands
            .iter()
            .map(|band| band.kind)
            .eq(expected_kinds)
        || definition
            .entity_bands
            .iter()
            .map(|band| band.kind)
            .collect::<BTreeSet<_>>()
            != expected_kinds.into_iter().collect::<BTreeSet<_>>()
        || definition.entity_bands.iter().any(|band| {
            band.enter_radius_m == 0
                || band.exit_radius_m <= band.enter_radius_m
                || band.update_interval_ticks == 0
                || band.maximum_entities == 0
        })
        || definition
            .entity_bands
            .iter()
            .map(|band| band.maximum_entities)
            .sum::<usize>()
            > definition.maximum_visible_entities
    {
        return Err("P1.5 interest entity bands must be unique, bounded, and ordered");
    }
    Ok(())
}

fn validate_celestial(definition: &CelestialDefinition) -> Result<(), &'static str> {
    if definition.minimum_fixed_body_surface_gap_um != 3_000_000_000 {
        return Err("P1.5 fixed celestial bodies must keep the pinned 3,000 metre surface gap");
    }
    for values in [
        &definition.geometry_definition_ids,
        &definition.voxel_definition_ids,
        &definition.material_definition_ids,
        &definition.gravity_definition_ids,
        &definition.atmosphere_definition_ids,
        &definition.resource_definition_ids,
    ] {
        if values.is_empty()
            || values.iter().any(|value| value.trim().is_empty())
            || values.iter().collect::<BTreeSet<_>>().len() != values.len()
            || !values.windows(2).all(|pair| pair[0] < pair[1])
        {
            return Err("celestial definition IDs must be nonempty, unique, and sorted");
        }
    }
    Ok(())
}

pub fn manifest() -> &'static ContentManifest {
    static MANIFEST: OnceLock<ContentManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        let parsed: ContentManifest =
            serde_json::from_str(P0_CONTENT).expect("embedded P0 content must be valid JSON");
        assert_eq!(parsed.schema_version, 11, "unsupported P1.5 content schema");
        assert_eq!(parsed.manifest_version, "p1.5.0");
        assert_eq!(
            parsed.license, "AGPL-3.0-or-later",
            "content definition license must be explicit"
        );
        assert!(parsed.physics.fixed_delta_seconds > 0.0);
        assert_eq!(parsed.physics.fixed_step_hz, 60);
        assert!((1..=16).contains(&parsed.physics.collision_substeps));
        validate_voxel_collision_chunk_edge_cells(parsed.physics.voxel_collision_chunk_edge_cells)
            .unwrap_or_else(|message| panic!("{message}"));
        validate_survival(&parsed.survival).unwrap_or_else(|message| panic!("{message}"));
        validate_celestial(&parsed.celestial).unwrap_or_else(|message| panic!("{message}"));
        validate_interest(&parsed.interest).unwrap_or_else(|message| panic!("{message}"));
        assert!(parsed.physics.control_force_newtons > 0.0);
        assert!(parsed.physics.control_torque_newton_meters > 0.0);
        assert!((0.0..=1.0).contains(&parsed.physics.friction));
        assert!((0.0..=1.0).contains(&parsed.physics.restitution));
        validate_character(&parsed.character).unwrap_or_else(|message| panic!("{message}"));
        validate_production(&parsed).unwrap_or_else(|message| panic!("{message}"));
        assert_eq!(parsed.blocks.len(), 11, "every P1.4 block must be defined");
        assert_eq!(
            parsed.voxel_materials.len(),
            2,
            "every P0 voxel material must be defined"
        );
        assert_eq!(
            parsed.recipes.refining.ore_input,
            parsed.recipes.refining.refined_output + parsed.recipes.refining.defined_loss,
            "refining recipe must conserve its registered material units"
        );
        assert_eq!(
            parsed.recipes.component_crafting.component_output, 1,
            "P0 crafting requests are expressed as individual component quantities"
        );
        parsed
    })
}

pub fn manifest_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| {
        let canonical: serde_json::Value =
            serde_json::from_str(P0_CONTENT).expect("embedded content must be valid JSON");
        let bytes = serde_json::to_vec(&canonical).expect("content canonical JSON serializes");
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"the-verse/content-manifest/v11\0");
        hasher.update(&bytes);
        hasher.finalize().to_hex().to_string()
    })
}

pub fn block(kind: BlockKind) -> &'static BlockDefinition {
    manifest()
        .blocks
        .iter()
        .find(|definition| definition.kind == kind)
        .expect("all BlockKind values must exist in the embedded content manifest")
}

pub fn voxel(material: VoxelMaterial) -> &'static VoxelDefinition {
    manifest()
        .voxel_materials
        .iter()
        .find(|definition| definition.material == material)
        .expect("all VoxelMaterial values must exist in the embedded content manifest")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn manifest_definitions_are_unique_and_conserved() {
        let content = manifest();
        let block_kinds = content
            .blocks
            .iter()
            .map(|definition| format!("{:?}", definition.kind))
            .collect::<BTreeSet<_>>();
        assert_eq!(block_kinds.len(), content.blocks.len());
        assert_eq!(content.schema_version, 11);
        assert_eq!(content.manifest_version, "p1.5.0");
        assert_eq!(content.physics.voxel_collision_chunk_edge_cells, 8);
        assert_eq!(content.survival.suit_oxygen_capacity_milli, 1_000);
        assert_eq!(content.survival.critical_oxygen_milli, 200);
        assert_eq!(content.survival.open_breathable_delta_milli_per_second, 25);
        assert_eq!(content.survival.open_vacuum_delta_milli_per_second, -40);
        assert_eq!(content.survival.sealed_breathable_delta_milli_per_second, 0);
        assert_eq!(content.survival.sealed_vacuum_delta_milli_per_second, -5);
        assert_eq!(content.survival.respawn_oxygen_milli, 1_000);
        assert!(content.survival.respawn_helmet_closed);
        assert!(content.survival.respawn_jetpack_enabled);
        assert_eq!(content.production.scheduler_interval_millis, 1_000);
        assert_eq!(content.production.queue_limit_per_machine, 32);
        assert_eq!(content.interest.spatial_bucket_edge_m, 256);
        assert_eq!(content.interest.enter_radius_m, 2_000);
        assert_eq!(content.interest.exit_radius_m, 2_250);
        assert_eq!(content.interest.exit_consecutive_ticks, 2);
        assert_eq!(content.interest.maximum_visible_entities, 4_096);
        assert_eq!(content.interest.public_spectator_anchor_um, I64Vec3::ZERO);
        assert_eq!(content.interest.entity_bands.len(), 4);
        assert_eq!(
            content.celestial.minimum_fixed_body_surface_gap_um,
            3_000_000_000
        );
        assert_eq!(manifest_hash().len(), 64);
        assert_eq!(content.recipes.refining.duration_ticks_per_batch, 120);
        assert_eq!(
            content.recipes.component_crafting.duration_ticks_per_batch,
            90
        );
        assert_eq!(
            content.survival.proof_recovery_position,
            Vec3::new(12.0, 4.5, 10.0)
        );
        assert!(content.blocks.iter().all(|definition| {
            definition.max_health > 0 && definition.component_cost > 0 && definition.mass_grams > 0
        }));
        assert_eq!(
            content.recipes.component_crafting.refined_input,
            content.recipes.component_crafting.component_output
        );
        assert_eq!(content.experience_rewards.mined_ore_unit, 5);
        assert_eq!(content.experience_rewards.refining_batch, 12);
        assert_eq!(content.experience_rewards.crafted_component, 18);
        assert_eq!(content.experience_rewards.frame_placed, 5);
        assert_eq!(content.experience_rewards.construction_completed, 20);
        assert_eq!(content.experience_rewards.weld_progress_or_repair, 0);
        assert_eq!(content.experience_rewards.inventory_transfer, 0);
        assert_eq!(content.experience_rewards.first_anchor_engagement, 40);
        assert_eq!(content.experience_rewards.block_damage, 0);
    }

    #[test]
    fn production_machine_pairing_and_conveyor_ports_are_pinned() {
        assert!(machine_supports_recipe(
            BlockKind::Refinery,
            ProductionRecipeKind::Refining
        ));
        assert!(machine_supports_recipe(
            BlockKind::Assembler,
            ProductionRecipeKind::Component
        ));
        for invalid in [
            (BlockKind::Refinery, ProductionRecipeKind::Component),
            (BlockKind::Assembler, ProductionRecipeKind::Refining),
            (BlockKind::Conveyor, ProductionRecipeKind::Refining),
            (BlockKind::Cargo, ProductionRecipeKind::Component),
        ] {
            assert!(!machine_supports_recipe(invalid.0, invalid.1));
        }

        for kind in [
            BlockKind::Cargo,
            BlockKind::Conveyor,
            BlockKind::Refinery,
            BlockKind::Assembler,
        ] {
            assert_eq!(block(kind).conveyor_ports, ALL_CONVEYOR_PORTS);
        }
        for kind in [
            BlockKind::Structural,
            BlockKind::ControlCore,
            BlockKind::PowerSource,
            BlockKind::Battery,
            BlockKind::Drill,
            BlockKind::Anchor,
            BlockKind::DamageTest,
        ] {
            assert_eq!(block(kind).conveyor_ports, 0);
        }
    }

    #[test]
    fn production_scheduler_queue_duration_and_port_bounds_are_validated() {
        let mut content = manifest().clone();
        content.production.scheduler_interval_millis = 999;
        assert!(validate_production(&content).is_err());

        let mut content = manifest().clone();
        content.production.queue_limit_per_machine = 31;
        assert!(validate_production(&content).is_err());

        let mut content = manifest().clone();
        content.recipes.refining.duration_ticks_per_batch = 0;
        assert!(validate_production(&content).is_err());

        let mut content = manifest().clone();
        content.recipes.component_crafting.duration_ticks_per_batch = 0;
        assert!(validate_production(&content).is_err());

        let mut content = manifest().clone();
        content
            .blocks
            .iter_mut()
            .find(|definition| definition.kind == BlockKind::Conveyor)
            .expect("conveyor definition exists")
            .conveyor_ports = 0b100_0000;
        assert!(validate_production(&content).is_err());

        let mut content = manifest().clone();
        content
            .blocks
            .iter_mut()
            .find(|definition| definition.kind == BlockKind::Structural)
            .expect("structural definition exists")
            .conveyor_ports = 1;
        assert!(validate_production(&content).is_err());

        let mut content = manifest().clone();
        let duplicate = content
            .blocks
            .iter()
            .find(|definition| definition.kind == BlockKind::Conveyor)
            .expect("conveyor definition exists")
            .clone();
        content.blocks.push(duplicate);
        assert!(validate_production(&content).is_err());

        let mut content = manifest().clone();
        content
            .blocks
            .retain(|definition| definition.kind != BlockKind::Assembler);
        assert!(validate_production(&content).is_err());
    }

    #[test]
    fn celestial_gap_and_definition_allowlists_are_pinned() {
        let celestial = &manifest().celestial;
        assert!(celestial.contains_geometry("procedural-voxel-v1"));
        assert!(celestial.contains_voxel("origin-voxel-field-v1"));
        assert!(celestial.contains_material("terrestrial-regolith-v1"));
        assert!(celestial.contains_gravity("radial-inverse-square-v1"));
        assert!(celestial.contains_atmosphere("oxygen-gradient-v1"));
        assert!(celestial.contains_resource("ferrite-deposit-v1"));

        let mut wrong_gap = celestial.clone();
        wrong_gap.minimum_fixed_body_surface_gap_um -= 1;
        assert!(validate_celestial(&wrong_gap).is_err());

        let mut duplicate = celestial.clone();
        duplicate
            .geometry_definition_ids
            .push("sphere-heightfield-v1".into());
        assert!(validate_celestial(&duplicate).is_err());

        let mut unsorted = celestial.clone();
        unsorted.material_definition_ids.reverse();
        assert!(validate_celestial(&unsorted).is_err());

        let mut empty = celestial.clone();
        empty.voxel_definition_ids.clear();
        assert!(validate_celestial(&empty).is_err());
    }

    #[test]
    fn interest_bands_anchor_and_budgets_are_pinned() {
        let interest = &manifest().interest;
        assert!(validate_interest(interest).is_ok());

        let mut reordered = interest.clone();
        reordered.entity_bands.swap(0, 1);
        assert!(validate_interest(&reordered).is_err());

        let mut duplicate = interest.clone();
        duplicate.entity_bands[1].kind = InterestEntityKind::Player;
        assert!(validate_interest(&duplicate).is_err());

        let mut invalid_exit = interest.clone();
        invalid_exit.entity_bands[0].exit_radius_m = invalid_exit.entity_bands[0].enter_radius_m;
        assert!(validate_interest(&invalid_exit).is_err());

        let mut excessive = interest.clone();
        excessive.entity_bands[0].maximum_entities = excessive.maximum_visible_entities;
        assert!(validate_interest(&excessive).is_err());

        let mut spoofable_anchor = interest.clone();
        spoofable_anchor.public_spectator_anchor_um.x = 1;
        assert!(validate_interest(&spoofable_anchor).is_err());
    }

    #[test]
    fn collision_chunk_size_is_required_and_pinned() {
        assert!(validate_voxel_collision_chunk_edge_cells(0).is_err());
        assert!(validate_voxel_collision_chunk_edge_cells(7).is_err());
        assert!(validate_voxel_collision_chunk_edge_cells(9).is_err());

        let mut json: serde_json::Value =
            serde_json::from_str(P0_CONTENT).expect("embedded content parses as JSON");
        json.get_mut("physics")
            .and_then(serde_json::Value::as_object_mut)
            .expect("physics is an object")
            .remove("voxel_collision_chunk_edge_cells");
        assert!(serde_json::from_value::<ContentManifest>(json).is_err());
    }

    fn assert_character_rejected(
        update: impl FnOnce(&mut CharacterDefinition),
        expected: &'static str,
    ) {
        let mut character = manifest().character.clone();
        update(&mut character);
        assert_eq!(validate_character(&character), Err(expected));
    }

    #[test]
    fn character_mass_radius_and_control_lease_are_validated() {
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_character_rejected(
                |character| character.mass_kg = invalid,
                "character mass must be finite and positive",
            );
            assert_character_rejected(
                |character| character.collision_radius_m = invalid,
                "character collision radius must be finite and positive",
            );
        }
        assert_character_rejected(
            |character| character.control_lease_ticks = 0,
            "character control lease must be positive",
        );
    }

    #[test]
    fn character_linear_acceleration_and_speed_bounds_are_validated() {
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_character_rejected(
                |character| character.thrust_acceleration_m_s2 = invalid,
                "character thrust acceleration must be finite and positive",
            );
            assert_character_rejected(
                |character| character.linear_dampener_acceleration_m_s2 = invalid,
                "character linear dampener acceleration must be finite and positive",
            );
            assert_character_rejected(
                |character| character.maximum_speed_m_s = invalid,
                "character maximum speed must be finite and positive",
            );
        }

        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_character_rejected(
                |character| character.boost_acceleration_m_s2 = invalid,
                "character boost acceleration must be finite and exceed thrust acceleration",
            );
            assert_character_rejected(
                |character| character.boost_maximum_speed_m_s = invalid,
                "character boost maximum speed must be finite and exceed maximum speed",
            );
        }
        assert_character_rejected(
            |character| {
                character.boost_acceleration_m_s2 = character.thrust_acceleration_m_s2;
            },
            "character boost acceleration must be finite and exceed thrust acceleration",
        );
        assert_character_rejected(
            |character| {
                character.boost_maximum_speed_m_s = character.maximum_speed_m_s;
            },
            "character boost maximum speed must be finite and exceed maximum speed",
        );
    }

    #[test]
    fn character_angular_acceleration_and_speed_bounds_are_validated() {
        for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_character_rejected(
                |character| {
                    character.angular_acceleration_radians_per_second_squared = invalid;
                },
                "character angular acceleration must be finite and positive",
            );
            assert_character_rejected(
                |character| {
                    character.angular_dampener_acceleration_radians_per_second_squared = invalid;
                },
                "character angular dampener acceleration must be finite and positive",
            );
            assert_character_rejected(
                |character| {
                    character.upright_alignment_acceleration_radians_per_second_squared = invalid;
                },
                "character upright alignment acceleration must be finite and positive",
            );
            assert_character_rejected(
                |character| character.maximum_angular_speed_radians_per_second = invalid,
                "character maximum angular speed must be finite and positive",
            );
        }
    }

    #[test]
    fn survival_threshold_and_capacity_are_validated() {
        let mut survival = manifest().survival.clone();
        survival.suit_oxygen_capacity_milli = 0;
        assert!(validate_survival(&survival).is_err());

        let mut survival = manifest().survival.clone();
        survival.critical_oxygen_milli = 0;
        assert!(validate_survival(&survival).is_err());

        let mut survival = manifest().survival.clone();
        survival.critical_oxygen_milli = survival.suit_oxygen_capacity_milli;
        assert!(validate_survival(&survival).is_err());
    }

    #[test]
    fn survival_recovery_position_must_be_finite() {
        for position in [
            Vec3::new(f64::NAN, 4.5, 10.0),
            Vec3::new(12.0, f64::INFINITY, 10.0),
            Vec3::new(12.0, 4.5, f64::NEG_INFINITY),
        ] {
            let mut survival = manifest().survival.clone();
            survival.proof_recovery_position = position;
            assert!(validate_survival(&survival).is_err());
        }
    }

    #[test]
    fn survival_respawn_bounds_and_oxygen_rates_are_validated() {
        for respawn_oxygen_milli in [
            0,
            manifest().survival.critical_oxygen_milli,
            manifest().survival.suit_oxygen_capacity_milli + 1,
        ] {
            let mut survival = manifest().survival.clone();
            survival.respawn_oxygen_milli = respawn_oxygen_milli;
            assert!(validate_survival(&survival).is_err());
        }

        let mut invalid_rates = Vec::new();
        let mut survival = manifest().survival.clone();
        survival.open_breathable_delta_milli_per_second = 0;
        invalid_rates.push(survival);
        let mut survival = manifest().survival.clone();
        survival.open_vacuum_delta_milli_per_second = 0;
        invalid_rates.push(survival);
        let mut survival = manifest().survival.clone();
        survival.sealed_breathable_delta_milli_per_second = 1;
        invalid_rates.push(survival);
        let mut survival = manifest().survival.clone();
        survival.sealed_vacuum_delta_milli_per_second = 0;
        invalid_rates.push(survival);
        let mut survival = manifest().survival.clone();
        survival.sealed_vacuum_delta_milli_per_second = -41;
        invalid_rates.push(survival);
        let mut survival = manifest().survival.clone();
        survival.open_breathable_delta_milli_per_second = 1_001;
        invalid_rates.push(survival);
        let mut survival = manifest().survival.clone();
        survival.open_vacuum_delta_milli_per_second = -1_001;
        invalid_rates.push(survival);

        assert!(
            invalid_rates
                .iter()
                .all(|survival| validate_survival(survival).is_err())
        );
    }
}
