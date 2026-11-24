//! Params: numbers, points, planes, colours and references to document objects.

use glam::DVec3;

use super::super::data::Value;
use super::*;

pub fn register(r: &mut Registry) {
    r.elem("Number", "Params", vec![num("Value", 0.0).slider(-10.0, 10.0)], &["Number"], |_, a| one(n(a.num(0)))).describe("A number, with a slider");
    r.elem("Integer", "Params", vec![int("Value", 0).slider(0.0, 10.0)], &["Integer"], |_, a| one(Value::Int(a.int(0))));
    r.elem("Boolean", "Params", vec![boolean("Value", true)], &["Boolean"], |_, a| one(Value::Bool(a.flag(0))));
    r.elem("Text", "Params", vec![text("Value", "text")], &["Text"], |_, a| one(Value::Text(a.text(0))));
    r.elem("Point", "Params", vec![num("X", 0.0), num("Y", 0.0), num("Z", 0.0)], &["Point"], |_, a| one(p(DVec3::new(a.num(0), a.num(1), a.num(2)))));
    r.elem("Vector", "Params", vec![num("X", 0.0), num("Y", 0.0), num("Z", 1.0)], &["Vector"], |_, a| one(p(DVec3::new(a.num(0), a.num(1), a.num(2)))));
    r.elem("Plane", "Params", vec![pt("Origin", 0.0, 0.0, 0.0), pt("Normal", 0.0, 0.0, 1.0)], &["Plane"], |_, a| one(Value::Plane(a.pt(0), a.pt_or(1, DVec3::Z))));
    r.elem("Geometry", "Params", vec![any("Objects")], &["Geometry"], |ctx, a| {
        let doc = ctx.doc?;
        let refs: Vec<Value> = match a.get(0) {
            Value::Null => return None,
            Value::List(items) => items.clone(),
            v => vec![v.clone()],
        };
        let shapes: Vec<Value> = refs.iter().filter_map(|r| geo::find_object(doc, r)).map(Value::geometry).collect();
        match shapes.len() {
            0 => None,
            1 => one(shapes.into_iter().next().unwrap()),
            _ => many(shapes),
        }
    })
    .describe("Reference document objects by id or name (Pick from selection)");
    r.elem("Colour RGB", "Params", vec![num("Red", 1.0), num("Green", 0.0), num("Blue", 0.0), num("Alpha", 1.0)], &["Colour"], |_, a| {
        let c = |i: usize| a.num(i).clamp(0.0, 1.0);
        one(Value::Colour([c(0), c(1), c(2), c(3)]))
    });
    r.elem("Colour HSV", "Params", vec![num("Hue", 0.0), num("Saturation", 1.0), num("Value", 1.0)], &["Colour"], |_, a| {
        let (r_, g_, b_) = hsv_to_rgb(a.num(0).rem_euclid(360.0) / 360.0, a.num(1).clamp(0.0, 1.0), a.num(2).clamp(0.0, 1.0));
        one(Value::Colour([r_, g_, b_, 1.0]))
    });
}

/// Python `colorsys.hsv_to_rgb`.
pub fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (v, v, v);
    }
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (pp, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match (i as i64).rem_euclid(6) {
        0 => (v, t, pp),
        1 => (q, v, pp),
        2 => (pp, v, t),
        3 => (pp, q, v),
        4 => (t, pp, v),
        _ => (v, pp, q),
    }
}
