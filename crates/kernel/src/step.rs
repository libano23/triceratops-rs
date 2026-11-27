//! STEP export of exact solids, written by the kernel from its own records: analytic surfaces,
//! circles and lines where the geometry is one, edges shared by the faces that meet there.

use tri_geom::Brep;

use crate::solid::{refused, KernelError, Solid};

/// The STEP file (AP242) of `bodies` (a name and the solid), or `None` when one of them has no
/// valid exact body: the caller then writes the faces as they are. `scale` multiplies every length
/// (millimetres in one unit of the document). `file_name` is the file's own
/// name, `program` the program that wrote it.
pub fn step_text(bodies: &[(String, &Brep)], scale: f64, file_name: &str, program: &str) -> Option<Result<String, KernelError>> {
    let mut records = Vec::new();
    for (name, brep) in bodies {
        // from_exact applies the placement the solid was moved by, so the record is where the solid is
        let mut solid = Solid::from_exact(brep)?;
        if (scale - 1.0).abs() > 1e-12 {
            // the file is in millimetres: scale the solid from the document's unit
            solid = match solid.transformed(&glam::DAffine3::from_scale(glam::DVec3::splat(scale))) {
                Ok(s) => s,
                Err(e) => return Some(Err(e)),
            };
        }
        match solid.record() {
            Ok(r) => records.push((name.clone(), r)),
            Err(e) => return Some(Err(e)),
        }
    }
    let list = records
        .iter()
        .map(|(name, record)| solidcore_step::write::product::ExportBody { name: name.clone(), record, color: None, faces: Vec::new() })
        .collect();
    let header = solidcore_step::write::product::Header { file_name: file_name.to_string(), preprocessor_version: program.to_string(), originating_system: program.to_string(), ..Default::default() };
    Some(solidcore_step::write::product::bodies(list, &header).map_err(|e| refused(&format!("STEP export: {e:?}"))))
}
