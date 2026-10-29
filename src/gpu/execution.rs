//! Captured raster execution; sampling strategies retain their distinct reconstruction order.

#[cfg(feature = "raw")]
mod originals;

#[cfg(feature = "raw")]
mod request;

#[cfg(all(test, feature = "raw", target_os = "macos"))]
pub(crate) use originals::Unit;
#[cfg(feature = "raw")]
pub(crate) use originals::{Stats, TILE, execute};
#[cfg(feature = "raw")]
pub(crate) use request::{OutputBlock, Request, SampleEncoding, WorkBudget};

#[cfg(feature = "editor")]
pub(crate) mod footprints;

pub(crate) mod raster;

#[cfg(feature = "editor")]
pub(crate) mod regional;

#[cfg(feature = "editor")]
pub(crate) mod source;
