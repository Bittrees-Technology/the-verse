// SPDX-License-Identifier: AGPL-3.0-or-later

//! Builds an isolated two-cell executable-test universe near one cell edge.
//!
//! This is fixture tooling only. The worker has no teleport or handoff bypass:
//! the ordinary authenticated control, physics, directory, and transfer paths
//! must move the pilot into the destination cell.

use std::env;
use std::error::Error;
use std::path::PathBuf;

use verse_protocol::{LocomotionKind, Vec3};
use verse_simulation::{
    Store, address_from_origin_offset_um, cell_id, local_position_from_address, proof_cell_keys,
};

const DEFAULT_SEED: u64 = 8_031;
const EAST_BOUNDARY_INSET_UM: i128 = 250_000;
const PROOF_CELL_EDGE_UM: i128 = 20_000_000_000;

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = env::args_os();
    let executable = arguments
        .next()
        .and_then(|value| {
            PathBuf::from(value)
                .file_name()
                .map(std::ffi::OsStr::to_owned)
        })
        .and_then(|value| value.to_str().map(str::to_owned))
        .unwrap_or_else(|| "two_cell_boundary_fixture".into());
    let Some(root) = arguments.next().map(PathBuf::from) else {
        return Err(format!("usage: {executable} <universe-root> [world-seed]").into());
    };
    let seed = arguments
        .next()
        .map(|value| value.to_string_lossy().parse::<u64>())
        .transpose()?
        .unwrap_or(DEFAULT_SEED);
    if arguments.next().is_some() {
        return Err(format!("usage: {executable} <universe-root> [world-seed]").into());
    }

    let [source_cell_key, destination_cell_key] = proof_cell_keys()?;
    let source_cell_id = cell_id(&source_cell_key)?;
    let destination_cell_id = cell_id(&destination_cell_key)?;
    let source_root = root.join("cells").join(&source_cell_id);
    let mut store = Store::open_for_cell(&source_root, seed, source_cell_key)?;
    let mut world = store.load_world()?;
    world.fencing_token = store.fencing_token();

    let boundary_address = address_from_origin_offset_um(
        &world.cell_address,
        [PROOF_CELL_EDGE_UM / 2 - EAST_BOUNDARY_INSET_UM, 0, 0],
    )?;
    let boundary_position = local_position_from_address(&world.cell_address, &boundary_address)?;
    let player = world
        .player
        .get_mut("player-local")
        .ok_or("the proof universe does not contain player-local")?;
    player.address = boundary_address;
    player.position = boundary_position;
    player.linear_velocity = Vec3::ZERO;
    player.locomotion.kind = LocomotionKind::Eva;
    player.locomotion.support = None;
    player.locomotion.magnetic_boots_enabled = false;
    player.jetpack_enabled = true;
    player.dampeners = true;
    store.save_snapshot(&world)?;

    println!("VERSE_TWO_CELL_FIXTURE_OK source={source_cell_id} destination={destination_cell_id}");
    Ok(())
}
