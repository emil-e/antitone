use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{history, source_loading, tests::authored_fixture},
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

fn fixture(scheduler: source_loading::SourceLoader) -> Editor {
    let mut editor = authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    let original = std::mem::replace(
        &mut editor.source_services.as_mut().unwrap().developments,
        scheduler,
    );
    editor.document.cache.set_budget(0);
    original.close_and_wait().unwrap();
    editor.sync_document();
    editor
}

fn observed() -> (source_loading::SourceLoader, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let starts = count.clone();
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
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
    let accepted = editor.document.state.project.clone();
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
            assert_eq!(editor.document.state.project, accepted);
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
            assert_eq!(editor.document.state.project, accepted);
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
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.history_label(history::Direction::Undo), undo);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn authored_analyze_first_keeps_regions_after_review_cancel_and_auto_crop_reuses_them() {
    let (scheduler, starts) = observed();
    let editor = fixture(scheduler.clone());
    let accepted = editor.document.state.project.clone();
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
            assert_eq!(editor.document.state.project, accepted);
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
            assert_eq!(editor.document.state.project, accepted);
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
    let accepted = editor.document.state.project.clone();
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
            assert_eq!(editor.document.state.project, accepted);
            assert!(editor.tasks.view().is_none());
        });
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        drop(admission);
    });
    scheduler.close_and_wait().unwrap();
}

fn paused() -> (
    source_loading::SourceLoader,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let once = AtomicUsize::new(0);
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin)
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
                Arc::downgrade(&editor.document.cache),
            )
        });
        app.update(|editor| editor.document = Document::default().into());
        app.pump();
        assert!(cancelled.is_cancelled());
        assert!(snapshot.upgrade().is_none());
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

#[test]
fn retiring_a_photo_discards_its_crop_consumer_but_keeps_shared_regions_for_the_document() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    // Declared geometry isolates continuation ownership from blind detector abstentions.
    let metadata = consumers::declared(&editor);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.begin_crop();
            assert!(editor.document.visit.crop_edit.is_some());
            editor.request_auto_crop(auto_crop::Target::Photo, cx);
        });
        await_native(app, &starts);
        let request = app.read(|editor| editor.document.spatial_preparing.as_ref().unwrap().id);
        let replacement_draft = app.update(|editor| {
            let ready = editor.document.visit.ready.clone().unwrap();
            let visit = editor.request_id();
            editor.replace_photo_scope(visit);
            assert!(editor.document.spatial_preparing.is_some());
            editor.document.visit.ready = Some(ready);
            editor.busy = false;
            editor.begin_crop();
            editor.busy = true;
            let draft = &mut editor.document.visit.crop_edit.as_mut().unwrap().draft;
            draft.size = [17, 19];
            *draft
        });
        app.enter(|editor, cx| editor.spatial_result(request, Ok(Some(metadata)), cx));
        release.send(()).unwrap();
        app.update(|editor| {
            assert!(!editor.document.spatial.ready().groups.is_empty());
            assert_eq!(
                editor.document.visit.crop_edit.as_ref().unwrap().draft,
                replacement_draft,
                "a departed consumer retargeted the replacement visit's draft"
            );
            editor.auto_crop_sources(
                auto_crop::Target::Photo,
                vec![editor.document.visit.current_source.unwrap()],
            );
            assert_ne!(
                editor.document.visit.crop_edit.as_ref().unwrap().draft,
                replacement_draft,
                "the control must provide a usable proposal, not merely an unavailable no-op"
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn failed_or_rejected_photo_region_preparation_can_retry_in_the_same_crop_visit() {
    for rejected_publication in [false, true] {
        let (scheduler, starts, release) = paused();
        let editor = fixture(scheduler.clone());
        let metadata = consumers::declared(&editor);
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.begin_crop();
                editor.request_auto_crop(auto_crop::Target::Photo, cx);
            });
            await_native(app, &starts);
            let (first, visit, before) = app.read(|editor| {
                (
                    editor.document.spatial_preparing.as_ref().unwrap().id,
                    editor.document.visit.view,
                    editor.document.visit.crop_edit.as_ref().unwrap().draft,
                )
            });
            app.enter(|editor, cx| {
                if rejected_publication {
                    editor.document.spatial.cancel();
                    editor.spatial_result(first, Ok(Some(metadata.clone())), cx);
                } else {
                    editor.spatial_result(
                        first,
                        Err("authored region preparation failure".into()),
                        cx,
                    );
                }
                assert!(editor.document.spatial_preparing.is_none());
                assert_eq!(
                    editor.document.visit.crop_edit.as_ref().unwrap().draft,
                    before
                );
                editor.request_auto_crop(auto_crop::Target::Photo, cx);
                let retry = editor
                    .document
                    .spatial_preparing
                    .as_ref()
                    .expect("a terminal failure stranded the crop consumer and blocked retry")
                    .id;
                assert_ne!(retry, first);
                assert_eq!(editor.document.visit.view, visit);
                editor.spatial_result(retry, Ok(Some(metadata.clone())), cx);
                assert!(editor.document.spatial_preparing.is_none());
                assert_ne!(
                    editor.document.visit.crop_edit.as_ref().unwrap().draft,
                    before,
                    "retry must consume a usable proposal, not merely register another request"
                );
            });
            release.send(()).unwrap();
        });
        scheduler.close_and_wait().unwrap();
    }
}
