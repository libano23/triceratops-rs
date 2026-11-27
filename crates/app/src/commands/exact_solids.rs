//! Exact bodies for the primitive commands and prisms (feature `kernel-solidcore`).
//!
//! The faces stay exactly what the legacy construction makes (so every legacy command and
//! Boolean behaves as before, including on surfaces shared with other solids); the kernel's body
//! of the same solid is attached next to them. Each function returns its `Brep` unchanged when
//! the feature is off or the kernel refuses the input.

use glam::DVec3;
use tri_geom::{Brep, NurbsCurve, Plane};

#[cfg(feature = "kernel-solidcore")]
mod on {
    use std::sync::Arc;

    use glam::{DAffine3, DMat3, DVec3};
    use tri_geom::{Brep, NurbsCurve, Plane};
    use tri_kernel::{KernelError, Solid};

    /// `b` with the record of `solid` attached, if the kernel made one and says it is closed.
    fn with(mut b: Brep, solid: Result<Solid, KernelError>) -> Brep {
        if let Ok(s) = solid {
            if let Ok(rec) = s.record() {
                b.closed = true;
                b.attach_exact_with_volume(Arc::new(rec), s.volume().ok());
            }
        }
        b
    }

    pub fn cuboid(b: Brep, base: &Plane, dx: f64, dy: f64, dz: f64) -> Brep {
        let (lo, hi) = (DVec3::new(dx.min(0.0), dy.min(0.0), dz.min(0.0)), DVec3::new(dx.max(0.0), dy.max(0.0), dz.max(0.0)));
        let xf = DAffine3 { matrix3: DMat3::from_cols(base.x_axis, base.y_axis, base.z_axis), translation: base.origin };
        with(b, Solid::cuboid(lo, hi).and_then(|s| s.transformed(&xf)))
    }

    /// The kernel wants a positive height: a negative one runs the other way along the axis.
    fn upright(axis: DVec3, h: f64) -> (DVec3, f64) {
        if h < 0.0 {
            (-axis, -h)
        } else {
            (axis, h)
        }
    }

    pub fn cylinder(b: Brep, base: DVec3, axis: DVec3, r: f64, h: f64) -> Brep {
        let (axis, h) = upright(axis, h);
        with(b, Solid::cylinder(base, axis, r, h))
    }

    pub fn cone(b: Brep, base: DVec3, axis: DVec3, r: f64, h: f64) -> Brep {
        let (axis, h) = upright(axis, h);
        with(b, Solid::cone(base, axis, r, 0.0, h))
    }

    pub fn sphere(b: Brep, c: DVec3, r: f64) -> Brep {
        with(b, Solid::sphere(c, r))
    }

    pub fn torus(b: Brep, c: DVec3, axis: DVec3, major: f64, minor: f64) -> Brep {
        with(b, Solid::torus(c, axis, major, minor))
    }

    pub fn prism(b: Brep, c: &NurbsCurve, plane: &Plane, half: f64) -> Brep {
        with(b, Solid::extrude_closed_curve(c, plane, half))
    }
}

#[cfg(feature = "kernel-solidcore")]
pub use on::*;

#[cfg(not(feature = "kernel-solidcore"))]
mod off {
    use super::*;
    pub fn cuboid(b: Brep, _: &Plane, _: f64, _: f64, _: f64) -> Brep {
        b
    }
    pub fn cylinder(b: Brep, _: DVec3, _: DVec3, _: f64, _: f64) -> Brep {
        b
    }
    pub fn cone(b: Brep, _: DVec3, _: DVec3, _: f64, _: f64) -> Brep {
        b
    }
    pub fn sphere(b: Brep, _: DVec3, _: f64) -> Brep {
        b
    }
    pub fn torus(b: Brep, _: DVec3, _: DVec3, _: f64, _: f64) -> Brep {
        b
    }
    pub fn prism(b: Brep, _: &NurbsCurve, _: &Plane, _: f64) -> Brep {
        b
    }
}

#[cfg(not(feature = "kernel-solidcore"))]
pub use off::*;
