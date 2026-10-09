//! Linetypes, materials, texture mapping, lights, pictures and view backgrounds.

use glam::DVec3;
use tri_doc::{Geometry, MappingKind, Material, ObjectId, BUILTIN_LINETYPES};
use tri_geom::{NurbsCurve, NurbsSurface, Plane};

use super::simple::{add_all, select_where, Ans, Ask, SeqCmd};
use super::{fmt_num, opt, optv, AppRequest, CmdCtx, Command, Input, ObjFilter, Prompt, Step};

// ---- linetypes --------------------------------------------------------------------------------

pub struct LinetypesCmd;

impl Command for LinetypesCmd {
    fn start(&mut self, ctx: &mut CmdCtx) -> Step {
        for (n, p) in BUILTIN_LINETYPES {
            ctx.log(format!("  {n}: {}", if p.is_empty() { "continuous".into() } else { p.iter().map(|x| fmt_num(*x)).collect::<Vec<_>>().join(",") }));
        }
        let custom: Vec<String> = ctx.doc.settings.linetypes.iter().map(|(n, p)| format!("  {n}: {}", p.iter().map(|x| fmt_num(*x)).collect::<Vec<_>>().join(","))).collect();
        for l in custom {
            ctx.log(l);
        }
        Prompt::options("Linetypes").opts([opt("Add"), opt("Delete"), optv("Scale", fmt_num(ctx.doc.settings.linetype_scale))]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "Add" => {
                ctx.mem_set("Linetypes.Mode", "add");
                Prompt::text("New linetype name").step()
            }
            Input::Option(o) if o == "Delete" => {
                ctx.mem_set("Linetypes.Mode", "delete");
                Prompt::text("Linetype name to delete").step()
            }
            Input::Option(_) => Prompt::number("Global linetype scale").default_num(ctx.doc.settings.linetype_scale).step(),
            Input::Number(s) => {
                ctx.doc.settings.linetype_scale = s.abs().max(1e-6);
                ctx.doc.touch_revision();
                Step::Done
            }
            Input::Text(t) => {
                let mode = ctx.memory.get("Linetypes.Mode").cloned().unwrap_or_default();
                let name = ctx.memory.get("Linetypes.Name").cloned();
                match (mode.as_str(), name) {
                    ("add", None) => {
                        if t.trim().is_empty() {
                            return Step::Cancelled;
                        }
                        ctx.mem_set("Linetypes.Name", t.trim());
                        Prompt::text("Pattern (dash,-gap,... in model units, e.g. 4,-2)").step()
                    }
                    ("add", Some(name)) => {
                        let pat: Vec<f64> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                        ctx.memory.remove("Linetypes.Name");
                        if pat.is_empty() {
                            ctx.log("Empty pattern.");
                            return Step::Cancelled;
                        }
                        ctx.doc.begin_undo("Linetypes");
                        ctx.doc.settings.linetypes.retain(|(n, _)| !n.eq_ignore_ascii_case(&name));
                        ctx.doc.settings.linetypes.push((name.clone(), pat));
                        ctx.log(format!("Linetype {name} added."));
                        Step::Done
                    }
                    _ => {
                        ctx.doc.begin_undo("Linetypes");
                        let before = ctx.doc.settings.linetypes.len();
                        ctx.doc.settings.linetypes.retain(|(n, _)| !n.eq_ignore_ascii_case(t.trim()));
                        ctx.log(if ctx.doc.settings.linetypes.len() < before { "Linetype deleted.".to_string() } else { "No custom linetype with that name (built-in linetypes cannot be deleted).".to_string() });
                        Step::Done
                    }
                }
            }
            _ => Step::Cancelled,
        }
    }
}

pub fn set_linetype() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Text("Linetype name (Continuous, Dashed, Dots, DashDot, Center, Hidden, Border, ByLayer or custom)")], |a, ctx| {
        let name = a[1].text();
        let lt = if name.is_empty() || name.eq_ignore_ascii_case("bylayer") { None } else { Some(name.clone()) };
        if let Some(n) = &lt {
            if ctx.doc.linetype_pattern(n).is_none() {
                ctx.log(format!("Unknown linetype {n}."));
                return Step::Cancelled;
            }
        }
        ctx.doc.begin_undo("SetLinetype");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.linetype = lt.clone();
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn set_custom_linetype() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select curves", ObjFilter::Curve, 1), Ask::Text("Pattern (dash,-gap,...)")], |a, ctx| {
        let pat: Vec<f64> = a[1].text().split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if pat.is_empty() {
            return Step::Cancelled;
        }
        let name = format!("Custom:{}", pat.iter().map(|x| fmt_num(*x)).collect::<Vec<_>>().join(","));
        ctx.doc.begin_undo("SetCustomLinetype");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.linetype = Some(name.clone());
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn set_layer_linetype() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Layer name"), Ask::Text("Linetype name")], |a, ctx| {
        let Some(idx) = ctx.doc.find_layer(&a[0].text()) else {
            ctx.log("No such layer.");
            return Step::Cancelled;
        };
        let lt = a[1].text();
        if ctx.doc.linetype_pattern(&lt).is_none() {
            ctx.log(format!("Unknown linetype {lt}."));
            return Step::Cancelled;
        }
        ctx.doc.begin_undo("SetLayerLinetype");
        if let Some(l) = ctx.doc.layer_mut(idx) {
            l.linetype = lt;
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn set_linetype_scale() -> SeqCmd {
    SeqCmd::new(vec![Ask::Number("Linetype scale", 1.0)], |a, ctx| {
        ctx.doc.settings.linetype_scale = a[0].number().abs().max(1e-6);
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn sel_linetype() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Linetype name")], |a, ctx| {
        let n = a[0].text();
        select_where(ctx, |d, o| d.linetype_of(o).eq_ignore_ascii_case(&n))
    })
}

pub fn extract_linetype_segments() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select curves", ObjFilter::Curve, 1)], |a, ctx| {
        let scale = ctx.doc.settings.linetype_scale;
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            let Some(c) = o.geometry.as_curve() else { continue };
            let Some(pat) = ctx.doc.linetype_pattern(&ctx.doc.linetype_of(o)) else { continue };
            if pat.is_empty() {
                continue;
            }
            let pts = c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3);
            let mut pairs = Vec::new();
            for w in pts.windows(2) {
                pairs.push(w[0]);
                pairs.push(w[1]);
            }
            let dashes = crate::app::dash_segments(&pairs, &pat, scale);
            for seg in dashes.chunks(2) {
                if seg.len() == 2 {
                    if let Ok(l) = NurbsCurve::line(seg[0], seg[1]) {
                        out.push(Geometry::Curve(l));
                    }
                }
            }
        }
        if out.is_empty() {
            ctx.log("The curves use continuous linetypes.");
            return Step::Done;
        }
        add_all(ctx, "ExtractLineTypeSegments", out)
    })
}

// ---- materials ----------------------------------------------------------------------------------

fn material_index(ctx: &mut CmdCtx, name: &str, create: bool) -> Option<usize> {
    if let Some(i) = ctx.doc.settings.materials.iter().position(|m| m.name.eq_ignore_ascii_case(name)) {
        return Some(i);
    }
    if !create {
        return None;
    }
    ctx.doc.settings.materials.push(Material { name: name.to_string(), ..Default::default() });
    Some(ctx.doc.settings.materials.len() - 1)
}

pub fn assign_material(target: &'static str) -> SeqCmd {
    let asks = match target {
        "layer" => vec![Ask::Text("Layer name"), Ask::Text("Material name")],
        _ => vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Text("Material name (empty = by layer)")],
    };
    SeqCmd::new(asks, move |a, ctx| {
        let name = a[1].text();
        ctx.doc.begin_undo("AssignMaterial");
        if !name.is_empty() {
            material_index(ctx, &name, true);
        }
        match target {
            "layer" => {
                let Some(idx) = ctx.doc.find_layer(&a[0].text()) else {
                    ctx.log("No such layer.");
                    return Step::Cancelled;
                };
                if let Some(l) = ctx.doc.layer_mut(idx) {
                    l.material = if name.is_empty() { None } else { Some(name) };
                }
            }
            "layers_of_objects" => {
                let layers: Vec<usize> = a[0].ids().iter().filter_map(|id| ctx.doc.get(*id).map(|o| o.attributes.layer)).collect();
                for idx in layers {
                    if let Some(l) = ctx.doc.layer_mut(idx) {
                        l.material = if name.is_empty() { None } else { Some(name.clone()) };
                    }
                }
            }
            _ => {
                for id in a[0].ids() {
                    if let Some(o) = ctx.doc.get_mut(id) {
                        o.attributes.material = if name.is_empty() { None } else { Some(name.clone()) };
                    }
                }
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn merge_identical_materials() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        let mats = ctx.doc.settings.materials.clone();
        let mut keep: Vec<Material> = Vec::new();
        let mut rename: Vec<(String, String)> = Vec::new();
        for m in mats {
            if let Some(k) = keep.iter().find(|k| k.color == m.color && k.gloss == m.gloss && k.transparency == m.transparency && k.texture == m.texture) {
                rename.push((m.name.clone(), k.name.clone()));
            } else {
                keep.push(m);
            }
        }
        ctx.doc.begin_undo("RenderMergeIdenticalMaterials");
        let n = rename.len();
        ctx.doc.settings.materials = keep;
        let ids: Vec<ObjectId> = ctx.doc.objects().map(|o| o.id).collect();
        for id in ids {
            if let Some(o) = ctx.doc.get_mut(id) {
                if let Some(m) = &o.attributes.material {
                    if let Some((_, to)) = rename.iter().find(|(from, _)| from == m) {
                        o.attributes.material = Some(to.clone());
                    }
                }
            }
        }
        for l in ctx.doc.state.layers.iter_mut() {
            if let Some(m) = &l.material {
                if let Some((_, to)) = rename.iter().find(|(from, _)| from == m) {
                    l.material = Some(to.clone());
                }
            }
        }
        ctx.doc.touch_revision();
        ctx.log(format!("{n} duplicate materials merged."));
        Step::Done
    })
}

pub fn sel_material_name() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Material name")], |a, ctx| {
        let n = a[0].text();
        select_where(ctx, |d, o| d.material_of(o).name.eq_ignore_ascii_case(&n))
    })
}

pub fn synchronize_render_colors() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("SynchronizeRenderColors");
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            let color = ctx.doc.display_color(&o);
            let name = o.attributes.material.clone().unwrap_or_else(|| format!("Material {id}"));
            let i = material_index(ctx, &name, true).unwrap();
            ctx.doc.settings.materials[i].color = color;
            if let Some(om) = ctx.doc.get_mut(id) {
                om.attributes.material = Some(name);
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn content_filter() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Filter text (empty lists all materials)")], |a, ctx| {
        let f = a[0].text().to_lowercase();
        let rows: Vec<String> = ctx.doc.settings.materials.iter().filter(|m| f.is_empty() || m.name.to_lowercase().contains(&f)).map(|m| format!("  {}: colour {:?} gloss {} transparency {} texture {}", m.name, &m.color[..3], fmt_num(m.gloss as f64), fmt_num(m.transparency as f64), m.texture.clone().unwrap_or_else(|| "none".into()))).collect();
        if rows.is_empty() {
            ctx.log("No materials.");
        }
        for r in rows {
            ctx.log(r);
        }
        Step::Done
    })
}

pub fn assign_texture() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Text("Image file (PNG/JPG)")], |a, ctx| {
        let path = a[1].text();
        if path.is_empty() || !std::path::Path::new(&path).exists() {
            ctx.log("Image file not found.");
            return Step::Cancelled;
        }
        ctx.doc.begin_undo("AssignBlankTexture");
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            let name = o.attributes.material.clone().unwrap_or_else(|| format!("Material {id}"));
            let i = material_index(ctx, &name, true).unwrap();
            ctx.doc.settings.materials[i].texture = Some(path.clone());
            if let Some(om) = ctx.doc.get_mut(id) {
                om.attributes.material = Some(name);
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn apply_mapping(kind: MappingKind) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1), Ask::Number("Texture size (model units per repeat)", 10.0)], move |a, ctx| {
        ctx.doc.begin_undo("ApplyMapping");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.mapping = Some((kind, a[1].number().abs().max(1e-6)));
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn remove_mapping() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1)], |a, ctx| {
        ctx.doc.begin_undo("RemoveMappingChannel");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.mapping = None;
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn match_mapping() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects to change", ObjFilter::Any, 1), Ask::Pick("Select source object", ObjFilter::Any)], |a, ctx| {
        let Some(src) = ctx.doc.get(a[1].ids()[0]).map(|o| (o.attributes.mapping, o.attributes.material.clone())) else { return Step::Cancelled };
        ctx.doc.begin_undo("MatchMapping");
        for id in a[0].ids() {
            if let Some(o) = ctx.doc.get_mut(id) {
                o.attributes.mapping = src.0;
                o.attributes.material = src.1.clone();
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn pack_textures(pack: bool) -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1)], move |a, ctx| {
        ctx.doc.begin_undo(if pack { "PackTextures" } else { "UnpackTextures" });
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id) else { continue };
            let size = ctx.doc.object_bbox(o).diagonal().max(1e-6);
            if let Some(om) = ctx.doc.get_mut(id) {
                om.attributes.mapping = Some(if pack { (MappingKind::Box, size) } else { (MappingKind::Surface, 1.0) });
            }
        }
        ctx.doc.touch_revision();
        Step::Done
    })
}

pub fn extract_uv_mesh() -> SeqCmd {
    SeqCmd::new(vec![Ask::Objects("Select objects", ObjFilter::Any, 1)], |a, ctx| {
        let mut out = Vec::new();
        for id in a[0].ids() {
            let Some(o) = ctx.doc.get(id).cloned() else { continue };
            let mesh = match &o.geometry {
                Geometry::Mesh(m) => m.clone(),
                Geometry::Surface(s) => s.tessellate(8, 8),
                Geometry::Brep(b) => b.tessellate(8, 8),
                _ => continue,
            };
            let mat = ctx.doc.material_of(&o);
            let (kind, scale) = o.attributes.mapping.unwrap_or((MappingKind::Surface, mat.texture_scale));
            let uvs = crate::app::texture_uvs(&o.geometry, &mesh, ctx.doc.object_bbox(&o), kind, scale);
            let mut flat = mesh.clone();
            let s = scale.max(1e-6);
            for (v, uv) in flat.vertices.iter_mut().zip(uvs.iter()) {
                *v = DVec3::new(uv[0] as f64 * s, uv[1] as f64 * s, 0.0);
            }
            flat.compute_normals();
            out.push(Geometry::Mesh(flat));
        }
        add_all(ctx, "ExtractUVMesh", out)
    })
}

/// Picture: an image on a rectangular plane (Triceratops' Picture / BackgroundBitmap).
pub struct PictureCmd {
    background: bool,
    path: Option<String>,
    first: Option<DVec3>,
    aspect: f64,
}

impl PictureCmd {
    pub fn new(background: bool) -> Self {
        Self { background, path: None, first: None, aspect: 1.0 }
    }
}

impl Command for PictureCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        if self.background {
            return Prompt::options("Background bitmap").opts([opt("Place"), opt("Remove"), opt("Hide"), opt("Show")]).step();
        }
        Prompt::text("Image file").step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match (&self.path, &self.first, input) {
            (_, _, Input::Option(o)) if self.background => match o.as_str() {
                "Place" => Prompt::text("Image file").step(),
                "Remove" => {
                    let ids: Vec<ObjectId> = ctx.doc.objects().filter(|o| o.attributes.name.as_deref() == Some("BackgroundBitmap")).map(|o| o.id).collect();
                    ctx.doc.begin_undo("BackgroundBitmap");
                    for id in ids {
                        ctx.doc.remove(id);
                    }
                    Step::Done
                }
                "Hide" | "Show" => {
                    let hide = o == "Hide";
                    let ids: Vec<ObjectId> = ctx.doc.objects().filter(|ob| ob.attributes.name.as_deref() == Some("BackgroundBitmap")).map(|ob| ob.id).collect();
                    ctx.doc.begin_undo("BackgroundBitmap");
                    for id in ids {
                        if let Some(ob) = ctx.doc.get_mut(id) {
                            ob.attributes.hidden = hide;
                        }
                    }
                    Step::Done
                }
                _ => Step::Done,
            },
            (None, _, Input::Text(p)) => {
                let p = p.trim().to_string();
                let Ok((w, h)) = image::image_dimensions(&p) else {
                    ctx.log(format!("Unable to read image {p}."));
                    return Step::Cancelled;
                };
                self.aspect = w as f64 / h.max(1) as f64;
                self.path = Some(p);
                Prompt::point("First corner of picture").step()
            }
            (Some(_), None, Input::Point(p)) => {
                self.first = Some(p);
                Prompt::point_or_number("Other corner or width").step()
            }
            (Some(path), Some(a), inp @ (Input::Point(_) | Input::Number(_))) => {
                let cp = ctx.cplane;
                let (u0, v0, w) = cp.closest_parameters(*a);
                let (width, height) = match inp {
                    Input::Number(n) => (n.abs(), n.abs() / self.aspect),
                    Input::Point(p) => {
                        let (u1, _, _) = cp.closest_parameters(p);
                        let wd = (u1 - u0).abs().max(1e-6);
                        (wd, wd / self.aspect)
                    }
                    _ => (1.0, 1.0 / self.aspect),
                };
                let plane = Plane { origin: cp.origin + cp.z_axis * w, ..cp };
                let Ok(s) = NurbsSurface::plane(&plane, u0, u0 + width, v0, v0 + height) else { return Step::Cancelled };
                let name = if self.background { "BackgroundBitmap".to_string() } else { format!("Picture {}", std::path::Path::new(path).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()) };
                let mat_name = format!("Picture {}", ctx.doc.settings.materials.len() + 1);
                ctx.doc.begin_undo(if self.background { "BackgroundBitmap" } else { "Picture" });
                ctx.doc.settings.materials.push(Material { name: mat_name.clone(), color: [255, 255, 255, 255], gloss: 0.0, transparency: 0.0, texture: Some(path.clone()), texture_scale: width });
                let attrs = tri_doc::Attributes { layer: ctx.doc.current_layer(), name: Some(name), material: Some(mat_name), mapping: Some((MappingKind::Surface, width)), ..Default::default() };
                ctx.doc.clear_selection();
                let id = ctx.doc.add_with(Geometry::Surface(s), attrs);
                ctx.doc.select(id, true);
                ctx.log("Picture placed; switch the viewport to Rendered mode to see the image.");
                Step::Done
            }
            _ => Step::Cancelled,
        }
    }
    fn preview(&self, cursor: Option<DVec3>, _doc: &tri_doc::Document) -> Vec<super::Preview> {
        match (self.first, cursor) {
            (Some(a), Some(c)) => vec![super::Preview::Polyline(super::create::rect_points(&Plane::world_xy(), a, c))],
            _ => vec![],
        }
    }
    fn base_point(&self) -> Option<DVec3> {
        self.first
    }
}

// ---- lights ---------------------------------------------------------------------------------------

fn light_attrs(ctx: &CmdCtx, kind: &str, dir: DVec3) -> tri_doc::Attributes {
    let mut a = tri_doc::Attributes { layer: ctx.doc.current_layer(), name: Some(format!("{kind} Light")), ..Default::default() };
    a.user_text.push(("Light".into(), kind.into()));
    a.user_text.push(("LightDir".into(), format!("{},{},{}", dir.x, dir.y, dir.z)));
    a.user_text.push(("LightColor".into(), "1,1,1".into()));
    a.user_text.push(("LightIntensity".into(), "1".into()));
    a.user_text.push(("LightOn".into(), "Yes".into()));
    a
}

pub fn light_cmd(kind: &'static str) -> SeqCmd {
    let asks = match kind {
        "Point" => vec![Ask::Point("Location of point light")],
        "Spot" => vec![Ask::Point("Location of spotlight"), Ask::Point("Target of spotlight")],
        "Directional" => vec![Ask::Point("Location of directional light"), Ask::Point("Point the light shines towards")],
        "Linear" => vec![Ask::Point("Start of linear light"), Ask::Point("End of linear light")],
        _ => vec![Ask::Point("First corner of rectangular light"), Ask::Point("Other corner"), Ask::Point("Direction point")],
    };
    SeqCmd::new(asks, move |a, ctx| {
        let (pos, dir) = match kind {
            "Point" => (a[0].point(), -DVec3::Z),
            "Spot" | "Directional" => (a[0].point(), (a[1].point() - a[0].point()).normalize_or(-DVec3::Z)),
            "Linear" => ((a[0].point() + a[1].point()) * 0.5, -ctx.cplane.z_axis),
            _ => ((a[0].point() + a[1].point()) * 0.5, (a[2].point() - (a[0].point() + a[1].point()) * 0.5).normalize_or(-DVec3::Z)),
        };
        let attrs = light_attrs(ctx, kind, dir);
        ctx.doc.begin_undo(format!("{kind}Light"));
        ctx.doc.clear_selection();
        let id = ctx.doc.add_with(Geometry::Point(pos), attrs);
        ctx.doc.select(id, true);
        ctx.log("Light added; lights are used in the Rendered display mode.");
        Step::Done
    })
}

fn set_light_dir(ctx: &mut CmdCtx, id: ObjectId, dir: DVec3) {
    if let Some(o) = ctx.doc.get_mut(id) {
        o.attributes.user_text.retain(|(k, _)| k != "LightDir");
        o.attributes.user_text.push(("LightDir".into(), format!("{},{},{}", dir.x, dir.y, dir.z)));
    }
    ctx.doc.touch_revision();
}

pub fn light_by_looking() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select spotlight", ObjFilter::Point)], |a, ctx| {
        let id = a[0].ids()[0];
        let dir = ctx.view_dir;
        ctx.doc.begin_undo("EditLightByLooking");
        set_light_dir(ctx, id, dir);
        Step::Done
    })
}

pub fn light_by_highlight() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select spotlight", ObjFilter::Point), Ask::Pick("Point on surface where the highlight should be", ObjFilter::Surface)], |a, ctx| {
        let id = a[0].ids()[0];
        let Ans::Pick(sid, p) = &a[1] else { return Step::Cancelled };
        let Some(faces) = super::surface_edit::faces_of(ctx.doc, *sid) else { return Step::Cancelled };
        let fi = super::surface_edit::face_under(&faces, *p);
        let (u, v) = faces[fi].surface.closest_uv(*p);
        let n = faces[fi].surface.normal_at(u, v);
        let view = ctx.view_dir;
        let r = (view - n * (2.0 * view.dot(n))).normalize_or(-view);
        let dist = ctx.doc.get(id).map(|o| (ctx.doc.object_bbox(o).center() - *p).length()).unwrap_or(10.0).max(1.0);
        let pos = *p - r * dist;
        ctx.doc.begin_undo("EditLightByHighlight");
        ctx.doc.replace_geometry(id, Geometry::Point(pos));
        set_light_dir(ctx, id, r);
        Step::Done
    })
}

pub fn view_to_spotlight() -> SeqCmd {
    SeqCmd::new(vec![Ask::Pick("Select spotlight", ObjFilter::Point)], |a, ctx| {
        let id = a[0].ids()[0];
        let Some(o) = ctx.doc.get(id) else { return Step::Cancelled };
        let Geometry::Point(pos) = o.geometry else { return Step::Cancelled };
        let dir = o.attributes.user_text.iter().find(|(k, _)| k == "LightDir").map(|(_, v)| { let n: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect(); if n.len() == 3 { DVec3::new(n[0], n[1], n[2]) } else { -DVec3::Z } }).unwrap_or(-DVec3::Z);
        ctx.requests.push(AppRequest::LookAt { from: pos, to: pos + dir * 10.0 });
        Step::Done
    })
}

pub struct GradientViewCmd;

impl Command for GradientViewCmd {
    fn start(&mut self, _ctx: &mut CmdCtx) -> Step {
        Prompt::options("Gradient background").opts([opt("On"), opt("Off"), optv("TopColor", "r,g,b"), optv("BottomColor", "r,g,b")]).step()
    }
    fn input(&mut self, input: Input, ctx: &mut CmdCtx) -> Step {
        match input {
            Input::Option(o) if o == "On" => {
                let parse = |s: Option<&String>, d: [u8; 3]| s.map(|v| { let n: Vec<u8> = v.split(',').filter_map(|x| x.trim().parse().ok()).collect(); if n.len() == 3 { [n[0], n[1], n[2]] } else { d } }).unwrap_or(d);
                let top = parse(ctx.memory.get("Gradient.Top"), [120, 150, 190]);
                let bottom = parse(ctx.memory.get("Gradient.Bottom"), [40, 40, 45]);
                ctx.requests.push(AppRequest::Gradient(Some((top, bottom))));
                Step::Done
            }
            Input::Option(o) if o == "Off" => {
                ctx.requests.push(AppRequest::Gradient(None));
                Step::Done
            }
            Input::Option(o) => {
                ctx.mem_set("Gradient.Asking", o);
                Prompt::text("Colour as r,g,b").step()
            }
            Input::Text(t) => {
                let key = if ctx.memory.remove("Gradient.Asking").as_deref() == Some("TopColor") { "Gradient.Top" } else { "Gradient.Bottom" };
                ctx.mem_set(key, t.trim());
                self.start(ctx)
            }
            _ => Step::Done,
        }
    }
}

pub fn refresh_textures() -> SeqCmd {
    SeqCmd::new(vec![], |_, ctx| {
        ctx.requests.push(AppRequest::RefreshTextures);
        ctx.doc.touch_revision();
        ctx.log("Textures reloaded.");
        Step::Done
    })
}

pub fn libraries() -> SeqCmd {
    SeqCmd::new(vec![Ask::Text("Folder with image files")], |a, ctx| {
        let dir = a[0].text();
        match std::fs::read_dir(if dir.is_empty() { ".".to_string() } else { dir }) {
            Ok(rd) => {
                let files: Vec<String> = rd.flatten().map(|e| e.path().to_string_lossy().to_string()).filter(|p| { let l = p.to_lowercase(); l.ends_with(".png") || l.ends_with(".jpg") || l.ends_with(".jpeg") }).collect();
                if files.is_empty() {
                    ctx.log("No image files found.");
                }
                for f in files {
                    ctx.log(format!("  {f}"));
                }
                ctx.log("Use AssignBlankTexture or the Materials panel to apply an image.");
            }
            Err(e) => ctx.log(format!("Unable to read folder: {e}")),
        }
        Step::Done
    })
}

pub fn mapping_widget(on: bool) -> SeqCmd {
    SeqCmd::new(vec![], move |_, ctx| {
        ctx.requests.push(AppRequest::SetOption { key: "mapping_widget".into(), value: if on { "On".into() } else { "Off".into() } });
        Step::Done
    })
}

#[allow(dead_code)]
fn unused() -> Option<MappingKind> {
    MappingKind::from_name("planar")
}
