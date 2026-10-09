//! Curve editing operations: intersections, extension, fillet, chamfer, offset,
//! rebuild and division.

use glam::DVec3;

use crate::{GeomError, NurbsCurve, Plane, Result, TOL};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveIntersection {
    pub ta: f64,
    pub tb: f64,
    pub point: DVec3,
}

/// Closest points of two segments; returns (s, t, distance).
fn segment_closest(a0: DVec3, a1: DVec3, b0: DVec3, b1: DVec3) -> (f64, f64, f64) {
    let u = a1 - a0;
    let v = b1 - b0;
    let w = a0 - b0;
    let a = u.dot(u);
    let b = u.dot(v);
    let c = v.dot(v);
    let d = u.dot(w);
    let e = v.dot(w);
    let den = a * c - b * b;
    let (mut s, mut t);
    if den.abs() < 1e-14 {
        s = 0.0;
        t = if c > 1e-18 { (e / c).clamp(0.0, 1.0) } else { 0.0 };
    } else {
        s = ((b * e - c * d) / den).clamp(0.0, 1.0);
        t = ((a * e - b * d) / den).clamp(0.0, 1.0);
        // re-clamp partner
        let pa = a0 + u * s;
        t = if c > 1e-18 { ((pa - b0).dot(v) / c).clamp(0.0, 1.0) } else { 0.0 };
        let pb = b0 + v * t;
        s = if a > 1e-18 { ((pb - a0).dot(u) / a).clamp(0.0, 1.0) } else { 0.0 };
    }
    let dist = (a0 + u * s - (b0 + v * t)).length();
    (s, t, dist)
}

/// Parameter of a tessellated point: `pts` sampled at `params`, interpolate linearly.
fn param_at(params: &[f64], i: usize, frac: f64) -> f64 {
    params[i] + (params[i + 1] - params[i]) * frac
}

fn adaptive_with_params(c: &NurbsCurve, tol: f64) -> (Vec<DVec3>, Vec<f64>) {
    // sample by parameter with enough density: 24 per span plus adaptive refinement
    let (t0, t1) = c.domain();
    let n = (c.control_points.len() * 24).max(32);
    let mut params: Vec<f64> = (0..=n).map(|i| t0 + (t1 - t0) * i as f64 / n as f64).collect();
    let mut pts: Vec<DVec3> = params.iter().map(|t| c.point_at(*t)).collect();
    // refine where chord deviation is too large
    for _ in 0..4 {
        let mut new_params = Vec::with_capacity(params.len() * 2);
        let mut refined = false;
        for i in 0..params.len() - 1 {
            new_params.push(params[i]);
            let m = (params[i] + params[i + 1]) * 0.5;
            let pm = c.point_at(m);
            if (pm - (pts[i] + pts[i + 1]) * 0.5).length() > tol {
                new_params.push(m);
                refined = true;
            }
        }
        new_params.push(*params.last().unwrap());
        if !refined {
            break;
        }
        params = new_params;
        pts = params.iter().map(|t| c.point_at(*t)).collect();
    }
    (pts, params)
}

/// Intersections (and near-touches within `tol`) between two curves.
pub fn curve_curve_intersections(a: &NurbsCurve, b: &NurbsCurve, tol: f64) -> Vec<CurveIntersection> {
    let same = std::ptr::eq(a, b);
    let (pa, ta) = adaptive_with_params(a, tol * 0.5);
    let (pb, tb) = adaptive_with_params(b, tol * 0.5);
    let mut out: Vec<CurveIntersection> = Vec::new();
    for i in 0..pa.len() - 1 {
        for j in 0..pb.len() - 1 {
            if same && (i as isize - j as isize).abs() <= 1 {
                continue;
            }
            let (s, t, d) = segment_closest(pa[i], pa[i + 1], pb[j], pb[j + 1]);
            if d > tol * 4.0 {
                continue;
            }
            let mut xa = param_at(&ta, i, s);
            let mut xb = param_at(&tb, j, t);
            // Gauss-Newton refinement of |A(xa) - B(xb)|^2
            for _ in 0..20 {
                let p = a.point_at(xa);
                let q = b.point_at(xb);
                let da = a.derivative_at(xa);
                let db = b.derivative_at(xb);
                let r = p - q;
                let m00 = da.dot(da);
                let m01 = -da.dot(db);
                let m11 = db.dot(db);
                let g0 = r.dot(da);
                let g1 = -r.dot(db);
                let det = m00 * m11 - m01 * m01;
                if det.abs() < 1e-18 {
                    break;
                }
                let dxa = (m11 * g0 - m01 * g1) / det;
                let dxb = (m00 * g1 - m01 * g0) / det;
                let (a0, a1) = a.domain();
                let (b0, b1) = b.domain();
                xa = (xa - dxa).clamp(a0, a1);
                xb = (xb - dxb).clamp(b0, b1);
                if dxa.abs() < 1e-13 && dxb.abs() < 1e-13 {
                    break;
                }
            }
            let p = a.point_at(xa);
            let q = b.point_at(xb);
            if (p - q).length() > tol {
                continue;
            }
            let point = (p + q) * 0.5;
            if out.iter().any(|x| (x.point - point).length() < tol * 2.0) {
                continue;
            }
            out.push(CurveIntersection { ta: xa, tb: xb, point });
        }
    }
    out.sort_by(|x, y| x.ta.partial_cmp(&y.ta).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// A straight line as a curve of the given degree (collinear control points).
pub fn line_of_degree(a: DVec3, b: DVec3, degree: usize) -> Result<NurbsCurve> {
    let n = degree + 1;
    let pts: Vec<DVec3> = (0..n).map(|i| a + (b - a) * (i as f64 / degree as f64)).collect();
    NurbsCurve::from_control_points(degree, pts)
}

impl NurbsCurve {
    /// Split the curve at several parameters, returning the pieces in order.
    pub fn split_at_many(&self, params: &[f64]) -> Vec<NurbsCurve> {
        let (t0, t1) = self.domain();
        let mut ps: Vec<f64> = params.iter().copied().filter(|t| *t > t0 + 1e-9 && *t < t1 - 1e-9).collect();
        ps.sort_by(|a, b| a.partial_cmp(b).unwrap());
        ps.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let mut pieces = Vec::new();
        let mut rest = self.clone();
        for t in ps {
            match rest.split(t) {
                Some((l, r)) => {
                    pieces.push(l);
                    rest = r;
                }
                None => {}
            }
        }
        pieces.push(rest);
        pieces
    }

    /// Extend one end along its tangent so that it reaches `target` (projected onto the tangent).
    pub fn extend_to(&self, at_end: bool, target: DVec3) -> Result<NurbsCurve> {
        let (t0, t1) = self.domain();
        let (p, tangent) = if at_end { (self.end_point(), self.tangent_at(t1)) } else { (self.start_point(), -self.tangent_at(t0)) };
        let len = (target - p).dot(tangent);
        self.extend_by(at_end, len)
    }

    /// Extend one end by a straight segment of the given length (negative shortens by splitting).
    pub fn extend_by(&self, at_end: bool, len: f64) -> Result<NurbsCurve> {
        if len.abs() < TOL {
            return Ok(self.clone());
        }
        let (t0, t1) = self.domain();
        if len < 0.0 {
            // shorten: walk back along the curve by |len|
            let pts = self.tessellate(16);
            let target = len.abs();
            let mut acc = 0.0;
            let n = pts.len();
            let mut cut = None;
            if at_end {
                for i in (1..n).rev() {
                    let d = (pts[i] - pts[i - 1]).length();
                    if acc + d >= target {
                        cut = Some(pts[i] + (pts[i - 1] - pts[i]) * ((target - acc) / d));
                        break;
                    }
                    acc += d;
                }
            } else {
                for i in 0..n - 1 {
                    let d = (pts[i + 1] - pts[i]).length();
                    if acc + d >= target {
                        cut = Some(pts[i] + (pts[i + 1] - pts[i]) * ((target - acc) / d));
                        break;
                    }
                    acc += d;
                }
            }
            let Some(cp) = cut else { return Err(GeomError::Degenerate("curve shorter than the trim length".into())) };
            let t = self.closest_parameter(cp);
            return match self.split(t) {
                Some((l, r)) => Ok(if at_end { l } else { r }),
                None => Ok(self.clone()),
            };
        }
        let (p, tangent) = if at_end { (self.end_point(), self.tangent_at(t1)) } else { (self.start_point(), -self.tangent_at(t0)) };
        let q = p + tangent * len;
        if self.degree == 1 {
            let mut c = self.clone();
            if at_end {
                *c.control_points.last_mut().unwrap() = q;
                let kn = c.knots.len();
                let add = len;
                c.knots[kn - 1] += add;
                c.knots[kn - 2] += add;
            } else {
                c.control_points[0] = q;
                c.knots[0] -= len;
                c.knots[1] -= len;
            }
            return Ok(c);
        }
        let seg = line_of_degree(p, q, self.degree)?;
        if at_end {
            self.join(&seg)
        } else {
            let mut s = seg;
            s.reverse();
            s.join(self)
        }
    }

    /// Points dividing the curve into `n` segments of equal length (including both ends).
    pub fn divide_by_count(&self, n: usize) -> Vec<DVec3> {
        let n = n.max(1);
        let pts = self.tessellate(32);
        let total: f64 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        let mut out = Vec::with_capacity(n + 1);
        for k in 0..=n {
            let target = total * k as f64 / n as f64;
            out.push(point_at_length(&pts, target));
        }
        out
    }

    /// Points along the curve every `len` units from the start.
    pub fn divide_by_length(&self, len: f64) -> Vec<DVec3> {
        let pts = self.tessellate(32);
        let total: f64 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
        let mut out = Vec::new();
        let mut d = 0.0;
        while d <= total + 1e-9 {
            out.push(point_at_length(&pts, d));
            d += len.max(1e-9);
        }
        out
    }

    /// Least-squares rebuild with `count` control points of the given degree, keeping the end points.
    pub fn rebuild(&self, count: usize, degree: usize) -> Result<NurbsCurve> {
        let degree = degree.max(1);
        let count = count.max(degree + 1);
        let m = (count * 6).max(64);
        let (t0, t1) = self.domain();
        // chord-length parameters of samples
        let samples: Vec<DVec3> = (0..=m).map(|i| self.point_at(t0 + (t1 - t0) * i as f64 / m as f64)).collect();
        let mut u = vec![0.0; m + 1];
        for i in 1..=m {
            u[i] = u[i - 1] + (samples[i] - samples[i - 1]).length();
        }
        let total = u[m].max(1e-12);
        for x in u.iter_mut() {
            *x /= total;
        }
        let knots = crate::basis::uniform_clamped_knots(count, degree);
        let n = count - 1;
        // unknowns: interior control points 1..n-1; ends fixed
        let inner = n - 1;
        if inner == 0 {
            return NurbsCurve::new(degree, vec![samples[0], samples[m]], vec![1.0; 2], knots);
        }
        let mut ata = vec![vec![0.0; inner]; inner];
        let mut atb = vec![DVec3::ZERO; inner];
        for (k, &uk) in u.iter().enumerate().skip(1).take(m - 1) {
            let span = crate::basis::find_span(n, degree, uk, &knots);
            let b = crate::basis::basis_funs(span, uk, degree, &knots);
            let mut row = vec![0.0; count];
            for (j, bj) in b.iter().enumerate() {
                row[span - degree + j] = *bj;
            }
            let rk = samples[k] - samples[0] * row[0] - samples[m] * row[n];
            for i in 0..inner {
                if row[i + 1] == 0.0 {
                    continue;
                }
                atb[i] += rk * row[i + 1];
                for j in 0..inner {
                    ata[i][j] += row[i + 1] * row[j + 1];
                }
            }
        }
        let sol = crate::solve_dense(ata, atb)?;
        let mut cps = Vec::with_capacity(count);
        cps.push(samples[0]);
        cps.extend(sol);
        cps.push(samples[m]);
        NurbsCurve::new(degree, cps, vec![1.0; count], knots)
    }

    /// Offset a planar curve in `plane` by `distance` (positive = to the left of the curve direction).
    pub fn offset(&self, plane: &Plane, distance: f64, tol: f64) -> Result<NurbsCurve> {
        if distance.abs() < TOL {
            return Ok(self.clone());
        }
        // exact cases: circles / arcs keep their centre
        if let Some(center) = arc_center(self) {
            let r = (self.start_point() - center).length();
            let new_r = r + distance * arc_orientation(self, plane, center);
            if new_r <= TOL {
                return Err(GeomError::Degenerate("offset radius is not positive".into()));
            }
            let f = new_r / r;
            let mut c = self.clone();
            for p in c.control_points.iter_mut() {
                *p = center + (*p - center) * f;
            }
            return Ok(c);
        }
        if self.degree == 1 {
            return offset_polyline(self, plane, distance);
        }
        let (pts, params) = adaptive_with_params(self, tol);
        let mut off = Vec::with_capacity(pts.len());
        for (i, &t) in params.iter().enumerate() {
            let tan = self.tangent_at(t);
            let n = plane.z_axis.cross(tan).normalize_or(plane.y_axis);
            off.push(pts[i] + n * distance);
        }
        // refit with the sample parameters
        let n = off.len();
        let u: Vec<f64> = params.iter().map(|t| (t - params[0]) / (params[n - 1] - params[0])).collect();
        let mut c = NurbsCurve::interpolate_params(&off, 3, &u)?;
        if self.is_closed() {
            c.control_points[n - 1] = c.control_points[0];
        }
        Ok(c)
    }
}

fn point_at_length(pts: &[DVec3], target: f64) -> DVec3 {
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let d = (w[1] - w[0]).length();
        if acc + d >= target {
            let f = if d > 0.0 { (target - acc) / d } else { 0.0 };
            return w[0] + (w[1] - w[0]) * f;
        }
        acc += d;
    }
    *pts.last().unwrap()
}

/// Centre of an arc or circle curve (rational degree 2 with constant radius).
pub fn arc_center(c: &NurbsCurve) -> Option<DVec3> {
    if c.degree != 2 || !c.is_rational() {
        return None;
    }
    let (t0, t1) = c.domain();
    let a = c.point_at(t0);
    let m = c.point_at((t0 + t1) * 0.5);
    let b = c.point_at(t1);
    let (center, r, _) = if (a - b).length() < 1e-9 {
        crate::curve::circle_through(a, c.point_at(t0 + (t1 - t0) * 0.25), m).ok()?
    } else {
        crate::curve::circle_through(a, m, b).ok()?
    };
    for i in 0..8 {
        let t = t0 + (t1 - t0) * (i as f64 + 0.37) / 8.0;
        if ((c.point_at(t) - center).length() - r).abs() > 1e-6 * r.max(1.0) {
            return None;
        }
    }
    Some(center)
}

/// +1 when the arc turns counter-clockwise around its centre in `plane` (so "left" is inward).
fn arc_orientation(c: &NurbsCurve, plane: &Plane, center: DVec3) -> f64 {
    let (t0, _) = c.domain();
    let p = c.point_at(t0) - center;
    let d = c.derivative_at(t0);
    if plane.z_axis.dot(p.cross(d)) > 0.0 {
        -1.0
    } else {
        1.0
    }
}

fn offset_polyline(c: &NurbsCurve, plane: &Plane, distance: f64) -> Result<NurbsCurve> {
    let pts = &c.control_points;
    let closed = c.is_closed();
    let n = if closed { pts.len() - 1 } else { pts.len() };
    let seg_dir = |i: usize| (pts[(i + 1) % pts.len().max(1)] - pts[i]).normalize_or(plane.x_axis);
    let seg_normal = |i: usize| plane.z_axis.cross(seg_dir(i)).normalize_or(plane.y_axis);
    let mut out = Vec::with_capacity(pts.len());
    let segs = if closed { n } else { n - 1 };
    for i in 0..n {
        let prev = if i == 0 { if closed { Some(segs - 1) } else { None } } else { Some(i - 1) };
        let next = if i < segs { Some(i) } else if closed { Some(0) } else { None };
        let p = match (prev, next) {
            (None, Some(j)) => pts[i] + seg_normal(j) * distance,
            (Some(j), None) => pts[i] + seg_normal(j) * distance,
            (Some(j), Some(k)) => {
                let a = pts[i] + seg_normal(j) * distance;
                let b = pts[i] + seg_normal(k) * distance;
                // intersect the two offset lines (sharp corner)
                let d1 = seg_dir(j);
                let d2 = seg_dir(k);
                let cross = d1.cross(d2);
                if cross.length_squared() < 1e-18 {
                    a
                } else {
                    let w = b - a;
                    let s = w.cross(d2).dot(cross) / cross.length_squared();
                    a + d1 * s
                }
            }
            (None, None) => pts[i],
        };
        out.push(p);
    }
    if closed {
        out.push(out[0]);
    }
    NurbsCurve::polyline(out)
}

/// Fillet between two curves: returns (trimmed a, trimmed b, arc). `pa`/`pb` are the pick
/// points that identify which parts of the curves to keep.
pub fn fillet_curves(a: &NurbsCurve, b: &NurbsCurve, radius: f64, pa: DVec3, pb: DVec3, plane: &Plane, tol: f64) -> Result<(NurbsCurve, NurbsCurve, NurbsCurve)> {
    if radius <= TOL {
        return Err(GeomError::Degenerate("fillet radius must be positive".into()));
    }
    // try the four offset combinations; keep the centre closest to both picks
    let mut best: Option<(f64, DVec3, f64, f64)> = None;
    for sa in [1.0, -1.0] {
        for sb in [1.0, -1.0] {
            let (Ok(oa), Ok(ob)) = (a.offset(plane, radius * sa, tol), b.offset(plane, radius * sb, tol)) else { continue };
            for x in curve_curve_intersections(&oa, &ob, tol * 10.0) {
                let center = x.point;
                let ta = a.closest_parameter(center);
                let tb = b.closest_parameter(center);
                let fa = a.point_at(ta);
                let fb = b.point_at(tb);
                if ((fa - center).length() - radius).abs() > tol * 100.0 || ((fb - center).length() - radius).abs() > tol * 100.0 {
                    continue;
                }
                // Triceratops keeps the sides of the curves that were picked
                let score = (center - pa).length() + (center - pb).length();
                if best.as_ref().map_or(true, |b| score < b.0) {
                    best = Some((score, center, ta, tb));
                }
            }
        }
    }
    let Some((_, center, ta, tb)) = best else {
        return Err(GeomError::Degenerate("the curves cannot be filleted with this radius".into()));
    };
    let fa = a.point_at(ta);
    let fb = b.point_at(tb);
    let keep_a = keep_side(a, ta, pa)?;
    let keep_b = keep_side(b, tb, pb)?;
    // arc through fa, mid, fb around the centre
    let mid_dir = ((fa - center) + (fb - center)).normalize_or((fa - center).normalize());
    let mid = center + mid_dir * radius;
    let arc = NurbsCurve::arc_3pt(fa, mid, fb)?;
    Ok((keep_a, keep_b, arc))
}

/// Keep the part of `c` on the side of parameter `t` where `pick` lies.
fn keep_side(c: &NurbsCurve, t: f64, pick: DVec3) -> Result<NurbsCurve> {
    let tp = c.closest_parameter(pick);
    match c.split(t) {
        Some((l, r)) => Ok(if tp < t { l } else { r }),
        None => Ok(c.clone()),
    }
}

/// Connect: extend or trim two curves so that they meet at their intersection.
pub fn connect_curves(a: &NurbsCurve, b: &NurbsCurve, pa: DVec3, pb: DVec3, tol: f64) -> Result<(NurbsCurve, NurbsCurve)> {
    let ea = a.extend_by(true, a.length() * 2.0).unwrap_or(a.clone()).extend_by(false, a.length() * 2.0).unwrap_or(a.clone());
    let eb = b.extend_by(true, b.length() * 2.0).unwrap_or(b.clone()).extend_by(false, b.length() * 2.0).unwrap_or(b.clone());
    let xs = curve_curve_intersections(&ea, &eb, tol * 10.0);
    let Some(x) = xs.iter().min_by(|p, q| ((p.point - pa).length() + (p.point - pb).length()).partial_cmp(&((q.point - pa).length() + (q.point - pb).length())).unwrap()) else {
        return Err(GeomError::Degenerate("the curves do not intersect".into()));
    };
    let ka = keep_side(&ea, x.ta, pa)?;
    let kb = keep_side(&eb, x.tb, pb)?;
    Ok((ka, kb))
}

/// Chamfer between two curves (straight connection at distances d1 and d2 from their intersection).
pub fn chamfer_curves(a: &NurbsCurve, b: &NurbsCurve, d1: f64, d2: f64, pa: DVec3, pb: DVec3, tol: f64) -> Result<(NurbsCurve, NurbsCurve, NurbsCurve)> {
    // extend both curves so that they intersect
    let ea = a.extend_by(true, a.length() * 2.0).unwrap_or(a.clone()).extend_by(false, a.length() * 2.0).unwrap_or(a.clone());
    let eb = b.extend_by(true, b.length() * 2.0).unwrap_or(b.clone()).extend_by(false, b.length() * 2.0).unwrap_or(b.clone());
    let xs = curve_curve_intersections(&ea, &eb, tol * 10.0);
    let Some(x) = xs.iter().min_by(|p, q| ((p.point - pa).length() + (p.point - pb).length()).partial_cmp(&((q.point - pa).length() + (q.point - pb).length())).unwrap()) else {
        return Err(GeomError::Degenerate("the curves do not intersect".into()));
    };
    let corner = x.point;
    // points at distance d along each curve towards the pick
    let ta = ea.closest_parameter(corner);
    let tb = eb.closest_parameter(corner);
    let dir_a = if ea.closest_parameter(pa) > ta { 1.0 } else { -1.0 };
    let dir_b = if eb.closest_parameter(pb) > tb { 1.0 } else { -1.0 };
    let qa = corner + ea.tangent_at(ta) * (d1 * dir_a);
    let qb = corner + eb.tangent_at(tb) * (d2 * dir_b);
    let sa = ea.closest_parameter(qa);
    let sb = eb.closest_parameter(qb);
    let ka = keep_side(&ea, sa, pa)?;
    let kb = keep_side(&eb, sb, pb)?;
    let line = NurbsCurve::line(ea.point_at(sa), eb.point_at(sb))?;
    Ok((ka, kb, line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersections_of_lines_and_circle() {
        let l = NurbsCurve::line(DVec3::new(-5.0, 0.0, 0.0), DVec3::new(5.0, 0.0, 0.0)).unwrap();
        let c = NurbsCurve::circle(&Plane::world_xy(), 2.0).unwrap();
        let xs = curve_curve_intersections(&l, &c, 1e-6);
        assert_eq!(xs.len(), 2);
        assert!((xs[0].point - DVec3::new(-2.0, 0.0, 0.0)).length() < 1e-6);
        assert!((xs[1].point - DVec3::new(2.0, 0.0, 0.0)).length() < 1e-6);
        let pieces = l.split_at_many(&xs.iter().map(|x| x.ta).collect::<Vec<_>>());
        assert_eq!(pieces.len(), 3);
    }

    #[test]
    fn extend_and_shorten() {
        let l = NurbsCurve::line(DVec3::ZERO, DVec3::X).unwrap();
        let e = l.extend_by(true, 2.0).unwrap();
        assert!((e.end_point() - DVec3::new(3.0, 0.0, 0.0)).length() < 1e-9);
        let s = e.extend_by(true, -1.0).unwrap();
        assert!((s.end_point() - DVec3::new(2.0, 0.0, 0.0)).length() < 1e-6);
        let c = NurbsCurve::arc(&Plane::world_xy(), 1.0, 0.0, 1.0).unwrap();
        let ce = c.extend_to(true, DVec3::new(-2.0, 3.0, 0.0)).unwrap();
        assert!(ce.length() > c.length());
    }

    #[test]
    fn fillet_two_lines() {
        let a = NurbsCurve::line(DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 0.0, 0.0)).unwrap();
        let b = NurbsCurve::line(DVec3::new(10.0, 0.0, 0.0), DVec3::new(10.0, 10.0, 0.0)).unwrap();
        let (ta, tb, arc) = fillet_curves(&a, &b, 2.0, DVec3::new(2.0, 0.0, 0.0), DVec3::new(10.0, 8.0, 0.0), &Plane::world_xy(), 1e-4).unwrap();
        assert!((ta.end_point() - DVec3::new(8.0, 0.0, 0.0)).length() < 1e-3);
        assert!((tb.start_point() - DVec3::new(10.0, 2.0, 0.0)).length() < 1e-3);
        let c = arc_center(&arc).unwrap();
        assert!((c - DVec3::new(8.0, 2.0, 0.0)).length() < 1e-3);
    }

    #[test]
    fn offset_circle_and_polyline() {
        let c = NurbsCurve::circle(&Plane::world_xy(), 2.0).unwrap();
        let o = c.offset(&Plane::world_xy(), 1.0, 1e-4).unwrap();
        assert!((o.point_at(0.3).length() - 1.0).abs() < 1e-9 || (o.point_at(0.3).length() - 3.0).abs() < 1e-9);
        let p = NurbsCurve::polyline(vec![DVec3::ZERO, DVec3::new(4.0, 0.0, 0.0), DVec3::new(4.0, 4.0, 0.0)]).unwrap();
        let op = p.offset(&Plane::world_xy(), 1.0, 1e-4).unwrap();
        assert!((op.control_points[1] - DVec3::new(3.0, 1.0, 0.0)).length() < 1e-9);
    }

    #[test]
    fn rebuild_and_divide() {
        let c = NurbsCurve::circle(&Plane::world_xy(), 1.0).unwrap();
        let r = c.rebuild(12, 3).unwrap();
        assert_eq!(r.control_points.len(), 12);
        for i in 0..10 {
            let t = i as f64 / 10.0;
            assert!((r.point_at(t).length() - 1.0).abs() < 0.05);
        }
        let pts = c.divide_by_count(4);
        assert_eq!(pts.len(), 5);
        assert!((pts[1] - DVec3::new(0.0, 1.0, 0.0)).length() < 1e-3);
    }
}
