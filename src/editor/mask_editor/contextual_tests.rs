use super::*;

mod history_tests;

fn start(editor: &mut Editor) -> AdjustmentLayerId {
    editor.tab = 1;
    let source = editor.document.visit.current_source.unwrap();
    let id = editor
        .document
        .project
        .source(source)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers[0]
        .id;
    editor.select_adjustment_layer(adjustment_layers::Target::Layer(id));
    editor.document.visit.working_view.compare = true;
    editor.begin_linear_mask(Some(id));
    let edit = editor.document.visit.masks.gesture_mut().unwrap();
    edit.mask = Some(AdjustmentLayerCoverage::Linear {
        start: SourcePoint([100., 70.]),
        end: SourcePoint([149., 99.]),
    });
    assert!(editor.mask_preview_source().is_some());
    assert!(editor.document.visit.working_view.compare);
    assert!(!editor.comparing_original());
    assert!(!editor.tool_interacting());
    assert!(editor.current_tool().is_none());
    id
}

#[test]
fn visit_retirement_and_layer_selection_discard_only_the_unfinished_refinement() {
    pollster::block_on(async {
        for depart in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;

            let before = editor.document.state.project.clone();
            let history = editor.history_label(history::Direction::Undo);
            start(&mut editor);
            if depart {
                let next = editor.request_id();
                editor.replace_photo_scope(next);
            } else {
                editor.select_adjustment_layer(adjustment_layers::Target::None);
            }
            assert!(!editor.document.visit.masks.editing());
            assert!(editor.mask_preview_source().is_none());
            assert!(editor.document.visit.working_view.compare);
            assert!(editor.comparing_original());
            editor.finish_mask(true);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.history_label(history::Direction::Undo), history);
        }
    });
}

#[test]
fn refinement_changes_only_geometry_and_preserves_later_accepted_controls() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let id = start(&mut editor);
        editor
            .edit_adjustment_layers(|layers| {
                layers
                    .layers
                    .iter_mut()
                    .find(|layer| layer.id == id)
                    .unwrap()
                    .adjustment
                    .photo
                    .exposure = 0.75;
                Ok(())
            })
            .unwrap();
        let graded = editor.document.state.project.clone();
        let source = editor.document.visit.current_source.unwrap();
        assert_eq!(
            graded
                .source(source)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers
                .iter()
                .find(|layer| layer.id == id)
                .unwrap()
                .adjustment
                .photo
                .exposure,
            0.75
        );
        let mut expected = graded.clone();
        expected
            .source_mut(source)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .unwrap()
            .adjustment
            .mask = editor.document.visit.masks.gesture().unwrap().mask.unwrap();
        assert!(editor.mask_preview_source().is_some());
        editor.finish_mask(true);
        assert!(!editor.document.visit.masks.editing());
        assert_eq!(editor.document.state.project, expected);
        editor.restore_history(history::Direction::Undo, &UiContext::render_only());
        assert_eq!(editor.document.state.project, graded);
        assert!(editor.document.visit.working_view.compare);
        assert!(editor.comparing_original());
    });
}

#[test]
fn document_admission_retires_the_gesture_before_preparation_is_polled() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        start(&mut editor);
        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, _| editor.new_project());
            app.read(|editor| {
                assert!(
                    !editor.document.visit.masks.editing(),
                    "replacement retires the unfinished refinement"
                );
                assert_ne!(editor.document.state.project, before);
                assert!(editor.document.project.sources.is_empty());
            });
        });
    });
}

#[test]
fn removed_or_changed_mask_targets_cannot_be_overwritten_by_a_held_refinement() {
    pollster::block_on(async {
        for remove in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            let id = start(&mut editor);
            editor
                .edit_adjustment_layers(|layers| {
                    if remove {
                        layers.remove(id)?;
                    } else {
                        layers
                            .layers
                            .iter_mut()
                            .find(|layer| layer.id == id)
                            .unwrap()
                            .adjustment
                            .mask = AdjustmentLayerCoverage::Linear {
                            start: SourcePoint([2., 3.]),
                            end: SourcePoint([18., 22.]),
                        };
                    }
                    Ok(())
                })
                .unwrap();
            let accepted = editor.document.state.project.clone();
            let history = editor
                .history_label(history::Direction::Undo)
                .map(str::to_owned);
            assert!(editor.mask_preview_source().is_none());
            editor.finish_mask(true);
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                history.as_deref()
            );
            assert!(
                editor.status.starts_with("Mask rejected:"),
                "{}",
                editor.status
            );
        }
    });
}

#[test]
fn save_and_export_are_independent_of_document_replacement_gesture_admission() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        assert!(
            editor.file_available(),
            "idle contextual handles do not block file actions"
        );
        start(&mut editor);
        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.read(|editor| {
                assert!(!editor.tool_interacting());
                assert!(!editor.file_available());
            });
            app.enter(|editor, cx| editor.file_action(FileAction::New, cx));
            app.update(Editor::open_export_dialog);
            app.read(|editor| {
                assert!(!editor.files.pending());
                assert!(editor.document.export.form.is_some());
                assert!(editor.document.visit.masks.editing());
                assert_eq!(editor.document.state.project, before);
            });
            app.update(|editor| {
                editor.document.export.form = None;
                editor.finish_mask(false);
            });
            app.read(|editor| assert!(editor.file_available()));
            app.update(Editor::open_export_dialog);
            app.read(|editor| {
                assert!(
                    editor.document.export.form.is_some(),
                    "export remains reachable immediately after the individual gesture"
                );
                assert_eq!(editor.document.state.project, before);
            });
        });
    });
}
