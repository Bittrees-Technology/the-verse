// SPDX-License-Identifier: AGPL-3.0-or-later

use verse_physics::{
    BodyControl, BodySpec, BodyState, BoxColliderSpec, ContactPhase, ContactSource, PhysicsError,
    Pose, Quat, Scene, SceneConfig, StepOutput, Vec3,
};

const P0_TOTAL_MOMENTUM_ERROR_KG_MPS: f64 = 1.0;

fn pose(x: f64, y: f64, z: f64) -> Pose {
    Pose::new(Vec3::new(x, y, z), Quat::IDENTITY)
}

fn body_y(scene: &mut Scene, body_id: &str) -> f64 {
    scene
        .body_states()
        .expect("native body states are valid")
        .into_iter()
        .find(|body| body.body_id == body_id)
        .expect("test body exists")
        .pose
        .position
        .y
}

fn output_body<'a>(output: &'a StepOutput, body_id: &str) -> &'a BodyState {
    output
        .bodies
        .iter()
        .find(|body| body.body_id == body_id)
        .expect("test body exists in step output")
}

#[test]
fn scene_can_move_between_authoritative_worker_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<Scene>();

    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    scene
        .rebuild(&[BodySpec::static_body(
            "threaded-grid",
            Pose::IDENTITY,
            vec![BoxColliderSpec::unit_cube("threaded-block")],
        )])
        .expect("body builds");
    let count = std::thread::spawn(move || scene.body_count())
        .join()
        .expect("scene moves to worker and drops there");
    assert_eq!(count, 1);
}

#[test]
fn falling_box_contacts_static_floor_and_settles() {
    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    let floor = BodySpec::static_body(
        "floor",
        pose(0.0, -0.5, 0.0),
        vec![BoxColliderSpec {
            collider_id: "floor-panel".into(),
            half_extents: Vec3::new(10.0, 0.5, 10.0),
            ..BoxColliderSpec::unit_cube("ignored")
        }],
    );
    let mut falling = BodySpec::dynamic(
        "falling-grid",
        pose(0.0, 2.0, 0.0),
        vec![BoxColliderSpec::unit_cube("armor-block")],
    );
    falling.gravity_factor = 1.0;
    scene.rebuild(&[falling, floor]).expect("bodies build");

    let mut observed_contact = None;
    for _ in 0..240 {
        let output = scene.step(&[]).expect("fixed step succeeds");
        if let Some(contact) = output.contacts.first() {
            observed_contact = Some(contact.clone());
        }
    }

    let contact = observed_contact.expect("falling box reports floor contact");
    assert_eq!(contact.body_a_id, "falling-grid");
    assert_eq!(contact.collider_a_id, "armor-block");
    assert_eq!(contact.body_b_id, "floor");
    assert_eq!(contact.collider_b_id, "floor-panel");
    assert_eq!(contact.source, ContactSource::JoltEstimatedResponse);
    assert!(contact.normal.y < -0.9, "normal points from box to floor");
    assert!(contact.point.is_finite());
    assert!(contact.penetration_m >= 0.0);
    assert!(matches!(
        contact.phase,
        ContactPhase::Began | ContactPhase::Persisted
    ));
    assert!(
        (0.45..=0.55).contains(&body_y(&mut scene, "falling-grid")),
        "Jolt should settle the cube on the floor"
    );

    let mut resting_origin = None;
    let mut maximum_translation_drift: f64 = 0.0;
    let mut maximum_speed: f64 = 0.0;
    for _ in 0..240 {
        let output = scene.step(&[]).expect("resting stability step succeeds");
        let body = output_body(&output, "falling-grid");
        let origin = *resting_origin.get_or_insert(body.pose.position);
        maximum_translation_drift =
            maximum_translation_drift.max((body.pose.position - origin).length());
        maximum_speed = maximum_speed.max(body.linear_velocity.length());
    }
    assert!(
        maximum_translation_drift <= 1.0e-4,
        "resting grid translation drift exceeded 0.1 mm: {maximum_translation_drift}"
    );
    assert!(
        maximum_speed <= 1.0e-3,
        "resting grid exceeded 1 mm/s: {maximum_speed}"
    );
}

#[test]
fn moving_dynamic_boxes_collide_and_emit_sorted_stable_ids() {
    let config = SceneConfig {
        fixed_delta_seconds: 1.0 / 120.0,
        collision_substeps: 2,
        ..SceneConfig::default()
    };
    let mut scene = Scene::new(config).expect("scene initializes");

    let mut alpha = BodySpec::dynamic(
        "alpha-grid",
        pose(-2.0, 0.0, 0.0),
        vec![BoxColliderSpec::unit_cube("alpha-armor")],
    );
    alpha.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
    let mut zeta = BodySpec::dynamic(
        "zeta-grid",
        pose(2.0, 0.0, 0.0),
        vec![BoxColliderSpec::unit_cube("zeta-armor")],
    );
    zeta.linear_velocity = Vec3::new(-4.0, 0.0, 0.0);

    // Deliberately reverse input order: output identity order is canonical.
    scene.rebuild(&[zeta, alpha]).expect("bodies build");
    let mut observed_contact = None;
    for _ in 0..180 {
        let output = scene.step(&[]).expect("fixed step succeeds");
        assert_eq!(output.bodies[0].body_id, "alpha-grid");
        assert_eq!(output.bodies[1].body_id, "zeta-grid");
        if let Some(contact) = output.contacts.first() {
            observed_contact = Some(contact.clone());
            break;
        }
    }

    let contact = observed_contact.expect("moving dynamic bodies report contact");
    assert_eq!(
        (
            contact.body_a_id.as_str(),
            contact.collider_a_id.as_str(),
            contact.body_b_id.as_str(),
            contact.collider_b_id.as_str(),
        ),
        ("alpha-grid", "alpha-armor", "zeta-grid", "zeta-armor")
    );
    assert!(contact.normal.x > 0.9);
    assert!(contact.impact_speed_mps > 7.0);
    assert!(contact.estimated_normal_impulse_ns > 0.0);
}

#[test]
fn equal_dynamic_grids_exchange_momentum_within_the_p0_tolerance() {
    let config = SceneConfig {
        fixed_delta_seconds: 1.0 / 120.0,
        collision_substeps: 2,
        ..SceneConfig::default()
    };
    let mut scene = Scene::new(config).expect("scene initializes");
    let mut alpha = BodySpec::dynamic(
        "alpha-grid",
        pose(-2.0, 0.0, 0.0),
        vec![BoxColliderSpec::unit_cube("alpha-armor")],
    );
    alpha.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
    alpha.friction = 0.62;
    alpha.restitution = 0.05;
    alpha.allow_sleeping = false;
    let mut zeta = BodySpec::dynamic(
        "zeta-grid",
        pose(2.0, 0.0, 0.0),
        vec![BoxColliderSpec::unit_cube("zeta-armor")],
    );
    zeta.linear_velocity = Vec3::new(-4.0, 0.0, 0.0);
    zeta.friction = 0.62;
    zeta.restitution = 0.05;
    zeta.allow_sleeping = false;
    scene.rebuild(&[alpha, zeta]).expect("grids build");

    let collision_output = (0..180)
        .find_map(|_| {
            let output = scene.step(&[]).expect("fixed step succeeds");
            (!output.contacts.is_empty()).then_some(output)
        })
        .expect("dynamic grids collide");
    let alpha = output_body(&collision_output, "alpha-grid");
    let zeta = output_body(&collision_output, "zeta-grid");
    assert!(alpha.linear_velocity.x < 0.0, "alpha must recoil");
    assert!(zeta.linear_velocity.x > 0.0, "zeta must recoil");

    let total_momentum_x = 1_000.0 * alpha.linear_velocity.x + 1_000.0 * zeta.linear_velocity.x;
    assert!(
        total_momentum_x.abs() <= P0_TOTAL_MOMENTUM_ERROR_KG_MPS,
        "total momentum error exceeded the published P0 tolerance: {total_momentum_x} kg m/s"
    );
}

#[test]
fn collider_density_changes_collision_mass_response() {
    let config = SceneConfig {
        fixed_delta_seconds: 1.0 / 120.0,
        collision_substeps: 2,
        ..SceneConfig::default()
    };
    let mut scene = Scene::new(config).expect("scene initializes");
    let mut light = BodySpec::dynamic(
        "light-grid",
        pose(-2.0, 0.0, 0.0),
        vec![BoxColliderSpec {
            density_kg_per_m3: 100.0,
            ..BoxColliderSpec::unit_cube("light-armor")
        }],
    );
    light.linear_velocity = Vec3::new(6.0, 0.0, 0.0);
    light.friction = 0.0;
    light.restitution = 1.0;
    light.allow_sleeping = false;
    let mut heavy = BodySpec::dynamic(
        "heavy-grid",
        pose(2.0, 0.0, 0.0),
        vec![BoxColliderSpec {
            density_kg_per_m3: 1_000.0,
            ..BoxColliderSpec::unit_cube("heavy-armor")
        }],
    );
    heavy.friction = 0.0;
    heavy.restitution = 1.0;
    heavy.allow_sleeping = false;
    scene.rebuild(&[heavy, light]).expect("grids build");

    let collision_output = (0..180)
        .find_map(|_| {
            let output = scene.step(&[]).expect("fixed step succeeds");
            (!output.contacts.is_empty()).then_some(output)
        })
        .expect("unequal grids collide");
    let light = output_body(&collision_output, "light-grid");
    let heavy = output_body(&collision_output, "heavy-grid");
    assert!(light.linear_velocity.x < -3.0, "light body must rebound");
    assert!(
        (0.5..1.5).contains(&heavy.linear_velocity.x),
        "heavy body must respond less than the light body: {}",
        heavy.linear_velocity.x
    );
}

#[test]
fn static_anchor_remains_exact_under_control_and_contact() {
    let config = SceneConfig {
        fixed_delta_seconds: 1.0 / 120.0,
        collision_substeps: 2,
        ..SceneConfig::default()
    };
    let mut scene = Scene::new(config).expect("scene initializes");
    let anchor = BodySpec::static_body(
        "anchored-grid",
        Pose::IDENTITY,
        vec![BoxColliderSpec::unit_cube("anchor-block")],
    );
    let mut striker = BodySpec::dynamic(
        "striker-grid",
        pose(-3.0, 0.0, 0.0),
        vec![BoxColliderSpec::unit_cube("striker-block")],
    );
    striker.linear_velocity = Vec3::new(8.0, 0.0, 0.0);
    striker.allow_sleeping = false;
    scene.rebuild(&[anchor, striker]).expect("grids build");

    let control_error = scene
        .step(&[BodyControl {
            body_id: "anchored-grid".into(),
            force_newtons: Vec3::new(1.0, 0.0, 0.0),
            torque_newton_meters: Vec3::ZERO,
        }])
        .expect_err("static bodies reject control");
    assert_eq!(
        control_error,
        PhysicsError::ControlBodyStatic("anchored-grid".into())
    );

    let mut observed_contact = false;
    let mut latest = None;
    for _ in 0..240 {
        let output = scene.step(&[]).expect("impact step succeeds");
        observed_contact |= !output.contacts.is_empty();
        latest = Some(output);
    }
    assert!(observed_contact, "striker must contact anchored grid");
    let output = latest.expect("steps produce output");
    let anchor = output_body(&output, "anchored-grid");
    assert_eq!(anchor.pose, Pose::IDENTITY);
    assert_eq!(anchor.linear_velocity, Vec3::ZERO);
    assert_eq!(anchor.angular_velocity, Vec3::ZERO);
}

#[test]
fn compound_colliders_keep_sorted_stable_identity() {
    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    let mut moving = BodySpec::dynamic(
        "ship",
        pose(-2.0, 0.0, 0.0),
        vec![
            BoxColliderSpec {
                collider_id: "right-block".into(),
                local_pose: pose(0.6, 0.0, 0.0),
                ..BoxColliderSpec::unit_cube("ignored")
            },
            BoxColliderSpec {
                collider_id: "left-block".into(),
                local_pose: pose(-0.6, 0.0, 0.0),
                ..BoxColliderSpec::unit_cube("ignored")
            },
        ],
    );
    moving.linear_velocity = Vec3::new(4.0, 0.0, 0.0);
    let wall = BodySpec::static_body(
        "wall",
        pose(0.0, 0.0, 0.0),
        vec![BoxColliderSpec::unit_cube("wall-block")],
    );
    scene.rebuild(&[wall, moving]).expect("compound builds");

    let mut collider_ids = Vec::new();
    for _ in 0..120 {
        let output = scene.step(&[]).expect("fixed step succeeds");
        collider_ids.extend(
            output
                .contacts
                .into_iter()
                .map(|contact| contact.collider_a_id),
        );
    }
    assert!(collider_ids.iter().any(|id| id == "right-block"));
}

#[test]
fn rebuild_is_atomic_and_controls_are_bounded() {
    let config = SceneConfig {
        max_force_newtons: 10.0,
        ..SceneConfig::default()
    };
    let mut scene = Scene::new(config).expect("scene initializes");
    let body = BodySpec::dynamic(
        "controlled-grid",
        Pose::IDENTITY,
        vec![BoxColliderSpec::unit_cube("block")],
    );
    scene
        .rebuild(std::slice::from_ref(&body))
        .expect("initial body builds");

    let mut invalid = body;
    invalid.colliders[0].half_extents.x = -1.0;
    assert!(scene.rebuild(&[invalid]).is_err());
    assert_eq!(scene.body_count(), 1, "failed rebuild preserves old scene");

    let error = scene
        .step(&[BodyControl {
            body_id: "controlled-grid".into(),
            force_newtons: Vec3::new(10.01, 0.0, 0.0),
            torque_newton_meters: Vec3::ZERO,
        }])
        .expect_err("out-of-bounds control is rejected before native mutation");
    assert!(matches!(error, PhysicsError::ControlOutOfBounds { .. }));
}

#[test]
fn one_body_replacement_preserves_unrelated_bodies_and_can_remove_the_final_chunk() {
    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    let target = BodySpec::static_body(
        "voxel-chunk-0-0-0",
        Pose::IDENTITY,
        vec![
            BoxColliderSpec::unit_cube("voxel-0-0-0"),
            BoxColliderSpec {
                local_pose: pose(1.0, 0.0, 0.0),
                ..BoxColliderSpec::unit_cube("voxel-1-0-0")
            },
        ],
    );
    let untouched = BodySpec::static_body(
        "voxel-chunk-1-0-0",
        pose(8.0, 0.0, 0.0),
        vec![BoxColliderSpec::unit_cube("voxel-8-0-0")],
    );
    let mut dynamic = BodySpec::dynamic(
        "moving-grid",
        pose(4.0, 2.0, 0.0),
        vec![BoxColliderSpec::unit_cube("moving-block")],
    );
    dynamic.linear_velocity = Vec3::new(0.5, -0.25, 0.0);
    dynamic.allow_sleeping = false;
    scene
        .rebuild(&[target.clone(), untouched, dynamic])
        .expect("chunked scene builds");
    let before = scene.body_states().expect("body state extracts");

    let replacement = BodySpec::static_body(
        target.body_id.clone(),
        target.pose,
        vec![target.colliders[0].clone()],
    );
    scene
        .replace_body(&target.body_id, Some(replacement))
        .expect("one chunk replaces");
    assert_eq!(scene.body_count(), 3);
    assert!(scene.contains_collider(&target.body_id, "voxel-0-0-0"));
    assert!(!scene.contains_collider(&target.body_id, "voxel-1-0-0"));
    assert_eq!(scene.body_states().expect("states remain exact"), before);

    let mut invalid = target.clone();
    invalid.colliders[0].half_extents.x = -1.0;
    assert!(scene.replace_body(&target.body_id, Some(invalid)).is_err());
    assert!(scene.contains_collider(&target.body_id, "voxel-0-0-0"));
    scene
        .step(&[])
        .expect("validation failure keeps scene usable");

    scene
        .replace_body(&target.body_id, None)
        .expect("empty final chunk removes its body");
    assert_eq!(scene.body_count(), 2);
    assert!(!scene.contains_collider(&target.body_id, "voxel-0-0-0"));
}

#[test]
fn world_position_is_double_precision() {
    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    let position = Vec3::new(1_000_000_000.125, -2_000_000_000.25, 3_000_000_000.5);
    let body = BodySpec::static_body(
        "far-grid",
        Pose::new(position, Quat::IDENTITY),
        vec![BoxColliderSpec::unit_cube("far-block")],
    );
    scene.rebuild(&[body]).expect("far body builds");
    assert_eq!(
        scene.body_states().expect("native body state is valid")[0]
            .pose
            .position,
        position
    );
}
