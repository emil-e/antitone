//! Bounded, colour-managed camera-preview data shared by display and source preparation.
use image::{ImageDecoder, metadata::Orientation};
#[cfg(any(feature = "editor", test))]
use std::borrow::Cow;

#[cfg(test)]
mod tests;

pub(crate) struct Preview {
    pub rgb: image::RgbImage,
    pub capture_flip: i32,
    color_space: ColorSpace,
}

#[derive(Clone, Copy)]
enum ColorSpace {
    Srgb,
    AdobeRgb,
}

impl ColorSpace {
    fn from_libraw(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::Srgb),
            2 => Some(Self::AdobeRgb),
            _ => None,
        }
    }

    fn luminance(self, rgb: [u8; 3]) -> f64 {
        let linear = rgb.map(|code| {
            let value = f64::from(code) / 255.;
            match self {
                Self::Srgb if value <= 0.04045 => value / 12.92,
                Self::Srgb => ((value + 0.055) / 1.055).powf(2.4),
                Self::AdobeRgb => value.powf(563. / 256.),
            }
        });
        let weights = match self {
            Self::Srgb => [0.2126, 0.7152, 0.0722],
            // D65 luminance row: https://drafts.csswg.org/css-color-4/conversions.js.
            Self::AdobeRgb => [591459. / 1989134., 6239551. / 9945670., 374412. / 4972835.],
        };
        weights[0] * linear[0] + weights[1] * linear[1] + weights[2] * linear[2]
    }
}

impl Preview {
    #[cfg(test)]
    pub(crate) fn srgb_fixture(rgb: image::RgbImage, capture_flip: i32) -> Self {
        Self {
            rgb,
            capture_flip,
            color_space: ColorSpace::Srgb,
        }
    }

    pub(crate) fn luminance(&self, rgb: [u8; 3]) -> f64 {
        self.color_space.luminance(rgb)
    }

    /// Display conversion is separate from metering, which preserves native-gamut luminance.
    #[cfg(any(feature = "editor", test))]
    pub(crate) fn srgb(&self) -> Result<Cow<'_, image::RgbImage>, String> {
        match self.color_space {
            ColorSpace::Srgb => Ok(Cow::Borrowed(&self.rgb)),
            ColorSpace::AdobeRgb => {
                let xy = |x, y| lcms2::CIExyY { x, y, Y: 1. };
                let primaries = lcms2::CIExyYTRIPLE {
                    Red: xy(0.64, 0.33),
                    Green: xy(0.21, 0.71),
                    Blue: xy(0.15, 0.06),
                };
                let curve = lcms2::ToneCurve::new(563. / 256.);
                let profile = lcms2::Profile::new_rgb(
                    &xy(0.3127, 0.3290),
                    &primaries,
                    &[&curve, &curve, &curve],
                )
                .map_err(|e| e.to_string())?;
                let mut rgb = self.rgb.clone();
                to_srgb(&mut rgb, &profile)?;
                Ok(Cow::Owned(rgb))
            }
        }
    }

    pub(crate) fn retained_bytes(&self) -> u64 {
        (std::mem::size_of::<Self>() + self.rgb.as_raw().capacity()) as u64
    }
}

pub(crate) fn read(
    path: &std::path::Path,
    expected: &crate::project::SourceDevelopment,
) -> Result<Option<std::sync::Arc<Preview>>, String> {
    let mut raw = crate::raw::RawFile::open_with_wb(path, Some(expected.decode.fixed_wb))
        .map_err(|e| e.to_string())?;
    let actual =
        crate::project::SourceDevelopment::from_metadata(&raw.metadata, expected.decode.fixed_wb)
            .map_err(|e| e.to_string())?;
    if !actual.same_processing(expected) {
        return Err("embedded preview source differs from the requested development".into());
    }
    let Some(thumbnail) = raw.embedded_thumbnail().map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    let mut preview = decode(thumbnail)?;
    if let Some(preview) = &mut preview {
        preview.capture_flip = raw.metadata.flip;
    }
    Ok(preview.map(std::sync::Arc::new))
}

pub(crate) fn decode(thumbnail: crate::raw::EmbeddedThumbnail) -> Result<Option<Preview>, String> {
    let declared = ColorSpace::from_libraw(thumbnail.color_space);
    let (mut image, profile, orientation) = if let Some([width, height]) = thumbnail.rgb_dimensions
    {
        if !matches!(width, 1..=8192)
            || !matches!(height, 1..=8192)
            || u64::from(width) * u64::from(height) * 3 > 96 * 1024 * 1024
        {
            return Err("embedded RGB preview dimensions exceed limits".into());
        }
        let rgb = image::RgbImage::from_raw(width, height, thumbnail.data)
            .ok_or("embedded RGB preview length mismatch")?;
        (image::DynamicImage::ImageRgb8(rgb), None, None)
    } else {
        let mut decoder =
            image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(thumbnail.data))
                .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(96 * 1024 * 1024);
        decoder.set_limits(limits).map_err(|e| e.to_string())?;
        let profile = decoder.icc_profile().map_err(|e| e.to_string())?;
        if profile.is_none() && declared.is_none() {
            return Ok(None);
        }
        let orientation = decoder
            .exif_metadata()
            .map_err(|e| e.to_string())?
            .as_deref()
            .and_then(Orientation::from_exif_chunk);
        (
            image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?,
            profile,
            orientation,
        )
    };
    if profile.is_none() && declared.is_none() {
        return Ok(None);
    }
    let orientation = match orientation {
        Some(orientation) => orientation,
        None => match thumbnail.flip {
            0 => Orientation::NoTransforms,
            3 => Orientation::Rotate180,
            5 => Orientation::Rotate270,
            6 => Orientation::Rotate90,
            _ => return Ok(None),
        },
    };
    image.apply_orientation(orientation);
    let edge = crate::scan::CameraProxy::MAX_EDGE;
    if image.width() > edge || image.height() > edge {
        image = image.thumbnail(edge, edge);
    }
    let mut rgb = image.into_rgb8();
    let color_space = if let Some(profile) = profile {
        let input = lcms2::Profile::new_icc(&profile).map_err(|e| e.to_string())?;
        to_srgb(&mut rgb, &input)?;
        ColorSpace::Srgb
    } else {
        declared.expect("unlabelled previews without ICC were excluded")
    };
    Ok(Some(Preview {
        rgb,
        capture_flip: thumbnail.flip,
        color_space,
    }))
}

fn to_srgb(rgb: &mut image::RgbImage, input: &lcms2::Profile) -> Result<(), String> {
    let output = lcms2::Profile::new_srgb();
    let transform = lcms2::Transform::<u8, u8>::new(
        input,
        lcms2::PixelFormat::RGB_8,
        &output,
        lcms2::PixelFormat::RGB_8,
        lcms2::Intent::RelativeColorimetric,
    )
    .map_err(|e| e.to_string())?;
    transform.transform_in_place(rgb.as_mut());
    Ok(())
}
