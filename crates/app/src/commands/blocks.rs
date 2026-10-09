//! Block commands: Block (define), Insert, ExplodeBlock, BlockManager (list), SelBlockInstance.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::{BlockDef, Document, Geometry, ObjectId};

use super::{fmt_num, opt, optv, CmdCtx, Command, Input, ObjFilter, Preview, Prompt, Step};

#[derive(Default)]
pub struct BlockCmd {
    ids: Vec<ObjectId>,
    base: Option<DVec3>,
}

impl Command for BlockCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select objects to define block", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.base, input) {
            (_, Input::Objects(ids)) => {
                self.ids = ids;
                Prompt::point("Block base point").step()
            }
            (None, Input::Point(p)) => {
                self.base = Some(p);
                let n = ctx.doc.state.blocks.len() + 1;
                Prompt::text("Block name").default(format!("Block {n:02}")).step()
            }
            (Some(base), inp @ (Input::Text(_) | Input::Empty)) => {
                let typed = match &inp { Input::Text(t) => t.clone(), _ => String::new() };
                let name = match input_name(&typed) { Some(n) => n, None => format!("Block {:02}", ctx.doc.state.blocks.len() + 1) };
                if ctx.doc.find_block(&name).is_some() {
                    ctx.log(format!("A block named \"{name}\" already exists."));
                    return Prompt::text("Block name").step();
                }
                let base = *base;
                let shift = DAffine3::from_translation(-base);
                let mut objects = Vec::new();
                for id in &self.ids {
                    if let Some(o) = ctx.doc.get(*id) {
                        let mut g = o.geometry.clone();
                        g.transform(&shift);
                        objects.push((g, o.attributes.clone()));
                    }
                }
                if objects.is_empty() {
                    return Step::Cancelled;
                }
                ctx.doc.begin_undo("Block");
                let idx = ctx.doc.add_block(BlockDef { name: name.clone(), base: DVec3::ZERO, objects });
                let layer = ctx.doc.current_layer();
                for id in &self.ids {
                    ctx.doc.remove(*id);
                }
                ctx.doc.clear_selection();
                let nid = ctx.doc.add_with(Geometry::Instance { block: idx, xf: DAffine3::from_translation(base) }, tri_doc::Attributes { layer, ..Default::default() });
                ctx.doc.select(nid, true);
                ctx.log(format!("Block definition \"{name}\" created with {} objects.", ctx.doc.state.blocks[idx].objects.len()));
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.base
    }
}

fn input_name(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

pub struct InsertCmd {
    block: Option<usize>,
    point: Option<DVec3>,
    scale: f64,
    rotation: f64,
    from_file: bool,
}

impl Default for InsertCmd {
    fn default() -> Self {
        Self { block: None, point: None, scale: 1.0, rotation: 0.0, from_file: false }
    }
}

impl InsertCmd {
    fn xf(&self, p: DVec3, cplane_z: DVec3) -> DAffine3 {
        DAffine3::from_translation(p) * DAffine3::from_quat(DQuat::from_axis_angle(cplane_z, self.rotation.to_radians())) * DAffine3::from_scale(DVec3::splat(self.scale))
    }
}

impl Command for InsertCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        if ctx.doc.state.blocks.is_empty() {
            ctx.log("No block definitions in the document. Use Block to define one, or Insert a file via File > Import.");
            return Step::Cancelled;
        }
        let names: Vec<String> = ctx.doc.state.blocks.iter().map(|b| b.name.clone()).collect();
        ctx.log(format!("Block definitions: {}", names.join(", ")));
        Prompt::text("Name of block to insert").default(names.last().cloned().unwrap_or_default()).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.block, self.point, input) {
            (None, _, inp @ (Input::Text(_) | Input::Empty)) => {
                let typed = match &inp { Input::Text(t) => t.clone(), _ => String::new() };
                let name = input_name(&typed).or_else(|| ctx.doc.state.blocks.last().map(|b| b.name.clone())).unwrap_or_default();
                let Some(idx) = ctx.doc.find_block(&name) else {
                    ctx.log(format!("No block named \"{name}\"."));
                    return Prompt::text("Name of block to insert").step();
                };
                self.block = Some(idx);
                Prompt::point("Insertion point").opts([optv("Scale", fmt_num(self.scale)), optv("Rotation", fmt_num(self.rotation)), optv("InsertAs", "BlockInstance"), opt("File")]).step()
            }
            (_, _, Input::Option(o)) => match o.as_str() {
                "Scale" => {
                    ctx.mem_set("Insert.Asking", "Scale");
                    Prompt::number("Scale factor").default_num(self.scale).step()
                }
                "Rotation" => {
                    ctx.mem_set("Insert.Asking", "Rotation");
                    Prompt::number("Rotation angle").default_num(self.rotation).step()
                }
                "InsertAs" => {
                    self.from_file = !self.from_file;
                    Prompt::point("Insertion point").opts([optv("InsertAs", if self.from_file { "Group" } else { "BlockInstance" })]).step()
                }
                _ => Prompt::point("Insertion point").step(),
            },
            (_, _, Input::Number(v)) => {
                match ctx.memory.remove("Insert.Asking").as_deref() {
                    Some("Scale") => self.scale = if v.abs() < 1e-9 { 1.0 } else { v },
                    Some("Rotation") => self.rotation = v,
                    _ => {}
                }
                Prompt::point("Insertion point").opts([optv("Scale", fmt_num(self.scale)), optv("Rotation", fmt_num(self.rotation))]).step()
            }
            (Some(idx), None, Input::Point(p)) => {
                let xf = self.xf(p, ctx.cplane.z_axis);
                ctx.doc.begin_undo("Insert");
                ctx.doc.clear_selection();
                if self.from_file {
                    let parts = ctx.doc.instance_geometry(&Geometry::Instance { block: idx, xf });
                    let group = format!("Group{:02}", ctx.doc.len());
                    for (g, mut a) in parts {
                        a.groups.push(group.clone());
                        let id = ctx.doc.add_with(g, a);
                        ctx.doc.select(id, true);
                    }
                } else {
                    let layer = ctx.doc.current_layer();
                    let id = ctx.doc.add_with(Geometry::Instance { block: idx, xf }, tri_doc::Attributes { layer, ..Default::default() });
                    ctx.doc.select(id, true);
                }
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, doc: &Document) -> Vec<Preview> {
        let (Some(idx), Some(p)) = (self.block, cursor) else { return vec![] };
        let xf = self.xf(p, DVec3::Z);
        let mut out = Vec::new();
        for (g, _) in doc.instance_geometry(&Geometry::Instance { block: idx, xf }) {
            match g {
                Geometry::Curve(c) => out.push(Preview::Curve(c)),
                Geometry::Point(p) => out.push(Preview::Point(p)),
                Geometry::Surface(s) => out.push(Preview::Surface(s)),
                Geometry::Brep(b) => out.push(Preview::Brep(b)),
                _ => {}
            }
        }
        out
    }
}

pub struct ExplodeBlockCmd;

impl Command for ExplodeBlockCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::objects("Select block instances to explode", ObjFilter::Any, 1).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        let Input::Objects(ids) = input else { return Step::Cancelled };
        ctx.doc.begin_undo("ExplodeBlock");
        let mut n = 0;
        for id in ids {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            if !matches!(o.geometry, Geometry::Instance { .. }) {
                continue;
            }
            let parts = ctx.doc.instance_geometry(&o.geometry);
            ctx.doc.remove(id);
            for (g, a) in parts {
                let nid = ctx.doc.add_with(g, tri_doc::Attributes { layer: o.attributes.layer, ..a });
                ctx.doc.select(nid, true);
                n += 1;
            }
        }
        ctx.log(format!("{n} objects created from exploded blocks."));
        Step::Done
    }
}

pub struct BlockManagerCmd;

impl Command for BlockManagerCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        if ctx.doc.state.blocks.is_empty() {
            ctx.log("No block definitions in the document.");
        }
        let counts: Vec<usize> = (0..ctx.doc.state.blocks.len())
            .map(|i| ctx.doc.objects().filter(|o| matches!(o.geometry, Geometry::Instance { block, .. } if block == i)).count())
            .collect();
        let lines: Vec<String> = ctx.doc.state.blocks.iter().enumerate().map(|(i, b)| format!("  {}: {} objects, {} instances", b.name, b.objects.len(), counts[i])).collect();
        for l in lines {
            ctx.log(l);
        }
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub struct SelBlockInstanceCmd;

impl Command for SelBlockInstanceCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| matches!(o.geometry, Geometry::Instance { .. }) && ctx.doc.is_selectable(o)).map(|o| o.id).collect();
        for id in &ids {
            ctx.doc.select(*id, true);
        }
        ctx.log(format!("{} block instances added to selection.", ids.len()));
        Step::Done
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}
