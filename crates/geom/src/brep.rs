//! Boundary representation: faces (optionally trimmed by UV loops) grouped into
//! polysurfaces. A closed polysurface is what Triceratops treats as a solid.

use glam::{DAffine3, DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::exact::ExactBody;
use crate::{BoundingBox, Mesh, NurbsCurve, NurbsSurface, Plane, Result, GeomError};

/// A surface with optional trim loops in parameter space. The first loop is the
/// outer boundary (counter-clockwise in UV), the others are holes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub surface: NurbsSurface,
    pub loops: Vec<Vec<DVec2>>,
}

impl Face {
    pub fn untrimmed(surface: NurbsSurface) -> Self {
        Self { surface, loops: Vec::new() }
    }

    pub fn is_trimmed(&self) -> bool {
        !self.loops.is_empty()
    }

    /// Planar face bounded by closed 3D polylines (first = outer, rest = holes).
    pub fn planar(plane: &Plane, loops: Vec<Vec<DVec3>>) -> Result<Self> {
        if loops.is_empty() || loops[0].len() < 3 {
            return Err(GeomError::Degenerate("planar face needs a closed boundary".into()));
        }
        let mut min = DVec2::splat(f64::INFINITY);
        let mut max = DVec2::splat(f64::NEG_INFINITY);
        let mut uv_loops: Vec<Vec<DVec2>> = Vec::new();
        for l in &loops {
            let mut uv: Vec<DVec2> = l
                .iter()
                .map(|p| {
                    let (u, v, _) = plane.closest_parameters(*p);
                    DVec2::new(u, v)
                })
                .collect();
            if uv.len() > 1 && (uv[0] - uv[uv.len() - 1]).length() < 1e-9 {
                uv.pop();
            }
            dedup_consecutive(&mut uv);
            if uv.len() < 3 {
                continue;
            }
            for p in &uv {
                min = min.min(*p);
                max = max.max(*p);
            }
            uv_loops.push(uv);
        }
        if uv_loops.is_empty() {
            return Err(GeomError::Degenerate("planar face boundary is degenerate".into()));
        }
        // orientation: outer CCW, holes CW
        if signed_area(&uv_loops[0]) < 0.0 {
            uv_loops[0].reverse();
        }
        for h in uv_loops.iter_mut().skip(1) {
            if signed_area(h) > 0.0 {
                h.reverse();
            }
        }
        let pad = (max - min).length() * 0.01 + 1e-6;
        let (u0, u1, v0, v1) = (min.x - pad, max.x + pad, min.y - pad, max.y + pad);
        // plane surface whose parameters are the plane coordinates themselves
        let cps = vec![plane.point_at(u0, v0), plane.point_at(u0, v1), plane.point_at(u1, v0), plane.point_at(u1, v1)];
        let surface = NurbsSurface::new(1, 1, 2, 2, cps, vec![1.0; 4], vec![u0, u0, u1, u1], vec![v0, v0, v1, v1])?;
        Ok(Self { surface, loops: uv_loops })
    }

    /// Planar face from closed planar curves (outer boundary first).
    pub fn from_planar_curves(curves: &[NurbsCurve], tol: f64) -> Result<Self> {
        let outer = curves.first().ok_or_else(|| GeomError::Degenerate("no boundary curve".into()))?;
        let plane = curve_plane(outer).ok_or_else(|| GeomError::Degenerate("boundary curve is not planar".into()))?;
        // sample every curve the way surface tessellation samples an extrusion of it, so a cap
        // and the side it closes share their rim vertices (watertight Boolean input)
        let _ = tol;
        let loops: Vec<Vec<DVec3>> = curves
            .iter()
            .map(|c| {
                let mut pts = c.span_samples(boundary_segs(c.degree));
                if pts.len() > 1 && (pts[0] - pts[pts.len() - 1]).length() < 1e-9 * (1.0 + c.bounding_box().diagonal()) {
                    pts.pop();
                }
                pts
            })
            .collect();
        Self::planar(&plane, loops)
    }

    /// A trim loop with its straight (u, v) segments subdivided at the boundary sampling density
    /// of the surface (so edges shared with other faces hit the same 3-D points).
    pub fn subdivided_loop(&self, l: &[DVec2]) -> Vec<DVec2> {
        let s = &self.surface;
        let planar = s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2;
        if planar || l.len() < 2 {
            return l.to_vec();
        }
        let mut out = Vec::new();
        for k in 0..l.len() {
            let (a, bq) = (l[k], l[(k + 1) % l.len()]);
            let n = self.segment_subdivisions(a, bq);
            for i in 0..n {
                out.push(a + (bq - a) * (i as f64 / n as f64));
            }
        }
        out
    }

    /// Number of pieces a straight (u, v) segment is drawn with (1 on planar faces): the
    /// per-span rule shared by every face so neighbours sample a common edge identically.
    pub fn segment_subdivisions(&self, a: DVec2, b: DVec2) -> usize {
        let s = &self.surface;
        let planar = s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2;
        if planar {
            return 1;
        }
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let du = (u1 - u0).max(1e-12);
        let dv = (v1 - v0).max(1e-12);
        let spans = |knots: &[f64], degree: usize| -> f64 {
            let mut k: Vec<f64> = knots[degree..knots.len() - degree].to_vec();
            k.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
            (k.len().saturating_sub(1)).max(1) as f64
        };
        let per_u = spans(&s.knots_u, s.degree_u) * boundary_segs(s.degree_u) as f64;
        let per_v = spans(&s.knots_v, s.degree_v) * boundary_segs(s.degree_v) as f64;
        (((b.x - a.x) / du).abs() * per_u).max(((b.y - a.y) / dv).abs() * per_v).ceil().clamp(1.0, 256.0) as usize
    }

    /// Triangulation of a trimmed curved face: the subdivided trim loops plus the interior grid
    /// samples are Delaunay-triangulated in normalised (u, v) and triangles outside the region
    /// are dropped. Boundary edges are exact (interior points keep away from the outline) so
    /// adjacent faces share their rim vertices.
    fn curved_trimmed_triangulation(&self, segs_u: usize, segs_v: usize) -> (Vec<DVec2>, Vec<[usize; 3]>) {
        let s = &self.surface;
        let loops: Vec<Vec<DVec2>> = self.loops.iter().map(|l| self.subdivided_loop(l)).collect();
        let us = NurbsSurface::samples(&s.knots_u, s.degree_u, segs_u.max(1));
        let vs = NurbsSurface::samples(&s.knots_v, s.degree_v, segs_v.max(1));
        self.trimmed_triangulation(&loops, &us, &vs)
    }

    /// Triangulation of a trimmed curved face from already subdivided loops and the
    /// parameter values of the interior grid.
    pub(crate) fn trimmed_triangulation(&self, loops: &[Vec<DVec2>], us: &[f64], vs: &[f64]) -> (Vec<DVec2>, Vec<[usize; 3]>) {
        let s = &self.surface;
        let (u0, u1) = s.domain_u();
        let (v0, v1) = s.domain_v();
        let du = (u1 - u0).max(1e-12);
        let dv = (v1 - v0).max(1e-12);
        // metric scaling: lengths of the mid isocurves, so Delaunay sees true aspect ratios
        let len_of = |c: &NurbsCurve| c.span_samples(8).windows(2).map(|w| (w[1] - w[0]).length()).sum::<f64>().max(1e-9);
        let lu = len_of(&s.isocurve_v((v0 + v1) * 0.5));
        let lv = len_of(&s.isocurve_u((u0 + u1) * 0.5));
        let metric = |p: DVec2| DVec2::new((p.x - u0) / du * lu, (p.y - v0) / dv * lv);
        let loops_m: Vec<Vec<DVec2>> = loops.iter().map(|l| l.iter().map(|p| metric(*p)).collect()).collect();
        let mut pts: Vec<DVec2> = loops.iter().flatten().copied().collect();
        // interior grid samples, kept away from the outline so the outline edges stay Delaunay
        let grid_step = {
            let gu = if us.len() > 1 { lu / (us.len() - 1) as f64 } else { lu };
            let gv = if vs.len() > 1 { lv / (vs.len() - 1) as f64 } else { lv };
            gu.min(gv)
        };
        // the loops' segments bucketed in space: both tests below look only at nearby ones
        let index = LoopIndex::new(&loops_m, grid_step.max(1e-9));
        let inside = |p: DVec2| index.inside(p);
        for &u in us {
            for &v in vs {
                let p = metric(DVec2::new(u, v));
                if inside(p) && !index.near_outline(p, grid_step) {
                    pts.push(DVec2::new(u, v));
                }
            }
        }
        // Delaunay on the metric coordinates, jittered to break the co-circular grid cases
        let jitter = |i: usize| -> DVec2 {
            let h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let a = ((h >> 11) as f64 / (1u64 << 53) as f64) - 0.5;
            let bq = (((h.rotate_left(29)) >> 11) as f64 / (1u64 << 53) as f64) - 0.5;
            DVec2::new(a, bq) * 1e-6 * lu.max(lv)
        };
        let pts3: Vec<DVec3> = pts.iter().enumerate().map(|(i, p)| { let m = metric(*p) + jitter(i); DVec3::new(m.x, m.y, 0.0) }).collect();
        let tri_mesh = crate::mesh_ops::delaunay(&pts3, &Plane::world_xy());
        let mut tris = Vec::new();
        for t in tri_mesh.indices.chunks(3) {
            let (a, b, c) = (metric(pts[t[0] as usize]), metric(pts[t[1] as usize]), metric(pts[t[2] as usize]));
            let centroid = (a + b + c) / 3.0;
            if inside(centroid) && (b - a).perp_dot(c - a).abs() > 1e-14 {
                tris.push([t[0] as usize, t[1] as usize, t[2] as usize]);
            }
        }
        (pts, tris)
    }

    /// Trim loops, or the full domain rectangle for an untrimmed face.
    pub fn effective_loops(&self) -> Vec<Vec<DVec2>> {
        if !self.loops.is_empty() {
            return self.loops.clone();
        }
        let (u0, u1) = self.surface.domain_u();
        let (v0, v1) = self.surface.domain_v();
        vec![vec![DVec2::new(u0, v0), DVec2::new(u1, v0), DVec2::new(u1, v1), DVec2::new(u0, v1)]]
    }

    /// Whether a UV point lies inside the face (outer loop minus holes).
    pub fn contains_uv(&self, uv: DVec2) -> bool {
        let loops = self.effective_loops();
        if !point_in_polygon(uv, &loops[0]) {
            return false;
        }
        !loops[1..].iter().any(|h| point_in_polygon(uv, h))
    }

    /// Project a world point onto the face along `dir`; falls back to the closest point.
    pub fn project_uv(&self, p: DVec3, dir: DVec3) -> DVec2 {
        let s = &self.surface;
        if s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2 {
            // planar: intersect the ray with the plane
            let a = s.point_at(s.domain_u().0, s.domain_v().0);
            let n = s.normal_at(s.domain_u().0, s.domain_v().0);
            let denom = dir.dot(n);
            let q = if denom.abs() > 1e-12 { p + dir * ((a - p).dot(n) / denom) } else { p };
            let (u, v) = s.closest_uv(q);
            return DVec2::new(u, v);
        }
        let (u, v) = s.closest_uv(p);
        DVec2::new(u, v)
    }

    /// Split the face by a UV path. Returns the two resulting faces.
    pub fn split_by_uv_path(&self, path: &[DVec2]) -> Option<(Face, Face)> {
        let loops = self.effective_loops();
        let (a, b) = split_loop_by_path(&loops[0], path)?;
        let mut fa = Face { surface: self.surface.clone(), loops: vec![a] };
        let mut fb = Face { surface: self.surface.clone(), loops: vec![b] };
        for h in loops.iter().skip(1) {
            if point_in_polygon(h[0], &fa.loops[0]) {
                fa.loops.push(h.clone());
            } else {
                fb.loops.push(h.clone());
            }
        }
        Some((fa, fb))
    }

    pub fn domain_center(&self) -> DVec2 {
        if let Some(outer) = self.loops.first() {
            let (c, _) = polygon_centroid(outer);
            return c;
        }
        let (u0, u1) = self.surface.domain_u();
        let (v0, v1) = self.surface.domain_v();
        DVec2::new((u0 + u1) * 0.5, (v0 + v1) * 0.5)
    }

    /// A point on the face and its normal (for orientation checks).
    pub fn sample(&self) -> (DVec3, DVec3) {
        let c = self.domain_center();
        (self.surface.point_at(c.x, c.y), self.surface.normal_at(c.x, c.y))
    }

    /// Boundary edges as 3D polylines (trim loops or the untrimmed surface border).
    pub fn edge_polylines(&self, tol: f64) -> Vec<Vec<DVec3>> {
        if self.loops.is_empty() {
            return self.surface.boundary_curves().iter().map(|c| c.tessellate_adaptive(tol)).collect();
        }
        // straight (u, v) segments are curves on a curved surface: subdivide them
        self.loops
            .iter()
            .map(|l| {
                let mut pts: Vec<DVec3> = self.subdivided_loop(l).iter().map(|uv| self.surface.point_at(uv.x, uv.y)).collect();
                if let Some(f) = pts.first().copied() {
                    pts.push(f);
                }
                pts
            })
            .collect()
    }

    /// Boundary polylines through the trim-loop vertices only (no subdivision of the (u, v)
    /// segments): what adjacent faces share exactly, used by the topology checks.
    pub fn edge_polylines_raw(&self, tol: f64) -> Vec<Vec<DVec3>> {
        if self.loops.is_empty() {
            return self.surface.boundary_curves().iter().map(|c| c.tessellate_adaptive(tol)).collect();
        }
        self.loops
            .iter()
            .map(|l| {
                let mut pts: Vec<DVec3> = l.iter().map(|uv| self.surface.point_at(uv.x, uv.y)).collect();
                if let Some(f) = pts.first().copied() {
                    pts.push(f);
                }
                pts
            })
            .collect()
    }

    /// Boundary edges as curves.
    pub fn edge_curves(&self, tol: f64) -> Vec<NurbsCurve> {
        if self.loops.is_empty() {
            return self.surface.boundary_curves().to_vec();
        }
        self.edge_polylines(tol).into_iter().filter_map(|p| NurbsCurve::polyline(p).ok()).collect()
    }

    pub fn tessellate(&self, segs_u: usize, segs_v: usize) -> Mesh {
        self.tessellate_uv(segs_u, segs_v).0
    }

    /// Tessellation plus the surface parameters of every vertex.
    pub fn tessellate_uv(&self, segs_u: usize, segs_v: usize) -> (Mesh, Vec<DVec2>) {
        if self.loops.is_empty() {
            let us = NurbsSurface::samples(&self.surface.knots_u, self.surface.degree_u, segs_u.max(1));
            let vs = NurbsSurface::samples(&self.surface.knots_v, self.surface.degree_v, segs_v.max(1));
            let mut uv = Vec::with_capacity(us.len() * vs.len());
            for &u in &us {
                for &v in &vs {
                    uv.push(DVec2::new(u, v));
                }
            }
            return (self.surface.tessellate(segs_u, segs_v), uv);
        }
        let planar = self.surface.degree_u == 1 && self.surface.degree_v == 1 && self.surface.count_u == 2 && self.surface.count_v == 2;
        let (uv, tris): (Vec<DVec2>, Vec<[usize; 3]>) = if planar {
            // planar faces: earcut of the trim polygon is exact
            let mut coords: Vec<f64> = Vec::new();
            let mut holes: Vec<usize> = Vec::new();
            for (i, l) in self.loops.iter().enumerate() {
                if i > 0 {
                    holes.push(coords.len() / 2);
                }
                for p in self.subdivided_loop(l) {
                    coords.push(p.x);
                    coords.push(p.y);
                }
            }
            let Ok(indices) = earcutr::earcut(&coords, &holes, 2) else {
                return (Mesh::new(), Vec::new());
            };
            (coords.chunks(2).map(|c| DVec2::new(c[0], c[1])).collect(), indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect())
        } else {
            self.curved_trimmed_triangulation(segs_u, segs_v)
        };
        let mesh = self.mesh_from_uv(&uv, &tris);
        (mesh, uv)
    }

    /// Mesh with surface points and normals at the given parameters.
    pub(crate) fn mesh_from_uv(&self, uv: &[DVec2], tris: &[[usize; 3]]) -> Mesh {
        let mut mesh = Mesh::new();
        for p in uv {
            let (pt, su, sv) = self.surface.evaluate(p.x, p.y);
            let n = su.cross(sv);
            mesh.vertices.push(pt);
            mesh.normals.push(if n.length_squared() > 1e-20 { n.normalize() } else { self.surface.normal_at(p.x, p.y) });
        }
        for t in tris {
            mesh.indices.extend_from_slice(&[t[0] as u32, t[1] as u32, t[2] as u32]);
        }
        mesh
    }

    pub fn bounding_box(&self) -> BoundingBox {
        if self.loops.is_empty() {
            return self.surface.bounding_box();
        }
        let mut bb = BoundingBox::EMPTY;
        for l in &self.loops {
            for uv in l {
                bb.add_point(self.surface.point_at(uv.x, uv.y));
            }
        }
        bb
    }

    pub fn transform(&mut self, xf: &DAffine3) {
        self.surface.transform(xf);
    }

    pub fn flip(&mut self) {
        self.surface.flip();
        for l in self.loops.iter_mut() {
            for p in l.iter_mut() {
                *p = DVec2::new(p.y, p.x);
            }
            l.reverse();
        }
    }
}

fn dedup_consecutive(pts: &mut Vec<DVec2>) {
    pts.dedup_by(|a, b| (*a - *b).length() < 1e-12);
}

/// Subdivisions per knot span used when a face boundary is sampled for Booleans and caps:
/// straight spans need one segment, curved spans eight.
pub fn boundary_segs(degree: usize) -> usize {
    if degree <= 1 { 1 } else { 8 }
}

pub fn signed_area(poly: &[DVec2]) -> f64 {
    let mut a = 0.0;
    for i in 0..poly.len() {
        let p = poly[i];
        let q = poly[(i + 1) % poly.len()];
        a += p.x * q.y - q.x * p.y;
    }
    a * 0.5
}

pub fn polygon_centroid(poly: &[DVec2]) -> (DVec2, f64) {
    let mut a = 0.0;
    let mut c = DVec2::ZERO;
    for i in 0..poly.len() {
        let p = poly[i];
        let q = poly[(i + 1) % poly.len()];
        let cross = p.x * q.y - q.x * p.y;
        a += cross;
        c += (p + q) * cross;
    }
    if a.abs() < 1e-18 {
        let n = poly.len().max(1) as f64;
        return (poly.iter().fold(DVec2::ZERO, |s, p| s + *p) / n, 0.0);
    }
    (c / (3.0 * a), a * 0.5)
}

/// Trim-loop segments in a uniform grid of cells (for "is this point near the outline") and
/// in horizontal bands (for even-odd inside tests), so meshing a face whose loops have
/// thousands of points does not test every sample against every segment.
struct LoopIndex {
    /// (start, end, loop index)
    segs: Vec<(DVec2, DVec2, usize)>,
    loops: usize,
    min: DVec2,
    cell: f64,
    nx: usize,
    ny: usize,
    /// segments touching each cell (row-major)
    cells: Vec<Vec<u32>>,
    /// segments whose y-range overlaps each band of rows
    bands: Vec<Vec<u32>>,
}

impl LoopIndex {
    fn new(loops: &[Vec<DVec2>], cell: f64) -> Self {
        let mut segs = Vec::new();
        for (li, l) in loops.iter().enumerate() {
            for k in 0..l.len() {
                segs.push((l[k], l[(k + 1) % l.len()], li));
            }
        }
        let (mut min, mut max) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for (a, _, _) in &segs {
            min = min.min(*a);
            max = max.max(*a);
        }
        if segs.is_empty() {
            (min, max) = (DVec2::ZERO, DVec2::ONE);
        }
        // at most about a million cells
        let span = (max - min).max(DVec2::splat(1e-12));
        let cell = cell.max((span.x * span.y / 1e6).sqrt());
        let (nx, ny) = (((span.x / cell) as usize + 1).max(1), ((span.y / cell) as usize + 1).max(1));
        let mut cells = vec![Vec::new(); nx * ny];
        let mut bands = vec![Vec::new(); ny];
        let ix = |x: f64| (((x - min.x) / cell) as isize).clamp(0, nx as isize - 1) as usize;
        let iy = |y: f64| (((y - min.y) / cell) as isize).clamp(0, ny as isize - 1) as usize;
        for (si, (a, b, _)) in segs.iter().enumerate() {
            let (y0, y1) = (iy(a.y.min(b.y)), iy(a.y.max(b.y)));
            for band in &mut bands[y0..=y1] {
                band.push(si as u32);
            }
            // the cells along the segment (and their neighbours, so a sample that skips a
            // corner still finds it): stepping by a third of a cell
            let steps = (((*b - *a).length() / cell) * 3.0).ceil() as usize + 1;
            let mut last = usize::MAX;
            for k in 0..=steps {
                let p = *a + (*b - *a) * (k as f64 / steps as f64);
                let (cx, cy) = (ix(p.x), iy(p.y));
                for (dx, dy) in [(0isize, 0isize), (1, 0), (0, 1), (1, 1), (-1, 0), (0, -1), (-1, -1), (1, -1), (-1, 1)] {
                    let (x, y) = (cx as isize + dx, cy as isize + dy);
                    if x < 0 || y < 0 || x >= nx as isize || y >= ny as isize {
                        continue;
                    }
                    let c = y as usize * nx + x as usize;
                    if c != last && cells[c].last() != Some(&(si as u32)) {
                        cells[c].push(si as u32);
                    }
                }
                last = cy * nx + cx;
            }
        }
        Self { segs, loops: loops.len(), min, cell, nx, ny, cells, bands }
    }

    fn row(&self, y: f64) -> Option<usize> {
        let r = ((y - self.min.y) / self.cell).floor();
        (r >= 0.0 && (r as usize) < self.ny).then_some(r as usize)
    }

    /// Inside the first loop and outside every other (even-odd per loop).
    fn inside(&self, p: DVec2) -> bool {
        let Some(row) = self.row(p.y) else { return false };
        let mut parity = vec![false; self.loops];
        for &si in &self.bands[row] {
            let (a, b, li) = self.segs[si as usize];
            if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
                parity[li] = !parity[li];
            }
        }
        parity.first().copied().unwrap_or(false) && !parity.iter().skip(1).any(|x| *x)
    }

    /// Closer to some segment than 0.6 × min(segment length, `step`).
    fn near_outline(&self, p: DVec2, step: f64) -> bool {
        let reach = 0.6 * step;
        let (cx0, cx1) = (((p.x - reach - self.min.x) / self.cell).floor() as isize, ((p.x + reach - self.min.x) / self.cell).floor() as isize);
        let (cy0, cy1) = (((p.y - reach - self.min.y) / self.cell).floor() as isize, ((p.y + reach - self.min.y) / self.cell).floor() as isize);
        for y in cy0.max(0)..=cy1.min(self.ny as isize - 1) {
            for x in cx0.max(0)..=cx1.min(self.nx as isize - 1) {
                for &si in &self.cells[y as usize * self.nx + x as usize] {
                    let (a, b, _) = self.segs[si as usize];
                    let d = b - a;
                    let len2 = d.length_squared();
                    let t = if len2 < 1e-24 { 0.0 } else { ((p - a).dot(d) / len2).clamp(0.0, 1.0) };
                    if (a + d * t - p).length() < 0.6 * len2.sqrt().min(step).max(1e-9) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

pub fn point_in_polygon(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (pi, pj) = (poly[i], poly[j]);
        if (pi.y > p.y) != (pj.y > p.y) && p.x < (pj.x - pi.x) * (p.y - pi.y) / (pj.y - pi.y) + pi.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

fn seg_intersect_2d(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2) -> Option<(f64, f64)> {
    let d1 = a1 - a0;
    let d2 = b1 - b0;
    let den = d1.x * d2.y - d1.y * d2.x;
    if den.abs() < 1e-18 {
        return None;
    }
    let w = b0 - a0;
    let s = (w.x * d2.y - w.y * d2.x) / den;
    let t = (w.x * d1.y - w.y * d1.x) / den;
    if (-1e-9..=1.0 + 1e-9).contains(&s) && (-1e-9..=1.0 + 1e-9).contains(&t) {
        Some((s.clamp(0.0, 1.0), t.clamp(0.0, 1.0)))
    } else {
        None
    }
}

/// Split a closed loop (CCW) by a polyline path that enters and leaves it.
/// Returns the two loops, both counter-clockwise.
pub fn split_loop_by_path(poly: &[DVec2], path: &[DVec2]) -> Option<(Vec<DVec2>, Vec<DVec2>)> {
    let n = poly.len();
    // crossings of the path with the polygon: (path segment index, path param, poly edge index, poly param)
    let mut crossings: Vec<(usize, f64, usize, f64)> = Vec::new();
    for (pi, w) in path.windows(2).enumerate() {
        for e in 0..n {
            let (a, b) = (poly[e], poly[(e + 1) % n]);
            if let Some((s, t)) = seg_intersect_2d(w[0], w[1], a, b) {
                crossings.push((pi, s, e, t));
            }
        }
    }
    crossings.sort_by(|x, y| (x.0, x.1).partial_cmp(&(y.0, y.1)).unwrap());
    crossings.dedup_by(|x, y| x.0 == y.0 && (x.1 - y.1).abs() < 1e-9);
    if crossings.len() < 2 {
        return None;
    }
    // find consecutive crossings whose intermediate path lies inside
    let mut chosen: Option<(usize, usize)> = None;
    for k in 0..crossings.len() - 1 {
        let (p0, s0, _, _) = crossings[k];
        let (p1, s1, _, _) = crossings[k + 1];
        let pa = path[p0].lerp(path[p0 + 1], s0);
        let pb = path[p1].lerp(path[p1 + 1], s1);
        let mid = if p0 == p1 { (pa + pb) * 0.5 } else { path[p0 + 1] };
        if point_in_polygon(mid, poly) && (pa - pb).length() > 1e-12 {
            chosen = Some((k, k + 1));
            break;
        }
    }
    let (ka, kb) = chosen?;
    let (pa_seg, sa, ea, _) = crossings[ka];
    let (pb_seg, sb, eb, _) = crossings[kb];
    let pa = path[pa_seg].lerp(path[pa_seg + 1], sa);
    let pb = path[pb_seg].lerp(path[pb_seg + 1], sb);
    let mut chord = vec![pa];
    for i in pa_seg + 1..=pb_seg {
        chord.push(path[i]);
    }
    chord.push(pb);
    chord.dedup_by(|x, y| (*x - *y).length() < 1e-12);
    // loop 1: chord pa->pb, then polygon vertices from eb+1 around to ea
    let mut l1 = chord.clone();
    let mut i = (eb + 1) % n;
    loop {
        l1.push(poly[i]);
        if i == ea {
            break;
        }
        i = (i + 1) % n;
    }
    // loop 2: reversed chord pb->pa, then polygon vertices from ea+1 around to eb
    let mut l2: Vec<DVec2> = chord.iter().rev().copied().collect();
    let mut i = (ea + 1) % n;
    if ea != eb {
        loop {
            l2.push(poly[i]);
            if i == eb {
                break;
            }
            i = (i + 1) % n;
        }
    }
    for l in [&mut l1, &mut l2] {
        l.dedup_by(|x, y| (*x - *y).length() < 1e-12);
        if l.len() > 1 && (l[0] - l[l.len() - 1]).length() < 1e-12 {
            l.pop();
        }
        if signed_area(l) < 0.0 {
            l.reverse();
        }
    }
    if l1.len() < 3 || l2.len() < 3 {
        return None;
    }
    Some((l1, l2))
}

/// Best-fit plane of a closed (or any) curve using Newell's method; `None` when not planar.
pub fn curve_plane(c: &NurbsCurve) -> Option<Plane> {
    let pts = c.tessellate(6);
    let mut n = DVec3::ZERO;
    let mut centroid = DVec3::ZERO;
    for i in 0..pts.len() {
        let a = pts[i];
        let b = pts[(i + 1) % pts.len()];
        n += DVec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        centroid += a;
    }
    let centroid = centroid / pts.len() as f64;
    if n.length_squared() < 1e-18 {
        // open curve: try a plane from three well separated points
        let a = pts[0];
        let b = pts[pts.len() / 2];
        let cpt = pts[pts.len() - 1];
        let n2 = (b - a).cross(cpt - a);
        if n2.length_squared() < 1e-18 {
            return None;
        }
        n = n2;
    }
    let n = n.normalize();
    let plane = Plane::from_normal(centroid, n);
    let scale = 1.0 + c.bounding_box().diagonal();
    if pts.iter().all(|p| plane.signed_distance(*p).abs() < 1e-6 * scale) {
        Some(plane)
    } else {
        None
    }
}

/// A polysurface: a set of faces. `closed` is true when every boundary edge is shared
/// with another face (a solid in Triceratops' terms).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Brep {
    pub faces: Vec<Face>,
    #[serde(default)]
    pub closed: bool,
    /// The solidcore body this solid was made as, trusted only while it matches `faces`
    /// ([`Brep::exact_valid`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<ExactBody>,
}

impl Brep {
    pub fn new(faces: Vec<Face>) -> Self {
        let mut b = Self { faces, closed: false, exact: None };
        b.closed = b.compute_closed();
        b
    }

    /// Hash of the geometry of the faces ([`crate::exact::fingerprint`]).
    pub fn fingerprint(&self) -> u64 {
        crate::exact::fingerprint(&self.faces, self.closed)
    }

    /// The exact body, only if it still matches the faces it was made with.
    pub fn exact_valid(&self) -> Option<&ExactBody> {
        self.exact.as_ref().filter(|e| e.faces_fingerprint == self.fingerprint())
    }

    /// Attach `record` as the exact body of these faces as they are now.
    pub fn attach_exact(&mut self, record: std::sync::Arc<solidcore_record::brep::Brep>) {
        self.exact = Some(ExactBody { record, faces_fingerprint: self.fingerprint() });
    }

    pub fn from_surfaces(surfaces: Vec<NurbsSurface>) -> Self {
        Self::new(surfaces.into_iter().map(Face::untrimmed).collect())
    }

    pub fn from_surface(s: NurbsSurface) -> Self {
        Self::from_surfaces(vec![s])
    }

    /// Box with one corner at the plane origin extending along the plane axes.
    pub fn cuboid(plane: &Plane, dx: f64, dy: f64, dz: f64) -> Result<Self> {
        let p = |u: f64, v: f64, w: f64| plane.point_at_3(u, v, w);
        let face = |a: DVec3, b: DVec3, c: DVec3, d: DVec3| NurbsSurface::from_grid(1, 1, vec![vec![a, b], vec![d, c]]);
        let faces = vec![
            face(p(0.0, 0.0, 0.0), p(dx, 0.0, 0.0), p(dx, dy, 0.0), p(0.0, dy, 0.0))?,
            face(p(0.0, 0.0, dz), p(0.0, dy, dz), p(dx, dy, dz), p(dx, 0.0, dz))?,
            face(p(0.0, 0.0, 0.0), p(0.0, 0.0, dz), p(dx, 0.0, dz), p(dx, 0.0, 0.0))?,
            face(p(0.0, dy, 0.0), p(dx, dy, 0.0), p(dx, dy, dz), p(0.0, dy, dz))?,
            face(p(0.0, 0.0, 0.0), p(0.0, dy, 0.0), p(0.0, dy, dz), p(0.0, 0.0, dz))?,
            face(p(dx, 0.0, 0.0), p(dx, 0.0, dz), p(dx, dy, dz), p(dx, dy, 0.0))?,
        ];
        let mut brep = Self::from_surfaces(faces);
        brep.orient_outward();
        brep.closed = true;
        Ok(brep)
    }

    /// Flip faces whose normal points toward the brep centroid (closed convex shapes only).
    pub fn orient_outward(&mut self) {
        let c = self.bounding_box().center();
        for f in self.faces.iter_mut() {
            let (p, n) = f.sample();
            if n.dot(p - c) < 0.0 {
                f.flip();
            }
        }
    }

    /// Approximate closedness test: every boundary sample point of every face must lie
    /// on the boundary of some other face (or on another boundary loop of the same face).
    pub fn compute_closed(&self) -> bool {
        if self.faces.is_empty() {
            return false;
        }
        let diag = self.bounding_box().diagonal().max(1e-6);
        let tol = diag * 2e-3;
        let edges: Vec<Vec<Vec<DVec3>>> = self.faces.iter().map(|f| f.edge_polylines_raw(diag * 2e-4)).collect();
        for (fi, face_edges) in edges.iter().enumerate() {
            for (ei, edge) in face_edges.iter().enumerate() {
                let n = (edge.len() / 4).clamp(6, 48);
                let samples = sample_polyline_indexed(edge, n);
                for (s, seg) in samples {
                    let mut found = false;
                    'search: for (fj, other_edges) in edges.iter().enumerate() {
                        for (ej, oe) in other_edges.iter().enumerate() {
                            if fi == fj && ei == ej {
                                // a trim loop may run along a seam twice (closed surfaces):
                                // a far part of the same loop counts as the partner edge. A seam
                                // is run the other way on its second pass: a part of the loop
                                // going the same way is just this edge again (cut into segments
                                // shorter than the tolerance, it would otherwise find itself)
                                if distance_to_other_pass(s, oe, seg, 20.0 * tol) < tol {
                                    found = true;
                                    break 'search;
                                }
                                continue;
                            }
                            if distance_to_polyline(s, oe) < tol {
                                found = true;
                                break 'search;
                            }
                        }
                    }
                    if !found {
                        if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
                            let dself = distance_to_polyline_skipping(s, edge, seg, 1);
                            let (a, bq) = (edge[seg], edge[seg + 1]);
                            let twin = edge.windows(2).enumerate().filter(|(k, _)| (*k as i64 - seg as i64).abs() > 1).map(|(k, w)| (k, ((w[0] - bq).length() + (w[1] - a).length()).min((w[0] - a).length() + (w[1] - bq).length()))).min_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
                            eprintln!("compute_closed: face {fi} edge {ei} seg {seg}/{} sample ({:.3},{:.3},{:.3}) unmatched, tol {:.4}, self-skip distance {:.5}; segment ({:.3},{:.3},{:.3})->({:.3},{:.3},{:.3}) len {:.4}; nearest twin segment {:?}", edge.len(), s.x, s.y, s.z, tol, dself, a.x, a.y, a.z, bq.x, bq.y, bq.z, (bq - a).length(), twin);
                            // neighbours of the segment in the loop
                            for k in seg.saturating_sub(2)..(seg + 3).min(edge.len() - 1) {
                                let p = edge[k];
                                eprintln!("   loop[{k}] = ({:.4},{:.4},{:.4})", p.x, p.y, p.z);
                            }
                        }
                        return false;
                    }
                }
            }
        }
        true
    }

    pub fn tessellate(&self, segs_u: usize, segs_v: usize) -> Mesh {
        let mut m = Mesh::new();
        for f in &self.faces {
            m.append(&f.tessellate(segs_u, segs_v));
        }
        m
    }

    pub fn edges(&self) -> Vec<NurbsCurve> {
        let tol = self.bounding_box().diagonal().max(1e-3) * 1e-3;
        self.faces.iter().flat_map(|f| f.edge_curves(tol)).collect()
    }

    pub fn bounding_box(&self) -> BoundingBox {
        self.faces.iter().fold(BoundingBox::EMPTY, |b, f| b.union(&f.bounding_box()))
    }

    pub fn transform(&mut self, xf: &DAffine3) {
        self.exact = None;
        for f in self.faces.iter_mut() {
            f.transform(xf);
        }
    }

    pub fn flip(&mut self) {
        for f in self.faces.iter_mut() {
            f.flip();
        }
    }

    /// Naked (unshared) boundary loops that are planar and closed, as 3D polylines
    /// with their planes. Used by Cap.
    pub fn naked_planar_loops(&self) -> Vec<(Plane, Vec<DVec3>)> {
        let diag = self.bounding_box().diagonal().max(1e-6);
        let tol = diag * 2e-3;
        let edges: Vec<Vec<Vec<DVec3>>> = self.faces.iter().map(|f| f.edge_polylines_raw(diag * 2e-4)).collect();
        let mut out = Vec::new();
        for (fi, face_edges) in edges.iter().enumerate() {
            for (ei, edge) in face_edges.iter().enumerate() {
                if edge.len() < 3 || (edge[0] - edge[edge.len() - 1]).length() > tol {
                    continue;
                }
                let samples = sample_polyline(edge, 8);
                let shared = samples.iter().all(|s| {
                    edges.iter().enumerate().any(|(fj, oes)| oes.iter().enumerate().any(|(ej, oe)| !(fi == fj && ei == ej) && distance_to_polyline(*s, oe) < tol))
                });
                if shared {
                    continue;
                }
                if let Ok(c) = NurbsCurve::polyline(edge.clone()) {
                    if let Some(plane) = curve_plane(&c) {
                        out.push((plane, edge.clone()));
                    }
                }
            }
        }
        out
    }
}

fn sample_polyline(p: &[DVec3], n: usize) -> Vec<DVec3> {
    if p.len() < 2 {
        return p.to_vec();
    }
    let total: f64 = p.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let mut out = Vec::new();
    for i in 0..n {
        let target = total * (i as f64 + 0.5) / n as f64;
        let mut acc = 0.0;
        for w in p.windows(2) {
            let l = (w[1] - w[0]).length();
            if acc + l >= target {
                let t = if l > 0.0 { (target - acc) / l } else { 0.0 };
                out.push(w[0] + (w[1] - w[0]) * t);
                break;
            }
            acc += l;
        }
    }
    out
}

/// Samples with the index of the segment each one lies on.
fn sample_polyline_indexed(p: &[DVec3], n: usize) -> Vec<(DVec3, usize)> {
    if p.len() < 2 {
        return p.iter().map(|q| (*q, 0)).collect();
    }
    let total: f64 = p.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let mut out = Vec::new();
    for i in 0..n {
        let target = total * (i as f64 + 0.5) / n as f64;
        let mut acc = 0.0;
        for (k, w) in p.windows(2).enumerate() {
            let l = (w[1] - w[0]).length();
            if acc + l >= target {
                let t = if l > 0.0 { (target - acc) / l } else { 0.0 };
                out.push((w[0] + (w[1] - w[0]) * t, k));
                break;
            }
            acc += l;
        }
    }
    out
}

/// Distance to a polyline ignoring the segments within `margin` of `skip` (cyclically).
fn distance_to_polyline_skipping(p: DVec3, poly: &[DVec3], skip: usize, margin: usize) -> f64 {
    let nseg = poly.len().saturating_sub(1);
    if nseg < 2 * margin + 1 {
        return f64::INFINITY;
    }
    let mut best = f64::INFINITY;
    for (k, w) in poly.windows(2).enumerate() {
        let d = (k as i64 - skip as i64).rem_euclid(nseg as i64).min((skip as i64 - k as i64).rem_euclid(nseg as i64));
        if d as usize <= margin {
            continue;
        }
        let ab = w[1] - w[0];
        let l2 = ab.length_squared();
        let t = if l2 < 1e-18 { 0.0 } else { ((p - w[0]).dot(ab) / l2).clamp(0.0, 1.0) };
        best = best.min((w[0] + ab * t - p).length());
    }
    best
}

/// Distance from `p` (on segment `skip` of the closed polyline `poly`) to another pass of
/// the same loop: a part running the opposite way (a seam's second pass), or one running the
/// same way but at least `reach` away along the loop. Never this edge itself, however short
/// its segments.
fn distance_to_other_pass(p: DVec3, poly: &[DVec3], skip: usize, reach: f64) -> f64 {
    let nseg = poly.len().saturating_sub(1);
    if nseg < 3 {
        return f64::INFINITY;
    }
    let skip = skip.min(nseg - 1);
    let dir = (poly[skip + 1] - poly[skip]).normalize_or_zero();
    let mut cum = vec![0.0; nseg + 1];
    for k in 0..nseg {
        cum[k + 1] = cum[k] + (poly[k + 1] - poly[k]).length();
    }
    let total = cum[nseg];
    let here = (cum[skip] + cum[skip + 1]) / 2.0;
    let mut best = f64::INFINITY;
    for (k, w) in poly.windows(2).enumerate() {
        let d = (k as i64 - skip as i64).rem_euclid(nseg as i64).min((skip as i64 - k as i64).rem_euclid(nseg as i64));
        let ab = w[1] - w[0];
        let along = {
            let x = ((cum[k] + cum[k + 1]) / 2.0 - here).abs();
            x.min(total - x)
        };
        if d <= 1 || (ab.normalize_or_zero().dot(dir) > 0.0 && along < reach) {
            continue;
        }
        let l2 = ab.length_squared();
        let t = if l2 < 1e-18 { 0.0 } else { ((p - w[0]).dot(ab) / l2).clamp(0.0, 1.0) };
        best = best.min((w[0] + ab * t - p).length());
    }
    best
}

fn distance_to_polyline(p: DVec3, poly: &[DVec3]) -> f64 {
    let mut best = f64::INFINITY;
    for w in poly.windows(2) {
        let ab = w[1] - w[0];
        let l2 = ab.length_squared();
        let t = if l2 < 1e-18 { 0.0 } else { ((p - w[0]).dot(ab) / l2).clamp(0.0, 1.0) };
        best = best.min((w[0] + ab * t - p).length());
    }
    if poly.len() == 1 {
        best = (poly[0] - p).length();
    }
    best
}


pub fn sample_polyline_pub(e: &[DVec3], n: usize) -> Vec<DVec3> {
    sample_polyline(e, n)
}

pub fn distance_to_polyline_pub(p: DVec3, e: &[DVec3]) -> f64 {
    distance_to_polyline(p, e)
}

pub fn sample_polyline_indexed_pub(p: &[DVec3], n: usize) -> Vec<(DVec3, usize)> {
    sample_polyline_indexed(p, n)
}

pub fn distance_to_polyline_skipping_pub(p: DVec3, poly: &[DVec3], skip: usize, margin: usize) -> f64 {
    distance_to_polyline_skipping(p, poly, skip, margin)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_record() -> std::sync::Arc<solidcore_record::brep::Brep> {
        std::sync::Arc::new(solidcore_record::brep::Brep {
            version: 4,
            tolerance: 1e-6,
            regions: Vec::new(),
            shells: Vec::new(),
            faces: Vec::new(),
            edges: Vec::new(),
            vertices: Vec::new(),
        })
    }

    #[test]
    fn exact_valid_until_faces_change() {
        let mut b = Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        assert!(b.exact.is_none() && b.exact_valid().is_none());
        b.attach_exact(dummy_record());
        assert!(b.exact_valid().is_some());
        b.faces[0].surface.control_points[0].x += 1e-9;
        assert!(b.exact_valid().is_none(), "an in-place face edit must invalidate the exact body");
    }

    #[test]
    fn exact_valid_ignores_nothing_a_face_edit_can_touch() {
        let mut b = Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        b.attach_exact(dummy_record());
        let ok = b.clone();
        let mut w = b.clone();
        w.faces[1].surface.weights[0] *= 1.0000001;
        let mut k = b.clone();
        k.faces[2].surface.knots_u[0] -= 0.5;
        let mut l = b.clone();
        l.faces[3].loops.push(vec![DVec2::new(0.1, 0.1), DVec2::new(0.2, 0.1), DVec2::new(0.2, 0.2)]);
        let mut n = b.clone();
        n.faces.pop();
        assert!(ok.exact_valid().is_some());
        for (name, m) in [("weight", w), ("knot", k), ("loop", l), ("face count", n)] {
            assert!(m.exact_valid().is_none(), "{name} edit not detected");
        }
    }

    #[test]
    fn transform_drops_the_exact_body() {
        let mut b = Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        b.attach_exact(dummy_record());
        b.transform(&DAffine3::from_translation(DVec3::X));
        assert!(b.exact.is_none());
    }

    #[test]
    fn exact_survives_a_serde_roundtrip() {
        let mut b = Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        b.attach_exact(dummy_record());
        let json = serde_json::to_string(&b).unwrap();
        let back: Brep = serde_json::from_str(&json).unwrap();
        assert_eq!(back, b);
        assert!(back.exact_valid().is_some());
    }

    #[test]
    fn json_without_exact_still_loads() {
        let b = Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        let mut v = serde_json::to_value(&b).unwrap();
        v.as_object_mut().unwrap().remove("exact");
        let back: Brep = serde_json::from_value(v).unwrap();
        assert!(back.exact.is_none());
        assert_eq!(back.faces, b.faces);
    }

    #[test]
    fn fingerprint_is_stable_across_runs() {
        // pinned: the value is stored in documents, so it must not change with the toolchain
        let b = Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        assert_eq!(b.fingerprint(), 15843823923039428258_u64, "the fingerprint changed: stored documents would lose their exact bodies");
    }

    #[test]
    fn box_normals_outward_and_closed() {
        let b = Brep::cuboid(&Plane::world_xy(), 2.0, 3.0, 4.0).unwrap();
        let c = b.bounding_box().center();
        for f in &b.faces {
            let (p, n) = f.sample();
            assert!(n.dot(p - c) > 0.0);
        }
        assert_eq!(b.tessellate(1, 1).triangle_count(), 12);
        assert!(b.compute_closed());
    }

    /// A loft of many closed sections is closed around but open at both ends: it must not
    /// count as a closed solid (a short-segment edge used to find "itself" as its partner).
    #[test]
    fn long_loft_of_closed_sections_is_open() {
        let sections: Vec<NurbsCurve> = (0..=200)
            .map(|k| {
                let mut pts: Vec<DVec3> = (0..72)
                    .map(|i| {
                        let t = std::f64::consts::TAU * i as f64 / 72.0;
                        let r = 5.0 + 0.5 * (3.0 * t + k as f64 * 0.3).sin();
                        DVec3::new(r * t.cos(), r * t.sin(), k as f64 * 0.2)
                    })
                    .collect();
                pts.push(pts[0]);
                NurbsCurve::polyline(pts).unwrap()
            })
            .collect();
        let s = NurbsSurface::loft(&sections, 3).unwrap();
        assert!(!Brep::from_surface(s).closed, "both ends are open");
    }

    /// A long diagonal trim segment is filed only in the cells it crosses, not in every cell
    /// of its bounding box (which could take gigabytes).
    #[test]
    fn loop_index_files_segments_along_their_path() {
        let diamond = vec![DVec2::new(500.0, 0.0), DVec2::new(1000.0, 500.0), DVec2::new(500.0, 1000.0), DVec2::new(0.0, 500.0)];
        let index = LoopIndex::new(&[diamond], 1.0);
        let entries: usize = index.cells.iter().map(|c| c.len()).sum();
        assert!(entries < 20_000, "{entries} cell entries");
        assert!(index.inside(DVec2::new(500.0, 500.0)) && !index.inside(DVec2::new(10.0, 10.0)));
        assert!(index.near_outline(DVec2::new(750.0, 250.2), 1.0));
    }

    /// A curved face trimmed by loops of thousands of points (a threaded nut's bore) must
    /// mesh for display in well under a second, not minutes.
    #[test]
    fn finely_trimmed_face_meshes_quickly() {
        let grid: Vec<Vec<DVec3>> = (0..200).map(|i| (0..80).map(|j| DVec3::new(i as f64, j as f64, (i as f64 * 0.3).sin() * (j as f64 * 0.2).cos() * 3.0)).collect()).collect();
        let s = NurbsSurface::interpolate_grid(&grid, 3, 3).unwrap();
        let ((u0, u1), (v0, v1)) = (s.domain_u(), s.domain_v());
        let c = DVec2::new((u0 + u1) / 2.0, (v0 + v1) / 2.0);
        let (ru, rv) = ((u1 - u0) * 0.45, (v1 - v0) * 0.45);
        let wavy = |n: usize, f: f64, k: f64| -> Vec<DVec2> {
            (0..n).map(|i| {
                let t = std::f64::consts::TAU * i as f64 / n as f64;
                let r = f * (1.0 + 0.05 * (k * t).sin());
                c + DVec2::new(ru * r * t.cos(), rv * r * t.sin())
            }).collect()
        };
        let mut hole = wavy(3000, 0.4, 13.0);
        hole.reverse();
        let face = Face { surface: s, loops: vec![wavy(8000, 1.0, 40.0), hole] };
        let d = crate::display_mesh::MeshDensity::for_size(200.0);
        let t = std::time::Instant::now();
        let m = face.display_mesh(&d);
        let took = t.elapsed();
        assert!(m.triangle_count() > 1000, "{}", m.triangle_count());
        assert!(took < std::time::Duration::from_secs(2), "took {took:?}");
    }

    #[test]
    fn planar_face_with_hole_tessellates() {
        let outer = NurbsCurve::circle(&Plane::world_xy(), 5.0).unwrap();
        let hole = NurbsCurve::circle(&Plane::world_xy(), 2.0).unwrap();
        let f = Face::from_planar_curves(&[outer, hole], 1e-3).unwrap();
        assert!(f.is_trimmed());
        let m = f.tessellate(4, 4);
        assert!(m.triangle_count() > 20);
        // area ≈ pi (25 - 4)
        let mut area = 0.0;
        for t in 0..m.triangle_count() {
            let (a, b, c) = m.triangle(t);
            area += (b - a).cross(c - a).length() * 0.5;
        }
        assert!((area - std::f64::consts::PI * 21.0).abs() < 0.5, "area {area}");
    }

    #[test]
    fn split_square_by_line() {
        let sq = vec![DVec2::new(0.0, 0.0), DVec2::new(4.0, 0.0), DVec2::new(4.0, 4.0), DVec2::new(0.0, 4.0)];
        let path = vec![DVec2::new(1.0, -1.0), DVec2::new(1.0, 5.0)];
        let (a, b) = split_loop_by_path(&sq, &path).unwrap();
        let areas = [signed_area(&a), signed_area(&b)];
        assert!((areas[0] + areas[1] - 16.0).abs() < 1e-9);
        assert!(areas.iter().any(|x| (x - 4.0).abs() < 1e-9));
        let f = Face::planar(&Plane::world_xy(), vec![sq.iter().map(|p| DVec3::new(p.x, p.y, 0.0)).collect()]).unwrap();
        let (fa, fb) = f.split_by_uv_path(&path).unwrap();
        assert!(fa.contains_uv(DVec2::new(0.5, 2.0)) != fb.contains_uv(DVec2::new(0.5, 2.0)));
    }

    #[test]
    fn extrusion_with_caps_is_closed() {
        let circle = NurbsCurve::circle(&Plane::world_xy(), 1.0).unwrap();
        let side = NurbsSurface::extrude(&circle, DVec3::Z * 2.0).unwrap();
        let bottom = Face::from_planar_curves(std::slice::from_ref(&circle), 1e-3).unwrap();
        let mut top_c = circle.clone();
        top_c.transform(&DAffine3::from_translation(DVec3::Z * 2.0));
        let top = Face::from_planar_curves(&[top_c], 1e-3).unwrap();
        let b = Brep::new(vec![Face::untrimmed(side.clone()), bottom, top]);
        assert!(b.closed);
        let open = Brep::new(vec![Face::untrimmed(side)]);
        assert!(!open.closed);
        assert_eq!(open.naked_planar_loops().len(), 2);
    }
}
