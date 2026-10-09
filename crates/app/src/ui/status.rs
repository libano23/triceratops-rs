//! Osnap control and status bar.

use egui::{Color32, RichText};

use crate::app::App;
use crate::commands::fmt_num;
use crate::snap::{OsnapSettings, SnapKind};

fn pane(ui: &mut egui::Ui, label: &str, on: bool, tooltip: &str) -> egui::Response {
    let v = ui.visuals();
    let (on_fg, off_fg, on_bg) = (v.widgets.active.fg_stroke.color, v.weak_text_color(), v.selection.bg_fill);
    let text = if on { RichText::new(label).strong().color(on_fg) } else { RichText::new(label).color(off_fg) };
    let r = ui.add(egui::Button::new(text).frame(false).fill(if on { on_bg } else { Color32::TRANSPARENT }));
    r.on_hover_text(tooltip)
}

pub fn status_bar(app: &mut App, ctx: &egui::Context) {
    let gap = app.theme.ui.region_gap;
    egui::TopBottomPanel::bottom("status_bar").frame(app.theme.region_frame().inner_margin(egui::Margin::symmetric(6, 2))).exact_height(22.0 + 2.0 * gap).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if !app.status_hint.is_empty() {
                ui.label(&app.status_hint);
                return;
            }
            // left sidebar toggle
            if ui.small_button("◧").on_hover_text("Toggle left sidebar").clicked() {
                app.toggle_side(crate::ui::dock::Placement::Left);
            }
            // CPlane / World
            let cw = if app.aids.world_coords { "World" } else { "CPlane" };
            if pane(ui, cw, false, "Construction plane / world coordinates toggle").clicked() {
                app.aids.world_coords = !app.aids.world_coords;
            }
            let (x, y, z) = match app.cursor {
                Some(c) => {
                    if app.aids.world_coords {
                        (c.world.x, c.world.y, c.world.z)
                    } else {
                        let cp = app.viewports[c.viewport].cplane();
                        cp.closest_parameters(c.world)
                    }
                }
                _ => (0.0, 0.0, 0.0),
            };
            let w = 72.0;
            ui.add_sized([w, 18.0], egui::Label::new(RichText::new(format!("x {}", fmt_num(x))).monospace()));
            ui.add_sized([w, 18.0], egui::Label::new(RichText::new(format!("y {}", fmt_num(y))).monospace()));
            ui.add_sized([w, 18.0], egui::Label::new(RichText::new(format!("z {}", fmt_num(z))).monospace()));
            // units / delta
            let units_text = match (app.runner.base_point(), app.cursor) {
                (Some(b), Some(c)) => format!("Delta {}", fmt_num((c.world - b).length())),
                _ => app.doc.settings.units.clone(),
            };
            ui.add_sized([90.0, 18.0], egui::Label::new(units_text));
            ui.separator();
            // layer pane with popup
            let layer_text = app.status_layer_text();
            let lr = ui.add(egui::Button::new(layer_text).frame(false));
            let popup_id = egui::Id::new("layer_popup");
            let mut popup_open: bool = ctx.data(|d| d.get_temp(popup_id).unwrap_or(false));
            if lr.clicked() {
                popup_open = !popup_open;
            }
            if lr.secondary_clicked() {
                app.show_panel("Layers");
            }
            if popup_open {
                let area = egui::Area::new(popup_id).fixed_pos(lr.rect.left_top() - egui::vec2(0.0, 22.0 * app.doc.layers().len() as f32 + 12.0)).order(egui::Order::Foreground).show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let sel = app.doc.selected_ids();
                        for i in 0..app.doc.layers().len() {
                            let l = app.doc.layers()[i].clone();
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                                ui.painter().rect_filled(rect, 0.0, Color32::from_rgb(l.color[0], l.color[1], l.color[2]));
                                if ui.selectable_label(i == app.doc.current_layer(), &l.name).clicked() {
                                    if sel.is_empty() {
                                        app.doc.set_current_layer(i);
                                    } else {
                                        app.doc.begin_undo("ChangeLayer");
                                        for id in &sel {
                                            if let Some(o) = app.doc.get_mut(*id) {
                                                o.attributes.layer = i;
                                            }
                                        }
                                    }
                                    popup_open = false;
                                }
                            });
                        }
                    });
                });
                if ctx.input(|i| i.pointer.any_pressed()) && !ctx.input(|i| i.pointer.interact_pos().map_or(false, |p| area.response.rect.expand(4.0).contains(p) || lr.rect.contains(p))) {
                    popup_open = false;
                }
            }
            ctx.data_mut(|d| d.insert_temp(popup_id, popup_open));
            ui.separator();
            if pane(ui, "Grid Snap", app.aids.grid_snap, "Toggle grid snap (F9)").clicked() {
                app.aids.grid_snap = !app.aids.grid_snap;
            }
            if pane(ui, "Ortho", app.aids.ortho, "Toggle ortho (F8)").clicked() {
                app.aids.ortho = !app.aids.ortho;
            }
            if pane(ui, "Planar", app.aids.planar, "Toggle planar mode").clicked() {
                app.aids.planar = !app.aids.planar;
            }
            let osnap_on = app.aids.osnap.any();
            if pane(ui, "Osnap", osnap_on, "Toggle object snaps").clicked() {
                app.aids.osnap.disable = !app.aids.osnap.disable;
            }
            if pane(ui, "SmartTrack", app.aids.smarttrack, "Toggle SmartTrack (not implemented yet)").clicked() {
                app.aids.smarttrack = !app.aids.smarttrack;
            }
            if pane(ui, "Auto CPlane", false, "Auto CPlane (not implemented yet)").clicked() {}
            if pane(ui, "Gumball", app.aids.gumball, "Toggle the gumball (not implemented yet)").clicked() {
                app.aids.gumball = !app.aids.gumball;
            }
            if pane(ui, "Record History", app.aids.record_history, "Toggle history recording").clicked() {
                app.aids.record_history = !app.aids.record_history;
            }
            if pane(ui, "Filter", app.selection_filter.is_some(), "Selection filter (SelectionFilter command)").clicked() {
                app.selection_filter = None;
            }
            if pane(ui, "Parametric", app.doc.state.parametric.enabled, "Toggle parametric sketch mode (constraints re-solve after every edit)").clicked() {
                app.doc.state.parametric.enabled = !app.doc.state.parametric.enabled;
                app.doc.touch_revision();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("◨").on_hover_text("Toggle right sidebar").clicked() {
                    app.toggle_side(crate::ui::dock::Placement::Right);
                }
                ui.label(RichText::new(format!("{} objects, {} selected", app.doc.len(), app.doc.selection.len())).small().color(Color32::GRAY));
            });
        });
    });
}

pub fn osnap_bar(app: &mut App, ctx: &egui::Context) {
    let gap = app.theme.ui.region_gap;
    egui::TopBottomPanel::bottom("osnap_bar").frame(app.theme.region_frame().inner_margin(egui::Margin::symmetric(6, 2))).exact_height(24.0 + 2.0 * gap).show(ctx, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let shift = ui.input(|i| i.modifiers.shift);
            for k in OsnapSettings::ALL {
                let mut on = app.aids.osnap.enabled(k) && !app.aids.osnap.disable;
                let one_shot = app.one_shot_snap == Some(k);
                let label = if one_shot { RichText::new(k.label()).color(Color32::from_rgb(200, 90, 0)).strong() } else { RichText::new(k.label()) };
                let r = ui.checkbox(&mut on, label);
                if r.clicked() {
                    if shift {
                        app.one_shot(k);
                    } else {
                        let v = !app.aids.osnap.enabled_raw(k);
                        app.aids.osnap.set(k, v);
                        if v {
                            app.aids.osnap.disable = false;
                        }
                    }
                }
                r.on_hover_text(snap_tip(k));
            }
            ui.separator();
            ui.checkbox(&mut app.aids.osnap.project, "Project").on_hover_text("Project object snaps to the construction plane");
            ui.checkbox(&mut app.aids.osnap.disable, "Disable").on_hover_text("Disable persistent object snaps (right-click: clear all)");
        });
    });
}

fn snap_tip(k: SnapKind) -> &'static str {
    match k {
        SnapKind::End => "Snap to the end of a curve",
        SnapKind::Near => "Snap onto a curve at the cursor location",
        SnapKind::Point => "Snap to a point object",
        SnapKind::Mid => "Snap to the midpoint of a curve or surface edge",
        SnapKind::Cen => "Snap to the center of a circle, arc, or closed polyline",
        SnapKind::Int => "Snap to the intersection of curves",
        SnapKind::Perp => "Snap perpendicular to a curve",
        SnapKind::Tan => "Snap tangent to a curve",
        SnapKind::Quad => "Snap to the quadrant of a curve relative to the construction plane",
        SnapKind::Knot => "Snap to a knot on a curve",
        SnapKind::Vertex => "Snap to a mesh vertex",
    }
}
