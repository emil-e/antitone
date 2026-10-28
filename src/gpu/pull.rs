//! Captured image layers, demand planning and bounded GPU execution.
#[cfg(all(test, feature = "editor"))]
mod boundary_tests;
mod cache;
mod contract;
pub(crate) use cache::RetainedBuffer;
pub use cache::RetainedKey;
#[cfg(all(test, feature = "editor"))]
mod contract_tests;
mod execute;
#[cfg(all(test, feature = "editor"))]
pub(crate) use execute::read as read_buffer;
#[cfg(all(test, feature = "editor"))]
mod fusion_tests;
mod grain;
#[cfg(all(test, feature = "editor"))]
mod native_tests;
mod operators;
#[cfg(feature = "raw")]
mod original;
mod plan;
mod resident;
pub(crate) use resident::raster_input;
#[cfg(all(test, feature = "editor"))]
mod production_tests;
#[cfg(all(test, feature = "editor"))]
mod resource_tests;
mod sampling;
#[cfg(all(test, feature = "editor"))]
mod tests;
#[cfg(all(test, feature = "editor"))]
mod validation_tests;

use crate::Error;
pub use contract::*;
pub(crate) use execute::{Executor, Programs, Recorded, Sink};
pub(crate) use grain::grain;
pub(crate) use operators::gaussian;
pub use operators::pointwise;
use operators::*;
#[cfg(feature = "raw")]
pub(crate) use original::original_chain;
pub(crate) use plan::*;
pub use sampling::{Access, SampleConsumer, SampleInput, SampleProducer, Sampled};
use std::sync::Arc;
