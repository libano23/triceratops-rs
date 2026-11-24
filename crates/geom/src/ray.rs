use glam::DVec3;

/// A half-line used for picking and plane intersection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: DVec3,
    pub dir: DVec3,
}

impl Ray {
    pub fn new(origin: DVec3, dir: DVec3) -> Self {
        Self { origin, dir: dir.normalize_or(DVec3::NEG_Z) }
    }

    pub fn at(&self, t: f64) -> DVec3 {
        self.origin + self.dir * t
    }

    /// Closest approach between this ray and a segment `a-b`.
    /// Returns `(t_ray, distance, point_on_segment)`.
    pub fn closest_to_segment(&self, a: DVec3, b: DVec3) -> (f64, f64, DVec3) {
        let u = self.dir;
        let v = b - a;
        let w0 = self.origin - a;
        let aa = u.dot(u);
        let bb = u.dot(v);
        let cc = v.dot(v);
        let dd = u.dot(w0);
        let ee = v.dot(w0);
        let denom = aa * cc - bb * bb;
        let (mut s, t);
        if denom.abs() < 1e-14 || cc < 1e-14 {
            // parallel or degenerate segment: project a onto ray
            t = 0.0;
            s = -dd / aa;
        } else {
            t = ((aa * ee - bb * dd) / denom).clamp(0.0, 1.0);
            s = (bb * t - dd) / aa;
        }
        s = s.max(0.0);
        let p_ray = self.at(s);
        let p_seg = a + v * t;
        (s, (p_ray - p_seg).length(), p_seg)
    }

    /// Möller–Trumbore ray/triangle intersection. Returns ray parameter.
    pub fn intersect_triangle(&self, v0: DVec3, v1: DVec3, v2: DVec3) -> Option<f64> {
        let e1 = v1 - v0;
        let e2 = v2 - v0;
        let p = self.dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-14 {
            return None;
        }
        let inv = 1.0 / det;
        let tv = self.origin - v0;
        let u = tv.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = tv.cross(e1);
        let v = self.dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        if t > 1e-9 {
            Some(t)
        } else {
            None
        }
    }
}
