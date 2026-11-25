use super::*;

mod history_tests;
use crate::adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage};
use crate::photographic::test_inputs::AuthoredPhotograph;

fn reinterpret_fixture(editor: &mut Editor) {
    let ready = editor.document.visit.canvas.ready.take();
    editor.reinterpret_photo();
    if let Some(ready) = ready {
        crate::editor::source::testing::complete(editor, Ok(Box::new(ready)));
    }
}

fn begin_reorder(editor: &mut Editor, id: AdjustmentLayerId) -> OrderKey {
    let key = OrderKey(id);
    editor.document.visit.adjustment_layers.reorder.0 = Some(key);
    key
}

fn reorder_layer(editor: &mut Editor, id: AdjustmentLayerId, position: DropPosition) {
    let key = begin_reorder(editor, id);
    apply_drop(editor, key, position);
}

#[test]
fn film_seed_routing_uses_source_identity_not_visit_identity() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.document.project.output.grain.enabled = true;
        let seed = |input: &AuthoredPhotograph| {
            let definition = input.definition();
            assert!(
                definition.grain.is_some_and(|grain| grain.amplitude > 0.),
                "fixture must request actual grain"
            );
            definition.seed
        };
        let source = editor.document.visit.current_source.unwrap();
        let first = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        assert_eq!(seed(&first), source.0);
        reinterpret_fixture(&mut editor);
        let reacquired = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        assert_eq!(seed(&first), seed(&reacquired));
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
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        assert_eq!(seed(&changed), other.0);
        assert_ne!(seed(&first), seed(&changed));
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
        editor.apply_adjustment_layer_action(
            Action::Select(Target::Layer(order[0])),
            &UiContext::render_only(),
        );
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        let accepted = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        let revision = editor.document.visit.canvas.original.continuation();
        editor.apply_adjustment_layer_action(
            Action::Rename(order[0], "Sky".into()),
            &UiContext::render_only(),
        );
        assert_eq!(
            editor.document.visit.canvas.original.continuation(),
            revision,
            "metadata must not invalidate processing"
        );
        let renamed = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        assert_eq!(accepted.clone(), renamed.clone());
        reorder_layer(&mut editor, order[0], DropPosition::Above(order[1]));
        assert_eq!(ids(&editor), vec![order[1], order[0]]);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        assert_ne!(
            editor
                .positive_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings()
                )
                .unwrap()
                .clone(),
            accepted.clone()
        );
        editor.apply_adjustment_layer_action(
            Action::Enable(order[0], false),
            &UiContext::render_only(),
        );
        let source = editor.document.state.project.source(id).unwrap();
        assert!(
            !source.adjustment_layers.as_ref().unwrap().layers[1]
                .adjustment
                .enabled
        );
        let before = editor.document.state.project.clone();
        let revision = editor.document.visit.canvas.original.continuation();
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
            editor.document.visit.canvas.original.continuation(),
            revision
        );
        editor.apply_adjustment_layer_action(
            Action::Rename(order[0], "  ".into()),
            &UiContext::render_only(),
        );
        assert_eq!(editor.document.state.project, before);
        reorder_layer(
            &mut editor,
            order[0],
            DropPosition::Below(AdjustmentLayerId(u64::MAX)),
        );
        assert_eq!(editor.document.state.project, before);
        editor.set_test_busy(true);
        editor
            .apply_adjustment_layer_action(Action::Select(Target::None), &UiContext::render_only());
        editor.apply_adjustment_layer_action(
            Action::Enable(order[0], true),
            &UiContext::render_only(),
        );
        assert_eq!(editor.document.state.project, before);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        editor.document.visit.current_source = None;
        assert_eq!(editor.adjustment_layer_target(), Target::None);
    });
}

#[test]
fn drop_uses_current_order_and_preserves_edits_made_during_the_drag() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.reveal_adjustment_layers();
        let order = ids(&editor);
        let key = begin_reorder(&mut editor, order[0]);
        editor
            .edit_adjustment_layers(|layers| {
                layers.layers[0].name = "Renamed".into();
                layers.layers[0].adjustment.strength = 0.5;
                let mut extra = layers.layers[0].adjustment;
                extra.photo = AdjustmentLayerSettings::default();
                layers.add("Unrelated".into(), extra)?;
                layers.layers.rotate_right(1);
                Ok(())
            })
            .unwrap();

        let mut expected = editor.document.state.project.clone();
        let source = expected
            .source_mut(editor.document.visit.current_source.unwrap())
            .unwrap();
        let records = &mut source.adjustment_layers.as_mut().unwrap().layers;
        records.swap(1, 2);
        apply_drop(&mut editor, key, DropPosition::Above(order[1]));
        assert_eq!(editor.document.state.project, expected);
        assert!(editor.document.visit.adjustment_layers.reorder.0.is_none());
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
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
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
        editor.reveal_adjustment_layers();
        reorder_layer(&mut editor, order[0], DropPosition::Above(order[2]));
        assert_eq!(ids(&editor), vec![order[1], order[2], order[0]]);
        reorder_layer(&mut editor, order[0], DropPosition::Below(order[1]));
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
fn layer_drop_preserves_accepted_values_after_layer_removal_or_scope_retirement() {
    pollster::block_on(async {
        for removed_layer in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            editor.reveal_adjustment_layers();
            let order = ids(&editor);
            let id = order[0];
            editor.select_adjustment_layer(Target::Layer(id));
            let key = begin_reorder(&mut editor, id);
            assert!(editor.document.visit.adjustment_layers.reorder.0.is_some());
            if removed_layer {
                editor
                    .edit_adjustment_layers(|layers| layers.remove(id))
                    .unwrap();
            } else {
                let departed = editor.visit_lifetime();
                reinterpret_fixture(&mut editor);
                assert!(departed.upgrade().is_none());
                assert!(editor.document.visit.adjustment_layers.reorder.0.is_none());
            }
            let before = editor.document.state.project.clone();
            let target = editor.adjustment_layer_target();
            let revision = editor.document.visit.canvas.original.continuation();
            let input = editor
                .positive_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
            apply_drop(&mut editor, key, DropPosition::Above(order[1]));
            if removed_layer {
                assert!(
                    editor
                        .status
                        .contains("dragged adjustment layer no longer exists")
                );
            }
            assert!(editor.document.visit.adjustment_layers.reorder.0.is_none());
            assert_eq!(editor.document.state.project, before);
            assert_eq!(
                editor.document.visit.canvas.original.continuation(),
                revision
            );
            assert_eq!(editor.adjustment_layer_target(), target);
            let current = editor
                .positive_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
            assert_eq!(input.clone(), current.clone());
        }
    });
}

#[test]
fn selection_makes_itself_valid_after_arbitrary_stack_changes_without_resurrecting_a_removed_target()
 {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let source = editor.document.visit.current_source.unwrap();
        let order = ids(&editor);
        editor.select_adjustment_layer(Target::Layer(order[1]));
        let original = editor.document.project.clone();
        editor
            .document
            .project
            .source_mut(source)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .remove(order[1])
            .unwrap();
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        assert_eq!(
            editor.photo_settings(),
            editor.selected_adjustment_layer_settings().unwrap()
        );
        editor.document.state.project = original.clone();
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        editor
            .document
            .project
            .source_mut(source)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
            .clear();
        assert_eq!(editor.adjustment_layer_target(), Target::None);
        editor.document.state.project = original;
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[1]));
        editor.select_adjustment_layer(Target::Layer(order[0]));
        editor.document.visit.current_source = None;
        assert_eq!(editor.adjustment_layer_target(), Target::None);
        editor.document.visit.current_source = Some(source);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[1]));
    });
}

#[test]
fn independent_tool_start_preserves_layer_workspace() {
    pollster::block_on(async {
        for dust in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            let id = ids(&editor)[1];
            let source = editor.document.visit.current_source.unwrap();
            editor
                .apply_adjustment_layer_action(Action::RenameStart(id), &UiContext::render_only());
            let rename = (source, id, "Private layer name".to_owned());
            editor.document.visit.adjustment_layers.rename = Some(rename.clone());
            editor.document.visit.adjustment_layers.show_coverage = true;
            let before = editor.document.state.project.clone();

            if dust {
                editor.begin_dust_cleanup();
                assert!(editor.dusting());
            } else {
                editor.begin_crop();
                assert!(editor.cropping());
            }

            assert_eq!(editor.adjustment_layer_target(), Target::Layer(id));
            assert_eq!(editor.document.visit.adjustment_layers.rename, Some(rename));
            assert!(editor.document.visit.adjustment_layers.show_coverage);
            assert_eq!(editor.document.state.project, before);
        }
    });
}

#[test]
fn starting_crop_preserves_the_selected_layer_and_accepted_grade() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;

        let order = ids(&editor);
        editor.select_adjustment_layer(Target::Layer(order[1]));
        let before = editor.document.state.project.clone();
        let processing = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        editor.begin_crop();
        assert!(editor.cropping());
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[1]));
        assert_eq!(editor.document.state.project, before);
        assert_eq!(
            processing.clone(),
            editor
                .positive_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings()
                )
                .unwrap()
                .clone()
        );
        editor
            .edit_adjustment_layers(|layers| layers.remove(order[1]))
            .unwrap();
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(order[0]));
        assert_eq!(
            editor.photo_settings(),
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
fn input_switch_keeps_the_photo_and_accepted_layers_but_resets_its_workspace() {
    pollster::block_on(async {
        let (editor, _, _) = processing::tests::fixture().await;
        crate::application::test_support::with_application(editor, |app| {
            let (fallback, accepted, source, drag) = app.enter(|editor, cx| {
                let layers = ids(editor);
                editor.select_adjustment_layer(Target::Layer(layers[0]));
                assert_eq!(editor.adjustment_layer_target(), Target::Layer(layers[0]));
                editor.document.visit.adjustment_layers.show_coverage = true;
                editor.document.visit.canvas.viewport.zoom = Some(2.);
                let accepted = editor.document.state.project.sources.clone();
                let source = editor.document.visit.current_source;
                let drag = begin_reorder(editor, layers[0]);
                editor.request_input(crate::processing::InputSettings::Digital, cx);
                if editor.dialogs.is_open() {
                    editor.dialogs.answer_for_test("Switch stage");
                }
                (layers[1], accepted, source, drag)
            });
            app.run_until(std::time::Duration::from_secs(5), |editor| {
                editor.document.input_change.is_none()
            });
            app.enter(|editor, _| {
                assert_eq!(
                    editor.document.project.input,
                    crate::processing::InputSettings::Digital
                );
                assert_eq!(editor.document.visit.current_source, source);
                assert_eq!(editor.document.visit.canvas.viewport, Viewport::default());
                assert_eq!(editor.adjustment_layer_target(), Target::Layer(fallback));
                assert_eq!(editor.tab, 1);
                apply_drop(editor, drag, DropPosition::Above(fallback));
                assert!(!editor.document.visit.adjustment_layers.show_coverage);
                assert_eq!(editor.document.project.sources, accepted);
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
        let key = begin_reorder(&mut editor, id);
        editor.tab = 0;
        let before = editor.document.state.project.clone();
        editor.apply_adjustment_layer_action(Action::Enable(id, false), &UiContext::render_only());
        let target = ids(&editor)[1];
        apply_drop(&mut editor, key, DropPosition::Above(target));
        editor.apply_adjustment_layer_action(Action::Delete(id), &UiContext::render_only());
        editor.apply_adjustment_layer_action(
            Action::Select(Target::Layer(id)),
            &UiContext::render_only(),
        );
        assert!(!editor.dialogs.is_open());
        assert!(!(editor.tab == 1));
        assert_eq!(editor.document.state.project, before);
        assert_eq!(editor.adjustment_layer_target(), Target::Layer(id));
    });
}

#[test]
fn ending_a_roll_pick_preserves_the_selected_layer() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = ids(&editor)[0];
        editor.select_adjustment_layer(Target::Layer(id));
        editor.preview_picker(PickerCase::Dmin);
        assert_eq!(editor.test_pick(), Some(PickerCase::Dmin.display()));
        let before = editor.document.state.project.clone();
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
        app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Dmin, cx));
        app.pump();
        app.update(|editor| {
            assert_eq!(editor.test_pick(), Some(PickerCase::Dmin.display()));
            editor.select_adjustment_layer(Target::Layer(second));
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(second));
        });
        app.enter(|editor, cx| editor.finish_pick(cx));
    });
}

#[test]
fn malformed_mask_edit_preserves_grade_view_revision_and_allocator() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let accepted = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        let before = editor.document.state.project.clone();
        let revision = editor.document.visit.canvas.original.continuation();
        let viewport = editor.document.visit.canvas.viewport;
        let result = editor.edit_adjustment_layers(|layers| {
            let mut adjustment = layers.layers[0].adjustment;
            adjustment.photo.exposure = 16.;
            adjustment.photo.contrast = 1.6;
            adjustment.strength = 1.;
            for _ in 0..16 {
                layers.add("Exposure".into(), adjustment)?;
            }
            adjustment.mask = AdjustmentLayerCoverage::Radial {
                center: crate::geometry::SourcePoint([0., 0.]),
                radii: [0., 1.],
                angle: 0.,
                feather: 0.5,
            };
            layers.add("Zero-radius mask".into(), adjustment)?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(editor.document.state.project, before);
        assert_eq!(
            editor.document.visit.canvas.original.continuation(),
            revision
        );
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
        let current = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        assert_eq!(accepted.clone(), current.clone());
    });
}

#[test]
fn deep_stack_reorder_and_reenable_preserve_selection_allocator_and_viewport() {
    pollster::block_on(async {
        for reorder in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            editor.document.project.roll = RollSettings::default();
            *editor.photo_settings_mut() = AdjustmentLayerSettings::default();
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

            editor.select_adjustment_layer(Target::Layer(subject));
            let accepted = editor
                .positive_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap_or_else(|error| panic!("accepted fixture reorder={reorder}: {error}"));
            let before = editor.document.state.project.clone();
            let viewport = editor.document.visit.canvas.viewport;
            if reorder {
                let first = ids(&editor)[0];
                reorder_layer(&mut editor, subject, DropPosition::Below(first));
            } else {
                editor.apply_adjustment_layer_action(
                    Action::Enable(subject, true),
                    &UiContext::render_only(),
                );
            }
            assert_ne!(editor.document.state.project, before);
            assert_eq!(editor.adjustment_layer_target(), Target::Layer(subject));
            let source = editor
                .document
                .state
                .project
                .source(editor.document.visit.current_source.unwrap())
                .unwrap();
            let updated = source.adjustment_layers.as_ref().unwrap();
            let original = before
                .source(source.id)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap();
            assert_eq!(updated.next_id, original.next_id);
            if reorder {
                assert_eq!(updated.layers[0].id, subject);
            } else {
                assert!(
                    updated
                        .layers
                        .iter()
                        .find(|layer| layer.id == subject)
                        .unwrap()
                        .adjustment
                        .enabled
                );
            }
            assert_eq!(editor.document.visit.canvas.viewport, viewport);
            let current = editor
                .positive_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
            assert_ne!(accepted, current);
        }
    });
}

#[test]
fn reenabling_a_finite_stack_is_not_blocked_by_a_precision_budget() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.document.project.roll = RollSettings::default();
        *editor.photo_settings_mut() = AdjustmentLayerSettings::default();
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
        editor.select_adjustment_layer(Target::Layer(subject));
        let before = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        let viewport = editor.document.visit.canvas.viewport;
        editor.apply_adjustment_layer_action(
            Action::Enable(subject, true),
            &UiContext::render_only(),
        );
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
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        assert_ne!(before.clone(), current.clone());
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
        editor.apply_adjustment_layer_action(Action::Enable(id, false), &UiContext::render_only());
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
        editor.apply_adjustment_layer_action(Action::Enable(id, true), &UiContext::render_only());
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
