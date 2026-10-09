//! Import and export of every file format Triceratops reads or writes besides its native
//! `.tri`, `.3dm` and DXF/DWG: a registry (`FORMATS`) with one importer/exporter per format and
//! the helpers the format modules share (mesh collection, polylines, XML and zip utilities).

use std::path::Path;

use anyhow::{bail, Context, Result};
use glam::DVec3;
use tri_geom::{Mesh, NurbsCurve};

use crate::{Attributes, Document, Geometry};

pub mod iges;
pub mod mesh_formats;
pub mod points;
pub mod step;
pub mod vda;
pub mod vector2d;

#[cfg(test)]
mod tests;

/// What a format can do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub import: bool,
    pub export: bool,
}

/// Every format handled here (native, 3dm and DXF/DWG live in their own modules).
pub const FORMATS: &[Format] = &[
    Format { name: "Stereolithography", extensions: &["stl"], import: true, export: true },
    Format { name: "Stanford Polygon", extensions: &["ply"], import: true, export: true },
    Format { name: "Object File Format", extensions: &["off"], import: true, export: true },
    Format { name: "3D Studio", extensions: &["3ds"], import: true, export: true },
    Format { name: "glTF / GLB", extensions: &["gltf", "glb"], import: true, export: true },
    Format { name: "COLLADA", extensions: &["dae"], import: true, export: true },
    Format { name: "VRML", extensions: &["wrl", "vrml"], import: true, export: true },
    Format { name: "X3D", extensions: &["x3d", "x3dv"], import: true, export: true },
    Format { name: "DirectX", extensions: &["x"], import: true, export: true },
    Format { name: "Additive Manufacturing", extensions: &["amf"], import: true, export: true },
    Format { name: "3D Manufacturing", extensions: &["3mf"], import: true, export: true },
    Format { name: "GTS", extensions: &["gts"], import: true, export: true },
    Format { name: "Raw triangles", extensions: &["raw"], import: true, export: true },
    Format { name: "Autodesk FBX (ASCII)", extensions: &["fbx"], import: true, export: true },
    Format { name: "POV-Ray", extensions: &["pov"], import: false, export: true },
    Format { name: "RenderMan", extensions: &["rib"], import: false, export: true },
    Format { name: "Slice", extensions: &["slc"], import: true, export: true },
    Format { name: "Points", extensions: &["pts", "xyz", "asc", "csv", "txt"], import: true, export: true },
    Format { name: "SVG", extensions: &["svg"], import: true, export: true },
    Format { name: "PDF", extensions: &["pdf"], import: false, export: true },
    Format { name: "Encapsulated PostScript", extensions: &["eps"], import: false, export: true },
    Format { name: "Adobe Illustrator", extensions: &["ai"], import: false, export: true },
    Format { name: "IGES", extensions: &["igs", "iges"], import: true, export: true },
    Format { name: "STEP", extensions: &["stp", "step"], import: true, export: true },
    Format { name: "VDA-FS", extensions: &["vda"], import: true, export: true },
];

/// Formats that need a proprietary or undocumented reader; they are reported, not pretended.
pub const UNSUPPORTED: &[(&str, &str)] = &[
    ("skp", "SketchUp files are a proprietary format: export OBJ, DAE or 3DS from SketchUp"),
    ("sat", "ACIS SAT solids are not supported: use STEP or IGES"),
    ("dgn", "MicroStation DGN is not supported: use DXF or DWG"),
    ("e57", "E57 point clouds are not supported: export PTS or XYZ"),
    ("lwo", "LightWave objects are not supported: use OBJ"),
    ("xgl", "XGL/ZGL is not supported: use DAE or OBJ"),
    ("zpr", "ZPrint files are not supported: use STL or 3MF"),
    ("udo", "UDO files are not supported: use OBJ"),
    ("kml", "KML/KMZ is not supported: use DAE"),
];

pub fn format_for(ext: &str) -> Option<&'static Format> {
    let ext = ext.to_ascii_lowercase();
    FORMATS.iter().find(|f| f.extensions.contains(&ext.as_str()))
}

pub fn unsupported_reason(ext: &str) -> Option<&'static str> {
    let ext = ext.to_ascii_lowercase();
    UNSUPPORTED.iter().find(|(e, _)| *e == ext).map(|(_, r)| *r)
}

/// Extensions that can be imported, for file dialogs.
pub fn import_extensions() -> Vec<&'static str> {
    FORMATS.iter().filter(|f| f.import).flat_map(|f| f.extensions.iter().copied()).collect()
}

pub fn export_extensions() -> Vec<&'static str> {
    FORMATS.iter().filter(|f| f.export).flat_map(|f| f.extensions.iter().copied()).collect()
}

/// Import a file by its extension. Returns the number of objects added.
pub fn import(doc: &mut Document, path: &Path) -> Result<usize> {
    let ext = extension(path);
    if let Some(reason) = unsupported_reason(&ext) {
        bail!("{reason}");
    }
    match ext.as_str() {
        "stl" => mesh_formats::import_stl(doc, path),
        "ply" => mesh_formats::import_ply(doc, path),
        "off" => mesh_formats::import_off(doc, path),
        "3ds" => mesh_formats::import_3ds(doc, path),
        "gltf" | "glb" => mesh_formats::import_gltf(doc, path),
        "dae" => mesh_formats::import_dae(doc, path),
        "wrl" | "vrml" => mesh_formats::import_wrl(doc, path),
        "x3d" | "x3dv" => mesh_formats::import_x3d(doc, path),
        "x" => mesh_formats::import_x(doc, path),
        "amf" => mesh_formats::import_amf(doc, path),
        "3mf" => mesh_formats::import_3mf(doc, path),
        "gts" => mesh_formats::import_gts(doc, path),
        "raw" => mesh_formats::import_raw(doc, path),
        "fbx" => mesh_formats::import_fbx(doc, path),
        "slc" => mesh_formats::import_slc(doc, path),
        "pts" | "xyz" | "asc" | "csv" | "txt" => points::import(doc, path),
        "svg" => vector2d::import_svg(doc, path),
        "igs" | "iges" => iges::import(doc, path),
        "stp" | "step" => step::import(doc, path),
        "vda" => vda::import(doc, path),
        other => bail!("unsupported file type: .{other}"),
    }
}

/// Export the visible (or selected) objects by extension. Returns the number of objects written.
pub fn export(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let ext = extension(path);
    if let Some(reason) = unsupported_reason(&ext) {
        bail!("{reason}");
    }
    match ext.as_str() {
        "stl" => mesh_formats::export_stl(doc, path, only_selected),
        "ply" => mesh_formats::export_ply(doc, path, only_selected),
        "off" => mesh_formats::export_off(doc, path, only_selected),
        "3ds" => mesh_formats::export_3ds(doc, path, only_selected),
        "gltf" | "glb" => mesh_formats::export_gltf(doc, path, only_selected),
        "dae" => mesh_formats::export_dae(doc, path, only_selected),
        "wrl" | "vrml" => mesh_formats::export_wrl(doc, path, only_selected),
        "x3d" | "x3dv" => mesh_formats::export_x3d(doc, path, only_selected),
        "x" => mesh_formats::export_x(doc, path, only_selected),
        "amf" => mesh_formats::export_amf(doc, path, only_selected),
        "3mf" => mesh_formats::export_3mf(doc, path, only_selected),
        "gts" => mesh_formats::export_gts(doc, path, only_selected),
        "raw" => mesh_formats::export_raw(doc, path, only_selected),
        "fbx" => mesh_formats::export_fbx(doc, path, only_selected),
        "pov" => mesh_formats::export_pov(doc, path, only_selected),
        "rib" => mesh_formats::export_rib(doc, path, only_selected),
        "slc" => mesh_formats::export_slc(doc, path, only_selected),
        "pts" | "xyz" | "asc" | "csv" | "txt" => points::export(doc, path, only_selected),
        "svg" => vector2d::export_svg(doc, path, only_selected),
        "pdf" => vector2d::export_pdf(doc, path, only_selected),
        "eps" | "ai" => vector2d::export_eps(doc, path, only_selected),
        "igs" | "iges" => iges::export(doc, path, only_selected),
        "stp" | "step" => step::export(doc, path, only_selected),
        "vda" => vda::export(doc, path, only_selected),
        other => bail!("unsupported file type: .{other}"),
    }
}

pub fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

// ---------------------------------------------------------------- shared helpers

/// A named triangle mesh ready to be written.
pub struct NamedMesh {
    pub name: String,
    pub mesh: Mesh,
    pub color: [u8; 4],
}

/// Whether an object takes part in an export.
pub fn exported(doc: &Document, o: &crate::Object, only_selected: bool) -> bool {
    doc.is_visible(o) && (!only_selected || doc.is_selected(o.id))
}

/// Triangle meshes of the exported objects (surfaces and polysurfaces are tessellated).
pub fn collect_meshes(doc: &Document, only_selected: bool) -> Vec<NamedMesh> {
    let mut out = Vec::new();
    for o in doc.objects() {
        if !exported(doc, o, only_selected) {
            continue;
        }
        let mesh = match &o.geometry {
            Geometry::Mesh(m) => m.clone(),
            Geometry::Surface(s) => s.tessellate(16, 16),
            Geometry::Brep(b) => b.tessellate(16, 16),
            _ => continue,
        };
        if mesh.triangle_count() == 0 {
            continue;
        }
        let mut mesh = mesh;
        if mesh.normals.len() != mesh.vertices.len() {
            mesh.compute_normals();
        }
        let c = doc.display_color(o);
        out.push(NamedMesh { name: o.attributes.name.clone().unwrap_or_else(|| format!("{}_{}", o.geometry.kind(), o.id)), mesh, color: [c[0], c[1], c[2], 255] });
    }
    out
}

/// Curves of the exported objects as polylines (and surface/polysurface edges).
pub fn collect_polylines(doc: &Document, only_selected: bool) -> Vec<(String, Vec<DVec3>, [u8; 4])> {
    let mut out = Vec::new();
    for o in doc.objects() {
        if !exported(doc, o, only_selected) {
            continue;
        }
        let c = doc.display_color(o);
        let col = [c[0], c[1], c[2], 255];
        let name = o.attributes.name.clone().unwrap_or_else(|| format!("{}_{}", o.geometry.kind(), o.id));
        match &o.geometry {
            Geometry::Curve(cv) => {
                let tol = cv.bounding_box().diagonal().max(1e-6) * 5e-4;
                out.push((name, cv.tessellate_adaptive(tol), col));
            }
            Geometry::Surface(s) => {
                let tol = s.bounding_box().diagonal().max(1e-6) * 5e-4;
                for e in s.boundary_curves() {
                    out.push((name.clone(), e.tessellate_adaptive(tol), col));
                }
            }
            Geometry::Brep(b) => {
                let tol = b.bounding_box().diagonal().max(1e-6) * 5e-4;
                for f in &b.faces {
                    for e in f.edge_polylines(tol) {
                        out.push((name.clone(), e, col));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Add a mesh object on the current layer.
pub fn add_mesh(doc: &mut Document, name: &str, mut mesh: Mesh, color: Option<[u8; 4]>) -> bool {
    if mesh.triangle_count() == 0 {
        return false;
    }
    mesh.compute_normals();
    let attrs = Attributes { layer: doc.current_layer(), name: if name.is_empty() { None } else { Some(name.to_string()) }, color, ..Default::default() };
    doc.add_with(Geometry::Mesh(mesh), attrs);
    true
}

/// Add a polyline curve on the current layer.
pub fn add_polyline(doc: &mut Document, name: &str, pts: Vec<DVec3>, color: Option<[u8; 4]>) -> bool {
    if pts.len() < 2 {
        return false;
    }
    let Ok(c) = NurbsCurve::polyline(pts) else { return false };
    let attrs = Attributes { layer: doc.current_layer(), name: if name.is_empty() { None } else { Some(name.to_string()) }, color, ..Default::default() };
    doc.add_with(Geometry::Curve(c), attrs);
    true
}

/// Build a triangle mesh from polygons (fan triangulation) over a shared vertex list.
pub fn mesh_from_polygons(vertices: Vec<DVec3>, faces: &[Vec<usize>]) -> Mesh {
    let mut m = Mesh::new();
    m.vertices = vertices;
    for f in faces {
        if f.len() < 3 {
            continue;
        }
        for k in 1..f.len() - 1 {
            if f[0] < m.vertices.len() && f[k] < m.vertices.len() && f[k + 1] < m.vertices.len() {
                m.indices.extend_from_slice(&[f[0] as u32, f[k] as u32, f[k + 1] as u32]);
            }
        }
    }
    m.compute_normals();
    m
}

pub fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Floats in a whitespace-separated text.
pub fn parse_floats(s: &str) -> Vec<f64> {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect()
}

pub fn parse_ints(s: &str) -> Vec<i64> {
    s.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty()).filter_map(|t| t.parse().ok()).collect()
}

// ---------------------------------------------------------------- minimal zip (for 3MF)

/// Entries of a zip archive: (name, bytes). Stored and deflated entries are supported.
pub fn zip_read(bytes: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    use std::io::Read;
    let mut out = Vec::new();
    // walk local file headers
    let mut pos = 0usize;
    let u16 = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]) as usize;
    let u32 = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
    // prefer the central directory for sizes (data descriptors may hide them locally)
    let mut central: std::collections::HashMap<String, (usize, usize, usize, usize)> = std::collections::HashMap::new();
    if let Some(eocd) = bytes.len().checked_sub(22).and_then(|start| (0..=start).rev().find(|&i| bytes[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])) {
        let cd_off = u32(bytes, eocd + 16);
        let count = u16(bytes, eocd + 10);
        let mut p = cd_off;
        for _ in 0..count {
            if p + 46 > bytes.len() || bytes[p..p + 4] != [0x50, 0x4b, 0x01, 0x02] {
                break;
            }
            let method = u16(bytes, p + 10);
            let csize = u32(bytes, p + 20);
            let usize_ = u32(bytes, p + 24);
            let nlen = u16(bytes, p + 28);
            let elen = u16(bytes, p + 30);
            let clen = u16(bytes, p + 32);
            let off = u32(bytes, p + 42);
            let name = String::from_utf8_lossy(&bytes[p + 46..p + 46 + nlen]).to_string();
            central.insert(name, (off, method, csize, usize_));
            p += 46 + nlen + elen + clen;
        }
    }
    if !central.is_empty() {
        for (name, (off, method, csize, _)) in central {
            if off + 30 > bytes.len() || bytes[off..off + 4] != [0x50, 0x4b, 0x03, 0x04] {
                continue;
            }
            let nlen = u16(bytes, off + 26);
            let elen = u16(bytes, off + 28);
            let start = off + 30 + nlen + elen;
            let data = &bytes[start..(start + csize).min(bytes.len())];
            let content = match method {
                0 => data.to_vec(),
                8 => {
                    let mut d = flate2::read::DeflateDecoder::new(data);
                    let mut v = Vec::new();
                    d.read_to_end(&mut v).context("inflating zip entry")?;
                    v
                }
                m => bail!("zip compression method {m} is not supported"),
            };
            out.push((name, content));
        }
        return Ok(out);
    }
    while pos + 30 <= bytes.len() && bytes[pos..pos + 4] == [0x50, 0x4b, 0x03, 0x04] {
        let method = u16(bytes, pos + 8);
        let csize = u32(bytes, pos + 18);
        let nlen = u16(bytes, pos + 26);
        let elen = u16(bytes, pos + 28);
        let name = String::from_utf8_lossy(&bytes[pos + 30..pos + 30 + nlen]).to_string();
        let start = pos + 30 + nlen + elen;
        let data = &bytes[start..(start + csize).min(bytes.len())];
        let content = match method {
            0 => data.to_vec(),
            8 => {
                let mut d = flate2::read::DeflateDecoder::new(data);
                let mut v = Vec::new();
                d.read_to_end(&mut v).context("inflating zip entry")?;
                v
            }
            m => bail!("zip compression method {m} is not supported"),
        };
        out.push((name, content));
        pos = start + csize;
    }
    Ok(out)
}

/// Write a zip archive with stored entries.
pub fn zip_write(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for (name, data) in entries {
        let crc = crc32fast::hash(data);
        let off = out.len() as u32;
        let n = name.as_bytes();
        // local header
        out.extend_from_slice(&[0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(n.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(n);
        out.extend_from_slice(data);
        // central directory entry
        central.extend_from_slice(&[0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(n.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        central.extend_from_slice(&off.to_le_bytes());
        central.extend_from_slice(n);
    }
    let cd_off = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&[0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
