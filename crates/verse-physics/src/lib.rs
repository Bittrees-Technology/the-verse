// SPDX-License-Identifier: AGPL-3.0-or-later

//! Domain-neutral, safe adapter around the pinned Jolt Physics bindings.
//!
//! Jolt performs motion integration and collision response. Its native contact
//! listener supplies authoritative manifold identity and ordered pre-solver
//! telemetry. Jolt's estimated response is not an applied solver impulse and
//! must not drive collision damage.

mod ffi;
mod math;

use std::collections::{BTreeMap, BTreeSet};

pub use math::{Pose, Quat, Vec3};
use thiserror::Error;

const MAX_NATIVE_BODIES: u32 = 262_144;
const MAX_NATIVE_BODY_PAIRS: u32 = 1_048_576;
const MAX_NATIVE_CONTACT_CONSTRAINTS: u32 = 262_144;
const MAX_NATIVE_CONTACT_RECORDS: u64 = 1_048_576;
const MAX_TEMPORARY_ALLOCATOR_BYTES: u32 = 1024 * 1024 * 1024;
const MAX_COLLIDERS_PER_BODY: usize = 262_144;
const MAX_CAPSULE_CAST_DISTANCE_M: f64 = 10_000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMotion {
    Static,
    Dynamic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MotionQuality {
    #[default]
    Discrete,
    /// Sweeps a dynamic body's shape along its linear motion to reduce
    /// tunneling through thin geometry at high speed.
    LinearCast,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxColliderSpec {
    pub collider_id: String,
    pub local_pose: Pose,
    pub half_extents: Vec3,
    pub density_kg_per_m3: f32,
}

impl BoxColliderSpec {
    pub fn unit_cube(collider_id: impl Into<String>) -> Self {
        Self {
            collider_id: collider_id.into(),
            local_pose: Pose::IDENTITY,
            half_extents: Vec3::new(0.5, 0.5, 0.5),
            density_kg_per_m3: 1_000.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SphereColliderSpec {
    pub collider_id: String,
    pub local_pose: Pose,
    pub radius: f32,
    pub density_kg_per_m3: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapsuleColliderSpec {
    pub collider_id: String,
    pub local_pose: Pose,
    /// Radius of both end caps and of the cylinder in meters.
    pub radius: f32,
    /// Half the length of the cylindrical section, excluding the end caps.
    pub half_height_of_cylinder: f32,
    pub density_kg_per_m3: f32,
}

impl CapsuleColliderSpec {
    pub fn new(collider_id: impl Into<String>, radius: f32, half_height_of_cylinder: f32) -> Self {
        Self {
            collider_id: collider_id.into(),
            local_pose: Pose::IDENTITY,
            radius,
            half_height_of_cylinder,
            density_kg_per_m3: 1_000.0,
        }
    }
}

impl SphereColliderSpec {
    pub fn new(collider_id: impl Into<String>, radius: f32) -> Self {
        Self {
            collider_id: collider_id.into(),
            local_pose: Pose::IDENTITY,
            radius,
            density_kg_per_m3: 1_000.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodySpec {
    pub body_id: String,
    pub motion: BodyMotion,
    pub pose: Pose,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub friction: f32,
    pub restitution: f32,
    pub gravity_factor: f32,
    /// Multiplies rotational inertia without changing translational mass.
    pub inertia_multiplier: f32,
    pub allow_sleeping: bool,
    pub motion_quality: MotionQuality,
    /// Box colliders are retained under the original field name so existing
    /// grid and voxel callers remain source-compatible.
    pub colliders: Vec<BoxColliderSpec>,
    pub sphere_colliders: Vec<SphereColliderSpec>,
    pub capsule_colliders: Vec<CapsuleColliderSpec>,
}

impl BodySpec {
    pub fn dynamic(
        body_id: impl Into<String>,
        pose: Pose,
        colliders: Vec<BoxColliderSpec>,
    ) -> Self {
        Self {
            body_id: body_id.into(),
            motion: BodyMotion::Dynamic,
            pose,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            friction: 0.4,
            restitution: 0.0,
            gravity_factor: 0.0,
            inertia_multiplier: 1.0,
            allow_sleeping: true,
            motion_quality: MotionQuality::Discrete,
            colliders,
            sphere_colliders: Vec::new(),
            capsule_colliders: Vec::new(),
        }
    }

    pub fn static_body(
        body_id: impl Into<String>,
        pose: Pose,
        colliders: Vec<BoxColliderSpec>,
    ) -> Self {
        Self {
            body_id: body_id.into(),
            motion: BodyMotion::Static,
            pose,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            friction: 0.4,
            restitution: 0.0,
            gravity_factor: 0.0,
            inertia_multiplier: 1.0,
            allow_sleeping: true,
            motion_quality: MotionQuality::Discrete,
            colliders,
            sphere_colliders: Vec::new(),
            capsule_colliders: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyControl {
    pub body_id: String,
    pub force_newtons: Vec3,
    pub torque_newton_meters: Vec3,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    pub body_id: String,
    pub pose: Pose,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub active: bool,
}

/// A bounded world-space capsule sweep. The capsule's local axis is positive Y
/// and `displacement` is the complete cast vector, not a velocity.
#[derive(Debug, Clone, PartialEq)]
pub struct CapsuleCast {
    pub pose: Pose,
    pub radius: f32,
    pub half_height_of_cylinder: f32,
    pub displacement: Vec3,
    pub ignore_body_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CapsuleCastHit {
    pub body_id: String,
    pub collider_id: String,
    /// Fraction of `displacement` at which the cast first reaches the body.
    pub fraction: f64,
    pub point_on_capsule: Vec3,
    pub point_on_body: Vec3,
    /// Unit surface normal directed from the hit body toward the query capsule.
    pub surface_normal: Vec3,
    pub penetration_depth_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactSource {
    /// Jolt 5.1+ manifold plus `EstimateCollisionResponse`, captured inside the
    /// pre-solver contact callback.
    JoltEstimatedResponse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactPhase {
    Began,
    Persisted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContactInvariant {
    Unknown = 0,
    NullCallbackInput = 1,
    MissingBodyIdentity = 2,
    MissingColliderIdentity = 3,
    IdentityIndexOverflow = 4,
    ContactCountOverflow = 5,
    ContactCountMismatch = 6,
    NonFiniteBaseOffset = 7,
    NonFiniteContactPoint = 8,
    InvalidNormal = 9,
    NonFinitePointVelocity = 10,
    NonFiniteMaterialResponse = 11,
    ImpulseCountOverflow = 12,
    ImpulseCountCapacityExceeded = 13,
    InvalidEstimatedImpulse = 14,
    EstimatedImpulseSumInvalid = 15,
    BufferLockPoisoned = 16,
    CatalogIdentityMissing = 17,
    CallbackPanicked = 18,
    LeftBodyCatalogMissing = 19,
    RightBodyCatalogMissing = 20,
    LeftColliderCatalogMissing = 21,
    RightColliderCatalogMissing = 22,
    NonFinitePenetration = 23,
    ImpulseContactCountMismatch = 24,
    NonFiniteClosingSpeed = 25,
}

impl ContactInvariant {
    pub const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Unknown,
            1 => Self::NullCallbackInput,
            2 => Self::MissingBodyIdentity,
            3 => Self::MissingColliderIdentity,
            4 => Self::IdentityIndexOverflow,
            5 => Self::ContactCountOverflow,
            6 => Self::ContactCountMismatch,
            7 => Self::NonFiniteBaseOffset,
            8 => Self::NonFiniteContactPoint,
            9 => Self::InvalidNormal,
            10 => Self::NonFinitePointVelocity,
            11 => Self::NonFiniteMaterialResponse,
            12 => Self::ImpulseCountOverflow,
            13 => Self::ImpulseCountCapacityExceeded,
            14 => Self::InvalidEstimatedImpulse,
            15 => Self::EstimatedImpulseSumInvalid,
            16 => Self::BufferLockPoisoned,
            17 => Self::CatalogIdentityMissing,
            18 => Self::CallbackPanicked,
            19 => Self::LeftBodyCatalogMissing,
            20 => Self::RightBodyCatalogMissing,
            21 => Self::LeftColliderCatalogMissing,
            22 => Self::RightColliderCatalogMissing,
            23 => Self::NonFinitePenetration,
            24 => Self::ImpulseContactCountMismatch,
            25 => Self::NonFiniteClosingSpeed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContactRecord {
    pub body_a_id: String,
    pub collider_a_id: String,
    pub body_b_id: String,
    pub collider_b_id: String,
    /// Unit normal directed from body A toward body B.
    pub normal: Vec3,
    pub point: Vec3,
    pub penetration_m: f64,
    /// Closing speed at the manifold contact point before the solver step.
    pub impact_speed_mps: f64,
    /// Sum of Jolt's pre-solver estimated normal impulses for the manifold in N s.
    pub estimated_normal_impulse_ns: f64,
    pub phase: ContactPhase,
    pub source: ContactSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepOutput {
    pub fixed_delta_seconds: f32,
    pub bodies: Vec<BodyState>,
    pub contacts: Vec<ContactRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneConfig {
    pub fixed_delta_seconds: f32,
    pub collision_substeps: i32,
    pub max_bodies: u32,
    pub max_body_pairs: u32,
    pub max_contact_constraints: u32,
    pub temporary_allocator_bytes: u32,
    pub max_colliders_per_body: usize,
    pub max_force_newtons: f64,
    pub max_torque_newton_meters: f64,
    pub max_body_translation_m: f64,
    pub max_linear_velocity_mps: f32,
    pub max_angular_velocity_radians_per_second: f32,
}

impl Default for SceneConfig {
    fn default() -> Self {
        Self {
            fixed_delta_seconds: 1.0 / 60.0,
            collision_substeps: 1,
            max_bodies: 4_096,
            max_body_pairs: 16_384,
            max_contact_constraints: 16_384,
            temporary_allocator_bytes: 16 * 1024 * 1024,
            max_colliders_per_body: 4_096,
            max_force_newtons: 10_000_000.0,
            max_torque_newton_meters: 10_000_000.0,
            max_body_translation_m: 1.0,
            max_linear_velocity_mps: 1_000.0,
            max_angular_velocity_radians_per_second: 100.0,
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum PhysicsError {
    #[error("invalid scene configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid body {body_id}: {message}")]
    InvalidBody { body_id: String, message: String },
    #[error("invalid collider {collider_id} on body {body_id}: {message}")]
    InvalidCollider {
        body_id: String,
        collider_id: String,
        message: String,
    },
    #[error("duplicate body ID {0}")]
    DuplicateBodyId(String),
    #[error("replacement body {0} does not exist")]
    ReplacementBodyMissing(String),
    #[error("replacement body ID {replacement_id} does not match target {target_id}")]
    ReplacementBodyIdMismatch {
        target_id: String,
        replacement_id: String,
    },
    #[error("control body {0} does not exist")]
    ControlBodyMissing(String),
    #[error("control body {0} is static")]
    ControlBodyStatic(String),
    #[error("duplicate control for body {0}")]
    DuplicateControl(String),
    #[error("translated body {0} does not exist")]
    BodyTranslationMissing(String),
    #[error("translated body {0} is static")]
    BodyTranslationStatic(String),
    #[error("translation for body {body_id} exceeds limit {limit}")]
    BodyTranslationOutOfBounds { body_id: String, limit: f64 },
    #[error("control for body {body_id} exceeds {kind} limit {limit}")]
    ControlOutOfBounds {
        body_id: String,
        kind: &'static str,
        limit: f64,
    },
    #[error("Jolt initialization failed: {0}")]
    Initialization(String),
    #[error("Jolt rejected shape for body {body_id}: {message}")]
    ShapeCreation { body_id: String, message: String },
    #[error("invalid capsule cast: {0}")]
    InvalidCapsuleCast(String),
    #[error("native capsule cast failed: {0}")]
    CapsuleCastFailed(String),
    #[error("Jolt could not allocate body {0}")]
    BodyCreation(String),
    #[error("Jolt update failed with error mask {0:#x}")]
    Update(u32),
    #[error("Jolt emitted invalid {field} for body {body_id}")]
    BodyStateInvalid {
        body_id: String,
        field: &'static str,
    },
    #[error("Jolt emitted more contacts than the configured authoritative budget")]
    ContactOverflow,
    #[error("Jolt contact callback failed invariant {0:?} during the authoritative update")]
    ContactCallbackFailed(ContactInvariant),
    #[error("the native scene must be rebuilt after a failed authoritative update")]
    SceneRequiresRebuild,
}

pub struct Scene {
    config: SceneConfig,
    specs: BTreeMap<String, BodySpec>,
    native: ffi::NativeScene,
    requires_rebuild: bool,
}

impl std::fmt::Debug for Scene {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Scene")
            .field("config", &self.config)
            .field("body_count", &self.specs.len())
            .finish_non_exhaustive()
    }
}

impl Scene {
    pub fn new(config: SceneConfig) -> Result<Self, PhysicsError> {
        validate_config(&config)?;
        let native = ffi::NativeScene::new(&config)?;
        Ok(Self {
            config,
            specs: BTreeMap::new(),
            native,
            requires_rebuild: false,
        })
    }

    /// Atomically replaces every body in the scene. Inputs are sorted by their
    /// stable string IDs before native bodies and subshapes are created.
    pub fn rebuild(&mut self, bodies: &[BodySpec]) -> Result<(), PhysicsError> {
        let specs = validated_specs(&self.config, bodies)?;
        let native = ffi::NativeScene::build(&self.config, specs.values())?;
        self.native = native;
        self.specs = specs;
        self.requires_rebuild = false;
        Ok(())
    }

    /// Atomically replaces or removes one existing body while leaving every
    /// unrelated native body untouched. A failed validation or native stage
    /// preserves the previous body and keeps the scene usable.
    pub fn replace_body(
        &mut self,
        body_id: &str,
        replacement: Option<BodySpec>,
    ) -> Result<(), PhysicsError> {
        if self.requires_rebuild {
            return Err(PhysicsError::SceneRequiresRebuild);
        }
        if !self.specs.contains_key(body_id) {
            return Err(PhysicsError::ReplacementBodyMissing(body_id.into()));
        }
        if let Some(spec) = &replacement
            && spec.body_id != body_id
        {
            return Err(PhysicsError::ReplacementBodyIdMismatch {
                target_id: body_id.into(),
                replacement_id: spec.body_id.clone(),
            });
        }

        let validated_replacement = replacement
            .map(|spec| {
                validated_specs(&self.config, std::slice::from_ref(&spec))?
                    .remove(body_id)
                    .ok_or_else(|| PhysicsError::ReplacementBodyMissing(body_id.into()))
            })
            .transpose()?;
        self.native
            .replace_body(&self.config, body_id, validated_replacement.as_ref())?;
        if let Some(spec) = validated_replacement {
            self.specs.insert(body_id.into(), spec);
        } else {
            self.specs.remove(body_id);
        }
        Ok(())
    }

    /// Applies bounded controls and advances exactly one configured fixed step.
    pub fn step(&mut self, controls: &[BodyControl]) -> Result<StepOutput, PhysicsError> {
        if self.requires_rebuild {
            return Err(PhysicsError::SceneRequiresRebuild);
        }
        validate_controls(&self.config, &self.specs, controls)?;
        self.native.apply_controls(controls);
        let update = self.native.step(&self.config);
        self.require_native_success(update)?;
        let extraction = self.native.body_states(&self.config);
        let bodies = self.require_native_success(extraction)?;
        let extraction = self.native.take_contacts();
        let contacts = aggregate_contacts(self.require_native_success(extraction)?);
        Ok(StepOutput {
            fixed_delta_seconds: self.config.fixed_delta_seconds,
            bodies,
            contacts,
        })
    }

    pub fn body_states(&mut self) -> Result<Vec<BodyState>, PhysicsError> {
        if self.requires_rebuild {
            return Err(PhysicsError::SceneRequiresRebuild);
        }
        let extraction = self.native.body_states(&self.config);
        self.require_native_success(extraction)
    }

    /// Casts a temporary capsule through the live scene without advancing it.
    /// Results use stable Verse identities and never expose native body IDs.
    pub fn cast_capsule(
        &self,
        query: &CapsuleCast,
    ) -> Result<Option<CapsuleCastHit>, PhysicsError> {
        validate_capsule_cast(&self.specs, query)?;
        self.native.cast_capsule(&self.specs, query)
    }

    /// Applies one bounded server-owned translation to a live dynamic body.
    /// Callers must prove the complete swept destination is collision-clear
    /// with queries before using this for character step or ground snap.
    pub fn translate_dynamic_body(
        &mut self,
        body_id: &str,
        displacement: Vec3,
    ) -> Result<(), PhysicsError> {
        if self.requires_rebuild {
            return Err(PhysicsError::SceneRequiresRebuild);
        }
        let body = self
            .specs
            .get(body_id)
            .ok_or_else(|| PhysicsError::BodyTranslationMissing(body_id.into()))?;
        if body.motion != BodyMotion::Dynamic {
            return Err(PhysicsError::BodyTranslationStatic(body_id.into()));
        }
        let prior_position = body.pose.position;
        if !displacement.is_finite()
            || displacement.length() <= f64::EPSILON
            || displacement.length() > self.config.max_body_translation_m
        {
            return Err(PhysicsError::BodyTranslationOutOfBounds {
                body_id: body_id.into(),
                limit: self.config.max_body_translation_m,
            });
        }
        self.native.translate_body(body_id, displacement);
        self.specs
            .get_mut(body_id)
            .expect("validated live body remains in the stable catalog")
            .pose
            .position = prior_position + displacement;
        Ok(())
    }

    pub fn body_count(&self) -> usize {
        self.specs.len()
    }

    pub fn contains_collider(&self, body_id: &str, collider_id: &str) -> bool {
        self.specs.get(body_id).is_some_and(|spec| {
            spec.colliders
                .iter()
                .map(|collider| collider.collider_id.as_str())
                .chain(
                    spec.sphere_colliders
                        .iter()
                        .map(|collider| collider.collider_id.as_str()),
                )
                .chain(
                    spec.capsule_colliders
                        .iter()
                        .map(|collider| collider.collider_id.as_str()),
                )
                .any(|candidate| candidate == collider_id)
        })
    }

    pub fn body_collider_fingerprint(&self) -> Vec<(String, Vec<String>)> {
        self.specs
            .iter()
            .map(|(body_id, spec)| {
                let mut collider_ids = spec
                    .colliders
                    .iter()
                    .map(|collider| collider.collider_id.clone())
                    .chain(
                        spec.sphere_colliders
                            .iter()
                            .map(|collider| collider.collider_id.clone()),
                    )
                    .chain(
                        spec.capsule_colliders
                            .iter()
                            .map(|collider| collider.collider_id.clone()),
                    )
                    .collect::<Vec<_>>();
                collider_ids.sort();
                (body_id.clone(), collider_ids)
            })
            .collect()
    }

    #[cfg(test)]
    fn fail_next_replacement_before_publish(&mut self) {
        self.native.fail_next_replacement_before_publish();
    }

    fn require_native_success<T>(
        &mut self,
        result: Result<T, PhysicsError>,
    ) -> Result<T, PhysicsError> {
        match result {
            Ok(value) => Ok(value),
            Err(source) => {
                self.requires_rebuild = true;
                Err(source)
            }
        }
    }
}

fn aggregate_contacts(contacts: Vec<ContactRecord>) -> Vec<ContactRecord> {
    let mut aggregate = BTreeMap::new();
    for contact in contacts {
        let key = (
            contact.body_a_id.clone(),
            contact.collider_a_id.clone(),
            contact.body_b_id.clone(),
            contact.collider_b_id.clone(),
        );
        match aggregate.entry(key) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(contact);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if contact_preference(&contact, entry.get()).is_gt() {
                    entry.insert(contact);
                }
            }
        }
    }
    aggregate.into_values().collect()
}

fn contact_preference(left: &ContactRecord, right: &ContactRecord) -> std::cmp::Ordering {
    left.estimated_normal_impulse_ns
        .total_cmp(&right.estimated_normal_impulse_ns)
        .then_with(|| left.impact_speed_mps.total_cmp(&right.impact_speed_mps))
        .then_with(|| left.penetration_m.total_cmp(&right.penetration_m))
        .then_with(|| contact_phase_rank(left.phase).cmp(&contact_phase_rank(right.phase)))
        .then_with(|| left.point.x.total_cmp(&right.point.x))
        .then_with(|| left.point.y.total_cmp(&right.point.y))
        .then_with(|| left.point.z.total_cmp(&right.point.z))
        .then_with(|| left.normal.x.total_cmp(&right.normal.x))
        .then_with(|| left.normal.y.total_cmp(&right.normal.y))
        .then_with(|| left.normal.z.total_cmp(&right.normal.z))
}

const fn contact_phase_rank(phase: ContactPhase) -> u8 {
    match phase {
        ContactPhase::Persisted => 0,
        ContactPhase::Began => 1,
    }
}

fn validate_config(config: &SceneConfig) -> Result<(), PhysicsError> {
    if !config.fixed_delta_seconds.is_finite()
        || config.fixed_delta_seconds <= 0.0
        || config.fixed_delta_seconds > 0.25
    {
        return Err(PhysicsError::InvalidConfiguration(
            "fixed delta must be finite and in (0, 0.25] seconds".into(),
        ));
    }
    if !(1..=16).contains(&config.collision_substeps) {
        return Err(PhysicsError::InvalidConfiguration(
            "collision substeps must be between 1 and 16".into(),
        ));
    }
    if config.max_bodies == 0
        || config.max_body_pairs == 0
        || config.max_contact_constraints == 0
        || config.temporary_allocator_bytes < 1024 * 1024
        || config.max_colliders_per_body == 0
    {
        return Err(PhysicsError::InvalidConfiguration(
            "body, pair, contact, collider, and allocator budgets must be positive".into(),
        ));
    }
    let collision_substeps = u64::try_from(config.collision_substeps).map_err(|_| {
        PhysicsError::InvalidConfiguration("collision substeps must be positive".into())
    })?;
    let contact_records = u64::from(config.max_contact_constraints) * collision_substeps;
    if config.max_bodies > MAX_NATIVE_BODIES
        || config.max_body_pairs > MAX_NATIVE_BODY_PAIRS
        || config.max_contact_constraints > MAX_NATIVE_CONTACT_CONSTRAINTS
        || contact_records > MAX_NATIVE_CONTACT_RECORDS
        || config.temporary_allocator_bytes > MAX_TEMPORARY_ALLOCATOR_BYTES
        || config.max_colliders_per_body > MAX_COLLIDERS_PER_BODY
    {
        return Err(PhysicsError::InvalidConfiguration(
            "native body, pair, contact, collider, or allocator budget exceeds the practical authority limit"
                .into(),
        ));
    }
    for (label, value) in [
        ("force", config.max_force_newtons),
        ("torque", config.max_torque_newton_meters),
        ("body translation", config.max_body_translation_m),
        ("linear velocity", f64::from(config.max_linear_velocity_mps)),
        (
            "angular velocity",
            f64::from(config.max_angular_velocity_radians_per_second),
        ),
    ] {
        if !value.is_finite() || value <= 0.0 || value > f64::from(f32::MAX) {
            return Err(PhysicsError::InvalidConfiguration(format!(
                "maximum {label} must be finite, positive, and representable by Jolt"
            )));
        }
    }
    Ok(())
}

fn validated_specs(
    config: &SceneConfig,
    bodies: &[BodySpec],
) -> Result<BTreeMap<String, BodySpec>, PhysicsError> {
    if bodies.len() > config.max_bodies as usize {
        return Err(PhysicsError::InvalidConfiguration(format!(
            "{} bodies exceed configured maximum {}",
            bodies.len(),
            config.max_bodies
        )));
    }
    let mut result = BTreeMap::new();
    for body in bodies {
        validate_id(&body.body_id).map_err(|message| PhysicsError::InvalidBody {
            body_id: body.body_id.clone(),
            message,
        })?;
        if result.contains_key(&body.body_id) {
            return Err(PhysicsError::DuplicateBodyId(body.body_id.clone()));
        }
        if !body.pose.position.is_finite()
            || !body.linear_velocity.is_finite()
            || !body.angular_velocity.is_finite()
        {
            return Err(PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: "pose and velocities must be finite".into(),
            });
        }
        let rotation =
            validated_rotation(body.pose.rotation).ok_or_else(|| PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: "rotation must be finite and normalized".into(),
            })?;
        if body.linear_velocity.length() > f64::from(config.max_linear_velocity_mps)
            || body.angular_velocity.length()
                > f64::from(config.max_angular_velocity_radians_per_second)
        {
            return Err(PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: "initial velocity exceeds configured maximum".into(),
            });
        }
        if !body.friction.is_finite()
            || !(0.0..=1.0).contains(&body.friction)
            || !body.restitution.is_finite()
            || !(0.0..=1.0).contains(&body.restitution)
            || !body.gravity_factor.is_finite()
            || !(0.0..=8.0).contains(&body.gravity_factor)
            || !body.inertia_multiplier.is_finite()
            || !(0.01..=100.0).contains(&body.inertia_multiplier)
        {
            return Err(PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: "friction, restitution, gravity factor, or inertia multiplier is outside its safe range".into(),
            });
        }
        if body.motion == BodyMotion::Static && body.motion_quality != MotionQuality::Discrete {
            return Err(PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: "linear-cast motion quality is valid only for dynamic bodies".into(),
            });
        }
        let collider_count = body
            .colliders
            .len()
            .checked_add(body.sphere_colliders.len())
            .and_then(|count| count.checked_add(body.capsule_colliders.len()))
            .ok_or_else(|| PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: "collider count overflowed".into(),
            })?;
        if collider_count == 0 || collider_count > config.max_colliders_per_body {
            return Err(PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: format!(
                    "body must contain between 1 and {} colliders",
                    config.max_colliders_per_body
                ),
            });
        }
        let mut collider_ids = BTreeSet::new();
        let mut validated = body.clone();
        validated.pose.rotation = rotation;
        for collider in &mut validated.colliders {
            validate_id(&collider.collider_id).map_err(|message| {
                PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message,
                }
            })?;
            if !collider_ids.insert(collider.collider_id.clone()) {
                return Err(PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "collider ID is duplicated within its body".into(),
                });
            }
            if !collider.local_pose.position.is_finite()
                || !collider.half_extents.is_finite()
                || collider.half_extents.x <= 0.0
                || collider.half_extents.y <= 0.0
                || collider.half_extents.z <= 0.0
                || collider.half_extents.x > 1_000_000.0
                || collider.half_extents.y > 1_000_000.0
                || collider.half_extents.z > 1_000_000.0
                || !collider.density_kg_per_m3.is_finite()
                || collider.density_kg_per_m3 <= 0.0
            {
                return Err(PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "pose, extents, and density must be finite and positive".into(),
                });
            }
            collider.local_pose.rotation = validated_rotation(collider.local_pose.rotation)
                .ok_or_else(|| PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "rotation must be finite and normalized".into(),
                })?;
        }
        for collider in &mut validated.sphere_colliders {
            validate_id(&collider.collider_id).map_err(|message| {
                PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message,
                }
            })?;
            if !collider_ids.insert(collider.collider_id.clone()) {
                return Err(PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "collider ID is duplicated within its body".into(),
                });
            }
            if !collider.local_pose.position.is_finite()
                || !collider.radius.is_finite()
                || collider.radius <= 0.0
                || collider.radius > 1_000_000.0
                || !collider.density_kg_per_m3.is_finite()
                || collider.density_kg_per_m3 <= 0.0
            {
                return Err(PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "pose, radius, and density must be finite and positive".into(),
                });
            }
            collider.local_pose.rotation = validated_rotation(collider.local_pose.rotation)
                .ok_or_else(|| PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "rotation must be finite and normalized".into(),
                })?;
        }
        for collider in &mut validated.capsule_colliders {
            validate_id(&collider.collider_id).map_err(|message| {
                PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message,
                }
            })?;
            if !collider_ids.insert(collider.collider_id.clone()) {
                return Err(PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "collider ID is duplicated within its body".into(),
                });
            }
            if !collider.local_pose.position.is_finite()
                || !collider.radius.is_finite()
                || collider.radius <= 0.0
                || collider.radius > 1_000_000.0
                || !collider.half_height_of_cylinder.is_finite()
                || collider.half_height_of_cylinder <= 0.0
                || collider.half_height_of_cylinder > 1_000_000.0
                || !collider.density_kg_per_m3.is_finite()
                || collider.density_kg_per_m3 <= 0.0
            {
                return Err(PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "pose, radius, half-height, and density must be finite and positive"
                        .into(),
                });
            }
            collider.local_pose.rotation = validated_rotation(collider.local_pose.rotation)
                .ok_or_else(|| PhysicsError::InvalidCollider {
                    body_id: body.body_id.clone(),
                    collider_id: collider.collider_id.clone(),
                    message: "rotation must be finite and normalized".into(),
                })?;
        }
        validated
            .colliders
            .sort_by(|left, right| left.collider_id.cmp(&right.collider_id));
        validated
            .sphere_colliders
            .sort_by(|left, right| left.collider_id.cmp(&right.collider_id));
        validated
            .capsule_colliders
            .sort_by(|left, right| left.collider_id.cmp(&right.collider_id));
        result.insert(validated.body_id.clone(), validated);
    }
    Ok(result)
}

fn validated_rotation(rotation: Quat) -> Option<Quat> {
    if !rotation.is_finite() || (rotation.length_squared() - 1.0).abs() > 1.0e-3 {
        None
    } else {
        rotation.normalized()
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.trim().is_empty() || id.len() > 128 {
        Err("ID must contain between 1 and 128 bytes".into())
    } else {
        Ok(())
    }
}

fn validate_controls(
    config: &SceneConfig,
    specs: &BTreeMap<String, BodySpec>,
    controls: &[BodyControl],
) -> Result<(), PhysicsError> {
    let mut controlled = BTreeSet::new();
    for control in controls {
        let Some(spec) = specs.get(&control.body_id) else {
            return Err(PhysicsError::ControlBodyMissing(control.body_id.clone()));
        };
        if spec.motion == BodyMotion::Static {
            return Err(PhysicsError::ControlBodyStatic(control.body_id.clone()));
        }
        if !controlled.insert(control.body_id.clone()) {
            return Err(PhysicsError::DuplicateControl(control.body_id.clone()));
        }
        for (kind, value, limit) in [
            ("force", control.force_newtons, config.max_force_newtons),
            (
                "torque",
                control.torque_newton_meters,
                config.max_torque_newton_meters,
            ),
        ] {
            if !value.is_finite() || value.length() > limit {
                return Err(PhysicsError::ControlOutOfBounds {
                    body_id: control.body_id.clone(),
                    kind,
                    limit,
                });
            }
        }
    }
    Ok(())
}

fn validate_capsule_cast(
    specs: &BTreeMap<String, BodySpec>,
    query: &CapsuleCast,
) -> Result<(), PhysicsError> {
    if !query.pose.position.is_finite()
        || validated_rotation(query.pose.rotation).is_none()
        || !query.radius.is_finite()
        || query.radius <= 0.0
        || !query.half_height_of_cylinder.is_finite()
        || query.half_height_of_cylinder <= 0.0
        || !query.displacement.is_finite()
        || query.displacement.length() <= f64::EPSILON
        || query.displacement.length() > MAX_CAPSULE_CAST_DISTANCE_M
    {
        return Err(PhysicsError::InvalidCapsuleCast(format!(
            "pose and dimensions must be finite and normalized, dimensions must be positive, and displacement must be in (0, {MAX_CAPSULE_CAST_DISTANCE_M}] meters"
        )));
    }
    if let Some(body_id) = &query.ignore_body_id
        && !specs.contains_key(body_id)
    {
        return Err(PhysicsError::InvalidCapsuleCast(format!(
            "ignored body {body_id} does not exist"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(estimated_impulse: f64, speed: f64, penetration: f64, point: Vec3) -> ContactRecord {
        ContactRecord {
            body_a_id: "body-a".into(),
            collider_a_id: "collider-a".into(),
            body_b_id: "body-b".into(),
            collider_b_id: "collider-b".into(),
            normal: Vec3::new(1.0, 0.0, 0.0),
            point,
            penetration_m: penetration,
            impact_speed_mps: speed,
            estimated_normal_impulse_ns: estimated_impulse,
            phase: ContactPhase::Began,
            source: ContactSource::JoltEstimatedResponse,
        }
    }

    #[test]
    fn contact_aggregation_selects_one_whole_deterministic_record() {
        let high_speed = contact(2.0, 9.0, 0.8, Vec3::new(1.0, 2.0, 3.0));
        let high_estimate = contact(7.0, 1.0, 0.1, Vec3::new(4.0, 5.0, 6.0));
        let first = aggregate_contacts(vec![high_speed.clone(), high_estimate.clone()]);
        let reversed = aggregate_contacts(vec![high_estimate.clone(), high_speed]);
        assert_eq!(first, vec![high_estimate.clone()]);
        assert_eq!(reversed, vec![high_estimate]);
    }

    #[test]
    fn contact_aggregation_breaks_equal_estimate_ties_without_arrival_order() {
        let left = contact(3.0, 2.0, 0.2, Vec3::new(1.0, 0.0, 0.0));
        let right = contact(3.0, 2.0, 0.2, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(
            aggregate_contacts(vec![left.clone(), right.clone()]),
            aggregate_contacts(vec![right, left])
        );
    }

    #[test]
    fn post_update_failure_requires_rebuild_before_another_step() {
        let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
        let body = BodySpec::dynamic(
            "grid",
            Pose::IDENTITY,
            vec![BoxColliderSpec::unit_cube("block")],
        );
        scene
            .rebuild(std::slice::from_ref(&body))
            .expect("scene builds");
        assert_eq!(
            scene.require_native_success::<()>(Err(PhysicsError::ContactOverflow)),
            Err(PhysicsError::ContactOverflow)
        );
        assert_eq!(scene.step(&[]), Err(PhysicsError::SceneRequiresRebuild));
        scene.rebuild(&[body]).expect("explicit rebuild succeeds");
        assert!(scene.step(&[]).is_ok());
    }

    #[test]
    fn staged_replacement_failure_restores_catalog_body_and_scene_usability() {
        let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
        let target = BodySpec::static_body(
            "voxel-chunk-0-0-0",
            Pose::IDENTITY,
            vec![
                BoxColliderSpec::unit_cube("voxel-0-0-0"),
                BoxColliderSpec {
                    local_pose: Pose::new(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY),
                    ..BoxColliderSpec::unit_cube("voxel-1-0-0")
                },
            ],
        );
        let probe = BodySpec::dynamic(
            "probe-grid",
            Pose::new(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY),
            vec![BoxColliderSpec::unit_cube("probe-block")],
        );
        scene
            .rebuild(&[target.clone(), probe])
            .expect("initial scene builds");
        let before = scene.body_states().expect("prior body state extracts");
        let replacement = BodySpec::static_body(
            target.body_id.clone(),
            target.pose,
            vec![target.colliders[0].clone()],
        );

        scene.fail_next_replacement_before_publish();
        let error = scene
            .replace_body(&target.body_id, Some(replacement))
            .expect_err("injected replacement fails");
        assert!(matches!(error, PhysicsError::Initialization(_)));
        assert_eq!(
            scene.body_states().expect("old scene remains usable"),
            before
        );
        assert!(scene.contains_collider(&target.body_id, "voxel-1-0-0"));
        let output = scene.step(&[]).expect("restored catalog remains stepable");
        assert!(
            output
                .contacts
                .iter()
                .any(|contact| contact.collider_b_id == "voxel-1-0-0")
        );
    }

    #[test]
    fn practical_native_budget_limits_reject_unbounded_allocations() {
        let too_many_bodies = SceneConfig {
            max_bodies: MAX_NATIVE_BODIES + 1,
            ..SceneConfig::default()
        };
        assert!(matches!(
            validate_config(&too_many_bodies),
            Err(PhysicsError::InvalidConfiguration(_))
        ));

        let too_many_contact_records = SceneConfig {
            collision_substeps: 16,
            max_contact_constraints: MAX_NATIVE_CONTACT_RECORDS as u32 / 16 + 1,
            ..SceneConfig::default()
        };
        assert!(matches!(
            validate_config(&too_many_contact_records),
            Err(PhysicsError::InvalidConfiguration(_))
        ));

        let too_large_allocator = SceneConfig {
            temporary_allocator_bytes: MAX_TEMPORARY_ALLOCATOR_BYTES + 1,
            ..SceneConfig::default()
        };
        assert!(matches!(
            validate_config(&too_large_allocator),
            Err(PhysicsError::InvalidConfiguration(_))
        ));
    }
}
