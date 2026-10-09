//! More surface and solid commands: EdgeSrf, RuledSrf, Sweep1, Sweep2, Pipe,
//! ExtrudeCrvAlongCrv, ExtrudeCrvTapered, ExtrudeCrvToPoint, ExtendSrf, Untrim, NetworkSrf,
//! MeshToNURB, Ellipsoid, Pyramid, TCone, Tube, Slab, BooleanSplit, ExtractSrf.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{mesh_boolean, BooleanOp, Brep, Face, Mesh, NurbsCurve, NurbsSurface, Plane};

use super::create::{planar_cap, plane_at};
use super::surface_edit::faces_of;
use super::{fmt_num, opt, optv, yesno, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn curve_of(doc: &Document, id: ObjectId) -> Option<NurbsCurve> {
    doc.get(id).and_then(|o| o.geometry.as_curve().cloned())
}

fn add_selected(ctx: &mut CmdCtx, label: &str, geoms: Vec<Geometry>) -> Step {
    if geoms.is_empty() {
        ctx.log(format!("{label} failed."));
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

/// Chain curves end to end (reversing as needed) so consecutive ones connect.
fn order_chain(mut curves: Vec<NurbsCurve>) -> Vec<NurbsCurve> {
    if curves.len() < 2 {
        return curves;
    }
    let mut out = vec![curves.remove(0)];
    while !curves.is_empty() {
        let end = out.last().unwrap().end_point();
        let mut best = (0usize, f64::INFINITY, false);
        for (i, c) in curves.iter().enumerate() {
            let d0 = (c.start_point() - end).length();
            let d1 = (c.end_point() - end).length();
            if d0 < best.1 {
                best = (i, d0, false);
            }
            if d1 < best.1 {
                best = (i, d1, true);
            }
        }
        let mut c = curves.remove(best.0);
        if best.2 {
            c.reverse();
        }
        out.push(c);
    }
    out
}

// ---- EdgeSrf / NetworkSrf ---------------------------------------------------------

pub struct EdgeSrfCmd;

impl Command for EdgeSrfCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select 2, 3, or 4 open curves", ObjFilter::Curve, 2).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        let mut curves: Vec<NurbsCurve> = ids.iter().filter_map(|id| curve_of(ctx.doc, *id)).collect();
        if curves.len() == 2 {
            // align the second curve's direction with the first (no bow-tie surfaces)
            let (s0, e0) = (curves[0].start_point(), curves[0].end_point());
            let (s1, e1) = (curves[1].start_point(), curves[1].end_point());
            if (s1 - s0).length() + (e1 - e0).length() > (e1 - s0).length() + (s1 - e0).length() {
                curves[1].reverse();
            }
        }
        let result = match curves.len() {
            2 => NurbsSurface::loft(&curves, 1).map(Geometry::Surface),
            3 | 4 => {
                let chain = order_chain(curves.clone());
                let c0 = chain[0].clone();
                let c1 = chain[1].clone();
                let (c2, c3) = if chain.len() == 4 {
                    (chain[2].clone(), chain[3].clone())
                } else {
                    // three sides: the fourth edge collapses to the shared corner
                    let apex = chain[2].end_point();
                    (chain[2].clone(), NurbsCurve::line(apex, apex + DVec3::new(1e-9, 0.0, 0.0)).unwrap())
                };
                NurbsSurface::coons(&c0, &c1, &c2, &c3, 12).map(Geometry::Surface)
            }
            _ => Err(tri_geom::GeomError::NotEnoughPoints { need: 2, got: curves.len() }),
        };
        match result {
            Ok(g) => add_selected(ctx, "EdgeSrf", vec![g]),
            Err(e) => {
                ctx.log(format!("EdgeSrf failed: {e}"));
                Step::Cancelled
            }
        }
    }
}

// ---- Sweep1 / Sweep2 / Pipe / ExtrudeCrvAlongCrv ----------------------------------------

/// Rotation-minimising frames along a curve at `n` stations: (point, rotation from start frame).
pub fn rail_frames(rail: &NurbsCurve, n: usize) -> Vec<(DVec3, DQuat)> {
    let (t0, t1) = rail.domain();
    let mut out = Vec::with_capacity(n + 1);
    let mut q = DQuat::IDENTITY;
    let mut prev_t = rail.tangent_at(t0);
    for i in 0..=n {
        let t = t0 + (t1 - t0) * i as f64 / n as f64;
        let tan = rail.tangent_at(t);
        if i > 0 {
            q = DQuat::from_rotation_arc(prev_t, tan) * q;
        }
        out.push((rail.point_at(t), q.normalize()));
        prev_t = tan;
    }
    out
}

pub fn sweep_sections_pub(profile: &NurbsCurve, rail: &NurbsCurve, n: usize) -> Vec<NurbsCurve> {
    sweep_sections(profile, rail, n, true)
}

fn sweep_sections(profile: &NurbsCurve, rail: &NurbsCurve, n: usize, rotate: bool) -> Vec<NurbsCurve> {
    let frames = rail_frames(rail, n);
    let origin = rail.start_point();
    frames
        .iter()
        .map(|(p, q)| {
            let xf = if rotate { DAffine3::from_translation(*p) * DAffine3::from_quat(*q) * DAffine3::from_translation(-origin) } else { DAffine3::from_translation(*p - origin) };
            profile.transformed(&xf)
        })
        .collect()
}

pub struct Sweep1Cmd {
    rail: Option<ObjectId>,
    translate_only: bool,
}

impl Sweep1Cmd {
    pub fn new(translate_only: bool) -> Self {
        Self { rail: None, translate_only }
    }
    fn label(&self) -> &'static str {
        if self.translate_only {
            "ExtrudeCrvAlongCrv"
        } else {
            "Sweep1"
        }
    }
}

impl Command for Sweep1Cmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        if self.translate_only {
            Prompt::objects("Select curves to extrude", ObjFilter::Curve, 1).step()
        } else {
            Prompt::pick("Select rail", ObjFilter::Curve).step()
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, _) if self.rail.is_none() && !self.translate_only => {
                self.rail = Some(id);
                ctx.doc.clear_selection();
                Prompt::objects("Select cross section curves", ObjFilter::Curve, 1).opts([optv("Style", "Freeform")]).step()
            }
            Input::Objects(ids) if self.translate_only && self.rail.is_none() => {
                ctx.mem_set("ExtrudeAlong.Profiles", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
                ctx.doc.clear_selection();
                Prompt::pick("Select path curve", ObjFilter::Curve).step()
            }
            Input::Pick(rail_id, _) if self.translate_only => {
                let profiles: Vec<ObjectId> = ctx.memory.get("ExtrudeAlong.Profiles").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                let Some(rail) = curve_of(ctx.doc, rail_id) else { return Step::Cancelled };
                let mut out = Vec::new();
                for pid in profiles {
                    if let Some(p) = curve_of(ctx.doc, pid) {
                        let sections = sweep_sections(&p, &rail, 24, false);
                        if let Ok(s) = NurbsSurface::loft(&sections, 3) {
                            out.push(Geometry::Surface(s));
                        }
                    }
                }
                add_selected(ctx, self.label(), out)
            }
            Input::Objects(ids) => {
                let Some(rail) = self.rail.and_then(|r| curve_of(ctx.doc, r)) else { return Step::Cancelled };
                let mut out = Vec::new();
                for pid in ids {
                    if let Some(p) = curve_of(ctx.doc, pid) {
                        let sections = sweep_sections(&p, &rail, 24, true);
                        if let Ok(s) = NurbsSurface::loft(&sections, 3) {
                            out.push(Geometry::Surface(s));
                        }
                    }
                }
                add_selected(ctx, self.label(), out)
            }
            Input::Option(_) => Prompt::objects("Select cross section curves", ObjFilter::Curve, 1).step(),
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct Sweep2Cmd {
    rails: Vec<ObjectId>,
}

impl Command for Sweep2Cmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select first rail", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, _) => {
                self.rails.push(id);
                if self.rails.len() == 1 {
                    Prompt::pick("Select second rail", ObjFilter::Curve).step()
                } else {
                    ctx.doc.clear_selection();
                    Prompt::objects("Select cross section curves", ObjFilter::Curve, 1).step()
                }
            }
            Input::Objects(ids) => {
                let (Some(r1), Some(r2)) = (curve_of(ctx.doc, self.rails[0]), curve_of(ctx.doc, self.rails[1])) else { return Step::Cancelled };
                let n = 24;
                let mut out = Vec::new();
                for pid in ids {
                    let Some(p) = curve_of(ctx.doc, pid) else { continue };
                    // profile frame: from rail1 start to rail2 start
                    let a0 = r1.start_point();
                    let b0 = r2.start_point();
                    let base_dir = b0 - a0;
                    if base_dir.length() < 1e-9 {
                        continue;
                    }
                    let (t0, t1) = r1.domain();
                    let (s0, s1) = r2.domain();
                    let sections: Vec<NurbsCurve> = (0..=n)
                        .map(|i| {
                            let f = i as f64 / n as f64;
                            let a = r1.point_at(t0 + (t1 - t0) * f);
                            let b = r2.point_at(s0 + (s1 - s0) * f);
                            let d = b - a;
                            let scale = d.length() / base_dir.length();
                            let q = DQuat::from_rotation_arc(base_dir.normalize(), d.normalize_or(base_dir.normalize()));
                            let xf = DAffine3::from_translation(a) * DAffine3::from_quat(q) * DAffine3::from_scale(DVec3::splat(scale.max(1e-9))) * DAffine3::from_translation(-a0);
                            p.transformed(&xf)
                        })
                        .collect();
                    if let Ok(s) = NurbsSurface::loft(&sections, 3) {
                        out.push(Geometry::Surface(s));
                    }
                }
                add_selected(ctx, "Sweep2", out)
            }
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct PipeCmd {
    rail: Option<ObjectId>,
    cap: bool,
}

impl Command for PipeCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.cap = ctx.mem_bool("Pipe.Cap", true);
        Prompt::pick("Select curve to create pipe around", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, _) => {
                self.rail = Some(id);
                Prompt::number("Radius").default_num(ctx.mem_num("Pipe.Radius", 1.0)).opts([optv("Cap", if self.cap { "Flat" } else { "None" }), opt("Thick"), opt("ShapeBlending")]).step()
            }
            Input::Option(o) if o == "Cap" => {
                self.cap = !self.cap;
                ctx.mem_set("Pipe.Cap", yesno(self.cap));
                Prompt::number("Radius").default_num(ctx.mem_num("Pipe.Radius", 1.0)).opts([optv("Cap", if self.cap { "Flat" } else { "None" })]).step()
            }
            Input::Option(_) => Prompt::number("Radius").default_num(ctx.mem_num("Pipe.Radius", 1.0)).step(),
            Input::Number(r) => {
                let Some(rail) = self.rail.and_then(|r| curve_of(ctx.doc, r)) else { return Step::Cancelled };
                ctx.mem_set("Pipe.Radius", fmt_num(r));
                let (t0, _) = rail.domain();
                let plane = Plane::from_normal(rail.start_point(), rail.tangent_at(t0));
                let Ok(circle) = NurbsCurve::circle(&plane, r.abs()) else { return Step::Cancelled };
                let sections = sweep_sections(&circle, &rail, 32, true);
                let Ok(side) = NurbsSurface::loft(&sections, 3) else { return Step::Cancelled };
                let g = if self.cap && !rail.is_closed() {
                    let first = sections.first().and_then(planar_cap);
                    let last = sections.last().and_then(planar_cap);
                    match (first, last) {
                        (Some(a), Some(b)) => {
                            let mut brep = Brep::new(vec![Face::untrimmed(side), a, b]);
                            brep.orient_outward();
                            brep.closed = true;
                            Geometry::Brep(brep)
                        }
                        _ => Geometry::Surface(side),
                    }
                } else {
                    Geometry::Surface(side)
                };
                add_selected(ctx, "Pipe", vec![g])
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- ExtrudeCrvTapered / ExtrudeCrvToPoint ----------------------------------------------

#[derive(Default)]
pub struct ExtrudeTaperedCmd {
    curves: Vec<ObjectId>,
    angle: f64,
}

impl Command for ExtrudeTaperedCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.angle = ctx.mem_num("ExtrudeTapered.Angle", 10.0);
        Prompt::objects("Select curves to extrude", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.curves = ids;
                Prompt::number("Extrusion distance").default_num(ctx.mem_num("ExtrudeTapered.Distance", 1.0)).opts([optv("DraftAngle", fmt_num(self.angle)), optv("Solid", "Yes"), optv("Corners", "Sharp")]).step()
            }
            Input::Option(o) if o == "DraftAngle" => Prompt::number("Draft angle").default_num(self.angle).step(),
            Input::Option(_) => Prompt::number("Extrusion distance").default_num(ctx.mem_num("ExtrudeTapered.Distance", 1.0)).step(),
            Input::Number(v) if ctx.memory.get("ExtrudeTapered.Asking").is_some() => {
                ctx.memory.remove("ExtrudeTapered.Asking");
                self.angle = v;
                Prompt::number("Extrusion distance").default_num(ctx.mem_num("ExtrudeTapered.Distance", 1.0)).step()
            }
            Input::Number(h) => {
                ctx.mem_set("ExtrudeTapered.Distance", fmt_num(h));
                ctx.mem_set("ExtrudeTapered.Angle", fmt_num(self.angle));
                let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
                let cp = ctx.cplane;
                let mut out = Vec::new();
                for id in &self.curves {
                    let Some(c) = curve_of(ctx.doc, *id) else { continue };
                    let plane = tri_geom::curve_plane(&c).unwrap_or(cp);
                    let d = -h * self.angle.to_radians().tan();
                    let Ok(top) = c.offset(&plane, d, tol) else { continue };
                    let top = top.transformed(&DAffine3::from_translation(plane.z_axis * h));
                    let Ok(side) = NurbsSurface::loft(&[c.clone(), top.clone()], 1) else { continue };
                    if c.is_closed() {
                        if let (Some(a), Some(b)) = (planar_cap(&c), planar_cap(&top)) {
                            let mut brep = Brep::new(vec![Face::untrimmed(side), a, b]);
                            brep.orient_outward();
                            brep.closed = true;
                            out.push(Geometry::Brep(brep));
                            continue;
                        }
                    }
                    out.push(Geometry::Surface(side));
                }
                add_selected(ctx, "ExtrudeCrvTapered", out)
            }
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct ExtrudeToPointCmd {
    curves: Vec<ObjectId>,
}

pub fn extrude_to_point(c: &NurbsCurve, apex: DVec3) -> Option<NurbsSurface> {
    let n = c.control_points.len();
    let mut cps = Vec::with_capacity(n * 2);
    let mut ws = Vec::with_capacity(n * 2);
    for (p, w) in c.control_points.iter().zip(&c.weights) {
        cps.push(*p);
        cps.push(apex);
        ws.push(*w);
        ws.push(*w);
    }
    NurbsSurface::new(c.degree, 1, n, 2, cps, ws, c.knots.clone(), vec![0.0, 0.0, 1.0, 1.0]).ok()
}

impl Command for ExtrudeToPointCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves to extrude", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.curves = ids;
                Prompt::point("Point to extrude to").opts([optv("Solid", "Yes")]).step()
            }
            Input::Point(apex) => {
                let mut out = Vec::new();
                for id in &self.curves {
                    let Some(c) = curve_of(ctx.doc, *id) else { continue };
                    let Some(side) = extrude_to_point(&c, apex) else { continue };
                    if c.is_closed() {
                        if let Some(cap) = planar_cap(&c) {
                            let mut brep = Brep::new(vec![Face::untrimmed(side), cap]);
                            brep.orient_outward();
                            brep.closed = true;
                            out.push(Geometry::Brep(brep));
                            continue;
                        }
                    }
                    out.push(Geometry::Surface(side));
                }
                add_selected(ctx, "ExtrudeCrvToPoint", out)
            }
            Input::Option(_) => Prompt::point("Point to extrude to").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let Some(apex) = cursor else { return vec![] };
        self.curves.iter().filter_map(|id| curve_of(doc, *id)).filter_map(|c| extrude_to_point(&c, apex)).map(Preview::Surface).collect()
    }
}

// ---- ExtendSrf / Untrim / MeshToNURB ----------------------------------------------------------

pub struct ExtendSrfCmd;

impl Command for ExtendSrfCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select edge of surface to extend", ObjFilter::Surface).opts([optv("Type", "Smooth")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                ctx.mem_set("ExtendSrf.Id", id);
                ctx.mem_set("ExtendSrf.P", format!("{},{},{}", p.x, p.y, p.z));
                Prompt::number("Extension factor").default_num(ctx.mem_num("ExtendSrf.Length", 1.0)).step()
            }
            Input::Number(len) => {
                let id: ObjectId = ctx.memory.get("ExtendSrf.Id").and_then(|s| s.parse().ok()).unwrap_or(0);
                let p: Vec<f64> = ctx.memory.get("ExtendSrf.P").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                let Some(Geometry::Surface(s)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else {
                    ctx.log("ExtendSrf works on untrimmed surfaces.");
                    return Step::Cancelled;
                };
                let pick = DVec3::new(p[0], p[1], p[2]);
                ctx.mem_set("ExtendSrf.Length", fmt_num(len));
                // which side? compare distances to the four boundary curves
                let (u0, u1) = s.domain_u();
                let (v0, v1) = s.domain_v();
                let sides = [(0usize, s.isocurve_u(u0)), (1, s.isocurve_u(u1)), (2, s.isocurve_v(v0)), (3, s.isocurve_v(v1))];
                let side = sides.iter().min_by(|a, b| {
                    let da = (a.1.point_at(a.1.closest_parameter(pick)) - pick).length();
                    let db = (b.1.point_at(b.1.closest_parameter(pick)) - pick).length();
                    da.partial_cmp(&db).unwrap()
                }).map(|x| x.0).unwrap_or(0);
                // extend row curves of the surface and loft them back
                let rows: Vec<NurbsCurve> = match side {
                    0 | 1 => (0..s.count_v).map(|j| {
                        let v = greville(&s.knots_v, s.degree_v, j);
                        s.isocurve_v(v).extend_by(side == 1, len).unwrap_or_else(|_| s.isocurve_v(v))
                    }).collect(),
                    _ => (0..s.count_u).map(|i| {
                        let u = greville(&s.knots_u, s.degree_u, i);
                        s.isocurve_u(u).extend_by(side == 3, len).unwrap_or_else(|_| s.isocurve_u(u))
                    }).collect(),
                };
                let deg = if side < 2 { s.degree_v } else { s.degree_u };
                match NurbsSurface::loft(&rows, deg.max(1)) {
                    Ok(mut ns) => {
                        if side >= 2 {
                            ns.flip();
                        }
                        ctx.doc.begin_undo("ExtendSrf");
                        ctx.doc.replace_geometry(id, Geometry::Surface(ns));
                        Step::Done
                    }
                    Err(e) => {
                        ctx.log(format!("ExtendSrf failed: {e}"));
                        Step::Cancelled
                    }
                }
            }
            Input::Option(_) => Prompt::pick("Select edge of surface to extend", ObjFilter::Surface).step(),
            _ => Step::Cancelled,
        }
    }
}

fn greville(knots: &[f64], degree: usize, i: usize) -> f64 {
    knots[i + 1..=i + degree].iter().sum::<f64>() / degree as f64
}

pub struct UntrimCmd;

impl Command for UntrimCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select trimmed surfaces to untrim", ObjFilter::Surface, 1).opts([optv("KeepTrimObjects", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                ctx.doc.begin_undo("Untrim");
                for id in ids {
                    if let Some(Geometry::Brep(b)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                        let faces: Vec<Face> = b.faces.iter().map(|f| Face::untrimmed(f.surface.clone())).collect();
                        if faces.len() == 1 {
                            ctx.doc.replace_geometry(id, Geometry::Surface(faces[0].surface.clone()));
                        } else {
                            ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(faces)));
                        }
                    }
                }
                Step::Done
            }
            Input::Option(_) => Prompt::objects("Select trimmed surfaces to untrim", ObjFilter::Surface, 1).step(),
            _ => Step::Cancelled,
        }
    }
}

pub struct MeshToNurbCmd;

impl Command for MeshToNurbCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select meshes to convert", ObjFilter::Mesh, 1).opts([optv("DeleteInput", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                let mut out = Vec::new();
                for id in ids {
                    let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
                    let mut faces = Vec::new();
                    for t in 0..m.triangle_count() {
                        let (a, b, c) = m.triangle(t);
                        let n = (b - a).cross(c - a);
                        if n.length_squared() < 1e-24 {
                            continue;
                        }
                        let plane = Plane::from_normal(a, n.normalize());
                        if let Ok(f) = Face::planar(&plane, vec![vec![a, b, c]]) {
                            faces.push(f);
                        }
                    }
                    if !faces.is_empty() {
                        out.push(Geometry::Brep(Brep::new(faces)));
                    }
                }
                add_selected(ctx, "MeshToNURB", out)
            }
            Input::Option(_) => Prompt::objects("Select meshes to convert", ObjFilter::Mesh, 1).step(),
            _ => Step::Cancelled,
        }
    }
}

// ---- solids ------------------------------------------------------------------------------------

#[derive(Default)]
pub struct EllipsoidCmd {
    center: Option<DVec3>,
    radii: Vec<f64>,
    plane: Plane,
}

impl Command for EllipsoidCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::point("Center of ellipsoid").opts([opt("Corner"), opt("Diameter"), opt("FromFoci"), opt("AroundCurve")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let c = self.center;
        match (c, input) {
            (None, Input::Point(p)) => {
                self.center = Some(p);
                Prompt::point_or_number("End of first axis").step()
            }
            (Some(c), inp @ (Input::Point(_) | Input::Number(_))) => {
                let axes = [self.plane.x_axis, self.plane.y_axis, self.plane.z_axis];
                let r = match inp {
                    Input::Number(v) => v.abs(),
                    Input::Point(p) => (p - c).dot(axes[self.radii.len()]).abs(),
                    _ => 1.0,
                };
                self.radii.push(r.max(1e-6));
                match self.radii.len() {
                    1 => Prompt::point_or_number("End of second axis").step(),
                    2 => Prompt::point_or_number("End of third axis").step(),
                    _ => {
                        let Ok(mut s) = NurbsSurface::sphere(DVec3::ZERO, 1.0) else { return Step::Cancelled };
                        let m = glam::DMat3::from_cols(self.plane.x_axis * self.radii[0], self.plane.y_axis * self.radii[1], self.plane.z_axis * self.radii[2]);
                        s.transform(&DAffine3::from_mat3_translation(m, c));
                        add_selected(ctx, "Ellipsoid", vec![Geometry::Surface(s)])
                    }
                }
            }
            (_, Input::Option(o)) => {
                ctx.log(format!("The {o} option is not implemented in triceratops yet."));
                Prompt::point("Center of ellipsoid").step()
            }
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center
    }
}

pub struct PyramidCmd {
    sides: usize,
    center: Option<DVec3>,
    corner: Option<DVec3>,
    plane: Plane,
}

impl Default for PyramidCmd {
    fn default() -> Self {
        Self { sides: 4, center: None, corner: None, plane: Plane::world_xy() }
    }
}

impl PyramidCmd {
    fn base(&self, corner: DVec3) -> Option<NurbsCurve> {
        let c = self.center?;
        let plane = plane_at(&self.plane, c);
        let (u, v, _) = plane.closest_parameters(corner);
        let r = (u * u + v * v).sqrt();
        let a0 = v.atan2(u);
        let pts: Vec<DVec3> = (0..=self.sides).map(|i| {
            let a = a0 + std::f64::consts::TAU * i as f64 / self.sides as f64;
            plane.point_at(r * a.cos(), r * a.sin())
        }).collect();
        NurbsCurve::polyline(pts).ok()
    }
}

impl Command for PyramidCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.sides = ctx.mem_num("Pyramid.NumSides", 4.0) as usize;
        Prompt::point("Center of pyramid base").opts([optv("NumSides", self.sides), optv("Mode", "Inscribed"), optv("Solid", "Yes")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.center, self.corner, input) {
            (_, _, Input::Option(o)) if o == "NumSides" => Prompt::number("Number of sides").default_num(self.sides as f64).step(),
            (None, _, Input::Number(n)) => {
                self.sides = (n.round() as usize).clamp(3, 360);
                ctx.mem_set("Pyramid.NumSides", self.sides);
                Prompt::point("Center of pyramid base").opts([optv("NumSides", self.sides)]).step()
            }
            (None, _, Input::Point(p)) => {
                self.center = Some(p);
                Prompt::point("Corner of pyramid base").step()
            }
            (Some(_), None, Input::Point(p)) => {
                self.corner = Some(p);
                Prompt::point_or_number("End of pyramid").step()
            }
            (Some(c), Some(corner), inp @ (Input::Point(_) | Input::Number(_))) => {
                let h = match inp {
                    Input::Number(v) => v,
                    Input::Point(p) => (p - c).dot(self.plane.z_axis),
                    _ => 1.0,
                };
                let Some(base) = self.base(corner) else { return Step::Cancelled };
                let apex = c + self.plane.z_axis * h;
                let Some(side) = extrude_to_point(&base, apex) else { return Step::Cancelled };
                let Some(cap) = planar_cap(&base) else { return Step::Cancelled };
                let mut brep = Brep::new(vec![Face::untrimmed(side), cap]);
                brep.orient_outward();
                brep.closed = true;
                add_selected(ctx, "Pyramid", vec![Geometry::Brep(brep)])
            }
            (_, _, Input::Option(_)) => Prompt::point("Center of pyramid base").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.center, self.corner, cursor) {
            (Some(_), None, Some(p)) => self.base(p).map(|b| vec![Preview::Curve(b)]).unwrap_or_default(),
            (Some(c), Some(corner), Some(p)) => {
                let h = (p - c).dot(self.plane.z_axis);
                self.base(corner).and_then(|b| extrude_to_point(&b, c + self.plane.z_axis * h)).map(|s| vec![Preview::Surface(s)]).unwrap_or_default()
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.corner.or(self.center)
    }
}

#[derive(Default)]
pub struct TConeCmd {
    base: Option<DVec3>,
    r1: Option<f64>,
    r2: Option<f64>,
    plane: Plane,
}

impl Command for TConeCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::point("Base of truncated cone").opts([optv("Solid", "Yes")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.base, self.r1, self.r2, input) {
            (None, _, _, Input::Point(p)) => {
                self.base = Some(p);
                Prompt::point_or_number("Base radius").default_num(1.0).step()
            }
            (Some(b), None, _, inp @ (Input::Point(_) | Input::Number(_))) => {
                self.r1 = Some(match inp { Input::Number(v) => v.abs(), Input::Point(p) => (p - b).length(), _ => 1.0 });
                Prompt::point_or_number("End of truncated cone").default_num(1.0).step()
            }
            (Some(b), Some(_), None, inp @ (Input::Point(_) | Input::Number(_))) => {
                let h = match inp { Input::Number(v) => v, Input::Point(p) => (p - b).dot(self.plane.z_axis), _ => 1.0 };
                ctx.mem_set("TCone.Height", fmt_num(h));
                self.r2 = Some(-1.0);
                Prompt::point_or_number("Top radius").default_num(0.5).step()
            }
            (Some(b), Some(r1), Some(_), inp @ (Input::Point(_) | Input::Number(_))) => {
                let h = ctx.mem_num("TCone.Height", 1.0);
                let top_c = b + self.plane.z_axis * h;
                let r2 = match inp { Input::Number(v) => v.abs(), Input::Point(p) => (p - top_c).length(), _ => 0.5 };
                let p0 = plane_at(&self.plane, b);
                let p1 = plane_at(&self.plane, top_c);
                let (Ok(c0), Ok(c1)) = (NurbsCurve::circle(&p0, r1.max(1e-6)), NurbsCurve::circle(&p1, r2.max(1e-6))) else { return Step::Cancelled };
                let Ok(side) = NurbsSurface::loft(&[c0.clone(), c1.clone()], 1) else { return Step::Cancelled };
                let (Some(a), Some(bb)) = (planar_cap(&c0), planar_cap(&c1)) else { return Step::Cancelled };
                let mut brep = Brep::new(vec![Face::untrimmed(side), a, bb]);
                brep.orient_outward();
                brep.closed = true;
                add_selected(ctx, "TCone", vec![Geometry::Brep(brep)])
            }
            (_, _, _, Input::Option(_)) => Prompt::point("Base of truncated cone").step(),
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

#[derive(Default)]
pub struct TubeCmd {
    base: Option<DVec3>,
    r1: Option<f64>,
    r2: Option<f64>,
    plane: Plane,
}

impl Command for TubeCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::point("Base of tube").opts([optv("Solid", "Yes")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.base, self.r1, self.r2, input) {
            (None, _, _, Input::Point(p)) => {
                self.base = Some(p);
                Prompt::point_or_number("Radius").default_num(ctx.mem_num("Tube.R1", 2.0)).step()
            }
            (Some(b), None, _, inp @ (Input::Point(_) | Input::Number(_))) => {
                let r = match inp { Input::Number(v) => v.abs(), Input::Point(p) => (p - b).length(), _ => 2.0 };
                self.r1 = Some(r);
                ctx.mem_set("Tube.R1", fmt_num(r));
                Prompt::point_or_number("Second radius").default_num(ctx.mem_num("Tube.R2", 1.0)).opts([optv("WallThickness", "No")]).step()
            }
            (Some(b), Some(_), None, inp @ (Input::Point(_) | Input::Number(_))) => {
                let r = match inp { Input::Number(v) => v.abs(), Input::Point(p) => (p - b).length(), _ => 1.0 };
                self.r2 = Some(r);
                ctx.mem_set("Tube.R2", fmt_num(r));
                Prompt::point_or_number("End of tube").default_num(ctx.mem_num("Tube.H", 5.0)).step()
            }
            (Some(b), Some(r1), Some(r2), inp @ (Input::Point(_) | Input::Number(_))) => {
                let h = match inp { Input::Number(v) => v, Input::Point(p) => (p - b).dot(self.plane.z_axis), _ => 5.0 };
                ctx.mem_set("Tube.H", fmt_num(h));
                let (ro, ri) = (r1.max(r2), r1.min(r2));
                if ri < 1e-9 || (ro - ri).abs() < 1e-9 {
                    return Step::Cancelled;
                }
                let p0 = plane_at(&self.plane, b);
                let p1 = plane_at(&self.plane, b + self.plane.z_axis * h);
                let tol = 1e-3 * (1.0 + ro);
                let mk = |p: &Plane| -> Option<Vec<Face>> {
                    let outer = NurbsCurve::circle(p, ro).ok()?;
                    let inner = NurbsCurve::circle(p, ri).ok()?;
                    let cap = Face::from_planar_curves(&[outer, inner], tol).ok()?;
                    Some(vec![cap])
                };
                let (Some(cap0), Some(cap1)) = (mk(&p0), mk(&p1)) else { return Step::Cancelled };
                let (Ok(outer), Ok(inner)) = (NurbsSurface::cylinder(&p0, ro, h), NurbsSurface::cylinder(&p0, ri, h)) else { return Step::Cancelled };
                let mut faces = vec![Face::untrimmed(outer), Face::untrimmed(inner)];
                faces.extend(cap0);
                faces.extend(cap1);
                let mut brep = Brep::new(faces);
                brep.closed = true;
                add_selected(ctx, "Tube", vec![Geometry::Brep(brep)])
            }
            (_, _, _, Input::Option(_)) => Prompt::point("Base of tube").step(),
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

#[derive(Default)]
pub struct SlabCmd {
    curves: Vec<ObjectId>,
    distance: Option<f64>,
}

impl Command for SlabCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select closed planar curves", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.distance, input) {
            (_, Input::Objects(ids)) => {
                self.curves = ids;
                Prompt::number("Offset distance").default_num(ctx.mem_num("Slab.Offset", 1.0)).step()
            }
            (None, Input::Number(d)) => {
                self.distance = Some(d);
                ctx.mem_set("Slab.Offset", fmt_num(d));
                Prompt::number("Height").default_num(ctx.mem_num("Slab.Height", 1.0)).step()
            }
            (Some(d), Input::Number(h)) => {
                ctx.mem_set("Slab.Height", fmt_num(h));
                let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
                let mut out = Vec::new();
                for id in &self.curves {
                    let Some(c) = curve_of(ctx.doc, *id) else { continue };
                    if !c.is_closed() {
                        continue;
                    }
                    let Some(plane) = tri_geom::curve_plane(&c) else { continue };
                    let Ok(o) = c.offset(&plane, d, tol) else { continue };
                    let (outer, inner) = if Document::area_of(&Geometry::Curve(o.clone())).unwrap_or(0.0) > Document::area_of(&Geometry::Curve(c.clone())).unwrap_or(0.0) { (o, c.clone()) } else { (c.clone(), o) };
                    let dz = plane.z_axis * h;
                    let Ok(cap0) = Face::from_planar_curves(&[outer.clone(), inner.clone()], tol) else { continue };
                    let mut cap1 = cap0.clone();
                    cap1.transform(&DAffine3::from_translation(dz));
                    let (Ok(so), Ok(si)) = (NurbsSurface::extrude(&outer, dz), NurbsSurface::extrude(&inner, dz)) else { continue };
                    let mut brep = Brep::new(vec![Face::untrimmed(so), Face::untrimmed(si), cap0, cap1]);
                    brep.closed = true;
                    out.push(Geometry::Brep(brep));
                }
                add_selected(ctx, "Slab", out)
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct BooleanSplitCmd {
    first: Vec<ObjectId>,
}

impl Default for BooleanSplitCmd {
    fn default() -> Self {
        Self { first: Vec::new() }
    }
}

fn solid_mesh(doc: &Document, id: ObjectId) -> Option<Mesh> {
    match doc.get(id).map(|o| &o.geometry) {
        Some(Geometry::Surface(s)) => Some(s.tessellate(12, 12)),
        Some(Geometry::Brep(b)) => Some(b.tessellate(12, 12)),
        Some(Geometry::Mesh(m)) => Some(m.clone()),
        _ => None,
    }
}

impl Command for BooleanSplitCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select surfaces or polysurfaces to split", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.first.is_empty() => {
                self.first = ids;
                ctx.doc.clear_selection();
                Prompt::objects("Select cutting surfaces or polysurfaces", ObjFilter::Any, 1).step()
            }
            Input::Objects(cutters) => {
                use super::misc4::{boolean_geometry, geometry_nonempty};
                let cut: Vec<Geometry> = cutters.iter().filter_map(|id| ctx.doc.get(*id).map(|o| o.geometry.clone())).filter(|g| matches!(g, Geometry::Surface(_) | Geometry::Brep(_) | Geometry::Mesh(_))).collect();
                if cut.is_empty() {
                    return Step::Cancelled;
                }
                let mut cutter = cut[0].clone();
                for g in &cut[1..] {
                    if let Some(r) = boolean_geometry(&cutter, g, BooleanOp::Union) {
                        cutter = r;
                    }
                }
                ctx.doc.begin_undo("BooleanSplit");
                let mut out = Vec::new();
                for id in self.first.clone() {
                    let Some(g) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
                    let (Some(a), Some(b)) = (boolean_geometry(&g, &cutter, BooleanOp::Difference), boolean_geometry(&g, &cutter, BooleanOp::Intersection)) else { continue };
                    ctx.doc.remove(id);
                    for part in [a, b] {
                        if geometry_nonempty(&part) {
                            out.push(part);
                        }
                    }
                }
                ctx.doc.clear_selection();
                for g in out {
                    let nid = ctx.doc.add(g);
                    ctx.doc.select(nid, true);
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct ExtractSrfCmd;

impl Command for ExtractSrfCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select surfaces to extract", ObjFilter::Polysurface).opts([optv("Copy", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                let Some(faces) = faces_of(ctx.doc, id) else { return Step::Cancelled };
                if faces.len() < 2 {
                    return Prompt::pick("Select surfaces to extract. Press Enter when done", ObjFilter::Polysurface).step();
                }
                let fi = super::surface_edit::face_under(&faces, p);
                let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
                let mut rest = faces.clone();
                let face = rest.remove(fi);
                ctx.doc.begin_undo("ExtractSrf");
                ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(rest)));
                let g = if face.is_trimmed() { Geometry::Brep(Brep { faces: vec![face], closed: false }) } else { Geometry::Surface(face.surface) };
                let nid = ctx.doc.add_with(g, attrs);
                ctx.doc.select(nid, true);
                Prompt::pick("Select surfaces to extract. Press Enter when done", ObjFilter::Polysurface).step()
            }
            Input::Empty => Step::Done,
            Input::Option(_) => Prompt::pick("Select surfaces to extract", ObjFilter::Polysurface).step(),
            _ => Step::Cancelled,
        }
    }
}

#[allow(dead_code)]
fn unused() {
    let _ = yesno(true);
}
