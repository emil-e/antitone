use super::*;
use crate::film_emulation::{FilmEmulationSettings, FilmPreset};

pub(super) fn exr_pixels(path: &Path) -> Vec<[f32; 3]> {
    use exr::prelude::{FlatSamples, read_first_flat_layer_from_file};
    let image = read_first_flat_layer_from_file(path).unwrap();
    let mut pixels = vec![[0.; 3]; image.layer_data.size.0 * image.layer_data.size.1];
    for channel in &image.layer_data.channel_data.list {
        let c = match channel.name.to_string().as_str() {
            "R" => 0,
            "G" => 1,
            "B" => 2,
            _ => panic!("unexpected channel"),
        };
        match &channel.sample_data {
            FlatSamples::F16(values) => {
                for (p, v) in pixels.iter_mut().zip(values) {
                    p[c] = v.to_f32();
                }
            }
            FlatSamples::F32(values) => {
                for (p, v) in pixels.iter_mut().zip(values) {
                    p[c] = *v;
                }
            }
            _ => panic!("unexpected EXR sample type"),
        }
    }
    pixels
}

pub(super) fn tiff_pixels(path: &Path) -> Vec<u16> {
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    match decoder.read_image().unwrap() {
        tiff::decoder::DecodingResult::U16(values) => values,
        _ => panic!("expected RGB16"),
    }
}

#[test]
fn exported_files_include_seeded_film_before_tiff_look_and_keep_it_in_both_scene_linear_exr_precisions()
 {
    let (device, queue, renderer) = super::tests::context();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let composition = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    let gate = SubmissionGate::default();
    let directory = tempfile::tempdir().unwrap();
    let film = FilmPreset::ThirtyFiveMm.settings();
    let seed = (1 << 60) + 17;
    let write_file = |name: &str, settings, seed, format, look| {
        let path = directory.path().join(name);
        write(
            &renderer,
            &device,
            (&queue, &gate),
            Request {
                scan: &scan,
                field: None,
                input: InputSettings::Digital,
                roll: Default::default(),
                grade: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                repairs: &[],
                composition,
                format,
                look,
                film_emulation: settings,
                halation: Default::default(),
                film_seed: seed,
            },
            &path,
            Overwrite::Refuse,
        )
        .unwrap();
        path
    };
    let look = Arc::new(
        crate::looks::LookAsset::new(
            "constant stage-order witness".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(
                crate::looks::Lut3D::new(2, [-16.; 3], [16.; 3], vec![[0.4, 0.2, 0.3]; 8]).unwrap(),
            ),
        )
        .unwrap(),
    );
    let clean_tiff = write_file(
        "clean-look.tif",
        FilmEmulationSettings::default(),
        seed,
        Format::Tiff(OutputTarget::Srgb),
        Some(look.clone()),
    );
    let film_tiff = write_file(
        "film-look.tif",
        film,
        seed,
        Format::Tiff(OutputTarget::Srgb),
        Some(look.clone()),
    );
    assert!(
        tiff_pixels(&clean_tiff) == tiff_pixels(&film_tiff),
        "a constant look erases upstream film; downstream grain would survive"
    );
    let clean_plain = write_file(
        "clean.tif",
        FilmEmulationSettings::default(),
        seed,
        Format::Tiff(OutputTarget::Srgb),
        None,
    );
    let film_plain = write_file(
        "film.tif",
        film,
        seed,
        Format::Tiff(OutputTarget::Srgb),
        None,
    );
    assert!(
        tiff_pixels(&clean_plain) != tiff_pixels(&film_plain),
        "plain TIFF must include film"
    );
    for (name, precision) in [("half", ExrPrecision::Half), ("full", ExrPrecision::Full)] {
        let plain = write_file(
            &format!("film-{name}.exr"),
            film,
            seed,
            Format::Exr(precision),
            None,
        );
        let looked = write_file(
            &format!("look-{name}.exr"),
            film,
            seed,
            Format::Exr(precision),
            Some(look.clone()),
        );
        let clean = write_file(
            &format!("clean-{name}.exr"),
            FilmEmulationSettings::default(),
            seed,
            Format::Exr(precision),
            None,
        );
        let reseeded = write_file(
            &format!("seed-{name}.exr"),
            film,
            seed + 1,
            Format::Exr(precision),
            None,
        );
        let actual = exr_pixels(&plain);
        assert!(actual.iter().flatten().all(|v| v.is_finite()));
        assert!(
            actual == exr_pixels(&looked),
            "EXR bypasses the creative look, not film emulation"
        );
        assert!(
            actual != exr_pixels(&clean),
            "film is not a display-only overlay"
        );
        assert!(
            actual != exr_pixels(&reseeded),
            "persistent source seed reaches authoritative export"
        );
    }
}

#[cfg(feature = "editor")]
#[test]
fn original_reference_film_matches_authoritative_tiff_at_native_scale() {
    use std::sync::atomic::AtomicU64;
    let (device, queue, renderer) = super::tests::context();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let film = FilmPreset::ThirtyFiveMm.settings();
    let seed = (1 << 60) + 17;
    let stack = AdjustmentLayerSnapshot::new(vec![
        crate::adjustment_layers::AdjustmentLayer {
            mask: crate::adjustment_layers::AdjustmentLayerCoverage::Linear {
                start: crate::geometry::SourcePoint([scan.area.x as f64, scan.area.y as f64]),
                end: crate::geometry::SourcePoint([
                    (scan.area.x + scan.area.width) as f64,
                    (scan.area.y + scan.area.height) as f64,
                ]),
            },
            photo: crate::adjustment_layers::AdjustmentLayerSettings {
                exposure: 0.7,
                contrast: 0.85,
                ..Default::default()
            },
            enabled: true,
            invert: false,
            strength: 0.8,
        },
        crate::adjustment_layers::AdjustmentLayer::global(
            crate::adjustment_layers::AdjustmentLayerSettings {
                contrast: 1.15,
                saturation: 0.9,
                ..Default::default()
            },
        ),
    ])
    .unwrap();
    let grade = Arc::new(
        ProcessingInput::new(
            &device,
            InputSettings::Digital,
            Default::default(),
            stack.clone(),
            scan.camera_to_ap1,
            scan.area,
        )
        .unwrap(),
    );
    let input = crate::gpu::ProcessingInput::from(grade)
        .with_film_emulation(film)
        .with_film_seed(seed);
    let gate = SubmissionGate::default();
    let generation = AtomicU64::new(1);
    let directory = tempfile::tempdir().unwrap();
    for crop in [
        None,
        Some(crate::crop::Crop {
            center: [
                scan.area.x as f64 + scan.area.width as f64 / 2.,
                scan.area.y as f64 + scan.area.height as f64 / 2.,
            ],
            size: [scan.area.width - 64, scan.area.height - 64],
            angle: 0.13,
        }),
    ] {
        let composition =
            Composition::with_crop(scan.area, Default::default(), Default::default(), crop)
                .unwrap();
        let frame = FrameRaster::full(composition).unwrap();
        let input = input.clone().with_frame_size(composition.crop().size);
        let view = crate::refinement::refine(
            &renderer,
            &device,
            (&queue, &gate),
            (&scan, None),
            input.clone(),
            OutputTarget::Srgb,
            (&generation, 1),
            crate::refinement::ViewRequest {
                area: frame.bounds().unwrap(),
                output: frame.output,
                frame: Some(frame),
            },
        )
        .unwrap()
        .unwrap();
        let actual = crate::viewer::readback::image_pixels(&device, &queue, &view);
        let path = directory.path().join(if crop.is_some() {
            "cropped.tif"
        } else {
            "whole.tif"
        });
        write(
            &renderer,
            &device,
            (&queue, &gate),
            Request {
                scan: &scan,
                field: None,
                input: InputSettings::Digital,
                roll: Default::default(),
                grade: stack.clone(),
                repairs: &[],
                composition,
                format: Format::Tiff(OutputTarget::Srgb),
                look: None,
                film_emulation: film,
                halation: Default::default(),
                film_seed: seed,
            },
            &path,
            Overwrite::Refuse,
        )
        .unwrap();
        let expected = tiff_pixels(&path);
        assert_eq!(expected.len(), actual.len() * 3);
        for (i, pixel) in actual.iter().enumerate() {
            for c in 0..3 {
                let v = pixel[c].clamp(0., 1.) as f64;
                let encoded = if v <= 0.0031308 {
                    12.92 * v
                } else {
                    1.055 * v.powf(1. / 2.4) - 0.055
                };
                let code = (encoded * 65535.).round() as i32;
                assert!(
                    (code - i32::from(expected[i * 3 + c])).abs() <= 2,
                    "reference/export pixel {i}, channel {c}, cropped={}",
                    crop.is_some()
                );
            }
        }
    }
}
