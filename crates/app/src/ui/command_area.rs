//! Command window: history, prompt with clickable options, text entry and autocomplete.

use egui::{Key, RichText};

use crate::app::App;
use crate::commands;
use crate::theme::c32;

/// Height of one prompt line.
const LINE_H: f32 = 18.0;

/// The command area's smallest height: the prompt line alone (line, frame margins, spacing).
pub const ONE_LINE: f32 = LINE_H + 2.0 * 2.0 + 4.0;

/// Whether `avail` (the panel's inner height) leaves room for history above the prompt.
pub fn history_fits(avail: f32) -> bool {
    avail >= 2.0 * LINE_H + 6.0
}

/// Lines shown in the tooltip of a one-line command area.
const TOOLTIP_LINES: usize = 8;

pub fn show(app: &mut App, root: &mut egui::Ui) {
    let ctx = &root.ctx().clone();
    let bg = c32(app.colors.prompt_background);
    let fg = c32(app.colors.prompt_text);
    let hover = c32(app.colors.prompt_hover);
    egui::Panel::top("command_window")
        .resizable(true)
        .default_size(62.0)
        .min_size(ONE_LINE)
        .frame(app.theme.region_frame().fill(bg).inner_margin(egui::Margin::symmetric(6, 2)))
        .show(root, |ui| {
            let line_h = LINE_H;
            let one_line = !history_fits(ui.available_height());
            // history, when there is room for it; a one-line area shows it in a tooltip
            let recent: Vec<String> = if one_line { app.log[app.log.len().saturating_sub(TOOLTIP_LINES)..].to_vec() } else { Vec::new() };
            let with_history = |r: egui::Response| if one_line && !recent.is_empty() {
                r.on_hover_ui(|ui| {
                    for line in &recent {
                        ui.label(line);
                    }
                })
            } else {
                r
            };
            if !one_line {
                let avail = (ui.available_height() - line_h - 6.0).max(line_h);
                egui::ScrollArea::vertical().id_salt("cmd_history").max_height(avail).stick_to_bottom(true).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let n = app.log.len();
                    let start = n.saturating_sub(400);
                    for line in &app.log[start..] {
                        ui.label(RichText::new(line).color(fg));
                    }
                });
            }
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
                        with_history(ui.label(RichText::new(msg).color(fg)));
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
                        with_history(ui.label(RichText::new("Command: ").color(fg)));
                    }
                }
                if let Some(o) = clicked_option {
                    app.option_clicked(&o);
                }
                let edit_id = egui::Id::unique("cmdline");
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut app.cmd_text)
                        .id(edit_id)
                        .desired_width(f32::INFINITY)
                        .frame(egui::Frame::NONE)
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
                    egui::Area::new(egui::Id::unique("autocomplete")).fixed_pos(pos).order(egui::Order::Foreground).show(ctx, |ui| {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_hides_history_and_two_lines_show_it() {
        // inner height of the smallest panel: ONE_LINE minus the 2 + 2 px frame margins
        assert!(!history_fits(ONE_LINE - 4.0));
        assert!(!history_fits(ONE_LINE));
        assert!(history_fits(2.0 * LINE_H + 6.0));
        // the old default height (62 px) still shows history
        assert!(history_fits(62.0 - 4.0));
    }

    #[test]
    fn command_area_draws_and_takes_typing() {
        let mut app = App::new();
        app.log.push("earlier message".into());
        let ctx = egui::Context::default();
        let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0))), ..Default::default() };
        for frame in 0..3 {
            if frame == 2 {
                input.events = vec![egui::Event::Text("Line".into())];
            }
            let mut out = ctx.run_ui(input.clone(), |ui| show(&mut app, ui));
            out.textures_delta.clear();
            input.events.clear();
        }
        assert_eq!(app.cmd_text, "Line");
    }

    /// Dragging the area's bottom edge to the top leaves the prompt line alone, still typeable.
    #[test]
    fn command_area_drags_down_to_one_line() {
        let mut app = App::new();
        app.log.push("earlier message".into());
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0));
        let mut bottom = 0.0;
        let frame = |events: Vec<egui::Event>, app: &mut App| -> f32 {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut b = 0.0;
            let mut out = ctx.run_ui(input, |ui| {
                show(app, ui);
                b = ui.available_rect_before_wrap().min.y;
            });
            out.textures_delta.clear();
            b
        };
        for _ in 0..2 {
            bottom = frame(vec![], &mut app);
        }
        assert!(bottom > 50.0, "default height shows history: {bottom}");
        let edge = egui::pos2(400.0, bottom);
        let press = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(vec![egui::Event::PointerMoved(edge)], &mut app);
        frame(vec![press(edge, true)], &mut app);
        for y in [bottom - 10.0, 30.0, 10.0, 0.0] {
            frame(vec![egui::Event::PointerMoved(egui::pos2(400.0, y))], &mut app);
        }
        frame(vec![press(egui::pos2(400.0, 0.0), false)], &mut app);
        let small = frame(vec![], &mut app);
        assert!(small <= ONE_LINE + 1.0, "one line high after the drag: {small} (one line {ONE_LINE})");
        frame(vec![egui::Event::Text("Line".into())], &mut app);
        assert_eq!(app.cmd_text, "Line");
    }
}
