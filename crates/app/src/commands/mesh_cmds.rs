//! Mesh commands: Mesh, MeshBoolean*, Weld, Unweld, mesh primitives, ExtractPt, SplitDisjointMesh.

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use super::mesh_bool;
use tri_geom::{BooleanOp, Brep, Mesh, NurbsSurface, Plane};

use super::create::plane_at;
use super::{fmt_num, optv, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn add_selected(ctx: &mut CmdCtx, label: &str, geoms: Vec<Geometry>) -> Step {
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

fn mesh_of(doc: &Document, id: ObjectId, density: usize) -> Option<Mesh> {
    match doc.get(id).map(|o| &o.geometry) {
        Some(Geometry::Surface(s)) => Some(s.tessellate(density, density)),
        Some(Geometry::Brep(b)) => Some(b.tessellate(density, density)),
        Some(Geometry::Mesh(m)) => Some(m.clone()),
        _ => None,
    }
}

/// Merge coincident vertices.
pub fn weld(m: &Mesh, tol: f64) -> Mesh {
    use std::collections::HashMap;
    let q = tol.max(1e-12);
    let mut map: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut out = Mesh::new();
    let mut remap = Vec::with_capacity(m.vertices.len());
    for v in &m.vertices {
        let key = ((v.x / q).round() as i64, (v.y / q).round() as i64, (v.z / q).round() as i64);
        let idx = *map.entry(key).or_insert_with(|| {
            out.vertices.push(*v);
            (out.vertices.len() - 1) as u32
        });
        remap.push(idx);
    }
    for t in m.indices.chunks(3) {
        let (a, b, c) = (remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
        if a != b && b != c && a != c {
            out.indices.extend_from_slice(&[a, b, c]);
        }
    }
    out.compute_normals();
    out
}

pub fn unweld(m: &Mesh) -> Mesh {
    let mut out = Mesh::new();
    for t in m.indices.chunks(3) {
        let base = out.vertices.len() as u32;
        for i in t {
            out.vertices.push(m.vertices[*i as usize]);
        }
        out.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }
    out.compute_normals();
    out
}

/// Connected components of a mesh.
pub fn components(m: &Mesh) -> Vec<Mesh> {
    let w = weld(m, 1e-9 * (1.0 + m.bounding_box().diagonal()));
    let n = w.vertices.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut x = i;
        while p[x] != r {
            let nx = p[x];
            p[x] = r;
            x = nx;
        }
        r
    }
    for t in w.indices.chunks(3) {
        let a = find(&mut parent, t[0] as usize);
        let b = find(&mut parent, t[1] as usize);
        let c = find(&mut parent, t[2] as usize);
        parent[b] = a;
        parent[c] = a;
    }
    let mut groups: std::collections::BTreeMap<usize, Mesh> = Default::default();
    let mut remaps: std::collections::HashMap<(usize, u32), u32> = Default::default();
    for t in w.indices.chunks(3) {
        let root = find(&mut parent, t[0] as usize);
        let g = groups.entry(root).or_default();
        for i in t {
            let key = (root, *i);
            let idx = *remaps.entry(key).or_insert_with(|| {
                g.vertices.push(w.vertices[*i as usize]);
                (g.vertices.len() - 1) as u32
            });
            g.indices.push(idx);
        }
    }
    groups.into_values().map(|mut g| { g.compute_normals(); g }).collect()
}

pub struct MeshCmd;

impl Command for MeshCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select surfaces and polysurfaces to mesh", ObjFilter::Surface, 1).opts([optv("Density", "8"), optv("DeleteInput", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                let density = ctx.mem_num("Mesh.Density", 8.0) as usize;
                let out: Vec<Geometry> = ids.iter().filter_map(|id| mesh_of(ctx.doc, *id, density.max(1))).map(Geometry::Mesh).collect();
                add_selected(ctx, "Mesh", out)
            }
            Input::Option(o) if o == "Density" => Prompt::number("Mesh density (subdivisions per span)").default_num(ctx.mem_num("Mesh.Density", 8.0)).step(),
            Input::Number(d) => {
                ctx.mem_set("Mesh.Density", (d.round() as usize).clamp(1, 64));
                Prompt::objects("Select surfaces and polysurfaces to mesh", ObjFilter::Surface, 1).step()
            }
            Input::Option(_) => Prompt::objects("Select surfaces and polysurfaces to mesh", ObjFilter::Surface, 1).step(),
            _ => Step::Cancelled,
        }
    }
}

pub struct MeshBooleanCmd {
    op: BooleanOp,
    split: bool,
    first: Vec<ObjectId>,
}

impl MeshBooleanCmd {
    pub fn new(op: BooleanOp, split: bool) -> Self {
        Self { op, split, first: Vec::new() }
    }
}

impl Command for MeshBooleanCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        let msg = match (self.op, self.split) {
            (BooleanOp::Union, _) => "Select meshes to union",
            (BooleanOp::Difference, false) => "Select meshes to subtract from",
            (BooleanOp::Intersection, false) => "Select first set of meshes",
            _ => "Select meshes to split",
        };
        Prompt::objects(msg, ObjFilter::Any, 1).opts([optv("DeleteInput", "Yes")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.first.is_empty() => {
                self.first = ids;
                if self.op == BooleanOp::Union && !self.split {
                    let meshes: Vec<Mesh> = self.first.iter().filter_map(|id| mesh_of(ctx.doc, *id, 12)).collect();
                    if meshes.is_empty() {
                        return Step::Cancelled;
                    }
                    let first = self.first.clone();
                    return ctx.try_work(
                        "MeshBooleanUnion",
                        move || {
                            let mut r = meshes[0].clone();
                            for m in &meshes[1..] {
                                r = mesh_bool(&r, m, BooleanOp::Union).map_err(|e| e.to_string())?;
                            }
                            Ok(r)
                        },
                        move |r, ctx| {
                            ctx.doc.begin_undo("MeshBooleanUnion");
                            for id in &first {
                                ctx.doc.remove(*id);
                            }
                            add_selected(ctx, "MeshBooleanUnion", vec![Geometry::Mesh(r)])
                        },
                    );
                }
                ctx.doc.clear_selection();
                let msg = if self.split { "Select cutting meshes" } else if self.op == BooleanOp::Difference { "Select meshes to subtract with" } else { "Select second set of meshes" };
                Prompt::objects(msg, ObjFilter::Any, 1).step()
            }
            Input::Objects(second) => {
                let b: Vec<Mesh> = second.iter().filter_map(|id| mesh_of(ctx.doc, *id, 12)).collect();
                if b.is_empty() {
                    return Step::Cancelled;
                }
                let label = if self.split { "MeshBooleanSplit" } else if self.op == BooleanOp::Difference { "MeshBooleanDifference" } else { "MeshBooleanIntersection" };
                let targets: Vec<(ObjectId, Mesh)> = self.first.iter().filter_map(|id| mesh_of(ctx.doc, *id, 12).map(|m| (*id, m))).collect();
                let (split, op) = (self.split, self.op);
                ctx.try_work(
                    label,
                    move || -> Result<_, String> {
                        let mut cutter = b[0].clone();
                        for m in &b[1..] {
                            cutter = mesh_bool(&cutter, m, BooleanOp::Union).map_err(|e| e.to_string())?;
                        }
                        let mut results = Vec::new();
                        for (id, m) in targets {
                            let parts = if split { vec![mesh_bool(&m, &cutter, BooleanOp::Difference), mesh_bool(&m, &cutter, BooleanOp::Intersection)] } else { vec![mesh_bool(&m, &cutter, op)] };
                            results.push((id, parts.into_iter().collect::<Result<Vec<_>, _>>().map_err(|e| e.to_string())?));
                        }
                        Ok(results)
                    },
                    move |results, ctx| {
                        ctx.doc.begin_undo(label);
                        let mut out = Vec::new();
                        for (id, parts) in results {
                            out.extend(parts.into_iter().filter(|p| p.triangle_count() > 0).map(Geometry::Mesh));
                            ctx.doc.remove(id);
                        }
                        for id in &second {
                            ctx.doc.remove(*id);
                        }
                        add_selected(ctx, label, out)
                    },
                )
            }
            Input::Option(_) => self.start(ctx),
            _ => Step::Cancelled,
        }
    }
}

pub enum MeshOp {
    Weld,
    Unweld,
    SplitDisjoint,
    ExtractPt,
}

pub struct MeshOpCmd(pub MeshOp);

impl Command for MeshOpCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        let msg = match self.0 {
            MeshOp::Weld => "Select meshes to weld",
            MeshOp::Unweld => "Select meshes to unweld",
            MeshOp::SplitDisjoint => "Select meshes to split into disjoint pieces",
            MeshOp::ExtractPt => "Select meshes to extract vertices from",
        };
        Prompt::objects(msg, ObjFilter::Mesh, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        match self.0 {
            MeshOp::Weld | MeshOp::Unweld => {
                ctx.doc.begin_undo("Weld");
                for id in ids {
                    if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                        let tol = 1e-6 * (1.0 + m.bounding_box().diagonal());
                        let r = if matches!(self.0, MeshOp::Weld) { weld(&m, tol) } else { unweld(&m) };
                        ctx.doc.replace_geometry(id, Geometry::Mesh(r));
                    }
                }
                Step::Done
            }
            MeshOp::SplitDisjoint => {
                ctx.doc.begin_undo("SplitDisjointMesh");
                let mut out = Vec::new();
                for id in ids {
                    if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                        let parts = components(&m);
                        if parts.len() > 1 {
                            ctx.doc.remove(id);
                            out.extend(parts.into_iter().map(Geometry::Mesh));
                        }
                    }
                }
                add_selected(ctx, "SplitDisjointMesh", out)
            }
            MeshOp::ExtractPt => {
                let mut out = Vec::new();
                for id in ids {
                    if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                        out.extend(weld(&m, 1e-9).vertices.into_iter().map(Geometry::Point));
                    }
                }
                add_selected(ctx, "ExtractPt", out)
            }
        }
    }
}

/// Mesh primitives: MeshPlane, MeshBox, MeshSphere, MeshCylinder, MeshCone, MeshTorus.
pub struct MeshPrimCmd {
    kind: &'static str,
    pts: Vec<DVec3>,
    nums: Vec<f64>,
    plane: Plane,
}

impl MeshPrimCmd {
    pub fn new(kind: &'static str) -> Self {
        Self { kind, pts: Vec::new(), nums: Vec::new(), plane: Plane::world_xy() }
    }
    fn prompt(&self) -> Prompt {
        let n = self.pts.len() + self.nums.len();
        match (self.kind, n) {
            ("MeshPlane", 0) | ("MeshBox", 0) => Prompt::point("First corner"),
            ("MeshPlane", _) | ("MeshBox", 1) => Prompt::point("Other corner"),
            ("MeshBox", _) => Prompt::point_or_number("Height"),
            (_, 0) => Prompt::point(if self.kind == "MeshSphere" { "Center of sphere" } else if self.kind == "MeshTorus" { "Center of torus" } else { "Base" }),
            ("MeshSphere", _) => Prompt::point_or_number("Radius").default_num(1.0),
            ("MeshTorus", 1) => Prompt::point_or_number("Radius").default_num(2.0),
            ("MeshTorus", _) => Prompt::point_or_number("Second radius").default_num(0.5),
            (_, 1) => Prompt::point_or_number("Radius").default_num(1.0),
            _ => Prompt::point_or_number("End").default_num(1.0),
        }
    }
    fn number(&self, inp: &Input) -> f64 {
        match inp {
            Input::Number(v) => *v,
            Input::Point(p) => {
                let b = self.pts[0];
                if self.kind == "MeshBox" || self.kind == "MeshCylinder" || self.kind == "MeshCone" {
                    if self.nums.len() + self.pts.len() >= 2 && self.kind != "MeshBox" {
                        return (*p - b).dot(self.plane.z_axis);
                    }
                    if self.kind == "MeshBox" {
                        return (*p - b).dot(self.plane.z_axis);
                    }
                }
                (*p - b).length()
            }
            _ => 1.0,
        }
    }
}

impl Command for MeshPrimCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let need_points = match self.kind {
            "MeshPlane" | "MeshBox" => 2,
            _ => 1,
        };
        if self.pts.len() < need_points {
            if let Input::Point(p) = input {
                self.pts.push(p);
                if self.kind == "MeshPlane" && self.pts.len() == 2 {
                    let p = plane_at(&self.plane, DVec3::ZERO);
                    let (u0, v0, w) = p.closest_parameters(self.pts[0]);
                    let (u1, v1, _) = p.closest_parameters(self.pts[1]);
                    let pl = Plane { origin: p.origin + p.z_axis * w, ..p };
                    let Ok(s) = NurbsSurface::plane(&pl, u0.min(u1), u0.max(u1), v0.min(v1), v0.max(v1)) else { return Step::Cancelled };
                    return add_selected(ctx, "MeshPlane", vec![Geometry::Mesh(s.tessellate(8, 8))]);
                }
                return self.prompt().step();
            }
            return Step::Cancelled;
        }
        if !matches!(input, Input::Point(_) | Input::Number(_)) {
            return Step::Cancelled;
        }
        let v = self.number(&input);
        self.nums.push(v);
        let b = self.pts[0];
        let p = plane_at(&self.plane, b);
        let geom = match (self.kind, self.nums.len()) {
            ("MeshBox", 1) => {
                let (u0, v0, w) = self.plane.closest_parameters(self.pts[0]);
                let (u1, v1, _) = self.plane.closest_parameters(self.pts[1]);
                let base = Plane { origin: self.plane.point_at_3(u0, v0, w), ..self.plane };
                Brep::cuboid(&base, u1 - u0, v1 - v0, v).ok().map(|b| Geometry::Mesh(b.tessellate(1, 1)))
            }
            ("MeshSphere", 1) => NurbsSurface::sphere(b, v.abs()).ok().map(|s| Geometry::Mesh(s.tessellate(8, 8))),
            ("MeshCylinder", 2) | ("MeshCone", 2) => {
                let r = self.nums[0].abs();
                let h = v;
                let side = if self.kind == "MeshCylinder" { NurbsSurface::cylinder(&p, r, h) } else { NurbsSurface::cone(&p, r, h) };
                side.ok().map(|s| {
                    let mut m = s.tessellate(8, 8);
                    if let Ok(c) = NurbsSurface::disc(&p, r) {
                        m.append(&c.tessellate(8, 2));
                    }
                    if self.kind == "MeshCylinder" {
                        if let Ok(c) = NurbsSurface::disc(&Plane { origin: b + self.plane.z_axis * h, ..p }, r) {
                            m.append(&c.tessellate(8, 2));
                        }
                    }
                    Geometry::Mesh(weld(&m, 1e-6 * (1.0 + r + h.abs())))
                })
            }
            ("MeshTorus", 2) => NurbsSurface::torus(&p, self.nums[0].abs(), v.abs()).ok().map(|s| Geometry::Mesh(s.tessellate(8, 8))),
            _ => return self.prompt().step(),
        };
        match geom {
            Some(g) => add_selected(ctx, self.kind, vec![g]),
            None => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.first(), cursor) {
            (Some(b), Some(c)) if self.nums.is_empty() && self.kind != "MeshPlane" && self.kind != "MeshBox" => {
                tri_geom::NurbsCurve::circle(&plane_at(&self.plane, *b), (c - *b).length()).map(|cv| vec![Preview::Curve(cv)]).unwrap_or_default()
            }
            (Some(b), Some(c)) if self.kind == "MeshPlane" || self.kind == "MeshBox" => vec![Preview::Polyline(super::create::rect_points(&self.plane, *b, c))],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[allow(dead_code)]
fn unused() -> String {
    fmt_num(0.0)
}
