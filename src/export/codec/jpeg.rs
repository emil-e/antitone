//! JPEG encoding from bounded bands of target-encoded RGB8.
use super::{sample_mismatch, workspace};
use crate::{
    Error,
    export::{BlockOrder, ImageSize, Result, Samples, failure, ordered_blocks},
    export_timing::timed,
    profiles,
    rendering::OutputTarget,
};
use image::{GenericImageView, ImageEncoder, Rgb, codecs::jpeg::JpegEncoder};
use std::{
    cell::RefCell,
    fs::File,
    io::{self, BufWriter, Write},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegQuality(u8);

impl JpegQuality {
    pub const RANGE: std::ops::RangeInclusive<u8> = 1..=100;

    pub fn new(value: u8) -> Result<Self> {
        if !Self::RANGE.contains(&value) {
            return Err(Error::InvalidParameter(
                "JPEG quality must be within 1..=100",
            ));
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl Default for JpegQuality {
    fn default() -> Self {
        Self(95)
    }
}

pub(super) fn encode(
    file: &mut File,
    size: ImageSize,
    target: OutputTarget,
    quality: JpegQuality,
    block_size: [u32; 2],
    block: &mut impl FnMut([u32; 2], [u32; 2]) -> Result<Samples>,
) -> Result<()> {
    if size.width > u32::from(u16::MAX) || size.height > u32::from(u16::MAX) {
        return Err(Error::Export(
            "JPEG dimensions must not exceed 65,535 pixels per side".into(),
        ));
    }
    let count = workspace(size, block_size[1], 3, 0)?;
    let view = BandView {
        size,
        block_size,
        band: RefCell::new(Band {
            top: None,
            pixels: timed!(FrameAllocation, vec![[0; 3]; count]),
            block,
        }),
        producer_error: RefCell::new(None),
    };
    let mut output = BufWriter::new(file);
    let mut encoder = JpegEncoder::new_with_quality(
        StopOnProducerError {
            output: &mut output,
            producer_error: &view.producer_error,
        },
        quality.get(),
    );
    encoder
        .set_icc_profile(profiles::icc(target).to_vec())
        .map_err(failure)?;
    let result = timed!(FileEncode, encoder.encode_image(&view));
    if let Some(error) = view.producer_error.into_inner() {
        return Err(error);
    }
    result.map_err(failure)?;
    timed!(FileFlush, output.flush().map_err(failure))
}

struct Band<B> {
    top: Option<u32>,
    pixels: Vec<[u8; 3]>,
    block: B,
}

impl<B: FnMut([u32; 2], [u32; 2]) -> Result<Samples>> Band<B> {
    fn load(&mut self, size: ImageSize, block_size: [u32; 2], top: u32) -> Result<()> {
        if self.top.is_some_and(|previous| top < previous) {
            return Err(Error::Export(
                "JPEG encoder requested a retired row band".into(),
            ));
        }
        let band = ImageSize {
            width: size.width,
            height: block_size[1].min(size.height - top),
        };
        for ([x, y], extent) in ordered_blocks(band, (BlockOrder::RowMajor, block_size)) {
            let Samples::Rgb8(samples) = (self.block)([x, y + top], extent)? else {
                return Err(sample_mismatch());
            };
            let width = extent[0] as usize;
            if samples.len() != width * extent[1] as usize {
                return Err(Error::InvalidParameter("invalid JPEG block length"));
            }
            timed!(FormatPack, {
                for (row, samples) in samples.chunks_exact(width).enumerate() {
                    let start = (y as usize + row) * size.width as usize + x as usize;
                    self.pixels[start..start + width].copy_from_slice(samples);
                }
            });
        }
        self.top = Some(top);
        Ok(())
    }
}

// The pinned image encoder visits 8-row groups left to right and replicates edge pixels.
// The export block height is a multiple of eight, so no visit needs a previously retired band.
struct BandView<B> {
    size: ImageSize,
    block_size: [u32; 2],
    band: RefCell<Band<B>>,
    producer_error: RefCell<Option<Error>>,
}

impl<B: FnMut([u32; 2], [u32; 2]) -> Result<Samples>> GenericImageView for BandView<B> {
    type Pixel = Rgb<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.size.width, self.size.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let mut band = self.band.borrow_mut();
        let top = y / self.block_size[1] * self.block_size[1];
        if band.top != Some(top) {
            let mut error = self.producer_error.borrow_mut();
            if error.is_some() {
                return Rgb([0; 3]);
            }
            if let Err(failure) = band.load(self.size, self.block_size, top) {
                *error = Some(failure);
                return Rgb([0; 3]);
            }
        }
        Rgb(band.pixels[(y - top) as usize * self.size.width as usize + x as usize])
    }
}

// Pixel access is infallible in image's API. The next encoder write interrupts it;
// the original producer error is returned instead of the resulting I/O error.
struct StopOnProducerError<'a, W> {
    output: W,
    producer_error: &'a RefCell<Option<Error>>,
}

impl<W: Write> Write for StopOnProducerError<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.producer_error.borrow().is_some() {
            return Err(io::Error::other("JPEG pixel producer failed"));
        }
        self.output.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.output.flush()
    }
}

#[cfg(test)]
mod tests;
