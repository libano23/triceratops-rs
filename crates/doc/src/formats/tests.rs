use std::path::PathBuf;

use glam::DVec3;
use tri_geom::{Brep, NurbsCurve, NurbsSurface, Plane};

use super::*;
use crate::{Document, Geometry};

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("triceratops_fmt_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

fn sample_doc() -> Document {
    let mut d = Document::new();
    let b = Brep::cuboid(&Plane::world_xy(), 10.0, 6.0, 4.0).unwrap();
    d.add(Geometry::Brep(b));
    let s = NurbsSurface::sphere(DVec3::new(20.0, 0.0, 0.0), 3.0).unwrap();
    d.add(Geometry::Surface(s));
    let c = NurbsCurve::from_control_points(3, vec![DVec3::ZERO, DVec3::new(5.0, 8.0, 0.0), DVec3::new(10.0, -3.0, 2.0), DVec3::new(15.0, 4.0, 0.0)]).unwrap();
    d.add(Geometry::Curve(c));
    d.add(Geometry::Curve(NurbsCurve::circle(&Plane::world_xy(), 7.0).unwrap()));
    d.add(Geometry::Point(DVec3::new(1.0, 2.0, 3.0)));
    d
}

fn meshes_in(d: &Document) -> usize {
    d.objects().filter(|o| matches!(o.geometry, Geometry::Mesh(_))).count()
}

fn total_triangles(d: &Document) -> usize {
    d.objects().filter_map(|o| if let Geometry::Mesh(m) = &o.geometry { Some(m.triangle_count()) } else { None }).sum()
}

#[test]
fn mesh_formats_round_trip() {
    let d = sample_doc();
    let src_tris: usize = collect_meshes(&d, false).iter().map(|m| m.mesh.triangle_count()).sum();
    assert!(src_tris > 0);
    for ext in ["stl", "ply", "off", "3ds", "gltf", "glb", "dae", "wrl", "x3d", "x", "amf", "3mf", "gts", "raw", "fbx"] {
        let p = tmp(&format!("rt.{ext}"));
        let n = export(&d, &p, false).unwrap_or_else(|e| panic!("{ext} export: {e}"));
        assert_eq!(n, 2, "{ext}: two meshes exported");
        let mut back = Document::new();
        let m = import(&mut back, &p).unwrap_or_else(|e| panic!("{ext} import: {e}"));
        assert!(m >= 1, "{ext}: imported {m}");
        assert!(meshes_in(&back) >= 1, "{ext}: mesh objects");
        let tris = total_triangles(&back);
        // welding drops the degenerate triangles at the sphere poles
        assert!(tris <= src_tris && tris >= src_tris * 95 / 100, "{ext}: triangle count {tris} vs {src_tris}");
    }
}

#[test]
fn stl_ascii_import_and_binary_detection() {
    let p = tmp("ascii.stl");
    std::fs::write(&p, "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n").unwrap();
    let mut d = Document::new();
    assert_eq!(import(&mut d, &p).unwrap(), 1);
    assert_eq!(total_triangles(&d), 1);
}

#[test]
fn render_and_slice_exports() {
    let d = sample_doc();
    for ext in ["pov", "rib", "slc"] {
        let p = tmp(&format!("r.{ext}"));
        assert_eq!(export(&d, &p, false).unwrap(), 2, "{ext}");
        assert!(std::fs::metadata(&p).unwrap().len() > 100);
    }
    let mut back = Document::new();
    let n = import(&mut back, &tmp("r.slc")).unwrap();
    assert!(n > 10, "slices imported: {n}");
}

#[test]
fn point_files() {
    let d = sample_doc();
    for ext in ["xyz", "pts", "csv", "txt", "asc"] {
        let p = tmp(&format!("p.{ext}"));
        assert!(export(&d, &p, false).unwrap() >= 1, "{ext}");
        let mut back = Document::new();
        let n = import(&mut back, &p).unwrap();
        assert!(n >= 1, "{ext}: {n} points");
        assert!(back.objects().any(|o| matches!(o.geometry, Geometry::Point(p) if (p - DVec3::new(1.0, 2.0, 3.0)).length() < 1e-9)), "{ext}: the point came back");
    }
}

#[test]
fn svg_pdf_eps_export_and_svg_import() {
    let d = sample_doc();
    let p = tmp("v.svg");
    assert!(export(&d, &p, false).unwrap() >= 2);
    let mut back = Document::new();
    let n = import(&mut back, &p).unwrap();
    assert!(n >= 2, "svg curves: {n}");
    for ext in ["pdf", "eps", "ai"] {
        let p = tmp(&format!("v.{ext}"));
        assert!(export(&d, &p, false).unwrap() >= 2, "{ext}");
    }
    let text = std::fs::read_to_string(tmp("v.pdf")).unwrap();
    assert!(text.starts_with("%PDF-1.4") && text.contains("%%EOF"));
    // svg path parsing: relative commands, cubic, close
    let s = tmp("path.svg");
    std::fs::write(&s, "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'><path d='M10 10 l20 0 L30 30 c0 10 -10 10 -10 0 z'/><circle cx='50' cy='50' r='10'/><rect x='1' y='1' width='5' height='5'/></svg>").unwrap();
    let mut back = Document::new();
    assert_eq!(import(&mut back, &s).unwrap(), 3);
    let circle = back.objects().filter_map(|o| if let Geometry::Curve(c) = &o.geometry { Some(c) } else { None }).find(|c| c.is_rational()).expect("circle as rational curve");
    assert!((circle.point_at(circle.domain().0) - DVec3::new(60.0, 50.0, 0.0)).length() < 1e-6, "circle start on the flipped y axis");
}

#[test]
fn iges_round_trip_keeps_nurbs() {
    let d = sample_doc();
    let p = tmp("t.igs");
    assert_eq!(export(&d, &p, false).unwrap(), 5);
    let mut back = Document::new();
    let n = import(&mut back, &p).unwrap();
    assert!(n >= 5, "iges objects: {n}");
    assert!(back.objects().any(|o| matches!(o.geometry, Geometry::Point(_))));
    let curves: Vec<&NurbsCurve> = back.objects().filter_map(|o| if let Geometry::Curve(c) = &o.geometry { Some(c) } else { None }).collect();
    assert!(curves.iter().any(|c| c.degree == 3), "degree-3 curve kept");
    assert!(curves.iter().any(|c| c.is_rational() && c.is_closed()), "circle kept rational");
    let brep = back.objects().find_map(|o| if let Geometry::Brep(b) = &o.geometry { Some(b) } else { None }).expect("polysurface from 402 group");
    assert_eq!(brep.faces.len(), 6);
    assert!(back.objects().any(|o| matches!(&o.geometry, Geometry::Surface(s) if s.degree_v == 2 && s.weights.iter().any(|w| (*w - 1.0).abs() > 1e-9))), "sphere stays a rational surface");
    // every control point of the cubic survived
    let orig = d.objects().find_map(|o| if let Geometry::Curve(c) = &o.geometry { if c.degree == 3 { Some(c.clone()) } else { None } } else { None }).unwrap();
    let got = curves.iter().find(|c| c.degree == 3).unwrap();
    for (a, b) in orig.control_points.iter().zip(&got.control_points) {
        assert!((*a - *b).length() < 1e-9);
    }
}

#[test]
fn iges_trimmed_face_round_trip() {
    let mut d = Document::new();
    let a = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
    let mut b = Brep::cuboid(&Plane::world_xy(), 4.0, 4.0, 20.0).unwrap();
    b.transform(&glam::DAffine3::from_translation(DVec3::new(3.0, 3.0, -5.0)));
    let r = tri_geom::brep_boolean(&a, &b, tri_geom::BooleanOp::Difference).unwrap();
    assert!(r.faces.iter().any(|f| f.loops.len() == 2));
    d.add(Geometry::Brep(r));
    let p = tmp("trim.igs");
    assert_eq!(export(&d, &p, false).unwrap(), 1);
    let mut back = Document::new();
    assert_eq!(import(&mut back, &p).unwrap(), 1);
    let brep = back.objects().find_map(|o| if let Geometry::Brep(b) = &o.geometry { Some(b) } else { None }).unwrap();
    assert_eq!(brep.faces.len(), 10);
    assert!(brep.faces.iter().any(|f| f.loops.len() == 2), "hole loop survived");
}

#[test]
fn step_round_trip() {
    let d = sample_doc();
    let p = tmp("t.stp");
    assert_eq!(export(&d, &p, false).unwrap(), 5);
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.starts_with("ISO-10303-21;"));
    assert!(text.contains("MANIFOLD_SOLID_BREP"));
    let mut back = Document::new();
    let n = import(&mut back, &p).unwrap();
    assert!(n >= 4, "step objects: {n}");
    let brep = back.objects().find_map(|o| if let Geometry::Brep(b) = &o.geometry { Some(b) } else { None }).expect("solid");
    assert_eq!(brep.faces.len(), 6);
    assert!(back.objects().any(|o| matches!(&o.geometry, Geometry::Curve(c) if c.degree == 3)), "cubic curve");
    assert!(back.objects().any(|o| matches!(&o.geometry, Geometry::Curve(c) if c.is_rational())), "circle");
}

#[test]
fn step_analytic_surfaces_from_other_cad() {
    // a cylinder-like file written the way other CAD systems do: CYLINDRICAL_SURFACE + PLANEs
    let text = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION((''),'2;1');
FILE_NAME('','',(''),(''),'','','');
FILE_SCHEMA(('AUTOMOTIVE_DESIGN'));
ENDSEC;
DATA;
#1 = CARTESIAN_POINT('',(0.,0.,0.));
#2 = DIRECTION('',(0.,0.,1.));
#3 = DIRECTION('',(1.,0.,0.));
#4 = AXIS2_PLACEMENT_3D('',#1,#2,#3);
#5 = PLANE('',#4);
#6 = CIRCLE('',#4,5.);
#7 = CARTESIAN_POINT('',(5.,0.,0.));
#8 = VERTEX_POINT('',#7);
#9 = EDGE_CURVE('',#8,#8,#6,.T.);
#10 = ORIENTED_EDGE('',*,*,#9,.T.);
#11 = EDGE_LOOP('',(#10));
#12 = FACE_OUTER_BOUND('',#11,.T.);
#13 = ADVANCED_FACE('',(#12),#5,.F.);
#14 = CARTESIAN_POINT('',(0.,0.,10.));
#15 = AXIS2_PLACEMENT_3D('',#14,#2,#3);
#16 = PLANE('',#15);
#17 = CIRCLE('',#15,5.);
#18 = CARTESIAN_POINT('',(5.,0.,10.));
#19 = VERTEX_POINT('',#18);
#20 = EDGE_CURVE('',#19,#19,#17,.T.);
#21 = ORIENTED_EDGE('',*,*,#20,.T.);
#22 = EDGE_LOOP('',(#21));
#23 = FACE_OUTER_BOUND('',#22,.T.);
#24 = ADVANCED_FACE('',(#23),#16,.T.);
#25 = CYLINDRICAL_SURFACE('',#4,5.);
#26 = VECTOR('',#2,1.);
#27 = LINE('',#7,#26);
#28 = EDGE_CURVE('',#8,#19,#27,.T.);
#29 = ORIENTED_EDGE('',*,*,#9,.T.);
#30 = ORIENTED_EDGE('',*,*,#28,.T.);
#31 = ORIENTED_EDGE('',*,*,#20,.F.);
#32 = ORIENTED_EDGE('',*,*,#28,.F.);
#33 = EDGE_LOOP('',(#29,#30,#31,#32));
#34 = FACE_OUTER_BOUND('',#33,.T.);
#35 = ADVANCED_FACE('',(#34),#25,.T.);
#36 = CLOSED_SHELL('',(#13,#24,#35));
#37 = MANIFOLD_SOLID_BREP('cyl',#36);
ENDSEC;
END-ISO-10303-21;
"#;
    let p = tmp("cyl.step");
    std::fs::write(&p, text).unwrap();
    let mut d = Document::new();
    assert_eq!(import(&mut d, &p).unwrap(), 1);
    let o = d.objects().next().unwrap();
    assert_eq!(o.attributes.name.as_deref(), Some("cyl"));
    let Geometry::Brep(b) = &o.geometry else { panic!("brep") };
    assert_eq!(b.faces.len(), 3);
    // the caps are trimmed planes with a circular loop, the wall is the full cylinder
    let caps = b.faces.iter().filter(|f| f.surface.degree_u == 1 && f.surface.degree_v == 1).count();
    assert_eq!(caps, 2);
    let wall = b.faces.iter().find(|f| f.surface.degree_u == 2 || f.surface.degree_v == 2).expect("cylinder wall");
    assert!(!wall.is_trimmed(), "wall is untrimmed (loop on the surface border)");
    let bb = wall.surface.bounding_box();
    assert!((bb.max.z - bb.min.z - 10.0).abs() < 1.5, "wall height ~10 with margin: {}", bb.max.z - bb.min.z);
    let mesh = b.tessellate(16, 16);
    assert!(mesh.triangle_count() > 50);
}

#[test]
fn vda_round_trip() {
    let d = sample_doc();
    let p = tmp("t.vda");
    assert_eq!(export(&d, &p, false).unwrap(), 5);
    let mut back = Document::new();
    let n = import(&mut back, &p).unwrap();
    assert!(n >= 4, "vda objects: {n}");
    assert!(back.objects().any(|o| matches!(o.geometry, Geometry::Point(_))));
    assert!(back.objects().any(|o| matches!(&o.geometry, Geometry::Brep(b) if b.faces.len() == 6)), "box as 6 patches");
    // the cubic comes back as a sampled curve passing through the original end points
    let orig = d.objects().find_map(|o| if let Geometry::Curve(c) = &o.geometry { if c.degree == 3 { Some(c.clone()) } else { None } } else { None }).unwrap();
    let got = back.objects().filter_map(|o| if let Geometry::Curve(c) = &o.geometry { Some(c) } else { None }).find(|c| (c.point_at(c.domain().0) - orig.point_at(orig.domain().0)).length() < 1e-6).expect("curve start");
    assert!((got.point_at(got.domain().1) - orig.point_at(orig.domain().1)).length() < 1e-6);
    // sphere patches are exact at their corners
    let sph = back.objects().filter_map(|o| if let Geometry::Brep(b) = &o.geometry { Some(b) } else { None }).find(|b| b.faces.len() != 6).expect("sphere patches");
    for f in &sph.faces {
        for p in &f.surface.control_points {
            let _ = p;
        }
        let c = f.surface.point_at(f.surface.domain_u().0, f.surface.domain_v().0);
        assert!(((c - DVec3::new(20.0, 0.0, 0.0)).length() - 3.0).abs() < 1e-6, "patch corner on the sphere");
    }
}

#[test]
fn registry_and_unsupported() {
    assert!(format_for("STL").unwrap().import);
    assert!(format_for("pdf").is_some_and(|f| !f.import && f.export));
    assert!(unsupported_reason("skp").is_some());
    let mut d = Document::new();
    let err = import(&mut d, &tmp("x.skp")).unwrap_err().to_string();
    assert!(err.contains("SketchUp"));
    assert!(import_extensions().len() > 20);
    assert!(export_extensions().contains(&"igs"));
}

#[test]
fn svg_smooth_curves_reflect_matching_control_point() {
    use super::vector2d::parse_path;
    let mid = |d: &str| {
        let p = parse_path(d);
        let seg = &p[0];
        seg[seg.len() - 16 + 7] // t = 0.5 of the last segment
    };
    // T reflects the previous quadratic control (10,10) about (20,0) -> (30,-10)
    let m = mid("M0 0 Q10 10 20 0 T40 0");
    assert!((m.x - 30.0).abs() < 1e-9 && (m.y + 5.0).abs() < 1e-9, "T midpoint {m:?}");
    // S after Q does not reflect: first control = current point
    let m = mid("M0 0 Q10 10 20 0 S30 10 40 0");
    assert!((m.x - 26.25).abs() < 1e-9 && (m.y - 3.75).abs() < 1e-9, "S midpoint {m:?}");
    // S after C reflects the cubic second control (15,10) about (20,0) -> (25,-10)
    let m = mid("M0 0 C5 10 15 10 20 0 S35 10 40 0");
    let e = DVec3::new(20.0, 0.0, 0.0) * 0.125 + DVec3::new(25.0, -10.0, 0.0) * 0.375 + DVec3::new(35.0, 10.0, 0.0) * 0.375 + DVec3::new(40.0, 0.0, 0.0) * 0.125;
    assert!((m - e).length() < 1e-9, "S after C midpoint {m:?}");
}


#[cfg(feature = "kernel-solidcore")]
mod exact_step {
    use super::*;
    use tri_kernel::{BooleanOp, Solid};

    fn sphere_minus_cylinder() -> Brep {
        let s = Solid::sphere(DVec3::ZERO, 20.0).unwrap();
        let c = Solid::cylinder(DVec3::new(5.0, -30.0, 3.0), DVec3::Y, 8.0, 60.0).unwrap();
        s.boolean(&c, BooleanOp::Difference).unwrap().to_brep(1e-3).unwrap()
    }

    fn count(text: &str, what: &str) -> usize {
        text.matches(what).count()
    }

    #[test]
    fn an_exact_solid_is_exported_with_its_analytic_surfaces_and_few_edges() {
        let mut d = Document::new();
        d.add(Geometry::Brep(sphere_minus_cylinder()));
        let p = tmp("exact.step");
        assert_eq!(export(&d, &p, false).unwrap(), 1);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("SPHERICAL_SURFACE"), "the sphere is a sphere, not a B-spline patch");
        assert!(text.contains("CYLINDRICAL_SURFACE"), "the bore is a cylinder");
        assert_eq!(count(&text, "B_SPLINE_SURFACE"), 0, "no freeform surface for an analytic solid");
        assert!(count(&text, "EDGE_CURVE(") <= 12, "edges are real edges shared by two faces, not one polyline per face: {}", count(&text, "EDGE_CURVE("));
        // the sphere-cylinder intersection is a freeform curve: its poles are the only points
        assert!(count(&text, "CARTESIAN_POINT") < 1500, "no tessellation points: {}", count(&text, "CARTESIAN_POINT"));
    }

    /// Written by the kernel and read back by a different CAD kernel (solidcraft's, 2026-11-27):
    /// 2 faces and 26183.2139 mm3, the volume of this solid. Here: the file is a valid exchange
    /// structure with one solid and the units of the document.
    #[test]
    fn the_exact_step_is_a_valid_file_with_one_solid_in_millimetres() {
        let mut d = Document::new();
        d.add(Geometry::Brep(sphere_minus_cylinder()));
        let p = tmp("exact_valid.step");
        export(&d, &p, false).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("ISO-10303-21;") && text.trim_end().ends_with("END-ISO-10303-21;"));
        assert_eq!(count(&text, "MANIFOLD_SOLID_BREP"), 1);
        assert_eq!(count(&text, "CLOSED_SHELL"), 1);
        assert!(text.contains(".MILLI.") && text.contains(".METRE."), "millimetres");
        assert!(text.contains("Triceratops"), "the program is named in the header");
    }

    #[test]
    fn a_moved_exact_solid_is_exported_where_it_is() {
        let mut b = sphere_minus_cylinder();
        b.transform(&glam::DAffine3::from_translation(DVec3::new(1000.0, 0.0, 0.0)));
        let mut d = Document::new();
        d.add(Geometry::Brep(b));
        let p = tmp("exact_moved.step");
        export(&d, &p, false).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        // the sphere's centre is a CARTESIAN_POINT at x = 1000
        assert!(text.contains("(1000.0,0.0,0.0)"), "the placement is applied to what is written");
    }

    #[test]
    fn a_stale_exact_body_exports_the_faces_as_before() {
        let mut b = sphere_minus_cylinder();
        b.faces[0].surface.control_points[0].x += 0.01;
        let mut d = Document::new();
        d.add(Geometry::Brep(b));
        let p = tmp("stale.step");
        export(&d, &p, false).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().contains("B_SPLINE_SURFACE"), "the edited faces are written as they are");
    }

    #[test]
    fn a_mixed_model_keeps_working() {
        let mut d = Document::new();
        d.add(Geometry::Brep(sphere_minus_cylinder()));
        d.add(Geometry::Curve(NurbsCurve::circle(&Plane::world_xy(), 7.0).unwrap()));
        let p = tmp("mixed.step");
        assert_eq!(export(&d, &p, false).unwrap(), 2);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("SPHERICAL_SURFACE") && text.contains("CIRCLE") || text.contains("B_SPLINE_CURVE"), "solid and curve are both in the file");
    }
}


// ---- mesh export quality (STL, OBJ, 3MF, ... all use collect_meshes) ----

fn max_sag_on_sphere(mesh: &tri_geom::Mesh, center: DVec3, r: f64) -> f64 {
    let mut worst: f64 = 0.0;
    for t in 0..mesh.triangle_count() {
        let (a, b, c) = mesh.triangle(t);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            worst = worst.max(((p + q) * 0.5 - center).length() - r).max(r - ((p + q) * 0.5 - center).length());
        }
    }
    worst
}

#[test]
fn a_curved_solid_is_exported_within_a_chord_tolerance_of_its_size() {
    let mut d = Document::new();
    d.add(Geometry::Surface(NurbsSurface::sphere(DVec3::ZERO, 20.0).unwrap()));
    let meshes = collect_meshes(&d, false);
    let sag = max_sag_on_sphere(&meshes[0].mesh, DVec3::ZERO, 20.0);
    // 5e-4 of the diameter at the worst; the fixed 16 x 16 grid was off by about 0.1 mm here
    assert!(sag <= 40.0 * 5e-4, "a triangle edge strays {sag} mm from the sphere");
}

#[test]
fn a_flat_solid_is_not_filled_with_grid_triangles() {
    let mut d = Document::new();
    d.add(Geometry::Brep(Brep::cuboid(&Plane::world_xy(), 10.0, 6.0, 4.0).unwrap()));
    let meshes = collect_meshes(&d, false);
    assert!(meshes[0].mesh.triangle_count() <= 24, "a box needs 12 triangles, not {}", meshes[0].mesh.triangle_count());
}

#[test]
fn obj_and_stl_use_the_same_fineness() {
    let mut d = Document::new();
    d.add(Geometry::Surface(NurbsSurface::sphere(DVec3::ZERO, 20.0).unwrap()));
    let want = collect_meshes(&d, false)[0].mesh.triangle_count();
    let p = tmp("fine.obj");
    crate::io::export_obj(&d, &p, false).unwrap();
    let faces = std::fs::read_to_string(&p).unwrap().lines().filter(|l| l.starts_with("f ")).count();
    assert_eq!(faces, want, "OBJ used its own coarser tessellation");
}

#[cfg(feature = "kernel-solidcore")]
#[test]
fn an_exact_solid_is_exported_as_a_watertight_mesh() {
    use tri_kernel::{BooleanOp, Solid};
    let s = Solid::sphere(DVec3::ZERO, 20.0).unwrap();
    let c = Solid::cylinder(DVec3::new(5.0, -30.0, 3.0), DVec3::Y, 8.0, 60.0).unwrap();
    let b = s.boolean(&c, BooleanOp::Difference).unwrap().to_brep(1e-3).unwrap();
    let mut d = Document::new();
    d.add(Geometry::Brep(b));
    let m = &collect_meshes(&d, false)[0].mesh;
    // every undirected edge, by the positions of its ends, is used by exactly two triangles
    let key = |p: DVec3| ((p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64, (p.z * 1e6).round() as i64);
    let mut edges: std::collections::HashMap<_, usize> = std::collections::HashMap::new();
    for t in 0..m.triangle_count() {
        let (a, b, c) = m.triangle(t);
        for (p, q) in [(a, b), (b, c), (c, a)] {
            let (k1, k2) = (key(p), key(q));
            *edges.entry(if k1 < k2 { (k1, k2) } else { (k2, k1) }).or_default() += 1;
        }
    }
    let bad = edges.values().filter(|n| **n != 2).count();
    assert_eq!(bad, 0, "{bad} of {} edges are open or non-manifold", edges.len());
}


#[test]
fn vda_keeps_the_trim_boundaries_of_a_trimmed_face_as_curves() {
    let mut d = Document::new();
    // a cap trimmed by a circle: a surface plus one boundary loop
    let circle = NurbsCurve::circle(&Plane::world_xy(), 4.0).unwrap();
    let cap = tri_geom::Face::from_planar_curves(std::slice::from_ref(&circle), 1e-3).unwrap();
    assert!(cap.is_trimmed());
    d.add(Geometry::Brep(Brep::new(vec![cap])));
    let p = tmp("trimmed.vda");
    export_vda_for_test(&d, &p);
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("SURF"), "the face's surface");
    assert!(text.matches("CURVE").count() >= 1, "the trim loop must be written, or the face is a whole patch again:\n{text}");
}

fn export_vda_for_test(d: &Document, p: &std::path::Path) {
    vda::export(d, p, false).unwrap();
}

#[test]
fn flat_faces_of_a_legacy_solid_are_written_as_planes_in_step() {
    let mut d = Document::new();
    d.add(Geometry::Brep(Brep::cuboid(&Plane::world_xy(), 10.0, 6.0, 4.0).unwrap()));
    let p = tmp("planes.step");
    step::export(&d, &p, false).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert_eq!(text.matches("PLANE('',").count(), 6, "six PLANE faces");
    assert_eq!(text.matches("B_SPLINE_SURFACE").count(), 0, "no freeform patch for a flat face");
    // and Triceratops reads its own file back as the same box
    let mut back = Document::new();
    let n = import(&mut back, &p).unwrap();
    assert_eq!(n, 1);
    let g = back.objects().next().expect("the box").geometry.clone();
    let v = Document::volume_of(&g).unwrap().0;
    assert!((v - 240.0).abs() < 1.0, "volume {v}");
}




#[test]
fn dxf_carries_the_faces_of_a_solid_and_not_only_its_edges() {
    let mut d = Document::new();
    d.add(Geometry::Brep(Brep::cuboid(&Plane::world_xy(), 10.0, 6.0, 4.0).unwrap()));
    let p = tmp("solid.dxf");
    crate::dxf::export(&d, &p, false).unwrap();
    let text = String::from_utf8_lossy(&std::fs::read(&p).unwrap()).to_string();
    let faces = text.matches("3DFACE").count();
    assert_eq!(faces, 12, "a box is 12 triangles, written as 3D faces");
    assert!(text.contains("LINE") || text.contains("POLYLINE") || text.contains("SPLINE"), "the edges are still there");
}

// ---- units: STEP and VDA are in millimetres, IGES declares its unit ----

fn inch_cube_doc() -> Document {
    let mut d = Document::new();
    d.settings.units = "Inches".into();
    d.add(Geometry::Brep(Brep::cuboid(&Plane::world_xy(), 1.0, 1.0, 1.0).unwrap()));
    d
}

fn max_coordinate(text: &str) -> f64 {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-' || c == 'E')).filter_map(|t| t.parse::<f64>().ok()).fold(0.0, f64::max)
}

#[test]
fn step_is_written_in_millimetres_whatever_the_document_unit() {
    let p = tmp("inch.step");
    step::export(&inch_cube_doc(), &p, false).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    // the coordinates inside CARTESIAN_POINT('',(x,y,z)), not the entity numbers
    let cps: String = text.lines().filter_map(|l| l.split("CARTESIAN_POINT('',(").nth(1)).map(|r| r.split("))").next().unwrap_or("").replace(',', " ")).collect::<Vec<_>>().join(" ");
    assert!((max_coordinate(&cps) - 25.4).abs() < 1e-9, "a 1 inch cube is 25.4 mm in the file: {}", max_coordinate(&cps));
}

#[test]
fn iges_declares_the_unit_of_the_document() {
    let p = tmp("inch.igs");
    iges::export(&inch_cube_doc(), &p, false).unwrap();
    let g = std::fs::read_to_string(&p).unwrap().lines().filter(|l| l.contains("G ")).collect::<Vec<_>>().join("");
    assert!(g.contains(",1,2HIN,"), "inches: unit flag 1, name IN: {g}");
    let mut mm = inch_cube_doc();
    mm.settings.units = "Millimeters".into();
    iges::export(&mm, &p, false).unwrap();
    assert!(std::fs::read_to_string(&p).unwrap().contains(",2,2HMM,"));
}

#[test]
fn vda_is_scaled_to_millimetres() {
    let p = tmp("inch.vda");
    vda::export(&inch_cube_doc(), &p, false).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(max_coordinate(&text) > 20.0, "a 1 inch cube spans 25.4 mm in the file: {}", max_coordinate(&text));
}

#[cfg(feature = "kernel-solidcore")]
#[test]
fn an_exact_solid_is_exported_in_millimetres_too() {
    let mut d = Document::new();
    d.settings.units = "Inches".into();
    d.add(Geometry::Brep(tri_kernel::Solid::sphere(DVec3::new(1.0, 0.0, 0.0), 1.0).unwrap().to_brep(1e-3).unwrap()));
    let p = tmp("inch_exact.step");
    step::export(&d, &p, false).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("SPHERICAL_SURFACE('',#") && text.contains(",25.4)"), "the sphere's radius is 25.4 mm");
    assert!(text.contains("(25.4,0.0,0.0)"), "its centre is at 25.4 mm");
}

/// The text of a file that may be a zip of XML parts (3MF, AMF): all parts joined.
fn text_of(p: &std::path::Path) -> String {
    let bytes = std::fs::read(p).unwrap();
    match zip_read(&bytes) {
        Ok(entries) if !entries.is_empty() => entries.iter().map(|(_, b)| String::from_utf8_lossy(b).to_string()).collect::<Vec<_>>().join("\n"),
        _ => String::from_utf8_lossy(&bytes).to_string(),
    }
}

#[test]
fn mesh_formats_declare_the_documents_unit() {
    let d = inch_cube_doc();
    let p = tmp("inch.3mf");
    mesh_formats::export_3mf(&d, &p, false).unwrap();
    assert!(text_of(&p).contains("unit=\"inch\""), "3MF in inches");
    let p = tmp("inch.amf");
    mesh_formats::export_amf(&d, &p, false).unwrap();
    assert!(text_of(&p).contains("unit=\"inch\""), "AMF in inches");
    let p = tmp("inch.dae");
    mesh_formats::export_dae(&d, &p, false).unwrap();
    assert!(text_of(&p).contains("<unit name=\"inch\" meter=\"0.0254\"/>"), "COLLADA in inches");
}

#[test]
fn gltf_is_written_in_metres() {
    let p = tmp("inch.gltf");
    mesh_formats::export_gltf(&inch_cube_doc(), &p, false).unwrap();
    let text = std::fs::read_to_string(&p).unwrap();
    assert!(text.contains("0.0254"), "a 1 inch cube spans 0.0254 m in a glTF file: {}", &text[..text.len().min(300)]);
}

#[test]
fn gltf_round_trips_in_the_units_of_the_document_and_reads_metres_from_other_programs() {
    // Triceratops writes metres and reads them back into its own unit
    let mut d = Document::new();
    d.add(Geometry::Mesh(tri_geom::Mesh::cuboid(DVec3::ZERO, DVec3::new(100.0, 60.0, 40.0))));
    let p = tmp("rt.gltf");
    mesh_formats::export_gltf(&d, &p, false).unwrap();
    let mut back = Document::new();
    mesh_formats::import_gltf(&mut back, &p).unwrap();
    let bb = back.objects().next().unwrap().geometry.bounding_box();
    assert!((bb.max - DVec3::new(100.0, 60.0, 40.0)).length() < 1e-3, "back in millimetres: {:?}", bb.max);
    // the same file opened in a document in metres is a 0.1 m part
    let mut metres = Document::new();
    metres.settings.units = "Meters".into();
    mesh_formats::import_gltf(&mut metres, &p).unwrap();
    let bb = metres.objects().next().unwrap().geometry.bounding_box();
    assert!((bb.max - DVec3::new(0.1, 0.06, 0.04)).length() < 1e-6, "{:?}", bb.max);
}

#[test]
fn mesh_importers_read_the_unit_the_file_declares() {
    // a 1 inch cube written by a document in inches comes back as 25.4 in a millimetre document
    for (name, export, import) in [
        ("u.3mf", mesh_formats::export_3mf as fn(&Document, &std::path::Path, bool) -> anyhow::Result<usize>, mesh_formats::import_3mf as fn(&mut Document, &std::path::Path) -> anyhow::Result<usize>),
        ("u.amf", mesh_formats::export_amf, mesh_formats::import_amf),
        ("u.dae", mesh_formats::export_dae, mesh_formats::import_dae),
    ] {
        let mut d = Document::new();
        d.settings.units = "Inches".into();
        d.add(Geometry::Mesh(tri_geom::Mesh::cuboid(DVec3::ZERO, DVec3::ONE)));
        let p = tmp(name);
        export(&d, &p, false).unwrap();
        let mut mm = Document::new();
        import(&mut mm, &p).unwrap();
        let bb = mm.objects().next().unwrap().geometry.bounding_box();
        assert!((bb.max.x - 25.4).abs() < 1e-6, "{name}: {:?}", bb.max);
        let mut back = Document::new();
        back.settings.units = "Inches".into();
        import(&mut back, &p).unwrap();
        assert!((back.objects().next().unwrap().geometry.bounding_box().max.x - 1.0).abs() < 1e-6, "{name} back in inches");
    }
}
