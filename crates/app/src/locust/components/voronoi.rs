//! Voronoi cells: in a convex region of a plane, and on a surface (computed in its
//! parameter space, scaled to the surface's size so cells are not stretched).

use glam::{DVec2, DVec3};
use tri_doc::Geometry;
use tri_geom::{NurbsCurve, NurbsSurface, Plane};

use super::populate::{points_on, region};
use super::*;

/// The most sites one Voronoi takes (the cell clipping is quadratic).
pub const MAX_SITES: usize = 5000;

/// Keep the part of `poly` on the side of the line through `m` facing away from `n`.
fn clip(poly: &[DVec2], m: DVec2, n: DVec2) -> Vec<DVec2> {
    let side = |p: DVec2| (p - m).dot(n);
    let mut out = Vec::with_capacity(poly.len() + 1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (da, db) = (side(a), side(b));
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// True when the polygon turns the same way at every corner.
pub fn is_convex(poly: &[DVec2]) -> bool {
    let n = poly.len();
    let mut sign = 0.0;
    for i in 0..n {
        let (a, b, c) = (poly[i], poly[(i + 1) % n], poly[(i + 2) % n]);
        let z = (b - a).perp_dot(c - b);
        if z.abs() < 1e-12 * (b - a).length().max(1.0) * (c - b).length().max(1.0) {
            continue;
        }
        if sign == 0.0 {
            sign = z.signum();
        } else if z.signum() != sign {
            return false;
        }
    }
    true
}

/// The Voronoi cell of each site inside the convex `boundary` (same order as the sites; a
/// site outside the boundary gets an empty cell).
pub fn cells_2d(sites: &[DVec2], boundary: &[DVec2]) -> Vec<Vec<DVec2>> {
    sites
        .iter()
        .enumerate()
        .take_while(|(i, _)| !i.is_multiple_of(16) || tri_geom::job::checkpoint(*i as u64).is_ok())
        .map(|(i, &s)| {
            let mut cell = boundary.to_vec();
            for (j, &o) in sites.iter().enumerate() {
                if i == j || (o - s).length_squared() < 1e-24 || cell.is_empty() {
                    continue;
                }
                cell = clip(&cell, (s + o) * 0.5, o - s);
            }
            cell
        })
        .collect()
}

/// Drop repeated corners (clipping leaves near-duplicates where a bisector meets a corner).
fn tidy(poly: Vec<DVec2>, eps: f64) -> Vec<DVec2> {
    let mut out: Vec<DVec2> = Vec::with_capacity(poly.len());
    for p in poly {
        if out.last().is_none_or(|q| (*q - p).length() > eps) {
            out.push(p);
        }
    }
    while out.len() > 1 && (out[0] - out[out.len() - 1]).length() <= eps {
        out.pop();
    }
    out
}

/// A rough length of the surface along u (at v) by sampling.
fn length_along(s: &NurbsSurface, along_u: bool) -> f64 {
    let ((u0, u1), (v0, v1)) = (s.domain_u(), s.domain_v());
    let mut total = 0.0f64;
    for k in 0..5 {
        let f = (k as f64 + 0.5) / 5.0;
        let mut prev: Option<DVec3> = None;
        for i in 0..=32 {
            let t = i as f64 / 32.0;
            let p = if along_u { s.point_at(u0 + (u1 - u0) * t, v0 + (v1 - v0) * f) } else { s.point_at(u0 + (u1 - u0) * f, v0 + (v1 - v0) * t) };
            if let Some(q) = prev {
                total += (p - q).length();
            }
            prev = Some(p);
        }
    }
    (total / 5.0).max(1e-9)
}

/// A curve on the surface along a polygon in scaled parameter space: each edge is
/// interpolated through points mapped onto the surface (so corners stay sharp), then the
/// edges are joined.
fn curve_on_surface(s: &NurbsSurface, poly: &[DVec2], to_uv: impl Fn(DVec2) -> (f64, f64)) -> Option<NurbsCurve> {
    const STEPS: usize = 16;
    let mut edges = Vec::with_capacity(poly.len());
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let pts: Vec<DVec3> = (0..=STEPS).map(|k| {
            let (u, v) = to_uv(a + (b - a) * (k as f64 / STEPS as f64));
            s.point_at(u, v)
        }).collect();
        edges.push(NurbsCurve::interpolate(&pts, 3).ok()?);
    }
    let mut joined = geo::join_curves(edges);
    (joined.len() == 1).then(|| joined.remove(0))
}

pub fn register(r: &mut Registry) {
    r.tree("Voronoi", "Mesh", vec![tree("Points"), geo("Boundary").opt()], &["Cells"], |ctx, t| {
        let mut pts: Vec<DVec3> = t[0].items().iter().filter_map(|v| match v {
            Value::Point(p) => Some(*p),
            Value::Geometry(g) => match g.as_ref() {
                Geometry::Point(p) => Some(*p),
                _ => None,
            },
            _ => None,
        }).collect();
        if pts.is_empty() {
            return t1(DataTree::empty());
        }
        if pts.len() > MAX_SITES {
            ctx.warn(format!("sites limited to {MAX_SITES}"));
            pts.truncate(MAX_SITES);
        }
        let boundary = t[1].first().and_then(|v| v.as_geometry());
        let (plane, outline) = match boundary {
            Some(Geometry::Curve(c)) => {
                let Some((plane, poly)) = region(Some(&Geometry::Curve(c.clone()))) else {
                    ctx.error("the boundary must be a closed planar curve");
                    return t1(DataTree::empty());
                };
                if !is_convex(&poly) {
                    ctx.error("the boundary must be convex (split a concave one into convex parts)");
                    return t1(DataTree::empty());
                }
                (plane, poly)
            }
            Some(_) => {
                ctx.error("the boundary must be a closed planar curve");
                return t1(DataTree::empty());
            }
            None => {
                // the points' bounding rectangle, grown by a tenth, in their mean height
                let z = pts.iter().map(|p| p.z).sum::<f64>() / pts.len() as f64;
                // world XY axes, so plane coordinates are the points' own x and y
                let plane = Plane { origin: DVec3::new(0.0, 0.0, z), ..Plane::world_xy() };
                let (lo, hi) = pts.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(lo, hi), p| (lo.min(p.truncate()), hi.max(p.truncate())));
                let pad = (hi - lo).max(DVec2::splat(1e-6)) * 0.1;
                let (lo, hi) = (lo - pad, hi + pad);
                (plane, vec![lo, DVec2::new(hi.x, lo.y), hi, DVec2::new(lo.x, hi.y)])
            }
        };
        let sites: Vec<DVec2> = pts.iter().map(|p| {
            let (x, y, _) = plane.closest_parameters(*p);
            DVec2::new(x, y)
        }).collect();
        let eps = outline.iter().fold(0.0f64, |m, p| m.max(p.length())).max(1.0) * 1e-9;
        let cells: Vec<Value> = cells_2d(&sites, &outline)
            .into_iter()
            .map(|c| tidy(c, eps))
            .filter(|c| c.len() >= 3)
            .filter_map(|c| geo::polyline(c.iter().map(|q| plane.point_at(q.x, q.y)).collect(), true))
            .map(gc)
            .collect();
        t1(DataTree::from_list(cells))
    })
    .describe("Voronoi cells of points (as closed polylines, in the points' order) inside a convex closed planar Boundary, or their bounding rectangle grown by a tenth");
    r.tree("Surface Voronoi", "Surface", vec![geo("Surface"), tree("Points").opt(), int("Count", 50), int("Seed", 1)], &["Cells", "Sites"], |ctx, t| {
        let Some(g) = t[0].first().and_then(|v| v.as_geometry()) else { return vec![DataTree::empty(), DataTree::empty()] };
        let Some(s) = geo::first_surface(g) else {
            ctx.error("Surface Voronoi needs a surface");
            return vec![DataTree::empty(), DataTree::empty()];
        };
        let given: Vec<DVec3> = t[1].items().iter().filter_map(|v| match v {
            Value::Point(p) => Some(*p),
            _ => None,
        }).collect();
        let mut pts = if given.is_empty() {
            let count = first_int(&t[2], 50);
            if count <= 0 {
                ctx.warn("count is 0");
            } else if count > MAX_SITES as i64 {
                ctx.warn(format!("count limited to {MAX_SITES}"));
            }
            points_on(&Geometry::Surface(s.clone()), count.clamp(0, MAX_SITES as i64) as usize, first_int(&t[3], 1))
        } else {
            given
        };
        if pts.len() > MAX_SITES {
            ctx.warn(format!("sites limited to {MAX_SITES}"));
            pts.truncate(MAX_SITES);
        }
        let ((u0, u1), (v0, v1)) = (s.domain_u(), s.domain_v());
        let (lu, lv) = (length_along(&s, true), length_along(&s, false));
        let to_scaled = |(u, v): (f64, f64)| DVec2::new((u - u0) / (u1 - u0) * lu, (v - v0) / (v1 - v0) * lv);
        let to_uv = |q: DVec2| (u0 + q.x / lu * (u1 - u0), v0 + q.y / lv * (v1 - v0));
        let sites_uv: Vec<(f64, f64)> = pts.iter().map(|p| s.closest_uv(*p)).collect();
        let sites: Vec<DVec2> = sites_uv.iter().map(|&uv| to_scaled(uv)).collect();
        let rect = vec![DVec2::ZERO, DVec2::new(lu, 0.0), DVec2::new(lu, lv), DVec2::new(0.0, lv)];
        let eps = lu.max(lv) * 1e-9;
        let cells: Vec<Value> = cells_2d(&sites, &rect).into_iter().map(|c| tidy(c, eps)).filter(|c| c.len() >= 3).filter_map(|c| curve_on_surface(&s, &c, to_uv)).map(gc).collect();
        let site_pts: Vec<Value> = sites_uv.iter().map(|&(u, v)| p(s.point_at(u, v))).collect();
        vec![DataTree::from_list(cells), DataTree::from_list(site_pts)]
    })
    .describe("Voronoi cells on a surface, as closed curves on it: around the given Points, or Count random ones (Seed); Sites are the points on the surface");
}
