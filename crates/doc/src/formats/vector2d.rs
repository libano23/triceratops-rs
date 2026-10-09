//! 2D vector formats: SVG (import/export), PDF, EPS and AI (export). Curves are projected onto
//! the world XY plane (the Top view).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use glam::DVec3;
use tri_geom::{BoundingBox, NurbsCurve, Plane};

use super::{add_polyline, collect_polylines, parse_floats, read_text, write_text, xml_escape};
use crate::{Attributes, Document, Geometry};

fn fmt(v: f64) -> String {
    let r = (v * 1000.0).round() / 1000.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

fn bounds(polys: &[(String, Vec<DVec3>, [u8; 4])]) -> BoundingBox {
    let mut bb = BoundingBox::EMPTY;
    for (_, p, _) in polys {
        for q in p {
            bb.add_point(*q);
        }
    }
    if bb.is_empty() {
        bb.add_point(DVec3::ZERO);
        bb.add_point(DVec3::new(100.0, 100.0, 0.0));
    }
    bb
}

// ---------------------------------------------------------------- SVG

pub fn export_svg(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let polys = collect_polylines(doc, only_selected);
    let bb = bounds(&polys);
    let (w, h) = ((bb.max.x - bb.min.x).max(1e-3), (bb.max.y - bb.min.y).max(1e-3));
    let mut s = String::new();
    writeln!(s, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}mm\" height=\"{}mm\" viewBox=\"{} {} {} {}\">\n<!-- Triceratops -->\n<g fill=\"none\" stroke-width=\"0.25\" stroke-linecap=\"round\" stroke-linejoin=\"round\" transform=\"matrix(1 0 0 -1 0 {})\">", fmt(w), fmt(h), fmt(bb.min.x), fmt(-bb.max.y), fmt(w), fmt(h), fmt(0.0))?;
    for (name, p, c) in &polys {
        if p.len() < 2 {
            continue;
        }
        let d: Vec<String> = p.iter().enumerate().map(|(i, q)| format!("{}{} {}", if i == 0 { "M" } else { "L" }, fmt(q.x), fmt(q.y))).collect();
        let closed = (p[0] - p[p.len() - 1]).length() < 1e-9;
        writeln!(s, "<path id=\"{}\" stroke=\"#{:02x}{:02x}{:02x}\" d=\"{}{}\"/>", xml_escape(name), c[0], c[1], c[2], d.join(" "), if closed { " Z" } else { "" })?;
    }
    s.push_str("</g>\n</svg>\n");
    write_text(path, &s)?;
    Ok(polys.len())
}

pub fn import_svg(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let xml = roxmltree::Document::parse(&text).context("parsing SVG")?;
    let layer = doc.current_layer();
    let mut count = 0;
    let add = |doc: &mut Document, pts: Vec<DVec3>, color: Option<[u8; 4]>, count: &mut usize| {
        *count += add_polyline(doc, "", pts, color) as usize;
    };
    let height = xml.root_element().attribute("viewBox").map(|v| parse_floats(v)).filter(|v| v.len() == 4).map(|v| v[1] + v[3]).or_else(|| xml.root_element().attribute("height").map(|h| parse_floats(h).first().copied().unwrap_or(0.0))).unwrap_or(0.0);
    let flip = |p: DVec3| DVec3::new(p.x, height - p.y, 0.0);
    for n in xml.descendants().filter(|n| n.is_element()) {
        let color = n.attribute("stroke").and_then(parse_color);
        match n.tag_name().name() {
            "path" => {
                for sub in parse_path(n.attribute("d").unwrap_or("")) {
                    add(doc, sub.into_iter().map(flip).collect(), color, &mut count);
                }
            }
            "polyline" | "polygon" => {
                let f = parse_floats(n.attribute("points").unwrap_or(""));
                let mut pts: Vec<DVec3> = f.chunks(2).filter(|c| c.len() == 2).map(|c| flip(DVec3::new(c[0], c[1], 0.0))).collect();
                if n.tag_name().name() == "polygon" && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                add(doc, pts, color, &mut count);
            }
            "line" => {
                let g = |a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                add(doc, vec![flip(DVec3::new(g("x1"), g("y1"), 0.0)), flip(DVec3::new(g("x2"), g("y2"), 0.0))], color, &mut count);
            }
            "rect" => {
                let g = |a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let (x, y, w, h) = (g("x"), g("y"), g("width"), g("height"));
                add(doc, [(x, y), (x + w, y), (x + w, y + h), (x, y + h), (x, y)].iter().map(|(a, b)| flip(DVec3::new(*a, *b, 0.0))).collect(), color, &mut count);
            }
            "circle" | "ellipse" => {
                let g = |a: &str| n.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
                let (cx, cy) = (g("cx"), g("cy"));
                let (rx, ry) = if n.tag_name().name() == "circle" { (g("r"), g("r")) } else { (g("rx"), g("ry")) };
                let plane = Plane { origin: flip(DVec3::new(cx, cy, 0.0)), ..Plane::world_xy() };
                let c = if (rx - ry).abs() < 1e-9 { NurbsCurve::circle(&plane, rx) } else { NurbsCurve::ellipse(&plane, rx, ry) };
                if let Ok(c) = c {
                    doc.add_with(Geometry::Curve(c), Attributes { layer, color, ..Default::default() });
                    count += 1;
                }
            }
            _ => {}
        }
    }
    Ok(count)
}

fn parse_color(s: &str) -> Option<[u8; 4]> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return Some(if hex.len() == 3 { [((v >> 8) & 0xf) as u8 * 17, ((v >> 4) & 0xf) as u8 * 17, (v & 0xf) as u8 * 17, 255] } else { [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255] });
    }
    if let Some(rgb) = s.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let f = parse_floats(rgb);
        if f.len() >= 3 {
            return Some([f[0] as u8, f[1] as u8, f[2] as u8, 255]);
        }
    }
    match s { "black" => Some([0, 0, 0, 255]), "red" => Some([255, 0, 0, 255]), "green" => Some([0, 128, 0, 255]), "blue" => Some([0, 0, 255, 255]), "white" => Some([255, 255, 255, 255]), _ => None }
}

/// Parse an SVG path into polylines (curves are flattened with 16 segments each).
pub fn parse_path(d: &str) -> Vec<Vec<DVec3>> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, tokens: &mut Vec<String>| {
        if !cur.is_empty() {
            tokens.push(std::mem::take(cur));
        }
    };
    let mut prev = ' ';
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() && ch != 'e' && ch != 'E' {
            flush(&mut cur, &mut tokens);
            tokens.push(ch.to_string());
        } else if ch == ',' || ch.is_whitespace() {
            flush(&mut cur, &mut tokens);
        } else if ch == '-' && !cur.is_empty() && prev != 'e' && prev != 'E' {
            flush(&mut cur, &mut tokens);
            cur.push(ch);
        } else if ch == '.' && cur.contains('.') {
            flush(&mut cur, &mut tokens);
            cur.push(ch);
        } else {
            cur.push(ch);
        }
        prev = ch;
    }
    flush(&mut cur, &mut tokens);
    let mut out: Vec<Vec<DVec3>> = Vec::new();
    let mut path: Vec<DVec3> = Vec::new();
    let mut pos = DVec3::ZERO;
    let mut start = DVec3::ZERO;
    let mut last_ctrl: Option<DVec3> = None;
    let mut cmd = 'M';
    let mut i = 0;
    let num = |tokens: &[String], i: &mut usize| -> Option<f64> {
        let v = tokens.get(*i)?.parse::<f64>().ok()?;
        *i += 1;
        Some(v)
    };
    while i < tokens.len() {
        let t = &tokens[i];
        if t.len() == 1 && t.chars().next().unwrap().is_ascii_alphabetic() {
            cmd = t.chars().next().unwrap();
            i += 1;
            if cmd == 'Z' || cmd == 'z' {
                if !path.is_empty() {
                    path.push(start);
                    out.push(std::mem::take(&mut path));
                }
                pos = start;
                continue;
            }
        }
        let rel = cmd.is_ascii_lowercase();
        let base = if rel { pos } else { DVec3::ZERO };
        match cmd.to_ascii_uppercase() {
            'M' => {
                let (Some(x), Some(y)) = (num(&tokens, &mut i), num(&tokens, &mut i)) else { break };
                if path.len() > 1 {
                    out.push(std::mem::take(&mut path));
                } else {
                    path.clear();
                }
                pos = base + DVec3::new(x, y, 0.0);
                start = pos;
                path.push(pos);
                cmd = if rel { 'l' } else { 'L' };
            }
            'L' => {
                let (Some(x), Some(y)) = (num(&tokens, &mut i), num(&tokens, &mut i)) else { break };
                pos = base + DVec3::new(x, y, 0.0);
                path.push(pos);
            }
            'H' => {
                let Some(x) = num(&tokens, &mut i) else { break };
                pos = DVec3::new(base.x + x, pos.y, 0.0);
                path.push(pos);
            }
            'V' => {
                let Some(y) = num(&tokens, &mut i) else { break };
                pos = DVec3::new(pos.x, base.y + y, 0.0);
                path.push(pos);
            }
            'C' | 'S' | 'Q' | 'T' => {
                let up = cmd.to_ascii_uppercase();
                let (p1, p2, p3) = match up {
                    'C' => {
                        let v: Vec<f64> = (0..6).filter_map(|_| num(&tokens, &mut i)).collect();
                        if v.len() < 6 { break }
                        (base + DVec3::new(v[0], v[1], 0.0), base + DVec3::new(v[2], v[3], 0.0), base + DVec3::new(v[4], v[5], 0.0))
                    }
                    'S' => {
                        let v: Vec<f64> = (0..4).filter_map(|_| num(&tokens, &mut i)).collect();
                        if v.len() < 4 { break }
                        let p1 = last_ctrl.map(|c| pos * 2.0 - c).unwrap_or(pos);
                        (p1, base + DVec3::new(v[0], v[1], 0.0), base + DVec3::new(v[2], v[3], 0.0))
                    }
                    'Q' => {
                        let v: Vec<f64> = (0..4).filter_map(|_| num(&tokens, &mut i)).collect();
                        if v.len() < 4 { break }
                        let q = base + DVec3::new(v[0], v[1], 0.0);
                        let p3 = base + DVec3::new(v[2], v[3], 0.0);
                        (pos + (q - pos) * (2.0 / 3.0), p3 + (q - p3) * (2.0 / 3.0), p3)
                    }
                    _ => {
                        let v: Vec<f64> = (0..2).filter_map(|_| num(&tokens, &mut i)).collect();
                        if v.len() < 2 { break }
                        let q = last_ctrl.map(|c| pos * 2.0 - c).unwrap_or(pos);
                        let p3 = base + DVec3::new(v[0], v[1], 0.0);
                        (pos + (q - pos) * (2.0 / 3.0), p3 + (q - p3) * (2.0 / 3.0), p3)
                    }
                };
                for k in 1..=16 {
                    let t = k as f64 / 16.0;
                    let mt = 1.0 - t;
                    path.push(pos * (mt * mt * mt) + p1 * (3.0 * mt * mt * t) + p2 * (3.0 * mt * t * t) + p3 * (t * t * t));
                }
                last_ctrl = Some(if up == 'Q' || up == 'T' { p2 } else { p2 });
                pos = p3;
                continue;
            }
            'A' => {
                let v: Vec<f64> = (0..7).filter_map(|_| num(&tokens, &mut i)).collect();
                if v.len() < 7 { break }
                let end = base + DVec3::new(v[5], v[6], 0.0);
                arc_to(&mut path, pos, v[0], v[1], v[2], v[3] != 0.0, v[4] != 0.0, end);
                pos = end;
            }
            _ => {
                i += 1;
            }
        }
        last_ctrl = None;
    }
    if path.len() > 1 {
        out.push(path);
    }
    out
}

/// SVG elliptical arc (endpoint parameterization) flattened to 24 segments.
fn arc_to(path: &mut Vec<DVec3>, from: DVec3, rx: f64, ry: f64, rot_deg: f64, large: bool, sweep: bool, to: DVec3) {
    if rx.abs() < 1e-12 || ry.abs() < 1e-12 {
        path.push(to);
        return;
    }
    let phi = rot_deg.to_radians();
    let (sp, cp) = phi.sin_cos();
    let dx = (from.x - to.x) / 2.0;
    let dy = (from.y - to.y) / 2.0;
    let x1 = cp * dx + sp * dy;
    let y1 = -sp * dx + cp * dy;
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    let lambda = x1 * x1 / (rx * rx) + y1 * y1 / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let num = (rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1).max(0.0);
    let den = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut coef = if den > 0.0 { (num / den).sqrt() } else { 0.0 };
    if large == sweep {
        coef = -coef;
    }
    let cx1 = coef * rx * y1 / ry;
    let cy1 = -coef * ry * x1 / rx;
    let cx = cp * cx1 - sp * cy1 + (from.x + to.x) / 2.0;
    let cy = sp * cx1 + cp * cy1 + (from.y + to.y) / 2.0;
    let ang = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let d = ux * vx + uy * vy;
        let l = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut a = (d / l).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let t1 = ang(1.0, 0.0, (x1 - cx1) / rx, (y1 - cy1) / ry);
    let mut dt = ang((x1 - cx1) / rx, (y1 - cy1) / ry, (-x1 - cx1) / rx, (-y1 - cy1) / ry);
    if !sweep && dt > 0.0 {
        dt -= std::f64::consts::TAU;
    } else if sweep && dt < 0.0 {
        dt += std::f64::consts::TAU;
    }
    for k in 1..=24 {
        let t = t1 + dt * k as f64 / 24.0;
        let (st, ct) = t.sin_cos();
        let x = cp * rx * ct - sp * ry * st + cx;
        let y = sp * rx * ct + cp * ry * st + cy;
        path.push(DVec3::new(x, y, 0.0));
    }
}

// ---------------------------------------------------------------- PDF / EPS / AI

/// Minimal single-page PDF with the curves as stroked paths (1 unit = 1 mm).
pub fn export_pdf(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let polys = collect_polylines(doc, only_selected);
    let bb = bounds(&polys);
    let scale = 72.0 / 25.4; // mm → points
    let (w, h) = ((bb.max.x - bb.min.x).max(1.0) * scale, (bb.max.y - bb.min.y).max(1.0) * scale);
    let mut content = String::from("0.5 w 1 J 1 j\n");
    for (_, p, c) in &polys {
        if p.len() < 2 {
            continue;
        }
        writeln!(content, "{} {} {} RG", c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0)?;
        for (i, q) in p.iter().enumerate() {
            writeln!(content, "{} {} {}", fmt((q.x - bb.min.x) * scale), fmt((q.y - bb.min.y) * scale), if i == 0 { "m" } else { "l" })?;
        }
        content.push_str("S\n");
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents 4 0 R /Resources << >> >>", fmt(w), fmt(h)),
        format!("<< /Length {} >>\nstream\n{}endstream", content.len(), content),
        "<< /Producer (Triceratops) >>".to_string(),
    ];
    let mut out = String::from("%PDF-1.4\n%\u{e2}\u{e3}\u{cf}\u{d3}\n");
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(out.len());
        writeln!(out, "{} 0 obj\n{}\nendobj", i + 1, o)?;
    }
    let xref = out.len();
    writeln!(out, "xref\n0 {}\n0000000000 65535 f ", objects.len() + 1)?;
    for o in &offsets {
        writeln!(out, "{o:010} 00000 n ")?;
    }
    writeln!(out, "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{}\n%%EOF", objects.len() + 1, xref)?;
    std::fs::write(path, out.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
    Ok(polys.len())
}

/// Encapsulated PostScript (also used for .ai, which Illustrator opens as PostScript).
pub fn export_eps(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let polys = collect_polylines(doc, only_selected);
    let bb = bounds(&polys);
    let scale = 72.0 / 25.4;
    let (w, h) = ((bb.max.x - bb.min.x).max(1.0) * scale, (bb.max.y - bb.min.y).max(1.0) * scale);
    let mut s = String::new();
    writeln!(s, "%!PS-Adobe-3.0 EPSF-3.0\n%%Creator: Triceratops\n%%BoundingBox: 0 0 {} {}\n%%EndComments\n0.5 setlinewidth 1 setlinecap 1 setlinejoin", w.ceil() as i64, h.ceil() as i64)?;
    for (_, p, c) in &polys {
        if p.len() < 2 {
            continue;
        }
        writeln!(s, "{} {} {} setrgbcolor newpath", c[0] as f64 / 255.0, c[1] as f64 / 255.0, c[2] as f64 / 255.0)?;
        for (i, q) in p.iter().enumerate() {
            writeln!(s, "{} {} {}", fmt((q.x - bb.min.x) * scale), fmt((q.y - bb.min.y) * scale), if i == 0 { "moveto" } else { "lineto" })?;
        }
        s.push_str("stroke\n");
    }
    s.push_str("showpage\n%%EOF\n");
    write_text(path, &s)?;
    Ok(polys.len())
}
