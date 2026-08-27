// SPDX-License-Identifier: AGPL-3.0-or-later

//! The only project-owned unsafe Jolt FFI boundary.
//!
//! Each unsafe block states the native lifetime or pointer invariant on which
//! it relies. No raw pointer or Jolt identifier escapes this module.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};

use joltc_sys::{
    JPC_ACTIVATION_ACTIVATE, JPC_ACTIVATION_DONT_ACTIVATE, JPC_Body_GetPointVelocity,
    JPC_Body_GetShape, JPC_Body_GetUserData, JPC_BodyCreationSettings, JPC_BodyID,
    JPC_BodyInterface_AddBody, JPC_BodyInterface_AddForceAndTorque, JPC_BodyInterface_CreateBody,
    JPC_BodyInterface_DestroyBody, JPC_BodyInterface_GetAngularVelocity,
    JPC_BodyInterface_GetLinearVelocity, JPC_BodyInterface_GetPosition,
    JPC_BodyInterface_GetRotation, JPC_BodyInterface_GetShape, JPC_BodyInterface_IsActive,
    JPC_BodyInterface_RemoveBody, JPC_BodyInterface_SetPosition, JPC_BoxShapeSettings,
    JPC_BoxShapeSettings_Create, JPC_CapsuleShapeSettings, JPC_CapsuleShapeSettings_Create,
    JPC_CollideShapeResult, JPC_CollisionEstimationResult,
    JPC_ContactListener as JpcContactListener, JPC_ContactListener_delete, JPC_ContactListener_new,
    JPC_ContactListenerFns, JPC_ContactManifold, JPC_ContactSettings,
    JPC_EstimateCollisionResponse, JPC_JobSystemThreadPool, JPC_JobSystemThreadPool_delete,
    JPC_JobSystemThreadPool_new3, JPC_MOTION_QUALITY_DISCRETE, JPC_MOTION_QUALITY_LINEAR_CAST,
    JPC_MOTION_TYPE_DYNAMIC, JPC_MOTION_TYPE_STATIC, JPC_PHYSICS_UPDATE_ERROR_NONE,
    JPC_PhysicsSystem_GetBodyInterface, JPC_PhysicsSystem_SetContactListener,
    JPC_PhysicsSystem_Update, JPC_Quat, JPC_RVec3, JPC_Shape, JPC_Shape_GetSubShapeUserData,
    JPC_Shape_Release, JPC_SphereShapeSettings, JPC_SphereShapeSettings_Create,
    JPC_StaticCompoundShapeSettings, JPC_StaticCompoundShapeSettings_Create, JPC_String,
    JPC_String_c_str, JPC_String_delete, JPC_SubShapeIDPair, JPC_SubShapeSettings,
    JPC_TempAllocatorImpl, JPC_TempAllocatorImpl_delete, JPC_TempAllocatorImpl_new,
    JPC_VALIDATE_RESULT_ACCEPT_ALL_CONTACTS, JPC_ValidateResult, JPC_Vec3, JPC_Vec4,
};
use rolt::{
    Body as RoltBody, BodyFilter, BodyFilterImpl, BodyId as RoltBodyId, BroadPhaseLayer,
    BroadPhaseLayerInterface, CastShapeArgs, CastShapeCollectorImpl, ClosestHitCastShapeCollector,
    ObjectLayer, ObjectLayerPairFilter, ObjectVsBroadPhaseLayerFilter, PhysicsSystem, RShapeCast,
    RVec3 as RoltRVec3, Vec3 as RoltVec3,
};

use crate::{
    BodyControl, BodyMotion, BodySpec, BodyState, BoxColliderSpec, CapsuleCast, CapsuleCastHit,
    CapsuleColliderSpec, ContactInvariant, ContactPhase, ContactRecord, ContactSource,
    MotionQuality, PhysicsError, Pose, Quat, SceneConfig, SphereColliderSpec, Vec3,
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

#[derive(Debug, Clone, Copy)]
struct IgnoreNativeBody {
    id: JPC_BodyID,
}

impl BodyFilter for IgnoreNativeBody {
    fn should_collide(&self, body_id: RoltBodyId) -> bool {
        body_id.raw() != self.id
    }

    fn should_collide_locked(&self, body: &mut RoltBody<'_>) -> bool {
        body.id().raw() != self.id
    }
}

#[derive(Debug)]
struct StagedNativeBody {
    body: NativeBody,
    #[cfg(test)]
    catalog_rollback: ContactCatalogRollback,
}

#[derive(Debug)]
struct ContactBuffer {
    bodies: Vec<BodyContactCatalog>,
    contacts: Vec<RawContact>,
    max_contacts: usize,
    overflowed: bool,
}

#[derive(Debug)]
struct BodyContactCatalog {
    body_id: String,
    collider_ids: Vec<String>,
}

#[derive(Debug)]
struct ContactCatalogRollback {
    index: usize,
    previous: Option<BodyContactCatalog>,
}

#[derive(Debug, Clone, Copy)]
struct RawContact {
    body1_index: usize,
    collider1_index: usize,
    body2_index: usize,
    collider2_index: usize,
    normal: Vec3,
    point: Vec3,
    penetration_m: f64,
    impact_speed_mps: f64,
    estimated_normal_impulse_ns: f64,
    phase: ContactPhase,
}

#[derive(Debug)]
struct NativeContactListener {
    buffer: Mutex<ContactBuffer>,
    failure_code: AtomicU8,
}

impl NativeContactListener {
    fn new(max_contacts: usize) -> Result<Self, PhysicsError> {
        let mut contacts = Vec::new();
        contacts.try_reserve_exact(max_contacts).map_err(|error| {
            PhysicsError::InvalidConfiguration(format!(
                "contact callback budget cannot be allocated: {error}"
            ))
        })?;
        Ok(Self {
            buffer: Mutex::new(ContactBuffer {
                bodies: Vec::new(),
                contacts,
                max_contacts,
                overflowed: false,
            }),
            failure_code: AtomicU8::new(0),
        })
    }

    fn fail(&self, invariant: ContactInvariant) {
        let _ = self.failure_code.compare_exchange(
            0,
            invariant as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    fn failure(&self) -> Option<ContactInvariant> {
        let code = self.failure_code.load(Ordering::Acquire);
        if code == 0 {
            None
        } else {
            Some(ContactInvariant::from_code(code).unwrap_or(ContactInvariant::Unknown))
        }
    }

    unsafe fn capture(
        &self,
        body1: *const joltc_sys::JPC_Body,
        body2: *const joltc_sys::JPC_Body,
        manifold: *const JPC_ContactManifold,
        settings: *const JPC_ContactSettings,
        phase: ContactPhase,
    ) {
        if body1.is_null() || body2.is_null() || manifold.is_null() || settings.is_null() {
            self.fail(ContactInvariant::NullCallbackInput);
            return;
        }
        // SAFETY: Jolt guarantees all callback pointers are live for the call.
        // Individual initialized scalar fields are copied with raw reads; no
        // Rust reference is formed to the C mirror's uninitialized tail slots.
        let body1_user_data = unsafe { JPC_Body_GetUserData(body1) };
        let body2_user_data = unsafe { JPC_Body_GetUserData(body2) };
        let shape1 = unsafe { JPC_Body_GetShape(body1) };
        let shape2 = unsafe { JPC_Body_GetShape(body2) };
        if body1_user_data == 0 || body2_user_data == 0 || shape1.is_null() || shape2.is_null() {
            self.fail(ContactInvariant::MissingBodyIdentity);
            return;
        }
        let sub_shape1 = unsafe { ptr::addr_of!((*manifold).SubShapeID1).read() };
        let sub_shape2 = unsafe { ptr::addr_of!((*manifold).SubShapeID2).read() };
        let collider1_user_data = unsafe { JPC_Shape_GetSubShapeUserData(shape1, sub_shape1) };
        let collider2_user_data = unsafe { JPC_Shape_GetSubShapeUserData(shape2, sub_shape2) };
        if collider1_user_data == 0 || collider2_user_data == 0 {
            self.fail(ContactInvariant::MissingColliderIdentity);
            return;
        }
        let (Some(body1_index), Some(collider1_index), Some(body2_index), Some(collider2_index)) = (
            usize::try_from(body1_user_data - 1).ok(),
            usize::try_from(collider1_user_data - 1).ok(),
            usize::try_from(body2_user_data - 1).ok(),
            usize::try_from(collider2_user_data - 1).ok(),
        ) else {
            self.fail(ContactInvariant::IdentityIndexOverflow);
            return;
        };

        let contacts_on_left = unsafe { ptr::addr_of!((*manifold).RelativeContactPointsOn1) };
        let contacts_on_right = unsafe { ptr::addr_of!((*manifold).RelativeContactPointsOn2) };
        let left_contact_count = unsafe { ptr::addr_of!((*contacts_on_left).length).read() };
        let right_contact_count = unsafe { ptr::addr_of!((*contacts_on_right).length).read() };
        let (Ok(contact_count), Ok(right_contact_count)) = (
            usize::try_from(left_contact_count),
            usize::try_from(right_contact_count),
        ) else {
            self.fail(ContactInvariant::ContactCountOverflow);
            return;
        };
        if contact_count == 0 || contact_count != right_contact_count || contact_count > 64 {
            self.fail(ContactInvariant::ContactCountMismatch);
            return;
        }
        let base = from_world_vec3(unsafe { ptr::addr_of!((*manifold).BaseOffset).read() });
        if !base.is_finite() {
            self.fail(ContactInvariant::NonFiniteBaseOffset);
            return;
        }
        let left_point_data =
            unsafe { ptr::addr_of!((*contacts_on_left).points).cast::<JPC_Vec3>() };
        let right_point_data =
            unsafe { ptr::addr_of!((*contacts_on_right).points).cast::<JPC_Vec3>() };
        let mut sum = Vec3::ZERO;
        for index in 0..contact_count {
            let point_on_left =
                base + from_local_vec3(unsafe { left_point_data.add(index).read() });
            let point_on_right =
                base + from_local_vec3(unsafe { right_point_data.add(index).read() });
            if !point_on_left.is_finite() || !point_on_right.is_finite() {
                self.fail(ContactInvariant::NonFiniteContactPoint);
                return;
            }
            sum = sum + (point_on_left + point_on_right) * 0.5;
        }
        let point = sum * (1.0 / contact_count as f64);
        let raw_normal =
            from_local_vec3(unsafe { ptr::addr_of!((*manifold).WorldSpaceNormal).read() });
        let Some(normal) = raw_normal.normalized() else {
            self.fail(ContactInvariant::InvalidNormal);
            return;
        };
        let velocity1 =
            from_local_vec3(unsafe { JPC_Body_GetPointVelocity(body1, world_vec3(point)) });
        let velocity2 =
            from_local_vec3(unsafe { JPC_Body_GetPointVelocity(body2, world_vec3(point)) });
        if !point.is_finite() || !velocity1.is_finite() || !velocity2.is_finite() {
            self.fail(ContactInvariant::NonFinitePointVelocity);
            return;
        }

        // Zero initialization makes every otherwise-unwritten tail impulse a
        // valid Rust float value before the C API fills the active prefix.
        let mut estimate = MaybeUninit::<JPC_CollisionEstimationResult>::zeroed();
        let friction = unsafe { ptr::addr_of!((*settings).CombinedFriction).read() };
        let restitution = unsafe { ptr::addr_of!((*settings).CombinedRestitution).read() };
        if !friction.is_finite() || !restitution.is_finite() {
            self.fail(ContactInvariant::NonFiniteMaterialResponse);
            return;
        }
        unsafe {
            JPC_EstimateCollisionResponse(
                body1,
                body2,
                manifold,
                estimate.as_mut_ptr(),
                friction,
                restitution,
                1.0,
                10,
            );
        }
        let estimate = unsafe { estimate.assume_init() };
        let Ok(impulse_count) = usize::try_from(estimate.NumImpulses) else {
            self.fail(ContactInvariant::ImpulseCountOverflow);
            return;
        };
        if impulse_count > estimate.Impulses.len() {
            self.fail(ContactInvariant::ImpulseCountCapacityExceeded);
            return;
        }
        if impulse_count != contact_count {
            self.fail(ContactInvariant::ImpulseContactCountMismatch);
            return;
        }
        let mut estimated_normal_impulse_ns = 0.0;
        for impulse in &estimate.Impulses[..impulse_count] {
            let value = f64::from(impulse.ContactImpulse);
            let Some(total) =
                accumulate_estimated_normal_impulse(estimated_normal_impulse_ns, value)
            else {
                self.fail(ContactInvariant::InvalidEstimatedImpulse);
                return;
            };
            estimated_normal_impulse_ns = total;
        }
        let signed_penetration_m =
            f64::from(unsafe { ptr::addr_of!((*manifold).PenetrationDepth).read() });
        let closing_speed_mps = (velocity1 - velocity2).dot(normal).max(0.0);
        if !estimated_normal_impulse_ns.is_finite() {
            self.fail(ContactInvariant::EstimatedImpulseSumInvalid);
            return;
        }
        if !signed_penetration_m.is_finite() {
            self.fail(ContactInvariant::NonFinitePenetration);
            return;
        }
        if !closing_speed_mps.is_finite() {
            self.fail(ContactInvariant::NonFiniteClosingSpeed);
            return;
        }
        // Jolt can report a negative depth for a valid speculative contact.
        // The public field represents overlap only, so separation maps to zero.
        let penetration_m = signed_penetration_m.max(0.0);

        let Ok(mut buffer) = self.buffer.lock() else {
            self.fail(ContactInvariant::BufferLockPoisoned);
            return;
        };
        if buffer
            .bodies
            .get(body1_index)
            .is_none_or(|body| body.collider_ids.get(collider1_index).is_none())
            || buffer
                .bodies
                .get(body2_index)
                .is_none_or(|body| body.collider_ids.get(collider2_index).is_none())
        {
            self.fail(ContactInvariant::CatalogIdentityMissing);
            return;
        }
        if buffer.contacts.len() >= buffer.max_contacts {
            buffer.overflowed = true;
            return;
        }
        buffer.contacts.push(RawContact {
            body1_index,
            collider1_index,
            body2_index,
            collider2_index,
            normal,
            point,
            penetration_m,
            impact_speed_mps: closing_speed_mps,
            estimated_normal_impulse_ns,
            phase,
        });
    }
}

unsafe extern "C" fn contact_validate(
    _this: *mut c_void,
    _body1: *const joltc_sys::JPC_Body,
    _body2: *const joltc_sys::JPC_Body,
    _base_offset: JPC_RVec3,
    _collision_result: *const JPC_CollideShapeResult,
) -> JPC_ValidateResult {
    JPC_VALIDATE_RESULT_ACCEPT_ALL_CONTACTS
}

unsafe extern "C" fn contact_added(
    this: *mut c_void,
    body1: *const joltc_sys::JPC_Body,
    body2: *const joltc_sys::JPC_Body,
    manifold: *const JPC_ContactManifold,
    settings: *mut JPC_ContactSettings,
) {
    if let Some(listener) = unsafe { this.cast::<NativeContactListener>().as_ref() } {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            listener.capture(body1, body2, manifold, settings, ContactPhase::Began);
        }));
        if result.is_err() {
            listener.fail(ContactInvariant::CallbackPanicked);
        }
    }
}

unsafe extern "C" fn contact_persisted(
    this: *mut c_void,
    body1: *const joltc_sys::JPC_Body,
    body2: *const joltc_sys::JPC_Body,
    manifold: *const JPC_ContactManifold,
    settings: *mut JPC_ContactSettings,
) {
    if let Some(listener) = unsafe { this.cast::<NativeContactListener>().as_ref() } {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
            listener.capture(body1, body2, manifold, settings, ContactPhase::Persisted);
        }));
        if result.is_err() {
            listener.fail(ContactInvariant::CallbackPanicked);
        }
    }
}

unsafe extern "C" fn contact_removed(_this: *mut c_void, _pair: *const JPC_SubShapeIDPair) {}

pub(crate) struct NativeScene {
    physics: Option<PhysicsSystem>,
    temporary_allocator: *mut JPC_TempAllocatorImpl,
    job_system: *mut JPC_JobSystemThreadPool,
    bodies: BTreeMap<String, NativeBody>,
    contact_listener: *mut JpcContactListener,
    contact_listener_state: Box<NativeContactListener>,
    #[cfg(test)]
    fail_replacement_before_publish: bool,
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
// implementation, the job system has zero worker threads, and the raw callback
// `this` pointer targets a stable owned Box rather than this movable struct.
// Moving a dormant scene between worker threads therefore does not create
// concurrent native access or invalidate a pointer.
unsafe impl Send for NativeScene {}

impl NativeScene {
    pub(crate) fn new(config: &SceneConfig) -> Result<Self, PhysicsError> {
        // Allocate every fallible Rust-owned callback resource before any raw
        // native allocation so an allocation error cannot leak Jolt objects.
        let mut contact_listener_state =
            Box::new(NativeContactListener::new(contact_record_budget(config)?)?);
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
        let callbacks = JPC_ContactListenerFns {
            OnContactValidate: Some(contact_validate),
            OnContactAdded: Some(contact_added),
            OnContactPersisted: Some(contact_persisted),
            OnContactRemoved: Some(contact_removed),
        };
        // SAFETY: The callback state is heap allocated and remains stable until
        // after PhysicsSystem is detached and destroyed in Drop.
        let contact_listener = unsafe {
            JPC_ContactListener_new(
                ptr::from_mut(contact_listener_state.as_mut()).cast::<c_void>(),
                callbacks,
            )
        };
        if contact_listener.is_null() {
            // SAFETY: Both are live owned allocations and no update can exist.
            unsafe {
                JPC_JobSystemThreadPool_delete(job_system);
                JPC_TempAllocatorImpl_delete(temporary_allocator);
            }
            return Err(PhysicsError::Initialization(
                "native contact listener allocation returned null".into(),
            ));
        }
        // SAFETY: Both pointers are live and remain so through every update.
        unsafe { JPC_PhysicsSystem_SetContactListener(physics.raw(), contact_listener) };
        Ok(Self {
            physics: Some(physics),
            temporary_allocator,
            job_system,
            bodies: BTreeMap::new(),
            contact_listener,
            contact_listener_state,
            #[cfg(test)]
            fail_replacement_before_publish: false,
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
        let staged = self.create_body(config, spec)?;
        let previous = self.bodies.insert(spec.body_id.clone(), staged.body);
        debug_assert!(previous.is_none(), "validated build contains unique bodies");
        Ok(())
    }

    pub(crate) fn replace_body(
        &mut self,
        config: &SceneConfig,
        body_id: &str,
        replacement: Option<&BodySpec>,
    ) -> Result<(), PhysicsError> {
        let previous = self
            .bodies
            .get(body_id)
            .copied()
            .ok_or_else(|| PhysicsError::ReplacementBodyMissing(body_id.into()))?;
        let staged = replacement
            .map(|spec| self.create_body(config, spec))
            .transpose()?;

        #[cfg(test)]
        if self.fail_replacement_before_publish {
            self.fail_replacement_before_publish = false;
            if let Some(staged) = staged {
                self.discard_staged_body(staged)?;
            }
            return Err(PhysicsError::Initialization(
                "injected replacement failure before publication".into(),
            ));
        }

        let interface = self.body_interface();
        // SAFETY: `previous` is the live body currently mapped by `body_id`.
        // A staged replacement, when present, was already added successfully;
        // no solver update or callback can overlap this exclusive mutation.
        unsafe {
            JPC_BodyInterface_RemoveBody(interface, previous.id);
            JPC_BodyInterface_DestroyBody(interface, previous.id);
        }
        if let Some(staged) = staged {
            self.bodies.insert(body_id.into(), staged.body);
        } else {
            self.bodies.remove(body_id);
        }
        Ok(())
    }

    fn create_body(
        &mut self,
        config: &SceneConfig,
        spec: &BodySpec,
    ) -> Result<StagedNativeBody, PhysicsError> {
        let ordered_colliders = ordered_colliders(spec);
        let mut child_shapes = Vec::with_capacity(ordered_colliders.len());
        let mut sub_shapes = Vec::with_capacity(ordered_colliders.len());
        for (index, collider) in ordered_colliders.iter().enumerate() {
            let shape = match collider {
                ColliderRef::Box(collider) => create_box(spec, collider, index)?,
                ColliderRef::Sphere(collider) => create_sphere(spec, collider, index)?,
                ColliderRef::Capsule(collider) => create_capsule(spec, collider, index)?,
            };
            let local_pose = collider.local_pose();
            sub_shapes.push(JPC_SubShapeSettings {
                Shape: shape.0,
                Position: local_vec3(local_pose.position),
                Rotation: quat(local_pose.rotation),
                UserData: u32::try_from(index + 1).unwrap_or(u32::MAX),
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

        let (body_user_data, catalog_rollback) = self.stage_contact_catalog(spec)?;

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
            MotionQuality: match spec.motion_quality {
                MotionQuality::Discrete => JPC_MOTION_QUALITY_DISCRETE,
                MotionQuality::LinearCast => JPC_MOTION_QUALITY_LINEAR_CAST,
            },
            AllowSleeping: spec.allow_sleeping,
            Friction: spec.friction,
            Restitution: spec.restitution,
            GravityFactor: spec.gravity_factor,
            InertiaMultiplier: spec.inertia_multiplier,
            MaxLinearVelocity: config.max_linear_velocity_mps,
            MaxAngularVelocity: config.max_angular_velocity_radians_per_second,
            Shape: compound.0,
            UserData: body_user_data,
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
            self.restore_contact_catalog(catalog_rollback)?;
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
        #[cfg(not(test))]
        drop(catalog_rollback);
        Ok(StagedNativeBody {
            body: NativeBody { id },
            #[cfg(test)]
            catalog_rollback,
        })
    }

    fn stage_contact_catalog(
        &mut self,
        spec: &BodySpec,
    ) -> Result<(u64, ContactCatalogRollback), PhysicsError> {
        let mut buffer = self.contact_listener_state.buffer.lock().map_err(|_| {
            PhysicsError::Initialization("native contact buffer lock was poisoned".into())
        })?;
        let existing_index = buffer
            .bodies
            .iter()
            .position(|catalog| catalog.body_id == spec.body_id);
        let index = existing_index.unwrap_or(buffer.bodies.len());
        let user_data = index
            .checked_add(1)
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| {
                PhysicsError::Initialization("native body contact catalog overflowed".into())
            })?;
        let replacement = BodyContactCatalog {
            body_id: spec.body_id.clone(),
            collider_ids: ordered_colliders(spec)
                .into_iter()
                .map(|collider| collider.id().to_owned())
                .collect(),
        };
        let previous = if existing_index.is_some() {
            Some(std::mem::replace(&mut buffer.bodies[index], replacement))
        } else {
            buffer.bodies.push(replacement);
            None
        };
        Ok((user_data, ContactCatalogRollback { index, previous }))
    }

    fn restore_contact_catalog(
        &mut self,
        rollback: ContactCatalogRollback,
    ) -> Result<(), PhysicsError> {
        let mut buffer = self.contact_listener_state.buffer.lock().map_err(|_| {
            PhysicsError::Initialization("native contact buffer lock was poisoned".into())
        })?;
        if let Some(previous) = rollback.previous {
            buffer.bodies[rollback.index] = previous;
        } else if rollback.index + 1 == buffer.bodies.len() {
            buffer.bodies.pop();
        } else {
            return Err(PhysicsError::Initialization(
                "native contact catalog rollback order was invalid".into(),
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    fn discard_staged_body(&mut self, staged: StagedNativeBody) -> Result<(), PhysicsError> {
        let interface = self.body_interface();
        // SAFETY: The staged body was added exactly once but has not replaced
        // the published body map. No update overlaps this rollback.
        unsafe {
            JPC_BodyInterface_RemoveBody(interface, staged.body.id);
            JPC_BodyInterface_DestroyBody(interface, staged.body.id);
        }
        self.restore_contact_catalog(staged.catalog_rollback)
    }

    #[cfg(test)]
    pub(crate) fn fail_next_replacement_before_publish(&mut self) {
        self.fail_replacement_before_publish = true;
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

    pub(crate) fn translate_body(&mut self, body_id: &str, displacement: Vec3) {
        let interface = self.body_interface();
        let body = self.bodies[body_id];
        // SAFETY: Safe-adapter validation guarantees this ID is a live dynamic
        // body and the bounded displacement is finite. The getter and setter
        // copy values synchronously and retain no pointers.
        unsafe {
            let position = from_world_vec3(JPC_BodyInterface_GetPosition(interface, body.id));
            JPC_BodyInterface_SetPosition(
                interface,
                body.id,
                world_vec3(position + displacement),
                JPC_ACTIVATION_ACTIVATE,
            );
        }
    }

    pub(crate) fn step(&mut self, config: &SceneConfig) -> Result<(), PhysicsError> {
        let mut contact_buffer = self.contact_listener_state.buffer.lock().map_err(|_| {
            PhysicsError::Initialization("native contact buffer lock was poisoned".into())
        })?;
        contact_buffer.contacts.clear();
        contact_buffer.overflowed = false;
        drop(contact_buffer);
        self.contact_listener_state
            .failure_code
            .store(0, Ordering::Release);
        let physics = self
            .physics
            .as_ref()
            .expect("live native scene owns physics system");
        // SAFETY: PhysicsSystem, allocator, and zero-worker job system are live,
        // exclusively owned by this `&mut self`, and configured together. The
        // fixed delta and substep count were validated by the safe layer.
        let error = unsafe {
            JPC_PhysicsSystem_Update(
                physics.raw(),
                config.fixed_delta_seconds,
                config.collision_substeps,
                self.temporary_allocator,
                self.job_system.cast::<joltc_sys::JPC_JobSystem>(),
            )
        };
        if error == JPC_PHYSICS_UPDATE_ERROR_NONE {
            Ok(())
        } else {
            Err(PhysicsError::Update(error))
        }
    }

    pub(crate) fn take_contacts(&mut self) -> Result<Vec<ContactRecord>, PhysicsError> {
        let mut buffer = self.contact_listener_state.buffer.lock().map_err(|_| {
            PhysicsError::Initialization("native contact buffer lock was poisoned".into())
        })?;
        if let Some(failure) = self.contact_listener_state.failure() {
            return Err(PhysicsError::ContactCallbackFailed(failure));
        }
        if buffer.overflowed {
            return Err(PhysicsError::ContactOverflow);
        }
        let raw_contacts = buffer.contacts.drain(..).collect::<Vec<_>>();
        let mut contacts = Vec::with_capacity(raw_contacts.len());
        for raw in raw_contacts {
            let Some(left) = buffer.bodies.get(raw.body1_index) else {
                return Err(PhysicsError::ContactCallbackFailed(
                    ContactInvariant::LeftBodyCatalogMissing,
                ));
            };
            let Some(right) = buffer.bodies.get(raw.body2_index) else {
                return Err(PhysicsError::ContactCallbackFailed(
                    ContactInvariant::RightBodyCatalogMissing,
                ));
            };
            let Some(left_collider) = left.collider_ids.get(raw.collider1_index) else {
                return Err(PhysicsError::ContactCallbackFailed(
                    ContactInvariant::LeftColliderCatalogMissing,
                ));
            };
            let Some(right_collider) = right.collider_ids.get(raw.collider2_index) else {
                return Err(PhysicsError::ContactCallbackFailed(
                    ContactInvariant::RightColliderCatalogMissing,
                ));
            };
            let mut left_body_id = left.body_id.clone();
            let mut left_collider_id = left_collider.clone();
            let mut right_body_id = right.body_id.clone();
            let mut right_collider_id = right_collider.clone();
            let mut normal = raw.normal;
            if (&right_body_id, &right_collider_id) < (&left_body_id, &left_collider_id) {
                std::mem::swap(&mut left_body_id, &mut right_body_id);
                std::mem::swap(&mut left_collider_id, &mut right_collider_id);
                normal = -normal;
            }
            contacts.push(ContactRecord {
                body_a_id: left_body_id,
                collider_a_id: left_collider_id,
                body_b_id: right_body_id,
                collider_b_id: right_collider_id,
                normal,
                point: raw.point,
                penetration_m: raw.penetration_m,
                impact_speed_mps: raw.impact_speed_mps,
                estimated_normal_impulse_ns: raw.estimated_normal_impulse_ns,
                phase: raw.phase,
                source: ContactSource::JoltEstimatedResponse,
            });
        }
        Ok(contacts)
    }

    pub(crate) fn body_states(&self, config: &SceneConfig) -> Result<Vec<BodyState>, PhysicsError> {
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
                validated_body_state(
                    body_id,
                    from_world_vec3(position),
                    from_quat(rotation),
                    from_local_vec3(linear_velocity),
                    from_local_vec3(angular_velocity),
                    active,
                    config,
                )
            })
            .collect()
    }

    pub(crate) fn cast_capsule(
        &self,
        specs: &BTreeMap<String, BodySpec>,
        query: &CapsuleCast,
    ) -> Result<Option<CapsuleCastHit>, PhysicsError> {
        let shape = create_query_capsule(query)?;
        let ignored = query
            .ignore_body_id
            .as_ref()
            .map(|body_id| IgnoreNativeBody {
                id: self.bodies[body_id].id,
            })
            .map(BodyFilterImpl::new);
        let base_offset = query.pose.position;
        let mut collector = ClosestHitCastShapeCollector::new();
        let physics = self
            .physics
            .as_ref()
            .expect("live native scene owns physics system");
        let narrow_phase = physics.narrow_phase_query();
        let settings = joltc_sys::JPC_ShapeCastSettings {
            ReturnDeepestPoint: true,
            ..joltc_sys::JPC_ShapeCastSettings::default()
        };
        // SAFETY: The temporary shape, collector bridge, filter bridge, and
        // physics query all remain live for this synchronous read-only cast.
        // Query validation bounds every scalar before the f32 conversion.
        unsafe {
            narrow_phase.cast_shape(CastShapeArgs {
                shapecast: RShapeCast {
                    shape: shape.0,
                    scale: RoltVec3::ONE,
                    center_of_mass_start: world_transform(query.pose),
                    direction: RoltVec3::new(
                        query.displacement.x as f32,
                        query.displacement.y as f32,
                        query.displacement.z as f32,
                    ),
                },
                base_offset: RoltRVec3::new(base_offset.x, base_offset.y, base_offset.z),
                settings,
                collector: Some(CastShapeCollectorImpl::new_borrowed(&mut collector)),
                broad_phase_layer_filter: None,
                object_layer_filter: None,
                body_filter: ignored,
                shape_filter: None,
            });
        }

        let Some(result) = collector.result else {
            return Ok(None);
        };
        let Some((body_id, native_body)) = self
            .bodies
            .iter()
            .find(|(_, body)| body.id == result.BodyID2)
        else {
            return Err(PhysicsError::CapsuleCastFailed(
                "hit body has no stable Verse identity".into(),
            ));
        };
        let interface = self.body_interface();
        // SAFETY: The hit ID belongs to this live body interface and its shape
        // remains owned by that body throughout this synchronous query.
        let body_shape = unsafe { JPC_BodyInterface_GetShape(interface, native_body.id) };
        if body_shape.is_null() {
            return Err(PhysicsError::CapsuleCastFailed(
                "hit body has no live collision shape".into(),
            ));
        }
        // SAFETY: `body_shape` is live and `SubShapeID2` was returned for it by
        // the same query. User data was assigned from the stable collider list.
        let collider_user_data =
            unsafe { JPC_Shape_GetSubShapeUserData(body_shape, result.SubShapeID2) };
        let Some(collider_index) = collider_user_data
            .checked_sub(1)
            .and_then(|index| usize::try_from(index).ok())
        else {
            return Err(PhysicsError::CapsuleCastFailed(
                "hit collider has no stable Verse identity".into(),
            ));
        };
        let collider_id = ordered_colliders(&specs[body_id])
            .get(collider_index)
            .map(|collider| collider.id().to_owned())
            .ok_or_else(|| {
                PhysicsError::CapsuleCastFailed(
                    "hit collider identity is outside the stable catalog".into(),
                )
            })?;
        let point_on_capsule = base_offset + from_local_vec3(result.ContactPointOn1);
        let point_on_body = base_offset + from_local_vec3(result.ContactPointOn2);
        let surface_normal = (-from_local_vec3(result.PenetrationAxis))
            .normalized()
            .ok_or_else(|| {
                PhysicsError::CapsuleCastFailed("hit returned an invalid surface normal".into())
            })?;
        let fraction = f64::from(result.Fraction);
        let penetration_depth_m = f64::from(result.PenetrationDepth);
        if !point_on_capsule.is_finite()
            || !point_on_body.is_finite()
            || !fraction.is_finite()
            || !(-1.0e-6..=1.0 + 1.0e-6).contains(&fraction)
            || !penetration_depth_m.is_finite()
            || penetration_depth_m < 0.0
        {
            return Err(PhysicsError::CapsuleCastFailed(
                "hit returned non-finite or out-of-range geometry".into(),
            ));
        }
        Ok(Some(CapsuleCastHit {
            body_id: body_id.clone(),
            collider_id,
            fraction: fraction.clamp(0.0, 1.0),
            point_on_capsule,
            point_on_body,
            surface_normal,
            penetration_depth_m,
        }))
    }

    fn body_interface(&self) -> *mut joltc_sys::JPC_BodyInterface {
        let physics = self
            .physics
            .as_ref()
            .expect("live native scene owns physics system");
        // SAFETY: `physics` is live and owns its body interface for its entire
        // lifetime. The returned pointer never escapes NativeScene methods.
        unsafe { JPC_PhysicsSystem_GetBodyInterface(physics.raw()) }
    }
}

fn contact_record_budget(config: &SceneConfig) -> Result<usize, PhysicsError> {
    usize::try_from(config.max_contact_constraints)
        .ok()
        .and_then(|constraints| {
            usize::try_from(config.collision_substeps)
                .ok()
                .and_then(|substeps| constraints.checked_mul(substeps))
        })
        .ok_or_else(|| {
            PhysicsError::InvalidConfiguration(
                "contact callback budget overflows the platform address space".into(),
            )
        })
}

fn accumulate_estimated_normal_impulse(total: f64, value: f64) -> Option<f64> {
    if !total.is_finite() || !value.is_finite() || value < 0.0 {
        return None;
    }
    let next = total + value;
    next.is_finite().then_some(next)
}

impl Drop for NativeScene {
    fn drop(&mut self) {
        if self.physics.is_some() {
            let physics_raw = self
                .physics
                .as_ref()
                .map_or(ptr::null_mut(), PhysicsSystem::raw);
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
            // SAFETY: No update is active. Detaching prevents PhysicsSystem
            // destruction from retaining the separately owned listener.
            unsafe {
                JPC_PhysicsSystem_SetContactListener(physics_raw, ptr::null_mut());
            }
        }
        // Drop PhysicsSystem before the callback and update allocators. No
        // native body remains at this point.
        drop(self.physics.take());
        if !self.contact_listener.is_null() {
            // SAFETY: Listener is detached, owned, and released exactly once.
            unsafe { JPC_ContactListener_delete(self.contact_listener) };
            self.contact_listener = ptr::null_mut();
        }
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

#[derive(Debug, Clone, Copy)]
enum ColliderRef<'a> {
    Box(&'a BoxColliderSpec),
    Sphere(&'a SphereColliderSpec),
    Capsule(&'a CapsuleColliderSpec),
}

impl<'a> ColliderRef<'a> {
    fn id(self) -> &'a str {
        match self {
            Self::Box(collider) => &collider.collider_id,
            Self::Sphere(collider) => &collider.collider_id,
            Self::Capsule(collider) => &collider.collider_id,
        }
    }

    fn local_pose(self) -> Pose {
        match self {
            Self::Box(collider) => collider.local_pose,
            Self::Sphere(collider) => collider.local_pose,
            Self::Capsule(collider) => collider.local_pose,
        }
    }
}

fn ordered_colliders(spec: &BodySpec) -> Vec<ColliderRef<'_>> {
    let mut colliders = spec
        .colliders
        .iter()
        .map(ColliderRef::Box)
        .chain(spec.sphere_colliders.iter().map(ColliderRef::Sphere))
        .chain(spec.capsule_colliders.iter().map(ColliderRef::Capsule))
        .collect::<Vec<_>>();
    colliders.sort_by(|left, right| left.id().cmp(right.id()));
    colliders
}

fn create_box(
    body: &BodySpec,
    collider: &BoxColliderSpec,
    index: usize,
) -> Result<ShapeRef, PhysicsError> {
    let settings = JPC_BoxShapeSettings {
        UserData: u64::try_from(index + 1).unwrap_or(u64::MAX),
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

fn create_sphere(
    body: &BodySpec,
    collider: &SphereColliderSpec,
    index: usize,
) -> Result<ShapeRef, PhysicsError> {
    let settings = JPC_SphereShapeSettings {
        UserData: u64::try_from(index + 1).unwrap_or(u64::MAX),
        Density: collider.density_kg_per_m3,
        Radius: collider.radius,
    };
    let mut shape = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: Output pointers refer to local variables valid for the call;
    // settings contains no borrowed pointers. Success returns one owned ref.
    let created = unsafe {
        JPC_SphereShapeSettings_Create(
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

fn create_capsule(
    body: &BodySpec,
    collider: &CapsuleColliderSpec,
    index: usize,
) -> Result<ShapeRef, PhysicsError> {
    let settings = JPC_CapsuleShapeSettings {
        UserData: u64::try_from(index + 1).unwrap_or(u64::MAX),
        Density: collider.density_kg_per_m3,
        Radius: collider.radius,
        HalfHeightOfCylinder: collider.half_height_of_cylinder,
    };
    let mut shape = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: Output pointers refer to local variables valid for the call;
    // settings contains no borrowed pointers. Success returns one owned ref.
    let created = unsafe {
        JPC_CapsuleShapeSettings_Create(
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

fn create_query_capsule(query: &CapsuleCast) -> Result<ShapeRef, PhysicsError> {
    let settings = JPC_CapsuleShapeSettings {
        UserData: 0,
        Density: 1_000.0,
        Radius: query.radius,
        HalfHeightOfCylinder: query.half_height_of_cylinder,
    };
    let mut shape = ptr::null_mut();
    let mut error = ptr::null_mut();
    // SAFETY: Output pointers refer to live locals and successful creation
    // returns one owned shape reference released by `ShapeRef`.
    let created = unsafe {
        JPC_CapsuleShapeSettings_Create(
            ptr::from_ref(&settings),
            ptr::from_mut(&mut shape),
            ptr::from_mut(&mut error),
        )
    };
    if created && !shape.is_null() {
        Ok(ShapeRef(shape))
    } else {
        Err(PhysicsError::CapsuleCastFailed(take_error(error)))
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

fn local_vec4(value: Vec3, w: f32) -> JPC_Vec4 {
    JPC_Vec4 {
        x: value.x as f32,
        y: value.y as f32,
        z: value.z as f32,
        w,
    }
}

#[allow(clippy::needless_update)]
fn world_transform(pose: Pose) -> joltc_sys::JPC_RMat44 {
    // SAFETY: JPC_RMat44 contains only scalar columns plus target-specific
    // padding. Zeroing the padding is the representation used by JoltC's own
    // Rust examples before the initialized columns are assigned.
    unsafe {
        joltc_sys::JPC_RMat44 {
            col: [
                local_vec4(pose.rotation.rotate(Vec3::new(1.0, 0.0, 0.0)), 0.0),
                local_vec4(pose.rotation.rotate(Vec3::new(0.0, 1.0, 0.0)), 0.0),
                local_vec4(pose.rotation.rotate(Vec3::new(0.0, 0.0, 1.0)), 0.0),
            ],
            col3: world_vec3(pose.position),
            ..std::mem::zeroed()
        }
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
}

fn validated_body_state(
    body_id: &str,
    position: Vec3,
    rotation: Quat,
    linear_velocity: Vec3,
    angular_velocity: Vec3,
    active: bool,
    config: &SceneConfig,
) -> Result<BodyState, PhysicsError> {
    for (field, value) in [
        ("position", position),
        ("linear velocity", linear_velocity),
        ("angular velocity", angular_velocity),
    ] {
        if !value.is_finite() {
            return Err(PhysicsError::BodyStateInvalid {
                body_id: body_id.into(),
                field,
            });
        }
    }
    let rotation =
        crate::validated_rotation(rotation).ok_or_else(|| PhysicsError::BodyStateInvalid {
            body_id: body_id.into(),
            field: "rotation",
        })?;
    let linear_limit = f64::from(config.max_linear_velocity_mps);
    let angular_limit = f64::from(config.max_angular_velocity_radians_per_second);
    let linear_tolerance = linear_limit * f64::from(f32::EPSILON) * 8.0 + 1.0e-9;
    let angular_tolerance = angular_limit * f64::from(f32::EPSILON) * 8.0 + 1.0e-9;
    if linear_velocity.length() > linear_limit + linear_tolerance {
        return Err(PhysicsError::BodyStateInvalid {
            body_id: body_id.into(),
            field: "linear velocity bound",
        });
    }
    if angular_velocity.length() > angular_limit + angular_tolerance {
        return Err(PhysicsError::BodyStateInvalid {
            body_id: body_id.into(),
            field: "angular velocity bound",
        });
    }
    Ok(BodyState {
        body_id: body_id.into(),
        pose: Pose::new(position, rotation),
        linear_velocity: linear_velocity.clamped_length(linear_limit),
        angular_velocity: angular_velocity.clamped_length(angular_limit),
        active,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_budget_covers_every_internal_collision_step() {
        let config = SceneConfig {
            collision_substeps: 2,
            max_contact_constraints: 16_384,
            ..SceneConfig::default()
        };
        assert_eq!(contact_record_budget(&config), Ok(32_768));
    }

    #[test]
    fn estimated_impulse_conversion_rejects_corrupt_values() {
        assert_eq!(accumulate_estimated_normal_impulse(2.0, 3.0), Some(5.0));
        assert_eq!(accumulate_estimated_normal_impulse(2.0, -0.1), None);
        assert_eq!(accumulate_estimated_normal_impulse(2.0, f64::NAN), None);
        assert_eq!(
            accumulate_estimated_normal_impulse(f64::MAX, f64::MAX),
            None
        );
    }

    #[test]
    fn callback_failure_retains_the_first_named_invariant() {
        let listener = NativeContactListener::new(4).expect("small callback buffer allocates");
        listener.fail(ContactInvariant::InvalidNormal);
        listener.fail(ContactInvariant::CallbackPanicked);
        assert_eq!(listener.failure(), Some(ContactInvariant::InvalidNormal));
    }

    #[test]
    fn body_state_extraction_rejects_corrupt_native_values() {
        let non_finite = validated_body_state(
            "grid",
            Vec3::new(f64::NAN, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ZERO,
            Vec3::ZERO,
            true,
            &SceneConfig::default(),
        );
        assert!(matches!(
            non_finite,
            Err(PhysicsError::BodyStateInvalid {
                field: "position",
                ..
            })
        ));

        let invalid_rotation = validated_body_state(
            "grid",
            Vec3::ZERO,
            Quat::new(0.0, 0.0, 0.0, 0.0),
            Vec3::ZERO,
            Vec3::ZERO,
            true,
            &SceneConfig::default(),
        );
        assert!(matches!(
            invalid_rotation,
            Err(PhysicsError::BodyStateInvalid {
                field: "rotation",
                ..
            })
        ));

        let over_speed = validated_body_state(
            "grid",
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(1_001.0, 0.0, 0.0),
            Vec3::ZERO,
            true,
            &SceneConfig::default(),
        );
        assert!(matches!(
            over_speed,
            Err(PhysicsError::BodyStateInvalid {
                field: "linear velocity bound",
                ..
            })
        ));

        let config = SceneConfig::default();
        let angular_limit = f64::from(config.max_angular_velocity_radians_per_second);
        let axis = config.max_angular_velocity_radians_per_second / 2.0_f32.sqrt();
        let rounded_up_axis = f32::from_bits(axis.to_bits() + 1);
        let reconstructed = Vec3::new(f64::from(rounded_up_axis), f64::from(rounded_up_axis), 0.0);
        assert!(reconstructed.length() > angular_limit);
        let tolerated_roundoff = validated_body_state(
            "player",
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
            reconstructed,
            true,
            &config,
        )
        .expect("f32-native roundoff within the tight tolerance is accepted");
        assert!(tolerated_roundoff.angular_velocity.length() <= angular_limit);

        let material_overspeed = validated_body_state(
            "player",
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::ZERO,
            Vec3::new(angular_limit + 0.001, 0.0, 0.0),
            true,
            &config,
        );
        assert!(matches!(
            material_overspeed,
            Err(PhysicsError::BodyStateInvalid {
                field: "angular velocity bound",
                ..
            })
        ));
    }
}
