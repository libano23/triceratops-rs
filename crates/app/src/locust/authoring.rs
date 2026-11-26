//! Definitions written by an assistant: the file format, plus names where the file has ids
//! and indices, and no need for positions. `prepare` checks everything before anything
//! replaces the canvas, and says exactly what is wrong.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value as J};

use super::components::registry;
use super::graph::{is_dynamic, new_uid, Graph, Node};

/// Horizontal and vertical spacing of the automatic layout.
const COLUMN: f64 = 260.0;
const ROW: f64 = 140.0;

/// Turn an assistant's definition into a graph, or say what is wrong with it.
///
/// Accepted on top of the file format: nodes without `uid` (the nickname, or the type when
/// it is unique, is the handle), connections written `{"from": "<node>[.<output>]", "to":
/// "<node>.<input>"}` with nodes by uid or nickname and ports by name or index, group
/// members by handle, and nodes without `x`/`y` (laid out left to right by depth).
pub fn prepare(input: &J) -> Result<Graph, String> {
    let clusters = input.get("clusters").and_then(|c| c.as_object()).cloned().unwrap_or_default();
    let mut nodes: Vec<Map<String, J>> = Vec::new();
    for n in input.get("nodes").and_then(|n| n.as_array()).ok_or("the definition needs \"nodes\": an array")? {
        let mut n = n.as_object().cloned().ok_or("every node must be an object")?;
        let kind = n.get("type").and_then(|t| t.as_str()).ok_or("every node needs a \"type\"")?.to_string();
        let known = registry().get(&kind).is_some() || is_dynamic(&kind) || kind.strip_prefix("Cluster:").is_some_and(|id| clusters.contains_key(id));
        if !known {
            return Err(format!("unknown component \"{kind}\" (did you mean: {}?)", suggestions(&kind).join(", ")));
        }
        if n.get("uid").and_then(|u| u.as_str()).is_none_or(str::is_empty) {
            n.insert("uid".into(), J::String(new_uid()));
        }
        nodes.push(n);
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for n in &nodes {
        *seen.entry(n["uid"].as_str().unwrap_or("").to_string()).or_default() += 1;
    }
    if let Some((uid, k)) = seen.iter().find(|(_, k)| **k > 1) {
        return Err(format!("uid \"{uid}\" is used by {k} nodes"));
    }
    // a graph of just the nodes, to ask for ports
    let shell = Graph::from_json(&json!({ "nodes": nodes, "clusters": clusters }));
    let handles = Handles::new(&shell);
    let mut wires: Vec<(String, usize, String, usize)> = Vec::new();
    for c in input.get("connections").and_then(|c| c.as_array()).cloned().unwrap_or_default() {
        let wire = if let (Some(from), Some(to)) = (c.get("from").and_then(|v| v.as_str()), c.get("to").and_then(|v| v.as_str())) {
            let (src, out) = handles.port(&shell, from, true)?;
            let (dst, inp) = handles.port(&shell, to, false)?;
            (src, out, dst, inp)
        } else if c.get("from").is_some() || c.get("to").is_some() || c.get("src").is_none() || c.get("dst").is_none() {
            return Err("a connection needs \"from\" and \"to\" (or src, src_port, dst, dst_port)".into());
        } else {
            let src = handles.node(c.get("src").and_then(|v| v.as_str()).unwrap_or(""))?;
            let dst = handles.node(c.get("dst").and_then(|v| v.as_str()).unwrap_or(""))?;
            let port = |k: &str| c.get(k).and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            (src, port("src_port"), dst, port("dst_port"))
        };
        if wires.iter().any(|w| w.2 == wire.2 && w.3 == wire.3) {
            let n = shell.node(&wire.2).expect("resolved");
            let name = shell.node_ports(n).0.get(wire.3).map(|p| p.0.clone()).unwrap_or_else(|| wire.3.to_string());
            return Err(format!("input \"{}.{name}\" is wired twice", n.label()));
        }
        wires.push(wire);
    }
    if let Some(cycle) = cycle(&shell, &wires) {
        return Err(format!("the graph has a cycle through: {}", cycle.join(", ")));
    }
    layout(&mut nodes, &wires);
    let connections: Vec<J> = wires.iter().map(|(s, sp, d, dp)| json!({ "src": s, "src_port": sp, "dst": d, "dst_port": dp })).collect();
    // comments without a place go under the nodes, one after another
    let mut comments = input.get("comments").and_then(|c| c.as_array()).cloned().unwrap_or_default();
    let mut below = nodes.iter().filter_map(|n| n.get("y").and_then(|y| y.as_f64())).fold(0.0f64, f64::max) + 220.0;
    for c in comments.iter_mut() {
        if c.get("x").and_then(|v| v.as_f64()).is_none() || c.get("y").and_then(|v| v.as_f64()).is_none() {
            c["x"] = json!(40.0);
            c["y"] = json!(below);
            below += 100.0;
        }
    }
    let mut groups = input.get("groups").and_then(|g| g.as_array()).cloned().unwrap_or_default();
    for g in groups.iter_mut() {
        if let Some(members) = g.get("nodes").and_then(|m| m.as_array()).cloned() {
            let uids: Result<Vec<J>, String> = members.iter().map(|m| handles.node(m.as_str().unwrap_or("")).map(J::String)).collect();
            g["nodes"] = J::Array(uids?);
        }
    }
    Ok(Graph::from_json(&json!({
        "nodes": nodes,
        "connections": connections,
        "groups": groups,
        "comments": comments,
        "clusters": clusters,
    })))
}

/// An input named `node.input` (or `node` alone: its first input) → (uid, input index).
pub fn resolve_input(g: &Graph, text: &str) -> Result<(String, usize), String> {
    let h = Handles::new(g);
    match h.lookup(text) {
        Some(uid) => Ok((uid?, 0)),
        None => h.port(g, text, false),
    }
}

/// How connections may name nodes: uid, nickname, or type when only one node has it.
struct Handles {
    by_uid: HashSet<String>,
    by_name: HashMap<String, Vec<String>>,
}

impl Handles {
    fn new(g: &Graph) -> Self {
        let mut by_name: HashMap<String, Vec<String>> = HashMap::new();
        for n in &g.nodes {
            by_name.entry(n.label().to_string()).or_default().push(n.uid.clone());
            if n.nickname.is_some() {
                // the type still names it when no other node has that type
                by_name.entry(format!("\u{0}{}", n.kind)).or_default().push(n.uid.clone());
            }
        }
        Self { by_uid: g.nodes.iter().map(|n| n.uid.clone()).collect(), by_name }
    }

    fn lookup(&self, h: &str) -> Option<Result<String, String>> {
        if self.by_uid.contains(h) {
            return Some(Ok(h.to_string()));
        }
        let hit = self.by_name.get(h).or_else(|| self.by_name.get(&format!("\u{0}{h}")))?;
        Some(match hit.as_slice() {
            [one] => Ok(one.clone()),
            many => Err(format!("nickname \"{h}\" is used by {} nodes", many.len())),
        })
    }

    fn node(&self, h: &str) -> Result<String, String> {
        self.lookup(h).unwrap_or_else(|| Err(format!("unknown node \"{h}\"")))
    }

    /// `node.port` (or `node` alone: output 0, or the only input) → (uid, port index).
    fn port(&self, g: &Graph, text: &str, out: bool) -> Result<(String, usize), String> {
        let (uid, port) = match self.lookup(text) {
            Some(r) => (r?, None),
            None => {
                // the node part may itself contain dots: take the first split that names a node
                let mut found = None;
                for (i, _) in text.match_indices('.') {
                    if let Some(r) = self.lookup(&text[..i]) {
                        found = Some((r?, Some(&text[i + 1..])));
                        break;
                    }
                }
                found.ok_or_else(|| format!("unknown node \"{}\"", text.split('.').next().unwrap_or(text)))?
            }
        };
        let n = g.node(&uid).expect("handle resolves to a node");
        let (ins, outs) = g.node_ports(n);
        let names: Vec<String> = if out { outs } else { ins }.into_iter().map(|p| p.0).collect();
        let side = if out { "output" } else { "input" };
        let list = || format!("({side}s: {})", names.join(", "));
        let index = match port {
            None if out => 0,
            None if names.len() == 1 => 0,
            None => return Err(format!("node \"{}\" has several inputs; name one {}", n.label(), list())),
            Some(p) => match p.parse::<usize>() {
                Ok(i) if i < names.len() => i,
                _ => names.iter().position(|x| x == p).or_else(|| names.iter().position(|x| x.eq_ignore_ascii_case(p))).ok_or_else(|| format!("node \"{}\" has no {side} \"{p}\" {}", n.label(), list()))?,
            },
        };
        if out && names.is_empty() {
            return Err(format!("node \"{}\" has no outputs", n.label()));
        }
        Ok((uid, index))
    }
}

/// The three registry names closest to `kind`.
fn suggestions(kind: &str) -> Vec<String> {
    let k = kind.to_lowercase();
    let mut all: Vec<(usize, &str)> = registry().components.keys().copied().map(|n| (edit_distance(&k, &n.to_lowercase()), n)).collect();
    all.sort();
    all.into_iter().take(3).map(|(_, n)| n.to_string()).collect()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Labels of the nodes left over when sources are peeled off one by one (a cycle), if any.
fn cycle(g: &Graph, wires: &[(String, usize, String, usize)]) -> Option<Vec<String>> {
    let mut indeg: HashMap<&str, usize> = g.nodes.iter().map(|n| (n.uid.as_str(), 0)).collect();
    for w in wires {
        *indeg.get_mut(w.2.as_str())? += 1;
    }
    let mut ready: Vec<&str> = indeg.iter().filter(|(_, d)| **d == 0).map(|(u, _)| *u).collect();
    while let Some(u) = ready.pop() {
        indeg.remove(u);
        for w in wires.iter().filter(|w| w.0 == u) {
            if let Some(d) = indeg.get_mut(w.2.as_str()) {
                *d -= 1;
                if *d == 0 {
                    ready.push(w.2.as_str());
                }
            }
        }
    }
    if indeg.is_empty() {
        return None;
    }
    let mut labels: Vec<String> = g.nodes.iter().filter(|n| indeg.contains_key(n.uid.as_str())).map(|n: &Node| n.label().to_string()).collect();
    labels.sort();
    Some(labels)
}

/// Place nodes without a position: column by longest path from a source, rows in order.
fn layout(nodes: &mut [Map<String, J>], wires: &[(String, usize, String, usize)]) {
    let uid = |n: &Map<String, J>| n["uid"].as_str().unwrap_or("").to_string();
    let mut depth: HashMap<String, usize> = nodes.iter().map(|n| (uid(n), 0)).collect();
    // the graph is acyclic here, so relaxing n times settles every depth
    for _ in 0..nodes.len() {
        let mut changed = false;
        for w in wires {
            let d = depth.get(&w.0).copied().unwrap_or(0) + 1;
            if depth.get(&w.2).is_some_and(|&x| x < d) {
                depth.insert(w.2.clone(), d);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut rows: HashMap<usize, usize> = HashMap::new();
    for n in nodes.iter_mut() {
        if n.get("x").and_then(|v| v.as_f64()).is_some() && n.get("y").and_then(|v| v.as_f64()).is_some() {
            continue;
        }
        let d = depth.get(&uid(n)).copied().unwrap_or(0);
        let row = rows.entry(d).or_insert(0);
        n.insert("x".into(), json!(40.0 + COLUMN * d as f64));
        n.insert("y".into(), json!(40.0 + ROW * *row as f64));
        *row += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn err(v: J) -> String {
        prepare(&v).expect_err("expected an error")
    }

    #[test]
    fn prepare_resolves_names_and_layout() {
        let g = prepare(&json!({
            "nodes": [
                { "type": "Number", "nickname": "Radius", "values": { "Value": 6 } },
                { "type": "Circle" },
                { "type": "Extrude", "values": { "Distance": 12 } },
                { "type": "Preview", "x": 900, "y": 10 }
            ],
            "connections": [
                { "from": "Radius", "to": "Circle.Radius" },
                { "from": "Circle", "to": "Extrude.Curve" },
                { "from": "Extrude.Extrusion", "to": "Preview.Geometry" }
            ]
        }))
        .unwrap();
        let by_kind = |k: &str| g.nodes.iter().find(|n| n.kind == k).unwrap();
        let (radius, circle, extrude, preview) = (by_kind("Number"), by_kind("Circle"), by_kind("Extrude"), by_kind("Preview"));
        assert!(g.connections.iter().any(|c| c.src == radius.uid && c.src_port == 0 && c.dst == circle.uid && c.dst_port == 1));
        assert!(g.connections.iter().any(|c| c.src == circle.uid && c.dst == extrude.uid && c.dst_port == 0));
        assert!(g.connections.iter().any(|c| c.src == extrude.uid && c.dst == preview.uid && c.dst_port == 0));
        assert!(circle.x > radius.x && extrude.x > circle.x, "laid out left to right");
        assert_eq!((preview.x, preview.y), (900.0, 10.0), "given positions are kept");
        assert_eq!(radius.values["Value"], json!(6));
    }

    #[test]
    fn file_form_still_works() {
        let g = prepare(&json!({
            "nodes": [{ "uid": "a", "type": "Number" }, { "uid": "b", "type": "Circle" }],
            "connections": [{ "src": "a", "src_port": 0, "dst": "b", "dst_port": 1 }]
        }))
        .unwrap();
        assert_eq!(g.connections.len(), 1);
    }

    #[test]
    fn unknown_component_suggests() {
        let e = err(json!({ "nodes": [{ "type": "Extrud" }] }));
        assert!(e.starts_with("unknown component \"Extrud\" (did you mean: "), "{e}");
        assert!(e.contains("Extrude"), "{e}");
    }

    #[test]
    fn unknown_node() {
        let e = err(json!({ "nodes": [{ "type": "Circle" }], "connections": [{ "from": "Nope", "to": "Circle.Radius" }] }));
        assert_eq!(e, "unknown node \"Nope\"");
    }

    #[test]
    fn ambiguous_nickname() {
        let e = err(json!({
            "nodes": [{ "type": "Number", "nickname": "R" }, { "type": "Number", "nickname": "R" }, { "type": "Circle" }],
            "connections": [{ "from": "R", "to": "Circle.Radius" }]
        }));
        assert_eq!(e, "nickname \"R\" is used by 2 nodes");
    }

    #[test]
    fn missing_ports() {
        let e = err(json!({ "nodes": [{ "type": "Number" }, { "type": "Circle" }], "connections": [{ "from": "Number", "to": "Circle.Size" }] }));
        assert_eq!(e, "node \"Circle\" has no input \"Size\" (inputs: Center, Radius, Normal)");
        let e = err(json!({ "nodes": [{ "type": "Number" }, { "type": "Circle" }], "connections": [{ "from": "Number.Out", "to": "Circle.Radius" }] }));
        assert_eq!(e, "node \"Number\" has no output \"Out\" (outputs: Number)");
    }

    #[test]
    fn cycle_is_refused() {
        let e = err(json!({
            "nodes": [{ "type": "Add", "nickname": "A" }, { "type": "Add", "nickname": "B" }],
            "connections": [{ "from": "A", "to": "B.A" }, { "from": "B", "to": "A.A" }]
        }));
        assert!(e.starts_with("the graph has a cycle through: "), "{e}");
    }

    #[test]
    fn double_wire_is_refused() {
        let e = err(json!({
            "nodes": [{ "type": "Number", "nickname": "P" }, { "type": "Number", "nickname": "Q" }, { "type": "Circle" }],
            "connections": [{ "from": "P", "to": "Circle.Radius" }, { "from": "Q", "to": "Circle.Radius" }]
        }));
        assert_eq!(e, "input \"Circle.Radius\" is wired twice");
    }

    #[test]
    fn script_ports_by_name() {
        let g = prepare(&json!({
            "nodes": [{ "type": "Number", "nickname": "N" }, { "type": "Script", "nickname": "S", "values": { "_inputs": ["count"], "_outputs": ["pts"], "code": "pts = count;" } }, { "type": "Panel" }],
            "connections": [{ "from": "N", "to": "S.count" }, { "from": "S.pts", "to": "Panel.Data" }]
        }))
        .unwrap();
        assert_eq!(g.connections.len(), 2);
    }

    #[test]
    fn duplicate_uids_are_refused() {
        let e = err(json!({ "nodes": [{ "uid": "a", "type": "Number" }, { "uid": "a", "type": "Circle" }] }));
        assert_eq!(e, "uid \"a\" is used by 2 nodes");
    }

    #[test]
    fn half_a_connection_says_what_is_missing() {
        let e = err(json!({ "nodes": [{ "type": "Number" }], "connections": [{ "from": "Number" }] }));
        assert_eq!(e, "a connection needs \"from\" and \"to\" (or src, src_port, dst, dst_port)");
    }
}
