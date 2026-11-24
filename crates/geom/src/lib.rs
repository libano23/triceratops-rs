//! tri-geom: a small NURBS geometry kernel (f64 precision).
//!
//! Everything here is independent of rendering and UI. Curves and surfaces
//! are represented as non-uniform rational B-splines with clamped knot vectors,
//! following the conventions of "The NURBS Book" (Piegl & Tiller).

pub mod basis;
pub mod bbox;
pub mod brep;
pub mod brep_boolean;
pub mod brep_boolean_exact;
pub mod curve;
pub mod curve_ops;
pub mod display_mesh;
pub mod mesh;
pub mod mesh_csg;
pub mod mesh_ops;
pub mod plane;
pub mod ray;
pub mod surface;
pub mod surface_ops;

pub use bbox::BoundingBox;
pub use brep::{curve_plane, Brep, Face};
pub use curve::NurbsCurve;
pub use display_mesh::MeshDensity;
pub use curve_ops::{chamfer_curves, connect_curves, curve_curve_intersections, fillet_curves, CurveIntersection};
pub use glam::{DAffine3, DMat3, DMat4, DQuat, DVec2, DVec3, DVec4};
pub use mesh::Mesh;
pub use brep_boolean::brep_boolean;
pub use brep_boolean_exact::brep_boolean_exact;
pub use mesh_csg::{mesh_boolean, BooleanOp};
pub use plane::Plane;
pub use ray::Ray;
pub use surface::NurbsSurface;
pub use surface_ops::{split_region, Projector};

/// Absolute tolerance used for geometric comparisons.
pub const TOL: f64 = 1e-9;
/// Tolerance used when comparing parameters / knots.
pub const KNOT_TOL: f64 = 1e-12;

#[derive(Debug, thiserror::Error)]
pub enum GeomError {
    #[error("invalid NURBS data: {0}")]
    Invalid(String),
    #[error("not enough input points (need at least {need}, got {got})")]
    NotEnoughPoints { need: usize, got: usize },
    #[error("degenerate geometry: {0}")]
    Degenerate(String),
    #[error("incompatible inputs: {0}")]
    Incompatible(String),
}

pub type Result<T> = std::result::Result<T, GeomError>;

/// A rigid or affine transform applied to geometry.
pub type Transform = DAffine3;

/// Solve a dense linear system A x = b with partial pivoting (small systems).
pub(crate) fn solve_dense(mut a: Vec<Vec<f64>>, mut b: Vec<DVec3>) -> Result<Vec<DVec3>> {
    let n = a.len();
    for col in 0..n {
        let mut piv = col;
        for r in col + 1..n {
            if a[r][col].abs() > a[piv][col].abs() {
                piv = r;
            }
        }
        if a[piv][col].abs() < 1e-14 {
            return Err(GeomError::Degenerate("singular interpolation matrix".into()));
        }
        a.swap(col, piv);
        b.swap(col, piv);
        for r in col + 1..n {
            let f = a[r][col] / a[col][col];
            if f == 0.0 {
                continue;
            }
            for c in col..n {
                a[r][c] -= f * a[col][c];
            }
            let bc = b[col];
            b[r] -= f * bc;
        }
    }
    let mut x = vec![DVec3::ZERO; n];
    for r in (0..n).rev() {
        let mut s = b[r];
        for c in r + 1..n {
            s -= a[r][c] * x[c];
        }
        x[r] = s / a[r][r];
    }
    Ok(x)
}
