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
