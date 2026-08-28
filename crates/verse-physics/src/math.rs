// SPDX-License-Identifier: AGPL-3.0-or-later

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[must_use]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub const fn splat(value: f64) -> Self {
        Self::new(value, value, value)
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }

    pub fn dot(self, rhs: Self) -> f64 {
        self.x.mul_add(rhs.x, self.y.mul_add(rhs.y, self.z * rhs.z))
    }

    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y.mul_add(rhs.z, -self.z * rhs.y),
            self.z.mul_add(rhs.x, -self.x * rhs.z),
            self.x.mul_add(rhs.y, -self.y * rhs.x),
        )
    }

    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }

    pub fn length(self) -> f64 {
        self.length_squared().sqrt()
    }

    pub fn normalized(self) -> Option<Self> {
        let length = self.length();
        (length > 1.0e-12 && length.is_finite()).then(|| self * (1.0 / length))
    }

    pub fn clamped_length(self, maximum: f64) -> Self {
        let length = self.length();
        if length <= maximum || length <= f64::EPSILON {
            self
        } else {
            self * (maximum / length)
        }
    }

    pub fn component_abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs(), self.z.abs())
    }

    pub fn component_min(self, rhs: Self) -> Self {
        Self::new(self.x.min(rhs.x), self.y.min(rhs.y), self.z.min(rhs.z))
    }

    pub fn component_max(self, rhs: Self) -> Self {
        Self::new(self.x.max(rhs.x), self.y.max(rhs.y), self.z.max(rhs.z))
    }

    pub fn lerp(self, rhs: Self, fraction: f64) -> Self {
        self * (1.0 - fraction) + rhs * fraction
    }
}

impl Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f64> for Vec3 {
    type Output = Self;

    fn mul(self, rhs: f64) -> Self::Output {
        Self::new(self.x * rhs, self.y * rhs, self.z * rhs)
    }
}

impl Neg for Vec3 {
    type Output = Self;

    fn neg(self) -> Self::Output {
        Self::new(-self.x, -self.y, -self.z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[must_use]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self::new(0.0, 0.0, 0.0, 1.0);

    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { x, y, z, w }
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite() && self.w.is_finite()
    }

    pub fn length_squared(self) -> f32 {
        self.x.mul_add(
            self.x,
            self.y
                .mul_add(self.y, self.z.mul_add(self.z, self.w * self.w)),
        )
    }

    pub fn dot(self, rhs: Self) -> f32 {
        self.x.mul_add(
            rhs.x,
            self.y.mul_add(rhs.y, self.z.mul_add(rhs.z, self.w * rhs.w)),
        )
    }

    pub fn normalized(self) -> Option<Self> {
        let length = self.length_squared().sqrt();
        (length > 1.0e-6 && length.is_finite()).then(|| {
            Self::new(
                self.x / length,
                self.y / length,
                self.z / length,
                self.w / length,
            )
        })
    }

    pub fn conjugate(self) -> Self {
        Self::new(-self.x, -self.y, -self.z, self.w)
    }

    pub fn rotate(self, vector: Vec3) -> Vec3 {
        let vector_quat = Self::new(vector.x as f32, vector.y as f32, vector.z as f32, 0.0);
        let rotated = self * vector_quat * self.conjugate();
        Vec3::new(
            f64::from(rotated.x),
            f64::from(rotated.y),
            f64::from(rotated.z),
        )
    }

    pub fn nlerp(self, mut rhs: Self, fraction: f64) -> Self {
        let dot = self.dot(rhs);
        if dot < 0.0 {
            rhs = Self::new(-rhs.x, -rhs.y, -rhs.z, -rhs.w);
        }
        let fraction = fraction as f32;
        Self::new(
            self.x * (1.0 - fraction) + rhs.x * fraction,
            self.y * (1.0 - fraction) + rhs.y * fraction,
            self.z * (1.0 - fraction) + rhs.z * fraction,
            self.w * (1.0 - fraction) + rhs.w * fraction,
        )
        .normalized()
        .unwrap_or(Self::IDENTITY)
    }
}

impl Default for Quat {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mul for Quat {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self::new(
            self.w.mul_add(
                rhs.x,
                self.x.mul_add(rhs.w, self.y * rhs.z - self.z * rhs.y),
            ),
            self.w.mul_add(
                rhs.y,
                self.y.mul_add(rhs.w, self.z * rhs.x - self.x * rhs.z),
            ),
            self.w.mul_add(
                rhs.z,
                self.z.mul_add(rhs.w, self.x * rhs.y - self.y * rhs.x),
            ),
            self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
        )
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[must_use]
pub struct Pose {
    pub position: Vec3,
    pub rotation: Quat,
}

impl Pose {
    pub const IDENTITY: Self = Self {
        position: Vec3::ZERO,
        rotation: Quat::IDENTITY,
    };

    pub const fn new(position: Vec3, rotation: Quat) -> Self {
        Self { position, rotation }
    }

    pub fn transform_point(self, point: Vec3) -> Vec3 {
        self.position + self.rotation.rotate(point)
    }

    pub fn combined(self, local: Self) -> Self {
        Self::new(
            self.transform_point(local.position),
            self.rotation * local.rotation,
        )
    }

    pub fn interpolate(self, rhs: Self, fraction: f64) -> Self {
        Self::new(
            self.position.lerp(rhs.position, fraction),
            self.rotation.nlerp(rhs.rotation, fraction),
        )
    }
}
