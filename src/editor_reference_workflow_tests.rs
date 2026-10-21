use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        development,
        document::Document,
        source,
        tests::{fixture, hold_blocking_work},
    },
};
use std::{
    path::PathBuf,
    sync::{Mutex, atomic::AtomicUsize},
    time::Duration,
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn editor(scheduler: development::Scheduler) -> Editor {
    let mut e = fixture();
    e.document = Document::default().into();
    e.dmin = None;
    e.dmax = None;
    e.floor = 0.;
    e.sync_document();
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
    e.source_services = Some(source::Services {
        developments: scheduler,
        jobs: e.workflow_jobs.clone(),
        renderers: Arc::default(),
        device,
        queue,
    });
    e
}

#[test]
fn reference_proposal_and_correction_finish_without_backend_or_paint_and_keep_working_context() {
    let reference_reads = Arc::new(AtomicUsize::new(0));
    let observed = reference_reads.clone();
    let scheduler = development::Scheduler::new_observed(move |file, phase, _| {
        if file == path("bare-light.dng") && matches!(phase, development::NativePhase::UnpackBegin)
        {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.add_raws(vec![path("illuminated-patches.dng")]);
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        let (before, visit) = app.update(|e| {
            assert!(e.document.visit.ready.is_some(), "{}", e.status);
            e.viewport.zoom = Some(1.375);
            (e.history_snapshot(), e.document.visit.view)
        });
        app.enter(|e, access| {
            e.begin_flat_edit(path("bare-light.dng"), None);
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.region.is_some());
            assert!(e.source_context().unwrap().correction.is_none());
            assert_ne!(e.document.visit.view, visit);
        });
        app.enter(|e, access| {
            e.apply_flat_edit();
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert!(e.correction.is_some(), "{}", e.status);
            assert!(e.flat_edit.is_none() && e.document_pending.is_none());
            assert_eq!(e.document.project.order, before.order);
            assert_eq!(e.viewport.zoom, Some(1.375));
            assert_eq!(
                (e.document.project.dmin, e.document.project.dmax),
                (before.dmin, before.dmax)
            );
            assert_eq!(
                e.document.visit.ready.as_ref().unwrap().path,
                path("illuminated-patches.dng")
            );
            assert_ne!(e.document.visit.view, visit);
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Correction")
            );
        });
        assert_eq!(
            reference_reads.load(Ordering::Relaxed),
            1,
            "estimation must consume the captured reference, not reopen it"
        );
        let accepted = app.read(|e| e.document.project.clone());
        let current = app.read(|e| e.document.visit.ready.as_ref().unwrap().developed.clone());
        app.enter(|e, access| {
            e.begin_flat_edit(path("missing-reference.dng"), None);
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert!(Arc::ptr_eq(
                &current,
                &e.document.visit.ready.as_ref().unwrap().developed
            ));
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::FlatReference);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retiring_queued_reference_stages_restores_context_and_cannot_finish_a_newer_operation() {
    for accept in [false, true] {
        let scheduler = development::Scheduler::new();
        with_application(editor(scheduler.clone()), |app| {
            let before = app.read(|e| e.document.project.clone());
            if accept {
                app.enter(|e, access| {
                    e.begin_flat_edit(path("bare-light.dng"), None);
                    e.start_reference_workflow(access);
                });
                app.run_until(Duration::from_secs(20), |e| !e.busy);
            }
            let hold = app.read(|e| hold_blocking_work(&e.workflow_jobs));
            let old = app.enter(|e, access| {
                if accept {
                    e.apply_flat_edit();
                } else {
                    e.begin_flat_edit(path("bare-light.dng"), None);
                }
                e.start_reference_workflow(access);
                e.id
            });
            app.pump();
            app.update(|e| e.finish_flat_edit());
            app.pump();
            app.read(|e| {
                assert_eq!(e.document.project, before);
                assert!(!e.busy && e.document_pending.is_none());
                assert!(e.flat_edit.is_none());
            });
            app.enter(|e, access| {
                e.start_document(crate::editor::Preparation::New);
                e.start_document_workflow(access);
            });
            let operation = app.read(|e| e.document_pending);
            app.update(|e| {
                e.reference_previewed(old, Err("retired".into()));
                e.document_prepared(old, Err("retired".into()));
            });
            app.read(|e| assert_eq!(e.document_pending, operation));
            drop(hold);
            app.run_until(Duration::from_secs(20), |e| !e.busy);
            app.read(|e| {
                assert!(e.failure.is_none());
                assert!(e.document.project.flat_field.is_none());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn failed_document_supersession_retires_reference_acceptance_but_preserves_its_return_context() {
    let scheduler = development::Scheduler::new();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.add_raws(vec![path("illuminated-patches.dng")]);
            e.start_document_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        let (before, pixels, visit) = app.update(|e| {
            (
                e.history_snapshot(),
                e.document.visit.ready.as_ref().unwrap().developed.clone(),
                e.document.visit.view,
            )
        });
        app.enter(|e, access| {
            e.begin_flat_edit(path("bare-light.dng"), None);
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        let hold = app.read(|e| hold_blocking_work(&e.workflow_jobs));
        let old = app.enter(|e, access| {
            e.apply_flat_edit();
            e.start_reference_workflow(access);
            e.id
        });
        app.pump();
        app.enter(|e, access| {
            e.start_document(crate::editor::Preparation::Open(path(
                "missing-project.antitone",
            )));
            e.start_document_workflow(access);
        });
        let operation = app.read(|e| {
            let session = e.flat_edit.as_ref().unwrap();
            assert!(session.request.is_none() && session.task.is_none());
            assert!(session.working.is_some());
            e.document_pending.unwrap()
        });
        assert_ne!(old, operation);
        app.update(|e| e.document_prepared(old, Err("retired reference".into())));
        drop(hold);
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.flat_edit.is_none() && e.correction.is_none());
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.ready.as_ref().unwrap().developed
            ));
            assert_ne!(e.document.visit.view, visit);
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Import photos")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn departed_reference_waiter_keeps_native_admission_until_running_unpack_and_cleanup_finish() {
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin)
            && observed.fetch_add(1, Ordering::Relaxed) == 0
        {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(20))
                .unwrap();
        }
        Ok(())
    });
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.begin_flat_edit(path("bare-light.dng"), None);
            e.start_reference_workflow(access);
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(15)).unwrap();
        app.enter(|e, access| {
            e.finish_flat_edit();
            e.begin_flat_edit(path("bare-light.dng"), None);
            e.start_reference_workflow(access);
        });
        app.pump();
        assert_eq!(count.load(Ordering::Relaxed), 1);
        assert!(!app.read(|e| e.workflow_jobs.idle()));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert!(e.document.visit.ready.is_some());
            assert!(e.document.project.flat_field.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn reference_session_keeps_its_prepared_input_when_the_display_visit_retires() {
    let scheduler = development::Scheduler::new();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.begin_flat_edit(path("bare-light.dng"), None);
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.enter(|e, access| {
            let visit = e.request_id();
            e.replace_photo_scope(visit);
            assert!(e.document.visit.ready.is_none());
            assert!(e.flat_edit.as_ref().unwrap().reference.is_some());
            e.apply_flat_edit();
            assert!(e.document_pending.is_some());
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert!(e.correction.is_some(), "{}", e.status);
            assert!(e.document.project.order.is_empty());
            assert_eq!(e.document.project.sources.len(), 1);
            assert!(e.document.project.sources[0].reference_only);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn invalid_measured_support_fails_without_accepting_reference_records_or_history() {
    let scheduler = development::Scheduler::new();
    with_application(editor(scheduler.clone()), |app| {
        let before = app.read(|e| e.document.project.clone());
        app.enter(|e, access| {
            e.begin_flat_edit(path("bare-light.dng"), None);
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.enter(|e, access| {
            let area = e.document.visit.ready.as_ref().unwrap().area;
            e.region = Some(crate::coverage::Coverage::rectangle(crate::raw::Rect {
                x: area.x,
                y: area.y,
                width: 2,
                height: 2,
            }));
            e.apply_flat_edit();
            assert!(
                e.document_pending.is_some(),
                "integer support reaches the actual estimator"
            );
            e.start_reference_workflow(access);
        });
        app.run_until(Duration::from_secs(20), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.flat_edit.is_none() && e.correction.is_none());
            assert_eq!(
                e.failure.as_ref().unwrap().kind,
                FailureKind::FlatCorrection
            );
            assert!(
                e.history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn missing_reference_services_finish_activity_and_preserve_accepted_state() {
    let mut e = fixture();
    crate::editor::tests::add_ready_fixture(&mut e);
    let pixels = e.document.visit.ready.as_ref().unwrap().developed.clone();
    with_application(e, |app| {
        let before = app.update(|e| e.history_snapshot());
        app.enter(|e, access| {
            e.begin_flat_edit(path("bare-light.dng"), None);
            e.start_reference_workflow(access);
        });
        app.read(|e| {
            assert!(!e.busy && e.tasks.view().is_none());
            assert!(e.flat_edit.is_none());
            assert_eq!(e.document.project, before);
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::FlatReference);
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.ready.as_ref().unwrap().developed
            ));
        });
    });
}
