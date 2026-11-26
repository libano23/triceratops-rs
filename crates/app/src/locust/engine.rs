//! The evaluation engine: run a graph and get data trees out.
//!
//! Inputs are matched branch by branch; each combination calls the component and the
//! results go back under the path they came from. An input that carries a single branch is
//! broadcast against the others, and inside a branch the longest list runs the combination
//! while the shorter ones repeat their last item — so one number can drive a list of
//! points. Tree components are handed the whole trees instead, because their meaning is
//! the structure.
//!
//! A disabled node passes its inputs through, a cluster solves its nested graph with the
//! outer inputs seeded into the Cluster Input proxies, and a component that fails (returns
//! nothing or panics) yields nothing rather than stopping the solve.

use std::collections::{BTreeMap, HashMap};
use std::panic::{catch_unwind, AssertUnwindSafe};

use super::components::{registry, Args, Component, Ctx, Func, Out, Registry};
use super::data::{cast_tree, match_trees, to_tree, DataTree, Value};
use super::graph::{Graph, Node};

/// Every node's outputs, keyed by node uid.
pub type Outputs = HashMap<String, Vec<DataTree>>;

fn raw_input_tree(graph: &Graph, node: &Node, index: usize, outputs: &Outputs) -> DataTree {
    if let Some(conn) = graph.source_of(&node.uid, index) {
        return outputs.get(&conn.src).and_then(|o| o.get(conn.src_port)).cloned().unwrap_or_default();
    }
    if let Some(v) = node.values.get(&index.to_string()) {
        return to_tree(&Value::from_json(v));
    }
    DataTree::empty()
}

fn input_tree(graph: &Graph, node: &Node, index: usize, comp: &Component, outputs: &Outputs) -> DataTree {
    let spec = &comp.inputs[index];
    let tree = raw_input_tree(graph, node, index, outputs);
    if tree.is_empty() {
        if let Some(v) = node.values.get(spec.name) {
            return to_tree(&Value::from_json(v));
        }
        if let Some(d) = &spec.default {
            return to_tree(d);
        }
    }
    tree
}

/// Run `f` and turn a panic in geometry code into "no result", noted as an error on the node.
fn guarded<T>(ctx_errors: &mut Vec<String>, f: impl FnOnce() -> Option<T>) -> Option<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "component panicked".into());
            ctx_errors.push(msg);
            None
        }
    }
}

fn element_outputs(comp: &Component, f: super::components::ElemFn, ctx: &mut Ctx, trees: &[DataTree]) -> Vec<DataTree> {
    // optional inputs that carry nothing are passed through as Null
    let active: Vec<usize> = (0..trees.len()).filter(|&i| !(comp.inputs[i].optional && trees[i].is_empty())).collect();
    let cast: Vec<DataTree> = active.iter().map(|&i| cast_tree(&trees[i], comp.inputs[i].kind)).collect();
    let combos = match_trees(&cast);
    let ran = !combos.is_empty();
    let mut buckets: Vec<BTreeMap<Vec<i64>, Vec<Value>>> = vec![BTreeMap::new(); comp.outputs.len()];
    let mut panics = Vec::new();
    for combo in combos {
        let mut args = vec![Value::Null; trees.len()];
        for (k, &i) in active.iter().enumerate() {
            args[i] = combo.values[k].clone();
        }
        let res = guarded(&mut panics, || f(ctx, &Args(&args)));
        let Some(vals) = res else { continue };
        for (k, v) in vals.into_iter().enumerate().take(comp.outputs.len()) {
            match v {
                Out::Skip => {}
                Out::One(x) => buckets[k].entry(combo.path.clone()).or_default().push(x),
                Out::Many(xs) => buckets[k].entry(combo.path.clone()).or_default().extend(xs),
            }
        }
    }
    for p in panics {
        ctx.error(p);
    }
    if ran && !comp.outputs.is_empty() && buckets.iter().all(|b| b.is_empty()) && !ctx.diagnostics.get(ctx.current.as_deref().unwrap_or("")).is_some_and(|d| d.iter().any(|x| x.level == super::components::Level::Error)) {
        ctx.warn("no result");
    }
    buckets.into_iter().map(|b| DataTree { branches: b }).collect()
}

fn tree_outputs(comp: &Component, f: super::components::TreeFn, ctx: &mut Ctx, trees: &[DataTree]) -> Vec<DataTree> {
    let mut panics = Vec::new();
    let mut out = guarded(&mut panics, || Some(f(ctx, trees))).unwrap_or_default();
    for p in panics {
        ctx.error(p);
    }
    out.resize(comp.outputs.len(), DataTree::empty());
    out
}

/// A disabled node: each output takes the input in its place.
fn bypass(comp: &Component, trees: &[DataTree]) -> Vec<DataTree> {
    (0..comp.outputs.len()).map(|i| trees.get(i).cloned().unwrap_or_default()).collect()
}

fn solve_cluster(graph: &Graph, node: &Node, ctx: &mut Ctx, outputs: &Outputs, reg: &Registry) -> Vec<DataTree> {
    let Some(def) = graph.cluster_of(node) else { return Vec::new() };
    let inner = Graph::from_json(def.get("graph").unwrap_or(&serde_json::Value::Null));
    let ports = |key: &str| def.get(key).and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let mut seeds: HashMap<String, Vec<DataTree>> = HashMap::new();
    for (i, d) in ports("inputs").iter().enumerate() {
        if let Some(uid) = d.get("node").and_then(|x| x.as_str()) {
            seeds.insert(uid.to_string(), vec![raw_input_tree(graph, node, i, outputs)]);
        }
    }
    let outer = ctx.current.clone();
    let inner_out = solve_with(&inner, ctx, &seeds, reg);
    // what the nodes inside said is shown on the cluster node
    ctx.current = outer.clone();
    if let Some(uid) = outer {
        for n in &inner.nodes {
            if let Some(list) = ctx.diagnostics.remove(&n.uid) {
                let moved: Vec<super::components::Diagnostic> = list.into_iter().map(|d| super::components::Diagnostic { level: d.level, message: format!("{}: {}", n.label(), d.message) }).collect();
                ctx.diagnostics.entry(uid.clone()).or_default().extend(moved);
            }
        }
    }
    ports("outputs")
        .iter()
        .map(|d| d.get("node").and_then(|x| x.as_str()).and_then(|uid| inner_out.get(uid)).and_then(|t| t.first()).cloned().unwrap_or_default())
        .collect()
}

/// A Script or Expression node.
fn solve_dynamic(graph: &Graph, node: &Node, ctx: &mut Ctx, outputs: &Outputs) -> Vec<DataTree> {
    super::script::solve_node(graph, node, ctx, outputs)
}

/// Every node's outputs as data trees, keyed by node uid.
pub fn solve(graph: &Graph, ctx: &mut Ctx) -> Outputs {
    solve_with(graph, ctx, &HashMap::new(), registry())
}

/// `solve` with pre-loaded outputs for some nodes (used for clusters) and a registry.
pub fn solve_with(graph: &Graph, ctx: &mut Ctx, seeds: &HashMap<String, Vec<DataTree>>, reg: &Registry) -> Outputs {
    let mut outputs: Outputs = HashMap::new();
    for uid in graph.topological() {
        let Some(node) = graph.node(&uid) else { continue };
        ctx.current = Some(uid.clone());
        if let Some(s) = seeds.get(&uid) {
            outputs.insert(uid, s.clone());
            continue;
        }
        if graph.is_cluster(node) {
            let o = solve_cluster(graph, node, ctx, &outputs, reg);
            outputs.insert(uid, o);
            continue;
        }
        if super::graph::is_dynamic(&node.kind) {
            let o = solve_dynamic(graph, node, ctx, &outputs);
            outputs.insert(uid, o);
            continue;
        }
        let Some(comp) = reg.get(&node.kind) else {
            ctx.error(format!("unknown component {}", node.kind));
            outputs.insert(uid, Vec::new());
            continue;
        };
        let trees: Vec<DataTree> = (0..comp.inputs.len()).map(|i| input_tree(graph, node, i, comp, &outputs)).collect();
        let o = if node.disabled {
            bypass(comp, &trees)
        } else {
            match comp.func {
                Func::Tree(f) => tree_outputs(comp, f, ctx, &trees),
                Func::Element(f) => element_outputs(comp, f, ctx, &trees),
            }
        };
        outputs.insert(uid, o);
    }
    outputs
}

/// What a single input port is receiving, for the canvas tooltip.
pub fn input_preview(graph: &Graph, node_uid: &str, port: usize, outputs: &Outputs) -> DataTree {
    let Some(node) = graph.node(node_uid) else { return DataTree::empty() };
    let Some(comp) = registry().get(&node.kind) else { return DataTree::empty() };
    if port >= comp.inputs.len() {
        return DataTree::empty();
    }
    input_tree(graph, node, port, comp, outputs)
}
