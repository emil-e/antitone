//! File settings, metadata and bounded encoding. No photographic graph or GPU state.
use super::{
    BlockOrder, ImageSize, RenderTarget, Result,
    samples::{FloatPrecision, Layout, Samples},
};
use crate::Error;
use std::fs;

mod exr;
mod tiff;
pub use exr::exr;
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
    Exr(FloatPrecision),
}

impl FileFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Tiff => "tif",
            Self::Exr(_) => "exr",
        }
    }

    pub(crate) fn layout(self) -> Layout {
        match self {
            Self::Tiff => Layout::Rgb16,
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
            (Self::Exr(precision), RenderTarget::SceneLinearAp0) => {
                exr::encode(file, size, precision, &mut block)
            }
            _ => unreachable!("the export plan admits only supported target and layout pairs"),
        }
    }
}
