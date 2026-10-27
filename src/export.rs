use crate::export_timing::timed;
#[cfg(feature = "ocio-reference")]
use crate::rendering::CpuRenderer;
use crate::{Error, color::ap1_to_ap0, profiles, rendering::OutputTarget};
use exr::{meta::attribute::Chromaticities, prelude::*};
use glam::DVec3;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};
use tempfile::NamedTempFile;
use tiff::{
    encoder::{Compression, DeflateLevel, Predictor, TiffEncoder, colortype::RGB16},
    tags::Tag,
};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExrPrecision {
    Half,
    Full,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum Overwrite {
    #[default]
    Refuse,
    Confirmed,
}

struct Icc(&'static [u8]);

impl tiff::encoder::TiffValue for Icc {
    const BYTE_LEN: u8 = 1;
    const FIELD_TYPE: tiff::tags::Type = tiff::tags::Type::UNDEFINED;

    fn count(&self) -> usize {
        self.0.len()
    }

    fn data(&self) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(self.0)
    }
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
// both paths pack the same checked RGB16 frame and keep the same codec/metadata/safety policy.
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
    let count = size.count()?;
    #[cfg(test)]
    let mut before_publish = Some(before_publish);
    #[cfg(test)]
    if let Some(result) =
        crate::export_timing::pixels::intercept(size, target, order, &mut block, || {
            before_publish.take().expect("one completion check")()
        })
    {
        return result;
    }
    #[cfg(test)]
    let before_publish = before_publish.expect("uncaptured completion check");
    if count > (1024 * 1024 * 1024) / 6 {
        return Err(Error::Export(
            "TIFF RGB16 buffer exceeds the current 1 GiB export budget".into(),
        ));
    }
    atomic_file(
        path,
        overwrite,
        |file| {
            let mut buffered = BufWriter::new(file);
            {
                let mut encoder = TiffEncoder::new(&mut buffered)
                    .map_err(failure)?
                    .with_compression(Compression::Deflate(DeflateLevel::Fast))
                    .with_predictor(Predictor::Horizontal);
                let mut image = encoder
                    .new_image::<RGB16>(size.width, size.height)
                    .map_err(failure)?;
                image.rows_per_strip(8).map_err(failure)?;
                image
                    .encoder()
                    .write_tag(Tag::Unknown(34675), Icc(profiles::icc(target)))
                    .map_err(failure)?;
                image
                    .encoder()
                    .write_tag(Tag::Software, "Antitone 0.1")
                    .map_err(failure)?;
                // tiff 0.11.3 activates compression only in write_data, not write_strip.
                let mut samples = timed!(FrameAllocation, vec![0; count * 3]);
                for (origin, extent) in ordered_blocks(size, order) {
                    let encoded = block(origin, extent)?;
                    if encoded.len() != (extent[0] * extent[1]) as usize {
                        return Err(Error::InvalidParameter("invalid TIFF block length"));
                    }
                    timed!(FormatPack, {
                        for (i, pixel) in encoded.into_iter().enumerate() {
                            let x = origin[0] + i as u32 % extent[0];
                            let y = origin[1] + i as u32 / extent[0];
                            let index = (y as usize * size.width as usize + x as usize) * 3;
                            samples[index..index + 3].copy_from_slice(&pixel);
                        }
                    });
                }
                timed!(FileEncode, image.write_data(&samples).map_err(failure))?;
            }
            timed!(FileFlush, buffered.flush().map_err(failure))
        },
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
    let count = size.count()?;
    let bytes_per_pixel = match precision {
        ExrPrecision::Half => 6,
        ExrPrecision::Full => 12,
    };
    // One planar frame is retained by the high-level EXR writer; no unbounded allocation.
    if count > (1024 * 1024 * 1024) / bytes_per_pixel {
        return Err(Error::Export(
            "EXR sample buffer exceeds the current 1 GiB export budget".into(),
        ));
    }
    let mut samples: [FlatSamples; 3] = timed!(
        FrameAllocation,
        std::array::from_fn(|_| match precision {
            ExrPrecision::Half => FlatSamples::F16(vec![half::f16::ZERO; count]),
            ExrPrecision::Full => FlatSamples::F32(vec![0.; count]),
        })
    );
    let conversion = ap1_to_ap0();
    for (origin, extent) in blocks(size) {
        let values = block(origin, extent)?;
        if values.len() != (extent[0] * extent[1]) as usize {
            return Err(Error::InvalidParameter("invalid EXR block length"));
        }
        timed!(FormatPack, {
            for (i, pixel) in values.into_iter().enumerate() {
                let x = origin[0] + i as u32 % extent[0];
                let y = origin[1] + i as u32 / extent[0];
                let index = y as usize * size.width as usize + x as usize;
                let ap0 = conversion * pixel;
                for (channel, value) in ap0.to_array().into_iter().enumerate() {
                    if !value.is_finite() {
                        return Err(Error::NonFinitePixel { channel });
                    }
                    match &mut samples[channel] {
                        FlatSamples::F16(values) => {
                            if value.abs() > 65504.0 {
                                return Err(Error::Export(
                                    "AP0 output exceeds finite half-float range; choose full-float EXR"
                                        .into(),
                                ));
                            }
                            values[index] = half::f16::from_f64(value);
                        }
                        FlatSamples::F32(values) => {
                            if value.abs() > f64::from(f32::MAX) {
                                return Err(Error::NumericalOverflow);
                            }
                            values[index] = value as f32;
                        }
                        _ => unreachable!(),
                    }
                }
            }
            Ok::<_, Error>(())
        })?;
    }
    let channels = AnyChannels::sort(
        ["R", "G", "B"]
            .into_iter()
            .zip(samples)
            .map(|(name, values)| AnyChannel::new(name, values))
            .collect(),
    );
    let attributes = LayerAttributes::named("scene-linear AP0");
    let layer = Layer::new(
        (size.width as usize, size.height as usize),
        attributes,
        Encoding {
            compression: exr::compression::Compression::ZIP16,
            blocks: Blocks::ScanLines,
            line_order: LineOrder::Increasing,
        },
        channels,
    );
    let mut image = Image::from_layer(layer);
    image.attributes.chromaticities = Some(Chromaticities {
        red: Vec2(0.7347, 0.2653),
        green: Vec2(0.0, 1.0),
        blue: Vec2(0.0001, -0.0770),
        white: Vec2(0.32168, 0.33767),
    });
    atomic_file(
        path,
        overwrite,
        |file| {
            timed!(
                FileEncode,
                image
                    .write()
                    .non_parallel()
                    .to_unbuffered(file)
                    .map_err(failure)
            )
        },
        before_publish,
    )
}
