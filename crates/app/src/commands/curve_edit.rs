//! Curve editing commands: Trim, Split, Extend, Fillet, Chamfer, Offset, Rebuild, Divide, Intersect.

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{chamfer_curves, curve_curve_intersections, curve_plane, fillet_curves, NurbsCurve, Plane};

use super::{fmt_num, opt, optv, yesno, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn doc_tol(doc: &Document) -> f64 {
    doc.settings.absolute_tolerance.max(1e-6)
}

fn curve_of(doc: &Document, id: ObjectId) -> Option<NurbsCurve> {
    doc.get(id).and_then(|o| o.geometry.as_curve().cloned())
}

/// Parameters where `c` meets any of the cutter curves.
fn cut_params(c: &NurbsCurve, cutters: &[NurbsCurve], tol: f64) -> Vec<f64> {
    let mut ps = Vec::new();
    for k in cutters {
        for x in curve_curve_intersections(c, k, tol) {
            ps.push(x.ta);
        }
    }
    ps
}

/// Split a (possibly closed) curve at parameters; closed curves get their seam pieces joined.
fn split_pieces(c: &NurbsCurve, params: &[f64]) -> Vec<NurbsCurve> {
    let mut pieces = c.split_at_many(params);
    if c.is_closed() && pieces.len() >= 2 {
        let last = pieces.pop().unwrap();
        let first = pieces.remove(0);
        if let Ok(j) = last.join(&first) {
            pieces.push(j);
        } else {
            pieces.push(last);
            pieces.insert(0, first);
        }
    }
    pieces
}

// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct TrimCmd {
    cutters: Vec<ObjectId>,
    trimmed: usize,
}

impl Command for TrimCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select cutting objects", ObjFilter::Curve, 1).opts([optv("ApparentIntersections", "No"), opt("Line")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.cutters = ids;
                ctx.doc.clear_selection();
                Prompt::pick("Select object to trim", ObjFilter::Any).opts([optv("ApparentIntersections", "No"), opt("Line")]).step()
            }
            Input::Pick(id, p) if matches!(ctx.doc.get(id).map(|o| &o.geometry), Some(Geometry::Surface(_)) | Some(Geometry::Brep(_))) => {
                let cutters: Vec<NurbsCurve> = self.cutters.iter().filter_map(|k| curve_of(ctx.doc, *k)).collect();
                if self.trimmed == 0 {
                    ctx.doc.begin_undo("Trim");
                }
                if super::surface_edit::trim_face(ctx, id, p, &cutters) {
                    self.trimmed += 1;
                } else {
                    ctx.log("The cutting curves do not cross the surface.");
                }
                Prompt::pick("Select object to trim. Press Enter when done", ObjFilter::Any).step()
            }
            Input::Pick(id, p) => {
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let cutters: Vec<NurbsCurve> = self.cutters.iter().filter(|k| **k != id).filter_map(|k| curve_of(ctx.doc, *k)).collect();
                let tol = doc_tol(ctx.doc) * 10.0;
                let params = cut_params(&c, &cutters, tol);
                if params.is_empty() || (c.is_closed() && params.len() < 2) {
                    ctx.log("The object does not intersect the cutting objects.");
                    return Prompt::pick("Select object to trim", ObjFilter::Any).step();
                }
                let pieces = split_pieces(&c, &params);
                let tp = c.closest_parameter(p);
                // the piece whose domain-centre point is nearest the pick is removed
                let remove = pieces
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let da = (a.point_at(a.closest_parameter(c.point_at(tp))) - p).length();
                        let db = (b.point_at(b.closest_parameter(c.point_at(tp))) - p).length();
                        da.partial_cmp(&db).unwrap()
                    })
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
                if self.trimmed == 0 {
                    ctx.doc.begin_undo("Trim");
                }
                ctx.doc.remove(id);
                for (i, piece) in pieces.into_iter().enumerate() {
                    if i != remove {
                        ctx.doc.add_with(Geometry::Curve(piece), attrs.clone());
                    }
                }
                self.trimmed += 1;
                Prompt::pick("Select object to trim. Press Enter when done", ObjFilter::Any).step()
            }
            Input::Empty => Step::Done,
            Input::Option(_) => Prompt::pick("Select object to trim", ObjFilter::Any).step(),
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct SplitCmd {
    targets: Vec<ObjectId>,
}

impl Command for SplitCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to split", ObjFilter::Any, 1).opts([opt("Point")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.targets.is_empty() => {
                self.targets = ids;
                ctx.doc.clear_selection();
                Prompt::objects("Select cutting objects", ObjFilter::Curve, 1).opts([opt("Point"), optv("Shrink", "No")]).step()
            }
            Input::Objects(cutter_ids) => {
                let cutters: Vec<NurbsCurve> = cutter_ids.iter().filter_map(|k| curve_of(ctx.doc, *k)).collect();
                let tol = doc_tol(ctx.doc) * 10.0;
                ctx.doc.begin_undo("Split");
                let mut n = 0;
                for id in self.targets.clone() {
                    if matches!(ctx.doc.get(id).map(|o| &o.geometry), Some(Geometry::Surface(_)) | Some(Geometry::Brep(_))) {
                        n += super::surface_edit::split_faces(ctx, id, &cutters);
                        continue;
                    }
                    let Some(c) = curve_of(ctx.doc, id) else { continue };
                    let own_cutters: Vec<NurbsCurve> = cutter_ids.iter().filter(|k| **k != id).filter_map(|k| curve_of(ctx.doc, *k)).collect();
                    let params = cut_params(&c, if own_cutters.len() == cutters.len() { &cutters } else { &own_cutters }, tol);
                    if params.is_empty() {
                        continue;
                    }
                    let pieces = split_pieces(&c, &params);
                    if pieces.len() < 2 {
                        continue;
                    }
                    let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
                    ctx.doc.remove(id);
                    for piece in pieces {
                        let nid = ctx.doc.add_with(Geometry::Curve(piece), attrs.clone());
                        ctx.doc.select(nid, true);
                        n += 1;
                    }
                }
                if n == 0 {
                    ctx.log("No objects were split.");
                } else {
                    ctx.log(format!("Split into {n} curves."));
                }
                Step::Done
            }
            Input::Option(_) => Prompt::objects("Select cutting objects", ObjFilter::Curve, 1).step(),
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct ExtendCmd {
    boundaries: Vec<ObjectId>,
    length: Option<f64>,
    dynamic: bool,
    target: Option<(ObjectId, bool)>,
}

impl ExtendCmd {
    fn pick_prompt(&self) -> Prompt {
        Prompt::pick("Select curve to extend", ObjFilter::Curve).opts([optv("Type", "Natural")])
    }
}

impl Command for ExtendCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select boundary objects or enter extension length. Press Enter for dynamic extend", ObjFilter::Curve, 0)
            .opts([optv("Type", "Natural")])
            .step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.boundaries = ids;
                self.dynamic = self.boundaries.is_empty();
                ctx.doc.clear_selection();
                self.pick_prompt().step()
            }
            Input::Text(t) if self.boundaries.is_empty() && self.target.is_none() => {
                match crate::input::parse_number(&t) {
                    Some(l) => {
                        self.length = Some(l);
                        self.pick_prompt().step()
                    }
                    None => Step::Cancelled,
                }
            }
            Input::Number(l) if self.target.is_none() => {
                self.length = Some(l);
                self.pick_prompt().step()
            }
            Input::Pick(id, p) => {
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let (t0, t1) = c.domain();
                let tp = c.closest_parameter(p);
                let at_end = (tp - t0) > (t1 - tp);
                if let Some(len) = self.length {
                    match c.extend_by(at_end, len) {
                        Ok(e) => {
                            ctx.doc.begin_undo("Extend");
                            ctx.doc.replace_geometry(id, Geometry::Curve(e));
                        }
                        Err(e) => ctx.log(format!("Extend failed: {e}")),
                    }
                    return self.pick_prompt().step();
                }
                if self.dynamic {
                    self.target = Some((id, at_end));
                    return Prompt::point("End of extension").step();
                }
                // extend along the tangent until the first boundary intersection
                let end = if at_end { c.end_point() } else { c.start_point() };
                let tan = if at_end { c.tangent_at(t1) } else { -c.tangent_at(t0) };
                let reach = (ctx.doc.bounding_box().diagonal() + 1.0) * 4.0;
                let Ok(probe) = NurbsCurve::line(end, end + tan * reach) else { return Step::Cancelled };
                let tol = doc_tol(ctx.doc) * 10.0;
                let mut best: Option<DVec3> = None;
                for b in &self.boundaries {
                    let Some(bc) = curve_of(ctx.doc, *b) else { continue };
                    for x in curve_curve_intersections(&probe, &bc, tol) {
                        if (x.point - end).length() < tol {
                            continue;
                        }
                        if best.is_none_or(|bp| (x.point - end).length() < (bp - end).length()) {
                            best = Some(x.point);
                        }
                    }
                }
                match best {
                    Some(target) => match c.extend_to(at_end, target) {
                        Ok(e) => {
                            ctx.doc.begin_undo("Extend");
                            ctx.doc.replace_geometry(id, Geometry::Curve(e));
                        }
                        Err(e) => ctx.log(format!("Extend failed: {e}")),
                    },
                    None => ctx.log("The curve does not reach the boundary objects."),
                }
                self.pick_prompt().step()
            }
            Input::Point(p) => {
                if let Some((id, at_end)) = self.target.take() {
                    if let Some(c) = curve_of(ctx.doc, id) {
                        match c.extend_to(at_end, p) {
                            Ok(e) => {
                                ctx.doc.begin_undo("Extend");
                                ctx.doc.replace_geometry(id, Geometry::Curve(e));
                            }
                            Err(e) => ctx.log(format!("Extend failed: {e}")),
                        }
                    }
                }
                self.pick_prompt().step()
            }
            Input::Empty => Step::Done,
            Input::Option(_) => self.pick_prompt().step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.target, cursor) {
            (Some((id, at_end)), Some(p)) => curve_of(doc, id).and_then(|c| c.extend_to(at_end, p).ok()).map(|e| vec![Preview::Curve(e)]).unwrap_or_default(),
            _ => vec![],
        }
    }
}

pub struct FilletCmd {
    radius: f64,
    join: bool,
    trim: bool,
    first: Option<(ObjectId, DVec3)>,
    asking: bool,
}

impl Default for FilletCmd {
    fn default() -> Self {
        Self { radius: 1.0, join: true, trim: true, first: None, asking: false }
    }
}

impl FilletCmd {
    fn prompt(&self) -> Prompt {
        let opts = [optv("Radius", fmt_num(self.radius)), optv("Join", yesno(self.join)), optv("Trim", yesno(self.trim)), optv("ArcExtension", "Arc")];
        if self.first.is_none() {
            Prompt::pick("Select first curve to fillet", ObjFilter::Curve).opts(opts)
        } else {
            Prompt::pick("Select second curve to fillet", ObjFilter::Curve).opts(opts)
        }
    }
}

/// Apply a fillet/chamfer result to the document.
fn apply_corner(ctx: &mut CmdCtx, label: &str, a_id: ObjectId, b_id: ObjectId, ta: NurbsCurve, tb: NurbsCurve, mid: NurbsCurve, join: bool, trim: bool) {
    ctx.doc.begin_undo(label);
    let attrs = ctx.doc.get(a_id).map(|o| o.attributes.clone()).unwrap_or_default();
    if !trim {
        ctx.doc.add_with(Geometry::Curve(mid), attrs);
        return;
    }
    if join {
        let joined = ta.join_any(&mid).and_then(|j| j.join_any(&tb));
        if let Ok(j) = joined {
            ctx.doc.remove(a_id);
            ctx.doc.remove(b_id);
            let id = ctx.doc.add_with(Geometry::Curve(j), attrs);
            ctx.doc.select(id, true);
            return;
        }
        ctx.log("Unable to join the fillet; the pieces were left separate.");
    }
    ctx.doc.replace_geometry(a_id, Geometry::Curve(ta));
    ctx.doc.replace_geometry(b_id, Geometry::Curve(tb));
    ctx.doc.add_with(Geometry::Curve(mid), attrs);
}

impl Command for FilletCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.radius = ctx.mem_num("Fillet.Radius", 1.0);
        self.join = ctx.mem_bool("Fillet.Join", true);
        self.trim = ctx.mem_bool("Fillet.Trim", true);
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => match o.as_str() {
                "Radius" => {
                    self.asking = true;
                    Prompt::number("Fillet radius").default_num(self.radius).step()
                }
                "Join" => {
                    self.join = !self.join;
                    ctx.mem_set("Fillet.Join", yesno(self.join));
                    self.prompt().step()
                }
                "Trim" => {
                    self.trim = !self.trim;
                    ctx.mem_set("Fillet.Trim", yesno(self.trim));
                    self.prompt().step()
                }
                _ => self.prompt().step(),
            },
            Input::Number(r) if self.asking => {
                self.asking = false;
                self.radius = r.abs();
                ctx.mem_set("Fillet.Radius", fmt_num(self.radius));
                self.prompt().step()
            }
            Input::Empty if self.asking => {
                self.asking = false;
                self.prompt().step()
            }
            Input::Pick(id, p) => match self.first {
                None => {
                    self.first = Some((id, p));
                    self.prompt().step()
                }
                Some((a_id, pa)) => {
                    if a_id == id {
                        ctx.log("Select a different curve for the second pick.");
                        return self.prompt().step();
                    }
                    let (Some(a), Some(b)) = (curve_of(ctx.doc, a_id), curve_of(ctx.doc, id)) else { return Step::Cancelled };
                    let plane = curve_plane(&a).filter(|_| a.degree > 1 || a.control_points.len() > 2).unwrap_or(ctx.cplane);
                    let plane = Plane { origin: pa, ..plane };
                    let tol = doc_tol(ctx.doc);
                    match fillet_curves(&a, &b, self.radius, pa, p, &plane, tol) {
                        Ok((ta, tb, arc)) => {
                            apply_corner(ctx, "Fillet", a_id, id, ta, tb, arc, self.join, self.trim);
                            Step::Done
                        }
                        Err(e) => {
                            ctx.log(format!("Fillet failed: {e}"));
                            self.first = None;
                            self.prompt().step()
                        }
                    }
                }
            },
            _ => Step::Cancelled,
        }
    }
}

pub struct ChamferCmd {
    d1: f64,
    d2: f64,
    join: bool,
    trim: bool,
    first: Option<(ObjectId, DVec3)>,
    asking: u8,
}

impl Default for ChamferCmd {
    fn default() -> Self {
        Self { d1: 1.0, d2: 1.0, join: true, trim: true, first: None, asking: 0 }
    }
}

impl ChamferCmd {
    fn prompt(&self) -> Prompt {
        let opts = [optv("Distances", format!("{},{}", fmt_num(self.d1), fmt_num(self.d2))), optv("Join", yesno(self.join)), optv("Trim", yesno(self.trim))];
        if self.first.is_none() {
            Prompt::pick("Select first curve to chamfer", ObjFilter::Curve).opts(opts)
        } else {
            Prompt::pick("Select second curve to chamfer", ObjFilter::Curve).opts(opts)
        }
    }
}

impl Command for ChamferCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.d1 = ctx.mem_num("Chamfer.D1", 1.0);
        self.d2 = ctx.mem_num("Chamfer.D2", 1.0);
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => match o.as_str() {
                "Distances" => {
                    self.asking = 1;
                    Prompt::number("First chamfer distance").default_num(self.d1).step()
                }
                "Join" => {
                    self.join = !self.join;
                    self.prompt().step()
                }
                "Trim" => {
                    self.trim = !self.trim;
                    self.prompt().step()
                }
                _ => self.prompt().step(),
            },
            Input::Number(v) if self.asking == 1 => {
                self.d1 = v.abs();
                ctx.mem_set("Chamfer.D1", fmt_num(self.d1));
                self.asking = 2;
                Prompt::number("Second chamfer distance").default_num(self.d1).step()
            }
            Input::Number(v) if self.asking == 2 => {
                self.d2 = v.abs();
                ctx.mem_set("Chamfer.D2", fmt_num(self.d2));
                self.asking = 0;
                self.prompt().step()
            }
            Input::Empty if self.asking > 0 => {
                if self.asking == 2 {
                    self.d2 = self.d1;
                }
                self.asking = 0;
                self.prompt().step()
            }
            Input::Pick(id, p) => match self.first {
                None => {
                    self.first = Some((id, p));
                    self.prompt().step()
                }
                Some((a_id, pa)) => {
                    let (Some(a), Some(b)) = (curve_of(ctx.doc, a_id), curve_of(ctx.doc, id)) else { return Step::Cancelled };
                    let tol = doc_tol(ctx.doc);
                    match chamfer_curves(&a, &b, self.d1, self.d2, pa, p, tol) {
                        Ok((ta, tb, line)) => {
                            apply_corner(ctx, "Chamfer", a_id, id, ta, tb, line, self.join, self.trim);
                            Step::Done
                        }
                        Err(e) => {
                            ctx.log(format!("Chamfer failed: {e}"));
                            self.first = None;
                            self.prompt().step()
                        }
                    }
                }
            },
            _ => Step::Cancelled,
        }
    }
}

pub struct OffsetCmd {
    distance: f64,
    both_sides: bool,
    in_cplane: bool,
    curve: Option<ObjectId>,
    asking: bool,
}

impl Default for OffsetCmd {
    fn default() -> Self {
        Self { distance: 1.0, both_sides: false, in_cplane: false, curve: None, asking: false }
    }
}

impl OffsetCmd {
    fn opts(&self) -> Vec<super::Opt> {
        vec![
            optv("Distance", fmt_num(self.distance)),
            optv("Corner", "Sharp"),
            opt("ThroughPoint"),
            optv("Tolerance", "0.001"),
            optv("BothSides", yesno(self.both_sides)),
            optv("InCPlane", yesno(self.in_cplane)),
            optv("Cap", "None"),
        ]
    }
    fn prompt(&self) -> Prompt {
        if self.curve.is_none() {
            Prompt::pick("Select curve to offset", ObjFilter::Curve).opts(self.opts())
        } else {
            Prompt::point("Side to offset").opts(self.opts())
        }
    }
    fn plane_for(&self, c: &NurbsCurve, cplane: &Plane) -> Plane {
        if self.in_cplane {
            return *cplane;
        }
        curve_plane(c).filter(|_| c.degree > 1 || c.control_points.len() > 2).unwrap_or(*cplane)
    }
    fn signed(&self, c: &NurbsCurve, plane: &Plane, side: DVec3) -> f64 {
        let t = c.closest_parameter(side);
        let p = c.point_at(t);
        let tan = c.tangent_at(t);
        let left = plane.z_axis.cross(tan);
        if (side - p).dot(left) >= 0.0 {
            self.distance
        } else {
            -self.distance
        }
    }
}

impl Command for OffsetCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.distance = ctx.mem_num("Offset.Distance", 1.0);
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => match o.as_str() {
                "Distance" => {
                    self.asking = true;
                    Prompt::number("Offset distance").default_num(self.distance).step()
                }
                "BothSides" => {
                    self.both_sides = !self.both_sides;
                    self.prompt().step()
                }
                "InCPlane" => {
                    self.in_cplane = !self.in_cplane;
                    self.prompt().step()
                }
                other => {
                    ctx.log(format!("The {other} option is not implemented in triceratops yet."));
                    self.prompt().step()
                }
            },
            Input::Number(d) if self.asking => {
                self.asking = false;
                self.distance = d.abs();
                ctx.mem_set("Offset.Distance", fmt_num(self.distance));
                self.prompt().step()
            }
            Input::Number(d) if self.curve.is_some() => {
                // typed distance at the side prompt sets the distance and waits for the side
                self.distance = d.abs();
                self.prompt().step()
            }
            Input::Empty if self.asking => {
                self.asking = false;
                self.prompt().step()
            }
            Input::Pick(id, _) => {
                self.curve = Some(id);
                self.prompt().step()
            }
            Input::Point(side) => {
                let Some(id) = self.curve else { return Step::Cancelled };
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let plane = self.plane_for(&c, &ctx.cplane);
                let d = self.signed(&c, &plane, side);
                let tol = doc_tol(ctx.doc);
                let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
                ctx.doc.begin_undo("Offset");
                ctx.doc.clear_selection();
                let dists = if self.both_sides { vec![d, -d] } else { vec![d] };
                for dd in dists {
                    match c.offset(&plane, dd, tol) {
                        Ok(o) => {
                            let nid = ctx.doc.add_with(Geometry::Curve(o), attrs.clone());
                            ctx.doc.select(nid, true);
                        }
                        Err(e) => ctx.log(format!("Offset failed: {e}")),
                    }
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let (Some(id), Some(side)) = (self.curve, cursor) else { return vec![] };
        let Some(c) = curve_of(doc, id) else { return vec![] };
        let plane = self.plane_for(&c, &Plane::world_xy());
        let d = self.signed(&c, &plane, side);
        let tol = doc_tol(doc);
        let mut out = Vec::new();
        if let Ok(o) = c.offset(&plane, d, tol) {
            out.push(Preview::Curve(o));
        }
        if self.both_sides {
            if let Ok(o) = c.offset(&plane, -d, tol) {
                out.push(Preview::Curve(o));
            }
        }
        out
    }
}

pub struct RebuildCmd {
    ids: Vec<ObjectId>,
    count: usize,
    degree: usize,
    delete_input: bool,
    asking: Option<&'static str>,
}

impl Default for RebuildCmd {
    fn default() -> Self {
        Self { ids: Vec::new(), count: 10, degree: 3, delete_input: true, asking: None }
    }
}

impl RebuildCmd {
    fn prompt(&self) -> Prompt {
        Prompt::options("Rebuild options. Press Enter to rebuild").opts([optv("PointCount", self.count), optv("Degree", self.degree), optv("DeleteInput", yesno(self.delete_input))])
    }
}

impl Command for RebuildCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.count = ctx.mem_num("Rebuild.PointCount", 10.0) as usize;
        self.degree = ctx.mem_num("Rebuild.Degree", 3.0) as usize;
        Prompt::objects("Select curves to rebuild", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.prompt().step()
            }
            Input::Option(o) => match o.as_str() {
                "PointCount" => {
                    self.asking = Some("PointCount");
                    Prompt::number("Point count").default_num(self.count as f64).step()
                }
                "Degree" => {
                    self.asking = Some("Degree");
                    Prompt::number("Degree").default_num(self.degree as f64).step()
                }
                "DeleteInput" => {
                    self.delete_input = !self.delete_input;
                    self.prompt().step()
                }
                _ => self.prompt().step(),
            },
            Input::Number(v) => {
                match self.asking.take() {
                    Some("PointCount") => {
                        self.count = (v.round() as usize).clamp(2, 1000);
                        ctx.mem_set("Rebuild.PointCount", self.count);
                    }
                    Some("Degree") => {
                        self.degree = (v.round() as usize).clamp(1, 11);
                        ctx.mem_set("Rebuild.Degree", self.degree);
                    }
                    _ => {}
                }
                self.prompt().step()
            }
            Input::Empty if self.asking.is_some() => {
                self.asking = None;
                self.prompt().step()
            }
            Input::Empty => {
                ctx.doc.begin_undo("Rebuild");
                let mut n = 0;
                for id in self.ids.clone() {
                    let Some(c) = curve_of(ctx.doc, id) else { continue };
                    match c.rebuild(self.count, self.degree) {
                        Ok(r) => {
                            if self.delete_input {
                                ctx.doc.replace_geometry(id, Geometry::Curve(r));
                            } else {
                                let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
                                ctx.doc.add_with(Geometry::Curve(r), attrs);
                            }
                            n += 1;
                        }
                        Err(e) => ctx.log(format!("Rebuild failed: {e}")),
                    }
                }
                ctx.log(format!("{n} curves rebuilt."));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct DivideCmd {
    ids: Vec<ObjectId>,
    by_length: bool,
    mark_ends: bool,
}

impl Default for DivideCmd {
    fn default() -> Self {
        Self { ids: Vec::new(), by_length: false, mark_ends: true }
    }
}

impl DivideCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        if self.by_length {
            Prompt::number("Segment length").default_num(ctx.mem_num("Divide.Length", 1.0)).opts([opt("Count"), optv("MarkEnds", yesno(self.mark_ends))])
        } else {
            Prompt::number("Number of segments").default_num(ctx.mem_num("Divide.Count", 10.0)).opts([opt("Length"), optv("MarkEnds", yesno(self.mark_ends)), optv("GroupOutput", "No")])
        }
    }
}

impl Command for DivideCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves to divide", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.prompt(ctx).step()
            }
            Input::Option(o) => {
                match o.as_str() {
                    "Length" => self.by_length = true,
                    "Count" => self.by_length = false,
                    "MarkEnds" => self.mark_ends = !self.mark_ends,
                    _ => {}
                }
                self.prompt(ctx).step()
            }
            Input::Number(v) => {
                ctx.doc.begin_undo("Divide");
                ctx.doc.clear_selection();
                for id in &self.ids {
                    let Some(c) = curve_of(ctx.doc, *id) else { continue };
                    let pts = if self.by_length {
                        ctx.mem_set("Divide.Length", fmt_num(v));
                        c.divide_by_length(v.abs().max(1e-6))
                    } else {
                        ctx.mem_set("Divide.Count", (v.round() as usize).max(1));
                        c.divide_by_count(v.round().max(1.0) as usize)
                    };
                    let n = pts.len();
                    for (i, p) in pts.into_iter().enumerate() {
                        if !self.mark_ends && (i == 0 || i == n - 1) {
                            continue;
                        }
                        let nid = ctx.doc.add(Geometry::Point(p));
                        ctx.doc.select(nid, true);
                    }
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct IntersectCmd;

impl Command for IntersectCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to intersect", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        let curves: Vec<NurbsCurve> = ids.iter().filter_map(|id| curve_of(ctx.doc, *id)).collect();
        let tol = doc_tol(ctx.doc) * 10.0;
        let mut points = Vec::new();
        for i in 0..curves.len() {
            for j in i..curves.len() {
                let xs = if i == j { curve_curve_intersections(&curves[i], &curves[i], tol) } else { curve_curve_intersections(&curves[i], &curves[j], tol) };
                for x in xs {
                    if !points.iter().any(|p: &DVec3| (*p - x.point).length() < tol) {
                        points.push(x.point);
                    }
                }
            }
        }
        if points.is_empty() {
            ctx.log("No intersections found.");
            return Step::Done;
        }
        ctx.doc.begin_undo("Intersect");
        ctx.doc.clear_selection();
        for p in &points {
            let id = ctx.doc.add(Geometry::Point(*p));
            ctx.doc.select(id, true);
        }
        ctx.log(format!("Found {} intersections.", points.len()));
        Step::Done
    }
}
