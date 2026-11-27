//! Transformation commands: Array, ArrayPolar, ArrayLinear, ArrayCrv, Orient, Orient3Pt,
//! Rotate3D, Shear, SetPt, Align, ProjectToCPlane.

use glam::{DAffine3, DMat3, DQuat, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{BoundingBox, Plane};

use super::edit::preview_transformed;
use super::{fmt_num, opt, optv, yesno, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn copy_with(ctx: &mut CmdCtx, ids: &[ObjectId], xf: &DAffine3) -> Vec<ObjectId> {
    let mut out = Vec::new();
    for id in ids {
        if let Some(o) = ctx.doc.get(*id).cloned() {
            let mut g = o.geometry;
            g.transform(xf);
            out.push(ctx.doc.add_with(g, o.attributes));
        }
    }
    out
}

fn bbox_of(doc: &Document, ids: &[ObjectId]) -> BoundingBox {
    ids.iter().filter_map(|id| doc.get(*id)).fold(BoundingBox::EMPTY, |b, o| b.union(&o.geometry.bounding_box()))
}

// ---- Array (rectangular) ---------------------------------------------------

#[derive(Default)]
pub struct ArrayCmd {
    ids: Vec<ObjectId>,
    counts: [usize; 3],
    spacing: [Option<f64>; 3],
    step: usize,
    base: Option<DVec3>,
}

impl ArrayCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        match self.step {
            0 => Prompt::number("Number in X direction").default_num(ctx.mem_num("Array.X", 2.0)),
            1 => Prompt::number("Number in Y direction").default_num(ctx.mem_num("Array.Y", 2.0)),
            2 => Prompt::number("Number in Z direction").default_num(ctx.mem_num("Array.Z", 1.0)),
            3 => Prompt::point_or_number("Unit cell or X spacing").default_num(ctx.mem_num("Array.DX", 1.0)),
            4 => Prompt::point_or_number("Y spacing").default_num(ctx.mem_num("Array.DY", 1.0)),
            _ => Prompt::point_or_number("Z spacing").default_num(ctx.mem_num("Array.DZ", 1.0)),
        }
    }
    fn transforms(&self, cplane: &Plane) -> Vec<DAffine3> {
        let mut out = Vec::new();
        let (dx, dy, dz) = (self.spacing[0].unwrap_or(1.0), self.spacing[1].unwrap_or(1.0), self.spacing[2].unwrap_or(1.0));
        for i in 0..self.counts[0].max(1) {
            for j in 0..self.counts[1].max(1) {
                for k in 0..self.counts[2].max(1) {
                    if i == 0 && j == 0 && k == 0 {
                        continue;
                    }
                    let d = cplane.x_axis * (dx * i as f64) + cplane.y_axis * (dy * j as f64) + cplane.z_axis * (dz * k as f64);
                    out.push(DAffine3::from_translation(d));
                }
            }
        }
        out
    }
}

impl Command for ArrayCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to array", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.base = Some(bbox_of(ctx.doc, &self.ids).min);
                self.prompt(ctx).step()
            }
            Input::Number(n) | Input::Point(DVec3 { x: n, .. }) if self.step < 3 => {
                let n = if let Input::Number(v) = input { v } else { n };
                self.counts[self.step] = (n.round() as usize).max(1);
                ctx.mem_set(["Array.X", "Array.Y", "Array.Z"][self.step], self.counts[self.step]);
                self.step += 1;
                // skip spacing prompts for directions with count 1
                self.prompt(ctx).step()
            }
            inp @ (Input::Number(_) | Input::Point(_)) => {
                let i = self.step - 3;
                let d = match inp {
                    Input::Number(v) => v,
                    Input::Point(p) => {
                        let b = self.base.unwrap_or(DVec3::ZERO);
                        let axis = [ctx.cplane.x_axis, ctx.cplane.y_axis, ctx.cplane.z_axis][i];
                        (p - b).dot(axis)
                    }
                    _ => 1.0,
                };
                self.spacing[i] = Some(d);
                ctx.mem_set(["Array.DX", "Array.DY", "Array.DZ"][i], fmt_num(d));
                self.step += 1;
                // only ask spacing for directions with more than one copy
                while self.step < 6 && self.counts[self.step - 3] <= 1 {
                    self.spacing[self.step - 3] = Some(0.0);
                    self.step += 1;
                }
                if self.step >= 6 {
                    let xfs = self.transforms(&ctx.cplane);
                    ctx.doc.begin_undo("Array");
                    for xf in &xfs {
                        copy_with(ctx, &self.ids, xf);
                    }
                    ctx.log(format!("{} copies created.", xfs.len()));
                    return Step::Done;
                }
                self.prompt(ctx).step()
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        if self.step < 3 {
            return vec![];
        }
        let mut tmp = ArrayCmd { ids: self.ids.clone(), counts: self.counts, spacing: self.spacing, step: self.step, base: self.base };
        if let Some(c) = cursor {
            let i = self.step - 3;
            let axis = [DVec3::X, DVec3::Y, DVec3::Z][i];
            tmp.spacing[i] = Some((c - self.base.unwrap_or(DVec3::ZERO)).dot(axis));
        }
        tmp.transforms(&Plane::world_xy()).iter().flat_map(|xf| preview_transformed(doc, &self.ids, xf)).collect()
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

// ---- ArrayPolar -----------------------------------------------------------

#[derive(Default)]
pub struct ArrayPolarCmd {
    ids: Vec<ObjectId>,
    center: Option<DVec3>,
    count: Option<usize>,
    plane: Plane,
    /// Rotate the copies as they go around (Rotate=Yes) or keep their orientation.
    rotate: bool,
    /// Move each copy along the axis by this amount per step.
    z_offset: f64,
    /// Angle between items when the StepAngle option is used.
    step_angle: Option<f64>,
    angle: f64,
    /// Sub-prompt state: 1 = axis start, 2 = axis end, 3 = step angle, 4 = z offset, 5 = accept.
    asking: u8,
    axis_start: Option<DVec3>,
    axis: super::axis::AxisAsk,
    base_hint: Option<DVec3>,
}

impl ArrayPolarCmd {
    fn transforms(&self, count: usize, angle_deg: f64) -> Vec<DAffine3> {
        let c = self.center.unwrap_or(DVec3::ZERO);
        let full = (angle_deg.abs() - 360.0).abs() < 1e-9;
        let step = match self.step_angle {
            Some(s) => s.to_radians(),
            None => angle_deg.to_radians() / if full { count as f64 } else { (count - 1).max(1) as f64 },
        };
        (1..count)
            .map(|i| {
                let a = step * i as f64;
                let lift = DAffine3::from_translation(self.plane.z_axis * (self.z_offset * i as f64));
                if self.rotate {
                    lift * DAffine3::from_translation(c) * DAffine3::from_quat(DQuat::from_axis_angle(self.plane.z_axis, a)) * DAffine3::from_translation(-c)
                } else {
                    // orientation kept: translate each copy to its place on the circle
                    let rot = DAffine3::from_quat(DQuat::from_axis_angle(self.plane.z_axis, a));
                    let base = self.base_center();
                    let moved = c + rot.transform_vector3(base - c);
                    lift * DAffine3::from_translation(moved - base)
                }
            })
            .collect()
    }

    /// Reference point of the objects for Rotate=No (their bounding-box centre).
    fn base_center(&self) -> DVec3 {
        self.base_hint.unwrap_or_else(|| self.center.unwrap_or(DVec3::ZERO))
    }

    fn angle_prompt(&self) -> Prompt {
        Prompt::number("Angle to fill or first reference point").default_num(360.0).opts([opt("StepAngle"), optv("Rotate", yesno(self.rotate)), optv("ZOffset", fmt_num(self.z_offset))])
    }

    fn accept_prompt(&self) -> Prompt {
        Prompt::number("Press Enter to accept").default_num(self.angle).opts([optv("Rotate", yesno(self.rotate)), optv("ZOffset", fmt_num(self.z_offset))])
    }

    fn apply(&self, ctx: &mut CmdCtx) -> Step {
        let Some(n) = self.count else { return Step::Cancelled };
        let xfs = self.transforms(n, self.angle);
        ctx.doc.begin_undo("ArrayPolar");
        for xf in &xfs {
            copy_with(ctx, &self.ids, xf);
        }
        Step::Done
    }
}

impl Command for ArrayPolarCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.rotate = ctx.mem_bool("ArrayPolar.Rotate", true);
        self.z_offset = ctx.mem_num("ArrayPolar.ZOffset", 0.0);
        self.angle = 360.0;
        Prompt::objects("Select objects to array", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        if self.axis.active() {
            return match self.axis.input(input, ctx) {
                super::axis::Asked::Prompt(p) => p.step(),
                super::axis::Asked::Chosen(d) => {
                    // two points give an axis through them: its start is the centre
                    let origin = self.axis.anchor().unwrap_or(self.plane.origin);
                    self.plane = Plane::from_normal(origin, d);
                    if let Some(a) = self.axis.anchor() {
                        self.center = Some(a);
                        return Prompt::number("Number of items").default_num(ctx.mem_num("ArrayPolar.Count", 4.0)).step();
                    }
                    Prompt::point("Center of polar array").opts([opt("Axis")]).step()
                }
                super::axis::Asked::Cancelled => super::axis::abandoned(ctx),
            };
        }
        match (self.center, self.count, self.asking, input) {
            (_, _, _, Input::Objects(ids)) => {
                let bb = ids.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&o.geometry.bounding_box()));
                self.base_hint = if bb.is_empty() { None } else { Some(bb.center()) };
                self.ids = ids;
                Prompt::point("Center of polar array").opts([opt("Axis")]).step()
            }
            (None, _, 0, Input::Option(o)) if o == "Axis" => self.axis.begin().step(),
            (None, _, 0, Input::Point(p)) => {
                self.center = Some(p);
                Prompt::number("Number of items").default_num(ctx.mem_num("ArrayPolar.Count", 4.0)).step()
            }
            (Some(_), None, 0, Input::Number(n)) => {
                self.count = Some((n.round() as usize).max(2));
                ctx.mem_set("ArrayPolar.Count", self.count.unwrap());
                self.angle_prompt().step()
            }
            (Some(_), Some(_), 0, Input::Number(angle)) => {
                self.angle = angle;
                self.step_angle = None;
                self.asking = 5;
                self.accept_prompt().step()
            }
            (Some(_), Some(_), 0, Input::Empty) => {
                self.asking = 5;
                self.accept_prompt().step()
            }
            (Some(_), Some(n), 0, Input::Point(p)) => {
                // first reference point: the angle is measured from it to a second point
                let c = self.center.unwrap_or(DVec3::ZERO);
                let d0 = p - c;
                self.axis_start = Some(d0);
                self.asking = 6;
                let _ = n;
                Prompt::point("Second reference point").step()
            }
            (Some(_), Some(_), 6, Input::Point(p)) => {
                let c = self.center.unwrap_or(DVec3::ZERO);
                let d0 = self.axis_start.unwrap_or(DVec3::X);
                let d1 = p - c;
                let x = self.plane.x_axis;
                let y = self.plane.y_axis;
                let a0 = d0.dot(y).atan2(d0.dot(x));
                let a1 = d1.dot(y).atan2(d1.dot(x));
                let mut ang = (a1 - a0).to_degrees();
                if ang <= 0.0 {
                    ang += 360.0;
                }
                self.angle = ang;
                self.step_angle = None;
                self.asking = 0;
                self.apply(ctx)
            }
            (Some(_), Some(_), 0 | 5, Input::Option(o)) => match o.as_str() {
                "StepAngle" => {
                    self.asking = 3;
                    Prompt::number("Angle between items").default_num(self.step_angle.unwrap_or(360.0 / self.count.unwrap_or(4) as f64)).step()
                }
                "Rotate" => {
                    self.rotate = !self.rotate;
                    ctx.mem_set("ArrayPolar.Rotate", yesno(self.rotate));
                    if self.asking == 5 { self.accept_prompt().step() } else { self.angle_prompt().step() }
                }
                "ZOffset" => {
                    let back = self.asking;
                    self.asking = if back == 5 { 7 } else { 4 };
                    Prompt::number("Z offset between items").default_num(self.z_offset).step()
                }
                _ => self.angle_prompt().step(),
            },
            (Some(_), Some(_), 3, Input::Number(step)) => {
                self.step_angle = Some(step);
                self.asking = 5;
                self.accept_prompt().step()
            }
            (Some(_), Some(_), 4, Input::Number(z)) => {
                self.z_offset = z;
                ctx.mem_set("ArrayPolar.ZOffset", fmt_num(z));
                self.asking = 0;
                self.angle_prompt().step()
            }
            (Some(_), Some(_), 7, Input::Number(z)) => {
                self.z_offset = z;
                ctx.mem_set("ArrayPolar.ZOffset", fmt_num(z));
                self.asking = 5;
                self.accept_prompt().step()
            }
            (Some(_), Some(_), 3 | 4 | 7, Input::Empty) => {
                self.asking = 5;
                self.accept_prompt().step()
            }
            (Some(_), Some(_), 5, Input::Empty) => self.apply(ctx),
            (Some(_), Some(_), 5, Input::Number(a)) => {
                self.angle = a;
                self.apply(ctx)
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, _cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.center, self.count) {
            (Some(_), Some(n)) => self.transforms(n, self.angle).iter().flat_map(|xf| preview_transformed(doc, &self.ids, xf)).collect(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center
    }
}

// ---- ArrayLinear ------------------------------------------------------------

#[derive(Default)]
pub struct ArrayLinearCmd {
    ids: Vec<ObjectId>,
    count: Option<usize>,
    base: Option<DVec3>,
}

impl Command for ArrayLinearCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to array", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.count, self.base, input) {
            (_, _, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::number("Number of items").default_num(ctx.mem_num("ArrayLinear.Count", 3.0)).step()
            }
            (None, _, Input::Number(n)) => {
                self.count = Some((n.round() as usize).max(2));
                ctx.mem_set("ArrayLinear.Count", self.count.unwrap());
                Prompt::point("Base point").step()
            }
            (Some(_), None, Input::Point(p)) => {
                self.base = Some(p);
                Prompt::point("Second reference point").step()
            }
            (Some(n), Some(b), Input::Point(p)) => {
                let d = p - b;
                ctx.doc.begin_undo("ArrayLinear");
                for i in 1..n {
                    copy_with(ctx, &self.ids, &DAffine3::from_translation(d * i as f64));
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.count, self.base, cursor) {
            (Some(n), Some(b), Some(c)) => (1..n).flat_map(|i| preview_transformed(doc, &self.ids, &DAffine3::from_translation((c - b) * i as f64))).collect(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

// ---- ArrayCrv ---------------------------------------------------------------

#[derive(Default)]
pub struct ArrayCrvCmd {
    ids: Vec<ObjectId>,
    curve: Option<ObjectId>,
    base: Option<DVec3>,
}

impl ArrayCrvCmd {
    fn transforms(&self, doc: &Document, n: usize, orient: bool) -> Vec<DAffine3> {
        let Some(c) = self.curve.and_then(|id| doc.get(id)).and_then(|o| o.geometry.as_curve()) else { return vec![] };
        let base = self.base.unwrap_or_else(|| c.start_point());
        let pts = c.divide_by_count(n.max(2) - 1);
        let t0 = c.tangent_at(c.domain().0);
        pts.iter()
            .skip(1)
            .map(|p| {
                let mut xf = DAffine3::from_translation(*p - base);
                if orient {
                    let t = c.tangent_at(c.closest_parameter(*p));
                    let q = DQuat::from_rotation_arc(t0, t);
                    xf = DAffine3::from_translation(*p) * DAffine3::from_quat(q) * DAffine3::from_translation(-base);
                }
                xf
            })
            .collect()
    }
}

impl Command for ArrayCrvCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to array", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) if self.ids.is_empty() => {
                self.ids = ids;
                self.base = Some(bbox_of(ctx.doc, &self.ids).center());
                ctx.doc.clear_selection();
                Prompt::pick("Select path curve", ObjFilter::Curve).step()
            }
            Input::Pick(id, _) => {
                self.curve = Some(id);
                Prompt::number("Number of items").default_num(ctx.mem_num("ArrayCrv.Count", 5.0)).opts([optv("Orientation", "Freeform"), opt("Distance")]).step()
            }
            Input::Number(n) => {
                let n = (n.round() as usize).max(2);
                ctx.mem_set("ArrayCrv.Count", n);
                let xfs = self.transforms(ctx.doc, n, ctx.mem_bool("ArrayCrv.Orient", true));
                ctx.doc.begin_undo("ArrayCrv");
                for xf in &xfs {
                    copy_with(ctx, &self.ids, xf);
                }
                Step::Done
            }
            Input::Option(o) => {
                if o == "Orientation" {
                    let v = !ctx.mem_bool("ArrayCrv.Orient", true);
                    ctx.mem_set("ArrayCrv.Orient", yesno(v));
                }
                Prompt::number("Number of items").default_num(ctx.mem_num("ArrayCrv.Count", 5.0)).step()
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- Orient / Orient3Pt ----------------------------------------------------

pub struct OrientCmd {
    three: bool,
    ids: Vec<ObjectId>,
    pts: Vec<DVec3>,
    copy: bool,
    scale: bool,
}

impl OrientCmd {
    pub fn new(three: bool) -> Self {
        Self { three, ids: Vec::new(), pts: Vec::new(), copy: false, scale: false }
    }
    fn n_ref(&self) -> usize {
        if self.three {
            3
        } else {
            2
        }
    }
    fn prompt(&self) -> Prompt {
        let n = self.pts.len();
        let r = self.n_ref();
        let opts = [optv("Copy", yesno(self.copy)), optv("Scale", if self.scale { "3D" } else { "No" })];
        let msg = match (n < r, n % r) {
            (true, 0) => "Reference point 1",
            (true, 1) => "Reference point 2",
            (true, _) => "Reference point 3",
            (false, 0) => "Target point 1",
            (false, 1) => "Target point 2",
            (false, _) => "Target point 3",
        };
        Prompt::point(msg).opts(opts)
    }
    fn xf(&self, pts: &[DVec3]) -> Option<DAffine3> {
        let r = self.n_ref();
        if pts.len() < 2 * r {
            return None;
        }
        let (a0, a1) = (pts[0], pts[1]);
        let (b0, b1) = (pts[r], pts[r + 1]);
        let da = a1 - a0;
        let db = b1 - b0;
        if da.length() < 1e-12 || db.length() < 1e-12 {
            return None;
        }
        let s = if self.scale { db.length() / da.length() } else { 1.0 };
        let rot = if self.three {
            let na = da.cross(pts[2] - a0).normalize_or(DVec3::Z);
            let nb = db.cross(pts[r + 2] - b0).normalize_or(DVec3::Z);
            let fa = DMat3::from_cols(da.normalize(), na.cross(da.normalize()), na);
            let fb = DMat3::from_cols(db.normalize(), nb.cross(db.normalize()), nb);
            fb * fa.transpose()
        } else {
            DMat3::from_quat(DQuat::from_rotation_arc(da.normalize(), db.normalize()))
        };
        Some(DAffine3::from_translation(b0) * DAffine3::from_mat3(rot * s) * DAffine3::from_translation(-a0))
    }
}

impl Command for OrientCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to orient", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                self.prompt().step()
            }
            Input::Option(o) => {
                match o.as_str() {
                    "Copy" => self.copy = !self.copy,
                    "Scale" => self.scale = !self.scale,
                    _ => {}
                }
                self.prompt().step()
            }
            Input::Point(p) => {
                self.pts.push(p);
                if self.pts.len() < 2 * self.n_ref() {
                    return self.prompt().step();
                }
                match self.xf(&self.pts) {
                    Some(xf) => {
                        ctx.doc.begin_undo(if self.three { "Orient3Pt" } else { "Orient" });
                        if self.copy {
                            copy_with(ctx, &self.ids, &xf);
                        } else {
                            ctx.doc.transform_objects(&self.ids, &xf);
                        }
                        Step::Done
                    }
                    None => Step::Cancelled,
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let Some(c) = cursor else { return vec![] };
        let mut pts = self.pts.clone();
        pts.push(c);
        let mut out = Vec::new();
        if let Some(last) = self.pts.last() {
            out.push(Preview::Line(*last, c));
        }
        if let Some(xf) = self.xf(&pts) {
            out.extend(preview_transformed(doc, &self.ids, &xf));
        }
        out
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

// ---- Rotate3D ----------------------------------------------------------------

#[derive(Default)]
pub struct Rotate3DCmd {
    ids: Vec<ObjectId>,
    axis: Vec<DVec3>,
    reference: Option<DVec3>,
    copy: bool,
}

impl Rotate3DCmd {
    fn xf(&self, angle: f64) -> DAffine3 {
        let a = self.axis[0];
        let d = (self.axis[1] - a).normalize_or(DVec3::Z);
        DAffine3::from_translation(a) * DAffine3::from_quat(DQuat::from_axis_angle(d, angle)) * DAffine3::from_translation(-a)
    }
    fn angle_from(&self, r: DVec3, p: DVec3) -> f64 {
        let a = self.axis[0];
        let d = (self.axis[1] - a).normalize_or(DVec3::Z);
        let pr = (r - a) - d * (r - a).dot(d);
        let pp = (p - a) - d * (p - a).dot(d);
        let x = pr.normalize_or(DVec3::X);
        let y = d.cross(x);
        pp.dot(y).atan2(pp.dot(x))
    }
}

impl Command for Rotate3DCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to rotate", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::point("Start of rotation axis").opts([optv("Copy", yesno(self.copy))]).step()
            }
            Input::Option(_) => {
                self.copy = !self.copy;
                Prompt::point("Start of rotation axis").opts([optv("Copy", yesno(self.copy))]).step()
            }
            Input::Point(p) if self.axis.len() < 2 => {
                self.axis.push(p);
                if self.axis.len() == 1 {
                    Prompt::point("End of rotation axis").step()
                } else {
                    Prompt::point_or_number("Angle or first reference point").opts([optv("Copy", yesno(self.copy))]).step()
                }
            }
            Input::Number(deg) => {
                let xf = self.xf(deg.to_radians());
                ctx.doc.begin_undo("Rotate3D");
                if self.copy {
                    copy_with(ctx, &self.ids, &xf);
                } else {
                    ctx.doc.transform_objects(&self.ids, &xf);
                }
                Step::Done
            }
            Input::Point(p) if self.reference.is_none() => {
                self.reference = Some(p);
                Prompt::point("Second reference point").step()
            }
            Input::Point(p) => {
                let xf = self.xf(self.angle_from(self.reference.unwrap(), p));
                ctx.doc.begin_undo("Rotate3D");
                if self.copy {
                    copy_with(ctx, &self.ids, &xf);
                } else {
                    ctx.doc.transform_objects(&self.ids, &xf);
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let Some(c) = cursor else { return vec![] };
        match (self.axis.len(), self.reference) {
            (1, _) => vec![Preview::Line(self.axis[0], c)],
            (2, None) => vec![Preview::Line(self.axis[0], self.axis[1]), Preview::Line(self.axis[0], c)],
            (2, Some(r)) => {
                let mut v = vec![Preview::Line(self.axis[0], self.axis[1])];
                v.extend(preview_transformed(doc, &self.ids, &self.xf(self.angle_from(r, c))));
                v
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.reference.or(self.axis.first().copied())
    }
}

// ---- Shear --------------------------------------------------------------------

#[derive(Default)]
pub struct ShearCmd {
    ids: Vec<ObjectId>,
    origin: Option<DVec3>,
    reference: Option<DVec3>,
    plane: Plane,
}

impl ShearCmd {
    fn xf(&self, o: DVec3, r: DVec3, p: DVec3) -> Option<DAffine3> {
        let n = (r - o).normalize_or(DVec3::ZERO);
        if n == DVec3::ZERO {
            return None;
        }
        let h = (r - o).length();
        let d = p - r;
        let shear = d - n * d.dot(n); // displacement of the reference point perpendicular to the axis
        let m = DMat3::IDENTITY + DMat3::from_cols(shear * (n.x / h), shear * (n.y / h), shear * (n.z / h));
        Some(DAffine3::from_translation(o) * DAffine3::from_mat3(m) * DAffine3::from_translation(-o))
    }
}

impl Command for ShearCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::objects("Select objects to shear", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.origin, self.reference, input) {
            (_, _, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point("Origin point").opts([optv("Copy", "No")]).step()
            }
            (None, _, Input::Point(p)) => {
                self.origin = Some(p);
                Prompt::point("Reference point").step()
            }
            (Some(_), None, Input::Point(p)) => {
                self.reference = Some(p);
                Prompt::point_or_number("Shear angle or point").step()
            }
            (Some(o), Some(r), Input::Point(p)) => match self.xf(o, r, p) {
                Some(xf) => {
                    ctx.doc.begin_undo("Shear");
                    ctx.doc.transform_objects(&self.ids, &xf);
                    Step::Done
                }
                None => Step::Cancelled,
            },
            (Some(o), Some(r), Input::Number(deg)) => {
                let n = (r - o).normalize_or(DVec3::Z);
                let side = self.plane.z_axis.cross(n).normalize_or(self.plane.x_axis);
                let p = r + side * ((r - o).length() * deg.to_radians().tan());
                match self.xf(o, r, p) {
                    Some(xf) => {
                        ctx.doc.begin_undo("Shear");
                        ctx.doc.transform_objects(&self.ids, &xf);
                        Step::Done
                    }
                    None => Step::Cancelled,
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.origin, self.reference, cursor) {
            (Some(o), None, Some(c)) => vec![Preview::Line(o, c)],
            (Some(o), Some(r), Some(c)) => {
                let mut v = vec![Preview::Line(o, r), Preview::Line(r, c)];
                if let Some(xf) = self.xf(o, r, c) {
                    v.extend(preview_transformed(doc, &self.ids, &xf));
                }
                v
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.reference.or(self.origin)
    }
}

// ---- SetPt / ProjectToCPlane ---------------------------------------------------

pub struct SetPtCmd {
    ids: Vec<ObjectId>,
    x: bool,
    y: bool,
    z: bool,
    to_cplane: bool,
}

impl SetPtCmd {
    pub fn new(to_cplane: bool) -> Self {
        Self { ids: Vec::new(), x: !to_cplane, y: !to_cplane, z: true, to_cplane }
    }
    fn prompt(&self) -> Prompt {
        Prompt::point("Location for points").opts([optv("XSet", yesno(self.x)), optv("YSet", yesno(self.y)), optv("ZSet", yesno(self.z)), optv("Alignment", "CPlane")])
    }
}

impl Command for SetPtCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects(if self.to_cplane { "Select objects to project to the construction plane" } else { "Select objects to set points" }, ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                if self.to_cplane {
                    // flatten every control point onto the construction plane
                    let cp = ctx.cplane;
                    ctx.doc.begin_undo("ProjectToCPlane");
                    for id in &self.ids {
                        if let Some(o) = ctx.doc.get_mut(*id) {
                            flatten(&mut o.geometry, &cp, false, false, true, cp.origin);
                        }
                    }
                    return Step::Done;
                }
                self.prompt().step()
            }
            Input::Option(o) => {
                match o.as_str() {
                    "XSet" => self.x = !self.x,
                    "YSet" => self.y = !self.y,
                    "ZSet" => self.z = !self.z,
                    _ => {}
                }
                self.prompt().step()
            }
            Input::Point(p) => {
                let cp = ctx.cplane;
                ctx.doc.begin_undo("SetPt");
                for id in &self.ids {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        flatten(&mut o.geometry, &cp, self.x, self.y, self.z, p);
                    }
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

/// Set the chosen construction-plane coordinates of all control points to those of `target`.
fn flatten(g: &mut Geometry, cp: &Plane, sx: bool, sy: bool, sz: bool, target: DVec3) {
    let (tu, tv, tw) = cp.closest_parameters(target);
    let set = |p: &mut DVec3| {
        let (mut u, mut v, mut w) = cp.closest_parameters(*p);
        if sx {
            u = tu;
        }
        if sy {
            v = tv;
        }
        if sz {
            w = tw;
        }
        *p = cp.point_at_3(u, v, w);
    };
    match g {
        Geometry::Point(p) => set(p),
        Geometry::Curve(c) => c.control_points.iter_mut().for_each(set),
        Geometry::Surface(s) => s.control_points.iter_mut().for_each(set),
        Geometry::Brep(b) => b.faces.iter_mut().for_each(|f| f.surface.control_points.iter_mut().for_each(set)),
        Geometry::Mesh(m) => {
            m.vertices.iter_mut().for_each(set);
            m.compute_normals();
        }
        Geometry::Instance { xf, .. } => {
            let mut t = xf.translation;
            set(&mut t);
            xf.translation = t;
        }
    }
}

// ---- Align ---------------------------------------------------------------------

#[derive(Default)]
pub struct AlignCmd {
    ids: Vec<ObjectId>,
}

impl Command for AlignCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to align", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::options("Alignment").opts([opt("Bottom"), opt("Top"), opt("Left"), opt("Right"), opt("HorizCenter"), opt("VertCenter"), opt("Concentric")]).step()
            }
            Input::Option(o) => {
                let cp = ctx.cplane;
                let all = bbox_of(ctx.doc, &self.ids);
                ctx.doc.begin_undo("Align");
                for id in self.ids.clone() {
                    let Some(ob) = ctx.doc.get(id).map(|x| x.geometry.bounding_box()) else { continue };
                    let (u0, v0, _) = cp.closest_parameters(ob.min);
                    let (u1, v1, _) = cp.closest_parameters(ob.max);
                    let (au0, av0, _) = cp.closest_parameters(all.min);
                    let (au1, av1, _) = cp.closest_parameters(all.max);
                    let (du, dv) = match o.as_str() {
                        "Bottom" => (0.0, av0 - v0),
                        "Top" => (0.0, av1 - v1),
                        "Left" => (au0 - u0, 0.0),
                        "Right" => (au1 - u1, 0.0),
                        "HorizCenter" => (0.0, (av0 + av1) * 0.5 - (v0 + v1) * 0.5),
                        "VertCenter" => ((au0 + au1) * 0.5 - (u0 + u1) * 0.5, 0.0),
                        _ => ((au0 + au1) * 0.5 - (u0 + u1) * 0.5, (av0 + av1) * 0.5 - (v0 + v1) * 0.5),
                    };
                    let xf = DAffine3::from_translation(cp.x_axis * du + cp.y_axis * dv);
                    ctx.doc.transform_objects(&[id], &xf);
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}
