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
