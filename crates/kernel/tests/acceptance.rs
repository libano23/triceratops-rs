#![cfg(feature = "solidcore")]
//! Acceptance cases for the exact-solid kernel (docs/superpowers/specs/2026-11-27-solidcore-kernel-design.md).

use std::f64::consts::PI;

use glam::{DAffine3, DQuat, DVec3};
use tri_kernel::Solid;

/// Relative error in percent.
fn pct(got: f64, want: f64) -> f64 {
    100.0 * (got - want) / want
}

fn assert_vol(s: &Solid, want: f64, tol_pct: f64) {
    let v = s.volume().unwrap();
    assert!(pct(v, want).abs() <= tol_pct, "volume {v} vs {want} ({:+.6} %)", pct(v, want));
}

#[test]
fn cuboid_volume() {
    assert_vol(&Solid::cuboid(DVec3::new(1.0, 2.0, 3.0), DVec3::new(3.0, 5.0, 7.0)).unwrap(), 2.0 * 3.0 * 4.0, 1e-3);
}

#[test]
fn cylinder_volume() {
    assert_vol(&Solid::cylinder(DVec3::ZERO, DVec3::Z, 3.0, 7.0).unwrap(), PI * 9.0 * 7.0, 1e-3);
}

#[test]
fn sphere_volume() {
    assert_vol(&Solid::sphere(DVec3::new(1.0, 2.0, 3.0), 5.0).unwrap(), 4.0 / 3.0 * PI * 125.0, 1e-3);
}

#[test]
fn torus_volume() {
    assert_vol(&Solid::torus(DVec3::ZERO, DVec3::Z, 14.0, 6.0).unwrap(), 2.0 * PI * PI * 14.0 * 36.0, 1e-3);
}

#[test]
fn cone_volume() {
    let (r, t, h) = (4.0, 1.5, 6.0);
    assert_vol(&Solid::cone(DVec3::ZERO, DVec3::Z, r, t, h).unwrap(), PI * h / 3.0 * (r * r + r * t + t * t), 1e-3);
}

#[test]
fn arbitrary_axis_keeps_volume() {
    let axis = DVec3::new(1.0, 0.25, 0.15);
    assert_vol(&Solid::cylinder(DVec3::new(-3.0, 2.0, 1.0), axis, 3.0, 7.0).unwrap(), PI * 9.0 * 7.0, 1e-3);
    assert_vol(&Solid::torus(DVec3::new(1.0, 1.0, 1.0), axis, 14.0, 6.0).unwrap(), 2.0 * PI * PI * 14.0 * 36.0, 1e-3);
}

#[test]
fn transform_moves_the_box_and_keeps_volume() {
    let b = Solid::cuboid(DVec3::ZERO, DVec3::new(2.0, 3.0, 4.0)).unwrap();
    // rotate 90 degrees about Z, then move by (10, 0, 0): x range 0..2 -> y range 0..2; y range 0..3 -> x range -3..0, +10
    let xf = DAffine3::from_rotation_translation(DQuat::from_rotation_z(PI / 2.0), DVec3::new(10.0, 0.0, 0.0));
    let m = b.transformed(&xf).unwrap();
    assert_vol(&m, 24.0, 1e-3);
    let rec = m.record().unwrap();
    let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
    for v in &rec.vertices {
        for i in 0..3 {
            lo[i] = lo[i].min(v.point[i]);
            hi[i] = hi[i].max(v.point[i]);
        }
    }
    let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9);
    assert!(close(lo, [7.0, 0.0, 0.0]), "lo {lo:?}");
    assert!(close(hi, [10.0, 2.0, 4.0]), "hi {hi:?}");
}

// ---- Booleans (analytic cases from the evaluation of 2026-11-27) ----

use tri_kernel::BooleanOp;

fn cyl_x(r: f64) -> Solid {
    Solid::cylinder(DVec3::new(-20.0, 0.0, 0.0), DVec3::X, r, 40.0).unwrap()
}
fn cyl_y(r: f64) -> Solid {
    Solid::cylinder(DVec3::new(0.0, -20.0, 0.0), DVec3::Y, r, 40.0).unwrap()
}
fn cyl_z(r: f64) -> Solid {
    Solid::cylinder(DVec3::new(0.0, 0.0, -20.0), DVec3::Z, r, 40.0).unwrap()
}
fn plate() -> Solid {
    Solid::cuboid(DVec3::new(-20.0, -20.0, 0.0), DVec3::new(20.0, 20.0, 20.0)).unwrap()
}

#[test]
fn steinmetz_two_cylinders() {
    let r = 10.0;
    let s = cyl_x(r).boolean(&cyl_y(r), BooleanOp::Intersection).unwrap();
    assert_vol(&s, 16.0 / 3.0 * r.powi(3), 1e-3);
}

#[test]
fn tricylinder() {
    let r = 10.0;
    let ab = cyl_x(r).boolean(&cyl_y(r), BooleanOp::Intersection).unwrap();
    let s = ab.boolean(&cyl_z(r), BooleanOp::Intersection).unwrap();
    assert_vol(&s, 8.0 * (2.0 - 2f64.sqrt()) * r.powi(3), 1e-3);
}

#[test]
fn crossing_cylinders_union() {
    let r = 10.0;
    let s = cyl_x(r).boolean(&cyl_y(r), BooleanOp::Union).unwrap();
    assert_vol(&s, 2.0 * PI * r * r * 40.0 - 16.0 / 3.0 * r.powi(3), 1e-3);
}

#[test]
fn flush_hole_with_coincident_faces() {
    let hole = Solid::cylinder(DVec3::ZERO, DVec3::Z, 8.0, 20.0).unwrap();
    let s = plate().boolean(&hole, BooleanOp::Difference).unwrap();
    assert_vol(&s, 40.0 * 40.0 * 20.0 - PI * 64.0 * 20.0, 1e-3);
}

#[test]
fn cylinder_tangent_to_the_side_wall() {
    let tool = Solid::cylinder(DVec3::new(10.0, 0.0, -5.0), DVec3::Z, 10.0, 30.0).unwrap();
    let s = plate().boolean(&tool, BooleanOp::Difference).unwrap();
    assert_vol(&s, 40.0 * 40.0 * 20.0 - PI * 100.0 * 20.0, 1e-3);
}

#[test]
fn sphere_tangent_to_all_six_faces() {
    let cube = Solid::cuboid(DVec3::splat(-20.0), DVec3::splat(20.0)).unwrap();
    let ball = Solid::sphere(DVec3::ZERO, 20.0).unwrap();
    let s = cube.boolean(&ball, BooleanOp::Difference).unwrap();
    assert_vol(&s, 64000.0 - 4.0 / 3.0 * PI * 8000.0, 1e-3);
}

#[test]
fn two_overlapping_spheres_union() {
    let (r, d) = (10.0, 10.0);
    let a = Solid::sphere(DVec3::ZERO, r).unwrap();
    let b = Solid::sphere(DVec3::new(d, 0.0, 0.0), r).unwrap();
    let lens = PI * (4.0 * r + d) * (2.0 * r - d).powi(2) / 12.0;
    assert_vol(&a.boolean(&b, BooleanOp::Union).unwrap(), 2.0 * 4.0 / 3.0 * PI * r.powi(3) - lens, 1e-3);
}

#[test]
fn sphere_internally_tangent_to_a_sphere_difference() {
    let a = Solid::sphere(DVec3::ZERO, 20.0).unwrap();
    let b = Solid::sphere(DVec3::new(10.0, 0.0, 0.0), 10.0).unwrap();
    assert_vol(&a.boolean(&b, BooleanOp::Difference).unwrap(), 4.0 / 3.0 * PI * (8000.0 - 1000.0), 1e-3);
}

#[test]
fn boolean_does_not_consume_its_operands() {
    let a = plate();
    let b = Solid::cylinder(DVec3::ZERO, DVec3::Z, 8.0, 20.0).unwrap();
    let _ = a.boolean(&b, BooleanOp::Difference).unwrap();
    assert_vol(&a, 32000.0, 1e-3);
    assert_vol(&b, PI * 64.0 * 20.0, 1e-3);
}

#[test]
fn disjoint_intersection_is_refused_as_empty() {
    let a = Solid::cuboid(DVec3::ZERO, DVec3::splat(1.0)).unwrap();
    let b = Solid::cuboid(DVec3::splat(5.0), DVec3::splat(6.0)).unwrap();
    let e = a.boolean(&b, BooleanOp::Intersection).err().expect("empty result is an error");
    assert_eq!(e.to_string(), "the result is empty");
}

#[test]
fn tiny_part_scale() {
    let a = Solid::cuboid(DVec3::ZERO, DVec3::splat(0.05)).unwrap();
    let hole = Solid::cylinder(DVec3::new(0.025, 0.025, 0.0), DVec3::Z, 0.01, 0.05).unwrap();
    let s = a.boolean(&hole, BooleanOp::Difference).unwrap();
    assert_vol(&s, 0.05f64.powi(3) - PI * 0.0001 * 0.05, 1e-2);
}

#[test]
fn huge_part_scale() {
    let a = Solid::cuboid(DVec3::splat(-10000.0), DVec3::splat(10000.0)).unwrap();
    let ball = Solid::sphere(DVec3::ZERO, 5000.0).unwrap();
    let s = a.boolean(&ball, BooleanOp::Difference).unwrap();
    assert_vol(&s, 8.0e12 - 4.0 / 3.0 * PI * 1.25e11, 1e-3);
}

// ---- Planar curve extrusion and free-form identities ----

use tri_geom::{NurbsCurve, Plane};

fn rectangle(w: f64, h: f64) -> NurbsCurve {
    NurbsCurve::polyline(vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(w, 0.0, 0.0),
        DVec3::new(w, h, 0.0),
        DVec3::new(0.0, h, 0.0),
        DVec3::new(0.0, 0.0, 0.0),
    ])
    .unwrap()
}

/// A closed smooth blob in the plane z = `z`: `r (1 + k cos(3a + ph))`, 8 control points.
fn blob(cx: f64, cy: f64, z: f64, r: f64, k: f64, ph: f64) -> NurbsCurve {
    let pts: Vec<DVec3> = (0..8)
        .map(|i| {
            let a = 2.0 * PI * i as f64 / 8.0;
            let rr = r * (1.0 + k * (3.0 * a + ph).cos());
            DVec3::new(cx + rr * a.cos(), cy + rr * a.sin(), z)
        })
        .collect();
    NurbsCurve::periodic(3, pts).unwrap()
}

fn plane_at(z: f64) -> Plane {
    Plane { origin: DVec3::new(0.0, 0.0, z), x_axis: DVec3::X, y_axis: DVec3::Y, z_axis: DVec3::Z }
}

#[test]
fn extrude_rectangle_volume() {
    let s = Solid::extrude_closed_curve(&rectangle(10.0, 6.0), &Plane::world_xy(), 2.0).unwrap();
    assert_vol(&s, 10.0 * 6.0 * 4.0, 1e-3);
}

#[test]
fn extrude_circle_is_an_analytic_cylinder() {
    let c = NurbsCurve::circle(&Plane::world_xy(), 5.0).unwrap();
    let s = Solid::extrude_closed_curve(&c, &Plane::world_xy(), 3.0).unwrap();
    assert_vol(&s, PI * 25.0 * 6.0, 1e-3);
    let rec = s.record().unwrap();
    let kinds: Vec<String> = rec.faces.iter().map(|f| format!("{:?}", f.surface).split(' ').next().unwrap().to_string()).collect();
    println!("circle prism faces: {kinds:?}");
    assert_eq!(kinds.iter().filter(|k| k.as_str() == "Cylinder").count(), 1, "one analytic wall: {kinds:?}");
}

#[test]
fn extrude_clockwise_curve_gives_the_same_solid() {
    let mut r = rectangle(10.0, 6.0);
    r.reverse();
    assert_vol(&Solid::extrude_closed_curve(&r, &Plane::world_xy(), 2.0).unwrap(), 240.0, 1e-3);
}

#[test]
fn extrude_curve_off_the_plane_is_refused() {
    let c = NurbsCurve::polyline(vec![
        DVec3::new(0.0, 0.0, 0.0),
        DVec3::new(4.0, 0.0, 0.0),
        DVec3::new(4.0, 4.0, 1.0),
        DVec3::new(0.0, 4.0, 0.0),
        DVec3::new(0.0, 0.0, 0.0),
    ])
    .unwrap();
    let e = Solid::extrude_closed_curve(&c, &Plane::world_xy(), 1.0).err().expect("non-planar curve is refused");
    assert!(e.to_string().contains("planar"), "{e}");
}

#[test]
fn extrude_open_curve_is_refused() {
    let c = NurbsCurve::line(DVec3::ZERO, DVec3::X).unwrap();
    assert!(Solid::extrude_closed_curve(&c, &Plane::world_xy(), 1.0).is_err());
}

fn free_form() -> Solid {
    Solid::extrude_closed_curve(&blob(0.0, 0.0, 15.0, 12.0, 0.2, 0.0), &plane_at(15.0), 15.0).unwrap()
}

fn check_identities(tool: Solid) {
    let l = free_form();
    let (vl, vt) = (l.volume().unwrap(), tool.volume().unwrap());
    let vi = l.boolean(&tool, BooleanOp::Intersection).unwrap().volume().unwrap();
    let vd = l.boolean(&tool, BooleanOp::Difference).unwrap().volume().unwrap();
    let vu = l.boolean(&tool, BooleanOp::Union).unwrap().volume().unwrap();
    assert!((vd + vi - vl).abs() < 1e-6, "(L-T)+(L^T)-L = {}", vd + vi - vl);
    assert!((vu - (vl + vt - vi)).abs() < 1e-6, "L+T-(L^T)-(LuT) = {}", vl + vt - vi - vu);
    assert!(vi > 0.0 && vd > 0.0, "the tool must overlap and not swallow L: {vi} {vd}");
}

#[test]
fn free_form_identities_with_an_oblique_cylinder() {
    check_identities(Solid::cylinder(DVec3::new(-20.0, -6.0, 8.0), DVec3::new(1.0, 0.25, 0.15), 4.0, 60.0).unwrap());
}

#[test]
fn free_form_identities_with_a_straddling_sphere() {
    check_identities(Solid::sphere(DVec3::new(10.0, 3.0, 15.0), 9.0).unwrap());
}

#[test]
fn free_form_identities_with_a_second_free_form() {
    let t = Solid::extrude_closed_curve(&blob(7.0, 4.0, 18.0, 10.0, 0.25, 1.0), &plane_at(18.0), 12.0).unwrap();
    check_identities(t);
}

#[test]
fn extrude_ellipse_volume() {
    let e = NurbsCurve::ellipse(&Plane::world_xy(), 6.0, 3.0).unwrap();
    assert_vol(&Solid::extrude_closed_curve(&e, &Plane::world_xy(), 2.0).unwrap(), PI * 6.0 * 3.0 * 4.0, 1e-3);
}

#[test]
fn extrude_clamped_interpolated_spline() {
    // a closed curve through 7 points (first = last): a clamped cubic, unlike the periodic blob
    let pts: Vec<DVec3> = (0..=6)
        .map(|i| {
            let a = 2.0 * PI * (i % 6) as f64 / 6.0;
            DVec3::new(10.0 * a.cos(), 10.0 * a.sin(), 0.0)
        })
        .collect();
    let c = NurbsCurve::interpolate(&pts, 3).unwrap();
    let v = Solid::extrude_closed_curve(&c, &Plane::world_xy(), 1.0).unwrap().volume().unwrap();
    // a circle of radius 10 through those points: area close to pi r^2 (spline bulge < 1 %)
    assert!((v / 2.0 - PI * 100.0).abs() / (PI * 100.0) < 0.01, "area {}", v / 2.0);
}

// ---- record -> NURBS faces ----

/// Signed volume of a Brep's display mesh (divergence theorem), the measure the app uses.
fn mesh_volume(b: &tri_geom::Brep, segs: usize) -> f64 {
    let m = b.tessellate(segs, segs);
    (0..m.triangle_count())
        .map(|t| {
            let (a, b, c) = m.triangle(t);
            a.dot(b.cross(c)) / 6.0
        })
        .sum::<f64>()
        .abs()
}

fn check_faces(name: &str, s: &Solid, tol_pct: f64) {
    let size = 100.0;
    let b = s.to_brep(1e-3 * size).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert!(b.closed, "{name}: the Brep is not closed");
    assert_eq!(b.faces.len(), s.record().unwrap().faces.len(), "{name}: one face per record face");
    let want = s.volume().unwrap();
    let got = mesh_volume(&b, 64);
    assert!(pct(got, want).abs() <= tol_pct, "{name}: mesh volume {got} vs kernel {want} ({:+.4} %)", pct(got, want));
    assert!(b.exact_valid().is_some(), "{name}: exact body attached and valid");
    let back = Solid::from_exact(&b).unwrap_or_else(|| panic!("{name}: from_exact"));
    assert!(pct(back.volume().unwrap(), want).abs() < 1e-6, "{name}: round trip volume");
}

#[test]
fn faces_of_a_box() {
    check_faces("box", &Solid::cuboid(DVec3::ZERO, DVec3::new(20.0, 30.0, 40.0)).unwrap(), 1e-6);
}

#[test]
fn faces_of_a_cylinder() {
    check_faces("cylinder", &Solid::cylinder(DVec3::ZERO, DVec3::Z, 10.0, 30.0).unwrap(), 0.2);
}

#[test]
fn faces_of_a_sphere() {
    check_faces("sphere", &Solid::sphere(DVec3::new(5.0, 5.0, 5.0), 20.0).unwrap(), 0.2);
}

#[test]
fn faces_of_a_torus() {
    check_faces("torus", &Solid::torus(DVec3::ZERO, DVec3::Z, 14.0, 6.0).unwrap(), 0.2);
}

#[test]
fn faces_of_a_cone() {
    check_faces("cone", &Solid::cone(DVec3::ZERO, DVec3::Z, 10.0, 3.0, 25.0).unwrap(), 0.2);
}

#[test]
fn faces_of_sphere_minus_two_cylinders() {
    let s = Solid::sphere(DVec3::ZERO, 20.0).unwrap();
    let c1 = Solid::cylinder(DVec3::new(5.0, -30.0, 3.0), DVec3::Y, 8.0, 60.0).unwrap();
    let c2 = Solid::cylinder(DVec3::new(-30.0, 0.0, 8.0), DVec3::new(1.0, 0.3, 0.1), 6.0, 60.0).unwrap();
    let r = s.boolean(&c1, BooleanOp::Difference).unwrap().boolean(&c2, BooleanOp::Difference).unwrap();
    check_faces("sphere minus two cylinders", &r, 0.3);
}

#[test]
fn faces_of_a_torus_minus_a_bore() {
    let t = Solid::torus(DVec3::ZERO, DVec3::Z, 14.0, 6.0).unwrap();
    let bore = Solid::cylinder(DVec3::new(14.0, -20.0, 0.0), DVec3::Y, 3.0, 40.0).unwrap();
    check_faces("torus minus a bore", &t.boolean(&bore, BooleanOp::Difference).unwrap(), 0.3);
}

#[test]
fn faces_of_a_spline_prism() {
    check_faces("spline prism", &free_form(), 0.3);
}

#[test]
fn from_exact_refuses_a_stale_body() {
    let s = Solid::cuboid(DVec3::ZERO, DVec3::splat(10.0)).unwrap();
    let mut b = s.to_brep(0.01).unwrap();
    b.faces[0].surface.control_points[0].x += 0.001;
    assert!(Solid::from_exact(&b).is_none());
}

#[test]
fn triceratops_own_closure_check_agrees_with_the_kernel() {
    let cases: Vec<(&str, Solid)> = vec![
        ("box", Solid::cuboid(DVec3::ZERO, DVec3::new(20.0, 30.0, 40.0)).unwrap()),
        ("cylinder", Solid::cylinder(DVec3::ZERO, DVec3::Z, 10.0, 30.0).unwrap()),
        ("sphere", Solid::sphere(DVec3::ZERO, 20.0).unwrap()),
        ("torus", Solid::torus(DVec3::ZERO, DVec3::Z, 14.0, 6.0).unwrap()),
        ("spline prism", free_form()),
    ];
    for (name, s) in cases {
        let b = s.to_brep(0.1).unwrap();
        assert!(b.compute_closed(), "{name}: tri-geom's own geometric closure check says open");
    }
}

// ---- cancellation ----

#[test]
fn a_cancelled_job_stops_a_boolean_and_leaves_the_operands() {
    let a = plate();
    let b = Solid::cylinder(DVec3::ZERO, DVec3::Z, 8.0, 20.0).unwrap();
    let token = tri_geom::job::Token::new();
    token.cancel();
    let _guard = tri_geom::job::install(token);
    let e = a.boolean(&b, BooleanOp::Difference).err().expect("cancelled");
    assert_eq!(e, tri_kernel::KernelError::Cancelled);
    assert_vol(&a, 32000.0, 1e-3);
    assert_vol(&b, PI * 64.0 * 20.0, 1e-3);
}

#[test]
fn a_job_that_is_not_cancelled_lets_the_boolean_finish() {
    let a = plate();
    let b = Solid::cylinder(DVec3::ZERO, DVec3::Z, 8.0, 20.0).unwrap();
    let _guard = tri_geom::job::install(tri_geom::job::Token::new());
    assert_vol(&a.boolean(&b, BooleanOp::Difference).unwrap(), 40.0 * 40.0 * 20.0 - PI * 64.0 * 20.0, 1e-3);
}

#[test]
fn near_coincident_free_form_walls_are_refused_with_a_message() {
    let a = Solid::extrude_closed_curve(&blob(0.0, 0.0, 15.0, 12.0, 0.2, 0.0), &plane_at(15.0), 15.0).unwrap();
    let b = Solid::extrude_closed_curve(&blob(0.001, 0.0, 15.0, 12.0, 0.2, 0.0), &plane_at(15.0), 15.0).unwrap();
    let e = a.boolean(&b, BooleanOp::Difference).err().expect("the kernel refuses this pair");
    assert!(!e.to_string().is_empty());
}

