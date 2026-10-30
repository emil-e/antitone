use super::measurement::testing;
use super::*;
use crate::{editor::Pick, raw::Rect};
use glam::DVec3;
use std::time::Duration;

#[test]
fn roll_wb_feature_measures_before_its_existing_balance_and_all_adjustment_layers() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.load_input(InputSettings::Digital);
    editor.document.roll.wb = Balance {
        warmth: 0.3,
        tint: -0.2,
    };
    editor.document.visit.adjustment_layer_settings.wb = Balance {
        warmth: -0.4,
        tint: 0.2,
    };
    editor.sync_document();
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let region = crate::coverage::Coverage::rectangle(Rect {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 32,
    });
    let reference = measurement::capture_scan(&editor)
        .unwrap()
        .white_balance_coverage(
            region,
            &editor.input(),
            RollSettings {
                wb: Balance::default(),
                ..editor.document.roll
            },
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            None,
        )
        .unwrap();
    let expected = RollInput::capture(&editor)
        .measure(
            &mut measurement::capture_scan(&editor).unwrap(),
            region,
            editor.source_services.as_ref().unwrap(),
            &|| false,
        )
        .unwrap();
    // The independent f64 path checks the captured prefix; production evaluates f32 pixels.
    assert!((expected.warmth - reference.warmth).abs() < 1e-6);
    assert!((expected.tint - reference.tint).abs() < 1e-6);
    let input = editor.document.input;
    let photo = editor.document.visit.adjustment_layer_settings;
    assert_ne!(expected, editor.document.roll.wb);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_pick(Pick::RollWb, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert_eq!(editor.document.roll.wb, expected);
            assert_eq!(editor.document.input, input);
            assert_eq!(editor.document.visit.adjustment_layer_settings, photo);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                Some("Calibration pick")
            );
            assert!(editor.active_pick().is_none());
        });
    });
}

#[test]
fn layer_white_balance_pick_captures_only_preceding_layers_and_fits_a_replacement() {
    use crate::{adjustment_layers::AdjustmentLayer, project::AdjustmentLayers};
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.document.input = crate::processing::InputSettings::Digital;
    editor.document.roll = RollSettings::default();
    let source = editor.document.visit.current_source.unwrap();
    let upstream = Balance {
        warmth: 0.2,
        tint: -0.1,
    };
    let mut layers = AdjustmentLayers::with_global(AdjustmentLayerSettings {
        wb: upstream,
        exposure: 1.,
        ..AdjustmentLayerSettings::default()
    });
    let selected = layers
        .add(
            "Selected".into(),
            AdjustmentLayer::global(AdjustmentLayerSettings {
                wb: Balance {
                    warmth: 0.4,
                    tint: 0.3,
                },
                exposure: 0.5,
                ..AdjustmentLayerSettings::default()
            }),
        )
        .unwrap();
    layers
        .add(
            "Downstream".into(),
            AdjustmentLayer::global(AdjustmentLayerSettings {
                wb: Balance {
                    warmth: -0.7,
                    tint: 0.6,
                },
                ..AdjustmentLayerSettings::default()
            }),
        )
        .unwrap();
    editor
        .document
        .project
        .source_mut(source)
        .unwrap()
        .adjustment_layers = Some(layers);
    editor.refresh_adjustment_layer_selection();
    editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(selected));
    let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
    let region = Coverage::rectangle(Rect {
        x: ready.area.x + ready.area.width / 3,
        y: ready.area.y + ready.area.height / 3,
        width: ready.area.width / 3,
        height: ready.area.height / 3,
    });
    let captured = LayerInput::capture(&editor).unwrap();
    let mut captured_image = measurement::capture_scan(&editor).unwrap();
    let mut scan = Scan::from_source(ready.developed.clone());
    let baseline = scan
        .white_balance_coverage(
            region,
            &editor.document.input,
            editor.document.roll,
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            None,
        )
        .unwrap();
    // Bradford gains add in log coordinates; the selected replacement cancels the upstream WB.
    let expected = Balance {
        warmth: baseline.warmth - upstream.warmth,
        tint: baseline.tint - upstream.tint,
    };
    editor
        .edit_adjustment_layers(|layers| {
            layers.layers[0].adjustment.photo.wb = Balance::default();
            Ok(())
        })
        .unwrap();
    let actual = captured
        .measure(
            &mut captured_image,
            region,
            editor.source_services.as_ref().unwrap(),
            &|| false,
        )
        .unwrap();
    // Compare the shared f32 forward model with the independent f64 log-gain identity.
    assert!(
        (actual.warmth - expected.warmth).abs() < 2e-6,
        "{actual:?} != {expected:?}"
    );
    assert!(
        (actual.tint - expected.tint).abs() < 2e-6,
        "{actual:?} != {expected:?}"
    );
}

#[test]
fn masked_and_partial_layer_wb_picks_neutralize_the_captured_patch_before_own_creative_controls() {
    use crate::{
        adjustment_layers::AdjustmentLayerCoverage, geometry::SourcePoint,
        project::AdjustmentLayers,
    };
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.document.input = InputSettings::Digital;
    editor.document.roll = RollSettings::default();
    let source = editor.document.visit.current_source.unwrap();
    let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
    let developed = ready.developed.clone();
    let patch = Rect {
        x: ready.area.x + ready.area.width / 3,
        y: ready.area.y + ready.area.height / 3,
        width: ready.area.width / 3,
        height: ready.area.height / 3,
    };
    let center = [
        f64::from(patch.x) + f64::from(patch.width) / 2.,
        f64::from(patch.y) + f64::from(patch.height) / 2.,
    ];
    for (mask, invert, strength) in [
        (AdjustmentLayerCoverage::Global {}, false, 0.45),
        (
            AdjustmentLayerCoverage::Linear {
                start: SourcePoint([f64::from(patch.x), center[1]]),
                end: SourcePoint([f64::from(patch.x + patch.width), center[1]]),
            },
            true,
            0.7,
        ),
        (
            AdjustmentLayerCoverage::Radial {
                center: SourcePoint(center),
                radii: [f64::from(patch.width), f64::from(patch.height)],
                angle: 0.3,
                feather: 1.,
            },
            false,
            0.65,
        ),
    ] {
        let selected = AdjustmentLayer {
            mask,
            invert,
            strength,
            enabled: true,
            photo: AdjustmentLayerSettings {
                exposure: 0.5,
                wb: Balance {
                    warmth: 0.8,
                    tint: -0.7,
                },
                contrast: 1.5,
                saturation: 1.8,
                ..Default::default()
            },
        };
        let mut layers = AdjustmentLayers::default();
        let id = layers.add("Selected".into(), selected).unwrap();
        layers
            .add(
                "Finishing".into(),
                AdjustmentLayer::global(AdjustmentLayerSettings {
                    wb: Balance {
                        warmth: -0.5,
                        tint: 0.4,
                    },
                    ..Default::default()
                }),
            )
            .unwrap();
        editor
            .document
            .project
            .source_mut(source)
            .unwrap()
            .adjustment_layers = Some(layers);
        editor.refresh_adjustment_layer_selection();
        editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(id));
        let balance = LayerInput::capture(&editor)
            .unwrap()
            .measure(
                &mut measurement::capture_scan(&editor).unwrap(),
                Coverage::rectangle(patch),
                editor.source_services.as_ref().unwrap(),
                &|| false,
            )
            .unwrap();
        let mut measured = selected;
        measured.photo.wb = balance;
        measured.photo.contrast = 1.;
        measured.photo.saturation = 1.;
        let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
            InputSettings::Digital,
            editor.document.roll,
            AdjustmentLayerSnapshot::new(vec![measured]).unwrap(),
            developed.camera_to_ap1(),
        )
        .unwrap();
        let scan = Scan::from_source(developed.clone());
        let mut rgb = DVec3::ZERO;
        for y in patch.y..patch.y + patch.height {
            for x in patch.x..patch.x + patch.width {
                rgb += pipeline
                    .apply(scan.pixel(x, y).unwrap(), [x, y])
                    .unwrap()
                    .rgb;
            }
        }
        rgb /= f64::from(patch.width * patch.height);
        assert!(
            (rgb - DVec3::splat(rgb.y)).abs().max_element() / rgb.abs().max_element() < 2e-6,
            "{mask:?}, {balance:?}: {rgb:?}"
        );
    }
}

#[test]
fn changing_layer_before_the_wb_caller_is_polled_withdraws_it_and_a_new_pick_can_apply() {
    use crate::{adjustment_layers::AdjustmentLayer, editor::adjustment_layers::Target};
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.load_input(InputSettings::Digital);
    editor
        .edit_adjustment_layers(|layers| {
            layers.add(
                "Other".into(),
                AdjustmentLayer::global(AdjustmentLayerSettings {
                    wb: Balance {
                        warmth: 0.4,
                        tint: -0.2,
                    },
                    ..Default::default()
                }),
            )?;
            Ok(())
        })
        .unwrap();
    let source = editor.document.visit.current_source.unwrap();
    let layers = &editor
        .document
        .project
        .source(source)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers;
    let first = layers[0].id;
    let second = layers.last().unwrap().id;
    assert_ne!(first, second);
    let first_adjustment = layers[0].adjustment;
    editor.select_adjustment_layer(Target::Layer(first));
    editor.sync_document();
    let before = editor.document.state.project.clone();
    let undo = editor
        .history_label(crate::editor::history::Direction::Undo)
        .map(str::to_owned);
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let region = Coverage::rectangle(Rect {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 32,
    });
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.begin_pick(Pick::Wb, cx);
            editor.select_adjustment_layer(Target::Layer(second));
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
        });
        app.pump();
        app.read(|editor| {
            assert!(editor.active_pick().is_none() && !editor.busy);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                undo.as_deref()
            );
        });
        let expected = app.read(|editor| {
            LayerInput::capture(editor)
                .unwrap()
                .measure(
                    &mut measurement::capture_scan(editor).unwrap(),
                    region,
                    editor.source_services.as_ref().unwrap(),
                    &|| false,
                )
                .unwrap()
        });
        assert_ne!(
            expected,
            app.read(|editor| editor.document.visit.adjustment_layer_settings.wb)
        );
        app.enter(|editor, cx| editor.begin_pick(Pick::Wb, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
            assert_eq!(editor.document.visit.adjustment_layer_settings.wb, expected);
            assert_eq!(
                editor
                    .document
                    .project
                    .source(source)
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap()
                    .layers[0]
                    .adjustment,
                first_adjustment
            );
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                Some("Calibration pick")
            );
        });
    });
}

#[test]
fn wb_picking_keeps_its_selected_layer_until_its_awaiting_feature_applies_the_value() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.load_input(InputSettings::Digital);
    editor.sync_document();
    let source = editor.document.visit.current_source.unwrap();
    let first = editor
        .document
        .project
        .source(source)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers[0]
        .id;
    let second_settings = AdjustmentLayerSettings {
        exposure: 0.5,
        ..Default::default()
    };
    let second = editor
        .document
        .project
        .source_mut(source)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .add("Other".into(), AdjustmentLayer::global(second_settings))
        .unwrap();
    editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(first));
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let region = Coverage::rectangle(Rect {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 32,
    });
    let balance = LayerInput::capture(&editor)
        .unwrap()
        .measure(
            &mut measurement::capture_scan(&editor).unwrap(),
            region,
            editor.source_services.as_ref().unwrap(),
            &|| false,
        )
        .unwrap();
    assert_ne!(balance, editor.document.visit.adjustment_layer_settings.wb);
    let gate = testing::Gate::install(&mut editor);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_pick(Pick::Wb, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        let call = gate.next(app);
        app.update(|editor| {
            editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(second));
            assert_eq!(
                editor.adjustment_layer_target(),
                crate::editor::adjustment_layers::Target::Layer(first)
            );
            assert!(editor.measurement_pending());
        });
        call.run();
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert!(!editor.measurement_pending());
            assert!(editor.active_pick().is_none());
            let layers = &editor
                .document
                .project
                .source(source)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers;
            assert_eq!(
                layers
                    .iter()
                    .find(|l| l.id == first)
                    .unwrap()
                    .adjustment
                    .photo
                    .wb,
                balance
            );
            assert_eq!(
                layers
                    .iter()
                    .find(|l| l.id == second)
                    .unwrap()
                    .adjustment
                    .photo,
                second_settings
            );
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                Some("Calibration pick")
            );
        });
        app.update(|editor| {
            editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(second));
            assert_eq!(
                editor.adjustment_layer_target(),
                crate::editor::adjustment_layers::Target::Layer(second)
            );
        });
    });
}

#[test]
fn photo_wb_pick_uses_its_captured_baseline_before_upstream_contrast() {
    use crate::exposure_reference::{ReferenceExposure, StartingExposure};
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.document.input = InputSettings::Digital;
    editor.document.state.project.input = InputSettings::Digital;
    editor.document.roll = RollSettings::default();
    let id = editor.document.visit.current_source.unwrap();
    let baseline = ReferenceExposure::Matched(StartingExposure {
        ev: 2.75,
        samples: 1024,
        spread: 0.,
    });
    editor
        .document
        .visit
        .canvas
        .ready
        .as_mut()
        .unwrap()
        .reference = Some(baseline.clone());
    let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
    let scan = Scan::with_reference(ready.developed.clone(), Some(baseline));
    let patch = Rect {
        x: scan.area.x + 8,
        y: scan.area.y + 8,
        width: 32,
        height: 32,
    };
    let mut layers = crate::project::AdjustmentLayers::with_global(AdjustmentLayerSettings {
        contrast: 0.6,
        ..Default::default()
    });
    let upstream = layers.layers[0].adjustment;
    let selected = layers
        .add(
            "WB target".into(),
            AdjustmentLayer::global(Default::default()),
        )
        .unwrap();
    editor
        .document
        .project
        .source_mut(id)
        .unwrap()
        .adjustment_layers = Some(layers);
    editor.refresh_adjustment_layer_selection();
    editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(selected));

    let expected_signature = crate::scan::ImageSource::new(&scan, InputSettings::Digital)
        .unwrap()
        .gpu_program()
        .unwrap()
        .signature()
        .unwrap();
    assert_eq!(
        editor
            .neutral_source_signature(&InputSettings::Digital)
            .unwrap(),
        expected_signature
    );
    let captured = LayerInput::capture(&editor).unwrap();
    let mut captured_image = measurement::capture_scan(&editor).unwrap();
    let mut zero_image = measurement::capture_scan(&editor).unwrap();
    zero_image.reference_exposure = None;
    let control = LayerInput::capture(&editor)
        .unwrap()
        .measure(
            &mut zero_image,
            Coverage::rectangle(patch),
            editor.source_services.as_ref().unwrap(),
            &|| false,
        )
        .unwrap();
    editor
        .document
        .visit
        .canvas
        .ready
        .as_mut()
        .unwrap()
        .reference = None;
    let balance = captured
        .measure(
            &mut captured_image,
            Coverage::rectangle(patch),
            editor.source_services.as_ref().unwrap(),
            &|| false,
        )
        .unwrap();

    let rendered = |balance| {
        let target = AdjustmentLayer::global(AdjustmentLayerSettings {
            wb: balance,
            ..Default::default()
        });
        let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
            InputSettings::Digital,
            RollSettings::default(),
            AdjustmentLayerSnapshot::new(vec![upstream, target]).unwrap(),
            scan.positive_matrix(&InputSettings::Digital).unwrap(),
        )
        .unwrap();
        let mut mean = DVec3::ZERO;
        for y in patch.y..patch.y + patch.height {
            for x in patch.x..patch.x + patch.width {
                mean += pipeline
                    .apply(scan.pixel(x, y).unwrap(), [x, y])
                    .unwrap()
                    .rgb;
            }
        }
        mean / f64::from(patch.width * patch.height)
    };
    let correct = rendered(balance);
    let wrong = rendered(control);
    let chroma =
        |rgb: DVec3| (rgb - DVec3::splat(rgb.y)).abs().max_element() / rgb.abs().max_element();
    assert!(
        chroma(wrong) > 1e-3,
        "control must distinguish the lost baseline: {wrong:?}"
    );
    assert!(
        chroma(correct) < 2e-6,
        "captured pick must neutralize the displayed patch: {correct:?}"
    );
}
