use super::*;

#[test]
fn history_retains_refinement_and_release_checks_the_restored_source() {
    pollster::block_on(async {
        for changed_source in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            let source = editor.document.visit.current_source.unwrap();
            let mut edit = editor.edit_photo("Before gesture").unwrap();
            edit.update(|project| {
                if changed_source {
                    project
                        .source_mut(source)
                        .unwrap()
                        .adjustment_layers
                        .as_mut()
                        .unwrap()
                        .layers[0]
                        .adjustment
                        .photo
                        .exposure = 0.75;
                } else {
                    project.output.grain.amount = 0.12;
                }
            });
            editor.commit_project(edit).unwrap();
            start(&mut editor);
            editor.restore_history(history::Direction::Undo, &UiContext::render_only());
            let restored = editor.document.state.project.clone();
            assert!(editor.document.visit.masks.editing());
            assert_eq!(editor.mask_preview_source().is_none(), changed_source);
            editor.finish_mask(true);
            assert!(!editor.document.visit.masks.editing());
            if changed_source {
                assert_eq!(editor.document.state.project, restored);
                assert_eq!(
                    editor.history_label(history::Direction::Redo),
                    Some("Before gesture")
                );
            } else {
                assert_ne!(editor.document.state.project, restored);
                assert_eq!(editor.document.project.output, restored.output);
                assert_eq!(
                    editor.history_label(history::Direction::Undo),
                    Some("Adjustment layers")
                );
            }
        }
    });
}
