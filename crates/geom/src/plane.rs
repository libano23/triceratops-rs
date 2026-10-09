use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::{Ray, TOL};

/// An oriented plane with an orthonormal frame (origin, x, y, z = normal).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plane {
    pub origin: DVec3,
    pub x_axis: DVec3,
    pub y_axis: DVec3,
    pub z_axis: DVec3,
}

impl Default for Plane {
    fn default() -> Self {
        Self::world_xy()
    }
}

impl Plane {
    pub fn world_xy() -> Self {
        Self { origin: DVec3::ZERO, x_axis: DVec3::X, y_axis: DVec3::Y, z_axis: DVec3::Z }
    }
    pub fn world_xz() -> Self {
        // "Front" view in Triceratops: x right, z up, normal pointing -Y (towards viewer)
        Self { origin: DVec3::ZERO, x_axis: DVec3::X, y_axis: DVec3::Z, z_axis: -DVec3::Y }
    }
    pub fn world_yz() -> Self {
        // "Right" view: y right, z up, normal +X
        Self { origin: DVec3::ZERO, x_axis: DVec3::Y, y_axis: DVec3::Z, z_axis: DVec3::X }
    }

    /// Build a plane from an origin and a normal; x axis is chosen deterministically.
    pub fn from_normal(origin: DVec3, normal: DVec3) -> Self {
        let z = normal.normalize_or(DVec3::Z);
        let helper = if z.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
        let x = helper.cross(z).normalize();
        let y = z.cross(x);
        Self { origin, x_axis: x, y_axis: y, z_axis: z }
    }

    /// Plane through origin with given x direction and a point hinting the y direction.
    pub fn from_frame(origin: DVec3, x_dir: DVec3, y_hint: DVec3) -> Self {
        let x = x_dir.normalize_or(DVec3::X);
        let mut z = x.cross(y_hint);
        if z.length_squared() < TOL {
            z = if x.z.abs() < 0.9 { x.cross(DVec3::Z) } else { x.cross(DVec3::X) };
        }
        let z = z.normalize();
        let y = z.cross(x);
        Self { origin, x_axis: x, y_axis: y, z_axis: z }
    }

    pub fn from_three_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Self> {
        let x = b - a;
        let n = x.cross(c - a);
        if x.length_squared() < TOL || n.length_squared() < TOL {
            return None;
        }
        Some(Self::from_frame(a, x, c - a))
    }

    pub fn point_at(&self, u: f64, v: f64) -> DVec3 {
        self.origin + self.x_axis * u + self.y_axis * v
    }

    pub fn point_at_3(&self, u: f64, v: f64, w: f64) -> DVec3 {
        self.origin + self.x_axis * u + self.y_axis * v + self.z_axis * w
    }

    /// Planar coordinates (u, v) plus signed distance w of a world point.
    pub fn closest_parameters(&self, p: DVec3) -> (f64, f64, f64) {
        let d = p - self.origin;
        (d.dot(self.x_axis), d.dot(self.y_axis), d.dot(self.z_axis))
    }

    pub fn closest_point(&self, p: DVec3) -> DVec3 {
        let (u, v, _) = self.closest_parameters(p);
        self.point_at(u, v)
    }

    pub fn signed_distance(&self, p: DVec3) -> f64 {
        (p - self.origin).dot(self.z_axis)
    }

    /// Intersect a ray with the plane. Returns the ray parameter.
    pub fn intersect_ray(&self, ray: &Ray) -> Option<f64> {
        let denom = ray.dir.dot(self.z_axis);
        if denom.abs() < 1e-12 {
            return None;
        }
        let t = (self.origin - ray.origin).dot(self.z_axis) / denom;
        Some(t)
    }

    pub fn transform(&self, xf: &DAffine3) -> Self {
        let origin = xf.transform_point3(self.origin);
        let x = xf.transform_vector3(self.x_axis).normalize();
        let y_hint = xf.transform_vector3(self.y_axis);
        Self::from_frame(origin, x, y_hint)
    }

    /// Matrix that maps plane coordinates (u, v, w) to world coordinates.
    pub fn to_world(&self) -> DAffine3 {
        DAffine3::from_cols(self.x_axis, self.y_axis, self.z_axis, self.origin)
    }
}
