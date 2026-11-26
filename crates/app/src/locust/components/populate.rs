//! Random points in a region or on geometry, the same for the same seed.

use glam::{DVec2, DVec3};
use tri_doc::Geometry;
use tri_geom::{curve_plane, Plane};

use super::random::SplitMix64;
use super::*;

/// The most points one Populate makes.
pub const MAX_COUNT: i64 = 100_000;

/// The count to make, with the warnings Populate gives about it.
fn checked_count(ctx: &mut Ctx, count: i64) -> usize {
    if count <= 0 {
        ctx.warn("count is 0");
        return 0;
    }
    if count > MAX_COUNT {
        ctx.warn(format!("count limited to {MAX_COUNT}"));
        return MAX_COUNT as usize;
    }
    count as usize
}

/// Even-odd point-in-polygon.
pub fn inside(p: DVec2, poly: &[DVec2]) -> bool {
    let mut inside = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// A closed planar region as its plane and outline in plane coordinates; the 100×100 square
/// on XY when there is none.
pub fn region(g: Option<&Geometry>) -> Option<(Plane, Vec<DVec2>)> {
    let Some(g) = g else {
        let square = vec![DVec2::new(0.0, 0.0), DVec2::new(100.0, 0.0), DVec2::new(100.0, 100.0), DVec2::new(0.0, 100.0)];
        return Some((Plane::world_xy(), square));
    };
    let Geometry::Curve(c) = g else { return None };
    if !c.is_closed() {
        return None;
    }
    let plane = curve_plane(c)?;
    let tol = c.bounding_box().diagonal().max(1e-9) * 1e-4;
    let poly: Vec<DVec2> = c.tessellate_adaptive(tol).into_iter().map(|p| {
        let (x, y, _) = plane.closest_parameters(p);
        DVec2::new(x, y)
    }).collect();
    (poly.len() >= 3).then_some((plane, poly))
}

/// `count` points uniformly inside the outline.
pub fn points_in(poly: &[DVec2], count: usize, seed: i64) -> Vec<DVec2> {
    let (lo, hi) = poly.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let mut rng = SplitMix64::new(seed);
    let mut out = Vec::with_capacity(count);
    let mut tries = 0usize;
    while out.len() < count && tries < count.saturating_mul(1000).max(1000) {
        tries += 1;
        let p = DVec2::new(lo.x + (hi.x - lo.x) * rng.next_f64(), lo.y + (hi.y - lo.y) * rng.next_f64());
        if inside(p, poly) {
            out.push(p);
        }
    }
    out
}

/// `count` points spread over the geometry by area, on its surface.
pub fn points_on(g: &Geometry, count: usize, seed: i64) -> Vec<DVec3> {
    let Some(mesh) = geo::to_mesh(g) else { return Vec::new() };
    let tris: Vec<(DVec3, DVec3, DVec3)> = (0..mesh.triangle_count()).map(|t| mesh.triangle(t)).collect();
    let mut cumulative = Vec::with_capacity(tris.len());
    let mut total = 0.0;
    for (a, b, c) in &tris {
        total += (*b - *a).cross(*c - *a).length() * 0.5;
        cumulative.push(total);
    }
    if total <= 0.0 {
        return Vec::new();
    }
    let mut rng = SplitMix64::new(seed);
    let snap = matches!(g, Geometry::Surface(_) | Geometry::Brep(_));
    (0..count)
        .map(|_| {
            let k = cumulative.partition_point(|&c| c < rng.next_f64() * total).min(tris.len() - 1);
            let (a, b, c) = tris[k];
            let (mut u, mut v) = (rng.next_f64(), rng.next_f64());
            if u + v > 1.0 {
                (u, v) = (1.0 - u, 1.0 - v);
            }
            let p = a + (b - a) * u + (c - a) * v;
            // the mesh is a chordal approximation: put the point on the real surface
            if snap {
                geo::closest_point(g, p).map(|(q, _)| q).unwrap_or(p)
            } else {
                p
            }
        })
        .collect()
}

pub fn register(r: &mut Registry) {
    r.tree("Populate 2D", "Vector", vec![geo("Region").opt(), int("Count", 100), int("Seed", 1)], &["Population"], |ctx, t| {
        let Some((plane, poly)) = region(t[0].first().and_then(|v| v.as_geometry())) else {
            ctx.error("the region must be a closed planar curve");
            return t1(DataTree::empty());
        };
        let count = checked_count(ctx, first_int(&t[1], 100));
        let seed = first_int(&t[2], 1);
        t1(DataTree::from_list(points_in(&poly, count, seed).into_iter().map(|q| p(plane.point_at(q.x, q.y))).collect()))
    })
    .describe("Random points inside a closed planar curve (the 100×100 square on XY when none is given); the same Seed gives the same points");
    r.tree("Populate Geometry", "Vector", vec![geo("Geometry"), int("Count", 100), int("Seed", 1)], &["Population"], |ctx, t| {
        let Some(g) = t[0].first().and_then(|v| v.as_geometry()) else { return t1(DataTree::empty()) };
        let count = checked_count(ctx, first_int(&t[1], 100));
        let pts = points_on(g, count, first_int(&t[2], 1));
        if pts.is_empty() && count > 0 {
            ctx.warn("this geometry has no area to put points on");
        }
        t1(DataTree::from_list(pts.into_iter().map(p).collect()))
    })
    .describe("Random points on surfaces, polysurfaces and meshes, spread evenly by area; the same Seed gives the same points");
}
