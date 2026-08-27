// SPDX-License-Identifier: AGPL-3.0-or-later

//! The only project-owned unsafe Jolt FFI boundary.
//!
//! Each unsafe block states the native lifetime or pointer invariant on which
//! it relies. No raw pointer or Jolt identifier escapes this module.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::ptr;
use std::sync::OnceLock;

use joltc_sys::{
    JPC_ACTIVATION_ACTIVATE, JPC_ACTIVATION_DONT_ACTIVATE, JPC_BodyCreationSettings, JPC_BodyID,
    JPC_BodyInterface_AddBody, JPC_BodyInterface_AddForceAndTorque, JPC_BodyInterface_CreateBody,
    JPC_BodyInterface_DestroyBody, JPC_BodyInterface_GetAngularVelocity,
    JPC_BodyInterface_GetLinearVelocity, JPC_BodyInterface_GetPosition,
    JPC_BodyInterface_GetRotation, JPC_BodyInterface_IsActive, JPC_BodyInterface_RemoveBody,
    JPC_BoxShapeSettings, JPC_BoxShapeSettings_Create, JPC_JobSystemThreadPool,
    JPC_JobSystemThreadPool_delete, JPC_JobSystemThreadPool_new3, JPC_MOTION_QUALITY_DISCRETE,
    JPC_MOTION_QUALITY_LINEAR_CAST, JPC_MOTION_TYPE_DYNAMIC, JPC_MOTION_TYPE_STATIC,
    JPC_PHYSICS_UPDATE_ERROR_NONE, JPC_PhysicsSystem_GetBodyInterface, JPC_PhysicsSystem_Update,
    JPC_Quat, JPC_RVec3, JPC_Shape, JPC_Shape_Release, JPC_StaticCompoundShapeSettings,
    JPC_StaticCompoundShapeSettings_Create, JPC_String, JPC_String_c_str, JPC_String_delete,
    JPC_SubShapeSettings, JPC_TempAllocatorImpl, JPC_TempAllocatorImpl_delete,
    JPC_TempAllocatorImpl_new, JPC_Vec3,
};
use rolt::{
    BroadPhaseLayer, BroadPhaseLayerInterface, ObjectLayer, ObjectLayerPairFilter,
    ObjectVsBroadPhaseLayerFilter, PhysicsSystem,
};

use crate::{
    BodyControl, BodyMotion, BodySpec, BodyState, BoxColliderSpec, PhysicsError, Pose, Quat,
    SceneConfig, Vec3,
};

const OBJECT_LAYER_STATIC: u16 = 0;
const OBJECT_LAYER_DYNAMIC: u16 = 1;
const BROAD_PHASE_STATIC: u8 = 0;
const BROAD_PHASE_DYNAMIC: u8 = 1;

#[derive(Debug)]
struct Layers;

impl BroadPhaseLayerInterface for Layers {
    fn get_num_broad_phase_layers(&self) -> u32 {
        2
    }

    fn get_broad_phase_layer(&self, layer: ObjectLayer) -> BroadPhaseLayer {
        match layer.raw() {
            OBJECT_LAYER_DYNAMIC => BroadPhaseLayer::new(BROAD_PHASE_DYNAMIC),
            _ => BroadPhaseLayer::new(BROAD_PHASE_STATIC),
        }
    }
}

#[derive(Debug)]
struct ObjectVsBroadPhase;

impl ObjectVsBroadPhaseLayerFilter for ObjectVsBroadPhase {
    fn should_collide(&self, object: ObjectLayer, broad_phase: BroadPhaseLayer) -> bool {
        match object.raw() {
            OBJECT_LAYER_STATIC => broad_phase.raw() == BROAD_PHASE_DYNAMIC,
            OBJECT_LAYER_DYNAMIC => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
struct ObjectPairs;

impl ObjectLayerPairFilter for ObjectPairs {
    fn should_collide(&self, left: ObjectLayer, right: ObjectLayer) -> bool {
        match left.raw() {
            OBJECT_LAYER_STATIC => right.raw() == OBJECT_LAYER_DYNAMIC,
            OBJECT_LAYER_DYNAMIC => {
                matches!(right.raw(), OBJECT_LAYER_STATIC | OBJECT_LAYER_DYNAMIC)
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct NativeBody {
    id: JPC_BodyID,
}

pub(crate) struct NativeScene {
    physics: Option<PhysicsSystem>,
    temporary_allocator: *mut JPC_TempAllocatorImpl,
    job_system: *mut JPC_JobSystemThreadPool,
    bodies: BTreeMap<String, NativeBody>,
}

impl std::fmt::Debug for NativeScene {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeScene")
            .field("body_count", &self.bodies.len())
            .finish_non_exhaustive()
    }
}

// SAFETY: NativeScene exclusively owns all Jolt resources and native callbacks.
// Its public owner exposes mutation only through `&mut self`, it has no Sync
// implementation, the job system has zero worker threads, and callback `this`
// pointers owned by rolt point to stable heap allocations rather than into this
// movable struct. Moving a dormant scene between worker threads therefore does
// not create concurrent native access or invalidate a pointer.
unsafe impl Send for NativeScene {}

impl NativeScene {
    pub(crate) fn new(config: &SceneConfig) -> Result<Self, PhysicsError> {
        initialize_jolt();
        let mut physics = PhysicsSystem::new();
        physics.init(
            config.max_bodies,
            0,
            config.max_body_pairs,
            config.max_contact_constraints,
            Layers,
            ObjectVsBroadPhase,
            ObjectPairs,
        );

        // SAFETY: The constructors return owned opaque pointers. A zero-thread
        // job system executes submitted work on the calling thread, which keeps
        // the authoritative step single-threaded. Both pointers are checked and
        // released in Drop after PhysicsSystem has been dropped.
        let temporary_allocator =
            unsafe { JPC_TempAllocatorImpl_new(config.temporary_allocator_bytes) };
        // SAFETY: Same ownership argument as above. Jolt documents zero worker
        // threads as using the calling thread to execute jobs.
        let job_system = unsafe {
            JPC_JobSystemThreadPool_new3(
                joltc_sys::JPC_MAX_PHYSICS_JOBS as u32,
                joltc_sys::JPC_MAX_PHYSICS_BARRIERS as u32,
                0,
            )
        };
        if temporary_allocator.is_null() || job_system.is_null() {
            if !job_system.is_null() {
                // SAFETY: `job_system` is a live owned pointer from the
                // constructor above and has not been released.
                unsafe { JPC_JobSystemThreadPool_delete(job_system) };
            }
            if !temporary_allocator.is_null() {
                // SAFETY: `temporary_allocator` is a live owned pointer from
                // the constructor above and has not been released.
                unsafe { JPC_TempAllocatorImpl_delete(temporary_allocator) };
            }
            return Err(PhysicsError::Initialization(
                "native allocator or job system allocation returned null".into(),
            ));
        }
        Ok(Self {
            physics: Some(physics),
            temporary_allocator,
            job_system,
            bodies: BTreeMap::new(),
        })
    }

    pub(crate) fn build<'a>(
        config: &SceneConfig,
        bodies: impl Iterator<Item = &'a BodySpec>,
    ) -> Result<Self, PhysicsError> {
        let mut scene = Self::new(config)?;
        for body in bodies {
            scene.add_body(config, body)?;
        }
        if !scene.bodies.is_empty() {
            scene
                .physics
                .as_ref()
                .expect("live native scene owns physics system")
                .optimize_broad_phase();
        }
        Ok(scene)
    }

    fn add_body(&mut self, config: &SceneConfig, spec: &BodySpec) -> Result<(), PhysicsError> {
        let mut child_shapes = Vec::with_capacity(spec.colliders.len());
        let mut sub_shapes = Vec::with_capacity(spec.colliders.len());
        for (index, collider) in spec.colliders.iter().enumerate() {
            let shape = create_box(spec, collider, index)?;
            sub_shapes.push(JPC_SubShapeSettings {
                Shape: shape.0,
                Position: local_vec3(collider.local_pose.position),
                Rotation: quat(collider.local_pose.rotation),
                UserData: u32::try_from(index).unwrap_or(u32::MAX),
                ..JPC_SubShapeSettings::default()
            });
            child_shapes.push(shape);
        }

        let compound_settings = JPC_StaticCompoundShapeSettings {
            UserData: 0,
            SubShapes: sub_shapes.as_ptr(),
            SubShapesLen: sub_shapes.len(),
        };
        let compound =
            create_compound(&compound_settings).map_err(|message| PhysicsError::ShapeCreation {
                body_id: spec.body_id.clone(),
                message,
            })?;

        let mut settings = JPC_BodyCreationSettings {
            Position: world_vec3(spec.pose.position),
            Rotation: quat(spec.pose.rotation),
            LinearVelocity: local_vec3(spec.linear_velocity),
            AngularVelocity: local_vec3(spec.angular_velocity),
            ObjectLayer: match spec.motion {
                BodyMotion::Static => OBJECT_LAYER_STATIC,
                BodyMotion::Dynamic => OBJECT_LAYER_DYNAMIC,
            },
            MotionType: match spec.motion {
                BodyMotion::Static => JPC_MOTION_TYPE_STATIC,
                BodyMotion::Dynamic => JPC_MOTION_TYPE_DYNAMIC,
            },
            MotionQuality: match spec.motion {
                BodyMotion::Static => JPC_MOTION_QUALITY_DISCRETE,
                BodyMotion::Dynamic => JPC_MOTION_QUALITY_LINEAR_CAST,
            },
            AllowSleeping: spec.allow_sleeping,
            Friction: spec.friction,
            Restitution: spec.restitution,
            GravityFactor: spec.gravity_factor,
            MaxLinearVelocity: config.max_linear_velocity_mps,
            MaxAngularVelocity: config.max_angular_velocity_radians_per_second,
            Shape: compound.0,
            ..JPC_BodyCreationSettings::default()
        };
        // A static body's stored velocities must be zero even if an input was
        // accidentally constructed before validation grew stricter.
        if spec.motion == BodyMotion::Static {
            settings.LinearVelocity = local_vec3(Vec3::ZERO);
            settings.AngularVelocity = local_vec3(Vec3::ZERO);
        }

        let interface = self.body_interface();
        // SAFETY: `interface` belongs to the live physics system. `settings`
        // references `compound`, which owns a live shape for the duration of
        // the call. Jolt's BodyCreationSettings copies/refcounts that shape.
        let body = unsafe { JPC_BodyInterface_CreateBody(interface, ptr::from_ref(&settings)) };
        if body.is_null() {
            return Err(PhysicsError::BodyCreation(spec.body_id.clone()));
        }
        // SAFETY: `body` is the non-null body pointer just returned by the
        // interface and remains owned by that interface.
        let id = unsafe { joltc_sys::JPC_Body_GetID(body) };
        // SAFETY: The body ID is allocated by this interface and is added once.
        unsafe {
            JPC_BodyInterface_AddBody(
                interface,
                id,
                if spec.motion == BodyMotion::Dynamic {
                    JPC_ACTIVATION_ACTIVATE
                } else {
                    JPC_ACTIVATION_DONT_ACTIVATE
                },
            );
        }
        self.bodies.insert(spec.body_id.clone(), NativeBody { id });
        Ok(())
    }

    pub(crate) fn apply_controls(&mut self, controls: &[BodyControl]) {
        let interface = self.body_interface();
        for control in controls {
            let body = self.bodies[&control.body_id];
            // SAFETY: Control validation guarantees this ID belongs to a live
            // dynamic body in this interface and all vector values fit f32.
            unsafe {
                JPC_BodyInterface_AddForceAndTorque(
                    interface,
                    body.id,
                    local_vec3(control.force_newtons),
                    local_vec3(control.torque_newton_meters),
                );
            }
        }
    }

    pub(crate) fn step(&mut self, config: &SceneConfig) -> Result<(), PhysicsError> {
        let physics = self
            .physics
            .as_ref()
            .expect("live native scene owns physics system");
        // SAFETY: PhysicsSystem, allocator, and zero-worker job system are live,
        // exclusively owned by this `&mut self`, and configured together. The
        // fixed delta and substep count were validated by the safe layer.
        let error = unsafe {
            JPC_PhysicsSystem_Update(
                physics.as_raw(),
                config.fixed_delta_seconds,
                config.collision_substeps,
                self.temporary_allocator,
                self.job_system,
            )
        };
        if error == JPC_PHYSICS_UPDATE_ERROR_NONE {
            Ok(())
        } else {
            Err(PhysicsError::Update(error))
        }
    }

    pub(crate) fn body_states(&self) -> Vec<BodyState> {
        let interface = self.body_interface();
        self.bodies
            .iter()
            .map(|(body_id, body)| {
                // SAFETY: Every stored ID belongs to the live body interface;
                // getters copy values and do not retain output pointers.
                let (position, rotation, linear_velocity, angular_velocity, active) = unsafe {
                    (
                        JPC_BodyInterface_GetPosition(interface, body.id),
                        JPC_BodyInterface_GetRotation(interface, body.id),
                        JPC_BodyInterface_GetLinearVelocity(interface, body.id),
                        JPC_BodyInterface_GetAngularVelocity(interface, body.id),
                        JPC_BodyInterface_IsActive(interface, body.id),
                    )
                };
                BodyState {
                    body_id: body_id.clone(),
                    pose: Pose::new(from_world_vec3(position), from_quat(rotation)),
                    linear_velocity: from_local_vec3(linear_velocity),
                    angular_velocity: from_local_vec3(angular_velocity),
                    active,
                }
            })
            .collect()
    }

    fn body_interface(&self) -> *mut joltc_sys::JPC_BodyInterface {
        let physics = self
            .physics
            .as_ref()
            .expect("live native scene owns physics system");
        // SAFETY: `physics` is live and owns its body interface for its entire
        // lifetime. The returned pointer never escapes NativeScene methods.
        unsafe { JPC_PhysicsSystem_GetBodyInterface(physics.as_raw()) }
    }
}

impl Drop for NativeScene {
    fn drop(&mut self) {
        if self.physics.is_some() {
            let interface = self.body_interface();
            for body in self.bodies.values() {
                // SAFETY: Stored bodies were added exactly once and have not
                // been removed. Removal precedes destruction as Jolt requires.
                unsafe {
                    JPC_BodyInterface_RemoveBody(interface, body.id);
                    JPC_BodyInterface_DestroyBody(interface, body.id);
                }
            }
            self.bodies.clear();
        }
        // Drop PhysicsSystem (and its callback bridges) before the allocators
        // used by updates. No native body remains at this point.
        drop(self.physics.take());
        if !self.job_system.is_null() {
            // SAFETY: Owned pointer, no update is in progress, released once.
            unsafe { JPC_JobSystemThreadPool_delete(self.job_system) };
            self.job_system = ptr::null_mut();
        }
        if !self.temporary_allocator.is_null() {
            // SAFETY: Owned pointer, no update is in progress, released once.
            unsafe { JPC_TempAllocatorImpl_delete(self.temporary_allocator) };
            self.temporary_allocator = ptr::null_mut();
        }
    }
}

fn initialize_jolt() {
    static INITIALIZED: OnceLock<()> = OnceLock::new();
    INITIALIZED.get_or_init(|| {
        // rolt contains the unsafe calls and exposes these initialization
        // operations as safe functions. OnceLock serializes and deduplicates
        // process-global initialization.
        rolt::register_default_allocator();
        rolt::factory_init();
        rolt::register_types();
    });
}

struct ShapeRef(*mut JPC_Shape);

impl Drop for ShapeRef {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: ShapeRef owns one active Jolt reference returned by a
            // successful shape-settings Create call. It is released once.
            unsafe { JPC_Shape_Release(self.0) };
        }
    }
}

fn create_box(
    body: &BodySpec,
    collider: &BoxColliderSpec,
    index: usize,
) -> Result<ShapeRef, PhysicsError> {
    let settings = JPC_BoxShapeSettings {
        UserData: u64::try_from(index).unwrap_or(u64::MAX),
        Density: collider.density_kg_per_m3,
        HalfExtent: local_vec3(collider.half_extents),
        ConvexRadius: 0.0,
        ..JPC_BoxShapeSettings::default()
    };
    let mut shape = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: Output pointers refer to local variables valid for the call;
    // settings contains no borrowed pointers. Success returns one owned ref.
    let created = unsafe {
        JPC_BoxShapeSettings_Create(
            ptr::from_ref(&settings),
            ptr::from_mut(&mut shape),
            ptr::from_mut(&mut error),
        )
    };
    if created && !shape.is_null() {
        Ok(ShapeRef(shape))
    } else {
        Err(PhysicsError::ShapeCreation {
            body_id: body.body_id.clone(),
            message: take_error(error),
        })
    }
}

fn create_compound(settings: &JPC_StaticCompoundShapeSettings) -> Result<ShapeRef, String> {
    let mut shape = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: `settings.SubShapes` points into a live Vec for the complete call,
    // and each referenced child ShapeRef remains live until this returns.
    let created = unsafe {
        JPC_StaticCompoundShapeSettings_Create(
            ptr::from_ref(settings),
            ptr::from_mut(&mut shape),
            ptr::from_mut(&mut error),
        )
    };
    if created && !shape.is_null() {
        Ok(ShapeRef(shape))
    } else {
        Err(take_error(error))
    }
}

fn take_error(error: *mut JPC_String) -> String {
    if error.is_null() {
        return "native shape creation failed without an error message".into();
    }
    // SAFETY: Failed shape creation returned an owned live JPC_String. Its C
    // string is read before the object is deleted exactly once.
    unsafe {
        let pointer = JPC_String_c_str(error);
        let text = if pointer.is_null() {
            "native shape creation returned an empty error".into()
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        };
        JPC_String_delete(error);
        text
    }
}

fn local_vec3(value: Vec3) -> JPC_Vec3 {
    JPC_Vec3 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
        _w: value.z as f32,
    }
}

fn world_vec3(value: Vec3) -> JPC_RVec3 {
    JPC_RVec3 {
        x: value.x,
        y: value.y,
        z: value.z,
        _w: value.z,
    }
}

fn quat(value: Quat) -> JPC_Quat {
    JPC_Quat {
        x: value.x,
        y: value.y,
        z: value.z,
        w: value.w,
    }
}

fn from_local_vec3(value: JPC_Vec3) -> Vec3 {
    Vec3::new(f64::from(value.x), f64::from(value.y), f64::from(value.z))
}

fn from_world_vec3(value: JPC_RVec3) -> Vec3 {
    Vec3::new(value.x, value.y, value.z)
}

fn from_quat(value: JPC_Quat) -> Quat {
    Quat::new(value.x, value.y, value.z, value.w)
        .normalized()
        .unwrap_or(Quat::IDENTITY)
}
