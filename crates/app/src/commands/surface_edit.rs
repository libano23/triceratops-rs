//! Surface and solid editing: ExtrudeSrf, OffsetSrf, Boolean operations, and the
//! surface branches of Trim and Split.

use glam::{DAffine3, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{BooleanOp, Brep, Face, NurbsCurve, NurbsSurface};

use super::{fmt_num, opt, optv, yesno, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

/// Faces of a surface or polysurface object.
pub fn faces_of(doc: &Document, id: ObjectId) -> Option<Vec<Face>> {
    match doc.get(id).map(|o| &o.geometry) {
        Some(Geometry::Surface(s)) => Some(vec![Face::untrimmed(s.clone())]),
        Some(Geometry::Brep(b)) => Some(b.faces.clone()),
        _ => None,
    }
}

/// Index of the face closest to a pick point.
pub fn face_under(faces: &[Face], p: DVec3) -> usize {
    let mut best = (0usize, f64::INFINITY);
    for (i, f) in faces.iter().enumerate() {
        let (u, v) = f.surface.closest_uv(p);
        let d = (f.surface.point_at(u, v) - p).length();
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

/// Split a face by cutter curves projected along `dir` (exact projection, any number of
/// crossings, seams of closed surfaces handled). One piece back means "not cut".
pub fn face_pieces(face: &Face, cutters: &[NurbsCurve], dir: DVec3, tol: f64) -> Vec<Face> {
    face.split_by_curves(cutters, dir, tol)
}

/// Replace one face of an object by new faces; surfaces become polysurfaces when needed.
pub fn replace_faces(ctx: &mut CmdCtx, id: ObjectId, mut faces: Vec<Face>, index: usize, new_faces: Vec<Face>) {
    faces.remove(index);
    let at = index.min(faces.len());
    for (k, f) in new_faces.into_iter().enumerate() {
        faces.insert(at + k, f);
    }
    if faces.len() == 1 && !faces[0].is_trimmed() {
        ctx.doc.replace_geometry(id, Geometry::Surface(faces.pop().unwrap().surface));
    } else {
        ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(faces)));
    }
}

/// Trim a face of an object with curves: the piece containing `pick` is removed, the other
/// pieces stay (as a polysurface when there are several).
pub fn trim_face(ctx: &mut CmdCtx, id: ObjectId, pick: DVec3, cutters: &[NurbsCurve]) -> bool {
    let Some(faces) = faces_of(ctx.doc, id) else { return false };
    let fi = face_under(&faces, pick);
    let face = faces[fi].clone();
    let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
    let dir = ctx.view_dir;
    let pieces = face_pieces(&face, cutters, dir, tol);
    if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
        eprintln!("trim_face: dir {dir:?} pick {pick:?} cutters {} pieces {}", cutters.len(), pieces.len());
    }
    if pieces.len() < 2 {
        return false;
    }
    // the piece under the pick: project the pick onto the face along the view direction
    let pick_uv = face.uv_near(pick, dir, tol);
    let remove = pieces.iter().position(|p| p.contains_uv(pick_uv)).unwrap_or_else(|| {
        // pick off every piece (edge of the tessellation): nearest piece by its interior point
        let mut best = (0usize, f64::INFINITY);
        for (i, p) in pieces.iter().enumerate() {
            let c = p.domain_center();
            let d = (p.surface.point_at(c.x, c.y) - pick).length();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    });
    let keep: Vec<Face> = pieces.into_iter().enumerate().filter(|(i, _)| *i != remove).map(|(_, p)| p).collect();
    replace_faces(ctx, id, faces, fi, keep);
    true
}

/// Split the faces of an object with curves; returns the number of new faces.
pub fn split_faces(ctx: &mut CmdCtx, id: ObjectId, cutters: &[NurbsCurve]) -> usize {
    let Some(faces) = faces_of(ctx.doc, id) else { return 0 };
    let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
    let dir = ctx.view_dir;
    let mut out: Vec<Face> = Vec::new();
    let mut produced = 0;
    for f in faces {
        let pieces = face_pieces(&f, cutters, dir, tol);
        if pieces.len() > 1 {
            produced += pieces.len() - 1;
        }
        out.extend(pieces);
    }
    if produced == 0 {
        return 0;
    }
    let is_surface = match ctx.doc.get(id).map(|o| &o.geometry) {
        Some(Geometry::Surface(_)) => true,
        Some(Geometry::Brep(b)) => b.faces.len() == 1,
        _ => false,
    };
    let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
    if is_surface {
        // a split surface becomes separate surfaces
        ctx.doc.remove(id);
        for f in out {
            let nid = ctx.doc.add_with(Geometry::Brep(Brep { faces: vec![f], closed: false, exact: None }), attrs.clone());
            ctx.doc.select(nid, true);
        }
    } else {
        ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(out)));
        ctx.doc.select(id, true);
    }
    produced
}

/// Side faces between the edges of a face and the same edges translated by `delta`.
fn side_faces(face: &Face, delta: DVec3, tol: f64) -> Vec<Face> {
    let mut out = Vec::new();
    for poly in face.edge_polylines(tol) {
        if let Ok(c) = NurbsCurve::polyline(poly) {
            if let Ok(s) = NurbsSurface::extrude(&c, delta) {
                out.push(Face::untrimmed(s));
            }
        }
    }
    out
}

fn face_normal(face: &Face) -> DVec3 {
    face.sample().1
}

#[derive(Default)]
pub struct ExtrudeSrfCmd {
    ids: Vec<ObjectId>,
    both_sides: bool,
    solid: bool,
    delete_input: bool,
}

impl ExtrudeSrfCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        Prompt::point_or_number("Extrusion distance")
            .default_num(ctx.mem_num("ExtrudeSrf.Distance", 1.0))
            .opts([opt("Direction"), optv("BothSides", yesno(self.both_sides)), optv("Solid", yesno(self.solid)), optv("DeleteInput", yesno(self.delete_input))])
    }

    fn build(&self, doc: &Document, h: f64) -> Vec<Brep> {
        let tol = doc.settings.absolute_tolerance.max(1e-6) * 10.0;
        let mut out = Vec::new();
        for id in &self.ids {
            let Some(faces) = faces_of(doc, *id) else { continue };
            for f in faces {
                let n = face_normal(&f);
                let (start, len) = if self.both_sides { (-h, 2.0 * h) } else { (0.0, h) };
                let mut base = f.clone();
                base.transform(&DAffine3::from_translation(n * start));
                let mut top = base.clone();
                top.transform(&DAffine3::from_translation(n * len));
                let mut faces = Vec::new();
                if self.solid {
                    base.flip();
                    faces.push(base.clone());
                    faces.push(top);
                    base.flip();
                }
                faces.extend(side_faces(&base, n * len, tol));
                let mut b = Brep::new(faces);
                if self.solid {
                    b.orient_outward();
                    b.closed = true;
                }
                out.push(b);
            }
        }
        out
    }
}

impl Command for ExtrudeSrfCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.solid = ctx.mem_bool("ExtrudeSrf.Solid", true);
        Prompt::objects("Select surfaces to extrude", ObjFilter::Surface, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.prompt(ctx).step()
            }
            Input::Option(o) => {
                match o.as_str() {
                    "BothSides" => self.both_sides = !self.both_sides,
                    "Solid" => {
                        self.solid = !self.solid;
                        ctx.mem_set("ExtrudeSrf.Solid", yesno(self.solid));
                    }
                    "DeleteInput" => self.delete_input = !self.delete_input,
                    other => ctx.log(format!("The {other} option is not implemented in triceratops yet.")),
                }
                self.prompt(ctx).step()
            }
            Input::Number(h) | Input::Point(DVec3 { z: h, .. }) if !self.ids.is_empty() => {
                if h.abs() < 1e-9 {
                    return Step::Cancelled;
                }
                ctx.mem_set("ExtrudeSrf.Distance", fmt_num(h));
                let breps = self.build(ctx.doc, h);
                ctx.doc.begin_undo("ExtrudeSrf");
                if self.delete_input {
                    for id in &self.ids {
                        ctx.doc.remove(*id);
                    }
                }
                ctx.doc.clear_selection();
                for b in breps {
                    let id = ctx.doc.add(Geometry::Brep(b));
                    ctx.doc.select(id, true);
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let Some(c) = cursor else { return vec![] };
        let Some(first) = self.ids.first().and_then(|id| faces_of(doc, *id)).and_then(|f| f.into_iter().next()) else { return vec![] };
        let (p, n) = first.sample();
        let h = (c - p).dot(n);
        self.build(doc, h).into_iter().map(Preview::Brep).collect()
    }
}

#[derive(Default)]
pub struct OffsetSrfCmd {
    ids: Vec<ObjectId>,
    solid: bool,
    both_sides: bool,
    delete_input: bool,
    flip: bool,
}

impl OffsetSrfCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        Prompt::number("Offset distance").default_num(ctx.mem_num("OffsetSrf.Distance", 1.0)).opts([
            opt("FlipAll"),
            optv("Solid", yesno(self.solid)),
            optv("Loose", "No"),
            optv("Tolerance", "0.001"),
            optv("BothSides", yesno(self.both_sides)),
            optv("DeleteInput", yesno(self.delete_input)),
            optv("Corner", "Sharp"),
        ])
    }

    pub fn offset_face(face: &Face, d: f64) -> Face {
        let s = &face.surface;
        if s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2 {
            let mut f = face.clone();
            f.transform(&DAffine3::from_translation(face_normal(face) * d));
            f
        } else {
            Face { surface: s.offset(d), loops: face.loops.clone() }
        }
    }

    fn build(&self, doc: &Document, d: f64) -> Vec<Geometry> {
        let tol = doc.settings.absolute_tolerance.max(1e-6) * 10.0;
        let d = if self.flip { -d } else { d };
        let mut out = Vec::new();
        for id in &self.ids {
            let Some(faces) = faces_of(doc, *id) else { continue };
            for f in faces {
                let offs = Self::offset_face(&f, d);
                if self.solid {
                    let mut base = f.clone();
                    base.flip();
                    let mut all = vec![base, offs.clone()];
                    // ruled sides between corresponding edges
                    let e0 = f.edge_polylines(tol);
                    let e1 = offs.edge_polylines(tol);
                    for (a, b) in e0.iter().zip(e1.iter()) {
                        if let (Ok(ca), Ok(cb)) = (NurbsCurve::polyline(a.clone()), NurbsCurve::polyline(b.clone())) {
                            if let Ok(s) = NurbsSurface::loft(&[ca, cb], 1) {
                                all.push(Face::untrimmed(s));
                            }
                        }
                    }
                    let mut b = Brep::new(all);
                    b.orient_outward();
                    b.closed = true;
                    out.push(Geometry::Brep(b));
                } else if offs.is_trimmed() {
                    out.push(Geometry::Brep(Brep { faces: vec![offs], closed: false, exact: None }));
                } else {
                    out.push(Geometry::Surface(offs.surface));
                }
            }
        }
        out
    }
}

impl Command for OffsetSrfCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select surfaces or polysurfaces to offset", ObjFilter::Surface, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.prompt(ctx).step()
            }
            Input::Option(o) => {
                match o.as_str() {
                    "Solid" => self.solid = !self.solid,
                    "BothSides" => self.both_sides = !self.both_sides,
                    "DeleteInput" => self.delete_input = !self.delete_input,
                    "FlipAll" => self.flip = !self.flip,
                    other => ctx.log(format!("The {other} option is not implemented in triceratops yet.")),
                }
                self.prompt(ctx).step()
            }
            Input::Number(d) if !self.ids.is_empty() => {
                ctx.mem_set("OffsetSrf.Distance", fmt_num(d));
                let mut geoms = self.build(ctx.doc, d);
                if self.both_sides {
                    geoms.extend(self.build(ctx.doc, -d));
                }
                ctx.doc.begin_undo("OffsetSrf");
                if self.delete_input {
                    for id in &self.ids {
                        ctx.doc.remove(*id);
                    }
                }
                ctx.doc.clear_selection();
                for g in geoms {
                    let id = ctx.doc.add(g);
                    ctx.doc.select(id, true);
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct BooleanCmd {
    op: BooleanOp,
    first: Vec<ObjectId>,
    delete_input: bool,
}

impl BooleanCmd {
    pub fn new(op: BooleanOp) -> Self {
        Self { op, first: Vec::new(), delete_input: true }
    }
    fn label(&self) -> &'static str {
        match self.op {
            BooleanOp::Union => "BooleanUnion",
            BooleanOp::Difference => "BooleanDifference",
            BooleanOp::Intersection => "BooleanIntersection",
        }
    }
    fn finish(&self, ctx: &mut CmdCtx, second: &[ObjectId]) -> Step {
        use super::misc4::geometry_nonempty;
        let closed_geometry = |ctx: &mut CmdCtx, id: ObjectId| -> Option<Geometry> {
            let o = ctx.doc.get(id)?;
            let closed = match &o.geometry {
                Geometry::Surface(s) => tri_doc::surface_is_closed(s),
                Geometry::Brep(b) => b.closed,
                Geometry::Mesh(m) => m.is_closed(),
                _ => return None,
            };
            if !closed {
                ctx.log("Boolean operations need closed solids; an open object was skipped.");
                return None;
            }
            Some(o.geometry.clone())
        };
        let mut set_a: Vec<Geometry> = self.first.iter().filter_map(|id| closed_geometry(ctx, *id)).collect();
        // an object cannot be combined with itself
        let second: Vec<ObjectId> = second.iter().copied().filter(|id| !self.first.contains(id)).collect();
        let second = &second[..];
        let mut set_b: Vec<Geometry> = second.iter().filter_map(|id| closed_geometry(ctx, *id)).collect();
        if set_a.is_empty() || (self.op != BooleanOp::Union && set_b.is_empty()) {
            ctx.log(format!("{} failed: nothing to combine.", self.label()));
            return Step::Cancelled;
        }
        let (op, label, delete_input) = (self.op, self.label(), self.delete_input);
        let inputs: Vec<ObjectId> = self.first.iter().chain(second.iter()).copied().collect();
        ctx.work(
            label,
            move || -> Result<Geometry, String> {
                use super::misc4::boolean_geometry_checked;
                let mut result = set_a.remove(0);
                let unite = if op == BooleanOp::Union { std::mem::take(&mut set_b) } else { Vec::new() };
                for g in set_a.into_iter().chain(unite) {
                    result = boolean_geometry_checked(&result, &g, BooleanOp::Union)?;
                }
                for g in set_b {
                    result = boolean_geometry_checked(&result, &g, op)?;
                }
                Ok(result)
            },
            move |result, ctx| {
                let result = match result {
                    Ok(r) if geometry_nonempty(&r) => r,
                    Ok(_) => {
                        ctx.log(format!("{label} produced an empty result."));
                        return Step::Cancelled;
                    }
                    Err(e) => {
                        ctx.log(format!("{label} failed: {e}."));
                        return Step::Cancelled;
                    }
                };
                ctx.doc.begin_undo(label);
                if delete_input {
                    for id in &inputs {
                        ctx.doc.remove(*id);
                    }
                }
                ctx.doc.clear_selection();
                let kind = result.type_name().to_string();
                let id = ctx.doc.add(result);
                ctx.doc.select(id, true);
                ctx.log(format!("{label}: created 1 {kind}."));
                Step::Done
            },
        )
    }
}

impl Command for BooleanCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        match self.op {
            BooleanOp::Union => Prompt::objects("Select surfaces or polysurfaces to union", ObjFilter::Any, 1).opts([optv("DeleteInput", yesno(self.delete_input))]).step(),
            BooleanOp::Difference => Prompt::objects("Select surfaces or polysurfaces to subtract from", ObjFilter::Any, 1).opts([optv("DeleteInput", yesno(self.delete_input))]).step(),
            BooleanOp::Intersection => Prompt::objects("Select first set of surfaces or polysurfaces", ObjFilter::Any, 1).opts([optv("DeleteInput", yesno(self.delete_input))]).step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(_) => {
                self.delete_input = !self.delete_input;
                self.start(ctx)
            }
            Input::Objects(ids) if self.first.is_empty() => {
                self.first = ids;
                ctx.doc.clear_selection();
                match self.op {
                    BooleanOp::Union => self.finish(ctx, &[]),
                    BooleanOp::Difference => Prompt::objects("Select surfaces or polysurfaces to subtract with", ObjFilter::Any, 1).opts([optv("DeleteInput", yesno(self.delete_input))]).step(),
                    BooleanOp::Intersection => Prompt::objects("Select second set of surfaces or polysurfaces", ObjFilter::Any, 1).opts([optv("DeleteInput", yesno(self.delete_input))]).step(),
                }
            }
            Input::Objects(ids) => self.finish(ctx, &ids),
            _ => Step::Cancelled,
        }
    }
}
