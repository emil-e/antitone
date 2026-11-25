//! Photographic stages, grading mathematics and graph composition.
#[cfg(feature = "gpu")]
pub mod graph;
pub mod stages;

pub use stages::grading::Grade;
