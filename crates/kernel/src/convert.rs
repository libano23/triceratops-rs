//! glam values in solidcore's conventions.

use glam::{DAffine3, DQuat, DVec3};

/// The matrix `Body::transform` takes: row-vector convention (`p' = p·A + t`), rows 0-2 the
/// images of x, y and z, row 3 the translation, fourth column `(0, 0, 0, 1)`.
pub fn matrix(xf: &DAffine3) -> [[f64; 4]; 4] {
    let m = xf.matrix3;
    let t = xf.translation;
    [
        [m.x_axis.x, m.x_axis.y, m.x_axis.z, 0.0],
        [m.y_axis.x, m.y_axis.y, m.y_axis.z, 0.0],
        [m.z_axis.x, m.z_axis.y, m.z_axis.z, 0.0],
        [t.x, t.y, t.z, 1.0],
    ]
}

/// The placement that takes a primitive built about +Z at the origin to `origin` with its axis
/// along `axis`. `None` for a zero or non-finite axis.
pub fn placement(origin: DVec3, axis: DVec3) -> Option<DAffine3> {
    let a = axis.try_normalize()?;
    Some(DAffine3::from_rotation_translation(DQuat::from_rotation_arc(DVec3::Z, a), origin))
}
