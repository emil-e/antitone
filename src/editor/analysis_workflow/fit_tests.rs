use super::tests::{LIMIT, await_native, fixture, start};
use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{Document, Preparation, development, history::Direction},
};
use std::{sync::mpsc, time::Duration};

fn review(app: &mut TestApplication<'_, Editor>) {
    start(app);
    app.run_until(LIMIT, |e| e.document.analysis_preparing.is_none());
    app.read(|e| assert!(e.document.analysis_review.is_some(), "{}", e.status));
}

fn confirm(app: &TestApplication<'_, Editor>) {
    app.enter(|e, access| {
        e.confirm_analysis_review(access);
        assert!(e.document.analysis_pending.is_some(), "{}", e.status);
    });
}

fn gated() -> (
    development::Scheduler,
    Arc<AtomicBool>,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let enabled = Arc::new(AtomicBool::new(false));
    let gate = enabled.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin)
            && gate.swap(false, Ordering::Relaxed)
        {
            entered.send(()).unwrap();
            resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
        }
        Ok(())
    });
    (scheduler, enabled, starts, release)
}

#[test]
fn reviewed_fit_returns_across_photo_visits_without_paint_and_undoes_support_and_grade_together() {
    let scheduler = development::Scheduler::new();
    let mut editor = fixture(scheduler.clone());
    let first = editor.current_source.unwrap();
    let other = editor.document.project.order[1];
    editor.select_roll_photo(
        1,
        egui::Modifiers::COMMAND,
        &crate::application::UiContext::render_only(),
    );
    editor.tab = 2;
    editor.scopes.visible = true;
    editor.viewport.zoom = Some(0.5);
    let viewport = editor.viewport;
    let before = editor.document.project.clone();
    with_application(editor, |app| {
        review(app);
        app.enter(|e, access| {
            e.browse_analysis_review(other, access);
        });
        app.run_until(LIMIT, |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.current_source, Some(other));
            assert_eq!(
                e.document.pick_return.as_ref().unwrap().path,
                e.document.source_path(first).unwrap()
            );
            assert_eq!(e.document.project, before);
        });
        confirm(app);
        app.run_until(LIMIT, |e| e.document.analysis_pending.is_none() && !e.busy);
        app.read(|e| {
            assert!(e.status.starts_with("Analyzed 2"), "{}", e.status);
            assert_eq!(e.current_source, Some(first));
            assert!(e.document.visit.ready.is_some());
            assert!(e.document.analysis_review.is_none() && e.document.pick_return.is_none());
            assert_eq!(e.viewport, viewport);
            assert_eq!(e.tab, 2);
            assert!(e.scopes.visible);
            assert!(e.roll_selection.contains(first) && e.roll_selection.contains(other));
            assert_eq!(e.document.project.analysis_regions.len(), 2);
            assert_ne!(e.document.project.dmin(), before.dmin());
            assert!(e.history_label(Direction::Undo).is_some());
        });
        app.enter(|e, access| {
            e.restore_history(Direction::Undo, access);
        });
        app.run_until(LIMIT, |e| !e.busy);
        app.read(|e| assert_eq!(e.document.project, before));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn fresh_negative_analysis_measures_both_anchors_and_undo_restores_the_uncalibrated_roll() {
    let scheduler = development::Scheduler::new();
    let mut editor = fixture(scheduler.clone());
    let crate::processing::InputSettings::Negative { dmin, dmax, .. } = &mut editor.input else {
        panic!("authored scan roll must use Negative");
    };
    *dmin = None;
    *dmax = None;
    editor.sync_document();
    let before = editor.document.project.clone();
    assert!(!before.input.ready());
    assert!(before.input.calibration().is_none());
    with_application(editor, |app| {
        review(app);
        confirm(app);
        app.run_until(LIMIT, |e| e.document.analysis_pending.is_none() && !e.busy);
        app.read(|e| {
            assert!(e.status.starts_with("Analyzed 2"), "{}", e.status);
            let calibration = e
                .document
                .project
                .input
                .calibration()
                .unwrap()
                .validate()
                .unwrap();
            assert!(
                (calibration.dmin - glam::DVec3::splat(3000. / 4031.))
                    .abs()
                    .max_element()
                    <= 2. / 65535.
            );
            assert!(e.input.ready());
            assert_eq!(e.document.project.analysis_regions.len(), 2);
        });
        app.enter(|e, access| e.restore_history(Direction::Undo, access));
        app.run_until(LIMIT, |e| !e.busy);
        app.read(|e| assert_eq!(e.document.project, before));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn native_fit_demand_cancellation_preserves_accepted_data_and_cleanup_admission() {
    let (scheduler, enabled, starts, release) = gated();
    let editor = fixture(scheduler.clone());
    let before = editor.document.project.clone();
    let jobs = editor.workflow_jobs.clone();
    with_application(editor, |app| {
        review(app);
        enabled.store(true, Ordering::Relaxed);
        confirm(app);
        await_native(app, &starts);
        app.update(Editor::cancel_analysis);
        app.run_until(LIMIT, |e| e.document.analysis_pending.is_none() && !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.document.analysis_review.is_none());
            assert!(e.tasks.view().is_none());
            assert!(e.history_label(Direction::Undo).is_none());
        });
        assert_eq!(scheduler.waiter_count(), 0);
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                jobs.run(move || entered.send(()).unwrap()).await.unwrap();
            })
            .unwrap();
        app.pump();
        assert!(matches!(
            observed.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        app.run_until(LIMIT, |_| observed.try_recv().is_ok());
        drop(probe);
        app.read(|e| assert_eq!(e.document.project, before));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn fitting_document_and_application_retirement_cannot_publish_late_results() {
    for close_application in [false, true] {
        let (scheduler, enabled, starts, release) = gated();
        let editor = fixture(scheduler.clone());
        let jobs = editor.workflow_jobs.clone();
        with_application(editor, |app| {
            review(app);
            enabled.store(true, Ordering::Relaxed);
            confirm(app);
            await_native(app, &starts);
            let cancelled = app.read(|e| {
                e.document
                    .analysis_pending
                    .as_ref()
                    .unwrap()
                    .cancelled
                    .clone()
            });
            if close_application {
                app.close_execution();
            } else {
                app.update(|e| e.document = Document::default().into());
                app.pump();
                app.read(|e| {
                    assert!(
                        e.document.analysis_review.is_none() && e.document.pick_return.is_none()
                    )
                });
            }
            assert!(cancelled.is_cancelled());
            assert_eq!(scheduler.waiter_count(), 0);
            assert!(!jobs.idle());
            release.send(()).unwrap();
        });
        scheduler.close_and_wait().unwrap();
        assert!(jobs.idle());
    }
}

#[test]
fn live_edits_during_fit_discard_the_entire_candidate_without_installing_reviewed_areas() {
    let (scheduler, enabled, starts, release) = gated();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        review(app);
        enabled.store(true, Ordering::Relaxed);
        confirm(app);
        await_native(app, &starts);
        app.update(|e| {
            e.set_gamma(0.8);
            e.sync_document();
        });
        let edited = app.read(|e| e.document.project.clone());
        release.send(()).unwrap();
        app.run_until(LIMIT, |e| e.document.analysis_pending.is_none() && !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, edited);
            assert!(e.document.project.analysis_regions.is_empty());
            assert!(e.document.analysis_review.is_none());
            assert!(e.status.contains("discarded"), "{}", e.status);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn new_document_supersedes_a_queued_fit_without_consuming_its_areas_or_history() {
    let scheduler = development::Scheduler::new();
    let editor = fixture(scheduler.clone());
    let jobs = editor.workflow_jobs.clone();
    with_application(editor, |app| {
        review(app);
        let admission = jobs.hold();
        confirm(app);
        app.pump();
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.pump();
        app.read(|e| {
            assert!(e.document.analysis_pending.is_none() && e.document.analysis_review.is_none());
            assert!(e.document.pick_return.is_none());
            assert!(e.document_pending.is_some());
        });
        drop(admission);
        app.run_until(LIMIT, |e| !e.busy);
        app.read(|e| {
            assert!(e.document.project.order.is_empty());
            assert!(e.document.project.analysis_regions.is_empty());
            assert!(e.history_label(Direction::Undo).is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}
