//! Surface: primitives, extrusions, lofts, sweeps, patches and the tools that evaluate,
//! divide and edit surfaces and polysurfaces.

use glam::{DAffine3, DVec3};
use tri_doc::Geometry;
use tri_geom::{Brep, Face, NurbsCurve, NurbsSurface, Plane, Projector};

use super::super::data::{as_point, DataTree, Value};
use super::geo::{self, curves};
use super::*;
use crate::commands::{misc4, misc6, surface2, surface_edit, solid_edit, surfaces3};

/// Curves of a list of values; a closed planar polysurface boundary is not needed here.
fn surface_items(items: &[Value]) -> Vec<Geometry> {
    items.iter().filter_map(|v| v.as_geometry()).filter(|g| matches!(g, Geometry::Surface(_) | Geometry::Brep(_))).cloned().collect()
}

pub fn register(r: &mut Registry) {
    r.elem("Plane Surface", "Surface", vec![pt("Origin", 0.0, 0.0, 0.0), num("Width", 10.0), num("Height", 10.0), pt("Normal", 0.0, 0.0, 1.0)], &["Surface"], |_, a| {
        let (w, h) = (a.num(1), a.num(2));
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        let pl = geo::plane(a.pt(0), a.dir(3, DVec3::Z));
        let s = NurbsSurface::from_grid(1, 1, vec![vec![pl.origin, pl.origin + pl.y_axis * h], vec![pl.origin + pl.x_axis * w, pl.origin + pl.x_axis * w + pl.y_axis * h]]).ok()?;
        one(g(Geometry::Surface(s)))
    });
    r.elem("Box", "Surface", vec![pt("Corner", 0.0, 0.0, 0.0), num("X", 10.0), num("Y", 10.0), num("Z", 10.0)], &["Box"], |_, a| og(geo::box_solid(a.pt(0), a.num(1), a.num(2), a.num(3))));
    r.elem("Sphere", "Surface", vec![pt("Center", 0.0, 0.0, 0.0), num("Radius", 5.0)], &["Sphere"], |_, a| og(geo::sphere(a.pt(0), a.num(1))));
    r.elem("Cylinder", "Surface", vec![pt("Base", 0.0, 0.0, 0.0), num("Radius", 5.0), num("Height", 10.0), pt("Axis", 0.0, 0.0, 1.0)], &["Cylinder"], |_, a| og(geo::cylinder(a.pt(0), a.num(1), a.num(2), a.dir(3, DVec3::Z))));
    r.elem("Cone", "Surface", vec![pt("Base", 0.0, 0.0, 0.0), num("Radius 1", 5.0), num("Radius 2", 0.0), num("Height", 10.0)], &["Cone"], |_, a| og(geo::cone(a.pt(0), a.num(1), a.num(2), a.num(3))));
    r.elem("Torus", "Surface", vec![pt("Center", 0.0, 0.0, 0.0), num("Major Radius", 5.0), num("Minor Radius", 1.0), pt("Axis", 0.0, 0.0, 1.0)], &["Torus"], |_, a| og(geo::torus(a.pt(0), a.num(1), a.num(2), a.dir(3, DVec3::Z))));
    r.elem("Extrude", "Surface", vec![geo("Curve"), pt("Direction", 0.0, 0.0, 1.0), num("Distance", 10.0), boolean("Cap", true)], &["Extrusion"], |_, a| {
        let d = a.dir(1, DVec3::Z);
        if d.length() < 1e-12 {
            return None;
        }
        og(geo::extrude(a.geo(0)?, d.normalize() * a.num(2), a.flag(3)))
    });
    r.tree("Loft", "Surface", vec![tree("Curves")], &["Surface"], |_, t| {
        let cs = curves(&t[0].items());
        if cs.len() < 2 {
            return t1(DataTree::empty());
        }
        let deg = 3.min(cs.len() - 1);
        match NurbsSurface::loft(&cs, deg) {
            Ok(s) => t1(DataTree::single(g(Geometry::Surface(s)))),
            Err(_) => t1(DataTree::empty()),
        }
    })
    .describe("A surface through the curves, in order");
    r.elem("Revolve", "Surface", vec![geo("Curve"), pt("Axis Point", 0.0, 0.0, 0.0), pt("Axis", 0.0, 0.0, 1.0), num("Angle", 360.0)], &["Surface"], |_, a| {
        let c = a.curve(0)?;
        let ang = a.num(3).to_radians();
        if ang.abs() < 1e-9 {
            return None;
        }
        let s = NurbsSurface::revolve(&c, a.pt(1), a.dir(2, DVec3::Z).normalize_or(DVec3::Z), ang.clamp(-std::f64::consts::TAU, std::f64::consts::TAU)).ok()?;
        one(g(Geometry::Surface(s)))
    });
    r.elem("Edge Surface", "Surface", vec![geo("Curve A"), geo("Curve B"), geo("Curve C").opt(), geo("Curve D").opt()], &["Surface"], |_, a| {
        let cs: Vec<NurbsCurve> = (0..4).filter_map(|i| a.curve(i)).collect();
        surface2::edge_surface(cs).ok().and_then(|x| one(g(x)))
    });
    r.tree("Patch", "Surface", vec![tree("Curves")], &["Surface"], |_, t| {
        let mut samples = Vec::new();
        let mut plane = None;
        for v in t[0].items() {
            match &v {
                Value::Point(q) => samples.push(*q),
                Value::Geometry(gm) => match gm.as_ref() {
                    Geometry::Curve(c) => {
                        plane = plane.or_else(|| tri_geom::curve_plane(c));
                        samples.extend(c.tessellate_adaptive(c.bounding_box().diagonal().max(1e-6) * 1e-3));
                    }
                    Geometry::Point(q) => samples.push(*q),
                    Geometry::Mesh(m) => samples.extend(m.vertices.iter().copied()),
                    _ => {}
                },
                _ => {}
            }
        }
        if samples.len() < 3 {
            return t1(DataTree::empty());
        }
        match solid_edit::patch_surface(&samples, plane.unwrap_or_else(Plane::world_xy), 10) {
            Ok(s) => t1(DataTree::single(g(Geometry::Surface(s)))),
            Err(_) => t1(DataTree::empty()),
        }
    });
    r.elem("Cap", "Surface", vec![geo("Shape")], &["Solid"], |_, a| {
        let b = a.brep(0)?;
        // look for open planar ends even when the shape says it is closed: a surface closed
        // around (a loft of closed sections) can pass the closed test with its ends open
        let loops = b.naked_planar_loops();
        if loops.is_empty() && b.closed {
            return one(g(Geometry::Brep(b)));
        }
        let mut faces = b.faces.clone();
        let mut added = 0;
        for (pl, pts) in loops {
            if let Ok(f) = Face::planar(&pl, vec![pts]) {
                faces.push(f);
                added += 1;
            }
        }
        if added == 0 {
            return None;
        }
        one(g(geo::solid(faces)))
    });
    r.elem("Shell", "Surface", vec![geo("Solid"), num("Thickness", 1.0)], &["Solid"], |_, a| misc4::shell_geometry(a.geo(0)?, a.num(1), &[]).ok().and_then(|x| one(g(x))));
    r.elem("Sweep1", "Surface", vec![geo("Section"), geo("Rail")], &["Surface"], |_, a| surface2::sweep1_surface(&a.curve(0)?, &a.curve(1)?, false).and_then(|s| one(g(Geometry::Surface(s)))));
    r.elem("Sweep2", "Surface", vec![geo("Section"), geo("Rail A"), geo("Rail B")], &["Surface"], |_, a| surface2::sweep2_surface(&a.curve(0)?, &a.curve(1)?, &a.curve(2)?).and_then(|s| one(g(Geometry::Surface(s)))));
    r.elem("Extrude Along Curve", "Surface", vec![geo("Profile"), geo("Path")], &["Solid"], |_, a| {
        // a closed profile swept along the path and capped into a solid
        let (prof, path) = (a.curve(0)?, a.curve(1)?);
        let side = surface2::sweep1_surface(&prof, &path, false)?;
        if prof.is_closed() && !path.is_closed() {
            let sections = surface2::sweep_sections_pub(&prof, &path, 24);
            if let (Some(c0), Some(c1)) = (sections.first().and_then(geo::cap), sections.last().and_then(geo::cap)) {
                let s = geo::solid(vec![Face::untrimmed(side.clone()), c0, c1]);
                if s.is_solid() {
                    return one(g(s));
                }
            }
        }
        one(g(Geometry::Surface(side)))
    });
    r.elem("Rail Revolve", "Surface", vec![geo("Profile"), geo("Rail"), pt("Axis Point", 0.0, 0.0, 0.0), pt("Axis", 0.0, 0.0, 1.0)], &["Surface"], |_, a| {
        surfaces3::rail_revolve_surface(&a.curve(0)?, &a.curve(1)?, a.pt(2), a.dir(3, DVec3::Z)).and_then(|s| one(g(Geometry::Surface(s))))
    });
    r.elem("Offset Surface", "Surface", vec![geo("Surface"), num("Distance", 1.0)], &["Surface"], |_, a| {
        let fs = a.faces(0)?;
        let out: Vec<Face> = fs.iter().map(|f| surface_edit::OffsetSrfCmd::offset_face(f, a.num(1))).collect();
        og(geo::from_faces(out))
    });
    r.elem("Untrim", "Surface", vec![geo("Surface")], &["Surface"], |_, a| {
        // holes only: the outer boundary stays, the inner loops go
        let fs = a.faces(0)?;
        let out: Vec<Face> = fs.into_iter().map(|mut f| {
            f.loops.truncate(1);
            f
        }).collect();
        og(geo::from_faces(out))
    });
    r.elem("Surface from Corners", "Surface", vec![pt("A", 0.0, 0.0, 0.0), pt("B", 1.0, 0.0, 0.0), pt("C", 1.0, 1.0, 0.0), pt("D", 0.0, 1.0, 0.0)], &["Surface"], |_, a| og(geo::surface_from_corners(a.pt(0), a.pt(1), a.pt(2), a.pt(3))));
    r.tree("Surface from Grid", "Surface", vec![tree("Grid")], &["Surface"], |_, t| {
        let rows: Vec<Vec<DVec3>> = t[0].paths().iter().map(|pth| t[0].branch_ref(pth).iter().map(as_point).collect::<Vec<_>>()).filter(|r| r.len() >= 2).collect();
        if rows.len() < 2 {
            return t1(DataTree::empty());
        }
        let nv = rows.iter().map(|r| r.len()).min().unwrap();
        let grid: Vec<Vec<DVec3>> = rows.iter().map(|r| r[..nv].to_vec()).collect();
        let (du, dv) = (3.min(grid.len() - 1), 3.min(nv - 1));
        match NurbsSurface::interpolate_grid(&grid, du, dv) {
            Ok(s) => t1(DataTree::single(g(Geometry::Surface(s)))),
            Err(_) => t1(DataTree::empty()),
        }
    });
    r.elem("Evaluate Surface", "Surface", vec![geo("Surface"), pt("Point", 0.0, 0.0, 0.0)], &["Point", "Normal"], |_, a| {
        let (s, u, v) = geo::surface_uv(a.geo(0)?, a.pt(1))?;
        outs(vec![Out::One(p(s.point_at(u, v))), Out::One(p(s.normal_at(u, v)))])
    });
    r.elem("Iso Curve", "Surface", vec![geo("Surface"), pt("Point", 0.0, 0.0, 0.0), text("Along", "u")], &["Curve"], |_, a| {
        let (s, u, v) = geo::surface_uv(a.geo(0)?, a.pt(1))?;
        // "u": the curve running along U (constant v), as the Python Locust
        let c = if a.text(2).eq_ignore_ascii_case("v") { s.isocurve_u(u) } else { s.isocurve_v(v) };
        one(gc(c))
    });
    r.tree("Deconstruct Brep", "Surface", vec![geo("Brep")], &["Faces", "Edges"], |_, t| {
        let (mut fs, mut es) = (Vec::new(), Vec::new());
        for item in t[0].items() {
            let Some(gm) = item.as_geometry() else { continue };
            for f in geo::faces(gm).unwrap_or_default() {
                es.extend(geo::edge_curves(&f).into_iter().map(gc));
                fs.push(g(geo::from_faces(vec![f]).unwrap()));
            }
        }
        vec![DataTree::from_list(fs), DataTree::from_list(es)]
    });
    r.tree("Join Brep", "Surface", vec![tree("Shapes")], &["Brep"], |_, t| {
        let items = surface_items(&t[0].items());
        if items.len() < 2 {
            return t1(t[0].clone());
        }
        let faces: Vec<Face> = items.iter().flat_map(|x| geo::faces(x).unwrap_or_default()).collect();
        t1(DataTree::single(g(geo::solid(faces))))
    });
    r.elem("Flip Surface", "Surface", vec![geo("Surface")], &["Surface"], |_, a| {
        let mut gm = a.geo(0)?.clone();
        match &mut gm {
            Geometry::Surface(s) => s.flip(),
            Geometry::Brep(b) => b.faces.iter_mut().for_each(|f| f.flip()),
            Geometry::Mesh(m) => m.flip(),
            _ => return None,
        }
        one(g(gm))
    });
    r.tree("Pull Curve", "Surface", vec![geo("Curve"), geo("Surface")], &["Curve"], |_, t| {
        let Some(target) = t[1].items().iter().find_map(|v| v.as_geometry().and_then(geo::faces)) else { return t1(DataTree::empty()) };
        t1(DataTree::from_list(curves(&t[0].items()).iter().filter_map(|c| misc6::pulled_curve(c, &target)).map(gc).collect()))
    });
    r.tree("Divide Surface", "Surface", vec![geo("Surface"), int("Count U", 5), int("Count V", 5)], &["Points"], |_, t| {
        let nu = first_int(&t[1], 5).max(2) as usize;
        let nv = first_int(&t[2], 5).max(2) as usize;
        let mut out = Vec::new();
        for item in t[0].items() {
            let Some(s) = item.as_geometry().and_then(geo::first_surface) else { continue };
            let ((u0, u1), (v0, v1)) = (s.domain_u(), s.domain_v());
            for j in 0..nv {
                let v = v0 + (v1 - v0) * j as f64 / (nv - 1) as f64;
                for i in 0..nu {
                    let u = u0 + (u1 - u0) * i as f64 / (nu - 1) as f64;
                    out.push(Value::Point(s.point_at(u, v)));
                }
            }
        }
        t1(DataTree::from_list(out))
    });
    r.elem("Surface Closest Point", "Surface", vec![geo("Surface"), pt("Point", 0.0, 0.0, 0.0)], &["Point", "UV"], |_, a| {
        let (s, u, v) = geo::surface_uv(a.geo(0)?, a.pt(1))?;
        outs(vec![Out::One(p(s.point_at(u, v))), Out::One(Value::Pair(u, v))])
    });
    r.elem("Surface Domain", "Surface", vec![geo("Surface")], &["U", "V"], |_, a| {
        let s = a.surface(0)?;
        let ((u0, u1), (v0, v1)) = (s.domain_u(), s.domain_v());
        outs(vec![Out::One(Value::Pair(u0, u1)), Out::One(Value::Pair(v0, v1))])
    });
    // Project lives under Transform but works like the surface tools
    r.tree("Project", "Transform", vec![geo("Geometry"), pt("Direction", 0.0, 0.0, -1.0), geo("Surface")], &["Curve"], |_, t| {
        let dir = t[1].first().map(as_point).unwrap_or(-DVec3::Z);
        let Some(target) = t[2].items().iter().find_map(|v| v.as_geometry().and_then(geo::faces)) else { return t1(DataTree::empty()) };
        let mut out = Vec::new();
        for c in curves(&t[0].items()) {
            out.extend(project_curve(&c, &target, dir).into_iter().map(gc));
        }
        for v in t[0].items() {
            if let Value::Point(q) = v {
                out.extend(project_point(q, &target, dir).into_iter().map(Value::Point));
            }
        }
        t1(DataTree::from_list(out))
    });
    let _ = DAffine3::IDENTITY;
    let _ = Brep::new;
}

/// Exact projection of a curve onto faces along `dir`: one curve per contiguous piece.
pub fn project_curve(c: &NurbsCurve, faces: &[Face], dir: DVec3) -> Vec<NurbsCurve> {
    let mut out = Vec::new();
    let tol = c.bounding_box().diagonal().max(1e-6) * 1e-5;
    for f in faces {
        let proj = Projector::new(&f.surface, dir, tol);
        for path in proj.project_curve(c) {
            // keep the parts inside the face's trims
            let mut run: Vec<DVec3> = Vec::new();
            let mut flush = |run: &mut Vec<DVec3>| {
                if run.len() >= 2 {
                    let crv = if c.degree == 1 { NurbsCurve::polyline(std::mem::take(run)) } else { NurbsCurve::interpolate(&std::mem::take(run), 3) };
                    if let Ok(x) = crv {
                        out.push(x);
                    }
                }
                run.clear();
            };
            for uv in path {
                if f.contains_uv(uv) {
                    run.push(f.surface.point_at(uv.x, uv.y));
                } else {
                    flush(&mut run);
                }
            }
            flush(&mut run);
        }
    }
    out
}

/// A point projected onto faces along `dir` (every hit).
pub fn project_point(q: DVec3, faces: &[Face], dir: DVec3) -> Vec<DVec3> {
    let tol: f64 = 1e-7;
    faces.iter().filter_map(|f| {
        let uv = f.project_uv(q, dir.normalize_or(DVec3::Z));
        f.contains_uv(uv).then(|| f.surface.point_at(uv.x, uv.y)).filter(|hit| (*hit - q).cross(dir).length() < tol.max((*hit - q).length() * 1e-4))
    }).collect()
}
