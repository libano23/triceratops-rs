//! egui user interface laid out like the Triceratops 8 window.

pub mod command_area;
pub mod dialogs;
pub mod dock;
pub mod dock_ui;
pub mod icons;
pub mod menu_tree;
pub mod menus;
pub mod panels;
pub mod status;
pub mod toolbars;
pub mod viewports;

use egui::{Key, Modifiers};

use crate::app::App;

pub fn draw(app: &mut App, ctx: &egui::Context) {
    let theme_id = egui::Id::new("applied_theme_revision");
    let applied: Option<u64> = ctx.data(|d| d.get_temp(theme_id));
    if applied != Some(app.theme_revision) {
        crate::theme::apply_theme(ctx, &app.theme);
        ctx.data_mut(|d| d.insert_temp(theme_id, app.theme_revision));
    }
    app.status_hint.clear();
    app.alt_down = ctx.input(|i| i.modifiers.alt);
    app.shift_down = ctx.input(|i| i.modifiers.shift);
    handle_shortcuts(app, ctx);
    menus::menu_bar(app, ctx);
    if app.show_command_area {
        command_area::show(app, ctx);
    }
    if app.dock.show_status_bar {
        status::status_bar(app, ctx);
    }
    if app.dock.show_osnap_bar {
        status::osnap_bar(app, ctx);
    }
    dock_ui::draw_docked(app, ctx);
    viewports::show(app, ctx);
    dock_ui::draw_floating(app, ctx);
    toolbars::flyouts(app, ctx);
    if !ctx.input(|i| i.pointer.any_down()) {
        app.autosave_options();
    }
    dialogs::show(app, ctx);
}

fn handle_shortcuts(app: &mut App, ctx: &egui::Context) {
    let text_prompt = app.runner.prompt().map_or(false, |p| p.kind == crate::commands::PromptKind::Text);
    let typing = !app.cmd_text.is_empty();
    let modal = app.loft_dialog.is_some() || app.options_page.is_some() || app.docprops_page.is_some() || app.message_box.is_some();
    let mut cmds: Vec<&'static str> = Vec::new();
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            app.cancel();
        }
        if modal {
            return;
        }
        let ctrl = Modifiers::CTRL;
        let pairs: [(Modifiers, Key, &'static str); 25] = [
            (ctrl, Key::Z, "Undo"),
            (ctrl, Key::Y, "Redo"),
            (ctrl, Key::A, "SelAll"),
            (ctrl, Key::S, "Save"),
            (ctrl, Key::O, "Open"),
            (ctrl, Key::N, "New"),
            (ctrl, Key::H, "Hide"),
            (ctrl, Key::L, "Lock"),
            (ctrl, Key::J, "Join"),
            (ctrl, Key::G, "Group"),
            (ctrl, Key::M, "MaxViewport"),
            (ctrl, Key::C, "CopyToClipboard"),
            (ctrl, Key::V, "Paste"),
            (ctrl, Key::X, "Cut"),
            (ctrl, Key::P, "Print"),
            (ctrl, Key::F1, "Top"),
            (ctrl, Key::F2, "Front"),
            (ctrl, Key::F3, "Right"),
            (ctrl, Key::F4, "Perspective"),
            (Modifiers::NONE, Key::F1, "Help"),
            (Modifiers::NONE, Key::F2, "CommandHistory"),
            (Modifiers::NONE, Key::F3, "Properties"),
            (Modifiers::NONE, Key::F7, "Grid"),
            (Modifiers::NONE, Key::F8, "Ortho"),
            (Modifiers::NONE, Key::F9, "Snap"),
        ];
        for (m, k, c) in pairs {
            if i.consume_key(m, k) {
                cmds.push(c);
            }
        }
        if i.consume_key(Modifiers::NONE, Key::F10) {
            cmds.push("PointsOn");
        }
        if i.consume_key(Modifiers::NONE, Key::F11) {
            cmds.push("PointsOff");
        }
        if !typing && i.consume_key(Modifiers::NONE, Key::Delete) {
            app.delete_key();
        }
        if app.walk_mode && !typing && !text_prompt {
            let step = app.viewports[app.active].camera.distance * 0.05;
            let mut mv = glam::DVec3::ZERO;
            let mut turn = 0.0;
            let (fwd, right) = {
                let c = &app.viewports[app.active].camera;
                let f = c.view_dir();
                let f = glam::DVec3::new(f.x, f.y, 0.0).normalize_or(glam::DVec3::X);
                (f, f.cross(glam::DVec3::Z))
            };
            if i.consume_key(Modifiers::NONE, Key::W) { mv += fwd * step; }
            if i.consume_key(Modifiers::NONE, Key::S) { mv -= fwd * step; }
            if i.consume_key(Modifiers::NONE, Key::A) { mv -= right * step; }
            if i.consume_key(Modifiers::NONE, Key::D) { mv += right * step; }
            if i.consume_key(Modifiers::NONE, Key::Q) { mv += glam::DVec3::Z * step; }
            if i.consume_key(Modifiers::NONE, Key::E) { mv -= glam::DVec3::Z * step; }
            if i.consume_key(Modifiers::NONE, Key::ArrowLeft) { turn += 5.0; }
            if i.consume_key(Modifiers::NONE, Key::ArrowRight) { turn -= 5.0; }
            if mv.length() > 0.0 {
                let c = &mut app.viewports[app.active].camera;
                c.target += mv;
            }
            if turn != 0.0 {
                app.viewports[app.active].camera.rotate_camera(-(turn as f64).to_radians(), 0.0);
            }
        }
        if i.consume_key(ctrl, Key::Tab) {
            app.next_viewport();
        }
        if i.consume_key(Modifiers::NONE, Key::Tab) {
            app.tab_lock();
        }
        if !text_prompt && i.consume_key(Modifiers::NONE, Key::Space) {
            app.submit();
        }
        if !typing {
            for k in [Key::ArrowLeft, Key::ArrowRight, Key::ArrowUp, Key::ArrowDown, Key::PageUp, Key::PageDown, Key::Home, Key::End] {
                let shift = i.modifiers.shift;
                let c = i.modifiers.ctrl;
                if i.consume_key(Modifiers::NONE, k) || i.consume_key(Modifiers::SHIFT, k) || i.consume_key(Modifiers::CTRL, k) {
                    app.key_nav(k, shift, c);
                }
            }
        }
    });
    for c in cmds {
        match c {
            "Grid" => app.doc.settings.show_grid = !app.doc.settings.show_grid,
            "Ortho" => app.aids.ortho = !app.aids.ortho,
            "Snap" => app.aids.grid_snap = !app.aids.grid_snap,
            "Help" => app.show_help = !app.show_help,
            "CommandHistory" => app.show_history = !app.show_history,
            "Properties" => {
                app.show_panel("Properties");
            }
            "MaxViewport" => app.toggle_maximize(app.active),
            other => app.start_command(other),
        }
    }
}
