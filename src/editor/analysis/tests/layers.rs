use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    project::AdjustmentLayers,
};

fn local_records() -> AdjustmentLayers {
    let mut layers = AdjustmentLayers::default();
    for (name, enabled, exposure) in [
        ("Active", true, 0.5),
        ("Disabled", false, 1.),
        ("Identity", true, 0.),
    ] {
        layers
            .add(
                name.into(),
                AdjustmentLayer {
                    mask: AdjustmentLayerCoverage::Linear {
                        start: SourcePoint([20., 20.]),
                        end: SourcePoint([500., 300.]),
                    },
                    photo: AdjustmentLayerSettings {
                        exposure,
                        ..Default::default()
                    },
                    enabled,
                    invert: false,
                    strength: 1.,
                },
            )
            .unwrap();
    }
    layers.next_id = 50;
    layers
}

fn proposal(document: &Document) -> roll_analysis::Proposal {
    roll_analysis::Proposal {
        input: crate::processing::InputSettings::Negative {
            dmin: Some(DVec3::new(0.8, 0.4, 0.2)),
            dmax: Some(DVec3::new(0.04, 0.02, 0.01)),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
            response: crate::processing::Response::film(),
            gamma: crate::processing::DEFAULT_GAMMA,
            balance: Default::default(),
        },
        roll: document.project.roll,
        photos: vec![
            AdjustmentLayerSettings {
                exposure: 0.4,
                ..Default::default()
            };
            document.project.order.len()
        ],
        frames: Vec::new(),
    }
}

#[test]
fn photo_initialization_replaces_only_analyzed_layers_and_retains_id_history() {
    let mut editor = state();
    let first = editor.current_source.unwrap();
    editor
        .document
        .project
        .source_mut(first)
        .unwrap()
        .adjustment_layers = Some(local_records());
    for (name, ordered) in [
        ("offscreen.ARW", true),
        ("retired.ARW", false),
        ("reference.ARW", false),
    ] {
        let mut source = editor.document.project.source(first).unwrap().clone();
        source.id = SourceId(0);
        source.path = name.into();
        source.reference_only = name == "reference.ARW";
        let id = editor.document.project.add_source(source).unwrap();
        if !ordered {
            editor.document.project.order.retain(|source| *source != id);
        }
    }
    let before = editor.document.project.clone();
    let picture = rect([52, 42, 720, 540]);
    let candidate =
        proposed_document(&editor.document, proposal(&editor.document), Some(picture)).unwrap();
    assert_eq!(editor.document.project, before);
    for prior in &before.sources {
        let mut source = candidate.project.source(prior.id).unwrap().clone();
        if before.order.contains(&prior.id) {
            assert_eq!(
                source.adjustment_layers.as_ref().unwrap().layers[0]
                    .adjustment
                    .photo,
                AdjustmentLayerSettings {
                    exposure: 0.4,
                    ..Default::default()
                }
            );
            let layers = source.adjustment_layers.as_ref().unwrap();
            assert_eq!(layers.layers.len(), 1);
            assert_eq!(layers.layers[0].id.0, 50);
            assert_eq!(
                layers.layers[0].adjustment.mask,
                AdjustmentLayerCoverage::Global {}
            );
            assert_eq!(layers.next_id, 51);
            let mut next = layers.clone();
            let id = next
                .add(
                    "New after Analyze".into(),
                    local_records().layers[0].adjustment,
                )
                .unwrap();
            assert_eq!(id.0, 51);
            source.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo = prior.adjustment_layers.as_ref().unwrap().layers[0]
                .adjustment
                .photo;
            source.adjustment_layers = prior.adjustment_layers.clone();
        }
        assert_eq!(&source, prior);
    }
    assert_eq!(candidate.project.order, before.order);
    assert_eq!(candidate.project.analysis_area, Some(array(picture)));
    candidate.validate().unwrap();
    let mut malformed = proposal(&editor.document);
    malformed.photos.pop();
    assert!(proposed_document(&editor.document, malformed, Some(picture)).is_err());
    assert_eq!(editor.document.project, before);
}

#[test]
fn any_local_record_adds_an_explicit_reset_warning_to_the_area_review() {
    for index in 0..3 {
        let editor = ready_state();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                let source = editor
                    .document
                    .project
                    .source_mut(editor.current_source.unwrap())
                    .unwrap();
                let mut layers = local_records();
                layers.layers = vec![layers.layers[index].clone()];
                source.adjustment_layers = Some(layers);
                editor.sync_document();
                let before = editor.document.project.clone();
                review_tests::start_area_preparation(editor, cx);
                review_tests::prepared(editor);
                assert!(
                    editor
                        .document
                        .analysis_review
                        .as_ref()
                        .unwrap()
                        .replaces_photos,
                    "record {index}"
                );
                let ctx = egui::Context::default();
                super::super::super::tests::app_frame(editor, &ctx, vec![]);
                let labels = super::super::super::tests::app_frame(editor, &ctx, vec![]);
                assert!(labels.iter().any(|(label, _)| label == "Tool warning"));
                editor.end_analysis_review(cx);
                assert_eq!(editor.document.project, before);
                assert!(editor.document.analysis_pending.is_none());
            });
        });
    }
}

#[test]
fn retired_local_records_and_empty_allocator_history_do_not_trigger_reset_warning() {
    let editor = ready_state();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let mut retired = editor.document.project.sources[0].clone();
            retired.id = SourceId(0);
            retired.path = "retired.ARW".into();
            retired.adjustment_layers = Some(local_records());
            let id = editor.document.project.add_source(retired).unwrap();
            editor.document.project.order.retain(|source| *source != id);
            let mut empty = local_records();
            empty.layers.clear();
            editor.document.project.sources[0].adjustment_layers = Some(empty);
            editor.sync_document();
            let before = editor.document.project.clone();
            review_tests::start_area_preparation(editor, cx);
            review_tests::prepared(editor);
            assert!(
                !editor
                    .document
                    .analysis_review
                    .as_ref()
                    .unwrap()
                    .replaces_photos
            );
            editor.end_analysis_review(cx);
            assert_eq!(editor.document.project, before);
        });
    });
}

#[test]
fn local_removal_is_installed_only_with_a_successful_current_analysis_candidate() {
    for mode in 0..4 {
        let mut editor = state();
        editor
            .document
            .project
            .source_mut(editor.current_source.unwrap())
            .unwrap()
            .adjustment_layers = Some(local_records());
        editor.refresh_adjustment_layer_selection();
        let settings_before = editor.adjustment_layer_settings;
        let before = editor.document.project.clone();
        let id = pending(&mut editor);
        let mut candidate = proposed_document(
            &editor.document,
            proposal(&editor.document),
            Some(rect([52, 42, 720, 540])),
        )
        .unwrap();
        candidate.validate().unwrap();
        if mode == 1 {
            editor.cancel_analysis();
        }
        if mode == 2 {
            candidate.project.set_dmax(candidate.project.dmin());
        }
        let result = if mode == 0 {
            Err("sampling failed".into())
        } else {
            Ok(Some(Box::new(Outcome {
                document: candidate,
                frames: 1,
                support: rect([12, 12, 800, 600]),
            })))
        };
        editor.analysis_result(id, result, &crate::application::UiContext::render_only());
        if mode == 3 {
            let source = editor
                .document
                .project
                .source(editor.current_source.unwrap())
                .unwrap();
            let layers = source.adjustment_layers.as_ref().unwrap();
            assert_eq!(layers.layers.len(), 1);
            assert_eq!(layers.layers[0].id.0, 50);
            assert_eq!(
                layers.layers[0].adjustment.mask,
                AdjustmentLayerCoverage::Global {}
            );
            assert_eq!(layers.next_id, 51);
            assert_eq!(
                editor.adjustment_layer_target(),
                adjustment_layers::Target::Layer(crate::project::AdjustmentLayerId(50))
            );
        } else {
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.adjustment_layer_settings, settings_before);
        }
    }
}
