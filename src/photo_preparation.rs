//! Worker-side source preparation. Acquisition resolves Digital exposure once;
//! AP1 production uses the completed normalization without IO or re-metering accepted edits.
use crate::{
    Error,
    embedded_preview::{self, Preview},
    exposure_reference::{self, ReferenceExposure, UnavailableReference as Unavailable},
    processing::{InputSettings, exposure_multiplier},
    project::SourceDevelopment,
    scan::{CameraPixels, CameraProxy, DevelopedSource, Scan},
    source::FileKey,
    source::{SourceCache, SourceKey},
};
use std::sync::{Arc, OnceLock};

pub(crate) struct PreparedSource {
    pub source: Arc<DevelopedSource>,
    pub reference: Option<ReferenceExposure>,
    pub proxy: Option<Arc<CameraPixels>>,
}

impl PreparedSource {
    pub fn scan(&self) -> Scan {
        Scan::with_reference(self.source.clone(), self.reference.clone())
    }
}

pub(crate) fn native(
    store: Option<&SourceCache>,
    reference: &OnceLock<ReferenceExposure>,
    input: InputSettings,
    source: &Arc<DevelopedSource>,
) -> Result<PreparedSource, Error> {
    if input.is_negative() {
        return Ok(PreparedSource {
            source: source.clone(),
            reference: None,
            proxy: None,
        });
    }
    let development = SourceDevelopment::from_metadata(&source.metadata(), source.fixed_wb())?;
    let key = SourceKey::from_revision(&source.file.path, source.file.clone(), &development);
    let mut sampled = None;
    let value = reference.get_or_init(|| {
        resolve(store, &source.file.path, &source.file, &development, || {
            let proxy = key
                .as_ref()
                .and_then(|key| store.and_then(|store| store.get_proxy(key)))
                .map_or_else(
                    || CameraPixels::from_scan(&Scan::from_source(source.clone())).map(Arc::new),
                    Ok,
                )?;
            sampled = Some(proxy.clone());
            Ok(proxy)
        })
    });
    if let (Some(store), Some(key), Some(proxy)) = (store, key, &sampled) {
        store.publish_proxy(key, proxy.clone())?;
    }
    Ok(PreparedSource {
        source: source.clone(),
        reference: Some(value.clone()),
        proxy: sampled,
    })
}

pub(crate) fn proxy(
    store: Option<&SourceCache>,
    reference: &OnceLock<ReferenceExposure>,
    path: &std::path::Path,
    file: &FileKey,
    development: &SourceDevelopment,
    input: InputSettings,
    proxy: &Arc<CameraPixels>,
) -> CameraProxy {
    let value = (!input.is_negative()).then(|| {
        reference
            .get_or_init(|| resolve(store, path, file, development, || Ok(proxy.clone())))
            .clone()
    });
    CameraProxy::new(proxy.clone(), value)
}

fn resolve(
    store: Option<&SourceCache>,
    path: &std::path::Path,
    file: &FileKey,
    development: &SourceDevelopment,
    sample: impl FnOnce() -> Result<Arc<CameraPixels>, Error>,
) -> ReferenceExposure {
    let read = || embedded_preview::read(path, development);
    let preview = match store.map_or_else(read, |store| store.preview(file, read)) {
        Ok(Some(preview)) => preview,
        Ok(None) => return ReferenceExposure::Unavailable(Unavailable::NoUsablePreview),
        Err(reason) => {
            log::debug!(reason = reason.as_str(); "digital_preview_reference_unavailable");
            return ReferenceExposure::Unavailable(Unavailable::ReadFailed);
        }
    };
    match sample() {
        Ok(proxy) => estimate(&proxy, &preview, development.input_floor),
        Err(reason) => {
            log::debug!(reason = reason.to_string().as_str(); "digital_reference_sampling_unavailable");
            ReferenceExposure::Unavailable(Unavailable::ReadFailed)
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "ocio-reference"))]
mod photographic;

fn estimate(proxy: &CameraPixels, preview: &Preview, floor: f64) -> ReferenceExposure {
    let unavailable = |reason| ReferenceExposure::Unavailable(reason);
    let native = match preview.capture_flip {
        0 | 3 => [proxy.width, proxy.height],
        5 | 6 => [proxy.height, proxy.width],
        _ => return unavailable(Unavailable::UnsupportedOrientation),
    };
    let aspect = f64::from(native[0]) / f64::from(native[1]);
    let Some([left, top, width, height]) = reference_frame(&preview.rgb, aspect) else {
        return unavailable(Unavailable::IncomparableFraming);
    };
    let grid = [width.min(native[0]), height.min(native[1])]
        .map(|side| ((f64::from(side) * 0.8).floor() as u32).clamp(1, 64));
    let weights = crate::color::AP1_TO_XYZ.transpose().y_axis;
    let linear = |code: u8| {
        let encoded = f64::from(code) / 255.;
        if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        }
    };
    let samples = (0..grid[1])
        .flat_map(|y| (0..grid[0]).map(move |x| (x, y)))
        .filter_map(|(x, y)| {
            let u = 0.1 + 0.8 * (f64::from(x) + 0.5) / f64::from(grid[0]);
            let v = 0.1 + 0.8 * (f64::from(y) + 0.5) / f64::from(grid[1]);
            let (sx, sy) = match preview.capture_flip {
                0 => (u, v),
                3 => (1. - u, 1. - v),
                5 => (1. - v, u),
                6 => (v, 1. - u),
                _ => unreachable!(),
            };
            let pixel = proxy.pixels[(sy * f64::from(proxy.height)) as usize
                * proxy.width as usize
                + (sx * f64::from(proxy.width)) as usize];
            let camera = glam::DVec3::new(
                f64::from(pixel[0]),
                f64::from(pixel[1]),
                f64::from(pixel[2]),
            ) / 65535.;
            let raw_y = weights.dot(proxy.matrix * camera);
            if raw_y < floor {
                return None;
            }
            let rgb = preview
                .rgb
                .get_pixel(
                    left + (u * f64::from(width)) as u32,
                    top + (v * f64::from(height)) as u32,
                )
                .0;
            let target_y =
                0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2]);
            Some((raw_y, target_y))
        });
    match exposure_reference::estimate(samples) {
        Some(value) if exposure_multiplier(value.ev).is_ok() => ReferenceExposure::Matched(value),
        Some(_) => unavailable(Unavailable::UnsupportedExposure),
        None => unavailable(Unavailable::InsufficientSamples),
    }
}

// Establish a centered letterbox viewport from source aspect, checking only the
// proposed margins so photographic black content cannot enlarge the removed border.
fn reference_frame(image: &image::RgbImage, aspect: f64) -> Option<[u32; 4]> {
    let [width, height] = [image.width(), image.height()];
    let matches = |w: u32, h: u32| {
        w > 0 && h > 0 && (aspect / (f64::from(w) / f64::from(h))).ln().abs() <= 0.05
    };
    if matches(width, height) {
        return Some([0, 0, width, height]);
    }
    let black = |pixel: &image::Rgb<u8>| pixel.0.iter().all(|v| *v <= 2);
    let inner_height = (f64::from(width) / aspect).round() as u32;
    if inner_height < height {
        let top = (height - inner_height) / 2;
        let bottom = top + inner_height;
        if top > 0
            && height - inner_height <= height / 2
            && matches(width, inner_height)
            && (0..height)
                .filter(|y| *y < top || *y >= bottom)
                .all(|y| (0..width).all(|x| black(image.get_pixel(x, y))))
        {
            return Some([0, top, width, inner_height]);
        }
    }
    let inner_width = (f64::from(height) * aspect).round() as u32;
    if inner_width < width {
        let left = (width - inner_width) / 2;
        let right = left + inner_width;
        if left > 0
            && width - inner_width <= width / 2
            && matches(inner_width, height)
            && (0..width)
                .filter(|x| *x < left || *x >= right)
                .all(|x| (0..height).all(|y| black(image.get_pixel(x, y))))
        {
            return Some([left, 0, inner_width, height]);
        }
    }
    None
}
