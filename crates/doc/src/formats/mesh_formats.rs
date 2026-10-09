//! Mesh file formats: STL, PLY, OFF, 3DS, glTF/GLB, COLLADA, VRML, X3D, DirectX, AMF, 3MF, GTS,
//! RAW, FBX (ASCII), POV-Ray, RenderMan and SLC slices.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::DVec3;
use tri_geom::{Mesh, Plane};

use super::{add_mesh, add_polyline, collect_meshes, mesh_from_polygons, parse_floats, parse_ints, read_text, write_text, xml_escape, zip_read, zip_write};
use crate::Document;

fn file_stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mesh".into())
}

fn f3(p: DVec3) -> String {
    format!("{} {} {}", fmt(p.x), fmt(p.y), fmt(p.z))
}

fn fmt(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}", v as i64) } else { format!("{v}") }
}

// ---------------------------------------------------------------- STL

pub fn export_stl(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    // binary STL
    let tris: usize = meshes.iter().map(|m| m.mesh.triangle_count()).sum();
    let mut out: Vec<u8> = Vec::with_capacity(84 + tris * 50);
    let mut header = b"Triceratops binary STL".to_vec();
    header.resize(80, 0);
    out.extend_from_slice(&header);
    out.extend_from_slice(&(tris as u32).to_le_bytes());
    for m in &meshes {
        for t in 0..m.mesh.triangle_count() {
            let (a, b, c) = m.mesh.triangle(t);
            let n = (b - a).cross(c - a).normalize_or(DVec3::Z);
            for p in [n, a, b, c] {
                for v in [p.x, p.y, p.z] {
                    out.extend_from_slice(&(v as f32).to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
    }
    std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok(meshes.len())
}

pub fn import_stl(doc: &mut Document, path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let is_ascii = bytes.len() >= 5 && bytes.starts_with(b"solid") && !(bytes.len() >= 84 && { let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize; 84 + n * 50 == bytes.len() });
    let mut m = Mesh::new();
    if is_ascii {
        let text = String::from_utf8_lossy(&bytes);
        let mut tri: Vec<DVec3> = Vec::new();
        for line in text.lines() {
            let l = line.trim();
            if let Some(rest) = l.strip_prefix("vertex") {
                let f = parse_floats(rest);
                if f.len() >= 3 {
                    tri.push(DVec3::new(f[0], f[1], f[2]));
                }
                if tri.len() == 3 {
                    let b = m.vertices.len() as u32;
                    m.vertices.extend_from_slice(&tri);
                    m.indices.extend_from_slice(&[b, b + 1, b + 2]);
                    tri.clear();
                }
            }
        }
    } else {
        if bytes.len() < 84 {
            bail!("STL file is too short");
        }
        let n = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        let f = |i: usize| f32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as f64;
        for t in 0..n {
            let base = 84 + t * 50;
            if base + 50 > bytes.len() {
                break;
            }
            let b = m.vertices.len() as u32;
            for k in 0..3 {
                let o = base + 12 + k * 12;
                m.vertices.push(DVec3::new(f(o), f(o + 4), f(o + 8)));
            }
            m.indices.extend_from_slice(&[b, b + 1, b + 2]);
        }
    }
    let welded = weld(&m);
    Ok(add_mesh(doc, &file_stem(path), welded, None) as usize)
}

/// Merge coincident vertices (file formats that repeat vertices per triangle).
pub fn weld(m: &Mesh) -> Mesh {
    let diag = m.bounding_box().diagonal().max(1e-9);
    let q = diag * 1e-7;
    let mut map: std::collections::HashMap<(i64, i64, i64), u32> = std::collections::HashMap::new();
    let mut out = Mesh::new();
    let mut remap = Vec::with_capacity(m.vertices.len());
    for v in &m.vertices {
        let key = ((v.x / q).round() as i64, (v.y / q).round() as i64, (v.z / q).round() as i64);
        let id = *map.entry(key).or_insert_with(|| {
            out.vertices.push(*v);
            out.vertices.len() as u32 - 1
        });
        remap.push(id);
    }
    for t in m.indices.chunks(3) {
        let (a, b, c) = (remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
        if a != b && b != c && a != c {
            out.indices.extend_from_slice(&[a, b, c]);
        }
    }
    out.compute_normals();
    out
}

// ---------------------------------------------------------------- PLY

pub fn export_ply(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let nv: usize = meshes.iter().map(|m| m.mesh.vertices.len()).sum();
    let nf: usize = meshes.iter().map(|m| m.mesh.triangle_count()).sum();
    let mut s = String::new();
    writeln!(s, "ply\nformat ascii 1.0\ncomment Triceratops\nelement vertex {nv}\nproperty float x\nproperty float y\nproperty float z\nproperty uchar red\nproperty uchar green\nproperty uchar blue\nelement face {nf}\nproperty list uchar int vertex_indices\nend_header")?;
    let mut base = 0usize;
    for m in &meshes {
        for v in &m.mesh.vertices {
            writeln!(s, "{} {} {} {}", fmt(v.x), fmt(v.y), fmt(v.z), format!("{} {} {}", m.color[0], m.color[1], m.color[2]))?;
        }
        let _ = base;
        base += m.mesh.vertices.len();
    }
    let mut base = 0usize;
    for m in &meshes {
        for t in m.mesh.indices.chunks(3) {
            writeln!(s, "3 {} {} {}", t[0] as usize + base, t[1] as usize + base, t[2] as usize + base)?;
        }
        base += m.mesh.vertices.len();
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_ply(doc: &mut Document, path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let header_end = bytes.windows(10).position(|w| w == b"end_header").ok_or_else(|| anyhow::anyhow!("PLY header not found"))? + 10;
    let header = String::from_utf8_lossy(&bytes[..header_end]).to_string();
    let mut body_start = header_end;
    while body_start < bytes.len() && (bytes[body_start] == b'\r' || bytes[body_start] == b'\n') {
        body_start += 1;
    }
    let mut format = "ascii".to_string();
    let mut nv = 0usize;
    let mut nf = 0usize;
    let mut vprops: Vec<(String, String)> = Vec::new();
    let mut fprops: Vec<(String, String, String)> = Vec::new(); // (count type, index type, name)
    let mut cur = "";
    for line in header.lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        match t.first().copied() {
            Some("format") => format = t.get(1).unwrap_or(&"ascii").to_string(),
            Some("element") => {
                cur = if t.get(1) == Some(&"vertex") { "v" } else if t.get(1) == Some(&"face") { "f" } else { "" };
                let n: usize = t.get(2).and_then(|x| x.parse().ok()).unwrap_or(0);
                if cur == "v" { nv = n } else if cur == "f" { nf = n }
            }
            Some("property") => {
                if cur == "v" && t.len() >= 3 {
                    vprops.push((t[1].to_string(), t[2].to_string()));
                } else if cur == "f" && t.len() >= 5 && t[1] == "list" {
                    fprops.push((t[2].to_string(), t[3].to_string(), t[4].to_string()));
                }
            }
            _ => {}
        }
    }
    let mut vertices: Vec<DVec3> = Vec::with_capacity(nv);
    let mut faces: Vec<Vec<usize>> = Vec::with_capacity(nf);
    let (ix, iy, iz) = (vprops.iter().position(|p| p.1 == "x"), vprops.iter().position(|p| p.1 == "y"), vprops.iter().position(|p| p.1 == "z"));
    if format.starts_with("ascii") {
        let text = String::from_utf8_lossy(&bytes[body_start..]);
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        for _ in 0..nv {
            let Some(l) = lines.next() else { break };
            let f = parse_floats(l);
            vertices.push(DVec3::new(ix.and_then(|i| f.get(i).copied()).unwrap_or(0.0), iy.and_then(|i| f.get(i).copied()).unwrap_or(0.0), iz.and_then(|i| f.get(i).copied()).unwrap_or(0.0)));
        }
        for _ in 0..nf {
            let Some(l) = lines.next() else { break };
            let v = parse_ints(l);
            if let Some(&n) = v.first() {
                faces.push(v.iter().skip(1).take(n as usize).map(|i| *i as usize).collect());
            }
        }
    } else {
        let little = format.contains("little");
        let size_of = |t: &str| match t { "char" | "uchar" | "int8" | "uint8" => 1, "short" | "ushort" | "int16" | "uint16" => 2, "int" | "uint" | "float" | "int32" | "uint32" | "float32" => 4, "double" | "float64" | "int64" | "uint64" => 8, _ => 4 };
        let mut pos = body_start;
        let read = |pos: &mut usize, t: &str| -> f64 {
            let n = size_of(t);
            if *pos + n > bytes.len() {
                *pos = bytes.len();
                return 0.0;
            }
            let b = &bytes[*pos..*pos + n];
            *pos += n;
            let mut arr = [0u8; 8];
            arr[..n].copy_from_slice(b);
            if !little {
                arr[..n].reverse();
            }
            match t {
                "char" | "int8" => arr[0] as i8 as f64,
                "uchar" | "uint8" => arr[0] as f64,
                "short" | "int16" => i16::from_le_bytes([arr[0], arr[1]]) as f64,
                "ushort" | "uint16" => u16::from_le_bytes([arr[0], arr[1]]) as f64,
                "int" | "int32" => i32::from_le_bytes([arr[0], arr[1], arr[2], arr[3]]) as f64,
                "uint" | "uint32" => u32::from_le_bytes([arr[0], arr[1], arr[2], arr[3]]) as f64,
                "float" | "float32" => f32::from_le_bytes([arr[0], arr[1], arr[2], arr[3]]) as f64,
                "double" | "float64" => f64::from_le_bytes(arr),
                _ => 0.0,
            }
        };
        for _ in 0..nv {
            let vals: Vec<f64> = vprops.iter().map(|(t, _)| read(&mut pos, t)).collect();
            vertices.push(DVec3::new(ix.and_then(|i| vals.get(i).copied()).unwrap_or(0.0), iy.and_then(|i| vals.get(i).copied()).unwrap_or(0.0), iz.and_then(|i| vals.get(i).copied()).unwrap_or(0.0)));
        }
        for _ in 0..nf {
            for (ct, it, name) in &fprops {
                let n = read(&mut pos, ct) as usize;
                let idx: Vec<usize> = (0..n).map(|_| read(&mut pos, it) as usize).collect();
                if name.starts_with("vertex_ind") {
                    faces.push(idx);
                }
            }
        }
    }
    let m = mesh_from_polygons(vertices, &faces);
    Ok(add_mesh(doc, &file_stem(path), m, None) as usize)
}

// ---------------------------------------------------------------- OFF / GTS / RAW

pub fn export_off(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let nv: usize = meshes.iter().map(|m| m.mesh.vertices.len()).sum();
    let nf: usize = meshes.iter().map(|m| m.mesh.triangle_count()).sum();
    let mut s = format!("OFF\n{nv} {nf} 0\n");
    for m in &meshes {
        for v in &m.mesh.vertices {
            writeln!(s, "{}", f3(*v))?;
        }
    }
    let mut base = 0usize;
    for m in &meshes {
        for t in m.mesh.indices.chunks(3) {
            writeln!(s, "3 {} {} {}", t[0] as usize + base, t[1] as usize + base, t[2] as usize + base)?;
        }
        base += m.mesh.vertices.len();
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_off(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let mut lines = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#'));
    let first = lines.next().unwrap_or("");
    let counts = if first.starts_with("OFF") { parse_ints(lines.next().unwrap_or("")) } else { parse_ints(first) };
    let (nv, nf) = (counts.first().copied().unwrap_or(0) as usize, counts.get(1).copied().unwrap_or(0) as usize);
    let mut vertices = Vec::with_capacity(nv);
    for _ in 0..nv {
        let f = parse_floats(lines.next().unwrap_or(""));
        if f.len() >= 3 {
            vertices.push(DVec3::new(f[0], f[1], f[2]));
        }
    }
    let mut faces = Vec::with_capacity(nf);
    for _ in 0..nf {
        let v = parse_ints(lines.next().unwrap_or(""));
        if let Some(&n) = v.first() {
            faces.push(v.iter().skip(1).take(n as usize).map(|i| *i as usize).collect());
        }
    }
    Ok(add_mesh(doc, &file_stem(path), mesh_from_polygons(vertices, &faces), None) as usize)
}

pub fn export_gts(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    // GTS: vertices, edges, faces (as edge triples)
    let mut verts: Vec<DVec3> = Vec::new();
    let mut edges: Vec<(usize, usize)> = Vec::new();
    let mut edge_ids: std::collections::HashMap<(usize, usize), usize> = std::collections::HashMap::new();
    let mut faces: Vec<[usize; 3]> = Vec::new();
    for m in &meshes {
        let base = verts.len();
        verts.extend_from_slice(&m.mesh.vertices);
        for t in m.mesh.indices.chunks(3) {
            let v = [t[0] as usize + base, t[1] as usize + base, t[2] as usize + base];
            let mut e = [0usize; 3];
            for k in 0..3 {
                let (a, b) = (v[k], v[(k + 1) % 3]);
                let key = (a.min(b), a.max(b));
                let id = *edge_ids.entry(key).or_insert_with(|| {
                    edges.push(key);
                    edges.len() - 1
                });
                e[k] = id;
            }
            faces.push(e);
        }
    }
    let mut s = format!("{} {} {} GtsSurface GtsFace GtsEdge GtsVertex\n", verts.len(), edges.len(), faces.len());
    for v in &verts {
        writeln!(s, "{}", f3(*v))?;
    }
    for (a, b) in &edges {
        writeln!(s, "{} {}", a + 1, b + 1)?;
    }
    for f in &faces {
        writeln!(s, "{} {} {}", f[0] + 1, f[1] + 1, f[2] + 1)?;
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_gts(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let mut lines = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty() && !l.starts_with('#'));
    let counts = parse_ints(lines.next().unwrap_or(""));
    let (nv, ne, nf) = (counts.first().copied().unwrap_or(0) as usize, counts.get(1).copied().unwrap_or(0) as usize, counts.get(2).copied().unwrap_or(0) as usize);
    let mut vertices = Vec::new();
    for _ in 0..nv {
        let f = parse_floats(lines.next().unwrap_or(""));
        if f.len() >= 3 {
            vertices.push(DVec3::new(f[0], f[1], f[2]));
        }
    }
    let mut edges = Vec::new();
    for _ in 0..ne {
        let v = parse_ints(lines.next().unwrap_or(""));
        if v.len() >= 2 {
            edges.push((v[0] as usize - 1, v[1] as usize - 1));
        }
    }
    let mut faces = Vec::new();
    for _ in 0..nf {
        let v = parse_ints(lines.next().unwrap_or(""));
        if v.len() >= 3 {
            let e: Vec<(usize, usize)> = v.iter().take(3).filter_map(|i| edges.get(*i as usize - 1).copied()).collect();
            if e.len() == 3 {
                // order the three vertices around the edge loop
                let a = e[0].0;
                let b = e[0].1;
                let c = if e[1].0 == a || e[1].0 == b { e[1].1 } else { e[1].0 };
                faces.push(vec![a, b, c]);
            }
        }
    }
    Ok(add_mesh(doc, &file_stem(path), mesh_from_polygons(vertices, &faces), None) as usize)
}

pub fn export_raw(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::new();
    for m in &meshes {
        writeln!(s, "{}", m.name)?;
        for t in 0..m.mesh.triangle_count() {
            let (a, b, c) = m.mesh.triangle(t);
            writeln!(s, "{} {} {}", f3(a), f3(b), f3(c))?;
        }
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_raw(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let mut count = 0;
    let mut name = file_stem(path);
    let mut m = Mesh::new();
    let flush = |doc: &mut Document, name: &str, m: &mut Mesh, count: &mut usize| {
        if m.triangle_count() > 0 {
            *count += add_mesh(doc, name, weld(m), None) as usize;
            *m = Mesh::new();
        }
    };
    for line in text.lines() {
        let f = parse_floats(line);
        if f.len() >= 9 {
            let b = m.vertices.len() as u32;
            m.vertices.push(DVec3::new(f[0], f[1], f[2]));
            m.vertices.push(DVec3::new(f[3], f[4], f[5]));
            m.vertices.push(DVec3::new(f[6], f[7], f[8]));
            m.indices.extend_from_slice(&[b, b + 1, b + 2]);
        } else if !line.trim().is_empty() {
            flush(doc, &name, &mut m, &mut count);
            name = line.trim().to_string();
        }
    }
    flush(doc, &name, &mut m, &mut count);
    Ok(count)
}

// ---------------------------------------------------------------- 3DS (binary chunks)

pub fn export_3ds(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    fn chunk(id: u16, body: &[u8]) -> Vec<u8> {
        let mut v = Vec::with_capacity(body.len() + 6);
        v.extend_from_slice(&id.to_le_bytes());
        v.extend_from_slice(&((body.len() + 6) as u32).to_le_bytes());
        v.extend_from_slice(body);
        v
    }
    let mut objects: Vec<u8> = Vec::new();
    for m in &meshes {
        // 3DS limits: 65535 vertices per object
        let nv = m.mesh.vertices.len().min(65535);
        let mut verts = Vec::new();
        verts.extend_from_slice(&(nv as u16).to_le_bytes());
        for v in m.mesh.vertices.iter().take(nv) {
            for c in [v.x, v.y, v.z] {
                verts.extend_from_slice(&(c as f32).to_le_bytes());
            }
        }
        let tris: Vec<&[u32]> = m.mesh.indices.chunks(3).filter(|t| t.iter().all(|i| (*i as usize) < nv)).take(65535).collect();
        let mut faces = Vec::new();
        faces.extend_from_slice(&(tris.len() as u16).to_le_bytes());
        for t in &tris {
            for i in *t {
                faces.extend_from_slice(&(*i as u16).to_le_bytes());
            }
            faces.extend_from_slice(&7u16.to_le_bytes());
        }
        let mut trimesh = chunk(0x4110, &verts);
        trimesh.extend_from_slice(&chunk(0x4120, &faces));
        let mut obj_body: Vec<u8> = m.name.bytes().take(10).filter(|b| *b != 0).collect();
        obj_body.push(0);
        obj_body.extend_from_slice(&chunk(0x4100, &trimesh));
        objects.extend_from_slice(&chunk(0x4000, &obj_body));
    }
    let mut mdata = chunk(0x3D3E, &3u32.to_le_bytes());
    mdata.extend_from_slice(&objects);
    let mut main_body = chunk(0x0002, &3u32.to_le_bytes());
    main_body.extend_from_slice(&chunk(0x3D3D, &mdata));
    let file = chunk(0x4D4D, &main_body);
    std::fs::write(path, file).with_context(|| format!("writing {}", path.display()))?;
    Ok(meshes.len())
}

pub fn import_3ds(doc: &mut Document, path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let u16 = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let u32 = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as usize;
    let f32 = |i: usize| f32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]) as f64;
    if bytes.len() < 6 || u16(0) != 0x4D4D {
        bail!("not a 3DS file");
    }
    let mut count = 0;
    // recursive chunk walk
    fn walk(bytes: &[u8], start: usize, end: usize, name: &mut String, verts: &mut Vec<DVec3>, faces: &mut Vec<Vec<usize>>, doc: &mut Document, count: &mut usize, u16: &dyn Fn(usize) -> usize, u32: &dyn Fn(usize) -> usize, f32: &dyn Fn(usize) -> f64) {
        let mut p = start;
        while p + 6 <= end {
            let id = u16(p);
            let len = u32(p + 2).max(6);
            let cend = (p + len).min(end);
            match id {
                0x4D4D | 0x3D3D => walk(bytes, p + 6, cend, name, verts, faces, doc, count, u16, u32, f32),
                0x4000 => {
                    let mut q = p + 6;
                    let mut n = Vec::new();
                    while q < cend && bytes[q] != 0 {
                        n.push(bytes[q]);
                        q += 1;
                    }
                    *name = String::from_utf8_lossy(&n).to_string();
                    verts.clear();
                    faces.clear();
                    walk(bytes, q + 1, cend, name, verts, faces, doc, count, u16, u32, f32);
                    if !verts.is_empty() && !faces.is_empty() {
                        *count += add_mesh(doc, name, mesh_from_polygons(verts.clone(), faces), None) as usize;
                    }
                    verts.clear();
                    faces.clear();
                }
                0x4100 => walk(bytes, p + 6, cend, name, verts, faces, doc, count, u16, u32, f32),
                0x4110 => {
                    let n = u16(p + 6);
                    for i in 0..n {
                        let o = p + 8 + i * 12;
                        if o + 12 <= cend {
                            verts.push(DVec3::new(f32(o), f32(o + 4), f32(o + 8)));
                        }
                    }
                }
                0x4120 => {
                    let n = u16(p + 6);
                    for i in 0..n {
                        let o = p + 8 + i * 8;
                        if o + 8 <= cend {
                            faces.push(vec![u16(o), u16(o + 2), u16(o + 4)]);
                        }
                    }
                }
                _ => {}
            }
            p = cend.max(p + 6);
        }
    }
    let mut name = file_stem(path);
    let (mut verts, mut faces) = (Vec::new(), Vec::new());
    walk(&bytes, 0, bytes.len(), &mut name, &mut verts, &mut faces, doc, &mut count, &u16, &u32, &f32);
    Ok(count)
}

// ---------------------------------------------------------------- glTF / GLB

pub fn export_gltf(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut gmeshes = Vec::new();
    let mut nodes = Vec::new();
    for (mi, m) in meshes.iter().enumerate() {
        let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
        let voff = bin.len();
        for v in &m.mesh.vertices {
            lo = lo.min(*v);
            hi = hi.max(*v);
            for c in [v.x, v.y, v.z] {
                bin.extend_from_slice(&(c as f32).to_le_bytes());
            }
        }
        let vlen = bin.len() - voff;
        let noff = bin.len();
        for n in &m.mesh.normals {
            for c in [n.x, n.y, n.z] {
                bin.extend_from_slice(&(c as f32).to_le_bytes());
            }
        }
        let nlen = bin.len() - noff;
        let ioff = bin.len();
        for i in &m.mesh.indices {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let ilen = bin.len() - ioff;
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let base = views.len();
        views.push(format!("{{\"buffer\":0,\"byteOffset\":{voff},\"byteLength\":{vlen},\"target\":34962}}"));
        views.push(format!("{{\"buffer\":0,\"byteOffset\":{noff},\"byteLength\":{nlen},\"target\":34962}}"));
        views.push(format!("{{\"buffer\":0,\"byteOffset\":{ioff},\"byteLength\":{ilen},\"target\":34963}}"));
        let acc = accessors.len();
        accessors.push(format!("{{\"bufferView\":{},\"componentType\":5126,\"count\":{},\"type\":\"VEC3\",\"min\":[{},{},{}],\"max\":[{},{},{}]}}", base, m.mesh.vertices.len(), lo.x as f32, lo.y as f32, lo.z as f32, hi.x as f32, hi.y as f32, hi.z as f32));
        accessors.push(format!("{{\"bufferView\":{},\"componentType\":5126,\"count\":{},\"type\":\"VEC3\"}}", base + 1, m.mesh.normals.len()));
        accessors.push(format!("{{\"bufferView\":{},\"componentType\":5125,\"count\":{},\"type\":\"SCALAR\"}}", base + 2, m.mesh.indices.len()));
        let col = m.color;
        gmeshes.push(format!("{{\"name\":\"{}\",\"primitives\":[{{\"attributes\":{{\"POSITION\":{},\"NORMAL\":{}}},\"indices\":{},\"material\":{}}}]}}", xml_escape(&m.name), acc, acc + 1, acc + 2, mi));
        nodes.push(format!("{{\"mesh\":{mi},\"name\":\"{}\"}}", xml_escape(&m.name)));
        let _ = col;
    }
    let materials: Vec<String> = meshes.iter().map(|m| format!("{{\"pbrMetallicRoughness\":{{\"baseColorFactor\":[{},{},{},1.0],\"metallicFactor\":0.0,\"roughnessFactor\":0.8}},\"doubleSided\":true}}", m.color[0] as f32 / 255.0, m.color[1] as f32 / 255.0, m.color[2] as f32 / 255.0)).collect();
    let scene_nodes: Vec<String> = (0..nodes.len()).map(|i| i.to_string()).collect();
    let glb = super::extension(path) == "glb";
    let buffer = if glb { format!("{{\"byteLength\":{}}}", bin.len()) } else { format!("{{\"byteLength\":{},\"uri\":\"{}.bin\"}}", bin.len(), xml_escape(&file_stem(path))) };
    let json = format!("{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"Triceratops\"}},\"scene\":0,\"scenes\":[{{\"nodes\":[{}]}}],\"nodes\":[{}],\"meshes\":[{}],\"materials\":[{}],\"accessors\":[{}],\"bufferViews\":[{}],\"buffers\":[{}]}}", scene_nodes.join(","), nodes.join(","), gmeshes.join(","), materials.join(","), accessors.join(","), views.join(","), buffer);
    if glb {
        let mut j = json.into_bytes();
        while j.len() % 4 != 0 {
            j.push(b' ');
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + 8 + j.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(j.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&j);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        std::fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    } else {
        write_text(path, &json)?;
        std::fs::write(path.with_extension("bin"), &bin).context("writing glTF buffer")?;
    }
    Ok(meshes.len())
}

pub fn import_gltf(doc: &mut Document, path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let (json_text, mut bin): (String, Option<Vec<u8>>) = if bytes.starts_with(b"glTF") {
        let mut p = 12;
        let mut json = String::new();
        let mut bin = None;
        while p + 8 <= bytes.len() {
            let len = u32::from_le_bytes([bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]) as usize;
            let kind = &bytes[p + 4..p + 8];
            let data = &bytes[p + 8..(p + 8 + len).min(bytes.len())];
            if kind == b"JSON" {
                json = String::from_utf8_lossy(data).to_string();
            } else if kind == b"BIN\0" {
                bin = Some(data.to_vec());
            }
            p += 8 + len;
        }
        (json, bin)
    } else {
        (String::from_utf8_lossy(&bytes).to_string(), None)
    };
    let v: serde_json::Value = serde_json::from_str(&json_text).context("parsing glTF JSON")?;
    if bin.is_none() {
        if let Some(uri) = v["buffers"][0]["uri"].as_str() {
            if let Some(data) = uri.strip_prefix("data:") {
                if let Some(b64) = data.split(",").nth(1) {
                    bin = Some(base64_decode(b64));
                }
            } else {
                bin = std::fs::read(path.parent().unwrap_or(Path::new(".")).join(uri)).ok();
            }
        }
    }
    let Some(bin) = bin else { bail!("glTF buffer not found") };
    let views = v["bufferViews"].as_array().cloned().unwrap_or_default();
    let accessors = v["accessors"].as_array().cloned().unwrap_or_default();
    let read_accessor = |idx: usize| -> Vec<f64> {
        let Some(a) = accessors.get(idx) else { return vec![] };
        let Some(view) = a["bufferView"].as_u64().and_then(|i| views.get(i as usize)) else { return vec![] };
        let off = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let count = a["count"].as_u64().unwrap_or(0) as usize;
        let comps = match a["type"].as_str().unwrap_or("SCALAR") { "VEC3" => 3, "VEC2" => 2, "VEC4" => 4, _ => 1 };
        let ct = a["componentType"].as_u64().unwrap_or(5126);
        let size = match ct { 5120 | 5121 => 1, 5122 | 5123 => 2, _ => 4 };
        let stride = view["byteStride"].as_u64().map(|s| s as usize).unwrap_or(comps * size);
        let mut out = Vec::with_capacity(count * comps);
        for i in 0..count {
            for c in 0..comps {
                let p = off + i * stride + c * size;
                if p + size > bin.len() {
                    out.push(0.0);
                    continue;
                }
                let b = &bin[p..p + size];
                out.push(match ct {
                    5120 => b[0] as i8 as f64,
                    5121 => b[0] as f64,
                    5122 => i16::from_le_bytes([b[0], b[1]]) as f64,
                    5123 => u16::from_le_bytes([b[0], b[1]]) as f64,
                    5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                    _ => f32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f64,
                });
            }
        }
        out
    };
    let mut count = 0;
    for (mi, mesh) in v["meshes"].as_array().cloned().unwrap_or_default().iter().enumerate() {
        let name = mesh["name"].as_str().map(|s| s.to_string()).unwrap_or_else(|| format!("mesh_{mi}"));
        let mut m = Mesh::new();
        for prim in mesh["primitives"].as_array().cloned().unwrap_or_default() {
            let Some(pa) = prim["attributes"]["POSITION"].as_u64() else { continue };
            let pos = read_accessor(pa as usize);
            let base = m.vertices.len() as u32;
            for c in pos.chunks(3) {
                if c.len() == 3 {
                    m.vertices.push(DVec3::new(c[0], c[1], c[2]));
                }
            }
            let n = (m.vertices.len() as u32 - base) as usize;
            let mode = prim["mode"].as_u64().unwrap_or(4);
            if mode != 4 {
                continue;
            }
            if let Some(ia) = prim["indices"].as_u64() {
                let idx = read_accessor(ia as usize);
                for t in idx.chunks(3) {
                    if t.len() == 3 {
                        m.indices.extend_from_slice(&[base + t[0] as u32, base + t[1] as u32, base + t[2] as u32]);
                    }
                }
            } else {
                for t in (0..n).step_by(3) {
                    if t + 2 < n {
                        m.indices.extend_from_slice(&[base + t as u32, base + t as u32 + 1, base + t as u32 + 2]);
                    }
                }
            }
        }
        count += add_mesh(doc, &name, m, None) as usize;
    }
    Ok(count)
}

fn base64_decode(s: &str) -> Vec<u8> {
    let table = |c: u8| -> Option<u32> {
        match c { b'A'..=b'Z' => Some((c - b'A') as u32), b'a'..=b'z' => Some((c - b'a') as u32 + 26), b'0'..=b'9' => Some((c - b'0') as u32 + 52), b'+' => Some(62), b'/' => Some(63), _ => None }
    };
    let mut out = Vec::new();
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let Some(v) = table(c) else { continue };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xff) as u8);
        }
    }
    out
}

// ---------------------------------------------------------------- XML mesh formats

fn coords_text(m: &Mesh) -> String {
    m.vertices.iter().map(|v| f3(*v)).collect::<Vec<_>>().join(" ")
}

pub fn export_dae(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">\n<asset><contributor><authoring_tool>Triceratops</authoring_tool></contributor><unit name=\"millimeter\" meter=\"0.001\"/><up_axis>Z_UP</up_axis></asset>\n<library_geometries>\n");
    for (i, m) in meshes.iter().enumerate() {
        let normals = m.mesh.normals.iter().map(|v| f3(*v)).collect::<Vec<_>>().join(" ");
        let idx = m.mesh.indices.iter().map(|k| format!("{k} {k}")).collect::<Vec<_>>().join(" ");
        writeln!(s, "<geometry id=\"g{i}\" name=\"{}\"><mesh>\n<source id=\"g{i}-pos\"><float_array id=\"g{i}-pos-a\" count=\"{}\">{}</float_array><technique_common><accessor source=\"#g{i}-pos-a\" count=\"{}\" stride=\"3\"><param name=\"X\" type=\"float\"/><param name=\"Y\" type=\"float\"/><param name=\"Z\" type=\"float\"/></accessor></technique_common></source>\n<source id=\"g{i}-nrm\"><float_array id=\"g{i}-nrm-a\" count=\"{}\">{}</float_array><technique_common><accessor source=\"#g{i}-nrm-a\" count=\"{}\" stride=\"3\"><param name=\"X\" type=\"float\"/><param name=\"Y\" type=\"float\"/><param name=\"Z\" type=\"float\"/></accessor></technique_common></source>\n<vertices id=\"g{i}-v\"><input semantic=\"POSITION\" source=\"#g{i}-pos\"/></vertices>\n<triangles count=\"{}\"><input semantic=\"VERTEX\" source=\"#g{i}-v\" offset=\"0\"/><input semantic=\"NORMAL\" source=\"#g{i}-nrm\" offset=\"1\"/><p>{}</p></triangles>\n</mesh></geometry>", xml_escape(&m.name), m.mesh.vertices.len() * 3, coords_text(&m.mesh), m.mesh.vertices.len(), m.mesh.normals.len() * 3, normals, m.mesh.normals.len(), m.mesh.triangle_count(), idx)?;
    }
    s.push_str("</library_geometries>\n<library_visual_scenes><visual_scene id=\"scene\">\n");
    for (i, m) in meshes.iter().enumerate() {
        writeln!(s, "<node id=\"n{i}\" name=\"{}\"><instance_geometry url=\"#g{i}\"/></node>", xml_escape(&m.name))?;
    }
    s.push_str("</visual_scene></library_visual_scenes>\n<scene><instance_visual_scene url=\"#scene\"/></scene>\n</COLLADA>\n");
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_dae(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let xml = roxmltree::Document::parse(&text).context("parsing COLLADA")?;
    let mut count = 0;
    for geom in xml.descendants().filter(|n| n.has_tag_name("geometry")) {
        let name = geom.attribute("name").or(geom.attribute("id")).unwrap_or("mesh").to_string();
        let Some(mesh) = geom.children().find(|n| n.has_tag_name("mesh")) else { continue };
        // sources by id
        let mut sources: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();
        for src in mesh.children().filter(|n| n.has_tag_name("source")) {
            if let (Some(id), Some(arr)) = (src.attribute("id"), src.children().find(|n| n.has_tag_name("float_array"))) {
                sources.insert(id.to_string(), parse_floats(arr.text().unwrap_or("")));
            }
        }
        let pos_source = mesh.children().find(|n| n.has_tag_name("vertices")).and_then(|v| v.children().find(|n| n.has_tag_name("input") && n.attribute("semantic") == Some("POSITION")).and_then(|i| i.attribute("source"))).map(|s| s.trim_start_matches('#').to_string());
        let Some(pos) = pos_source.and_then(|id| sources.get(&id)) else { continue };
        let vertices: Vec<DVec3> = pos.chunks(3).filter(|c| c.len() == 3).map(|c| DVec3::new(c[0], c[1], c[2])).collect();
        let mut faces: Vec<Vec<usize>> = Vec::new();
        for prim in mesh.children().filter(|n| n.has_tag_name("triangles") || n.has_tag_name("polylist") || n.has_tag_name("polygons")) {
            let inputs: Vec<roxmltree::Node> = prim.children().filter(|n| n.has_tag_name("input")).collect();
            let stride = inputs.iter().filter_map(|i| i.attribute("offset").and_then(|o| o.parse::<usize>().ok())).max().map_or(1, |m| m + 1);
            let voff = inputs.iter().find(|i| i.attribute("semantic") == Some("VERTEX")).and_then(|i| i.attribute("offset")).and_then(|o| o.parse::<usize>().ok()).unwrap_or(0);
            let p: Vec<i64> = prim.children().filter(|n| n.has_tag_name("p")).flat_map(|n| parse_ints(n.text().unwrap_or(""))).collect();
            let idx: Vec<usize> = p.chunks(stride).filter_map(|c| c.get(voff).map(|i| *i as usize)).collect();
            if prim.has_tag_name("polylist") {
                let vc = parse_ints(prim.children().find(|n| n.has_tag_name("vcount")).and_then(|n| n.text()).unwrap_or(""));
                let mut k = 0;
                for n in vc {
                    let n = n as usize;
                    if k + n <= idx.len() {
                        faces.push(idx[k..k + n].to_vec());
                    }
                    k += n;
                }
            } else {
                for t in idx.chunks(3) {
                    if t.len() == 3 {
                        faces.push(t.to_vec());
                    }
                }
            }
        }
        count += add_mesh(doc, &name, mesh_from_polygons(vertices, &faces), None) as usize;
    }
    Ok(count)
}

pub fn export_x3d(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<X3D profile=\"Interchange\" version=\"3.3\"><Scene>\n");
    for m in &meshes {
        let idx = m.mesh.indices.chunks(3).map(|t| format!("{} {} {} -1", t[0], t[1], t[2])).collect::<Vec<_>>().join(" ");
        writeln!(s, "<Transform><Shape DEF=\"{}\"><Appearance><Material diffuseColor=\"{} {} {}\"/></Appearance><IndexedFaceSet coordIndex=\"{}\"><Coordinate point=\"{}\"/></IndexedFaceSet></Shape></Transform>", xml_escape(&m.name), m.color[0] as f32 / 255.0, m.color[1] as f32 / 255.0, m.color[2] as f32 / 255.0, idx, coords_text(&m.mesh))?;
    }
    s.push_str("</Scene></X3D>\n");
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_x3d(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    if !text.trim_start().starts_with('<') {
        return import_wrl(doc, path);
    }
    let xml = roxmltree::Document::parse(&text).context("parsing X3D")?;
    let mut count = 0;
    for (i, ifs) in xml.descendants().filter(|n| n.has_tag_name("IndexedFaceSet")).enumerate() {
        let Some(coord) = ifs.children().find(|n| n.has_tag_name("Coordinate")) else { continue };
        let pts = parse_floats(coord.attribute("point").unwrap_or(""));
        let vertices: Vec<DVec3> = pts.chunks(3).filter(|c| c.len() == 3).map(|c| DVec3::new(c[0], c[1], c[2])).collect();
        let faces = split_minus_one(&parse_ints(ifs.attribute("coordIndex").unwrap_or("")));
        let name = ifs.parent().and_then(|p| p.attribute("DEF")).map(|s| s.to_string()).unwrap_or_else(|| format!("{}_{i}", file_stem(path)));
        count += add_mesh(doc, &name, mesh_from_polygons(vertices, &faces), None) as usize;
    }
    Ok(count)
}

fn split_minus_one(idx: &[i64]) -> Vec<Vec<usize>> {
    let mut faces = Vec::new();
    let mut cur = Vec::new();
    for &i in idx {
        if i < 0 {
            if cur.len() >= 3 {
                faces.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        } else {
            cur.push(i as usize);
        }
    }
    if cur.len() >= 3 {
        faces.push(cur);
    }
    faces
}

pub fn export_wrl(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("#VRML V2.0 utf8\n# Triceratops\n");
    for m in &meshes {
        let pts = m.mesh.vertices.iter().map(|v| format!("{} {} {}", fmt(v.x), fmt(v.y), fmt(v.z))).collect::<Vec<_>>().join(", ");
        let idx = m.mesh.indices.chunks(3).map(|t| format!("{}, {}, {}, -1", t[0], t[1], t[2])).collect::<Vec<_>>().join(", ");
        writeln!(s, "DEF {} Shape {{\n  appearance Appearance {{ material Material {{ diffuseColor {} {} {} }} }}\n  geometry IndexedFaceSet {{\n    coord Coordinate {{ point [ {} ] }}\n    coordIndex [ {} ]\n  }}\n}}", m.name.replace(' ', "_"), m.color[0] as f32 / 255.0, m.color[1] as f32 / 255.0, m.color[2] as f32 / 255.0, pts, idx)?;
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_wrl(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let text: String = text.lines().map(|l| l.split('#').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
    let mut count = 0;
    let mut search = 0;
    let bracket_block = |s: &str, key: &str, from: usize| -> Option<(usize, usize)> {
        let k = s[from..].find(key)? + from;
        let open = s[k..].find('[')? + k;
        let close = s[open..].find(']')? + open;
        Some((open + 1, close))
    };
    while let Some(k) = text[search..].find("IndexedFaceSet") {
        let start = search + k;
        // the block ends at the matching brace
        let mut depth = 0;
        let mut end = text.len();
        for (i, c) in text[start..].char_indices() {
            if c == '{' {
                depth += 1;
            } else if c == '}' {
                depth -= 1;
                if depth == 0 {
                    end = start + i;
                    break;
                }
            }
        }
        let block = &text[start..end];
        let verts: Vec<DVec3> = bracket_block(block, "point", 0).map(|(a, b)| parse_floats(&block[a..b]).chunks(3).filter(|c| c.len() == 3).map(|c| DVec3::new(c[0], c[1], c[2])).collect()).unwrap_or_default();
        let faces = bracket_block(block, "coordIndex", 0).map(|(a, b)| split_minus_one(&parse_ints(&block[a..b]))).unwrap_or_default();
        if !verts.is_empty() && !faces.is_empty() {
            count += add_mesh(doc, &format!("{}_{count}", file_stem(path)), mesh_from_polygons(verts, &faces), None) as usize;
        }
        search = end.max(start + 14);
    }
    Ok(count)
}

pub fn export_amf(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<amf unit=\"millimeter\" version=\"1.1\">\n<metadata type=\"producer\">Triceratops</metadata>\n");
    for (i, m) in meshes.iter().enumerate() {
        writeln!(s, "<object id=\"{i}\"><metadata type=\"name\">{}</metadata><mesh><vertices>", xml_escape(&m.name))?;
        for v in &m.mesh.vertices {
            writeln!(s, "<vertex><coordinates><x>{}</x><y>{}</y><z>{}</z></coordinates></vertex>", fmt(v.x), fmt(v.y), fmt(v.z))?;
        }
        s.push_str("</vertices><volume>\n");
        for t in m.mesh.indices.chunks(3) {
            writeln!(s, "<triangle><v1>{}</v1><v2>{}</v2><v3>{}</v3></triangle>", t[0], t[1], t[2])?;
        }
        s.push_str("</volume></mesh></object>\n");
    }
    s.push_str("</amf>\n");
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_amf(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let xml = roxmltree::Document::parse(&text).context("parsing AMF")?;
    let mut count = 0;
    for (i, obj) in xml.descendants().filter(|n| n.has_tag_name("object")).enumerate() {
        let name = obj.children().find(|n| n.has_tag_name("metadata") && n.attribute("type") == Some("name")).and_then(|n| n.text()).map(|s| s.to_string()).unwrap_or_else(|| format!("object_{i}"));
        let num = |n: roxmltree::Node, tag: &str| n.children().find(|c| c.has_tag_name(tag)).and_then(|c| c.text()).and_then(|t| t.trim().parse::<f64>().ok()).unwrap_or(0.0);
        let mut vertices = Vec::new();
        for v in obj.descendants().filter(|n| n.has_tag_name("coordinates")) {
            vertices.push(DVec3::new(num(v, "x"), num(v, "y"), num(v, "z")));
        }
        let mut faces = Vec::new();
        for t in obj.descendants().filter(|n| n.has_tag_name("triangle")) {
            faces.push(vec![num(t, "v1") as usize, num(t, "v2") as usize, num(t, "v3") as usize]);
        }
        count += add_mesh(doc, &name, mesh_from_polygons(vertices, &faces), None) as usize;
    }
    Ok(count)
}

pub fn export_3mf(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut model = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n<metadata name=\"Application\">Triceratops</metadata>\n<resources>\n");
    for (i, m) in meshes.iter().enumerate() {
        writeln!(model, "<object id=\"{}\" name=\"{}\" type=\"model\"><mesh><vertices>", i + 1, xml_escape(&m.name))?;
        for v in &m.mesh.vertices {
            writeln!(model, "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>", fmt(v.x), fmt(v.y), fmt(v.z))?;
        }
        model.push_str("</vertices><triangles>\n");
        for t in m.mesh.indices.chunks(3) {
            writeln!(model, "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2])?;
        }
        model.push_str("</triangles></mesh></object>\n");
    }
    model.push_str("</resources>\n<build>\n");
    for i in 0..meshes.len() {
        writeln!(model, "<item objectid=\"{}\"/>", i + 1)?;
    }
    model.push_str("</build>\n</model>\n");
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"model\" ContentType=\"application/vnd.ms-package.3dmanufacturing-3dmodel+xml\"/></Types>";
    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Target=\"/3D/3dmodel.model\" Id=\"rel0\" Type=\"http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel\"/></Relationships>";
    let zip = zip_write(&[("[Content_Types].xml".into(), content_types.as_bytes().to_vec()), ("_rels/.rels".into(), rels.as_bytes().to_vec()), ("3D/3dmodel.model".into(), model.into_bytes())]);
    std::fs::write(path, zip).with_context(|| format!("writing {}", path.display()))?;
    Ok(meshes.len())
}

pub fn import_3mf(doc: &mut Document, path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let entries = zip_read(&bytes)?;
    let Some((_, model)) = entries.iter().find(|(n, _)| n.ends_with(".model")) else { bail!("no 3D model in the 3MF package") };
    let text = String::from_utf8_lossy(model).to_string();
    let xml = roxmltree::Document::parse(&text).context("parsing 3MF model")?;
    let mut count = 0;
    for (i, obj) in xml.descendants().filter(|n| n.has_tag_name("object")).enumerate() {
        let name = obj.attribute("name").map(|s| s.to_string()).unwrap_or_else(|| format!("object_{i}"));
        let attr = |n: roxmltree::Node, a: &str| n.attribute(a).and_then(|t| t.parse::<f64>().ok()).unwrap_or(0.0);
        let vertices: Vec<DVec3> = obj.descendants().filter(|n| n.has_tag_name("vertex")).map(|v| DVec3::new(attr(v, "x"), attr(v, "y"), attr(v, "z"))).collect();
        let faces: Vec<Vec<usize>> = obj.descendants().filter(|n| n.has_tag_name("triangle")).map(|t| vec![attr(t, "v1") as usize, attr(t, "v2") as usize, attr(t, "v3") as usize]).collect();
        if !vertices.is_empty() {
            count += add_mesh(doc, &name, mesh_from_polygons(vertices, &faces), None) as usize;
        }
    }
    Ok(count)
}

// ---------------------------------------------------------------- DirectX .x (text)

pub fn export_x(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("xof 0303txt 0032\n");
    for m in &meshes {
        writeln!(s, "Mesh {} {{\n {};", m.name.replace(|c: char| !c.is_alphanumeric(), "_"), m.mesh.vertices.len())?;
        let pts: Vec<String> = m.mesh.vertices.iter().map(|v| format!(" {};{};{};", fmt(v.x), fmt(v.y), fmt(v.z))).collect();
        writeln!(s, "{};", pts.join(",\n"))?;
        writeln!(s, " {};", m.mesh.triangle_count())?;
        let tris: Vec<String> = m.mesh.indices.chunks(3).map(|t| format!(" 3;{},{},{};", t[0], t[1], t[2])).collect();
        writeln!(s, "{};\n}}", tris.join(",\n"))?;
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_x(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let mut count = 0;
    let mut search = 0;
    while let Some(k) = text[search..].find("Mesh ") {
        let start = search + k;
        let Some(open) = text[start..].find('{') else { break };
        let body_start = start + open + 1;
        let name = text[start + 5..start + open].trim().to_string();
        // the body: vertex count; vertices; face count; faces
        let nums: Vec<f64> = text[body_start..].split(|c: char| c == ';' || c == ',' || c.is_whitespace() || c == '{' || c == '}').filter(|t| !t.is_empty()).map_while(|t| t.parse::<f64>().ok()).collect();
        if nums.is_empty() {
            search = body_start;
            continue;
        }
        let nv = nums[0] as usize;
        let mut vertices = Vec::with_capacity(nv);
        let mut p = 1;
        for _ in 0..nv {
            if p + 3 > nums.len() {
                break;
            }
            vertices.push(DVec3::new(nums[p], nums[p + 1], nums[p + 2]));
            p += 3;
        }
        let nf = nums.get(p).copied().unwrap_or(0.0) as usize;
        p += 1;
        let mut faces = Vec::with_capacity(nf);
        for _ in 0..nf {
            let Some(&n) = nums.get(p) else { break };
            let n = n as usize;
            p += 1;
            if p + n > nums.len() {
                break;
            }
            faces.push(nums[p..p + n].iter().map(|i| *i as usize).collect());
            p += n;
        }
        count += add_mesh(doc, if name.is_empty() { &"mesh" } else { &name }, mesh_from_polygons(vertices, &faces), None) as usize;
        search = body_start;
    }
    Ok(count)
}

// ---------------------------------------------------------------- FBX (ASCII 7.x)

pub fn export_fbx(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("; FBX 7.4.0 project file\n; Triceratops\nFBXHeaderExtension:  {\n\tFBXHeaderVersion: 1003\n\tFBXVersion: 7400\n\tCreator: \"Triceratops\"\n}\nGlobalSettings:  {\n\tVersion: 1000\n\tProperties70:  {\n\t\tP: \"UpAxis\", \"int\", \"Integer\", \"\",2\n\t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",1\n\t}\n}\n");
    writeln!(s, "Definitions:  {{\n\tVersion: 100\n\tCount: {}\n\tObjectType: \"Geometry\" {{\n\t\tCount: {}\n\t}}\n\tObjectType: \"Model\" {{\n\t\tCount: {}\n\t}}\n}}", meshes.len() * 2, meshes.len(), meshes.len())?;
    s.push_str("Objects:  {\n");
    for (i, m) in meshes.iter().enumerate() {
        let gid = 1000 + i as u64 * 2;
        let mid = gid + 1;
        let verts: Vec<String> = m.mesh.vertices.iter().flat_map(|v| [fmt(v.x), fmt(v.y), fmt(v.z)]).collect();
        let idx: Vec<String> = m.mesh.indices.chunks(3).flat_map(|t| [t[0].to_string(), t[1].to_string(), format!("{}", -(t[2] as i64) - 1)]).collect();
        writeln!(s, "\tGeometry: {gid}, \"Geometry::{}\", \"Mesh\" {{\n\t\tVertices: *{} {{\n\t\t\ta: {}\n\t\t}}\n\t\tPolygonVertexIndex: *{} {{\n\t\t\ta: {}\n\t\t}}\n\t\tGeometryVersion: 124\n\t}}", m.name, verts.len(), verts.join(","), idx.len(), idx.join(","))?;
        writeln!(s, "\tModel: {mid}, \"Model::{}\", \"Mesh\" {{\n\t\tVersion: 232\n\t\tProperties70:  {{\n\t\t}}\n\t\tShading: T\n\t\tCulling: \"CullingOff\"\n\t}}", m.name)?;
    }
    s.push_str("}\nConnections:  {\n");
    for i in 0..meshes.len() {
        let gid = 1000 + i as u64 * 2;
        writeln!(s, "\tC: \"OO\",{},{}\n\tC: \"OO\",{},0", gid, gid + 1, gid + 1)?;
    }
    s.push_str("}\n");
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_fbx(doc: &mut Document, path: &Path) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    if bytes.starts_with(b"Kaydara FBX Binary") {
        bail!("binary FBX is not supported: save the file as ASCII FBX");
    }
    let text = String::from_utf8_lossy(&bytes).to_string();
    let mut count = 0;
    let mut search = 0;
    let array_after = |s: &str, key: &str, from: usize| -> Option<(Vec<f64>, usize)> {
        let k = s[from..].find(key)? + from;
        let open = s[k..].find('{')? + k;
        let close = s[open..].find('}')? + open;
        let body = &s[open + 1..close];
        let body = body.trim().trim_start_matches("a:");
        Some((parse_floats(body), close))
    };
    while let Some(k) = text[search..].find("Geometry:") {
        let start = search + k;
        let name = text[start..].find('"').and_then(|a| { let rest = &text[start + a + 1..]; rest.find('"').map(|b| rest[..b].trim_start_matches("Geometry::").to_string()) }).unwrap_or_else(|| "mesh".into());
        let Some((verts, after)) = array_after(&text, "Vertices:", start) else { search = start + 9; continue };
        let Some((idx, after2)) = array_after(&text, "PolygonVertexIndex:", after) else { search = after; continue };
        let vertices: Vec<DVec3> = verts.chunks(3).filter(|c| c.len() == 3).map(|c| DVec3::new(c[0], c[1], c[2])).collect();
        let mut faces = Vec::new();
        let mut cur = Vec::new();
        for i in idx {
            let i = i as i64;
            if i < 0 {
                cur.push((-i - 1) as usize);
                faces.push(std::mem::take(&mut cur));
            } else {
                cur.push(i as usize);
            }
        }
        count += add_mesh(doc, &name, mesh_from_polygons(vertices, &faces), None) as usize;
        search = after2;
    }
    Ok(count)
}

// ---------------------------------------------------------------- POV-Ray / RenderMan (export)

pub fn export_pov(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("// Triceratops POV-Ray export\n#version 3.7;\nglobal_settings { assumed_gamma 1.0 }\ncamera { location <-50, -50, 40> look_at <0, 0, 0> up z sky z right x*image_width/image_height }\nlight_source { <-100, -100, 150> color rgb 1 }\n");
    for m in &meshes {
        writeln!(s, "#declare {} = mesh2 {{\n  vertex_vectors {{ {}, {} }}\n  face_indices {{ {}, {} }}\n  pigment {{ color rgb <{}, {}, {}> }}\n}}\nobject {{ {} }}", ident(&m.name), m.mesh.vertices.len(), m.mesh.vertices.iter().map(|v| format!("<{},{},{}>", fmt(v.x), fmt(v.y), fmt(v.z))).collect::<Vec<_>>().join(", "), m.mesh.triangle_count(), m.mesh.indices.chunks(3).map(|t| format!("<{},{},{}>", t[0], t[1], t[2])).collect::<Vec<_>>().join(", "), m.color[0] as f32 / 255.0, m.color[1] as f32 / 255.0, m.color[2] as f32 / 255.0, ident(&m.name))?;
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

fn ident(name: &str) -> String {
    let mut s: String = name.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    if s.chars().next().map_or(true, |c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

pub fn export_rib(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let meshes = collect_meshes(doc, only_selected);
    let mut s = String::from("##RenderMan RIB-Structure 1.1\n# Triceratops\nDisplay \"render.tif\" \"file\" \"rgb\"\nProjection \"perspective\" \"fov\" [40]\nTranslate 0 0 120\nRotate -60 1 0 0\nWorldBegin\nLightSource \"distantlight\" 1 \"from\" [-1 -1 2] \"to\" [0 0 0]\n");
    for m in &meshes {
        let nverts: Vec<String> = m.mesh.indices.chunks(3).map(|_| "3".to_string()).collect();
        writeln!(s, "AttributeBegin\nAttribute \"identifier\" \"name\" [\"{}\"]\nColor [{} {} {}]\nPointsPolygons [{}] [{}] \"P\" [{}]\nAttributeEnd", m.name, m.color[0] as f32 / 255.0, m.color[1] as f32 / 255.0, m.color[2] as f32 / 255.0, nverts.join(" "), m.mesh.indices.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(" "), coords_text(&m.mesh))?;
    }
    s.push_str("WorldEnd\n");
    write_text(path, &s)?;
    Ok(meshes.len())
}

// ---------------------------------------------------------------- SLC slices

pub fn export_slc(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    // planar sections of every mesh every `layer` units along Z, written as the SLC ASCII variant
    let meshes = collect_meshes(doc, only_selected);
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for m in &meshes {
        let bb = m.mesh.bounding_box();
        lo = lo.min(bb.min.z);
        hi = hi.max(bb.max.z);
    }
    if meshes.is_empty() || !lo.is_finite() {
        bail!("nothing to slice");
    }
    let layer = ((hi - lo) / 100.0).max(1e-3);
    let mut s = format!("-SLCVER 2.0 -UNIT MM -TYPE PART -PACKAGE Triceratops -EXTENTS {},{} {},{}\n", fmt(lo), fmt(hi), fmt(lo), fmt(hi));
    let mut z = lo + layer * 0.5;
    while z < hi {
        let plane = Plane::from_normal(DVec3::new(0.0, 0.0, z), DVec3::Z);
        let mut polys = Vec::new();
        for m in &meshes {
            polys.extend(m.mesh.section(&plane, layer * 1e-6));
        }
        writeln!(s, "Z {} {}", fmt(z), polys.len())?;
        for p in polys {
            let pts: Vec<String> = p.iter().map(|q| format!("{} {}", fmt(q.x), fmt(q.y))).collect();
            writeln!(s, "{} {}", p.len(), pts.join(" "))?;
        }
        z += layer;
    }
    write_text(path, &s)?;
    Ok(meshes.len())
}

pub fn import_slc(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let mut count = 0;
    let mut z = 0.0;
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("Z ") {
            z = parse_floats(rest).first().copied().unwrap_or(z);
            continue;
        }
        if l.starts_with('-') || l.is_empty() {
            continue;
        }
        let f = parse_floats(l);
        if f.len() >= 5 {
            let n = f[0] as usize;
            let pts: Vec<DVec3> = f[1..].chunks(2).take(n).filter(|c| c.len() == 2).map(|c| DVec3::new(c[0], c[1], z)).collect();
            let mut pts = pts;
            if pts.len() > 2 {
                if (pts[0] - pts[pts.len() - 1]).length() > 1e-9 {
                    pts.push(pts[0]);
                }
                count += add_polyline(doc, "slice", pts, None) as usize;
            }
        }
    }
    Ok(count)
}

