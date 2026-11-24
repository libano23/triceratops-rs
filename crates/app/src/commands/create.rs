//! Geometry creation commands with Triceratops prompts and options.

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{Brep, Face, NurbsCurve, NurbsSurface, Plane};

use super::{fmt_num, opt, optv, yesno, AppRequest, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn not_implemented(ctx: &mut CmdCtx, option: &str) {
    ctx.log(format!("The {option} option is not implemented in triceratops yet."));
}

fn add_geometry(ctx: &mut CmdCtx, label: &str, g: Geometry) -> ObjectId {
    ctx.doc.begin_undo(label);
    ctx.doc.add(g)
}

fn add_curve(ctx: &mut CmdCtx, label: &str, c: tri_geom::Result<NurbsCurve>) -> Step {
    match c {
        Ok(c) => {
            add_geometry(ctx, label, Geometry::Curve(c));
            Step::Done
        }
        Err(e) => {
            ctx.log(format!("{label} failed: {e}"));
            Step::Cancelled
        }
    }
}

fn add_surface(ctx: &mut CmdCtx, label: &str, s: tri_geom::Result<NurbsSurface>) -> Step {
    match s {
        Ok(s) => {
            add_geometry(ctx, label, Geometry::Surface(s));
            Step::Done
        }
        Err(e) => {
            ctx.log(format!("{label} failed: {e}"));
            Step::Cancelled
        }
    }
}

fn add_brep(ctx: &mut CmdCtx, label: &str, b: tri_geom::Result<Brep>) -> Step {
    match b {
        Ok(b) => {
            add_geometry(ctx, label, Geometry::Brep(b));
            Step::Done
        }
        Err(e) => {
            ctx.log(format!("{label} failed: {e}"));
            Step::Cancelled
        }
    }
}

pub fn plane_at(cplane: &Plane, origin: DVec3) -> Plane {
    Plane { origin, ..*cplane }
}

/// Plane perpendicular to the construction plane through `origin`, whose x axis
/// points from `origin` towards `toward` (projected onto the construction plane).
fn vertical_plane(cplane: &Plane, origin: DVec3, toward: DVec3) -> Plane {
    let d = toward - origin;
    let d = d - cplane.z_axis * d.dot(cplane.z_axis);
    let x = d.normalize_or(cplane.x_axis);
    Plane { origin, x_axis: x, y_axis: cplane.z_axis, z_axis: x.cross(cplane.z_axis) }
}

fn height_from(cplane: &Plane, base: DVec3, input: &Input) -> Option<f64> {
    match input {
        Input::Number(n) => Some(*n),
        Input::Point(p) => Some((*p - base).dot(cplane.z_axis)),
        _ => None,
    }
}

pub use tri_geom::curve_plane;

/// Planar cap for a closed planar curve.
pub fn planar_cap(c: &NurbsCurve) -> Option<Face> {
    if !c.is_closed() {
        return None;
    }
    Face::from_planar_curves(std::slice::from_ref(c), 1e-3 * (1.0 + c.bounding_box().diagonal())).ok()
}

/// Group closed planar curves into faces with holes: a curve inside another coplanar
/// curve becomes a hole of it.
pub fn planar_faces(curves: &[NurbsCurve]) -> Vec<Face> {
    use tri_geom::brep::point_in_polygon;
    let tol = |c: &NurbsCurve| 1e-3 * (1.0 + c.bounding_box().diagonal());
    let mut items: Vec<(Plane, Vec<DVec3>, f64)> = Vec::new();
    for c in curves {
        if !c.is_closed() {
            continue;
        }
        let Some(plane) = curve_plane(c) else { continue };
        let pts = c.tessellate_adaptive(tol(c));
        let area = {
            let uv: Vec<glam::DVec2> = pts.iter().map(|p| { let (u, v, _) = plane.closest_parameters(*p); glam::DVec2::new(u, v) }).collect();
            tri_geom::brep::signed_area(&uv).abs()
        };
        items.push((plane, pts, area));
    }
    // largest first; each smaller coplanar loop inside a larger one becomes its hole
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|a, b| items[*b].2.partial_cmp(&items[*a].2).unwrap_or(std::cmp::Ordering::Equal));
    let mut used = vec![false; items.len()];
    let mut faces = Vec::new();
    for &i in &order {
        if used[i] {
            continue;
        }
        used[i] = true;
        let (plane, outer, _) = &items[i];
        let outer_uv: Vec<glam::DVec2> = outer.iter().map(|p| { let (u, v, _) = plane.closest_parameters(*p); glam::DVec2::new(u, v) }).collect();
        let mut loops = vec![outer.clone()];
        for &j in &order {
            if used[j] {
                continue;
            }
            let (pj, pts_j, _) = &items[j];
            let coplanar = pj.z_axis.dot(plane.z_axis).abs() > 0.999 && plane.signed_distance(pj.origin).abs() < 1e-6 * (1.0 + outer.len() as f64);
            if !coplanar {
                continue;
            }
            let (u, v, _) = plane.closest_parameters(pts_j[0]);
            if point_in_polygon(glam::DVec2::new(u, v), &outer_uv) {
                loops.push(pts_j.clone());
                used[j] = true;
            }
        }
        if let Ok(f) = Face::planar(plane, loops) {
            faces.push(f);
        }
    }
    faces
}

// ---------------------------------------------------------------------------

pub struct PointCmd {
    multiple: bool,
}

impl PointCmd {
    pub fn new(multiple: bool) -> Self {
        Self { multiple }
    }
}

impl Command for PointCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Location of point object").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                add_geometry(ctx, "Point", Geometry::Point(p));
                if self.multiple {
                    Prompt::point("Location of point object. Press Enter when done").step()
                } else {
                    Step::Done
                }
            }
            _ => Step::Done,
        }
    }
}

#[derive(Default)]
pub struct LineCmd {
    start: Option<DVec3>,
    both_sides: bool,
    vertical: bool,
    /// Angled: reference line points, then the angle.
    angled: Vec<DVec3>,
    angle: Option<f64>,
    /// Perpendicular / Tangent / Normal: constrained start direction from a picked object.
    constrained_dir: Option<DVec3>,
    mode: Option<&'static str>,
    /// FourPoint: two points for the direction, then start/end.
    four: Vec<DVec3>,
    /// Bisector: three points defining the angle.
    bisector: Vec<DVec3>,
}

impl LineCmd {
    fn prompt(&self) -> Prompt {
        if let Some(m) = self.mode {
            match (m, self.start, self.angled.len(), self.four.len(), self.bisector.len()) {
                ("Angled", _, 0, _, _) => return Prompt::point("Start of reference line"),
                ("Angled", _, 1, _, _) => return Prompt::point("End of reference line"),
                ("Angled", _, _, _, _) if self.angle.is_none() => return Prompt::number("Angle").default_num(45.0),
                ("FourPoint", _, _, 0, _) => return Prompt::point("Start of base line"),
                ("FourPoint", _, _, 1, _) => return Prompt::point("End of base line"),
                ("Bisector", _, _, _, 0) => return Prompt::point("Start of bisector"),
                ("Bisector", _, _, _, 1) => return Prompt::point("Start of angle to bisect"),
                ("Bisector", _, _, _, 2) => return Prompt::point("End of angle to bisect"),
                ("Perpendicular", None, _, _, _) | ("Tangent", None, _, _, _) => {
                    return Prompt::pick(if m == "Perpendicular" { "Select curve for perpendicular line" } else { "Select curve for tangent line" }, ObjFilter::Curve)
                }
                ("Normal", None, _, _, _) => return Prompt::pick("Select surface for normal line", ObjFilter::Surface),
                _ => {}
            }
        }
        match self.start {
            None => Prompt::point("Start of line").opts([
                opt("BothSides"),
                opt("Normal"),
                opt("Angled"),
                opt("Vertical"),
                opt("FourPoint"),
                opt("Bisector"),
                opt("Perpendicular"),
                opt("Tangent"),
                opt("Extension"),
            ]),
            Some(_) => {
                let mut p = Prompt::point_or_number("End of line").opts([opt("BothSides")]);
                if self.vertical {
                    p.msg = "End of line (vertical)".into();
                } else if self.constrained_dir.is_some() || self.angle.is_some() || self.four.len() == 2 || self.bisector.len() == 3 {
                    p.msg = "End of line or length".into();
                }
                p
            }
        }
    }

    /// Direction constraint for the end point, if any.
    fn direction(&self, cplane: &Plane) -> Option<DVec3> {
        if self.vertical {
            return Some(cplane.z_axis);
        }
        if let Some(d) = self.constrained_dir {
            return Some(d);
        }
        if let (Some(a), 2) = (self.angle, self.angled.len()) {
            let d = (self.angled[1] - self.angled[0]).normalize_or(cplane.x_axis);
            let q = glam::DQuat::from_axis_angle(cplane.z_axis, a.to_radians());
            return Some(q * d);
        }
        if self.four.len() == 2 {
            return Some((self.four[1] - self.four[0]).normalize_or(cplane.x_axis));
        }
        if self.bisector.len() == 3 {
            let a = (self.bisector[1] - self.bisector[0]).normalize_or(cplane.x_axis);
            let b = (self.bisector[2] - self.bisector[0]).normalize_or(cplane.y_axis);
            return Some((a + b).normalize_or(a));
        }
        None
    }

    fn end_from(&self, start: DVec3, input: &Input, cplane: &Plane) -> Option<DVec3> {
        match (self.direction(cplane), input) {
            (Some(d), Input::Number(len)) => Some(start + d * *len),
            (Some(d), Input::Point(p)) => Some(start + d * (*p - start).dot(d)),
            (None, Input::Point(p)) => Some(*p),
            _ => None,
        }
    }
}

impl Command for LineCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.start, input) {
            (_, Input::Option(o)) => {
                match o.as_str() {
                    "BothSides" => self.both_sides = !self.both_sides,
                    "Vertical" => self.vertical = true,
                    "Angled" | "FourPoint" | "Bisector" | "Perpendicular" | "Tangent" | "Normal" => {
                        self.mode = Some(match o.as_str() {
                            "Angled" => "Angled",
                            "FourPoint" => "FourPoint",
                            "Bisector" => "Bisector",
                            "Perpendicular" => "Perpendicular",
                            "Tangent" => "Tangent",
                            _ => "Normal",
                        });
                    }
                    other => not_implemented(ctx, other),
                }
                self.prompt().step()
            }
            (None, Input::Pick(id, p)) => {
                let Some(o) = ctx.doc.get(id) else { return Step::Cancelled };
                match (&o.geometry, self.mode) {
                    (Geometry::Curve(c), Some("Perpendicular")) | (Geometry::Curve(c), Some("Tangent")) => {
                        let t = c.closest_parameter(p);
                        let tan = c.tangent_at(t);
                        let start = c.point_at(t);
                        self.constrained_dir = Some(if self.mode == Some("Tangent") { tan } else { ctx.cplane.z_axis.cross(tan).normalize_or(ctx.cplane.y_axis) });
                        self.start = Some(start);
                    }
                    (Geometry::Surface(s), Some("Normal")) => {
                        let (u, v) = s.closest_uv(p);
                        self.constrained_dir = Some(s.normal_at(u, v));
                        self.start = Some(s.point_at(u, v));
                    }
                    (Geometry::Brep(b), Some("Normal")) => {
                        let fi = super::surface_edit::face_under(&b.faces, p);
                        let s = &b.faces[fi].surface;
                        let (u, v) = s.closest_uv(p);
                        self.constrained_dir = Some(s.normal_at(u, v));
                        self.start = Some(s.point_at(u, v));
                    }
                    _ => return self.prompt().step(),
                }
                self.prompt().step()
            }
            (None, Input::Number(a)) if self.mode == Some("Angled") && self.angled.len() == 2 => {
                self.angle = Some(a);
                Prompt::point("Start of line").step()
            }
            (None, Input::Point(p)) if self.mode == Some("Angled") && self.angled.len() < 2 => {
                self.angled.push(p);
                self.prompt().step()
            }
            (None, Input::Point(p)) if self.mode == Some("FourPoint") && self.four.len() < 2 => {
                self.four.push(p);
                if self.four.len() == 2 {
                    Prompt::point("Start of line").step()
                } else {
                    self.prompt().step()
                }
            }
            (None, Input::Point(p)) if self.mode == Some("Bisector") && self.bisector.len() < 3 => {
                self.bisector.push(p);
                if self.bisector.len() == 3 {
                    self.start = Some(self.bisector[0]);
                }
                self.prompt().step()
            }
            (None, Input::Point(p)) => {
                self.start = Some(p);
                self.prompt().step()
            }
            (Some(a), inp @ (Input::Point(_) | Input::Number(_))) => {
                let Some(b) = self.end_from(a, &inp, &ctx.cplane) else { return Step::Cancelled };
                let (a, b) = if self.both_sides { (a - (b - a), b) } else { (a, b) };
                add_curve(ctx, "Line", NurbsCurve::line(a, b))
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let cp = Plane::world_xy();
        match (self.start, cursor) {
            (Some(a), Some(c)) => {
                let b = self.end_from(a, &Input::Point(c), &cp).unwrap_or(c);
                let a0 = if self.both_sides { a - (b - a) } else { a };
                vec![Preview::Line(a0, b)]
            }
            (None, Some(c)) if !self.angled.is_empty() => vec![Preview::Line(*self.angled.last().unwrap(), c)],
            (None, Some(c)) if !self.four.is_empty() => vec![Preview::Line(*self.four.last().unwrap(), c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.start.or(self.angled.last().copied()).or(self.four.last().copied()).or(self.bisector.last().copied())
    }
}

#[derive(Default)]
pub struct PolylineCmd {
    points: Vec<DVec3>,
    persistent_close: bool,
    arc_mode: bool,
    helpers: bool,
    separate_lines: bool,
    pending_length: Option<f64>,
}


impl PolylineCmd {
    pub fn lines() -> Self {
        Self { separate_lines: true, ..Default::default() }
    }

    fn prompt(&self) -> Prompt {
        let n = self.points.len();
        if self.separate_lines {
            return match n {
                0 => Prompt::point("Start of first line"),
                1 => Prompt::point("End of line").opts([opt("Undo")]),
                _ => Prompt::point("End of next line. Press Enter when done").opts([opt("Close"), opt("Undo")]),
            };
        }
        let mut opts = vec![
            optv("PersistentClose", yesno(self.persistent_close)),
            optv("Mode", if self.arc_mode { "Arc" } else { "Line" }),
            optv("Helpers", yesno(self.helpers)),
        ];
        match n {
            0 => Prompt::point("Start of polyline").opts(opts),
            1 => {
                opts.push(opt("Length"));
                opts.push(opt("Undo"));
                Prompt::point("Next point of polyline").opts(opts)
            }
            2 => {
                opts.push(opt("Length"));
                opts.push(opt("Undo"));
                Prompt::point("Next point of polyline. Press Enter when done").opts(opts)
            }
            _ => {
                opts.push(opt("Length"));
                opts.push(opt("Close"));
                opts.push(opt("Undo"));
                Prompt::point("Next point of polyline. Press Enter when done").opts(opts)
            }
        }
    }

    fn finish(&mut self, ctx: &mut CmdCtx, close: bool) -> Step {
        if self.points.len() < 2 {
            return Step::Cancelled;
        }
        let mut pts = std::mem::take(&mut self.points);
        if (close || self.persistent_close) && pts.len() >= 3 {
            pts.push(pts[0]);
        }
        if self.separate_lines {
            ctx.doc.begin_undo("Lines");
            for w in pts.windows(2) {
                if let Ok(l) = NurbsCurve::line(w[0], w[1]) {
                    ctx.doc.add(Geometry::Curve(l));
                }
            }
            return Step::Done;
        }
        add_curve(ctx, "Polyline", NurbsCurve::polyline(pts))
    }
}

impl Command for PolylineCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.points.push(p);
                self.prompt().step()
            }
            Input::Option(o) => match o.as_str() {
                "Close" => self.finish(ctx, true),
                "Undo" => {
                    self.points.pop();
                    self.prompt().step()
                }
                "PersistentClose" => {
                    self.persistent_close = !self.persistent_close;
                    self.prompt().step()
                }
                "Mode" => {
                    self.arc_mode = !self.arc_mode;
                    if self.arc_mode {
                        not_implemented(ctx, "Mode=Arc");
                        self.arc_mode = false;
                    }
                    self.prompt().step()
                }
                "Helpers" => {
                    self.helpers = !self.helpers;
                    self.prompt().step()
                }
                "Length" => Prompt::number("Length of next segment").step(),
                _ => self.prompt().step(),
            },
            Input::Number(len) => {
                // Length option: next point at the given length along the cursor direction
                self.pending_length = Some(len);
                ctx.log(format!("Next segment length: {}", fmt_num(len)));
                self.prompt().step()
            }
            Input::Empty => self.finish(ctx, false),
            Input::Text(_) => self.prompt().step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let mut pts = self.points.clone();
        if let Some(mut c) = cursor {
            if let (Some(len), Some(last)) = (self.pending_length, pts.last().copied()) {
                c = last + (c - last).normalize_or(DVec3::X) * len;
            }
            pts.push(c);
        }
        if self.persistent_close && pts.len() >= 3 {
            pts.push(pts[0]);
        }
        if pts.len() >= 2 {
            vec![Preview::Polyline(pts)]
        } else {
            vec![]
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.points.last().copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RectMode {
    Corner,
    ThreePoint,
    Vertical,
    Center,
}

pub struct RectangleCmd {
    mode: RectMode,
    pts: Vec<DVec3>,
    length: Option<f64>,
    plane: Plane,
    rounded: bool,
    pending_round: Option<(DVec3, DVec3)>,
}

impl Default for RectangleCmd {
    fn default() -> Self {
        Self { mode: RectMode::Corner, pts: Vec::new(), length: None, plane: Plane::world_xy(), rounded: false, pending_round: None }
    }
}

pub fn rect_points(cplane: &Plane, a: DVec3, b: DVec3) -> Vec<DVec3> {
    let (u0, v0, w) = cplane.closest_parameters(a);
    let (u1, v1, _) = cplane.closest_parameters(b);
    vec![
        cplane.point_at_3(u0, v0, w),
        cplane.point_at_3(u1, v0, w),
        cplane.point_at_3(u1, v1, w),
        cplane.point_at_3(u0, v1, w),
        cplane.point_at_3(u0, v0, w),
    ]
}

impl RectangleCmd {
    fn mode_opts(&self) -> Vec<super::Opt> {
        vec![opt("3Point"), opt("Vertical"), opt("Center"), optv("Rounded", yesno(self.rounded))]
    }

    fn prompt(&self) -> Prompt {
        let n = self.pts.len();
        match (self.mode, n) {
            (RectMode::Corner, 0) => Prompt::point("First corner of rectangle").opts(self.mode_opts()),
            (RectMode::Corner, _) if self.length.is_none() => Prompt::point_or_number("Other corner or length").opts(self.mode_opts()),
            (RectMode::Corner, _) => Prompt::point_or_number("Width. Press Enter to use length").default_num(self.length.unwrap_or(1.0)),
            (RectMode::Center, 0) => Prompt::point("Center of rectangle").opts(self.mode_opts()),
            (RectMode::Center, _) => Prompt::point_or_number("Other corner or length"),
            (RectMode::ThreePoint, 0) | (RectMode::Vertical, 0) => Prompt::point("Start of edge").opts(self.mode_opts()),
            (RectMode::ThreePoint, 1) | (RectMode::Vertical, 1) => Prompt::point_or_number("End of edge"),
            (RectMode::ThreePoint, _) => Prompt::point_or_number("Width. Press Enter to use length").default_num(self.edge_len()),
            (RectMode::Vertical, _) => Prompt::point_or_number("Height. Press Enter to use length").default_num(self.edge_len()),
        }
    }

    fn edge_len(&self) -> f64 {
        if self.pts.len() >= 2 {
            (self.pts[1] - self.pts[0]).length()
        } else {
            1.0
        }
    }

    fn points_for(&self, third: DVec3) -> Vec<DVec3> {
        match self.mode {
            RectMode::Corner => rect_points(&self.plane, self.pts[0], third),
            RectMode::Center => {
                let c = self.pts[0];
                let d = third - c;
                rect_points(&self.plane, c - d, c + d)
            }
            RectMode::ThreePoint | RectMode::Vertical => {
                let a = self.pts[0];
                let b = self.pts[1];
                let x = (b - a).normalize_or(self.plane.x_axis);
                let y = if self.mode == RectMode::Vertical {
                    self.plane.z_axis
                } else {
                    let n = self.plane.z_axis;
                    n.cross(x).normalize_or(self.plane.y_axis)
                };
                let w = (third - a).dot(y);
                vec![a, b, b + y * w, a + y * w, a]
            }
        }
    }

    fn finish_with(&mut self, ctx: &mut CmdCtx, third: DVec3) -> Step {
        let pts = self.points_for(third);
        if pts.len() < 5 || (pts[1] - pts[0]).length() < 1e-9 || (pts[3] - pts[0]).length() < 1e-9 {
            ctx.log("Rectangle has zero area.");
            return Step::Cancelled;
        }
        if self.rounded {
            self.pending_round = Some((pts[0], pts[2]));
            return Prompt::point_or_number("Radius or point for corner").default_num(ctx.mem_num("Rectangle.Radius", 1.0)).opts([optv("Corner", "Arc")]).step();
        }
        add_curve(ctx, "Rectangle", NurbsCurve::polyline(pts))
    }
}

impl Command for RectangleCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                match o.as_str() {
                    "3Point" => self.mode = RectMode::ThreePoint,
                    "Vertical" => self.mode = RectMode::Vertical,
                    "Center" => self.mode = RectMode::Center,
                    "Rounded" => self.rounded = !self.rounded,
                    _ => {}
                }
                self.prompt().step()
            }
            inp @ (Input::Point(_) | Input::Number(_)) if self.pending_round.is_some() => {
                let (a, b) = self.pending_round.unwrap();
                let r = match inp {
                    Input::Number(v) => v.abs(),
                    Input::Point(p) => {
                        let (u, _, _) = self.plane.closest_parameters(p);
                        let (ua, _, _) = self.plane.closest_parameters(a);
                        let (ub, _, _) = self.plane.closest_parameters(b);
                        (u - ua.min(ub)).abs().min((ua.max(ub) - u).abs())
                    }
                    _ => 1.0,
                };
                ctx.mem_set("Rectangle.Radius", fmt_num(r));
                match super::cplane_cmds::rounded_rectangle(&self.plane, a, b, r) {
                    Some(c) => add_curve(ctx, "Rectangle", Ok(c)),
                    None => Step::Cancelled,
                }
            }
            Input::Point(p) => {
                let n = self.pts.len();
                match (self.mode, n) {
                    (RectMode::Corner, 0) | (RectMode::Center, 0) => {
                        self.pts.push(p);
                        self.prompt().step()
                    }
                    (RectMode::Corner, _) if self.length.is_some() => {
                        let a = self.pts[0];
                        let l = self.length.unwrap();
                        let w = (p - a).dot(self.plane.y_axis);
                        let third = a + self.plane.x_axis * l + self.plane.y_axis * w;
                        self.finish_with(ctx, third)
                    }
                    (RectMode::Corner, _) | (RectMode::Center, _) => self.finish_with(ctx, p),
                    (_, 0) | (_, 1) => {
                        self.pts.push(p);
                        self.prompt().step()
                    }
                    _ => self.finish_with(ctx, p),
                }
            }
            Input::Number(v) => {
                let n = self.pts.len();
                match (self.mode, n) {
                    (RectMode::Corner, 1) if self.length.is_none() => {
                        self.length = Some(v);
                        self.prompt().step()
                    }
                    (RectMode::Corner, _) => {
                        let a = self.pts[0];
                        let third = a + self.plane.x_axis * self.length.unwrap_or(v) + self.plane.y_axis * v;
                        self.finish_with(ctx, third)
                    }
                    (RectMode::Center, _) => {
                        let c = self.pts[0];
                        let third = c + self.plane.x_axis * (v * 0.5) + self.plane.y_axis * (v * 0.5);
                        self.finish_with(ctx, third)
                    }
                    (_, 1) => {
                        // typed edge length along the cursor direction is not known: along x
                        let a = self.pts[0];
                        self.pts.push(a + self.plane.x_axis * v);
                        self.prompt().step()
                    }
                    (RectMode::ThreePoint, _) | (RectMode::Vertical, _) => {
                        let a = self.pts[0];
                        let b = self.pts[1];
                        let x = (b - a).normalize_or(self.plane.x_axis);
                        let y = if self.mode == RectMode::Vertical { self.plane.z_axis } else { self.plane.z_axis.cross(x) };
                        self.finish_with(ctx, a + y * v)
                    }
                }
            }
            Input::Empty => {
                let n = self.pts.len();
                if (self.mode == RectMode::Corner && self.length.is_some()) || n >= 2 {
                    let l = self.length.unwrap_or_else(|| self.edge_len());
                    self.input(Input::Number(l), ctx)
                } else {
                    Step::Cancelled
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(c) = cursor else { return vec![] };
        let n = self.pts.len();
        match (self.mode, n) {
            (RectMode::Corner, 1) if self.length.is_some() => {
                let a = self.pts[0];
                let w = (c - a).dot(self.plane.y_axis);
                vec![Preview::Polyline(self.points_for(a + self.plane.x_axis * self.length.unwrap() + self.plane.y_axis * w))]
            }
            (RectMode::Corner, 1) | (RectMode::Center, 1) => vec![Preview::Polyline(self.points_for(c))],
            (_, 1) => vec![Preview::Line(self.pts[0], c)],
            (_, 2) => vec![Preview::Polyline(self.points_for(c))],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CircleMode {
    CenterRadius,
    TwoPoint,
    ThreePoint,
    Vertical,
}

pub struct CircleCmd {
    mode: CircleMode,
    pts: Vec<DVec3>,
    diameter: bool,
    deformable: bool,
    plane: Plane,
    /// Sub-mode set by Tangent / AroundCurve / FitPoints, with the picked curve.
    sub: Option<&'static str>,
    sub_curve: Option<NurbsCurve>,
    /// Radius entry mode: "Circumference" or "Area" instead of radius/diameter.
    size_mode: Option<&'static str>,
}

impl Default for CircleCmd {
    fn default() -> Self {
        Self { mode: CircleMode::CenterRadius, pts: Vec::new(), diameter: false, deformable: false, plane: Plane::world_xy(), sub: None, sub_curve: None, size_mode: None }
    }
}

impl CircleCmd {
    /// Radius from a typed number honouring Diameter / Circumference / Area modes.
    fn radius_from_number(&self, v: f64) -> f64 {
        match self.size_mode {
            Some("Circumference") => v.abs() / std::f64::consts::TAU,
            Some("Area") => (v.abs() / std::f64::consts::PI).sqrt(),
            _ => {
                if self.diameter {
                    v.abs() * 0.5
                } else {
                    v.abs()
                }
            }
        }
    }

    /// Least-squares circle through points (plane fit + algebraic fit).
    fn fit_circle(pts: &[DVec3]) -> Option<NurbsCurve> {
        if pts.len() < 3 {
            return None;
        }
        let c = pts.iter().fold(DVec3::ZERO, |a, p| a + *p) / pts.len() as f64;
        let (dir, normal) = super::misc3::principal_axes(pts, c);
        let plane = Plane::from_frame(c, dir, normal.cross(dir));
        // algebraic fit: x² + y² + D x + E y + F = 0
        let uv: Vec<(f64, f64)> = pts.iter().map(|p| { let (u, v, _) = plane.closest_parameters(*p); (u, v) }).collect();
        let mut a = [[0.0f64; 3]; 3];
        let mut b = [0.0f64; 3];
        for (u, v) in &uv {
            let row = [*u, *v, 1.0];
            let rhs = -(u * u + v * v);
            for i in 0..3 {
                for j in 0..3 {
                    a[i][j] += row[i] * row[j];
                }
                b[i] += row[i] * rhs;
            }
        }
        // solve 3x3 by Cramer's rule
        let det = |m: &[[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        let d = det(&a);
        if d.abs() < 1e-18 {
            return None;
        }
        let mut sol = [0.0; 3];
        for k in 0..3 {
            let mut m = a;
            for i in 0..3 {
                m[i][k] = b[i];
            }
            sol[k] = det(&m) / d;
        }
        let (cu, cv) = (-sol[0] * 0.5, -sol[1] * 0.5);
        let r2 = cu * cu + cv * cv - sol[2];
        if r2 <= 0.0 {
            return None;
        }
        let center = plane.point_at(cu, cv);
        NurbsCurve::circle(&Plane { origin: center, ..plane }, r2.sqrt()).ok()
    }
}

impl CircleCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        let n = self.pts.len();
        let first_opts = [opt("Deformable"), opt("Vertical"), opt("2Point"), opt("3Point"), opt("Tangent"), opt("AroundCurve"), opt("FitPoints")];
        let radius_opts = [
            opt(if self.diameter { "Radius" } else { "Diameter" }),
            opt("Orientation"),
            opt("Circumference"),
            opt("Area"),
        ];
        match self.sub {
            Some("Tangent") if self.sub_curve.is_none() => return Prompt::pick("Select first tangent curve", ObjFilter::Curve),
            Some("Tangent") if self.pts.is_empty() => return Prompt::point("Point on curve for tangent"),
            Some("Tangent") => return Prompt::point_or_number("Radius").default_num(ctx.mem_num("Circle.Radius", 1.0)),
            Some("AroundCurve") if self.sub_curve.is_none() => return Prompt::pick("Select curve", ObjFilter::Curve),
            Some("AroundCurve") if self.pts.is_empty() => return Prompt::point("Center of circle on curve"),
            Some("AroundCurve") => return Prompt::point_or_number("Radius").default_num(ctx.mem_num("Circle.Radius", 1.0)),
            Some("FitPoints") => return Prompt::objects("Select point objects to fit circle through", ObjFilter::Point, 3),
            _ => {}
        }
        match (self.mode, n) {
            (CircleMode::CenterRadius, 0) | (CircleMode::Vertical, 0) => Prompt::point("Center of circle").opts(first_opts),
            (CircleMode::CenterRadius, _) | (CircleMode::Vertical, _) => {
                let last = ctx.mem_num("Circle.Radius", 1.0);
                let (label, def) = match self.size_mode {
                    Some("Circumference") => ("Circumference", last * std::f64::consts::TAU),
                    Some("Area") => ("Area", last * last * std::f64::consts::PI),
                    _ => if self.diameter { ("Diameter", last * 2.0) } else { ("Radius", last) },
                };
                Prompt::point_or_number(label).default_num(def).opts(radius_opts)
            }
            (CircleMode::TwoPoint, 0) => Prompt::point("Start of diameter").opts(first_opts),
            (CircleMode::TwoPoint, _) => Prompt::point("End of diameter"),
            (CircleMode::ThreePoint, 0) => Prompt::point("First point on circle").opts(first_opts),
            (CircleMode::ThreePoint, 1) => Prompt::point("Second point on circle"),
            (CircleMode::ThreePoint, _) => Prompt::point("Third point on circle"),
        }
    }

    fn circle_plane(&self, center: DVec3, toward: DVec3) -> Plane {
        if self.mode == CircleMode::Vertical {
            vertical_plane(&self.plane, center, toward)
        } else {
            plane_at(&self.plane, center)
        }
    }

    fn build(&self, plane: &Plane, r: f64) -> tri_geom::Result<NurbsCurve> {
        let c = NurbsCurve::circle(plane, r)?;
        if self.deformable {
            let pts: Vec<DVec3> = (0..12).map(|i| c.point_at(i as f64 / 12.0)).collect();
            NurbsCurve::periodic(3, pts)
        } else {
            Ok(c)
        }
    }
}

impl Command for CircleCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.prompt(ctx).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                match o.as_str() {
                    "2Point" => self.mode = CircleMode::TwoPoint,
                    "3Point" => self.mode = CircleMode::ThreePoint,
                    "Vertical" => self.mode = CircleMode::Vertical,
                    "Deformable" => self.deformable = !self.deformable,
                    "Diameter" => {
                        self.diameter = true;
                        self.size_mode = None;
                    }
                    "Radius" => {
                        self.diameter = false;
                        self.size_mode = None;
                    }
                    "Circumference" => self.size_mode = Some("Circumference"),
                    "Area" => self.size_mode = Some("Area"),
                    "Tangent" => self.sub = Some("Tangent"),
                    "AroundCurve" => self.sub = Some("AroundCurve"),
                    "FitPoints" => self.sub = Some("FitPoints"),
                    other => not_implemented(ctx, other),
                }
                self.prompt(ctx).step()
            }
            Input::Pick(id, _) if self.sub.is_some() => {
                self.sub_curve = ctx.doc.get(id).and_then(|o| o.geometry.as_curve().cloned());
                if self.sub_curve.is_none() {
                    return Step::Cancelled;
                }
                self.prompt(ctx).step()
            }
            Input::Objects(ids) if self.sub == Some("FitPoints") => {
                let pts: Vec<DVec3> = ids.iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p) } else { None }).collect();
                match Self::fit_circle(&pts) {
                    Some(c) => add_curve(ctx, "Circle", Ok(c)),
                    None => {
                        ctx.log("Unable to fit a circle through the points.");
                        Step::Cancelled
                    }
                }
            }
            Input::Point(p) if self.sub.is_some() && self.pts.is_empty() => {
                // snap the picked point onto the curve
                let c = self.sub_curve.as_ref().unwrap();
                let t = c.closest_parameter(p);
                self.pts.push(c.point_at(t));
                ctx.mem_set("Circle.T", fmt_num(t));
                self.prompt(ctx).step()
            }
            inp @ (Input::Point(_) | Input::Number(_)) if self.sub.is_some() => {
                let c = self.sub_curve.as_ref().unwrap();
                let t = ctx.mem_num("Circle.T", 0.0);
                let on = self.pts[0];
                let tan = c.tangent_at(t);
                if self.sub == Some("AroundCurve") {
                    let plane = Plane::from_normal(on, tan);
                    let r = match inp { Input::Number(v) => self.radius_from_number(v), Input::Point(q) => (q - on).length(), _ => 1.0 };
                    ctx.mem_set("Circle.Radius", fmt_num(r));
                    return add_curve(ctx, "Circle", self.build(&plane, r));
                }
                // Tangent: centre lies along the in-plane normal of the curve at the touch point
                let nrm = self.plane.z_axis.cross(tan).normalize_or(self.plane.y_axis);
                let (r, side) = match inp {
                    Input::Number(v) => (self.radius_from_number(v), 1.0),
                    Input::Point(q) => {
                        let d = (q - on).dot(nrm);
                        // circle through q tangent at `on`: r = |q-on|² / (2 d)
                        let l2 = (q - on).length_squared();
                        if d.abs() < 1e-12 { (l2.sqrt() * 0.5, 1.0) } else { (l2 / (2.0 * d.abs()), d.signum()) }
                    }
                    _ => (1.0, 1.0),
                };
                ctx.mem_set("Circle.Radius", fmt_num(r));
                let center = on + nrm * (r * side);
                add_curve(ctx, "Circle", self.build(&plane_at(&self.plane, center), r))
            }
            Input::Point(p) => {
                let n = self.pts.len();
                match (self.mode, n) {
                    (CircleMode::CenterRadius, 0) | (CircleMode::Vertical, 0) => {
                        self.pts.push(p);
                        self.prompt(ctx).step()
                    }
                    (CircleMode::CenterRadius, _) | (CircleMode::Vertical, _) => {
                        let c = self.pts[0];
                        let r = (p - c).length();
                        ctx.mem_set("Circle.Radius", fmt_num(r));
                        let plane = self.circle_plane(c, p);
                        add_curve(ctx, "Circle", self.build(&plane, r))
                    }
                    (CircleMode::TwoPoint, 0) => {
                        self.pts.push(p);
                        self.prompt(ctx).step()
                    }
                    (CircleMode::TwoPoint, _) => {
                        let c = (self.pts[0] + p) * 0.5;
                        let r = (p - c).length();
                        add_curve(ctx, "Circle", self.build(&plane_at(&self.plane, c), r))
                    }
                    (CircleMode::ThreePoint, 0) | (CircleMode::ThreePoint, 1) => {
                        self.pts.push(p);
                        self.prompt(ctx).step()
                    }
                    (CircleMode::ThreePoint, _) => add_curve(ctx, "Circle", NurbsCurve::circle_3pt(self.pts[0], self.pts[1], p)),
                }
            }
            Input::Number(v) if !self.pts.is_empty() && matches!(self.mode, CircleMode::CenterRadius | CircleMode::Vertical) => {
                let r = self.radius_from_number(v);
                ctx.mem_set("Circle.Radius", fmt_num(r));
                let c = self.pts[0];
                let plane = self.circle_plane(c, c + self.plane.x_axis);
                add_curve(ctx, "Circle", self.build(&plane, r))
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(p) = cursor else { return vec![] };
        let n = self.pts.len();
        match (self.mode, n) {
            (CircleMode::CenterRadius, 1) | (CircleMode::Vertical, 1) => {
                let c = self.pts[0];
                self.build(&self.circle_plane(c, p), (p - c).length()).map(|cv| vec![Preview::Curve(cv), Preview::Line(c, p)]).unwrap_or_default()
            }
            (CircleMode::TwoPoint, 1) => {
                let c = (self.pts[0] + p) * 0.5;
                self.build(&plane_at(&self.plane, c), (p - c).length()).map(|cv| vec![Preview::Curve(cv), Preview::Line(self.pts[0], p)]).unwrap_or_default()
            }
            (CircleMode::ThreePoint, 1) => vec![Preview::Line(self.pts[0], p)],
            (CircleMode::ThreePoint, 2) => NurbsCurve::circle_3pt(self.pts[0], self.pts[1], p).map(|cv| vec![Preview::Curve(cv)]).unwrap_or_default(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ArcMode {
    Center,
    StartPoint,
    /// StartPoint mode after the Center option: start, end, then centre.
    EndCenter,
}

pub struct ArcCmd {
    mode: ArcMode,
    pts: Vec<DVec3>,
    plane: Plane,
}

impl ArcCmd {
    pub fn center() -> Self {
        Self { mode: ArcMode::Center, pts: Vec::new(), plane: Plane::world_xy() }
    }
    pub fn three_point() -> Self {
        Self { mode: ArcMode::StartPoint, pts: Vec::new(), plane: Plane::world_xy() }
    }

    fn prompt(&self) -> Prompt {
        let n = self.pts.len();
        match (self.mode, n) {
            (ArcMode::Center, 0) => Prompt::point("Center of arc").opts([opt("Deformable"), opt("StartPoint"), opt("Tangent"), opt("Extension")]),
            (ArcMode::Center, 1) => Prompt::point("Start of arc"),
            (ArcMode::Center, _) => Prompt::point_or_number("End of arc or angle").opts([opt("Length"), opt("Tilted")]),
            (ArcMode::StartPoint, 0) => Prompt::point("Start of arc").opts([opt("Deformable"), opt("Tangent"), opt("Extension")]),
            (ArcMode::StartPoint, 1) => Prompt::point("End of arc"),
            (ArcMode::StartPoint, _) => Prompt::point("Point on arc").opts([opt("Direction"), opt("ThroughPoint"), opt("Center")]),
            (ArcMode::EndCenter, _) => Prompt::point("Center of arc"),
        }
    }

    fn center_arc(&self, end_or_angle: Result<DVec3, f64>) -> tri_geom::Result<NurbsCurve> {
        let c = self.pts[0];
        let s = self.pts[1];
        let plane = Plane::from_frame(c, s - c, self.plane.y_axis);
        let plane = Plane { z_axis: self.plane.z_axis, y_axis: self.plane.z_axis.cross(plane.x_axis), ..plane };
        let r = (s - c).length();
        let angle = match end_or_angle {
            Ok(e) => {
                let (u, v, _) = plane.closest_parameters(e);
                let mut a = v.atan2(u);
                if a < 0.0 {
                    a += std::f64::consts::TAU;
                }
                a
            }
            Err(deg) => deg.to_radians(),
        };
        NurbsCurve::arc(&plane, r, 0.0, angle)
    }
}

impl Command for ArcCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                match o.as_str() {
                    "StartPoint" => self.mode = ArcMode::StartPoint,
                    "Center" if self.mode == ArcMode::StartPoint && self.pts.len() == 2 => {
                        // Arc StartPoint, Center option: start, end, then center
                        self.mode = ArcMode::EndCenter;
                        return Prompt::point("Center of arc").step();
                    }
                    other => not_implemented(ctx, other),
                }
                self.prompt().step()
            }
            Input::Point(p) if self.mode == ArcMode::EndCenter => {
                let (s, e) = (self.pts[0], self.pts[1]);
                let r = (s - p).length();
                let plane = Plane::from_frame(p, s - p, self.plane.y_axis);
                let plane = Plane { z_axis: self.plane.z_axis, y_axis: self.plane.z_axis.cross(plane.x_axis), ..plane };
                let (u, v, _) = plane.closest_parameters(e);
                let mut a = v.atan2(u);
                if a < 0.0 {
                    a += std::f64::consts::TAU;
                }
                add_curve(ctx, "Arc", NurbsCurve::arc(&plane, r, 0.0, a))
            }
            Input::Point(p) => {
                let n = self.pts.len();
                match (self.mode, n) {
                    (ArcMode::Center, 2) => add_curve(ctx, "Arc", self.center_arc(Ok(p))),
                    (ArcMode::StartPoint, 2) => add_curve(ctx, "Arc", NurbsCurve::arc_3pt(self.pts[0], p, self.pts[1])),
                    _ => {
                        self.pts.push(p);
                        self.prompt().step()
                    }
                }
            }
            Input::Number(a) if self.mode == ArcMode::Center && self.pts.len() == 2 => add_curve(ctx, "Arc", self.center_arc(Err(a))),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(p) = cursor else { return vec![] };
        match (self.mode, self.pts.len()) {
            (_, 1) => vec![Preview::Line(self.pts[0], p)],
            (ArcMode::Center, 2) => self.center_arc(Ok(p)).map(|c| vec![Preview::Curve(c), Preview::Line(self.pts[0], p)]).unwrap_or_default(),
            (ArcMode::StartPoint, 2) => NurbsCurve::arc_3pt(self.pts[0], p, self.pts[1]).map(|c| vec![Preview::Curve(c)]).unwrap_or_default(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[derive(Default)]
pub struct EllipseCmd {
    pts: Vec<DVec3>,
    plane: Plane,
}

impl EllipseCmd {
    fn build(&self, second: DVec3) -> tri_geom::Result<NurbsCurve> {
        let c = self.pts[0];
        let a = self.pts[1] - c;
        let plane = Plane::from_frame(c, a, self.plane.y_axis);
        let plane = Plane { z_axis: self.plane.z_axis, y_axis: self.plane.z_axis.cross(plane.x_axis), ..plane };
        let b = (second - c).dot(plane.y_axis).abs();
        NurbsCurve::ellipse(&plane, a.length(), b)
    }
}

impl Command for EllipseCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::point("Ellipse center").opts([opt("Corner"), opt("Diameter"), opt("FromFoci"), opt("AroundCurve"), opt("Vertical"), opt("Deformable")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                not_implemented(ctx, &o);
                Prompt::point("Ellipse center").step()
            }
            Input::Point(p) => match self.pts.len() {
                0 => {
                    self.pts.push(p);
                    Prompt::point("End of first axis").step()
                }
                1 => {
                    self.pts.push(p);
                    Prompt::point_or_number("End of second axis").step()
                }
                _ => add_curve(ctx, "Ellipse", self.build(p)),
            },
            Input::Number(b) if self.pts.len() == 2 => {
                let c = self.pts[0];
                let plane = Plane::from_frame(c, self.pts[1] - c, self.plane.y_axis);
                add_curve(ctx, "Ellipse", self.build(c + plane.y_axis * b))
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(p) = cursor else { return vec![] };
        match self.pts.len() {
            1 => vec![Preview::Line(self.pts[0], p)],
            2 => self.build(p).map(|c| vec![Preview::Curve(c)]).unwrap_or_default(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PolygonMode {
    Inscribed,
    Circumscribed,
    Edge,
    Star,
}

pub struct PolygonCmd {
    sides: usize,
    mode: PolygonMode,
    pts: Vec<DVec3>,
    plane: Plane,
    asking: Option<&'static str>,
}

impl Default for PolygonCmd {
    fn default() -> Self {
        Self { sides: 6, mode: PolygonMode::Inscribed, pts: Vec::new(), plane: Plane::world_xy(), asking: None }
    }
}

impl PolygonCmd {
    fn mode_name(&self) -> &'static str {
        match self.mode {
            PolygonMode::Inscribed => "Inscribed",
            PolygonMode::Circumscribed => "Circumscribed",
            PolygonMode::Edge => "Edge",
            PolygonMode::Star => "Star",
        }
    }

    fn prompt(&self) -> Prompt {
        let opts = [optv("NumSides", self.sides), optv("Mode", self.mode_name()), opt("Vertical"), opt("AroundCurve")];
        match (self.mode, self.pts.len()) {
            (PolygonMode::Edge, 0) => Prompt::point("Start of edge").opts(opts),
            (PolygonMode::Edge, _) => Prompt::point("End of edge").opts([opt("Flip")]),
            (_, 0) => Prompt::point("Center of polygon").opts(opts),
            (PolygonMode::Inscribed, _) => Prompt::point("Corner of polygon").opts([optv("NumSides", self.sides)]),
            (PolygonMode::Circumscribed, _) => Prompt::point("Midpoint of edge").opts([optv("NumSides", self.sides)]),
            (PolygonMode::Star, 1) => Prompt::point("Corner of star").opts([optv("NumSides", self.sides)]),
            (PolygonMode::Star, _) => Prompt::point("Radius of second star").opts([opt("Automatic")]),
        }
    }

    fn points(&self, p: DVec3, inner: Option<DVec3>) -> Vec<DVec3> {
        let n = self.sides.max(3);
        let tau = std::f64::consts::TAU;
        match self.mode {
            PolygonMode::Edge => {
                let a = self.pts[0];
                let edge = p - a;
                let len = edge.length();
                if len < 1e-12 {
                    return vec![];
                }
                let plane = Plane { origin: a, x_axis: edge / len, y_axis: self.plane.z_axis.cross(edge / len), z_axis: self.plane.z_axis };
                let r = len / (2.0 * (std::f64::consts::PI / n as f64).sin());
                let apothem = r * (std::f64::consts::PI / n as f64).cos();
                let c = plane.point_at(len * 0.5, apothem);
                let a0 = (a - c).dot(plane.y_axis).atan2((a - c).dot(plane.x_axis));
                (0..=n).map(|i| {
                    let ang = a0 + tau * i as f64 / n as f64;
                    c + plane.x_axis * (r * ang.cos()) + plane.y_axis * (r * ang.sin())
                }).collect()
            }
            _ => {
                let c = self.pts[0];
                let plane = plane_at(&self.plane, c);
                let (u, v, _) = plane.closest_parameters(p);
                let mut r = (u * u + v * v).sqrt();
                let a0 = v.atan2(u);
                if self.mode == PolygonMode::Circumscribed {
                    r /= (std::f64::consts::PI / n as f64).cos();
                }
                let mut pts = Vec::new();
                if self.mode == PolygonMode::Star {
                    let r2 = inner.map_or(r * 0.5, |q| (q - c).length());
                    for i in 0..n {
                        let ang = a0 + tau * i as f64 / n as f64;
                        let ang2 = ang + tau * 0.5 / n as f64;
                        pts.push(plane.point_at(r * ang.cos(), r * ang.sin()));
                        pts.push(plane.point_at(r2 * ang2.cos(), r2 * ang2.sin()));
                    }
                    pts.push(pts[0]);
                    return pts;
                }
                let offset = if self.mode == PolygonMode::Circumscribed { std::f64::consts::PI / n as f64 } else { 0.0 };
                for i in 0..=n {
                    let ang = a0 + offset + tau * i as f64 / n as f64;
                    pts.push(plane.point_at(r * ang.cos(), r * ang.sin()));
                }
                pts
            }
        }
    }
}

impl Command for PolygonCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.sides = ctx.mem_num("Polygon.NumSides", 6.0) as usize;
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => match o.as_str() {
                "NumSides" => {
                    self.asking = Some("NumSides");
                    Prompt::number("Number of sides").default_num(self.sides as f64).step()
                }
                "Mode" => {
                    self.asking = Some("Mode");
                    Prompt::options("Polygon mode").opts([opt("Inscribed"), opt("Circumscribed"), opt("Edge"), opt("Star")]).step()
                }
                "Inscribed" | "Circumscribed" | "Edge" | "Star" if self.asking == Some("Mode") => {
                    self.asking = None;
                    self.mode = match o.as_str() {
                        "Inscribed" => PolygonMode::Inscribed,
                        "Circumscribed" => PolygonMode::Circumscribed,
                        "Edge" => PolygonMode::Edge,
                        _ => PolygonMode::Star,
                    };
                    self.prompt().step()
                }
                other => {
                    not_implemented(ctx, other);
                    self.prompt().step()
                }
            },
            Input::Number(n) if self.asking == Some("NumSides") => {
                self.asking = None;
                self.sides = (n.round() as usize).clamp(3, 360);
                ctx.mem_set("Polygon.NumSides", self.sides);
                self.prompt().step()
            }
            Input::Empty if self.asking.is_some() => {
                self.asking = None;
                self.prompt().step()
            }
            Input::Point(p) => match (self.mode, self.pts.len()) {
                (PolygonMode::Star, 2) => {
                    let pts = self.points(self.pts[1], Some(p));
                    add_curve(ctx, "Polygon", NurbsCurve::polyline(pts))
                }
                (PolygonMode::Star, 1) => {
                    self.pts.push(p);
                    self.prompt().step()
                }
                (_, 0) => {
                    self.pts.push(p);
                    self.prompt().step()
                }
                _ => {
                    let pts = self.points(p, None);
                    add_curve(ctx, "Polygon", NurbsCurve::polyline(pts))
                }
            },
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(p) = cursor else { return vec![] };
        match (self.mode, self.pts.len()) {
            (PolygonMode::Star, 2) => vec![Preview::Polyline(self.points(self.pts[1], Some(p)))],
            (_, 1) if (p - self.pts[0]).length() > 1e-9 => vec![Preview::Polyline(self.points(p, None))],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

pub struct CurveCmd {
    interpolate: bool,
    degree: usize,
    points: Vec<DVec3>,
    persistent_close: bool,
    knots: usize,
    asking: Option<&'static str>,
}

impl CurveCmd {
    pub fn control_points() -> Self {
        Self { interpolate: false, degree: 3, points: Vec::new(), persistent_close: false, knots: 2, asking: None }
    }
    pub fn interpolated() -> Self {
        Self { interpolate: true, ..Self::control_points() }
    }

    fn knots_name(&self) -> &'static str {
        ["Uniform", "Chord", "SqrtChord"][self.knots.min(2)]
    }

    fn prompt(&self) -> Prompt {
        let n = self.points.len();
        let mut opts = vec![optv("Degree", self.degree)];
        if self.interpolate {
            opts.push(optv("Knots", self.knots_name()));
        }
        opts.push(optv("PersistentClose", yesno(self.persistent_close)));
        if n == 0 {
            if self.interpolate {
                opts.push(opt("StartTangent"));
            }
            opts.push(optv("SubDFriendly", "No"));
            return Prompt::point("Start of curve").opts(opts);
        }
        if n >= 3 {
            opts.push(opt("Close"));
            opts.push(opt("Sharp"));
        }
        if self.interpolate && n >= 2 {
            opts.push(opt("EndTangent"));
        }
        opts.push(opt("Undo"));
        let msg = if n >= 2 {
            if self.interpolate { "Next point of curve. Press Enter when done" } else { "Next point. Press Enter when done" }
        } else if self.interpolate {
            "Next point of curve"
        } else {
            "Next point"
        };
        Prompt::point(msg).opts(opts)
    }

    fn build(&self, pts: &[DVec3], close: Option<bool>) -> tri_geom::Result<NurbsCurve> {
        let smooth_close = close == Some(true);
        let sharp_close = close == Some(false);
        if self.interpolate {
            let mut p = pts.to_vec();
            if smooth_close || sharp_close {
                p.push(p[0]);
            }
            let n = p.len();
            let params: Vec<f64> = match self.knots {
                0 => (0..n).map(|i| i as f64 / (n - 1) as f64).collect(),
                k => {
                    let mut acc = vec![0.0; n];
                    for i in 1..n {
                        let d = (p[i] - p[i - 1]).length();
                        acc[i] = acc[i - 1] + if k == 2 { d.sqrt() } else { d };
                    }
                    let total = acc[n - 1].max(1e-12);
                    acc.iter().map(|a| a / total).collect()
                }
            };
            return NurbsCurve::interpolate_params(&p, self.degree, &params);
        }
        if smooth_close {
            return NurbsCurve::periodic(self.degree, pts.to_vec());
        }
        let mut p = pts.to_vec();
        if sharp_close {
            p.push(p[0]);
        }
        let d = self.degree.min(p.len() - 1).max(1);
        NurbsCurve::from_control_points(d, p)
    }

    fn finish(&mut self, ctx: &mut CmdCtx, close: Option<bool>) -> Step {
        if self.points.len() < 2 {
            return Step::Cancelled;
        }
        let pts = std::mem::take(&mut self.points);
        let close = if self.persistent_close && close.is_none() && pts.len() >= 3 { Some(true) } else { close };
        let label = if self.interpolate { "InterpCrv" } else { "Curve" };
        add_curve(ctx, label, self.build(&pts, close))
    }
}

impl Command for CurveCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.degree = ctx.mem_num("Curve.Degree", 3.0) as usize;
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.points.push(p);
                self.prompt().step()
            }
            Input::Option(o) => match o.as_str() {
                "Degree" => {
                    self.asking = Some("Degree");
                    Prompt::number("Degree").default_num(self.degree as f64).step()
                }
                "Knots" => {
                    self.knots = (self.knots + 1) % 3;
                    self.prompt().step()
                }
                "PersistentClose" => {
                    self.persistent_close = !self.persistent_close;
                    self.prompt().step()
                }
                "Close" => self.finish(ctx, Some(true)),
                "Sharp" => self.finish(ctx, Some(false)),
                "Undo" => {
                    self.points.pop();
                    self.prompt().step()
                }
                other => {
                    not_implemented(ctx, other);
                    self.prompt().step()
                }
            },
            Input::Number(d) if self.asking == Some("Degree") => {
                self.asking = None;
                self.degree = (d.round() as usize).clamp(1, 11);
                ctx.mem_set("Curve.Degree", self.degree);
                self.prompt().step()
            }
            Input::Empty if self.asking.is_some() => {
                self.asking = None;
                self.prompt().step()
            }
            Input::Empty => self.finish(ctx, None),
            Input::Text(_) => self.prompt().step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let mut pts = self.points.clone();
        if let Some(c) = cursor {
            pts.push(c);
        }
        if pts.len() < 2 {
            return vec![];
        }
        let mut out = vec![];
        if !self.interpolate {
            out.push(Preview::Polyline(pts.clone()));
        }
        let close = if self.persistent_close && pts.len() >= 3 { Some(true) } else { None };
        if let Ok(c) = self.build(&pts, close) {
            out.push(Preview::Curve(c));
        }
        out
    }
    fn base_point(&self) -> Option<DVec3> {
        self.points.last().copied()
    }
}

#[derive(Default)]
pub struct SrfPtCmd {
    pts: Vec<DVec3>,
}

impl Command for SrfPtCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("First corner of surface").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("Second corner of surface").step(),
                    2 => Prompt::point("Third corner of surface").step(),
                    3 => Prompt::point("Fourth corner of surface. Press Enter to make triangular surface").step(),
                    _ => {
                        let p = &self.pts;
                        add_surface(ctx, "SrfPt", NurbsSurface::from_grid(1, 1, vec![vec![p[0], p[3]], vec![p[1], p[2]]]))
                    }
                }
            }
            Input::Empty if self.pts.len() == 3 => {
                let p = &self.pts;
                add_surface(ctx, "SrfPt", NurbsSurface::from_grid(1, 1, vec![vec![p[0], p[2]], vec![p[1], p[2]]]))
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(c) = cursor else { return vec![] };
        let mut pts = self.pts.clone();
        pts.push(c);
        if pts.len() >= 3 {
            pts.push(pts[0]);
            vec![Preview::Polyline(pts)]
        } else if pts.len() == 2 {
            vec![Preview::Line(pts[0], pts[1])]
        } else {
            vec![]
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[derive(Default)]
pub struct PlaneCmd {
    corner: Option<DVec3>,
    plane: Plane,
}

fn plane_surface(cplane: &Plane, a: DVec3, b: DVec3) -> tri_geom::Result<NurbsSurface> {
    let (u0, v0, w) = cplane.closest_parameters(a);
    let (u1, v1, _) = cplane.closest_parameters(b);
    let p = Plane { origin: cplane.origin + cplane.z_axis * w, ..*cplane };
    NurbsSurface::plane(&p, u0.min(u1), u0.max(u1), v0.min(v1), v0.max(v1))
}

impl Command for PlaneCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        Prompt::point("First corner of plane").opts([opt("3Point"), opt("Vertical"), opt("Center"), opt("Deformable")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.corner, input) {
            (_, Input::Option(o)) => {
                not_implemented(ctx, &o);
                Prompt::point("First corner of plane").step()
            }
            (None, Input::Point(p)) => {
                self.corner = Some(p);
                Prompt::point("Other corner or length").step()
            }
            (Some(a), Input::Point(b)) => add_surface(ctx, "Plane", plane_surface(&self.plane, a, b)),
            (Some(a), Input::Number(l)) => add_surface(ctx, "Plane", plane_surface(&self.plane, a, a + self.plane.x_axis * l + self.plane.y_axis * l)),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.corner, cursor) {
            (Some(a), Some(b)) => vec![Preview::Polyline(rect_points(&self.plane, a, b))],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.corner
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoxMode {
    Corner,
    Diagonal,
    Center,
    Cube,
}

pub struct BoxCmd {
    mode: BoxMode,
    corner: Option<DVec3>,
    other: Option<DVec3>,
    plane: Plane,
}

impl Default for BoxCmd {
    fn default() -> Self {
        Self { mode: BoxMode::Corner, corner: None, other: None, plane: Plane::world_xy() }
    }
}

fn box_brep(cplane: &Plane, a: DVec3, b: DVec3, h: f64) -> tri_geom::Result<Brep> {
    let (u0, v0, w) = cplane.closest_parameters(a);
    let (u1, v1, _) = cplane.closest_parameters(b);
    let base = Plane { origin: cplane.point_at_3(u0, v0, w), ..*cplane };
    Brep::cuboid(&base, u1 - u0, v1 - v0, h)
}

impl BoxCmd {
    fn prompt(&self) -> Prompt {
        let opts = [opt("Diagonal"), opt("3Point"), opt("Vertical"), opt("Center")];
        match (self.mode, self.corner, self.other) {
            (BoxMode::Center, None, _) => Prompt::point("Center of base").opts(opts),
            (BoxMode::Diagonal, None, _) | (BoxMode::Cube, None, _) => Prompt::point("First corner").opts(opts),
            (_, None, _) => Prompt::point("First corner of base").opts(opts),
            (BoxMode::Diagonal, Some(_), _) | (BoxMode::Cube, Some(_), _) => Prompt::point("Other corner"),
            (_, Some(_), None) => Prompt::point_or_number("Other corner of base or length"),
            (_, Some(a), Some(b)) => {
                let w = (b - a).dot(self.plane.x_axis).abs();
                Prompt::point_or_number("Height. Press Enter to use width").default_num(w)
            }
        }
    }

    fn base_corners(&self, a: DVec3, b: DVec3) -> (DVec3, DVec3) {
        match self.mode {
            BoxMode::Center => (a - (b - a), b),
            _ => (a, b),
        }
    }
}

impl Command for BoxCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.corner, self.other, input) {
            (_, _, Input::Option(o)) => {
                match o.as_str() {
                    "Diagonal" => self.mode = BoxMode::Diagonal,
                    "Center" => self.mode = BoxMode::Center,
                    "Cube" => self.mode = BoxMode::Cube,
                    other => not_implemented(ctx, other),
                }
                self.prompt().step()
            }
            (None, _, Input::Point(p)) => {
                self.corner = Some(p);
                self.prompt().step()
            }
            (Some(a), None, Input::Point(p)) if matches!(self.mode, BoxMode::Diagonal | BoxMode::Cube) => {
                let (u0, v0, w0) = self.plane.closest_parameters(a);
                let (u1, v1, w1) = self.plane.closest_parameters(p);
                if self.mode == BoxMode::Cube {
                    let s = (u1 - u0).abs().max((v1 - v0).abs()).max(1e-9);
                    let sx = (u1 - u0).signum();
                    let sy = (v1 - v0).signum();
                    let b = self.plane.point_at_3(u0 + s * sx, v0 + s * sy, w0);
                    return add_brep(ctx, "Box", box_brep(&self.plane, a, b, s));
                }
                let b = self.plane.point_at_3(u1, v1, w0);
                add_brep(ctx, "Box", box_brep(&self.plane, a, b, w1 - w0))
            }
            (Some(a), None, Input::Point(p)) => {
                let (a2, b2) = self.base_corners(a, p);
                self.corner = Some(a2);
                self.other = Some(b2);
                self.prompt().step()
            }
            (Some(a), None, Input::Number(l)) => {
                let b = a + self.plane.x_axis * l + self.plane.y_axis * l;
                let (a2, b2) = self.base_corners(a, b);
                self.corner = Some(a2);
                self.other = Some(b2);
                self.prompt().step()
            }
            (Some(a), Some(b), input) => {
                let h = match &input {
                    Input::Empty => Some((b - a).dot(self.plane.x_axis).abs()),
                    other => height_from(&self.plane, b, other),
                };
                match h {
                    Some(h) if h.abs() > 1e-9 => add_brep(ctx, "Box", box_brep(&self.plane, a, b, h)),
                    _ => Step::Cancelled,
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.corner, self.other, cursor) {
            (Some(a), None, Some(b)) if matches!(self.mode, BoxMode::Diagonal | BoxMode::Cube) => {
                let (_, _, w0) = self.plane.closest_parameters(a);
                let (u1, v1, w1) = self.plane.closest_parameters(b);
                let bb = self.plane.point_at_3(u1, v1, w0);
                box_brep(&self.plane, a, bb, w1 - w0).map(|br| vec![Preview::Brep(br)]).unwrap_or_default()
            }
            (Some(a), None, Some(b)) => {
                let (a2, b2) = self.base_corners(a, b);
                vec![Preview::Polyline(rect_points(&self.plane, a2, b2))]
            }
            (Some(a), Some(b), Some(c)) => {
                let h = (c - b).dot(self.plane.z_axis);
                let mut out = vec![Preview::Polyline(rect_points(&self.plane, a, b))];
                if let Ok(brep) = box_brep(&self.plane, a, b, h) {
                    out.push(Preview::Brep(brep));
                }
                out
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.other.or(self.corner)
    }
}

#[derive(Default)]
pub struct SphereCmd {
    center: Option<DVec3>,
    diameter: bool,
    pts: Vec<DVec3>,
    mode: u8,
}

impl SphereCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        match (self.mode, self.center, self.pts.len()) {
            (0, None, _) => Prompt::point("Center of sphere").opts([opt("2Point"), opt("3Point"), opt("Tangent"), opt("AroundCurve"), opt("4Point"), opt("FitPoints")]),
            (0, Some(_), _) => {
                let last = ctx.mem_num("Sphere.Radius", 1.0);
                let (label, d) = if self.diameter { ("Diameter", last * 2.0) } else { ("Radius", last) };
                Prompt::point_or_number(label).default_num(d).opts([opt(if self.diameter { "Radius" } else { "Diameter" }), opt("Orientation"), opt("Circumference"), opt("Area")])
            }
            (2, _, 0) => Prompt::point("Start of diameter"),
            (2, _, _) => Prompt::point("End of diameter"),
            (_, _, 0) => Prompt::point("First point on sphere"),
            (_, _, 1) => Prompt::point("Second point on sphere"),
            _ => Prompt::point("Third point on sphere"),
        }
    }
}

impl Command for SphereCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.prompt(ctx).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                match o.as_str() {
                    "2Point" => self.mode = 2,
                    "3Point" => self.mode = 3,
                    "Diameter" => self.diameter = true,
                    "Radius" => self.diameter = false,
                    other => not_implemented(ctx, other),
                }
                self.prompt(ctx).step()
            }
            Input::Point(p) => match (self.mode, self.center) {
                (0, None) => {
                    self.center = Some(p);
                    self.prompt(ctx).step()
                }
                (0, Some(c)) => {
                    let r = (p - c).length();
                    ctx.mem_set("Sphere.Radius", fmt_num(r));
                    add_surface(ctx, "Sphere", NurbsSurface::sphere(c, r))
                }
                (2, _) => {
                    if self.pts.is_empty() {
                        self.pts.push(p);
                        return self.prompt(ctx).step();
                    }
                    let c = (self.pts[0] + p) * 0.5;
                    add_surface(ctx, "Sphere", NurbsSurface::sphere(c, (p - c).length()))
                }
                _ => {
                    self.pts.push(p);
                    if self.pts.len() < 3 {
                        return self.prompt(ctx).step();
                    }
                    match tri_geom::curve::circle_through(self.pts[0], self.pts[1], self.pts[2]) {
                        Ok((c, r, _)) => add_surface(ctx, "Sphere", NurbsSurface::sphere(c, r)),
                        Err(e) => {
                            ctx.log(format!("Sphere failed: {e}"));
                            Step::Cancelled
                        }
                    }
                }
            },
            Input::Number(v) if self.center.is_some() => {
                let r = if self.diameter { v.abs() * 0.5 } else { v.abs() };
                ctx.mem_set("Sphere.Radius", fmt_num(r));
                add_surface(ctx, "Sphere", NurbsSurface::sphere(self.center.unwrap(), r))
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let Some(p) = cursor else { return vec![] };
        let (c, r) = match (self.mode, self.center) {
            (0, Some(c)) => (c, (p - c).length()),
            (2, _) if self.pts.len() == 1 => {
                let c = (self.pts[0] + p) * 0.5;
                (c, (p - c).length())
            }
            _ => return vec![],
        };
        let mut out = vec![Preview::Line(c, p)];
        for pl in [Plane::world_xy(), Plane::world_xz(), Plane::world_yz()] {
            if let Ok(cv) = NurbsCurve::circle(&Plane { origin: c, ..pl }, r) {
                out.push(Preview::Curve(cv));
            }
        }
        out
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center.or(self.pts.last().copied())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Solid {
    Cylinder,
    Cone,
}

pub struct CylinderCmd {
    kind: Solid,
    base: Option<DVec3>,
    radius: Option<f64>,
    plane: Plane,
    vertical: bool,
    solid: bool,
    both_sides: bool,
    diameter: bool,
}

impl CylinderCmd {
    pub fn new(kind: Solid) -> Self {
        Self { kind, base: None, radius: None, plane: Plane::world_xy(), vertical: true, solid: true, both_sides: false, diameter: false }
    }
    fn label(&self) -> &'static str {
        match self.kind {
            Solid::Cylinder => "Cylinder",
            Solid::Cone => "Cone",
        }
    }
    fn lower(&self) -> &'static str {
        match self.kind {
            Solid::Cylinder => "cylinder",
            Solid::Cone => "cone",
        }
    }
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        let dc = optv("DirectionConstraint", if self.vertical { "Vertical" } else { "None" });
        match (self.base, self.radius) {
            (None, _) => Prompt::point(format!("Base of {}", self.lower())).opts([dc, optv("Solid", yesno(self.solid)), opt("2Point"), opt("3Point"), opt("Tangent"), opt("FitPoints")]),
            (Some(_), None) => {
                let last = ctx.mem_num(&format!("{}.Radius", self.label()), 1.0);
                let (label, d) = if self.diameter { ("Diameter", last * 2.0) } else { ("Radius", last) };
                Prompt::point_or_number(label).default_num(d).opts([opt(if self.diameter { "Radius" } else { "Diameter" }), opt("Circumference"), opt("Area")])
            }
            (Some(_), Some(_)) => {
                let last = ctx.mem_num(&format!("{}.Height", self.label()), 1.0);
                Prompt::point_or_number(format!("End of {}", self.lower())).default_num(last).opts([dc, optv("BothSides", yesno(self.both_sides)), optv("Solid", yesno(self.solid))])
            }
        }
    }
    fn build(&self, base: DVec3, r: f64, h: f64) -> tri_geom::Result<Geometry> {
        let (base, h) = if self.both_sides { (base - self.plane.z_axis * h, h * 2.0) } else { (base, h) };
        let p = plane_at(&self.plane, base);
        let side = match self.kind {
            Solid::Cylinder => NurbsSurface::cylinder(&p, r, h)?,
            Solid::Cone => NurbsSurface::cone(&p, r, h)?,
        };
        if !self.solid {
            return Ok(Geometry::Surface(side));
        }
        let mut faces = vec![Face::untrimmed(side)];
        let bottom = NurbsCurve::circle(&p, r)?;
        faces.push(Face::from_planar_curves(std::slice::from_ref(&bottom), 1e-3 * (1.0 + r))?);
        if self.kind == Solid::Cylinder {
            let top = NurbsCurve::circle(&Plane { origin: base + self.plane.z_axis * h, ..p }, r)?;
            faces.push(Face::from_planar_curves(&[top], 1e-3 * (1.0 + r))?);
        }
        let mut b = Brep::new(faces);
        b.orient_outward();
        b.closed = true;
        Ok(Geometry::Brep(b))
    }
}

impl Command for CylinderCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.solid = ctx.mem_bool(&format!("{}.Solid", self.label()), true);
        self.prompt(ctx).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                match o.as_str() {
                    "DirectionConstraint" => self.vertical = !self.vertical,
                    "Solid" => {
                        self.solid = !self.solid;
                        let k = format!("{}.Solid", self.label());
                        ctx.mem_set(&k, yesno(self.solid));
                    }
                    "BothSides" => self.both_sides = !self.both_sides,
                    "Diameter" => self.diameter = true,
                    "Radius" => self.diameter = false,
                    other => not_implemented(ctx, other),
                }
                self.prompt(ctx).step()
            }
            input => match (self.base, self.radius, &input) {
                (None, _, Input::Point(p)) => {
                    self.base = Some(*p);
                    self.prompt(ctx).step()
                }
                (Some(b), None, inp) => {
                    let r = match inp {
                        Input::Number(n) => {
                            if self.diameter {
                                n.abs() * 0.5
                            } else {
                                n.abs()
                            }
                        }
                        Input::Point(p) => (*p - b).length(),
                        _ => return Step::Cancelled,
                    };
                    if r <= 1e-9 {
                        return Step::Cancelled;
                    }
                    ctx.mem_set(&format!("{}.Radius", self.label()), fmt_num(r));
                    self.radius = Some(r);
                    self.prompt(ctx).step()
                }
                (Some(b), Some(r), inp) => {
                    let h = match inp {
                        Input::Number(n) => *n,
                        Input::Point(p) => {
                            if self.vertical {
                                (*p - b).dot(self.plane.z_axis)
                            } else {
                                // free direction: use the picked point as the end of the axis
                                let d = *p - b;
                                let len = d.length();
                                if len > 1e-9 {
                                    self.plane = Plane::from_normal(b, d);
                                }
                                len
                            }
                        }
                        _ => return Step::Cancelled,
                    };
                    if h.abs() < 1e-9 {
                        return Step::Cancelled;
                    }
                    ctx.mem_set(&format!("{}.Height", self.label()), fmt_num(h));
                    let label = self.label();
                    match self.build(b, r, h) {
                        Ok(g) => {
                            add_geometry(ctx, label, g);
                            Step::Done
                        }
                        Err(e) => {
                            ctx.log(format!("{label} failed: {e}"));
                            Step::Cancelled
                        }
                    }
                }
                _ => Step::Cancelled,
            },
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.base, self.radius, cursor) {
            (Some(b), None, Some(p)) => {
                let r = (p - b).length();
                NurbsCurve::circle(&plane_at(&self.plane, b), r).map(|c| vec![Preview::Curve(c), Preview::Line(b, p)]).unwrap_or_default()
            }
            (Some(b), Some(r), Some(p)) => {
                let h = (p - b).dot(self.plane.z_axis);
                match self.build(b, r, h) {
                    Ok(Geometry::Surface(s)) => vec![Preview::Surface(s)],
                    Ok(Geometry::Brep(br)) => vec![Preview::Brep(br)],
                    _ => vec![],
                }
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

#[derive(Default)]
pub struct TorusCmd {
    center: Option<DVec3>,
    major: Option<f64>,
    plane: Plane,
    diameter: bool,
}

impl TorusCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        match (self.center, self.major) {
            (None, _) => Prompt::point("Center of torus").opts([opt("2Point"), opt("3Point"), opt("Tangent"), opt("AroundCurve"), opt("FitPoints")]),
            (Some(_), None) => {
                let last = ctx.mem_num("Torus.Radius", 2.0);
                let (label, d) = if self.diameter { ("Diameter", last * 2.0) } else { ("Radius", last) };
                Prompt::point_or_number(label).default_num(d).opts([opt(if self.diameter { "Radius" } else { "Diameter" }), opt("Orientation"), opt("Circumference"), opt("Area")])
            }
            (Some(_), Some(_)) => {
                let last = ctx.mem_num("Torus.SecondRadius", 0.5);
                Prompt::point_or_number("Second radius").default_num(last).opts([opt("Diameter")])
            }
        }
    }
}

impl Command for TorusCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.plane = ctx.cplane;
        self.prompt(ctx).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                match o.as_str() {
                    "Diameter" => self.diameter = true,
                    "Radius" => self.diameter = false,
                    other => not_implemented(ctx, other),
                }
                self.prompt(ctx).step()
            }
            input => match (self.center, self.major, &input) {
                (None, _, Input::Point(p)) => {
                    self.center = Some(*p);
                    self.prompt(ctx).step()
                }
                (Some(c), None, inp) => {
                    let r = match inp {
                        Input::Number(n) => {
                            if self.diameter {
                                n.abs() * 0.5
                            } else {
                                n.abs()
                            }
                        }
                        Input::Point(p) => (*p - c).length(),
                        _ => return Step::Cancelled,
                    };
                    if r <= 1e-9 {
                        return Step::Cancelled;
                    }
                    ctx.mem_set("Torus.Radius", fmt_num(r));
                    self.major = Some(r);
                    self.prompt(ctx).step()
                }
                (Some(c), Some(big), inp) => {
                    let minor = match inp {
                        Input::Number(n) => n.abs(),
                        Input::Point(p) => ((*p - c).length() - big).abs(),
                        _ => return Step::Cancelled,
                    };
                    ctx.mem_set("Torus.SecondRadius", fmt_num(minor));
                    add_surface(ctx, "Torus", NurbsSurface::torus(&plane_at(&self.plane, c), big, minor))
                }
                _ => Step::Cancelled,
            },
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.center, self.major, cursor) {
            (Some(c), None, Some(p)) => NurbsCurve::circle(&plane_at(&self.plane, c), (p - c).length()).map(|cv| vec![Preview::Curve(cv), Preview::Line(c, p)]).unwrap_or_default(),
            (Some(c), Some(big), Some(p)) => {
                let minor = ((p - c).length() - big).abs();
                NurbsSurface::torus(&plane_at(&self.plane, c), big, minor).map(|s| vec![Preview::Surface(s)]).unwrap_or_default()
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.center
    }
}

pub struct ExtrudeCmd {
    curves: Vec<ObjectId>,
    both_sides: bool,
    solid: bool,
    delete_input: bool,
    direction: Option<DVec3>,
    dir_pts: Vec<DVec3>,
    base_point: Option<DVec3>,
    asking: Option<&'static str>,
    cplane: Plane,
}

impl Default for ExtrudeCmd {
    fn default() -> Self {
        Self { curves: Vec::new(), both_sides: false, solid: true, delete_input: false, direction: None, dir_pts: Vec::new(), base_point: None, asking: None, cplane: Plane::world_xy() }
    }
}

impl ExtrudeCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        let last = ctx.mem_num("ExtrudeCrv.Distance", 1.0);
        Prompt::point_or_number("Extrusion distance").default_num(last).opts([
            opt("Direction"),
            optv("BothSides", yesno(self.both_sides)),
            optv("Solid", yesno(self.solid)),
            optv("DeleteInput", yesno(self.delete_input)),
            opt("ToBoundary"),
            opt("SetBasePoint"),
        ])
    }

    /// Extrusion direction for a curve: normal of a planar curve, else the CPlane z axis.
    fn dir_for(&self, c: &NurbsCurve) -> DVec3 {
        if let Some(d) = self.direction {
            return d;
        }
        match curve_plane(c) {
            Some(p) if c.is_closed() || c.degree > 1 || c.control_points.len() > 2 => {
                // keep the orientation consistent with the CPlane normal
                if p.z_axis.dot(self.cplane.z_axis) < 0.0 {
                    -p.z_axis
                } else {
                    p.z_axis
                }
            }
            _ => self.cplane.z_axis,
        }
    }

    fn base(&self, doc: &Document) -> DVec3 {
        self.base_point.unwrap_or_else(|| {
            self.curves.first().and_then(|id| doc.get(*id)).and_then(|o| o.geometry.as_curve()).map_or(DVec3::ZERO, |c| c.start_point())
        })
    }

    fn distance_from_point(&self, doc: &Document, p: DVec3) -> f64 {
        let first = self.curves.first().and_then(|id| doc.get(*id)).and_then(|o| o.geometry.as_curve());
        let dir = first.map_or(self.cplane.z_axis, |c| self.dir_for(c));
        (p - self.base(doc)).dot(dir)
    }

    fn geometry(&self, doc: &Document, h: f64) -> Vec<Geometry> {
        let mut out = Vec::new();
        for id in &self.curves {
            let Some(c) = doc.get(*id).and_then(|o| o.geometry.as_curve()) else { continue };
            let dir = self.dir_for(c);
            let (start, len) = if self.both_sides { (c.transformed(&glam::DAffine3::from_translation(-dir * h)), h * 2.0) } else { (c.clone(), h) };
            let Ok(side) = NurbsSurface::extrude(&start, dir * len) else { continue };
            if self.solid && c.is_closed() {
                if let Some(cap0) = planar_cap(&start) {
                    let mut cap1 = cap0.clone();
                    cap1.transform(&glam::DAffine3::from_translation(dir * len));
                    let mut b = Brep::new(vec![Face::untrimmed(side), cap0, cap1]);
                    b.orient_outward();
                    b.closed = true;
                    out.push(Geometry::Brep(b));
                    continue;
                }
            }
            out.push(Geometry::Surface(side));
        }
        out
    }

    fn finish(&self, ctx: &mut CmdCtx, h: f64) -> Step {
        if h.abs() < 1e-9 {
            ctx.log("Extrusion distance is zero.");
            return Step::Cancelled;
        }
        let geoms = self.geometry(ctx.doc, h);
        if geoms.is_empty() {
            return Step::Cancelled;
        }
        ctx.mem_set("ExtrudeCrv.Distance", fmt_num(h));
        ctx.doc.begin_undo("ExtrudeCrv");
        if self.delete_input {
            for id in &self.curves {
                ctx.doc.remove(*id);
            }
        }
        ctx.doc.clear_selection();
        for g in geoms {
            let id = ctx.doc.add(g);
            ctx.doc.select(id, true);
        }
        Step::Done
    }
}

impl Command for ExtrudeCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.cplane = ctx.cplane;
        self.solid = ctx.mem_bool("ExtrudeCrv.Solid", true);
        Prompt::objects("Select curves to extrude", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.curves = ids;
                self.prompt(ctx).step()
            }
            Input::Option(o) => match o.as_str() {
                "BothSides" => {
                    self.both_sides = !self.both_sides;
                    self.prompt(ctx).step()
                }
                "Solid" => {
                    self.solid = !self.solid;
                    ctx.mem_set("ExtrudeCrv.Solid", yesno(self.solid));
                    self.prompt(ctx).step()
                }
                "DeleteInput" => {
                    self.delete_input = !self.delete_input;
                    self.prompt(ctx).step()
                }
                "Direction" => {
                    self.asking = Some("Direction");
                    self.dir_pts.clear();
                    Prompt::point("Base point of direction").step()
                }
                "SetBasePoint" => {
                    self.asking = Some("SetBasePoint");
                    Prompt::point("Base point").step()
                }
                other => {
                    not_implemented(ctx, other);
                    self.prompt(ctx).step()
                }
            },
            Input::Point(p) if self.asking == Some("Direction") => {
                self.dir_pts.push(p);
                if self.dir_pts.len() == 1 {
                    Prompt::point("Second point of direction").step()
                } else {
                    let d = self.dir_pts[1] - self.dir_pts[0];
                    if d.length() > 1e-9 {
                        self.direction = Some(d.normalize());
                    }
                    self.asking = None;
                    self.prompt(ctx).step()
                }
            }
            Input::Point(p) if self.asking == Some("SetBasePoint") => {
                self.base_point = Some(p);
                self.asking = None;
                self.prompt(ctx).step()
            }
            Input::Number(h) if !self.curves.is_empty() => self.finish(ctx, h),
            Input::Point(p) if !self.curves.is_empty() => {
                let h = self.distance_from_point(ctx.doc, p);
                self.finish(ctx, h)
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let (Some(p), false) = (cursor, self.curves.is_empty()) else { return vec![] };
        if self.asking.is_some() {
            return self.dir_pts.first().map_or(vec![], |a| vec![Preview::Line(*a, p)]);
        }
        let h = self.distance_from_point(doc, p);
        self.geometry(doc, h)
            .into_iter()
            .map(|g| match g {
                Geometry::Surface(s) => Preview::Surface(s),
                Geometry::Brep(b) => Preview::Brep(b),
                _ => Preview::Point(p),
            })
            .collect()
    }
    fn base_point(&self) -> Option<DVec3> {
        self.dir_pts.first().copied().or(self.base_point)
    }
}

#[derive(Default)]
pub struct RevolveCmd {
    curves: Vec<ObjectId>,
    axis_start: Option<DVec3>,
    axis_end: Option<DVec3>,
    start_angle: Option<f64>,
    deformable: bool,
    delete_input: bool,
}

impl RevolveCmd {
    fn angle_prompt(&self) -> Prompt {
        if self.start_angle.is_none() {
            Prompt::number("Start angle").default_num(0.0).opts([opt("FullCircle"), optv("Deformable", yesno(self.deformable)), optv("DeleteInput", yesno(self.delete_input))])
        } else {
            Prompt::number("Revolution angle").default_num(360.0).opts([opt("FullCircle")])
        }
    }

    fn build(&self, doc: &Document, start_deg: f64, sweep_deg: f64) -> Vec<NurbsSurface> {
        let (Some(a), Some(b)) = (self.axis_start, self.axis_end) else { return vec![] };
        let axis = b - a;
        self.curves
            .iter()
            .filter_map(|id| doc.get(*id))
            .filter_map(|o| o.geometry.as_curve())
            .filter_map(|c| {
                let q = glam::DQuat::from_axis_angle(axis.normalize_or(DVec3::Z), start_deg.to_radians());
                let xf = glam::DAffine3::from_translation(a) * glam::DAffine3::from_quat(q) * glam::DAffine3::from_translation(-a);
                let c = c.transformed(&xf);
                let s = NurbsSurface::revolve(&c, a, axis, sweep_deg.to_radians()).ok()?;
                if self.deformable {
                    // approximate with a non-rational degree 3 loft of sampled sections
                    let n = 12;
                    let sections: Vec<NurbsCurve> = (0..=n)
                        .map(|i| {
                            let ang = sweep_deg.to_radians() * i as f64 / n as f64;
                            let q = glam::DQuat::from_axis_angle(axis.normalize_or(DVec3::Z), ang);
                            let xf = glam::DAffine3::from_translation(a) * glam::DAffine3::from_quat(q) * glam::DAffine3::from_translation(-a);
                            c.transformed(&xf)
                        })
                        .collect();
                    NurbsSurface::loft(&sections, 3).ok()
                } else {
                    Some(s)
                }
            })
            .collect()
    }

    fn finish(&self, ctx: &mut CmdCtx, start: f64, sweep: f64) -> Step {
        let srfs = self.build(ctx.doc, start, sweep);
        if srfs.is_empty() {
            ctx.log("Revolve failed.");
            return Step::Cancelled;
        }
        ctx.doc.begin_undo("Revolve");
        if self.delete_input {
            for id in &self.curves {
                ctx.doc.remove(*id);
            }
        }
        ctx.doc.clear_selection();
        for s in srfs {
            let id = ctx.doc.add(Geometry::Surface(s));
            ctx.doc.select(id, true);
        }
        Step::Done
    }
}

impl Command for RevolveCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves to revolve", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.curves = ids;
                Prompt::point("Start of revolve axis").step()
            }
            Input::Point(p) if self.axis_start.is_none() => {
                self.axis_start = Some(p);
                Prompt::point("End of revolve axis").step()
            }
            Input::Point(p) if self.axis_end.is_none() => {
                if (p - self.axis_start.unwrap()).length() < 1e-9 {
                    ctx.log("Axis has zero length.");
                    return Prompt::point("End of revolve axis").step();
                }
                self.axis_end = Some(p);
                self.angle_prompt().step()
            }
            Input::Empty if self.axis_start.is_some() && self.axis_end.is_none() => {
                self.axis_end = Some(self.axis_start.unwrap() + ctx.cplane.z_axis);
                self.angle_prompt().step()
            }
            Input::Option(o) if self.axis_end.is_some() => match o.as_str() {
                "FullCircle" => self.finish(ctx, self.start_angle.unwrap_or(0.0), 360.0),
                "Deformable" => {
                    self.deformable = !self.deformable;
                    self.angle_prompt().step()
                }
                "DeleteInput" => {
                    self.delete_input = !self.delete_input;
                    self.angle_prompt().step()
                }
                _ => self.angle_prompt().step(),
            },
            ref input @ (Input::Number(_) | Input::Empty) if self.axis_end.is_some() => {
                let n = if let Input::Number(v) = input { *v } else if self.start_angle.is_none() { 0.0 } else { 360.0 };
                if let Some(start) = self.start_angle {
                    self.finish(ctx, start, n)
                } else {
                    self.start_angle = Some(n);
                    self.angle_prompt().step()
                }
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        match (self.axis_start, self.axis_end, cursor) {
            (Some(a), None, Some(c)) => vec![Preview::Line(a, c)],
            (Some(a), Some(b), _) => {
                let mut out = vec![Preview::Line(a, b)];
                for s in self.build(doc, self.start_angle.unwrap_or(0.0), 360.0) {
                    out.push(Preview::Surface(s));
                }
                out
            }
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.axis_start
    }
}

#[derive(Default)]
pub struct LoftCmd;

impl Command for LoftCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves to loft", ObjFilter::Curve, 2).opts([opt("Point")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                ctx.requests.push(AppRequest::LoftDialog(ids));
                Step::Done
            }
            Input::Option(o) => {
                not_implemented(ctx, &o);
                Prompt::objects("Select curves to loft. Press Enter when done", ObjFilter::Curve, 2).step()
            }
            _ => Step::Cancelled,
        }
    }
}

/// Cap: close planar openings of surfaces and polysurfaces.
pub struct CapCmd;

impl Command for CapCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select surfaces or polysurfaces to cap", ObjFilter::Surface, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("Cap");
        let mut capped = 0;
        for id in ids {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            let faces: Vec<Face> = match &o.geometry {
                Geometry::Surface(s) => vec![Face::untrimmed(s.clone())],
                Geometry::Brep(b) => b.faces.clone(),
                _ => continue,
            };
            let brep = Brep::new(faces);
            if brep.closed {
                continue;
            }
            let mut faces = brep.faces.clone();
            let mut added = 0;
            for (plane, pts) in brep.naked_planar_loops() {
                if let Ok(f) = Face::planar(&plane, vec![pts]) {
                    faces.push(f);
                    added += 1;
                }
            }
            if added == 0 {
                ctx.log("Unable to cap: the openings are not planar.");
                continue;
            }
            let mut b = Brep::new(faces);
            b.orient_outward();
            ctx.doc.replace_geometry(id, Geometry::Brep(b));
            ctx.doc.select(id, true);
            capped += 1;
        }
        ctx.log(format!("{capped} objects capped."));
        Step::Done
    }
}

/// PlanarSrf: planar surfaces from closed planar curves (nested curves become holes).
pub struct PlanarSrfCmd;

impl Command for PlanarSrfCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select planar curves to build surface", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        let curves: Vec<NurbsCurve> = ids.iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| o.geometry.as_curve().cloned()).collect();
        let faces = planar_faces(&curves);
        if faces.is_empty() {
            ctx.log("No closed planar curves selected.");
            return Step::Cancelled;
        }
        ctx.doc.begin_undo("PlanarSrf");
        ctx.doc.clear_selection();
        for f in faces {
            let id = ctx.doc.add(Geometry::Brep(Brep::new(vec![f])));
            ctx.doc.select(id, true);
        }
        Step::Done
    }
}
