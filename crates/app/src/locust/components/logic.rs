//! Logic: booleans and comparisons.

use super::super::data::Value;
use super::*;

pub fn register(r: &mut Registry) {
    r.elem("Not", "Logic", vec![boolean("Value", false)], &["Result"], |_, a| one(Value::Bool(!a.flag(0))));
    r.elem("And", "Logic", vec![boolean("A", true), boolean("B", true)], &["Result"], |_, a| one(Value::Bool(a.flag(0) && a.flag(1))));
    r.elem("Or", "Logic", vec![boolean("A", false), boolean("B", false)], &["Result"], |_, a| one(Value::Bool(a.flag(0) || a.flag(1))));
    r.elem("Xor", "Logic", vec![boolean("A", false), boolean("B", false)], &["Result"], |_, a| one(Value::Bool(a.flag(0) != a.flag(1))));
    r.elem("Larger Than", "Logic", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(Value::Bool(a.num(0) > a.num(1))));
    r.elem("Smaller Than", "Logic", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(Value::Bool(a.num(0) < a.num(1))));
    r.elem("Equality", "Logic", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(Value::Bool((a.num(0) - a.num(1)).abs() < 1e-9)));
}
