use super::*;
use crate::application::test_support::{TestApplication, with_application};
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(5);

fn request(app: &mut TestApplication<'_, Editor>) {
    let send = app.enter(|editor, cx| {
        let (send, receive) = oneshot::channel();
        editor.document.spatial_reply = Some(receive);
        editor.review_crop_proposals(editor.document.project.order.clone(), "Whole roll", cx);
        assert!(editor.document.auto_crop_edit().is_none());
        send
    });
    send.send(Ok(Some(Default::default()))).unwrap();
    app.run_until(LIMIT, |editor| editor.document.auto_crop_edit().is_some());
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
        app.run_until(LIMIT, |editor| editor.document.auto_crop_edit().is_some());
        app.update(|editor| {
            assert!(editor.document.crop_edit().is_none());
            assert_eq!(editor.document.visit.current_source, Some(current));
            let review = editor.document.auto_crop_edit().unwrap();
            assert_eq!(review.entries.len(), 1);
            assert_eq!(review.entries[0].source, photo);
            assert!(editor.auto_crop_preview(current).is_none());
            assert!(editor.auto_crop_preview(photo).is_some());
            editor.finish_auto_crop(true);
        });
        app.run_until(LIMIT, |editor| {
            editor.document.auto_crop_operation().is_none()
        });
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
            editor.propose_photo_crop(other, cx);
            assert!(editor.document.spatial_preparing.is_none());
            assert!(
                editor
                    .document
                    .crop_edit()
                    .unwrap()
                    .auto_crop_task
                    .is_none()
            );
            assert_eq!(editor.document.crop_edit().unwrap().draft, before);
            editor.propose_photo_crop(current, cx);
            reply
        });
        reply.send(Ok(Some(Default::default()))).unwrap();
        app.run_until(LIMIT, |editor| {
            editor
                .document
                .crop_edit()
                .unwrap()
                .auto_crop_task
                .is_none()
        });
        app.read(|editor| {
            assert_ne!(editor.document.crop_edit().unwrap().draft, before);
            assert!(editor.document.auto_crop_edit().is_none());
            assert!(editor.document.auto_crop_operation().is_none());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), None);
        });
    });
}

#[test]
fn apply_snapshots_the_review_before_the_continuation_resumes() {
    let (editor, _files) = super::super::tests::fixture();
    let accepted = editor.document.state.project.clone();
    with_application(editor, |app| {
        request(app);
        let expected = app.update(|editor| {
            let expected: Vec<_> = editor
                .document
                .auto_crop_edit()
                .unwrap()
                .entries
                .iter()
                .map(|entry| (entry.source, entry.draft))
                .collect();
            editor.finish_auto_crop(true);
            editor.finish_auto_crop(true);
            assert_eq!(
                editor.document.state.project, accepted,
                "the decision has not resumed yet"
            );
            for entry in &mut editor.document.auto_crop_edit_mut().unwrap().entries {
                entry.draft = Some(Crop::from_region([200, 150, 100, 90].into()));
            }
            expected
        });
        app.run_until(LIMIT, |editor| {
            editor.document.auto_crop_operation().is_none()
        });
        app.read(|editor| {
            assert!(editor.document.auto_crop_edit().is_none());
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
            editor.finish_auto_crop(true);
            let id = *editor.document.project.order.last().unwrap();
            editor.document.project.source_mut(id).unwrap().crop =
                Some(Crop::from_region([200, 150, 100, 90].into()));
            (id, editor.document.state.project.clone())
        });
        app.run_until(LIMIT, |editor| {
            editor
                .document
                .auto_crop_operation()
                .is_some_and(|op| op.apply.is_some())
        });
        app.update(|editor| {
            assert_eq!(editor.document.state.project, changed);
            assert!(editor.document.auto_crop_edit().is_some());
            assert_eq!(editor.history_label(history::Direction::Undo), None);
            editor.document.project.source_mut(id).unwrap().crop = None;
            assert!(editor.auto_crop_valid());
            editor.finish_auto_crop(true);
        });
        app.run_until(LIMIT, |editor| {
            editor.document.auto_crop_operation().is_none()
        });
        app.read(|editor| {
            assert!(editor.document.auto_crop_edit().is_none());
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
            assert!(editor.document.auto_crop_operation().is_none());
            assert!(editor.document.auto_crop_edit().is_none());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), None);
        });
        request(app);
        app.update(|editor| editor.finish_auto_crop(true));
        app.run_until(LIMIT, |editor| {
            editor.document.auto_crop_operation().is_none()
        });
        app.read(|editor| {
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Auto-crop")
            )
        });
    });
}
