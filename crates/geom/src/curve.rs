use glam::{DAffine3, DVec3, DVec4};
use serde::{Deserialize, Serialize};

use crate::basis::{basis_funs, ders_basis_funs, find_span, uniform_clamped_knots, validate_knots};
use crate::{BoundingBox, GeomError, Plane, Result, TOL};

/// A non-uniform rational B-spline curve with a clamped knot vector.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NurbsCurve {
    pub degree: usize,
    pub control_points: Vec<DVec3>,
    pub weights: Vec<f64>,
    pub knots: Vec<f64>,
}

impl NurbsCurve {
    pub fn new(degree: usize, control_points: Vec<DVec3>, weights: Vec<f64>, knots: Vec<f64>) -> Result<Self> {
        if weights.len() != control_points.len() {
            return Err(GeomError::Invalid("weights/control point count mismatch".into()));
        }
        if weights.iter().any(|w| *w <= 0.0) {
            return Err(GeomError::Invalid("weights must be positive".into()));
        }
        validate_knots(control_points.len(), degree, &knots)?;
        Ok(Self { degree, control_points, weights, knots })
    }

    /// Non-rational curve with a uniform clamped knot vector.
    pub fn from_control_points(degree: usize, control_points: Vec<DVec3>) -> Result<Self> {
        let n = control_points.len();
        if n < degree + 1 {
            return Err(GeomError::NotEnoughPoints { need: degree + 1, got: n });
        }
        let knots = uniform_clamped_knots(n, degree);
        Self::new(degree, control_points, vec![1.0; n], knots)
    }

    pub fn line(a: DVec3, b: DVec3) -> Result<Self> {
        if (a - b).length_squared() < TOL * TOL {
            return Err(GeomError::Degenerate("line endpoints coincide".into()));
        }
        Self::new(1, vec![a, b], vec![1.0, 1.0], vec![0.0, 0.0, 1.0, 1.0])
    }

    /// Degree-1 curve through the points; parameterised by chord length.
    pub fn polyline(points: Vec<DVec3>) -> Result<Self> {
        if points.len() < 2 {
            return Err(GeomError::NotEnoughPoints { need: 2, got: points.len() });
        }
        let mut knots = vec![0.0, 0.0];
        let mut acc = 0.0;
        for w in points.windows(2) {
            acc += (w[1] - w[0]).length().max(TOL);
            knots.push(acc);
        }
        knots.push(acc);
        let n = points.len();
        Self::new(1, points, vec![1.0; n], knots)
    }

    /// Full circle as a degree-2 rational curve with 9 control points.
    pub fn circle(plane: &Plane, radius: f64) -> Result<Self> {
        if radius <= TOL {
            return Err(GeomError::Degenerate("circle radius must be positive".into()));
        }
        let w = std::f64::consts::FRAC_1_SQRT_2;
        let r = radius;
        let p = |u: f64, v: f64| plane.point_at(u * r, v * r);
        let control_points = vec![
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 1.0),
            p(-1.0, 1.0),
            p(-1.0, 0.0),
            p(-1.0, -1.0),
            p(0.0, -1.0),
            p(1.0, -1.0),
            p(1.0, 0.0),
        ];
        let weights = vec![1.0, w, 1.0, w, 1.0, w, 1.0, w, 1.0];
        let knots = vec![0.0, 0.0, 0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0, 1.0, 1.0];
        Self::new(2, control_points, weights, knots)
    }

    /// Circular arc in `plane` from `start_angle` to `end_angle` (radians, CCW).
    pub fn arc(plane: &Plane, radius: f64, start_angle: f64, end_angle: f64) -> Result<Self> {
        if radius <= TOL {
            return Err(GeomError::Degenerate("arc radius must be positive".into()));
        }
        let mut sweep = end_angle - start_angle;
        if sweep.abs() < 1e-9 {
            return Err(GeomError::Degenerate("arc sweep is zero".into()));
        }
        let two_pi = std::f64::consts::TAU;
        if sweep > two_pi {
            sweep = two_pi;
        }
        if sweep < -two_pi {
            sweep = -two_pi;
        }
        // NURBS Book A7.1: build with n_arcs segments of <= 90 degrees
        let n_arcs = ((sweep.abs() / std::f64::consts::FRAC_PI_2) - 1e-9).floor() as usize + 1;
        let d_theta = sweep / n_arcs as f64;
        let w1 = (d_theta / 2.0).cos();
        let mut cps = Vec::with_capacity(2 * n_arcs + 1);
        let mut ws = Vec::with_capacity(2 * n_arcs + 1);
        let pt = |a: f64| plane.point_at(radius * a.cos(), radius * a.sin());
        let tan = |a: f64| plane.x_axis * (-a.sin()) + plane.y_axis * a.cos();
        let mut p0 = pt(start_angle);
        let mut t0 = tan(start_angle);
        cps.push(p0);
        ws.push(1.0);
        let mut angle = start_angle;
        for _ in 0..n_arcs {
            angle += d_theta;
            let p2 = pt(angle);
            let t2 = tan(angle);
            // intersect tangents to get middle control point
            let p1 = intersect_lines(p0, t0, p2, t2).unwrap_or((p0 + p2) * 0.5);
            cps.push(p1);
            ws.push(w1);
            cps.push(p2);
            ws.push(1.0);
            p0 = p2;
            t0 = t2;
        }
        let mut knots = vec![0.0, 0.0, 0.0];
        for i in 1..n_arcs {
            let v = i as f64 / n_arcs as f64;
            knots.push(v);
            knots.push(v);
        }
        knots.extend_from_slice(&[1.0, 1.0, 1.0]);
        Self::new(2, cps, ws, knots)
    }

    /// Arc through three points.
    pub fn arc_3pt(a: DVec3, b: DVec3, c: DVec3) -> Result<Self> {
        let (center, radius, plane) = circle_through(a, b, c)?;
        let ang = |p: DVec3| {
            let (u, v, _) = plane.closest_parameters(p);
            v.atan2(u)
        };
        let _ = center;
        let a0 = ang(a);
        let mut a1 = ang(b);
        let mut a2 = ang(c);
        // unwrap so that a0 -> a1 -> a2 is monotonic
        while a1 < a0 {
            a1 += std::f64::consts::TAU;
        }
        while a2 < a1 {
            a2 += std::f64::consts::TAU;
        }
        Self::arc(&plane, radius, a0, a2)
    }

    /// Circle through three points.
    pub fn circle_3pt(a: DVec3, b: DVec3, c: DVec3) -> Result<Self> {
        let (_, radius, plane) = circle_through(a, b, c)?;
        Self::circle(&plane, radius)
    }

    /// Global interpolation through points with chord-length parameterisation.
    pub fn interpolate(points: &[DVec3], degree: usize) -> Result<Self> {
        let n = points.len();
        if n < 2 {
            return Err(GeomError::NotEnoughPoints { need: 2, got: n });
        }
        let mut params = vec![0.0; n];
        let mut total = 0.0;
        for i in 1..n {
            total += (points[i] - points[i - 1]).length();
            params[i] = total;
        }
        if total < TOL {
            return Err(GeomError::Degenerate("all interpolation points coincide".into()));
        }
        for p in params.iter_mut() {
            *p /= total;
        }
        Self::interpolate_params(points, degree, &params)
    }

    /// Global interpolation with caller-supplied parameters in [0, 1]
    /// (NURBS Book A9.1 with knot averaging, eq. 9.8).
    pub fn interpolate_params(points: &[DVec3], degree: usize, params: &[f64]) -> Result<Self> {
        let n = points.len();
        if n < 2 {
            return Err(GeomError::NotEnoughPoints { need: 2, got: n });
        }
        if params.len() != n {
            return Err(GeomError::Invalid("parameter count mismatch".into()));
        }
        let degree = degree.min(n - 1).max(1);
        if degree == 1 {
            return Self::polyline(points.to_vec());
        }
        let knots = averaged_knots(params, degree);
        let mut a = vec![vec![0.0; n]; n];
        for (i, &u) in params.iter().enumerate() {
            let span = find_span(n - 1, degree, u, &knots);
            let b = basis_funs(span, u, degree, &knots);
            for (j, bj) in b.iter().enumerate() {
                a[i][span - degree + j] = *bj;
            }
        }
        let cps = crate::solve_dense(a, points.to_vec())?;
        Self::new(degree, cps, vec![1.0; n], knots)
    }

    /// Smoothly closed (periodic) curve through control points: the first `degree`
    /// control points are repeated and a uniform unclamped knot vector is used.
    pub fn periodic(degree: usize, mut control_points: Vec<DVec3>) -> Result<Self> {
        let n = control_points.len();
        if n < degree + 1 {
            return Err(GeomError::NotEnoughPoints { need: degree + 1, got: n });
        }
        let degree = degree.max(1);
        for i in 0..degree {
            let p = control_points[i];
            control_points.push(p);
        }
        let m = control_points.len() + degree + 1;
        let knots: Vec<f64> = (0..m).map(|i| i as f64 - degree as f64).collect();
        let cnt = control_points.len();
        Self::new(degree, control_points, vec![1.0; cnt], knots)
    }

    /// Ellipse from a plane and the two semi-axis lengths (exact rational curve).
    pub fn ellipse(plane: &Plane, a: f64, b: f64) -> Result<Self> {
        if a <= TOL || b <= TOL {
            return Err(GeomError::Degenerate("ellipse radii must be positive".into()));
        }
        let mut c = Self::circle(plane, 1.0)?;
        for p in c.control_points.iter_mut() {
            let (u, v, w) = plane.closest_parameters(*p);
            *p = plane.point_at_3(u * a, v * b, w);
        }
        Ok(c)
    }

    /// Insert knot `t` `r` times (NURBS Book A5.1).
    pub fn insert_knot(&mut self, t: f64, r: usize) {
        let p = self.degree;
        let n = self.control_points.len() - 1;
        let k = find_span(n, p, t, &self.knots);
        let mut s = 0; // existing multiplicity
        for kv in &self.knots {
            if (kv - t).abs() < crate::KNOT_TOL {
                s += 1;
            }
        }
        let r = r.min(p - s.min(p));
        if r == 0 {
            return;
        }
        let hom: Vec<DVec4> = (0..=n).map(|i| self.homogeneous(i)).collect();
        let mut new_knots = Vec::with_capacity(self.knots.len() + r);
        new_knots.extend_from_slice(&self.knots[..=k]);
        for _ in 0..r {
            new_knots.push(t);
        }
        new_knots.extend_from_slice(&self.knots[k + 1..]);
        let mut q = vec![DVec4::ZERO; n + r + 1];
        q[..=k - p].copy_from_slice(&hom[..=k - p]);
        q[k - s + r..=n + r].copy_from_slice(&hom[k - s..=n]);
        let mut rw: Vec<DVec4> = (0..=p - s).map(|i| hom[k - p + i]).collect();
        let mut l = 0;
        for j in 1..=r {
            l = k - p + j;
            for i in 0..=p - j - s {
                let alpha = (t - self.knots[l + i]) / (self.knots[i + k + 1] - self.knots[l + i]);
                rw[i] = rw[i + 1] * alpha + rw[i] * (1.0 - alpha);
            }
            q[l] = rw[0];
            q[k + r - j - s] = rw[p - j - s];
        }
        if l + 1 < k - s {
            q[l + 1..k - s].copy_from_slice(&rw[1..k - s - l]);
        }
        self.control_points = q.iter().map(|h| h.truncate() / h.w).collect();
        self.weights = q.iter().map(|h| h.w).collect();
        self.knots = new_knots;
    }

    /// Split the curve at parameter `t` into two clamped curves.
    pub fn split(&self, t: f64) -> Option<(Self, Self)> {
        let (t0, t1) = self.domain();
        if t <= t0 + crate::KNOT_TOL || t >= t1 - crate::KNOT_TOL {
            return None;
        }
        let mut c = self.clone();
        c.insert_knot(t, self.degree);
        let p = self.degree;
        // index of first knot equal to t
        let first = c.knots.iter().position(|k| (k - t).abs() < crate::KNOT_TOL)?;
        // left: control points 0..first, knots 0..first+p, plus t repeated p+1
        let left_cp = c.control_points[..first].to_vec();
        let left_w = c.weights[..first].to_vec();
        let mut left_k = c.knots[..first].to_vec();
        left_k.extend(std::iter::repeat_n(t, p + 1));
        let right_cp = c.control_points[first - 1..].to_vec();
        let right_w = c.weights[first - 1..].to_vec();
        let mut right_k = vec![t; p + 1];
        right_k.extend_from_slice(&c.knots[first + p..]);
        let l = Self::new(p, left_cp, left_w, left_k).ok()?;
        let r = Self::new(p, right_cp, right_w, right_k).ok()?;
        Some((l, r))
    }

    /// Join another curve to the end of this one. Both must have the same degree and
    /// the end of `self` must coincide with the start of `other` (or its end, in which
    /// case `other` is reversed).
    pub fn join(&self, other: &Self) -> Result<Self> {
        if self.degree != other.degree {
            return Err(GeomError::Incompatible("curves have different degrees".into()));
        }
        let mut o = other.clone();
        let tol = 1e-6;
        if (self.end_point() - o.start_point()).length() > tol {
            if (self.end_point() - o.end_point()).length() <= tol {
                o.reverse();
            } else {
                return Err(GeomError::Incompatible("curve ends do not meet".into()));
            }
        }
        let (a0, a1) = self.domain();
        let (b0, b1) = o.domain();
        let shift = a1 - b0;
        let mut cps = self.control_points.clone();
        let mut ws = self.weights.clone();
        cps.extend_from_slice(&o.control_points[1..]);
        ws.extend_from_slice(&o.weights[1..]);
        let p = self.degree;
        let mut knots = self.knots[..self.knots.len() - 1].to_vec(); // drop one end multiplicity
        knots.extend(o.knots[p + 1..].iter().map(|k| k + shift));
        let _ = (a0, b1);
        Self::new(p, cps, ws, knots)
    }

    /// Elevate the degree by one (Bezier decomposition, elevation, reassembly).
    /// The result has full-multiplicity interior knots, like a polycurve.
    pub fn elevate_degree_once(&self) -> Result<Self> {
        let p = self.degree;
        // decompose into Bezier segments by raising every interior knot to multiplicity p
        let mut c = self.clone();
        let mut interior: Vec<f64> = c.knots[p + 1..c.knots.len() - p - 1].to_vec();
        interior.dedup_by(|a, b| (*a - *b).abs() < crate::KNOT_TOL);
        for k in interior.iter() {
            c.insert_knot(*k, p);
        }
        let hom: Vec<DVec4> = (0..c.control_points.len()).map(|i| c.homogeneous(i)).collect();
        let n_seg = (hom.len() - 1) / p;
        let mut out: Vec<DVec4> = Vec::new();
        for s in 0..n_seg {
            let seg = &hom[s * p..s * p + p + 1];
            let q = p + 1;
            for i in 0..=q {
                let a = i as f64 / q as f64;
                let v = if i == 0 {
                    seg[0]
                } else if i == q {
                    seg[p]
                } else {
                    seg[i - 1] * a + seg[i] * (1.0 - a)
                };
                if s > 0 && i == 0 {
                    continue; // shared with previous segment
                }
                out.push(v);
            }
        }
        // knots: distinct values with multiplicity p+2 at the ends and p+1 inside
        let mut distinct: Vec<f64> = c.knots.clone();
        distinct.dedup_by(|a, b| (*a - *b).abs() < crate::KNOT_TOL);
        let mut knots = Vec::new();
        for (i, k) in distinct.iter().enumerate() {
            let m = if i == 0 || i == distinct.len() - 1 { p + 2 } else { p + 1 };
            knots.extend(std::iter::repeat_n(*k, m));
        }
        let cps: Vec<DVec3> = out.iter().map(|h| h.truncate() / h.w).collect();
        let ws: Vec<f64> = out.iter().map(|h| h.w).collect();
        Self::new(p + 1, cps, ws, knots)
    }

    /// Elevate (never lower) to the requested degree.
    pub fn to_degree(&self, degree: usize) -> Result<Self> {
        let mut c = self.clone();
        while c.degree < degree {
            c = c.elevate_degree_once()?;
        }
        Ok(c)
    }

    /// Join curves of different degrees by elevating the lower one first.
    pub fn join_any(&self, other: &Self) -> Result<Self> {
        let d = self.degree.max(other.degree);
        self.to_degree(d)?.join(&other.to_degree(d)?)
    }

    pub fn domain(&self) -> (f64, f64) {
        (self.knots[self.degree], self.knots[self.knots.len() - 1 - self.degree])
    }

    pub fn is_rational(&self) -> bool {
        self.weights.iter().any(|w| (w - 1.0).abs() > 1e-12)
    }

    pub fn is_closed(&self) -> bool {
        let (a, b) = self.domain();
        (self.point_at(a) - self.point_at(b)).length() < 1e-6
    }

    pub fn start_point(&self) -> DVec3 {
        self.point_at(self.domain().0)
    }

    pub fn end_point(&self) -> DVec3 {
        self.point_at(self.domain().1)
    }

    fn homogeneous(&self, i: usize) -> DVec4 {
        let w = self.weights[i];
        let p = self.control_points[i];
        DVec4::new(p.x * w, p.y * w, p.z * w, w)
    }

    pub fn point_at(&self, t: f64) -> DVec3 {
        let (t0, t1) = self.domain();
        let t = t.clamp(t0, t1);
        let n = self.control_points.len() - 1;
        let span = find_span(n, self.degree, t, &self.knots);
        let b = basis_funs(span, t, self.degree, &self.knots);
        let mut acc = DVec4::ZERO;
        for (j, bj) in b.iter().enumerate() {
            acc += self.homogeneous(span - self.degree + j) * *bj;
        }
        acc.truncate() / acc.w
    }

    /// First derivative (velocity) of the curve.
    pub fn derivative_at(&self, t: f64) -> DVec3 {
        let (t0, t1) = self.domain();
        let t = t.clamp(t0, t1);
        let n = self.control_points.len() - 1;
        let span = find_span(n, self.degree, t, &self.knots);
        let d = ders_basis_funs(span, t, self.degree, 1, &self.knots);
        let mut a0 = DVec4::ZERO;
        let mut a1 = DVec4::ZERO;
        for j in 0..=self.degree {
            let h = self.homogeneous(span - self.degree + j);
            a0 += h * d[0][j];
            a1 += h * d[1][j];
        }
        // quotient rule: C' = (A' - w' C) / w
        let c = a0.truncate() / a0.w;
        (a1.truncate() - c * a1.w) / a0.w
    }

    pub fn tangent_at(&self, t: f64) -> DVec3 {
        self.derivative_at(t).normalize_or(DVec3::X)
    }

    /// Sample the curve into a polyline suitable for display.
    pub fn tessellate(&self, segments_per_span: usize) -> Vec<DVec3> {
        if self.degree == 1 {
            return self.control_points.clone();
        }
        let mut pts = Vec::new();
        let spans: Vec<f64> = {
            let mut v: Vec<f64> = self.knots[self.degree..self.knots.len() - self.degree].to_vec();
            v.dedup_by(|a, b| (*a - *b).abs() < crate::KNOT_TOL);
            v
        };
        for w in spans.windows(2) {
            for i in 0..segments_per_span {
                let t = w[0] + (w[1] - w[0]) * i as f64 / segments_per_span as f64;
                pts.push(self.point_at(t));
            }
        }
        pts.push(self.end_point());
        pts
    }

    /// Adaptive tessellation: subdivide until the chord deviation is below `tol`.
    /// Points at `segs` equal parameter steps in every knot span (the sampling used by
    /// surface tessellation, so caps built from a curve match the extruded sides exactly).
    pub fn span_samples(&self, segs: usize) -> Vec<DVec3> {
        let mut spans: Vec<f64> = self.knots[self.degree..self.knots.len() - self.degree].to_vec();
        spans.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        let mut out = Vec::new();
        for w in spans.windows(2) {
            for i in 0..segs.max(1) {
                out.push(self.point_at(w[0] + (w[1] - w[0]) * i as f64 / segs.max(1) as f64));
            }
        }
        if let Some(last) = spans.last() {
            out.push(self.point_at(*last));
        }
        out
    }

    pub fn tessellate_adaptive(&self, tol: f64) -> Vec<DVec3> {
        if self.degree == 1 {
            return self.control_points.clone();
        }
        let (t0, t1) = self.domain();
        let base = self.tessellate(4);
        let n_base = base.len() - 1;
        let mut out = vec![self.point_at(t0)];
        for i in 0..n_base {
            let a = t0 + (t1 - t0) * i as f64 / n_base as f64;
            let b = t0 + (t1 - t0) * (i + 1) as f64 / n_base as f64;
            self.subdivide(a, b, tol, 0, &mut out);
        }
        out
    }

    fn subdivide(&self, a: f64, b: f64, tol: f64, depth: usize, out: &mut Vec<DVec3>) {
        let pa = self.point_at(a);
        let pb = self.point_at(b);
        let m = (a + b) * 0.5;
        let pm = self.point_at(m);
        let chord_mid = (pa + pb) * 0.5;
        if depth < 10 && (pm - chord_mid).length() > tol {
            self.subdivide(a, m, tol, depth + 1, out);
            self.subdivide(m, b, tol, depth + 1, out);
        } else {
            out.push(pb);
        }
    }

    pub fn length(&self) -> f64 {
        self.tessellate(16).windows(2).map(|w| (w[1] - w[0]).length()).sum()
    }

    pub fn bounding_box(&self) -> BoundingBox {
        BoundingBox::from_points(&self.control_points)
    }

    pub fn transform(&mut self, xf: &DAffine3) {
        for p in self.control_points.iter_mut() {
            *p = xf.transform_point3(*p);
        }
    }

    pub fn transformed(&self, xf: &DAffine3) -> Self {
        let mut c = self.clone();
        c.transform(xf);
        c
    }

    pub fn reverse(&mut self) {
        self.control_points.reverse();
        self.weights.reverse();
        let (a, b) = (self.knots[0], *self.knots.last().unwrap());
        let mut k: Vec<f64> = self.knots.iter().map(|t| a + b - t).collect();
        k.reverse();
        self.knots = k;
    }

    /// Closest parameter on the curve to a point (sampling + Newton refinement).
    pub fn closest_parameter(&self, p: DVec3) -> f64 {
        let (t0, t1) = self.domain();
        let n = 64 * self.control_points.len();
        let mut best_t = t0;
        let mut best_d = f64::INFINITY;
        for i in 0..=n {
            let t = t0 + (t1 - t0) * i as f64 / n as f64;
            let d = (self.point_at(t) - p).length_squared();
            if d < best_d {
                best_d = d;
                best_t = t;
            }
        }
        // Newton iterations on f(t) = (C(t)-p)·C'(t)
        let mut t = best_t;
        for _ in 0..8 {
            let c = self.point_at(t);
            let d1 = self.derivative_at(t);
            let h = 1e-6 * (t1 - t0);
            let d2 = (self.derivative_at((t + h).min(t1)) - self.derivative_at((t - h).max(t0))) / (2.0 * h);
            let f = (c - p).dot(d1);
            let fp = d1.dot(d1) + (c - p).dot(d2);
            if fp.abs() < 1e-14 {
                break;
            }
            let nt = (t - f / fp).clamp(t0, t1);
            if (nt - t).abs() < 1e-12 {
                break;
            }
            t = nt;
        }
        t
    }
}

/// Knot vector by averaging parameters (NURBS Book eq. 9.8).
pub fn averaged_knots(params: &[f64], degree: usize) -> Vec<f64> {
    let n = params.len();
    let m = n + degree + 1;
    let mut knots = vec![0.0; m];
    for k in knots.iter_mut().skip(n).take(degree + 1) {
        *k = 1.0;
    }
    for j in 1..n - degree {
        let s: f64 = params[j..j + degree].iter().sum();
        knots[j + degree] = s / degree as f64;
    }
    knots
}

pub(crate) fn intersect_lines(p0: DVec3, d0: DVec3, p1: DVec3, d1: DVec3) -> Option<DVec3> {
    // closest points between two lines; return midpoint
    let w = p0 - p1;
    let a = d0.dot(d0);
    let b = d0.dot(d1);
    let c = d1.dot(d1);
    let d = d0.dot(w);
    let e = d1.dot(w);
    let den = a * c - b * b;
    if den.abs() < 1e-14 {
        return None;
    }
    let s = (b * e - c * d) / den;
    let t = (a * e - b * d) / den;
    Some((p0 + d0 * s + p1 + d1 * t) * 0.5)
}

/// Center, radius and plane of the circle through three points.
pub fn circle_through(a: DVec3, b: DVec3, c: DVec3) -> Result<(DVec3, f64, Plane)> {
    let ab = b - a;
    let ac = c - a;
    let n = ab.cross(ac);
    if n.length_squared() < TOL * TOL {
        return Err(GeomError::Degenerate("three points are collinear".into()));
    }
    let n2 = n.length_squared();
    let center = a + (ac * ab.length_squared() - ab * ac.length_squared()).cross(n) / (2.0 * n2);
    let radius = (center - a).length();
    let plane = Plane::from_frame(center, a - center, n.cross(a - center));
    Ok((center, radius, plane))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_is_exact() {
        let c = NurbsCurve::circle(&Plane::world_xy(), 2.0).unwrap();
        for i in 0..200 {
            let t = i as f64 / 200.0;
            let p = c.point_at(t);
            assert!((p.length() - 2.0).abs() < 1e-12, "r = {}", p.length());
        }
        assert!(c.is_closed());
        assert!((c.length() - std::f64::consts::TAU * 2.0).abs() < 1e-2);
    }

    #[test]
    fn arc_is_exact() {
        let c = NurbsCurve::arc(&Plane::world_xy(), 1.0, 0.3, 4.0).unwrap();
        assert!((c.start_point() - DVec3::new(0.3f64.cos(), 0.3f64.sin(), 0.0)).length() < 1e-12);
        assert!((c.end_point() - DVec3::new(4.0f64.cos(), 4.0f64.sin(), 0.0)).length() < 1e-12);
        for i in 0..50 {
            let t = i as f64 / 50.0;
            assert!((c.point_at(t).length() - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn interpolation_passes_through_points() {
        let pts = vec![
            DVec3::new(0.0, 0.0, 0.0),
            DVec3::new(1.0, 2.0, 0.0),
            DVec3::new(3.0, 1.0, 1.0),
            DVec3::new(4.0, 4.0, 0.0),
            DVec3::new(6.0, 2.0, 2.0),
        ];
        let c = NurbsCurve::interpolate(&pts, 3).unwrap();
        for p in &pts {
            let t = c.closest_parameter(*p);
            assert!((c.point_at(t) - *p).length() < 1e-8);
        }
    }

    #[test]
    fn derivative_matches_finite_difference() {
        let c = NurbsCurve::circle(&Plane::world_xy(), 1.0).unwrap();
        let t = 0.31;
        let h = 1e-6;
        let fd = (c.point_at(t + h) - c.point_at(t - h)) / (2.0 * h);
        assert!((fd - c.derivative_at(t)).length() < 1e-5);
    }

    #[test]
    fn split_and_join_preserve_shape() {
        let c = NurbsCurve::circle(&Plane::world_xy(), 1.0).unwrap();
        let (l, r) = c.split(0.4).unwrap();
        assert!((l.end_point() - c.point_at(0.4)).length() < 1e-12);
        assert!((r.start_point() - c.point_at(0.4)).length() < 1e-12);
        for i in 0..=10 {
            let t = 0.4 * i as f64 / 10.0;
            assert!((l.point_at(t) - c.point_at(t)).length() < 1e-10);
        }
        let j = l.join(&r).unwrap();
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            assert!((j.point_at(t) - c.point_at(t)).length() < 1e-9, "t={t}");
        }
        let a = NurbsCurve::line(DVec3::ZERO, DVec3::X).unwrap();
        let b = NurbsCurve::line(DVec3::X, DVec3::new(1.0, 1.0, 0.0)).unwrap();
        let ab = a.join(&b).unwrap();
        assert_eq!(ab.control_points.len(), 3);
        assert!((ab.point_at(1.0) - DVec3::X).length() < 1e-12);
    }

    #[test]
    fn degree_elevation_preserves_shape() {
        let c = NurbsCurve::circle(&Plane::world_xy(), 1.5).unwrap();
        let e = c.elevate_degree_once().unwrap();
        assert_eq!(e.degree, 3);
        for i in 0..=40 {
            let t = i as f64 / 40.0;
            assert!((e.point_at(t) - c.point_at(t)).length() < 1e-9, "t={t}");
        }
        let l = NurbsCurve::line(DVec3::ZERO, DVec3::X).unwrap();
        let a = NurbsCurve::arc(&Plane { origin: DVec3::new(1.0, 1.0, 0.0), ..Plane::world_xy() }, 1.0, -std::f64::consts::FRAC_PI_2, 0.0).unwrap();
        let j = l.join_any(&a).unwrap();
        assert_eq!(j.degree, 2);
        assert!((j.start_point() - DVec3::ZERO).length() < 1e-9);
        assert!((j.end_point() - DVec3::new(2.0, 1.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn periodic_curve_is_closed_and_smooth() {
        let pts = vec![DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
        let c = NurbsCurve::periodic(3, pts).unwrap();
        assert!(c.is_closed());
        let (t0, t1) = c.domain();
        let d0 = c.tangent_at(t0);
        let d1 = c.tangent_at(t1);
        assert!((d0 - d1).length() < 1e-9);
    }

    #[test]
    fn ellipse_is_exact() {
        let e = NurbsCurve::ellipse(&Plane::world_xy(), 3.0, 1.0).unwrap();
        for i in 0..50 {
            let p = e.point_at(i as f64 / 50.0);
            assert!(((p.x / 3.0).powi(2) + p.y.powi(2) - 1.0).abs() < 1e-10);
        }
    }

    #[test]
    fn arc_3pt_works() {
        let a = DVec3::new(1.0, 0.0, 0.0);
        let b = DVec3::new(0.0, 1.0, 0.0);
        let c = DVec3::new(-1.0, 0.0, 0.0);
        let arc = NurbsCurve::arc_3pt(a, b, c).unwrap();
        assert!((arc.start_point() - a).length() < 1e-9);
        assert!((arc.end_point() - c).length() < 1e-9);
        assert!((arc.point_at(0.5) - b).length() < 1e-9);
    }
}
