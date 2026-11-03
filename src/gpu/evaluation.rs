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
mod cell_region;
pub use cell_region::CellRegion;

mod context;

#[cfg(all(test, feature = "editor"))]
mod context_tests;

mod contract;

pub(crate) use cache::RetainedBuffer;
pub use cache::RetainedKey;
pub use context::{CachePolicy, Context, Evaluation, Production, Region};
pub use execute::{Sink, WorkCounts};
pub use plan::Budget;

#[cfg(all(test, feature = "editor"))]
mod contract_tests;

mod execute;

#[cfg(all(test, feature = "editor"))]
pub(crate) use execute::read as read_buffer;

#[cfg(all(test, feature = "editor"))]
mod fusion_tests;

pub(crate) mod grading;

#[cfg(all(test, feature = "editor"))]
mod grading_tests;

mod grain;

mod halation;
pub(crate) use halation::halation;

#[cfg(feature = "editor")]
mod inputs;
mod materializations;
mod materialized;
mod supplied;
#[cfg(feature = "editor")]
pub(crate) use inputs::ActiveInput;
use materialized::Stored;
pub(crate) use supplied::SuppliedImage;

pub(crate) use crate::gpu::Grading;
pub(crate) use materializations::BufferImage;
pub use materializations::{
    GpuImage, Image, ImageBacking, ImageFormat, ImageFuture, ImageLayout, Materializations,
};

#[cfg(all(test, feature = "editor"))]
mod native_tests;

mod operators;

mod photograph;

#[cfg(all(test, feature = "editor"))]
mod photograph_tests;

#[cfg(feature = "editor")]
pub(crate) use photograph::Neutral;
pub use photograph::Photograph;
#[cfg(test)]
pub(crate) use photograph::test_support as camera_fixture;

#[cfg(all(feature = "raw", feature = "render"))]
mod original;

#[cfg(feature = "editor")]
mod camera_response;
#[cfg(all(
    feature = "raw",
    feature = "project",
    any(feature = "editor", feature = "export")
))]
mod neutral_readout;
#[cfg(feature = "editor")]
pub(crate) use camera_response::CameraReadout;

mod plan;

#[cfg(feature = "editor")]
mod inspection;
#[cfg(feature = "editor")]
pub(crate) use inspection::inspect;

#[cfg(feature = "render")]
mod reference;

#[cfg(all(test, feature = "editor"))]
mod reconstruction_tests;

#[cfg(feature = "render")]
pub(crate) use reference::Reference;

#[cfg(all(test, feature = "editor"))]
mod reference_tests;

mod resident;

#[cfg(all(test, feature = "editor"))]
mod resident_camera_tests;

#[cfg(feature = "render")]
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
pub(crate) use operators::{gaussian, window};
mod reconstruction;
#[cfg(feature = "render")]
use operators::*;
pub use operators::{pointwise, source};
#[cfg(all(feature = "raw", feature = "render"))]
pub use original::camera_source;
#[cfg(all(feature = "raw", feature = "render"))]
pub(crate) use original::original_chain;
pub(crate) use plan::{Job, Plan, Policy, Tile};
#[cfg(test)]
pub(crate) use reconstruction::{
    Mapping as ReconstructionMapping, encode_supplied as encode_reconstruction_test,
    supplied_mask as reconstruction_mask_for_test,
};
pub(crate) use reconstruction::{
    accumulate_supplied, reconstruct, reconstruct_cells, reconstruct_complete,
};
pub use sampling::{
    Access, SampleConsumer, SampleInput, SampleProducer, SampleProduction, Sampled,
};
use std::sync::Arc;
