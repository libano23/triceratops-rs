//! STEP (ISO 10303-21, AP214/AP203) import and export. Reads B-spline curves and surfaces,
//! analytic surfaces (plane, cylinder, cone, sphere, torus, revolution, extrusion), lines,
//! circles, ellipses, polylines, trimmed curves, and the B-rep topology (faces, loops, edges,
//! shells, solids). Writes every object as NURBS geometry with face/loop/edge topology.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{bail, Result};
use glam::{DVec2, DVec3};
use tri_geom::{Brep, Face, NurbsCurve, NurbsSurface, Plane};

use super::{exported, read_text, write_text};
use crate::{Attributes, Document, Geometry};

// ---------------------------------------------------------------- Part 21 parser

#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Ref(usize),
    Num(f64),
    Str(String),
    Enum(String),
    List(Vec<Arg>),
    Typed(String, Vec<Arg>),
    Null,
}

impl Arg {
    fn num(&self) -> Option<f64> {
        match self { Arg::Num(v) => Some(*v), Arg::Typed(_, a) => a.first().and_then(|a| a.num()), _ => None }
    }
    fn r(&self) -> Option<usize> {
        if let Arg::Ref(i) = self { Some(*i) } else { None }
    }
    fn list(&self) -> &[Arg] {
        if let Arg::List(l) = self { l } else { &[] }
    }
    fn flag(&self) -> bool {
        matches!(self, Arg::Enum(e) if e == "T")
    }
}

#[derive(Clone, Debug)]
pub struct Entity {
    /// Simple entity type, or every type of a complex entity joined with '+'.
    pub kind: String,
    /// Arguments; for a complex entity the arguments of every part concatenated in order,
    /// with `parts` recording (type, start, len).
    pub args: Vec<Arg>,
    pub parts: Vec<(String, usize, usize)>,
}

impl Entity {
    fn part(&self, name: &str) -> Option<&[Arg]> {
        self.parts.iter().find(|(n, _, _)| n == name).map(|(_, s, l)| &self.args[*s..*s + *l])
    }
    fn has(&self, name: &str) -> bool {
        self.parts.iter().any(|(n, _, _)| n == name)
    }
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn skip_ws(&mut self) {
        while self.i < self.s.len() {
            match self.s[self.i] {
                b' ' | b'\t' | b'\r' | b'\n' => self.i += 1,
                b'/' if self.s.get(self.i + 1) == Some(&b'*') => {
                    while self.i + 1 < self.s.len() && !(self.s[self.i] == b'*' && self.s[self.i + 1] == b'/') {
                        self.i += 1;
                    }
                    self.i += 2;
                }
                _ => break,
            }
        }
    }

    fn ident(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len() && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b'_') {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).to_string()
    }

    fn arg(&mut self) -> Arg {
        self.skip_ws();
        let Some(&c) = self.s.get(self.i) else { return Arg::Null };
        match c {
            b'#' => {
                self.i += 1;
                let id = self.ident();
                Arg::Ref(id.parse().unwrap_or(0))
            }
            b'$' => {
                self.i += 1;
                Arg::Null
            }
            b'*' => {
                self.i += 1;
                Arg::Null
            }
            b'\'' => {
                self.i += 1;
                let mut out = String::new();
                while self.i < self.s.len() {
                    if self.s[self.i] == b'\'' {
                        if self.s.get(self.i + 1) == Some(&b'\'') {
                            out.push('\'');
                            self.i += 2;
                            continue;
                        }
                        self.i += 1;
                        break;
                    }
                    out.push(self.s[self.i] as char);
                    self.i += 1;
                }
                Arg::Str(out)
            }
            b'.' => {
                self.i += 1;
                let name = self.ident();
                if self.s.get(self.i) == Some(&b'.') {
                    self.i += 1;
                }
                Arg::Enum(name)
            }
            b'(' => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.s.get(self.i) {
                        Some(b')') => {
                            self.i += 1;
                            break;
                        }
                        Some(b',') => {
                            self.i += 1;
                        }
                        None => break,
                        _ => items.push(self.arg()),
                    }
                }
                Arg::List(items)
            }
            b'-' | b'+' | b'0'..=b'9' => {
                let start = self.i;
                self.i += 1;
                while self.i < self.s.len() && (self.s[self.i].is_ascii_digit() || matches!(self.s[self.i], b'.' | b'E' | b'e' | b'-' | b'+')) {
                    self.i += 1;
                }
                Arg::Num(String::from_utf8_lossy(&self.s[start..self.i]).parse().unwrap_or(0.0))
            }
            _ if c.is_ascii_alphabetic() => {
                let name = self.ident();
                self.skip_ws();
                if self.s.get(self.i) == Some(&b'(') {
                    let inner = self.arg();
                    Arg::Typed(name, inner.list().to_vec())
                } else {
                    Arg::Enum(name)
                }
            }
            _ => {
                self.i += 1;
                Arg::Null
            }
        }
    }
}

/// Parse the DATA section into entities by id.
pub fn parse(text: &str) -> BTreeMap<usize, Entity> {
    let mut out = BTreeMap::new();
    let data_start = text.find("DATA;").map(|i| i + 5).unwrap_or(0);
    let mut p = Parser { s: text.as_bytes(), i: data_start };
    loop {
        p.skip_ws();
        let Some(&c) = p.s.get(p.i) else { break };
        if c != b'#' {
            // ENDSEC or junk: skip to next '#'
            if text[p.i..].starts_with("ENDSEC") {
                break;
            }
            p.i += 1;
            continue;
        }
        p.i += 1;
        let id: usize = p.ident().parse().unwrap_or(0);
        p.skip_ws();
        if p.s.get(p.i) != Some(&b'=') {
            continue;
        }
        p.i += 1;
        p.skip_ws();
        let mut parts = Vec::new();
        let mut args = Vec::new();
        if p.s.get(p.i) == Some(&b'(') {
            // complex entity
            p.i += 1;
            loop {
                p.skip_ws();
                match p.s.get(p.i) {
                    Some(b')') => {
                        p.i += 1;
                        break;
                    }
                    None => break,
                    _ => {
                        let name = p.ident();
                        if name.is_empty() {
                            p.i += 1;
                            continue;
                        }
                        let a = p.arg();
                        let l = a.list().to_vec();
                        parts.push((name, args.len(), l.len()));
                        args.extend(l);
                    }
                }
            }
        } else {
            let name = p.ident();
            let a = p.arg();
            let l = a.list().to_vec();
            parts.push((name, 0, l.len()));
            args = l;
        }
        // skip to ';'
        while p.i < p.s.len() && p.s[p.i] != b';' {
            p.i += 1;
        }
        p.i += 1;
        let kind = parts.iter().map(|(n, _, _)| n.as_str()).collect::<Vec<_>>().join("+");
        out.insert(id, Entity { kind, args, parts });
    }
    out
}

// ---------------------------------------------------------------- reader

struct Reader {
    e: BTreeMap<usize, Entity>,
}

impl Reader {
    fn get(&self, id: usize) -> Option<&Entity> {
        self.e.get(&id)
    }

    fn point(&self, id: usize) -> Option<DVec3> {
        let e = self.get(id)?;
        if e.has("VERTEX_POINT") {
            return self.point(e.args.get(1)?.r()?);
        }
        let coords = e.part("CARTESIAN_POINT")?.get(1)?.list();
        Some(DVec3::new(coords.first().and_then(|a| a.num()).unwrap_or(0.0), coords.get(1).and_then(|a| a.num()).unwrap_or(0.0), coords.get(2).and_then(|a| a.num()).unwrap_or(0.0)))
    }

    fn direction(&self, id: usize) -> Option<DVec3> {
        let e = self.get(id)?;
        let a = e.part("DIRECTION")?.get(1)?.list();
        Some(DVec3::new(a.first().and_then(|a| a.num()).unwrap_or(0.0), a.get(1).and_then(|a| a.num()).unwrap_or(0.0), a.get(2).and_then(|a| a.num()).unwrap_or(0.0)).normalize_or(DVec3::Z))
    }

    fn vector(&self, id: usize) -> Option<DVec3> {
        let e = self.get(id)?;
        let a = e.part("VECTOR")?;
        Some(self.direction(a.get(1)?.r()?)? * a.get(2).and_then(|a| a.num()).unwrap_or(1.0))
    }

    fn placement(&self, id: usize) -> Option<Plane> {
        let e = self.get(id)?;
        let a = e.part("AXIS2_PLACEMENT_3D").or_else(|| e.part("AXIS1_PLACEMENT"))?;
        let origin = self.point(a.get(1)?.r()?)?;
        let z = a.get(2).and_then(|a| a.r()).and_then(|r| self.direction(r)).unwrap_or(DVec3::Z);
        let x_ref = a.get(3).and_then(|a| a.r()).and_then(|r| self.direction(r));
        let x = match x_ref {
            Some(x) => (x - z * x.dot(z)).normalize_or(perp(z)),
            None => perp(z),
        };
        Some(Plane { origin, x_axis: x, y_axis: z.cross(x).normalize(), z_axis: z })
    }

    fn bspline_curve(&self, e: &Entity) -> Option<NurbsCurve> {
        let (degree, pts, mults, knots, weights) = if let Some(a) = e.part("B_SPLINE_CURVE_WITH_KNOTS").filter(|a| a.len() >= 8) {
            (a[1].num()? as usize, a[2].list().to_vec(), a[6].list().to_vec(), a[7].list().to_vec(), None)
        } else {
            let b = e.part("B_SPLINE_CURVE")?;
            let k = e.part("B_SPLINE_CURVE_WITH_KNOTS")?;
            (b[0].num()? as usize, b[1].list().to_vec(), k[0].list().to_vec(), k[1].list().to_vec(), e.part("RATIONAL_B_SPLINE_CURVE").map(|w| w[0].list().to_vec()))
        };
        let cps: Vec<DVec3> = pts.iter().filter_map(|a| self.point(a.r()?)).collect();
        if cps.len() < degree + 1 {
            return None;
        }
        let mut kv = Vec::new();
        for (m, k) in mults.iter().zip(&knots) {
            for _ in 0..(m.num().unwrap_or(1.0) as usize) {
                kv.push(k.num().unwrap_or(0.0));
            }
        }
        let w: Vec<f64> = weights.map(|w| w.iter().map(|a| a.num().unwrap_or(1.0)).collect()).unwrap_or_else(|| vec![1.0; cps.len()]);
        if kv.len() != cps.len() + degree + 1 {
            // unclamped or malformed: fall back on a polyline through the control points
            return NurbsCurve::polyline(cps).ok();
        }
        NurbsCurve::new(degree, cps, w, kv).ok()
    }

    fn curve(&self, id: usize) -> Option<NurbsCurve> {
        let e = self.get(id)?;
        if e.has("B_SPLINE_CURVE") || e.has("B_SPLINE_CURVE_WITH_KNOTS") {
            return self.bspline_curve(e);
        }
        if let Some(a) = e.part("LINE") {
            let p = self.point(a.get(1)?.r()?)?;
            let v = self.vector(a.get(2)?.r()?)?;
            return NurbsCurve::line(p, p + v).ok();
        }
        if let Some(a) = e.part("CIRCLE") {
            let plane = self.placement(a.get(1)?.r()?)?;
            return NurbsCurve::circle(&plane, a.get(2)?.num()?).ok();
        }
        if let Some(a) = e.part("ELLIPSE") {
            let plane = self.placement(a.get(1)?.r()?)?;
            return NurbsCurve::ellipse(&plane, a.get(2)?.num()?, a.get(3)?.num()?).ok();
        }
        if let Some(a) = e.part("POLYLINE") {
            let pts: Vec<DVec3> = a.get(1)?.list().iter().filter_map(|p| self.point(p.r()?)).collect();
            return NurbsCurve::polyline(pts).ok();
        }
        if let Some(a) = e.part("TRIMMED_CURVE") {
            let basis = self.curve(a.get(1)?.r()?)?;
            let t = |arg: &Arg| -> Option<f64> {
                for item in arg.list() {
                    match item {
                        Arg::Typed(n, v) if n == "PARAMETER_VALUE" => return v.first()?.num(),
                        Arg::Ref(r) => {
                            let p = self.point(*r)?;
                            return Some(closest_param(&basis, p));
                        }
                        _ => {}
                    }
                }
                None
            };
            let (t0, t1) = (t(a.get(2)?)?, t(a.get(3)?)?);
            let sense = a.get(4).is_none_or(|s| s.flag());
            return sub_curve(&basis, t0, t1, sense, self.get(id).is_some_and(|e| e.has("CIRCLE")));
        }
        if let Some(a) = e.part("COMPOSITE_CURVE") {
            let mut pts: Vec<DVec3> = Vec::new();
            for seg in a.get(1)?.list() {
                let se = self.get(seg.r()?)?;
                let sa = se.part("COMPOSITE_CURVE_SEGMENT")?;
                let same = sa.get(1).is_none_or(|s| s.flag());
                let c = self.curve(sa.get(2)?.r()?)?;
                let mut poly = c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3);
                if !same {
                    poly.reverse();
                }
                if pts.is_empty() { pts.extend(poly) } else { pts.extend(poly.into_iter().skip(1)) }
            }
            return NurbsCurve::polyline(pts).ok();
        }
        None
    }

    /// Edge as a 3D polyline between its vertices (respecting the edge sense).
    fn edge_points(&self, id: usize, tol: f64) -> Option<Vec<DVec3>> {
        let e = self.get(id)?;
        let a = e.part("EDGE_CURVE")?;
        let v1 = self.point(a.get(1)?.r()?)?;
        let v2 = self.point(a.get(2)?.r()?)?;
        let c = self.curve(a.get(3)?.r()?)?;
        let same = a.get(4).is_none_or(|s| s.flag());
        // trim the curve between the vertices (lines/circles are unbounded in STEP)
        let is_line = self.get(a.get(3)?.r()?).is_some_and(|ce| ce.has("LINE"));
        let pts = if is_line {
            vec![v1, v2]
        } else {
            let t0 = closest_param(&c, v1);
            let t1 = closest_param(&c, v2);
            let closed_full = (v1 - v2).length() < tol * 10.0 && c.is_closed();
            let piece = if closed_full { c.clone() } else { sub_curve(&c, t0, t1, true, c.is_closed()).unwrap_or_else(|| c.clone()) };
            let mut p = piece.tessellate_adaptive(tol);
            if let (Some(f), Some(l)) = (p.first().copied(), p.last().copied()) {
                if !closed_full && (f - v1).length() > (f - v2).length() && (l - v2).length() > (l - v1).length() {
                    p.reverse();
                }
            }
            p
        };
        let mut pts = pts;
        if !same {
            pts.reverse();
        }
        Some(pts)
    }

    fn loop_points(&self, id: usize, tol: f64) -> Option<Vec<DVec3>> {
        let e = self.get(id)?;
        let edges = e.part("EDGE_LOOP")?.get(1)?.list();
        let mut out: Vec<DVec3> = Vec::new();
        for oe in edges {
            let oe_e = self.get(oe.r()?)?;
            let (edge_id, orient) = if let Some(a) = oe_e.part("ORIENTED_EDGE") { (a.get(3)?.r()?, a.get(4).is_none_or(|s| s.flag())) } else { (oe.r()?, true) };
            let mut pts = self.edge_points(edge_id, tol)?;
            if !orient {
                pts.reverse();
            }
            if out.is_empty() {
                out.extend(pts);
            } else {
                out.extend(pts.into_iter().skip(1));
            }
        }
        if out.len() > 2 && (out[0] - out[out.len() - 1]).length() > tol {
            out.push(out[0]);
        }
        Some(out)
    }

    /// Surface of a face as NURBS, sized to hold the given boundary points for analytic types.
    fn surface(&self, id: usize, hint: &[DVec3]) -> Option<NurbsSurface> {
        let e = self.get(id)?;
        if e.has("B_SPLINE_SURFACE") || e.has("B_SPLINE_SURFACE_WITH_KNOTS") {
            return self.bspline_surface(e);
        }
        let extent = tri_geom::BoundingBox::from_points(hint.iter()).diagonal().max(1.0);
        let margin = extent * 0.02 + 1e-3;
        if let Some(a) = e.part("PLANE") {
            let plane = self.placement(a.get(1)?.r()?)?;
            let (mut u0, mut u1, mut v0, mut v1) = (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
            for p in hint {
                let d = *p - plane.origin;
                let (u, v) = (d.dot(plane.x_axis), d.dot(plane.y_axis));
                u0 = u0.min(u);
                u1 = u1.max(u);
                v0 = v0.min(v);
                v1 = v1.max(v);
            }
            if !u0.is_finite() {
                (u0, u1, v0, v1) = (-extent, extent, -extent, extent);
            }
            return NurbsSurface::plane(&plane, u0 - margin, u1 + margin, v0 - margin, v1 + margin).ok();
        }
        if let Some(a) = e.part("CYLINDRICAL_SURFACE") {
            let plane = self.placement(a.get(1)?.r()?)?;
            let r = a.get(2)?.num()?;
            let plane = seam_away(plane, hint);
            let (h0, h1) = axis_range(&plane, hint, extent);
            let base = Plane { origin: plane.origin + plane.z_axis * (h0 - margin), ..plane };
            return NurbsSurface::cylinder(&base, r, h1 - h0 + 2.0 * margin).ok();
        }
        if let Some(a) = e.part("CONICAL_SURFACE") {
            let plane = self.placement(a.get(1)?.r()?)?;
            let r = a.get(2)?.num()?;
            let ang = a.get(3)?.num()?;
            let plane = seam_away(plane, hint);
            let (h0, h1) = axis_range(&plane, hint, extent);
            let (h0, h1) = (h0 - margin, h1 + margin);
            let r0 = r + h0 * ang.tan();
            let r1 = r + h1 * ang.tan();
            let profile = NurbsCurve::line(plane.point_at_3(r0.max(0.0), 0.0, h0), plane.point_at_3(r1.max(0.0), 0.0, h1)).ok()?;
            return NurbsSurface::revolve(&profile, plane.origin, plane.z_axis, std::f64::consts::TAU).ok();
        }
        if let Some(a) = e.part("SPHERICAL_SURFACE") {
            let plane = self.placement(a.get(1)?.r()?)?;
            let r = a.get(2)?.num()?;
            let plane = seam_away(plane, hint);
            let prof_plane = Plane { origin: plane.origin, x_axis: plane.x_axis, y_axis: plane.z_axis, z_axis: -plane.y_axis };
            let profile = NurbsCurve::arc(&prof_plane, r, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2).ok()?;
            return NurbsSurface::revolve(&profile, plane.origin, plane.z_axis, std::f64::consts::TAU).ok();
        }
        if let Some(a) = e.part("TOROIDAL_SURFACE") {
            let plane = self.placement(a.get(1)?.r()?)?;
            let plane = seam_away(plane, hint);
            return NurbsSurface::torus(&plane, a.get(2)?.num()?, a.get(3)?.num()?).ok();
        }
        if let Some(a) = e.part("SURFACE_OF_REVOLUTION") {
            let c = self.curve(a.get(1)?.r()?)?;
            let axis = self.placement(a.get(2)?.r()?)?;
            return NurbsSurface::revolve(&c, axis.origin, axis.z_axis, std::f64::consts::TAU).ok();
        }
        if let Some(a) = e.part("SURFACE_OF_LINEAR_EXTRUSION") {
            let c = self.curve(a.get(1)?.r()?)?;
            let v = self.vector(a.get(2)?.r()?)?;
            // extend so the hint points are covered along the direction
            let dir = v.normalize_or(DVec3::Z);
            let (mut lo, mut hi) = (0.0f64, v.length());
            for p in hint {
                let d = (*p - c.point_at(c.domain().0)).dot(dir);
                lo = lo.min(d);
                hi = hi.max(d);
            }
            let mut moved = c.clone();
            moved.transform(&glam::DAffine3::from_translation(dir * (lo - margin)));
            return NurbsSurface::extrude(&moved, dir * (hi - lo + 2.0 * margin)).ok();
        }
        if let Some(a) = e.part("OFFSET_SURFACE") {
            let base = self.surface(a.get(1)?.r()?, hint)?;
            return Some(base.offset(a.get(2)?.num()?));
        }
        None
    }

    fn bspline_surface(&self, e: &Entity) -> Option<NurbsSurface> {
        let (du, dv, grid, um, vm, uk, vk, weights) = if let Some(a) = e.part("B_SPLINE_SURFACE_WITH_KNOTS").filter(|a| a.len() >= 12) {
            (a[1].num()? as usize, a[2].num()? as usize, a[3].list().to_vec(), a[8].list().to_vec(), a[9].list().to_vec(), a[10].list().to_vec(), a[11].list().to_vec(), None)
        } else {
            let b = e.part("B_SPLINE_SURFACE")?;
            let k = e.part("B_SPLINE_SURFACE_WITH_KNOTS")?;
            (b[0].num()? as usize, b[1].num()? as usize, b[2].list().to_vec(), k[0].list().to_vec(), k[1].list().to_vec(), k[2].list().to_vec(), k[3].list().to_vec(), e.part("RATIONAL_B_SPLINE_SURFACE").map(|w| w[0].list().to_vec()))
        };
        let rows: Vec<Vec<DVec3>> = grid.iter().map(|r| r.list().iter().filter_map(|p| self.point(p.r()?)).collect()).collect();
        let cu = rows.len();
        let cv = rows.first().map_or(0, |r| r.len());
        if cu < du + 1 || cv < dv + 1 || rows.iter().any(|r| r.len() != cv) {
            return None;
        }
        let expand = |m: &[Arg], k: &[Arg]| -> Vec<f64> {
            let mut out = Vec::new();
            for (mm, kk) in m.iter().zip(k) {
                for _ in 0..(mm.num().unwrap_or(1.0) as usize) {
                    out.push(kk.num().unwrap_or(0.0));
                }
            }
            out
        };
        let (ku, kv) = (expand(&um, &uk), expand(&vm, &vk));
        let cps: Vec<DVec3> = rows.iter().flatten().copied().collect();
        let w: Vec<f64> = weights.map(|w| w.iter().flat_map(|r| r.list().iter().map(|a| a.num().unwrap_or(1.0)).collect::<Vec<_>>()).collect()).filter(|w: &Vec<f64>| w.len() == cps.len()).unwrap_or_else(|| vec![1.0; cps.len()]);
        if ku.len() != cu + du + 1 || kv.len() != cv + dv + 1 {
            return NurbsSurface::from_grid(du, dv, rows).ok();
        }
        NurbsSurface::new(du, dv, cu, cv, cps, w, ku, kv).ok()
    }

    fn face(&self, id: usize) -> Option<Face> {
        let e = self.get(id)?;
        let a = e.part("ADVANCED_FACE").or_else(|| e.part("FACE_SURFACE"))?;
        let bounds = a.get(1)?.list();
        let srf_id = a.get(2)?.r()?;
        // loops in 3D first (needed to size analytic surfaces)
        let mut loops3: Vec<(bool, Vec<DVec3>)> = Vec::new();
        let mut all: Vec<DVec3> = Vec::new();
        let rough_tol = 1e-3;
        for b in bounds {
            let be = self.get(b.r()?)?;
            let ba = be.part("FACE_OUTER_BOUND").or_else(|| be.part("FACE_BOUND"))?;
            let outer = be.has("FACE_OUTER_BOUND");
            let Some(pts) = self.loop_points(ba.get(1)?.r()?, rough_tol) else { continue };
            all.extend(pts.iter().copied());
            loops3.push((outer, pts));
        }
        let srf = self.surface(srf_id, &all)?;
        let diag = srf.bounding_box().diagonal().max(1e-6);
        let tol = diag * 5e-4;
        // re-tessellate the loops at the final tolerance
        let mut loops3: Vec<(bool, Vec<DVec3>)> = loops3;
        if tol < rough_tol {
            loops3.clear();
            for b in bounds {
                let be = self.get(b.r()?)?;
                let ba = be.part("FACE_OUTER_BOUND").or_else(|| be.part("FACE_BOUND"))?;
                if let Some(pts) = self.loop_points(ba.get(1)?.r()?, tol) {
                    loops3.push((be.has("FACE_OUTER_BOUND"), pts));
                }
            }
        }
        let mut loops_uv: Vec<Vec<DVec2>> = Vec::new();
        let (du, dv) = (srf.domain_u(), srf.domain_v());
        for (_, pts) in &loops3 {
            let mut uv: Vec<DVec2> = Vec::with_capacity(pts.len());
            let (mut u, mut v) = srf.closest_uv(pts[0]);
            for p in pts {
                let r = srf.closest_uv_from(*p, u, v);
                u = r.0;
                v = r.1;
                let q = DVec2::new(u, v);
                if uv.last().is_none_or(|l: &DVec2| (*l - q).length() > 1e-9) {
                    uv.push(q);
                }
            }
            if uv.len() > 2 && (uv[0] - uv[uv.len() - 1]).length() > 1e-9 {
                uv.push(uv[0]);
            }
            if uv.len() >= 4 {
                loops_uv.push(uv);
            }
        }
        // loops that only run along the surface boundary mean "untrimmed"
        let on_border = |l: &Vec<DVec2>| {
            let eps_u = (du.1 - du.0) * 1e-3;
            let eps_v = (dv.1 - dv.0) * 1e-3;
            l.iter().all(|p| (p.x - du.0).abs() < eps_u || (p.x - du.1).abs() < eps_u || (p.y - dv.0).abs() < eps_v || (p.y - dv.1).abs() < eps_v)
        };
        // a loop that jumps across the seam of a periodic surface cannot be represented: drop it
        let jumps = |l: &Vec<DVec2>| l.windows(2).any(|w| (w[0].x - w[1].x).abs() > (du.1 - du.0) * 0.5 || (w[0].y - w[1].y).abs() > (dv.1 - dv.0) * 0.5);
        loops_uv.retain(|l| !on_border(l) && !jumps(l));
        // outer loop first
        if loops_uv.len() > 1 {
            let area = |l: &Vec<DVec2>| l.windows(2).map(|w| w[0].x * w[1].y - w[1].x * w[0].y).sum::<f64>().abs();
            let (imax, _) = loops_uv.iter().enumerate().max_by(|a, b| area(a.1).partial_cmp(&area(b.1)).unwrap()).unwrap();
            loops_uv.swap(0, imax);
        }
        let mut f = Face { surface: srf, loops: loops_uv };
        let sense = a.get(3).is_none_or(|s| s.flag());
        if !sense {
            // keep the surface orientation consistent with the face normal
            let _ = &mut f;
        }
        Some(f)
    }

    fn faces_of(&self, id: usize, out: &mut Vec<Face>, depth: usize) {
        if depth > 8 {
            return;
        }
        let Some(e) = self.get(id) else { return };
        if e.has("ADVANCED_FACE") || e.has("FACE_SURFACE") {
            if let Some(f) = self.face(id) {
                out.push(f);
            }
            return;
        }
        for (name, idx) in [("CLOSED_SHELL", 1), ("OPEN_SHELL", 1), ("MANIFOLD_SOLID_BREP", 1), ("BREP_WITH_VOIDS", 1), ("SHELL_BASED_SURFACE_MODEL", 1), ("FACETED_BREP", 1)] {
            if let Some(a) = e.part(name) {
                match a.get(idx) {
                    Some(Arg::List(items)) => {
                        for it in items {
                            if let Some(r) = it.r() {
                                self.faces_of(r, out, depth + 1);
                            }
                        }
                    }
                    Some(Arg::Ref(r)) => self.faces_of(*r, out, depth + 1),
                    _ => {}
                }
                return;
            }
        }
    }
}

fn perp(z: DVec3) -> DVec3 {
    let c = if z.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
    (c - z * c.dot(z)).normalize_or(DVec3::X)
}

/// Rotate a periodic surface's placement so its seam points away from the loop centroid.
fn seam_away(plane: Plane, hint: &[DVec3]) -> Plane {
    if hint.is_empty() {
        return plane;
    }
    let c = hint.iter().fold(DVec3::ZERO, |a, p| a + *p) / hint.len() as f64;
    let d = c - plane.origin;
    let radial = d - plane.z_axis * d.dot(plane.z_axis);
    if radial.length() < 1e-9 {
        return plane;
    }
    let x = (-radial).normalize();
    Plane { origin: plane.origin, x_axis: x, y_axis: plane.z_axis.cross(x).normalize(), z_axis: plane.z_axis }
}

fn axis_range(plane: &Plane, hint: &[DVec3], extent: f64) -> (f64, f64) {
    let (mut h0, mut h1) = (f64::INFINITY, f64::NEG_INFINITY);
    for p in hint {
        let h = (*p - plane.origin).dot(plane.z_axis);
        h0 = h0.min(h);
        h1 = h1.max(h);
    }
    if !h0.is_finite() {
        (-extent, extent)
    } else {
        (h0, h1)
    }
}

fn closest_param(c: &NurbsCurve, p: DVec3) -> f64 {
    let (t0, t1) = c.domain();
    let n = 256;
    let mut best = (f64::INFINITY, t0);
    for i in 0..=n {
        let t = t0 + (t1 - t0) * i as f64 / n as f64;
        let d = (c.point_at(t) - p).length_squared();
        if d < best.0 {
            best = (d, t);
        }
    }
    // refine by ternary search around the best sample
    let step = (t1 - t0) / n as f64;
    let (mut lo, mut hi) = ((best.1 - step).max(t0), (best.1 + step).min(t1));
    for _ in 0..40 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if (c.point_at(m1) - p).length_squared() < (c.point_at(m2) - p).length_squared() {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    (lo + hi) * 0.5
}

/// Sub-curve between two parameters as a polyline-fitted NURBS (exact for straight pieces).
fn sub_curve(c: &NurbsCurve, t0: f64, t1: f64, sense: bool, periodic: bool) -> Option<NurbsCurve> {
    let (d0, d1) = c.domain();
    let (mut a, mut b) = (t0, t1);
    if periodic && b <= a + 1e-12 {
        b += d1 - d0;
    }
    if !periodic && b < a {
        std::mem::swap(&mut a, &mut b);
    }
    if (b - a).abs() < 1e-12 {
        return None;
    }
    let n = 64;
    let mut pts: Vec<DVec3> = (0..=n).map(|i| {
        let mut t = a + (b - a) * i as f64 / n as f64;
        if periodic && t > d1 {
            t -= d1 - d0;
        }
        c.point_at(t.clamp(d0, d1))
    }).collect();
    if !sense {
        pts.reverse();
    }
    if c.degree == 1 {
        pts = vec![pts[0], pts[pts.len() - 1]];
    }
    NurbsCurve::polyline(pts).ok()
}

pub fn import(doc: &mut Document, path: &Path) -> Result<usize> {
    let text = read_text(path)?;
    if !text.starts_with("ISO-10303-21") {
        bail!("not a STEP Part 21 file");
    }
    let r = Reader { e: parse(&text) };
    let layer = doc.current_layer();
    let mut count = 0;
    // top level: solids, shells, surface models; then faces/curves/points not used by them
    let mut used: HashSet<usize> = HashSet::new();
    let mark = |id: usize, used: &mut HashSet<usize>| {
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            if !used.insert(i) {
                continue;
            }
            if let Some(e) = r.get(i) {
                for a in &e.args {
                    collect_refs(a, &mut stack);
                }
            }
        }
    };
    let tops: Vec<usize> = r.e.iter().filter(|(_, e)| e.has("MANIFOLD_SOLID_BREP") || e.has("BREP_WITH_VOIDS") || e.has("SHELL_BASED_SURFACE_MODEL") || e.has("FACETED_BREP")).map(|(i, _)| *i).collect();
    for id in &tops {
        let mut faces = Vec::new();
        r.faces_of(*id, &mut faces, 0);
        mark(*id, &mut used);
        if faces.is_empty() {
            continue;
        }
        let e = r.get(*id).unwrap();
        let name = e.args.first().and_then(|a| if let Arg::Str(s) = a { Some(s.clone()) } else { None }).filter(|s| !s.is_empty());
        let geom = if faces.len() == 1 && !faces[0].is_trimmed() { Geometry::Surface(faces.remove(0).surface) } else { Geometry::Brep(Brep::new(faces)) };
        doc.add_with(geom, Attributes { layer, name, ..Default::default() });
        count += 1;
    }
    // shells not inside a solid
    let shells: Vec<usize> = r.e.iter().filter(|(i, e)| !used.contains(i) && (e.has("CLOSED_SHELL") || e.has("OPEN_SHELL"))).map(|(i, _)| *i).collect();
    for id in shells {
        let mut faces = Vec::new();
        r.faces_of(id, &mut faces, 0);
        mark(id, &mut used);
        if !faces.is_empty() {
            doc.add_with(Geometry::Brep(Brep::new(faces)), Attributes { layer, ..Default::default() });
            count += 1;
        }
    }
    // loose faces
    let faces: Vec<usize> = r.e.iter().filter(|(i, e)| !used.contains(i) && (e.has("ADVANCED_FACE") || e.has("FACE_SURFACE"))).map(|(i, _)| *i).collect();
    for id in faces {
        if let Some(f) = r.face(id) {
            mark(id, &mut used);
            doc.add_with(Geometry::Brep(Brep::new(vec![f])), Attributes { layer, ..Default::default() });
            count += 1;
        }
    }
    // loose surfaces, curves and points (members of geometric sets or unreferenced)
    let mut referenced: HashSet<usize> = HashSet::new();
    for e in r.e.values() {
        for a in &e.args {
            let mut st = Vec::new();
            collect_refs(a, &mut st);
            referenced.extend(st);
        }
    }
    let set_members: HashSet<usize> = r.e.values().filter(|e| e.has("GEOMETRIC_SET") || e.has("GEOMETRIC_CURVE_SET") || e.parts.iter().any(|(n, _, _)| n.ends_with("SHAPE_REPRESENTATION"))).flat_map(|e| e.args.get(1).map(|a| a.list().iter().filter_map(|x| x.r()).collect::<Vec<_>>()).unwrap_or_default()).collect();
    for (id, e) in &r.e {
        if used.contains(id) {
            continue;
        }
        let loose = !referenced.contains(id) || set_members.contains(id);
        if !loose {
            continue;
        }
        if e.has("B_SPLINE_SURFACE") || e.has("B_SPLINE_SURFACE_WITH_KNOTS") {
            if let Some(s) = r.bspline_surface(e) {
                doc.add_with(Geometry::Surface(s), Attributes { layer, ..Default::default() });
                count += 1;
            }
        } else if e.has("B_SPLINE_CURVE") || e.has("B_SPLINE_CURVE_WITH_KNOTS") || e.has("POLYLINE") || e.has("TRIMMED_CURVE") || e.has("COMPOSITE_CURVE") || (set_members.contains(id) && (e.has("CIRCLE") || e.has("ELLIPSE") || e.has("LINE"))) {
            if let Some(c) = r.curve(*id) {
                doc.add_with(Geometry::Curve(c), Attributes { layer, ..Default::default() });
                count += 1;
            }
        } else if e.has("CARTESIAN_POINT") && set_members.contains(id) {
            if let Some(p) = r.point(*id) {
                doc.add_with(Geometry::Point(p), Attributes { layer, ..Default::default() });
                count += 1;
            }
        }
    }
    Ok(count)
}

fn collect_refs(a: &Arg, out: &mut Vec<usize>) {
    match a {
        Arg::Ref(r) => out.push(*r),
        Arg::List(l) | Arg::Typed(_, l) => l.iter().for_each(|x| collect_refs(x, out)),
        _ => {}
    }
}

// ---------------------------------------------------------------- writer

struct Writer {
    lines: Vec<String>,
    next: usize,
}

impl Writer {
    fn add(&mut self, text: String) -> usize {
        let id = self.next;
        self.next += 1;
        self.lines.push(format!("#{id} = {text};"));
        id
    }
    fn point(&mut self, p: DVec3) -> usize {
        self.add(format!("CARTESIAN_POINT('',({},{},{}))", fnum(p.x), fnum(p.y), fnum(p.z)))
    }
    fn direction(&mut self, d: DVec3) -> usize {
        self.add(format!("DIRECTION('',({},{},{}))", fnum(d.x), fnum(d.y), fnum(d.z)))
    }
    fn placement(&mut self, plane: &Plane) -> usize {
        let o = self.point(plane.origin);
        let z = self.direction(plane.z_axis);
        let x = self.direction(plane.x_axis);
        self.add(format!("AXIS2_PLACEMENT_3D('',#{o},#{z},#{x})"))
    }
    fn curve(&mut self, c: &NurbsCurve) -> usize {
        let pts: Vec<String> = c.control_points.iter().map(|p| format!("#{}", self.point(*p))).collect();
        let (mults, knots) = knot_mults(&c.knots);
        let closed = if c.is_closed() { ".T." } else { ".F." };
        if c.is_rational() {
            let w: Vec<String> = c.weights.iter().map(|w| fnum(*w)).collect();
            self.add(format!("(BOUNDED_CURVE()B_SPLINE_CURVE({},({}),.UNSPECIFIED.,{closed},.F.)B_SPLINE_CURVE_WITH_KNOTS(({}),({}),.UNSPECIFIED.)CURVE()GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_CURVE(({}))REPRESENTATION_ITEM(''))", c.degree, pts.join(","), mults.join(","), knots.join(","), w.join(",")))
        } else {
            self.add(format!("B_SPLINE_CURVE_WITH_KNOTS('',{},({}),.UNSPECIFIED.,{closed},.F.,({}),({}),.UNSPECIFIED.)", c.degree, pts.join(","), mults.join(","), knots.join(",")))
        }
    }
    fn surface(&mut self, s: &NurbsSurface) -> usize {
        let mut rows = Vec::new();
        for i in 0..s.count_u {
            let row: Vec<String> = (0..s.count_v).map(|j| format!("#{}", self.point(s.control_points[i * s.count_v + j]))).collect();
            rows.push(format!("({})", row.join(",")));
        }
        let (um, uk) = knot_mults(&s.knots_u);
        let (vm, vk) = knot_mults(&s.knots_v);
        let rational = s.weights.iter().any(|w| (*w - 1.0).abs() > 1e-12);
        if rational {
            let wrows: Vec<String> = (0..s.count_u).map(|i| format!("({})", (0..s.count_v).map(|j| fnum(s.weights[i * s.count_v + j])).collect::<Vec<_>>().join(","))).collect();
            self.add(format!("(BOUNDED_SURFACE()B_SPLINE_SURFACE({},{},({}),.UNSPECIFIED.,.F.,.F.,.F.)B_SPLINE_SURFACE_WITH_KNOTS(({}),({}),({}),({}),.UNSPECIFIED.)GEOMETRIC_REPRESENTATION_ITEM()RATIONAL_B_SPLINE_SURFACE(({}))REPRESENTATION_ITEM('')SURFACE())", s.degree_u, s.degree_v, rows.join(","), um.join(","), vm.join(","), uk.join(","), vk.join(","), wrows.join(",")))
        } else {
            self.add(format!("B_SPLINE_SURFACE_WITH_KNOTS('',{},{},({}),.UNSPECIFIED.,.F.,.F.,.F.,({}),({}),({}),({}),.UNSPECIFIED.)", s.degree_u, s.degree_v, rows.join(","), um.join(","), vm.join(","), uk.join(","), vk.join(",")))
        }
    }
    /// A closed loop given as 3D points → FACE_BOUND through a single closed polyline edge.
    fn bound(&mut self, pts: &[DVec3], outer: bool) -> Option<usize> {
        if pts.len() < 3 {
            return None;
        }
        let mut pts = pts.to_vec();
        if (pts[0] - pts[pts.len() - 1]).length() > 1e-12 {
            pts.push(pts[0]);
        }
        let poly = NurbsCurve::polyline(pts.clone()).ok()?;
        let cid = self.curve(&poly);
        let vp = self.point(pts[0]);
        let v = self.add(format!("VERTEX_POINT('',#{vp})"));
        let ec = self.add(format!("EDGE_CURVE('',#{v},#{v},#{cid},.T.)"));
        let oe = self.add(format!("ORIENTED_EDGE('',*,*,#{ec},.T.)"));
        let lp = self.add(format!("EDGE_LOOP('',(#{oe}))"));
        Some(self.add(format!("{}('',#{lp},.T.)", if outer { "FACE_OUTER_BOUND" } else { "FACE_BOUND" })))
    }
    fn face(&mut self, f: &Face) -> usize {
        let sid = self.surface(&f.surface);
        let tol = f.surface.bounding_box().diagonal().max(1e-6) * 5e-4;
        let mut bounds = Vec::new();
        if f.is_trimmed() {
            for (i, lp) in f.loops.iter().enumerate() {
                let pts: Vec<DVec3> = lp.iter().map(|uv| f.surface.point_at(uv.x, uv.y)).collect();
                if let Some(b) = self.bound(&pts, i == 0) {
                    bounds.push(format!("#{b}"));
                }
            }
        } else {
            let mut pts = Vec::new();
            for (k, c) in f.surface.boundary_curves().iter().enumerate() {
                let mut p = c.tessellate_adaptive(tol);
                // boundary_curves order: v=v0 (u→), u=u1 (v→), v=v1, u=u0: orient as a loop
                if k == 2 || k == 3 {
                    p.reverse();
                }
                if pts.is_empty() { pts.extend(p) } else { pts.extend(p.into_iter().skip(1)) }
            }
            if let Some(b) = self.bound(&pts, true) {
                bounds.push(format!("#{b}"));
            }
        }
        self.add(format!("ADVANCED_FACE('',({}),#{sid},.T.)", bounds.join(",")))
    }
}

fn fnum(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 { format!("{}.", v as i64) } else { format!("{v}") }
}

fn knot_mults(knots: &[f64]) -> (Vec<String>, Vec<String>) {
    let mut mults: Vec<usize> = Vec::new();
    let mut vals: Vec<f64> = Vec::new();
    for k in knots {
        if let Some(last) = vals.last() {
            if (k - last).abs() < 1e-12 {
                *mults.last_mut().unwrap() += 1;
                continue;
            }
        }
        vals.push(*k);
        mults.push(1);
    }
    (mults.iter().map(|m| m.to_string()).collect(), vals.iter().map(|v| fnum(*v)).collect())
}

pub fn export(doc: &Document, path: &Path, only_selected: bool) -> Result<usize> {
    let mut w = Writer { lines: Vec::new(), next: 1 };
    let mut items: Vec<String> = Vec::new();
    let mut count = 0;
    let mut names: HashMap<usize, String> = HashMap::new();
    for o in doc.objects() {
        if !exported(doc, o, only_selected) {
            continue;
        }
        let name = o.attributes.name.clone().unwrap_or_default().replace('\'', "''");
        let id = match &o.geometry {
            Geometry::Point(p) => w.point(*p),
            Geometry::Curve(c) => w.curve(c),
            Geometry::Surface(s) => {
                let f = w.face(&Face::untrimmed(s.clone()));
                let shell = w.add(format!("OPEN_SHELL('{name}',(#{f}))"));
                w.add(format!("SHELL_BASED_SURFACE_MODEL('{name}',(#{shell}))"))
            }
            Geometry::Brep(b) => {
                let faces: Vec<String> = b.faces.iter().map(|f| format!("#{}", w.face(f))).collect();
                if b.closed {
                    let shell = w.add(format!("CLOSED_SHELL('{name}',({}))", faces.join(",")));
                    w.add(format!("MANIFOLD_SOLID_BREP('{name}',#{shell})"))
                } else {
                    let shell = w.add(format!("OPEN_SHELL('{name}',({}))", faces.join(",")));
                    w.add(format!("SHELL_BASED_SURFACE_MODEL('{name}',(#{shell}))"))
                }
            }
            Geometry::Mesh(m) => {
                // faceted: one planar face per triangle
                let mut faces = Vec::new();
                for t in 0..m.triangle_count() {
                    let (a, b, c) = m.triangle(t);
                    let nrm = (b - a).cross(c - a);
                    if nrm.length() < 1e-14 {
                        continue;
                    }
                    let plane = Plane::from_normal(a, nrm.normalize());
                    let pl = w.placement(&plane);
                    let sid = w.add(format!("PLANE('',#{pl})"));
                    if let Some(bd) = w.bound(&[a, b, c], true) {
                        faces.push(format!("#{}", w.add(format!("ADVANCED_FACE('',(#{bd}),#{sid},.T.)"))));
                    }
                }
                if faces.is_empty() {
                    continue;
                }
                let shell = w.add(format!("OPEN_SHELL('{name}',({}))", faces.join(",")));
                w.add(format!("SHELL_BASED_SURFACE_MODEL('{name}',(#{shell}))"))
            }
            Geometry::Instance { .. } => continue,
        };
        names.insert(id, name);
        items.push(format!("#{id}"));
        count += 1;
    }
    if count == 0 {
        bail!("nothing to export");
    }
    // context and product structure (AP214 minimal)
    let ctx = w.add("(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#LEN_UNC))GLOBAL_UNIT_ASSIGNED_CONTEXT((#LEN_UNIT,#ANG_UNIT,#SOLID_ANGLE_UNIT))REPRESENTATION_CONTEXT('Context','3D'))".to_string());
    let len_unit = w.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))".to_string());
    let ang_unit = w.add("(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))".to_string());
    let sa_unit = w.add("(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT())".to_string());
    let unc = w.add(format!("UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-3),#{len_unit},'distance_accuracy_value','')"));
    let ctx_line = w.lines[ctx - 1].replace("#LEN_UNC", &format!("#{unc}")).replace("#LEN_UNIT", &format!("#{len_unit}")).replace("#ANG_UNIT", &format!("#{ang_unit}")).replace("#SOLID_ANGLE_UNIT", &format!("#{sa_unit}"));
    w.lines[ctx - 1] = ctx_line;
    let origin = w.placement(&Plane::world_xy());
    let rep = w.add(format!("ADVANCED_BREP_SHAPE_REPRESENTATION('Triceratops',(#{origin},{}),#{ctx})", items.join(",")));
    let app = w.add("APPLICATION_CONTEXT('automotive design')".to_string());
    w.add(format!("APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2010,#{app})"));
    let pc = w.add(format!("PRODUCT_CONTEXT('',#{app},'mechanical')"));
    let product = w.add(format!("PRODUCT('Triceratops','Triceratops','',(#{pc}))"));
    let pdc = w.add(format!("PRODUCT_DEFINITION_CONTEXT('part definition',#{app},'design')"));
    let pdf = w.add(format!("PRODUCT_DEFINITION_FORMATION('','',#{product})"));
    let pd = w.add(format!("PRODUCT_DEFINITION('design','',#{pdf},#{pdc})"));
    let pds = w.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{pd})"));
    w.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{pds},#{rep})"));
    w.add(format!("PRODUCT_RELATED_PRODUCT_CATEGORY('part','',(#{product}))"));
    let filename = path.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default().replace('\'', "''");
    let mut out = String::new();
    writeln!(out, "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Triceratops model'),'2;1');\nFILE_NAME('{filename}','',('Triceratops'),(''),'Triceratops','Triceratops','');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\nENDSEC;\nDATA;")?;
    for l in &w.lines {
        writeln!(out, "{l}")?;
    }
    writeln!(out, "ENDSEC;\nEND-ISO-10303-21;")?;
    write_text(path, &out)?;
    Ok(count)
}
