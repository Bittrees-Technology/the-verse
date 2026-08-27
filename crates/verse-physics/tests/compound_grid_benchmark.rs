// SPDX-License-Identifier: AGPL-3.0-or-later

use std::time::Instant;

use verse_physics::{BodySpec, BoxColliderSpec, Pose, Quat, Scene, SceneConfig, Vec3};

#[test]
#[ignore = "focused performance probe; run with --ignored --nocapture"]
fn steps_against_two_thousand_voxel_static_compound() {
    let mut floor_colliders = Vec::with_capacity(2_048);
    for z in 0..32 {
        for x in 0..64 {
            floor_colliders.push(BoxColliderSpec {
                collider_id: format!("voxel-{z:02}-{x:02}"),
                local_pose: Pose::new(
                    Vec3::new(f64::from(x) - 31.5, 0.0, f64::from(z) - 15.5),
                    Quat::IDENTITY,
                ),
                ..BoxColliderSpec::unit_cube("ignored")
            });
        }
    }
    let floor = BodySpec::static_body(
        "voxel-grid",
        Pose::new(Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY),
        floor_colliders,
    );
    let mut falling = BodySpec::dynamic(
        "falling-grid",
        Pose::new(Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY),
        vec![BoxColliderSpec::unit_cube("moving-block")],
    );
    falling.gravity_factor = 1.0;

    let started_build = Instant::now();
    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    scene.rebuild(&[floor, falling]).expect("compound builds");
    let build_elapsed = started_build.elapsed();

    let started_steps = Instant::now();
    let mut observed_contact = false;
    for _ in 0..120 {
        let output = scene.step(&[]).expect("fixed step succeeds");
        observed_contact |= !output.contacts.is_empty();
    }
    let step_elapsed = started_steps.elapsed();
    assert!(observed_contact, "broad-phase gate preserves floor contact");

    eprintln!(
        "2,048-collider build={build_elapsed:?}, 120 steps={step_elapsed:?}, per-step={:?}",
        step_elapsed / 120
    );
}

#[test]
#[ignore = "focused performance probe; run with --ignored --nocapture"]
fn starter_grid_against_proof_asteroid_budget() {
    let mut asteroid_colliders = Vec::with_capacity(2_816);
    for z in 0..44 {
        for x in 0..64 {
            asteroid_colliders.push(BoxColliderSpec {
                collider_id: format!("voxel-{z:02}-{x:02}"),
                local_pose: Pose::new(
                    Vec3::new(f64::from(x) - 31.5, 0.0, f64::from(z) - 21.5),
                    Quat::IDENTITY,
                ),
                ..BoxColliderSpec::unit_cube("ignored")
            });
        }
    }
    let asteroid = BodySpec::static_body(
        "proof-asteroid",
        Pose::new(Vec3::new(0.0, -0.5, 0.0), Quat::IDENTITY),
        asteroid_colliders,
    );
    let mut grid_colliders = Vec::with_capacity(25);
    for z in 0..5 {
        for x in 0..5 {
            grid_colliders.push(BoxColliderSpec {
                collider_id: format!("starter-block-{z}-{x}"),
                local_pose: Pose::new(
                    Vec3::new(f64::from(x) - 2.0, 0.0, f64::from(z) - 2.0),
                    Quat::IDENTITY,
                ),
                ..BoxColliderSpec::unit_cube("ignored")
            });
        }
    }
    let mut starter = BodySpec::dynamic(
        "starter-grid",
        Pose::new(Vec3::new(0.0, 2.0, 0.0), Quat::IDENTITY),
        grid_colliders,
    );
    starter.gravity_factor = 1.0;

    let started_build = Instant::now();
    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    scene
        .rebuild(&[asteroid, starter])
        .expect("proof scene builds");
    let build_elapsed = started_build.elapsed();

    let started_steps = Instant::now();
    let mut observed_contact = false;
    for _ in 0..120 {
        let output = scene.step(&[]).expect("fixed step succeeds");
        observed_contact |= !output.contacts.is_empty();
    }
    let step_elapsed = started_steps.elapsed();
    assert!(
        observed_contact,
        "broad-phase gate preserves compound contact"
    );

    eprintln!(
        "2,816-voxel/25-block build={build_elapsed:?}, 120 steps={step_elapsed:?}, per-step={:?}",
        step_elapsed / 120
    );
}

#[test]
#[ignore = "focused performance probe; run with --ignored --nocapture"]
fn dirty_collision_chunk_replacement_distribution() {
    let mut chunks = Vec::with_capacity(32);
    for chunk in 0..32 {
        let mut colliders = Vec::with_capacity(88);
        for index in 0..88 {
            colliders.push(BoxColliderSpec {
                collider_id: format!("voxel-{chunk}-{index}"),
                local_pose: Pose::new(
                    Vec3::new(
                        f64::from(index % 8),
                        f64::from((index / 8) % 8),
                        f64::from(index / 64),
                    ),
                    Quat::IDENTITY,
                ),
                ..BoxColliderSpec::unit_cube("ignored")
            });
        }
        chunks.push(BodySpec::static_body(
            format!("voxel-chunk-{chunk}-0-0"),
            Pose::new(Vec3::new(f64::from(chunk * 8), 0.0, 0.0), Quat::IDENTITY),
            colliders,
        ));
    }
    let target_id = chunks[0].body_id.clone();
    let untouched_before = chunks[1].clone();
    let mut replacement = chunks[0].clone();
    let mut scene = Scene::new(SceneConfig::default()).expect("scene initializes");
    scene.rebuild(&chunks).expect("chunked asteroid builds");

    let mut samples = Vec::with_capacity(64);
    for _ in 0..64 {
        replacement.colliders.pop();
        let started = Instant::now();
        scene
            .replace_body(&target_id, Some(replacement.clone()))
            .expect("dirty chunk replaces");
        samples.push(started.elapsed());
    }
    samples.sort();
    let median = samples[samples.len() / 2];
    let p95 = samples[samples.len() * 95 / 100];
    let maximum = *samples.last().expect("samples exist");
    assert_eq!(scene.body_count(), 32);
    assert!(scene.contains_collider(
        &untouched_before.body_id,
        &untouched_before.colliders[0].collider_id
    ));
    eprintln!(
        "32 chunks/2,816 leaves, 64 single-chunk replacements: median={median:?}, p95={p95:?}, max={maximum:?}"
    );
}
