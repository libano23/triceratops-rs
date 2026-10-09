//! Selection, layer, measurement, view and application-level commands.

use glam::DVec3;
use tri_doc::{Geometry, ObjectId};

use super::{fmt_num, opt, optv, yesno, AppRequest, CmdCtx, Command, Input, ObjFilter, Prompt, Step};

pub enum SelectCmd {
    All,
    None,
    Invert,
    Filter(ObjFilter),
    Last,
}

impl Command for SelectCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let before = ctx.doc.selection.len();
        match self {
            SelectCmd::All => ctx.doc.select_all(),
            SelectCmd::None => ctx.doc.clear_selection(),
            SelectCmd::Invert => {
                let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o)).map(|o| o.id).collect();
                for id in ids {
                    ctx.doc.toggle_select(id);
                }
            }
            SelectCmd::Filter(f) => {
                let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && f.accepts(&o.geometry)).map(|o| o.id).collect();
                for id in ids {
                    ctx.doc.select(id, true);
                }
            }
            SelectCmd::Last => {
                let ids: Vec<ObjectId> = ctx
                    .memory
                    .get("SelLast")
                    .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
                    .unwrap_or_default();
                for id in ids {
                    ctx.doc.select(id, true);
                }
            }
        }
        let after = ctx.doc.selection.len();
        match self {
            SelectCmd::None => {}
            _ => {
                let added = after.saturating_sub(before);
                ctx.log(format!("{added} objects added to selection. {after} total."));
            }
        }
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

#[derive(Default)]
pub struct LayerCmd;

impl Command for LayerCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.requests.push(AppRequest::ShowPanel("Layers".into()));
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

#[derive(Default)]
pub struct ChangeLayerCmd {
    ids: Vec<ObjectId>,
}

impl Command for ChangeLayerCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to change layer", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                let names: Vec<String> = ctx.doc.layers().iter().map(|l| l.name.clone()).collect();
                ctx.log(format!("Layers: {}", names.join(", ")));
                Prompt::text("Layer name").step()
            }
            Input::Text(name) => {
                let name = name.trim();
                let Some(idx) = ctx.doc.find_layer(name) else {
                    ctx.log(format!("No layer named \"{name}\"."));
                    return Step::Cancelled;
                };
                ctx.doc.begin_undo("ChangeLayer");
                for id in &self.ids {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        o.attributes.layer = idx;
                    }
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

pub struct WhatCmd;

impl Command for WhatCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects for details", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        for id in ids {
            if let Some(o) = ctx.doc.get(id) {
                let layer = ctx.doc.layers().get(o.attributes.layer).map_or("?", |l| l.name.as_str());
                let bb = o.geometry.bounding_box();
                let msg = format!(
                    "Object: {} (ID {})  Layer: {layer}  Bounding box: ({}, {}, {}) to ({}, {}, {})",
                    o.describe_long(),
                    o.id,
                    fmt_num(bb.min.x),
                    fmt_num(bb.min.y),
                    fmt_num(bb.min.z),
                    fmt_num(bb.max.x),
                    fmt_num(bb.max.y),
                    fmt_num(bb.max.z)
                );
                ctx.log(msg);
            }
        }
        Step::Done
    }
}

#[derive(Default)]
pub struct DistanceCmd {
    first: Option<DVec3>,
}

impl Command for DistanceCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("First point for distance").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, input) {
            (None, Input::Point(p)) => {
                self.first = Some(p);
                Prompt::point("Second point for distance").step()
            }
            (Some(a), Input::Point(b)) => {
                let d = b - a;
                let units = ctx.doc.settings.units.to_ascii_lowercase();
                ctx.log(format!("Distance = {} {units}", fmt_num(d.length())));
                let (u, v, w) = ctx.cplane.closest_parameters(b);
                let (u0, v0, w0) = ctx.cplane.closest_parameters(a);
                ctx.log(format!(
                    "CPlane angles and deltas: xy = {}  elevation = {}  dx = {}  dy = {}  dz = {}",
                    fmt_num((v - v0).atan2(u - u0).to_degrees()),
                    fmt_num((w - w0).atan2(((u - u0).powi(2) + (v - v0).powi(2)).sqrt()).to_degrees()),
                    fmt_num(u - u0),
                    fmt_num(v - v0),
                    fmt_num(w - w0)
                ));
                ctx.log(format!("World angles and deltas: dx = {}  dy = {}  dz = {}", fmt_num(d.x), fmt_num(d.y), fmt_num(d.z)));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &tri_doc::Document) -> Vec<super::Preview> {
        match (self.first, cursor) {
            (Some(a), Some(b)) => vec![super::Preview::Line(a, b)],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.first
    }
}

pub struct LengthCmd;

impl Command for LengthCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select curves for length calculation", ObjFilter::Curve, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        let mut total = 0.0;
        for id in ids {
            if let Some(Geometry::Curve(c)) = ctx.doc.get(id).map(|o| &o.geometry) {
                total += c.length();
            }
        }
        ctx.log(format!("Length = {} {}", fmt_num(total), ctx.doc.settings.units.to_ascii_lowercase()));
        Step::Done
    }
}

#[derive(Default)]
pub struct ZoomCmd;

impl Command for ZoomCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::options("Drag a window to zoom")
            .opts([opt("All"), opt("Dynamic"), opt("Extents"), opt("Factor"), opt("In"), opt("Out"), opt("Selected"), opt("Target"), opt("1To1")])
            .step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                match o.as_str() {
                    "Extents" => ctx.requests.push(AppRequest::ZoomExtents { all: false, selected: false }),
                    "Selected" => ctx.requests.push(AppRequest::ZoomExtents { all: false, selected: true }),
                    "All" => {
                        return Prompt::options("Zoom all viewports").opts([opt("Extents"), opt("Selected")]).step();
                    }
                    "In" => ctx.requests.push(AppRequest::CameraOp { op: "Zoom".into(), dx: 1.25, dy: 0.0 }),
                    "Out" => ctx.requests.push(AppRequest::CameraOp { op: "Zoom".into(), dx: 0.8, dy: 0.0 }),
                    "Factor" => return Prompt::number("Zoom factor").default_num(2.0).step(),
                    other => ctx.log(format!("The {other} option is not implemented in triceratops yet.")),
                }
                Step::Done
            }
            Input::Number(f) => {
                ctx.requests.push(AppRequest::CameraOp { op: "Zoom".into(), dx: f.abs().max(0.01), dy: 0.0 });
                Step::Done
            }
            Input::Empty => {
                ctx.requests.push(AppRequest::ZoomWindow);
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct SetViewCmd;

impl Command for SetViewCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::options("Choose a view")
            .opts([opt("Top"), opt("Bottom"), opt("Left"), opt("Right"), opt("Front"), opt("Back"), opt("Perspective")])
            .step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        if let Input::Option(o) = input {
            ctx.requests.push(AppRequest::SetView(o));
        }
        Step::Done
    }
}

#[derive(Default)]
pub struct SetDisplayModeCmd;

impl Command for SetDisplayModeCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::options("Display mode")
            .opts([opt("Wireframe"), opt("Shaded"), opt("Rendered"), opt("Ghosted"), opt("XRay"), opt("Technical"), opt("Pen"), opt("Arctic"), opt("Zebra"), opt("EMap"), opt("Curvature"), opt("DraftAngle")])
            .step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        if let Input::Option(o) = input {
            ctx.requests.push(AppRequest::SetDisplayMode(o));
        }
        Step::Done
    }
}

#[derive(Default)]
pub struct GridCmd {
    asking: Option<&'static str>,
}

impl GridCmd {
    fn prompt(&self, ctx: &CmdCtx) -> Prompt {
        let s = &ctx.doc.settings;
        Prompt::options("Grid options").opts([
            optv("SnapSpacing", fmt_num(s.grid_snap_spacing)),
            optv("MinorLineSpacing", fmt_num(s.grid_minor_spacing)),
            optv("MajorLineInterval", s.grid_major_every),
            optv("Extents", fmt_num(s.grid_minor_spacing * s.grid_line_count as f64)),
            optv("ShowGrid", yesno(s.show_grid)),
            optv("ShowGridAxes", yesno(s.show_grid_axes)),
            optv("ShowWorldAxes", yesno(s.show_world_axes)),
        ])
    }
}

impl Command for GridCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        self.prompt(ctx).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => match o.as_str() {
                "ShowGrid" => {
                    ctx.doc.settings.show_grid = !ctx.doc.settings.show_grid;
                    self.prompt(ctx).step()
                }
                "ShowGridAxes" => {
                    ctx.doc.settings.show_grid_axes = !ctx.doc.settings.show_grid_axes;
                    self.prompt(ctx).step()
                }
                "ShowWorldAxes" => {
                    ctx.doc.settings.show_world_axes = !ctx.doc.settings.show_world_axes;
                    self.prompt(ctx).step()
                }
                "SnapSpacing" => {
                    self.asking = Some("SnapSpacing");
                    Prompt::number("Snap spacing").default_num(ctx.doc.settings.grid_snap_spacing).step()
                }
                "MinorLineSpacing" => {
                    self.asking = Some("MinorLineSpacing");
                    Prompt::number("Minor grid line spacing").default_num(ctx.doc.settings.grid_minor_spacing).step()
                }
                "MajorLineInterval" => {
                    self.asking = Some("MajorLineInterval");
                    Prompt::number("Major lines every").default_num(ctx.doc.settings.grid_major_every as f64).step()
                }
                "Extents" => {
                    self.asking = Some("Extents");
                    Prompt::number("Grid extents").default_num(ctx.doc.settings.grid_minor_spacing * ctx.doc.settings.grid_line_count as f64).step()
                }
                _ => self.prompt(ctx).step(),
            },
            Input::Number(v) => {
                let s = &mut ctx.doc.settings;
                match self.asking.take() {
                    Some("SnapSpacing") => s.grid_snap_spacing = v.abs().max(1e-6),
                    Some("MinorLineSpacing") => s.grid_minor_spacing = v.abs().max(1e-6),
                    Some("MajorLineInterval") => s.grid_major_every = (v.round() as i32).max(1),
                    Some("Extents") => s.grid_line_count = ((v.abs() / s.grid_minor_spacing).round() as i32).clamp(1, 100_000),
                    _ => {}
                }
                ctx.doc.modified = true;
                self.prompt(ctx).step()
            }
            Input::Empty => {
                self.asking = None;
                if self.asking.is_none() {
                    Step::Done
                } else {
                    self.prompt(ctx).step()
                }
            }
            _ => Step::Done,
        }
    }
}

pub struct CommandListCmd;

impl Command for CommandListCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let all = super::registry::all_commands();
        let implemented = all.iter().filter(|c| super::registry::is_implemented(c.name)).count();
        for c in &all {
            let a = if c.aliases.is_empty() { String::new() } else { format!(" ({})", c.aliases.join(", ")) };
            let mark = if super::registry::is_implemented(c.name) { "" } else { " [not implemented yet]" };
            ctx.log(format!("  {}{a}: {}{mark}", c.name, c.description));
        }
        ctx.log(format!("{} commands, {implemented} implemented in triceratops.", all.len()));
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

/// A command that only forwards a request to the application.
pub struct RequestCmd(pub AppRequest);

impl Command for RequestCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        ctx.requests.push(self.0.clone());
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}
