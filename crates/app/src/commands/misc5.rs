//! Fifth batch: UnrollSrf/Smash, MeshSplit/MeshTrim, DimOrdinate/DimRotated,
//! CurvatureGraph, Symmetry.

use glam::{DAffine3, DVec2, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{curve_plane, mesh_boolean, BooleanOp, Mesh, NurbsCurve, NurbsSurface, Plane};

use super::annotate::text_curves;
use super::misc4::{prism_mesh, solid_mesh};
use super::surface_edit::faces_of;
use super::{fmt_num, opt, optv, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

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

fn add_group(ctx: &mut CmdCtx, label: &str, curves: Vec<NurbsCurve>) -> Step {
    if curves.is_empty() {
        return Step::Cancelled;
    }
    let n = ctx.memory.get("GroupCounter").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) + 1;
    ctx.mem_set("GroupCounter", n);
    let group = format!("{label}{n:02}");
    ctx.doc.begin_undo(label);
    ctx.doc.clear_selection();
    for c in curves {
        let mut a = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
        a.groups.push(group.clone());
        let id = ctx.doc.add_with(Geometry::Curve(c), a);
        ctx.doc.select(id, true);
    }
    Step::Done
}

// ---- UnrollSrf / Smash -----------------------------------------------------------------------

/// Flatten a sampled surface grid onto the XY plane by unfolding triangle strips
/// (exact for developable surfaces, approximate otherwise).
pub fn unroll_grid(grid: &[Vec<DVec3>]) -> Vec<Vec<DVec2>> {
    let nu = grid.len();
    let nv = grid[0].len();
    let mut out = vec![vec![DVec2::ZERO; nv]; nu];
    // first row along v: lay out by arc length on the x axis
    let mut x = 0.0;
    for j in 0..nv {
        if j > 0 {
            x += (grid[0][j] - grid[0][j - 1]).length();
        }
        out[0][j] = DVec2::new(x, 0.0);
    }
    for i in 1..nu {
        // first point of the row: distance from the previous row's first point, direction +y
        let d0 = (grid[i][0] - grid[i - 1][0]).length();
        out[i][0] = out[i - 1][0] + DVec2::new(0.0, d0);
        for j in 1..nv {
            // place P[i][j] from A = out[i][j-1] and B = out[i-1][j] by triangle edge lengths
            let a3 = grid[i][j - 1];
            let b3 = grid[i - 1][j];
            let p3 = grid[i][j];
            let (a, b) = (out[i][j - 1], out[i - 1][j]);
            let la = (p3 - a3).length();
            let lb = (p3 - b3).length();
            let ab = b - a;
            let d = ab.length().max(1e-12);
            let t = ((la * la - lb * lb + d * d) / (2.0 * d)).clamp(-la, la);
            let h2 = (la * la - t * t).max(0.0);
            let h = h2.sqrt();
            let dir = ab / d;
            let perp = DVec2::new(-dir.y, dir.x);
            // choose the side away from the previous row's point (out[i-1][j-1])
            let prev = out[i - 1][j - 1];
            let cand1 = a + dir * t + perp * h;
            let cand2 = a + dir * t - perp * h;
            out[i][j] = if (cand1 - prev).length() >= (cand2 - prev).length() { cand1 } else { cand2 };
        }
    }
    out
}

pub struct UnrollCmd(pub &'static str);

impl Command for UnrollCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects(if self.0 == "Smash" { "Select surfaces to smash" } else { "Select surfaces to unroll" }, ObjFilter::Surface, 1).opts([optv("Explode", "Yes"), optv("Labels", "No")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                let cp = ctx.cplane;
                let mut out = Vec::new();
                let mut offset_x = 0.0;
                for id in ids {
                    for f in faces_of(ctx.doc, id).unwrap_or_default() {
                        let s = &f.surface;
                        let (u0, u1) = s.domain_u();
                        let (v0, v1) = s.domain_v();
                        let (nu, nv) = ((s.count_u * 4).clamp(2, 64), (s.count_v * 4).clamp(2, 64));
                        let grid: Vec<Vec<DVec3>> = (0..nu).map(|i| (0..nv).map(|j| s.point_at(u0 + (u1 - u0) * i as f64 / (nu - 1) as f64, v0 + (v1 - v0) * j as f64 / (nv - 1) as f64)).collect()).collect();
                        let flat = unroll_grid(&grid);
                        let (mut minx, mut maxx) = (f64::INFINITY, f64::NEG_INFINITY);
                        for row in &flat {
                            for p in row {
                                minx = minx.min(p.x);
                                maxx = maxx.max(p.x);
                            }
                        }
                        let shift = offset_x - minx;
                        offset_x += (maxx - minx) * 1.1;
                        let grid3: Vec<Vec<DVec3>> = flat.iter().map(|row| row.iter().map(|p| cp.point_at(p.x + shift, p.y)).collect()).collect();
                        if let Ok(ns) = NurbsSurface::from_grid(1, 1, grid3) {
                            out.push(Geometry::Surface(ns));
                        }
                    }
                }
                if self.0 == "Smash" {
                    ctx.log("Smash: surfaces flattened by unfolding sample strips (approximate for doubly curved surfaces).");
                }
                add_all(ctx, self.0, out)
            }
            Input::Option(_) => self.start(ctx),
            _ => Step::Cancelled,
        }
    }
}

// ---- MeshSplit / MeshTrim -----------------------------------------------------------------------------

pub struct MeshSplitCmd {
    trim: bool,
    first: Vec<ObjectId>,
}

impl MeshSplitCmd {
    pub fn new(trim: bool) -> Self {
        Self { trim, first: Vec::new() }
    }
}

/// Closed cutter mesh for a cutting object: closed meshes/solids as they are, planar faces as a half-space box.
fn cutter_mesh(doc: &Document, id: ObjectId, pick: DVec3, target_bb: &tri_geom::BoundingBox) -> Option<Mesh> {
    let diag = target_bb.diagonal().max(1e-3) * 2.0;
    let o = doc.get(id)?;
    match &o.geometry {
        Geometry::Mesh(m) if m.is_closed() => Some(m.clone()),
        Geometry::Brep(b) if b.closed => Some(b.tessellate(12, 12)),
        Geometry::Surface(_) | Geometry::Brep(_) => {
            let faces = faces_of(doc, id)?;
            let fi = super::surface_edit::face_under(&faces, pick);
            let (p, n) = faces[fi].sample();
            let (p, n) = {
                let (u, v) = faces[fi].surface.closest_uv(pick);
                let _ = (p, n);
                (faces[fi].surface.point_at(u, v), faces[fi].surface.normal_at(u, v))
            };
            let plane = Plane::from_normal(p, n);
            let c = target_bb.center();
            let (cu, cv, _) = plane.closest_parameters(c);
            let poly = [plane.point_at(cu - diag, cv - diag), plane.point_at(cu + diag, cv - diag), plane.point_at(cu + diag, cv + diag), plane.point_at(cu - diag, cv + diag)];
            // half-space below the plane: box from -2*diag to 0 along the normal
            let below = Plane { origin: p - n * diag, ..plane };
            let poly: Vec<DVec3> = poly.iter().map(|q| *q - n * diag).collect();
            prism_mesh(&poly, &below, diag)
        }
        Geometry::Curve(c) => {
            let plane = curve_plane(c)?;
            let tol = c.bounding_box().diagonal().max(1e-6) * 2e-4;
            let polygon = c.tessellate_adaptive(tol);
            prism_mesh(&polygon, &plane, diag)
        }
        _ => None,
    }
}

impl Command for MeshSplitCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects(if self.trim { "Select mesh to trim" } else { "Select meshes to split" }, ObjFilter::Mesh, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.first.is_empty() => {
                self.first = ids;
                ctx.doc.clear_selection();
                Prompt::pick("Select cutting object (closed mesh, solid, planar surface or closed curve)", ObjFilter::Any).step()
            }
            Input::Pick(cid, p) => {
                let bb = self.first.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&ctx.doc.object_bbox(o)));
                let Some(cutter) = cutter_mesh(ctx.doc, cid, p, &bb) else {
                    ctx.log("The cutting object must be a closed mesh, a solid, a planar surface or a closed planar curve.");
                    return Step::Cancelled;
                };
                ctx.doc.begin_undo(if self.trim { "MeshTrim" } else { "MeshSplit" });
                let mut out = Vec::new();
                for id in self.first.clone() {
                    let Some(m) = solid_mesh(ctx.doc, id) else { continue };
                    let inside = mesh_boolean(&m, &cutter, BooleanOp::Intersection);
                    let outside = mesh_boolean(&m, &cutter, BooleanOp::Difference);
                    ctx.doc.remove(id);
                    if self.trim {
                        // keep the part away from the pick point
                        let keep = if (inside.bounding_box().center() - p).length() > (outside.bounding_box().center() - p).length() { inside } else { outside };
                        if keep.triangle_count() > 0 {
                            out.push(Geometry::Mesh(keep));
                        }
                    } else {
                        for part in [inside, outside] {
                            if part.triangle_count() > 0 {
                                out.push(Geometry::Mesh(part));
                            }
                        }
                    }
                }
                add_all(ctx, if self.trim { "MeshTrim" } else { "MeshSplit" }, out)
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- DimOrdinate / DimRotated ---------------------------------------------------------------------------

#[derive(Default)]
pub struct DimOrdinateCmd {
    pts: Vec<DVec3>,
    direction: &'static str,
}

impl Command for DimOrdinateCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        self.direction = "Automatic";
        Prompt::point("Select point to dimension").opts([optv("Direction", self.direction), opt("BasePoint")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Direction" => {
                self.direction = match self.direction { "Automatic" => "X", "X" => "Y", _ => "Automatic" };
                Prompt::point("Select point to dimension").opts([optv("Direction", self.direction)]).step()
            }
            Input::Option(_) => Prompt::point("Select point to dimension").step(),
            Input::Point(p) => {
                self.pts.push(p);
                if self.pts.len() < 2 {
                    return Prompt::point("Leader end point").step();
                }
                let (a, b) = (self.pts[0], self.pts[1]);
                let cp = ctx.cplane;
                let (ua, va, _) = cp.closest_parameters(a);
                let (ub, vb, _) = cp.closest_parameters(b);
                let base = ctx.memory.get("DimOrdinate.Base").map(|s| s.split(',').filter_map(|x| x.parse::<f64>().ok()).collect::<Vec<_>>()).unwrap_or_default();
                let (bx, by) = if base.len() == 2 { (base[0], base[1]) } else { (0.0, 0.0) };
                let measure_x = match self.direction { "X" => true, "Y" => false, _ => (ub - ua).abs() < (vb - va).abs() };
                let h = ctx.mem_num("Dim.TextHeight", 1.0);
                // leader: dog-leg from the point to the leader end
                let mid = if measure_x { cp.point_at(ua, va + (vb - va) * 0.6) } else { cp.point_at(ua + (ub - ua) * 0.6, va) };
                let knee = if measure_x { cp.point_at(ub, va + (vb - va) * 0.6 + (vb - va).signum() * h * 0.3) } else { cp.point_at(ua + (ub - ua) * 0.6 + (ub - ua).signum() * h * 0.3, vb) };
                let mut curves = Vec::new();
                if let Ok(l) = NurbsCurve::polyline(vec![a, mid, knee, b]) {
                    curves.push(l);
                }
                let value = if measure_x { ua - bx } else { va - by };
                let rot = if measure_x { std::f64::consts::FRAC_PI_2 } else { 0.0 };
                let origin = if measure_x { b + cp.x_axis * (h * 0.5) } else { b + cp.y_axis * (h * 0.3) };
                curves.extend(text_curves(&fmt_num(value), &cp, origin, h, rot));
                add_group(ctx, "DimOrdinate", curves)
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.first(), cursor) {
            (Some(a), Some(c)) => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[derive(Default)]
pub struct DimRotatedCmd {
    pts: Vec<DVec3>,
    angle: Option<f64>,
}

impl Command for DimRotatedCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        Prompt::number("Dimension line angle").default_num(ctx.mem_num("DimRotated.Angle", 30.0)).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.angle, input) {
            (None, Input::Number(a)) => {
                self.angle = Some(a);
                ctx.mem_set("DimRotated.Angle", fmt_num(a));
                Prompt::point("First dimension point").step()
            }
            (None, Input::Empty) => {
                self.angle = Some(ctx.mem_num("DimRotated.Angle", 30.0));
                Prompt::point("First dimension point").step()
            }
            (Some(a), Input::Point(p)) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("Second dimension point").step(),
                    2 => Prompt::point("Dimension line location").step(),
                    _ => {
                        let cp = ctx.cplane;
                        let (s, c) = a.to_radians().sin_cos();
                        // rotate the CPlane so its x axis follows the angle; the aligned dimension then measures along it
                        let plane = Plane { origin: cp.origin, x_axis: cp.x_axis * c + cp.y_axis * s, y_axis: -cp.x_axis * s + cp.y_axis * c, z_axis: cp.z_axis };
                        let (_ua, va, _) = plane.closest_parameters(self.pts[0]);
                        let (ub, _, _) = plane.closest_parameters(self.pts[1]);
                        // project the second point onto the rotated x direction through the first point's row
                        let pb = plane.point_at(ub, va);
                        let h = ctx.mem_num("Dim.TextHeight", 1.0);
                        let curves = super::annotate::dimension_curves(self.pts[0], pb, self.pts[2], &plane, false, h);
                        add_group(ctx, "DimRotated", curves)
                    }
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.last(), cursor) {
            (Some(a), Some(c)) => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

// ---- CurvatureGraph ------------------------------------------------------------------------------------------

pub struct CurvatureGraphCmd(pub bool);

impl Command for CurvatureGraphCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        if !self.0 {
            // CurvatureGraphOff: remove graph objects
            let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| o.attributes.groups.iter().any(|g| g.starts_with("CurvatureGraph"))).map(|o| o.id).collect();
            if !ids.is_empty() {
                ctx.doc.begin_undo("CurvatureGraphOff");
                for id in ids {
                    ctx.doc.remove(id);
                }
            }
            return Step::Done;
        }
        Prompt::objects("Select curves and surfaces for curvature graph display", ObjFilter::Curve, 1).opts([optv("Scale", fmt_num(ctx.mem_num("CurvatureGraph.Scale", 100.0))), optv("Density", "50")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Scale" => Prompt::number("Display scale").default_num(ctx.mem_num("CurvatureGraph.Scale", 100.0)).step(),
            Input::Option(_) => self.start(ctx),
            Input::Number(v) => {
                ctx.mem_set("CurvatureGraph.Scale", fmt_num(v.abs()));
                Prompt::objects("Select curves and surfaces for curvature graph display", ObjFilter::Curve, 1).step()
            }
            Input::Objects(ids) => {
                let scale = ctx.mem_num("CurvatureGraph.Scale", 100.0);
                let mut curves = Vec::new();
                for id in ids {
                    let Some(c) = ctx.doc.get(id).and_then(|o| o.geometry.as_curve().cloned()) else { continue };
                    let (t0, t1) = c.domain();
                    let n = 50;
                    let h = (t1 - t0) * 1e-4;
                    let mut tips = Vec::with_capacity(n + 1);
                    for i in 0..=n {
                        let t = t0 + (t1 - t0) * i as f64 / n as f64;
                        let d1 = c.derivative_at(t);
                        let d2 = (c.derivative_at((t + h).min(t1)) - c.derivative_at((t - h).max(t0))) / (2.0 * h);
                        let k = d1.cross(d2).length() / d1.length().powi(3).max(1e-18);
                        let nrm = (d2 - d1 * (d2.dot(d1) / d1.length_squared().max(1e-18))).normalize_or(DVec3::ZERO);
                        let p = c.point_at(t);
                        let tip = p - nrm * (k * scale * 0.01);
                        if let Ok(l) = NurbsCurve::line(p, tip) {
                            if (tip - p).length() > 1e-9 {
                                curves.push(l);
                            }
                        }
                        tips.push(tip);
                    }
                    if let Ok(env) = NurbsCurve::polyline(tips) {
                        curves.push(env);
                    }
                }
                add_group(ctx, "CurvatureGraph", curves)
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- Symmetry -------------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct SymmetryCmd {
    ids: Vec<ObjectId>,
    pts: Vec<DVec3>,
}

impl Command for SymmetryCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves or surfaces for symmetry", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::point("Start of symmetry plane").opts([optv("Smooth", "Yes")]).step()
            }
            Input::Point(p) => {
                self.pts.push(p);
                if self.pts.len() < 2 {
                    return Prompt::point("End of symmetry plane").step();
                }
                let (a, b) = (self.pts[0], self.pts[1]);
                let n = ctx.cplane.z_axis.cross(b - a).normalize_or(ctx.cplane.y_axis);
                let reflect = DAffine3::from_translation(a) * DAffine3::from_mat3(glam::DMat3::IDENTITY - glam::DMat3::from_cols(n * (2.0 * n.x), n * (2.0 * n.y), n * (2.0 * n.z))) * DAffine3::from_translation(-a);
                ctx.doc.begin_undo("Symmetry");
                let mut out = Vec::new();
                for id in self.ids.clone() {
                    let Some(o) = ctx.doc.get(id).cloned() else { continue };
                    let mut g = o.geometry.clone();
                    g.transform(&reflect);
                    match (&o.geometry, g) {
                        (Geometry::Curve(c), Geometry::Curve(m)) => {
                            let mut m2 = m.clone();
                            m2.reverse();
                            let joined = c.join_any(&m).or_else(|_| c.join_any(&m2)).or_else(|_| m.join_any(c)).or_else(|_| m2.join_any(c));
                            match joined {
                                Ok(j) => {
                                    ctx.doc.replace_geometry(id, Geometry::Curve(j));
                                }
                                Err(_) => out.push(Geometry::Curve(m)),
                            }
                        }
                        (_, mirrored) => out.push(mirrored),
                    }
                }
                for g in out {
                    let nid = ctx.doc.add(g);
                    ctx.doc.select(nid, true);
                }
                Step::Done
            }
            Input::Option(_) => Prompt::point("Start of symmetry plane").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.first(), cursor) {
            (Some(a), Some(c)) => vec![Preview::Line(*a, c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[allow(dead_code)]
fn unused() -> Option<Plane> {
    let _ = opt("x");
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tri_geom::Brep;

    #[test]
    fn half_space_cutter_splits_box() {
        let bx = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap().tessellate(1, 1);
        let mut doc = Document::new();
        let plane_srf = NurbsSurface::plane(&Plane::from_normal(DVec3::new(0.0, 0.0, 5.0), DVec3::Z), -5.0, 15.0, -5.0, 15.0).unwrap();
        let pid = doc.add(Geometry::Surface(plane_srf));
        let bb = bx.bounding_box();
        let cutter = cutter_mesh(&doc, pid, DVec3::new(5.0, 5.0, 5.0), &bb).unwrap();
        eprintln!("cutter bbox {:?} closed {}", cutter.bounding_box(), cutter.is_closed());
        let a = mesh_boolean(&bx, &cutter, BooleanOp::Intersection);
        let b = mesh_boolean(&bx, &cutter, BooleanOp::Difference);
        eprintln!("inside {} tris bbox {:?}; outside {} tris bbox {:?}", a.triangle_count(), a.bounding_box(), b.triangle_count(), b.bounding_box());
        assert!(a.triangle_count() > 0 && b.triangle_count() > 0);
    }
}
