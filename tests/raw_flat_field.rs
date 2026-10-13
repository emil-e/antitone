#![cfg(all(feature = "raw", target_os = "macos"))]
use antitone::{
    color::AP1_TO_XYZ, flat_field::FlatField, processing::Calibration, raw::Rect, scan::Scan,
};
use glam::DVec3;
use std::{fs, path::PathBuf, sync::Arc};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn corrected_scan() -> (Scan, Arc<FlatField>, DVec3) {
    let mut reference = Scan::open(&fixture("bare-light.dng"), None).unwrap();
    assert_eq!(reference.dimensions().unwrap(), (384, 320));
    // The proposal's two-pixel cells discard one boundary cell on each side.
    assert_eq!(
        reference.propose_flat_field_area().unwrap(),
        Some(Rect {
            x: 10,
            y: 10,
            width: 364,
            height: 300
        })
    );
    let center = reference
        .reference(Rect {
            x: 64,
            y: 32,
            width: 256,
            height: 256,
        })
        .unwrap();
    let target = reference.camera_to_ap1.inverse()
        * DVec3::splat((AP1_TO_XYZ * (reference.camera_to_ap1 * center)).y);
    let normalization = target / center;
    let map = Arc::new(
        reference
            .estimate_flat_field(Rect {
                x: 16,
                y: 16,
                width: 352,
                height: 288,
            })
            .unwrap(),
    );
    assert!(
        (map.gain([192., 160.]).unwrap() - normalization)
            .abs()
            .max_element()
            < 2e-12
    );
    let mut scan = Scan::open(
        &fixture("illuminated-patches.dng"),
        Some(reference.fixed_wb),
    )
    .unwrap();
    assert_eq!(scan.dimensions().unwrap(), (384, 320));
    assert_eq!(scan.area, reference.area);
    assert_eq!(scan.camera_model, reference.camera_model);
    assert_eq!(scan.camera_to_ap1, reference.camera_to_ap1);
    scan.set_flat_field(Some(map.clone())).unwrap();
    (scan, map, normalization)
}

fn calibration(scan: &mut Scan) -> Calibration {
    Calibration {
        dmin: scan
            .reference(Rect {
                x: 16,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        dmax: scan
            .reference(Rect {
                x: 80,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap(),
        input_floor: scan.input_floor,
    }
}

#[test]
fn authored_bare_light_corrects_known_illumination_and_persists_the_measured_map() {
    let originals = ["bare-light.dng", "illuminated-patches.dng"]
        .map(|name| (fixture(name), fs::read(fixture(name)).unwrap()));
    let (mut scan, map, normalization) = corrected_scan();
    let plain = Scan::open(&fixture("illuminated-patches.dng"), Some(scan.fixed_wb)).unwrap();
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
            let relative = map
                .gain([point[0] as f64 + 0.5, point[1] as f64 + 0.5])
                .unwrap()
                / normalization;
            // Four authored sensor codes allow quantization/demosaicing and local-fit error,
            // not a recorded-output baseline or a relaxation of existing numerical gates.
            assert!((relative - illumination.recip()).abs().max_element() < 4. / 2400.);
            let expected =
                normalization * DVec3::from_array(patches[((y * 6 + x) % 12) as usize]) / 4031.;
            let allowance = 4. / 4031. * map.bounds().1.max_element() * normalization.max_element();
            assert!(
                (scan.pixel(point[0], point[1]).unwrap() - expected)
                    .abs()
                    .max_element()
                    < allowance
            );
        }
    }
    assert!(
        (plain.pixel(32, 32).unwrap() - scan.pixel(32, 32).unwrap())
            .abs()
            .max_element()
            > 0.02
    );
    assert_eq!(map.gain([8.5, 8.5]).unwrap(), DVec3::ONE);
    assert_eq!(scan.pixel(8, 8).unwrap(), plain.pixel(8, 8).unwrap());
    assert_eq!(scan.effective_floor(), map.precision_floor().unwrap());
    assert!(scan.effective_floor() > scan.input_floor);
    let invalid =
        Arc::new(FlatField::from_nodes([380, 0, 16, 16], [2, 2], vec![DVec3::ONE; 4]).unwrap());
    let before = scan.pixel(32, 32).unwrap();
    assert!(scan.set_flat_field(Some(invalid)).is_err());
    assert_eq!(scan.flat_field().unwrap(), &map);
    assert_eq!(scan.pixel(32, 32).unwrap(), before);
    let anchors = calibration(&mut scan);
    assert_eq!(
        scan.processing_calibration(anchors).unwrap().input_floor,
        scan.effective_floor()
    );

    #[cfg(feature = "project")]
    {
        use antitone::{
            processing::PhotoSettings,
            project::{
                Development, FlatFieldSettings, Project, Save, Source, SourceDevelopment, SourceId,
            },
        };
        let mut project = Project {
            development: Some(Development::new(antitone::raw::version(), scan.fixed_wb)),
            input_floor: Some(scan.input_floor),
            dmin: Some(anchors.dmin.to_array()),
            dmax: Some(anchors.dmax.to_array()),
            ..Project::default()
        };
        let mut reference_id = None;
        for name in ["illuminated-patches.dng", "bare-light.dng"] {
            let reference_only = name == "bare-light.dng";
            let id = project
                .add_source(Source {
                    id: SourceId(0),
                    path: fixture(name),
                    development: SourceDevelopment {
                        dimensions: [384, 320],
                        active_area: [8, 8, 368, 304],
                        camera_model: scan.camera_model.clone(),
                        camera_to_ap1: scan.camera_to_ap1.to_cols_array_2d(),
                        input_floor: scan.input_floor,
                    },
                    reference_only,
                    photo: PhotoSettings::default(),
                    orientation: Default::default(),
                    crop: None,
                })
                .unwrap();
            if reference_only {
                project.remove_from_filmstrip(id).unwrap();
                reference_id = Some(id);
            }
        }
        project.flat_field = Some(FlatFieldSettings::new(
            project.source(reference_id.unwrap()).unwrap(),
            map.clone(),
            project.development.as_ref().unwrap(),
        ));
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("corrected.antitone.json");
        project.save(&path, Save::New).unwrap();
        let reopened = Project::load(&path).unwrap();
        assert_eq!(reopened, project);
        assert_eq!(
            reopened.processing_calibration().unwrap().unwrap(),
            scan.processing_calibration(anchors).unwrap()
        );
        let mut restored = Scan::open(
            &fixture("illuminated-patches.dng"),
            Some(reopened.development.as_ref().unwrap().fixed_wb),
        )
        .unwrap();
        restored
            .set_flat_field(Some(reopened.active_flat_field().unwrap().map.clone()))
            .unwrap();
        for y in (8..312).step_by(7) {
            for x in (8..376).step_by(7) {
                assert_eq!(restored.pixel(x, y).unwrap(), scan.pixel(x, y).unwrap());
            }
        }
        scan.set_flat_field(None).unwrap();
        assert_eq!(scan.pixel(32, 32).unwrap(), plain.pixel(32, 32).unwrap());
        assert_eq!(scan.effective_floor(), scan.input_floor);
    }
    for (path, original) in originals {
        assert_eq!(fs::read(path).unwrap(), original);
    }
}

#[cfg(feature = "export")]
#[test]
fn authored_corrected_dng_exports_reopen_in_rgb16_and_ap0() {
    use antitone::{
        color::ap1_to_ap0,
        export::{self, ExrPrecision, ImageSize, Overwrite},
        processing::{PhotoSettings, Pipeline, RollSettings},
        profiles,
        rendering::{CpuRenderer, OutputTarget},
    };
    let (mut scan, _, _) = corrected_scan();
    let anchors = calibration(&mut scan);
    let roll = RollSettings::new_grade();
    let pipeline = Pipeline::new(
        scan.processing_calibration(anchors).unwrap(),
        roll,
        PhotoSettings::default(),
        scan.camera_to_ap1,
    )
    .unwrap();
    let size = ImageSize {
        width: scan.area.width,
        height: scan.area.height,
    };
    let working: Vec<_> = (0..size.height)
        .flat_map(|y| (0..size.width).map(move |x| (x, y)))
        .map(|(x, y)| {
            pipeline
                .apply(scan.pixel(scan.area.x + x, scan.area.y + y).unwrap())
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
        export::tiff(&path, size, target, Overwrite::Refuse, roll.response, pixel).unwrap();
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
    export::exr(
        &path,
        size,
        ExrPrecision::Half,
        Overwrite::Refuse,
        roll.response,
        pixel,
    )
    .unwrap();
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
