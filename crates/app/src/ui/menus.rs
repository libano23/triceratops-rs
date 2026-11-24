//! Menu bar: File, Edit, View, Curve, Surface, SubD, Solid, Mesh, Dimension,
//! Transform, Tools, Analyze, Render, Panels, Help.

use crate::app::{App, DisplayMode, Layout};
use crate::ui::menu_tree;

/// A menu entry: label, command, status-bar description.
type Entry = (&'static str, &'static str);

fn item(ui: &mut egui::Ui, app: &mut App, label: &str, cmd: &'static str) {
    let r = ui.button(label);
    if r.hovered() {
        if let Some(info) = crate::commands::command_info(cmd) {
            app.status_hint = info.description.to_string();
        }
    }
    if r.clicked() {
        app.start_command(cmd);
        ui.close();
    }
}

/// Render a static menu tree (see `menu_tree`).
fn tree(ui: &mut egui::Ui, app: &mut App, nodes: &[menu_tree::Node]) {
    for n in nodes {
        match n {
            menu_tree::Node::Item(label, cmd) => item(ui, app, label, cmd),
            menu_tree::Node::Sep => {
                ui.separator();
            }
            menu_tree::Node::Sub(label, children) => {
                ui.menu_button(*label, |ui| tree(ui, app, children));
            }
        }
    }
}

#[allow(dead_code)]
fn items(ui: &mut egui::Ui, app: &mut App, entries: &[Entry]) {
    for (label, cmd) in entries {
        if label.is_empty() {
            ui.separator();
        } else {
            item(ui, app, label, cmd);
        }
    }
}

pub fn menu_bar(app: &mut App, ctx: &egui::Context) {
    egui::TopBottomPanel::top("menu").frame(app.theme.header_frame()).show(ctx, |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| tree(ui, app, menu_tree::FILE));
            ui.menu_button("Edit", |ui| tree(ui, app, menu_tree::EDIT));
            ui.menu_button("View", |ui| {
                tree(ui, app, menu_tree::VIEW_EXTRA);
                ui.separator();
                ui.menu_button("Viewport Layout", |ui| {
                    if ui.button("4 Viewports").clicked() {
                        app.layout = Layout::Four;
                        ui.close();
                    }
                    if ui.button("3 Viewports").clicked() {
                        app.layout = Layout::Three;
                        ui.close();
                    }
                    if ui.button("Maximize Viewport\tCtrl+M").clicked() {
                        app.toggle_maximize(app.active);
                        ui.close();
                    }
                });
                ui.menu_button("Display", |ui| {
                    for m in DisplayMode::ALL {
                        if ui.button(m.name()).clicked() {
                            app.set_display(app.active, m);
                            ui.close();
                        }
                    }
                });
                ui.separator();
                let mut g = app.doc.settings.show_grid;
                if ui.checkbox(&mut g, "Grid\tF7").changed() {
                    app.doc.settings.show_grid = g;
                }
                let mut a = app.doc.settings.show_grid_axes;
                if ui.checkbox(&mut a, "Grid Axes").changed() {
                    app.doc.settings.show_grid_axes = a;
                }
                let mut w = app.doc.settings.show_world_axes;
                if ui.checkbox(&mut w, "World Axes Icon").changed() {
                    app.doc.settings.show_world_axes = w;
                }
                ui.separator();
                item(ui, app, "Viewport Properties...", "ViewportProperties");
                item(ui, app, "Command History...\tF2", "CommandHistory");
                ui.separator();
                let mut dark = app.dark_mode;
                if ui.checkbox(&mut dark, "Dark color scheme").changed() {
                    app.set_dark_mode(dark);
                }
                ui.menu_button("Theme", |ui| {
                    let current = app.theme.name.clone();
                    for t in crate::theme::Theme::all() {
                        if ui.radio(t.name == current, &t.name).clicked() {
                            app.set_theme(t);
                            ui.close();
                        }
                    }
                });
            });
            ui.menu_button("Curve", |ui| tree(ui, app, menu_tree::CURVE));
            ui.menu_button("Surface", |ui| tree(ui, app, menu_tree::SURFACE));
            ui.menu_button("SubD", |ui| tree(ui, app, menu_tree::SUBD));
            ui.menu_button("Solid", |ui| tree(ui, app, menu_tree::SOLID));
            ui.menu_button("Mesh", |ui| tree(ui, app, menu_tree::MESH));
            ui.menu_button("Dimension", |ui| tree(ui, app, menu_tree::DIMENSION));
            ui.menu_button("Transform", |ui| tree(ui, app, menu_tree::TRANSFORM));
            ui.menu_button("Tools", |ui| tree(ui, app, menu_tree::TOOLS));
            ui.menu_button("Analyze", |ui| tree(ui, app, menu_tree::ANALYZE));
            ui.menu_button("Render", |ui| tree(ui, app, menu_tree::RENDER));
            ui.menu_button("Panels", |ui| {
                for p in crate::ui::dock::PANELS {
                    let on = app.dock.is_visible(crate::ui::dock::ContainerKind::Panels, p);
                    if ui.selectable_label(on, p).clicked() {
                        app.show_panel(p);
                        ui.close();
                    }
                }
                ui.separator();
                ui.menu_button("Toolbars", |ui| {
                    for name in std::iter::once("Main").chain(crate::ui::toolbars::TAB_NAMES.iter().copied()) {
                        let on = app.dock.is_visible(crate::ui::dock::ContainerKind::Toolbars, name);
                        if ui.selectable_label(on, name).clicked() {
                            app.dock.show_tab(crate::ui::dock::ContainerKind::Toolbars, name);
                            app.options_dirty = true;
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.label(egui::RichText::new("Flyouts (open floating)").small());
                    for name in crate::ui::toolbars::FLYOUT_NAMES.iter().copied() {
                        let on = app.dock.find_tab(crate::ui::dock::ContainerKind::Toolbars, name).is_some() && app.dock.is_visible(crate::ui::dock::ContainerKind::Toolbars, name);
                        if ui.selectable_label(on, name).clicked() {
                            let pos = ui.ctx().input(|i| i.pointer.latest_pos()).unwrap_or(egui::pos2(200.0, 200.0));
                            app.dock.float_group(crate::ui::dock::ContainerKind::Toolbars, name, pos.x + 20.0, pos.y);
                            app.options_dirty = true;
                            ui.close();
                        }
                    }
                });
                if ui.button("Toggle Right Sidebar").clicked() {
                    app.toggle_side(crate::ui::dock::Placement::Right);
                    ui.close();
                }
                if ui.button("Toggle Left Sidebar").clicked() {
                    app.toggle_side(crate::ui::dock::Placement::Left);
                    ui.close();
                }
                ui.separator();
                if ui.button("Save Window Layout...").clicked() {
                    app.start_command("SaveWindowLayout");
                    ui.close();
                }
                if ui.button("Load Window Layout...").clicked() {
                    app.start_command("WindowLayout");
                    ui.close();
                }
                if ui.button("Reset Window Layout").clicked() {
                    app.start_command("Reset");
                    ui.close();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button("Command Help\tF1").clicked() {
                    app.show_help = true;
                    ui.close();
                }
                if ui.button("Command List").clicked() {
                    app.start_command("CommandList");
                    ui.close();
                }
                ui.separator();
                if ui.button("About triceratops").clicked() {
                    app.message_box = Some("triceratops 0.1 — a Rust re-implementation of the Triceratops interface and NURBS modeling workflow.".into());
                    ui.close();
                }
            });
        });
    });
}
