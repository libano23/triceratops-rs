//! String: text.

use super::super::data::{parse_py_float, DataTree, Value};
use super::*;

pub fn register(r: &mut Registry) {
    r.elem("Concatenate", "String", vec![text("A", ""), text("B", "")], &["Result"], |_, a| one(Value::Text(format!("{}{}", a.text(0), a.text(1)))));
    r.tree("Text Join", "String", vec![tree("Tree"), text("Separator", ", ")], &["Text"], |_, t| {
        let sep = first_text(&t[1], ", ");
        t1(DataTree::single(Value::Text(t[0].items().iter().map(|v| v.to_text()).collect::<Vec<_>>().join(&sep))))
    });
    r.tree("Text Split", "String", vec![text("Text", ""), text("Separator", ",")], &["Text"], |_, t| {
        let s = if t[0].count() > 0 { first_text(&t[0], "") } else { String::new() };
        let sep = first_text(&t[1], ",");
        if sep.is_empty() {
            return t1(DataTree::empty());
        }
        t1(DataTree::from_list(s.split(sep.as_str()).map(|x| Value::Text(x.to_string())).collect()))
    });
    r.elem("Text Length", "String", vec![text("Text", "")], &["Length"], |_, a| one(Value::Int(a.text(0).chars().count() as i64)));
    r.elem("Substring", "String", vec![text("Text", ""), int("Start", 0), int("Length", 3)], &["Text"], |_, a| {
        let i = a.int(1).max(0) as usize;
        let len = a.int(2).max(0) as usize;
        one(Value::Text(a.text(0).chars().skip(i).take(len).collect()))
    });
    r.elem("Replace Text", "String", vec![text("Text", ""), text("Find", ""), text("Replace", "")], &["Text"], |_, a| {
        let (s, f, rp) = (a.text(0), a.text(1), a.text(2));
        // Python inserts the replacement between every character for an empty pattern
        let out = if f.is_empty() {
            let mut o = rp.clone();
            for c in s.chars() {
                o.push(c);
                o.push_str(&rp);
            }
            o
        } else {
            s.replace(&f, &rp)
        };
        one(Value::Text(out))
    });
    r.elem("Number to Text", "String", vec![num("Value", 0.0), int("Decimals", 3)], &["Text"], |_, a| one(Value::Text(format!("{:.*}", a.int(1).max(0) as usize, a.num(0)))));
    r.elem("Text to Number", "String", vec![text("Text", "0")], &["Number"], |_, a| one(n(parse_py_float(&a.text(0)).unwrap_or(0.0))));
}
