// SPDX-License-Identifier: AGPL-3.0-or-later

use verse_physics::{
    BodyControl, BodySpec, BoxColliderSpec, ContactPhase, ContactSource, PhysicsError, Pose, Quat,
    Scene, SceneConfig, Vec3,
};

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
