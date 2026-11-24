//! More curve commands: Helix, Spiral, BlendCrv, Connect, ChangeDegree, CloseCrv, Smooth,
//! CurveThroughPt, Project, DupEdge, DupBorder, Section, Contour, SimplifyCrv, FitCrv, InsertKnot.

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{connect_curves, Face, NurbsCurve, Plane, Ray};

use super::surface_edit::faces_of;
use super::{fmt_num, opt, optv, yesno, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn curve_of(doc: &Document, id: ObjectId) -> Option<NurbsCurve> {
    doc.get(id).and_then(|o| o.geometry.as_curve().cloned())
}

fn add_curve(ctx: &mut CmdCtx, label: &str, c: NurbsCurve) -> ObjectId {
    ctx.doc.begin_undo(label);
    ctx.doc.add(Geometry::Curve(c))
}

// ---- Helix / Spiral -------------------------------------------------------------

pub struct HelixCmd {
    spiral: bool,
    pts: Vec<DVec3>,
    turns: f64,
    radius1: Option<f64>,
}

impl HelixCmd {
    pub fn new(spiral: bool) -> Self {
        Self { spiral, pts: Vec::new(), turns: 10.0, radius1: None }
    }
    fn build(&self, r2: f64) -> Option<NurbsCurve> {
        if self.pts.len() < 2 {
            return None;
        }
        let a = self.pts[0];
        let axis = self.pts[1] - a;
        let len = axis.length();
        if len < 1e-9 && !self.spiral {
            return None;
        }
        let plane = if len > 1e-9 { Plane::from_normal(a, axis) } else { Plane::world_xy() };
        let r1 = self.radius1.unwrap_or(r2);
        let n = (self.turns * 24.0).max(8.0) as usize;
        let pts: Vec<DVec3> = (0..=n)
            .map(|i| {
                let t = i as f64 / n as f64;
                let ang = self.turns * std::f64::consts::TAU * t;
                let r = r1 + (r2 - r1) * t;
                plane.point_at_3(r * ang.cos(), r * ang.sin(), len * t)
            })
            .collect();
        NurbsCurve::interpolate(&pts, 3).ok()
    }
    fn prompt(&self) -> Prompt {
        let opts = [optv("Turns", fmt_num(self.turns)), optv("Pitch", fmt_num(0.0)), optv("Mode", "Turns"), opt("Flat"), opt("ReverseTwist")];
        match (self.pts.len(), self.radius1) {
            (0, _) => Prompt::point("Start of axis").opts(opts),
            (1, _) => Prompt::point("End of axis").opts(opts),
            (_, None) if self.spiral => Prompt::point_or_number("First radius").opts(opts),
            _ => Prompt::point_or_number(if self.spiral { "Second radius" } else { "Radius" }).opts(opts),
        }
    }
}

impl Command for HelixCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.turns = ctx.mem_num("Helix.Turns", 10.0);
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Turns" => Prompt::number("Number of turns").default_num(self.turns).step(),
            Input::Option(_) => self.prompt().step(),
            Input::Number(n) if self.pts.len() < 2 => {
                self.turns = n.max(0.01);
                ctx.mem_set("Helix.Turns", fmt_num(self.turns));
                self.prompt().step()
            }
            Input::Point(p) if self.pts.len() < 2 => {
                self.pts.push(p);
                self.prompt().step()
            }
            inp @ (Input::Point(_) | Input::Number(_)) => {
                let r = match inp {
                    Input::Number(v) => v.abs(),
                    Input::Point(p) => {
                        let a = self.pts[0];
                        let axis = (self.pts[1] - a).normalize_or(DVec3::Z);
                        let d = p - a;
                        (d - axis * d.dot(axis)).length()
                    }
                    _ => 1.0,
                };
                if self.spiral && self.radius1.is_none() {
                    self.radius1 = Some(r);
                    return self.prompt().step();
                }
                match self.build(r) {
                    Some(c) => {
                        add_curve(ctx, if self.spiral { "Spiral" } else { "Helix" }, c);
                        Step::Done
                    }
                    None => Step::Cancelled,
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(c) = cursor else { return vec![] };
        match self.pts.len() {
            1 => vec![Preview::Line(self.pts[0], c)],
            2 => {
                let a = self.pts[0];
                let axis = (self.pts[1] - a).normalize_or(DVec3::Z);
                let d = c - a;
                let r = (d - axis * d.dot(axis)).length();
                self.build(r).map(|cv| vec![Preview::Curve(cv)]).unwrap_or_default()
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

// ---- BlendCrv -------------------------------------------------------------------

#[derive(Default)]
pub struct BlendCrvCmd {
    first: Option<(ObjectId, DVec3)>,
}

fn end_data(c: &NurbsCurve, near: DVec3) -> (DVec3, DVec3) {
    let (t0, t1) = c.domain();
    let tp = c.closest_parameter(near);
    if (tp - t0) < (t1 - tp) {
        (c.start_point(), -c.tangent_at(t0))
    } else {
        (c.end_point(), c.tangent_at(t1))
    }
}

pub fn blend_curve(a: &NurbsCurve, pa: DVec3, b: &NurbsCurve, pb: DVec3) -> Option<NurbsCurve> {
    let (p0, t0) = end_data(a, pa);
    let (p1, t1) = end_data(b, pb);
    let d = (p1 - p0).length();
    if d < 1e-12 {
        return None;
    }
    let cps = vec![p0, p0 + t0 * (d / 3.0), p1 + t1 * (d / 3.0), p1];
    NurbsCurve::from_control_points(3, cps).ok()
}

impl Command for BlendCrvCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select first curve to blend", ObjFilter::Curve).opts([optv("Continuity", "Tangency"), optv("Join", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, input) {
            (_, Input::Option(_)) => Prompt::pick("Select first curve to blend", ObjFilter::Curve).step(),
            (None, Input::Pick(id, p)) => {
                self.first = Some((id, p));
                Prompt::pick("Select second curve to blend", ObjFilter::Curve).step()
            }
            (Some((a_id, pa)), Input::Pick(b_id, pb)) => {
                let (Some(a), Some(b)) = (curve_of(ctx.doc, a_id), curve_of(ctx.doc, b_id)) else { return Step::Cancelled };
                match blend_curve(&a, pa, &b, pb) {
                    Some(c) => {
                        add_curve(ctx, "BlendCrv", c);
                        Step::Done
                    }
                    None => Step::Cancelled,
                }
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- Connect --------------------------------------------------------------------

#[derive(Default)]
pub struct ConnectCmd {
    first: Option<(ObjectId, DVec3)>,
}

impl Command for ConnectCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select first curve to connect", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, input) {
            (None, Input::Pick(id, p)) => {
                self.first = Some((id, p));
                Prompt::pick("Select second curve to connect", ObjFilter::Curve).step()
            }
            (Some((a_id, pa)), Input::Pick(b_id, pb)) => {
                let (Some(a), Some(b)) = (curve_of(ctx.doc, a_id), curve_of(ctx.doc, b_id)) else { return Step::Cancelled };
                match connect_curves(&a, &b, pa, pb, ctx.doc.settings.absolute_tolerance.max(1e-6)) {
                    Ok((ka, kb)) => {
                        ctx.doc.begin_undo("Connect");
                        ctx.doc.replace_geometry(a_id, Geometry::Curve(ka));
                        ctx.doc.replace_geometry(b_id, Geometry::Curve(kb));
                        Step::Done
                    }
                    Err(e) => {
                        ctx.log(format!("Connect failed: {e}"));
                        Step::Cancelled
                    }
                }
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- ChangeDegree / CloseCrv / Smooth / SimplifyCrv / FitCrv / InsertKnot -------

#[derive(Default)]
pub struct ChangeDegreeCmd {
    ids: Vec<ObjectId>,
}


impl Command for ChangeDegreeCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves or surfaces to change degree", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::number("New degree").default_num(3.0).opts([optv("Deformable", "No")]).step()
            }
            Input::Number(d) => {
                let d = (d.round() as usize).clamp(1, 11);
                ctx.doc.begin_undo("ChangeDegree");
                for id in &self.ids {
                    if let Some(c) = curve_of(ctx.doc, *id) {
                        let r = if d >= c.degree { c.to_degree(d) } else { c.rebuild(c.control_points.len().max(d + 1), d) };
                        if let Ok(nc) = r {
                            ctx.doc.replace_geometry(*id, Geometry::Curve(nc));
                        }
                    }
                }
                Step::Done
            }
            Input::Option(_) => Prompt::number("New degree").default_num(3.0).step(),
            _ => Step::Cancelled,
        }
    }
}

pub struct CloseCrvCmd;

impl Command for CloseCrvCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves to close", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("CloseCrv");
        for id in ids {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            if c.is_closed() {
                continue;
            }
            let gap = (c.end_point() - c.start_point()).length();
            let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
            let closed = if gap < tol * 10.0 {
                let mut cc = c.clone();
                let n = cc.control_points.len();
                cc.control_points[n - 1] = cc.control_points[0];
                Ok(cc)
            } else {
                NurbsCurve::line(c.end_point(), c.start_point()).and_then(|l| c.join_any(&l))
            };
            if let Ok(cc) = closed {
                ctx.doc.replace_geometry(id, Geometry::Curve(cc));
            }
        }
        Step::Done
    }
}

#[derive(Default)]
pub struct SmoothCmd {
    ids: Vec<ObjectId>,
}


impl Command for SmoothCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves or surfaces to smooth", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::number("Smooth factor").default_num(0.2).opts([optv("SmoothX", "Yes"), optv("SmoothY", "Yes"), optv("SmoothZ", "Yes"), optv("FixBoundaries", "Yes")]).step()
            }
            Input::Number(f) => {
                ctx.doc.begin_undo("Smooth");
                for id in &self.ids {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        match &mut o.geometry {
                            Geometry::Curve(c) => {
                                let n = c.control_points.len();
                                let old = c.control_points.clone();
                                for i in 1..n.saturating_sub(1) {
                                    let avg = (old[i - 1] + old[i + 1]) * 0.5;
                                    c.control_points[i] = old[i] + (avg - old[i]) * f;
                                }
                            }
                            Geometry::Surface(s) => {
                                let old = s.control_points.clone();
                                for i in 1..s.count_u.saturating_sub(1) {
                                    for j in 1..s.count_v.saturating_sub(1) {
                                        let idx = i * s.count_v + j;
                                        let avg = (old[idx - 1] + old[idx + 1] + old[idx - s.count_v] + old[idx + s.count_v]) * 0.25;
                                        s.control_points[idx] = old[idx] + (avg - old[idx]) * f;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Step::Done
            }
            Input::Option(_) => Prompt::number("Smooth factor").default_num(0.2).step(),
            _ => Step::Cancelled,
        }
    }
}

pub struct SimplifyCrvCmd;

impl Command for SimplifyCrvCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves to simplify", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("SimplifyCrv");
        let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
        for id in ids {
            let Some(c) = curve_of(ctx.doc, id) else { continue };
            if c.degree != 1 {
                continue;
            }
            // drop collinear interior vertices
            let pts = &c.control_points;
            let mut out = vec![pts[0]];
            for i in 1..pts.len() - 1 {
                let a = out.last().copied().unwrap();
                let d1 = (pts[i] - a).normalize_or(DVec3::X);
                let d2 = (pts[i + 1] - pts[i]).normalize_or(DVec3::X);
                if d1.cross(d2).length() > tol {
                    out.push(pts[i]);
                }
            }
            out.push(pts[pts.len() - 1]);
            if out.len() < pts.len() {
                if let Ok(nc) = NurbsCurve::polyline(out) {
                    ctx.doc.replace_geometry(id, Geometry::Curve(nc));
                }
            }
        }
        Step::Done
    }
}

#[derive(Default)]
pub struct FitCrvCmd {
    ids: Vec<ObjectId>,
}


impl Command for FitCrvCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves to fit", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::number("Fitting tolerance").default_num(ctx.doc.settings.absolute_tolerance).opts([optv("Degree", "3"), optv("DeleteInput", "Yes")]).step()
            }
            Input::Number(tol) => {
                ctx.doc.begin_undo("FitCrv");
                for id in &self.ids {
                    let Some(c) = curve_of(ctx.doc, *id) else { continue };
                    let mut count = 4;
                    let mut best = None;
                    while count <= c.control_points.len().max(4) * 2 + 4 {
                        if let Ok(r) = c.rebuild(count, 3) {
                            let dev = (0..40).map(|i| {
                                let (t0, t1) = c.domain();
                                let p = c.point_at(t0 + (t1 - t0) * i as f64 / 40.0);
                                (r.point_at(r.closest_parameter(p)) - p).length()
                            }).fold(0.0, f64::max);
                            best = Some(r);
                            if dev <= tol {
                                break;
                            }
                        }
                        count += 2;
                    }
                    if let Some(r) = best {
                        ctx.doc.replace_geometry(*id, Geometry::Curve(r));
                    }
                }
                Step::Done
            }
            Input::Option(_) => Prompt::number("Fitting tolerance").default_num(ctx.doc.settings.absolute_tolerance).step(),
            _ => Step::Cancelled,
        }
    }
}

pub struct InsertKnotCmd;

impl Command for InsertKnotCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select curve for knot insertion", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                let Some(mut c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let t = c.closest_parameter(p);
                c.insert_knot(t, 1);
                ctx.doc.begin_undo("InsertKnot");
                ctx.doc.replace_geometry(id, Geometry::Curve(c));
                Prompt::pick("Select curve for knot insertion. Press Enter when done", ObjFilter::Curve).step()
            }
            Input::Empty => Step::Done,
            _ => Step::Cancelled,
        }
    }
}

// ---- CurveThroughPt --------------------------------------------------------------

pub struct CurveThroughPtCmd;

impl Command for CurveThroughPtCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select point objects to fit curve through", ObjFilter::Point, 2).opts([optv("Degree", "3"), optv("CurveType", "Interpolated"), optv("Closed", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                let pts: Vec<DVec3> = ids.iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p) } else { None }).collect();
                match NurbsCurve::interpolate(&pts, 3) {
                    Ok(c) => {
                        add_curve(ctx, "CurveThroughPt", c);
                        Step::Done
                    }
                    Err(e) => {
                        ctx.log(format!("CurveThroughPt failed: {e}"));
                        Step::Cancelled
                    }
                }
            }
            Input::Option(_) => Prompt::objects("Select point objects to fit curve through", ObjFilter::Point, 2).step(),
            _ => Step::Cancelled,
        }
    }
}

// ---- Project -----------------------------------------------------------------------

#[derive(Default)]
pub struct ProjectCmd {
    curves: Vec<ObjectId>,
}

impl Command for ProjectCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves and points to project", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.curves.is_empty() => {
                self.curves = ids;
                ctx.doc.clear_selection();
                Prompt::objects("Select surfaces, polysurfaces or meshes to project onto", ObjFilter::Any, 1).opts([optv("Direction", "CPlane"), optv("DeleteInput", "No")]).step()
            }
            Input::Objects(targets) => {
                let dir = ctx.cplane.z_axis;
                let meshes: Vec<tri_geom::Mesh> = targets
                    .iter()
                    .filter_map(|id| ctx.doc.get(*id))
                    .filter_map(|o| match &o.geometry {
                        Geometry::Surface(s) => Some(s.tessellate(16, 16)),
                        Geometry::Brep(b) => Some(b.tessellate(16, 16)),
                        Geometry::Mesh(m) => Some(m.clone()),
                        _ => None,
                    })
                    .collect();
                if meshes.is_empty() {
                    return Step::Cancelled;
                }
                let hit = |p: DVec3| -> Vec<DVec3> {
                    let mut out = Vec::new();
                    for m in &meshes {
                        for d in [dir, -dir] {
                            let r = Ray::new(p + d * 1e6 * -1.0, d);
                            // cast from far away so both sides are found
                            let far = Ray::new(p - d * 1e5, d);
                            let _ = r;
                            if let Some(t) = m.intersect_ray(&far) {
                                out.push(far.at(t));
                            }
                        }
                    }
                    out
                };
                ctx.doc.begin_undo("Project");
                ctx.doc.clear_selection();
                let mut n = 0;
                for id in &self.curves {
                    let Some(o) = ctx.doc.get(*id).cloned() else { continue };
                    match &o.geometry {
                        Geometry::Point(p) => {
                            for q in hit(*p) {
                                let nid = ctx.doc.add_with(Geometry::Point(q), o.attributes.clone());
                                ctx.doc.select(nid, true);
                                n += 1;
                            }
                        }
                        Geometry::Curve(c) => {
                            let samples = c.tessellate_adaptive(1e-3 * (1.0 + c.bounding_box().diagonal()));
                            // one projected polyline per target side, following contiguous hits
                            let mut runs: Vec<Vec<DVec3>> = Vec::new();
                            let mut current: Vec<DVec3> = Vec::new();
                            for p in &samples {
                                let hits = hit(*p);
                                match hits.first() {
                                    Some(q) => current.push(*q),
                                    None => {
                                        if current.len() >= 2 {
                                            runs.push(std::mem::take(&mut current));
                                        }
                                        current.clear();
                                    }
                                }
                            }
                            if current.len() >= 2 {
                                runs.push(current);
                            }
                            for run in runs {
                                let curve = if c.degree == 1 { NurbsCurve::polyline(run) } else { NurbsCurve::interpolate(&run, 3) };
                                if let Ok(pc) = curve {
                                    let nid = ctx.doc.add_with(Geometry::Curve(pc), o.attributes.clone());
                                    ctx.doc.select(nid, true);
                                    n += 1;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                ctx.log(format!("{n} objects projected."));
                Step::Done
            }
            Input::Option(_) => Prompt::objects("Select surfaces, polysurfaces or meshes to project onto", ObjFilter::Any, 1).step(),
            _ => Step::Cancelled,
        }
    }
}

// ---- DupEdge / DupBorder -------------------------------------------------------------

pub struct DupEdgeCmd;

impl Command for DupEdgeCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select edges to duplicate", ObjFilter::Surface).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                let Some(faces) = faces_of(ctx.doc, id) else { return Step::Cancelled };
                let tol = 1e-3 * (1.0 + ctx.doc.get(id).map_or(1.0, |o| o.geometry.bounding_box().diagonal()));
                let mut best: Option<(f64, NurbsCurve)> = None;
                for f in &faces {
                    for e in f.edge_curves(tol) {
                        let d = (e.point_at(e.closest_parameter(p)) - p).length();
                        if best.as_ref().is_none_or(|b| d < b.0) {
                            best = Some((d, e));
                        }
                    }
                }
                if let Some((_, e)) = best {
                    ctx.doc.begin_undo("DupEdge");
                    let nid = ctx.doc.add(Geometry::Curve(e));
                    ctx.doc.select(nid, true);
                }
                Prompt::pick("Select edges to duplicate. Press Enter when done", ObjFilter::Surface).step()
            }
            Input::Empty => Step::Done,
            _ => Step::Cancelled,
        }
    }
}

pub struct DupBorderCmd;

impl Command for DupBorderCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select surfaces, polysurfaces or meshes for border duplication", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("DupBorder");
        ctx.doc.clear_selection();
        for id in ids {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            let faces: Vec<Face> = match &o.geometry {
                Geometry::Surface(s) => vec![Face::untrimmed(s.clone())],
                Geometry::Brep(b) => b.faces.clone(),
                _ => continue,
            };
            let brep = tri_geom::Brep::new(faces);
            for (_, pts) in brep.naked_planar_loops() {
                if let Ok(c) = NurbsCurve::polyline(pts) {
                    let nid = ctx.doc.add(Geometry::Curve(c));
                    ctx.doc.select(nid, true);
                }
            }
            if brep.closed {
                ctx.log(format!("Object #{} is closed: no naked edges.", id));
            }
        }
        Step::Done
    }
}

// ---- Section / Contour -----------------------------------------------------------------

#[derive(Default)]
pub struct SectionCmd {
    ids: Vec<ObjectId>,
    first: Option<DVec3>,
}

fn section_objects(ctx: &mut CmdCtx, ids: &[ObjectId], plane: &Plane) -> usize {
    let mut n = 0;
    for id in ids {
        let Some(o) = ctx.doc.get(*id).cloned() else { continue };
        let mesh = match &o.geometry {
            Geometry::Surface(s) => s.tessellate(24, 24),
            Geometry::Brep(b) => b.tessellate(24, 24),
            Geometry::Mesh(m) => m.clone(),
            _ => continue,
        };
        let tol = 1e-4 * (1.0 + mesh.bounding_box().diagonal());
        for chain in mesh.section(plane, tol) {
            if chain.len() >= 2 {
                if let Ok(c) = NurbsCurve::polyline(chain) {
                    let nid = ctx.doc.add_with(Geometry::Curve(c), o.attributes.clone());
                    ctx.doc.select(nid, true);
                    n += 1;
                }
            }
        }
    }
    n
}

impl Command for SectionCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to section", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, input) {
            (_, Input::Objects(ids)) => {
                self.ids = ids;
                ctx.doc.clear_selection();
                Prompt::point("Start of section").step()
            }
            (None, Input::Point(p)) => {
                self.first = Some(p);
                Prompt::point("End of section").step()
            }
            (Some(a), Input::Point(b)) => {
                let normal = (b - a).cross(ctx.cplane.z_axis);
                if normal.length() < 1e-9 {
                    return Step::Cancelled;
                }
                let plane = Plane::from_normal(a, normal);
                ctx.doc.begin_undo("Section");
                let n = section_objects(ctx, &self.ids, &plane);
                ctx.log(format!("{n} section curves created."));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.first, cursor) {
            (Some(a), Some(b)) => vec![Preview::Line(a, b)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.first
    }
}

#[derive(Default)]
pub struct ContourCmd {
    ids: Vec<ObjectId>,
    base: Option<DVec3>,
    dir: Option<DVec3>,
}

impl Command for ContourCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to contour", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.base, self.dir, input) {
            (_, _, Input::Objects(ids)) => {
                self.ids = ids;
                ctx.doc.clear_selection();
                Prompt::point("Contour plane base point").step()
            }
            (None, _, Input::Point(p)) => {
                self.base = Some(p);
                Prompt::point("Direction perpendicular to contour planes").step()
            }
            (Some(b), None, Input::Point(p)) => {
                let d = p - b;
                if d.length() < 1e-9 {
                    return Step::Cancelled;
                }
                self.dir = Some(d.normalize());
                Prompt::number("Distance between contours").default_num(ctx.mem_num("Contour.Distance", 1.0)).step()
            }
            (Some(b), Some(d), Input::Number(spacing)) => {
                let spacing = spacing.abs().max(1e-6);
                ctx.mem_set("Contour.Distance", fmt_num(spacing));
                let bb = self.ids.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |a, o| a.union(&o.geometry.bounding_box()));
                let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                for c in bb.corners() {
                    let s = (c - b).dot(d);
                    lo = lo.min(s);
                    hi = hi.max(s);
                }
                ctx.doc.begin_undo("Contour");
                let mut n = 0;
                let mut s = (lo / spacing).ceil() * spacing;
                while s <= hi + 1e-9 {
                    let plane = Plane::from_normal(b + d * s, d);
                    n += section_objects(ctx, &self.ids, &plane);
                    s += spacing;
                }
                ctx.log(format!("{n} contour curves created."));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

#[allow(dead_code)]
fn unused() -> &'static str {
    yesno(true)
}
