// SPDX-License-Identifier: AGPL-3.0-or-later

use std::sync::OnceLock;

use serde::Deserialize;
use verse_protocol::{BlockKind, Vec3, VoxelMaterial};

const P0_CONTENT: &str = include_str!("../../../content/definitions/p0-content.json");

#[derive(Debug, Clone, Deserialize)]
pub struct ContentManifest {
    pub schema_version: u32,
    pub manifest_version: String,
    pub license: String,
    pub voxel_materials: Vec<VoxelDefinition>,
    pub blocks: Vec<BlockDefinition>,
    pub recipes: Recipes,
    pub physics: PhysicsDefinition,
    pub character: CharacterDefinition,
    pub survival: SurvivalDefinition,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoxelDefinition {
    pub material: VoxelMaterial,
    pub ore_yield: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockDefinition {
    pub kind: BlockKind,
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
    pub control_lease_ticks: u64,
    pub thrust_acceleration_m_s2: f64,
    pub boost_acceleration_m_s2: f64,
    pub linear_dampener_acceleration_m_s2: f64,
    pub angular_acceleration_radians_per_second_squared: f64,
    pub angular_dampener_acceleration_radians_per_second_squared: f64,
    pub maximum_speed_m_s: f64,
    pub boost_maximum_speed_m_s: f64,
    pub maximum_angular_speed_radians_per_second: f64,
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
pub struct Recipes {
    pub refining: RefiningRecipe,
    pub component_crafting: ComponentRecipe,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RefiningRecipe {
    pub ore_input: u64,
    pub refined_output: u64,
    pub defined_loss: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ComponentRecipe {
    pub refined_input: u64,
    pub component_output: u64,
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

pub fn manifest() -> &'static ContentManifest {
    static MANIFEST: OnceLock<ContentManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        let parsed: ContentManifest =
            serde_json::from_str(P0_CONTENT).expect("embedded P0 content must be valid JSON");
        assert_eq!(parsed.schema_version, 7, "unsupported P0 content schema");
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
        assert!(parsed.physics.control_force_newtons > 0.0);
        assert!(parsed.physics.control_torque_newton_meters > 0.0);
        assert!((0.0..=1.0).contains(&parsed.physics.friction));
        assert!((0.0..=1.0).contains(&parsed.physics.restitution));
        validate_character(&parsed.character).unwrap_or_else(|message| panic!("{message}"));
        assert_eq!(parsed.blocks.len(), 8, "every P0 block must be defined");
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
        assert_eq!(content.schema_version, 7);
        assert_eq!(content.manifest_version, "p0.9.0");
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
