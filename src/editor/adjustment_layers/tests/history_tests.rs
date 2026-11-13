use super::*;

#[test]
fn history_during_layer_drag_leaves_drop_validation_with_the_captured_layers() {
    pollster::block_on(async {
        for changed_layers in [false, true] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            editor.reveal_adjustment_layers();
            let order = ids(&editor);
            let source = editor.document.visit.current_source.unwrap();
            let mut edit = editor.edit_photo("Before drag").unwrap();
            edit.update(|project| {
                if changed_layers {
                    project
                        .source_mut(source)
                        .unwrap()
                        .adjustment_layers
                        .as_mut()
                        .unwrap()
                        .layers[0]
                        .name = "Renamed".into();
                } else {
                    project.roll.exposure = 0.75;
                }
            });
            editor.commit_project(edit).unwrap();
            let cx = UiContext::render_only();
            editor.adjustment_layer_order_input(Some(order[0]), None, false, &cx);
            assert!(editor.document.visit.adjustment_layers.dragging());
            editor.restore_history(history::Direction::Undo, &cx);
            let restored = editor.document.state.project.clone();
            assert!(editor.document.visit.adjustment_layers.dragging());
            let key = OrderKey {
                loaded: editor.id,
                source,
                id: order[0],
            };
            editor.adjustment_layer_order_input(None, Some((key, 2)), true, &cx);
            if changed_layers {
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
