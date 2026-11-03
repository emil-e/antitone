use super::*;
use crate::application::test_support::{TestApplication, with_application};
use crate::editor::crop_editor::tests::request_auto_crop;
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(5);

#[test]
fn photo_proposal_state_is_not_redirected_to_supersession_and_releases_without_cycles() {
    for poll_before_retirement in [false, true] {
        let (editor, _files) = super::super::tests::fixture();
        let accepted = editor.document.state.project.clone();
        with_application(editor, |app| {
            let (reply, old) = app.enter(|editor, cx| {
                editor.begin_crop();
                let state = editor.document.crop_state().unwrap();
                let (reply, receive) = oneshot::channel();
                editor.document.spatial_reply = Some(receive);
                request_auto_crop(editor.document.visit.current_source.unwrap(), editor, cx);
                assert!(editor.document.crop_auto_crop_pending());
                (reply, state)
            });
            if poll_before_retirement {
                app.pump();
            }
            let new = app.update(|editor| {
                editor.end_tool();
                editor.begin_crop();
                let state = editor.document.crop_state().unwrap();
                state.borrow_mut().draft.size = [17, 19];
                state
            });
            app.pump();
            app.read(|editor| assert!(editor.document.spatial_preparing.is_some()));
            assert_eq!(
                std::rc::Rc::strong_count(&old),
                1,
                "retired task releases UI state while independent preparation is still waiting"
            );
            assert!(!std::rc::Rc::ptr_eq(&old, &new));
            old.borrow_mut().draft.size = [23, 29];
            let old_draft = old.borrow().draft;
            let new_draft = new.borrow().draft;
            reply.send(Ok(Some(Default::default()))).unwrap();
            app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
            app.pump();
            assert_eq!(
                old.borrow().draft,
                old_draft,
                "retired continuation cannot publish"
            );
            assert_eq!(
                new.borrow().draft,
                new_draft,
                "replacement is a different state target"
            );
            app.read(|editor| assert_eq!(editor.document.state.project, accepted));
            let old_weak = std::rc::Rc::downgrade(&old);
            drop(old);
            app.run_until(LIMIT, |_| old_weak.upgrade().is_none());

            // Positive control: the successor can request and accept its own proposal.
            let reply = app.enter(|editor, cx| {
                let (reply, receive) = oneshot::channel();
                editor.document.spatial_reply = Some(receive);
                request_auto_crop(editor.document.visit.current_source.unwrap(), editor, cx);
                assert!(editor.document.crop_auto_crop_pending());
                reply
            });
            reply.send(Ok(Some(Default::default()))).unwrap();
            app.run_until(LIMIT, |editor| !editor.document.crop_auto_crop_pending());
            assert_ne!(new.borrow().draft, new_draft);
            app.read(|editor| assert_eq!(editor.document.state.project, accepted));
            let new_weak = std::rc::Rc::downgrade(&new);
            app.update(Editor::end_tool);
            drop(new);
            app.run_until(LIMIT, |_| new_weak.upgrade().is_none());
        });
    }
}

fn request(app: &mut TestApplication<'_, Editor>) {
    let send = app.enter(|editor, cx| {
        let (send, receive) = oneshot::channel();
        editor.document.spatial_reply = Some(receive);
        editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx);
        assert!(!editor.document.auto_crop.reviewing());
        send
    });
    send.send(Ok(Some(Default::default()))).unwrap();
    app.run_until(LIMIT, |editor| editor.document.auto_crop.reviewing());
    app.read(|editor| assert!(editor.auto_crop_valid(), "{}", editor.status));
}

#[test]
fn a_one_photo_batch_reviews_the_explicit_photo_instead_of_seeding_manual_crop() {
    let (mut editor, _files) = super::super::tests::fixture();
    let current = editor.document.visit.current_source.unwrap();
    let photo = editor.document.project.order[1];
    assert_ne!(photo, current);
    let order = editor.document.project.order.clone();
    editor
        .document
        .roll_selection
        .reconcile(&order, Some(current));
    assert!(editor.document.roll_selection.contains(current));
    assert!(!editor.document.roll_selection.contains(photo));
    with_application(editor, |app| {
        let reply = app.enter(|editor, cx| {
            let (reply, receive) = oneshot::channel();
            editor.document.spatial_reply = Some(receive);
            editor.review_crop_proposals(vec![photo], "Entry selection", cx);
            reply
        });
        reply.send(Ok(Some(Default::default()))).unwrap();
        app.run_until(LIMIT, |editor| editor.document.auto_crop.reviewing());
        app.update(|editor| {
            assert!(editor.document.crop_edit().is_none());
            assert_eq!(editor.document.visit.current_source, Some(current));
            let review = editor.document.auto_crop.snapshot().unwrap();
            assert_eq!(review.entries.len(), 1);
            assert_eq!(review.entries[0].source, photo);
            assert!(editor.auto_crop_preview(current).is_none());
            assert!(editor.auto_crop_preview(photo).is_some());
            editor.finish_auto_crop(true);
        });
        app.run_until(LIMIT, |editor| !editor.document.auto_crop.active());
        app.read(|editor| {
            assert!(
                editor
                    .document
                    .project
                    .source(photo)
                    .unwrap()
                    .crop
                    .is_some()
            );
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .filter(|source| source.id != photo)
                    .all(|source| source.crop.is_none())
            );
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Auto-crop")
            );
            assert!(editor.document.crop_edit().is_none());
        });
    });
}

#[test]
fn a_photo_proposal_only_seeds_its_own_existing_crop_without_accepting_an_edit() {
    let (mut editor, _files) = super::super::tests::fixture();
    editor.begin_crop();
    let current = editor.document.visit.current_source.unwrap();
    let other = editor.document.project.order[1];
    let before = editor.document.crop_edit().unwrap().draft;
    let accepted = editor.document.state.project.clone();
    with_application(editor, |app| {
        let reply = app.enter(|editor, cx| {
            let (reply, receive) = oneshot::channel();
            editor.document.spatial_reply = Some(receive);
            request_auto_crop(other, editor, cx);
            assert!(editor.document.spatial_preparing.is_none());
            assert!(!editor.document.crop_auto_crop_pending());
            assert_eq!(editor.document.crop_edit().unwrap().draft, before);
            request_auto_crop(current, editor, cx);
            reply
        });
        reply.send(Ok(Some(Default::default()))).unwrap();
        app.run_until(LIMIT, |editor| !editor.document.crop_auto_crop_pending());
        app.read(|editor| {
            assert_ne!(editor.document.crop_edit().unwrap().draft, before);
            assert!(!editor.document.auto_crop.reviewing());
            assert!(!editor.document.auto_crop.active());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), None);
        });
    });
}

#[test]
fn apply_accepts_the_current_batch_atomically_and_ends_the_interaction() {
    let (editor, _files) = super::super::tests::fixture();
    let accepted = editor.document.state.project.clone();
    with_application(editor, |app| {
        request(app);
        let expected = app.update(|editor| {
            let expected: Vec<_> = editor
                .document
                .auto_crop
                .snapshot()
                .unwrap()
                .entries
                .iter()
                .map(|entry| (entry.source, entry.draft))
                .collect();
            assert_eq!(
                editor.document.state.project, accepted,
                "review has not accepted a crop"
            );
            editor.finish_auto_crop(true);
            assert!(
                !editor.document.auto_crop.active(),
                "acceptance and tool completion are one action"
            );
            editor.finish_auto_crop(true);
            expected
        });
        app.read(|editor| {
            assert!(!editor.document.auto_crop.reviewing());
            for (id, crop) in expected {
                assert_eq!(editor.document.project.source(id).unwrap().crop, crop);
            }
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Auto-crop")
            );
        });
    });
}

#[test]
fn rejected_apply_keeps_the_review_without_history_and_a_valid_retry_can_accept() {
    let (editor, _files) = super::super::tests::fixture();
    with_application(editor, |app| {
        request(app);
        let (id, changed) = app.update(|editor| {
            let id = *editor.document.project.order.last().unwrap();
            editor.document.project.source_mut(id).unwrap().crop =
                Some(Crop::from_region([200, 150, 100, 90].into()));
            let changed = editor.document.state.project.clone();
            editor.finish_auto_crop(true);
            (id, changed)
        });
        app.update(|editor| {
            assert_eq!(
                editor.document.state.project, changed,
                "reject the whole batch, not a valid prefix"
            );
            assert!(editor.document.auto_crop.reviewing());
            assert_eq!(editor.history_label(history::Direction::Undo), None);
            editor.document.project.source_mut(id).unwrap().crop = None;
            assert!(editor.auto_crop_valid());
            editor.finish_auto_crop(true);
        });
        app.read(|editor| {
            assert!(!editor.document.auto_crop.active());
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Auto-crop")
            );
            assert!(
                editor
                    .document
                    .project
                    .sources
                    .iter()
                    .all(|source| source.crop.is_some())
            );
        });
    });
}

#[test]
fn cancelling_the_waiting_review_retires_its_action_and_allows_another_complete_operation() {
    let (editor, _files) = super::super::tests::fixture();
    let accepted = editor.document.state.project.clone();
    with_application(editor, |app| {
        request(app);
        app.update(|editor| {
            editor.adjust_auto_crop_margin(1.5);
            editor.finish_auto_crop(false);
        });
        app.pump();
        app.read(|editor| {
            assert!(!editor.document.auto_crop.active());
            assert!(!editor.document.auto_crop.reviewing());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), None);
        });
        request(app);
        app.update(|editor| editor.finish_auto_crop(true));
        app.run_until(LIMIT, |editor| !editor.document.auto_crop.active());
        app.read(|editor| {
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Auto-crop")
            )
        });
    });
}
