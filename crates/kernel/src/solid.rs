//! `Solid`: a solidcore body with the operations Triceratops uses.

use std::fmt;

use glam::{DAffine3, DVec3};
use solidcore_ops::body::{make_box, make_cone, make_cylinder, make_sphere, make_torus};
use solidcore_ops::{Body, Error};
use solidcore_record::brep::Brep as Record;

use crate::convert;

/// Relative accuracy asked of the kernel's volume.
const VOLUME_ACCURACY: f64 = 1e-9;

/// Why the kernel did not give a result. `Display` is the text shown to the user.
#[derive(Clone, Debug, PartialEq)]
pub enum KernelError {
    /// The kernel refused, with its own message.
    Refused(String),
    /// The call was cancelled before it finished.
    Cancelled,
}

impl fmt::Display for KernelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelError::Refused(m) => write!(f, "{m}"),
            KernelError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for KernelError {}

impl From<Error> for KernelError {
    fn from(e: Error) -> Self {
        match e {
            Error::Cancelled => KernelError::Cancelled,
            other => KernelError::Refused(other.to_string()),
        }
    }
}

pub(crate) fn refused(msg: &str) -> KernelError {
    KernelError::Refused(msg.to_string())
}

/// A solid held by the kernel.
pub struct Solid {
    pub(crate) body: Body,
}

impl Solid {
    pub(crate) fn new(body: Body) -> Self {
        Self { body }
    }

    /// An independent copy (the kernel edits bodies in place).
    pub fn duplicate(&self) -> Result<Solid, KernelError> {
        Ok(Solid::new(self.body.copy()?))
    }

    /// Axis-aligned box between two opposite corners.
    pub fn cuboid(min: DVec3, max: DVec3) -> Result<Solid, KernelError> {
        let size = max - min;
        Ok(Solid::new(make_box(min.to_array(), size.to_array())?))
    }

    /// Cylinder from the centre of its base along `axis` for `height`.
    pub fn cylinder(base: DVec3, axis: DVec3, radius: f64, height: f64) -> Result<Solid, KernelError> {
        Self::placed(make_cylinder([0.0; 3], radius, height)?, base, axis)
    }

    pub fn sphere(center: DVec3, radius: f64) -> Result<Solid, KernelError> {
        Ok(Solid::new(make_sphere(center.to_array(), radius)?))
    }

    /// Cone or frustum from the centre of its base along `axis`; `top_radius` may be 0.
    pub fn cone(base: DVec3, axis: DVec3, base_radius: f64, top_radius: f64, height: f64) -> Result<Solid, KernelError> {
        Self::placed(make_cone([0.0; 3], base_radius, top_radius, height)?, base, axis)
    }

    /// Ring torus about `axis` through `center`; `major` > `minor` > 0.
    pub fn torus(center: DVec3, axis: DVec3, major: f64, minor: f64) -> Result<Solid, KernelError> {
        Self::placed(make_torus([0.0; 3], major, minor)?, center, axis)
    }

    fn placed(body: Body, origin: DVec3, axis: DVec3) -> Result<Solid, KernelError> {
        let xf = convert::placement(origin, axis).ok_or_else(|| refused("the axis has no direction"))?;
        let s = Solid::new(body);
        s.body.transform(&convert::matrix(&xf))?;
        Ok(s)
    }

    /// Enclosed volume.
    pub fn volume(&self) -> Result<f64, KernelError> {
        Ok(self.body.volume(VOLUME_ACCURACY)?)
    }

    /// A moved copy; `self` is unchanged.
    pub fn transformed(&self, xf: &DAffine3) -> Result<Solid, KernelError> {
        let copy = self.duplicate()?;
        copy.body.transform(&convert::matrix(xf))?;
        Ok(copy)
    }

    /// The body as solidcore's data-only record.
    pub fn record(&self) -> Result<Record, KernelError> {
        Ok(self.body.to_record()?.0)
    }
}
