//! Real implementations of the remaining surface and solid commands.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{curve_plane, mesh_boolean, mesh_ops, BooleanOp, Brep, Face, Mesh, NurbsCurve, NurbsSurface, Plane, Ray};

use super::create::planar_cap;
use super::misc4::{prism_mesh, solid_mesh};
use super::misc6::pull_point;
use super::simple::{add_all, Ans, Ask, SeqCmd};
use super::solid_edit::planar_loops;
use super::surface_edit::{face_under, faces_of};
use super::{fmt_num, opt, optv, yesno, CmdCtx, Input, ObjFilter, Prompt, Step};

fn curve_of(doc: &Document, id: ObjectId) -> Option<NurbsCurve> {
    doc.get(id).and_then(|o| o.geometry.as_curve().cloned())
}

/// Split a surface at parameter `u` (dir = true) or `v` into two surfaces.
pub fn split_surface(s: &NurbsSurface, dir_u: bool, t: f64) -> Option<(NurbsSurface, NurbsSurface)> {
    let (nu, nv) = (s.count_u, s.count_v);
    let mut left_rows: Vec<Vec<DVec3>> = Vec::new();
    let mut right_rows: Vec<Vec<DVec3>> = Vec::new();
    let mut knots_l = Vec::new();
    let mut knots_r = Vec::new();
    let lines = if dir_u { nv } else { nu };
    for k in 0..lines {
        let (cps, ws): (Vec<DVec3>, Vec<f64>) = if dir_u {
            ((0..nu).map(|i| s.control_points[i * nv + k]).collect(), (0..nu).map(|i| s.weights[i * nv + k]).collect())
        } else {
            ((0..nv).map(|j| s.control_points[k * nv + j]).collect(), (0..nv).map(|j| s.weights[k * nv + j]).collect())
        };
        let (deg, knots) = if dir_u { (s.degree_u, s.knots_u.clone()) } else { (s.degree_v, s.knots_v.clone()) };
        let c = NurbsCurve::new(deg, cps, ws, knots).ok()?;
        let (l, r) = c.split(t)?;
        knots_l = l.knots.clone();
        knots_r = r.knots.clone();
        left_rows.push(l.control_points);
        right_rows.push(r.control_points);
    }
    let build = |rows: &Vec<Vec<DVec3>>, knots: Vec<f64>| -> Option<NurbsSurface> {
        let n = rows[0].len();
        if dir_u {
            let mut cps = vec![DVec3::ZERO; n * nv];
            for (k, row) in rows.iter().enumerate() {
                for (i, p) in row.iter().enumerate() {
                    cps[i * nv + k] = *p;
                }
            }
            NurbsSurface::new(s.degree_u, s.degree_v, n, nv, cps, vec![1.0; n * nv], knots, s.knots_v.clone()).ok()
        } else {
            let mut cps = vec![DVec3::ZERO; nu * n];
            for (k, row) in rows.iter().enumerate() {
                for (j, p) in row.iter().enumerate() {
                    cps[k * n + j] = *p;
                }
            }
            NurbsSurface::new(s.degree_u, s.degree_v, nu, n, cps, vec![1.0; nu * n], s.knots_u.clone(), knots).ok()
        }
    };
    Some((build(&left_rows, knots_l)?, build(&right_rows, knots_r)?))
}

fn interior_knots(knots: &[f64], degree: usize, full_only: bool) -> Vec<f64> {
    let inner = &knots[degree + 1..knots.len() - degree - 1];
    let mut out: Vec<f64> = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        let k = inner[i];
        let mut m = 1;
        while i + m < inner.len() && (inner[i + m] - k).abs() < 1e-12 {
            m += 1;
        }
        if !full_only || m >= degree {
            out.push(k);
        }
        i += m;
    }
    out
}

pub fn split_spans(full_only: bool) -> SeqCmd {
    let label = if full_only { "DivideAlongCreases" } else { "ConvertToSingleSpans" };
    SeqCmd::new(vec![Ask::Objects("Select surfaces", ObjFilter::Surface, 1)], move |a, ctx| {
        ctx.doc.begin_undo(label);
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(Geometry::Surface(s)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let mut pieces = vec![s.clone()];
            for u in interior_knots(&s.knots_u, s.degree_u, full_only) {
                let mut next = Vec::new();
                for p in pieces {
                    let (u0, u1) = p.domain_u();
                    if u > u0 + 1e-9 && u < u1 - 1e-9 {
                        if let Some((l, r)) = split_surface(&p, true, u) {
                            next.push(l);
                            next.push(r);
                            continue;
                        }
                    }
                    next.push(p);
                }
                pieces = next;
            }
            for v in interior_knots(&s.knots_v, s.degree_v, full_only) {
                let mut next = Vec::new();
                for p in pieces {
                    let (v0, v1) = p.domain_v();
                    if v > v0 + 1e-9 && v < v1 - 1e-9 {
                        if let Some((l, r)) = split_surface(&p, false, v) {
                            next.push(l);
                            next.push(r);
                            continue;
                        }
                    }
                    next.push(p);
                }
                pieces = next;
            }
            if pieces.len() > 1 {
                ctx.doc.remove(id);
                if full_only {
                    out.push(Geometry::Brep(Brep::new(pieces.into_iter().map(Face::untrimmed).collect())));
                } else {
                    out.extend(pieces.into_iter().map(Geometry::Surface));
                }
            }
        }
        if out.is_empty() {
            ctx.log("The surfaces already have single spans.");
            return Step::Done;
        }
        add_all(ctx, label, out)
    })
}

pub fn create_solid() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces and polysurfaces to join into a solid", ObjFilter::Surface, 2)], |a, ctx| {
        let mut faces = Vec::new();
        for id in a[0].ids() {
            faces.extend(faces_of(ctx.doc, id).unwrap_or_default());
        }
        let brep = Brep::new(faces);
        ctx.doc.begin_undo("CreateSolid");
        for id in a[0].ids() {
            ctx.doc.remove(id);
        }
        let closed = brep.closed;
        let r = add_all(ctx, "CreateSolid", vec![Geometry::Brep(brep)]);
        ctx.log(if closed { "Closed solid created." } else { "The surfaces do not enclose a volume: open polysurface created." });
        r
    })
}

pub fn chamfer_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first planar surface", ObjFilter::Surface), Ask::Pick("Select second planar surface", ObjFilter::Surface), Ask::Number("Chamfer distance", 1.0)], |a, ctx| {
        let (Ans::Pick(ida, pa), Ans::Pick(idb, pb)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let d = a[2].number().abs();
        let (Some(fa), Some(fb)) = (faces_of(ctx.doc, *ida), faces_of(ctx.doc, *idb)) else { return Step::Cancelled };
        let (Some((plane_a, loops_a)), Some((plane_b, loops_b))) = (planar_loops(&fa[face_under(&fa, *pa)]), planar_loops(&fb[face_under(&fb, *pb)])) else {
            ctx.log("ChamferSrf works between planar surfaces in triceratops.");
            return Step::Cancelled;
        };
        let (na, nb) = (plane_a.z_axis, plane_b.z_axis);
        let e = na.cross(nb);
        if e.length() < 1e-9 {
            return Step::Cancelled;
        }
        let e = e.normalize();
        let m = glam::DMat3::from_cols(na, nb, e).transpose();
        let line_pt = m.inverse() * DVec3::new(na.dot(plane_a.origin), nb.dot(plane_b.origin), e.dot(plane_a.origin));
        let perp = |p: DVec3| { let v = p - line_pt; v - e * v.dot(e) };
        let da = perp(*pa).normalize_or(DVec3::X);
        let db = perp(*pb).normalize_or(DVec3::Y);
        let ta = line_pt + da * d;
        let tb = line_pt + db * d;
        let extent = |loops: &Vec<Vec<DVec3>>| loops.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| { let t = (*p - line_pt).dot(e); (lo.min(t), hi.max(t)) });
        let (la, ha) = extent(&loops_a);
        let (lb, hb) = extent(&loops_b);
        let (lo, hi) = (la.max(lb), ha.min(hb));
        if hi - lo < 1e-9 {
            return Step::Cancelled;
        }
        let Ok(chamfer) = NurbsSurface::from_grid(1, 1, vec![vec![ta + e * lo, tb + e * lo], vec![ta + e * hi, tb + e * hi]]) else { return Step::Cancelled };
        ctx.doc.begin_undo("ChamferSrf");
        let big = (hi - lo).max(1.0) * 10.0;
        for (id, pick, t) in [(*ida, *pa, ta), (*idb, *pb, tb)] {
            let cutter = NurbsCurve::line(t - e * big, t + e * big).unwrap();
            let dd = perp(pick).normalize_or(DVec3::X);
            let remove_pt = t - dd * (d * 0.5) + e * ((pick - line_pt).dot(e) - (t - line_pt).dot(e));
            super::surface_edit::trim_face(ctx, id, remove_pt, &[cutter]);
        }
        ctx.doc.clear_selection();
        let nid = ctx.doc.add(Geometry::Surface(chamfer));
        ctx.doc.select(nid, true);
        Step::Done
    })
}

pub fn connect_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first planar surface", ObjFilter::Surface), Ask::Pick("Select second planar surface", ObjFilter::Surface)], |a, ctx| {
        let (Ans::Pick(ida, pa), Ans::Pick(idb, pb)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(fa), Some(fb)) = (faces_of(ctx.doc, *ida), faces_of(ctx.doc, *idb)) else { return Step::Cancelled };
        let (Some((plane_a, loops_a)), Some((plane_b, loops_b))) = (planar_loops(&fa[face_under(&fa, *pa)]), planar_loops(&fb[face_under(&fb, *pb)])) else {
            ctx.log("ConnectSrf works between planar surfaces in triceratops.");
            return Step::Cancelled;
        };
        let (na, nb) = (plane_a.z_axis, plane_b.z_axis);
        let e = na.cross(nb);
        if e.length() < 1e-9 {
            ctx.log("The surfaces are parallel.");
            return Step::Cancelled;
        }
        let e = e.normalize();
        let m = glam::DMat3::from_cols(na, nb, e).transpose();
        let line_pt = m.inverse() * DVec3::new(na.dot(plane_a.origin), nb.dot(plane_b.origin), e.dot(plane_a.origin));
        ctx.doc.begin_undo("ConnectSrf");
        for (id, loops, plane, pick) in [(*ida, loops_a, plane_a, *pa), (*idb, loops_b, plane_b, *pb)] {
            // rebuild the face as the part of its plane on the pick side of the intersection line,
            // extended to the line: move vertices across the line onto it, add the line as an edge
            let perp = |p: DVec3| { let v = p - line_pt; v - e * v.dot(e) };
            let side = perp(pick).normalize_or(DVec3::X);
            let (lo, hi) = loops.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| { let t = (*p - line_pt).dot(e); (lo.min(t), hi.max(t)) });
            let far = loops.iter().flatten().map(|p| perp(*p).dot(side)).fold(0.0f64, f64::max).max(1e-6);
            let poly = vec![line_pt + e * lo, line_pt + e * hi, line_pt + e * hi + side * far, line_pt + e * lo + side * far];
            if let Ok(f) = Face::planar(&plane, vec![poly]) {
                ctx.doc.replace_geometry(id, Geometry::Surface(f.surface));
            }
        }
        Step::Done
    })
}

pub fn merge_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first untrimmed surface near the shared edge", ObjFilter::Surface), Ask::Pick("Select second untrimmed surface", ObjFilter::Surface)], |a, ctx| {
        let (Some(Geometry::Surface(s1)), Some(Geometry::Surface(s2))) = (ctx.doc.get(a[0].ids()[0]).map(|o| o.geometry.clone()), ctx.doc.get(a[1].ids()[0]).map(|o| o.geometry.clone())) else {
            ctx.log("MergeSrf needs two untrimmed surfaces.");
            return Step::Cancelled;
        };
        // sample both surfaces as grids along their shared edge and interpolate one grid
        let n = 12;
        let (u0, u1) = s1.domain_u();
        let (v0, v1) = s1.domain_v();
        let (w0, w1) = s2.domain_u();
        let (x0, x1) = s2.domain_v();
        let corner = |s: &NurbsSurface, u: f64, v: f64| s.point_at(u, v);
        // find which edges touch: compare edge midpoints
        let edges1 = [(corner(&s1, u0, (v0 + v1) * 0.5), "u0"), (corner(&s1, u1, (v0 + v1) * 0.5), "u1"), (corner(&s1, (u0 + u1) * 0.5, v0), "v0"), (corner(&s1, (u0 + u1) * 0.5, v1), "v1")];
        let edges2 = [(corner(&s2, w0, (x0 + x1) * 0.5), "u0"), (corner(&s2, w1, (x0 + x1) * 0.5), "u1"), (corner(&s2, (w0 + w1) * 0.5, x0), "v0"), (corner(&s2, (w0 + w1) * 0.5, x1), "v1")];
        let mut best = (f64::INFINITY, "u1", "u0");
        for (p, e1) in &edges1 {
            for (q, e2) in &edges2 {
                let d = (*p - *q).length();
                if d < best.0 {
                    best = (d, e1, e2);
                }
            }
        }
        let tol = (s1.bounding_box().diagonal() + s2.bounding_box().diagonal()) * 1e-3;
        if best.0 > tol {
            ctx.log("The surfaces do not share an edge.");
            return Step::Cancelled;
        }
        // grid of s1 oriented so the shared edge is at the end (i = n), then continue with s2
        let sample = |s: &NurbsSurface, edge: &str, from_edge: bool| -> Vec<Vec<DVec3>> {
            let (a0, a1) = s.domain_u();
            let (b0, b1) = s.domain_v();
            (0..=n).map(|i| {
                let f = i as f64 / n as f64;
                (0..=n).map(|j| {
                    let g = j as f64 / n as f64;
                    let (u, v) = match edge {
                        "u1" => (a0 + (a1 - a0) * if from_edge { 1.0 - f } else { f }, b0 + (b1 - b0) * g),
                        "u0" => (a0 + (a1 - a0) * if from_edge { f } else { 1.0 - f }, b0 + (b1 - b0) * g),
                        "v1" => (a0 + (a1 - a0) * g, b0 + (b1 - b0) * if from_edge { 1.0 - f } else { f }),
                        _ => (a0 + (a1 - a0) * g, b0 + (b1 - b0) * if from_edge { f } else { 1.0 - f }),
                    };
                    s.point_at(u, v)
                }).collect()
            }).collect()
        };
        let mut g1 = sample(&s1, best.1, false);
        let mut g2 = sample(&s2, best.2, true);
        // align the shared row direction
        if (g1[n][0] - g2[0][0]).length() > (g1[n][0] - g2[0][n]).length() {
            for row in g2.iter_mut() {
                row.reverse();
            }
        }
        g1.extend(g2.into_iter().skip(1));
        match NurbsSurface::interpolate_grid(&g1, 3, 3) {
            Ok(s) => {
                ctx.doc.begin_undo("MergeSrf");
                ctx.doc.remove(a[0].ids()[0]);
                ctx.doc.remove(a[1].ids()[0]);
                add_all(ctx, "MergeSrf", vec![Geometry::Surface(s)])
            }
            Err(e) => {
                ctx.log(format!("MergeSrf failed: {e}"));
                Step::Cancelled
            }
        }
    })
}

pub fn match_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select untrimmed surface edge to change", ObjFilter::Surface), Ask::Pick("Select target surface edge", ObjFilter::Surface)], |a, ctx| {
        let (Ans::Pick(id1, p1), Ans::Pick(id2, p2)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(Geometry::Surface(mut s)), Some(f2)) = (ctx.doc.get(*id1).map(|o| o.geometry.clone()), faces_of(ctx.doc, *id2)) else {
            ctx.log("MatchSrf changes an untrimmed surface.");
            return Step::Cancelled;
        };
        // nearest edge of s to the pick: move that row/column of control points onto the target surface
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let mid = |a: f64, b: f64| (a + b) * 0.5;
        let cands = [((s.point_at(u0, mid(v0, v1)) - *p1).length(), 0usize), ((s.point_at(u1, mid(v0, v1)) - *p1).length(), 1), ((s.point_at(mid(u0, u1), v0) - *p1).length(), 2), ((s.point_at(mid(u0, u1), v1) - *p1).length(), 3)];
        let edge = cands.iter().min_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).unwrap().1;
        let (nu, nv) = (s.count_u, s.count_v);
        let idx: Vec<usize> = match edge {
            0 => (0..nv).collect(),
            1 => (0..nv).map(|j| (nu - 1) * nv + j).collect(),
            2 => (0..nu).map(|i| i * nv).collect(),
            _ => (0..nu).map(|i| i * nv + nv - 1).collect(),
        };
        for i in idx {
            let p = s.control_points[i];
            s.control_points[i] = pull_point(&f2, p).0;
        }
        let _ = p2;
        ctx.doc.begin_undo("MatchSrf");
        ctx.doc.replace_geometry(*id1, Geometry::Surface(s));
        Step::Done
    })
}

pub fn bounce() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces, polysurfaces or meshes to bounce off", ObjFilter::Any, 1), Ask::Point("Start of ray"), Ask::Point("Direction point"), Ask::Number("Number of bounces", 5.0)], |a, ctx| {
        let meshes: Vec<Mesh> = a[0].ids().iter().filter_map(|id| solid_mesh(ctx.doc, *id)).collect();
        let mut origin = a[1].point();
        let mut dir = (a[2].point() - origin).normalize_or(DVec3::X);
        let mut pts = vec![origin];
        for _ in 0..a[3].number().round().max(1.0) as usize {
            let ray = Ray::new(origin + dir * 1e-6, dir);
            let mut best: Option<(f64, DVec3)> = None;
            for m in &meshes {
                if let Some(t) = m.intersect_ray(&ray) {
                    if best.is_none_or(|b| t < b.0) {
                        // normal of the hit triangle
                        let hit = ray.at(t);
                        let mut n = DVec3::Z;
                        for k in 0..m.triangle_count() {
                            let (p, q, r) = m.triangle(k);
                            let tn = (q - p).cross(r - p);
                            if tn.length_squared() < 1e-24 {
                                continue;
                            }
                            let tn = tn.normalize();
                            if ((hit - p).dot(tn)).abs() < 1e-6 && tri_geom::mesh_ops::tri_tri_segment((p, q, r), (hit - tn * 1e-3, hit + tn * 1e-3, hit + tn * 1e-3 + (q - p) * 1e-6)).is_some() {
                                n = tn;
                                break;
                            }
                        }
                        best = Some((t, n));
                    }
                }
            }
            let Some((t, n)) = best else { break };
            origin = ray.at(t);
            pts.push(origin);
            dir = (dir - n * (2.0 * dir.dot(n))).normalize_or(dir);
        }
        if pts.len() < 2 {
            ctx.log("The ray hits nothing.");
            return Step::Cancelled;
        }
        match NurbsCurve::polyline(pts) {
            Ok(c) => add_all(ctx, "Bounce", vec![Geometry::Curve(c)]),
            Err(_) => Step::Cancelled,
        }
    })
}

pub fn infinite_plane() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to intersect with the plane", ObjFilter::Any, 1), Ask::Point("First point on plane"), Ask::Point("Second point on plane"), Ask::Point("Third point on plane")], |a, ctx| {
        let (p, q, r) = (a[1].point(), a[2].point(), a[3].point());
        let plane = Plane::from_frame(p, q - p, r - p);
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(m) = solid_mesh(ctx.doc, id).or_else(|| ctx.doc.get(id).and_then(|o| o.geometry.as_curve()).map(|_| Mesh::new())) else { continue };
            if m.triangle_count() == 0 {
                if let Some(c) = curve_of(ctx.doc, id) {
                    for x in super::curves3_plane_points(&c, &plane) {
                        out.push(Geometry::Point(x));
                    }
                }
                continue;
            }
            for poly in m.section(&plane, m.bounding_box().diagonal().max(1e-6) * 1e-6) {
                if let Ok(c) = NurbsCurve::polyline(poly) {
                    out.push(Geometry::Curve(c));
                }
            }
        }
        add_all(ctx, "InfinitePlane", out)
    })
}

pub fn truncated_pyramid() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Center of base"), Ask::Point("Corner of base"), Ask::Number("Number of sides", 4.0), Ask::PointOrNumber("Height", 5.0), Ask::Number("Top scale (0..1)", 0.5)], |a, ctx| {
        let (c, corner) = (a[0].point(), a[1].point());
        let sides = a[2].number().round().clamp(3.0, 360.0) as usize;
        let cp = ctx.cplane;
        let h = match &a[3] { Ans::Number(n) => *n, Ans::Point(p) => (*p - c).dot(cp.z_axis), _ => 5.0 };
        let scale = a[4].number().clamp(0.01, 0.99);
        let plane = Plane { origin: c, ..cp };
        let (u, v, _) = plane.closest_parameters(corner);
        let r = (u * u + v * v).sqrt();
        let a0 = v.atan2(u);
        let poly = |rr: f64, z: f64| -> Vec<DVec3> { (0..=sides).map(|i| { let ang = a0 + std::f64::consts::TAU * i as f64 / sides as f64; plane.point_at(rr * ang.cos(), rr * ang.sin()) + cp.z_axis * z }).collect() };
        let (Ok(bottom), Ok(top)) = (NurbsCurve::polyline(poly(r, 0.0)), NurbsCurve::polyline(poly(r * scale, h))) else { return Step::Cancelled };
        let Ok(side) = NurbsSurface::loft(&[bottom.clone(), top.clone()], 1) else { return Step::Cancelled };
        let (Some(fb), Some(ft)) = (planar_cap(&bottom), planar_cap(&top)) else { return Step::Cancelled };
        let mut brep = Brep::new(vec![Face::untrimmed(side), fb, ft]);
        brep.orient_outward();
        brep.closed = true;
        add_all(ctx, "TruncatedPyramid", vec![Geometry::Brep(brep)])
    })
}

pub fn paraboloid() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Vertex of paraboloid"), Ask::Point("Focus"), Ask::PointOrNumber("Radius at the rim", 5.0), Ask::Options("Cap", vec![opt("Yes"), opt("No")])], |a, ctx| {
        let (v, f) = (a[0].point(), a[1].point());
        let axis = (f - v).normalize_or(ctx.cplane.z_axis);
        let fl = (f - v).length().max(1e-9);
        let r = match &a[2] { Ans::Number(n) => n.abs(), Ans::Point(p) => (*p - v).length(), _ => 5.0 };
        let x = axis.any_orthonormal_vector();
        let h = r * r / (4.0 * fl);
        let profile = NurbsCurve::from_control_points(2, vec![v, v + x * (r * 0.5) + axis * 0.0, v + x * r + axis * h]);
        // exact parabola profile: quadratic Bézier with control point at (r/2, 0)
        let Ok(profile) = profile else { return Step::Cancelled };
        let Ok(mut srf) = NurbsSurface::revolve(&profile, v, axis, std::f64::consts::TAU) else { return Step::Cancelled };
        srf.flip();
        if a[3].text() == "No" {
            return add_all(ctx, "Paraboloid", vec![Geometry::Surface(srf)]);
        }
        let rim_plane = Plane::from_normal(v + axis * h, axis);
        let Ok(rim) = NurbsCurve::circle(&rim_plane, r) else { return Step::Cancelled };
        let Some(cap) = planar_cap(&rim) else { return Step::Cancelled };
        let mut brep = Brep::new(vec![Face::untrimmed(srf), cap]);
        brep.orient_outward();
        brep.closed = true;
        add_all(ctx, "Paraboloid", vec![Geometry::Brep(brep)])
    })
}

pub fn drape(points_only: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to drape over", ObjFilter::Any, 1), Ask::Point("First corner of drape"), Ask::Point("Other corner"), Ask::Number("Samples per side", 20.0)], move |a, ctx| {
        let meshes: Vec<Mesh> = a[0].ids().iter().filter_map(|id| solid_mesh(ctx.doc, *id)).collect();
        if meshes.is_empty() {
            return Step::Cancelled;
        }
        let cp = ctx.cplane;
        let (u0, v0, _) = cp.closest_parameters(a[1].point());
        let (u1, v1, _) = cp.closest_parameters(a[2].point());
        let n = a[3].number().round().clamp(2.0, 100.0) as usize;
        let top = meshes.iter().fold(tri_geom::BoundingBox::EMPTY, |b, m| b.union(&m.bounding_box()));
        let zmax = top.corners().iter().map(|c| (*c - cp.origin).dot(cp.z_axis)).fold(f64::NEG_INFINITY, f64::max) + 1.0;
        let zmin = top.corners().iter().map(|c| (*c - cp.origin).dot(cp.z_axis)).fold(f64::INFINITY, f64::min);
        let mut grid: Vec<Vec<DVec3>> = Vec::new();
        for i in 0..n {
            let mut row = Vec::new();
            for j in 0..n {
                let u = u0 + (u1 - u0) * i as f64 / (n - 1) as f64;
                let v = v0 + (v1 - v0) * j as f64 / (n - 1) as f64;
                let start = cp.point_at_3(u, v, zmax);
                let ray = Ray::new(start, -cp.z_axis);
                let hit = meshes.iter().filter_map(|m| m.intersect_ray(&ray)).fold(f64::INFINITY, f64::min);
                row.push(if hit.is_finite() { ray.at(hit) } else { cp.point_at_3(u, v, zmin) });
            }
            grid.push(row);
        }
        if points_only {
            let pts: Vec<Geometry> = grid.iter().flatten().map(|p| Geometry::Point(*p)).collect();
            return add_all(ctx, "DrapePt", pts);
        }
        match NurbsSurface::interpolate_grid(&grid, 3, 3) {
            Ok(s) => add_all(ctx, "Drape", vec![Geometry::Surface(s)]),
            Err(e) => {
                ctx.log(format!("Drape failed: {e}"));
                Step::Cancelled
            }
        }
    })
}

pub fn heightfield() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Image file"), Ask::Point("First corner"), Ask::Point("Other corner"), Ask::Number("Height", 5.0), Ask::Number("Samples per side", 40.0)], |a, ctx| {
        let path = a[0].text();
        let Ok(img) = image::open(&path) else {
            ctx.log(format!("Unable to read image {path}."));
            return Step::Cancelled;
        };
        let gray = img.to_luma8();
        let (w, h) = gray.dimensions();
        let cp = ctx.cplane;
        let (u0, v0, z) = cp.closest_parameters(a[1].point());
        let (u1, v1, _) = cp.closest_parameters(a[2].point());
        let height = a[3].number();
        let n = a[4].number().round().clamp(2.0, 200.0) as usize;
        let grid: Vec<Vec<DVec3>> = (0..n).map(|i| (0..n).map(|j| {
            let fu = i as f64 / (n - 1) as f64;
            let fv = j as f64 / (n - 1) as f64;
            let px = ((fu * (w - 1) as f64).round() as u32).min(w - 1);
            let py = (((1.0 - fv) * (h - 1) as f64).round() as u32).min(h - 1);
            let g = gray.get_pixel(px, py).0[0] as f64 / 255.0;
            cp.point_at_3(u0 + (u1 - u0) * fu, v0 + (v1 - v0) * fv, z + g * height)
        }).collect()).collect();
        match NurbsSurface::interpolate_grid(&grid, 3, 3) {
            Ok(s) => add_all(ctx, "Heightfield", vec![Geometry::Surface(s)]),
            Err(e) => {
                ctx.log(format!("Heightfield failed: {e}"));
                Step::Cancelled
            }
        }
    })
}

/// Map objects from a base surface's UV space onto a target surface (FlowAlongSrf / Splop).
pub fn flow_along_srf(label: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to flow", ObjFilter::Any, 1), Ask::Pick("Select base surface", ObjFilter::Surface), Ask::Pick("Select target surface", ObjFilter::Surface), Ask::Options("Copy", vec![opt("Yes"), opt("No")])], move |a, ctx| {
        let (Some(fb), Some(ft)) = (faces_of(ctx.doc, a[1].ids()[0]), faces_of(ctx.doc, a[2].ids()[0])) else { return Step::Cancelled };
        let (sb, st) = (&fb[0].surface, &ft[0].surface);
        let (bu0, bu1) = sb.domain_u();
        let (bv0, bv1) = sb.domain_v();
        let (tu0, tu1) = st.domain_u();
        let (tv0, tv1) = st.domain_v();
        let map = |p: DVec3| -> DVec3 {
            let (u, v) = sb.closest_uv(p);
            let on = sb.point_at(u, v);
            let off = (p - on).dot(sb.normal_at(u, v));
            let fu = (u - bu0) / (bu1 - bu0).max(1e-12);
            let fv = (v - bv0) / (bv1 - bv0).max(1e-12);
            let (tu, tv) = (tu0 + (tu1 - tu0) * fu, tv0 + (tv1 - tv0) * fv);
            st.point_at(tu, tv) + st.normal_at(tu, tv) * off
        };
        let copy = a[3].text() != "No";
        ctx.doc.begin_undo(label);
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(mut g) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            match &mut g {
                Geometry::Point(p) => *p = map(*p),
                Geometry::Curve(c) => c.control_points.iter_mut().for_each(|p| *p = map(*p)),
                Geometry::Surface(s) => s.control_points.iter_mut().for_each(|p| *p = map(*p)),
                Geometry::Brep(b) => b.faces.iter_mut().for_each(|f| f.surface.control_points.iter_mut().for_each(|p| *p = map(*p))),
                Geometry::Mesh(m) => {
                    m.vertices.iter_mut().for_each(|p| *p = map(*p));
                    m.compute_normals();
                }
                Geometry::Instance { xf, .. } => xf.translation = map(xf.translation),
            }
            if copy {
                out.push(g);
            } else {
                ctx.doc.replace_geometry(id, g);
            }
        }
        if copy {
            return add_all(ctx, label, out);
        }
        Step::Done
    })
}

pub fn scale_by_plane() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Point("Origin"), Ask::Number("X scale factor", 1.0), Ask::Number("Y scale factor", 1.0)], |a, ctx| {
        let cp = ctx.cplane;
        let o = a[1].point();
        let (sx, sy) = (a[2].number(), a[3].number());
        ctx.doc.begin_undo("ScaleByPlane");
        for id in a[0].ids() {
            let Some(mut g) = ctx.doc.get(id).map(|ob| ob.geometry.clone()) else { continue };
            super::misc3::deform_pub(&mut g, &|p| { let d = p - o; o + cp.x_axis * (d.dot(cp.x_axis) * sx) + cp.y_axis * (d.dot(cp.y_axis) * sy) + cp.z_axis * d.dot(cp.z_axis) });
            ctx.doc.replace_geometry(id, g);
        }
        Step::Done
    })
}

pub fn scale_positions() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 2), Ask::Point("Base point"), Ask::Number("Scale factor", 2.0)], |a, ctx| {
        let base = a[1].point();
        let s = a[2].number();
        ctx.doc.begin_undo("ScalePositions");
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            let c = ctx.doc.object_bbox(o).center();
            let target = base + (c - base) * s;
            ctx.doc.transform_objects(&[id], &DAffine3::from_translation(target - c));
        }
        Step::Done
    })
}

pub fn remap_cplane() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Options("Target construction plane", vec![opt("Top"), opt("Front"), opt("Right"), opt("Back"), opt("Left"), opt("Bottom")])], |a, ctx| {
        let from = ctx.cplane;
        let to = match a[1].text().as_str() {
            "Front" => Plane::world_xz(),
            "Right" => Plane::world_yz(),
            "Back" => Plane { x_axis: -DVec3::X, y_axis: DVec3::Z, z_axis: DVec3::Y, ..Plane::world_xy() },
            "Left" => Plane { x_axis: -DVec3::Y, y_axis: DVec3::Z, z_axis: -DVec3::X, ..Plane::world_xy() },
            "Bottom" => Plane { x_axis: DVec3::X, y_axis: -DVec3::Y, z_axis: -DVec3::Z, ..Plane::world_xy() },
            _ => Plane::world_xy(),
        };
        let m_from = glam::DMat3::from_cols(from.x_axis, from.y_axis, from.z_axis);
        let m_to = glam::DMat3::from_cols(to.x_axis, to.y_axis, to.z_axis);
        let xf = DAffine3::from_mat3_translation(m_to, to.origin) * DAffine3::from_mat3_translation(m_from, from.origin).inverse();
        ctx.doc.begin_undo("RemapCPlane");
        ctx.doc.transform_objects(&a[0].ids(), &xf);
        Step::Done
    })
}

pub fn array_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to array", ObjFilter::Any, 1), Ask::Point("Base point"), Ask::Pick("Select target surface", ObjFilter::Surface), Ask::Number("Number in U", 4.0), Ask::Number("Number in V", 4.0)], |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[2].ids()[0]) else { return Step::Cancelled };
        let s = &faces[0].surface;
        let base = a[1].point();
        let cp = ctx.cplane;
        let (nu, nv) = (a[3].number().round().max(1.0) as usize, a[4].number().round().max(1.0) as usize);
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let mut out = Vec::new();
        for i in 0..nu {
            for j in 0..nv {
                let u = u0 + (u1 - u0) * (i as f64 + 0.5) / nu as f64;
                let v = v0 + (v1 - v0) * (j as f64 + 0.5) / nv as f64;
                let p = s.point_at(u, v);
                let n = s.normal_at(u, v);
                let xf = DAffine3::from_translation(p) * DAffine3::from_quat(DQuat::from_rotation_arc(cp.z_axis, n)) * DAffine3::from_translation(-base);
                for id in a[0].ids() {
                    if let Some(o) = ctx.doc.get(id) {
                        let mut g = o.geometry.clone();
                        g.transform(&xf);
                        out.push(g);
                    }
                }
            }
        }
        add_all(ctx, "ArraySrf", out)
    })
}

pub fn array_crv_on_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to array", ObjFilter::Any, 1), Ask::Point("Base point"), Ask::Pick("Select path curve on the surface", ObjFilter::Curve), Ask::Pick("Select surface", ObjFilter::Surface), Ask::Number("Number of items", 5.0)], |a, ctx| {
        let (Some(c), Some(faces)) = (curve_of(ctx.doc, a[2].ids()[0]), faces_of(ctx.doc, a[3].ids()[0])) else { return Step::Cancelled };
        let base = a[1].point();
        let cp = ctx.cplane;
        let n = a[4].number().round().max(1.0) as usize;
        let (t0, t1) = c.domain();
        let mut out = Vec::new();
        for k in 0..n {
            let t = t0 + (t1 - t0) * (k as f64 / (n.max(2) - 1) as f64);
            let p = c.point_at(t);
            let (on, nrm) = pull_point(&faces, p);
            let tan = c.tangent_at(t);
            let x = (tan - nrm * tan.dot(nrm)).normalize_or(cp.x_axis);
            let m = glam::DMat3::from_cols(x, nrm.cross(x), nrm);
            let xf = DAffine3::from_mat3_translation(m, on) * DAffine3::from_translation(-base);
            for id in a[0].ids() {
                if let Some(o) = ctx.doc.get(id) {
                    let mut g = o.geometry.clone();
                    g.transform(&xf);
                    out.push(g);
                }
            }
        }
        add_all(ctx, "ArrayCrvOnSrf", out)
    })
}

pub fn tween_surfaces() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first surface", ObjFilter::Surface), Ask::Pick("Select second surface", ObjFilter::Surface), Ask::Number("Number of tween surfaces", 1.0)], |a, ctx| {
        let (Some(f1), Some(f2)) = (faces_of(ctx.doc, a[0].ids()[0]), faces_of(ctx.doc, a[1].ids()[0])) else { return Step::Cancelled };
        let n = 12;
        let grid = |s: &NurbsSurface| -> Vec<Vec<DVec3>> {
            let (u0, u1) = s.domain_u();
            let (v0, v1) = s.domain_v();
            (0..=n).map(|i| (0..=n).map(|j| s.point_at(u0 + (u1 - u0) * i as f64 / n as f64, v0 + (v1 - v0) * j as f64 / n as f64)).collect()).collect()
        };
        let (g1, g2) = (grid(&f1[0].surface), grid(&f2[0].surface));
        let count = a[2].number().round().max(1.0) as usize;
        let mut out = Vec::new();
        for k in 1..=count {
            let f = k as f64 / (count + 1) as f64;
            let g: Vec<Vec<DVec3>> = g1.iter().zip(&g2).map(|(r1, r2)| r1.iter().zip(r2).map(|(p, q)| *p * (1.0 - f) + *q * f).collect()).collect();
            if let Ok(s) = NurbsSurface::interpolate_grid(&g, 3, 3) {
                out.push(Geometry::Surface(s));
            }
        }
        add_all(ctx, "TweenSurfaces", out)
    })
}

pub fn rail_revolve() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select profile curve", ObjFilter::Curve), Ask::Pick("Select rail curve", ObjFilter::Curve), Ask::Point("Start of axis"), Ask::Point("End of axis")], |a, ctx| {
        let (Some(profile), Some(rail)) = (curve_of(ctx.doc, a[0].ids()[0]), curve_of(ctx.doc, a[1].ids()[0])) else { return Step::Cancelled };
        let (o, e) = (a[2].point(), a[3].point());
        let axis = (e - o).normalize_or(DVec3::Z);
        let radial = |p: DVec3| { let d = p - o; d - axis * d.dot(axis) };
        let r0 = radial(rail.start_point()).length().max(1e-9);
        let x0 = radial(rail.start_point()).normalize_or(DVec3::X);
        let n = 48;
        let (t0, t1) = rail.domain();
        let sections: Vec<NurbsCurve> = (0..=n).map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / n as f64;
            let rp = rail.point_at(t);
            let r = radial(rp);
            let ang = { let x = r.normalize_or(x0); let s = x0.cross(x).dot(axis); let c = x0.dot(x); s.atan2(c) };
            let scale = r.length() / r0;
            let xf = DAffine3::from_translation(o) * DAffine3::from_quat(DQuat::from_axis_angle(axis, ang)) * DAffine3::from_translation(-o);
            let mut sec = profile.transformed(&xf);
            for p in sec.control_points.iter_mut() {
                let d = *p - o;
                let along = axis * d.dot(axis);
                let rad = d - along;
                *p = o + along + rad * scale + axis * ((rp - o).dot(axis) - (rail.start_point() - o).dot(axis));
            }
            sec
        }).collect();
        match NurbsSurface::loft(&sections, 3) {
            Ok(s) => add_all(ctx, "RailRevolve", vec![Geometry::Surface(s)]),
            Err(e) => {
                ctx.log(format!("RailRevolve failed: {e}"));
                Step::Cancelled
            }
        }
    })
}

pub fn fold_face() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select planar face to fold", ObjFilter::Polysurface), Ask::Point("Start of fold axis"), Ask::Point("End of fold axis"), Ask::Number("Angle", 30.0)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let fi = face_under(&faces, *p);
        let Some((plane, _)) = planar_loops(&faces[fi]) else {
            ctx.log("FoldFace works on planar faces.");
            return Step::Cancelled;
        };
        let (o, e) = (a[1].point(), a[2].point());
        let axis = (e - o).normalize_or(plane.x_axis);
        let q = DQuat::from_axis_angle(axis, a[3].number().to_radians());
        let tol = faces.iter().fold(tri_geom::BoundingBox::EMPTY, |b, f| b.union(&f.bounding_box())).diagonal() * 1e-6;
        let on_plane = |v: DVec3| (v - plane.origin).dot(plane.z_axis).abs() < tol * 100.0;
        let rotate = |v: DVec3| o + q * (v - o);
        let mut out = Vec::new();
        for (k, f) in faces.iter().enumerate() {
            match planar_loops(f) {
                Some((fp, loops)) => {
                    let new_loops: Vec<Vec<DVec3>> = loops.iter().map(|l| l.iter().map(|v| if k == fi || on_plane(*v) { rotate(*v) } else { *v }).collect()).collect();
                    let poly = &new_loops[0];
                    let mut n = DVec3::ZERO;
                    for i in 0..poly.len() {
                        n += poly[i].cross(poly[(i + 1) % poly.len()]);
                    }
                    let n = n.normalize_or(fp.z_axis);
                    let n = if n.dot(if k == fi { q * fp.z_axis } else { fp.z_axis }) < 0.0 { -n } else { n };
                    let c = poly.iter().fold(DVec3::ZERO, |s, v| s + *v) / poly.len() as f64;
                    match Face::planar(&Plane::from_normal(c, n), new_loops) {
                        Ok(nf) => out.push(nf),
                        Err(_) => out.push(f.clone()),
                    }
                }
                None => out.push(f.clone()),
            }
        }
        ctx.doc.begin_undo("FoldFace");
        ctx.doc.replace_geometry(*id, Geometry::Brep(Brep::new(out)));
        Step::Done
    })
}

pub fn push_pull() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select face to push or pull", ObjFilter::Surface), Ask::Number("Distance (negative pushes in)", 1.0)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let fi = face_under(&faces, *p);
        let d = a[1].number();
        let Some((plane, loops)) = planar_loops(&faces[fi]) else {
            ctx.log("PushPull works on planar faces.");
            return Step::Cancelled;
        };
        let mut poly = loops[0].clone();
        poly.push(poly[0]);
        let Ok(c) = NurbsCurve::polyline(poly) else { return Step::Cancelled };
        let Ok(side) = NurbsSurface::extrude(&c, plane.z_axis * d) else { return Step::Cancelled };
        let Some(cap0) = planar_cap(&c) else { return Step::Cancelled };
        let mut cap1 = cap0.clone();
        cap1.transform(&DAffine3::from_translation(plane.z_axis * d));
        let mut tool = Brep::new(vec![Face::untrimmed(side), cap0, cap1]);
        tool.orient_outward();
        tool.closed = true;
        let Some(base) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        let Some(result) = super::misc4::boolean_geometry(&base, &Geometry::Brep(tool), if d >= 0.0 { BooleanOp::Union } else { BooleanOp::Difference }) else { return Step::Cancelled };
        ctx.doc.begin_undo("PushPull");
        ctx.doc.replace_geometry(*id, result);
        Step::Done
    })
}

/// WireCut: trim surfaces, polysurfaces and solids with a curve extruded along a direction,
/// like cutting foam with a hot wire.
///
/// Prompts follow the reference: cutting curve, objects, then the cut depth points (Enter for
/// an infinite cut) with the options `Direction=CPlaneNormal|View|Custom`, `KeepAll=Yes|No`
/// and `DeleteInput`. Closed solids are cut with the exact Boolean; open surfaces and
/// polysurfaces are trimmed face by face with the projected curve.
#[derive(Default)]
pub struct WireCutCmd {
    curve: Option<ObjectId>,
    targets: Vec<ObjectId>,
    direction: Direction,
    custom_dir: Option<DVec3>,
    keep_all: bool,
    delete_input: bool,
    depth: Vec<DVec3>,
    stage: WireStage,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Direction {
    #[default]
    CPlaneNormal,
    View,
    Custom,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum WireStage {
    #[default]
    Curve,
    Objects,
    Depth,
    CustomFrom,
    CustomTo(u8),
    KeepPieces,
}

impl WireCutCmd {
    fn depth_prompt(&self) -> Prompt {
        let dir = match self.direction { Direction::CPlaneNormal => "CPlaneNormal", Direction::View => "View", Direction::Custom => "Custom" };
        let msg = if self.depth.is_empty() { "Cut depth point. Press Enter for an infinite cut" } else { "Second cut depth point. Press Enter for an infinite cut" };
        Prompt::point(msg).opts([optv("Direction", dir), optv("KeepAll", yesno(self.keep_all)), optv("DeleteInput", yesno(self.delete_input))])
    }

    fn cut_direction(&self, ctx: &CmdCtx) -> DVec3 {
        match self.direction {
            Direction::CPlaneNormal => ctx.cplane.z_axis,
            Direction::View => ctx.view_dir.normalize_or(ctx.cplane.z_axis),
            Direction::Custom => self.custom_dir.unwrap_or(ctx.cplane.z_axis).normalize_or(ctx.cplane.z_axis),
        }
    }

    fn run(&mut self, ctx: &mut CmdCtx) -> Step {
        let Some(cid) = self.curve else { return Step::Cancelled };
        let Some(c) = curve_of(ctx.doc, cid) else { return Step::Cancelled };
        let dir = self.cut_direction(ctx);
        let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
        let bb_all = self.targets.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&o.geometry.bounding_box()));
        let big = bb_all.union(&c.bounding_box()).diagonal().max(1.0) * 4.0;
        // the cutting curve projected on the plane perpendicular to the direction (an extrusion
        // along `dir` of the projection is the same wire surface)
        let centre = bb_all.center();
        let plane = Plane::from_normal(centre, dir);
        let proj = |p: DVec3| p - dir * (p - centre).dot(dir);
        let base = c.transformed(&DAffine3::IDENTITY);
        let flat_pts: Vec<DVec3> = base.span_samples(8).into_iter().map(proj).collect();
        // open curves are closed far away on one side (the side of the curve's own bulge)
        let closed = if c.is_closed() {
            let mut pts = flat_pts.clone();
            if (pts[0] - pts[pts.len() - 1]).length() > 1e-9 {
                pts.push(pts[0]);
            }
            NurbsCurve::polyline(pts).ok()
        } else {
            let (t0, t1) = c.domain();
            let d0 = proj(c.tangent_at(t0) + centre) - centre;
            let d1 = proj(c.tangent_at(t1) + centre) - centre;
            let d0 = d0.normalize_or(plane.x_axis);
            let d1 = d1.normalize_or(plane.x_axis);
            let start_far = flat_pts[0] - d0 * big;
            let end_far = flat_pts[flat_pts.len() - 1] + d1 * big;
            let chord = (end_far - start_far).normalize_or(plane.x_axis);
            let side = dir.cross(chord).normalize_or(plane.y_axis);
            let mut poly = vec![start_far];
            poly.extend(flat_pts.iter().copied());
            poly.push(end_far);
            poly.push(end_far + side * big);
            poly.push(start_far + side * big);
            poly.push(start_far);
            NurbsCurve::polyline(poly).ok()
        };
        let Some(closed) = closed else { return Step::Cancelled };
        // depth range along the direction (infinite when no points were picked)
        let range: Option<(f64, f64)> = match self.depth.len() {
            2 => {
                let (a, b) = ((self.depth[0] - centre).dot(dir), (self.depth[1] - centre).dot(dir));
                Some((a.min(b), a.max(b)))
            }
            1 => {
                let a = (self.depth[0] - centre).dot(dir);
                Some((a.min(-big), a.max(-big)))
            }
            _ => None,
        };
        ctx.doc.begin_undo("WireCut");
        let mut cut_any = false;
        for id in self.targets.clone() {
            let Some(g) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let solid = g.is_solid() && matches!(g, Geometry::Brep(_) | Geometry::Surface(_));
            let mut results: Vec<Geometry> = Vec::new();
            if solid {
                // closed solids: exact Boolean with the extruded cutter
                let (lo, hi) = range.unwrap_or((-big, big));
                let mid = centre + dir * ((lo + hi) * 0.5);
                let tool_plane = Plane { origin: mid, ..plane };
                let tool_curve = closed.transformed(&DAffine3::from_translation(mid - centre));
                let Some(tool) = super::misc4::prism_tool(&tool_curve, &tool_plane, (hi - lo) * 0.5) else { continue };
                let (inside, outside) = (super::misc4::boolean_geometry(&g, &tool, BooleanOp::Intersection), super::misc4::boolean_geometry(&g, &tool, BooleanOp::Difference));
                if let Some(i) = inside.filter(super::misc4::geometry_nonempty) {
                    results.push(i);
                }
                if let Some(o) = outside.filter(super::misc4::geometry_nonempty) {
                    results.push(o);
                }
            } else {
                // open surfaces / polysurfaces: trim every face by the projected cutter and
                // group the pieces by their side of the wire
                let Some(faces) = faces_of(ctx.doc, id) else { continue };
                let mut inside_faces = Vec::new();
                let mut outside_faces = Vec::new();
                let inside_wire = |p: DVec3| -> bool {
                    let q = proj(p);
                    let h = (p - centre).dot(dir);
                    let in_depth = range.is_none_or(|(lo, hi)| h >= lo && h <= hi);
                    in_depth && curve_contains_point(&closed, &plane, q)
                };
                for f in faces {
                    let mut cutters = vec![closed.clone()];
                    if let Some((lo, hi)) = range {
                        // depth limits: cut by the two bounding planes too (their sections)
                        for h in [lo, hi] {
                            let section = Plane::from_normal(centre + dir * h, dir);
                            if let Some(c) = plane_section_curve(&f, &section, tol) {
                                cutters.push(c);
                            }
                        }
                    }
                    let pieces = f.split_by_curves(&cutters, dir, tol);
                    for piece in pieces {
                        let (p, _) = piece.sample();
                        if inside_wire(p) {
                            inside_faces.push(piece);
                        } else {
                            outside_faces.push(piece);
                        }
                    }
                }
                if !inside_faces.is_empty() && !outside_faces.is_empty() {
                    results.push(Geometry::Brep(Brep::new(inside_faces)));
                    results.push(Geometry::Brep(Brep::new(outside_faces)));
                }
            }
            if results.len() >= 2 {
                cut_any = true;
                let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
                ctx.doc.remove(id);
                for r in results {
                    let nid = ctx.doc.add_with(r, attrs.clone());
                    ctx.doc.select(nid, true);
                }
            }
        }
        if !cut_any {
            ctx.log("WireCut: the curve does not cut the objects.");
            return Step::Cancelled;
        }
        if self.delete_input {
            ctx.doc.remove(cid);
        }
        if self.keep_all {
            return Step::Done;
        }
        self.stage = WireStage::KeepPieces;
        Prompt::objects("Select the pieces to delete. Press Enter to keep all", ObjFilter::Any, 0).step()
    }
}

/// Whether a point on the cutter plane lies inside the closed cutter outline.
fn curve_contains_point(closed: &NurbsCurve, plane: &Plane, q: DVec3) -> bool {
    let poly: Vec<glam::DVec2> = closed.control_points.iter().map(|p| { let d = *p - plane.origin; glam::DVec2::new(d.dot(plane.x_axis), d.dot(plane.y_axis)) }).collect();
    let d = q - plane.origin;
    tri_geom::brep::point_in_polygon(glam::DVec2::new(d.dot(plane.x_axis), d.dot(plane.y_axis)), &poly)
}

/// Section of a face by a plane as a polyline curve (for depth-limited wire cuts).
fn plane_section_curve(face: &Face, plane: &Plane, tol: f64) -> Option<NurbsCurve> {
    let m = face.tessellate(12, 12);
    let polys = m.section(plane, tol);
    let longest = polys.into_iter().max_by(|a, b| a.len().cmp(&b.len()))?;
    NurbsCurve::polyline(longest).ok()
}

impl super::Command for WireCutCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.keep_all = ctx.mem_bool("WireCut.KeepAll", true);
        self.direction = match ctx.memory.get("WireCut.Direction").map(|s| s.as_str()) { Some("View") => Direction::View, Some("Custom") => Direction::Custom, _ => Direction::CPlaneNormal };
        Prompt::pick("Select cutting curve", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.stage, input) {
            (WireStage::Curve, Input::Pick(id, _)) => {
                self.curve = Some(id);
                self.stage = WireStage::Objects;
                ctx.doc.clear_selection();
                Prompt::objects("Select objects to cut", ObjFilter::Any, 1).step()
            }
            (WireStage::Objects, Input::Objects(ids)) => {
                self.targets = ids.into_iter().filter(|i| Some(*i) != self.curve).collect();
                if self.targets.is_empty() {
                    return Step::Cancelled;
                }
                self.stage = WireStage::Depth;
                self.depth_prompt().step()
            }
            (WireStage::Depth, Input::Point(p)) => {
                self.depth.push(p);
                if self.depth.len() >= 2 {
                    self.run(ctx)
                } else {
                    self.depth_prompt().step()
                }
            }
            (WireStage::Depth, Input::Empty) => self.run(ctx),
            (WireStage::Depth, Input::Option(o)) => match o.as_str() {
                "Direction" => {
                    self.direction = match self.direction { Direction::CPlaneNormal => Direction::View, Direction::View => Direction::Custom, Direction::Custom => Direction::CPlaneNormal };
                    ctx.mem_set("WireCut.Direction", match self.direction { Direction::CPlaneNormal => "CPlaneNormal", Direction::View => "View", Direction::Custom => "Custom" });
                    if self.direction == Direction::Custom {
                        self.stage = WireStage::CustomFrom;
                        return Prompt::point("Start of cut direction").step();
                    }
                    self.depth_prompt().step()
                }
                "KeepAll" => {
                    self.keep_all = !self.keep_all;
                    ctx.mem_set("WireCut.KeepAll", yesno(self.keep_all));
                    self.depth_prompt().step()
                }
                "DeleteInput" => {
                    self.delete_input = !self.delete_input;
                    self.depth_prompt().step()
                }
                _ => self.depth_prompt().step(),
            },
            (WireStage::CustomFrom, Input::Point(p)) => {
                self.custom_dir = Some(p);
                self.stage = WireStage::CustomTo(0);
                Prompt::point("End of cut direction").step()
            }
            (WireStage::CustomTo(_), Input::Point(p)) => {
                let from = self.custom_dir.unwrap_or(DVec3::ZERO);
                self.custom_dir = Some((p - from).normalize_or(ctx.cplane.z_axis));
                self.stage = WireStage::Depth;
                self.depth_prompt().step()
            }
            (WireStage::KeepPieces, Input::Objects(ids)) => {
                for id in ids {
                    ctx.doc.remove(id);
                }
                Step::Done
            }
            (WireStage::KeepPieces, Input::Empty) => Step::Done,
            (_, Input::Cancel) => Step::Cancelled,
            _ => Step::Cancelled,
        }
    }
}

pub fn boss_rib(rib: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects(if rib { "Select curves to extrude as ribs" } else { "Select closed planar curves to extrude as bosses" }, ObjFilter::Curve, 1), Ask::Pick("Select boundary object", ObjFilter::Any), Ask::Number(if rib { "Rib thickness" } else { "Draft angle" }, if rib { 1.0 } else { 0.0 })], move |a, ctx| {
        let tid = a[1].ids()[0];
        let Some(target) = ctx.doc.get(tid).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        if super::misc4::geometry_mesh(&target).is_none() {
            return Step::Cancelled;
        }
        let diag = target.bounding_box().diagonal().max(1e-3);
        let mut result = target;
        for id in a[0].ids() {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            let plane = curve_plane(&c).unwrap_or(ctx.cplane);
            let tool = if rib {
                // thin slab around the curve, both directions along the plane normal
                let t = a[2].number().abs().max(1e-6);
                let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
                let Ok(o1) = c.offset(&plane, t * 0.5, tol) else { continue };
                let Ok(o2) = c.offset(&plane, -t * 0.5, tol) else { continue };
                let mut pts = o1.tessellate_adaptive(tol * 10.0);
                let mut back = o2.tessellate_adaptive(tol * 10.0);
                back.reverse();
                pts.extend(back);
                pts.push(pts[0]);
                NurbsCurve::polyline(pts).ok().and_then(|c| super::misc4::prism_tool(&c, &Plane { origin: plane.origin - plane.z_axis * diag, ..plane }, diag * 2.0))
            } else {
                let tol = c.bounding_box().diagonal().max(1e-6) * 2e-4;
                // extrude towards the boundary: ray from the centroid along the normal finds the side
                let pts = c.tessellate_adaptive(tol);
                let centroid = pts.iter().fold(DVec3::ZERO, |s, p| s + *p) / pts.len() as f64;
                let probe = super::misc4::geometry_mesh(&result).is_some_and(|m| m.intersect_ray(&Ray::new(centroid + plane.z_axis * 1e-6, plane.z_axis)).is_some());
                let dir = if probe { plane.z_axis } else { -plane.z_axis };
                super::misc4::prism_tool(&c, &Plane { origin: plane.origin + dir * diag, ..plane }, diag)
            };
            let Some(tool) = tool else { continue };
            if let Some(r) = super::misc4::boolean_geometry(&result, &tool, BooleanOp::Union) {
                result = r;
            }
        }
        ctx.doc.begin_undo(if rib { "Rib" } else { "Boss" });
        ctx.doc.replace_geometry(tid, result);
        Step::Done
    })
}

pub fn ribbon_offset() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select closed planar curves", ObjFilter::Curve, 1), Ask::Number("Offset distance", 1.0), Ask::Number("Height", 1.0)], |a, ctx| {
        let (d, h) = (a[1].number(), a[2].number());
        let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            let plane = curve_plane(&c).unwrap_or(ctx.cplane);
            let Ok(o) = c.offset(&plane, d, tol) else { continue };
            let o = o.transformed(&DAffine3::from_translation(plane.z_axis * h));
            if let Ok(s) = NurbsSurface::loft(&[c, o], 1) {
                out.push(Geometry::Surface(s));
            }
        }
        add_all(ctx, "RibbonOffset", out)
    })
}

pub fn extrude_srf_along_crv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select planar surfaces to extrude", ObjFilter::Surface, 1), Ask::Pick("Select path curve", ObjFilter::Curve)], |a, ctx| {
        let Some(rail) = curve_of(ctx.doc, a[1].ids()[0]) else { return Step::Cancelled };
        let mut out = Vec::new();
        for id in a[0].ids() {
            for f in faces_of(ctx.doc, id).unwrap_or_default() {
                let Some((_, loops)) = planar_loops(&f) else { continue };
                let mut poly = loops[0].clone();
                poly.push(poly[0]);
                let Ok(c) = NurbsCurve::polyline(poly) else { continue };
                let sections = super::surface2::sweep_sections_pub(&c, &rail, 24);
                let Ok(side) = NurbsSurface::loft(&sections, 3) else { continue };
                let (Some(a0), Some(a1)) = (planar_cap(&sections[0]), planar_cap(sections.last().unwrap())) else { continue };
                let mut brep = Brep::new(vec![Face::untrimmed(side), a0, a1]);
                brep.orient_outward();
                brep.closed = true;
                out.push(Geometry::Brep(brep));
            }
        }
        add_all(ctx, "ExtrudeSrfAlongCrv", out)
    })
}

pub fn hydrostatics() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select closed objects", ObjFilter::Any, 1), Ask::Number("Waterline height (world Z)", 0.0)], |a, ctx| {
        let z = a[1].number();
        for id in a[0].ids() {
            let Some(m) = solid_mesh(ctx.doc, id) else { continue };
            let bb = m.bounding_box();
            let diag = bb.diagonal().max(1e-3) * 2.0;
            let c = bb.center();
            let plane = Plane::from_normal(DVec3::new(c.x, c.y, z - diag), DVec3::Z);
            let poly = vec![plane.point_at(-diag, -diag), plane.point_at(diag, -diag), plane.point_at(diag, diag), plane.point_at(-diag, diag)];
            let Some(water) = prism_mesh(&poly, &plane, diag) else { continue };
            let below = mesh_boolean(&m, &water, BooleanOp::Intersection);
            let (vol, centroid) = Document::volume_of(&Geometry::Mesh(below.clone())).unwrap_or((0.0, c));
            let wl = Mesh::section(&m, &Plane::from_normal(DVec3::new(c.x, c.y, z), DVec3::Z), diag * 1e-6);
            let area: f64 = wl.iter().filter_map(|p| NurbsCurve::polyline(p.clone()).ok()).filter_map(|cv| Document::area_of(&Geometry::Curve(cv))).sum();
            ctx.log(format!("Object #{id}: displaced volume {} , centre of buoyancy ({}, {}, {}), waterline area {}", fmt_num(vol), fmt_num(centroid.x), fmt_num(centroid.y), fmt_num(centroid.z), fmt_num(area)));
        }
        Step::Done
    })
}

pub fn cut_volume() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select solid", ObjFilter::Any), Ask::Point("First corner of box"), Ask::Point("Other corner of box"), Ask::PointOrNumber("Box height", 10.0)], |a, ctx| {
        let Some(m) = solid_mesh(ctx.doc, a[0].ids()[0]) else { return Step::Cancelled };
        let (p, q) = (a[1].point(), a[2].point());
        let h = match &a[3] { Ans::Number(n) => *n, Ans::Point(r) => (*r - q).z, _ => 10.0 };
        let min = p.min(q);
        let max = DVec3::new(p.max(q).x, p.max(q).y, p.max(q).z + h);
        let bx = Mesh::cuboid(min.min(max), min.max(max));
        let inter = mesh_boolean(&m, &bx, BooleanOp::Intersection);
        let v = Document::volume_of(&Geometry::Mesh(inter)).map(|v| v.0).unwrap_or(0.0);
        ctx.log(format!("Volume inside the box = {}", fmt_num(v)));
        Step::Done
    })
}

pub fn dim_crease_angle() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first planar face", ObjFilter::Surface), Ask::Pick("Select second planar face", ObjFilter::Surface), Ask::Point("Text location")], |a, ctx| {
        let (Ans::Pick(ida, pa), Ans::Pick(idb, pb)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(fa), Some(fb)) = (faces_of(ctx.doc, *ida), faces_of(ctx.doc, *idb)) else { return Step::Cancelled };
        let (_, na) = pull_point(&fa[face_under(&fa, *pa)..=face_under(&fa, *pa)], *pa);
        let (_, nb) = pull_point(&fb[face_under(&fb, *pb)..=face_under(&fb, *pb)], *pb);
        let ang = 180.0 - na.dot(nb).clamp(-1.0, 1.0).acos().to_degrees();
        let h = ctx.mem_num("Dim.TextHeight", 1.0);
        let curves = super::annotate::text_curves(&format!("{}%%d", fmt_num(ang)), &ctx.cplane, a[2].point(), h, 0.0);
        ctx.log(format!("Crease angle = {} degrees", fmt_num(ang)));
        add_all(ctx, "DimCreaseAngle", curves.into_iter().map(Geometry::Curve).collect())
    })
}

pub fn edge_continuity() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first surface near the edge", ObjFilter::Surface), Ask::Pick("Select second surface near the edge", ObjFilter::Surface)], |a, ctx| {
        let (Ans::Pick(ida, pa), Ans::Pick(idb, pb)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(fa), Some(fb)) = (faces_of(ctx.doc, *ida), faces_of(ctx.doc, *idb)) else { return Step::Cancelled };
        let f1 = &fa[face_under(&fa, *pa)];
        let tol = f1.bounding_box().diagonal().max(1e-6) * 1e-3;
        let edges = f1.edge_curves(tol);
        let Some(edge) = edges.into_iter().min_by(|x, y| (x.point_at(x.closest_parameter(*pa)) - *pa).length().partial_cmp(&(y.point_at(y.closest_parameter(*pa)) - *pa).length()).unwrap()) else { return Step::Cancelled };
        let (t0, t1) = edge.domain();
        let (mut dmax, mut amax) = (0.0f64, 0.0f64);
        let mut hairs = Vec::new();
        for i in 0..=20 {
            let p = edge.point_at(t0 + (t1 - t0) * i as f64 / 20.0);
            let (q, n2) = pull_point(&fb, p);
            let (_, n1) = pull_point(&fa, p);
            dmax = dmax.max((q - p).length());
            amax = amax.max(n1.dot(n2).abs().clamp(-1.0, 1.0).acos().to_degrees());
            if let Ok(l) = NurbsCurve::line(p, q + n2 * ((q - p).length() * 10.0 + 1e-9)) {
                hairs.push(Geometry::Curve(l));
            }
        }
        let _ = pb;
        ctx.log(format!("Maximum distance {} , maximum normal angle {} degrees", fmt_num(dmax), fmt_num(amax)));
        add_all(ctx, "EdgeContinuity", hairs)
    })
}

pub fn point_cloud_section(contour: bool) -> SeqCmd {
    let asks = if contour { vec![Ask::Objects("Select points", ObjFilter::Point, 3), Ask::Point("Contour base point"), Ask::Point("Contour direction"), Ask::Number("Distance between contours", 1.0)] } else { vec![Ask::Objects("Select points", ObjFilter::Point, 3), Ask::Point("First point on section plane"), Ask::Point("Second point on section plane")] };
    SeqCmd::new(asks, move |a, ctx| {
        let pts: Vec<DVec3> = a[0].ids().iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p) } else { None }).collect();
        let cp = ctx.cplane;
        let mut out = Vec::new();
        let band = |plane: &Plane, tol: f64| -> Option<NurbsCurve> {
            let mut sel: Vec<DVec3> = pts.iter().copied().filter(|p| (*p - plane.origin).dot(plane.z_axis).abs() <= tol).collect();
            if sel.len() < 2 {
                return None;
            }
            // order along the plane's x axis
            sel.sort_by(|p, q| (*p - plane.origin).dot(plane.x_axis).partial_cmp(&(*q - plane.origin).dot(plane.x_axis)).unwrap());
            NurbsCurve::polyline(sel).ok()
        };
        let bb = tri_geom::BoundingBox::from_points(&pts);
        let tol = bb.diagonal().max(1e-6) * 0.02;
        if contour {
            let (base, dirp) = (a[1].point(), a[2].point());
            let dir = (dirp - base).normalize_or(cp.z_axis);
            let step = a[3].number().abs().max(1e-6);
            let (lo, hi) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), p| { let t = (*p - base).dot(dir); (l.min(t), h.max(t)) });
            let mut t = lo;
            while t <= hi {
                if let Some(c) = band(&Plane::from_normal(base + dir * t, dir), tol) {
                    out.push(Geometry::Curve(c));
                }
                t += step;
            }
        } else {
            let (p, q) = (a[1].point(), a[2].point());
            let plane = Plane::from_frame(p, q - p, cp.z_axis);
            let plane = Plane { z_axis: plane.y_axis, y_axis: plane.z_axis, ..plane };
            if let Some(c) = band(&plane, tol) {
                out.push(Geometry::Curve(c));
            }
        }
        add_all(ctx, if contour { "PointCloudContour" } else { "PointCloudSection" }, out)
    })
}

pub fn sphere_tangent_three() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first planar surface", ObjFilter::Surface), Ask::Pick("Select second planar surface", ObjFilter::Surface), Ask::Pick("Select third planar surface", ObjFilter::Surface), Ask::Number("Radius", 1.0)], |a, ctx| {
        let mut planes = Vec::new();
        for k in 0..3 {
            let Ans::Pick(id, p) = &a[k] else { return Step::Cancelled };
            let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
            let Some((plane, _)) = planar_loops(&faces[face_under(&faces, *p)]) else {
                ctx.log("The surfaces must be planar.");
                return Step::Cancelled;
            };
            // the sphere lies on the side of the pick point... the pick is on the surface; use the normal side facing the other picks
            planes.push((plane, *p));
        }
        let r = a[3].number().abs();
        let centroid = (planes[0].1 + planes[1].1 + planes[2].1) / 3.0;
        let m = glam::DMat3::from_cols(planes[0].0.z_axis, planes[1].0.z_axis, planes[2].0.z_axis).transpose();
        if m.determinant().abs() < 1e-9 {
            ctx.log("The planes do not meet at a point.");
            return Step::Cancelled;
        }
        let rhs = DVec3::new(
            planes[0].0.z_axis.dot(planes[0].0.origin) + r * (centroid - planes[0].0.origin).dot(planes[0].0.z_axis).signum(),
            planes[1].0.z_axis.dot(planes[1].0.origin) + r * (centroid - planes[1].0.origin).dot(planes[1].0.z_axis).signum(),
            planes[2].0.z_axis.dot(planes[2].0.origin) + r * (centroid - planes[2].0.origin).dot(planes[2].0.z_axis).signum(),
        );
        let c = m.inverse() * rhs;
        match NurbsSurface::sphere(c, r) {
            Ok(s) => add_all(ctx, "SphereTangentToThreeSurfaces", vec![Geometry::Surface(s)]),
            Err(_) => Step::Cancelled,
        }
    })
}

pub fn revolved_hole() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select profile curve", ObjFilter::Curve), Ask::Point("Start of axis"), Ask::Point("End of axis"), Ask::Pick("Select object to cut", ObjFilter::Any)], |a, ctx| {
        let Some(profile) = curve_of(ctx.doc, a[0].ids()[0]) else { return Step::Cancelled };
        let (o, e) = (a[1].point(), a[2].point());
        let Ok(srf) = NurbsSurface::revolve(&profile, o, (e - o).normalize_or(DVec3::Z), std::f64::consts::TAU) else { return Step::Cancelled };
        let tid = a[3].ids()[0];
        let Some(target) = ctx.doc.get(tid).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        // the revolved surface plus planar caps at the profile ends (when they are off the axis)
        let axis = (e - o).normalize_or(DVec3::Z);
        let mut faces = vec![Face::untrimmed(srf.clone())];
        for p in [profile.start_point(), profile.end_point()] {
            let r = { let d = p - o; (d - axis * d.dot(axis)).length() };
            if r > 1e-9 {
                let centre = o + axis * (p - o).dot(axis);
                if let Ok(c) = NurbsCurve::circle(&Plane::from_normal(centre, axis), r) {
                    if let Some(cap) = planar_cap(&c) {
                        faces.push(cap);
                    }
                }
            }
        }
        let mut tool = Brep::new(faces);
        tool.orient_outward();
        if !tool.closed {
            ctx.log("RevolvedHole: the profile must start and end on the axis or give a closed cutter.");
            return Step::Cancelled;
        }
        let tool = Geometry::Brep(tool);
        let Some(result) = super::misc4::boolean_geometry(&target, &tool, BooleanOp::Difference) else { return Step::Cancelled };
        ctx.doc.begin_undo("RevolvedHole");
        ctx.doc.replace_geometry(tid, result);
        Step::Done
    })
}

pub fn boolean_two_objects() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first object", ObjFilter::Any), Ask::Pick("Select second object", ObjFilter::Any), Ask::Options("Result", vec![opt("Union"), opt("Difference"), opt("Intersection"), opt("ReverseDifference")])], |a, ctx| {
        let (Some(g1), Some(g2)) = (ctx.doc.get(a[0].ids()[0]).map(|o| o.geometry.clone()), ctx.doc.get(a[1].ids()[0]).map(|o| o.geometry.clone())) else { return Step::Cancelled };
        let r = match a[2].text().as_str() {
            "Difference" => super::misc4::boolean_geometry(&g1, &g2, BooleanOp::Difference),
            "Intersection" => super::misc4::boolean_geometry(&g1, &g2, BooleanOp::Intersection),
            "ReverseDifference" => super::misc4::boolean_geometry(&g2, &g1, BooleanOp::Difference),
            _ => super::misc4::boolean_geometry(&g1, &g2, BooleanOp::Union),
        };
        let Some(r) = r else {
            ctx.log("Boolean2Objects failed.");
            return Step::Cancelled;
        };
        ctx.doc.begin_undo("Boolean2Objects");
        ctx.doc.remove(a[0].ids()[0]);
        ctx.doc.remove(a[1].ids()[0]);
        add_all(ctx, "Boolean2Objects", vec![r])
    })
}

pub fn clash() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select first set of objects", ObjFilter::Any, 1), Ask::Objects("Select second set of objects", ObjFilter::Any, 1), Ask::Number("Clearance distance", 0.0)], |a, ctx| {
        let clearance = a[2].number().abs();
        let set_a: Vec<(ObjectId, Mesh)> = a[0].ids().iter().filter_map(|id| solid_mesh(ctx.doc, *id).map(|m| (*id, m))).collect();
        let set_b: Vec<(ObjectId, Mesh)> = a[1].ids().iter().filter_map(|id| solid_mesh(ctx.doc, *id).map(|m| (*id, m))).collect();
        let mut clashes = Vec::new();
        for (ia, ma) in &set_a {
            for (ib, mb) in &set_b {
                if ia == ib {
                    continue;
                }
                let hit = !mesh_ops::mesh_intersection(ma, mb).is_empty() || mb.vertices.iter().any(|v| mesh_ops::contains_point(ma, *v)) || ma.vertices.iter().any(|v| mesh_ops::contains_point(mb, *v)) || (clearance > 0.0 && ma.vertices.iter().any(|v| mb.vertices.iter().any(|w| (*v - *w).length() < clearance)));
                if hit {
                    clashes.push((*ia, *ib));
                }
            }
        }
        if clashes.is_empty() {
            ctx.log("No clashes found.");
        }
        for (x, y) in &clashes {
            ctx.log(format!("Clash: #{x} with #{y}"));
            ctx.doc.select(*x, true);
            ctx.doc.select(*y, true);
        }
        Step::Done
    })
}

pub fn extract_piped_curve() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select pipes (surfaces made by Pipe)", ObjFilter::Surface, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            for f in faces_of(ctx.doc, id).unwrap_or_default() {
                let s = &f.surface;
                // centre line: average of the u isocurve rows
                let (v0, v1) = s.domain_v();
                let pts: Vec<DVec3> = (0..=24).map(|i| {
                    let v = v0 + (v1 - v0) * i as f64 / 24.0;
                    let c = s.isocurve_v(v);
                    let (t0, t1) = c.domain();
                    (0..16).map(|k| c.point_at(t0 + (t1 - t0) * k as f64 / 16.0)).fold(DVec3::ZERO, |acc, p| acc + p) / 16.0
                }).collect();
                if let Ok(c) = NurbsCurve::interpolate(&pts, 3) {
                    out.push(Geometry::Curve(c));
                }
            }
        }
        add_all(ctx, "ExtractPipedCurve", out)
    })
}

pub fn draft_angle_point() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface", ObjFilter::Surface), Ask::Number("Draft angle (degrees)", 10.0)], |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[0].ids()[0]) else { return Step::Cancelled };
        let up = ctx.cplane.z_axis;
        let target = a[1].number().to_radians();
        let s = &faces[0].surface;
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let n = 40;
        let mut out = Vec::new();
        for i in 0..=n {
            let u = u0 + (u1 - u0) * i as f64 / n as f64;
            let mut prev: Option<(f64, DVec3)> = None;
            for j in 0..=n {
                let v = v0 + (v1 - v0) * j as f64 / n as f64;
                let p = s.point_at(u, v);
                let ang = std::f64::consts::FRAC_PI_2 - s.normal_at(u, v).dot(up).clamp(-1.0, 1.0).acos();
                let d = ang.abs() - target;
                if let Some((pd, pp)) = prev {
                    if (pd < 0.0) != (d < 0.0) {
                        out.push(Geometry::Point(pp + (p - pp) * (pd / (pd - d))));
                    }
                }
                prev = Some((d, p));
            }
        }
        if out.is_empty() {
            ctx.log("No draft angle break found.");
            return Step::Done;
        }
        add_all(ctx, "DraftAnglePoint", out)
    })
}
