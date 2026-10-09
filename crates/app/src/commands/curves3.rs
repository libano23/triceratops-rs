//! Real implementations of the remaining curve commands.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{curve_ops::arc_center, curve_plane, NurbsCurve, Plane};

use super::misc6::{pull_point, pulled_curve};
use super::simple::{add_all, Ans, Ask, SeqCmd};
use super::surface_edit::{face_under, faces_of};
use super::{fmt_num, opt, optv, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn curve_of(doc: &Document, id: ObjectId) -> Option<NurbsCurve> {
    doc.get(id).and_then(|o| o.geometry.as_curve().cloned())
}

/// Arc starting at `a` with tangent `t`, ending at `b` (None when collinear).
pub fn arc_from_tangent(a: DVec3, t: DVec3, b: DVec3) -> Option<NurbsCurve> {
    let d = b - a;
    let normal = t.cross(d);
    if normal.length() < 1e-12 {
        return NurbsCurve::line(a, b).ok();
    }
    let normal = normal.normalize();
    let n = normal.cross(t).normalize();
    let r = d.length_squared() / (2.0 * d.dot(n));
    let center = a + n * r;
    let x = (a - center).normalize();
    let y = normal.cross(x).normalize();
    let plane = Plane { origin: center, x_axis: x, y_axis: y, z_axis: normal };
    let (u, v, _) = plane.closest_parameters(b);
    let mut ang = v.atan2(u);
    if ang < 0.0 {
        ang += std::f64::consts::TAU;
    }
    let mut arc = NurbsCurve::arc(&plane, r.abs(), 0.0, ang).ok()?;
    if (arc.start_point() - a).length() > 1e-9 {
        arc.reverse();
    }
    Some(arc)
}

/// Biarc between (a, ta) and (b, tb) with equal tangent lengths.
pub fn biarc(a: DVec3, ta: DVec3, b: DVec3, tb: DVec3) -> Option<NurbsCurve> {
    let v = b - a;
    let s = ta + tb;
    let qa = s.length_squared() - 4.0;
    let qb = -2.0 * v.dot(s);
    let qc = v.length_squared();
    let d = if qa.abs() < 1e-12 { -qc / qb } else {
        let disc = qb * qb - 4.0 * qa * qc;
        if disc < 0.0 {
            return None;
        }
        let r1 = (-qb + disc.sqrt()) / (2.0 * qa);
        let r2 = (-qb - disc.sqrt()) / (2.0 * qa);
        if r1 > 0.0 && r2 > 0.0 { r1.min(r2) } else { r1.max(r2) }
    };
    if !(d > 0.0) {
        return None;
    }
    let j = (a + ta * d + b - tb * d) * 0.5;
    let a1 = arc_from_tangent(a, ta, j)?;
    let mut a2 = arc_from_tangent(b, -tb, j)?;
    a2.reverse();
    a1.join_any(&a2).ok()
}

/// End of a curve nearest to a pick: (point, outward tangent, at_end).
fn curve_end(c: &NurbsCurve, p: DVec3) -> (DVec3, DVec3, bool) {
    let (t0, t1) = c.domain();
    if (c.start_point() - p).length() <= (c.end_point() - p).length() {
        (c.start_point(), -c.tangent_at(t0), false)
    } else {
        (c.end_point(), c.tangent_at(t1), true)
    }
}

pub fn arc_blend() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first curve near the end to blend", ObjFilter::Curve), Ask::Pick("Select second curve near the end to blend", ObjFilter::Curve)], |a, ctx| {
        let (Ans::Pick(i1, p1), Ans::Pick(i2, p2)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(c1), Some(c2)) = (curve_of(ctx.doc, *i1), curve_of(ctx.doc, *i2)) else { return Step::Cancelled };
        let (e1, t1, _) = curve_end(&c1, *p1);
        let (e2, t2, _) = curve_end(&c2, *p2);
        match biarc(e1, t1, e2, -t2) {
            Some(c) => add_all(ctx, "ArcBlend", vec![Geometry::Curve(c)]),
            None => {
                ctx.log("No biarc blend exists for these ends.");
                Step::Cancelled
            }
        }
    })
}

pub fn catenary() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Start of catenary"), Ask::Point("End of catenary"), Ask::Number("Sag", 2.0)], |a, ctx| {
        let (p, q) = (a[0].point(), a[1].point());
        let sag = a[2].number().abs().max(1e-6);
        let cp = ctx.cplane;
        let span = (q - p).length();
        if span < 1e-9 {
            return Step::Cancelled;
        }
        // solve sag = a (cosh(span/(2a)) - 1) for a by bisection
        let f = |aa: f64| aa * ((span / (2.0 * aa)).cosh() - 1.0) - sag;
        let (mut lo, mut hi) = (1e-6, span * 1e4);
        for _ in 0..200 {
            let mid = 0.5 * (lo + hi);
            if f(mid) > 0.0 { lo = mid } else { hi = mid }
        }
        let aa = 0.5 * (lo + hi);
        let down = -cp.y_axis;
        let dir = (q - p) / span;
        let pts: Vec<DVec3> = (0..=32).map(|i| {
            let x = span * i as f64 / 32.0;
            let y = aa * ((x - span / 2.0) / aa).cosh() - aa * (span / (2.0 * aa)).cosh();
            p + dir * x - down * y
        }).collect();
        match NurbsCurve::interpolate(&pts, 3) {
            Ok(c) => add_all(ctx, "Catenary", vec![Geometry::Curve(c)]),
            Err(_) => Step::Cancelled,
        }
    })
}

pub fn hyperbola() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Center of hyperbola"), Ask::Point("Vertex"), Ask::Point("End point")], |a, ctx| {
        let (c, v, e) = (a[0].point(), a[1].point(), a[2].point());
        let aa = (v - c).length();
        if aa < 1e-9 {
            return Step::Cancelled;
        }
        let x = (v - c) / aa;
        let n = ctx.cplane.z_axis;
        let y = n.cross(x).normalize_or(ctx.cplane.y_axis);
        let (xe, ye) = ((e - c).dot(x), (e - c).dot(y));
        if xe.abs() <= aa {
            ctx.log("The end point must lie beyond the vertex.");
            return Step::Cancelled;
        }
        let bb = (ye * ye / (xe * xe / (aa * aa) - 1.0)).sqrt().max(1e-9);
        let tmax = (xe / aa).acosh();
        let pts: Vec<DVec3> = (0..=32).map(|i| {
            let t = -tmax + 2.0 * tmax * i as f64 / 32.0;
            c + x * (aa * t.cosh()) + y * (bb * t.sinh())
        }).collect();
        match NurbsCurve::interpolate(&pts, 3) {
            Ok(cv) => add_all(ctx, "Hyperbola", vec![Geometry::Curve(cv)]),
            Err(_) => Step::Cancelled,
        }
    })
}

pub fn parabola_3pt() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("First point on parabola"), Ask::Point("Second point"), Ask::Point("Third point")], |a, ctx| {
        let cp = ctx.cplane;
        let uv: Vec<(f64, f64)> = (0..3).map(|i| { let (u, v, _) = cp.closest_parameters(a[i].point()); (u, v) }).collect();
        let (x0, y0) = uv[0];
        let (x1, y1) = uv[1];
        let (x2, y2) = uv[2];
        let den = (x0 - x1) * (x0 - x2) * (x1 - x2);
        if den.abs() < 1e-12 {
            ctx.log("The points must have distinct x coordinates in the construction plane.");
            return Step::Cancelled;
        }
        let qa = (x2 * (y1 - y0) + x1 * (y0 - y2) + x0 * (y2 - y1)) / den;
        let qb = (x2 * x2 * (y0 - y1) + x1 * x1 * (y2 - y0) + x0 * x0 * (y1 - y2)) / den;
        let qc = (x1 * x2 * (x1 - x2) * y0 + x2 * x0 * (x2 - x0) * y1 + x0 * x1 * (x0 - x1) * y2) / den;
        let f = |x: f64| qa * x * x + qb * x + qc;
        let (xa, xb) = (x0.min(x1).min(x2), x0.max(x1).max(x2));
        let xm = 0.5 * (xa + xb);
        let p0 = cp.point_at(xa, f(xa));
        let p2 = cp.point_at(xb, f(xb));
        let p1 = cp.point_at(xm, f(xa) + (2.0 * qa * xa + qb) * (xm - xa));
        match NurbsCurve::from_control_points(2, vec![p0, p1, p2]) {
            Ok(c) => add_all(ctx, "Parabola3Pt", vec![Geometry::Curve(c)]),
            Err(_) => Step::Cancelled,
        }
    })
}

pub fn crv_2_view() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first planar curve", ObjFilter::Curve), Ask::Pick("Select second planar curve", ObjFilter::Curve)], |a, ctx| {
        let (Some(c1), Some(c2)) = (curve_of(ctx.doc, a[0].ids()[0]), curve_of(ctx.doc, a[1].ids()[0])) else { return Step::Cancelled };
        let (Some(p1), Some(p2)) = (curve_plane(&c1), curve_plane(&c2)) else {
            ctx.log("Both curves must be planar.");
            return Step::Cancelled;
        };
        // shared direction = intersection of the two planes
        let shared = p1.z_axis.cross(p2.z_axis);
        if shared.length() < 1e-6 {
            ctx.log("The curves must lie in different planes.");
            return Step::Cancelled;
        }
        let shared = shared.normalize();
        let n = 64;
        let (a0, a1) = c1.domain();
        let (b0, b1) = c2.domain();
        let pts: Vec<DVec3> = (0..=n).map(|i| {
            let t = i as f64 / n as f64;
            let q1 = c1.point_at(a0 + (a1 - a0) * t);
            let q2 = c2.point_at(b0 + (b1 - b0) * t);
            // component along the shared direction from the first curve, the rest from each plane
            let s = q1.dot(shared);
            let off1 = q1 - shared * s;
            let off2 = q2 - shared * q2.dot(shared);
            // combine: take off1's component perpendicular to plane2's normal direction and off2's along it
            let n2 = p2.z_axis;
            let n1 = p1.z_axis;
            let comp2 = off2.dot(n1) * n1; // what curve 1 cannot see
            let comp1 = off1 - n1 * off1.dot(n1);
            let _ = n2;
            shared * s + comp1 + comp2
        }).collect();
        match NurbsCurve::interpolate(&pts, 3) {
            Ok(c) => add_all(ctx, "Crv2View", vec![Geometry::Curve(c)]),
            Err(_) => Step::Cancelled,
        }
    })
}

/// Points where a curve crosses a plane.
pub fn plane_curve_points(c: &NurbsCurve, plane: &Plane) -> Vec<DVec3> {
    let pts = c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3);
    let mut out = Vec::new();
    for w in pts.windows(2) {
        let da = (w[0] - plane.origin).dot(plane.z_axis);
        let db = (w[1] - plane.origin).dot(plane.z_axis);
        if (da <= 0.0) != (db <= 0.0) && (da - db).abs() > 1e-18 {
            out.push(w[0] + (w[1] - w[0]) * (da / (da - db)));
        }
    }
    out
}

pub fn csec() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select profile curves", ObjFilter::Curve, 2), Ask::Point("Start of section line"), Ask::Point("End of section line")], |a, ctx| {
        let cp = ctx.cplane;
        let (p, q) = (a[1].point(), a[2].point());
        let plane = Plane::from_frame(p, q - p, cp.z_axis);
        let mut pts = Vec::new();
        for id in a[0].ids() {
            if let Some(c) = curve_of(ctx.doc, id) {
                pts.extend(plane_curve_points(&c, &Plane { z_axis: plane.y_axis, ..plane }));
            }
        }
        if pts.len() < 2 {
            ctx.log("The section line must cross at least two profiles.");
            return Step::Cancelled;
        }
        pts.sort_by(|x, y| (*x - p).dot(q - p).partial_cmp(&(*y - p).dot(q - p)).unwrap());
        match NurbsCurve::interpolate(&pts, 3.min(pts.len() - 1)) {
            Ok(c) => add_all(ctx, "CSec", vec![Geometry::Curve(c)]),
            Err(_) => Step::Cancelled,
        }
    })
}

pub fn insert_line_into_crv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve", ObjFilter::Curve), Ask::Point("Start of line"), Ask::Point("End of line")], |a, ctx| {
        let id = a[0].ids()[0];
        let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
        let mut t0 = c.closest_parameter(a[1].point());
        let mut t1 = c.closest_parameter(a[2].point());
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        let parts = c.split_at_many(&[t0, t1]);
        if parts.len() < 2 {
            return Step::Cancelled;
        }
        let (first, last) = (parts.first().unwrap().clone(), parts.last().unwrap().clone());
        let Ok(line) = NurbsCurve::line(first.end_point(), last.start_point()) else { return Step::Cancelled };
        let joined = first.join_any(&line).and_then(|j| j.join_any(&last));
        ctx.doc.begin_undo("InsertLineIntoCrv");
        match joined {
            Ok(j) => {
                ctx.doc.replace_geometry(id, Geometry::Curve(j));
            }
            Err(_) => {
                ctx.doc.remove(id);
                for g in [first, line, last] {
                    ctx.doc.add(Geometry::Curve(g));
                }
            }
        }
        Step::Done
    })
}

pub struct ContinueCurveCmd {
    interp: bool,
    id: Option<ObjectId>,
    at_end: bool,
    pts: Vec<DVec3>,
}

impl ContinueCurveCmd {
    pub fn new(interp: bool) -> Self {
        Self { interp, id: None, at_end: true, pts: Vec::new() }
    }
    fn build(&self, doc: &Document) -> Option<NurbsCurve> {
        let mut c = curve_of(doc, self.id?)?;
        if !self.at_end {
            c.reverse();
        }
        if self.interp {
            let (t0, t1) = c.domain();
            let mut pts: Vec<DVec3> = (0..=16).map(|i| c.point_at(t0 + (t1 - t0) * i as f64 / 16.0)).collect();
            pts.extend(self.pts.iter().copied());
            NurbsCurve::interpolate(&pts, 3).ok()
        } else {
            let mut cps = c.control_points.clone();
            cps.extend(self.pts.iter().copied());
            NurbsCurve::from_control_points(c.degree.max(1), cps).ok()
        }
    }
}

impl Command for ContinueCurveCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select curve near the end to continue", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                self.id = Some(id);
                self.at_end = (c.end_point() - p).length() <= (c.start_point() - p).length();
                Prompt::point("Next point. Press Enter when done").step()
            }
            Input::Point(p) => {
                self.pts.push(p);
                Prompt::point("Next point. Press Enter when done").opts([opt("Undo")]).step()
            }
            Input::Option(_) => {
                self.pts.pop();
                Prompt::point("Next point. Press Enter when done").step()
            }
            Input::Empty => {
                if self.pts.is_empty() {
                    return Step::Cancelled;
                }
                let Some(id) = self.id else { return Step::Cancelled };
                match self.build(ctx.doc) {
                    Some(c) => {
                        ctx.doc.begin_undo(if self.interp { "ContinueInterpCrv" } else { "ContinueCurve" });
                        ctx.doc.replace_geometry(id, Geometry::Curve(c));
                        Step::Done
                    }
                    None => Step::Cancelled,
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let mut tmp = ContinueCurveCmd { interp: self.interp, id: self.id, at_end: self.at_end, pts: self.pts.clone() };
        if let Some(c) = cursor {
            tmp.pts.push(c);
        }
        tmp.build(doc).map(|c| vec![Preview::Curve(c)]).unwrap_or_default()
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

pub fn curve_through_polyline() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select polylines", ObjFilter::Curve, 1), Ask::Options("Curve type", vec![opt("Interpolated"), opt("ControlPoints")])], |a, ctx| {
        let interp = a[1].text() != "ControlPoints";
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            let pts = c.control_points.clone();
            let r = if interp { NurbsCurve::interpolate(&pts, 3) } else { NurbsCurve::from_control_points(3.min(pts.len() - 1), pts) };
            if let Ok(nc) = r {
                out.push(Geometry::Curve(nc));
            }
        }
        add_all(ctx, "CurveThroughPolyline", out)
    })
}

pub fn curve_through_srf_control_pt() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface", ObjFilter::Surface), Ask::Options("Direction", vec![opt("U"), opt("V"), opt("Both")])], |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[0].ids()[0]) else { return Step::Cancelled };
        let dir = a[1].text();
        let mut out = Vec::new();
        for f in &faces {
            let s = &f.surface;
            if dir != "V" {
                for i in 0..s.count_u {
                    let row: Vec<DVec3> = (0..s.count_v).map(|j| s.control_points[i * s.count_v + j]).collect();
                    if let Ok(c) = NurbsCurve::interpolate(&row, 3.min(row.len() - 1)) {
                        out.push(Geometry::Curve(c));
                    }
                }
            }
            if dir != "U" {
                for j in 0..s.count_v {
                    let col: Vec<DVec3> = (0..s.count_u).map(|i| s.control_points[i * s.count_v + j]).collect();
                    if let Ok(c) = NurbsCurve::interpolate(&col, 3.min(col.len() - 1)) {
                        out.push(Geometry::Curve(c));
                    }
                }
            }
        }
        add_all(ctx, "CurveThroughSrfControlPt", out)
    })
}

pub fn extend_crv_on_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve on surface to extend", ObjFilter::Curve), Ask::Pick("Select surface", ObjFilter::Surface)], |a, ctx| {
        let id = a[0].ids()[0];
        let (Some(c), Some(faces)) = (curve_of(ctx.doc, id), faces_of(ctx.doc, a[1].ids()[0])) else { return Step::Cancelled };
        let diag = faces.iter().fold(tri_geom::BoundingBox::EMPTY, |b, f| b.union(&f.bounding_box())).diagonal().max(1.0);
        let ext = c.extend_by(true, diag).and_then(|e| e.extend_by(false, diag)).unwrap_or(c.clone());
        let Some(pulled) = pulled_curve(&ext, &faces) else { return Step::Cancelled };
        // keep the part whose samples are inside the face
        let (t0, t1) = pulled.domain();
        let n = 200;
        let inside: Vec<bool> = (0..=n).map(|i| {
            let p = pulled.point_at(t0 + (t1 - t0) * i as f64 / n as f64);
            let fi = face_under(&faces, p);
            let (u, v) = faces[fi].surface.closest_uv(p);
            let (u0, u1) = faces[fi].surface.domain_u();
            let (v0, v1) = faces[fi].surface.domain_v();
            let eps = 1e-6;
            u > u0 - eps && u < u1 + eps && v > v0 - eps && v < v1 + eps && faces[fi].contains_uv(glam::DVec2::new(u, v))
        }).collect();
        let mid = n / 2;
        let (mut lo, mut hi) = (mid, mid);
        while lo > 0 && inside[lo - 1] {
            lo -= 1;
        }
        while hi < n && inside[hi + 1] {
            hi += 1;
        }
        let ta = t0 + (t1 - t0) * lo as f64 / n as f64;
        let tb = t0 + (t1 - t0) * hi as f64 / n as f64;
        let parts = pulled.split_at_many(&[ta, tb]);
        let piece = if parts.len() == 3 { parts[1].clone() } else if parts.len() == 2 { if lo == 0 { parts[0].clone() } else { parts[1].clone() } } else { pulled };
        ctx.doc.begin_undo("ExtendCrvOnSrf");
        ctx.doc.replace_geometry(id, Geometry::Curve(piece));
        Step::Done
    })
}

pub fn gcon() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first curve near the end to test", ObjFilter::Curve), Ask::Pick("Select second curve near the end to test", ObjFilter::Curve)], |a, ctx| {
        let (Ans::Pick(i1, p1), Ans::Pick(i2, p2)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(c1), Some(c2)) = (curve_of(ctx.doc, *i1), curve_of(ctx.doc, *i2)) else { return Step::Cancelled };
        let (e1, t1, end1) = curve_end(&c1, *p1);
        let (e2, t2, end2) = curve_end(&c2, *p2);
        let gap = (e1 - e2).length();
        let ang = t1.dot(-t2).clamp(-1.0, 1.0).acos().to_degrees();
        let k = |c: &NurbsCurve, at_end: bool| {
            let (a0, a1) = c.domain();
            let t = if at_end { a1 } else { a0 };
            let h = (a1 - a0) * 1e-4;
            let d1 = c.derivative_at(t);
            let d2 = (c.derivative_at((t + h).min(a1)) - c.derivative_at((t - h).max(a0))) / (2.0 * h);
            d1.cross(d2).length() / d1.length().powi(3).max(1e-18)
        };
        let (k1, k2) = (k(&c1, end1), k(&c2, end2));
        let level = if gap > ctx.doc.settings.absolute_tolerance { "none (G0 gap)" } else if ang > ctx.doc.settings.angle_tolerance_deg { "G0 (position)" } else if (k1 - k2).abs() > 1e-3 * (1.0 + k1.max(k2)) { "G1 (tangency)" } else { "G2 (curvature)" };
        ctx.log(format!("Curve end difference = {} units, tangent angle = {} degrees, curvature {} / {}", fmt_num(gap), fmt_num(ang), fmt_num(k1), fmt_num(k2)));
        ctx.log(format!("Continuity: {level}"));
        Step::Done
    })
}

pub fn match_crv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select open curve to change near the end", ObjFilter::Curve), Ask::Pick("Select open curve to match near the end", ObjFilter::Curve), Ask::Options("Continuity", vec![opt("Position"), opt("Tangency")])], |a, ctx| {
        let (Ans::Pick(i1, p1), Ans::Pick(i2, p2)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(mut c1), Some(c2)) = (curve_of(ctx.doc, *i1), curve_of(ctx.doc, *i2)) else { return Step::Cancelled };
        let tangency = a[2].text() != "Position";
        let (_, _, end1) = curve_end(&c1, *p1);
        let (e2, t2, _) = curve_end(&c2, *p2);
        let n = c1.control_points.len();
        let (i_end, i_next) = if end1 { (n - 1, n.saturating_sub(2)) } else { (0, 1.min(n - 1)) };
        let old_end = c1.control_points[i_end];
        c1.control_points[i_end] = e2;
        if tangency && n > 1 {
            let len = (c1.control_points[i_next] - old_end).length();
            c1.control_points[i_next] = e2 - t2 * len;
        }
        ctx.doc.begin_undo("Match");
        ctx.doc.replace_geometry(*i1, Geometry::Curve(c1));
        Step::Done
    })
}

pub fn modify_radius() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select arc or circle", ObjFilter::Curve), Ask::Number("New radius", 1.0)], |a, ctx| {
        let id = a[0].ids()[0];
        let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
        let Some(center) = arc_center(&c) else {
            ctx.log("The curve is not an arc or circle.");
            return Step::Cancelled;
        };
        let r_old = (c.start_point() - center).length().max(1e-12);
        let s = a[1].number().abs() / r_old;
        let xf = DAffine3::from_translation(center) * DAffine3::from_scale(DVec3::splat(s)) * DAffine3::from_translation(-center);
        ctx.doc.begin_undo("ModifyRadius");
        ctx.doc.replace_geometry(id, Geometry::Curve(c.transformed(&xf)));
        Step::Done
    })
}

pub fn move_crv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select polyline segment to move", ObjFilter::Curve), Ask::Point("Point to move from"), Ask::Point("Point to move to")], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(mut c) = curve_of(ctx.doc, *id) else { return Step::Cancelled };
        if c.degree != 1 {
            ctx.log("MoveCrv moves polyline segments; use PointsOn for other curves.");
            return Step::Cancelled;
        }
        let n = c.control_points.len();
        let (si, _) = (0..n - 1).map(|i| { let (x, y) = (c.control_points[i], c.control_points[i + 1]); let ab = y - x; let t = ((*p - x).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0); (i, (x + ab * t - *p).length()) }).min_by(|x, y| x.1.partial_cmp(&y.1).unwrap()).unwrap();
        let d = a[2].point() - a[1].point();
        c.control_points[si] += d;
        c.control_points[si + 1] += d;
        if c.is_closed() && (si == 0 || si + 1 == n - 1) {
            let first = c.control_points[0];
            c.control_points[n - 1] = first;
        }
        ctx.doc.begin_undo("MoveCrv");
        ctx.doc.replace_geometry(*id, Geometry::Curve(c));
        Step::Done
    })
}

pub fn offset_crv_on_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve on surface", ObjFilter::Curve), Ask::Pick("Select surface", ObjFilter::Surface), Ask::Number("Offset distance", 1.0)], |a, ctx| {
        let (Some(c), Some(faces)) = (curve_of(ctx.doc, a[0].ids()[0]), faces_of(ctx.doc, a[1].ids()[0])) else { return Step::Cancelled };
        let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
        // offset in the local tangent plane: sample, move along (normal x tangent), pull back
        let n = 48;
        let (t0, t1) = c.domain();
        let d = a[2].number();
        let pts: Vec<DVec3> = (0..=n).map(|i| {
            let t = t0 + (t1 - t0) * i as f64 / n as f64;
            let p = c.point_at(t);
            let (_, nrm) = pull_point(&faces, p);
            let side = nrm.cross(c.tangent_at(t)).normalize_or(DVec3::X);
            pull_point(&faces, p + side * d).0
        }).collect();
        let _ = tol;
        match NurbsCurve::interpolate(&pts, 3) {
            Ok(o) => add_all(ctx, "OffsetCrvOnSrf", vec![Geometry::Curve(o)]),
            Err(_) => Step::Cancelled,
        }
    })
}

/// Rigid transform mapping segment (a0, a1) onto (b0, b1) with uniform scale.
fn two_point_xf(a0: DVec3, a1: DVec3, b0: DVec3, b1: DVec3, scale: bool) -> DAffine3 {
    let da = a1 - a0;
    let db = b1 - b0;
    let s = if scale && da.length() > 1e-12 { db.length() / da.length() } else { 1.0 };
    let q = DQuat::from_rotation_arc(da.normalize_or(DVec3::X), db.normalize_or(DVec3::X));
    DAffine3::from_translation(b0) * DAffine3::from_quat(q) * DAffine3::from_scale(DVec3::splat(s)) * DAffine3::from_translation(-a0)
}

pub fn orient_crv_to_edge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve to orient", ObjFilter::Curve), Ask::Pick("Select surface edge", ObjFilter::Surface), Ask::Options("Copy", vec![opt("Yes"), opt("No")])], |a, ctx| {
        let Ans::Pick(cid, _) = &a[0] else { return Step::Cancelled };
        let Ans::Pick(sid, p) = &a[1] else { return Step::Cancelled };
        let (Some(c), Some(faces)) = (curve_of(ctx.doc, *cid), faces_of(ctx.doc, *sid)) else { return Step::Cancelled };
        let f = &faces[face_under(&faces, *p)];
        let edges = f.edge_curves(f.bounding_box().diagonal().max(1e-6) * 1e-3);
        let Some(edge) = edges.into_iter().min_by(|x, y| (x.point_at(x.closest_parameter(*p)) - *p).length().partial_cmp(&(y.point_at(y.closest_parameter(*p)) - *p).length()).unwrap()) else { return Step::Cancelled };
        let xf = two_point_xf(c.start_point(), c.end_point(), edge.start_point(), edge.end_point(), true);
        ctx.doc.begin_undo("OrientCrvToEdge");
        if a[2].text() == "No" {
            ctx.doc.replace_geometry(*cid, Geometry::Curve(c.transformed(&xf)));
            Step::Done
        } else {
            add_all(ctx, "OrientCrvToEdge", vec![Geometry::Curve(c.transformed(&xf))])
        }
    })
}

pub fn orient_on_crv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to orient", ObjFilter::Any, 1), Ask::Point("Base point"), Ask::Pick("Select target curve near the target point", ObjFilter::Curve), Ask::Options("Copy", vec![opt("Yes"), opt("No")])], |a, ctx| {
        let Ans::Pick(cid, p) = &a[2] else { return Step::Cancelled };
        let Some(c) = curve_of(ctx.doc, *cid) else { return Step::Cancelled };
        let base = a[1].point();
        let t = c.closest_parameter(*p);
        let target = c.point_at(t);
        let xf = two_point_xf(base, base + ctx.cplane.x_axis, target, target + c.tangent_at(t), false);
        let ids = a[0].ids();
        ctx.doc.begin_undo("OrientOnCrv");
        if a[3].text() == "No" {
            ctx.doc.transform_objects(&ids, &xf);
            return Step::Done;
        }
        let mut out = Vec::new();
        for id in ids {
            if let Some(o) = ctx.doc.get(id) {
                let mut g = o.geometry.clone();
                g.transform(&xf);
                out.push(g);
            }
        }
        add_all(ctx, "OrientOnCrv", out)
    })
}

pub fn orient_on_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to orient", ObjFilter::Any, 1), Ask::Point("Base point"), Ask::Pick("Select target surface at the target point", ObjFilter::Surface), Ask::Options("Copy", vec![opt("Yes"), opt("No")])], |a, ctx| {
        let Ans::Pick(sid, p) = &a[2] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *sid) else { return Step::Cancelled };
        let (target, n) = pull_point(&faces, *p);
        let base = a[1].point();
        let cp = ctx.cplane;
        let q = DQuat::from_rotation_arc(cp.z_axis, n);
        let xf = DAffine3::from_translation(target) * DAffine3::from_quat(q) * DAffine3::from_translation(-base);
        let ids = a[0].ids();
        ctx.doc.begin_undo("OrientOnSrf");
        if a[3].text() == "No" {
            ctx.doc.transform_objects(&ids, &xf);
            return Step::Done;
        }
        let mut out = Vec::new();
        for id in ids {
            if let Some(o) = ctx.doc.get(id) {
                let mut g = o.geometry.clone();
                g.transform(&xf);
                out.push(g);
            }
        }
        add_all(ctx, "OrientOnSrf", out)
    })
}

pub struct RevCloudCmd {
    pts: Vec<DVec3>,
}

impl Default for RevCloudCmd {
    fn default() -> Self {
        Self { pts: Vec::new() }
    }
}

impl RevCloudCmd {
    fn build(&self, cp: &Plane, pts: &[DVec3]) -> Option<NurbsCurve> {
        if pts.len() < 2 {
            return None;
        }
        let mut joined: Option<NurbsCurve> = None;
        let n = pts.len();
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            if i == n - 1 && (a - b).length() < 1e-9 {
                break;
            }
            let chord = b - a;
            let mid = (a + b) * 0.5 + cp.z_axis.cross(chord).normalize_or(cp.y_axis) * (chord.length() * 0.35) * -1.0;
            let Ok(arc) = NurbsCurve::arc_3pt(a, mid, b) else { continue };
            joined = Some(match joined {
                None => arc,
                Some(j) => j.join_any(&arc).unwrap_or(j),
            });
        }
        joined
    }
}

impl Command for RevCloudCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Start of revision cloud").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                Prompt::point("Next point. Press Enter to close").step()
            }
            Input::Empty if self.pts.len() >= 3 => match self.build(&ctx.cplane, &self.pts) {
                Some(c) => add_all(ctx, "RevCloud", vec![Geometry::Curve(c)]),
                None => Step::Cancelled,
            },
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let mut pts = self.pts.clone();
        if let Some(c) = cursor {
            pts.push(c);
        }
        if pts.len() >= 2 {
            self.build(&Plane::world_xy(), &pts).map(|c| vec![Preview::Curve(c)]).unwrap_or_default()
        } else {
            vec![]
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

pub fn short_path() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface", ObjFilter::Surface), Ask::Point("Start of path"), Ask::Point("End of path")], |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[0].ids()[0]) else { return Step::Cancelled };
        let n = 48;
        let (p, q) = (pull_point(&faces, a[1].point()).0, pull_point(&faces, a[2].point()).0);
        let mut pts: Vec<DVec3> = (0..=n).map(|i| p + (q - p) * (i as f64 / n as f64)).collect();
        for _ in 0..60 {
            for i in 1..n {
                pts[i] = (pts[i - 1] + pts[i + 1]) * 0.5;
            }
            for pt in pts.iter_mut() {
                *pt = pull_point(&faces, *pt).0;
            }
        }
        match NurbsCurve::interpolate(&pts, 3) {
            Ok(c) => add_all(ctx, "ShortPath", vec![Geometry::Curve(c)]),
            Err(_) => Step::Cancelled,
        }
    })
}

pub fn align_profiles() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve to align", ObjFilter::Curve), Ask::Pick("Select reference curve", ObjFilter::Curve)], |a, ctx| {
        let (Some(c), Some(r)) = (curve_of(ctx.doc, a[0].ids()[0]), curve_of(ctx.doc, a[1].ids()[0])) else { return Step::Cancelled };
        let xf = two_point_xf(c.start_point(), c.end_point(), r.start_point(), r.end_point(), false);
        ctx.doc.begin_undo("AlignProfiles");
        ctx.doc.replace_geometry(a[0].ids()[0], Geometry::Curve(c.transformed(&xf)));
        Step::Done
    })
}

pub fn arrowhead() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve near the end for the arrowhead", ObjFilter::Curve), Ask::Options("Arrowhead", vec![opt("Add"), opt("Remove")])], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let group = format!("Arrowhead{id}");
        if a[1].text() == "Remove" {
            let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| o.attributes.groups.contains(&group)).map(|o| o.id).filter(|i| i != id).collect();
            ctx.doc.begin_undo("Arrowhead");
            for i in ids {
                ctx.doc.remove(i);
            }
            return Step::Done;
        }
        let Some(c) = curve_of(ctx.doc, *id) else { return Step::Cancelled };
        let (e, t, _) = curve_end(&c, *p);
        let h = ctx.mem_num("Dim.TextHeight", 1.0);
        let n = ctx.cplane.z_axis.cross(t).normalize_or(ctx.cplane.y_axis);
        let Ok(head) = NurbsCurve::polyline(vec![e - t * (h * 0.8) + n * (h * 0.25), e, e - t * (h * 0.8) - n * (h * 0.25), e - t * (h * 0.8) + n * (h * 0.25)]) else { return Step::Cancelled };
        ctx.doc.begin_undo("Arrowhead");
        let mut attrs = ctx.doc.get(*id).map(|o| o.attributes.clone()).unwrap_or_default();
        attrs.groups.push(group.clone());
        if let Some(o) = ctx.doc.get_mut(*id) {
            if !o.attributes.groups.contains(&group) {
                o.attributes.groups.push(group);
            }
        }
        ctx.doc.add_with(Geometry::Curve(head), attrs);
        Step::Done
    })
}

pub fn fillet_corners() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select polylines", ObjFilter::Curve, 1), Ask::Number("Fillet radius", 1.0)], |a, ctx| {
        let r = a[1].number().abs();
        ctx.doc.begin_undo("FilletCorners");
        for id in a[0].ids() {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            if c.degree != 1 || c.control_points.len() < 3 {
                continue;
            }
            let pts = c.control_points.clone();
            let n = pts.len();
            let closed = c.is_closed();
            let count = if closed { n - 1 } else { n };
            let mut pieces: Vec<NurbsCurve> = Vec::new();
            let corner_range: Vec<usize> = if closed { (0..count).collect() } else { (1..count - 1).collect() };
            let mut tangent_pts: Vec<(DVec3, DVec3)> = Vec::new(); // (in, out) per corner index
            for &i in &corner_range {
                let prev = pts[(i + count - 1) % count];
                let cur = pts[i % count];
                let next = pts[(i + 1) % count];
                let d1 = (prev - cur).normalize_or(DVec3::X);
                let d2 = (next - cur).normalize_or(DVec3::Y);
                let half = d1.dot(d2).clamp(-1.0, 1.0).acos() * 0.5;
                let dist = if half.tan().abs() < 1e-12 { 0.0 } else { r / half.tan() };
                let dist = dist.min((prev - cur).length() * 0.5).min((next - cur).length() * 0.5);
                tangent_pts.push((cur + d1 * dist, cur + d2 * dist));
            }
            // build: segments between consecutive fillets plus arcs
            let mut seq: Vec<DVec3> = Vec::new();
            if !closed {
                seq.push(pts[0]);
            }
            for (k, &i) in corner_range.iter().enumerate() {
                let (tin, tout) = tangent_pts[k];
                let cur = pts[i % count];
                if let Some(last) = seq.last().copied() {
                    if let Ok(l) = NurbsCurve::line(last, tin) {
                        pieces.push(l);
                    }
                }
                let mid = cur + ((tin - cur) + (tout - cur)).normalize_or(DVec3::X) * {
                    let d1 = (tin - cur).length();
                    let half_ang = (tin - cur).normalize_or(DVec3::X).dot((tout - cur).normalize_or(DVec3::Y)).clamp(-1.0, 1.0).acos() * 0.5;
                    (d1 / half_ang.cos().max(1e-9)) - r
                };
                if let Ok(arc) = NurbsCurve::arc_3pt(tin, mid, tout) {
                    pieces.push(arc);
                }
                seq.push(tout);
            }
            if closed {
                if let (Some(first), Some(last)) = (tangent_pts.first(), seq.last().copied()) {
                    if let Ok(l) = NurbsCurve::line(last, first.0) {
                        pieces.push(l);
                    }
                }
            } else if let (Some(last), Some(end)) = (seq.last().copied(), pts.last().copied()) {
                if let Ok(l) = NurbsCurve::line(last, end) {
                    pieces.push(l);
                }
            }
            if pieces.is_empty() {
                continue;
            }
            let mut joined = pieces[0].clone();
            for p in &pieces[1..] {
                if let Ok(j) = joined.join_any(p) {
                    joined = j;
                }
            }
            ctx.doc.replace_geometry(id, Geometry::Curve(joined));
        }
        Step::Done
    })
}

pub fn mark_foci() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select ellipses", ObjFilter::Curve, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            let pts = c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3);
            let center = pts.iter().fold(DVec3::ZERO, |s, p| s + *p) / pts.len() as f64;
            let (far, near) = pts.iter().fold((DVec3::ZERO, f64::INFINITY), |(f, nmin), p| { let d = (*p - center).length(); ((if d > (f - center).length() { *p } else { f }), nmin.min(d)) });
            let aa = (far - center).length();
            let bb = near;
            if aa <= bb + 1e-9 {
                out.push(Geometry::Point(center));
                continue;
            }
            let f = (aa * aa - bb * bb).sqrt();
            let dir = (far - center).normalize();
            out.push(Geometry::Point(center + dir * f));
            out.push(Geometry::Point(center - dir * f));
        }
        add_all(ctx, "MarkFoci", out)
    })
}

pub fn evaluate_uv_pt() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface at the point to evaluate", ObjFilter::Surface)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let f = &faces[face_under(&faces, *p)];
        let (u, v) = f.surface.closest_uv(*p);
        let (u0, u1) = f.surface.domain_u();
        let (v0, v1) = f.surface.domain_v();
        ctx.log(format!("u = {} ({} normalized), v = {} ({} normalized)", fmt_num(u), fmt_num((u - u0) / (u1 - u0).max(1e-12)), fmt_num(v), fmt_num((v - v0) / (v1 - v0).max(1e-12))));
        Step::Done
    })
}

pub fn points_from_uv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface", ObjFilter::Surface), Ask::Number("u (normalized 0..1)", 0.5), Ask::Number("v (normalized 0..1)", 0.5)], |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[0].ids()[0]) else { return Step::Cancelled };
        let s = &faces[0].surface;
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let p = s.point_at(u0 + (u1 - u0) * a[1].number(), v0 + (v1 - v0) * a[2].number());
        add_all(ctx, "PointsFromUV", vec![Geometry::Point(p)])
    })
}

pub fn point_deviation() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select points or point-like objects", ObjFilter::Any, 1), Ask::Pick("Select surface or mesh", ObjFilter::Any)], |a, ctx| {
        let tid = a[1].ids()[0];
        let faces = faces_of(ctx.doc, tid);
        let mesh = match ctx.doc.get(tid).map(|o| &o.geometry) { Some(Geometry::Mesh(m)) => Some(m.clone()), _ => None };
        let (mut dmax, mut dmin, mut sum, mut n) = (0.0f64, f64::INFINITY, 0.0, 0);
        for id in a[0].ids() {
            let pts: Vec<DVec3> = match ctx.doc.get(id).map(|o| &o.geometry) {
                Some(Geometry::Point(p)) => vec![*p],
                Some(Geometry::Curve(c)) => c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-2),
                Some(Geometry::Mesh(m)) => m.vertices.clone(),
                _ => Vec::new(),
            };
            for p in pts {
                let d = if let Some(f) = &faces { (pull_point(f, p).0 - p).length() } else if let Some(m) = &mesh { m.vertices.iter().map(|v| (*v - p).length()).fold(f64::INFINITY, f64::min) } else { continue };
                dmax = dmax.max(d);
                dmin = dmin.min(d);
                sum += d;
                n += 1;
            }
        }
        if n == 0 {
            return Step::Cancelled;
        }
        ctx.log(format!("{n} points: minimum deviation {}, maximum {}, average {}", fmt_num(dmin), fmt_num(dmax), fmt_num(sum / n as f64)));
        Step::Done
    })
}

pub fn create_uv_crv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces", ObjFilter::Surface, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            for f in faces_of(ctx.doc, id).unwrap_or_default() {
                let s = &f.surface;
                let (u0, u1) = s.domain_u();
                let (v0, v1) = s.domain_v();
                let loops: Vec<Vec<glam::DVec2>> = if f.loops.is_empty() { vec![vec![glam::DVec2::new(u0, v0), glam::DVec2::new(u1, v0), glam::DVec2::new(u1, v1), glam::DVec2::new(u0, v1)]] } else { f.loops.clone() };
                for l in loops {
                    let mut pts: Vec<DVec3> = l.iter().map(|uv| DVec3::new(uv.x - u0, uv.y - v0, 0.0)).collect();
                    if let Some(first) = pts.first().copied() {
                        pts.push(first);
                    }
                    if let Ok(c) = NurbsCurve::polyline(pts) {
                        out.push(Geometry::Curve(c));
                    }
                }
            }
        }
        add_all(ctx, "CreateUVCrv", out)
    })
}

pub fn apply_crv() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select planar curves in the world XY plane", ObjFilter::Curve, 1), Ask::Pick("Select surface", ObjFilter::Surface)], |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[1].ids()[0]) else { return Step::Cancelled };
        let s = &faces[0].surface;
        let (u0, _) = s.domain_u();
        let (v0, _) = s.domain_v();
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            let pts: Vec<DVec3> = c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3).iter().map(|p| s.point_at(u0 + p.x, v0 + p.y)).collect();
            let r = if c.degree == 1 { NurbsCurve::polyline(pts) } else { NurbsCurve::interpolate(&pts, 3) };
            if let Ok(nc) = r {
                out.push(Geometry::Curve(nc));
            }
        }
        add_all(ctx, "ApplyCrv", out)
    })
}

pub fn flatten_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces", ObjFilter::Surface, 1)], |a, ctx| {
        let cp = ctx.cplane;
        let mut out = Vec::new();
        for id in a[0].ids() {
            for f in faces_of(ctx.doc, id).unwrap_or_default() {
                for mut c in f.edge_curves(f.bounding_box().diagonal().max(1e-6) * 1e-3) {
                    for p in c.control_points.iter_mut() {
                        *p = *p - cp.z_axis * (*p - cp.origin).dot(cp.z_axis);
                    }
                    out.push(Geometry::Curve(c));
                }
            }
        }
        add_all(ctx, "FlattenSrf", out)
    })
}

pub fn convert() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select curves to convert", ObjFilter::Curve, 1), Ask::Options("Output", vec![opt("Lines"), opt("Arcs")]), Ask::Number("Tolerance", 0.01)], |a, ctx| {
        let arcs = a[1].text() == "Arcs";
        let tol = a[2].number().abs().max(1e-9);
        ctx.doc.begin_undo("Convert");
        for id in a[0].ids() {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            let pts = c.tessellate_adaptive(tol);
            let nc = if arcs {
                let mut joined: Option<NurbsCurve> = None;
                let mut i = 0;
                while i + 2 < pts.len() {
                    let piece = NurbsCurve::arc_3pt(pts[i], pts[i + 1], pts[i + 2]).or_else(|_| NurbsCurve::line(pts[i], pts[i + 2]));
                    if let Ok(p) = piece {
                        joined = Some(match joined { None => p, Some(j) => j.join_any(&p).unwrap_or(j) });
                    }
                    i += 2;
                }
                if i + 1 < pts.len() {
                    if let Ok(l) = NurbsCurve::line(pts[i], pts[i + 1]) {
                        joined = Some(match joined { None => l, Some(j) => j.join_any(&l).unwrap_or(j) });
                    }
                }
                joined
            } else {
                NurbsCurve::polyline(pts).ok()
            };
            if let Some(n) = nc {
                ctx.doc.replace_geometry(id, Geometry::Curve(n));
            }
        }
        Step::Done
    })
}

pub fn crv_seam() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select closed curve", ObjFilter::Curve), Ask::Point("New seam point")], |a, ctx| {
        let id = a[0].ids()[0];
        let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
        if !c.is_closed() {
            ctx.log("The curve is not closed.");
            return Step::Cancelled;
        }
        let t = c.closest_parameter(a[1].point());
        let Some((l, r)) = c.split(t) else { return Step::Cancelled };
        match r.join_any(&l) {
            Ok(j) => {
                ctx.doc.begin_undo("CrvSeam");
                ctx.doc.replace_geometry(id, Geometry::Curve(j));
                Step::Done
            }
            Err(e) => {
                ctx.log(format!("CrvSeam failed: {e}"));
                Step::Cancelled
            }
        }
    })
}

/// Weighted move of control points / vertices around a centre (SoftEditCrv, SoftEditSrf, SoftMove).
fn soft_move(g: &mut Geometry, center: DVec3, radius: f64, delta: DVec3) {
    let w = |p: DVec3| {
        let d = (p - center).length() / radius.max(1e-9);
        if d >= 1.0 { 0.0 } else { (1.0 - d * d).powi(2) }
    };
    let f = |p: DVec3| p + delta * w(p);
    match g {
        Geometry::Point(p) => *p = f(*p),
        Geometry::Curve(c) => c.control_points.iter_mut().for_each(|p| *p = f(*p)),
        Geometry::Surface(s) => s.control_points.iter_mut().for_each(|p| *p = f(*p)),
        Geometry::Brep(b) => b.faces.iter_mut().for_each(|fc| fc.surface.control_points.iter_mut().for_each(|p| *p = f(*p))),
        Geometry::Mesh(m) => {
            m.vertices.iter_mut().for_each(|p| *p = f(*p));
            m.compute_normals();
        }
        Geometry::Instance { xf, .. } => xf.translation = f(xf.translation),
    }
}

pub fn soft_edit(label: &'static str, filter: ObjFilter) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to edit", filter, 1), Ask::Point("Point to move"), Ask::Number("Falloff distance", 5.0), Ask::Point("Point to move to")], move |a, ctx| {
        let center = a[1].point();
        let radius = a[2].number().abs();
        let delta = a[3].point() - center;
        ctx.doc.begin_undo(label);
        for id in a[0].ids() {
            if let Some(mut g) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                soft_move(&mut g, center, radius, delta);
                ctx.doc.replace_geometry(id, g);
            }
        }
        Step::Done
    })
}

pub fn fixed_length_edit() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve near the control point to move", ObjFilter::Curve), Ask::Point("Point to move to")], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(mut c) = curve_of(ctx.doc, *id) else { return Step::Cancelled };
        let len0 = c.length();
        let (i, _) = c.control_points.iter().enumerate().min_by(|x, y| (*x.1 - *p).length().partial_cmp(&(*y.1 - *p).length()).unwrap()).unwrap();
        c.control_points[i] = a[1].point();
        let len1 = c.length().max(1e-12);
        let start = c.start_point();
        let s = len0 / len1;
        let xf = DAffine3::from_translation(start) * DAffine3::from_scale(DVec3::splat(s)) * DAffine3::from_translation(-start);
        ctx.doc.begin_undo("FixedLengthCrvEdit");
        ctx.doc.replace_geometry(*id, Geometry::Curve(c.transformed(&xf)));
        Step::Done
    })
}

pub fn end_bulge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve near the end to adjust", ObjFilter::Curve), Ask::Number("Bulge factor (1 keeps, 2 doubles the end tangent length)", 1.5)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(mut c) = curve_of(ctx.doc, *id) else { return Step::Cancelled };
        let (_, _, at_end) = curve_end(&c, *p);
        let n = c.control_points.len();
        if n < 3 {
            return Step::Cancelled;
        }
        let (e, i) = if at_end { (n - 1, n - 2) } else { (0, 1) };
        let end = c.control_points[e];
        let v = c.control_points[i] - end;
        c.control_points[i] = end + v * a[1].number();
        ctx.doc.begin_undo("EndBulge");
        ctx.doc.replace_geometry(*id, Geometry::Curve(c));
        Step::Done
    })
}

pub fn move_uvn() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces", ObjFilter::Surface, 1), Ask::Number("Move along U", 0.0), Ask::Number("Move along V", 0.0), Ask::Number("Move along normal", 1.0)], |a, ctx| {
        let (du, dv, dn) = (a[1].number(), a[2].number(), a[3].number());
        ctx.doc.begin_undo("MoveUVN");
        for id in a[0].ids() {
            let Some(Geometry::Surface(mut s)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let grev = |knots: &Vec<f64>, deg: usize, i: usize| knots[i + 1..=i + deg].iter().sum::<f64>() / deg.max(1) as f64;
            let orig = s.clone();
            for i in 0..s.count_u {
                for j in 0..s.count_v {
                    let (u, v) = (grev(&orig.knots_u, orig.degree_u, i), grev(&orig.knots_v, orig.degree_v, j));
                    let (_, su, sv) = orig.evaluate(u, v);
                    let n = orig.normal_at(u, v);
                    s.control_points[i * s.count_v + j] += su.normalize_or(DVec3::X) * du + sv.normalize_or(DVec3::Y) * dv + n * dn;
                }
            }
            ctx.doc.replace_geometry(id, Geometry::Surface(s));
        }
        Step::Done
    })
}

pub struct HandleCurveCmd {
    pts: Vec<DVec3>,
    awaiting_handle: bool,
}

impl Default for HandleCurveCmd {
    fn default() -> Self {
        Self { pts: Vec::new(), awaiting_handle: false }
    }
}

impl HandleCurveCmd {
    fn build(pts: &[DVec3]) -> Option<NurbsCurve> {
        // pts = anchor, handle, anchor, handle, ...
        if pts.len() < 4 {
            return None;
        }
        let mut joined: Option<NurbsCurve> = None;
        let mut i = 0;
        while i + 3 < pts.len() {
            let (p0, h0, p1, h1) = (pts[i], pts[i + 1], pts[i + 2], pts[i + 3]);
            let seg = NurbsCurve::from_control_points(3, vec![p0, h0, p1 - (h1 - p1), p1]).ok()?;
            joined = Some(match joined { None => seg, Some(j) => j.join_any(&seg).unwrap_or(j) });
            i += 2;
        }
        joined
    }
}

impl Command for HandleCurveCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Start of curve").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                self.awaiting_handle = !self.awaiting_handle;
                Prompt::point(if self.awaiting_handle { "Handle end (tangent direction)" } else { "Next point. Press Enter when done" }).step()
            }
            Input::Empty => {
                if self.awaiting_handle {
                    // repeat the last anchor as its own handle
                    let last = *self.pts.last().unwrap();
                    self.pts.push(last);
                }
                match Self::build(&self.pts) {
                    Some(c) => add_all(ctx, "HandleCurve", vec![Geometry::Curve(c)]),
                    None => Step::Cancelled,
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let mut pts = self.pts.clone();
        if let Some(c) = cursor {
            pts.push(c);
            if pts.len() % 2 == 1 {
                pts.push(c);
            }
        }
        let mut out = Vec::new();
        if let Some(c) = Self::build(&pts) {
            out.push(Preview::Curve(c));
        }
        for pair in pts.chunks(2) {
            if pair.len() == 2 {
                out.push(Preview::Line(pair[0], pair[1]));
            }
        }
        out
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

/// Sketch: points collected while dragging in a viewport (see App::sketch_points).
pub struct SketchCmd;

impl Command for SketchCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.mem_set("Sketch.Active", "Yes");
        Prompt::text("Drag in a viewport to sketch a curve, or type points as x,y,z separated by ';'").opts([optv("Degree", "3")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Text(t) = input else {
            ctx.memory.remove("Sketch.Active");
            return Step::Cancelled;
        };
        ctx.memory.remove("Sketch.Active");
        let pts: Vec<DVec3> = t.split(';').filter_map(|s| { let n: Vec<f64> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect(); if n.len() == 3 { Some(DVec3::new(n[0], n[1], n[2])) } else { None } }).collect();
        if pts.len() < 2 {
            return Step::Cancelled;
        }
        // thin out nearly coincident samples
        let mut thin: Vec<DVec3> = vec![pts[0]];
        for p in &pts[1..] {
            if (*p - *thin.last().unwrap()).length() > 1e-6 {
                thin.push(*p);
            }
        }
        match NurbsCurve::interpolate(&thin, 3.min(thin.len() - 1)) {
            Ok(c) => add_all(ctx, "Sketch", vec![Geometry::Curve(c)]),
            Err(_) => Step::Cancelled,
        }
    }
}
