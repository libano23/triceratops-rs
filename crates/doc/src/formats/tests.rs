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
    assert!(format_for("pdf").map_or(false, |f| !f.import && f.export));
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
