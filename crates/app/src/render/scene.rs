//! Tessellation cache, picking and precomputed object snap points.

use glam::{DVec2, DVec3};
use tri_doc::{Document, Geometry, ObjectId};
use tri_geom::{BoundingBox, Mesh, MeshDensity, NurbsCurve, NurbsSurface, Ray};

use crate::camera::{Camera, ViewRect};
use crate::snap::SnapKind;

/// Display data for one document object.
pub struct ObjectDisplay {
    pub id: ObjectId,
    /// Line segments as consecutive pairs of points (curves, edges, isocurves).
    pub lines: Vec<DVec3>,
    /// Surface edge segments only (drawn even when isocurves are hidden).
    pub edges: Vec<DVec3>,
    pub mesh: Option<Mesh>,
    pub points: Vec<DVec3>,
    pub snaps: Vec<(DVec3, SnapKind)>,
    pub bbox: BoundingBox,
    pub is_curve: bool,
}

#[derive(Default)]
pub struct SceneCache {
    revision: Option<u64>,
    /// View tolerance tier the lines were tessellated for (see `view_tolerance_tier`).
    pub tier: Option<i32>,
    pub objects: Vec<ObjectDisplay>,
}

pub fn curve_polyline(c: &NurbsCurve, tol: f64) -> Vec<DVec3> {
    c.tessellate_adaptive(tol)
}

fn polyline_to_segments(pts: &[DVec3], out: &mut Vec<DVec3>) {
    for w in pts.windows(2) {
        out.push(w[0]);
        out.push(w[1]);
    }
}

/// Edges and isocurves of a surface as line segments.
pub fn surface_lines(s: &NurbsSurface, tol: f64, edges: &mut Vec<DVec3>, isos: &mut Vec<DVec3>) {
    for c in s.boundary_curves() {
        polyline_to_segments(&curve_polyline(&c, tol), edges);
    }
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    let mut ku: Vec<f64> = s.knots_u[s.degree_u + 1..s.knots_u.len() - s.degree_u - 1].to_vec();
    ku.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    let mut kv: Vec<f64> = s.knots_v[s.degree_v + 1..s.knots_v.len() - s.degree_v - 1].to_vec();
    kv.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    if ku.is_empty() && s.degree_u > 1 {
        ku.push((u0 + u1) * 0.5);
    }
    if kv.is_empty() && s.degree_v > 1 {
        kv.push((v0 + v1) * 0.5);
    }
    for u in ku {
        if u > u0 + 1e-12 && u < u1 - 1e-12 {
            polyline_to_segments(&curve_polyline(&s.isocurve_u(u), tol), isos);
        }
    }
    for v in kv {
        if v > v0 + 1e-12 && v < v1 - 1e-12 {
            polyline_to_segments(&curve_polyline(&s.isocurve_v(v), tol), isos);
        }
    }
}

/// Edges and isocurves of a (possibly trimmed) face.
pub fn face_lines(f: &tri_geom::Face, tol: f64, edges: &mut Vec<DVec3>, isos: &mut Vec<DVec3>) {
    if !f.is_trimmed() {
        surface_lines(&f.surface, tol, edges, isos);
        return;
    }
    for poly in f.display_edges(tol) {
        polyline_to_segments(&poly, edges);
    }
    // isocurves clipped to the trim region (planar faces have none, like Triceratops)
    let s = &f.surface;
    if s.degree_u > 1 || s.degree_v > 1 {
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let params = |knots: &[f64], degree: usize, lo: f64, hi: f64| -> Vec<f64> {
            let mut k: Vec<f64> = knots[degree + 1..knots.len() - degree - 1].to_vec();
            k.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
            if k.is_empty() && degree > 1 {
                k.push((lo + hi) * 0.5);
            }
            k.into_iter().filter(|x| *x > lo + 1e-12 && *x < hi - 1e-12).collect()
        };
        let loops = f.effective_loops();
        // crossings of the line (const = c along `axis`) with the trim loops, sorted along the other axis
        let crossings = |c: f64, axis_u: bool| -> Vec<f64> {
            let mut out = Vec::new();
            for l in &loops {
                for k in 0..l.len() {
                    let (a, b) = (l[k], l[(k + 1) % l.len()]);
                    let (pa, pb, qa, qb) = if axis_u { (a.x, b.x, a.y, b.y) } else { (a.y, b.y, a.x, b.x) };
                    if (pa - c) * (pb - c) < 0.0 {
                        out.push(qa + (c - pa) / (pb - pa) * (qb - qa));
                    }
                }
            }
            out.sort_by(|a, b| a.partial_cmp(b).unwrap());
            out
        };
        for u in params(&s.knots_u, s.degree_u, u0, u1) {
            let vs = crossings(u, true);
            for pair in vs.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let pts = tri_geom::display_mesh::uv_polyline(s, DVec2::new(u, pair[0]), DVec2::new(u, pair[1]), tol);
                polyline_to_segments(&pts, isos);
            }
        }
        for v in params(&s.knots_v, s.degree_v, v0, v1) {
            let us = crossings(v, false);
            for pair in us.chunks(2) {
                if pair.len() < 2 {
                    break;
                }
                let pts = tri_geom::display_mesh::uv_polyline(s, DVec2::new(pair[0], v), DVec2::new(pair[1], v), tol);
                polyline_to_segments(&pts, isos);
            }
        }
    }
}

pub fn display_tolerance(bb: &BoundingBox) -> f64 {
    (bb.diagonal() * 0.0015).max(1e-4)
}

/// Tolerance for curves, edges and isocurves of an object: the view tolerance (a fraction
/// of a pixel at the current zoom), never coarser than the zoomed-out default and never
/// finer than the object can need.
pub fn line_tolerance(bb: &BoundingBox, view_tol: f64) -> f64 {
    let coarse = display_tolerance(bb);
    let fine = (bb.diagonal() * 1e-6).max(1e-7);
    view_tol.clamp(fine, coarse.max(fine))
}

/// View tolerance quantised to powers of two, so zooming re-tessellates only at each step.
pub fn view_tolerance_tier(world_per_pixel: f64) -> i32 {
    (world_per_pixel.max(1e-12) * 0.35).log2().floor() as i32
}

fn curve_snaps(c: &NurbsCurve, out: &mut Vec<(DVec3, SnapKind)>) {
    let (t0, t1) = c.domain();
    out.push((c.start_point(), SnapKind::End));
    out.push((c.end_point(), SnapKind::End));
    if c.degree == 1 {
        for w in c.control_points.windows(2) {
            out.push((w[0], SnapKind::End));
            out.push(((w[0] + w[1]) * 0.5, SnapKind::Mid));
        }
    } else {
        out.push((c.point_at((t0 + t1) * 0.5), SnapKind::Mid));
        // knots
        let mut ks: Vec<f64> = c.knots[c.degree + 1..c.knots.len() - c.degree - 1].to_vec();
        ks.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
        for k in ks {
            if k > t0 + 1e-12 && k < t1 - 1e-12 {
                out.push((c.point_at(k), SnapKind::Knot));
            }
        }
    }
    if let Some(center) = circle_center(c) {
        out.push((center, SnapKind::Cen));
    } else if c.is_closed() {
        if let Some(center) = closed_curve_center(c) {
            out.push((center, SnapKind::Cen));
        }
    }
}

/// Centre of a closed curve: ellipse centre for rational degree-2 curves, otherwise the
/// area centroid of the (tessellated) closed curve.
pub fn closed_curve_center(c: &NurbsCurve) -> Option<DVec3> {
    if c.degree == 2 && c.is_rational() && c.control_points.len() == 9 {
        return Some((c.control_points[0] + c.control_points[4]) * 0.5);
    }
    let pts = if c.degree == 1 { c.control_points.clone() } else { c.tessellate(8) };
    if pts.len() < 3 {
        return None;
    }
    // polygon centroid in the best-fit plane (Newell normal)
    let mut n = DVec3::ZERO;
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        n += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
    }
    if n.length_squared() < 1e-20 {
        return None;
    }
    let plane = tri_geom::Plane::from_normal(pts[0], n.normalize());
    let uv: Vec<(f64, f64)> = pts.iter().map(|p| { let (u, v, _) = plane.closest_parameters(*p); (u, v) }).collect();
    let mut area = 0.0;
    let (mut cx, mut cy) = (0.0, 0.0);
    for i in 0..uv.len() {
        let (x0, y0) = uv[i];
        let (x1, y1) = uv[(i + 1) % uv.len()];
        let cross = x0 * y1 - x1 * y0;
        area += cross;
        cx += (x0 + x1) * cross;
        cy += (x0 + x1 - x0 + y0 + y1 - x1 + x1 - y0 - y1 + y0 + y1) * 0.0 + (y0 + y1) * cross;
    }
    if area.abs() < 1e-18 {
        return None;
    }
    let f = 1.0 / (3.0 * area);
    Some(plane.point_at(cx * f, cy * f))
}

/// Centre of a curve that is an arc or circle (rational degree 2 with constant distance).
pub fn circle_center(c: &NurbsCurve) -> Option<DVec3> {
    if c.degree != 2 || !c.is_rational() || c.control_points.len() < 3 {
        return None;
    }
    let a = c.start_point();
    let m = c.point_at((c.domain().0 + c.domain().1) * 0.5);
    let b = c.end_point();
    let (center, r, _) = if (a - b).length() < 1e-9 {
        let q = c.point_at(c.domain().0 + (c.domain().1 - c.domain().0) * 0.25);
        tri_geom::curve::circle_through(a, q, m).ok()?
    } else {
        tri_geom::curve::circle_through(a, m, b).ok()?
    };
    // verify a few samples
    for i in 0..8 {
        let t = c.domain().0 + (c.domain().1 - c.domain().0) * (i as f64 + 0.37) / 8.0;
        if ((c.point_at(t) - center).length() - r).abs() > 1e-6 * r.max(1.0) {
            return None;
        }
    }
    Some(center)
}

/// Display data of an object. `old_mesh` (when the geometry is unchanged and only the view
/// tolerance moved) is reused instead of tessellating the shading mesh again.
fn build(doc: &Document, o: &tri_doc::Object, view_tol: f64, old_mesh: Option<Option<Mesh>>) -> ObjectDisplay {
    let bbox = doc.object_bbox(o);
    let skip_mesh = old_mesh.is_some();
    let mut d = build_geometry(&o.geometry, &bbox, view_tol, skip_mesh);
    if let Geometry::Instance { .. } = &o.geometry {
        for (g, _) in doc.instance_geometry(&o.geometry) {
            let part = build_geometry(&g, &bbox, view_tol, skip_mesh);
            d.lines.extend(part.lines);
            d.edges.extend(part.edges);
            d.points.extend(part.points);
            d.snaps.extend(part.snaps);
            if let Some(pm) = part.mesh {
                match &mut d.mesh {
                    Some(m) => m.append(&pm),
                    None => d.mesh = Some(pm),
                }
            }
        }
    }
    if let Some(m) = old_mesh {
        d.mesh = m;
    }
    if let Geometry::Instance { .. } = &o.geometry {
        d.is_curve = d.mesh.is_none();
    }
    d.id = o.id;
    d.bbox = bbox;
    d
}

/// Display data for geometry that is not in the document (the Locust preview).
pub fn preview_display(geometry: &Geometry, view_tier: i32) -> ObjectDisplay {
    let bbox = geometry.bounding_box();
    build_geometry(geometry, &bbox, 2f64.powi(view_tier), false)
}

/// Display data of one geometry; `skip_mesh` leaves `mesh` empty (the caller reuses one).
fn build_geometry(geometry: &Geometry, bbox: &tri_geom::BoundingBox, view_tol: f64, skip_mesh: bool) -> ObjectDisplay {
    let bbox = *bbox;
    let tol = line_tolerance(&bbox, view_tol);
    let density = MeshDensity::for_size(bbox.diagonal());
    let mut lines = Vec::new();
    let mut edges = Vec::new();
    let mut mesh = None;
    let mut points = Vec::new();
    let mut snaps = Vec::new();
    let mut is_curve = false;
    match geometry {
        Geometry::Instance { .. } => {}
        Geometry::Point(p) => {
            points.push(*p);
            snaps.push((*p, SnapKind::Point));
        }
        Geometry::Curve(c) => {
            is_curve = true;
            polyline_to_segments(&curve_polyline(c, tol), &mut lines);
            curve_snaps(c, &mut snaps);
        }
        Geometry::Surface(s) => {
            surface_lines(s, tol, &mut edges, &mut lines);
            if !skip_mesh {
                mesh = Some(s.display_mesh(&density));
            }
            for c in s.boundary_curves() {
                curve_snaps(&c, &mut snaps);
            }
            if s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2 {
                let c = s.control_points.iter().fold(DVec3::ZERO, |a, p| a + *p) / 4.0;
                snaps.push((c, SnapKind::Cen));
            }
        }
        Geometry::Brep(b) => {
            for f in &b.faces {
                face_lines(f, tol, &mut edges, &mut lines);
                for c in f.edge_curves(tol) {
                    curve_snaps(&c, &mut snaps);
                }
                if f.is_trimmed() && f.surface.degree_u == 1 && f.surface.degree_v == 1 {
                    let (p, _) = f.sample();
                    snaps.push((p, SnapKind::Cen));
                }
            }
            if !skip_mesh {
                mesh = Some(b.display_mesh(&density));
            }
        }
        Geometry::Mesh(m) => {
            for (a, b) in m.edges() {
                edges.push(m.vertices[a as usize]);
                edges.push(m.vertices[b as usize]);
            }
            let mut mm = m.clone();
            if mm.normals.len() != mm.vertices.len() {
                mm.compute_normals();
            }
            mesh = Some(mm);
            for v in &m.vertices {
                snaps.push((*v, SnapKind::Vertex));
            }
        }
    }
    ObjectDisplay { id: 0, lines, edges, mesh, points, snaps, bbox, is_curve }
}

pub struct PickHit {
    pub id: ObjectId,
    pub depth: f64,
    /// Location of the pick on the object.
    pub point: DVec3,
}

impl SceneCache {
    /// Rebuild after a document change, or only the lines when the zoom moved to another
    /// tolerance tier (shading meshes do not depend on the view and are kept).
    pub fn update(&mut self, doc: &Document, tier: i32) {
        let same_doc = self.revision == Some(doc.revision);
        if same_doc && self.tier == Some(tier) {
            return;
        }
        let view_tol = 2f64.powi(tier);
        if same_doc {
            let old = std::mem::take(&mut self.objects);
            self.objects = doc.objects().zip(old).map(|(o, prev)| build(doc, o, view_tol, Some(prev.mesh))).collect();
        } else {
            self.objects = doc.objects().map(|o| build(doc, o, view_tol, None)).collect();
        }
        self.revision = Some(doc.revision);
        self.tier = Some(tier);
    }

    pub fn get(&self, id: ObjectId) -> Option<&ObjectDisplay> {
        self.objects.iter().find(|o| o.id == id)
    }

    /// All objects under the cursor within `tol_px`, nearest first (for the selection menu).
    pub fn pick_all(&self, doc: &Document, cam: &Camera, rect: &ViewRect, x: f64, y: f64, tol_px: f64) -> Vec<PickHit> {
        let cursor = DVec2::new(x, y);
        let ray = cam.ray_at(rect, x, y);
        let mut hits: Vec<PickHit> = Vec::new();
        for od in &self.objects {
            let Some(obj) = doc.get(od.id) else { continue };
            if !doc.is_selectable(obj) {
                continue;
            }
            let mut best: Option<(f64, DVec3)> = None;
            let consider = |z: f64, p: DVec3, best: &mut Option<(f64, DVec3)>| {
                if best.is_none_or(|b| z < b.0) {
                    *best = Some((z, p));
                }
            };
            for p in &od.points {
                if let Some((s, z)) = cam.project(rect, *p) {
                    if (s - cursor).length() <= tol_px + 2.0 {
                        consider(z - 1e-3, *p, &mut best);
                    }
                }
            }
            for seg in od.lines.chunks(2).chain(od.edges.chunks(2)) {
                let (Some((a, za)), Some((b, zb))) = (cam.project(rect, seg[0]), cam.project(rect, seg[1])) else {
                    continue;
                };
                let (d, t) = point_segment_distance_2d(cursor, a, b);
                if d <= tol_px {
                    consider(za + (zb - za) * t - 1e-4, seg[0] + (seg[1] - seg[0]) * t, &mut best);
                }
            }
            if best.is_none() {
                if let Some(m) = &od.mesh {
                    if let Some(t) = m.intersect_ray(&ray) {
                        if let Some((_, z)) = cam.project(rect, ray.at(t)) {
                            consider(z, ray.at(t), &mut best);
                        }
                    }
                }
            }
            if let Some((z, p)) = best {
                hits.push(PickHit { id: od.id, depth: z, point: p });
            }
        }
        hits.sort_by(|a, b| a.depth.partial_cmp(&b.depth).unwrap_or(std::cmp::Ordering::Equal));
        hits
    }

    pub fn pick(&self, doc: &Document, cam: &Camera, rect: &ViewRect, x: f64, y: f64, tol_px: f64) -> Option<PickHit> {
        self.pick_all(doc, cam, rect, x, y, tol_px).into_iter().next()
    }

    /// Objects inside a screen rectangle. `crossing` selects anything touching the rectangle.
    pub fn pick_window(&self, doc: &Document, cam: &Camera, rect: &ViewRect, min: DVec2, max: DVec2, crossing: bool) -> Vec<ObjectId> {
        let inside = |p: DVec2| p.x >= min.x && p.x <= max.x && p.y >= min.y && p.y <= max.y;
        let mut out = Vec::new();
        for od in &self.objects {
            let Some(obj) = doc.get(od.id) else { continue };
            if !doc.is_selectable(obj) {
                continue;
            }
            let pts: &[DVec3] = if !od.points.is_empty() {
                &od.points
            } else if !od.edges.is_empty() {
                &od.edges
            } else if !od.lines.is_empty() {
                &od.lines
            } else if let Some(m) = &od.mesh {
                &m.vertices
            } else {
                continue;
            };
            let mut any = false;
            let mut all = true;
            for p in pts {
                match cam.project(rect, *p) {
                    Some((s, _)) if inside(s) => any = true,
                    _ => all = false,
                }
                if crossing && any {
                    break;
                }
                if !crossing && !all {
                    break;
                }
            }
            if (!crossing && all && !pts.is_empty()) || (crossing && any) {
                out.push(od.id);
            } else if crossing {
                let crosses = od.lines.chunks(2).chain(od.edges.chunks(2)).any(|seg| match (cam.project(rect, seg[0]), cam.project(rect, seg[1])) {
                    (Some((a, _)), Some((b, _))) => segment_intersects_rect(a, b, min, max),
                    _ => false,
                });
                if crosses {
                    out.push(od.id);
                }
            }
        }
        out
    }

    pub fn ray_hit_any(&self, ray: &Ray) -> Option<f64> {
        self.objects
            .iter()
            .filter_map(|o| o.mesh.as_ref().and_then(|m| m.intersect_ray(ray)))
            .fold(None, |a, t| Some(a.map_or(t, |x: f64| x.min(t))))
    }
}

fn point_segment_distance_2d(p: DVec2, a: DVec2, b: DVec2) -> (f64, f64) {
    let ab = b - a;
    let l2 = ab.length_squared();
    if l2 < 1e-12 {
        return ((p - a).length(), 0.0);
    }
    let t = ((p - a).dot(ab) / l2).clamp(0.0, 1.0);
    ((a + ab * t - p).length(), t)
}

fn segment_intersects_rect(a: DVec2, b: DVec2, min: DVec2, max: DVec2) -> bool {
    let d = b - a;
    let mut t0 = 0.0f64;
    let mut t1 = 1.0f64;
    for (p, q) in [(-d.x, a.x - min.x), (d.x, max.x - a.x), (-d.y, a.y - min.y), (d.y, max.y - a.y)] {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return false;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}
