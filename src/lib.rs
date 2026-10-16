#[cfg(feature = "project")]
pub mod auto_crop;
pub mod color;
pub mod coverage;
pub mod crop;
pub mod dust_repair;
#[cfg(feature = "editor")]
pub mod editor;
#[cfg(feature = "export")]
pub mod export;
#[cfg(feature = "export")]
mod export_timing;
pub mod flat_field;
pub mod frame_raster;
pub mod geometry;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(all(feature = "gpu", feature = "export", feature = "raw"))]
pub mod gpu_export;
pub mod local_adjustments;
pub mod photographic;
#[cfg(feature = "ui")]
pub mod platform;
pub mod processing;
#[cfg(feature = "render")]
pub mod profiles;
#[cfg(feature = "project")]
pub mod project;
#[cfg(feature = "raw")]
pub mod raw;
#[cfg(feature = "editor")]
mod refinement;
#[cfg(feature = "render")]
pub mod rendering;
#[cfg(feature = "raw")]
mod resources;
pub mod roll_analysis;
pub mod sampling;
#[cfg(feature = "raw")]
pub mod scan;
#[cfg(feature = "project")]
pub mod scan_regions;
#[cfg(all(feature = "raw", feature = "project"))]
mod source_revision;
#[cfg(all(feature = "raw", feature = "project"))]
pub mod source_store;
#[cfg(feature = "editor")]
pub mod viewport;

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
#[cfg(all(test, feature = "raw", feature = "gpu", target_os = "macos"))]
mod raw_test_support;

#[cfg(all(test, feature = "ocio-reference", target_os = "macos"))]
mod view_sampling_tests;

pub const PROCESSING_MODEL_VERSION: u32 = 4;

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
            Self::Project(message) => write!(f, "Project: {message}"),
        }
    }
}

impl std::error::Error for Error {}
