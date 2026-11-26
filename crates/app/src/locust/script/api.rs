//! What scripts can call: points and vectors, a seeded random generator, any component
//! through `run`, and shorthands for the common ones (a Rhai prelude over `run`).

use std::cell::RefCell;

use rhai::{Array, Dynamic, Engine, EvalAltResult, FLOAT, INT};

use super::super::components::{registry, Args, Ctx, Func, Level, Objects, Out};
use super::super::data::{cast_tree, DataTree, Value};
use super::convert::{from_dynamic, to_dynamic, Pt};
use crate::locust::components::random::SplitMix64;

thread_local! {
    /// The document objects of the solve running on this thread (components called through
    /// `run` read them), and what the script printed.
    static OBJECTS: RefCell<Objects> = RefCell::new(Default::default());
    static PRINTED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Make `objects` the ones `run` sees on this thread while a node solves.
pub fn set_objects(objects: Objects) {
    OBJECTS.with(|o| *o.borrow_mut() = objects);
}

/// What the script printed since the last call.
pub fn take_printed() -> Vec<String> {
    PRINTED.with(|p| std::mem::take(&mut *p.borrow_mut()))
}

/// A seeded random generator (Rhai type `Random`).
#[derive(Clone)]
pub struct Rng(SplitMix64);

fn num(d: &Dynamic) -> Result<FLOAT, Box<EvalAltResult>> {
    d.as_float().or_else(|_| d.as_int().map(|i| i as FLOAT)).map_err(|t| format!("expected a number, got {t}").into())
}

fn point(x: Dynamic, y: Dynamic, z: Dynamic) -> Result<Pt, Box<EvalAltResult>> {
    Ok(Pt(glam::DVec3::new(num(&x)?, num(&y)?, num(&z)?)))
}

/// An argument for a tree input: an array of arrays is one branch per inner array.
fn arg_tree(d: &Dynamic) -> Result<DataTree, String> {
    if d.is_array() {
        let items: Array = d.clone().cast();
        if !items.is_empty() && items.iter().all(|x| x.is_array()) {
            let branches = items.iter().enumerate().map(|(i, b)| match from_dynamic(b.clone())? {
                Value::List(v) => Ok((vec![i as i64], v)),
                v => Ok((vec![i as i64], vec![v])),
            });
            return Ok(DataTree::from_branches(branches.collect::<Result<Vec<_>, String>>()?));
        }
        return Ok(DataTree::from_list(items.into_iter().map(from_dynamic).collect::<Result<Vec<_>, _>>()?));
    }
    Ok(DataTree::single(from_dynamic(d.clone())?))
}

/// One output value for a script: one item as itself, several as an array, none as ().
fn out_dynamic(tree: &DataTree) -> Dynamic {
    let items = tree.items();
    match items.len() {
        0 => Dynamic::UNIT,
        1 => to_dynamic(&items[0]),
        _ => Dynamic::from_array(items.iter().map(to_dynamic).collect()),
    }
}

/// Call component `name` with `args` (in input order; missing ones take their defaults);
/// every output, as `out_dynamic` gives them.
fn run_component(name: &str, args: Array) -> Result<Vec<Dynamic>, Box<EvalAltResult>> {
    let reg = registry();
    let comp = reg.get(name).filter(|c| c.category != "Output" && !crate::locust::graph::is_dynamic(c.name)).ok_or_else(|| format!("no component \"{name}\" (scripts cannot call Output components)"))?;
    if args.len() > comp.inputs.len() {
        let names: Vec<&str> = comp.inputs.iter().map(|i| i.name).collect();
        return Err(format!("{} takes {} inputs: {}", comp.name, names.len(), names.join(", ")).into());
    }
    let mut trees = Vec::with_capacity(comp.inputs.len());
    for (i, spec) in comp.inputs.iter().enumerate() {
        let t = match args.get(i) {
            Some(d) if !d.is_unit() => arg_tree(d)?,
            _ => spec.default.clone().map(DataTree::single).unwrap_or_default(),
        };
        trees.push(t);
    }
    let mut ctx = Ctx::with_objects(OBJECTS.with(|o| o.borrow().clone()));
    ctx.current = Some("run".into());
    let outs: Vec<DataTree> = match comp.func {
        Func::Tree(f) => crate::locust::engine::catch_panic(|| f(&mut ctx, &trees)).map_err(|e| format!("{}: {e}", comp.name))?,
        Func::Element(f) => {
            let values: Vec<Value> = trees.iter().zip(&comp.inputs).map(|(t, spec)| if t.is_empty() { Value::Null } else { cast_tree(t, spec.kind).first().cloned().unwrap_or(Value::Null) }).collect();
            let res = crate::locust::engine::catch_panic(|| f(&mut ctx, &Args(&values))).map_err(|e| format!("{}: {e}", comp.name))?.unwrap_or_default();
            res.into_iter()
                .map(|o| match o {
                    Out::Skip => DataTree::empty(),
                    Out::One(v) => DataTree::single(v),
                    Out::Many(v) => DataTree::from_list(v),
                })
                .collect()
        }
    };
    if let Some(e) = ctx.diagnostics.get("run").and_then(|d| d.iter().find(|x| x.level == Level::Error)) {
        return Err(format!("{}: {}", comp.name, e.message).into());
    }
    Ok((0..comp.outputs.len()).map(|k| outs.get(k).map(out_dynamic).unwrap_or(Dynamic::UNIT)).collect())
}

/// Shorthands for scripts, over `run`.
pub const PRELUDE: &str = r#"
fn line(a, b) { run("Line", [a, b]) }
fn polyline(pts) { run("Polyline", [pts]) }
fn polyline(pts, closed) { let p = pts; if closed && p.len() > 0 { p.push(p[0]); } run("Polyline", [p]) }
fn interpolate(pts) { run("Interpolate", [pts]) }
fn interpolate(pts, periodic) { run("Interpolate", [pts, periodic]) }
fn nurbs_curve(pts, degree) { run("Nurbs Curve", [pts, degree]) }
fn circle(c, r) { run("Circle", [c, r]) }
fn circle(c, r, normal) { run("Circle", [c, r, normal]) }
fn arc(c, r, angle) { run("Arc", [c, r, angle]) }
fn helix(start, end, r, turns) { run("Helix", [start, end, r, turns]) }
fn helix(start, end, r, turns, end_radius) { run("Helix", [start, end, r, turns, (), end_radius]) }
fn divide(c, n) { run("Divide Curve", [c, n]) }
fn offset(c, d) { run("Offset Curve", [c, d]) }
fn join(curves) { run("Join Curves", [curves]) }
fn curve_point(c, t) { run("Evaluate Curve", [c, t]) }
fn curve_length(c) { run("Curve Length", [c]) }
fn loft(curves) { run("Loft", [curves]) }
fn extrude(g, dir) { run("Extrude", [g, dir.unit(), dir.length()]) }
fn revolve(curve, axis_point, axis, angle) { run("Revolve", [curve, axis_point, axis, angle]) }
fn sweep1(rail, profile) { run("Sweep1", [profile, rail]) }
fn cap(g) { run("Cap", [g]) }
fn surface_from_grid(rows) { run("Surface from Grid", [rows]) }
fn surface_point(s, u, v) { run("Evaluate Surface", [s, point(u, v, 0.0)]) }
fn box(corner, x, y, z) { run("Box", [corner, x, y, z]) }
fn cylinder(base, r, h) { run("Cylinder", [base, r, h]) }
fn sphere(c, r) { run("Sphere", [c, r]) }
fn union(a, b) { run("Union", [a, b]) }
fn difference(a, b) { run("Difference", [a, b]) }
fn intersection(a, b) { run("Intersection", [a, b]) }
fn move(g, v) { run("Move", [g, v]) }
fn rotate(g, angle, center, axis) { run("Rotate", [g, angle, center, axis]) }
fn scale(g, center, f) { run("Scale", [g, center, f]) }
fn mirror(g, origin, normal) { run("Mirror", [g, origin, normal]) }
fn array_polar(g, center, count, angle) { run("Array Polar", [g, center, count, angle]) }
fn area(g) { run("Area", [g]) }
fn volume(g) { run("Volume", [g]) }
"#;

pub fn register(e: &mut Engine) {
    e.register_fn("point", point).register_fn("vec", point);
    e.register_fn("point", |x: Dynamic, y: Dynamic| point(x, y, Dynamic::from_float(0.0)));
    e.register_fn("vec", |x: Dynamic, y: Dynamic| point(x, y, Dynamic::from_float(0.0)));
    e.register_type_with_name::<Rng>("Random").register_fn("rng", |seed: INT| Rng(SplitMix64::new(seed))).register_fn("next", |r: &mut Rng| r.0.next_f64());
    e.register_fn("run", |name: &str, args: Array| -> Result<Dynamic, Box<EvalAltResult>> { Ok(run_component(name, args)?.into_iter().next().unwrap_or(Dynamic::UNIT)) });
    e.register_fn("run_all", |name: &str, args: Array| -> Result<Array, Box<EvalAltResult>> { run_component(name, args) });
    e.on_print(|s| PRINTED.with(|p| p.borrow_mut().push(s.to_string())));
}
