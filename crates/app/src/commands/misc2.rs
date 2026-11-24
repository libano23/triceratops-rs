//! More selection, layer, group, clipboard, visibility, view and analysis commands.

use glam::DVec3;
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{curve_ops::arc_center, BoundingBox, Brep, NurbsCurve, Plane};

use super::{fmt_num, opt, optv, yesno, AppRequest, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

fn select_where(ctx: &mut CmdCtx, label: &str, pred: impl Fn(&Document, &tri_doc::Object) -> bool) -> Step {
    let before = ctx.doc.selection.len();
    let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && pred(ctx.doc, o)).map(|o| o.id).collect();
    for id in &ids {
        ctx.doc.select(*id, true);
    }
    let added = ctx.doc.selection.len() - before;
    ctx.log(format!("{added} {label} added to selection. {} total.", ctx.doc.selection.len()));
    Step::Done
}

pub enum SelKind {
    OpenCrv,
    ClosedCrv,
    Line,
    Polyline,
    OpenSrf,
    ClosedSrf,
    OpenPolysrf,
    ClosedPolysrf,
    Dup,
    Prev,
    Visible,
    Hidden,
    Locked,
}

pub struct SelCmd(pub SelKind);

impl Command for SelCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.0 {
            SelKind::OpenCrv => select_where(ctx, "open curves", |_, o| o.geometry.as_curve().is_some_and(|c| !c.is_closed())),
            SelKind::ClosedCrv => select_where(ctx, "closed curves", |_, o| o.geometry.as_curve().is_some_and(|c| c.is_closed())),
            SelKind::Line => select_where(ctx, "lines", |_, o| o.geometry.as_curve().is_some_and(|c| c.degree == 1 && c.control_points.len() == 2)),
            SelKind::Polyline => select_where(ctx, "polylines", |_, o| o.geometry.as_curve().is_some_and(|c| c.degree == 1 && c.control_points.len() > 2)),
            SelKind::OpenSrf => select_where(ctx, "open surfaces", |_, o| matches!(&o.geometry, Geometry::Surface(s) if !tri_doc::surface_is_closed(s))),
            SelKind::ClosedSrf => select_where(ctx, "closed surfaces", |_, o| matches!(&o.geometry, Geometry::Surface(s) if tri_doc::surface_is_closed(s))),
            SelKind::OpenPolysrf => select_where(ctx, "open polysurfaces", |_, o| matches!(&o.geometry, Geometry::Brep(b) if !b.closed)),
            SelKind::ClosedPolysrf => select_where(ctx, "closed polysurfaces", |_, o| matches!(&o.geometry, Geometry::Brep(b) if b.closed)),
            SelKind::Visible => select_where(ctx, "objects", |_, _| true),
            SelKind::Hidden => {
                let n = ctx.doc.objects().filter(|o| o.attributes.hidden).count();
                ctx.log(format!("{n} hidden objects (hidden objects cannot be selected; use Show)."));
                Step::Done
            }
            SelKind::Locked => {
                let n = ctx.doc.objects().filter(|o| ctx.doc.is_locked(o)).count();
                ctx.log(format!("{n} locked objects (locked objects cannot be selected; use Unlock)."));
                Step::Done
            }
            SelKind::Prev => {
                let ids: Vec<ObjectId> = ctx.memory.get("SelPrev").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                for id in &ids {
                    ctx.doc.select(*id, true);
                }
                ctx.log(format!("{} objects added to selection. {} total.", ids.len(), ctx.doc.selection.len()));
                Step::Done
            }
            SelKind::Dup => {
                // objects whose geometry equals another object's geometry (all but the first of each set)
                let objs: Vec<(ObjectId, Geometry)> = ctx.doc.objects().map(|o| (o.id, o.geometry.clone())).collect();
                let mut dups = Vec::new();
                for i in 0..objs.len() {
                    for j in 0..i {
                        if objs[i].1 == objs[j].1 {
                            dups.push(objs[i].0);
                            break;
                        }
                    }
                }
                let n = dups.len();
                for id in dups {
                    ctx.doc.select(id, true);
                }
                ctx.log(format!("{n} duplicate objects added to selection."));
                Step::Done
            }
        }
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub struct SelNameCmd;

impl Command for SelNameCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::text("Name of objects to select").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Text(name) = input else { return Step::Done };
        let name = name.trim().to_string();
        select_where(ctx, "objects", move |_, o| o.attributes.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&name) || wildcard(&name, n)))
    }
}

fn wildcard(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_ascii_lowercase().chars().collect();
    let t: Vec<char> = text.to_ascii_lowercase().chars().collect();
    fn rec(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => rec(&p[1..], t) || (!t.is_empty() && rec(p, &t[1..])),
            (Some('?'), Some(_)) => rec(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => rec(&p[1..], &t[1..]),
            _ => false,
        }
    }
    rec(&p, &t)
}

pub struct SelSmallCmd;

impl Command for SelSmallCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::number("Select objects smaller than").default_num(1.0).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Number(size) = input else { return Step::Done };
        select_where(ctx, "small objects", move |_, o| o.geometry.bounding_box().diagonal() < size)
    }
}

pub struct SelIdCmd;

impl Command for SelIdCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::text("Object ID to select").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Text(t) = input else { return Step::Done };
        match t.trim().trim_start_matches('#').parse::<ObjectId>() {
            Ok(id) if ctx.doc.get(id).is_some() => {
                ctx.doc.select(id, true);
                ctx.log(format!("Object #{id} selected."));
            }
            _ => ctx.log("No object with that ID."),
        }
        Step::Done
    }
}

pub struct SelLayerCmd;

impl Command for SelLayerCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let names: Vec<String> = ctx.doc.layers().iter().map(|l| l.name.clone()).collect();
        ctx.log(format!("Layers: {}", names.join(", ")));
        Prompt::text("Layer name").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Text(name) = input else { return Step::Done };
        let Some(idx) = ctx.doc.find_layer(name.trim()) else {
            ctx.log(format!("No layer named \"{}\".", name.trim()));
            return Step::Done;
        };
        select_where(ctx, "objects", move |_, o| o.attributes.layer == idx)
    }
}

/// Window / crossing selection from two picked points.
pub struct SelWindowCmd {
    crossing: bool,
    first: Option<DVec3>,
}

impl SelWindowCmd {
    pub fn new(crossing: bool) -> Self {
        Self { crossing, first: None }
    }
}

impl Command for SelWindowCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("First corner of window").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, input) {
            (None, Input::Point(p)) => {
                self.first = Some(p);
                Prompt::point("Other corner of window").step()
            }
            (Some(a), Input::Point(b)) => {
                let bb = BoundingBox::from_points(&[a, b]).inflate(1e-9);
                let crossing = self.crossing;
                let cp = ctx.cplane;
                select_where(ctx, "objects", move |_, o| {
                    let ob = o.geometry.bounding_box();
                    let inside = |p: DVec3| {
                        let (u, v, _) = cp.closest_parameters(p);
                        let (u0, v0, _) = cp.closest_parameters(bb.min);
                        let (u1, v1, _) = cp.closest_parameters(bb.max);
                        u >= u0.min(u1) && u <= u0.max(u1) && v >= v0.min(v1) && v <= v0.max(v1)
                    };
                    let corners = ob.corners();
                    if crossing {
                        corners.iter().any(|c| inside(*c))
                    } else {
                        corners.iter().all(|c| inside(*c))
                    }
                })
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.first, cursor) {
            (Some(a), Some(b)) => vec![Preview::Polyline(super::create::rect_points(&Plane::world_xy(), a, b))],
            _ => vec![],
        }
    }
}

// ---- layers --------------------------------------------------------------

pub enum LayerOp {
    On,
    Off,
    Lock,
    Unlock,
    OneOn,
    New,
    SetCurrentToObject,
}

pub struct LayerOpCmd(pub LayerOp);

impl Command for LayerOpCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.0 {
            LayerOp::New => Prompt::text("New layer name").default(format!("Layer {:02}", ctx.doc.layers().len())).step(),
            LayerOp::SetCurrentToObject => Prompt::objects("Select object whose layer becomes current", ObjFilter::Any, 1).step(),
            _ => {
                let names: Vec<String> = ctx.doc.layers().iter().map(|l| l.name.clone()).collect();
                ctx.log(format!("Layers: {}", names.join(", ")));
                Prompt::text("Layer name").step()
            }
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.0, input) {
            (LayerOp::New, Input::Text(name)) => {
                ctx.doc.begin_undo("NewLayer");
                let i = ctx.doc.add_layer(name.trim());
                ctx.doc.set_current_layer(i);
                Step::Done
            }
            (LayerOp::SetCurrentToObject, Input::Objects(ids)) => {
                if let Some(o) = ids.first().and_then(|id| ctx.doc.get(*id)) {
                    let l = o.attributes.layer;
                    ctx.doc.set_current_layer(l);
                }
                Step::Done
            }
            (op, Input::Text(name)) => {
                let Some(idx) = ctx.doc.find_layer(name.trim()) else {
                    ctx.log(format!("No layer named \"{}\".", name.trim()));
                    return Step::Done;
                };
                ctx.doc.begin_undo("Layer");
                match op {
                    LayerOp::On => ctx.doc.layer_mut(idx).map(|l| l.visible = true),
                    LayerOp::Off => {
                        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| o.attributes.layer == idx).map(|o| o.id).collect();
                        for id in ids {
                            ctx.doc.select(id, false);
                        }
                        ctx.doc.layer_mut(idx).map(|l| l.visible = false)
                    }
                    LayerOp::Lock => ctx.doc.layer_mut(idx).map(|l| l.locked = true),
                    LayerOp::Unlock => ctx.doc.layer_mut(idx).map(|l| l.locked = false),
                    LayerOp::OneOn => {
                        for i in 0..ctx.doc.layers().len() {
                            if let Some(l) = ctx.doc.layer_mut(i) {
                                l.visible = i == idx;
                            }
                        }
                        ctx.doc.set_current_layer(idx);
                        ctx.doc.clear_selection();
                        Some(())
                    }
                    _ => None,
                };
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

#[derive(Default)]
pub struct CopyToLayerCmd {
    ids: Vec<ObjectId>,
}


impl Command for CopyToLayerCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to copy to a layer", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::text("Layer name").step()
            }
            Input::Text(name) => {
                let Some(idx) = ctx.doc.find_layer(name.trim()) else {
                    ctx.log(format!("No layer named \"{}\".", name.trim()));
                    return Step::Done;
                };
                ctx.doc.begin_undo("CopyToLayer");
                for id in &self.ids {
                    if let Some(o) = ctx.doc.get(*id).cloned() {
                        let mut a = o.attributes;
                        a.layer = idx;
                        ctx.doc.add_with(o.geometry, a);
                    }
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
}

// ---- groups --------------------------------------------------------------

pub enum GroupOp {
    Group,
    Ungroup,
    RemoveFrom,
    SetName,
    Select,
}

pub struct GroupCmd {
    op: GroupOp,
    ids: Vec<ObjectId>,
}

impl GroupCmd {
    pub fn new(op: GroupOp) -> Self {
        Self { op, ids: Vec::new() }
    }
}

impl Command for GroupCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        match self.op {
            GroupOp::Group => Prompt::objects("Select objects to group", ObjFilter::Any, 1).step(),
            GroupOp::Ungroup => Prompt::objects("Select groups to ungroup", ObjFilter::Any, 1).step(),
            GroupOp::RemoveFrom => Prompt::objects("Select objects to remove from their group", ObjFilter::Any, 1).step(),
            GroupOp::SetName => Prompt::objects("Select group to rename", ObjFilter::Any, 1).step(),
            GroupOp::Select => Prompt::text("Group name to select").step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.op, input) {
            (GroupOp::Group, Input::Objects(ids)) => {
                let n = ctx.memory.get("GroupCounter").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) + 1;
                ctx.mem_set("GroupCounter", n);
                let name = format!("Group{n:02}");
                ctx.doc.begin_undo("Group");
                for id in &ids {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        o.attributes.groups.push(name.clone());
                    }
                }
                ctx.log(format!("{} objects grouped as {name}.", ids.len()));
                Step::Done
            }
            (GroupOp::Ungroup, Input::Objects(ids)) => {
                ctx.doc.begin_undo("Ungroup");
                for id in &ids {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        o.attributes.groups.pop();
                    }
                }
                Step::Done
            }
            (GroupOp::RemoveFrom, Input::Objects(ids)) => {
                ctx.doc.begin_undo("RemoveFromGroup");
                for id in &ids {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        o.attributes.groups.clear();
                    }
                }
                Step::Done
            }
            (GroupOp::SetName, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::text("New group name").step()
            }
            (GroupOp::SetName, Input::Text(name)) => {
                let name = name.trim().to_string();
                let olds: Vec<String> = self.ids.iter().filter_map(|id| ctx.doc.get(*id)).filter_map(|o| o.attributes.groups.last().cloned()).collect();
                ctx.doc.begin_undo("SetGroupName");
                let all: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).collect();
                for id in all {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        for g in o.attributes.groups.iter_mut() {
                            if olds.contains(g) {
                                *g = name.clone();
                            }
                        }
                    }
                }
                Step::Done
            }
            (GroupOp::Select, Input::Text(name)) => {
                let name = name.trim().to_string();
                select_where(ctx, "objects", move |_, o| o.attributes.groups.iter().any(|g| g.eq_ignore_ascii_case(&name)))
            }
            _ => Step::Cancelled,
        }
    }
}

/// AddToGroup needs two object prompts; handled by a dedicated command.
#[derive(Default)]
pub struct AddToGroupCmd {
    ids: Vec<ObjectId>,
}


impl Command for AddToGroupCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to add to a group", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        if self.ids.is_empty() {
            self.ids = ids;
            ctx.doc.clear_selection();
            return Prompt::objects("Select a member of the target group", ObjFilter::Any, 1).step();
        }
        let Some(group) = ids.first().and_then(|id| ctx.doc.get(*id)).and_then(|o| o.attributes.groups.last().cloned()) else {
            ctx.log("The selected object is not in a group.");
            return Step::Done;
        };
        ctx.doc.begin_undo("AddToGroup");
        for id in &self.ids {
            if let Some(o) = ctx.doc.get_mut(*id) {
                if !o.attributes.groups.contains(&group) {
                    o.attributes.groups.push(group.clone());
                }
            }
        }
        Step::Done
    }
}

// ---- visibility ----------------------------------------------------------

pub enum VisOp {
    HideSwap,
    ShowSelected,
    Isolate,
    Unisolate,
}

pub struct VisCmd(pub VisOp);

impl Command for VisCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        match self.0 {
            VisOp::HideSwap => {
                ctx.doc.begin_undo("HideSwap");
                let ids: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).collect();
                for id in ids {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.hidden = !o.attributes.hidden;
                    }
                }
                ctx.doc.clear_selection();
                Step::Done
            }
            VisOp::ShowSelected => {
                let hidden: Vec<ObjectId> = ctx.doc.objects().filter(|o| o.attributes.hidden).map(|o| o.id).collect();
                if hidden.is_empty() {
                    ctx.log("No hidden objects.");
                    return Step::Done;
                }
                // temporarily show everything so hidden objects can be picked
                ctx.doc.begin_undo("ShowSelected");
                for id in &hidden {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        o.attributes.hidden = false;
                    }
                }
                ctx.mem_set("ShowSelected.Hidden", hidden.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
                Prompt::objects("Select objects to show", ObjFilter::Any, 1).step()
            }
            VisOp::Isolate => Prompt::objects("Select objects to isolate", ObjFilter::Any, 1).step(),
            VisOp::Unisolate => {
                let ids: Vec<ObjectId> = ctx.memory.get("Isolate.Hidden").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                ctx.doc.begin_undo("Unisolate");
                for id in ids {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.hidden = false;
                    }
                }
                ctx.memory.remove("Isolate.Hidden");
                Step::Done
            }
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.0, input) {
            (VisOp::ShowSelected, inp) => {
                let keep: Vec<ObjectId> = if let Input::Objects(ids) = inp { ids } else { Vec::new() };
                let hidden: Vec<ObjectId> = ctx.memory.get("ShowSelected.Hidden").map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
                for id in hidden {
                    if !keep.contains(&id) {
                        if let Some(o) = ctx.doc.get_mut(id) {
                            o.attributes.hidden = true;
                        }
                    }
                }
                ctx.doc.clear_selection();
                Step::Done
            }
            (VisOp::Isolate, Input::Objects(ids)) => {
                ctx.doc.begin_undo("Isolate");
                let others: Vec<ObjectId> = ctx.doc.objects().filter(|o| !ids.contains(&o.id) && !o.attributes.hidden).map(|o| o.id).collect();
                for id in &others {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        o.attributes.hidden = true;
                    }
                }
                ctx.mem_set("Isolate.Hidden", others.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","));
                Step::Done
            }
            _ => Step::Done,
        }
    }
}

// ---- document / misc -----------------------------------------------------

pub struct PurgeCmd;

impl Command for PurgeCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let used: Vec<usize> = ctx.doc.objects().map(|o| o.attributes.layer).collect();
        let mut removed = 0;
        let mut i = ctx.doc.layers().len();
        while i > 1 {
            i -= 1;
            if !used.contains(&i) && i != ctx.doc.current_layer() {
                if removed == 0 {
                    ctx.doc.begin_undo("Purge");
                }
                ctx.doc.delete_layer(i);
                removed += 1;
            }
        }
        ctx.log(format!("{removed} unused layers purged."));
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

#[derive(Default)]
pub struct SetObjectNameCmd {
    ids: Vec<ObjectId>,
}


impl Command for SetObjectNameCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to name", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Objects(ids) => {
                self.ids = ids;
                Prompt::text("Object name").step()
            }
            Input::Text(name) => {
                ctx.doc.begin_undo("SetObjectName");
                for id in &self.ids {
                    if let Some(o) = ctx.doc.get_mut(*id) {
                        o.attributes.name = if name.trim().is_empty() { None } else { Some(name.trim().to_string()) };
                    }
                }
                Step::Done
            }
            Input::Empty => Step::Done,
            _ => Step::Cancelled,
        }
    }
}

pub struct BoundingBoxCmd;

impl Command for BoundingBoxCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects for bounding box calculation", ObjFilter::Any, 1).opts([optv("CoordinateSystem", "World"), optv("Output", "Curves")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Done };
        let bb = ids.iter().filter_map(|id| ctx.doc.get(*id)).fold(BoundingBox::EMPTY, |b, o| b.union(&o.geometry.bounding_box()));
        if bb.is_empty() {
            return Step::Done;
        }
        ctx.log(format!("Bounding box: ({}, {}, {}) to ({}, {}, {})", fmt_num(bb.min.x), fmt_num(bb.min.y), fmt_num(bb.min.z), fmt_num(bb.max.x), fmt_num(bb.max.y), fmt_num(bb.max.z)));
        ctx.doc.begin_undo("BoundingBox");
        let d = bb.max - bb.min;
        if d.z.abs() < 1e-9 {
            let pts = super::create::rect_points(&Plane { origin: DVec3::new(0.0, 0.0, bb.min.z), ..Plane::world_xy() }, bb.min, bb.max);
            if let Ok(c) = NurbsCurve::polyline(pts) {
                ctx.doc.add(Geometry::Curve(c));
            }
        } else if let Ok(b) = Brep::cuboid(&Plane { origin: bb.min, ..Plane::world_xy() }, d.x, d.y, d.z) {
            ctx.doc.add(Geometry::Brep(b));
        }
        Step::Done
    }
}

pub enum Measure {
    Area,
    AreaCentroid,
    Volume,
    VolumeCentroid,
}

pub struct MeasureCmd(pub Measure);

impl Command for MeasureCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        let msg = match self.0 {
            Measure::Area | Measure::AreaCentroid => "Select closed curves, surfaces, polysurfaces or meshes for area calculation",
            _ => "Select closed surfaces, polysurfaces or meshes for volume calculation",
        };
        Prompt::objects(msg, ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Done };
        let units = ctx.doc.settings.units.to_ascii_lowercase();
        match self.0 {
            Measure::Area | Measure::AreaCentroid => {
                let mut total = 0.0;
                let mut centroid = DVec3::ZERO;
                for id in &ids {
                    if let Some(o) = ctx.doc.get(*id) {
                        if let Some(a) = Document::area_of(&o.geometry) {
                            total += a;
                            centroid += o.geometry.bounding_box().center() * a;
                        }
                    }
                }
                ctx.log(format!("Area = {} square {units}", fmt_num(total)));
                if matches!(self.0, Measure::AreaCentroid) && total > 0.0 {
                    let c = centroid / total;
                    ctx.log(format!("Area centroid = {}, {}, {}", fmt_num(c.x), fmt_num(c.y), fmt_num(c.z)));
                    ctx.doc.begin_undo("AreaCentroid");
                    ctx.doc.add(Geometry::Point(c));
                }
            }
            Measure::Volume | Measure::VolumeCentroid => {
                let mut total = 0.0;
                let mut centroid = DVec3::ZERO;
                let mut any = false;
                for id in &ids {
                    if let Some(o) = ctx.doc.get(*id) {
                        match Document::volume_of(&o.geometry) {
                            Some((v, c)) => {
                                total += v;
                                centroid += c * v;
                                any = true;
                            }
                            None => ctx.log(format!("Object #{} is not closed; volume skipped.", o.id)),
                        }
                    }
                }
                if any {
                    ctx.log(format!("Volume = {} cubic {units}", fmt_num(total)));
                    if matches!(self.0, Measure::VolumeCentroid) && total > 0.0 {
                        let c = centroid / total;
                        ctx.log(format!("Volume centroid = {}, {}, {}", fmt_num(c.x), fmt_num(c.y), fmt_num(c.z)));
                        ctx.doc.begin_undo("VolumeCentroid");
                        ctx.doc.add(Geometry::Point(c));
                    }
                }
            }
        }
        Step::Done
    }
}

#[derive(Default)]
pub struct AngleCmd {
    pts: Vec<DVec3>,
}

impl Command for AngleCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Start of first line").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Point(p) = input else { return Step::Cancelled };
        self.pts.push(p);
        match self.pts.len() {
            1 => Prompt::point("End of first line").step(),
            2 => Prompt::point("Start of second line").step(),
            3 => Prompt::point("End of second line").step(),
            _ => {
                let a = (self.pts[1] - self.pts[0]).normalize_or(DVec3::X);
                let b = (self.pts[3] - self.pts[2]).normalize_or(DVec3::X);
                let ang = a.dot(b).clamp(-1.0, 1.0).acos().to_degrees();
                ctx.log(format!("Angle = {} degrees", fmt_num(ang)));
                Step::Done
            }
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        let mut v = Vec::new();
        if self.pts.len() >= 2 {
            v.push(Preview::Line(self.pts[0], self.pts[1]));
        }
        if let (Some(c), Some(last)) = (cursor, self.pts.last()) {
            if self.pts.len() == 1 || self.pts.len() == 3 {
                v.push(Preview::Line(*last, c));
            }
        }
        v
    }
    fn base_point(&self) -> Option<DVec3> {
        self.pts.last().copied()
    }
}

pub struct RadiusCmd;

impl Command for RadiusCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::pick("Select curve for radius measurement", ObjFilter::Curve).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Pick(id, p) = input else { return Step::Done };
        let Some(c) = ctx.doc.get(id).and_then(|o| o.geometry.as_curve()) else { return Step::Done };
        match arc_center(c) {
            Some(center) => ctx.log(format!("Radius = {}", fmt_num((c.start_point() - center).length()))),
            None => {
                // curvature radius at the pick: |C'|^3 / |C' x C''|
                let t = c.closest_parameter(p);
                let (t0, t1) = c.domain();
                let h = (t1 - t0) * 1e-5;
                let d1 = c.derivative_at(t);
                let d2 = (c.derivative_at((t + h).min(t1)) - c.derivative_at((t - h).max(t0))) / (2.0 * h);
                let k = d1.cross(d2).length() / d1.length().powi(3).max(1e-18);
                if k < 1e-12 {
                    ctx.log("Radius = infinite (the curve is straight here)");
                } else {
                    ctx.log(format!("Radius = {}", fmt_num(1.0 / k)));
                }
            }
        }
        Step::Done
    }
}

pub struct EvaluatePtCmd;

impl Command for EvaluatePtCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::point("Point to evaluate").opts([optv("CoordinateSystem", "World")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Point(p) => {
                let (u, v, w) = ctx.cplane.closest_parameters(p);
                ctx.log(format!("World: {}, {}, {}   CPlane: {}, {}, {}", fmt_num(p.x), fmt_num(p.y), fmt_num(p.z), fmt_num(u), fmt_num(v), fmt_num(w)));
                Step::Done
            }
            Input::Option(_) => Prompt::point("Point to evaluate").step(),
            _ => Step::Cancelled,
        }
    }
}

pub struct ZoomWindowCmd {
    first: Option<DVec3>,
    target: bool,
}

impl ZoomWindowCmd {
    pub fn new(target: bool) -> Self {
        Self { first: None, target }
    }
}

impl Command for ZoomWindowCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        if self.target {
            Prompt::point("Target point").step()
        } else {
            Prompt::point("First corner of zoom window").step()
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.first, input) {
            (None, Input::Point(p)) if self.target => {
                self.first = Some(p);
                Prompt::point("Corner of zoom window").step()
            }
            (None, Input::Point(p)) => {
                self.first = Some(p);
                Prompt::point("Other corner of zoom window").step()
            }
            (Some(a), Input::Point(b)) => {
                let (a, b) = if self.target { (a - (b - a), b) } else { (a, b) };
                ctx.requests.push(AppRequest::ZoomWindowPoints(a, b));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &Document) -> Vec<Preview> {
        match (self.first, cursor) {
            (Some(a), Some(b)) => vec![Preview::Polyline(super::create::rect_points(&Plane::world_xy(), a, b))],
            _ => vec![],
        }
    }
}

pub struct NamedViewCmd;

impl Command for NamedViewCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::options("Named view").opts([opt("Save"), opt("Restore")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) => {
                ctx.mem_set("NamedView.Op", o.clone());
                Prompt::text("View name").step()
            }
            Input::Text(name) => {
                let restore = ctx.memory.get("NamedView.Op").is_some_and(|o| o == "Restore");
                ctx.requests.push(AppRequest::NamedView { name: name.trim().to_string(), restore });
                Step::Done
            }
            _ => Step::Done,
        }
    }
}

pub struct ListCmd;

impl Command for ListCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to list", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Done };
        for id in ids {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            ctx.log(format!("Object #{}: {}", o.id, o.describe_long()));
            match &o.geometry {
                Geometry::Curve(c) => {
                    ctx.log(format!("  degree {}  knots {:?}", c.degree, c.knots.iter().map(|k| fmt_num(*k)).collect::<Vec<_>>()));
                    for (i, p) in c.control_points.iter().enumerate() {
                        ctx.log(format!("  CV[{i}] = ({}, {}, {}) w={}", fmt_num(p.x), fmt_num(p.y), fmt_num(p.z), fmt_num(c.weights[i])));
                    }
                }
                Geometry::Surface(s) => ctx.log(format!("  degree {}x{}, {}x{} control points, domain u {:?} v {:?}", s.degree_u, s.degree_v, s.count_u, s.count_v, s.domain_u(), s.domain_v())),
                Geometry::Brep(b) => {
                    for (i, f) in b.faces.iter().enumerate() {
                        ctx.log(format!("  face {i}: degree {}x{}, {} trim loops", f.surface.degree_u, f.surface.degree_v, f.loops.len()));
                    }
                }
                Geometry::Mesh(m) => ctx.log(format!("  {} vertices, {} triangles", m.vertices.len(), m.triangle_count())),
                Geometry::Point(p) => ctx.log(format!("  ({}, {}, {})", fmt_num(p.x), fmt_num(p.y), fmt_num(p.z))),
                Geometry::Instance { block, xf } => {
                    let name = ctx.doc.state.blocks.get(*block).map(|b| b.name.clone()).unwrap_or_default();
                    ctx.log(format!("  block \"{name}\", insertion ({}, {}, {})", fmt_num(xf.translation.x), fmt_num(xf.translation.y), fmt_num(xf.translation.z)));
                }
            }
            if !o.attributes.groups.is_empty() {
                ctx.log(format!("  groups: {}", o.attributes.groups.join(", ")));
            }
        }
        Step::Done
    }
}

#[allow(dead_code)]
fn unused(_: &str) -> &'static str {
    yesno(true)
}
