use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        OriginalPurpose, OutputTarget, ProcessingInput, ProcessingParameters, ViewRequest,
        development, tests::fixture,
    },
};
use std::time::{Duration, Instant};

fn editor() -> (Editor, development::Scheduler) {
    let mut e = fixture();
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
    let scheduler = development::Scheduler::new();
    e.source_services = Some(source::Services {
        device,
        queue,
        developments: scheduler.clone(),
        renderers: Arc::default(),
    });
    (e, scheduler)
}

fn demand(
    e: &mut Editor,
    ev: f64,
    cx: &UiContext<'_, Editor>,
) -> std::sync::Weak<crate::scan::DevelopedSource> {
    let source = source::Context {
        source: crate::editor::tests::developed_fixture(),
        correction: None,
    };
    let resource = Arc::downgrade(&source.source);
    let view = ViewRequest {
        area: source.source.area(),
        output: [16, 12],
        frame: None,
    };
    let composition =
        crate::geometry::Composition::new(view.area, Default::default(), Default::default())
            .unwrap();
    let input = ProcessingInput::from(ProcessingParameters::source_inspection(ev).unwrap());
    e.observe_grade(&input);
    e.observe_original(
        Some(OriginalPurpose::Tool),
        (
            input.signature(),
            OutputTarget::Srgb,
            Some(view),
            composition,
        ),
    );
    e.original.last_edit = Instant::now() - Duration::from_secs(1);
    e.request_original(
        &input,
        OutputTarget::Srgb,
        &egui::Context::default(),
        source,
        cx,
    );
    resource
}

#[test]
fn original_reconstruction_finishes_without_backend_or_another_paint() {
    let (e, scheduler) = editor();
    with_application(e, |app| {
        app.enter(|e, access| {
            demand(e, 0., access);
        });
        app.run_until(Duration::from_secs(15), |e| e.original.settled.is_some());
        app.read(|e| {
            let (revision, view, image) = e.original.settled.as_ref().unwrap();
            assert_eq!(*revision, e.original.revision());
            assert_eq!(view.output, [16, 12]);
            assert_eq!(image.processing_byte_len(), (0, 0));
            assert!(e.original.refining.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn rapid_output_demand_drops_queued_inputs_and_only_latest_reconstruction_can_publish() {
    let (e, scheduler) = editor();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let mut retired = Vec::new();
        for n in 0..40 {
            app.enter(|e, access| {
                retired.push(demand(e, n as f64 / 100., access));
            });
            app.pump();
        }
        assert!(
            retired[..39]
                .iter()
                .all(|source| source.upgrade().is_none()),
            "queued physical jobs must not retain every slider snapshot"
        );
        app.read(|e| assert!(e.original.settled.is_none()));
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| e.original.settled.is_some());
        app.read(|e| {
            assert_eq!(
                e.original.settled.as_ref().unwrap().0,
                e.original.revision()
            )
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn leaving_a_photo_retires_queued_original_work_and_cannot_publish_into_a_revisit() {
    let (e, scheduler) = editor();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let (old, resource) = app.enter(|e, access| {
            let resource = demand(e, 0., access);
            (e.original.revision(), resource)
        });
        app.pump();
        app.update(|e| e.replace_photo_scope(8));
        app.pump();
        assert!(resource.upgrade().is_none());
        drop(hold);
        app.enter(|e, access| {
            demand(e, 0.2, access);
        });
        app.run_until(Duration::from_secs(15), |e| e.original.settled.is_some());
        let image = app.read(|e| e.original.settled.as_ref().unwrap().2.clone());
        app.update(|e| e.original_result(old, Err("retired".into())));
        app.read(|e| {
            assert_eq!(e.document.visit.view, 8);
            assert!(Arc::ptr_eq(&image, &e.original.settled.as_ref().unwrap().2));
            assert!(e.original.refinement_error.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn leaving_a_purpose_releases_input_before_worker_dispatch() {
    let (e, scheduler) = editor();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let source = app.enter(|e, access| demand(e, 0., access));
        app.pump();
        app.update(Editor::leave_original);
        app.pump();
        assert!(source.upgrade().is_none());
        app.read(|e| {
            assert!(e.original.refining.is_none());
            assert!(e.document.visit.grade.task.is_none());
        });
        drop(hold);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retired_running_original_keeps_its_source_until_the_submitted_unit_returns() {
    let (e, scheduler) = editor();
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = std::sync::Mutex::new(resume);
    let submissions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = submissions.clone();
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
        let source = app.enter(|e, access| demand(e, 0., access));
        app.pump();
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        app.enter(|e, access| {
            e.replace_photo_scope(8);
            demand(e, 0.2, access);
        });
        app.pump();
        assert!(
            source.upgrade().is_some(),
            "running work still owns the native input after its UI waiter departs"
        );
        assert_eq!(
            submissions.load(Ordering::Relaxed),
            1,
            "the replacement cannot submit while the retired physical unit is held"
        );
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| e.original.settled.is_some());
        assert!(source.upgrade().is_none());
        app.read(|e| {
            assert_eq!(
                e.original.settled.as_ref().unwrap().0,
                e.original.revision()
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn malformed_original_request_latches_error_and_retires_its_demand() {
    let (e, scheduler) = editor();
    with_application(e, |app| {
        app.enter(|e, access| {
            let source = source::Context {
                source: crate::editor::tests::developed_fixture(),
                correction: None,
            };
            let area = source.source.area();
            let composition =
                crate::geometry::Composition::new(area, Default::default(), Default::default())
                    .unwrap();
            let input = ProcessingInput::from(ProcessingParameters::source_inspection(0.).unwrap());
            let mut view = ViewRequest {
                area,
                output: [16, 12],
                frame: None,
            };
            view.area.width = u32::MAX;
            e.observe_original(
                Some(OriginalPurpose::Tool),
                (
                    input.signature(),
                    OutputTarget::Srgb,
                    Some(view),
                    composition,
                ),
            );
            e.original.last_edit = Instant::now() - Duration::from_secs(1);
            e.request_original(
                &input,
                OutputTarget::Srgb,
                &egui::Context::default(),
                source,
                access,
            );
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.original.refinement_error.is_some()
        });
        app.read(|e| {
            assert!(e.original.settled.is_none());
            assert!(e.original.refining.is_none());
            assert!(e.status.starts_with("Original view failed:"));
        });
    });
    scheduler.close_and_wait().unwrap();
}
