//! Procedurally drawn toolbar icons (Triceratops' icons are not redistributable, so
//! these are simple glyphs in a similar colour language).

use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

pub const ICON: f32 = 24.0;

const CURVE: Color32 = Color32::from_rgb(20, 20, 20);
const BLUE: Color32 = Color32::from_rgb(60, 110, 200);
const YELLOW: Color32 = Color32::from_rgb(240, 200, 60);
const GREEN: Color32 = Color32::from_rgb(90, 170, 90);
const RED: Color32 = Color32::from_rgb(210, 70, 60);
const GRAY: Color32 = Color32::from_rgb(120, 120, 120);
const SURF: Color32 = Color32::from_rgb(190, 205, 230);

pub struct IconButton<'a> {
    pub name: &'a str,
    pub tooltip: &'a str,
    pub size: f32,
    pub flyout: bool,
    pub selected: bool,
}

/// Draw an icon button; returns the response.
pub fn icon_button(ui: &mut egui::Ui, b: &IconButton) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(b.size + 6.0), Sense::click_and_drag());
    let dark = ui.visuals().dark_mode;
    let p = ui.painter();
    if resp.hovered() || b.selected {
        let (fill, stroke) = if dark {
            let v = ui.visuals();
            (if b.selected { v.selection.bg_fill } else { v.widgets.hovered.weak_bg_fill }, v.selection.stroke.color)
        } else {
            (if b.selected { Color32::from_rgb(190, 210, 240) } else { Color32::from_rgb(214, 228, 248) }, Color32::from_rgb(120, 160, 210))
        };
        p.rect_filled(rect, 2.0, fill);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0_f32, stroke), egui::StrokeKind::Inside);
    }
    draw_icon_themed(p, rect.shrink(3.0), b.name, dark);
    if b.flyout {
        let c = rect.right_bottom() - Vec2::new(1.0, 1.0);
        let tri = if dark { Color32::from_gray(220) } else { Color32::BLACK };
        p.add(egui::Shape::convex_polygon(vec![c, c - Vec2::new(5.0, 0.0), c - Vec2::new(0.0, 5.0)], tri, Stroke::NONE));
    }
    resp.on_hover_text(b.tooltip)
}

/// Draw an icon and, on dark themes, recolour what was just painted so the glyphs read on a
/// dark background: near-black strokes become light, white and pale fills become dark, while
/// the accent colours (blue, red, green, yellow) keep their hue and brightness.
pub fn draw_icon_themed(p: &egui::Painter, r: Rect, name: &str, dark: bool) {
    if !dark {
        draw_icon(p, r, name);
        return;
    }
    let layer = p.layer_id();
    let start = p.ctx().graphics_mut(|g| g.entry(layer).next_idx());
    draw_icon(p, r, name);
    let end = p.ctx().graphics_mut(|g| g.entry(layer).next_idx());
    p.ctx().graphics_mut(|g| {
        let list = g.entry(layer);
        for i in start.0..end.0 {
            list.mutate_shape(egui::layers::ShapeIdx(i), |cs| recolor_shape(&mut cs.shape));
        }
    });
}

/// Dark-theme mapping of an icon colour (see `draw_icon_themed`).
pub fn dark_icon_color(c: Color32) -> Color32 {
    if c.a() == 0 {
        return c;
    }
    let (r, g, b) = (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) * 0.5;
    let sat = if max == min { 0.0 } else if l < 0.5 { (max - min) / (max + min) } else { (max - min) / (2.0 - max - min) };
    // accent colours (saturated, mid lightness) stay as they are
    if sat > 0.25 && (0.3..=0.75).contains(&l) {
        return c;
    }
    // neutral mid greys are lifted a little; everything else has its lightness inverted
    let new_l = if (0.35..=0.65).contains(&l) && sat < 0.15 { (l + 0.2).min(0.85) } else { 1.0 - l };
    // keep the hue: scale the channels around the old lightness onto the new one
    let map = |x: f32| -> f32 {
        let d = x - l;
        (new_l + d * if sat > 0.0 { 0.8 } else { 0.0 }).clamp(0.0, 1.0)
    };
    Color32::from_rgba_unmultiplied((map(r) * 255.0).round() as u8, (map(g) * 255.0).round() as u8, (map(b) * 255.0).round() as u8, c.a())
}

fn recolor_shape(shape: &mut egui::Shape) {
    use egui::epaint::{ColorMode, Shape};
    let stroke_color = |cm: &mut ColorMode| {
        if let ColorMode::Solid(c) = cm {
            *c = dark_icon_color(*c);
        }
    };
    match shape {
        Shape::Vec(v) => v.iter_mut().for_each(recolor_shape),
        Shape::Circle(c) => {
            c.fill = dark_icon_color(c.fill);
            c.stroke.color = dark_icon_color(c.stroke.color);
        }
        Shape::Ellipse(e) => {
            e.fill = dark_icon_color(e.fill);
            e.stroke.color = dark_icon_color(e.stroke.color);
        }
        Shape::LineSegment { stroke, .. } => stroke.color = dark_icon_color(stroke.color),
        Shape::Path(path) => {
            path.fill = dark_icon_color(path.fill);
            stroke_color(&mut path.stroke.color);
        }
        Shape::Rect(rs) => {
            rs.fill = dark_icon_color(rs.fill);
            rs.stroke.color = dark_icon_color(rs.stroke.color);
        }
        Shape::Text(t) => {
            // the galley carries the colour it was laid out with (= fallback_color for
            // Painter::text): force the mapped colour through the override
            let base = t.override_text_color.unwrap_or(t.fallback_color);
            t.fallback_color = dark_icon_color(t.fallback_color);
            t.override_text_color = Some(dark_icon_color(base));
        }
        Shape::Mesh(m) => {
            let m = std::sync::Arc::make_mut(m);
            for v in &mut m.vertices {
                v.color = dark_icon_color(v.color);
            }
        }
        Shape::QuadraticBezier(q) => {
            q.fill = dark_icon_color(q.fill);
            stroke_color(&mut q.stroke.color);
        }
        Shape::CubicBezier(cb) => {
            cb.fill = dark_icon_color(cb.fill);
            stroke_color(&mut cb.stroke.color);
        }
        _ => {}
    }
}

#[cfg(test)]
mod dark_tests {
    use super::*;

    #[test]
    fn dark_mapping_inverts_neutrals_and_keeps_accents() {
        let l = |c: Color32| (c.r() as u32 + c.g() as u32 + c.b() as u32) / 3;
        assert!(l(dark_icon_color(CURVE)) > 200, "black strokes become light");
        assert!(l(dark_icon_color(Color32::WHITE)) < 60, "white fills become dark");
        let s = dark_icon_color(SURF);
        assert!(l(s) < 110 && s.b() > s.r(), "pale surface fill becomes a dark blue-grey: {s:?}");
        for c in [BLUE, RED, GREEN, YELLOW] {
            assert_eq!(dark_icon_color(c), c, "accent kept");
        }
        assert!(l(dark_icon_color(GRAY)) > l(GRAY), "mid grey lifted");
    }
}

fn pt(r: &Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(r.left() + r.width() * x, r.top() + r.height() * y)
}

fn poly(p: &egui::Painter, r: &Rect, pts: &[(f32, f32)], color: Color32, w: f32) {
    let v: Vec<Pos2> = pts.iter().map(|(x, y)| pt(r, *x, *y)).collect();
    p.add(egui::Shape::line(v, Stroke::new(w, color)));
}

fn fill(p: &egui::Painter, r: &Rect, pts: &[(f32, f32)], color: Color32, stroke: Color32) {
    let v: Vec<Pos2> = pts.iter().map(|(x, y)| pt(r, *x, *y)).collect();
    p.add(egui::Shape::convex_polygon(v, color, Stroke::new(1.0_f32, stroke)));
}

fn circle(p: &egui::Painter, r: &Rect, cx: f32, cy: f32, rad: f32, color: Color32, w: f32) {
    p.circle_stroke(pt(r, cx, cy), rad * r.width(), Stroke::new(w, color));
}

fn arrow(p: &egui::Painter, r: &Rect, a: (f32, f32), b: (f32, f32), color: Color32) {
    let pa = pt(r, a.0, a.1);
    let pb = pt(r, b.0, b.1);
    p.line_segment([pa, pb], Stroke::new(1.5_f32, color));
    let d = (pb - pa).normalized();
    let n = Vec2::new(-d.y, d.x);
    p.add(egui::Shape::convex_polygon(vec![pb, pb - d * 5.0 + n * 3.0, pb - d * 5.0 - n * 3.0], color, Stroke::NONE));
}

fn label(p: &egui::Painter, r: &Rect, text: &str, color: Color32) {
    p.text(r.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(9.0), color);
}

pub fn draw_icon(p: &egui::Painter, r: Rect, name: &str) {
    let r = &r;
    match name {
        "cancel" => {
            poly(p, r, &[(0.2, 0.2), (0.8, 0.8)], RED, 2.5);
            poly(p, r, &[(0.8, 0.2), (0.2, 0.8)], RED, 2.5);
        }
        "point" => {
            p.rect_filled(Rect::from_center_size(pt(r, 0.5, 0.5), Vec2::splat(5.0)), 0.0, CURVE);
        }
        "points" => {
            for (x, y) in [(0.25, 0.3), (0.6, 0.25), (0.45, 0.65), (0.8, 0.7)] {
                p.rect_filled(Rect::from_center_size(pt(r, x, y), Vec2::splat(4.0)), 0.0, CURVE);
            }
        }
        "line" => poly(p, r, &[(0.15, 0.85), (0.85, 0.15)], CURVE, 2.0),
        "lines" => poly(p, r, &[(0.1, 0.8), (0.4, 0.2), (0.6, 0.7), (0.9, 0.3)], CURVE, 2.0),
        "polyline" => poly(p, r, &[(0.1, 0.85), (0.3, 0.25), (0.55, 0.7), (0.9, 0.15)], CURVE, 2.0),
        "curve" => {
            let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (0.1 + 0.8 * t, 0.5 - 0.35 * (t * std::f32::consts::TAU).sin())
            }).collect();
            poly(p, r, &pts, CURVE, 2.0);
            for (x, y) in [(0.1, 0.5), (0.35, 0.1), (0.65, 0.9), (0.9, 0.5)] {
                p.rect_stroke(Rect::from_center_size(pt(r, x, y), Vec2::splat(4.0)), 0.0, Stroke::new(1.0_f32, GRAY), egui::StrokeKind::Inside);
            }
        }
        "interpcrv" => {
            let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (0.1 + 0.8 * t, 0.5 - 0.35 * (t * std::f32::consts::TAU).sin())
            }).collect();
            poly(p, r, &pts, CURVE, 2.0);
            for (x, y) in [(0.1, 0.5), (0.3, 0.17), (0.7, 0.83), (0.9, 0.5)] {
                p.circle_filled(pt(r, x, y), 2.5, RED);
            }
        }
        "circle" => circle(p, r, 0.5, 0.5, 0.38, CURVE, 2.0),
        "circle3pt" => {
            circle(p, r, 0.5, 0.5, 0.38, CURVE, 2.0);
            for (x, y) in [(0.5, 0.12), (0.17, 0.69), (0.83, 0.69)] {
                p.circle_filled(pt(r, x, y), 2.5, RED);
            }
        }
        "ellipse" => {
            let pts: Vec<(f32, f32)> = (0..=24).map(|i| {
                let a = i as f32 / 24.0 * std::f32::consts::TAU;
                (0.5 + 0.42 * a.cos(), 0.5 + 0.25 * a.sin())
            }).collect();
            poly(p, r, &pts, CURVE, 2.0);
        }
        "arc" => {
            let pts: Vec<(f32, f32)> = (0..=16).map(|i| {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                (0.5 + 0.4 * a.cos(), 0.75 + 0.5 * a.sin())
            }).collect();
            poly(p, r, &pts, CURVE, 2.0);
        }
        "rectangle" => { p.rect_stroke(Rect::from_min_max(pt(r, 0.12, 0.2), pt(r, 0.88, 0.8)), 0.0, Stroke::new(2.0_f32, CURVE), egui::StrokeKind::Inside); }
        "polygon" => {
            let pts: Vec<(f32, f32)> = (0..=6).map(|i| {
                let a = i as f32 / 6.0 * std::f32::consts::TAU;
                (0.5 + 0.42 * a.cos(), 0.5 + 0.42 * a.sin())
            }).collect();
            poly(p, r, &pts, CURVE, 2.0);
        }
        "fillet" => {
            poly(p, r, &[(0.15, 0.85), (0.15, 0.45)], CURVE, 2.0);
            poly(p, r, &[(0.55, 0.15), (0.85, 0.15)], CURVE, 2.0);
            let pts: Vec<(f32, f32)> = (0..=8).map(|i| {
                let a = std::f32::consts::PI * (1.0 + 0.5 * i as f32 / 8.0);
                (0.55 + 0.4 * a.cos(), 0.55 + 0.4 * a.sin())
            }).collect();
            poly(p, r, &pts, RED, 2.0);
        }
        "srfpt" | "plane" => fill(p, r, &[(0.15, 0.75), (0.45, 0.25), (0.9, 0.3), (0.6, 0.85)], SURF, CURVE),
        "filletsrf" => {
            fill(p, r, &[(0.1, 0.9), (0.1, 0.4), (0.5, 0.4), (0.5, 0.9)], SURF, CURVE);
            fill(p, r, &[(0.5, 0.4), (0.5, 0.1), (0.9, 0.1), (0.9, 0.4)], SURF, CURVE);
        }
        "box" => {
            fill(p, r, &[(0.15, 0.4), (0.55, 0.4), (0.55, 0.9), (0.15, 0.9)], SURF, CURVE);
            fill(p, r, &[(0.15, 0.4), (0.45, 0.15), (0.85, 0.15), (0.55, 0.4)], Color32::from_rgb(220, 230, 245), CURVE);
            fill(p, r, &[(0.55, 0.4), (0.85, 0.15), (0.85, 0.65), (0.55, 0.9)], Color32::from_rgb(150, 170, 200), CURVE);
        }
        "sphere" => {
            p.circle_filled(pt(r, 0.5, 0.5), 0.4 * r.width(), SURF);
            circle(p, r, 0.5, 0.5, 0.4, CURVE, 1.5);
            let pts: Vec<(f32, f32)> = (0..=16).map(|i| {
                let a = i as f32 / 16.0 * std::f32::consts::TAU;
                (0.5 + 0.4 * a.cos(), 0.5 + 0.14 * a.sin())
            }).collect();
            poly(p, r, &pts, GRAY, 1.0);
        }
        "cylinder" => {
            fill(p, r, &[(0.2, 0.25), (0.8, 0.25), (0.8, 0.8), (0.2, 0.8)], SURF, CURVE);
            let pts: Vec<(f32, f32)> = (0..=16).map(|i| {
                let a = i as f32 / 16.0 * std::f32::consts::TAU;
                (0.5 + 0.3 * a.cos(), 0.25 + 0.1 * a.sin())
            }).collect();
            poly(p, r, &pts, CURVE, 1.5);
        }
        "cone" => {
            fill(p, r, &[(0.5, 0.1), (0.85, 0.8), (0.15, 0.8)], SURF, CURVE);
        }
        "torus" => {
            circle(p, r, 0.5, 0.5, 0.42, CURVE, 1.5);
            circle(p, r, 0.5, 0.5, 0.18, CURVE, 1.5);
        }
        "booleanunion" => {
            p.circle_filled(pt(r, 0.38, 0.5), 0.28 * r.width(), SURF);
            p.circle_filled(pt(r, 0.62, 0.5), 0.28 * r.width(), SURF);
            circle(p, r, 0.38, 0.5, 0.28, CURVE, 1.5);
            circle(p, r, 0.62, 0.5, 0.28, CURVE, 1.5);
        }
        "project" => {
            fill(p, r, &[(0.1, 0.9), (0.4, 0.55), (0.9, 0.6), (0.6, 0.95)], SURF, CURVE);
            poly(p, r, &[(0.3, 0.15), (0.7, 0.2)], CURVE, 2.0);
            arrow(p, r, (0.5, 0.25), (0.5, 0.65), BLUE);
        }
        "mesh" => {
            poly(p, r, &[(0.1, 0.8), (0.5, 0.15), (0.9, 0.8), (0.1, 0.8), (0.5, 0.55), (0.9, 0.8)], CURVE, 1.5);
            poly(p, r, &[(0.5, 0.15), (0.5, 0.55)], CURVE, 1.5);
        }
        "join" => {
            poly(p, r, &[(0.1, 0.8), (0.45, 0.3)], CURVE, 2.0);
            poly(p, r, &[(0.55, 0.3), (0.9, 0.8)], CURVE, 2.0);
            arrow(p, r, (0.3, 0.1), (0.48, 0.28), RED);
        }
        "explode" => {
            poly(p, r, &[(0.1, 0.8), (0.4, 0.4)], CURVE, 2.0);
            poly(p, r, &[(0.6, 0.4), (0.9, 0.8)], CURVE, 2.0);
            arrow(p, r, (0.5, 0.3), (0.5, 0.05), RED);
        }
        "trim" => {
            poly(p, r, &[(0.1, 0.5), (0.9, 0.5)], CURVE, 2.0);
            poly(p, r, &[(0.5, 0.1), (0.5, 0.9)], RED, 1.5);
            poly(p, r, &[(0.6, 0.3), (0.9, 0.3)], GRAY, 1.0);
        }
        "split" => {
            poly(p, r, &[(0.1, 0.5), (0.45, 0.5)], CURVE, 2.0);
            poly(p, r, &[(0.55, 0.5), (0.9, 0.5)], CURVE, 2.0);
            poly(p, r, &[(0.5, 0.1), (0.5, 0.9)], RED, 1.5);
        }
        "group" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.0_f32, GRAY), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(pt(r, 0.2, 0.2), pt(r, 0.5, 0.5)), 0.0, BLUE);
            p.rect_filled(Rect::from_min_max(pt(r, 0.55, 0.55), pt(r, 0.8, 0.8)), 0.0, BLUE);
        }
        "ungroup" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.45, 0.45)), 0.0, BLUE);
            p.rect_filled(Rect::from_min_max(pt(r, 0.55, 0.55), pt(r, 0.9, 0.9)), 0.0, BLUE);
        }
        "editpton" | "pointson" => {
            let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (0.1 + 0.8 * t, 0.5 - 0.3 * (t * std::f32::consts::TAU).sin())
            }).collect();
            poly(p, r, &pts, CURVE, 1.5);
            for (x, y) in [(0.1, 0.5), (0.35, 0.15), (0.65, 0.85), (0.9, 0.5)] {
                p.rect_filled(Rect::from_center_size(pt(r, x, y), Vec2::splat(4.0)), 0.0, if name == "pointson" { BLUE } else { RED });
            }
        }
        "textobject" => label(p, r, "Ab", CURVE),
        "move" => {
            arrow(p, r, (0.5, 0.5), (0.5, 0.08), BLUE);
            arrow(p, r, (0.5, 0.5), (0.5, 0.92), BLUE);
            arrow(p, r, (0.5, 0.5), (0.08, 0.5), BLUE);
            arrow(p, r, (0.5, 0.5), (0.92, 0.5), BLUE);
        }
        "copy" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.6, 0.6)), 0.0, SURF);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.6, 0.6)), 0.0, Stroke::new(1.0_f32, CURVE), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(pt(r, 0.4, 0.4), pt(r, 0.9, 0.9)), 0.0, SURF);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.4, 0.4), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.0_f32, CURVE), egui::StrokeKind::Inside);
        }
        "rotate" => {
            let pts: Vec<(f32, f32)> = (0..=14).map(|i| {
                let a = -0.3 + 1.6 * std::f32::consts::PI * i as f32 / 14.0;
                (0.5 + 0.36 * a.cos(), 0.5 + 0.36 * a.sin())
            }).collect();
            poly(p, r, &pts, BLUE, 2.0);
            arrow(p, r, (0.5 + 0.36 * (1.3f32 * std::f32::consts::PI - 0.3).cos(), 0.5 + 0.36 * (1.3f32 * std::f32::consts::PI - 0.3).sin()), (0.5 + 0.36 * (1.3f32 * std::f32::consts::PI + 0.05).cos(), 0.5 + 0.36 * (1.3f32 * std::f32::consts::PI + 0.05).sin()), BLUE);
        }
        "scale" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.4), pt(r, 0.6, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.0_f32, GRAY), egui::StrokeKind::Inside);
            arrow(p, r, (0.6, 0.4), (0.9, 0.1), BLUE);
        }
        "dir" => {
            fill(p, r, &[(0.1, 0.9), (0.4, 0.5), (0.9, 0.55), (0.6, 0.95)], SURF, CURVE);
            arrow(p, r, (0.5, 0.7), (0.5, 0.15), RED);
        }
        "array" => {
            for (x, y) in [(0.15, 0.15), (0.55, 0.15), (0.15, 0.55), (0.55, 0.55)] {
                p.rect_filled(Rect::from_min_max(pt(r, x, y), pt(r, x + 0.3, y + 0.3)), 0.0, BLUE);
            }
        }
        "arraypolar" => {
            circle(p, r, 0.5, 0.5, 0.33, Color32::GRAY, 1.0);
            for i in 0..6 {
                let a = i as f32 / 6.0 * std::f32::consts::TAU;
                let (cx, cy) = (0.5 + 0.33 * a.cos(), 0.5 + 0.33 * a.sin());
                p.rect_filled(Rect::from_min_max(pt(r, cx - 0.1, cy - 0.1), pt(r, cx + 0.1, cy + 0.1)), 0.0, if i == 0 { RED } else { BLUE });
            }
            p.circle_filled(pt(r, 0.5, 0.5), 1.5, CURVE);
        }
        "arraylinear" => {
            for i in 0..4 {
                let x = 0.08 + i as f32 * 0.23;
                p.rect_filled(Rect::from_min_max(pt(r, x, 0.55), pt(r, x + 0.16, 0.8)), 0.0, if i == 0 { RED } else { BLUE });
            }
            arrow(p, r, (0.1, 0.3), (0.9, 0.3), CURVE);
        }
        "arraycrv" => {
            let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (0.08 + 0.84 * t, 0.75 - 0.5 * (t * std::f32::consts::PI).sin())
            }).collect();
            poly(p, r, &pts, CURVE, 1.5);
            for (k, i) in [0usize, 4, 8, 12].iter().enumerate() {
                let (x, y) = pts[*i];
                p.rect_filled(Rect::from_min_max(pt(r, x - 0.08, y - 0.08), pt(r, x + 0.08, y + 0.08)), 0.0, if k == 0 { RED } else { BLUE });
            }
        }
        "arraysrf" => {
            fill(p, r, &[(0.1, 0.9), (0.4, 0.45), (0.95, 0.5), (0.65, 0.95)], SURF, CURVE);
            for (x, y) in [(0.3, 0.7), (0.5, 0.72), (0.7, 0.75), (0.42, 0.57), (0.6, 0.59), (0.78, 0.62)] {
                p.rect_filled(Rect::from_min_max(pt(r, x - 0.05, y - 0.05), pt(r, x + 0.05, y + 0.05)), 0.0, BLUE);
            }
        }
        "arraycrvonsrf" => {
            fill(p, r, &[(0.1, 0.9), (0.4, 0.45), (0.95, 0.5), (0.65, 0.95)], SURF, CURVE);
            let pts: Vec<(f32, f32)> = (0..=10).map(|i| {
                let t = i as f32 / 10.0;
                (0.2 + 0.6 * t, 0.82 - 0.3 * t)
            }).collect();
            poly(p, r, &pts, RED, 1.5);
            for i in [0usize, 5, 10] {
                let (x, y) = pts[i];
                p.rect_filled(Rect::from_min_max(pt(r, x - 0.06, y - 0.06), pt(r, x + 0.06, y + 0.06)), 0.0, BLUE);
            }
        }
        "align" => {
            poly(p, r, &[(0.15, 0.1), (0.15, 0.9)], RED, 1.5);
            p.rect_filled(Rect::from_min_max(pt(r, 0.2, 0.2), pt(r, 0.6, 0.4)), 0.0, BLUE);
            p.rect_filled(Rect::from_min_max(pt(r, 0.2, 0.6), pt(r, 0.85, 0.8)), 0.0, BLUE);
        }
        "flowalongsrf" => {
            let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (0.1 + 0.8 * t, 0.7 - 0.3 * (t * std::f32::consts::PI).sin())
            }).collect();
            fill(p, r, &[(0.1, 0.9), (0.1, 0.7), (0.9, 0.7), (0.9, 0.9)], SURF, CURVE);
            poly(p, r, &pts, RED, 2.0);
        }
        "pushpull" => {
            fill(p, r, &[(0.15, 0.6), (0.55, 0.6), (0.55, 0.9), (0.15, 0.9)], SURF, CURVE);
            arrow(p, r, (0.35, 0.55), (0.35, 0.1), BLUE);
        }
        "check" => label(p, r, "✓", GREEN),
        "block" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(pt(r, 0.3, 0.3), pt(r, 0.7, 0.7)), 0.0, BLUE);
        }
        "history" => {
            circle(p, r, 0.5, 0.5, 0.4, CURVE, 1.5);
            poly(p, r, &[(0.5, 0.5), (0.5, 0.2)], CURVE, 1.5);
            poly(p, r, &[(0.5, 0.5), (0.7, 0.6)], CURVE, 1.5);
        }
        "new" => {
            fill(p, r, &[(0.2, 0.1), (0.65, 0.1), (0.8, 0.25), (0.8, 0.9), (0.2, 0.9)], Color32::WHITE, GRAY);
        }
        "open" => {
            fill(p, r, &[(0.1, 0.3), (0.4, 0.3), (0.5, 0.4), (0.9, 0.4), (0.9, 0.85), (0.1, 0.85)], YELLOW, GRAY);
        }
        "save" => {
            fill(p, r, &[(0.15, 0.15), (0.85, 0.15), (0.85, 0.85), (0.15, 0.85)], BLUE, CURVE);
            p.rect_filled(Rect::from_min_max(pt(r, 0.3, 0.15), pt(r, 0.7, 0.4)), 0.0, Color32::WHITE);
        }
        "import" => arrow(p, r, (0.2, 0.2), (0.75, 0.75), GREEN),
        "export" => arrow(p, r, (0.75, 0.75), (0.2, 0.2), GREEN),
        "print" => {
            fill(p, r, &[(0.1, 0.4), (0.9, 0.4), (0.9, 0.8), (0.1, 0.8)], GRAY, CURVE);
            p.rect_filled(Rect::from_min_max(pt(r, 0.3, 0.15), pt(r, 0.7, 0.4)), 0.0, Color32::WHITE);
        }
        "documentproperties" | "options" => {
            circle(p, r, 0.5, 0.5, 0.3, CURVE, 2.5);
            for i in 0..8 {
                let a = i as f32 / 8.0 * std::f32::consts::TAU;
                poly(p, r, &[(0.5 + 0.3 * a.cos(), 0.5 + 0.3 * a.sin()), (0.5 + 0.45 * a.cos(), 0.5 + 0.45 * a.sin())], CURVE, 2.5);
            }
        }
        "cut" => {
            poly(p, r, &[(0.2, 0.2), (0.8, 0.8)], CURVE, 1.5);
            poly(p, r, &[(0.8, 0.2), (0.2, 0.8)], CURVE, 1.5);
            circle(p, r, 0.3, 0.8, 0.12, CURVE, 1.5);
            circle(p, r, 0.7, 0.8, 0.12, CURVE, 1.5);
        }
        "copyclip" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.15, 0.1), pt(r, 0.6, 0.65)), 0.0, Color32::WHITE);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.15, 0.1), pt(r, 0.6, 0.65)), 0.0, Stroke::new(1.0_f32, CURVE), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(pt(r, 0.4, 0.35), pt(r, 0.85, 0.9)), 0.0, Color32::WHITE);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.4, 0.35), pt(r, 0.85, 0.9)), 0.0, Stroke::new(1.0_f32, CURVE), egui::StrokeKind::Inside);
        }
        "paste" => {
            fill(p, r, &[(0.2, 0.2), (0.8, 0.2), (0.8, 0.9), (0.2, 0.9)], YELLOW, CURVE);
            p.rect_filled(Rect::from_min_max(pt(r, 0.35, 0.1), pt(r, 0.65, 0.3)), 0.0, GRAY);
        }
        "undo" => arrow(p, r, (0.85, 0.6), (0.15, 0.6), BLUE),
        "redo" => arrow(p, r, (0.15, 0.6), (0.85, 0.6), BLUE),
        "pan" => {
            arrow(p, r, (0.5, 0.5), (0.5, 0.1), CURVE);
            arrow(p, r, (0.5, 0.5), (0.5, 0.9), CURVE);
            arrow(p, r, (0.5, 0.5), (0.1, 0.5), CURVE);
            arrow(p, r, (0.5, 0.5), (0.9, 0.5), CURVE);
        }
        "rotateview" => {
            circle(p, r, 0.5, 0.5, 0.3, CURVE, 1.5);
            arrow(p, r, (0.8, 0.5), (0.8, 0.2), BLUE);
        }
        "zoom" | "zoom_window" | "zoom_extents" | "zoom_selected" | "zoom_dynamic" => {
            circle(p, r, 0.42, 0.42, 0.28, CURVE, 2.0);
            poly(p, r, &[(0.62, 0.62), (0.9, 0.9)], CURVE, 3.0);
            match name {
                "zoom_extents" => { p.rect_stroke(Rect::from_min_max(pt(r, 0.25, 0.25), pt(r, 0.6, 0.6)), 0.0, Stroke::new(1.0_f32, BLUE), egui::StrokeKind::Inside); }
                "zoom_selected" => { p.rect_filled(Rect::from_min_max(pt(r, 0.3, 0.3), pt(r, 0.55, 0.55)), 0.0, YELLOW); }
                "zoom_window" => { p.rect_stroke(Rect::from_min_max(pt(r, 0.3, 0.3), pt(r, 0.55, 0.55)), 0.0, Stroke::new(1.0_f32, CURVE), egui::StrokeKind::Inside); }
                _ => {}
            }
        }
        "4view" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            poly(p, r, &[(0.5, 0.1), (0.5, 0.9)], CURVE, 1.5);
            poly(p, r, &[(0.1, 0.5), (0.9, 0.5)], CURVE, 1.5);
        }
        "setview" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            label(p, r, "R", BLUE);
        }
        "cplane" => {
            poly(p, r, &[(0.1, 0.8), (0.5, 0.2), (0.9, 0.4), (0.5, 1.0), (0.1, 0.8)], CURVE, 1.0);
            poly(p, r, &[(0.3, 0.5), (0.7, 0.7)], RED, 1.5);
            poly(p, r, &[(0.3, 0.9), (0.7, 0.3)], GREEN, 1.5);
        }
        "showosnap" => {
            poly(p, r, &[(0.1, 0.5), (0.9, 0.5)], CURVE, 1.5);
            p.rect_stroke(Rect::from_center_size(pt(r, 0.5, 0.5), Vec2::splat(9.0)), 0.0, Stroke::new(1.5_f32, RED), egui::StrokeKind::Inside);
        }
        "selall" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.1), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.0_f32, GRAY), egui::StrokeKind::Inside);
            fill(p, r, &[(0.3, 0.2), (0.3, 0.75), (0.45, 0.6), (0.6, 0.8), (0.68, 0.72), (0.52, 0.55), (0.7, 0.5)], YELLOW, CURVE);
        }
        "hide" | "show" => {
            let pts: Vec<(f32, f32)> = (0..=16).map(|i| {
                let a = i as f32 / 16.0 * std::f32::consts::TAU;
                (0.5 + 0.42 * a.cos(), 0.5 + 0.25 * a.sin())
            }).collect();
            poly(p, r, &pts, CURVE, 1.5);
            p.circle_filled(pt(r, 0.5, 0.5), 0.12 * r.width(), if name == "show" { BLUE } else { GRAY });
            if name == "hide" {
                poly(p, r, &[(0.15, 0.85), (0.85, 0.15)], RED, 2.0);
            }
        }
        "lock" | "unlock" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.25, 0.45), pt(r, 0.75, 0.9)), 1.0, YELLOW);
            let pts: Vec<(f32, f32)> = (0..=10).map(|i| {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 10.0);
                ((if name == "unlock" { 0.35 } else { 0.5 }) + 0.2 * a.cos(), 0.45 + 0.3 * a.sin())
            }).collect();
            poly(p, r, &pts, CURVE, 2.0);
        }
        "layer" => {
            for (i, c) in [RED, GREEN, BLUE].iter().enumerate() {
                let y = 0.2 + 0.25 * i as f32;
                fill(p, r, &[(0.1, y + 0.1), (0.5, y - 0.1), (0.9, y + 0.1), (0.5, y + 0.3)], *c, CURVE);
            }
        }
        "properties" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.15, 0.1), pt(r, 0.85, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            for i in 0..3 {
                let y = 0.3 + 0.2 * i as f32;
                poly(p, r, &[(0.3, y), (0.7, y)], GRAY, 1.5);
            }
        }
        "shade" | "viewport_shaded" => {
            p.circle_filled(pt(r, 0.5, 0.5), 0.38 * r.width(), SURF);
            circle(p, r, 0.5, 0.5, 0.38, CURVE, 1.5);
        }
        "viewport_wireframe" => {
            circle(p, r, 0.5, 0.5, 0.38, CURVE, 1.5);
            circle(p, r, 0.5, 0.5, 0.2, GRAY, 1.0);
        }
        "render" => {
            p.circle_filled(pt(r, 0.5, 0.5), 0.38 * r.width(), BLUE);
            p.circle_filled(pt(r, 0.38, 0.38), 0.1 * r.width(), Color32::WHITE);
        }
        "spotlight" => {
            fill(p, r, &[(0.5, 0.15), (0.85, 0.85), (0.15, 0.85)], YELLOW, CURVE);
        }
        "dim" => {
            poly(p, r, &[(0.1, 0.8), (0.9, 0.8)], CURVE, 1.0);
            arrow(p, r, (0.5, 0.5), (0.12, 0.5), CURVE);
            arrow(p, r, (0.5, 0.5), (0.88, 0.5), CURVE);
            poly(p, r, &[(0.1, 0.3), (0.1, 0.8)], GRAY, 1.0);
            poly(p, r, &[(0.9, 0.3), (0.9, 0.8)], GRAY, 1.0);
        }
        "grasshopper" => label(p, r, "GH", GREEN),
        "help" => label(p, r, "?", BLUE),
        "extrudecrv" => {
            fill(p, r, &[(0.15, 0.85), (0.15, 0.4), (0.85, 0.4), (0.85, 0.85)], SURF, CURVE);
            poly(p, r, &[(0.15, 0.85), (0.85, 0.85)], RED, 2.0);
            arrow(p, r, (0.5, 0.45), (0.5, 0.1), BLUE);
        }
        "revolve" => {
            let pts: Vec<(f32, f32)> = (0..=10).map(|i| {
                let t = i as f32 / 10.0;
                (0.55 + 0.25 * (t * std::f32::consts::PI).sin(), 0.15 + 0.7 * t)
            }).collect();
            poly(p, r, &pts, RED, 2.0);
            poly(p, r, &[(0.3, 0.1), (0.3, 0.9)], GRAY, 1.0);
        }
        "loft" => {
            for y in [0.2, 0.5, 0.8] {
                let pts: Vec<(f32, f32)> = (0..=16).map(|i| {
                    let a = i as f32 / 16.0 * std::f32::consts::TAU;
                    (0.5 + 0.35 * a.cos(), y + 0.08 * a.sin())
                }).collect();
                poly(p, r, &pts, RED, 1.5);
            }
            poly(p, r, &[(0.15, 0.2), (0.15, 0.8)], CURVE, 1.0);
            poly(p, r, &[(0.85, 0.2), (0.85, 0.8)], CURVE, 1.0);
        }
        "mirror" => {
            fill(p, r, &[(0.1, 0.85), (0.4, 0.15), (0.4, 0.85)], SURF, CURVE);
            fill(p, r, &[(0.9, 0.85), (0.6, 0.15), (0.6, 0.85)], Color32::from_rgb(220, 230, 245), GRAY);
            poly(p, r, &[(0.5, 0.05), (0.5, 0.95)], RED, 1.0);
        }
        "delete" => {
            poly(p, r, &[(0.2, 0.2), (0.8, 0.8)], RED, 2.5);
            poly(p, r, &[(0.8, 0.2), (0.2, 0.8)], RED, 2.5);
        }
        "distance" => {
            arrow(p, r, (0.5, 0.5), (0.1, 0.5), CURVE);
            arrow(p, r, (0.5, 0.5), (0.9, 0.5), CURVE);
        }
        "offset" => {
            circle(p, r, 0.5, 0.5, 0.38, CURVE, 1.5);
            circle(p, r, 0.5, 0.5, 0.22, RED, 1.5);
        }
        "extend" => {
            poly(p, r, &[(0.1, 0.5), (0.55, 0.5)], CURVE, 2.0);
            poly(p, r, &[(0.55, 0.5), (0.9, 0.5)], RED, 2.0);
        }
        "rebuild" => {
            let pts: Vec<(f32, f32)> = (0..=12).map(|i| {
                let t = i as f32 / 12.0;
                (0.1 + 0.8 * t, 0.5 - 0.3 * (t * std::f32::consts::TAU).sin())
            }).collect();
            poly(p, r, &pts, CURVE, 1.5);
            for i in 0..5 {
                p.circle_filled(pt(r, 0.1 + 0.2 * i as f32, 0.5), 2.0, BLUE);
            }
        }
        "booleandifference" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.1, 0.3), pt(r, 0.6, 0.9)), 0.0, SURF);
            circle(p, r, 0.65, 0.35, 0.28, CURVE, 1.5);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.3), pt(r, 0.6, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
        }
        "booleanintersection" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.3), pt(r, 0.6, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            circle(p, r, 0.65, 0.35, 0.28, CURVE, 1.5);
            p.rect_filled(Rect::from_min_max(pt(r, 0.42, 0.3), pt(r, 0.6, 0.55)), 0.0, YELLOW);
        }
        "booleansplit" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.3), pt(r, 0.6, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            circle(p, r, 0.65, 0.35, 0.28, CURVE, 1.5);
            poly(p, r, &[(0.35, 0.05), (0.35, 0.95)], RED, 1.5);
        }
        "makehole" | "roundhole" | "movehole" | "arrayhole" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.1, 0.2), pt(r, 0.9, 0.85)), 0.0, SURF);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.2), pt(r, 0.9, 0.85)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            if name == "makehole" {
                p.rect_filled(Rect::from_min_max(pt(r, 0.35, 0.4), pt(r, 0.65, 0.65)), 0.0, Color32::WHITE);
                p.rect_stroke(Rect::from_min_max(pt(r, 0.35, 0.4), pt(r, 0.65, 0.65)), 0.0, Stroke::new(1.0_f32, CURVE), egui::StrokeKind::Inside);
            } else {
                circle(p, r, 0.5, 0.52, 0.16, CURVE, 1.5);
                p.circle_filled(pt(r, 0.5, 0.52), (r.width() * 0.16).max(1.0), Color32::WHITE);
                p.circle_stroke(pt(r, 0.5, 0.52), (r.width() * 0.16).max(1.0), Stroke::new(1.5_f32, CURVE));
            }
            if name == "movehole" {
                arrow(p, r, (0.5, 0.52), (0.82, 0.52), BLUE);
            }
            if name == "arrayhole" {
                circle(p, r, 0.25, 0.52, 0.1, CURVE, 1.0);
                circle(p, r, 0.75, 0.52, 0.1, CURVE, 1.0);
            }
        }
        "filletedge" | "chamferedge" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.15, 0.3), pt(r, 0.85, 0.9)), 0.0, SURF);
            if name == "filletedge" {
                poly(p, r, &[(0.15, 0.9), (0.15, 0.5), (0.2, 0.38), (0.3, 0.3), (0.85, 0.3)], CURVE, 1.5);
            } else {
                poly(p, r, &[(0.15, 0.9), (0.15, 0.5), (0.35, 0.3), (0.85, 0.3)], CURVE, 1.5);
            }
            poly(p, r, &[(0.85, 0.3), (0.85, 0.9), (0.15, 0.9)], CURVE, 1.5);
        }
        "shell" => {
            p.rect_stroke(Rect::from_min_max(pt(r, 0.15, 0.2), pt(r, 0.85, 0.9)), 0.0, Stroke::new(2.5_f32, CURVE), egui::StrokeKind::Inside);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.3, 0.35), pt(r, 0.7, 0.9)), 0.0, Stroke::new(1.0_f32, GRAY), egui::StrokeKind::Inside);
        }
        "wirecut" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.15, 0.3), pt(r, 0.85, 0.9)), 0.0, SURF);
            poly(p, r, &[(0.3, 0.1), (0.5, 0.95)], RED, 1.5);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.15, 0.3), pt(r, 0.85, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
        }
        "boss" | "slab" | "extrudesrf" => {
            p.rect_filled(Rect::from_min_max(pt(r, 0.1, 0.55), pt(r, 0.9, 0.9)), 0.0, SURF);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.1, 0.55), pt(r, 0.9, 0.9)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(pt(r, 0.35, 0.15), pt(r, 0.65, 0.55)), 0.0, YELLOW);
            p.rect_stroke(Rect::from_min_max(pt(r, 0.35, 0.15), pt(r, 0.65, 0.55)), 0.0, Stroke::new(1.5_f32, CURVE), egui::StrokeKind::Inside);
        }
        "pyramid" => {
            poly(p, r, &[(0.1, 0.85), (0.5, 0.15), (0.9, 0.85), (0.1, 0.85)], CURVE, 1.5);
            poly(p, r, &[(0.5, 0.15), (0.6, 0.9), (0.9, 0.85)], CURVE, 1.0);
        }
        "pipe" => {
            poly(p, r, &[(0.1, 0.4), (0.5, 0.3), (0.9, 0.5)], CURVE, 4.0);
            poly(p, r, &[(0.1, 0.4), (0.5, 0.3), (0.9, 0.5)], SURF, 2.0);
        }
        _ => label(p, r, &name.chars().take(3).collect::<String>(), CURVE),
    }
}
