// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic, server-authoritative hand-tool targeting.
//!
//! Clients send only stable voxel coordinates or block identities. The server
//! reconstructs the actor's eye ray and accepts an operation only when that
//! identity is the closest visible surface on the ray.

use std::collections::BTreeMap;

use verse_protocol::{IVec3, LocomotionKind, Vec3};

use crate::content;
use crate::model::{Grid, Player, VoxelField};

pub(crate) const TOOL_SURFACE_RANGE_M: f64 = 9.0;
const HIT_DISTANCE_EPSILON_M: f64 = 1.0e-9;
const DIRECTION_EPSILON: f64 = 1.0e-12;
const MAX_DDA_STEPS: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ToolTarget {
    Voxel {
        coordinate: IVec3,
    },
    Block {
        grid_id: String,
        block_id: String,
        coordinate: IVec3,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ToolHit {
    pub target: ToolTarget,
    pub distance_m: f64,
    /// Outward face normal in the target's coordinate space. `None` means the
    /// eye started inside or on occupied geometry and interactions must fail
    /// closed even though the geometry remains an occluder.
    pub local_face: Option<IVec3>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Ray {
    origin: Vec3,
    direction: Vec3,
}

pub(crate) fn closest_tool_hit(
    actor: &Player,
    voxels: &VoxelField,
    grids: &BTreeMap<String, Grid>,
) -> Option<ToolHit> {
    let ray = actor_eye_ray(actor)?;
    let mut closest = closest_voxel_hit(ray, voxels);

    for (grid_id, grid) in grids {
        let local_origin = grid
            .orientation
            .conjugate()
            .rotate(ray.origin - grid.position);
        let local_direction = grid.orientation.conjugate().rotate(ray.direction);
        let local_ray = Ray {
            origin: local_origin,
            direction: local_direction,
        };
        for (block_id, block) in &grid.blocks {
            let Some(surface) = ray_unit_aabb(local_ray, block.coordinate) else {
                continue;
            };
            if surface.distance_m > TOOL_SURFACE_RANGE_M + HIT_DISTANCE_EPSILON_M {
                continue;
            }
            let candidate = ToolHit {
                target: ToolTarget::Block {
                    grid_id: grid_id.clone(),
                    block_id: block_id.clone(),
                    coordinate: block.coordinate,
                },
                distance_m: surface.distance_m,
                local_face: surface.face,
            };
            retain_closest(&mut closest, candidate);
        }
    }

    closest
}

fn actor_eye_ray(actor: &Player) -> Option<Ray> {
    let orientation_up = actor.orientation.rotate(Vec3::new(0.0, 1.0, 0.0));
    let up = if matches!(
        actor.locomotion.kind,
        LocomotionKind::Grounded | LocomotionKind::Magnetic | LocomotionKind::Airborne
    ) && actor.locomotion.up.magnitude() > DIRECTION_EPSILON
    {
        actor.locomotion.up * (1.0 / actor.locomotion.up.magnitude())
    } else {
        orientation_up
    };
    let character = &content::manifest().character;
    let eye_offset_m = character.eye_height_m - character.standing_height_m * 0.5;
    let origin = actor.position + up * eye_offset_m;

    let pitch = if actor.locomotion.kind == LocomotionKind::Eva {
        0.0
    } else {
        actor.locomotion.view_pitch_radians
    };
    let local_forward = Vec3::new(0.0, pitch.sin(), -pitch.cos());
    let unnormalized = actor.orientation.rotate(local_forward);
    let magnitude = unnormalized.magnitude();
    if !origin.x.is_finite()
        || !origin.y.is_finite()
        || !origin.z.is_finite()
        || !magnitude.is_finite()
        || magnitude <= DIRECTION_EPSILON
    {
        return None;
    }
    Some(Ray {
        origin,
        direction: unnormalized * (1.0 / magnitude),
    })
}

fn closest_voxel_hit(ray: Ray, voxels: &VoxelField) -> Option<ToolHit> {
    // Shifting by half a cell maps integer-centered voxel cubes to the usual
    // floor-based DDA lattice.
    let shifted = ray.origin + Vec3::new(0.5, 0.5, 0.5);
    let mut cell = IVec3::new(
        shifted.x.floor() as i32,
        shifted.y.floor() as i32,
        shifted.z.floor() as i32,
    );
    let step = IVec3::new(
        direction_step(ray.direction.x),
        direction_step(ray.direction.y),
        direction_step(ray.direction.z),
    );
    let mut t_max = Vec3::new(
        first_boundary_distance(shifted.x, ray.direction.x, cell.x, step.x),
        first_boundary_distance(shifted.y, ray.direction.y, cell.y, step.y),
        first_boundary_distance(shifted.z, ray.direction.z, cell.z, step.z),
    );
    let t_delta = Vec3::new(
        boundary_delta(ray.direction.x),
        boundary_delta(ray.direction.y),
        boundary_delta(ray.direction.z),
    );
    let mut closest = None;
    // `floor(origin + 0.5)` names only one cell at an exact face, edge, or
    // corner. Every closed voxel cube touching the eye is nevertheless an
    // occluder, so enumerate all boundary-adjacent origin cells first.
    for origin_cell in origin_touching_cells(ray.origin) {
        if voxels.occupied.contains(&origin_cell)
            && let Some(surface) = ray_unit_aabb(ray, origin_cell)
        {
            retain_closest(
                &mut closest,
                ToolHit {
                    target: ToolTarget::Voxel {
                        coordinate: origin_cell,
                    },
                    distance_m: surface.distance_m,
                    local_face: surface.face,
                },
            );
        }
    }

    for _ in 0..MAX_DDA_STEPS {
        // A ray parallel to and exactly on a voxel plane touches both columns
        // for its full length. Test both closed-cell sides, not only the cell
        // selected by `floor`, so a boundary ray cannot see through terrain.
        for touched_cell in parallel_boundary_cells(cell, shifted, step) {
            if voxels.occupied.contains(&touched_cell)
                && let Some(surface) = ray_unit_aabb(ray, touched_cell)
                && surface.distance_m <= TOOL_SURFACE_RANGE_M + HIT_DISTANCE_EPSILON_M
            {
                retain_closest(
                    &mut closest,
                    ToolHit {
                        target: ToolTarget::Voxel {
                            coordinate: touched_cell,
                        },
                        distance_m: surface.distance_m,
                        local_face: surface.face,
                    },
                );
            }
        }

        let next_boundary = t_max.x.min(t_max.y).min(t_max.z);
        if next_boundary > TOOL_SURFACE_RANGE_M + HIT_DISTANCE_EPSILON_M
            || closest.as_ref().is_some_and(|hit: &ToolHit| {
                next_boundary > hit.distance_m + HIT_DISTANCE_EPSILON_M
            })
        {
            break;
        }

        // Crossing an exact edge/corner is deliberately advanced one axis at
        // a time in X/Y/Z order. Combined with the stable hit comparator this
        // gives boundary rays one platform-independent canonical result.
        if t_max.x <= t_max.y && t_max.x <= t_max.z {
            cell.x = cell.x.saturating_add(step.x);
            t_max.x += t_delta.x;
        } else if t_max.y <= t_max.z {
            cell.y = cell.y.saturating_add(step.y);
            t_max.y += t_delta.y;
        } else {
            cell.z = cell.z.saturating_add(step.z);
            t_max.z += t_delta.z;
        }
    }

    closest
}

fn parallel_boundary_cells(cell: IVec3, shifted_origin: Vec3, step: IVec3) -> Vec<IVec3> {
    fn axis_cells(cell: i32, shifted_origin: f64, step: i32) -> Vec<i32> {
        if step == 0 && (shifted_origin - shifted_origin.round()).abs() <= DIRECTION_EPSILON {
            vec![cell.saturating_sub(1), cell]
        } else {
            vec![cell]
        }
    }

    let xs = axis_cells(cell.x, shifted_origin.x, step.x);
    let ys = axis_cells(cell.y, shifted_origin.y, step.y);
    let zs = axis_cells(cell.z, shifted_origin.z, step.z);
    let mut cells = Vec::with_capacity(xs.len() * ys.len() * zs.len());
    for x in xs {
        for &y in &ys {
            for &z in &zs {
                cells.push(IVec3::new(x, y, z));
            }
        }
    }
    cells.sort_unstable();
    cells.dedup();
    cells
}

fn origin_touching_cells(origin: Vec3) -> Vec<IVec3> {
    fn axis_cells(value: f64) -> Vec<i32> {
        let shifted = value + 0.5;
        let upper = shifted.floor() as i32;
        if (shifted - shifted.round()).abs() <= DIRECTION_EPSILON {
            vec![upper.saturating_sub(1), upper]
        } else {
            vec![upper]
        }
    }

    let xs = axis_cells(origin.x);
    let ys = axis_cells(origin.y);
    let zs = axis_cells(origin.z);
    let mut cells = Vec::with_capacity(xs.len() * ys.len() * zs.len());
    for x in xs {
        for &y in &ys {
            for &z in &zs {
                cells.push(IVec3::new(x, y, z));
            }
        }
    }
    cells.sort_unstable();
    cells.dedup();
    cells
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SurfaceHit {
    distance_m: f64,
    face: Option<IVec3>,
}

fn ray_unit_aabb(ray: Ray, coordinate: IVec3) -> Option<SurfaceHit> {
    let center = Vec3::new(
        f64::from(coordinate.x),
        f64::from(coordinate.y),
        f64::from(coordinate.z),
    );
    let minimum = center - Vec3::new(0.5, 0.5, 0.5);
    let maximum = center + Vec3::new(0.5, 0.5, 0.5);
    let mut entry = f64::NEG_INFINITY;
    let mut exit = f64::INFINITY;
    let mut face = None;

    // Axis order is semantically significant: equal-distance slab entries use
    // the X face, then Y, then Z.
    for (origin, direction, lower, upper, negative_face, positive_face) in [
        (
            ray.origin.x,
            ray.direction.x,
            minimum.x,
            maximum.x,
            IVec3::new(-1, 0, 0),
            IVec3::new(1, 0, 0),
        ),
        (
            ray.origin.y,
            ray.direction.y,
            minimum.y,
            maximum.y,
            IVec3::new(0, -1, 0),
            IVec3::new(0, 1, 0),
        ),
        (
            ray.origin.z,
            ray.direction.z,
            minimum.z,
            maximum.z,
            IVec3::new(0, 0, -1),
            IVec3::new(0, 0, 1),
        ),
    ] {
        if direction.abs() <= DIRECTION_EPSILON {
            if origin < lower || origin > upper {
                return None;
            }
            continue;
        }
        let lower_t = (lower - origin) / direction;
        let upper_t = (upper - origin) / direction;
        let (near, far, near_face) = if lower_t <= upper_t {
            (lower_t, upper_t, negative_face)
        } else {
            (upper_t, lower_t, positive_face)
        };
        if near > entry + HIT_DISTANCE_EPSILON_M {
            entry = near;
            face = Some(near_face);
        }
        exit = exit.min(far);
        if entry > exit + HIT_DISTANCE_EPSILON_M {
            return None;
        }
    }

    if exit < -HIT_DISTANCE_EPSILON_M {
        return None;
    }
    let distance_m = entry.max(0.0);
    Some(SurfaceHit {
        distance_m,
        face: (distance_m > HIT_DISTANCE_EPSILON_M)
            .then_some(face)
            .flatten(),
    })
}

fn direction_step(direction: f64) -> i32 {
    if direction > DIRECTION_EPSILON {
        1
    } else if direction < -DIRECTION_EPSILON {
        -1
    } else {
        0
    }
}

fn first_boundary_distance(origin: f64, direction: f64, cell: i32, step: i32) -> f64 {
    match step.cmp(&0) {
        std::cmp::Ordering::Greater => (f64::from(cell.saturating_add(1)) - origin) / direction,
        std::cmp::Ordering::Less => (f64::from(cell) - origin) / direction,
        std::cmp::Ordering::Equal => f64::INFINITY,
    }
}

fn boundary_delta(direction: f64) -> f64 {
    if direction.abs() <= DIRECTION_EPSILON {
        f64::INFINITY
    } else {
        direction.abs().recip()
    }
}

fn retain_closest(closest: &mut Option<ToolHit>, candidate: ToolHit) {
    let replace = closest
        .as_ref()
        .is_none_or(|current| hit_precedes(&candidate, current));
    if replace {
        *closest = Some(candidate);
    }
}

fn hit_precedes(candidate: &ToolHit, current: &ToolHit) -> bool {
    if candidate.distance_m < current.distance_m - HIT_DISTANCE_EPSILON_M {
        return true;
    }
    if candidate.distance_m > current.distance_m + HIT_DISTANCE_EPSILON_M {
        return false;
    }
    target_sort_key(&candidate.target) < target_sort_key(&current.target)
}

fn target_sort_key(target: &ToolTarget) -> (u8, String, String, IVec3) {
    match target {
        ToolTarget::Block {
            grid_id,
            block_id,
            coordinate,
        } => (0, grid_id.clone(), block_id.clone(), *coordinate),
        ToolTarget::Voxel { coordinate } => (1, String::new(), String::new(), *coordinate),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet, VecDeque};

    use verse_protocol::{
        CareerSnapshot, LocomotionKind, PlayerLifeState, PlayerLocomotionSnapshot, Quat,
    };

    use super::*;
    use crate::model::{Block, Grid};

    fn actor(position: Vec3, orientation: Quat) -> Player {
        Player {
            player_id: "targeting-pilot".into(),
            position,
            orientation,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            surface_contact: false,
            locomotion: PlayerLocomotionSnapshot {
                kind: LocomotionKind::Eva,
                up: Vec3::new(0.0, 1.0, 0.0),
                view_pitch_radians: 0.0,
                support: None,
                jump_held: false,
                jump_buffer_expires_at_simulation_tick: 0,
                support_grace_expires_at_simulation_tick: 0,
                magnetic_boots_enabled: false,
                magnetic_reattach_after_simulation_tick: 0,
            },
            movement_epoch: 1,
            last_received_input_sequence: 0,
            last_processed_input_sequence: 0,
            pending_control_frames: VecDeque::new(),
            control_linear_input: Vec3::ZERO,
            control_angular_input: Vec3::ZERO,
            boost: false,
            dampeners: true,
            jump: false,
            control_expires_at_simulation_tick: 0,
            inventory_id: "inventory-targeting-pilot".into(),
            experience: 0,
            career: CareerSnapshot::default(),
            suit_oxygen_milli: 1_000,
            helmet_closed: true,
            jetpack_enabled: true,
            life_state: PlayerLifeState::Alive,
        }
    }

    fn voxel_field(coordinates: impl IntoIterator<Item = IVec3>) -> VoxelField {
        VoxelField {
            occupied: coordinates.into_iter().collect(),
            ferrite_ore: BTreeSet::new(),
        }
    }

    fn grid(grid_id: &str, position: Vec3, orientation: Quat, blocks: Vec<Block>) -> Grid {
        Grid {
            grid_id: grid_id.into(),
            owner_player_id: "player-local".into(),
            anchor_reward_eligible: true,
            position,
            orientation,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            control_linear_input: Vec3::ZERO,
            control_angular_input: Vec3::ZERO,
            dampeners: true,
            anchored: false,
            blocks: blocks
                .into_iter()
                .map(|block| (block.block_id.clone(), block))
                .collect(),
        }
    }

    #[test]
    fn eye_ray_uses_capsule_center_offset_and_eva_forward() {
        let ray = actor_eye_ray(&actor(Vec3::new(2.0, 3.0, 4.0), Quat::IDENTITY)).unwrap();
        assert!((ray.origin.x - 2.0).abs() < 1.0e-12);
        assert!((ray.origin.y - 3.72).abs() < 1.0e-12);
        assert!((ray.origin.z - 4.0).abs() < 1.0e-12);
        assert_eq!(ray.direction, Vec3::new(0.0, 0.0, -1.0));
    }

    #[test]
    fn voxel_dda_selects_nearest_surface_and_outward_face() {
        let pilot = actor(Vec3::new(0.0, -0.72, 4.0), Quat::IDENTITY);
        let voxels = voxel_field([IVec3::new(0, 0, 0), IVec3::new(0, 0, -2)]);
        let hit = closest_tool_hit(&pilot, &voxels, &BTreeMap::new()).unwrap();
        assert_eq!(
            hit.target,
            ToolTarget::Voxel {
                coordinate: IVec3::ZERO
            }
        );
        assert!((hit.distance_m - 3.5).abs() < 1.0e-12);
        assert_eq!(hit.local_face, Some(IVec3::new(0, 0, 1)));
    }

    #[test]
    fn rotated_grid_is_intersected_in_local_space() {
        let half_turn_y = Quat::new(0.0, 1.0, 0.0, 0.0);
        let pilot = actor(Vec3::new(0.0, -0.72, 4.0), Quat::IDENTITY);
        let block = Block::new(
            "rotated",
            IVec3::new(0, 0, 2),
            verse_protocol::BlockKind::Structural,
        );
        let grids = BTreeMap::from([(
            "grid-rotated".into(),
            grid("grid-rotated", Vec3::ZERO, half_turn_y, vec![block]),
        )]);
        let hit = closest_tool_hit(&pilot, &voxel_field([]), &grids).unwrap();
        assert!(matches!(
            hit.target,
            ToolTarget::Block { ref block_id, .. } if block_id == "rotated"
        ));
        assert!((hit.distance_m - 5.5).abs() < 1.0e-12);
        assert_eq!(hit.local_face, Some(IVec3::new(0, 0, -1)));
    }

    #[test]
    fn block_wins_an_exact_tie_with_voxel() {
        let pilot = actor(Vec3::new(0.0, -0.72, 4.0), Quat::IDENTITY);
        let block = Block::new(
            "tie-block",
            IVec3::ZERO,
            verse_protocol::BlockKind::Structural,
        );
        let grids = BTreeMap::from([(
            "tie-grid".into(),
            grid("tie-grid", Vec3::ZERO, Quat::IDENTITY, vec![block]),
        )]);
        let hit = closest_tool_hit(&pilot, &voxel_field([IVec3::ZERO]), &grids).unwrap();
        assert!(matches!(hit.target, ToolTarget::Block { .. }));
    }

    #[test]
    fn origin_inside_geometry_is_an_unusable_zero_distance_occluder() {
        let pilot = actor(Vec3::new(0.0, -0.72, 0.0), Quat::IDENTITY);
        let hit = closest_tool_hit(&pilot, &voxel_field([IVec3::ZERO]), &BTreeMap::new()).unwrap();
        assert!(hit.distance_m.abs() < f64::EPSILON);
        assert_eq!(hit.local_face, None);
    }

    #[test]
    fn origin_on_shared_voxel_face_checks_both_sides_and_uses_stable_identity() {
        let pilot = actor(Vec3::new(0.5, -0.72, 0.0), Quat::IDENTITY);
        let hit = closest_tool_hit(
            &pilot,
            &voxel_field([IVec3::ZERO, IVec3::new(1, 0, 0)]),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(
            hit.target,
            ToolTarget::Voxel {
                coordinate: IVec3::ZERO
            }
        );
        assert!(hit.distance_m.abs() < f64::EPSILON);
        assert_eq!(hit.local_face, None);
    }

    #[test]
    fn ray_parallel_to_shared_voxel_face_checks_both_columns_for_its_full_length() {
        let pilot = actor(Vec3::new(0.5, -0.72, 4.0), Quat::IDENTITY);
        let hit = closest_tool_hit(&pilot, &voxel_field([IVec3::ZERO]), &BTreeMap::new())
            .expect("closed lower-side voxel face occludes a parallel boundary ray");
        assert_eq!(
            hit.target,
            ToolTarget::Voxel {
                coordinate: IVec3::ZERO
            }
        );
        assert!((hit.distance_m - 3.5).abs() < 1.0e-12);
    }

    #[test]
    fn surface_distance_is_inclusive_at_nine_meters() {
        let pilot = actor(Vec3::new(0.0, -0.72, 9.5), Quat::IDENTITY);
        assert!(closest_tool_hit(&pilot, &voxel_field([IVec3::ZERO]), &BTreeMap::new()).is_some());
        let too_far = actor(Vec3::new(0.0, -0.72, 9.500_000_01), Quat::IDENTITY);
        assert!(
            closest_tool_hit(&too_far, &voxel_field([IVec3::ZERO]), &BTreeMap::new()).is_none()
        );
    }
}
