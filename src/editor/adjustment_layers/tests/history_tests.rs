use super::*;

#[test]
fn undo_during_layer_drag_only_blocks_drop_if_the_destination_disappears() {
    pollster::block_on(async {
        for added_destination in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            editor.reveal_adjustment_layers();
            let order = ids(&editor);
            let source = editor.document.visit.current_source.unwrap();
            let mut edit = editor.edit_photo("Before drag").unwrap();
            let mut destination = order[1];
            edit.update(|project| {
                if added_destination {
                    let layers = project
                        .source_mut(source)
                        .unwrap()
                        .adjustment_layers
                        .as_mut()
                        .unwrap();
                    let mut adjustment = layers.layers[0].adjustment;
                    adjustment.photo = AdjustmentLayerSettings::default();
                    destination = layers.add("New destination".into(), adjustment).unwrap();
                } else {
                    project.roll.exposure = 0.75;
                }
            });
            editor.commit_project(edit).unwrap();
            let cx = UiContext::render_only();
            let key = begin_reorder(&mut editor, order[0]);
            assert!(editor.document.visit.adjustment_layers.reorder.0.is_some());
            editor.restore_history(history::Direction::Undo, &cx);
            let restored = editor.document.state.project.clone();
            assert!(editor.document.visit.adjustment_layers.reorder.0.is_some());
            apply_drop(&mut editor, key, DropPosition::Above(destination));
            if added_destination {
                assert!(
                    editor
                        .status
                        .contains("destination adjustment layer no longer exists")
                );
                assert_eq!(editor.document.state.project, restored);
                assert_eq!(ids(&editor), order);
                assert_eq!(
                    editor.history_label(history::Direction::Redo),
                    Some("Before drag")
                );
            } else {
                assert_eq!(ids(&editor), vec![order[1], order[0]]);
                assert_eq!(editor.document.project.roll, restored.roll);
                assert_eq!(
                    editor.history_label(history::Direction::Undo),
                    Some("Adjustment layers")
                );
            }
        }
    });
}
