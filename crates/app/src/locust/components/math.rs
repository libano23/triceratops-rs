//! Math: arithmetic, trigonometry (in degrees, like the Python Locust) and statistics.

use super::super::data::{as_number, DataTree, Value};
use super::*;

/// Python `round(v, d)` (halves to even).
fn py_round(v: f64, d: i64) -> f64 {
    let m = 10f64.powi(d as i32);
    let r = (v * m).round_ties_even() / m;
    if r.is_finite() {
        r
    } else {
        v
    }
}

pub fn register(r: &mut Registry) {
    r.elem("Add", "Math", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(n(a.num(0) + a.num(1))));
    r.elem("Subtract", "Math", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(n(a.num(0) - a.num(1))));
    r.elem("Multiply", "Math", vec![num("A", 1.0), num("B", 1.0)], &["Result"], |_, a| one(n(a.num(0) * a.num(1))));
    r.elem("Divide", "Math", vec![num("A", 1.0), num("B", 1.0)], &["Result"], |_, a| one(n(if a.num(1).abs() > 1e-12 { a.num(0) / a.num(1) } else { 0.0 })));
    r.elem("Power", "Math", vec![num("A", 1.0), num("B", 2.0)], &["Result"], |_, a| one(n(a.num(0).powf(a.num(1)))));
    r.elem("Modulus", "Math", vec![num("A", 1.0), num("B", 1.0)], &["Result"], |_, a| one(n(if a.num(1).abs() > 1e-12 { a.num(0) % a.num(1) } else { 0.0 })));
    r.elem("Negate", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(-a.num(0))));
    r.elem("Absolute", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).abs())));
    r.elem("Square Root", "Math", vec![num("Value", 1.0)], &["Result"], |_, a| one(n(a.num(0).abs().sqrt())));
    r.elem("Sine", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).to_radians().sin())));
    r.elem("Cosine", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).to_radians().cos())));
    r.elem("Tangent", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).to_radians().tan())));
    r.elem("Minimum", "Math", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(n(a.num(0).min(a.num(1)))));
    r.elem("Maximum", "Math", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(n(a.num(0).max(a.num(1)))));
    r.elem("Average", "Math", vec![num("A", 0.0), num("B", 0.0)], &["Result"], |_, a| one(n((a.num(0) + a.num(1)) / 2.0)));
    r.elem("Round", "Math", vec![num("Value", 0.0), int("Decimals", 0)], &["Result"], |_, a| one(n(py_round(a.num(0), a.int(1)))));
    r.elem("Clamp", "Math", vec![num("Value", 0.0), num("Min", 0.0), num("Max", 1.0)], &["Result"], |_, a| one(n(a.num(1).max(a.num(2).min(a.num(0))))));
    r.elem("Remap", "Math", vec![num("Value", 0.0), num("From Min", 0.0), num("From Max", 1.0), num("To Min", 0.0), num("To Max", 1.0)], &["Result"], |_, a| {
        let (lo, hi, c, d) = (a.num(1), a.num(2), a.num(3), a.num(4));
        let t = if (hi - lo).abs() > 1e-12 { (a.num(0) - lo) / (hi - lo) } else { 0.0 };
        one(n(c + (d - c) * t))
    });
    r.elem("Floor", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).floor())));
    r.elem("Ceiling", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).ceil())));
    r.elem("Truncate", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).trunc())));
    r.elem("Sign", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| {
        let v = a.num(0);
        one(n(((v > 0.0) as i32 - (v < 0.0) as i32) as f64))
    });
    r.elem("Exponential", "Math", vec![num("Value", 0.0)], &["Result"], |_, a| one(n(a.num(0).exp())));
    r.elem("Logarithm", "Math", vec![num("Value", 1.0), num("Base", 10.0)], &["Result"], |_, a| {
        let (v, b) = (a.num(0), a.num(1));
        if v <= 0.0 || b <= 0.0 || (b - 1.0).abs() < 1e-12 {
            return None;
        }
        one(n(v.ln() / b.ln()))
    });
    r.elem("Degrees", "Math", vec![num("Radians", 0.0)], &["Degrees"], |_, a| one(n(a.num(0).to_degrees())));
    r.elem("Radians", "Math", vec![num("Degrees", 0.0)], &["Radians"], |_, a| one(n(a.num(0).to_radians())));
    r.elem("Arc Sine", "Math", vec![num("Value", 0.0)], &["Degrees"], |_, a| one(n(a.num(0).clamp(-1.0, 1.0).asin().to_degrees())));
    r.elem("Arc Cosine", "Math", vec![num("Value", 1.0)], &["Degrees"], |_, a| one(n(a.num(0).clamp(-1.0, 1.0).acos().to_degrees())));
    r.elem("Arc Tangent", "Math", vec![num("Y", 0.0), num("X", 1.0)], &["Degrees"], |_, a| one(n(a.num(0).atan2(a.num(1)).to_degrees())));
    r.elem("Factorial", "Math", vec![int("Value", 5)], &["Result"], |_, a| {
        let k = a.int(0).max(0);
        if k > 170 {
            return None;
        }
        one(n((1..=k).fold(1.0, |acc, i| acc * i as f64)))
    });
    r.tree("Sum", "Math", vec![tree("Tree")], &["Sum"], |_, t| t1(DataTree::single(n(t[0].items().iter().map(as_number).sum()))));
    r.tree("Mean", "Math", vec![tree("Tree")], &["Mean"], |_, t| {
        let items: Vec<f64> = t[0].items().iter().map(as_number).collect();
        t1(DataTree::single(n(if items.is_empty() { 0.0 } else { items.iter().sum::<f64>() / items.len() as f64 })))
    });
    r.tree("Weighted Average", "Math", vec![tree("Tree"), tree("Weights")], &["Result"], |_, t| {
        let values: Vec<f64> = t[0].items().iter().map(as_number).collect();
        let ws: Vec<f64> = t[1].items().iter().map(as_number).collect();
        if values.is_empty() || ws.is_empty() {
            return t1(DataTree::empty());
        }
        let total: f64 = (0..values.len()).map(|i| ws[i % ws.len()]).sum();
        if total.abs() < 1e-12 {
            return t1(DataTree::single(n(0.0)));
        }
        let acc: f64 = (0..values.len()).map(|i| values[i] * ws[i % ws.len()]).sum();
        t1(DataTree::single(Value::Number(acc / total)))
    });
}
