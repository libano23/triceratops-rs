//! Script and Expression nodes, on one shared Rhai engine.
//!
//! An Expression is a formula of its inputs; a Script is Rhai code whose inputs are
//! variables and whose outputs are whatever the output variables hold at the end. Both are
//! matched like element components (or take whole lists or trees, per input), compiled once
//! per source text, sandboxed (Rhai has no file, process or network access here, and
//! `import` is compiled out) and bounded: an operation limit stops runaway loops and the
//! solve's cancel token (Esc) stops them sooner.

pub mod api;
pub mod convert;

use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};

use rhai::{Dynamic, Engine, EvalAltResult, ParseError, Position, Scope, AST, FLOAT, INT};

use super::components::{Ctx, Out};
use super::data::{match_trees, to_tree, DataTree, Value};
use super::engine::Outputs;
use super::graph::{Access, Graph, Node};

/// Operations one evaluation may take (a runaway loop ends with an error, not a hang).
const MAX_OPERATIONS: u64 = 50_000_000;

/// The shared engine with the Locust types, maths helpers and limits.
pub fn engine() -> &'static Engine {
    static ENGINE: OnceLock<Engine> = OnceLock::new();
    ENGINE.get_or_init(build)
}

fn build() -> Engine {
    let mut e = Engine::new();
    e.set_max_operations(MAX_OPERATIONS)
        .set_max_call_levels(64)
        .set_max_expr_depths(128, 64)
        .set_max_string_size(1 << 20)
        .set_max_array_size(1_000_000)
        .set_max_map_size(100_000);
    // Esc / a newer solve: stop at the next check
    e.on_progress(|ops| (ops % 256 == 0 && tri_geom::job::check().is_err()).then(|| Dynamic::from("cancelled")));
    e.register_fn("deg", |r: FLOAT| r.to_degrees()).register_fn("rad", |d: FLOAT| d.to_radians());
    e.register_fn("deg", |r: INT| (r as FLOAT).to_degrees()).register_fn("rad", |d: INT| (d as FLOAT).to_radians());
    // maths on integers too, so sin(1) works like sin(1.0)
    for (name, f) in [
        ("sin", f64::sin as fn(f64) -> f64),
        ("cos", f64::cos),
        ("tan", f64::tan),
        ("asin", f64::asin),
        ("acos", f64::acos),
        ("atan", f64::atan),
        ("sqrt", f64::sqrt),
        ("exp", f64::exp),
        ("ln", f64::ln),
    ] {
        e.register_fn(name, move |x: INT| f(x as FLOAT));
    }
    e.register_fn("pow", |a: FLOAT, b: FLOAT| a.powf(b))
        .register_fn("pow", |a: FLOAT, b: INT| a.powi(b as i32))
        .register_fn("pow", |a: INT, b: FLOAT| (a as FLOAT).powf(b))
        .register_fn("pow", |a: INT, b: INT| (a as FLOAT).powi(b as i32))
        .register_fn("atan2", |y: FLOAT, x: FLOAT| y.atan2(x))
        .register_fn("hypot", |a: FLOAT, b: FLOAT| a.hypot(b))
        .register_fn("min", |a: FLOAT, b: FLOAT| a.min(b))
        .register_fn("max", |a: FLOAT, b: FLOAT| a.max(b))
        .register_fn("clamp", |x: FLOAT, lo: FLOAT, hi: FLOAT| x.clamp(lo.min(hi), hi.max(lo)));
    convert::register_types(&mut e);
    api::register(&mut e);
    e
}

/// The shorthands every script can call, compiled once.
fn prelude() -> &'static AST {
    static PRELUDE: OnceLock<AST> = OnceLock::new();
    PRELUDE.get_or_init(|| engine().compile(api::PRELUDE).expect("the script prelude compiles"))
}

/// Compiled sources, by kind and text.
fn compiled(expression: bool, src: &str) -> Result<Arc<AST>, String> {
    static CACHE: OnceLock<Mutex<HashMap<u64, Arc<AST>>>> = OnceLock::new();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (expression, src).hash(&mut h);
    let key = h.finish();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(ast) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok(ast.clone());
    }
    let ast = if expression { engine().compile_expression(src).map_err(parse_error)? } else { prelude().merge(&engine().compile(src).map_err(parse_error)?) };
    let ast = Arc::new(ast);
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    if c.len() > 512 {
        c.clear();
    }
    c.insert(key, ast.clone());
    Ok(ast)
}

fn at(pos: Position) -> String {
    match (pos.line(), pos.position()) {
        (Some(l), Some(c)) => format!("line {l}, column {c}: "),
        (Some(l), None) => format!("line {l}: "),
        _ => String::new(),
    }
}

fn parse_error(e: ParseError) -> String {
    format!("{}{}", at(e.1), e.0)
}

fn eval_error(mut e: Box<EvalAltResult>) -> String {
    if matches!(*e, EvalAltResult::ErrorTerminated(..)) {
        return "cancelled".into();
    }
    let pos = e.take_position();
    format!("{}{e}", at(pos))
}

/// The tree an input of a Script/Expression node receives.
fn input_tree(graph: &Graph, node: &Node, index: usize, name: &str, outputs: &Outputs) -> DataTree {
    if let Some(c) = graph.source_of(&node.uid, index) {
        return outputs.get(&c.src).and_then(|o| o.get(c.src_port)).cloned().unwrap_or_default();
    }
    node.values.get(name).or_else(|| node.values.get(&index.to_string())).map(|v| to_tree(&Value::from_json(v))).unwrap_or_default()
}

/// Apply an input's access: a List input gets each branch as one item, a Tree input the
/// whole tree as one list of branches.
fn with_access(tree: DataTree, access: Access) -> DataTree {
    match access {
        Access::Item => tree,
        Access::List => DataTree::from_branches(tree.branches.into_iter().map(|(p, items)| (p, vec![Value::List(items)]))),
        Access::Tree => DataTree::single(Value::List(tree.branches.into_values().map(Value::List).collect())),
    }
}

/// Solve a Script or Expression node: one output tree per output port.
pub fn solve_node(graph: &Graph, node: &Node, ctx: &mut Ctx, outputs: &Outputs) -> Vec<DataTree> {
    let (ins, outs) = graph.dynamic_ports(node);
    let empty = || vec![DataTree::empty(); outs.len()];
    let expression = node.kind == "Expression";
    let src = node.values.get(if expression { "Expression" } else { "code" }).and_then(|v| v.as_str()).unwrap_or(if expression { "x" } else { "" }).to_string();
    if src.trim().is_empty() {
        return empty();
    }
    let ast = match compiled(expression, &src) {
        Ok(a) => a,
        Err(e) => {
            ctx.error(e);
            return empty();
        }
    };
    // inputs that carry nothing are left out of the matching and seen as ()
    let trees: Vec<(usize, DataTree)> = ins.iter().enumerate().map(|(i, d)| (i, with_access(input_tree(graph, node, i, &d.name, outputs), d.access))).filter(|(_, t)| !t.is_empty()).collect();
    let combos = if trees.is_empty() {
        vec![super::data::Match { path: vec![0], values: Vec::new() }]
    } else {
        match_trees(&trees.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>())
    };
    let mut buckets: Vec<BTreeMap<Vec<i64>, Vec<Value>>> = vec![BTreeMap::new(); outs.len()];
    api::set_objects(ctx.objects.clone());
    api::take_printed();
    for combo in combos {
        let mut scope = Scope::new();
        scope.push_constant("PI", std::f64::consts::PI as FLOAT).push_constant("E", std::f64::consts::E as FLOAT);
        let mut args: Vec<Dynamic> = vec![Dynamic::UNIT; ins.len()];
        for (k, (i, _)) in trees.iter().enumerate() {
            args[*i] = convert::to_dynamic(&combo.values[k]);
        }
        for (d, v) in ins.iter().zip(args) {
            scope.push_dynamic(d.name.as_str(), v);
        }
        let results: Vec<Dynamic> = if expression {
            match engine().eval_ast_with_scope::<Dynamic>(&mut scope, &ast) {
                Ok(v) => vec![v],
                Err(e) => {
                    ctx.error(eval_error(e));
                    break;
                }
            }
        } else {
            // outputs exist before the code runs, so `a = …` needs no `let`
            for d in &outs {
                if !scope.contains(&d.name) {
                    scope.push_dynamic(d.name.as_str(), Dynamic::UNIT);
                }
            }
            if let Err(e) = engine().run_ast_with_scope(&mut scope, &ast) {
                ctx.error(eval_error(e));
                break;
            }
            outs.iter().map(|d| scope.get_value::<Dynamic>(&d.name).unwrap_or(Dynamic::UNIT)).collect()
        };
        for (k, r) in results.into_iter().enumerate().take(outs.len()) {
            let out = if r.is_array() {
                match r.cast::<rhai::Array>().into_iter().map(convert::from_dynamic).collect::<Result<Vec<_>, _>>() {
                    Ok(items) => Out::Many(items),
                    Err(e) => {
                        ctx.error(format!("output `{}`: {e}", outs[k].name));
                        Out::Skip
                    }
                }
            } else {
                match convert::from_dynamic(r) {
                    Ok(Value::Null) => Out::Skip,
                    Ok(v) => Out::One(v),
                    Err(e) => {
                        ctx.error(format!("output `{}`: {e}", outs[k].name));
                        Out::Skip
                    }
                }
            };
            match out {
                Out::Skip => {}
                Out::One(x) => buckets[k].entry(combo.path.clone()).or_default().push(x),
                Out::Many(xs) => buckets[k].entry(combo.path.clone()).or_default().extend(xs),
            }
        }
    }
    for line in api::take_printed() {
        ctx.print(line);
    }
    buckets.into_iter().map(|b| DataTree { branches: b }).collect()
}

#[cfg(test)]
mod tests;
