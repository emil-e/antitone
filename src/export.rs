use crate::{
    Error,
    color::ap1_to_ap0,
    processing::Response,
    profiles,
    rendering::{CpuRenderer, OutputTarget},
};
use exr::{meta::attribute::Chromaticities, prelude::*};
use glam::DVec3;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};
use tempfile::NamedTempFile;
use tiff::{
    encoder::{Compression, Predictor, TiffEncoder, colortype::RGB16},
    tags::Tag,
};

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
#[derive(Debug, Clone, Copy)]
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
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent).map_err(failure)?;
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all().map_err(failure)?;
    match overwrite {
        Overwrite::Confirmed => fs::rename(temporary.path(), path).map_err(failure)?,
        Overwrite::Refuse => exclusive_rename(temporary.path(), path)?,
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn exclusive_rename(from: &Path, to: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let from = CString::new(from.as_os_str().as_bytes()).map_err(failure)?;
    let to = CString::new(to.as_os_str().as_bytes()).map_err(failure)?;
    // Same-directory temporary output; RENAME_EXCL atomically refuses a competing destination.
    if unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) } != 0 {
        return Err(failure(std::io::Error::last_os_error()));
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn exclusive_rename(_from: &Path, _to: &Path) -> Result<()> {
    Err(Error::Export(
        "exclusive atomic export is implemented only on macOS".into(),
    ))
}

pub fn tiff(
    path: &Path,
    size: ImageSize,
    target: OutputTarget,
    overwrite: Overwrite,
    response: Response,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    let renderer = CpuRenderer::new(target)?;
    tiff_encoded(path, size, target, overwrite, response, |y, end| {
        let mut working = Vec::with_capacity(size.width as usize * (end - y) as usize);
        for row in y..end {
            for x in 0..size.width {
                working.push(pixel(x, row)?);
            }
        }
        target.encode_u16(&renderer.render(&working)?)
    })
}

pub fn tiff_composed(
    path: &Path,
    composition: crate::geometry::Composition,
    target: OutputTarget,
    overwrite: Overwrite,
    response: Response,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    let [width, height] = composition.size();
    let size = ImageSize { width, height };
    if composition.is_exact() {
        return tiff(path, size, target, overwrite, response, |x, y| {
            let [sx, sy] = composition.source_pixel([x, y])?;
            pixel(sx, sy)
        });
    }
    let raster = crate::frame_raster::FrameRaster::full(composition)?;
    let renderer = CpuRenderer::new(target)?;
    tiff_encoded(path, size, target, overwrite, response, |y, end| {
        let mut working = Vec::new();
        let mut weights = Vec::new();
        let mut groups = Vec::new();
        for row in y..end {
            for x in 0..width {
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

pub fn exr_composed(
    path: &Path,
    composition: crate::geometry::Composition,
    precision: ExrPrecision,
    overwrite: Overwrite,
    response: Response,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    let [width, height] = composition.size();
    let raster = crate::frame_raster::FrameRaster::full(composition)?;
    exr(
        path,
        ImageSize { width, height },
        precision,
        overwrite,
        response,
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

fn tiff_encoded(
    path: &Path,
    size: ImageSize,
    target: OutputTarget,
    overwrite: Overwrite,
    response: Response,
    mut strip: impl FnMut(u32, u32) -> Result<Vec<[u16; 3]>>,
) -> Result<()> {
    response.shape()?;
    let count = size.count()?;
    if count > (1024 * 1024 * 1024) / 6 {
        return Err(Error::Export(
            "TIFF RGB16 buffer exceeds the current 1 GiB export budget".into(),
        ));
    }
    atomic_file(path, overwrite, |file| {
        let mut buffered = BufWriter::new(file);
        {
            let mut encoder = TiffEncoder::new(&mut buffered)
                .map_err(failure)?
                .with_compression(Compression::Lzw)
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
            let model = response.processing_model();
            let mut description = format!(
                "Antitone model {model}; native-camera RGB inversion to AP1; ACES 2 / OCIO 2.6.0; {target:?} v1 / 100-nit SDR; rendering and encoding in matching target RGB"
            );
            if let Response::Film { toe, shoulder } = response {
                description.push_str(&format!(
                    "; film toe={toe}; shoulder={shoulder}; signed-root continuation above base"
                ));
            }
            image
                .encoder()
                .write_tag(Tag::ImageDescription, description.as_str())
                .map_err(failure)?;
            // tiff 0.11.3 activates compression only in write_data, not write_strip.
            let mut samples = Vec::with_capacity(count * 3);
            for y in (0..size.height).step_by(8) {
                let end = (y + 8).min(size.height);
                let encoded = strip(y, end)?;
                samples.extend(encoded.into_iter().flatten());
            }
            image.write_data(&samples).map_err(failure)?;
        }
        buffered.flush().map_err(failure)
    })
}

pub fn exr(
    path: &Path,
    size: ImageSize,
    precision: ExrPrecision,
    overwrite: Overwrite,
    response: Response,
    mut pixel: impl FnMut(u32, u32) -> Result<DVec3>,
) -> Result<()> {
    response.shape()?;
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
    let mut samples: [FlatSamples; 3] = std::array::from_fn(|_| match precision {
        ExrPrecision::Half => FlatSamples::F16(Vec::with_capacity(count)),
        ExrPrecision::Full => FlatSamples::F32(Vec::with_capacity(count)),
    });
    let conversion = ap1_to_ap0();
    for y in 0..size.height {
        for x in 0..size.width {
            let ap0 = conversion * pixel(x, y)?;
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
                        values.push(half::f16::from_f64(value));
                    }
                    FlatSamples::F32(values) => {
                        if value.abs() > f64::from(f32::MAX) {
                            return Err(Error::NumericalOverflow);
                        }
                        values.push(value as f32);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
    let channels = AnyChannels::sort(
        ["R", "G", "B"]
            .into_iter()
            .zip(samples)
            .map(|(name, values)| AnyChannel::new(name, values))
            .collect(),
    );
    let mut attributes = LayerAttributes::named("scene-linear AP0");
    attributes.other.insert(
        Text::from("antitoneProcessingModel"),
        AttributeValue::I32(response.processing_model() as i32),
    );
    attributes.other.insert(
        Text::from("antitoneInversionSpace"),
        AttributeValue::Text(Text::from(
            "native-camera RGB; post-inversion metadata camera-to-AP1",
        )),
    );
    if let Response::Film { toe, shoulder } = response {
        attributes.other.insert(
            Text::from("antitoneFilmResponse"),
            AttributeValue::Text(Text::from(
                format!("toe={toe}; shoulder={shoulder}; signed-root continuation above base")
                    .as_str(),
            )),
        );
    }
    attributes.other.insert(
        Text::from("antitoneColorSpace"),
        AttributeValue::Text(Text::from(
            "linear AP0 / ACES2065-1 primaries, D60; no output rendering",
        )),
    );
    attributes.other.insert(
        Text::from("antitoneExposureConvention"),
        AttributeValue::Text(Text::from(
            "2^(roll stops + photo stops), applied before photographic Bradford WB",
        )),
    );
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
    atomic_file(path, overwrite, |file| {
        image
            .write()
            .non_parallel()
            .to_unbuffered(file)
            .map_err(failure)
    })
}
