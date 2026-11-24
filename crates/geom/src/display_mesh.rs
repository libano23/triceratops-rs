//! Display tessellation: meshes and edge polylines whose density follows the curvature of
//! the geometry (maximum angle between adjacent segments plus a chord tolerance), like the
//! render-mesh settings of NURBS modellers, instead of a fixed number of segments per span.
//!
//! The fixed-density `tessellate*` functions stay unchanged: the Boolean and intersection
//! code depends on their sampling.

use glam::{DVec2, DVec3};

use crate::{Brep, Face, Mesh, NurbsSurface};

/// Density of a display mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshDensity {
    /// Maximum turning angle between adjacent mesh segments, in radians.
    pub max_angle: f64,
    /// Maximum distance between a mesh segment and the surface (world units).
    pub chord: f64,
    /// Upper bound of segments per knot span and direction.
    pub max_per_span: usize,
}

impl MeshDensity {
    /// Smooth shading density for an object with the given bounding-box diagonal.
    pub fn for_size(diagonal: f64) -> Self {
        Self { max_angle: 6f64.to_radians(), chord: (diagonal * 2e-4).max(1e-6), max_per_span: 128 }
    }

    /// Segments needed for a polyline sample of a curve piece: by turning angle and by sag.
    fn segments_for(&self, pts: &[DVec3]) -> usize {
        let mut turning = 0.0;
        let mut length = 0.0;
        for w in pts.windows(3) {
            let (a, b) = (w[1] - w[0], w[2] - w[1]);
            if a.length_squared() > 1e-24 && b.length_squared() > 1e-24 {
                turning += a.angle_between(b);
            }
        }
        for w in pts.windows(2) {
            length += (w[1] - w[0]).length();
        }
        if turning < 1e-9 || length < 1e-12 {
            return 1;
        }
        let by_angle = (turning / self.max_angle).ceil();
        // a piece of radius R = length / turning sags R (1 - cos(phi / 2)) over a segment of angle phi
        // (the samples are equal steps in the parameter, which is not arc length on rational
        // spans: aim at 60 % of the tolerance so the longest segment still meets it)
        let radius = length / turning;
        let chord = self.chord * 0.6;
        let phi = if chord >= 2.0 * radius { std::f64::consts::PI } else { 2.0 * (1.0 - chord / radius).clamp(-1.0, 1.0).acos() };
        let by_chord = (turning / phi.max(1e-6)).ceil();
        by_angle.max(by_chord).clamp(1.0, self.max_per_span as f64) as usize
    }
}

/// Points of a straight (u, v) segment mapped onto the surface, `n + 1` samples.
fn uv_segment_points(s: &NurbsSurface, a: DVec2, b: DVec2, n: usize) -> Vec<DVec3> {
    (0..=n).map(|i| a + (b - a) * (i as f64 / n as f64)).map(|p| s.point_at(p.x, p.y)).collect()
}

impl NurbsSurface {
    /// Parameter values of a display grid in one direction: every knot span gets the number
    /// of segments its curvature needs, measured along several isocurves across the surface.
    pub fn display_params(&self, along_u: bool, d: &MeshDensity) -> Vec<f64> {
        let (knots, degree) = if along_u { (&self.knots_u, self.degree_u) } else { (&self.knots_v, self.degree_v) };
        let spans = {
            let mut k: Vec<f64> = knots[degree..knots.len() - degree].to_vec();
            k.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
            k
        };
        let (o0, o1) = if along_u { self.domain_v() } else { self.domain_u() };
        let across: Vec<f64> = (0..=4).map(|i| o0 + (o1 - o0) * i as f64 / 4.0).collect();
        let mut out = Vec::new();
        for w in spans.windows(2) {
            let n = if degree <= 1 {
                1
            } else {
                across
                    .iter()
                    .map(|&o| {
                        let pts: Vec<DVec3> = (0..=16)
                            .map(|i| w[0] + (w[1] - w[0]) * i as f64 / 16.0)
                            .map(|t| if along_u { self.point_at(t, o) } else { self.point_at(o, t) })
                            .collect();
                        d.segments_for(&pts)
                    })
                    .max()
                    .unwrap_or(1)
                    .max(2)
            };
            for i in 0..n {
                out.push(w[0] + (w[1] - w[0]) * i as f64 / n as f64);
            }
        }
        if let Some(last) = spans.last() {
            out.push(*last);
        }
        out
    }

    /// Smooth-shading mesh of the whole surface.
    pub fn display_mesh(&self, d: &MeshDensity) -> Mesh {
        let us = self.display_params(true, d);
        let vs = self.display_params(false, d);
        self.tessellate_params(&us, &vs)
    }
}

impl Face {
    /// A trim loop subdivided for display: at least the shared per-span rule, more where the
    /// edge turns faster than the density allows.
    pub fn display_loop(&self, l: &[DVec2], d: &MeshDensity) -> Vec<DVec2> {
        let s = &self.surface;
        let planar = s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2;
        if planar || l.len() < 2 {
            return l.to_vec();
        }
        let mut out = Vec::new();
        for k in 0..l.len() {
            let (a, b) = (l[k], l[(k + 1) % l.len()]);
            let rule = self.segment_subdivisions(a, b);
            let n = rule.max(d.segments_for(&uv_segment_points(s, a, b, 16)));
            for i in 0..n {
                out.push(a + (b - a) * (i as f64 / n as f64));
            }
        }
        out
    }

    /// Smooth-shading mesh of the face.
    pub fn display_mesh(&self, d: &MeshDensity) -> Mesh {
        if self.loops.is_empty() {
            return self.surface.display_mesh(d);
        }
        let s = &self.surface;
        let planar = s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2;
        if planar {
            return self.tessellate(1, 1);
        }
        let us = s.display_params(true, d);
        let vs = s.display_params(false, d);
        let loops: Vec<Vec<DVec2>> = self.loops.iter().map(|l| self.display_loop(l, d)).collect();
        let (uv, tris) = self.trimmed_triangulation(&loops, &us, &vs);
        self.mesh_from_uv(&uv, &tris)
    }

    /// Edge polylines within `tol` of the true edges: trim segments are subdivided until
    /// their 3-D chords stay within the tolerance.
    pub fn display_edges(&self, tol: f64) -> Vec<Vec<DVec3>> {
        if self.loops.is_empty() {
            return self.surface.boundary_curves().iter().map(|c| c.tessellate_adaptive(tol)).collect();
        }
        let s = &self.surface;
        self.loops
            .iter()
            .map(|l| {
                let base = self.subdivided_loop(l);
                let mut pts = Vec::with_capacity(base.len() * 2);
                for k in 0..base.len() {
                    let (a, b) = (base[k], base[(k + 1) % base.len()]);
                    let (pa, pb) = (s.point_at(a.x, a.y), s.point_at(b.x, b.y));
                    pts.push(pa);
                    refine_uv_segment(s, a, b, pa, pb, tol, 0, &mut pts);
                }
                if let Some(f) = pts.first().copied() {
                    pts.push(f);
                }
                pts
            })
            .collect()
    }
}

/// Polyline of the straight (u, v) segment `a`-`b` mapped onto the surface, within `tol`.
pub fn uv_polyline(s: &NurbsSurface, a: DVec2, b: DVec2, tol: f64) -> Vec<DVec3> {
    let (pa, pb) = (s.point_at(a.x, a.y), s.point_at(b.x, b.y));
    let mut out = vec![pa];
    // start from 8 pieces so that S-shaped spans are not taken for straight ones
    let n = 8;
    let mut prev = (a, pa);
    for i in 1..=n {
        let uv = a + (b - a) * (i as f64 / n as f64);
        let p = s.point_at(uv.x, uv.y);
        refine_uv_segment(s, prev.0, uv, prev.1, p, tol, 0, &mut out);
        out.push(p);
        prev = (uv, p);
    }
    let _ = pb;
    out
}

/// Emit the points strictly between `a` and `b` needed to keep chords within `tol`.
fn refine_uv_segment(s: &NurbsSurface, a: DVec2, b: DVec2, pa: DVec3, pb: DVec3, tol: f64, depth: usize, out: &mut Vec<DVec3>) {
    if depth >= 12 {
        return;
    }
    let chord = pb - pa;
    let len2 = chord.length_squared();
    let dev = |f: f64| {
        let uv = a + (b - a) * f;
        let q = s.point_at(uv.x, uv.y);
        let t = if len2 < 1e-24 { 0.0 } else { ((q - pa).dot(chord) / len2).clamp(0.0, 1.0) };
        (q - (pa + chord * t)).length()
    };
    if dev(0.25).max(dev(0.5)).max(dev(0.75)) <= tol {
        return;
    }
    let m = (a + b) * 0.5;
    let pm = s.point_at(m.x, m.y);
    refine_uv_segment(s, a, m, pa, pm, tol, depth + 1, out);
    out.push(pm);
    refine_uv_segment(s, m, b, pm, pb, tol, depth + 1, out);
}

impl Brep {
    /// Smooth-shading mesh of every face.
    pub fn display_mesh(&self, d: &MeshDensity) -> Mesh {
        let mut m = Mesh::new();
        for f in &self.faces {
            m.append(&f.display_mesh(d));
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Plane;

    /// Largest distance of a mesh edge midpoint from a cylinder of radius `r` around Z.
    fn max_sag(m: &Mesh, r: f64) -> f64 {
        let mut worst: f64 = 0.0;
        for t in m.indices.chunks(3) {
            for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                let (a, b) = (m.vertices[t[i] as usize], m.vertices[t[j] as usize]);
                let mid = (a + b) * 0.5;
                worst = worst.max(r - DVec3::new(mid.x, mid.y, 0.0).length());
            }
        }
        worst
    }

    #[test]
    fn cylinder_display_mesh_follows_curvature() {
        let r = 10.0;
        let s = NurbsSurface::cylinder(&Plane::world_xy(), r, 5.0).unwrap();
        let d = MeshDensity::for_size(s.bounding_box().diagonal());
        // the fixed 8-per-span grid is visibly faceted: 11.25 degrees per segment
        assert!(max_sag(&s.tessellate(8, 8), r) > d.chord * 5.0);
        let m = s.display_mesh(&d);
        assert!(max_sag(&m, r) <= d.chord * 1.01, "sag {}", max_sag(&m, r));
        // the straight direction stays a single segment per span
        let vs = s.display_params(false, &d);
        assert_eq!(vs.len(), 2);
    }

    #[test]
    fn trimmed_curved_face_mesh_and_edges_follow_curvature() {
        let r = 10.0;
        let s = NurbsSurface::cylinder(&Plane::world_xy(), r, 5.0).unwrap();
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let vm = (v0 + v1) * 0.5;
        // lower half of the cylinder: two long straight UV segments run around the circle
        let face = Face { surface: s.clone(), loops: vec![vec![DVec2::new(u0, v0), DVec2::new(u1, v0), DVec2::new(u1, vm), DVec2::new(u0, vm)]] };
        let d = MeshDensity::for_size(s.bounding_box().diagonal());
        let m = face.display_mesh(&d);
        assert!(!m.indices.is_empty());
        assert!(max_sag(&m, r) <= d.chord * 1.01, "mesh sag {}", max_sag(&m, r));
        let tol = 1e-3;
        for poly in face.display_edges(tol) {
            for w in poly.windows(2) {
                let mid = (w[0] + w[1]) * 0.5;
                let sag = r - DVec3::new(mid.x, mid.y, 0.0).length();
                assert!(sag <= tol * 1.01, "edge sag {sag}");
            }
        }
    }
}
