use super::*;
use crate::{
    adjustment_layers::AdjustmentLayerCoverage,
    geometry::{SourcePoint, SourceRegion},
    gpu::evaluation,
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
    let samples = Samples::new(services, &mut scan, Coverage::rectangle(patch), &|| false).unwrap();

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
        let image =
            Grading::weight_image(&services.device, samples.source.clone(), selected).unwrap();
        assert_eq!(image.domain.meaning, evaluation::Meaning::Weight);
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
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let services = editor.source_services.as_ref().unwrap();
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let coverage = Coverage::rectangle(SourceRegion {
        x: area.x + area.width / 3,
        y: area.y + area.height / 3,
        width: area.width / 3,
        height: area.height / 3,
    });
    let input = |enabled, invert, strength| LayerInput {
        input: InputSettings::Digital,
        roll: RollSettings::default(),
        upstream: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
        layer: AdjustmentLayer {
            enabled,
            invert,
            strength,
            ..AdjustmentLayer::global(AdjustmentLayerSettings::default())
        },
    };
    let mut scan = measurement::capture_scan(&editor).unwrap();
    for (enabled, invert, strength) in [(false, false, 1.), (true, false, 0.), (true, true, 1.)] {
        assert!(matches!(
            layer(
                services,
                &mut scan,
                coverage,
                input(enabled, invert, strength),
                &|| false
            ),
            Err(Error::UnusableSample(_))
        ));
    }
    assert!(matches!(
        layer(
            services,
            &mut scan,
            coverage,
            input(true, false, 1.),
            &|| true
        ),
        Err(Error::Cancelled)
    ));
    let checks = std::cell::Cell::new(0);
    assert!(matches!(
        layer(
            services,
            &mut scan,
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
        services,
        &mut scan,
        coverage,
        input(true, false, 1.),
        &|| false,
    )
    .unwrap();
    assert_ne!(balance, Balance::default());
}

#[test]
fn white_balance_queries_keep_signed_working_light_and_recorded_hdr_exposure() {
    let file = crate::raw::fixture::bayer_dng_with_camera(
        [260, 1000, 600],
        glam::DMat3::IDENTITY.to_cols_array(),
    );
    let editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
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
        let roll_balance = roll(
            services,
            &mut scan,
            coverage,
            RollInput {
                input: InputSettings::Digital,
                roll: RollSettings::default(),
            },
            &|| false,
        )
        .unwrap();
        let layer_balance = layer(
            services,
            &mut scan,
            coverage,
            LayerInput {
                input: InputSettings::Digital,
                roll: RollSettings::default(),
                upstream: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                layer: AdjustmentLayer::global(AdjustmentLayerSettings::default()),
            },
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
