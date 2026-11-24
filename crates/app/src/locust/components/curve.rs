//! Curve: lines, arcs, polylines, interpolated and control-point curves, and the tools
//! that measure, divide and edit them.

use glam::DVec3;
use tri_geom::NurbsCurve;

use super::super::data::{as_point, DataTree, Value};
use super::geo::{self, curves};
use super::*;


/// Points dividing a curve; a closed curve drops the repeated end.
fn divide(c: &NurbsCurve, n: usize) -> Vec<DVec3> {
    // the division points sit on the tessellation: put them exactly on the curve
    let mut pts: Vec<DVec3> = c.divide_by_count(n.max(1)).into_iter().map(|q| c.point_at(c.closest_parameter(q))).collect();
    if c.is_closed() && pts.len() > 1 && (pts[0] - pts[pts.len() - 1]).length() < 1e-9 {
        pts.pop();
    }
    pts
}

pub fn register(r: &mut Registry) {
    r.elem("Line", "Curve", vec![pt("Start", 0.0, 0.0, 0.0), pt("End", 1.0, 0.0, 0.0)], &["Line"], |_, a| NurbsCurve::line(a.pt(0), a.pt(1)).ok().and_then(|c| one(gc(c))));
    r.elem("Line SDL", "Curve", vec![pt("Start", 0.0, 0.0, 0.0), pt("Direction", 1.0, 0.0, 0.0), num("Length", 1.0)], &["Line"], |_, a| {
        let d = a.pt(1);
        if d.length() < 1e-12 {
            return None;
        }
        NurbsCurve::line(a.pt(0), a.pt(0) + d.normalize() * a.num(2)).ok().and_then(|c| one(gc(c)))
    });
    r.elem("Circle", "Curve", vec![pt("Center", 0.0, 0.0, 0.0), num("Radius", 5.0), pt("Normal", 0.0, 0.0, 1.0)], &["Circle"], |_, a| {
        if a.num(1) < 1e-9 {
            return None;
        }
        NurbsCurve::circle(&geo::plane(a.pt(0), a.dir(2, DVec3::Z)), a.num(1)).ok().and_then(|c| one(gc(c)))
    });
    r.elem("Circle 2Pt", "Curve", vec![pt("Start", -1.0, 0.0, 0.0), pt("End", 1.0, 0.0, 0.0), pt("Normal", 0.0, 0.0, 1.0)], &["Circle"], |_, a| {
        let (s, e) = (a.pt(0), a.pt(1));
        let rad = (s - e).length() / 2.0;
        if rad < 1e-9 {
            return None;
        }
        NurbsCurve::circle(&geo::plane((s + e) / 2.0, a.dir(2, DVec3::Z)), rad).ok().and_then(|c| one(gc(c)))
    });
    r.elem("Arc 3Pt", "Curve", vec![pt("Start", 1.0, 0.0, 0.0), pt("Middle", 0.0, 1.0, 0.0), pt("End", -1.0, 0.0, 0.0)], &["Arc"], |_, a| NurbsCurve::arc_3pt(a.pt(0), a.pt(1), a.pt(2)).ok().and_then(|c| one(gc(c))));
    r.elem("Arc", "Curve", vec![pt("Center", 0.0, 0.0, 0.0), num("Radius", 5.0), num("Angle", 90.0), pt("Normal", 0.0, 0.0, 1.0)], &["Arc"], |_, a| {
        if a.num(1) < 1e-9 || a.num(2).abs() < 1e-9 {
            return None;
        }
        let ang = a.num(2).to_radians();
        let (lo, hi) = if ang > 0.0 { (0.0, ang) } else { (ang, 0.0) };
        NurbsCurve::arc(&geo::plane(a.pt(0), a.dir(3, DVec3::Z)), a.num(1), lo, hi).ok().and_then(|c| one(gc(c)))
    });
    r.elem("Ellipse", "Curve", vec![pt("Center", 0.0, 0.0, 0.0), num("Radius 1", 5.0), num("Radius 2", 3.0), pt("Normal", 0.0, 0.0, 1.0)], &["Ellipse"], |_, a| {
        if a.num(1) < 1e-9 || a.num(2) < 1e-9 {
            return None;
        }
        NurbsCurve::ellipse(&geo::plane(a.pt(0), a.dir(3, DVec3::Z)), a.num(1), a.num(2)).ok().and_then(|c| one(gc(c)))
    });
    r.elem("Rectangle", "Curve", vec![pt("Plane Origin", 0.0, 0.0, 0.0), num("Width", 10.0), num("Height", 5.0)], &["Rectangle"], |_, a| {
        let (o, w, h) = (a.pt(0), a.num(1), a.num(2));
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        geo::polyline(vec![o, o + DVec3::X * w, o + DVec3::new(w, h, 0.0), o + DVec3::Y * h], true).and_then(|c| one(gc(c)))
    });
    r.elem("Polygon", "Curve", vec![pt("Center", 0.0, 0.0, 0.0), num("Radius", 5.0), int("Sides", 6), pt("Normal", 0.0, 0.0, 1.0)], &["Polygon"], |_, a| geo::polygon(a.pt(0), a.num(1), a.int(2), a.dir(3, DVec3::Z)).and_then(|c| one(gc(c))));
    r.tree("Polyline", "Curve", vec![tree("Vertices")], &["Curve"], |_, t| {
        t1(DataTree::from_branches(t[0].paths().into_iter().filter_map(|path| {
            let pts: Vec<DVec3> = t[0].branch_ref(&path).iter().map(as_point).collect();
            (pts.len() >= 2).then(|| geo::polyline(pts, false)).flatten().map(|c| (path, vec![gc(c)]))
        })))
    })
    .describe("A curve through every point, per branch");
    r.tree("Interpolate", "Curve", vec![tree("Vertices"), boolean("Periodic", false)], &["Curve"], |_, t| {
        let closed = first_bool(&t[1], false);
        t1(DataTree::from_branches(t[0].paths().into_iter().filter_map(|path| {
            let pts: Vec<DVec3> = t[0].branch_ref(&path).iter().map(as_point).collect();
            (pts.len() >= if closed { 3 } else { 2 }).then(|| geo::interpolate(&pts, closed)).flatten().map(|c| (path, vec![gc(c)]))
        })))
    });
    r.tree("Nurbs Curve", "Curve", vec![tree("Control Points"), int("Degree", 3)], &["Curve"], |_, t| {
        let d = first_int(&t[1], 3).max(1) as usize;
        t1(DataTree::from_branches(t[0].paths().into_iter().filter_map(|path| {
            let pts: Vec<DVec3> = t[0].branch_ref(&path).iter().map(as_point).collect();
            let deg = d.min(pts.len().saturating_sub(1)).max(1);
            (pts.len() >= 2).then(|| NurbsCurve::from_control_points(deg, pts).ok()).flatten().map(|c| (path, vec![gc(c)]))
        })))
    });
    r.elem("Offset Curve", "Curve", vec![geo("Curve"), num("Distance", 1.0), pt("Plane Normal", 0.0, 0.0, 1.0)], &["Curve"], |_, a| {
        let c = a.curve(0)?;
        let pl = tri_geom::curve_plane(&c).unwrap_or_else(|| geo::plane(c.start_point(), a.dir(2, DVec3::Z)));
        let tol = c.bounding_box().diagonal().max(1e-6) * 1e-4;
        c.offset(&pl, a.num(1), tol).ok().and_then(|o| one(gc(o)))
    });
    r.elem("Offset Curve Loose", "Curve", vec![geo("Curve"), num("Distance", 1.0), pt("Normal", 0.0, 0.0, 1.0)], &["Curve"], |_, a| {
        // loose: the control points move along the offset directions, no refit
        let c = a.curve(0)?;
        let n = a.dir(2, DVec3::Z).normalize_or(DVec3::Z);
        let mut out = c.clone();
        for (i, cp) in out.control_points.iter_mut().enumerate() {
            let t = c.closest_parameter(c.control_points[i]);
            let side = c.tangent_at(t).cross(n).normalize_or(DVec3::X);
            *cp += side * -a.num(1);
        }
        one(gc(out))
    });
    r.tree("Divide Curve", "Curve", vec![geo("Curve"), int("Count", 10)], &["Points"], |_, t| {
        let n = first_int(&t[1], 10).max(2) as usize;
        t1(DataTree::from_list(curves(&t[0].items()).iter().flat_map(|c| divide(c, n - 1)).map(Value::Point).collect()))
    });
    r.tree("Divide Distance", "Curve", vec![geo("Curve"), num("Distance", 1.0)], &["Points"], |_, t| {
        let step = first_num(&t[1], 1.0).max(1e-6);
        t1(DataTree::from_list(curves(&t[0].items()).iter().flat_map(|c| {
            let n = ((c.length() / step) as usize).max(1);
            divide(c, n)
        }).map(Value::Point).collect()))
    });
    r.elem("Curve Length", "Curve", vec![geo("Curve")], &["Length"], |_, a| one(n(a.curve(0)?.length())));
    r.elem("Curve End", "Curve", vec![geo("Curve")], &["Start", "End"], |_, a| {
        let c = a.curve(0)?;
        outs(vec![Out::One(p(c.start_point())), Out::One(p(c.end_point()))])
    });
    r.elem("Curve Midpoint", "Curve", vec![geo("Curve")], &["Point"], |_, a| one(p(geo::point_at_length_fraction(&a.curve(0)?, 0.5))));
    r.elem("Extend Curve", "Curve", vec![geo("Curve"), num("Length", 1.0), text("End", "end")], &["Curve"], |_, a| {
        let c = a.curve(0)?;
        let len = a.num(1);
        if len <= 0.0 {
            return None;
        }
        let end = a.text(2).to_ascii_lowercase();
        let out = match end.as_str() {
            "start" => c.extend_by(false, len).ok()?,
            "both" => c.extend_by(false, len).ok()?.extend_by(true, len).ok()?,
            _ => c.extend_by(true, len).ok()?,
        };
        one(gc(out))
    });
    r.elem("Curve Closest Point", "Curve", vec![geo("Curve"), pt("Point", 0.0, 0.0, 0.0)], &["Point"], |_, a| {
        let c = a.curve(0)?;
        one(p(c.point_at(c.closest_parameter(a.pt(1)))))
    });
    r.elem("Evaluate Curve", "Curve", vec![geo("Curve"), num("t", 0.5)], &["Point", "Tangent"], |_, a| {
        let c = a.curve(0)?;
        let t = geo::param(&c, a.num(1));
        outs(vec![Out::One(p(c.point_at(t))), Out::One(p(c.tangent_at(t)))])
    });
    r.elem("Point On Curve", "Curve", vec![geo("Curve"), num("t", 0.5)], &["Point"], |_, a| {
        let c = a.curve(0)?;
        one(p(c.point_at(geo::param(&c, a.num(1)))))
    });
    r.elem("Perp Frame", "Curve", vec![geo("Curve"), num("t", 0.5)], &["Point", "Tangent", "Normal"], |_, a| {
        let c = a.curve(0)?;
        let t = geo::param(&c, a.num(1));
        let tan = c.tangent_at(t);
        // the curve's own plane normal when it has one, else a normal square to the tangent
        let normal = tri_geom::curve_plane(&c).map(|pl| pl.z_axis).filter(|z| z.cross(tan).length() > 1e-9).unwrap_or_else(|| geo::axes_from_normal(tan).0);
        outs(vec![Out::One(p(c.point_at(t))), Out::One(p(tan)), Out::One(p(normal))])
    });
    r.tree("Curve Control Points", "Curve", vec![geo("Curve")], &["Points"], |_, t| t1(DataTree::from_list(curves(&t[0].items()).iter().flat_map(|c| c.control_points.clone()).map(Value::Point).collect())));
    r.elem("Rebuild Curve", "Curve", vec![geo("Curve"), int("Count", 10), int("Degree", 3)], &["Curve"], |_, a| {
        let c = a.curve(0)?;
        let deg = a.int(2).max(1) as usize;
        let count = (a.int(1).max(2) as usize).max(deg + 1);
        c.rebuild(count, deg).ok().and_then(|o| one(gc(o)))
    });
    r.elem("Reverse Curve", "Curve", vec![geo("Curve")], &["Curve"], |_, a| {
        let mut c = a.curve(0)?;
        c.reverse();
        one(gc(c))
    });
    r.elem("Curve Seam", "Curve", vec![geo("Curve"), pt("Point", 0.0, 0.0, 0.0)], &["Curve"], |_, a| geo::curve_seam(&a.curve(0)?, a.pt(1)).and_then(|c| one(gc(c))));
    r.tree("Join Curves", "Curve", vec![tree("Curves")], &["Curve"], |_, t| {
        let cs = curves(&t[0].items());
        if cs.len() < 2 {
            return t1(t[0].clone());
        }
        t1(DataTree::from_list(geo::join_curves(cs).into_iter().map(gc).collect()))
    });
    r.tree("Explode", "Curve", vec![geo("Curve")], &["Segments"], |_, t| t1(DataTree::from_list(curves(&t[0].items()).iter().flat_map(crate::commands::edit::explode_curve).map(gc).collect())));
    r.elem("Curve Curvature", "Curve", vec![geo("Curve"), pt("Point", 0.0, 0.0, 0.0)], &["Curvature", "Radius"], |_, a| {
        let c = a.curve(0)?;
        let (k, rad) = geo::curvature(&c, c.closest_parameter(a.pt(1)));
        outs(vec![Out::One(n(k)), if rad.is_finite() { Out::One(n(rad)) } else { Out::Skip }])
    });
    r.elem("Curve Domain", "Curve", vec![geo("Curve")], &["Domain"], |_, a| {
        let (lo, hi) = a.curve(0)?.domain();
        one(Value::Pair(lo, hi))
    });
    // intersections of every A curve with every B curve
    r.tree("Curve-Curve Intersection", "Intersect", vec![geo("A"), geo("B")], &["Points"], |_, t| {
        let (ca, cb) = (curves(&t[0].items()), curves(&t[1].items()));
        let mut pts = Vec::new();
        for x in &ca {
            for y in &cb {
                let tol = (x.bounding_box().diagonal() + y.bounding_box().diagonal()).max(1e-6) * 1e-6;
                for hit in tri_geom::curve_curve_intersections(x, y, tol) {
                    pts.push(Value::Point(hit.point));
                }
            }
        }
        t1(DataTree::from_list(pts))
    });
}
