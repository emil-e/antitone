use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Preparation, development,
        document::Document,
        source,
        tests::{fixture, hold_blocking_work},
    },
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicUsize, mpsc},
    time::Duration,
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn editor(scheduler: development::Scheduler) -> Editor {
    let mut editor = fixture();
    editor.document.install(Document::default());
    editor.dmin = None;
    editor.dmax = None;
    editor.floor = 0.;
    editor.sync_document();
    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    editor.source_services = Some(source::Services {
        developments: scheduler,
        jobs: editor.workflow_jobs.clone(),
        renderers: Arc::default(),
        device,
        queue,
    });
    editor
}

#[test]
fn import_history_correction_and_new_run_without_backend_or_painting() {
    let scheduler = development::Scheduler::new();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.add_raws(vec![path("patches.dng")]);
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        let (accepted, selected, visit) = app.read(|e| {
            assert!(e.document.visit.ready.is_some(), "{}", e.status);
            assert_eq!(e.document.project.order.len(), 1);
            assert!(e.document.dirty());
            (
                e.document.project.clone(),
                e.current_source,
                e.document.visit.view,
            )
        });
        app.enter(|e, access| {
            e.start_document(Preparation::Correction(e.document.capture(), selected));
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert_eq!(e.document.visit.view, visit);
        });
        app.enter(|e, access| {
            e.restore_history(crate::editor::history::Direction::Undo);
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(
                e.document.project.order.is_empty(),
                "status={} undo={:?}",
                e.status,
                e.history_label(crate::editor::history::Direction::Undo)
            );
            assert!(e.document.visit.ready.is_none());
        });
        app.enter(|e, access| {
            e.restore_history(crate::editor::history::Direction::Redo);
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert!(e.document.visit.ready.is_some());
        });
        let cache = app.read(|e| e.document.cache().clone());
        app.enter(|e, access| {
            e.start_document(Preparation::New);
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, crate::project::Project::default());
            assert!(!e.document.dirty());
            assert!(e.document.visit.ready.is_none());
            assert!(
                e.history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
        });
        assert!(cache.is_closed());
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn open_prepares_first_source_before_promotion_and_failed_open_preserves_owner_history_and_pixels()
{
    let scheduler = development::Scheduler::new();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("candidate.antitone");
    let mut candidate = Document::default();
    candidate.import(&[path("patches.dng")]).unwrap();
    candidate
        .save(&destination, crate::project::Save::New)
        .unwrap();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.start_document(Preparation::Open(destination.clone()));
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        let (snapshot, visit, native, store) = app.update(|e| {
            let before = e.history_snapshot();
            e.roll.exposure = 0.25;
            e.record_edit(before, "Exposure", None);
            (
                e.document.project.clone(),
                e.document.visit.view,
                e.document.visit.ready.as_ref().unwrap().developed.clone(),
                e.document.cache().clone(),
            )
        });
        app.enter(|e, access| {
            e.start_document(Preparation::Open(directory.path().join("missing.antitone")));
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, snapshot);
            assert_eq!(e.document.visit.view, visit);
            assert!(Arc::ptr_eq(
                &e.document.visit.ready.as_ref().unwrap().developed,
                &native
            ));
            assert!(!store.is_closed());
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Exposure")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_supersession_retires_old_continuation_and_stale_progress_cannot_touch_new_work() {
    let scheduler = development::Scheduler::new();
    let e = editor(scheduler.clone());
    let hold = hold_blocking_work(&e.workflow_jobs);
    with_application(e, |app| {
        app.enter(|e, access| {
            e.add_raws(vec![path("patches.dng")]);
            e.start_document_workflow(access);
        });
        app.pump();
        let old = app.read(|e| e.document_pending.unwrap());
        app.enter(|e, access| {
            e.start_document(Preparation::New);
            e.start_document_workflow(access);
        });
        let current = app.read(|e| e.document_pending.unwrap());
        app.pump();
        app.update(|e| {
            e.document_progress(old, 5, 10);
            e.document_prepared(old, Err("stale".into()));
        });
        app.read(|e| {
            assert_eq!(e.document_pending, Some(current));
            assert!(e.tasks.view().unwrap().measured.is_none());
            assert!(e.busy);
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(e.document.project.sources.is_empty());
            assert!(e.document.visit.ready.is_none());
            assert_eq!(e.status, "Project ready");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn pending_native_candidate_keeps_accepted_state_and_admission_after_logical_retirement() {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin) {
            observed.fetch_add(1, Ordering::Relaxed);
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        Ok(())
    });
    let e = editor(scheduler.clone());
    with_application(e, |app| {
        let accepted = app.read(|e| e.document.project.clone());
        app.enter(|e, access| {
            e.add_raws(vec![path("patches.dng")]);
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(10), |_| scheduler.waiter_count() > 0);
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert!(e.document.visit.ready.is_none() && e.busy);
        });
        app.enter(|e, access| {
            e.start_document(Preparation::New);
            e.start_document_workflow(access);
        });
        app.pump();
        assert_eq!(scheduler.waiter_count(), 0);
        assert!(app.read(|e| e.busy));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        app.read(|e| {
            assert!(e.document.project.sources.is_empty() && e.document.visit.ready.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}
