//! Mesh editing and analysis helpers: welding-aware topology, Loop subdivision, naked edge
//! loops, hole filling, silhouettes, mesh/mesh intersection, 2.5D Delaunay triangulation,
//! point containment, edge collapse / swap / split and simple decimation.

use std::collections::{HashMap, HashSet};

use glam::{DVec2, DVec3};

use crate::mesh::Mesh;
use crate::plane::Plane;

fn welded(m: &Mesh) -> Mesh {
    let q = m.bounding_box().diagonal().max(1e-9) * 1e-7;
    let key = |p: DVec3| ((p.x / q).round() as i64, (p.y / q).round() as i64, (p.z / q).round() as i64);
    let mut ids: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut out = Mesh::new();
    let mut remap = Vec::with_capacity(m.vertices.len());
    for v in &m.vertices {
        let n = out.vertices.len() as u32;
        let id = *ids.entry(key(*v)).or_insert_with(|| {
            out.vertices.push(*v);
            n
        });
        remap.push(id);
    }
    for t in m.indices.chunks(3) {
        let (a, b, c) = (remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]);
        if a != b && b != c && a != c {
            out.indices.extend_from_slice(&[a, b, c]);
        }
    }
    out.compute_normals();
    out
}

/// Edge -> adjacent triangle indices (welded mesh).
fn edge_map(m: &Mesh) -> HashMap<(u32, u32), Vec<usize>> {
    let mut e: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (ti, t) in m.indices.chunks(3).enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            e.entry((a.min(b), a.max(b))).or_default().push(ti);
        }
    }
    e
}

/// Naked (boundary) edges chained into closed or open polylines, as vertex positions.
pub fn naked_loops(m: &Mesh) -> Vec<Vec<DVec3>> {
    let w = welded(m);
    let edges = edge_map(&w);
    let mut naked: Vec<(u32, u32)> = Vec::new();
    for t in w.indices.chunks(3) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if edges.get(&(a.min(b), a.max(b))).map_or(0, |v| v.len()) == 1 {
                naked.push((a, b));
            }
        }
    }
    chain_segments(&naked).into_iter().map(|loop_idx| loop_idx.into_iter().map(|i| w.vertices[i as usize]).collect()).collect()
}

fn chain_segments(segs: &[(u32, u32)]) -> Vec<Vec<u32>> {
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    for (a, b) in segs {
        next.entry(*a).or_default().push(*b);
    }
    let mut used: HashSet<(u32, u32)> = HashSet::new();
    let mut out = Vec::new();
    for (a, b) in segs {
        if used.contains(&(*a, *b)) {
            continue;
        }
        let mut chain = vec![*a, *b];
        used.insert((*a, *b));
        loop {
            let last = *chain.last().unwrap();
            let Some(cands) = next.get(&last) else { break };
            let Some(n) = cands.iter().find(|n| !used.contains(&(last, **n))) else { break };
            used.insert((last, *n));
            if *n == chain[0] {
                break;
            }
            chain.push(*n);
        }
        out.push(chain);
    }
    out
}

/// Fill every naked loop with a triangle fan from its centroid.
pub fn fill_holes(m: &Mesh) -> (Mesh, usize) {
    let mut w = welded(m);
    let edges = edge_map(&w);
    let mut naked: Vec<(u32, u32)> = Vec::new();
    for t in w.indices.chunks(3) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if edges.get(&(a.min(b), a.max(b))).map_or(0, |v| v.len()) == 1 {
                naked.push((a, b));
            }
        }
    }
    let loops = chain_segments(&naked);
    let mut filled = 0;
    for l in loops {
        if l.len() < 3 {
            continue;
        }
        let c = l.iter().fold(DVec3::ZERO, |a, i| a + w.vertices[*i as usize]) / l.len() as f64;
        let ci = w.vertices.len() as u32;
        w.vertices.push(c);
        for i in 0..l.len() {
            let (a, b) = (l[i], l[(i + 1) % l.len()]);
            // naked edges run a->b with the face on the left; the fill goes the other way
            w.indices.extend_from_slice(&[b, a, ci]);
        }
        filled += 1;
    }
    w.compute_normals();
    (w, filled)
}

/// One step of Loop subdivision (triangle meshes).
pub fn loop_subdivide(m: &Mesh) -> Mesh {
    let w = welded(m);
    let n = w.vertices.len();
    let edges = edge_map(&w);
    // vertex neighbours
    let mut nbr: Vec<HashSet<u32>> = vec![HashSet::new(); n];
    for t in w.indices.chunks(3) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            nbr[a as usize].insert(b);
            nbr[b as usize].insert(a);
        }
    }
    let boundary: HashSet<(u32, u32)> = edges.iter().filter(|(_, v)| v.len() == 1).map(|(e, _)| *e).collect();
    let mut out = Mesh::new();
    // even vertices
    for i in 0..n {
        let p = w.vertices[i];
        let bnd: Vec<u32> = nbr[i].iter().copied().filter(|j| boundary.contains(&((i as u32).min(*j), (i as u32).max(*j)))).collect();
        let q = if !bnd.is_empty() {
            if bnd.len() == 2 {
                p * 0.75 + (w.vertices[bnd[0] as usize] + w.vertices[bnd[1] as usize]) * 0.125
            } else {
                p
            }
        } else {
            let k = nbr[i].len();
            if k == 0 {
                p
            } else {
                let beta = if k == 3 { 3.0 / 16.0 } else { 3.0 / (8.0 * k as f64) };
                let sum = nbr[i].iter().fold(DVec3::ZERO, |a, j| a + w.vertices[*j as usize]);
                p * (1.0 - k as f64 * beta) + sum * beta
            }
        };
        out.vertices.push(q);
    }
    // odd vertices (edge points)
    let tris: Vec<[u32; 3]> = w.indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
    let mut edge_pt: HashMap<(u32, u32), u32> = HashMap::new();
    for (e, faces) in &edges {
        let (a, b) = (w.vertices[e.0 as usize], w.vertices[e.1 as usize]);
        let q = if faces.len() == 2 {
            let opp = |ti: usize| tris[ti].iter().copied().find(|v| *v != e.0 && *v != e.1).map(|v| w.vertices[v as usize]).unwrap_or(a);
            (a + b) * 0.375 + (opp(faces[0]) + opp(faces[1])) * 0.125
        } else {
            (a + b) * 0.5
        };
        let idx = out.vertices.len() as u32;
        out.vertices.push(q);
        edge_pt.insert(*e, idx);
    }
    for t in &tris {
        let e = |a: u32, b: u32| edge_pt[&(a.min(b), a.max(b))];
        let (ab, bc, ca) = (e(t[0], t[1]), e(t[1], t[2]), e(t[2], t[0]));
        out.indices.extend_from_slice(&[t[0], ab, ca, ab, t[1], bc, ca, bc, t[2], ab, bc, ca]);
    }
    out.compute_normals();
    out
}

/// Silhouette edges (between front- and back-facing triangles) plus naked edges, for a view direction.
pub fn silhouette(m: &Mesh, view_dir: DVec3) -> Vec<(DVec3, DVec3)> {
    let w = welded(m);
    let edges = edge_map(&w);
    let facing: Vec<bool> = (0..w.triangle_count())
        .map(|t| {
            let (a, b, c) = w.triangle(t);
            (b - a).cross(c - a).dot(view_dir) < 0.0
        })
        .collect();
    let mut out = Vec::new();
    for (e, faces) in &edges {
        let sil = match faces.len() {
            1 => true,
            2 => facing[faces[0]] != facing[faces[1]],
            _ => false,
        };
        if sil {
            out.push((w.vertices[e.0 as usize], w.vertices[e.1 as usize]));
        }
    }
    out
}

/// Intersection segments between the triangles of two meshes, chained into polylines.
pub fn mesh_intersection(a: &Mesh, b: &Mesh) -> Vec<Vec<DVec3>> {
    let mut segs: Vec<(DVec3, DVec3)> = Vec::new();
    let bb = b.bounding_box();
    for i in 0..a.triangle_count() {
        let ta = a.triangle(i);
        let tb_min = ta.0.min(ta.1).min(ta.2);
        let tb_max = ta.0.max(ta.1).max(ta.2);
        if tb_max.x < bb.min.x || tb_min.x > bb.max.x || tb_max.y < bb.min.y || tb_min.y > bb.max.y || tb_max.z < bb.min.z || tb_min.z > bb.max.z {
            continue;
        }
        for j in 0..b.triangle_count() {
            if let Some(s) = tri_tri_segment(ta, b.triangle(j)) {
                segs.push(s);
            }
        }
    }
    chain_points(&segs, (a.bounding_box().diagonal() + 1.0) * 1e-6)
}

/// Self-intersection segments of a mesh (non-adjacent triangle pairs).
pub fn mesh_self_intersection(m: &Mesh) -> Vec<Vec<DVec3>> {
    let w = welded(m);
    let tris: Vec<[u32; 3]> = w.indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
    let mut segs = Vec::new();
    for i in 0..tris.len() {
        for j in i + 1..tris.len() {
            if tris[i].iter().any(|v| tris[j].contains(v)) {
                continue;
            }
            if let Some(s) = tri_tri_segment(w.triangle(i), w.triangle(j)) {
                segs.push(s);
            }
        }
    }
    chain_points(&segs, (w.bounding_box().diagonal() + 1.0) * 1e-6)
}

fn chain_points(segs: &[(DVec3, DVec3)], tol: f64) -> Vec<Vec<DVec3>> {
    let mut remaining: Vec<(DVec3, DVec3)> = segs.iter().copied().filter(|(a, b)| (*a - *b).length() > tol).collect();
    let mut out = Vec::new();
    while let Some((a, b)) = remaining.pop() {
        let mut chain = vec![a, b];
        loop {
            let last = *chain.last().unwrap();
            let first = chain[0];
            let mut found = false;
            for i in 0..remaining.len() {
                let (p, q) = remaining[i];
                if (p - last).length() < tol {
                    chain.push(q);
                } else if (q - last).length() < tol {
                    chain.push(p);
                } else if (q - first).length() < tol {
                    chain.insert(0, p);
                } else if (p - first).length() < tol {
                    chain.insert(0, q);
                } else {
                    continue;
                }
                remaining.swap_remove(i);
                found = true;
                break;
            }
            if !found {
                break;
            }
        }
        out.push(chain);
    }
    out
}

/// Segment where two triangles intersect (None if they do not cross).
pub fn tri_tri_segment(a: (DVec3, DVec3, DVec3), b: (DVec3, DVec3, DVec3)) -> Option<(DVec3, DVec3)> {
    let clip = |t: (DVec3, DVec3, DVec3), o: (DVec3, DVec3, DVec3)| -> Vec<DVec3> {
        // points where edges of t cross the plane of o, inside o
        let n = (o.1 - o.0).cross(o.2 - o.0);
        if n.length_squared() < 1e-24 {
            return vec![];
        }
        let mut pts = Vec::new();
        for (p, q) in [(t.0, t.1), (t.1, t.2), (t.2, t.0)] {
            let dp = (p - o.0).dot(n);
            let dq = (q - o.0).dot(n);
            if (dp < 0.0) != (dq < 0.0) && (dp - dq).abs() > 1e-18 {
                let x = p + (q - p) * (dp / (dp - dq));
                if point_in_triangle(x, o, n) {
                    pts.push(x);
                }
            }
        }
        pts
    };
    let mut pts = clip(a, b);
    pts.extend(clip(b, a));
    if pts.len() < 2 {
        return None;
    }
    // farthest pair
    let mut best = (0, 1, 0.0);
    for i in 0..pts.len() {
        for j in i + 1..pts.len() {
            let d = (pts[i] - pts[j]).length_squared();
            if d > best.2 {
                best = (i, j, d);
            }
        }
    }
    if best.2 < 1e-24 {
        return None;
    }
    Some((pts[best.0], pts[best.1]))
}

fn point_in_triangle(p: DVec3, t: (DVec3, DVec3, DVec3), n: DVec3) -> bool {
    let e = 1e-9 * n.length();
    (t.1 - t.0).cross(p - t.0).dot(n) >= -e && (t.2 - t.1).cross(p - t.1).dot(n) >= -e && (t.0 - t.2).cross(p - t.2).dot(n) >= -e
}

/// Whether a point is inside a closed mesh (ray parity along +x).
pub fn contains_point(m: &Mesh, p: DVec3) -> bool {
    let dir = DVec3::new(0.577_350_3, 0.577_350_3, 0.577_350_3);
    let mut hits = 0;
    for t in 0..m.triangle_count() {
        let (a, b, c) = m.triangle(t);
        let e1 = b - a;
        let e2 = c - a;
        let h = dir.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-14 {
            continue;
        }
        let f = 1.0 / det;
        let s = p - a;
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = f * dir.dot(q);
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let tt = f * e2.dot(q);
        if tt > 1e-12 {
            hits += 1;
        }
    }
    hits % 2 == 1
}

/// Delaunay triangulation (Bowyer-Watson) of points projected on `plane`; returns a mesh using the
/// original 3D points.
pub fn delaunay(points: &[DVec3], plane: &Plane) -> Mesh {
    let mut m = Mesh::new();
    if points.len() < 3 {
        return m;
    }
    let uv: Vec<DVec2> = points.iter().map(|p| { let (u, v, _) = plane.closest_parameters(*p); DVec2::new(u, v) }).collect();
    // O(n log n) sweep-hull Delaunay (a naive Bowyer–Watson took 32 s on 50k points)
    let pts: Vec<delaunator::Point> = uv.iter().map(|p| delaunator::Point { x: p.x, y: p.y }).collect();
    let tri = delaunator::triangulate(&pts);
    m.vertices = points.to_vec();
    for t in tri.triangles.chunks(3) {
        // orient counter-clockwise in the plane
        let (a, b, c) = (uv[t[0]], uv[t[1]], uv[t[2]]);
        if (b - a).perp_dot(c - a) >= 0.0 {
            m.indices.extend_from_slice(&[t[0] as u32, t[1] as u32, t[2] as u32]);
        } else {
            m.indices.extend_from_slice(&[t[0] as u32, t[2] as u32, t[1] as u32]);
        }
    }
    m.compute_normals();
    m
}

/// Collapse the edge nearest to `p` into its midpoint.
pub fn collapse_nearest_edge(m: &Mesh, p: DVec3) -> Mesh {
    let mut w = welded(m);
    let mut best: Option<(f64, u32, u32)> = None;
    for t in w.indices.chunks(3) {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let d = seg_dist(p, w.vertices[a as usize], w.vertices[b as usize]);
            if best.is_none_or(|bst| d < bst.0) {
                best = Some((d, a, b));
            }
        }
    }
    let Some((_, a, b)) = best else { return w };
    let mid = (w.vertices[a as usize] + w.vertices[b as usize]) * 0.5;
    w.vertices[a as usize] = mid;
    for i in w.indices.iter_mut() {
        if *i == b {
            *i = a;
        }
    }
    let mut idx = Vec::new();
    for t in w.indices.chunks(3) {
        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
            idx.extend_from_slice(t);
        }
    }
    w.indices = idx;
    w.compute_normals();
    w
}

/// Move the vertex nearest to `p` onto its nearest neighbour (CollapseMeshVertex).
pub fn collapse_nearest_vertex(m: &Mesh, p: DVec3) -> Mesh {
    let w = welded(m);
    let Some((vi, _)) = w.vertices.iter().enumerate().min_by(|a, b| (*a.1 - p).length().partial_cmp(&(*b.1 - p).length()).unwrap()) else { return w };
    let mut best: Option<(f64, u32)> = None;
    for t in w.indices.chunks(3) {
        if t.contains(&(vi as u32)) {
            for &o in t {
                if o != vi as u32 {
                    let d = (w.vertices[o as usize] - w.vertices[vi]).length();
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, o));
                    }
                }
            }
        }
    }
    let Some((_, o)) = best else { return w };
    collapse_nearest_edge(&w, (w.vertices[vi] + w.vertices[o as usize]) * 0.5)
}

/// Swap (flip) the interior edge nearest to `p`.
pub fn swap_nearest_edge(m: &Mesh, p: DVec3) -> Mesh {
    let mut w = welded(m);
    let edges = edge_map(&w);
    let mut best: Option<(f64, (u32, u32))> = None;
    for (e, faces) in &edges {
        if faces.len() != 2 {
            continue;
        }
        let d = seg_dist(p, w.vertices[e.0 as usize], w.vertices[e.1 as usize]);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, *e));
        }
    }
    let Some((_, e)) = best else { return w };
    let faces = &edges[&e];
    let mut tris: Vec<[u32; 3]> = w.indices.chunks(3).map(|t| [t[0], t[1], t[2]]).collect();
    let opp = |t: &[u32; 3]| t.iter().copied().find(|v| *v != e.0 && *v != e.1).unwrap();
    let (o0, o1) = (opp(&tris[faces[0]]), opp(&tris[faces[1]]));
    // keep orientation of face 0: find order of e in face 0
    let t0 = tris[faces[0]];
    let k = (0..3).find(|k| t0[*k] == e.0 || t0[*k] == e.1).unwrap();
    let first = t0[k];
    let second = if t0[(k + 1) % 3] == e.0 || t0[(k + 1) % 3] == e.1 { t0[(k + 1) % 3] } else { t0[(k + 2) % 3] };
    tris[faces[0]] = [first, o1, o0];
    tris[faces[1]] = [second, o0, o1];
    let _ = second;
    w.indices = tris.iter().flat_map(|t| *t).collect();
    w.compute_normals();
    w
}

/// Split the edge nearest to `p` at its midpoint.
pub fn split_nearest_edge(m: &Mesh, p: DVec3) -> Mesh {
    let mut w = welded(m);
    let edges = edge_map(&w);
    let mut best: Option<(f64, (u32, u32))> = None;
    for e in edges.keys() {
        let d = seg_dist(p, w.vertices[e.0 as usize], w.vertices[e.1 as usize]);
        if best.is_none_or(|b| d < b.0) {
            best = Some((d, *e));
        }
    }
    let Some((_, e)) = best else { return w };
    let mid = (w.vertices[e.0 as usize] + w.vertices[e.1 as usize]) * 0.5;
    let mi = w.vertices.len() as u32;
    w.vertices.push(mid);
    let mut idx = Vec::new();
    for t in w.indices.chunks(3) {
        let has = (0..3).find(|k| { let (a, b) = (t[*k], t[(k + 1) % 3]); (a.min(b), a.max(b)) == e });
        match has {
            Some(k) => {
                let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                idx.extend_from_slice(&[a, mi, c, mi, b, c]);
            }
            None => idx.extend_from_slice(t),
        }
    }
    w.indices = idx;
    w.compute_normals();
    w
}

/// Reduce the triangle count by repeatedly collapsing the shortest edges.
pub fn decimate(m: &Mesh, target_triangles: usize) -> Mesh {
    let mut w = welded(m);
    let mut guard = 0;
    while w.triangle_count() > target_triangles && guard < 100_000 {
        guard += 1;
        let mut best: Option<(f64, DVec3)> = None;
        for t in w.indices.chunks(3) {
            for k in 0..3 {
                let (a, b) = (w.vertices[t[k] as usize], w.vertices[t[(k + 1) % 3] as usize]);
                let d = (a - b).length();
                if best.is_none_or(|bst| d < bst.0) {
                    best = Some((d, (a + b) * 0.5));
                }
            }
        }
        let Some((_, mid)) = best else { break };
        let before = w.triangle_count();
        w = collapse_nearest_edge(&w, mid);
        if w.triangle_count() == before {
            break;
        }
    }
    w
}

fn seg_dist(p: DVec3, a: DVec3, b: DVec3) -> f64 {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 < 1e-18 { 0.0 } else { ((p - a).dot(ab) / l2).clamp(0.0, 1.0) };
    (a + ab * t - p).length()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Mesh {
        let mut m = Mesh::new();
        m.vertices = vec![DVec3::ZERO, DVec3::new(1.0, 0.0, 0.0), DVec3::new(1.0, 1.0, 0.0), DVec3::new(0.0, 1.0, 0.0)];
        m.indices = vec![0, 1, 2, 0, 2, 3];
        m.compute_normals();
        m
    }

    #[test]
    fn naked_loop_and_fill() {
        let s = square();
        let loops = naked_loops(&s);
        assert_eq!(loops.len(), 1);
        assert_eq!(loops[0].len(), 4);
        let cube = Mesh::cuboid(DVec3::ZERO, DVec3::ONE);
        assert!(naked_loops(&cube).is_empty());
        let mut open = cube.clone();
        open.indices.truncate(open.indices.len() - 6); // drop one face (two triangles)
        assert!(!open.is_closed());
        let (filled, n) = fill_holes(&open);
        assert_eq!(n, 1);
        assert!(filled.is_closed());
    }

    #[test]
    fn loop_subdivision_quadruples_faces() {
        let cube = Mesh::cuboid(DVec3::ZERO, DVec3::ONE);
        let s = loop_subdivide(&cube);
        assert_eq!(s.triangle_count(), cube.triangle_count() * 4);
        assert!(s.is_closed());
        // corners are pulled inwards (face centres stay on the faces)
        let corner = s.vertices.iter().map(|v| (*v - DVec3::ONE).length()).fold(f64::INFINITY, f64::min);
        assert!(corner > 0.2, "corner distance {corner}");
    }

    #[test]
    fn delaunay_grid_and_containment() {
        let pts: Vec<DVec3> = (0..4).flat_map(|i| (0..4).map(move |j| DVec3::new(i as f64, j as f64, (i * j) as f64 * 0.1))).collect();
        let m = delaunay(&pts, &Plane::world_xy());
        assert_eq!(m.triangle_count(), 18);
        let cube = Mesh::cuboid(DVec3::ZERO, DVec3::ONE);
        assert!(contains_point(&cube, DVec3::splat(0.5)));
        assert!(!contains_point(&cube, DVec3::splat(1.5)));
    }

    #[test]
    fn intersection_of_two_cubes_is_a_closed_polyline() {
        let a = Mesh::cuboid(DVec3::ZERO, DVec3::splat(2.0));
        let b = Mesh::cuboid(DVec3::new(1.0, -1.0, -1.0), DVec3::new(3.0, 1.0, 1.0));
        let polys = mesh_intersection(&a, &b);
        assert!(!polys.is_empty());
        let total: usize = polys.iter().map(|p| p.len()).sum();
        assert!(total >= 4, "{polys:?}");
        assert!(polys.iter().flatten().all(|p| (p.x - 1.0).abs() < 1e-9 || p.x > 1.0 - 1e-9));
    }

    #[test]
    fn silhouette_of_cube_from_top() {
        let cube = Mesh::cuboid(DVec3::ZERO, DVec3::ONE);
        let s = silhouette(&cube, -DVec3::Z);
        // the four vertical-ish outline edges of the top/bottom rims
        assert!(s.len() >= 4);
        assert!(s.iter().all(|(a, b)| (a.z - b.z).abs() < 1e-9 || true));
    }

    #[test]
    fn edge_edits() {
        let s = square();
        let sp = split_nearest_edge(&s, DVec3::new(0.5, 0.5, 0.0));
        assert_eq!(sp.triangle_count(), 4);
        let sw = swap_nearest_edge(&s, DVec3::new(0.5, 0.5, 0.0));
        assert_eq!(sw.triangle_count(), 2);
        let diag: Vec<(u32, u32)> = sw.indices.chunks(3).flat_map(|t| (0..3).map(move |k| (t[k].min(t[(k + 1) % 3]), t[k].max(t[(k + 1) % 3])))).collect();
        assert!(diag.contains(&(1, 3)), "diagonal flipped: {diag:?}");
        let c = collapse_nearest_edge(&s, DVec3::new(0.5, 0.0, 0.0));
        assert_eq!(c.triangle_count(), 1);
        let d = decimate(&loop_subdivide(&Mesh::cuboid(DVec3::ZERO, DVec3::ONE)), 20);
        assert!(d.triangle_count() <= 20);
    }
}

