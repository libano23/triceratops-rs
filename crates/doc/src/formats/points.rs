//! Point files: PTS, XYZ, ASC, CSV, TXT (one point per line, optional RGB).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Result;
use glam::DVec3;

use super::{exported, parse_floats, read_text, write_text};
use crate::{Attributes, Document, Geometry};

pub fn import(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    let layer = doc.current_layer();
    let mut count = 0;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with("//") {
            continue;
        }
        let f = parse_floats(l);
        // PTS files start with a point count on its own line
        if f.len() < 3 {
            continue;
        }
        let color = if f.len() >= 6 && f[3..6].iter().all(|c| (0.0..=255.0).contains(c) && c.fract() == 0.0) && f.len() != 7 {
            Some([f[3] as u8, f[4] as u8, f[5] as u8, 255])
        } else if f.len() >= 7 {
            Some([f[4] as u8, f[5] as u8, f[6] as u8, 255])
        } else {
            None
        };
        doc.add_with(Geometry::Point(DVec3::new(f[0], f[1], f[2])), Attributes { layer, color, ..Default::default() });
        count += 1;
    }
    Ok(count)
}

pub fn export(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let ext = super::extension(path);
    let sep = if ext == "csv" { "," } else { " " };
    let mut lines: Vec<String> = Vec::new();
    let mut count = 0;
    for o in doc.objects() {
        if !exported(doc, o, only_selected) {
            continue;
        }
        let c = doc.display_color(o);
        let pts: Vec<DVec3> = match &o.geometry {
            Geometry::Point(p) => vec![*p],
            Geometry::Mesh(m) => m.vertices.clone(),
            Geometry::Curve(cv) => cv.control_points.clone(),
            _ => continue,
        };
        count += 1;
        for p in pts {
            let mut s = String::new();
            write!(s, "{}{sep}{}{sep}{}", p.x, p.y, p.z).ok();
            if ext == "pts" {
                write!(s, "{sep}0{sep}{}{sep}{}{sep}{}", c[0], c[1], c[2]).ok();
            } else if ext != "csv" {
                write!(s, "{sep}{}{sep}{}{sep}{}", c[0], c[1], c[2]).ok();
            }
            lines.push(s);
        }
    }
    let mut out = String::new();
    if ext == "pts" {
        writeln!(out, "{}", lines.len())?;
    } else if ext == "csv" {
        out.push_str("x,y,z\n");
    }
    out.push_str(&lines.join("\n"));
    out.push('\n');
    write_text(path, &out)?;
    Ok(count)
}
