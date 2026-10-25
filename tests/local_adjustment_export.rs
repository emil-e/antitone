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
    processing::{Calibration, InputSettings, PhotoSettings, Pipeline, RollSettings},
    project::{Development, LocalAdjustments, Project, Save, Source, SourceDevelopment, SourceId},
    rendering::{CpuRenderer, OutputTarget},
    sampling::weighted_mean_iter,
};
use exr::prelude::*;
use glam::{DMat3, DVec3};
use tiff::decoder::{Decoder, DecodingResult};

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
        input: InputSettings::negative(calibration),
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
                decode: Development::new("test-backend".into(), [1.; 4]),
                lens_correction: None,
                dng_development: None,
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
            repairs: Default::default(),
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
    let spatial = SpatialPipeline::new(
        InputSettings::negative(calibration),
        loaded.roll,
        grade,
        DMat3::IDENTITY,
    )
    .unwrap();
    let global = Pipeline::new(
        InputSettings::negative(calibration),
        RollSettings::default(),
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
        export::tiff_composed(&path, composition, target, Overwrite::Refuse, |x, y| {
            spatial.apply(camera(x, y), [x, y]).map(|pixel| pixel.rgb)
        })
        .unwrap();
        let mut decoder = Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (width, height));
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
        export::exr_composed(&path, composition, precision, Overwrite::Refuse, |x, y| {
            spatial.apply(camera(x, y), [x, y]).map(|pixel| pixel.rgb)
        })
        .unwrap();
        let image = read_first_flat_layer_from_file(path).unwrap();
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
