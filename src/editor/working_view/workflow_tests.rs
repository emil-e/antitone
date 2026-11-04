use super::*;
use crate::gpu::photograph::request::PhotographRequest;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;
use crate::{
    application::test_support::with_application,
    geometry::{Composition, FramePoint},
};
use std::sync::atomic::Ordering;

mod coalescing;
mod inputs;

fn request_working(
    e: &mut Editor,
    spec: SourceRequest,
    pixels: source::Context,
    cx: &UiContext<'_, Editor>,
) {
    e.request_working(Demand::fixture(spec, pixels), cx);
}

fn fixture() -> (
    Editor,
    source::Context,
    SourceRequest,
    source_loading::SourceLoader,
) {
    let mut e = crate::editor::tests::fixture();
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
    let spec = SourceRequest::inspection_fixture(frame, scan.camera_to_ap1, None)
        .unwrap()
        .with_identity(crate::gpu::source::identity::Identity::camera(
            scan.source().identity(),
        ));
    let input = source::Context {
        reference: None,
        source: scan.source().clone(),
        correction: None,
    };
    let scheduler = source_loading::SourceLoader::new();
    e.source_services = Some(source::Services {
        device: device.clone(),
        queue: queue.clone(),
        developments: scheduler.clone(),
        renderers: Arc::default(),
    });
    e.document.visit.working_view = WorkingView::new(device, queue.clone());
    e.document
        .visit
        .working_view
        .mailbox
        .pressure(memory::Pressure::Normal);
    e.source_epoch.store(e.id, Ordering::Relaxed);
    (e, input, spec, scheduler)
}

fn positive_spec(frame: FrameRaster, input: &source::Context, sigma: f64) -> SourceRequest {
    let scan = input.scan().unwrap();
    SourceRequest::new(
        frame,
        crate::processing::InputSettings::Digital,
        scan.positive_matrix(&crate::processing::InputSettings::Digital)
            .unwrap(),
        None,
    )
    .unwrap()
    .with_pipeline(crate::pixel_pipeline::PixelPipeline {
        stages: vec![crate::pixel_pipeline::PixelStage::new(
            crate::image_mtf::ImageMtfParameters { sigma },
        )],
    })
    .unwrap()
    .with_identity(crate::gpu::source::identity::Identity::camera(
        input.source.identity(),
    ))
}

#[test]
fn parked_support_growth_rejects_stale_layouts_and_smaller_demand_reuses_acquired_neighbours() {
    let (e, input, inspection, scheduler) = fixture();
    let mut frame = inspection.frame();
    frame.output = std::array::from_fn(|i| (frame.max.0[i] - frame.min.0[i]).ceil() as u32);
    let small = positive_spec(frame, &input, 0.25);
    let queued = positive_spec(frame, &input, 0.5);
    let latest = positive_spec(frame, &input, 0.75);
    assert!(
        small.processing_plan(latest.plan.pipeline()).is_err(),
        "fixture must actually increase required raster support"
    );
    assert!(
        small.same_source(&latest),
        "support is not upstream content identity"
    );
    with_application(e, |app| {
        app.enter(|e, access| request_working(e, small.clone(), input.clone(), access));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let old = app.read(|e| e.document.visit.working_view.source().unwrap().clone());
        let hold = crate::background::testing::pause();
        app.enter(|e, access| request_working(e, queued.clone(), input.clone(), access));
        app.pump();
        app.enter(|e, access| request_working(e, latest.clone(), input.clone(), access));
        app.pump();
        app.update(|e| {
            assert!(!e.document.visit.working_view.support_ready());
            let underlay = e.document.visit.working_view.underlay().unwrap();
            assert!(
                !Demand::fixture(latest.clone(), input.clone())
                    .can_use(underlay)
                    .unwrap_or(false),
                "retained old pixels cannot supply the new neighbourhood"
            );
            assert!(
                e.document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .processing(&Default::default())
                    .unwrap()
                    == small.processing_fixture(),
                "fallback presentation retains its own processing, not the newer pipeline"
            );
            assert!(e.document.visit.working.task.is_some());
            e.document.visit.working_view.acquired(Completion {
                owner: Arc::downgrade(&e.document.visit.working_view.mailbox),
                demand: Completion::fixture_demand(&e.document.visit.working_view, queued.clone()),
                result: Ok(Arc::new(Frame {
                    source: old.clone(),
                    underlay: None,
                    request: Arc::new(Demand::fixture(small.clone(), input.clone())),
                    processed: Vec::new(),
                })),
            });
            assert!(
                e.document.visit.working_view.error.is_none(),
                "obsolete completion cannot fail current demand"
            );
            // Publication must validate actual coverage as well as the completion ticket.
            e.document.visit.working_view.acquired(Completion {
                owner: Arc::downgrade(&e.document.visit.working_view.mailbox),
                demand: Completion::fixture_demand(&e.document.visit.working_view, latest.clone()),
                result: Ok(Arc::new(Frame {
                    source: old.clone(),
                    underlay: None,
                    request: Arc::new(Demand::fixture(small.clone(), input.clone())),
                    processed: Vec::new(),
                })),
            });
            assert!(!e.document.visit.working_view.support_ready());
            assert!(e.document.visit.working_view.error.is_some());
            assert!(Arc::ptr_eq(
                &old,
                e.document.visit.working_view.source().unwrap()
            ));
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let acquired = app.read(|e| {
            assert!(e.document.visit.working_view.error.is_none());
            assert!(
                e.document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .satisfies(&Demand::fixture(latest.clone(), input.clone()))
            );
            assert!(
                e.document
                    .visit
                    .working_view
                    .underlay()
                    .is_some_and(|underlay| Demand::fixture(latest.clone(), input.clone())
                        .can_use(underlay)
                        .unwrap())
            );
            e.document.visit.working_view.source().unwrap().clone()
        });
        app.enter(|e, access| request_working(e, small.clone(), input, access));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working.task.is_none()
        });
        app.read(|e| {
            assert!(e.document.visit.working_view.support_ready());
            assert!(
                acquired.same_pixels_as(e.document.visit.working_view.source().unwrap()),
                "the new downstream calculation must reuse the sufficient native value"
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn support_growth_during_input_submission_reuses_source_and_publishes_only_latest_support() {
    let (e, _, _, scheduler) = fixture();
    let fixture = crate::dng_fixture::Fixture {
        side: 768,
        ramp: true,
        ..Default::default()
    }
    .write();
    let scan = Scan::open(fixture.path(), None).unwrap();
    let composition = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([200., 220.]),
        FramePoint([328., 316.]),
        [64, 48],
    )
    .unwrap();
    let input = source::Context {
        reference: None,
        source: scan.source().clone(),
        correction: None,
    };
    let small = positive_spec(frame, &input, 0.25);
    let latest = positive_spec(frame, &input, 0.75);
    let services = e.source_services.as_ref().unwrap();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let kernels = pollster::block_on(ViewKernels::new(&services.device)).unwrap();
    let count_work = |spec: &SourceRequest| {
        let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let observed = count.clone();
        let gate = crate::gpu::submission::SubmissionGate::default();
        gate.observe_submissions(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        let scope = renderer.evaluation().test_materializations(false);
        let active = pollster::block_on(renderer.evaluation().prepare_input(
            &services.queue,
            &gate,
            &scope,
            crate::gpu::evaluation::camera_source(&scan).unwrap(),
            || false,
        ))
        .unwrap()
        .unwrap();
        let input_submissions = count.load(Ordering::Relaxed);
        let images = pollster::block_on(ViewSource::acquire_originals_fixture(
            &services.device,
            &services.queue,
            &gate,
            renderer.clone(),
            kernels.clone(),
            &scan,
            None,
            vec![spec.clone(), spec.underlay().unwrap()],
            &scope,
            || false,
        ))
        .unwrap()
        .unwrap();
        let processing = spec.processing_fixture();
        let requests = images
            .iter()
            .map(|image| {
                let (root, _) = crate::gpu::view::processing::ProcessedView::definition(
                    &services.device.limits(),
                    &image.spec,
                    image.definition.clone(),
                    &processing,
                )
                .unwrap();
                crate::gpu::view::processing::ProcessedView::request(renderer.evaluation(), root)
                    .unwrap()
            })
            .collect();
        let processed = pollster::block_on(futures_util::future::try_join_all(
            renderer
                .evaluation()
                .materialize_many(&services.queue, &gate, &scope, requests)
                .unwrap(),
        ))
        .unwrap();
        let total = count.load(Ordering::Relaxed);
        drop((processed, images, active));
        scope.clear();
        (input_submissions, total - input_submissions)
    };
    assert!(
        count_work(&small).1 > 0,
        "control: neutral production must follow input preparation"
    );
    let (input_work, latest_work) = count_work(&latest);
    let expected = input_work + latest_work;
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
        app.enter(|e, access| request_working(e, small.clone(), input.clone(), access));
        // Submission observers do not wake the application executor; advance its preparation awaits.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            app.pump();
            match starts.recv_timeout(Duration::from_millis(1)) {
                Ok(()) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => assert!(
                    std::time::Instant::now() < deadline,
                    "submission did not start"
                ),
                Err(error) => panic!("submission observer disconnected: {error}"),
            }
        }
        app.enter(|e, access| request_working(e, latest.clone(), input.clone(), access));
        app.pump();
        app.read(|e| {
            assert!(!e.document.visit.working_view.support_ready());
            let state = e.document.visit.working_view.mailbox.0.lock().unwrap();
            assert!(
                state.completed.is_none() && e.document.visit.working_view.source().is_none(),
                "partially acquired buffers must remain private"
            );
            assert!(
                e.document.visit.working.task.is_some(),
                "latest support remains pending while the previous submission is parked"
            );
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            assert!(
                e.document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .satisfies(&Demand::fixture(latest.clone(), input.clone()))
            );
            assert!(
                e.document
                    .visit
                    .working_view
                    .underlay()
                    .is_some_and(|underlay| Demand::fixture(latest.clone(), input.clone())
                        .can_use(underlay)
                        .unwrap())
            );
            assert!(e.document.visit.working.task.is_none());
        });
    });
    // Input preparation remains useful across layout changes. The obsolete calculation
    // must not run; compare with measured input plus complete latest work, not fixed tiling.
    assert_eq!(count.load(Ordering::Relaxed), expected);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn departed_working_owner_cannot_publish_equal_captured_pixels_or_fail_a_revisit() {
    let (e, input, spec, scheduler) = fixture();
    with_application(e, |app| {
        app.enter(|e, cx| request_working(e, spec.clone(), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let (departed, frame) = app.read(|e| {
            (
                Arc::downgrade(&e.document.visit.working_view.mailbox),
                e.document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone(),
            )
        });
        app.update(|e| {
            e.replace_photo_scope(e.id);
            e.document
                .visit
                .working_view
                .request(Demand::fixture(spec.clone(), input.clone()));
            assert!(!e.document.visit.working_view.support_ready());
            e.document.visit.working_view.acquired(Completion {
                owner: departed.clone(),
                demand: Completion::fixture_demand(&e.document.visit.working_view, spec.clone()),
                result: Ok(frame.clone()),
            });
            assert!(
                e.document.visit.working_view.source().is_none(),
                "even a complete matching calculation must arrive through its live owner"
            );
            e.document.visit.working_view.acquired(Completion {
                owner: departed.clone(),
                demand: Completion::fixture_demand(&e.document.visit.working_view, spec.clone()),
                result: Err("departed failure".into()),
            });
            assert!(e.document.visit.working_view.error.is_none());
        });
        app.enter(|e, cx| e.acquire_working(cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.update(|e| {
            let accepted = e
                .document
                .visit
                .working_view
                .frame
                .as_ref()
                .unwrap()
                .clone();
            assert!(accepted.source.spec == frame.source.spec);
            e.document.visit.working_view.acquired(Completion {
                owner: departed,
                demand: Completion::fixture_demand(&e.document.visit.working_view, spec),
                result: Err("late departed failure".into()),
            });
            assert!(e.document.visit.working_view.error.is_none());
            assert!(Arc::ptr_eq(
                e.document.visit.working_view.frame.as_ref().unwrap(),
                &accepted
            ));
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn physical_support_completes_without_backend_or_another_paint() {
    let (e, input, spec, scheduler) = fixture();
    with_application(e, |app| {
        app.enter(|e, access| {
            request_working(e, spec.clone(), input, access);
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            assert!(e.document.visit.working_view.source().unwrap().spec == spec);
            assert!(e.document.visit.working_view.underlay().is_some());
            assert!(e.document.visit.working.task.is_none());
            assert!(
                e.document.visit.working_view.displayed.is_none(),
                "colour capture remains frame-bound"
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn recipe_request_produces_complete_graph_values_and_presents_them_without_recompiling_the_photograph()
 {
    let mut e = crate::editor::tests::authored_fixture(&["patches.dng"]);
    e.set_dmin(Some(DVec3::splat(0.8)));
    e.set_dmax(Some(DVec3::splat(0.2)));
    e.document.roll.exposure = 1.;
    let recipe = Arc::new(
        e.view_recipe(
            e.input(),
            e.document.roll,
            e.document.visit.adjustment_layer_settings,
        )
        .unwrap(),
    );
    let pixels = e.source_context().unwrap();
    let device = e.document.visit.working_view.device().unwrap().clone();
    let input = AuthoredPhotograph::from_recipe(&device.limits(), &recipe).unwrap();
    let mut frame = FrameRaster::full(recipe.composition).unwrap();
    frame.output = [32, 24];
    let spec = SourceRequest::for_processing(frame, &input).unwrap();
    let demand = Demand {
        spec,
        pixels,
        capture: PhotographRequest::Recipe(recipe.clone()),
    };
    with_application(e, |app| {
        app.enter(|e, cx| e.request_working(demand, cx));
        app.update(|e| e.document.roll.exposure = 2.);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.update(|e| {
            let working = &mut e.document.visit.working_view;
            let ready = working.frame.as_ref().unwrap().clone();
            let PhotographRequest::Recipe(captured) = &ready.request.capture else {
                panic!("recipe capture was replaced");
            };
            assert!(Arc::ptr_eq(captured, &recipe));
            assert_eq!(ready.processed.len(), 2);
            let mut different = (*recipe).clone();
            different.roll.exposure = 2.;
            let changed = AuthoredPhotograph::from_recipe(&device.limits(), &different).unwrap();
            for value in &ready.processed {
                assert!(matches!(
                    value.pixels,
                    crate::gpu::evaluation::BufferImage::Produced(_)
                ));
                assert!(
                    value.stages.is_none(),
                    "publication owns a completed value, not unsubmitted grading work"
                );
                assert!(value.matches(&device.limits(), &input));
                assert!(
                    !value.matches(&device.limits(), &changed),
                    "the requested snapshot must not read subsequent document edits"
                );
            }
            working
                .render(&input, OutputTarget::Srgb, &egui::Context::default())
                .unwrap();
            assert!(
                working
                    .rendered
                    .as_ref()
                    .unwrap()
                    .uses_processed(&ready.processed[0])
            );
            assert!(
                working
                    .underlay_rendered
                    .as_ref()
                    .unwrap()
                    .uses_processed(&ready.processed[1])
            );
        });
        let (mut wanted, context) = app.read(|e| {
            let ready = e.document.visit.working_view.frame.as_ref().unwrap();
            (
                (*ready.request).clone(),
                ready.source.renderer.evaluation.clone(),
            )
        });
        let scope = context.materializations();
        scope.set_retention_enabled(false);
        let mut edited = (*recipe).clone();
        edited.roll.exposure = 2.;
        wanted.capture = PhotographRequest::Recipe(Arc::new(edited));
        let input = wanted.processing(&device.limits()).unwrap();
        app.enter(|e, cx| e.request_working(wanted, cx));
        let definition = app.read(|e| {
            let working = &e.document.visit.working_view;
            assert!(working.support_ready());
            let value = &working.frame.as_ref().unwrap().processed[0];
            assert!(
                value.stages.is_some(),
                "control: the resident edit must create recorded work"
            );
            assert!(matches!(
                value.pixels,
                crate::gpu::evaluation::BufferImage::Produced(_)
            ));
            value.definition.clone()
        });
        assert!(
            scope
                .find_ready::<crate::gpu::evaluation::ImageLayout>(
                    definition.identity().unwrap(),
                    |_| true
                )
                .is_none()
        );
        app.update(|e| {
            e.document
                .visit
                .working_view
                .render(&input, OutputTarget::Srgb, &egui::Context::default())
                .unwrap()
        });
        let (rendered, queue, submissions) = app.read(|e| {
            (
                e.document.visit.working_view.rendered.clone().unwrap(),
                e.source_services.as_ref().unwrap().queue.clone(),
                e.submissions.clone(),
            )
        });
        rendered.ensure_submitted(&device, &queue, &submissions, &egui::Context::default());
        drop(rendered);
        let layout = definition.domain();
        let evaluation = context
            .request(
                definition.clone(),
                crate::gpu::evaluation::Region {
                    bounds: layout.finite().unwrap(),
                    grid: layout.grid,
                },
                crate::gpu::evaluation::Budget {
                    tile: [32, 24],
                    scratch_bytes: 1 << 20,
                },
            )
            .unwrap();
        let held = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &submissions,
                    &scope,
                    evaluation.clone(),
                    crate::gpu::evaluation::ImageFormat::Rgba32Float,
                    crate::gpu::evaluation::ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(held.identity, *definition.identity().unwrap());
        app.update(|e| e.document.visit.working_view.clear());
        assert!(
            scope
                .find_ready::<crate::gpu::evaluation::ImageLayout>(
                    definition.identity().unwrap(),
                    |_| true
                )
                .is_some(),
            "a separate generic consumer survives view departure"
        );
        let mut pixels = Vec::new();
        let stats = context
            .read(
                &queue,
                &submissions,
                &evaluation,
                || false,
                |_, values| {
                    pixels.extend_from_slice(values);
                    Ok(())
                },
            )
            .unwrap()
            .unwrap();
        assert_eq!(stats.source_cells, 0);
        assert_eq!(pixels.len(), 32 * 24);
        assert!(
            pixels
                .iter()
                .any(|pixel| pixel[..3].iter().any(|value| *value > 0.))
        );
        assert_eq!(context.retained_bytes(), 0);
    });
}

#[test]
fn resolved_repair_arguments_require_new_values_even_when_the_recipe_is_unchanged() {
    let mut e = crate::editor::tests::authored_fixture(&["patches.dng"]);
    e.set_dmin(Some(DVec3::splat(0.8)));
    e.set_dmax(Some(DVec3::splat(0.2)));
    let id = e.document.visit.current_source.unwrap();
    e.document
        .state
        .project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            crate::dust_repair::SupportMask::new([90, 30], [2, 2], vec![true; 4]).unwrap(),
            crate::dust_repair::RepairOrigin::Manual,
            [-40, 0],
        )
        .unwrap();
    let recipe = Arc::new(
        e.view_recipe(
            e.input(),
            e.document.roll,
            e.document.visit.adjustment_layer_settings,
        )
        .unwrap(),
    );
    assert_eq!(recipe.repairs().len(), 1);
    let pixels = e.source_context().unwrap();
    let scan = pixels.scan().unwrap();
    let source =
        crate::scan::ImageSource::new(&scan, recipe.neutral.processing_input().unwrap()).unwrap();
    let evaluated =
        Arc::new(crate::repair_source::evaluate(&source, &recipe.repairs()[0]).unwrap());
    let device = e.document.visit.working_view.device().unwrap();
    let resolved = crate::gpu::repairs::Snapshot(vec![
        crate::gpu::repairs::Patch::new(device, evaluated).unwrap(),
    ]);
    let processing = AuthoredPhotograph::from_recipe(&device.limits(), &recipe).unwrap();
    let mut frame = FrameRaster::full(recipe.composition).unwrap();
    frame.output = [32, 24];
    let initial = Demand {
        spec: SourceRequest::for_processing(frame, &processing).unwrap(),
        pixels,
        capture: PhotographRequest::Recipe(recipe),
    };
    assert!(initial.spec.repairs.0.is_empty());
    with_application(e, |app| {
        app.enter(|e, cx| e.request_working(initial, cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let old = app.read(|e| {
            e.document
                .visit
                .working_view
                .frame
                .as_ref()
                .unwrap()
                .clone()
        });
        let mut hydrated = (*old.request).clone();
        hydrated.spec = hydrated.spec.with_repairs(resolved.clone());
        assert!(old.request.capture == hydrated.capture);
        assert!(!old.satisfies(&hydrated));
        assert!(
            !old.supports(&hydrated),
            "matching intent cannot substitute missing resolved arguments"
        );
        app.enter(|e, cx| e.request_working(hydrated.clone(), cx));
        app.read(|e| assert!(!e.document.visit.working_view.support_ready()));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            let ready = e.document.visit.working_view.frame.as_ref().unwrap();
            assert!(ready.request.same_calculation(&hydrated));
            for source in std::iter::once(&ready.source).chain(ready.underlay.as_ref()) {
                let calculation = hydrated.calculation(source).unwrap();
                assert!(source.definition.same_definition(&calculation.native));
                assert!(!source.definition.same_definition(&source.basis));
            }
        });
    });
}

#[test]
fn queued_navigation_uses_latest_demand_and_reuses_native_values_after_output_changes() {
    let (e, input, mut spec, scheduler) = fixture();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        for _ in 0..40 {
            let mut moved_frame = spec.frame();
            moved_frame.min.0[0] += 1.;
            moved_frame.max.0[0] += 1.;
            spec = spec.with_frame(moved_frame).unwrap();
            app.enter(|e, access| {
                request_working(e, spec.clone(), input.clone(), access);
            });
            app.pump();
            app.read(|e| {
                assert!(!e.document.visit.working_view.support_ready());
                assert!(e.document.visit.working.task.is_some());
            });
        }
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let (neutral, before) = app.read(|e| {
            assert!(e.document.visit.working_view.source().unwrap().spec == spec);
            (
                e.document.visit.working_view.neutral_source().unwrap(),
                e.document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone(),
            )
        });
        let processing = AuthoredPhotograph::from(
            crate::gpu::parameters::ProcessingParameters::source_inspection(1.2).unwrap(),
        );
        let edited = Demand {
            spec: spec.clone(),
            pixels: input,
            capture: PhotographRequest::Fixture(processing.clone()),
        };
        assert!(
            before.request.capture != edited.capture,
            "this phase must submit genuinely changed processing"
        );
        app.enter(|e, access| {
            e.document.target = OutputTarget::Srgb;
            e.request_working(edited.clone(), access);
        });
        app.pump();
        let ui = egui::Context::default();
        let (rendered, device, queue, gate) = app.update(|e| {
            assert!(
                e.document.visit.working.task.is_none(),
                "output-only demand must not reacquire originals"
            );
            let services = e.source_services.as_ref().unwrap();
            let (device, queue, gate) = (
                services.device.clone(),
                services.queue.clone(),
                e.submissions.clone(),
            );
            let target = e.document.target;
            let working = &mut e.document.visit.working_view;
            assert!(neutral.same_pixels_as(&working.neutral_source().unwrap()));
            let frame = working.frame.as_ref().unwrap();
            assert!(frame.request.same_calculation(&edited));
            assert_eq!(before.processed.len(), frame.processed.len());
            for (old, new) in before.processed.iter().zip(&frame.processed) {
                assert!(
                    !old.definition.same_definition(&new.definition),
                    "both native consumers must apply the changed output calculation"
                );
            }
            working.render(&processing, target, &ui).unwrap();
            let rendered = working.present(spec.frame(), None).unwrap().unwrap();
            assert!(rendered.processing() == processing);
            assert_eq!(rendered.target, OutputTarget::Srgb);
            (rendered, device, queue, gate)
        });
        rendered.ensure_submitted(&device, &queue, &gate, &ui);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(!rendered.in_flight());
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retiring_running_support_keeps_its_native_lease_through_submission() {
    let (e, input, mut spec, scheduler) = fixture();
    let source = Arc::downgrade(&input.source);
    let scan = Scan::open(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let next = source::Context {
        reference: None,
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
            request_working(e, spec.clone(), input, access);
        });
        // Submission observers do not wake the application executor; advance its preparation awaits.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            app.pump();
            match starts.recv_timeout(Duration::from_millis(1)) {
                Ok(()) => break,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => assert!(
                    std::time::Instant::now() < deadline,
                    "submission did not start"
                ),
                Err(error) => panic!("submission observer disconnected: {error}"),
            }
        }
        let mut moved_frame = spec.frame();
        moved_frame.min.0[0] += 1.;
        moved_frame.max.0[0] += 1.;
        spec = spec.with_frame(moved_frame).unwrap();
        app.enter(|e, access| {
            e.replace_photo_scope(8);
            request_working(e, spec.clone(), next, access);
        });
        app.pump();
        assert!(source.upgrade().is_some());
        assert!(
            count.load(Ordering::Relaxed) > 0,
            "the source lease must cover submitted work"
        );
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while source.upgrade().is_some() {
            app.pump();
            assert!(
                std::time::Instant::now() < deadline,
                "retired physical support did not release its source"
            );
            std::thread::yield_now();
        }
        app.read(|e| {
            assert!(e.document.visit.working_view.source().unwrap().spec == spec);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn visit_retirement_drops_queued_support_and_cannot_restore_the_old_source() {
    let (e, input, spec, scheduler) = fixture();
    let source = Arc::downgrade(&input.source);
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        app.enter(|e, access| {
            request_working(e, spec, input, access);
        });
        app.pump();
        app.update(|e| e.replace_photo_scope(8));
        app.pump();
        assert!(source.upgrade().is_none());
        drop(hold);
        app.pump();
        app.read(|e| {
            assert!(
                e.document.visit.working_view.source().is_none()
                    && e.document.visit.working_view.underlay().is_none()
            );
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
            request_working(e, spec.clone(), input, cx);
            assert!(e.document_pending.is_some());
            assert!(
                e.document.visit.working.task.is_none(),
                "working-source dispatch is paused"
            );
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document_pending.is_none() && e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            assert!(e.document.visit.working_view.source().unwrap().spec == spec);
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
            request_working(e, spec, input, cx);
            assert!(e.document.visit.working.task.is_none());
        });
        app.run_until(Duration::from_secs(15), |e| e.allow_close);
        app.pump();
        app.read(|e| {
            assert!(e.document_pending.is_none() && !e.busy);
            assert!(e.document.visit.working.task.is_none());
            assert!(
                e.document.visit.working_view.source().is_none(),
                "close must not start another acquisition"
            );
        });
        assert!(path.is_file());
    });
    scheduler.close_and_wait().unwrap();
}
