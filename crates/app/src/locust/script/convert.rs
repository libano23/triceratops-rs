//! Locust values as Rhai values and back.

use std::sync::Arc;

use glam::DVec3;
use rhai::{Array, Dynamic, Engine, FLOAT, INT};
use tri_doc::Geometry;

use super::super::data::Value;

/// A point or a vector (Rhai type `Point`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt(pub DVec3);

/// Any geometry (Rhai type `Geometry`).
#[derive(Clone, Debug)]
pub struct Geo(pub Arc<Geometry>);

/// A domain or (u, v) pair (Rhai type `Domain`).
#[derive(Clone, Copy, Debug)]
pub struct Domain(pub f64, pub f64);

/// A plane (Rhai type `Plane`).
#[derive(Clone, Copy, Debug)]
pub struct PlaneV(pub DVec3, pub DVec3);

/// An RGBA colour (Rhai type `Colour`).
#[derive(Clone, Copy, Debug)]
pub struct Colour(pub [f64; 4]);

pub fn to_dynamic(v: &Value) -> Dynamic {
    match v {
        Value::Null => Dynamic::UNIT,
        Value::Bool(b) => Dynamic::from_bool(*b),
        Value::Int(i) => Dynamic::from_int(*i as INT),
        Value::Number(f) => Dynamic::from_float(*f as FLOAT),
        Value::Text(t) => Dynamic::from(t.clone()),
        Value::Point(p) => Dynamic::from(Pt(*p)),
        Value::Pair(a, b) => Dynamic::from(Domain(*a, *b)),
        Value::Colour(c) => Dynamic::from(Colour(*c)),
        Value::Plane(o, n) => Dynamic::from(PlaneV(*o, *n)),
        Value::List(items) => Dynamic::from_array(items.iter().map(to_dynamic).collect()),
        Value::Geometry(g) => Dynamic::from(Geo(g.clone())),
    }
}

pub fn from_dynamic(d: Dynamic) -> Result<Value, String> {
    if d.is_unit() {
        return Ok(Value::Null);
    }
    if let Some(b) = d.clone().try_cast::<bool>() {
        return Ok(Value::Bool(b));
    }
    if let Ok(i) = d.as_int() {
        return Ok(Value::Int(i));
    }
    if let Ok(f) = d.as_float() {
        return Ok(Value::Number(f));
    }
    if d.is_string() {
        return Ok(Value::Text(d.into_string().unwrap_or_default()));
    }
    if d.is_array() {
        let items: Array = d.cast();
        return items.into_iter().map(from_dynamic).collect::<Result<Vec<_>, _>>().map(Value::List);
    }
    let type_name = d.type_name().to_string();
    if let Some(p) = d.clone().try_cast::<Pt>() {
        return Ok(Value::Point(p.0));
    }
    if let Some(g) = d.clone().try_cast::<Geo>() {
        return Ok(Value::Geometry(g.0));
    }
    if let Some(x) = d.clone().try_cast::<Domain>() {
        return Ok(Value::Pair(x.0, x.1));
    }
    if let Some(x) = d.clone().try_cast::<PlaneV>() {
        return Ok(Value::Plane(x.0, x.1));
    }
    if let Some(x) = d.try_cast::<Colour>() {
        return Ok(Value::Colour(x.0));
    }
    Err(format!("expected number, point, geometry or array, got {}", short_type(&type_name)))
}

fn short_type(t: &str) -> &str {
    t.rsplit("::").next().unwrap_or(t)
}

/// The Locust types and their properties and operators.
pub fn register_types(e: &mut Engine) {
    e.register_type_with_name::<Pt>("Point")
        .register_get("x", |p: &mut Pt| p.0.x)
        .register_get("y", |p: &mut Pt| p.0.y)
        .register_get("z", |p: &mut Pt| p.0.z)
        .register_set("x", |p: &mut Pt, v: FLOAT| p.0.x = v)
        .register_set("y", |p: &mut Pt, v: FLOAT| p.0.y = v)
        .register_set("z", |p: &mut Pt, v: FLOAT| p.0.z = v)
        .register_fn("+", |a: Pt, b: Pt| Pt(a.0 + b.0))
        .register_fn("-", |a: Pt, b: Pt| Pt(a.0 - b.0))
        .register_fn("-", |a: Pt| Pt(-a.0))
        .register_fn("*", |a: Pt, f: FLOAT| Pt(a.0 * f))
        .register_fn("*", |f: FLOAT, a: Pt| Pt(a.0 * f))
        .register_fn("*", |a: Pt, f: INT| Pt(a.0 * f as f64))
        .register_fn("*", |f: INT, a: Pt| Pt(a.0 * f as f64))
        .register_fn("/", |a: Pt, f: FLOAT| Pt(a.0 / f))
        .register_fn("/", |a: Pt, f: INT| Pt(a.0 / f as f64))
        .register_fn("==", |a: Pt, b: Pt| a == b)
        .register_fn("dot", |a: Pt, b: Pt| a.0.dot(b.0))
        .register_fn("cross", |a: Pt, b: Pt| Pt(a.0.cross(b.0)))
        .register_fn("length", |a: Pt| a.0.length())
        .register_fn("distance", |a: Pt, b: Pt| a.0.distance(b.0))
        .register_fn("unit", |a: Pt| Pt(a.0.normalize_or_zero()))
        .register_fn("to_string", |a: &mut Pt| format!("({}, {}, {})", a.0.x, a.0.y, a.0.z))
        .register_fn("to_debug", |a: &mut Pt| format!("({}, {}, {})", a.0.x, a.0.y, a.0.z));
    e.register_type_with_name::<Geo>("Geometry")
        .register_get("kind", |g: &mut Geo| super::super::data::geometry_kind(&g.0).to_string())
        .register_fn("to_string", |g: &mut Geo| super::super::data::geometry_kind(&g.0).to_string());
    e.register_type_with_name::<Domain>("Domain").register_get("start", |d: &mut Domain| d.0).register_get("end", |d: &mut Domain| d.1);
    e.register_type_with_name::<PlaneV>("Plane").register_get("origin", |p: &mut PlaneV| Pt(p.0)).register_get("normal", |p: &mut PlaneV| Pt(p.1));
    e.register_type_with_name::<Colour>("Colour")
        .register_get("r", |c: &mut Colour| c.0[0])
        .register_get("g", |c: &mut Colour| c.0[1])
        .register_get("b", |c: &mut Colour| c.0[2])
        .register_get("a", |c: &mut Colour| c.0[3]);
}
