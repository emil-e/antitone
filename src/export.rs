use crate::export_timing::timed;
use crate::{Error, rendering::OutputTarget};
use std::{fs, path::Path};
use tempfile::NamedTempFile;
mod codec;
pub use codec::{FileFormat, exr, tiff_encoded};
#[cfg(all(test, target_os = "macos"))]
use codec::{exr_blocks, exr_blocks_checked, tiff_encoded_checked, tiff_encoded_ordered};
#[cfg(feature = "ocio-reference")]
pub use codec::{exr_composed, tiff, tiff_composed};
mod samples;
pub use samples::{FloatPrecision, FloatPrecision as ExrPrecision};
pub(crate) use samples::{Layout, Samples, SceneEncoding};
#[cfg(all(test, target_os = "macos"))]
mod publication_tests;
#[cfg(all(test, target_os = "macos"))]
mod tests;

type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

impl ImageSize {
    fn count(self) -> Result<usize> {
        if self.width == 0
            || self.height == 0
            || self.width > i32::MAX as u32
            || self.height > i32::MAX as u32
        {
            return Err(Error::InvalidParameter("invalid export dimensions"));
        }
        (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or(Error::NumericalOverflow)
    }
}

/// Selects the photographic endpoint, independent of the file settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderTarget {
    Display(OutputTarget),
    /// Render working AP1; convert to linear AP0 after reconstruction.
    SceneLinearAp0,
}

/// A supported pairing of rendering target and file settings.
#[derive(Debug, Clone, Copy)]
pub struct Plan {
    target: RenderTarget,
    format: FileFormat,
}

impl Plan {
    pub fn new(target: RenderTarget, format: FileFormat) -> Result<Self> {
        match (target, format.layout()) {
            (RenderTarget::Display(_), Layout::Rgb16)
            | (RenderTarget::SceneLinearAp0, Layout::Float(_)) => Ok(Self { target, format }),
            _ => Err(Error::InvalidParameter(
                "file layout does not support the render target",
            )),
        }
    }

    pub fn target(self) -> RenderTarget {
        self.target
    }

    pub fn format(self) -> FileFormat {
        self.format
    }

    pub(crate) fn write(
        self,
        (path, overwrite): (&Path, Overwrite),
        size: ImageSize,
        blocks: (BlockOrder, [u32; 2]),
        mut block: impl FnMut([u32; 2], [u32; 2]) -> Result<Samples>,
        before_publish: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        size.count()?;
        #[cfg(test)]
        let before_publish = {
            let mut before_publish = Some(before_publish);
            if let (RenderTarget::Display(target), Layout::Rgb16) =
                (self.target, self.format.layout())
                && let Some(result) = crate::export_timing::pixels::intercept(
                    size,
                    target,
                    blocks,
                    &mut |origin, extent| match block(origin, extent)? {
                        Samples::Rgb16(codes) => Ok(codes),
                        _ => Err(Error::InvalidParameter("expected RGB16 samples")),
                    },
                    || before_publish.take().expect("one completion check")(),
                )
            {
                return result;
            }
            before_publish.expect("uncaptured completion check")
        };
        atomic_file(
            path,
            overwrite,
            |file| {
                self.format
                    .encode(file, size, self.target, blocks, &mut block)
            },
            before_publish,
        )
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum Overwrite {
    #[default]
    Refuse,
    Confirmed,
}

fn failure(error: impl std::fmt::Display) -> Error {
    Error::Export(error.to_string())
}

fn atomic_file(
    path: &Path,
    overwrite: Overwrite,
    write: impl FnOnce(&mut fs::File) -> Result<()>,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent).map_err(failure)?;
    write(temporary.as_file_mut())?;
    timed!(DestinationFinalize, {
        temporary.as_file().sync_all().map_err(failure)?;
        before_publish()?;
        match overwrite {
            Overwrite::Confirmed => fs::rename(temporary.path(), path).map_err(failure)?,
            Overwrite::Refuse => {
                crate::sys::exclusive_rename(temporary.path(), path).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::AlreadyExists {
                        Error::DestinationExists
                    } else {
                        failure(error)
                    }
                })?
            }
        }
        Ok(())
    })
}

// Square units avoid orientation-dependent repeated work without a large fixed workspace.
pub(crate) const BLOCK: [u32; 2] = [128, 128];

#[derive(Clone, Copy)]
pub(crate) enum BlockOrder {
    RowMajor,
    Spatial2x2,
}

impl BlockOrder {
    fn group_side(self) -> u32 {
        match self {
            Self::RowMajor => 1,
            Self::Spatial2x2 => 2,
        }
    }
}

fn band_rows((order, block): (BlockOrder, [u32; 2])) -> u32 {
    block[1] * order.group_side()
}

pub(crate) fn ordered_blocks(
    size: ImageSize,
    (order, block): (BlockOrder, [u32; 2]),
) -> impl Iterator<Item = ([u32; 2], [u32; 2])> {
    let group = block.map(|side| side * order.group_side());
    (0..size.height)
        .step_by(group[1] as usize)
        .flat_map(move |gy| {
            (0..size.width)
                .step_by(group[0] as usize)
                .flat_map(move |gx| {
                    (gy..(gy + group[1]).min(size.height))
                        .step_by(block[1] as usize)
                        .flat_map(move |y| {
                            (gx..(gx + group[0]).min(size.width))
                                .step_by(block[0] as usize)
                                .map(move |x| {
                                    (
                                        [x, y],
                                        [
                                            block[0].min(size.width - x),
                                            block[1].min(size.height - y),
                                        ],
                                    )
                                })
                        })
                })
        })
}
