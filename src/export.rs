use crate::export_timing::timed;
#[cfg(feature = "ocio-reference")]
use crate::rendering::CpuRenderer;
use crate::{Error, rendering::OutputTarget};
use glam::DVec3;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};
use tempfile::NamedTempFile;
mod codec;
pub use codec::FileFormat;
mod samples;
pub use samples::{FloatPrecision, FloatPrecision as ExrPrecision};
pub(crate) use samples::{Layout, Samples, SceneEncoding};
mod stream;
#[cfg(all(test, target_os = "macos"))]
mod stream_tests;

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
        order: BlockOrder,
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
                    order,
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
                    .encode(file, size, self.target, order, &mut block)
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

/// Scalar/OCIO reference adapter, not a production rendering path.
#[cfg(feature = "ocio-reference")]
pub fn tiff(
    path: &Path,
    size: ImageSize,
    target: OutputTarget,
    overwrite: Overwrite,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    let renderer = CpuRenderer::new(target)?;
    tiff_encoded(path, size, target, overwrite, |origin, extent| {
        let mut working = Vec::with_capacity((extent[0] * extent[1]) as usize);
        for row in origin[1]..origin[1] + extent[1] {
            for x in origin[0]..origin[0] + extent[0] {
                working.push(pixel(x, row)?);
            }
        }
        target.encode_u16(&renderer.render(&working)?)
    })
}

/// Scalar/OCIO original-sample reconstruction oracle.
#[cfg(feature = "ocio-reference")]
pub fn tiff_composed(
    path: &Path,
    composition: crate::geometry::Composition,
    target: OutputTarget,
    overwrite: Overwrite,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    let [width, height] = composition.size();
    let size = ImageSize { width, height };
    if composition.is_exact() {
        return tiff(path, size, target, overwrite, |x, y| {
            let [sx, sy] = composition.source_pixel([x, y])?;
            pixel(sx, sy)
        });
    }
    let raster = crate::frame_raster::FrameRaster::full(composition)?;
    let renderer = CpuRenderer::new(target)?;
    tiff_encoded(path, size, target, overwrite, |origin, extent| {
        let mut working = Vec::new();
        let mut weights = Vec::new();
        let mut groups = Vec::new();
        for row in origin[1]..origin[1] + extent[1] {
            for x in origin[0]..origin[0] + extent[0] {
                let start = working.len();
                for ([sx, sy], weight) in raster.footprint([x, row])?.pixels(composition.area())? {
                    working.push(pixel(sx, sy)?);
                    weights.push(weight);
                }
                groups.push(start..working.len());
            }
        }
        // ACES is per original processed pixel; reconstruction is in linear target RGB.
        let rendered = renderer.render(&working)?;
        let linear = groups
            .into_iter()
            .map(|range| {
                crate::sampling::weighted_mean_iter(
                    range.map(|i| Ok((rendered[i].as_dvec3(), weights[i]))),
                )
                .map(|p| p.as_vec3())
            })
            .collect::<Result<Vec<_>>>()?;
        target.encode_u16(&linear)
    })
}

/// Scalar original-sample reconstruction oracle.
#[cfg(feature = "ocio-reference")]
pub fn exr_composed(
    path: &Path,
    composition: crate::geometry::Composition,
    precision: ExrPrecision,
    overwrite: Overwrite,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    let [width, height] = composition.size();
    let raster = crate::frame_raster::FrameRaster::full(composition)?;
    exr(
        path,
        ImageSize { width, height },
        precision,
        overwrite,
        |x, y| {
            if composition.is_exact() {
                let [sx, sy] = composition.source_pixel([x, y])?;
                return pixel(sx, sy);
            }
            crate::sampling::weighted_mean_iter(
                raster
                    .footprint([x, y])?
                    .pixels(composition.area())?
                    .map(|([sx, sy], weight)| pixel(sx, sy).map(|p| (p, weight))),
            )
        },
    )
}

// Square units avoid orientation-dependent repeated work without a large fixed workspace.
pub(crate) const BLOCK: [u32; 2] = [128, 128];

fn blocks(size: ImageSize) -> impl Iterator<Item = ([u32; 2], [u32; 2])> {
    (0..size.height)
        .step_by(BLOCK[1] as usize)
        .flat_map(move |y| {
            (0..size.width).step_by(BLOCK[0] as usize).map(move |x| {
                (
                    [x, y],
                    [BLOCK[0].min(size.width - x), BLOCK[1].min(size.height - y)],
                )
            })
        })
}

#[derive(Clone, Copy)]
pub(crate) enum BlockOrder {
    RowMajor,
    Spatial2x2,
}

pub(crate) fn ordered_blocks(
    size: ImageSize,
    order: BlockOrder,
) -> impl Iterator<Item = ([u32; 2], [u32; 2])> {
    let step = match order {
        BlockOrder::RowMajor => [1, 1],
        BlockOrder::Spatial2x2 => [2, 2],
    };
    let group = [BLOCK[0] * step[0], BLOCK[1] * step[1]];
    (0..size.height)
        .step_by(group[1] as usize)
        .flat_map(move |gy| {
            (0..size.width)
                .step_by(group[0] as usize)
                .flat_map(move |gx| {
                    (gy..(gy + group[1]).min(size.height))
                        .step_by(BLOCK[1] as usize)
                        .flat_map(move |y| {
                            (gx..(gx + group[0]).min(size.width))
                                .step_by(BLOCK[0] as usize)
                                .map(move |x| {
                                    (
                                        [x, y],
                                        [
                                            BLOCK[0].min(size.width - x),
                                            BLOCK[1].min(size.height - y),
                                        ],
                                    )
                                })
                        })
                })
        })
}

/// Encode supplied target RGB16 codes, with matching ICC and atomic destination safety.
/// The callback receives bounded block origin/extent and returns row-major codes.
/// This encoder performs no photographic processing or display rendering.
pub fn tiff_encoded(
    path: &Path,
    size: ImageSize,
    target: OutputTarget,
    overwrite: Overwrite,
    block: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>>,
) -> Result<()> {
    tiff_encoded_ordered(path, size, target, overwrite, BlockOrder::RowMajor, block)
}

// Only the internal GPU producer selects spatial visits. Public callbacks remain row-major;
// both paths pack bounded RGB16 row bands with the same codec/metadata/safety policy.
pub(crate) fn tiff_encoded_ordered(
    path: &Path,
    size: ImageSize,
    target: OutputTarget,
    overwrite: Overwrite,
    order: BlockOrder,
    block: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>>,
) -> Result<()> {
    tiff_encoded_checked((path, overwrite), size, target, order, block, || Ok(()))
}

pub(crate) fn tiff_encoded_checked(
    (path, overwrite): (&Path, Overwrite),
    size: ImageSize,
    target: OutputTarget,
    order: BlockOrder,
    mut block: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>>,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<()> {
    Plan::new(RenderTarget::Display(target), FileFormat::Tiff)?.write(
        (path, overwrite),
        size,
        order,
        |origin, extent| block(origin, extent).map(Samples::Rgb16),
        before_publish,
    )
}

pub fn exr(
    path: &Path,
    size: ImageSize,
    precision: ExrPrecision,
    overwrite: Overwrite,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    exr_blocks(path, size, precision, overwrite, |origin, extent| {
        let mut values = Vec::with_capacity((extent[0] * extent[1]) as usize);
        for row in origin[1]..origin[1] + extent[1] {
            for x in origin[0]..origin[0] + extent[0] {
                values.push(pixel(x, row)?);
            }
        }
        Ok(values)
    })
}

pub(crate) fn exr_blocks(
    path: &Path,
    size: ImageSize,
    precision: ExrPrecision,
    overwrite: Overwrite,
    block: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<DVec3>>,
) -> Result<()> {
    exr_blocks_checked((path, overwrite), size, precision, block, || Ok(()))
}

pub(crate) fn exr_blocks_checked(
    (path, overwrite): (&Path, Overwrite),
    size: ImageSize,
    precision: ExrPrecision,
    mut block: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<DVec3>>,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let encoding = SceneEncoding::new(precision);
    Plan::new(RenderTarget::SceneLinearAp0, FileFormat::Exr(precision))?.write(
        (path, overwrite),
        size,
        BlockOrder::RowMajor,
        |origin, extent| encoding.pack(block(origin, extent)?.into_iter().map(Ok)),
        before_publish,
    )
}
