//! Expression and Script nodes.

use std::time::{Duration, Instant};

use serde_json::{json, Map};

use super::super::components::{Ctx, Level};
use super::super::data::{as_number, DataTree};
use super::super::engine::solve;
use super::super::graph::Graph;

fn vals(pairs: &[(&str, serde_json::Value)]) -> Map<String, serde_json::Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn node(g: &mut Graph, kind: &str, pairs: &[(&str, serde_json::Value)]) -> String {
    g.add_node_with(kind, 0.0, 0.0, vals(pairs))
}

fn nums(t: &DataTree) -> Vec<f64> {
    t.items().iter().map(as_number).collect()
}

fn errors(ctx: &Ctx, uid: &str) -> Vec<String> {
    ctx.diagnostics.get(uid).map(|d| d.iter().filter(|x| x.level == Level::Error).map(|x| x.message.clone()).collect()).unwrap_or_default()
}

/// Solve and return the outputs of `uid` and the context.
fn run(g: &Graph, uid: &str) -> (Vec<DataTree>, Ctx) {
    let mut ctx = Ctx::default();
    let mut out = solve(g, &mut ctx);
    (out.remove(uid).unwrap(), ctx)
}

#[test]
fn expression_maps_over_list() {
    let mut g = Graph::new();
    let series = node(&mut g, "Series", &[("Start", json!(0)), ("Step", json!(1)), ("Count", json!(5))]);
    let e = node(&mut g, "Expression", &[("Expression", json!("x*x"))]);
    g.connect(&series, 0, &e, 0);
    let (out, ctx) = run(&g, &e);
    assert!(errors(&ctx, &e).is_empty(), "{:?}", ctx.diagnostics);
    assert_eq!(nums(&out[0]), [0.0, 1.0, 4.0, 9.0, 16.0]);
}

#[test]
fn expression_radians() {
    let mut g = Graph::new();
    let e = node(&mut g, "Expression", &[("Expression", json!("sin(x) + cos(0) + deg(PI)")), ("x", json!(std::f64::consts::FRAC_PI_2))]);
    let (out, ctx) = run(&g, &e);
    assert!(errors(&ctx, &e).is_empty(), "{:?}", ctx.diagnostics);
    assert!((nums(&out[0])[0] - 182.0).abs() < 1e-9, "{:?}", nums(&out[0]));
}

#[test]
fn expression_two_vars_broadcast() {
    let mut g = Graph::new();
    let series = node(&mut g, "Series", &[("Start", json!(1)), ("Step", json!(1)), ("Count", json!(3))]);
    let e = node(&mut g, "Expression", &[("Expression", json!("x + y")), ("y", json!(10))]);
    g.connect(&series, 0, &e, 0);
    let (out, _) = run(&g, &e);
    assert_eq!(nums(&out[0]), [11.0, 12.0, 13.0]);
}

#[test]
fn expression_unknown_variable() {
    let mut g = Graph::new();
    let e = node(&mut g, "Expression", &[("Expression", json!("sin(xx)")), ("x", json!(1.0))]);
    let (out, ctx) = run(&g, &e);
    assert!(out[0].is_empty());
    let errs = errors(&ctx, &e);
    assert_eq!(errs.len(), 1, "said once: {errs:?}");
    assert!(errs[0].contains("xx") && errs[0].contains("column"), "{errs:?}");
}

#[test]
fn script_infinite_loop_is_cut() {
    let mut g = Graph::new();
    let s = node(&mut g, "Script", &[("code", json!("loop { }"))]);
    let t = Instant::now();
    let (_, ctx) = run(&g, &s);
    assert!(t.elapsed() < Duration::from_secs(60));
    assert!(errors(&ctx, &s).iter().any(|e| e.contains("operations")), "{:?}", ctx.diagnostics);
}

#[test]
fn script_stops_when_cancelled() {
    let mut g = Graph::new();
    // real work per turn (an empty loop reaches the operation limit within 0.1 s)
    let s = node(&mut g, "Script", &[("code", json!("let i = 0; loop { i += 1; }"))]);
    let token = tri_geom::job::Token::new();
    let _guard = tri_geom::job::install(token.clone());
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        token.cancel();
        Instant::now()
    });
    let (_, ctx) = run(&g, &s);
    let done = Instant::now();
    let cancelled_at = canceller.join().unwrap();
    assert!(errors(&ctx, &s).iter().any(|e| e.contains("cancelled")), "{:?}", ctx.diagnostics);
    assert!(done.duration_since(cancelled_at) < Duration::from_millis(500), "{:?}", done.duration_since(cancelled_at));
}

