//! Dialogs: Loft Options, Options, Document Properties, Command History, Help, messages.

use egui::{Color32, RichText};

use crate::app::{App, LOFT_STYLES};

pub fn show(app: &mut App, ctx: &egui::Context) {
    loft_dialog(app, ctx);
    options(app, ctx);
    docprops(app, ctx);
    command_history(app, ctx);
    help_window(app, ctx);
    message_box(app, ctx);
    save_prompt(app, ctx);
    recovery(app, ctx);
}

/// Options › General: autosave (and, later, updates).
fn general_page(app: &mut App, ui: &mut egui::Ui) {
    ui.label(RichText::new("Autosave").strong());
    ui.horizontal(|ui| {
        if ui.checkbox(&mut app.autosave, "Autosave every").changed() {
            app.options_dirty = true;
        }
        if ui.add_enabled(app.autosave, egui::DragValue::new(&mut app.autosave_minutes).range(1..=60)).changed() {
            app.options_dirty = true;
        }
        ui.label("minutes");
    });
    ui.label(RichText::new("A modified model is written to its own file in the settings folder; your file is not touched.").weak());
}

/// Autosave files left by a session that ended badly.
fn recovery(app: &mut App, ctx: &egui::Context) {
    if app.recovered.is_empty() {
        return;
    }
    let now = std::time::SystemTime::now();
    let mut open: Option<std::path::PathBuf> = None;
    let mut delete: Option<usize> = None;
    let mut later = false;
    egui::Window::new("Recover autosaved models").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        ui.label("Triceratops did not close normally. These models were autosaved:");
        ui.add_space(6.0);
        egui::Grid::new("recovery").spacing([12.0, 6.0]).show(ui, |ui| {
            for (i, (p, t)) in app.recovered.iter().enumerate() {
                let name = p.file_name().map(|n| n.to_string_lossy().trim_end_matches(".autosave.3dm").to_string()).unwrap_or_default();
                ui.label(RichText::new(name).strong());
                ui.label(crate::autosave::ago(*t, now));
                if ui.button("Open").clicked() {
                    open = Some(p.clone());
                }
                if ui.button("Delete").clicked() {
                    delete = Some(i);
                }
                ui.end_row();
            }
        });
        ui.add_space(6.0);
        if ui.button("Later").on_hover_text("Keep them and ask again at the next start").clicked() {
            later = true;
        }
    });
    if let Some(p) = open {
        if app.doc.modified {
            app.log.push("Save or close the current model before opening a recovered one.".into());
        } else {
            app.recovered.retain(|(q, _)| *q != p);
            app.open_recovered(&p);
        }
    }
    if let Some(i) = delete {
        let (p, _) = app.recovered.remove(i);
        let _ = std::fs::remove_file(p);
    }
    if later {
        app.recovered.clear();
    }
}

/// "Save changes?" before closing, opening or clearing a modified model.
fn save_prompt(app: &mut App, ctx: &egui::Context) {
    if app.save_prompt.is_none() {
        return;
    }
    let mut answer: Option<crate::app::SaveAnswer> = None;
    egui::Window::new("Unsaved changes").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        ui.label(format!("Save changes to {}?", app.doc.title()));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                answer = Some(crate::app::SaveAnswer::Save);
            }
            if ui.button("Don't Save").clicked() {
                answer = Some(crate::app::SaveAnswer::DontSave);
            }
            if ui.button("Cancel").clicked() {
                answer = Some(crate::app::SaveAnswer::Cancel);
            }
        });
    });
    if answer.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        answer = Some(crate::app::SaveAnswer::Cancel);
    }
    if let Some(a) = answer {
        app.answer_save_prompt(a);
    }
}

fn loft_dialog(app: &mut App, ctx: &egui::Context) {
    if app.loft_dialog.is_none() {
        return;
    }
    let mut result: Option<bool> = None;
    egui::Window::new("Loft Options").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        let d = app.loft_dialog.as_mut().unwrap();
        ui.horizontal(|ui| {
            ui.label("Style");
            egui::ComboBox::from_id_salt("loft_style").selected_text(LOFT_STYLES[d.style]).show_ui(ui, |ui| {
                for (i, s) in LOFT_STYLES.iter().enumerate() {
                    ui.selectable_value(&mut d.style, i, *s);
                }
            });
        });
        ui.checkbox(&mut d.closed, "Closed loft");
        ui.add_enabled(false, egui::Checkbox::new(&mut false, "Match start tangent"));
        ui.add_enabled(false, egui::Checkbox::new(&mut false, "Match end tangent"));
        ui.add_enabled(false, egui::Checkbox::new(&mut false, "Split at tangents"));
        ui.separator();
        ui.label("Cross-section curve options");
        ui.radio_value(&mut d.rebuild, 0, "Do not simplify");
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.rebuild, 1, "Rebuild with");
            ui.add(egui::DragValue::new(&mut d.rebuild_count).range(2..=200));
            ui.label("control points");
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut d.rebuild, 2, "Refit within");
            ui.add(egui::DragValue::new(&mut d.refit_tol).speed(0.001).range(0.0001..=10.0));
            ui.label("millimeters");
        });
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                result = Some(true);
            }
            if ui.button("Cancel").clicked() {
                result = Some(false);
            }
            if ui.button("Help").clicked() {
                app.show_help = true;
            }
        });
    });
    if let Some(ok) = result {
        app.finish_loft(ok);
    }
}

const OPTION_PAGES: [&str; 6] = ["General", "Appearance", "Colors", "Modeling Aids", "Mouse", "AI Assistants"];

fn options(app: &mut App, ctx: &egui::Context) {
    let Some(page) = app.options_page.clone() else { return };
    let mut open = true;
    let mut new_page = page.clone();
    egui::Window::new("Triceratops Options").open(&mut open).default_width(620.0).default_height(420.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(150.0);
                ui.label(RichText::new("Triceratops Options").strong());
                for p in OPTION_PAGES {
                    let indent = if p == "Colors" { "    " } else { "" };
                    if ui.selectable_label(page == p, format!("{indent}{p}")).clicked() {
                        new_page = p.to_string();
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| match page.as_str() {
                "General" => general_page(app, ui),
                "AI Assistants" => {
                    ui.label(RichText::new("AI Assistants").strong());
                    if ui.checkbox(&mut app.allow_assistants, "Allow AI assistants (MCP)").changed() {
                        app.options_dirty = true;
                    }
                    ui.label(
                        RichText::new("Claude, Codex and other MCP clients can model in this window through `triceratops mcp`. Only programs running as you on this computer can connect.")
                            .small()
                            .color(Color32::GRAY),
                    );
                    ui.label(if app.assistant_connected { "An assistant is connected." } else { "No assistant is connected." });
                }
                "Appearance" => {
                    ui.label(RichText::new("Appearance").strong());
                    ui.label("Language used for display: English");
                    ui.horizontal(|ui| {
                        ui.label("Color scheme");
                        let mut dark = app.dark_mode;
                        egui::ComboBox::from_id_salt("scheme").selected_text(if dark { "Dark" } else { "Light" }).show_ui(ui, |ui| {
                            ui.selectable_value(&mut dark, false, "Light");
                            ui.selectable_value(&mut dark, true, "Dark");
                        });
                        if dark != app.dark_mode {
                            app.set_dark_mode(dark);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Icon size");
                        let r = ui.add(egui::Slider::new(&mut app.icon_size, crate::ui::icons::ICON_MIN..=crate::ui::icons::ICON_MAX).step_by(2.0).suffix(" px"));
                        if r.changed() {
                            app.options_dirty = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Theme");
                        let current = app.theme.name.clone();
                        let mut chosen = current.clone();
                        egui::ComboBox::from_id_salt("theme").selected_text(&current).show_ui(ui, |ui| {
                            for t in crate::theme::Theme::all() {
                                ui.selectable_value(&mut chosen, t.name.clone(), &t.name);
                            }
                        });
                        if chosen != current {
                            app.set_theme_by_name(&chosen);
                        }
                        if ui.button("Save as...").on_hover_text("Save the current colours as a user theme (Theme command, Save option)").clicked() {
                            app.start_command("Theme");
                        }
                        if ui.button("Themes folder").clicked() {
                            if let Some(d) = crate::theme::Theme::user_dir() {
                                let _ = std::fs::create_dir_all(&d);
                                let _ = crate::commands::simple::open_external(&d.to_string_lossy());
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Graphics backend");
                        let mut b = app.backend.clone();
                        egui::ComboBox::from_id_salt("backend").selected_text(&b).show_ui(ui, |ui| {
                            for o in ["auto", "gl", "vulkan"] {
                                ui.selectable_value(&mut b, o.to_string(), o);
                            }
                        });
                        if b != app.backend {
                            app.backend = b;
                            app.save_options();
                            app.log.push("Restart triceratops to switch the graphics backend.".into());
                        }
                    });
                    ui.separator();
                    ui.label("Command prompt");
                    ui.horizontal(|ui| {
                        ui.label("Background");
                        ui.color_edit_button_srgb(&mut app.colors.prompt_background);
                        ui.label("Text color");
                        ui.color_edit_button_srgb(&mut app.colors.prompt_text);
                        ui.label("Hover color");
                        ui.color_edit_button_srgb(&mut app.colors.prompt_hover);
                    });
                    ui.separator();
                    ui.label("Show the following items");
                    ui.checkbox(&mut app.doc.settings.show_world_axes, "World axes icon");
                    let mut sb = app.dock.show_status_bar;
                    if ui.checkbox(&mut sb, "Status bar").changed() {
                        app.dock.show_status_bar = sb;
                        app.options_dirty = true;
                    }
                    let mut ob = app.dock.show_osnap_bar;
                    if ui.checkbox(&mut ob, "Osnap bar").changed() {
                        app.dock.show_osnap_bar = ob;
                        app.options_dirty = true;
                    }
                    let mut ca = app.show_command_area;
                    if ui.checkbox(&mut ca, "Command area").changed() {
                        app.show_command_area = ca;
                        app.options_dirty = true;
                    }
                    let mut vt = app.show_viewport_tabs;
                    if ui.checkbox(&mut vt, "Viewport tabs").changed() {
                        app.show_viewport_tabs = vt;
                        app.options_dirty = true;
                    }
                    ui.separator();
                    ui.label("Window layout");
                    ui.horizontal(|ui| {
                        if ui.button("Save layout as...").clicked() {
                            app.start_command("SaveWindowLayout");
                        }
                        if ui.button("Load layout...").clicked() {
                            app.start_command("WindowLayout");
                        }
                        if ui.button("Reset layout").clicked() {
                            app.layout_file("reset", "");
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Reset all options").on_hover_text("Restores every option, the theme, the modeling aids and the window layout to the defaults").clicked() {
                            app.layout_file("reset_all", "");
                        }
                        if ui.button("Options folder").clicked() {
                            if let Some(p) = crate::app::options_path().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
                                let _ = std::fs::create_dir_all(&p);
                                let _ = crate::commands::simple::open_external(&p.to_string_lossy());
                            }
                        }
                    });
                }
                "Colors" => {
                    ui.label(RichText::new("Colors").strong());
                    egui::Grid::new("colors").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                        let c = &mut app.colors;
                        let rows: [(&str, &mut [u8; 3]); 15] = [
                            ("Background", &mut c.background),
                            ("Major grid line", &mut c.major_grid),
                            ("Minor grid line", &mut c.minor_grid),
                            ("X-axis line", &mut c.x_axis),
                            ("Y-axis line", &mut c.y_axis),
                            ("Z-axis line", &mut c.z_axis),
                            ("World axes icon X", &mut c.world_x),
                            ("World axes icon Y", &mut c.world_y),
                            ("World axes icon Z", &mut c.world_z),
                            ("Selected objects", &mut c.selected),
                            ("Locked objects", &mut c.locked),
                            ("Feedback", &mut c.feedback),
                            ("Tracking lines", &mut c.tracking),
                            ("Active viewport title", &mut c.active_title),
                            ("Inactive viewport title", &mut c.inactive_title),
                        ];
                        for (label, col) in rows {
                            ui.label(label);
                            ui.color_edit_button_srgb(col);
                            ui.end_row();
                        }
                    });
                    if ui.button("Restore Defaults").clicked() {
                        app.colors = Default::default();
                        let d = app.dark_mode;
                        app.colors.set_scheme(d);
                    }
                }
                "Modeling Aids" => {
                    ui.label(RichText::new("Modeling Aids").strong());
                    ui.checkbox(&mut app.aids.grid_snap, "Grid snap on");
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut app.aids.ortho, "Ortho snap every");
                        ui.add(egui::DragValue::new(&mut app.aids.ortho_angle).range(1.0..=180.0));
                        ui.label("degrees");
                    });
                    ui.checkbox(&mut app.aids.planar, "Planar mode on");
                    ui.separator();
                    ui.label("Object snaps");
                    ui.checkbox(&mut app.aids.osnap.disable, "Disable object snaps");
                    ui.checkbox(&mut app.aids.osnap.project, "Project object snaps to CPlane");
                    ui.horizontal(|ui| {
                        ui.label("Snap radius");
                        ui.add(egui::DragValue::new(&mut app.aids.osnap.radius_px).range(1.0..=100.0));
                        ui.label("pixels");
                    });
                }
                "Mouse" => {
                    ui.label(RichText::new("Mouse").strong());
                    ui.label("Right mouse button: rotate view in perspective viewports, pan in parallel viewports.");
                    ui.label("Shift + right drag: pan.  Ctrl + right drag: zoom.  Middle button: Popup toolbar.");
                }
                _ => {}
            });
        });
    });
    app.options_page = if open { Some(new_page) } else { None };
}

const DOCPROPS_PAGES: [&str; 3] = ["Units", "Grid", "Notes"];

fn docprops(app: &mut App, ctx: &egui::Context) {
    let Some(page) = app.docprops_page.clone() else { return };
    let mut open = true;
    let mut new_page = page.clone();
    egui::Window::new("Document Properties").open(&mut open).default_width(560.0).default_height(380.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(140.0);
                ui.label(RichText::new("Document Properties").strong());
                for p in DOCPROPS_PAGES {
                    if ui.selectable_label(page == p, p).clicked() {
                        new_page = p.to_string();
                    }
                }
            });
            ui.separator();
            ui.vertical(|ui| match page.as_str() {
                "Units" => {
                    ui.label(RichText::new("Units and tolerances").strong());
                    let s = &mut app.doc.settings;
                    ui.horizontal(|ui| {
                        ui.label("Model units");
                        egui::ComboBox::from_id_salt("units").selected_text(&s.units).show_ui(ui, |ui| {
                            for u in ["Millimeters", "Centimeters", "Meters", "Inches", "Feet"] {
                                ui.selectable_value(&mut s.units, u.to_string(), u);
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Absolute tolerance");
                        ui.add(egui::DragValue::new(&mut s.absolute_tolerance).speed(0.001).range(1e-6..=10.0));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Angle tolerance");
                        ui.add(egui::DragValue::new(&mut s.angle_tolerance_deg).speed(0.1).range(0.01..=90.0));
                        ui.label("degrees");
                    });
                }
                "Grid" => {
                    ui.label(RichText::new("Grid properties").strong());
                    let s = &mut app.doc.settings;
                    ui.horizontal(|ui| {
                        ui.label("Grid line count");
                        ui.add(egui::DragValue::new(&mut s.grid_line_count).range(1..=100_000));
                    });
                    ui.horizontal(|ui| {
                        ui.label("Minor grid line spacing");
                        ui.add(egui::DragValue::new(&mut s.grid_minor_spacing).speed(0.1).range(0.001..=1e6));
                        ui.label(s.units.to_ascii_lowercase());
                    });
                    ui.horizontal(|ui| {
                        ui.label("Major lines every");
                        ui.add(egui::DragValue::new(&mut s.grid_major_every).range(1..=1000));
                        ui.label("minor grid lines");
                    });
                    ui.checkbox(&mut s.show_grid, "Show grid lines");
                    ui.checkbox(&mut s.show_grid_axes, "Show grid axes");
                    ui.checkbox(&mut s.show_world_axes, "Show world axes icon");
                    ui.separator();
                    ui.label(RichText::new("Grid snap").strong());
                    ui.horizontal(|ui| {
                        ui.label("Snap spacing");
                        ui.add(egui::DragValue::new(&mut s.grid_snap_spacing).speed(0.1).range(0.001..=1e6));
                        ui.label(s.units.to_ascii_lowercase());
                    });
                }
                "Notes" => {
                    ui.add_sized([ui.available_width(), 250.0], egui::TextEdit::multiline(&mut app.doc.settings.notes));
                }
                _ => {}
            });
        });
    });
    app.docprops_page = if open { Some(new_page) } else { None };
}

fn command_history(app: &mut App, ctx: &egui::Context) {
    if !app.show_history {
        return;
    }
    let mut open = true;
    egui::Window::new("Command History").open(&mut open).default_width(640.0).default_height(400.0).show(ctx, |ui| {
        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                app.log.clear();
            }
            if ui.button("Copy to clipboard").clicked() {
                ctx.copy_text(app.log.join("\n"));
            }
        });
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            let n = app.log.len();
            for line in &app.log[n.saturating_sub(500)..] {
                ui.label(RichText::new(line).monospace());
            }
        });
    });
    app.show_history = open;
}

fn help_window(app: &mut App, ctx: &egui::Context) {
    if !app.show_help {
        return;
    }
    let mut open = true;
    egui::Window::new("Triceratops Help").open(&mut open).default_width(520.0).default_height(420.0).show(ctx, |ui| {
        let name = app.runner.name().map(|s| s.to_string()).or_else(|| app.runner.last_command.clone());
        if let Some(info) = name.as_deref().and_then(crate::commands::command_info) {
            ui.label(RichText::new(info.name).strong().size(18.0));
            ui.label(info.description);
            ui.separator();
        }
        ui.label("See docs/ui-reference.md for the interaction conventions implemented by triceratops.");
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("help_cmds").striped(true).show(ui, |ui| {
                for c in crate::commands::registry::all_commands() {
                    let done = crate::commands::registry::is_implemented(c.name);
                    ui.label(if done { RichText::new(c.name).strong() } else { RichText::new(c.name).color(Color32::GRAY) });
                    ui.label(RichText::new(c.aliases.join(", ")).color(Color32::GRAY));
                    ui.label(c.description);
                    ui.end_row();
                }
            });
        });
    });
    app.show_help = open;
}

fn message_box(app: &mut App, ctx: &egui::Context) {
    let Some(msg) = app.message_box.clone() else { return };
    let mut close = false;
    egui::Window::new("Triceratops").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
        ui.label(msg);
        ui.add_space(8.0);
        if ui.button("OK").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            close = true;
        }
    });
    if close {
        app.message_box = None;
    }
}
