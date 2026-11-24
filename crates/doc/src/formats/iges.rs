//! IGES 5.3 import and export. Entities: 100 circular arc, 102 composite curve, 106 copious
//! data (polylines), 110 line, 116 point, 126 rational B-spline curve, 128 rational B-spline
//! surface, 142 curve on surface, 144 trimmed surface, 124 transformation matrix (import).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Result};
use glam::{DAffine3, DMat3, DVec2, DVec3};
use tri_geom::{Brep, Face, NurbsCurve, NurbsSurface, Plane};

use super::{exported, read_text, write_text};
use crate::{Attributes, Document, Geometry};

// ---------------------------------------------------------------- writer

struct Writer {
    dir: Vec<String>,   // directory entry lines (two per entity)
    params: Vec<String>, // parameter lines
    next_de: usize,
}

impl Writer {
    fn new() -> Self {
        Self { dir: Vec::new(), params: Vec::new(), next_de: 1 }
    }

    /// Add an entity; returns its directory entry pointer (odd sequence number).
    fn entity(&mut self, kind: u32, form: u32, values: &[String], color: u32, status: &str, name: &str) -> usize {
        let de = self.next_de;
        let text = format!("{};", values.join(","));
        // fold the parameter text into 64-column lines, breaking at commas
        let mut lines = Vec::new();
        let mut rest = text.as_str();
        while !rest.is_empty() {
            if rest.len() <= 64 {
                lines.push(rest.to_string());
                break;
            }
            let cut = rest[..64].rfind(',').map(|i| i + 1).unwrap_or(64);
            lines.push(rest[..cut].to_string());
            rest = &rest[cut..];
        }
        let first_p = self.params.len() + 1;
        for l in &lines {
            self.params.push(format!("{:<64}{:>8}P", l, de));
        }
        let label: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
        self.dir.push(format!("{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}D", kind, first_p, 0, 0, 0, 0, 0, 0, status));
        self.dir.push(format!("{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}{:>8}D", kind, 0, color, lines.len(), form, "", "", label, 0));
        self.next_de += 2;
        de
    }

    fn finish(self, filename: &str) -> String {
        let mut out = String::new();
        writeln!(out, "{:<72}S{:>7}", "Triceratops IGES export", 1).ok();
        let now = chrono_like_now();
        let global = format!("1H,,1H;,10HTriceratops,{}H{},11HTriceratops,3H1.0,32,38,6,308,15,10HTriceratops,1.0,2,2HMM,1,0.01,15H{},0.001,1000.0,10HTriceratops,10HTriceratops,11,0,15H{};", filename.len(), filename, now, now);
        let mut g_lines = 0;
        let mut rest = global.as_str();
        while !rest.is_empty() {
            let cut = if rest.len() <= 72 { rest.len() } else { rest[..72].rfind(',').map(|i| i + 1).unwrap_or(72) };
            g_lines += 1;
            writeln!(out, "{:<72}G{:>7}", &rest[..cut], g_lines).ok();
            rest = &rest[cut..];
        }
        for (i, d) in self.dir.iter().enumerate() {
            writeln!(out, "{}{:>7}", d, i + 1).ok();
        }
        for (i, p) in self.params.iter().enumerate() {
            writeln!(out, "{}{:>7}", p, i + 1).ok();
        }
        writeln!(out, "S{:>7}G{:>7}D{:>7}P{:>7}{:<40}T{:>7}", 1, g_lines, self.dir.len(), self.params.len(), "", 1).ok();
        out
    }
}

fn chrono_like_now() -> String {
    // IGES date: YYYYMMDD.HHNNSS; without a clock crate use the epoch seconds
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = secs / 86400;
    let (y, m, d) = civil_from_days(days as i64);
    let rem = secs % 86400;
    format!("{y:04}{m:02}{d:02}.{:02}{:02}{:02}", rem / 3600, (rem % 3600) / 60, rem % 60)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn n(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}.", v as i64) } else { format!("{v}") }
}

fn iges_color(c: [u8; 4]) -> u32 {
    // nearest of the IGES standard colours
    let table: [([u8; 3], u32); 8] = [([0, 0, 0], 1), ([255, 0, 0], 2), ([0, 255, 0], 3), ([0, 0, 255], 4), ([255, 255, 0], 5), ([255, 0, 255], 6), ([0, 255, 255], 7), ([255, 255, 255], 8)];
    table.iter().min_by_key(|(rgb, _)| rgb.iter().zip(&c[..3]).map(|(a, b)| (*a as i32 - *b as i32).pow(2)).sum::<i32>()).map(|(_, i)| *i).unwrap_or(0)
}

fn curve_126(w: &mut Writer, c: &NurbsCurve, color: u32, status: &str, name: &str) -> usize {
    let k = c.control_points.len() - 1;
    let m = c.degree;
    let mut v: Vec<String> = vec!["126".into(), k.to_string(), m.to_string(), "0".into(), if c.is_closed() { "1".into() } else { "0".into() }, if c.is_rational() { "0".into() } else { "1".into() }, "0".into()];
    v.extend(c.knots.iter().map(|t| n(*t)));
    v.extend(c.weights.iter().map(|t| n(*t)));
    for p in &c.control_points {
        v.push(n(p.x));
        v.push(n(p.y));
        v.push(n(p.z));
    }
    let (t0, t1) = c.domain();
    v.push(n(t0));
    v.push(n(t1));
    v.extend(["0.".into(), "0.".into(), "1.".into()]);
    w.entity(126, 0, &v, color, status, name)
}

fn surface_128(w: &mut Writer, s: &NurbsSurface, color: u32, status: &str, name: &str) -> usize {
    let mut v: Vec<String> = vec!["128".into(), (s.count_u - 1).to_string(), (s.count_v - 1).to_string(), s.degree_u.to_string(), s.degree_v.to_string(), "0".into(), "0".into(), if s.weights.iter().all(|w| (*w - 1.0).abs() < 1e-12) { "1".into() } else { "0".into() }, "0".into(), "0".into()];
    v.extend(s.knots_u.iter().map(|t| n(*t)));
    v.extend(s.knots_v.iter().map(|t| n(*t)));
    // IGES orders weights/points with the u index varying fastest; ours is cp[i*count_v + j]
    for j in 0..s.count_v {
        for i in 0..s.count_u {
            v.push(n(s.weights[i * s.count_v + j]));
        }
    }
    for j in 0..s.count_v {
        for i in 0..s.count_u {
            let p = s.control_points[i * s.count_v + j];
            v.push(n(p.x));
            v.push(n(p.y));
            v.push(n(p.z));
        }
    }
    v.push(n(s.knots_u[s.degree_u]));
    v.push(n(s.knots_u[s.knots_u.len() - 1 - s.degree_u]));
    v.push(n(s.knots_v[s.degree_v]));
    v.push(n(s.knots_v[s.knots_v.len() - 1 - s.degree_v]));
    w.entity(128, 0, &v, color, status, name)
}

/// A trimmed face: 128 surface + per loop a 126 parameter-space polyline (as a degree-1 curve
/// with coordinates (u, v, 0)) wrapped in a 142 curve-on-surface, then a 144 trimmed surface.
fn face_144(w: &mut Writer, f: &Face, color: u32, name: &str, status: &str) -> usize {
    let dep = "00010000";
    let srf = surface_128(w, &f.surface, color, dep, name);
    if !f.is_trimmed() {
        // 144 with the outer boundary = the surface boundary
        return w.entity(144, 0, &["144".into(), srf.to_string(), "0".into(), "0".into(), "0".into()], color, status, name);
    }
    let mut loop_des = Vec::new();
    for lp in &f.loops {
        let mut pts: Vec<DVec3> = lp.iter().map(|p| DVec3::new(p.x, p.y, 0.0)).collect();
        if pts.len() < 2 {
            continue;
        }
        if (pts[0] - pts[pts.len() - 1]).length() > 1e-12 {
            pts.push(pts[0]);
        }
        let Ok(pc) = NurbsCurve::polyline(pts.clone()) else { continue };
        let pde = curve_126(w, &pc, color, dep, name);
        // 3D model curve: the parameter-space points evaluated on the surface
        let world: Vec<DVec3> = pts.iter().map(|p| f.surface.point_at(p.x, p.y)).collect();
        let cde = NurbsCurve::polyline(world).ok().map(|c| curve_126(w, &c, color, dep, name)).unwrap_or(0);
        let de142 = w.entity(142, 0, &["142".into(), "1".into(), srf.to_string(), pde.to_string(), cde.to_string(), "1".into()], color, dep, name);
        loop_des.push(de142);
    }
    let mut v = vec!["144".into(), srf.to_string(), "1".into(), (loop_des.len().saturating_sub(1)).to_string(), loop_des.first().map(|d| d.to_string()).unwrap_or("0".into())];
    v.extend(loop_des.iter().skip(1).map(|d| d.to_string()));
    w.entity(144, 0, &v, color, status, name)
}

pub fn export(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let mut w = Writer::new();
    let mut count = 0;
    for o in doc.objects() {
        if !exported(doc, o, only_selected) {
            continue;
        }
        let color = iges_color(doc.display_color(o));
        let name = o.attributes.name.clone().unwrap_or_default();
        match &o.geometry {
            Geometry::Point(p) => {
                w.entity(116, 0, &["116".into(), n(p.x), n(p.y), n(p.z), "0".into()], color, "00000000", &name);
            }
            Geometry::Curve(c) => {
                if c.degree == 1 && !c.is_rational() && c.control_points.len() == 2 {
                    let (a, b) = (c.control_points[0], c.control_points[1]);
                    w.entity(110, 0, &["110".into(), n(a.x), n(a.y), n(a.z), n(b.x), n(b.y), n(b.z)], color, "00000000", &name);
                } else {
                    curve_126(&mut w, c, color, "00000000", &name);
                }
            }
            Geometry::Surface(s) => {
                surface_128(&mut w, s, color, "00000000", &name);
            }
            Geometry::Brep(b) => {
                // faces grouped in a 402 form 7 (unordered group) so the polysurface stays together
                let mut des = Vec::new();
                let status = if b.faces.len() > 1 { "00010000" } else { "00000000" };
                for f in &b.faces {
                    des.push(face_144(&mut w, f, color, &name, status));
                }
                if des.len() > 1 {
                    let mut v = vec!["402".into(), des.len().to_string()];
                    v.extend(des.iter().map(|d| d.to_string()));
                    w.entity(402, 7, &v, color, "00000000", &name);
                }
            }
            Geometry::Mesh(m) => {
                // meshes go out as copious data polylines of their triangles (IGES has no mesh entity)
                for t in 0..m.triangle_count() {
                    let (a, b, c) = m.triangle(t);
                    let mut v = vec!["106".into(), "2".into(), "4".into()];
                    for p in [a, b, c, a] {
                        v.push(n(p.x));
                        v.push(n(p.y));
                        v.push(n(p.z));
                    }
                    w.entity(106, 12, &v, color, "00000000", &name);
                }
            }
            Geometry::Instance { .. } => continue,
        }
        count += 1;
    }
    if count == 0 {
        bail!("nothing to export");
    }
    let filename = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    write_text(path, &w.finish(&filename))?;
    Ok(count)
}

// ---------------------------------------------------------------- reader

#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
struct Entry {
    kind: u32,
    form: u32,
    params: Vec<f64>,
    status: String,
    xf: usize,
    label: String,
}

fn parse_params(text: &str, pd: char, rd: char) -> Vec<f64> {
    let body = text.split(rd).next().unwrap_or("");
    body.split(pd).map(|t| t.trim()).map(|t| t.parse::<f64>().unwrap_or(0.0)).collect()
}

pub fn import(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let mut global = String::new();
    let mut dir: Vec<String> = Vec::new();
    let mut params: Vec<(usize, String)> = Vec::new(); // (DE pointer, text)
    for line in text.lines() {
        if line.len() < 73 {
            continue;
        }
        let section = line.as_bytes()[72] as char;
        match section {
            'G' => global.push_str(&line[..72]),
            'D' => dir.push(line[..72].to_string()),
            'P' => {
                let de: usize = line[64..72].trim().parse().unwrap_or(0);
                params.push((de, line[..64].to_string()));
            }
            _ => {}
        }
    }
    if dir.is_empty() {
        bail!("no IGES directory section found");
    }
    // delimiters from the global section (defaults , and ;)
    let pd = if global.starts_with("1H") { global.chars().nth(2).unwrap_or(',') } else { ',' };
    let after = global.find(pd).map(|i| i + 1).unwrap_or(0);
    let rd = if global[after..].starts_with("1H") { global[after..].chars().nth(2).unwrap_or(';') } else { ';' };
    // glue parameter lines by DE pointer
    let mut ptext: std::collections::BTreeMap<usize, String> = std::collections::BTreeMap::new();
    for (de, t) in params {
        ptext.entry(de).or_default().push_str(&t);
    }
    let mut entries: std::collections::BTreeMap<usize, Entry> = std::collections::BTreeMap::new();
    for i in (0..dir.len()).step_by(2) {
        let l1 = &dir[i];
        let l2 = dir.get(i + 1).map(|s| s.as_str()).unwrap_or("");
        let field = |l: &str, k: usize| l.get(k * 8..k * 8 + 8).unwrap_or("").trim().to_string();
        let kind: u32 = field(l1, 0).parse().unwrap_or(0);
        let xf: usize = field(l1, 6).parse().unwrap_or(0);
        let status = field(l1, 8);
        let form: u32 = field(l2, 4).parse().unwrap_or(0);
        let label = field(l2, 7);
        let de = i + 1;
        let p = ptext.get(&de).cloned().unwrap_or_default();
        entries.insert(de, Entry { kind, form, params: parse_params(&p, pd, rd), status: format!("{status:0>8}"), xf, label });
    }
    let ctx = Reader { entries, doc_layer: doc.current_layer() };
    let mut count = 0;
    let dependent = |e: &Entry| e.status.get(2..4).is_some_and(|s| s == "01" || s == "02" || s == "03");
    for (de, e) in &ctx.entries {
        if dependent(e) {
            continue;
        }
        let name = if e.label.is_empty() { None } else { Some(e.label.clone()) };
        let attrs = Attributes { layer: ctx.doc_layer, name, ..Default::default() };
        for g in ctx.geometry(*de) {
            doc.add_with(g, attrs.clone());
            count += 1;
        }
    }
    Ok(count)
}

struct Reader {
    entries: std::collections::BTreeMap<usize, Entry>,
    doc_layer: usize,
}

impl Reader {
    fn xform(&self, de: usize) -> DAffine3 {
        let Some(e) = self.entries.get(&de) else { return DAffine3::IDENTITY };
        if e.xf == 0 {
            return DAffine3::IDENTITY;
        }
        let Some(t) = self.entries.get(&e.xf) else { return DAffine3::IDENTITY };
        if t.kind != 124 || t.params.len() < 13 {
            return DAffine3::IDENTITY;
        }
        let p = &t.params;
        let m = DMat3::from_cols(DVec3::new(p[1], p[5], p[9]), DVec3::new(p[2], p[6], p[10]), DVec3::new(p[3], p[7], p[11]));
        DAffine3::from_mat3_translation(m, DVec3::new(p[4], p[8], p[12]))
    }

    fn curve(&self, de: usize) -> Option<NurbsCurve> {
        let e = self.entries.get(&de)?;
        let p = &e.params;
        let mut c = match e.kind {
            110 if p.len() >= 7 => NurbsCurve::line(DVec3::new(p[1], p[2], p[3]), DVec3::new(p[4], p[5], p[6])).ok()?,
            100 if p.len() >= 8 => {
                let zt = p[1];
                let c = DVec3::new(p[2], p[3], zt);
                let a = DVec3::new(p[4], p[5], zt);
                let b = DVec3::new(p[6], p[7], zt);
                let r = (a - c).length();
                let a0 = (a.y - c.y).atan2(a.x - c.x);
                let mut a1 = (b.y - c.y).atan2(b.x - c.x);
                if a1 <= a0 + 1e-12 {
                    a1 += std::f64::consts::TAU;
                }
                let plane = Plane { origin: c, ..Plane::world_xy() };
                if (a1 - a0 - std::f64::consts::TAU).abs() < 1e-9 { NurbsCurve::circle(&plane, r).ok()? } else { NurbsCurve::arc(&plane, r, a0, a1).ok()? }
            }
            106 => {
                let ip = p.get(1).copied().unwrap_or(2.0) as usize;
                let cnt = p.get(2).copied().unwrap_or(0.0) as usize;
                let pts: Vec<DVec3> = match ip {
                    1 => {
                        let z = p.get(3).copied().unwrap_or(0.0);
                        p[4..].chunks(2).take(cnt).filter(|c| c.len() == 2).map(|c| DVec3::new(c[0], c[1], z)).collect()
                    }
                    2 => p[3..].chunks(3).take(cnt).filter(|c| c.len() == 3).map(|c| DVec3::new(c[0], c[1], c[2])).collect(),
                    _ => p[3..].chunks(6).take(cnt).filter(|c| c.len() == 6).map(|c| DVec3::new(c[0], c[1], c[2])).collect(),
                };
                NurbsCurve::polyline(pts).ok()?
            }
            126 => {
                let k = p.get(1).copied()? as usize;
                let m = p.get(2).copied()? as usize;
                let nk = k + m + 2;
                let knots = p.get(7..7 + nk)?.to_vec();
                let weights = p.get(7 + nk..7 + nk + k + 1)?.to_vec();
                let base = 7 + nk + k + 1;
                let cps: Vec<DVec3> = (0..=k).map(|i| DVec3::new(p[base + i * 3], p[base + i * 3 + 1], p[base + i * 3 + 2])).collect();
                if base + (k + 1) * 3 > p.len() {
                    return None;
                }
                NurbsCurve::new(m, cps, weights.iter().map(|w| if *w <= 0.0 { 1.0 } else { *w }).collect(), knots).ok()?
            }
            102 => {
                // composite: join the pieces as a polyline approximation unless they are all lines
                let cnt = p.get(1).copied().unwrap_or(0.0) as usize;
                let mut pts: Vec<DVec3> = Vec::new();
                for i in 0..cnt {
                    let sub = p.get(2 + i).copied()? as usize;
                    let c = self.curve(sub)?;
                    let tol = c.bounding_box().diagonal().max(1e-6) * 1e-3;
                    let poly = c.tessellate_adaptive(tol);
                    if pts.is_empty() {
                        pts.extend(poly);
                    } else {
                        pts.extend(poly.into_iter().skip(1));
                    }
                }
                NurbsCurve::polyline(pts).ok()?
            }
            _ => return None,
        };
        let xf = self.xform(de);
        if xf != DAffine3::IDENTITY {
            c.transform(&xf);
        }
        Some(c)
    }

    fn surface(&self, de: usize) -> Option<NurbsSurface> {
        let e = self.entries.get(&de)?;
        if e.kind != 128 {
            return None;
        }
        let p = &e.params;
        let k1 = p.get(1).copied()? as usize;
        let k2 = p.get(2).copied()? as usize;
        let m1 = p.get(3).copied()? as usize;
        let m2 = p.get(4).copied()? as usize;
        let (nu, nv) = (k1 + 1, k2 + 1);
        let (nku, nkv) = (k1 + m1 + 2, k2 + m2 + 2);
        let mut i = 10;
        let knots_u = p.get(i..i + nku)?.to_vec();
        i += nku;
        let knots_v = p.get(i..i + nkv)?.to_vec();
        i += nkv;
        let w = p.get(i..i + nu * nv)?.to_vec();
        i += nu * nv;
        if i + nu * nv * 3 > p.len() {
            return None;
        }
        // IGES: index u fastest → W(i,j) at i + j*nu; ours: cp[i*nv + j]
        let mut cps = vec![DVec3::ZERO; nu * nv];
        let mut weights = vec![1.0; nu * nv];
        for j in 0..nv {
            for u in 0..nu {
                let src = u + j * nu;
                cps[u * nv + j] = DVec3::new(p[i + src * 3], p[i + src * 3 + 1], p[i + src * 3 + 2]);
                weights[u * nv + j] = if w[src] <= 0.0 { 1.0 } else { w[src] };
            }
        }
        let mut s = NurbsSurface::new(m1, m2, nu, nv, cps, weights, knots_u, knots_v).ok()?;
        let xf = self.xform(de);
        if xf != DAffine3::IDENTITY {
            s.transform(&xf);
        }
        Some(s)
    }

    /// Parameter-space loop of a 142 curve-on-surface.
    fn uv_loop(&self, de142: usize, srf: &NurbsSurface) -> Option<Vec<DVec2>> {
        let e = self.entries.get(&de142)?;
        if e.kind != 142 {
            return None;
        }
        let bptr = e.params.get(3).copied().unwrap_or(0.0) as usize;
        let cptr = e.params.get(4).copied().unwrap_or(0.0) as usize;
        if bptr != 0 {
            if let Some(c) = self.curve_raw(bptr) {
                let tol = c.bounding_box().diagonal().max(1e-9) * 1e-3;
                let pts = c.tessellate_adaptive(tol);
                return Some(pts.iter().map(|p| DVec2::new(p.x, p.y)).collect());
            }
        }
        if cptr != 0 {
            // only the 3D curve: project it onto the surface
            let c = self.curve(cptr)?;
            let tol = c.bounding_box().diagonal().max(1e-9) * 1e-3;
            let pts = c.tessellate_adaptive(tol);
            let mut uv = Vec::with_capacity(pts.len());
            let (mut u, mut v) = (0.5, 0.5);
            for p in pts {
                let r = srf.closest_uv_from(p, u, v);
                u = r.0;
                v = r.1;
                uv.push(DVec2::new(u, v));
            }
            return Some(uv);
        }
        None
    }

    /// Curve without the transformation (parameter-space curves must stay untransformed).
    fn curve_raw(&self, de: usize) -> Option<NurbsCurve> {
        let mut tmp = self.entries.get(&de)?.clone();
        tmp.xf = 0;
        let mut copy = Reader { entries: Default::default(), doc_layer: self.doc_layer };
        copy.entries.insert(de, tmp);
        // composite/sub-curves need the full table: fall back on self when the type is 102
        if copy.entries[&de].kind == 102 {
            return self.curve(de);
        }
        copy.curve(de)
    }

    fn face(&self, de: usize) -> Option<Face> {
        let e = self.entries.get(&de)?;
        match e.kind {
            128 => Some(Face::untrimmed(self.surface(de)?)),
            144 => {
                let sptr = e.params.get(1).copied()? as usize;
                let srf = self.surface(sptr)?;
                let n1 = e.params.get(2).copied().unwrap_or(0.0) as usize;
                let n2 = e.params.get(3).copied().unwrap_or(0.0) as usize;
                let mut loops = Vec::new();
                if n1 == 1 {
                    let outer = e.params.get(4).copied().unwrap_or(0.0) as usize;
                    if let Some(l) = self.uv_loop(outer, &srf) {
                        loops.push(l);
                    }
                }
                for i in 0..n2 {
                    let inner = e.params.get(5 + i).copied().unwrap_or(0.0) as usize;
                    if let Some(l) = self.uv_loop(inner, &srf) {
                        loops.push(l);
                    }
                }
                Some(Face { surface: srf, loops })
            }
            _ => None,
        }
    }

    fn geometry(&self, de: usize) -> Vec<Geometry> {
        let Some(e) = self.entries.get(&de) else { return vec![] };
        match e.kind {
            116 if e.params.len() >= 4 => {
                let p = self.xform(de).transform_point3(DVec3::new(e.params[1], e.params[2], e.params[3]));
                vec![Geometry::Point(p)]
            }
            110 | 100 | 106 | 126 | 102 => self.curve(de).map(Geometry::Curve).into_iter().collect(),
            128 => self.surface(de).map(Geometry::Surface).into_iter().collect(),
            144 => self.face(de).map(|f| Geometry::Brep(Brep::new(vec![f]))).into_iter().collect(),
            402 | 186 | 514 | 510 | 508 => {
                // groups and B-rep solids: gather every face reachable from the entity
                let mut faces = Vec::new();
                self.collect_faces(de, &mut faces, 0);
                if faces.is_empty() { vec![] } else { vec![Geometry::Brep(Brep::new(faces))] }
            }
            _ => vec![],
        }
    }

    fn collect_faces(&self, de: usize, out: &mut Vec<Face>, depth: usize) {
        if depth > 6 {
            return;
        }
        let Some(e) = self.entries.get(&de) else { return };
        match e.kind {
            128 | 144 => {
                if let Some(f) = self.face(de) {
                    out.push(f);
                }
            }
            402 => {
                let n = e.params.get(1).copied().unwrap_or(0.0) as usize;
                for i in 0..n {
                    if let Some(p) = e.params.get(2 + i) {
                        self.collect_faces(*p as usize, out, depth + 1);
                    }
                }
            }
            186 => {
                if let Some(p) = e.params.get(1) {
                    self.collect_faces(*p as usize, out, depth + 1);
                }
            }
            514 => {
                let n = e.params.get(1).copied().unwrap_or(0.0) as usize;
                for i in 0..n {
                    if let Some(p) = e.params.get(2 + i * 2) {
                        self.collect_faces(*p as usize, out, depth + 1);
                    }
                }
            }
            510 => {
                // face entity: surface pointer; trims (508 loops) are skipped, the surface is used untrimmed
                if let Some(p) = e.params.get(1) {
                    self.collect_faces(*p as usize, out, depth + 1);
                }
            }
            _ => {}
        }
    }
}

