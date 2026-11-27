//! Native JSON file format (`.tri`) and OBJ import/export.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::DVec3;
use tri_geom::{Mesh, NurbsCurve};
use serde::{Deserialize, Serialize};

use crate::{Document, Geometry, Object, Settings, State};

pub const FILE_VERSION: u32 = 1;
pub const NATIVE_EXTENSION: &str = "tri";

#[derive(Serialize, Deserialize)]
struct FileData {
    format: String,
    version: u32,
    settings: Settings,
    state: State,
}

pub fn save(doc: &Document, path: &Path) -> Result<()> {
    let data = FileData {
        format: "triceratops".into(),
        version: FILE_VERSION,
        settings: doc.settings.clone(),
        state: doc.state.clone(),
    };
    let json = serde_json::to_string(&data)?;
    std::fs::write(path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn load(path: &Path) -> Result<(State, Settings)> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let data: FileData = serde_json::from_str(&text).context("parsing document")?;
    if data.format != "triceratops" {
        bail!("not a triceratops document");
    }
    if data.version > FILE_VERSION {
        bail!("document version {} is newer than supported {}", data.version, FILE_VERSION);
    }
    Ok((data.state, data.settings))
}

/// Export visible objects to Wavefront OBJ. Surfaces are tessellated,
/// curves become `l` polylines.
pub fn export_obj(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let mut out = String::new();
    writeln!(out, "# exported by triceratops")?;
    let mut v_base = 1usize;
    let mut count = 0usize;
    for o in doc.objects() {
        if !doc.is_visible(o) || (only_selected && !doc.is_selected(o.id)) {
            continue;
        }
        let name = o.attributes.name.clone().unwrap_or_else(|| format!("{}_{}", o.geometry.kind(), o.id));
        match &o.geometry {
            Geometry::Point(p) => {
                writeln!(out, "o {name}")?;
                writeln!(out, "v {} {} {}", p.x, p.y, p.z)?;
                writeln!(out, "p {v_base}")?;
                v_base += 1;
            }
            Geometry::Curve(c) => {
                let pts = c.tessellate_adaptive(doc.settings.absolute_tolerance.max(1e-4));
                writeln!(out, "o {name}")?;
                for p in &pts {
                    writeln!(out, "v {} {} {}", p.x, p.y, p.z)?;
                }
                write!(out, "l")?;
                for i in 0..pts.len() {
                    write!(out, " {}", v_base + i)?;
                }
                writeln!(out)?;
                v_base += pts.len();
            }
            Geometry::Instance { .. } => {
                for (g, _) in doc.instance_geometry(&o.geometry) {
                    if let Geometry::Mesh(m) = g {
                        write_mesh(&mut out, &name, &m, &mut v_base)?;
                    }
                }
            }
            Geometry::Surface(_) | Geometry::Brep(_) | Geometry::Mesh(_) => {
                let mesh = match &o.geometry {
                    Geometry::Surface(s) => s.tessellate(8, 8),
                    Geometry::Brep(b) => b.tessellate(8, 8),
                    Geometry::Mesh(m) => m.clone(),
                    _ => unreachable!(),
                };
                write_mesh(&mut out, &name, &mesh, &mut v_base)?;
            }
        }
        count += 1;
    }
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok(count)
}

fn write_mesh(out: &mut String, name: &str, mesh: &Mesh, v_base: &mut usize) -> Result<()> {
    writeln!(out, "o {name}")?;
    for p in &mesh.vertices {
        writeln!(out, "v {} {} {}", p.x, p.y, p.z)?;
    }
    for n in &mesh.normals {
        writeln!(out, "vn {} {} {}", n.x, n.y, n.z)?;
    }
    for t in mesh.indices.chunks(3) {
        let a = *v_base + t[0] as usize;
        let b = *v_base + t[1] as usize;
        let c = *v_base + t[2] as usize;
        writeln!(out, "f {a}//{a} {b}//{b} {c}//{c}")?;
    }
    *v_base += mesh.vertices.len();
    Ok(())
}

/// Import an OBJ file: faces become meshes (one per `o` group), lines become polylines.
pub fn import_obj(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut vertices: Vec<DVec3> = Vec::new();
    // per group: triangles and the original polygons
    type ObjGroup = (Vec<[usize; 3]>, Vec<Vec<usize>>);
    let mut groups: BTreeMap<String, ObjGroup> = BTreeMap::new();
    let mut current = "default".to_string();
    let parse_index = |s: &str, n: usize| -> Option<usize> {
        let first = s.split('/').next()?;
        let i: i64 = first.parse().ok()?;
        if i < 0 {
            Some((n as i64 + i) as usize)
        } else {
            Some((i - 1) as usize)
        }
    };
    for line in text.lines() {
        let mut it = line.split_whitespace();
        match it.next() {
            Some("v") => {
                let x: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let y: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let z: f64 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                vertices.push(DVec3::new(x, y, z));
            }
            Some("o") | Some("g") => {
                current = it.collect::<Vec<_>>().join(" ");
                if current.is_empty() {
                    current = "default".into();
                }
            }
            Some("f") => {
                let idx: Vec<usize> = it.filter_map(|s| parse_index(s, vertices.len())).collect();
                let g = groups.entry(current.clone()).or_default();
                for k in 1..idx.len().saturating_sub(1) {
                    g.0.push([idx[0], idx[k], idx[k + 1]]);
                }
            }
            Some("l") => {
                let idx: Vec<usize> = it.filter_map(|s| parse_index(s, vertices.len())).collect();
                groups.entry(current.clone()).or_default().1.push(idx);
            }
            _ => {}
        }
    }
    let mut count = 0;
    for (name, (tris, lines)) in groups {
        if !tris.is_empty() {
            let mut remap: BTreeMap<usize, u32> = BTreeMap::new();
            let mut mesh = Mesh::new();
            for t in &tris {
                for &vi in t {
                    if vi >= vertices.len() {
                        continue;
                    }
                    let next = mesh.vertices.len() as u32;
                    let id = *remap.entry(vi).or_insert_with(|| {
                        mesh.vertices.push(vertices[vi]);
                        next
                    });
                    mesh.indices.push(id);
                }
            }
            if !mesh.indices.len().is_multiple_of(3) {
                mesh.indices.truncate(mesh.indices.len() / 3 * 3);
            }
            mesh.compute_normals();
            let id = doc.add(Geometry::Mesh(mesh));
            doc.get_mut(id).unwrap().attributes.name = Some(name.clone());
            count += 1;
        }
        for l in lines {
            let pts: Vec<DVec3> = l.iter().filter(|&&i| i < vertices.len()).map(|&i| vertices[i]).collect();
            if let Ok(c) = NurbsCurve::polyline(pts) {
                let id = doc.add(Geometry::Curve(c));
                doc.get_mut(id).unwrap().attributes.name = Some(name.clone());
                count += 1;
            }
        }
    }
    Ok(count)
}

impl Object {
    /// Short type description, as in the Properties panel.
    pub fn describe(&self) -> String {
        self.geometry.type_name()
    }

    /// Detailed description, as printed by the What command.
    pub fn describe_long(&self) -> String {
        match &self.geometry {
            Geometry::Point(p) => format!("point ({:.3}, {:.3}, {:.3})", p.x, p.y, p.z),
            Geometry::Curve(c) => format!(
                "{} NURBS curve, degree {}, {} control points{}",
                if c.is_rational() { "rational" } else { "non-rational" },
                c.degree,
                c.control_points.len(),
                if c.is_closed() { ", closed" } else { ", open" }
            ),
            Geometry::Surface(s) => format!(
                "{} NURBS surface, degree {}x{}, {}x{} control points",
                self.geometry.type_name(),
                s.degree_u,
                s.degree_v,
                s.count_u,
                s.count_v
            ),
            Geometry::Brep(b) => format!("{} with {} faces ({} trimmed)", self.geometry.type_name(), b.faces.len(), b.faces.iter().filter(|f| f.is_trimmed()).count()),
            Geometry::Mesh(m) => format!("{}, {} vertices, {} faces", self.geometry.type_name(), m.vertices.len(), m.triangle_count()),
            Geometry::Instance { block, .. } => format!("block instance (definition #{block})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tri_geom::{NurbsSurface, Plane};

    #[test]
    fn save_load_roundtrip() {
        let mut d = Document::new();
        d.add(Geometry::Curve(NurbsCurve::circle(&Plane::world_xy(), 2.0).unwrap()));
        d.add(Geometry::Surface(NurbsSurface::sphere(DVec3::ZERO, 1.0).unwrap()));
        let dir = std::env::temp_dir().join(format!("triceratops-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.tri");
        save(&d, &p).unwrap();
        let (state, _) = load(&p).unwrap();
        assert_eq!(state, d.state);
        let obj = dir.join("t.obj");
        assert_eq!(export_obj(&d, &obj, false).unwrap(), 2);
        let mut d2 = Document::new();
        assert_eq!(import_obj(&mut d2, &obj).unwrap(), 2);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn volume_of_prefers_the_exact_volume_and_ignores_a_stale_one() {
        let rec = std::sync::Arc::new(solidcore_record::brep::Brep {
            version: 4,
            tolerance: 1e-6,
            regions: Vec::new(),
            shells: Vec::new(),
            faces: Vec::new(),
            edges: Vec::new(),
            vertices: Vec::new(),
        });
        let mut b = tri_geom::Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        b.attach_exact_with_volume(rec, Some(24.5));
        let (v, c) = Document::volume_of(&Geometry::Brep(b.clone())).unwrap();
        assert!((v - 24.5).abs() < 1e-12, "{v}");
        assert!((c - DVec3::new(1.0, 1.5, 2.0)).length() < 1e-6, "the centroid still comes from the mesh: {c:?}");
        b.faces[0].surface.control_points[0].x += 0.1; // stale now: the mesh volume is used again
        let (v2, _) = Document::volume_of(&Geometry::Brep(b)).unwrap();
        assert!((v2 - 24.5).abs() > 0.01 && (v2 - 24.0).abs() < 0.5, "{v2}");
    }

    #[test]
    fn exact_body_survives_save_and_load_and_old_files_still_open() {
        let mut brep = tri_geom::Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        brep.attach_exact(std::sync::Arc::new(solidcore_record::brep::Brep {
            version: 4,
            tolerance: 1e-6,
            regions: Vec::new(),
            shells: Vec::new(),
            faces: Vec::new(),
            edges: Vec::new(),
            vertices: Vec::new(),
        }));
        let mut d = Document::new();
        let id = d.add(Geometry::Brep(brep));
        let dir = std::env::temp_dir().join(format!("triceratops-exact-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("e.tri");
        save(&d, &p).unwrap();
        let (state, _) = load(&p).unwrap();
        match &state.objects[&id].geometry {
            Geometry::Brep(b) => assert!(b.exact_valid().is_some()),
            other => panic!("not a brep: {other:?}"),
        }
        // the same file written by a version that had no `exact` field
        let text = std::fs::read_to_string(&p).unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        fn strip(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(m) => {
                    m.remove("exact");
                    m.values_mut().for_each(strip);
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
                _ => {}
            }
        }
        strip(&mut v);
        let old = dir.join("old.tri");
        std::fs::write(&old, serde_json::to_string(&v).unwrap()).unwrap();
        let (state, _) = load(&old).unwrap();
        match &state.objects[&id].geometry {
            Geometry::Brep(b) => assert!(b.exact.is_none() && b.faces.len() == 6),
            other => panic!("not a brep: {other:?}"),
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
