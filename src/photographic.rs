//! Photographic stages, grading mathematics and graph composition.
#[cfg(feature = "gpu")]
pub mod graph;
#[cfg(feature = "gpu")]
pub mod parameters;
pub mod stages;

#[cfg(all(test, feature = "gpu"))]
mod gpu_tests;
#[cfg(all(test, feature = "gpu"))]
mod input_grading_tests;
#[cfg(all(test, feature = "gpu"))]
mod shader_layout_tests;
#[cfg(all(test, feature = "gpu"))]
pub(crate) mod test_inputs;
#[cfg(all(test, feature = "gpu", feature = "render"))]
pub(crate) mod test_support;

pub use stages::grading::Grade;
