//! VDA-FS (German automotive surface exchange): POINT, PSET, CURVE (polynomial segments),
//! SURF (polynomial patches). Export writes curves and surfaces as Bezier segments.

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;
use glam::DVec3;
use tri_geom::{NurbsCurve, NurbsSurface};

use super::{exported, parse_floats, read_text, write_text};
use crate::{Attributes, Document, Geometry};

/// Bezier segments (degree p, p+1 points each) of a NURBS curve via knot insertion.
fn bezier_segments(c: &NurbsCurve) -> Vec<Vec<DVec3>> {
    let (t0, t1) = c.domain();
    let mut spans: Vec<f64> = c.knots.iter().copied().filter(|k| *k > t0 && *k < t1).collect();
    spans.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    let mut breaks = vec![t0];
    breaks.extend(spans);
    breaks.push(t1);
    // sample each span and fit a Bezier of the curve degree by interpolation at Chebyshev-like params
    let p = c.degree.max(1);
    let mut out = Vec::new();
    for w in breaks.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = p + 1;
        let params: Vec<f64> = (0..n).map(|i| i as f64 / (n - 1) as f64).collect();
        let pts: Vec<DVec3> = params.iter().map(|s| c.point_at(a + (b - a) * s)).collect();
        // solve Bernstein interpolation
        let mut m = vec![vec![0.0; n]; n];
        for (r, s) in params.iter().enumerate() {
            for (k, cell) in m[r].iter_mut().enumerate() {
                *cell = binom(p, k) as f64 * s.powi(k as i32) * (1.0 - s).powi((p - k) as i32);
            }
        }
        let inv = invert(m);
        let cps: Vec<DVec3> = (0..n).map(|k| (0..n).fold(DVec3::ZERO, |acc, r| acc + pts[r] * inv[k][r])).collect();
        out.push(cps);
    }
    out
}

fn binom(n: usize, k: usize) -> usize {
    (0..k).fold(1, |acc, i| acc * (n - i) / (i + 1))
}

fn invert(mut m: Vec<Vec<f64>>) -> Vec<Vec<f64>> {
    let n = m.len();
    let mut inv: Vec<Vec<f64>> = (0..n).map(|i| (0..n).map(|j| if i == j { 1.0 } else { 0.0 }).collect()).collect();
    for col in 0..n {
        let piv = (col..n).max_by(|a, b| m[*a][col].abs().partial_cmp(&m[*b][col].abs()).unwrap()).unwrap();
        m.swap(col, piv);
        inv.swap(col, piv);
        let d = m[col][col];
        if d.abs() < 1e-14 {
            continue;
        }
        for j in 0..n {
            m[col][j] /= d;
            inv[col][j] /= d;
        }
        for r in 0..n {
            if r != col {
                let f = m[r][col];
                for j in 0..n {
                    m[r][j] -= f * m[col][j];
                    inv[r][j] -= f * inv[col][j];
                }
            }
        }
    }
    inv
}

/// Convert Bernstein control points of a segment to power-basis coefficients a0..ap.
fn bezier_to_power(cps: &[DVec3]) -> Vec<DVec3> {
    let p = cps.len() - 1;
    (0..=p).map(|j| {
        let mut sum = DVec3::ZERO;
        for i in 0..=j {
            let sign = if (j - i) % 2 == 0 { 1.0 } else { -1.0 };
            sum += cps[i] * (sign * binom(p, j) as f64 * binom(j, i) as f64);
        }
        sum
    }).collect()
}

fn power_to_bezier(coef: &[DVec3]) -> Vec<DVec3> {
    let p = coef.len() - 1;
    (0..=p).map(|k| {
        let mut sum = DVec3::ZERO;
        for i in 0..=k {
            sum += coef[i] * (binom(k, i) as f64 / binom(p, i) as f64);
        }
        sum
    }).collect()
}

pub fn export(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let mut s = String::from("VDAFS   = HEADER /  4\n$$ Triceratops VDA-FS 2.0 export\n$$ UNITS mm\n$$\n$$\n");
    let mut count = 0;
    let mut n = 0;
    for o in doc.objects() {
        if !exported(doc, o, only_selected) {
            continue;
        }
        n += 1;
        match &o.geometry {
            Geometry::Point(p) => {
                writeln!(s, "P{n:<7}= POINT / {}, {}, {}", p.x, p.y, p.z)?;
                count += 1;
            }
            Geometry::Curve(c) => {
                let segs = bezier_segments(c);
                let mut vals: Vec<String> = vec![segs.len().to_string()];
                vals.extend((0..=segs.len()).map(|i| i.to_string()));
                for seg in &segs {
                    vals.push((seg.len()).to_string());
                    let coef = bezier_to_power(seg);
                    for axis in 0..3 {
                        vals.extend(coef.iter().map(|c| c[axis].to_string()));
                    }
                }
                writeln!(s, "C{n:<7}= CURVE / {}", vals.join(", "))?;
                count += 1;
            }
            Geometry::Surface(srf) => {
                write_surface(&mut s, n, srf)?;
                count += 1;
            }
            Geometry::Brep(b) => {
                for (k, f) in b.faces.iter().enumerate() {
                    write_surface_named(&mut s, &format!("S{n}F{k}"), &f.surface)?;
                }
                count += 1;
            }
            _ => {}
        }
    }
    s.push_str("END     = END\n");
    write_text(path, &fold(&s))?;
    Ok(count)
}

fn write_surface(s: &mut String, n: usize, srf: &NurbsSurface) -> Result<()> {
    write_surface_named(s, &format!("S{n}"), srf)
}

fn write_surface_named(s: &mut String, name: &str, srf: &NurbsSurface) -> Result<()> {
    // one patch per knot span, Bezier form via sampling + interpolation in each direction
    let (pu, pv) = (srf.degree_u.max(1), srf.degree_v.max(1));
    let (u0, u1) = (srf.knots_u[pu], srf.knots_u[srf.knots_u.len() - 1 - pu]);
    let (v0, v1) = (srf.knots_v[pv], srf.knots_v[srf.knots_v.len() - 1 - pv]);
    let mut bu: Vec<f64> = vec![u0];
    bu.extend(srf.knots_u.iter().copied().filter(|k| *k > u0 && *k < u1));
    bu.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    bu.push(u1);
    let mut bv: Vec<f64> = vec![v0];
    bv.extend(srf.knots_v.iter().copied().filter(|k| *k > v0 && *k < v1));
    bv.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    bv.push(v1);
    let mut vals: Vec<String> = vec![(bu.len() - 1).to_string(), (bv.len() - 1).to_string()];
    vals.extend((0..bu.len()).map(|i| i.to_string()));
    vals.extend((0..bv.len()).map(|i| i.to_string()));
    let (nu, nv) = (pu + 1, pv + 1);
    let su: Vec<f64> = (0..nu).map(|i| i as f64 / (nu - 1) as f64).collect();
    let sv: Vec<f64> = (0..nv).map(|i| i as f64 / (nv - 1) as f64).collect();
    let bern = |p: usize, params: &[f64]| -> Vec<Vec<f64>> {
        let m: Vec<Vec<f64>> = params.iter().map(|s| (0..=p).map(|k| binom(p, k) as f64 * s.powi(k as i32) * (1.0 - s).powi((p - k) as i32)).collect()).collect();
        invert(m)
    };
    let (iu, iv) = (bern(pu, &su), bern(pv, &sv));
    for wu in bu.windows(2) {
        for wv in bv.windows(2) {
            // sample grid
            let grid: Vec<Vec<DVec3>> = su.iter().map(|a| sv.iter().map(|b| srf.point_at(wu[0] + (wu[1] - wu[0]) * a, wv[0] + (wv[1] - wv[0]) * b)).collect()).collect();
            // Bezier control net: C = Iu * G * Iv^T
            let mut cps = vec![vec![DVec3::ZERO; nv]; nu];
            for i in 0..nu {
                for j in 0..nv {
                    let mut acc = DVec3::ZERO;
                    for r in 0..nu {
                        for c in 0..nv {
                            acc += grid[r][c] * (iu[i][r] * iv[j][c]);
                        }
                    }
                    cps[i][j] = acc;
                }
            }
            // power basis in both directions
            let rows: Vec<Vec<DVec3>> = cps.iter().map(|row| bezier_to_power(row)).collect();
            let mut coef = vec![vec![DVec3::ZERO; nv]; nu];
            for j in 0..nv {
                let col: Vec<DVec3> = rows.iter().map(|r| r[j]).collect();
                let pc = bezier_to_power(&col);
                for i in 0..nu {
                    coef[i][j] = pc[i];
                }
            }
            vals.push(nu.to_string());
            vals.push(nv.to_string());
            for axis in 0..3 {
                for i in 0..nu {
                    for j in 0..nv {
                        vals.push(coef[i][j][axis].to_string());
                    }
                }
            }
        }
    }
    writeln!(s, "{name:<8}= SURF / {}", vals.join(", "))?;
    Ok(())
}

/// Fold long logical lines into the 72-column VDA-FS layout (continuation lines are indented).
fn fold(s: &str) -> String {
    let mut out = String::new();
    for line in s.lines() {
        if line.len() <= 72 {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let mut rest = line;
        let mut first = true;
        while !rest.is_empty() {
            let width = if first { 72 } else { 62 };
            let cut = if rest.len() <= width { rest.len() } else { rest[..width].rfind(',').map(|i| i + 1).unwrap_or(width) };
            if !first {
                out.push_str("          ");
            }
            out.push_str(&rest[..cut]);
            out.push('\n');
            rest = &rest[cut..];
            first = false;
        }
    }
    out
}

pub fn import(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    // join logical statements: "NAME = TYPE / values" possibly spanning lines until the next "name ="
    let mut statements: Vec<(String, String)> = Vec::new();
    for raw in text.lines() {
        let line = if raw.len() > 72 { &raw[..72] } else { raw };
        let line = line.trim_end();
        if line.trim_start().starts_with("$$") || line.trim().is_empty() {
            continue;
        }
        if line.len() > 8 && &line[8..9] == "=" || line.contains(" = ") && !line.starts_with(' ') {
            let (name, rest) = line.split_once('=').unwrap_or((line, ""));
            statements.push((name.trim().to_string(), rest.trim().to_string()));
        } else if let Some(last) = statements.last_mut() {
            last.1.push(' ');
            last.1.push_str(line.trim());
        }
    }
    let layer = doc.current_layer();
    let mut count = 0;
    // faces of one polysurface are written as S<n>F<k>: collect them and add them together
    let mut pending: Option<(String, Vec<tri_geom::Face>)> = None;
    let flush = |pending: &mut Option<(String, Vec<tri_geom::Face>)>, doc: &mut Document, count: &mut usize| {
        if let Some((name, faces)) = pending.take() {
            if !faces.is_empty() {
                doc.add_with(Geometry::Brep(tri_geom::Brep::new(faces)), Attributes { layer, name: Some(name), ..Default::default() });
                *count += 1;
            }
        }
    };
    for (name, body) in statements {
        let Some((kind, vals)) = body.split_once('/') else { continue };
        let kind = kind.trim().to_ascii_uppercase();
        let f = parse_floats(vals);
        let attrs = Attributes { layer, name: Some(name.clone()), ..Default::default() };
        match kind.as_str() {
            "POINT" if f.len() >= 3 => {
                doc.add_with(Geometry::Point(DVec3::new(f[0], f[1], f[2])), attrs);
                count += 1;
            }
            "PSET" => {
                let n = f.first().copied().unwrap_or(0.0) as usize;
                for c in f[1..].chunks(3).take(n) {
                    if c.len() == 3 {
                        doc.add_with(Geometry::Point(DVec3::new(c[0], c[1], c[2])), attrs.clone());
                        count += 1;
                    }
                }
            }
            "CURVE" => {
                let nseg = f.first().copied().unwrap_or(0.0) as usize;
                let mut p = 1 + nseg + 1;
                let mut pts: Vec<DVec3> = Vec::new();
                for _ in 0..nseg {
                    let Some(&ord) = f.get(p) else { break };
                    let ord = ord as usize;
                    p += 1;
                    if p + ord * 3 > f.len() {
                        break;
                    }
                    let coef: Vec<DVec3> = (0..ord).map(|i| DVec3::new(f[p + i], f[p + ord + i], f[p + 2 * ord + i])).collect();
                    p += ord * 3;
                    let bez = power_to_bezier(&coef);
                    // sample the segment (exact for lines)
                    let steps = if ord <= 2 { 1 } else { 12 };
                    for k in (if pts.is_empty() { 0 } else { 1 })..=steps {
                        let t = k as f64 / steps as f64;
                        pts.push(eval_power(&coef, t));
                    }
                    let _ = bez;
                }
                if pts.len() >= 2 {
                    if let Ok(c) = NurbsCurve::polyline(pts) {
                        doc.add_with(Geometry::Curve(c), attrs);
                        count += 1;
                    }
                }
            }
            "SURF" => {
                let (nu, nv) = (f.first().copied().unwrap_or(0.0) as usize, f.get(1).copied().unwrap_or(0.0) as usize);
                let mut p = 2 + (nu + 1) + (nv + 1);
                let mut faces = Vec::new();
                for _ in 0..nu * nv {
                    let (Some(&ou), Some(&ov)) = (f.get(p), f.get(p + 1)) else { break };
                    let (ou, ov) = (ou as usize, ov as usize);
                    p += 2;
                    if p + ou * ov * 3 > f.len() {
                        break;
                    }
                    let mut coef = vec![vec![DVec3::ZERO; ov]; ou];
                    for axis in 0..3 {
                        for i in 0..ou {
                            for j in 0..ov {
                                coef[i][j][axis] = f[p + axis * ou * ov + i * ov + j];
                            }
                        }
                    }
                    p += ou * ov * 3;
                    // power → Bezier control net
                    let rows: Vec<Vec<DVec3>> = coef.iter().map(|r| power_to_bezier(r)).collect();
                    let mut grid = vec![vec![DVec3::ZERO; ov]; ou];
                    for j in 0..ov {
                        let col: Vec<DVec3> = rows.iter().map(|r| r[j]).collect();
                        let bz = power_to_bezier(&col);
                        for i in 0..ou {
                            grid[i][j] = bz[i];
                        }
                    }
                    if let Ok(srf) = NurbsSurface::from_grid(ou - 1, ov - 1, grid) {
                        faces.push(tri_geom::Face::untrimmed(srf));
                    }
                }
                let group = name.rfind('F').filter(|i| *i > 0 && name[*i + 1..].chars().all(|c| c.is_ascii_digit()) && name.starts_with('S')).map(|i| name[..i].to_string());
                if let Some(g) = group {
                    if pending.as_ref().map_or(false, |(n, _)| *n != g) {
                        flush(&mut pending, doc, &mut count);
                    }
                    pending.get_or_insert_with(|| (g.clone(), Vec::new())).1.extend(faces);
                    continue;
                }
                flush(&mut pending, doc, &mut count);
                if faces.len() == 1 {
                    doc.add_with(Geometry::Surface(faces.remove(0).surface), attrs);
                    count += 1;
                } else if !faces.is_empty() {
                    doc.add_with(Geometry::Brep(tri_geom::Brep::new(faces)), attrs);
                    count += 1;
                }
            }
            _ => {}
        }
    }
    flush(&mut pending, doc, &mut count);
    Ok(count)
}

fn eval_power(coef: &[DVec3], t: f64) -> DVec3 {
    coef.iter().rev().fold(DVec3::ZERO, |acc, c| acc * t + *c)
}
