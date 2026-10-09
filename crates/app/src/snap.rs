//! Object snaps (Osnap): End, Near, Point, Mid, Cen, Int, Perp, Tan, Quad, Knot, Vertex,
//! plus Project and Disable, following Triceratops' persistent object snap behaviour.

use glam::{DVec2, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{NurbsCurve, Plane};

use crate::camera::{Camera, ViewRect};
use crate::render::scene::SceneCache;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SnapKind {
    End,
    Near,
    Point,
    Mid,
    Cen,
    Int,
    Perp,
    Tan,
    Quad,
    Knot,
    Vertex,
}

impl SnapKind {
    pub fn label(&self) -> &'static str {
        match self {
            SnapKind::End => "End",
            SnapKind::Near => "Near",
            SnapKind::Point => "Point",
            SnapKind::Mid => "Mid",
            SnapKind::Cen => "Cen",
            SnapKind::Int => "Int",
            SnapKind::Perp => "Perp",
            SnapKind::Tan => "Tan",
            SnapKind::Quad => "Quad",
            SnapKind::Knot => "Knot",
            SnapKind::Vertex => "Vertex",
        }
    }

    /// Lower wins when several snaps are within the snap radius.
    fn priority(&self) -> f64 {
        match self {
            SnapKind::Point => 0.0,
            SnapKind::End => 1.0,
            SnapKind::Int => 2.0,
            SnapKind::Vertex => 2.5,
            SnapKind::Knot => 3.0,
            SnapKind::Quad => 3.5,
            SnapKind::Mid => 4.0,
            SnapKind::Cen => 4.5,
            SnapKind::Perp => 5.0,
            SnapKind::Tan => 5.5,
            SnapKind::Near => 8.0,
        }
    }
}

/// Persistent object snap toggles (the Osnap control).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct OsnapSettings {
    pub end: bool,
    pub near: bool,
    pub point: bool,
    pub mid: bool,
    pub cen: bool,
    pub int: bool,
    pub perp: bool,
    pub tan: bool,
    pub quad: bool,
    pub knot: bool,
    pub vertex: bool,
    pub project: bool,
    pub disable: bool,
    /// Snap activation radius in pixels (Options > Modeling Aids).
    pub radius_px: f64,
}

impl Default for OsnapSettings {
    fn default() -> Self {
        Self {
            end: true,
            near: false,
            point: true,
            mid: true,
            cen: true,
            int: true,
            perp: false,
            tan: false,
            quad: false,
            knot: false,
            vertex: false,
            project: false,
            disable: false,
            radius_px: 15.0,
        }
    }
}

impl OsnapSettings {
    pub fn enabled(&self, k: SnapKind) -> bool {
        if self.disable {
            return false;
        }
        match k {
            SnapKind::End => self.end,
            SnapKind::Near => self.near,
            SnapKind::Point => self.point,
            SnapKind::Mid => self.mid,
            SnapKind::Cen => self.cen,
            SnapKind::Int => self.int,
            SnapKind::Perp => self.perp,
            SnapKind::Tan => self.tan,
            SnapKind::Quad => self.quad,
            SnapKind::Knot => self.knot,
            SnapKind::Vertex => self.vertex,
        }
    }

    /// Toggle state ignoring the Disable button.
    pub fn enabled_raw(&self, k: SnapKind) -> bool {
        match k {
            SnapKind::End => self.end,
            SnapKind::Near => self.near,
            SnapKind::Point => self.point,
            SnapKind::Mid => self.mid,
            SnapKind::Cen => self.cen,
            SnapKind::Int => self.int,
            SnapKind::Perp => self.perp,
            SnapKind::Tan => self.tan,
            SnapKind::Quad => self.quad,
            SnapKind::Knot => self.knot,
            SnapKind::Vertex => self.vertex,
        }
    }

    pub fn any(&self) -> bool {
        !self.disable
            && (self.end || self.near || self.point || self.mid || self.cen || self.int || self.perp || self.tan || self.quad || self.knot || self.vertex)
    }

    pub fn set(&mut self, k: SnapKind, v: bool) {
        match k {
            SnapKind::End => self.end = v,
            SnapKind::Near => self.near = v,
            SnapKind::Point => self.point = v,
            SnapKind::Mid => self.mid = v,
            SnapKind::Cen => self.cen = v,
            SnapKind::Int => self.int = v,
            SnapKind::Perp => self.perp = v,
            SnapKind::Tan => self.tan = v,
            SnapKind::Quad => self.quad = v,
            SnapKind::Knot => self.knot = v,
            SnapKind::Vertex => self.vertex = v,
        }
    }

    pub const ALL: [SnapKind; 11] = [
        SnapKind::End,
        SnapKind::Near,
        SnapKind::Point,
        SnapKind::Mid,
        SnapKind::Cen,
        SnapKind::Int,
        SnapKind::Perp,
        SnapKind::Tan,
        SnapKind::Quad,
        SnapKind::Knot,
        SnapKind::Vertex,
    ];
}

#[derive(Clone, Copy, Debug)]
pub struct SnapHit {
    pub point: DVec3,
    pub kind: SnapKind,
    pub object: ObjectId,
}

pub struct SnapQuery<'a> {
    pub cache: &'a SceneCache,
    pub doc: &'a Document,
    pub cam: &'a Camera,
    pub rect: &'a ViewRect,
    pub cursor: DVec2,
    pub base: Option<DVec3>,
    pub cplane: &'a Plane,
    pub settings: &'a OsnapSettings,
    /// One-shot override: only this snap for one pick.
    pub one_shot: Option<SnapKind>,
}

impl SnapQuery<'_> {
    fn enabled(&self, k: SnapKind) -> bool {
        match self.one_shot {
            Some(o) => o == k,
            None => self.settings.enabled(k),
        }
    }

    /// Find the best object snap within the snap radius.
    pub fn find(&self) -> Option<SnapHit> {
        if self.one_shot.is_none() && !self.settings.any() {
            return None;
        }
        let r = self.settings.radius_px;
        let mut best: Option<(f64, SnapHit)> = None;
        let mut consider = |p: DVec3, kind: SnapKind, id: ObjectId, best: &mut Option<(f64, SnapHit)>| {
            let Some((s, _)) = self.cam.project(self.rect, p) else { return };
            let d = (s - self.cursor).length();
            if d > r {
                return;
            }
            let score = d + kind.priority() * 2.0;
            if best.as_ref().map_or(true, |b| score < b.0) {
                *best = Some((score, SnapHit { point: p, kind, object: id }));
            }
        };

        // precomputed point snaps
        for od in &self.cache.objects {
            let Some(obj) = self.doc.get(od.id) else { continue };
            if !self.doc.is_visible(obj) {
                continue;
            }
            // cheap reject: bounding box far from cursor
            if !self.bbox_near(&od.bbox, r + 2.0) {
                continue;
            }
            for (p, kind) in &od.snaps {
                if self.enabled(*kind) {
                    consider(*p, *kind, od.id, &mut best);
                }
            }
            if let Geometry::Curve(c) = &obj.geometry {
                if self.enabled(SnapKind::Quad) {
                    for p in quad_points(c, self.cplane) {
                        consider(p, SnapKind::Quad, od.id, &mut best);
                    }
                }
                let needs_curve = self.enabled(SnapKind::Near) || self.enabled(SnapKind::Perp) || self.enabled(SnapKind::Tan);
                if needs_curve {
                    if let Some(t_near) = self.near_param(c, &od.lines) {
                        if self.enabled(SnapKind::Near) {
                            consider(c.point_at(t_near), SnapKind::Near, od.id, &mut best);
                        }
                        if let Some(b) = self.base {
                            if self.enabled(SnapKind::Perp) {
                                if let Some(t) = perp_param(c, b, t_near) {
                                    consider(c.point_at(t), SnapKind::Perp, od.id, &mut best);
                                }
                            }
                            if self.enabled(SnapKind::Tan) {
                                if let Some(t) = tan_param(c, b, t_near) {
                                    consider(c.point_at(t), SnapKind::Tan, od.id, &mut best);
                                }
                            }
                        }
                    }
                }
            }
        }
        if self.enabled(SnapKind::Int) {
            for (p, id) in self.intersections(r) {
                consider(p, SnapKind::Int, id, &mut best);
            }
        }
        best.map(|(_, mut h)| {
            if self.settings.project {
                h.point = self.cplane.closest_point(h.point);
            }
            h
        })
    }

    fn bbox_near(&self, bb: &tri_geom::BoundingBox, margin: f64) -> bool {
        if bb.is_empty() {
            return false;
        }
        let mut min = DVec2::splat(f64::INFINITY);
        let mut max = DVec2::splat(f64::NEG_INFINITY);
        let mut any = false;
        for c in bb.corners() {
            if let Some((s, _)) = self.cam.project(self.rect, c) {
                min = min.min(s);
                max = max.max(s);
                any = true;
            }
        }
        if !any {
            return true;
        }
        self.cursor.x >= min.x - margin && self.cursor.x <= max.x + margin && self.cursor.y >= min.y - margin && self.cursor.y <= max.y + margin
    }

    /// Parameter of the curve point closest to the cursor on screen, if within radius.
    fn near_param(&self, c: &NurbsCurve, segments: &[DVec3]) -> Option<f64> {
        let r = self.settings.radius_px;
        let mut best: Option<(f64, DVec3)> = None;
        for seg in segments.chunks(2) {
            let (Some((a, _)), Some((b, _))) = (self.cam.project(self.rect, seg[0]), self.cam.project(self.rect, seg[1])) else {
                continue;
            };
            let ab = b - a;
            let l2 = ab.length_squared();
            let t = if l2 < 1e-12 { 0.0 } else { ((self.cursor - a).dot(ab) / l2).clamp(0.0, 1.0) };
            let d = (a + ab * t - self.cursor).length();
            if d <= r && best.map_or(true, |b| d < b.0) {
                best = Some((d, seg[0] + (seg[1] - seg[0]) * t));
            }
        }
        best.map(|(_, p)| c.closest_parameter(p))
    }

    /// Curve/curve intersections near the cursor (tessellation based).
    fn intersections(&self, r: f64) -> Vec<(DVec3, ObjectId)> {
        let mut near: Vec<(&crate::render::scene::ObjectDisplay, Vec<usize>)> = Vec::new();
        for od in &self.cache.objects {
            if !od.is_curve || !self.bbox_near(&od.bbox, r + 2.0) {
                continue;
            }
            let Some(obj) = self.doc.get(od.id) else { continue };
            if !self.doc.is_visible(obj) {
                continue;
            }
            // segments whose projection is within radius of the cursor
            let mut idx = Vec::new();
            for (i, seg) in od.lines.chunks(2).enumerate() {
                if let (Some((a, _)), Some((b, _))) = (self.cam.project(self.rect, seg[0]), self.cam.project(self.rect, seg[1])) {
                    let ab = b - a;
                    let l2 = ab.length_squared();
                    let t = if l2 < 1e-12 { 0.0 } else { ((self.cursor - a).dot(ab) / l2).clamp(0.0, 1.0) };
                    if (a + ab * t - self.cursor).length() <= r * 1.5 {
                        idx.push(i);
                    }
                }
            }
            if !idx.is_empty() {
                near.push((od, idx));
            }
        }
        let tol = self.cam.world_per_pixel(self.rect.h) * 0.5;
        let mut out = Vec::new();
        for i in 0..near.len() {
            for j in i..near.len() {
                let (oa, ia) = &near[i];
                let (ob, ib) = &near[j];
                for &sa in ia {
                    for &sb in ib {
                        if i == j && sa.abs_diff(sb) <= 1 {
                            continue;
                        }
                        let a0 = oa.lines[2 * sa];
                        let a1 = oa.lines[2 * sa + 1];
                        let b0 = ob.lines[2 * sb];
                        let b1 = ob.lines[2 * sb + 1];
                        if let Some(p) = segment_intersection(a0, a1, b0, b1, tol) {
                            out.push((p, oa.id));
                        }
                    }
                }
            }
        }
        out
    }
}

/// Closest points between two segments; returns the midpoint when they are within `tol`.
fn segment_intersection(a0: DVec3, a1: DVec3, b0: DVec3, b1: DVec3, tol: f64) -> Option<DVec3> {
    let u = a1 - a0;
    let v = b1 - b0;
    let w = a0 - b0;
    let a = u.dot(u);
    let b = u.dot(v);
    let c = v.dot(v);
    let d = u.dot(w);
    let e = v.dot(w);
    let den = a * c - b * b;
    let (s, t) = if den.abs() < 1e-14 {
        return None;
    } else {
        (((b * e - c * d) / den).clamp(0.0, 1.0), ((a * e - b * d) / den).clamp(0.0, 1.0))
    };
    let pa = a0 + u * s;
    let pb = b0 + v * t;
    if (pa - pb).length() <= tol {
        Some((pa + pb) * 0.5)
    } else {
        None
    }
}

/// Points where the curve tangent is parallel to a construction plane axis.
pub fn quad_points(c: &NurbsCurve, cplane: &Plane) -> Vec<DVec3> {
    let (t0, t1) = c.domain();
    let n = 32 * c.control_points.len();
    let mut out = Vec::new();
    for axis in [cplane.x_axis, cplane.y_axis] {
        let f = |t: f64| c.derivative_at(t).dot(axis);
        let mut prev_t = t0;
        let mut prev = f(t0);
        for i in 1..=n {
            let t = t0 + (t1 - t0) * i as f64 / n as f64;
            let v = f(t);
            if prev == 0.0 || (prev < 0.0) != (v < 0.0) {
                // bisection
                let (mut a, mut b) = (prev_t, t);
                let mut fa = prev;
                for _ in 0..40 {
                    let m = (a + b) * 0.5;
                    let fm = f(m);
                    if (fa < 0.0) == (fm < 0.0) {
                        a = m;
                        fa = fm;
                    } else {
                        b = m;
                    }
                }
                out.push(c.point_at((a + b) * 0.5));
            }
            prev_t = t;
            prev = v;
        }
    }
    out
}

/// Parameter where the line from `base` meets the curve perpendicularly (Newton from `t_start`).
fn perp_param(c: &NurbsCurve, base: DVec3, t_start: f64) -> Option<f64> {
    let (t0, t1) = c.domain();
    let mut t = t_start;
    let h = (t1 - t0) * 1e-6;
    for _ in 0..30 {
        let p = c.point_at(t);
        let d1 = c.derivative_at(t);
        let d2 = (c.derivative_at((t + h).min(t1)) - c.derivative_at((t - h).max(t0))) / (2.0 * h);
        let f = (p - base).dot(d1);
        let fp = d1.dot(d1) + (p - base).dot(d2);
        if fp.abs() < 1e-14 {
            break;
        }
        let nt = (t - f / fp).clamp(t0, t1);
        if (nt - t).abs() < 1e-12 {
            t = nt;
            break;
        }
        t = nt;
    }
    let p = c.point_at(t);
    let d1 = c.derivative_at(t).normalize_or(DVec3::X);
    let v = (p - base).normalize_or(DVec3::X);
    if v.dot(d1).abs() < 1e-3 {
        Some(t)
    } else {
        None
    }
}

/// Parameter where the line from `base` is tangent to the curve.
fn tan_param(c: &NurbsCurve, base: DVec3, t_start: f64) -> Option<f64> {
    let (t0, t1) = c.domain();
    // minimise |(C - base) x C'| / (|C-base||C'|) by sampling near t_start then golden search
    let g = |t: f64| {
        let p = c.point_at(t) - base;
        let d = c.derivative_at(t);
        let cr = p.cross(d).length();
        cr / (p.length() * d.length()).max(1e-12)
    };
    let span = (t1 - t0) * 0.25;
    let (mut a, mut b) = ((t_start - span).max(t0), (t_start + span).min(t1));
    let phi = 0.618_033_988_749_895;
    for _ in 0..60 {
        let x1 = b - phi * (b - a);
        let x2 = a + phi * (b - a);
        if g(x1) < g(x2) {
            b = x2;
        } else {
            a = x1;
        }
    }
    let t = (a + b) * 0.5;
    if g(t) < 2e-3 {
        Some(t)
    } else {
        None
    }
}
