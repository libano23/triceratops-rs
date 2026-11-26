//! Surface operations shared by Trim, Split, WireCut and curve projection:
//!
//! * `Projector` – exact projection of points and curves onto a NURBS surface along a
//!   direction (line/surface intersection seeded by a tessellation and refined by a
//!   three-unknown Newton iteration), with seam handling on closed surfaces;
//! * `split_region` – general partition of a trimmed (u, v) region by any number of paths
//!   (planar-graph face extraction), used by `Face::split_by_paths`.

use std::collections::HashMap;

use glam::{DMat3, DVec2, DVec3};

use crate::brep::{point_in_polygon, signed_area, Face};
use crate::{NurbsCurve, NurbsSurface};

// ---------------------------------------------------------------- projection

/// One intersection of a line with a surface.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub u: f64,
    pub v: f64,
    /// Line parameter (`p + t * dir`).
    pub t: f64,
    pub point: DVec3,
}

/// Projects points and curves onto a surface along a fixed direction.
pub struct Projector<'a> {
    surface: &'a NurbsSurface,
    dir: DVec3,
    tris: Vec<([DVec3; 3], [DVec2; 3])>,
    bbox: (DVec3, DVec3),
    pub tol: f64,
}

impl<'a> Projector<'a> {
    pub fn new(surface: &'a NurbsSurface, dir: DVec3, tol: f64) -> Self {
        let dir = dir.normalize_or(DVec3::Z);
        let segs_u = if surface.degree_u == 1 { 1 } else { 6 };
        let segs_v = if surface.degree_v == 1 { 1 } else { 6 };
        let us = NurbsSurface::samples(&surface.knots_u, surface.degree_u, segs_u);
        let vs = NurbsSurface::samples(&surface.knots_v, surface.degree_v, segs_v);
        let pts: Vec<Vec<(DVec3, DVec2)>> = us.iter().map(|&u| vs.iter().map(|&v| (surface.point_at(u, v), DVec2::new(u, v))).collect()).collect();
        let mut tris = Vec::new();
        for i in 0..us.len() - 1 {
            for j in 0..vs.len() - 1 {
                let (a, b, c, d) = (pts[i][j], pts[i + 1][j], pts[i + 1][j + 1], pts[i][j + 1]);
                tris.push(([a.0, b.0, c.0], [a.1, b.1, c.1]));
                tris.push(([a.0, c.0, d.0], [a.1, c.1, d.1]));
            }
        }
        let diag = surface.bounding_box().diagonal().max(1e-9);
        let bb = surface.bounding_box();
        let pad = DVec3::splat(diag * 1e-3 + tol);
        Self { surface, dir, tris, bbox: (bb.min - pad, bb.max + pad), tol: tol.max(1e-12) }
    }

    /// Whether the infinite line through `p` along the direction meets the surface's box.
    pub fn line_hits_bbox(&self, p: DVec3) -> bool {
        let (lo, hi) = self.bbox;
        let (mut tmin, mut tmax) = (f64::NEG_INFINITY, f64::INFINITY);
        for k in 0..3 {
            let (o, d, l, h) = (p[k], self.dir[k], lo[k], hi[k]);
            if d.abs() < 1e-15 {
                if o < l || o > h {
                    return false;
                }
                continue;
            }
            let (mut a, mut b) = ((l - o) / d, (h - o) / d);
            if a > b {
                std::mem::swap(&mut a, &mut b);
            }
            tmin = tmin.max(a);
            tmax = tmax.min(b);
            if tmin > tmax {
                return false;
            }
        }
        true
    }

    pub fn surface(&self) -> &NurbsSurface {
        self.surface
    }

    /// Every intersection of the infinite line through `p` along the direction with the surface.
    pub fn hits(&self, p: DVec3) -> Vec<Hit> {
        if !self.line_hits_bbox(p) {
            return Vec::new();
        }
        let mut seeds: Vec<(DVec2, f64)> = Vec::new();
        for (tri, uv) in &self.tris {
            if let Some((t, b)) = line_triangle(p, self.dir, tri) {
                seeds.push((uv[0] * b.x + uv[1] * b.y + uv[2] * b.z, t));
            }
        }
        let mut out: Vec<Hit> = Vec::new();
        for (uv, t) in seeds {
            if let Some(h) = self.refine(p, uv.x, uv.y, t) {
                if !out.iter().any(|o| (o.point - h.point).length() < self.tol * 2.0) {
                    out.push(h);
                }
            }
        }
        // a line that grazes between triangles can miss every seed: fall back on the closest point
        if out.is_empty() {
            let (u, v) = self.surface.closest_uv(p);
            let q = self.surface.point_at(u, v);
            let t = (q - p).dot(self.dir);
            if let Some(h) = self.refine(p, u, v, t) {
                out.push(h);
            }
        }
        out.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap());
        out
    }

    /// The hit closest to a reference point on the surface (or along the line).
    pub fn nearest_hit(&self, p: DVec3, reference: DVec3) -> Option<Hit> {
        self.hits(p).into_iter().min_by(|a, b| (a.point - reference).length().partial_cmp(&(b.point - reference).length()).unwrap())
    }

    /// Newton iteration on S(u, v) - p - t·dir = 0.
    fn refine(&self, p: DVec3, u0: f64, v0: f64, t0: f64) -> Option<Hit> {
        let (ua, ub) = self.surface.domain_u();
        let (va, vb) = self.surface.domain_v();
        let (mut u, mut v, mut t) = (u0.clamp(ua, ub), v0.clamp(va, vb), t0);
        let mut best: Option<(f64, f64, f64, f64, DVec3)> = None;
        for _ in 0..40 {
            let (s, su, sv) = self.surface.evaluate(u, v);
            let f = s - p - self.dir * t;
            let r = f.length();
            if best.is_none_or(|b| r < b.0) {
                best = Some((r, u, v, t, s));
            }
            if r < self.tol * 1e-3 {
                break;
            }
            let j = DMat3::from_cols(su, sv, -self.dir);
            if j.determinant().abs() < 1e-18 {
                break;
            }
            let d = j.inverse() * f;
            let nu = (u - d.x).clamp(ua, ub);
            let nv = (v - d.y).clamp(va, vb);
            t -= d.z;
            if (nu - u).abs() + (nv - v).abs() + d.z.abs() < 1e-15 {
                break;
            }
            u = nu;
            v = nv;
        }
        let (r, u, v, t, s) = best?;
        if r > self.tol {
            return None;
        }
        Some(Hit { u, v, t, point: s })
    }

    /// Project a curve: polylines in (u, v) that follow the exact projection within the
    /// tolerance. Several branches come back when the line hits the surface more than once
    /// or when the projection crosses a seam of a closed surface.
    pub fn project_curve(&self, curve: &NurbsCurve) -> Vec<Vec<DVec2>> {
        let (t0, t1) = curve.domain();
        // sample only the parameter ranges where the line can reach the surface (a long
        // cutter crossing a small surface would otherwise get a handful of samples there)
        let coarse = 256.max(curve.control_points.len() * 4);
        let flags: Vec<bool> = (0..=coarse).map(|i| self.line_hits_bbox(curve.point_at(t0 + (t1 - t0) * i as f64 / coarse as f64))).collect();
        let mut params: Vec<f64> = Vec::new();
        let mut i = 0;
        while i <= coarse {
            if !flags[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i <= coarse && flags[i] {
                i += 1;
            }
            let (a, b) = (start.saturating_sub(1), (i).min(coarse));
            let (ta, tb) = (t0 + (t1 - t0) * a as f64 / coarse as f64, t0 + (t1 - t0) * b as f64 / coarse as f64);
            let n = 32.max(((b - a) as f64 / coarse as f64 * curve.control_points.len() as f64 * 8.0).ceil() as usize);
            for k in 0..=n {
                params.push(ta + (tb - ta) * k as f64 / n as f64);
            }
        }
        params.sort_by(|a, b| a.partial_cmp(b).unwrap());
        params.dedup_by(|a, b| (*a - *b).abs() < 1e-15);
        if params.is_empty() {
            return Vec::new();
        }
        let samples: Vec<(f64, Vec<Hit>)> = params.iter().map(|&t| (t, self.hits(curve.point_at(t)))).collect();
        // chain hits into branches
        let (u0, u1) = self.surface.domain_u();
        let (v0, v1) = self.surface.domain_v();
        let span = DVec2::new(u1 - u0, v1 - v0);
        let near = |a: &Hit, b: &Hit| -> bool {
            let d = DVec2::new((a.u - b.u).abs() / span.x.max(1e-12), (a.v - b.v).abs() / span.y.max(1e-12));
            d.x < 0.2 && d.y < 0.2 && (a.point - b.point).length() < self.surface.bounding_box().diagonal().max(1e-9) * 0.25
        };
        struct Branch {
            pts: Vec<(f64, Hit)>,
            open: bool,
        }
        let mut branches: Vec<Branch> = Vec::new();
        for (t, hits) in &samples {
            let mut used = vec![false; hits.len()];
            // extend open branches with their nearest unused hit
            for b in branches.iter_mut().filter(|b| b.open) {
                let last = b.pts.last().unwrap().1;
                let best = hits.iter().enumerate().filter(|(i, h)| !used[*i] && near(&last, h)).min_by(|a, b| {
                    let da = (a.1.u - last.u).abs() / span.x + (a.1.v - last.v).abs() / span.y;
                    let db = (b.1.u - last.u).abs() / span.x + (b.1.v - last.v).abs() / span.y;
                    da.partial_cmp(&db).unwrap()
                });
                match best {
                    Some((i, h)) => {
                        used[i] = true;
                        b.pts.push((*t, *h));
                    }
                    None => b.open = false,
                }
            }
            for (i, h) in hits.iter().enumerate() {
                if !used[i] {
                    branches.push(Branch { pts: vec![(*t, *h)], open: true });
                }
            }
        }
        if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
            eprintln!("project_curve: {} samples ({} with hits), {} branches of lengths {:?}", samples.len(), samples.iter().filter(|s| !s.1.is_empty()).count(), branches.len(), branches.iter().map(|b| b.pts.len()).collect::<Vec<_>>());
        }
        // refine every branch so the chords follow the projected curve within the tolerance,
        // then extend its ends to the domain border when the curve continues past them
        let mut out: Vec<Vec<DVec2>> = Vec::new();
        for b in &branches {
            let mut refined: Vec<(f64, Hit)> = vec![b.pts[0]];
            for w in b.pts.windows(2) {
                self.subdivide(curve, w[0], w[1], 0, &mut refined);
            }
            let mut uv: Vec<DVec2> = refined.iter().map(|(_, h)| DVec2::new(h.u, h.v)).collect();
            let first = refined[0];
            let last = *refined.last().unwrap();
            if first.0 > t0 + 1e-12 {
                if let Some(p) = self.border_point(curve, first.0, first.1, -1.0) {
                    uv.insert(0, p);
                }
            }
            if last.0 < t1 - 1e-12 {
                if let Some(p) = self.border_point(curve, last.0, last.1, 1.0) {
                    uv.push(p);
                }
            }
            uv.dedup_by(|a, b| (*a - *b).length() < 1e-12);
            if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
                eprintln!("  branch refined to {} points: first {:?} last {:?} (curve params {:.4}..{:.4} of {:.4}..{:.4})", uv.len(), uv.first(), uv.last(), first.0, last.0, t0, t1);
            }
            if uv.len() >= 2 {
                out.push(uv);
            }
        }
        // join branches whose ends meet (tangential exits split one projection in two)
        let join_tol = DVec2::new(span.x, span.y) * 0.01;
        let close = |a: DVec2, b: DVec2| (a.x - b.x).abs() <= join_tol.x && (a.y - b.y).abs() <= join_tol.y;
        let mut joined = true;
        while joined && out.len() > 1 {
            joined = false;
            'outer: for i in 0..out.len() {
                for j in 0..out.len() {
                    if i == j {
                        continue;
                    }
                    let (ai, bi) = (out[i][0], *out[i].last().unwrap());
                    let (aj, bj) = (out[j][0], *out[j].last().unwrap());
                    let mut other = None;
                    if close(bi, aj) {
                        other = Some(out[j].clone());
                    } else if close(bi, bj) {
                        let mut r = out[j].clone();
                        r.reverse();
                        other = Some(r);
                    } else if close(ai, aj) {
                        out[i].reverse();
                        other = Some(out[j].clone());
                    } else if close(ai, bj) {
                        out[i].reverse();
                        let mut r = out[j].clone();
                        r.reverse();
                        other = Some(r);
                    }
                    if let Some(o) = other {
                        let mut merged = out[i].clone();
                        merged.extend(o.into_iter().skip(1));
                        let (lo, hi) = (i.min(j), i.max(j));
                        out.remove(hi);
                        out.remove(lo);
                        out.push(merged);
                        joined = true;
                        break 'outer;
                    }
                }
            }
        }
        out
    }

    fn subdivide(&self, curve: &NurbsCurve, a: (f64, Hit), b: (f64, Hit), depth: usize, out: &mut Vec<(f64, Hit)>) {
        let tm = (a.0 + b.0) * 0.5;
        let chord_mid = (a.1.point + b.1.point) * 0.5;
        let hit = self.hits(curve.point_at(tm)).into_iter().filter(|h| (h.point - chord_mid).length() < (a.1.point - b.1.point).length().max(self.tol * 4.0)).min_by(|x, y| (x.point - chord_mid).length().partial_cmp(&(y.point - chord_mid).length()).unwrap());
        match hit {
            Some(h) if depth < 12 && (h.point - chord_mid).length() > self.tol => {
                self.subdivide(curve, a, (tm, h), depth + 1, out);
                self.subdivide(curve, (tm, h), b, depth + 1, out);
            }
            _ => out.push(b),
        }
    }

    /// Where the projection leaves the surface past a branch end: the last hit found by
    /// bisection on the curve parameter, snapped to the domain border when it is close to one
    /// (seams of closed surfaces and open edges); tangential exits are returned as they are.
    fn border_point(&self, curve: &NurbsCurve, t_in: f64, h_in: Hit, sign: f64) -> Option<DVec2> {
        let (t0, t1) = curve.domain();
        let step = (t1 - t0) / 32.0;
        let mut inside = t_in;
        let mut outside = (t_in + sign * step).clamp(t0, t1);
        let mut last = h_in;
        let (u0, u1) = self.surface.domain_u();
        let (v0, v1) = self.surface.domain_v();
        let span = DVec2::new(u1 - u0, v1 - v0);
        let follows = |prev: &Hit, h: &Hit| (h.u - prev.u).abs() < span.x * 0.2 && (h.v - prev.v).abs() < span.y * 0.2;
        for _ in 0..48 {
            let tm = (inside + outside) * 0.5;
            let hits = self.hits(curve.point_at(tm));
            match hits.iter().filter(|h| follows(&last, h)).min_by(|a, b| ((a.u - last.u).abs() + (a.v - last.v).abs()).partial_cmp(&((b.u - last.u).abs() + (b.v - last.v).abs())).unwrap()) {
                Some(h) => {
                    last = *h;
                    inside = tm;
                }
                None => outside = tm,
            }
        }
        let mut p = DVec2::new(last.u, last.v);
        let du = [(p.x - u0).abs(), (u1 - p.x).abs()];
        let dv = [(p.y - v0).abs(), (v1 - p.y).abs()];
        let m = du[0].min(du[1]).min(dv[0]).min(dv[1]);
        let near_u = (du[0] == m || du[1] == m) && m < span.x * 0.02;
        let near_v = (dv[0] == m || dv[1] == m) && m < span.y * 0.02;
        if near_u {
            p.x = if du[0] == m { u0 } else { u1 };
        } else if near_v {
            p.y = if dv[0] == m { v0 } else { v1 };
        }
        Some(p)
    }
}

/// Infinite line / triangle intersection: returns the line parameter and barycentric weights.
fn line_triangle(p: DVec3, dir: DVec3, tri: &[DVec3; 3]) -> Option<(f64, DVec3)> {
    let e1 = tri[1] - tri[0];
    let e2 = tri[2] - tri[0];
    let pv = dir.cross(e2);
    let det = e1.dot(pv);
    if det.abs() < 1e-18 {
        return None;
    }
    let inv = 1.0 / det;
    let tv = p - tri[0];
    let u = tv.dot(pv) * inv;
    let qv = tv.cross(e1);
    let v = dir.dot(qv) * inv;
    let eps = -1e-6;
    if u < eps || v < eps || u + v > 1.0 - eps {
        return None;
    }
    let t = e2.dot(qv) * inv;
    Some((t, DVec3::new(1.0 - u - v, u, v)))
}

// ---------------------------------------------------------------- region splitting

/// Partition a region (outer loop + holes) by paths. Each result is `[outer, holes...]` with
/// the outer loop counter-clockwise and the holes clockwise. Paths that do not cross the
/// region, or only touch it, leave it unchanged (one piece comes back).
pub fn split_region(loops: &[Vec<DVec2>], paths: &[Vec<DVec2>]) -> Vec<Vec<Vec<DVec2>>> {
    if loops.is_empty() || loops[0].len() < 3 {
        return Vec::new();
    }
    // scale-aware epsilon
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for p in loops.iter().flatten() {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let extent = (hi - lo).max_element().max(1e-12);
    let eps = extent * 1e-9;
    // segments: (a, b, is_path)
    let mut segs: Vec<(DVec2, DVec2, bool)> = Vec::new();
    for l in loops {
        let n = l.len();
        for i in 0..n {
            let (a, b) = (l[i], l[(i + 1) % n]);
            if (a - b).length() > eps {
                segs.push((a, b, false));
            }
        }
    }
    for p in paths {
        for w in p.windows(2) {
            if (w[0] - w[1]).length() > eps {
                segs.push((w[0], w[1], true));
            }
        }
    }
    // split parameters per segment
    let mut cuts: Vec<Vec<f64>> = vec![Vec::new(); segs.len()];
    for i in 0..segs.len() {
        for j in i + 1..segs.len() {
            if !segs[i].2 && !segs[j].2 {
                // loop edges only meet at their shared vertices (already nodes)
                continue;
            }
            let (a0, a1, _) = segs[i];
            let (b0, b1, _) = segs[j];
            let bb_ok = a0.x.min(a1.x) <= b0.x.max(b1.x) + eps && b0.x.min(b1.x) <= a0.x.max(a1.x) + eps && a0.y.min(a1.y) <= b0.y.max(b1.y) + eps && b0.y.min(b1.y) <= a0.y.max(a1.y) + eps;
            if !bb_ok {
                continue;
            }
            match seg_seg(a0, a1, b0, b1, eps) {
                SegRel::Cross(s, t) => {
                    cuts[i].push(s);
                    cuts[j].push(t);
                }
                SegRel::Collinear => {
                    for (p, idx) in [(b0, i), (b1, i), (a0, j), (a1, j)] {
                        let (c0, c1, _) = segs[idx];
                        if let Some(s) = param_on_segment(p, c0, c1, eps) {
                            cuts[idx].push(s);
                        }
                    }
                }
                SegRel::None => {}
            }
        }
    }
    // nodes with tolerance merging
    let mut nodes: Vec<DVec2> = Vec::new();
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let cell = eps * 4.0;
    let node_of = |p: DVec2, nodes: &mut Vec<DVec2>, grid: &mut HashMap<(i64, i64), Vec<usize>>| -> usize {
        let (cx, cy) = ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(list) = grid.get(&(cx + dx, cy + dy)) {
                    for &i in list {
                        if (nodes[i] - p).length() <= eps * 2.0 {
                            return i;
                        }
                    }
                }
            }
        }
        nodes.push(p);
        grid.entry((cx, cy)).or_default().push(nodes.len() - 1);
        nodes.len() - 1
    };
    let mut edges: Vec<(usize, usize, bool)> = Vec::new();
    for (i, (a, b, is_path)) in segs.iter().enumerate() {
        let mut ts = cuts[i].clone();
        ts.push(0.0);
        ts.push(1.0);
        ts.sort_by(|x, y| x.partial_cmp(y).unwrap());
        ts.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
        for w in ts.windows(2) {
            let p = a.lerp(*b, w[0]);
            let q = a.lerp(*b, w[1]);
            let (na, nb) = (node_of(p, &mut nodes, &mut grid), node_of(q, &mut nodes, &mut grid));
            if na != nb {
                edges.push((na.min(nb), na.max(nb), *is_path));
            }
        }
    }
    // path pieces outside the region are dropped; duplicates merged (loop edges win)
    edges.sort_by_key(|x| (x.0, x.1, x.2));
    edges.dedup_by(|x, y| x.0 == y.0 && x.1 == y.1);
    let inside_region = |p: DVec2| point_in_polygon(p, &loops[0]) && !loops[1..].iter().any(|h| point_in_polygon(p, h));
    edges.retain(|(a, b, is_path)| !*is_path || inside_region((nodes[*a] + nodes[*b]) * 0.5));
    // prune dangling edges
    loop {
        let mut degree = vec![0usize; nodes.len()];
        for (a, b, _) in &edges {
            degree[*a] += 1;
            degree[*b] += 1;
        }
        let before = edges.len();
        edges.retain(|(a, b, _)| degree[*a] > 1 && degree[*b] > 1);
        if edges.len() == before {
            break;
        }
    }
    let dbg = std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok();
    if dbg {
        eprintln!("split_region: {} segs, {} nodes, {} edges after pruning, eps {eps:e}", segs.len(), nodes.len(), edges.len());
    }
    if edges.is_empty() {
        return vec![loops.to_vec()];
    }
    // half-edge walk
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nodes.len()];
    for (a, b, _) in &edges {
        adj[*a].push(*b);
        adj[*b].push(*a);
    }
    for (i, list) in adj.iter_mut().enumerate() {
        let o = nodes[i];
        list.sort_by(|&x, &y| {
            let ax = (nodes[x] - o).y.atan2((nodes[x] - o).x);
            let ay = (nodes[y] - o).y.atan2((nodes[y] - o).x);
            ax.partial_cmp(&ay).unwrap()
        });
        list.dedup();
    }
    let mut used: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut cycles: Vec<Vec<DVec2>> = Vec::new();
    for (a, b, _) in &edges {
        for (s, e) in [(*a, *b), (*b, *a)] {
            if used.contains(&(s, e)) {
                continue;
            }
            let mut cycle: Vec<usize> = vec![s];
            let (mut prev, mut cur) = (s, e);
            let mut guard = 0;
            loop {
                used.insert((prev, cur));
                cycle.push(cur);
                // next: first clockwise from the reversed incoming direction
                let back = nodes[prev] - nodes[cur];
                let a_in = back.y.atan2(back.x);
                let list = &adj[cur];
                let next = list.iter().copied().filter(|&n| n != prev || list.len() == 1).min_by(|&x, &y| {
                    let cw = |n: usize| {
                        let d = nodes[n] - nodes[cur];
                        let a = d.y.atan2(d.x);
                        let mut r = a_in - a;
                        while r <= 1e-12 {
                            r += std::f64::consts::TAU;
                        }
                        r
                    };
                    cw(x).partial_cmp(&cw(y)).unwrap()
                });
                let Some(next) = next else { break };
                prev = cur;
                cur = next;
                guard += 1;
                if cur == s && prev != s {
                    // closed when we are back at the start and about to reuse the first half-edge
                    if used.contains(&(prev, cur)) {
                        break;
                    }
                    used.insert((prev, cur));
                    break;
                }
                if guard > edges.len() * 2 + 2 {
                    break;
                }
            }
            if cycle.len() >= 3 {
                let mut pts: Vec<DVec2> = cycle.iter().map(|&i| nodes[i]).collect();
                if pts.len() > 1 && (pts[0] - pts[pts.len() - 1]).length() <= eps * 2.0 {
                    pts.pop();
                }
                if pts.len() >= 3 {
                    cycles.push(pts);
                }
            }
        }
    }
    let mut positive: Vec<(f64, Vec<DVec2>)> = Vec::new();
    let mut negative: Vec<(f64, Vec<DVec2>)> = Vec::new();
    for c in cycles {
        let a = signed_area(&c);
        if a > eps * eps {
            // a bounded cycle outside the region (the inside of a hole, say) is not a piece
            let n = c.len();
            let probe = (0..n).map(|i| {
                let (p, q) = (c[i], c[(i + 1) % n]);
                let d = q - p;
                let inward = DVec2::new(-d.y, d.x).normalize_or_zero();
                (p + q) * 0.5 + inward * (eps * 1e3).min(d.length() * 0.01)
            }).find(|m| !on_boundary(*m, &c, eps));
            if probe.is_none_or(inside_region) {
                positive.push((a, c));
            }
        } else if a < -eps * eps {
            negative.push((a, c));
        }
    }
    if dbg {
        eprintln!("split_region: {} positive cycles, {} negative", positive.len(), negative.len());
    }
    if positive.is_empty() {
        return vec![loops.to_vec()];
    }
    // the unbounded face is the most negative cycle; the others are holes
    negative.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
    let holes: Vec<Vec<DVec2>> = negative.into_iter().skip(1).map(|(_, c)| c).collect();
    positive.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
    let mut pieces: Vec<Vec<Vec<DVec2>>> = positive.iter().map(|(_, c)| vec![c.clone()]).collect();
    for h in holes {
        // a hole belongs to the smallest piece strictly larger than it that contains a point
        // of its interior (an island cut out by a closed path has the same outline as the hole)
        let hole_area = signed_area(&h).abs();
        let probe = interior_point(&h).unwrap_or(h[0]);
        if let Some(idx) = (0..pieces.len()).find(|&i| signed_area(&pieces[i][0]) > hole_area * (1.0 + 1e-9) + eps * eps && point_in_polygon(probe, &pieces[i][0])) {
            pieces[idx].push(h);
        }
    }
    pieces
}

/// A point inside a simple polygon: the grid sample farthest from its edges.
pub fn interior_point(poly: &[DVec2]) -> Option<DVec2> {
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for p in poly {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let dist = |p: DVec2| -> f64 {
        let n = poly.len();
        (0..n).map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % n]);
            let e = b - a;
            let t = ((p - a).dot(e) / e.length_squared().max(1e-300)).clamp(0.0, 1.0);
            (a + e * t - p).length()
        }).fold(f64::INFINITY, f64::min)
    };
    let mut best: Option<(f64, DVec2)> = None;
    for n in [9usize, 17, 33] {
        for i in 0..n {
            for j in 0..n {
                let p = DVec2::new(lo.x + (hi.x - lo.x) * (i as f64 + 0.5) / n as f64, lo.y + (hi.y - lo.y) * (j as f64 + 0.5) / n as f64);
                if point_in_polygon(p, poly) {
                    let d = dist(p);
                    if best.is_none_or(|b| d > b.0) {
                        best = Some((d, p));
                    }
                }
            }
        }
        if best.is_some() && n >= 17 {
            break;
        }
    }
    best.map(|b| b.1)
}

fn on_boundary(p: DVec2, poly: &[DVec2], eps: f64) -> bool {
    let n = poly.len();
    (0..n).any(|i| param_on_segment(p, poly[i], poly[(i + 1) % n], eps).is_some())
}

enum SegRel {
    Cross(f64, f64),
    Collinear,
    None,
}

fn seg_seg(a0: DVec2, a1: DVec2, b0: DVec2, b1: DVec2, eps: f64) -> SegRel {
    let d1 = a1 - a0;
    let d2 = b1 - b0;
    let den = d1.x * d2.y - d1.y * d2.x;
    let w = b0 - a0;
    if den.abs() <= 1e-14 * d1.length() * d2.length() {
        // parallel: collinear when b0 lies on line a
        let dist = (w.x * d1.y - w.y * d1.x).abs() / d1.length().max(1e-300);
        return if dist <= eps { SegRel::Collinear } else { SegRel::None };
    }
    let s = (w.x * d2.y - w.y * d2.x) / den;
    let t = (w.x * d1.y - w.y * d1.x) / den;
    let es = eps / d1.length().max(1e-300);
    let et = eps / d2.length().max(1e-300);
    if (-es..=1.0 + es).contains(&s) && (-et..=1.0 + et).contains(&t) {
        SegRel::Cross(s.clamp(0.0, 1.0), t.clamp(0.0, 1.0))
    } else {
        SegRel::None
    }
}

fn param_on_segment(p: DVec2, a: DVec2, b: DVec2, eps: f64) -> Option<f64> {
    let d = b - a;
    let l2 = d.length_squared();
    if l2 < 1e-300 {
        return None;
    }
    let t = (p - a).dot(d) / l2;
    if !(-1e-9..=1.0 + 1e-9).contains(&t) {
        return None;
    }
    let q = a + d * t.clamp(0.0, 1.0);
    if (q - p).length() <= eps { Some(t.clamp(0.0, 1.0)) } else { None }
}

// ---------------------------------------------------------------- face helpers

impl Face {
    /// Split the face by (u, v) paths; returns the pieces (one piece = unchanged face).
    pub fn split_by_paths(&self, paths: &[Vec<DVec2>]) -> Vec<Face> {
        let loops = self.effective_loops();
        let pieces = split_region(&loops, paths);
        pieces.into_iter().map(|l| Face { surface: self.surface.clone(), loops: l }).collect()
    }

    /// Split the face by curves projected along `dir`.
    pub fn split_by_curves(&self, cutters: &[NurbsCurve], dir: DVec3, tol: f64) -> Vec<Face> {
        let projector = Projector::new(&self.surface, dir, tol);
        let paths: Vec<Vec<DVec2>> = cutters.iter().flat_map(|c| projector.project_curve(c)).collect();
        if paths.is_empty() {
            return vec![self.clone()];
        }
        self.split_by_paths(&paths)
    }

    /// (u, v) of a world point on the face: the projection along `dir` nearest to the point.
    pub fn uv_near(&self, p: DVec3, dir: DVec3, tol: f64) -> DVec2 {
        let projector = Projector::new(&self.surface, dir, tol);
        match projector.nearest_hit(p, p) {
            Some(h) => DVec2::new(h.u, h.v),
            None => {
                let (u, v) = self.surface.closest_uv(p);
                DVec2::new(u, v)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Plane;

    fn square() -> Vec<Vec<DVec2>> {
        vec![vec![DVec2::new(0.0, 0.0), DVec2::new(10.0, 0.0), DVec2::new(10.0, 10.0), DVec2::new(0.0, 10.0)]]
    }

    #[test]
    fn split_square_by_one_and_two_paths() {
        let p1 = vec![DVec2::new(4.0, -1.0), DVec2::new(4.0, 11.0)];
        let pieces = split_region(&square(), std::slice::from_ref(&p1));
        assert_eq!(pieces.len(), 2);
        let areas: Vec<f64> = pieces.iter().map(|p| signed_area(&p[0])).collect();
        assert!(areas.iter().all(|a| *a > 0.0));
        assert!((areas.iter().sum::<f64>() - 100.0).abs() < 1e-9);
        let p2 = vec![DVec2::new(-1.0, 5.0), DVec2::new(11.0, 5.0)];
        let pieces = split_region(&square(), &[p1, p2]);
        assert_eq!(pieces.len(), 4);
        assert!((pieces.iter().map(|p| signed_area(&p[0])).sum::<f64>() - 100.0).abs() < 1e-9);
        // a path that does not reach across leaves the square alone
        let dangling = vec![DVec2::new(5.0, 5.0), DVec2::new(5.0, 7.0)];
        assert_eq!(split_region(&square(), &[dangling]).len(), 1);
        // a path that misses the square entirely
        let outside = vec![DVec2::new(20.0, 0.0), DVec2::new(20.0, 10.0)];
        assert_eq!(split_region(&square(), &[outside]).len(), 1);
    }

    #[test]
    fn split_region_with_hole_and_closed_path() {
        let mut loops = square();
        loops.push(vec![DVec2::new(4.0, 4.0), DVec2::new(4.0, 6.0), DVec2::new(6.0, 6.0), DVec2::new(6.0, 4.0)]); // CW hole
        // vertical path through the hole: two pieces, no holes left
        let p = vec![DVec2::new(5.0, -1.0), DVec2::new(5.0, 11.0)];
        let pieces = split_region(&loops, &[p]);
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().all(|p| p.len() == 1));
        assert!((pieces.iter().map(|p| signed_area(&p[0])).sum::<f64>() - 96.0).abs() < 1e-9);
        // vertical path missing the hole: the hole stays in the right piece
        let p = vec![DVec2::new(2.0, -1.0), DVec2::new(2.0, 11.0)];
        let pieces = split_region(&loops, &[p]);
        assert_eq!(pieces.len(), 2);
        let with_hole = pieces.iter().find(|p| p.len() == 2).expect("piece with hole");
        assert!((signed_area(&with_hole[0]) - 80.0).abs() < 1e-9);
        assert!(signed_area(&with_hole[1]) < 0.0, "hole is clockwise");
        // closed path inside: island + ring with hole
        let circle: Vec<DVec2> = (0..=32).map(|i| { let a = i as f64 / 32.0 * std::f64::consts::TAU; DVec2::new(8.0 + a.cos(), 8.0 + a.sin()) }).collect();
        let pieces = split_region(&square(), &[circle]);
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().any(|p| p.len() == 2), "ring has a hole");
    }

    #[test]
    fn projection_of_a_line_on_a_loft_follows_the_exact_projection() {
        // loft between an arc and a line: a curved, non-planar surface
        let arc = NurbsCurve::arc(&Plane::world_xz(), 10.0, 0.0, std::f64::consts::PI).unwrap();
        let mut line = NurbsCurve::line(DVec3::new(10.0, 20.0, 0.0), DVec3::new(-10.0, 20.0, 0.0)).unwrap();
        line.transform(&glam::DAffine3::from_translation(DVec3::new(0.0, 0.0, 3.0)));
        let srf = NurbsSurface::loft(&[arc, line], 1).unwrap();
        let face = Face::untrimmed(srf.clone());
        let cutter = NurbsCurve::line(DVec3::new(-15.0, 10.0, 0.0), DVec3::new(15.0, 10.0, 0.0)).unwrap();
        let projector = Projector::new(&srf, DVec3::Z, 1e-4);
        let paths = projector.project_curve(&cutter);
        assert_eq!(paths.len(), 1, "one branch");
        let path = &paths[0];
        assert!(path.len() > 8, "refined path has {} points", path.len());
        for uv in path {
            let p = srf.point_at(uv.x, uv.y);
            assert!((p.y - 10.0).abs() < 1e-6, "projected point stays over the line: {p}");
        }
        let pieces = face.split_by_curves(&[cutter], DVec3::Z, 1e-4);
        assert_eq!(pieces.len(), 2);
        // the new edge lies on y = 10 within tolerance (sampled densely)
        for f in &pieces {
            for l in &f.loops {
                for uv in l {
                    let p = srf.point_at(uv.x, uv.y);
                    let on_border = uv.y.abs() < 1e-9 || (uv.y - 1.0).abs() < 1e-9 || uv.x.abs() < 1e-9 || (uv.x - 1.0).abs() < 1e-9;
                    assert!(on_border || (p.y - 10.0).abs() < 1e-3, "loop vertex off the cut: {p}");
                }
            }
        }
    }

    #[test]
    fn projection_across_the_seam_of_a_cylinder() {
        let cyl = NurbsSurface::cylinder(&Plane::world_xy(), 5.0, 10.0).unwrap();
        let face = Face::untrimmed(cyl.clone());
        // a line along x through the axis, projected along z: hits the wall twice; one of the
        // hits sits on the seam (u=0/u=1 at +x)
        let cutter = NurbsCurve::line(DVec3::new(-8.0, 0.0, 5.0), DVec3::new(8.0, 0.0, 5.0)).unwrap();
        let projector = Projector::new(&cyl, DVec3::Y, 1e-4);
        let paths = projector.project_curve(&cutter);
        // along Y the line projects onto the wall as the whole circle at z=5: the branches
        // (front/back, split at the seam at +x) are joined into one path from seam to seam
        assert_eq!(paths.len(), 1, "branches: {}", paths.len());
        let ends = (paths[0][0], *paths[0].last().unwrap());
        assert!((ends.0.x - ends.1.x).abs() > 0.99, "path runs from one side of the seam to the other: {ends:?}");
        for p in &paths {
            for uv in p {
                let q = cyl.point_at(uv.x, uv.y);
                assert!((q.z - 5.0).abs() < 1e-6);
                assert!((q.x * q.x + q.y * q.y - 25.0).abs() < 1e-5, "point on the wall: {q}");
            }
        }
        let pieces = face.split_by_curves(&[cutter], DVec3::Y, 1e-4);
        assert_eq!(pieces.len(), 2, "cylinder wall cut in two rings: {}", pieces.len());
        let heights: Vec<f64> = pieces.iter().map(|f| f.bounding_box().max.z - f.bounding_box().min.z).collect();
        assert!(heights.iter().all(|h| (h - 5.0).abs() < 0.2), "ring heights {heights:?}");
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn four_paths_forming_a_square_make_an_island() {
        let sq = vec![vec![DVec2::new(0.0, 0.0), DVec2::new(1.0, 0.0), DVec2::new(1.0, 1.0), DVec2::new(0.0, 1.0)]];
        let paths = vec![
            vec![DVec2::new(0.3, 0.3), DVec2::new(0.3, 0.5), DVec2::new(0.3, 0.7)],
            vec![DVec2::new(0.3, 0.7), DVec2::new(0.5, 0.7), DVec2::new(0.7, 0.7)],
            vec![DVec2::new(0.7, 0.7), DVec2::new(0.7, 0.5), DVec2::new(0.7, 0.3)],
            vec![DVec2::new(0.7, 0.3), DVec2::new(0.5, 0.3), DVec2::new(0.3, 0.3)],
        ];
        let pieces = split_region(&sq, &paths);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        let paths = vec![
            vec![DVec2::new(3e-1,3e-1), DVec2::new(3e-1,6e-1), DVec2::new(2.999_999_999_999_999_3e-1,6.999_999_999_999_998e-1)],
            vec![DVec2::new(7e-1,3e-1), DVec2::new(7e-1,6e-1), DVec2::new(7e-1,7e-1)],
            vec![DVec2::new(3e-1,3e-1), DVec2::new(6e-1,3e-1), DVec2::new(7e-1,3e-1)],
            vec![DVec2::new(2.999_999_999_999_999_3e-1,6.999_999_999_999_998e-1), DVec2::new(6e-1,7e-1), DVec2::new(7e-1,7e-1)],
        ];
        let pieces = split_region(&sq, &paths);
        assert_eq!(pieces.len(), 2, "{pieces:?}");
        let a = crate::Brep::cuboid(&crate::Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
        let f = &a.faces[0];
        eprintln!("loops {:?} domain {:?} {:?}", f.effective_loops(), f.surface.domain_u(), f.surface.domain_v());
        let pieces = f.split_by_paths(&paths);
        assert_eq!(pieces.len(), 2, "{:?}", pieces.iter().map(|p| p.loops.clone()).collect::<Vec<_>>());
    }
}
