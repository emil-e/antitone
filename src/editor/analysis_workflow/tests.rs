use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{Document, Preparation, Thumbnails, source, source_loading},
};
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::mpsc,
    time::{Duration, Instant},
};

pub(super) const LIMIT: Duration = Duration::from_secs(20);

pub(super) fn fixture(scheduler: source_loading::SourceLoader) -> Editor {
    let mut editor = crate::editor::tests::fixture();
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    let initial = source_loading::SourceLoader::new();
    editor.source_services = Some(source::Services {
        developments: initial.clone(),
        renderers: Arc::default(),
        device,
        queue,
    });
    editor.document.thumbnails =
        (Thumbnails::default()).with_sources(editor.document.cache.clone());
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw");
    let paths = [root.join("roll-0.dng"), root.join("roll-1.dng")];
    let mut document = Document::default();
    document.import(&paths).unwrap();
    editor.document.cache.set_budget(0);
    editor.document.install(document);
    let owner = RefCell::new(Some(editor));
    {
        let mut app = TestApplication::new(&owner, || {});
        app.enter(|e, cx| e.load(paths[0].clone(), cx));
        app.run_until(LIMIT, |e| !e.busy);
        app.read(|e| assert!(e.document.visit.ready.is_some(), "{}", e.status));
        app.close_execution();
    }
    let mut editor = owner.into_inner().unwrap();
    editor.source_services.as_mut().unwrap().developments = scheduler;
    // Source promotion applies the configured cache budget. Disable optional
    // retention afterward; the live selected source may still be reused.
    editor.document.cache.set_budget(0);
    initial.close_and_wait().unwrap();
    editor
}

pub(super) fn start(app: &TestApplication<'_, Editor>) {
    app.enter(|e, access| {
        // These cases isolate sampling preparation after shared metadata. The composed
        // Analyze entry and both consumer orders are covered by spatial workflow tests.
        let request = e.analysis_request().expect("eligible analysis request");
        e.prepare_analysis_review(request, access);
        assert!(e.document.analysis_preparing.is_some());
    });
}

fn paused() -> (
    source_loading::SourceLoader,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume.lock().unwrap().recv_timeout(LIMIT).unwrap();
        }
        Ok(())
    });
    (scheduler, starts, release)
}

pub(super) fn await_native(app: &mut TestApplication<'_, Editor>, starts: &mpsc::Receiver<()>) {
    let deadline = Instant::now() + LIMIT;
    loop {
        app.pump();
        if starts.try_recv().is_ok() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "native preparation did not start: {}",
            app.read(|e| format!(
                "{}; preparing={}",
                e.status,
                e.document.analysis_preparing.is_some()
            ))
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn areas_open_review_without_backend_or_painting_and_remain_drafts() {
    let scheduler = source_loading::SourceLoader::new();
    let editor = fixture(scheduler.clone());
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        start(app);
        app.run_until(LIMIT, |e| e.document.analysis_preparing.is_none());
        app.read(|e| {
            let review = e
                .document
                .analysis_review
                .as_ref()
                .expect("prepared review");
            assert_eq!(review.approved.entries.len(), 2);
            assert!(review.approved.entries.iter().all(|e| e.areas.ready()));
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.document.picking.kind, Some(crate::editor::Pick::Analysis));
            assert!(!e.busy);
            assert!(e.tasks.view().is_none());
            assert_eq!(e.document.cache.charges().retained, 0);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_user_cancel_acknowledges_without_starting_native_work() {
    let scheduler = source_loading::SourceLoader::new();
    let editor = fixture(scheduler.clone());
    let before = editor.document.state.project.clone();
    let admission = crate::background::testing::pause();
    with_application(editor, |app| {
        start(app);
        app.pump();
        app.update(Editor::cancel_analysis);
        drop(admission);
        app.run_until(LIMIT, |e| e.document.analysis_preparing.is_none());
        app.read(|e| {
            assert!(!e.busy && e.document.analysis_review.is_none());
            assert_eq!(e.document.state.project, before);
            assert!(e.tasks.view().is_none());
        });
        assert_eq!(scheduler.waiter_count(), 0);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn native_demand_cancellation_acknowledges_without_blocking_unrelated_cpu_work() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        app.update(Editor::cancel_analysis);
        app.run_until(LIMIT, |e| e.document.analysis_preparing.is_none());
        app.read(|e| {
            assert!(!e.busy && e.document.analysis_review.is_none());
            assert_eq!(e.document.state.project, before);
        });
        assert_eq!(scheduler.waiter_count(), 0);
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
        app.read(|e| assert_eq!(e.document.state.project, before));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn document_drop_and_application_close_retire_preparation_before_native_completion() {
    for close_application in [false, true] {
        let (scheduler, starts, release) = paused();
        let editor = fixture(scheduler.clone());
        with_application(editor, |app| {
            start(app);
            await_native(app, &starts);
            let cancelled = app.read(|e| {
                e.document
                    .analysis_preparing
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
            }
            assert!(cancelled.is_cancelled());
            assert_eq!(scheduler.waiter_count(), 0);
            app.read(|e| assert!(e.document.analysis_review.is_none()));
            release.send(()).unwrap();
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn document_transition_retires_areas_and_waits_for_native_work_before_promotion() {
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    let before = editor.document.state.project.clone();
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.pump();
        app.read(|e| {
            assert!(e.document_pending.is_some());
            assert!(e.document.analysis_preparing.is_none());
            assert_eq!(e.document.state.project, before);
        });
        release.send(()).unwrap();
        app.run_until(LIMIT, |e| e.document_pending.is_none());
        app.read(|e| {
            assert!(e.document.state.project.order.is_empty());
            assert!(e.document.analysis_review.is_none());
            assert!(e.document.analysis_preparing.is_none());
            assert!(!e.busy);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn live_edits_while_native_preparation_runs_discard_the_draft() {
    // Gate the source not already held by the selected visit. A stale preparation
    // still completes its captured service operation before the controller rejects it.
    let (scheduler, starts, release) = paused();
    let editor = fixture(scheduler.clone());
    with_application(editor, |app| {
        start(app);
        await_native(app, &starts);
        app.update(|e| {
            e.set_gamma(0.8);
            e.sync_document();
        });
        let edited = app.read(|e| e.document.state.project.clone());
        release.send(()).unwrap();
        app.run_until(LIMIT, |e| e.document.analysis_preparing.is_none());
        app.read(|e| {
            assert_eq!(e.document.state.project, edited);
            assert!(e.document.analysis_review.is_none());
            assert!(!e.busy);
            assert!(e.status.contains("discarded"), "{}", e.status);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn unavailable_services_finish_the_activity_without_opening_review() {
    let scheduler = source_loading::SourceLoader::new();
    let mut editor = fixture(scheduler.clone());
    let before = editor.document.state.project.clone();
    editor.source_services = None;
    with_application(editor, |app| {
        start(app);
        assert!(app.read(|e| e.document.analysis_preparing.is_some() && e.busy));
        app.run_until(LIMIT, |e| e.document.analysis_preparing.is_none());
        app.read(|e| {
            assert!(e.document.analysis_preparing.is_none());
            assert!(e.document.analysis_review.is_none());
            assert_eq!(e.document.state.project, before);
            assert!(!e.busy);
            assert!(e.tasks.view().is_none());
            assert!(e.status.contains("unavailable"));
        });
    });
    scheduler.close_and_wait().unwrap();
}
