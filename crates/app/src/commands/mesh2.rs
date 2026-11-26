//! Mesh editing and SubD-as-mesh commands.

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{mesh_ops, Mesh, NurbsCurve, NurbsSurface, Plane};

use super::create::plane_at;
use super::mesh_cmds::{components, unweld, weld};
use super::simple::{add_all, select_where, Ans, Ask, SeqCmd};
use super::{fmt_num, CmdCtx, Command, Input, ObjFilter, Step};

fn mesh_of(doc: &Document, id: ObjectId) -> Option<Mesh> {
    match doc.get(id).map(|o| &o.geometry) {
        Some(Geometry::Mesh(m)) => Some(m.clone()),
        Some(Geometry::Surface(s)) => Some(s.tessellate(12, 12)),
        Some(Geometry::Brep(b)) => Some(b.tessellate(12, 12)),
        _ => None,
    }
}

/// Tag a mesh as a SubD (kept in the object name's user text).
fn mark_subd(ctx: &mut CmdCtx, id: ObjectId) {
    if let Some(o) = ctx.doc.get_mut(id) {
        o.attributes.user_text.retain(|(k, _)| k != "SubD");
        o.attributes.user_text.push(("SubD".into(), "1".into()));
    }
}

pub fn is_subd(o: &tri_doc::Object) -> bool {
    matches!(o.geometry, Geometry::Mesh(_)) && o.attributes.user_text.iter().any(|(k, _)| k == "SubD")
}

/// Replace each selected mesh with `f(mesh)`.
pub fn mesh_map(label: &'static str, prompt: &'static str, f: fn(&Mesh) -> Mesh) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects(prompt, ObjFilter::Mesh, 1)], move |a, ctx| {
        ctx.doc.begin_undo(label);
        for id in a[0].ids() {
            if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                ctx.doc.replace_geometry(id, Geometry::Mesh(f(&m)));
            }
        }
        Step::Done
    })
}

/// Pick-based edit: `f(mesh, pick point)`.
pub fn mesh_pick(label: &'static str, prompt: &'static str, f: fn(&Mesh, DVec3) -> Mesh) -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick(prompt, ObjFilter::Mesh)], move |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        if let Some(Geometry::Mesh(m)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) {
            ctx.doc.begin_undo(label);
            ctx.doc.replace_geometry(*id, Geometry::Mesh(f(&m, *p)));
        }
        Step::Done
    })
}

pub fn fill_holes_cmd(all: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects(if all { "Select meshes to fill holes" } else { "Select mesh to fill a hole" }, ObjFilter::Mesh, 1)], |a, ctx| {
        ctx.doc.begin_undo("FillMeshHoles");
        let mut n = 0;
        for id in a[0].ids() {
            if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                let (filled, k) = mesh_ops::fill_holes(&m);
                n += k;
                ctx.doc.replace_geometry(id, Geometry::Mesh(filled));
            }
        }
        ctx.log(format!("{n} holes filled."));
        Step::Done
    })
}

pub fn dup_mesh_edges() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                for l in mesh_ops::naked_loops(&m) {
                    let mut pts = l;
                    if pts.len() >= 3 {
                        pts.push(pts[0]);
                    }
                    if let Ok(c) = NurbsCurve::polyline(pts) {
                        out.push(Geometry::Curve(c));
                    }
                }
            }
        }
        if out.is_empty() {
            ctx.log("The meshes have no naked edges.");
            return Step::Done;
        }
        add_all(ctx, "DupMeshEdge", out)
    })
}

pub fn mesh_intersect(self_only: bool) -> SeqCmd {
    let asks = if self_only { vec![Ask::Objects("Select meshes to check for self-intersection", ObjFilter::Mesh, 1)] } else { vec![Ask::Objects("Select meshes to intersect", ObjFilter::Any, 2)] };
    SeqCmd::new(asks, move |a, ctx| {
        let ids = a[0].ids();
        let meshes: Vec<Mesh> = ids.iter().filter_map(|id| mesh_of(ctx.doc, *id)).collect();
        let mut out = Vec::new();
        if self_only {
            for m in &meshes {
                for p in mesh_ops::mesh_self_intersection(m) {
                    if let Ok(c) = NurbsCurve::polyline(p) {
                        out.push(Geometry::Curve(c));
                    }
                }
            }
        } else {
            for i in 0..meshes.len() {
                for j in i + 1..meshes.len() {
                    for p in mesh_ops::mesh_intersection(&meshes[i], &meshes[j]) {
                        if let Ok(c) = NurbsCurve::polyline(p) {
                            out.push(Geometry::Curve(c));
                        }
                    }
                }
            }
        }
        if out.is_empty() {
            ctx.log("No intersections found.");
            return Step::Done;
        }
        add_all(ctx, if self_only { "MeshSelfIntersect" } else { "MeshIntersect" }, out)
    })
}

pub fn silhouette_cmd(project: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces, polysurfaces or meshes", ObjFilter::Any, 1)], move |a, ctx| {
        let dir = ctx.view_dir;
        let cp = ctx.cplane;
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx.doc, id) else { continue };
            for (p, q) in mesh_ops::silhouette(&m, dir) {
                let (p, q) = if project {
                    let proj = |x: DVec3| x - cp.z_axis * (x - cp.origin).dot(cp.z_axis);
                    (proj(p), proj(q))
                } else {
                    (p, q)
                };
                if let Ok(c) = NurbsCurve::line(p, q) {
                    out.push(Geometry::Curve(c));
                }
            }
        }
        add_all(ctx, if project { "MeshOutline" } else { "Silhouette" }, out)
    })
}

pub fn mesh_from_points() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select points, curves or meshes", ObjFilter::Any, 3)], |a, ctx| {
        let mut pts = Vec::new();
        for id in a[0].ids() {
            match ctx.doc.get(id).map(|o| &o.geometry) {
                Some(Geometry::Point(p)) => pts.push(*p),
                Some(Geometry::Curve(c)) => pts.extend(c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3)),
                Some(Geometry::Mesh(m)) => pts.extend(m.vertices.iter().copied()),
                _ => {}
            }
        }
        if pts.len() < 3 {
            ctx.log("At least three points are needed.");
            return Step::Cancelled;
        }
        let m = mesh_ops::delaunay(&pts, &ctx.cplane);
        add_all(ctx, "MeshFromPoints", vec![Geometry::Mesh(m)])
    })
}

pub fn mesh_polyline() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select closed planar polylines", ObjFilter::Curve, 1)], |a, ctx| {
        let curves: Vec<NurbsCurve> = a[0].ids().iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| o.geometry.as_curve().cloned()).collect();
        let faces = super::create::planar_faces(&curves);
        add_all(ctx, "MeshPolyline", faces.iter().map(|f| Geometry::Mesh(f.tessellate(1, 1))).collect())
    })
}

pub fn extract_mesh_faces_by(kind: &'static str) -> SeqCmd {
    let (label, prompt) = match kind {
        "Area" => ("ExtractMeshFacesByArea", "Maximum face area to extract"),
        "AspectRatio" => ("ExtractMeshFacesByAspectRatio", "Minimum aspect ratio to extract"),
        "EdgeLength" => ("ExtractMeshFacesByEdgeLength", "Maximum edge length to extract"),
        _ => ("ExtractMeshFacesByDraftAngle", "Maximum draft angle (degrees) to extract"),
    };
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1), Ask::Number(prompt, 1.0)], move |a, ctx| {
        let v = a[1].number();
        let up = ctx.cplane.z_axis;
        ctx.doc.begin_undo(label);
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let mut keep = Mesh::new();
            let mut take = Mesh::new();
            keep.vertices = m.vertices.clone();
            take.vertices = m.vertices.clone();
            for t in 0..m.triangle_count() {
                let (p, q, r) = m.triangle(t);
                let e = [(q - p).length(), (r - q).length(), (p - r).length()];
                let area = (q - p).cross(r - p).length() * 0.5;
                let n = (q - p).cross(r - p).normalize_or(up);
                let matches = match kind {
                    "Area" => area <= v,
                    "AspectRatio" => e.iter().cloned().fold(0.0, f64::max) / e.iter().cloned().fold(f64::INFINITY, f64::min).max(1e-12) >= v,
                    "EdgeLength" => e.iter().cloned().fold(0.0, f64::max) <= v,
                    _ => n.dot(up).clamp(-1.0, 1.0).acos().to_degrees() <= v,
                };
                let dst = if matches { &mut take } else { &mut keep };
                dst.indices.extend_from_slice(&m.indices[t * 3..t * 3 + 3]);
            }
            if take.indices.is_empty() {
                continue;
            }
            keep.compute_normals();
            take.compute_normals();
            if keep.indices.is_empty() {
                ctx.doc.remove(id);
            } else {
                ctx.doc.replace_geometry(id, Geometry::Mesh(weld(&keep, 1e-9)));
            }
            out.push(Geometry::Mesh(weld(&take, 1e-9)));
        }
        if out.is_empty() {
            ctx.log("No faces matched.");
            return Step::Done;
        }
        add_all(ctx, label, out)
    })
}

pub fn extract_mesh_faces_pick() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh face to extract", ObjFilter::Mesh)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(Geometry::Mesh(m)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        let (ti, _) = (0..m.triangle_count())
            .map(|t| { let (x, y, z) = m.triangle(t); (t, ((x + y + z) / 3.0 - *p).length()) })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap_or((0, 0.0));
        let mut keep = Mesh::new();
        keep.vertices = m.vertices.clone();
        let mut take = Mesh::new();
        take.vertices = m.vertices.clone();
        for t in 0..m.triangle_count() {
            let dst = if t == ti { &mut take } else { &mut keep };
            dst.indices.extend_from_slice(&m.indices[t * 3..t * 3 + 3]);
        }
        keep.compute_normals();
        take.compute_normals();
        ctx.doc.begin_undo("ExtractMeshFaces");
        ctx.doc.replace_geometry(*id, Geometry::Mesh(weld(&keep, 1e-9)));
        add_all(ctx, "ExtractMeshFaces", vec![Geometry::Mesh(weld(&take, 1e-9))])
    })
}

pub fn extract_duplicate_faces() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], |a, ctx| {
        let mut out = Vec::new();
        ctx.doc.begin_undo("ExtractDuplicateMeshFaces");
        for id in a[0].ids() {
            let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let w = weld(&m, 1e-9 * (1.0 + m.bounding_box().diagonal()));
            let mut seen = std::collections::HashSet::new();
            let mut keep = Mesh::new();
            keep.vertices = w.vertices.clone();
            let mut dup = Mesh::new();
            dup.vertices = w.vertices.clone();
            for t in w.indices.chunks(3) {
                let mut k = [t[0], t[1], t[2]];
                k.sort();
                let dst = if seen.insert(k) { &mut keep } else { &mut dup };
                dst.indices.extend_from_slice(t);
            }
            if dup.indices.is_empty() {
                continue;
            }
            keep.compute_normals();
            dup.compute_normals();
            ctx.doc.replace_geometry(id, Geometry::Mesh(keep));
            out.push(Geometry::Mesh(dup));
        }
        if out.is_empty() {
            ctx.log("No duplicate faces found.");
            return Step::Done;
        }
        add_all(ctx, "ExtractDuplicateMeshFaces", out)
    })
}

pub fn extract_nonmanifold() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let w = weld(&m, 1e-9 * (1.0 + m.bounding_box().diagonal()));
            let mut count: std::collections::HashMap<(u32, u32), usize> = Default::default();
            for t in w.indices.chunks(3) {
                for k in 0..3 {
                    let (p, q) = (t[k], t[(k + 1) % 3]);
                    *count.entry((p.min(q), p.max(q))).or_default() += 1;
                }
            }
            for (e, c) in count {
                if c > 2 {
                    if let Ok(l) = NurbsCurve::line(w.vertices[e.0 as usize], w.vertices[e.1 as usize]) {
                        out.push(Geometry::Curve(l));
                    }
                }
            }
        }
        if out.is_empty() {
            ctx.log("No non-manifold edges found.");
            return Step::Done;
        }
        add_all(ctx, "ExtractNonManifoldMeshEdges", out)
    })
}

pub fn is_nonmanifold(m: &Mesh) -> bool {
    let w = weld(m, 1e-9 * (1.0 + m.bounding_box().diagonal()));
    let mut count: std::collections::HashMap<(u32, u32), usize> = Default::default();
    for t in w.indices.chunks(3) {
        for k in 0..3 {
            let (p, q) = (t[k], t[(k + 1) % 3]);
            *count.entry((p.min(q), p.max(q))).or_default() += 1;
        }
    }
    count.values().any(|c| *c > 2)
}

pub fn reduce_mesh() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes to reduce", ObjFilter::Mesh, 1), Ask::Number("Reduction percentage", 50.0)], |a, ctx| {
        let pct = a[1].number().clamp(1.0, 99.0);
        ctx.doc.begin_undo("ReduceMesh");
        for id in a[0].ids() {
            if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                let target = ((m.triangle_count() as f64) * (1.0 - pct / 100.0)).max(4.0) as usize;
                ctx.doc.replace_geometry(id, Geometry::Mesh(mesh_ops::decimate(&m, target)));
            }
        }
        Step::Done
    })
}

pub fn align_vertices() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1), Ask::Number("Distance", 0.01)], |a, ctx| {
        let tol = a[1].number().abs().max(1e-12);
        ctx.doc.begin_undo("AlignVertices");
        for id in a[0].ids() {
            if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                ctx.doc.replace_geometry(id, Geometry::Mesh(weld(&m, tol)));
            }
        }
        Step::Done
    })
}

pub fn stitch() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first vertex (pick near it)", ObjFilter::Mesh), Ask::Point("Second vertex location")], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let q = a[1].point();
        let Some(Geometry::Mesh(m)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        let mut w = weld(&m, 1e-9);
        let near = |pt: DVec3, verts: &Vec<DVec3>| verts.iter().enumerate().min_by(|x, y| (*x.1 - pt).length().partial_cmp(&(*y.1 - pt).length()).unwrap()).map(|x| x.0);
        let (Some(i), Some(j)) = (near(*p, &w.vertices), near(q, &w.vertices)) else { return Step::Cancelled };
        let mid = (w.vertices[i] + w.vertices[j]) * 0.5;
        w.vertices[i] = mid;
        w.vertices[j] = mid;
        ctx.doc.begin_undo("Stitch");
        ctx.doc.replace_geometry(*id, Geometry::Mesh(weld(&w, 1e-9)));
        Step::Done
    })
}

pub fn subdivide_cmd(label: &'static str, levels_prompt: bool) -> SeqCmd {
    let asks = if levels_prompt { vec![Ask::Objects("Select meshes or SubDs to subdivide", ObjFilter::Mesh, 1), Ask::Number("Subdivision levels", 1.0)] } else { vec![Ask::Objects("Select meshes to convert to SubD", ObjFilter::Mesh, 1)] };
    SeqCmd::new(asks, move |a, ctx| {
        let levels = a.get(1).map(|n| n.number().round().clamp(1.0, 4.0) as usize).unwrap_or(1);
        ctx.doc.begin_undo(label);
        for id in a[0].ids() {
            if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                let mut r = m;
                for _ in 0..levels {
                    r = mesh_ops::loop_subdivide(&r);
                }
                ctx.doc.replace_geometry(id, Geometry::Mesh(r));
                if label == "ToSubD" {
                    mark_subd(ctx, id);
                }
            }
        }
        Step::Done
    })
}

/// SubD primitives: a mesh primitive smoothed by one Loop subdivision step and tagged as SubD.
pub struct SubDPrimCmd {
    inner: super::mesh_cmds::MeshPrimCmd,
    before: Vec<ObjectId>,
}

impl SubDPrimCmd {
    pub fn new(kind: &'static str) -> Self {
        Self { inner: super::mesh_cmds::MeshPrimCmd::new(kind), before: Vec::new() }
    }
    fn finish(&self, step: Step, ctx: &mut CmdCtx) -> Step {
        if matches!(step, Step::Done) {
            let new: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).filter(|id| !self.before.contains(id)).collect();
            for id in new {
                if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                    ctx.doc.replace_geometry(id, Geometry::Mesh(mesh_ops::loop_subdivide(&mesh_ops::loop_subdivide(&m))));
                    mark_subd(ctx, id);
                }
            }
        }
        step
    }
}

impl Command for SubDPrimCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.before = ctx.doc.objects().map(|o| o.id).collect();
        let s = self.inner.start(ctx);
        self.finish(s, ctx)
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let s = self.inner.input(input, ctx);
        self.finish(s, ctx)
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<super::Preview> {
        self.inner.preview(cursor, doc)
    }
    fn base_point(&self) -> Option<DVec3> {
        self.inner.base_point()
    }
}

/// Mesh ellipsoid / truncated cone primitives.
pub fn mesh_ellipsoid() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Center of ellipsoid"), Ask::PointOrNumber("First radius", 2.0), Ask::PointOrNumber("Second radius", 1.5), Ask::PointOrNumber("Third radius", 1.0)], |a, ctx| {
        let c = a[0].point();
        let cp = ctx.cplane;
        let axes = [cp.x_axis, cp.y_axis, cp.z_axis];
        let r: Vec<f64> = (0..3).map(|i| match &a[i + 1] { Ans::Number(n) => n.abs(), Ans::Point(p) => (*p - c).dot(axes[i]).abs(), _ => 1.0 }).collect();
        let Ok(mut s) = NurbsSurface::sphere(DVec3::ZERO, 1.0) else { return Step::Cancelled };
        let m = glam::DMat3::from_cols(cp.x_axis * r[0].max(1e-6), cp.y_axis * r[1].max(1e-6), cp.z_axis * r[2].max(1e-6));
        s.transform(&glam::DAffine3::from_mat3_translation(m, c));
        add_all(ctx, "MeshEllipsoid", vec![Geometry::Mesh(weld(&s.tessellate(12, 12), 1e-9))])
    })
}

pub fn mesh_tcone() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Base of truncated cone"), Ask::PointOrNumber("Base radius", 2.0), Ask::PointOrNumber("Height", 3.0), Ask::PointOrNumber("Top radius", 1.0)], |a, ctx| {
        let b = a[0].point();
        let cp = ctx.cplane;
        let r1 = match &a[1] { Ans::Number(n) => n.abs(), Ans::Point(p) => (*p - b).length(), _ => 2.0 };
        let h = match &a[2] { Ans::Number(n) => *n, Ans::Point(p) => (*p - b).dot(cp.z_axis), _ => 3.0 };
        let r2 = match &a[3] { Ans::Number(n) => n.abs(), Ans::Point(p) => (*p - (b + cp.z_axis * h)).length(), _ => 1.0 };
        let p0 = plane_at(&cp, b);
        let p1 = plane_at(&cp, b + cp.z_axis * h);
        let (Ok(c0), Ok(c1)) = (NurbsCurve::circle(&p0, r1.max(1e-6)), NurbsCurve::circle(&p1, r2.max(1e-6))) else { return Step::Cancelled };
        let Ok(side) = NurbsSurface::loft(&[c0, c1], 1) else { return Step::Cancelled };
        let mut m = side.tessellate(24, 1);
        if let Ok(d) = NurbsSurface::disc(&p0, r1.max(1e-6)) {
            m.append(&d.tessellate(24, 2));
        }
        if let Ok(d) = NurbsSurface::disc(&p1, r2.max(1e-6)) {
            m.append(&d.tessellate(24, 2));
        }
        add_all(ctx, "MeshTruncatedCone", vec![Geometry::Mesh(weld(&m, 1e-6 * (1.0 + r1 + h.abs())))])
    })
}

pub fn shrink_wrap() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to wrap", ObjFilter::Any, 1)], |a, ctx| {
        let meshes: Vec<Mesh> = a[0].ids().iter().filter_map(|id| mesh_of(ctx.doc, *id)).collect();
        if meshes.is_empty() {
            return Step::Cancelled;
        }
        ctx.try_work(
            "ShrinkWrap",
            move || {
                let mut r = meshes[0].clone();
                for m in &meshes[1..] {
                    r = super::mesh_bool(&r, m, tri_geom::BooleanOp::Union).map_err(|e| e.to_string())?;
                }
                Ok(r)
            },
            |r, ctx| {
                ctx.log("ShrinkWrap: union of the objects' meshes (no offset or hole closing).");
                add_all(ctx, "ShrinkWrap", vec![Geometry::Mesh(r)])
            },
        )
    })
}

pub fn sel_subd(kind: &'static str) -> super::simple::FnCmd {
    match kind {
        "open" => super::simple::FnCmd(|ctx| select_where(ctx, |_, o| is_subd(o) && matches!(&o.geometry, Geometry::Mesh(m) if !m.is_closed()))),
        "closed" => super::simple::FnCmd(|ctx| select_where(ctx, |_, o| is_subd(o) && matches!(&o.geometry, Geometry::Mesh(m) if m.is_closed()))),
        _ => super::simple::FnCmd(|ctx| select_where(ctx, |_, o| is_subd(o))),
    }
}

pub fn crease_cmd(remove: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects(if remove { "Select meshes to smooth (weld)" } else { "Select meshes to crease (unweld)" }, ObjFilter::Mesh, 1)], move |a, ctx| {
        ctx.doc.begin_undo(if remove { "RemoveCrease" } else { "Crease" });
        for id in a[0].ids() {
            if let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) {
                let r = if remove { weld(&m, 1e-9 * (1.0 + m.bounding_box().diagonal())) } else { unweld(&m) };
                ctx.doc.replace_geometry(id, Geometry::Mesh(r));
            }
        }
        Step::Done
    })
}

pub fn sel_mesh_part() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh face", ObjFilter::Mesh)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(Geometry::Mesh(m)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        let parts = components(&m);
        if parts.len() > 1 {
            ctx.log(format!("The mesh has {} disjoint parts; selecting the whole mesh object (sub-object selection is not supported).", parts.len()));
        }
        let _ = p;
        ctx.doc.select(*id, true);
        Step::Done
    })
}

pub fn mesh_to_nurb_or_subd(label: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to convert", ObjFilter::Any, 1)], move |a, ctx| {
        let mut out = Vec::new();
        ctx.doc.begin_undo(label);
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            match &o.geometry {
                Geometry::Mesh(m) => {
                    let mut faces = Vec::new();
                    for t in 0..m.triangle_count() {
                        let (p, q, r) = m.triangle(t);
                        let n = (q - p).cross(r - p);
                        if n.length_squared() < 1e-24 {
                            continue;
                        }
                        if let Ok(f) = tri_geom::Face::planar(&Plane::from_normal(p, n.normalize()), vec![vec![p, q, r]]) {
                            faces.push(f);
                        }
                    }
                    if !faces.is_empty() {
                        ctx.doc.remove(id);
                        out.push(Geometry::Brep(tri_geom::Brep::new(faces)));
                    }
                }
                Geometry::Curve(_) | Geometry::Surface(_) | Geometry::Brep(_) => {
                    ctx.log(format!("Object #{id} is already NURBS geometry."));
                }
                Geometry::Instance { .. } => {
                    let parts = ctx.doc.instance_geometry(&o.geometry);
                    ctx.doc.remove(id);
                    out.extend(parts.into_iter().map(|(g, _)| g));
                }
                Geometry::Point(_) => {}
            }
        }
        if out.is_empty() {
            return Step::Done;
        }
        add_all(ctx, label, out)
    })
}

pub fn collapse_by(kind: &'static str) -> SeqCmd {
    let (label, prompt) = match kind {
        "Area" => ("CollapseMeshFacesByArea", "Collapse faces with area below"),
        "AspectRatio" => ("CollapseMeshFacesByAspectRatio", "Collapse faces with aspect ratio above"),
        _ => ("CollapseMeshFacesByEdgeLength", "Collapse faces with an edge shorter than"),
    };
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1), Ask::Number(prompt, 0.1)], move |a, ctx| {
        let v = a[1].number();
        ctx.doc.begin_undo(label);
        for id in a[0].ids() {
            let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            let mut r = m;
            for _ in 0..64 {
                let mut target: Option<DVec3> = None;
                for t in 0..r.triangle_count() {
                    let (p, q, s) = r.triangle(t);
                    let e = [(q - p).length(), (s - q).length(), (p - s).length()];
                    let hit = match kind {
                        "Area" => (q - p).cross(s - p).length() * 0.5 < v,
                        "AspectRatio" => e.iter().cloned().fold(0.0, f64::max) / e.iter().cloned().fold(f64::INFINITY, f64::min).max(1e-12) > v,
                        _ => e.iter().cloned().fold(f64::INFINITY, f64::min) < v,
                    };
                    if hit {
                        let k = (0..3).min_by(|x, y| e[*x].partial_cmp(&e[*y]).unwrap()).unwrap();
                        let pts = [p, q, s];
                        target = Some((pts[k] + pts[(k + 1) % 3]) * 0.5);
                        break;
                    }
                }
                let Some(mid) = target else { break };
                let before = r.triangle_count();
                r = mesh_ops::collapse_nearest_edge(&r, mid);
                if r.triangle_count() == before {
                    break;
                }
            }
            ctx.doc.replace_geometry(id, Geometry::Mesh(r));
        }
        Step::Done
    })
}

#[allow(dead_code)]
fn unused() -> String {
    fmt_num(0.0)
}
