//! Roll and adjustment-layer grading, from scalar controls to image operations.

mod contrast;
mod scalar;
mod tonal_ranges;

#[cfg(feature = "editor")]
pub(crate) use scalar::CONTRAST_RANGE;
pub use scalar::{Grade, RollPipeline};
pub(crate) use tonal_ranges::MAX_EV as MAX_TONAL_EV;
#[cfg(feature = "gpu")]
use tonal_ranges::{WIDTH as TONAL_WIDTH, encoded_grey};

#[cfg(feature = "gpu")]
mod graph;
#[cfg(all(test, feature = "editor"))]
mod graph_tests;
#[cfg(feature = "gpu")]
mod operation;
#[cfg(feature = "gpu")]
pub use operation::Grading;
#[cfg(all(test, feature = "gpu"))]
pub(crate) use operation::{AdjustmentLayerContext, AdjustmentLayerParameters};
