//! Transform: move, rotate, scale, mirror, arrays, orient, non-uniform scale and twist.
//! Points are transformed as well as geometry.

use glam::{DAffine3, DQuat, DVec3};
use tri_doc::Geometry;

use super::super::data::Value;
use super::geo;
use super::*;

/// Apply an affine transform to a geometry or point value.
fn xform(v: &Value, xf: &DAffine3) -> Option<Value> {
    match v {
        Value::Geometry(gm) => Some(g(geo::transformed(gm, xf))),
        Value::Point(q) => Some(Value::Point(xf.transform_point3(*q))),
        Value::Plane(o, n) => Some(Value::Plane(xf.transform_point3(*o), xf.transform_vector3(*n).normalize_or(*n))),
        _ => None,
    }
}

pub fn register(r: &mut Registry) {
    r.elem("Move", "Transform", vec![geo("Geometry"), pt("Vector", 0.0, 0.0, 0.0)], &["Geometry"], |_, a| xform(a.get(0), &DAffine3::from_translation(a.pt(1))).and_then(one));
    r.elem("Rotate", "Transform", vec![geo("Geometry"), num("Angle", 90.0), pt("Axis Point", 0.0, 0.0, 0.0), pt("Axis", 0.0, 0.0, 1.0)], &["Geometry"], |_, a| xform(a.get(0), &geo::rotation(a.pt(2), a.dir(3, DVec3::Z), a.num(1))).and_then(one));
    r.elem("Scale", "Transform", vec![geo("Geometry"), pt("Center", 0.0, 0.0, 0.0), num("Factor", 2.0)], &["Geometry"], |_, a| {
        let c = a.pt(1);
        xform(a.get(0), &(DAffine3::from_translation(c) * DAffine3::from_scale(DVec3::splat(a.num(2))) * DAffine3::from_translation(-c))).and_then(one)
    });
    r.elem("Scale NU", "Transform", vec![geo("Geometry"), pt("Center", 0.0, 0.0, 0.0), num("Factor X", 1.0), num("Factor Y", 1.0), num("Factor Z", 1.0)], &["Geometry"], |_, a| {
        let c = a.pt(1);
        xform(a.get(0), &(DAffine3::from_translation(c) * DAffine3::from_scale(DVec3::new(a.num(2), a.num(3), a.num(4))) * DAffine3::from_translation(-c))).and_then(one)
    });
    r.elem("Mirror", "Transform", vec![geo("Geometry"), pt("Plane Point", 0.0, 0.0, 0.0), pt("Plane Normal", 1.0, 0.0, 0.0)], &["Geometry"], |_, a| {
        let xf = geo::mirror(a.pt(1), a.dir(2, DVec3::X));
        match a.get(0) {
            Value::Geometry(gm) => {
                // a reflection turns surfaces inside out: flip them back
                let mut x = geo::transformed(gm, &xf);
                match &mut x {
                    Geometry::Surface(s) => s.flip(),
                    Geometry::Brep(b) => b.faces.iter_mut().for_each(|f| f.flip()),
                    Geometry::Mesh(m) => m.flip(),
                    _ => {}
                }
                one(g(x))
            }
            v => xform(v, &xf).and_then(one),
        }
    });
    r.elem("Array Linear", "Transform", vec![geo("Geometry"), pt("Direction", 1.0, 0.0, 0.0), int("Count", 5), num("Spacing", 10.0)], &["Geometry"], |_, a| {
        let d = a.dir(1, DVec3::X);
        if d.length() < 1e-12 {
            return None;
        }
        let d = d.normalize();
        many((0..a.int(2).max(1)).filter_map(|i| xform(a.get(0), &DAffine3::from_translation(d * a.num(3) * i as f64))).collect())
    });
    r.elem("Array Polar", "Transform", vec![geo("Geometry"), pt("Center", 0.0, 0.0, 0.0), int("Count", 6), num("Angle", 360.0), pt("Axis", 0.0, 0.0, 1.0)], &["Geometry"], |_, a| {
        let k = a.int(2).max(1);
        many((0..k).filter_map(|i| xform(a.get(0), &geo::rotation(a.pt(1), a.dir(4, DVec3::Z), a.num(3) * i as f64 / k as f64))).collect())
    });
    r.elem("Orient", "Transform", vec![geo("Geometry"), pt("From Origin", 0.0, 0.0, 0.0), pt("From Normal", 0.0, 0.0, 1.0), pt("To Origin", 0.0, 0.0, 0.0), pt("To Normal", 0.0, 0.0, 1.0)], &["Geometry"], |_, a| {
        let f0 = geo::plane(a.pt(1), a.dir(2, DVec3::Z));
        let f1 = geo::plane(a.pt(3), a.dir(4, DVec3::Z));
        xform(a.get(0), &geo::frame_to_frame(&f0, &f1)).and_then(one)
    });
    r.elem("Twist", "Transform", vec![geo("Geometry"), num("Angle", 90.0), pt("Axis Point", 0.0, 0.0, 0.0), pt("Axis", 0.0, 0.0, 1.0)], &["Geometry"], |_, a| {
        // rotation grows with the height along the axis, over the object's extent
        let mut x = a.geo(0)?.clone();
        let (o, axis) = (a.pt(2), a.dir(3, DVec3::Z).normalize_or(DVec3::Z));
        let bb = x.bounding_box();
        let hs: Vec<f64> = bb.corners().iter().map(|c| (*c - o).dot(axis)).collect();
        let (lo, hi) = (hs.iter().cloned().fold(f64::INFINITY, f64::min), hs.iter().cloned().fold(f64::NEG_INFINITY, f64::max));
        let span = (hi - lo).max(1e-12);
        let total = a.num(1).to_radians();
        crate::commands::misc3::deform_pub(&mut x, &|q| {
            let h = (q - o).dot(axis);
            let radial = (q - o) - axis * h;
            o + axis * h + DQuat::from_axis_angle(axis, total * ((h - lo) / span).clamp(0.0, 1.0)) * radial
        });
        one(g(x))
    });
}
