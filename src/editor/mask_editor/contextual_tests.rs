use super::*;

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
    let candidate = edit.candidate().unwrap().0;
    editor.validate_source_candidate(&candidate).unwrap();
    editor.document.visit.masks.gesture_mut().unwrap().preview = Some(candidate);
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
fn accepted_controls_survive_rejection_of_a_refinement_captured_before_them() {
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
        let history = editor.history_label(history::Direction::Undo);
        assert!(editor.mask_preview_source().is_none());
        editor.finish_mask(true);
        assert!(!editor.document.visit.masks.editing());
        assert_eq!(editor.document.state.project, graded);
        assert_eq!(editor.history_label(history::Direction::Undo), history);
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
                assert!(editor.document_pending.is_none());
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
fn file_and_export_admission_wait_for_the_gesture_not_for_a_mask_mode() {
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
                assert!(editor.document_pending.is_none());
                assert!(!editor.files.pending());
                assert!(editor.document.export.draft.is_none());
                assert_eq!(editor.document.state.project, before);
            });
            app.update(|editor| editor.finish_mask(false));
            app.read(|editor| assert!(editor.file_available()));
            app.update(Editor::open_export_dialog);
            app.read(|editor| {
                assert!(
                    editor.document.export.draft.is_some(),
                    "export remains reachable immediately after the individual gesture"
                );
                assert_eq!(editor.document.state.project, before);
            });
        });
    });
}
