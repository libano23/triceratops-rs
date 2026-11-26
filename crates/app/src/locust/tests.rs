//! Locust tests, ported from the Python Locust's test suite (tests/test_locust.py), plus
//! checks for the Rust specifics.

use std::collections::HashMap;
use std::f64::consts::PI;

use glam::DVec3;
use serde_json::{json, Map};

use super::components::{num, registry, Ctx, Registry};
use super::data::{match_trees, DataTree, Value};
use super::engine::{solve, solve_with};
use super::graph::Graph;

fn vals(pairs: &[(&str, serde_json::Value)]) -> Map<String, serde_json::Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn node(g: &mut Graph, kind: &str, pairs: &[(&str, serde_json::Value)]) -> String {
    g.add_node_with(kind, 0.0, 0.0, vals(pairs))
}

fn run(g: &Graph) -> HashMap<String, Vec<DataTree>> {
    solve(g, &mut Ctx::default())
}

fn first_num(t: &DataTree) -> f64 {
    super::data::as_number(t.first().expect("a value"))
}

fn first_pt(t: &DataTree) -> DVec3 {
    super::data::as_point(t.first().expect("a value"))
}

fn nums(t: &DataTree) -> Vec<f64> {
    t.items().iter().map(super::data::as_number).collect()
}

fn solve_node(kind: &str, pairs: &[(&str, serde_json::Value)]) -> Vec<DataTree> {
    let mut g = Graph::new();
    let n = node(&mut g, kind, pairs);
    run(&g).remove(&n).unwrap()
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    (a - b).abs() <= rel * b.abs().max(1e-12)
}

// --------------------------------------------------------------- data trees

#[test]
fn tree_flatten_and_graft() {
    let t = DataTree::from_branches([(vec![0], vec![Value::Int(1), Value::Int(2)]), (vec![1], vec![Value::Int(3)])]);
    assert_eq!(nums(&t.flatten()), vec![1.0, 2.0, 3.0]);
    assert_eq!(t.flatten().paths(), vec![Vec::<i64>::new()]);
    let gr = t.graft();
    assert_eq!(gr.branch_count(), 3);
    assert_eq!(gr.count(), 3);
}

#[test]
fn tree_simplify_drops_a_leading_zero() {
    let t = DataTree::from_branches([(vec![0, 0], vec![Value::Int(1)]), (vec![0, 1], vec![Value::Int(2)])]);
    assert_eq!(t.simplify().paths(), vec![vec![0], vec![1]]);
}

#[test]
fn match_broadcasts_a_single_branch() {
    let a = DataTree::single(Value::Number(10.0));
    let b = DataTree::from_list(vec![Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)]);
    let combos = match_trees(&[a, b]);
    assert_eq!(combos.len(), 3);
    assert!(combos.iter().all(|c| super::data::as_number(&c.values[0]) == 10.0));
    assert_eq!(combos.iter().map(|c| super::data::as_number(&c.values[1])).collect::<Vec<_>>(), vec![1.0, 2.0, 3.0]);
}

#[test]
fn match_repeats_the_shorter_list() {
    let a = DataTree::from_list(vec![Value::Number(1.0), Value::Number(2.0)]);
    let b = DataTree::from_list(vec![Value::Number(10.0), Value::Number(20.0), Value::Number(30.0)]);
    let combos = match_trees(&[a, b]);
    assert_eq!(combos.iter().map(|c| super::data::as_number(&c.values[0])).collect::<Vec<_>>(), vec![1.0, 2.0, 2.0]);
}

#[test]
fn match_keeps_paths_apart() {
    let a = DataTree::from_branches([(vec![0], vec![Value::Number(1.0)]), (vec![1], vec![Value::Number(2.0)])]);
    let b = DataTree::from_branches([(vec![0], vec![Value::Number(10.0)]), (vec![1], vec![Value::Number(20.0)])]);
    let combos = match_trees(&[a, b]);
    let got: HashMap<Vec<i64>, Vec<f64>> = combos.iter().map(|c| (c.path.clone(), c.values.iter().map(super::data::as_number).collect())).collect();
    assert_eq!(got[&vec![0]], vec![1.0, 10.0]);
    assert_eq!(got[&vec![1]], vec![2.0, 20.0]);
}

#[test]
fn tree_trim_shift_and_partition() {
    let t = DataTree::from_branches([(vec![0, 0], (1..=4).map(Value::Int).collect())]);
    assert_eq!(t.trim(1).paths(), vec![vec![0]]);
    assert_eq!(t.shift_paths(2).paths(), vec![vec![0, 2]]);
    let parts = t.partition(2, Some(2));
    assert_eq!(nums(&DataTree::from_list(parts.branch(&[0, 0, 0]))), vec![1.0, 2.0]);
    assert_eq!(nums(&DataTree::from_list(parts.branch(&[0, 0, 1]))), vec![3.0, 4.0]);
    assert_eq!(t.graft_by(2).branch_count(), 2);
}

#[test]
fn branch_paths_lists_the_paths() {
    let t = DataTree::from_branches([(vec![0], vec![Value::Int(1)]), (vec![2], vec![Value::Int(3)])]);
    let reprs: Vec<String> = t.branch_paths().items().iter().map(|v| v.to_text()).collect();
    assert_eq!(reprs, vec!["[0]", "[2]"]);
}

// ----------------------------------------------------------------- components

#[test]
fn every_component_is_well_formed() {
    let reg = registry();
    assert!(reg.len() >= 214, "{} components", reg.len());
    for (name, c) in &reg.components {
        assert_eq!(c.name, *name);
        assert!(!c.outputs.is_empty(), "{name}");
        let mut names: Vec<&str> = c.inputs.iter().map(|i| i.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), c.inputs.len(), "{name}");
    }
}

#[test]
fn component_library_matches_the_python_one() {
    // every component of the Python Locust exists here, in the same category
    let expected: &[(&str, &[&str])] = &[
        ("Params", &["Number", "Integer", "Boolean", "Text", "Point", "Vector", "Plane", "Geometry", "Colour RGB", "Colour HSV"]),
        ("Vector", &["Deconstruct Point", "Vector 2Pt", "Amplitude", "Unit Z", "Dot Product", "Cross Product", "Unit Vector", "Vector Length", "Vector Add", "Vector Subtract", "Vector Scale", "Vector Angle", "Vector Rotate", "Point Add", "Point Subtract", "Point Average"]),
        ("Math", &["Add", "Subtract", "Multiply", "Divide", "Power", "Modulus", "Negate", "Absolute", "Square Root", "Sine", "Cosine", "Tangent", "Minimum", "Maximum", "Average", "Round", "Clamp", "Remap", "Floor", "Ceiling", "Truncate", "Sign", "Exponential", "Logarithm", "Degrees", "Radians", "Arc Sine", "Arc Cosine", "Arc Tangent", "Factorial", "Sum", "Mean", "Weighted Average"]),
        ("Logic", &["Not", "And", "Or", "Xor", "Larger Than", "Smaller Than", "Equality"]),
        ("Sets", &["Series", "Range", "Flatten", "Graft", "Simplify", "List Length", "List Item", "Reverse", "Shift", "Cull Pattern", "Merge", "Sort", "Partition", "Graft By", "Trim Tree", "Shift Paths", "Deconstruct Path", "Cull Index", "Replace Items", "Insert Items", "Dispatch", "Weave", "Entwine", "List Union", "List Intersection", "List Difference", "Item Index", "Reverse List", "Sub List", "Fibonacci"]),
        ("Curve", &["Line", "Line SDL", "Circle", "Circle 2Pt", "Arc 3Pt", "Arc", "Ellipse", "Rectangle", "Polygon", "Polyline", "Interpolate", "Offset Curve", "Divide Curve", "Curve Length", "Curve End", "Curve Midpoint", "Nurbs Curve", "Offset Curve Loose", "Extend Curve", "Curve Closest Point", "Evaluate Curve", "Curve Control Points", "Rebuild Curve", "Reverse Curve", "Curve Seam", "Join Curves", "Explode", "Divide Distance", "Curve Curvature", "Point On Curve", "Perp Frame", "Curve Domain"]),
        ("Surface", &["Plane Surface", "Box", "Sphere", "Cylinder", "Cone", "Torus", "Extrude", "Loft", "Revolve", "Edge Surface", "Patch", "Cap", "Shell", "Sweep1", "Sweep2", "Extrude Along Curve", "Rail Revolve", "Offset Surface", "Untrim", "Surface from Corners", "Surface from Grid", "Evaluate Surface", "Iso Curve", "Deconstruct Brep", "Join Brep", "Flip Surface", "Pull Curve", "Divide Surface", "Surface Closest Point", "Surface Domain"]),
        ("Solid", &["Union", "Difference", "Intersection", "Solid Union", "Fillet Edge", "Solid Intersection", "Solid Difference", "Create Solid", "Chamfer Edge", "Split Solid", "Wire Cut", "Split Brep"]),
        ("Transform", &["Move", "Rotate", "Scale", "Mirror", "Array Linear", "Array Polar", "Orient", "Project", "Scale NU", "Twist"]),
        ("Analysis", &["Area", "Volume", "Bounding Box", "Centroid", "Distance", "Volume Centroid", "Area Centroid", "Deconstruct Box", "Closest Point", "Surface Area Centroid"]),
        ("Output", &["Preview", "Bake", "Panel", "Cluster Input", "Cluster Output", "Custom Preview"]),
        ("Domain", &["Construct Domain", "Deconstruct Domain", "Bounds"]),
        ("String", &["Concatenate", "Text Join", "Text Split", "Text Length", "Substring", "Replace Text", "Number to Text", "Text to Number"]),
        ("Intersect", &["Curve-Curve Intersection"]),
        ("Mesh", &["Mesh Brep", "Mesh to Brep", "Mesh Vertices", "Mesh Box", "Mesh Sphere", "Deconstruct Mesh"]),
    ];
    let reg = registry();
    let mut total = 0;
    for (cat, names) in expected {
        for n in *names {
            let c = reg.get(n).unwrap_or_else(|| panic!("missing component {n}"));
            assert_eq!(c.category, *cat, "{n}");
            total += 1;
        }
    }
    assert_eq!(total, 214);
}

// -------------------------------------------------------------------- engine

#[test]
fn series_drives_a_list_through_one_number() {
    let mut g = Graph::new();
    let series = node(&mut g, "Series", &[("Start", json!(1.0)), ("Step", json!(1.0)), ("Count", json!(3))]);
    let circle = node(&mut g, "Circle", &[]);
    let extrude = node(&mut g, "Extrude", &[("Distance", json!(2.0))]);
    let volume = node(&mut g, "Volume", &[]);
    g.connect(&series, 0, &circle, 1);
    g.connect(&circle, 0, &extrude, 0);
    g.connect(&extrude, 0, &volume, 0);
    let got = nums(&run(&g)[&volume][0]);
    let want: Vec<f64> = [1.0f64, 2.0, 3.0].iter().map(|r| PI * r * r * 2.0).collect();
    assert_eq!(got.len(), 3);
    for (a, b) in got.iter().zip(&want) {
        assert!((a - b).abs() < 0.005, "{got:?} vs {want:?}");
    }
}

#[test]
fn boolean_union_volume() {
    let mut g = Graph::new();
    let a = node(&mut g, "Box", &[("X", json!(10.0)), ("Y", json!(10.0)), ("Z", json!(10.0))]);
    let b = node(&mut g, "Box", &[("X", json!(10.0)), ("Y", json!(10.0)), ("Z", json!(10.0)), ("Corner", json!([5.0, 0.0, 0.0]))]);
    let u = node(&mut g, "Union", &[]);
    let vol = node(&mut g, "Volume", &[]);
    g.connect(&a, 0, &u, 0);
    g.connect(&b, 0, &u, 1);
    g.connect(&u, 0, &vol, 0);
    assert!(close(first_num(&run(&g)[&vol][0]), 1500.0, 1e-6));
}

#[test]
fn a_component_that_fails_yields_nothing() {
    let mut reg: Registry = registry().clone();
    reg.elem("_RaisesTest", "Test", vec![num("X", 0.0)], &["Y"], |_, _| panic!("boom"));
    let mut g = Graph::new();
    let n = node(&mut g, "_RaisesTest", &[]);
    let out = solve_with(&g, &mut Ctx::default(), &HashMap::new(), &reg);
    assert!(out[&n][0].is_empty());
}

#[test]
fn a_missing_component_yields_nothing() {
    let mut g = Graph::new();
    let n = node(&mut g, "NoSuchComponent", &[]);
    assert!(run(&g)[&n].is_empty());
}

#[test]
fn geometry_parameter_reads_the_document() {
    let mut doc = tri_doc::Document::new();
    let b = super::components::geo::box_solid(DVec3::ZERO, 2.0, 2.0, 2.0).unwrap();
    let id = doc.add(b);
    let mut g = Graph::new();
    let par = node(&mut g, "Geometry", &[("Objects", json!([id]))]);
    let vol = node(&mut g, "Volume", &[]);
    g.connect(&par, 0, &vol, 0);
    let out = solve(&g, &mut Ctx::with_doc(&doc));
    assert!(close(first_num(&out[&vol][0]), 8.0, 1e-6));
    // by "#id" text too
    g.set_value(&par, "Objects", json!(format!("#{id}")));
    let out = solve(&g, &mut Ctx::with_doc(&doc));
    assert!(close(first_num(&out[&vol][0]), 8.0, 1e-6));
}

// --------------------------------------------------------------------- graph

#[test]
fn graph_rejects_a_cycle() {
    let mut g = Graph::new();
    let a = node(&mut g, "Number", &[]);
    let b = node(&mut g, "Add", &[]);
    assert!(g.connect(&a, 0, &b, 0));
    assert!(!g.connect(&b, 0, &b, 0));
    assert!(!g.connect(&b, 0, &a, 0));
}

#[test]
fn graph_round_trips_through_json() {
    let mut g = Graph::new();
    let a = node(&mut g, "Box", &[("X", json!(1.0)), ("Y", json!(2.0)), ("Z", json!(3.0))]);
    let b = node(&mut g, "Volume", &[]);
    g.connect(&a, 0, &b, 0);
    let text = serde_json::to_string(&g.to_json()).unwrap();
    let back = Graph::from_json(&serde_json::from_str(&text).unwrap());
    assert_eq!(back.nodes.len(), 2);
    assert_eq!(back.connections.len(), 1);
    assert_eq!(back.node(&a).unwrap().values["Y"], json!(2.0));
    assert_eq!(back, g);
}

#[test]
fn reads_a_python_locust_file() {
    // the JSON the Python Locust writes for its example (uids, tuples as lists)
    let text = r##"{"nodes": [
        {"uid": "a1", "type": "Number", "x": 20.0, "y": 40.0, "values": {"Value": 6.0}, "nickname": "Radius", "group": null, "disabled": false},
        {"uid": "b2", "type": "Circle", "x": 320.0, "y": 40.0, "values": {"Center": [1.0, 2.0, 0.0]}, "nickname": null, "group": null, "disabled": false},
        {"uid": "c3", "type": "Curve Length", "x": 560.0, "y": 90.0, "values": {}, "nickname": null, "group": null, "disabled": false}],
      "connections": [{"src": "a1", "src_port": 0, "dst": "b2", "dst_port": 1}, {"src": "b2", "src_port": 0, "dst": "c3", "dst_port": 0}],
      "groups": [], "comments": [{"uid": "k", "text": "hi", "x": 0, "y": 0, "w": 220, "h": 80, "color": "#2b2c30"}], "clusters": {}}"##;
    let g = Graph::from_json(&serde_json::from_str(text).unwrap());
    let out = run(&g);
    assert!(close(first_num(&out["c3"][0]), 2.0 * PI * 6.0, 1e-6));
    assert_eq!(g.comments[0].text, "hi");
}

#[test]
fn groups_comments_and_duplicate_round_trip() {
    let mut g = Graph::new();
    let a = node(&mut g, "Number", &[]);
    let b = node(&mut g, "Add", &[]);
    g.connect(&a, 0, &b, 0);
    g.add_group(&[a.clone(), b.clone()], "G");
    g.add_comment(0.0, 0.0, "note");
    let copies = g.duplicate(std::slice::from_ref(&a), 100.0, 0.0);
    assert_eq!(copies.len(), 1);
    let back = Graph::from_json(&g.to_json());
    assert_eq!(back.groups.len(), 1);
    assert_eq!(back.groups[0].name, "G");
    assert_eq!(back.comments[0].text, "note");
}

// ------------------------------------------------------------ new components

#[test]
fn domain_and_bounds() {
    let d = solve_node("Construct Domain", &[("Start", json!(2.0)), ("End", json!(5.0))]);
    assert!(matches!(d[0].first(), Some(Value::Pair(a, b)) if *a == 2.0 && *b == 5.0));
    let mut g = Graph::new();
    let s = node(&mut g, "Series", &[("Start", json!(3.0)), ("Step", json!(1.0)), ("Count", json!(3))]);
    let bounds = node(&mut g, "Bounds", &[]);
    g.connect(&s, 0, &bounds, 0);
    assert!(matches!(run(&g)[&bounds][0].first(), Some(Value::Pair(a, b)) if *a == 3.0 && *b == 5.0));
}

#[test]
fn math_and_string_components() {
    assert_eq!(first_num(&solve_node("Floor", &[("Value", json!(2.9))])[0]), 2.0);
    assert_eq!(first_num(&solve_node("Ceiling", &[("Value", json!(2.1))])[0]), 3.0);
    assert!((first_num(&solve_node("Exponential", &[("Value", json!(0.0))])[0]) - 1.0).abs() < 1e-12);
    assert_eq!(solve_node("Concatenate", &[("A", json!("a")), ("B", json!("b"))])[0].first().unwrap().to_text(), "ab");
    assert_eq!(solve_node("Number to Text", &[("Value", json!(1.23456)), ("Decimals", json!(2))])[0].first().unwrap().to_text(), "1.23");
    assert_eq!(first_num(&solve_node("Round", &[("Value", json!(2.5))])[0]), 2.0, "Python rounds halves to even");
    assert_eq!(first_num(&solve_node("Sine", &[("Value", json!(90.0))])[0]), 1.0, "trigonometry in degrees");
}

#[test]
fn string_split_and_join() {
    let mut g = Graph::new();
    let t = node(&mut g, "Text", &[("Value", json!("a,b,c"))]);
    let split = node(&mut g, "Text Split", &[]);
    let join = node(&mut g, "Text Join", &[("Separator", json!("-"))]);
    g.connect(&t, 0, &split, 0);
    g.connect(&split, 0, &join, 0);
    let out = run(&g);
    assert_eq!(out[&split][0].items().iter().map(|v| v.to_text()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    assert_eq!(out[&join][0].first().unwrap().to_text(), "a-b-c");
}

#[test]
fn list_set_operations() {
    let mut g = Graph::new();
    let a = node(&mut g, "Series", &[("Start", json!(0.0)), ("Step", json!(1.0)), ("Count", json!(4))]);
    let b = node(&mut g, "Series", &[("Start", json!(2.0)), ("Step", json!(1.0)), ("Count", json!(4))]);
    let inter = node(&mut g, "List Intersection", &[]);
    let diff = node(&mut g, "List Difference", &[]);
    let uni = node(&mut g, "List Union", &[]);
    for n in [&inter, &diff, &uni] {
        g.connect(&a, 0, n, 0);
        g.connect(&b, 0, n, 1);
    }
    let out = run(&g);
    assert_eq!(nums(&out[&inter][0]), vec![2.0, 3.0]);
    assert_eq!(nums(&out[&diff][0]), vec![0.0, 1.0]);
    assert_eq!(nums(&out[&uni][0]), vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
}

#[test]
fn dispatch_weave_and_cull() {
    let mut g = Graph::new();
    let s = node(&mut g, "Series", &[("Start", json!(0.0)), ("Step", json!(1.0)), ("Count", json!(4))]);
    let pat = node(&mut g, "Boolean", &[("Value", json!(true))]);
    let f = node(&mut g, "Boolean", &[("Value", json!(false))]);
    let pattern = node(&mut g, "Merge", &[]);
    g.connect(&pat, 0, &pattern, 0);
    g.connect(&f, 0, &pattern, 1);
    let disp = node(&mut g, "Dispatch", &[]);
    g.connect(&s, 0, &disp, 0);
    g.connect(&pattern, 0, &disp, 1);
    let weave = node(&mut g, "Weave", &[]);
    g.connect(&disp, 0, &weave, 0);
    g.connect(&disp, 1, &weave, 1);
    g.connect(&pattern, 0, &weave, 2);
    let out = run(&g);
    assert_eq!(out[&disp].len(), 2);
    assert_eq!(nums(&out[&disp][0]), vec![0.0, 2.0]);
    assert_eq!(nums(&out[&disp][1]), vec![1.0, 3.0]);
    assert_eq!(nums(&out[&weave][0]), vec![0.0, 1.0, 2.0, 3.0]);
}

#[test]
fn entwine_creates_branches() {
    let mut g = Graph::new();
    let a = node(&mut g, "Series", &[("Start", json!(0.0)), ("Step", json!(1.0)), ("Count", json!(2))]);
    let b = node(&mut g, "Series", &[("Start", json!(10.0)), ("Step", json!(1.0)), ("Count", json!(2))]);
    let e = node(&mut g, "Entwine", &[]);
    g.connect(&a, 0, &e, 0);
    g.connect(&b, 0, &e, 1);
    let out = run(&g);
    assert_eq!(out[&e][0].branch_count(), 2);
    assert_eq!(nums(&out[&e][0]), vec![0.0, 1.0, 10.0, 11.0]);
}

#[test]
fn list_item_shift_sort_and_sublist() {
    let mut g = Graph::new();
    let s = node(&mut g, "Series", &[("Start", json!(0.0)), ("Step", json!(1.0)), ("Count", json!(6))]);
    let item = node(&mut g, "List Item", &[("Index", json!(-1))]);
    let shift = node(&mut g, "Shift", &[("Offset", json!(2)), ("Wrap", json!(true))]);
    let sub = node(&mut g, "Sub List", &[("From", json!(2)), ("Count", json!(3))]);
    let rev = node(&mut g, "Reverse", &[]);
    let sort = node(&mut g, "Sort", &[]);
    for n in [&item, &shift, &sub, &rev] {
        g.connect(&s, 0, n, 0);
    }
    g.connect(&rev, 0, &sort, 0);
    let out = run(&g);
    assert_eq!(nums(&out[&item][0]), vec![5.0]);
    assert_eq!(nums(&out[&shift][0]), vec![2.0, 3.0, 4.0, 5.0, 0.0, 1.0]);
    assert_eq!(nums(&out[&sub][0]), vec![2.0, 3.0, 4.0]);
    assert_eq!(nums(&out[&sort][0]), vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
}

#[test]
fn solid_intersection_of_two_boxes() {
    let mut g = Graph::new();
    let a = node(&mut g, "Box", &[("X", json!(10.0)), ("Y", json!(10.0)), ("Z", json!(10.0))]);
    let b = node(&mut g, "Box", &[("X", json!(10.0)), ("Y", json!(10.0)), ("Z", json!(10.0)), ("Corner", json!([5.0, 0.0, 0.0]))]);
    let ent = node(&mut g, "Entwine", &[]);
    g.connect(&a, 0, &ent, 0);
    g.connect(&b, 0, &ent, 1);
    let inter = node(&mut g, "Solid Intersection", &[]);
    let vol = node(&mut g, "Volume", &[]);
    g.connect(&ent, 0, &inter, 0);
    g.connect(&inter, 0, &vol, 0);
    assert!(close(first_num(&run(&g)[&vol][0]), 500.0, 1e-6));
}

#[test]
fn deconstruct_box() {
    let mut g = Graph::new();
    let b = node(&mut g, "Box", &[("X", json!(2.0)), ("Y", json!(3.0)), ("Z", json!(4.0))]);
    let d = node(&mut g, "Deconstruct Box", &[]);
    g.connect(&b, 0, &d, 0);
    let out = run(&g);
    assert!((first_pt(&out[&d][3]) - DVec3::new(2.0, 3.0, 4.0)).length() < 1e-9);
    assert!((first_pt(&out[&d][2]) - DVec3::new(1.0, 1.5, 2.0)).length() < 1e-9);
}

#[test]
fn mesh_round_trip_component() {
    let mut g = Graph::new();
    let b = node(&mut g, "Box", &[("X", json!(1.0)), ("Y", json!(1.0)), ("Z", json!(1.0))]);
    let m = node(&mut g, "Mesh Brep", &[]);
    let back = node(&mut g, "Mesh to Brep", &[]);
    let vol = node(&mut g, "Volume", &[]);
    g.connect(&b, 0, &m, 0);
    g.connect(&m, 0, &back, 0);
    g.connect(&back, 0, &vol, 0);
    assert!(close(first_num(&run(&g)[&vol][0]), 1.0, 1e-6));
}

// ------------------------------------------------------------------- clusters

#[test]
fn cluster_solves_and_exposes_ports() {
    let mut g = Graph::new();
    let n = node(&mut g, "Number", &[("Value", json!(3.0))]);
    let circle = node(&mut g, "Circle", &[]);
    let extrude = node(&mut g, "Extrude", &[("Distance", json!(2.0))]);
    let vol = node(&mut g, "Volume", &[]);
    g.connect(&n, 0, &circle, 1);
    g.connect(&circle, 0, &extrude, 0);
    g.connect(&extrude, 0, &vol, 0);
    let cl = g.make_cluster(&[circle, extrude], "Shaft").unwrap();
    let cn = g.node(&cl).unwrap().clone();
    assert!(g.is_cluster(&cn));
    let (ins, outs) = g.cluster_ports(&cn);
    assert!(!ins.is_empty());
    assert_eq!(outs.len(), 1);
    assert!((first_num(&run(&g)[&vol][0]) - PI * 9.0 * 2.0).abs() < 0.005);
}

#[test]
fn cluster_moved_into_another_graph_solves() {
    let mut inner = Graph::new();
    let a = node(&mut inner, "Box", &[("X", json!(2.0)), ("Y", json!(2.0)), ("Z", json!(2.0))]);
    let b = node(&mut inner, "Box", &[("X", json!(2.0)), ("Y", json!(2.0)), ("Z", json!(2.0)), ("Corner", json!([1.0, 0.0, 0.0]))]);
    let u = node(&mut inner, "Union", &[]);
    let vol = node(&mut inner, "Volume", &[]);
    inner.connect(&a, 0, &u, 0);
    inner.connect(&b, 0, &u, 1);
    inner.connect(&u, 0, &vol, 0);
    let sub = inner.make_cluster(&[a, b, u], "Pair").unwrap();
    // Booleans are exact to the modelling tolerance (0.001), not to round-off
    assert!(close(first_num(&run(&inner)[&vol][0]), 12.0, 2e-4));
    let mut outer = Graph::new();
    outer.clusters = inner.clusters.clone();
    outer.nodes.push(inner.node(&sub).unwrap().clone());
    let v2 = node(&mut outer, "Volume", &[]);
    outer.connect(&sub, 0, &v2, 0);
    assert!(close(first_num(&run(&outer)[&v2][0]), 12.0, 2e-4));
}

#[test]
fn disabled_node_bypasses_its_input() {
    let mut g = Graph::new();
    let c = node(&mut g, "Circle", &[("Radius", json!(4.0))]);
    let e = node(&mut g, "Extrude", &[("Distance", json!(5.0))]);
    let vol = node(&mut g, "Volume", &[]);
    g.connect(&c, 0, &e, 0);
    g.connect(&e, 0, &vol, 0);
    assert!((first_num(&run(&g)[&vol][0]) - PI * 16.0 * 5.0).abs() < 0.01);
    g.node_mut(&e).unwrap().disabled = true;
    assert!(first_num(&run(&g)[&vol][0]).abs() < 1e-6);
}

#[test]
fn solving_every_component_alone_does_not_crash() {
    let mut g = Graph::new();
    let nodes: Vec<(String, &str)> = registry().names().map(|n| (g.add_node(n, 0.0, 0.0), n)).collect();
    let out = run(&g);
    for (uid, name) in nodes {
        assert!(out.contains_key(&uid), "{name}");
    }
    serde_json::to_string(&g.to_json()).unwrap();
}

// ----------------------------------------------------------------- geometry

#[test]
fn point_on_curve_and_divide_surface() {
    let mut g = Graph::new();
    let c = node(&mut g, "Circle", &[("Radius", json!(10.0))]);
    let mid = node(&mut g, "Point On Curve", &[]);
    let div = node(&mut g, "Divide Curve", &[("Count", json!(8))]);
    g.connect(&c, 0, &mid, 0);
    g.connect(&c, 0, &div, 0);
    let out = run(&g);
    assert!((first_pt(&out[&mid][0]).length() - 10.0).abs() < 1e-9);
    // a closed curve: 8 points, all on the circle
    assert_eq!(out[&div][0].count(), 7);
    assert!(out[&div][0].items().iter().all(|v| (super::data::as_point(v).length() - 10.0).abs() < 1e-6));

    let mut g2 = Graph::new();
    let s = node(&mut g2, "Plane Surface", &[("Width", json!(10.0)), ("Height", json!(10.0))]);
    let dv = node(&mut g2, "Divide Surface", &[("Count U", json!(3)), ("Count V", json!(3))]);
    g2.connect(&s, 0, &dv, 0);
    assert_eq!(run(&g2)[&dv][0].count(), 9);
}

#[test]
fn surface_closest_point_and_mesh_volume() {
    let mut g = Graph::new();
    let sph = node(&mut g, "Sphere", &[("Radius", json!(5.0))]);
    let cp = node(&mut g, "Surface Closest Point", &[("Point", json!([10.0, 0.0, 0.0]))]);
    g.connect(&sph, 0, &cp, 0);
    assert!((first_pt(&run(&g)[&cp][0]) - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-4);

    let mut mb = Graph::new();
    let m = node(&mut mb, "Mesh Box", &[("X", json!(2.0)), ("Y", json!(2.0)), ("Z", json!(2.0))]);
    let vol = node(&mut mb, "Volume", &[]);
    mb.connect(&m, 0, &vol, 0);
    assert!(close(first_num(&run(&mb)[&vol][0]), 8.0, 1e-6));

    let mut ms = Graph::new();
    let m = node(&mut ms, "Mesh Sphere", &[("Radius", json!(1.0))]);
    let vol = node(&mut ms, "Volume", &[]);
    ms.connect(&m, 0, &vol, 0);
    assert!(close(first_num(&run(&ms)[&vol][0]), 4.0 / 3.0 * PI, 1e-2));
}

#[test]
fn colour_and_fibonacci() {
    assert!(matches!(solve_node("Colour RGB", &[("Red", json!(0.5)), ("Green", json!(0.25)), ("Blue", json!(0.0))])[0].first(), Some(Value::Colour(c)) if c[0] == 0.5));
    assert_eq!(nums(&solve_node("Fibonacci", &[("Count", json!(8))])[0]), vec![0.0, 1.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0]);
}

#[test]
fn custom_preview_and_bake_collect_geometry() {
    let mut g = Graph::new();
    let b = node(&mut g, "Box", &[]);
    let col = node(&mut g, "Colour RGB", &[("Red", json!(0.1)), ("Green", json!(0.9)), ("Blue", json!(0.3))]);
    let pr = node(&mut g, "Custom Preview", &[]);
    let bake = node(&mut g, "Bake", &[]);
    g.connect(&b, 0, &pr, 0);
    g.connect(&col, 0, &pr, 1);
    g.connect(&b, 0, &bake, 0);
    let mut ctx = Ctx::default();
    solve(&g, &mut ctx);
    assert_eq!(ctx.preview.len(), 1);
    let c = ctx.preview[0].1.unwrap();
    assert!((c[0] - 0.1).abs() < 1e-9 && (c[1] - 0.9).abs() < 1e-9 && (c[2] - 0.3).abs() < 1e-9);
    assert_eq!(ctx.bake.len(), 1);
}

#[test]
fn perp_frame_and_domains() {
    let mut g = Graph::new();
    let c = node(&mut g, "Circle", &[("Radius", json!(4.0))]);
    let f = node(&mut g, "Perp Frame", &[]);
    let cd = node(&mut g, "Curve Domain", &[]);
    g.connect(&c, 0, &f, 0);
    g.connect(&c, 0, &cd, 0);
    let out = run(&g);
    assert!((first_pt(&out[&f][0]).length() - 4.0).abs() < 1e-9);
    assert!(matches!(out[&cd][0].first(), Some(Value::Pair(lo, hi)) if hi > lo));
    let mut g2 = Graph::new();
    let s = node(&mut g2, "Plane Surface", &[("Width", json!(10.0)), ("Height", json!(10.0))]);
    let sd = node(&mut g2, "Surface Domain", &[]);
    g2.connect(&s, 0, &sd, 0);
    assert!(matches!(run(&g2)[&sd][0].first(), Some(Value::Pair(lo, hi)) if hi > lo));
}

#[test]
fn curve_tools() {
    let mut g = Graph::new();
    let s = node(&mut g, "Series", &[("Start", json!(0.0)), ("Step", json!(1.0)), ("Count", json!(5))]);
    let zero = node(&mut g, "Number", &[("Value", json!(0.0))]);
    let pts = node(&mut g, "Point", &[]);
    g.connect(&s, 0, &pts, 0);
    g.connect(&zero, 0, &pts, 1);
    let pl = node(&mut g, "Polyline", &[]);
    g.connect(&pts, 0, &pl, 0);
    let len = node(&mut g, "Curve Length", &[]);
    g.connect(&pl, 0, &len, 0);
    let ex = node(&mut g, "Explode", &[]);
    g.connect(&pl, 0, &ex, 0);
    let ends = node(&mut g, "Curve End", &[]);
    g.connect(&pl, 0, &ends, 0);
    let interp = node(&mut g, "Interpolate", &[("Periodic", json!(true))]);
    g.connect(&pts, 0, &interp, 0);
    let out = run(&g);
    assert!((first_num(&out[&len][0]) - 4.0).abs() < 1e-9);
    assert_eq!(out[&ex][0].count(), 4);
    assert!((first_pt(&out[&ends][1]) - DVec3::new(4.0, 0.0, 0.0)).length() < 1e-9);
    // a closed interpolated curve through collinear points is degenerate, but a square works
    let sq = [DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::Y];
    let c = super::components::geo::interpolate_closed(&sq).unwrap();
    assert!(c.is_closed());
    for q in sq {
        assert!((c.point_at(c.closest_parameter(q)) - q).length() < 1e-6, "passes through {q}");
    }
}

#[test]
fn surfaces_and_solids() {
    let vol_of = |kind: &str, pairs: &[(&str, serde_json::Value)]| -> f64 {
        let mut g = Graph::new();
        let a = node(&mut g, kind, pairs);
        let v = node(&mut g, "Volume", &[]);
        g.connect(&a, 0, &v, 0);
        first_num(&run(&g)[&v][0])
    };
    assert!(close(vol_of("Sphere", &[("Radius", json!(2.0))]), 4.0 / 3.0 * PI * 8.0, 1e-4));
    assert!(close(vol_of("Cylinder", &[("Radius", json!(1.0)), ("Height", json!(3.0))]), PI * 3.0, 1e-4));
    assert!(close(vol_of("Cone", &[("Radius 1", json!(1.0)), ("Radius 2", json!(0.0)), ("Height", json!(3.0))]), PI, 1e-3));
    assert!(close(vol_of("Torus", &[("Major Radius", json!(5.0)), ("Minor Radius", json!(1.0))]), 2.0 * PI * PI * 5.0, 1e-3));

    // loft two circles into a tube, revolve a line into a disc-like surface
    let mut g = Graph::new();
    let c1 = node(&mut g, "Circle", &[("Radius", json!(2.0))]);
    let c2 = node(&mut g, "Circle", &[("Radius", json!(2.0)), ("Center", json!([0.0, 0.0, 4.0]))]);
    let m = node(&mut g, "Merge", &[]);
    g.connect(&c1, 0, &m, 0);
    g.connect(&c2, 0, &m, 1);
    let loft = node(&mut g, "Loft", &[]);
    g.connect(&m, 0, &loft, 0);
    let area = node(&mut g, "Area", &[]);
    g.connect(&loft, 0, &area, 0);
    let out = run(&g);
    assert!(close(first_num(&out[&area][0]), 2.0 * PI * 2.0 * 4.0, 1e-4));

    let mut g = Graph::new();
    let b = node(&mut g, "Box", &[]);
    let sh = node(&mut g, "Shell", &[("Thickness", json!(1.0))]);
    let v = node(&mut g, "Volume", &[]);
    g.connect(&b, 0, &sh, 0);
    g.connect(&sh, 0, &v, 0);
    assert!(close(first_num(&run(&g)[&v][0]), 1000.0 - 512.0, 1e-6));
}

#[test]
fn transforms_and_arrays() {
    let mut g = Graph::new();
    let b = node(&mut g, "Box", &[("X", json!(1.0)), ("Y", json!(1.0)), ("Z", json!(1.0))]);
    let arr = node(&mut g, "Array Polar", &[("Count", json!(4)), ("Center", json!([5.0, 0.0, 0.0]))]);
    let cen = node(&mut g, "Centroid", &[]);
    g.connect(&b, 0, &arr, 0);
    g.connect(&arr, 0, &cen, 0);
    let out = run(&g);
    assert_eq!(out[&arr][0].count(), 4);
    let pts: Vec<DVec3> = out[&cen][0].items().iter().map(super::data::as_point).collect();
    assert!((pts[0] - DVec3::new(0.5, 0.5, 0.5)).length() < 1e-6);
    // the third copy is opposite the centre
    assert!((pts[2] - DVec3::new(9.5, -0.5, 0.5)).length() < 1e-6, "{:?}", pts[2]);

    let mut g = Graph::new();
    let p = node(&mut g, "Point", &[("X", json!(1.0))]);
    let mv = node(&mut g, "Move", &[("Vector", json!([0.0, 2.0, 0.0]))]);
    let mir = node(&mut g, "Mirror", &[]);
    g.connect(&p, 0, &mv, 0);
    g.connect(&mv, 0, &mir, 0);
    let out = run(&g);
    assert!((first_pt(&out[&mir][0]) - DVec3::new(-1.0, 2.0, 0.0)).length() < 1e-12);
}

#[test]
fn wire_cut_splits_a_box() {
    let mut g = Graph::new();
    let b = node(&mut g, "Box", &[]);
    let l = node(&mut g, "Line", &[("Start", json!([-5.0, 5.0, 0.0])), ("End", json!([15.0, 5.0, 0.0]))]);
    let cut = node(&mut g, "Wire Cut", &[]);
    let vol = node(&mut g, "Volume", &[]);
    g.connect(&b, 0, &cut, 0);
    g.connect(&l, 0, &cut, 1);
    g.connect(&cut, 0, &vol, 0);
    let v = nums(&run(&g)[&vol][0]);
    assert_eq!(v.len(), 2, "{v:?}");
    assert!(v.iter().all(|x| close(*x, 500.0, 1e-6)), "{v:?}");
}

#[test]
fn example_solves_and_previews() {
    let g = super::example::build_example();
    let mut ctx = Ctx::default();
    let out = solve(&g, &mut ctx);
    assert_eq!(ctx.preview.len(), 1);
    assert_eq!(ctx.bake.len(), 1);
    let panel = g.nodes.iter().find(|n| n.kind == "Panel").unwrap();
    assert!((first_num(&out[&panel.uid][0]) - PI * 36.0 * 12.0).abs() < 0.05);
}


#[test]
fn numbers_print_like_python() {
    use super::data::py_float;
    assert_eq!(py_float(1357.1508820539107), "1357.1508820539107");
    assert_eq!(py_float(3.0), "3.0");
    assert_eq!(py_float(0.1), "0.1");
    assert_eq!(py_float(1e-5), "1e-05");
    assert_eq!(py_float(1.5e20), "1.5e+20");
}

// --------------------------------------------------------------- diagnostics

mod diagnostics {
    use super::*;
    use crate::locust::components::Level;

    fn messages(ctx: &Ctx, uid: &str, level: Level) -> Vec<String> {
        ctx.diagnostics.get(uid).map(|d| d.iter().filter(|x| x.level == level).map(|x| x.message.clone()).collect()).unwrap_or_default()
    }

    #[test]
    fn panic_becomes_error_diagnostic() {
        let mut reg = Registry::default();
        reg.elem("Boom", "Test", vec![num("A", 1.0)], &["R"], |_, _| panic!("boom here"));
        let mut g = Graph::new();
        let n = node(&mut g, "Boom", &[]);
        let mut ctx = Ctx::default();
        solve_with(&g, &mut ctx, &HashMap::new(), &reg);
        let errs = messages(&ctx, &n, Level::Error);
        assert!(errs.iter().any(|m| m.contains("boom here")), "{errs:?}");
    }

    #[test]
    fn empty_result_warns() {
        let mut g = Graph::new();
        let number = node(&mut g, "Number", &[("Value", json!(3.0))]);
        let eval = node(&mut g, "Evaluate Curve", &[]);
        g.connect(&number, 0, &eval, 0);
        let mut ctx = Ctx::default();
        solve(&g, &mut ctx);
        assert_eq!(messages(&ctx, &eval, Level::Warning), vec!["no result".to_string()]);
        assert!(messages(&ctx, &number, Level::Warning).is_empty());
    }

    #[test]
    fn unknown_kind_errors() {
        let mut g = Graph::new();
        let n = node(&mut g, "Nope", &[]);
        let mut ctx = Ctx::default();
        solve(&g, &mut ctx);
        assert_eq!(messages(&ctx, &n, Level::Error), vec!["unknown component Nope".to_string()]);
    }

    /// A Boolean that cannot be done says why on its node.
    #[test]
    fn boolean_error_reaches_node() {
        let mut g = Graph::new();
        let a = node(&mut g, "Line", &[]);
        let b = node(&mut g, "Line", &[("End", json!([0.0, 1.0, 0.0]))]);
        let u = node(&mut g, "Union", &[]);
        g.connect(&a, 0, &u, 0);
        g.connect(&b, 0, &u, 1);
        let mut ctx = Ctx::default();
        solve(&g, &mut ctx);
        let errs = messages(&ctx, &u, Level::Error);
        assert!(errs.iter().any(|m| m.contains("cannot be combined")), "{errs:?}");
    }
}

// --------------------------------------------------------------- nodes with their own ports

mod dynamic_ports {
    use super::*;
    use crate::locust::graph::{is_dynamic, Access, PortDef};

    fn names(p: &[PortDef]) -> Vec<&str> {
        p.iter().map(|d| d.name.as_str()).collect()
    }

    #[test]
    fn dynamic_ports_default_and_custom() {
        assert!(is_dynamic("Script") && is_dynamic("Expression") && !is_dynamic("Circle"));
        let mut g = Graph::new();
        let e = node(&mut g, "Expression", &[]);
        let (ins, outs) = g.dynamic_ports(g.node(&e).unwrap());
        assert_eq!(names(&ins), ["x", "y", "z"]);
        assert_eq!(names(&outs), ["Result"]);
        let s = node(&mut g, "Script", &[]);
        let (ins, outs) = g.dynamic_ports(g.node(&s).unwrap());
        assert_eq!((names(&ins), names(&outs)), (vec!["x", "y"], vec!["a"]));
        let c = node(&mut g, "Script", &[("_inputs", json!([{ "name": "pts", "kind": "point", "access": "list" }, "n"])), ("_outputs", json!(["cells", { "name": "count" }]))]);
        let (ins, outs) = g.dynamic_ports(g.node(&c).unwrap());
        assert_eq!(names(&ins), ["pts", "n"]);
        assert_eq!((ins[0].kind.as_str(), ins[0].access), ("point", Access::List));
        assert_eq!((ins[1].kind.as_str(), ins[1].access), ("any", Access::Item));
        assert_eq!(names(&outs), ["cells", "count"]);
    }

    #[test]
    fn node_ports_covers_all_three() {
        let mut g = Graph::new();
        let circle = node(&mut g, "Circle", &[]);
        let script = node(&mut g, "Script", &[("_inputs", json!(["r"])), ("_outputs", json!(["c"]))]);
        let (ins, outs) = g.node_ports(g.node(&circle).unwrap());
        assert_eq!(ins.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), registry().get("Circle").unwrap().inputs.iter().map(|i| i.name).collect::<Vec<_>>());
        assert_eq!(outs.len(), 1);
        let (ins, outs) = g.node_ports(g.node(&script).unwrap());
        assert_eq!((ins[0].0.as_str(), outs[0].0.as_str()), ("r", "c"));
        let a = node(&mut g, "Number", &[]);
        let b = node(&mut g, "Number", &[]);
        g.connect(&a, 0, &b, 0);
        let cl = g.make_cluster(std::slice::from_ref(&b), "C").unwrap();
        let (ins, _) = g.node_ports(g.node(&cl).unwrap());
        assert_eq!(ins.len(), 1, "cluster ports come through node_ports too");
    }

    #[test]
    fn dynamic_node_round_trips_json() {
        let mut g = Graph::new();
        node(&mut g, "Script", &[("_inputs", json!([{ "name": "pts", "kind": "point", "access": "tree" }])), ("code", json!("a = pts;"))]);
        let back = Graph::from_json(&g.to_json());
        assert_eq!(back, g);
        let (ins, _) = back.dynamic_ports(&back.nodes[0]);
        assert_eq!(ins[0].access, Access::Tree);
    }

    #[test]
    fn dynamic_node_is_not_unknown() {
        let mut g = Graph::new();
        let s = node(&mut g, "Script", &[]);
        let mut ctx = Ctx::default();
        let out = solve(&g, &mut ctx);
        assert!(!ctx.diagnostics.get(&s).is_some_and(|d| d.iter().any(|x| x.message.starts_with("unknown component"))), "{:?}", ctx.diagnostics);
        assert_eq!(out[&s].len(), 1, "one output tree per output port");
    }

    #[test]
    fn ports_can_be_added_renamed_and_removed() {
        let mut g = Graph::new();
        let src = node(&mut g, "Number", &[]);
        let s = node(&mut g, "Script", &[]);
        assert_eq!(g.add_port(&s, false).as_deref(), Some("z"));
        assert_eq!(g.add_port(&s, true).as_deref(), Some("b"));
        g.connect(&src, 0, &s, 2); // into z
        assert!(g.rename_port(&s, false, 2, "height"));
        assert!(!g.rename_port(&s, false, 0, "y"), "names stay unique");
        assert!(!g.rename_port(&s, false, 0, "bad name"), "names are identifiers");
        g.remove_port(&s, false, 0); // x goes: the wire into height moves from 2 to 1
        let (ins, outs) = g.dynamic_ports(g.node(&s).unwrap());
        assert_eq!(names(&ins), ["y", "height"]);
        assert_eq!(names(&outs), ["a", "b"]);
        assert_eq!(g.source_of(&s, 1).map(|c| c.src.clone()), Some(src.clone()));
        g.remove_port(&s, false, 1);
        assert!(g.connections.is_empty(), "a removed port loses its wire");
    }
}

// --------------------------------------------------------------- helix

mod helix {
    use super::*;

    fn curve(t: &DataTree) -> tri_geom::NurbsCurve {
        match t.first().and_then(|v| v.as_geometry()) {
            Some(tri_doc::Geometry::Curve(c)) => c.clone(),
            other => panic!("expected a curve, got {other:?}"),
        }
    }

    #[test]
    fn helix_length_and_ends() {
        let out = solve_node("Helix", &[("Radius", json!(5.0)), ("Turns", json!(5.0)), ("End", json!([0.0, 0.0, 10.0]))]);
        let c = curve(&out[0]);
        let expect = ((2.0 * PI * 5.0 * 5.0).powi(2) + 100.0).sqrt();
        assert!(close(c.length(), expect, 0.01), "length {} vs {expect}", c.length());
        assert!((c.start_point() - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-6, "{:?}", c.start_point());
        assert!((c.end_point().z - 10.0).abs() < 1e-6);
    }

    #[test]
    fn helix_pitch_overrides_turns() {
        let out = solve_node("Helix", &[("Radius", json!(5.0)), ("Turns", json!(1.0)), ("Pitch", json!(2.0)), ("End", json!([0.0, 0.0, 10.0]))]);
        let expect = ((2.0 * PI * 5.0 * 5.0).powi(2) + 100.0).sqrt();
        assert!(close(curve(&out[0]).length(), expect, 0.01));
    }

    #[test]
    fn conical_helix_ends_at_end_radius() {
        let out = solve_node("Helix", &[("Radius", json!(5.0)), ("End Radius", json!(2.0)), ("Turns", json!(3.0)), ("End", json!([0.0, 0.0, 6.0]))]);
        let e = curve(&out[0]).end_point();
        assert!((DVec3::new(e.x, e.y, 0.0).length() - 2.0).abs() < 1e-6, "{e:?}");
    }
}

// --------------------------------------------------------------- populate

mod populate {
    use super::*;
    use crate::locust::components::Level;

    fn pts(t: &DataTree) -> Vec<DVec3> {
        t.items().iter().map(super::super::data::as_point).collect()
    }

    fn populate_circle(count: i64, seed: i64) -> (Vec<DVec3>, Ctx, String) {
        let mut g = Graph::new();
        let c = node(&mut g, "Circle", &[("Radius", json!(10.0))]);
        let p = node(&mut g, "Populate 2D", &[("Count", json!(count)), ("Seed", json!(seed))]);
        g.connect(&c, 0, &p, 0);
        let mut ctx = Ctx::default();
        let out = solve(&g, &mut ctx);
        (pts(&out[&p][0]), ctx, p)
    }

    fn warnings(ctx: &Ctx, uid: &str) -> Vec<String> {
        ctx.diagnostics.get(uid).map(|d| d.iter().filter(|x| x.level == Level::Warning).map(|x| x.message.clone()).collect()).unwrap_or_default()
    }

    #[test]
    fn populate_2d_inside_and_deterministic() {
        let (a, _, _) = populate_circle(200, 1);
        assert_eq!(a.len(), 200);
        assert!(a.iter().all(|p| p.length() <= 10.0 + 1e-9 && p.z.abs() < 1e-9));
        let (b, _, _) = populate_circle(200, 1);
        assert_eq!(a, b, "same seed, same points");
        let (c, _, _) = populate_circle(200, 2);
        assert_ne!(a, c);
    }

    #[test]
    fn populate_2d_count_limits() {
        let (a, ctx, p) = populate_circle(0, 1);
        assert!(a.is_empty());
        assert_eq!(warnings(&ctx, &p), ["count is 0"]);
        let (a, ctx, p) = populate_circle(10_000_000, 1);
        assert_eq!(a.len(), 100_000);
        assert_eq!(warnings(&ctx, &p), ["count limited to 100000"]);
    }

    #[test]
    fn populate_2d_default_region() {
        let out = solve_node("Populate 2D", &[("Count", json!(50))]);
        let p = pts(&out[0]);
        assert_eq!(p.len(), 50);
        assert!(p.iter().all(|q| (0.0..=100.0).contains(&q.x) && (0.0..=100.0).contains(&q.y)));
    }

    #[test]
    fn populate_geometry_on_sphere() {
        let mut g = Graph::new();
        let s = node(&mut g, "Sphere", &[("Radius", json!(10.0))]);
        let p = node(&mut g, "Populate Geometry", &[("Count", json!(300)), ("Seed", json!(4))]);
        g.connect(&s, 0, &p, 0);
        let out = run(&g);
        let ps = pts(&out[&p][0]);
        assert_eq!(ps.len(), 300);
        assert!(ps.iter().all(|q| (q.length() - 10.0).abs() < 1e-3), "{:?}", ps.iter().map(|q| q.length()).fold(0.0f64, |m, l| m.max((l - 10.0).abs())));
        let above = ps.iter().filter(|q| q.z > 0.0).count();
        assert!((100..200).contains(&above), "spread over the whole sphere: {above} above the equator");
    }
}

// --------------------------------------------------------------- voronoi

mod voronoi {
    use super::*;
    use crate::locust::components::voronoi::cells_2d;
    use crate::locust::components::Level;
    use glam::DVec2;

    fn area(poly: &[DVec2]) -> f64 {
        let n = poly.len();
        (0..n).map(|i| poly[i].perp_dot(poly[(i + 1) % n])).sum::<f64>().abs() / 2.0
    }

    fn square(s: f64) -> Vec<DVec2> {
        vec![DVec2::new(0.0, 0.0), DVec2::new(s, 0.0), DVec2::new(s, s), DVec2::new(0.0, s)]
    }

    #[test]
    fn four_sites_four_quadrants() {
        let sites = [DVec2::new(25.0, 25.0), DVec2::new(75.0, 25.0), DVec2::new(75.0, 75.0), DVec2::new(25.0, 75.0)];
        let cells = cells_2d(&sites, &square(100.0));
        assert_eq!(cells.len(), 4);
        for (cell, site) in cells.iter().zip(sites) {
            assert!((area(cell) - 2500.0).abs() < 1e-6, "{}", area(cell));
            assert!(crate::locust::components::populate::inside(site, cell));
        }
    }

    #[test]
    fn cells_tile_the_boundary() {
        let sites = crate::locust::components::populate::points_in(&square(100.0), 30, 1);
        let cells = cells_2d(&sites, &square(100.0));
        assert_eq!(cells.len(), 30);
        let total: f64 = cells.iter().map(|c| area(c)).sum();
        assert!((total - 10_000.0).abs() < 10.0, "{total}");
    }

    #[test]
    fn voronoi_component_makes_closed_cells() {
        let mut g = Graph::new();
        let pop = node(&mut g, "Populate 2D", &[("Count", json!(40)), ("Seed", json!(3))]);
        let v = node(&mut g, "Voronoi", &[]);
        g.connect(&pop, 0, &v, 0);
        let out = run(&g);
        let cells = out[&v][0].items();
        assert_eq!(cells.len(), 40);
        assert!(cells.iter().all(|c| matches!(c.as_geometry(), Some(tri_doc::Geometry::Curve(k)) if k.is_closed())));
    }

    #[test]
    fn voronoi_rejects_concave_boundary() {
        let mut g = Graph::new();
        let l = node(&mut g, "Polyline", &[]);
        let pts = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 5.0, 0.0], [5.0, 5.0, 0.0], [5.0, 10.0, 0.0], [0.0, 10.0, 0.0], [0.0, 0.0, 0.0]];
        g.node_mut(&l).unwrap().values.insert("Vertices".into(), json!(pts));
        let pop = node(&mut g, "Populate 2D", &[("Count", json!(5))]);
        let v = node(&mut g, "Voronoi", &[]);
        g.connect(&pop, 0, &v, 0);
        g.connect(&l, 0, &v, 1);
        let mut ctx = Ctx::default();
        solve(&g, &mut ctx);
        let errs: Vec<String> = ctx.diagnostics.get(&v).map(|d| d.iter().filter(|x| x.level == Level::Error).map(|x| x.message.clone()).collect()).unwrap_or_default();
        assert!(errs.iter().any(|e| e.contains("convex")), "{errs:?}");
    }

    #[test]
    fn surface_voronoi_on_cylinder() {
        let mut g = Graph::new();
        let arc = node(&mut g, "Arc", &[("Radius", json!(10.0)), ("Angle", json!(90.0))]);
        let ext = node(&mut g, "Extrude", &[("Distance", json!(20.0)), ("Cap", json!(false))]);
        let sv = node(&mut g, "Surface Voronoi", &[("Count", json!(20)), ("Seed", json!(2))]);
        g.connect(&arc, 0, &ext, 0);
        g.connect(&ext, 0, &sv, 0);
        let out = run(&g);
        let cells = out[&sv][0].items();
        assert_eq!(cells.len(), 20);
        assert_eq!(out[&sv][1].count(), 20, "sites");
        for c in &cells {
            let Some(tri_doc::Geometry::Curve(k)) = c.as_geometry() else { panic!("a curve") };
            let (t0, t1) = k.domain();
            for i in 0..50 {
                let p = k.point_at(t0 + (t1 - t0) * i as f64 / 50.0);
                let r = DVec3::new(p.x, p.y, 0.0).length();
                assert!((r - 10.0).abs() < 1e-3, "off the cylinder by {}", (r - 10.0).abs());
            }
        }
    }
}

/// A loft of closed sections is closed around but open at the ends: Cap must close it.
#[test]
fn cap_closes_a_loft_of_closed_sections() {
    let mut g = Graph::new();
    let s = g.add_node_with("Script", 0.0, 0.0, vals(&[("code", json!("a = cap(loft([circle(point(0,0,0), 5), circle(point(0,0,10), 5)]));")), ("_inputs", json!([])), ("_outputs", json!(["a"]))]));
    let out = run(&g);
    let Some(tri_doc::Geometry::Brep(b)) = out[&s][0].first().and_then(|v| v.as_geometry()) else { panic!("a polysurface") };
    assert_eq!(b.faces.len(), 3, "side and two caps");
    let v = tri_doc::Document::volume_of(&tri_doc::Geometry::Brep(b.clone())).unwrap().0;
    assert!(close(v, PI * 25.0 * 10.0, 0.02), "volume {v}");
}

/// A long loft (a screw's 200 sections) can pass the closed test with its ends open: Cap
/// still closes it.
#[test]
fn cap_closes_a_long_loft() {
    let mut g = Graph::new();
    let code = "let s = []; for k in 0..=200 { let p = []; for i in 0..72 { let t = 2.0 * PI * i / 72; let r = 5.0 + 0.5 * sin(3.0 * t + k * 0.3); p.push(point(r * cos(t), r * sin(t), k * 0.2)); } s.push(polyline(p, true)); } a = cap(loft(s));";
    let s = g.add_node_with("Script", 0.0, 0.0, vals(&[("code", json!(code)), ("_inputs", json!([])), ("_outputs", json!(["a"]))]));
    let out = run(&g);
    let Some(tri_doc::Geometry::Brep(b)) = out[&s][0].first().and_then(|v| v.as_geometry()) else { panic!("a polysurface") };
    assert_eq!(b.faces.len(), 3, "side and two caps");
}

/// The same with polyline sections, as a screw's are.
#[test]
fn cap_closes_a_loft_of_polylines() {
    let mut g = Graph::new();
    let code = "let ring = |z| { let p = []; for i in 0..36 { let t = 2.0 * PI * i / 36; p.push(point(5.0 * cos(t), 5.0 * sin(t), z)); } polyline(p, true) }; \
                let s = []; for k in 0..=10 { s.push(ring.call(k * 1.0)); } a = cap(loft(s));";
    let s = g.add_node_with("Script", 0.0, 0.0, vals(&[("code", json!(code)), ("_inputs", json!([])), ("_outputs", json!(["a"]))]));
    let mut ctx = Ctx::default();
    let out = solve(&g, &mut ctx);
    let Some(tri_doc::Geometry::Brep(b)) = out[&s][0].first().and_then(|v| v.as_geometry()) else { panic!("a polysurface: {:?}", ctx.diagnostics) };
    assert_eq!(b.faces.len(), 3, "side and two caps");
}

/// Review: Surface Voronoi says when it clamps the count, like Populate.
#[test]
fn surface_voronoi_count_warnings() {
    for (count, warning) in [(0, "count is 0"), (100_000, "count limited to 5000")] {
        let mut g = Graph::new();
        let s = node(&mut g, "Sphere", &[]);
        let sv = node(&mut g, "Surface Voronoi", &[("Count", json!(count))]);
        g.connect(&s, 0, &sv, 0);
        let mut ctx = Ctx::default();
        let token = tri_geom::job::Token::new();
        let _guard = tri_geom::job::install(token.clone());
        // only the warning matters here: stop the big solve once it is under way
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            token.cancel();
        });
        solve(&g, &mut ctx);
        canceller.join().unwrap();
        let warnings: Vec<String> = ctx.diagnostics.get(&sv).map(|d| d.iter().map(|x| x.message.clone()).collect()).unwrap_or_default();
        assert!(warnings.iter().any(|w| w == warning), "{count}: {warnings:?}");
    }
}

/// A thread made the classic way: an ISO profile swept along a helix and united with its
/// core. The union must be one closed solid.
#[test]
fn helical_sweep_unites_with_its_core() {
    let g = crate::locust::authoring::prepare(&json!({
        "nodes": [
            { "type": "Script", "nickname": "Prof", "values": { "_inputs": [], "_outputs": ["profile"], "code": "let rc = 5.0 - 0.6134 * 1.5; let inner = rc - 0.15; let w = 2.0 * (5.0 - inner) * 0.57735; profile = polyline([point(inner, 0, -w / 2), point(5.0, 0, -0.06), point(5.0, 0, 0.06), point(inner, 0, w / 2)], true);" } },
            { "type": "Helix", "nickname": "Rail", "values": { "Radius": 4.08, "Pitch": 1.5, "End": [0, 0, 20] } },
            { "type": "Extrude Along Curve", "nickname": "Thread" },
            { "type": "Circle", "nickname": "C", "values": { "Radius": 4.08 } },
            { "type": "Extrude", "nickname": "Core", "values": { "Distance": 20 } },
            { "type": "Union", "nickname": "Screw" }
        ],
        "connections": [
            { "from": "Prof.profile", "to": "Thread.Profile" }, { "from": "Rail", "to": "Thread.Path" },
            { "from": "C", "to": "Core.Curve" }, { "from": "Core", "to": "Screw.A" }, { "from": "Thread", "to": "Screw.B" }
        ]
    }))
    .unwrap();
    let mut ctx = Ctx::default();
    let out = solve(&g, &mut ctx);
    let screw = g.nodes.iter().find(|n| n.nickname.as_deref() == Some("Screw")).unwrap();
    let result = out[&screw.uid][0].first().and_then(|v| v.as_geometry()).cloned();
    let Some(tri_doc::Geometry::Brep(b)) = result.as_ref() else { panic!("no solid: {:?}", ctx.diagnostics.get(&screw.uid)) };
    assert!(b.closed);
    let v = tri_doc::Document::volume_of(&tri_doc::Geometry::Brep(b.clone())).unwrap().0;
    let core = PI * 4.08 * 4.08 * 20.0;
    assert!(v > core && v < PI * 25.0 * 21.0, "volume {v}");
}


/// The female thread the same way: a hex nut body minus the swept screw is one closed solid.
#[test]
fn hex_nut_minus_a_swept_screw() {
    // TEMPORARY: diagnose the macOS-only difference in CI
    std::env::set_var("TRICERATOPS_BOOL_DEBUG", "1");
    let g = crate::locust::authoring::prepare(&json!({
        "nodes": [
            { "type": "Script", "nickname": "Prof", "values": { "_inputs": [], "_outputs": ["profile"], "code": "let rc = 5.0 - 0.6134 * 1.5; let inner = rc - 0.15; let w = 2.0 * (5.0 - inner) * 0.57735; profile = polyline([point(inner, 0, -w / 2), point(5.0, 0, -0.06), point(5.0, 0, 0.06), point(inner, 0, w / 2)], true);" } },
            { "type": "Helix", "nickname": "Rail", "values": { "Radius": 4.08, "Pitch": 1.5, "End": [0, 0, 20] } },
            { "type": "Extrude Along Curve", "nickname": "Thread" },
            { "type": "Circle", "nickname": "C", "values": { "Radius": 4.08 } },
            { "type": "Extrude", "nickname": "Core", "values": { "Distance": 20 } },
            { "type": "Union", "nickname": "Screw" },
            { "type": "Polygon", "nickname": "Hex", "values": { "Radius": 9.24, "Sides": 6, "Center": [0, 0, 6] } },
            { "type": "Extrude", "nickname": "Body", "values": { "Distance": 8 } },
            { "type": "Difference", "nickname": "Nut" }
        ],
        "connections": [
            { "from": "Prof.profile", "to": "Thread.Profile" }, { "from": "Rail", "to": "Thread.Path" },
            { "from": "C", "to": "Core.Curve" }, { "from": "Core", "to": "Screw.A" }, { "from": "Thread", "to": "Screw.B" },
            { "from": "Hex", "to": "Body.Curve" }, { "from": "Body", "to": "Nut.A" }, { "from": "Screw", "to": "Nut.B" }
        ]
    }))
    .unwrap();
    let mut ctx = Ctx::default();
    let out = solve(&g, &mut ctx);
    let nut = g.nodes.iter().find(|n| n.nickname.as_deref() == Some("Nut")).unwrap();
    let result = out[&nut.uid][0].first().and_then(|v| v.as_geometry()).cloned();
    let Some(tri_doc::Geometry::Brep(b)) = result.as_ref() else { panic!("no solid: {:?}", ctx.diagnostics.get(&nut.uid)) };
    assert!(b.closed);
    let v = tri_doc::Document::volume_of(&tri_doc::Geometry::Brep(b.clone())).unwrap().0;
    let hex = 3.0f64.sqrt() / 2.0 * 16.0 * 16.0 * 8.0;
    eprintln!("NUTVOLUME {v}");
    assert!(v < hex - PI * 4.08 * 4.08 * 8.0 && v > hex - PI * 25.0 * 8.0, "volume {v}");
}
