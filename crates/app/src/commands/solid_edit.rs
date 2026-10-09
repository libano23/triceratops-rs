//! Solid editing on planar-faced polysurfaces: FilletEdge, ChamferEdge, BlendEdge,
//! plus BlendSrf, Patch and view camera commands (RotateView, Pan, Zoom factor).

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{Brep, Face, NurbsCurve, NurbsSurface, Plane};

use super::surface_edit::{face_under, faces_of};
use super::{fmt_num, opt, optv, AppRequest, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

/// 3D loops of a planar face (outer first) and its plane, or None for non-planar faces.
pub fn planar_loops(f: &Face) -> Option<(Plane, Vec<Vec<DVec3>>)> {
    let s = &f.surface;
    if s.degree_u != 1 || s.degree_v != 1 || s.count_u != 2 || s.count_v != 2 {
        return None;
    }
    let (p, n) = f.sample();
    let plane = Plane::from_normal(p, n);
    let loops = if f.loops.is_empty() {
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        vec![vec![s.point_at(u0, v0), s.point_at(u1, v0), s.point_at(u1, v1), s.point_at(u0, v1)]]
    } else {
        f.loops.iter().map(|l| l.iter().map(|uv| s.point_at(uv.x, uv.y)).collect()).collect()
    };
    Some((plane, loops))
}

fn seg_distance(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 < 1e-18 { 0.0 } else { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) };
    (a + ab * t - p).length()
}

fn centroid(poly: &[DVec3]) -> DVec3 {
    poly.iter().fold(DVec3::ZERO, |a, p| a + *p) / poly.len().max(1) as f64
}

struct EdgeInfo {
    p0: DVec3,
    p1: DVec3,
    face_a: usize,
    face_b: usize,
}

/// Find the planar edge nearest to `pick` shared by two planar faces.
fn find_edge(faces: &[Face], pick: DVec3, tol: f64) -> Option<EdgeInfo> {
    let loops: Vec<Option<(Plane, Vec<Vec<DVec3>>)>> = faces.iter().map(planar_loops).collect();
    let mut best: Option<(f64, DVec3, DVec3, usize)> = None;
    for (fi, l) in loops.iter().enumerate() {
        let Some((_, ls)) = l else { continue };
        for poly in ls {
            for i in 0..poly.len() {
                let a = poly[i];
                let b = poly[(i + 1) % poly.len()];
                let d = seg_distance(pick, a, b);
                if best.map_or(true, |bst| d < bst.0) {
                    best = Some((d, a, b, fi));
                }
            }
        }
    }
    let (_, a, b, fa) = best?;
    // find the other face with the same segment
    for (fi, l) in loops.iter().enumerate() {
        if fi == fa {
            continue;
        }
        let Some((_, ls)) = l else { continue };
        for poly in ls {
            for i in 0..poly.len() {
                let c = poly[i];
                let d = poly[(i + 1) % poly.len()];
                if ((c - a).length() < tol && (d - b).length() < tol) || ((c - b).length() < tol && (d - a).length() < tol) {
                    return Some(EdgeInfo { p0: a, p1: b, face_a: fa, face_b: fi });
                }
            }
        }
    }
    None
}

/// Replace the edge between two planar faces with a fillet (radius) or chamfer (distance).
pub fn round_edge(faces: &[Face], pick: DVec3, amount: f64, chamfer: bool) -> Result<Vec<Face>, String> {
    let bb = faces.iter().fold(tri_geom::BoundingBox::EMPTY, |b, f| b.union(&f.bounding_box()));
    let tol = bb.diagonal().max(1e-6) * 1e-6;
    let edge = find_edge(faces, pick, tol.max(1e-9) * 1000.0).ok_or("The selected edge must be shared by two planar faces.")?;
    let (plane_a, loops_a) = planar_loops(&faces[edge.face_a]).unwrap();
    let (plane_b, loops_b) = planar_loops(&faces[edge.face_b]).unwrap();
    let (na, nb) = (plane_a.z_axis, plane_b.z_axis);
    let e = (edge.p1 - edge.p0).normalize_or(DVec3::X);
    if na.cross(nb).length() < 1e-6 {
        return Err("The faces at the selected edge are coplanar.".into());
    }
    let inward = |n: DVec3, loops: &Vec<Vec<DVec3>>| {
        let d = e.cross(n).normalize_or(DVec3::Y);
        if d.dot(centroid(&loops[0]) - edge.p0) < 0.0 { -d } else { d }
    };
    let da = inward(na, &loops_a);
    let db = inward(nb, &loops_b);
    let r = amount.abs().max(1e-9);
    // tangent points (offsets from the edge) and arc centre offset
    let (ta, tb, c) = if chamfer {
        (da * r, db * r, DVec3::ZERO)
    } else {
        let alpha = -r / da.dot(nb);
        let beta = -r / db.dot(na);
        let c = da * alpha + db * beta;
        (c + na * r, c + nb * r, c)
    };
    let near = |p: DVec3, q: DVec3| (p - q).length() < tol * 1000.0;
    // arc polyline between ta and tb (relative to an edge point)
    let arc_rel: Vec<DVec3> = if chamfer {
        vec![ta, tb]
    } else {
        let x = (ta - c).normalize();
        let y_raw = tb - c;
        let ang = x.dot(y_raw.normalize()).clamp(-1.0, 1.0).acos();
        let axis = x.cross(y_raw).normalize_or(e);
        (0..=8)
            .map(|i| {
                let t = ang * i as f64 / 8.0;
                c + glam::DQuat::from_axis_angle(axis, t) * x * r
            })
            .collect()
    };
    let mut out: Vec<Face> = Vec::new();
    for (fi, f) in faces.iter().enumerate() {
        if fi == edge.face_a || fi == edge.face_b {
            let (plane, loops) = planar_loops(f).unwrap();
            let shift = if fi == edge.face_a { ta } else { tb };
            let new_loops: Vec<Vec<DVec3>> = loops
                .iter()
                .map(|l| l.iter().map(|p| if near(*p, edge.p0) || near(*p, edge.p1) { *p + shift } else { *p }).collect())
                .collect();
            out.push(Face::planar(&plane, new_loops).map_err(|e| e.to_string())?);
            continue;
        }
        match planar_loops(f) {
            Some((plane, loops)) if loops.iter().any(|l| l.iter().any(|p| near(*p, edge.p0) || near(*p, edge.p1))) => {
                let mut new_loops = Vec::new();
                for l in &loops {
                    let mut nl: Vec<DVec3> = Vec::new();
                    for i in 0..l.len() {
                        let p = l[i];
                        if near(p, edge.p0) || near(p, edge.p1) {
                            let prev = l[(i + l.len() - 1) % l.len()];
                            // neighbour on face A? then the arc starts at ta
                            let prev_on_a = (prev - p).dot(na).abs() < (prev - p).dot(nb).abs();
                            let pts: Vec<DVec3> = arc_rel.iter().map(|q| p + *q).collect();
                            if prev_on_a {
                                nl.extend(pts);
                            } else {
                                nl.extend(pts.into_iter().rev());
                            }
                        } else {
                            nl.push(p);
                        }
                    }
                    new_loops.push(nl);
                }
                out.push(Face::planar(&plane, new_loops).map_err(|e| e.to_string())?);
            }
            _ => out.push(f.clone()),
        }
    }
    // the new fillet / chamfer face
    let new_face = if chamfer {
        let s = NurbsSurface::from_grid(1, 1, vec![vec![edge.p0 + ta, edge.p0 + tb], vec![edge.p1 + ta, edge.p1 + tb]]).map_err(|e| e.to_string())?;
        let mut f = Face::untrimmed(s);
        let (_, n) = f.sample();
        if n.dot(da + db) > 0.0 {
            f.flip();
        }
        f
    } else {
        let x = (ta - c).normalize();
        let y_raw = (tb - c).normalize();
        let ang = x.dot(y_raw).clamp(-1.0, 1.0).acos();
        let axis = x.cross(y_raw).normalize_or(e);
        let plane = Plane { origin: edge.p0 + c, x_axis: x, y_axis: axis.cross(x).normalize(), z_axis: axis };
        let arc = NurbsCurve::arc(&plane, r, 0.0, ang).map_err(|e| e.to_string())?;
        let s = NurbsSurface::extrude(&arc, edge.p1 - edge.p0).map_err(|e| e.to_string())?;
        let mut f = Face::untrimmed(s);
        let (p, n) = f.sample();
        let centre_line = edge.p0 + c + e * (p - edge.p0 - c).dot(e);
        if n.dot(p - centre_line) < 0.0 {
            f.flip();
        }
        f
    };
    out.push(new_face);
    Ok(out)
}

pub struct EdgeRoundCmd {
    chamfer: bool,
    picks: Vec<(ObjectId, DVec3)>,
    amount: f64,
}

impl EdgeRoundCmd {
    pub fn new(chamfer: bool) -> Self {
        Self { chamfer, picks: Vec::new(), amount: 1.0 }
    }
    fn label(&self) -> &'static str {
        if self.chamfer {
            "ChamferEdge"
        } else {
            "FilletEdge"
        }
    }
    fn prompt(&self) -> Prompt {
        let key = if self.chamfer { "NextDistance" } else { "NextRadius" };
        Prompt::pick("Select edges to fillet. Press Enter when done", ObjFilter::Polysurface).opts([optv("ShowRadius", "Yes"), optv(key, fmt_num(self.amount)), optv("Preview", "Yes"), optv("Edit", "No")])
    }
}

impl Command for EdgeRoundCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let key = if self.chamfer { "ChamferEdge.Distance" } else { "FilletEdge.Radius" };
        self.amount = ctx.mem_num(key, 1.0);
        let mut p = self.prompt();
        if self.chamfer {
            p.msg = "Select edges to chamfer. Press Enter when done".into();
        }
        p.step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Pick(id, p) => {
                self.picks.push((id, p));
                self.prompt().step()
            }
            Input::Option(o) if o == "NextRadius" || o == "NextDistance" => Prompt::number(if self.chamfer { "Chamfer distance" } else { "Fillet radius" }).default_num(self.amount).step(),
            Input::Option(_) => self.prompt().step(),
            Input::Number(v) => {
                self.amount = v.abs().max(1e-6);
                let key = if self.chamfer { "ChamferEdge.Distance" } else { "FilletEdge.Radius" };
                ctx.mem_set(key, fmt_num(self.amount));
                self.prompt().step()
            }
            Input::Empty => {
                if self.picks.is_empty() {
                    return Step::Cancelled;
                }
                ctx.doc.begin_undo(self.label());
                let mut done = 0;
                let picks = self.picks.clone();
                for (id, p) in picks {
                    let Some(faces) = faces_of(ctx.doc, id) else { continue };
                    match round_edge(&faces, p, self.amount, self.chamfer) {
                        Ok(new_faces) => {
                            let brep = Brep::new(new_faces);
                            ctx.doc.replace_geometry(id, Geometry::Brep(brep));
                            done += 1;
                        }
                        Err(e) => ctx.log(format!("{}: {e}", self.label())),
                    }
                }
                if done == 0 {
                    ctx.log(format!("{} works on edges between planar faces in triceratops.", self.label()));
                    return Step::Cancelled;
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, _cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let mut out = Vec::new();
        for (id, p) in &self.picks {
            if let Some(faces) = faces_of(doc, *id) {
                if let Ok(new_faces) = round_edge(&faces, *p, self.amount, self.chamfer) {
                    if let Some(f) = new_faces.last() {
                        out.push(Preview::Surface(f.surface.clone()));
                    }
                }
            }
        }
        out
    }
}

// ---- BlendSrf ---------------------------------------------------------------------------------

/// Edge of a face nearest to a pick: sampled points and outward in-surface directions.
fn edge_samples(face: &Face, pick: DVec3, n: usize) -> Option<(Vec<DVec3>, Vec<DVec3>)> {
    let tol = face.bounding_box().diagonal().max(1e-6) * 1e-3;
    let curves = face.edge_curves(tol);
    let (p_c, _) = face.sample();
    let best = curves.into_iter().min_by(|a, b| {
        let da = (a.point_at(a.closest_parameter(pick)) - pick).length();
        let db = (b.point_at(b.closest_parameter(pick)) - pick).length();
        da.partial_cmp(&db).unwrap()
    })?;
    let (t0, t1) = best.domain();
    let mut pts = Vec::with_capacity(n + 1);
    let mut dirs = Vec::with_capacity(n + 1);
    for i in 0..=n {
        let t = t0 + (t1 - t0) * i as f64 / n as f64;
        let p = best.point_at(t);
        let (u, v) = face.surface.closest_uv(p);
        let nrm = face.surface.normal_at(u, v);
        let tan = best.tangent_at(t);
        let mut d = nrm.cross(tan).normalize_or(DVec3::X);
        if d.dot(p - p_c) < 0.0 {
            d = -d;
        }
        pts.push(p);
        dirs.push(d);
    }
    Some((pts, dirs))
}

pub fn blend_surface(a: (&[DVec3], &[DVec3]), b: (&[DVec3], &[DVec3]), bulge: f64) -> Option<NurbsSurface> {
    let n = a.0.len();
    if n < 2 || b.0.len() != n {
        return None;
    }
    let m = 8;
    let mut grid = Vec::with_capacity(n);
    for i in 0..n {
        let p0 = a.0[i];
        let p1 = b.0[i];
        let l = (p1 - p0).length();
        let m0 = a.1[i] * (l * bulge);
        let m1 = -b.1[i] * (l * bulge);
        let mut row = Vec::with_capacity(m + 1);
        for j in 0..=m {
            let s = j as f64 / m as f64;
            let (s2, s3) = (s * s, s * s * s);
            let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
            let h10 = s3 - 2.0 * s2 + s;
            let h01 = -2.0 * s3 + 3.0 * s2;
            let h11 = s3 - s2;
            row.push(p0 * h00 + m0 * h10 + p1 * h01 + m1 * h11);
        }
        grid.push(row);
    }
    NurbsSurface::interpolate_grid(&grid, 3, 3).ok()
}

#[derive(Default)]
pub struct BlendSrfCmd {
    first: Option<(ObjectId, DVec3)>,
    second: Option<(ObjectId, DVec3)>,
    bulge: f64,
}

impl BlendSrfCmd {
    fn samples(&self, doc: &Document, pick: (ObjectId, DVec3)) -> Option<(Vec<DVec3>, Vec<DVec3>)> {
        let faces = faces_of(doc, pick.0)?;
        let fi = face_under(&faces, pick.1);
        edge_samples(&faces[fi], pick.1, 16)
    }
    fn build(&self, doc: &Document) -> Option<NurbsSurface> {
        let a = self.samples(doc, self.first?)?;
        let mut b = self.samples(doc, self.second?)?;
        // align directions: reverse b if its start is farther from a's start than its end
        if (b.0[0] - a.0[0]).length() > (b.0[b.0.len() - 1] - a.0[0]).length() {
            b.0.reverse();
            b.1.reverse();
        }
        blend_surface((&a.0, &a.1), (&b.0, &b.1), if self.bulge > 0.0 { self.bulge } else { 1.0 })
    }
}

impl Command for BlendSrfCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.bulge = ctx.mem_num("BlendSrf.Bulge", 1.0);
        Prompt::pick("Select segment for first edge", ObjFilter::Surface).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, self.second, input) {
            (None, _, Input::Pick(id, p)) => {
                self.first = Some((id, p));
                Prompt::pick("Select segment for second edge", ObjFilter::Surface).step()
            }
            (Some(_), None, Input::Pick(id, p)) => {
                self.second = Some((id, p));
                Prompt::options("Press Enter to accept blend").opts([optv("Bulge", fmt_num(self.bulge)), optv("Continuity", "Tangency"), optv("Join", "No")]).step()
            }
            (_, _, Input::Option(o)) if o == "Bulge" => Prompt::number("Bulge").default_num(self.bulge).step(),
            (_, _, Input::Option(_)) => Prompt::options("Press Enter to accept blend").opts([optv("Bulge", fmt_num(self.bulge))]).step(),
            (_, _, Input::Number(v)) => {
                self.bulge = v.abs().max(0.05);
                ctx.mem_set("BlendSrf.Bulge", fmt_num(self.bulge));
                Prompt::options("Press Enter to accept blend").opts([optv("Bulge", fmt_num(self.bulge))]).step()
            }
            (Some(_), Some(_), Input::Empty) => match self.build(ctx.doc) {
                Some(s) => {
                    ctx.doc.begin_undo("BlendSrf");
                    ctx.doc.clear_selection();
                    let id = ctx.doc.add(Geometry::Surface(s));
                    ctx.doc.select(id, true);
                    Step::Done
                }
                None => {
                    ctx.log("Unable to create blend surface.");
                    Step::Cancelled
                }
            },
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, _cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        self.build(doc).map(|s| vec![Preview::Surface(s)]).unwrap_or_default()
    }
}

// ---- Patch --------------------------------------------------------------------------------------

#[derive(Default)]
pub struct PatchCmd {
    spans: usize,
}

impl Command for PatchCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.spans = ctx.mem_num("Patch.Spans", 10.0) as usize;
        Prompt::objects("Select curves, points, point clouds, and meshes to fit surface through", ObjFilter::Any, 1).opts([optv("Spans", self.spans), optv("Stiffness", "1")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Spans" => Prompt::number("Surface spans").default_num(self.spans as f64).step(),
            Input::Option(_) => self.start(ctx),
            Input::Number(v) => {
                self.spans = (v.round() as usize).clamp(2, 40);
                ctx.mem_set("Patch.Spans", self.spans);
                Prompt::objects("Select curves, points, point clouds, and meshes to fit surface through", ObjFilter::Any, 1).opts([optv("Spans", self.spans)]).step()
            }
            Input::Objects(ids) => {
                let mut samples: Vec<DVec3> = Vec::new();
                let mut plane: Option<Plane> = None;
                for id in &ids {
                    let Some(o) = ctx.doc.get(*id) else { continue };
                    match &o.geometry {
                        Geometry::Point(p) => samples.push(*p),
                        Geometry::Curve(c) => {
                            if plane.is_none() {
                                plane = tri_geom::curve_plane(c);
                            }
                            samples.extend(c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3));
                        }
                        Geometry::Mesh(m) => samples.extend(m.vertices.iter().copied()),
                        _ => {}
                    }
                }
                if samples.len() < 3 {
                    ctx.log("Patch needs at least three points.");
                    return Step::Cancelled;
                }
                let plane = plane.unwrap_or(ctx.cplane);
                let plane = Plane { origin: centroid(&samples), ..plane };
                let uvw: Vec<(f64, f64, f64)> = samples.iter().map(|p| plane.closest_parameters(*p)).collect();
                let (mut umin, mut umax, mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
                for (u, v, _) in &uvw {
                    umin = umin.min(*u);
                    umax = umax.max(*u);
                    vmin = vmin.min(*v);
                    vmax = vmax.max(*v);
                }
                let n = self.spans.max(2);
                let mut grid = Vec::with_capacity(n + 1);
                for i in 0..=n {
                    let u = umin + (umax - umin) * i as f64 / n as f64;
                    let mut row = Vec::with_capacity(n + 1);
                    for j in 0..=n {
                        let v = vmin + (vmax - vmin) * j as f64 / n as f64;
                        // inverse-distance weighted height
                        let mut num = 0.0;
                        let mut den = 0.0;
                        for (su, sv, sw) in &uvw {
                            let d2 = (su - u) * (su - u) + (sv - v) * (sv - v);
                            let w = 1.0 / (d2 + 1e-12);
                            num += w * sw;
                            den += w;
                        }
                        row.push(plane.point_at_3(u, v, num / den));
                    }
                    grid.push(row);
                }
                match NurbsSurface::interpolate_grid(&grid, 3, 3) {
                    Ok(s) => {
                        ctx.doc.begin_undo("Patch");
                        ctx.doc.clear_selection();
                        let id = ctx.doc.add(Geometry::Surface(s));
                        ctx.doc.select(id, true);
                        Step::Done
                    }
                    Err(e) => {
                        ctx.log(format!("Patch failed: {e}"));
                        Step::Cancelled
                    }
                }
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- view camera commands ------------------------------------------------------------------------

pub struct ViewOpCmd {
    kind: &'static str,
}

impl ViewOpCmd {
    pub fn new(kind: &'static str) -> Self {
        Self { kind }
    }
}

impl Command for ViewOpCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.kind {
            "RotateView" => Prompt::options("Rotate view").opts([opt("Left"), opt("Right"), opt("Up"), opt("Down"), optv("Angle", fmt_num(ctx.mem_num("RotateView.Angle", 15.0)))]).step(),
            "Pan" => Prompt::options("Pan view").opts([opt("Left"), opt("Right"), opt("Up"), opt("Down"), optv("Distance", fmt_num(ctx.mem_num("Pan.Distance", 0.25)))]).step(),
            _ => Prompt::options("Tilt view").opts([opt("Left"), opt("Right")]).step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Angle" => Prompt::number("Rotation angle").default_num(ctx.mem_num("RotateView.Angle", 15.0)).step(),
            Input::Option(o) if o == "Distance" => Prompt::number("Pan distance (fraction of view)").default_num(ctx.mem_num("Pan.Distance", 0.25)).step(),
            Input::Number(v) => {
                if self.kind == "RotateView" {
                    ctx.mem_set("RotateView.Angle", fmt_num(v));
                } else {
                    ctx.mem_set("Pan.Distance", fmt_num(v));
                }
                self.start(ctx)
            }
            Input::Option(dir) => {
                let amount = if self.kind == "RotateView" { ctx.mem_num("RotateView.Angle", 15.0) } else { ctx.mem_num("Pan.Distance", 0.25) };
                let (dx, dy) = match dir.as_str() {
                    "Left" => (-1.0, 0.0),
                    "Right" => (1.0, 0.0),
                    "Up" => (0.0, 1.0),
                    _ => (0.0, -1.0),
                };
                ctx.requests.push(AppRequest::CameraOp { op: self.kind.to_string(), dx: dx * amount, dy: dy * amount });
                self.start(ctx)
            }
            Input::Empty => Step::Done,
            _ => Step::Cancelled,
        }
    }
}
