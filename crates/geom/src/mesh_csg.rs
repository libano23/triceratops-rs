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

fn split_polygon(plane: &PlaneEq, polygon: &Polygon, coplanar_front: &mut Vec<Polygon>, coplanar_back: &mut Vec<Polygon>, front: &mut Vec<Polygon>, back: &mut Vec<Polygon>) {
    let mut polygon_type = 0u8;
    let mut types = Vec::with_capacity(polygon.vertices.len());
    for v in &polygon.vertices {
        let t = plane.normal.dot(v.pos) - plane.w;
        let ty = if t < -EPS { BACK } else if t > EPS { FRONT } else { COPLANAR };
        polygon_type |= ty;
        types.push(ty);
    }
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
}

#[derive(Default)]
struct Node {
    plane: Option<PlaneEq>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
    polygons: Vec<Polygon>,
}

impl Node {
    fn new(polygons: Vec<Polygon>) -> Self {
        let mut n = Node::default();
        if !polygons.is_empty() {
            n.build(polygons);
        }
        n
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

    fn clip_polygons(&self, polygons: Vec<Polygon>) -> Vec<Polygon> {
        let Some(plane) = &self.plane else { return polygons };
        let mut front = Vec::new();
        let mut back = Vec::new();
        for p in &polygons {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            split_polygon(plane, p, &mut cf, &mut cb, &mut front, &mut back);
            front.extend(cf);
            back.extend(cb);
        }
        let mut out = match &self.front {
            Some(f) => f.clip_polygons(front),
            None => front,
        };
        match &self.back {
            Some(b) => out.extend(b.clip_polygons(back)),
            None => {}
        }
        out
    }

    fn clip_to(&mut self, bsp: &Node) {
        self.polygons = bsp.clip_polygons(std::mem::take(&mut self.polygons));
        if let Some(f) = &mut self.front {
            f.clip_to(bsp);
        }
        if let Some(b) = &mut self.back {
            b.clip_to(bsp);
        }
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

    fn build(&mut self, polygons: Vec<Polygon>) {
        if polygons.is_empty() {
            return;
        }
        if self.plane.is_none() {
            match polygons.iter().find(|p| !p.degenerate) {
                Some(p) => self.plane = Some(p.plane),
                None => {
                    // only zero-area polygons left: keep them here, they split nothing
                    self.polygons.extend(polygons);
                    return;
                }
            }
        }
        let plane = self.plane.unwrap();
        let mut front = Vec::new();
        let mut back = Vec::new();
        for p in &polygons {
            let (mut cf, mut cb) = (Vec::new(), Vec::new());
            split_polygon(&plane, p, &mut cf, &mut cb, &mut front, &mut back);
            self.polygons.extend(cf);
            self.polygons.extend(cb);
        }
        if !front.is_empty() {
            self.front.get_or_insert_with(|| Box::new(Node::default())).build(front);
        }
        if !back.is_empty() {
            self.back.get_or_insert_with(|| Box::new(Node::default())).build(back);
        }
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
pub fn mesh_boolean_tagged(a: &Mesh, a_uv: &[DVec2], a_tags: &[u32], b: &Mesh, b_uv: &[DVec2], b_tags: &[u32], op: BooleanOp) -> Vec<TaggedPolygon> {
    let polys = with_big_stack(|| {
        let na = Node::new(mesh_to_tagged_polygons(a, a_uv, a_tags, 0));
        let nb = Node::new(mesh_to_tagged_polygons(b, b_uv, b_tags, TAG_B));
        boolean_nodes(na, nb, op)
    });
    polys.into_iter().map(|p| TaggedPolygon { tag: p.tag, normal: p.plane.normal, points: p.vertices.iter().map(|v| (v.pos, v.uv)).collect() }).collect()
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
pub fn mesh_boolean(a: &Mesh, b: &Mesh, op: BooleanOp) -> Mesh {
    with_big_stack(|| mesh_boolean_raw(a, b, op).repair_tjunctions())
}

/// The BSP recursion depth grows with the polygon count (every triangle of a sphere is behind
/// every other), so the Booleans run on a thread with a large stack.
pub fn with_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| {
        std::thread::Builder::new().stack_size(256 << 20).spawn_scoped(s, f).expect("spawn boolean thread").join().expect("boolean thread panicked")
    })
}

fn mesh_boolean_raw(a: &Mesh, b: &Mesh, op: BooleanOp) -> Mesh {
    let na = Node::new(mesh_to_polygons(a));
    let nb = Node::new(mesh_to_polygons(b));
    polygons_to_mesh(&boolean_nodes(na, nb, op))
}

fn boolean_nodes(mut na: Node, mut nb: Node, op: BooleanOp) -> Vec<Polygon> {
    match op {
        BooleanOp::Union => {
            na.clip_to(&nb);
            nb.clip_to(&na);
            nb.invert();
            nb.clip_to(&na);
            nb.invert();
            na.build(nb.all_polygons());
            na.all_polygons()
        }
        BooleanOp::Difference => {
            na.invert();
            na.clip_to(&nb);
            nb.clip_to(&na);
            nb.invert();
            nb.clip_to(&na);
            nb.invert();
            na.build(nb.all_polygons());
            na.invert();
            na.all_polygons()
        }
        BooleanOp::Intersection => {
            na.invert();
            nb.clip_to(&na);
            nb.invert();
            na.clip_to(&nb);
            nb.clip_to(&na);
            na.build(nb.all_polygons());
            na.invert();
            na.all_polygons()
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

    #[test]
    fn cube_booleans_have_expected_volumes() {
        let a = Mesh::cuboid(DVec3::ZERO, DVec3::splat(2.0));
        let b = Mesh::cuboid(DVec3::splat(1.0), DVec3::splat(3.0));
        assert!((volume(&a) - 8.0).abs() < 1e-9);
        let u = mesh_boolean(&a, &b, BooleanOp::Union);
        assert!((volume(&u) - 15.0).abs() < 1e-6, "union {}", volume(&u));
        let d = mesh_boolean(&a, &b, BooleanOp::Difference);
        assert!((volume(&d) - 7.0).abs() < 1e-6, "difference {}", volume(&d));
        let i = mesh_boolean(&a, &b, BooleanOp::Intersection);
        assert!((volume(&i) - 1.0).abs() < 1e-6, "intersection {}", volume(&i));
    }
}
