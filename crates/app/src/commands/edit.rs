//! Editing and transformation commands with Triceratops prompts and options.

use glam::{DAffine3, DMat3, DQuat, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{Brep, Face, NurbsCurve, Plane};

use super::{opt, optv, yesno, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

/// Preview of objects under a transform: curves exactly, surfaces by edges and isocurves.
pub fn preview_transformed(doc: &Document, ids: &[ObjectId], xf: &DAffine3) -> Vec<Preview> {
    let mut out = Vec::new();
    for id in ids {
        let Some(o) = doc.get(*id) else { continue };
        match &o.geometry {
            Geometry::Curve(c) => out.push(Preview::Curve(c.transformed(xf))),
            Geometry::Point(p) => out.push(Preview::Point(xf.transform_point3(*p))),
            Geometry::Surface(s) => {
                let mut s2 = s.clone();
                s2.transform(xf);
                out.push(Preview::Surface(s2));
            }
            Geometry::Brep(b) => {
                let mut b2 = b.clone();
                b2.transform(xf);
                out.push(Preview::Brep(b2));
            }
            Geometry::Mesh(_) | Geometry::Instance { .. } => {
                let bb = doc.object_bbox(o);
                let c: Vec<DVec3> = bb.corners().iter().map(|p| xf.transform_point3(*p)).collect();
                for (a, b) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
                    out.push(Preview::Line(c[a], c[b]));
                }
            }
        }
    }
    out
}

fn apply_transform(ctx: &mut CmdCtx, label: &str, ids: &[ObjectId], xf: &DAffine3, copy: bool) {
    ctx.doc.begin_undo(label);
    if copy {
        let mut new_ids = Vec::new();
        for id in ids {
            if let Some(o) = ctx.doc.get(*id).cloned() {
                let mut g = o.geometry;
                g.transform(xf);
                new_ids.push(ctx.doc.add_with(g, o.attributes));
            }
        }
        ctx.doc.clear_selection();
        for id in new_ids {
            ctx.doc.select(id, true);
        }
    } else {
        ctx.doc.transform_objects(ids, xf);
    }
}

fn bbox_center(doc: &Document, ids: &[ObjectId]) -> DVec3 {
    ids.iter().filter_map(|id| doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&o.geometry.bounding_box())).center()
}

pub struct DeleteCmd;

impl Command for DeleteCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to delete", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        if let Input::Objects(ids) = input {
            ctx.doc.begin_undo("Delete");
            for id in &ids {
                ctx.doc.remove(*id);
            }
            Step::Done
        } else {
            Step::Cancelled
        }
    }
}

pub struct UndoCmd(pub bool);

impl Command for UndoCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let r = if self.0 { ctx.doc.redo() } else { ctx.doc.undo() };
        match r {
            Some(label) => ctx.log(format!("{} {label}", if self.0 { "Redoing" } else { "Undoing" })),
            None => ctx.log(if self.0 { "Nothing to redo." } else { "Nothing to undo." }),
        }
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

#[derive(Default)]
pub struct MoveCmd {
    ids: Vec<ObjectId>,
    from: Option<DVec3>,
    vertical: bool,
}

impl Command for MoveCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to move", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.from, input) {
            (_, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point("Point to move from").opts([optv("Vertical", yesno(self.vertical))]).step()
            }
            (None, Input::Option(_)) => {
                self.vertical = !self.vertical;
                Prompt::point("Point to move from").opts([optv("Vertical", yesno(self.vertical))]).step()
            }
            (None, Input::Point(p)) => {
                self.from = Some(p);
                Prompt::point_or_number("Point to move to").step()
            }
            (None, Input::Empty) => {
                self.from = Some(bbox_center(ctx.doc, &self.ids));
                Prompt::point_or_number("Point to move to").step()
            }
            (Some(from), Input::Point(to)) => {
                let d = if self.vertical { ctx.cplane.z_axis * (to - *from).dot(ctx.cplane.z_axis) } else { to - *from };
                apply_transform(ctx, "Move", &self.ids, &DAffine3::from_translation(d), false);
                Step::Done
            }
            (Some(_), Input::Number(d)) if self.vertical => {
                apply_transform(ctx, "Move", &self.ids, &DAffine3::from_translation(ctx.cplane.z_axis * d), false);
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.from, cursor) {
            (Some(f), Some(c)) => {
                let d = if self.vertical { (c - f).project_onto(DVec3::Z) } else { c - f };
                let mut v = preview_transformed(doc, &self.ids, &DAffine3::from_translation(d));
                v.push(Preview::Line(f, f + d));
                v
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.from
    }
}

#[derive(Default)]
pub struct CopyCmd {
    ids: Vec<ObjectId>,
    from: Option<DVec3>,
    vertical: bool,
    from_last_point: bool,
    use_last_distance: bool,
    use_last_direction: bool,
    last_delta: Option<DVec3>,
    copies: usize,
}

impl CopyCmd {
    fn to_prompt(&self) -> Prompt {
        Prompt::point("Point to copy to").opts([
            optv("FromLastPoint", yesno(self.from_last_point)),
            optv("UseLastDistance", yesno(self.use_last_distance)),
            optv("UseLastDirection", yesno(self.use_last_direction)),
        ])
    }
    fn delta(&self, from: DVec3, to: DVec3, cplane: &Plane) -> DVec3 {
        let mut d = to - from;
        if self.vertical {
            d = cplane.z_axis * d.dot(cplane.z_axis);
        }
        if let Some(last) = self.last_delta {
            if self.use_last_direction && last.length() > 1e-12 {
                let dir = last.normalize();
                d = dir * d.dot(dir).max(0.0);
            }
            if self.use_last_distance && d.length() > 1e-12 {
                d = d.normalize() * last.length();
            }
        }
        d
    }
}

impl Command for CopyCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to copy", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.from, input) {
            (_, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point("Point to copy from").opts([optv("Vertical", yesno(self.vertical)), opt("InPlace")]).step()
            }
            (None, Input::Option(o)) => {
                if o == "InPlace" {
                    apply_transform(ctx, "Copy", &self.ids, &DAffine3::IDENTITY, true);
                    return Step::Done;
                }
                self.vertical = !self.vertical;
                Prompt::point("Point to copy from").opts([optv("Vertical", yesno(self.vertical)), opt("InPlace")]).step()
            }
            (None, Input::Point(p)) => {
                self.from = Some(p);
                self.to_prompt().step()
            }
            (None, Input::Empty) => {
                self.from = Some(bbox_center(ctx.doc, &self.ids));
                self.to_prompt().step()
            }
            (Some(_), Input::Option(o)) => {
                match o.as_str() {
                    "FromLastPoint" => self.from_last_point = !self.from_last_point,
                    "UseLastDistance" => self.use_last_distance = !self.use_last_distance,
                    "UseLastDirection" => self.use_last_direction = !self.use_last_direction,
                    _ => {}
                }
                self.to_prompt().step()
            }
            (Some(from), Input::Point(to)) => {
                let d = self.delta(*from, to, &ctx.cplane);
                let original_ids = self.ids.clone();
                apply_transform(ctx, "Copy", &original_ids, &DAffine3::from_translation(d), true);
                self.copies += 1;
                self.last_delta = Some(d);
                if self.from_last_point {
                    self.from = Some(*from + d);
                }
                // keep the originals selected for further copies, like Triceratops
                ctx.doc.clear_selection();
                for id in &original_ids {
                    ctx.doc.select(*id, true);
                }
                self.to_prompt().step()
            }
            (Some(_), Input::Empty) => Step::Done,
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.from, cursor) {
            (Some(f), Some(c)) => {
                let d = self.delta(f, c, &Plane::world_xy());
                let mut v = preview_transformed(doc, &self.ids, &DAffine3::from_translation(d));
                v.push(Preview::Line(f, f + d));
                v
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.from
    }
}

#[derive(Default)]
pub struct RotateCmd {
    ids: Vec<ObjectId>,
    center: Option<DVec3>,
    reference: Option<DVec3>,
    plane: Plane,
    copy: bool,
}

impl RotateCmd {
    fn xf(&self, center: DVec3, angle: f64) -> DAffine3 {
        let q = DQuat::from_axis_angle(self.plane.z_axis, angle);
        DAffine3::from_translation(center) * DAffine3::from_quat(q) * DAffine3::from_translation(-center)
    }
    fn angle_between(&self, center: DVec3, a: DVec3, b: DVec3) -> f64 {
        let pa = a - center;
        let pb = b - center;
        let (ua, va) = (pa.dot(self.plane.x_axis), pa.dot(self.plane.y_axis));
        let (ub, vb) = (pb.dot(self.plane.x_axis), pb.dot(self.plane.y_axis));
        vb.atan2(ub) - va.atan2(ua)
    }
    fn prompt(&self) -> Prompt {
        let copy = optv("Copy", yesno(self.copy));
        match (self.center, self.reference) {
            (None, _) => Prompt::point("Center of rotation").opts([copy, opt("UseLastCenter")]),
            (Some(_), None) => Prompt::point_or_number("Angle or first reference point").opts([copy]),
            (Some(_), Some(_)) => Prompt::point("Second reference point").opts([copy]),
        }
    }
}

impl Command for RotateCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.copy = ctx.mem_bool("Rotate.Copy", false);
        Prompt::objects("Select objects to rotate", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.center, self.reference, input) {
            (_, _, Input::Objects(ids)) => {
                self.ids = ids;
                self.prompt().step()
            }
            (_, _, Input::Option(o)) => {
                match o.as_str() {
                    "Copy" => {
                        self.copy = !self.copy;
                        ctx.mem_set("Rotate.Copy", yesno(self.copy));
                    }
                    "UseLastCenter" => {
                        if let Some(c) = ctx.memory.get("Rotate.Center").and_then(|s| {
                            let v: Vec<f64> = s.split(',').filter_map(|x| x.parse().ok()).collect();
                            (v.len() == 3).then(|| DVec3::new(v[0], v[1], v[2]))
                        }) {
                            self.center = Some(c);
                        }
                    }
                    _ => {}
                }
                self.prompt().step()
            }
            (None, _, Input::Point(p)) => {
                self.center = Some(p);
                ctx.mem_set("Rotate.Center", format!("{},{},{}", p.x, p.y, p.z));
                self.prompt().step()
            }
            (Some(c), None, Input::Number(deg)) => {
                let xf = self.xf(c, deg.to_radians());
                apply_transform(ctx, "Rotate", &self.ids, &xf, self.copy);
                Step::Done
            }
            (Some(_), None, Input::Point(p)) => {
                self.reference = Some(p);
                self.prompt().step()
            }
            (Some(c), Some(r), Input::Point(p)) => {
                let xf = self.xf(c, self.angle_between(c, r, p));
                apply_transform(ctx, "Rotate", &self.ids, &xf, self.copy);
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.center, self.reference, cursor) {
            (Some(c), None, Some(p)) => vec![Preview::Line(c, p)],
            (Some(c), Some(r), Some(p)) => {
                let mut v = preview_transformed(doc, &self.ids, &self.xf(c, self.angle_between(c, r, p)));
                v.push(Preview::Line(c, r));
                v.push(Preview::Line(c, p));
                v
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center
    }
}

pub struct ScaleCmd {
    dims: u8,
    ids: Vec<ObjectId>,
    origin: Option<DVec3>,
    reference: Option<DVec3>,
    copy: bool,
    rigid: bool,
    plane: Plane,
}

impl ScaleCmd {
    pub fn new(dims: u8) -> Self {
        Self { dims, ids: Vec::new(), origin: None, reference: None, copy: false, rigid: false, plane: Plane::world_xy() }
    }
    fn label(&self) -> &'static str {
        match self.dims {
            1 => "Scale1D",
            2 => "Scale2D",
            _ => "Scale",
        }
    }
    fn xf(&self, origin: DVec3, f: f64, dir: Option<DVec3>) -> DAffine3 {
        let m = match self.dims {
            3 => DMat3::from_diagonal(DVec3::splat(f)),
            2 => {
                let (x, y, z) = (self.plane.x_axis, self.plane.y_axis, self.plane.z_axis);
                DMat3::from_cols(x, y, z) * DMat3::from_diagonal(DVec3::new(f, f, 1.0)) * DMat3::from_cols(x, y, z).transpose()
            }
            _ => {
                let d = dir.unwrap_or(self.plane.x_axis).normalize_or(DVec3::X);
                DMat3::IDENTITY + DMat3::from_cols(d * (d.x * (f - 1.0)), d * (d.y * (f - 1.0)), d * (d.z * (f - 1.0)))
            }
        };
        DAffine3::from_translation(origin) * DAffine3::from_mat3(m) * DAffine3::from_translation(-origin)
    }
    fn prompt(&self) -> Prompt {
        let opts = [optv("Copy", yesno(self.copy)), optv("Rigid", yesno(self.rigid))];
        match (self.origin, self.reference) {
            (None, _) => Prompt::point("Origin point").opts(opts),
            (Some(_), None) => Prompt::point_or_number("Scale factor or first reference point").default_num(1.0).opts(opts),
            (Some(_), Some(_)) => Prompt::point_or_number("Second reference point").opts(opts),
        }
    }
    fn factor(&self, o: DVec3, r: DVec3, p: DVec3) -> Option<(f64, DVec3)> {
        let d0 = r - o;
        let l0 = d0.length();
        if l0 < 1e-12 {
            return None;
        }
        match self.dims {
            1 => {
                let dir = d0 / l0;
                Some(((p - o).dot(dir) / l0, dir))
            }
            _ => Some(((p - o).length() / l0, d0 / l0)),
        }
    }
}

impl Command for ScaleCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::objects(format!("Select objects to {}", if self.dims == 3 { "scale".into() } else { format!("scale ({}D)", self.dims) }), ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.origin, self.reference, input) {
            (_, _, Input::Objects(ids)) => {
                self.ids = ids;
                self.prompt().step()
            }
            (_, _, Input::Option(o)) => {
                match o.as_str() {
                    "Copy" => self.copy = !self.copy,
                    "Rigid" => self.rigid = !self.rigid,
                    _ => {}
                }
                self.prompt().step()
            }
            (None, _, Input::Point(p)) => {
                self.origin = Some(p);
                self.prompt().step()
            }
            (None, _, Input::Empty) => {
                self.origin = Some(bbox_center(ctx.doc, &self.ids));
                self.prompt().step()
            }
            (Some(o), None, Input::Number(f)) if f.abs() > 1e-12 || self.dims == 1 => {
                let label = self.label();
                apply_transform(ctx, label, &self.ids, &self.xf(o, f, None), self.copy);
                Step::Done
            }
            (Some(_), None, Input::Point(p)) => {
                self.reference = Some(p);
                self.prompt().step()
            }
            (Some(o), Some(r), Input::Number(f)) => {
                let dir = (r - o).normalize_or(DVec3::X);
                let label = self.label();
                apply_transform(ctx, label, &self.ids, &self.xf(o, f, Some(dir)), self.copy);
                Step::Done
            }
            (Some(o), Some(r), Input::Point(p)) => {
                let Some((f, dir)) = self.factor(o, r, p) else { return Step::Cancelled };
                let label = self.label();
                apply_transform(ctx, label, &self.ids, &self.xf(o, f, Some(dir)), self.copy);
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.origin, self.reference, cursor) {
            (Some(o), None, Some(p)) => vec![Preview::Line(o, p)],
            (Some(o), Some(r), Some(p)) => {
                let Some((f, dir)) = self.factor(o, r, p) else { return vec![] };
                let mut v = preview_transformed(doc, &self.ids, &self.xf(o, f, Some(dir)));
                v.push(Preview::Line(o, p));
                v
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.origin
    }
}

#[derive(Default)]
pub struct MirrorCmd {
    ids: Vec<ObjectId>,
    start: Option<DVec3>,
    plane: Plane,
    copy: bool,
    three_point: Vec<DVec3>,
    use_three: bool,
}

pub fn mirror_xf(a: DVec3, n: DVec3) -> Option<DAffine3> {
    let n = n.normalize_or(DVec3::ZERO);
    if n == DVec3::ZERO {
        return None;
    }
    let m = DMat3::IDENTITY - DMat3::from_cols(n * (2.0 * n.x), n * (2.0 * n.y), n * (2.0 * n.z));
    let t = n * (2.0 * a.dot(n));
    Some(DAffine3::from_mat3_translation(m, t))
}

impl MirrorCmd {
    fn line_xf(&self, a: DVec3, b: DVec3) -> Option<DAffine3> {
        let dir = (b - a).normalize_or(DVec3::ZERO);
        if dir == DVec3::ZERO {
            return None;
        }
        mirror_xf(a, dir.cross(self.plane.z_axis))
    }
    fn prompt(&self) -> Prompt {
        let copy = optv("Copy", yesno(self.copy));
        if self.use_three {
            return match self.three_point.len() {
                0 => Prompt::point("First point on mirror plane").opts([copy]),
                1 => Prompt::point("Second point on mirror plane").opts([copy]),
                _ => Prompt::point("Third point on mirror plane").opts([copy]),
            };
        }
        match self.start {
            None => Prompt::point("Start of mirror plane").opts([opt("3Point"), copy, opt("XAxis"), opt("YAxis"), opt("ZAxis"), opt("Object")]),
            Some(_) => Prompt::point("End of mirror plane").opts([copy]),
        }
    }
    fn apply(&self, ctx: &mut CmdCtx, xf: Option<DAffine3>) -> Step {
        match xf {
            Some(xf) => {
                apply_transform(ctx, "Mirror", &self.ids, &xf, self.copy);
                Step::Done
            }
            None => {
                ctx.log("Mirror plane is degenerate.");
                Step::Cancelled
            }
        }
    }
}

impl Command for MirrorCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.copy = ctx.mem_bool("Mirror.Copy", true);
        Prompt::objects("Select objects to mirror", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.prompt().step()
            }
            Input::Option(o) => match o.as_str() {
                "Copy" => {
                    self.copy = !self.copy;
                    ctx.mem_set("Mirror.Copy", yesno(self.copy));
                    self.prompt().step()
                }
                "3Point" => {
                    self.use_three = true;
                    self.prompt().step()
                }
                "XAxis" => self.apply(ctx, mirror_xf(self.plane.origin, self.plane.y_axis)),
                "YAxis" => self.apply(ctx, mirror_xf(self.plane.origin, self.plane.x_axis)),
                "ZAxis" => self.apply(ctx, mirror_xf(self.plane.origin, self.plane.z_axis)),
                _ => {
                    ctx.log("The Object option is not implemented in triceratops yet.");
                    self.prompt().step()
                }
            },
            Input::Point(p) if self.use_three => {
                self.three_point.push(p);
                if self.three_point.len() < 3 {
                    return self.prompt().step();
                }
                let t = &self.three_point;
                let n = (t[1] - t[0]).cross(t[2] - t[0]);
                self.apply(ctx, mirror_xf(t[0], n))
            }
            Input::Point(p) => match self.start {
                None => {
                    self.start = Some(p);
                    self.prompt().step()
                }
                Some(a) => self.apply(ctx, self.line_xf(a, p)),
            },
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let Some(c) = cursor else { return vec![] };
        if self.use_three {
            return match self.three_point.len() {
                1 => vec![Preview::Line(self.three_point[0], c)],
                2 => {
                    let t = &self.three_point;
                    let mut v = vec![Preview::Line(t[0], t[1]), Preview::Line(t[1], c), Preview::Line(c, t[0])];
                    if let Some(xf) = mirror_xf(t[0], (t[1] - t[0]).cross(c - t[0])) {
                        v.extend(preview_transformed(doc, &self.ids, &xf));
                    }
                    v
                }
                _ => vec![],
            };
        }
        match self.start {
            Some(a) => {
                let mut v = vec![Preview::Line(a, c)];
                if let Some(xf) = self.line_xf(a, c) {
                    v.extend(preview_transformed(doc, &self.ids, &xf));
                }
                v
            }
            None => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.start.or(self.three_point.last().copied())
    }
}

pub struct JoinCmd;

impl Command for JoinCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select object to join", ObjFilter::Any, 2).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        let mut curves: Vec<(ObjectId, NurbsCurve)> = Vec::new();
        let mut faces: Vec<(ObjectId, Vec<Face>)> = Vec::new();
        let mut meshes: Vec<(ObjectId, tri_geom::Mesh)> = Vec::new();
        for id in &ids {
            match ctx.doc.get(*id).map(|o| &o.geometry) {
                Some(Geometry::Curve(c)) => curves.push((*id, c.clone())),
                Some(Geometry::Surface(s)) => faces.push((*id, vec![Face::untrimmed(s.clone())])),
                Some(Geometry::Brep(b)) => faces.push((*id, b.faces.clone())),
                Some(Geometry::Mesh(m)) => meshes.push((*id, m.clone())),
                _ => {}
            }
        }
        ctx.doc.begin_undo("Join");
        let mut joined = 0;
        // curves: greedy chaining
        let mut remaining = curves;
        while let Some((first_id, mut chain)) = remaining.pop() {
            let mut used = vec![first_id];
            let mut progress = true;
            while progress {
                progress = false;
                let mut i = 0;
                while i < remaining.len() {
                    let (id, c) = &remaining[i];
                    if let Ok(j) = chain.join(c) {
                        chain = j;
                        used.push(*id);
                        remaining.remove(i);
                        progress = true;
                    } else if let Ok(j) = c.join(&chain) {
                        chain = j;
                        used.push(*id);
                        remaining.remove(i);
                        progress = true;
                    } else {
                        i += 1;
                    }
                }
            }
            if used.len() > 1 {
                let attrs = ctx.doc.get(first_id).map(|o| o.attributes.clone()).unwrap_or_default();
                for id in &used {
                    ctx.doc.remove(*id);
                }
                let nid = ctx.doc.add_with(Geometry::Curve(chain), attrs);
                ctx.doc.select(nid, true);
                joined += used.len();
            }
        }
        if faces.len() > 1 {
            let attrs = ctx.doc.get(faces[0].0).map(|o| o.attributes.clone()).unwrap_or_default();
            let mut all = Vec::new();
            for (id, f) in &faces {
                ctx.doc.remove(*id);
                all.extend(f.iter().cloned());
            }
            let n = faces.len();
            let brep = Brep::new(all);
            if brep.closed {
                ctx.log("Joined into a closed solid polysurface.");
            }
            let nid = ctx.doc.add_with(Geometry::Brep(brep), attrs);
            ctx.doc.select(nid, true);
            joined += n;
        }
        // meshes join into one mesh, as in the reference application
        if meshes.len() > 1 {
            let attrs = ctx.doc.get(meshes[0].0).map(|o| o.attributes.clone()).unwrap_or_default();
            let mut all = tri_geom::Mesh::new();
            for (id, m) in &meshes {
                ctx.doc.remove(*id);
                all.append(m);
            }
            let nid = ctx.doc.add_with(Geometry::Mesh(all), attrs);
            ctx.doc.select(nid, true);
            joined += meshes.len();
        }
        if joined == 0 {
            ctx.log("Unable to join the selected objects (curves must meet end to end and share the same degree).");
        } else {
            ctx.log(format!("{joined} objects joined."));
        }
        Step::Done
    }
}

pub struct ExplodeCmd;

/// The segments of a curve: split at knots with full multiplicity (polyline vertices, arc
/// segments, joined pieces). A smooth curve comes back whole.
pub fn explode_curve(c: &NurbsCurve) -> Vec<NurbsCurve> {
    let p = c.degree;
    let mut interior: Vec<f64> = Vec::new();
    let ks = &c.knots[p + 1..c.knots.len() - p - 1];
    let mut i = 0;
    while i < ks.len() {
        let k = ks[i];
        let mut m = 1;
        while i + m < ks.len() && (ks[i + m] - k).abs() < 1e-12 {
            m += 1;
        }
        if m >= p {
            interior.push(k);
        }
        i += m;
    }
    let mut pieces = Vec::new();
    let mut rest = c.clone();
    for k in interior {
        if let Some((l, r)) = rest.split(k) {
            pieces.push(l);
            rest = r;
        }
    }
    pieces.push(rest);
    pieces
}

impl Command for ExplodeCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to explode", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("Explode");
        let mut n = 0;
        for id in ids {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            match o.geometry {
                Geometry::Instance { .. } => {
                    let parts = ctx.doc.instance_geometry(&o.geometry);
                    ctx.doc.remove(id);
                    for (g, a) in parts {
                        let nid = ctx.doc.add_with(g, tri_doc::Attributes { layer: o.attributes.layer, ..a });
                        ctx.doc.select(nid, true);
                        n += 1;
                    }
                }
                Geometry::Brep(b) => {
                    ctx.doc.remove(id);
                    for f in b.faces {
                        let g = if f.is_trimmed() { Geometry::Brep(Brep { faces: vec![f], closed: false, exact: None }) } else { Geometry::Surface(f.surface) };
                        let nid = ctx.doc.add_with(g, o.attributes.clone());
                        ctx.doc.select(nid, true);
                        n += 1;
                    }
                }
                Geometry::Curve(c) => {
                    let pieces = explode_curve(&c);
                    if pieces.len() < 2 {
                        continue;
                    }
                    ctx.doc.remove(id);
                    for pc in pieces {
                        let nid = ctx.doc.add_with(Geometry::Curve(pc), o.attributes.clone());
                        ctx.doc.select(nid, true);
                        n += 1;
                    }
                }
                _ => {}
            }
        }
        if n == 0 {
            ctx.log("Nothing to explode.");
        } else {
            ctx.log(format!("Exploded into {n} objects."));
        }
        Step::Done
    }
}

pub struct FlipCmd;

impl Command for FlipCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves or surfaces to flip direction", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("Flip");
        for id in ids {
            if let Some(o) = ctx.doc.get_mut(id) {
                match &mut o.geometry {
                    Geometry::Curve(c) => c.reverse(),
                    Geometry::Surface(s) => s.flip(),
                    Geometry::Brep(b) => b.faces.iter_mut().for_each(|f| f.flip()),
                    Geometry::Mesh(m) => m.flip(),
                    _ => {}
                }
            }
        }
        Step::Done
    }
}

pub struct HideCmd;

impl Command for HideCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to hide", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("Hide");
        for id in &ids {
            if let Some(o) = ctx.doc.get_mut(*id) {
                o.attributes.hidden = true;
            }
            ctx.doc.select(*id, false);
        }
        Step::Done
    }
}

pub struct ShowCmd;

impl Command for ShowCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.doc.begin_undo("Show");
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| o.attributes.hidden).map(|o| o.id).collect();
        for id in &ids {
            if let Some(o) = ctx.doc.get_mut(*id) {
                o.attributes.hidden = false;
            }
        }
        if ids.is_empty() {
            ctx.log("No hidden objects.");
        }
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub struct LockCmd;

impl Command for LockCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to lock", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("Lock");
        for id in &ids {
            if let Some(o) = ctx.doc.get_mut(*id) {
                o.attributes.locked = true;
            }
            ctx.doc.select(*id, false);
        }
        Step::Done
    }
}

pub struct UnlockCmd;

impl Command for UnlockCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.doc.begin_undo("Unlock");
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| o.attributes.locked).map(|o| o.id).collect();
        for id in &ids {
            if let Some(o) = ctx.doc.get_mut(*id) {
                o.attributes.locked = false;
            }
        }
        if ids.is_empty() {
            ctx.log("No locked objects.");
        }
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub struct PointsOnCmd;

impl Command for PointsOnCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects for control point display", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        let ids: Vec<ObjectId> = ids.into_iter().filter(|id| matches!(ctx.doc.get(*id).map(|o| &o.geometry), Some(Geometry::Curve(_)) | Some(Geometry::Surface(_)))).collect();
        if ids.is_empty() {
            ctx.log("Control points can be turned on for curves and surfaces only.");
            return Step::Done;
        }
        ctx.requests.push(super::AppRequest::PointsOn(ids));
        ctx.doc.clear_selection();
        Step::Done
    }
}
