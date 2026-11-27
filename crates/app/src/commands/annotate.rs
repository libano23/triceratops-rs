//! Annotation commands built from curves: Text / TextObject (stroke font), Dim (linear),
//! DimAligned, Leader and Hatch (solid or parallel-line patterns).

use glam::{DVec2, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{brep::point_in_polygon, Brep, Face, NurbsCurve, Plane};

use super::{fmt_num, opt, optv, yesno, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

/// Shape a line of text into 3D polylines on `plane` at `origin`, `height` tall.
pub fn text_curves(text: &str, plane: &Plane, origin: DVec3, height: f64, rotation: f64) -> Vec<NurbsCurve> {
    let shaped = cadcraft_fonts::shape_line(&cadcraft_fonts::TextFont::Stroke, text, height, 1.0, 0.0);
    let (s, c) = rotation.sin_cos();
    let mut out = Vec::new();
    for stroke in shaped.strokes.iter().chain(shaped.glyphs.iter().flatten()) {
        let pts: Vec<DVec3> = stroke.iter().map(|p| {
            let (x, y) = (p.x * c - p.y * s, p.x * s + p.y * c);
            origin + plane.x_axis * x + plane.y_axis * y
        }).collect();
        if pts.len() >= 2 {
            if let Ok(cv) = NurbsCurve::polyline(pts) {
                out.push(cv);
            }
        }
    }
    out
}

fn add_group(ctx: &mut CmdCtx, label: &str, curves: Vec<NurbsCurve>, name: Option<String>) -> Step {
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
        a.name = name.clone();
        let id = ctx.doc.add_with(Geometry::Curve(c), a);
        ctx.doc.select(id, true);
    }
    Step::Done
}

pub struct TextCmd {
    as_curves: bool,
    text: Option<String>,
    height: f64,
}

impl TextCmd {
    pub fn new(as_curves: bool) -> Self {
        Self { as_curves, text: None, height: 1.0 }
    }
}

impl Command for TextCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.height = ctx.mem_num("Text.Height", 1.0);
        Prompt::text("Text to insert").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.text, input) {
            (None, Input::Text(t)) => {
                if t.trim().is_empty() {
                    return Step::Cancelled;
                }
                self.text = Some(t);
                Prompt::point("Start point").opts([optv("Height", fmt_num(self.height)), optv("Font", "CADCraft Stroke"), opt("Justify")]).step()
            }
            (_, Input::Option(o)) if o == "Height" => Prompt::number("Text height").default_num(self.height).step(),
            (_, Input::Option(_)) => Prompt::point("Start point").step(),
            (_, Input::Number(h)) => {
                self.height = h.abs().max(1e-6);
                ctx.mem_set("Text.Height", fmt_num(self.height));
                Prompt::point("Start point").opts([optv("Height", fmt_num(self.height))]).step()
            }
            (Some(t), Input::Point(p)) => {
                let curves = text_curves(t, &ctx.cplane, p, self.height, 0.0);
                let label = if self.as_curves { "TextObject" } else { "Text" };
                add_group(ctx, label, curves, Some(t.clone()))
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (&self.text, cursor) {
            (Some(t), Some(p)) => text_curves(t, &Plane::world_xy(), p, self.height, 0.0).into_iter().map(Preview::Curve).collect(),
            _ => vec![],
        }
    }
}

/// Linear or aligned dimension: extension lines, dimension line with arrows and the value text.
pub fn dimension_curves(a: DVec3, b: DVec3, line_pt: DVec3, plane: &Plane, aligned: bool, text_height: f64) -> Vec<NurbsCurve> {
    let (ua, va, _) = plane.closest_parameters(a);
    let (ub, vb, _) = plane.closest_parameters(b);
    let (ul, vl, _) = plane.closest_parameters(line_pt);
    let (pa, pb) = (DVec2::new(ua, va), DVec2::new(ub, vb));
    let pl = DVec2::new(ul, vl);
    // direction of the dimension line
    let dir = if aligned {
        (pb - pa).normalize_or(DVec2::X)
    } else if (ub - ua).abs() >= (vb - va).abs() {
        DVec2::X
    } else {
        DVec2::Y
    };
    let normal = DVec2::new(-dir.y, dir.x);
    let offset = (pl - pa).dot(normal);
    let da = pa + normal * offset;
    let db = pb + normal * offset;
    let p3 = |q: DVec2| plane.point_at(q.x, q.y);
    let mut out = Vec::new();
    let mut push = |pts: Vec<DVec2>| {
        if let Ok(c) = NurbsCurve::polyline(pts.into_iter().map(p3).collect()) {
            out.push(c);
        }
    };
    let gap = text_height * 0.25;
    let ext = text_height * 0.5;
    let sign = offset.signum();
    push(vec![pa + normal * (gap * sign), da + normal * (ext * sign)]);
    push(vec![pb + normal * (gap * sign), db + normal * (ext * sign)]);
    push(vec![da, db]);
    let ah = text_height * 0.6;
    let aw = text_height * 0.2;
    let d = (db - da).normalize_or(dir);
    push(vec![da + d * ah + normal * aw, da, da + d * ah - normal * aw]);
    push(vec![db - d * ah + normal * aw, db, db - d * ah - normal * aw]);
    let value = (db - da).length();
    let label = fmt_num(value);
    let width = cadcraft_fonts::text_width(&cadcraft_fonts::TextFont::Stroke, &label, text_height, 1.0);
    let mid = (da + db) * 0.5 - d * (width * 0.5) + normal * (text_height * 0.3);
    let rot = d.y.atan2(d.x);
    out.extend(text_curves(&label, plane, p3(mid), text_height, rot));
    out
}

pub struct DimCmd {
    aligned: bool,
    pts: Vec<DVec3>,
}

impl DimCmd {
    pub fn new(aligned: bool) -> Self {
        Self { aligned, pts: Vec::new() }
    }
}

impl Command for DimCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("First dimension point").opts([opt("Object")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                match self.pts.len() {
                    1 => Prompt::point("Second dimension point").step(),
                    2 => Prompt::point("Dimension line location").step(),
                    _ => {
                        let h = ctx.mem_num("Dim.TextHeight", 1.0);
                        let curves = dimension_curves(self.pts[0], self.pts[1], self.pts[2], &ctx.cplane, self.aligned, h);
                        add_group(ctx, if self.aligned { "DimAligned" } else { "Dim" }, curves, None)
                    }
                }
            }
            Input::Option(_) => Prompt::point("First dimension point").step(),
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.pts.len(), cursor) {
            (1, Some(c)) => vec![Preview::Line(self.pts[0], c)],
            (2, Some(c)) => dimension_curves(self.pts[0], self.pts[1], c, &Plane::world_xy(), self.aligned, 1.0).into_iter().map(Preview::Curve).collect(),
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

#[derive(Default)]
pub struct LeaderCmd {
    pts: Vec<DVec3>,
}

impl Command for LeaderCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Start of leader").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                Prompt::point("Next point of leader. Press Enter when done").step()
            }
            Input::Empty if self.pts.len() >= 2 => Prompt::text("Leader text").step(),
            Input::Text(t) => {
                let h = ctx.mem_num("Dim.TextHeight", 1.0);
                let cp = ctx.cplane;
                let mut curves = Vec::new();
                if let Ok(c) = NurbsCurve::polyline(self.pts.clone()) {
                    curves.push(c);
                }
                // arrowhead at the start
                let (a, b) = (self.pts[0], self.pts[1]);
                let d = (b - a).normalize_or(cp.x_axis);
                let n = cp.z_axis.cross(d);
                if let Ok(c) = NurbsCurve::polyline(vec![a + d * (h * 0.6) + n * (h * 0.2), a, a + d * (h * 0.6) - n * (h * 0.2)]) {
                    curves.push(c);
                }
                let last = *self.pts.last().unwrap();
                let prev = self.pts[self.pts.len() - 2];
                let left = (last - prev).dot(cp.x_axis) < 0.0;
                let width = cadcraft_fonts::text_width(&cadcraft_fonts::TextFont::Stroke, &t, h, 1.0);
                let origin = last + cp.x_axis * (if left { -width - h * 0.3 } else { h * 0.3 }) - cp.y_axis * (h * 0.5);
                curves.extend(text_curves(&t, &cp, origin, h, 0.0));
                add_group(ctx, "Leader", curves, Some(t))
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
            vec![Preview::Polyline(pts)]
        } else {
            vec![]
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

/// Hatch lines at `angle` (degrees) and `spacing` clipped to a polygon (outer loop + holes) in UV.
pub fn hatch_lines(loops: &[Vec<DVec2>], angle_deg: f64, spacing: f64) -> Vec<(DVec2, DVec2)> {
    let outer = &loops[0];
    let (s, c) = angle_deg.to_radians().sin_cos();
    let dir = DVec2::new(c, s);
    let nrm = DVec2::new(-s, c);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut tlo = f64::INFINITY;
    let mut thi = f64::NEG_INFINITY;
    for p in outer {
        lo = lo.min(p.dot(nrm));
        hi = hi.max(p.dot(nrm));
        tlo = tlo.min(p.dot(dir));
        thi = thi.max(p.dot(dir));
    }
    let mut out = Vec::new();
    let spacing = spacing.max(1e-6);
    let mut k = (lo / spacing).ceil() * spacing;
    let inside = |p: DVec2| point_in_polygon(p, outer) && !loops[1..].iter().any(|h| point_in_polygon(p, h));
    while k <= hi {
        // intersections of the line n·p = k with all loop edges
        let mut ts = Vec::new();
        for l in loops {
            for i in 0..l.len() {
                let a = l[i];
                let b = l[(i + 1) % l.len()];
                let da = a.dot(nrm) - k;
                let db = b.dot(nrm) - k;
                if (da < 0.0) != (db < 0.0) {
                    let f = da / (da - db);
                    let p = a + (b - a) * f;
                    ts.push(p.dot(dir));
                }
            }
        }
        ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for w in ts.chunks(2) {
            if w.len() == 2 {
                let mid = nrm * k + dir * ((w[0] + w[1]) * 0.5);
                if inside(mid) {
                    out.push((nrm * k + dir * w[0], nrm * k + dir * w[1]));
                }
            }
        }
        k += spacing;
    }
    let _ = (tlo, thi);
    out
}

pub struct HatchCmd {
    pattern: usize,
    scale: f64,
    rotation: f64,
}

const PATTERNS: [&str; 4] = ["Solid", "Hatch1", "Grid", "Squares"];

impl Default for HatchCmd {
    fn default() -> Self {
        Self { pattern: 1, scale: 1.0, rotation: 45.0 }
    }
}

impl HatchCmd {
    fn prompt(&self) -> Prompt {
        Prompt::objects("Select closed planar curves for hatch boundary", ObjFilter::Curve, 1).opts([optv("Pattern", PATTERNS[self.pattern]), optv("Scale", fmt_num(self.scale)), optv("Rotation", fmt_num(self.rotation))])
    }
}

impl Command for HatchCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.pattern = ctx.mem_num("Hatch.Pattern", 1.0) as usize % PATTERNS.len();
        self.scale = ctx.mem_num("Hatch.Scale", 1.0);
        self.rotation = ctx.mem_num("Hatch.Rotation", 45.0);
        self.prompt().step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => match o.as_str() {
                "Pattern" => {
                    self.pattern = (self.pattern + 1) % PATTERNS.len();
                    ctx.mem_set("Hatch.Pattern", self.pattern);
                    self.prompt().step()
                }
                "Scale" => {
                    ctx.mem_set("Hatch.Asking", "Scale");
                    Prompt::number("Pattern scale").default_num(self.scale).step()
                }
                "Rotation" => {
                    ctx.mem_set("Hatch.Asking", "Rotation");
                    Prompt::number("Pattern rotation").default_num(self.rotation).step()
                }
                _ => self.prompt().step(),
            },
            Input::Number(v) => {
                match ctx.memory.remove("Hatch.Asking").as_deref() {
                    Some("Scale") => {
                        self.scale = v.abs().max(1e-6);
                        ctx.mem_set("Hatch.Scale", fmt_num(self.scale));
                    }
                    Some("Rotation") => {
                        self.rotation = v;
                        ctx.mem_set("Hatch.Rotation", fmt_num(self.rotation));
                    }
                    _ => {}
                }
                self.prompt().step()
            }
            Input::Objects(ids) => {
                let curves: Vec<NurbsCurve> = ids.iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| o.geometry.as_curve().cloned()).collect();
                let faces = super::create::planar_faces(&curves);
                if faces.is_empty() {
                    ctx.log("No closed planar curves selected.");
                    return Step::Cancelled;
                }
                if self.pattern == 0 {
                    ctx.doc.begin_undo("Hatch");
                    ctx.doc.clear_selection();
                    for f in faces {
                        let id = ctx.doc.add(Geometry::Brep(Brep { faces: vec![f], closed: false, exact: None }));
                        ctx.doc.select(id, true);
                    }
                    return Step::Done;
                }
                let mut curves_out = Vec::new();
                for f in &faces {
                    let spacing = self.scale;
                    let angles: Vec<f64> = match self.pattern {
                        2 => vec![self.rotation, self.rotation + 90.0],
                        3 => vec![self.rotation - 45.0, self.rotation + 45.0],
                        _ => vec![self.rotation],
                    };
                    for a in angles {
                        for (p, q) in hatch_lines(&f.loops, a, spacing) {
                            let a3 = f.surface.point_at(p.x, p.y);
                            let b3 = f.surface.point_at(q.x, q.y);
                            if let Ok(l) = NurbsCurve::line(a3, b3) {
                                curves_out.push(l);
                            }
                        }
                    }
                }
                add_group(ctx, "Hatch", curves_out, None)
            }
            _ => Step::Cancelled,
        }
    }
}

#[allow(dead_code)]
fn unused(_: ObjectId, _: &Face) -> &'static str {
    yesno(false)
}
