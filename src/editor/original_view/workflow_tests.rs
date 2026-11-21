use super::*;
use crate::application::test_support::with_application;
use crate::editor::OutputTarget;
use crate::editor::ViewRequest;
use crate::editor::source_loading;
use crate::editor::tests::fixture;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::test_inputs::AuthoredPhotograph;
use std::time::{Duration, Instant};

fn editor() -> (Editor, source_loading::SourceLoader) {
    let mut e = fixture();
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        crate::gpu::submission::request_device(&adapter, &Default::default())
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
        correction: Default::default(),
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
    let input = AuthoredPhotograph::from(ProcessingParameters::source_inspection(ev).unwrap());
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

    e.observe_original(Some(wanted));
    e.active_canvas_mut().original.last_edit = Instant::now() - Duration::from_secs(1);
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
    original.observe(wanted);
    original.last_edit = Instant::now() - Duration::from_secs(1);
    original.request_with(
        e.source_services.clone().unwrap(),
        &egui::Context::default(),
        cx,
    );
    resource
}

fn held_correction(
    editor: &mut Editor,
    demand: &mut Demand,
) -> (
    crate::editor::correction::Resources,
    futures_channel::oneshot::Receiver<()>,
    futures_channel::oneshot::Sender<()>,
) {
    editor
        .document
        .register(crate::project::Source {
            id: SourceId(0),
            path: demand.source.source.path().to_path_buf(),
            development: crate::project::SourceDevelopment::from_metadata(
                &demand.source.source.metadata(),
                demand.source.source.white_balance(),
            )
            .unwrap(),
            development_mode: crate::project::DevelopmentMode::Unity,
            repairs: Default::default(),
            adjustment_layers: None,
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    let area = demand.source.source.area();
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![glam::DVec3::new(1.4, 0.7, 1.2); 4],
        )
        .unwrap(),
    );
    editor
        .document
        .project
        .set_flat_field(Some(crate::project::FlatFieldSettings::new(
            "reference.dng",
            map,
        )));
    let (started, observed) = futures_channel::oneshot::channel();
    let (release, resume) = futures_channel::oneshot::channel();
    let resources = editor.correction().after(async move {
        started.send(()).unwrap();
        resume
            .await
            .expect("release captured correction preparation");
    });
    editor.document.correction = resources.clone().into();
    demand.source.correction = resources.clone();
    (resources, observed, release)
}

#[test]
fn original_request_awaits_its_captured_correction_without_an_inspector_observer() {
    let (mut editor, scheduler) = editor();
    let (mut wanted, _) = demand_input(0.);
    let (resources, started, release) = held_correction(&mut editor, &mut wanted);
    let services = editor.source_services.clone().unwrap();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.observe_original(Some(wanted.clone()));
            editor.document.visit.canvas.original.last_edit =
                Instant::now() - Duration::from_secs(1);
            editor.request_original(&egui::Context::default(), cx);
        });
        let waiting = app
            .access()
            .spawn(move |_| async move { started.await.unwrap() })
            .unwrap();
        app.run_until(Duration::from_secs(15), |_| waiting.is_finished());
        app.read(|editor| {
            let original = &editor.document.visit.canvas.original;
            assert!(resources.pending());
            assert!(original.refining() && original.settled().is_none());
            assert!(!original.failed());
        });
        app.update(|editor| editor.document.project.set_flat_field(None));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.canvas.original.settled().is_some()
        });
        let correction = pollster::block_on(resources.acquire()).unwrap().unwrap();
        let corrected = app.read(|editor| {
            let original = &editor.document.visit.canvas.original;
            assert!(!original.failed() && !original.refining());
            assert!(Arc::ptr_eq(
                &original
                    .wanted
                    .as_ref()
                    .unwrap()
                    .source
                    .correction
                    .gpu()
                    .unwrap()
                    .unwrap(),
                &correction.gpu,
            ));
            original.settled().unwrap().1
        });
        let corrected =
            crate::viewer::readback::image_pixels(&services.device, &services.queue, &corrected);
        wanted.source.correction = Default::default();
        app.enter(|editor, cx| {
            editor.observe_original(Some(wanted.clone()));
            editor.document.visit.canvas.original.last_edit =
                Instant::now() - Duration::from_secs(1);
            editor.request_original(&egui::Context::default(), cx);
        });
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.canvas.original.settled().is_some()
        });
        let uncorrected =
            app.read(|editor| editor.document.visit.canvas.original.settled().unwrap().1);
        let uncorrected =
            crate::viewer::readback::image_pixels(&services.device, &services.queue, &uncorrected);
        assert!(
            corrected
                .iter()
                .chain(&uncorrected)
                .flatten()
                .all(|value| value.is_finite())
        );
        assert!(
            corrected
                .iter()
                .zip(&uncorrected)
                .any(|(corrected, uncorrected)| corrected[..3] != uncorrected[..3]),
            "the completed request must actually apply its captured RGB gains"
        );
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn replacing_an_original_request_does_not_wait_for_the_retired_corrections_preparation() {
    let (mut editor, scheduler) = editor();
    let (mut wanted, _) = demand_input(0.);
    let (resources, started, release) = held_correction(&mut editor, &mut wanted);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.observe_original(Some(wanted));
            editor.document.visit.canvas.original.last_edit =
                Instant::now() - Duration::from_secs(1);
            editor.request_original(&egui::Context::default(), cx);
        });
        let waiting = app
            .access()
            .spawn(move |_| async move { started.await.unwrap() })
            .unwrap();
        app.run_until(Duration::from_secs(15), |_| waiting.is_finished());
        app.enter(|editor, cx| {
            editor.document.project.set_flat_field(None);
            demand(editor, 0.2, cx);
        });
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.canvas.original.settled().is_some()
        });
        let completed = app.read(|editor| {
            let original = &editor.document.visit.canvas.original;
            assert!(
                resources.pending(),
                "replacement must finish before the old map is released"
            );
            assert!(!original.failed() && !original.refining());
            assert!(
                original
                    .wanted
                    .as_ref()
                    .unwrap()
                    .source
                    .correction
                    .map()
                    .is_none()
            );
            original.settled().unwrap().1
        });
        release.send(()).unwrap();
        let retired = app
            .access()
            .spawn(move |_| async move { resources.acquire().await.unwrap() })
            .unwrap();
        app.run_until(Duration::from_secs(15), |_| retired.is_finished());
        app.pump();
        app.read(|editor| {
            let original = &editor.document.visit.canvas.original;
            assert!(!original.failed());
            assert!(Arc::ptr_eq(&original.settled().unwrap().1, &completed));
        });
    });
    scheduler.close_and_wait().unwrap();
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
            e.document.visit.canvas.original.settled().is_some()
        });
        app.read(|e| {
            assert!(e.document.flat_edit().is_some());
            let state = e.document.flat_edit().unwrap();
            assert!(state.view.test_original().settled().is_none());
            assert!(!state.view.test_original().refining());
            assert!(!e.document.visit.canvas.original.refining());
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
            e.document.visit.canvas.original.settled().is_some()
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
            e.document.visit.canvas.original.settled().is_some()
        });
        app.read(|e| {
            let (view, image) = e.document.visit.canvas.original.settled().unwrap();
            assert_eq!(view.output, [16, 12]);
            assert_eq!(image.processing_byte_len(), (0, 0));
            assert!(!e.document.visit.canvas.original.refining());
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
        app.read(|e| assert!(e.document.visit.canvas.original.settled().is_none()));
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.original.settled().is_some()
        });
        app.read(|e| {
            assert!(
                e.document
                    .visit
                    .canvas
                    .original
                    .wanted
                    .as_ref()
                    .unwrap()
                    .capture
                    == crate::gpu::photograph::request::PhotographRequest::Fixture(
                        AuthoredPhotograph::from(
                            ProcessingParameters::source_inspection(0.39).unwrap()
                        )
                    )
            );
            assert!(!e.document.visit.canvas.original.refining());
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
            (e.document.visit.canvas.original.continuation(), resource)
        });
        app.pump();
        app.update(|e| e.replace_photo_scope(8));
        app.pump();
        assert!(resource.upgrade().is_none());
        app.enter(|e, access| {
            demand(e, 0.2, access);
            let current = &e.document.visit.canvas.original;
            assert_ne!(departed, current.continuation());
            assert!(departed.receive(Err("retired".into())).is_none());
            assert!(current.refining() && !current.failed() && current.settled().is_none());
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.original.settled().is_some()
        });
        let image = app.read(|e| e.document.visit.canvas.original.settled().unwrap().1);
        assert!(departed.receive(Err("retired".into())).is_none());
        app.read(|e| {
            assert_eq!(e.document.visit.view, 8);
            assert!(Arc::ptr_eq(
                &image,
                &e.document.visit.canvas.original.settled().unwrap().1
            ));
            assert!(!e.document.visit.canvas.original.failed());
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
        app.update(|editor| editor.observe_original(None));
        app.pump();
        assert!(source.upgrade().is_none());
        app.read(|e| {
            assert!(!e.document.visit.canvas.original.refining());
            assert!(
                e.document
                    .visit
                    .canvas
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
                correction: Default::default(),
            };
            let area = source.source.area();
            let composition =
                crate::geometry::Composition::new(area, Default::default(), Default::default())
                    .unwrap();
            let input =
                AuthoredPhotograph::from(ProcessingParameters::source_inspection(0.).unwrap());
            let mut view = ViewRequest {
                area,
                output: [16, 12],
                frame: None,
            };
            view.area.width = u32::MAX;
            e.observe_original(Some(Demand::fixture(
                input,
                OutputTarget::Srgb,
                Some(view),
                composition,
                source,
            )));
            e.document.visit.canvas.original.last_edit = Instant::now() - Duration::from_secs(1);
            e.request_original(&egui::Context::default(), access);
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.canvas.original.failed()
        });
        app.read(|e| {
            assert!(e.document.visit.canvas.original.settled().is_none());
            assert!(!e.document.visit.canvas.original.refining());
            assert!(e.status.starts_with("Original view failed:"));
        });
    });
    scheduler.close_and_wait().unwrap();
}
