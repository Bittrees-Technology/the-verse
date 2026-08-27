// SPDX-License-Identifier: AGPL-3.0-or-later

use std::time::{Duration, Instant};

use serde::Serialize;
use tempfile::tempdir;
use verse_protocol::{BlockKind, ClientMessage, IVec3, ResourceKind, Vec3};
use verse_simulation::Runtime;

#[derive(Debug, Serialize)]
struct LatencySummary {
    samples: usize,
    mean_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    max_ms: f64,
}

#[derive(Debug, Serialize)]
struct BaselineReport {
    report_version: u32,
    platform_os: &'static str,
    platform_arch: &'static str,
    content_manifest_version: String,
    initial_voxels: usize,
    snapshot_json_bytes: usize,
    startup_ms: f64,
    mining: LatencySummary,
    grid_split_ms: f64,
    recovered_events: u64,
    recovery_ms: f64,
    recovery_hash_matched: bool,
    conservation_valid: bool,
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn summarize(mut samples: Vec<Duration>) -> LatencySummary {
    samples.sort_unstable();
    let count = samples.len();
    let percentile = |numerator: usize| {
        let index = (count.saturating_sub(1) * numerator) / 100;
        milliseconds(samples[index])
    };
    LatencySummary {
        samples: count,
        mean_ms: samples.iter().copied().map(milliseconds).sum::<f64>() / count as f64,
        p50_ms: percentile(50),
        p95_ms: percentile(95),
        max_ms: milliseconds(*samples.last().expect("benchmark has samples")),
    }
}

fn move_player(
    runtime: &mut Runtime,
    target: Vec3,
    sequence: &mut u64,
) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..240 {
        let current = runtime.state().player.position;
        let delta = Vec3::new(
            target.x - current.x,
            target.y - current.y,
            target.z - current.z,
        );
        if delta.magnitude() <= 0.5 {
            runtime.execute(&ClientMessage::SetPlayerControl {
                operation_id: format!("benchmark-control-{sequence}"),
                movement_epoch: runtime.state().player.movement_epoch,
                input_sequence: *sequence + 1,
                linear_input: Vec3::ZERO,
                angular_input: Vec3::ZERO,
                boost: false,
                dampeners: true,
            })?;
            *sequence += 1;
            runtime.advance(100)?;
            return Ok(());
        }
        let direction = delta * (1.0 / delta.magnitude());
        runtime.execute(&ClientMessage::SetPlayerControl {
            operation_id: format!("benchmark-control-{sequence}"),
            movement_epoch: runtime.state().player.movement_epoch,
            input_sequence: *sequence + 1,
            linear_input: direction,
            angular_input: Vec3::ZERO,
            boost: true,
            dampeners: true,
        })?;
        *sequence += 1;
        runtime.advance(100)?;
    }
    Err("authoritative player movement did not converge".into())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempdir()?;
    let startup_started = Instant::now();
    let mut runtime = Runtime::open(directory.path(), 42, 10_000)?;
    let startup = startup_started.elapsed();
    let initial_voxels = runtime.state().voxels.occupied.len();
    let snapshot_json_bytes = serde_json::to_vec(&runtime.snapshot())?.len();

    let mut mining_move_sequence = 0;
    move_player(
        &mut runtime,
        Vec3::new(10.0, 0.0, 3.0),
        &mut mining_move_sequence,
    )?;
    let mut mining_samples = Vec::new();
    let cargo_inventory_id = runtime
        .state()
        .inventories
        .keys()
        .find(|inventory_id| inventory_id.contains("cargo"))
        .cloned()
        .ok_or("benchmark cargo inventory is missing")?;
    for index in 0..100_u64 {
        let target = runtime
            .state()
            .voxels
            .occupied
            .iter()
            .copied()
            .find(|coordinate| {
                let position = Vec3::new(
                    f64::from(coordinate.x),
                    f64::from(coordinate.y),
                    f64::from(coordinate.z),
                );
                runtime.state().player.position.squared_distance(position) <= 8.5 * 8.5
            })
            .ok_or("benchmark exhausted reachable voxels")?;
        let started = Instant::now();
        runtime.execute(&ClientMessage::MineVoxel {
            operation_id: format!("benchmark-mine-{index}"),
            coordinate: target,
        })?;
        mining_samples.push(started.elapsed());
        let mined_quantity = runtime.state().inventories["inventory-player-local"]
            .contents
            .ore;
        runtime.execute(&ClientMessage::TransferInventory {
            operation_id: format!("benchmark-offload-{index}"),
            source_inventory_id: "inventory-player-local".into(),
            destination_inventory_id: cargo_inventory_id.clone(),
            resource: ResourceKind::Ore,
            quantity: mined_quantity,
        })?;
    }
    runtime.persist_snapshot()?;
    let expected_hash = runtime.state().state_hash();
    let recovered_events = runtime.state().event_sequence;
    drop(runtime);

    let recovery_started = Instant::now();
    let recovered = Runtime::open(directory.path(), 42, 10_000)?;
    let recovery = recovery_started.elapsed();
    let recovery_hash_matched = recovered.state().state_hash() == expected_hash;
    let conservation_valid = recovered.state().conservation().valid;

    let split_directory = tempdir()?;
    let mut split_runtime = Runtime::open(split_directory.path(), 84, 10_000)?;
    let mut move_sequence = 0;
    for y in 1..=20 {
        move_player(
            &mut split_runtime,
            Vec3::new(10.0, f64::from(y), 3.0),
            &mut move_sequence,
        )?;
        split_runtime.execute(&ClientMessage::BuildBlock {
            operation_id: format!("benchmark-build-{y}"),
            grid_id: "grid-starter".into(),
            coordinate: IVec3::new(0, y, 0),
            kind: if y == 10 {
                BlockKind::DamageTest
            } else {
                BlockKind::Structural
            },
            orientation: 0,
        })?;
        let block_id = split_runtime.state().grids["grid-starter"]
            .block_at(IVec3::new(0, y, 0))
            .expect("construction frame exists")
            .block_id
            .clone();
        for stage in 0..3 {
            split_runtime.execute(&ClientMessage::WeldBlock {
                operation_id: format!("benchmark-weld-{y}-{stage}"),
                grid_id: "grid-starter".into(),
                block_id: block_id.clone(),
            })?;
        }
    }
    let bridge_id = split_runtime.state().grids["grid-starter"]
        .block_at(IVec3::new(0, 10, 0))
        .expect("bridge exists")
        .block_id
        .clone();
    move_player(
        &mut split_runtime,
        Vec3::new(10.0, 10.0, 3.0),
        &mut move_sequence,
    )?;
    split_runtime.execute(&ClientMessage::DamageBlock {
        operation_id: "benchmark-damage-1".into(),
        grid_id: "grid-starter".into(),
        block_id: bridge_id.clone(),
    })?;
    let split_started = Instant::now();
    split_runtime.execute(&ClientMessage::DamageBlock {
        operation_id: "benchmark-damage-2".into(),
        grid_id: "grid-starter".into(),
        block_id: bridge_id,
    })?;
    let grid_split = split_started.elapsed();
    assert_eq!(split_runtime.state().grids.len(), 2);

    let report = BaselineReport {
        report_version: 1,
        platform_os: std::env::consts::OS,
        platform_arch: std::env::consts::ARCH,
        content_manifest_version: recovered.state().content_manifest_version.clone(),
        initial_voxels,
        snapshot_json_bytes,
        startup_ms: milliseconds(startup),
        mining: summarize(mining_samples),
        grid_split_ms: milliseconds(grid_split),
        recovered_events,
        recovery_ms: milliseconds(recovery),
        recovery_hash_matched,
        conservation_valid,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
