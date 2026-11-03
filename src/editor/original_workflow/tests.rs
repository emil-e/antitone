use super::*;
use crate::application::test_support::with_application;
use crate::editor::OriginalPurpose;
use crate::editor::OutputTarget;
use crate::editor::ViewRequest;
use crate::editor::source_loading;
use crate::editor::tests::fixture;
use crate::gpu::input::ProcessingInput;
use crate::gpu::parameters::ProcessingParameters;
use std::time::{Duration, Instant};

fn editor() -> (Editor, source_loading::SourceLoader) {
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
    let scheduler = source_loading::SourceLoader::new();
    e.source_services = Some(source::Services {
        device,
        queue,
        developments: scheduler.clone(),
        renderers: Arc::default(),
    });
    (e, scheduler)
}

fn demand_input(ev: f64) -> (Demand, std::sync::Weak<crate::scan::DevelopedSource>) {
    let source = source::Context {
        reference: None,
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
    (
        Demand::fixture(input, OutputTarget::Srgb, Some(view), composition, source),
        resource,
    )
}

fn demand(
    e: &mut Editor,
    ev: f64,
    cx: &UiContext<'_, Editor>,
) -> std::sync::Weak<crate::scan::DevelopedSource> {
    let (wanted, resource) = demand_input(ev);
    e.observe_grade(None);
    e.observe_original(Some(OriginalPurpose::Tool), Some(wanted));
    e.active_canvas_mut().grade.original.last_edit = Instant::now() - Duration::from_secs(1);
    e.request_original(&egui::Context::default(), cx);
    resource
}

fn reference_demand(
    e: &mut Editor,
    ev: f64,
    cx: &UiContext<'_, Editor>,
) -> std::sync::Weak<crate::scan::DevelopedSource> {
    let (wanted, resource) = demand_input(ev);
    let mut state = e.document.flat_edit_mut().unwrap();
    let original = state.view.test_original_mut();
    original.observe(OriginalPurpose::Tool, wanted);
    original.last_edit = Instant::now() - Duration::from_secs(1);
    original.request_with(
        e.source_services.clone().unwrap(),
        e.submissions.clone(),
        &egui::Context::default(),
        cx,
    );
    resource
}

#[test]
fn working_reconstruction_can_complete_while_the_reference_view_is_active() {
    let (mut e, scheduler) = editor();
    crate::editor::tests::add_ready_fixture(&mut e);
    let reference = e.document.visit.canvas.ready.as_ref().unwrap().clone();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        app.enter(|e, cx| {
            demand(e, 0., cx);
        });
        app.pump();
        app.update(|e| crate::editor::tests::paused_reference_fixture(e, reference, None));
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled().is_some()
        });
        app.read(|e| {
            assert!(e.document.flat_edit().is_some());
            let state = e.document.flat_edit().unwrap();
            assert!(state.view.test_original().settled().is_none());
            assert!(!state.view.test_original().refining());
            assert!(!e.document.visit.canvas.grade.original.refining());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn ending_reference_view_retires_only_its_reconstruction_not_the_working_views() {
    let (mut e, scheduler) = editor();
    crate::editor::tests::add_ready_fixture(&mut e);
    let reference = e.document.visit.canvas.ready.as_ref().unwrap().clone();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let working = app.enter(|e, cx| demand(e, 0., cx));
        app.pump();
        app.update(|e| crate::editor::tests::paused_reference_fixture(e, reference, None));
        let temporary = app.enter(|e, cx| reference_demand(e, 0.5, cx));
        app.pump();
        app.update(Editor::finish_flat_edit);
        app.pump();
        assert!(temporary.upgrade().is_none());
        assert!(working.upgrade().is_some());
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled().is_some()
        });
        app.read(|e| assert!(e.document.flat_edit().is_none()));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn original_reconstruction_finishes_without_backend_or_another_paint() {
    let (e, scheduler) = editor();
    with_application(e, |app| {
        app.enter(|e, access| {
            demand(e, 0., access);
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled().is_some()
        });
        app.read(|e| {
            let (view, image) = e.document.visit.canvas.grade.original.settled().unwrap();
            assert_eq!(view.output, [16, 12]);
            assert_eq!(image.processing_byte_len(), (0, 0));
            assert!(!e.document.visit.canvas.grade.original.refining());
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
        app.read(|e| assert!(e.document.visit.canvas.grade.original.settled().is_none()));
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled().is_some()
        });
        app.read(|e| {
            assert!(
                e.document
                    .visit
                    .canvas
                    .grade
                    .original
                    .wanted
                    .as_ref()
                    .unwrap()
                    .capture
                    == crate::gpu::view::input::ViewInput::Processing(ProcessingInput::from(
                        ProcessingParameters::source_inspection(0.39).unwrap()
                    ))
            );
            assert!(!e.document.visit.canvas.grade.original.refining());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn leaving_a_photo_retires_queued_original_work_and_cannot_publish_into_a_revisit() {
    let (e, scheduler) = editor();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        let (departed, resource) = app.enter(|e, access| {
            let resource = demand(e, 0., access);
            (
                e.document.visit.canvas.grade.original.continuation(),
                resource,
            )
        });
        app.pump();
        app.update(|e| e.replace_photo_scope(8));
        app.pump();
        assert!(resource.upgrade().is_none());
        app.enter(|e, access| {
            demand(e, 0.2, access);
            let current = &e.document.visit.canvas.grade.original;
            assert_ne!(departed, current.continuation());
            assert!(departed.receive(Err("retired".into())).is_none());
            assert!(current.refining() && !current.failed() && current.settled().is_none());
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled().is_some()
        });
        let image = app.read(|e| e.document.visit.canvas.grade.original.settled().unwrap().1);
        assert!(departed.receive(Err("retired".into())).is_none());
        app.read(|e| {
            assert_eq!(e.document.visit.view, 8);
            assert!(Arc::ptr_eq(
                &image,
                &e.document.visit.canvas.grade.original.settled().unwrap().1
            ));
            assert!(!e.document.visit.canvas.grade.original.failed());
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
            assert!(!e.document.visit.canvas.grade.original.refining());
            assert!(
                e.document
                    .visit
                    .canvas
                    .grade
                    .original
                    .task
                    .as_ref()
                    .is_none_or(|task| task.is_finished())
            );
        });
        drop(hold);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn malformed_original_request_latches_error_and_retires_its_demand() {
    let (e, scheduler) = editor();
    with_application(e, |app| {
        app.enter(|e, access| {
            let source = source::Context {
                reference: None,
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
                Some(Demand::fixture(
                    input,
                    OutputTarget::Srgb,
                    Some(view),
                    composition,
                    source,
                )),
            );
            e.document.visit.canvas.grade.original.last_edit =
                Instant::now() - Duration::from_secs(1);
            e.request_original(&egui::Context::default(), access);
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.failed()
        });
        app.read(|e| {
            assert!(e.document.visit.canvas.grade.original.settled().is_none());
            assert!(!e.document.visit.canvas.grade.original.refining());
            assert!(e.status.starts_with("Original view failed:"));
        });
    });
    scheduler.close_and_wait().unwrap();
}
