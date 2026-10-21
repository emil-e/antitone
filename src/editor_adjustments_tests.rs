use super::*;
use crate::local_adjustments::{GradientMask, LocalAdjustment};

fn ids(editor: &Editor) -> Vec<AdjustmentId> {
    editor
        .document
        .project
        .source(editor.current_source.unwrap())
        .unwrap()
        .local_adjustments
        .as_ref()
        .unwrap()
        .adjustments
        .iter()
        .map(|record| record.id)
        .collect()
}

#[test]
fn list_actions_keep_identity_and_reject_candidates_without_mutating_accepted_resources() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = editor.current_source.unwrap();
        let order = ids(&editor);
        editor.open_adjustments();
        editor.apply_adjustment_action(Action::Select(Target::Local(order[0])));
        assert_eq!(editor.adjustment_target(), Target::Local(order[0]));
        let accepted = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        let revision = editor.original.revision();
        editor.apply_adjustment_action(Action::Rename(order[0], "Sky".into()));
        assert_eq!(
            editor.original.revision(),
            revision,
            "metadata must not invalidate processing"
        );
        let renamed = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        assert!(std::ptr::eq(
            accepted.grade().unwrap(),
            renamed.grade().unwrap()
        ));
        editor.apply_adjustment_action(Action::Reorder(editor.order_drag(order[0]).unwrap(), 2));
        assert_eq!(ids(&editor), vec![order[1], order[0]]);
        assert_eq!(editor.adjustment_target(), Target::Local(order[0]));
        assert_ne!(
            editor
                .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
                .unwrap()
                .signature(),
            accepted.signature()
        );
        editor.apply_adjustment_action(Action::Enable(order[0], false));
        let source = editor.document.project.source(id).unwrap();
        assert!(
            !source.local_adjustments.as_ref().unwrap().adjustments[1]
                .adjustment
                .enabled
        );
        let before = editor.document.project.clone();
        let revision = editor.original.revision();
        assert!(
            editor
                .edit_adjustments(|locals| {
                    record_mut(locals, order[0])?.adjustment.photo.contrast = f64::NAN;
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.original.revision(), revision);
        editor.apply_adjustment_action(Action::Rename(order[0], "  ".into()));
        assert_eq!(editor.document.project, before);
        editor.apply_adjustment_action(Action::Reorder(editor.order_drag(order[0]).unwrap(), 3));
        assert_eq!(editor.document.project, before);
        editor.busy = true;
        editor.apply_adjustment_action(Action::Select(Target::Global));
        editor.apply_adjustment_action(Action::Enable(order[0], true));
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.adjustment_target(), Target::Local(order[0]));
        editor.current_source = None;
        assert_eq!(editor.adjustment_target(), Target::Global);
    });
}

#[test]
fn deferred_drag_captures_locals_after_an_outgoing_rename_commits() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.open_adjustments();
        let order = ids(&editor);
        let key = OrderKey {
            loaded: editor.id,
            source: editor.current_source.unwrap(),
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
        let locals = editor
            .document
            .project
            .source(key.source)
            .unwrap()
            .local_adjustments
            .as_ref()
            .unwrap();
        assert_eq!(locals.adjustments[1].name, "Renamed");
        assert!(!editor.document.visit.adjustments.dragging());
    });
}

#[test]
fn drop_positions_insert_without_swapping_neighbours_or_consuming_ids() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor
            .edit_adjustments(|locals| {
                let mut adjustment = locals.adjustments[0].adjustment;
                adjustment.photo = PhotoSettings::default();
                locals.add("Identity".into(), adjustment)?;
                Ok(())
            })
            .unwrap();
        let order = ids(&editor);
        editor.select_adjustment(Target::Local(order[0]));
        let input = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        let snapshot = input.grade().unwrap().snapshot().clone();
        let next_id = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .local_adjustments
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
        assert_eq!(editor.adjustment_target(), Target::Local(order[0]));
        assert_eq!(
            editor
                .document
                .project
                .source(editor.current_source.unwrap())
                .unwrap()
                .local_adjustments
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
            editor.select_adjustment(Target::Local(id));
            let drag = editor.order_drag(id).unwrap();
            if changed_records {
                editor
                    .edit_adjustments(|locals| {
                        locals.adjustments[0].name = "Renamed during drag".into();
                        Ok(())
                    })
                    .unwrap();
            } else {
                editor.id += 1;
            }
            let before = editor.document.project.clone();
            let revision = editor.original.revision();
            let input = editor
                .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
                .unwrap();
            assert!(editor.reorder_adjustment(drag, 2).is_err());
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.original.revision(), revision);
            assert_eq!(editor.adjustment_target(), Target::Local(id));
            let current = editor
                .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
                .unwrap();
            assert!(std::ptr::eq(
                input.grade().unwrap(),
                current.grade().unwrap()
            ));
        }
    });
}

#[test]
fn drawer_visibility_routes_global_and_reopening_validates_source_owned_selection() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let source = editor.current_source.unwrap();
        let order = ids(&editor);
        editor.select_adjustment(Target::Local(order[1]));
        let before = editor.document.project.clone();
        let input = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        editor.close_adjustments();
        assert_eq!(editor.adjustment_target(), Target::Global);
        assert!(!editor.document.visit.adjustments.drawer_open());
        assert!(!editor.mask_handles_available());
        assert_eq!(editor.document.project, before);
        assert_eq!(
            input.signature(),
            editor
                .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
                .unwrap()
                .signature()
        );
        editor.open_adjustments();
        assert_eq!(editor.adjustment_target(), Target::Local(order[1]));
        editor.close_adjustments();
        editor
            .edit_adjustments(|locals| locals.remove(order[1]))
            .unwrap();
        editor.open_adjustments();
        assert_eq!(editor.adjustment_target(), Target::Local(order[0]));
        editor.close_adjustments();
        let mut other = editor.document.project.source(source).unwrap().clone();
        other.id = SourceId(0);
        other.path = "another-local-owner.ARW".into();
        let other = editor.document.project.add_source(other).unwrap();
        editor.current_source = Some(other);
        let before = editor.document.project.clone();
        editor.open_adjustments();
        assert_eq!(
            editor.document.visit.adjustments.selected,
            Some((other, order[0]))
        );
        assert_eq!(editor.document.project, before);
    });
}

#[test]
fn failed_document_candidate_preserves_the_visit_and_its_local_tool_context() {
    pollster::block_on(async {
        let (editor, _, _) = processing::tests::fixture().await;
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                let id = ids(editor)[1];
                editor.select_adjustment(Target::Local(id));
                editor.document.visit.adjustments.show_coverage = true;
                editor.sync_document();
                let accepted = editor.document.project.clone();
                let visit = editor.document.visit.view;
                editor.start_document(Preparation::Open("unavailable.antitone".into()), cx);
                let request = editor.document_pending.unwrap();
                editor.document_prepared(request, Err("candidate rejected".into()));
                assert_eq!(editor.document.visit.view, visit);
                assert_eq!(editor.adjustment_target(), Target::Local(id));
                assert!(editor.document.visit.adjustments.drawer_open());
                assert!(editor.document.visit.adjustments.show_coverage);
                assert_eq!(editor.document.project, accepted);
            });
        });
    });
}

#[test]
fn late_local_actions_cannot_mutate_the_stack_after_the_tool_closes() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = ids(&editor)[0];
        editor.select_adjustment(Target::Local(id));
        let drag = editor.order_drag(id).unwrap();
        editor.close_adjustments();
        let before = editor.document.project.clone();
        editor.apply_adjustment_action(Action::Enable(id, false));
        editor.apply_adjustment_action(Action::Reorder(drag, 2));
        editor.apply_adjustment_action(Action::Delete(id));
        editor.apply_adjustment_action(Action::Select(Target::Local(id)));
        assert!(!editor.document.visit.adjustments.confirming());
        assert!(!editor.document.visit.adjustments.drawer_open());
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.adjustment_target(), Target::Global);
    });
}

#[test]
fn explicitly_closing_during_a_roll_pick_prevents_its_return_context_reopening_locals() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = ids(&editor)[0];
        editor.select_adjustment(Target::Local(id));
        editor.begin_pick(Pick::Dmin, &crate::application::UiContext::render_only());
        assert!(editor.document.pick_return.is_some());
        let before = editor.document.project.clone();
        editor.close_adjustments();
        editor.finish_pick(&crate::application::UiContext::render_only());
        assert_eq!(editor.adjustment_target(), Target::Global);
        assert!(!editor.document.visit.adjustments.drawer_open());
        assert_eq!(editor.document.project, before);
    });
}

#[test]
fn unsupported_local_edit_preserves_grade_view_revision_and_allocator() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let accepted = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        let before = editor.document.project.clone();
        let revision = editor.original.revision();
        let viewport = editor.viewport;
        let result = editor.edit_adjustments(|locals| {
            let mut adjustment = locals.adjustments[0].adjustment;
            adjustment.photo.exposure = 16.;
            adjustment.photo.contrast = 1.6;
            adjustment.strength = 1.;
            for _ in 0..16 {
                locals.add("Unsupported".into(), adjustment)?;
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.original.revision(), revision);
        assert_eq!(editor.viewport, viewport);
        let current = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
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
            editor.roll = RollSettings::new_grade();
            editor.photo = PhotoSettings::default();
            assert_eq!(editor.roll.gamma, 0.6);
            let source = editor
                .document
                .project
                .source_mut(editor.current_source.unwrap())
                .unwrap();
            let base = source.local_adjustments.as_ref().unwrap().adjustments[0].adjustment;
            let photos = if reorder {
                vec![
                    PhotoSettings {
                        contrast: 0.6,
                        contrast_colour_mix: 1.,
                        ..Default::default()
                    },
                    PhotoSettings {
                        exposure: 12.,
                        ..Default::default()
                    },
                    PhotoSettings {
                        exposure: 12.,
                        ..Default::default()
                    },
                ]
            } else {
                vec![
                    PhotoSettings {
                        exposure: 16.,
                        ..Default::default()
                    };
                    4
                ]
            };
            let mut locals = LocalAdjustments::default();
            for (index, photo) in photos.into_iter().enumerate() {
                locals
                    .add(
                        "Precision boundary".into(),
                        LocalAdjustment {
                            photo,
                            enabled: reorder || index < 3,
                            strength: 1.,
                            ..base
                        },
                    )
                    .unwrap();
            }
            let subject = locals.adjustments[if reorder { 0 } else { 3 }].id;
            source.local_adjustments = Some(locals);
            editor.sync_document();
            editor.select_adjustment(Target::Local(subject));
            let accepted = editor
                .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
                .unwrap();
            let before = editor.document.project.clone();
            let revision = editor.original.revision();
            let viewport = editor.viewport;
            editor.apply_adjustment_action(if reorder {
                Action::Reorder(editor.order_drag(subject).unwrap(), 2)
            } else {
                Action::Enable(subject, true)
            });
            assert!(
                editor.status.contains("normal GPU precision domain"),
                "{}",
                editor.status
            );
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.adjustment_target(), Target::Local(subject));
            assert_eq!(editor.original.revision(), revision);
            assert_eq!(editor.viewport, viewport);
            let current = editor
                .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
                .unwrap();
            assert!(std::ptr::eq(
                accepted.grade().unwrap(),
                current.grade().unwrap()
            ));
        }
    });
}

#[test]
fn temporary_roll_picking_restores_only_an_existing_source_owned_target() {
    pollster::block_on(async {
        for removed in [false, true] {
            let (editor, _, _) = processing::tests::fixture().await;
            crate::application::test_support::with_application(editor, |app| {
                app.enter(|editor, cx| {
                    let original = editor.current_source.unwrap();
                    let target = ids(editor)[0];
                    let mut other = editor.document.project.source(original).unwrap().clone();
                    other.id = SourceId(0);
                    other.path = "other-fixture.ARW".into();
                    let other = editor.document.project.add_source(other).unwrap();
                    editor.sync_document();
                    editor.select_adjustment(Target::Local(target));
                    editor.begin_pick(Pick::Dmin, cx);
                    let mut ready = editor.document.visit.ready.take().unwrap();
                    editor.pick_roll_source(other, cx);
                    assert_eq!(editor.current_source, Some(other));
                    assert_eq!(editor.adjustment_target(), Target::Global);
                    assert!(editor.document.visit.source.pending());
                    // Both fixture sources use identical pixels/metadata; deliver the other owner's Ready.
                    ready.path = editor.document.source_path(other).unwrap();
                    ready.record = editor.document.project.source(other).unwrap().clone();
                    ready.record.id = SourceId(0);
                    ready.record.path = ready.path.clone();
                    editor.source_result(editor.id, Ok(Box::new(ready)));
                    assert!(
                        editor.document.visit.ready.is_some() && !editor.busy,
                        "{}",
                        editor.status
                    );
                    if removed {
                        editor
                            .document
                            .project
                            .source_mut(original)
                            .unwrap()
                            .local_adjustments
                            .as_mut()
                            .unwrap()
                            .adjustments
                            .clear();
                    }
                    editor.finish_pick(cx);
                    assert_eq!(editor.current_source, Some(original));
                    assert!(editor.document.pick_return.is_none());
                    assert_eq!(
                        editor.adjustment_target(),
                        if removed {
                            Target::Global
                        } else {
                            Target::Local(target)
                        }
                    );
                    assert!(editor.document.visit.source.pending());
                });
            });
        }
    });
}

#[test]
fn disabling_and_editing_an_inactive_record_does_not_drop_its_mask_or_controls() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = ids(&editor)[0];
        let mask = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .local_adjustments
            .as_ref()
            .unwrap()
            .adjustments[0]
            .adjustment
            .mask;
        editor.open_adjustments();
        editor.apply_adjustment_action(Action::Enable(id, false));
        editor
            .edit_adjustments(|locals| {
                record_mut(locals, id)?.adjustment.photo.exposure = 1.25;
                Ok(())
            })
            .unwrap();
        let adjustment = editor
            .document
            .project
            .source(editor.current_source.unwrap())
            .unwrap()
            .local_adjustments
            .as_ref()
            .unwrap()
            .adjustments[0]
            .adjustment;
        assert_eq!(adjustment.mask, mask);
        assert_eq!(adjustment.photo.exposure, 1.25);
        assert!(!adjustment.enabled);
        assert!(matches!(mask, GradientMask::Linear { .. }));
        editor.apply_adjustment_action(Action::Enable(id, true));
        assert!(
            editor
                .document
                .project
                .source(editor.current_source.unwrap())
                .unwrap()
                .local_adjustments
                .as_ref()
                .unwrap()
                .adjustments[0]
                .adjustment
                .enabled
        );
    });
}
