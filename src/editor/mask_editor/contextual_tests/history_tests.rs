use super::*;

#[test]
fn history_retains_refinement_and_release_applies_only_mask_geometry_to_restored_state() {
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
            let id = start(&mut editor);
            let mask = editor.document.visit.masks.gesture().unwrap().mask.unwrap();
            editor.restore_history(history::Direction::Undo, &UiContext::render_only());
            let restored = editor.document.state.project.clone();
            let mut expected = restored.clone();
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
                .mask = mask;
            assert!(editor.document.visit.masks.editing());
            assert!(editor.mask_preview_source().is_some());
            editor.finish_mask(true);
            assert!(!editor.document.visit.masks.editing());
            assert_eq!(editor.document.state.project, expected);
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Adjustment layers")
            );
            editor.restore_history(history::Direction::Undo, &UiContext::render_only());
            assert_eq!(editor.document.state.project, restored);
        }
    });
}
