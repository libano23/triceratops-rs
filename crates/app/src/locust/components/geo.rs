//! Geometry helpers shared by the components: conversions between data-tree values and
//! kernel objects, primitive builders and measurements. Modelling operations reuse the
//! functions the commands are built on.

use glam::{DAffine3, DQuat, DVec2, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{Brep, Face, Mesh, MeshDensity, NurbsCurve, NurbsSurface, Plane};

use super::super::data::{as_int, Value};

// ------------------------------------------------------------- conversions

pub fn as_curve(v: &Value) -> Option<NurbsCurve> {
    match v {
        Value::Geometry(g) => match g.as_ref() {
            Geometry::Curve(c) => Some(c.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// Every curve in a list of values (geometry items that are not curves are skipped).
pub fn curves(items: &[Value]) -> Vec<NurbsCurve> {
    items.iter().filter_map(as_curve).collect()
}

pub fn faces(g: &Geometry) -> Option<Vec<Face>> {
    match g {
        Geometry::Surface(s) => Some(vec![Face::untrimmed(s.clone())]),
        Geometry::Brep(b) => Some(b.faces.clone()),
        _ => None,
    }
}

pub fn to_brep(g: &Geometry) -> Option<Brep> {
    match g {
        Geometry::Brep(b) => Some(b.clone()),
        Geometry::Surface(s) => Some(Brep::new(vec![Face::untrimmed(s.clone())])),
        _ => None,
    }
}

pub fn first_surface(g: &Geometry) -> Option<NurbsSurface> {
    match g {
        Geometry::Surface(s) => Some(s.clone()),
        Geometry::Brep(b) => b.faces.first().map(|f| f.surface.clone()),
        _ => None,
    }
}

/// A mesh of any surface-like geometry (meshes as they are).
pub fn to_mesh(g: &Geometry) -> Option<Mesh> {
    let d = MeshDensity::for_size(g.bounding_box().diagonal().max(1e-9));
    match g {
        Geometry::Mesh(m) => Some(m.clone()),
        Geometry::Surface(s) => Some(s.display_mesh(&d)),
        Geometry::Brep(b) => Some(b.display_mesh(&d)),
        _ => None,
    }
}

/// A surface result: a single untrimmed face stays a surface, anything else a polysurface.
pub fn from_faces(mut faces: Vec<Face>) -> Option<Geometry> {
    if faces.is_empty() {
        return None;
    }
    if faces.len() == 1 && !faces[0].is_trimmed() {
        return Some(Geometry::Surface(faces.pop().unwrap().surface));
    }
    let mut b = Brep::new(faces);
    if b.closed {
        b.orient_outward();
    }
    Some(Geometry::Brep(b))
}

/// A document object referenced by id (`12`, `"12"`, `"#12"`) or by name.
pub fn find_object(doc: &Document, r: &Value) -> Option<Geometry> {
    let by_id = |id: ObjectId| doc.get(id).map(|o| o.geometry.clone());
    match r {
        Value::Int(_) | Value::Number(_) => by_id(as_int(r).max(0) as ObjectId),
        Value::Text(t) => {
            let t = t.trim();
            if let Ok(id) = t.trim_start_matches('#').parse::<ObjectId>() {
                if let Some(g) = by_id(id) {
                    return Some(g);
                }
            }
            doc.objects().find(|o| o.attributes.name.as_deref() == Some(t)).map(|o| o.geometry.clone())
        }
        _ => None,
    }
}

// ------------------------------------------------------------------ frames

/// An orthonormal frame (x, y, z) with z along `normal`; x does not collapse for a vertical
/// normal (the same convention as the Python Locust).
pub fn axes_from_normal(normal: DVec3) -> (DVec3, DVec3, DVec3) {
    let n = normal.normalize_or(DVec3::Z);
    let r = if n.z.abs() < 0.9 { DVec3::Z } else { DVec3::X };
    let x = r.cross(n).normalize();
    let y = n.cross(x);
    (x, y, n)
}

pub fn plane(origin: DVec3, normal: DVec3) -> Plane {
    let (x, y, z) = axes_from_normal(normal);
    Plane { origin, x_axis: x, y_axis: y, z_axis: z }
}

/// The transform taking frame `a` to frame `b`.
pub fn frame_to_frame(a: &Plane, b: &Plane) -> DAffine3 {
    let to_world = |p: &Plane| DAffine3::from_cols(p.x_axis, p.y_axis, p.z_axis, p.origin);
    to_world(b) * to_world(a).inverse()
}

pub fn transformed(g: &Geometry, xf: &DAffine3) -> Geometry {
    let mut out = g.clone();
    out.transform(xf);
    out
}

pub fn rotation(center: DVec3, axis: DVec3, degrees: f64) -> DAffine3 {
    DAffine3::from_translation(center) * DAffine3::from_quat(DQuat::from_axis_angle(axis.normalize_or(DVec3::Z), degrees.to_radians())) * DAffine3::from_translation(-center)
}

pub fn mirror(point: DVec3, normal: DVec3) -> DAffine3 {
    let n = normal.normalize_or(DVec3::X);
    let m = glam::DMat3::IDENTITY - glam::DMat3::from_cols(n * n.x, n * n.y, n * n.z) * 2.0;
    DAffine3::from_translation(point) * DAffine3::from_mat3(m) * DAffine3::from_translation(-point)
}

// --------------------------------------------------------------- curves

pub fn polyline(pts: Vec<DVec3>, closed: bool) -> Option<NurbsCurve> {
    let mut p = pts;
    if closed && p.len() > 2 && (p[0] - p[p.len() - 1]).length() > 1e-12 {
        p.push(p[0]);
    }
    NurbsCurve::polyline(p).ok()
}

/// A smooth closed cubic through the points (periodic interpolation).
pub fn interpolate_closed(pts: &[DVec3]) -> Option<NurbsCurve> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    // uniform periodic cubic: (P[i-1] + 4 P[i] + P[i+1]) / 6 = Q[i]; Gauss-Seidel converges
    // quickly (the system is diagonally dominant)
    let mut cp: Vec<DVec3> = pts.to_vec();
    for _ in 0..200 {
        let mut change: f64 = 0.0;
        for i in 0..n {
            let prev = cp[(i + n - 1) % n];
            let next = cp[(i + 1) % n];
            let new = (pts[i] * 6.0 - prev - next) / 4.0;
            change = change.max((new - cp[i]).length());
            cp[i] = new;
        }
        if change < 1e-12 {
            break;
        }
    }
    let c = NurbsCurve::periodic(3, cp).ok()?;
    // the periodic constructor may start at another station: re-seam at the first point
    let t = c.closest_parameter(pts[0]);
    let (t0, t1) = c.domain();
    if (t - t0).abs() < 1e-9 || (t - t1).abs() < 1e-9 {
        return Some(c);
    }
    c.split(t).and_then(|(l, r)| r.join_any(&l).ok()).or(Some(c))
}

pub fn interpolate(pts: &[DVec3], closed: bool) -> Option<NurbsCurve> {
    if closed {
        return interpolate_closed(pts);
    }
    if pts.len() < 2 {
        return None;
    }
    NurbsCurve::interpolate(pts, 3.min(pts.len() - 1)).ok()
}

/// Parameter at a normalised position `t` (0..1) of the domain.
pub fn param(c: &NurbsCurve, t: f64) -> f64 {
    let (a, b) = c.domain();
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// The point at a fraction of the curve length.
pub fn point_at_length_fraction(c: &NurbsCurve, f: f64) -> DVec3 {
    let pts = c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-9) * 1e-5);
    let total: f64 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let target = total * f.clamp(0.0, 1.0);
    let mut acc = 0.0;
    for w in pts.windows(2) {
        let l = (w[1] - w[0]).length();
        if acc + l >= target && l > 0.0 {
            let p = w[0] + (w[1] - w[0]) * ((target - acc) / l);
            return c.point_at(c.closest_parameter(p));
        }
        acc += l;
    }
    c.end_point()
}

/// Curvature and its radius at the parameter `t`.
pub fn curvature(c: &NurbsCurve, t: f64) -> (f64, f64) {
    let d1 = c.derivative_at(t);
    let h = (c.domain().1 - c.domain().0).abs().max(1e-9) * 1e-5;
    let d1b = c.derivative_at((t + h).min(c.domain().1));
    let d1a = c.derivative_at((t - h).max(c.domain().0));
    let dt = ((t + h).min(c.domain().1) - (t - h).max(c.domain().0)).max(1e-15);
    let d2 = (d1b - d1a) / dt;
    let s = d1.length();
    if s < 1e-15 {
        return (0.0, f64::INFINITY);
    }
    let k = d1.cross(d2).length() / (s * s * s);
    (k, if k > 1e-15 { 1.0 / k } else { f64::INFINITY })
}

/// A closed curve re-seamed at the point nearest `p`.
pub fn curve_seam(c: &NurbsCurve, p: DVec3) -> Option<NurbsCurve> {
    if !c.is_closed() {
        return None;
    }
    let t = c.closest_parameter(p);
    let (l, r) = c.split(t)?;
    r.join_any(&l).ok()
}

/// Join curves end to end, greedily (like the Join command).
pub fn join_curves(mut items: Vec<NurbsCurve>) -> Vec<NurbsCurve> {
    let mut out = Vec::new();
    while let Some(mut chain) = items.pop() {
        let mut progress = true;
        while progress {
            progress = false;
            let mut i = 0;
            while i < items.len() {
                if let Ok(j) = chain.join_any(&items[i]) {
                    chain = j;
                    items.remove(i);
                    progress = true;
                } else {
                    i += 1;
                }
            }
        }
        out.push(chain);
    }
    out.reverse();
    out
}

pub fn polygon(center: DVec3, radius: f64, sides: i64, normal: DVec3) -> Option<NurbsCurve> {
    if sides < 3 || radius <= 0.0 {
        return None;
    }
    let pl = plane(center, normal);
    let pts: Vec<DVec3> = (0..sides).map(|i| {
        let a = std::f64::consts::TAU * i as f64 / sides as f64;
        pl.origin + pl.x_axis * (radius * a.cos()) + pl.y_axis * (radius * a.sin())
    }).collect();
    polyline(pts, true)
}

// ---------------------------------------------------------- primitives

/// A closed polysurface from faces, oriented outward when it closes.
pub fn solid(faces: Vec<Face>) -> Geometry {
    let mut b = Brep::new(faces);
    if b.closed {
        b.orient_outward();
    }
    Geometry::Brep(b)
}

/// A planar face closing a closed planar curve. Its outline follows the curve within a
/// tight tolerance, so a capped solid measures (and draws) like the true shape.
pub fn cap(c: &NurbsCurve) -> Option<Face> {
    if !c.is_closed() {
        return None;
    }
    let pl = tri_geom::curve_plane(c)?;
    let tol = c.bounding_box().diagonal().max(1e-9) * 2e-5;
    let mut pts = c.tessellate_adaptive(tol);
    if pts.len() > 1 && (pts[0] - pts[pts.len() - 1]).length() < tol {
        pts.pop();
    }
    Face::planar(&pl, vec![pts]).ok()
}

/// Extrude a curve (capped into a solid when it is closed and planar, and `cap` is set) or
/// a surface (into a solid: the surface, its moved copy and the swept edges).
pub fn extrude(g: &Geometry, dir: DVec3, cap_ends: bool) -> Option<Geometry> {
    if dir.length() < 1e-12 {
        return None;
    }
    match g {
        Geometry::Curve(c) => {
            let side = NurbsSurface::extrude(c, dir).ok()?;
            if cap_ends && c.is_closed() {
                if let (Some(a), Some(mut b)) = (cap(c), cap(c)) {
                    b.transform(&DAffine3::from_translation(dir));
                    let s = solid(vec![Face::untrimmed(side.clone()), a, b]);
                    if s.is_solid() {
                        return Some(s);
                    }
                }
            }
            Some(Geometry::Surface(side))
        }
        Geometry::Surface(_) | Geometry::Brep(_) => {
            let fs = faces(g)?;
            let tol = g.bounding_box().diagonal().max(1e-6) * 1e-4;
            let mut all = Vec::new();
            for f in &fs {
                let mut base = f.clone();
                base.flip();
                all.push(base);
                let mut top = f.clone();
                top.transform(&DAffine3::from_translation(dir));
                all.push(top);
            }
            // side walls along the naked edges: exact for one face, from the welded mesh
            // boundary for a polysurface
            let walls: Vec<NurbsCurve> = if fs.len() == 1 {
                fs[0].edge_curves(tol)
            } else {
                let m = Brep::new(fs.clone()).display_mesh(&MeshDensity::for_size(g.bounding_box().diagonal().max(1e-9)));
                tri_geom::mesh_ops::naked_loops(&m).into_iter().filter_map(|mut l| {
                    l.push(l[0]);
                    NurbsCurve::polyline(l).ok()
                }).collect()
            };
            for c in walls {
                if let Ok(s) = NurbsSurface::extrude(&c, dir) {
                    all.push(Face::untrimmed(s));
                }
            }
            Some(solid(all))
        }
        _ => None,
    }
}

pub fn box_solid(corner: DVec3, x: f64, y: f64, z: f64) -> Option<Geometry> {
    if x.abs() < 1e-12 || y.abs() < 1e-12 || z.abs() < 1e-12 {
        return None;
    }
    let mut b = Brep::cuboid(&Plane { origin: corner, ..Plane::world_xy() }, x, y, z).ok()?;
    b.orient_outward();
    Some(Geometry::Brep(b))
}

pub fn sphere(center: DVec3, r: f64) -> Option<Geometry> {
    if r <= 0.0 {
        return None;
    }
    Some(Geometry::Surface(NurbsSurface::sphere(center, r).ok()?))
}

/// A capped cylinder (radius, height along `axis` from `base`).
pub fn cylinder(base: DVec3, r: f64, h: f64, axis: DVec3) -> Option<Geometry> {
    if r <= 0.0 || h.abs() < 1e-12 {
        return None;
    }
    let pl = plane(base, axis);
    let c = NurbsCurve::circle(&pl, r).ok()?;
    extrude(&Geometry::Curve(c), pl.z_axis * h, true)
}

/// A cone or truncated cone (r2 at the top; 0 makes a point).
pub fn cone(base: DVec3, r1: f64, r2: f64, h: f64) -> Option<Geometry> {
    if r1 < 0.0 || r2 < 0.0 || (r1 <= 0.0 && r2 <= 0.0) || h.abs() < 1e-12 {
        return None;
    }
    let pl = Plane { origin: base, ..Plane::world_xy() };
    let profile = NurbsCurve::line(pl.point_at_3(r1, 0.0, 0.0), pl.point_at_3(r2, 0.0, h)).ok()?;
    let side = NurbsSurface::revolve(&profile, base, DVec3::Z, std::f64::consts::TAU).ok()?;
    let mut fs = vec![Face::untrimmed(side)];
    for (r, z) in [(r1, 0.0), (r2, h)] {
        if r > 1e-12 {
            let c = NurbsCurve::circle(&Plane { origin: base + DVec3::Z * z, ..Plane::world_xy() }, r).ok()?;
            fs.push(cap(&c)?);
        }
    }
    Some(solid(fs))
}

pub fn torus(center: DVec3, major: f64, minor: f64, axis: DVec3) -> Option<Geometry> {
    if major <= 0.0 || minor <= 0.0 {
        return None;
    }
    Some(Geometry::Surface(NurbsSurface::torus(&plane(center, axis), major, minor).ok()?))
}

/// A bilinear patch through four corners (three make a triangle).
pub fn surface_from_corners(a: DVec3, b: DVec3, c: DVec3, d: DVec3) -> Option<Geometry> {
    let line = |p: DVec3, q: DVec3| NurbsCurve::line(p, q).ok();
    let curves = if (c - d).length() < 1e-9 { vec![line(a, b)?, line(b, c)?, line(c, a)?] } else { vec![line(a, b)?, line(b, c)?, line(c, d)?, line(d, a)?] };
    if curves.len() == 4 {
        return Some(Geometry::Surface(NurbsSurface::from_grid(1, 1, vec![vec![a, d], vec![b, c]]).ok()?));
    }
    crate::commands::surface2::edge_surface(curves).ok()
}

// ------------------------------------------------------------ measures

/// A tight mesh for measurements (area, volume, centroids).
pub fn measure_mesh(g: &Geometry) -> Option<Mesh> {
    let diag = g.bounding_box().diagonal().max(1e-9);
    let d = MeshDensity { max_angle: 2f64.to_radians(), chord: diag * 2e-6, max_per_span: 256 };
    match g {
        Geometry::Mesh(m) => Some(m.clone()),
        Geometry::Surface(s) => Some(s.display_mesh(&d)),
        Geometry::Brep(b) => Some(b.display_mesh(&d)),
        _ => None,
    }
}

pub fn area(g: &Geometry) -> Option<f64> {
    if let Geometry::Curve(_) = g {
        return Document::area_of(g);
    }
    let m = measure_mesh(g)?;
    Some((0..m.triangle_count()).map(|t| {
        let (a, b, c) = m.triangle(t);
        (b - a).cross(c - a).length() * 0.5
    }).sum())
}

pub fn area_centroid(g: &Geometry) -> Option<DVec3> {
    let m = measure_mesh(g)?;
    let (mut total, mut c) = (0.0, DVec3::ZERO);
    for t in 0..m.triangle_count() {
        let (a, b, d) = m.triangle(t);
        let ar = (b - a).cross(d - a).length() * 0.5;
        total += ar;
        c += (a + b + d) / 3.0 * ar;
    }
    (total > 1e-18).then(|| c / total)
}

/// Volume and volume centroid of a closed object (0 for open ones, like the Python Locust).
pub fn volume(g: &Geometry) -> Option<(f64, DVec3)> {
    let closed = match g {
        Geometry::Brep(b) => b.closed,
        Geometry::Surface(s) => tri_doc::surface_is_closed(s),
        Geometry::Mesh(m) => m.is_closed(),
        Geometry::Curve(_) | Geometry::Point(_) => return Some((0.0, centroid(g)?)),
        _ => return None,
    };
    if !closed {
        return Some((0.0, area_centroid(g)?));
    }
    let m = measure_mesh(g)?;
    let (mut v, mut c) = (0.0, DVec3::ZERO);
    for t in 0..m.triangle_count() {
        let (a, b, d) = m.triangle(t);
        let vt = a.dot(b.cross(d)) / 6.0;
        v += vt;
        c += (a + b + d) / 4.0 * vt;
    }
    if v.abs() < 1e-18 {
        return Some((0.0, area_centroid(g)?));
    }
    Some((v.abs(), c / v))
}

/// Centre of mass: by volume for solids, by area for surfaces, by length for curves.
pub fn centroid(g: &Geometry) -> Option<DVec3> {
    match g {
        Geometry::Point(p) => Some(*p),
        Geometry::Curve(c) => {
            let pts = c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-9) * 1e-5);
            let (mut total, mut acc) = (0.0, DVec3::ZERO);
            for w in pts.windows(2) {
                let l = (w[1] - w[0]).length();
                total += l;
                acc += (w[0] + w[1]) * 0.5 * l;
            }
            Some(if total > 0.0 { acc / total } else { c.start_point() })
        }
        _ => match volume(g) {
            Some((v, c)) if v > 0.0 => Some(c),
            _ => area_centroid(g),
        },
    }
}

/// Closest point on any geometry, and its distance.
pub fn closest_point(g: &Geometry, p: DVec3) -> Option<(DVec3, f64)> {
    let q = match g {
        Geometry::Point(x) => *x,
        Geometry::Curve(c) => c.point_at(c.closest_parameter(p)),
        Geometry::Surface(s) => {
            let (u, v) = s.closest_uv(p);
            s.point_at(u, v)
        }
        Geometry::Brep(b) => b.faces.iter().map(|f| face_closest(f, p)).min_by(|a, b| (*a - p).length().total_cmp(&(*b - p).length()))?,
        Geometry::Mesh(m) => (0..m.triangle_count()).map(|t| {
            let (a, b, c) = m.triangle(t);
            closest_on_triangle(p, a, b, c)
        }).min_by(|a, b| (*a - p).length().total_cmp(&(*b - p).length()))?,
        _ => return None,
    };
    Some((q, (q - p).length()))
}

/// Closest point on a (possibly trimmed) face: on the surface inside the trims, else on
/// its edges.
pub fn face_closest(f: &Face, p: DVec3) -> DVec3 {
    let (u, v) = f.surface.closest_uv(p);
    if !f.is_trimmed() || f.contains_uv(DVec2::new(u, v)) {
        return f.surface.point_at(u, v);
    }
    let tol = f.bounding_box().diagonal().max(1e-9) * 1e-5;
    f.display_edges(tol)
        .iter()
        .flat_map(|poly| poly.windows(2).map(|w| closest_on_segment(p, w[0], w[1])).collect::<Vec<_>>())
        .min_by(|a, b| (*a - p).length().total_cmp(&(*b - p).length()))
        .unwrap_or_else(|| f.surface.point_at(u, v))
}

pub fn closest_on_segment(p: DVec3, a: DVec3, b: DVec3) -> DVec3 {
    let d = b - a;
    let l2 = d.length_squared();
    if l2 < 1e-30 {
        return a;
    }
    a + d * ((p - a).dot(d) / l2).clamp(0.0, 1.0)
}

pub fn closest_on_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    let n = (b - a).cross(c - a);
    if n.length_squared() > 1e-30 {
        let nn = n.normalize();
        let q = p - nn * (p - a).dot(nn);
        let inside = |x: DVec3, y: DVec3| (y - x).cross(q - x).dot(n) >= 0.0;
        if inside(a, b) && inside(b, c) && inside(c, a) {
            return q;
        }
    }
    [closest_on_segment(p, a, b), closest_on_segment(p, b, c), closest_on_segment(p, c, a)].into_iter().min_by(|x, y| (*x - p).length().total_cmp(&(*y - p).length())).unwrap()
}

/// Surface parameters, point and normal nearest `p` on the first face.
pub fn surface_uv(g: &Geometry, p: DVec3) -> Option<(NurbsSurface, f64, f64)> {
    let s = first_surface(g)?;
    let (u, v) = s.closest_uv(p);
    Some((s, u, v))
}

/// Edge curves of a face, as curves.
pub fn edge_curves(f: &Face) -> Vec<NurbsCurve> {
    f.edge_curves(f.bounding_box().diagonal().max(1e-9) * 1e-4)
}

/// Fillet or chamfer every edge shared by two planar faces of a polysurface, one after the
/// other (the edges that cannot be rounded are left sharp).
pub fn round_all_edges(b: &Brep, amount: f64, chamfer: bool) -> Option<Geometry> {
    if amount <= 0.0 {
        return None;
    }
    let mut faces = b.faces.clone();
    // the midpoints of the original edges, each tried once
    let mut picks: Vec<DVec3> = Vec::new();
    for f in &b.faces {
        if let Some((_, loops)) = crate::commands::solid_edit::planar_loops(f) {
            for l in loops {
                for k in 0..l.len() {
                    let m = (l[k] + l[(k + 1) % l.len()]) * 0.5;
                    if !picks.iter().any(|q| (*q - m).length() < 1e-9) {
                        picks.push(m);
                    }
                }
            }
        }
    }
    let mut done = 0;
    for m in picks {
        if let Ok(nf) = crate::commands::solid_edit::round_edge(&faces, m, amount, chamfer) {
            faces = nf;
            done += 1;
        }
    }
    if done == 0 {
        return None;
    }
    Some(solid(faces))
}
