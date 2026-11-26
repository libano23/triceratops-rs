//! Worked definitions for the kinds of models Locust is good at, written in the authoring
//! format an assistant uses (`locust::authoring`), so they double as examples for both.

use super::authoring::prepare;
use super::graph::Graph;

/// (menu name, definition JSON).
pub const EXAMPLES: &[(&str, &str)] = &[
    ("Screw thread", include_str!("thread.json")),
    ("Involute gear", include_str!("spur_gear.json")),
    ("Equation surface", include_str!("equation_surface.json")),
    ("Voronoi", include_str!("voronoi.json")),
    ("Dovetail joint", include_str!("dovetail.json")),
];

/// The example called `name`, ready for the canvas.
pub fn load(name: &str) -> Option<Graph> {
    let (_, src) = EXAMPLES.iter().find(|(n, _)| *n == name)?;
    prepare(&serde_json::from_str(src).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tri_doc::{Document, Geometry};

    use super::*;
    use crate::locust::components::{Ctx, Level};
    use crate::locust::engine::solve;

    /// Solve an example; no node may report an error. Returns what goes to Bake.
    fn solve_example(name: &str) -> Vec<Geometry> {
        let g = load(name).unwrap_or_else(|| panic!("{name} loads"));
        let mut ctx = Ctx::default();
        let t = Instant::now();
        solve(&g, &mut ctx);
        eprintln!("{name}: solved in {:?}", t.elapsed());
        let errors: Vec<String> = g.nodes.iter().flat_map(|n| ctx.diagnostics.get(&n.uid).into_iter().flatten().filter(|d| d.level == Level::Error).map(move |d| format!("{}: {}", n.label(), d.message))).collect();
        assert!(errors.is_empty(), "{name}: {errors:?}");
        ctx.bake.iter().map(|g| g.as_ref().clone()).collect()
    }

    fn volume(g: &Geometry) -> f64 {
        Document::volume_of(g).map(|v| v.0).unwrap_or(0.0)
    }

    #[test]
    fn every_example_loads() {
        for (name, _) in EXAMPLES {
            assert!(load(name).is_some(), "{name}");
        }
    }

    /// Male and female: a closed screw and a closed nut whose thread is the screw's grown
    /// by the clearance. (Sizes are checked on the surfaces: the volume measure under-samples
    /// a loft of this many sections.)
    #[test]
    fn thread_makes_screw_and_nut() {
        let bake = solve_example("Screw thread");
        assert_eq!(bake.len(), 2, "screw and nut");
        for g in &bake {
            assert!(matches!(g, Geometry::Brep(b) if b.closed), "closed solid, got {}", g.type_name());
        }
        let (d, p, l, s, h) = (10.0f64, 1.5f64, 30.0f64, 16.0f64, 8.0f64);
        let (rmaj, rmin) = (d / 2.0, d / 2.0 - 0.5413 * p);
        // the screw is the tall one (bake order follows the solve, not the node list)
        let height = |g: &Geometry| g.bounding_box().max.z - g.bounding_box().min.z;
        let (screw, nut) = if height(&bake[0]) > height(&bake[1]) { (&bake[0], &bake[1]) } else { (&bake[1], &bake[0]) };
        assert!((height(screw) - l).abs() < 1e-6 && (height(nut) - h).abs() < 1e-6);
        // the thread surface swings between the minor and major radius
        let Geometry::Brep(b) = screw else { unreachable!() };
        let side = b.faces.iter().map(|f| &f.surface).max_by(|a, c| a.bounding_box().diagonal().total_cmp(&c.bounding_box().diagonal())).unwrap();
        let ((u0, u1), (v0, v1)) = (side.domain_u(), side.domain_v());
        let radii: Vec<f64> = (0..=40).flat_map(|i| (0..=40).map(move |j| (i, j))).map(|(i, j)| {
            let q = side.point_at(u0 + (u1 - u0) * i as f64 / 40.0, v0 + (v1 - v0) * j as f64 / 40.0);
            glam::DVec2::new(q.x, q.y).length()
        }).collect();
        let (lo, hi) = radii.iter().fold((f64::MAX, f64::MIN), |(a, b), r| (a.min(*r), b.max(*r)));
        assert!(lo > rmin - 0.05 && hi < rmaj + 0.05 && hi - lo > 0.5 * (rmaj - rmin), "radii {lo}..{hi}");
        let corner = s / 2.0 / (std::f64::consts::PI / 6.0).cos();
        assert!((diameter_xy(nut) - 2.0 * corner).abs() < 0.05, "nut across corners {}", diameter_xy(nut));
    }

    fn diameter_xy(g: &Geometry) -> f64 {
        let b = g.bounding_box();
        (b.max.x - b.min.x).max(b.max.y - b.min.y)
    }

    #[test]
    fn spur_gear_has_the_right_size() {
        let bake = solve_example("Involute gear");
        assert_eq!(bake.len(), 1);
        assert!(matches!(&bake[0], Geometry::Brep(b) if b.closed), "closed solid");
        let (m, z) = (2.0f64, 20.0f64);
        assert!((diameter_xy(&bake[0]) - m * (z + 2.0)).abs() < 0.3, "tip diameter {}", diameter_xy(&bake[0]));
        let v = volume(&bake[0]);
        let (rf, ra) = (m * (z - 2.5) / 2.0, m * (z + 2.0) / 2.0);
        let pi = std::f64::consts::PI;
        assert!(v > pi * rf * rf * 10.0 && v < pi * ra * ra * 10.0, "volume {v}");
    }

    #[test]
    fn helical_gear_is_twisted() {
        let mut g = load("Involute gear").unwrap();
        let ha = g.nodes.iter_mut().find(|n| n.nickname.as_deref() == Some("Helix Angle")).unwrap();
        ha.values.insert("Value".into(), serde_json::json!(20.0));
        let mut ctx = Ctx::default();
        solve(&g, &mut ctx);
        assert_eq!(ctx.bake.len(), 1, "{:?}", ctx.diagnostics);
        assert!(matches!(ctx.bake[0].as_ref(), Geometry::Brep(b) if b.closed));
    }

    #[test]
    fn equation_surface_follows_the_formula() {
        let bake = solve_example("Equation surface");
        assert_eq!(bake.len(), 1);
        assert!(matches!(bake[0], Geometry::Surface(_)), "{}", bake[0].type_name());
        let target = glam::DVec3::new(std::f64::consts::FRAC_PI_2, 0.0, 1.0);
        let (_, d) = crate::locust::components::geo::closest_point(&bake[0], target).unwrap();
        assert!(d < 0.02, "z = sin(x)cos(y) at (pi/2, 0) is 1: off by {d}");
    }

    #[test]
    fn torus_knot_tube_is_made() {
        let g = load("Equation surface").unwrap();
        let mut ctx = Ctx::default();
        let out = solve(&g, &mut ctx);
        let knot = g.nodes.iter().find(|n| n.nickname.as_deref() == Some("Torus knot")).unwrap();
        let tube = out[&knot.uid][0].first().and_then(|v| v.as_geometry()).cloned();
        assert!(matches!(tube, Some(Geometry::Surface(_)) | Some(Geometry::Brep(_))), "{:?}", ctx.diagnostics.get(&knot.uid));
    }

    #[test]
    fn voronoi_tiles_and_surface_cells() {
        let bake = solve_example("Voronoi");
        assert_eq!(bake.len(), 40, "one tile per cell");
        assert!(bake.iter().all(|g| matches!(g, Geometry::Brep(b) if b.closed)));
        // every tile is smaller than the panel share it came from: the cells shrank inward
        let total: f64 = bake.iter().map(volume).sum();
        assert!(total < 120.0 * 80.0 * 4.0 * 0.95 && total > 120.0 * 80.0 * 4.0 * 0.5, "tiles volume {total}");
        let g = load("Voronoi").unwrap();
        let mut ctx = Ctx::default();
        let out = solve(&g, &mut ctx);
        let sv = g.nodes.iter().find(|n| n.kind == "Surface Voronoi").unwrap();
        assert_eq!(out[&sv.uid][0].count(), 30, "{:?}", ctx.diagnostics.get(&sv.uid));
    }

    /// The two parts fill the two boards except for the clearance gap, which is the joint's
    /// outline times the clearance: so they cannot overlap. With no clearance they meet exactly.
    #[test]
    fn dovetail_parts_fit_with_clearance() {
        let solve_with_clearance = |c: f64| {
            let mut g = load("Dovetail joint").unwrap();
            let n = g.nodes.iter_mut().find(|n| n.nickname.as_deref() == Some("Clearance")).unwrap();
            n.values.insert("Value".into(), serde_json::json!(c));
            let mut ctx = Ctx::default();
            solve(&g, &mut ctx);
            assert_eq!(ctx.bake.len(), 2, "{:?}", ctx.diagnostics);
            assert!(ctx.bake.iter().all(|b| matches!(b.as_ref(), Geometry::Brep(x) if x.closed)));
            ctx.bake.iter().map(|b| volume(b)).sum::<f64>()
        };
        let boards = 2.0 * 60.0 * 40.0 * 20.0;
        let snug = solve_with_clearance(0.0);
        assert!((snug - boards).abs() < 1e-6 * boards, "no clearance: {snug} vs {boards}");
        let c = 0.2;
        let loose = solve_with_clearance(c);
        // gap ≈ clearance × (two slanted sides + tail end) × thickness
        let (depth, spread) = (18.0f64, 3.0f64);
        let outline = 2.0 * (depth * depth + spread * spread).sqrt() + (14.0 + 2.0 * spread);
        let gap = boards - loose;
        assert!(gap > 0.0, "the parts must not overlap");
        assert!((gap - c * outline * 20.0).abs() < 0.1 * c * outline * 20.0, "gap {gap} vs {}", c * outline * 20.0);
    }
}
