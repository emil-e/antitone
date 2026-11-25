use super::*;
use crate::photographic::graph::request::PhotographRequest;
use crate::photographic::graph::view_source::SourceRequest;
use crate::photographic::test_inputs::AuthoredPhotograph;
use crate::{
    application::test_support::with_application,
    geometry::{Composition, FramePoint},
};
use std::sync::atomic::Ordering;

mod coalescing;
mod inputs;
mod modality;
mod regional_identity;
mod retention;

fn requested_image(
    demand: &Demand,
    source: &ViewImage,
    frame: &Frame,
) -> Result<crate::evaluation::Value, Error> {
    crate::viewer::view::rendering::photograph(
        &source.context.device().limits(),
        &demand.pixels.scan()?,
        demand.pixels.correction.gpu()?,
        &demand.capture,
        source.spec.clone(),
        &frame.looks,
        frame.targets.get(demand.target),
    )
    .map(|image| image.definition)
}

fn image_matches(demand: &Demand, source: &ViewImage, frame: &Frame) -> bool {
    requested_image(demand, source, frame)
        .is_ok_and(|image| source.definition.same_definition(&image))
}

fn cached_inputs(view: &ViewImage) -> Vec<std::sync::Weak<crate::evaluation::ImageValue>> {
    use crate::evaluation::ImageLayout;

    let scope = view.context.materializations();
    let root = &view.definition;
    root.ancestors()
        .skip(1)
        .filter_map(|input| {
            let region = root.region_for(input, view.region).unwrap()?;
            input.output(region).unwrap().identity().cloned()
        })
        .flat_map(|key| scope.ready::<ImageLayout>(&key))
        .map(|image| Arc::downgrade(&image.image))
        .collect()
}

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
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        crate::gpu::submission::request_device(&adapter, &Default::default())
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
        .with_identity(crate::acquisition::camera::identity::Identity::camera(
            scan.source().identity(),
        ));
    let input = source::Context {
        reference: None,
        source: scan.source().clone(),
        correction: Default::default(),
    };
    let scheduler = source_loading::SourceLoader::new();
    e.source_services = Some(source::Services::fixture(
        scheduler.clone(),
        &device,
        &queue,
        &crate::editor::display::DisplayFixture::for_device(&device),
    ));
    e.document.visit.working_view = WorkingView::new(device, queue.clone());
    (e, input, spec, scheduler)
}

fn complete_frame(frame: &Arc<Frame>, services: &source::Services) {
    let ui = egui::Context::default();
    for image in std::iter::once(&frame.source).chain(frame.underlay.as_ref()) {
        let rendered = RenderedView::from_image(
            &services.device,
            image.clone(),
            frame.request.capture.clone(),
            frame.request.target,
        )
        .unwrap();
        rendered.ensure_submitted(&services.device, &services.queue, &ui);
    }
    services
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
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
    .with_definition({
        let mtf = crate::photographic::stages::image_mtf::ImageMtfParameters { sigma };
        let pitch = [Some(mtf.sigma / 0.6)]
            .into_iter()
            .flatten()
            .reduce(f64::min);
        crate::photographic::graph::authoring::Assembly::authored(move |mut image, output| {
            image = mtf.compose(image)?;
            if let Some(output) = output {
                image = output.compose(image)?;
            }
            Ok(image)
        })
        .with_view_pitch(pitch)
    })
    .unwrap()
    .with_identity(crate::acquisition::camera::identity::Identity::camera(
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
        latest.definition().processing_plan(&small).is_err(),
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
                !image_matches(
                    &Demand::fixture(latest.clone(), input.clone()),
                    underlay,
                    e.document.visit.working_view.frame.as_ref().unwrap()
                ),
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
            assert!(e.document.visit.working_view.task.is_some());
            let working = &mut e.document.visit.working_view;
            let current = working.frame.as_ref().unwrap();
            let (looks, targets) = (current.looks.clone(), current.targets.clone());
            working.acquired(Completion {
                owner: Arc::downgrade(&working.mailbox),
                demand: Completion::fixture_demand(working, queued.clone()),
                result: Ok(Arc::new(Frame {
                    sequence: 0,
                    source: old.clone(),
                    underlay: None,
                    request: Arc::new(Demand::fixture(small.clone(), input.clone())),
                    looks: looks.clone(),
                    targets: targets.clone(),
                })),
            });
            assert!(
                e.document.visit.working_view.error.is_none(),
                "obsolete completion cannot fail current demand"
            );
            // Publication must validate actual coverage as well as the completion ticket.
            let working = &mut e.document.visit.working_view;
            working.acquired(Completion {
                owner: Arc::downgrade(&working.mailbox),
                demand: Completion::fixture_demand(working, latest.clone()),
                result: Ok(Arc::new(Frame {
                    sequence: 0,
                    source: old.clone(),
                    underlay: None,
                    request: Arc::new(Demand::fixture(small.clone(), input.clone())),
                    looks: looks.clone(),
                    targets: targets.clone(),
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
        app.read(|e| {
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
            assert!(e.document.visit.working_view.underlay().is_some_and(
                |underlay| image_matches(
                    &Demand::fixture(latest.clone(), input.clone()),
                    underlay,
                    e.document.visit.working_view.frame.as_ref().unwrap()
                )
            ));
        });
        app.enter(|e, access| request_working(e, small.clone(), input, access));
        app.read(|e| {
            assert!(e.document.visit.working_view.support_ready());
            assert!(
                e.document.visit.working_view.task.is_none(),
                "the smaller neighbourhood must use available inputs without reacquisition"
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
        correction: Default::default(),
    };
    let small = positive_spec(frame, &input, 0.25);
    let latest = positive_spec(frame, &input, 0.75);
    let services = e.source_services.as_ref().unwrap();
    let renderer = services.render_fixture().unwrap();
    let scope = renderer.evaluation().materializations();
    scope.set_retention_enabled(false);
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    e.submissions().observe_submissions(Arc::new(move || {
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
                e.document.visit.working_view.task.is_some(),
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
            assert!(e.document.visit.working_view.underlay().is_some_and(
                |underlay| image_matches(
                    &Demand::fixture(latest.clone(), input.clone()),
                    underlay,
                    e.document.visit.working_view.frame.as_ref().unwrap()
                )
            ));
            assert!(e.document.visit.working_view.task.is_none());
        });
    });
    assert!(count.load(Ordering::Relaxed) > 0);
    assert_eq!(renderer.evaluation().retained_bytes(), 0);
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
            e.replace_photo_scope(e.document.visit.current_source);
            e.document
                .visit
                .working_view
                .request(Demand::fixture(spec.clone(), input.clone()));
            assert!(!e.document.visit.working_view.support_ready());
            let working = &mut e.document.visit.working_view;
            working.acquired(Completion {
                owner: departed.clone(),
                demand: Completion::fixture_demand(working, spec.clone()),
                result: Ok(frame.clone()),
            });
            assert!(
                e.document.visit.working_view.source().is_none(),
                "even a complete matching calculation must arrive through its live owner"
            );
            let working = &mut e.document.visit.working_view;
            working.acquired(Completion {
                owner: departed.clone(),
                demand: Completion::fixture_demand(working, spec.clone()),
                result: Err(Error::Gpu("departed failure".into())),
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
            let working = &mut e.document.visit.working_view;
            working.acquired(Completion {
                owner: departed,
                demand: Completion::fixture_demand(working, spec),
                result: Err(Error::Gpu("late departed failure".into())),
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
fn detail_can_present_while_underlay_is_parked() {
    let (mut e, input, spec, scheduler) = fixture();
    let (release, gate) = futures_channel::oneshot::channel();
    let view = &mut e.document.visit.working_view;
    view.mailbox.0.lock().unwrap().underlay_gate = Some(gate);
    let evaluation = &e.source_services.as_ref().unwrap().evaluation;
    evaluation.set_retention_limit(0);
    with_application(e, |app| {
        app.enter(|e, access| request_working(e, spec.clone(), input, access));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.source().is_some()
        });
        let detail = app.enter(|e, _| {
            let view = &mut e.document.visit.working_view;
            assert!(view.task.is_some());
            assert!(view.underlay().is_none());
            view.render(&egui::Context::default()).unwrap();
            assert!(view.present(spec.frame(), None).unwrap().unwrap().spec == spec);
            view.source().unwrap().clone()
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.task.is_none()
        });
        app.read(|e| {
            let view = &e.document.visit.working_view;
            assert!(view.underlay().is_some());
            assert!(Arc::ptr_eq(view.source().unwrap(), &detail));
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
            assert!(e.document.visit.working_view.task.is_none());
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
    e.document.project.roll.exposure = 1.;
    let context = &e.source_services.as_ref().unwrap().evaluation;
    context.set_retention_limit(128 << 20);
    context
        .materializations()
        .set_test_pressure(crate::sys::MemoryPressure::Normal);
    let recipe = Arc::new(
        e.view_recipe(e.input(), e.document.project.roll, e.photo_settings())
            .unwrap(),
    );
    let pixels = e.source_context().unwrap();
    let device = e.document.visit.working_view.device().unwrap().clone();
    let edited = Arc::new(
        e.view_recipe(
            e.input(),
            RollSettings {
                exposure: 2.,
                ..e.document.project.roll
            },
            e.photo_settings(),
        )
        .unwrap(),
    );
    let mut frame = FrameRaster::full(recipe.composition).unwrap();
    frame.output = [32, 24];
    let demand = Demand {
        rendering: OnceLock::new(),
        target: OutputTarget::Srgb,
        frame,
        pixels,
        capture: PhotographRequest::Recipe(recipe.clone()),
    };
    with_application(e, |app| {
        app.enter(|e, cx| e.request_working(demand, cx));
        app.update(|e| e.document.project.roll.exposure = 2.);
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
            assert_eq!(ready.images().count(), 2);
            let mut changed = (*ready.request).clone();
            changed.capture = PhotographRequest::Recipe(edited.clone());
            for value in ready.images() {
                assert!(matches!(
                    value.pixels,
                    crate::evaluation::BufferImage::Produced(_)
                ));
                assert!(
                    value.recording().is_none(),
                    "publication owns a completed value, not unsubmitted grading work"
                );
                assert!(
                    value
                        .definition
                        .same_definition(&requested_image(&ready.request, value, &ready).unwrap())
                );
                assert!(
                    !value
                        .definition
                        .same_definition(&requested_image(&changed, value, &ready).unwrap()),
                    "the requested snapshot must not read subsequent document edits"
                );
            }
            working.render(&egui::Context::default()).unwrap();
            assert!(working.rendered.as_ref().unwrap().uses_image(&ready.source));
            assert!(
                working
                    .underlay_rendered
                    .as_ref()
                    .unwrap()
                    .uses_image(ready.underlay.as_ref().unwrap())
            );
        });
        let (mut wanted, context) = app.read(|e| {
            let ready = e.document.visit.working_view.frame.as_ref().unwrap();
            ((*ready.request).clone(), ready.source.context.clone())
        });
        let scope = context.materializations();
        wanted.capture = PhotographRequest::Recipe(edited.clone());
        app.enter(|e, cx| e.request_working(wanted, cx));
        let definition = app.read(|e| {
            let working = &e.document.visit.working_view;
            assert!(working.support_ready());
            let value = &working.frame.as_ref().unwrap().source;
            assert!(
                value.recording().is_some(),
                "control: the resident edit must create recorded work"
            );
            assert!(matches!(
                value.pixels,
                crate::evaluation::BufferImage::Produced(_)
            ));
            value.definition.clone()
        });
        assert!(
            scope
                .find_ready::<crate::evaluation::ImageLayout>(
                    definition.identity().unwrap(),
                    |_| true
                )
                .is_none()
        );
        app.update(|e| {
            e.document
                .visit
                .working_view
                .render(&egui::Context::default())
                .unwrap()
        });
        let (rendered, underlay, queue) = app.read(|e| {
            let view = &e.document.visit.working_view;
            (
                view.rendered.clone().unwrap(),
                view.underlay_rendered.clone().unwrap(),
                e.source_services.as_ref().unwrap().queue.clone(),
            )
        });
        let ui = egui::Context::default();
        let product = rendered.product().unwrap();
        let second = RenderedView::from_image(
            &device,
            product.clone(),
            rendered.capture.clone(),
            rendered.target,
        )
        .unwrap();
        assert!(product.recording().is_some());
        for output in [&rendered, &underlay, &second] {
            output.ensure_submitted(&device, &queue, &ui);
            assert!(output.product().unwrap().recording().is_none());
        }
        let layout = definition.domain();
        let evaluation = context
            .request(
                definition.clone(),
                crate::evaluation::Region {
                    bounds: layout.finite().unwrap(),
                    grid: layout.grid,
                },
                crate::evaluation::Tiling { tile: [32, 24] },
            )
            .unwrap();
        let held = pollster::block_on(
            context
                .materialize(
                    &queue,
                    &scope,
                    evaluation.clone(),
                    crate::evaluation::ImageFormat::Rgba32Float,
                    crate::evaluation::ImageBacking::Buffer,
                )
                .unwrap(),
        )
        .unwrap();
        assert_eq!(held.identity, *definition.identity().unwrap());
        let inputs = held.image.used_inputs();
        assert!(inputs.iter().any(|input| input.strong_count() > 0));
        let expected = crate::viewer::readback::image_pixels(&device, &queue, &rendered.image);
        assert_eq!(
            crate::viewer::readback::image_pixels(&device, &queue, &second.image),
            expected,
            "both presentations must use the same submitted image"
        );
        scope.set_retention_enabled(false);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        assert!(inputs.iter().all(|input| input.upgrade().is_none()));
        let redraw = RenderedView::from_image(
            &device,
            rendered.product().unwrap().clone(),
            rendered.capture.clone(),
            rendered.target,
        )
        .unwrap();
        redraw.ensure_submitted(&device, &queue, &ui);
        assert_eq!(
            crate::viewer::readback::image_pixels(&device, &queue, &redraw.image),
            expected,
        );
        drop((rendered, underlay, redraw));
        app.update(|e| e.document.visit.working_view.clear());
        assert!(
            scope
                .find_ready::<crate::evaluation::ImageLayout>(
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
fn authored_repair_changes_require_new_values_without_prepared_patch_arguments() {
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
        e.view_recipe(e.input(), e.document.project.roll, e.photo_settings())
            .unwrap(),
    );
    assert_eq!(recipe.repairs.len(), 1);
    let pixels = e.source_context().unwrap();
    let mut frame = FrameRaster::full(recipe.composition).unwrap();
    frame.output = [32, 24];
    let initial = Demand {
        rendering: OnceLock::new(),
        target: OutputTarget::Srgb,
        frame,
        pixels,
        capture: PhotographRequest::Recipe(recipe),
    };
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
        let mut changed = (*old.request).clone();
        let mut recipe = (**changed.capture.recipe().unwrap()).clone();
        recipe.repairs[0].offset = [-39, 0];
        changed.capture = PhotographRequest::Recipe(Arc::new(recipe));
        assert!(
            old.request.capture != changed.capture,
            "the donor instruction must actually change"
        );
        assert!(!old.satisfies(&changed));
        assert!(
            !image_matches(&changed, &old.source, &old),
            "an old repaired raster cannot represent a changed donor calculation"
        );
        app.enter(|e, cx| e.request_working(changed.clone(), cx));
        app.read(|e| assert!(!e.document.visit.working_view.support_ready()));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            let ready = e.document.visit.working_view.frame.as_ref().unwrap();
            assert!(ready.request.same_calculation(&changed));
            for source in std::iter::once(&ready.source).chain(ready.underlay.as_ref()) {
                assert!(image_matches(&changed, source, ready));
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
                assert!(e.document.visit.working_view.task.is_some());
            });
        }
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let before = app.read(|e| {
            assert!(e.document.visit.working_view.source().unwrap().spec == spec);
            e.document
                .visit
                .working_view
                .frame
                .as_ref()
                .unwrap()
                .clone()
        });
        let before_inputs = cached_inputs(&before.source);
        let processing = AuthoredPhotograph::from(
            crate::photographic::parameters::ProcessingParameters::source_inspection(1.2).unwrap(),
        );
        let edited = Demand {
            rendering: OnceLock::new(),
            target: OutputTarget::Srgb,
            frame: spec.frame(),
            pixels: input,
            capture: PhotographRequest::Fixture(processing.clone()),
        };
        assert!(
            before.request.capture != edited.capture,
            "this phase must submit genuinely changed processing"
        );
        app.enter(|e, access| {
            e.document.set_target(OutputTarget::Srgb);
            e.request_working(edited.clone(), access);
        });
        app.pump();
        let ui = egui::Context::default();
        let (rendered, device, queue) = app.update(|e| {
            assert!(
                e.document.visit.working_view.task.is_none(),
                "output-only demand must not reacquire originals"
            );
            let services = e.source_services.as_ref().unwrap();
            let (device, queue) = (services.device.clone(), services.queue.clone());
            let working = &mut e.document.visit.working_view;
            let frame = working.frame.as_ref().unwrap();
            let crate::evaluation::BufferImage::Produced(output) = &frame.source.pixels else {
                panic!("expected a new calculated image");
            };
            assert!(
                output.image.used_inputs().iter().any(|bound| {
                    bound.strong_count() != 0
                        && before_inputs.iter().any(|prior| prior.ptr_eq(bound))
                }),
                "the edited recording binds a cached input from the preceding calculation"
            );
            assert!(frame.request.same_calculation(&edited));
            assert_eq!(before.images().count(), frame.images().count());
            for (old, new) in before.images().zip(frame.images()) {
                assert!(
                    !old.definition.same_definition(&new.definition),
                    "both native consumers must apply the changed output calculation"
                );
            }
            working.render(&ui).unwrap();
            let rendered = working.present(spec.frame(), None).unwrap().unwrap();
            assert!(rendered.processing() == processing);
            assert_eq!(rendered.target, OutputTarget::Srgb);
            (rendered, device, queue)
        });
        rendered.ensure_submitted(&device, &queue, &ui);
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
        correction: Default::default(),
    };
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    e.submissions().observe_submissions(Arc::new(move || {
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
            e.replace_photo_scope(e.document.visit.current_source);
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
        app.update(|e| e.replace_photo_scope(e.document.visit.current_source));
        app.pump();
        assert!(source.upgrade().is_none());
        drop(hold);
        app.pump();
        app.read(|e| {
            assert!(
                e.document.visit.working_view.source().is_none()
                    && e.document.visit.working_view.underlay().is_none()
            );
            assert!(e.document.visit.working_view.task.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn ordinary_canvas_during_save_keeps_acquisition_and_tracks_current_demand() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.document.source = true;
    let input = editor.source_context().unwrap();
    let scan = input.scan().unwrap();
    let composition = Composition::new(scan.area, Default::default(), Default::default()).unwrap();
    let size = composition.size();
    let frame = FrameRaster::new(
        composition,
        FramePoint([0., 0.]),
        FramePoint(size.map(f64::from)),
        [64, 48],
    )
    .unwrap();
    let spec = SourceRequest::inspection_fixture(frame, scan.camera_to_ap1, None)
        .unwrap()
        .with_identity(crate::acquisition::camera::identity::Identity::camera(
            input.source.identity(),
        ));
    let (entered, reached) = std::sync::mpsc::sync_channel(1);
    let (release, resume) = std::sync::mpsc::sync_channel::<()>(0);
    let resume = std::sync::Mutex::new(resume);
    editor.document.write_observer = Some(Arc::new(move |phase| {
        if phase == crate::project::WritePhase::TemporarySynced {
            entered.send(()).unwrap();
            let _ = resume.lock().unwrap().recv();
        }
    }));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.antitone");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project(path.clone(), crate::project::Save::New, None, cx)
        });
        app.pump();
        reached.recv_timeout(Duration::from_secs(15)).unwrap();
        // Park only the later working acquisition; the write is already at its own physical boundary.
        let working = crate::background::testing::pause();
        app.enter(|editor, cx| request_working(editor, spec.clone(), input.clone(), cx));
        app.pump();
        app.read(|editor| {
            assert!(
                editor.document.visit.working_view.task.is_some()
                    && !editor.document.visit.working_view.support_ready()
            )
        });
        let context = egui::Context::default();
        crate::editor::tests::app_frame(app, &context, vec![]);
        let requested = app.read(|editor| {
            assert!(
                editor.document.visit.canvas.last_layout.is_some(),
                "paint the ordinary composed canvas"
            );
            assert!(
                editor
                    .document
                    .visit
                    .working_view
                    .mailbox
                    .0
                    .lock()
                    .unwrap()
                    .demand
                    .is_some(),
                "input modality must not withdraw captured physical demand"
            );
            let requested = editor.document.visit.working_view.wanted.clone().unwrap();
            assert!(requested.pixels.source.identity() == input.source.identity());
            assert!(requested.capture.is_inspection());
            requested
        });
        drop(working);
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        app.read(|editor| {
            assert!(editor.document.write.pending() && editor.interaction_blocked());
            let source = editor.document.visit.working_view.source().unwrap();
            assert_eq!(source.spec.frame(), requested.frame);
            assert!(image_matches(
                &requested,
                source,
                editor.document.visit.working_view.frame.as_ref().unwrap()
            ));
            assert!(!editor.dialogs.is_open(), "{}", editor.status);
        });
        drop(release);
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.write.pending() && !editor.document.files.pending()
        });
        assert!(path.is_file());
    });
}

#[test]
fn closing_after_save_does_not_dispatch_new_working_demand() {
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
            request_working(e, spec.clone(), input.clone(), cx);
            assert!(e.document.visit.working_view.task.is_some());
        });
        app.run_until(Duration::from_secs(15), |e| e.allow_close);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.task.is_none()
        });
        app.enter(|e, cx| {
            assert!(!e.document.write.pending() && !e.interaction_blocked());
            e.document.visit.working_view.clear();
            request_working(e, spec, input, cx);
            assert!(
                e.document.visit.working_view.task.is_none(),
                "an approved close admits no new acquisition"
            );
        });
        assert!(path.is_file());
    });
    scheduler.close_and_wait().unwrap();
}
