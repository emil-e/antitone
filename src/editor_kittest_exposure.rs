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
            editor.input = crate::processing::InputSettings::Digital;
            editor.target = OutputTarget::Srgb;
            // Zero grain amplitude keeps the constant-patch oracle; MTF still needs padding.
            editor.film_emulation.enabled = film_enabled;
            editor.film_emulation.amount = 0.;
            editor.sync_document();
        });
        h.enter(|editor, cx| editor.add_raws(vec![file.path().to_owned()], cx));
        wait_for(&mut h, |editor| {
            !editor.busy
                && editor.document_pending.is_none()
                && editor.working.support_ready()
                && editor
                    .thumbnails
                    .ready_count(&editor.document, editor.target)
                    == 1
        });
        h.event(egui::Event::PointerGone);
        h.step();
        h.render().unwrap();
        let image = h.read(|editor| {
            assert!(!editor.source);
            let ready = editor.document.visit.ready.as_ref().unwrap();
            let support = crate::pixel_pipeline::PixelPipeline::film(
                editor.film_emulation,
                ready.developed.area().height,
                0,
            )
            .unwrap()
            .support([1., 1.])
            .unwrap();
            assert_eq!(
                support != crate::processing_support::Support::NONE,
                film_enabled
            );
            assert_eq!(editor.roll.exposure, 0.);
            assert_eq!(editor.adjustment_layer_settings.exposure, 0.);
            let reference = editor
                .document
                .visit
                .ready
                .as_ref()
                .unwrap()
                .developed
                .reference_exposure
                .get()
                .unwrap();
            assert!(matches!(
                reference,
                crate::exposure_reference::ReferenceExposure::Matched(_)
            ));
            assert!((reference.ev() - 1.3).abs() < 0.03, "{reference:?}");
            editor.last_layout.unwrap().0.image.center()
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
                editor.original.refinement_error.is_none(),
                "{}",
                editor.status
            );
            editor.working.reference_ready()
        });
        h.render().unwrap();
        let point = h.read(|editor| editor.last_layout.unwrap().0.image.center());
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
        h.enter(|editor, cx| editor.begin_pick(Pick::Wb, cx));
        h.step();
        wait_for(&mut h, |editor| {
            assert!(
                editor.original.refinement_error.is_none(),
                "{}",
                editor.status
            );
            editor.original.settled.is_some()
        });
        h.event(egui::Event::PointerGone);
        h.step();
        h.render().unwrap();
        let point = h.read(|editor| {
            assert_eq!(editor.pick, Some(Pick::Wb));
            editor.last_layout.unwrap().0.image.center()
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
            !editor.busy && editor.document_pending.is_none() && editor.working.support_ready()
        });
        let (selected, native, development) = h.read(|editor| {
            assert!(editor.input.is_negative());
            let ready = editor.document.visit.ready.as_ref().unwrap();
            assert!(ready.developed.reference_exposure.get().is_none());
            (
                editor.current_source,
                ready.developed.clone(),
                ready.record.development.clone(),
            )
        });
        h.enter(|editor, cx| editor.add_raws(vec![second], cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document_pending.is_none() && editor.working.support_ready()
        });
        h.render().unwrap();
        h.read(|editor| {
            assert_eq!(editor.input, crate::processing::InputSettings::Digital);
            assert_eq!(editor.current_source, selected);
            let ready = editor.document.visit.ready.as_ref().unwrap();
            assert_eq!(
                ready.record.development, development,
                "the original unity/as-shot basis is unchanged"
            );
            assert!(
                Arc::ptr_eq(&native, &ready.developed),
                "the selected pixels must not be redeveloped"
            );
            let reference = ready
                .developed
                .reference_exposure
                .get()
                .expect("Digital readiness must include preparation");
            assert!((reference.ev() - 1.3).abs() < 0.03, "{reference:?}");
            assert!(!editor.source);
            assert_eq!(editor.roll.exposure, 0.);
            assert_eq!(editor.adjustment_layer_settings.exposure, 0.);
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
    let native = Scan::open(
        file.path(),
        Some(
            document
                .project
                .source(id)
                .unwrap()
                .development
                .decode
                .fixed_wb,
        ),
    )
    .unwrap();
    document
        .cache()
        .prepare_source(document.project.input, native.source())
        .unwrap();
    document.save(&first, Save::New).unwrap();
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
                !editor.busy
                    && editor.document_pending.is_none()
                    && editor.working.support_ready()
                    && editor
                        .thumbnails
                        .ready_count(&editor.document, editor.target)
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
                            .ready
                            .as_ref()
                            .unwrap()
                            .developed
                            .reference_exposure
                            .get(),
                        Some(&ReferenceExposure::Unavailable(
                            Unavailable::NoUsablePreview
                        ))
                    );
                }
                editor.last_layout.unwrap().0.image.center()
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
fn cold_open_admits_selected_layers_only_after_source_normalization() {
    use crate::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings},
        project::{AdjustmentLayers, Save},
    };
    let file = crate::exposure_fixture::neutral_dng(1.3);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("unprepared.antitone");
    let mut document = Document::default();
    document.project.input = crate::processing::InputSettings::Digital;
    document.import(&[file.path().to_owned()]).unwrap();
    let id = document.project.order[0];
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
    Document::open(&path).expect("header-only Open defers this unresolved numerical domain");
    editor_harness(|mut h, _| {
        open_project(&mut h, path.clone());
        h.read(|editor| {
            assert!(
                editor.document.project.order.is_empty(),
                "the unsafe normalized selected grade must not replace the accepted document"
            );
            assert!(editor.current_source.is_none());
        });
    });
}
