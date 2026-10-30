use super::*;
use crate::adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage};

#[test]
fn film_seed_routing_uses_source_identity_not_visit_identity() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let source = editor.document.visit.current_source.unwrap();
        let first = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        assert_eq!(first.film_seed(), Some(source.0));
        editor.id = editor.id.wrapping_add(123);
        let reacquired = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        assert_eq!(first.film_seed(), reacquired.film_seed());
        let mut other = editor
            .document
            .state
            .project
            .source(source)
            .unwrap()
            .clone();
        other.id = SourceId(0);
        other.path = "another-film-seed.ARW".into();
        let other = editor.document.state.project.add_source(other).unwrap();
        editor.document.visit.current_source = Some(other);
        let changed = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        assert_eq!(changed.film_seed(), Some(other.0));
        assert_ne!(first.film_seed(), changed.film_seed());
    });
}

fn ids(editor: &Editor) -> Vec<AdjustmentLayerId> {
    editor
        .document
        .state
        .project
        .source(editor.document.visit.current_source.unwrap())
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers
        .iter()
        .map(|record| record.id)
        .collect()
}

#[test]
fn list_actions_keep_identity_and_reject_candidates_without_mutating_accepted_resources() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = editor.document.visit.current_source.unwrap();
        let order = ids(&editor);
        editor.reveal_adjustment_layers();
        editor.apply_adjustment_layer_action(Action::Select(Target::Layer(order[0])));
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        let accepted = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        let revision = editor.document.visit.canvas.grade.original.revision();
        editor.apply_adjustment_layer_action(Action::Rename(order[0], "Sky".into()));
        assert_eq!(
            editor.document.visit.canvas.grade.original.revision(),
            revision,
            "metadata must not invalidate processing"
        );
        let renamed = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        assert!(std::ptr::eq(
            accepted.grade().unwrap(),
            renamed.grade().unwrap()
        ));
        editor.apply_adjustment_layer_action(Action::Reorder(
            editor.order_drag(order[0]).unwrap(),
            2,
        ));
        assert_eq!(ids(&editor), vec![order[1], order[0]]);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        assert_ne!(
            editor
                .positive_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings
                )
                .unwrap()
                .signature(),
            accepted.signature()
        );
        editor.apply_adjustment_layer_action(Action::Enable(order[0], false));
        let source = editor.document.state.project.source(id).unwrap();
        assert!(
            !source.adjustment_layers.as_ref().unwrap().layers[1]
                .adjustment
                .enabled
        );
        let before = editor.document.state.project.clone();
        let revision = editor.document.visit.canvas.grade.original.revision();
        assert!(
            editor
                .edit_adjustment_layers(|layers| {
                    record_mut(layers, order[0])?.adjustment.photo.contrast = f64::NAN;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(editor.document.state.project, before);
        assert_eq!(
            editor.document.visit.canvas.grade.original.revision(),
            revision
        );
        editor.apply_adjustment_layer_action(Action::Rename(order[0], "  ".into()));
        assert_eq!(editor.document.state.project, before);
        editor.apply_adjustment_layer_action(Action::Reorder(
            editor.order_drag(order[0]).unwrap(),
            3,
        ));
        assert_eq!(editor.document.state.project, before);
        editor.busy = true;
        editor.apply_adjustment_layer_action(Action::Select(Target::None));
        editor.apply_adjustment_layer_action(Action::Enable(order[0], true));
        assert_eq!(editor.document.state.project, before);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        editor.document.visit.current_source = None;
        assert_eq!(editor.adjustment_layer_target(), Target::None);
    });
}

#[test]
fn deferred_drag_captures_locals_after_an_outgoing_rename_commits() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.reveal_adjustment_layers();
        let order = ids(&editor);
        let key = OrderKey {
            loaded: editor.id,
            source: editor.document.visit.current_source.unwrap(),
            id: order[0],
        };
        editor.begin_control_history("Photo settings");
        let cx = UiContext::render_only();
        editor.control_command(
            editing::Command::Adjustment(Action::Rename(order[0], "Renamed".into())),
            &cx,
        );
        editor.control_command(
            editing::Command::OrderInput {
                start: Some(order[0]),
                drop: Some((key, 2)),
                finish: true,
            },
            &cx,
        );
        editor.history_controls = None;
        for command in std::mem::take(&mut editor.control_commands) {
            editor.apply_control_command(command, &cx);
        }
        assert_eq!(ids(&editor), vec![order[1], order[0]]);
        let layers = editor
            .document
            .state
            .project
            .source(key.source)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap();
        assert_eq!(layers.layers[1].name, "Renamed");
        assert!(!editor.document.visit.adjustment_layers.dragging());
    });
}

#[test]
fn drop_positions_insert_without_swapping_neighbours_or_consuming_ids() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor
            .edit_adjustment_layers(|layers| {
                let mut adjustment = layers.layers[0].adjustment;
                adjustment.photo = AdjustmentLayerSettings::default();
                layers.add("Identity".into(), adjustment)?;
                Ok(())
            })
            .unwrap();
        let order = ids(&editor);
        editor.select_adjustment_layer(Target::Layer(order[0]));
        let input = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        let snapshot = input.grade().unwrap().snapshot().clone();
        let next_id = editor
            .document
            .state
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .next_id;
        editor
            .reorder_adjustment(editor.order_drag(order[0]).unwrap(), 3)
            .unwrap();
        assert_eq!(ids(&editor), vec![order[1], order[2], order[0]]);
        editor
            .reorder_adjustment(editor.order_drag(order[0]).unwrap(), 0)
            .unwrap();
        assert_eq!(ids(&editor), order);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        assert_eq!(
            editor
                .document
                .state
                .project
                .source(editor.document.visit.current_source.unwrap())
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .next_id,
            next_id
        );
        assert_eq!(input.grade().unwrap().snapshot(), &snapshot);
    });
}

#[test]
fn stale_drops_reject_changed_records_and_source_generations_atomically() {
    pollster::block_on(async {
        for changed_records in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            let id = ids(&editor)[0];
            editor.select_adjustment_layer(Target::Layer(id));
            let drag = editor.order_drag(id).unwrap();
            if changed_records {
                editor
                    .edit_adjustment_layers(|layers| {
                        layers.layers[0].name = "Renamed during drag".into();
                        Ok(())
                    })
                    .unwrap();
            } else {
                editor.id += 1;
            }
            let before = editor.document.state.project.clone();
            let revision = editor.document.visit.canvas.grade.original.revision();
            let input = editor
                .positive_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings,
                )
                .unwrap();
            assert!(editor.reorder_adjustment(drag, 2).is_err());
            assert_eq!(editor.document.state.project, before);
            assert_eq!(
                editor.document.visit.canvas.grade.original.revision(),
                revision
            );
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(id));
            let current = editor
                .positive_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings,
                )
                .unwrap();
            assert!(std::ptr::eq(
                input.grade().unwrap(),
                current.grade().unwrap()
            ));
        }
    });
}

#[test]
fn finishing_canvas_interaction_preserves_the_selected_layer_and_accepted_grade() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;

        let order = ids(&editor);
        editor.select_adjustment_layer(Target::Layer(order[1]));
        let before = editor.document.state.project.clone();
        let processing = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        editor.finish_adjustment_layer_interaction();
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[1]));
        assert_eq!(editor.document.state.project, before);
        assert_eq!(
            processing.signature(),
            editor
                .positive_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings
                )
                .unwrap()
                .signature()
        );
        editor
            .edit_adjustment_layers(|layers| layers.remove(order[1]))
            .unwrap();
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        assert_eq!(
            editor.document.visit.adjustment_layer_settings,
            editor.selected_adjustment_layer_settings().unwrap()
        );
        editor
            .edit_adjustment_layers(|layers| layers.remove(order[0]))
            .unwrap();
        assert_eq!(editor.adjustment_layer_target(), Target::None);
        assert!(editor.selected_adjustment_layer_settings().is_none());
    });
}

#[test]
fn failed_document_candidate_preserves_the_visit_and_its_local_tool_context() {
    pollster::block_on(async {
        let (editor, _, _) = processing::tests::fixture().await;
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                let id = ids(editor)[1];
                editor.select_adjustment_layer(Target::Layer(id));
                editor.document.visit.adjustment_layers.show_coverage = true;
                editor.sync_document();
                let accepted = editor.document.state.project.clone();
                let visit = editor.document.visit.view;
                editor.start_document(Preparation::Open("unavailable.antitone".into()), cx);
                let request = editor.document_pending.unwrap();
                editor.document_prepared(request, Err("candidate rejected".into()));
                assert_eq!(editor.document.visit.view, visit);
                assert_eq!(editor.adjustment_layer_target(), Target::Layer(id));
                assert!((editor.tab == 1));
                assert!(editor.document.visit.adjustment_layers.show_coverage);
                assert_eq!(editor.document.state.project, accepted);
            });
        });
    });
}

#[test]
fn deferred_layer_actions_cannot_mutate_the_stack_after_leaving_the_photo_stage() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = ids(&editor)[0];
        editor.select_adjustment_layer(Target::Layer(id));
        let drag = editor.order_drag(id).unwrap();
        editor.tab = 0;
        editor.finish_adjustment_layer_interaction();
        let before = editor.document.state.project.clone();
        editor.apply_adjustment_layer_action(Action::Enable(id, false));
        editor.apply_adjustment_layer_action(Action::Reorder(drag, 2));
        editor.apply_adjustment_layer_action(Action::Delete(id));
        editor.apply_adjustment_layer_action(Action::Select(Target::Layer(id)));
        assert!(!editor.document.visit.adjustment_layers.confirming());
        assert!(!(editor.tab == 1));
        assert_eq!(editor.document.state.project, before);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(id));
    });
}

#[test]
fn cancelling_layer_interaction_during_a_roll_pick_preserves_its_selected_target() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = ids(&editor)[0];
        editor.select_adjustment_layer(Target::Layer(id));
        editor.preview_picker(Pick::Dmin);
        assert_eq!(editor.active_pick(), Some(Pick::Dmin.display()));
        let before = editor.document.state.project.clone();
        editor.finish_adjustment_layer_interaction();
        editor.finish_pick(&crate::application::UiContext::render_only());
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(id));
        assert_eq!(editor.tab, 1);
        assert_eq!(editor.document.state.project, before);
    });
}

#[test]
fn source_calibration_picking_does_not_lock_unrelated_layer_selection() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor
        .edit_adjustment_layers(|layers| {
            layers.add("Other".into(), AdjustmentLayer::global(Default::default()))?;
            Ok(())
        })
        .unwrap();
    let layers = ids(&editor);
    let first = layers[0];
    let second = *layers.last().unwrap();
    assert_ne!(first, second);
    editor.select_adjustment_layer(Target::Layer(first));
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_pick(Pick::Dmin, cx));
        app.pump();
        app.update(|editor| {
            assert_eq!(editor.active_pick(), Some(Pick::Dmin.display()));
            editor.select_adjustment_layer(Target::Layer(second));
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
        });
        app.enter(|editor, cx| editor.finish_pick(cx));
    });
}

#[test]
fn unsupported_local_edit_preserves_grade_view_revision_and_allocator() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let accepted = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        let before = editor.document.state.project.clone();
        let revision = editor.document.visit.canvas.grade.original.revision();
        let viewport = editor.document.visit.canvas.viewport;
        let result = editor.edit_adjustment_layers(|layers| {
            let mut adjustment = layers.layers[0].adjustment;
            adjustment.photo.exposure = 16.;
            adjustment.photo.contrast = 1.6;
            adjustment.strength = 1.;
            for _ in 0..16 {
                layers.add("Unsupported".into(), adjustment)?;
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(editor.document.state.project, before);
        assert_eq!(
            editor.document.visit.canvas.grade.original.revision(),
            revision
        );
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
        let current = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        assert!(std::ptr::eq(
            accepted.grade().unwrap(),
            current.grade().unwrap()
        ));
    });
}

#[test]
fn unsupported_reorder_and_reenable_preserve_selection_allocator_grade_and_view() {
    pollster::block_on(async {
        for reorder in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            editor.document.roll = RollSettings::default();
            editor.document.visit.adjustment_layer_settings = AdjustmentLayerSettings::default();
            assert_eq!(editor.gamma(), 0.6);
            let source = editor
                .document
                .state
                .project
                .source_mut(editor.document.visit.current_source.unwrap())
                .unwrap();
            let base = source.adjustment_layers.as_ref().unwrap().layers[0].adjustment;
            let photos = if reorder {
                let mut photos = vec![
                    AdjustmentLayerSettings {
                        contrast: 1.4,
                        ..Default::default()
                    };
                    4
                ];
                photos.extend(
                    [AdjustmentLayerSettings {
                        exposure: 16.,
                        ..Default::default()
                    }; 2],
                );
                photos
            } else {
                vec![
                    AdjustmentLayerSettings {
                        exposure: 16.,
                        ..Default::default()
                    };
                    7
                ]
            };
            let last = photos.len() - 1;
            let mut layers = AdjustmentLayers::default();
            for (index, photo) in photos.into_iter().enumerate() {
                layers
                    .add(
                        "Range boundary".into(),
                        AdjustmentLayer {
                            photo,
                            enabled: reorder || index < last,
                            strength: 1.,
                            ..base
                        },
                    )
                    .unwrap();
            }
            let subject = layers.layers[last].id;
            source.adjustment_layers = Some(layers);
            editor.document.visit.adjustment_layer_settings =
                editor.selected_adjustment_layer_settings().unwrap();
            editor.sync_document();
            editor.select_adjustment_layer(Target::Layer(subject));
            let accepted = editor
                .positive_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings,
                )
                .unwrap_or_else(|error| panic!("accepted fixture reorder={reorder}: {error}"));
            let before = editor.document.state.project.clone();
            let revision = editor.document.visit.canvas.grade.original.revision();
            let viewport = editor.document.visit.canvas.viewport;
            editor.apply_adjustment_layer_action(if reorder {
                Action::Reorder(editor.order_drag(subject).unwrap(), 0)
            } else {
                Action::Enable(subject, true)
            });
            assert!(
                editor.status.contains("f32 intermediate working range"),
                "reorder={reorder}: {}",
                editor.status
            );
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(subject));
            assert_eq!(
                editor.document.visit.canvas.grade.original.revision(),
                revision
            );
            assert_eq!(editor.document.visit.canvas.viewport, viewport);
            let current = editor
                .positive_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings,
                )
                .unwrap();
            assert!(std::ptr::eq(
                accepted.grade().unwrap(),
                current.grade().unwrap()
            ));
        }
    });
}

#[test]
fn reenabling_a_finite_stack_is_not_blocked_by_a_precision_budget() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.document.roll = RollSettings::default();
        editor.document.visit.adjustment_layer_settings = AdjustmentLayerSettings::default();
        let source = editor
            .document
            .state
            .project
            .source_mut(editor.document.visit.current_source.unwrap())
            .unwrap();
        let base = source.adjustment_layers.as_ref().unwrap().layers[0].adjustment;
        let mut layers = AdjustmentLayers::default();
        for index in 0..4 {
            layers
                .add(
                    "Exposure".into(),
                    AdjustmentLayer {
                        photo: AdjustmentLayerSettings {
                            exposure: 16.,
                            ..Default::default()
                        },
                        enabled: index < 3,
                        strength: 1.,
                        ..base
                    },
                )
                .unwrap();
        }
        let subject = layers.layers[3].id;
        let allocator = layers.next_id;
        source.adjustment_layers = Some(layers);
        editor.refresh_adjustment_layer_selection();
        editor.sync_document();
        editor.select_adjustment_layer(Target::Layer(subject));
        let before = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        let viewport = editor.document.visit.canvas.viewport;
        editor.apply_adjustment_layer_action(Action::Enable(subject, true));
        let layers = editor
            .document
            .state
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap();
        assert!(layers.layers[3].adjustment.enabled);
        assert!(
            layers
                .layers
                .iter()
                .all(|record| record.adjustment.photo.exposure == 16.)
        );
        assert_eq!(layers.next_id, allocator);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(subject));
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
        let current = editor
            .positive_processing(
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        assert_ne!(before.signature(), current.signature());
    });
}

#[test]
fn disabling_and_editing_an_inactive_record_does_not_drop_its_mask_or_controls() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = ids(&editor)[0];
        let mask = editor
            .document
            .state
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment
            .mask;
        editor.reveal_adjustment_layers();
        editor.apply_adjustment_layer_action(Action::Enable(id, false));
        editor
            .edit_adjustment_layers(|layers| {
                record_mut(layers, id)?.adjustment.photo.exposure = 1.25;
                Ok(())
            })
            .unwrap();
        let adjustment = editor
            .document
            .state
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .adjustment;
        assert_eq!(adjustment.mask, mask);
        assert_eq!(adjustment.photo.exposure, 1.25);
        assert!(!adjustment.enabled);
        assert!(matches!(mask, AdjustmentLayerCoverage::Linear { .. }));
        editor.apply_adjustment_layer_action(Action::Enable(id, true));
        assert!(
            editor
                .document
                .state
                .project
                .source(editor.document.visit.current_source.unwrap())
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .adjustment
                .enabled
        );
    });
}
