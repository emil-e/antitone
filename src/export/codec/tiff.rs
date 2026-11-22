//! TIFF block encoding and its direct-call adapters.
use super::{sample_mismatch, workspace};
#[cfg(feature = "ocio-reference")]
use crate::rendering::CpuRenderer;
use crate::{
    Error,
    export::{
        BLOCK, BlockOrder, FileFormat, ImageSize, Overwrite, Plan, RenderTarget, Result, Samples,
        band_rows, failure, ordered_blocks,
    },
    export_timing::timed,
    profiles,
    rendering::OutputTarget,
};
#[cfg(feature = "ocio-reference")]
use glam::DVec3;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};
use tiff::{
    encoder::{
        Rational, TiffEncoder,
        compression::{CompressionAlgorithm, Deflate, DeflateLevel},
    },
    tags::{
        CompressionMethod, PhotometricInterpretation, Predictor, ResolutionUnit, SampleFormat, Tag,
    },
};

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

const STRIP_ROWS: u32 = 8;

pub(super) fn encode(
    file: &mut fs::File,
    size: ImageSize,
    target: OutputTarget,
    layout: (BlockOrder, [u32; 2]),
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Samples>,
) -> Result<()> {
    write_blocks(
        file,
        size,
        target,
        layout,
        &mut |origin, extent| match block(origin, extent)? {
            Samples::Rgb16(codes) => Ok(codes),
            _ => Err(sample_mismatch()),
        },
    )
}

fn write_blocks(
    file: &mut fs::File,
    size: ImageSize,
    target: OutputTarget,
    layout: (BlockOrder, [u32; 2]),
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>>,
) -> Result<()> {
    let band_rows = band_rows(layout);
    let count = workspace(
        size,
        band_rows,
        6,
        u64::from(size.height.div_ceil(STRIP_ROWS)) * 8,
    )?;
    let mut band = timed!(FrameAllocation, vec![0u16; count * 3]);

    let mut buffered = BufWriter::new(file);
    let mut encoder = TiffEncoder::new(&mut buffered).map_err(failure)?;
    // ImageEncoder::write_strip does not enable compression in tiff 0.11.3. The public
    // directory API lets each predicted strip use the same crate's Deflate implementation.
    let mut directory = encoder.image_directory().map_err(failure)?;
    directory
        .write_tag(Tag::ImageWidth, size.width)
        .map_err(failure)?;
    directory
        .write_tag(Tag::ImageLength, size.height)
        .map_err(failure)?;
    directory
        .write_tag(Tag::BitsPerSample, &[16u16; 3][..])
        .map_err(failure)?;
    directory
        .write_tag(Tag::Compression, CompressionMethod::Deflate.to_u16())
        .map_err(failure)?;
    directory
        .write_tag(Tag::Predictor, Predictor::Horizontal.to_u16())
        .map_err(failure)?;
    directory
        .write_tag(
            Tag::PhotometricInterpretation,
            PhotometricInterpretation::RGB.to_u16(),
        )
        .map_err(failure)?;
    directory
        .write_tag(Tag::SamplesPerPixel, 3u16)
        .map_err(failure)?;
    directory
        .write_tag(Tag::RowsPerStrip, STRIP_ROWS)
        .map_err(failure)?;
    directory
        .write_tag(Tag::SampleFormat, &[SampleFormat::Uint.to_u16(); 3][..])
        .map_err(failure)?;
    directory
        .write_tag(Tag::XResolution, Rational { n: 1, d: 1 })
        .map_err(failure)?;
    directory
        .write_tag(Tag::YResolution, Rational { n: 1, d: 1 })
        .map_err(failure)?;
    directory
        .write_tag(Tag::ResolutionUnit, ResolutionUnit::None)
        .map_err(failure)?;
    directory
        .write_tag(Tag::Unknown(34675), Icc(profiles::icc(target)))
        .map_err(failure)?;
    directory
        .write_tag(Tag::Software, "Antitone 0.1")
        .map_err(failure)?;

    let strips = size.height.div_ceil(STRIP_ROWS) as usize;
    let mut offsets = Vec::with_capacity(strips);
    let mut lengths = Vec::with_capacity(strips);
    let mut predicted = Vec::new();
    let mut compressed = Vec::new();

    for top in (0..size.height).step_by(band_rows as usize) {
        let rows = band_rows.min(size.height - top);
        for (mut origin, extent) in ordered_blocks(
            ImageSize {
                width: size.width,
                height: rows,
            },
            layout,
        ) {
            origin[1] += top;
            let values = block(origin, extent)?;
            if values.len() != (extent[0] * extent[1]) as usize {
                return Err(Error::InvalidParameter("invalid TIFF block length"));
            }
            timed!(FormatPack, {
                for (i, value) in values.into_iter().enumerate() {
                    let x = origin[0] as usize + i % extent[0] as usize;
                    let y = (origin[1] - top) as usize + i / extent[0] as usize;
                    let index = (y * size.width as usize + x) * 3;
                    band[index..index + 3].copy_from_slice(&value);
                }
            });
        }

        for row in (0..rows).step_by(STRIP_ROWS as usize) {
            let rows = STRIP_ROWS.min(rows - row);
            predicted.clear();
            for y in row..row + rows {
                let mut previous = [0u16; 3];
                for pixel in band[y as usize * size.width as usize * 3
                    ..(y + 1) as usize * size.width as usize * 3]
                    .as_chunks::<3>()
                    .0
                {
                    for channel in 0..3 {
                        predicted.extend_from_slice(
                            &pixel[channel].wrapping_sub(previous[channel]).to_ne_bytes(),
                        );
                        previous[channel] = pixel[channel];
                    }
                }
            }
            compressed.clear();
            timed!(
                FileEncode,
                Deflate::with_level(DeflateLevel::Fast)
                    .write_to(&mut compressed, &predicted)
                    .map_err(failure)
            )?;
            let offset = directory
                .write_data(compressed.as_slice())
                .map_err(failure)?;
            offsets.push(
                u32::try_from(offset)
                    .map_err(|_| Error::Export("TIFF offset exceeds classic TIFF range".into()))?,
            );
            lengths.push(u32::try_from(compressed.len()).map_err(|_| Error::NumericalOverflow)?);
        }
    }

    directory
        .write_tag(Tag::StripOffsets, offsets.as_slice())
        .map_err(failure)?;
    directory
        .write_tag(Tag::StripByteCounts, lengths.as_slice())
        .map_err(failure)?;
    directory.finish().map_err(failure)?;
    timed!(FileFlush, buffered.flush().map_err(failure))
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
    tiff_encoded_checked(
        (path, overwrite),
        size,
        target,
        (order, BLOCK),
        block,
        || Ok(()),
    )
}

pub(crate) fn tiff_encoded_checked(
    (path, overwrite): (&Path, Overwrite),
    size: ImageSize,
    target: OutputTarget,
    layout: (BlockOrder, [u32; 2]),
    mut block: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>>,
    before_publish: impl FnOnce() -> Result<()>,
) -> Result<()> {
    Plan::new(RenderTarget::Display(target), FileFormat::Tiff)?.write(
        (path, overwrite),
        size,
        layout,
        |origin, extent| block(origin, extent).map(Samples::Rgb16),
        before_publish,
    )
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
