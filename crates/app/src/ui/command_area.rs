//! Command window: history, prompt with clickable options, text entry and autocomplete.

use egui::{Key, RichText};

use crate::app::App;
use crate::commands;
use crate::theme::c32;

pub fn show(app: &mut App, ctx: &egui::Context) {
    let bg = c32(app.colors.prompt_background);
    let fg = c32(app.colors.prompt_text);
    let hover = c32(app.colors.prompt_hover);
    egui::TopBottomPanel::top("command_window")
        .resizable(true)
        .default_height(62.0)
        .min_height(40.0)
        .frame(app.theme.region_frame().fill(bg).inner_margin(egui::Margin::symmetric(6, 2)))
        .show(ctx, |ui| {
            let line_h = 18.0;
            let avail = (ui.available_height() - line_h - 6.0).max(line_h);
            // history
            egui::ScrollArea::vertical().id_salt("cmd_history").max_height(avail).stick_to_bottom(true).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                let n = app.log.len();
                let start = n.saturating_sub(400);
                for line in &app.log[start..] {
                    ui.label(RichText::new(line).color(fg));
                }
            });
            // prompt line
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let mut clicked_option: Option<String> = None;
                match app.runner.prompt().cloned() {
                    Some(p) => {
                        let mut msg = p.msg.clone();
                        if let Some(d) = &p.default {
                            msg.push_str(&format!(" <{d}>"));
                        }
                        ui.label(RichText::new(msg).color(fg));
                        if !p.options.is_empty() {
                            ui.label(RichText::new(" ( ").color(fg));
                            for (i, o) in p.options.iter().enumerate() {
                                if i > 0 {
                                    ui.label(RichText::new("  ").color(fg));
                                }
                                let r = ui.add(egui::Label::new(RichText::new(o.label()).color(hover).underline()).sense(egui::Sense::click()));
                                if r.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if r.clicked() {
                                    clicked_option = Some(o.name.clone());
                                }
                            }
                            ui.label(RichText::new(" )").color(fg));
                        }
                        ui.label(RichText::new(": ").color(fg));
                    }
                    None => {
                        ui.label(RichText::new("Command: ").color(fg));
                    }
                }
                if let Some(o) = clicked_option {
                    app.option_clicked(&o);
                }
                let edit_id = egui::Id::new("cmdline");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut app.cmd_text)
                        .id(edit_id)
                        .desired_width(f32::INFINITY)
                        .frame(false)
                        .text_color(fg)
                        .lock_focus(true),
                );
                // autocomplete list while typing a command name
                let show_ac = !app.runner.is_active() && !app.cmd_text.trim().is_empty();
                let (exact, fuzzy) = if show_ac { commands::lookup(&app.cmd_text, 12) } else { (vec![], vec![]) };
                let total = exact.len() + fuzzy.len();
                if total == 0 {
                    app.autocomplete_index = None;
                }
                let (up, down, enter) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown), i.key_pressed(Key::Enter)));
                if show_ac && total > 0 {
                    if down {
                        app.autocomplete_index = Some(app.autocomplete_index.map_or(0, |i| (i + 1) % total));
                    }
                    if up {
                        app.autocomplete_index = Some(app.autocomplete_index.map_or(total - 1, |i| (i + total - 1) % total));
                    }
                }
                let mut run: Option<&'static str> = None;
                if show_ac && total > 0 {
                    let pos = resp.rect.left_bottom();
                    egui::Area::new(egui::Id::new("autocomplete")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).fill(c32(app.theme.ui.popup_bg)).show(ui, |ui| {
                            ui.set_min_width(220.0);
                            let mut idx = 0;
                            for (k, list) in [exact.as_slice(), fuzzy.as_slice()].iter().enumerate() {
                                if k == 1 && !list.is_empty() && !exact.is_empty() {
                                    ui.separator();
                                }
                                for c in list.iter() {
                                    let selected = app.autocomplete_index == Some(idx);
                                    let r = ui.selectable_label(selected, RichText::new(c.name).color(c32(app.theme.ui.text)));
                                    if r.clicked() {
                                        run = Some(c.name);
                                    }
                                    idx += 1;
                                }
                            }
                        });
                    });
                }
                let submitted = enter && (resp.has_focus() || resp.lost_focus());
                if let Some(name) = run {
                    app.cmd_text = name.to_string();
                    app.submit();
                } else if submitted {
                    if let Some(i) = app.autocomplete_index {
                        let all: Vec<&str> = exact.iter().chain(fuzzy.iter()).map(|c| c.name).collect();
                        if let Some(n) = all.get(i) {
                            app.cmd_text = n.to_string();
                        }
                    }
                    app.submit();
                }
                // the command line always has the keyboard unless another text field is active
                let other_focus = ctx.memory(|m| m.focused().is_some_and(|f| f != edit_id));
                if !other_focus && !resp.has_focus() {
                    resp.request_focus();
                }
            });
        });
}
