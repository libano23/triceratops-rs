//! Analysis: areas, volumes, centroids, bounding boxes, distances and closest points.

use glam::DVec3;
use tri_doc::Geometry;

use super::super::data::Value;
use super::geo;
use super::*;

fn bbox_of(v: &Value) -> Option<(DVec3, DVec3)> {
    match v {
        Value::Point(q) => Some((*q, *q)),
        Value::Geometry(gm) => {
            let bb = gm.bounding_box();
            (!bb.is_empty()).then_some((bb.min, bb.max))
        }
        _ => None,
    }
}

fn geom_of(v: &Value) -> Option<Geometry> {
    match v {
        Value::Geometry(gm) => Some(gm.as_ref().clone()),
        Value::Point(q) => Some(Geometry::Point(*q)),
        _ => None,
    }
}

pub fn register(r: &mut Registry) {
    r.elem("Area", "Analysis", vec![geo("Geometry")], &["Area"], |_, a| one(n(geo::area(a.geo(0)?)?)));
    r.elem("Volume", "Analysis", vec![geo("Geometry")], &["Volume"], |_, a| one(n(geo::volume(a.geo(0)?)?.0)));
    r.elem("Bounding Box", "Analysis", vec![geo("Geometry")], &["Min", "Max"], |_, a| {
        let (lo, hi) = bbox_of(a.get(0))?;
        outs(vec![Out::One(p(lo)), Out::One(p(hi))])
    });
    r.elem("Deconstruct Box", "Analysis", vec![geo("Geometry")], &["Min", "Max", "Center", "Size"], |_, a| {
        let (lo, hi) = bbox_of(a.get(0))?;
        outs(vec![Out::One(p(lo)), Out::One(p(hi)), Out::One(p((lo + hi) / 2.0)), Out::One(p(hi - lo))])
    });
    r.elem("Centroid", "Analysis", vec![geo("Geometry")], &["Point"], |_, a| one(p(geo::centroid(&geom_of(a.get(0))?)?)));
    r.elem("Volume Centroid", "Analysis", vec![geo("Solid")], &["Point"], |_, a| one(p(geo::centroid(&geom_of(a.get(0))?)?)));
    r.elem("Area Centroid", "Analysis", vec![geo("Geometry")], &["Point"], |_, a| {
        let gm = geom_of(a.get(0))?;
        one(p(geo::area_centroid(&gm).or_else(|| geo::centroid(&gm))?))
    });
    r.elem("Surface Area Centroid", "Analysis", vec![geo("Surface")], &["Point"], |_, a| one(p(geo::area_centroid(a.geo(0)?)?)));
    r.elem("Distance", "Analysis", vec![pt("A", 0.0, 0.0, 0.0), pt("B", 1.0, 0.0, 0.0)], &["Distance"], |_, a| one(n((a.pt(0) - a.pt(1)).length())));
    r.elem("Closest Point", "Analysis", vec![geo("Geometry"), pt("Point", 0.0, 0.0, 0.0)], &["Point", "Distance"], |_, a| {
        let (q, d) = geo::closest_point(&geom_of(a.get(0))?, a.pt(1))?;
        outs(vec![Out::One(p(q)), Out::One(n(d))])
    });
}
