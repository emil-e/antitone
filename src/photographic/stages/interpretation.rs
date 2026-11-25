//! Camera response, conversion to working light, and source inspection.

mod model;
pub use model::{InputPipeline, Inversion, InvertedPixel};
pub(crate) use model::{log_ratio, validate_camera_matrix};

#[cfg(feature = "gpu")]
pub(crate) mod operation;
#[cfg(feature = "gpu")]
mod transform;
#[cfg(all(test, feature = "editor"))]
pub(crate) use transform::has_filtered_inspection;
#[cfg(feature = "editor")]
pub(crate) use transform::inspect_filtered;
#[cfg(feature = "gpu")]
pub use transform::{InputTransform, InspectionView};
