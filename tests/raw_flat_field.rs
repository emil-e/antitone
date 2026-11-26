#![cfg(all(feature = "raw", target_os = "macos"))]
use antitone::adjustment_layers::AdjustmentLayerSettings;
use antitone::{
    flat_field::{FlatField, reference as reference_field, sample_camera_light},
    processing::Calibration,
    raw::Rect,
    sampling::camera::reference as measure_reference,
    scan::DevelopedSource,
};
use glam::DVec3;
use std::{fs, path::PathBuf, sync::Arc};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn estimate_map(reference: &DevelopedSource) -> Arc<FlatField> {
    let image = reference.camera_image().unwrap();
    assert_eq!((image.width, image.height), (384, 320));
    // The proposal's two-pixel cells discard one boundary cell on each side.
    assert_eq!(
        reference_field::suggest_region(reference).unwrap(),
        Some(Rect {
            x: 10,
            y: 10,
            width: 364,
            height: 300
        })
    );
    Arc::new(reference_field::estimate(reference, [16, 16, 352, 288].into()).unwrap())
}

fn correction_fixture() -> (Arc<DevelopedSource>, Arc<FlatField>) {
    let reference = DevelopedSource::open(&fixture("bare-light.dng"), None).unwrap();
    let map = estimate_map(&reference);
    let scan = DevelopedSource::open(
        &fixture("illuminated-patches.dng"),
        Some(reference.white_balance()),
    )
    .unwrap();
    assert_eq!(scan.dimensions().unwrap(), (384, 320));
    assert_eq!(scan.area(), reference.area());
    assert_eq!(scan.camera_model(), reference.camera_model());
    assert_eq!(scan.camera_to_ap1(), reference.camera_to_ap1());
    (scan, map)
}

fn calibration(scan: &DevelopedSource, map: &FlatField) -> Calibration {
    Calibration {
        dmin: measure_reference(
            scan,
            Some(map),
            Rect {
                x: 16,
                y: 16,
                width: 32,
                height: 32,
            },
        )
        .unwrap(),
        dmax: measure_reference(
            scan,
            Some(map),
            Rect {
                x: 80,
                y: 16,
                width: 32,
                height: 32,
            },
        )
        .unwrap(),
    }
}

#[test]
fn authored_bare_light_corrected_by_itself_matches_its_original_center() {
    let reference = DevelopedSource::open(&fixture("bare-light.dng"), None).unwrap();
    // Offset the rectangle so its midpoint is not the image centre.
    let map = Arc::new(reference_field::estimate(&reference, [96, 16, 264, 288].into()).unwrap());
    let center = reference.pixel(191, 159).unwrap();
    let points = [(191, 159), (104, 32), (351, 287)];
    assert!(
        (reference.pixel(104, 32).unwrap() - center)
            .abs()
            .max_element()
            > 0.02
    );
    assert!(
        (map.gain([192., 160.]).unwrap() - DVec3::ONE)
            .abs()
            .max_element()
            < 2e-12
    );

    for (x, y) in points {
        // Four sensor codes cover the half-pixel centre offset, quantization and fitting.
        let actual = sample_camera_light(&reference, Some(&map), [x, y]).unwrap();
        assert!(
            (actual - center).abs().max_element() < 4. / 4031.,
            "{x},{y}: {actual:?}"
        );
    }
}

#[test]
fn reference_estimation_bounds_the_support_to_the_developed_image() {
    let reference = DevelopedSource::open(&fixture("bare-light.dng"), None).unwrap();
    let source = &reference;
    let area = source.area();
    for region in [
        [area.x, area.y, 0, area.height],
        [area.x, area.y, area.width + 1, area.height],
    ] {
        assert!(matches!(
            reference_field::estimate(source, region.into()),
            Err(antitone::Error::InvalidParameter(
                "flat-field support is outside the active source area"
            ))
        ));
    }
}

#[test]
fn authored_bare_light_corrects_known_illumination_and_persists_the_measured_map() {
    let originals = ["bare-light.dng", "illuminated-patches.dng"]
        .map(|name| (fixture(name), fs::read(fixture(name)).unwrap()));
    let (scan, map) = correction_fixture();
    let plain = DevelopedSource::open(
        &fixture("illuminated-patches.dng"),
        Some(scan.white_balance()),
    )
    .unwrap();
    let patches = [
        [3000., 3000., 3000.],
        [128., 128., 128.],
        [1000., 1000., 1000.],
        [2400., 1200., 600.],
        [1200., 2400., 600.],
        [600., 1200., 2400.],
        [3200., 3200., 3200.],
        [600., 1000., 1800.],
        [256., 256., 256.],
        [512., 512., 512.],
        [1024., 1024., 1024.],
        [2048., 2048., 2048.],
    ];
    for y in 0..5 {
        for x in 0..6 {
            let point = [64 * x + 32, 64 * y + 32];
            let u = (2. * (point[0] as f64 + 0.5) - 384.) / 384.;
            let v = (2. * (point[1] as f64 + 0.5) - 320.) / 320.;
            let illumination =
                DVec3::ONE + DVec3::new(0.08, -0.05, 0.03) * u + DVec3::new(-0.04, 0.06, 0.08) * v;
            let gain = map
                .gain([point[0] as f64 + 0.5, point[1] as f64 + 0.5])
                .unwrap();
            // Four authored sensor codes allow quantization/demosaicing and local-fit error.
            assert!((gain - illumination.recip()).abs().max_element() < 4. / 2400.);
            let expected = DVec3::from_array(patches[((y * 6 + x) % 12) as usize]) / 4031.;
            let allowance = 4. / 4031. * map.bounds().1.max_element();
            assert!(
                (sample_camera_light(&scan, Some(&map), [point[0], point[1]]).unwrap() - expected)
                    .abs()
                    .max_element()
                    < allowance
            );
        }
    }
    assert!(
        (plain.pixel(32, 32).unwrap() - sample_camera_light(&scan, Some(&map), [32, 32]).unwrap())
            .abs()
            .max_element()
            > 0.02
    );
    assert_eq!(map.gain([8.5, 8.5]).unwrap(), DVec3::ONE);
    assert_eq!(
        sample_camera_light(&scan, Some(&map), [8, 8]).unwrap(),
        plain.pixel(8, 8).unwrap()
    );
    let partial = Arc::new(
        FlatField::from_nodes([372, 0, 16, 16], [2, 2], vec![DVec3::splat(3.); 4]).unwrap(),
    );
    let before = sample_camera_light(&scan, Some(&map), [32, 32]).unwrap();
    assert_eq!(
        sample_camera_light(&scan, Some(&partial), [32, 32]).unwrap(),
        plain.pixel(32, 32).unwrap(),
    );
    assert_eq!(
        sample_camera_light(&scan, Some(&partial), [374, 10]).unwrap(),
        plain.pixel(374, 10).unwrap() * 3.
    );
    assert_eq!(
        sample_camera_light(&scan, Some(&map), [32, 32]).unwrap(),
        before
    );
    let anchors = calibration(&scan, &map);
    assert_eq!((anchors).validate().unwrap(), anchors);

    #[cfg(feature = "project")]
    {
        use antitone::{
            processing::{DEFAULT_GAMMA, InputSettings, Response},
            project::{FlatFieldSettings, Project, Save, Source, SourceDevelopment, SourceId},
        };
        let decode = scan.metadata().development([1.; 4]);
        let mut project = Project {
            input: InputSettings::Negative {
                dmin: Some(anchors.dmin),
                dmax: Some(anchors.dmax),
                response: Response::film(),
                gamma: DEFAULT_GAMMA,
                balance: Default::default(),
            },
            ..Project::default()
        };
        let mut reference_id = None;
        for name in ["illuminated-patches.dng", "bare-light.dng"] {
            let is_reference = name == "bare-light.dng";
            let id = project
                .add_source(Source {
                    capture_time: None,
                    development_mode: antitone::project::DevelopmentMode::Unity,
                    id: SourceId(0),
                    path: fixture(name),
                    development: SourceDevelopment {
                        decode: antitone::project::Development {
                            white_balance: scan.white_balance(),
                            ..decode.clone()
                        },
                        dimensions: [384, 320],
                        active_area: [8, 8, 368, 304],
                        camera_model: scan.camera_model().to_owned(),
                        camera_to_ap1: scan.camera_to_ap1().to_cols_array_2d(),
                    },
                    adjustment_layers: Some(antitone::project::AdjustmentLayers::with_global(
                        AdjustmentLayerSettings::default(),
                    )),
                    repairs: Default::default(),
                    orientation: Default::default(),
                    crop: None,
                })
                .unwrap();
            if is_reference {
                reference_id = Some(id);
            }
        }
        let reference_id = reference_id.unwrap();
        project.set_flat_field(Some(FlatFieldSettings::new(
            project.source(reference_id).unwrap().path.clone(),
            map.clone(),
        )));
        project.remove_from_filmstrip(reference_id).unwrap();
        assert!(project.source(reference_id).is_err());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("corrected.antitone.json");
        project.save(&path, Save::New).unwrap();
        let reopened = Project::load(&path).unwrap();
        assert_eq!(reopened, project);
        assert_eq!(
            reopened.calibration().unwrap().unwrap(),
            (anchors).validate().unwrap()
        );
        let restored = DevelopedSource::open(
            &fixture("illuminated-patches.dng"),
            reopened.acquisition_wb(),
        )
        .unwrap();
        let restored_map = &reopened.active_flat_field().unwrap().map;
        for y in (8..312).step_by(7) {
            for x in (8..376).step_by(7) {
                assert_eq!(
                    sample_camera_light(&restored, Some(restored_map), [x, y]).unwrap(),
                    sample_camera_light(&scan, Some(&map), [x, y]).unwrap()
                );
            }
        }
        assert_eq!(scan.pixel(32, 32).unwrap(), plain.pixel(32, 32).unwrap());
    }
    for (path, original) in originals {
        assert_eq!(fs::read(path).unwrap(), original);
    }
}

#[cfg(all(feature = "export", feature = "ocio-reference"))]
#[test]
fn authored_corrected_dng_exports_reopen_in_rgb16_and_ap0() {
    use antitone::{
        color::ap1_to_ap0,
        export::{self, ExrPrecision, ImageSize, Overwrite},
        processing::{InputSettings, Pipeline, RollSettings},
        profiles,
        rendering::{CpuRenderer, OutputTarget},
    };
    let (scan, map) = correction_fixture();
    let anchors = calibration(&scan, &map);
    let roll = RollSettings::default();
    let input = InputSettings::negative((anchors).validate().unwrap());
    let pipeline = Pipeline::new(input, roll, scan.camera_to_ap1()).unwrap();
    let size = ImageSize {
        width: scan.area().width,
        height: scan.area().height,
    };
    let working: Vec<_> = (0..size.height)
        .flat_map(|y| (0..size.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            pipeline
                .apply(
                    sample_camera_light(&scan, Some(&map), [scan.area().x + x, scan.area().y + y])
                        .unwrap(),
                )
                .unwrap()
                .rgb
        })
        .collect();
    assert!(working.iter().any(|p| p.min_element() < 0.));
    assert!(working.iter().any(|p| p.max_element() > 1.));
    let pixel = |x, y| Ok(working[(y * size.width + x) as usize]);
    let directory = tempfile::tempdir().unwrap();
    for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
        let path = directory.path().join(format!("{target:?}.tif"));
        export::tiff(&path, size, target, Overwrite::Refuse, pixel).unwrap();
        let mut decoder = tiff::decoder::Decoder::new(fs::File::open(path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (size.width, size.height));
        assert_eq!(
            decoder
                .get_tag_u8_vec(tiff::tags::Tag::Unknown(34675))
                .unwrap(),
            profiles::icc(target)
        );
        let tiff::decoder::DecodingResult::U16(actual) = decoder.read_image().unwrap() else {
            panic!("not RGB16")
        };
        let expected: Vec<_> = target
            .encode_u16(&CpuRenderer::new(target).unwrap().render(&working).unwrap())
            .unwrap()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(actual, expected);
    }
    let path = directory.path().join("corrected.exr");
    export::exr(&path, size, ExrPrecision::Half, Overwrite::Refuse, pixel).unwrap();
    let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
    assert_eq!(
        image.layer_data.size,
        exr::math::Vec2(size.width as usize, size.height as usize)
    );
    for channel in &image.layer_data.channel_data.list {
        let index = match channel.name.to_string().as_str() {
            "R" => 0,
            "G" => 1,
            "B" => 2,
            _ => panic!("unexpected channel"),
        };
        let exr::prelude::FlatSamples::F16(actual) = &channel.sample_data else {
            panic!("not half-float")
        };
        for (p, actual) in working.iter().zip(actual) {
            assert_eq!(*actual, half::f16::from_f64((ap1_to_ap0() * *p)[index]));
        }
    }
}
