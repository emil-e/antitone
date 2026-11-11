use super::measurement::testing;
use super::*;
use crate::{editor::PickerCase, raw::Rect};
use glam::DVec3;
use std::time::Duration;

#[test]
fn roll_wb_feature_measures_before_its_existing_balance_and_all_adjustment_layers() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor = crate::editor::tests::accepted_input(editor, InputSettings::Digital);
    editor.document.project.roll.wb = Balance {
        warmth: 0.3,
        tint: -0.2,
    };
    editor.photo_settings_mut().wb = Balance {
        warmth: -0.4,
        tint: 0.2,
    };
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
                ..editor.document.project.roll
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
    let input = editor.document.project.input;
    let photo = editor.photo_settings();
    assert_ne!(expected, editor.document.project.roll.wb);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::RollWb, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.roll.wb, expected);
            assert_eq!(editor.document.project.input, input);
            assert_eq!(editor.photo_settings(), photo);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                Some("Calibration pick")
            );
            assert!(editor.test_pick().is_none());
        });
    });
}

#[test]
fn layer_white_balance_pick_captures_only_preceding_layers_and_fits_a_replacement() {
    use crate::{adjustment_layers::AdjustmentLayer, project::AdjustmentLayers};
    let mut editor = crate::editor::tests::accepted_input(
        crate::editor::tests::authored_fixture(&["patches.dng"]),
        crate::processing::InputSettings::Digital,
    );
    editor.document.project.roll = RollSettings::default();
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
            &editor.document.project.input,
            editor.document.project.roll,
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
    editor.document.project.input = InputSettings::Digital;
    editor.document.project.roll = RollSettings::default();
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
            editor.document.project.roll,
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
fn wb_target_is_locked_while_open_and_cancel_allows_a_new_layer_pick_without_old_publication() {
    use crate::{adjustment_layers::AdjustmentLayer, editor::adjustment_layers::Target};
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor = crate::editor::tests::accepted_input(editor, InputSettings::Digital);
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
            editor.begin_test_picker(PickerCase::Wb, cx);
        });
        app.pump();
        app.update(|editor| {
            assert_eq!(editor.test_pick(), Some(LAYER));
            editor.select_adjustment_layer(Target::Layer(second));
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(first));
            editor.end_tool();
            editor.select_adjustment_layer(Target::Layer(second));
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
        });
        app.pump();
        app.read(|editor| {
            assert!(editor.test_pick().is_none() && !editor.interaction_blocked());
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
        assert_ne!(expected, app.read(|editor| editor.photo_settings().wb));
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Wb, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
            assert_eq!(editor.photo_settings().wb, expected);
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
fn a_completed_wb_result_applies_to_its_captured_layer_after_selection_changes() {
    use crate::{adjustment_layers::AdjustmentLayer, editor::adjustment_layers::Target};
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor = crate::editor::tests::accepted_input(editor, InputSettings::Digital);
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
    let other = AdjustmentLayerSettings {
        exposure: 0.5,
        wb: Balance {
            warmth: 0.4,
            tint: -0.2,
        },
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
        .add("Other".into(), AdjustmentLayer::global(other))
        .unwrap();
    editor.select_adjustment_layer(Target::Layer(first));
    let target = (source, Target::Layer(first));
    let mut image = measurement::capture_scan(&editor).unwrap();
    let area = image.area;
    let region = Coverage::rectangle(Rect {
        x: area.x + 8,
        y: area.y + 8,
        width: 32,
        height: 32,
    });
    let input = LayerInput::capture(&editor).unwrap();
    let services = editor.source_services.clone().unwrap();
    let before = editor.photo_settings().wb;
    crate::application::test_support::with_application(editor, |app| {
        let reading = app
            .access()
            .spawn(move |cx| async move {
                let worker = cx.clone();
                let region = picking::pick(&cx, LAYER).await?;
                measurement::run(&cx, LAYER.label, async move {
                    measurement::query(&worker, move |cancelled| {
                        input.measure(&mut image, region, &services, &|| cancelled.is_cancelled())
                    })
                    .await
                })
                .await
            })
            .unwrap();
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.test_picker_active() && reading.is_finished()
        });
        app.update(|editor| {
            editor.select_adjustment_layer(Target::Layer(second));
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
        });
        let value = pollster::block_on(reading).unwrap();
        assert_ne!(value, before);
        app.update(|editor| set_layer(editor, value, target).unwrap());
        app.read(|editor| {
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
            assert_eq!(editor.photo_settings(), other);
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
                    .find(|layer| layer.id == first)
                    .unwrap()
                    .adjustment
                    .photo
                    .wb,
                value
            );
            assert_eq!(
                layers
                    .iter()
                    .find(|layer| layer.id == second)
                    .unwrap()
                    .adjustment
                    .photo,
                other
            );
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                Some("Calibration pick")
            );
        });
    });
}

#[test]
fn layer_selection_is_locked_during_wb_measurement_and_unlocked_after_completion() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.tab = 1;
    editor = crate::editor::tests::accepted_input(editor, InputSettings::Digital);
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
    assert_ne!(balance, editor.photo_settings().wb);
    let gate = testing::Gate::install(&mut editor);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Wb, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_test_pick_region(Some(region));
            editor.apply_pick(cx);
        });
        let call = gate.next(app);
        app.enter(|editor, cx| {
            editor.apply_adjustment_layer_action(
                crate::editor::adjustment_layers::Action::Select(
                    crate::editor::adjustment_layers::Target::Layer(second),
                ),
                cx,
            );
            assert_eq!(
                editor.adjustment_layer_target(),
                crate::editor::adjustment_layers::Target::Layer(first)
            );
            assert!(editor.measurement_pending());
            assert!(!editor.picking());
        });
        call.run();
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert!(!editor.measurement_pending());
            assert!(editor.test_pick().is_none());
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
        app.enter(|editor, cx| {
            editor.apply_adjustment_layer_action(
                crate::editor::adjustment_layers::Action::Select(
                    crate::editor::adjustment_layers::Target::Layer(second),
                ),
                cx,
            );
            assert_eq!(
                editor.adjustment_layer_target(),
                crate::editor::adjustment_layers::Target::Layer(second)
            );
        });
    });
}

#[test]
fn completed_wb_measurement_cannot_publish_after_cancellation_photo_or_document_retirement() {
    #[derive(Clone, Copy)]
    enum Replacement {
        Crop,
        Photo,
        Document,
    }
    for replacement in [Replacement::Crop, Replacement::Photo, Replacement::Document] {
        let first = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
        let second = crate::raw::fixture::bayer_dng_with_rgb([500, 300, 400]);
        let mut editor = crate::editor::tests::authored_fixture(&[
            first.path().to_str().unwrap(),
            second.path().to_str().unwrap(),
        ]);
        editor = crate::editor::tests::accepted_input(editor, InputSettings::Digital);

        let area = editor.selected_area().unwrap();
        let region = Coverage::rectangle(Rect {
            x: area.x + 8,
            y: area.y + 8,
            width: 32,
            height: 32,
        });
        let would_apply = LayerInput::capture(&editor)
            .unwrap()
            .measure(
                &mut measurement::capture_scan(&editor).unwrap(),
                region,
                editor.source_services.as_ref().unwrap(),
                &|| false,
            )
            .unwrap();
        assert_ne!(would_apply, editor.photo_settings().wb);
        let gate = testing::Gate::after_work();
        editor.document.visit.measurement_gate = Some(gate.clone());
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.start_interaction(cx, pick_layer).unwrap();
            });
            app.pump();
            app.enter(|editor, cx| {
                editor.set_test_pick_region(Some(region));
                editor.apply_pick(cx);
            });
            let worker = gate.next(app);
            app.enter(|editor, cx| match replacement {
                Replacement::Crop => editor.activities.view().unwrap().request_cancel(),
                Replacement::Photo => editor.load(second.path().to_owned(), cx),
                Replacement::Document => {
                    editor.document = crate::editor::document::Document::default().into()
                }
            });
            app.pump();
            assert!(worker.cancelled.is_cancelled());
            if matches!(replacement, Replacement::Crop) {
                app.update(Editor::begin_crop);
                app.read(|editor| assert!(editor.cropping()));
            }
            app.run_until(Duration::from_secs(10), |editor| {
                !editor.document.visit.source.pending()
            });
            let accepted = app.read(|editor| {
                (
                    editor.document.project.clone(),
                    editor
                        .history_label(crate::editor::history::Direction::Undo)
                        .map(str::to_owned),
                    editor.status.clone(),
                )
            });
            let completed = worker.completed.clone();
            worker.run();
            completed.wait(app);
            app.pump();
            app.read(|editor| {
                assert_eq!(editor.document.project, accepted.0);
                assert_eq!(
                    editor.history_label(crate::editor::history::Direction::Undo),
                    accepted.1.as_deref()
                );
                assert_eq!(editor.status, accepted.2);
                assert!(editor.test_pick().is_none());
                if matches!(replacement, Replacement::Crop) {
                    assert!(editor.cropping());
                }
            });
        });
    }
}

#[test]
fn photo_wb_pick_uses_its_captured_baseline_before_upstream_contrast() {
    use crate::exposure_reference::{ReferenceExposure, StartingExposure};
    let file = crate::raw::fixture::bayer_dng_with_rgb([350, 600, 450]);
    let mut editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    editor.document.project.input = InputSettings::Digital;
    editor.document.state.project.input = InputSettings::Digital;
    editor.document.project.roll = RollSettings::default();
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
