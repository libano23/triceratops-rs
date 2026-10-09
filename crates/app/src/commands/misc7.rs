//! Seventh batch: ClippingPlane family, NamedPosition/Snapshots, user text, ReadCommandFile,
//! DeleteFaces/MoveFace/MoveEdge (planar solids), ExtrudeSrfToPoint/Tapered, OffsetMultiple.

use glam::{DAffine3, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{curve_plane, Brep, Face, NurbsCurve, NurbsSurface, Plane};

use super::create::planar_cap;
use super::solid_edit::planar_loops;
use super::surface2::extrude_to_point;
use super::surface_edit::{face_under, faces_of};
use super::{fmt_num, opt, optv, AppRequest, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

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

fn clipping_ids(ctx: &CmdCtx) -> Vec<ObjectId> {
    ctx.memory.get("ClippingPlanes").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default()
}

// ---- ClippingPlane --------------------------------------------------------------------------------

#[derive(Default)]
pub struct ClippingPlaneCmd {
    pts: Vec<DVec3>,
    plane: Plane,
}

impl Command for ClippingPlaneCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::point("Start of clipping plane").opts([opt("3Point"), opt("Vertical")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                if self.pts.len() < 2 {
                    return Prompt::point("End of clipping plane").step();
                }
                let (u0, v0, w) = self.plane.closest_parameters(self.pts[0]);
                let (u1, v1, _) = self.plane.closest_parameters(self.pts[1]);
                let plane = Plane { origin: self.plane.origin + self.plane.z_axis * w, ..self.plane };
                let Ok(s) = NurbsSurface::plane(&plane, u0.min(u1), u0.max(u1), v0.min(v1), v0.max(v1)) else { return Step::Cancelled };
                ctx.doc.begin_undo("ClippingPlane");
                ctx.doc.clear_selection();
                let id = ctx.doc.add_with(Geometry::Surface(s), tri_doc::Attributes { layer: ctx.doc.current_layer(), name: Some("Clipping Plane".into()), ..Default::default() });
                ctx.doc.select(id, true);
                ctx.requests.push(AppRequest::AddClippingPlane(id));
                Step::Done
            }
            Input::Option(o) => {
                if o == "Vertical" {
                    self.plane = Plane::from_frame(ctx.cplane.origin, ctx.cplane.x_axis, ctx.cplane.z_axis);
                }
                Prompt::point("Start of clipping plane").step()
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.first(), cursor) {
            (Some(a), Some(c)) => vec![Preview::Polyline(super::create::rect_points(&self.plane, *a, c))],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

pub struct ClipStateCmd(pub &'static str);

impl Command for ClipStateCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let ids = clipping_ids(ctx);
        match self.0 {
            "SelClippingPlane" => {
                for id in &ids {
                    ctx.doc.select(*id, true);
                }
                ctx.log(format!("{} clipping planes added to selection.", ids.len()));
                Step::Done
            }
            "EnableClippingPlane" | "DisableClippingPlane" | "Flip" => {
                if ids.is_empty() {
                    ctx.log("No clipping planes in the document.");
                    return Step::Done;
                }
                Prompt::objects("Select clipping planes. Press Enter for all", ObjFilter::Surface, 0).step()
            }
            _ => Step::Done,
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let sel = match input {
            Input::Objects(ids) => ids,
            Input::Empty => Vec::new(),
            _ => return Step::Cancelled,
        };
        let known = clipping_ids(ctx);
        let ids: Vec<ObjectId> = sel.into_iter().filter(|id| known.contains(id)).collect();
        ctx.requests.push(AppRequest::SetClippingPlanes { ids, enabled: self.0 == "EnableClippingPlane", flip: self.0 == "Flip" });
        Step::Done
    }
}

// ---- NamedPosition / Snapshots ----------------------------------------------------------------------

pub struct NamedPositionCmd(pub bool);

impl Command for NamedPositionCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let prefix = if self.0 { "Snapshot." } else { "NamedPosition." };
        let names: Vec<String> = ctx.memory.keys().filter_map(|k| k.strip_prefix(prefix).map(|s| s.to_string())).collect();
        if !names.is_empty() {
            ctx.log(format!("Saved: {}", names.join(", ")));
        }
        Prompt::options(if self.0 { "Snapshots" } else { "Named positions" }).opts([opt("Save"), opt("Restore"), opt("Delete")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let prefix = if self.0 { "Snapshot." } else { "NamedPosition." };
        match input {
            Input::Option(o) => {
                ctx.mem_set("NamedPosition.Mode", o.clone());
                if o == "Save" && !self.0 && ctx.doc.selected_ids().is_empty() {
                    return Prompt::objects("Select objects to save position", ObjFilter::Any, 1).step();
                }
                Prompt::text("Name").step()
            }
            Input::Objects(_) => Prompt::text("Name").step(),
            Input::Text(name) => {
                let key = format!("{prefix}{}", name.trim());
                match ctx.memory.get("NamedPosition.Mode").map(|s| s.as_str()) {
                    Some("Save") => {
                        let ids = if self.0 { ctx.doc.objects().map(|o| o.id).collect::<Vec<_>>() } else { ctx.doc.selected_ids() };
                        let data: Vec<(ObjectId, Geometry)> = ids.iter().filter_map(|id| ctx.doc.get(*id).map(|o| (*id, o.geometry.clone()))).collect();
                        match serde_json::to_string(&data) {
                            Ok(json) => {
                                ctx.mem_set(&key, json);
                                if self.0 {
                                    ctx.requests.push(AppRequest::NamedView { name: format!("Snapshot {}", name.trim()), restore: false });
                                }
                                ctx.log(format!("{} \"{}\" saved ({} objects).", if self.0 { "Snapshot" } else { "Position" }, name.trim(), data.len()));
                            }
                            Err(e) => ctx.log(format!("Unable to save: {e}")),
                        }
                    }
                    Some("Restore") => {
                        let Some(json) = ctx.memory.get(&key).cloned() else {
                            ctx.log(format!("No saved entry named \"{}\".", name.trim()));
                            return Step::Cancelled;
                        };
                        let data: Vec<(ObjectId, Geometry)> = serde_json::from_str(&json).unwrap_or_default();
                        ctx.doc.begin_undo(if self.0 { "Snapshots" } else { "NamedPosition" });
                        let mut n = 0;
                        for (id, g) in data {
                            if ctx.doc.replace_geometry(id, g) {
                                n += 1;
                            }
                        }
                        if self.0 {
                            ctx.requests.push(AppRequest::NamedView { name: format!("Snapshot {}", name.trim()), restore: true });
                        }
                        ctx.log(format!("{n} objects restored."));
                    }
                    _ => {
                        ctx.memory.remove(&key);
                    }
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- user text ------------------------------------------------------------------------------------------

pub struct UserTextCmd(pub &'static str);

impl Command for UserTextCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.0 {
            "SetUserText" | "GetUserText" => Prompt::objects("Select objects", ObjFilter::Any, 1).step(),
            "SetDocumentUserText" => Prompt::text("Key").step(),
            _ => {
                // GetDocumentUserText / DocumentText
                if ctx.doc.settings.user_text.is_empty() {
                    ctx.log("No document user text.");
                }
                let lines: Vec<String> = ctx.doc.settings.user_text.iter().map(|(k, v)| format!("  {k} = {v}")).collect();
                for l in lines {
                    ctx.log(l);
                }
                Step::Done
            }
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.0, input) {
            ("GetUserText", Input::Objects(ids)) => {
                for id in ids {
                    let lines: Vec<String> = match ctx.doc.get(id) {
                        Some(o) if o.attributes.user_text.is_empty() => vec![format!("Object #{id}: no user text.")],
                        Some(o) => o.attributes.user_text.iter().map(|(k, v)| format!("Object #{id}: {k} = {v}")).collect(),
                        None => Vec::new(),
                    };
                    for l in lines {
                        ctx.log(l);
                    }
                }
                Step::Done
            }
            ("SetUserText", Input::Objects(ids)) => {
                ctx.mem_set("UserText.Ids", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
                Prompt::text("Key").step()
            }
            (_, Input::Text(t)) => {
                if ctx.memory.get("UserText.Key").is_none() {
                    if t.trim().is_empty() {
                        return Step::Cancelled;
                    }
                    ctx.mem_set("UserText.Key", t.trim());
                    return Prompt::text("Value").step();
                }
                let key = ctx.memory.remove("UserText.Key").unwrap_or_default();
                let value = t.trim().to_string();
                if self.0 == "SetDocumentUserText" {
                    ctx.doc.begin_undo("SetDocumentUserText");
                    ctx.doc.settings.user_text.retain(|(k, _)| k != &key);
                    if !value.is_empty() {
                        ctx.doc.settings.user_text.push((key, value));
                    }
                } else {
                    let ids: Vec<ObjectId> = ctx.memory.get("UserText.Ids").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                    ctx.doc.begin_undo("SetUserText");
                    for id in ids {
                        if let Some(o) = ctx.doc.get_mut(id) {
                            o.attributes.user_text.retain(|(k, _)| k != &key);
                            if !value.is_empty() {
                                o.attributes.user_text.push((key.clone(), value.clone()));
                            }
                        }
                    }
                }
                Step::Done
            }
            (_, Input::Empty) => {
                ctx.memory.remove("UserText.Key");
                Step::Cancelled
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct ReadCommandFileCmd;

impl Command for ReadCommandFileCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::text("Command file name").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Text(t) if !t.trim().is_empty() => {
                ctx.requests.push(AppRequest::RunCommandFile(t.trim().to_string()));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- planar solid editing: DeleteFaces, MoveFace, MoveEdge -------------------------------------------

pub struct DeleteFacesCmd;

impl Command for DeleteFacesCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select faces to delete. Press Enter when done", ObjFilter::Polysurface).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                let Some(faces) = faces_of(ctx.doc, id) else { return Step::Cancelled };
                if faces.len() < 2 {
                    ctx.log("Deleting the only face would delete the object.");
                    return Step::Done;
                }
                let fi = face_under(&faces, p);
                let mut rest = faces;
                rest.remove(fi);
                ctx.doc.begin_undo("DeleteFaces");
                ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(rest)));
                Prompt::pick("Select faces to delete. Press Enter when done", ObjFilter::Polysurface).step()
            }
            Input::Empty => Step::Done,
            _ => Step::Cancelled,
        }
    }
}

/// Translate the vertices of planar faces that satisfy `moves`, rebuilding each face on its plane.
fn move_vertices(faces: &[Face], moves: &dyn Fn(DVec3) -> Option<DVec3>) -> Option<Vec<Face>> {
    let mut out = Vec::with_capacity(faces.len());
    for f in faces {
        let Some((plane, loops)) = planar_loops(f) else {
            out.push(f.clone());
            continue;
        };
        let mut changed = false;
        let new_loops: Vec<Vec<DVec3>> = loops
            .iter()
            .map(|l| {
                l.iter()
                    .map(|p| match moves(*p) {
                        Some(q) => {
                            changed = true;
                            q
                        }
                        None => *p,
                    })
                    .collect()
            })
            .collect();
        if !changed {
            out.push(f.clone());
            continue;
        }
        // new plane through three non-collinear moved vertices (the face may tilt)
        let poly = &new_loops[0];
        let mut normal = DVec3::ZERO;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            normal += a.cross(b);
        }
        let normal = normal.normalize_or(plane.z_axis);
        let normal = if normal.dot(plane.z_axis) < 0.0 { -normal } else { normal };
        let origin = poly.iter().fold(DVec3::ZERO, |a, p| a + *p) / poly.len() as f64;
        let new_plane = Plane::from_normal(origin, normal);
        out.push(Face::planar(&new_plane, new_loops).ok()?);
    }
    Some(out)
}

pub struct MoveFaceCmd {
    edge: bool,
    pick: Option<(ObjectId, DVec3)>,
    base: Option<DVec3>,
}

impl MoveFaceCmd {
    pub fn new(edge: bool) -> Self {
        Self { edge, pick: None, base: None }
    }
    fn moved(&self, doc: &Document, delta: DVec3) -> Option<Vec<Face>> {
        let (id, p) = self.pick?;
        let faces = faces_of(doc, id)?;
        let bb = faces.iter().fold(tri_geom::BoundingBox::EMPTY, |b, f| b.union(&f.bounding_box()));
        let tol = bb.diagonal().max(1e-6) * 1e-6;
        if self.edge {
            // nearest planar edge: its two endpoints move
            let mut best: Option<(f64, DVec3, DVec3)> = None;
            for f in &faces {
                let Some((_, loops)) = planar_loops(f) else { continue };
                for l in &loops {
                    for i in 0..l.len() {
                        let (a, b) = (l[i], l[(i + 1) % l.len()]);
                        let ab = b - a;
                        let t = if ab.length_squared() < 1e-18 { 0.0 } else { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) };
                        let d = (a + ab * t - p).length();
                        if best.map_or(true, |bst| d < bst.0) {
                            best = Some((d, a, b));
                        }
                    }
                }
            }
            let (_, a, b) = best?;
            move_vertices(&faces, &|v| if (v - a).length() < tol || (v - b).length() < tol { Some(v + delta) } else { None })
        } else {
            let fi = face_under(&faces, p);
            let (plane, _) = planar_loops(&faces[fi])?;
            move_vertices(&faces, &|v| if (v - plane.origin).dot(plane.z_axis).abs() < tol * 100.0 { Some(v + delta) } else { None })
        }
    }
}

impl Command for MoveFaceCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick(if self.edge { "Select edge to move" } else { "Select face to move" }, ObjFilter::Polysurface).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.pick, self.base, input) {
            (None, _, Input::Pick(id, p)) => {
                self.pick = Some((id, p));
                Prompt::point("Point to move from").step()
            }
            (Some(_), None, Input::Point(p)) => {
                self.base = Some(p);
                Prompt::point("Point to move to").step()
            }
            (Some((id, _)), Some(b), Input::Point(p)) => match self.moved(ctx.doc, p - b) {
                Some(faces) => {
                    ctx.doc.begin_undo(if self.edge { "MoveEdge" } else { "MoveFace" });
                    ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(faces)));
                    Step::Done
                }
                None => {
                    ctx.log(format!("{} works on polysurfaces with planar faces in triceratops.", if self.edge { "MoveEdge" } else { "MoveFace" }));
                    Step::Cancelled
                }
            },
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.base, cursor) {
            (Some(b), Some(c)) => self.moved(doc, c - b).map(|f| vec![Preview::Brep(Brep { faces: f, closed: false })]).unwrap_or_default(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

// ---- ExtrudeSrfToPoint / ExtrudeSrfTapered ----------------------------------------------------------------

pub struct ExtrudeSrfVariantCmd {
    tapered: bool,
    ids: Vec<ObjectId>,
}

impl ExtrudeSrfVariantCmd {
    pub fn new(tapered: bool) -> Self {
        Self { tapered, ids: Vec::new() }
    }
}

impl Command for ExtrudeSrfVariantCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select planar surfaces to extrude", ObjFilter::Surface, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                if self.tapered {
                    Prompt::number("Extrusion distance").default_num(ctx.mem_num("ExtrudeTapered.Distance", 1.0)).opts([optv("DraftAngle", fmt_num(ctx.mem_num("ExtrudeTapered.Angle", 10.0)))]).step()
                } else {
                    Prompt::point("Point to extrude to").step()
                }
            }
            Input::Option(_) => Prompt::number("Draft angle").default_num(ctx.mem_num("ExtrudeTapered.Angle", 10.0)).step(),
            inp @ (Input::Point(_) | Input::Number(_)) => {
                let mut out = Vec::new();
                let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
                for id in &self.ids {
                    for f in faces_of(ctx.doc, *id).unwrap_or_default() {
                        let Some((plane, loops)) = planar_loops(&f) else { continue };
                        let mut outer = loops[0].clone();
                        outer.push(loops[0][0]);
                        let Ok(c) = NurbsCurve::polyline(outer) else { continue };
                        match &inp {
                            Input::Point(apex) => {
                                let Some(side) = extrude_to_point(&c, *apex) else { continue };
                                let mut brep = Brep::new(vec![Face::untrimmed(side), f.clone()]);
                                brep.orient_outward();
                                brep.closed = true;
                                out.push(Geometry::Brep(brep));
                            }
                            Input::Number(h) => {
                                let angle = ctx.mem_num("ExtrudeTapered.Angle", 10.0);
                                let d = -h * angle.to_radians().tan();
                                let Ok(top) = c.offset(&plane, d, tol) else { continue };
                                let top = top.transformed(&DAffine3::from_translation(plane.z_axis * *h));
                                let Ok(side) = NurbsSurface::loft(&[c.clone(), top.clone()], 1) else { continue };
                                let (Some(a), Some(b)) = (planar_cap(&c), planar_cap(&top)) else { continue };
                                let mut brep = Brep::new(vec![Face::untrimmed(side), a, b]);
                                brep.orient_outward();
                                brep.closed = true;
                                out.push(Geometry::Brep(brep));
                            }
                            _ => {}
                        }
                    }
                }
                add_all(ctx, if self.tapered { "ExtrudeSrfTapered" } else { "ExtrudeSrfToPoint" }, out)
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- OffsetMultiple ------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct OffsetMultipleCmd {
    ids: Vec<ObjectId>,
    distance: f64,
}

impl Command for OffsetMultipleCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.distance = ctx.mem_num("Offset.Distance", 1.0);
        Prompt::objects("Select curves to offset", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::point("Side to offset").opts([optv("Distance", fmt_num(self.distance))]).step()
            }
            Input::Option(_) => Prompt::number("Offset distance").default_num(self.distance).step(),
            Input::Number(d) => {
                self.distance = d.abs();
                ctx.mem_set("Offset.Distance", fmt_num(self.distance));
                Prompt::point("Side to offset").opts([optv("Distance", fmt_num(self.distance))]).step()
            }
            Input::Point(side) => {
                let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
                let mut out = Vec::new();
                for id in &self.ids {
                    let Some(c) = ctx.doc.get(*id).and_then(|o| o.geometry.as_curve().cloned()) else { continue };
                    let plane = curve_plane(&c).filter(|_| c.degree > 1 || c.control_points.len() > 2).unwrap_or(ctx.cplane);
                    let t = c.closest_parameter(side);
                    let left = plane.z_axis.cross(c.tangent_at(t));
                    let d = if (side - c.point_at(t)).dot(left) >= 0.0 { self.distance } else { -self.distance };
                    if let Ok(o) = c.offset(&plane, d, tol) {
                        out.push(Geometry::Curve(o));
                    }
                }
                add_all(ctx, "OffsetMultiple", out)
            }
            _ => Step::Cancelled,
        }
    }
}
