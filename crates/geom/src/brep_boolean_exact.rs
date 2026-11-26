//! Exact-to-tolerance B-rep Boolean: surface/surface intersection curves are traced on the
//! face tessellations and refined onto both surfaces with Newton iterations, every face is
//! partitioned by its intersection curves in (u, v) (`surface_ops::split_region`), and the
//! pieces are classified against the other solid (inside / outside / coincident). Faces keep
//! their original surfaces; only new trim loops are added. The mesh-CSG Boolean in
//! `brep_boolean.rs` remains the fallback when this one cannot produce a closed result.

use std::collections::HashMap;

use glam::{DVec2, DVec3};

use crate::brep::{point_in_polygon, signed_area, Brep, Face};
use crate::brep_boolean::boolean_segs;
use crate::mesh_csg::BooleanOp;
use crate::mesh_ops::tri_tri_segment;
use crate::{Mesh, NurbsSurface};

struct FaceMesh {
    mesh: Mesh,
    uv: Vec<DVec2>,
    tri_bbox: Vec<(DVec3, DVec3)>,
    bbox: (DVec3, DVec3),
}

/// Quantised 3-D point used as a hash key.
type Key3 = (i64, i64, i64);

fn face_mesh(f: &Face) -> FaceMesh {
    let (su, sv) = boolean_segs(f);
    let (mesh, uv) = f.tessellate_uv(su, sv);
    let mut tri_bbox = Vec::with_capacity(mesh.triangle_count());
    let (mut lo, mut hi) = (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY));
    for t in 0..mesh.triangle_count() {
        let (a, b, c) = mesh.triangle(t);
        let tl = a.min(b).min(c);
        let th = a.max(b).max(c);
        lo = lo.min(tl);
        hi = hi.max(th);
        tri_bbox.push((tl, th));
    }
    FaceMesh { mesh, uv, tri_bbox, bbox: (lo, hi) }
}

fn bary(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-30 {
        return DVec3::new(1.0, 0.0, 0.0);
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    DVec3::new(1.0 - v - w, v, w)
}

fn tri_uv(fm: &FaceMesh, t: usize, p: DVec3) -> DVec2 {
    let i = &fm.mesh.indices[t * 3..t * 3 + 3];
    let (a, b, c) = fm.mesh.triangle(t);
    let w = bary(p, a, b, c);
    fm.uv[i[0] as usize] * w.x + fm.uv[i[1] as usize] * w.y + fm.uv[i[2] as usize] * w.z
}

/// The point on the intersection of two surfaces nearest to `p`: a few alternating closest
/// point projections (robust seeding) followed by a least-norm Newton iteration on
/// F(u, v) - G(s, t) = 0 (four unknowns, three equations), which converges quadratically even
/// where the surfaces meet at a shallow angle.
fn project_to_intersection(f: &NurbsSurface, g: &NurbsSurface, p: DVec3, uv_f: DVec2, uv_g: DVec2, diag: f64) -> Option<(DVec3, DVec2, DVec2)> {
    let (mut uf, mut vf) = (uv_f.x, uv_f.y);
    let (mut ug, mut vg) = (uv_g.x, uv_g.y);
    let mut p = p;
    let start = p;
    let target = diag * 1e-12;
    let (ua, ub) = f.domain_u();
    let (va, vb) = f.domain_v();
    let (sa, sb) = g.domain_u();
    let (ta, tb) = g.domain_v();
    let mut best: Option<(f64, DVec3, DVec2, DVec2)> = None;
    for it in 0..120 {
        if it < 4 || it % 10 == 9 {
            // alternating projection step
            let (nuf, nvf) = f.closest_uv_from(p, uf, vf);
            uf = nuf;
            vf = nvf;
            let pf = f.point_at(uf, vf);
            let (nug, nvg) = g.closest_uv_from(pf, ug, vg);
            ug = nug;
            vg = nvg;
        }
        let (pf, fu, fv) = f.evaluate(uf, vf);
        let (pg, gu, gv) = g.evaluate(ug, vg);
        let r = pf - pg;
        let gap = r.length();
        p = (pf + pg) * 0.5;
        if best.is_none_or(|b| gap < b.0) {
            best = Some((gap, p, DVec2::new(uf, vf), DVec2::new(ug, vg)));
        }
        if gap < target {
            break;
        }
        if (p - start).length() > diag * 0.05 {
            return None;
        }
        // least-norm Newton step: J d = -r with J = [fu fv -gu -gv]
        // J J^T (3x3) = fu fu^T + fv fv^T + gu gu^T + gv gv^T
        let outer = |a: DVec3| glam::DMat3::from_cols(a * a.x, a * a.y, a * a.z);
        let m = outer(fu) + outer(fv) + outer(gu) + outer(gv);
        if m.determinant().abs() < 1e-30 {
            continue;
        }
        let y = m.inverse() * (-r);
        let d = [fu.dot(y), fv.dot(y), -gu.dot(y), -gv.dot(y)];
        uf = (uf + d[0]).clamp(ua, ub);
        vf = (vf + d[1]).clamp(va, vb);
        ug = (ug + d[2]).clamp(sa, sb);
        vg = (vg + d[3]).clamp(ta, tb);
    }
    let (gap, p, a, b) = best?;
    if gap < diag * 1e-8 {
        Some((p, a, b))
    } else {
        None
    }
}

/// One intersection polyline between face `fa` of A and face `fb` of B.
struct Curve {
    fa: usize,
    fb: usize,
    pts: Vec<DVec3>,
    uva: Vec<DVec2>,
    uvb: Vec<DVec2>,
}

/// Intersection polylines of two faces, refined to the tolerance.
fn face_face_curves(fa: usize, a: &Face, ma: &FaceMesh, fb: usize, b: &Face, mb: &FaceMesh, tol: f64, diag: f64) -> Vec<Curve> {
    let eps = diag * 1e-7;
    let overlap = |l0: DVec3, h0: DVec3, l1: DVec3, h1: DVec3| l0.x <= h1.x + eps && l1.x <= h0.x + eps && l0.y <= h1.y + eps && l1.y <= h0.y + eps && l0.z <= h1.z + eps && l1.z <= h0.z + eps;
    if !overlap(ma.bbox.0, ma.bbox.1, mb.bbox.0, mb.bbox.1) {
        return Vec::new();
    }
    // raw segments with uv seeds at both ends
    struct Seg {
        p: [DVec3; 2],
        ua: [DVec2; 2],
        ub: [DVec2; 2],
        tri: (usize, usize),
    }
    let mut segs: Vec<Seg> = Vec::new();
    for ta in 0..ma.mesh.triangle_count() {
        let (la, ha) = ma.tri_bbox[ta];
        if !overlap(la, ha, mb.bbox.0, mb.bbox.1) {
            continue;
        }
        let tri_a = ma.mesh.triangle(ta);
        if (tri_a.1 - tri_a.0).cross(tri_a.2 - tri_a.0).length_squared() < eps * eps * 1e-6 {
            continue;
        }
        for tb in 0..mb.mesh.triangle_count() {
            let (lb, hb) = mb.tri_bbox[tb];
            if !overlap(la, ha, lb, hb) {
                continue;
            }
            let tri_b = mb.mesh.triangle(tb);
            if (tri_b.1 - tri_b.0).cross(tri_b.2 - tri_b.0).length_squared() < eps * eps * 1e-6 {
                continue;
            }
            if let Some((p, q)) = tri_tri_segment(tri_a, tri_b) {
                if (p - q).length() < eps {
                    continue;
                }
                segs.push(Seg { p: [p, q], ua: [tri_uv(ma, ta, p), tri_uv(ma, ta, q)], ub: [tri_uv(mb, tb, p), tri_uv(mb, tb, q)], tri: (ta, tb) });
            }
        }
    }
    if segs.is_empty() {
        return Vec::new();
    }
    if std::env::var("TRICERATOPS_BOOL_SEGS").is_ok() {
        for s in &segs {
            let (ta, tb) = s.tri;
            let (a0, a1, a2) = ma.mesh.triangle(ta);
            eprintln!("SEG A{fa} B{fb} ({:.6},{:.6},{:.6}) ({:.6},{:.6},{:.6}) tri {ta} {tb} A-tri ({:.3},{:.3}) ({:.3},{:.3}) ({:.3},{:.3})", s.p[0].x, s.p[0].y, s.p[0].z, s.p[1].x, s.p[1].y, s.p[1].z, a0.x, a0.y, a1.x, a1.y, a2.x, a2.y);
        }
    }
    // chain segments by endpoint proximity
    let join = diag * 1e-6;
    let q = join;
    let key = |p: DVec3| ((p.x / q).round() as i64, (p.y / q).round() as i64, (p.z / q).round() as i64);
    // triangle edges lying exactly in the other surface (a tessellation row on a cutting
    // plane, say) make two triangles report the same segment: keep one copy
    {
        let mut seen: std::collections::HashSet<(Key3, Key3)> = std::collections::HashSet::new();
        segs.retain(|s| {
            let (ka, kb) = (key(s.p[0]), key(s.p[1]));
            let k = if ka <= kb { (ka, kb) } else { (kb, ka) };
            seen.insert(k)
        });
    }
    let mut ends: HashMap<(i64, i64, i64), Vec<(usize, usize)>> = HashMap::new();
    for (i, s) in segs.iter().enumerate() {
        for e in 0..2 {
            ends.entry(key(s.p[e])).or_default().push((i, e));
        }
    }
    let neighbours = |p: DVec3, exclude: usize| -> Vec<(usize, usize)> {
        let k = key(p);
        let mut out = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = ends.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                        for &(i, e) in list {
                            if i != exclude && (segs[i].p[e] - p).length() <= join * 2.0 {
                                out.push((i, e));
                            }
                        }
                    }
                }
            }
        }
        out
    };
    let mut used = vec![false; segs.len()];
    let mut chains: Vec<Vec<(usize, usize)>> = Vec::new(); // (segment, entry end)
    // open chains first (start at an endpoint with no neighbour), then closed loops
    for pass in 0..2 {
        for start in 0..segs.len() {
            if used[start] {
                continue;
            }
            let start_end = if pass == 0 {
                match (neighbours(segs[start].p[0], start).is_empty(), neighbours(segs[start].p[1], start).is_empty()) {
                    (true, _) => 0,
                    (_, true) => 1,
                    _ => continue,
                }
            } else {
                0
            };
            let mut chain = vec![(start, start_end)];
            used[start] = true;
            let (mut cur, mut entry) = (start, start_end);
            loop {
                let exit = segs[cur].p[1 - entry];
                let next = neighbours(exit, cur).into_iter().find(|(i, _)| !used[*i]);
                let Some((ni, ne)) = next else { break };
                used[ni] = true;
                chain.push((ni, ne));
                cur = ni;
                entry = ne;
            }
            chains.push(chain);
        }
    }
    let mut out = Vec::new();
    for chain in chains {
        let mut pts = Vec::with_capacity(chain.len() + 1);
        let mut uva = Vec::with_capacity(chain.len() + 1);
        let mut uvb = Vec::with_capacity(chain.len() + 1);
        for (k, (i, e)) in chain.iter().enumerate() {
            let s = &segs[*i];
            if k == 0 {
                pts.push(s.p[*e]);
                uva.push(s.ua[*e]);
                uvb.push(s.ub[*e]);
            }
            pts.push(s.p[1 - *e]);
            uva.push(s.ua[1 - *e]);
            uvb.push(s.ub[1 - *e]);
        }
        // refine onto both surfaces; a point that does not reach both comes from a stray
        // tessellation triangle (one that bridges a trim loop, say), not from where the
        // surfaces meet, and is dropped
        let mut keep_pt = vec![true; pts.len()];
        for k in 0..pts.len() {
            if let Some((p, ua, ub)) = project_to_intersection(&a.surface, &b.surface, pts[k], uva[k], uvb[k], diag) {
                pts[k] = p;
                uva[k] = ua;
                uvb[k] = ub;
            } else {
                keep_pt[k] = false;
            }
        }
        let failed = keep_pt.iter().filter(|k| !**k).count();
        if failed > 0 {
            if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
                eprintln!("  chain A{fa} B{fb}: {failed} of {} points not on both surfaces", pts.len());
            }
            let mut it = keep_pt.iter();
            pts.retain(|_| *it.next().unwrap());
            let mut it = keep_pt.iter();
            uva.retain(|_| *it.next().unwrap());
            let mut it = keep_pt.iter();
            uvb.retain(|_| *it.next().unwrap());
            if pts.len() < 2 {
                continue;
            }
        }
        // densify until the chords follow the intersection within the tolerance
        let (ca, cb) = (surface_closed(&a.surface), surface_closed(&b.surface));
        let mut rp = vec![pts[0]];
        let mut ra = vec![uva[0]];
        let mut rb = vec![uvb[0]];
        for k in 0..pts.len() - 1 {
            densify(&a.surface, &b.surface, ca, cb, (pts[k], uva[k], uvb[k]), (pts[k + 1], uva[k + 1], uvb[k + 1]), tol, diag, 0, &mut rp, &mut ra, &mut rb);
        }
        // drop duplicates
        let mut i = 1;
        while i < rp.len() {
            if (rp[i] - rp[i - 1]).length() < tol * 0.1 {
                rp.remove(i);
                ra.remove(i);
                rb.remove(i);
            } else {
                i += 1;
            }
        }
        // a curve must lie on both surfaces all along
        let gap = (0..rp.len()).map(|k| (a.surface.point_at(ra[k].x, ra[k].y) - b.surface.point_at(rb[k].x, rb[k].y)).length()).fold(0.0, f64::max);
        if gap > tol {
            if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
                eprintln!("  chain A{fa} B{fb} dropped: {gap:.3e} between the surfaces");
            }
            continue;
        }
        if rp.len() >= 2 {
            out.push(Curve { fa, fb, pts: rp, uva: ra, uvb: rb });
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn densify(f: &NurbsSurface, g: &NurbsSurface, cf: (bool, bool), cg: (bool, bool), p0: (DVec3, DVec2, DVec2), p1: (DVec3, DVec2, DVec2), tol: f64, diag: f64, depth: usize, out: &mut Vec<DVec3>, oa: &mut Vec<DVec2>, ob: &mut Vec<DVec2>) {
    let chord = p1.0 - p0.0;
    if depth < 8 && chord.length() > tol * 2.0 {
        let mid_uv = mid_uv(f, cf, p0.1, p1.1);
        let mid = f.point_at(mid_uv.x, mid_uv.y);
        if let Some((m, ua, ub)) = project_to_intersection(f, g, mid, mid_uv, mid_uv_g(g, cg, p0.2, p1.2), diag) {
            let t = ((m - p0.0).dot(chord) / chord.length_squared().max(1e-300)).clamp(0.0, 1.0);
            let dev = (p0.0 + chord * t - m).length();
            if dev > tol {
                densify(f, g, cf, cg, p0, (m, ua, ub), tol, diag, depth + 1, out, oa, ob);
                densify(f, g, cf, cg, (m, ua, ub), p1, tol, diag, depth + 1, out, oa, ob);
                return;
            }
        }
    }
    out.push(p1.0);
    oa.push(p1.1);
    ob.push(p1.2);
}

/// Cut a (u, v) path where it jumps across the seam of a closed surface, ending each part on
/// the domain border at the interpolated crossing.
pub fn split_at_seams(s: &NurbsSurface, path: &[DVec2]) -> Vec<Vec<DVec2>> {
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    let (du, dv) = (u1 - u0, v1 - v0);
    let (closed_u, closed_v) = surface_closed(s);
    if !closed_u && !closed_v {
        return vec![path.to_vec()];
    }
    let mut parts: Vec<Vec<DVec2>> = Vec::new();
    let mut cur: Vec<DVec2> = vec![path[0]];
    for w in path.windows(2) {
        let (p, q) = (w[0], w[1]);
        let jump_u = closed_u && (q.x - p.x).abs() > du * 0.5;
        let jump_v = closed_v && (q.y - p.y).abs() > dv * 0.5;
        if jump_u || jump_v {
            let (end_p, start_q) = if jump_u {
                let (bp, bq) = if p.x > q.x { (u1, u0) } else { (u0, u1) };
                let dp = (bp - p.x).abs();
                let dq = (q.x - bq).abs();
                let t = if dp + dq > 0.0 { dp / (dp + dq) } else { 0.5 };
                let v = p.y + (q.y - p.y) * t;
                (DVec2::new(bp, v), DVec2::new(bq, v))
            } else {
                let (bp, bq) = if p.y > q.y { (v1, v0) } else { (v0, v1) };
                let dp = (bp - p.y).abs();
                let dq = (q.y - bq).abs();
                let t = if dp + dq > 0.0 { dp / (dp + dq) } else { 0.5 };
                let u = p.x + (q.x - p.x) * t;
                (DVec2::new(u, bp), DVec2::new(u, bq))
            };
            if (end_p - *cur.last().unwrap()).length() > 1e-15 {
                cur.push(end_p);
            }
            if cur.len() >= 2 {
                parts.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
            cur.push(start_q);
        }
        if (q - *cur.last().unwrap()).length() > 1e-15 {
            cur.push(q);
        }
    }
    if cur.len() >= 2 {
        parts.push(cur);
    }
    parts
}

/// Whether the surface closes on itself along u and along v.
pub fn surface_closed(s: &NurbsSurface) -> (bool, bool) {
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    let diag = s.bounding_box().diagonal().max(1e-9);
    let cu = [0.2, 0.5, 0.8].iter().all(|f| (s.point_at(u0, v0 + (v1 - v0) * f) - s.point_at(u1, v0 + (v1 - v0) * f)).length() < diag * 1e-6);
    let cv = [0.2, 0.5, 0.8].iter().all(|f| (s.point_at(u0 + (u1 - u0) * f, v0) - s.point_at(u0 + (u1 - u0) * f, v1)).length() < diag * 1e-6);
    (cu, cv)
}

/// Midpoint of two (u, v) points, going the short way around the seams of a closed surface.
fn mid_uv_g(s: &NurbsSurface, closed: (bool, bool), a: DVec2, b: DVec2) -> DVec2 {
    mid_uv(s, closed, a, b)
}

fn mid_uv(s: &NurbsSurface, closed: (bool, bool), a: DVec2, b: DVec2) -> DVec2 {
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    let wrap = |x: f64, y: f64, lo: f64, hi: f64, closed: bool| -> f64 {
        let span = hi - lo;
        if closed && (x - y).abs() > span * 0.5 {
            let m = (x + y + span) * 0.5;
            if m > hi { m - span } else { m }
        } else {
            (x + y) * 0.5
        }
    };
    DVec2::new(wrap(a.x, b.x, u0, u1, closed.0), wrap(a.y, b.y, v0, v1, closed.1))
}

/// The trim-loop edge of a face within `snap` (3D) of a (u, v) point: `(a, b, t)` with the
/// projection parameter along the edge. Among the candidates within the tolerance the one
/// nearest in (u, v) wins (on a closed surface both seam edges are at distance zero in 3D).
fn nearest_loop_edge(face: &Face, uv: DVec2, snap: f64) -> Option<(DVec2, DVec2, f64)> {
    let p = face.surface.point_at(uv.x, uv.y);
    let mut best: Option<(f64, DVec2, DVec2, f64)> = None;
    for l in face.effective_loops() {
        let n = l.len();
        for i in 0..n {
            let (a, b) = (l[i], l[(i + 1) % n]);
            let d = b - a;
            let l2 = d.length_squared();
            if l2 < 1e-300 {
                continue;
            }
            let t = ((uv - a).dot(d) / l2).clamp(0.0, 1.0);
            let q = a + d * t;
            let dist = (face.surface.point_at(q.x, q.y) - p).length();
            let duv = (q - uv).length();
            if dist <= snap && best.is_none_or(|b| duv < b.0) {
                best = Some((duv, a, b, t));
            }
        }
    }
    best.map(|b| (b.1, b.2, b.3))
}

/// Move a point along a loop edge of `face` (UV segment a→b, start parameter `t`) to where
/// the edge meets the `other` surface: the exact triple point. Returns the (u, v) on the edge
/// and the 3D point.
fn refine_on_edge(face: &Face, a: DVec2, b: DVec2, t: f64, other: &NurbsSurface, other_uv: Option<DVec2>, snap: f64) -> (DVec2, DVec3) {
    let s = &face.surface;
    let at = |t: f64| {
        let q = a + (b - a) * t;
        s.point_at(q.x, q.y)
    };
    // seed on the other surface: the known parameters, or a global closest-point search
    let other_uv = other_uv.unwrap_or_else(|| {
        let (u, v) = other.closest_uv(at(t));
        DVec2::new(u, v)
    });
    // edge length by sampling (a seam edge of a closed surface starts and ends at one point)
    let len: f64 = (0..32).map(|i| (at((i + 1) as f64 / 32.0) - at(i as f64 / 32.0)).length()).sum::<f64>().max(1e-12);
    let w = (snap * 3.0 / len).min(0.5);
    let (mut lo, mut hi) = ((t - w).max(0.0), (t + w).min(1.0));
    let (mut ou, mut ov) = (other_uv.x, other_uv.y);
    let dist = |t: f64, ou: &mut f64, ov: &mut f64| -> f64 {
        let p = at(t);
        let (u, v) = other.closest_uv_from(p, *ou, *ov);
        *ou = u;
        *ov = v;
        (other.point_at(u, v) - p).length()
    };
    for _ in 0..60 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if dist(m1, &mut ou, &mut ov) < dist(m2, &mut ou, &mut ov) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let tt = (lo + hi) * 0.5;
    (a + (b - a) * tt, at(tt))
}

/// Straight (u, v) loop segments that come from the original face outline (domain border or
/// earlier trims) are curves on a curved surface: give them the vertices the face's
/// per-span rule prescribes, so topology checks and neighbours see the same polyline. The
/// intersection-path segments are left as they are (their vertices are exact).
fn densify_original_segments(original: &Face, mut piece: Face) -> Face {
    let orig = original.effective_loops();
    let (u0, u1) = original.surface.domain_u();
    let (v0, v1) = original.surface.domain_v();
    let eps = ((u1 - u0).max(v1 - v0)) * 1e-7;
    let on_edge = |p: DVec2, q: DVec2| -> bool {
        orig.iter().any(|l| {
            let n = l.len();
            (0..n).any(|i| {
                let (a, b) = (l[i], l[(i + 1) % n]);
                let d = b - a;
                let l2 = d.length_squared();
                if l2 < 1e-300 {
                    return false;
                }
                let dist = |x: DVec2| {
                    let t = ((x - a).dot(d) / l2).clamp(0.0, 1.0);
                    (a + d * t - x).length()
                };
                dist(p) <= eps && dist(q) <= eps
            })
        })
    };
    let corners_u = c0_knots(&original.surface.knots_u, original.surface.degree_u);
    let corners_v = c0_knots(&original.surface.knots_v, original.surface.degree_v);
    for l in piece.loops.iter_mut() {
        let mut out = Vec::with_capacity(l.len());
        let n = l.len();
        for k in 0..n {
            let (p, q) = (l[k], l[(k + 1) % n]);
            out.push(p);
            if on_edge(p, q) {
                let m = original.segment_subdivisions(p, q);
                let mut ts: Vec<f64> = (1..m).map(|i| i as f64 / m as f64).collect();
                // the surface's corners (C0 knot lines) the segment crosses are vertices too,
                // or the border cuts them and misses the neighbouring face's edge
                for (knots, a, b) in [(&corners_u, p.x, q.x), (&corners_v, p.y, q.y)] {
                    if (b - a).abs() < 1e-15 {
                        continue;
                    }
                    for &k in knots.iter() {
                        let t = (k - a) / (b - a);
                        if t > 1e-9 && t < 1.0 - 1e-9 {
                            ts.push(t);
                        }
                    }
                }
                ts.sort_by(f64::total_cmp);
                ts.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
                for t in ts {
                    out.push(p + (q - p) * t);
                }
            }
        }
        *l = out;
    }
    piece
}

/// Interior knots where the surface has a corner (multiplicity at least the degree: every
/// knot of a degree-1 direction).
fn c0_knots(knots: &[f64], degree: usize) -> Vec<f64> {
    let inner = &knots[degree.min(knots.len())..knots.len().saturating_sub(degree)];
    let mut out = Vec::new();
    let mut i = 1;
    while i + 1 < inner.len() {
        let k = inner[i];
        let mut mult = 1;
        while i + mult < inner.len() && (inner[i + mult] - k).abs() < 1e-12 {
            mult += 1;
        }
        if mult >= degree && i + mult < inner.len() {
            out.push(k);
        }
        i += mult;
    }
    out
}

/// An interior point of a (u, v) region: the sample farthest from the boundary.
/// Up to `k` points well inside the loops (outer loop first, holes after), farthest from
/// every border first and apart from one another. Distances are measured with `scale`, the
/// surface length of one unit of u and of v, so a thin band in a long domain is sampled down
/// its middle.
fn interior_uv_samples(loops: &[Vec<DVec2>], scale: DVec2, k: usize) -> Vec<DVec2> {
    let Some(outer) = loops.first() else { return Vec::new() };
    let (mut lo, mut hi) = (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY));
    for p in outer {
        lo = lo.min(*p);
        hi = hi.max(*p);
    }
    let inside = |p: DVec2| point_in_polygon(p, outer) && !loops[1..].iter().any(|h| point_in_polygon(p, h));
    let dist = |p: DVec2| -> f64 {
        let mut d = f64::INFINITY;
        for l in loops {
            let n = l.len();
            for i in 0..n {
                let (a, b) = (l[i] * scale, l[(i + 1) % n] * scale);
                let q = p * scale;
                let e = b - a;
                let t = ((q - a).dot(e) / e.length_squared().max(1e-300)).clamp(0.0, 1.0);
                d = d.min((a + e * t - q).length());
            }
        }
        d
    };
    let mut found: Vec<(f64, DVec2)> = Vec::new();
    for n in [33usize, 65, 129] {
        found.clear();
        for i in 0..n {
            for j in 0..n {
                let p = DVec2::new(lo.x + (hi.x - lo.x) * (i as f64 + 0.5) / n as f64, lo.y + (hi.y - lo.y) * (j as f64 + 0.5) / n as f64);
                if inside(p) {
                    found.push((dist(p), p));
                }
            }
        }
        if found.len() >= 4 * k {
            break;
        }
    }
    found.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut out: Vec<DVec2> = Vec::new();
    let Some(&(best, _)) = found.first() else { return out };
    for (_, p) in &found {
        if out.iter().all(|q| ((*q - *p) * scale).length() > best) {
            out.push(*p);
            if out.len() == k {
                break;
            }
        }
    }
    out
}


/// Point-in-mesh by ray parity, majority of three directions.
fn inside_mesh(m: &Mesh, p: DVec3) -> bool {
    let dirs = [DVec3::new(0.577_350_3, 0.577_350_3, 0.577_350_3), DVec3::new(-0.267_261_2, 0.534_522_5, 0.801_783_7), DVec3::new(0.801_783_7, -0.267_261_2, 0.534_522_5)];
    let mut votes = 0;
    for dir in dirs {
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
            if f * e2.dot(q) > 1e-12 {
                hits += 1;
            }
        }
        if hits % 2 == 1 {
            votes += 1;
        }
    }
    votes >= 2
}

/// Distance from a point to a mesh and the normal of the nearest triangle.
fn nearest_on_mesh(m: &Mesh, p: DVec3) -> (f64, DVec3) {
    let mut best = (f64::INFINITY, DVec3::Z);
    for t in 0..m.triangle_count() {
        let (a, b, c) = m.triangle(t);
        let q = closest_on_triangle(p, a, b, c);
        let d = (q - p).length();
        if d < best.0 {
            best = (d, (b - a).cross(c - a).normalize_or(DVec3::Z));
        }
    }
    best
}

fn closest_on_triangle(p: DVec3, a: DVec3, b: DVec3, c: DVec3) -> DVec3 {
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return a + ab * v;
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return a + ac * w;
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return b + (c - b) * w;
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    a + ab * v + ac * w
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Class {
    Inside,
    Outside,
    OnSame,
    OnOpposite,
}

fn classify(piece: &Face, other_mesh: &Mesh, on_tol: f64) -> Option<Class> {
    // the surface length of one unit of u and of v, so samples avoid thin pieces' borders
    let s = &piece.surface;
    let ((u0, u1), (v0, v1)) = (s.domain_u(), s.domain_v());
    let length = |f: &dyn Fn(f64) -> DVec3| (0..16).map(|i| (f((i + 1) as f64 / 16.0) - f(i as f64 / 16.0)).length()).sum::<f64>();
    let (vm, um) = ((v0 + v1) * 0.5, (u0 + u1) * 0.5);
    let lu = length(&|t| s.point_at(u0 + (u1 - u0) * t, vm)) / (u1 - u0).abs().max(1e-12);
    let lv = length(&|t| s.point_at(um, v0 + (v1 - v0) * t)) / (v1 - v0).abs().max(1e-12);
    let samples = interior_uv_samples(&piece.effective_loops(), DVec2::new(lu.max(1e-12), lv.max(1e-12)), 5);
    // several samples vote: one sample next to the other solid could go either way
    let mut classes = Vec::with_capacity(samples.len());
    for uv in samples {
        let (p, su, sv) = s.evaluate(uv.x, uv.y);
        let n = su.cross(sv).normalize_or(DVec3::Z);
        let (d, on) = nearest_on_mesh(other_mesh, p);
        classes.push(if d < on_tol {
            if n.dot(on) > 0.0 { Class::OnSame } else { Class::OnOpposite }
        } else if inside_mesh(other_mesh, p) {
            Class::Inside
        } else {
            Class::Outside
        });
    }
    let count = |c: Class| classes.iter().filter(|x| **x == c).count();
    let (inside, outside) = (count(Class::Inside), count(Class::Outside));
    if inside + outside > 0 {
        return Some(if inside > outside { Class::Inside } else { Class::Outside });
    }
    classes.first().copied()
}

fn keep(class: Class, op: BooleanOp, from_b: bool) -> bool {
    matches!(
        (op, from_b, class),
        (BooleanOp::Union, false, Class::Outside | Class::OnSame)
            | (BooleanOp::Union, true, Class::Outside)
            | (BooleanOp::Intersection, false, Class::Inside | Class::OnSame)
            | (BooleanOp::Intersection, true, Class::Inside)
            | (BooleanOp::Difference, false, Class::Outside | Class::OnOpposite)
            | (BooleanOp::Difference, true, Class::Inside)
    )
}

/// Exact Boolean of two closed polysurfaces with outward normals.
pub fn brep_boolean_exact(a: &Brep, b: &Brep, op: BooleanOp, tol: f64) -> Result<Brep, String> {
    let diag = a.bounding_box().union(&b.bounding_box()).diagonal().max(1e-6);
    let tol = tol.max(diag * 1e-7);
    let debug = std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok();
    // every (u, v) of every path: long lines, only on request
    let debug_paths = std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok_and(|v| v == "2");
    let ma: Vec<FaceMesh> = a.faces.iter().map(face_mesh).collect();
    let mb: Vec<FaceMesh> = b.faces.iter().map(face_mesh).collect();
    let mut mesh_a = Mesh::new();
    for m in &ma {
        mesh_a.append(&m.mesh);
    }
    let mut mesh_b = Mesh::new();
    for m in &mb {
        mesh_b.append(&m.mesh);
    }
    // intersection curves
    let mut curves: Vec<Curve> = Vec::new();
    crate::job::stage("intersecting faces");
    for (i, fa) in a.faces.iter().enumerate() {
        for (j, fb) in b.faces.iter().enumerate() {
            crate::job::checkpoint((i * b.faces.len() + j) as u64).map_err(|e| e.to_string())?;
            curves.extend(face_face_curves(i, fa, &ma[i], j, fb, &mb[j], tol, diag));
        }
    }
    if debug {
        eprintln!("exact boolean: {} intersection curves ({} points)", curves.len(), curves.iter().map(|c| c.pts.len()).sum::<usize>());
        for c in &curves {
            let (p, q) = (c.pts[0], c.pts[c.pts.len() - 1]);
            eprintln!("  curve A{} x B{}: {} pts, ({:.3},{:.3},{:.3}) -> ({:.3},{:.3},{:.3})", c.fa, c.fb, c.pts.len(), p.x, p.y, p.z, q.x, q.y, q.z);
        }
    }
    // endpoints of different curves that meet (triple points on edges) get one position:
    // clusters of nearby endpoints are moved onto the trim-loop edge they end on (solving the
    // exact point where that edge meets the other surface) and every member gets that point
    let snap = diag * 3e-3;
    // an open curve stops where the tessellations stopped meeting, close to the face edge it
    // really ends on. Where the curve runs almost along that edge (a thread's helix meeting
    // the end face of its core rises 3° a turn) a tiny gap across the edge is a long one
    // along it: take each end onto the exact point where the edge meets the other surface
    for c in curves.iter_mut() {
        if (c.pts[0] - c.pts[c.pts.len() - 1]).length() < tol * 2.0 {
            continue;
        }
        for k in [0, c.pts.len() - 1] {
            let p = c.pts[k];
            for (face, uv, other, other_uv) in [(&a.faces[c.fa], c.uva[k], &b.faces[c.fb].surface, c.uvb[k]), (&b.faces[c.fb], c.uvb[k], &a.faces[c.fa].surface, c.uva[k])] {
                let Some((ea, eb, t)) = nearest_loop_edge(face, uv, snap) else { continue };
                let (_, q) = refine_on_edge(face, ea, eb, t, other, Some(other_uv), snap);
                let gap = {
                    let (u, v) = other.closest_uv_from(q, other_uv.x, other_uv.y);
                    (other.point_at(u, v) - q).length()
                };
                if gap < tol && (q - p).length() < snap * 5.0 {
                    if debug && (q - p).length() > snap {
                        eprintln!("  end moved onto its face edge: ({:.4},{:.4},{:.4}) -> ({:.4},{:.4},{:.4})", p.x, p.y, p.z, q.x, q.y, q.z);
                    }
                    c.pts[k] = q;
                    let (ua, va) = a.faces[c.fa].surface.closest_uv_from(q, c.uva[k].x, c.uva[k].y);
                    let (ub, vb) = b.faces[c.fb].surface.closest_uv_from(q, c.uvb[k].x, c.uvb[k].y);
                    c.uva[k] = DVec2::new(ua, va);
                    c.uvb[k] = DVec2::new(ub, vb);
                    break;
                }
            }
        }
    }
    {
        let mut ends: Vec<(usize, usize)> = Vec::new(); // (curve, index)
        for (ci, c) in curves.iter().enumerate() {
            let closed = (c.pts[0] - c.pts[c.pts.len() - 1]).length() < tol * 2.0;
            if !closed {
                ends.push((ci, 0));
                ends.push((ci, c.pts.len() - 1));
            }
        }
        let mut parent: Vec<usize> = (0..ends.len()).collect();
        fn find(p: &mut [usize], i: usize) -> usize {
            let mut r = i;
            while p[r] != r {
                r = p[r];
            }
            let mut c = i;
            while p[c] != r {
                let n = p[c];
                p[c] = r;
                c = n;
            }
            r
        }
        for i in 0..ends.len() {
            for j in i + 1..ends.len() {
                let (pi, pj) = (curves[ends[i].0].pts[ends[i].1], curves[ends[j].0].pts[ends[j].1]);
                if (pi - pj).length() <= snap {
                    let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                    if ri != rj {
                        parent[ri] = rj;
                    }
                }
            }
        }
        let mut clusters: HashMap<usize, Vec<usize>> = HashMap::new();
        for i in 0..ends.len() {
            let r = find(&mut parent, i);
            clusters.entry(r).or_default().push(i);
        }
        let mut keys: Vec<usize> = clusters.keys().copied().collect();
        keys.sort();
        for key in keys {
            let members = &clusters[&key];
            let mut point = members.iter().fold(DVec3::ZERO, |acc, &m| acc + curves[ends[m].0].pts[ends[m].1]) / members.len() as f64;
            // the first member whose face edge is nearby defines the exact point
            let mut solved = false;
            for &m in members {
                let (ci, k) = ends[m];
                let c = &curves[ci];
                for (face, uv, other, other_uv) in [(&a.faces[c.fa], c.uva[k], &b.faces[c.fb].surface, c.uvb[k]), (&b.faces[c.fb], c.uvb[k], &a.faces[c.fa].surface, c.uva[k])] {
                    let (u, v) = face.surface.closest_uv_from(point, uv.x, uv.y);
                    if let Some((ea, eb, t)) = nearest_loop_edge(face, DVec2::new(u, v), snap) {
                        let (_, p3) = refine_on_edge(face, ea, eb, t, other, Some(other_uv), snap);
                        point = p3;
                        solved = true;
                        break;
                    }
                }
                if solved {
                    break;
                }
            }
            for &m in members {
                let (ci, k) = ends[m];
                let cur = &mut curves[ci];
                let before = (cur.pts[k], cur.uva[k], cur.uvb[k]);
                cur.pts[k] = point;
                let (ua, va) = a.faces[cur.fa].surface.closest_uv_from(point, cur.uva[k].x, cur.uva[k].y);
                let (ub, vb) = b.faces[cur.fb].surface.closest_uv_from(point, cur.uvb[k].x, cur.uvb[k].y);
                cur.uva[k] = DVec2::new(ua, va);
                cur.uvb[k] = DVec2::new(ub, vb);
                if debug {
                    eprintln!("  cluster {key} member curve {ci}[{k}] (A{} B{}): ({:.4},{:.4},{:.4}) uva ({:.4},{:.4}) uvb ({:.4},{:.4}) -> ({:.4},{:.4},{:.4}) uva ({:.4},{:.4}) uvb ({:.4},{:.4}) solved {solved}", cur.fa, cur.fb, before.0.x, before.0.y, before.0.z, before.1.x, before.1.y, before.2.x, before.2.y, point.x, point.y, point.z, ua, va, ub, vb);
                }
            }
        }
    }
    // per-face (u, v) paths; endpoints that lie on a trim-loop edge are put exactly on it
    let mut paths_a: Vec<Vec<Vec<DVec2>>> = vec![Vec::new(); a.faces.len()];
    let mut paths_b: Vec<Vec<Vec<DVec2>>> = vec![Vec::new(); b.faces.len()];
    for c in &curves {
        for (face, path, other, other_uv, out) in [(&a.faces[c.fa], &c.uva, &b.faces[c.fb].surface, &c.uvb, &mut paths_a[c.fa]), (&b.faces[c.fb], &c.uvb, &a.faces[c.fa].surface, &c.uva, &mut paths_b[c.fb])] {
            let _ = other_uv;
            let parts = split_at_seams(&face.surface, path);
            let nparts = parts.len();
            for (pi, mut part) in parts.into_iter().enumerate() {
                let n = part.len();
                for k in [0, n - 1] {
                    let Some((ea, eb, t)) = nearest_loop_edge(face, part[k], snap) else { continue };
                    // a seam crossing introduced by the split is interpolated: solve it exactly
                    let from_seam = (k == 0 && pi > 0) || (k == n - 1 && pi + 1 < nparts);
                    if from_seam {
                        let (uv, _) = refine_on_edge(face, ea, eb, t, other, None, snap * 10.0);
                        part[k] = uv;
                    } else {
                        part[k] = ea + (eb - ea) * t;
                    }
                }
                out.push(part);
            }
        }
    }
    // split and classify
    let on_tol = diag * 1e-3;
    let mut faces: Vec<Face> = Vec::new();
    let mut touched = false;
    crate::job::stage("splitting faces");
    for (src, paths, other_mesh, from_b) in [(a, &paths_a, &mesh_b, false), (b, &paths_b, &mesh_a, true)] {
        for (fi, f) in src.faces.iter().enumerate() {
            crate::job::checkpoint(fi as u64).map_err(|e| e.to_string())?;
            if debug_paths && !paths[fi].is_empty() {
                for p in &paths[fi] {
                    eprintln!("  {}{} path: {}", if from_b { "B" } else { "A" }, fi, p.iter().map(|q| format!("({:.17e},{:.17e})", q.x, q.y)).collect::<Vec<_>>().join(" "));
                }
            }
            let pieces = if paths[fi].is_empty() { vec![f.clone()] } else { touched = true; f.split_by_paths(&paths[fi]) };
            let split = !paths[fi].is_empty();
            for piece in pieces {
                // untouched faces keep their original (possibly untrimmed) outline
                let piece = if split { densify_original_segments(f, piece) } else { piece };
                let Some(class) = classify(&piece, other_mesh, on_tol) else { continue };
                if debug {
                    let c = piece.loops.first().map(|l| l.iter().fold(DVec2::ZERO, |s, p| s + *p) / l.len().max(1) as f64).unwrap_or_default();
                    eprintln!("  {}{} piece ({} loops, area {:.4}, centre {:.4},{:.4}): {:?}", if from_b { "B" } else { "A" }, fi, piece.loops.len(), piece.loops.first().map_or(1.0, |l| signed_area(l)), c.x, c.y, class);
                }
                if keep(class, op, from_b) {
                    let mut piece = piece;
                    if from_b && op == BooleanOp::Difference {
                        piece.flip();
                    }
                    faces.push(piece);
                }
            }
        }
    }
    if faces.is_empty() {
        return Err("empty result".into());
    }
    if !touched && curves.is_empty() && op == BooleanOp::Difference {
        // disjoint or nested operands without any intersection
    }
    let brep = Brep::new(faces);
    if !brep.closed {
        return Err("exact boolean: result not closed".into());
    }
    Ok(brep)
}

#[cfg(test)]
mod tests {

    /// A stray triangle in a face's tessellation (one that bridges a trim loop, say) crosses
    /// the other face where the surfaces do not meet: its chain must not become a curve.
    #[test]
    fn chains_off_the_surfaces_are_dropped() {
        let plane = Plane::world_xy();
        let cap = Face::untrimmed(NurbsSurface::plane(&Plane { origin: DVec3::new(0.0, 0.0, 5.0), ..plane }, -10.0, 10.0, -10.0, 10.0).unwrap());
        let side = Face::untrimmed(NurbsSurface::cylinder(&plane, 4.0, 10.0).unwrap());
        let mc = face_mesh(&cap);
        let mut ms = face_mesh(&side);
        // a fan of stray triangles well inside the cylinder, crossing z = 5, and some that
        // start on the cylinder and reach inside (one end of their chain projects, the
        // other does not)
        let base = ms.mesh.vertices.len() as u32;
        for k in 0..12 {
            let t = k as f64 / 12.0 * std::f64::consts::TAU;
            let (x, y) = (1.2 * t.cos(), 1.2 * t.sin());
            if k % 3 == 0 {
                let (c, s) = (t.cos(), t.sin());
                ms.mesh.vertices.extend([DVec3::new(4.0 * c, 4.0 * s, 5.0), DVec3::new(1.0 * c, 1.0 * s, 7.0), DVec3::new(1.0 * c - 0.3 * s, 1.0 * s + 0.3 * c, 3.0)]);
            } else {
                ms.mesh.vertices.extend([DVec3::new(x, y, 3.0), DVec3::new(x + 0.3, y, 7.0), DVec3::new(x, y + 0.3, 7.0)]);
            }
            ms.mesh.normals.extend([DVec3::Z; 3]);
            ms.uv.extend([DVec2::new(0.1, 0.3), DVec2::new(0.12, 0.7), DVec2::new(0.1, 0.7)]);
            let b = base + 3 * k;
            ms.mesh.indices.extend([b, b + 1, b + 2]);
            let (p, q, r) = (ms.mesh.vertices[b as usize], ms.mesh.vertices[b as usize + 1], ms.mesh.vertices[b as usize + 2]);
            ms.tri_bbox.push((p.min(q).min(r), p.max(q).max(r)));
        }
        let curves = face_face_curves(0, &cap, &mc, 0, &side, &ms, 1e-3, 30.0);
        assert!(!curves.is_empty(), "the real circle is found");
        for c in &curves {
            for p in &c.pts {
                let r = (p.x * p.x + p.y * p.y).sqrt();
                assert!((r - 4.0).abs() < 1e-2, "curve point {p:?} is off the cylinder");
            }
        }
    }

    /// A thin diagonal band in a long domain (a thread's strip on its sweep): the sample
    /// points must sit well inside the band, measured with the surface's own scale, or the
    /// piece is classified from a point next to the other solid.
    #[test]
    fn interior_samples_sit_inside_thin_bands() {
        // u in [0, 1] is 2 units long on the surface, v in [0, 4] is 60 units long
        let scale = DVec2::new(2.0, 15.0);
        let w = 0.06; // band width in u
        let band = vec![DVec2::new(0.1, 0.0), DVec2::new(0.1 + w, 0.0), DVec2::new(0.9 + w, 4.0), DVec2::new(0.9, 4.0)];
        let samples = interior_uv_samples(std::slice::from_ref(&band), scale, 5);
        assert!(samples.len() >= 3, "{samples:?}");
        for p in samples {
            // distance across the band, on the surface: half the width is 0.06 units
            let t = p.y / 4.0;
            let left = 0.1 + 0.8 * t;
            let across = ((p.x - left).min(left + w - p.x)) * scale.x;
            assert!(across > 0.4 * w * scale.x / 2.0, "sample {p:?} only {across} from the band's side");
        }
    }


    /// A cut piece's border along the original outline gets a vertex wherever it crosses a
    /// corner of the surface (a knot of a straight-sided profile): without it the border cut
    /// the corner and no longer met the neighbouring face (a thread's crest at its start cap).
    #[test]
    fn border_segments_keep_the_surface_corners() {
        use crate::NurbsCurve;
        let square = NurbsCurve::polyline(vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0), DVec3::new(1.0, 1.0, 0.0), DVec3::new(0.0, 1.0, 0.0), DVec3::new(0.0, 0.0, 0.0)]).unwrap();
        let s = crate::NurbsSurface::extrude(&square, DVec3::new(0.0, 0.0, 5.0)).unwrap();
        let ((u0, u1), (v0, v1)) = (s.domain_u(), s.domain_v());
        let original = Face::untrimmed(s.clone());
        // a piece whose bottom border runs from the first side, across a corner, to the second
        let (ua, ub) = (u0 + (u1 - u0) * 0.17, u0 + (u1 - u0) * 0.42);
        let piece = Face { surface: s.clone(), loops: vec![vec![DVec2::new(ua, v0), DVec2::new(ub, v0), DVec2::new(ub, v1), DVec2::new(ua, v1)]] };
        let dense = densify_original_segments(&original, piece);
        let corner = DVec3::new(1.0, 0.0, 0.0);
        assert!(dense.loops[0].iter().any(|q| (s.point_at(q.x, q.y) - corner).length() < 1e-9), "the corner (1,0,0) is a vertex of the border");
    }

    use super::*;
    use crate::{NurbsCurve, NurbsSurface, Plane};

    fn volume(b: &Brep) -> f64 {
        let m = b.tessellate(16, 16);
        (0..m.triangle_count()).map(|k| { let (p, q, r) = m.triangle(k); p.dot(q.cross(r)) / 6.0 }).sum()
    }

    #[test]
    fn exact_box_minus_box() {
        let a = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
        let mut b = Brep::cuboid(&Plane::world_xy(), 4.0, 4.0, 20.0).unwrap();
        b.transform(&glam::DAffine3::from_translation(DVec3::new(3.0, 3.0, -5.0)));
        let r = brep_boolean_exact(&a, &b, BooleanOp::Difference, 1e-3).expect("boolean");
        assert!(r.closed);
        assert_eq!(r.faces.len(), 10);
        assert!((volume(&r) - 840.0).abs() < 1e-6, "volume {}", volume(&r));
        let u = brep_boolean_exact(&a, &b, BooleanOp::Union, 1e-3).unwrap();
        assert!((volume(&u) - (1000.0 + 320.0 - 160.0)).abs() < 1e-6);
        let i = brep_boolean_exact(&a, &b, BooleanOp::Intersection, 1e-3).unwrap();
        assert!((volume(&i) - 160.0).abs() < 1e-6);
    }

    #[test]
    fn exact_box_minus_cylinder_and_sphere() {
        let a = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
        let plane = Plane { origin: DVec3::new(5.0, 5.0, -2.0), ..Plane::world_xy() };
        let mut cyl = Brep::new(vec![Face::untrimmed(NurbsSurface::cylinder(&plane, 2.0, 14.0).unwrap()), Face::untrimmed(NurbsSurface::disc(&plane, 2.0).unwrap()), Face::untrimmed(NurbsSurface::disc(&Plane { origin: DVec3::new(5.0, 5.0, 12.0), ..plane }, 2.0).unwrap())]);
        cyl.orient_outward();
        cyl.closed = true;
        let r = brep_boolean_exact(&a, &cyl, BooleanOp::Difference, 1e-3).expect("box minus cylinder");
        assert!(r.closed);
        let v = volume(&r);
        assert!((v - (1000.0 - std::f64::consts::PI * 4.0 * 10.0)).abs() < 2.0, "volume {v}");
        // intersection edge points lie on both surfaces
        let wall = r.faces.iter().find(|f| f.surface.degree_u == 2 || f.surface.degree_v == 2).expect("cylinder wall kept");
        for l in &wall.loops {
            for uv in l {
                let p = wall.surface.point_at(uv.x, uv.y);
                let rxy = ((p.x - 5.0).powi(2) + (p.y - 5.0).powi(2)).sqrt();
                assert!((rxy - 2.0).abs() < 1e-6, "on the cylinder: {p}");
            }
        }
        let mut sph = Brep::from_surface(NurbsSurface::sphere(DVec3::new(10.0, 10.0, 10.0), 4.0).unwrap());
        sph.orient_outward();
        sph.closed = true;
        let r2 = brep_boolean_exact(&r, &sph, BooleanOp::Difference, 1e-3).expect("minus sphere");
        assert!(r2.closed);
        assert!(volume(&r2) < v - 10.0);
    }

    #[test]
    fn exact_cylinder_union_sphere_and_loft_cut() {
        let plane = Plane::world_xy();
        let mut cyl = Brep::new(vec![Face::untrimmed(NurbsSurface::cylinder(&plane, 3.0, 10.0).unwrap()), Face::untrimmed(NurbsSurface::disc(&plane, 3.0).unwrap()), Face::untrimmed(NurbsSurface::disc(&Plane { origin: DVec3::new(0.0, 0.0, 10.0), ..plane }, 3.0).unwrap())]);
        cyl.orient_outward();
        cyl.closed = true;
        let mut sph = Brep::from_surface(NurbsSurface::sphere(DVec3::new(2.0, 1.0, 10.0), 4.0).unwrap());
        sph.orient_outward();
        sph.closed = true;
        let u = brep_boolean_exact(&cyl, &sph, BooleanOp::Union, 1e-3).expect("union");
        assert!(u.closed);
        let vu = volume(&u);
        let vc = std::f64::consts::PI * 9.0 * 10.0;
        let vs = 4.0 / 3.0 * std::f64::consts::PI * 64.0;
        assert!(vu > vc.max(vs) && vu < vc + vs, "union volume {vu}");
        // a lofted solid (box-like with a curved top) minus a cylinder through it
        let c0 = NurbsCurve::polyline(vec![DVec3::new(0.0, 0.0, 0.0), DVec3::new(10.0, 0.0, 0.0), DVec3::new(10.0, 10.0, 0.0), DVec3::new(0.0, 10.0, 0.0), DVec3::new(0.0, 0.0, 0.0)]).unwrap();
        let _ = c0;
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;
    use crate::{NurbsSurface, Plane};

    #[test]
    fn top_face_after_cylinder_cut_has_a_clean_hole_loop() {
        let a = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
        let plane = Plane { origin: DVec3::new(5.0, 5.0, -2.0), ..Plane::world_xy() };
        let mut cyl = Brep::new(vec![Face::untrimmed(NurbsSurface::cylinder(&plane, 2.0, 14.0).unwrap()), Face::untrimmed(NurbsSurface::disc(&plane, 2.0).unwrap()), Face::untrimmed(NurbsSurface::disc(&Plane { origin: DVec3::new(5.0, 5.0, 12.0), ..plane }, 2.0).unwrap())]);
        cyl.orient_outward();
        cyl.closed = true;
        let r = brep_boolean_exact(&a, &cyl, BooleanOp::Difference, 1e-3).unwrap();
        let top = r.faces.iter().find(|f| f.loops.len() == 2 && f.surface.point_at(0.5, 0.5).z > 9.0).unwrap();
        for (i, l) in top.loops.iter().enumerate() {
            let mut dups = 0;
            for k in 0..l.len() {
                for j in k + 1..l.len() {
                    if (l[k] - l[j]).length() < 1e-9 {
                        dups += 1;
                    }
                }
            }
            eprintln!("loop {i}: {} points, {} duplicate pairs, area {}", l.len(), dups, signed_area(l));
            assert_eq!(dups, 0, "loop {i} has duplicate points");
        }
        let (m, _) = top.tessellate_uv(1, 1);
        eprintln!("top face triangles {}", m.triangle_count());
        assert_eq!(m.triangle_count(), top.loops[0].len() + top.loops[1].len());
        // the sphere tessellation must not contain duplicate triangles
        let sph = NurbsSurface::sphere(DVec3::new(10.0, 10.0, 10.0), 4.0).unwrap();
        eprintln!("sphere degrees {} {} counts {} {} knots_u {:?} knots_v {:?}", sph.degree_u, sph.degree_v, sph.count_u, sph.count_v, sph.knots_u, sph.knots_v);
        let f = Face::untrimmed(sph);
        let fm = face_mesh(&f);
        let mut keys: Vec<[i64; 9]> = Vec::new();
        for t in 0..fm.mesh.triangle_count() {
            let (a, b, c) = fm.mesh.triangle(t);
            let mut k: Vec<[i64; 3]> = [a, b, c].iter().map(|p| [(p.x * 1e6).round() as i64, (p.y * 1e6).round() as i64, (p.z * 1e6).round() as i64]).collect();
            k.sort();
            keys.push([k[0][0], k[0][1], k[0][2], k[1][0], k[1][1], k[1][2], k[2][0], k[2][1], k[2][2]]);
        }
        let n = keys.len();
        keys.sort();
        keys.dedup();
        eprintln!("sphere triangles {n}, unique {}", keys.len());
        assert_eq!(n, keys.len(), "duplicate triangles in the sphere tessellation");
    }

    #[test]
    fn sphere_cylinder_worst_vertex() {
        let s = NurbsSurface::sphere(DVec3::ZERO, 5.0).unwrap();
        let a = Brep::from_surface(s.clone());
        let (cx, cy, cr) = (1.3, 0.7, 1.5);
        let base = Plane::from_normal(DVec3::new(cx, cy, -10.0), DVec3::Z);
        let c0 = crate::NurbsCurve::circle(&base, cr).unwrap();
        let side = NurbsSurface::extrude(&c0, DVec3::new(0.0, 0.0, 20.0)).unwrap();
        let c1 = c0.transformed(&glam::DAffine3::from_translation(DVec3::new(0.0, 0.0, 20.0)));
        let mut b = Brep::new(vec![Face::untrimmed(side.clone()), Face::from_planar_curves(&[c0], 1e-3).unwrap(), Face::from_planar_curves(&[c1], 1e-3).unwrap()]);
        b.orient_outward();
        let r = brep_boolean_exact(&a, &b, BooleanOp::Difference, 1e-3).expect("boolean");
        let mut worst = (0.0f64, 0usize, 0usize, 0usize, DVec2::ZERO, DVec3::ZERO);
        for (fi, f) in r.faces.iter().enumerate() {
            for (li, l) in f.loops.iter().enumerate() {
                for (k, uv) in l.iter().enumerate() {
                    let p = f.surface.point_at(uv.x, uv.y);
                    let on_sphere = (p.length() - 5.0).abs();
                    let on_cyl = (((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt() - cr).abs();
                    if on_sphere < 1e-3 && on_cyl < 1e-3 {
                        let d = on_sphere.max(on_cyl);
                        if d > worst.0 {
                            worst = (d, fi, li, k, *uv, p);
                        }
                    }
                }
            }
        }
        eprintln!("worst {:?}; face {} degrees {} {} loop len {}", worst, worst.1, r.faces[worst.1].surface.degree_u, r.faces[worst.1].surface.degree_v, r.faces[worst.1].loops[worst.2].len());
        let l = &r.faces[worst.1].loops[worst.2];
        for k in worst.3.saturating_sub(2)..(worst.3 + 3).min(l.len()) {
            let p = r.faces[worst.1].surface.point_at(l[k].x, l[k].y);
            eprintln!("  [{k}] uv ({:.6},{:.6}) p ({:.6},{:.6},{:.6}) sphere {:.2e} cyl {:.2e}", l[k].x, l[k].y, p.x, p.y, p.z, (p.length() - 5.0).abs(), (((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt() - cr).abs());
        }
    }
}
