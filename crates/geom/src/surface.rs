use glam::{DAffine3, DVec3, DVec4};
use serde::{Deserialize, Serialize};

use crate::basis::{basis_funs, ders_basis_funs, find_span, uniform_clamped_knots, validate_knots};
use crate::curve::{averaged_knots, intersect_lines};
use crate::{BoundingBox, GeomError, Mesh, NurbsCurve, Plane, Result, KNOT_TOL, TOL};

/// A NURBS surface. Control points are stored row-major: index = i * count_v + j.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NurbsSurface {
    pub degree_u: usize,
    pub degree_v: usize,
    pub count_u: usize,
    pub count_v: usize,
    pub control_points: Vec<DVec3>,
    pub weights: Vec<f64>,
    pub knots_u: Vec<f64>,
    pub knots_v: Vec<f64>,
}

impl NurbsSurface {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        degree_u: usize,
        degree_v: usize,
        count_u: usize,
        count_v: usize,
        control_points: Vec<DVec3>,
        weights: Vec<f64>,
        knots_u: Vec<f64>,
        knots_v: Vec<f64>,
    ) -> Result<Self> {
        if control_points.len() != count_u * count_v || weights.len() != control_points.len() {
            return Err(GeomError::Invalid("surface control point / weight count mismatch".into()));
        }
        validate_knots(count_u, degree_u, &knots_u)?;
        validate_knots(count_v, degree_v, &knots_v)?;
        Ok(Self { degree_u, degree_v, count_u, count_v, control_points, weights, knots_u, knots_v })
    }

    /// Non-rational surface from a grid `grid[i][j]` with uniform clamped knots.
    pub fn from_grid(degree_u: usize, degree_v: usize, grid: Vec<Vec<DVec3>>) -> Result<Self> {
        let cu = grid.len();
        let cv = grid.first().map_or(0, |r| r.len());
        if cu < degree_u + 1 || cv < degree_v + 1 {
            return Err(GeomError::NotEnoughPoints { need: (degree_u + 1).max(degree_v + 1), got: cu.min(cv) });
        }
        if grid.iter().any(|r| r.len() != cv) {
            return Err(GeomError::Invalid("ragged control point grid".into()));
        }
        let cps: Vec<DVec3> = grid.into_iter().flatten().collect();
        let n = cps.len();
        Self::new(
            degree_u,
            degree_v,
            cu,
            cv,
            cps,
            vec![1.0; n],
            uniform_clamped_knots(cu, degree_u),
            uniform_clamped_knots(cv, degree_v),
        )
    }

    /// Planar rectangle in `plane` from (u0, v0) to (u1, v1).
    pub fn plane(plane: &Plane, u0: f64, u1: f64, v0: f64, v1: f64) -> Result<Self> {
        if (u1 - u0).abs() < TOL || (v1 - v0).abs() < TOL {
            return Err(GeomError::Degenerate("zero-area plane".into()));
        }
        let grid = vec![
            vec![plane.point_at(u0, v0), plane.point_at(u0, v1)],
            vec![plane.point_at(u1, v0), plane.point_at(u1, v1)],
        ];
        Self::from_grid(1, 1, grid)
    }

    /// Extrude a curve along a vector (u = curve direction, v = extrusion).
    pub fn extrude(curve: &NurbsCurve, dir: DVec3) -> Result<Self> {
        if dir.length_squared() < TOL * TOL {
            return Err(GeomError::Degenerate("extrusion vector is zero".into()));
        }
        let cu = curve.control_points.len();
        let mut cps = Vec::with_capacity(cu * 2);
        let mut ws = Vec::with_capacity(cu * 2);
        for (p, w) in curve.control_points.iter().zip(&curve.weights) {
            cps.push(*p);
            cps.push(*p + dir);
            ws.push(*w);
            ws.push(*w);
        }
        Self::new(curve.degree, 1, cu, 2, cps, ws, curve.knots.clone(), vec![0.0, 0.0, 1.0, 1.0])
    }

    /// Revolve a profile curve around an axis (NURBS Book A8.1).
    /// u = angular direction (degree 2, rational), v = profile direction.
    pub fn revolve(profile: &NurbsCurve, axis_origin: DVec3, axis_dir: DVec3, angle: f64) -> Result<Self> {
        let t_axis = axis_dir.normalize_or(DVec3::Z);
        let theta = angle.abs().min(std::f64::consts::TAU);
        if theta < 1e-9 {
            return Err(GeomError::Degenerate("revolve angle is zero".into()));
        }
        let narcs = ((theta / std::f64::consts::FRAC_PI_2) - 1e-9).floor() as usize + 1;
        let dtheta = theta / narcs as f64 * angle.signum();
        let n_u = 2 * narcs + 1;
        let mut knots_u = vec![0.0, 0.0, 0.0];
        for i in 1..narcs {
            let v = i as f64 / narcs as f64;
            knots_u.push(v);
            knots_u.push(v);
        }
        knots_u.extend_from_slice(&[1.0, 1.0, 1.0]);
        let wm = (dtheta / 2.0).cos();
        let m = profile.control_points.len();
        let mut cps = vec![DVec3::ZERO; n_u * m];
        let mut ws = vec![1.0; n_u * m];
        for j in 0..m {
            let pj = profile.control_points[j];
            let wj = profile.weights[j];
            let o = axis_origin + t_axis * (pj - axis_origin).dot(t_axis);
            let mut x = pj - o;
            let r = x.length();
            let mut y = t_axis.cross(x);
            if r > TOL {
                x /= r;
                y = y.normalize();
            }
            let mut p0 = pj;
            let mut t0 = y;
            cps[j] = pj;
            ws[j] = wj;
            let mut index = 0;
            let mut ang = 0.0;
            for _ in 0..narcs {
                ang += dtheta;
                let (s, c) = ang.sin_cos();
                let p2 = o + x * (r * c) + y * (r * s);
                cps[(index + 2) * m + j] = p2;
                ws[(index + 2) * m + j] = wj;
                let t2 = x * (-s) + y * c;
                let p1 = if r > TOL { intersect_lines(p0, t0, p2, t2).unwrap_or(p0) } else { p0 };
                cps[(index + 1) * m + j] = p1;
                ws[(index + 1) * m + j] = wm * wj;
                index += 2;
                p0 = p2;
                t0 = t2;
            }
        }
        Self::new(2, profile.degree, n_u, m, cps, ws, knots_u, profile.knots.clone())
    }

    pub fn sphere(center: DVec3, radius: f64) -> Result<Self> {
        // profile: semicircle in the XZ plane from south pole to north pole
        let plane = Plane { origin: center, x_axis: DVec3::X, y_axis: DVec3::Z, z_axis: -DVec3::Y };
        let profile = NurbsCurve::arc(&plane, radius, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2)?;
        Self::revolve(&profile, center, DVec3::Z, std::f64::consts::TAU)
    }

    pub fn cylinder(plane: &Plane, radius: f64, height: f64) -> Result<Self> {
        let circle = NurbsCurve::circle(plane, radius)?;
        Self::extrude(&circle, plane.z_axis * height)
    }

    /// Flat circular disc (revolved radial line); used to cap cylinders and extrusions.
    pub fn disc(plane: &Plane, radius: f64) -> Result<Self> {
        let profile = NurbsCurve::line(plane.origin, plane.point_at(radius, 0.0))?;
        Self::revolve(&profile, plane.origin, plane.z_axis, std::f64::consts::TAU)
    }

    pub fn cone(plane: &Plane, radius: f64, height: f64) -> Result<Self> {
        let profile = NurbsCurve::line(plane.point_at(radius, 0.0), plane.point_at_3(0.0, 0.0, height))?;
        Self::revolve(&profile, plane.origin, plane.z_axis, std::f64::consts::TAU)
    }

    pub fn torus(plane: &Plane, major_radius: f64, minor_radius: f64) -> Result<Self> {
        let section_plane = Plane {
            origin: plane.point_at(major_radius, 0.0),
            x_axis: plane.x_axis,
            y_axis: plane.z_axis,
            z_axis: -plane.y_axis,
        };
        let circle = NurbsCurve::circle(&section_plane, minor_radius)?;
        Self::revolve(&circle, plane.origin, plane.z_axis, std::f64::consts::TAU)
    }

    /// Loft through section curves. Curves are made compatible by refitting
    /// when their degree/knots differ.
    pub fn loft(curves: &[NurbsCurve], degree_v: usize) -> Result<Self> {
        let k = curves.len();
        if k < 2 {
            return Err(GeomError::NotEnoughPoints { need: 2, got: k });
        }
        let curves = make_compatible(curves)?;
        let cu = curves[0].control_points.len();
        let degree_v = degree_v.min(k - 1).max(1);
        // average chord-length parameters across all control point columns
        let mut params = vec![0.0; k];
        for i in 0..cu {
            let mut acc = 0.0;
            let mut local = vec![0.0; k];
            for s in 1..k {
                acc += (curves[s].control_points[i] - curves[s - 1].control_points[i]).length();
                local[s] = acc;
            }
            if acc > TOL {
                for s in 0..k {
                    params[s] += local[s] / acc / cu as f64;
                }
            } else {
                for s in 0..k {
                    params[s] += s as f64 / (k - 1) as f64 / cu as f64;
                }
            }
        }
        params[0] = 0.0;
        params[k - 1] = 1.0;
        let mut cps = vec![DVec3::ZERO; cu * k];
        let mut ws = vec![1.0; cu * k];
        let mut knots_v = Vec::new();
        for i in 0..cu {
            let column: Vec<DVec3> = curves.iter().map(|c| c.control_points[i]).collect();
            let col_curve = if degree_v == 1 {
                let kn = averaged_knots(&params, 1);
                NurbsCurve::new(1, column, vec![1.0; k], kn)?
            } else {
                NurbsCurve::interpolate_params(&column, degree_v, &params)?
            };
            for s in 0..k {
                cps[i * k + s] = col_curve.control_points[s];
                ws[i * k + s] = curves[s].weights[i];
            }
            knots_v = col_curve.knots.clone();
        }
        Self::new(curves[0].degree, degree_v, cu, k, cps, ws, curves[0].knots.clone(), knots_v)
    }

    /// Global surface interpolation through a grid of points `grid[i][j]` (i along u, j along v)
    /// with uniform parameters (NURBS Book A9.4, parameters shared across rows).
    pub fn interpolate_grid(grid: &[Vec<DVec3>], degree_u: usize, degree_v: usize) -> Result<Self> {
        let nu = grid.len();
        let nv = grid.first().map_or(0, |r| r.len());
        if nu < 2 || nv < 2 || grid.iter().any(|r| r.len() != nv) {
            return Err(GeomError::NotEnoughPoints { need: 2, got: nu.min(nv) });
        }
        let du = degree_u.min(nu - 1).max(1);
        let dv = degree_v.min(nv - 1).max(1);
        let pu: Vec<f64> = (0..nu).map(|i| i as f64 / (nu - 1) as f64).collect();
        let pv: Vec<f64> = (0..nv).map(|j| j as f64 / (nv - 1) as f64).collect();
        // interpolate each column (fixed j) along u -> intermediate control points R[k][j]
        let mut r: Vec<Vec<DVec3>> = vec![vec![DVec3::ZERO; nv]; nu];
        let mut knots_u = Vec::new();
        for j in 0..nv {
            let col: Vec<DVec3> = (0..nu).map(|i| grid[i][j]).collect();
            let c = NurbsCurve::interpolate_params(&col, du, &pu)?;
            knots_u = c.knots.clone();
            for i in 0..nu {
                r[i][j] = c.control_points[i];
            }
        }
        // then each row along v
        let mut cps = vec![DVec3::ZERO; nu * nv];
        let mut knots_v = Vec::new();
        for i in 0..nu {
            let c = NurbsCurve::interpolate_params(&r[i], dv, &pv)?;
            knots_v = c.knots.clone();
            for j in 0..nv {
                cps[i * nv + j] = c.control_points[j];
            }
        }
        Self::new(du, dv, nu, nv, cps, vec![1.0; nu * nv], knots_u, knots_v)
    }

    /// Bilinearly blended Coons patch through four boundary curves given in order
    /// (bottom u, right v, top u reversed, left v reversed), sampled and interpolated.
    pub fn coons(c0: &NurbsCurve, c1: &NurbsCurve, c2: &NurbsCurve, c3: &NurbsCurve, samples: usize) -> Result<Self> {
        let n = samples.max(4);
        let at = |c: &NurbsCurve, t: f64| {
            let (a, b) = c.domain();
            c.point_at(a + (b - a) * t)
        };
        // corners: p00 = c0 start, p10 = c0 end, p11 = c2 start (top end), p01 = c2 end
        let p00 = at(c0, 0.0);
        let p10 = at(c0, 1.0);
        let p11 = at(c1, 1.0);
        let p01 = at(c3, 0.0);
        let mut grid = Vec::with_capacity(n + 1);
        for i in 0..=n {
            let u = i as f64 / n as f64;
            let mut row = Vec::with_capacity(n + 1);
            for j in 0..=n {
                let v = j as f64 / n as f64;
                let bottom = at(c0, u);
                let top = at(c2, 1.0 - u);
                let right = at(c1, v);
                let left = at(c3, 1.0 - v);
                let p = bottom * (1.0 - v) + top * v + left * (1.0 - u) + right * u
                    - (p00 * (1.0 - u) * (1.0 - v) + p10 * u * (1.0 - v) + p01 * (1.0 - u) * v + p11 * u * v);
                row.push(p);
            }
            grid.push(row);
        }
        Self::interpolate_grid(&grid, 3, 3)
    }

    pub fn domain_u(&self) -> (f64, f64) {
        (self.knots_u[self.degree_u], self.knots_u[self.knots_u.len() - 1 - self.degree_u])
    }

    pub fn domain_v(&self) -> (f64, f64) {
        (self.knots_v[self.degree_v], self.knots_v[self.knots_v.len() - 1 - self.degree_v])
    }

    #[inline]
    fn homogeneous(&self, i: usize, j: usize) -> DVec4 {
        let idx = i * self.count_v + j;
        let w = self.weights[idx];
        let p = self.control_points[idx];
        DVec4::new(p.x * w, p.y * w, p.z * w, w)
    }

    pub fn point_at(&self, u: f64, v: f64) -> DVec3 {
        let (u0, u1) = self.domain_u();
        let (v0, v1) = self.domain_v();
        let u = u.clamp(u0, u1);
        let v = v.clamp(v0, v1);
        let su = find_span(self.count_u - 1, self.degree_u, u, &self.knots_u);
        let sv = find_span(self.count_v - 1, self.degree_v, v, &self.knots_v);
        let nu = basis_funs(su, u, self.degree_u, &self.knots_u);
        let nv = basis_funs(sv, v, self.degree_v, &self.knots_v);
        let mut acc = DVec4::ZERO;
        for (a, nua) in nu.iter().enumerate() {
            let mut row = DVec4::ZERO;
            for (b, nvb) in nv.iter().enumerate() {
                row += self.homogeneous(su - self.degree_u + a, sv - self.degree_v + b) * *nvb;
            }
            acc += row * *nua;
        }
        acc.truncate() / acc.w
    }

    /// Point and the two first partial derivatives.
    pub fn evaluate(&self, u: f64, v: f64) -> (DVec3, DVec3, DVec3) {
        let (u0, u1) = self.domain_u();
        let (v0, v1) = self.domain_v();
        let u = u.clamp(u0, u1);
        let v = v.clamp(v0, v1);
        let su = find_span(self.count_u - 1, self.degree_u, u, &self.knots_u);
        let sv = find_span(self.count_v - 1, self.degree_v, v, &self.knots_v);
        let du = ders_basis_funs(su, u, self.degree_u, 1, &self.knots_u);
        let dv = ders_basis_funs(sv, v, self.degree_v, 1, &self.knots_v);
        let mut a = DVec4::ZERO;
        let mut au = DVec4::ZERO;
        let mut av = DVec4::ZERO;
        for i in 0..=self.degree_u {
            for j in 0..=self.degree_v {
                let h = self.homogeneous(su - self.degree_u + i, sv - self.degree_v + j);
                a += h * (du[0][i] * dv[0][j]);
                au += h * (du[1][i] * dv[0][j]);
                av += h * (du[0][i] * dv[1][j]);
            }
        }
        let s = a.truncate() / a.w;
        let s_u = (au.truncate() - s * au.w) / a.w;
        let s_v = (av.truncate() - s * av.w) / a.w;
        (s, s_u, s_v)
    }

    pub fn normal_at(&self, u: f64, v: f64) -> DVec3 {
        let (_, su, sv) = self.evaluate(u, v);
        let n = su.cross(sv);
        if n.length_squared() > 1e-20 {
            return n.normalize();
        }
        // degenerate (e.g. sphere pole): nudge inward
        let (u0, u1) = self.domain_u();
        let (v0, v1) = self.domain_v();
        let eu = (u1 - u0) * 1e-4;
        let ev = (v1 - v0) * 1e-4;
        let uu = if u - u0 < eu { u + eu } else if u1 - u < eu { u - eu } else { u };
        let vv = if v - v0 < ev { v + ev } else if v1 - v < ev { v - ev } else { v };
        let (_, su, sv) = self.evaluate(uu, vv);
        su.cross(sv).normalize_or(DVec3::Z)
    }

    fn unique_knots(knots: &[f64], degree: usize) -> Vec<f64> {
        let mut v: Vec<f64> = knots[degree..knots.len() - degree].to_vec();
        v.dedup_by(|a, b| (*a - *b).abs() < KNOT_TOL);
        v
    }

    /// Parameter samples per direction: `segs` subdivisions in each knot span.
    pub fn samples(knots: &[f64], degree: usize, segs: usize) -> Vec<f64> {
        let spans = Self::unique_knots(knots, degree);
        let mut out = Vec::new();
        for w in spans.windows(2) {
            for i in 0..segs {
                out.push(w[0] + (w[1] - w[0]) * i as f64 / segs as f64);
            }
        }
        out.push(*spans.last().unwrap());
        out
    }

    /// Tessellate into a triangle mesh with `segs_u`/`segs_v` subdivisions per knot span.
    pub fn tessellate(&self, segs_u: usize, segs_v: usize) -> Mesh {
        let us = Self::samples(&self.knots_u, self.degree_u, segs_u.max(1));
        let vs = Self::samples(&self.knots_v, self.degree_v, segs_v.max(1));
        self.tessellate_params(&us, &vs)
    }

    /// Grid mesh through the given parameter values (each list sorted, at least two values).
    pub fn tessellate_params(&self, us: &[f64], vs: &[f64]) -> Mesh {
        let mut mesh = Mesh::new();
        mesh.vertices.reserve(us.len() * vs.len());
        mesh.normals.reserve(us.len() * vs.len());
        for &u in us {
            for &v in vs {
                let (p, su, sv) = self.evaluate(u, v);
                let n = su.cross(sv);
                let n = if n.length_squared() > 1e-20 { n.normalize() } else { self.normal_at(u, v) };
                mesh.vertices.push(p);
                mesh.normals.push(n);
            }
        }
        let nv = vs.len() as u32;
        for i in 0..us.len() as u32 - 1 {
            for j in 0..nv - 1 {
                let a = i * nv + j;
                let b = a + 1;
                let c = a + nv;
                let d = c + 1;
                mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        mesh
    }

    /// Isoparametric curve at constant u.
    pub fn isocurve_u(&self, u: f64) -> NurbsCurve {
        let (u0, u1) = self.domain_u();
        let u = u.clamp(u0, u1);
        let su = find_span(self.count_u - 1, self.degree_u, u, &self.knots_u);
        let nu = basis_funs(su, u, self.degree_u, &self.knots_u);
        let mut cps = Vec::with_capacity(self.count_v);
        let mut ws = Vec::with_capacity(self.count_v);
        for j in 0..self.count_v {
            let mut h = DVec4::ZERO;
            for (a, n) in nu.iter().enumerate() {
                h += self.homogeneous(su - self.degree_u + a, j) * *n;
            }
            cps.push(h.truncate() / h.w);
            ws.push(h.w);
        }
        NurbsCurve { degree: self.degree_v, control_points: cps, weights: ws, knots: self.knots_v.clone() }
    }

    /// Isoparametric curve at constant v.
    pub fn isocurve_v(&self, v: f64) -> NurbsCurve {
        let (v0, v1) = self.domain_v();
        let v = v.clamp(v0, v1);
        let sv = find_span(self.count_v - 1, self.degree_v, v, &self.knots_v);
        let nv = basis_funs(sv, v, self.degree_v, &self.knots_v);
        let mut cps = Vec::with_capacity(self.count_u);
        let mut ws = Vec::with_capacity(self.count_u);
        for i in 0..self.count_u {
            let mut h = DVec4::ZERO;
            for (b, n) in nv.iter().enumerate() {
                h += self.homogeneous(i, sv - self.degree_v + b) * *n;
            }
            cps.push(h.truncate() / h.w);
            ws.push(h.w);
        }
        NurbsCurve { degree: self.degree_u, control_points: cps, weights: ws, knots: self.knots_u.clone() }
    }

    /// Four boundary curves (u0, u1, v0, v1).
    pub fn boundary_curves(&self) -> [NurbsCurve; 4] {
        let (u0, u1) = self.domain_u();
        let (v0, v1) = self.domain_v();
        [self.isocurve_u(u0), self.isocurve_u(u1), self.isocurve_v(v0), self.isocurve_v(v1)]
    }

    /// Interior isocurves for display: `n` per direction (excluding boundaries).
    pub fn display_isocurves(&self, n: usize) -> Vec<NurbsCurve> {
        let (u0, u1) = self.domain_u();
        let (v0, v1) = self.domain_v();
        let mut out = Vec::new();
        for i in 1..=n {
            let f = i as f64 / (n + 1) as f64;
            out.push(self.isocurve_u(u0 + (u1 - u0) * f));
            out.push(self.isocurve_v(v0 + (v1 - v0) * f));
        }
        out
    }

    pub fn bounding_box(&self) -> BoundingBox {
        BoundingBox::from_points(&self.control_points)
    }

    pub fn transform(&mut self, xf: &DAffine3) {
        for p in self.control_points.iter_mut() {
            *p = xf.transform_point3(*p);
        }
    }

    /// Surface parameters of the point closest to `p` (sampling plus Newton refinement).
    pub fn closest_uv(&self, p: DVec3) -> (f64, f64) {
        let (u0, u1) = self.domain_u();
        let (v0, v1) = self.domain_v();
        let nu = (self.count_u * 6).max(12);
        let nv = (self.count_v * 6).max(12);
        let mut best = (u0, v0, f64::INFINITY);
        for i in 0..=nu {
            let u = u0 + (u1 - u0) * i as f64 / nu as f64;
            for j in 0..=nv {
                let v = v0 + (v1 - v0) * j as f64 / nv as f64;
                let d = (self.point_at(u, v) - p).length_squared();
                if d < best.2 {
                    best = (u, v, d);
                }
            }
        }
        self.closest_uv_from(p, best.0, best.1)
    }

    /// Newton refinement of the closest point starting from a known parameter pair.
    pub fn closest_uv_from(&self, p: DVec3, u_start: f64, v_start: f64) -> (f64, f64) {
        let (u0, u1) = self.domain_u();
        let (v0, v1) = self.domain_v();
        let (mut u, mut v) = (u_start.clamp(u0, u1), v_start.clamp(v0, v1));
        for _ in 0..40 {
            let (s, su, sv) = self.evaluate(u, v);
            let r = s - p;
            let h = 1e-6;
            let (_, suu, _) = self.evaluate((u + h).min(u1), v);
            let (_, suv, svv) = self.evaluate(u, (v + h).min(v1));
            let s_uu = (suu - su) / h;
            let s_uv = (suv - su) / h;
            let s_vv = (svv - sv) / h;
            let g = [r.dot(su), r.dot(sv)];
            let j00 = su.dot(su) + r.dot(s_uu);
            let j01 = su.dot(sv) + r.dot(s_uv);
            let j11 = sv.dot(sv) + r.dot(s_vv);
            let det = j00 * j11 - j01 * j01;
            if det.abs() < 1e-18 {
                break;
            }
            let du = (j11 * g[0] - j01 * g[1]) / det;
            let dv = (j00 * g[1] - j01 * g[0]) / det;
            let nu_ = (u - du).clamp(u0, u1);
            let nv_ = (v - dv).clamp(v0, v1);
            let moved = (nu_ - u).abs() + (nv_ - v).abs();
            u = nu_;
            v = nv_;
            if moved < 1e-14 {
                break;
            }
        }
        (u, v)
    }

    /// Offset surface by moving control points along the normals at their Greville points.
    pub fn offset(&self, distance: f64) -> Self {
        let mut s = self.clone();
        let greville = |knots: &[f64], degree: usize, i: usize| -> f64 { knots[i + 1..=i + degree].iter().sum::<f64>() / degree as f64 };
        for i in 0..self.count_u {
            for j in 0..self.count_v {
                let u = greville(&self.knots_u, self.degree_u, i);
                let v = greville(&self.knots_v, self.degree_v, j);
                let n = self.normal_at(u, v);
                s.control_points[i * self.count_v + j] += n * distance;
            }
        }
        s
    }

    pub fn flip(&mut self) {
        // swap u and v to reverse the normal
        let mut cps = vec![DVec3::ZERO; self.control_points.len()];
        let mut ws = vec![1.0; self.weights.len()];
        for i in 0..self.count_u {
            for j in 0..self.count_v {
                cps[j * self.count_u + i] = self.control_points[i * self.count_v + j];
                ws[j * self.count_u + i] = self.weights[i * self.count_v + j];
            }
        }
        self.control_points = cps;
        self.weights = ws;
        std::mem::swap(&mut self.degree_u, &mut self.degree_v);
        std::mem::swap(&mut self.count_u, &mut self.count_v);
        std::mem::swap(&mut self.knots_u, &mut self.knots_v);
    }
}

/// Make curves share degree and knot vector. Compatible curves are returned
/// unchanged; otherwise every curve is refitted through sampled points.
pub fn make_compatible(curves: &[NurbsCurve]) -> Result<Vec<NurbsCurve>> {
    let first = &curves[0];
    let compatible = curves.iter().all(|c| {
        c.degree == first.degree
            && c.knots.len() == first.knots.len()
            && c.knots.iter().zip(&first.knots).all(|(a, b)| (a - b).abs() < 1e-9)
    });
    if compatible {
        return Ok(curves.to_vec());
    }
    let samples = 24usize.max(curves.iter().map(|c| c.control_points.len() * 2).max().unwrap_or(0));
    let params: Vec<f64> = (0..=samples).map(|i| i as f64 / samples as f64).collect();
    curves
        .iter()
        .map(|c| {
            let (t0, t1) = c.domain();
            let pts: Vec<DVec3> = params.iter().map(|f| c.point_at(t0 + (t1 - t0) * f)).collect();
            NurbsCurve::interpolate_params(&pts, 3, &params)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sphere_is_exact() {
        let s = NurbsSurface::sphere(DVec3::new(1.0, 2.0, 3.0), 1.5).unwrap();
        for i in 0..=20 {
            for j in 0..=20 {
                let p = s.point_at(i as f64 / 20.0, j as f64 / 20.0);
                assert!(((p - DVec3::new(1.0, 2.0, 3.0)).length() - 1.5).abs() < 1e-10);
            }
        }
    }

    #[test]
    fn cylinder_normals_point_outwards() {
        let s = NurbsSurface::cylinder(&Plane::world_xy(), 1.0, 2.0).unwrap();
        let p = s.point_at(0.1, 0.5);
        let n = s.normal_at(0.1, 0.5);
        assert!(n.dot(DVec3::new(p.x, p.y, 0.0)) > 0.9);
    }

    #[test]
    fn tessellation_has_consistent_counts() {
        let s = NurbsSurface::torus(&Plane::world_xy(), 3.0, 1.0).unwrap();
        let m = s.tessellate(4, 4);
        assert_eq!(m.vertices.len(), m.normals.len());
        assert!(m.triangle_count() > 0);
        assert!(m.indices.iter().all(|i| (*i as usize) < m.vertices.len()));
    }

    #[test]
    fn loft_interpolates_sections() {
        let c0 = NurbsCurve::circle(&Plane::world_xy(), 1.0).unwrap();
        let mut p1 = Plane::world_xy();
        p1.origin.z = 2.0;
        let c1 = NurbsCurve::circle(&p1, 2.0).unwrap();
        let mut p2 = Plane::world_xy();
        p2.origin.z = 4.0;
        let c2 = NurbsCurve::circle(&p2, 1.0).unwrap();
        let s = NurbsSurface::loft(&[c0.clone(), c1.clone(), c2.clone()], 3).unwrap();
        let (v0, v1) = s.domain_v();
        for i in 0..=10 {
            let u = i as f64 / 10.0;
            assert!((s.point_at(u, v0) - c0.point_at(u)).length() < 1e-9);
            assert!((s.point_at(u, v1) - c2.point_at(u)).length() < 1e-9);
        }
    }

    #[test]
    fn coons_patch_matches_boundaries() {
        let c0 = NurbsCurve::line(DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0)).unwrap();
        let _c1 = NurbsCurve::arc(&Plane { origin: DVec3::new(4.0, 2.0, 0.0), x_axis: DVec3::Z, y_axis: -DVec3::Y, z_axis: DVec3::X }, 2.0, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2).unwrap();
        let c1 = NurbsCurve::line(DVec3::new(4.0, 0.0, 0.0), DVec3::new(4.0, 4.0, 1.0)).unwrap();
        let c2 = NurbsCurve::line(DVec3::new(4.0, 4.0, 1.0), DVec3::new(0.0, 4.0, 0.0)).unwrap();
        let c3 = NurbsCurve::line(DVec3::new(0.0, 4.0, 0.0), DVec3::ZERO).unwrap();
        let s = NurbsSurface::coons(&c0, &c1, &c2, &c3, 8).unwrap();
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        assert!((s.point_at(u0, v0) - DVec3::ZERO).length() < 1e-9);
        assert!((s.point_at(u1, v1) - DVec3::new(4.0, 4.0, 1.0)).length() < 1e-9);
        assert!((s.point_at(u0 + (u1 - u0) * 0.5, v0) - DVec3::new(2.0, 0.0, 0.0)).length() < 1e-6);
    }

    #[test]
    fn isocurve_matches_surface() {
        let s = NurbsSurface::sphere(DVec3::ZERO, 1.0).unwrap();
        let c = s.isocurve_u(0.3);
        for j in 0..=10 {
            let v = j as f64 / 10.0;
            assert!((c.point_at(v) - s.point_at(0.3, v)).length() < 1e-10);
        }
    }

    #[test]
    fn incompatible_loft_refits() {
        let line = NurbsCurve::line(DVec3::ZERO, DVec3::X).unwrap();
        let arc = NurbsCurve::arc(&Plane { origin: DVec3::new(0.0, 0.0, 1.0), ..Plane::world_xy() }, 1.0, 0.0, 1.0).unwrap();
        let s = NurbsSurface::loft(&[line, arc], 1).unwrap();
        assert_eq!(s.degree_v, 1);
        assert!((s.point_at(0.0, 0.0) - DVec3::ZERO).length() < 1e-9);
    }
}
