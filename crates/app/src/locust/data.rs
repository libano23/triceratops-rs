//! Data trees, the values that flow between Locust nodes.
//!
//! A Locust value is never just a number: it is a tree of lists. Every item sits in a
//! branch, and a branch has a path — `{0}`, `{0;1}`, `{2}` — so a component can tell
//! "the third circle of the second row" from "the second circle of the third row"
//! without counting. Most of the work is following the trees: matching the branches of
//! several inputs, broadcasting a single branch against many, and putting the results
//! back under the path they came from.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use glam::DVec3;
use tri_doc::Geometry;

/// A branch path: `{0;1}` is `vec![0, 1]`. Sorted like Python tuples (lexicographic).
pub type Path = Vec<i64>;

/// One item of a data tree.
#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Int(i64),
    Number(f64),
    Text(String),
    /// A point or a vector.
    Point(DVec3),
    /// A domain (start, end) or a (u, v) pair.
    Pair(f64, f64),
    /// RGBA, each 0..1.
    Colour([f64; 4]),
    /// Origin and normal.
    Plane(DVec3, DVec3),
    /// A list held as one item (a path from Deconstruct Path, a list of object ids).
    List(Vec<Value>),
    Geometry(Arc<Geometry>),
}

impl Value {
    pub fn geometry(g: Geometry) -> Self {
        Value::Geometry(Arc::new(g))
    }

    pub fn as_geometry(&self) -> Option<&Geometry> {
        match self {
            Value::Geometry(g) => Some(g),
            _ => None,
        }
    }


    /// Python-style `str()` of the value, used for Text inputs and the Panel.
    pub fn to_text(&self) -> String {
        match self {
            Value::Null => "None".into(),
            Value::Bool(b) => if *b { "True" } else { "False" }.into(),
            Value::Int(i) => i.to_string(),
            Value::Number(f) => py_float(*f),
            Value::Text(s) => s.clone(),
            Value::Point(p) => format!("({}, {}, {})", py_float(p.x), py_float(p.y), py_float(p.z)),
            Value::Pair(a, b) => format!("({}, {})", py_float(*a), py_float(*b)),
            Value::Colour(c) => format!("({}, {}, {}, {})", py_float(c[0]), py_float(c[1]), py_float(c[2]), py_float(c[3])),
            Value::Plane(o, n) => format!("({}, {})", Value::Point(*o).to_text(), Value::Point(*n).to_text()),
            Value::List(items) => format!("[{}]", items.iter().map(|v| v.repr()).collect::<Vec<_>>().join(", ")),
            Value::Geometry(g) => format!("<{}>", geometry_kind(g)),
        }
    }

    /// Python-style `repr()`: strings are quoted. Used as identity key by the set tools.
    pub fn repr(&self) -> String {
        match self {
            Value::Text(s) => format!("'{s}'"),
            Value::Geometry(g) => format!("<{} {:p}>", geometry_kind(g), Arc::as_ptr(g)),
            other => other.to_text(),
        }
    }

    /// A short label for the canvas.
    pub fn short(&self) -> String {
        if let Value::Geometry(g) = self {
            return geometry_kind(g).to_string();
        }
        let text = self.repr();
        if text.chars().count() <= 24 {
            text
        } else {
            format!("{}…", text.chars().take(21).collect::<String>())
        }
    }

    // ------------------------------------------------------------------ json

    /// Inline values are stored as JSON in the graph; tuples are JSON arrays.
    pub fn from_json(v: &serde_json::Value) -> Value {
        use serde_json::Value as J;
        match v {
            J::Null => Value::Null,
            J::Bool(b) => Value::Bool(*b),
            J::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Value::Int(i)
                } else {
                    Value::Number(n.as_f64().unwrap_or(0.0))
                }
            }
            J::String(s) => Value::Text(s.clone()),
            J::Array(items) => {
                let nums: Option<Vec<f64>> = items.iter().map(|x| x.as_f64()).collect();
                match (nums, items.len()) {
                    (Some(n), 3) => Value::Point(DVec3::new(n[0], n[1], n[2])),
                    (Some(n), 2) => Value::Pair(n[0], n[1]),
                    (Some(n), 4) => Value::Colour([n[0], n[1], n[2], n[3]]),
                    _ => {
                        let vals: Vec<Value> = items.iter().map(Value::from_json).collect();
                        if let [Value::Point(o), Value::Point(n)] = vals.as_slice() {
                            Value::Plane(*o, *n)
                        } else {
                            Value::List(vals)
                        }
                    }
                }
            }
            J::Object(_) => Value::Text(v.to_string()),
        }
    }

}

/// Python's `repr(float)`: integral values keep a `.0`.
pub fn py_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf" } else { "-inf" }.into();
    }
    let a = f.abs();
    if a != 0.0 && !(1e-4..1e16).contains(&a) {
        // Python switches to an exponent outside [1e-4, 1e16): 1e-05, 1.5e+20
        let s = format!("{f:e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let e: i32 = e.parse().unwrap_or(0);
        return format!("{m}e{}{:02}", if e < 0 { '-' } else { '+' }, e.abs());
    }
    if f == f.trunc() {
        return format!("{f:.1}");
    }
    // shortest round-trip digits, like Python's repr
    format!("{f}")
}

pub fn geometry_kind(g: &Geometry) -> &'static str {
    match g {
        Geometry::Point(_) => "Point",
        Geometry::Curve(_) => "Curve",
        Geometry::Surface(_) => "Surface",
        Geometry::Brep(b) => {
            if b.closed {
                "Solid"
            } else {
                "Brep"
            }
        }
        Geometry::Mesh(_) => "Mesh",
        _ => "Geometry",
    }
}

// --------------------------------------------------------------- coercion

/// Python truthiness.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Int(i) => *i != 0,
        Value::Number(f) => *f != 0.0,
        Value::Text(s) => !s.is_empty(),
        Value::List(l) => !l.is_empty(),
        _ => true,
    }
}

pub fn as_number_or(v: &Value, default: f64) -> f64 {
    match v {
        Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        Value::Int(i) => *i as f64,
        Value::Number(f) => *f,
        Value::Text(s) => parse_py_float(s).unwrap_or(default),
        _ => default,
    }
}

pub fn as_number(v: &Value) -> f64 {
    as_number_or(v, 0.0)
}

/// Python `float(str)`: surrounding whitespace allowed, `inf`/`nan` accepted.
pub fn parse_py_float(s: &str) -> Option<f64> {
    let t = s.trim();
    match t.to_ascii_lowercase().as_str() {
        "inf" | "+inf" | "infinity" | "+infinity" => return Some(f64::INFINITY),
        "-inf" | "-infinity" => return Some(f64::NEG_INFINITY),
        "nan" | "+nan" | "-nan" => return Some(f64::NAN),
        _ => {}
    }
    t.replace('_', "").parse::<f64>().ok()
}

/// Python `int(round(float(v)))`: halves round to even.
pub fn as_int_or(v: &Value, default: i64) -> i64 {
    match v {
        Value::Bool(b) => *b as i64,
        Value::Int(i) => *i,
        Value::Number(f) if f.is_finite() => f.round_ties_even() as i64,
        Value::Text(s) => parse_py_float(s).filter(|f| f.is_finite()).map(|f| f.round_ties_even() as i64).unwrap_or(default),
        _ => default,
    }
}

pub fn as_int(v: &Value) -> i64 {
    as_int_or(v, 0)
}

/// A point from a point value or a 2/3-number list; `default` otherwise.
pub fn as_point_or(v: &Value, default: DVec3) -> DVec3 {
    match v {
        Value::Point(p) => *p,
        Value::Pair(a, b) => DVec3::new(*a, *b, 0.0),
        Value::Colour(c) => DVec3::new(c[0], c[1], c[2]),
        Value::Geometry(g) => match g.as_ref() {
            Geometry::Point(p) => *p,
            _ => default,
        },
        Value::List(items) if items.len() >= 2 => {
            let n: Vec<f64> = items.iter().take(3).map(as_number).collect();
            DVec3::new(n[0], n[1], n.get(2).copied().unwrap_or(0.0))
        }
        _ => default,
    }
}

pub fn as_point(v: &Value) -> DVec3 {
    as_point_or(v, DVec3::ZERO)
}

// ------------------------------------------------------------------ trees

/// Items grouped into branches keyed by path. A branch is a list.
#[derive(Clone, Debug, Default)]
pub struct DataTree {
    pub branches: BTreeMap<Path, Vec<Value>>,
}

impl DataTree {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn single(v: Value) -> Self {
        Self::from_list(vec![v])
    }

    pub fn from_list(items: Vec<Value>) -> Self {
        Self::from_list_at(items, Vec::new())
    }

    pub fn from_list_at(items: Vec<Value>, path: Path) -> Self {
        let mut b = BTreeMap::new();
        b.insert(path, items);
        Self { branches: b }
    }

    pub fn from_branches(branches: impl IntoIterator<Item = (Path, Vec<Value>)>) -> Self {
        let mut b = BTreeMap::new();
        for (p, items) in branches {
            b.insert(p, items);
        }
        Self { branches: b }
    }

    pub fn is_empty(&self) -> bool {
        self.branches.values().all(|b| b.is_empty())
    }


    /// Paths of the non-empty branches, sorted.
    pub fn paths(&self) -> Vec<Path> {
        self.branches.iter().filter(|(_, items)| !items.is_empty()).map(|(p, _)| p.clone()).collect()
    }

    pub fn branch(&self, path: &[i64]) -> Vec<Value> {
        self.branches.get(path).cloned().unwrap_or_default()
    }

    pub fn branch_ref(&self, path: &[i64]) -> &[Value] {
        self.branches.get(path).map(|b| b.as_slice()).unwrap_or(&[])
    }

    /// Every item, branch after branch.
    pub fn items(&self) -> Vec<Value> {
        self.branches.values().flatten().cloned().collect()
    }

    pub fn first(&self) -> Option<&Value> {
        self.branches.values().flatten().next()
    }

    pub fn count(&self) -> usize {
        self.branches.values().map(|b| b.len()).sum()
    }

    pub fn branch_count(&self) -> usize {
        self.branches.values().filter(|b| !b.is_empty()).count()
    }

    /// The first branch as stored (empty or not) — what a single-branch tree broadcasts.
    fn first_branch(&self) -> Vec<Value> {
        self.branches.values().find(|b| !b.is_empty()).or_else(|| self.branches.values().next()).cloned().unwrap_or_default()
    }

    // ------------------------------------------------------------ transforms

    /// Every item into the single root branch.
    pub fn flatten(&self) -> Self {
        Self::from_list(self.items())
    }

    /// Each item into its own branch, one under the next.
    pub fn graft(&self) -> Self {
        let mut out = BTreeMap::new();
        let mut n = 0;
        for path in self.paths() {
            for item in &self.branches[&path] {
                let mut p = path.clone();
                p.push(n);
                out.insert(p, vec![item.clone()]);
                n += 1;
            }
        }
        if out.is_empty() {
            out.insert(Vec::new(), Vec::new());
        }
        Self { branches: out }
    }

    /// Drop paths that only add a constant leading index — `{0;0}` -> `{0}`.
    pub fn simplify(&self) -> Self {
        let paths = self.paths();
        if paths.len() <= 1 {
            return self.clone();
        }
        let mut out: BTreeMap<Path, Vec<Value>> = BTreeMap::new();
        for (i, path) in paths.iter().enumerate() {
            let mut key: Path = if path.len() > 1 && path[0] == 0 { path[1..].to_vec() } else { path.clone() };
            if out.contains_key(&key) {
                key.push(i as i64);
            }
            out.insert(key, self.branches[path].clone());
        }
        Self { branches: out }
    }

    pub fn map(&self, f: impl Fn(&Value) -> Value) -> Self {
        Self { branches: self.branches.iter().map(|(p, items)| (p.clone(), items.iter().map(&f).collect())).collect() }
    }

    /// Concatenate two trees, branches under the same path joined.
    pub fn merge(&self, other: &Self) -> Self {
        let mut out = self.clone();
        for path in other.paths() {
            out.branches.entry(path.clone()).or_default().extend(other.branches[&path].iter().cloned());
        }
        out
    }

    /// Keep only the last `depth` indices of every path.
    pub fn trim(&self, depth: i64) -> Self {
        if depth <= 0 {
            return Self::from_list(self.items());
        }
        let mut out: BTreeMap<Path, Vec<Value>> = BTreeMap::new();
        for (path, items) in &self.branches {
            let d = depth as usize;
            let key = if path.len() > d { path[path.len() - d..].to_vec() } else { path.clone() };
            out.entry(key).or_default().extend(items.iter().cloned());
        }
        Self { branches: out }
    }

    /// Add `offset` to the last index of every path.
    pub fn shift_paths(&self, offset: i64) -> Self {
        let mut out = BTreeMap::new();
        for (path, items) in &self.branches {
            let key = match path.split_last() {
                Some((last, head)) => {
                    let mut k = head.to_vec();
                    k.push(last + offset);
                    k
                }
                None => vec![offset],
            };
            out.insert(key, items.clone());
        }
        Self { branches: out }
    }

    /// The paths themselves, as a list of index lists.
    pub fn branch_paths(&self) -> Self {
        Self::from_list(self.paths().into_iter().map(|p| Value::List(p.into_iter().map(Value::Int).collect())).collect())
    }

    /// Split each branch into chunks of `count`, one branch each.
    pub fn graft_by(&self, count: i64) -> Self {
        if count < 1 {
            return self.clone();
        }
        let mut out = BTreeMap::new();
        let mut n = 0;
        for path in self.paths() {
            for chunk in self.branches[&path].chunks(count as usize) {
                let mut p = path.clone();
                p.push(n);
                out.insert(p, chunk.to_vec());
                n += 1;
            }
        }
        Self { branches: out }
    }

    /// Sliding windows of `size`, `step` apart, one branch per window.
    pub fn partition(&self, size: i64, step: Option<i64>) -> Self {
        if size < 1 {
            return self.clone();
        }
        let size = size as usize;
        let stride = step.map(|s| s.max(1) as usize).unwrap_or(size);
        let mut out = BTreeMap::new();
        for path in self.paths() {
            let branch = &self.branches[&path];
            let mut n = 0;
            let mut start = 0;
            while start + size <= branch.len() {
                let mut p = path.clone();
                p.push(n);
                out.insert(p, branch[start..start + size].to_vec());
                n += 1;
                start += stride;
            }
            if n == 0 {
                let mut p = path.clone();
                p.push(0);
                out.insert(p, Vec::new());
            }
        }
        Self { branches: out }
    }

    /// A few lines of the tree, for a Panel node on the canvas.
    pub fn format(&self, max_paths: usize, max_items: usize) -> String {
        let mut s = String::new();
        for path in self.paths().into_iter().take(max_paths) {
            let items = &self.branches[&path];
            let head = format!("{{{}}}", path.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(";"));
            let mut shown = items.iter().take(max_items).map(|v| v.short()).collect::<Vec<_>>().join(", ");
            if items.len() > max_items {
                shown.push_str(" …");
            }
            if !s.is_empty() {
                s.push('\n');
            }
            let _ = write!(s, "{head}  {shown}");
        }
        s
    }
}

/// Wrap a value as a one-branch tree; a list value becomes the branch's items.
pub fn to_tree(v: &Value) -> DataTree {
    match v {
        Value::List(items) => DataTree::from_list(items.clone()),
        other => DataTree::single(other.clone()),
    }
}

// --------------------------------------------------------------- matching

/// One combination of inputs: the path it belongs to and the values.
#[derive(Clone, Debug)]
pub struct Match {
    pub path: Path,
    pub values: Vec<Value>,
}

/// A one-branch tree is stretched to every path the others use.
fn broadcast(trees: &[DataTree]) -> Vec<DataTree> {
    let multi: Vec<&DataTree> = trees.iter().filter(|t| t.branch_count() > 1).collect();
    if multi.is_empty() {
        return trees.to_vec();
    }
    let mut paths: Vec<Path> = multi.iter().flat_map(|t| t.paths()).collect();
    paths.sort();
    paths.dedup();
    trees
        .iter()
        .map(|t| {
            if t.branch_count() <= 1 {
                let b = t.first_branch();
                DataTree::from_branches(paths.iter().map(|p| (p.clone(), b.clone())))
            } else {
                t.clone()
            }
        })
        .collect()
}

/// Grasshopper-style input matching: branches by path, items by longest list.
///
/// A single-branch input broadcasts to every path the others use. Inside a path the
/// longest branch runs the combination and the shorter ones repeat their last item, which
/// is what lets a lone number drive a list of points. An input with no items at all
/// contributes nothing and the combination is skipped.
pub fn match_trees(trees: &[DataTree]) -> Vec<Match> {
    let trees = broadcast(trees);
    if trees.is_empty() {
        return Vec::new();
    }
    let mut paths: Vec<Path> = trees.iter().flat_map(|t| t.paths()).collect();
    paths.sort();
    paths.dedup();
    let mut out = Vec::new();
    for path in paths {
        let branches: Vec<Vec<Value>> = trees.iter().map(|t| if t.branches.contains_key(&path) { t.branch(&path) } else { t.first_branch() }).collect();
        if branches.iter().any(|b| b.is_empty()) {
            continue;
        }
        let n = branches.iter().map(|b| b.len()).max().unwrap_or(0);
        for i in 0..n {
            out.push(Match { path: path.clone(), values: branches.iter().map(|b| b.get(i).unwrap_or(b.last().unwrap()).clone()).collect() });
        }
    }
    out
}

/// The kinds of input a component declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Number,
    Int,
    Bool,
    Text,
    Point,
    Geometry,
    Any,
    Tree,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Number => "number",
            Kind::Int => "int",
            Kind::Bool => "bool",
            Kind::Text => "text",
            Kind::Point => "point",
            Kind::Geometry => "geometry",
            Kind::Any => "any",
            Kind::Tree => "tree",
        }
    }

    pub fn is_scalar(self) -> bool {
        matches!(self, Kind::Number | Kind::Int | Kind::Bool | Kind::Text)
    }
}

/// Coerce a tree toward a component's input kind.
pub fn cast_tree(tree: &DataTree, kind: Kind) -> DataTree {
    match kind {
        Kind::Number => tree.map(|v| Value::Number(as_number(v))),
        Kind::Int => tree.map(|v| Value::Int(as_int(v))),
        Kind::Bool => tree.map(|v| Value::Bool(truthy(v))),
        Kind::Point => tree.map(|v| Value::Point(as_point(v))),
        Kind::Text => tree.map(|v| Value::Text(if matches!(v, Value::Null) { String::new() } else { v.to_text() })),
        _ => tree.clone(),
    }
}
