//! Bounded codec workspaces; photographic processing remains with the block producer.
use super::*;
use super::{Error, Result};
use exr::{
    block::{UncompressedBlock, writer::ChunksWriter},
    meta::{
        BlockDescription,
        attribute::{ChannelDescription, Chromaticities, SampleType},
        header::Header,
    },
    prelude::*,
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

const STRIP_ROWS: u32 = 8;
const MAX_WORKSPACE: u64 = 1 << 30;

pub(super) fn workspace(size: ImageSize, rows: u32, bytes: u64, metadata: u64) -> Result<usize> {
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

pub(super) fn tiff(
    file: &mut fs::File,
    size: ImageSize,
    target: OutputTarget,
    order: BlockOrder,
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<[u16; 3]>>,
) -> Result<()> {
    let band_rows = match order {
        BlockOrder::RowMajor => BLOCK[1],
        BlockOrder::Spatial2x2 => 2 * BLOCK[1],
    };
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
            order,
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

pub(super) fn exr(
    file: &mut fs::File,
    size: ImageSize,
    precision: ExrPrecision,
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<DVec3>>,
) -> Result<()> {
    let bytes = match precision {
        ExrPrecision::Half => 6,
        ExrPrecision::Full => 12,
    };
    let count = workspace(
        size,
        BLOCK[1],
        bytes,
        u64::from(size.height.div_ceil(16)) * 16,
    )?;
    let mut samples: [FlatSamples; 3] = timed!(
        FrameAllocation,
        std::array::from_fn(|_| match precision {
            ExrPrecision::Half => FlatSamples::F16(vec![half::f16::ZERO; count]),
            ExrPrecision::Full => FlatSamples::F32(vec![0.; count]),
        })
    );
    let channels = ["B", "G", "R"]
        .into_iter()
        .map(|name| {
            ChannelDescription::named(
                name,
                match precision {
                    ExrPrecision::Half => SampleType::F16,
                    ExrPrecision::Full => SampleType::F32,
                },
            )
        })
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
    let conversion = ap1_to_ap0();
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
                            for (mut origin, extent) in blocks(ImageSize {
                                width: size.width,
                                height: rows,
                            }) {
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
                                        for (channel, value) in
                                            (conversion * value).to_array().into_iter().enumerate()
                                        {
                                            if !value.is_finite() {
                                                return Err(Error::NonFinitePixel { channel });
                                            }
                                            match &mut samples[channel] {
                                                FlatSamples::F16(values) => {
                                                    if value.abs() > 65504. {
                                                        return Err(Error::Export("AP0 output exceeds finite half-float range; choose full-float EXR".into()));
                                                    }
                                                    values[pixel] = half::f16::from_f64(value);
                                                }
                                                FlatSamples::F32(values) => {
                                                    if value.abs() > f64::from(f32::MAX) {
                                                        return Err(Error::NumericalOverflow);
                                                    }
                                                    values[pixel] = value as f32;
                                                }
                                                _ => unreachable!(),
                                            }
                                        }
                                    }
                                    Ok::<_, Error>(())
                                })?;
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
                            let result = match &samples[channel] {
                                FlatSamples::F16(values) => line.write_samples_from_slice(
                                    &values[row..row + size.width as usize],
                                ),
                                FlatSamples::F32(values) => line.write_samples_from_slice(
                                    &values[row..row + size.width as usize],
                                ),
                                _ => unreachable!(),
                            };
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
