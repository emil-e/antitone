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
        estimate(&proxy, &preview),
        ReferenceExposure::Unavailable(Unavailable::IncomparableFraming)
    );
    preview.rgb = image::RgbImage::from_pixel(6, 4, image::Rgb([89; 3]));
    assert_eq!(
        estimate(&proxy, &preview),
        ReferenceExposure::Unavailable(Unavailable::InsufficientSamples)
    );
    preview.rgb = image::RgbImage::new(96, 64);
    assert_eq!(
        estimate(&proxy, &preview),
        ReferenceExposure::Unavailable(Unavailable::InsufficientSamples)
    );
    preview.capture_flip = 9;
    assert_eq!(
        estimate(&proxy, &preview),
        ReferenceExposure::Unavailable(Unavailable::UnsupportedOrientation)
    );
}

#[test]
fn positive_low_signal_matches_the_same_preview_without_a_decoder_precision_threshold() {
    let reference =
        Preview::srgb_fixture(image::RgbImage::from_pixel(96, 64, image::Rgb([89; 3])), 0);
    let normal = flat(96, 64);
    let mut dim = flat(96, 64);
    for pixel in Arc::make_mut(&mut dim.pixels) {
        for value in &mut pixel[..3] {
            *value *= 2_f32.powi(-16);
        }
    }
    assert!(f64::from(dim.pixels[0][0]) / 65535. < crate::processing::CAMERA_INPUT_FLOOR);
    let ReferenceExposure::Matched(normal) = estimate(&normal, &reference) else {
        panic!("normal positive control must meter");
    };
    let ReferenceExposure::Matched(dim) = estimate(&dim, &reference) else {
        panic!("positive samples below one nominal decoder code remain usable");
    };
    assert!((dim.ev - normal.ev - 16.).abs() < 1e-9);
    let mut zero = flat(96, 64);
    Arc::make_mut(&mut zero.pixels).fill([0.; 4]);
    assert_eq!(
        estimate(&zero, &reference),
        ReferenceExposure::Unavailable(Unavailable::InsufficientSamples)
    );
}

#[test]
fn negative_preparation_does_not_require_a_file_preview_or_an_exposure_memo() {
    let proxy_data = Arc::new(flat(96, 64));
    let prepared = proxy(
        &OnceLock::new(),
        InputSettings::default(),
        &proxy_data,
        Ok(None),
    );
    assert!(prepared.reference.is_none());
}

#[test]
fn native_and_proxy_preparation_share_captured_reference_precedence_without_mutating_inputs() {
    use crate::project::Project;
    let file = crate::raw::fixture::bayer_dng();
    let reference = |ev| {
        ReferenceExposure::Matched(exposure_reference::StartingExposure {
            ev,
            samples: 1024,
            spread: 0.,
        })
    };
    let mut scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
    scan.reference_exposure = Some(reference(-2.));
    let mut project = Project {
        input: InputSettings::Digital,
        ..Default::default()
    };
    let metadata = scan.source().metadata();
    let source = crate::project::Source {
        development_mode: crate::project::DevelopmentMode::AsShot,
        id: crate::project::SourceId(0),
        path: file.path().to_owned(),
        development: SourceDevelopment::from_metadata(&metadata, scan.source().white_balance())
            .unwrap(),
        adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
            Default::default(),
        )),
        repairs: Default::default(),
        orientation: metadata.photo_orientation().unwrap(),
        crop: None,
    };
    let id = project.add_source(source).unwrap();
    let pixels = Arc::new(CameraPixels::from_scan(&scan).unwrap());
    for (recorded, expected_ev) in [
        (None, -2.),
        (Some(reference(1.)), 1.),
        (
            Some(ReferenceExposure::Unavailable(Unavailable::NoUsablePreview)),
            0.,
        ),
    ] {
        let mut authored = project.image_recipe(id).unwrap();
        authored.neutral.reference = recorded.clone();
        let authored = Arc::new(authored);
        let memo = OnceLock::from(reference(-2.));
        let (native, native_recipe) = native_image(&scan, &authored).unwrap();
        let (proxy, proxy_recipe) = proxy_image(&authored, &memo, &pixels, Ok(None));
        assert_eq!(
            native.reference_exposure.as_ref().unwrap().ev(),
            expected_ev
        );
        assert_eq!(proxy.reference.as_ref().unwrap().ev(), expected_ev);
        for prepared in [&native_recipe, &*proxy_recipe] {
            let (input, matrix) = prepared.neutral.processing_transform().unwrap();
            assert_eq!(input, InputSettings::Digital);
            assert_eq!(matrix, scan.camera_to_ap1 * expected_ev.exp2());
        }
        assert_eq!(authored.neutral.reference, recorded);
        assert_eq!(scan.reference_exposure.as_ref().unwrap().ev(), -2.);
        assert_eq!(memo.get().unwrap().ev(), -2.);
        if recorded.is_some() {
            assert!(Arc::ptr_eq(&authored, &proxy_recipe));
        }
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
            let result = estimate(&proxy, &Preview::srgb_fixture(rgb, flip));
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
    let ReferenceExposure::Matched(value) = estimate(&proxy, &preview) else {
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
        estimate(&data, &preview),
        ReferenceExposure::Matched(_)
    ));
    let recorded = ReferenceExposure::Unavailable(Unavailable::NoUsablePreview);
    let memo = OnceLock::from(recorded.clone());
    let prepared = proxy(&memo, InputSettings::Digital, &data, Ok(Some(preview)));
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
