//! A worked example definition, so a fresh canvas solves on the first look.
//!
//! Radius and Height sliders drive a Circle and an Extrude; the solid is previewed in the
//! viewport, wired to Bake, and its volume is shown on a Panel.

use serde_json::{json, Map};

use super::graph::Graph;

fn values(pairs: &[(&str, serde_json::Value)]) -> Map<String, serde_json::Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

pub fn build_example() -> Graph {
    let mut g = Graph::new();
    let radius = g.add_node_with("Number", 20.0, 40.0, values(&[("Value", json!(6.0))]));
    g.node_mut(&radius).unwrap().nickname = Some("Radius".into());
    let height = g.add_node_with("Number", 20.0, 200.0, values(&[("Value", json!(12.0))]));
    g.node_mut(&height).unwrap().nickname = Some("Height".into());
    let circle = g.add_node("Circle", 320.0, 40.0);
    let extrude = g.add_node("Extrude", 560.0, 90.0);
    let preview = g.add_node("Preview", 820.0, 20.0);
    let bake = g.add_node("Bake", 820.0, 200.0);
    let volume = g.add_node("Volume", 820.0, 360.0);
    let panel = g.add_node("Panel", 1080.0, 360.0);
    g.connect(&radius, 0, &circle, 1);
    g.connect(&circle, 0, &extrude, 0);
    g.connect(&height, 0, &extrude, 2);
    g.connect(&extrude, 0, &preview, 0);
    g.connect(&extrude, 0, &bake, 0);
    g.connect(&extrude, 0, &volume, 0);
    g.connect(&volume, 0, &panel, 0);
    g.add_group(&[radius, height], "Parameters");
    g.add_comment(20.0, 300.0, "Drag a slider and the solid in the viewport follows.\nPreview shows it; Bake writes it into the document.");
    g
}
