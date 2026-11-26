//! The component library: what each node does.
//!
//! A component is a name, its inputs and outputs, and a function. Most are *element*
//! components: the engine matches the input branches and calls the function once per
//! combination with plain values, and the results go back under the path they came from.
//! A few are *tree* components that take whole data trees — the list tools, the ones that
//! make a curve through many points, and the ones that preview or bake — because their
//! meaning is the structure, not one item of it.
//!
//! Geometry comes from the application's own kernel (`tri_geom`) and the same helpers the
//! modelling commands use; nothing is re-implemented for the node graph.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

use glam::DVec3;
use tri_doc::{Document, Geometry};
use tri_geom::{Brep, Mesh, NurbsCurve, NurbsSurface};

use super::data::{as_int, as_number, as_point_or, truthy, DataTree, Kind, Value};

mod analysis;
mod curve;
mod domain;
mod logic;
mod math;
mod mesh;
mod output;
mod params;
pub mod populate;
mod sets;
mod solid;
mod string;
mod surface;
mod transform;
mod vector;
pub mod voronoi;

pub mod geo;
pub mod random;

/// One declared input of a component.
#[derive(Clone, Debug)]
pub struct InputSpec {
    pub name: &'static str,
    pub kind: Kind,
    pub default: Option<Value>,
    /// (min, max) to offer a slider on the canvas.
    pub slider: Option<(f64, f64)>,
    pub optional: bool,
}

impl InputSpec {
    pub fn slider(mut self, lo: f64, hi: f64) -> Self {
        self.slider = Some((lo, hi));
        self
    }

    pub fn opt(mut self) -> Self {
        self.optional = true;
        self
    }
}

fn spec(name: &'static str, kind: Kind, default: Option<Value>) -> InputSpec {
    InputSpec { name, kind, default, slider: None, optional: false }
}

pub fn num(name: &'static str, d: f64) -> InputSpec {
    spec(name, Kind::Number, Some(Value::Number(d)))
}
/// An optional number with no default: `Value::Null` when nothing is given.
pub fn num_opt(name: &'static str) -> InputSpec {
    spec(name, Kind::Number, None).opt()
}
pub fn int(name: &'static str, d: i64) -> InputSpec {
    spec(name, Kind::Int, Some(Value::Int(d)))
}
pub fn boolean(name: &'static str, d: bool) -> InputSpec {
    spec(name, Kind::Bool, Some(Value::Bool(d)))
}
pub fn text(name: &'static str, d: &str) -> InputSpec {
    spec(name, Kind::Text, Some(Value::Text(d.to_string())))
}
pub fn pt(name: &'static str, x: f64, y: f64, z: f64) -> InputSpec {
    spec(name, Kind::Point, Some(Value::Point(DVec3::new(x, y, z))))
}
pub fn geo(name: &'static str) -> InputSpec {
    spec(name, Kind::Geometry, None)
}
pub fn any(name: &'static str) -> InputSpec {
    spec(name, Kind::Any, None)
}
pub fn any_d(name: &'static str, d: Value) -> InputSpec {
    spec(name, Kind::Any, Some(d))
}
pub fn tree(name: &'static str) -> InputSpec {
    spec(name, Kind::Tree, None)
}
/// A tree input whose items default to `d` when nothing is wired.
pub fn tree_d(name: &'static str, kind: Kind, d: Value) -> InputSpec {
    spec(name, kind, Some(d))
}

/// How serious a node's message is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    /// `print` from a script.
    Info,
    Warning,
    Error,
}

/// A message a node left during a solve (shown on the canvas and in the MCP report).
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub level: Level,
    pub message: String,
}

/// Document objects a solve may read, by reference (`object_key`): taken on the GUI thread
/// so the solve can run on another one.
pub type Objects = Arc<HashMap<String, Arc<Geometry>>>;

/// How a reference to a document object (an id, `#id` or a name) is looked up.
pub fn object_key(r: &Value) -> Option<String> {
    match r {
        Value::Int(_) | Value::Number(_) => Some(format!("#{}", as_int(r).max(0))),
        Value::Text(t) => {
            let t = t.trim();
            Some(match t.trim_start_matches('#').parse::<u64>() {
                Ok(id) => format!("#{id}"),
                Err(_) => t.to_string(),
            })
        }
        _ => None,
    }
}

/// Every object of `doc`, by id and by name.
pub fn all_objects(doc: &Document) -> HashMap<String, Arc<Geometry>> {
    let mut m = HashMap::new();
    for o in doc.objects() {
        let g = Arc::new(o.geometry.clone());
        if let Some(name) = o.attributes.name.as_deref() {
            m.entry(name.to_string()).or_insert_with(|| g.clone());
        }
        m.insert(format!("#{}", o.id), g);
    }
    m
}

/// The objects a definition reads: those named in its Geometry parameters, or the whole
/// document when a reference is computed (wired) or hidden in a cluster.
pub fn objects_for(graph: &super::graph::Graph, doc: &Document) -> HashMap<String, Arc<Geometry>> {
    let geometry_nodes: Vec<&super::graph::Node> = graph.nodes.iter().filter(|n| n.kind == "Geometry").collect();
    if !graph.clusters.is_empty() || geometry_nodes.iter().any(|n| graph.source_of(&n.uid, 0).is_some()) {
        return all_objects(doc);
    }
    let mut m = HashMap::new();
    for n in geometry_nodes {
        let Some(v) = n.values.get("Objects").or_else(|| n.values.get("0")) else { continue };
        let refs = match super::data::Value::from_json(v) {
            Value::List(items) => items,
            one => vec![one],
        };
        for r in refs {
            if let (Some(k), Some(g)) = (object_key(&r), geo::find_object(doc, &r)) {
                m.insert(k, Arc::new(g));
            }
        }
    }
    m
}

/// Where previews and bakes go during a solve, and the document geometry parameters read.
#[derive(Default)]
pub struct Ctx {
    pub objects: Objects,
    /// Geometry to show in the viewport, with an optional RGBA colour.
    pub preview: Vec<(Arc<Geometry>, Option<[f64; 4]>)>,
    /// Geometry waiting for the Bake button.
    pub bake: Vec<Arc<Geometry>>,
    /// Messages by node uid.
    pub diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// The node being solved (messages go to it).
    pub current: Option<String>,
}

impl Ctx {
    /// A context that can read every object of `doc`.
    #[cfg(test)]
    pub fn with_doc(doc: &Document) -> Self {
        Self::with_objects(Arc::new(all_objects(doc)))
    }

    pub fn with_objects(objects: Objects) -> Self {
        Self { objects, ..Default::default() }
    }

    /// The document object `r` refers to.
    pub fn object(&self, r: &Value) -> Option<Arc<Geometry>> {
        self.objects.get(&object_key(r)?).cloned()
    }

    fn note(&mut self, level: Level, message: String) {
        let Some(uid) = self.current.clone() else { return };
        let list = self.diagnostics.entry(uid).or_default();
        // element components run once per combination: say each thing once
        if !list.iter().any(|d| d.level == level && d.message == message) {
            list.push(Diagnostic { level, message });
        }
    }

    pub fn error(&mut self, msg: impl Into<String>) {
        self.note(Level::Error, msg.into());
    }

    pub fn warn(&mut self, msg: impl Into<String>) {
        self.note(Level::Warning, msg.into());
    }

    pub fn print(&mut self, msg: impl Into<String>) {
        self.note(Level::Info, msg.into());
    }

}

/// Values a geometry output can be shown or baked from (points become point objects).
pub fn displayable(v: &Value) -> Option<Arc<Geometry>> {
    match v {
        Value::Geometry(g) => Some(g.clone()),
        Value::Point(p) => Some(Arc::new(Geometry::Point(*p))),
        _ => None,
    }
}

/// The values passed to an element component, already cast to the input kinds.
/// An optional input that carries nothing is `Value::Null`.
pub struct Args<'v>(pub &'v [Value]);

impl Args<'_> {
    pub fn get(&self, i: usize) -> &Value {
        self.0.get(i).unwrap_or(&Value::Null)
    }
    pub fn num(&self, i: usize) -> f64 {
        as_number(self.get(i))
    }
    pub fn int(&self, i: usize) -> i64 {
        as_int(self.get(i))
    }
    pub fn flag(&self, i: usize) -> bool {
        truthy(self.get(i))
    }
    pub fn text(&self, i: usize) -> String {
        match self.get(i) {
            Value::Null => String::new(),
            v => v.to_text(),
        }
    }
    pub fn pt(&self, i: usize) -> DVec3 {
        as_point_or(self.get(i), DVec3::ZERO)
    }
    pub fn pt_or(&self, i: usize, d: DVec3) -> DVec3 {
        as_point_or(self.get(i), d)
    }
    /// A direction: the value when it is a point/vector, `d` otherwise.
    pub fn dir(&self, i: usize, d: DVec3) -> DVec3 {
        as_point_or(self.get(i), d)
    }
    pub fn geo(&self, i: usize) -> Option<&Geometry> {
        self.get(i).as_geometry()
    }
    pub fn curve(&self, i: usize) -> Option<NurbsCurve> {
        geo::as_curve(self.get(i))
    }
    pub fn faces(&self, i: usize) -> Option<Vec<tri_geom::Face>> {
        self.geo(i).and_then(geo::faces)
    }
    pub fn brep(&self, i: usize) -> Option<Brep> {
        self.geo(i).and_then(geo::to_brep)
    }
    pub fn surface(&self, i: usize) -> Option<NurbsSurface> {
        self.geo(i).and_then(geo::first_surface)
    }
    pub fn mesh(&self, i: usize) -> Option<Mesh> {
        self.geo(i).and_then(geo::to_mesh)
    }
}

/// What one output of an element call produces.
#[derive(Clone, Debug)]
pub enum Out {
    /// Nothing for this output (Python `None`).
    Skip,
    One(Value),
    /// A list result: every item goes into the branch.
    Many(Vec<Value>),
}

/// Result of an element call: `None` skips the combination entirely.
pub type Res = Option<Vec<Out>>;

pub fn one(v: Value) -> Res {
    Some(vec![Out::One(v)])
}
pub fn many(v: Vec<Value>) -> Res {
    Some(vec![Out::Many(v)])
}
pub fn outs(v: Vec<Out>) -> Res {
    Some(v)
}
pub fn n(x: f64) -> Value {
    Value::Number(x)
}
pub fn p(v: DVec3) -> Value {
    Value::Point(v)
}
pub fn g(geometry: Geometry) -> Value {
    Value::geometry(geometry)
}
pub fn gc(c: NurbsCurve) -> Value {
    Value::geometry(Geometry::Curve(c))
}
/// A geometry result, or nothing.
pub fn og(geometry: Option<Geometry>) -> Res {
    geometry.map(|x| one(g(x))).unwrap_or(None)
}

pub type ElemFn = fn(&mut Ctx, &Args) -> Res;
pub type TreeFn = fn(&mut Ctx, &[DataTree]) -> Vec<DataTree>;

#[derive(Clone, Copy)]
pub enum Func {
    Element(ElemFn),
    Tree(TreeFn),
}

#[derive(Clone)]
pub struct Component {
    pub name: &'static str,
    pub category: &'static str,
    pub inputs: Vec<InputSpec>,
    pub outputs: Vec<&'static str>,
    pub func: Func,
    pub description: &'static str,
}

/// Every component by name.
#[derive(Clone, Default)]
pub struct Registry {
    pub components: BTreeMap<&'static str, Component>,
}

impl Registry {
    pub fn get(&self, name: &str) -> Option<&Component> {
        self.components.get(name)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.components.len()
    }


    #[cfg(test)]
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.components.keys().copied()
    }

    pub fn elem(&mut self, name: &'static str, category: &'static str, inputs: Vec<InputSpec>, outputs: &[&'static str], f: ElemFn) -> &mut Component {
        self.add(name, category, inputs, outputs, Func::Element(f))
    }

    pub fn tree(&mut self, name: &'static str, category: &'static str, inputs: Vec<InputSpec>, outputs: &[&'static str], f: TreeFn) -> &mut Component {
        self.add(name, category, inputs, outputs, Func::Tree(f))
    }

    fn add(&mut self, name: &'static str, category: &'static str, inputs: Vec<InputSpec>, outputs: &[&'static str], func: Func) -> &mut Component {
        let c = Component { name, category, inputs, outputs: outputs.to_vec(), func, description: "" };
        self.components.insert(name, c);
        self.components.get_mut(name).unwrap()
    }
}

impl Component {
    pub fn describe(&mut self, d: &'static str) -> &mut Self {
        self.description = d;
        self
    }
}

fn build() -> Registry {
    let mut r = Registry::default();
    params::register(&mut r);
    vector::register(&mut r);
    populate::register(&mut r);
    voronoi::register(&mut r);
    math::register(&mut r);
    logic::register(&mut r);
    sets::register(&mut r);
    domain::register(&mut r);
    string::register(&mut r);
    curve::register(&mut r);
    surface::register(&mut r);
    solid::register(&mut r);
    transform::register(&mut r);
    analysis::register(&mut r);
    mesh::register(&mut r);
    output::register(&mut r);
    r
}

/// The shared component library.
pub fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(build)
}

// ---------------------------------------------------------- tree helpers

/// First item of a tree as a number, or `d`.
pub fn first_num(t: &DataTree, d: f64) -> f64 {
    t.first().map(|v| super::data::as_number_or(v, d)).unwrap_or(d)
}

pub fn first_int(t: &DataTree, d: i64) -> i64 {
    t.first().map(|v| super::data::as_int_or(v, d)).unwrap_or(d)
}

pub fn first_bool(t: &DataTree, d: bool) -> bool {
    t.first().map(truthy).unwrap_or(d)
}

pub fn first_text(t: &DataTree, d: &str) -> String {
    t.first().map(|v| v.to_text()).unwrap_or_else(|| d.to_string())
}

/// One output tree.
pub fn t1(t: DataTree) -> Vec<DataTree> {
    vec![t]
}
