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
    // Script and Expression solve in `locust::script` (their ports are the node's own); these
    // entries put them in the catalogue
    r.elem("Expression", "Script", vec![], &["Result"], |_, _| None).describe(
        "A formula of the node's inputs in Rhai syntax, e.g. sin(x)*cos(y), pow(x, 2) + y, if x > 0 { x } else { 0 }. \
         The formula is values.Expression. Inputs default to x, y, z (values._inputs to change them); the output is Result. \
         Lists are matched like any component's. Trigonometry in radians; PI, E, deg(), rad(), pow, atan2, min, max, clamp.",
    );
    r.elem("Script", "Script", vec![], &["a"], |_, _| None).describe(
        "Rhai code with its own inputs and outputs: values.code is the code, values._inputs and values._outputs list the \
         ports as names or {name, kind, access: item|list|tree} (default inputs x, y; output a). Inputs are variables; \
         whatever each output variable holds at the end goes out, an array as a list. Item inputs run the code once per \
         matched item; list inputs get the whole branch as an array; tree inputs an array of branches. \
         Helpers: point(x, y, z), vec(x, y, z) (with .x .y .z, + - * /, dot, cross, length, unit, distance), rng(seed).next(), \
         line, polyline(pts[, closed]), interpolate(pts[, periodic]), nurbs_curve(pts, degree), circle(c, r[, normal]), \
         arc(c, r, angle), helix(start, end, r, turns[, end_radius]), divide(c, n), offset(c, d), join(curves), curve_point(c, t 0..1), curve_length(c), loft(curves), \
         extrude(g, dir), revolve(curve, axis_point, axis, angle), sweep1(rail, profile), cap(g), surface_from_grid(rows), \
         surface_point(s, u, v), box(corner, x, y, z), cylinder(base, r, h), sphere(c, r), union, difference, intersection, \
         move(g, v), rotate(g, degrees, center, axis), scale(g, center, f), mirror(g, origin, normal), \
         array_polar(g, center, count, degrees), area(g), volume(g); run(\"Component\", [args]) calls any component \
         (run_all gives every output). print() shows on the node. Maths in radians: sin, cos, pow, atan2, PI, E, deg, rad.",
    );
    r.elem("Geometry", "Params", vec![any("Objects")], &["Geometry"], |ctx, a| {
        let refs: Vec<Value> = match a.get(0) {
            Value::Null => return None,
            Value::List(items) => items.clone(),
            v => vec![v.clone()],
        };
        let shapes: Vec<Value> = refs.iter().filter_map(|r| ctx.object(r)).map(Value::Geometry).collect();
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
