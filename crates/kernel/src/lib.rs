//! tri-kernel: exact B-rep solids on the solidcore kernel (feature `solidcore`).
//!
//! Every solidcore type stays inside this crate; the rest of Triceratops sees `Solid`,
//! `KernelError` and plain `glam`/`tri_geom` values.

#[cfg(feature = "solidcore")]
mod convert;
#[cfg(feature = "solidcore")]
mod solid;

#[cfg(feature = "solidcore")]
pub use solid::{KernelError, Solid};
