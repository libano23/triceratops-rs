use glam::{DAffine3, DVec3};
use serde::{Deserialize, Serialize};

use crate::{BoundingBox, Ray};

/// Indexed triangle mesh used both as display tessellation and as a document object.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Mesh {
    pub vertices: Vec<DVec3>,
    pub normals: Vec<DVec3>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn triangle(&self, i: usize) -> (DVec3, DVec3, DVec3) {
        (
            self.vertices[self.indices[3 * i] as usize],
            self.vertices[self.indices[3 * i + 1] as usize],
            self.vertices[self.indices[3 * i + 2] as usize],
        )
    }

    /// Recompute smooth, area-weighted vertex normals.
    pub fn compute_normals(&mut self) {
        self.normals = vec![DVec3::ZERO; self.vertices.len()];
        for t in 0..self.triangle_count() {
            let (a, b, c) = self.triangle(t);
            let n = (b - a).cross(c - a);
            for k in 0..3 {
                let vi = self.indices[3 * t + k] as usize;
                self.normals[vi] += n;
            }
        }
        for n in self.normals.iter_mut() {
            *n = n.normalize_or(DVec3::Z);
        }
    }

    pub fn append(&mut self, other: &Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&other.vertices);
        self.normals.extend_from_slice(&other.normals);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    pub fn bounding_box(&self) -> BoundingBox {
        BoundingBox::from_points(&self.vertices)
    }

    pub fn transform(&mut self, xf: &DAffine3) {
        for v in self.vertices.iter_mut() {
            *v = xf.transform_point3(*v);
        }
        let nm = xf.matrix3.inverse().transpose();
        for n in self.normals.iter_mut() {
            *n = (nm * *n).normalize_or(DVec3::Z);
        }
    }

    pub fn flip(&mut self) {
        for t in self.indices.chunks_mut(3) {
            t.swap(1, 2);
        }
        for n in self.normals.iter_mut() {
            *n = -*n;
        }
    }

    /// Nearest ray hit parameter against all triangles.
    pub fn intersect_ray(&self, ray: &Ray) -> Option<f64> {
        let mut best: Option<f64> = None;
        for t in 0..self.triangle_count() {
            let (a, b, c) = self.triangle(t);
            if let Some(h) = ray.intersect_triangle(a, b, c) {
                if best.is_none_or(|bt| h < bt) {
                    best = Some(h);
                }
            }
        }
        best
    }

    /// Whether the mesh is watertight: every edge (by vertex position) is used by exactly two triangles.
    pub fn is_closed(&self) -> bool {
        use std::collections::HashMap;
        if self.triangle_count() == 0 {
            return false;
        }
        let bb = self.bounding_box();
        let q = bb.diagonal().max(1e-9) * 1e-7;
        let key = |p: DVec3| ((p.x / q).round() as i64, (p.y / q).round() as i64, (p.z / q).round() as i64);
        let mut ids: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut remap = Vec::with_capacity(self.vertices.len());
        for v in &self.vertices {
            let n = ids.len() as u32;
            remap.push(*ids.entry(key(*v)).or_insert(n));
        }
        let mut count: HashMap<(u32, u32), u32> = HashMap::new();
        for t in self.indices.chunks(3) {
            for k in 0..3 {
                let a = remap[t[k] as usize];
                let b = remap[t[(k + 1) % 3] as usize];
                if a == b {
                    continue;
                }
                *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        count.values().all(|c| *c == 2)
    }

    /// Weld coincident vertices (by position) and split triangles whose edges pass through
    /// another vertex (T-junctions, typical of CSG output) so the mesh becomes watertight.
    pub fn repair_tjunctions(&self) -> Mesh {
        use std::collections::HashMap;
        let bb = self.bounding_box();
        let q = bb.diagonal().max(1e-9) * 1e-7;
        let key = |p: DVec3| ((p.x / q).round() as i64, (p.y / q).round() as i64, (p.z / q).round() as i64);
        // weld
        let mut ids: HashMap<(i64, i64, i64), u32> = HashMap::new();
        let mut verts: Vec<DVec3> = Vec::new();
        let mut remap = Vec::with_capacity(self.vertices.len());
        for v in &self.vertices {
            let n = verts.len() as u32;
            let id = *ids.entry(key(*v)).or_insert_with(|| {
                verts.push(*v);
                n
            });
            remap.push(id);
        }
        let mut tris: Vec<[u32; 3]> = self
            .indices
            .chunks(3)
            .map(|t| [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]])
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .collect();
        // spatial grid of vertices for candidate lookup
        let cell = (bb.diagonal().max(1e-9) * 0.02).max(q * 10.0);
        let ckey = |p: DVec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
        let mut grid: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        for (i, v) in verts.iter().enumerate() {
            grid.entry(ckey(*v)).or_default().push(i as u32);
        }
        let tol = q * 20.0;
        for _round in 0..24 {
            // naked edges
            let mut count: HashMap<(u32, u32), u32> = HashMap::new();
            for t in &tris {
                for k in 0..3 {
                    let (a, b) = (t[k], t[(k + 1) % 3]);
                    *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
            let naked: Vec<(u32, u32)> = count.iter().filter(|(_, c)| **c == 1).map(|(e, _)| *e).collect();
            if naked.is_empty() {
                break;
            }
            // for each naked edge find a vertex strictly inside the segment
            let mut splits: HashMap<(u32, u32), u32> = HashMap::new();
            for (a, b) in naked {
                let (pa, pb) = (verts[a as usize], verts[b as usize]);
                let ab = pb - pa;
                let l2 = ab.length_squared();
                if l2 < q * q {
                    continue;
                }
                let lo = ckey(pa.min(pb) - DVec3::splat(tol));
                let hi = ckey(pa.max(pb) + DVec3::splat(tol));
                let mut best: Option<(f64, u32)> = None;
                for x in lo.0..=hi.0 {
                    for y in lo.1..=hi.1 {
                        for z in lo.2..=hi.2 {
                            if let Some(list) = grid.get(&(x, y, z)) {
                                for &vi in list {
                                    if vi == a || vi == b {
                                        continue;
                                    }
                                    let p = verts[vi as usize];
                                    let t = (p - pa).dot(ab) / l2;
                                    if t <= 1e-6 || t >= 1.0 - 1e-6 {
                                        continue;
                                    }
                                    let d = (pa + ab * t - p).length();
                                    if d <= tol && best.is_none_or(|(bt, _)| t < bt) {
                                        best = Some((t, vi));
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some((_, vi)) = best {
                    splits.insert((a.min(b), a.max(b)), vi);
                }
            }
            if splits.is_empty() {
                break;
            }
            let mut next: Vec<[u32; 3]> = Vec::with_capacity(tris.len() + splits.len());
            for t in &tris {
                let mut done = false;
                for k in 0..3 {
                    let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    if let Some(&v) = splits.get(&(a.min(b), a.max(b))) {
                        next.push([a, v, c]);
                        next.push([v, b, c]);
                        done = true;
                        break;
                    }
                }
                if !done {
                    next.push(*t);
                }
            }
            tris = next;
        }
        let mut out = Mesh::new();
        out.vertices = verts;
        for t in tris {
            out.indices.extend_from_slice(&t);
        }
        out.compute_normals();
        out
    }

    /// Unique edges as index pairs (useful for wireframe display).
    pub fn edges(&self) -> Vec<(u32, u32)> {
        let mut e: Vec<(u32, u32)> = Vec::with_capacity(self.indices.len());
        for t in self.indices.chunks(3) {
            for k in 0..3 {
                let a = t[k];
                let b = t[(k + 1) % 3];
                e.push((a.min(b), a.max(b)));
            }
        }
        e.sort_unstable();
        e.dedup();
        e
    }

    /// Intersection of the mesh with a plane, chained into polylines.
    pub fn section(&self, plane: &crate::Plane, tol: f64) -> Vec<Vec<DVec3>> {
        let mut segs: Vec<(DVec3, DVec3)> = Vec::new();
        for t in 0..self.triangle_count() {
            let (a, b, c) = self.triangle(t);
            let d = [plane.signed_distance(a), plane.signed_distance(b), plane.signed_distance(c)];
            let v = [a, b, c];
            let mut pts = Vec::new();
            for i in 0..3 {
                let j = (i + 1) % 3;
                if (d[i] < 0.0) != (d[j] < 0.0) {
                    let f = d[i] / (d[i] - d[j]);
                    pts.push(v[i] + (v[j] - v[i]) * f);
                } else if d[i].abs() < 1e-12 {
                    pts.push(v[i]);
                }
            }
            pts.dedup_by(|p, q| (*p - *q).length() < tol);
            if pts.len() >= 2 && (pts[0] - pts[1]).length() > tol {
                segs.push((pts[0], pts[1]));
            }
        }
        // chain segments by endpoints
        let mut chains: Vec<Vec<DVec3>> = Vec::new();
        let mut used = vec![false; segs.len()];
        for i in 0..segs.len() {
            if used[i] {
                continue;
            }
            used[i] = true;
            let mut chain = vec![segs[i].0, segs[i].1];
            loop {
                let mut extended = false;
                for j in 0..segs.len() {
                    if used[j] {
                        continue;
                    }
                    let (a, b) = segs[j];
                    let end = *chain.last().unwrap();
                    let start = chain[0];
                    if (a - end).length() < tol {
                        chain.push(b);
                    } else if (b - end).length() < tol {
                        chain.push(a);
                    } else if (b - start).length() < tol {
                        chain.insert(0, a);
                    } else if (a - start).length() < tol {
                        chain.insert(0, b);
                    } else {
                        continue;
                    }
                    used[j] = true;
                    extended = true;
                }
                if !extended {
                    break;
                }
            }
            chains.push(chain);
        }
        chains
    }

    /// Axis-aligned box mesh (used for tests and quick previews).
    pub fn cuboid(min: DVec3, max: DVec3) -> Self {
        let bb = BoundingBox::new(min, max);
        let c = bb.corners();
        let faces: [([usize; 4], DVec3); 6] = [
            ([0, 3, 2, 1], -DVec3::Z),
            ([4, 5, 6, 7], DVec3::Z),
            ([0, 1, 5, 4], -DVec3::Y),
            ([2, 3, 7, 6], DVec3::Y),
            ([1, 2, 6, 5], DVec3::X),
            ([3, 0, 4, 7], -DVec3::X),
        ];
        let mut m = Mesh::new();
        for (idx, n) in faces {
            let base = m.vertices.len() as u32;
            for i in idx {
                m.vertices.push(c[i]);
                m.normals.push(n);
            }
            m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        m
    }
}
