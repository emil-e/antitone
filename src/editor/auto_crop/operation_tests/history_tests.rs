use super::*;

#[test]
fn history_retains_batch_proposals_and_apply_validates_the_restored_crops() {
    for changed_crop in [false, true] {
        let (mut editor, _files) = super::super::tests::fixture();
        let source = editor.document.project.order[0];
        let mut edit = editor.edit_project("Before review").unwrap();
        edit.update(|project| {
            if changed_crop {
                project.source_mut(source).unwrap().crop =
                    Some(Crop::from_region([200, 150, 100, 90].into()));
            } else {
                project.output.grain.amount = 0.12;
            }
        });
        editor.commit_project(edit).unwrap();
        with_application(editor, |app| {
            request(app);
            let review = app.read(|editor| editor.document.auto_crop.current().unwrap());
            let proposals: Vec<_> = review
                .borrow()
                .as_ref()
                .unwrap()
                .entries
                .iter()
                .map(|entry| entry.draft)
                .collect();
            app.enter(|editor, cx| editor.restore_history(history::Direction::Undo, cx));
            let restored = app.read(|editor| {
                assert!(Rc::ptr_eq(
                    &review,
                    &editor.document.auto_crop.current().unwrap()
                ));
                assert_eq!(editor.auto_crop_valid(), !changed_crop);
                editor.document.state.project.clone()
            });
            assert_eq!(
                review
                    .borrow()
                    .as_ref()
                    .unwrap()
                    .entries
                    .iter()
                    .map(|entry| entry.draft)
                    .collect::<Vec<_>>(),
                proposals
            );
            drop(review);
            app.update(|editor| editor.finish_auto_crop(true));
            app.read(|editor| {
                if changed_crop {
                    assert_eq!(editor.document.state.project, restored);
                    assert!(editor.document.auto_crop.reviewing());
                    assert_eq!(
                        editor.history_label(history::Direction::Redo),
                        Some("Before review")
                    );
                } else {
                    assert!(!editor.document.auto_crop.active(), "{}", editor.status);
                    assert_eq!(editor.document.project.output, restored.output);
                    assert_eq!(
                        editor.history_label(history::Direction::Undo),
                        Some("Auto-crop")
                    );
                }
            });
        });
    }
}
