#![cfg(all(
    feature = "project",
    feature = "export",
    feature = "ocio-reference",
    target_os = "macos"
))]
use antitone::{
    color::{AP1_TO_XYZ, ap1_to_ap0},
    crop::Crop,
    export::{self, ExrPrecision, Overwrite},
    frame_raster::FrameRaster,
    geometry::{
        Composition, Flips, PhotoOrientation, QuarterTurns, RollOrientation, SourcePoint,
        SourceRegion,
    },
    local_adjustments::{GradientMask, LocalAdjustment, SpatialPipeline},
    processing::{Calibration, PhotoSettings, Pipeline, RollSettings},
    project::{Development, LocalAdjustments, Project, Save, Source, SourceDevelopment, SourceId},
    rendering::{CpuRenderer, OutputTarget},
    sampling::weighted_mean_iter,
};
use exr::prelude::*;
use glam::{DMat3, DVec3};
use tiff::{
    decoder::{Decoder, DecodingResult},
    tags::Tag,
};

#[test]
fn persisted_locals_export_at_source_centres_before_rotated_reconstruction() {
    let directory = tempfile::tempdir().unwrap();
    let area = SourceRegion {
        x: 10,
        y: 20,
        width: 6,
        height: 6,
    };
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.02),
        input_floor: 1. / 65535.,
    };
    let mut project = Project {
        development: Some(Development::new("test-backend".into(), [2., 1., 1.5, 1.])),
        input_floor: Some(calibration.input_floor),
        dmin: Some(calibration.dmin.to_array()),
        dmax: Some(calibration.dmax.to_array()),
        ..Project::default()
    };
    let mut locals = LocalAdjustments::default();
    locals
        .add(
            "Linear exposure".into(),
            LocalAdjustment {
                mask: GradientMask::Linear {
                    start: SourcePoint([10., 20.]),
                    end: SourcePoint([16., 20.]),
                },
                photo: PhotoSettings {
                    exposure: 2.,
                    ..PhotoSettings::default()
                },
                enabled: true,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    locals
        .add(
            "Radial saturation".into(),
            LocalAdjustment {
                mask: GradientMask::Radial {
                    center: SourcePoint([13., 23.]),
                    radii: [4., 2.],
                    angle: 0.,
                    feather: 1.,
                },
                photo: PhotoSettings {
                    saturation: 2.,
                    saturation_density: 0.4,
                    ..PhotoSettings::default()
                },
                enabled: true,
                invert: false,
                strength: 1.,
            },
        )
        .unwrap();
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "read-only-not-loaded.ARW".into(),
            development: SourceDevelopment {
                dimensions: [20, 30],
                active_area: [10, 20, 6, 6],
                camera_model: "test-camera".into(),
                camera_to_ap1: DMat3::IDENTITY.to_cols_array_2d(),
                input_floor: calibration.input_floor,
            },
            reference_only: false,
            photo: PhotoSettings {
                contrast: 1.15,
                ..PhotoSettings::default()
            },
            local_adjustments: Some(locals),
            orientation: PhotoOrientation {
                turns: QuarterTurns::One,
                flips: Flips {
                    horizontal: true,
                    vertical: false,
                },
            },
            crop: Some(Crop {
                center: [13., 23.],
                size: [3, 2],
                angle: std::f64::consts::FRAC_PI_4,
            }),
        })
        .unwrap();
    let path = directory.path().join("grade.json");
    project.save(&path, Save::New).unwrap();
    let loaded = Project::load(&path).unwrap();
    let source = loaded.source(id).unwrap();
    let composition = Composition::with_crop(
        area,
        RollOrientation::default(),
        source.orientation,
        source.crop,
    )
    .unwrap();
    let grade = source.grade_snapshot().unwrap();
    let metadata = export::ProcessingMetadata::with_grade(loaded.roll.response, &grade);
    let spatial = SpatialPipeline::new(calibration, loaded.roll, grade, DMat3::IDENTITY).unwrap();
    let global = Pipeline::new(
        calibration,
        RollSettings::new_grade(),
        source.photo,
        DMat3::IDENTITY,
    )
    .unwrap();
    let camera = |x: u32, y: u32| {
        DVec3::new(
            0.3 + f64::from(x - 10) * 0.01,
            0.02 + f64::from(y - 20) * 0.02,
            0.001,
        )
    };
    // Independent coverage/exposure/saturation oracle, not the local evaluator.
    let expected_pixel = |x: u32, y: u32| {
        let rgb = global.apply(camera(x, y)).unwrap().rgb;
        let t = (f64::from(x) + 0.5 - 10.) / 6.;
        let exposure = 2. * t * t * (3. - 2. * t);
        let rgb = rgb * exposure.exp2();
        let rx = (f64::from(x) + 0.5 - 13.) / 4.;
        let ry = (f64::from(y) + 0.5 - 23.) / 2.;
        let t = (1. - (rx * rx + ry * ry).sqrt()).clamp(0., 1.);
        let saturation = 1. + t * t * (3. - 2. * t);
        let grey = DVec3::splat((AP1_TO_XYZ * rgb).y);
        let maximum = DVec3::splat(rgb.max_element());
        0.6 * (grey + saturation * (rgb - grey)) + 0.4 * (maximum + saturation * (rgb - maximum))
    };
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            let actual = spatial.apply(camera(x, y), [x, y]).unwrap().rgb;
            let expected = expected_pixel(x, y);
            for c in 0..3 {
                assert!((actual[c] - expected[c]).abs() <= 2e-12 + 2e-11 * expected[c].abs());
            }
        }
    }
    let raster = FrameRaster::full(composition).unwrap();
    let [width, height] = composition.size();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("local-{target:?}.tif"));
        export::tiff_composed(
            &path,
            composition,
            target,
            Overwrite::Refuse,
            metadata,
            |x, y| spatial.apply(camera(x, y), [x, y]).map(|pixel| pixel.rgb),
        )
        .unwrap();
        let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (width, height));
        let description = decoder.get_tag_ascii_string(Tag::ImageDescription).unwrap();
        assert!(description.contains(antitone::local_adjustments::CONTRACT));
        assert!(description.contains(antitone::photographic::CONTRACT));
        assert!(!description.contains("Linear exposure"));
        assert!(!description.contains("Radial saturation"));
        let DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!()
        };
        let renderer = CpuRenderer::new(target).unwrap();
        let mut expected = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let pixels: Vec<_> = raster
                    .footprint([x, y])
                    .unwrap()
                    .pixels(area)
                    .unwrap()
                    .collect();
                let rgb: Vec<_> = pixels
                    .iter()
                    .map(|([sx, sy], _)| expected_pixel(*sx, *sy))
                    .collect();
                let rendered = renderer.render(&rgb).unwrap();
                let mean = weighted_mean_iter(
                    rendered
                        .iter()
                        .zip(&pixels)
                        .map(|(rgb, (_, w))| Ok((rgb.as_dvec3(), *w))),
                )
                .unwrap()
                .as_vec3();
                expected.extend(target.encode_u16(&[mean]).unwrap()[0]);
            }
        }
        assert_eq!(actual, expected);
    }
    for precision in [ExrPrecision::Half, ExrPrecision::Full] {
        let path = directory.path().join(format!("local-{precision:?}.exr"));
        export::exr_composed(
            &path,
            composition,
            precision,
            Overwrite::Refuse,
            metadata,
            |x, y| spatial.apply(camera(x, y), [x, y]).map(|pixel| pixel.rgb),
        )
        .unwrap();
        let image = read_first_flat_layer_from_file(path).unwrap();
        assert_eq!(
            image
                .layer_data
                .attributes
                .other
                .get(&Text::from("antitoneLocalAdjustmentProcessing")),
            Some(&AttributeValue::Text(Text::from(
                antitone::local_adjustments::CONTRACT
            )))
        );
        let mut signed = false;
        let mut hdr = false;
        for channel in &image.layer_data.channel_data.list {
            let c = match channel.name.to_string().as_str() {
                "R" => 0,
                "G" => 1,
                "B" => 2,
                _ => panic!(),
            };
            for y in 0..height {
                for x in 0..width {
                    let expected = ap1_to_ap0()
                        * weighted_mean_iter(
                            raster
                                .footprint([x, y])
                                .unwrap()
                                .pixels(area)
                                .unwrap()
                                .map(|([sx, sy], w)| Ok((expected_pixel(sx, sy), w))),
                        )
                        .unwrap();
                    signed |= expected[c] < 0.;
                    hdr |= expected[c] > 1.;
                    let i = (y * width + x) as usize;
                    match &channel.sample_data {
                        FlatSamples::F16(values) => {
                            assert_eq!(values[i], half::f16::from_f64(expected[c]))
                        }
                        FlatSamples::F32(values) => assert_eq!(values[i], expected[c] as f32),
                        _ => panic!(),
                    }
                }
            }
        }
        assert!(
            signed && hdr,
            "fixture must exercise signed/HDR AP0: signed={signed}, hdr={hdr}"
        );
    }
}

#[test]
fn contract_metadata_tracks_snapshot_presence_including_inactive_records() {
    use antitone::processing::Response;
    let directory = tempfile::tempdir().unwrap();
    let area = SourceRegion {
        x: 0,
        y: 0,
        width: 1,
        height: 1,
    };
    let composition = Composition::new(
        area,
        RollOrientation::default(),
        PhotoOrientation::default(),
    )
    .unwrap();
    for case in [
        "absent",
        "allocator-only",
        "disabled",
        "identity",
        "zero-strength",
        "active",
    ] {
        let present = !matches!(case, "absent" | "allocator-only");
        let mut locals = LocalAdjustments::default();
        if case != "absent" {
            let id = locals
                .add(
                    "private recipe name".into(),
                    LocalAdjustment {
                        mask: GradientMask::Linear {
                            start: SourcePoint([0., 0.]),
                            end: SourcePoint([1., 0.]),
                        },
                        photo: PhotoSettings {
                            exposure: if case == "identity" { 0. } else { 1. },
                            ..Default::default()
                        },
                        enabled: case != "disabled",
                        invert: false,
                        strength: if case == "zero-strength" { 0. } else { 1. },
                    },
                )
                .unwrap();
            if case == "allocator-only" {
                locals.remove(id).unwrap();
                assert!(locals.next_id > 1);
            }
        }
        let mut source = Source {
            id: SourceId(1),
            path: "private-source.ARW".into(),
            development: SourceDevelopment {
                dimensions: [1, 1],
                active_area: [0, 0, 1, 1],
                camera_model: "test-camera".into(),
                camera_to_ap1: DMat3::IDENTITY.to_cols_array_2d(),
                input_floor: 1. / 65535.,
            },
            reference_only: false,
            photo: PhotoSettings::default(),
            local_adjustments: (case != "absent").then_some(locals),
            orientation: PhotoOrientation::default(),
            crop: None,
        };
        let grade = source.grade_snapshot().unwrap();
        for response in [
            Response::Original,
            Response::Film {
                toe: 2.54321,
                shoulder: 0.00321,
            },
        ] {
            let metadata = export::ProcessingMetadata::with_grade(response, &grade);
            // Later document changes cannot relabel an already captured export snapshot.
            source.local_adjustments = None;
            for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                let path = directory
                    .path()
                    .join(format!("{case}-{response:?}-{target:?}.tif"));
                export::tiff_composed(
                    &path,
                    composition,
                    target,
                    Overwrite::Refuse,
                    metadata,
                    |_, _| Ok(DVec3::splat(0.18)),
                )
                .unwrap();
                let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
                let description = decoder.get_tag_ascii_string(Tag::ImageDescription).unwrap();
                assert_eq!(
                    description.contains(antitone::local_adjustments::CONTRACT),
                    present,
                    "{case}"
                );
                assert!(description.contains(antitone::photographic::CONTRACT));
                assert!(description.contains("Antitone model 4"));
                assert!(!description.contains("private recipe name"));
                assert!(!description.contains("private-source.ARW"));
                if let Response::Film { .. } = response {
                    assert!(description.contains("toe=2.54321; shoulder=0.00321"));
                }
            }
            for precision in [ExrPrecision::Half, ExrPrecision::Full] {
                let path = directory
                    .path()
                    .join(format!("{case}-{response:?}-{precision:?}.exr"));
                export::exr_composed(
                    &path,
                    composition,
                    precision,
                    Overwrite::Refuse,
                    metadata,
                    |_, _| Ok(DVec3::splat(0.18)),
                )
                .unwrap();
                let image = read_first_flat_layer_from_file(path).unwrap();
                let attrs = &image.layer_data.attributes.other;
                let expected = present.then(|| {
                    AttributeValue::Text(Text::from(antitone::local_adjustments::CONTRACT))
                });
                assert_eq!(
                    attrs.get(&Text::from("antitoneLocalAdjustmentProcessing")),
                    expected.as_ref(),
                    "{case}"
                );
                assert_eq!(
                    attrs.get(&Text::from("antitonePhotographicProcessing")),
                    Some(&AttributeValue::Text(Text::from(
                        antitone::photographic::CONTRACT
                    )))
                );
                let Some(AttributeValue::Text(exposure)) =
                    attrs.get(&Text::from("antitoneExposureConvention"))
                else {
                    panic!("missing exposure convention")
                };
                assert_eq!(
                    exposure.to_string().contains("after the global grade"),
                    present
                );
                assert_eq!(
                    attrs.get(&Text::from("antitoneProcessingModel")),
                    Some(&AttributeValue::I32(4))
                );
            }
        }
    }
}
