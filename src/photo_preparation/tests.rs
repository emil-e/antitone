use super::*;
#[cfg(feature = "ocio-reference")]
use crate::source::SourceCache;

fn flat(width: u32, height: u32) -> CameraPixels {
    CameraPixels {
        identity: None,
        width,
        height,
        matrix: glam::DMat3::IDENTITY,
        pixels: vec![[0.18 * 65535., 0.18 * 65535., 0.18 * 65535., 0.]; (width * height) as usize]
            .into(),
    }
}

#[test]
fn unsupported_geometry_and_tiny_or_dark_references_abstain_instead_of_inventing_samples() {
    let proxy = flat(96, 64);
    let mut preview =
        Preview::srgb_fixture(image::RgbImage::from_pixel(4, 4, image::Rgb([89; 3])), 0);
    assert_eq!(
        estimate(&proxy, &preview, 1. / 65535.),
        ReferenceExposure::Unavailable(Unavailable::IncomparableFraming)
    );
    preview.rgb = image::RgbImage::from_pixel(6, 4, image::Rgb([89; 3]));
    assert_eq!(
        estimate(&proxy, &preview, 1. / 65535.),
        ReferenceExposure::Unavailable(Unavailable::InsufficientSamples)
    );
    preview.rgb = image::RgbImage::new(96, 64);
    assert_eq!(
        estimate(&proxy, &preview, 1. / 65535.),
        ReferenceExposure::Unavailable(Unavailable::InsufficientSamples)
    );
    preview.capture_flip = 9;
    assert_eq!(
        estimate(&proxy, &preview, 1. / 65535.),
        ReferenceExposure::Unavailable(Unavailable::UnsupportedOrientation)
    );
}

#[test]
fn negative_preparation_does_not_require_a_file_preview_or_an_exposure_memo() {
    let proxy_data = Arc::new(flat(96, 64));
    let development = development();
    let prepared = proxy(
        &OnceLock::new(),
        &development,
        InputSettings::default(),
        &proxy_data,
        Ok(None),
    );
    assert!(prepared.reference.is_none());
}

fn development() -> SourceDevelopment {
    SourceDevelopment {
        decode: crate::project::Development::new(crate::raw::version(), [1.; 4]),
        lens_correction: None,
        dng_development: None,
        dimensions: [96, 64],
        active_area: [0, 0, 96, 64],
        camera_model: "controlled".into(),
        camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
        input_floor: 1. / 65535.,
    }
}

#[cfg(feature = "ocio-reference")]
fn control(desired: f64, adobe: bool) -> (CameraPixels, Preview) {
    use crate::rendering::{CpuRenderer, OutputTarget};
    let mut proxy = flat(96, 64);
    for y in 0..64 {
        for x in 0..96 {
            let value = if x < 16 {
                0.0002
            } else if x > 80 {
                8.
            } else {
                0.04 + x as f64 * 0.001 + y as f64 * 0.002
            };
            Arc::make_mut(&mut proxy.pixels)[y * 96 + x] = [
                (value * 65535.) as f32,
                (value * 65535.) as f32,
                (value * 65535.) as f32,
                0.,
            ];
        }
    }
    let source = proxy
        .pixels
        .iter()
        .map(|p| glam::DVec3::splat(f64::from(p[0]) / 65535. * desired.exp2()))
        .collect::<Vec<_>>();
    let rendered = CpuRenderer::new(OutputTarget::Srgb)
        .unwrap()
        .render(&source)
        .unwrap();
    let data = rendered
        .into_iter()
        .flat_map(|pixel| {
            pixel.to_array().map(|v| {
                let encoded = if adobe {
                    f64::from(v).powf(256. / 563.)
                } else {
                    OutputTarget::Srgb.encode(f64::from(v)).unwrap()
                };
                (encoded * 255.).round() as u8
            })
        })
        .collect();
    let preview = embedded_preview::decode(crate::raw::EmbeddedThumbnail {
        data,
        rgb_dimensions: Some([96, 64]),
        flip: 0,
        color_space: if adobe { 2 } else { 1 },
    })
    .unwrap()
    .unwrap();
    (proxy, preview)
}

#[cfg(feature = "ocio-reference")]
#[test]
fn quantized_reference_matches_known_gain_through_rotations_and_symmetric_letterboxing() {
    let desired = 0.7;
    let (proxy, reference) = control(desired, false);
    for flip in [0, 3, 5, 6] {
        let rgb = match flip {
            0 => reference.rgb.clone(),
            3 => image::imageops::rotate180(&reference.rgb),
            5 => image::imageops::rotate270(&reference.rgb),
            6 => image::imageops::rotate90(&reference.rgb),
            _ => unreachable!(),
        };
        let mut boxed = image::RgbImage::new(rgb.width(), rgb.height() + 20);
        image::imageops::replace(&mut boxed, &rgb, 0, 10);
        for rgb in [rgb, boxed] {
            let result = estimate(&proxy, &Preview::srgb_fixture(rgb, flip), 1. / 65535.);
            let ReferenceExposure::Matched(value) = result else {
                panic!("flip={flip} {result:?}");
            };
            assert!((value.ev - desired).abs() < 0.025, "flip={flip} {value:?}");
        }
    }
}

#[cfg(feature = "ocio-reference")]
#[test]
fn native_adobe_preview_recovers_known_gain_through_the_same_aces_inverse() {
    let desired = 0.7;
    let (proxy, preview) = control(desired, true);
    let ReferenceExposure::Matched(value) = estimate(&proxy, &preview, 1. / 65535.) else {
        panic!("the authored Adobe RGB preview must provide usable samples");
    };
    // Same neutral ACES response and 8-bit error budget as the sRGB control above.
    assert!((value.ev - desired).abs() < 0.025, "{value:?}");
}

#[test]
fn a_recorded_unavailable_baseline_is_not_replaced_when_preview_support_improves() {
    let data = Arc::new(flat(96, 64));
    let preview = Arc::new(Preview::srgb_fixture(
        image::RgbImage::from_pixel(96, 64, image::Rgb([128; 3])),
        0,
    ));
    assert!(matches!(
        estimate(&data, &preview, 1. / 65535.),
        ReferenceExposure::Matched(_)
    ));
    let recorded = ReferenceExposure::Unavailable(Unavailable::NoUsablePreview);
    let memo = OnceLock::from(recorded.clone());
    let prepared = proxy(
        &memo,
        &development(),
        InputSettings::Digital,
        &data,
        Ok(Some(preview)),
    );
    assert_eq!(prepared.reference, Some(recorded));
}

#[cfg(feature = "ocio-reference")]
#[test]
fn source_exposure_survives_pixel_eviction_and_does_not_re_meter_a_later_proxy() {
    let path = crate::fixture_path::FixturePath::write(
        "source.raw",
        b"not a RAW: repeated extraction must not occur",
    );
    let file = FileKey::new(path.path());
    let store = Arc::new(SourceCache::new(1 << 20));
    let (data, reference) = control(0.7, false);
    store
        .publish_preview(file.clone(), Arc::new(reference))
        .unwrap();
    let data = Arc::new(data);
    let memo = OnceLock::new();
    let prepared = proxy(
        &memo,
        &development(),
        InputSettings::Digital,
        &data,
        Ok(store.get_preview(&file)),
    );
    let initial = prepared.reference.unwrap();
    assert!((initial.ev() - 0.7).abs() < 0.025);
    store.set_budget(0);
    assert_eq!(store.charges().retained, 0);
    let changed = Arc::new(flat(96, 64));
    let prepared = proxy(
        &memo,
        &development(),
        InputSettings::Digital,
        &changed,
        Err("recorded exposure must not be recalculated".into()),
    );
    assert_eq!(prepared.reference.as_ref(), Some(&initial));
    assert_eq!(
        std::fs::read(path.path()).unwrap(),
        b"not a RAW: repeated extraction must not occur"
    );
}
