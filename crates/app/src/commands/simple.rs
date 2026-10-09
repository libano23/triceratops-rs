//! Declarative helpers for small commands plus the system / scripting / selection batch.

use glam::{DVec2, DVec3};
use tri_doc::{Document, Geometry, Object, ObjectId};
use tri_geom::{curve_curve_intersections, mesh_ops, NurbsCurve};

use super::{fmt_num, opt, AppRequest, CmdCtx, Command, Input, ObjFilter, Opt, Preview, Prompt, Step};

/// A command that only prints a message (features that have no equivalent in triceratops).
pub struct MsgCmd(pub &'static str);

impl Command for MsgCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.log(self.0);
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

/// A command that runs a closure immediately.
pub struct FnCmd(pub fn(&mut CmdCtx) -> Step);

impl Command for FnCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        (self.0)(ctx)
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

/// One question of a sequential command.
#[derive(Clone)]
pub enum Ask {
    Objects(&'static str, ObjFilter, usize),
    Pick(&'static str, ObjFilter),
    Point(&'static str),
    PointOrNumber(&'static str, f64),
    Number(&'static str, f64),
    Text(&'static str),
    Options(&'static str, Vec<Opt>),
}

/// An answer collected by a sequential command.
#[derive(Clone, Debug)]
pub enum Ans {
    Objects(Vec<ObjectId>),
    Pick(ObjectId, DVec3),
    Point(DVec3),
    Number(f64),
    Text(String),
    Option(String),
}

impl Ans {
    pub fn ids(&self) -> Vec<ObjectId> {
        match self {
            Ans::Objects(v) => v.clone(),
            Ans::Pick(id, _) => vec![*id],
            _ => Vec::new(),
        }
    }
    pub fn point(&self) -> DVec3 {
        match self {
            Ans::Point(p) | Ans::Pick(_, p) => *p,
            _ => DVec3::ZERO,
        }
    }
    pub fn number(&self) -> f64 {
        match self {
            Ans::Number(n) => *n,
            _ => 0.0,
        }
    }
    pub fn text(&self) -> String {
        match self {
            Ans::Text(t) | Ans::Option(t) => t.clone(),
            _ => String::new(),
        }
    }
}

type Finish = Box<dyn Fn(&[Ans], &mut CmdCtx) -> Step>;

/// Sequential command: asks the questions in order and calls `finish` with the answers.
pub struct SeqCmd {
    asks: Vec<Ask>,
    answers: Vec<Ans>,
    finish: Finish,
    /// Options that can be chosen at any step; choosing one re-asks the current question.
    pub clear_selection_after_objects: bool,
    /// Continuation used when `finish` returns `Step::Next`: every later input goes here.
    then: Option<Box<dyn Fn(Input, &mut CmdCtx) -> Step>>,
    in_then: bool,
}

impl SeqCmd {
    pub fn new(asks: Vec<Ask>, finish: impl Fn(&[Ans], &mut CmdCtx) -> Step + 'static) -> Self {
        Self { asks, answers: Vec::new(), finish: Box::new(finish), clear_selection_after_objects: true, then: None, in_then: false }
    }
    /// Handle inputs after `finish` returned a prompt (multi-stage commands).
    pub fn then(mut self, f: impl Fn(Input, &mut CmdCtx) -> Step + 'static) -> Self {
        self.then = Some(Box::new(f));
        self
    }
    fn prompt_for(&self, ask: &Ask) -> Prompt {
        match ask {
            Ask::Objects(m, f, n) => Prompt::objects(*m, *f, *n),
            Ask::Pick(m, f) => Prompt::pick(*m, *f),
            Ask::Point(m) => Prompt::point(*m),
            Ask::PointOrNumber(m, d) => Prompt::point_or_number(*m).default_num(*d),
            Ask::Number(m, d) => Prompt::number(*m).default_num(*d),
            Ask::Text(m) => Prompt::text(*m),
            Ask::Options(m, o) => Prompt::options(*m).opts(o.clone()),
        }
    }
    fn next(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.asks.get(self.answers.len()) {
            Some(a) => self.prompt_for(a).step(),
            None => {
                let s = (self.finish)(&self.answers, ctx);
                if matches!(s, Step::Prompt(_)) && self.then.is_some() {
                    self.in_then = true;
                }
                s
            }
        }
    }
}

impl Command for SeqCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.next(ctx)
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        if self.in_then {
            return match &self.then {
                Some(f) => f(input, ctx),
                None => Step::Done,
            };
        }
        let Some(ask) = self.asks.get(self.answers.len()).cloned() else { return Step::Cancelled };
        let ans = match (&ask, input) {
            (Ask::Objects(..), Input::Objects(ids)) => {
                if self.clear_selection_after_objects {
                    ctx.doc.clear_selection();
                }
                Ans::Objects(ids)
            }
            (Ask::Pick(..), Input::Pick(id, p)) => Ans::Pick(id, p),
            (Ask::Point(_), Input::Point(p)) | (Ask::PointOrNumber(..), Input::Point(p)) => Ans::Point(p),
            (Ask::Number(..), Input::Number(n)) | (Ask::PointOrNumber(..), Input::Number(n)) => Ans::Number(n),
            (Ask::Number(_, d), Input::Empty) | (Ask::PointOrNumber(_, d), Input::Empty) => Ans::Number(*d),
            (Ask::Text(_), Input::Text(t)) => Ans::Text(t),
            (Ask::Text(_), Input::Empty) => Ans::Text(String::new()),
            (Ask::Options(..), Input::Option(o)) => Ans::Option(o),
            (Ask::Options(..), Input::Empty) => Ans::Option(String::new()),
            (_, Input::Option(_)) => return self.next(ctx),
            _ => return Step::Cancelled,
        };
        self.answers.push(ans);
        self.next(ctx)
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.answers.last(), cursor) {
            (Some(a), Some(c)) if matches!(a, Ans::Point(_)) && matches!(self.asks.get(self.answers.len()), Some(Ask::Point(_)) | Some(Ask::PointOrNumber(..))) => vec![Preview::Line(a.point(), c)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.answers.iter().rev().find_map(|a| if let Ans::Point(p) = a { Some(*p) } else { None })
    }
}

pub fn add_all(ctx: &mut CmdCtx, label: &str, geoms: Vec<Geometry>) -> Step {
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

pub fn select_where(ctx: &mut CmdCtx, pred: impl Fn(&Document, &Object) -> bool) -> Step {
    let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && pred(ctx.doc, o)).map(|o| o.id).collect();
    for id in &ids {
        ctx.doc.select(*id, true);
    }
    ctx.log(match ids.len() {
        0 => "No objects added to selection.".to_string(),
        1 => "1 object added to selection.".to_string(),
        n => format!("{n} objects added to selection."),
    });
    Step::Done
}

pub fn open_external(target: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open").arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn group_prefix(o: &Object, prefix: &str) -> bool {
    o.attributes.groups.iter().any(|g| g.starts_with(prefix))
}

pub fn has_user_text(o: &Object, key: &str) -> bool {
    o.attributes.user_text.iter().any(|(k, _)| k == key)
}

/// Self intersection test for a curve by tessellated segments.
pub fn curve_self_intersects(c: &NurbsCurve) -> bool {
    let tol = c.bounding_box().diagonal().max(1e-6) * 1e-3;
    let pts = c.tessellate_adaptive(tol);
    let n = pts.len();
    for i in 0..n.saturating_sub(1) {
        for j in i + 2..n.saturating_sub(1) {
            if i == 0 && j == n - 2 && c.is_closed() {
                continue;
            }
            if seg_seg_distance(pts[i], pts[i + 1], pts[j], pts[j + 1]) < tol {
                return true;
            }
        }
    }
    false
}

fn seg_seg_distance(p1: DVec3, p2: DVec3, q1: DVec3, q2: DVec3) -> f64 {
    let d1 = p2 - p1;
    let d2 = q2 - q1;
    let r = p1 - q1;
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (b, c) = (d1.dot(d2), d1.dot(r));
    let denom = a * e - b * b;
    let mut s = if denom.abs() > 1e-18 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let mut t = (b * s + f) / e.max(1e-18);
    if t < 0.0 {
        t = 0.0;
        s = (-c / a.max(1e-18)).clamp(0.0, 1.0);
    } else if t > 1.0 {
        t = 1.0;
        s = ((b - c) / a.max(1e-18)).clamp(0.0, 1.0);
    }
    ((p1 + d1 * s) - (q1 + d2 * t)).length()
}

/// Polygon containment in the CPlane for Lasso / SelFence style selections.
pub fn point_in_poly2(p: DVec2, poly: &[DVec2]) -> bool {
    tri_geom::brep::point_in_polygon(p, poly)
}

// ---- selection commands that need prompts ----------------------------------------------------

pub struct SelPolygonCmd {
    pub crossing: bool,
    pub label: &'static str,
    pts: Vec<DVec3>,
}

impl SelPolygonCmd {
    pub fn new(label: &'static str, crossing: bool) -> Self {
        Self { crossing, label, pts: Vec::new() }
    }
}

impl Command for SelPolygonCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Start of selection polygon").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                self.pts.push(p);
                Prompt::point("Next point. Press Enter when done").step()
            }
            Input::Empty if self.pts.len() >= 3 => {
                let cp = ctx.cplane;
                let poly: Vec<DVec2> = self.pts.iter().map(|p| { let (u, v, _) = cp.closest_parameters(*p); DVec2::new(u, v) }).collect();
                let crossing = self.crossing;
                select_where(ctx, |d, o| {
                    let bb = d.object_bbox(o);
                    let corners: Vec<DVec2> = bb.corners().iter().map(|c| { let (u, v, _) = cp.closest_parameters(*c); DVec2::new(u, v) }).collect();
                    if crossing {
                        corners.iter().any(|c| point_in_poly2(*c, &poly)) || poly.iter().any(|q| { let (mn, mx) = corners.iter().fold((DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)), |(a, b), c| (a.min(*c), b.max(*c))); q.x >= mn.x && q.x <= mx.x && q.y >= mn.y && q.y <= mx.y })
                    } else {
                        corners.iter().all(|c| point_in_poly2(*c, &poly))
                    }
                })
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
}

pub fn sel_volume_object() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select closed surface, polysurface or mesh as volume", ObjFilter::Any)], |a, ctx| {
        let id = a[0].ids()[0];
        let Some(vol) = super::misc4::solid_mesh(ctx.doc, id) else { return Step::Cancelled };
        select_where(ctx, |d, o| o.id != id && mesh_ops::contains_point(&vol, d.object_bbox(o).center()))
    })
}

pub fn sel_volume_pipe() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select curve for pipe axis", ObjFilter::Curve), Ask::PointOrNumber("Pipe radius", 1.0)], |a, ctx| {
        let Some(c) = ctx.doc.get(a[0].ids()[0]).and_then(|o| o.geometry.as_curve().cloned()) else { return Step::Cancelled };
        let r = match &a[1] { Ans::Number(n) => n.abs(), Ans::Point(p) => (*p - c.point_at(c.closest_parameter(*p))).length(), _ => 1.0 };
        select_where(ctx, |d, o| { let p = d.object_bbox(o).center(); (c.point_at(c.closest_parameter(p)) - p).length() <= r })
    })
}

pub fn sel_circular() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Center of selection circle"), Ask::PointOrNumber("Radius", 1.0)], |a, ctx| {
        let c = a[0].point();
        let r = match &a[1] { Ans::Number(n) => n.abs(), Ans::Point(p) => (*p - c).length(), _ => 1.0 };
        let cp = ctx.cplane;
        select_where(ctx, |d, o| { let p = d.object_bbox(o).center(); let (u, v, _) = cp.closest_parameters(p); let (cu, cv, _) = cp.closest_parameters(c); ((u - cu).powi(2) + (v - cv).powi(2)).sqrt() <= r })
    })
}

pub fn sel_rectangular() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("First corner of selection window"), Ask::Point("Other corner")], |a, ctx| {
        let cp = ctx.cplane;
        let (u0, v0, _) = cp.closest_parameters(a[0].point());
        let (u1, v1, _) = cp.closest_parameters(a[1].point());
        let (umin, umax, vmin, vmax) = (u0.min(u1), u0.max(u1), v0.min(v1), v0.max(v1));
        let crossing = u1 < u0;
        select_where(ctx, |d, o| {
            let corners: Vec<(f64, f64)> = d.object_bbox(o).corners().iter().map(|c| { let (u, v, _) = cp.closest_parameters(*c); (u, v) }).collect();
            let inside = |(u, v): &(f64, f64)| *u >= umin && *u <= umax && *v >= vmin && *v <= vmax;
            if crossing { corners.iter().any(inside) } else { corners.iter().all(inside) }
        })
    })
}

pub fn sel_key(with_value: bool) -> SeqCmd {
    let asks = if with_value { vec![Ask::Text("Key"), Ask::Text("Value")] } else { vec![Ask::Text("Key")] };
    SeqCmd::new(asks, move |a, ctx| {
        let key = a[0].text();
        let value = a.get(1).map(|v| v.text());
        select_where(ctx, |_, o| o.attributes.user_text.iter().any(|(k, v)| *k == key && value.as_ref().map_or(true, |val| v == val)))
    })
}

pub fn sel_value() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Value")], |a, ctx| {
        let value = a[0].text();
        select_where(ctx, |_, o| o.attributes.user_text.iter().any(|(_, v)| *v == value))
    })
}

pub fn sel_layer_number() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Layer number", 0.0)], |a, ctx| {
        let n = a[0].number().round() as usize;
        select_where(ctx, |_, o| o.attributes.layer == n)
    })
}

pub fn select_by_name() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Object name or id")], |a, ctx| {
        let t = a[0].text();
        select_where(ctx, |_, o| o.id.to_string() == t || o.attributes.name.as_deref() == Some(t.as_str()))
    })
}

pub fn intersect_self() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select curves to check for self-intersection", ObjFilter::Curve, 1)], |a, ctx| {
        let mut pts = Vec::new();
        for id in a[0].ids() {
            let Some(c) = ctx.doc.get(id).and_then(|o| o.geometry.as_curve().cloned()) else { continue };
            let tol = c.bounding_box().diagonal().max(1e-6) * 1e-3;
            let poly = c.tessellate_adaptive(tol);
            let n = poly.len();
            for i in 0..n.saturating_sub(1) {
                for j in i + 2..n.saturating_sub(1) {
                    if i == 0 && j == n - 2 && c.is_closed() {
                        continue;
                    }
                    if seg_seg_distance(poly[i], poly[i + 1], poly[j], poly[j + 1]) < tol {
                        pts.push(Geometry::Point((poly[i] + poly[i + 1] + poly[j] + poly[j + 1]) * 0.25));
                    }
                }
            }
        }
        if pts.is_empty() {
            ctx.log("No self-intersections found.");
            return Step::Done;
        }
        add_all(ctx, "IntersectSelf", pts)
    })
}

pub fn intersect_two_sets() -> SeqCmd {
    let mut cmd = SeqCmd::new(vec![Ask::Objects("Select first set of curves", ObjFilter::Curve, 1), Ask::Objects("Select second set of curves", ObjFilter::Curve, 1)], |a, ctx| {
        let mut pts = Vec::new();
        let tol = ctx.doc.settings.absolute_tolerance.max(1e-6);
        for ia in a[0].ids() {
            for ib in a[1].ids() {
                let (Some(ca), Some(cb)) = (ctx.doc.get(ia).and_then(|o| o.geometry.as_curve().cloned()), ctx.doc.get(ib).and_then(|o| o.geometry.as_curve().cloned())) else { continue };
                for x in curve_curve_intersections(&ca, &cb, tol) {
                    pts.push(Geometry::Point(x.point));
                }
            }
        }
        if pts.is_empty() {
            ctx.log("No intersections found.");
            return Step::Done;
        }
        add_all(ctx, "IntersectTwoSets", pts)
    });
    cmd.clear_selection_after_objects = true;
    cmd
}

pub fn run_python() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Python script file")], |a, ctx| {
        let path = a[0].text();
        if path.is_empty() {
            return Step::Cancelled;
        }
        match std::process::Command::new("python3").arg(&path).output() {
            Ok(out) => {
                let text = String::from_utf8_lossy(&out.stdout).to_string();
                let err = String::from_utf8_lossy(&out.stderr).to_string();
                if !err.trim().is_empty() {
                    ctx.log(format!("python3: {}", err.trim()));
                }
                let tmp = std::env::temp_dir().join(format!("triceratops-py-{}.txt", std::process::id()));
                if std::fs::write(&tmp, &text).is_ok() {
                    ctx.log("Python output is fed to the command line, one entry per line.");
                    ctx.requests.push(AppRequest::RunCommandFile(tmp.to_string_lossy().to_string()));
                }
                Step::Done
            }
            Err(e) => {
                ctx.log(format!("Unable to run python3: {e}"));
                Step::Cancelled
            }
        }
    })
}

pub fn open_url_cmd(prompt: &'static str, fixed: Option<&'static str>) -> SeqCmd {
    let asks = if fixed.is_some() { vec![] } else { vec![Ask::Text(prompt)] };
    SeqCmd::new(asks, move |a, ctx| {
        let target = fixed.map(|s| s.to_string()).unwrap_or_else(|| a.first().map(|t| t.text()).unwrap_or_default());
        if target.is_empty() {
            return Step::Cancelled;
        }
        match open_external(&target) {
            Ok(()) => ctx.log(format!("Opening {target}")),
            Err(e) => ctx.log(format!("Unable to open {target}: {e}")),
        }
        Step::Done
    })
}

pub fn run_program() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Program to run")], |a, ctx| {
        let cmd = a[0].text();
        if cmd.is_empty() {
            return Step::Cancelled;
        }
        let mut parts = cmd.split_whitespace();
        let exe = parts.next().unwrap_or_default().to_string();
        match std::process::Command::new(&exe).args(parts).spawn() {
            Ok(_) => ctx.log(format!("Started {exe}")),
            Err(e) => ctx.log(format!("Unable to start {exe}: {e}")),
        }
        Step::Done
    })
}

pub fn edit_file_cmd() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Script file to edit")], |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        if !std::path::Path::new(&p).exists() {
            let _ = std::fs::write(&p, "");
        }
        match open_external(&p) {
            Ok(()) => ctx.log(format!("Opening {p} in the default editor.")),
            Err(e) => ctx.log(format!("Unable to open editor: {e}")),
        }
        Step::Done
    })
}

pub fn set_flag_cmd(key: &'static str, value: bool, note: &'static str) -> FnCmd {
    // closures cannot capture in fn pointers; use memory keys through a small table
    let _ = (key, value, note);
    FnCmd(|_| Step::Done)
}

/// Sets a memory flag (Echo/NoEcho, SetRedrawOn/Off, DragMode ...).
pub struct FlagCmd(pub &'static str, pub &'static str, pub &'static str);

impl Command for FlagCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.mem_set(self.0, self.1);
        ctx.log(self.2);
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

/// Sends an app request chosen from a fixed table (window ops, grip ops, filters).
pub struct ReqCmd(pub AppRequest);

impl Command for ReqCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.requests.push(self.0.clone());
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub fn export_with_origin() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to export", ObjFilter::Any, 1), Ask::Point("Origin point")], |a, ctx| {
        ctx.requests.push(AppRequest::ExportWithOrigin { ids: a[0].ids(), origin: a[1].point() });
        Step::Done
    })
}

pub fn set_working_folder() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Working folder")], |a, ctx| {
        let f = a[0].text();
        if f.is_empty() {
            let cur = ctx.memory.get("WorkingFolder").cloned().unwrap_or_else(|| std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
            ctx.log(format!("Working folder: {cur}"));
        } else {
            let _ = std::env::set_current_dir(&f);
            ctx.mem_set("WorkingFolder", &f);
            ctx.log(format!("Working folder set to {f}"));
        }
        Step::Done
    })
}

pub fn ortho_angle() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Ortho angle", 90.0)], |a, ctx| {
        ctx.requests.push(AppRequest::SetOption { key: "ortho_angle".into(), value: fmt_num(a[0].number()) });
        Step::Done
    })
}

pub fn snap_size() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Grid snap spacing", 1.0)], |a, ctx| {
        ctx.doc.settings.grid_snap_spacing = a[0].number().abs().max(1e-9);
        Step::Done
    })
}

pub fn perspective_angle() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Field of view angle", 50.0)], |a, ctx| {
        ctx.requests.push(AppRequest::SetOption { key: "fov".into(), value: fmt_num(a[0].number()) });
        Step::Done
    })
}

pub fn zoom_lens() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Lens length (mm)", 50.0)], |a, ctx| {
        ctx.requests.push(AppRequest::SetOption { key: "lens".into(), value: fmt_num(a[0].number()) });
        Step::Done
    })
}

pub fn set_mode_cmd(key: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Mode", vec![opt("On"), opt("Off"), opt("Toggle")])], move |a, ctx| {
        let v = a[0].text();
        ctx.requests.push(AppRequest::SetOption { key: key.into(), value: if v.is_empty() { "Toggle".into() } else { v } });
        Step::Done
    })
}

pub fn set_active_viewport() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Viewport name")], |a, ctx| {
        ctx.requests.push(AppRequest::SetOption { key: "active_viewport".into(), value: a[0].text() });
        Step::Done
    })
}

pub fn hyperlink() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Text("URL (empty to remove)")], |a, ctx| {
        let url = a[1].text();
        ctx.doc.begin_undo("Hyperlink");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.user_text.retain(|(k, _)| k != "Hyperlink");
                if !url.is_empty() {
                    o.attributes.user_text.push(("Hyperlink".into(), url.clone()));
                }
            }
        }
        Step::Done
    })
}

pub fn match_properties() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to change", ObjFilter::Any, 1), Ask::Pick("Select source object", ObjFilter::Any)], |a, ctx| {
        let Some(src) = ctx.doc.get(a[1].ids()[0]).map(|o| o.attributes.clone()) else { return Step::Cancelled };
        ctx.doc.begin_undo("MatchProperties");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.layer = src.layer;
                o.attributes.color = src.color;
                o.attributes.user_text = src.user_text.clone();
            }
        }
        Step::Done
    })
}

pub fn find_text() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Text to find")], |a, ctx| {
        let t = a[0].text().to_lowercase();
        if t.is_empty() {
            return Step::Cancelled;
        }
        select_where(ctx, |_, o| o.attributes.name.as_ref().map_or(false, |n| n.to_lowercase().contains(&t)))
    })
}

pub fn scale_text_height() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select text objects", ObjFilter::Any, 1), Ask::Number("Scale factor", 2.0)], |a, ctx| {
        let f = a[1].number();
        let ids = a[0].ids();
        let bb = ids.iter().filter_map(|id| ctx.doc.get(*id)).fold(tri_geom::BoundingBox::EMPTY, |b, o| b.union(&ctx.doc.object_bbox(o)));
        let c = bb.center();
        ctx.doc.begin_undo("ScaleTextHeight");
        ctx.doc.transform_objects(&ids, &(glam::DAffine3::from_translation(c) * glam::DAffine3::from_scale(DVec3::splat(f)) * glam::DAffine3::from_translation(-c)));
        Step::Done
    })
}

pub fn undo_selected() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to restore to their previous state", ObjFilter::Any, 1)], |a, ctx| {
        let n = ctx.doc.undo_selected(&a[0].ids());
        ctx.log(format!("{n} objects restored from the previous undo state."));
        Step::Done
    })
}

pub fn dim_area_volume(volume: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick(if volume { "Select closed object to dimension volume" } else { "Select object to dimension area" }, ObjFilter::Any), Ask::Point("Text location")], move |a, ctx| {
        let Some(o) = ctx.doc.get(a[0].ids()[0]) else { return Step::Cancelled };
        let value = if volume { Document::volume_of(&o.geometry).map(|v| v.0) } else { Document::area_of(&o.geometry) };
        let Some(v) = value else {
            ctx.log("The object has no measurable value.");
            return Step::Cancelled;
        };
        let h = ctx.mem_num("Dim.TextHeight", 1.0);
        let label = if volume { format!("V={}", fmt_num(v)) } else { format!("A={}", fmt_num(v)) };
        let curves = super::annotate::text_curves(&label, &ctx.cplane, a[1].point(), h, 0.0);
        let n = ctx.memory.get("GroupCounter").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) + 1;
        ctx.mem_set("GroupCounter", n);
        ctx.doc.begin_undo(if volume { "DimVolume" } else { "DimArea" });
        ctx.doc.clear_selection();
        for c in curves {
            let mut at = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
            at.groups.push(format!("Dim{n:02}"));
            let id = ctx.doc.add_with(Geometry::Curve(c), at);
            ctx.doc.select(id, true);
        }
        Step::Done
    })
}

/// SelectionFilter: restrict picking to one object kind.
pub struct SelectionFilterCmd;

impl Command for SelectionFilterCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::options("Selection filter").opts([opt("All"), opt("Curves"), opt("Surfaces"), opt("Polysurfaces"), opt("Meshes"), opt("Points")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Option(o) = input else { return Step::Done };
        let f = match o.as_str() {
            "Curves" => Some("curve"),
            "Surfaces" => Some("surface"),
            "Polysurfaces" => Some("polysurface"),
            "Meshes" => Some("mesh"),
            "Points" => Some("point"),
            _ => None,
        };
        ctx.requests.push(AppRequest::SelectionFilter(f.map(|s| s.to_string())));
        Step::Done
    }
}
