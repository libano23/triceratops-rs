//! Third batch of smaller commands: curve utilities, point/grid extraction, surface
//! wireframes, mesh utilities, more Sel* filters, deformations, dimensions and tools.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::{Document, Geometry, Object, ObjectId};
use tri_geom::{curve_plane, Brep, Face, Mesh, NurbsCurve, NurbsSurface, Plane};

use super::annotate::text_curves;
use super::{fmt_num, opt, optv, AppRequest, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn curve_of(doc: &Document, id: ObjectId) -> Option<NurbsCurve> {
    doc.get(id).and_then(|o| o.geometry.as_curve().cloned())
}

fn add_all(ctx: &mut CmdCtx, label: &str, geoms: Vec<Geometry>) -> Step {
    if geoms.is_empty() {
        ctx.log(format!("{label}: nothing created."));
        return Step::Cancelled;
    }
    ctx.doc.begin_undo(label);
    ctx.doc.clear_selection();
    for g in geoms {
        let id = ctx.doc.add(g);
        ctx.doc.select(id, true);
    }
    Step::Done
}

// ---- generic selection by predicate ------------------------------------------------------------

pub struct SelByCmd {
    pub label: &'static str,
    pub pred: fn(&Document, &Object) -> bool,
}

impl Command for SelByCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && (self.pred)(ctx.doc, o)).map(|o| o.id).collect();
        for id in &ids {
            ctx.doc.select(*id, true);
        }
        ctx.log(match ids.len() {
            0 => "No objects added to selection.".to_string(),
            1 => "1 object added to selection.".to_string(),
            n => format!("{n} objects added to selection."),
        });
        let _ = self.label;
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub fn is_planar_curve(c: &NurbsCurve) -> bool {
    curve_plane(c).is_some()
}

pub fn is_box(b: &Brep) -> bool {
    b.closed && b.faces.len() == 6 && b.faces.iter().all(|f| super::solid_edit::planar_loops(f).is_some())
}

/// SelVolumeSphere / SelVolumePipe-like: objects whose bounding box centre is inside a sphere.
#[derive(Default)]
pub struct SelVolumeSphereCmd {
    center: Option<DVec3>,
}

impl Command for SelVolumeSphereCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Center of sphere").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.center, input) {
            (None, Input::Point(p)) => {
                self.center = Some(p);
                Prompt::point_or_number("Radius").step()
            }
            (Some(c), inp @ (Input::Point(_) | Input::Number(_))) => {
                let r = match inp { Input::Number(v) => v.abs(), Input::Point(p) => (p - c).length(), _ => 0.0 };
                let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && (ctx.doc.object_bbox(o).center() - c).length() <= r).map(|o| o.id).collect();
                for id in &ids {
                    ctx.doc.select(*id, true);
                }
                ctx.log(format!("{} objects added to selection.", ids.len()));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center
    }
}

// ---- curve utilities ------------------------------------------------------------------------------

pub enum CurveUtil {
    SubCrv,
    ExtractSubCrv,
    DeleteSubCrv,
    ReducePolyline,
    ConvertToBeziers,
    MakeUniform,
    MakePeriodic,
    MakeNonPeriodic,
    Reparameterize,
    Domain,
    CrvStart,
    CrvEnd,
    InsertKink,
    RemoveKnot,
    ClosestPt,
    Curvature,
    TweenCurves,
    LineThroughPt,
    PlaneThroughPt,
    ExtractControlPolygon,
    MatchCrvDir,
}

pub struct CurveUtilCmd {
    op: CurveUtil,
    ids: Vec<ObjectId>,
    pts: Vec<DVec3>,
    pick: Option<(ObjectId, DVec3)>,
}

impl CurveUtilCmd {
    pub fn new(op: CurveUtil) -> Self {
        Self { op, ids: Vec::new(), pts: Vec::new(), pick: None }
    }
}

fn douglas_peucker(pts: &[DVec3], tol: f64) -> Vec<DVec3> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let ab = b - a;
    let mut best = (0usize, 0.0f64);
    for (i, p) in pts.iter().enumerate().skip(1).take(pts.len() - 2) {
        let d = if ab.length_squared() < 1e-18 { (*p - a).length() } else { (*p - a).cross(ab).length() / ab.length() };
        if d > best.1 {
            best = (i, d);
        }
    }
    if best.1 <= tol {
        return vec![a, b];
    }
    let mut left = douglas_peucker(&pts[..=best.0], tol);
    let right = douglas_peucker(&pts[best.0..], tol);
    left.pop();
    left.extend(right);
    left
}

impl Command for CurveUtilCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        match self.op {
            CurveUtil::SubCrv | CurveUtil::ExtractSubCrv | CurveUtil::DeleteSubCrv => Prompt::pick("Select curve", ObjFilter::Curve).opts([optv("Copy", "No"), optv("Mode", "Shorten")]).step(),
            CurveUtil::ReducePolyline => Prompt::objects("Select polylines to reduce", ObjFilter::Curve, 1).opts([optv("Tolerance", "0.01")]).step(),
            CurveUtil::ConvertToBeziers => Prompt::objects("Select curves to convert to Bézier segments", ObjFilter::Curve, 1).step(),
            CurveUtil::MakeUniform => Prompt::objects("Select curves to make uniform", ObjFilter::Curve, 1).step(),
            CurveUtil::MakePeriodic => Prompt::objects("Select curves to make periodic", ObjFilter::Curve, 1).step(),
            CurveUtil::MakeNonPeriodic => Prompt::objects("Select curves to make non-periodic", ObjFilter::Curve, 1).step(),
            CurveUtil::Reparameterize => Prompt::objects("Select curves to reparameterize", ObjFilter::Curve, 1).opts([optv("Automatic", "No")]).step(),
            CurveUtil::Domain => Prompt::objects("Select curves", ObjFilter::Curve, 1).step(),
            CurveUtil::CrvStart => Prompt::objects("Select curves to mark start points", ObjFilter::Curve, 1).step(),
            CurveUtil::CrvEnd => Prompt::objects("Select curves to mark end points", ObjFilter::Curve, 1).step(),
            CurveUtil::InsertKink => Prompt::pick("Select curve for kink", ObjFilter::Curve).step(),
            CurveUtil::RemoveKnot => Prompt::pick("Select curve near knot to remove", ObjFilter::Curve).step(),
            CurveUtil::ClosestPt => Prompt::objects("Select objects to find closest points on", ObjFilter::Any, 1).step(),
            CurveUtil::Curvature => Prompt::pick("Select curve for curvature measurement", ObjFilter::Curve).step(),
            CurveUtil::TweenCurves => Prompt::objects("Select two curves", ObjFilter::Curve, 2).opts([optv("Number", "1")]).step(),
            CurveUtil::LineThroughPt | CurveUtil::PlaneThroughPt => Prompt::objects("Select point objects", ObjFilter::Point, 2).step(),
            CurveUtil::ExtractControlPolygon => Prompt::objects("Select curves and surfaces", ObjFilter::Any, 1).step(),
            CurveUtil::MatchCrvDir => Prompt::pick("Select curve whose direction to match", ObjFilter::Curve).step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.op, input) {
            (CurveUtil::SubCrv | CurveUtil::ExtractSubCrv | CurveUtil::DeleteSubCrv, Input::Pick(id, _)) => {
                self.pick = Some((id, DVec3::ZERO));
                Prompt::point("Start of sub-curve").step()
            }
            (CurveUtil::SubCrv | CurveUtil::ExtractSubCrv | CurveUtil::DeleteSubCrv, Input::Point(p)) => {
                self.pts.push(p);
                if self.pts.len() < 2 {
                    return Prompt::point("End of sub-curve").step();
                }
                let Some((id, _)) = self.pick else { return Step::Cancelled };
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let mut t0 = c.closest_parameter(self.pts[0]);
                let mut t1 = c.closest_parameter(self.pts[1]);
                if t0 > t1 {
                    std::mem::swap(&mut t0, &mut t1);
                }
                let (d0, d1) = c.domain();
                let left = if t0 > d0 + 1e-9 { c.split(t0) } else { None };
                let (head, rest) = match left { Some((l, r)) => (Some(l), r), None => (None, c.clone()) };
                let (mid, tail) = if t1 < d1 - 1e-9 { match rest.split(t1) { Some((m, t)) => (m, Some(t)), None => (rest, None) } } else { (rest, None) };
                match self.op {
                    CurveUtil::SubCrv => {
                        ctx.doc.begin_undo("SubCrv");
                        ctx.doc.replace_geometry(id, Geometry::Curve(mid));
                    }
                    CurveUtil::ExtractSubCrv => {
                        ctx.doc.begin_undo("ExtractSubCrv");
                        let nid = ctx.doc.add(Geometry::Curve(mid));
                        ctx.doc.clear_selection();
                        ctx.doc.select(nid, true);
                    }
                    _ => {
                        ctx.doc.begin_undo("DeleteSubCrv");
                        ctx.doc.remove(id);
                        for part in [head, tail].into_iter().flatten() {
                            ctx.doc.add(Geometry::Curve(part));
                        }
                    }
                }
                Step::Done
            }
            (CurveUtil::ReducePolyline, Input::Objects(ids)) => {
                let tol = ctx.mem_num("ReducePolyline.Tolerance", 0.01);
                ctx.doc.begin_undo("ReducePolyline");
                for id in ids {
                    let Some(c) = curve_of(ctx.doc, id) else { continue };
                    if c.degree != 1 {
                        continue;
                    }
                    let reduced = douglas_peucker(&c.control_points, tol);
                    if let Ok(nc) = NurbsCurve::polyline(reduced) {
                        ctx.doc.replace_geometry(id, Geometry::Curve(nc));
                    }
                }
                Step::Done
            }
            (CurveUtil::ReducePolyline, Input::Option(_)) => Prompt::number("Tolerance").default_num(ctx.mem_num("ReducePolyline.Tolerance", 0.01)).step(),
            (CurveUtil::ReducePolyline, Input::Number(v)) => {
                ctx.mem_set("ReducePolyline.Tolerance", fmt_num(v.abs()));
                Prompt::objects("Select polylines to reduce", ObjFilter::Curve, 1).step()
            }
            (CurveUtil::ConvertToBeziers, Input::Objects(ids)) => {
                ctx.doc.begin_undo("ConvertToBeziers");
                for id in ids {
                    let Some(mut c) = curve_of(ctx.doc, id) else { continue };
                    let p = c.degree;
                    let interior: Vec<f64> = {
                        let ks = &c.knots[p + 1..c.knots.len() - p - 1];
                        let mut out = Vec::new();
                        for k in ks {
                            if out.last().map_or(true, |l: &f64| (l - k).abs() > 1e-12) {
                                out.push(*k);
                            }
                        }
                        out
                    };
                    for k in interior {
                        let mult = c.knots.iter().filter(|x| (*x - k).abs() < 1e-12).count();
                        if mult < p {
                            c.insert_knot(k, p - mult);
                        }
                    }
                    ctx.doc.replace_geometry(id, Geometry::Curve(c));
                }
                Step::Done
            }
            (CurveUtil::MakeUniform, Input::Objects(ids)) => {
                ctx.doc.begin_undo("MakeUniform");
                for id in ids {
                    let Some(mut c) = curve_of(ctx.doc, id) else { continue };
                    let n = c.control_points.len();
                    let p = c.degree;
                    let spans = n - p;
                    let mut knots = vec![0.0; p + 1];
                    for i in 1..spans {
                        knots.push(i as f64 / spans as f64);
                    }
                    knots.extend(std::iter::repeat(1.0).take(p + 1));
                    if knots.len() == c.knots.len() {
                        c.knots = knots;
                        ctx.doc.replace_geometry(id, Geometry::Curve(c));
                    }
                }
                Step::Done
            }
            (CurveUtil::MakePeriodic, Input::Objects(ids)) => {
                ctx.doc.begin_undo("MakePeriodic");
                for id in ids {
                    let Some(c) = curve_of(ctx.doc, id) else { continue };
                    let mut cps = c.control_points.clone();
                    if c.is_closed() && cps.len() > 1 {
                        cps.pop();
                    }
                    if let Ok(nc) = NurbsCurve::periodic(c.degree.max(2), cps) {
                        ctx.doc.replace_geometry(id, Geometry::Curve(nc));
                    }
                }
                Step::Done
            }
            (CurveUtil::MakeNonPeriodic, Input::Objects(ids)) => {
                ctx.doc.begin_undo("MakeNonPeriodic");
                for id in ids {
                    let Some(c) = curve_of(ctx.doc, id) else { continue };
                    let (t0, t1) = c.domain();
                    let mut cc = c.clone();
                    cc.insert_knot(t0, 0);
                    if let Some((_, r)) = c.split(t0 + (t1 - t0) * 1e-9) {
                        ctx.doc.replace_geometry(id, Geometry::Curve(r));
                    }
                }
                Step::Done
            }
            (CurveUtil::Reparameterize, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::number("Domain start").default_num(0.0).step()
            }
            (CurveUtil::Reparameterize, Input::Number(v)) => {
                self.pts.push(DVec3::splat(v));
                if self.pts.len() < 2 {
                    return Prompt::number("Domain end").default_num(1.0).step();
                }
                let (a, b) = (self.pts[0].x, self.pts[1].x);
                if (b - a).abs() < 1e-12 {
                    return Step::Cancelled;
                }
                ctx.doc.begin_undo("Reparameterize");
                for id in &self.ids {
                    let Some(mut c) = curve_of(ctx.doc, *id) else { continue };
                    let (t0, t1) = c.domain();
                    for k in c.knots.iter_mut() {
                        *k = a + (*k - t0) / (t1 - t0) * (b - a);
                    }
                    ctx.doc.replace_geometry(*id, Geometry::Curve(c));
                }
                Step::Done
            }
            (CurveUtil::Reparameterize, Input::Option(_)) => Prompt::objects("Select curves to reparameterize", ObjFilter::Curve, 1).step(),
            (CurveUtil::Domain, Input::Objects(ids)) => {
                for id in ids {
                    if let Some(c) = curve_of(ctx.doc, id) {
                        let (a, b) = c.domain();
                        ctx.log(format!("Curve domain: {} to {}", fmt_num(a), fmt_num(b)));
                    }
                }
                Step::Done
            }
            (CurveUtil::CrvStart | CurveUtil::CrvEnd, Input::Objects(ids)) => {
                let start = matches!(self.op, CurveUtil::CrvStart);
                let pts: Vec<Geometry> = ids.iter().filter_map(|id| curve_of(ctx.doc, *id)).map(|c| Geometry::Point(if start { c.start_point() } else { c.end_point() })).collect();
                add_all(ctx, if start { "CrvStart" } else { "CrvEnd" }, pts)
            }
            (CurveUtil::InsertKink, Input::Pick(id, _)) => {
                self.pick = Some((id, DVec3::ZERO));
                Prompt::point("Point on curve for kink").step()
            }
            (CurveUtil::InsertKink, Input::Point(p)) => {
                let Some((id, _)) = self.pick else { return Step::Cancelled };
                let Some(mut c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let t = c.closest_parameter(p);
                let mult = c.knots.iter().filter(|x| (*x - t).abs() < 1e-12).count();
                if mult < c.degree {
                    c.insert_knot(t, c.degree - mult);
                }
                ctx.doc.begin_undo("InsertKink");
                ctx.doc.replace_geometry(id, Geometry::Curve(c));
                Step::Done
            }
            (CurveUtil::RemoveKnot, Input::Pick(id, p)) => {
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let t = c.closest_parameter(p);
                let deg = c.degree;
                let interior: Vec<f64> = c.knots[deg + 1..c.knots.len() - deg - 1].to_vec();
                let Some(k) = interior.iter().min_by(|a, b| (*a - t).abs().partial_cmp(&(*b - t).abs()).unwrap()).copied() else {
                    ctx.log("The curve has no interior knots.");
                    return Step::Cancelled;
                };
                // rebuild with the same point count minus one using least squares
                let n = c.control_points.len().saturating_sub(1).max(deg + 1);
                match c.rebuild(n, deg) {
                    Ok(nc) => {
                        ctx.doc.begin_undo("RemoveKnot");
                        ctx.doc.replace_geometry(id, Geometry::Curve(nc));
                        ctx.log(format!("Knot at {} removed (curve refit).", fmt_num(k)));
                        Step::Done
                    }
                    Err(e) => {
                        ctx.log(format!("RemoveKnot failed: {e}"));
                        Step::Cancelled
                    }
                }
            }
            (CurveUtil::ClosestPt, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point("Point to measure from").step()
            }
            (CurveUtil::ClosestPt, Input::Point(p)) => {
                let mut best: Option<DVec3> = None;
                for id in &self.ids {
                    let Some(o) = ctx.doc.get(*id) else { continue };
                    let q = match &o.geometry {
                        Geometry::Point(q) => *q,
                        Geometry::Curve(c) => c.point_at(c.closest_parameter(p)),
                        Geometry::Surface(s) => {
                            let (u, v) = s.closest_uv(p);
                            s.point_at(u, v)
                        }
                        Geometry::Brep(b) => {
                            let fi = super::surface_edit::face_under(&b.faces, p);
                            let (u, v) = b.faces[fi].surface.closest_uv(p);
                            b.faces[fi].surface.point_at(u, v)
                        }
                        Geometry::Mesh(m) => *m.vertices.iter().min_by(|a, b| (**a - p).length().partial_cmp(&(**b - p).length()).unwrap()).unwrap_or(&p),
                        _ => continue,
                    };
                    if best.map_or(true, |b| (q - p).length() < (b - p).length()) {
                        best = Some(q);
                    }
                }
                match best {
                    Some(q) => {
                        ctx.log(format!("Closest point: ({}, {}, {}), distance {}", fmt_num(q.x), fmt_num(q.y), fmt_num(q.z), fmt_num((q - p).length())));
                        add_all(ctx, "ClosestPt", vec![Geometry::Point(q)])
                    }
                    None => Step::Cancelled,
                }
            }
            (CurveUtil::Curvature, Input::Pick(id, _)) => {
                self.pick = Some((id, DVec3::ZERO));
                Prompt::point("Select point on curve for curvature measurement").opts([optv("MarkCurvature", "No")]).step()
            }
            (CurveUtil::Curvature, Input::Point(p)) => {
                let Some((id, _)) = self.pick else { return Step::Cancelled };
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let t = c.closest_parameter(p);
                let (t0, t1) = c.domain();
                let h = (t1 - t0) * 1e-4;
                let d1 = c.derivative_at(t);
                let d2 = (c.derivative_at((t + h).min(t1)) - c.derivative_at((t - h).max(t0))) / (2.0 * h);
                let k = d1.cross(d2).length() / d1.length().powi(3).max(1e-18);
                let r = if k > 1e-12 { 1.0 / k } else { f64::INFINITY };
                ctx.log(format!("Curvature = {}, radius = {}", fmt_num(k), if r.is_finite() { fmt_num(r) } else { "infinite".into() }));
                if r.is_finite() {
                    let n = (d2 - d1 * (d2.dot(d1) / d1.length_squared())).normalize_or(DVec3::Y);
                    let center = c.point_at(t) + n * r;
                    let plane = Plane::from_frame(center, n, d1);
                    if let Ok(circle) = NurbsCurve::circle(&plane, r) {
                        return add_all(ctx, "Curvature", vec![Geometry::Curve(circle)]);
                    }
                }
                Step::Done
            }
            (CurveUtil::TweenCurves, Input::Objects(ids)) => {
                let n = ctx.mem_num("TweenCurves.Number", 1.0) as usize;
                let (Some(a), Some(b)) = (ids.first().and_then(|i| curve_of(ctx.doc, *i)), ids.get(1).and_then(|i| curve_of(ctx.doc, *i))) else { return Step::Cancelled };
                let Ok(compat) = tri_geom::surface::make_compatible(&[a, b]) else { return Step::Cancelled };
                let mut out = Vec::new();
                for i in 1..=n.max(1) {
                    let f = i as f64 / (n.max(1) + 1) as f64;
                    let cps: Vec<DVec3> = compat[0].control_points.iter().zip(&compat[1].control_points).map(|(p, q)| *p * (1.0 - f) + *q * f).collect();
                    let ws: Vec<f64> = compat[0].weights.iter().zip(&compat[1].weights).map(|(p, q)| p * (1.0 - f) + q * f).collect();
                    if let Ok(c) = NurbsCurve::new(compat[0].degree, cps, ws, compat[0].knots.clone()) {
                        out.push(Geometry::Curve(c));
                    }
                }
                add_all(ctx, "TweenCurves", out)
            }
            (CurveUtil::TweenCurves, Input::Option(_)) => Prompt::number("Number of tween curves").default_num(ctx.mem_num("TweenCurves.Number", 1.0)).step(),
            (CurveUtil::TweenCurves, Input::Number(v)) => {
                ctx.mem_set("TweenCurves.Number", (v.round() as usize).clamp(1, 100));
                Prompt::objects("Select two curves", ObjFilter::Curve, 2).step()
            }
            (CurveUtil::LineThroughPt | CurveUtil::PlaneThroughPt, Input::Objects(ids)) => {
                let pts: Vec<DVec3> = ids.iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p) } else { None }).collect();
                if pts.len() < 2 {
                    return Step::Cancelled;
                }
                let c = pts.iter().fold(DVec3::ZERO, |a, p| a + *p) / pts.len() as f64;
                let (dir, normal) = principal_axes(&pts, c);
                if matches!(self.op, CurveUtil::LineThroughPt) {
                    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                    for p in &pts {
                        let t = (*p - c).dot(dir);
                        lo = lo.min(t);
                        hi = hi.max(t);
                    }
                    add_all(ctx, "LineThroughPt", NurbsCurve::line(c + dir * lo, c + dir * hi).ok().map(Geometry::Curve).into_iter().collect())
                } else {
                    let plane = Plane::from_frame(c, dir, normal.cross(dir));
                    let (mut umin, mut umax, mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
                    for p in &pts {
                        let (u, v, _) = plane.closest_parameters(*p);
                        umin = umin.min(u);
                        umax = umax.max(u);
                        vmin = vmin.min(v);
                        vmax = vmax.max(v);
                    }
                    add_all(ctx, "PlaneThroughPt", NurbsSurface::plane(&plane, umin, umax, vmin, vmax).ok().map(Geometry::Surface).into_iter().collect())
                }
            }
            (CurveUtil::ExtractControlPolygon, Input::Objects(ids)) => {
                let mut out = Vec::new();
                for id in ids {
                    match ctx.doc.get(id).map(|o| &o.geometry) {
                        Some(Geometry::Curve(c)) => {
                            if let Ok(p) = NurbsCurve::polyline(c.control_points.clone()) {
                                out.push(Geometry::Curve(p));
                            }
                        }
                        Some(Geometry::Surface(s)) => {
                            for i in 0..s.count_u {
                                let row: Vec<DVec3> = (0..s.count_v).map(|j| s.control_points[i * s.count_v + j]).collect();
                                if let Ok(p) = NurbsCurve::polyline(row) {
                                    out.push(Geometry::Curve(p));
                                }
                            }
                            for j in 0..s.count_v {
                                let col: Vec<DVec3> = (0..s.count_u).map(|i| s.control_points[i * s.count_v + j]).collect();
                                if let Ok(p) = NurbsCurve::polyline(col) {
                                    out.push(Geometry::Curve(p));
                                }
                            }
                        }
                        _ => {}
                    }
                }
                add_all(ctx, "ExtractControlPolygon", out)
            }
            (CurveUtil::MatchCrvDir, Input::Pick(id, _)) if self.pick.is_none() => {
                self.pick = Some((id, DVec3::ZERO));
                ctx.doc.clear_selection();
                Prompt::objects("Select curves to change direction", ObjFilter::Curve, 1).step()
            }
            (CurveUtil::MatchCrvDir, Input::Objects(ids)) => {
                let Some(reference) = self.pick.and_then(|(id, _)| curve_of(ctx.doc, id)) else { return Step::Cancelled };
                let rd = (reference.end_point() - reference.start_point()).normalize_or(reference.tangent_at(reference.domain().0));
                ctx.doc.begin_undo("MatchCrvDir");
                for id in ids {
                    let Some(mut c) = curve_of(ctx.doc, id) else { continue };
                    let d = (c.end_point() - c.start_point()).normalize_or(c.tangent_at(c.domain().0));
                    if d.dot(rd) < 0.0 {
                        c.reverse();
                        ctx.doc.replace_geometry(id, Geometry::Curve(c));
                    }
                }
                Step::Done
            }
            (_, Input::Option(_)) => self.start(ctx),
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

/// Principal direction (largest spread) and normal (smallest spread) of a point set.
pub fn principal_axes(pts: &[DVec3], c: DVec3) -> (DVec3, DVec3) {
    let mut cov = glam::DMat3::ZERO;
    for p in pts {
        let d = *p - c;
        cov += glam::DMat3::from_cols(d * d.x, d * d.y, d * d.z);
    }
    // power iteration for the largest eigenvector, then deflate for the smallest
    let mut v = DVec3::new(1.0, 0.7, 0.3).normalize();
    for _ in 0..64 {
        let nv = cov * v;
        if nv.length() < 1e-18 {
            break;
        }
        v = nv.normalize();
    }
    let dir = v;
    let l1 = (cov * dir).dot(dir);
    let deflated = cov - glam::DMat3::from_cols(dir * dir.x * l1, dir * dir.y * l1, dir * dir.z * l1);
    let mut w = dir.any_orthonormal_vector();
    for _ in 0..64 {
        let nw = deflated * w;
        if nw.length() < 1e-18 {
            break;
        }
        w = nw.normalize();
    }
    let second = w;
    let normal = dir.cross(second).normalize_or(DVec3::Z);
    (dir, normal)
}

// ---- points / grids ----------------------------------------------------------------------------------

pub enum GridKind {
    PointGrid,
    SrfPtGrid,
    SrfControlPtGrid,
    ExtractIsocurve,
    ExtractWireframe,
}

pub struct GridMakeCmd {
    kind: GridKind,
    pts: Vec<DVec3>,
    nums: Vec<f64>,
    ids: Vec<ObjectId>,
    pick: Option<(ObjectId, DVec3)>,
    direction: &'static str,
    plane: Plane,
}

impl GridMakeCmd {
    pub fn new(kind: GridKind) -> Self {
        Self { kind, pts: Vec::new(), nums: Vec::new(), ids: Vec::new(), pick: None, direction: "U", plane: Plane::world_xy() }
    }
}

impl Command for GridMakeCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        match self.kind {
            GridKind::PointGrid => Prompt::point("First corner of grid").step(),
            GridKind::SrfPtGrid | GridKind::SrfControlPtGrid => Prompt::objects("Select surfaces", ObjFilter::Surface, 1).step(),
            GridKind::ExtractIsocurve => Prompt::pick("Select surface for isocurve extraction", ObjFilter::Surface).step(),
            GridKind::ExtractWireframe => Prompt::objects("Select surfaces and polysurfaces", ObjFilter::Surface, 1).step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.kind, input) {
            (GridKind::PointGrid, Input::Point(p)) => {
                self.pts.push(p);
                if self.pts.len() < 2 {
                    return Prompt::point("Other corner of grid").step();
                }
                Prompt::number("Number of points in X").default_num(ctx.mem_num("PointGrid.X", 5.0)).step()
            }
            (GridKind::PointGrid, Input::Number(n)) => {
                self.nums.push(n.round().max(1.0));
                if self.nums.len() < 2 {
                    ctx.mem_set("PointGrid.X", fmt_num(self.nums[0]));
                    return Prompt::number("Number of points in Y").default_num(ctx.mem_num("PointGrid.Y", 5.0)).step();
                }
                ctx.mem_set("PointGrid.Y", fmt_num(self.nums[1]));
                let (nx, ny) = (self.nums[0] as usize, self.nums[1] as usize);
                let (u0, v0, w) = self.plane.closest_parameters(self.pts[0]);
                let (u1, v1, _) = self.plane.closest_parameters(self.pts[1]);
                let mut out = Vec::new();
                for i in 0..nx {
                    for j in 0..ny {
                        let fu = if nx > 1 { i as f64 / (nx - 1) as f64 } else { 0.0 };
                        let fv = if ny > 1 { j as f64 / (ny - 1) as f64 } else { 0.0 };
                        out.push(Geometry::Point(self.plane.point_at_3(u0 + (u1 - u0) * fu, v0 + (v1 - v0) * fv, w)));
                    }
                }
                add_all(ctx, "PointGrid", out)
            }
            (GridKind::SrfPtGrid, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::number("Number of points in U").default_num(10.0).step()
            }
            (GridKind::SrfPtGrid, Input::Number(n)) => {
                self.nums.push(n.round().max(2.0));
                if self.nums.len() < 2 {
                    return Prompt::number("Number of points in V").default_num(10.0).step();
                }
                let (nu, nv) = (self.nums[0] as usize, self.nums[1] as usize);
                let mut out = Vec::new();
                for id in &self.ids {
                    let faces = super::surface_edit::faces_of(ctx.doc, *id).unwrap_or_default();
                    for f in faces {
                        let (u0, u1) = f.surface.domain_u();
                        let (v0, v1) = f.surface.domain_v();
                        for i in 0..nu {
                            for j in 0..nv {
                                let u = u0 + (u1 - u0) * i as f64 / (nu - 1) as f64;
                                let v = v0 + (v1 - v0) * j as f64 / (nv - 1) as f64;
                                if f.contains_uv(glam::DVec2::new(u, v)) {
                                    out.push(Geometry::Point(f.surface.point_at(u, v)));
                                }
                            }
                        }
                    }
                }
                add_all(ctx, "SrfPtGrid", out)
            }
            (GridKind::SrfControlPtGrid, Input::Objects(ids)) => {
                let mut out = Vec::new();
                for id in ids {
                    for f in super::surface_edit::faces_of(ctx.doc, id).unwrap_or_default() {
                        out.extend(f.surface.control_points.iter().map(|p| Geometry::Point(*p)));
                    }
                }
                add_all(ctx, "SrfControlPtGrid", out)
            }
            (GridKind::ExtractIsocurve, Input::Pick(id, p)) => {
                self.pick = Some((id, p));
                Prompt::point("Select point on surface for isocurve").opts([optv("Direction", self.direction), optv("Toggle", "No")]).step()
            }
            (GridKind::ExtractIsocurve, Input::Option(o)) => {
                if o == "Direction" || o == "Toggle" {
                    self.direction = match self.direction { "U" => "V", "V" => "Both", _ => "U" };
                }
                Prompt::point("Select point on surface for isocurve").opts([optv("Direction", self.direction)]).step()
            }
            (GridKind::ExtractIsocurve, Input::Point(p)) => {
                let Some((id, _)) = self.pick else { return Step::Cancelled };
                let Some(faces) = super::surface_edit::faces_of(ctx.doc, id) else { return Step::Cancelled };
                let fi = super::surface_edit::face_under(&faces, p);
                let s = &faces[fi].surface;
                let (u, v) = s.closest_uv(p);
                let mut out = Vec::new();
                if self.direction != "V" {
                    out.push(Geometry::Curve(s.isocurve_u(u)));
                }
                if self.direction != "U" {
                    out.push(Geometry::Curve(s.isocurve_v(v)));
                }
                add_all(ctx, "ExtractIsocurve", out)
            }
            (GridKind::ExtractWireframe, Input::Objects(ids)) => {
                let mut out = Vec::new();
                for id in ids {
                    for f in super::surface_edit::faces_of(ctx.doc, id).unwrap_or_default() {
                        let tol = f.bounding_box().diagonal().max(1e-6) * 1e-3;
                        out.extend(f.edge_curves(tol).into_iter().map(Geometry::Curve));
                        if !f.is_trimmed() {
                            out.extend(f.surface.display_isocurves(4).into_iter().map(Geometry::Curve));
                        }
                    }
                }
                add_all(ctx, "ExtractWireframe", out)
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (&self.kind, self.pts.first(), cursor) {
            (GridKind::PointGrid, Some(a), Some(c)) if self.pts.len() == 1 => vec![Preview::Polyline(super::create::rect_points(&self.plane, *a, c))],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

// ---- surface / mesh utilities -------------------------------------------------------------------

pub enum SrfUtil {
    UntrimAll,
    UntrimBorder,
    UntrimHoles,
    ShrinkTrimmedSrf,
    TriangulateMesh,
    UnifyMeshNormals,
    OffsetMesh,
    ExtrudeMesh,
    PlanarMesh,
    MeshFromPoints3DFace,
}

pub struct SrfUtilCmd {
    op: SrfUtil,
    ids: Vec<ObjectId>,
    pts: Vec<DVec3>,
}

impl SrfUtilCmd {
    pub fn new(op: SrfUtil) -> Self {
        Self { op, ids: Vec::new(), pts: Vec::new() }
    }
}

impl Command for SrfUtilCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        match self.op {
            SrfUtil::UntrimAll | SrfUtil::UntrimBorder | SrfUtil::UntrimHoles => Prompt::objects("Select surfaces to untrim", ObjFilter::Surface, 1).step(),
            SrfUtil::ShrinkTrimmedSrf => Prompt::objects("Select trimmed surfaces to shrink", ObjFilter::Surface, 1).step(),
            SrfUtil::TriangulateMesh | SrfUtil::UnifyMeshNormals => Prompt::objects("Select meshes", ObjFilter::Mesh, 1).step(),
            SrfUtil::OffsetMesh => Prompt::objects("Select meshes to offset", ObjFilter::Mesh, 1).step(),
            SrfUtil::ExtrudeMesh => Prompt::objects("Select meshes to extrude", ObjFilter::Mesh, 1).step(),
            SrfUtil::PlanarMesh => Prompt::objects("Select closed planar curves", ObjFilter::Curve, 1).step(),
            SrfUtil::MeshFromPoints3DFace => Prompt::point("First corner").step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.op, input) {
            (SrfUtil::UntrimAll | SrfUtil::UntrimBorder | SrfUtil::UntrimHoles, Input::Objects(ids)) => {
                ctx.doc.begin_undo("Untrim");
                for id in ids {
                    let Some(Geometry::Brep(b)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
                    let faces: Vec<Face> = b
                        .faces
                        .iter()
                        .map(|f| match self.op {
                            SrfUtil::UntrimHoles => Face { surface: f.surface.clone(), loops: f.loops.iter().take(1).cloned().collect() },
                            _ => Face::untrimmed(f.surface.clone()),
                        })
                        .collect();
                    if faces.len() == 1 && !faces[0].is_trimmed() {
                        ctx.doc.replace_geometry(id, Geometry::Surface(faces[0].surface.clone()));
                    } else {
                        ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(faces)));
                    }
                }
                Step::Done
            }
            (SrfUtil::ShrinkTrimmedSrf, Input::Objects(_)) => {
                ctx.log("Surfaces are already as small as their trims in triceratops (nothing to shrink).");
                Step::Done
            }
            (SrfUtil::TriangulateMesh, Input::Objects(_)) => {
                ctx.log("Meshes are already triangulated in triceratops.");
                Step::Done
            }
            (SrfUtil::UnifyMeshNormals, Input::Objects(ids)) => {
                ctx.doc.begin_undo("UnifyMeshNormals");
                for id in ids {
                    if let Some(Geometry::Mesh(mut m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                        m.compute_normals();
                        // flip if the mesh is closed and normals point inward
                        if m.is_closed() {
                            let c = m.bounding_box().center();
                            let (a, b, cc) = m.triangle(0);
                            let n = (b - a).cross(cc - a);
                            if n.dot(a - c) < 0.0 {
                                m.flip();
                            }
                        }
                        ctx.doc.replace_geometry(id, Geometry::Mesh(m));
                    }
                }
                Step::Done
            }
            (SrfUtil::OffsetMesh | SrfUtil::ExtrudeMesh, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::number(if matches!(self.op, SrfUtil::OffsetMesh) { "Offset distance" } else { "Extrusion distance" }).default_num(1.0).opts([optv("Solidify", "No")]).step()
            }
            (SrfUtil::OffsetMesh | SrfUtil::ExtrudeMesh, Input::Number(d)) => {
                let offset = matches!(self.op, SrfUtil::OffsetMesh);
                ctx.doc.begin_undo(if offset { "OffsetMesh" } else { "ExtrudeMesh" });
                let mut out = Vec::new();
                for id in &self.ids {
                    let Some(Geometry::Mesh(m)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { continue };
                    let m = super::mesh_cmds::weld(&m, 1e-9 * (1.0 + m.bounding_box().diagonal()));
                    let dir = ctx.cplane.z_axis;
                    let mut top = m.clone();
                    for (i, v) in top.vertices.iter_mut().enumerate() {
                        *v += if offset { m.normals.get(i).copied().unwrap_or(dir) * d } else { dir * d };
                    }
                    if offset {
                        out.push(Geometry::Mesh(top));
                        continue;
                    }
                    // extrusion: bottom (flipped) + top + side quads along naked edges
                    let mut result = m.clone();
                    result.flip();
                    let base = result.vertices.len() as u32;
                    result.append(&top);
                    let mut edge_count: std::collections::HashMap<(u32, u32), i32> = Default::default();
                    for t in m.indices.chunks(3) {
                        for k in 0..3 {
                            let (a, b) = (t[k], t[(k + 1) % 3]);
                            *edge_count.entry((a.min(b), a.max(b))).or_default() += if a < b { 1 } else { -1 };
                            *edge_count.entry((a.min(b), a.max(b))).or_default() += 0;
                        }
                    }
                    let mut naked: Vec<(u32, u32)> = Vec::new();
                    let mut seen: std::collections::HashMap<(u32, u32), usize> = Default::default();
                    for t in m.indices.chunks(3) {
                        for k in 0..3 {
                            let (a, b) = (t[k], t[(k + 1) % 3]);
                            *seen.entry((a.min(b), a.max(b))).or_default() += 1;
                        }
                    }
                    for t in m.indices.chunks(3) {
                        for k in 0..3 {
                            let (a, b) = (t[k], t[(k + 1) % 3]);
                            if seen.get(&(a.min(b), a.max(b))).copied().unwrap_or(0) == 1 {
                                naked.push((a, b));
                            }
                        }
                    }
                    for (a, b) in naked {
                        result.indices.extend_from_slice(&[a, b, base + b, a, base + b, base + a]);
                    }
                    result.compute_normals();
                    out.push(Geometry::Mesh(result));
                }
                for id in &self.ids {
                    ctx.doc.remove(*id);
                }
                add_all(ctx, if offset { "OffsetMesh" } else { "ExtrudeMesh" }, out)
            }
            (SrfUtil::PlanarMesh, Input::Objects(ids)) => {
                let curves: Vec<NurbsCurve> = ids.iter().filter_map(|id| curve_of(ctx.doc, *id)).collect();
                let faces = super::create::planar_faces(&curves);
                let out: Vec<Geometry> = faces.iter().map(|f| Geometry::Mesh(f.tessellate(1, 1))).collect();
                add_all(ctx, "PlanarMesh", out)
            }
            (SrfUtil::MeshFromPoints3DFace, Input::Point(p)) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("Second corner").step(),
                    2 => Prompt::point("Third corner").step(),
                    3 => Prompt::point("Fourth corner. Press Enter for a triangle").step(),
                    _ => self.input(Input::Empty, ctx),
                }
            }
            (SrfUtil::MeshFromPoints3DFace, Input::Empty) if self.pts.len() >= 3 => {
                let mut m = Mesh::new();
                m.vertices = self.pts.clone();
                m.indices = vec![0, 1, 2];
                if self.pts.len() == 4 {
                    m.indices.extend_from_slice(&[0, 2, 3]);
                }
                m.compute_normals();
                add_all(ctx, "3DFace", vec![Geometry::Mesh(m)])
            }
            (_, Input::Option(_)) => self.start(ctx),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        if matches!(self.op, SrfUtil::MeshFromPoints3DFace) && !self.pts.is_empty() {
            let mut pts = self.pts.clone();
            if let Some(c) = cursor {
                pts.push(c);
            }
            pts.push(self.pts[0]);
            return vec![Preview::Polyline(pts)];
        }
        vec![]
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

// ---- deformations: ScaleNU, Twist, Taper ----------------------------------------------------------

pub fn deform_pub(g: &mut Geometry, f: &dyn Fn(DVec3) -> DVec3) {
    deform_geometry(g, f)
}

fn deform_geometry(g: &mut Geometry, f: &dyn Fn(DVec3) -> DVec3) {
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

pub struct ScaleNUCmd {
    ids: Vec<ObjectId>,
    origin: Option<DVec3>,
    factors: Vec<f64>,
}

impl Default for ScaleNUCmd {
    fn default() -> Self {
        Self { ids: Vec::new(), origin: None, factors: Vec::new() }
    }
}

impl Command for ScaleNUCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to scale", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.origin, input) {
            (_, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point("Origin point").opts([optv("Copy", "No")]).step()
            }
            (None, Input::Point(p)) => {
                self.origin = Some(p);
                Prompt::number("X scale factor").default_num(1.0).step()
            }
            (Some(o), Input::Number(f)) => {
                self.factors.push(f);
                match self.factors.len() {
                    1 => Prompt::number("Y scale factor").default_num(1.0).step(),
                    2 => Prompt::number("Z scale factor").default_num(1.0).step(),
                    _ => {
                        let cp = ctx.cplane;
                        let (sx, sy, sz) = (self.factors[0], self.factors[1], self.factors[2]);
                        ctx.doc.begin_undo("ScaleNU");
                        for id in &self.ids {
                            let Some(mut g) = ctx.doc.get(*id).map(|ob| ob.geometry.clone()) else { continue };
                            deform_geometry(&mut g, &|p| {
                                let d = p - o;
                                o + cp.x_axis * (d.dot(cp.x_axis) * sx) + cp.y_axis * (d.dot(cp.y_axis) * sy) + cp.z_axis * (d.dot(cp.z_axis) * sz)
                            });
                            ctx.doc.replace_geometry(*id, g);
                        }
                        Step::Done
                    }
                }
            }
            (_, Input::Option(_)) => Prompt::point("Origin point").step(),
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.origin
    }
}

pub struct TwistTaperCmd {
    twist: bool,
    ids: Vec<ObjectId>,
    axis: Vec<DVec3>,
}

impl TwistTaperCmd {
    pub fn new(twist: bool) -> Self {
        Self { twist, ids: Vec::new(), axis: Vec::new() }
    }
}

impl Command for TwistTaperCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects(if self.twist { "Select objects to twist" } else { "Select objects to taper" }, ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::point(if self.twist { "Start of twist axis" } else { "Start of taper axis" }).step()
            }
            Input::Point(p) => {
                self.axis.push(p);
                if self.axis.len() < 2 {
                    return Prompt::point(if self.twist { "End of twist axis" } else { "End of taper axis" }).step();
                }
                if self.twist {
                    Prompt::number("Twist angle").default_num(ctx.mem_num("Twist.Angle", 90.0)).opts([optv("Infinite", "No"), optv("Rigid", "No")]).step()
                } else {
                    Prompt::number("End distance (scale at the end of the axis)").default_num(ctx.mem_num("Taper.End", 0.5)).opts([optv("Flat", "No"), optv("Infinite", "No")]).step()
                }
            }
            Input::Number(v) => {
                let (a, b) = (self.axis[0], self.axis[1]);
                let axis = (b - a).normalize_or(DVec3::Z);
                let len = (b - a).length().max(1e-9);
                ctx.mem_set(if self.twist { "Twist.Angle" } else { "Taper.End" }, fmt_num(v));
                let twist = self.twist;
                ctx.doc.begin_undo(if twist { "Twist" } else { "Taper" });
                for id in &self.ids {
                    let Some(mut g) = ctx.doc.get(*id).map(|ob| ob.geometry.clone()) else { continue };
                    deform_geometry(&mut g, &|p| {
                        let t = ((p - a).dot(axis) / len).clamp(0.0, 1.0);
                        let radial = (p - a) - axis * (p - a).dot(axis);
                        if twist {
                            let q = DQuat::from_axis_angle(axis, v.to_radians() * t);
                            a + axis * (p - a).dot(axis) + q * radial
                        } else {
                            let s = 1.0 + (v - 1.0) * t;
                            a + axis * (p - a).dot(axis) + radial * s
                        }
                    });
                    ctx.doc.replace_geometry(*id, g);
                }
                Step::Done
            }
            Input::Option(_) => Prompt::number("Value").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.axis.first(), cursor) {
            (Some(a), Some(c)) if self.axis.len() == 1 => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.axis.last().copied()
    }
}

// ---- Ribbon ---------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct RibbonCmd {
    ids: Vec<ObjectId>,
}

impl Command for RibbonCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves for ribbon", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::number("Offset distance").default_num(ctx.mem_num("Ribbon.Distance", 1.0)).step()
            }
            Input::Number(d) => {
                ctx.mem_set("Ribbon.Distance", fmt_num(d));
                let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
                let mut out = Vec::new();
                for id in &self.ids {
                    let Some(c) = curve_of(ctx.doc, *id) else { continue };
                    let plane = curve_plane(&c).unwrap_or(ctx.cplane);
                    let Ok(o) = c.offset(&plane, d, tol) else { continue };
                    if let Ok(s) = NurbsSurface::loft(&[c, o], 1) {
                        out.push(Geometry::Surface(s));
                    }
                }
                add_all(ctx, "Ribbon", out)
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- dimensions: DimRadius, DimDiameter, DimAngle, Centermark, DimCurveLength ------------------------

pub enum DimKind {
    Radius,
    Diameter,
    Angle,
    Centermark,
    CurveLength,
}

pub struct Dim2Cmd {
    kind: DimKind,
    pick: Option<(ObjectId, DVec3)>,
    pts: Vec<DVec3>,
}

impl Dim2Cmd {
    pub fn new(kind: DimKind) -> Self {
        Self { kind, pick: None, pts: Vec::new() }
    }
}

fn add_dim_group(ctx: &mut CmdCtx, label: &str, curves: Vec<NurbsCurve>) -> Step {
    let n = ctx.memory.get("GroupCounter").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) + 1;
    ctx.mem_set("GroupCounter", n);
    let group = format!("{label}{n:02}");
    ctx.doc.begin_undo(label);
    ctx.doc.clear_selection();
    for c in curves {
        let mut a = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
        a.groups.push(group.clone());
        let id = ctx.doc.add_with(Geometry::Curve(c), a);
        ctx.doc.select(id, true);
    }
    Step::Done
}

impl Command for Dim2Cmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        match self.kind {
            DimKind::Radius => Prompt::pick("Select curve for radius dimension", ObjFilter::Curve).step(),
            DimKind::Diameter => Prompt::pick("Select curve for diameter dimension", ObjFilter::Curve).step(),
            DimKind::Angle => Prompt::point("Vertex of angle").opts([opt("Curve")]).step(),
            DimKind::Centermark => Prompt::objects("Select curves for center marks", ObjFilter::Curve, 1).step(),
            DimKind::CurveLength => Prompt::pick("Select curve to dimension", ObjFilter::Curve).step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let h = ctx.mem_num("Dim.TextHeight", 1.0);
        match (&self.kind, input) {
            (DimKind::Radius | DimKind::Diameter | DimKind::CurveLength, Input::Pick(id, p)) => {
                self.pick = Some((id, p));
                Prompt::point("Dimension location").step()
            }
            (DimKind::Radius | DimKind::Diameter, Input::Point(loc)) => {
                let Some((id, pick)) = self.pick else { return Step::Cancelled };
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let _ = pick;
                let (t0, t1) = c.domain();
                let span = t1 - t0;
                let (a, b, cc) = (c.point_at(t0 + span * 0.15), c.point_at(t0 + span * 0.5), c.point_at(t0 + span * 0.85));
                let Ok((center, r, _)) = tri_geom::curve::circle_through(a, b, cc) else {
                    ctx.log("The curve has no radius at the picked point.");
                    return Step::Cancelled;
                };
                let diameter = matches!(self.kind, DimKind::Diameter);
                let on = center + (loc - center).normalize_or(DVec3::X) * r;
                let mut curves = Vec::new();
                let start = if diameter { center - (on - center) } else { center };
                if let Ok(l) = NurbsCurve::polyline(vec![start, on, loc]) {
                    curves.push(l);
                }
                let d = (on - loc).normalize_or(DVec3::X);
                let n = ctx.cplane.z_axis.cross(d);
                if let Ok(ah) = NurbsCurve::polyline(vec![on - d * (h * 0.6) + n * (h * 0.2), on, on - d * (h * 0.6) - n * (h * 0.2)]) {
                    curves.push(ah);
                }
                let label = if diameter { format!("%%c{}", fmt_num(2.0 * r)) } else { format!("R{}", fmt_num(r)) };
                curves.extend(text_curves(&label, &ctx.cplane, loc + ctx.cplane.x_axis * (h * 0.3), h, 0.0));
                add_dim_group(ctx, if diameter { "DimDiameter" } else { "DimRadius" }, curves)
            }
            (DimKind::CurveLength, Input::Point(loc)) => {
                let Some((id, _)) = self.pick else { return Step::Cancelled };
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let mid = c.point_at((c.domain().0 + c.domain().1) * 0.5);
                let mut curves = Vec::new();
                if let Ok(l) = NurbsCurve::line(mid, loc) {
                    curves.push(l);
                }
                curves.extend(text_curves(&fmt_num(c.length()), &ctx.cplane, loc + ctx.cplane.x_axis * (h * 0.3), h, 0.0));
                add_dim_group(ctx, "DimCurveLength", curves)
            }
            (DimKind::Angle, Input::Point(p)) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("First angle point").step(),
                    2 => Prompt::point("Second angle point").step(),
                    3 => Prompt::point("Dimension arc location").step(),
                    _ => {
                        let (v, a, b, loc) = (self.pts[0], self.pts[1], self.pts[2], self.pts[3]);
                        let r = (loc - v).length().max(1e-6);
                        let da = (a - v).normalize_or(DVec3::X);
                        let db = (b - v).normalize_or(DVec3::Y);
                        let normal = da.cross(db).normalize_or(ctx.cplane.z_axis);
                        let ang = da.dot(db).clamp(-1.0, 1.0).acos();
                        let plane = Plane { origin: v, x_axis: da, y_axis: normal.cross(da).normalize(), z_axis: normal };
                        let mut curves = Vec::new();
                        if let Ok(arc) = NurbsCurve::arc(&plane, r, 0.0, ang) {
                            curves.push(arc);
                        }
                        for d in [da, db] {
                            if let Ok(l) = NurbsCurve::line(v, v + d * (r + h * 0.5)) {
                                curves.push(l);
                            }
                        }
                        let mid = v + DQuat::from_axis_angle(normal, ang * 0.5) * da * (r + h * 0.5);
                        curves.extend(text_curves(&format!("{}%%d", fmt_num(ang.to_degrees())), &ctx.cplane, mid, h, 0.0));
                        add_dim_group(ctx, "DimAngle", curves)
                    }
                }
            }
            (DimKind::Angle, Input::Option(_)) => Prompt::point("Vertex of angle").step(),
            (DimKind::Centermark, Input::Objects(ids)) => {
                let mut curves = Vec::new();
                for id in ids {
                    let Some(c) = curve_of(ctx.doc, id) else { continue };
                    let (t0, t1) = c.domain();
                    let pts = [c.point_at(t0), c.point_at(t0 + (t1 - t0) / 3.0), c.point_at(t0 + (t1 - t0) * 2.0 / 3.0)];
                    let Ok((center, r, plane)) = tri_geom::curve::circle_through(pts[0], pts[1], pts[2]) else { continue };
                    let s = (r * 0.15).max(h * 0.5);
                    for d in [plane.x_axis, plane.y_axis] {
                        if let Ok(l) = NurbsCurve::line(center - d * s, center + d * s) {
                            curves.push(l);
                        }
                    }
                }
                add_dim_group(ctx, "Centermark", curves)
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.last(), cursor) {
            (Some(a), Some(c)) => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

// ---- tools: Calc, Commands, UndoMultiple, ClearUndo, Audit/Check, PolygonCount, SystemInfo, Make2D, CutPlane

pub struct CalcCmd;

impl Command for CalcCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::text("Expression").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Text(t) => {
                match crate::input::calc(&t) {
                    Some(v) => ctx.log(format!("{t} = {}", fmt_num(v))),
                    None => ctx.log(format!("Unable to evaluate \"{t}\".")),
                }
                Prompt::text("Expression").step()
            }
            _ => Step::Done,
        }
    }
}

pub struct UndoMultipleCmd(pub bool);

impl Command for UndoMultipleCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::number(if self.0 { "Number of actions to redo" } else { "Number of actions to undo" }).default_num(1.0).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let n = match input { Input::Number(v) => v.round().max(1.0) as usize, Input::Empty => 1, _ => return Step::Cancelled };
        let mut done = 0;
        for _ in 0..n {
            let r = if self.0 { ctx.doc.redo() } else { ctx.doc.undo() };
            match r {
                Some(label) => {
                    ctx.log(format!("{} {label}", if self.0 { "Redoing" } else { "Undoing" }));
                    done += 1;
                }
                None => break,
            }
        }
        if done == 0 {
            ctx.log(if self.0 { "Nothing to redo." } else { "Nothing to undo." });
        }
        Step::Done
    }
}

pub struct ReportCmd(pub &'static str);

impl Command for ReportCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.0 {
            "Commands" => {
                let names: Vec<&str> = super::registry::all_commands().iter().filter(|c| super::registry::is_implemented(c.name)).map(|c| c.name).collect();
                ctx.log(format!("{} commands implemented: {}", names.len(), names.join(" ")));
            }
            "PolygonCount" => {
                let mut tris = 0usize;
                let mut meshes = 0usize;
                for o in ctx.doc.objects() {
                    if let Geometry::Mesh(m) = &o.geometry {
                        tris += m.triangle_count();
                        meshes += 1;
                    }
                }
                ctx.log(format!("{tris} polygons in {meshes} mesh objects (render meshes of surfaces not counted)."));
            }
            "SystemInfo" => {
                ctx.log(format!("triceratops {} on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH));
                ctx.log(format!("Document: {} objects, {} layers, {} block definitions", ctx.doc.len(), ctx.doc.layers().len(), ctx.doc.state.blocks.len()));
            }
            "ClearUndo" => {
                ctx.doc.clear_undo();
                ctx.log("Undo information cleared.");
            }
            _ => {
                // Audit / Check / SelBadObjects
                let mut bad = Vec::new();
                for o in ctx.doc.objects() {
                    let ok = match &o.geometry {
                        Geometry::Point(p) => p.is_finite(),
                        Geometry::Curve(c) => c.control_points.iter().all(|p| p.is_finite()) && c.knots.windows(2).all(|w| w[0] <= w[1]),
                        Geometry::Surface(s) => s.control_points.iter().all(|p| p.is_finite()),
                        Geometry::Brep(b) => !b.faces.is_empty() && b.faces.iter().all(|f| f.surface.control_points.iter().all(|p| p.is_finite())),
                        Geometry::Mesh(m) => m.indices.iter().all(|i| (*i as usize) < m.vertices.len()) && m.indices.len() % 3 == 0,
                        Geometry::Instance { block, .. } => *block < ctx.doc.state.blocks.len(),
                    };
                    if !ok {
                        bad.push(o.id);
                    }
                }
                if self.0 == "SelBadObjects" {
                    for id in &bad {
                        ctx.doc.select(*id, true);
                    }
                }
                ctx.log(if bad.is_empty() { "No bad objects found.".to_string() } else { format!("{} bad objects found.", bad.len()) });
            }
        }
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub struct Make2DCmd;

impl Command for Make2DCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to draw", ObjFilter::Any, 1).opts([optv("View", "CPlane"), optv("HiddenLines", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                let cp = ctx.cplane;
                let proj = |p: DVec3| cp.origin + (p - cp.origin) - cp.z_axis * (p - cp.origin).dot(cp.z_axis);
                let mut out = Vec::new();
                for id in ids {
                    let Some(o) = ctx.doc.get(id) else { continue };
                    let curves: Vec<NurbsCurve> = match &o.geometry {
                        Geometry::Curve(c) => vec![c.clone()],
                        Geometry::Surface(s) => s.boundary_curves().to_vec(),
                        Geometry::Brep(b) => b.edges(),
                        Geometry::Mesh(m) => m.edges().iter().filter_map(|(a, b)| NurbsCurve::line(m.vertices[*a as usize], m.vertices[*b as usize]).ok()).collect(),
                        _ => Vec::new(),
                    };
                    for mut c in curves {
                        for p in c.control_points.iter_mut() {
                            *p = proj(*p);
                        }
                        out.push(Geometry::Curve(c));
                    }
                }
                ctx.log("Note: triceratops Make2D projects edges without hidden-line removal.");
                add_all(ctx, "Make2D", out)
            }
            Input::Option(_) => Prompt::objects("Select objects to draw", ObjFilter::Any, 1).step(),
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct CutPlaneCmd {
    ids: Vec<ObjectId>,
    pts: Vec<DVec3>,
}

impl Command for CutPlaneCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects for cutting plane", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::point("Start of cut plane").step()
            }
            Input::Point(p) => {
                self.pts.push(p);
                if self.pts.len() < 2 {
                    return Prompt::point("End of cut plane").step();
                }
                let bb = self.ids.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&ctx.doc.object_bbox(o)));
                if bb.is_empty() {
                    return Step::Cancelled;
                }
                let (a, b) = (self.pts[0], self.pts[1]);
                let x = (b - a).normalize_or(DVec3::X);
                let z = ctx.cplane.z_axis;
                let plane = Plane::from_frame(a, x, z);
                let margin = bb.diagonal() * 0.1;
                let (mut umin, mut umax, mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
                for c in bb.corners() {
                    let (u, v, _) = plane.closest_parameters(c);
                    umin = umin.min(u);
                    umax = umax.max(u);
                    vmin = vmin.min(v);
                    vmax = vmax.max(v);
                }
                let (u0, _, _) = plane.closest_parameters(a);
                let (u1, _, _) = plane.closest_parameters(b);
                let s = NurbsSurface::plane(&plane, u0.min(u1).min(umin) - margin, u0.max(u1).max(umax) + margin, vmin - margin, vmax + margin);
                add_all(ctx, "CutPlane", s.ok().map(Geometry::Surface).into_iter().collect())
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.first(), cursor) {
            (Some(a), Some(c)) => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

/// Parabola from vertex, focus direction and end: y = x^2 / (4 f) sampled and interpolated.
#[derive(Default)]
pub struct ParabolaCmd {
    pts: Vec<DVec3>,
}

impl Command for ParabolaCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Vertex of parabola").opts([optv("Mode", "Vertex")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("Focus").step(),
                    2 => Prompt::point("End of parabola").step(),
                    _ => {
                        let (v, f, e) = (self.pts[0], self.pts[1], self.pts[2]);
                        let y = (f - v).normalize_or(DVec3::Y);
                        let fl = (f - v).length().max(1e-9);
                        let n = ctx.cplane.z_axis;
                        let x = y.cross(n).normalize_or(DVec3::X);
                        let xe = (e - v).dot(x);
                        let half = xe.abs().max(1e-6);
                        // exact quadratic Bézier: control points (-h, h²/4f), (0, -h²/4f), (h, h²/4f)
                        let yh = half * half / (4.0 * fl);
                        let cps = vec![v - x * half + y * yh, v - y * yh, v + x * half + y * yh];
                        match NurbsCurve::from_control_points(2, cps) {
                            Ok(c) => add_all(ctx, "Parabola", vec![Geometry::Curve(c)]),
                            Err(_) => Step::Cancelled,
                        }
                    }
                }
            }
            Input::Option(_) => Prompt::point("Vertex of parabola").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.last(), cursor) {
            (Some(a), Some(c)) => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

/// Visibility / lock helpers that act on the selection without prompting.
pub struct SelStateCmd(pub &'static str);

impl Command for SelStateCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let sel = ctx.doc.selected_ids();
        match self.0 {
            "UngroupAll" => {
                ctx.doc.begin_undo("UngroupAll");
                let ids: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).collect();
                for id in ids {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.groups.clear();
                    }
                }
            }
            "UnlockSelected" => {
                ctx.doc.begin_undo("UnlockSelected");
                for id in sel {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.locked = false;
                    }
                }
            }
            "LockSwap" => {
                ctx.doc.begin_undo("LockSwap");
                let ids: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).collect();
                for id in ids {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.locked = !o.attributes.locked;
                    }
                }
            }
            "IsolateLock" => {
                ctx.doc.begin_undo("IsolateLock");
                let ids: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).collect();
                for id in ids {
                    if !sel.contains(&id) {
                        if let Some(o) = ctx.doc.get_mut(id) {
                            o.attributes.locked = true;
                        }
                    }
                }
            }
            "UnisolateLock" => {
                ctx.doc.begin_undo("UnisolateLock");
                let ids: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).collect();
                for id in ids {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.locked = false;
                    }
                }
            }
            "PtOffSelected" | "PointsOff" => ctx.requests.push(AppRequest::PointsOff),
            "SolidPtOn" => ctx.requests.push(AppRequest::PointsOn(sel)),
            _ => {}
        }
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

#[allow(dead_code)]
fn unused(_: &DAffine3) {}
