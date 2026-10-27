use super::*;

fn flat(width: u32, height: u32) -> CameraProxy {
    CameraProxy {
        width,
        height,
        matrix: glam::DMat3::IDENTITY,
        pixels: vec![[0.18 * 65535., 0.18 * 65535., 0.18 * 65535., 0.]; (width * height) as usize]
            .into(),
        reference_exposure: Default::default(),
    }
}

#[test]
fn unsupported_geometry_and_tiny_or_dark_references_abstain_instead_of_inventing_samples() {
    let proxy = flat(96, 64);
    let mut preview = Preview {
        rgb: image::RgbImage::from_pixel(4, 4, image::Rgb([89; 3])),
        capture_flip: 0,
    };
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
    let file = FileKey::new(std::path::Path::new("nonexistent-reference-source.raw"));
    let development = development();
    proxy(
        &SourceStore::new(0),
        &file.path,
        &file,
        &development,
        InputSettings::default(),
        &proxy_data,
    )
    .unwrap();
    assert!(proxy_data.reference_exposure.get().is_none());
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
fn control(desired: f64) -> (CameraProxy, Preview) {
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
            pixel
                .to_array()
                .map(|v| (OutputTarget::Srgb.encode(f64::from(v)).unwrap() * 255.).round() as u8)
        })
        .collect();
    let preview = Preview {
        rgb: image::RgbImage::from_raw(96, 64, data).unwrap(),
        capture_flip: 0,
    };
    (proxy, preview)
}

#[cfg(feature = "ocio-reference")]
#[test]
fn quantized_reference_matches_known_gain_through_rotations_and_symmetric_letterboxing() {
    let desired = 0.7;
    let (proxy, reference) = control(desired);
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
            let result = estimate(
                &proxy,
                &Preview {
                    rgb,
                    capture_flip: flip,
                },
                1. / 65535.,
            );
            let ReferenceExposure::Matched(value) = result else {
                panic!("flip={flip} {result:?}");
            };
            assert!((value.ev - desired).abs() < 0.025, "flip={flip} {value:?}");
        }
    }
}

#[cfg(feature = "ocio-reference")]
#[test]
fn source_exposure_survives_pixel_eviction_and_does_not_re_meter_a_later_proxy() {
    let path = crate::fixture_path::FixturePath::write(
        "source.raw",
        b"not a RAW: repeated extraction must not occur",
    );
    let file = FileKey::new(path.path());
    let store = SourceStore::new(1 << 20);
    let (data, reference) = control(0.7);
    store
        .preview(&file, || Ok(Some(Arc::new(reference))))
        .unwrap();
    let data = Arc::new(data);
    proxy(
        &store,
        path.path(),
        &file,
        &development(),
        InputSettings::Digital,
        &data,
    )
    .unwrap();
    let initial = data.reference_exposure.get().unwrap().clone();
    assert!((initial.ev() - 0.7).abs() < 0.025);
    store.set_budget(0);
    assert_eq!(store.charges().retained, 0);
    let changed = Arc::new(flat(96, 64));
    proxy(
        &store,
        path.path(),
        &file,
        &development(),
        InputSettings::Digital,
        &changed,
    )
    .unwrap();
    assert_eq!(changed.reference_exposure.get(), Some(&initial));
    assert_eq!(
        std::fs::read(path.path()).unwrap(),
        b"not a RAW: repeated extraction must not occur"
    );
}
