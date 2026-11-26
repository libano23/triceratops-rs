//! Boolean operations on polysurfaces that keep the NURBS faces: the mesh CSG runs on the
//! face tessellations with per-triangle face tags and per-vertex surface parameters, and the
//! surviving pieces of every face are turned back into trimmed faces whose trim loops are the
//! (u, v) outlines of those pieces. Results are polysurfaces, not meshes.

use std::collections::HashMap;

use glam::{DVec2, DVec3};

use crate::brep::{point_in_polygon, signed_area, Brep, Face};
use crate::mesh_csg::{mesh_boolean_tagged, BooleanOp, TaggedPolygon, TAG_B};
use crate::Mesh;

/// Tessellation density for a face used by the Boolean: planar faces need a single quad,
/// curved faces are sampled by span.
pub fn boolean_segs(f: &Face) -> (usize, usize) {
    let s = &f.surface;
    (crate::brep::boundary_segs(s.degree_u), crate::brep::boundary_segs(s.degree_v))
}

struct Tagged {
    mesh: Mesh,
    uv: Vec<DVec2>,
    tags: Vec<u32>,
}

/// Quantised 3-D point used as a hash key.
type Key3 = (i64, i64, i64);
/// Points to insert into a loop after the vertex with the given index.
type LoopInsert = (usize, Vec<DVec3>);

fn tessellate_brep(b: &Brep) -> Tagged {
    let mut mesh = Mesh::new();
    let mut uv = Vec::new();
    let mut tags = Vec::new();
    for (fi, f) in b.faces.iter().enumerate() {
        let (su, sv) = boolean_segs(f);
        let (m, fuv) = f.tessellate_uv(su, sv);
        let before = mesh.triangle_count();
        mesh.append(&m);
        uv.extend(fuv);
        tags.extend(std::iter::repeat_n(fi as u32, mesh.triangle_count() - before));
    }
    if mesh.normals.len() != mesh.vertices.len() {
        mesh.compute_normals();
    }
    Tagged { mesh, uv, tags }
}

/// Boolean of two closed polysurfaces with outward normals. `None` when a face outline could
/// not be rebuilt (callers fall back to the mesh Boolean).
pub fn brep_boolean(a: &Brep, b: &Brep, op: BooleanOp) -> Option<Brep> {
    brep_boolean_verbose(a, b, op, DEFAULT_TOLERANCE).ok()
}

/// Absolute tolerance (model units) to which intersection edges are refined.
pub const DEFAULT_TOLERANCE: f64 = 1e-3;

/// Boolean refined to a given absolute tolerance.
pub fn brep_boolean_tol(a: &Brep, b: &Brep, op: BooleanOp, tol: f64) -> Option<Brep> {
    brep_boolean_verbose(a, b, op, tol).ok()
}

/// Like `brep_boolean` but says why a result could not be built.
pub fn brep_boolean_verbose(a: &Brep, b: &Brep, op: BooleanOp, tol: f64) -> Result<Brep, String> {
    // exact surface/surface intersection first; the mesh-CSG rebuild is the fallback
    let exact_err = match crate::brep_boolean_exact::brep_boolean_exact(a, b, op, tol) {
        Ok(r) => return Ok(r),
        Err(e) => e,
    };
    crate::job::check().map_err(|e| e.to_string())?;
    if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
        eprintln!("exact boolean failed ({exact_err}); falling back to the mesh-CSG rebuild");
    }
    let ta = tessellate_brep(a);
    let tb = tessellate_brep(b);
    if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
        for (name, t) in [("A", &ta), ("B", &tb)] {
            let vol: f64 = (0..t.mesh.triangle_count()).map(|k| { let (p, q, r) = t.mesh.triangle(k); p.dot(q.cross(r)) / 6.0 }).sum();
            let welded = t.mesh.repair_tjunctions();
            eprintln!("operand {name}: {} triangles, closed {} (after weld {}), signed volume {:.3}", t.mesh.triangle_count(), t.mesh.is_closed(), welded.is_closed(), vol);
        }
    }
    let polys = mesh_boolean_tagged(&ta.mesh, &ta.uv, &ta.tags, &tb.mesh, &tb.uv, &tb.tags, op).map_err(|e| e.to_string())?;
    if polys.is_empty() {
        return Err("empty result".into());
    }
    let diag = a.bounding_box().union(&b.bounding_box()).diagonal().max(1e-6);
    let mut groups: HashMap<u32, Vec<&TaggedPolygon>> = HashMap::new();
    for p in &polys {
        groups.entry(p.tag).or_default().push(p);
    }
    let mut keys: Vec<u32> = groups.keys().copied().collect();
    keys.sort();
    let mut faces = Vec::new();
    let mut from_b: Vec<bool> = Vec::new();
    crate::job::stage("rebuilding faces");
    for (k, tag) in keys.into_iter().enumerate() {
        crate::job::checkpoint(k as u64).map_err(|e| e.to_string())?;
        let (src, fi) = if tag & TAG_B != 0 { (b, (tag & !TAG_B) as usize) } else { (a, tag as usize) };
        let face = src.faces.get(fi).ok_or_else(|| format!("bad tag {tag}"))?;
        let group = &groups[&tag];
        let label = format!("{}{}", if tag & TAG_B != 0 { "B" } else { "A" }, fi);
        let rebuilt = rebuild_face(face, group, diag, &label).map_err(|e| format!("face {} of {}: {e}", fi, if tag & TAG_B != 0 { "B" } else { "A" }))?;
        if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
            let loops: Vec<String> = rebuilt.iter().map(|f| f.loops.iter().map(|l| l.len().to_string()).collect::<Vec<_>>().join("+")).collect();
            eprintln!("face {} of {}: {} polygons -> {} faces (loop points {})", fi, if tag & TAG_B != 0 { "B" } else { "A" }, group.len(), rebuilt.len(), loops.join(" | "));
        }
        from_b.extend(std::iter::repeat_n(tag & TAG_B != 0, rebuilt.len()));
        faces.extend(rebuilt);
    }
    if faces.is_empty() {
        return Err("no faces".into());
    }
    // exact-to-tolerance intersection edges: vertices on both surfaces, segments densified
    refine_intersections(&mut faces, &from_b, a, b, &ta, &tb, tol, diag);
    let brep = Brep::new(faces);
    if !brep.closed {
        let open: Vec<String> = (0..brep.faces.len()).map(|i| format!("{}:{}", i, brep.faces[i].loops.len())).collect();
        return Err(format!("result not closed (faces loops {})", open.join(" ")));
    }
    Ok(brep)
}

/// Rebuild the trimmed faces of one source face from its surviving polygons.
fn rebuild_face(face: &Face, polys: &[&TaggedPolygon], diag: f64, label: &str) -> Result<Vec<Face>, String> {
    let s = &face.surface;
    let (u0, u1) = s.domain_u();
    let (v0, v1) = s.domain_v();
    let du = (u1 - u0).max(1e-12);
    let dv = (v1 - v0).max(1e-12);
    let tol = diag * 2e-6;
    let uv_tol = 1e-6;
    // weld vertices by position and parameters (seam twins stay distinct); nearly coincident
    // vertices are found through the neighbouring cells, not just the same rounded key
    let cell_of = |p: DVec3| ((p.x / tol).floor() as i64, (p.y / tol).floor() as i64, (p.z / tol).floor() as i64);
    let mut cells: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    let mut verts: Vec<(DVec3, DVec2)> = Vec::new();
    let mut polygons: Vec<Vec<usize>> = Vec::new();
    let mut flipped_votes = 0i32;
    let same_uv = |a: DVec2, b: DVec2| ((a.x - b.x) / du).abs() < uv_tol * 4.0 && ((a.y - b.y) / dv).abs() < uv_tol * 4.0;
    for p in polys {
        let mut idx: Vec<usize> = Vec::with_capacity(p.points.len());
        for (pos, uv) in &p.points {
            let c = cell_of(*pos);
            let mut found = None;
            'cells: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(list) = cells.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) {
                            for &k in list {
                                if (verts[k].0 - *pos).length() <= tol && same_uv(verts[k].1, *uv) {
                                    found = Some(k);
                                    break 'cells;
                                }
                            }
                        }
                    }
                }
            }
            let id = match found {
                Some(k) => k,
                None => {
                    verts.push((*pos, *uv));
                    cells.entry(c).or_default().push(verts.len() - 1);
                    verts.len() - 1
                }
            };
            if idx.last() != Some(&id) {
                idx.push(id);
            }
        }
        while idx.len() > 1 && idx[0] == idx[idx.len() - 1] {
            idx.pop();
        }
        if idx.len() < 3 {
            continue;
        }
        // orientation vote: polygon normal against the surface normal at its centre
        let c = p.points.iter().fold(DVec2::ZERO, |acc, (_, uv)| acc + *uv) / p.points.len() as f64;
        let n = s.normal_at(c.x.clamp(u0, u1), c.y.clamp(v0, v1));
        flipped_votes += if p.normal.dot(n) < 0.0 { 1 } else { -1 };
        polygons.push(idx);
    }
    if polygons.is_empty() {
        return Ok(Vec::new());
    }
    let flipped = flipped_votes > 0;
    // T-junctions: insert vertices lying on polygon edges. The test runs in (u, v) space, where
    // both the tessellation midpoints and the CSG split points are exactly on the segments.
    let nuv = |uv: DVec2| DVec2::new((uv.x - u0) / du, (uv.y - v0) / dv);
    let uv_eps = 1e-6;
    let on_segment = |a: &(DVec3, DVec2), b: &(DVec3, DVec2), p: &(DVec3, DVec2)| -> Option<f64> {
        let (pa, pb, pp) = (nuv(a.1), nuv(b.1), nuv(p.1));
        let d = pb - pa;
        let len2 = d.length_squared();
        if len2 < 1e-24 {
            return None;
        }
        let t = (pp - pa).dot(d) / len2;
        if t <= 1e-6 || t >= 1.0 - 1e-6 {
            return None;
        }
        if (pa + d * t - pp).length() > uv_eps {
            return None;
        }
        Some(t)
    };
    let cell = 1e-3;
    let bucket = |uv: DVec2| ((uv.x / cell).floor() as i64, (uv.y / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, v) in verts.iter().enumerate() {
        grid.entry(bucket(nuv(v.1))).or_default().push(i);
    }
    let mut fixed: Vec<Vec<usize>> = Vec::with_capacity(polygons.len());
    for poly in &polygons {
        let mut out = Vec::with_capacity(poly.len());
        for k in 0..poly.len() {
            let (i, j) = (poly[k], poly[(k + 1) % poly.len()]);
            out.push(i);
            let (a, b) = (&verts[i], &verts[j]);
            let (pa, pb) = (nuv(a.1), nuv(b.1));
            let seg = pb - pa;
            let steps = ((seg.length() / cell).ceil() as usize).max(1);
            let mut inserts: Vec<(f64, usize)> = Vec::new();
            let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
            for s in 0..=steps {
                let c0 = bucket(pa + seg * (s as f64 / steps as f64));
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        if let Some(list) = grid.get(&(c0.0 + dx, c0.1 + dy)) {
                            for &c in list {
                                if c == i || c == j || !seen.insert(c) {
                                    continue;
                                }
                                if let Some(t) = on_segment(a, b, &verts[c]) {
                                    inserts.push((t, c));
                                }
                            }
                        }
                    }
                }
            }
            inserts.sort_by(|p, q| p.0.partial_cmp(&q.0).unwrap());
            inserts.dedup_by_key(|x| x.1);
            for (_, c) in inserts {
                if out.last() != Some(&c) {
                    out.push(c);
                }
            }
        }
        fixed.push(out);
    }
    // boundary = directed edges without a partner
    let mut count: HashMap<(usize, usize), i32> = HashMap::new();
    for poly in &fixed {
        for k in 0..poly.len() {
            let (i, j) = (poly[k], poly[(k + 1) % poly.len()]);
            if i == j {
                continue;
            }
            *count.entry((i, j)).or_insert(0) += 1;
        }
    }
    let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
    for (&(i, j), &c) in &count {
        let back = count.get(&(j, i)).copied().unwrap_or(0);
        for _ in 0..(c - back).max(0) {
            next.entry(i).or_default().push(j);
        }
    }
    if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
        let boundary: usize = next.values().map(|v| v.len()).sum();
        eprintln!("   [{label}] verts {} polygons {} boundary edges {}", verts.len(), fixed.len(), boundary);
        for poly in fixed.iter().take(8) {
            let desc: Vec<String> = poly.iter().map(|&i| format!("{}=({:.4},{:.4})", i, verts[i].1.x, verts[i].1.y)).collect();
            eprintln!("   [{label}] poly-uv {}", desc.join(" "));
        }
        {
            let mut hist: HashMap<(i32, i32), usize> = HashMap::new();
            for (&(i, j), &c) in &count {
                let back = count.get(&(j, i)).copied().unwrap_or(0);
                *hist.entry((c, back)).or_insert(0) += 1;
            }
            eprintln!("   [{label}] edge count histogram (fwd,back)->n: {:?}", hist);
            let mut shown = 0;
            for (&i, list) in next.iter() {
                for &j in list {
                    let (a, bq) = (verts[i].1, verts[j].1);
                    if ((a.x - bq.x) / du).abs() > 1e-9 && ((a.y - bq.y) / dv).abs() > 1e-9 && shown < 10 {
                        eprintln!("   [{label}] oblique boundary {}->{} uv ({:.4},{:.4})->({:.4},{:.4})", i, j, a.x, a.y, bq.x, bq.y);
                        shown += 1;
                    }
                }
            }
        }
        let inserted: usize = fixed.iter().map(|p| p.len()).sum::<usize>() - polygons.iter().map(|p| p.len()).sum::<usize>();
        let area: f64 = polys.iter().map(|p| { let mut n = DVec3::ZERO; for k in 1..p.points.len() - 1 { n += (p.points[k].0 - p.points[0].0).cross(p.points[k + 1].0 - p.points[0].0); } n.length() * 0.5 }).sum();
        let bb = face.bounding_box();
        eprintln!("   [{label}] inserted T-vertices {inserted}; polygon area {area:.3}; face bbox diag {:.3}", bb.diagonal());
        let mut uv_pairs: Vec<(usize, usize)> = Vec::new();
        for i in 0..verts.len() {
            for j in i + 1..verts.len() {
                if (verts[i].0 - verts[j].0).length() < tol * 50.0 {
                    uv_pairs.push((i, j));
                }
            }
        }
        for (i, j) in uv_pairs.iter().take(6) {
            eprintln!("   [{label}] near-duplicate verts {} {}: pos ({:.4},{:.4},{:.4}) uv ({:.6},{:.6}) vs ({:.6},{:.6})", i, j, verts[*i].0.x, verts[*i].0.y, verts[*i].0.z, verts[*i].1.x, verts[*i].1.y, verts[*j].1.x, verts[*j].1.y);
        }
        let pick = next.iter().flat_map(|(&i, v)| v.iter().map(move |&j| (i, j))).find(|&(i, j)| (verts[i].0.z - verts[j].0.z).abs() > 1e-6 && verts[i].0.z > 1e-6 && verts[j].0.z > 1e-6 && verts[i].0.z < 10.0 - 1e-6 && verts[j].0.z < 10.0 - 1e-6);
        if let Some((i0, j0)) = pick {
            for (pi, poly) in fixed.iter().enumerate() {
                if poly.contains(&i0) || poly.contains(&j0) {
                    let desc: Vec<String> = poly.iter().map(|&i| format!("{}=({:.3},{:.3},{:.3})", i, verts[i].0.x, verts[i].0.y, verts[i].0.z)).collect();
                    eprintln!("   [{label}] around {}->{}: poly {} : {}", i0, j0, pi, desc.join(" "));
                }
            }
        }
        let mut shown = 0;
        for (&i, list) in next.iter() {
            for &j in list {
                if shown < 60 {
                    let (a, bb) = (verts[i], verts[j]);
                    eprintln!("   [{label}] boundary {}->{} ({:.3},{:.3},{:.3})->({:.3},{:.3},{:.3}) uv ({:.4},{:.4})->({:.4},{:.4}) counts {}/{}", i, j, a.0.x, a.0.y, a.0.z, bb.0.x, bb.0.y, bb.0.z, a.1.x, a.1.y, bb.1.x, bb.1.y, count.get(&(i, j)).copied().unwrap_or(0), count.get(&(j, i)).copied().unwrap_or(0));
                    shown += 1;
                }
            }
        }
    }
    let mut loops_uv: Vec<Vec<DVec2>> = Vec::new();
    let nuv_walk = |i: usize| DVec2::new((verts[i].1.x - u0) / du, (verts[i].1.y - v0) / dv);
    // deterministic start (HashMap order is random): the lowest vertex with an unused edge
    while let Some(start) = next.iter().filter(|(_, v)| !v.is_empty()).map(|(k, _)| *k).min() {
        let mut chain = vec![start];
        let mut cur = start;
        let mut prev: Option<usize> = None;
        let mut ok = false;
        for _ in 0..verts.len() * 4 {
            let Some(list) = next.get_mut(&cur) else { break };
            if list.is_empty() {
                break;
            }
            list.sort_unstable();
            // at a junction continue with the face on the left: the outgoing edge that is the
            // first one clockwise from the reversed incoming direction
            let pick = if list.len() == 1 {
                0
            } else if let Some(p) = prev {
                let back = nuv_walk(p) - nuv_walk(cur);
                let base = back.y.atan2(back.x);
                let mut best = (f64::INFINITY, 0usize);
                for (idx, &n) in list.iter().enumerate() {
                    let d = nuv_walk(n) - nuv_walk(cur);
                    let ang = d.y.atan2(d.x);
                    // clockwise angle from `back` to `d`
                    let mut cw = base - ang;
                    while cw <= 1e-12 {
                        cw += std::f64::consts::TAU;
                    }
                    if cw < best.0 {
                        best = (cw, idx);
                    }
                }
                best.1
            } else {
                0
            };
            let n = list.remove(pick);
            if n == start {
                ok = true;
                break;
            }
            chain.push(n);
            prev = Some(cur);
            cur = n;
        }
        if !ok {
            // open chain: the outline could not be rebuilt
            if chain.len() > 1 {
                let p0 = verts[chain[0]].0;
                let p1 = verts[*chain.last().unwrap()].0;
                return Err(format!("open outline chain of {} points from ({:.3},{:.3},{:.3}) to ({:.3},{:.3},{:.3})", chain.len(), p0.x, p0.y, p0.z, p1.x, p1.y, p1.z));
            }
            continue;
        }
        let mut uv: Vec<DVec2> = chain.iter().map(|&i| verts[i].1).collect();
        let planar = s.degree_u == 1 && s.degree_v == 1 && s.count_u == 2 && s.count_v == 2;
        if planar {
            simplify_loop(&mut uv, du, dv);
        } else {
            uv.dedup_by(|a, b| (*a - *b).length() < 1e-12);
            if uv.len() > 1 && (uv[0] - uv[uv.len() - 1]).length() < 1e-12 {
                uv.pop();
            }
        }
        if uv.len() >= 3 && signed_area(&uv).abs() > 1e-10 * du * dv {
            loops_uv.push(uv);
        }
    }
    if std::env::var("TRICERATOPS_BOOL_DEBUG").is_ok() {
        for l in &loops_uv {
            let s: Vec<String> = l.iter().take(6).map(|p| format!("({:.4},{:.4})", p.x, p.y)).collect();
            eprintln!("   [{label}] loop {} pts: {} ...", l.len(), s.join(" "));
        }
        let mut deg: HashMap<usize, (usize, usize)> = HashMap::new();
        for (&(i, j), &c) in &count {
            let back = count.get(&(j, i)).copied().unwrap_or(0);
            if c > back {
                deg.entry(i).or_insert((0, 0)).0 += 1;
                deg.entry(j).or_insert((0, 0)).1 += 1;
            }
        }
        let odd: Vec<String> = deg.iter().filter(|(_, (o, i))| *o != 1 || *i != 1).take(8).map(|(v, (o, i))| format!("{}=({:.4},{:.4}) out {} in {}", v, verts[*v].1.x, verts[*v].1.y, o, i)).collect();
        eprintln!("   [{label}] boundary vertices with degree != 1/1: {}", odd.join("; "));
    }
    if loops_uv.is_empty() {
        return Ok(Vec::new());
    }
    // untouched face: every loop point lies on the original outline
    let original = face.effective_loops();
    let eps = 1e-5 * du.max(dv);
    let same_loop = |l: &Vec<DVec2>, o: &Vec<DVec2>| l.iter().all(|p| dist_to_loop(*p, o) < eps) && o.iter().all(|p| dist_to_loop(*p, l) < eps);
    let untouched = loops_uv.len() == original.len() && loops_uv.iter().all(|l| original.iter().any(|o| same_loop(l, o)));
    if untouched {
        let mut f = face.clone();
        if flipped {
            f.flip();
        }
        return Ok(vec![f]);
    }
    // outer loops are those not contained in another loop; holes go to their container
    let n = loops_uv.len();
    let mut container: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        let probe = loops_uv[i][0];
        let mut best: Option<(usize, f64)> = None;
        for j in 0..n {
            if i == j {
                continue;
            }
            if point_in_polygon(probe, &loops_uv[j]) && dist_to_loop(probe, &loops_uv[j]) > 1e-9 {
                let area = signed_area(&loops_uv[j]).abs();
                if best.is_none_or(|(_, a)| area < a) {
                    best = Some((j, area));
                }
            }
        }
        container[i] = best.map(|(j, _)| j);
    }
    // a loop inside a hole (nested twice) is an outer loop again
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(c) = container[i] {
            d += 1;
            i = c;
            if d > n {
                break;
            }
        }
        d
    };
    let mut out = Vec::new();
    for i in 0..n {
        if depth(i) % 2 != 0 {
            continue;
        }
        let mut loops = vec![loops_uv[i].clone()];
        for j in 0..n {
            if container[j] == Some(i) {
                loops.push(loops_uv[j].clone());
            }
        }
        let mut f = Face { surface: s.clone(), loops };
        if flipped {
            f.flip();
        }
        out.push(f);
    }
    Ok(out)
}

// ------------------------------------------------------------------------------------------------
// Intersection refinement

/// Closest point on a triangle to `p`.
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

/// Closest triangle of a tagged tessellation to a point: (triangle index, closest point, distance).
fn closest_triangle(t: &Tagged, p: DVec3) -> Option<(usize, DVec3, f64)> {
    let m = &t.mesh;
    let mut best: Option<(usize, DVec3, f64)> = None;
    for k in 0..m.triangle_count() {
        let (a, b, c) = m.triangle(k);
        // cheap reject on the triangle bounding box
        let lo = a.min(b).min(c);
        let hi = a.max(b).max(c);
        if let Some((_, _, d)) = best {
            if p.x < lo.x - d || p.x > hi.x + d || p.y < lo.y - d || p.y > hi.y + d || p.z < lo.z - d || p.z > hi.z + d {
                continue;
            }
        }
        let q = closest_on_triangle(p, a, b, c);
        let d = (q - p).length();
        if best.is_none_or(|(_, _, bd)| d < bd) {
            best = Some((k, q, d));
        }
    }
    best
}

/// Surface parameters of a point inside a tagged triangle, from its vertex parameters.
fn triangle_uv(t: &Tagged, k: usize, q: DVec3) -> DVec2 {
    let m = &t.mesh;
    let i = [m.indices[3 * k] as usize, m.indices[3 * k + 1] as usize, m.indices[3 * k + 2] as usize];
    let (a, b, c) = (m.vertices[i[0]], m.vertices[i[1]], m.vertices[i[2]]);
    let v0 = b - a;
    let v1 = c - a;
    let v2 = q - a;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let den = d00 * d11 - d01 * d01;
    let (bv, bw) = if den.abs() < 1e-24 { (0.0, 0.0) } else { ((d11 * d20 - d01 * d21) / den, (d00 * d21 - d01 * d20) / den) };
    let bu = 1.0 - bv - bw;
    t.uv[i[0]] * bu + t.uv[i[1]] * bv + t.uv[i[2]] * bw
}

/// Move a point onto the intersection of two surfaces by alternating closest-point projections.
/// Returns the point and the parameters on both surfaces, or `None` when it does not converge.
fn project_to_intersection(f: &crate::NurbsSurface, g: &crate::NurbsSurface, p: DVec3, uv_f: DVec2, uv_g: DVec2, diag: f64) -> Option<(DVec3, DVec2, DVec2)> {
    let (mut uf, mut vf) = (uv_f.x, uv_f.y);
    let (mut ug, mut vg) = (uv_g.x, uv_g.y);
    let mut p = p;
    let start = p;
    for _ in 0..60 {
        let (nuf, nvf) = f.closest_uv_from(p, uf, vf);
        uf = nuf;
        vf = nvf;
        let pf = f.point_at(uf, vf);
        let (nug, nvg) = g.closest_uv_from(pf, ug, vg);
        ug = nug;
        vg = nvg;
        let pg = g.point_at(ug, vg);
        let gap = (pf - pg).length();
        p = (pf + pg) * 0.5;
        if (p - start).length() > diag * 0.05 {
            return None;
        }
        if gap < diag * 1e-11 {
            return Some((p, DVec2::new(uf, vf), DVec2::new(ug, vg)));
        }
    }
    // accept a slow convergence when the remaining gap is tiny
    let pf = f.point_at(uf, vf);
    let pg = g.point_at(ug, vg);
    if (pf - pg).length() < diag * 1e-7 {
        return Some(((pf + pg) * 0.5, DVec2::new(uf, vf), DVec2::new(ug, vg)));
    }
    None
}

/// Snap every outline vertex that lies on the other solid onto the exact surface/surface
/// intersection and densify the intersection edges until the chords deviate less than `tol`.
fn refine_intersections(faces: &mut [Face], from_b: &[bool], a: &Brep, b: &Brep, ta: &Tagged, tb: &Tagged, tol: f64, diag: f64) {
    let q = diag * 1e-5;
    let key = |p: DVec3| ((p.x / q).round() as i64, (p.y / q).round() as i64, (p.z / q).round() as i64);
    let on_other = diag * 6e-3;
    // refined position per original vertex position
    let mut refined: HashMap<(i64, i64, i64), DVec3> = HashMap::new();
    // pass 1: snap intersection vertices
    for (fi, face) in faces.iter_mut().enumerate() {
        let (other, other_brep) = if from_b[fi] { (ta, a) } else { (tb, b) };
        let surf = face.surface.clone();
        for l in face.loops.iter_mut() {
            for uv in l.iter_mut() {
                let p = surf.point_at(uv.x, uv.y);
                let k = key(p);
                if let Some(rp) = refined.get(&k) {
                    let (u, v) = surf.closest_uv_from(*rp, uv.x, uv.y);
                    *uv = DVec2::new(u, v);
                    continue;
                }
                let Some((tri, qp, d)) = closest_triangle(other, p) else { continue };
                if d > on_other {
                    continue;
                }
                let tag = other.tags[tri] as usize;
                let Some(g) = other_brep.faces.get(tag) else { continue };
                let uv_g = triangle_uv(other, tri, qp);
                if let Some((rp, uv_f, _)) = project_to_intersection(&surf, &g.surface, p, *uv, uv_g, diag) {
                    refined.insert(k, rp);
                    *uv = uv_f;
                } else {
                    refined.insert(k, p);
                }
            }
        }
    }
    if refined.is_empty() {
        return;
    }
    let rkey = |p: DVec3| ((p.x / q).round() as i64, (p.y / q).round() as i64, (p.z / q).round() as i64);
    let refined_keys: std::collections::HashSet<(i64, i64, i64)> = refined.values().map(|p| rkey(*p)).collect();
    // pass 1b: the two sides of an intersection carry different split vertices (A's edges cut by
    // B's facets and vice versa); give every loop the union by copying, for each intersection
    // segment, the vertices the partner loop has between the same two endpoints
    let loop_keys: Vec<Vec<Vec<Key3>>> = faces.iter().map(|f| f.loops.iter().map(|l| l.iter().map(|uv| rkey(f.surface.point_at(uv.x, uv.y))).collect()).collect()).collect();
    let mut where_key: HashMap<Key3, Vec<(usize, usize, usize)>> = HashMap::new();
    for (fi, fl) in loop_keys.iter().enumerate() {
        for (li, l) in fl.iter().enumerate() {
            for (k, key) in l.iter().enumerate() {
                where_key.entry(*key).or_default().push((fi, li, k));
            }
        }
    }
    let mut merge_ins: HashMap<(usize, usize), Vec<LoopInsert>> = HashMap::new();
    for (fi, fl) in loop_keys.iter().enumerate() {
        for (li, l) in fl.iter().enumerate() {
            let n = l.len();
            for k in 0..n {
                let (k0, k1) = (l[k], l[(k + 1) % n]);
                if k0 == k1 || !refined_keys.contains(&k0) || !refined_keys.contains(&k1) {
                    continue;
                }
                let p0 = faces[fi].surface.point_at(faces[fi].loops[li][k].x, faces[fi].loops[li][k].y);
                let p1 = faces[fi].surface.point_at(faces[fi].loops[li][(k + 1) % n].x, faces[fi].loops[li][(k + 1) % n].y);
                let seg_len = (p1 - p0).length();
                let mut best: Option<Vec<DVec3>> = None;
                for &(fj, lj, i0) in where_key.get(&k0).into_iter().flatten() {
                    if fj == fi && lj == li {
                        continue;
                    }
                    let lk = &loop_keys[fj][lj];
                    let m = lk.len();
                    let Some(i1) = lk.iter().position(|kk| *kk == k1) else { continue };
                    // the two cyclic arcs from i0 to i1
                    for dir in [1i64, -1i64] {
                        let mut arc: Vec<usize> = Vec::new();
                        let mut i = i0 as i64;
                        let mut steps = 0;
                        loop {
                            i = (i + dir).rem_euclid(m as i64);
                            steps += 1;
                            if i as usize == i1 || steps > m {
                                break;
                            }
                            arc.push(i as usize);
                        }
                        if steps > m || arc.is_empty() {
                            continue;
                        }
                        // every intermediate vertex must be an intersection vertex near the chord
                        let pts: Vec<DVec3> = arc.iter().map(|&i| faces[fj].surface.point_at(faces[fj].loops[lj][i].x, faces[fj].loops[lj][i].y)).collect();
                        let ok = arc.iter().all(|&i| refined_keys.contains(&lk[i])) && pts.iter().all(|r| {
                            let d = p1 - p0;
                            let t = ((*r - p0).dot(d) / d.length_squared().max(1e-24)).clamp(0.0, 1.0);
                            (p0 + d * t - *r).length() <= 0.5 * seg_len + tol
                        });
                        if ok && best.as_ref().is_none_or(|b| pts.len() < b.len()) {
                            best = Some(pts);
                        }
                    }
                }
                if let Some(pts) = best {
                    merge_ins.entry((fi, li)).or_default().push((k, pts));
                }
            }
        }
    }
    for ((fi, li), mut ins) in merge_ins {
        ins.sort_by_key(|(k, _)| *k);
        let surf = faces[fi].surface.clone();
        let old = faces[fi].loops[li].clone();
        let mut new_loop: Vec<DVec2> = Vec::with_capacity(old.len() + ins.len() * 2);
        let mut j = 0;
        for k in 0..old.len() {
            new_loop.push(old[k]);
            while j < ins.len() && ins[j].0 == k {
                let start = old[k];
                for p in &ins[j].1 {
                    let (u, v) = surf.closest_uv_from(*p, start.x, start.y);
                    if (surf.point_at(u, v) - *p).length() < tol * 4.0 + q {
                        new_loop.push(DVec2::new(u, v));
                    }
                }
                j += 1;
            }
        }
        faces[fi].loops[li] = new_loop;
    }
    // pass 2: densify intersection segments (both endpoints refined) to the tolerance
    // occurrences of each 3-D edge: (face, loop, index of the first vertex, reversed)
    // (face, loop, index of the first vertex, reversed)
    type EdgeUse = (usize, usize, usize, bool);
    let mut edges: HashMap<(Key3, Key3), Vec<EdgeUse>> = HashMap::new();
    for (fi, face) in faces.iter().enumerate() {
        for (li, l) in face.loops.iter().enumerate() {
            for k in 0..l.len() {
                let p0 = face.surface.point_at(l[k].x, l[k].y);
                let p1 = face.surface.point_at(l[(k + 1) % l.len()].x, l[(k + 1) % l.len()].y);
                let (k0, k1) = (rkey(p0), rkey(p1));
                if !refined_keys.contains(&k0) || !refined_keys.contains(&k1) || k0 == k1 {
                    continue;
                }
                let (ek, rev) = if k0 <= k1 { ((k0, k1), false) } else { ((k1, k0), true) };
                edges.entry(ek).or_default().push((fi, li, k, rev));
            }
        }
    }
    // new points per edge, ordered from the smaller key to the larger
    let mut inserts_per_edge: HashMap<(Key3, Key3), Vec<DVec3>> = HashMap::new();
    for (ek, occ) in &edges {
        let Some(&(fi, li, k, rev)) = occ.first() else { continue };
        let face = &faces[fi];
        let l = &face.loops[li];
        let (i0, i1) = if rev { ((k + 1) % l.len(), k) } else { (k, (k + 1) % l.len()) };
        let p0 = face.surface.point_at(l[i0].x, l[i0].y);
        let p1 = face.surface.point_at(l[i1].x, l[i1].y);
        let (other, other_brep) = if from_b[fi] { (ta, a) } else { (tb, b) };
        let mut pts: Vec<DVec3> = Vec::new();
        subdivide_edge(&face.surface, other, other_brep, p0, l[i0], p1, l[i1], tol, diag, 0, &mut pts);
        if !pts.is_empty() {
            inserts_per_edge.insert(*ek, pts);
        }
    }
    if inserts_per_edge.is_empty() {
        return;
    }
    // apply insertions to every occurrence
    let mut per_loop: HashMap<(usize, usize), Vec<LoopInsert>> = HashMap::new();
    for (ek, occ) in &edges {
        let Some(pts) = inserts_per_edge.get(ek) else { continue };
        for &(fi, li, k, rev) in occ {
            let mut ordered = pts.clone();
            if rev {
                ordered.reverse();
            }
            per_loop.entry((fi, li)).or_default().push((k, ordered));
        }
    }
    for ((fi, li), mut ins) in per_loop {
        ins.sort_by_key(|(k, _)| *k);
        let surf = faces[fi].surface.clone();
        let old = faces[fi].loops[li].clone();
        let mut new_loop: Vec<DVec2> = Vec::with_capacity(old.len() + ins.len() * 2);
        let mut j = 0;
        for k in 0..old.len() {
            new_loop.push(old[k]);
            while j < ins.len() && ins[j].0 == k {
                let start = old[k];
                for p in &ins[j].1 {
                    let (u, v) = surf.closest_uv_from(*p, start.x, start.y);
                    new_loop.push(DVec2::new(u, v));
                }
                j += 1;
            }
        }
        faces[fi].loops[li] = new_loop;
    }
}

/// Recursive midpoint refinement of an intersection edge between two refined endpoints.
#[allow(clippy::too_many_arguments)]
fn subdivide_edge(surf: &crate::NurbsSurface, other: &Tagged, other_brep: &Brep, p0: DVec3, uv0: DVec2, p1: DVec3, uv1: DVec2, tol: f64, diag: f64, depth: usize, out: &mut Vec<DVec3>) {
    if depth >= 6 || (p1 - p0).length() < tol * 2.0 {
        return;
    }
    let mid_uv = (uv0 + uv1) * 0.5;
    let mid = surf.point_at(mid_uv.x, mid_uv.y);
    let Some((tri, qp, _)) = closest_triangle(other, mid) else { return };
    let tag = other.tags[tri] as usize;
    let Some(g) = other_brep.faces.get(tag) else { return };
    let uv_g = triangle_uv(other, tri, qp);
    let Some((m, uv_m, _)) = project_to_intersection(surf, &g.surface, mid, mid_uv, uv_g, diag) else { return };
    // deviation of the exact point from the straight chord
    let d = p1 - p0;
    let t = ((m - p0).dot(d) / d.length_squared().max(1e-24)).clamp(0.0, 1.0);
    let dev = (p0 + d * t - m).length();
    if dev <= tol {
        return;
    }
    subdivide_edge(surf, other, other_brep, p0, uv0, m, uv_m, tol, diag, depth + 1, out);
    out.push(m);
    subdivide_edge(surf, other, other_brep, m, uv_m, p1, uv1, tol, diag, depth + 1, out);
}

fn dist_to_loop(p: DVec2, l: &[DVec2]) -> f64 {
    let mut best = f64::INFINITY;
    for k in 0..l.len() {
        let (a, b) = (l[k], l[(k + 1) % l.len()]);
        let d = b - a;
        let t = if d.length_squared() < 1e-24 { 0.0 } else { ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) };
        best = best.min((a + d * t - p).length());
    }
    best
}

/// Drop points that lie on the straight segment between their neighbours (in UV).
fn simplify_loop(l: &mut Vec<DVec2>, du: f64, dv: f64) {
    let scale = DVec2::new(1.0 / du, 1.0 / dv);
    let mut changed = true;
    while changed && l.len() > 3 {
        changed = false;
        let n = l.len();
        let mut keep = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            let (a, b, c) = (l[(i + n - 1) % n] * scale, l[i] * scale, l[(i + 1) % n] * scale);
            let ab = b - a;
            let ac = c - a;
            let cross = (ab.x * ac.y - ab.y * ac.x).abs();
            let on_line = cross < 1e-9 * ac.length().max(1e-12) && ab.dot(ac) > 0.0 && ab.length() < ac.length();
            if on_line && !changed {
                changed = true;
            } else {
                keep.push(l[i]);
            }
            i += 1;
        }
        *l = keep;
    }
    l.dedup_by(|a, b| (*a - *b).length() < 1e-12);
    if l.len() > 1 && (l[0] - l[l.len() - 1]).length() < 1e-12 {
        l.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NurbsSurface, Plane};

    #[test]
    fn box_minus_capped_cylinder() {
        let a = Brep::cuboid(&Plane::world_xy(), 20.0, 20.0, 10.0).unwrap();
        let base = Plane::from_normal(DVec3::new(10.0, 10.0, -5.0), DVec3::Z);
        // built like the Cylinder command: an extruded circle with planar caps from the same curve
        let c0 = crate::NurbsCurve::circle(&base, 4.0).unwrap();
        let side = NurbsSurface::extrude(&c0, DVec3::new(0.0, 0.0, 20.0)).unwrap();
        let c1 = c0.transformed(&glam::DAffine3::from_translation(DVec3::new(0.0, 0.0, 20.0)));
        let cap0 = Face::from_planar_curves(&[c0], 1e-3).unwrap();
        let cap1 = Face::from_planar_curves(&[c1], 1e-3).unwrap();
        let mut b = Brep::new(vec![Face::untrimmed(side), cap0, cap1]);
        b.orient_outward();
        assert!(b.closed, "cylinder operand is closed");
        let r = match brep_boolean_verbose(&a, &b, BooleanOp::Difference, DEFAULT_TOLERANCE) {
            Ok(r) => r,
            Err(e) => panic!("boolean failed: {e}"),
        };
        assert!(r.closed);
        for (i, f) in r.faces.iter().enumerate() {
            let m = f.tessellate(24, 4);
            let area: f64 = (0..m.triangle_count()).map(|t| { let (a, b, c) = m.triangle(t); (b - a).cross(c - a).length() * 0.5 }).sum();
            let vol: f64 = (0..m.triangle_count()).map(|t| { let (a, b, c) = m.triangle(t); a.dot(b.cross(c)) / 6.0 }).sum();
            let (mm, uv) = f.tessellate_uv(24, 4);
            let mut disagree = 0;
            for t in 0..mm.triangle_count() {
                let (a, b, c) = mm.triangle(t);
                let n = (b - a).cross(c - a);
                let idx = [mm.indices[3 * t], mm.indices[3 * t + 1], mm.indices[3 * t + 2]];
                let cu = (uv[idx[0] as usize] + uv[idx[1] as usize] + uv[idx[2] as usize]) / 3.0;
                if n.dot(f.surface.normal_at(cu.x, cu.y)) < 0.0 {
                    disagree += 1;
                }
            }
            eprintln!("face {i}: deg {}x{} loops {:?} tris {} area {area:.3} signed vol {vol:.3} flipped-tris {disagree}", f.surface.degree_u, f.surface.degree_v, f.loops.iter().map(|l| l.len()).collect::<Vec<_>>(), m.triangle_count());
            if f.surface.degree_v == 2 || f.surface.degree_u == 2 {
                let c = DVec3::new(10.0, 10.0, 0.0);
                let radii: Vec<f64> = mm.vertices.iter().map(|p| ((p.x - c.x).powi(2) + (p.y - c.y).powi(2)).sqrt()).collect();
                let (rmin, rmax) = radii.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), r| (a.min(*r), b.max(*r)));
                let (zmin, zmax) = mm.vertices.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.z), b.max(p.z)));
                let mut pn = 0.0;
                for t in 0..mm.triangle_count() {
                    let (a, b, cc) = mm.triangle(t);
                    let n = (b - a).cross(cc - a) * 0.5;
                    pn += ((a + b + cc) / 3.0).dot(n);
                }
                let (lu0, lu1, lv0, lv1) = f.loops[0].iter().fold((f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY), |(a, b, c2, d), p| (a.min(p.x), b.max(p.x), c2.min(p.y), d.max(p.y)));
                let mut longest = 0.0f64;
                let mut long_tris = 0;
                for t in 0..mm.triangle_count() {
                    let (a, b, cc) = mm.triangle(t);
                    let l = (b - a).length().max((cc - b).length()).max((a - cc).length());
                    longest = longest.max(l);
                    if l > 2.0 { long_tris += 1; }
                }
                let (umin, umax, vmin, vmax) = uv.iter().fold((f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY), |(a, b, c2, d), p| (a.min(p.x), b.max(p.x), c2.min(p.y), d.max(p.y)));
                eprintln!("   wall mesh: longest edge {longest:.3}, {long_tris} triangles with an edge > 2; uv range u {umin:.3}..{umax:.3} v {vmin:.3}..{vmax:.3}");
                eprintln!("   wall: {} verts radius {rmin:.3}..{rmax:.3} z {zmin:.3}..{zmax:.3} int(p.n) {pn:.3} loop u {lu0:.3}..{lu1:.3} v {lv0:.3}..{lv1:.3} domain u {:?} v {:?}", mm.vertices.len(), f.surface.domain_u(), f.surface.domain_v());
            }
        }
        let v = volume(&r.tessellate(24, 4));
        let expect = 4000.0 - std::f64::consts::PI * 16.0 * 10.0;
        assert!((v - expect).abs() < expect * 0.02, "volume {v} vs {expect}");
    }

    fn volume(m: &Mesh) -> f64 {
        (0..m.triangle_count()).map(|t| { let (a, b, c) = m.triangle(t); a.dot(b.cross(c)) / 6.0 }).sum()
    }

    #[test]
    fn box_minus_box_is_closed_polysurface() {
        let a = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
        let mut b = Brep::cuboid(&Plane::world_xy(), 4.0, 4.0, 20.0).unwrap();
        b.transform(&glam::DAffine3::from_translation(DVec3::new(3.0, 3.0, -5.0)));
        let r = brep_boolean(&a, &b, BooleanOp::Difference).expect("boolean");
        assert!(r.closed, "result is closed");
        // faces stay planar surfaces: 6 outer + 4 hole walls (top/bottom get a hole loop)
        assert!(r.faces.iter().all(|f| f.surface.degree_u == 1 && f.surface.degree_v == 1));
        assert_eq!(r.faces.len(), 10, "faces: {}", r.faces.len());
        let v = volume(&r.tessellate(4, 4));
        assert!((v - (1000.0 - 160.0)).abs() < 1.0, "volume {v}");
    }

    #[test]
    fn two_holes_in_sequence_stay_closed() {
        let a = Brep::cuboid(&Plane::world_xy(), 20.0, 20.0, 5.0).unwrap();
        let cutter = |x: f64| {
            let base = Plane::from_normal(DVec3::new(x, 5.0, -5.0), DVec3::Z);
            let c0 = crate::NurbsCurve::circle(&base, 2.0).unwrap();
            let side = NurbsSurface::extrude(&c0, DVec3::new(0.0, 0.0, 15.0)).unwrap();
            let c1 = c0.transformed(&glam::DAffine3::from_translation(DVec3::new(0.0, 0.0, 15.0)));
            let mut b = Brep::new(vec![Face::untrimmed(side), Face::from_planar_curves(&[c0], 1e-3).unwrap(), Face::from_planar_curves(&[c1], 1e-3).unwrap()]);
            b.orient_outward();
            b
        };
        let r1 = brep_boolean_verbose(&a, &cutter(5.0), BooleanOp::Difference, DEFAULT_TOLERANCE).unwrap_or_else(|e| panic!("first hole: {e}"));
        let r2 = brep_boolean_verbose(&r1, &cutter(13.0), BooleanOp::Difference, DEFAULT_TOLERANCE).unwrap_or_else(|e| panic!("second hole: {e}"));
        assert!(r2.closed);
        let v = volume(&r2.tessellate(24, 4));
        let expect = 2000.0 - 2.0 * std::f64::consts::PI * 4.0 * 5.0;
        assert!((v - expect).abs() < expect * 0.02, "volume {v} vs {expect}");
    }

    #[test]
    fn app_like_round_holes() {
        // RoundHole on a 20x20x5 box: cutter prism = circle on the top face extruded +-diag
        let a = Brep::cuboid(&Plane::world_xy(), 20.0, 20.0, 5.0).unwrap();
        let diag = a.bounding_box().diagonal();
        let cutter = |x: f64| {
            let plane = Plane::from_normal(DVec3::new(x, 5.0, 5.0), DVec3::Z);
            let c = crate::NurbsCurve::circle(&plane, 2.0).unwrap();
            let dn = plane.z_axis * diag;
            let bottom = c.transformed(&glam::DAffine3::from_translation(-dn));
            let top = c.transformed(&glam::DAffine3::from_translation(dn));
            let side = NurbsSurface::extrude(&bottom, dn * 2.0).unwrap();
            let mut b = Brep::new(vec![Face::untrimmed(side), Face::from_planar_curves(&[bottom], 1e-3).unwrap(), Face::from_planar_curves(&[top], 1e-3).unwrap()]);
            b.orient_outward();
            b.closed = true;
            b
        };
        let r1 = brep_boolean_verbose(&a, &cutter(5.0), BooleanOp::Difference, DEFAULT_TOLERANCE).unwrap_or_else(|e| panic!("first hole: {e}"));
        let r2 = brep_boolean_verbose(&r1, &cutter(13.0), BooleanOp::Difference, DEFAULT_TOLERANCE).unwrap_or_else(|e| panic!("second hole: {e}"));
        assert!(r2.closed);
    }

    #[test]
    fn intersection_edges_are_exact_to_tolerance() {
        // box minus cylinder: every hole-loop vertex lies on the cylinder (radius 4 about (10,10))
        let a = Brep::cuboid(&Plane::world_xy(), 20.0, 20.0, 10.0).unwrap();
        let base = Plane::from_normal(DVec3::new(10.0, 10.0, -5.0), DVec3::Z);
        let c0 = crate::NurbsCurve::circle(&base, 4.0).unwrap();
        let side = NurbsSurface::extrude(&c0, DVec3::new(0.0, 0.0, 20.0)).unwrap();
        let c1 = c0.transformed(&glam::DAffine3::from_translation(DVec3::new(0.0, 0.0, 20.0)));
        let mut b = Brep::new(vec![Face::untrimmed(side), Face::from_planar_curves(&[c0], 1e-3).unwrap(), Face::from_planar_curves(&[c1], 1e-3).unwrap()]);
        b.orient_outward();
        let tol = 1e-4;
        let r = brep_boolean_verbose(&a, &b, BooleanOp::Difference, tol).expect("boolean");
        assert!(r.closed);
        let top = r.faces.iter().find(|f| f.surface.degree_u == 1 && f.loops.len() == 2 && f.surface.point_at(0.5, 0.5).z > 9.0).expect("top face with hole");
        let hole = &top.loops[1];
        assert!(hole.len() > 32, "hole loop densified to the tolerance: {} points", hole.len());
        let mut worst = 0.0f64;
        for uv in hole {
            let p = top.surface.point_at(uv.x, uv.y);
            let rad = ((p.x - 10.0).powi(2) + (p.y - 10.0).powi(2)).sqrt();
            worst = worst.max((rad - 4.0).abs());
        }
        assert!(worst < 1e-6, "hole vertices on the cylinder: worst radius error {worst}");
        // chords within tolerance: midpoints of consecutive vertices stay within tol of the circle
        let mut chord = 0.0f64;
        for k in 0..hole.len() {
            let p = top.surface.point_at(hole[k].x, hole[k].y);
            let q = top.surface.point_at(hole[(k + 1) % hole.len()].x, hole[(k + 1) % hole.len()].y);
            let m = (p + q) * 0.5;
            let rad = ((m.x - 10.0).powi(2) + (m.y - 10.0).powi(2)).sqrt();
            chord = chord.max((rad - 4.0).abs());
        }
        assert!(chord < tol * 1.5, "chord deviation {chord} vs tol {tol}");
        // the volume is now within the chord tolerance of the exact value
        let v = volume(&r.tessellate(24, 4));
        let expect = 4000.0 - std::f64::consts::PI * 16.0 * 10.0;
        assert!((v - expect).abs() < expect * 0.005, "volume {v} vs {expect}");
    }

    #[test]
    fn sphere_cylinder_edge_points_lie_on_both_surfaces() {
        let s = NurbsSurface::sphere(DVec3::ZERO, 5.0).unwrap();
        let a = Brep::from_surface(s.clone());
        // a cylinder through the sphere, away from the poles and the seam crossings
        let (cx, cy, cr) = (1.3, 0.7, 1.5);
        let base = Plane::from_normal(DVec3::new(cx, cy, -10.0), DVec3::Z);
        let c0 = crate::NurbsCurve::circle(&base, cr).unwrap();
        let side = NurbsSurface::extrude(&c0, DVec3::new(0.0, 0.0, 20.0)).unwrap();
        let c1 = c0.transformed(&glam::DAffine3::from_translation(DVec3::new(0.0, 0.0, 20.0)));
        let mut b = Brep::new(vec![Face::untrimmed(side.clone()), Face::from_planar_curves(&[c0], 1e-3).unwrap(), Face::from_planar_curves(&[c1], 1e-3).unwrap()]);
        b.orient_outward();
        let r = brep_boolean_verbose(&a, &b, BooleanOp::Difference, 1e-3).expect("boolean");
        assert!(r.closed);
        let mut worst = 0.0f64;
        let mut checked = 0;
        for f in &r.faces {
            for l in &f.loops {
                for uv in l {
                    let p = f.surface.point_at(uv.x, uv.y);
                    let on_sphere = (p.length() - 5.0).abs();
                    let on_cyl = (((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt() - cr).abs();
                    if on_sphere < 1e-3 && on_cyl < 1e-3 {
                        worst = worst.max(on_sphere.max(on_cyl));
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 30, "intersection vertices found: {checked}");
        assert!(worst < 1e-6, "worst distance to both surfaces {worst}");
    }

    #[test]
    fn box_union_box_and_intersection() {
        let a = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
        let mut b = Brep::cuboid(&Plane::world_xy(), 10.0, 10.0, 10.0).unwrap();
        b.transform(&glam::DAffine3::from_translation(DVec3::new(5.0, 5.0, 5.0)));
        let u = brep_boolean(&a, &b, BooleanOp::Union).expect("union");
        assert!(u.closed);
        let vu = volume(&u.tessellate(4, 4));
        assert!((vu - 1875.0).abs() < 1.0, "union volume {vu}");
        let i = brep_boolean(&a, &b, BooleanOp::Intersection).expect("intersection");
        assert!(i.closed);
        let vi = volume(&i.tessellate(4, 4));
        assert!((vi - 125.0).abs() < 0.5, "intersection volume {vi}");
    }

    #[test]
    fn sphere_minus_box_keeps_sphere_surface() {
        let s = NurbsSurface::sphere(DVec3::ZERO, 5.0).unwrap();
        let a = Brep::from_surface(s);
        let mut b = Brep::cuboid(&Plane::world_xy(), 20.0, 20.0, 20.0).unwrap();
        b.transform(&glam::DAffine3::from_translation(DVec3::new(-10.0, -10.0, 0.0)));
        let r = brep_boolean(&a, &b, BooleanOp::Difference).expect("boolean");
        assert!(r.closed, "half sphere is closed");
        assert!(r.faces.iter().any(|f| f.surface.degree_u == 2 || f.surface.degree_v == 2), "the spherical face survives");
        let v = volume(&r.tessellate(24, 24));
        let half = 0.5 * 4.0 / 3.0 * std::f64::consts::PI * 125.0;
        assert!((v - half).abs() < half * 0.03, "volume {v} vs {half}");
    }
}


