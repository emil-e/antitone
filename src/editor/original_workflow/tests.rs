use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        OriginalPurpose, OutputTarget, ProcessingInput, ProcessingParameters, ViewRequest,
        source_loading, tests::fixture,
    },
};
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

fn demand(
    e: &mut Editor,
    ev: f64,
    cx: &UiContext<'_, Editor>,
) -> std::sync::Weak<crate::scan::DevelopedSource> {
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
    e.active_canvas_mut().grade.original.last_edit = Instant::now() - Duration::from_secs(1);
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
            e.document.visit.canvas.grade.original.settled.is_some()
        });
        app.read(|e| {
            assert!(e.document.flat_edit().is_some());
            assert!(e.active_canvas().grade.original.settled.is_none());
            assert!(e.active_canvas().grade.original.refining.is_none());
            assert!(e.document.visit.canvas.grade.original.refining.is_none());
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
        let temporary = app.enter(|e, cx| demand(e, 0.5, cx));
        app.pump();
        app.update(Editor::finish_flat_edit);
        app.pump();
        assert!(temporary.upgrade().is_none());
        assert!(working.upgrade().is_some());
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled.is_some()
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
            e.document.visit.canvas.grade.original.settled.is_some()
        });
        app.read(|e| {
            let (revision, view, image) = e
                .document
                .visit
                .canvas
                .grade
                .original
                .settled
                .as_ref()
                .unwrap();
            assert_eq!(*revision, e.document.visit.canvas.grade.original.revision());
            assert_eq!(view.output, [16, 12]);
            assert_eq!(image.processing_byte_len(), (0, 0));
            assert!(e.document.visit.canvas.grade.original.refining.is_none());
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
        app.read(|e| assert!(e.document.visit.canvas.grade.original.settled.is_none()));
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled.is_some()
        });
        app.read(|e| {
            assert_eq!(
                e.document
                    .visit
                    .canvas
                    .grade
                    .original
                    .settled
                    .as_ref()
                    .unwrap()
                    .0,
                e.document.visit.canvas.grade.original.revision()
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
        let (old_generation, old, resource) = app.enter(|e, access| {
            let resource = demand(e, 0., access);
            let original = &e.document.visit.canvas.grade.original;
            (original.generation.clone(), original.revision(), resource)
        });
        app.pump();
        app.update(|e| e.replace_photo_scope(8));
        app.pump();
        assert!(resource.upgrade().is_none());
        app.enter(|e, access| {
            demand(e, 0.2, access);
            let current = &e.document.visit.canvas.grade.original;
            assert_eq!(
                old,
                current.revision(),
                "exercise generation identity, not revision inequality"
            );
            assert!(!Arc::ptr_eq(&old_generation, &current.generation));
            e.receive_original_for(&old_generation, old, Err("retired".into()));
            let current = &e.document.visit.canvas.grade.original;
            assert!(current.refining.is_some());
            assert!(current.refinement_error.is_none());
            assert!(current.settled.is_none());
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.grade.original.settled.is_some()
        });
        let image = app.read(|e| {
            e.document
                .visit
                .canvas
                .grade
                .original
                .settled
                .as_ref()
                .unwrap()
                .2
                .clone()
        });
        app.update(|e| e.receive_original_for(&old_generation, old, Err("retired".into())));
        app.read(|e| {
            assert_eq!(e.document.visit.view, 8);
            assert!(Arc::ptr_eq(
                &image,
                &e.document
                    .visit
                    .canvas
                    .grade
                    .original
                    .settled
                    .as_ref()
                    .unwrap()
                    .2
            ));
            assert!(
                e.document
                    .visit
                    .canvas
                    .grade
                    .original
                    .refinement_error
                    .is_none()
            );
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
            assert!(e.document.visit.canvas.grade.original.refining.is_none());
            assert!(e.document.visit.canvas.grade.task.is_none());
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
                (
                    input.signature(),
                    OutputTarget::Srgb,
                    Some(view),
                    composition,
                ),
            );
            e.document.visit.canvas.grade.original.last_edit =
                Instant::now() - Duration::from_secs(1);
            e.request_original(
                &input,
                OutputTarget::Srgb,
                &egui::Context::default(),
                source,
                access,
            );
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document
                .visit
                .canvas
                .grade
                .original
                .refinement_error
                .is_some()
        });
        app.read(|e| {
            assert!(e.document.visit.canvas.grade.original.settled.is_none());
            assert!(e.document.visit.canvas.grade.original.refining.is_none());
            assert!(e.status.starts_with("Original view failed:"));
        });
    });
    scheduler.close_and_wait().unwrap();
}
