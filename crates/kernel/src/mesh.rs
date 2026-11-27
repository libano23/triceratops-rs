//! The triangle mesh of an exact solid for a mesh file (STL, 3MF, OBJ, ...): the kernel's own
//! mesher, whose faces meet at identical edge nodes, so the mesh has no cracks.

use glam::DVec3;
use solidcore_tess::{plan, MeshTolerance};
use tri_geom::{Brep, Mesh};

use crate::solid::Solid;

/// The mesh of `brep` at the export tolerance of its size (1/5000 of its box diagonal), or `None`
/// when it has no valid exact body or the kernel leaves a face out: the caller then meshes the
/// faces itself.
pub fn export_mesh(brep: &Brep) -> Option<Mesh> {
    let record = Solid::from_exact(brep)?.record().ok()?;
    let size = brep.bounding_box().diagonal().max(1e-9);
    let (plan, _stats) = plan(&record, &MeshTolerance::export_of(size), size).ok()?;
    let (mesh, refused) = plan.evaluate();
    if !refused.is_empty() || mesh.triangles.is_empty() {
        return None;
    }
    let v = |p: &solidcore_record::Vec3| DVec3::new(p.x, p.y, p.z);
    Some(Mesh { vertices: mesh.positions.iter().map(v).collect(), normals: mesh.normals.iter().map(v).collect(), indices: mesh.triangles.iter().flatten().copied().collect() })
}
