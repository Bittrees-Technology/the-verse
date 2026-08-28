// SPDX-License-Identifier: AGPL-3.0-or-later

//! Immutable P1.5 celestial identity and exact hierarchical addressing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use verse_protocol::{
    CELESTIAL_REGISTRY_SCHEMA_VERSION, CELL_DIRECTORY_SCHEMA_VERSION, CELL_KEY_SCHEMA_VERSION,
    CelestialBodyKind, CelestialBodySnapshot, CelestialRegistrySnapshot, CelestialScaleClass,
    CellCoordinate, CellKeyV1, I64Vec3, INTENT_FINGERPRINT_SCHEMA_VERSION, INTEREST_SCHEMA_VERSION,
    LIFECYCLE_CONTROL_SCHEMA_VERSION, PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
    PROJECTION_SCHEMA_VERSION, SectorCoordinate, TRANSFER_PACKAGE_SCHEMA_VERSION,
    UNIVERSE_MANIFEST_SCHEMA_VERSION, UniverseAddress, UniverseManifestSnapshot, Vec3,
};

use crate::content;

const REGISTRY_DEFINITION: &str =
    include_str!("../../../content/universes/the-verse-local.registry.json");
pub const ADDRESS_SCHEMA_VERSION: u32 = 1;
pub const SECTOR_EDGE_UM: u64 = 20_000_000_000_000;
pub const CELL_EDGE_UM: u64 = 20_000_000_000;
pub const CELLS_PER_SECTOR_AXIS: u32 = 1_000;
pub const GRAVITY_BODY_ID: &str = "khepri-prime";
pub const VOXEL_BODY_ID: &str = "origin-asteroid";
const MICROMETRES_PER_METRE: f64 = 1_000_000.0;
const MAX_EXACT_DERIVED_OFFSET_UM: i128 = (1_i128 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CelestialError {
    #[error("celestial registry definition is invalid JSON: {0}")]
    InvalidJson(String),
    #[error("celestial registry invariant failed: {0}")]
    InvalidRegistry(String),
    #[error("universe address arithmetic overflowed")]
    AddressOverflow,
    #[error("universe address is invalid: {0}")]
    InvalidAddress(String),
    #[error("active-cell position contains a non-finite component")]
    NonFinitePosition,
    #[error("universe address is outside the exact active-physics conversion range")]
    UnsafeDerivedPosition,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryDefinition {
    schema_version: u32,
    license: String,
    universe_id: String,
    generation_rule_version: String,
    minimum_fixed_body_surface_gap_um: u64,
    bodies: Vec<BodyDefinition>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct BodyDefinition {
    body_id: String,
    display_name: String,
    kind: CelestialBodyKind,
    #[serde(default)]
    parent_body_id: Option<String>,
    #[serde(default)]
    field_id: Option<String>,
    center: UniverseAddress,
    surface_radius_um: u64,
    exclusion_radius_um: u64,
    fixed_orientation_microradians: I64Vec3,
    surface_gravity_millimetres_per_second_squared: u64,
    atmosphere_height_um: u64,
    oxygen_parts_per_million: u32,
    #[serde(default)]
    voxel_field_id: Option<String>,
    geometry_definition_id: String,
    #[serde(default)]
    voxel_definition_id: Option<String>,
    material_definition_id: String,
    gravity_definition_id: String,
    atmosphere_definition_id: String,
    resource_definition_id: String,
    visual_descriptor_id: String,
    scale_class: CelestialScaleClass,
    generation_seed_offset: u64,
    materialized_registry_version: u64,
}

#[derive(Serialize)]
struct RegistryHashMaterial<'a> {
    schema_version: u32,
    license: &'a str,
    universe_id: &'a str,
    generation_rule_version: &'a str,
    minimum_fixed_body_surface_gap_um: u64,
    bodies: &'a [CelestialBodySnapshot],
}

#[derive(Serialize)]
struct UniverseManifestHashMaterial<'a> {
    schema_version: u32,
    universe_id: &'a str,
    world_seed: &'a str,
    address_schema_version: u32,
    sector_edge_um: u64,
    cell_edge_um: u64,
    cells_per_sector_axis: u32,
    generation_rule_version: &'a str,
    frontier_policy_version: &'a str,
    celestial_registry_schema_version: u32,
    celestial_registry_hash: &'a str,
    content_schema_version: u32,
    content_manifest_version: &'a str,
    content_hash: &'a str,
    world_schema_version: u32,
    event_schema_version: u32,
    projection_schema_version: u32,
    interest_schema_version: u32,
    operation_fingerprint_schema_version: u32,
    cell_key_schema_version: u32,
    cell_directory_schema_version: u32,
    transfer_package_schema_version: u32,
    lifecycle_control_schema_version: u32,
    production_schedule_occurrence_schema_version: u32,
    lifecycle_policy_hash: &'a str,
}

#[derive(Serialize)]
struct LifecyclePolicyHashMaterial {
    lifecycle_control_schema_version: u32,
    production_schedule_occurrence_schema_version: u32,
    lease_duration_millis: u64,
    lease_renewal_interval_millis: u64,
    lease_write_safety_margin_millis: u64,
    trusted_clock_rollback_tolerance_millis: u64,
    production_occurrence_interval_millis: u64,
    max_background_queue_bearing_machines: u32,
    max_production_catch_up_quanta: u32,
    max_production_catch_up_millis: u64,
    max_claimed_unacknowledged_occurrences: u32,
}

fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, CelestialError> {
    let value = serde_json::to_value(value)
        .map_err(|source| CelestialError::InvalidJson(source.to_string()))?;
    serde_json::to_vec(&value).map_err(|source| CelestialError::InvalidJson(source.to_string()))
}

fn definition() -> &'static RegistryDefinition {
    static DEFINITION: OnceLock<RegistryDefinition> = OnceLock::new();
    DEFINITION.get_or_init(|| {
        serde_json::from_str(REGISTRY_DEFINITION)
            .unwrap_or_else(|source| panic!("embedded celestial registry is invalid: {source}"))
    })
}

pub fn normalize_axis(
    sector: i128,
    cell: i128,
    local_um: i128,
) -> Result<(i128, u32, i64), CelestialError> {
    let cell_edge = i128::from(CELL_EDGE_UM);
    let half_cell = cell_edge / 2;
    let shifted = local_um
        .checked_add(half_cell)
        .ok_or(CelestialError::AddressOverflow)?;
    let local_carry = shifted.div_euclid(cell_edge);
    let normalized_local = shifted.rem_euclid(cell_edge) - half_cell;
    let carried_cell = cell
        .checked_add(local_carry)
        .ok_or(CelestialError::AddressOverflow)?;
    let cells_per_sector = i128::from(CELLS_PER_SECTOR_AXIS);
    let sector_carry = carried_cell.div_euclid(cells_per_sector);
    let normalized_cell = carried_cell.rem_euclid(cells_per_sector);
    let normalized_sector = sector
        .checked_add(sector_carry)
        .ok_or(CelestialError::AddressOverflow)?;
    Ok((
        normalized_sector,
        u32::try_from(normalized_cell).map_err(|_| CelestialError::AddressOverflow)?,
        i64::try_from(normalized_local).map_err(|_| CelestialError::AddressOverflow)?,
    ))
}

pub fn cell_origin_address() -> UniverseAddress {
    UniverseAddress {
        universe_id: definition().universe_id.clone(),
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

pub fn cell_origin_key() -> CellKeyV1 {
    cell_key_from_address(&cell_origin_address())
        .expect("embedded origin address is a canonical cell key")
}

pub fn cell_key_from_address(address: &UniverseAddress) -> Result<CellKeyV1, CelestialError> {
    validate_universe_address(address, &address.universe_id)?;
    Ok(CellKeyV1 {
        schema_version: CELL_KEY_SCHEMA_VERSION,
        universe_id: address.universe_id.clone(),
        sector: address.sector.clone(),
        cell: address.cell,
    })
}

pub fn cell_address_from_key(key: &CellKeyV1) -> Result<UniverseAddress, CelestialError> {
    validate_cell_key(key)?;
    Ok(cell_address_from_parts(key))
}

pub fn validate_cell_key(key: &CellKeyV1) -> Result<(), CelestialError> {
    if key.schema_version != CELL_KEY_SCHEMA_VERSION {
        return Err(CelestialError::InvalidAddress(format!(
            "cell-key schema {} does not match required schema {CELL_KEY_SCHEMA_VERSION}",
            key.schema_version
        )));
    }
    validate_universe_address(&cell_address_from_parts(key), &key.universe_id)
}

fn cell_address_from_parts(key: &CellKeyV1) -> UniverseAddress {
    UniverseAddress {
        universe_id: key.universe_id.clone(),
        sector: key.sector.clone(),
        cell: key.cell,
        local_um: I64Vec3::ZERO,
    }
}

pub fn neighbor_cell_key(key: &CellKeyV1, delta: [i32; 3]) -> Result<CellKeyV1, CelestialError> {
    validate_cell_key(key)?;
    let sectors = [&key.sector.x, &key.sector.y, &key.sector.z];
    let cells = [key.cell.x, key.cell.y, key.cell.z];
    let mut normalized_sectors = [0_i128; 3];
    let mut normalized_cells = [0_u32; 3];
    for axis in 0..3 {
        let sector = sectors[axis]
            .parse::<i128>()
            .map_err(|_| CelestialError::AddressOverflow)?;
        let requested_cell = i128::from(cells[axis])
            .checked_add(i128::from(delta[axis]))
            .ok_or(CelestialError::AddressOverflow)?;
        let (sector, cell, local_um) = normalize_axis(sector, requested_cell, 0)?;
        debug_assert_eq!(local_um, 0);
        normalized_sectors[axis] = sector;
        normalized_cells[axis] = cell;
    }
    Ok(CellKeyV1 {
        schema_version: CELL_KEY_SCHEMA_VERSION,
        universe_id: key.universe_id.clone(),
        sector: SectorCoordinate {
            x: normalized_sectors[0].to_string(),
            y: normalized_sectors[1].to_string(),
            z: normalized_sectors[2].to_string(),
        },
        cell: CellCoordinate {
            x: normalized_cells[0],
            y: normalized_cells[1],
            z: normalized_cells[2],
        },
    })
}

pub fn cell_id(key: &CellKeyV1) -> Result<String, CelestialError> {
    validate_cell_key(key)?;
    let bytes = canonical_json_bytes(key)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"the-verse/cell-key/v1\0");
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn validate_universe_address(
    address: &UniverseAddress,
    expected_universe_id: &str,
) -> Result<(), CelestialError> {
    if address.universe_id != expected_universe_id {
        return Err(CelestialError::InvalidAddress(
            "address belongs to a different universe".into(),
        ));
    }
    for component in [&address.sector.x, &address.sector.y, &address.sector.z] {
        let parsed = component.parse::<i128>().map_err(|_| {
            CelestialError::InvalidAddress("sector coordinate is not signed 128-bit".into())
        })?;
        if parsed.to_string() != *component {
            return Err(CelestialError::InvalidAddress(
                "sector coordinate is not canonical decimal".into(),
            ));
        }
    }
    if [address.cell.x, address.cell.y, address.cell.z]
        .into_iter()
        .any(|cell| cell >= CELLS_PER_SECTOR_AXIS)
    {
        return Err(CelestialError::InvalidAddress(
            "cell coordinate lies outside the sector".into(),
        ));
    }
    let half = i64::try_from(CELL_EDGE_UM / 2).expect("cell edge fits i64");
    if [address.local_um.x, address.local_um.y, address.local_um.z]
        .into_iter()
        .any(|local| !(-half..half).contains(&local))
    {
        return Err(CelestialError::InvalidAddress(
            "local coordinate is not normalized".into(),
        ));
    }
    Ok(())
}

pub fn address_from_origin_offset_um(
    origin: &UniverseAddress,
    offset_um: [i128; 3],
) -> Result<UniverseAddress, CelestialError> {
    validate_universe_address(origin, &origin.universe_id)?;
    let sectors = [&origin.sector.x, &origin.sector.y, &origin.sector.z];
    let cells = [origin.cell.x, origin.cell.y, origin.cell.z];
    let locals = [origin.local_um.x, origin.local_um.y, origin.local_um.z];
    let mut normalized_sectors = [0_i128; 3];
    let mut normalized_cells = [0_u32; 3];
    let mut normalized_locals = [0_i64; 3];
    for axis in 0..3 {
        let sector = sectors[axis]
            .parse::<i128>()
            .map_err(|_| CelestialError::AddressOverflow)?;
        let local = i128::from(locals[axis])
            .checked_add(offset_um[axis])
            .ok_or(CelestialError::AddressOverflow)?;
        let (sector, cell, local) = normalize_axis(sector, i128::from(cells[axis]), local)?;
        normalized_sectors[axis] = sector;
        normalized_cells[axis] = cell;
        normalized_locals[axis] = local;
    }
    Ok(UniverseAddress {
        universe_id: origin.universe_id.clone(),
        sector: SectorCoordinate {
            x: normalized_sectors[0].to_string(),
            y: normalized_sectors[1].to_string(),
            z: normalized_sectors[2].to_string(),
        },
        cell: CellCoordinate {
            x: normalized_cells[0],
            y: normalized_cells[1],
            z: normalized_cells[2],
        },
        local_um: I64Vec3::new(
            normalized_locals[0],
            normalized_locals[1],
            normalized_locals[2],
        ),
    })
}

pub fn relative_offset_um(
    origin: &UniverseAddress,
    address: &UniverseAddress,
) -> Result<[i128; 3], CelestialError> {
    validate_universe_address(origin, &origin.universe_id)?;
    validate_universe_address(address, &origin.universe_id)?;
    let origin_sectors = [&origin.sector.x, &origin.sector.y, &origin.sector.z];
    let address_sectors = [&address.sector.x, &address.sector.y, &address.sector.z];
    let origin_cells = [origin.cell.x, origin.cell.y, origin.cell.z];
    let address_cells = [address.cell.x, address.cell.y, address.cell.z];
    let origin_locals = [origin.local_um.x, origin.local_um.y, origin.local_um.z];
    let address_locals = [address.local_um.x, address.local_um.y, address.local_um.z];
    let mut offset = [0_i128; 3];
    for axis in 0..3 {
        let origin_sector = origin_sectors[axis]
            .parse::<i128>()
            .map_err(|_| CelestialError::AddressOverflow)?;
        let address_sector = address_sectors[axis]
            .parse::<i128>()
            .map_err(|_| CelestialError::AddressOverflow)?;
        offset[axis] = address_sector
            .checked_sub(origin_sector)
            .and_then(|value| value.checked_mul(i128::from(SECTOR_EDGE_UM)))
            .and_then(|value| {
                let cell_delta =
                    i128::from(address_cells[axis]).checked_sub(i128::from(origin_cells[axis]))?;
                value.checked_add(cell_delta.checked_mul(i128::from(CELL_EDGE_UM))?)
            })
            .and_then(|value| {
                value
                    .checked_add(i128::from(address_locals[axis]) - i128::from(origin_locals[axis]))
            })
            .ok_or(CelestialError::AddressOverflow)?;
    }
    Ok(offset)
}

fn metres_to_exact_um(component: f64) -> Result<i128, CelestialError> {
    if !component.is_finite() {
        return Err(CelestialError::NonFinitePosition);
    }
    let scaled = component * MICROMETRES_PER_METRE;
    if !scaled.is_finite() || scaled.abs() > MAX_EXACT_DERIVED_OFFSET_UM as f64 {
        return Err(CelestialError::UnsafeDerivedPosition);
    }
    Ok(scaled.round() as i128)
}

pub fn address_from_local_position(
    origin: &UniverseAddress,
    position: Vec3,
) -> Result<UniverseAddress, CelestialError> {
    address_from_origin_offset_um(
        origin,
        [
            metres_to_exact_um(position.x)?,
            metres_to_exact_um(position.y)?,
            metres_to_exact_um(position.z)?,
        ],
    )
}

pub fn local_position_from_address(
    origin: &UniverseAddress,
    address: &UniverseAddress,
) -> Result<Vec3, CelestialError> {
    let offset = relative_offset_um(origin, address)?;
    if offset
        .into_iter()
        .any(|component| component.unsigned_abs() > MAX_EXACT_DERIVED_OFFSET_UM as u128)
    {
        return Err(CelestialError::UnsafeDerivedPosition);
    }
    Ok(Vec3::new(
        offset[0] as f64 / MICROMETRES_PER_METRE,
        offset[1] as f64 / MICROMETRES_PER_METRE,
        offset[2] as f64 / MICROMETRES_PER_METRE,
    ))
}

pub fn registry_snapshot(world_seed: u64) -> Result<CelestialRegistrySnapshot, CelestialError> {
    build_registry(definition(), world_seed)
}

fn build_registry(
    definition: &RegistryDefinition,
    world_seed: u64,
) -> Result<CelestialRegistrySnapshot, CelestialError> {
    if definition.schema_version != CELESTIAL_REGISTRY_SCHEMA_VERSION {
        return Err(CelestialError::InvalidRegistry(format!(
            "registry schema {} does not match required schema {CELESTIAL_REGISTRY_SCHEMA_VERSION}",
            definition.schema_version
        )));
    }
    if definition.license != "CC-BY-SA-4.0" {
        return Err(CelestialError::InvalidRegistry(
            "celestial registry license must be CC-BY-SA-4.0".into(),
        ));
    }
    if definition.universe_id != "the-verse-local"
        || definition.generation_rule_version != "p1.5-proof-1"
        || definition.minimum_fixed_body_surface_gap_um != 3_000_000_000
    {
        return Err(CelestialError::InvalidRegistry(
            "P1.5 proof registry header does not match the accepted manifest".into(),
        ));
    }

    let content = content::manifest();
    let content_hash = content::manifest_hash();
    let bodies = definition
        .bodies
        .iter()
        .map(|body| {
            let generation_seed = world_seed
                .checked_add(body.generation_seed_offset)
                .ok_or_else(|| {
                    CelestialError::InvalidRegistry(format!(
                        "body {} generation seed overflows",
                        body.body_id
                    ))
                })?;
            Ok(CelestialBodySnapshot {
                body_id: body.body_id.clone(),
                display_name: body.display_name.clone(),
                kind: body.kind,
                parent_body_id: body.parent_body_id.clone(),
                field_id: body.field_id.clone(),
                center: body.center.clone(),
                surface_radius_um: body.surface_radius_um,
                exclusion_radius_um: body.exclusion_radius_um,
                fixed_orientation_microradians: body.fixed_orientation_microradians,
                surface_gravity_millimetres_per_second_squared: body
                    .surface_gravity_millimetres_per_second_squared,
                atmosphere_height_um: body.atmosphere_height_um,
                oxygen_parts_per_million: body.oxygen_parts_per_million,
                voxel_field_id: body.voxel_field_id.clone(),
                geometry_definition_id: body.geometry_definition_id.clone(),
                voxel_definition_id: body.voxel_definition_id.clone(),
                material_definition_id: body.material_definition_id.clone(),
                gravity_definition_id: body.gravity_definition_id.clone(),
                atmosphere_definition_id: body.atmosphere_definition_id.clone(),
                resource_definition_id: body.resource_definition_id.clone(),
                visual_descriptor_id: body.visual_descriptor_id.clone(),
                scale_class: body.scale_class,
                generation_seed: generation_seed.to_string(),
                generation_rule_version: definition.generation_rule_version.clone(),
                materialized_registry_version: body.materialized_registry_version,
                content_manifest_version: content.manifest_version.clone(),
                content_hash: content_hash.into(),
            })
        })
        .collect::<Result<Vec<_>, CelestialError>>()?;
    validate_registry(definition, &bodies)?;

    let material = RegistryHashMaterial {
        schema_version: definition.schema_version,
        license: &definition.license,
        universe_id: &definition.universe_id,
        generation_rule_version: &definition.generation_rule_version,
        minimum_fixed_body_surface_gap_um: definition.minimum_fixed_body_surface_gap_um,
        bodies: &bodies,
    };
    let bytes = canonical_json_bytes(&material)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"the-verse/celestial-registry/v1\0");
    hasher.update(&bytes);

    Ok(CelestialRegistrySnapshot {
        schema_version: definition.schema_version,
        registry_hash: hasher.finalize().to_hex().to_string(),
        license: definition.license.clone(),
        universe_id: definition.universe_id.clone(),
        generation_rule_version: definition.generation_rule_version.clone(),
        minimum_fixed_body_surface_gap_um: definition.minimum_fixed_body_surface_gap_um,
        bodies,
    })
}

fn validate_registry(
    definition: &RegistryDefinition,
    bodies: &[CelestialBodySnapshot],
) -> Result<(), CelestialError> {
    if bodies.is_empty() {
        return Err(CelestialError::InvalidRegistry(
            "celestial registry must contain at least one body".into(),
        ));
    }
    if !bodies
        .windows(2)
        .all(|pair| pair[0].body_id < pair[1].body_id)
    {
        return Err(CelestialError::InvalidRegistry(
            "celestial bodies must be unique and sorted by body ID".into(),
        ));
    }

    let by_id = bodies
        .iter()
        .map(|body| (body.body_id.as_str(), body))
        .collect::<BTreeMap<_, _>>();
    let mut centers = BTreeSet::new();
    for body in bodies {
        validate_body(definition, body, &by_id)?;
        if !centers.insert(body.center.clone()) {
            return Err(CelestialError::InvalidRegistry(format!(
                "body {} duplicates another normalized center",
                body.body_id
            )));
        }
        let mut ancestry = BTreeSet::new();
        let mut cursor = body;
        while let Some(parent_id) = &cursor.parent_body_id {
            if !ancestry.insert(cursor.body_id.as_str()) {
                return Err(CelestialError::InvalidRegistry(format!(
                    "body {} has cyclic parentage",
                    body.body_id
                )));
            }
            cursor = by_id.get(parent_id.as_str()).ok_or_else(|| {
                CelestialError::InvalidRegistry(format!(
                    "body {} references missing parent {parent_id}",
                    body.body_id
                ))
            })?;
        }
        if body.kind == CelestialBodyKind::Asteroid {
            let field_id = body.field_id.as_deref().ok_or_else(|| {
                CelestialError::InvalidRegistry(format!(
                    "asteroid {} requires a registered asteroid field",
                    body.body_id
                ))
            })?;
            let field = by_id.get(field_id).ok_or_else(|| {
                CelestialError::InvalidRegistry(format!(
                    "asteroid {} references missing field {field_id}",
                    body.body_id
                ))
            })?;
            if !asteroid_field_contains(field, body)? {
                return Err(CelestialError::InvalidRegistry(format!(
                    "asteroid {} is outside registered field {field_id}",
                    body.body_id
                )));
            }
        }
    }

    for (index, left) in bodies.iter().enumerate() {
        for right in &bodies[index + 1..] {
            if is_asteroid_field_pair(left, right) {
                continue;
            }
            validate_separation(left, right, definition.minimum_fixed_body_surface_gap_um)?;
        }
    }
    Ok(())
}

fn validate_body(
    definition: &RegistryDefinition,
    body: &CelestialBodySnapshot,
    by_id: &BTreeMap<&str, &CelestialBodySnapshot>,
) -> Result<(), CelestialError> {
    if body.body_id.is_empty()
        || body.body_id.len() > 128
        || !body
            .body_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || body.display_name.trim().is_empty()
        || body.visual_descriptor_id.trim().is_empty()
        || body.materialized_registry_version == 0
    {
        return Err(CelestialError::InvalidRegistry(
            "celestial identity and visual descriptor must be bounded".into(),
        ));
    }
    validate_address(&body.center, &definition.universe_id)?;
    if body.surface_radius_um == 0
        || body.exclusion_radius_um < body.surface_radius_um
        || body.exclusion_radius_um > CELL_EDGE_UM / 2
    {
        return Err(CelestialError::InvalidRegistry(format!(
            "body {} has invalid radii",
            body.body_id
        )));
    }
    let half_cell = i128::from(CELL_EDGE_UM / 2);
    let exclusion = i128::from(body.exclusion_radius_um);
    for local in [
        body.center.local_um.x,
        body.center.local_um.y,
        body.center.local_um.z,
    ] {
        if i128::from(local).abs() + exclusion >= half_cell {
            return Err(CelestialError::InvalidRegistry(format!(
                "body {} exclusion volume crosses its bounded proof cell",
                body.body_id
            )));
        }
    }
    match body.kind {
        CelestialBodyKind::Moon => {
            let parent_id = body.parent_body_id.as_deref().ok_or_else(|| {
                CelestialError::InvalidRegistry(format!(
                    "moon {} requires a planet parent",
                    body.body_id
                ))
            })?;
            if parent_id == body.body_id
                || by_id.get(parent_id).map(|parent| parent.kind) != Some(CelestialBodyKind::Planet)
            {
                return Err(CelestialError::InvalidRegistry(format!(
                    "moon {} parent must be a distinct registered planet",
                    body.body_id
                )));
            }
        }
        CelestialBodyKind::Planet => {
            if body.parent_body_id.is_some()
                || body.surface_gravity_millimetres_per_second_squared == 0
            {
                return Err(CelestialError::InvalidRegistry(format!(
                    "planet {} cannot have a parent and must define gravity",
                    body.body_id
                )));
            }
        }
        CelestialBodyKind::Asteroid => {
            if body.parent_body_id.is_some() {
                return Err(CelestialError::InvalidRegistry(format!(
                    "body {} kind cannot have a parent in schema 1",
                    body.body_id
                )));
            }
        }
        CelestialBodyKind::AsteroidField => {
            if body.parent_body_id.is_some()
                || body.field_id.as_deref() != Some(body.body_id.as_str())
                || body.voxel_field_id.is_some()
                || body.voxel_definition_id.is_some()
            {
                return Err(CelestialError::InvalidRegistry(format!(
                    "asteroid field {} must own its field ID and cannot be a voxel body",
                    body.body_id
                )));
            }
        }
    }
    if body.kind == CelestialBodyKind::Asteroid && body.voxel_field_id.is_none() {
        return Err(CelestialError::InvalidRegistry(format!(
            "asteroid {} must bind a voxel field",
            body.body_id
        )));
    }
    let celestial = &content::manifest().celestial;
    if !celestial.contains_geometry(&body.geometry_definition_id)
        || body
            .voxel_definition_id
            .as_deref()
            .is_some_and(|value| !celestial.contains_voxel(value))
        || !celestial.contains_material(&body.material_definition_id)
        || !celestial.contains_gravity(&body.gravity_definition_id)
        || !celestial.contains_atmosphere(&body.atmosphere_definition_id)
        || !celestial.contains_resource(&body.resource_definition_id)
        || (body.kind == CelestialBodyKind::Asteroid && body.voxel_definition_id.is_none())
        || definition.minimum_fixed_body_surface_gap_um
            != celestial.minimum_fixed_body_surface_gap_um
    {
        return Err(CelestialError::InvalidRegistry(format!(
            "body {} references an unknown content definition or unpinned separation policy",
            body.body_id
        )));
    }
    if body.content_manifest_version != content::manifest().manifest_version
        || body.content_hash != content::manifest_hash()
    {
        return Err(CelestialError::InvalidRegistry(format!(
            "body {} content binding does not match the active manifest",
            body.body_id
        )));
    }
    Ok(())
}

fn validate_address(address: &UniverseAddress, universe_id: &str) -> Result<(), CelestialError> {
    validate_universe_address(address, universe_id).map_err(|source| {
        CelestialError::InvalidRegistry(format!("celestial body address is invalid: {source}"))
    })
}

fn global_axis_um(sector: &str, cell: u32, local_um: i64) -> Result<i128, CelestialError> {
    sector
        .parse::<i128>()
        .map_err(|_| CelestialError::AddressOverflow)?
        .checked_mul(i128::from(SECTOR_EDGE_UM))
        .and_then(|value| value.checked_add(i128::from(cell) * i128::from(CELL_EDGE_UM)))
        .and_then(|value| value.checked_add(i128::from(local_um)))
        .ok_or(CelestialError::AddressOverflow)
}

fn global_position_um(address: &UniverseAddress) -> Result<[i128; 3], CelestialError> {
    Ok([
        global_axis_um(&address.sector.x, address.cell.x, address.local_um.x)?,
        global_axis_um(&address.sector.y, address.cell.y, address.local_um.y)?,
        global_axis_um(&address.sector.z, address.cell.z, address.local_um.z)?,
    ])
}

fn is_asteroid_field_pair(left: &CelestialBodySnapshot, right: &CelestialBodySnapshot) -> bool {
    match (left.kind, right.kind) {
        (CelestialBodyKind::Asteroid, CelestialBodyKind::AsteroidField) => {
            left.field_id.as_deref() == Some(right.body_id.as_str())
        }
        (CelestialBodyKind::AsteroidField, CelestialBodyKind::Asteroid) => {
            right.field_id.as_deref() == Some(left.body_id.as_str())
        }
        _ => false,
    }
}

/// Exact deterministic membership for one materialized asteroid and its
/// bounded schema-1 field. The asteroid exclusion volume must fit wholly
/// inside the field's published surface radius.
pub fn asteroid_field_contains(
    field: &CelestialBodySnapshot,
    asteroid: &CelestialBodySnapshot,
) -> Result<bool, CelestialError> {
    if field.kind != CelestialBodyKind::AsteroidField
        || asteroid.kind != CelestialBodyKind::Asteroid
        || asteroid.field_id.as_deref() != Some(field.body_id.as_str())
    {
        return Ok(false);
    }
    let available_radius = field
        .surface_radius_um
        .checked_sub(asteroid.exclusion_radius_um)
        .ok_or_else(|| {
            CelestialError::InvalidRegistry(format!(
                "asteroid {} cannot fit inside field {}",
                asteroid.body_id, field.body_id
            ))
        })?;
    let field_position = global_position_um(&field.center)?;
    let asteroid_position = global_position_um(&asteroid.center)?;
    let distance_squared = field_position.into_iter().zip(asteroid_position).try_fold(
        0_u128,
        |sum, (left, right)| {
            let delta = left.abs_diff(right);
            delta
                .checked_mul(delta)
                .and_then(|square| sum.checked_add(square))
                .ok_or(CelestialError::AddressOverflow)
        },
    )?;
    let available_squared = u128::from(available_radius)
        .checked_mul(u128::from(available_radius))
        .ok_or(CelestialError::AddressOverflow)?;
    Ok(distance_squared <= available_squared)
}

fn validate_separation(
    left: &CelestialBodySnapshot,
    right: &CelestialBodySnapshot,
    minimum_gap_um: u64,
) -> Result<(), CelestialError> {
    let left_position = global_position_um(&left.center)?;
    let right_position = global_position_um(&right.center)?;
    let required = u128::from(left.exclusion_radius_um)
        + u128::from(right.exclusion_radius_um)
        + u128::from(minimum_gap_um);
    let deltas = [
        left_position[0].abs_diff(right_position[0]),
        left_position[1].abs_diff(right_position[1]),
        left_position[2].abs_diff(right_position[2]),
    ];
    if deltas.into_iter().any(|delta| delta >= required) {
        return Ok(());
    }
    let distance_squared = deltas.into_iter().try_fold(0_u128, |sum, delta| {
        delta
            .checked_mul(delta)
            .and_then(|square| sum.checked_add(square))
            .ok_or(CelestialError::AddressOverflow)
    })?;
    let required_squared = required
        .checked_mul(required)
        .ok_or(CelestialError::AddressOverflow)?;
    if distance_squared < required_squared {
        return Err(CelestialError::InvalidRegistry(format!(
            "bodies {} and {} violate the fixed-body surface gap",
            left.body_id, right.body_id
        )));
    }
    Ok(())
}

pub fn universe_manifest(
    world_seed: u64,
    world_schema_version: u32,
    event_schema_version: u32,
) -> Result<UniverseManifestSnapshot, CelestialError> {
    let registry = registry_snapshot(world_seed)?;
    let content = content::manifest();
    let world_seed = world_seed.to_string();
    let frontier_policy_version = "closed-proof-frontier-v1";
    let universe_id = registry.universe_id.clone();
    let generation_rule_version = registry.generation_rule_version.clone();
    let celestial_registry_hash = registry.registry_hash.clone();
    let lifecycle_policy_hash = lifecycle_policy_hash()?;
    let material = UniverseManifestHashMaterial {
        schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
        universe_id: &universe_id,
        world_seed: &world_seed,
        address_schema_version: ADDRESS_SCHEMA_VERSION,
        sector_edge_um: SECTOR_EDGE_UM,
        cell_edge_um: CELL_EDGE_UM,
        cells_per_sector_axis: CELLS_PER_SECTOR_AXIS,
        generation_rule_version: &generation_rule_version,
        frontier_policy_version,
        celestial_registry_schema_version: registry.schema_version,
        celestial_registry_hash: &celestial_registry_hash,
        content_schema_version: content.schema_version,
        content_manifest_version: &content.manifest_version,
        content_hash: content::manifest_hash(),
        world_schema_version,
        event_schema_version,
        projection_schema_version: PROJECTION_SCHEMA_VERSION,
        interest_schema_version: INTEREST_SCHEMA_VERSION,
        operation_fingerprint_schema_version: INTENT_FINGERPRINT_SCHEMA_VERSION,
        cell_key_schema_version: CELL_KEY_SCHEMA_VERSION,
        cell_directory_schema_version: CELL_DIRECTORY_SCHEMA_VERSION,
        transfer_package_schema_version: TRANSFER_PACKAGE_SCHEMA_VERSION,
        lifecycle_control_schema_version: LIFECYCLE_CONTROL_SCHEMA_VERSION,
        production_schedule_occurrence_schema_version:
            PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
        lifecycle_policy_hash: &lifecycle_policy_hash,
    };
    let bytes = canonical_json_bytes(&material)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"the-verse/universe-manifest/v4\0");
    hasher.update(&bytes);
    let manifest_hash = hasher.finalize().to_hex().to_string();
    Ok(UniverseManifestSnapshot {
        schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
        manifest_hash,
        universe_id,
        world_seed,
        address_schema_version: ADDRESS_SCHEMA_VERSION,
        sector_edge_um: SECTOR_EDGE_UM,
        cell_edge_um: CELL_EDGE_UM,
        cells_per_sector_axis: CELLS_PER_SECTOR_AXIS,
        generation_rule_version,
        frontier_policy_version: frontier_policy_version.into(),
        celestial_registry_schema_version: registry.schema_version,
        celestial_registry_hash,
        content_schema_version: content.schema_version,
        content_manifest_version: content.manifest_version.clone(),
        content_hash: content::manifest_hash().into(),
        world_schema_version,
        event_schema_version,
        projection_schema_version: PROJECTION_SCHEMA_VERSION,
        interest_schema_version: INTEREST_SCHEMA_VERSION,
        operation_fingerprint_schema_version: INTENT_FINGERPRINT_SCHEMA_VERSION,
        cell_key_schema_version: CELL_KEY_SCHEMA_VERSION,
        cell_directory_schema_version: CELL_DIRECTORY_SCHEMA_VERSION,
        transfer_package_schema_version: TRANSFER_PACKAGE_SCHEMA_VERSION,
        lifecycle_control_schema_version: LIFECYCLE_CONTROL_SCHEMA_VERSION,
        production_schedule_occurrence_schema_version:
            PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
        lifecycle_policy_hash,
    })
}

pub(crate) fn lifecycle_policy_hash() -> Result<String, CelestialError> {
    let material = LifecyclePolicyHashMaterial {
        lifecycle_control_schema_version: LIFECYCLE_CONTROL_SCHEMA_VERSION,
        production_schedule_occurrence_schema_version:
            PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
        lease_duration_millis: 15_000,
        lease_renewal_interval_millis: 5_000,
        lease_write_safety_margin_millis: 5_000,
        trusted_clock_rollback_tolerance_millis: 1_000,
        production_occurrence_interval_millis: 1_000,
        max_background_queue_bearing_machines: 256,
        max_production_catch_up_quanta: 60,
        max_production_catch_up_millis: 250,
        max_claimed_unacknowledged_occurrences: 1,
    };
    let bytes = canonical_json_bytes(&material)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"the-verse/lifecycle-policy/v1\0");
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

pub fn body_snapshot(world_seed: u64, body_id: &str) -> CelestialBodySnapshot {
    registry_snapshot(world_seed)
        .expect("embedded registry is valid")
        .bodies
        .into_iter()
        .find(|body| body.body_id == body_id)
        .unwrap_or_else(|| panic!("embedded registry body {body_id} exists"))
}

pub fn body_center_m(body_id: &str) -> Vec3 {
    let body = definition()
        .bodies
        .iter()
        .find(|body| body.body_id == body_id)
        .unwrap_or_else(|| panic!("embedded registry body {body_id} exists"));
    let body_position = global_position_um(&body.center)
        .expect("embedded registry body center fits signed address arithmetic");
    let active_origin = global_position_um(&cell_origin_address())
        .expect("embedded active-cell origin fits signed address arithmetic");
    Vec3::new(
        (body_position[0] - active_origin[0]) as f64 / 1_000_000.0,
        (body_position[1] - active_origin[1]) as f64 / 1_000_000.0,
        (body_position[2] - active_origin[2]) as f64 / 1_000_000.0,
    )
}

pub fn body_surface_radius_m(body_id: &str) -> f64 {
    definition()
        .bodies
        .iter()
        .find(|body| body.body_id == body_id)
        .map_or_else(
            || panic!("embedded registry body {body_id} exists"),
            |body| body.surface_radius_um as f64 / 1_000_000.0,
        )
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn bounded_exact_offsets_round_trip_through_hierarchical_addresses(
            x in -100_000_000_000_000_i64..=100_000_000_000_000_i64,
            y in -100_000_000_000_000_i64..=100_000_000_000_000_i64,
            z in -100_000_000_000_000_i64..=100_000_000_000_000_i64,
        ) {
            let origin = cell_origin_address();
            let offset = [i128::from(x), i128::from(y), i128::from(z)];
            let address = address_from_origin_offset_um(&origin, offset)
                .expect("bounded property offset canonicalizes");
            prop_assert_eq!(relative_offset_um(&origin, &address), Ok(offset));
            prop_assert_eq!(
                address_from_origin_offset_um(
                    &origin,
                    relative_offset_um(&origin, &address).expect("relative offset fits"),
                ),
                Ok(address),
            );
        }
    }

    #[test]
    fn registry_and_manifest_are_deterministic_and_bound_to_seed() {
        let first = registry_snapshot(41).expect("registry builds");
        let second = registry_snapshot(41).expect("registry rebuilds");
        let other = registry_snapshot(42).expect("other registry builds");
        assert_eq!(first, second);
        assert_ne!(first.registry_hash, other.registry_hash);
        assert_eq!(
            first
                .bodies
                .iter()
                .map(|body| body.body_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "khepri-prime",
                "origin-asteroid",
                "origin-field",
                "sable-moon"
            ]
        );
        let manifest = universe_manifest(41, 19, 15).expect("manifest builds");
        assert_eq!(manifest.celestial_registry_hash, first.registry_hash);
        assert_eq!(manifest.content_hash, content::manifest_hash());
        assert_eq!(manifest.manifest_hash.len(), 64);
    }

    #[test]
    fn registry_and_universe_manifest_match_cross_platform_golden_hashes() {
        let seed = 20_260_826;
        let registry = registry_snapshot(seed).expect("golden registry builds");
        let manifest = universe_manifest(
            seed,
            crate::WORLD_SCHEMA_VERSION,
            crate::EVENT_SCHEMA_VERSION,
        )
        .expect("golden manifest builds");
        assert_eq!(
            manifest.lifecycle_policy_hash,
            "8abc99b5e076bd89a8914c3727560baaa82433b1b1b4191b2379355ac7d81471"
        );
        assert_eq!(
            registry.registry_hash,
            "4c367bbfa04218ece14104f0a3a7ec2c7e9fefcc37d4cf78a265df2d711a59da"
        );
        assert_eq!(
            manifest.manifest_hash,
            "3e93c305169eeecee44f2630e57ad183b319375197547344c45e1509e8aaf76b"
        );
    }

    #[test]
    fn address_normalization_uses_euclidean_carries() {
        assert_eq!(normalize_axis(0, 500, 0), Ok((0, 500, 0)));
        assert_eq!(
            normalize_axis(0, 500, i128::from(CELL_EDGE_UM / 2)),
            Ok((0, 501, -10_000_000_000))
        );
        assert_eq!(
            normalize_axis(0, 0, -10_000_000_001),
            Ok((-1, 999, 9_999_999_999))
        );
        assert!(normalize_axis(i128::MAX, 999, i128::from(CELL_EDGE_UM)).is_err());
    }

    #[test]
    fn cell_keys_have_stable_identity_and_neighbor_carries() {
        let origin = cell_origin_key();
        assert_eq!(origin.schema_version, CELL_KEY_SCHEMA_VERSION);
        assert_eq!(origin.sector.x, "0");
        assert_eq!(origin.cell.x, 500);
        assert_eq!(
            cell_address_from_key(&origin).expect("origin key becomes an address"),
            cell_origin_address()
        );

        let east = neighbor_cell_key(&origin, [1, 0, 0]).expect("east neighbor derives");
        assert_eq!(east.sector.x, "0");
        assert_eq!(east.cell.x, 501);
        assert_ne!(cell_id(&origin), cell_id(&east));

        let mut low = origin.clone();
        low.sector.x = "0".into();
        low.cell.x = 0;
        let west = neighbor_cell_key(&low, [-1, 0, 0]).expect("west neighbor carries");
        assert_eq!(west.sector.x, "-1");
        assert_eq!(west.cell.x, 999);

        let mut high = origin.clone();
        high.sector.y = "-2".into();
        high.cell.y = 999;
        let north = neighbor_cell_key(&high, [0, 1, 0]).expect("north neighbor carries");
        assert_eq!(north.sector.y, "-1");
        assert_eq!(north.cell.y, 0);
    }

    #[test]
    fn cell_key_identity_has_cross_platform_golden_hashes() {
        let origin = cell_origin_key();
        let east = neighbor_cell_key(&origin, [1, 0, 0]).expect("east neighbor derives");
        assert_eq!(
            cell_id(&origin).expect("origin cell hashes"),
            "5110e8ef07316dc5fc8cd48210915d3e879779c67dc3e11a9da0402656c76d17"
        );
        assert_eq!(
            cell_id(&east).expect("east cell hashes"),
            "e24242afc42c71a9629093e0c82b1779e306e92c52804ebc105ef373fa5a8f4d"
        );
    }

    #[test]
    fn cell_keys_reject_noncanonical_or_wrong_schema_material() {
        let mut key = cell_origin_key();
        key.sector.x = "-0".into();
        assert!(validate_cell_key(&key).is_err());

        let mut key = cell_origin_key();
        key.cell.z = CELLS_PER_SECTOR_AXIS;
        assert!(validate_cell_key(&key).is_err());

        let mut key = cell_origin_key();
        key.schema_version += 1;
        assert!(validate_cell_key(&key).is_err());
    }

    #[test]
    fn registry_rejects_noncanonical_addresses_parent_errors_and_separation() {
        let mut malformed = definition().clone();
        malformed.bodies[0].center.sector.x = "-0".into();
        assert!(build_registry(&malformed, 7).is_err());

        let mut missing_parent = definition().clone();
        missing_parent.bodies[2].parent_body_id = Some("missing".into());
        assert!(build_registry(&missing_parent, 7).is_err());

        let mut self_parent = definition().clone();
        self_parent.bodies[2].parent_body_id = Some("sable-moon".into());
        assert!(build_registry(&self_parent, 7).is_err());

        let mut too_close = definition().clone();
        too_close.bodies[2].center.local_um =
            I64Vec3::new(2_000_000_000, -2_200_000_000, -3_800_000_000);
        assert!(build_registry(&too_close, 7).is_err());
    }

    #[test]
    fn proof_bodies_meet_the_integer_surface_gap() {
        let registry = registry_snapshot(99).expect("registry validates");
        for (index, left) in registry.bodies.iter().enumerate() {
            for right in &registry.bodies[index + 1..] {
                if is_asteroid_field_pair(left, right) {
                    continue;
                }
                validate_separation(left, right, registry.minimum_fixed_body_surface_gap_um)
                    .expect("proof bodies meet the configured surface gap");
            }
        }
    }

    #[test]
    fn asteroid_field_membership_is_exact_deterministic_and_bounded() {
        let registry = registry_snapshot(99).expect("registry validates");
        let field = registry
            .bodies
            .iter()
            .find(|body| body.body_id == "origin-field")
            .expect("origin field exists");
        let asteroid = registry
            .bodies
            .iter()
            .find(|body| body.body_id == VOXEL_BODY_ID)
            .expect("origin asteroid exists");
        assert_eq!(asteroid_field_contains(field, asteroid), Ok(true));
        assert_eq!(asteroid_field_contains(field, asteroid), Ok(true));

        let mut outside = asteroid.clone();
        let available = field.surface_radius_um - outside.exclusion_radius_um;
        outside.center = field.center.clone();
        outside.center.local_um.x +=
            i64::try_from(available + 1).expect("proof field radius fits i64");
        assert_eq!(asteroid_field_contains(field, &outside), Ok(false));

        let mut missing = definition().clone();
        missing.bodies.retain(|body| body.body_id != "origin-field");
        assert!(build_registry(&missing, 99).is_err());
    }

    #[test]
    fn separation_accepts_equality_and_rejects_one_micrometre_below() {
        let registry = registry_snapshot(151).expect("registry validates");
        let left = registry
            .bodies
            .iter()
            .find(|body| body.body_id == VOXEL_BODY_ID)
            .expect("origin asteroid exists")
            .clone();
        let mut right = registry
            .bodies
            .iter()
            .find(|body| body.body_id == "sable-moon")
            .expect("registered moon exists")
            .clone();
        let required = left.exclusion_radius_um
            + right.exclusion_radius_um
            + registry.minimum_fixed_body_surface_gap_um;
        right.center = left.center.clone();
        right.center.local_um.x += i64::try_from(required).expect("proof distance fits i64");
        validate_separation(&left, &right, registry.minimum_fixed_body_surface_gap_um)
            .expect("equality satisfies the pinned surface gap");

        right.center.local_um.x -= 1;
        assert!(
            validate_separation(&left, &right, registry.minimum_fixed_body_surface_gap_um,)
                .is_err(),
            "one micrometre below the boundary must fail"
        );
    }

    #[test]
    fn signed_sector_coordinates_round_trip_as_exact_json_strings() {
        let mut address = cell_origin_address();
        address.sector.x = i128::MIN.to_string();
        address.sector.y = i128::MAX.to_string();
        address.sector.z = "-9007199254740993".into();
        let encoded = serde_json::to_string(&address).expect("address serializes");
        assert!(encoded.contains("\"-9007199254740993\""));
        assert_eq!(
            serde_json::from_str::<UniverseAddress>(&encoded).expect("address deserializes"),
            address
        );
    }

    #[test]
    fn registry_definition_rejects_unknown_header_and_body_fields() {
        let source = serde_json::from_str::<serde_json::Value>(REGISTRY_DEFINITION)
            .expect("embedded registry is JSON");

        let mut unknown_header = source.clone();
        unknown_header
            .as_object_mut()
            .expect("registry is an object")
            .insert("unexpected_header".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<RegistryDefinition>(unknown_header).is_err());

        let mut unknown_body = source;
        unknown_body["bodies"][0]
            .as_object_mut()
            .expect("body is an object")
            .insert("unexpected_body_field".into(), serde_json::json!(true));
        assert!(serde_json::from_value::<RegistryDefinition>(unknown_body).is_err());
    }

    #[test]
    fn exact_address_conversion_carries_across_half_cell_cell_and_sector_boundaries() {
        let origin = cell_origin_address();
        let half_cell = i128::from(CELL_EDGE_UM / 2);
        let positive_half = address_from_origin_offset_um(&origin, [half_cell, 0, 0])
            .expect("positive half-cell normalizes");
        assert_eq!(positive_half.cell.x, 501);
        assert_eq!(positive_half.local_um.x, -10_000_000_000);
        assert_eq!(
            relative_offset_um(&origin, &positive_half),
            Ok([half_cell, 0, 0])
        );

        let negative_half = address_from_origin_offset_um(&origin, [-half_cell, 0, 0])
            .expect("negative half-cell remains in the canonical half-open interval");
        assert_eq!(negative_half.cell.x, 500);
        assert_eq!(negative_half.local_um.x, -10_000_000_000);

        let sector_forward = address_from_origin_offset_um(
            &origin,
            [i128::from(CELL_EDGE_UM) * 500 + half_cell, 0, 0],
        )
        .expect("positive cell carry crosses the sector");
        assert_eq!(sector_forward.sector.x, "1");
        assert_eq!(sector_forward.cell.x, 1);
        assert_eq!(sector_forward.local_um.x, -10_000_000_000);

        let sector_backward = address_from_origin_offset_um(
            &origin,
            [-(i128::from(CELL_EDGE_UM) * 501) - half_cell - 1, 0, 0],
        )
        .expect("negative Euclidean carry crosses into a negative sector");
        assert_eq!(sector_backward.sector.x, "-1");
        assert_eq!(sector_backward.cell.x, 998);
        assert_eq!(sector_backward.local_um.x, 9_999_999_999);
        assert_eq!(
            address_from_origin_offset_um(
                &origin,
                relative_offset_um(&origin, &sector_backward).expect("relative offset fits"),
            ),
            Ok(sector_backward)
        );
    }

    #[test]
    fn address_arithmetic_handles_i128_extremes_without_wrapping() {
        assert_eq!(normalize_axis(i128::MAX, 0, 0), Ok((i128::MAX, 0, 0)));
        assert_eq!(normalize_axis(i128::MIN, 0, 0), Ok((i128::MIN, 0, 0)));
        assert!(normalize_axis(i128::MAX, 999, i128::from(CELL_EDGE_UM)).is_err());
        assert!(normalize_axis(i128::MIN, 0, -i128::from(CELL_EDGE_UM)).is_err());

        let mut far = cell_origin_address();
        far.sector.x = i128::MAX.to_string();
        assert!(matches!(
            relative_offset_um(&cell_origin_address(), &far),
            Err(CelestialError::AddressOverflow)
        ));
    }

    #[test]
    fn local_pose_is_micrometre_quantized_and_origin_rebase_invariant() {
        let origin = cell_origin_address();
        let requested = Vec3::new(12.345_678_49, -9_999.999_999_6, 0.000_000_51);
        let address =
            address_from_local_position(&origin, requested).expect("bounded pose canonicalizes");
        let hydrated =
            local_position_from_address(&origin, &address).expect("canonical address hydrates");
        for (actual, expected) in [
            (hydrated.x, requested.x),
            (hydrated.y, requested.y),
            (hydrated.z, requested.z),
        ] {
            assert!((actual - expected).abs() <= 0.000_000_5);
        }

        let rebased_origin = address_from_origin_offset_um(
            &origin,
            [
                i128::from(CELL_EDGE_UM) * 3,
                -i128::from(CELL_EDGE_UM) * 2,
                0,
            ],
        )
        .expect("rebase origin canonicalizes");
        let rebased_offset =
            relative_offset_um(&rebased_origin, &address).expect("rebased offset fits");
        assert_eq!(
            address_from_origin_offset_um(&rebased_origin, rebased_offset),
            Ok(address),
            "rebasing changes only the derived local pose"
        );
    }

    #[test]
    fn local_pose_conversion_rejects_nonfinite_unsafe_and_wrong_universe_inputs() {
        let origin = cell_origin_address();
        assert_eq!(
            address_from_local_position(&origin, Vec3::new(f64::NAN, 0.0, 0.0)),
            Err(CelestialError::NonFinitePosition)
        );
        assert_eq!(
            address_from_local_position(&origin, Vec3::new(10_000_000_000.0, 0.0, 0.0)),
            Err(CelestialError::UnsafeDerivedPosition)
        );
        let mut wrong_universe = origin.clone();
        wrong_universe.universe_id = "another-universe".into();
        assert!(matches!(
            relative_offset_um(&origin, &wrong_universe),
            Err(CelestialError::InvalidAddress(_))
        ));
    }
}
