// SPDX-License-Identifier: Apache-2.0

//! Independent validation of immutable universe-manifest and celestial-registry
//! commitments. This module intentionally depends only on public protocol
//! values and the portable canonical serializer.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use verse_protocol::{
    CELESTIAL_REGISTRY_SCHEMA_VERSION, CELL_DIRECTORY_SCHEMA_VERSION, CELL_KEY_SCHEMA_VERSION,
    CelestialBodyKind, CelestialBodySnapshot, CelestialRegistrySnapshot, CelestialScaleClass,
    EnvironmentSnapshot, INTENT_FINGERPRINT_SCHEMA_VERSION, INTEREST_SCHEMA_VERSION,
    LIFECYCLE_CONTROL_SCHEMA_VERSION, PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
    PROJECTION_SCHEMA_VERSION, TRANSFER_PACKAGE_SCHEMA_VERSION, UNIVERSE_MANIFEST_SCHEMA_VERSION,
    UniverseAddress, UniverseManifestSnapshot,
};

use crate::canonical;
use crate::error::{ErrorCode, Result, VerifyError};

const ADDRESS_SCHEMA_VERSION: u32 = 1;
const REGISTRY_DOMAIN: &[u8] = b"the-verse/celestial-registry/v1\0";
const MANIFEST_DOMAIN: &[u8] = b"the-verse/universe-manifest/v4\0";

#[derive(Debug, Clone, Copy)]
pub(crate) struct AddressDimensions {
    sector_edge_um: u64,
    cell_edge_um: u64,
    cells_per_sector_axis: u32,
}

#[derive(Debug, Clone)]
struct RegisteredBody {
    display_name: String,
    scale_class: CelestialScaleClass,
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedRegistry {
    pub(crate) universe_id: String,
    pub(crate) registry_hash: String,
    pub(crate) universe_manifest_hash: String,
    dimensions: AddressDimensions,
    bodies: BTreeMap<String, RegisteredBody>,
}

impl ValidatedRegistry {
    pub(crate) fn validate_address(&self, address: &UniverseAddress, label: &str) -> Result<()> {
        validate_address(address, &self.universe_id, self.dimensions, label)
    }

    pub(crate) fn require_body(&self, body_id: &str, label: &str) -> Result<()> {
        if !self.bodies.contains_key(body_id) {
            return Err(VerifyError::new(
                ErrorCode::BindingMismatch,
                format!("{label} does not resolve to the committed celestial registry"),
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_environment(
        &self,
        environment: &EnvironmentSnapshot,
        label: &str,
    ) -> Result<()> {
        let body = self
            .bodies
            .get(&environment.celestial_body_id)
            .ok_or_else(|| {
                VerifyError::new(
                    ErrorCode::BindingMismatch,
                    format!("{label} celestial body does not resolve to the registry"),
                )
            })?;
        if body.display_name != environment.celestial_body_name
            || body.scale_class != environment.celestial_scale_class
        {
            return Err(VerifyError::new(
                ErrorCode::BindingMismatch,
                format!("{label} celestial body metadata disagrees with the registry"),
            ));
        }
        let nearest = self
            .bodies
            .get(&environment.nearest_body_id)
            .ok_or_else(|| {
                VerifyError::new(
                    ErrorCode::BindingMismatch,
                    format!("{label} nearest body does not resolve to the registry"),
                )
            })?;
        if nearest.display_name != environment.nearest_body_name {
            return Err(VerifyError::new(
                ErrorCode::BindingMismatch,
                format!("{label} nearest body name disagrees with the registry"),
            ));
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn validate_documents(
    expected_world_schema: u32,
    expected_event_schema: u32,
    expected_content_schema: u32,
    expected_content_manifest: &str,
    expected_content_hash: &str,
    expected_universe_id: &str,
    expected_registry_hash: &str,
    expected_manifest_hash: &str,
    max_registry_bodies: usize,
    max_registry_pair_comparisons: usize,
    registry: &CelestialRegistrySnapshot,
    manifest: &UniverseManifestSnapshot,
) -> Result<ValidatedRegistry> {
    validate_hash(&registry.registry_hash, "registry_hash")?;
    validate_hash(&manifest.manifest_hash, "manifest_hash")?;
    validate_hash(
        &manifest.celestial_registry_hash,
        "manifest celestial_registry_hash",
    )?;
    validate_hash(&manifest.content_hash, "manifest content_hash")?;
    validate_expected_commitments(
        expected_universe_id,
        expected_content_hash,
        expected_registry_hash,
        expected_manifest_hash,
    )?;

    if registry.universe_id != expected_universe_id
        || manifest.universe_id != expected_universe_id
        || manifest.content_hash != expected_content_hash
        || registry.registry_hash != expected_registry_hash
        || manifest.celestial_registry_hash != expected_registry_hash
        || manifest.manifest_hash != expected_manifest_hash
    {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "registry frame does not match the client-pinned universe commitments",
        ));
    }

    validate_registry_budget(
        registry.bodies.len(),
        max_registry_bodies,
        max_registry_pair_comparisons,
    )?;

    let dimensions = validate_dimensions(manifest)?;
    let bindings_valid = registry.schema_version == CELESTIAL_REGISTRY_SCHEMA_VERSION
        && registry.license == "CC-BY-SA-4.0"
        && manifest.schema_version == UNIVERSE_MANIFEST_SCHEMA_VERSION
        && manifest.address_schema_version == ADDRESS_SCHEMA_VERSION
        && manifest.world_schema_version == expected_world_schema
        && manifest.event_schema_version == expected_event_schema
        && manifest.projection_schema_version == PROJECTION_SCHEMA_VERSION
        && manifest.interest_schema_version == INTEREST_SCHEMA_VERSION
        && manifest.operation_fingerprint_schema_version == INTENT_FINGERPRINT_SCHEMA_VERSION
        && manifest.cell_key_schema_version == CELL_KEY_SCHEMA_VERSION
        && manifest.cell_directory_schema_version == CELL_DIRECTORY_SCHEMA_VERSION
        && manifest.transfer_package_schema_version == TRANSFER_PACKAGE_SCHEMA_VERSION
        && manifest.lifecycle_control_schema_version == LIFECYCLE_CONTROL_SCHEMA_VERSION
        && manifest.production_schedule_occurrence_schema_version
            == PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION
        && manifest.content_schema_version == expected_content_schema
        && manifest.content_manifest_version == expected_content_manifest
        && manifest.celestial_registry_schema_version == registry.schema_version
        && manifest.celestial_registry_hash == registry.registry_hash
        && manifest.universe_id == registry.universe_id
        && manifest.generation_rule_version == registry.generation_rule_version
        && !registry.universe_id.is_empty()
        && !registry.generation_rule_version.is_empty()
        && !manifest.world_seed.is_empty()
        && !manifest.frontier_policy_version.is_empty();
    if !bindings_valid {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "registry and universe manifest bindings disagree",
        ));
    }

    validate_identifier(&registry.universe_id, "universe_id")?;
    validate_identifier(
        &registry.generation_rule_version,
        "registry generation_rule_version",
    )?;
    validate_identifier(
        &manifest.frontier_policy_version,
        "manifest frontier_policy_version",
    )?;
    validate_identifier(
        &manifest.content_manifest_version,
        "manifest content_manifest_version",
    )?;
    validate_hash(
        &manifest.lifecycle_policy_hash,
        "manifest lifecycle_policy_hash",
    )?;

    let mut bodies = BTreeMap::new();
    let mut centers = BTreeSet::new();
    let mut prior_id: Option<&str> = None;
    for body in &registry.bodies {
        validate_body(body, registry, manifest, dimensions, prior_id, &mut centers)?;
        prior_id = Some(&body.body_id);
        bodies.insert(
            body.body_id.clone(),
            RegisteredBody {
                display_name: body.display_name.clone(),
                scale_class: body.scale_class,
            },
        );
    }
    validate_parents(&registry.bodies)?;
    let field_memberships = validate_field_memberships(&registry.bodies, dimensions)?;
    validate_separation(registry, dimensions, &field_memberships)?;

    let computed_registry_hash = registry_hash(registry)?;
    if registry.registry_hash.as_bytes() != computed_registry_hash.as_bytes() {
        return Err(VerifyError::new(
            ErrorCode::HashMismatch,
            "celestial registry commitment does not match its canonical body",
        ));
    }
    let computed_manifest_hash = manifest_hash(manifest)?;
    if manifest.manifest_hash.as_bytes() != computed_manifest_hash.as_bytes() {
        return Err(VerifyError::new(
            ErrorCode::HashMismatch,
            "universe manifest commitment does not match its canonical body",
        ));
    }

    Ok(ValidatedRegistry {
        universe_id: registry.universe_id.clone(),
        registry_hash: registry.registry_hash.clone(),
        universe_manifest_hash: manifest.manifest_hash.clone(),
        dimensions,
        bodies,
    })
}

pub(crate) fn validate_expected_commitments(
    universe_id: &str,
    content_hash: &str,
    registry_hash: &str,
    manifest_hash: &str,
) -> Result<()> {
    validate_identifier(universe_id, "configured universe_id")?;
    validate_hash(content_hash, "configured content_hash")?;
    validate_hash(registry_hash, "configured celestial_registry_hash")?;
    validate_hash(manifest_hash, "configured universe_manifest_hash")
}

fn validate_registry_budget(
    body_count: usize,
    max_registry_bodies: usize,
    max_registry_pair_comparisons: usize,
) -> Result<()> {
    if body_count == 0 {
        return Err(VerifyError::new(
            ErrorCode::InvalidRegistry,
            "celestial registry must contain at least one body",
        ));
    }
    let pair_comparisons = body_count
        .checked_mul(body_count.saturating_sub(1))
        .and_then(|value| value.checked_div(2))
        .ok_or_else(|| {
            VerifyError::new(
                ErrorCode::ResourceLimit,
                "celestial registry pair-comparison count overflows",
            )
        })?;
    if body_count > max_registry_bodies || pair_comparisons > max_registry_pair_comparisons {
        return Err(VerifyError::new(
            ErrorCode::ResourceLimit,
            "celestial registry exceeds the configured body or pair-comparison budget",
        ));
    }
    Ok(())
}

fn validate_dimensions(manifest: &UniverseManifestSnapshot) -> Result<AddressDimensions> {
    let valid_shape = manifest.sector_edge_um > 0
        && manifest.cell_edge_um > 0
        && manifest.cell_edge_um.is_multiple_of(2)
        && i64::try_from(manifest.cell_edge_um).is_ok()
        && manifest.cells_per_sector_axis > 0;
    let checked_sector_edge = manifest
        .cell_edge_um
        .checked_mul(u64::from(manifest.cells_per_sector_axis));
    if !valid_shape || checked_sector_edge != Some(manifest.sector_edge_um) {
        return Err(VerifyError::new(
            ErrorCode::InvalidAddress,
            "manifest address dimensions are not positive, even, bounded, and multiplicatively consistent",
        ));
    }
    Ok(AddressDimensions {
        sector_edge_um: manifest.sector_edge_um,
        cell_edge_um: manifest.cell_edge_um,
        cells_per_sector_axis: manifest.cells_per_sector_axis,
    })
}

fn validate_body(
    body: &CelestialBodySnapshot,
    registry: &CelestialRegistrySnapshot,
    manifest: &UniverseManifestSnapshot,
    dimensions: AddressDimensions,
    prior_id: Option<&str>,
    centers: &mut BTreeSet<UniverseAddress>,
) -> Result<()> {
    validate_identifier(&body.body_id, "body_id")?;
    if prior_id.is_some_and(|prior| prior.as_bytes() >= body.body_id.as_bytes()) {
        return Err(VerifyError::new(
            ErrorCode::NonCanonicalOrder,
            "celestial bodies are not in strict body_id byte order",
        ));
    }
    validate_address(
        &body.center,
        &registry.universe_id,
        dimensions,
        "body center",
    )?;
    if !centers.insert(body.center.clone()) {
        return Err(VerifyError::new(
            ErrorCode::InvalidRegistry,
            "celestial body centers must be unique",
        ));
    }

    validate_hash(&body.content_hash, "body content_hash")?;
    if body.display_name.trim().is_empty() {
        return Err(VerifyError::new(
            ErrorCode::InvalidRegistry,
            "body display_name must be nonempty",
        ));
    }
    validate_identifier(&body.visual_descriptor_id, "body visual_descriptor_id")?;
    validate_definition_ids_and_kind(body)?;
    if body.content_manifest_version != manifest.content_manifest_version
        || body.content_hash != manifest.content_hash
        || body.generation_rule_version != registry.generation_rule_version
        || body.materialized_registry_version == 0
        || body.generation_seed.is_empty()
    {
        return Err(VerifyError::new(
            ErrorCode::BindingMismatch,
            "celestial body generation or content binding disagrees with its registry and manifest",
        ));
    }
    validate_identifier(&body.generation_seed, "body generation_seed")?;

    let atmosphere_edge = body
        .surface_radius_um
        .checked_add(body.atmosphere_height_um)
        .ok_or_else(|| {
            VerifyError::new(
                ErrorCode::InvalidRegistry,
                "celestial body surface and atmosphere radius overflow",
            )
        })?;
    if body.exclusion_radius_um == 0
        || body.surface_radius_um == 0
        || body.exclusion_radius_um < atmosphere_edge
        || body.oxygen_parts_per_million > 1_000_000
    {
        return Err(VerifyError::new(
            ErrorCode::InvalidRegistry,
            "celestial body radius, atmosphere envelope, or oxygen fraction is invalid",
        ));
    }
    Ok(())
}

fn validate_definition_ids_and_kind(body: &CelestialBodySnapshot) -> Result<()> {
    for (value, label) in [
        (&body.geometry_definition_id, "geometry_definition_id"),
        (&body.material_definition_id, "material_definition_id"),
        (&body.gravity_definition_id, "gravity_definition_id"),
        (&body.atmosphere_definition_id, "atmosphere_definition_id"),
        (&body.resource_definition_id, "resource_definition_id"),
    ] {
        validate_identifier(value, label)?;
    }
    if let Some(voxel) = body.voxel_definition_id.as_deref() {
        validate_identifier(voxel, "voxel_definition_id")?;
    }
    if let Some(voxel_field) = body.voxel_field_id.as_deref() {
        validate_identifier(voxel_field, "voxel_field_id")?;
    }

    let definitions_match_kind = match body.kind {
        CelestialBodyKind::Planet => {
            body.voxel_definition_id.is_none()
                && body.voxel_field_id.is_none()
                && body.field_id.is_none()
                && body.surface_gravity_millimetres_per_second_squared > 0
        }
        CelestialBodyKind::Moon => {
            body.voxel_definition_id.is_none()
                && body.voxel_field_id.is_none()
                && body.field_id.is_none()
        }
        CelestialBodyKind::Asteroid => {
            body.parent_body_id.is_none()
                && body.voxel_definition_id.is_some()
                && body.voxel_field_id.is_some()
        }
        CelestialBodyKind::AsteroidField => {
            body.parent_body_id.is_none()
                && body.field_id.as_deref() == Some(body.body_id.as_str())
                && body.voxel_definition_id.is_none()
                && body.voxel_field_id.is_none()
        }
    };
    if !definitions_match_kind {
        return Err(VerifyError::new(
            ErrorCode::InvalidRegistry,
            "celestial body definitions or physical environment do not match its schema-1 kind",
        ));
    }
    Ok(())
}

fn validate_parents(bodies: &[CelestialBodySnapshot]) -> Result<()> {
    let by_id: BTreeMap<&str, &CelestialBodySnapshot> = bodies
        .iter()
        .map(|body| (body.body_id.as_str(), body))
        .collect();
    for body in bodies {
        match (body.kind, body.parent_body_id.as_deref()) {
            (CelestialBodyKind::Moon, None) => {
                return Err(invalid_parent("a moon must name a planet parent"));
            }
            (CelestialBodyKind::Planet, Some(_)) => {
                return Err(invalid_parent("a planet cannot name a parent"));
            }
            _ => {}
        }
        if let Some(parent_id) = body.parent_body_id.as_deref() {
            validate_identifier(parent_id, "parent_body_id")?;
            if parent_id == body.body_id {
                return Err(invalid_parent("a body cannot parent itself"));
            }
            let parent = by_id
                .get(parent_id)
                .ok_or_else(|| invalid_parent("a body parent is absent from the registry"))?;
            if body.kind == CelestialBodyKind::Moon && parent.kind != CelestialBodyKind::Planet {
                return Err(invalid_parent("a moon parent is not a planet"));
            }
        }

        let mut visited = BTreeSet::new();
        let mut cursor = Some(body.body_id.as_str());
        while let Some(id) = cursor {
            if !visited.insert(id) {
                return Err(invalid_parent("celestial parent ancestry contains a cycle"));
            }
            cursor = by_id
                .get(id)
                .and_then(|candidate| candidate.parent_body_id.as_deref());
        }
    }
    Ok(())
}

fn invalid_parent(detail: &str) -> VerifyError {
    VerifyError::new(ErrorCode::InvalidRegistry, detail)
}

fn validate_field_memberships(
    bodies: &[CelestialBodySnapshot],
    dimensions: AddressDimensions,
) -> Result<BTreeSet<(String, String)>> {
    let by_id: BTreeMap<&str, &CelestialBodySnapshot> = bodies
        .iter()
        .map(|body| (body.body_id.as_str(), body))
        .collect();
    let mut memberships = BTreeSet::new();
    for body in bodies {
        match body.kind {
            CelestialBodyKind::AsteroidField => {
                if body
                    .field_id
                    .as_deref()
                    .is_some_and(|field_id| field_id != body.body_id)
                {
                    return Err(VerifyError::new(
                        ErrorCode::InvalidRegistry,
                        "an asteroid-field field_id must equal its body_id",
                    ));
                }
            }
            CelestialBodyKind::Asteroid => {
                let Some(field_id) = body.field_id.as_deref() else {
                    continue;
                };
                validate_identifier(field_id, "asteroid field_id")?;
                let field = by_id.get(field_id).ok_or_else(|| {
                    VerifyError::new(
                        ErrorCode::InvalidRegistry,
                        "an asteroid field_id does not resolve to a registry body",
                    )
                })?;
                if field.kind != CelestialBodyKind::AsteroidField
                    || field
                        .field_id
                        .as_deref()
                        .is_some_and(|own_id| own_id != field.body_id)
                {
                    return Err(VerifyError::new(
                        ErrorCode::InvalidRegistry,
                        "an asteroid field_id does not resolve unambiguously to its named field",
                    ));
                }
                validate_field_containment(body, field, dimensions)?;
                memberships.insert((body.body_id.clone(), field.body_id.clone()));
            }
            CelestialBodyKind::Planet | CelestialBodyKind::Moon => {
                if body.field_id.is_some() {
                    return Err(VerifyError::new(
                        ErrorCode::InvalidRegistry,
                        "only asteroids and asteroid fields may carry a field_id",
                    ));
                }
            }
        }
    }
    Ok(memberships)
}

fn validate_field_containment(
    member: &CelestialBodySnapshot,
    field: &CelestialBodySnapshot,
    dimensions: AddressDimensions,
) -> Result<()> {
    let available_radius = field
        .exclusion_radius_um
        .checked_sub(member.exclusion_radius_um)
        .ok_or_else(|| {
            VerifyError::new(
                ErrorCode::InvalidRegistry,
                "asteroid member exclusion radius exceeds its field radius",
            )
        })?;
    let available_squared = u128::from(available_radius)
        .checked_mul(u128::from(available_radius))
        .ok_or_else(address_distance_overflow)?;
    let distance_squared = address_distance_squared(&member.center, &field.center, dimensions)?;
    if distance_squared > available_squared {
        return Err(VerifyError::new(
            ErrorCode::InvalidRegistry,
            "asteroid member exclusion volume is not contained by its field",
        ));
    }
    Ok(())
}

fn validate_separation(
    registry: &CelestialRegistrySnapshot,
    dimensions: AddressDimensions,
    field_memberships: &BTreeSet<(String, String)>,
) -> Result<()> {
    for (index, left) in registry.bodies.iter().enumerate() {
        for right in registry.bodies.iter().skip(index + 1) {
            let member_pair = field_memberships
                .contains(&(left.body_id.clone(), right.body_id.clone()))
                || field_memberships.contains(&(right.body_id.clone(), left.body_id.clone()));
            if member_pair {
                continue;
            }
            let distance_squared =
                address_distance_squared(&left.center, &right.center, dimensions)?;
            let required = u128::from(left.exclusion_radius_um)
                .checked_add(u128::from(right.exclusion_radius_um))
                .and_then(|sum| {
                    sum.checked_add(u128::from(registry.minimum_fixed_body_surface_gap_um))
                })
                .ok_or_else(|| {
                    VerifyError::new(
                        ErrorCode::InvalidRegistry,
                        "celestial separation threshold overflow",
                    )
                })?;
            let required_squared = required.checked_mul(required).ok_or_else(|| {
                VerifyError::new(
                    ErrorCode::InvalidRegistry,
                    "celestial separation square overflow",
                )
            })?;
            if distance_squared < required_squared {
                return Err(VerifyError::new(
                    ErrorCode::InvalidRegistry,
                    format!(
                        "celestial exclusion volumes {} and {} violate the minimum fixed-body surface gap",
                        left.body_id, right.body_id
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn address_distance_squared(
    left: &UniverseAddress,
    right: &UniverseAddress,
    dimensions: AddressDimensions,
) -> Result<u128> {
    let left_sector = parse_sector(&left.sector)?;
    let right_sector = parse_sector(&right.sector)?;
    let left_cell = [left.cell.x, left.cell.y, left.cell.z];
    let right_cell = [right.cell.x, right.cell.y, right.cell.z];
    let left_local = [left.local_um.x, left.local_um.y, left.local_um.z];
    let right_local = [right.local_um.x, right.local_um.y, right.local_um.z];
    let mut sum = 0_u128;
    for axis in 0..3 {
        let sector_delta = left_sector[axis]
            .checked_sub(right_sector[axis])
            .and_then(|value| value.checked_mul(i128::from(dimensions.sector_edge_um)))
            .ok_or_else(address_distance_overflow)?;
        let cell_delta = i128::from(left_cell[axis])
            .checked_sub(i128::from(right_cell[axis]))
            .and_then(|value| value.checked_mul(i128::from(dimensions.cell_edge_um)))
            .ok_or_else(address_distance_overflow)?;
        let local_delta = i128::from(left_local[axis])
            .checked_sub(i128::from(right_local[axis]))
            .ok_or_else(address_distance_overflow)?;
        let delta = sector_delta
            .checked_add(cell_delta)
            .and_then(|value| value.checked_add(local_delta))
            .ok_or_else(address_distance_overflow)?;
        let magnitude = delta.unsigned_abs();
        let square = magnitude
            .checked_mul(magnitude)
            .ok_or_else(address_distance_overflow)?;
        sum = sum
            .checked_add(square)
            .ok_or_else(address_distance_overflow)?;
    }
    Ok(sum)
}

fn address_distance_overflow() -> VerifyError {
    VerifyError::new(
        ErrorCode::InvalidAddress,
        "checked celestial address distance overflow",
    )
}

fn validate_address(
    address: &UniverseAddress,
    universe_id: &str,
    dimensions: AddressDimensions,
    label: &str,
) -> Result<()> {
    if address.universe_id != universe_id {
        return Err(VerifyError::new(
            ErrorCode::InvalidAddress,
            format!("{label} names the wrong universe"),
        ));
    }
    parse_sector(&address.sector)
        .map_err(|error| VerifyError::new(error.code(), format!("{label}: {}", error.detail())))?;
    let cells = [address.cell.x, address.cell.y, address.cell.z];
    if cells
        .iter()
        .any(|value| *value >= dimensions.cells_per_sector_axis)
    {
        return Err(VerifyError::new(
            ErrorCode::InvalidAddress,
            format!("{label} cell index is outside the manifest dimensions"),
        ));
    }
    let half = i64::try_from(dimensions.cell_edge_um / 2).map_err(|_| {
        VerifyError::new(
            ErrorCode::InvalidAddress,
            "manifest cell half-edge does not fit a local coordinate",
        )
    })?;
    let locals = [address.local_um.x, address.local_um.y, address.local_um.z];
    if locals.iter().any(|value| *value < -half || *value >= half) {
        return Err(VerifyError::new(
            ErrorCode::InvalidAddress,
            format!("{label} local coordinate is not normalized"),
        ));
    }
    Ok(())
}

fn parse_sector(sector: &verse_protocol::SectorCoordinate) -> Result<[i128; 3]> {
    Ok([
        parse_canonical_i128(&sector.x)?,
        parse_canonical_i128(&sector.y)?,
        parse_canonical_i128(&sector.z)?,
    ])
}

fn parse_canonical_i128(value: &str) -> Result<i128> {
    let bytes = value.as_bytes();
    let canonical_shape = match bytes {
        [b'0'] => true,
        [b'1'..=b'9', rest @ ..] | [b'-', b'1'..=b'9', rest @ ..] => {
            rest.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    };
    if !canonical_shape {
        return Err(VerifyError::new(
            ErrorCode::InvalidAddress,
            "sector coordinate is not a canonical signed decimal integer",
        ));
    }
    value.parse::<i128>().map_err(|_| {
        VerifyError::new(
            ErrorCode::InvalidAddress,
            "sector coordinate is outside the signed 128-bit range",
        )
    })
}

fn validate_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(VerifyError::new(
            ErrorCode::InvalidRegistry,
            format!("{label} is not a nonempty schema-bounded ASCII identifier"),
        ));
    }
    Ok(())
}

fn validate_hash(value: &str, label: &str) -> Result<()> {
    let valid = value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !valid {
        return Err(VerifyError::new(
            ErrorCode::HashMismatch,
            format!("{label} is not a 64-character lowercase hexadecimal digest"),
        ));
    }
    Ok(())
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

pub(crate) fn registry_hash(registry: &CelestialRegistrySnapshot) -> Result<String> {
    domain_digest(
        REGISTRY_DOMAIN,
        &RegistryHashMaterial {
            schema_version: registry.schema_version,
            license: &registry.license,
            universe_id: &registry.universe_id,
            generation_rule_version: &registry.generation_rule_version,
            minimum_fixed_body_surface_gap_um: registry.minimum_fixed_body_surface_gap_um,
            bodies: &registry.bodies,
        },
    )
}

#[derive(Serialize)]
struct ManifestHashMaterial<'a> {
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

pub(crate) fn manifest_hash(manifest: &UniverseManifestSnapshot) -> Result<String> {
    domain_digest(
        MANIFEST_DOMAIN,
        &ManifestHashMaterial {
            schema_version: manifest.schema_version,
            universe_id: &manifest.universe_id,
            world_seed: &manifest.world_seed,
            address_schema_version: manifest.address_schema_version,
            sector_edge_um: manifest.sector_edge_um,
            cell_edge_um: manifest.cell_edge_um,
            cells_per_sector_axis: manifest.cells_per_sector_axis,
            generation_rule_version: &manifest.generation_rule_version,
            frontier_policy_version: &manifest.frontier_policy_version,
            celestial_registry_schema_version: manifest.celestial_registry_schema_version,
            celestial_registry_hash: &manifest.celestial_registry_hash,
            content_schema_version: manifest.content_schema_version,
            content_manifest_version: &manifest.content_manifest_version,
            content_hash: &manifest.content_hash,
            world_schema_version: manifest.world_schema_version,
            event_schema_version: manifest.event_schema_version,
            projection_schema_version: manifest.projection_schema_version,
            interest_schema_version: manifest.interest_schema_version,
            operation_fingerprint_schema_version: manifest.operation_fingerprint_schema_version,
            cell_key_schema_version: manifest.cell_key_schema_version,
            cell_directory_schema_version: manifest.cell_directory_schema_version,
            transfer_package_schema_version: manifest.transfer_package_schema_version,
            lifecycle_control_schema_version: manifest.lifecycle_control_schema_version,
            production_schedule_occurrence_schema_version: manifest
                .production_schedule_occurrence_schema_version,
            lifecycle_policy_hash: &manifest.lifecycle_policy_hash,
        },
    )
}

fn domain_digest<T: Serialize>(domain: &[u8], material: &T) -> Result<String> {
    let canonical = canonical::fixed_json(material)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(&canonical);
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use verse_protocol::{CellCoordinate, I64Vec3, SectorCoordinate};

    use super::*;

    const CONTENT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn address(x: &str, local_x: i64) -> UniverseAddress {
        UniverseAddress {
            universe_id: "universe-test".into(),
            sector: SectorCoordinate {
                x: x.into(),
                y: "0".into(),
                z: "0".into(),
            },
            cell: CellCoordinate { x: 0, y: 0, z: 0 },
            local_um: I64Vec3 {
                x: local_x,
                y: 0,
                z: 0,
            },
        }
    }

    fn body(id: &str, kind: CelestialBodyKind, center: UniverseAddress) -> CelestialBodySnapshot {
        let (field_id, voxel_field_id, voxel_definition_id, surface_gravity) = match kind {
            CelestialBodyKind::Planet => (None, None, None, 1),
            CelestialBodyKind::Moon => (None, None, None, 0),
            CelestialBodyKind::Asteroid => (
                None,
                Some(format!("voxel-field-{id}")),
                Some("voxel-definition".into()),
                0,
            ),
            CelestialBodyKind::AsteroidField => (Some(id.into()), None, None, 0),
        };
        CelestialBodySnapshot {
            body_id: id.into(),
            display_name: id.into(),
            kind,
            parent_body_id: None,
            field_id,
            center,
            surface_radius_um: 10,
            exclusion_radius_um: 10,
            fixed_orientation_microradians: I64Vec3::ZERO,
            surface_gravity_millimetres_per_second_squared: surface_gravity,
            atmosphere_height_um: 0,
            oxygen_parts_per_million: 0,
            voxel_field_id,
            geometry_definition_id: "geometry".into(),
            voxel_definition_id,
            material_definition_id: "material".into(),
            gravity_definition_id: "gravity".into(),
            atmosphere_definition_id: "atmosphere".into(),
            resource_definition_id: "resource".into(),
            visual_descriptor_id: "visual".into(),
            scale_class: CelestialScaleClass::Proof,
            generation_seed: "seed".into(),
            generation_rule_version: "generation-v1".into(),
            materialized_registry_version: 1,
            content_manifest_version: "content-v1".into(),
            content_hash: CONTENT_HASH.into(),
        }
    }

    fn documents(
        bodies: Vec<CelestialBodySnapshot>,
        gap: u64,
    ) -> (CelestialRegistrySnapshot, UniverseManifestSnapshot) {
        let mut registry = CelestialRegistrySnapshot {
            schema_version: 1,
            registry_hash: String::new(),
            license: "CC-BY-SA-4.0".into(),
            universe_id: "universe-test".into(),
            generation_rule_version: "generation-v1".into(),
            minimum_fixed_body_surface_gap_um: gap,
            bodies,
        };
        registry.registry_hash = registry_hash(&registry).expect("registry hashes");
        let mut manifest = UniverseManifestSnapshot {
            schema_version: UNIVERSE_MANIFEST_SCHEMA_VERSION,
            manifest_hash: String::new(),
            universe_id: "universe-test".into(),
            world_seed: "seed".into(),
            address_schema_version: 1,
            sector_edge_um: 100,
            cell_edge_um: 100,
            cells_per_sector_axis: 1,
            generation_rule_version: "generation-v1".into(),
            frontier_policy_version: "frontier-v1".into(),
            celestial_registry_schema_version: 1,
            celestial_registry_hash: registry.registry_hash.clone(),
            content_schema_version: 13,
            content_manifest_version: "content-v1".into(),
            content_hash: CONTENT_HASH.into(),
            world_schema_version: 11,
            event_schema_version: 12,
            projection_schema_version: PROJECTION_SCHEMA_VERSION,
            interest_schema_version: verse_protocol::INTEREST_SCHEMA_VERSION,
            operation_fingerprint_schema_version: verse_protocol::INTENT_FINGERPRINT_SCHEMA_VERSION,
            cell_key_schema_version: verse_protocol::CELL_KEY_SCHEMA_VERSION,
            cell_directory_schema_version: verse_protocol::CELL_DIRECTORY_SCHEMA_VERSION,
            transfer_package_schema_version: verse_protocol::TRANSFER_PACKAGE_SCHEMA_VERSION,
            lifecycle_control_schema_version: LIFECYCLE_CONTROL_SCHEMA_VERSION,
            production_schedule_occurrence_schema_version:
                PRODUCTION_SCHEDULE_OCCURRENCE_SCHEMA_VERSION,
            lifecycle_policy_hash: CONTENT_HASH.into(),
        };
        manifest.manifest_hash = manifest_hash(&manifest).expect("manifest hashes");
        (registry, manifest)
    }

    fn validate(
        registry: &CelestialRegistrySnapshot,
        manifest: &UniverseManifestSnapshot,
    ) -> Result<ValidatedRegistry> {
        validate_documents(
            11,
            12,
            13,
            "content-v1",
            &manifest.content_hash,
            &manifest.universe_id,
            &registry.registry_hash,
            &manifest.manifest_hash,
            512,
            130_816,
            registry,
            manifest,
        )
    }

    fn validate_with_expected(
        registry: &CelestialRegistrySnapshot,
        manifest: &UniverseManifestSnapshot,
        expected_content_hash: &str,
        expected_universe_id: &str,
        expected_registry_hash: &str,
        expected_manifest_hash: &str,
    ) -> Result<ValidatedRegistry> {
        validate_documents(
            11,
            12,
            13,
            "content-v1",
            expected_content_hash,
            expected_universe_id,
            expected_registry_hash,
            expected_manifest_hash,
            512,
            130_816,
            registry,
            manifest,
        )
    }

    fn refresh_hashes(
        registry: &mut CelestialRegistrySnapshot,
        manifest: &mut UniverseManifestSnapshot,
    ) {
        registry.registry_hash = registry_hash(registry).expect("registry rehashes");
        manifest
            .celestial_registry_hash
            .clone_from(&registry.registry_hash);
        manifest.manifest_hash = manifest_hash(manifest).expect("manifest rehashes");
    }

    #[test]
    fn canonical_i128_parser_rejects_aliases_and_overflow() {
        for valid in [
            "0",
            "1",
            "-1",
            "170141183460469231731687303715884105727",
            "-170141183460469231731687303715884105728",
        ] {
            assert!(parse_canonical_i128(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            "+0",
            "+1",
            "-0",
            "00",
            "01",
            "-01",
            " 1",
            "1 ",
            "170141183460469231731687303715884105728",
            "-170141183460469231731687303715884105729",
        ] {
            assert_eq!(
                parse_canonical_i128(invalid)
                    .expect_err("alias or overflow is rejected")
                    .code(),
                ErrorCode::InvalidAddress,
                "{invalid}"
            );
        }
    }

    #[test]
    fn client_pins_reject_self_consistent_universe_content_and_definition_substitution() {
        let (registry, manifest) = documents(
            vec![body("body", CelestialBodyKind::Planet, address("0", 0))],
            0,
        );
        let expected_registry_hash = registry.registry_hash.clone();
        let expected_manifest_hash = manifest.manifest_hash.clone();

        let mut substituted_registry = registry.clone();
        let mut substituted_manifest = manifest.clone();
        substituted_registry.universe_id = "substituted-universe".into();
        substituted_registry.bodies[0].center.universe_id = "substituted-universe".into();
        substituted_manifest.universe_id = "substituted-universe".into();
        refresh_hashes(&mut substituted_registry, &mut substituted_manifest);
        assert_eq!(
            validate_with_expected(
                &substituted_registry,
                &substituted_manifest,
                CONTENT_HASH,
                "universe-test",
                &expected_registry_hash,
                &expected_manifest_hash,
            )
            .expect_err("self-consistent universe substitution is rejected")
            .code(),
            ErrorCode::BindingMismatch
        );

        let mut substituted_registry = registry.clone();
        let mut substituted_manifest = manifest.clone();
        substituted_registry.bodies[0].content_hash = "b".repeat(64);
        substituted_manifest.content_hash = "b".repeat(64);
        refresh_hashes(&mut substituted_registry, &mut substituted_manifest);
        assert_eq!(
            validate_with_expected(
                &substituted_registry,
                &substituted_manifest,
                CONTENT_HASH,
                "universe-test",
                &expected_registry_hash,
                &expected_manifest_hash,
            )
            .expect_err("self-consistent content substitution is rejected")
            .code(),
            ErrorCode::BindingMismatch
        );

        let mut substituted_registry = registry;
        let mut substituted_manifest = manifest;
        substituted_registry.bodies[0].geometry_definition_id = "unknown-surface-v9".into();
        substituted_registry.bodies[0].generation_seed = "substituted-seed".into();
        refresh_hashes(&mut substituted_registry, &mut substituted_manifest);
        assert_eq!(
            validate_with_expected(
                &substituted_registry,
                &substituted_manifest,
                CONTENT_HASH,
                "universe-test",
                &expected_registry_hash,
                &expected_manifest_hash,
            )
            .expect_err("definition and seed substitution cannot choose new roots")
            .code(),
            ErrorCode::BindingMismatch
        );
    }

    #[test]
    fn registry_budget_is_checked_before_pairwise_separation() {
        assert!(validate_registry_budget(512, 512, 130_816).is_ok());
        assert_eq!(
            validate_registry_budget(513, 512, 130_816)
                .expect_err("body budget is bounded")
                .code(),
            ErrorCode::ResourceLimit
        );
        assert_eq!(
            validate_registry_budget(3, 512, 2)
                .expect_err("pair-comparison budget is bounded")
                .code(),
            ErrorCode::ResourceLimit
        );
    }

    #[test]
    fn schema_one_definition_fields_and_body_kind_matrix_fail_closed() {
        let mut invalid_definition = body("body", CelestialBodyKind::Planet, address("0", 0));
        invalid_definition.geometry_definition_id.clear();
        let (registry, manifest) = documents(vec![invalid_definition], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("empty definition ID is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut oversized_definition = body("body", CelestialBodyKind::Planet, address("0", 0));
        oversized_definition.geometry_definition_id = "g".repeat(129);
        let (registry, manifest) = documents(vec![oversized_definition], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("oversized definition ID is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut mismatched_surface = body("body", CelestialBodyKind::Planet, address("0", 0));
        mismatched_surface.voxel_field_id = Some("voxel-field".into());
        mismatched_surface.voxel_definition_id = Some("voxel-definition".into());
        let (registry, manifest) = documents(vec![mismatched_surface], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("planet cannot use voxel-body surface definitions")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut mismatched_generation = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        mismatched_generation.generation_rule_version = "different-generation".into();
        let (registry, manifest) = documents(vec![mismatched_generation], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("body generation definition must match registry")
                .code(),
            ErrorCode::BindingMismatch
        );
    }

    #[test]
    fn dimensions_and_normalized_address_boundaries_are_independent() {
        let (registry, mut manifest) = documents(
            vec![body("body", CelestialBodyKind::Planet, address("0", 0))],
            0,
        );
        assert!(validate(&registry, &manifest).is_ok());
        for mutate in [
            |value: &mut UniverseManifestSnapshot| value.cell_edge_um = 0,
            |value: &mut UniverseManifestSnapshot| value.cell_edge_um = 99,
            |value: &mut UniverseManifestSnapshot| value.cells_per_sector_axis = 0,
            |value: &mut UniverseManifestSnapshot| value.sector_edge_um = 101,
        ] {
            let mut invalid = manifest.clone();
            mutate(&mut invalid);
            invalid.manifest_hash = manifest_hash(&invalid).expect("invalid manifest hashes");
            assert_eq!(
                validate(&registry, &invalid)
                    .expect_err("invalid dimensions are rejected")
                    .code(),
                ErrorCode::InvalidAddress
            );
        }

        let validated = validate(&registry, &manifest).expect("registry validates");
        assert!(
            validated
                .validate_address(&address("0", -50), "lower boundary")
                .is_ok()
        );
        assert!(
            validated
                .validate_address(&address("0", 49), "upper interior")
                .is_ok()
        );
        assert_eq!(
            validated
                .validate_address(&address("0", 50), "upper boundary")
                .expect_err("half-open upper boundary is rejected")
                .code(),
            ErrorCode::InvalidAddress
        );
        manifest.sector_edge_um = 200;
        manifest.manifest_hash = manifest_hash(&manifest).expect("manifest hashes");
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("dimension product mismatch is rejected")
                .code(),
            ErrorCode::InvalidAddress
        );
    }

    #[test]
    fn commitments_are_exact_lowercase_and_cover_every_top_level_field() {
        let (mut registry, mut manifest) = documents(
            vec![body("body", CelestialBodyKind::Planet, address("0", 0))],
            0,
        );
        assert!(validate(&registry, &manifest).is_ok());
        let mut wrong_license = registry.clone();
        let mut wrong_license_manifest = manifest.clone();
        wrong_license.license = "CC0-1.0".into();
        refresh_hashes(&mut wrong_license, &mut wrong_license_manifest);
        assert_eq!(
            validate(&wrong_license, &wrong_license_manifest)
                .expect_err("schema-1 registry license is pinned")
                .code(),
            ErrorCode::BindingMismatch
        );
        registry.license.push_str("-changed");
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("unlicensed registry mutation is rejected")
                .code(),
            ErrorCode::BindingMismatch
        );
        registry.license = "CC-BY-SA-4.0".into();
        registry.registry_hash = registry.registry_hash.to_uppercase();
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("uppercase commitment is rejected")
                .code(),
            ErrorCode::HashMismatch
        );

        let (registry, original_manifest) = documents(
            vec![body("body", CelestialBodyKind::Planet, address("0", 0))],
            0,
        );
        manifest = original_manifest;
        manifest.world_seed.push_str("-changed");
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("unrehashable manifest mutation is rejected")
                .code(),
            ErrorCode::HashMismatch
        );
    }

    #[test]
    fn registry_requires_order_unique_centers_and_valid_parent_graph() {
        let planet = body("planet", CelestialBodyKind::Planet, address("0", 0));
        let mut moon = body("moon", CelestialBodyKind::Moon, address("1", 0));
        moon.parent_body_id = Some("planet".into());
        let (mut registry, mut manifest) = documents(vec![planet.clone(), moon.clone()], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("wrong body order is rejected")
                .code(),
            ErrorCode::NonCanonicalOrder
        );

        registry.bodies = vec![moon, planet];
        registry
            .bodies
            .sort_by(|left, right| left.body_id.cmp(&right.body_id));
        refresh_hashes(&mut registry, &mut manifest);
        assert!(validate(&registry, &manifest).is_ok());

        registry.bodies[1].center = registry.bodies[0].center.clone();
        refresh_hashes(&mut registry, &mut manifest);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("duplicate centers are rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut left = body("a", CelestialBodyKind::Asteroid, address("0", 0));
        let mut right = body("b", CelestialBodyKind::Asteroid, address("1", 0));
        left.parent_body_id = Some("b".into());
        right.parent_body_id = Some("a".into());
        let (registry, manifest) = documents(vec![left, right], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("parent cycle is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );
    }

    #[test]
    fn parent_rules_reject_every_invalid_shape() {
        let cases = [
            vec![body("moon", CelestialBodyKind::Moon, address("0", 0))],
            {
                let mut planet = body("planet", CelestialBodyKind::Planet, address("0", 0));
                planet.parent_body_id = Some("missing".into());
                vec![planet]
            },
            {
                let mut moon = body("moon", CelestialBodyKind::Moon, address("0", 0));
                moon.parent_body_id = Some("missing".into());
                vec![moon]
            },
            {
                let asteroid = body("asteroid", CelestialBodyKind::Asteroid, address("0", 0));
                let mut moon = body("moon", CelestialBodyKind::Moon, address("1", 0));
                moon.parent_body_id = Some("asteroid".into());
                vec![asteroid, moon]
            },
            {
                let mut asteroid = body("asteroid", CelestialBodyKind::Asteroid, address("0", 0));
                asteroid.parent_body_id = Some("asteroid".into());
                vec![asteroid]
            },
        ];
        for bodies in cases {
            let (registry, manifest) = documents(bodies, 0);
            assert_eq!(
                validate(&registry, &manifest)
                    .expect_err("invalid parent shape is rejected")
                    .code(),
                ErrorCode::InvalidRegistry
            );
        }
    }

    #[test]
    fn body_radius_generation_and_content_invariants_fail_closed() {
        let mut invalid_bodies = Vec::new();
        let mut no_surface = body("body", CelestialBodyKind::Planet, address("0", 0));
        no_surface.surface_radius_um = 0;
        invalid_bodies.push((no_surface, ErrorCode::InvalidRegistry));
        let mut no_exclusion = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        no_exclusion.exclusion_radius_um = 0;
        invalid_bodies.push((no_exclusion, ErrorCode::InvalidRegistry));
        let mut uncovered_atmosphere = body("body", CelestialBodyKind::Planet, address("0", 0));
        uncovered_atmosphere.atmosphere_height_um = 1;
        invalid_bodies.push((uncovered_atmosphere, ErrorCode::InvalidRegistry));
        let mut bad_oxygen = body("body", CelestialBodyKind::Planet, address("0", 0));
        bad_oxygen.oxygen_parts_per_million = 1_000_001;
        invalid_bodies.push((bad_oxygen, ErrorCode::InvalidRegistry));
        let mut wrong_generation = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        wrong_generation.generation_rule_version = "generation-v2".into();
        invalid_bodies.push((wrong_generation, ErrorCode::BindingMismatch));
        let mut wrong_manifest = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        wrong_manifest.content_manifest_version = "content-v2".into();
        invalid_bodies.push((wrong_manifest, ErrorCode::BindingMismatch));
        let mut wrong_content = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        wrong_content.content_hash = "b".repeat(64);
        invalid_bodies.push((wrong_content, ErrorCode::BindingMismatch));
        let mut bad_content_shape = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        bad_content_shape.content_hash = "A".repeat(64);
        invalid_bodies.push((bad_content_shape, ErrorCode::HashMismatch));
        let mut unversioned = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        unversioned.materialized_registry_version = 0;
        invalid_bodies.push((unversioned, ErrorCode::BindingMismatch));
        let mut unseeded = body("body", CelestialBodyKind::Asteroid, address("0", 0));
        unseeded.generation_seed.clear();
        invalid_bodies.push((unseeded, ErrorCode::BindingMismatch));

        for (body, expected) in invalid_bodies {
            let (registry, manifest) = documents(vec![body], 0);
            assert_eq!(
                validate(&registry, &manifest)
                    .expect_err("invalid celestial body is rejected")
                    .code(),
                expected
            );
        }
    }

    #[test]
    fn separation_accepts_equality_and_rejects_one_micrometre_less() {
        let left = body("a", CelestialBodyKind::Asteroid, address("0", 0));
        let right = body("b", CelestialBodyKind::Asteroid, address("1", 0));
        let (registry, manifest) = documents(vec![left.clone(), right.clone()], 80);
        assert!(validate(&registry, &manifest).is_ok());

        let mut too_close = right;
        too_close.center.sector.x = "0".into();
        too_close.center.local_um.x = 49;
        let mut left_shifted = left;
        left_shifted.center.local_um.x = -50;
        let (registry, manifest) = documents(vec![left_shifted, too_close], 80);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("one micrometre below required distance is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );
    }

    #[test]
    fn asteroid_field_containment_accepts_equality_and_exempts_only_its_member_pair() {
        let mut field = body("field", CelestialBodyKind::AsteroidField, address("0", -50));
        field.field_id = Some("field".into());
        field.exclusion_radius_um = 100;
        let mut member = body("member", CelestialBodyKind::Asteroid, address("0", 40));
        member.field_id = Some("field".into());
        let (registry, manifest) = documents(vec![field.clone(), member.clone()], u64::MAX);
        assert!(
            validate(&registry, &manifest).is_ok(),
            "exact containment is valid even though the member and field exclusion volumes overlap"
        );

        member.center.local_um.x = 41;
        let (registry, manifest) = documents(vec![field.clone(), member], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("one micrometre outside field containment is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut external = body("external", CelestialBodyKind::Asteroid, address("0", 49));
        external.exclusion_radius_um = 1;
        let (registry, manifest) = documents(vec![external, field], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("an unrelated asteroid is not exempt from field separation")
                .code(),
            ErrorCode::InvalidRegistry
        );
    }

    #[test]
    fn asteroid_field_membership_rejects_missing_ambiguous_and_overflowing_relations() {
        let field = body("field", CelestialBodyKind::AsteroidField, address("0", 0));
        let mut member = body("member", CelestialBodyKind::Asteroid, address("1", 0));
        member.field_id = Some("missing".into());
        let (registry, manifest) = documents(vec![field.clone(), member], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("missing field reference is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut ambiguous_field = field.clone();
        ambiguous_field.field_id = Some("other".into());
        let (registry, manifest) = documents(vec![ambiguous_field], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("field alias is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut planet = body("planet", CelestialBodyKind::Planet, address("0", 0));
        planet.field_id = Some("field".into());
        let (registry, manifest) = documents(vec![planet], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("planet field_id is rejected")
                .code(),
            ErrorCode::InvalidRegistry
        );

        let mut extreme_field = field;
        extreme_field.center.sector.x = i128::MIN.to_string();
        extreme_field.exclusion_radius_um = u64::MAX;
        let mut extreme_member = body(
            "member",
            CelestialBodyKind::Asteroid,
            address(&i128::MAX.to_string(), 0),
        );
        extreme_member.field_id = Some("field".into());
        let (registry, manifest) = documents(vec![extreme_field, extreme_member], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("field containment distance overflow is rejected")
                .code(),
            ErrorCode::InvalidAddress
        );
    }

    #[test]
    fn checked_address_distance_overflow_fails_closed() {
        let left = body(
            "a",
            CelestialBodyKind::Asteroid,
            address("-170141183460469231731687303715884105728", 0),
        );
        let right = body(
            "b",
            CelestialBodyKind::Asteroid,
            address("170141183460469231731687303715884105727", 0),
        );
        let (registry, manifest) = documents(vec![left, right], 0);
        assert_eq!(
            validate(&registry, &manifest)
                .expect_err("distance overflow is rejected")
                .code(),
            ErrorCode::InvalidAddress
        );
    }
}
