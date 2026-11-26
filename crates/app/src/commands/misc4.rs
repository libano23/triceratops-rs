//! Fourth batch: planar region booleans (PlanarUnion/Difference/Intersection, CurveBoolean),
//! RoundHole/MakeHole, Shell (mesh based), Bend, Stretch, Flow, Dot, NamedCPlane, SelChain,
//! InsertControlPoint/RemoveControlPoint, Weight, Conic.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use super::mesh_bool;
use tri_geom::{curve_plane, BooleanOp, Brep, Face, Mesh, NurbsCurve, NurbsSurface, Plane};

use super::annotate::text_curves;
use super::create::{planar_cap, planar_faces};
use super::surface2::rail_frames;
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

/// Watertight prism mesh over a closed planar polygon: caps share the polygon vertices with the sides.
pub fn prism_mesh(polygon: &[DVec3], plane: &Plane, half: f64) -> Option<Mesh> {
    let mut poly: Vec<DVec3> = polygon.to_vec();
    if poly.len() > 1 && (poly[0] - poly[poly.len() - 1]).length() < 1e-9 {
        poly.pop();
    }
    if poly.len() < 3 {
        return None;
    }
    let cap = Face::planar(plane, vec![poly.clone()]).ok()?;
    let cap_mesh = cap.tessellate(1, 1);
    let n = poly.len();
    let mut m = Mesh::new();
    let dn = plane.z_axis * half;
    // bottom cap (flipped) and top cap
    let mut bottom = cap_mesh.clone();
    bottom.transform(&DAffine3::from_translation(-dn));
    bottom.flip();
    let mut top = cap_mesh;
    top.transform(&DAffine3::from_translation(dn));
    m.append(&bottom);
    m.append(&top);
    // sides
    let base = m.vertices.len() as u32;
    for p in &poly {
        m.vertices.push(*p - dn);
    }
    for p in &poly {
        m.vertices.push(*p + dn);
    }
    let ccw = {
        let (u, _) = (plane.x_axis, plane.y_axis);
        let _ = u;
        let area: f64 = (0..n).map(|i| {
            let a = poly[i] - plane.origin;
            let b = poly[(i + 1) % n] - plane.origin;
            a.cross(b).dot(plane.z_axis)
        }).sum();
        area > 0.0
    };
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b, c, d) = (base + i as u32, base + j as u32, base + (n + j) as u32, base + (n + i) as u32);
        if ccw {
            m.indices.extend_from_slice(&[a, b, c, a, c, d]);
        } else {
            m.indices.extend_from_slice(&[a, c, b, a, d, c]);
        }
    }
    let welded = super::mesh_cmds::weld(&m, 1e-9 * (1.0 + m.bounding_box().diagonal()));
    Some(welded)
}

/// Thin prism solid mesh built on a planar closed curve (used for region booleans).
fn region_prism(c: &NurbsCurve, plane: &Plane, half: f64) -> Option<Mesh> {
    let tol = c.bounding_box().diagonal().max(1e-6) * 2e-4;
    let mut polygon = c.tessellate_adaptive(tol);
    if polygon.len() > 64 && c.degree > 1 {
        // keep Boolean cutters light: at most 64 samples for smooth curves
        let (t0, t1) = c.domain();
        polygon = (0..64).map(|i| c.point_at(t0 + (t1 - t0) * i as f64 / 64.0)).collect();
        polygon.push(polygon[0]);
    }
    prism_mesh(&polygon, plane, half)
}

/// Boolean of planar regions: returns the closed outline curves of the result.
pub fn region_boolean(a: &[NurbsCurve], b: &[NurbsCurve], op: BooleanOp) -> Vec<NurbsCurve> {
    let Some(first) = a.first() else { return vec![] };
    let Some(plane) = curve_plane(first) else { return vec![] };
    let bb = a.iter().chain(b.iter()).fold(tri_geom::BoundingBox::EMPTY, |bb, c| bb.union(&c.bounding_box()));
    let half = bb.diagonal().max(1e-3) * 0.05;
    let union = |cs: &[NurbsCurve]| -> Option<Mesh> {
        let mut it = cs.iter().filter(|c| c.is_closed()).filter_map(|c| region_prism(c, &plane, half));
        let mut m = it.next()?;
        for n in it {
            m = mesh_bool(&m, &n, BooleanOp::Union).ok()?;
        }
        Some(m)
    };
    let Some(ma) = union(a) else { return vec![] };
    let result = match (op, union(b)) {
        (BooleanOp::Union, None) => ma,
        (_, None) => return vec![],
        (o, Some(mb)) => match mesh_bool(&ma, &mb, o) {
            Ok(m) => m,
            Err(_) => return vec![],
        },
    };
    let tol = bb.diagonal().max(1e-3) * 1e-6;
    result
        .section(&plane, tol)
        .into_iter()
        .filter(|p| p.len() >= 3)
        .filter_map(|mut p| {
            // close and simplify collinear runs
            if let (Some(f), Some(l)) = (p.first().copied(), p.last().copied()) {
                if (f - l).length() > tol * 10.0 {
                    p.push(f);
                }
            }
            let simplified = simplify_collinear(&p, bb.diagonal() * 1e-6);
            NurbsCurve::polyline(simplified).ok()
        })
        .collect()
}

fn simplify_collinear(p: &[DVec3], tol: f64) -> Vec<DVec3> {
    let mut out: Vec<DVec3> = Vec::new();
    for &q in p {
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            if (b - a).cross(q - b).length() <= tol * (b - a).length().max(1e-12) {
                out.pop();
            }
        }
        if out.last().is_none_or(|l| (*l - q).length() > tol) {
            out.push(q);
        }
    }
    out
}

pub struct PlanarBoolCmd {
    op: BooleanOp,
    curve_boolean: bool,
    first: Vec<ObjectId>,
}

impl PlanarBoolCmd {
    pub fn new(op: BooleanOp, curve_boolean: bool) -> Self {
        Self { op, curve_boolean, first: Vec::new() }
    }
    fn label(&self) -> &'static str {
        if self.curve_boolean {
            return "CurveBoolean";
        }
        match self.op {
            BooleanOp::Union => "PlanarUnion",
            BooleanOp::Difference => "PlanarDifference",
            BooleanOp::Intersection => "PlanarIntersection",
        }
    }
}

impl Command for PlanarBoolCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        if self.curve_boolean {
            return Prompt::objects("Select curves for Boolean", ObjFilter::Curve, 1).opts([optv("DeleteInput", "No"), optv("CombineRegions", "Yes"), optv("AllRegions", "Union")]).step();
        }
        let msg = match self.op {
            BooleanOp::Union => "Select closed planar curves to union",
            BooleanOp::Difference => "Select closed planar curves to subtract from",
            BooleanOp::Intersection => "Select first set of closed planar curves",
        };
        Prompt::objects(msg, ObjFilter::Curve, 1).opts([optv("DeleteInput", "Yes")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.first.is_empty() && (self.op == BooleanOp::Union || self.curve_boolean) => {
                let curves: Vec<NurbsCurve> = ids.iter().filter_map(|id| curve_of(ctx.doc, *id)).collect();
                let result = region_boolean(&curves, &[], BooleanOp::Union);
                if result.is_empty() {
                    ctx.log("No closed planar regions found.");
                    return Step::Cancelled;
                }
                ctx.doc.begin_undo(self.label());
                if !self.curve_boolean {
                    for id in &ids {
                        ctx.doc.remove(*id);
                    }
                }
                add_all(ctx, self.label(), result.into_iter().map(Geometry::Curve).collect())
            }
            Input::Objects(ids) if self.first.is_empty() => {
                self.first = ids;
                ctx.doc.clear_selection();
                Prompt::objects(if self.op == BooleanOp::Difference { "Select closed planar curves to subtract with" } else { "Select second set of closed planar curves" }, ObjFilter::Curve, 1).step()
            }
            Input::Objects(second) => {
                let a: Vec<NurbsCurve> = self.first.iter().filter_map(|id| curve_of(ctx.doc, *id)).collect();
                let b: Vec<NurbsCurve> = second.iter().filter_map(|id| curve_of(ctx.doc, *id)).collect();
                let result = region_boolean(&a, &b, self.op);
                if result.is_empty() {
                    ctx.log("Boolean produced no regions.");
                    return Step::Cancelled;
                }
                ctx.doc.begin_undo(self.label());
                for id in self.first.iter().chain(second.iter()) {
                    ctx.doc.remove(*id);
                }
                add_all(ctx, self.label(), result.into_iter().map(Geometry::Curve).collect())
            }
            Input::Option(_) => self.start(ctx),
            _ => Step::Cancelled,
        }
    }
}

// ---- RoundHole / MakeHole / Shell (mesh based) ----------------------------------------------------------

/// Mesh of any geometry used by the Booleans (fallback path).
pub fn geometry_mesh(g: &Geometry) -> Option<Mesh> {
    match g {
        Geometry::Surface(s) => Some(s.tessellate(12, 12)),
        Geometry::Brep(b) => Some(b.tessellate(12, 12)),
        Geometry::Mesh(m) => Some(m.clone()),
        _ => None,
    }
}

/// Closed polysurface view of a surface or polysurface.
pub fn geometry_brep(g: &Geometry) -> Option<Brep> {
    match g {
        Geometry::Brep(b) => Some(b.clone()),
        Geometry::Surface(s) => Some(Brep::from_surface(s.clone())),
        _ => None,
    }
}

/// Whether a Boolean result still contains something.
pub fn geometry_nonempty(g: &Geometry) -> bool {
    match g {
        Geometry::Brep(b) => !b.faces.is_empty(),
        Geometry::Mesh(m) => m.triangle_count() > 0,
        _ => true,
    }
}

/// Boolean between two solids: a NURBS polysurface when both operands are surfaces or
/// polysurfaces and the face outlines can be rebuilt, otherwise a mesh (meshes stay meshes).
pub fn boolean_geometry(a: &Geometry, b: &Geometry, op: BooleanOp) -> Option<Geometry> {
    boolean_geometry_checked(a, b, op).ok()
}

/// `boolean_geometry` saying why there is no result ("cancelled", "Boolean too complex …").
pub fn boolean_geometry_checked(a: &Geometry, b: &Geometry, op: BooleanOp) -> Result<Geometry, String> {
    if let (Some(ba), Some(bb)) = (geometry_brep(a), geometry_brep(b)) {
        // surfaces and polysurfaces always give a polysurface (no mesh fallback)
        let r = tri_geom::brep_boolean::brep_boolean_verbose(&ba, &bb, op, tri_geom::brep_boolean::DEFAULT_TOLERANCE)?;
        return if r.closed { Ok(Geometry::Brep(r)) } else { Err("the result is not a closed solid".into()) };
    }
    let (Some(ma), Some(mb)) = (geometry_mesh(a), geometry_mesh(b)) else { return Err("these objects cannot be combined".into()) };
    let m = mesh_bool(&ma, &mb, op).map_err(|e| e.to_string())?;
    if m.triangle_count() == 0 {
        return Err("the result is empty".into());
    }
    Ok(Geometry::Mesh(m))
}

/// Extrusion solid of a closed planar curve, `half` on each side of its plane, as a polysurface
/// (used as a cutter for holes, WireCut, Boss and Rib).
pub fn prism_brep(c: &NurbsCurve, plane: &Plane, half: f64) -> Option<Brep> {
    if !c.is_closed() {
        return None;
    }
    let dn = plane.z_axis * half;
    let bottom = c.transformed(&DAffine3::from_translation(-dn));
    let top = c.transformed(&DAffine3::from_translation(dn));
    let side = NurbsSurface::extrude(&bottom, dn * 2.0).ok()?;
    let cap0 = planar_cap(&bottom)?;
    let cap1 = planar_cap(&top)?;
    let mut b = Brep::new(vec![Face::untrimmed(side), cap0, cap1]);
    b.orient_outward();
    b.closed = true;
    Some(b)
}

/// Cutter for a planar closed curve: polysurface when possible, mesh otherwise.
pub fn prism_tool(c: &NurbsCurve, plane: &Plane, half: f64) -> Option<Geometry> {
    if let Some(b) = prism_brep(c, plane, half) {
        return Some(Geometry::Brep(b));
    }
    region_prism(c, plane, half).map(Geometry::Mesh)
}

pub fn solid_mesh(doc: &Document, id: ObjectId) -> Option<Mesh> {
    match doc.get(id).map(|o| &o.geometry) {
        Some(Geometry::Surface(s)) => Some(s.tessellate(12, 12)),
        Some(Geometry::Brep(b)) => Some(b.tessellate(12, 12)),
        Some(Geometry::Mesh(m)) => Some(m.clone()),
        _ => None,
    }
}

#[derive(Default)]
pub struct RoundHoleCmd {
    target: Option<ObjectId>,
    center: Option<DVec3>,
    normal: DVec3,
    radius: Option<f64>,
}

impl Command for RoundHoleCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select surface or polysurface for hole", ObjFilter::Surface).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.target, self.center, self.radius, input) {
            (None, _, _, Input::Pick(id, _)) => {
                self.target = Some(id);
                Prompt::point("Center point of hole").opts([optv("Radius", fmt_num(ctx.mem_num("RoundHole.Radius", 1.0))), optv("Depth", "Through"), optv("Direction", "SurfaceNormal")]).step()
            }
            (Some(id), None, _, Input::Point(p)) => {
                let faces = super::surface_edit::faces_of(ctx.doc, id).unwrap_or_default();
                self.normal = if faces.is_empty() {
                    ctx.cplane.z_axis
                } else {
                    let fi = super::surface_edit::face_under(&faces, p);
                    let (u, v) = faces[fi].surface.closest_uv(p);
                    faces[fi].surface.normal_at(u, v)
                };
                self.center = Some(p);
                Prompt::point_or_number("Radius").default_num(ctx.mem_num("RoundHole.Radius", 1.0)).step()
            }
            (Some(id), Some(c), None, inp @ (Input::Point(_) | Input::Number(_))) => {
                let r = match inp { Input::Number(v) => v.abs(), Input::Point(p) => (p - c).length(), _ => 1.0 };
                ctx.mem_set("RoundHole.Radius", fmt_num(r));
                self.radius = Some(r);
                let Some(m) = solid_mesh(ctx.doc, id) else { return Step::Cancelled };
                let diag = m.bounding_box().diagonal().max(1e-3);
                let plane = Plane::from_normal(c, self.normal);
                let polygon: Vec<DVec3> = (0..48).map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / 48.0;
                    plane.point_at(r.max(1e-6) * a.cos(), r.max(1e-6) * a.sin())
                }).collect();
                let Some(_cutter) = prism_mesh(&polygon, &plane, diag) else { return Step::Cancelled };
                let Ok(circle) = NurbsCurve::circle(&plane, r.max(1e-6)) else { return Step::Cancelled };
                ctx.doc.begin_undo("RoundHole");
                let cid = add_hole_curve(ctx, circle);
                register_hole(ctx, id, vec![cid]);
                recut_holes(ctx, id);
                ctx.log("The hole curve stays as a locked definition curve (MoveHole, CopyHole, ... re-cut it).");
                Step::Done
            }
            (_, _, _, Input::Option(_)) => Prompt::point("Center point of hole").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.center, cursor) {
            (Some(c), Some(p)) if self.radius.is_none() => NurbsCurve::circle(&Plane::from_normal(c, self.normal), (p - c).length()).map(|cv| vec![Preview::Curve(cv)]).unwrap_or_default(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center
    }
}

#[derive(Default)]
pub struct MakeHoleCmd {
    curves: Vec<ObjectId>,
}

impl Command for MakeHoleCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select closed planar curves for hole", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.curves.is_empty() => {
                self.curves = ids;
                ctx.doc.clear_selection();
                Prompt::pick("Select surface or polysurface to cut", ObjFilter::Surface).step()
            }
            Input::Pick(id, _) => {
                if solid_mesh(ctx.doc, id).is_none() {
                    return Step::Cancelled;
                }
                ctx.doc.begin_undo("MakeHole");
                let curves = self.curves.clone();
                for cid in &curves {
                    if let Some(o) = ctx.doc.get_mut(*cid) {
                        o.attributes.locked = true;
                        o.attributes.name = Some("Hole".into());
                    }
                }
                register_hole(ctx, id, curves);
                recut_holes(ctx, id);
                ctx.log("The hole curves stay as locked definition curves (MoveHole, CopyHole, ... re-cut them).");
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct ShellCmd {
    picks: Vec<(ObjectId, DVec3)>,
}

impl Command for ShellCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select faces to remove. Press Enter when done", ObjFilter::Polysurface).opts([optv("Thickness", "1")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                self.picks.push((id, p));
                Prompt::pick("Select faces to remove. Press Enter when done", ObjFilter::Polysurface).step()
            }
            Input::Option(_) => Prompt::number("Thickness").default_num(ctx.mem_num("Shell.Thickness", 1.0)).step(),
            Input::Empty if self.picks.is_empty() => Step::Cancelled,
            Input::Empty => Prompt::number("Thickness").default_num(ctx.mem_num("Shell.Thickness", 1.0)).step(),
            Input::Number(t) => {
                ctx.mem_set("Shell.Thickness", fmt_num(t));
                if self.picks.is_empty() {
                    return Prompt::pick("Select faces to remove. Press Enter when done", ObjFilter::Polysurface).step();
                }
                let id = self.picks[0].0;
                let Some(outer) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
                let remove: Vec<DVec3> = self.picks.iter().filter(|(pid, _)| *pid == id).map(|(_, p)| *p).collect();
                ctx.work("Shell", move || shell_geometry(&outer, t, &remove), move |result, ctx| match result {
                    Ok(r) => {
                        ctx.doc.begin_undo("Shell");
                        ctx.doc.replace_geometry(id, r);
                        Step::Done
                    }
                    Err(e) => {
                        ctx.log(e);
                        Step::Cancelled
                    }
                })
            }
            _ => Step::Cancelled,
        }
    }
}

/// Shell: hollow a solid with planar faces to wall thickness `t`; faces under the points in
/// `remove` are opened. Errors explain why it could not be done.
pub fn shell_geometry(outer: &Geometry, t: f64, remove: &[DVec3]) -> Result<Geometry, String> {
    let faces: Vec<Face> = match outer { Geometry::Brep(b) => b.faces.clone(), Geometry::Surface(sf) => vec![Face::untrimmed(sf.clone())], _ => return Err("Shell needs a polysurface.".into()) };
    let t = t.abs().max(1e-6);
    // inner solid: every planar face moved inward, vertices rebuilt as the
    // intersections of the three offset planes meeting there
    let mut planar: Vec<(Plane, Vec<Vec<DVec3>>)> = Vec::with_capacity(faces.len());
    for f in &faces {
        match super::solid_edit::planar_loops(f) {
            Some(pl) => planar.push(pl),
            None => return Err("Shell: triceratops shells solids with planar faces only (use OffsetSrf for curved faces).".into()),
        }
    }
    let diag = outer.bounding_box().diagonal().max(1e-6);
    let vtol = diag * 1e-6;
    let offset_plane = |i: usize| -> Plane { let p = &planar[i].0; Plane { origin: p.origin - p.z_axis * t, ..*p } };
    let mut inner_faces = Vec::with_capacity(faces.len());
    let mut lid_loops: Vec<Vec<DVec3>> = Vec::with_capacity(faces.len());
    for i in 0..planar.len() {
        let pi = offset_plane(i);
        let mut new_loops = Vec::new();
        for l in &planar[i].1 {
            let mut nl = Vec::with_capacity(l.len());
            for v in l {
                // other faces sharing this vertex
                let mut others: Vec<usize> = Vec::new();
                for j in 0..planar.len() {
                    if j == i {
                        continue;
                    }
                    if planar[j].1.iter().flatten().any(|w| (*w - *v).length() < vtol) {
                        others.push(j);
                    }
                }
                let mut solved = None;
                'pair: for a in 0..others.len() {
                    for b in a + 1..others.len() {
                        let (pa, pb) = (offset_plane(others[a]), offset_plane(others[b]));
                        let mat = glam::DMat3::from_cols(pi.z_axis, pa.z_axis, pb.z_axis).transpose();
                        if mat.determinant().abs() < 1e-9 {
                            continue;
                        }
                        let rhs = DVec3::new(pi.z_axis.dot(pi.origin), pa.z_axis.dot(pa.origin), pb.z_axis.dot(pb.origin));
                        let x = mat.inverse() * rhs;
                        if (x - *v).length() < diag {
                            solved = Some(x);
                            break 'pair;
                        }
                    }
                }
                nl.push(solved.unwrap_or(*v - planar[i].0.z_axis * t));
            }
            new_loops.push(nl);
        }
        lid_loops.push(new_loops[0].clone());
        match Face::planar(&pi, new_loops) {
            Ok(f) => inner_faces.push(f),
            Err(_) => {
                return Err("Shell: the thickness is too large for this solid.".into());
            }
        }
    }
    let mut inner = Brep::new(inner_faces);
    inner.orient_outward();
    if !inner.closed {
        return Err("Shell: the inner solid is not closed (thickness too large?).".into());
    }
    let Some(mut result) = boolean_geometry(outer, &Geometry::Brep(inner), BooleanOp::Difference) else {
        return Err("Shell failed.".into());
    };
    // open the picked faces: remove the lids between the outer and inner surfaces
    for p in remove {
        // the lid: the inner outline of the picked face, between the two surfaces
        let fi = super::surface_edit::face_under(&faces, *p);
        let plane = &planar[fi].0;
        let mut l = lid_loops[fi].clone();
        l.push(lid_loops[fi][0]);
        let Ok(poly) = NurbsCurve::polyline(l) else { continue };
        if let Some(tool) = prism_tool(&poly, plane, t * 1.01) {
            if let Some(r) = boolean_geometry(&result, &tool, BooleanOp::Difference) {
                result = r;
            }
        }
    }
    Ok(result)
}

// ---- deformations: Bend, Stretch, Flow ------------------------------------------------------------------------

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

pub struct DeformCmd {
    kind: &'static str,
    ids: Vec<ObjectId>,
    pts: Vec<DVec3>,
    rail: Option<ObjectId>,
}

impl DeformCmd {
    pub fn new(kind: &'static str) -> Self {
        Self { kind, ids: Vec::new(), pts: Vec::new(), rail: None }
    }
}

impl Command for DeformCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects(match self.kind { "Bend" => "Select objects to bend", "Stretch" => "Select objects to stretch", _ => "Select objects to flow along a curve" }, ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.kind, input) {
            (_, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point(match self.kind { "Bend" => "Start of spine", "Stretch" => "Start of stretch axis", _ => "Start of base line" }).step()
            }
            ("Bend", Input::Point(p)) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("End of spine").step(),
                    2 => Prompt::point("Point to bend through").opts([optv("Symmetric", "No"), optv("Straight", "No")]).step(),
                    _ => {
                        let (a, b, through) = (self.pts[0], self.pts[1], self.pts[2]);
                        let axis = (b - a).normalize_or(DVec3::X);
                        let len = (b - a).length().max(1e-9);
                        // bend plane: contains the spine and the through point
                        let side = (through - a) - axis * (through - a).dot(axis);
                        let side_dir = side.normalize_or(DVec3::Z);
                        let normal = axis.cross(side_dir).normalize_or(DVec3::Y);
                        // the spine end bends to the through point: arc through a with tangent axis, ending at `through`
                        let d = through - a;
                        let chord = d.length().max(1e-9);
                        let sin_half = d.cross(axis).length() / chord;
                        let angle = if sin_half < 1e-6 { 0.0 } else { 2.0 * (d.normalize().dot(axis)).clamp(-1.0, 1.0).acos() };
                        let total = if angle.abs() < 1e-9 { 0.0 } else { angle };
                        let radius = if total.abs() < 1e-9 { f64::INFINITY } else { len / total };
                        let center = a + side_dir * radius;
                        ctx.doc.begin_undo("Bend");
                        for id in &self.ids {
                            let Some(mut g) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { continue };
                            deform(&mut g, &|p| {
                                if !radius.is_finite() {
                                    return p;
                                }
                                let t = (p - a).dot(axis).clamp(0.0, len);
                                let rest = p - a - axis * (p - a).dot(axis);
                                let theta = t / radius;
                                let q = DQuat::from_axis_angle(normal, -theta);
                                let base = center + q * (a - center);
                                let extra = ((p - a).dot(axis) - t) * (q * axis);
                                base + q * rest + extra
                            });
                            ctx.doc.replace_geometry(*id, g);
                        }
                        Step::Done
                    }
                }
            }
            ("Stretch", Input::Point(p)) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("End of stretch axis").step(),
                    2 => Prompt::point_or_number("Point to stretch to").step(),
                    _ => unreachable!(),
                }
            }
            ("Stretch", Input::Number(v)) if self.pts.len() == 2 => {
                let (a, b) = (self.pts[0], self.pts[1]);
                let axis = (b - a).normalize_or(DVec3::X);
                self.input(Input::Point(b + axis * v), ctx)
            }
            ("Flow", Input::Point(p)) => {
                self.pts.push(p);
                if self.pts.len() < 2 {
                    return Prompt::point("End of base line").step();
                }
                Prompt::pick("Select target curve", ObjFilter::Curve).opts([optv("Copy", "No"), optv("Rigid", "No"), optv("Stretch", "No")]).step()
            }
            ("Flow", Input::Pick(id, _)) => {
                self.rail = Some(id);
                let Some(rail) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                let (a, b) = (self.pts[0], self.pts[1]);
                let axis = (b - a).normalize_or(DVec3::X);
                let len = (b - a).length().max(1e-9);
                let n = 64;
                let frames = rail_frames(&rail, n);
                let rail_len = rail.length().max(1e-9);
                let base_up = ctx.cplane.z_axis;
                let base_y = axis.cross(base_up).normalize_or(DVec3::Y);
                let base_z = axis.cross(base_y).normalize_or(DVec3::Z);
                let (t0, _) = rail.domain();
                let tan0 = rail.tangent_at(t0);
                let q0 = DQuat::from_rotation_arc(axis, tan0);
                ctx.doc.begin_undo("Flow");
                for oid in &self.ids {
                    let Some(mut g) = ctx.doc.get(*oid).map(|o| o.geometry.clone()) else { continue };
                    deform(&mut g, &|p| {
                        let t = ((p - a).dot(axis) / len).clamp(0.0, 1.0);
                        let y = (p - a).dot(base_y);
                        let z = (p - a).dot(base_z);
                        let fi = (t * n as f64).round() as usize;
                        let (origin, q) = frames[fi.min(n)];
                        let q = q * q0;
                        let _ = rail_len;
                        origin + q * (base_y * y + base_z * z)
                    });
                    ctx.doc.replace_geometry(*oid, g);
                }
                Step::Done
            }
            (_, Input::Option(_)) => Prompt::point("Point").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.first(), cursor) {
            (Some(a), Some(c)) if self.pts.len() == 1 => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

impl DeformCmd {
    #[allow(dead_code)]
    fn stretch_apply(&self, ctx: &mut CmdCtx, target: DVec3) -> Step {
        let (a, b) = (self.pts[0], self.pts[1]);
        let axis = (b - a).normalize_or(DVec3::X);
        let len = (b - a).length().max(1e-9);
        let new_len = (target - a).dot(axis);
        let scale = new_len / len;
        ctx.doc.begin_undo("Stretch");
        for id in &self.ids {
            let Some(mut g) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { continue };
            deform(&mut g, &|p| {
                let t = (p - a).dot(axis);
                let rest = p - a - axis * t;
                let nt = if t <= 0.0 { t } else if t >= len { t + (new_len - len) } else { t * scale };
                a + axis * nt + rest
            });
            ctx.doc.replace_geometry(*id, g);
        }
        Step::Done
    }
}

pub struct StretchCmd(pub DeformCmd);

impl Command for StretchCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.0.start(ctx)
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) if self.0.pts.len() == 2 => self.0.stretch_apply(ctx, p),
            Input::Number(v) if self.0.pts.len() == 2 => {
                let (a, b) = (self.0.pts[0], self.0.pts[1]);
                let axis = (b - a).normalize_or(DVec3::X);
                self.0.stretch_apply(ctx, b + axis * v)
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

// ---- Dot, NamedCPlane, SelChain, control points, Conic -----------------------------------------------------------

#[derive(Default)]
pub struct DotCmd {
    text: Option<String>,
}

impl Command for DotCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::text("Dot text").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.text, input) {
            (None, Input::Text(t)) => {
                self.text = Some(t);
                Prompt::point("Location of dot").step()
            }
            (Some(t), Input::Point(p)) => {
                let h = ctx.mem_num("Dim.TextHeight", 1.0);
                let mut curves = text_curves(t, &ctx.cplane, p + ctx.cplane.x_axis * (h * 0.3), h, 0.0);
                if let Ok(c) = NurbsCurve::circle(&Plane { origin: p, ..ctx.cplane }, h * 0.2) {
                    curves.push(c);
                }
                let n = ctx.memory.get("GroupCounter").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) + 1;
                ctx.mem_set("GroupCounter", n);
                ctx.doc.begin_undo("Dot");
                ctx.doc.clear_selection();
                for c in curves {
                    let mut a = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
                    a.groups.push(format!("Dot{n:02}"));
                    a.name = Some(t.clone());
                    let id = ctx.doc.add_with(Geometry::Curve(c), a);
                    ctx.doc.select(id, true);
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct NamedCPlaneCmd;

impl Command for NamedCPlaneCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let names: Vec<String> = ctx.memory.keys().filter_map(|k| k.strip_prefix("NamedCPlane.").map(|s| s.to_string())).collect();
        if !names.is_empty() {
            ctx.log(format!("Named construction planes: {}", names.join(", ")));
        }
        Prompt::options("Named CPlane").opts([opt("Save"), opt("Restore"), opt("Delete")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                ctx.mem_set("NamedCPlane.Mode", o.clone());
                Prompt::text(match o.as_str() { "Save" => "Name of construction plane to save", "Restore" => "Name of construction plane to restore", _ => "Name of construction plane to delete" }).step()
            }
            Input::Text(name) => {
                let key = format!("NamedCPlane.{}", name.trim());
                match ctx.memory.get("NamedCPlane.Mode").map(|s| s.as_str()) {
                    Some("Save") => {
                        let p = ctx.cplane;
                        ctx.mem_set(&key, format!("{} {} {} {} {} {} {} {} {}", p.origin.x, p.origin.y, p.origin.z, p.x_axis.x, p.x_axis.y, p.x_axis.z, p.y_axis.x, p.y_axis.y, p.y_axis.z));
                    }
                    Some("Restore") => {
                        let Some(v) = ctx.memory.get(&key).cloned() else {
                            ctx.log(format!("No construction plane named \"{}\".", name.trim()));
                            return Step::Cancelled;
                        };
                        let f: Vec<f64> = v.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                        if f.len() == 9 {
                            let plane = Plane::from_frame(DVec3::new(f[0], f[1], f[2]), DVec3::new(f[3], f[4], f[5]), DVec3::new(f[6], f[7], f[8]));
                            ctx.requests.push(AppRequest::SetCPlane(Some(plane)));
                        }
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

pub struct SelChainCmd;

impl Command for SelChainCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select curve to start chain", ObjFilter::Curve).opts([optv("ChainContinuity", "Position")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, _) => {
                let tol = ctx.doc.settings.absolute_tolerance.max(1e-6) * 10.0;
                let curves: Vec<(ObjectId, DVec3, DVec3)> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o)).filter_map(|o| o.geometry.as_curve().map(|c| (o.id, c.start_point(), c.end_point()))).collect();
                let mut chain = vec![id];
                let mut frontier = vec![id];
                while let Some(cur) = frontier.pop() {
                    let Some(&(_, s, e)) = curves.iter().find(|c| c.0 == cur) else { continue };
                    for (oid, os, oe) in &curves {
                        if chain.contains(oid) {
                            continue;
                        }
                        if [s, e].iter().any(|p| (*p - *os).length() < tol || (*p - *oe).length() < tol) {
                            chain.push(*oid);
                            frontier.push(*oid);
                        }
                    }
                }
                for cid in &chain {
                    ctx.doc.select(*cid, true);
                }
                ctx.log(format!("{} curves added to selection.", chain.len()));
                Step::Done
            }
            Input::Option(_) => Prompt::pick("Select curve to start chain", ObjFilter::Curve).step(),
            _ => Step::Cancelled,
        }
    }
}

pub struct ControlPointCmd(pub &'static str);

impl Command for ControlPointCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick(match self.0 { "Insert" => "Select curve for control point insertion", "Remove" => "Select curve near control point to remove", _ => "Select curve near control point to weight" }, ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                let Some(c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                match self.0 {
                    "Insert" => {
                        let t = c.closest_parameter(p);
                        let mut nc = c.clone();
                        nc.insert_knot(t, 1);
                        ctx.doc.begin_undo("InsertControlPoint");
                        ctx.doc.replace_geometry(id, Geometry::Curve(nc));
                        Step::Done
                    }
                    "Remove" => {
                        let n = c.control_points.len().saturating_sub(1).max(c.degree + 1);
                        match c.rebuild(n, c.degree) {
                            Ok(nc) => {
                                ctx.doc.begin_undo("RemoveControlPoint");
                                ctx.doc.replace_geometry(id, Geometry::Curve(nc));
                                Step::Done
                            }
                            Err(e) => {
                                ctx.log(format!("RemoveControlPoint failed: {e}"));
                                Step::Cancelled
                            }
                        }
                    }
                    _ => {
                        let (i, _) = c.control_points.iter().enumerate().min_by(|a, b| (*a.1 - p).length().partial_cmp(&(*b.1 - p).length()).unwrap()).unwrap();
                        ctx.mem_set("Weight.Id", id);
                        ctx.mem_set("Weight.Index", i);
                        Prompt::number("Weight").default_num(c.weights[i]).step()
                    }
                }
            }
            Input::Number(w) => {
                let id: ObjectId = ctx.memory.get("Weight.Id").and_then(|s| s.parse().ok()).unwrap_or(0);
                let i: usize = ctx.memory.get("Weight.Index").and_then(|s| s.parse().ok()).unwrap_or(0);
                let Some(mut c) = curve_of(ctx.doc, id) else { return Step::Cancelled };
                if i < c.weights.len() {
                    c.weights[i] = w.max(1e-6);
                    ctx.doc.begin_undo("Weight");
                    ctx.doc.replace_geometry(id, Geometry::Curve(c));
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

/// Conic section: start, end, apex and rho (0.5 = parabola) as a rational quadratic Bézier.
#[derive(Default)]
pub struct ConicCmd {
    pts: Vec<DVec3>,
}

impl Command for ConicCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Start of conic").opts([opt("Tangent"), opt("Perpendicular")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("End of conic").step(),
                    2 => Prompt::point("Apex").step(),
                    _ => Prompt::point_or_number("Point on curve or rho").default_num(ctx.mem_num("Conic.Rho", 0.5)).step(),
                }
            }
            Input::Number(rho) if self.pts.len() == 3 => {
                let rho = rho.clamp(0.01, 0.99);
                ctx.mem_set("Conic.Rho", fmt_num(rho));
                let w = rho / (1.0 - rho);
                let (a, b, apex) = (self.pts[0], self.pts[1], self.pts[2]);
                match NurbsCurve::new(2, vec![a, apex, b], vec![1.0, w, 1.0], vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0]) {
                    Ok(c) => add_all(ctx, "Conic", vec![Geometry::Curve(c)]),
                    Err(_) => Step::Cancelled,
                }
            }
            Input::Option(_) => Prompt::point("Start of conic").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.len(), cursor) {
            (1, Some(c)) => vec![Preview::Line(self.pts[0], c)],
            (2, Some(c)) => vec![Preview::Polyline(vec![self.pts[0], c, self.pts[1]])],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[allow(dead_code)]
fn unused() -> (Vec<Face>, Option<Face>, Option<NurbsSurface>, Option<Brep>) {
    (planar_faces(&[]), planar_cap(&NurbsCurve::line(DVec3::ZERO, DVec3::X).unwrap()), None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hole_cutter_and_box_are_closed() {
        let plane = Plane::from_normal(DVec3::new(5.0, 5.0, 10.0), DVec3::Z);
        let polygon: Vec<DVec3> = (0..48).map(|i| {
            let a = std::f64::consts::TAU * i as f64 / 48.0;
            plane.point_at(2.0 * a.cos(), 2.0 * a.sin())
        }).collect();
        let cutter = prism_mesh(&polygon, &plane, 17.0).unwrap();
        {
            let mut counts: std::collections::HashMap<(u32, u32), usize> = Default::default();
            for t in cutter.indices.chunks(3) {
                for k in 0..3 {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    *counts.entry((a.min(b), a.max(b))).or_default() += 1;
                }
            }
            let naked: Vec<_> = counts.iter().filter(|(_, c)| **c != 2).map(|(e, c)| (cutter.vertices[e.0 as usize], cutter.vertices[e.1 as usize], *c)).take(6).collect();
            eprintln!("cutter: {} verts {} tris, odd edges {} e.g. {:?}", cutter.vertices.len(), cutter.triangle_count(), counts.values().filter(|c| **c != 2).count(), naked);
            let cap = Face::planar(&plane, vec![polygon.clone()]).unwrap().tessellate(1, 1);
            eprintln!("cap: {} verts {} tris", cap.vertices.len(), cap.triangle_count());
        }
        assert!(cutter.is_closed(), "cutter closed");
        let bx = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap().tessellate(12, 12);
        assert!(bx.is_closed(), "box closed");
        let result = tri_geom::mesh_boolean(&bx, &cutter, BooleanOp::Difference).unwrap();
        let welded = super::super::mesh_cmds::weld(&result, 1e-7);
        let mut counts: std::collections::HashMap<(u32, u32), usize> = Default::default();
        for t in welded.indices.chunks(3) {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                *counts.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let naked = counts.values().filter(|c| **c == 1).count();
        let multi = counts.values().filter(|c| **c > 2).count();
        eprintln!("result: {} tris, naked {naked}, multi {multi}", welded.triangle_count());
        assert!(result.is_closed(), "result closed");
    }
}


/// Add a locked, named hole definition curve.
pub fn add_hole_curve(ctx: &mut CmdCtx, c: NurbsCurve) -> ObjectId {
    let attrs = tri_doc::Attributes { layer: ctx.doc.current_layer(), name: Some("Hole".into()), locked: true, ..Default::default() };
    ctx.doc.add_with(Geometry::Curve(c), attrs)
}

/// Register hole curves on a target (keeping the target's original geometry the first time).
pub fn register_hole(ctx: &mut CmdCtx, target: ObjectId, curves: Vec<ObjectId>) {
    if let Some(rec) = ctx.doc.state.history.holes.iter_mut().find(|h| h.target == target) {
        rec.curves.extend(curves);
        return;
    }
    let Some(original) = ctx.doc.get(target).map(|o| o.geometry.clone()) else { return };
    ctx.doc.state.history.holes.push(tri_doc::history::HoleRecord { target, original, curves });
}

/// Cut all registered holes of a target into its original geometry again.
pub fn recut_holes(ctx: &mut CmdCtx, target: ObjectId) {
    let Some(rec) = ctx.doc.state.history.holes.iter().find(|h| h.target == target).cloned() else { return };
    if !matches!(rec.original, Geometry::Surface(_) | Geometry::Brep(_) | Geometry::Mesh(_)) {
        return;
    }
    let mut result = rec.original.clone();
    let diag = result.bounding_box().diagonal().max(1e-3);
    let mut live = Vec::new();
    for cid in &rec.curves {
        let Some(c) = curve_of(ctx.doc, *cid) else { continue };
        live.push(*cid);
        let Some(plane) = curve_plane(&c) else { continue };
        if let Some(tool) = prism_tool(&c, &plane, diag) {
            if let Some(r) = boolean_geometry(&result, &tool, BooleanOp::Difference) {
                result = r;
            }
        }
    }
    if let Some(r) = ctx.doc.state.history.holes.iter_mut().find(|h| h.target == target) {
        r.curves = live;
    }
    ctx.doc.replace_geometry(target, result);
}

/// The hole record owning a hole curve.
pub fn hole_of_curve(ctx: &CmdCtx, curve: ObjectId) -> Option<ObjectId> {
    ctx.doc.state.history.holes.iter().find(|h| h.curves.contains(&curve)).map(|h| h.target)
}

#[cfg(test)]
mod hole_tests {
    use super::*;
    use tri_geom::Brep;

    #[test]
    fn circle_prism_cuts_box_closed() {
        let plane = Plane::from_normal(DVec3::new(5.0, 5.0, 5.0), DVec3::Z);
        let circle = NurbsCurve::circle(&plane, 2.0).unwrap();
        let cp = curve_plane(&circle).unwrap();
        eprintln!("curve plane normal {:?} origin {:?}", cp.z_axis, cp.origin);
        let bx = Brep::cuboid(&Plane::world_xy(), 20.0, 20.0, 5.0).unwrap().tessellate(12, 12);
        let diag = bx.bounding_box().diagonal();
        let prism = region_prism(&circle, &cp, diag).unwrap();
        eprintln!("prism closed {} bbox {:?}", prism.is_closed(), prism.bounding_box());
        let result = tri_geom::mesh_boolean(&bx, &prism, BooleanOp::Difference).unwrap();
        eprintln!("result closed {} tris {}", result.is_closed(), result.triangle_count());
        assert!(result.is_closed());
    }
}
