//! Output: preview in the viewport, bake into the document, panels and cluster ports.

use super::super::data::{as_number, DataTree, Value};
use super::*;

fn colour_of(v: Option<&Value>) -> [f64; 4] {
    match v {
        Some(Value::Colour(c)) => *c,
        Some(Value::Point(q)) => [q.x, q.y, q.z, 1.0],
        Some(Value::List(l)) if l.len() >= 3 => [as_number(&l[0]), as_number(&l[1]), as_number(&l[2]), l.get(3).map(as_number).unwrap_or(1.0)],
        _ => [0.8, 0.2, 0.2, 1.0],
    }
}

pub fn register(r: &mut Registry) {
    r.tree("Preview", "Output", vec![geo("Geometry"), boolean("Show", true)], &["Geometry"], |ctx, t| {
        if first_bool(&t[1], true) {
            ctx.preview.extend(t[0].items().iter().filter_map(displayable).map(|x| (x, None)));
        }
        t1(t[0].clone())
    })
    .describe("Show the geometry in the viewport without baking it");
    r.tree("Bake", "Output", vec![geo("Geometry")], &["Geometry"], |ctx, t| {
        ctx.bake.extend(t[0].items().iter().filter_map(displayable));
        t1(t[0].clone())
    })
    .describe("Put the geometry into the document as real objects");
    r.tree("Custom Preview", "Output", vec![geo("Geometry"), any_d("Colour", Value::Colour([0.8, 0.2, 0.2, 1.0])), boolean("Show", true)], &["Geometry"], |ctx, t| {
        if first_bool(&t[2], true) {
            let col = colour_of(t[1].first());
            ctx.preview.extend(t[0].items().iter().filter_map(displayable).map(|x| (x, Some(col))));
        }
        t1(t[0].clone())
    })
    .describe("Preview the geometry in a colour you choose");
    r.tree("Panel", "Output", vec![any("Data")], &["Data"], |_, t| t1(t[0].clone())).describe("Show the data flowing through, unchanged");
    r.tree("Cluster Input", "Output", vec![], &["Output"], |_, _| t1(DataTree::empty())).describe("The outer data a cluster receives");
    r.tree("Cluster Output", "Output", vec![tree("Input")], &["Output"], |_, t| t1(t[0].clone())).describe("An exposed result of a cluster");
}
