//! Drawing of the dockable containers (see `dock.rs`): docked side strips, floating windows,
//! tab dragging to detach, and the container menu to dock, float, hide or close.

use egui::{Pos2, Sense, Vec2};

use crate::app::App;
use crate::ui::dock::{ContainerKind, Placement};

/// A tab being dragged out of its container.
#[derive(Clone, Debug)]
struct TabDrag {
    kind: ContainerKind,
    tab: String,
    from: u32,
}

fn drag_id() -> egui::Id {
    egui::Id::unique("dock_tab_drag")
}

/// Docked containers: top, bottom, left, right (in that order so the sides fit between the bars).
pub fn draw_docked(app: &mut App, root: &mut egui::Ui) {
    let ctx = &root.ctx().clone();
    for side in [Placement::Top, Placement::Bottom, Placement::Left, Placement::Right] {
        for id in app.dock.containers_at(&side) {
            let Some(c) = app.dock.container(id) else { continue };
            let kind = c.kind;
            let size = c.size;
            let frame = app.theme.region_frame();
            let name = format!("dock_{id}");
            let rect = match side {
                Placement::Top | Placement::Bottom => {
                    let panel = if side == Placement::Top { egui::Panel::top(name.clone()) } else { egui::Panel::bottom(name.clone()) };
                    let r = panel.frame(frame).resizable(true).default_size(size).min_size(28.0).show(root, |ui| {
                        container_contents(app, ui, id, false);
                    });
                    r.response.rect
                }
                _ => {
                    let panel = if side == Placement::Left { egui::Panel::left(name.clone()) } else { egui::Panel::right(name.clone()) };
                    let min = if kind == ContainerKind::Toolbars { 40.0 } else { 160.0 };
                    let r = panel.frame(frame).resizable(true).default_size(size).min_size(min).max_size(700.0).show(root, |ui| {
                        container_contents(app, ui, id, true);
                    });
                    r.response.rect
                }
            };
            app.dock_rects.insert(id, rect);
            // remember the size the user dragged the panel to
            let new_size = match side { Placement::Top | Placement::Bottom => rect.height(), _ => rect.width() };
            if let Some(c) = app.dock.container_mut(id) {
                if (c.size - new_size).abs() > 0.5 {
                    c.size = new_size;
                    if !ctx.input(|i| i.pointer.any_down()) {
                        app.options_dirty = true;
                    }
                }
            }
        }
    }
    finish_tab_drag(app, ctx);
}

/// Floating containers as windows.
pub fn draw_floating(app: &mut App, ctx: &egui::Context) {
    for id in app.dock.floating() {
        let Some(c) = app.dock.container(id).cloned() else { continue };
        let Placement::Floating { x, y, w, h } = c.placement else { continue };
        let title = c.tabs.get(c.active).cloned().unwrap_or_else(|| "Panel".into());
        let mut open = true;
        let win = egui::Window::new(title.clone()).id(egui::Id::unique(("dock_float", id))).open(&mut open).default_pos(Pos2::new(x, y)).default_size(Vec2::new(w, h)).resizable(true).collapsible(true).show(ctx, |ui| {
            container_contents(app, ui, id, false);
        });
        if let Some(r) = win {
            let rect = r.response.rect;
            app.dock_rects.insert(id, rect);
            if let Some(c) = app.dock.container_mut(id) {
                let moved = match c.placement { Placement::Floating { x, y, w, h } => (x - rect.left()).abs() > 0.5 || (y - rect.top()).abs() > 0.5 || (w - rect.width()).abs() > 0.5 || (h - rect.height()).abs() > 0.5, _ => false };
                if moved {
                    c.placement = Placement::Floating { x: rect.left(), y: rect.top(), w: rect.width(), h: rect.height() };
                    if !ctx.input(|i| i.pointer.any_down()) {
                        app.options_dirty = true;
                    }
                }
            }
        }
        if !open {
            app.dock.hide(id);
            app.options_dirty = true;
        }
    }
    finish_tab_drag(app, ctx);
}

/// Tab strip plus the body of the active tab.
fn container_contents(app: &mut App, ui: &mut egui::Ui, id: u32, vertical: bool) {
    let Some(c) = app.dock.container(id).cloned() else { return };
    let icons = c.kind == ContainerKind::Panels;
    // --- tab strip
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, tab) in c.tabs.iter().enumerate() {
            let selected = i == c.active;
            let resp = if icons {
                let icon = super::panels::panel_icon(tab);
                super::icons::icon_button(ui, &super::icons::IconButton { name: icon, tooltip: tab, size: 18.0, flyout: false, selected })
            } else {
                ui.selectable_label(selected, tab.as_str())
            };
            let resp = resp.interact(Sense::click_and_drag());
            if resp.clicked() {
                if let Some(cm) = app.dock.container_mut(id) {
                    cm.active = i;
                }
                app.options_dirty = true;
            }
            if resp.drag_started() {
                ui.ctx().data_mut(|d| d.insert_temp(drag_id(), TabDrag { kind: c.kind, tab: tab.clone(), from: id }));
            }
            resp.context_menu(|ui| {
                tab_menu(app, ui, id, c.kind, tab);
            });
        }
        // container menu
        ui.menu_button("▾", |ui| {
            ui.set_min_width(170.0);
            container_menu(app, ui, id);
        }).response.on_hover_text("Dock, float, hide");
    });
    ui.separator();
    // --- body
    let Some(tab) = c.tabs.get(c.active).cloned() else { return };
    match c.kind {
        ContainerKind::Panels => {
            egui::ScrollArea::vertical().id_salt(("panel_scroll", id)).auto_shrink([false, false]).show(ui, |ui| {
                super::panels::draw_panel(app, ui, &tab);
            });
        }
        ContainerKind::Toolbars => super::toolbars::draw_group(app, ui, &tab, vertical),
    }
}

fn tab_menu(app: &mut App, ui: &mut egui::Ui, id: u32, kind: ContainerKind, tab: &str) {
    if ui.button("Float this tab").clicked() {
        let pos = ui.ctx().input(|i| i.pointer.latest_pos()).unwrap_or(Pos2::new(200.0, 200.0));
        app.dock.detach(kind, tab, pos.x, pos.y);
        app.options_dirty = true;
        ui.close();
    }
    if ui.button("Close this tab").clicked() {
        if let Some((cid, idx)) = app.dock.find_tab(kind, tab) {
            if cid == id {
                let empty = app.dock.container_mut(cid).map(|c| { c.tabs.remove(idx); if c.active >= c.tabs.len() { c.active = c.tabs.len().saturating_sub(1); } c.tabs.is_empty() }).unwrap_or(false);
                if empty {
                    app.dock.containers.retain(|c| c.id != cid);
                }
            }
        }
        app.options_dirty = true;
        ui.close();
    }
}

fn container_menu(app: &mut App, ui: &mut egui::Ui, id: u32) {
    let placement = app.dock.container(id).map(|c| c.placement.clone());
    for (label, side) in [("Dock top", Placement::Top), ("Dock bottom", Placement::Bottom), ("Dock left", Placement::Left), ("Dock right", Placement::Right)] {
        let on = placement.as_ref() == Some(&side);
        if ui.selectable_label(on, label).clicked() && !on {
            app.dock.dock(id, side);
            app.options_dirty = true;
        }
    }
    let floating = matches!(placement, Some(Placement::Floating { .. }));
    if ui.selectable_label(floating, "Float").clicked() && !floating {
        let pos = ui.ctx().input(|i| i.pointer.latest_pos()).unwrap_or(Pos2::new(200.0, 200.0));
        app.dock.float(id, pos.x + 10.0, pos.y + 10.0);
        app.options_dirty = true;
    }
    ui.separator();
    if ui.button("Hide").clicked() {
        app.dock.hide(id);
        app.options_dirty = true;
    }
}

/// When a dragged tab is released outside its container it becomes a floating window.
fn finish_tab_drag(app: &mut App, ctx: &egui::Context) {
    let Some(drag) = ctx.data(|d| d.get_temp::<TabDrag>(drag_id())) else { return };
    if ctx.input(|i| i.pointer.any_down()) {
        ctx.request_repaint();
        return;
    }
    ctx.data_mut(|d| d.remove::<TabDrag>(drag_id()));
    let Some(pos) = ctx.input(|i| i.pointer.latest_pos()) else { return };
    let inside = app.dock_rects.get(&drag.from).is_some_and(|r| r.contains(pos));
    if inside {
        return;
    }
    // dropped over another container of the same kind: move the tab there
    let target = app.dock_rects.iter().find(|(cid, r)| **cid != drag.from && r.contains(pos) && app.dock.container(**cid).is_some_and(|c| c.kind == drag.kind)).map(|(cid, _)| *cid);
    if let Some(tid) = target {
        if let Some((cid, idx)) = app.dock.find_tab(drag.kind, &drag.tab) {
            let empty = app.dock.container_mut(cid).map(|c| { c.tabs.remove(idx); if c.active >= c.tabs.len() { c.active = c.tabs.len().saturating_sub(1); } c.tabs.is_empty() }).unwrap_or(false);
            if empty {
                app.dock.containers.retain(|c| c.id != cid);
            }
            if let Some(t) = app.dock.container_mut(tid) {
                t.tabs.push(drag.tab.clone());
                t.active = t.tabs.len() - 1;
            }
        }
    } else {
        app.dock.detach(drag.kind, &drag.tab, pos.x - 40.0, pos.y - 10.0);
    }
    app.options_dirty = true;
}

