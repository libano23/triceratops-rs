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


// ---------------------------------------------------------------- the Script API

fn script(g: &mut Graph, code: &str, ins: serde_json::Value, outs: serde_json::Value, extra: &[(&str, serde_json::Value)]) -> String {
    let mut pairs = vec![("code", json!(code)), ("_inputs", ins), ("_outputs", outs)];
    pairs.extend(extra.iter().cloned());
    node(g, "Script", &pairs)
}

#[test]
fn script_points_on_circle() {
    let mut g = Graph::new();
    let s = script(&mut g, "a = []; for i in 0..n { a.push(point(r*cos(i*2*PI/n), r*sin(i*2*PI/n), 0)) }", json!(["n", "r"]), json!(["a"]), &[("n", json!(8)), ("r", json!(10.0))]);
    let (out, ctx) = run(&g, &s);
    assert!(errors(&ctx, &s).is_empty(), "{:?}", ctx.diagnostics);
    let pts: Vec<glam::DVec3> = out[0].items().iter().map(super::super::data::as_point).collect();
    assert_eq!(pts.len(), 8);
    assert!(pts.iter().all(|p| (p.length() - 10.0).abs() < 1e-9));
}

#[test]
fn script_run_extrude_volume() {
    let mut g = Graph::new();
    let s = script(&mut g, "a = extrude(circle(point(0,0,0), 5), vec(0,0,10)); v = volume(a);", json!([]), json!(["a", "v"]), &[]);
    let (out, ctx) = run(&g, &s);
    assert!(errors(&ctx, &s).is_empty(), "{:?}", ctx.diagnostics);
    assert!(out[0].first().and_then(|v| v.as_geometry()).is_some());
    let v = nums(&out[1])[0];
    assert!((v - 785.398).abs() < 8.0, "volume {v}");
}

#[test]
fn script_bad_output_type() {
    let mut g = Graph::new();
    let s = script(&mut g, "a = #{k: 1}; b = 1;", json!([]), json!(["a", "b"]), &[]);
    let (out, ctx) = run(&g, &s);
    assert!(errors(&ctx, &s).iter().any(|e| e.starts_with("output `a`: expected number, point, geometry or array")), "{:?}", ctx.diagnostics);
    assert_eq!(nums(&out[1]), [1.0], "the other output still comes out");
}

#[test]
fn script_list_access() {
    let mut g = Graph::new();
    let series = node(&mut g, "Series", &[("Count", json!(7))]);
    let s = script(&mut g, "a = pts.len();", json!([{ "name": "pts", "access": "list" }]), json!(["a"]), &[]);
    g.connect(&series, 0, &s, 0);
    let (out, _) = run(&g, &s);
    assert_eq!(nums(&out[0]), [7.0], "one run with the whole branch");
}

#[test]
fn script_cannot_import() {
    let mut g = Graph::new();
    let s = script(&mut g, "import \"x\" as y;", json!([]), json!(["a"]), &[]);
    let (_, ctx) = run(&g, &s);
    assert!(!errors(&ctx, &s).is_empty(), "import is compiled out");
}

/// A component that fails inside a script stops the script with its reason.
#[test]
fn script_boolean_error_propagates() {
    let mut g = Graph::new();
    let s = script(&mut g, "a = difference(line(point(0,0,0), point(1,0,0)), line(point(0,0,0), point(0,1,0)));", json!([]), json!(["a"]), &[]);
    let (_, ctx) = run(&g, &s);
    assert!(errors(&ctx, &s).iter().any(|e| e.contains("cannot be combined")), "{:?}", ctx.diagnostics);
}

#[test]
fn script_run_checks_the_component() {
    let mut g = Graph::new();
    let s = script(&mut g, "a = run(\"Nope\", []);", json!([]), json!(["a"]), &[]);
    let (_, ctx) = run(&g, &s);
    assert!(errors(&ctx, &s).iter().any(|e| e.contains("no component \"Nope\"")), "{:?}", ctx.diagnostics);
    let s2 = script(&mut g, "a = run(\"Line\", [1, 2, 3]);", json!([]), json!(["a"]), &[]);
    let (_, ctx) = run(&g, &s2);
    assert!(errors(&ctx, &s2).iter().any(|e| e.contains("Line takes 2 inputs: Start, End")), "{:?}", ctx.diagnostics);
}

#[test]
fn script_print_goes_to_node() {
    let mut g = Graph::new();
    let s = script(&mut g, "print(\"hello \" + 42); a = 1;", json!([]), json!(["a"]), &[]);
    let (_, ctx) = run(&g, &s);
    let infos: Vec<String> = ctx.diagnostics.get(&s).map(|d| d.iter().filter(|x| x.level == Level::Info).map(|x| x.message.clone()).collect()).unwrap_or_default();
    assert_eq!(infos, ["hello 42"]);
}

#[test]
fn script_rng_is_deterministic() {
    let mut g = Graph::new();
    let s = script(&mut g, "let r = rng(7); let q = rng(7); a = [r.next(), q.next(), r.next()];", json!([]), json!(["a"]), &[]);
    let (out, _) = run(&g, &s);
    let v = nums(&out[0]);
    assert_eq!(v[0], v[1]);
    assert_ne!(v[0], v[2]);
    assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
}

#[test]
fn script_helix_shorthand() {
    let mut g = Graph::new();
    let s = script(&mut g, "let h = helix(point(0,0,0), point(0,0,10), 5, 2); a = curve_length(h);", json!([]), json!(["a"]), &[]);
    let (out, ctx) = run(&g, &s);
    assert!(errors(&ctx, &s).is_empty(), "{:?}", ctx.diagnostics);
    let expect = ((2.0 * std::f64::consts::PI * 5.0 * 2.0f64).powi(2) + 100.0).sqrt();
    assert!((nums(&out[0])[0] - expect).abs() < expect * 0.01);
}
