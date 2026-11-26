//! Solid: Booleans, fillets and chamfers, splitting and cutting.

use glam::DVec3;
use tri_doc::Geometry;
use tri_geom::{BooleanOp, Face};

use super::super::data::{as_point, DataTree, Value};
use super::geo::{self, curves};
use super::*;
use crate::commands::{misc4, surfaces3};

/// A Boolean whose failure is noted on the node ("Boolean too complex …", "cancelled", …).
fn boolean(ctx: &mut Ctx, a: &Geometry, b: &Geometry, op: BooleanOp) -> Option<Geometry> {
    match misc4::boolean_geometry_checked(a, b, op) {
        Ok(r) if misc4::geometry_nonempty(&r) => Some(r),
        Ok(_) => {
            ctx.warn("the Boolean result is empty");
            None
        }
        Err(e) => {
            ctx.error(format!("Boolean failed: {e}"));
            None
        }
    }
}

fn geoms(t: &DataTree) -> Vec<Geometry> {
    t.items().iter().filter_map(|v| v.as_geometry().cloned()).collect()
}

/// Split one object by a cutter: a curve cuts along `dir` (WireCut), a closed solid cuts
/// into the part inside and the part outside it.
fn split(ctx: &mut Ctx, target: &Geometry, cutter: &Geometry, dir: DVec3) -> Vec<Geometry> {
    match cutter {
        Geometry::Curve(c) => {
            let tol = target.bounding_box().diagonal().max(1e-6) * 1e-6;
            surfaces3::wire_cut_geometry(target, c, dir, &[], tol)
        }
        _ if cutter.is_solid() => [boolean(ctx, target, cutter, BooleanOp::Intersection), boolean(ctx, target, cutter, BooleanOp::Difference)].into_iter().flatten().collect(),
        _ => Vec::new(),
    }
}

fn reduce(ctx: &mut Ctx, items: Vec<Geometry>, op: BooleanOp) -> Option<Geometry> {
    let mut it = items.into_iter();
    let mut acc = it.next()?;
    for s in it {
        acc = boolean(ctx, &acc, &s, op)?;
    }
    Some(acc)
}

pub fn register(r: &mut Registry) {
    r.elem("Union", "Solid", vec![geo("A"), geo("B")], &["Result"], |ctx, a| og(boolean(ctx, a.geo(0)?, a.geo(1)?, BooleanOp::Union)));
    r.elem("Difference", "Solid", vec![geo("A"), geo("B")], &["Result"], |ctx, a| og(boolean(ctx, a.geo(0)?, a.geo(1)?, BooleanOp::Difference)));
    r.elem("Intersection", "Solid", vec![geo("A"), geo("B")], &["Result"], |ctx, a| og(boolean(ctx, a.geo(0)?, a.geo(1)?, BooleanOp::Intersection)));
    r.tree("Solid Union", "Solid", vec![tree("Shapes")], &["Result"], |ctx, t| {
        let items = geoms(&t[0]);
        if items.len() < 2 {
            return t1(t[0].clone());
        }
        t1(reduce(ctx, items, BooleanOp::Union).map(|x| DataTree::single(g(x))).unwrap_or_default())
    })
    .describe("Union every shape in the tree");
    r.tree("Solid Intersection", "Solid", vec![tree("Shapes")], &["Solid"], |ctx, t| {
        let items = geoms(&t[0]);
        if items.len() < 2 {
            return t1(t[0].clone());
        }
        t1(reduce(ctx, items, BooleanOp::Intersection).map(|x| DataTree::single(g(x))).unwrap_or_default())
    });
    r.tree("Solid Difference", "Solid", vec![tree("Shapes"), geo("Cutter")], &["Solid"], |ctx, t| {
        let Some(cut) = t[1].first().and_then(|v| v.as_geometry().cloned()) else { return t1(t[0].clone()) };
        let pieces: Vec<Geometry> = geoms(&t[0]).iter().map(|s| boolean(ctx, s, &cut, BooleanOp::Difference).unwrap_or_else(|| s.clone())).collect();
        if pieces.is_empty() {
            return t1(t[0].clone());
        }
        match reduce(ctx, pieces.clone(), BooleanOp::Union) {
            Some(x) => t1(DataTree::single(g(x))),
            None => t1(DataTree::from_list(pieces.into_iter().map(g).collect())),
        }
    });
    r.elem("Fillet Edge", "Solid", vec![geo("Solid"), num("Radius", 1.0)], &["Solid"], |_, a| og(geo::round_all_edges(&a.brep(0)?, a.num(1), false)));
    r.elem("Chamfer Edge", "Solid", vec![geo("Solid"), num("Distance", 1.0)], &["Solid"], |_, a| og(geo::round_all_edges(&a.brep(0)?, a.num(1), true)));
    r.tree("Create Solid", "Solid", vec![tree("Surfaces")], &["Solid"], |_, t| {
        let faces: Vec<Face> = geoms(&t[0]).iter().flat_map(|x| geo::faces(x).unwrap_or_default()).collect();
        if faces.is_empty() {
            return t1(DataTree::empty());
        }
        let s = geo::solid(faces);
        if !s.is_solid() {
            return t1(DataTree::empty());
        }
        t1(DataTree::single(g(s)))
    });
    r.tree("Split Solid", "Solid", vec![geo("Solid"), geo("Cutter"), pt("Direction", 0.0, 0.0, 1.0)], &["Pieces"], |ctx, t| {
        let dir = t[2].first().map(as_point).unwrap_or(DVec3::Z);
        let cutters = geoms(&t[1]);
        let mut out = Vec::new();
        for s in geoms(&t[0]) {
            for c in &cutters {
                out.extend(split(ctx, &s, c, dir).into_iter().map(g));
            }
        }
        t1(DataTree::from_list(out))
    });
    r.tree("Wire Cut", "Solid", vec![geo("Solid"), geo("Curve"), pt("Direction", 0.0, 0.0, 1.0)], &["Pieces"], |ctx, t| {
        let dir = t[2].first().map(as_point).unwrap_or(DVec3::Z);
        let cs = curves(&t[1].items());
        let mut out = Vec::new();
        for s in geoms(&t[0]) {
            for c in &cs {
                out.extend(split(ctx, &s, &Geometry::Curve(c.clone()), dir).into_iter().map(g));
            }
        }
        t1(DataTree::from_list(out))
    });
    r.tree("Split Brep", "Solid", vec![geo("Brep"), geo("Cutter")], &["Pieces"], |ctx, t| {
        // every cutter in turn splits the pieces made so far
        let cutters = geoms(&t[1]);
        let mut out: Vec<Value> = Vec::new();
        for s in geoms(&t[0]) {
            let mut pieces = vec![s];
            for c in &cutters {
                pieces = pieces.into_iter().flat_map(|p| {
                    let parts = split(ctx, &p, c, DVec3::Z);
                    if parts.len() >= 2 { parts } else { vec![p] }
                }).collect();
            }
            out.extend(pieces.into_iter().map(g));
        }
        t1(DataTree::from_list(out))
    });
}
