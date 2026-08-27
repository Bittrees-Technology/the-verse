// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use crate::{BodyMotion, BodySpec, BodyState, BoxColliderSpec, ContactRecord, Quat, Vec3};

const CONTACT_SLOP_M: f64 = 0.02;
const AXIS_EPSILON_SQUARED: f64 = 1.0e-16;
const SWEEP_SAMPLES: usize = 8;

#[derive(Clone, Copy)]
struct OrientedBox<'a> {
    body_id: &'a str,
    collider_id: &'a str,
    body_position: Vec3,
    center: Vec3,
    axes: [Vec3; 3],
    half_extents: Vec3,
    linear_velocity: Vec3,
    angular_velocity: Vec3,
    motion: BodyMotion,
}

#[derive(Clone, Copy)]
struct SweptAabb {
    minimum: Vec3,
    maximum: Vec3,
}

pub(crate) fn contacts_for_step(
    specs: &BTreeMap<String, BodySpec>,
    before: &[BodyState],
    after: &[BodyState],
) -> Vec<ContactRecord> {
    let before_by_id = before
        .iter()
        .map(|state| (state.body_id.as_str(), state))
        .collect::<BTreeMap<_, _>>();
    let after_by_id = after
        .iter()
        .map(|state| (state.body_id.as_str(), state))
        .collect::<BTreeMap<_, _>>();
    let ids = specs.keys().collect::<Vec<_>>();
    let mut contacts = BTreeMap::new();

    for left_index in 0..ids.len() {
        for right_index in (left_index + 1)..ids.len() {
            let left_spec = &specs[ids[left_index]];
            let right_spec = &specs[ids[right_index]];
            if left_spec.motion == BodyMotion::Static && right_spec.motion == BodyMotion::Static {
                continue;
            }
            let Some(left_before) = before_by_id.get(left_spec.body_id.as_str()) else {
                continue;
            };
            let Some(right_before) = before_by_id.get(right_spec.body_id.as_str()) else {
                continue;
            };
            let Some(left_after) = after_by_id.get(left_spec.body_id.as_str()) else {
                continue;
            };
            let Some(right_after) = after_by_id.get(right_spec.body_id.as_str()) else {
                continue;
            };

            for left_collider in &left_spec.colliders {
                let left_bounds = swept_aabb(left_collider, left_before, left_after);
                for right_collider in &right_spec.colliders {
                    let right_bounds = swept_aabb(right_collider, right_before, right_after);
                    if !left_bounds.overlaps(right_bounds) {
                        continue;
                    }
                    let key = (
                        left_spec.body_id.clone(),
                        left_collider.collider_id.clone(),
                        right_spec.body_id.clone(),
                        right_collider.collider_id.clone(),
                    );
                    for sample in 0..=SWEEP_SAMPLES {
                        let fraction = sample as f64 / SWEEP_SAMPLES as f64;
                        let left_box = interpolated_box(
                            left_spec,
                            left_collider,
                            left_before,
                            left_after,
                            fraction,
                        );
                        let right_box = interpolated_box(
                            right_spec,
                            right_collider,
                            right_before,
                            right_after,
                            fraction,
                        );
                        let Some(record) = detect_box_contact(left_box, right_box) else {
                            continue;
                        };
                        contacts
                            .entry(key.clone())
                            .and_modify(|existing: &mut ContactRecord| {
                                existing.penetration_m =
                                    existing.penetration_m.max(record.penetration_m);
                                existing.impact_speed_mps =
                                    existing.impact_speed_mps.max(record.impact_speed_mps);
                                if record.penetration_m >= existing.penetration_m {
                                    existing.point = record.point;
                                    existing.normal = record.normal;
                                }
                            })
                            .or_insert(record);
                    }
                }
            }
        }
    }

    contacts.into_values().collect()
}

impl SweptAabb {
    fn overlaps(self, rhs: Self) -> bool {
        self.minimum.x <= rhs.maximum.x + CONTACT_SLOP_M
            && self.maximum.x + CONTACT_SLOP_M >= rhs.minimum.x
            && self.minimum.y <= rhs.maximum.y + CONTACT_SLOP_M
            && self.maximum.y + CONTACT_SLOP_M >= rhs.minimum.y
            && self.minimum.z <= rhs.maximum.z + CONTACT_SLOP_M
            && self.maximum.z + CONTACT_SLOP_M >= rhs.minimum.z
    }
}

/// Computes a conservative bound for every point on a collider during the
/// fixed step. Body translation is bounded exactly by the endpoint segment.
/// Rotation is bounded by the maximum chord displacement of a sphere that
/// encloses the collider and its offset from the body's origin. This may admit
/// false positives, but cannot reject an intermediate rotating-box contact.
fn swept_aabb(collider: &BoxColliderSpec, before: &BodyState, after: &BodyState) -> SweptAabb {
    let before_pose = before.pose.combined(collider.local_pose);
    let before_axes = axes(before_pose.rotation);
    let oriented_extent = Vec3::new(
        projection_radius_for_axes(before_axes, collider.half_extents, Vec3::new(1.0, 0.0, 0.0)),
        projection_radius_for_axes(before_axes, collider.half_extents, Vec3::new(0.0, 1.0, 0.0)),
        projection_radius_for_axes(before_axes, collider.half_extents, Vec3::new(0.0, 0.0, 1.0)),
    );
    let translation = after.pose.position - before.pose.position;
    let translated_center = before_pose.position + translation;

    let quaternion_dot = f64::from(
        before
            .pose
            .rotation
            .dot(after.pose.rotation)
            .abs()
            .clamp(0.0, 1.0),
    );
    let half_angle_sine = (1.0 - quaternion_dot * quaternion_dot).max(0.0).sqrt();
    let enclosing_radius = collider.local_pose.position.length() + collider.half_extents.length();
    let rotation_padding = 2.0 * enclosing_radius * half_angle_sine;
    let padding = oriented_extent + Vec3::splat(rotation_padding);

    SweptAabb {
        minimum: before_pose.position.component_min(translated_center) - padding,
        maximum: before_pose.position.component_max(translated_center) + padding,
    }
}

fn interpolated_box<'a>(
    spec: &'a BodySpec,
    collider: &'a BoxColliderSpec,
    before: &BodyState,
    after: &BodyState,
    fraction: f64,
) -> OrientedBox<'a> {
    let body_pose = before.pose.interpolate(after.pose, fraction);
    let world_pose = body_pose.combined(collider.local_pose);
    OrientedBox {
        body_id: &spec.body_id,
        collider_id: &collider.collider_id,
        body_position: body_pose.position,
        center: world_pose.position,
        axes: axes(world_pose.rotation),
        half_extents: collider.half_extents,
        linear_velocity: before.linear_velocity,
        angular_velocity: before.angular_velocity,
        motion: spec.motion,
    }
}

fn axes(rotation: Quat) -> [Vec3; 3] {
    [
        rotation.rotate(Vec3::new(1.0, 0.0, 0.0)),
        rotation.rotate(Vec3::new(0.0, 1.0, 0.0)),
        rotation.rotate(Vec3::new(0.0, 0.0, 1.0)),
    ]
}

fn detect_box_contact(left: OrientedBox<'_>, right: OrientedBox<'_>) -> Option<ContactRecord> {
    let center_delta = right.center - left.center;
    let mut candidates = Vec::with_capacity(15);
    candidates.extend(left.axes);
    candidates.extend(right.axes);
    for left_axis in left.axes {
        for right_axis in right.axes {
            let cross = left_axis.cross(right_axis);
            if cross.length_squared() > AXIS_EPSILON_SQUARED {
                candidates.push(cross);
            }
        }
    }

    let mut minimum_overlap = f64::INFINITY;
    let mut normal = Vec3::ZERO;
    for candidate in candidates {
        let axis = candidate.normalized()?;
        let left_radius = projection_radius(left, axis);
        let right_radius = projection_radius(right, axis);
        let signed_distance = center_delta.dot(axis);
        let overlap = left_radius + right_radius - signed_distance.abs();
        if overlap < -CONTACT_SLOP_M {
            return None;
        }
        if overlap < minimum_overlap {
            minimum_overlap = overlap;
            normal = if signed_distance >= 0.0 { axis } else { -axis };
        }
    }

    let left_support = support(left, normal);
    let right_support = support(right, -normal);
    let point = (left_support + right_support) * 0.5;
    let left_velocity = point_velocity(left, point);
    let right_velocity = point_velocity(right, point);
    let impact_speed_mps = (left_velocity - right_velocity).dot(normal).max(0.0);

    Some(ContactRecord {
        body_a_id: left.body_id.to_owned(),
        collider_a_id: left.collider_id.to_owned(),
        body_b_id: right.body_id.to_owned(),
        collider_b_id: right.collider_id.to_owned(),
        normal,
        point,
        penetration_m: minimum_overlap.max(0.0),
        impact_speed_mps,
        source: crate::ContactSource::GeometricFallback,
    })
}

fn projection_radius(collider: OrientedBox<'_>, axis: Vec3) -> f64 {
    projection_radius_for_axes(collider.axes, collider.half_extents, axis)
}

fn projection_radius_for_axes(
    box_axes: [Vec3; 3],
    half_extents: Vec3,
    projection_axis: Vec3,
) -> f64 {
    half_extents.x * box_axes[0].dot(projection_axis).abs()
        + half_extents.y * box_axes[1].dot(projection_axis).abs()
        + half_extents.z * box_axes[2].dot(projection_axis).abs()
}

fn support(collider: OrientedBox<'_>, direction: Vec3) -> Vec3 {
    let mut point = collider.center;
    for (axis, extent) in collider.axes.into_iter().zip([
        collider.half_extents.x,
        collider.half_extents.y,
        collider.half_extents.z,
    ]) {
        point = point
            + axis
                * if axis.dot(direction) >= 0.0 {
                    extent
                } else {
                    -extent
                };
    }
    point
}

fn point_velocity(collider: OrientedBox<'_>, point: Vec3) -> Vec3 {
    if collider.motion == BodyMotion::Static {
        Vec3::ZERO
    } else {
        collider.linear_velocity
            + collider
                .angular_velocity
                .cross(point - collider.body_position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Pose, Quat};

    #[test]
    fn swept_bounds_cover_intermediate_rotation() {
        let collider = BoxColliderSpec {
            collider_id: "long-block".into(),
            local_pose: Pose::IDENTITY,
            half_extents: Vec3::new(2.0, 0.1, 0.1),
            density_kg_per_m3: 1_000.0,
        };
        let before = BodyState {
            body_id: "grid".into(),
            pose: Pose::IDENTITY,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            active: true,
        };
        let after = BodyState {
            pose: Pose::new(Vec3::ZERO, Quat::new(0.0, 0.0, 1.0, 0.0)),
            ..before.clone()
        };

        let bounds = swept_aabb(&collider, &before, &after);
        assert!(bounds.minimum.y <= -2.0);
        assert!(bounds.maximum.y >= 2.0);
    }
}
