use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{history, source_loader, tests::authored_fixture},
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

mod consumers;

const LIMIT: Duration = Duration::from_secs(30);

fn fixture(scheduler: source_loader::SourceLoader) -> Editor {
    let mut editor = authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    let original = std::mem::replace(
        &mut editor.source_services.as_mut().unwrap().developments,
        scheduler,
    );
    editor.document.cache().set_budget(0);
    original.close_and_wait().unwrap();
    editor.sync_document();
    editor
}

fn observed() -> (source_loader::SourceLoader, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let starts = count.clone();
    let scheduler = source_loader::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loader::NativePhase::UnpackBegin) {
            starts.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    });
    (scheduler, count)
}

#[test]
fn authored_auto_crop_first_reuses_regions_for_analyze_without_accepting_either_consumers_edits() {
    let (scheduler, starts) = observed();
    let editor = fixture(scheduler.clone());
    let accepted = editor.document.project.clone();
    let dirty = editor.document.dirty();
    let undo = editor.history_label(history::Direction::Undo);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_auto_crop(auto_crop::Target::Roll, cx));
        assert!(app.read(|editor| editor.document.spatial_preparing.is_some()));
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        let regions = app.read(|editor| {
            assert!(!editor.busy, "{}", editor.status);
            assert!(editor.document.analysis_preparing.is_none());
            assert!(editor.document.analysis_pending.is_none());
            assert!(editor.document.analysis_review.is_none());
            assert_eq!(editor.document.project, accepted);
            assert_eq!(editor.document.dirty(), dirty);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
            let regions = editor.document.spatial.ready();
            assert_eq!(
                regions
                    .groups
                    .iter()
                    .map(|g| g.sources.len())
                    .sum::<usize>(),
                2
            );
            assert!(
                editor.document.auto_crop_edit.is_some(),
                "{}",
                editor.status
            );
            regions
        });
        let acquired = starts.load(Ordering::SeqCst);
        assert!(
            acquired > 0,
            "the first request must exercise native acquisition, not just a supplied metadata fixture"
        );
        app.enter(|editor, cx| {
            editor.finish_auto_crop(false);
            editor.auto_crop_margin = 1.;
            editor.request_auto_crop(auto_crop::Target::Roll, cx);
        });
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.read(|editor| {
            assert_eq!(*editor.document.spatial.ready(), *regions);
            assert_eq!(editor.document.project, accepted);
            assert!(
                editor.document.auto_crop_edit.is_some(),
                "{}",
                editor.status
            );
        });
        assert_eq!(
            starts.load(Ordering::SeqCst),
            acquired,
            "changing a consumer margin must not reacquire RAWs or redetect"
        );
        app.enter(|editor, cx| {
            editor.finish_auto_crop(false);
            editor.request_analysis(cx);
        });
        app.run_until(LIMIT, |editor| {
            editor.document.spatial_preparing.is_none()
                && editor.document.analysis_preparing.is_none()
        });
        app.read(|editor| {
            assert!(
                editor.document.analysis_review.is_some(),
                "{}",
                editor.status
            );
            assert_eq!(*editor.document.spatial.ready(), *regions);
            assert_eq!(editor.document.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn authored_analyze_first_keeps_regions_after_review_cancel_and_auto_crop_reuses_them() {
    let (scheduler, starts) = observed();
    let editor = fixture(scheduler.clone());
    let accepted = editor.document.project.clone();
    let undo = editor.history_label(history::Direction::Undo);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.run_until(LIMIT, |editor| {
            editor.document.spatial_preparing.is_none()
                && editor.document.analysis_preparing.is_none()
        });
        let regions = app.read(|editor| {
            assert!(
                editor.document.analysis_review.is_some(),
                "{}",
                editor.status
            );
            assert_eq!(editor.document.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
            assert_eq!(
                editor
                    .document
                    .spatial
                    .ready()
                    .groups
                    .iter()
                    .map(|g| g.sources.len())
                    .sum::<usize>(),
                2
            );
            editor.document.spatial.ready()
        });
        app.enter(|editor, cx| editor.finish_pick(cx));
        app.run_until(LIMIT, |editor| !editor.busy);
        assert!(app.read(|editor| editor.document.analysis_review.is_none()));
        let acquired = starts.load(Ordering::SeqCst);
        app.enter(|editor, cx| editor.request_auto_crop(auto_crop::Target::Roll, cx));
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.read(|editor| {
            assert_eq!(*editor.document.spatial.ready(), *regions);
            assert_eq!(editor.document.project, accepted);
            assert!(
                editor.document.auto_crop_edit.is_some(),
                "{}",
                editor.status
            );
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
        });
        assert_eq!(starts.load(Ordering::SeqCst), acquired);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_shared_preparation_cancels_without_native_work_or_accepted_changes() {
    let (scheduler, starts) = observed();
    let editor = fixture(scheduler.clone());
    let accepted = editor.document.project.clone();
    let admission = crate::background::testing::pause();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_analysis(cx));
        app.pump();
        app.update(Editor::cancel_analysis);
        app.pump();
        app.read(|editor| {
            assert!(editor.document.spatial_preparing.is_none());
            assert!(editor.document.analysis_preparing.is_none());
            assert!(editor.document.analysis_review.is_none());
            assert!(!editor.busy);
            assert_eq!(editor.document.project, accepted);
            assert!(editor.tasks.view().is_none());
        });
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        drop(admission);
    });
    scheduler.close_and_wait().unwrap();
}

fn paused() -> (
    source_loader::SourceLoader,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let once = AtomicUsize::new(0);
    let scheduler = source_loader::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loader::NativePhase::UnpackBegin)
            && once.fetch_add(1, Ordering::SeqCst) == 0
        {
            entered.send(()).unwrap();
            resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
        }
        Ok(())
    });
    (scheduler, starts, release)
}

fn await_native(app: &mut TestApplication<'_, Editor>, starts: &mpsc::Receiver<()>) {
    let deadline = Instant::now() + LIMIT;
    loop {
        app.pump();
        if starts.try_recv().is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{}",
            app.read(|editor| editor.status.clone())
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn retiring_a_photo_visit_does_not_retire_document_region_preparation() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_auto_crop(auto_crop::Target::Roll, cx));
        await_native(app, &starts);
        let request = app.read(|editor| editor.document.spatial_preparing.as_ref().unwrap().id);
        app.update(|editor| {
            let visit = editor.request_id();
            editor.replace_photo_scope(visit);
            assert_eq!(
                editor.document.spatial_preparing.as_ref().unwrap().id,
                request
            );
        });
        release.send(()).unwrap();
        app.run_until(LIMIT, |editor| editor.document.spatial_preparing.is_none());
        app.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .spatial
                    .ready()
                    .groups
                    .iter()
                    .map(|g| g.sources.len())
                    .sum::<usize>(),
                2
            );
            assert!(editor.document.analysis_review.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_retirement_cancels_publication_without_blocking_unrelated_cpu_work() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_auto_crop(auto_crop::Target::Roll, cx));
        await_native(app, &starts);
        let (cancelled, snapshot) = app.read(|editor| {
            (
                editor
                    .document
                    .spatial_preparing
                    .as_ref()
                    .unwrap()
                    .cancelled
                    .clone(),
                editor.document.capture(),
            )
        });
        app.update(|editor| editor.document = Document::default().into());
        app.pump();
        assert!(cancelled.is_cancelled());
        assert!(snapshot.cache().is_closed());
        let (entered, observed) = mpsc::channel();
        let probe = app
            .access()
            .spawn(move |_| async move {
                crate::background::run(move || entered.send(()).unwrap())
                    .await
                    .unwrap();
            })
            .unwrap();
        app.run_until(LIMIT, |_| observed.try_recv().is_ok());
        release.send(()).unwrap();
        drop(probe);
        app.read(|editor| {
            assert!(editor.document.spatial.ready().groups.is_empty());
            assert!(editor.document.auto_crop_edit.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn application_retirement_cancels_shared_continuation_before_native_cleanup_returns() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_auto_crop(auto_crop::Target::Roll, cx));
        await_native(app, &starts);
        let cancelled = app.read(|editor| {
            editor
                .document
                .spatial_preparing
                .as_ref()
                .unwrap()
                .cancelled
                .clone()
        });
        app.close_execution();
        assert!(cancelled.is_cancelled());
        app.read(|editor| {
            assert!(editor.document.spatial.ready().groups.is_empty());
            assert!(editor.document.auto_crop_edit.is_none());
        });
        release.send(()).unwrap();
    });
    scheduler.close_and_wait().unwrap();
}
