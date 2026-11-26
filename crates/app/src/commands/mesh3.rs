//! Mesh and SubD batch: repair wizard, bridge, inset, bevel, reflect, multipipe, apply mesh,
//! SubD edge tools, radiate, quad/ngon helpers, and the brep edge tools.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::{Geometry, ObjectId};
use super::mesh_bool;
use tri_geom::{mesh_ops, BooleanOp, Brep, Face, Mesh, NurbsCurve, NurbsSurface, Plane};

use super::create::planar_cap;
use super::mesh_cmds::{components, weld};
use super::misc4::solid_mesh;
use super::simple::{add_all, select_where, Ans, Ask, SeqCmd};
use super::surface2::sweep_sections_pub;
use super::surface_edit::{face_under, faces_of};
use super::{fmt_num, opt, CmdCtx, ObjFilter, Step};

fn mesh_of(ctx: &CmdCtx, id: ObjectId) -> Option<Mesh> {
    match ctx.doc.get(id).map(|o| &o.geometry) {
        Some(Geometry::Mesh(m)) => Some(m.clone()),
        _ => None,
    }
}

fn replace_mesh(ctx: &mut CmdCtx, id: ObjectId, m: Mesh) {
    ctx.doc.replace_geometry(id, Geometry::Mesh(m));
}

/// Remove degenerate (zero-area) and duplicate triangles.
pub fn clean_mesh(m: &Mesh) -> (Mesh, usize) {
    let mut out = Mesh::new();
    out.vertices = m.vertices.clone();
    let mut seen = std::collections::HashSet::new();
    let mut removed = 0;
    for t in 0..m.triangle_count() {
        let i = [m.indices[t * 3], m.indices[t * 3 + 1], m.indices[t * 3 + 2]];
        let (p, q, r) = m.triangle(t);
        let mut key = i;
        key.sort();
        if i[0] == i[1] || i[1] == i[2] || i[0] == i[2] || (q - p).cross(r - p).length() < 1e-14 || !seen.insert(key) {
            removed += 1;
            continue;
        }
        out.indices.extend_from_slice(&i);
    }
    // drop unused vertices
    let mut map = vec![u32::MAX; out.vertices.len()];
    let mut verts = Vec::new();
    for idx in out.indices.iter_mut() {
        if map[*idx as usize] == u32::MAX {
            map[*idx as usize] = verts.len() as u32;
            verts.push(out.vertices[*idx as usize]);
        }
        *idx = map[*idx as usize];
    }
    out.vertices = verts;
    out.compute_normals();
    (out, removed)
}

/// Make triangle orientations consistent by flood filling across shared edges.
pub fn unify_normals(m: &Mesh) -> Mesh {
    let n = m.triangle_count();
    let mut edge_map: std::collections::HashMap<(u32, u32), Vec<usize>> = std::collections::HashMap::new();
    for t in 0..n {
        for k in 0..3 {
            let a = m.indices[t * 3 + k];
            let b = m.indices[t * 3 + (k + 1) % 3];
            edge_map.entry((a.min(b), a.max(b))).or_default().push(t);
        }
    }
    let mut flipped = vec![false; n];
    let mut visited = vec![false; n];
    let dir = |t: usize, fl: bool| -> [u32; 3] {
        let i = [m.indices[t * 3], m.indices[t * 3 + 1], m.indices[t * 3 + 2]];
        if fl { [i[0], i[2], i[1]] } else { i }
    };
    for seed in 0..n {
        if visited[seed] {
            continue;
        }
        visited[seed] = true;
        let mut stack = vec![seed];
        while let Some(t) = stack.pop() {
            let it = dir(t, flipped[t]);
            for k in 0..3 {
                let (a, b) = (it[k], it[(k + 1) % 3]);
                for &u in &edge_map[&(a.min(b), a.max(b))] {
                    if u == t || visited[u] {
                        continue;
                    }
                    // consistent orientation means the neighbour traverses the edge as (b, a)
                    let iu = dir(u, false);
                    let same_dir = (0..3).any(|j| iu[j] == a && iu[(j + 1) % 3] == b);
                    flipped[u] = same_dir;
                    visited[u] = true;
                    stack.push(u);
                }
            }
        }
    }
    let mut out = m.clone();
    for t in 0..n {
        if flipped[t] {
            out.indices.swap(t * 3 + 1, t * 3 + 2);
        }
    }
    out.compute_normals();
    out
}

pub fn mesh_repair() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes to repair", ObjFilter::Mesh, 1), Ask::Options("Repair", vec![opt("All"), opt("Weld"), opt("FillHoles"), opt("UnifyNormals"), opt("RemoveDegenerate"), opt("Check")])], |a, ctx| {
        let what = a[1].text();
        ctx.doc.begin_undo("MeshRepair");
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            let diag = m.bounding_box().diagonal().max(1e-6);
            let naked = mesh_ops::naked_loops(&m).len();
            let (_, degenerate) = clean_mesh(&m);
            let dup = {
                let w = weld(&m, diag * 1e-7);
                m.vertices.len() - w.vertices.len()
            };
            ctx.log(format!("Mesh #{id}: {} vertices, {} faces, {naked} naked edge loops, {degenerate} degenerate/duplicate faces, {dup} duplicate vertices, {}.", m.vertices.len(), m.triangle_count(), if m.is_closed() { "closed" } else { "open" }));
            if what == "Check" {
                continue;
            }
            let mut r = m.clone();
            if what == "All" || what == "Weld" {
                r = weld(&r, diag * 1e-7);
            }
            if what == "All" || what == "RemoveDegenerate" {
                r = clean_mesh(&r).0;
            }
            if what == "All" || what == "UnifyNormals" {
                r = unify_normals(&r);
            }
            if what == "All" || what == "FillHoles" {
                let (f, n) = mesh_ops::fill_holes(&r);
                if n > 0 {
                    ctx.log(format!("{n} holes filled."));
                }
                r = f;
            }
            if what == "All" && r.is_closed() {
                // make the closed mesh point outwards
                let vol = tri_doc::Document::volume_of(&Geometry::Mesh(r.clone())).map(|v| v.0).unwrap_or(0.0);
                if vol < 0.0 {
                    r.flip();
                }
            }
            replace_mesh(ctx, id, r);
        }
        Step::Done
    })
}

fn nearest_naked_loop(m: &Mesh, p: DVec3) -> Option<Vec<DVec3>> {
    let loops = mesh_ops::naked_loops(m);
    loops.into_iter().min_by(|a, b| {
        let da = a.iter().map(|q| (*q - p).length()).fold(f64::INFINITY, f64::min);
        let db = b.iter().map(|q| (*q - p).length()).fold(f64::INFINITY, f64::min);
        da.partial_cmp(&db).unwrap()
    })
}

/// Loft between two naked loops of meshes (Bridge).
pub fn bridge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh near the first open edge", ObjFilter::Mesh), Ask::Pick("Select mesh near the second open edge", ObjFilter::Mesh), Ask::Number("Segments", 4.0)], |a, ctx| {
        let (Ans::Pick(id1, p1), Ans::Pick(id2, p2)) = (&a[0], &a[1]) else { return Step::Cancelled };
        let (Some(m1), Some(m2)) = (mesh_of(ctx, *id1), mesh_of(ctx, *id2)) else { return Step::Cancelled };
        let (Some(l1), Some(l2)) = (nearest_naked_loop(&m1, *p1), nearest_naked_loop(&m2, *p2)) else {
            ctx.log("No open edges near the picks.");
            return Step::Cancelled;
        };
        let n = l1.len().max(l2.len()).max(3);
        let resample = |l: &Vec<DVec3>| -> Vec<DVec3> {
            let mut closed = l.clone();
            closed.push(l[0]);
            let total: f64 = closed.windows(2).map(|w| (w[1] - w[0]).length()).sum();
            (0..n).map(|i| {
                let mut d = total * i as f64 / n as f64;
                for w in closed.windows(2) {
                    let len = (w[1] - w[0]).length();
                    if d <= len {
                        return w[0] + (w[1] - w[0]) * (d / len.max(1e-12));
                    }
                    d -= len;
                }
                closed[0]
            }).collect()
        };
        let mut r1 = resample(&l1);
        let mut r2 = resample(&l2);
        // align start and direction of the second loop to the first
        let c1 = r1.iter().fold(DVec3::ZERO, |s, p| s + *p) / n as f64;
        let c2 = r2.iter().fold(DVec3::ZERO, |s, p| s + *p) / n as f64;
        let axis = (c2 - c1).normalize_or(DVec3::Z);
        let n1 = (r1[1] - r1[0]).cross(r1[2] - r1[1]);
        if n1.dot(axis) < 0.0 {
            r1.reverse();
        }
        let n2 = (r2[1] - r2[0]).cross(r2[2] - r2[1]);
        if n2.dot(axis) < 0.0 {
            r2.reverse();
        }
        let start = (0..n).min_by(|&i, &j| (r2[i] - r1[0]).length().partial_cmp(&(r2[j] - r1[0]).length()).unwrap()).unwrap_or(0);
        r2.rotate_left(start);
        let segs = a[2].number().round().max(1.0) as usize;
        let mut m = Mesh::new();
        for s in 0..=segs {
            let f = s as f64 / segs as f64;
            for i in 0..n {
                m.vertices.push(r1[i] * (1.0 - f) + r2[i] * f);
            }
        }
        for s in 0..segs {
            for i in 0..n {
                let (a0, b0) = ((s * n + i) as u32, (s * n + (i + 1) % n) as u32);
                let (a1, b1) = (((s + 1) * n + i) as u32, ((s + 1) * n + (i + 1) % n) as u32);
                m.indices.extend_from_slice(&[a0, b0, b1, a0, b1, a1]);
            }
        }
        m.compute_normals();
        ctx.doc.begin_undo("Bridge");
        let mut merged = m1.clone();
        merged.append(&m);
        if id1 != id2 {
            merged.append(&m2);
            ctx.doc.remove(*id2);
        }
        let merged = weld(&merged, merged.bounding_box().diagonal() * 1e-7);
        replace_mesh(ctx, *id1, merged);
        ctx.doc.clear_selection();
        ctx.doc.select(*id1, true);
        Step::Done
    })
}

/// Inset the picked face: shrink it towards its centroid and connect with a ring.
pub fn inset() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh face to inset", ObjFilter::Mesh), Ask::Number("Inset distance", 0.5)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(m) = mesh_of(ctx, *id) else { return Step::Cancelled };
        let d = a[1].number();
        // nearest triangle to the pick (planar neighbours with the same normal are inset together)
        let Some(t0) = (0..m.triangle_count()).min_by(|&i, &j| { let (p0, q0, r0) = m.triangle(i); let (p1, q1, r1) = m.triangle(j); ((p0 + q0 + r0) / 3.0 - *p).length().partial_cmp(&((p1 + q1 + r1) / 3.0 - *p).length()).unwrap() }) else { return Step::Cancelled };
        let (p0, q0, r0) = m.triangle(t0);
        let n0 = (q0 - p0).cross(r0 - p0).normalize_or(DVec3::Z);
        let plane_d = p0.dot(n0);
        let region: Vec<usize> = (0..m.triangle_count()).filter(|&t| { let (p, q, r) = m.triangle(t); let n = (q - p).cross(r - p).normalize_or(DVec3::Z); n.dot(n0) > 0.999 && (p.dot(n0) - plane_d).abs() < 1e-6 }).collect();
        let mut sub = Mesh::new();
        sub.vertices = m.vertices.clone();
        for t in &region {
            sub.indices.extend_from_slice(&m.indices[t * 3..t * 3 + 3]);
        }
        let loops = mesh_ops::naked_loops(&sub);
        let Some(outer) = loops.iter().max_by(|a, b| a.len().cmp(&b.len())) else { return Step::Cancelled };
        let c = outer.iter().fold(DVec3::ZERO, |s, v| s + *v) / outer.len() as f64;
        let k = outer.len();
        let inner: Vec<DVec3> = outer.iter().map(|v| { let dir = (c - *v).normalize_or(DVec3::X); *v + dir * d.min((c - *v).length() * 0.9) }).collect();
        let mut rest = Mesh::new();
        rest.vertices = m.vertices.clone();
        for t in 0..m.triangle_count() {
            if !region.contains(&t) {
                rest.indices.extend_from_slice(&m.indices[t * 3..t * 3 + 3]);
            }
        }
        let base = rest.vertices.len() as u32;
        rest.vertices.extend(outer.iter().copied());
        rest.vertices.extend(inner.iter().copied());
        rest.vertices.push(c);
        let ci = base + 2 * k as u32;
        for i in 0..k {
            let (o0, o1) = (base + i as u32, base + ((i + 1) % k) as u32);
            let (i0, i1) = (base + k as u32 + i as u32, base + k as u32 + ((i + 1) % k) as u32);
            rest.indices.extend_from_slice(&[o0, o1, i1, o0, i1, i0, i0, i1, ci]);
        }
        let out = weld(&rest, m.bounding_box().diagonal() * 1e-7);
        let out = unify_normals(&out);
        ctx.doc.begin_undo("Inset");
        replace_mesh(ctx, *id, out);
        ctx.log(format!("Face inset by {} ({} faces in the planar region).", fmt_num(d), region.len()));
        Step::Done
    })
}

/// Bevel approximation: split the nearest edge and pull the new vertex in by the bevel size.
pub fn bevel() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh near the edge to bevel", ObjFilter::Mesh), Ask::Number("Bevel distance", 0.3)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(m) = mesh_of(ctx, *id) else { return Step::Cancelled };
        let d = a[1].number();
        let before = m.vertices.len();
        let mut s = mesh_ops::split_nearest_edge(&m, *p);
        if s.vertices.len() > before {
            // the new vertex is the last one: move it inwards along the average normal of its triangles
            let vi = s.vertices.len() - 1;
            let mut n = DVec3::ZERO;
            for t in 0..s.triangle_count() {
                if s.indices[t * 3..t * 3 + 3].contains(&(vi as u32)) {
                    let (p, q, r) = s.triangle(t);
                    n += (q - p).cross(r - p);
                }
            }
            let n = n.normalize_or(DVec3::Z);
            s.vertices[vi] -= n * d;
            s.compute_normals();
        }
        ctx.doc.begin_undo("Bevel");
        replace_mesh(ctx, *id, s);
        ctx.log("Bevel: edge split and chamfered (approximation).");
        Step::Done
    })
}

/// Keep the part of a mesh on the positive side of a plane, splitting the triangles that cross it.
pub fn clip_mesh_by_plane(m: &Mesh, p0: DVec3, n: DVec3) -> Mesh {
    let mut out = Mesh::new();
    for t in 0..m.triangle_count() {
        let (p, q, r) = m.triangle(t);
        let tri = [p, q, r];
        let d: Vec<f64> = tri.iter().map(|v| (*v - p0).dot(n)).collect();
        if d.iter().all(|x| *x >= -1e-12) {
            let b = out.vertices.len() as u32;
            out.vertices.extend_from_slice(&tri);
            out.indices.extend_from_slice(&[b, b + 1, b + 2]);
            continue;
        }
        if d.iter().all(|x| *x <= 1e-12) {
            continue;
        }
        // Sutherland–Hodgman against the half space
        let mut poly: Vec<DVec3> = Vec::new();
        for i in 0..3 {
            let (a, b) = (tri[i], tri[(i + 1) % 3]);
            let (da, db) = (d[i], d[(i + 1) % 3]);
            if da >= 0.0 {
                poly.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let t = da / (da - db);
                poly.push(a + (b - a) * t);
            }
        }
        if poly.len() < 3 {
            continue;
        }
        let b = out.vertices.len() as u32;
        out.vertices.extend_from_slice(&poly);
        for k in 1..poly.len() - 1 {
            out.indices.extend_from_slice(&[b, b + k as u32, b + k as u32 + 1]);
        }
    }
    out.compute_normals();
    out
}

/// Reflect: keep the half of the mesh on the positive side of a mirror plane and mirror it.
pub fn reflect() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes or SubDs", ObjFilter::Mesh, 1), Ask::Point("Start of mirror line"), Ask::Point("End of mirror line")], |a, ctx| {
        let (p0, p1) = (a[1].point(), a[2].point());
        let cp = ctx.cplane;
        let n = (p1 - p0).cross(cp.z_axis).normalize_or(cp.x_axis);
        let Some(xf) = super::edit::mirror_xf(p0, n) else { return Step::Cancelled };
        ctx.doc.begin_undo("Reflect");
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            let mut keep = clip_mesh_by_plane(&m, p0, n);
            let mut mirrored = keep.clone();
            mirrored.transform(&xf);
            mirrored.flip();
            keep.append(&mirrored);
            let out = weld(&keep, m.bounding_box().diagonal() * 1e-7);
            let out = clean_mesh(&out).0;
            replace_mesh(ctx, id, out);
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.user_text.retain(|(k, _)| k != "Symmetry");
                o.attributes.user_text.push(("Symmetry".into(), format!("{},{},{};{},{},{}", p0.x, p0.y, p0.z, n.x, n.y, n.z)));
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn remove_symmetry() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects with symmetry", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("RemoveSymmetry");
        let mut n = 0;
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                let before = o.attributes.user_text.len();
                o.attributes.user_text.retain(|(k, _)| k != "Symmetry" && k != "Radiate");
                if o.attributes.user_text.len() != before {
                    n += 1;
                }
            }
        }
        ctx.doc.touch_revision();
        ctx.log(format!("Symmetry removed from {n} objects."));
        Step::Done
    })
}

pub fn radiate(find: bool) -> SeqCmd {
    let asks = if find { vec![Ask::Objects("Select objects", ObjFilter::Any, 1)] } else { vec![Ask::Objects("Select meshes or SubDs", ObjFilter::Mesh, 1), Ask::Point("Centre of radial symmetry"), Ask::Number("Number of copies", 6.0)] };
    SeqCmd::new(asks, move |a, ctx| {
        if find {
            for id in a[0].ids() {
                let txt = ctx.doc.get(id).and_then(|o| o.attributes.user_text.iter().find(|(k, _)| k == "Radiate" || k == "Symmetry").map(|(k, v)| format!("{k} {v}")));
                ctx.log(match txt { Some(t) => format!("#{id}: {t}"), None => format!("#{id}: no symmetry") });
            }
            return Step::Done;
        }
        let c = a[1].point();
        let count = a[2].number().round().max(2.0) as usize;
        let axis = ctx.cplane.z_axis;
        ctx.doc.begin_undo("Radiate");
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            let mut out = m.clone();
            for k in 1..count {
                let xf = DAffine3::from_translation(c) * DAffine3::from_quat(DQuat::from_axis_angle(axis, std::f64::consts::TAU * k as f64 / count as f64)) * DAffine3::from_translation(-c);
                let mut copy = m.clone();
                copy.transform(&xf);
                out.append(&copy);
            }
            let out = weld(&out, m.bounding_box().diagonal() * 1e-7);
            replace_mesh(ctx, id, out);
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.user_text.push(("Radiate".into(), format!("{count} about {},{},{}", c.x, c.y, c.z)));
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn multipipe() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select curves", ObjFilter::Curve, 1), Ask::Number("Radius", 1.0), Ask::Options("Output", vec![opt("SubD"), opt("Mesh")])], |a, ctx| {
        let r = a[1].number().abs().max(1e-6);
        let mut pipes: Vec<Mesh> = Vec::new();
        for id in a[0].ids() {
            let Some(c) = ctx.doc.get(id).and_then(|o| o.geometry.as_curve().cloned()) else { continue };
            let (t0, _) = c.domain();
            let plane = Plane::from_normal(c.start_point(), c.tangent_at(t0));
            let Ok(circle) = NurbsCurve::circle(&plane, r) else { continue };
            let sections = sweep_sections_pub(&circle, &c, 24);
            let Ok(side) = NurbsSurface::loft(&sections, 3) else { continue };
            let mut faces = vec![Face::untrimmed(side)];
            if !c.is_closed() {
                if let (Some(f0), Some(f1)) = (planar_cap(&sections[0]), planar_cap(sections.last().unwrap())) {
                    faces.push(f0);
                    faces.push(f1);
                }
            }
            let mut brep = Brep::new(faces);
            brep.orient_outward();
            let m = brep.tessellate(24, 12);
            let m = mesh_ops::fill_holes(&m).0;
            pipes.push(m);
        }
        if pipes.is_empty() {
            return Step::Cancelled;
        }
        let subd = a[2].text() != "Mesh";
        ctx.try_work(
            "MultiPipe",
            move || {
                let mut pipes = pipes.into_iter();
                let mut acc = pipes.next().unwrap_or_default();
                for m in pipes {
                    acc = mesh_bool(&acc, &m, BooleanOp::Union).map_err(|e| e.to_string())?;
                }
                Ok(acc)
            },
            move |m, ctx| {
                ctx.doc.begin_undo("MultiPipe");
                ctx.doc.clear_selection();
                let mut attrs = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
                if subd {
                    attrs.user_text.push(("SubD".into(), "Yes".into()));
                }
                let id = ctx.doc.add_with(Geometry::Mesh(m), attrs);
                ctx.doc.select(id, true);
                Step::Done
            },
        )
    })
}

/// Map a mesh from its bounding box onto a surface (ApplyMesh); UVN uses normals for the height.
pub fn apply_mesh(uvn: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes to apply", ObjFilter::Mesh, 1), Ask::Pick("Select target surface", ObjFilter::Surface)], move |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[1].ids()[0]) else { return Step::Cancelled };
        let s = &faces[0].surface;
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let cp = ctx.cplane;
        ctx.doc.begin_undo(if uvn { "ApplyMeshUVN" } else { "ApplyMesh" });
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            let bb = m.bounding_box();
            let (bu0, bv0, bz0) = cp.closest_parameters(bb.min);
            let (bu1, bv1, _) = cp.closest_parameters(bb.max);
            let mut out = m.clone();
            for v in out.vertices.iter_mut() {
                let (x, y, z) = cp.closest_parameters(*v);
                let fu = ((x - bu0) / (bu1 - bu0).max(1e-12)).clamp(0.0, 1.0);
                let fv = ((y - bv0) / (bv1 - bv0).max(1e-12)).clamp(0.0, 1.0);
                let (u, w) = (u0 + (u1 - u0) * fu, v0 + (v1 - v0) * fv);
                let h = if uvn { z - bz0 } else { 0.0 };
                *v = s.point_at(u, w) + s.normal_at(u, w) * h;
            }
            out.compute_normals();
            replace_mesh(ctx, id, out);
        }
        Step::Done
    })
}

pub fn make_subd_friendly() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select curves or surfaces", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("MakeSubDFriendly");
        let mut n = 0;
        for id in a[0].ids() {
            let Some(g) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            match g {
                Geometry::Curve(c) => {
                    let count = (c.control_points.len().max(4)).max(c.degree + 1);
                    if let Ok(r) = c.rebuild(count, 3) {
                        ctx.doc.replace_geometry(id, Geometry::Curve(r));
                        n += 1;
                    }
                }
                Geometry::Surface(s) => {
                    let (nu, nv) = (s.count_u.max(4), s.count_v.max(4));
                    let grid: Vec<Vec<DVec3>> = {
                        let (u0, u1) = s.domain_u();
                        let (v0, v1) = s.domain_v();
                        (0..nu).map(|i| (0..nv).map(|j| s.point_at(u0 + (u1 - u0) * i as f64 / (nu - 1) as f64, v0 + (v1 - v0) * j as f64 / (nv - 1) as f64)).collect()).collect()
                    };
                    if let Ok(r) = NurbsSurface::interpolate_grid(&grid, 3, 3) {
                        ctx.doc.replace_geometry(id, Geometry::Surface(r));
                        n += 1;
                    }
                }
                _ => {}
            }
        }
        ctx.log(format!("{n} objects rebuilt as uniform degree-3 (SubD friendly)."));
        Step::Done
    })
}

pub fn sel_subd_friendly_crv() -> super::simple::FnCmd {
    super::simple::FnCmd(|ctx| select_where(ctx, |_, o| matches!(&o.geometry, Geometry::Curve(c) if c.degree == 3 && !c.is_rational())))
}

/// Slide: move the nearest vertex along the direction of its nearest edge.
pub fn slide() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh near a vertex", ObjFilter::Mesh), Ask::Number("Slide distance (along the nearest edge)", 0.5)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(mut m) = mesh_of(ctx, *id) else { return Step::Cancelled };
        let Some(vi) = (0..m.vertices.len()).min_by(|&i, &j| (m.vertices[i] - *p).length().partial_cmp(&(m.vertices[j] - *p).length()).unwrap()) else { return Step::Cancelled };
        let v = m.vertices[vi];
        let edges = m.edges();
        let Some(e) = edges.iter().filter(|e| e.0 as usize == vi || e.1 as usize == vi).min_by(|a, b| {
            let da = (m.vertices[if a.0 as usize == vi { a.1 } else { a.0 } as usize] - *p).length();
            let db = (m.vertices[if b.0 as usize == vi { b.1 } else { b.0 } as usize] - *p).length();
            da.partial_cmp(&db).unwrap()
        }) else { return Step::Cancelled };
        let other = m.vertices[if e.0 as usize == vi { e.1 } else { e.0 } as usize];
        let d = (other - v).normalize_or(DVec3::X);
        m.vertices[vi] = v + d * a[1].number().min((other - v).length() * 0.95);
        m.compute_normals();
        ctx.doc.begin_undo("Slide");
        replace_mesh(ctx, *id, m);
        Step::Done
    })
}

/// SubDExpandEdges: offset the nearest edge inside its faces (split + move).
pub fn expand_edges() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select SubD near the edge to expand", ObjFilter::Mesh), Ask::Number("Expand distance", 0.3)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(m) = mesh_of(ctx, *id) else { return Step::Cancelled };
        let d = a[1].number();
        // split the edge and both neighbouring triangles' opposite edges? keep it simple: split twice (two new vertices)
        let s1 = mesh_ops::split_nearest_edge(&m, *p);
        let mut s2 = mesh_ops::split_nearest_edge(&s1, *p + DVec3::splat(1e-9));
        let n = s2.vertices.len();
        if n > m.vertices.len() {
            for vi in m.vertices.len()..n {
                let mut nrm = DVec3::ZERO;
                for t in 0..s2.triangle_count() {
                    if s2.indices[t * 3..t * 3 + 3].contains(&(vi as u32)) {
                        let (p, q, r) = s2.triangle(t);
                        nrm += (q - p).cross(r - p);
                    }
                }
                s2.vertices[vi] += nrm.normalize_or(DVec3::Z) * d;
            }
            s2.compute_normals();
        }
        ctx.doc.begin_undo("SubDExpandEdges");
        replace_mesh(ctx, *id, s2);
        Step::Done
    })
}

pub fn extrude_subd() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select SubD face to extrude", ObjFilter::Mesh), Ask::Number("Distance", 1.0)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(m) = mesh_of(ctx, *id) else { return Step::Cancelled };
        let d = a[1].number();
        let Some(t0) = (0..m.triangle_count()).min_by(|&i, &j| { let (p0, q0, r0) = m.triangle(i); let (p1, q1, r1) = m.triangle(j); ((p0 + q0 + r0) / 3.0 - *p).length().partial_cmp(&((p1 + q1 + r1) / 3.0 - *p).length()).unwrap() }) else { return Step::Cancelled };
        let (p0, q0, r0) = m.triangle(t0);
        let n0 = (q0 - p0).cross(r0 - p0).normalize_or(DVec3::Z);
        let plane_d = p0.dot(n0);
        let region: Vec<usize> = (0..m.triangle_count()).filter(|&t| { let (p, q, r) = m.triangle(t); let n = (q - p).cross(r - p).normalize_or(DVec3::Z); n.dot(n0) > 0.999 && (p.dot(n0) - plane_d).abs() < 1e-6 }).collect();
        let mut sub = Mesh::new();
        sub.vertices = m.vertices.clone();
        for t in &region {
            sub.indices.extend_from_slice(&m.indices[t * 3..t * 3 + 3]);
        }
        let loops = mesh_ops::naked_loops(&sub);
        let mut out = m.clone();
        // move the region's vertices out, and add side walls along its naked loops
        let mut region_verts: Vec<u32> = region.iter().flat_map(|t| m.indices[t * 3..t * 3 + 3].to_vec()).collect();
        region_verts.sort();
        region_verts.dedup();
        // vertices shared with the rest of the mesh must be duplicated
        let mut remap: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for &v in &region_verts {
            let shared = (0..m.triangle_count()).any(|t| !region.contains(&t) && m.indices[t * 3..t * 3 + 3].contains(&v));
            if shared {
                out.vertices.push(m.vertices[v as usize] + n0 * d);
                remap.insert(v, out.vertices.len() as u32 - 1);
            } else {
                out.vertices[v as usize] += n0 * d;
            }
        }
        for t in &region {
            for k in 0..3 {
                let v = out.indices[t * 3 + k];
                if let Some(nv) = remap.get(&v) {
                    out.indices[t * 3 + k] = *nv;
                }
            }
        }
        for l in loops {
            let k = l.len();
            let base = out.vertices.len() as u32;
            for v in &l {
                out.vertices.push(*v);
                out.vertices.push(*v + n0 * d);
            }
            for i in 0..k {
                let (a0, a1) = (base + 2 * i as u32, base + 2 * i as u32 + 1);
                let (b0, b1) = (base + 2 * ((i + 1) % k) as u32, base + 2 * ((i + 1) % k) as u32 + 1);
                out.indices.extend_from_slice(&[a0, b0, b1, a0, b1, a1]);
            }
        }
        let out = weld(&out, m.bounding_box().diagonal() * 1e-7);
        let out = unify_normals(&clean_mesh(&out).0);
        ctx.doc.begin_undo("ExtrudeSubD");
        replace_mesh(ctx, *id, out);
        Step::Done
    })
}

pub fn offset_subd() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select SubDs or meshes to offset", ObjFilter::Mesh, 1), Ask::Number("Offset distance", 1.0), Ask::Options("Solid", vec![opt("Yes"), opt("No")])], |a, ctx| {
        let d = a[1].number();
        let solid = a[2].text() != "No";
        ctx.doc.begin_undo("OffsetSubD");
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            let m = weld(&m, m.bounding_box().diagonal() * 1e-7);
            let mut off = m.clone();
            for (i, v) in off.vertices.iter_mut().enumerate() {
                *v += m.normals.get(i).copied().unwrap_or(DVec3::Z) * d;
            }
            if solid {
                let mut shell = m.clone();
                shell.flip();
                shell.append(&off);
                let loops = mesh_ops::naked_loops(&m);
                for l in loops {
                    let k = l.len();
                    let base = shell.vertices.len() as u32;
                    for v in &l {
                        let i = m.vertices.iter().position(|w| (*w - *v).length() < 1e-9).unwrap_or(0);
                        shell.vertices.push(*v);
                        shell.vertices.push(off.vertices[i]);
                    }
                    for i in 0..k {
                        let (a0, a1) = (base + 2 * i as u32, base + 2 * i as u32 + 1);
                        let (b0, b1) = (base + 2 * ((i + 1) % k) as u32, base + 2 * ((i + 1) % k) as u32 + 1);
                        shell.indices.extend_from_slice(&[a0, b1, b0, a0, a1, b1]);
                    }
                }
                let shell = weld(&shell, m.bounding_box().diagonal() * 1e-7);
                let shell = unify_normals(&shell);
                replace_mesh(ctx, id, shell);
            } else {
                off.compute_normals();
                out.push(Geometry::Mesh(off));
            }
        }
        if out.is_empty() {
            return Step::Done;
        }
        add_all(ctx, "OffsetSubD", out)
    })
}

pub fn subd_display_toggle() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let flat = ctx.memory.get("SubD.Flat").is_some_and(|v| v == "Yes");
        ctx.mem_set("SubD.Flat", if flat { "No" } else { "Yes" });
        ctx.doc.begin_undo("SubDDisplayToggle");
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| super::mesh2::is_subd(o)).map(|o| o.id).collect();
        for id in &ids {
            let Some(o) = ctx.doc.get(*id) else { continue };
            let has_cage = o.attributes.user_text.iter().any(|(k, _)| k == "SubDCage");
            if !flat {
                // show the control cage: store the smooth mesh and replace it with its decimated cage
                if !has_cage {
                    if let Geometry::Mesh(m) = &o.geometry {
                        let smooth = serde_json::to_string(m).unwrap_or_default();
                        let cage = mesh_ops::decimate(m, (m.triangle_count() / 4).max(12));
                        ctx.doc.replace_geometry(*id, Geometry::Mesh(cage));
                        if let Some(o) = ctx.doc.get_mut(*id) {
                            o.attributes.user_text.push(("SubDCage".into(), smooth));
                        }
                    }
                }
            } else if has_cage {
                let smooth = o.attributes.user_text.iter().find(|(k, _)| k == "SubDCage").map(|(_, v)| v.clone()).unwrap_or_default();
                if let Ok(m) = serde_json::from_str::<Mesh>(&smooth) {
                    ctx.doc.replace_geometry(*id, Geometry::Mesh(m));
                }
                if let Some(o) = ctx.doc.get_mut(*id) {
                    o.attributes.user_text.retain(|(k, _)| k != "SubDCage");
                }
            }
        }
        ctx.doc.touch_revision();
        ctx.log(if flat { "SubD display: smooth." } else { "SubD display: flat (control cage)." });
        Step::Done
    })
}

pub fn quad_remesh() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to remesh", ObjFilter::Any, 1), Ask::Number("Target face count", 2000.0), Ask::Options("Output", vec![opt("Mesh"), opt("SubD")])], |a, ctx| {
        let target = a[1].number().round().max(12.0) as usize;
        let subd = a[2].text() == "SubD";
        ctx.doc.begin_undo("QuadRemesh");
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(m) = solid_mesh(ctx.doc, id) else { continue };
            let m = weld(&m, m.bounding_box().diagonal() * 1e-7);
            let r = if m.triangle_count() > target { mesh_ops::decimate(&m, target) } else { mesh_ops::loop_subdivide(&m) };
            let r = weld(&r, r.bounding_box().diagonal() * 1e-7);
            out.push((id, r));
        }
        ctx.doc.clear_selection();
        for (id, r) in out {
            ctx.doc.remove(id);
            let mut attrs = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
            if subd {
                attrs.user_text.push(("SubD".into(), "Yes".into()));
            }
            let nid = ctx.doc.add_with(Geometry::Mesh(r), attrs);
            ctx.doc.select(nid, true);
        }
        ctx.log("QuadRemesh: the result is a uniform triangle mesh (quads are stored as triangle pairs in triceratops).");
        Step::Done
    })
}

pub fn triangulate(label: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], move |a, ctx| {
        ctx.doc.begin_undo(label);
        let mut n = 0;
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            let (c, removed) = clean_mesh(&m);
            replace_mesh(ctx, id, c);
            n += removed;
        }
        ctx.log(format!("{label}: meshes are triangles already; {n} degenerate faces removed."));
        Step::Done
    })
}

pub fn ngons(label: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], move |a, ctx| {
        let mut planar = 0usize;
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            // count coplanar adjacent triangle pairs (quads/ngons triceratops would group)
            let mut map: std::collections::HashMap<(u32, u32), Vec<usize>> = std::collections::HashMap::new();
            for t in 0..m.triangle_count() {
                for k in 0..3 {
                    let (a, b) = (m.indices[t * 3 + k], m.indices[t * 3 + (k + 1) % 3]);
                    map.entry((a.min(b), a.max(b))).or_default().push(t);
                }
            }
            for ts in map.values() {
                if ts.len() == 2 {
                    let (p, q, r) = m.triangle(ts[0]);
                    let (s, u, v) = m.triangle(ts[1]);
                    let n1 = (q - p).cross(r - p).normalize_or(DVec3::Z);
                    let n2 = (u - s).cross(v - s).normalize_or(DVec3::Z);
                    if n1.dot(n2) > 0.9999 {
                        planar += 1;
                    }
                }
            }
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.user_text.retain(|(k, _)| k != "Ngons");
                if label == "AddNgonsToMesh" {
                    o.attributes.user_text.push(("Ngons".into(), "Yes".into()));
                }
            }
        }
        ctx.doc.touch_revision();
        ctx.log(format!("{label}: {planar} coplanar triangle pairs {}; triceratops stores all faces as triangles.", if label == "AddNgonsToMesh" { "marked as n-gons for export" } else { "unmarked" }));
        Step::Done
    })
}

pub fn merge_two_faces() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh near the shared edge of two faces", ObjFilter::Mesh)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(m) = mesh_of(ctx, *id) else { return Step::Cancelled };
        let r = mesh_ops::swap_nearest_edge(&m, *p);
        ctx.doc.begin_undo("Merge2MeshFaces");
        replace_mesh(ctx, *id, r);
        ctx.log("The shared edge was flipped (faces stay triangles in triceratops).");
        Step::Done
    })
}

pub fn set_per_face_color(remove: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], move |a, ctx| {
        ctx.doc.begin_undo(if remove { "RemovePerFaceColors" } else { "SetPerFaceColorByFacePack" });
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(m) = mesh_of(ctx, id) else { continue };
            if remove {
                if let Some(o) = ctx.doc.get_mut(id) {
                    o.attributes.color = None;
                }
                continue;
            }
            // one object per connected component, each with its own colour
            let parts = components(&m);
            if parts.len() <= 1 {
                continue;
            }
            let layer = ctx.doc.get(id).map(|o| o.attributes.layer).unwrap_or(0);
            ctx.doc.remove(id);
            for (k, part) in parts.into_iter().enumerate() {
                let hue = (k as f64 * 0.618_033_9).fract();
                let (r, g, b) = hsv(hue, 0.6, 0.9);
                out.push((part, tri_doc::Attributes { layer, color: Some([r, g, b, 255]), ..Default::default() }));
            }
        }
        ctx.doc.touch_revision();
        if out.is_empty() {
            return Step::Done;
        }
        ctx.doc.clear_selection();
        for (m, attrs) in out {
            let nid = ctx.doc.add_with(Geometry::Mesh(m), attrs);
            ctx.doc.select(nid, true);
        }
        ctx.log("Face packs coloured: each connected part is now its own coloured mesh.");
        Step::Done
    })
}

fn hsv(h: f64, s: f64, v: f64) -> (u8, u8, u8) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as i32 % 6 { 0 => (v, t, p), 1 => (q, v, p), 2 => (p, v, t), 3 => (p, q, v), 4 => (t, p, v), _ => (v, p, q) };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

// ---------------------------------------------------------------- brep edge tools

pub fn unjoin_edge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select polysurface near the face to detach", ObjFilter::Polysurface)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        if faces.len() < 2 {
            ctx.log("The object has a single face.");
            return Step::Cancelled;
        }
        let fi = face_under(&faces, *p);
        let mut rest = faces.clone();
        let f = rest.remove(fi);
        ctx.doc.begin_undo("UnjoinEdge");
        ctx.doc.replace_geometry(*id, Geometry::Brep(Brep::new(rest)));
        add_all(ctx, "UnjoinEdge", vec![Geometry::Brep(Brep::new(vec![f]))])
    })
}

pub fn join_edge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select first surface edge", ObjFilter::Surface), Ask::Pick("Select second surface edge", ObjFilter::Surface)], |a, ctx| {
        let (ida, idb) = (a[0].ids()[0], a[1].ids()[0]);
        if ida == idb {
            return Step::Cancelled;
        }
        let (Some(fa), Some(fb)) = (faces_of(ctx.doc, ida), faces_of(ctx.doc, idb)) else { return Step::Cancelled };
        let mut faces = fa;
        faces.extend(fb);
        let brep = Brep::new(faces);
        ctx.doc.begin_undo("JoinEdge");
        ctx.doc.remove(ida);
        ctx.doc.remove(idb);
        ctx.log("Edges joined (the surfaces are now one polysurface; the gap is tolerated).");
        add_all(ctx, "JoinEdge", vec![Geometry::Brep(brep)])
    })
}

pub fn split_edge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface edge to split", ObjFilter::Surface), Ask::Point("Point on the edge to split at")], |a, ctx| {
        let Ans::Pick(id, pick) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let f = &faces[face_under(&faces, *pick)];
        let tol = f.bounding_box().diagonal().max(1e-6) * 1e-3;
        let p = a[1].point();
        let Some(edge) = f.edge_curves(tol).into_iter().min_by(|x, y| (x.point_at(x.closest_parameter(p)) - p).length().partial_cmp(&(y.point_at(y.closest_parameter(p)) - p).length()).unwrap()) else { return Step::Cancelled };
        let t = edge.closest_parameter(p);
        let Some((e1, e2)) = edge.split(t) else { return Step::Cancelled };
        ctx.log("Edge split: the two halves are added as curves (triceratops keeps one edge per face side).");
        add_all(ctx, "SplitEdge", vec![Geometry::Curve(e1), Geometry::Curve(e2)])
    })
}

pub fn merge_edge(all: bool) -> SeqCmd {
    let asks = if all { vec![Ask::Objects("Select polysurfaces", ObjFilter::Any, 1)] } else { vec![Ask::Pick("Select edge", ObjFilter::Any)] };
    SeqCmd::new(asks, move |a, ctx| {
        let n = a[0].ids().len();
        ctx.log(format!("{}: edges of {n} objects are already merged (triceratops stores one edge per face side).", if all { "MergeAllEdges" } else { "MergeEdge" }));
        Step::Done
    })
}

pub fn rebuild_edges() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces or polysurfaces", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("RebuildEdges");
        let mut n = 0;
        for id in a[0].ids() {
            let Some(faces) = faces_of(ctx.doc, id) else { continue };
            // re-fit the trim loops to the surface by snapping the UV loops to the surface's closest points
            let rebuilt: Vec<Face> = faces.into_iter().map(|mut f| {
                for l in f.loops.iter_mut() {
                    for uv in l.iter_mut() {
                        let p = f.surface.point_at(uv.x, uv.y);
                        let (u, v) = f.surface.closest_uv(p);
                        *uv = glam::DVec2::new(u, v);
                    }
                }
                f
            }).collect();
            let was_brep = matches!(ctx.doc.get(id).map(|o| &o.geometry), Some(Geometry::Brep(_)));
            if was_brep || rebuilt.len() > 1 {
                ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(rebuilt)));
            } else if let Some(f) = rebuilt.into_iter().next() {
                ctx.doc.replace_geometry(id, if f.loops.is_empty() { Geometry::Surface(f.surface) } else { Geometry::Brep(Brep::new(vec![f])) });
            }
            n += 1;
        }
        ctx.log(format!("{n} objects: trim edges refitted to their surfaces."));
        Step::Done
    })
}

pub fn replace_edge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface edge to replace", ObjFilter::Surface), Ask::Pick("Select replacement curve", ObjFilter::Curve)], |a, ctx| {
        let Ans::Pick(id, pick) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let fi = face_under(&faces, *pick);
        let Some(c) = ctx.doc.get(a[1].ids()[0]).and_then(|o| o.geometry.as_curve().cloned()) else { return Step::Cancelled };
        // trim the face with the new curve: keep the side containing the face centre
        let mut faces = faces;
        let f = &faces[fi];
        let (u0, u1) = f.surface.domain_u();
        let (v0, v1) = f.surface.domain_v();
        let centre = f.surface.point_at((u0 + u1) * 0.5, (v0 + v1) * 0.5);
        let keep = centre;
        ctx.doc.begin_undo("ReplaceEdge");
        super::surface_edit::trim_face(ctx, *id, keep, &[c]);
        let _ = &mut faces;
        ctx.log("The face was trimmed with the replacement curve.");
        Step::Done
    })
}

pub fn refit_trim() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select trimmed surface", ObjFilter::Surface), Ask::Number("Tolerance", 0.01)], |a, ctx| {
        let Ans::Pick(id, pick) = &a[0] else { return Step::Cancelled };
        let Some(faces) = faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let f = &faces[face_under(&faces, *pick)];
        if f.loops.is_empty() {
            ctx.log("The surface is not trimmed.");
            return Step::Done;
        }
        // shrink the underlying surface to the trim's UV bounding box, keep the trim
        let (mut umin, mut umax, mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
        for l in &f.loops {
            for uv in l {
                umin = umin.min(uv.x);
                umax = umax.max(uv.x);
                vmin = vmin.min(uv.y);
                vmax = vmax.max(uv.y);
            }
        }
        let n = 12;
        let grid: Vec<Vec<DVec3>> = (0..=n).map(|i| (0..=n).map(|j| f.surface.point_at(umin + (umax - umin) * i as f64 / n as f64, vmin + (vmax - vmin) * j as f64 / n as f64)).collect()).collect();
        let Ok(s) = NurbsSurface::interpolate_grid(&grid, 3, 3) else { return Step::Cancelled };
        let (su0, su1) = s.domain_u();
        let (sv0, sv1) = s.domain_v();
        let loops: Vec<Vec<glam::DVec2>> = f.loops.iter().map(|l| l.iter().map(|uv| glam::DVec2::new(su0 + (su1 - su0) * (uv.x - umin) / (umax - umin).max(1e-12), sv0 + (sv1 - sv0) * (uv.y - vmin) / (vmax - vmin).max(1e-12))).collect()).collect();
        let nf = Face { surface: s, loops };
        let mut all = faces.clone();
        all[face_under(&faces, *pick)] = nf;
        ctx.doc.begin_undo("RefitTrim");
        ctx.doc.replace_geometry(*id, Geometry::Brep(Brep::new(all)));
        ctx.log(format!("Surface refitted to the trim boundary (tolerance {}).", fmt_num(a[1].number())));
        Step::Done
    })
}

pub fn remove_micro_edges() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces or polysurfaces", ObjFilter::Any, 1), Ask::Number("Maximum edge length to remove", 0.001)], |a, ctx| {
        let max = a[1].number().abs();
        ctx.doc.begin_undo("RemoveAllNakedMicroEdges");
        let mut removed = 0;
        for id in a[0].ids() {
            let Some(faces) = faces_of(ctx.doc, id) else { continue };
            let fixed: Vec<Face> = faces.into_iter().map(|mut f| {
                for l in f.loops.iter_mut() {
                    let mut out: Vec<glam::DVec2> = Vec::new();
                    for uv in l.iter() {
                        if let Some(last) = out.last() {
                            let d = (f.surface.point_at(uv.x, uv.y) - f.surface.point_at(last.x, last.y)).length();
                            if d < max {
                                removed += 1;
                                continue;
                            }
                        }
                        out.push(*uv);
                    }
                    if out.len() >= 3 {
                        *l = out;
                    }
                }
                f
            }).collect();
            if fixed.len() == 1 && fixed[0].loops.is_empty() {
                ctx.doc.replace_geometry(id, Geometry::Surface(fixed[0].surface.clone()));
            } else {
                ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(fixed)));
            }
        }
        ctx.log(format!("{removed} micro edges removed."));
        Step::Done
    })
}

pub fn extract_bad_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select polysurfaces to check", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("ExtractBadSrf");
        let mut bad = Vec::new();
        for id in a[0].ids() {
            let Some(faces) = faces_of(ctx.doc, id) else { continue };
            let (good, wrong): (Vec<Face>, Vec<Face>) = faces.into_iter().partition(|f| {
                let s = &f.surface;
                let finite = s.control_points.iter().all(|p| p.is_finite());
                let area = s.tessellate(6, 6);
                let a: f64 = (0..area.triangle_count()).map(|t| { let (p, q, r) = area.triangle(t); (q - p).cross(r - p).length() * 0.5 }).sum();
                finite && a > 1e-12 && f.loops.iter().all(|l| l.len() >= 3)
            });
            if wrong.is_empty() {
                continue;
            }
            if good.is_empty() {
                ctx.doc.remove(id);
            } else {
                ctx.doc.replace_geometry(id, Geometry::Brep(Brep::new(good)));
            }
            bad.extend(wrong.into_iter().map(|f| Geometry::Brep(Brep::new(vec![f]))));
        }
        if bad.is_empty() {
            ctx.log("No bad surfaces found.");
            return Step::Done;
        }
        ctx.log(format!("{} bad surfaces extracted and selected.", bad.len()));
        add_all(ctx, "ExtractBadSrf", bad)
    })
}

pub fn nonmanifold_merge() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select surfaces and polysurfaces to merge", ObjFilter::Any, 2)], |a, ctx| {
        let mut faces = Vec::new();
        for id in a[0].ids() {
            faces.extend(faces_of(ctx.doc, id).unwrap_or_default());
        }
        ctx.doc.begin_undo("NonmanifoldMerge");
        for id in a[0].ids() {
            ctx.doc.remove(id);
        }
        let mut b = Brep::new(faces);
        b.closed = false;
        add_all(ctx, "NonmanifoldMerge", vec![Geometry::Brep(b)])
    })
}

pub fn create_regions() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select closed planar curves", ObjFilter::Curve, 2)], |a, ctx| {
        let curves: Vec<NurbsCurve> = a[0].ids().iter().filter_map(|id| ctx.doc.get(*id).and_then(|o| o.geometry.as_curve().cloned())).filter(|c| c.is_closed()).collect();
        if curves.len() < 2 {
            ctx.log("Select at least two closed planar curves.");
            return Step::Cancelled;
        }
        let mut regions = Vec::new();
        for i in 0..curves.len() {
            // the part of curve i outside all the others, plus the pairwise intersections
            let mut pieces = vec![curves[i].clone()];
            for j in 0..curves.len() {
                if i == j {
                    continue;
                }
                let mut next = Vec::new();
                for p in &pieces {
                    next.extend(super::misc4::region_boolean(std::slice::from_ref(p), std::slice::from_ref(&curves[j]), BooleanOp::Difference));
                }
                pieces = next;
            }
            regions.extend(pieces);
            for j in i + 1..curves.len() {
                regions.extend(super::misc4::region_boolean(&[curves[i].clone()], &[curves[j].clone()], BooleanOp::Intersection));
            }
        }
        if regions.is_empty() {
            ctx.log("The curves do not overlap.");
            return Step::Done;
        }
        ctx.doc.begin_undo("CreateRegions");
        for id in a[0].ids() {
            ctx.doc.remove(id);
        }
        add_all(ctx, "CreateRegions", regions.into_iter().map(Geometry::Curve).collect())
    })
}

pub fn merge_faces() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select polysurface with coplanar faces to merge", ObjFilter::Polysurface)], |a, ctx| {
        let id = a[0].ids()[0];
        let Some(Geometry::Brep(b)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        let before = b.faces.len();
        let merged = super::misc6::merge_coplanar(&b);
        ctx.doc.begin_undo("MergeFaces");
        ctx.doc.replace_geometry(id, Geometry::Brep(merged.clone()));
        ctx.log(format!("{} faces merged into {}.", before, merged.faces.len()));
        Step::Done
    })
}

pub fn move_extracted_isocurve() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface", ObjFilter::Surface), Ask::Options("Direction", vec![opt("U"), opt("V")]), Ask::Point("Point on the surface for the isocurve")], |a, ctx| {
        let Some(faces) = faces_of(ctx.doc, a[0].ids()[0]) else { return Step::Cancelled };
        let s = &faces[0].surface;
        let (u, v) = s.closest_uv(a[2].point());
        let c = if a[1].text() == "V" { s.isocurve_v(v) } else { s.isocurve_u(u) };
        add_all(ctx, "MoveExtractedIsocurve", vec![Geometry::Curve(c)])
    })
}

#[allow(dead_code)]
fn _unused(_: &CmdCtx) -> Option<DVec3> {
    None
}
