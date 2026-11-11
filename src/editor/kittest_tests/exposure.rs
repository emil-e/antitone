use super::*;

#[test]
fn matched_reference_reaches_selected_canvas_and_navigation_at_zero_manual_exposure() {
    assert_matched_reference_in_views(false);
}

#[test]
fn matched_reference_reaches_padded_film_views_at_zero_manual_exposure() {
    assert_matched_reference_in_views(true);
}

fn assert_matched_reference_in_views(film_enabled: bool) {
    let file = crate::exposure_fixture::neutral_dng(1.3);
    editor_harness_with_thumbnails(true, |mut h, capture| {
        h.update(|editor| {
            editor.document.project.input = crate::processing::InputSettings::Digital;
            editor.document.set_target(OutputTarget::Srgb);
            // Zero grain amplitude keeps the constant-patch oracle; MTF still needs padding.
            editor.document.project.output.grain.enabled = film_enabled;
            editor.document.project.output.grain.amount = 0.;
        });
        h.enter(|editor, cx| editor.add_raws(vec![file.path().to_owned()], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked()
                && editor.document.visit.working_view.support_ready()
                && editor
                    .document
                    .thumbnails
                    .ready_count(&editor.document, editor.document.target())
                    == 1
        });
        h.event(egui::Event::PointerGone);
        h.step();
        h.render().unwrap();
        let image = h.read(|editor| {
            assert!(!editor.document.source);
            let support = crate::pixel_pipeline::PixelPipeline::film(
                editor.document.project.output.grain,
                editor
                    .source_composition(
                        editor
                            .document
                            .project
                            .source(editor.document.visit.current_source.unwrap())
                            .unwrap(),
                    )
                    .crop()
                    .size,
                0,
            )
            .unwrap()
            .support([1., 1.])
            .unwrap();
            assert_eq!(
                support != crate::processing_support::Support::NONE,
                film_enabled
            );
            assert_eq!(editor.document.project.roll.exposure, 0.);
            assert_eq!(editor.photo_settings().exposure, 0.);
            let reference = editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .reference
                .as_ref()
                .unwrap();
            assert!(matches!(
                reference,
                crate::exposure_reference::ReferenceExposure::Matched(_)
            ));
            assert!((reference.ev() - 1.3).abs() < 0.03, "{reference:?}");
            editor
                .document
                .visit
                .canvas
                .last_layout
                .unwrap()
                .0
                .image
                .center()
        });
        let card = h
            .get_by_label(file.path().file_name().unwrap().to_str().unwrap())
            .rect()
            .center();
        let linear = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)
            .unwrap()
            .render(&[DVec3::splat(6000. / 65535. * 1.3_f64.exp2())])
            .unwrap()[0];
        let expected = linear
            .to_array()
            .map(|v| OutputTarget::Srgb.encode(f64::from(v)).unwrap());
        let captured = capture.borrow();
        for (name, point) in [("selected canvas", image), ("navigation positive", card)] {
            let actual = captured.pixels
                [point.y.floor() as usize * captured.size[0] as usize + point.x.floor() as usize];
            for c in 0..3 {
                assert!(
                    (f64::from(actual[c]) - expected[c]).abs() < 0.004,
                    "{name}: {actual:?} vs independent reference {expected:?}"
                );
            }
        }
        drop(captured);

        h.get_by_label("Original-resolution reference").click();
        h.step();
        wait_for(&mut h, |editor| {
            assert!(
                !editor.document.visit.canvas.original.failed(),
                "{}",
                editor.status
            );
            editor.document.visit.working_view.reference_ready()
        });
        h.render().unwrap();
        let point = h.read(|editor| {
            editor
                .document
                .visit
                .canvas
                .last_layout
                .unwrap()
                .0
                .image
                .center()
        });
        {
            let pixels = capture.borrow();
            let actual = pixels.pixels
                [point.y.floor() as usize * pixels.size[0] as usize + point.x.floor() as usize];
            for c in 0..3 {
                assert!(
                    (f64::from(actual[c]) - expected[c]).abs() < 0.004,
                    "original reference {actual:?} vs {expected:?}"
                );
            }
        }
        h.get_by_label("Original-resolution reference").click();
        h.step();
        h.enter(|editor, cx| editor.begin_test_picker(PickerCase::Wb, cx));
        h.step();
        wait_for(&mut h, |editor| {
            assert!(
                !editor.document.visit.canvas.original.failed(),
                "{}",
                editor.status
            );
            editor.document.visit.canvas.original.settled().is_some()
        });
        h.event(egui::Event::PointerGone);
        h.step();
        h.render().unwrap();
        let point = h.read(|editor| {
            assert_eq!(editor.test_pick(), Some(PickerCase::Wb.display()));
            editor
                .document
                .visit
                .canvas
                .last_layout
                .unwrap()
                .0
                .image
                .center()
        });
        let pixels = capture.borrow();
        let actual = pixels.pixels
            [point.y.floor() as usize * pixels.size[0] as usize + point.x.floor() as usize];
        for c in 0..3 {
            assert!(
                (f64::from(actual[c]) - expected[c]).abs() < 0.004,
                "WB precision view {actual:?} vs {expected:?}"
            );
        }
    });
}

#[test]
fn automatic_digital_import_prepares_reused_negative_pixels_before_ready() {
    let first = crate::exposure_fixture::neutral_dng(1.3);
    let second = fixture("patches.dng");
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.add_raws(vec![first.path().to_owned()], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.working_view.support_ready()
        });
        let (selected, native, development) = h.read(|editor| {
            assert!(editor.document.project.input.is_negative());
            let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
            assert!(ready.reference.as_ref().is_none());
            (
                editor.document.visit.current_source,
                ready.developed.clone(),
                ready.record.development.clone(),
            )
        });
        h.enter(|editor, cx| editor.add_raws(vec![second], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.working_view.support_ready()
        });
        h.render().unwrap();
        h.read(|editor| {
            assert_eq!(
                editor.document.project.input,
                crate::processing::InputSettings::Digital
            );
            assert_eq!(editor.document.visit.current_source, selected);
            let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
            assert_eq!(
                ready.record.development, development,
                "the original unity/as-shot basis is unchanged"
            );
            assert!(
                Arc::ptr_eq(&native, &ready.developed),
                "the selected pixels must not be redeveloped"
            );
            let reference = ready
                .reference
                .as_ref()
                .expect("Digital readiness must include preparation");
            assert!((reference.ev() - 1.3).abs() < 0.03, "{reference:?}");
            assert!(!editor.document.source);
            assert_eq!(editor.document.project.roll.exposure, 0.);
            assert_eq!(editor.photo_settings().exposure, 0.);
        });
    });
}

#[test]
fn opening_same_source_with_different_normalization_replaces_navigation_pixels_and_regrades() {
    use crate::{
        exposure_reference::{ReferenceExposure, UnavailableReference as Unavailable},
        project::{Project, Save},
    };
    let file = crate::exposure_fixture::neutral_dng(1.3);
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("matched.antitone");
    let second = directory.path().join("unavailable.antitone");
    let mut document = Document::default();
    document.project.input = crate::processing::InputSettings::Digital;
    document.import(&[file.path().to_owned()]).unwrap();
    let id = document.project.order[0];
    let native = Scan::open(file.path(), document.project.acquisition_wb()).unwrap();
    let document: crate::editor::document_scope::DocumentScope = document.into();
    crate::photo_preparation::native(
        None,
        &document.exposure_slot(&document.initialization, id),
        document.project.input,
        native.source(),
    )
    .unwrap();
    document.capture().save(&first, Save::New).unwrap();
    let mut saved = Project::load(&first).unwrap();
    let entry = &mut saved.exposure_metadata.as_mut().unwrap().entries[0];
    assert!((entry.reference.ev() - 1.3).abs() < 0.03);
    entry.reference = ReferenceExposure::Unavailable(Unavailable::NoUsablePreview);
    saved.save(&second, Save::New).unwrap();

    editor_harness_with_thumbnails(true, |mut h, capture| {
        for (stage, expected_ev) in [(0, 1.3_f64), (1, 0.), (2, 0.6)] {
            if stage < 2 {
                open_project(
                    &mut h,
                    if stage == 0 {
                        first.clone()
                    } else {
                        second.clone()
                    },
                );
            } else {
                h.update(|editor| {
                    editor
                        .edit_adjustment_layers(|layers| {
                            layers.layers[0].adjustment.photo.exposure = 0.6;
                            Ok(())
                        })
                        .unwrap()
                });
            }
            wait_for(&mut h, |editor| {
                !editor.interaction_blocked()
                    && editor.document.visit.working_view.support_ready()
                    && editor
                        .document
                        .thumbnails
                        .ready_count(&editor.document, editor.document.target())
                        == 1
            });
            h.event(egui::Event::PointerGone);
            h.step();
            h.render().unwrap();
            let center = h.read(|editor| {
                if stage > 0 {
                    assert_eq!(
                        editor
                            .document
                            .visit
                            .canvas
                            .ready
                            .as_ref()
                            .unwrap()
                            .reference
                            .as_ref(),
                        Some(&ReferenceExposure::Unavailable(
                            Unavailable::NoUsablePreview
                        ))
                    );
                }
                editor
                    .document
                    .visit
                    .canvas
                    .last_layout
                    .unwrap()
                    .0
                    .image
                    .center()
            });
            let card = h
                .get_by_label(file.path().file_name().unwrap().to_str().unwrap())
                .rect()
                .center();
            let expected = crate::rendering::CpuRenderer::new(OutputTarget::Srgb)
                .unwrap()
                .render(&[DVec3::splat(6000. / 65535. * expected_ev.exp2())])
                .unwrap()[0]
                .to_array()
                .map(|v| OutputTarget::Srgb.encode(f64::from(v)).unwrap());
            let pixels = capture.borrow();
            for (name, point) in [("canvas", center), ("navigation", card)] {
                let actual = pixels.pixels
                    [point.y.floor() as usize * pixels.size[0] as usize + point.x.floor() as usize];
                for c in 0..3 {
                    assert!(
                        (f64::from(actual[c]) - expected[c]).abs() < 0.004,
                        "stage {stage} {name}: {actual:?} vs independent reference {expected:?}"
                    );
                }
            }
        }
    });
}

#[test]
fn cold_open_accepts_metadata_but_unsafe_normalized_layers_cannot_produce_a_positive() {
    use crate::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings},
        project::{AdjustmentLayers, Save},
    };
    let file = crate::exposure_fixture::neutral_dng(1.3);
    let other = crate::exposure_fixture::neutral_dng(0.);
    let other_name = "supported.dng";
    let directory = tempfile::tempdir().unwrap();
    let first = directory.path().join("unsupported.dng");
    let second = directory.path().join(other_name);
    std::fs::copy(file.path(), &first).unwrap();
    std::fs::copy(other.path(), &second).unwrap();
    let path = directory.path().join("unprepared.antitone");
    let mut document = Document::default();
    document.project.input = crate::processing::InputSettings::Digital;
    document.import(&[first, second]).unwrap();
    let id = document.project.order[0];
    let other_id = document.project.order[1];
    document.project.roll.exposure = 16.;
    let mut layers = AdjustmentLayers::default();
    for exposure in [16., 16., 16., 16., 16., 16., 5.5] {
        layers
            .add(
                "Range control".into(),
                AdjustmentLayer::global(AdjustmentLayerSettings {
                    exposure,
                    ..Default::default()
                }),
            )
            .unwrap();
    }
    document.project.source_mut(id).unwrap().adjustment_layers = Some(layers);
    super::super::processing::validate_source(
        None,
        document.project.input,
        document.project.roll,
        document.project.source(id).unwrap(),
    )
    .expect("zero-start range is supported");
    document.save(&path, Save::New).unwrap();
    assert!(
        crate::project::Project::load(&path)
            .unwrap()
            .exposure_metadata
            .is_none()
    );
    let accepted =
        Document::open(&path).expect("metadata Open defers this unresolved numerical domain");
    editor_harness(|mut h, _| {
        open_project(&mut h, path.clone());
        wait_opened_photo(&mut h);
        h.step();
        assert!(h.output().shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("Positive preview unavailable:")
        )), "the production view must report rejected rendering before nonpublication is checked");
        h.read(|editor| {
            assert_eq!(editor.document.state.project, accepted.project);
            assert_eq!(editor.document.visit.current_source, Some(id));
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "{}",
                editor.status
            );
            assert!(
                editor
                    .validate_document_candidate(&editor.document)
                    .is_err(),
                "resolved normalization must expose the unsupported numerical domain"
            );
            assert!(
                editor.canvas_frame.displayed.is_none(),
                "an unsafe positive must not be published"
            );
        });
        h.get_by_label(other_name).scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_label(other_name).click();
        h.step();
        wait_opened_photo(&mut h);
        h.read(|editor| {
            assert_eq!(
                editor.document.visit.current_source,
                Some(other_id),
                "one unsupported positive must not veto navigation to another photo"
            );
            assert_eq!(editor.document.state.project, accepted.project);
            assert!(!editor.document.dirty());
        });
        super::workflow::wait_settled(&mut h);
        h.get_by_label("unsupported.dng").click();
        h.step();
        wait_opened_photo(&mut h);
        h.update(|editor| editor.document.project.roll.exposure = 0.);
        h.step();
        super::workflow::wait_settled(&mut h);
        h.read(|editor| {
            editor
                .validate_document_candidate(&editor.document)
                .unwrap();
            assert!(
                !editor
                    .canvas_frame
                    .displayed
                    .as_ref()
                    .unwrap()
                    .view
                    .spec
                    .is_inspection(),
                "the same resolved source can display a supported positive"
            );
        });
    });
}
