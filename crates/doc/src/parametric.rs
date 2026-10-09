//! Parametric sketching inspired by CADCraft: geometric and dimensional constraints on planar
//! curves (lines, arcs, circles, polylines, points) solved by the `cadcraft-constraints`
//! damped Gauss-Newton solver, user parameters with expressions and automatic re-solving.
//!
//! Constraints live in the sketch plane (the world XY plane by default). Geometry is converted
//! to CADCraft entities in plane coordinates, solved, and written back into the document.

use std::collections::HashMap;

use cadcraft_constraints::{InferOptions, SolveOptions};
use cadcraft_doc::{Constraint, ConstraintKind, DistAxis, Drawing, EntityKind, GeomRef, Handle, Parameter, Space, Sub};
use cadcraft_geom::{Vec2, Vec3};
use glam::DVec3;
use tri_geom::{NurbsCurve, Plane};
use serde::{Deserialize, Serialize};

use crate::{Document, Geometry, ObjectId};

/// A constraint between document objects (CADCraft naming).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParamConstraint {
    pub id: u32,
    /// `Coincident`, `Horizontal`, `Aligned`, `Radius`, ... (see `kind_from_name`).
    pub kind: String,
    /// (object, sub-part: `whole`, `start`, `end`, `mid`, `center`, `v3`, `seg2`)
    pub refs: Vec<(ObjectId, String)>,
    pub name: String,
    pub expr: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Parametric {
    pub enabled: bool,
    pub constraints: Vec<ParamConstraint>,
    pub parameters: Vec<(String, String)>,
    pub plane: Plane,
    pub distance_tolerance: f64,
    pub angle_tolerance: f64,
    pub show_bars: bool,
}

impl Default for Parametric {
    fn default() -> Self {
        Self { enabled: false, constraints: Vec::new(), parameters: Vec::new(), plane: Plane::world_xy(), distance_tolerance: 0.05, angle_tolerance: 1.0, show_bars: true }
    }
}

/// Parse a constraint kind name.
pub fn kind_from_name(n: &str) -> Option<ConstraintKind> {
    Some(match n.to_ascii_lowercase().as_str() {
        "coincident" => ConstraintKind::Coincident,
        "collinear" => ConstraintKind::Collinear,
        "concentric" => ConstraintKind::Concentric,
        "fix" => ConstraintKind::Fix,
        "parallel" => ConstraintKind::Parallel,
        "perpendicular" => ConstraintKind::Perpendicular,
        "horizontal" => ConstraintKind::Horizontal,
        "vertical" => ConstraintKind::Vertical,
        "tangent" => ConstraintKind::Tangent,
        "smooth" => ConstraintKind::Smooth,
        "symmetric" => ConstraintKind::Symmetric,
        "equal" => ConstraintKind::Equal,
        "aligned" | "distance" => ConstraintKind::Distance(DistAxis::Aligned),
        "horizontaldistance" => ConstraintKind::Distance(DistAxis::Horizontal),
        "verticaldistance" => ConstraintKind::Distance(DistAxis::Vertical),
        "angular" => ConstraintKind::Angular,
        "radius" => ConstraintKind::Radius,
        "diameter" => ConstraintKind::Diameter,
        _ => return None,
    })
}

/// Result of a solve.
#[derive(Clone, Debug, Default)]
pub struct SolveOutcome {
    pub moved: Vec<ObjectId>,
    pub messages: Vec<String>,
}

struct Bridge {
    drawing: Drawing,
    handles: HashMap<ObjectId, Handle>,
    objects: HashMap<Handle, ObjectId>,
}

impl Bridge {
    fn to2(plane: &Plane, p: DVec3) -> Vec2 {
        let (u, v, _) = plane.closest_parameters(p);
        Vec2::new(u, v)
    }
    fn to3(plane: &Plane, p: Vec2) -> DVec3 {
        plane.point_at(p.x, p.y)
    }

    /// Entity for a document geometry in sketch plane coordinates (None = unsupported).
    fn entity_of(plane: &Plane, g: &Geometry) -> Option<EntityKind> {
        match g {
            Geometry::Point(p) => {
                let q = Self::to2(plane, *p);
                Some(EntityKind::Point(cadcraft_doc::Point { p: Vec3::new(q.x, q.y, 0.0), angle: 0.0 }))
            }
            Geometry::Curve(c) => Self::curve_entity(plane, c),
            _ => None,
        }
    }

    fn curve_entity(plane: &Plane, c: &NurbsCurve) -> Option<EntityKind> {
        // planarity check
        let tol = c.bounding_box().diagonal().max(1e-6) * 1e-4;
        if c.control_points.iter().any(|p| (plane.closest_parameters(*p).2).abs() > tol * 10.0 + 1e-6) {
            return None;
        }
        if c.degree == 1 && c.control_points.len() == 2 {
            let (a, b) = (Self::to2(plane, c.control_points[0]), Self::to2(plane, c.control_points[1]));
            return Some(EntityKind::Line(cadcraft_doc::Line { a: Vec3::new(a.x, a.y, 0.0), b: Vec3::new(b.x, b.y, 0.0) }));
        }
        if c.degree == 1 {
            let closed = c.is_closed();
            let n = if closed { c.control_points.len() - 1 } else { c.control_points.len() };
            let verts: Vec<cadcraft_geom::PolyVertex> = c.control_points.iter().take(n).map(|p| cadcraft_geom::PolyVertex { p: Self::to2(plane, *p), bulge: 0.0, start_width: 0.0, end_width: 0.0 }).collect();
            return Some(EntityKind::LwPolyline(cadcraft_doc::LwPolyline { vertices: verts, closed, const_width: 0.0, elevation: 0.0, plinegen: false }));
        }
        if let Some(center) = tri_geom::curve_ops::arc_center(c) {
            let r = (c.start_point() - center).length();
            let c2 = Self::to2(plane, center);
            if c.is_closed() {
                return Some(EntityKind::Circle(cadcraft_doc::Circle { center: Vec3::new(c2.x, c2.y, 0.0), radius: r }));
            }
            let ang = |p: DVec3| { let q = Self::to2(plane, p); (q.y - c2.y).atan2(q.x - c2.x) };
            let (t0, t1) = c.domain();
            let mid = c.point_at((t0 + t1) * 0.5);
            let d0 = c.start_point() - center;
            let ccw = plane.z_axis.dot(d0.cross(mid - c.start_point())) > 0.0;
            let (s, e) = if ccw { (ang(c.start_point()), ang(c.end_point())) } else { (ang(c.end_point()), ang(c.start_point())) };
            return Some(EntityKind::Arc(cadcraft_doc::Arc { center: Vec3::new(c2.x, c2.y, 0.0), radius: r, start: s, end: e }));
        }
        None
    }

    /// Document geometry for a solved entity.
    fn geometry_of(plane: &Plane, k: &EntityKind, original: &Geometry) -> Option<Geometry> {
        match k {
            EntityKind::Point(p) => Some(Geometry::Point(Self::to3(plane, Vec2::new(p.p.x, p.p.y)))),
            EntityKind::Line(l) => NurbsCurve::line(Self::to3(plane, Vec2::new(l.a.x, l.a.y)), Self::to3(plane, Vec2::new(l.b.x, l.b.y))).ok().map(Geometry::Curve),
            EntityKind::LwPolyline(pl) => {
                let mut pts: Vec<DVec3> = pl.vertices.iter().map(|v| Self::to3(plane, v.p)).collect();
                if pl.closed {
                    pts.push(pts[0]);
                }
                NurbsCurve::polyline(pts).ok().map(Geometry::Curve)
            }
            EntityKind::Circle(c) => {
                let center = Self::to3(plane, Vec2::new(c.center.x, c.center.y));
                NurbsCurve::circle(&Plane { origin: center, ..*plane }, c.radius).ok().map(Geometry::Curve)
            }
            EntityKind::Arc(a) => {
                let center = Self::to3(plane, Vec2::new(a.center.x, a.center.y));
                let mut end = a.end;
                while end <= a.start {
                    end += std::f64::consts::TAU;
                }
                let mut arc = NurbsCurve::arc(&Plane { origin: center, ..*plane }, a.radius, a.start, end).ok()?;
                // keep the original direction
                if let Geometry::Curve(oc) = original {
                    if (arc.start_point() - oc.start_point()).length() > (arc.end_point() - oc.start_point()).length() {
                        arc.reverse();
                    }
                }
                Some(Geometry::Curve(arc))
            }
            _ => None,
        }
    }

    fn build(doc: &Document, ids: &[ObjectId]) -> (Self, Vec<String>) {
        let plane = doc.state.parametric.plane;
        let mut drawing = Drawing::new_metric();
        let mut handles = HashMap::new();
        let mut objects = HashMap::new();
        let mut messages = Vec::new();
        for id in ids {
            if handles.contains_key(id) {
                continue;
            }
            let Some(o) = doc.get(*id) else { continue };
            match Self::entity_of(&plane, &o.geometry) {
                Some(kind) => {
                    if let Ok(h) = drawing.add(&Space::Model, Default::default(), kind) {
                        handles.insert(*id, h);
                        objects.insert(h, *id);
                    }
                }
                None => messages.push(format!("Object #{id} ({}) cannot be constrained: only planar lines, polylines, arcs, circles and points in the sketch plane.", o.geometry.type_name())),
            }
        }
        for (name, expr) in &doc.state.parametric.parameters {
            drawing.parametric.parameters.push(Parameter { name: name.clone(), expr: expr.clone(), description: String::new() });
        }
        drawing.parametric.settings.distance_tolerance = doc.state.parametric.distance_tolerance;
        drawing.parametric.settings.angle_tolerance = doc.state.parametric.angle_tolerance;
        for c in &doc.state.parametric.constraints {
            let Some(kind) = kind_from_name(&c.kind) else { continue };
            let refs: Option<Vec<GeomRef>> = c.refs.iter().map(|(id, sub)| handles.get(id).map(|h| GeomRef::new(*h, Sub::parse(sub).unwrap_or(Sub::Whole)))).collect();
            let Some(refs) = refs else { continue };
            drawing.constraints.push(Constraint { id: c.id, kind, refs, name: c.name.clone(), expr: c.expr.clone() });
        }
        (Self { drawing, handles, objects }, messages)
    }

    fn write_back(&self, doc: &mut Document, moved: &[Handle]) -> Vec<ObjectId> {
        let plane = doc.state.parametric.plane;
        let mut out = Vec::new();
        for h in moved {
            let Some(id) = self.objects.get(h) else { continue };
            let Some(e) = self.drawing.entity(*h) else { continue };
            let Some(original) = doc.get(*id).map(|o| o.geometry.clone()) else { continue };
            if let Some(g) = Self::geometry_of(&plane, &e.kind, &original) {
                if g != original {
                    doc.replace_geometry(*id, g);
                    out.push(*id);
                }
            }
        }
        out
    }
}

impl Document {
    fn constrained_ids(&self) -> Vec<ObjectId> {
        let mut ids: Vec<ObjectId> = self.state.parametric.constraints.iter().flat_map(|c| c.refs.iter().map(|(id, _)| *id)).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Solve all constraints, moving `keep` objects as little as possible. Returns the moved
    /// objects or an error message (nothing is changed on failure).
    pub fn solve_constraints(&mut self, keep: &[ObjectId]) -> Result<SolveOutcome, String> {
        self.purge_constraints();
        if self.state.parametric.constraints.is_empty() {
            return Ok(SolveOutcome::default());
        }
        let ids = self.constrained_ids();
        let (mut bridge, messages) = Bridge::build(self, &ids);
        let opts = SolveOptions { keep: keep.iter().filter_map(|id| bridge.handles.get(id).copied()).collect(), prefer_move: Vec::new() };
        let report = cadcraft_constraints::solve(&mut bridge.drawing, &opts).map_err(|e| e.to_string())?;
        let moved = bridge.write_back(self, &report.moved);
        let mut messages = messages;
        for (id, why) in report.skipped {
            messages.push(format!("Constraint #{id} skipped: {why}"));
        }
        Ok(SolveOutcome { moved, messages })
    }

    /// Add a constraint (validated and solved like CADCraft's GEOMCONSTRAINT / DIMCONSTRAINT).
    pub fn add_constraint(&mut self, kind: &str, refs: Vec<(ObjectId, String)>, name: &str, expr: &str) -> Result<u32, String> {
        let Some(ck) = kind_from_name(kind) else { return Err(format!("unknown constraint type {kind}")) };
        let mut ids: Vec<ObjectId> = self.constrained_ids();
        ids.extend(refs.iter().map(|(id, _)| *id));
        let (mut bridge, messages) = Bridge::build(self, &ids);
        let grefs: Option<Vec<GeomRef>> = refs.iter().map(|(id, sub)| bridge.handles.get(id).map(|h| GeomRef::new(*h, Sub::parse(sub).unwrap_or(Sub::Whole)))).collect();
        let Some(grefs) = grefs else {
            return Err(messages.first().cloned().unwrap_or_else(|| "the objects cannot be constrained".into()));
        };
        let c = Constraint { id: 0, kind: ck, refs: grefs, name: name.to_string(), expr: expr.to_string() };
        let keep: Vec<Handle> = refs.first().and_then(|(id, _)| bridge.handles.get(id).copied()).into_iter().collect();
        let id = cadcraft_constraints::add(&mut bridge.drawing, c, &SolveOptions { keep, prefer_move: Vec::new() }).map_err(|e| e.to_string())?;
        // store the constraint with the solver's name/expression and write the solved geometry back
        if let Some(solved) = bridge.drawing.constraints.iter().find(|c| c.id == id) {
            self.state.parametric.constraints.push(ParamConstraint { id, kind: solved.kind.name().to_string(), refs, name: solved.name.clone(), expr: solved.expr.clone() });
        }
        let moved: Vec<Handle> = bridge.objects.keys().copied().collect();
        bridge.write_back(self, &moved);
        self.touch_revision();
        Ok(id)
    }

    pub fn remove_constraint(&mut self, id: u32) -> bool {
        let before = self.state.parametric.constraints.len();
        self.state.parametric.constraints.retain(|c| c.id != id);
        self.state.parametric.constraints.len() != before
    }

    /// Remove constraints whose objects no longer exist.
    pub fn purge_constraints(&mut self) {
        let existing: Vec<ObjectId> = self.state.objects.keys().copied().collect();
        self.state.parametric.constraints.retain(|c| c.refs.iter().all(|(id, _)| existing.contains(id)));
    }

    /// Set a user parameter or a dimensional constraint's expression and re-solve.
    pub fn set_parameter(&mut self, name: &str, expr: &str) -> Result<SolveOutcome, String> {
        if self.state.parametric.constraints.iter().any(|c| c.name == name) {
            for c in self.state.parametric.constraints.iter_mut() {
                if c.name == name {
                    c.expr = expr.to_string();
                }
            }
        } else {
            self.state.parametric.parameters.retain(|(n, _)| n != name);
            if !expr.trim().is_empty() {
                self.state.parametric.parameters.push((name.to_string(), expr.to_string()));
            }
        }
        self.solve_constraints(&[])
    }

    /// Infer and add constraints between the given objects (AUTOCONSTRAIN).
    pub fn auto_constrain(&mut self, ids: &[ObjectId]) -> Vec<u32> {
        let mut all = self.constrained_ids();
        all.extend_from_slice(ids);
        let (mut bridge, _) = Bridge::build(self, &all);
        let handles: Vec<Handle> = ids.iter().filter_map(|id| bridge.handles.get(id).copied()).collect();
        let opts = InferOptions { distance_tolerance: self.state.parametric.distance_tolerance, angle_tolerance: self.state.parametric.angle_tolerance, types: Vec::new() };
        let added = cadcraft_constraints::autoconstrain(&mut bridge.drawing, &handles, &opts);
        for id in &added {
            if let Some(c) = bridge.drawing.constraints.iter().find(|c| c.id == *id) {
                let refs: Vec<(ObjectId, String)> = c.refs.iter().filter_map(|r| bridge.objects.get(&r.handle).map(|o| (*o, r.sub.label()))).collect();
                self.state.parametric.constraints.push(ParamConstraint { id: *id, kind: c.kind.name().to_string(), refs, name: c.name.clone(), expr: c.expr.clone() });
            }
        }
        let moved: Vec<Handle> = bridge.objects.keys().copied().collect();
        bridge.write_back(self, &moved);
        self.touch_revision();
        added
    }

    /// Parameter table rows: (name, expression, value, kind).
    pub fn parameter_rows(&self) -> Vec<(String, String, Option<f64>, String)> {
        let (bridge, _) = Bridge::build(self, &self.constrained_ids());
        cadcraft_constraints::parameter_table(&bridge.drawing).into_iter().map(|r| (r.name, r.expr, r.value, r.kind.to_string())).collect()
    }

    /// Constraint glyphs: (constraint id, label, anchor points in world space, satisfied).
    pub fn constraint_glyphs(&self) -> Vec<(u32, String, Vec<DVec3>, bool)> {
        let plane = self.state.parametric.plane;
        let (bridge, _) = Bridge::build(self, &self.constrained_ids());
        cadcraft_constraints::glyphs(&bridge.drawing)
            .into_iter()
            .map(|g| {
                let label = if g.dimensional { format!("{}={}", g.name, g.value.map(cadcraft_constraints::format_value).unwrap_or_else(|| g.expr.clone())) } else { g.kind.to_string() };
                (g.id, label, g.anchors.iter().map(|a| Bridge::to3(&plane, *a)).collect(), g.satisfied)
            })
            .collect()
    }

    /// Nearest sub-part of an object to a pick point: `start`, `end`, `mid`, `center`, `vN`.
    pub fn sub_at(&self, id: ObjectId, p: DVec3, want_point: bool) -> String {
        let Some(o) = self.get(id) else { return "whole".into() };
        match &o.geometry {
            Geometry::Point(_) => "whole".into(),
            Geometry::Curve(c) => {
                let tol = c.bounding_box().diagonal().max(1e-6) * 0.15;
                if c.degree == 1 && c.control_points.len() > 2 {
                    let n = if c.is_closed() { c.control_points.len() - 1 } else { c.control_points.len() };
                    let (vi, dv) = (0..n).map(|i| (i, (c.control_points[i] - p).length())).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
                    if want_point || dv < tol {
                        return format!("v{vi}");
                    }
                    let (si, _) = (0..if c.is_closed() { n } else { n - 1 }).map(|i| { let a = c.control_points[i]; let b = c.control_points[(i + 1) % n]; let ab = b - a; let t = ((p - a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0); (i, (a + ab * t - p).length()) }).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
                    return format!("seg{si}");
                }
                if tri_geom::curve_ops::arc_center(c).is_some() && c.degree > 1 {
                    if c.is_closed() {
                        return if want_point { "center".into() } else { "whole".into() };
                    }
                    let ds = (c.start_point() - p).length();
                    let de = (c.end_point() - p).length();
                    if want_point {
                        return if ds < de { "start".into() } else { "end".into() };
                    }
                    return if ds < tol { "start".into() } else if de < tol { "end".into() } else { "whole".into() };
                }
                let ds = (c.start_point() - p).length();
                let de = (c.end_point() - p).length();
                let (t0, t1) = c.domain();
                let dm = (c.point_at((t0 + t1) * 0.5) - p).length();
                if want_point {
                    if dm < ds && dm < de { "mid".into() } else if ds < de { "start".into() } else { "end".into() }
                } else if ds < tol {
                    "start".into()
                } else if de < tol {
                    "end".into()
                } else {
                    "whole".into()
                }
            }
            _ => "whole".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_and_distance_constraints_solve() {
        let mut d = Document::new();
        let a = d.add(Geometry::Curve(NurbsCurve::line(DVec3::ZERO, DVec3::new(10.0, 2.0, 0.0)).unwrap()));
        let id = d.add_constraint("Horizontal", vec![(a, "whole".into())], "", "").expect("horizontal");
        assert!(id > 0);
        let c = d.get(a).unwrap().geometry.as_curve().unwrap().clone();
        assert!((c.start_point().y - c.end_point().y).abs() < 1e-6, "{:?} {:?}", c.start_point(), c.end_point());
        let _ = d.add_constraint("Aligned", vec![(a, "start".into()), (a, "end".into())], "", "20").expect("distance");
        let c = d.get(a).unwrap().geometry.as_curve().unwrap().clone();
        assert!((c.length() - 20.0).abs() < 1e-4, "length {}", c.length());
        // parameter change re-solves
        let rows = d.parameter_rows();
        let name = rows.iter().find(|r| r.3 == "dimensional").map(|r| r.0.clone()).unwrap();
        d.set_parameter(&name, "30").expect("re-solve");
        let c = d.get(a).unwrap().geometry.as_curve().unwrap().clone();
        assert!((c.length() - 30.0).abs() < 1e-4, "length {}", c.length());
    }

    #[test]
    fn coincident_and_radius() {
        let mut d = Document::new();
        let l = d.add(Geometry::Curve(NurbsCurve::line(DVec3::ZERO, DVec3::new(10.0, 0.0, 0.0)).unwrap()));
        let circ = d.add(Geometry::Curve(NurbsCurve::circle(&Plane { origin: DVec3::new(12.0, 1.0, 0.0), ..Plane::world_xy() }, 3.0).unwrap()));
        d.add_constraint("Coincident", vec![(l, "end".into()), (circ, "center".into())], "", "").expect("coincident");
        let c = d.get(circ).unwrap().geometry.as_curve().unwrap().clone();
        let l2 = d.get(l).unwrap().geometry.as_curve().unwrap().clone();
        let center = tri_geom::curve_ops::arc_center(&c).unwrap();
        assert!((center - l2.end_point()).length() < 1e-6);
        d.add_constraint("Radius", vec![(circ, "whole".into())], "", "5").expect("radius");
        let c = d.get(circ).unwrap().geometry.as_curve().unwrap().clone();
        let center = tri_geom::curve_ops::arc_center(&c).unwrap();
        assert!(((c.start_point() - center).length() - 5.0).abs() < 1e-6);
        assert_eq!(d.state.parametric.constraints.len(), 2);
        assert!(!d.constraint_glyphs().is_empty());
        let added = d.auto_constrain(&[l]);
        let _ = added;
    }
}
