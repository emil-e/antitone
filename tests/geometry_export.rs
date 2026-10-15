#![cfg(all(feature = "export", feature = "ocio-reference", target_os = "macos"))]
use antitone::{
    color::ap1_to_ap0,
    crop::Crop,
    export::{self, ExrPrecision, Overwrite},
    geometry::{Composition, Flips, PhotoOrientation, QuarterTurns, RollOrientation, SourceRegion},
    processing::Response,
    rendering::{CpuRenderer, OutputTarget},
};
use exr::prelude::*;
use glam::DVec3;
use tiff::decoder::{Decoder, DecodingResult};

#[test]
fn rotated_diamond_exports_filter_after_aces_and_preserve_signed_hdr_scene_data() {
    use antitone::geometry::SourcePoint;
    let directory = tempfile::tempdir().unwrap();
    let area = SourceRegion {
        x: 10,
        y: 20,
        width: 4,
        height: 4,
    };
    // Four native-resolution rotated footprints exercise per-pixel rendering before filtering.
    let crop = Crop {
        center: [12., 22.],
        size: [2, 2],
        angle: std::f64::consts::FRAC_PI_4,
    };
    let composition = Composition::with_crop(
        area,
        RollOrientation::default(),
        PhotoOrientation::default(),
        Some(crop),
    )
    .unwrap();
    let source = |x: u32, y: u32| {
        DVec3::new(
            f64::from(x - 10) * 0.8 - 0.2,
            f64::from(y - 20) * 1.3 + 0.01,
            0.15,
        )
    };
    let raster = antitone::frame_raster::FrameRaster::full(composition).unwrap();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("rotated-{target:?}.tif"));
        export::tiff_composed(
            &path,
            composition,
            target,
            Overwrite::Refuse,
            Response::film(),
            |x, y| Ok(source(x, y)),
        )
        .unwrap();
        let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (2, 2));
        let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!("not RGB16")
        };
        let renderer = CpuRenderer::new(target).unwrap();
        let mut expected = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                let pixels: Vec<_> = raster
                    .footprint([x, y])
                    .unwrap()
                    .pixels(area)
                    .unwrap()
                    .collect();
                let input: Vec<_> = pixels
                    .iter()
                    .map(|([sx, sy], _)| source(*sx, *sy))
                    .collect();
                let rendered = renderer.render(&input).unwrap();
                let mean = antitone::sampling::weighted_mean_iter(
                    rendered
                        .iter()
                        .zip(&pixels)
                        .map(|(rgb, (_, weight))| Ok((rgb.as_dvec3(), *weight))),
                )
                .unwrap()
                .as_vec3();
                expected.extend(target.encode_u16(&[mean]).unwrap()[0]);
            }
        }
        assert_eq!(actual, expected);
        let mut wrong = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                let working = antitone::sampling::weighted_mean_iter(
                    raster
                        .footprint([x, y])
                        .unwrap()
                        .pixels(area)
                        .unwrap()
                        .map(|([sx, sy], w)| Ok((source(sx, sy), w))),
                )
                .unwrap();
                wrong.extend(
                    target
                        .encode_u16(&renderer.render(&[working]).unwrap())
                        .unwrap()[0],
                );
            }
        }
        assert_ne!(actual, wrong, "must not render an averaged working pixel");
    }
    let scene =
        |x: u32, y: u32| DVec3::new(-8. + f64::from(x - 10) * 0.2, 5. + f64::from(y - 20), -0.1);
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let path = directory.path().join(format!("rotated-{precision:?}.exr"));
        export::exr_composed(
            &path,
            composition,
            precision,
            Overwrite::Refuse,
            Response::film(),
            |x, y| Ok(scene(x, y)),
        )
        .unwrap();
        let image = read_first_flat_layer_from_file(path).unwrap();
        assert_eq!(image.layer_data.size, Vec2(2, 2));
        for channel in &image.layer_data.channel_data.list {
            let c = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!(),
            };
            for y in 0..2 {
                for x in 0..2 {
                    let value = antitone::sampling::weighted_mean_iter(
                        raster
                            .footprint([x, y])
                            .unwrap()
                            .pixels(area)
                            .unwrap()
                            .map(|([sx, sy], w)| Ok((ap1_to_ap0() * scene(sx, sy), w))),
                    )
                    .unwrap()[c];
                    let i = (y * 2 + x) as usize;
                    match &channel.sample_data {
                        FlatSamples::F16(data) => assert_eq!(data[i], half::f16::from_f64(value)),
                        FlatSamples::F32(data) => assert_eq!(data[i], value as f32),
                        _ => panic!(),
                    }
                }
            }
        }
    }
    // A separate analytic mask verifies the sampling primitive independently of export code.
    let diamond = antitone::coverage::Coverage::new(
        [[12., 20.], [14., 22.], [12., 24.], [10., 22.]].map(SourcePoint),
    )
    .unwrap();
    let weights = [
        [0., 0.5, 0.5, 0.],
        [0.5, 1., 1., 0.5],
        [0.5, 1., 1., 0.5],
        [0., 0.5, 0.5, 0.],
    ];
    let expected = weights.iter().enumerate().flat_map(|(y, row)| {
        row.iter().enumerate().filter_map(move |(x, &w)| {
            (w > 0.).then_some(Ok((source(x as u32 + 10, y as u32 + 20), w)))
        })
    });
    assert_eq!(
        antitone::sampling::weighted_mean_iter(expected).unwrap(),
        antitone::sampling::weighted_mean_iter(
            diamond
                .pixels(area)
                .unwrap()
                .map(|([x, y], w)| Ok((source(x, y), w)))
        )
        .unwrap()
    );
}

#[test]
fn native_camera_flat_field_compositions_reopen_with_source_anchored_correction() {
    use antitone::{
        flat_field::FlatField,
        processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    };
    use glam::DMat3;
    let directory = tempfile::tempdir().unwrap();
    let area = SourceRegion {
        x: 10,
        y: 20,
        width: 4,
        height: 4,
    };
    let matrix = DMat3::from_cols(
        DVec3::new(0.6, 0.02, -0.01),
        DVec3::new(0.4, 1.2, -0.15),
        DVec3::new(0., -0.22, 1.16),
    );
    let field = FlatField::from_nodes(
        [11, 21, 2, 2],
        [2, 2],
        vec![
            DVec3::new(0.9, 1., 0.95),
            DVec3::new(1.2, 1.1, 1.),
            DVec3::new(1., 1.2, 0.9),
            DVec3::new(1.3, 1.15, 1.1),
        ],
    )
    .unwrap();
    let base = Calibration {
        dmin: DVec3::new(0.8, 0.7, 0.9),
        dmax: DVec3::new(0.003, 0.002, 0.001),
        input_floor: antitone::processing::CAMERA_INPUT_FLOOR,
    };
    let roll = RollSettings {
        exposure: 1.2,
        ..RollSettings::new_grade()
    };
    let camera = |x: u32, y: u32| {
        DVec3::new(
            3000. + f64::from(x - 10) * 13000.,
            4000. + f64::from(y - 20) * 15000.,
            5000. + f64::from(x + y - 30) * 8000.,
        ) / 65535.
    };
    for corrected in [false, true] {
        let calibration = if corrected {
            field.processing_calibration(base).unwrap()
        } else {
            base
        };
        let photo = PhotoSettings {
            contrast: 0.8,
            saturation: 1.3,
            ..PhotoSettings::default()
        };
        let pipeline = Pipeline::new(calibration, roll, photo, matrix).unwrap();
        let source = |x: u32, y: u32| {
            pipeline
                .apply(
                    camera(x, y)
                        * if corrected {
                            field.gain([x as f64 + 0.5, y as f64 + 0.5]).unwrap()
                        } else {
                            DVec3::ONE
                        },
                )
                .map(|p| p.rgb)
        };
        let working: Vec<_> = (20..24)
            .flat_map(|y| (10..14).map(move |x| source(x, y).unwrap()))
            .collect();
        for angle in [0., std::f64::consts::FRAC_PI_4] {
            let composition = Composition::with_crop(
                area,
                RollOrientation {
                    turns: QuarterTurns::One,
                    horizontal: true,
                    ..Default::default()
                },
                PhotoOrientation {
                    flips: Flips {
                        vertical: true,
                        ..Default::default()
                    },
                    ..Default::default()
                },
                Some(Crop {
                    center: [12., 22.],
                    size: [2, 2],
                    angle,
                }),
            )
            .unwrap();
            let raster = antitone::frame_raster::FrameRaster::full(composition).unwrap();
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let path = directory
                    .path()
                    .join(format!("camera-{corrected}-{angle}-{target:?}.tif"));
                export::tiff_composed(
                    &path,
                    composition,
                    target,
                    Overwrite::Refuse,
                    roll.response,
                    source,
                )
                .unwrap();
                let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
                assert_eq!(decoder.dimensions().unwrap(), (2, 2));
                let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
                    panic!("not RGB16")
                };
                let rendered = CpuRenderer::new(target).unwrap().render(&working).unwrap();
                let mut expected = Vec::new();
                for y in 0..2 {
                    for x in 0..2 {
                        let mean = antitone::sampling::weighted_mean_iter(
                            raster.footprint([x, y]).unwrap().pixels(area).unwrap().map(
                                |([sx, sy], weight)| {
                                    Ok((
                                        rendered[((sy - 20) * 4 + sx - 10) as usize].as_dvec3(),
                                        weight,
                                    ))
                                },
                            ),
                        )
                        .unwrap();
                        expected.extend(target.encode_u16(&[mean.as_vec3()]).unwrap()[0]);
                    }
                }
                assert_eq!(
                    actual, expected,
                    "corrected={corrected} angle={angle} target={target:?}"
                );
            }
            for precision in [ExrPrecision::Half, ExrPrecision::Full] {
                let path = directory
                    .path()
                    .join(format!("camera-{corrected}-{angle}-{precision:?}.exr"));
                export::exr_composed(
                    &path,
                    composition,
                    precision,
                    Overwrite::Refuse,
                    roll.response,
                    source,
                )
                .unwrap();
                let image = read_first_flat_layer_from_file(path).unwrap();
                assert_eq!(image.layer_data.size, Vec2(2, 2));
                for channel in &image.layer_data.channel_data.list {
                    let c = match channel.name.to_string().as_str() {
                        "R" => 0,
                        "G" => 1,
                        "B" => 2,
                        _ => panic!(),
                    };
                    for y in 0..2 {
                        for x in 0..2 {
                            let value = antitone::sampling::weighted_mean_iter(
                                raster.footprint([x, y]).unwrap().pixels(area).unwrap().map(
                                    |([sx, sy], weight)| {
                                        Ok((
                                            ap1_to_ap0()
                                                * working[((sy - 20) * 4 + sx - 10) as usize],
                                            weight,
                                        ))
                                    },
                                ),
                            )
                            .unwrap()[c];
                            let i = (y * 2 + x) as usize;
                            match &channel.sample_data {
                                FlatSamples::F16(data) => {
                                    assert_eq!(data[i], half::f16::from_f64(value))
                                }
                                FlatSamples::F32(data) => assert_eq!(data[i], value as f32),
                                _ => panic!(),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn composed_tiff_and_ap0_exr_reopen_with_matching_dimensions_and_pixel_permutation() {
    for turns in [
        QuarterTurns::Zero,
        QuarterTurns::One,
        QuarterTurns::Two,
        QuarterTurns::Three,
    ] {
        check_exports(None, turns);
        check_exports(
            Some(Crop::from_region(SourceRegion {
                x: 12,
                y: 7,
                width: 3,
                height: 2,
            })),
            turns,
        );
    }
}
fn check_exports(crop: Option<Crop>, turns: QuarterTurns) {
    let directory = tempfile::tempdir().unwrap();
    let area = SourceRegion {
        x: 12,
        y: 7,
        width: 5,
        height: 3,
    };
    let composition = Composition::with_crop(
        area,
        RollOrientation {
            turns,
            horizontal: true,
            vertical: false,
        },
        PhotoOrientation {
            turns: QuarterTurns::One,
            flips: Flips {
                horizontal: false,
                vertical: true,
            },
        },
        crop,
    )
    .unwrap();
    let [width, height] = composition.size();
    let swap = matches!(turns, QuarterTurns::Zero | QuarterTurns::Two);
    assert_eq!(
        [width, height],
        match (crop.is_some(), swap) {
            (true, true) => [2, 3],
            (true, false) => [3, 2],
            (false, true) => [3, 5],
            (false, false) => [5, 3],
        }
    );
    let source = |index: u32| DVec3::new(f64::from(index) * 0.13 - 0.1, f64::from(index % 5), 0.2);
    let pixel = |sx, sy| Ok(source((sy - area.y) * area.width + sx - area.x));
    // Independent layouts: roll turn/flip, then photo clockwise turn and vertical flip.
    let indices: Vec<_> = match (crop.is_some(), turns) {
        (true, QuarterTurns::Zero) => vec![5, 0, 6, 1, 7, 2],
        (true, QuarterTurns::One) => vec![7, 6, 5, 2, 1, 0],
        (true, QuarterTurns::Two) => vec![2, 7, 1, 6, 0, 5],
        (true, QuarterTurns::Three) => vec![0, 1, 2, 5, 6, 7],
        (false, QuarterTurns::Zero) => (0..5).flat_map(|x| [10 + x, 5 + x, x]).collect(),
        (false, QuarterTurns::One) => (0..15).rev().collect(),
        (false, QuarterTurns::Two) => (0..5).rev().flat_map(|x| [x, 5 + x, 10 + x]).collect(),
        (false, QuarterTurns::Three) => (0..15).collect(),
    };
    let expected: Vec<_> = indices.into_iter().map(source).collect();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("{target:?}.tif"));
        export::tiff_composed(
            &path,
            composition,
            target,
            Overwrite::Refuse,
            Response::film(),
            pixel,
        )
        .unwrap();
        let mut decoder = Decoder::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (width, height));
        let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!("not RGB16")
        };
        let rendered = CpuRenderer::new(target).unwrap().render(&expected).unwrap();
        let codes: Vec<_> = target
            .encode_u16(&rendered)
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(actual, codes);
    }
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let path = directory.path().join(format!("{precision:?}.exr"));
        export::exr_composed(
            &path,
            composition,
            precision,
            Overwrite::Refuse,
            Response::film(),
            pixel,
        )
        .unwrap();
        let image = read_first_flat_layer_from_file(&path).unwrap();
        assert_eq!(image.layer_data.size, Vec2(width as usize, height as usize));
        for channel in &image.layer_data.channel_data.list {
            let index = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!("unexpected channel"),
            };
            for (i, input) in expected.iter().enumerate() {
                let value = (ap1_to_ap0() * input)[index];
                match (&channel.sample_data, precision) {
                    (FlatSamples::F16(data), ExrPrecision::Half) => {
                        assert_eq!(data[i], half::f16::from_f64(value))
                    }
                    (FlatSamples::F32(data), ExrPrecision::Full) => {
                        assert_eq!(data[i], value as f32)
                    }
                    _ => panic!("wrong precision"),
                }
            }
        }
    }
}
