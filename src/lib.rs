pub mod color;
pub mod coverage;
pub mod crop;
#[cfg(feature = "editor")]
pub mod editor;
#[cfg(feature = "export")]
pub mod export;
pub mod flat_field;
pub mod frame_raster;
pub mod geometry;
#[cfg(feature = "gpu")]
pub mod gpu;
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
pub mod sampling;
#[cfg(feature = "raw")]
pub mod scan;
#[cfg(feature = "editor")]
pub mod viewport;

#[cfg(all(feature = "ui", feature = "render"))]
pub mod resample;
#[cfg(all(feature = "ui", feature = "render"))]
pub mod viewer;

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
