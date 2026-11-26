//! What a solve produced, as an assistant reads it: problems by node, what the panels show,
//! and a summary of everything that goes to the viewport or to Bake.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value as J};
use tri_doc::{Document, Geometry};
use tri_geom::BoundingBox;

use super::components::{Diagnostic, Level};
use super::data::{DataTree, Value};
use super::engine::{input_preview, Outputs};
use super::graph::Graph;

/// Kinds that send geometry to the viewport or the document.
const SINKS: [&str; 3] = ["Preview", "Custom Preview", "Bake"];

fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Number(_) => "number",
        Value::Text(_) => "text",
        Value::Point(_) => "point",
        Value::Pair(..) => "domain",
        Value::Colour(_) => "colour",
        Value::Plane(..) => "plane",
        Value::List(_) => "list",
        Value::Geometry(g) => match g.as_ref() {
            Geometry::Point(_) => "point",
            Geometry::Curve(_) => "curve",
            Geometry::Surface(_) => "surface",
            Geometry::Brep(_) => "brep",
            Geometry::Mesh(_) => "mesh",
            _ => "geometry",
        },
    }
}

/// Count, kinds, bounding box and (for the first item) volume or area of a tree.
fn summary(tree: &DataTree) -> J {
    let items = tree.items();
    let mut kinds: HashMap<&str, usize> = HashMap::new();
    let mut bb = BoundingBox::EMPTY;
    for v in &items {
        *kinds.entry(kind_name(v)).or_default() += 1;
        match v {
            Value::Geometry(g) => bb = bb.union(&g.bounding_box()),
            Value::Point(p) => bb = bb.union(&BoundingBox { min: *p, max: *p }),
            _ => {}
        }
    }
    let mut out = json!({ "items": items.len(), "branches": tree.branch_count(), "kinds": kinds });
    if bb.min.x <= bb.max.x {
        out["bbox"] = json!([[bb.min.x, bb.min.y, bb.min.z], [bb.max.x, bb.max.y, bb.max.z]]);
    }
    if let Some(Value::Geometry(g)) = items.first() {
        match measure(g) {
            Some(Measure::Volume(v)) => out["volume"] = json!(v),
            Some(Measure::Area(a)) => out["area"] = json!(a),
            None => {}
        }
    }
    out
}

enum Measure {
    Volume(f64),
    Area(f64),
}

/// Volume of a closed solid or area of a surface, from the display mesh: as accurate as what
/// is drawn and quick even for a screw of hundreds of sections (`Document::volume_of`
/// meshes every span 16×16).
fn measure(g: &Geometry) -> Option<Measure> {
    let d = tri_geom::display_mesh::MeshDensity::for_size(g.bounding_box().diagonal().max(1e-9));
    let (mesh, closed) = match g {
        Geometry::Brep(b) => (b.faces.iter().fold(tri_geom::Mesh::new(), |mut m, f| {
            m.append(&f.display_mesh(&d));
            m
        }), b.closed),
        Geometry::Surface(s) => (s.display_mesh(&d), false),
        Geometry::Mesh(m) => (m.clone(), m.is_closed()),
        Geometry::Curve(_) => return Document::area_of(g).map(Measure::Area),
        _ => return None,
    };
    if closed {
        let v: f64 = (0..mesh.triangle_count()).map(|t| {
            let (a, b, c) = mesh.triangle(t);
            a.dot(b.cross(c)) / 6.0
        }).sum();
        return Some(Measure::Volume(v.abs()));
    }
    let a: f64 = (0..mesh.triangle_count()).map(|t| {
        let (a, b, c) = mesh.triangle(t);
        (b - a).cross(c - a).length() / 2.0
    }).sum();
    (a > 0.0).then_some(Measure::Area(a))
}

/// The report of a solve of `graph`.
pub fn solve_report(graph: &Graph, outputs: &Outputs, diags: &HashMap<String, Vec<Diagnostic>>, preview: usize, bake: usize, elapsed: Duration) -> J {
    let level = |l: Level| match l {
        Level::Error => "error",
        Level::Warning => "warning",
        Level::Info => "print",
    };
    let mut problems = Vec::new();
    for n in &graph.nodes {
        for d in diags.get(&n.uid).into_iter().flatten() {
            problems.push(json!({ "uid": n.uid, "label": n.label(), "type": n.kind, "level": level(d.level), "message": d.message }));
        }
    }
    let count = |l: Level| diags.values().flatten().filter(|d| d.level == l).count();
    let panels: Vec<J> = graph.nodes.iter().filter(|n| n.kind == "Panel").map(|n| json!({ "label": n.label(), "text": input_preview(graph, &n.uid, 0, outputs).format(6, 5) })).collect();
    // one entry per output that goes to the viewport or Bake, saying where it goes
    let mut sources: Vec<((String, usize), Vec<String>)> = Vec::new();
    for c in &graph.connections {
        let Some(dst) = graph.node(&c.dst).filter(|n| SINKS.contains(&n.kind.as_str()) && c.dst_port == 0) else { continue };
        let key = (c.src.clone(), c.src_port);
        match sources.iter_mut().find(|(k, _)| *k == key) {
            Some((_, to)) => to.push(dst.kind.clone()),
            None => sources.push((key, vec![dst.kind.clone()])),
        }
    }
    let mut results = Vec::new();
    for ((src_uid, port_index), to) in sources {
        let Some(src) = graph.node(&src_uid) else { continue };
        let tree = outputs.get(&src_uid).and_then(|o| o.get(port_index)).cloned().unwrap_or_default();
        let port = graph.node_ports(src).1.get(port_index).map(|p| p.0.clone()).unwrap_or_default();
        let mut r = summary(&tree);
        r["label"] = json!(src.label());
        r["port"] = json!(port);
        r["to"] = json!(to);
        results.push(r);
    }
    json!({
        "nodes": graph.nodes.len(),
        "errors": count(Level::Error),
        "warnings": count(Level::Warning),
        "preview": preview,
        "bake": bake,
        "solve_ms": elapsed.as_millis() as u64,
        "problems": problems,
        "panels": panels,
        "results": results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locust::components::Ctx;
    use crate::locust::engine::solve;
    use serde_json::json;

    #[test]
    fn results_once_per_output_with_fast_measures() {
        let g = crate::locust::authoring::prepare(&json!({
            "nodes": [{ "type": "Box", "nickname": "B" }, { "type": "Preview" }, { "type": "Bake" }],
            "connections": [{ "from": "B", "to": "Preview.Geometry" }, { "from": "B", "to": "Bake.Geometry" }]
        }))
        .unwrap();
        let mut ctx = Ctx::default();
        let out = solve(&g, &mut ctx);
        let r = solve_report(&g, &out, &ctx.diagnostics, ctx.preview.len(), ctx.bake.len(), Duration::ZERO);
        let results = r["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "{r}");
        assert_eq!(results[0]["to"], json!(["Preview", "Bake"]));
        assert!((results[0]["volume"].as_f64().unwrap() - 1000.0).abs() < 1.0, "{r}");
    }

    #[test]
    fn comments_without_a_place_go_under_the_nodes() {
        let g = crate::locust::authoring::prepare(&json!({
            "nodes": [{ "type": "Number" }, { "type": "Number" }, { "type": "Number" }],
            "comments": [{ "text": "hello" }]
        }))
        .unwrap();
        let bottom = g.nodes.iter().map(|n| n.y).fold(f64::MIN, f64::max);
        assert!(g.comments[0].y > bottom + 100.0, "comment at {} under nodes down to {bottom}", g.comments[0].y);
    }
}
