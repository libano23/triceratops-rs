//! Right container: tabbed panels (Properties, Layers, Display, Help, Notes).

use egui::{Color32, RichText};

use crate::app::{App, DisplayMode};
use crate::camera::Projection;
use crate::commands::fmt_num;

const TABS: [(&str, &str); 8] = [("Properties", "properties"), ("Layers", "layer"), ("Display", "viewport_shaded"), ("Materials", "render"), ("Lights", "spotlight"), ("Rendering", "viewport_rendered"), ("Help", "help"), ("Notes", "textobject")];

/// Icon name of a panel tab.
pub fn panel_icon(name: &str) -> &'static str {
    TABS.iter().find(|(n, _)| *n == name).map(|(_, i)| *i).unwrap_or("properties")
}

/// Draw one panel's contents into a container body.
pub fn draw_panel(app: &mut App, ui: &mut egui::Ui, name: &str) {
    ui.horizontal(|ui| {
        ui.heading(name);
    });
    ui.separator();
    match name {
        "Properties" => properties(app, ui),
        "Layers" => layers(app, ui),
        "Display" => display(app, ui),
        "Materials" => materials(app, ui),
        "Lights" => lights(app, ui),
        "Rendering" => rendering(app, ui),
        "Help" => help(app, ui),
        "Notes" => notes(app, ui),
        _ => {}
    }
}

fn properties(app: &mut App, ui: &mut egui::Ui) {
    let ids = app.doc.selected_ids();
    if ids.is_empty() {
        // View page
        let vp = app.active;
        ui.label(RichText::new("Viewport").strong());
        egui::Grid::new("view_props").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Title");
            let mut name = app.viewports[vp].name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                app.viewports[vp].name = name;
            }
            ui.end_row();
            ui.label("Width");
            ui.label(format!("{}", app.viewports[vp].rect.w as i32));
            ui.end_row();
            ui.label("Height");
            ui.label(format!("{}", app.viewports[vp].rect.h as i32));
            ui.end_row();
            ui.label("Projection");
            let cam = &mut app.viewports[vp].camera;
            let mut proj = cam.projection;
            egui::ComboBox::from_id_salt("proj").selected_text(format!("{proj:?}")).show_ui(ui, |ui| {
                ui.selectable_value(&mut proj, Projection::Parallel, "Parallel");
                ui.selectable_value(&mut proj, Projection::Perspective, "Perspective");
            });
            if proj != cam.projection {
                cam.projection = proj;
                if proj == Projection::Perspective {
                    cam.rotatable = true;
                    cam.up_override = None;
                }
            }
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Camera").strong());
        let cam = &mut app.viewports[vp].camera;
        egui::Grid::new("cam_props").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            let eye = cam.eye();
            ui.label("Location");
            ui.label(format!("{}, {}, {}", fmt_num(eye.x), fmt_num(eye.y), fmt_num(eye.z)));
            ui.end_row();
            ui.label("Target");
            let mut t = [cam.target.x, cam.target.y, cam.target.z];
            ui.horizontal(|ui| {
                for v in t.iter_mut() {
                    ui.add(egui::DragValue::new(v).speed(0.1));
                }
            });
            cam.target = glam::DVec3::new(t[0], t[1], t[2]);
            ui.end_row();
            ui.label("Lens length");
            let mut lens = cam.lens_length();
            if ui.add(egui::DragValue::new(&mut lens).speed(1.0).range(5.0..=1000.0).suffix(" mm")).changed() {
                cam.set_lens_length(lens);
            }
            ui.end_row();
            ui.label("Distance");
            ui.label(fmt_num(cam.distance));
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Wallpaper").strong());
        ui.label(RichText::new("(none)").color(Color32::GRAY));
        return;
    }
    // Object page
    ui.label(RichText::new("Object").strong());
    let first = app.doc.get(ids[0]).cloned();
    let Some(first) = first else { return };
    let kinds: Vec<&str> = ids.iter().filter_map(|id| app.doc.get(*id)).map(|o| o.geometry.kind()).collect();
    let same_kind = kinds.iter().all(|k| *k == kinds[0]);
    egui::Grid::new("obj_props").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
        ui.label("Type");
        if ids.len() == 1 {
            ui.label(first.describe());
        } else if same_kind {
            ui.label(format!("{} {}s", ids.len(), kinds[0]));
        } else {
            ui.label(format!("{} objects (varies)", ids.len()));
        }
        ui.end_row();
        ui.label("Name");
        let mut name = first.attributes.name.clone().unwrap_or_default();
        let r = ui.text_edit_singleline(&mut name);
        if r.changed() {
            for id in &ids {
                if let Some(o) = app.doc.get_mut(*id) {
                    o.attributes.name = if name.is_empty() { None } else { Some(name.clone()) };
                }
            }
        }
        ui.end_row();
        ui.label("Layer");
        let layer_names: Vec<String> = app.doc.layers().iter().map(|l| l.name.clone()).collect();
        let mut layer = first.attributes.layer;
        egui::ComboBox::from_id_salt("obj_layer").selected_text(layer_names.get(layer).cloned().unwrap_or_default()).show_ui(ui, |ui| {
            for (i, n) in layer_names.iter().enumerate() {
                ui.selectable_value(&mut layer, i, n);
            }
        });
        if layer != first.attributes.layer {
            app.doc.begin_undo("ChangeLayer");
            for id in &ids {
                if let Some(o) = app.doc.get_mut(*id) {
                    o.attributes.layer = layer;
                }
            }
        }
        ui.end_row();
        ui.label("Display Color");
        let by_layer = first.attributes.color.is_none();
        let mut mode = if by_layer { 0 } else { 1 };
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("obj_color").selected_text(if mode == 0 { "By Layer" } else { "Custom" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut mode, 0, "By Layer");
                ui.selectable_value(&mut mode, 1, "Custom");
            });
            if mode == 1 {
                let c = first.attributes.color.unwrap_or_else(|| app.doc.display_color(&first));
                let mut rgb = [c[0], c[1], c[2]];
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    for id in &ids {
                        if let Some(o) = app.doc.get_mut(*id) {
                            o.attributes.color = Some([rgb[0], rgb[1], rgb[2], 255]);
                        }
                    }
                }
            }
        });
        if (mode == 0) != by_layer {
            let custom = app.doc.display_color(&first);
            for id in &ids {
                if let Some(o) = app.doc.get_mut(*id) {
                    o.attributes.color = if mode == 0 { None } else { Some(custom) };
                }
            }
        }
        ui.end_row();
        ui.label("Linetype");
        ui.label("By Layer");
        ui.end_row();
        ui.label("Print Color");
        ui.label("By Layer");
        ui.end_row();
        ui.label("Print Width");
        ui.label("By Layer");
        ui.end_row();
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Details...").clicked() {
            app.start_command("What");
        }
        if ui.button("Match...").clicked() {
            app.message_box = Some("Match properties is not implemented in triceratops yet.".into());
        }
    });
}

fn layers(app: &mut App, ui: &mut egui::Ui) {
    // toolbar
    ui.horizontal_wrapped(|ui| {
        if ui.button("New Layer").on_hover_text("New Layer (right-click: random colour)").clicked() {
            let n = app.doc.layers().len();
            app.doc.begin_undo("Layer");
            let idx = app.doc.add_layer(format!("Layer {n:02}"));
            app.panels.layer_selected = Some(idx);
            app.panels.layer_editing = Some((idx, format!("Layer {n:02}")));
        }
        if ui.button("New Sublayer").clicked() {
            app.message_box = Some("Sublayers are not implemented in triceratops yet.".into());
        }
        if ui.button("Delete").clicked() {
            if let Some(i) = app.panels.layer_selected {
                let has_objects = app.doc.objects().any(|o| o.attributes.layer == i);
                if i == 0 {
                    app.message_box = Some("The Default layer cannot be deleted.".into());
                } else if has_objects {
                    app.message_box = Some("The layer has objects on it and cannot be deleted.".into());
                } else {
                    app.doc.begin_undo("Delete layer");
                    app.doc.delete_layer(i);
                    app.panels.layer_selected = None;
                }
            }
        }
        if ui.button("Change Object Layer").clicked() {
            if let Some(i) = app.panels.layer_selected {
                let ids = app.doc.selected_ids();
                app.doc.begin_undo("ChangeLayer");
                for id in ids {
                    if let Some(o) = app.doc.get_mut(id) {
                        o.attributes.layer = i;
                    }
                }
            }
        }
        if ui.button("▲").clicked() {
            app.message_box = Some("Move Up is not implemented in triceratops yet.".into());
        }
        if ui.button("▼").clicked() {
            app.message_box = Some("Move Down is not implemented in triceratops yet.".into());
        }
        ui.menu_button("Tools", |ui| {
            if ui.button("Select Objects").clicked() {
                if let Some(i) = app.panels.layer_selected {
                    let ids: Vec<_> = app.doc.objects().filter(|o| o.attributes.layer == i).map(|o| o.id).collect();
                    for id in ids {
                        app.doc.select(id, true);
                    }
                }
                ui.close();
            }
            if ui.button("All Layers On").clicked() {
                for i in 0..app.doc.layers().len() {
                    if let Some(l) = app.doc.layer_mut(i) {
                        l.visible = true;
                    }
                }
                ui.close();
            }
            if ui.button("Select All").clicked() {
                ui.close();
            }
        });
    });
    ui.separator();
    let current = app.doc.current_layer();
    let n = app.doc.layers().len();
    let mut set_current = None;
    let mut toggle_on = None;
    let mut toggle_lock = None;
    let mut recolor = None;
    let mut rename: Option<(usize, String)> = None;
    let mut start_edit = None;
    let mut select = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("layer_table").striped(true).num_columns(9).spacing([6.0, 2.0]).show(ui, |ui| {
            for h in ["Name", "✓", "On", "Lock", "Color", "Material", "Linetype", "Print Color", "Print Width"] {
                ui.label(RichText::new(h).small().strong());
            }
            ui.end_row();
            for i in 0..n {
                let l = app.doc.layers()[i].clone();
                let selected = app.panels.layer_selected == Some(i);
                if let Some((ei, text)) = &mut app.panels.layer_editing {
                    if *ei == i {
                        let r = ui.text_edit_singleline(text);
                        if r.lost_focus() {
                            rename = Some((i, text.clone()));
                        }
                        r.request_focus();
                    } else {
                        let r = ui.selectable_label(selected, &l.name);
                        if r.clicked() {
                            select = Some(i);
                        }
                        if r.double_clicked() {
                            start_edit = Some(i);
                        }
                    }
                } else {
                    let r = ui.selectable_label(selected, &l.name);
                    if r.clicked() {
                        select = Some(i);
                    }
                    if r.double_clicked() {
                        start_edit = Some(i);
                    }
                }
                let r = ui.selectable_label(i == current, if i == current { "✓" } else { " " });
                if r.clicked() {
                    set_current = Some(i);
                }
                let bulb = ui.add(egui::Label::new(RichText::new("💡").color(if l.visible { Color32::from_rgb(230, 180, 0) } else { Color32::GRAY })).sense(egui::Sense::click()));
                if bulb.clicked() {
                    toggle_on = Some(i);
                }
                let lock = ui.add(egui::Label::new(if l.locked { "🔒" } else { "🔓" }).sense(egui::Sense::click()));
                if lock.clicked() {
                    toggle_lock = Some(i);
                }
                let mut c = [l.color[0], l.color[1], l.color[2]];
                if ui.color_edit_button_srgb(&mut c).changed() {
                    recolor = Some((i, c));
                }
                ui.label("Default");
                ui.label("Continuous");
                let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(l.color[0], l.color[1], l.color[2]));
                ui.label("Default");
                ui.end_row();
            }
        });
    });
    if let Some(i) = select {
        app.panels.layer_selected = Some(i);
    }
    if let Some(i) = start_edit {
        let name = app.doc.layers()[i].name.clone();
        app.panels.layer_editing = Some((i, name));
    }
    if let Some((i, name)) = rename {
        app.panels.layer_editing = None;
        let name = name.trim().to_string();
        if !name.is_empty() && app.doc.find_layer(&name).is_none_or(|j| j == i) {
            app.doc.begin_undo("Rename layer");
            if let Some(l) = app.doc.layer_mut(i) {
                l.name = name;
            }
        }
    }
    if let Some(i) = set_current {
        app.doc.set_current_layer(i);
    }
    if let Some(i) = toggle_on {
        if let Some(l) = app.doc.layer_mut(i) {
            l.visible = !l.visible;
        }
        let ids: Vec<_> = app.doc.objects().filter(|o| o.attributes.layer == i).map(|o| o.id).collect();
        for id in ids {
            app.doc.select(id, false);
        }
    }
    if let Some(i) = toggle_lock {
        if let Some(l) = app.doc.layer_mut(i) {
            l.locked = !l.locked;
        }
        let ids: Vec<_> = app.doc.objects().filter(|o| o.attributes.layer == i).map(|o| o.id).collect();
        for id in ids {
            app.doc.select(id, false);
        }
    }
    if let Some((i, c)) = recolor {
        if let Some(l) = app.doc.layer_mut(i) {
            l.color = [c[0], c[1], c[2], 255];
        }
    }
}

fn display(app: &mut App, ui: &mut egui::Ui) {
    let vp = app.active;
    ui.label(RichText::new(format!("Viewport: {}", app.viewports[vp].name)).strong());
    ui.horizontal(|ui| {
        ui.label("Display mode");
        let mut mode = app.viewports[vp].display;
        egui::ComboBox::from_id_salt("display_mode").selected_text(mode.name()).show_ui(ui, |ui| {
            for m in DisplayMode::ALL {
                ui.selectable_value(&mut mode, m, m.name());
            }
        });
        app.viewports[vp].display = mode;
    });
    ui.separator();
    ui.label(RichText::new("Grid").strong());
    ui.checkbox(&mut app.doc.settings.show_grid, "Show grid lines");
    ui.checkbox(&mut app.doc.settings.show_grid_axes, "Show grid axes");
    ui.checkbox(&mut app.doc.settings.show_world_axes, "Show world axes icon");
    ui.separator();
    ui.label(RichText::new("Background").strong());
    ui.horizontal(|ui| {
        ui.label("Solid color");
        ui.color_edit_button_srgb(&mut app.colors.background);
    });
    ui.separator();
    ui.label(RichText::new("Modeling aids").strong());
    ui.checkbox(&mut app.aids.grid_snap, "Grid snap");
    ui.checkbox(&mut app.aids.ortho, "Ortho");
    ui.checkbox(&mut app.aids.planar, "Planar");
    ui.horizontal(|ui| {
        ui.label("Ortho angle");
        ui.add(egui::DragValue::new(&mut app.aids.ortho_angle).speed(1.0).range(1.0..=180.0));
    });
    ui.horizontal(|ui| {
        ui.label("Snap radius (px)");
        ui.add(egui::DragValue::new(&mut app.aids.osnap.radius_px).speed(1.0).range(2.0..=60.0));
    });
}

fn help(app: &mut App, ui: &mut egui::Ui) {
    let name = app.runner.name().map(|s| s.to_string()).or_else(|| app.runner.last_command.clone());
    match name.as_deref().and_then(crate::commands::command_info) {
        Some(info) => {
            ui.label(RichText::new(info.name).strong().size(16.0));
            ui.label(info.description);
            if !info.aliases.is_empty() {
                ui.label(format!("Aliases: {}", info.aliases.join(", ")));
            }
        }
        None => {
            ui.label("Start a command to see its help here.");
        }
    }
    ui.separator();
    ui.label(RichText::new("Mouse").strong());
    ui.label("Left click: select. Shift: add, Ctrl: remove. Drag left→right: window, right→left: crossing.");
    ui.label("Right drag: rotate (perspective) / pan (parallel). Shift+right drag: pan. Ctrl+right drag: zoom. Wheel: zoom.");
    ui.label("Right click or Space: Enter. Esc: cancel. Enter with no command: repeat last command.");
    ui.separator();
    ui.label(RichText::new("Coordinates").strong());
    ui.label("x,y[,z]  r x,y (relative)  w x,y,z (world)  d<a (polar)  number = distance constraint  <a = angle constraint");
    ui.label("Ctrl+click: elevator mode. Tab: lock direction. Alt: suspend object snaps. Shift: toggle Ortho.");
    ui.separator();
    let all = crate::commands::registry::all_commands();
    let implemented = all.iter().filter(|c| crate::commands::registry::is_implemented(c.name)).count();
    ui.label(RichText::new(format!("{} Triceratops commands, {} implemented in triceratops", all.len(), implemented)).small());
    let mut filter = ui.data(|d| d.get_temp::<String>(egui::Id::unique("help_filter"))).unwrap_or_default();
    if ui.text_edit_singleline(&mut filter).changed() {
        ui.data_mut(|d| d.insert_temp(egui::Id::unique("help_filter"), filter.clone()));
    }
    let f = filter.to_ascii_lowercase();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for c in all.iter().filter(|c| f.is_empty() || c.name.to_ascii_lowercase().contains(&f)) {
            let done = crate::commands::registry::is_implemented(c.name);
            ui.horizontal(|ui| {
                let text = if done { RichText::new(c.name) } else { RichText::new(c.name).color(Color32::GRAY) };
                if ui.link(text).clicked() {
                    app.start_command(c.name);
                }
                ui.label(RichText::new(c.description).small().color(Color32::GRAY));
            });
        }
    });
}

fn notes(app: &mut App, ui: &mut egui::Ui) {
    let r = ui.add_sized(ui.available_size(), egui::TextEdit::multiline(&mut app.doc.settings.notes));
    if r.changed() {
        app.doc.modified = true;
    }
}


fn materials(app: &mut App, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut app.panels.material_new_name);
        if ui.button("New").clicked() {
            let name = if app.panels.material_new_name.trim().is_empty() { format!("Material {}", app.doc.settings.materials.len() + 1) } else { app.panels.material_new_name.trim().to_string() };
            app.doc.begin_undo("Material");
            app.doc.settings.materials.push(tri_doc::Material { name, ..Default::default() });
            app.panels.material_selected = Some(app.doc.settings.materials.len() - 1);
            app.panels.material_new_name.clear();
        }
        if ui.button("Delete").clicked() {
            if let Some(i) = app.panels.material_selected {
                if i < app.doc.settings.materials.len() {
                    app.doc.begin_undo("Delete material");
                    app.doc.settings.materials.remove(i);
                    app.panels.material_selected = None;
                }
            }
        }
    });
    ui.separator();
    let names: Vec<String> = app.doc.settings.materials.iter().map(|m| m.name.clone()).collect();
    for (i, n) in names.iter().enumerate() {
        if ui.selectable_label(app.panels.material_selected == Some(i), n).clicked() {
            app.panels.material_selected = Some(i);
        }
    }
    if names.is_empty() {
        ui.label(RichText::new("No materials. Objects use their display colour.").small().color(Color32::GRAY));
    }
    let Some(i) = app.panels.material_selected else { return };
    if i >= app.doc.settings.materials.len() {
        return;
    }
    ui.separator();
    let mut changed = false;
    {
        let m = &mut app.doc.settings.materials[i];
        ui.horizontal(|ui| {
            ui.label("Name");
            changed |= ui.text_edit_singleline(&mut m.name).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Color");
            let mut c = [m.color[0], m.color[1], m.color[2]];
            if ui.color_edit_button_srgb(&mut c).changed() {
                m.color = [c[0], c[1], c[2], 255];
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Gloss");
            changed |= ui.add(egui::Slider::new(&mut m.gloss, 0.0..=1.0)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Transparency");
            changed |= ui.add(egui::Slider::new(&mut m.transparency, 0.0..=1.0)).changed();
        });
        ui.horizontal(|ui| {
            ui.label("Texture");
            ui.label(m.texture.as_deref().map(|t| std::path::Path::new(t).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default()).unwrap_or_else(|| "none".into()));
            if ui.button("Choose...").clicked() {
                if let Some(p) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg"]).pick_file() {
                    m.texture = Some(p.to_string_lossy().to_string());
                    changed = true;
                }
            }
            if m.texture.is_some() && ui.button("Clear").clicked() {
                m.texture = None;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Texture size");
            changed |= ui.add(egui::DragValue::new(&mut m.texture_scale).speed(0.5).range(0.001..=1e6)).changed();
        });
    }
    if ui.button("Assign to selected objects").clicked() {
        let name = app.doc.settings.materials[i].name.clone();
        let ids = app.doc.selected_ids();
        app.doc.begin_undo("Assign material");
        for id in ids {
            if let Some(o) = app.doc.get_mut(id) {
                o.attributes.material = Some(name.clone());
            }
        }
        changed = true;
    }
    if changed {
        app.doc.touch_revision();
    }
    ui.label(RichText::new("Materials show in the Rendered display mode.").small().color(Color32::GRAY));
}

fn lights(app: &mut App, ui: &mut egui::Ui) {
    let lights = app.doc.lights();
    if lights.is_empty() {
        ui.label(RichText::new("No lights. Use PointLight, Spotlight, DirectionalLight, LinearLight or RectangularLight.").small().color(Color32::GRAY));
    }
    let mut changed = false;
    for (id, kind, _pos, _dir, color, intensity, on) in lights {
        ui.horizontal(|ui| {
            let mut on_v = on;
            if ui.checkbox(&mut on_v, format!("{kind} light #{id}")).changed() {
                set_user_text(app, id, "LightOn", if on_v { "Yes" } else { "No" });
                changed = true;
            }
            let mut inten = intensity;
            if ui.add(egui::DragValue::new(&mut inten).speed(0.05).range(0.0..=20.0)).changed() {
                set_user_text(app, id, "LightIntensity", &inten.to_string());
                changed = true;
            }
            let mut c = [(color[0] * 255.0) as u8, (color[1] * 255.0) as u8, (color[2] * 255.0) as u8];
            if ui.color_edit_button_srgb(&mut c).changed() {
                set_user_text(app, id, "LightColor", &format!("{},{},{}", c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0));
                changed = true;
            }
            if ui.small_button("Select").clicked() {
                app.doc.clear_selection();
                app.doc.select(id, true);
            }
        });
    }
    ui.separator();
    ui.label(RichText::new("Sun").strong());
    let mut sun_on = app.sun.is_some();
    if ui.checkbox(&mut sun_on, "Sun light").changed() {
        app.sun = if sun_on { Some(glam::DVec3::new(0.5, 0.5, 0.7).normalize()) } else { None };
        changed = true;
    }
    if changed {
        app.doc.touch_revision();
    }
}

fn set_user_text(app: &mut App, id: tri_doc::ObjectId, key: &str, value: &str) {
    if let Some(o) = app.doc.get_mut(id) {
        o.attributes.user_text.retain(|(k, _)| k != key);
        o.attributes.user_text.push((key.to_string(), value.to_string()));
    }
}

fn rendering(app: &mut App, ui: &mut egui::Ui) {
    let vp = app.active;
    ui.label(RichText::new(format!("Viewport: {}", app.viewports[vp].name)).strong());
    ui.horizontal(|ui| {
        ui.label("Background");
        ui.color_edit_button_srgb(&mut app.colors.background);
    });
    let mut grad = app.viewports[vp].gradient.is_some();
    if ui.checkbox(&mut grad, "Gradient background").changed() {
        app.viewports[vp].gradient = if grad { Some(([120, 150, 190], [40, 40, 45])) } else { None };
    }
    if let Some((top, bottom)) = app.viewports[vp].gradient.as_mut() {
        ui.horizontal(|ui| {
            ui.label("Top");
            ui.color_edit_button_srgb(top);
            ui.label("Bottom");
            ui.color_edit_button_srgb(bottom);
        });
    }
    ui.separator();
    let mut gp = app.ground_plane.is_some();
    if ui.checkbox(&mut gp, "Ground plane").changed() {
        app.ground_plane = if gp { Some(0.0) } else { None };
        app.doc.touch_revision();
    }
    if let Some(h) = app.ground_plane.as_mut() {
        ui.horizontal(|ui| {
            ui.label("Height");
            if ui.add(egui::DragValue::new(h).speed(0.1)).changed() {
                app.doc.touch_revision();
            }
        });
    }
    ui.separator();
    ui.label(RichText::new("Sun").strong());
    let mut sun_on = app.sun.is_some();
    if ui.checkbox(&mut sun_on, "Sun").changed() {
        app.sun = if sun_on { Some(glam::DVec3::new(0.5, 0.5, 0.7).normalize()) } else { None };
    }
    if let Some(s) = app.sun {
        let mut az = s.x.atan2(s.y).to_degrees();
        let mut alt = s.z.clamp(-1.0, 1.0).asin().to_degrees();
        let mut ch = false;
        ui.horizontal(|ui| {
            ui.label("Azimuth");
            ch |= ui.add(egui::DragValue::new(&mut az).speed(1.0)).changed();
            ui.label("Altitude");
            ch |= ui.add(egui::DragValue::new(&mut alt).speed(1.0).range(-90.0..=90.0)).changed();
        });
        if ch {
            let (sa, ca) = az.to_radians().sin_cos();
            let (sl, cl) = alt.to_radians().sin_cos();
            app.sun = Some(glam::DVec3::new(sa * cl, ca * cl, sl).normalize());
        }
    }
    ui.separator();
    ui.label(RichText::new("Render writes a PNG of the active viewport (Render, ViewCaptureToFile).").small().color(Color32::GRAY));
    if ui.button("Render to file...").clicked() {
        app.capture_requests.push(crate::app::CaptureRequest::default());
    }
}
