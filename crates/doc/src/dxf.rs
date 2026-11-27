//! DXF / DWG import and export through the cadcraft drawing database
//! (https://github.com/storytold/cadcraft, MIT OR Apache-2.0).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use cadcraft_color::{aci_rgb, Color, Rgb};
use cadcraft_doc::{Drawing, Entity, EntityKind};
use cadcraft_geom::{Spline as CSpline, Vec2, Vec3};
use glam::{DAffine3, DQuat, DVec3};
use tri_geom::{curve_ops::arc_center, NurbsCurve, Plane};

use crate::{Attributes, Document, Geometry, Layer};

fn v3(v: Vec3) -> DVec3 {
    DVec3::new(v.x, v.y, v.z)
}

fn v2(v: Vec2) -> DVec3 {
    DVec3::new(v.x, v.y, 0.0)
}

fn color_rgb(c: Color, layer: Color) -> Option<[u8; 4]> {
    match c {
        Color::ByLayer | Color::ByBlock => {
            let _ = layer;
            None
        }
        Color::Index(i) => {
            let Rgb(r, g, b) = aci_rgb(i);
            Some([r, g, b, 255])
        }
        Color::True(Rgb(r, g, b)) => Some([r, g, b, 255]),
    }
}

fn layer_rgb(c: Color) -> [u8; 4] {
    match c {
        Color::Index(i) => {
            let Rgb(r, g, b) = aci_rgb(i);
            [r, g, b, 255]
        }
        Color::True(Rgb(r, g, b)) => [r, g, b, 255],
        _ => [0, 0, 0, 255],
    }
}

/// Arc between two points with a DXF bulge (tan of a quarter of the included angle).
fn bulge_arc(a: DVec3, b: DVec3, bulge: f64) -> Option<NurbsCurve> {
    let chord = b - a;
    let len = chord.length();
    if len < 1e-12 {
        return None;
    }
    let mid = (a + b) * 0.5;
    let left = DVec3::Z.cross(chord / len);
    let sagitta = bulge * len * 0.5;
    let on_arc = mid + left * sagitta;
    NurbsCurve::arc_3pt(a, on_arc, b).ok()
}

/// Convert one entity (recursively for block inserts) into document geometry.
fn convert(d: &Drawing, e: &Entity, xf: &DAffine3, depth: usize, out: &mut Vec<(Geometry, String, Option<[u8; 4]>)>) {
    let layer_name = e.common.layer.clone();
    let layer_color = d.layer(&layer_name).map(|l| l.color).unwrap_or(Color::Index(7));
    let color = color_rgb(e.common.color, layer_color);
    let mut push = |g: Geometry| {
        let mut g = g;
        g.transform(xf);
        out.push((g, layer_name.clone(), color));
    };
    match &e.kind {
        EntityKind::Line(l) => {
            if let Ok(c) = NurbsCurve::line(v3(l.a), v3(l.b)) {
                push(Geometry::Curve(c));
            }
        }
        EntityKind::Point(p) => push(Geometry::Point(v3(p.p))),
        EntityKind::Circle(c) => {
            if let Ok(cv) = NurbsCurve::circle(&Plane { origin: v3(c.center), ..Plane::world_xy() }, c.radius) {
                push(Geometry::Curve(cv));
            }
        }
        EntityKind::Arc(a) => {
            let mut end = a.end;
            while end <= a.start {
                end += std::f64::consts::TAU;
            }
            if let Ok(cv) = NurbsCurve::arc(&Plane { origin: v3(a.center), ..Plane::world_xy() }, a.radius, a.start, end) {
                push(Geometry::Curve(cv));
            }
        }
        EntityKind::Ellipse(el) => {
            let center = v3(el.center);
            let major = v3(el.major);
            let a = major.length();
            if a < 1e-12 {
                return;
            }
            let plane = Plane::from_frame(center, major, DVec3::Z.cross(major));
            let b = a * el.ratio;
            let full = (el.end - el.start - std::f64::consts::TAU).abs() < 1e-6 || (el.end - el.start).abs() < 1e-9;
            if full {
                if let Ok(cv) = NurbsCurve::ellipse(&plane, a, b) {
                    push(Geometry::Curve(cv));
                }
            } else {
                let mut end = el.end;
                while end <= el.start {
                    end += std::f64::consts::TAU;
                }
                let n = 48;
                let pts: Vec<DVec3> = (0..=n).map(|i| {
                    let t = el.start + (end - el.start) * i as f64 / n as f64;
                    plane.point_at(a * t.cos(), b * t.sin())
                }).collect();
                if let Ok(cv) = NurbsCurve::interpolate(&pts, 3) {
                    push(Geometry::Curve(cv));
                }
            }
        }
        EntityKind::LwPolyline(pl) => {
            let n = pl.vertices.len();
            if n < 2 {
                return;
            }
            let segs = if pl.closed { n } else { n - 1 };
            let mut pieces: Vec<NurbsCurve> = Vec::new();
            for i in 0..segs {
                let va = &pl.vertices[i];
                let vb = &pl.vertices[(i + 1) % n];
                let a = v2(va.p) + DVec3::Z * pl.elevation;
                let b = v2(vb.p) + DVec3::Z * pl.elevation;
                let piece = if va.bulge.abs() > 1e-12 { bulge_arc(a, b, va.bulge) } else { NurbsCurve::line(a, b).ok() };
                if let Some(p) = piece {
                    pieces.push(p);
                }
            }
            if pieces.is_empty() {
                return;
            }
            let all_lines = pieces.iter().all(|p| p.degree == 1);
            if all_lines {
                let mut pts: Vec<DVec3> = pieces.iter().map(|p| p.start_point()).collect();
                pts.push(pieces.last().unwrap().end_point());
                if let Ok(c) = NurbsCurve::polyline(pts) {
                    push(Geometry::Curve(c));
                }
            } else {
                let mut joined = pieces[0].clone();
                for p in &pieces[1..] {
                    match joined.join_any(p) {
                        Ok(j) => joined = j,
                        Err(_) => {
                            push(Geometry::Curve(joined.clone()));
                            joined = p.clone();
                        }
                    }
                }
                push(Geometry::Curve(joined));
            }
        }
        EntityKind::Polyline3d(pl) => {
            let mut pts: Vec<DVec3> = pl.points.iter().map(|p| v3(*p)).collect();
            if pl.closed && pts.len() >= 3 {
                pts.push(pts[0]);
            }
            if let Ok(c) = NurbsCurve::polyline(pts) {
                push(Geometry::Curve(c));
            }
        }
        EntityKind::Spline(s) => {
            if let Some(c) = spline_to_curve(s) {
                push(Geometry::Curve(c));
            }
        }
        EntityKind::Insert(ins) => {
            if depth > 8 {
                return;
            }
            let Some(block) = d.blocks.get(&ins.block) else { return };
            let scale = DVec3::new(ins.scale.x, ins.scale.y, ins.scale.z);
            let local = DAffine3::from_translation(v3(ins.insert))
                * DAffine3::from_quat(DQuat::from_rotation_z(ins.rotation))
                * DAffine3::from_scale(scale)
                * DAffine3::from_translation(-v3(block.base));
            let child = *xf * local;
            for be in block.entities.iter() {
                convert(d, be, &child, depth + 1, out);
            }
        }
        _ => {}
    }
}

fn spline_to_curve(s: &CSpline) -> Option<NurbsCurve> {
    if !s.control.is_empty() && s.knots.len() == s.control.len() + s.degree + 1 {
        let cps: Vec<DVec3> = s.control.iter().map(|p| v2(*p)).collect();
        let ws = if s.weights.len() == cps.len() { s.weights.clone() } else { vec![1.0; cps.len()] };
        if let Ok(c) = NurbsCurve::new(s.degree, cps, ws, s.knots.clone()) {
            return Some(c);
        }
    }
    if !s.control.is_empty() {
        let cps: Vec<DVec3> = s.control.iter().map(|p| v2(*p)).collect();
        return NurbsCurve::from_control_points(s.degree.max(1), cps).ok();
    }
    if s.fit.len() >= 2 {
        let pts: Vec<DVec3> = s.fit.iter().map(|p| v2(*p)).collect();
        return NurbsCurve::interpolate(&pts, 3).ok();
    }
    None
}

/// Import a DXF or DWG file: model space entities become curves and points.
pub fn import(doc: &mut Document, path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let drawing = cadcraft_io::read(&bytes, &name).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut out = Vec::new();
    for e in drawing.model.iter() {
        convert(&drawing, e, &DAffine3::IDENTITY, 0, &mut out);
    }
    // layers
    let mut layer_index: HashMap<String, usize> = HashMap::new();
    for l in &drawing.layers {
        let idx = match doc.find_layer(&l.name) {
            Some(i) => i,
            None => {
                let i = doc.add_layer(l.name.clone());
                if let Some(layer) = doc.layer_mut(i) {
                    layer.color = layer_rgb(l.color);
                    layer.visible = l.on && !l.frozen;
                    layer.locked = l.locked;
                }
                i
            }
        };
        layer_index.insert(l.name.to_ascii_lowercase(), idx);
    }
    let n = out.len();
    for (g, layer_name, color) in out {
        let layer = layer_index.get(&layer_name.to_ascii_lowercase()).copied().unwrap_or(doc.current_layer());
        doc.add_with(g, Attributes { layer, color, ..Default::default() });
    }
    Ok(n)
}

fn planar_xy(pts: &[DVec3]) -> Option<f64> {
    let z = pts.first()?.z;
    pts.iter().all(|p| (p.z - z).abs() < 1e-9).then_some(z)
}

fn curve_entity(c: &NurbsCurve) -> EntityKind {
    // straight line
    if c.degree == 1 && c.control_points.len() == 2 {
        return EntityKind::Line(cadcraft_doc::Line { a: to_v3(c.control_points[0]), b: to_v3(c.control_points[1]) });
    }
    if c.degree == 1 {
        let pts = &c.control_points;
        if let Some(z) = planar_xy(pts) {
            let closed = c.is_closed();
            let verts: Vec<cadcraft_geom::PolyVertex> = pts
                .iter()
                .take(if closed { pts.len() - 1 } else { pts.len() })
                .map(|p| cadcraft_geom::PolyVertex { p: Vec2::new(p.x, p.y), bulge: 0.0, start_width: 0.0, end_width: 0.0 })
                .collect();
            return EntityKind::LwPolyline(cadcraft_doc::LwPolyline { vertices: verts, closed, const_width: 0.0, elevation: z, plinegen: false });
        }
        return EntityKind::Polyline3d(cadcraft_doc::Polyline3d { points: pts.iter().map(|p| to_v3(*p)).collect(), closed: false });
    }
    // circles and arcs in the XY plane
    if let Some(center) = arc_center(c) {
        if planar_xy(&c.control_points).is_some() {
            let r = (c.start_point() - center).length();
            if c.is_closed() {
                return EntityKind::Circle(cadcraft_doc::Circle { center: to_v3(center), radius: r });
            }
            let ang = |p: DVec3| (p.y - center.y).atan2(p.x - center.x);
            let (t0, t1) = c.domain();
            let mid = c.point_at((t0 + t1) * 0.5);
            let d0 = c.start_point() - center;
            let ccw = DVec3::Z.dot(d0.cross(mid - c.start_point())) > 0.0;
            let (s, e) = if ccw { (ang(c.start_point()), ang(c.end_point())) } else { (ang(c.end_point()), ang(c.start_point())) };
            return EntityKind::Arc(cadcraft_doc::Arc { center: to_v3(center), radius: r, start: s, end: e });
        }
    }
    if planar_xy(&c.control_points).is_some() {
        return EntityKind::Spline(CSpline {
            degree: c.degree,
            knots: c.knots.clone(),
            control: c.control_points.iter().map(|p| Vec2::new(p.x, p.y)).collect(),
            weights: if c.is_rational() { c.weights.clone() } else { Vec::new() },
            fit: Vec::new(),
            closed: c.is_closed(),
        });
    }
    let pts = c.tessellate_adaptive(1e-3 * (1.0 + c.bounding_box().diagonal()));
    EntityKind::Polyline3d(cadcraft_doc::Polyline3d { points: pts.iter().map(|p| to_v3(*p)).collect(), closed: false })
}

fn to_v3(p: DVec3) -> Vec3 {
    Vec3::new(p.x, p.y, p.z)
}

/// Export curves and points to DXF or DWG; surfaces, solids and meshes as their edges and as 3D
/// faces.
pub fn export(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let mut d = Drawing::new_metric();
    let layers: Vec<&Layer> = doc.layers().iter().collect();
    for l in &layers {
        if d.layer(&l.name).is_none() {
            let mut layer = cadcraft_doc::Layer::new(&l.name);
            layer.color = Color::True(Rgb(l.color[0], l.color[1], l.color[2]));
            layer.on = l.visible;
            layer.locked = l.locked;
            d.layers.push(layer);
        }
    }
    let mut n = 0;
    for o in doc.objects() {
        if !doc.is_visible(o) || (only_selected && !doc.is_selected(o.id)) {
            continue;
        }
        let mut kinds: Vec<EntityKind> = match &o.geometry {
            Geometry::Point(p) => vec![EntityKind::Point(cadcraft_doc::Point { p: to_v3(*p), angle: 0.0 })],
            Geometry::Curve(c) => vec![curve_entity(c)],
            Geometry::Surface(s) => s.boundary_curves().iter().map(curve_entity).collect(),
            Geometry::Brep(b) => b.edges().iter().map(curve_entity).collect(),
            Geometry::Mesh(m) => m
                .edges()
                .iter()
                .map(|(a, b)| EntityKind::Line(cadcraft_doc::Line { a: to_v3(m.vertices[*a as usize]), b: to_v3(m.vertices[*b as usize]) }))
                .collect(),
            Geometry::Instance { .. } => doc
                .instance_geometry(&o.geometry)
                .iter()
                .filter_map(|(g, _)| g.as_curve().map(curve_entity))
                .collect(),
        };
        // surfaces, solids and meshes also go out as 3D faces (triangles with their inner edges
        // hidden), or they would arrive as a wireframe with nothing to shade
        if matches!(o.geometry, Geometry::Surface(_) | Geometry::Brep(_) | Geometry::Mesh(_)) {
            if let Some(m) = crate::formats::export_mesh_of(&o.geometry) {
                for t in 0..m.triangle_count() {
                    let (a, b, c) = m.triangle(t);
                    kinds.push(EntityKind::Face3d(cadcraft_doc::Face3d { corners: [to_v3(a), to_v3(b), to_v3(c), to_v3(c)], hidden_edges: 7 }));
                }
            }
        }
        let layer_name = layers.get(o.attributes.layer).map(|l| l.name.clone()).unwrap_or_else(|| "0".into());
        for k in kinds {
            let mut e = Entity::new(d.new_handle(), k);
            e.common.layer = layer_name.clone();
            if let Some(c) = o.attributes.color {
                e.common.color = Color::True(Rgb(c[0], c[1], c[2]));
            }
            d.model.push(e);
        }
        n += 1;
    }
    let name = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "export.dxf".into());
    let bytes = cadcraft_io::write(&d, &name).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tri_geom::Plane;

    #[test]
    fn dxf_roundtrip() {
        let mut d = Document::new();
        d.add(Geometry::Curve(NurbsCurve::line(DVec3::ZERO, DVec3::new(10.0, 5.0, 0.0)).unwrap()));
        d.add(Geometry::Curve(NurbsCurve::circle(&Plane { origin: DVec3::new(3.0, 3.0, 0.0), ..Plane::world_xy() }, 2.0).unwrap()));
        d.add(Geometry::Curve(NurbsCurve::arc(&Plane::world_xy(), 4.0, 0.5, 2.0).unwrap()));
        d.add(Geometry::Curve(NurbsCurve::polyline(vec![DVec3::ZERO, DVec3::X, DVec3::new(1.0, 1.0, 0.0), DVec3::ZERO]).unwrap()));
        d.add(Geometry::Curve(NurbsCurve::interpolate(&[DVec3::ZERO, DVec3::new(1.0, 2.0, 0.0), DVec3::new(3.0, 1.0, 0.0), DVec3::new(5.0, 3.0, 0.0)], 3).unwrap()));
        d.add(Geometry::Point(DVec3::new(7.0, 7.0, 0.0)));
        let dir = std::env::temp_dir().join(format!("triceratops-dxf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.dxf");
        assert_eq!(export(&d, &p, false).unwrap(), 6);
        let mut d2 = Document::new();
        assert_eq!(import(&mut d2, &p).unwrap(), 6);
        let curves: Vec<&NurbsCurve> = d2.objects().filter_map(|o| o.geometry.as_curve()).collect();
        assert_eq!(curves.len(), 5);
        let circle = curves.iter().find(|c| c.is_closed() && c.is_rational()).expect("circle");
        assert!(((circle.point_at(0.3) - DVec3::new(3.0, 3.0, 0.0)).length() - 2.0).abs() < 1e-9);
        let arc = curves.iter().find(|c| !c.is_closed() && c.is_rational()).expect("arc");
        assert!((arc.start_point() - DVec3::new(4.0 * 0.5f64.cos(), 4.0 * 0.5f64.sin(), 0.0)).length() < 1e-9);
        assert!((arc.end_point() - DVec3::new(4.0 * 2.0f64.cos(), 4.0 * 2.0f64.sin(), 0.0)).length() < 1e-9);
        assert!((arc.length() - 4.0 * 1.5).abs() < 1e-2);
        std::fs::remove_dir_all(dir).ok();
    }
}
