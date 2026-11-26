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
        if let Some((v, _)) = Document::volume_of(g) {
            out["volume"] = json!(v);
        } else if let Some(a) = Document::area_of(g) {
            out["area"] = json!(a);
        }
    }
    out
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
    let mut results = Vec::new();
    for c in &graph.connections {
        let Some(dst) = graph.node(&c.dst).filter(|n| SINKS.contains(&n.kind.as_str()) && c.dst_port == 0) else { continue };
        let Some(src) = graph.node(&c.src) else { continue };
        let tree = outputs.get(&c.src).and_then(|o| o.get(c.src_port)).cloned().unwrap_or_default();
        let port = graph.node_ports(src).1.get(c.src_port).map(|p| p.0.clone()).unwrap_or_default();
        let mut r = summary(&tree);
        r["label"] = json!(src.label());
        r["port"] = json!(port);
        r["to"] = json!(dst.kind);
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
