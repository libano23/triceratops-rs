//! Viewport area: viewports, titles with menus, viewport tabs, selection menu,
//! snap marker, selection rectangles and the world axes icon.

use egui::{Color32, PointerButton, Pos2, Rect, Sense, Stroke, Vec2};
use glam::DVec2;

use crate::app::{App, DisplayMode, Layout};
use crate::camera::{Projection, ViewRect};
use crate::theme::c32;

#[derive(Clone)]
struct TitleMenu {
    vp: usize,
    pos: Pos2,
}

fn title_menu_id() -> egui::Id {
    egui::Id::new("viewport_title_menu")
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let full = ui.available_rect_before_wrap();
        let tabs_h = if app.show_viewport_tabs { 22.0 } else { 0.0 };
        let area = Rect::from_min_max(full.min, Pos2::new(full.right(), full.bottom() - tabs_h));
        let tabs_rect = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - tabs_h), full.max);
        let gap = 3.0_f32.max(app.theme.ui.region_gap * 2.0);
        let region_gap = app.theme.ui.region_gap;
        let area = area.shrink(region_gap);
        let base_active = app.viewports[app.active].detail_of.unwrap_or(app.active);
        let rects: Vec<(usize, Rect)> = match app.layout {
            Layout::Single => vec![(base_active, area)],
            Layout::Four => {
                let w = (area.width() - gap) / 2.0;
                let h = (area.height() - gap) / 2.0;
                let r = |c: usize, rw: usize| Rect::from_min_size(Pos2::new(area.left() + c as f32 * (w + gap), area.top() + rw as f32 * (h + gap)), Vec2::new(w, h));
                vec![(0, r(0, 0)), (1, r(1, 0)), (2, r(0, 1)), (3, r(1, 1))]
            }
            Layout::Three => {
                let w = (area.width() - gap) / 2.0;
                let h = (area.height() - gap) / 2.0;
                vec![
                    (1, Rect::from_min_size(area.min, Vec2::new(w, area.height()))),
                    (0, Rect::from_min_size(Pos2::new(area.left() + w + gap, area.top()), Vec2::new(w, h))),
                    (2, Rect::from_min_size(Pos2::new(area.left() + w + gap, area.top() + h + gap), Vec2::new(w, h))),
                ]
            }
            Layout::Grid => {
                let n = app.viewports.iter().filter(|v| !v.is_detail()).count().max(1);
                let cols = (n as f32).sqrt().ceil().max(1.0) as usize;
                let rows = n.div_ceil(cols);
                let w = (area.width() - gap * (cols as f32 - 1.0)) / cols as f32;
                let h = (area.height() - gap * (rows as f32 - 1.0)) / rows as f32;
                app.viewport_tab_order.iter().copied().filter(|i| *i < app.viewports.len() && !app.viewports[*i].is_detail()).enumerate().map(|(k, vp)| {
                    let (c, r) = (k % cols, k / cols);
                    (vp, Rect::from_min_size(Pos2::new(area.left() + c as f32 * (w + gap), area.top() + r as f32 * (h + gap)), Vec2::new(w, h)))
                }).collect()
            }
        };
        // details of visible pages are nested viewports drawn inside the page
        let mut rects = rects;
        for (vp, rect) in rects.clone() {
            if app.viewports[vp].is_page() {
                app.viewports[vp].rect = ViewRect { x: rect.left() as f64, y: rect.top() as f64, w: rect.width() as f64, h: rect.height() as f64 };
                for d in app.details_of(vp) {
                    if let Some(r) = app.detail_screen_rect(d) {
                        let dr = Rect::from_min_size(Pos2::new(r.x as f32, r.y as f32), Vec2::new(r.w as f32, r.h as f32)).intersect(rect);
                        if dr.width() > 2.0 && dr.height() > 2.0 {
                            rects.push((d, dr));
                        }
                    }
                }
            }
        }
        let mods = ui.input(|i| i.modifiers);
        let menu_open = ctx.data(|d| d.get_temp::<TitleMenu>(title_menu_id())).is_some() || app.selection_menu.is_some() || app.popup_toolbar.is_some();
        for (vp, rect) in &rects {
            let (vp, rect) = (*vp, *rect);
            app.viewports[vp].rect = ViewRect { x: rect.left() as f64, y: rect.top() as f64, w: rect.width() as f64, h: rect.height() as f64 };
            let id = ui.id().with(("viewport", vp));
            let resp = ui.interact(rect, id, Sense::click_and_drag());
            let to_d = |p: Pos2| DVec2::new(p.x as f64, p.y as f64);

            if !menu_open {
                if let Some(pos) = resp.hover_pos() {
                    if app.window_drag.is_none() && app.drag_move.is_none() {
                        app.update_cursor(vp, to_d(pos));
                    }
                    let scroll = ui.input(|i| i.raw_scroll_delta.y);
                    if scroll.abs() > 0.0 {
                        app.zoom(vp, to_d(pos), scroll as f64);
                    }
                } else if app.cursor.is_some_and(|c| c.viewport == vp) && app.window_drag.is_none() && app.drag_move.is_none() {
                    app.clear_cursor();
                }
                if resp.clicked_by(PointerButton::Primary) {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        app.viewport_click(vp, to_d(pos), mods.shift, mods.ctrl);
                    }
                }
                if resp.clicked_by(PointerButton::Secondary) {
                    app.viewport_right_click(vp);
                }
                if resp.clicked_by(PointerButton::Middle) {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        app.active = vp;
                        app.popup_toolbar = Some(pos);
                    }
                }
                if resp.drag_started_by(PointerButton::Primary) {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        app.begin_left_drag(vp, to_d(pos));
                    }
                }
                if resp.dragged_by(PointerButton::Primary) {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        app.update_left_drag(to_d(pos));
                    }
                }
                if resp.drag_stopped_by(PointerButton::Primary) {
                    app.end_left_drag(mods.shift, mods.ctrl);
                }
                let delta = resp.drag_delta();
                if resp.drag_started_by(PointerButton::Secondary) || resp.drag_started_by(PointerButton::Middle) {
                    app.begin_view_change(vp);
                }
                if resp.dragged_by(PointerButton::Secondary) {
                    app.active = vp;
                    let rotatable = app.viewports[vp].camera.rotatable;
                    if mods.ctrl && !mods.shift {
                        app.zoom_drag(vp, delta.y as f64);
                    } else if mods.shift || !rotatable {
                        app.pan(vp, delta.x as f64, delta.y as f64);
                    } else {
                        app.orbit(vp, delta.x as f64, delta.y as f64);
                    }
                }
                if resp.dragged_by(PointerButton::Middle) {
                    app.active = vp;
                    app.pan(vp, delta.x as f64, delta.y as f64);
                }
                if resp.drag_started() || resp.clicked() || resp.secondary_clicked() {
                    app.active = vp;
                }
            }

            // ---- decorations -------------------------------------------------
            let painter = ui.painter_at(rect);
            let active = app.active == vp;
            let white_bg = app.viewports[vp].display.white_background();
            if app.theme.ui.region_rounding > 0 && app.viewports[vp].detail_of.is_none() {
                crate::theme::round_region_corners(&painter, rect, app.theme.ui.region_rounding as f32, c32(app.theme.ui.window_bg));
            }
            painter.rect_stroke(rect.shrink(0.5), app.theme.ui.region_rounding as f32, Stroke::new(1.0_f32, c32(app.theme.ui.viewport_border)), egui::StrokeKind::Inside);

            // title with triangle
            let name = app.viewports[vp].name.clone();
            let font = if active { egui::FontId::new(13.0, egui::FontFamily::Proportional) } else { egui::FontId::proportional(13.0) };
            let title_color = if white_bg {
                Color32::BLACK
            } else if active {
                c32(app.colors.active_title)
            } else {
                c32(app.colors.inactive_title)
            };
            let galley = painter.layout_no_wrap(name.clone(), font.clone(), title_color);
            let title_pos = rect.min + Vec2::new(7.0, 4.0);
            let title_rect = Rect::from_min_size(title_pos, galley.size() + Vec2::new(16.0, 2.0));
            let title_resp = ui.interact(title_rect, id.with("title"), Sense::click());
            if active {
                // Triceratops highlights the active viewport title
                painter.rect_filled(title_rect.expand(3.0), 2.0, if white_bg { Color32::from_rgb(220, 220, 220) } else { Color32::from_rgba_unmultiplied(255, 255, 255, 70) });
            }
            if title_resp.hovered() {
                painter.rect_filled(title_rect.expand(2.0), 2.0, Color32::from_black_alpha(40));
            }
            painter.galley(title_pos, galley, title_color);
            let tri = Pos2::new(title_rect.right() - 9.0, title_rect.center().y - 2.0);
            painter.add(egui::Shape::convex_polygon(vec![tri, tri + Vec2::new(7.0, 0.0), tri + Vec2::new(3.5, 5.0)], title_color, Stroke::NONE));
            if title_resp.double_clicked() {
                app.toggle_maximize(vp);
            } else if title_resp.clicked() || title_resp.secondary_clicked() {
                app.active = vp;
                ctx.data_mut(|d| d.insert_temp(title_menu_id(), TitleMenu { vp, pos: title_rect.left_bottom() }));
            }

            // world axes icon
            if app.doc.settings.show_world_axes && !app.viewports[vp].is_page() {
                let cam = &app.viewports[vp].camera;
                let (right, up) = cam.basis();
                let origin = Pos2::new(rect.left() + 22.0, rect.bottom() - 22.0);
                let len = 16.0;
                let axes = [
                    (glam::DVec3::X, c32(app.colors.world_x), "x"),
                    (glam::DVec3::Y, c32(app.colors.world_y), "y"),
                    (glam::DVec3::Z, c32(app.colors.world_z), "z"),
                ];
                for (axis, col, label) in axes {
                    let d = Vec2::new(axis.dot(right) as f32, -axis.dot(up) as f32);
                    let end = origin + d * len;
                    painter.line_segment([origin, end], Stroke::new(1.5_f32, col));
                    painter.text(end + d * 6.0, egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(10.0), col);
                }
            }

            // gumball
            if let Some(g) = app.gumball_screen(vp) {
                let cols = [c32(app.theme.gumball[0]), c32(app.theme.gumball[1]), c32(app.theme.gumball[2])];
                let o = Pos2::new(g.origin.x as f32, g.origin.y as f32);
                for i in 0..3 {
                    let e = Pos2::new(g.arrow_end[i].x as f32, g.arrow_end[i].y as f32);
                    painter.line_segment([o, e], Stroke::new(2.0_f32, cols[i]));
                    let d = (e - o).normalized();
                    let n = Vec2::new(-d.y, d.x);
                    painter.add(egui::Shape::convex_polygon(vec![e, e - d * 10.0 + n * 4.0, e - d * 10.0 - n * 4.0], cols[i], Stroke::NONE));
                    let sp = Pos2::new(g.scale_pos[i].x as f32, g.scale_pos[i].y as f32);
                    painter.rect_filled(Rect::from_center_size(sp, Vec2::splat(7.0)), 0.0, cols[i]);
                    let pts: Vec<Pos2> = g.arcs[i].iter().map(|p| Pos2::new(p.x as f32, p.y as f32)).collect();
                    if pts.len() >= 2 {
                        painter.add(egui::Shape::line(pts, Stroke::new(2.0_f32, cols[i])));
                    }
                }
                painter.circle_filled(o, 4.0, Color32::WHITE);
                painter.circle_stroke(o, 4.0, Stroke::new(1.0_f32, Color32::BLACK));
            }

            // snap marker + tooltip
            if let Some(c) = app.cursor {
                if c.viewport == vp {
                    if let Some(k) = c.snap {
                        let sp = Pos2::new(c.screen.x as f32, c.screen.y as f32);
                        if let Some((pp, _)) = app.viewports[vp].camera.project(&app.viewports[vp].rect, c.world) {
                            let mp = Pos2::new(pp.x as f32, pp.y as f32);
                            painter.rect_stroke(Rect::from_center_size(mp, Vec2::splat(9.0)), 0.0, Stroke::new(1.5_f32, if white_bg { Color32::BLACK } else { Color32::WHITE }), egui::StrokeKind::Inside);
                        }
                        let txt = if app.aids.osnap.project { format!("Projected {}", k.label()) } else { k.label().to_string() };
                        let (tbg, tfg) = (c32(app.theme.ui.tooltip_bg), c32(app.theme.ui.tooltip_text));
                        let g = painter.layout_no_wrap(txt, egui::FontId::proportional(12.0), tfg);
                        let tp = sp + Vec2::new(14.0, 14.0);
                        painter.rect_filled(Rect::from_min_size(tp, g.size() + Vec2::new(6.0, 2.0)), app.theme.ui.rounding as f32, tbg);
                        painter.galley(tp + Vec2::new(3.0, 1.0), g, tfg);
                    }
                }
            }

            // window / crossing selection rectangle
            if let Some(d) = app.window_drag {
                if d.viewport == vp {
                    let r = Rect::from_two_pos(Pos2::new(d.start.x as f32, d.start.y as f32), Pos2::new(d.current.x as f32, d.current.y as f32));
                    let crossing = d.current.x < d.start.x;
                    let col = if white_bg { Color32::BLACK } else { Color32::WHITE };
                    if crossing {
                        let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
                        for w in pts.windows(2) {
                            painter.add(egui::Shape::dashed_line(&[w[0], w[1]], Stroke::new(1.0_f32, col), 4.0, 3.0));
                        }
                    } else {
                        painter.rect_stroke(r, 0.0, Stroke::new(1.0_f32, col), egui::StrokeKind::Inside);
                    }
                }
            }
        }

        // viewport tabs
        if !app.show_viewport_tabs {
            return;
        }
        let p = ui.painter_at(tabs_rect);
        let t = app.theme.ui.clone();
        p.rect_filled(tabs_rect, 0.0, c32(t.window_bg));
        let mut x = tabs_rect.left() + 4.0;
        for vp in app.viewport_tab_order.clone() {
            let name = app.viewports[vp].name.clone();
            let fg = c32(t.text);
            let g = p.layout_no_wrap(name, egui::FontId::proportional(12.0), fg);
            let w = g.size().x + 18.0;
            let r = Rect::from_min_size(Pos2::new(x, tabs_rect.top() + 1.0), Vec2::new(w, tabs_h - 2.0));
            let resp = ui.interact(r, ui.id().with(("vptab", vp)), Sense::click());
            let active = app.active == vp;
            let (fill_a, fill_i) = (c32(t.tab_active), c32(t.tab_inactive));
            let rr = egui::CornerRadius { nw: t.rounding, ne: t.rounding, sw: 0, se: 0 };
            p.rect_filled(r, rr, if active { fill_a } else { fill_i });
            p.rect_stroke(r, rr, Stroke::new(1.0_f32, c32(t.border)), egui::StrokeKind::Inside);
            p.galley(Pos2::new(x + 9.0, r.center().y - g.size().y * 0.5), g, fg);
            if resp.clicked() {
                app.active = vp;
                if app.layout == Layout::Single {
                    // stays maximized on the chosen viewport
                }
            }
            if resp.double_clicked() {
                app.toggle_maximize(vp);
            }
            x += w + 2.0;
        }
        let plus = Rect::from_min_size(Pos2::new(x + 2.0, tabs_rect.top() + 1.0), Vec2::new(18.0, tabs_h - 2.0));
        p.text(plus.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(14.0), c32(t.text_dim));
        let plus_resp = ui.interact(plus, ui.id().with("vptab_plus"), Sense::click());
        if plus_resp.clicked() {
            app.layout_op("new:297;210;1", "");
        }
    });
    title_menu(app, ctx);
    selection_menu(app, ctx);
}

fn title_menu(app: &mut App, ctx: &egui::Context) {
    let Some(menu) = ctx.data(|d| d.get_temp::<TitleMenu>(title_menu_id())) else { return };
    let vp = menu.vp;
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("title_menu_area")).fixed_pos(menu.pos).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(190.0);
            let maximized = app.layout == Layout::Single;
            if ui.button(if maximized { "Restore" } else { "Maximize" }).clicked() {
                app.toggle_maximize(vp);
                close = true;
            }
            ui.separator();
            for m in DisplayMode::ALL {
                let on = app.viewports[vp].display == m;
                if ui.radio(on, m.name()).clicked() {
                    app.set_display(vp, m);
                    close = true;
                }
            }
            ui.separator();
            ui.menu_button("Set View", |ui| {
                for v in ["Top", "Bottom", "Left", "Right", "Front", "Back", "Perspective"] {
                    if ui.button(v).clicked() {
                        app.active = vp;
                        app.set_view(v);
                        close = true;
                        ui.close();
                    }
                }
            });
            ui.menu_button("Set CPlane", |ui| {
                ui.label("The construction plane follows the view (World Top / Front / Right).");
            });
            ui.menu_button("Zoom", |ui| {
                if ui.button("Extents").clicked() {
                    app.active = vp;
                    app.zoom_extents(false, false);
                    close = true;
                    ui.close();
                }
                if ui.button("Selected").clicked() {
                    app.active = vp;
                    app.zoom_extents(true, false);
                    close = true;
                    ui.close();
                }
                if ui.button("Extents All").clicked() {
                    app.zoom_extents(false, true);
                    close = true;
                    ui.close();
                }
            });
            ui.menu_button("Viewport Layout", |ui| {
                if ui.button("4 Viewports").clicked() {
                    app.layout = Layout::Four;
                    close = true;
                    ui.close();
                }
                if ui.button("3 Viewports").clicked() {
                    app.layout = Layout::Three;
                    close = true;
                    ui.close();
                }
            });
            ui.separator();
            let mut persp = app.viewports[vp].camera.projection == Projection::Perspective;
            if ui.checkbox(&mut persp, "Perspective projection").changed() {
                let cam = &mut app.viewports[vp].camera;
                cam.projection = if persp { Projection::Perspective } else { Projection::Parallel };
                cam.rotatable = true;
                cam.up_override = None;
            }
            ui.separator();
            if ui.button("Viewport Properties...").clicked() {
                app.show_panel("Properties");
                close = true;
            }
        });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| area.response.rect.expand(4.0).contains(p)));
    if close || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        ctx.data_mut(|d| d.remove::<TitleMenu>(title_menu_id()));
    }
}

fn selection_menu(app: &mut App, ctx: &egui::Context) {
    let Some(menu) = app.selection_menu.clone() else { return };
    let pos = Pos2::new(menu.screen.x as f32, menu.screen.y as f32);
    let mut pick: Option<Option<tri_doc::ObjectId>> = None;
    let mut hovered = None;
    let area = egui::Area::new(egui::Id::new("selection_menu")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            ui.set_min_width(160.0);
            for id in &menu.candidates {
                let Some(o) = app.doc.get(*id) else { continue };
                let label = match &o.attributes.name {
                    Some(n) => format!("{} \"{}\"", o.geometry.kind(), n),
                    None => o.geometry.kind().to_string(),
                };
                let r = ui.selectable_label(menu.hovered == Some(*id), label);
                if r.hovered() {
                    hovered = Some(*id);
                }
                if r.clicked() {
                    pick = Some(Some(*id));
                }
            }
            ui.separator();
            if ui.button("None").clicked() {
                pick = Some(None);
            }
        });
    });
    if let Some(h) = hovered {
        if let Some(m) = &mut app.selection_menu {
            m.hovered = Some(h);
        }
    }
    let outside = ctx.input(|i| i.pointer.any_pressed()) && !ctx.input(|i| i.pointer.interact_pos().is_some_and(|p| area.response.rect.expand(4.0).contains(p)));
    match pick {
        Some(p) => app.selection_menu_pick(p),
        None if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) => app.selection_menu = None,
        None => {}
    }
}
