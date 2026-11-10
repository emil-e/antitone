use super::*;
use antitone::processing::{
    CAMERA_INPUT_FLOOR, Calibration, DEFAULT_GAMMA, InputSettings, Response,
};

fn vignette(order: Order, coefficients: &[i16]) -> Entry {
    (0x7032, 8, order.signed(coefficients))
}

#[test]
fn selected_ifd_shading_is_signed_bounded_independent_and_padding_is_not_data() {
    for order in [Order::Little, Order::Big] {
        let file = source(order, vec![vignette(order, &[2, 8192, 8192])], false, true);
        let raw = RawFile::open(file.path()).unwrap();
        assert_eq!(
            raw.metadata.vendor.sony_lens.vignetting_status(),
            SonyLensDataStatus::Available {
                camera_setting: None
            }
        );
        assert_eq!(
            raw.metadata.vendor.sony_lens.distortion_status(),
            SonyLensDataStatus::Absent
        );
        assert_eq!(
            raw.metadata.vendor.sony_lens.ca_status(),
            SonyLensDataStatus::Absent
        );
        let geometry = raw.metadata.lens_geometry().unwrap().unwrap();
        assert_eq!(geometry.size(), [64, 64]);
        assert_eq!(
            geometry.sample_positions(9, 13).unwrap(),
            [glam::DVec2::new(9., 13.); 3]
        );
        assert!(
            (raw.metadata.precision_floor().unwrap() / CAMERA_INPUT_FLOOR
                - std::f64::consts::SQRT_2)
                .abs()
                < 1e-14
        );
        let mut padded = vec![11];
        padded.extend([0; 11]);
        padded.extend([i16::MAX; 5]);
        let file = source(order, vec![vignette(order, &padded)], false, true);
        let mut raw = RawFile::open(file.path()).unwrap();
        assert!(raw.metadata.lens_identity().unwrap().is_none());
        assert_eq!(raw.metadata.precision_floor().unwrap(), CAMERA_INPUT_FLOOR);
        assert!(!raw.develop_camera([1.; 4]).unwrap().is_continuous());
        for tags in [
            vec![(0x7032, 3, order.signed(&[2, 0, 8192]))],
            vec![vignette(order, &[2, 0])],
            vec![vignette(order, &[0])],
            vec![vignette(order, &[17; 18])],
            vec![
                vignette(order, &[2, 0, 8192]),
                vignette(order, &[2, 0, 8192]),
            ],
        ] {
            let file = source(order, tags, false, false);
            assert!(RawFile::open(file.path()).is_err());
            let mut baseline = RawFile::open_uncorrected(file.path()).unwrap();
            assert!(baseline.metadata.vendor.sony_lens.profile().is_err());
            baseline.develop_camera([1.; 4]).unwrap();
        }
        let file = source(order, vec![vignette(order, &[2, 8192, 8192])], true, false);
        let raw = RawFile::open(file.path()).unwrap();
        assert_eq!(
            raw.metadata.vendor.sony_lens.vignetting_status(),
            SonyLensDataStatus::Absent
        );
    }
}

#[test]
fn shading_curve_provenance_differs_even_with_equal_geometry_and_precision_floor() {
    let a = source(
        Order::Little,
        vec![vignette(Order::Little, &[3, 0, 8192, 16384])],
        false,
        false,
    );
    let b = source(
        Order::Little,
        vec![vignette(Order::Little, &[3, 8192, 8192, 16384])],
        false,
        false,
    );
    let a = antitone::scan::Scan::open(a.path(), Some([1.; 4])).unwrap();
    let b = antitone::scan::Scan::open(b.path(), Some([1.; 4])).unwrap();
    let first = development(&a);
    let second = development(&b);
    assert_eq!(first.dimensions, second.dimensions);
    assert_eq!(a.source().precision_floor(), b.source().precision_floor());
    assert_ne!(
        a.source().metadata().lens_identity().unwrap(),
        b.source().metadata().lens_identity().unwrap()
    );
    assert_ne!(a.pixel(32, 32).unwrap(), b.pixel(32, 32).unwrap());
}

#[test]
fn gains_are_applied_at_original_sites_before_the_combined_channel_reconstruction() {
    let order = Order::Little;
    let mut tags = profile_tags(order);
    tags.push(vignette(order, &[3, 0, 8192, 16384]));
    let file = source(order, tags, false, false);
    let mut raw = RawFile::open(file.path()).unwrap();
    let geometry = raw.metadata.lens_geometry().unwrap().unwrap();
    let mut native = RawFile::open_uncorrected(file.path()).unwrap();
    let native = native.develop_camera([1.; 4]).unwrap();
    let image = raw.develop_camera([1.; 4]).unwrap();
    let mut differs_from_gain_after_warp = false;
    for (x, y) in [(0, 0), (9, 7), (17, 23), (32, 32), (63, 63)] {
        let actual = image.pixel_samples(x, y).unwrap();
        for (c, point) in geometry
            .sample_positions(x, y)
            .unwrap()
            .into_iter()
            .enumerate()
        {
            let left = point.x.floor() as u32;
            let top = point.y.floor() as u32;
            let right = point.x.ceil() as u32;
            let bottom = point.y.ceil() as u32;
            let gain = |x: f64, y: f64| {
                // Original 64x64 pixel centres, independent of the lens implementation.
                let r = ((x - 31.5).hypot(y - 31.5)) / 32f64.hypot(32.);
                let attenuation = if r <= 0.25 {
                    1.
                } else if r <= 0.75 {
                    1. + (1. / 2f64.sqrt() - 1.) * (r - 0.25) * 2.
                } else {
                    1. / 2f64.sqrt() + (1. / 2f64.powf(1.5) - 1. / 2f64.sqrt()) * (r - 0.75) * 2.
                };
                1. / attenuation
            };
            let mut expected = 0.;
            let mut unshaded = 0.;
            let u = point.x - left as f64;
            let v = point.y - top as f64;
            for (xx, yy, weight) in [
                (left, top, (1. - u) * (1. - v)),
                (right, top, u * (1. - v)),
                (left, bottom, (1. - u) * v),
                (right, bottom, u * v),
            ] {
                let code = native.pixel_codes(xx, yy).unwrap()[c] as f64;
                expected += weight * code * gain(xx as f64, yy as f64);
                unshaded += weight * code;
            }
            assert!(
                (actual[c] as f64 - expected).abs() <= 0.004,
                "{x},{y} channel={c}: {} != {expected}",
                actual[c]
            );
            differs_from_gain_after_warp |=
                (expected - unshaded * gain(point.x, point.y)).abs() > 0.01;
        }
    }
    assert!(
        differs_from_gain_after_warp,
        "fixture must distinguish the processing order"
    );
}

#[test]
fn headroom_measurements_and_source_floor_compose_with_residual_flat_field() {
    use antitone::{
        flat_field::FlatField,
        project::{FlatFieldSettings, Project, Save, Source, SourceId},
    };
    use std::sync::Arc;
    let file = source(
        Order::Little,
        vec![vignette(Order::Little, &[2, 8192, 8192])],
        false,
        false,
    );
    let mut scan = antitone::scan::Scan::open(file.path(), Some([1.; 4])).unwrap();
    let base = Calibration {
        dmin: DVec3::splat(0.7),
        dmax: DVec3::splat(0.01),
    };
    assert_eq!(scan.precision_floor, scan.source().precision_floor());
    let developed_floor = scan.source().precision_floor();
    assert!(developed_floor > CAMERA_INPUT_FLOOR);
    assert_eq!(scan.effective_floor(), developed_floor);
    let region = antitone::raw::Rect::from([56, 56, 8, 8]);
    assert!(scan.pixel(60, 60).unwrap().max_element() > 1.);
    assert!(scan.reference(region).unwrap().max_element() > 1.);
    let samples = scan.analysis_samples(region, 1, || false).unwrap().unwrap();
    assert_eq!(samples.pixels.len(), 1);
    let estimated = scan.estimate_flat_field(scan.area).unwrap();
    let center_camera = scan
        .source()
        .camera_image()
        .unwrap()
        .sample_mean(scan.area, glam::DMat3::IDENTITY)
        .unwrap();
    let neutral = scan.camera_to_ap1 * (center_camera * estimated.gain([32., 32.]).unwrap());
    assert!((neutral - DVec3::splat(neutral.y)).abs().max_element() < 1e-12);
    let mut project = Project {
        input: InputSettings::Negative {
            dmin: Some(DVec3::splat(0.7)),
            dmax: Some(DVec3::splat(0.01)),
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Default::default(),
        },
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: file.path().into(),
            development: development(&scan),

            repairs: Default::default(),
            adjustment_layers: Some(antitone::project::AdjustmentLayers::with_global(
                Default::default(),
            )),
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    assert_eq!(
        project
            .image_recipe(id)
            .unwrap()
            .neutral
            .input
            .calibration()
            .unwrap(),
        base
    );
    let field =
        Arc::new(FlatField::from_nodes([0, 0, 64, 64], [2, 2], vec![DVec3::splat(3.); 4]).unwrap());
    scan.set_flat_field(Some(field.clone()));
    project
        .set_flat_field(Some(FlatFieldSettings::new(
            project.source(id).unwrap().path.clone(),
            field,
        )))
        .unwrap();
    let recipe = project.image_recipe(id).unwrap();
    assert_eq!(
        (base).validate().unwrap(),
        recipe.neutral.input.calibration().unwrap()
    );
    assert_eq!(scan.effective_floor(), developed_floor * 3.);
    recipe.neutral.require_scan(&scan).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("shaded.antitone");
    project.save(&saved, Save::New).unwrap();
    let reopened = Project::load(&saved).unwrap();
    reopened
        .image_recipe(id)
        .unwrap()
        .neutral
        .require_scan(&scan)
        .unwrap();
    let mut changed = reopened.clone();
    changed.set_flat_field(None).unwrap();
    assert!(
        changed
            .image_recipe(id)
            .unwrap()
            .neutral
            .require_scan(&scan)
            .is_err()
    );
    // Small positive anchors remain usable independently of the measurement precision estimate.
    let mut low_light = reopened;
    if let InputSettings::Negative { dmax, .. } = &mut low_light.input {
        *dmax = Some(DVec3::splat(CAMERA_INPUT_FLOOR));
    }
    low_light.validate().unwrap();
    scan.set_flat_field(None);
    assert_eq!(scan.effective_floor(), developed_floor);
}
