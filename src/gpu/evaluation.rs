//! Captured image nodes, operation planning and bounded GPU evaluation.

#[cfg(feature = "editor")]
pub(crate) mod acquire;

#[cfg(all(test, feature = "editor"))]
mod acquire_tests;

#[cfg(all(test, feature = "editor"))]
mod authoring_tests;

#[cfg(all(test, feature = "editor"))]
mod boundary_tests;

mod allocations;
pub(crate) use allocations::Allocations;

mod cache;
mod cell_region;
pub use cell_region::CellRegion;

mod context;

#[cfg(all(test, feature = "editor"))]
mod context_tests;

mod contract;
mod unit;
pub mod value;
mod working;
pub use unit::*;
pub use working::RunWorkingSet;

pub(crate) use crate::float_hash::f64 as hash_f64;
pub(crate) use cache::RetainedBuffer;
pub use cache::RetainedKey;
#[cfg(test)]
pub(crate) mod numeric_fixture;
pub use context::{CachePolicy, Context, Evaluation, Production, Region, Worker};
#[cfg(feature = "editor")]
pub(crate) use context::{Preparation, RecordedImage};
pub use execute::{Sink, WorkCounts};
pub use plan::Tiling;

#[cfg(all(test, feature = "editor"))]
mod contract_tests;
#[cfg(all(test, feature = "editor"))]
mod dependency_tests;
#[cfg(all(test, feature = "editor"))]
mod unit_runtime_tests;

mod execute;

#[cfg(all(test, feature = "editor"))]
pub(crate) use execute::read as read_buffer;

#[cfg(all(test, feature = "editor"))]
mod fusion_tests;

mod materializations;
mod materialized;
mod supplied;
use materialized::Stored;
pub(crate) use supplied::SuppliedImage;

pub(crate) use materializations::BufferImage;
pub use materializations::{
    Image, ImageBacking, ImageFormat, ImageFuture, ImageLayout, ImageValue, Materializations,
};

#[cfg(all(test, feature = "editor"))]
mod native_tests;

mod operators;

mod plan;

#[cfg(all(test, feature = "editor"))]
mod reconstruction_tests;

mod resident;

#[cfg(all(test, feature = "editor"))]
mod resident_camera_tests;

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
pub(crate) use operators::window;
#[cfg(all(test, feature = "editor"))]
use operators::{grade, interpret};
pub use value::Value;
mod reconstruction;
pub use operators::{pointwise, source};
pub(crate) use plan::{Job, Plan, Policy, Tile};
#[cfg(test)]
pub(crate) use reconstruction::{
    Mapping as ReconstructionMapping, encode_supplied as encode_reconstruction_test,
    supplied_mask as reconstruction_mask_for_test,
};
pub(crate) use reconstruction::{
    accumulate_supplied, reconstruct, reconstruct_cells, reconstruct_complete, reconstruct_tiled,
};
pub(crate) use sampling::{Access, SampleInput, Sampled};
use std::sync::Arc;
