//! View, viewport, animation, analysis-overlay, gumball, guide, block-editing, layer-state,
//! worksession and selection commands that act through the application state.

use glam::{DAffine3, DMat3, DVec3};
use tri_doc::{Document, Geometry, Object, ObjectId};
use tri_geom::{Mesh, NurbsCurve, Plane};

use super::misc4::solid_mesh;
use super::simple::{add_all, has_user_text, Ans, Ask, SeqCmd};
use super::{fmt_num, opt, AppRequest, CmdCtx, Command, Input, ObjFilter, Prompt, Step};

fn req(ctx: &mut CmdCtx, r: AppRequest) -> Step {
    ctx.requests.push(r);
    Step::Done
}

/// `SetOption` with a fixed value.
pub struct OptCmd(pub &'static str, pub &'static str);

impl Command for OptCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        req(ctx, AppRequest::SetOption { key: self.0.into(), value: self.1.into() })
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

// ---------------------------------------------------------------- guides

pub fn add_guide() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Start of guide"), Ask::Point("End of guide")], |a, ctx| req(ctx, AppRequest::Guide(Some(a[0].point()), Some(a[1].point()))))
}

pub fn remove_guide() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Remove", vec![opt("Nearest"), opt("All")])], |a, ctx| {
        if a[0].text() == "All" {
            return req(ctx, AppRequest::Guide(None, None));
        }
        Step::Prompt(Prompt::point("Point near the guide to remove"))
    })
    .then(|inp, ctx| {
        if let Input::Point(p) = inp {
            return req(ctx, AppRequest::Guide(Some(p), None));
        }
        Step::Cancelled
    })
}

// ---------------------------------------------------------------- camera

pub fn tilt_view() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Tilt angle (degrees)", 10.0)], |a, ctx| req(ctx, AppRequest::SetOption { key: "tilt".into(), value: fmt_num(a[0].number()) }))
}

pub fn dolly_zoom() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Lens change factor (>1 narrows the lens, the target keeps its size)", 1.5)], |a, ctx| req(ctx, AppRequest::SetOption { key: "dolly".into(), value: fmt_num(a[0].number()) }))
}

pub fn rotate_camera() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Horizontal angle (degrees)", 15.0), Ask::Number("Vertical angle (degrees)", 0.0)], |a, ctx| req(ctx, AppRequest::SetOption { key: "rotate_camera".into(), value: format!("{},{}", a[0].number(), a[1].number()) }))
}

pub fn set_target() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("New camera target")], |a, ctx| {
        let t = a[0].point();
        let eye = ctx.view_eye;
        req(ctx, AppRequest::LookAt { from: eye, to: t })
    })
}

pub fn set_camera() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Camera location"), Ask::Point("Camera target"), Ask::Number("Lens length (mm)", 50.0)], |a, ctx| {
        ctx.requests.push(AppRequest::LookAt { from: a[0].point(), to: a[1].point() });
        req(ctx, AppRequest::SetOption { key: "lens".into(), value: fmt_num(a[2].number()) })
    })
}

pub fn focus_distance() -> SeqCmd {
    SeqCmd::new(vec![Ask::PointOrNumber("Focus distance or point", 10.0)], |a, ctx| {
        let d = match &a[0] { Ans::Number(n) => n.abs(), Ans::Point(p) => (*p - ctx.view_eye).length(), _ => 10.0 };
        let to = ctx.view_eye + ctx.view_dir * d;
        req(ctx, AppRequest::LookAt { from: ctx.view_eye, to })
    })
}

pub fn orient_camera_to_srf() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select surface to look at", ObjFilter::Surface), Ask::Number("Distance from the surface", 10.0)], |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(faces) = super::surface_edit::faces_of(ctx.doc, *id) else { return Step::Cancelled };
        let (on, n) = super::misc6::pull_point(&faces, *p);
        req(ctx, AppRequest::LookAt { from: on + n * a[1].number(), to: on })
    })
}

pub fn perspective_match() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Background image file"), Ask::Point("Model point seen at the image centre"), Ask::Point("Camera location estimate")], |a, ctx| {
        let path = a[0].text();
        ctx.requests.push(AppRequest::LookAt { from: a[2].point(), to: a[1].point() });
        if !path.is_empty() {
            ctx.mem_set("Picture.Background", path.clone());
            ctx.log(format!("Use BackgroundBitmap to place {path}; the camera now looks from the estimate at the model point."));
        }
        Step::Done
    })
}

// ---------------------------------------------------------------- animation

pub fn anim_setup(kind: &'static str) -> SeqCmd {
    let asks = match kind {
        "path" => vec![Ask::Pick("Select camera path curve", ObjFilter::Curve), Ask::Point("Target point"), Ask::Number("Number of frames", 60.0)],
        "fly" => vec![Ask::Pick("Select camera path curve", ObjFilter::Curve), Ask::Pick("Select target path curve", ObjFilter::Curve), Ask::Number("Number of frames", 60.0)],
        _ => vec![Ask::Number("Number of frames", 30.0)],
    };
    SeqCmd::new(asks, move |a, ctx| {
        let curve = |ans: &Ans| -> Vec<DVec3> {
            let ids = ans.ids();
            let Some(o) = ids.first().and_then(|id| ctx.doc.get(*id)) else { return Vec::new() };
            let Some(c) = o.geometry.as_curve() else { return Vec::new() };
            let (t0, t1) = c.domain();
            (0..=100).map(|i| c.point_at(t0 + (t1 - t0) * i as f64 / 100.0)).collect()
        };
        let (path, targets, frames) = match kind {
            "path" => (curve(&a[0]), vec![a[1].point()], a[2].number()),
            "fly" => (curve(&a[0]), curve(&a[1]), a[2].number()),
            _ => (Vec::new(), Vec::new(), a[0].number()),
        };
        req(ctx, AppRequest::AnimSetup { kind: kind.into(), frames: frames.round().max(2.0) as usize, path, targets, record: None })
    })
}

pub struct AnimCtl(pub &'static str);

impl Command for AnimCtl {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        if self.0 == "frame" {
            return Prompt::number("Frame number").step();
        }
        req(ctx, AppRequest::AnimControl(self.0.into(), 0))
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Number(n) => req(ctx, AppRequest::AnimControl("frame".into(), n.round() as i64)),
            _ => Step::Cancelled,
        }
    }
}

pub fn export_bitmaps() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Output folder")], |a, ctx| {
        let dir = a[0].text();
        if dir.is_empty() {
            return Step::Cancelled;
        }
        let _ = std::fs::create_dir_all(&dir);
        ctx.mem_set("Animation.Folder", dir);
        req(ctx, AppRequest::AnimControl("record".into(), 0))
    })
}

// ---------------------------------------------------------------- viewports

pub struct ViewportOpCmd(pub &'static str);

impl Command for ViewportOpCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        req(ctx, AppRequest::ViewportOp(self.0.into()))
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub fn read_viewports() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Viewport file (.tri or .json saved by SaveViewports)")], |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        req(ctx, AppRequest::ViewportsFile { path: p, save: false })
    })
}

pub fn save_viewports() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("File to save the viewports to")], |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        req(ctx, AppRequest::ViewportsFile { path: p, save: true })
    })
}

pub fn zoom_border() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Zoom extents border (percent of the object size)", 5.0)], |a, ctx| req(ctx, AppRequest::SetOption { key: "zoom_border".into(), value: fmt_num(a[0].number()) }))
}

pub fn zoom_calibrate() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Pixels per millimetre of your screen (measure a ruler against the grid)", 3.78)], |a, ctx| req(ctx, AppRequest::SetOption { key: "px_per_mm".into(), value: fmt_num(a[0].number()) }))
}

pub fn drag_strength() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Drag strength (percent)", 100.0)], |a, ctx| req(ctx, AppRequest::SetOption { key: "drag_strength".into(), value: fmt_num(a[0].number()) }))
}

pub fn drag_mode() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Drag mode", vec![opt("CPlane"), opt("World"), opt("View"), opt("UVN"), opt("ControlPolygon")])], |a, ctx| req(ctx, AppRequest::SetOption { key: "drag_mode".into(), value: a[0].text() }))
}

pub fn gumball_op(op: &'static str) -> SeqCmd {
    let asks = match op {
        "relocate" => vec![Ask::Point("New gumball origin"), Ask::Point("X axis direction (Enter keeps it)")],
        "alignment" => vec![Ask::Options("Gumball alignment", vec![opt("CPlane"), opt("World"), opt("Object"), opt("View")])],
        "scale_mode" => vec![Ask::Options("Scale mode", vec![opt("Independent"), opt("Uniform")])],
        _ => vec![],
    };
    SeqCmd::new(asks, move |a, ctx| {
        let pts: Vec<DVec3> = a.iter().filter_map(|x| if let Ans::Point(p) = x { Some(*p) } else { None }).collect();
        let text = a.iter().find_map(|x| if let Ans::Text(t) = x { Some(t.clone()) } else { None }).unwrap_or_default();
        req(ctx, AppRequest::GumballOp { op: op.into(), points: pts, value: text })
    })
}

pub fn gumball_settings() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Gumball", vec![opt("On"), opt("Off"), opt("AutoReset"), opt("Reset"), opt("Alignment")])], |a, ctx| match a[0].text().as_str() {
        "On" | "Off" => req(ctx, AppRequest::SetOption { key: "gumball".into(), value: a[0].text() }),
        "AutoReset" => req(ctx, AppRequest::GumballOp { op: "auto_reset".into(), points: vec![], value: "Toggle".into() }),
        "Reset" => req(ctx, AppRequest::GumballOp { op: "reset".into(), points: vec![], value: String::new() }),
        _ => Step::Prompt(Prompt::options("Gumball alignment").opts([opt("CPlane"), opt("World"), opt("Object"), opt("View")])),
    })
    .then(|inp, ctx| match inp {
        Input::Option(o) => req(ctx, AppRequest::GumballOp { op: "alignment".into(), points: vec![], value: o }),
        _ => Step::Cancelled,
    })
}

// ---------------------------------------------------------------- CPlanes

pub struct CPlaneOpCmd(pub &'static str);

impl Command for CPlaneOpCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        req(ctx, AppRequest::CPlaneOp(self.0.into()))
    }
    fn input(&mut self, _input: Input, _ctx: &mut CmdCtx) -> Step {
        Step::Done
    }
}

pub fn mplane() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Mobile construction plane", vec![opt("Attach"), opt("Detach"), opt("Refresh")])], |a, ctx| {
        if a[0].text() != "Attach" {
            return req(ctx, AppRequest::CPlaneOp(format!("mplane_{}", a[0].text().to_ascii_lowercase())));
        }
        Step::Prompt(Prompt::objects("Select object to attach the construction plane to", ObjFilter::Any, 1))
    })
    .then(|inp, ctx| match inp {
        Input::Objects(ids) if !ids.is_empty() => req(ctx, AppRequest::CPlaneOp(format!("mplane_attach:{}", ids[0]))),
        _ => Step::Cancelled,
    })
}

// ---------------------------------------------------------------- display

pub fn set_object_display_mode() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Options("Display mode", vec![opt("UseView"), opt("Wireframe"), opt("Shaded"), opt("Rendered"), opt("Ghosted"), opt("XRay"), opt("Technical"), opt("Artistic"), opt("Pen"), opt("Arctic")])], |a, ctx| {
        let mode = a[1].text();
        ctx.doc.begin_undo("SetObjectDisplayMode");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.user_text.retain(|(k, _)| k != "DisplayMode");
                if mode != "UseView" && !mode.is_empty() {
                    o.attributes.user_text.push(("DisplayMode".into(), mode.clone()));
                }
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn clear_object_display_modes() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        ctx.doc.begin_undo("ClearAllObjectDisplayModes");
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| has_user_text(o, "DisplayMode")).map(|o| o.id).collect();
        for id in &ids {
            if let Some(o) = ctx.doc.get_mut(*id) {
                o.attributes.user_text.retain(|(k, _)| k != "DisplayMode");
            }
        }
        ctx.doc.touch_revision();
        ctx.log(format!("{} objects now use the viewport display mode.", ids.len()));
        Step::Done
    })
}

pub fn render_mesh_visibility(mode: &'static str) -> SeqCmd {
    SeqCmd::new(vec![], move |_, ctx| {
        let cur = ctx.memory.get("RenderMesh.Hidden").map_or(false, |v| v == "Yes");
        let hide = match mode { "hide" => true, "show" => false, _ => !cur };
        ctx.mem_set("RenderMesh.Hidden", if hide { "Yes" } else { "No" });
        req(ctx, AppRequest::SetOption { key: "render_mesh_hidden".into(), value: if hide { "On".into() } else { "Off".into() } })
    })
}

// ---------------------------------------------------------------- layer states / named selections

pub fn layer_state_manager() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Layer state", vec![opt("Save"), opt("Restore"), opt("Delete"), opt("List")])], |a, ctx| {
        let op = a[0].text();
        if op == "List" {
            return req(ctx, AppRequest::LayerState { op: "list".into(), name: String::new() });
        }
        ctx.mem_set("LayerState.Op", op.to_ascii_lowercase());
        Step::Prompt(Prompt::text("Layer state name"))
    })
    .then(|inp, ctx| match inp {
        Input::Text(name) if !name.trim().is_empty() => {
            let op = ctx.memory.get("LayerState.Op").cloned().unwrap_or_else(|| "save".into());
            req(ctx, AppRequest::LayerState { op, name: name.trim().to_string() })
        }
        _ => Step::Cancelled,
    })
}

pub fn layer_book() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("LayerBook", vec![opt("Next"), opt("Previous"), opt("First"), opt("All")])], |a, ctx| req(ctx, AppRequest::LayerState { op: format!("book_{}", a[0].text().to_ascii_lowercase()), name: String::new() }))
}

pub fn highlight_object_layers() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1)], |a, ctx| {
        let mut layers: Vec<usize> = a[0].ids().iter().filter_map(|id| ctx.doc.get(*id)).map(|o| o.attributes.layer).collect();
        layers.sort();
        layers.dedup();
        let names: Vec<String> = layers.iter().filter_map(|i| ctx.doc.layers().get(*i)).map(|l| l.name.clone()).collect();
        if let Some(first) = layers.first() {
            ctx.doc.set_current_layer(*first);
        }
        ctx.log(format!("Layers of the selection: {}", names.join(", ")));
        ctx.mem_set("Layers.Highlight", names.join(";"));
        Step::Done
    })
}

pub fn named_selections() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Named selection", vec![opt("Save"), opt("Restore"), opt("Delete"), opt("List")])], |a, ctx| {
        let op = a[0].text();
        if op == "List" {
            return req(ctx, AppRequest::NamedSelection { op: "list".into(), name: String::new() });
        }
        ctx.mem_set("NamedSelection.Op", op.to_ascii_lowercase());
        Step::Prompt(Prompt::text("Selection name"))
    })
    .then(|inp, ctx| match inp {
        Input::Text(name) if !name.trim().is_empty() => {
            let op = ctx.memory.get("NamedSelection.Op").cloned().unwrap_or_else(|| "save".into());
            req(ctx, AppRequest::NamedSelection { op, name: name.trim().to_string() })
        }
        _ => Step::Cancelled,
    })
}

// ---------------------------------------------------------------- blocks

fn instance_of(doc: &Document, id: ObjectId) -> Option<(usize, DAffine3)> {
    match doc.get(id)?.geometry {
        Geometry::Instance { block, xf } => Some((block, xf)),
        _ => None,
    }
}

pub fn block_edit() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        // finishing an edit session: objects tagged BlockEdit=<name> redefine the block
        let editing: Vec<ObjectId> = ctx.doc.objects().filter(|o| has_user_text(o, "BlockEdit")).map(|o| o.id).collect();
        if !editing.is_empty() {
            let name = ctx.doc.get(editing[0]).and_then(|o| o.attributes.user_text.iter().find(|(k, _)| k == "BlockEdit").map(|(_, v)| v.clone())).unwrap_or_default();
            let xf_text = ctx.doc.get(editing[0]).and_then(|o| o.attributes.user_text.iter().find(|(k, _)| k == "BlockEditXf").map(|(_, v)| v.clone())).unwrap_or_default();
            let xf: DAffine3 = serde_json::from_str(&xf_text).unwrap_or(DAffine3::IDENTITY);
            let inv = xf.inverse();
            let Some(bi) = ctx.doc.find_block(&name) else {
                ctx.log(format!("Block \"{name}\" no longer exists."));
                return Step::Cancelled;
            };
            ctx.doc.begin_undo("BlockEdit");
            let mut objects = Vec::new();
            for id in &editing {
                if let Some(o) = ctx.doc.get(*id) {
                    let mut g = o.geometry.clone();
                    g.transform(&inv);
                    let mut attrs = o.attributes.clone();
                    attrs.user_text.retain(|(k, _)| k != "BlockEdit" && k != "BlockEditXf");
                    objects.push((g, attrs));
                }
            }
            for id in &editing {
                ctx.doc.remove(*id);
            }
            let base = ctx.doc.state.blocks[bi].base;
            ctx.doc.state.blocks[bi] = tri_doc::BlockDef { name: name.clone(), base, objects };
            ctx.doc.clear_selection();
            let nid = ctx.doc.add(Geometry::Instance { block: bi, xf });
            ctx.doc.select(nid, true);
            ctx.doc.touch_revision();
            ctx.log(format!("Block \"{name}\" redefined from {} objects.", editing.len()));
            return Step::Done;
        }
        Step::Prompt(Prompt::objects("Select block instance to edit (run BlockEdit again to finish)", ObjFilter::Any, 1))
    })
    .then(|inp, ctx| {
        let Input::Objects(ids) = inp else { return Step::Cancelled };
        let Some((bi, xf)) = ids.iter().find_map(|id| instance_of(ctx.doc, *id).map(|v| (*id, v))).map(|(_, v)| v) else {
            ctx.log("Select a block instance.");
            return Step::Cancelled;
        };
        let iid = ids.iter().find(|id| instance_of(ctx.doc, **id).is_some()).copied().unwrap();
        let def = ctx.doc.state.blocks[bi].clone();
        ctx.doc.begin_undo("BlockEdit");
        ctx.doc.remove(iid);
        ctx.doc.clear_selection();
        for (g, attrs) in def.objects {
            let mut g = g;
            g.transform(&xf);
            let mut attrs = attrs;
            attrs.user_text.push(("BlockEdit".into(), def.name.clone()));
            attrs.user_text.push(("BlockEditXf".into(), serde_json::to_string(&xf).unwrap_or_default()));
            let id = ctx.doc.add_with(g, attrs);
            ctx.doc.select(id, true);
        }
        ctx.log(format!("Editing block \"{}\": modify the objects, then run BlockEdit to finish.", def.name));
        Step::Done
    })
}

pub fn block_reset_scale() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select block instances", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("BlockResetScale");
        let mut n = 0;
        for id in a[0].ids() {
            let Some((block, xf)) = instance_of(ctx.doc, id) else { continue };
            let (_, rot, trans) = xf.to_scale_rotation_translation();
            let new = DAffine3::from_rotation_translation(rot, trans);
            ctx.doc.replace_geometry(id, Geometry::Instance { block, xf: new });
            n += 1;
        }
        ctx.log(format!("{n} block instances reset to unit scale."));
        Step::Done
    })
}

pub fn add_objects_to_block() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to add", ObjFilter::Any, 1), Ask::Pick("Select block instance", ObjFilter::Any)], |a, ctx| {
        let Some((bi, xf)) = instance_of(ctx.doc, a[1].ids()[0]) else {
            ctx.log("Select a block instance.");
            return Step::Cancelled;
        };
        let inv = xf.inverse();
        ctx.doc.begin_undo("AddObjectsToBlock");
        let mut n = 0;
        for id in a[0].ids() {
            if instance_of(ctx.doc, id).map_or(false, |(b, _)| b == bi) {
                continue;
            }
            let Some(o) = ctx.doc.get(id) else { continue };
            let mut g = o.geometry.clone();
            g.transform(&inv);
            let attrs = o.attributes.clone();
            ctx.doc.state.blocks[bi].objects.push((g, attrs));
            ctx.doc.remove(id);
            n += 1;
        }
        ctx.doc.touch_revision();
        ctx.log(format!("{n} objects added to block \"{}\".", ctx.doc.state.blocks[bi].name));
        Step::Done
    })
}

pub fn create_unique_block() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select block instances to make unique", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("CreateUniqueBlock");
        let mut n = 0;
        for id in a[0].ids() {
            let Some((bi, xf)) = instance_of(ctx.doc, id) else { continue };
            let mut def = ctx.doc.state.blocks[bi].clone();
            let mut k = 1;
            while ctx.doc.find_block(&format!("{} {:02}", def.name, k)).is_some() {
                k += 1;
            }
            def.name = format!("{} {:02}", def.name, k);
            let nb = ctx.doc.add_block(def);
            ctx.doc.replace_geometry(id, Geometry::Instance { block: nb, xf });
            n += 1;
        }
        ctx.log(format!("{n} unique block definitions created."));
        Step::Done
    })
}

pub fn replace_block() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select block instances to replace", ObjFilter::Any, 1), Ask::Text("Name of the replacement block definition")], |a, ctx| {
        let name = a[1].text();
        let Some(nb) = ctx.doc.find_block(name.trim()) else {
            ctx.log(format!("No block definition named \"{name}\"."));
            return Step::Cancelled;
        };
        ctx.doc.begin_undo("ReplaceBlock");
        let mut n = 0;
        for id in a[0].ids() {
            if let Some((_, xf)) = instance_of(ctx.doc, id) {
                ctx.doc.replace_geometry(id, Geometry::Instance { block: nb, xf });
                n += 1;
            }
        }
        ctx.log(format!("{n} instances now reference \"{name}\"."));
        Step::Done
    })
}

pub fn copy_block_definition() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Block definition to copy"), Ask::Text("New name")], |a, ctx| {
        let Some(bi) = ctx.doc.find_block(a[0].text().trim()) else {
            ctx.log("No such block definition.");
            return Step::Cancelled;
        };
        let mut def = ctx.doc.state.blocks[bi].clone();
        def.name = a[1].text().trim().to_string();
        if def.name.is_empty() || ctx.doc.find_block(&def.name).is_some() {
            ctx.log("Choose a new, unused name.");
            return Step::Cancelled;
        }
        ctx.doc.begin_undo("CopyLinkedBlockDefinition");
        ctx.doc.add_block(def);
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn rescue_block_orphans() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let count = ctx.doc.state.blocks.len();
        let orphans: Vec<ObjectId> = ctx.doc.objects().filter(|o| matches!(o.geometry, Geometry::Instance { block, .. } if block >= count)).map(|o| o.id).collect();
        if orphans.is_empty() {
            ctx.log("No orphan block instances.");
            return Step::Done;
        }
        ctx.doc.begin_undo("RescueBlockOrphans");
        for id in &orphans {
            ctx.doc.remove(*id);
        }
        ctx.log(format!("{} orphan instances removed.", orphans.len()));
        Step::Done
    })
}

pub fn rescue_3dm() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Damaged .3dm file")], |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        match tri_doc::io3dm::load(std::path::Path::new(&p)) {
            Ok((state, _, skipped)) => {
                ctx.doc.begin_undo("Rescue3dmFile");
                let mut n = 0;
                for o in state.objects.values() {
                    ctx.doc.add_with(o.geometry.clone(), tri_doc::Attributes { layer: ctx.doc.current_layer(), ..o.attributes.clone() });
                    n += 1;
                }
                ctx.log(format!("{n} objects recovered ({skipped} unreadable)."));
            }
            Err(e) => ctx.log(format!("Unable to read the file: {e}")),
        }
        Step::Done
    })
}

pub fn sel_mirrored_blocks() -> super::simple::FnCmd {
    super::simple::FnCmd(|ctx| {
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && matches!(o.geometry, Geometry::Instance { xf, .. } if xf.matrix3.determinant() < 0.0)).map(|o| o.id).collect();
        for id in &ids {
            ctx.doc.select(*id, true);
        }
        ctx.log(format!("{} mirrored block instances selected.", ids.len()));
        Step::Done
    })
}

// ---------------------------------------------------------------- worksession

pub fn worksession() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Worksession", vec![opt("Attach"), opt("Detach"), opt("List")])], |a, ctx| {
        match a[0].text().as_str() {
            "Attach" => Step::Prompt(Prompt::text("Reference file to attach")),
            "Detach" => req(ctx, AppRequest::Worksession { op: "detach".into(), path: String::new() }),
            _ => req(ctx, AppRequest::Worksession { op: "list".into(), path: String::new() }),
        }
    })
    .then(|inp, ctx| match inp {
        Input::Text(p) if !p.trim().is_empty() => req(ctx, AppRequest::Worksession { op: "attach".into(), path: p.trim().to_string() }),
        _ => Step::Cancelled,
    })
}

pub fn purge_ref_objects() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| has_user_text(o, "Reference")).map(|o| o.id).collect();
        ctx.doc.begin_undo("PurgeRefObjects");
        for id in &ids {
            ctx.doc.remove(*id);
        }
        ctx.log(format!("{} reference objects purged.", ids.len()));
        Step::Done
    })
}

pub fn limit_reference_model() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("First corner of the region to keep"), Ask::Point("Other corner")], |a, ctx| {
        let bb = tri_geom::BoundingBox::from_points(&[a[0].point(), a[1].point()]);
        let big = bb.diagonal().max(1.0) * 1e3;
        let bb = tri_geom::BoundingBox::new(DVec3::new(bb.min.x, bb.min.y, -big), DVec3::new(bb.max.x, bb.max.y, big));
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| has_user_text(o, "Reference") && !bb.intersects(&ctx.doc.object_bbox(o))).map(|o| o.id).collect();
        ctx.doc.begin_undo("LimitReferenceModel");
        for id in &ids {
            if let Some(o) = ctx.doc.get_mut(*id) {
                o.attributes.hidden = true;
            }
        }
        ctx.doc.touch_revision();
        ctx.log(format!("{} reference objects outside the region hidden.", ids.len()));
        Step::Done
    })
}

// ---------------------------------------------------------------- clipping sections

fn clip_planes_of(doc: &Document) -> Vec<(ObjectId, Plane)> {
    doc.objects().filter(|o| has_user_text(o, "ClippingPlane")).filter_map(|o| {
        let plane = match &o.geometry {
            Geometry::Surface(s) => {
                let (u0, u1) = s.domain_u();
                let (v0, v1) = s.domain_v();
                Plane::from_normal(s.point_at((u0 + u1) * 0.5, (v0 + v1) * 0.5), s.normal_at((u0 + u1) * 0.5, (v0 + v1) * 0.5))
            }
            _ => return None,
        };
        Some((o.id, plane))
    }).collect()
}

pub fn clipping_sections(mode: &'static str) -> SeqCmd {
    SeqCmd::new(vec![], move |_, ctx| {
        let planes = clip_planes_of(ctx.doc);
        if planes.is_empty() {
            ctx.log("No clipping planes in the model (use ClippingPlane).");
            return Step::Done;
        }
        let objs: Vec<(ObjectId, Mesh)> = ctx.doc.objects().filter(|o| ctx.doc.is_visible(o) && !has_user_text(o, "ClippingPlane") && !has_user_text(o, "ClippingSection")).map(|o| o.id).collect::<Vec<_>>().into_iter().filter_map(|id| solid_mesh(ctx.doc, id).map(|m| (id, m))).collect();
        let mut out = Vec::new();
        for (_, plane) in &planes {
            for (_, m) in &objs {
                let tol = m.bounding_box().diagonal().max(1e-6) * 1e-6;
                for poly in m.section(plane, tol) {
                    if let Ok(c) = NurbsCurve::polyline(poly) {
                        out.push(Geometry::Curve(c));
                    }
                }
                if mode == "slices" {
                    let off = Plane { origin: plane.origin + plane.z_axis * ctx.mem_num("ClippingSlice.Thickness", 1.0), ..*plane };
                    for poly in m.section(&off, tol) {
                        if let Ok(c) = NurbsCurve::polyline(poly) {
                            out.push(Geometry::Curve(c));
                        }
                    }
                }
            }
        }
        if out.is_empty() {
            ctx.log("The clipping planes cut nothing.");
            return Step::Done;
        }
        ctx.doc.begin_undo("ClippingSections");
        ctx.doc.clear_selection();
        for g in out {
            let mut attrs = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
            attrs.user_text.push(("ClippingSection".into(), "Yes".into()));
            let id = ctx.doc.add_with(g, attrs);
            ctx.doc.select(id, true);
        }
        Step::Done
    })
}

pub fn clear_clipping_sections() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| has_user_text(o, "ClippingSection")).map(|o| o.id).collect();
        ctx.doc.begin_undo("ClearClippingSections");
        for id in &ids {
            ctx.doc.remove(*id);
        }
        ctx.log(format!("{} section curves removed.", ids.len()));
        Step::Done
    })
}

pub fn clipping_section_views(cplanes: bool) -> SeqCmd {
    SeqCmd::new(vec![], move |_, ctx| {
        let planes = clip_planes_of(ctx.doc);
        if planes.is_empty() {
            ctx.log("No clipping planes in the model.");
            return Step::Done;
        }
        for (k, (id, plane)) in planes.iter().enumerate() {
            let name = format!("Section {} (#{id})", k + 1);
            if cplanes {
                ctx.requests.push(AppRequest::NamedCPlane { name, plane: *plane });
            } else {
                ctx.requests.push(AppRequest::NamedSectionView { name, plane: *plane });
            }
        }
        ctx.log(format!("{} named {} saved.", planes.len(), if cplanes { "construction planes" } else { "views" }));
        Step::Done
    })
}

pub fn view_clipping_sections() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select clipping plane to look through", ObjFilter::Any)], |a, ctx| {
        let planes = clip_planes_of(ctx.doc);
        let Some((_, plane)) = planes.iter().find(|(id, _)| *id == a[0].ids()[0]) else {
            ctx.log("Select a clipping plane.");
            return Step::Cancelled;
        };
        let bb = ctx.doc.bounding_box();
        let d = bb.diagonal().max(10.0);
        req(ctx, AppRequest::LookAt { from: plane.origin + plane.z_axis * d, to: plane.origin })
    })
}

pub fn clipping_plane_properties() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select clipping plane", ObjFilter::Any), Ask::Options("Property", vec![opt("Info"), opt("Enable"), opt("Disable"), opt("Flip"), opt("Depth")])], |a, ctx| {
        let id = a[0].ids()[0];
        match a[1].text().as_str() {
            "Enable" => req(ctx, AppRequest::SetClippingPlanes { ids: vec![id], enabled: true, flip: false }),
            "Disable" => req(ctx, AppRequest::SetClippingPlanes { ids: vec![id], enabled: false, flip: false }),
            "Flip" => req(ctx, AppRequest::SetClippingPlanes { ids: vec![id], enabled: true, flip: true }),
            "Depth" => Step::Prompt(Prompt::number("Section depth (used by ExtractClippingSlices)").default_num(ctx.mem_num("ClippingSlice.Thickness", 1.0))),
            _ => {
                let planes = clip_planes_of(ctx.doc);
                if let Some((_, p)) = planes.iter().find(|(i, _)| *i == id) {
                    ctx.log(format!("Clipping plane #{id}: origin ({}, {}, {}), normal ({}, {}, {})", fmt_num(p.origin.x), fmt_num(p.origin.y), fmt_num(p.origin.z), fmt_num(p.z_axis.x), fmt_num(p.z_axis.y), fmt_num(p.z_axis.z)));
                }
                Step::Done
            }
        }
    })
    .then(|inp, ctx| {
        if let Input::Number(n) = inp {
            ctx.mem_set("ClippingSlice.Thickness", fmt_num(n));
        }
        Step::Done
    })
}

/// 2-D section drawings: section curves plus the visible outline, laid flat on the construction plane.
pub fn clipping_drawings(update: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Lower-left corner of the drawing")], move |a, ctx| {
        let planes = clip_planes_of(ctx.doc);
        if planes.is_empty() {
            ctx.log("No clipping planes in the model (use ClippingPlane).");
            return Step::Done;
        }
        if update {
            let old: Vec<ObjectId> = ctx.doc.objects().filter(|o| has_user_text(o, "ClippingDrawing")).map(|o| o.id).collect();
            ctx.doc.begin_undo("UpdateClippingDrawings");
            for id in old {
                ctx.doc.remove(id);
            }
        } else {
            ctx.doc.begin_undo("ClippingDrawings");
        }
        let cp = ctx.cplane;
        let mut corner = a[0].point();
        let objs: Vec<Mesh> = ctx.doc.objects().filter(|o| ctx.doc.is_visible(o) && !has_user_text(o, "ClippingPlane") && !has_user_text(o, "ClippingDrawing")).map(|o| o.id).collect::<Vec<_>>().into_iter().filter_map(|id| solid_mesh(ctx.doc, id)).collect();
        let mut made = 0;
        for (_, plane) in &planes {
            // local 2-D frame of the plane; map to the cplane at `corner`
            let mut curves: Vec<(NurbsCurve, bool)> = Vec::new();
            for m in &objs {
                let tol = m.bounding_box().diagonal().max(1e-6) * 1e-6;
                for poly in m.section(plane, tol) {
                    if let Ok(c) = NurbsCurve::polyline(poly) {
                        curves.push((c, true));
                    }
                }
                // visible outline behind the plane: silhouette edges on the kept side
                for (p, q) in tri_geom::mesh_ops::silhouette(m, -plane.z_axis) {
                    let (dp, dq) = ((p - plane.origin).dot(plane.z_axis), (q - plane.origin).dot(plane.z_axis));
                    if dp <= 0.0 && dq <= 0.0 {
                        if let Ok(c) = NurbsCurve::line(p, q) {
                            curves.push((c, false));
                        }
                    }
                }
            }
            if curves.is_empty() {
                continue;
            }
            let bb = curves.iter().fold(tri_geom::BoundingBox::EMPTY, |b, (c, _)| b.union(&c.bounding_box()));
            let (mut umin, mut vmin) = (f64::INFINITY, f64::INFINITY);
            let mut flat: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
            for (c, is_section) in &curves {
                let pts = c.tessellate_adaptive(bb.diagonal().max(1e-6) * 1e-4);
                let uv: Vec<(f64, f64)> = pts.iter().map(|p| { let (u, v, _) = plane.closest_parameters(*p); (u, v) }).collect();
                for (u, v) in &uv {
                    umin = umin.min(*u);
                    vmin = vmin.min(*v);
                }
                flat.push((uv, *is_section));
            }
            let mut width: f64 = 0.0;
            for (uv, is_section) in flat {
                let pts: Vec<DVec3> = uv.iter().map(|(u, v)| { width = width.max(*u - umin); cp.point_at(0.0, 0.0) + (corner - cp.origin) + cp.x_axis * (*u - umin) + cp.y_axis * (*v - vmin) }).collect();
                if let Ok(c) = NurbsCurve::polyline(pts) {
                    let mut attrs = tri_doc::Attributes { layer: ctx.doc.current_layer(), ..Default::default() };
                    attrs.user_text.push(("ClippingDrawing".into(), "Yes".into()));
                    if is_section {
                        attrs.color = Some([200, 0, 0, 255]);
                    }
                    ctx.doc.add_with(Geometry::Curve(c), attrs);
                    made += 1;
                }
            }
            corner += cp.x_axis * (width * 1.2 + 1.0);
        }
        ctx.doc.touch_revision();
        ctx.log(format!("{made} drawing curves created (red = section, black = visible outline)."));
        Step::Done
    })
}

pub fn export_clipping_sections() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("File to write (.dxf or .3dm)")], |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| has_user_text(o, "ClippingSection") || has_user_text(o, "ClippingDrawing")).map(|o| o.id).collect();
        if ids.is_empty() {
            ctx.log("Run ClippingSections or ClippingDrawings first.");
            return Step::Done;
        }
        ctx.requests.push(AppRequest::ExportSelected { ids, path: p });
        Step::Done
    })
}

pub fn export_clipping_section_info() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Text file to write")], |a, ctx| {
        let planes = clip_planes_of(ctx.doc);
        let mut text = String::new();
        for (id, p) in &planes {
            text += &format!("plane #{id}: origin {} {} {} normal {} {} {}\n", p.origin.x, p.origin.y, p.origin.z, p.z_axis.x, p.z_axis.y, p.z_axis.z);
        }
        let path = a[0].text();
        if path.is_empty() {
            ctx.log(text.trim_end());
        } else {
            match std::fs::write(&path, text) {
                Ok(()) => ctx.log(format!("Section info written to {path}.")),
                Err(e) => ctx.log(format!("Unable to write: {e}")),
            }
        }
        Step::Done
    })
}

// ---------------------------------------------------------------- anchors

pub fn earth_anchor() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Model point of the anchor"), Ask::Number("Latitude (degrees)", 45.4642), Ask::Number("Longitude (degrees)", 9.19), Ask::Number("North direction angle from the Y axis (degrees)", 0.0)], |a, ctx| {
        let p = a[0].point();
        ctx.doc.begin_undo("EarthAnchorPoint");
        let ut = &mut ctx.doc.settings.user_text;
        ut.retain(|(k, _)| !k.starts_with("EarthAnchor."));
        ut.push(("EarthAnchor.Point".into(), format!("{},{},{}", p.x, p.y, p.z)));
        ut.push(("EarthAnchor.Latitude".into(), fmt_num(a[1].number())));
        ut.push(("EarthAnchor.Longitude".into(), fmt_num(a[2].number())));
        ut.push(("EarthAnchor.North".into(), fmt_num(a[3].number())));
        ctx.doc.touch_revision();
        ctx.log("Earth anchor saved with the model (used by Sun for the site position).");
        Step::Done
    })
}

pub fn model_basepoint() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Model base point (used when this file is inserted as a block)")], |a, ctx| {
        let p = a[0].point();
        ctx.doc.begin_undo("ModelBasepoint");
        ctx.doc.settings.user_text.retain(|(k, _)| k != "ModelBasepoint");
        ctx.doc.settings.user_text.push(("ModelBasepoint".into(), format!("{},{},{}", p.x, p.y, p.z)));
        ctx.doc.touch_revision();
        Step::Done
    })
}

// ---------------------------------------------------------------- text / annotation edits

fn text_of(o: &Object) -> Option<String> {
    if o.attributes.groups.iter().any(|g| g.starts_with("Text") || g.starts_with("Leader") || g.starts_with("Dot")) {
        return o.attributes.name.clone();
    }
    None
}

pub fn edit_text(label: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select text, leader or dot to edit", ObjFilter::Any), Ask::Text("New text")], move |a, ctx| {
        let id = a[0].ids()[0];
        let Some(o) = ctx.doc.get(id) else { return Step::Cancelled };
        let Some(old) = text_of(o) else {
            ctx.log("Select a text, leader or dot made by triceratops.");
            return Step::Cancelled;
        };
        let new = a[1].text();
        if new.is_empty() {
            return Step::Cancelled;
        }
        let group = o.attributes.groups.last().cloned().unwrap_or_default();
        let members: Vec<ObjectId> = ctx.doc.objects().filter(|x| x.attributes.groups.contains(&group)).map(|x| x.id).collect();
        let bb = members.iter().filter_map(|m| ctx.doc.get(*m)).fold(tri_geom::BoundingBox::EMPTY, |b, x| b.union(&x.geometry.bounding_box()));
        let height = ctx.doc.get(id).and_then(|x| x.attributes.user_text.iter().find(|(k, _)| k == "TextHeight").and_then(|(_, v)| v.parse().ok())).unwrap_or(ctx.mem_num("Text.Height", 1.0));
        let origin = DVec3::new(bb.min.x, bb.min.y, bb.min.z);
        let layer = ctx.doc.get(id).map(|x| x.attributes.layer).unwrap_or(0);
        let curves = super::annotate::text_curves(&new, &ctx.cplane, origin, height, 0.0);
        ctx.doc.begin_undo(label);
        for m in &members {
            ctx.doc.remove(*m);
        }
        ctx.doc.clear_selection();
        for c in curves {
            let mut attrs = tri_doc::Attributes { layer, ..Default::default() };
            attrs.groups.push(group.clone());
            attrs.name = Some(new.clone());
            attrs.user_text.push(("TextHeight".into(), fmt_num(height)));
            let nid = ctx.doc.add_with(Geometry::Curve(c), attrs);
            ctx.doc.select(nid, true);
        }
        ctx.log(format!("\"{old}\" changed to \"{new}\"."));
        Step::Done
    })
}

pub fn convert_dots() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select dots to convert to text", ObjFilter::Any, 1), Ask::Number("Text height", 1.0)], |a, ctx| {
        let h = a[1].number();
        ctx.doc.begin_undo("ConvertDots");
        let mut n = 0;
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            if !o.attributes.groups.iter().any(|g| g.starts_with("Dot")) {
                continue;
            }
            let Some(text) = o.attributes.name.clone() else { continue };
            let group = o.attributes.groups.last().cloned().unwrap_or_default();
            let members: Vec<ObjectId> = ctx.doc.objects().filter(|x| x.attributes.groups.contains(&group)).map(|x| x.id).collect();
            let bb = members.iter().filter_map(|m| ctx.doc.get(*m)).fold(tri_geom::BoundingBox::EMPTY, |b, x| b.union(&x.geometry.bounding_box()));
            let layer = o.attributes.layer;
            for m in &members {
                ctx.doc.remove(*m);
            }
            let new_group = group.replace("Dot", "Text");
            for c in super::annotate::text_curves(&text, &ctx.cplane, bb.min, h, 0.0) {
                let mut attrs = tri_doc::Attributes { layer, ..Default::default() };
                attrs.groups.push(new_group.clone());
                attrs.name = Some(text.clone());
                attrs.user_text.push(("TextHeight".into(), fmt_num(h)));
                ctx.doc.add_with(Geometry::Curve(c), attrs);
            }
            n += 1;
        }
        ctx.log(format!("{n} dots converted to text."));
        Step::Done
    })
}

pub fn match_annotation() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select source annotation", ObjFilter::Any), Ask::Objects("Select annotations to change", ObjFilter::Any, 1)], |a, ctx| {
        let src = a[0].ids()[0];
        let Some(h) = ctx.doc.get(src).and_then(|x| x.attributes.user_text.iter().find(|(k, _)| k == "TextHeight").and_then(|(_, v)| v.parse::<f64>().ok())) else {
            ctx.log("The source has no recorded text height.");
            return Step::Cancelled;
        };
        ctx.mem_set("Text.Height", fmt_num(h));
        ctx.mem_set("Dim.TextHeight", fmt_num(h));
        ctx.log(format!("Text height {} applied to new annotations; use ScaleTextHeight on the selection.", fmt_num(h)));
        ctx.doc.begin_undo("MatchAnnotation");
        for id in a[1].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.user_text.retain(|(k, _)| k != "TextHeight");
                o.attributes.user_text.push(("TextHeight".into(), fmt_num(h)));
            }
        }
        Step::Done
    })
}

pub fn dim_recenter_text() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select dimensions", ObjFilter::Any, 1)], |a, ctx| {
        // dimension text is group "Dim..": move text curves (name set) to the dimension line centre
        ctx.doc.begin_undo("DimRecenterText");
        let mut n = 0;
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            let Some(group) = o.attributes.groups.iter().find(|g| g.starts_with("Dim")).cloned() else { continue };
            let members: Vec<ObjectId> = ctx.doc.objects().filter(|x| x.attributes.groups.contains(&group)).map(|x| x.id).collect();
            let (text, lines): (Vec<ObjectId>, Vec<ObjectId>) = members.iter().partition(|m| ctx.doc.get(**m).map_or(false, |x| x.attributes.name.is_some()));
            if text.is_empty() || lines.is_empty() {
                continue;
            }
            let bb_t = text.iter().filter_map(|m| ctx.doc.get(*m)).fold(tri_geom::BoundingBox::EMPTY, |b, x| b.union(&x.geometry.bounding_box()));
            let bb_l = lines.iter().filter_map(|m| ctx.doc.get(*m)).fold(tri_geom::BoundingBox::EMPTY, |b, x| b.union(&x.geometry.bounding_box()));
            let d = bb_l.center() - bb_t.center();
            let d = d - ctx.cplane.y_axis * d.dot(ctx.cplane.y_axis) + ctx.cplane.y_axis * (bb_t.diagonal() * 0.0);
            ctx.doc.transform_objects(&text, &DAffine3::from_translation(d));
            n += 1;
        }
        ctx.log(format!("{n} dimension texts recentred."));
        Step::Done
    })
}

// ---------------------------------------------------------------- selection helpers

pub fn sel_by_text_key(label: &'static str, key: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Value to match (empty for any)")], move |a, ctx| {
        let v = a[0].text();
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && o.attributes.user_text.iter().any(|(k, val)| k == key && (v.is_empty() || val.eq_ignore_ascii_case(&v)))).map(|o| o.id).collect();
        for id in &ids {
            ctx.doc.select(*id, true);
        }
        ctx.log(format!("{label}: {} objects selected.", ids.len()));
        Step::Done
    })
}

pub fn sel_mesh_faces_by(kind: &'static str) -> SeqCmd {
    let prompt = match kind {
        "Area" => "Maximum face area",
        "AspectRatio" => "Minimum aspect ratio",
        "EdgeLength" => "Maximum edge length",
        _ => "Maximum draft angle (degrees)",
    };
    SeqCmd::new(vec![Ask::Number(prompt, 1.0)], move |a, ctx| {
        let v = a[0].number();
        let up = ctx.cplane.z_axis;
        let mut n = 0;
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && matches!(o.geometry, Geometry::Mesh(_))).map(|o| o.id).collect();
        for id in ids {
            let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| &o.geometry) else { continue };
            let hit = (0..m.triangle_count()).any(|t| {
                let (p, q, r) = m.triangle(t);
                let e = [(q - p).length(), (r - q).length(), (p - r).length()];
                match kind {
                    "Area" => (q - p).cross(r - p).length() * 0.5 <= v,
                    "AspectRatio" => e.iter().cloned().fold(0.0, f64::max) / e.iter().cloned().fold(f64::INFINITY, f64::min).max(1e-12) >= v,
                    "EdgeLength" => e.iter().cloned().fold(0.0, f64::max) <= v,
                    _ => (q - p).cross(r - p).normalize_or(up).dot(up).clamp(-1.0, 1.0).acos().to_degrees() <= v,
                }
            });
            if hit {
                ctx.doc.select(id, true);
                n += 1;
            }
        }
        ctx.log(format!("{n} meshes with matching faces selected (sub-object face selection is not supported; use ExtractMeshFacesBy{kind})."));
        Step::Done
    })
}

// ---------------------------------------------------------------- scripting / macro helpers

pub fn macros_cmd() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Macro", vec![opt("Run"), opt("Edit"), opt("List")])], |a, ctx| {
        match a[0].text().as_str() {
            "Run" => Step::Prompt(Prompt::text("Macro (commands separated by spaces, _Enter for Enter, e.g. \"Circle 0,0,0 5 _Enter\")")),
            "Edit" => Step::Prompt(Prompt::text("Macro file to edit")),
            _ => {
                let list: Vec<String> = ctx.memory.iter().filter(|(k, _)| k.starts_with("Macro.")).map(|(k, v)| format!("{} = {}", &k[6..], v)).collect();
                ctx.log(if list.is_empty() { "No macros saved (use Alias or ToolbarEdit).".to_string() } else { list.join("\n") });
                Step::Done
            }
        }
    })
    .then(|inp, ctx| match inp {
        Input::Text(t) if t.ends_with(".txt") || t.ends_with(".macro") => {
            if let Err(e) = super::simple::open_external(&t) {
                ctx.log(format!("Unable to open {t}: {e}"));
            }
            Step::Done
        }
        Input::Text(t) if !t.trim().is_empty() => {
            let script: Vec<String> = t.split_whitespace().map(|w| if w == "_Enter" || w == "Enter" { String::new() } else { w.trim_start_matches('_').to_string() }).collect();
            ctx.requests.push(AppRequest::RunLines(script));
            Step::Done
        }
        _ => Step::Cancelled,
    })
}

pub fn load_script(run: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Script file (.py or .txt command file)")], move |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        if p.ends_with(".py") {
            ctx.mem_set("Script.Loaded", p.clone());
            if run {
                ctx.requests.push(AppRequest::RunPython(p));
            } else {
                ctx.log(format!("Script {p} loaded; RunScript runs it."));
            }
            return Step::Done;
        }
        ctx.requests.push(AppRequest::RunCommandFile(p));
        Step::Done
    })
}

pub fn run_loaded_script() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Script file (empty runs the loaded script)")], |a, ctx| {
        let mut p = a[0].text();
        if p.is_empty() {
            p = ctx.memory.get("Script.Loaded").cloned().unwrap_or_default();
        }
        if p.is_empty() {
            ctx.log("No script loaded (LoadScript).");
            return Step::Done;
        }
        if p.ends_with(".py") {
            ctx.requests.push(AppRequest::RunPython(p));
        } else {
            ctx.requests.push(AppRequest::RunCommandFile(p));
        }
        Step::Done
    })
}

pub struct PauseCmd(pub &'static str);

impl Command for PauseCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        match self.0 {
            "Enter" | "EnterEnd" => Step::Done,
            "MultiPause" => Prompt::point("Pick points (Enter when done)").step(),
            _ => Prompt::point("Pick a point or press Enter").step(),
        }
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (self.0, input) {
            ("MultiPause", Input::Point(p)) => {
                ctx.log(format!("{}, {}, {}", fmt_num(p.x), fmt_num(p.y), fmt_num(p.z)));
                Prompt::point("Pick points (Enter when done)").step()
            }
            (_, Input::Point(p)) => {
                ctx.log(format!("{}, {}, {}", fmt_num(p.x), fmt_num(p.y), fmt_num(p.z)));
                Step::Done
            }
            _ => Step::Done,
        }
    }
}

pub fn decimal_point() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Decimal separator", vec![opt("Point"), opt("Comma")])], |a, ctx| {
        ctx.mem_set("DecimalSeparator", a[0].text());
        ctx.log(format!("Decimal separator: {} (numbers typed with either separator are accepted).", a[0].text()));
        Step::Done
    })
}

pub fn check_new_objects() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Check new objects", vec![opt("On"), opt("Off")])], |a, ctx| {
        ctx.mem_set("CheckNewObjects", a[0].text());
        ctx.log(format!("CheckNewObjects {}: {}", a[0].text(), if a[0].text() == "On" { "new objects are tested with Check after every command." } else { "new objects are not tested." }));
        Step::Done
    })
}

pub fn notifications() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let lines: Vec<String> = ctx.log.iter().rev().take(10).cloned().collect();
        ctx.log(if lines.is_empty() { "No notifications.".to_string() } else { format!("Recent messages:\n{}", lines.join("\n")) });
        Step::Done
    })
}

pub fn alerter() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Alerter", vec![opt("On"), opt("Off")])], |a, ctx| {
        ctx.mem_set("Alerter", a[0].text());
        req(ctx, AppRequest::SetOption { key: "alerter".into(), value: a[0].text() })
    })
}

pub fn export_rui() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("File to write (.rui)")], |a, ctx| {
        let p = a[0].text();
        if p.is_empty() {
            return Step::Cancelled;
        }
        let toolbars = ["Standard", "Main", "Curve Tools", "Surface Tools", "Solid Tools", "Mesh Tools", "SubD Tools", "Transform", "Analyze", "Annotate", "Render Tools", "Drafting"];
        let mut xml = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<TriceratopsUI major_ver=\"3\" minor_ver=\"0\">\n  <tool_bar_groups>\n");
        for t in &toolbars {
            xml += &format!("    <tool_bar_group name=\"{t}\" />\n");
        }
        xml += "  </tool_bar_groups>\n  <macros />\n</TriceratopsUI>\n";
        match std::fs::write(&p, xml) {
            Ok(()) => ctx.log(format!("Toolbar layout exported to {p}.")),
            Err(e) => ctx.log(format!("Unable to write {p}: {e}")),
        }
        Step::Done
    })
}

pub fn acad_schemes() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("DXF/DWG export scheme", vec![opt("Default"), opt("R12LinesAndArcs"), opt("R12NaturalLinesAndArcs"), opt("CAM"), opt("Curves")])], |a, ctx| {
        ctx.mem_set("Dxf.Scheme", a[0].text());
        ctx.log(format!("DXF export scheme: {} (R12 schemes write arcs as polylines).", a[0].text()));
        Step::Done
    })
}

pub fn set_dimension_layer() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Layer for new dimensions (empty = current layer)")], |a, ctx| {
        let name = a[0].text();
        if name.is_empty() {
            ctx.mem_set("Dim.Layer", "");
            ctx.log("New dimensions go to the current layer.");
            return Step::Done;
        }
        if ctx.doc.find_layer(&name).is_none() {
            ctx.doc.begin_undo("SetDimensionLayer");
            ctx.doc.add_layer(name.clone());
        }
        ctx.mem_set("Dim.Layer", name.clone());
        ctx.log(format!("New dimensions go to layer \"{name}\"."));
        Step::Done
    })
}

pub fn hatch_base() -> SeqCmd {
    SeqCmd::new(vec![Ask::Point("Hatch pattern base point")], |a, ctx| {
        let p = a[0].point();
        ctx.mem_set("Hatch.Base", format!("{},{},{}", p.x, p.y, p.z));
        ctx.log("Hatch base point set for new hatches.");
        Step::Done
    })
}

pub fn hatch_scale() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select hatches", ObjFilter::Any, 1), Ask::Number("Scale factor", 2.0)], |a, ctx| {
        let s = a[1].number();
        ctx.doc.begin_undo("HatchScale");
        let mut n = 0;
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            let Some(group) = o.attributes.groups.iter().find(|g| g.starts_with("Hatch")).cloned() else { continue };
            let members: Vec<ObjectId> = ctx.doc.objects().filter(|x| x.attributes.groups.contains(&group)).map(|x| x.id).collect();
            // keep the boundary (closed curves), rescale the pattern lines about the boundary centre
            let bb = members.iter().filter_map(|m| ctx.doc.get(*m)).fold(tri_geom::BoundingBox::EMPTY, |b, x| b.union(&x.geometry.bounding_box()));
            let c = bb.center();
            let lines: Vec<ObjectId> = members.iter().copied().filter(|m| ctx.doc.get(*m).and_then(|x| x.geometry.as_curve()).map_or(false, |cv| !cv.is_closed())).collect();
            ctx.doc.transform_objects(&lines, &(DAffine3::from_translation(c) * DAffine3::from_scale(DVec3::splat(s)) * DAffine3::from_translation(-c)));
            n += 1;
        }
        ctx.log(format!("{n} hatches rescaled (pattern lines are rescaled about the hatch centre)."));
        Step::Done
    })
}

pub fn print_display() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("PrintDisplay", vec![opt("On"), opt("Off"), opt("Toggle")])], |a, ctx| req(ctx, AppRequest::SetOption { key: "print_display".into(), value: a[0].text() }))
}

pub fn thickness_analysis(off: bool) -> SeqCmd {
    SeqCmd::new(vec![], move |_, ctx| req(ctx, AppRequest::SetDisplayMode(if off { "Shaded".into() } else { "Thickness".into() })))
}

pub fn selection_filter_sub(kind: &'static str) -> SeqCmd {
    SeqCmd::new(vec![], move |_, ctx| {
        ctx.mem_set("SelectionFilter.Sub", kind);
        ctx.log(format!("Sub-object filter {kind}: triceratops selects whole objects; edge/face/vertex picks use the Extract commands."));
        Step::Done
    })
}

pub fn selection_filter_toggle() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let on = ctx.memory.get("SelectionFilter.Enabled").map_or(true, |v| v == "Yes");
        ctx.mem_set("SelectionFilter.Enabled", if on { "No" } else { "Yes" });
        req(ctx, AppRequest::SelectionFilter(if on { Some("All".into()) } else { None }))
    })
}

pub fn ortho_snap_cplane_z() -> super::simple::FlagCmd {
    super::simple::FlagCmd("Ortho.SnapCPlaneZ", "Yes", "Ortho now also snaps to the construction plane Z axis.")
}

pub fn sel_extrusion() -> super::simple::FnCmd {
    super::simple::FnCmd(|ctx| {
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && (has_user_text(o, "Extrusion") || matches!(&o.geometry, Geometry::Brep(b) if b.faces.len() >= 2 && b.faces.iter().all(|f| f.surface.degree_u == 1 || f.surface.degree_v == 1)))).map(|o| o.id).collect();
        for id in &ids {
            ctx.doc.select(*id, true);
        }
        ctx.log(format!("{} extrusion-like polysurfaces selected.", ids.len()));
        Step::Done
    })
}

pub fn sel_naked_mesh_edge_pt() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(Geometry::Mesh(m)) = ctx.doc.get(id).map(|o| o.geometry.clone()) else { continue };
            for l in tri_geom::mesh_ops::naked_loops(&m) {
                for p in l {
                    out.push(Geometry::Point(p));
                }
            }
        }
        if out.is_empty() {
            ctx.log("No naked mesh edges.");
            return Step::Done;
        }
        ctx.log(format!("{} naked edge points added and selected.", out.len()));
        add_all(ctx, "SelNakedMeshEdgePt", out)
    })
}

pub fn sel_edges_of_mesh(kind: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh or SubD near an edge", ObjFilter::Mesh)], move |a, ctx| {
        let Ans::Pick(id, p) = &a[0] else { return Step::Cancelled };
        let Some(Geometry::Mesh(m)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        // nearest edge to the pick, then grow a loop (continue straight) or a ring (parallel edges)
        let edges = m.edges();
        let dist = |(a, b): (u32, u32)| {
            let (pa, pb) = (m.vertices[a as usize], m.vertices[b as usize]);
            let d = pb - pa;
            let t = ((*p - pa).dot(d) / d.length_squared().max(1e-12)).clamp(0.0, 1.0);
            (pa + d * t - *p).length()
        };
        let Some(start) = edges.iter().copied().min_by(|x, y| dist(*x).partial_cmp(&dist(*y)).unwrap()) else { return Step::Cancelled };
        let mut chain = vec![start];
        let mut cur = start;
        for _ in 0..200 {
            let dir = (m.vertices[cur.1 as usize] - m.vertices[cur.0 as usize]).normalize_or(DVec3::X);
            let next = edges.iter().copied().filter(|e| (e.0 == cur.1 || e.1 == cur.1) && *e != cur && !chain.contains(e) && !chain.contains(&(e.1, e.0))).map(|e| {
                let e = if e.0 == cur.1 { e } else { (e.1, e.0) };
                let d = (m.vertices[e.1 as usize] - m.vertices[e.0 as usize]).normalize_or(DVec3::X);
                (e, d.dot(dir))
            }).max_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
            match next {
                Some((e, c)) if c > 0.5 => {
                    chain.push(e);
                    cur = e;
                }
                _ => break,
            }
        }
        let out: Vec<Geometry> = chain.iter().filter_map(|(a, b)| NurbsCurve::line(m.vertices[*a as usize], m.vertices[*b as usize]).ok().map(Geometry::Curve)).collect();
        ctx.log(format!("{kind}: {} edges extracted as curves (sub-object selection is not supported).", out.len()));
        add_all(ctx, kind, out)
    })
}

pub fn sel_faces_to_boundary() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select mesh face", ObjFilter::Mesh), Ask::Pick("Select boundary curve", ObjFilter::Curve)], |a, ctx| {
        let Ans::Pick(id, _) = &a[0] else { return Step::Cancelled };
        let Some(Geometry::Mesh(m)) = ctx.doc.get(*id).map(|o| o.geometry.clone()) else { return Step::Cancelled };
        let Some(c) = ctx.doc.get(a[1].ids()[0]).and_then(|o| o.geometry.as_curve().cloned()) else { return Step::Cancelled };
        let Some(plane) = tri_geom::curve_plane(&c) else { return Step::Cancelled };
        let poly = c.tessellate_adaptive(c.bounding_box().diagonal() * 1e-4);
        let inside = |p: DVec3| {
            let (u, v, _) = plane.closest_parameters(p);
            let mut wn = 0i32;
            for i in 0..poly.len().saturating_sub(1) {
                let (a0, b0) = plane.closest_parameters(poly[i]).into_uv();
                let (a1, b1) = plane.closest_parameters(poly[i + 1]).into_uv();
                if (b0 <= v) != (b1 <= v) {
                    let x = a0 + (v - b0) / (b1 - b0) * (a1 - a0);
                    if x > u {
                        wn += if b1 > b0 { 1 } else { -1 };
                    }
                }
            }
            wn != 0
        };
        let mut take = Mesh::new();
        take.vertices = m.vertices.clone();
        let mut keep = Mesh::new();
        keep.vertices = m.vertices.clone();
        for t in 0..m.triangle_count() {
            let (p, q, r) = m.triangle(t);
            let dst = if inside((p + q + r) / 3.0) { &mut take } else { &mut keep };
            dst.indices.extend_from_slice(&m.indices[t * 3..t * 3 + 3]);
        }
        if take.indices.is_empty() {
            ctx.log("No faces inside the boundary.");
            return Step::Done;
        }
        take.compute_normals();
        keep.compute_normals();
        ctx.doc.begin_undo("SelFacesToBoundary");
        if keep.indices.is_empty() {
            ctx.doc.remove(*id);
        } else {
            ctx.doc.replace_geometry(*id, Geometry::Mesh(super::mesh_cmds::weld(&keep, 1e-9)));
        }
        ctx.log("Faces inside the boundary extracted to a new mesh and selected.");
        add_all(ctx, "SelFacesToBoundary", vec![Geometry::Mesh(super::mesh_cmds::weld(&take, 1e-9))])
    })
}

trait IntoUv {
    fn into_uv(self) -> (f64, f64);
}

impl IntoUv for (f64, f64, f64) {
    fn into_uv(self) -> (f64, f64) {
        (self.0, self.1)
    }
}

pub fn sel_dim_override(text_only: bool) -> super::simple::FnCmd {
    if text_only {
        super::simple::FnCmd(|ctx| {
            let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && has_user_text(o, "DimTextOverride")).map(|o| o.id).collect();
            for id in &ids {
                ctx.doc.select(*id, true);
            }
            ctx.log(format!("{} dimensions with text overrides selected.", ids.len()));
            Step::Done
        })
    } else {
        super::simple::FnCmd(|ctx| {
            let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && o.attributes.groups.iter().any(|g| g.starts_with("Dim")) && has_user_text(o, "TextHeight")).map(|o| o.id).collect();
            for id in &ids {
                ctx.doc.select(*id, true);
            }
            ctx.log(format!("{} dimensions with style overrides selected.", ids.len()));
            Step::Done
        })
    }
}

pub fn sel_picture() -> super::simple::FnCmd {
    super::simple::FnCmd(|ctx| {
        let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| ctx.doc.is_selectable(o) && (has_user_text(o, "Picture") || o.attributes.material.as_deref().map_or(false, |m| m.starts_with("Picture")))).map(|o| o.id).collect();
        for id in &ids {
            ctx.doc.select(*id, true);
        }
        ctx.log(format!("{} pictures selected.", ids.len()));
        Step::Done
    })
}

pub fn automatic_subd_from_mesh() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Automatically convert imported meshes to SubD", vec![opt("On"), opt("Off")])], |a, ctx| {
        ctx.mem_set("Import.MeshToSubD", a[0].text());
        ctx.log(format!("AutomaticSubDFromMesh {}.", a[0].text()));
        Step::Done
    })
}

pub fn reset_message_boxes() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let keys: Vec<String> = ctx.memory.keys().filter(|k| k.starts_with("DontAsk.")).cloned().collect();
        for k in keys {
            ctx.memory.remove(&k);
        }
        ctx.log("All \"don't show again\" message boxes will be shown again.");
        Step::Done
    })
}

pub fn get_issue_state() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let bad = ctx.doc.objects().filter(|o| o.geometry.bounding_box().is_empty() && !matches!(o.geometry, Geometry::Instance { .. })).count();
        ctx.log(format!("Model: {} objects, {} with empty geometry, {} layers, {} block definitions.", ctx.doc.len(), bad, ctx.doc.layers().len(), ctx.doc.state.blocks.len()));
        Step::Done
    })
}

pub fn set_mesh_surface_parameters() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], |a, ctx| {
        // store planar UV parameters (bounding box normalised) as user text so texture mapping can use them
        ctx.doc.begin_undo("SetMeshSurfaceParameters");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.user_text.retain(|(k, _)| k != "MeshUV");
                o.attributes.user_text.push(("MeshUV".into(), "BoundingBox".into()));
                o.attributes.mapping = Some((tri_doc::MappingKind::Planar, 1.0));
            }
        }
        ctx.doc.touch_revision();
        ctx.log("Mesh texture coordinates set from the bounding box (planar mapping).");
        Step::Done
    })
}

pub fn compute_vertex_colors(label: &'static str) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select meshes", ObjFilter::Mesh, 1)], move |a, ctx| {
        ctx.doc.begin_undo(label);
        let mut n = 0;
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            let col = ctx.doc.display_color(o);
            let mat = { let m = ctx.doc.material_of(o).color; if o.attributes.material.is_some() || ctx.doc.layers().get(o.attributes.layer).map_or(false, |l| l.material.is_some()) { m } else { [col[0], col[1], col[2], 255] } };
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.color = Some(mat);
                o.attributes.user_text.retain(|(k, _)| k != "VertexColors");
                o.attributes.user_text.push(("VertexColors".into(), "Baked".into()));
                n += 1;
            }
        }
        ctx.doc.touch_revision();
        ctx.log(format!("{n} meshes: material colour baked into the object colour (per-vertex colours are uniform in triceratops)."));
        Step::Done
    })
}

pub fn extract_custom_mapping() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects with a mapping", ObjFilter::Any, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            if o.attributes.mapping.is_none() {
                continue;
            }
            let bb = ctx.doc.object_bbox(o);
            if bb.is_empty() {
                continue;
            }
            let (mn, mx) = (bb.min, bb.max);
            let corners = [DVec3::new(mn.x, mn.y, mn.z), DVec3::new(mx.x, mn.y, mn.z), DVec3::new(mx.x, mx.y, mn.z), DVec3::new(mn.x, mx.y, mn.z), DVec3::new(mn.x, mn.y, mx.z), DVec3::new(mx.x, mn.y, mx.z), DVec3::new(mx.x, mx.y, mx.z), DVec3::new(mn.x, mx.y, mx.z)];
            for (i, j) in [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)] {
                if let Ok(l) = NurbsCurve::line(corners[i], corners[j]) {
                    out.push(Geometry::Curve(l));
                }
            }
        }
        if out.is_empty() {
            ctx.log("No mapping widgets on the selection.");
            return Step::Done;
        }
        add_all(ctx, "ExtractCustomMappingObject", out)
    })
}

pub fn download_library_textures() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let dir = dirs_fallback();
        let _ = std::fs::create_dir_all(&dir);
        ctx.log(format!("Texture library folder: {} (put image files there; Libraries lists them).", dir.display()));
        Step::Done
    })
}

fn dirs_fallback() -> std::path::PathBuf {
    std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_default().join(".local/share/triceratops/textures")
}

pub fn mat3_from_plane(p: &Plane) -> DMat3 {
    DMat3::from_cols(p.x_axis, p.y_axis, p.z_axis)
}


// ---------------------------------------------------------------- themes

/// Theme command: switch, list, save, export, import or reload UI themes; set the graphics backend.
pub fn theme_cmd() -> SeqCmd {
    SeqCmd::new(vec![Ask::Options("Theme", vec![opt("Set"), opt("List"), opt("Save"), opt("Export"), opt("Import"), opt("Reload"), opt("Backend")])], |a, ctx| {
        let op = a[0].text();
        ctx.mem_set("Theme.Op", op.clone());
        match op.as_str() {
            "List" | "" => req(ctx, AppRequest::ThemeOp("list".into())),
            "Reload" => req(ctx, AppRequest::ThemeOp("reload".into())),
            "Set" => Step::Prompt(Prompt::options("Theme name").opts(crate::theme::Theme::all().into_iter().map(|t| opt(&t.name)).collect::<Vec<_>>())),
            "Save" => Step::Prompt(Prompt::text("Name of the user theme (empty keeps the current name)")),
            "Export" => Step::Prompt(Prompt::text("Theme file to write (.json)")),
            "Import" => Step::Prompt(Prompt::text("Theme file to read (.json)")),
            _ => Step::Prompt(Prompt::options("Graphics backend").opts([opt("Auto"), opt("Gl"), opt("Vulkan")])),
        }
    })
    .then(|inp, ctx| {
        let op = ctx.memory.get("Theme.Op").cloned().unwrap_or_default();
        let value = match inp {
            Input::Text(t) => t,
            Input::Option(o) => o,
            Input::Empty => String::new(),
            _ => return Step::Cancelled,
        };
        match op.as_str() {
            "Set" => req(ctx, AppRequest::ThemeOp(format!("set:{value}"))),
            "Save" => req(ctx, AppRequest::ThemeOp(format!("save:{value}"))),
            "Export" if !value.is_empty() => req(ctx, AppRequest::ThemeOp(format!("export:{value}"))),
            "Import" if !value.is_empty() => req(ctx, AppRequest::ThemeOp(format!("import:{value}"))),
            "Backend" => req(ctx, AppRequest::ThemeOp(format!("backend:{}", value.to_ascii_lowercase()))),
            _ => Step::Cancelled,
        }
    })
}
