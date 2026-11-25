//! Photographic stages, grading mathematics and graph composition.
#[cfg(feature = "project")]
mod acescct;
#[cfg(feature = "gpu")]
pub mod graph;
pub mod stages;

#[cfg(feature = "project")]
pub(crate) use acescct::{decode, encode};

pub use stages::grading::Grade;
