//! Mesh: meshes from NURBS geometry and back, primitives and vertices.

use tri_doc::Geometry;
use tri_geom::{Mesh, NurbsSurface};

use super::super::data::{DataTree, Value};
use super::geo;
use super::*;
use crate::commands::{mesh_cmds, surface2};

/// A welded mesh, so closed shapes give closed meshes.
fn welded(m: &Mesh) -> Mesh {
    let tol = m.bounding_box().diagonal().max(1e-9) * 1e-9;
    mesh_cmds::weld(m, tol)
}

fn meshes(t: &DataTree) -> Vec<Mesh> {
    t.items().iter().filter_map(|v| v.as_geometry()).filter_map(|gm| match gm {
        Geometry::Mesh(m) => Some(m.clone()),
        _ => None,
    }).collect()
}

pub fn register(r: &mut Registry) {
    r.elem("Mesh Brep", "Mesh", vec![geo("Brep")], &["Mesh"], |_, a| {
        let m = geo::to_mesh(a.geo(0)?)?;
        one(g(Geometry::Mesh(welded(&m))))
    });
    r.elem("Mesh to Brep", "Mesh", vec![geo("Mesh")], &["Brep"], |_, a| {
        let m = a.mesh(0)?;
        let mut b = surface2::mesh_to_brep(&m)?;
        if b.closed {
            b.orient_outward();
        }
        one(g(Geometry::Brep(b)))
    });
    r.tree("Mesh Vertices", "Mesh", vec![geo("Mesh")], &["Points"], |_, t| t1(DataTree::from_list(meshes(&t[0]).iter().flat_map(|m| m.vertices.clone()).map(Value::Point).collect())));
    r.elem("Mesh Box", "Mesh", vec![pt("Corner", 0.0, 0.0, 0.0), num("X", 5.0), num("Y", 5.0), num("Z", 5.0)], &["Mesh"], |_, a| {
        let c = a.pt(0);
        let other = c + glam::DVec3::new(a.num(1), a.num(2), a.num(3));
        let m = Mesh::cuboid(c.min(other), c.max(other));
        one(g(Geometry::Mesh(m)))
    });
    r.elem("Mesh Sphere", "Mesh", vec![pt("Center", 0.0, 0.0, 0.0), num("Radius", 5.0)], &["Mesh"], |_, a| {
        if a.num(1) <= 0.0 {
            return None;
        }
        let s = NurbsSurface::sphere(a.pt(0), a.num(1)).ok()?;
        let m = geo::to_mesh(&Geometry::Surface(s))?;
        one(g(Geometry::Mesh(welded(&m))))
    });
    r.tree("Deconstruct Mesh", "Mesh", vec![geo("Mesh")], &["Vertices", "Count"], |_, t| {
        let verts: Vec<Value> = meshes(&t[0]).iter().flat_map(|m| m.vertices.clone()).map(Value::Point).collect();
        let count = verts.len() as f64;
        vec![DataTree::from_list(verts), DataTree::single(n(count))]
    });
}
