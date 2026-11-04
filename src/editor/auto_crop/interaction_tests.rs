use super::*;
use crate::application::test_support::with_application;
use futures_channel::oneshot;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(5);

#[test]
fn retiring_batch_preparation_releases_its_owner_without_retiring_shared_work_or_its_successor() {
    for polled in [false, true] {
        for failed in [false, true] {
            let (editor, _files) = super::tests::fixture();
            let accepted = editor.document.state.project.clone();
            with_application(editor, |app| {
                let (reply, old) = app.enter(|editor, cx| {
                    let (reply, receive) = oneshot::channel();
                    editor.document.spatial_reply = Some(receive);
                    editor.review_crop_proposals(
                        editor.document.project.order.clone(),
                        "Whole roll",
                        cx,
                    );
                    assert!(editor.current_tool().unwrap().is::<AutoCropTool>());
                    let state = editor.document.auto_crop.current().unwrap();
                    (reply, Rc::downgrade(&state))
                });
                if polled {
                    app.pump();
                }
                let successor = app.update(|editor| {
                    editor.begin_crop();
                    let state = editor.document.crop_state().unwrap();
                    state.borrow_mut().draft.size = [17, 19];
                    state
                });
                app.pump();
                assert!(
                    old.upgrade().is_none(),
                    "the preparation continuation must not keep its tool observation alive"
                );
                app.read(|editor| {
                    assert!(
                        editor.document.spatial_preparing.is_some(),
                        "shared work has an independent owner"
                    );
                    assert!(!editor.document.auto_crop.active());
                });
                reply
                    .send(if failed {
                        Err("held preparation failure".into())
                    } else {
                        Ok(Some(Default::default()))
                    })
                    .unwrap();
                app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
                app.pump();
                app.read(|editor| {
                    assert!(
                        editor.cropping(),
                        "a retired batch cannot finish its successor"
                    );
                    assert!(!editor.document.auto_crop.active());
                    assert_eq!(editor.document.state.project, accepted);
                });
                assert_eq!(successor.borrow().draft.size, [17, 19]);

                let reply = app.enter(|editor, cx| {
                    let (reply, receive) = oneshot::channel();
                    editor.document.spatial_reply = Some(receive);
                    editor.review_crop_proposals(
                        editor.document.project.order.clone(),
                        "Whole roll",
                        cx,
                    );
                    reply
                });
                reply.send(Ok(Some(Default::default()))).unwrap();
                app.run_until(LIMIT, |editor| editor.document.auto_crop.reviewing());
                app.update(|editor| editor.finish_auto_crop(true));
                app.read(|editor| {
                    assert!(!editor.document.auto_crop.active());
                    assert!(
                        editor
                            .document
                            .project
                            .sources
                            .iter()
                            .all(|source| source.crop.is_some())
                    );
                    assert_eq!(
                        editor.history_label(history::Direction::Undo),
                        Some("Auto-crop")
                    );
                });
            });
        }
    }
}

#[test]
fn replacing_the_document_retires_a_preparing_batch_and_its_publication_rights() {
    for polled in [false, true] {
        let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
        assert!(
            editor.source_services.is_some(),
            "the replacement must have real preparation services"
        );
        with_application(editor, |app| {
            let (reply, state) = app.enter(|editor, cx| {
                let (reply, receive) = oneshot::channel();
                editor.document.spatial_reply = Some(receive);
                editor.review_crop_proposals(
                    editor.document.project.order.clone(),
                    "Whole roll",
                    cx,
                );
                let state = editor.document.auto_crop.current().unwrap();
                (reply, Rc::downgrade(&state))
            });
            if polled {
                app.pump();
            }
            app.enter(|editor, cx| editor.start_document(Preparation::New, cx));
            app.run_until(LIMIT, |editor| !editor.busy);
            assert!(state.upgrade().is_none());
            app.pump();
            assert!(reply.send(Ok(Some(Default::default()))).is_err());
            app.read(|editor| {
                assert!(!editor.document.auto_crop.active());
                assert!(editor.document.spatial_preparing.is_none());
                assert!(editor.document.project.order.is_empty());
                assert_eq!(editor.history_label(history::Direction::Undo), None);
            });
        });
    }
}

#[test]
fn captured_loaded_source_keys_accept_locator_aliases_but_reject_foreign_inputs() {
    for foreign in [false, true] {
        let (mut editor, directory) = super::tests::fixture();
        let id = editor.document.visit.current_source.unwrap();
        let path = editor.document.source_path(id).unwrap();
        let alias = directory.path().join("alias");
        std::fs::create_dir(&alias).unwrap();
        editor.document.visit.canvas.ready.as_mut().unwrap().path =
            alias.join("..").join(path.file_name().unwrap());
        std::fs::remove_dir(&alias).unwrap();
        if foreign {
            editor.document.visit.canvas.ready.as_mut().unwrap().file =
                thumbnails::FileKey::unversioned(&directory.path().join("foreign.ARW"));
        }
        let before = editor.document.state.project.clone();
        editor.begin_auto_crop_review(vec![id], "Entry selection");
        if foreign {
            assert!(!editor.auto_crop_valid());
            assert!(editor.auto_crop_preview(id).is_none());
            assert!(
                !editor
                    .current_tool()
                    .unwrap()
                    .presentation()
                    .primary
                    .unwrap()
                    .enabled
            );
            editor.finish_auto_crop(true);
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.history_label(history::Direction::Undo), None);
        } else {
            assert!(
                editor.auto_crop_valid(),
                "captured source identity, not a redundant live locator lookup, qualifies the lease"
            );
            assert!(editor.auto_crop_preview(id).is_some());
            editor.finish_auto_crop(true);
            assert!(editor.document.project.source(id).unwrap().crop.is_some());
            assert!(!editor.document.auto_crop.active());
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Auto-crop")
            );
        }
    }
}
