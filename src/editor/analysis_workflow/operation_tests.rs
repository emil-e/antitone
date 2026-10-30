use super::tests::{LIMIT, fixture};
use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{Preparation, history::Direction, source_loading},
};

fn open(app: &mut TestApplication<'_, Editor>) {
    app.enter(|editor, cx| {
        // Program shared geometry only. Its producer/publication and the Analyze
        // continuation run normally; sampling preparation uses real RAW inputs.
        let (reply, receive) = oneshot::channel();
        editor.document.spatial_reply = Some(receive);
        let metadata = editor.document.capture().ready_spatial_metadata();
        editor.request_analysis(cx);
        reply.send(Ok(Some(metadata))).ok().unwrap();
    });
    app.run_until(LIMIT, |editor| {
        editor.document.analysis_review().is_some() && !editor.busy
    });
}

#[test]
fn confirmation_freezes_reviewed_areas_and_keeps_numerical_capture_out_of_the_review() {
    let sources = source_loading::SourceLoader::new();
    let editor = fixture(sources.clone());
    let before = editor.document.project.clone();
    with_application(editor, |app| {
        open(app);
        let (reviewed, mut reply) = app.enter(|editor, cx| {
            editor.stash_review_region();
            let reviewed = editor.document.analysis_review().unwrap().approved.clone();
            let reply = testing::fitting(editor);
            editor.confirm_analysis_review(cx);
            assert!(editor.analysis_fitting(), "{}", editor.status);
            // Deliberately bypass disabled controls before the action resumes.
            // Neither a changed draft nor its presentation snapshot is fit input.
            let review = editor.document.analysis_review_mut().unwrap();
            review.approved.entries[0].areas.picture = None;
            review.document.project.set_gamma(0.1);
            (reviewed, reply)
        });
        let request = testing::request(app, &mut reply);
        let captured = request.areas.unwrap();
        assert_eq!(captured.entries, reviewed.entries);
        assert_eq!(captured.files, reviewed.files);
        assert_eq!(request.document.project, before);
        reply
            .result
            .send(Err("numerical probe completed".into()))
            .ok()
            .unwrap();
        app.run_until(LIMIT, |editor| {
            editor.document.analysis_operation().is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert!(editor.history_label(Direction::Undo).is_none());
            assert!(editor.document.analysis_review().is_none());
        });
    });
    sources.close_and_wait().unwrap();
}

#[test]
fn cancelling_the_waiting_review_allows_an_immediate_new_action_and_one_complete_accepted_edit() {
    let sources = source_loading::SourceLoader::new();
    let editor = fixture(sources.clone());
    let before = editor.document.project.clone();
    with_application(editor, |app| {
        open(app);
        app.enter(|editor, cx| {
            let stopped = editor
                .document
                .analysis_operation()
                .unwrap()
                .cancelled
                .clone();
            editor.finish_pick(cx);
            assert!(stopped.is_cancelled());
            assert!(editor.document.analysis_operation().is_none());
            assert_eq!(editor.document.project, before);
            assert!(editor.history_label(Direction::Undo).is_none());
            // Start before the retired waiter gets another executor turn.
            editor.request_analysis(cx);
        });
        app.run_until(LIMIT, |editor| {
            editor.document.analysis_review().is_some() && !editor.busy
        });
        let reviewed = app.enter(|editor, cx| {
            let reviewed = editor
                .document
                .analysis_review()
                .unwrap()
                .approved
                .entries
                .clone();
            assert!(
                !editor
                    .document
                    .analysis_operation()
                    .unwrap()
                    .cancelled
                    .is_cancelled()
            );
            editor.confirm_analysis_review(cx);
            reviewed
        });
        app.run_until(LIMIT, |editor| {
            editor.document.analysis_operation().is_none() && !editor.busy
        });
        app.read(|editor| {
            assert!(editor.document.analysis_review().is_none());
            assert!(editor.tasks.view().is_none());
            assert_eq!(editor.document.project.analysis_regions, reviewed);
            assert_ne!(editor.document.project.dmin(), before.dmin());
            assert_eq!(editor.history_label(Direction::Undo), Some("Analyze roll"));
        });
        app.enter(|editor, cx| editor.restore_history(Direction::Undo, cx));
        app.run_until(LIMIT, |editor| !editor.busy);
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert!(editor.history_label(Direction::Undo).is_none());
        });
    });
    sources.close_and_wait().unwrap();
}

#[test]
fn incomplete_confirmation_keeps_the_review_available_for_a_valid_retry() {
    let sources = source_loading::SourceLoader::new();
    let editor = fixture(sources.clone());
    let before = editor.document.project.clone();
    with_application(editor, |app| {
        open(app);
        let reviewed = app.enter(|editor, cx| {
            let reviewed = editor
                .document
                .analysis_review()
                .unwrap()
                .approved
                .entries
                .clone();
            editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries[0]
                .areas
                .picture = None;
            editor.confirm_analysis_review(cx);
            assert!(!editor.analysis_fitting());
            assert!(editor.document.analysis_review().is_some() && !editor.busy);
            assert_eq!(editor.document.project, before);
            assert!(editor.history_label(Direction::Undo).is_none());
            editor
                .document
                .analysis_review_mut()
                .unwrap()
                .approved
                .entries = reviewed.clone();
            editor.confirm_analysis_review(cx);
            assert!(editor.analysis_fitting(), "{}", editor.status);
            reviewed
        });
        app.run_until(LIMIT, |editor| {
            editor.document.analysis_operation().is_none() && !editor.busy
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.analysis_regions, reviewed);
            assert_ne!(editor.document.project.dmin(), before.dmin());
            assert!(editor.document.analysis_review().is_none());
            assert!(editor.history_label(Direction::Undo).is_some());
        });
    });
    sources.close_and_wait().unwrap();
}

#[test]
fn document_replacement_retires_a_waiting_review_without_restoring_its_retired_photo() {
    let sources = source_loading::SourceLoader::new();
    let editor = fixture(sources.clone());
    with_application(editor, |app| {
        open(app);
        app.enter(|editor, cx| {
            let stopped = editor
                .document
                .analysis_operation()
                .unwrap()
                .cancelled
                .clone();
            editor.start_document(Preparation::New, cx);
            assert!(stopped.is_cancelled());
            assert!(editor.document.analysis_operation().is_none());
            assert!(editor.document.analysis_review().is_none());
        });
        app.run_until(LIMIT, |editor| !editor.busy);
        app.read(|editor| {
            assert!(editor.document.project.order.is_empty());
            assert!(editor.document.visit.current_source.is_none());
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(editor.history_label(Direction::Undo).is_none());
        });
    });
    sources.close_and_wait().unwrap();
}
