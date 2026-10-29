//! Captured image nodes, operation planning and bounded GPU evaluation.

#[cfg(feature = "editor")]
pub(crate) mod acquire;

#[cfg(all(test, feature = "editor"))]
mod acquire_tests;

#[cfg(all(test, feature = "editor"))]
mod authoring_tests;

#[cfg(all(test, feature = "editor"))]
mod boundary_tests;

mod cache;

mod context;

#[cfg(all(test, feature = "editor"))]
mod context_tests;

mod contract;

pub(crate) use cache::RetainedBuffer;
pub use cache::RetainedKey;
pub use context::{CachePolicy, Context, Evaluation, Region};
pub use execute::{Sink, Stats};
pub use plan::Budget;

#[cfg(all(test, feature = "editor"))]
mod contract_tests;

mod execute;

#[cfg(all(test, feature = "editor"))]
pub(crate) use execute::read as read_buffer;

#[cfg(all(test, feature = "editor"))]
mod fusion_tests;

mod grading;

#[cfg(all(test, feature = "editor"))]
mod grading_tests;

mod grain;

mod images;

#[cfg(all(test, feature = "editor"))]
mod images_tests;

pub(crate) use grading::Grading;
pub use images::{CachedImage, ImageCache};

#[cfg(all(test, feature = "editor"))]
mod native_tests;

mod operators;

#[cfg(any(feature = "raw", feature = "ui"))]
mod photograph;

#[cfg(all(test, feature = "editor"))]
mod photograph_tests;

#[cfg(feature = "editor")]
pub(crate) use photograph::Neutral;
#[cfg(any(feature = "raw", feature = "ui"))]
pub use photograph::Photograph;

#[cfg(feature = "raw")]
mod original;

mod plan;

#[cfg(any(feature = "raw", feature = "ui"))]
mod recipe;

mod reference;

pub(crate) use reference::Reference;

#[cfg(all(test, feature = "editor"))]
mod reference_tests;

mod resident;

#[cfg(all(test, feature = "editor"))]
mod resident_camera_tests;

#[cfg(any(feature = "raw", feature = "ui"))]
pub(crate) use recipe::Recipe;
pub(crate) use resident::borrowed_input;
#[cfg(test)]
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
pub(crate) use execute::{Executor, Programs, Recorded};
pub(crate) use grain::grain;
pub(crate) use operators::gaussian;
pub use operators::pointwise;
use operators::*;
#[cfg(feature = "raw")]
pub use original::camera_source;
#[cfg(feature = "raw")]
pub(crate) use original::original_chain;
pub(crate) use plan::{Job, Plan, Policy, Tile};
pub use sampling::{Access, SampleConsumer, SampleInput, SampleProducer, Sampled};
use std::sync::Arc;
