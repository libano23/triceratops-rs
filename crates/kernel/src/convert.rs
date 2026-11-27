//! glam values in solidcore's conventions.

use glam::{DAffine3, DQuat, DVec3, DVec4};

/// The matrix `Body::transform` takes: row-vector convention (`p' = p·A + t`), rows 0-2 the
/// images of x, y and z, row 3 the translation, fourth column `(0, 0, 0, 1)`.
pub fn matrix(xf: &DAffine3) -> [[f64; 4]; 4] {
    let m = xf.matrix3;
    let t = xf.translation;
    [
        [m.x_axis.x, m.x_axis.y, m.x_axis.z, 0.0],
        [m.y_axis.x, m.y_axis.y, m.y_axis.z, 0.0],
        [m.z_axis.x, m.z_axis.y, m.z_axis.z, 0.0],
        [t.x, t.y, t.z, 1.0],
    ]
}

/// The placement that takes a primitive built about +Z at the origin to `origin` with its axis
/// along `axis`. `None` for a zero or non-finite axis.
pub fn placement(origin: DVec3, axis: DVec3) -> Option<DAffine3> {
    let a = axis.try_normalize()?;
    Some(DAffine3::from_rotation_translation(DQuat::from_rotation_arc(DVec3::Z, a), origin))
}

// ---- planar curves as solidcore profiles ----

use solidcore_record::{Loop2, Plane as RecPlane, Region2, Seg2, Vec2, Vec3 as RecVec3, MAX_BEZIER_CTRL, MAX_BEZIER_DEGREE};
use tri_geom::{NurbsCurve, Plane};

use crate::solid::{refused, KernelError};

const KNOT_TOL: f64 = 1e-9;

pub fn record_plane(p: &Plane) -> RecPlane {
    let v = |d: DVec3| RecVec3::new(d.x, d.y, d.z);
    RecPlane { origin: v(p.origin), x: v(p.x_axis), y: v(p.y_axis) }
}

/// A closed planar curve as a region on `plane`: lines, cubics, higher Béziers and conics, the
/// outer loop counter-clockwise seen from the plane's normal. A circle becomes one arc, so its
/// extrusion has an analytic wall.
pub fn region2(curve: &NurbsCurve, plane: &Plane) -> Result<Region2, KernelError> {
    if !curve.is_closed() {
        return Err(refused("the curve is not closed"));
    }
    let samples = curve.tessellate(32);
    let size = curve.bounding_box().diagonal().max(1.0);
    let n = plane.z_axis;
    let off_plane = |p: &DVec3| ((*p - plane.origin).dot(n)).abs() > 1e-6 * size;
    if samples.iter().any(off_plane) || curve.control_points.iter().any(off_plane) {
        return Err(refused("the curve is not planar: it leaves the plane it is extruded from"));
    }
    let to2 = |p: DVec3| {
        let d = p - plane.origin;
        Vec2::new(d.dot(plane.x_axis), d.dot(plane.y_axis))
    };
    if let Some((c, r)) = circle_of(curve, &samples) {
        return Ok(Region2 { outer: Loop2::circle(to2(c), r), holes: Vec::new() });
    }
    let mut segs = Vec::new();
    for piece in bezier_pieces(curve)? {
        if let Some(s) = segment(&piece, &to2)? {
            segs.push(s);
        }
    }
    if segs.is_empty() {
        return Err(refused("the curve has no length"));
    }
    close_exactly(&mut segs);
    let poly: Vec<Vec2> = samples.iter().map(|p| to2(*p)).collect();
    let area: f64 = poly.windows(2).map(|w| w[0].x * w[1].y - w[1].x * w[0].y).sum();
    if area < 0.0 {
        segs.reverse();
        segs = segs.into_iter().map(reversed).collect();
    }
    Ok(Region2 { outer: Loop2 { segs }, holes: Vec::new() })
}

/// The centre and radius when `curve` is a full circle.
pub fn full_circle(curve: &NurbsCurve) -> Option<(DVec3, f64)> {
    circle_of(curve, &curve.tessellate(32))
}

fn circle_of(curve: &NurbsCurve, samples: &[DVec3]) -> Option<(DVec3, f64)> {
    if curve.degree != 2 || !curve.is_rational() {
        return None;
    }
    let pts = &samples[..samples.len().checked_sub(1)?];
    let c = pts.iter().copied().sum::<DVec3>() / pts.len() as f64;
    let r = (pts[0] - c).length();
    pts.iter().all(|p| ((*p - c).length() - r).abs() <= 1e-9 * r.max(1.0)).then_some((c, r))
}

/// One Bézier piece of a NURBS: control points and weights, `degree + 1` of each.
struct Piece {
    degree: usize,
    points: Vec<DVec3>,
    weights: Vec<f64>,
}

/// The curve as Bézier pieces, one per span of its domain. A piece's control points are the
/// blossom (polar form) of the span's B-spline, evaluated by de Boor's recursion with the
/// span's two ends as arguments, so any knot vector works (clamped or periodic) and rational
/// curves are handled in homogeneous coordinates. Each piece's ends are checked against the
/// curve.
fn bezier_pieces(c: &NurbsCurve) -> Result<Vec<Piece>, KernelError> {
    let p = c.degree;
    let n = c.control_points.len() - 1;
    let u = &c.knots;
    let bad = || refused("the curve could not be split into Bézier pieces");
    if u.len() != n + p + 2 {
        return Err(bad());
    }
    let hom: Vec<DVec4> = (0..=n)
        .map(|i| {
            let (q, w) = (c.control_points[i], c.weights[i]);
            DVec4::new(q.x * w, q.y * w, q.z * w, w)
        })
        .collect();
    let tol = 1e-7 * c.bounding_box().diagonal().max(1.0);
    let mut out = Vec::new();
    for i in p..=n {
        let (a, b) = (u[i], u[i + 1]);
        if b - a < KNOT_TOL {
            continue;
        }
        let mut points = Vec::with_capacity(p + 1);
        let mut weights = Vec::with_capacity(p + 1);
        for k in 0..=p {
            // blossom at (a repeated p-k times, b repeated k times)
            let args: Vec<f64> = (0..p).map(|r| if r < p - k { a } else { b }).collect();
            let mut d: Vec<DVec4> = hom[i - p..=i].to_vec();
            for r in 1..=p {
                let t = args[r - 1];
                let mut next = Vec::with_capacity(d.len() - 1);
                for jj in (i - p + r)..=i {
                    let (lo, hi) = (u[jj], u[jj + p - r + 1]);
                    let alpha = (t - lo) / (hi - lo);
                    next.push(d[jj - (i - p + r) ] * (1.0 - alpha) + d[jj - (i - p + r) + 1] * alpha);
                }
                d = next;
            }
            let h = d[0];
            points.push(DVec3::new(h.x, h.y, h.z) / h.w);
            weights.push(h.w);
        }
        let piece = Piece { degree: p, points, weights };
        if (piece.points[0] - c.point_at(a)).length() > tol || (piece.points[p] - c.point_at(b)).length() > tol {
            return Err(bad());
        }
        out.push(piece);
    }
    Ok(out)
}

fn segment(b: &Piece, to2: &impl Fn(DVec3) -> Vec2) -> Result<Option<Seg2>, KernelError> {
    let p: Vec<Vec2> = b.points.iter().map(|q| to2(*q)).collect();
    if p.iter().all(|q| (q.x - p[0].x).abs() < 1e-12 && (q.y - p[0].y).abs() < 1e-12) {
        return Ok(None);
    }
    let rational = b.weights.iter().any(|w| (w - b.weights[0]).abs() > 1e-12);
    let last = p.len() - 1;
    Ok(Some(match (b.degree, rational) {
        (1, _) => Seg2::Line { a: p[0], b: p[1] },
        (2, true) => {
            let w = b.weights[1] / (b.weights[0] * b.weights[2]).sqrt();
            Seg2::Conic { a: p[0], apex: p[1], b: p[2], w }
        }
        (_, true) => return Err(refused("rational curves above degree 2 cannot be extruded exactly yet")),
        (2, false) => {
            // degree elevation of a quadratic Bézier
            let c = |a: Vec2, b: Vec2, k: f64| Vec2::new(a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k);
            Seg2::Cubic { p0: p[0], p1: c(p[0], p[1], 2.0 / 3.0), p2: c(p[2], p[1], 2.0 / 3.0), p3: p[2] }
        }
        (3, false) => Seg2::Cubic { p0: p[0], p1: p[1], p2: p[2], p3: p[3] },
        (d, false) if d <= MAX_BEZIER_DEGREE => {
            let mut arr = [Vec2::new(0.0, 0.0); MAX_BEZIER_CTRL];
            arr[..=last].copy_from_slice(&p);
            Seg2::Bezier { n: d as u8, p: arr }
        }
        _ => return Err(refused("curves above degree 9 cannot be extruded")),
    }))
}

fn reversed(s: Seg2) -> Seg2 {
    match s {
        Seg2::Line { a, b } => Seg2::Line { a: b, b: a },
        Seg2::Arc { center, radius, start, sweep } => Seg2::Arc { center, radius, start: start + sweep, sweep: -sweep },
        Seg2::Cubic { p0, p1, p2, p3 } => Seg2::Cubic { p0: p3, p1: p2, p2: p1, p3: p0 },
        Seg2::Conic { a, apex, b, w } => Seg2::Conic { a: b, apex, b: a, w },
        Seg2::Bezier { n, mut p } => {
            p[..=n as usize].reverse();
            Seg2::Bezier { n, p }
        }
    }
}

fn start_of(s: &Seg2) -> Vec2 {
    match *s {
        Seg2::Line { a, .. } | Seg2::Conic { a, .. } => a,
        Seg2::Cubic { p0, .. } => p0,
        Seg2::Bezier { p, .. } => p[0],
        Seg2::Arc { center, radius, start, .. } => Vec2::new(center.x + radius * start.cos(), center.y + radius * start.sin()),
    }
}

fn with_end(s: Seg2, e: Vec2) -> Seg2 {
    match s {
        Seg2::Line { a, .. } => Seg2::Line { a, b: e },
        Seg2::Conic { a, apex, w, .. } => Seg2::Conic { a, apex, b: e, w },
        Seg2::Cubic { p0, p1, p2, .. } => Seg2::Cubic { p0, p1, p2, p3: e },
        Seg2::Bezier { n, mut p } => {
            p[n as usize] = e;
            Seg2::Bezier { n, p }
        }
        arc @ Seg2::Arc { .. } => arc,
    }
}

/// Each segment ends exactly where the next starts and the last where the first starts: the
/// pieces of a split curve differ by rounding, and the kernel wants a closed loop.
fn close_exactly(segs: &mut [Seg2]) {
    let n = segs.len();
    for i in 0..n {
        let next = start_of(&segs[(i + 1) % n]);
        segs[i] = with_end(segs[i], next);
    }
}


