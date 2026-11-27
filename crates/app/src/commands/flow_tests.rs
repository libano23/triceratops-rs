//! Command flow tests: drive the runner the way the UI does, without a window.

use std::collections::HashMap;

use glam::DVec3;
use tri_doc::{Document, Geometry};
use tri_geom::Plane;

use super::*;

struct Harness {
    doc: Document,
    log: Vec<String>,
    requests: Vec<AppRequest>,
    memory: HashMap<String, String>,
    runner: CommandRunner,
}

impl Harness {
    fn new() -> Self {
        Self { doc: Document::new(), log: Vec::new(), requests: Vec::new(), memory: HashMap::new(), runner: CommandRunner::default() }
    }
    fn ctx<R>(&mut self, f: impl FnOnce(&mut CommandRunner, &mut CmdCtx) -> R) -> R {
        let mut ctx = CmdCtx {
            doc: &mut self.doc,
            log: &mut self.log,
            cplane: Plane::world_xy(),
            view_dir: -DVec3::Z,
            view_eye: DVec3::new(0.0, 0.0, 100.0),
            requests: &mut self.requests,
            memory: &mut self.memory,
        };
        let r = f(&mut self.runner, &mut ctx);
        // long work (Booleans) runs as a job: these tests step through commands synchronously
        self.runner.wait_job(&mut ctx);
        r
    }
    fn start(&mut self, name: &str) {
        let pre = self.doc.selected_ids();
        self.ctx(|r, c| r.start(name, name, c, &pre));
    }
    fn point(&mut self, x: f64, y: f64, z: f64) {
        self.ctx(|r, c| r.feed(Input::Point(DVec3::new(x, y, z)), c));
    }
    fn number(&mut self, v: f64) {
        self.ctx(|r, c| r.feed(Input::Number(v), c));
    }
    fn option(&mut self, name: &str, value: Option<&str>) {
        self.ctx(|r, c| r.feed_option(name, value, c));
    }
    fn enter(&mut self) {
        self.ctx(|r, c| r.feed(Input::Empty, c));
    }
    fn objects(&mut self, ids: Vec<tri_doc::ObjectId>) {
        self.ctx(|r, c| r.feed(Input::Objects(ids), c));
    }
    fn prompt(&self) -> String {
        self.runner.prompt().map_or_else(|| "Command".to_string(), |p| p.display())
    }
}

#[test]
fn line_prompts_and_creates_curve() {
    let mut h = Harness::new();
    h.start("Line");
    assert!(h.prompt().starts_with("Start of line ( BothSides  Normal  Angled  Vertical"));
    h.point(0.0, 0.0, 0.0);
    assert_eq!(h.prompt(), "End of line ( BothSides )");
    h.point(3.0, 4.0, 0.0);
    assert!(!h.runner.is_active());
    assert_eq!(h.doc.len(), 1);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!((c.length() - 5.0).abs() < 1e-9);
    assert!(h.log.iter().any(|l| l.starts_with("Command: Line")));
    assert!(h.log.iter().any(|l| l.starts_with("Start of line (")));
}

#[test]
fn line_both_sides_option() {
    let mut h = Harness::new();
    h.start("Line");
    h.option("BothSides", None);
    h.point(0.0, 0.0, 0.0);
    h.point(1.0, 0.0, 0.0);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!((c.start_point() - DVec3::new(-1.0, 0.0, 0.0)).length() < 1e-9);
}

#[test]
fn polyline_close_option_and_undo() {
    let mut h = Harness::new();
    h.start("Polyline");
    assert_eq!(h.prompt(), "Start of polyline ( PersistentClose=No  Mode=Line  Helpers=No )");
    h.point(0.0, 0.0, 0.0);
    h.point(1.0, 0.0, 0.0);
    h.point(1.0, 1.0, 0.0);
    h.point(5.0, 5.0, 0.0);
    h.option("Undo", None);
    assert!(h.prompt().contains("Close"));
    h.option("Close", None);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!(c.is_closed());
    assert_eq!(c.control_points.len(), 4);
    assert!(h.doc.can_undo());
    assert!(h.doc.undo().is_some());
    assert_eq!(h.doc.len(), 0);
}

#[test]
fn circle_radius_default_remembered_and_diameter_option() {
    let mut h = Harness::new();
    h.start("Circle");
    assert!(h.prompt().starts_with("Center of circle ( Deformable  Vertical  2Point  3Point"));
    h.point(0.0, 0.0, 0.0);
    assert!(h.prompt().starts_with("Radius <1> ( Diameter"));
    h.number(2.5);
    h.start("Circle");
    h.point(10.0, 0.0, 0.0);
    assert!(h.prompt().starts_with("Radius <2.5>"));
    h.option("Diameter", None);
    assert!(h.prompt().starts_with("Diameter <5>"));
    h.enter_default();
    assert_eq!(h.doc.len(), 2);
}

impl Harness {
    fn enter_default(&mut self) {
        let d = self.runner.prompt().and_then(|p| p.default.clone()).and_then(|d| d.parse::<f64>().ok());
        match d {
            Some(v) => self.number(v),
            None => self.enter(),
        }
    }
}

#[test]
fn box_uses_width_for_height_on_enter() {
    let mut h = Harness::new();
    h.start("Box");
    assert_eq!(h.prompt(), "First corner of base ( Diagonal  3Point  Vertical  Center )");
    h.point(0.0, 0.0, 0.0);
    assert_eq!(h.prompt(), "Other corner of base or length");
    h.point(4.0, 2.0, 0.0);
    assert!(h.prompt().starts_with("Height. Press Enter to use width <4>"));
    h.enter();
    let o = h.doc.objects().next().unwrap();
    let bb = o.geometry.bounding_box();
    assert!((bb.max - DVec3::new(4.0, 2.0, 4.0)).length() < 1e-9);
    assert!(matches!(o.geometry, Geometry::Brep(_)));
}

#[test]
fn extrude_closed_circle_makes_solid_and_remembers_distance() {
    let mut h = Harness::new();
    h.start("Circle");
    h.point(0.0, 0.0, 0.0);
    h.number(1.0);
    let id = h.doc.objects().next().unwrap().id;
    h.doc.select(id, true);
    h.start("ExtrudeCrv");
    assert!(h.prompt().starts_with("Extrusion distance <1> ( Direction  BothSides=No  Solid=Yes"));
    h.option("Solid", None);
    assert!(h.prompt().contains("Solid=No"));
    h.option("Solid", None);
    h.number(3.0);
    let brep = h.doc.objects().find(|o| matches!(o.geometry, Geometry::Brep(_))).expect("solid extrusion");
    if let Geometry::Brep(b) = &brep.geometry {
        assert_eq!(b.faces.len(), 3);
    }
    assert_eq!(h.memory.get("ExtrudeCrv.Distance").map(String::as_str), Some("3"));
}

#[test]
fn move_copy_rotate_mirror() {
    let mut h = Harness::new();
    h.start("Point");
    h.point(1.0, 0.0, 0.0);
    let id = h.doc.objects().next().unwrap().id;
    h.start("Move");
    h.objects(vec![id]);
    assert_eq!(h.prompt(), "Point to move from ( Vertical=No )");
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 5.0, 0.0);
    assert!(matches!(h.doc.get(id).unwrap().geometry, Geometry::Point(p) if (p - DVec3::new(1.0, 5.0, 0.0)).length() < 1e-9));

    h.start("Copy");
    h.objects(vec![id]);
    h.point(0.0, 0.0, 0.0);
    h.point(1.0, 0.0, 0.0);
    h.point(2.0, 0.0, 0.0);
    h.enter();
    assert_eq!(h.doc.len(), 3);

    h.start("Rotate");
    h.objects(vec![id]);
    assert_eq!(h.prompt(), "Center of rotation ( Copy=No  UseLastCenter )");
    h.point(0.0, 0.0, 0.0);
    h.number(90.0);
    assert!(matches!(h.doc.get(id).unwrap().geometry, Geometry::Point(p) if (p - DVec3::new(-5.0, 1.0, 0.0)).length() < 1e-9));

    h.start("Mirror");
    h.objects(vec![id]);
    assert_eq!(h.prompt(), "Start of mirror plane ( 3Point  Copy=Yes  XAxis  YAxis  ZAxis  Object )");
    h.option("YAxis", None);
    assert_eq!(h.doc.len(), 4);
}

#[test]
fn option_value_typed_sets_degree() {
    let mut h = Harness::new();
    h.start("Curve");
    h.option("Degree", Some("5"));
    assert!(h.prompt().contains("Degree=5"));
    for i in 0..7 {
        h.point(i as f64, (i % 2) as f64, 0.0);
    }
    h.enter();
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert_eq!(c.degree, 5);
}

#[test]
fn unknown_option_text_is_ignored_and_cancel_logs() {
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    assert!(h.runner.is_active());
    h.ctx(|r, c| r.cancel(c));
    assert!(!h.runner.is_active());
    assert_eq!(h.log.last().map(String::as_str), Some("Command: "));
}

#[test]
fn preselection_feeds_objects_prompt() {
    let mut h = Harness::new();
    h.start("Point");
    h.point(0.0, 0.0, 0.0);
    let id = h.doc.objects().next().unwrap().id;
    h.doc.select(id, true);
    h.start("Delete");
    assert!(!h.runner.is_active());
    assert_eq!(h.doc.len(), 0);
}

#[test]
fn registry_resolves_aliases_and_autocomplete() {
    assert_eq!(find_command("_Line").map(|c| c.name), Some("Line"));
    assert_eq!(find_command("pl").map(|c| c.name), Some("Polyline"));
    let (exact, fuzzy) = lookup("sph", 10);
    assert_eq!(exact[0].name, "Sphere");
    assert!(fuzzy.is_empty() || fuzzy.iter().all(|c| c.name.to_ascii_lowercase().contains("sph")));
    for c in super::registry::COMMANDS {
        // every registered command either constructs or is explicitly unimplemented
        let _ = registry::create_command(c.name);
    }
}

#[test]
fn trim_split_extend_fillet_offset_flow() {
    let mut h = Harness::new();
    // a horizontal line crossed by a vertical line
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    h.start("Line");
    h.point(5.0, -5.0, 0.0);
    h.point(5.0, 5.0, 0.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    let (horiz, vert) = (ids[0], ids[1]);
    // Trim: cutter = vertical, remove the right half of the horizontal line
    h.start("Trim");
    h.objects(vec![vert]);
    assert!(h.prompt().starts_with("Select object to trim"));
    h.ctx(|r, c| r.feed(Input::Pick(horiz, DVec3::new(8.0, 0.0, 0.0)), c));
    h.enter();
    let remaining: Vec<NurbsCurve> = h.doc.objects().filter_map(|o| o.geometry.as_curve().cloned()).collect();
    assert_eq!(remaining.len(), 2);
    let short = remaining.iter().find(|c| (c.length() - 5.0).abs() < 1e-6 && c.start_point().y.abs() < 1e-9).expect("left half kept");
    assert!((short.end_point() - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-6);

    // Split the vertical line with the short line
    let short_id = h.doc.objects().find(|o| o.geometry.as_curve().is_some_and(|c| (c.length() - 5.0).abs() < 1e-6 && c.start_point().y.abs() < 1e-9)).unwrap().id;
    h.start("Split");
    h.objects(vec![vert]);
    h.objects(vec![short_id]);
    assert_eq!(h.doc.len(), 3);

    // Extend the short line by length 2
    h.start("Extend");
    h.number(2.0);
    assert!(h.prompt().starts_with("Select curve to extend"));
    h.ctx(|r, c| r.feed(Input::Pick(short_id, DVec3::new(4.5, 0.0, 0.0)), c));
    h.enter();
    let ext = h.doc.get(short_id).unwrap().geometry.as_curve().unwrap();
    assert!((ext.length() - 7.0).abs() < 1e-6);

    // Fillet two fresh perpendicular lines with Join=Yes
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    h.start("Line");
    h.point(10.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("Fillet");
    h.option("Radius", Some("2"));
    assert!(h.prompt().contains("Radius=2"));
    h.ctx(|r, c| r.feed(Input::Pick(ids[0], DVec3::new(3.0, 0.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Pick(ids[1], DVec3::new(10.0, 7.0, 0.0)), c));
    assert_eq!(h.doc.len(), 1, "joined polycurve");
    let pc = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!((pc.length() - (8.0 + 8.0 + std::f64::consts::PI)).abs() < 1e-2);

    // Offset the polycurve inward (towards the corner side)
    let pid = h.doc.objects().next().unwrap().id;
    h.start("Offset");
    h.ctx(|r, c| r.feed(Input::Pick(pid, DVec3::new(5.0, 0.0, 0.0)), c));
    assert!(h.prompt().starts_with("Side to offset"));
    h.point(5.0, 3.0, 0.0);
    assert_eq!(h.doc.len(), 2);
}

#[test]
fn cap_makes_extrusion_closed() {
    let mut h = Harness::new();
    h.start("Rectangle");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 3.0, 0.0);
    let rect = h.doc.objects().next().unwrap().id;
    h.doc.select(rect, true);
    h.start("ExtrudeCrv");
    h.option("Solid", None); // Solid=No -> open surface
    h.number(2.0);
    let srf = h.doc.objects().find(|o| matches!(o.geometry, Geometry::Surface(_))).unwrap();
    assert_eq!(srf.geometry.type_name(), "open surface");
    let sid = srf.id;
    h.start("Cap");
    h.objects(vec![sid]);
    let capped = h.doc.get(sid).unwrap();
    assert_eq!(capped.geometry.type_name(), "closed solid polysurface");
    assert!(capped.geometry.is_solid());
}

#[test]
fn surface_trim_split_extrude_offset_boolean() {
    let mut h = Harness::new();
    // planar square surface 10x10 from a rectangle
    h.start("Rectangle");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    let rect = h.doc.objects().next().unwrap().id;
    h.start("PlanarSrf");
    h.objects(vec![rect]);
    let srf = h.doc.objects().find(|o| matches!(o.geometry, Geometry::Brep(_))).unwrap().id;
    // cutter: vertical line across the square
    h.start("Line");
    h.point(4.0, -2.0, 0.0);
    h.point(4.0, 12.0, 0.0);
    let cutter = h.doc.objects().filter_map(|o| o.geometry.as_curve().map(|_| o.id)).last().unwrap();
    // Trim: remove the left part (pick at x=1)
    h.start("Trim");
    h.objects(vec![cutter]);
    h.ctx(|r, c| r.feed(Input::Pick(srf, DVec3::new(1.0, 5.0, 0.0)), c));
    h.enter();
    let trimmed = h.doc.get(srf).unwrap();
    let bb = trimmed.geometry.bounding_box();
    assert!((bb.min.x - 4.0).abs() < 1e-6 && (bb.max.x - 10.0).abs() < 1e-6, "trimmed bbox {:?}", bb);
    // Split the remaining face with a horizontal line -> two faces
    h.start("Line");
    h.point(0.0, 5.0, 0.0);
    h.point(12.0, 5.0, 0.0);
    let cutter2 = h.doc.objects().filter_map(|o| o.geometry.as_curve().map(|_| o.id)).last().unwrap();
    h.start("Split");
    h.objects(vec![srf]);
    h.objects(vec![cutter2]);
    let breps: Vec<tri_doc::ObjectId> = h.doc.objects().filter(|o| matches!(o.geometry, Geometry::Brep(_))).map(|o| o.id).collect();
    assert_eq!(breps.len(), 2, "split surface into two");
    // ExtrudeSrf of one piece -> closed solid
    let piece = breps[0];
    h.start("ExtrudeSrf");
    h.objects(vec![piece]);
    h.number(3.0);
    let solid = h.doc.objects().find(|o| o.geometry.is_solid()).expect("solid from ExtrudeSrf");
    assert_eq!(solid.geometry.type_name(), "closed solid polysurface");
    // OffsetSrf Solid=Yes on the other piece -> closed solid
    let other = breps[1];
    h.start("OffsetSrf");
    h.objects(vec![other]);
    h.option("Solid", None);
    h.number(1.0);
    let solids: Vec<_> = h.doc.objects().filter(|o| o.geometry.is_solid()).collect();
    assert_eq!(solids.len(), 2);
    // BooleanUnion of two overlapping boxes -> one closed mesh
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(2.0, 2.0, 0.0);
    h.number(2.0);
    h.start("Box");
    h.point(1.0, 1.0, 1.0);
    h.point(3.0, 3.0, 1.0);
    h.number(2.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("BooleanUnion");
    h.objects(ids);
    assert_eq!(h.doc.len(), 1);
    let m = h.doc.objects().next().unwrap();
    assert!(matches!(m.geometry.type_name().as_str(), "closed mesh" | "closed solid polysurface"), "{}", m.geometry.type_name());
}

#[test]
fn transform_surface_solid_mesh_batches() {
    let mut h = Harness::new();
    // Array 2x3 of a point -> 6 points
    h.start("Point");
    h.point(0.0, 0.0, 0.0);
    let pid = h.doc.objects().next().unwrap().id;
    h.start("Array");
    h.objects(vec![pid]);
    h.number(2.0);
    h.number(3.0);
    h.number(1.0);
    h.number(5.0);
    h.number(4.0);
    assert_eq!(h.doc.len(), 6);
    // ArrayPolar of one point, 4 items
    let mut h = Harness::new();
    h.start("Point");
    h.point(1.0, 0.0, 0.0);
    let pid = h.doc.objects().next().unwrap().id;
    h.start("ArrayPolar");
    h.objects(vec![pid]);
    h.point(0.0, 0.0, 0.0);
    h.number(4.0);
    h.number(360.0);
    assert!(h.prompt().contains("Press Enter to accept"), "{}", h.prompt());
    h.enter();
    assert_eq!(h.doc.len(), 4);
    assert!(h.doc.objects().any(|o| matches!(o.geometry, Geometry::Point(p) if (p - DVec3::new(-1.0, 0.0, 0.0)).length() < 1e-9)));
    // Orient: move a point from (0,0)-(1,0) frame to (10,10)-(10,12) frame (rotates 90 deg)
    let mut h = Harness::new();
    h.start("Point");
    h.point(1.0, 0.0, 0.0);
    let pid = h.doc.objects().next().unwrap().id;
    h.start("Orient");
    h.objects(vec![pid]);
    h.point(0.0, 0.0, 0.0);
    h.point(1.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.point(10.0, 12.0, 0.0);
    assert!(matches!(h.doc.get(pid).unwrap().geometry, Geometry::Point(p) if (p - DVec3::new(10.0, 11.0, 0.0)).length() < 1e-9));
    // Pipe along a line with caps -> closed solid
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 0.0, 10.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.start("Pipe");
    h.ctx(|r, c| r.feed(Input::Pick(lid, DVec3::new(0.0, 0.0, 5.0)), c));
    h.number(1.0);
    assert!(h.doc.objects().any(|o| o.geometry.is_solid()), "pipe is solid");
    // Tube -> closed polysurface with annular caps
    let mut h = Harness::new();
    h.start("Tube");
    h.point(0.0, 0.0, 0.0);
    h.number(3.0);
    h.number(2.0);
    h.number(4.0);
    let t = h.doc.objects().next().unwrap();
    assert_eq!(t.geometry.type_name(), "closed solid polysurface");
    // EdgeSrf from four lines
    let mut h = Harness::new();
    let corners = [DVec3::ZERO, DVec3::new(5.0, 0.0, 0.0), DVec3::new(5.0, 5.0, 2.0), DVec3::new(0.0, 5.0, 0.0)];
    for i in 0..4 {
        h.start("Line");
        h.point(corners[i].x, corners[i].y, corners[i].z);
        let n = corners[(i + 1) % 4];
        h.point(n.x, n.y, n.z);
    }
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("EdgeSrf");
    h.objects(ids);
    let srf = h.doc.objects().find(|o| matches!(o.geometry, Geometry::Surface(_))).expect("edge surface");
    let bb = srf.geometry.bounding_box();
    assert!((bb.max.z - 2.0).abs() < 1e-6);
    // Helix
    let mut h = Harness::new();
    h.start("Helix");
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 0.0, 10.0);
    h.number(2.0);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap().clone();
    assert!((c.start_point() - DVec3::new(2.0, 0.0, 0.0)).length() < 1e-6 || (c.start_point().length() - 2.0).abs() < 1e-6);
    assert!((c.end_point().z - 10.0).abs() < 1e-6);
    // Section of a box with a vertical plane
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    h.number(4.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.start("Section");
    h.objects(vec![bid]);
    h.point(2.0, -1.0, 0.0);
    h.point(2.0, 5.0, 0.0);
    let curves: Vec<_> = h.doc.objects().filter(|o| o.geometry.is_curve()).collect();
    assert!(!curves.is_empty(), "section curves");
    assert!(curves.iter().all(|o| o.geometry.bounding_box().min.x > 1.99 && o.geometry.bounding_box().max.x < 2.01));
    // MeshBooleanDifference of two boxes meshed
    let mut h = Harness::new();
    h.start("MeshBox");
    h.point(0.0, 0.0, 0.0);
    h.point(2.0, 2.0, 0.0);
    h.number(2.0);
    h.start("MeshBox");
    h.point(1.0, 1.0, 1.0);
    h.point(3.0, 3.0, 1.0);
    h.number(2.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("MeshBooleanDifference");
    h.objects(vec![ids[0]]);
    h.objects(vec![ids[1]]);
    assert_eq!(h.doc.len(), 1);
    assert_eq!(h.doc.objects().next().unwrap().geometry.type_name(), "closed mesh");
}

#[test]
fn annotations_text_dim_hatch() {
    let mut h = Harness::new();
    h.start("Text");
    h.ctx(|r, c| r.feed(Input::Text("AB".into()), c));
    assert!(h.prompt().starts_with("Start point"));
    h.point(0.0, 0.0, 0.0);
    assert!(h.doc.len() >= 2, "text strokes created");
    assert!(h.doc.objects().all(|o| !o.attributes.groups.is_empty()), "text strokes are grouped");
    let mut h = Harness::new();
    h.start("Dim");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    h.point(5.0, 3.0, 0.0);
    assert!(h.doc.len() >= 6);
    let bb = h.doc.bounding_box();
    assert!(bb.max.x > 9.9 && bb.min.x < 0.1 && bb.max.y > 3.0);
    let mut h = Harness::new();
    h.start("Rectangle");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    let rid = h.doc.objects().next().unwrap().id;
    h.start("Hatch");
    h.option("Scale", Some("2"));
    h.objects(vec![rid]);
    let lines = h.doc.objects().filter(|o| o.attributes.groups.iter().any(|g| g.starts_with("Hatch"))).count();
    assert!((5..=20).contains(&lines), "hatch lines {lines}");
}

#[test]
fn cplane_and_rounded_rectangle() {
    let mut h = Harness::new();
    h.start("CPlane");
    h.option("3Point", None);
    h.point(0.0, 0.0, 5.0);
    h.point(1.0, 0.0, 5.0);
    h.point(0.0, 0.0, 6.0);
    let req = h.requests.iter().find(|r| matches!(r, AppRequest::SetCPlane(Some(_)))).expect("cplane request");
    if let AppRequest::SetCPlane(Some(p)) = req {
        assert!((p.z_axis - DVec3::new(0.0, -1.0, 0.0)).length() < 1e-9, "normal {:?}", p.z_axis);
    }
    let mut h = Harness::new();
    h.start("Rectangle");
    h.option("Rounded", None);
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 6.0, 0.0);
    assert!(h.prompt().starts_with("Radius or point for corner"));
    h.number(1.0);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!(c.is_closed());
    let expected = 2.0 * (8.0 + 4.0) + std::f64::consts::TAU;
    assert!((c.length() - expected).abs() < 0.05, "len {}", c.length());
}

#[test]
fn line_angled_and_perpendicular_options() {
    let mut h = Harness::new();
    h.start("Line");
    h.option("Angled", None);
    assert_eq!(h.prompt(), "Start of reference line");
    h.point(0.0, 0.0, 0.0);
    h.point(1.0, 0.0, 0.0);
    h.number(90.0);
    h.point(5.0, 5.0, 0.0);
    h.number(3.0);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!((c.end_point() - DVec3::new(5.0, 8.0, 0.0)).length() < 1e-9, "end {:?}", c.end_point());
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.start("Line");
    h.option("Perpendicular", None);
    h.ctx(|r, c| r.feed(Input::Pick(lid, DVec3::new(4.0, 0.0, 0.0)), c));
    h.number(2.0);
    let c = h.doc.objects().last().unwrap().geometry.as_curve().unwrap();
    assert!((c.start_point() - DVec3::new(4.0, 0.0, 0.0)).length() < 1e-9);
    assert!((c.end_point().y.abs() - 2.0).abs() < 1e-9);
}

#[test]
fn block_define_insert_explode() {
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(1.0, 0.0, 0.0);
    h.start("Circle");
    h.point(0.0, 0.0, 0.0);
    h.number(1.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("Block");
    h.objects(ids);
    h.point(0.0, 0.0, 0.0);
    h.ctx(|r, c| r.feed(Input::Text("Bolt".into()), c));
    assert_eq!(h.doc.len(), 1, "one instance replaces the two objects");
    assert_eq!(h.doc.state.blocks.len(), 1);
    assert_eq!(h.doc.objects().next().unwrap().geometry.type_name(), "block instance");
    h.start("Insert");
    h.ctx(|r, c| r.feed(Input::Text("Bolt".into()), c));
    assert!(h.prompt().starts_with("Insertion point"));
    h.point(10.0, 5.0, 0.0);
    assert_eq!(h.doc.len(), 2);
    let inst = h.doc.objects().last().unwrap();
    let bb = h.doc.object_bbox(inst);
    assert!((bb.min.x - 9.0).abs() < 1e-9 && (bb.max.x - 11.0).abs() < 1e-9, "bbox {bb:?}");
    let iid = inst.id;
    h.start("ExplodeBlock");
    h.objects(vec![iid]);
    assert_eq!(h.doc.len(), 3);
    assert!(h.doc.objects().filter(|o| o.geometry.is_curve()).count() == 2);
}

#[test]
fn fillet_and_chamfer_box_edge() {
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.start("FilletEdge");
    // pick the vertical edge at x=10,y=10
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(10.0, 10.0, 5.0)), c));
    h.number(2.0);
    h.ctx(|r, c| r.feed(Input::Empty, c));
    let o = h.doc.get(bid).unwrap();
    let Geometry::Brep(b) = &o.geometry else { panic!("brep expected") };
    assert_eq!(b.faces.len(), 7, "six faces plus the fillet");
    assert!(b.closed, "filleted box stays a closed solid");
    let vol = Document::volume_of(&o.geometry).unwrap().0;
    let expected = 1000.0 - (4.0 - std::f64::consts::PI) * 10.0;
    assert!((vol - expected).abs() < 2.0, "volume {vol} vs {expected}");
    // chamfer another edge
    h.start("ChamferEdge");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(0.0, 5.0, 10.0)), c));
    h.number(1.0);
    h.ctx(|r, c| r.feed(Input::Empty, c));
    let o = h.doc.get(bid).unwrap();
    let Geometry::Brep(b) = &o.geometry else { panic!() };
    assert_eq!(b.faces.len(), 8);
    assert!(b.closed);
}

#[test]
fn blend_srf_between_two_planes() {
    let mut h = Harness::new();
    h.start("Plane");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.start("Plane");
    h.point(20.0, 0.0, 5.0);
    h.point(30.0, 10.0, 5.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("BlendSrf");
    h.ctx(|r, c| r.feed(Input::Pick(ids[0], DVec3::new(10.0, 5.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Pick(ids[1], DVec3::new(20.0, 5.0, 5.0)), c));
    h.ctx(|r, c| r.feed(Input::Empty, c));
    assert_eq!(h.doc.len(), 3);
    let s = h.doc.objects().last().unwrap();
    let bb = s.geometry.bounding_box();
    assert!((bb.min.x - 10.0).abs() < 1e-6 && (bb.max.x - 20.0).abs() < 1e-6, "{bb:?}");
    assert!((bb.max.z - 5.0).abs() < 1e-6 && bb.min.z.abs() < 1e-6);
}

#[test]
fn misc3_batch_commands() {
    // SubCrv keeps the middle part of a line
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.start("SubCrv");
    h.ctx(|r, c| r.feed(Input::Pick(lid, DVec3::new(5.0, 0.0, 0.0)), c));
    h.point(2.0, 0.0, 0.0);
    h.point(7.0, 0.0, 0.0);
    let c = h.doc.get(lid).unwrap().geometry.as_curve().unwrap();
    assert!((c.start_point().x - 2.0).abs() < 1e-6 && (c.end_point().x - 7.0).abs() < 1e-6);
    // ReducePolyline removes collinear vertices
    let mut h = Harness::new();
    h.start("Polyline");
    for x in [0.0, 1.0, 2.0, 3.0] {
        h.point(x, 0.0, 0.0);
    }
    h.point(3.0, 3.0, 0.0);
    h.ctx(|r, c| r.feed(Input::Empty, c));
    let pid = h.doc.objects().next().unwrap().id;
    h.start("ReducePolyline");
    h.objects(vec![pid]);
    assert_eq!(h.doc.get(pid).unwrap().geometry.as_curve().unwrap().control_points.len(), 3);
    // TweenCurves between two lines
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    h.start("Line");
    h.point(0.0, 4.0, 0.0);
    h.point(10.0, 4.0, 0.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("TweenCurves");
    h.objects(ids);
    assert_eq!(h.doc.len(), 3);
    let t = h.doc.objects().last().unwrap().geometry.as_curve().unwrap();
    assert!((t.start_point().y - 2.0).abs() < 1e-9);
    // ExtractIsocurve on a plane
    let mut h = Harness::new();
    h.start("Plane");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    let sid = h.doc.objects().next().unwrap().id;
    h.start("ExtractIsocurve");
    h.ctx(|r, c| r.feed(Input::Pick(sid, DVec3::new(3.0, 4.0, 0.0)), c));
    h.point(3.0, 4.0, 0.0);
    assert_eq!(h.doc.len(), 2);
    // ExtrudeMesh of a mesh plane gives a closed mesh
    let mut h = Harness::new();
    h.start("MeshPlane");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    let mid = h.doc.objects().next().unwrap().id;
    h.start("ExtrudeMesh");
    h.objects(vec![mid]);
    h.number(3.0);
    assert_eq!(h.doc.objects().next().unwrap().geometry.type_name(), "closed mesh");
    // DimRadius on a circle creates a grouped dimension
    let mut h = Harness::new();
    h.start("Circle");
    h.point(0.0, 0.0, 0.0);
    h.number(5.0);
    let cid = h.doc.objects().next().unwrap().id;
    h.start("DimRadius");
    h.ctx(|r, c| r.feed(Input::Pick(cid, DVec3::new(5.0, 0.0, 0.0)), c));
    h.point(8.0, 2.0, 0.0);
    assert!(h.doc.objects().filter(|o| o.attributes.groups.iter().any(|g| g.starts_with("DimRadius"))).count() >= 3);
    // Twist a vertical line by 90 degrees about Z: end moves to the y axis
    let mut h = Harness::new();
    h.start("Line");
    h.point(1.0, 0.0, 0.0);
    h.point(1.0, 0.0, 10.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.start("Twist");
    h.objects(vec![lid]);
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 0.0, 10.0);
    h.number(90.0);
    let c = h.doc.get(lid).unwrap().geometry.as_curve().unwrap();
    assert!((c.end_point() - DVec3::new(0.0, 1.0, 10.0)).length() < 1e-9, "{:?}", c.end_point());
    // Parabola through vertex (0,0), focus (0,1), end at x=4: y = x²/4 = 4
    let mut h = Harness::new();
    h.start("Parabola");
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 1.0, 0.0);
    h.point(4.0, 0.0, 0.0);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!((c.end_point() - DVec3::new(4.0, 4.0, 0.0)).length() < 1e-9, "{:?}", c.end_point());
    assert!((c.point_at(0.5)).length() < 1e-9, "vertex at the origin");
}

#[test]
fn planar_boolean_and_deformations() {
    // PlanarDifference: square minus circle -> one closed outline with a hole, or an outline + hole loop
    let mut h = Harness::new();
    h.start("Rectangle");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.start("Rectangle");
    h.point(5.0, 5.0, 0.0);
    h.point(15.0, 15.0, 0.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("PlanarUnion");
    h.objects(ids.clone());
    assert_eq!(h.doc.len(), 1, "union of two overlapping squares is one region");
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap().clone();
    assert!(c.is_closed());
    let area = Document::area_of(&Geometry::Curve(c)).unwrap();
    assert!((area - 175.0).abs() < 0.5, "area {area}");
    // PlanarIntersection of the same squares -> 25
    let mut h = Harness::new();
    h.start("Rectangle");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.start("Rectangle");
    h.point(5.0, 5.0, 0.0);
    h.point(15.0, 15.0, 0.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("PlanarIntersection");
    h.objects(vec![ids[0]]);
    h.objects(vec![ids[1]]);
    assert_eq!(h.doc.len(), 1);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap().clone();
    let area = Document::area_of(&Geometry::Curve(c)).unwrap();
    assert!((area - 25.0).abs() < 0.5, "area {area}");
    // Stretch: line 0..10 along x, stretch end to 15
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.start("Stretch");
    h.objects(vec![lid]);
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    h.point(15.0, 0.0, 0.0);
    let c = h.doc.get(lid).unwrap().geometry.as_curve().unwrap();
    assert!((c.end_point().x - 15.0).abs() < 1e-9);
    // Conic with rho 0.5 is a parabola through the apex-midpoint
    let mut h = Harness::new();
    h.start("Conic");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 0.0, 0.0);
    h.point(2.0, 4.0, 0.0);
    h.number(0.5);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap();
    assert!((c.point_at(0.5) - DVec3::new(2.0, 2.0, 0.0)).length() < 1e-9);
    // RoundHole through a box -> open the mesh in the middle: volume decreases
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.start("RoundHole");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(5.0, 5.0, 10.0)), c));
    h.point(5.0, 5.0, 10.0);
    h.number(2.0);
    let g = &h.doc.get(bid).unwrap().geometry;
    assert!(matches!(g.type_name().as_str(), "closed mesh" | "closed solid polysurface"), "{}", g.type_name());
    let vol = Document::volume_of(g).unwrap().0;
    let expected = 1000.0 - std::f64::consts::PI * 4.0 * 10.0;
    assert!((vol - expected).abs() < 6.0, "volume {vol} vs {expected}");
}

#[test]
fn circle_options_and_batch5() {
    // Circle Tangent to a line at x=3 with radius 2 -> centre (3, ±2)
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.start("Circle");
    h.option("Tangent", None);
    h.ctx(|r, c| r.feed(Input::Pick(lid, DVec3::new(3.0, 0.0, 0.0)), c));
    h.point(3.0, 0.5, 0.0);
    h.number(2.0);
    let c = h.doc.objects().last().unwrap().geometry.as_curve().unwrap().clone();
    let bb = c.bounding_box();
    assert!((bb.max.x - 5.0).abs() < 1e-6 && (bb.min.x - 1.0).abs() < 1e-6, "{bb:?}");
    assert!(bb.min.y.abs() < 1e-6 || bb.max.y.abs() < 1e-6, "tangent to the line: {bb:?}");
    // Circle FitPoints through three points on a known circle
    let mut h = Harness::new();
    for (x, y) in [(5.0, 0.0), (0.0, 5.0), (-5.0, 0.0), (0.0, -5.0)] {
        h.start("Point");
        h.point(x, y, 0.0);
    }
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("Circle");
    h.option("FitPoints", None);
    h.objects(ids);
    let c = h.doc.objects().last().unwrap().geometry.as_curve().unwrap().clone();
    let bb = c.bounding_box();
    assert!((c.length() - 10.0 * std::f64::consts::PI).abs() < 2e-2, "len {} bbox {:?}", c.length(), bb);
    // Circle with Area option: area 100π -> radius 10
    let mut h = Harness::new();
    h.start("Circle");
    h.point(0.0, 0.0, 0.0);
    h.option("Area", None);
    h.number(100.0 * std::f64::consts::PI);
    let c = h.doc.objects().last().unwrap().geometry.as_curve().unwrap().clone();
    assert!((c.bounding_box().max.x - 10.0).abs() < 1e-6);
    // Arc StartPoint + Center option: start (10,0), end (0,10), centre origin -> quarter circle
    let mut h = Harness::new();
    h.start("Arc");
    h.option("StartPoint", None);
    h.point(10.0, 0.0, 0.0);
    h.point(0.0, 10.0, 0.0);
    h.option("Center", None);
    h.point(0.0, 0.0, 0.0);
    let c = h.doc.objects().last().unwrap().geometry.as_curve().unwrap().clone();
    assert!((c.length() - 5.0 * std::f64::consts::PI).abs() < 2e-2, "len {}", c.length());
    // UnrollSrf of a quarter cylinder -> flat rectangle of width π r / 2 and height h
    let mut h = Harness::new();
    h.start("Arc");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 0.0, 0.0);
    h.point(0.0, 4.0, 0.0);
    let aid = h.doc.objects().next().unwrap().id;
    h.start("ExtrudeCrv");
    h.objects(vec![aid]);
    h.number(3.0);
    let sid = h.doc.objects().last().unwrap().id;
    h.start("UnrollSrf");
    h.objects(vec![sid]);
    let flat = h.doc.objects().last().unwrap();
    let bb = flat.geometry.bounding_box();
    let w = (bb.max.x - bb.min.x).max(bb.max.y - bb.min.y);
    let hh = (bb.max.x - bb.min.x).min(bb.max.y - bb.min.y);
    assert!(bb.max.z.abs() < 1e-9 && bb.min.z.abs() < 1e-9, "flat on the CPlane");
    assert!((w - 2.0 * std::f64::consts::PI).abs() < 0.05, "unrolled width {w}");
    assert!((hh - 3.0).abs() < 1e-6, "height {hh}");
    // MeshSplit of a mesh box by a planar surface at z=5 -> two closed meshes
    let mut h = Harness::new();
    h.start("MeshBox");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    let mid = h.doc.objects().next().unwrap().id;
    h.start("Plane");
    h.point(-5.0, -5.0, 5.0);
    h.point(15.0, 15.0, 5.0);
    let pid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("MeshSplit");
    h.objects(vec![mid]);
    h.ctx(|r, c| r.feed(Input::Pick(pid, DVec3::new(5.0, 5.0, 5.0)), c));
    let meshes: Vec<_> = h.doc.objects().filter(|o| matches!(o.geometry, Geometry::Mesh(_))).collect();
    assert_eq!(meshes.len(), 2, "log {:?} objects {:?}", h.log, h.doc.objects().map(|o| o.geometry.type_name()).collect::<Vec<_>>());
    assert!(meshes.iter().all(|o| o.geometry.type_name() == "closed mesh"));
    // Symmetry joins a mirrored curve with the original
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(5.0, 5.0, 0.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.start("Symmetry");
    h.objects(vec![lid]);
    h.point(0.0, -1.0, 0.0);
    h.point(0.0, 1.0, 0.0);
    assert_eq!(h.doc.len(), 1);
    let c = h.doc.get(lid).unwrap().geometry.as_curve().unwrap();
    assert!((c.bounding_box().min.x + 5.0).abs() < 1e-9 && (c.bounding_box().max.x - 5.0).abs() < 1e-9);
}

#[test]
fn batch6_pull_distribute_merge_filletsrf() {
    // Pull a line onto a sphere: pulled points lie on the sphere
    let mut h = Harness::new();
    h.start("Sphere");
    h.point(0.0, 0.0, 0.0);
    h.number(5.0);
    let sid = h.doc.objects().next().unwrap().id;
    h.start("Line");
    h.point(-3.0, 0.0, 9.0);
    h.point(3.0, 0.0, 9.0);
    let lid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("Pull");
    h.objects(vec![lid]);
    h.ctx(|r, c| r.feed(Input::Pick(sid, DVec3::new(0.0, 0.0, 5.0)), c));
    let pulled = h.doc.objects().last().unwrap().geometry.as_curve().unwrap().clone();
    for i in 0..=10 {
        let p = pulled.point_at(i as f64 / 10.0);
        assert!((p.length() - 5.0).abs() < 0.05, "{p:?}");
    }
    // Distribute three points evenly along X
    let mut h = Harness::new();
    for x in [0.0, 1.0, 10.0] {
        h.start("Point");
        h.point(x, 0.0, 0.0);
    }
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.start("Distribute");
    h.objects(ids);
    let mut xs: Vec<f64> = h.doc.objects().filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p.x) } else { None }).collect();
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!((xs[1] - 5.0).abs() < 1e-9, "{xs:?}");
    // MergeAllCoplanarFaces: a box split in two along x then joined back has 2 coplanar top faces -> merged to 6 faces
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.start("Box");
    h.point(10.0, 0.0, 0.0);
    h.point(20.0, 10.0, 0.0);
    h.number(10.0);
    let bid2 = h.doc.objects().last().unwrap().id;
    // union the two boxes' faces into one polysurface by joining
    let (Geometry::Brep(a), Geometry::Brep(b)) = (h.doc.get(bid).unwrap().geometry.clone(), h.doc.get(bid2).unwrap().geometry.clone()) else { panic!() };
    let mut faces = a.faces.clone();
    // drop the two touching faces at x=10
    faces.retain(|f| (f.sample().0.x - 10.0).abs() > 1e-6);
    faces.extend(b.faces.iter().filter(|f| (f.sample().0.x - 10.0).abs() > 1e-6).cloned());
    let joined = Brep::new(faces);
    assert_eq!(joined.faces.len(), 10);
    h.doc.remove(bid2);
    h.doc.replace_geometry(bid, Geometry::Brep(joined));
    h.doc.clear_selection();
    h.start("MergeAllCoplanarFaces");
    h.objects(vec![bid]);
    let Geometry::Brep(m) = &h.doc.get(bid).unwrap().geometry else { panic!() };
    assert_eq!(m.faces.len(), 6, "coplanar pairs merged");
    assert!(m.closed);
    let vol = Document::volume_of(&h.doc.get(bid).unwrap().geometry).unwrap().0;
    assert!((vol - 2000.0).abs() < 1.0, "volume {vol}");
    // FilletSrf between two perpendicular planes: fillet surface + both planes trimmed
    let mut h = Harness::new();
    h.start("Plane");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    let pa = h.doc.objects().last().unwrap().id;
    let pb = h.doc.add(Geometry::Surface(tri_geom::NurbsSurface::plane(&Plane::world_yz(), 0.0, 10.0, 0.0, 10.0).unwrap()));
    let bb = h.doc.get(pb).unwrap().geometry.bounding_box();
    assert!(bb.max.z > 9.0, "vertical plane built: {bb:?}");
    h.doc.clear_selection();
    h.start("FilletSrf");
    h.option("Radius", Some("2"));
    h.ctx(|r, c| r.feed(Input::Pick(pa, DVec3::new(5.0, 5.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Pick(pb, DVec3::new(0.0, 5.0, 5.0)), c));
    assert_eq!(h.doc.len(), 3, "log {:?}", h.log);
    let fillet = h.doc.objects().last().unwrap().geometry.bounding_box();
    assert!((fillet.max.x - 2.0).abs() < 1e-6 && (fillet.max.z - 2.0).abs() < 1e-6, "fillet bbox {fillet:?}");
    let a_bb = h.doc.get(pa).unwrap().geometry.bounding_box();
    assert!((a_bb.min.x - 2.0).abs() < 1e-6, "plane A trimmed back to x=2: {a_bb:?}");
    let b_bb = h.doc.get(pb).unwrap().geometry.bounding_box();
    assert!((b_bb.min.z - 2.0).abs() < 1e-6, "plane B trimmed back to z=2: {b_bb:?}");
}

#[test]
fn batch7_moveface_usertext_clipping() {
    // MoveFace: lift the top face of a box by 5 -> volume 1500, still closed
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("MoveFace");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(5.0, 5.0, 10.0)), c));
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 0.0, 5.0);
    let g = &h.doc.get(bid).unwrap().geometry;
    let Geometry::Brep(b) = g else { panic!() };
    assert_eq!(b.faces.len(), 6);
    assert!(b.closed, "moved-face box stays closed");
    let vol = Document::volume_of(g).unwrap().0;
    assert!((vol - 1500.0).abs() < 1.0, "volume {vol}");
    // MoveEdge: move the top edge at y=10,z=15 inward by 2 in y -> faces tilt but stay closed
    h.start("MoveEdge");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(5.0, 10.0, 15.0)), c));
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, -2.0, 0.0);
    let Geometry::Brep(b) = &h.doc.get(bid).unwrap().geometry else { panic!() };
    assert!(b.closed, "moved-edge box stays closed");
    // DeleteFaces opens the solid
    h.start("DeleteFaces");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(5.0, 5.0, 0.0)), c));
    h.enter();
    let Geometry::Brep(b) = &h.doc.get(bid).unwrap().geometry else { panic!() };
    assert_eq!(b.faces.len(), 5);
    assert!(!b.closed);
    // user text on objects and the document
    let mut h = Harness::new();
    h.start("Point");
    h.point(0.0, 0.0, 0.0);
    let pid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("SetUserText");
    h.objects(vec![pid]);
    h.ctx(|r, c| r.feed(Input::Text("material".into()), c));
    h.ctx(|r, c| r.feed(Input::Text("steel".into()), c));
    assert_eq!(h.doc.get(pid).unwrap().attributes.user_text, vec![("material".to_string(), "steel".to_string())]);
    h.start("SetDocumentUserText");
    h.ctx(|r, c| r.feed(Input::Text("project".into()), c));
    h.ctx(|r, c| r.feed(Input::Text("demo".into()), c));
    assert_eq!(h.doc.settings.user_text.len(), 1);
    // ClippingPlane creates a plane object and requests registration
    h.start("ClippingPlane");
    h.point(0.0, 0.0, 3.0);
    h.point(10.0, 10.0, 3.0);
    assert!(h.requests.iter().any(|r| matches!(r, AppRequest::AddClippingPlane(_))));
    let cp = h.doc.objects().last().unwrap();
    assert_eq!(cp.attributes.name.as_deref(), Some("Clipping Plane"));
    assert!((cp.geometry.bounding_box().max.z - 3.0).abs() < 1e-9);
    // ExtrudeSrfToPoint on a plane -> closed pyramid
    let mut h = Harness::new();
    h.start("Plane");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    let sid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("ExtrudeSrfToPoint");
    h.objects(vec![sid]);
    h.point(2.0, 2.0, 6.0);
    let g = &h.doc.objects().last().unwrap().geometry;
    assert_eq!(g.type_name(), "closed solid polysurface");
    let vol = Document::volume_of(g).unwrap().0;
    assert!((vol - 32.0).abs() < 0.5, "pyramid volume {vol}");
}

#[test]
fn batch8_mesh_subd_selection_tools() {
    // FillMeshHoles closes an open mesh box
    let mut h = Harness::new();
    h.start("MeshBox");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    h.number(4.0);
    let mid = h.doc.objects().next().unwrap().id;
    if let Some(Geometry::Mesh(m)) = h.doc.get(mid).map(|o| o.geometry.clone()) {
        let mut open = m;
        open.indices.truncate(open.indices.len() - 6);
        open.compute_normals();
        h.doc.replace_geometry(mid, Geometry::Mesh(open));
    }
    assert_eq!(h.doc.get(mid).unwrap().geometry.type_name(), "open mesh");
    h.doc.clear_selection();
    h.start("FillMeshHoles");
    h.objects(vec![mid]);
    assert_eq!(h.doc.get(mid).unwrap().geometry.type_name(), "closed mesh");
    // SubDBox creates a smoothed closed mesh tagged as SubD; SelSubD finds it
    let mut h = Harness::new();
    h.start("SubDBox");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    h.number(4.0);
    let o = h.doc.objects().next().unwrap();
    assert_eq!(o.geometry.type_name(), "closed mesh");
    assert!(o.attributes.user_text.iter().any(|(k, _)| k == "SubD"));
    if let Geometry::Mesh(m) = &o.geometry {
        assert!(m.triangle_count() > 100, "subdivided twice: {}", m.triangle_count());
    }
    h.doc.clear_selection();
    h.start("SelSubD");
    assert_eq!(h.doc.selected_ids().len(), 1);
    // MeshIntersect of two mesh boxes gives curves on the overlap
    let mut h = Harness::new();
    h.start("MeshBox");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    h.number(4.0);
    h.start("MeshBox");
    h.point(2.0, -2.0, -2.0);
    h.point(6.0, 2.0, -2.0);
    h.number(4.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.doc.clear_selection();
    h.start("MeshIntersect");
    h.objects(ids);
    assert!(h.doc.objects().any(|o| o.geometry.is_curve()), "intersection curves created");
    // SelCircular selects the point inside the circle only
    let mut h = Harness::new();
    h.start("Point");
    h.point(0.0, 0.0, 0.0);
    h.start("Point");
    h.point(10.0, 0.0, 0.0);
    h.doc.clear_selection();
    h.start("SelCircular");
    h.point(0.0, 0.0, 0.0);
    h.number(2.0);
    assert_eq!(h.doc.selected_ids().len(), 1);
    // BoxEdit resizes a box to 2x2x2
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    h.number(4.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("BoxEdit");
    h.objects(vec![bid]);
    h.number(2.0);
    h.number(2.0);
    h.number(2.0);
    h.enter();
    h.enter();
    h.enter();
    let bb = h.doc.get(bid).unwrap().geometry.bounding_box();
    assert!((bb.max - DVec3::splat(2.0)).length() < 1e-9 && bb.min.length() < 1e-9, "{bb:?}");
    // draw order and CalcRPN
    h.doc.clear_selection();
    h.start("BringToFront");
    h.objects(vec![bid]);
    assert_eq!(h.doc.get(bid).unwrap().attributes.draw_order, 1);
    h.start("CalcRPN");
    h.ctx(|r, c| r.feed(Input::Text("3 4 + 2 *".into()), c));
    assert!(h.log.iter().any(|l| l.contains("= 14")), "{:?}", h.log.last());
    // every catalog command can be created
    for (name, _) in super::catalog::CATALOG {
        assert!(super::registry::is_implemented(name), "{name} not implemented");
    }
}

#[test]
fn parametric_sketch_commands() {
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 3.0, 0.0);
    let l1 = h.doc.objects().next().unwrap().id;
    h.start("Line");
    h.point(10.0, 3.0, 0.0);
    h.point(12.0, 9.0, 0.0);
    let l2 = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("Parametric");
    h.option("On", None);
    assert!(h.doc.state.parametric.enabled);
    h.start("GcHorizontal");
    h.ctx(|r, c| r.feed(Input::Pick(l1, DVec3::new(5.0, 1.5, 0.0)), c));
    let c = h.doc.get(l1).unwrap().geometry.as_curve().unwrap().clone();
    assert!((c.start_point().y - c.end_point().y).abs() < 1e-6, "horizontal: {:?}", c.end_point());
    h.start("GcCoincident");
    h.ctx(|r, c| r.feed(Input::Pick(l1, DVec3::new(10.0, 3.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Pick(l2, DVec3::new(10.0, 3.0, 0.0)), c));
    h.start("GcPerpendicular");
    h.ctx(|r, c| r.feed(Input::Pick(l1, DVec3::new(5.0, 0.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Pick(l2, DVec3::new(11.0, 6.0, 0.0)), c));
    let a = h.doc.get(l1).unwrap().geometry.as_curve().unwrap().clone();
    let b = h.doc.get(l2).unwrap().geometry.as_curve().unwrap().clone();
    let da = (a.end_point() - a.start_point()).normalize();
    let db = (b.end_point() - b.start_point()).normalize();
    assert!(da.dot(db).abs() < 1e-5, "perpendicular: {da:?} {db:?}");
    assert!((a.end_point() - b.start_point()).length() < 1e-5 || (a.end_point() - b.end_point()).length() < 1e-5, "coincident");
    h.start("DcAligned");
    h.ctx(|r, c| r.feed(Input::Pick(l1, DVec3::new(0.0, 0.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Pick(l1, DVec3::new(10.0, 3.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Text("25".into()), c));
    let a = h.doc.get(l1).unwrap().geometry.as_curve().unwrap().clone();
    assert!((a.length() - 25.0).abs() < 1e-4, "length {}", a.length());
    assert_eq!(h.doc.state.parametric.constraints.len(), 4);
    // parameter edit re-solves
    let name = h.doc.state.parametric.constraints.iter().find(|c| !c.name.is_empty()).map(|c| c.name.clone()).unwrap();
    h.start("Parameters");
    h.option("Set", None);
    h.ctx(|r, c| r.feed(Input::Text(name.clone()), c));
    h.ctx(|r, c| r.feed(Input::Text("40".into()), c));
    let a = h.doc.get(l1).unwrap().geometry.as_curve().unwrap().clone();
    assert!((a.length() - 40.0).abs() < 1e-4, "length {}", a.length());
    // moving a line in parametric mode keeps the constraints after re-solve
    h.doc.clear_selection();
    h.start("Move");
    h.objects(vec![l2]);
    h.point(0.0, 0.0, 0.0);
    h.point(3.0, 2.0, 0.0);
    let r = h.doc.solve_constraints(&[l2]).expect("solve after move");
    let _ = r;
    let a = h.doc.get(l1).unwrap().geometry.as_curve().unwrap().clone();
    let b = h.doc.get(l2).unwrap().geometry.as_curve().unwrap().clone();
    assert!((a.end_point() - b.start_point()).length() < 1e-4 || (a.end_point() - b.end_point()).length() < 1e-4, "still coincident after move");
    h.start("DelConstraint");
    h.option("All", None);
    assert!(h.doc.state.parametric.constraints.is_empty());
}

#[test]
fn render_batch_linetypes_materials_lights() {
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("SetLinetype");
    h.objects(vec![lid]);
    h.ctx(|r, c| r.feed(Input::Text("Dashed".into()), c));
    assert_eq!(h.doc.get(lid).unwrap().attributes.linetype.as_deref(), Some("Dashed"));
    let pat = h.doc.linetype_pattern("Dashed").unwrap();
    let dashes = crate::app::dash_segments(&[DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0)], &pat, 1.0);
    assert_eq!(dashes.len(), 4, "two dashes of 4 units in 10: {dashes:?}");
    h.start("ExtractLineTypeSegments");
    h.objects(vec![lid]);
    assert_eq!(h.doc.objects().filter(|o| o.geometry.is_curve()).count(), 3);
    // materials
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(4.0, 4.0, 0.0);
    h.number(4.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("RenderAssignMaterialToObjects");
    h.objects(vec![bid]);
    h.ctx(|r, c| r.feed(Input::Text("Steel".into()), c));
    assert_eq!(h.doc.settings.materials.len(), 1);
    assert_eq!(h.doc.material_of(h.doc.get(bid).unwrap()).name, "Steel");
    h.start("ApplyBoxMapping");
    h.objects(vec![bid]);
    h.number(2.0);
    assert_eq!(h.doc.get(bid).unwrap().attributes.mapping.map(|m| m.0), Some(tri_doc::MappingKind::Box));
    h.start("SelMaterialName");
    h.ctx(|r, c| r.feed(Input::Text("Steel".into()), c));
    assert!(h.doc.is_selected(bid));
    // lights
    h.start("PointLight");
    h.point(5.0, 5.0, 10.0);
    h.start("Spotlight");
    h.point(0.0, 0.0, 10.0);
    h.point(2.0, 2.0, 0.0);
    assert_eq!(h.doc.lights().len(), 2);
    h.doc.clear_selection();
    h.start("SelLight");
    assert_eq!(h.doc.selected_ids().len(), 2);
}

#[test]
fn history_cage_and_holes() {
    use crate::app::{App, Layout};
    // History: ExtrudeCrv recorded, moving the curve re-extrudes
    let mut app = App::new();
    app.layout = Layout::Single;
    app.aids.record_history = true;
    app.start_command("Line");
    app.feed_input(Input::Point(DVec3::ZERO));
    app.feed_input(Input::Point(DVec3::new(10.0, 0.0, 0.0)));
    let lid = app.doc.objects().next().unwrap().id;
    app.doc.clear_selection();
    app.start_command("ExtrudeCrv");
    app.feed_input(Input::Objects(vec![lid]));
    app.feed_input(Input::Number(5.0));
    app.process_requests();
    assert_eq!(app.doc.state.history.records.len(), 1, "log {:?}", app.log);
    let child = app.doc.state.history.records[0].outputs[0];
    assert!(app.doc.get(child).is_some());
    // move the parent line: the surface is rebuilt
    app.doc.clear_selection();
    app.start_command("Move");
    app.feed_input(Input::Objects(vec![lid]));
    app.feed_input(Input::Point(DVec3::ZERO));
    app.feed_input(Input::Point(DVec3::new(0.0, 7.0, 0.0)));
    let rec = &app.doc.state.history.records[0];
    let new_child = rec.outputs[0];
    let bb = app.doc.get(new_child).expect("child replaced").geometry.bounding_box();
    assert!((bb.min.y - 7.0).abs() < 1e-9 && (bb.max.z - 5.0).abs() < 1e-9, "rebuilt child {bb:?}");
    // Cage: capture a point, move a cage corner, captive moves
    let mut h = Harness::new();
    h.start("Point");
    h.point(5.0, 5.0, 5.0);
    let pid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("Cage");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    h.number(2.0);
    h.number(2.0);
    h.number(2.0);
    assert_eq!(h.doc.len(), 9);
    let corner = h.doc.objects().find(|o| matches!(o.geometry, Geometry::Point(p) if p == DVec3::ZERO) && o.attributes.user_text.iter().any(|(k, _)| k == "CageIndex")).unwrap().id;
    h.doc.clear_selection();
    h.start("CageEdit");
    h.objects(vec![pid]);
    h.ctx(|r, c| r.feed(Input::Pick(corner, DVec3::ZERO), c));
    assert_eq!(h.doc.state.history.cages.len(), 1);
    // move every cage point by +2 in x: trilinear FFD translates the captive
    let pts: Vec<_> = h.doc.state.history.cages[0].points.clone();
    for id in &pts {
        let Geometry::Point(p) = h.doc.get(*id).unwrap().geometry else { panic!() };
        h.doc.replace_geometry(*id, Geometry::Point(p + DVec3::X * 2.0));
    }
    let cage = h.doc.state.history.cages[0].clone();
    let current: Vec<DVec3> = pts.iter().map(|id| match h.doc.get(*id).unwrap().geometry { Geometry::Point(p) => p, _ => unreachable!() }).collect();
    let moved = cage.deform_point(&current, DVec3::new(5.0, 5.0, 5.0));
    assert!((moved - DVec3::new(7.0, 5.0, 5.0)).length() < 1e-9, "{moved:?}");
    // Holes: RoundHole then MoveHole re-cuts from the original
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(20.0, 20.0, 0.0);
    h.number(5.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("RoundHole");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(5.0, 5.0, 5.0)), c));
    h.point(5.0, 5.0, 5.0);
    h.number(2.0);
    assert_eq!(h.doc.state.history.holes.len(), 1);
    let hole_curve = h.doc.state.history.holes[0].curves[0];
    let v1 = Document::volume_of(&h.doc.get(bid).unwrap().geometry).unwrap().0;
    assert!((v1 - (2000.0 - std::f64::consts::PI * 4.0 * 5.0)).abs() < 5.0, "volume {v1}");
    h.start("CopyHole");
    h.ctx(|r, c| r.feed(Input::Pick(hole_curve, DVec3::new(7.0, 5.0, 5.0)), c));
    h.point(0.0, 0.0, 0.0);
    h.point(8.0, 0.0, 0.0);
    assert_eq!(h.doc.state.history.holes[0].curves.len(), 2);
    let v2 = Document::volume_of(&h.doc.get(bid).unwrap().geometry).unwrap().0;
    assert!(v2 < v1 - 50.0, "second hole cut: {v2} < {v1}");
    h.start("MoveHole");
    h.ctx(|r, c| r.feed(Input::Pick(hole_curve, DVec3::new(7.0, 5.0, 5.0)), c));
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 10.0, 0.0);
    let v3 = Document::volume_of(&h.doc.get(bid).unwrap().geometry).unwrap().0;
    assert!((v3 - v2).abs() < 5.0, "moved hole keeps the volume: {v3} vs {v2}");
}

#[test]
fn curves3_and_surfaces3_batches() {
    // Catenary endpoints and sag
    let mut h = Harness::new();
    h.start("Catenary");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 0.0, 0.0);
    h.number(2.0);
    let c = h.doc.objects().next().unwrap().geometry.as_curve().unwrap().clone();
    assert!(c.start_point().length() < 1e-6 && (c.end_point() - DVec3::new(10.0, 0.0, 0.0)).length() < 1e-6);
    assert!((c.point_at(0.5).y + 2.0).abs() < 0.05, "sag {:?}", c.point_at(0.5));
    // ModifyRadius
    let mut h = Harness::new();
    h.start("Circle");
    h.point(0.0, 0.0, 0.0);
    h.number(3.0);
    let cid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("ModifyRadius");
    h.ctx(|r, c| r.feed(Input::Pick(cid, DVec3::new(3.0, 0.0, 0.0)), c));
    h.number(5.0);
    let c = h.doc.get(cid).unwrap().geometry.as_curve().unwrap();
    assert!((c.bounding_box().max.x - 5.0).abs() < 1e-9);
    // FilletCorners on a square polyline
    let mut h = Harness::new();
    h.start("Rectangle");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    let rid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("FilletCorners");
    h.objects(vec![rid]);
    h.number(2.0);
    let c = h.doc.get(rid).unwrap().geometry.as_curve().unwrap();
    assert!(c.is_closed());
    assert!((c.length() - (40.0 - 16.0 + 2.0 * std::f64::consts::PI * 2.0)).abs() < 0.1, "len {}", c.length());
    // ArcBlend between two lines
    let mut h = Harness::new();
    h.start("Line");
    h.point(0.0, 0.0, 0.0);
    h.point(5.0, 0.0, 0.0);
    h.start("Line");
    h.point(10.0, 5.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.doc.clear_selection();
    h.start("ArcBlend");
    h.ctx(|r, c| r.feed(Input::Pick(ids[0], DVec3::new(5.0, 0.0, 0.0)), c));
    h.ctx(|r, c| r.feed(Input::Pick(ids[1], DVec3::new(10.0, 5.0, 0.0)), c));
    assert_eq!(h.doc.len(), 3);
    let b = h.doc.objects().last().unwrap().geometry.as_curve().unwrap();
    assert!((b.start_point() - DVec3::new(5.0, 0.0, 0.0)).length() < 1e-6 && (b.end_point() - DVec3::new(10.0, 5.0, 0.0)).length() < 1e-6);
    // ConvertToSingleSpans on a 2-span surface
    let mut h = Harness::new();
    let pts: Vec<Vec<DVec3>> = (0..6).map(|i| (0..6).map(|j| DVec3::new(i as f64 * 5.0, j as f64 * 5.0, (i * j) as f64 * 0.2)).collect()).collect();
    let s = tri_geom::NurbsSurface::interpolate_grid(&pts, 3, 3).unwrap();
    let sid = h.doc.add(Geometry::Surface(s));
    h.start("ConvertToSingleSpans");
    h.objects(vec![sid]);
    assert!(h.doc.len() >= 2, "surface split into spans: {}", h.doc.len());
    // TruncatedPyramid is a closed solid
    let mut h = Harness::new();
    h.start("TruncatedPyramid");
    h.point(0.0, 0.0, 0.0);
    h.point(5.0, 0.0, 0.0);
    h.number(4.0);
    h.number(6.0);
    h.number(0.5);
    assert_eq!(h.doc.objects().next().unwrap().geometry.type_name(), "closed solid polysurface");
    // Drape over a box gives a surface at the box top in the middle
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(5.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("Drape");
    h.objects(vec![bid]);
    h.point(-5.0, -5.0, 0.0);
    h.point(15.0, 15.0, 0.0);
    h.number(30.0);
    let d = h.doc.objects().last().unwrap();
    let Geometry::Surface(s) = &d.geometry else { panic!("drape surface") };
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    let top = s.point_at((u0 + u1) * 0.5, (v0 + v1) * 0.5);
    assert!((top.z - 5.0).abs() < 0.6, "drape top {top:?}");
    // PushPull a box face outwards: volume grows
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("PushPull");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(5.0, 5.0, 10.0)), c));
    h.number(5.0);
    let v = Document::volume_of(&h.doc.get(bid).unwrap().geometry).unwrap().0;
    assert!((v - 1500.0).abs() < 5.0, "volume {v}");
}


#[test]
fn mesh3_batch() {
    // MeshRepair closes a box with a missing triangle
    let mut h = Harness::new();
    let mut m = tri_geom::Mesh::cuboid(DVec3::ZERO, DVec3::splat(10.0));
    m.indices.truncate(m.indices.len() - 3);
    assert!(!m.is_closed());
    let id = h.doc.add(Geometry::Mesh(m));
    h.start("MeshRepair");
    h.objects(vec![id]);
    h.option("All", None);
    let Geometry::Mesh(r) = &h.doc.get(id).unwrap().geometry else { panic!() };
    assert!(r.is_closed(), "repaired mesh is closed");
    let v = Document::volume_of(&Geometry::Mesh(r.clone())).unwrap().0;
    assert!((v - 1000.0).abs() < 1.0, "volume {v}");
    // ExtrudeSubD pulls the top face up by 5: volume 1500
    let mut h = Harness::new();
    let id = h.doc.add(Geometry::Mesh(tri_geom::Mesh::cuboid(DVec3::ZERO, DVec3::splat(10.0))));
    h.start("ExtrudeSubD");
    h.ctx(|r, c| r.feed(Input::Pick(id, DVec3::new(5.0, 5.0, 10.0)), c));
    h.number(5.0);
    let Geometry::Mesh(r) = &h.doc.get(id).unwrap().geometry else { panic!() };
    assert!(r.is_closed(), "extruded mesh is closed");
    let v = Document::volume_of(&Geometry::Mesh(r.clone())).unwrap().0;
    assert!((v - 1500.0).abs() < 1.0, "volume {v}");
    // Reflect keeps the +X half and mirrors it: still a closed 10x10x10 box
    let mut h = Harness::new();
    let id = h.doc.add(Geometry::Mesh(tri_geom::Mesh::cuboid(DVec3::new(-5.0, 0.0, 0.0), DVec3::new(5.0, 10.0, 10.0))));
    h.doc.clear_selection();
    h.start("Reflect");
    h.objects(vec![id]);
    h.point(0.0, 0.0, 0.0);
    h.point(0.0, 10.0, 0.0);
    let Geometry::Mesh(r) = &h.doc.get(id).unwrap().geometry else { panic!() };
    assert!(r.is_closed(), "reflected mesh is closed");
    let v = Document::volume_of(&Geometry::Mesh(r.clone())).unwrap().0;
    assert!((v - 1000.0).abs() < 1.0, "volume {v}");
    // Inset keeps the mesh closed and the volume
    let mut h = Harness::new();
    let id = h.doc.add(Geometry::Mesh(tri_geom::Mesh::cuboid(DVec3::ZERO, DVec3::splat(10.0))));
    h.start("Inset");
    h.ctx(|r, c| r.feed(Input::Pick(id, DVec3::new(5.0, 5.0, 10.0)), c));
    h.number(1.0);
    let Geometry::Mesh(r) = &h.doc.get(id).unwrap().geometry else { panic!() };
    assert!(r.is_closed(), "inset mesh is closed");
    assert!(r.triangle_count() > 12);
    // BlockEdit round trip: explode, move, redefine
    let mut h = Harness::new();
    h.start("Circle");
    h.point(0.0, 0.0, 0.0);
    h.number(2.0);
    let cid = h.doc.objects().next().unwrap().id;
    h.start("Block");
    h.objects(vec![cid]);
    h.point(0.0, 0.0, 0.0);
    h.ctx(|r, c| r.feed(Input::Text("B1".into()), c));
    h.enter();
    let inst = h.doc.objects().find(|o| matches!(o.geometry, Geometry::Instance { .. })).map(|o| o.id).expect("instance");
    h.doc.clear_selection();
    h.start("BlockEdit");
    h.objects(vec![inst]);
    let edit: Vec<_> = h.doc.objects().filter(|o| o.attributes.user_text.iter().any(|(k, _)| k == "BlockEdit")).map(|o| o.id).collect();
    assert_eq!(edit.len(), 1);
    h.doc.transform_objects(&edit, &glam::DAffine3::from_translation(DVec3::new(5.0, 0.0, 0.0)));
    h.doc.clear_selection();
    h.start("BlockEdit");
    assert!(h.doc.objects().any(|o| matches!(o.geometry, Geometry::Instance { .. })));
    let def = &h.doc.state.blocks[0];
    assert!((def.objects[0].0.bounding_box().center().x - 5.0).abs() < 1e-6, "block redefined with the moved circle");
    // ClippingSections: a clipping plane through a box gives a section curve
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    h.doc.clear_selection();
    let plane = tri_geom::NurbsSurface::plane(&tri_geom::Plane::from_normal(DVec3::new(5.0, 5.0, 4.3), DVec3::Z), -15.0, 15.0, -15.0, 15.0).unwrap();
    let mut attrs = tri_doc::Attributes::default();
    attrs.user_text.push(("ClippingPlane".into(), "Yes".into()));
    h.doc.add_with(Geometry::Surface(plane), attrs);
    let n = h.doc.len();
    h.start("ClippingSections");
    assert!(h.doc.len() > n, "section curves created");
    let secs: Vec<f64> = h.doc.objects().filter(|o| o.attributes.user_text.iter().any(|(k, _)| k == "ClippingSection")).filter_map(|o| o.geometry.as_curve()).map(|c| c.length()).collect();
    let total: f64 = secs.iter().sum();
    assert!((total - 40.0).abs() < 0.5, "section length {total} pieces {secs:?} objects {}", h.doc.len());
}

#[test]
fn booleans_and_holes_keep_nurbs_polysurfaces() {
    // BooleanDifference box - cylinder: a closed polysurface whose faces are still NURBS surfaces
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(20.0, 20.0, 0.0);
    h.number(10.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("Cylinder");
    h.point(10.0, 10.0, -5.0);
    h.number(4.0);
    h.number(20.0);
    let cid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("BooleanDifference");
    h.objects(vec![bid]);
    h.objects(vec![cid]);
    assert_eq!(h.doc.len(), 1);
    let r = h.doc.objects().next().unwrap();
    let Geometry::Brep(b) = &r.geometry else { panic!("expected a polysurface, got {}", r.geometry.type_name()) };
    assert!(b.closed, "result is closed");
    assert!(b.faces.iter().any(|f| f.surface.degree_u == 2 || f.surface.degree_v == 2), "the cylinder wall stays a NURBS face");
    let v = Document::volume_of(&r.geometry).unwrap().0;
    let expect = 4000.0 - std::f64::consts::PI * 16.0 * 10.0;
    assert!((v - expect).abs() < expect * 0.02, "volume {v} vs {expect}");
    // BooleanUnion of two boxes stays a polysurface
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    h.start("Box");
    h.point(5.0, 5.0, 5.0);
    h.point(15.0, 15.0, 5.0);
    h.number(10.0);
    let ids: Vec<_> = h.doc.objects().map(|o| o.id).collect();
    h.doc.clear_selection();
    h.start("BooleanUnion");
    h.objects(ids);
    let r = h.doc.objects().next().unwrap();
    assert!(matches!(&r.geometry, Geometry::Brep(b) if b.closed));
    let v = Document::volume_of(&r.geometry).unwrap().0;
    assert!((v - 1875.0).abs() < 2.0, "union volume {v}");
    // RoundHole keeps the polysurface and cuts through
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(20.0, 20.0, 0.0);
    h.number(5.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("RoundHole");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(10.0, 10.0, 5.0)), c));
    h.point(10.0, 10.0, 5.0);
    h.number(3.0);
    let r = h.doc.get(bid).unwrap();
    assert!(matches!(&r.geometry, Geometry::Brep(b) if b.closed), "hole target stays a closed polysurface: {}", r.geometry.type_name());
    let v = Document::volume_of(&r.geometry).unwrap().0;
    let expect = 2000.0 - std::f64::consts::PI * 9.0 * 5.0;
    assert!((v - expect).abs() < expect * 0.02, "hole volume {v} vs {expect}");
}

#[test]
fn shell_and_wirecut_give_polysurfaces() {
    // Shell: box 10x10x10, top face removed, thickness 1 -> closed polysurface, volume 424
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(10.0, 10.0, 0.0);
    h.number(10.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("Shell");
    h.ctx(|r, c| r.feed(Input::Pick(bid, DVec3::new(5.0, 5.0, 10.0)), c));
    h.enter();
    h.number(1.0);
    let g = &h.doc.get(bid).unwrap().geometry;
    assert!(matches!(g, Geometry::Brep(b) if b.closed), "shell is a closed polysurface: {}", g.type_name());
    let v = Document::volume_of(g).unwrap().0;
    assert!((v - 424.0).abs() < 2.0, "shell volume {v}");
    // WireCut with an open line across a box: two closed polysurfaces
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(20.0, 10.0, 0.0);
    h.number(5.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.start("Line");
    h.point(7.0, -5.0, 0.0);
    h.point(7.0, 15.0, 0.0);
    let lid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("WireCut");
    h.ctx(|r, c| r.feed(Input::Pick(lid, DVec3::new(7.0, 0.0, 0.0)), c));
    h.objects(vec![bid]);
    assert!(h.prompt().contains("Cut depth"), "depth prompt: {}", h.prompt());
    h.enter();
    let parts: Vec<_> = h.doc.objects().filter(|o| o.id != lid).collect();
    assert_eq!(parts.len(), 2, "two parts");
    let mut vols: Vec<f64> = parts.iter().map(|o| { assert!(matches!(&o.geometry, Geometry::Brep(b) if b.closed), "{}", o.geometry.type_name()); Document::volume_of(&o.geometry).unwrap().0 }).collect();
    vols.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert!((vols[0] - 350.0).abs() < 1.0 && (vols[1] - 650.0).abs() < 1.0, "volumes {vols:?}");
}

/// Loft between an arc and a line: a curved, non-planar surface for the projection tests.
fn loft_surface() -> tri_geom::NurbsSurface {
    use tri_geom::{NurbsCurve, NurbsSurface};
    let arc = NurbsCurve::arc(&Plane::world_xz(), 10.0, 0.0, std::f64::consts::PI).unwrap();
    let line = NurbsCurve::line(DVec3::new(10.0, 20.0, 3.0), DVec3::new(-10.0, 20.0, 3.0)).unwrap();
    NurbsSurface::loft(&[arc, line], 1).unwrap()
}

#[test]
fn trim_loft_with_a_line_follows_the_projection() {
    let mut h = Harness::new();
    let sid = h.doc.add(Geometry::Surface(loft_surface()));
    h.start("Line");
    h.point(-15.0, 10.0, 0.0);
    h.point(15.0, 10.0, 0.0);
    let lid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    // Trim: remove the far half (pick at y = 15, projected along the view direction -Z)
    h.start("Trim");
    h.objects(vec![lid]);
    h.ctx(|r, c| r.feed(Input::Pick(sid, DVec3::new(0.0, 15.0, 3.0)), c));
    h.enter();
    let g = &h.doc.get(sid).unwrap().geometry;
    let Geometry::Brep(b) = g else { panic!("trimmed surface is a polysurface: {}", g.type_name()) };
    assert_eq!(b.faces.len(), 1);
    let bb = g.bounding_box();
    assert!(bb.max.y < 10.0 + 1e-3 && bb.min.y < 1.0, "trimmed bbox {bb:?}");
    // the new edge lies on y = 10 within tolerance
    let f = &b.faces[0];
    for l in &f.loops {
        for uv in l {
            let p = f.surface.point_at(uv.x, uv.y);
            let on_border = uv.y.abs() < 1e-9 || uv.x.abs() < 1e-9 || (uv.x - 1.0).abs() < 1e-9;
            assert!(on_border || (p.y - 10.0).abs() < 2e-3, "loop vertex off the cut line: {p}");
        }
    }
    // Split the rest with a diagonal line: two surfaces
    h.start("Line");
    h.point(-12.0, 2.0, 0.0);
    h.point(12.0, 8.0, 0.0);
    let lid2 = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("Split");
    h.objects(vec![sid]);
    h.objects(vec![lid2]);
    let breps = h.doc.objects().filter(|o| matches!(o.geometry, Geometry::Brep(_))).count();
    assert_eq!(breps, 2, "split loft into two surfaces");
}

#[test]
fn wirecut_open_surface_and_direction_options() {
    // WireCut of an open loft with a line: two trimmed surfaces, one each side of the wire
    let mut h = Harness::new();
    let sid = h.doc.add(Geometry::Surface(loft_surface()));
    h.start("Line");
    h.point(-15.0, 10.0, 0.0);
    h.point(15.0, 10.0, 0.0);
    let lid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("WireCut");
    h.ctx(|r, c| r.feed(Input::Pick(lid, DVec3::new(0.0, 10.0, 0.0)), c));
    h.objects(vec![sid]);
    let prompt = h.prompt();
    assert!(prompt.contains("Direction") && prompt.contains("KeepAll"), "options in the prompt: {prompt}");
    h.enter();
    let parts: Vec<_> = h.doc.objects().filter(|o| o.id != lid).collect();
    assert_eq!(parts.len(), 2, "two surfaces");
    let mut ys: Vec<(f64, f64)> = parts.iter().map(|o| { let bb = o.geometry.bounding_box(); (bb.min.y, bb.max.y) }).collect();
    ys.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    assert!(ys[0].1 < 10.0 + 1e-3 && ys[1].0 > 10.0 - 1e-3, "pieces split at y=10: {ys:?}");
    // a box cut with Direction=View (looking down -Z the result is the same as CPlaneNormal)
    let mut h = Harness::new();
    h.start("Box");
    h.point(0.0, 0.0, 0.0);
    h.point(20.0, 10.0, 0.0);
    h.number(5.0);
    let bid = h.doc.objects().next().unwrap().id;
    h.start("Line");
    h.point(7.0, -5.0, 0.0);
    h.point(7.0, 15.0, 0.0);
    let lid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("WireCut");
    h.ctx(|r, c| r.feed(Input::Pick(lid, DVec3::new(7.0, 0.0, 0.0)), c));
    h.objects(vec![bid]);
    h.option("Direction", None);
    assert!(h.prompt().contains("Direction=View"), "{}", h.prompt());
    // depth limited to z 1..4: the cut only goes partway, so the solid stays one piece
    h.point(0.0, 0.0, 1.0);
    h.point(0.0, 0.0, 4.0);
    let parts: Vec<_> = h.doc.objects().filter(|o| o.id != lid).collect();
    assert!(parts.len() == 2 || parts.len() == 1, "parts {}", parts.len());
    if parts.len() == 2 {
        let mut vols: Vec<f64> = parts.iter().map(|o| Document::volume_of(&o.geometry).unwrap().0).collect();
        vols.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert!((vols[0] + vols[1] - 1000.0).abs() < 2.0, "volumes {vols:?}");
    }
}

#[test]
fn trim_edgesrf_like_the_script() {
    let mut h = Harness::new();
    h.start("InterpCrv");
    for p in [(-10.0, 0.0, 0.0), (-5.0, 0.0, 6.0), (0.0, 0.0, 8.0), (5.0, 0.0, 6.0), (10.0, 0.0, 0.0)] {
        h.point(p.0, p.1, p.2);
    }
    h.enter();
    h.start("Line");
    h.point(10.0, 20.0, 3.0);
    h.point(-10.0, 20.0, 3.0);
    let ids: Vec<tri_doc::ObjectId> = h.doc.objects().map(|o| o.id).collect();
    assert_eq!(ids.len(), 2, "curve and line");
    h.start("EdgeSrf");
    h.objects(ids.clone());
    let sid = h.doc.objects().find(|o| matches!(o.geometry, Geometry::Surface(_))).expect("surface").id;
    h.doc.clear_selection();
    h.start("Line");
    h.point(-15.0, 10.0, 0.0);
    h.point(15.0, 10.0, 0.0);
    let lid = h.doc.objects().last().unwrap().id;
    h.doc.clear_selection();
    h.start("Trim");
    h.objects(vec![lid]);
    h.ctx(|r, c| r.feed(Input::Pick(sid, DVec3::new(0.0, 15.0, 3.0)), c));
    h.enter();
    let bb = h.doc.get(sid).unwrap().geometry.bounding_box();
    assert!(bb.max.y < 10.0 + 1e-3, "far half removed: {bb:?}; log {:?}", h.log);
}

#[test]
fn array_polar_axis_step_angle_and_z_offset() {
    // Axis option: array around the X axis; StepAngle 90 with 3 items; ZOffset 2 per step
    let mut h = Harness::new();
    h.start("Point");
    h.point(0.0, 1.0, 0.0);
    let pid = h.doc.objects().next().unwrap().id;
    h.start("ArrayPolar");
    h.objects(vec![pid]);
    h.option("Axis", None);
    h.point(0.0, 0.0, 0.0);
    h.point(1.0, 0.0, 0.0);
    h.number(3.0);
    assert!(h.prompt().contains("StepAngle") && h.prompt().contains("ZOffset"), "{}", h.prompt());
    h.option("ZOffset", None);
    h.number(2.0);
    h.option("StepAngle", None);
    h.number(90.0);
    h.enter();
    assert_eq!(h.doc.len(), 3);
    let pts: Vec<DVec3> = h.doc.objects().filter_map(|o| if let Geometry::Point(p) = o.geometry { Some(p) } else { None }).collect();
    // rotation of (0,1,0) about X by 90 deg gives (0,0,1), moved 2 along the axis -> (2,0,1)
    assert!(pts.iter().any(|p| (*p - DVec3::new(2.0, 0.0, 1.0)).length() < 1e-9), "{pts:?}");
    assert!(pts.iter().any(|p| (*p - DVec3::new(4.0, -1.0, 0.0)).length() < 1e-9), "{pts:?}");
    // Rotate=No keeps the orientation: a line arrayed 180 deg with 2 items stays parallel
    let mut h = Harness::new();
    h.start("Line");
    h.point(2.0, 0.0, 0.0);
    h.point(3.0, 0.0, 0.0);
    let lid = h.doc.objects().next().unwrap().id;
    h.doc.clear_selection();
    h.start("ArrayPolar");
    h.objects(vec![lid]);
    h.point(0.0, 0.0, 0.0);
    h.number(2.0);
    h.option("Rotate", None);
    assert!(h.prompt().contains("Rotate=No"), "{}", h.prompt());
    h.number(180.0);
    h.enter();
    let lines: Vec<tri_geom::NurbsCurve> = h.doc.objects().filter_map(|o| o.geometry.as_curve().cloned()).collect();
    assert_eq!(lines.len(), 2);
    let d1 = (lines[1].end_point() - lines[1].start_point()).normalize();
    assert!((d1 - DVec3::X).length() < 1e-9, "copy keeps its direction: {d1}");
    let c1 = (lines[1].start_point() + lines[1].end_point()) * 0.5;
    assert!((c1 - DVec3::new(-2.5, 0.0, 0.0)).length() < 1e-9, "copy centre on the circle: {c1}");
}

#[test]
fn join_merges_meshes() {
    let mut h = Harness::new();
    let a = h.doc.add(Geometry::Mesh(tri_geom::Mesh::cuboid(DVec3::ZERO, DVec3::splat(1.0))));
    let b = h.doc.add(Geometry::Mesh(tri_geom::Mesh::cuboid(DVec3::splat(2.0), DVec3::splat(3.0))));
    let (va, vb) = [a, b].map(|id| match &h.doc.get(id).unwrap().geometry {
        Geometry::Mesh(m) => m.vertices.len(),
        _ => unreachable!(),
    }).into();
    h.start("Join");
    h.objects(vec![a, b]);
    let meshes: Vec<_> = h.doc.objects().filter_map(|o| match &o.geometry { Geometry::Mesh(m) => Some(m.clone()), _ => None }).collect();
    assert_eq!(meshes.len(), 1, "two meshes join into one");
    assert_eq!(meshes[0].vertices.len(), va + vb);
}


// ---- exact solids on solidcore (feature `kernel-solidcore`) ----

#[cfg(feature = "kernel-solidcore")]
mod exact_solids {
    use super::*;
    use tri_geom::Brep;
    use tri_kernel::Solid;

    fn add(h: &mut Harness, s: &Solid) -> tri_doc::ObjectId {
        h.doc.add(Geometry::Brep(s.to_brep(0.05).unwrap()))
    }

    fn only_brep(h: &Harness) -> Brep {
        assert_eq!(h.doc.len(), 1, "log: {:?}", h.log);
        match &h.doc.objects().next().unwrap().geometry {
            Geometry::Brep(b) => b.clone(),
            other => panic!("expected a polysurface, got {}", other.type_name()),
        }
    }

    fn kernel_volume(b: &Brep) -> f64 {
        Solid::from_exact(b).expect("the result carries a valid exact body").volume().unwrap()
    }

    #[test]
    fn exact_sphere_minus_two_cylinders() {
        let mut h = Harness::new();
        let s = add(&mut h, &Solid::sphere(DVec3::ZERO, 20.0).unwrap());
        let c1 = add(&mut h, &Solid::cylinder(DVec3::new(5.0, -30.0, 3.0), DVec3::Y, 8.0, 60.0).unwrap());
        let c2 = add(&mut h, &Solid::cylinder(DVec3::new(-30.0, 0.0, 8.0), DVec3::new(1.0, 0.3, 0.1), 6.0, 60.0).unwrap());
        h.start("BooleanDifference");
        h.objects(vec![s]);
        h.objects(vec![c1, c2]);
        let b = only_brep(&h);
        assert!(b.closed);
        let v = kernel_volume(&b);
        // Monte Carlo reference 23658.8 (8e6 samples: +-0.03 %); the legacy Boolean gave 26268 (+11 %)
        assert!((v - 23658.8).abs() < 23658.8 * 0.001, "volume {v}");
    }

    #[test]
    fn exact_torus_minus_a_bore_along_its_tube() {
        let mut h = Harness::new();
        let t = add(&mut h, &Solid::torus(DVec3::ZERO, DVec3::Z, 14.0, 6.0).unwrap());
        let c = add(&mut h, &Solid::cylinder(DVec3::new(14.0, -20.0, 0.0), DVec3::Y, 3.0, 40.0).unwrap());
        h.start("BooleanDifference");
        h.objects(vec![t]);
        h.objects(vec![c]);
        let v = kernel_volume(&only_brep(&h));
        // Monte Carlo reference 9163.74; the legacy Boolean gave 9704.6 (+5.9 %)
        assert!((v - 9163.74).abs() < 9163.74 * 0.001, "volume {v}");
    }

    #[test]
    fn legacy_operands_keep_the_legacy_path() {
        let mut h = Harness::new();
        let a = h.doc.add(Geometry::Brep(Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap()));
        let moved = Plane { origin: DVec3::new(5.0, 5.0, 5.0), ..Plane::world_xy() };
        let b = h.doc.add(Geometry::Brep(Brep::cuboid(&moved, 10.0, 10.0, 10.0).unwrap()));
        h.start("BooleanUnion");
        h.objects(vec![a, b]);
        let r = only_brep(&h);
        assert!(r.exact.is_none(), "legacy operands give a legacy result");
        let v = Document::volume_of(&Geometry::Brep(r)).unwrap().0;
        assert!((v - 1875.0).abs() < 2.0, "union volume {v}");
    }

    #[test]
    fn an_exact_and_a_legacy_operand_take_the_legacy_path() {
        let mut h = Harness::new();
        let a = add(&mut h, &Solid::cuboid(DVec3::ZERO, DVec3::splat(10.0)).unwrap());
        let moved = Plane { origin: DVec3::new(5.0, 5.0, 5.0), ..Plane::world_xy() };
        let b = h.doc.add(Geometry::Brep(Brep::cuboid(&moved, 10.0, 10.0, 10.0).unwrap()));
        h.start("BooleanUnion");
        h.objects(vec![a, b]);
        assert!(only_brep(&h).exact.is_none());
    }

    #[test]
    fn a_stale_exact_body_is_not_used() {
        let mut h = Harness::new();
        let mut brep = Solid::cuboid(DVec3::ZERO, DVec3::splat(10.0)).unwrap().to_brep(0.05).unwrap();
        brep.faces[0].surface.control_points[0].x += 0.01; // a legacy edit, in place
        let a = h.doc.add(Geometry::Brep(brep));
        let b = add(&mut h, &Solid::cuboid(DVec3::new(5.0, 5.0, 5.0), DVec3::splat(15.0)).unwrap());
        h.start("BooleanUnion");
        h.objects(vec![a, b]);
        assert!(only_brep(&h).exact.is_none(), "the edited operand must not reach the kernel");
    }

    #[test]
    fn a_kernel_refusal_keeps_both_inputs_and_says_why() {
        use std::f64::consts::PI;
        let blob = |cx: f64| {
            let pts: Vec<DVec3> = (0..8)
                .map(|i| {
                    let a = 2.0 * PI * i as f64 / 8.0;
                    let r = 12.0 * (1.0 + 0.2 * (3.0 * a).cos());
                    DVec3::new(cx + r * a.cos(), r * a.sin(), 15.0)
                })
                .collect();
            tri_geom::NurbsCurve::periodic(3, pts).unwrap()
        };
        let plane = Plane { origin: DVec3::new(0.0, 0.0, 15.0), ..Plane::world_xy() };
        let mut h = Harness::new();
        let a = add(&mut h, &Solid::extrude_closed_curve(&blob(0.0), &plane, 15.0).unwrap());
        let b = add(&mut h, &Solid::extrude_closed_curve(&blob(0.001), &plane, 15.0).unwrap());
        h.start("BooleanDifference");
        h.objects(vec![a]);
        h.objects(vec![b]);
        assert!(h.doc.get(a).is_some() && h.doc.get(b).is_some(), "both inputs must stay; log: {:?}", h.log);
        assert_eq!(h.doc.len(), 2);
        assert!(h.log.iter().any(|l| l.contains("failed") && l.contains("solid kernel")), "the log names the kernel: {:?}", h.log);
    }
}

#[cfg(feature = "kernel-solidcore")]
mod exact_primitives {
    use super::*;
    use std::f64::consts::PI;
    use tri_geom::Brep;
    use tri_kernel::Solid;

    fn only_brep(h: &Harness) -> Brep {
        assert_eq!(h.doc.len(), 1, "log: {:?}", h.log);
        match &h.doc.objects().next().unwrap().geometry {
            Geometry::Brep(b) => b.clone(),
            other => panic!("expected an exact polysurface, got {}; log: {:?}", other.type_name(), h.log),
        }
    }

    fn volume(b: &Brep) -> f64 {
        Solid::from_exact(b).expect("a valid exact body").volume().unwrap()
    }

    #[test]
    fn box_command_makes_an_exact_solid() {
        let mut h = Harness::new();
        h.start("Box");
        h.point(0.0, 0.0, 0.0);
        h.point(20.0, 20.0, 0.0);
        h.number(10.0);
        let b = only_brep(&h);
        assert!(b.closed);
        assert!((volume(&b) - 4000.0).abs() < 1e-3);
    }

    #[test]
    fn box_with_negative_extents_is_still_exact_and_positive() {
        let mut h = Harness::new();
        h.start("Box");
        h.point(20.0, 20.0, 0.0);
        h.point(0.0, 0.0, 0.0);
        h.number(10.0);
        let b = only_brep(&h);
        assert!((volume(&b) - 4000.0).abs() < 1e-3);
        let bb = b.bounding_box();
        assert!((bb.min - DVec3::new(0.0, 0.0, 0.0)).length() < 1e-9 && (bb.max - DVec3::new(20.0, 20.0, 10.0)).length() < 1e-9, "{bb:?}");
    }

    #[test]
    fn cylinder_command_makes_an_exact_solid() {
        let mut h = Harness::new();
        h.start("Cylinder");
        h.point(1.0, 2.0, 3.0);
        h.number(4.0);
        h.number(20.0);
        let b = only_brep(&h);
        assert!((volume(&b) - PI * 16.0 * 20.0).abs() < 1e-2);
        assert!((b.bounding_box().center() - DVec3::new(1.0, 2.0, 13.0)).length() < 1e-6);
    }

    #[test]
    fn cone_command_makes_an_exact_solid() {
        let mut h = Harness::new();
        h.start("Cone");
        h.point(0.0, 0.0, 0.0);
        h.number(6.0);
        h.number(15.0);
        let b = only_brep(&h);
        assert!((volume(&b) - PI * 36.0 * 15.0 / 3.0).abs() < 1e-2);
    }

    #[test]
    fn sphere_command_makes_an_exact_solid() {
        let mut h = Harness::new();
        h.start("Sphere");
        h.point(0.0, 0.0, 0.0);
        h.number(20.0);
        let b = only_brep(&h);
        assert!(b.closed);
        assert!((volume(&b) - 4.0 / 3.0 * PI * 8000.0).abs() < 1.0);
    }

    #[test]
    fn torus_command_makes_an_exact_solid() {
        let mut h = Harness::new();
        h.start("Torus");
        h.point(0.0, 0.0, 0.0);
        h.number(14.0);
        h.number(6.0);
        let b = only_brep(&h);
        assert!((volume(&b) - 2.0 * PI * PI * 14.0 * 36.0).abs() < 1.0);
    }

    #[test]
    fn a_torus_whose_tube_reaches_the_axis_falls_back_to_the_legacy_surface() {
        let mut h = Harness::new();
        h.start("Torus");
        h.point(0.0, 0.0, 0.0);
        h.number(5.0);
        h.number(8.0); // minor > major: not a ring torus for the kernel
        assert_eq!(h.doc.len(), 1, "log: {:?}", h.log);
        assert!(matches!(h.doc.objects().next().unwrap().geometry, Geometry::Surface(_)));
    }

    #[test]
    fn non_solid_cylinder_stays_a_surface() {
        let mut h = Harness::new();
        h.start("Cylinder");
        h.option("Solid", None);
        h.point(0.0, 0.0, 0.0);
        h.number(4.0);
        h.number(20.0);
        assert!(matches!(h.doc.objects().next().unwrap().geometry, Geometry::Surface(_)));
    }

    fn exact_box(h: &mut Harness) -> tri_doc::ObjectId {
        h.start("Box");
        h.point(0.0, 0.0, 0.0);
        h.point(20.0, 20.0, 0.0);
        h.number(10.0);
        h.doc.objects().last().unwrap().id
    }

    fn exact_of(h: &Harness, id: tri_doc::ObjectId) -> Brep {
        match &h.doc.get(id).unwrap().geometry {
            Geometry::Brep(b) => b.clone(),
            other => panic!("not a polysurface: {}", other.type_name()),
        }
    }

    fn solid_bounds(b: &Brep) -> ([f64; 3], [f64; 3]) {
        let rec = Solid::from_exact(b).expect("a valid exact body").record().unwrap();
        let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
        for v in &rec.vertices {
            for i in 0..3 {
                lo[i] = lo[i].min(v.point[i]);
                hi[i] = hi[i].max(v.point[i]);
            }
        }
        (lo, hi)
    }

    #[test]
    fn move_keeps_the_exact_body_and_moves_it() {
        let mut h = Harness::new();
        let id = exact_box(&mut h);
        h.doc.clear_selection();
        h.start("Move");
        h.objects(vec![id]);
        h.point(0.0, 0.0, 0.0);
        h.point(100.0, 5.0, 0.0);
        let b = exact_of(&h, id);
        let (lo, hi) = solid_bounds(&b);
        assert!((lo[0] - 100.0).abs() < 1e-9 && (lo[1] - 5.0).abs() < 1e-9 && (hi[0] - 120.0).abs() < 1e-9, "{lo:?} {hi:?}");
        assert!((volume(&b) - 4000.0).abs() < 1e-3);
    }

    #[test]
    fn rotate_keeps_the_exact_body() {
        let mut h = Harness::new();
        let id = exact_box(&mut h);
        h.doc.clear_selection();
        h.start("Rotate");
        h.objects(vec![id]);
        h.point(0.0, 0.0, 0.0);
        h.number(90.0);
        let (lo, hi) = solid_bounds(&exact_of(&h, id));
        assert!((lo[0] + 20.0).abs() < 1e-9 && lo[1].abs() < 1e-9 && hi[0].abs() < 1e-9 && (hi[1] - 20.0).abs() < 1e-9, "{lo:?} {hi:?}");
    }

    #[test]
    fn mirror_copy_keeps_a_positive_exact_solid() {
        let mut h = Harness::new();
        let id = exact_box(&mut h);
        h.doc.clear_selection();
        h.start("Mirror");
        h.objects(vec![id]);
        h.option("YAxis", None);
        assert_eq!(h.doc.len(), 2, "log: {:?}", h.log);
        let copy = h.doc.objects().map(|o| o.id).find(|i| *i != id).unwrap();
        let b = exact_of(&h, copy);
        assert!((volume(&b) - 4000.0).abs() < 1e-3);
        let (lo, hi) = solid_bounds(&b);
        assert!((lo[0] + 20.0).abs() < 1e-9 && hi[0].abs() < 1e-9, "{lo:?} {hi:?}");
    }

    #[test]
    fn boolean_after_a_move_is_still_exact() {
        let mut h = Harness::new();
        let a = exact_box(&mut h);
        h.doc.clear_selection();
        h.start("Cylinder");
        h.point(10.0, 10.0, -5.0);
        h.number(4.0);
        h.number(20.0);
        let c = h.doc.objects().last().unwrap().id;
        h.doc.clear_selection();
        h.start("Move");
        h.objects(vec![c]);
        h.point(0.0, 0.0, 0.0);
        h.point(3.0, 2.0, 0.0);
        h.doc.clear_selection();
        h.start("BooleanDifference");
        h.objects(vec![a]);
        h.objects(vec![c]);
        let r = only_brep(&h);
        assert!(r.exact.is_some(), "the Boolean took the kernel path; log: {:?}", h.log);
        let expect = 4000.0 - PI * 16.0 * 10.0;
        assert!((volume(&r) - expect).abs() < 1e-3, "{} vs {expect}", volume(&r));
    }

    #[test]
    fn a_non_uniform_scale_drops_the_exact_body() {
        let mut h = Harness::new();
        let id = exact_box(&mut h);
        h.doc.transform_objects(&[id], &glam::DAffine3::from_scale(DVec3::new(1.0, 2.0, 1.0)));
        assert!(exact_of(&h, id).exact.is_none());
    }

    #[test]
    fn prism_of_a_closed_curve_is_exact_and_cuts_an_exact_sphere() {
        let c = tri_geom::NurbsCurve::circle(&Plane::world_xy(), 6.0).unwrap();
        let prism = super::misc4::prism_brep(&c, &Plane::world_xy(), 30.0).unwrap();
        assert!((volume(&prism) - PI * 36.0 * 60.0).abs() < 1e-2);
        let sphere = Solid::sphere(DVec3::ZERO, 20.0).unwrap().to_brep(0.05).unwrap();
        let (gs, gp) = (Geometry::Brep(sphere), Geometry::Brep(prism));
        let cut = super::misc4::boolean_geometry_checked(&gs, &gp, tri_geom::BooleanOp::Difference).unwrap();
        let inter = super::misc4::boolean_geometry_checked(&gs, &gp, tri_geom::BooleanOp::Intersection).unwrap();
        let (Geometry::Brep(cut), Geometry::Brep(inter)) = (cut, inter) else { panic!("polysurfaces expected") };
        let total = volume(&cut) + volume(&inter);
        assert!((total - 4.0 / 3.0 * PI * 8000.0).abs() < 1e-3, "V(S-P)+V(S^P) = {total}");
    }
}
