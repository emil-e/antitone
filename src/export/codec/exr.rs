//! OpenEXR block encoding and its direct-call adapters.
use super::{sample_mismatch, workspace};
use crate::{
    Error,
    export::{
        BLOCK, BlockOrder, ExrPrecision, FileFormat, FloatPrecision, ImageSize, Overwrite, Plan,
        RenderTarget, Result, Samples, SceneEncoding, failure, ordered_blocks,
    },
    export_timing::timed,
};
use exr::{
    block::{UncompressedBlock, writer::ChunksWriter},
    io::Data,
    meta::{
        BlockDescription,
        attribute::{ChannelDescription, Chromaticities, SampleType},
        header::Header,
    },
    prelude::*,
};
use glam::DVec3;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};

pub(super) fn encode(
    file: &mut fs::File,
    size: ImageSize,
    precision: FloatPrecision,
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Samples>,
) -> Result<()> {
    match precision {
        FloatPrecision::Half => write_blocks(file, size, &mut |origin, extent| {
            let samples = block(origin, extent)?;
            match samples {
                Samples::F16(codes) => Ok(codes),
                _ => Err(sample_mismatch()),
            }
        }),
        FloatPrecision::Full => write_blocks(file, size, &mut |origin, extent| {
            let samples = block(origin, extent)?;
            match samples {
                Samples::F32(codes) => Ok(codes),
                _ => Err(sample_mismatch()),
            }
        }),
    }
}

trait FloatSample: Data + Copy {
    const KIND: SampleType;
}

impl FloatSample for half::f16 {
    const KIND: SampleType = SampleType::F16;
}

impl FloatSample for f32 {
    const KIND: SampleType = SampleType::F32;
}

fn write_blocks<T: FloatSample>(
    file: &mut fs::File,
    size: ImageSize,
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[T; 3]>>,
) -> Result<()> {
    let count = workspace(
        size,
        BLOCK[1],
        3 * T::BYTE_SIZE as u64,
        u64::from(size.height.div_ceil(16)) * 16,
    )?;
    let mut samples: [Vec<T>; 3] = timed!(
        FrameAllocation,
        std::array::from_fn(|_| vec![T::default(); count])
    );

    let channels = ["B", "G", "R"]
        .into_iter()
        .map(|name| ChannelDescription::named(name, T::KIND))
        .collect();
    let mut header = Header::new(
        "scene-linear AP0".into(),
        (size.width as usize, size.height as usize),
        channels,
    )
    .with_encoding(
        exr::compression::Compression::ZIP16,
        BlockDescription::ScanLines,
        LineOrder::Increasing,
    );
    header.shared_attributes.chromaticities = Some(Chromaticities {
        red: Vec2(0.7347, 0.2653),
        green: Vec2(0., 1.),
        blue: Vec2(0.0001, -0.0770),
        white: Vec2(0.32168, 0.33767),
    });

    let mut producer_error = None;
    let mut buffered = BufWriter::new(file);
    let result = timed!(
        FileEncode,
        exr::block::write(
            &mut buffered,
            vec![header].into_iter().collect(),
            true,
            |meta, writer| {
                let mut compressor = writer.sequential_blocks_compressor(&meta);
                let mut band_top = None;
                for (index, placeholder) in meta.collect_ordered_blocks(|index| UncompressedBlock {
                    index,
                    data: Vec::new(),
                }) {
                    let location = placeholder.index;
                    let top = (location.pixel_position.y() as u32 / BLOCK[1]) * BLOCK[1];
                    if band_top != Some(top) {
                        let result = (|| {
                            let rows = BLOCK[1].min(size.height - top);
                            for (mut origin, extent) in ordered_blocks(
                                ImageSize {
                                    width: size.width,
                                    height: rows,
                                },
                                (BlockOrder::RowMajor, BLOCK),
                            ) {
                                origin[1] += top;
                                let values = block(origin, extent)?;
                                if values.len() != (extent[0] * extent[1]) as usize {
                                    return Err(Error::InvalidParameter(
                                        "invalid EXR block length",
                                    ));
                                }
                                timed!(FormatPack, {
                                    for (i, value) in values.into_iter().enumerate() {
                                        let x = origin[0] as usize + i % extent[0] as usize;
                                        let y = (origin[1] - top) as usize + i / extent[0] as usize;
                                        let pixel = y * size.width as usize + x;
                                        for (channel, value) in value.into_iter().enumerate() {
                                            samples[channel][pixel] = value;
                                        }
                                    }
                                });
                            }
                            Ok(())
                        })();
                        if let Err(error) = result {
                            producer_error = Some(error);
                            return Err(exr::error::Error::Aborted);
                        }
                        band_top = Some(top);
                    }
                    let mut line_error = None;
                    let raw = UncompressedBlock::from_lines(
                        &meta.headers[0].channels,
                        location,
                        |line| {
                            let channel = 2 - line.location.channel;
                            let row =
                                (line.location.position.y() - top as usize) * size.width as usize;
                            let result = line.write_samples_from_slice(
                                &samples[channel][row..row + size.width as usize],
                            );
                            if let Err(error) = result {
                                line_error = Some(error);
                            }
                        },
                    );
                    if let Some(error) = line_error {
                        return Err(error);
                    }
                    compressor.compress_block(index, raw)?;
                }
                Ok(())
            },
        )
    );

    if let Some(error) = producer_error {
        return Err(error);
    }
    result.map_err(failure)?;
    timed!(FileFlush, buffered.flush().map_err(failure))
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
        (BlockOrder::RowMajor, BLOCK),
        |origin, extent| encoding.pack(block(origin, extent)?.into_iter().map(Ok)),
        before_publish,
    )
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
