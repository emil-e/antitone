//! Captured raster execution; sampling strategies retain their distinct reconstruction order.

#[cfg(feature = "raw")]
mod originals;

#[cfg(feature = "raw")]
mod request;

#[cfg(all(feature = "raw", feature = "export"))]
pub(crate) use originals::{Stats, execute_blocks};
#[cfg(all(feature = "raw", test))]
pub(crate) use originals::{TILE, execute};
#[cfg(feature = "raw")]
pub(crate) use request::{OutputBlock, Request, SampleEncoding};

#[cfg(test)]
pub(crate) mod raster;

#[cfg(all(test, feature = "raw"))]
pub(crate) mod test_support;

#[cfg(all(test, feature = "raw", target_os = "macos"))]
mod photographic_tests;

#[cfg(feature = "editor")]
pub(crate) mod regional;
