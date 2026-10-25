use super::*;
use crate::{
    application::test_support::with_application,
    geometry::{Composition, FramePoint},
};
use std::sync::atomic::Ordering;

fn fixture() -> (
    Editor,
    source::Context,
    ViewSourceSpec,
    development::Scheduler,
) {
    let mut e = crate::editor::tests::fixture();
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
    let scan = Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let composition = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    let size = composition.size();
    let frame = FrameRaster::new(
        composition,
        FramePoint([0., 0.]),
        FramePoint(size.map(|n| f64::from(n) / 2.)),
        [64, 48],
    )
    .unwrap();
    let spec = ViewSourceSpec::inspection(e.id, frame, scan.camera_to_ap1, None).unwrap();
    let input = source::Context {
        source: scan.source().clone(),
        correction: None,
    };
    let scheduler = development::Scheduler::new();
    e.source_services = Some(source::Services {
        device: device.clone(),
        queue: queue.clone(),
        developments: scheduler.clone(),
        jobs: e.workflow_jobs.clone(),
        renderers: Arc::default(),
    });
    e.working = WorkingView::new(device, queue.clone());
    e.source_epoch.store(e.id, Ordering::Relaxed);
    (e, input, spec, scheduler)
}

#[test]
fn physical_support_completes_without_backend_or_another_paint() {
    let (e, input, spec, scheduler) = fixture();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.request_working(spec.clone(), input, access);
        });
        app.run_until(Duration::from_secs(15), |e| e.working.support_ready());
        app.read(|e| {
            assert!(e.working.source.as_ref().unwrap().spec == spec);
            assert!(e.working.underlay.is_some());
            assert!(e.document.visit.working.task.is_none());
            assert!(
                e.working.displayed.is_none(),
                "colour capture remains frame-bound"
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_navigation_uses_latest_demand_and_reuses_neutral_support_after_grade_output_changes() {
    let (e, input, mut spec, scheduler) = fixture();
    let hold = crate::editor::tests::hold_blocking_work(&e.workflow_jobs);
    with_application(e, |app| {
        for _ in 0..40 {
            spec.frame.min.0[0] += 1.;
            spec.frame.max.0[0] += 1.;
            app.enter(|e, access| {
                e.request_working(spec.clone(), input.clone(), access);
            });
            app.pump();
            app.read(|e| {
                assert!(!e.working.support_ready());
                assert!(e.document.visit.working.task.is_some());
            });
        }
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| e.working.support_ready());
        let neutral = app.read(|e| {
            assert!(e.working.source.as_ref().unwrap().spec == spec);
            e.working
                .mailbox
                .0
                .lock()
                .unwrap()
                .neutral_detail
                .clone()
                .unwrap()
        });
        app.enter(|e, access| {
            e.target = OutputTarget::Srgb;
            e.source_exposure = 1.2;
            let processing =
                ProcessingInput::from(ProcessingParameters::source_inspection(1.2).unwrap());
            e.observe_grade(&processing);
            e.request_working(spec.clone(), input, access);
        });
        app.pump();
        app.read(|e| {
            assert!(
                e.document.visit.working.task.is_none(),
                "output-only demand must not reacquire originals"
            );
            assert!(Arc::ptr_eq(
                &neutral,
                e.working
                    .mailbox
                    .0
                    .lock()
                    .unwrap()
                    .neutral_detail
                    .as_ref()
                    .unwrap()
            ));
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retiring_running_support_keeps_native_lease_and_physical_admission_through_submission() {
    let (e, input, mut spec, scheduler) = fixture();
    let source = Arc::downgrade(&input.source);
    let scan = Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let next = source::Context {
        source: scan.source().clone(),
        correction: None,
    };
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    e.submissions.observe_submissions(Arc::new(move || {
        if observed.fetch_add(1, Ordering::Relaxed) == 0 {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
    }));
    with_application(e, |app| {
        app.enter(|e, access| {
            e.request_working(spec.clone(), input, access);
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        spec.frame.min.0[0] += 1.;
        spec.frame.max.0[0] += 1.;
        app.enter(|e, access| {
            e.replace_photo_scope(8);
            e.request_working(spec.clone(), next, access);
        });
        app.pump();
        assert!(source.upgrade().is_some());
        assert!(!app.read(|e| e.workflow_jobs.idle()));
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "a departed acquisition still owns physical admission"
        );
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| e.working.support_ready());
        assert!(source.upgrade().is_none());
        app.read(|e| {
            assert!(e.working.source.as_ref().unwrap().spec == spec);
            assert!(e.workflow_jobs.idle());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn visit_retirement_drops_queued_support_and_cannot_restore_the_old_source() {
    let (e, input, spec, scheduler) = fixture();
    let source = Arc::downgrade(&input.source);
    let hold = crate::editor::tests::hold_blocking_work(&e.workflow_jobs);
    with_application(e, |app| {
        app.enter(|e, access| {
            e.request_working(spec, input, access);
        });
        app.pump();
        app.update(|e| e.replace_photo_scope(8));
        app.pump();
        assert!(source.upgrade().is_none());
        drop(hold);
        app.pump();
        app.read(|e| {
            assert!(e.working.source.is_none() && e.working.underlay.is_none());
            assert!(e.document.visit.working.task.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn save_completion_readmits_existing_working_demand_without_another_paint() {
    let (e, input, spec, scheduler) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.antitone");
    with_application(e, |app| {
        app.enter(|e, cx| {
            e.save_project(path.clone(), crate::project::Save::New, None, cx);
            e.request_working(spec.clone(), input, cx);
            assert!(e.document_pending.is_some());
            assert!(
                e.document.visit.working.task.is_none(),
                "foreground admission is paused"
            );
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document_pending.is_none() && e.working.support_ready()
        });
        app.read(|e| {
            assert!(e.working.source.as_ref().unwrap().spec == spec);
            assert!(e.document.visit.working.task.is_none() && !e.busy);
            assert!(e.failure.is_none(), "{}", e.status);
        });
        assert!(path.is_file());
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn guarded_save_close_does_not_readmit_waiting_working_demand() {
    let (e, input, spec, scheduler) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.antitone");
    with_application(e, |app| {
        app.enter(|e, cx| {
            e.save_project(
                path.clone(),
                crate::project::Save::New,
                Some(crate::editor::Transition::Close),
                cx,
            );
            e.request_working(spec, input, cx);
            assert!(e.document.visit.working.task.is_none());
        });
        app.run_until(Duration::from_secs(15), |e| e.allow_close);
        app.pump();
        app.read(|e| {
            assert!(e.document_pending.is_none() && !e.busy);
            assert!(e.document.visit.working.task.is_none());
            assert!(
                e.working.source.is_none(),
                "close must not start another acquisition"
            );
        });
        assert!(path.is_file());
    });
    scheduler.close_and_wait().unwrap();
}
