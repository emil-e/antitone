use super::*;
use crate::{application::test_support::with_application, looks::spektrafilm};
use std::time::Duration;

fn accepted_bw(e: &mut Editor) {
    let selection = crate::looks::monochrome::Selection {
        negative: crate::looks::monochrome::Negative::from_id("kodak_tmax_100_d76_7m30s"),
        paper: Default::default(),
    };
    e.document.state.project.output.film_and_paper = Some(Arc::new(
        crate::looks::monochrome::Model::new(selection)
            .bake(3, &AtomicBool::new(false))
            .unwrap(),
    ));
    e.document.state.project.output.lut = Some(super::tests::asset(0.3));
}

#[test]
fn declining_a_switch_keeps_earlier_preparation_alive_even_if_it_completes_under_the_dialog() {
    let (e, scheduler) = super::tests::editor();
    with_application(e, |app| {
        let paper = crate::looks::monochrome::Paper::default();
        let activity = app.enter(|e, cx| {
            e.select_paper(Some(paper.into()), cx);
            let activity = e.document.look.as_ref().unwrap().activity.clone();
            e.select_negative(
                spektrafilm::Negative::from_id("kodak_portra_400").map(Into::into),
                cx,
            );
            assert!(e.dialogs.is_open());
            assert_eq!(e.document.look.as_ref().unwrap().activity.clone(), activity);
            activity
        });
        app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
        let accepted = app.read(|e| {
            assert!(e.dialogs.is_open());
            assert!(e.dialogs.is_open() && !e.interaction_blocked());
            assert_eq!(
                e.material_selection(),
                Some(MaterialSelection::Monochrome(paper.into()))
            );
            e.document.state.project.clone()
        });
        app.update(|e| e.dialogs.answer_for_test("Cancel"));
        app.pump();
        app.read(|e| {
            assert!(!e.dialogs.is_open());
            assert_eq!(e.document.state.project, accepted);
            assert!(e.activities.view().is_none(), "retired look {activity:?}");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn confirmed_switch_failure_or_worker_cancel_never_replaces_either_accepted_slot() {
    for cancel in [false, true] {
        let (mut e, scheduler) = if cancel {
            super::tests::editor()
        } else {
            super::tests::editor_with_limits(wgpu::Limits {
                max_storage_buffer_binding_size: 1024,
                ..Default::default()
            })
        };
        accepted_bw(&mut e);
        let before = e.document.state.project.clone();
        let hold = cancel.then(crate::background::testing::pause);
        with_application(e, |app| {
            app.enter(|e, cx| {
                e.select_negative(
                    spektrafilm::Negative::from_id("kodak_portra_400").map(Into::into),
                    cx,
                );
                assert!(e.dialogs.is_open());
                assert!(e.document.look.is_none());
                e.dialogs.answer_for_test("Switch materials");
                assert!(!e.dialogs.is_open());
                assert_eq!(e.document.state.project, before);
            });
            app.pump();
            if cancel {
                let activity = app.read(|e| e.document.look.as_ref().unwrap().activity.clone());
                app.update(|_| activity.request_cancel());
                app.pump();
                drop(hold);
            }
            app.run_until(Duration::from_secs(90), |e| e.document.look.is_none());
            app.read(|e| {
                assert_eq!(e.document.state.project, before);
                assert!(!e.interaction_blocked());
                assert_eq!(
                    e.history_label(crate::editor::history::Direction::Undo),
                    None
                );
                assert_eq!(e.dialogs.is_open(), !cancel);
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn document_replacement_retires_the_continuation_but_not_the_unanswered_dialog() {
    let (mut e, scheduler) = super::tests::editor();
    accepted_bw(&mut e);
    with_application(e, |app| {
        app.enter(|e, cx| {
            e.select_negative(
                spektrafilm::Negative::from_id("kodak_portra_400").map(Into::into),
                cx,
            );
            assert!(e.dialogs.is_open());
            e.start_document(Preparation::New, cx);
        });
        app.run_until(Duration::from_secs(30), |e| {
            e.document_pending.is_none() && !e.interaction_blocked()
        });
        app.read(|e| {
            assert!(e.document.dialog_action.is_none() && e.document.look.is_none());
            assert!(e.document.project.output.film_and_paper.is_none());
            assert!(e.dialogs.is_open());
        });
        app.update(|e| e.dialogs.answer_for_test("Switch materials"));
        app.pump();
        app.read(|e| {
            assert!(!e.dialogs.is_open() && e.document.look.is_none());
            assert!(e.document.project.output.film_and_paper.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn confirming_after_earlier_preparation_fails_keeps_the_notice_and_starts_the_captured_choice() {
    let (mut editor, scheduler) = super::tests::editor_with_limits(wgpu::Limits {
        max_storage_buffer_binding_size: 1024,
        ..Default::default()
    });
    let (prepared, ready) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = std::sync::Mutex::new(resume);
    editor.look.services.as_mut().unwrap().prepared = Some(Arc::new(move |valid, _stop| {
        prepared.send(valid).unwrap();
        resume
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(90))
            .unwrap();
    }));
    let before = editor.document.project.clone();
    let negative = spektrafilm::Negative::from_id("kodak_portra_400").unwrap();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.select_paper(Some(crate::looks::monochrome::Paper::default().into()), cx);
            editor.select_negative(Some(negative.into()), cx);
            assert!(editor.dialogs.is_open() && editor.document.look.is_some());
        });
        app.pump();
        assert!(!ready.recv_timeout(Duration::from_secs(90)).unwrap());
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(90), |editor| {
            editor.document.look.is_none()
        });
        app.read(|editor| {
            assert!(
                editor.dialogs.is_open() && editor.dialogs.message().is_none(),
                "the question remains in front of the worker's failure notice"
            );
            assert_eq!(editor.document.project, before);
        });
        app.update(|editor| editor.dialogs.answer_for_test("Switch materials"));
        app.pump();
        app.read(|editor| {
            let operation = editor
                .document
                .look
                .as_ref()
                .expect("confirmation must start its continuation");
            assert_eq!(
                operation.selection.unwrap().negative(),
                Some(negative.into())
            );
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                crate::editor::FailureCase::Look.title()
            );
        });
        assert!(!ready.recv_timeout(Duration::from_secs(90)).unwrap());
        app.update(|editor| editor.dialogs.answer_for_test("OK"));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(90), |editor| {
            editor.document.look.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                crate::editor::FailureCase::Look.title()
            );
            assert!(
                editor
                    .history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
        });
        app.update(|editor| editor.dialogs.answer_for_test("OK"));
        app.read(|editor| assert!(!editor.dialogs.is_open()));
    });
    scheduler.close_and_wait().unwrap();
}
