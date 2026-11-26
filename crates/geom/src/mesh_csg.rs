//! Constructive solid geometry on triangle meshes (BSP tree algorithm, after csg.js).
//! Used for Boolean operations between closed polysurfaces; results are meshes.

use glam::{DVec2, DVec3};

use crate::Mesh;

const EPS: f64 = 1e-7;

#[derive(Clone, Debug)]
struct Vertex {
    pos: DVec3,
    normal: DVec3,
    /// Surface parameters of the source face (carried through splits for Brep Booleans).
    uv: DVec2,
}

#[derive(Clone, Copy, Debug)]
struct PlaneEq {
    normal: DVec3,
    w: f64,
}

impl PlaneEq {
    fn from_points(a: DVec3, b: DVec3, c: DVec3) -> Option<Self> {
        let n = (b - a).cross(c - a);
        if n.length_squared() < 1e-24 {
            return None;
        }
        let normal = n.normalize();
        Some(Self { normal, w: normal.dot(a) })
    }
    fn flip(&mut self) {
        self.normal = -self.normal;
        self.w = -self.w;
    }
}

#[derive(Clone, Debug)]
struct Polygon {
    vertices: Vec<Vertex>,
    plane: PlaneEq,
    /// Source tag (face index) kept through all splits.
    tag: u32,
    /// Zero-area polygon (sphere poles, slivers): carried along for its edges but never used
    /// as a splitting plane.
    degenerate: bool,
}

impl Polygon {
    fn new(vertices: Vec<Vertex>) -> Option<Self> {
        Self::tagged(vertices, 0)
    }
    fn tagged(vertices: Vec<Vertex>, tag: u32) -> Option<Self> {
        let plane = PlaneEq::from_points(vertices[0].pos, vertices[1].pos, vertices[2].pos)?;
        Some(Self { vertices, plane, tag, degenerate: false })
    }
    /// Like `tagged` but degenerate polygons (sphere poles, zero-area slivers) keep a given normal
    /// so the edges they own still count when face outlines are rebuilt.
    fn tagged_or_degenerate(vertices: Vec<Vertex>, tag: u32, fallback_normal: DVec3) -> Option<Self> {
        if let Some(plane) = PlaneEq::from_points(vertices[0].pos, vertices[1].pos, vertices[2].pos) {
            return Some(Self { vertices, plane, tag, degenerate: false });
        }
        if fallback_normal.length_squared() < 1e-12 {
            return None;
        }
        // collinear vertices: the plane must contain the line, or the polygon would be
        // "spanning" its own plane and split forever in the BSP
        let mut dir = DVec3::ZERO;
        for i in 0..vertices.len() {
            let d = vertices[(i + 1) % vertices.len()].pos - vertices[i].pos;
            if d.length_squared() > dir.length_squared() {
                dir = d;
            }
        }
        let mut normal = fallback_normal.normalize();
        if dir.length_squared() > 1e-24 {
            let d = dir.normalize();
            normal -= d * normal.dot(d);
            if normal.length_squared() < 1e-12 {
                normal = d.any_orthonormal_vector();
            }
            normal = normal.normalize();
        }
        let w = normal.dot(vertices[0].pos);
        Some(Self { vertices, plane: PlaneEq { normal, w }, tag, degenerate: true })
    }
    fn flip(&mut self) {
        self.vertices.reverse();
        for v in self.vertices.iter_mut() {
            v.normal = -v.normal;
        }
        self.plane.flip();
    }
}

const COPLANAR: u8 = 0;
const FRONT: u8 = 1;
const BACK: u8 = 2;
const SPANNING: u8 = 3;

/// Returns true when the polygon spanned the plane and was cut in two.
fn split_polygon(plane: &PlaneEq, polygon: &Polygon, coplanar_front: &mut Vec<Polygon>, coplanar_back: &mut Vec<Polygon>, front: &mut Vec<Polygon>, back: &mut Vec<Polygon>) -> bool {
    let mut polygon_type = 0u8;
    let mut types = Vec::with_capacity(polygon.vertices.len());
    for v in &polygon.vertices {
        let t = plane.normal.dot(v.pos) - plane.w;
        let ty = if t < -EPS { BACK } else if t > EPS { FRONT } else { COPLANAR };
        polygon_type |= ty;
        types.push(ty);
    }
    let mut spanned = false;
    match polygon_type {
        COPLANAR => {
            if plane.normal.dot(polygon.plane.normal) > 0.0 {
                coplanar_front.push(polygon.clone());
            } else {
                coplanar_back.push(polygon.clone());
            }
        }
        FRONT => front.push(polygon.clone()),
        BACK => back.push(polygon.clone()),
        _ => {
            spanned = true;
            let mut f = Vec::new();
            let mut b = Vec::new();
            let n = polygon.vertices.len();
            for i in 0..n {
                let j = (i + 1) % n;
                let (ti, tj) = (types[i], types[j]);
                let (vi, vj) = (&polygon.vertices[i], &polygon.vertices[j]);
                if ti != BACK {
                    f.push(vi.clone());
                }
                if ti != FRONT {
                    b.push(vi.clone());
                }
                if (ti | tj) == SPANNING {
                    let t = (plane.w - plane.normal.dot(vi.pos)) / plane.normal.dot(vj.pos - vi.pos);
                    let v = Vertex { pos: vi.pos.lerp(vj.pos, t), normal: vi.normal.lerp(vj.normal, t).normalize_or(vi.normal), uv: vi.uv.lerp(vj.uv, t) };
                    f.push(v.clone());
                    b.push(v);
                }
            }
            if f.len() >= 3 {
                if let Some(p) = if polygon.degenerate { Polygon::tagged_or_degenerate(f, polygon.tag, polygon.plane.normal) } else { Polygon::tagged(f, polygon.tag) } {
                    front.push(p);
                }
            }
            if b.len() >= 3 {
                if let Some(p) = if polygon.degenerate { Polygon::tagged_or_degenerate(b, polygon.tag, polygon.plane.normal) } else { Polygon::tagged(b, polygon.tag) } {
                    back.push(p);
                }
            }
        }
    }
    spanned
}

/// Why a mesh Boolean gave no result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BooleanError {
    /// The BSP cut the inputs into more pieces than the budget allows (it grows quadratically
    /// on dense convex meshes and would exhaust memory).
    TooComplex { polygons: usize },
    /// The running job was cancelled.
    Cancelled,
    /// Nothing came out.
    Empty,
    /// The Boolean thread panicked (a geometry case the BSP does not handle).
    Panicked,
}

impl std::fmt::Display for BooleanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BooleanError::TooComplex { polygons } => write!(f, "Boolean too complex ({polygons} polygons) — simplify the inputs or use a coarser mesh"),
            BooleanError::Cancelled => write!(f, "cancelled"),
            BooleanError::Empty => write!(f, "Boolean produced nothing"),
            BooleanError::Panicked => write!(f, "Boolean failed (internal error)"),
        }
    }
}

impl std::error::Error for BooleanError {}

/// How many polygons a Boolean may make before it gives up.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    pub max_polygons: usize,
}

impl Budget {
    /// The budget for inputs of `na` and `nb` triangles; `TRICERATOPS_BOOL_BUDGET` overrides it.
    pub fn for_inputs(na: usize, nb: usize) -> Self {
        if let Some(n) = std::env::var("TRICERATOPS_BOOL_BUDGET").ok().and_then(|v| v.trim().parse().ok()) {
            return Budget { max_polygons: n };
        }
        Budget { max_polygons: (40 * (na + nb)).max(200_000) }
    }
}

/// Counts the pieces the BSP makes (the memory a Boolean takes) and the work it does (to
/// look for cancellation every few thousand polygons).
struct Tracker {
    polygons: usize,
    limit: usize,
    work: u64,
}

impl Tracker {
    fn new(inputs: usize, budget: Budget) -> Self {
        Self { polygons: inputs, limit: budget.max_polygons, work: 0 }
    }

    fn tick(&mut self, handled: usize, splits: usize) -> Result<(), BooleanError> {
        self.polygons += splits;
        if self.polygons > self.limit {
            return Err(BooleanError::TooComplex { polygons: self.polygons });
        }
        let before = self.work >> 12;
        self.work += handled as u64;
        if self.work >> 12 != before {
            crate::job::checkpoint(self.work).map_err(|_| BooleanError::Cancelled)?;
        }
        Ok(())
    }
}

#[derive(Default)]
struct Node {
    plane: Option<PlaneEq>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
    polygons: Vec<Polygon>,
}

impl Node {
    fn new(polygons: Vec<Polygon>, t: &mut Tracker) -> Result<Self, BooleanError> {
        let mut n = Node::default();
        if !polygons.is_empty() {
            n.build(polygons, t)?;
        }
        Ok(n)
    }

    fn invert(&mut self) {
        for p in self.polygons.iter_mut() {
            p.flip();
        }
        if let Some(p) = &mut self.plane {
            p.flip();
        }
        if let Some(f) = &mut self.front {
            f.invert();
        }
        if let Some(b) = &mut self.back {
            b.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    fn clip_polygons(&self, polygons: Vec<Polygon>, t: &mut Tracker) -> Result<Vec<Polygon>, BooleanError> {
        let Some(plane) = &self.plane else { return Ok(polygons) };
        let mut front = Vec::new();
        let mut back = Vec::new();
        let mut splits = 0;
        for p in &polygons {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            splits += split_polygon(plane, p, &mut cf, &mut cb, &mut front, &mut back) as usize;
            front.extend(cf);
            back.extend(cb);
        }
        t.tick(polygons.len(), splits)?;
        drop(polygons);
        let mut out = match &self.front {
            Some(f) => f.clip_polygons(front, t)?,
            None => front,
        };
        if let Some(b) = &self.back {
            out.extend(b.clip_polygons(back, t)?)
        }
        Ok(out)
    }

    fn clip_to(&mut self, bsp: &Node, t: &mut Tracker) -> Result<(), BooleanError> {
        self.polygons = bsp.clip_polygons(std::mem::take(&mut self.polygons), t)?;
        if let Some(f) = &mut self.front {
            f.clip_to(bsp, t)?;
        }
        if let Some(b) = &mut self.back {
            b.clip_to(bsp, t)?;
        }
        Ok(())
    }

    fn all_polygons(&self) -> Vec<Polygon> {
        let mut out = self.polygons.clone();
        if let Some(f) = &self.front {
            out.extend(f.all_polygons());
        }
        if let Some(b) = &self.back {
            out.extend(b.all_polygons());
        }
        out
    }

    fn build(&mut self, polygons: Vec<Polygon>, t: &mut Tracker) -> Result<(), BooleanError> {
        if polygons.is_empty() {
            return Ok(());
        }
        if self.plane.is_none() {
            match polygons.iter().find(|p| !p.degenerate) {
                Some(p) => self.plane = Some(p.plane),
                None => {
                    // only zero-area polygons left: keep them here, they split nothing
                    self.polygons.extend(polygons);
                    return Ok(());
                }
            }
        }
        let plane = self.plane.unwrap();
        let mut front = Vec::new();
        let mut back = Vec::new();
        let mut splits = 0;
        for p in &polygons {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            splits += split_polygon(&plane, p, &mut cf, &mut cb, &mut front, &mut back) as usize;
            self.polygons.extend(cf);
            self.polygons.extend(cb);
        }
        t.tick(polygons.len(), splits)?;
        drop(polygons);
        if !front.is_empty() {
            self.front.get_or_insert_with(|| Box::new(Node::default())).build(front, t)?;
        }
        if !back.is_empty() {
            self.back.get_or_insert_with(|| Box::new(Node::default())).build(back, t)?;
        }
        Ok(())
    }
}

fn mesh_to_polygons(m: &Mesh) -> Vec<Polygon> {
    let mut out = Vec::with_capacity(m.triangle_count());
    for t in 0..m.triangle_count() {
        let idx = [m.indices[3 * t], m.indices[3 * t + 1], m.indices[3 * t + 2]];
        let verts: Vec<Vertex> = idx
            .iter()
            .map(|&i| Vertex { pos: m.vertices[i as usize], normal: m.normals.get(i as usize).copied().unwrap_or(DVec3::Z), uv: DVec2::ZERO })
            .collect();
        if let Some(p) = Polygon::new(verts) {
            out.push(p);
        }
    }
    out
}

fn mesh_to_tagged_polygons(m: &Mesh, uvs: &[DVec2], tags: &[u32], tag_offset: u32) -> Vec<Polygon> {
    let mut out = Vec::with_capacity(m.triangle_count());
    for t in 0..m.triangle_count() {
        let idx = [m.indices[3 * t], m.indices[3 * t + 1], m.indices[3 * t + 2]];
        let verts: Vec<Vertex> = idx
            .iter()
            .map(|&i| Vertex { pos: m.vertices[i as usize], normal: m.normals.get(i as usize).copied().unwrap_or(DVec3::Z), uv: uvs.get(i as usize).copied().unwrap_or(DVec2::ZERO) })
            .collect();
        let fallback = verts.iter().fold(DVec3::ZERO, |a, v| a + v.normal);
        if let Some(p) = Polygon::tagged_or_degenerate(verts, tags.get(t).copied().unwrap_or(0) + tag_offset, fallback) {
            out.push(p);
        }
    }
    out
}

/// A result polygon of a tagged Boolean: source face tag and its vertices with surface parameters.
#[derive(Clone, Debug)]
pub struct TaggedPolygon {
    pub tag: u32,
    pub points: Vec<(DVec3, DVec2)>,
    pub normal: DVec3,
}

/// Tag offset added to the second operand's tags.
pub const TAG_B: u32 = 0x8000_0000;

/// Boolean on two closed meshes whose triangles carry a face tag and per-vertex surface
/// parameters; the result polygons keep their tags so faces can be rebuilt as trimmed surfaces.
pub fn mesh_boolean_tagged(a: &Mesh, a_uv: &[DVec2], a_tags: &[u32], b: &Mesh, b_uv: &[DVec2], b_tags: &[u32], op: BooleanOp) -> Result<Vec<TaggedPolygon>, BooleanError> {
    let budget = Budget::for_inputs(a.triangle_count(), b.triangle_count());
    let polys = on_big_stack(|| {
        let pa = mesh_to_tagged_polygons(a, a_uv, a_tags, 0);
        let pb = mesh_to_tagged_polygons(b, b_uv, b_tags, TAG_B);
        polygons_boolean(pa, pb, op, budget)
    })
    ?;
    Ok(polys.into_iter().map(|p| TaggedPolygon { tag: p.tag, normal: p.plane.normal, points: p.vertices.iter().map(|v| (v.pos, v.uv)).collect() }).collect())
}

fn polygons_to_mesh(polys: &[Polygon]) -> Mesh {
    let mut m = Mesh::new();
    for p in polys {
        let base = m.vertices.len() as u32;
        for v in &p.vertices {
            m.vertices.push(v.pos);
            m.normals.push(v.normal);
        }
        for i in 1..p.vertices.len() - 1 {
            m.indices.extend_from_slice(&[base, base + i as u32, base + i as u32 + 1]);
        }
    }
    m.compute_normals();
    m
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanOp {
    Union,
    Difference,
    Intersection,
}

/// Boolean operation between two closed meshes with outward normals.
pub fn mesh_boolean(a: &Mesh, b: &Mesh, op: BooleanOp) -> Result<Mesh, BooleanError> {
    boolean_budgeted(a, b, op, Budget::for_inputs(a.triangle_count(), b.triangle_count()))
}

/// `mesh_boolean` with an explicit budget.
fn boolean_budgeted(a: &Mesh, b: &Mesh, op: BooleanOp, budget: Budget) -> Result<Mesh, BooleanError> {
    let polys = on_big_stack(|| polygons_boolean(mesh_to_polygons(a), mesh_to_polygons(b), op, budget))?;
    if polys.is_empty() {
        return Err(BooleanError::Empty);
    }
    Ok(polygons_to_mesh(&polys).repair_tjunctions())
}

/// The BSP recursion depth grows with the polygon count (every triangle of a sphere is behind
/// every other), so the Booleans run on a thread with a large stack. `None` when that thread
/// panicked.
pub fn with_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> Option<T> {
    // the caller's job token goes along, so the Boolean can be cancelled and report progress
    let token = crate::job::current();
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn_scoped(s, move || {
                let _guard = token.map(crate::job::install);
                f()
            })
            .expect("spawn boolean thread")
            .join()
            .ok()
    })
}

/// A Boolean on the big-stack thread; a panic there is `Panicked`, not a crash.
fn on_big_stack<T: Send>(f: impl FnOnce() -> Result<T, BooleanError> + Send) -> Result<T, BooleanError> {
    with_big_stack(f).unwrap_or(Err(BooleanError::Panicked))
}

/// Build both trees and combine them; everything (including dropping the deep trees) happens
/// on the caller's thread, which `with_big_stack` provides.
fn polygons_boolean(pa: Vec<Polygon>, pb: Vec<Polygon>, op: BooleanOp, budget: Budget) -> Result<Vec<Polygon>, BooleanError> {
    crate::job::stage("mesh Boolean");
    let mut t = Tracker::new(pa.len() + pb.len(), budget);
    let na = Node::new(pa, &mut t)?;
    let nb = Node::new(pb, &mut t)?;
    boolean_nodes(na, nb, op, &mut t)
}

fn boolean_nodes(mut na: Node, mut nb: Node, op: BooleanOp, t: &mut Tracker) -> Result<Vec<Polygon>, BooleanError> {
    match op {
        BooleanOp::Union => {
            na.clip_to(&nb, t)?;
            nb.clip_to(&na, t)?;
            nb.invert();
            nb.clip_to(&na, t)?;
            nb.invert();
            na.build(nb.all_polygons(), t)?;
            Ok(na.all_polygons())
        }
        BooleanOp::Difference => {
            na.invert();
            na.clip_to(&nb, t)?;
            nb.clip_to(&na, t)?;
            nb.invert();
            nb.clip_to(&na, t)?;
            nb.invert();
            na.build(nb.all_polygons(), t)?;
            na.invert();
            Ok(na.all_polygons())
        }
        BooleanOp::Intersection => {
            na.invert();
            nb.clip_to(&na, t)?;
            nb.invert();
            na.clip_to(&nb, t)?;
            nb.clip_to(&na, t)?;
            na.build(nb.all_polygons(), t)?;
            na.invert();
            Ok(na.all_polygons())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(m: &Mesh) -> f64 {
        let mut v = 0.0;
        for t in 0..m.triangle_count() {
            let (a, b, c) = m.triangle(t);
            v += a.dot(b.cross(c)) / 6.0;
        }
        v
    }

    /// A closed UV sphere with about `2 * seg * seg` triangles.
    fn uv_sphere(c: DVec3, r: f64, seg: usize) -> Mesh {
        let mut m = Mesh::new();
        let rings = seg;
        for i in 0..=rings {
            let th = std::f64::consts::PI * i as f64 / rings as f64;
            for j in 0..seg {
                let ph = std::f64::consts::TAU * j as f64 / seg as f64;
                m.vertices.push(c + r * DVec3::new(th.sin() * ph.cos(), th.sin() * ph.sin(), th.cos()));
            }
        }
        let idx = |i: usize, j: usize| (i * seg + j % seg) as u32;
        for i in 0..rings {
            for j in 0..seg {
                let (a, b, cc, d) = (idx(i, j), idx(i, j + 1), idx(i + 1, j + 1), idx(i + 1, j));
                if i != 0 {
                    m.indices.extend_from_slice(&[a, d, b]);
                }
                if i != rings - 1 {
                    m.indices.extend_from_slice(&[b, d, cc]);
                }
            }
        }
        m.compute_normals();
        m
    }

    #[test]
    fn budget_trips_with_clear_error() {
        let a = uv_sphere(DVec3::ZERO, 10.0, 100);
        let b = uv_sphere(DVec3::new(5.0, 0.0, 0.0), 10.0, 100);
        assert!(a.triangle_count() > 19_000, "{}", a.triangle_count());
        let r = boolean_budgeted(&a, &b, BooleanOp::Union, Budget { max_polygons: 1000 });
        let Err(e) = r else { panic!("expected the budget to trip") };
        assert!(matches!(e, BooleanError::TooComplex { .. }), "{e:?}");
        assert!(e.to_string().starts_with("Boolean too complex ("), "{e}");
    }

    /// The case that used to exhaust 8 GB in 20 s (every recursion level of the depth-20k
    /// chain kept its input polygons alive). Slow (about a minute in release), so run by hand:
    /// `(ulimit -v 4000000; cargo test -p tri-geom --release dense_spheres -- --ignored --nocapture)`.
    #[test]
    #[ignore]
    fn dense_spheres_stop_within_budget() {
        let a = uv_sphere(DVec3::ZERO, 10.0, 100);
        let b = uv_sphere(DVec3::new(5.0, 0.0, 0.0), 10.0, 100);
        let t = std::time::Instant::now();
        let r = mesh_boolean(&a, &b, BooleanOp::Union);
        eprintln!("dense spheres: {:?} in {:?}", r.as_ref().map(|m| m.triangle_count()), t.elapsed());
        assert!(matches!(r, Err(BooleanError::TooComplex { .. }) | Ok(_)), "{r:?}");
    }

    #[test]
    fn cancelled_boolean_returns_quickly() {
        let a = uv_sphere(DVec3::ZERO, 10.0, 100);
        let b = uv_sphere(DVec3::new(5.0, 0.0, 0.0), 10.0, 100);
        let t = crate::job::Token::new();
        let _g = crate::job::install(t.clone());
        let canceller = {
            let t = t.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                t.cancel();
                std::time::Instant::now()
            })
        };
        let r = mesh_boolean(&a, &b, BooleanOp::Union);
        let done = std::time::Instant::now();
        let cancelled_at = canceller.join().unwrap();
        assert_eq!(r.err(), Some(BooleanError::Cancelled));
        assert!(done.duration_since(cancelled_at) < std::time::Duration::from_millis(500), "took {:?} after cancel", done.duration_since(cancelled_at));
    }

    #[test]
    fn a_panic_in_the_boolean_thread_is_an_error() {
        let r: Result<(), BooleanError> = on_big_stack(|| panic!("bad polygon"));
        assert_eq!(r, Err(BooleanError::Panicked));
        assert_eq!(BooleanError::Panicked.to_string(), "Boolean failed (internal error)");
    }

    #[test]
    fn ordinary_boolean_unaffected() {
        let a = Mesh::cuboid(DVec3::ZERO, DVec3::new(20.0, 20.0, 10.0));
        let b = uv_sphere(DVec3::new(10.0, 10.0, 10.0), 6.0, 24);
        let d = mesh_boolean(&a, &b, BooleanOp::Difference).expect("difference");
        assert!(d.repair_tjunctions().is_closed());
        assert_eq!(d.triangle_count(), 2836);
    }

    #[test]
    fn cube_booleans_have_expected_volumes() {
        let a = Mesh::cuboid(DVec3::ZERO, DVec3::splat(2.0));
        let b = Mesh::cuboid(DVec3::splat(1.0), DVec3::splat(3.0));
        assert!((volume(&a) - 8.0).abs() < 1e-9);
        let u = mesh_boolean(&a, &b, BooleanOp::Union).unwrap();
        assert!((volume(&u) - 15.0).abs() < 1e-6, "union {}", volume(&u));
        let d = mesh_boolean(&a, &b, BooleanOp::Difference).unwrap();
        assert!((volume(&d) - 7.0).abs() < 1e-6, "difference {}", volume(&d));
        let i = mesh_boolean(&a, &b, BooleanOp::Intersection).unwrap();
        assert!((volume(&i) - 1.0).abs() < 1e-6, "intersection {}", volume(&i));
    }
}
