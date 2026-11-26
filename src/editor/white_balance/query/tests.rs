use super::*;
use crate::photographic::stages::grading::Grading;
use crate::{
    adjustment_layers::AdjustmentLayerCoverage,
    geometry::{SourcePoint, SourceRegion},
};

#[test]
fn weight_images_preserve_original_coordinates_for_identity_controls_and_disabled_layers() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let services = editor.source_services.as_ref().unwrap();
    let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
    let patch = SourceRegion {
        x: ready.area.x + ready.area.width / 3,
        y: ready.area.y + ready.area.height / 3,
        width: ready.area.width / 3,
        height: ready.area.height / 3,
    };
    let center = [
        f64::from(patch.x) + f64::from(patch.width) / 2.,
        f64::from(patch.y) + f64::from(patch.height) / 2.,
    ];
    let linear = AdjustmentLayerCoverage::Linear {
        start: SourcePoint([f64::from(patch.x), center[1]]),
        end: SourcePoint([f64::from(patch.x + patch.width), center[1]]),
    };
    let radial = AdjustmentLayerCoverage::Radial {
        center: SourcePoint(center),
        radii: [f64::from(patch.width), f64::from(patch.height)],
        angle: 0.3,
        feather: 1.,
    };
    let mut scan = measurement::capture_scan(&editor).unwrap();
    let samples = Samples::new(
        (&services.evaluation, &services.queue),
        &mut scan,
        None,
        Coverage::rectangle(patch),
        &|| false,
    )
    .unwrap();

    for (mask, invert, strength, enabled) in [
        (AdjustmentLayerCoverage::Global {}, false, 1., true),
        (linear, true, 0.7, true),
        (radial, false, 0.65, true),
        (radial, true, 0.8, false),
        (AdjustmentLayerCoverage::Global {}, true, 0.8, true),
        (linear, false, 0., true),
    ] {
        let selected = AdjustmentLayer {
            mask,
            invert,
            strength,
            enabled,
            photo: AdjustmentLayerSettings::default(),
        };
        let image = Grading::weight_image(samples.source.clone(), selected).unwrap();
        let mut count = 0;
        let mut range = [1_f64, 0_f64];
        samples
            .read(image, &|| false, |point, pixel, _| {
                let expected = selected
                    .weight(SourcePoint([
                        f64::from(point[0]) + 0.5,
                        f64::from(point[1]) + 0.5,
                    ]))
                    .unwrap();
                assert!(
                    (f64::from(pixel[0]) - expected).abs() < 2e-5,
                    "{selected:?} {point:?}: {pixel:?} != {expected}"
                );
                assert_eq!(pixel[1], pixel[0]);
                assert_eq!(pixel[2], pixel[0]);
                assert_eq!(pixel[3], 0.);
                range[0] = range[0].min(expected);
                range[1] = range[1].max(expected);
                count += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(count, patch.width * patch.height);
        if enabled
            && strength > 0.
            && matches!(
                mask,
                AdjustmentLayerCoverage::Linear { .. } | AdjustmentLayerCoverage::Radial { .. }
            )
        {
            assert!(
                range[1] - range[0] > 0.1,
                "the fixture must distinguish spatial weights from a constant"
            );
        }
    }
}

#[test]
fn layer_queries_reject_zero_effective_coverage_and_cancel_without_poisoning_later_queries() {
    let editor = crate::editor::tests::accepted_input(
        crate::editor::tests::authored_fixture(&["patches.dng"]),
        InputSettings::Digital,
    );
    let services = editor.source_services.as_ref().unwrap();
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let coverage = Coverage::rectangle(SourceRegion {
        x: area.x + area.width / 3,
        y: area.y + area.height / 3,
        width: area.width / 3,
        height: area.height / 3,
    });
    let input = |enabled, invert, strength| {
        let mut recipe = editor
            .positive_recipe(
                InputSettings::Digital,
                RollSettings::default(),
                AdjustmentLayerSettings::default(),
            )
            .unwrap();
        recipe.grade = AdjustmentLayerSnapshot::new(vec![AdjustmentLayer {
            enabled,
            invert,
            strength,
            ..AdjustmentLayer::global(AdjustmentLayerSettings::default())
        }])
        .unwrap();
        LayerInput { recipe, index: 0 }
    };
    let mut scan = measurement::capture_scan(&editor).unwrap();
    for (enabled, invert, strength) in [(false, false, 1.), (true, false, 0.), (true, true, 1.)] {
        assert!(matches!(
            layer(
                (&services.evaluation, &services.queue),
                &mut scan,
                None,
                coverage,
                input(enabled, invert, strength),
                &|| false
            ),
            Err(Error::UnusableSample(_))
        ));
    }
    assert!(matches!(
        layer(
            (&services.evaluation, &services.queue),
            &mut scan,
            None,
            coverage,
            input(true, false, 1.),
            &|| true
        ),
        Err(Error::Cancelled)
    ));
    let checks = std::cell::Cell::new(0);
    assert!(matches!(
        layer(
            (&services.evaluation, &services.queue),
            &mut scan,
            None,
            coverage,
            input(true, false, 1.),
            &|| {
                checks.set(checks.get() + 1);
                checks.get() > 24
            }
        ),
        Err(Error::Cancelled)
    ));
    let balance = layer(
        (&services.evaluation, &services.queue),
        &mut scan,
        None,
        coverage,
        input(true, false, 1.),
        &|| false,
    )
    .unwrap();
    assert_ne!(balance, Balance::default());
}

#[test]
fn white_balance_queries_include_black_camera_samples_in_a_usable_patch() {
    let file = crate::raw::fixture::bayer_dng_with_as_shot_samples([1; 3], |x, y| {
        if (20..36).contains(&x) && (20..36).contains(&y) {
            [0; 3]
        } else {
            [1800, 2000, 2200]
        }
    });
    let path = file.path().to_str().unwrap();
    let dmin = DVec3::splat(0.5);
    let dmax = DVec3::splat(0.025);
    let negative = InputSettings::Negative {
        dmin: Some(dmin),
        dmax: Some(dmax),
        response: crate::processing::Response::film(),
        gamma: crate::processing::DEFAULT_GAMMA,
        balance: Balance::default(),
    };

    for input in [InputSettings::Digital, negative] {
        let editor = crate::editor::tests::authored_fixture(&[path]);
        let editor = if input == InputSettings::Digital {
            crate::editor::tests::accepted_input(editor, input)
        } else {
            editor
        };
        let services = editor.source_services.as_ref().unwrap();
        let mut scan = measurement::capture_scan(&editor).unwrap();
        let coverage = Coverage::rectangle(scan.area);
        let mut black = false;
        let mut lit = false;
        for ([x, y], _) in coverage.pixels(scan.area).unwrap() {
            let pixel = scan.pixel(x, y).unwrap();
            black |= pixel == DVec3::ZERO;
            lit |= pixel.min_element() > 0.;
        }
        assert!(
            black && lit,
            "the developed patch must contain both black and lit samples"
        );

        let expected = scan
            .white_balance_coverage(
                coverage,
                &input,
                RollSettings::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                None,
            )
            .unwrap();
        let recipe = editor
            .positive_recipe(
                input,
                RollSettings::default(),
                AdjustmentLayerSettings::default(),
            )
            .unwrap();
        let gpu = (&services.evaluation, &services.queue);
        let roll_balance = roll(gpu, &mut scan, None, coverage, recipe.clone(), &|| false).unwrap();
        let layer_balance = layer(
            gpu,
            &mut scan,
            None,
            coverage,
            LayerInput { recipe, index: 0 },
            &|| false,
        )
        .unwrap();

        for actual in [roll_balance, layer_balance] {
            assert!(
                (actual.warmth - expected.warmth).abs() < 2e-6,
                "{actual:?} {expected:?}"
            );
            assert!(
                (actual.tint - expected.tint).abs() < 2e-6,
                "{actual:?} {expected:?}"
            );
        }
    }
}

#[test]
fn white_balance_queries_keep_signed_working_light_and_recorded_hdr_exposure() {
    let file = crate::raw::fixture::bayer_dng_with_camera(
        [260, 1000, 600],
        glam::DMat3::IDENTITY.to_cols_array(),
    );
    let editor = crate::editor::tests::accepted_input(
        crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]),
        InputSettings::Digital,
    );
    let services = editor.source_services.as_ref().unwrap();
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let coverage = Coverage::rectangle(SourceRegion {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 32,
    });

    for ev in [0., 8.] {
        let mut scan = measurement::capture_scan(&editor).unwrap();
        scan.reference_exposure = Some(crate::exposure_reference::ReferenceExposure::Matched(
            crate::exposure_reference::StartingExposure {
                ev,
                samples: 1024,
                spread: 0.,
            },
        ));
        let working = crate::scan::ImageSource::new(&scan, InputSettings::Digital)
            .unwrap()
            .sample([area.x + 16, area.y + 16])
            .unwrap();
        assert!(
            working.min_element() < 0.,
            "the fixture must exercise signed working RGB: {working:?}"
        );
        if ev != 0. {
            assert!(
                working.max_element() > 1.,
                "the recorded exposure must exercise HDR light: {working:?}"
            );
        }
        let reference = scan
            .white_balance_coverage(
                coverage,
                &InputSettings::Digital,
                RollSettings::default(),
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                None,
            )
            .unwrap();
        let mut recipe = editor
            .positive_recipe(
                InputSettings::Digital,
                RollSettings::default(),
                AdjustmentLayerSettings::default(),
            )
            .unwrap();
        recipe.neutral.reference = scan.reference_exposure.clone();
        let roll_balance = roll(
            (&services.evaluation, &services.queue),
            &mut scan,
            None,
            coverage,
            recipe.clone(),
            &|| false,
        )
        .unwrap();
        let layer_balance = layer(
            (&services.evaluation, &services.queue),
            &mut scan,
            None,
            coverage,
            LayerInput { recipe, index: 0 },
            &|| false,
        )
        .unwrap();

        for actual in [roll_balance, layer_balance] {
            assert!(
                (actual.warmth - reference.warmth).abs() < 2e-6,
                "{actual:?} {reference:?}"
            );
            assert!(
                (actual.tint - reference.tint).abs() < 2e-6,
                "{actual:?} {reference:?}"
            );
            let neutral = crate::color::wb_matrix(actual).unwrap() * working;
            assert!(
                (neutral - DVec3::splat(neutral.y)).abs().max_element()
                    / neutral.abs().max_element()
                    < 2e-6,
                "{actual:?} {neutral:?}"
            );
        }
    }
}
