//! Sixth batch: Pull, Fin/OffsetNormal, InterpCrvOnSrf, CrvDeviation, Diameter, MakeUniformUV,
//! ExtractMeshPart, Maelstrom, Distribute, layer utilities, MergeAllCoplanarFaces, moments,
//! FilletSrf (planar surfaces), file/UI requests.

use glam::{DQuat, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{Brep, Face, NurbsCurve, NurbsSurface, Plane};

use super::misc4::region_boolean;
use super::surface_edit::{face_under, faces_of, trim_face};
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

/// Closest point on any face of a surface object and the surface normal there.
pub fn pull_point(faces: &[Face], p: DVec3) -> (DVec3, DVec3) {
    let fi = face_under(faces, p);
    let s = &faces[fi].surface;
    let (u, v) = s.closest_uv(p);
    (s.point_at(u, v), s.normal_at(u, v))
}

pub fn pulled_curve(c: &NurbsCurve, faces: &[Face]) -> Option<NurbsCurve> {
    let tol = c.bounding_box().diagonal().max(1e-6) * 1e-3;
    let mut samples = c.tessellate_adaptive(tol);
    if samples.len() < 33 {
        let (t0, t1) = c.domain();
        samples = (0..=32).map(|i| c.point_at(t0 + (t1 - t0) * i as f64 / 32.0)).collect();
    }
    let pts: Vec<DVec3> = samples.iter().map(|p| pull_point(faces, *p).0).collect();
    if c.degree == 1 {
        NurbsCurve::polyline(pts).ok()
    } else {
        NurbsCurve::interpolate(&pts, 3).ok()
    }
}

// ---- Pull / Fin / InterpCrvOnSrf -------------------------------------------------------------

#[derive(Default)]
pub struct PullCmd {
    curves: Vec<ObjectId>,
}

impl Command for PullCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves and points to pull", ObjFilter::Any, 1).opts([optv("DeleteInput", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.curves.is_empty() => {
                self.curves = ids;
                ctx.doc.clear_selection();
                Prompt::pick("Select surface to pull to", ObjFilter::Surface).step()
            }
            Input::Pick(id, _) => {
                let Some(faces) = faces_of(ctx.doc, id) else { return Step::Cancelled };
                let mut out = Vec::new();
                for cid in &self.curves {
                    match ctx.doc.get(*cid).map(|o| &o.geometry) {
                        Some(Geometry::Curve(c)) => {
                            if let Some(pc) = pulled_curve(c, &faces) {
                                out.push(Geometry::Curve(pc));
                            }
                        }
                        Some(Geometry::Point(p)) => out.push(Geometry::Point(pull_point(&faces, *p).0)),
                        _ => {}
                    }
                }
                add_all(ctx, "Pull", out)
            }
            Input::Option(_) => self.start(ctx),
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct FinCmd {
    curves: Vec<ObjectId>,
    surface: Option<ObjectId>,
}

impl Command for FinCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves on surface", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.surface, input) {
            (_, Input::Objects(ids)) => {
                self.curves = ids;
                ctx.doc.clear_selection();
                Prompt::pick("Select base surface", ObjFilter::Surface).step()
            }
            (None, Input::Pick(id, _)) => {
                self.surface = Some(id);
                Prompt::number("Distance").default_num(ctx.mem_num("Fin.Distance", 1.0)).opts([optv("Direction", "Normal"), optv("BothSides", "No")]).step()
            }
            (Some(sid), Input::Number(d)) => {
                ctx.mem_set("Fin.Distance", fmt_num(d));
                let Some(faces) = faces_of(ctx.doc, sid) else { return Step::Cancelled };
                let mut out = Vec::new();
                for cid in &self.curves {
                    let Some(c) = curve_of(ctx.doc, *cid) else { continue };
                    let tol = c.bounding_box().diagonal().max(1e-6) * 1e-3;
                    let base = c.tessellate_adaptive(tol);
                    let offset: Vec<DVec3> = base.iter().map(|p| { let (q, n) = pull_point(&faces, *p); q + n * d }).collect();
                    let (Ok(a), Ok(b)) = (NurbsCurve::polyline(base), NurbsCurve::polyline(offset)) else { continue };
                    if let Ok(s) = NurbsSurface::loft(&[a, b], 1) {
                        out.push(Geometry::Surface(s));
                    }
                }
                add_all(ctx, "Fin", out)
            }
            (_, Input::Option(_)) => Prompt::number("Distance").default_num(ctx.mem_num("Fin.Distance", 1.0)).step(),
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct InterpCrvOnSrfCmd {
    surface: Option<ObjectId>,
    pts: Vec<DVec3>,
}

impl Command for InterpCrvOnSrfCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select surface to draw curve on", ObjFilter::Surface).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.surface, input) {
            (None, Input::Pick(id, _)) => {
                self.surface = Some(id);
                Prompt::point("Start of curve").step()
            }
            (Some(sid), Input::Point(p)) => {
                let faces = faces_of(ctx.doc, sid).unwrap_or_default();
                self.pts.push(if faces.is_empty() { p } else { pull_point(&faces, p).0 });
                Prompt::point("Next point. Press Enter when done").opts([opt("Undo")]).step()
            }
            (Some(sid), Input::Empty) => {
                if self.pts.len() < 2 {
                    return Step::Cancelled;
                }
                let faces = faces_of(ctx.doc, sid).unwrap_or_default();
                let Ok(c) = NurbsCurve::interpolate(&self.pts, 3) else { return Step::Cancelled };
                let pulled = pulled_curve(&c, &faces).unwrap_or(c);
                add_all(ctx, "InterpCrvOnSrf", vec![Geometry::Curve(pulled)])
            }
            (_, Input::Option(_)) => {
                self.pts.pop();
                Prompt::point("Next point. Press Enter when done").step()
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let mut pts = self.pts.clone();
        if let Some(c) = cursor {
            pts.push(c);
        }
        if pts.len() >= 2 {
            NurbsCurve::interpolate(&pts, 3).map(|c| vec![Preview::Curve(c)]).unwrap_or_default()
        } else {
            vec![]
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

// ---- analysis: CrvDeviation, Diameter, moments ----------------------------------------------------

#[derive(Default)]
pub struct CrvDeviationCmd {
    first: Option<ObjectId>,
}

impl Command for CrvDeviationCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select first curve", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, input) {
            (None, Input::Pick(id, _)) => {
                self.first = Some(id);
                Prompt::pick("Select second curve", ObjFilter::Curve).step()
            }
            (Some(a), Input::Pick(b, _)) => {
                let (Some(ca), Some(cb)) = (curve_of(ctx.doc, a), curve_of(ctx.doc, b)) else { return Step::Cancelled };
                let n = 200;
                let (t0, t1) = ca.domain();
                let (mut dmin, mut dmax, mut sum) = (f64::INFINITY, 0.0f64, 0.0);
                let (mut pmin, mut pmax) = (DVec3::ZERO, DVec3::ZERO);
                for i in 0..=n {
                    let p = ca.point_at(t0 + (t1 - t0) * i as f64 / n as f64);
                    let q = cb.point_at(cb.closest_parameter(p));
                    let d = (p - q).length();
                    sum += d;
                    if d < dmin {
                        dmin = d;
                        pmin = p;
                    }
                    if d > dmax {
                        dmax = d;
                        pmax = p;
                    }
                }
                ctx.log(format!("Minimum deviation = {}, maximum deviation = {}, average = {}", fmt_num(dmin), fmt_num(dmax), fmt_num(sum / (n + 1) as f64)));
                add_all(ctx, "CrvDeviation", vec![Geometry::Point(pmin), Geometry::Point(pmax)])
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct DiameterCmd;

impl Command for DiameterCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select curve for diameter measurement", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Pick(id, _) = input else { return Step::Cancelled };
        let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
        let (t0, t1) = c.domain();
        let span = t1 - t0;
        match tri_geom::curve::circle_through(c.point_at(t0 + span * 0.15), c.point_at(t0 + span * 0.5), c.point_at(t0 + span * 0.85)) {
            Ok((center, r, _)) => ctx.log(format!("Diameter = {} (radius {}, center {}, {}, {})", fmt_num(2.0 * r), fmt_num(r), fmt_num(center.x), fmt_num(center.y), fmt_num(center.z))),
            Err(_) => ctx.log("The curve is not an arc or circle."),
        }
        Step::Done
    }
}

pub struct MomentsCmd(pub bool);

impl Command for MomentsCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects(if self.0 { "Select surfaces, polysurfaces or meshes for volume moments" } else { "Select closed curves, surfaces or meshes for area moments" }, ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        for id in ids {
            let Some(o) = ctx.doc.get(id) else { continue };
            let bb = ctx.doc.object_bbox(o);
            if self.0 {
                if let Some((v, c)) = Document::volume_of(&o.geometry) {
                    ctx.log(format!("Volume = {}", fmt_num(v)));
                    ctx.log(format!("Centroid = ({}, {}, {})", fmt_num(c.x), fmt_num(c.y), fmt_num(c.z)));
                    let d = bb.max - bb.min;
                    let (ix, iy, iz) = (v * (d.y * d.y + d.z * d.z) / 12.0, v * (d.x * d.x + d.z * d.z) / 12.0, v * (d.x * d.x + d.y * d.y) / 12.0);
                    ctx.log(format!("Moments of inertia about centroid axes (box approximation) = {}, {}, {}", fmt_num(ix), fmt_num(iy), fmt_num(iz)));
                }
            } else if let Some(a) = Document::area_of(&o.geometry) {
                let c = bb.center();
                ctx.log(format!("Area = {}", fmt_num(a)));
                ctx.log(format!("Area centroid = ({}, {}, {})", fmt_num(c.x), fmt_num(c.y), fmt_num(c.z)));
                let d = bb.max - bb.min;
                ctx.log(format!("Area moments of inertia (box approximation) = {}, {}", fmt_num(a * d.y * d.y / 12.0), fmt_num(a * d.x * d.x / 12.0)));
            }
        }
        Step::Done
    }
}

// ---- MakeUniformUV, ExtractMeshPart --------------------------------------------------------------------

pub struct MakeUniformUVCmd;

impl Command for MakeUniformUVCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select surfaces to make uniform", ObjFilter::Surface, 1).opts([optv("Direction", "Both")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                ctx.doc.begin_undo("MakeUniformUV");
                for id in ids {
                    let Some(Geometry::Surface(mut s)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
                    let uniform = |n: usize, p: usize| -> Vec<f64> {
                        let spans = n - p;
                        let mut k = vec![0.0; p + 1];
                        for i in 1..spans {
                            k.push(i as f64 / spans as f64);
                        }
                        k.extend(std::iter::repeat(1.0).take(p + 1));
                        k
                    };
                    let (ku, kv) = (uniform(s.count_u, s.degree_u), uniform(s.count_v, s.degree_v));
                    if ku.len() == s.knots_u.len() && kv.len() == s.knots_v.len() {
                        s.knots_u = ku;
                        s.knots_v = kv;
                        ctx.doc.replace_geometry(id, Geometry::Surface(s));
                    }
                }
                Step::Done
            }
            Input::Option(_) => self.start(ctx),
            _ => Step::Cancelled,
        }
    }
}

pub struct ExtractMeshPartCmd;

impl Command for ExtractMeshPartCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select mesh part to extract", ObjFilter::Mesh).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Pick(id, p) = input else { return Step::Cancelled };
        let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        let parts = super::mesh_cmds::components(&m);
        if parts.len() < 2 {
            ctx.log("The mesh has a single connected part.");
            return Step::Done;
        }
        let dist = |m: &tri_geom::Mesh| m.vertices.iter().map(|v| (*v - p).length()).fold(f64::INFINITY, f64::min);
        let (pi, _) = parts.iter().enumerate().min_by(|a, b| dist(a.1).partial_cmp(&dist(b.1)).unwrap()).unwrap();
        let attrs = ctx.doc.get(id).map(|o| o.attributes.clone()).unwrap_or_default();
        ctx.doc.begin_undo("ExtractMeshPart");
        let mut rest = tri_geom::Mesh::new();
        for (i, part) in parts.iter().enumerate() {
            if i != pi {
                rest.append(part);
            }
        }
        ctx.doc.replace_geometry(id, Geometry::Mesh(rest));
        ctx.doc.clear_selection();
        let nid = ctx.doc.add_with(Geometry::Mesh(parts[pi].clone()), attrs);
        ctx.doc.select(nid, true);
        Step::Done
    }
}

// ---- Maelstrom, Distribute ----------------------------------------------------------------------------

fn deform(g: &mut Geometry, f: &dyn Fn(DVec3) -> DVec3) {
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

#[derive(Default)]
pub struct MaelstromCmd {
    ids: Vec<ObjectId>,
    center: Option<DVec3>,
    radii: Vec<f64>,
}

impl Command for MaelstromCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to deform", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.center, input) {
            (_, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point("Center of maelstrom").step()
            }
            (None, Input::Point(p)) => {
                self.center = Some(p);
                Prompt::point_or_number("First radius").default_num(1.0).step()
            }
            (Some(c), inp @ (Input::Point(_) | Input::Number(_))) => {
                let r = match inp { Input::Number(v) => v.abs(), Input::Point(p) => (p - c).length(), _ => 1.0 };
                self.radii.push(r);
                if self.radii.len() < 2 {
                    return Prompt::point_or_number("Second radius").default_num(r * 2.0).step();
                }
                Prompt::number("Rotation angle in degrees").default_num(ctx.mem_num("Maelstrom.Angle", 90.0)).step()
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.center, cursor) {
            (Some(c), Some(p)) if self.radii.len() < 2 => NurbsCurve::circle(&Plane::from_normal(c, DVec3::Z), (p - c).length()).map(|cv| vec![Preview::Curve(cv)]).unwrap_or_default(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center
    }
}

impl MaelstromCmd {
    pub fn apply(&mut self, angle: f64, ctx: &mut CmdCtx) -> Step {
        let Some(c) = self.center else { return Step::Cancelled };
        let axis = ctx.cplane.z_axis;
        let (r1, r2) = (self.radii[0], self.radii[1]);
        ctx.mem_set("Maelstrom.Angle", fmt_num(angle));
        ctx.doc.begin_undo("Maelstrom");
        for id in &self.ids {
            let Some(mut g) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { continue };
            deform(&mut g, &|p| {
                let radial = (p - c) - axis * (p - c).dot(axis);
                let r = radial.length();
                let t = if (r2 - r1).abs() < 1e-12 { 1.0 } else { ((r - r1) / (r2 - r1)).clamp(0.0, 1.0) };
                let q = DQuat::from_axis_angle(axis, angle.to_radians() * t);
                c + axis * (p - c).dot(axis) + q * radial
            });
            ctx.doc.replace_geometry(*id, g);
        }
        Step::Done
    }
}

pub struct MaelstromWrap(pub MaelstromCmd);

impl Command for MaelstromWrap {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.0.start(ctx)
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Number(a) if self.0.radii.len() == 2 => self.0.apply(a, ctx),
            Input::Empty if self.0.radii.len() == 2 => {
                let a = ctx.mem_num("Maelstrom.Angle", 90.0);
                self.0.apply(a, ctx)
            }
            other => self.0.input(other, ctx),
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        self.0.preview(cursor, doc)
    }
    fn base_point(&self) -> Option<DVec3> {
        self.0.base_point()
    }
}

pub struct DistributeCmd;

impl Command for DistributeCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to distribute", ObjFilter::Any, 3).opts([optv("Direction", "X"), optv("Mode", "Centers")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Direction" => {
                let cur = ctx.memory.get("Distribute.Dir").cloned().unwrap_or_else(|| "X".into());
                let next = match cur.as_str() { "X" => "Y", "Y" => "Z", _ => "X" };
                ctx.mem_set("Distribute.Dir", next);
                Prompt::objects("Select objects to distribute", ObjFilter::Any, 3).opts([optv("Direction", next)]).step()
            }
            Input::Option(_) => self.start(ctx),
            Input::Objects(ids) => {
                let dir = match ctx.memory.get("Distribute.Dir").map(|s| s.as_str()) { Some("Y") => ctx.cplane.y_axis, Some("Z") => ctx.cplane.z_axis, _ => ctx.cplane.x_axis };
                let mut items: Vec<(ObjectId, f64)> = ids.iter().filter_map(|id| ctx.doc.get(*id).map(|o| (*id, ctx.doc.object_bbox(o).center().dot(dir)))).collect();
                if items.len() < 3 {
                    return Step::Cancelled;
                }
                items.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
                let (lo, hi) = (items[0].1, items[items.len() - 1].1);
                let step = (hi - lo) / (items.len() - 1) as f64;
                ctx.doc.begin_undo("Distribute");
                for (i, (id, cur)) in items.iter().enumerate() {
                    let target = lo + step * i as f64;
                    ctx.doc.transform_objects(&[*id], &glam::DAffine3::from_translation(dir * (target - cur)));
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- layer utilities ------------------------------------------------------------------------------------

pub struct LayerUtilCmd(pub &'static str);

impl Command for LayerUtilCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.0 {
            "ChangeToCurrentLayer" => {
                let sel = ctx.doc.selected_ids();
                if sel.is_empty() {
                    return Prompt::objects("Select objects to change to the current layer", ObjFilter::Any, 1).step();
                }
                self.input(Input::Objects(sel), ctx)
            }
            "MatchLayer" => Prompt::objects("Select objects to change layer", ObjFilter::Any, 1).step(),
            "DupLayer" => {
                let name = ctx.doc.layers()[ctx.doc.current_layer()].name.clone();
                Prompt::text("Name of layer to duplicate").default(name).step()
            }
            _ => Prompt::pick("Select object on layer to turn off", ObjFilter::Any).step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.0, input) {
            ("ChangeToCurrentLayer", Input::Objects(ids)) => {
                let cur = ctx.doc.current_layer();
                ctx.doc.begin_undo("ChangeToCurrentLayer");
                for id in ids {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.layer = cur;
                    }
                }
                Step::Done
            }
            ("MatchLayer", Input::Objects(ids)) => {
                ctx.mem_set("MatchLayer.Ids", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
                ctx.doc.clear_selection();
                Prompt::pick("Select object on the target layer", ObjFilter::Any).step()
            }
            ("MatchLayer", Input::Pick(id, _)) => {
                let layer = ctx.doc.get(id).map(|o| o.attributes.layer).unwrap_or(0);
                let ids: Vec<ObjectId> = ctx.memory.get("MatchLayer.Ids").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                ctx.doc.begin_undo("MatchLayer");
                for oid in ids {
                    if let Some(o) = ctx.doc.get_mut(oid) {
                        o.attributes.layer = layer;
                    }
                }
                Step::Done
            }
            ("DupLayer", inp @ (Input::Text(_) | Input::Empty)) => {
                let name = match inp { Input::Text(t) if !t.trim().is_empty() => t.trim().to_string(), _ => ctx.doc.layers()[ctx.doc.current_layer()].name.clone() };
                let Some(src) = ctx.doc.find_layer(&name) else {
                    ctx.log(format!("No layer named \"{name}\"."));
                    return Step::Cancelled;
                };
                let layer = ctx.doc.layers()[src].clone();
                let mut n = 1;
                let mut new_name = format!("{name} {n:02}");
                while ctx.doc.find_layer(&new_name).is_some() {
                    n += 1;
                    new_name = format!("{name} {n:02}");
                }
                ctx.doc.begin_undo("DupLayer");
                let idx = ctx.doc.add_layer(new_name.clone());
                if let Some(l) = ctx.doc.layer_mut(idx) {
                    l.color = layer.color;
                    l.visible = layer.visible;
                    l.locked = layer.locked;
                }
                ctx.log(format!("Layer \"{new_name}\" created."));
                Step::Done
            }
            ("OneLayerOff", Input::Pick(id, _)) => {
                let layer = ctx.doc.get(id).map(|o| o.attributes.layer).unwrap_or(0);
                if layer == ctx.doc.current_layer() {
                    ctx.log("The current layer cannot be turned off.");
                    return Step::Cancelled;
                }
                ctx.doc.begin_undo("OneLayerOff");
                if let Some(l) = ctx.doc.layer_mut(layer) {
                    l.visible = false;
                }
                ctx.doc.clear_selection();
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- MergeAllCoplanarFaces ---------------------------------------------------------------------------------

/// Merge coplanar adjacent planar faces of a polysurface using planar region unions.
pub fn merge_coplanar(b: &Brep) -> Brep {
    let tol = b.bounding_box().diagonal().max(1e-6) * 1e-6;
    let infos: Vec<Option<(Plane, Vec<Vec<DVec3>>)>> = b.faces.iter().map(super::solid_edit::planar_loops).collect();
    let mut used = vec![false; b.faces.len()];
    let mut out = Vec::new();
    for i in 0..b.faces.len() {
        if used[i] {
            continue;
        }
        let Some((plane, _)) = &infos[i] else {
            out.push(b.faces[i].clone());
            used[i] = true;
            continue;
        };
        // gather coplanar faces (same normal and offset)
        let mut group = vec![i];
        for j in i + 1..b.faces.len() {
            if used[j] {
                continue;
            }
            if let Some((pj, _)) = &infos[j] {
                if pj.z_axis.dot(plane.z_axis) > 1.0 - 1e-9 && (pj.origin - plane.origin).dot(plane.z_axis).abs() < tol * 100.0 {
                    group.push(j);
                }
            }
        }
        if group.len() == 1 {
            out.push(b.faces[i].clone());
            used[i] = true;
            continue;
        }
        let mut curves = Vec::new();
        for &g in &group {
            if let Some((_, loops)) = &infos[g] {
                let mut l = loops[0].clone();
                l.push(loops[0][0]);
                if let Ok(c) = NurbsCurve::polyline(l) {
                    curves.push(c);
                }
            }
        }
        let regions = region_boolean(&curves, &[], tri_geom::BooleanOp::Union);
        if regions.is_empty() {
            for &g in &group {
                out.push(b.faces[g].clone());
                used[g] = true;
            }
            continue;
        }
        // each region outline becomes a face; the largest loop is the outer, others are holes when inside it
        let mut loops3: Vec<(f64, Vec<DVec3>)> = regions
            .iter()
            .map(|c| {
                let mut pts = c.control_points.clone();
                if pts.len() > 1 && (pts[0] - pts[pts.len() - 1]).length() < tol {
                    pts.pop();
                }
                (Document::area_of(&Geometry::Curve(c.clone())).unwrap_or(0.0), pts)
            })
            .collect();
        loops3.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let outer = loops3.remove(0).1;
        let holes: Vec<Vec<DVec3>> = loops3.into_iter().map(|(_, l)| l).collect();
        let mut all = vec![outer];
        all.extend(holes);
        match Face::planar(plane, all) {
            Ok(f) => out.push(f),
            Err(_) => {
                for &g in &group {
                    out.push(b.faces[g].clone());
                }
            }
        }
        for &g in &group {
            used[g] = true;
        }
    }
    Brep::new(out)
}

pub struct MergeCoplanarCmd;

impl Command for MergeCoplanarCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select polysurfaces to merge coplanar faces", ObjFilter::Polysurface, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("MergeAllCoplanarFaces");
        let mut merged = 0;
        for id in ids {
            let Some(Geometry::Brep(b)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let nb = merge_coplanar(&b);
            if nb.faces.len() < b.faces.len() {
                merged += b.faces.len() - nb.faces.len();
                ctx.doc.replace_geometry(id, Geometry::Brep(nb));
            }
        }
        ctx.log(format!("{merged} faces merged."));
        Step::Done
    }
}

// ---- FilletSrf between planar surfaces ---------------------------------------------------------------------

#[derive(Default)]
pub struct FilletSrfCmd {
    picks: Vec<(ObjectId, DVec3)>,
    radius: f64,
}

impl Command for FilletSrfCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.radius = ctx.mem_num("FilletSrf.Radius", 1.0);
        Prompt::pick("Select first surface to fillet", ObjFilter::Surface).opts([optv("Radius", fmt_num(self.radius)), optv("Extend", "Yes"), optv("Trim", "Yes")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Radius" => Prompt::number("Fillet radius").default_num(self.radius).step(),
            Input::Option(_) => Prompt::pick("Select first surface to fillet", ObjFilter::Surface).step(),
            Input::Number(r) => {
                self.radius = r.abs().max(1e-6);
                ctx.mem_set("FilletSrf.Radius", fmt_num(self.radius));
                Prompt::pick(if self.picks.is_empty() { "Select first surface to fillet" } else { "Select second surface to fillet" }, ObjFilter::Surface).opts([optv("Radius", fmt_num(self.radius))]).step()
            }
            Input::Pick(id, p) => {
                self.picks.push((id, p));
                if self.picks.len() < 2 {
                    return Prompt::pick("Select second surface to fillet", ObjFilter::Surface).opts([optv("Radius", fmt_num(self.radius))]).step();
                }
                let (ida, pa) = self.picks[0];
                let (idb, pb) = self.picks[1];
                let (Some(fa), Some(fb)) = (faces_of(ctx.doc, ida), faces_of(ctx.doc, idb)) else { return Step::Cancelled };
                let (Some((plane_a, loops_a)), Some((plane_b, loops_b))) = (super::solid_edit::planar_loops(&fa[face_under(&fa, pa)]), super::solid_edit::planar_loops(&fb[face_under(&fb, pb)])) else {
                    ctx.log("FilletSrf works between planar surfaces in triceratops.");
                    return Step::Cancelled;
                };
                let (na, nb) = (plane_a.z_axis, plane_b.z_axis);
                let e = na.cross(nb);
                if e.length() < 1e-9 {
                    ctx.log("The surfaces are parallel.");
                    return Step::Cancelled;
                }
                let e = e.normalize();
                // a point on the intersection line: solve for the point closest to both plane origins
                let m = glam::DMat3::from_cols(na, nb, e).transpose();
                let rhs = DVec3::new(na.dot(plane_a.origin), nb.dot(plane_b.origin), e.dot(plane_a.origin));
                let line_pt = m.inverse() * rhs;
                let r = self.radius;
                // sides: fillet centre lies on the side of each plane where the other pick point is
                let sa = (pb - plane_a.origin).dot(na).signum();
                let sb = (pa - plane_b.origin).dot(nb).signum();
                let m2 = glam::DMat3::from_cols(na, nb, e).transpose();
                let center = m2.inverse() * DVec3::new(na.dot(plane_a.origin) + sa * r, nb.dot(plane_b.origin) + sb * r, e.dot(line_pt));
                let ta = center - na * (sa * r);
                let tb = center - nb * (sb * r);
                // extent along the line: overlap of both faces' projections
                let extent = |loops: &Vec<Vec<DVec3>>| {
                    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
                    for l in loops {
                        for p in l {
                            let t = (*p - line_pt).dot(e);
                            lo = lo.min(t);
                            hi = hi.max(t);
                        }
                    }
                    (lo, hi)
                };
                let (la, ha) = extent(&loops_a);
                let (lb, hb) = extent(&loops_b);
                let (lo, hi) = (la.max(lb), ha.min(hb));
                if hi - lo < 1e-9 {
                    ctx.log("The surfaces do not overlap along the fillet.");
                    return Step::Cancelled;
                }
                let x = (ta - center).normalize();
                let y_raw = (tb - center).normalize();
                let ang = x.dot(y_raw).clamp(-1.0, 1.0).acos();
                let axis = x.cross(y_raw).normalize_or(e);
                let arc_plane = Plane { origin: center + e * lo, x_axis: x, y_axis: axis.cross(x).normalize(), z_axis: axis };
                let Ok(arc) = NurbsCurve::arc(&arc_plane, r, 0.0, ang) else { return Step::Cancelled };
                let Ok(fillet) = NurbsSurface::extrude(&arc, e * (hi - lo)) else { return Step::Cancelled };
                ctx.doc.begin_undo("FilletSrf");
                // trim the surfaces back to the tangent lines (remove the side toward the other surface)
                let big = (hi - lo).max(1.0) * 10.0;
                for (id, pick, t) in [(ida, pa, ta), (idb, pb, tb)] {
                    let cutter = NurbsCurve::line(t - e * big, t + e * big).unwrap();
                    // remove the strip between the intersection line and the tangent line
                    let perp = |p: DVec3| { let v = p - line_pt; v - e * v.dot(e) };
                    let d = perp(pick).normalize_or(perp(t).normalize_or(DVec3::X));
                    let dist = perp(t).length();
                    let remove_pt = t - d * (dist * 0.5) + e * ((pick - line_pt).dot(e) - (t - line_pt).dot(e));
                    if !trim_face(ctx, id, remove_pt, &[cutter]) {
                        ctx.log("FilletSrf: a surface could not be trimmed; the fillet surface was still created.");
                    }
                }
                ctx.doc.clear_selection();
                let nid = ctx.doc.add(Geometry::Surface(fillet));
                ctx.doc.select(nid, true);
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct NoopCmd(pub &'static str);

impl Command for NoopCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.log(self.0);
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

#[allow(dead_code)]
fn unused(_: &AppRequest) {}
