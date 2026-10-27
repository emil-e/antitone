//! Bounded, colour-managed camera-preview data shared by display and source preparation.
use image::{ImageDecoder, metadata::Orientation};

#[cfg(test)]
mod tests;

pub(crate) struct Preview {
    pub rgb: image::RgbImage,
    pub capture_flip: i32,
}

impl Preview {
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
    if actual != *expected {
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
        if profile.is_none() && thumbnail.color_space != 1 {
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
    if profile.is_none() && thumbnail.color_space != 1 {
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
    if let Some(profile) = profile {
        let input = lcms2::Profile::new_icc(&profile).map_err(|e| e.to_string())?;
        let output = lcms2::Profile::new_srgb();
        let transform = lcms2::Transform::<u8, u8>::new(
            &input,
            lcms2::PixelFormat::RGB_8,
            &output,
            lcms2::PixelFormat::RGB_8,
            lcms2::Intent::RelativeColorimetric,
        )
        .map_err(|e| e.to_string())?;
        transform.transform_in_place(rgb.as_mut());
    }
    Ok(Some(Preview {
        rgb,
        capture_flip: thumbnail.flip,
    }))
}
