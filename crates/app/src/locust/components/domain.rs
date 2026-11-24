//! Domain: numeric intervals.

use super::super::data::{as_number, DataTree, Value};
use super::*;

pub fn register(r: &mut Registry) {
    r.elem("Construct Domain", "Domain", vec![num("Start", 0.0), num("End", 1.0)], &["Domain"], |_, a| one(Value::Pair(a.num(0), a.num(1))));
    r.elem("Deconstruct Domain", "Domain", vec![any_d("Domain", Value::Pair(0.0, 1.0))], &["Start", "End"], |_, a| match a.get(0) {
        Value::Pair(s, e) => outs(vec![Out::One(n(*s)), Out::One(n(*e))]),
        Value::Point(q) => outs(vec![Out::One(n(q.x)), Out::One(n(q.y))]),
        Value::List(l) if l.len() >= 2 => outs(vec![Out::One(n(as_number(&l[0]))), Out::One(n(as_number(&l[1])))]),
        _ => None,
    });
    r.tree("Bounds", "Domain", vec![tree("Tree")], &["Domain"], |_, t| {
        let nums: Vec<f64> = t[0].items().iter().filter_map(|v| match v {
            Value::Int(i) => Some(*i as f64),
            Value::Number(f) => Some(*f),
            Value::Bool(b) => Some(*b as i64 as f64),
            _ => None,
        }).collect();
        if nums.is_empty() {
            return t1(DataTree::empty());
        }
        let lo = nums.iter().cloned().fold(f64::INFINITY, f64::min);
        let hi = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        t1(DataTree::single(Value::Pair(lo, hi)))
    });
}
