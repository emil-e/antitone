use super::*;
use crate::project::Development;

fn fixture(index: usize) -> Editor {
    let mut editor = crate::editor::tests::fixture();
    for index in 0..5 {
        editor
            .document
            .state
            .register_with_development(
                Source {
                    id: SourceId(0),
                    path: format!("photo-{index}.ARW").into(),
                    development: crate::project::SourceDevelopment {
                        decode: Development::new("test-backend".into(), [1.; 4]),
                        lens_correction: None,
                        dng_development: None,
                        dimensions: [900, 600],
                        active_area: [0, 0, 900, 600],
                        camera_model: "test-camera".into(),
                        camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                    },
                    reference_only: false,
                    adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
                        AdjustmentLayerSettings::default(),
                    )),
                    repairs: Default::default(),
                    orientation: PhotoOrientation::default(),
                    crop: None,
                },
                crate::project::Development::new(crate::raw::version(), [1.; 4]),
            )
            .unwrap();
    }
    editor.document.visit.current_source = Some(editor.document.state.project.order[index]);
    editor.document.selected = index;
    editor
        .document
        .roll_selection
        .only(editor.document.visit.current_source.unwrap());
    editor
}

fn settle(editor: &mut Editor) {
    if editor.busy {
        assert!(editor.document.visit.source.pending());
        editor.source_result(editor.id, Err("fixture RAW unavailable".into()));
    }
}

#[test]
fn plain_command_and_shift_keep_active_selected_and_last_plain_anchor() {
    let editor = fixture(2);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.sync_document();
            let before = editor.document.state.project.clone();
            let ids = editor.document.state.project.order.clone();
            editor.select_roll_photo(4, egui::Modifiers::COMMAND, cx);
            assert_eq!(editor.document.roll_selection.ids, vec![ids[2], ids[4]]);
            assert_eq!(editor.document.visit.current_source, Some(ids[2]));
            assert!(!editor.document.visit.source.pending());
            editor.select_roll_photo(2, egui::Modifiers::COMMAND, cx);
            settle(editor);
            assert_eq!(editor.document.visit.current_source, Some(ids[4]));
            assert_eq!(editor.document.roll_selection.anchor, Some(ids[2]));
            editor.select_roll_photo(4, egui::Modifiers::COMMAND, cx);
            assert_eq!(editor.document.visit.current_source, Some(ids[4]));
            assert_eq!(editor.document.roll_selection.ids, vec![ids[4]]);
            assert!(
                !editor.document.visit.source.pending(),
                "sole selection cannot be deselected"
            );
            editor.select_roll_photo(0, egui::Modifiers::SHIFT, cx);
            settle(editor);
            assert_eq!(editor.document.roll_selection.ids, ids[..=2]);
            assert_eq!(
                editor.document.visit.current_source,
                Some(ids[2]),
                "range restores the last plain anchor as active"
            );
            editor.select_roll_photo(1, egui::Modifiers::NONE, cx);
            settle(editor);
            assert_eq!(editor.document.visit.current_source, Some(ids[1]));
            assert_eq!(
                editor.document.roll_selection.ids,
                ids[..=2],
                "activating a selected photo retains membership"
            );
            assert_eq!(editor.document.roll_selection.anchor, Some(ids[1]));
            editor.select_roll_photo(4, egui::Modifiers::SHIFT, cx);
            assert_eq!(editor.document.roll_selection.ids, ids[1..]);
            assert_eq!(editor.document.visit.current_source, Some(ids[1]));
            let request = editor.id;
            editor.select_active_photo_only();
            assert_eq!(editor.document.roll_selection.ids, vec![ids[1]]);
            assert_eq!(editor.document.roll_selection.anchor, Some(ids[1]));
            assert_eq!(editor.document.visit.current_source, Some(ids[1]));
            assert_eq!(editor.id, request);
            assert!(
                !editor.document.visit.source.pending(),
                "reducing selection does not reload the photo"
            );
            editor.select_roll_photo(0, egui::Modifiers::NONE, cx);
            settle(editor);
            assert_eq!(editor.document.visit.current_source, Some(ids[0]));
            assert_eq!(
                editor.document.roll_selection.ids,
                vec![ids[0]],
                "an unselected photo starts a new selection"
            );
            editor.sync_document();
            assert_eq!(
                editor.document.state.project, before,
                "selection is not photographic state"
            );
        });
    });
}

#[test]
fn pending_photo_navigation_keeps_multiselection_but_removal_waits_for_completion() {
    let editor = fixture(2);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let ids = editor.document.state.project.order.clone();
            editor.select_roll_photo(0, egui::Modifiers::NONE, cx);
            let first = editor.id;
            assert!(editor.document.visit.source.pending());
            assert!(editor.busy && editor.document.visit.selection_timing.is_some());
            editor.select_roll_photo(4, egui::Modifiers::COMMAND, cx);
            assert_eq!(editor.document.roll_selection.ids, vec![ids[0], ids[4]]);
            assert_eq!(editor.document.visit.current_source, Some(ids[0]));
            editor.request_removal();
            assert!(editor.document.removal_pending.is_none());
            editor.select_roll_photo(4, egui::Modifiers::NONE, cx);
            assert!(editor.document.visit.source.pending());
            assert_ne!(editor.id, first);
            assert_eq!(
                editor
                    .source_epoch
                    .load(std::sync::atomic::Ordering::Relaxed),
                editor.id
            );
            assert_eq!(editor.document.roll_selection.ids, vec![ids[0], ids[4]]);
            assert_eq!(editor.document.visit.current_source, Some(ids[4]));
            editor.source_result(first, Err("obsolete source".into()));
            assert!(editor.busy && editor.document.visit.current_source == Some(ids[4]));
            editor.source_result(editor.id, Err("fixture unavailable".into()));
            editor.request_removal();
            assert_eq!(editor.document.removal_pending, Some(vec![ids[0], ids[4]]));
        });
    });
}

#[test]
fn deselecting_active_prefers_selected_successor_then_predecessor() {
    let editor = fixture(2);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let ids = editor.document.state.project.order.clone();
            editor.select_roll_photo(0, egui::Modifiers::COMMAND, cx);
            editor.select_roll_photo(4, egui::Modifiers::COMMAND, cx);
            editor.select_roll_photo(2, egui::Modifiers::COMMAND, cx);
            settle(editor);
            assert_eq!(editor.document.visit.current_source, Some(ids[4]));
            editor.select_roll_photo(4, egui::Modifiers::COMMAND, cx);
            settle(editor);
            assert_eq!(editor.document.visit.current_source, Some(ids[0]));
            assert_eq!(editor.document.roll_selection.ids, vec![ids[0]]);
        });
    });
}

#[test]
fn removal_is_staged_cancellable_and_uses_old_index_in_the_updated_order() {
    for (index, removed, expected) in [(2, vec![0, 2], 4), (4, vec![0, 4], 3), (0, vec![0], 1)] {
        let editor = fixture(index);
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                let ids = editor.document.state.project.order.clone();
                editor.document.roll_selection.ids =
                    removed.iter().map(|index| ids[*index]).collect();
                editor.sync_document();
                let before = editor.document.state.project.clone();
                editor.request_removal();
                assert_eq!(editor.document.state.project, before);
                let selection = editor.document.roll_selection.ids.clone();
                editor.document.removal_pending = None;
                assert_eq!(editor.document.roll_selection.ids, selection);
                assert!(!editor.document.visit.source.pending());
                editor.request_removal();
                editor.remove_selected_photos(cx);
                assert_eq!(editor.document.visit.current_source, Some(ids[expected]));
                assert_eq!(
                    editor.document.selected,
                    index.min(editor.document.state.project.order.len() - 1)
                );
                assert_eq!(editor.document.roll_selection.ids, vec![ids[expected]]);
                let mut expected_project = before;
                expected_project.order.retain(|id| !selection.contains(id));
                expected_project
                    .sources
                    .retain(|source| !selection.contains(&source.id));
                expected_project
                    .analysis_regions
                    .retain(|entry| !selection.contains(&entry.source));
                assert_eq!(editor.document.state.project, expected_project);
                assert!(editor.document.removal_pending.is_none());
                assert!(editor.document.visit.source.pending());
            });
        });
    }
}

#[test]
fn removing_every_photo_clears_working_images_and_stale_replies_but_keeps_roll_basis() {
    let editor = fixture(2);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            crate::editor::tests::add_ready_fixture(editor);
            editor.document.roll_selection.ids = editor.document.state.project.order.clone();
            editor.sync_document();
            let mut expected = editor.document.state.project.clone();
            expected.order.clear();
            expected.sources.clear();
            expected.analysis_regions.clear();
            let old_id = editor.id;
            let ready = editor.document.visit.ready.take().unwrap();
            let source = Arc::downgrade(&ready.developed);
            editor.request_removal();
            editor.remove_selected_photos(cx);
            assert_eq!(editor.document.state.project, expected);
            assert!(editor.document.visit.current_source.is_none());
            assert!(
                editor.document.visit.ready.is_none()
                    && editor.document.visit.pending_preview.is_none()
            );
            assert!(
                editor.document.visit.grade.original.settled.is_none()
                    && editor.document.visit.grade.original.refining.is_none()
                    && editor.document.visit.grade.original.signature.is_none()
            );
            assert!(editor.document.roll_selection.ids.is_empty());
            assert_eq!(
                editor
                    .source_epoch
                    .load(std::sync::atomic::Ordering::Relaxed),
                editor.id
            );
            assert!(editor.document.visit.support.pending.is_none());
            assert!(!editor.document.visit.source.pending());
            editor.source_result(old_id, Ok(Box::new(ready)));
            assert!(
                editor.document.visit.ready.is_none()
                    && editor.document.visit.current_source.is_none()
            );
            assert!(
                source.upgrade().is_none(),
                "the retired source must be released after stale delivery"
            );
            editor.sync_document();
            assert_eq!(editor.document.state.project, expected);
            editor.document.validate().unwrap();
            let path = std::env::temp_dir().join(format!(
                "antitone-empty-removal-{}.json",
                std::process::id()
            ));
            let saved = editor
                .document
                .save(&path, crate::project::Save::New)
                .unwrap();
            assert!(!saved.dirty());
            let reopened = Document::open(&path).unwrap();
            assert_eq!(reopened.project, saved.project);
            std::fs::remove_file(path).unwrap();
        });
    });
}

#[test]
fn an_ordinary_picker_preserves_multiselection_and_anchor() {
    let editor = fixture(2);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            crate::editor::tests::add_ready_fixture(editor);
            editor.document.selected = editor.document.state.project.order.len() - 1;
            let active = editor.document.visit.current_source.unwrap();
            editor.document.roll_selection.only(active);
            editor.select_roll_photo(0, egui::Modifiers::COMMAND, cx);
            let selection = editor.document.roll_selection.ids.clone();
            let anchor = editor.document.roll_selection.anchor;
            editor.preview_picker(Pick::Dmin);
            settle(editor);
            assert_eq!(editor.document.roll_selection.ids, selection);
            assert_eq!(editor.document.roll_selection.anchor, anchor);
            editor.request_removal();
            assert!(editor.document.removal_pending.is_none());
            editor.finish_pick(cx);
            settle(editor);
            assert_eq!(editor.document.visit.current_source, Some(active));
            assert_eq!(editor.document.roll_selection.ids, selection);
            assert_eq!(editor.document.roll_selection.anchor, anchor);
        });
    });
}

#[test]
fn drafts_foreground_work_and_decisions_reject_selection_and_removal() {
    for blocker in [
        "busy",
        "pick",
        "failure",
        "transition",
        "analysis",
        "removal",
    ] {
        let editor = fixture(2);
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor
                    .document
                    .roll_selection
                    .ids
                    .push(editor.document.state.project.order[4]);
                match blocker {
                    "busy" => editor.busy = true,
                    "pick" => editor.document.visit.picking.display = Some(Pick::Dmax.display()),
                    "failure" => editor.report_failure(FailureKind::Save, "fixture".into()),
                    "transition" => editor.transition = Some(Transition::Close),
                    "analysis" => editor.document_pending = Some(12),
                    "removal" => {
                        editor.document.removal_pending =
                            Some(vec![editor.document.visit.current_source.unwrap()])
                    }
                    _ => unreachable!(),
                }
                let before = editor.document.state.project.clone();
                let selection = editor.document.roll_selection.ids.clone();
                let pending = editor.document.removal_pending.clone();
                editor.select_roll_photo(0, egui::Modifiers::NONE, cx);
                editor.select_active_photo_only();
                editor.request_removal();
                assert_eq!(editor.document.state.project, before);
                assert_eq!(editor.document.roll_selection.ids, selection);
                assert_eq!(editor.document.removal_pending, pending);
                assert!(!editor.document.visit.source.pending());
            });
        });
    }
}
