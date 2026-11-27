//! Native Triceratops `.3dm` files through openNURBS (crate `on3dm`).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, Result};
use glam::{DAffine3, DVec2, DVec3};
use tri_geom::{Brep, Face, Mesh, NurbsCurve, NurbsSurface};

use crate::{Attributes, BlockDef, Document, Geometry, Layer, Settings, State};

fn units_name(code: i32) -> &'static str {
    match code {
        on3dm::UNITS_MILLIMETERS => "Millimeters",
        on3dm::UNITS_CENTIMETERS => "Centimeters",
        on3dm::UNITS_METERS => "Meters",
        on3dm::UNITS_INCHES => "Inches",
        on3dm::UNITS_FEET => "Feet",
        _ => "Millimeters",
    }
}

fn units_code(name: &str) -> i32 {
    match name.to_ascii_lowercase().as_str() {
        "centimeters" => on3dm::UNITS_CENTIMETERS,
        "meters" => on3dm::UNITS_METERS,
        "inches" => on3dm::UNITS_INCHES,
        "feet" => on3dm::UNITS_FEET,
        _ => on3dm::UNITS_MILLIMETERS,
    }
}

fn to_curve(c: &on3dm::Curve) -> Option<NurbsCurve> {
    NurbsCurve::new(c.degree, c.control_points.iter().map(|p| DVec3::from_array(*p)).collect(), c.weights.clone(), c.knots.clone()).ok()
}

fn to_surface(s: &on3dm::Surface) -> Option<NurbsSurface> {
    NurbsSurface::new(
        s.degree_u,
        s.degree_v,
        s.count_u,
        s.count_v,
        s.control_points.iter().map(|p| DVec3::from_array(*p)).collect(),
        s.weights.clone(),
        s.knots_u.clone(),
        s.knots_v.clone(),
    )
    .ok()
}

fn xform_to_affine(x: &[f64; 16]) -> DAffine3 {
    let m = glam::DMat3::from_cols(DVec3::new(x[0], x[4], x[8]), DVec3::new(x[1], x[5], x[9]), DVec3::new(x[2], x[6], x[10]));
    DAffine3::from_mat3_translation(m, DVec3::new(x[3], x[7], x[11]))
}

fn affine_to_xform(a: &DAffine3) -> [f64; 16] {
    let m = a.matrix3;
    let t = a.translation;
    [m.x_axis.x, m.y_axis.x, m.z_axis.x, t.x, m.x_axis.y, m.y_axis.y, m.z_axis.y, t.y, m.x_axis.z, m.y_axis.z, m.z_axis.z, t.z, 0.0, 0.0, 0.0, 1.0]
}

fn to_geometry(g: &on3dm::Geometry) -> Option<Geometry> {
    Some(match g {
        on3dm::Geometry::Instance { block, xform } => Geometry::Instance { block: *block, xf: xform_to_affine(xform) },
        on3dm::Geometry::Point(p) => Geometry::Point(DVec3::from_array(*p)),
        on3dm::Geometry::Curve(c) => Geometry::Curve(to_curve(c)?),
        on3dm::Geometry::Surface(s) => Geometry::Surface(to_surface(s)?),
        on3dm::Geometry::Brep { faces, solid } => {
            let mut out = Vec::new();
            for f in faces {
                let surface = to_surface(&f.surface)?;
                let loops: Vec<Vec<DVec2>> = f.loops.iter().map(|l| l.iter().map(|p| DVec2::new(p[0], p[1])).collect()).collect();
                out.push(Face { surface, loops });
            }
            if out.len() == 1 && !out[0].is_trimmed() {
                Geometry::Surface(out.pop().unwrap().surface)
            } else {
                let mut b = Brep { faces: out, closed: *solid, exact: None };
                if !*solid {
                    b.closed = b.compute_closed();
                }
                Geometry::Brep(b)
            }
        }
        on3dm::Geometry::Mesh { vertices, triangles } => {
            let mut m = Mesh::new();
            m.vertices = vertices.iter().map(|v| DVec3::from_array(*v)).collect();
            m.indices = triangles.iter().flat_map(|t| *t).collect();
            m.compute_normals();
            Geometry::Mesh(m)
        }
    })
}

/// Load a `.3dm` file as a document state.
pub fn load(path: &Path) -> Result<(State, Settings, usize)> {
    let model = on3dm::read(path).map_err(|e| anyhow!("{e}"))?;
    let mut doc = Document::new();
    doc.state.layers.clear();
    let mut layer_map: HashMap<i32, usize> = HashMap::new();
    for l in &model.layers {
        let idx = doc.state.layers.len();
        doc.state.layers.push(Layer { name: l.name.clone(), color: [l.color[0], l.color[1], l.color[2], 255], visible: l.visible, locked: l.locked, ..Default::default() });
        layer_map.insert(l.index, idx);
    }
    if doc.state.layers.is_empty() {
        doc.state.layers.push(Layer::new("Default", [0, 0, 0, 255]));
    }
    let mut skipped = model.unsupported;
    let attrs_of = |o: &on3dm::Object| Attributes {
        layer: layer_map.get(&o.layer).copied().unwrap_or(0),
        name: if o.name.is_empty() { None } else { Some(o.name.clone()) },
        color: o.color.map(|c| [c[0], c[1], c[2], 255]),
        hidden: o.hidden,
        locked: o.locked,
        groups: Vec::new(),
        user_text: Vec::new(),
        draw_order: 0,
        linetype: None,
        material: None,
        mapping: None,
    };
    for b in &model.blocks {
        let objects: Vec<(Geometry, Attributes)> = b.objects.iter().filter_map(|o| to_geometry(&o.geometry).map(|g| (g, attrs_of(o)))).collect();
        doc.state.blocks.push(BlockDef { name: b.name.clone(), base: DVec3::ZERO, objects });
    }
    for o in &model.objects {
        let Some(g) = to_geometry(&o.geometry) else {
            skipped += 1;
            continue;
        };
        let attrs = attrs_of(o);
        doc.add_with(g, attrs);
    }
    let settings = Settings { units: units_name(model.unit_system).into(), absolute_tolerance: model.absolute_tolerance, notes: model.notes.clone(), ..Settings::default() };
    Ok((doc.state, settings, skipped))
}

fn from_curve(c: &NurbsCurve) -> on3dm::Curve {
    on3dm::Curve { degree: c.degree, control_points: c.control_points.iter().map(|p| p.to_array()).collect(), weights: c.weights.clone(), knots: c.knots.clone() }
}

fn from_surface(s: &NurbsSurface) -> on3dm::Surface {
    on3dm::Surface {
        degree_u: s.degree_u,
        degree_v: s.degree_v,
        count_u: s.count_u,
        count_v: s.count_v,
        control_points: s.control_points.iter().map(|p| p.to_array()).collect(),
        weights: s.weights.clone(),
        knots_u: s.knots_u.clone(),
        knots_v: s.knots_v.clone(),
    }
}

fn from_geometry(g: &Geometry) -> on3dm::Geometry {
    match g {
        Geometry::Instance { block, xf } => on3dm::Geometry::Instance { block: *block, xform: affine_to_xform(xf) },
        Geometry::Point(p) => on3dm::Geometry::Point(p.to_array()),
        Geometry::Curve(c) => on3dm::Geometry::Curve(from_curve(c)),
        Geometry::Surface(s) => on3dm::Geometry::Surface(from_surface(s)),
        Geometry::Brep(b) => on3dm::Geometry::Brep {
            faces: b
                .faces
                .iter()
                .map(|f| on3dm::Face { surface: from_surface(&f.surface), loops: f.loops.iter().map(|l| l.iter().map(|p| [p.x, p.y]).collect()).collect() })
                .collect(),
            solid: b.closed,
        },
        Geometry::Mesh(m) => on3dm::Geometry::Mesh { vertices: m.vertices.iter().map(|v| v.to_array()).collect(), triangles: m.indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect() },
    }
}

/// Save the document as a `.3dm` file. Returns the number of objects written.
pub fn save(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let layers: Vec<on3dm::Layer> = doc
        .layers()
        .iter()
        .enumerate()
        .map(|(i, l)| on3dm::Layer { index: i as i32, name: l.name.clone(), color: [l.color[0], l.color[1], l.color[2]], visible: l.visible, locked: l.locked })
        .collect();
    let to_obj = |g: &Geometry, a: &Attributes| on3dm::Object {
        geometry: from_geometry(g),
        layer: a.layer as i32,
        name: a.name.clone().unwrap_or_default(),
        color: a.color.map(|c| [c[0], c[1], c[2]]),
        hidden: a.hidden,
        locked: a.locked,
    };
    let blocks: Vec<on3dm::BlockDef> = doc.state.blocks.iter().map(|b| on3dm::BlockDef { name: b.name.clone(), objects: b.objects.iter().map(|(g, a)| to_obj(g, a)).collect() }).collect();
    let objects: Vec<on3dm::Object> = doc.objects().filter(|o| !only_selected || doc.is_selected(o.id)).map(|o| to_obj(&o.geometry, &o.attributes)).collect();
    let model = on3dm::Model { layers, objects, blocks, unit_system: units_code(&doc.settings.units), absolute_tolerance: doc.settings.absolute_tolerance, notes: doc.settings.notes.clone(), unsupported: 0 };
    on3dm::write(&model, path).map_err(|e| anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tri_geom::Plane;

    #[test]
    fn roundtrip_document() {
        let mut d = Document::new();
        d.add(Geometry::Curve(NurbsCurve::circle(&Plane::world_xy(), 3.0).unwrap()));
        d.add(Geometry::Brep(Brep::cuboid(&Plane::world_xy(), 1.0, 2.0, 3.0).unwrap()));
        d.add(Geometry::Surface(NurbsSurface::sphere(DVec3::ZERO, 1.0).unwrap()));
        let circle = NurbsCurve::circle(&Plane::world_xy(), 1.0).unwrap();
        let cap = Face::from_planar_curves(std::slice::from_ref(&circle), 1e-3).unwrap();
        let side = NurbsSurface::extrude(&circle, DVec3::Z * 2.0).unwrap();
        let mut top = cap.clone();
        top.transform(&glam::DAffine3::from_translation(DVec3::Z * 2.0));
        d.add(Geometry::Brep(Brep::new(vec![Face::untrimmed(side), cap, top])));
        d.settings.notes = "note".into();
        let bi = d.add_block(BlockDef { name: "Bolt".into(), base: DVec3::ZERO, objects: vec![(Geometry::Point(DVec3::new(1.0, 0.0, 0.0)), Attributes::default())] });
        d.add(Geometry::Instance { block: bi, xf: glam::DAffine3::from_translation(DVec3::new(5.0, 6.0, 7.0)) });
        let dir = std::env::temp_dir().join(format!("triceratops-3dm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.3dm");
        assert_eq!(save(&d, &p, false).unwrap(), 5);
        let (state, settings, skipped) = load(&p).unwrap();
        assert_eq!(skipped, 0);
        assert_eq!(settings.notes, "note");
        assert_eq!(state.objects.len(), 5);
        assert_eq!(state.blocks.len(), 1);
        assert_eq!(state.blocks[0].name, "Bolt");
        let inst = state.objects.values().find_map(|o| if let Geometry::Instance { block, xf } = &o.geometry { Some((*block, *xf)) } else { None }).expect("instance");
        assert_eq!(inst.0, 0);
        assert!((inst.1.translation - DVec3::new(5.0, 6.0, 7.0)).length() < 1e-9);
        let box_closed = state.objects.values().any(|o| matches!(&o.geometry, Geometry::Brep(b) if b.faces.len() == 6 && b.closed));
        assert!(box_closed, "box should come back as a closed polysurface");
        let cyl = state.objects.values().find(|o| matches!(&o.geometry, Geometry::Brep(b) if b.faces.len() == 3)).expect("capped cylinder");
        if let Geometry::Brep(b) = &cyl.geometry {
            assert!(b.faces.iter().any(|f| f.is_trimmed()), "caps keep their trims");
            assert!(b.closed);
        }
        std::fs::remove_dir_all(dir).ok();
    }

    /// A flat, wide capped cylinder saved as an extrusion: it must load as a closed
    /// polysurface, and its cap loops must follow the circle closely enough that the
    /// kernel's own edge matching (compute_closed) also sees it as closed.
    #[test]
    fn capped_extrusion_loads_as_closed_polysurface() {
        let dir = std::env::temp_dir().join(format!("tri-io3dm-ext-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("ext.3dm");
        let (r, h) = (100.0, 1.0);
        assert!(on3dm::write_test_extrusion(&p, r, h));
        let (state, _, skipped) = load(&p).unwrap();
        assert_eq!(skipped, 0);
        let mut d = Document::new();
        d.state = state;
        let objs: Vec<_> = d.objects().collect();
        assert_eq!(objs.len(), 1);
        let Geometry::Brep(b) = &objs[0].geometry else { panic!("extrusion did not load as a polysurface") };
        assert!(b.closed, "solid flag from the file");
        assert!(b.compute_closed(), "edges of caps and wall must match");
        // every chord of a cap loop stays near the circle (32 samples would sag by ~0.5)
        let diag = b.bounding_box().diagonal();
        for f in b.faces.iter().filter(|f| f.is_trimmed()) {
            for l in &f.loops {
                for k in 0..l.len() {
                    let a = f.surface.point_at(l[k].x, l[k].y);
                    let c = f.surface.point_at(l[(k + 1) % l.len()].x, l[(k + 1) % l.len()].y);
                    let mid = (a + c) * 0.5;
                    let sag = r - DVec3::new(mid.x, mid.y, 0.0).length();
                    assert!(sag < diag * 2e-4, "chord sag {sag} on a loop of {} points", l.len());
                }
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
