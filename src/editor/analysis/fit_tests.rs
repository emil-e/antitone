use super::workflow_tests::{LIMIT, await_native, fixture, preparing, start};
use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{Document, Preparation, history::Direction, source_loading},
};
use std::sync::mpsc;

fn review(app: &mut TestApplication<'_, Editor>) {
    start(app);
    app.run_until(LIMIT, |e| !preparing(e));
    app.read(|e| assert!(e.document.analysis_review().is_some(), "{}", e.status));
}

fn confirm(app: &TestApplication<'_, Editor>) {
    app.enter(|e, access| {
        e.confirm_analysis_review(access);
        assert!(e.analysis_fitting(), "{}", e.status);
    });
}

fn gated() -> (
    source_loading::SourceLoader,
    Arc<AtomicBool>,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let enabled = Arc::new(AtomicBool::new(false));
    let gate = enabled.clone();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin)
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
fn reviewed_fit_keeps_the_browsed_photo_and_undoes_support_and_grade_together() {
    let scheduler = source_loading::SourceLoader::new();
    let mut editor = fixture(scheduler.clone());
    let first = editor.document.visit.current_source.unwrap();
    let other = editor.document.state.project.order[1];
    editor.select_roll_photo(
        1,
        egui::Modifiers::COMMAND,
        &crate::application::UiContext::render_only(),
    );
    editor.tab = 2;
    editor.scopes.visible = true;
    editor.preferences.scopes_visible = true;
    editor.document.visit.canvas.viewport.zoom = Some(0.5);
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        review(app);
        app.enter(|e, access| {
            e.select_roll_photo(1, egui::Modifiers::NONE, access);
        });
        app.run_until(LIMIT, |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.visit.current_source, Some(other));
            assert_eq!(e.document.state.project, before);
        });
        app.update(|e| e.select_active_photo_only());
        confirm(app);
        app.run_until(LIMIT, |e| !e.analysis_fitting() && !e.interaction_blocked());
        app.read(|e| {
            assert!(e.status.starts_with("Analyzed 2"), "{}", e.status);
            assert_eq!(e.document.visit.current_source, Some(other));
            assert!(e.document.visit.canvas.ready.is_some());
            assert!(e.document.analysis_review().is_none());
            assert_eq!(e.tab, 0);
            assert!(e.scopes.visible);
            assert!(
                !e.document.roll_selection.contains(first)
                    && e.document.roll_selection.contains(other)
            );
            assert_eq!(e.document.state.project.analysis_regions.len(), 2);
            assert_ne!(e.document.state.project.dmin(), before.dmin());
            assert!(e.history_label(Direction::Undo).is_some());
        });
        app.enter(|e, access| {
            e.restore_history(Direction::Undo, access);
        });
        app.run_until(LIMIT, |e| !e.interaction_blocked());
        app.read(|e| assert_eq!(e.document.state.project, before));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn cancelling_review_keeps_the_browsed_photo_and_its_layer_selection() {
    use crate::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings},
        editor::adjustment_layers::Target,
    };
    let scheduler = source_loading::SourceLoader::new();
    let mut editor = fixture(scheduler.clone());
    let first = editor.document.visit.current_source.unwrap();
    let other = editor.document.project.order[1];
    let old_layer = editor
        .document
        .project
        .source_mut(first)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .add(
            "Other".into(),
            AdjustmentLayer::global(AdjustmentLayerSettings::default()),
        )
        .unwrap();
    editor.select_adjustment_layer(Target::Layer(old_layer));
    with_application(editor, |app| {
        review(app);
        app.enter(|editor, cx| editor.select_roll_photo(1, egui::Modifiers::NONE, cx));
        app.run_until(LIMIT, |editor| !editor.interaction_blocked());
        let selected = app.read(|editor| editor.adjustment_layer_target());
        app.enter(|editor, cx| {
            editor.tab = 1;
            editor.document.visit.canvas.viewport.zoom = Some(0.75);
            editor.finish_pick(cx);
        });
        app.read(|editor| {
            assert_eq!(editor.document.visit.current_source, Some(other));
            assert_eq!(editor.document.selected, 1);
            assert_eq!(editor.adjustment_layer_target(), selected);
            assert_ne!(selected, Target::Layer(old_layer));
            assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(0.75));
            assert_eq!(editor.tab, 1);
            assert!(editor.document.analysis_review().is_none() && editor.test_pick().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn fresh_negative_analysis_measures_both_anchors_and_undo_restores_the_uncalibrated_roll() {
    let scheduler = source_loading::SourceLoader::new();
    let mut editor = fixture(scheduler.clone());
    let crate::processing::InputSettings::Negative { dmin, dmax, .. } = &mut editor.document.input
    else {
        panic!("authored scan roll must use Negative");
    };
    *dmin = None;
    *dmax = None;
    editor.sync_document();
    let before = editor.document.state.project.clone();
    assert!(!before.input.ready());
    assert!(before.input.calibration().is_none());
    with_application(editor, |app| {
        review(app);
        confirm(app);
        app.run_until(LIMIT, |e| !e.analysis_fitting() && !e.interaction_blocked());
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
            assert!(e.document.input.ready());
            assert_eq!(e.document.state.project.analysis_regions.len(), 2);
        });
        app.enter(|e, access| e.restore_history(Direction::Undo, access));
        app.run_until(LIMIT, |e| !e.interaction_blocked());
        app.read(|e| assert_eq!(e.document.state.project, before));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn native_fit_cancellation_preserves_accepted_data_without_blocking_unrelated_cpu_work() {
    let (scheduler, enabled, starts, release) = gated();
    let editor = fixture(scheduler.clone());
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        review(app);
        // Exercise the native-acquisition fallback explicitly; normal confirmation uses patches.
        app.update(|e| e.document.sampling_inputs = Default::default());
        enabled.store(true, Ordering::Relaxed);
        confirm(app);
        await_native(app, &starts);
        app.update(Editor::cancel_analysis);
        app.run_until(LIMIT, |e| !e.analysis_fitting() && !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.document.analysis_review().is_none());
            assert!(e.activities.view().is_none());
            assert!(e.history_label(Direction::Undo).is_none());
        });
        assert_eq!(scheduler.waiter_count(), 0);
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(LIMIT, |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        drop(probe);
        app.read(|e| assert_eq!(e.document.state.project, before));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn native_fallback_document_and_application_retirement_cannot_publish_late_results() {
    for close_application in [false, true] {
        let (scheduler, enabled, starts, release) = gated();
        let editor = fixture(scheduler.clone());
        with_application(editor, |app| {
            review(app);
            app.update(|e| e.document.sampling_inputs = Default::default());
            enabled.store(true, Ordering::Relaxed);
            confirm(app);
            await_native(app, &starts);
            let cancelled =
                app.read(|e| e.document.analysis_operation().unwrap().cancelled.clone());
            if close_application {
                app.close_execution();
            } else {
                app.update(|e| e.document = Document::default().into());
                app.pump();
                app.read(|e| assert!(e.document.analysis_review().is_none()));
            }
            assert!(cancelled.is_cancelled());
            assert_eq!(scheduler.waiter_count(), 0);
            release.send(()).unwrap();
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn cached_fit_cancellation_waits_for_worker_cleanup_without_blocking_other_cpu_jobs() {
    let scheduler = source_loading::SourceLoader::new();
    let editor = fixture(scheduler.clone());
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        review(app);
        scheduler.close_and_wait().unwrap();
        let (entered, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let enabled = AtomicBool::new(true);
        app.read(|e| {
            e.source_services
                .as_ref()
                .unwrap()
                .renderers
                .submissions()
                .observe_submissions(Arc::new(move || {
                    if enabled.swap(false, Ordering::SeqCst) {
                        entered.send(()).unwrap();
                        resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
                    }
                }))
        });
        confirm(app);
        let deadline = std::time::Instant::now() + LIMIT;
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            app.read(|e| {
                assert!(
                    e.analysis_fitting(),
                    "fit ended before GPU gate: {}",
                    e.status
                )
            });
            assert!(
                std::time::Instant::now() < deadline,
                "GPU fitting did not reach the observed submission"
            );
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        app.update(Editor::cancel_analysis);
        assert!(app.read(|e| {
            e.interaction_blocked()
                && e.document
                    .analysis_operation()
                    .unwrap()
                    .cancelled
                    .is_cancelled()
        }));
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move |_| entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(LIMIT, |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        app.run_until(LIMIT, |e| {
            !e.interaction_blocked() && e.document.analysis_operation().is_none()
        });
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.history_label(Direction::Undo).is_none());
            assert!(e.document.sampling_inputs.retained_bytes() > 0);
        });
        drop(probe);
    });
}

#[test]
fn new_document_supersedes_a_queued_fit_without_consuming_its_areas_or_history() {
    let scheduler = source_loading::SourceLoader::new();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        review(app);
        let admission = crate::background::testing::pause();
        confirm(app);
        app.pump();
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.pump();
        app.read(|e| {
            assert!(!e.analysis_fitting() && e.document.analysis_review().is_none());
            assert!(e.document_pending.is_some());
        });
        drop(admission);
        app.run_until(LIMIT, |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(e.document.state.project.order.is_empty());
            assert!(e.document.state.project.analysis_regions.is_empty());
            assert!(e.history_label(Direction::Undo).is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}
