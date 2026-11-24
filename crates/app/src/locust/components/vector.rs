//! Vector: points and vectors.

use glam::DVec3;

use super::super::data::{as_point, DataTree, Value};
use super::*;

fn rotate_vec(v: DVec3, axis: DVec3, deg: f64) -> DVec3 {
    if axis.length() < 1e-12 {
        return v;
    }
    let a = axis.normalize();
    let ang = deg.to_radians();
    v * ang.cos() + a.cross(v) * ang.sin() + a * a.dot(v) * (1.0 - ang.cos())
}

pub fn register(r: &mut Registry) {
    r.elem("Deconstruct Point", "Vector", vec![pt("Point", 0.0, 0.0, 0.0)], &["X", "Y", "Z"], |_, a| {
        let q = a.pt(0);
        outs(vec![Out::One(n(q.x)), Out::One(n(q.y)), Out::One(n(q.z))])
    });
    r.elem("Vector 2Pt", "Vector", vec![pt("From", 0.0, 0.0, 0.0), pt("To", 1.0, 0.0, 0.0)], &["Vector"], |_, a| one(p(a.pt(1) - a.pt(0))));
    r.elem("Amplitude", "Vector", vec![pt("Vector", 1.0, 0.0, 0.0)], &["Length"], |_, a| one(n(a.pt(0).length())));
    r.elem("Unit Z", "Vector", vec![num("Factor", 1.0)], &["Vector"], |_, a| one(p(DVec3::new(0.0, 0.0, a.num(0)))));
    r.elem("Dot Product", "Vector", vec![pt("A", 1.0, 0.0, 0.0), pt("B", 1.0, 0.0, 0.0)], &["Dot"], |_, a| one(n(a.pt(0).dot(a.pt(1)))));
    r.elem("Cross Product", "Vector", vec![pt("A", 1.0, 0.0, 0.0), pt("B", 0.0, 1.0, 0.0)], &["Vector"], |_, a| one(p(a.pt(0).cross(a.pt(1)))));
    r.elem("Unit Vector", "Vector", vec![pt("Vector", 1.0, 0.0, 0.0)], &["Vector"], |_, a| {
        let v = a.pt(0);
        one(p(if v.length() > 1e-12 { v / v.length() } else { DVec3::ZERO }))
    });
    r.elem("Vector Length", "Vector", vec![pt("Vector", 1.0, 0.0, 0.0)], &["Length"], |_, a| one(n(a.pt(0).length())));
    r.elem("Vector Add", "Vector", vec![pt("A", 0.0, 0.0, 0.0), pt("B", 0.0, 0.0, 0.0)], &["Vector"], |_, a| one(p(a.pt(0) + a.pt(1))));
    r.elem("Vector Subtract", "Vector", vec![pt("A", 0.0, 0.0, 0.0), pt("B", 0.0, 0.0, 0.0)], &["Vector"], |_, a| one(p(a.pt(0) - a.pt(1))));
    r.elem("Vector Scale", "Vector", vec![pt("Vector", 1.0, 0.0, 0.0), num("Factor", 2.0)], &["Vector"], |_, a| one(p(a.pt(0) * a.num(1))));
    r.elem("Vector Angle", "Vector", vec![pt("A", 1.0, 0.0, 0.0), pt("B", 0.0, 1.0, 0.0)], &["Angle"], |_, a| {
        let (u, v) = (a.pt(0), a.pt(1));
        let (nu, nv) = (u.length(), v.length());
        if nu < 1e-12 || nv < 1e-12 {
            return one(n(0.0));
        }
        one(n((u.dot(v) / (nu * nv)).clamp(-1.0, 1.0).acos().to_degrees()))
    });
    r.elem("Vector Rotate", "Vector", vec![pt("Vector", 1.0, 0.0, 0.0), num("Angle", 90.0), pt("Axis", 0.0, 0.0, 1.0)], &["Vector"], |_, a| one(p(rotate_vec(a.pt(0), a.pt(2), a.num(1)))));
    r.elem("Point Add", "Vector", vec![pt("Point", 0.0, 0.0, 0.0), pt("Vector", 0.0, 0.0, 0.0)], &["Point"], |_, a| one(p(a.pt(0) + a.pt(1))));
    r.elem("Point Subtract", "Vector", vec![pt("Point", 0.0, 0.0, 0.0), pt("Vector", 0.0, 0.0, 0.0)], &["Point"], |_, a| one(p(a.pt(0) - a.pt(1))));
    r.tree("Point Average", "Vector", vec![tree("Tree")], &["Point"], |_, t| {
        let pts: Vec<DVec3> = t[0].items().iter().map(as_point).collect();
        if pts.is_empty() {
            return t1(DataTree::empty());
        }
        t1(DataTree::single(Value::Point(pts.iter().sum::<DVec3>() / pts.len() as f64)))
    });
}
