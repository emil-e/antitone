//! File settings, metadata and bounded encoding. No photographic graph or GPU state.
#[cfg(any(test, all(feature = "gpu", feature = "raw")))]
use super::samples::Layout;
use super::{
    BlockOrder, ImageSize, RenderTarget, Result,
    samples::{FloatPrecision, Samples},
};
use crate::Error;
use std::fs;

mod exr;
mod jpeg;
mod tiff;
pub use exr::exr;
pub use jpeg::JpegQuality;
pub use tiff::tiff_encoded;
#[cfg(feature = "ocio-reference")]
pub use {
    exr::exr_composed,
    tiff::{tiff, tiff_composed},
};
#[cfg(all(test, target_os = "macos"))]
pub(super) use {
    exr::{exr_blocks, exr_blocks_checked},
    tiff::{tiff_encoded_checked, tiff_encoded_ordered},
};
#[cfg(all(test, target_os = "macos"))]
mod tests;

fn sample_mismatch() -> Error {
    Error::InvalidParameter("samples do not match the file encoding")
}

const MAX_WORKSPACE: u64 = 1 << 30;

fn workspace(size: ImageSize, rows: u32, bytes: u64, metadata: u64) -> Result<usize> {
    size.count()?;
    let count = u64::from(size.width) * u64::from(size.height.min(rows));
    // A band, prediction/compression scratch and the format's offset tables are bounded.
    if (count * bytes + metadata) * 3 > MAX_WORKSPACE {
        return Err(Error::Export(
            "codec band exceeds the current 1 GiB workspace budget".into(),
        ));
    }
    usize::try_from(count).map_err(|_| Error::NumericalOverflow)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Tiff,
    Jpeg(JpegQuality),
    Exr(FloatPrecision),
}

impl FileFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiff => "tif",
            Self::Jpeg(_) => "jpg",
            Self::Exr(_) => "exr",
        }
    }

    pub(crate) fn supports_target(self, target: RenderTarget) -> bool {
        match self {
            Self::Tiff => matches!(target, RenderTarget::Display(_)),
            Self::Jpeg(_) => matches!(target, RenderTarget::Display(display) if !display.is_hdr()),
            Self::Exr(_) => matches!(target, RenderTarget::SceneLinearAp0),
        }
    }

    #[cfg(any(test, all(feature = "gpu", feature = "raw")))]
    pub(crate) fn layout(self) -> Layout {
        match self {
            Self::Tiff => Layout::Rgb16,
            Self::Jpeg(_) => Layout::Rgb8,
            Self::Exr(precision) => Layout::Float(precision),
        }
    }

    pub(super) fn encode(
        self,
        file: &mut fs::File,
        size: ImageSize,
        target: RenderTarget,
        blocks: (BlockOrder, [u32; 2]),
        mut block: impl FnMut([u32; 2], [u32; 2]) -> Result<Samples>,
    ) -> Result<()> {
        match (self, target) {
            (Self::Tiff, RenderTarget::Display(target)) => {
                tiff::encode(file, size, target, blocks, &mut block)
            }
            (Self::Jpeg(quality), RenderTarget::Display(target)) => {
                jpeg::encode(file, size, target, quality, blocks.1, &mut block)
            }
            (Self::Exr(precision), RenderTarget::SceneLinearAp0) => {
                exr::encode(file, size, precision, &mut block)
            }
            _ => unreachable!("the export plan admits only supported target and codec pairs"),
        }
    }
}
