// SPDX-License-Identifier: AGPL-3.0-or-later

//! Domain-neutral, safe adapter around the pinned Jolt Physics bindings.
//!
//! Jolt performs motion integration and collision response. The current pinned
//! `JoltC` API does not expose solved contact manifolds, so contact telemetry is
//! conservatively reconstructed from the same compound box geometry. Callers
//! must treat [`ContactRecord::impact_speed_mps`] as a bounded gameplay input,
//! not as a solver impulse.

mod contact;
mod ffi;
mod math;

use std::collections::{BTreeMap, BTreeSet};

pub use math::{Pose, Quat, Vec3};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyMotion {
    Static,
    Dynamic,
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
pub struct BodySpec {
    pub body_id: String,
    pub motion: BodyMotion,
    pub pose: Pose,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub friction: f32,
    pub restitution: f32,
    pub gravity_factor: f32,
    pub allow_sleeping: bool,
    pub colliders: Vec<BoxColliderSpec>,
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
            allow_sleeping: true,
            colliders,
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
            allow_sleeping: true,
            colliders,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactSource {
    /// Approximate compound-box telemetry because `JoltC` 0.3.1 does not expose
    /// Jolt's contact listener or solved manifolds.
    GeometricFallback,
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
    /// Closing speed at the approximate contact point before the step.
    pub impact_speed_mps: f64,
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
    #[error("control body {0} does not exist")]
    ControlBodyMissing(String),
    #[error("control body {0} is static")]
    ControlBodyStatic(String),
    #[error("duplicate control for body {0}")]
    DuplicateControl(String),
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
    #[error("Jolt could not allocate body {0}")]
    BodyCreation(String),
    #[error("Jolt update failed with error mask {0:#x}")]
    Update(u32),
}

pub struct Scene {
    config: SceneConfig,
    specs: BTreeMap<String, BodySpec>,
    native: ffi::NativeScene,
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
        })
    }

    /// Atomically replaces every body in the scene. Inputs are sorted by their
    /// stable string IDs before native bodies and subshapes are created.
    pub fn rebuild(&mut self, bodies: &[BodySpec]) -> Result<(), PhysicsError> {
        let specs = validated_specs(&self.config, bodies)?;
        let native = ffi::NativeScene::build(&self.config, specs.values())?;
        self.native = native;
        self.specs = specs;
        Ok(())
    }

    /// Applies bounded controls and advances exactly one configured fixed step.
    pub fn step(&mut self, controls: &[BodyControl]) -> Result<StepOutput, PhysicsError> {
        validate_controls(&self.config, &self.specs, controls)?;
        let before = self.native.body_states();
        self.native.apply_controls(controls);
        self.native.step(&self.config)?;
        let bodies = self.native.body_states();
        let contacts = contact::contacts_for_step(&self.specs, &before, &bodies);
        Ok(StepOutput {
            fixed_delta_seconds: self.config.fixed_delta_seconds,
            bodies,
            contacts,
        })
    }

    pub fn body_states(&self) -> Vec<BodyState> {
        self.native.body_states()
    }

    pub fn body_count(&self) -> usize {
        self.specs.len()
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
    for (label, value) in [
        ("force", config.max_force_newtons),
        ("torque", config.max_torque_newton_meters),
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
        {
            return Err(PhysicsError::InvalidBody {
                body_id: body.body_id.clone(),
                message: "friction, restitution, or gravity factor is outside its safe range"
                    .into(),
            });
        }
        if body.colliders.is_empty() || body.colliders.len() > config.max_colliders_per_body {
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
        validated
            .colliders
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
