#[cfg(test)]
extern crate self as antitone;

pub mod acquisition;
pub mod adjustment_layers;
#[cfg(feature = "ui")]
pub mod application;
#[cfg(feature = "project")]
pub mod auto_crop;
#[cfg(feature = "editor")]
mod background;
#[cfg(feature = "editor")]
mod cancellation;
pub mod color;
pub mod coverage;
pub mod crop;
#[cfg(feature = "dng-sdk")]
pub mod dng;
#[cfg(all(test, feature = "dng-sdk"))]
#[path = "../tests/support/dng_fixture.rs"]
mod dng_fixture;
pub mod dust_repair;
#[cfg(feature = "editor")]
pub mod editor;
#[cfg(all(feature = "raw", feature = "project"))]
pub(crate) mod embedded_preview;
#[cfg(feature = "export")]
pub mod export;
#[cfg(feature = "export")]
mod export_timing;
#[cfg(feature = "project")]
pub mod exposure_metadata;
pub mod exposure_reference;
pub mod film_emulation;
pub mod flat_field;
pub mod frame_raster;
pub mod geometry;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(all(feature = "gpu", feature = "export", feature = "raw"))]
pub mod gpu_export;
pub mod grain;
pub mod halation;
pub mod image_mtf;
#[cfg(feature = "project")]
pub mod image_recipe;
pub mod lens;
#[cfg(feature = "logging")]
pub mod logging;
#[cfg(any(feature = "project", feature = "gpu"))]
pub mod looks;
#[cfg(any(feature = "raw", feature = "dng-sdk", feature = "ocio-reference"))]
mod native_logging;
#[cfg(all(feature = "raw", feature = "project"))]
pub(crate) mod photo_preparation;
pub mod photographic;
pub mod pixel_pipeline;
#[cfg(feature = "ui")]
pub mod platform;
pub mod processing;
pub mod processing_support;
#[cfg(feature = "render")]
pub mod profiles;
#[cfg(feature = "project")]
pub mod project;
#[cfg(feature = "raw")]
pub mod raw;
#[cfg(feature = "editor")]
mod refinement;
#[cfg(feature = "project")]
mod region_timing;
#[cfg(feature = "render")]
pub mod rendering;
#[cfg(feature = "raw")]
pub mod repair_source;
pub mod roll_analysis;
pub mod sampling;
#[cfg(feature = "raw")]
pub mod scan;
#[cfg(feature = "project")]
pub mod scan_regions;

#[cfg(any(feature = "project", feature = "raw"))]
pub mod source;

#[cfg(feature = "project")]
pub mod spatial_metadata;
#[cfg(any(
    feature = "raw",
    feature = "project",
    feature = "export",
    feature = "gpu",
    feature = "ui"
))]
pub(crate) mod sys;
#[cfg(feature = "editor")]
pub mod viewport;
#[cfg(all(
    test,
    feature = "gpu",
    feature = "export",
    feature = "raw",
    any(feature = "editor", feature = "ocio-reference")
))]
#[path = "../tests/support/working_precision.rs"]
mod working_precision;

#[cfg(all(feature = "ui", feature = "render"))]
pub mod gpu_scopes;
#[cfg(all(feature = "gpu", feature = "render"))]
pub mod renderer;
#[cfg(all(feature = "ui", feature = "render"))]
pub mod resample;
#[cfg(feature = "render")]
pub mod scopes;
#[cfg(all(feature = "ui", feature = "render"))]
pub mod viewer;

#[cfg(all(test, feature = "editor", target_os = "macos"))]
#[path = "../native/cache_observation.rs"]
pub(crate) mod cache_observation;
#[cfg(all(
    test,
    feature = "raw",
    feature = "project",
    feature = "ocio-reference",
    any(
        feature = "editor",
        all(target_os = "macos", feature = "gpu", feature = "export")
    )
))]
#[path = "../tests/support/exposure_fixture.rs"]
mod exposure_fixture;
#[cfg(all(test, any(feature = "dng-sdk", feature = "raw")))]
#[path = "../tests/support/fixture_path.rs"]
mod fixture_path;
#[cfg(all(test, feature = "raw", feature = "project"))]
#[path = "../tests/support/preview_fixture.rs"]
mod preview_fixture;
#[cfg(all(test, feature = "raw", feature = "gpu"))]
mod raw_test_support;

#[cfg(all(test, feature = "ocio-reference", target_os = "macos"))]
mod view_sampling_tests;

#[cfg(all(test, feature = "raw", feature = "project"))]
#[path = "../native/spatial_observation.rs"]
mod spatial_observation;

#[cfg(all(test, feature = "gpu"))]
mod shader_validation_tests;

pub const PROCESSING_MODEL_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    InvalidParameter(&'static str),
    InvalidCalibration {
        channel: usize,
        reason: &'static str,
    },
    NonFinitePixel {
        channel: usize,
    },
    UnusableSample(&'static str),
    NumericalOverflow,
    Raw(String),
    Gpu(String),
    Rendering(String),
    Export(String),
    DestinationExists,
    Cancelled,
    Project(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidParameter(reason) => write!(f, "Invalid parameter: {reason}"),
            Self::InvalidCalibration { channel, reason } => {
                write!(f, "Invalid calibration in channel {channel}: {reason}")
            }
            Self::NonFinitePixel { channel } => write!(f, "Non-finite pixel in channel {channel}"),
            Self::UnusableSample(reason) => write!(f, "Sample ignored: {reason}"),
            Self::NumericalOverflow => write!(f, "Processing exceeded the finite numeric range"),
            Self::Raw(message) => write!(f, "RAW development: {message}"),
            Self::Gpu(message) => write!(f, "GPU processing: {message}"),
            Self::Rendering(message) => write!(f, "Output rendering: {message}"),
            Self::Export(message) => write!(f, "Export: {message}"),
            Self::DestinationExists => write!(f, "Export destination already exists"),
            Self::Cancelled => write!(f, "Operation cancelled"),
            Self::Project(message) => write!(f, "Project: {message}"),
        }
    }
}

impl std::error::Error for Error {}
