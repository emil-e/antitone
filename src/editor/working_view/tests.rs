use super::*;
use crate::geometry::{Composition, FramePoint};
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::photograph::request::PhotographRequest;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;

#[test]
fn demand_revision_tracks_view_intent_and_idle_requires_presentation() {
    let frame = FrameRaster::full(
        Composition::new(
            [0, 0, 32, 24].into(),
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let processing =
        PhotographRequest::Fixture(AuthoredPhotograph::from(ProcessingParameters::source()));

    let mut working = WorkingView::default();
    assert_eq!(working.revision(), None);

    assert!(working.observe(frame, processing.clone(), OutputTarget::Srgb));
    let initial_revision = working.revision();
    assert!(!working.observe(frame, processing.clone(), OutputTarget::Srgb));
    assert_eq!(working.revision(), initial_revision);

    assert!(working.observe(frame, processing.clone(), OutputTarget::DisplayP3));
    assert!(working.revision() > initial_revision);
    let mut revisit = working.for_visit();
    assert!(revisit.observe(frame, processing, OutputTarget::Srgb));

    working.changed_at = Some(std::time::Instant::now() - Duration::from_secs(1));
    assert!(
        !working.idle(),
        "elapsed time without accepted physical presentation is not idle readiness"
    );

    working.clear();
    assert_eq!(working.revision(), None);
}

#[test]
fn completed_frames_keep_their_own_processing_until_scope_retirement() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let kernels = ViewKernels::new(&device).await.unwrap();
        let gate = crate::gpu::submission::SubmissionGate::default();

        let scan = Scan::open(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/raw/patches.dng"),
            None,
        )
        .unwrap();
        let input = source::Context {
            reference: None,
            source: scan.source().clone(),
            correction: None,
        };
        let composition =
            Composition::new(scan.area, Default::default(), Default::default()).unwrap();
        let dimensions = composition.size();
        let frame = FrameRaster::new(
            composition,
            FramePoint([0., 0.]),
            FramePoint(dimensions.map(|n| f64::from(n) / 2.)),
            [32, 24],
        )
        .unwrap();
        let cal = Calibration {
            dmin: DVec3::new(0.19, 0.14, 0.12),
            dmax: DVec3::new(0.008, 0.004, 0.002),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        };
        let first = SourceRequest::new(frame, cal, scan.camera_to_ap1, None)
            .unwrap()
            .with_identity(crate::gpu::source::identity::Identity::camera(
                input.source.identity(),
            ));

        for retirement in ["basis", "support", "clear", "drop"] {
            let mut sources = ViewSource::acquire_many(
                &device,
                &queue,
                &gate,
                renderer.clone(),
                kernels.clone(),
                &scan,
                None,
                vec![first.underlay().unwrap()],
                || false,
            )
            .unwrap()
            .unwrap();
            let underlay = sources.pop().unwrap();
            let weak_underlay = Arc::downgrade(&underlay);

            let mut working = WorkingView::default();
            let mailbox = working.mailbox.clone();
            mailbox.0.lock().unwrap().completed = Some(Arc::new(Frame {
                source: underlay.clone(),
                underlay: Some(underlay),
                request: Arc::new(Demand::fixture(first.clone(), input.clone())),
                processed: Vec::new(),
            }));
            working.request(Demand::fixture(first.clone(), input.clone()));
            let mut navigated = first.clone();
            let mut moved_frame = navigated.frame();
            moved_frame.min.0[0] += 10.;
            moved_frame.max.0[0] += 10.;
            navigated = navigated.with_frame(moved_frame).unwrap();
            working.request(Demand::fixture(navigated, input.clone()));

            assert!(working.underlay().is_some());
            assert!(weak_underlay.upgrade().is_some());

            match retirement {
                "basis" => working.request(Demand::fixture(
                    SourceRequest::new(
                        frame,
                        crate::processing::NegativeInput {
                            calibration: cal,
                            response: crate::processing::Response::film(),
                            gamma: 0.7,
                            balance: Default::default(),
                        },
                        scan.camera_to_ap1,
                        None,
                    )
                    .unwrap(),
                    input.clone(),
                )),
                "support" => working.request(Demand::fixture(
                    first
                        .clone()
                        .with_pipeline(crate::pixel_pipeline::PixelPipeline {
                            stages: vec![crate::pixel_pipeline::PixelStage::new(
                                crate::image_mtf::ImageMtfParameters { sigma: 2. },
                            )],
                        })
                        .unwrap(),
                    input.clone(),
                )),
                "clear" => working.clear(),
                _ => {
                    drop(working);
                    assert!(mailbox.0.lock().unwrap().completed.is_none());
                    assert!(
                        weak_underlay.upgrade().is_none(),
                        "dropping the owner releases completed support"
                    );
                    continue;
                }
            }

            if matches!(retirement, "basis" | "support") {
                let frame = working.frame.as_ref().unwrap();
                assert!(
                    !mailbox
                        .0
                        .lock()
                        .unwrap()
                        .demand
                        .as_ref()
                        .unwrap()
                        .can_use(&frame.source)
                        .unwrap_or(false),
                    "old support must not be treated as input for the new calculation"
                );
                assert!(
                    frame.processing(&Default::default()).unwrap() == first.processing_fixture(),
                    "retained frames keep the settings belonging to their source"
                );
                assert!(weak_underlay.upgrade().is_some());
                working.clear();
            }
            assert!(mailbox.0.lock().unwrap().completed.is_none());
            assert!(
                weak_underlay.upgrade().is_none(),
                "{retirement}: a queued mailbox must not retain retired source support"
            );
        }
    });
}

fn spec(offset: f64) -> SourceRequest {
    let composition = Composition::new(
        Rect {
            x: 12,
            y: 14,
            width: 4096,
            height: 3072,
        },
        Default::default(),
        Default::default(),
    )
    .unwrap();
    let frame = FrameRaster::new(
        composition,
        FramePoint([offset, 0.]),
        FramePoint([2048. + offset, 1536.]),
        [1920, 1080],
    )
    .unwrap();
    SourceRequest::new(
        frame,
        Calibration {
            dmin: DVec3::new(0.19, 0.14, 0.12),
            dmax: DVec3::new(0.008, 0.004, 0.002),
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        },
        glam::DMat3::IDENTITY,
        None,
    )
    .unwrap()
}

#[test]
fn demand_coalesces_navigation_and_rejects_stale_products() {
    let mut working = WorkingView::default();
    // These requests exercise mailbox coalescing, not acquisition of the synthetic view geometry.
    let input = source::Context {
        reference: None,
        source: crate::editor::tests::developed_fixture(),
        correction: None,
    };
    let identity = crate::gpu::source::identity::Identity::camera(input.source.identity());
    let first = spec(0.).with_identity(identity.clone());
    working.request(Demand::fixture(first.clone(), input.clone()));
    for n in 1..40 {
        working.request(Demand::fixture(
            spec(f64::from(n)).with_identity(identity.clone()),
            input.clone(),
        ));
    }
    let last = spec(39.).with_identity(identity);
    assert!(
        working
            .mailbox
            .0
            .lock()
            .unwrap()
            .demand
            .as_ref()
            .map(|demand| &demand.spec)
            == Some(&last)
    );

    working.acquired(Completion {
        owner: Arc::downgrade(&working.mailbox),
        demand: Completion::fixture_demand(&working, first),
        result: Err("obsolete failure".into()),
    });
    working.request(Demand::fixture(last.clone(), input.clone()));
    assert!(working.error.is_none());
    assert!(working.failed.is_none());
    working.acquired(Completion {
        owner: Arc::downgrade(&working.mailbox),
        demand: Completion::fixture_demand(&working, last.clone()),
        result: Err("current failure".into()),
    });
    for _ in 0..5 {
        working.request(Demand::fixture(last.clone(), input.clone()));
    }
    assert_eq!(working.error.as_deref(), Some("current failure"));
    assert!(
        working.acquisition().is_none(),
        "unchanged failure does not spin retries"
    );

    working.clear();
    assert!(working.mailbox.0.lock().unwrap().demand.is_none());
    assert!(working.error.is_none());
}

#[test]
fn the_same_captured_demand_does_not_give_a_departed_owner_continuation_rights() {
    let pixels = source::Context {
        reference: None,
        source: crate::editor::tests::developed_fixture(),
        correction: None,
    };
    let captured = spec(0.).with_identity(crate::gpu::source::identity::Identity::camera(
        pixels.source.identity(),
    ));
    let mut former = WorkingView::default();
    former.request(Demand::fixture(captured.clone(), pixels.clone()));
    let departed = Arc::downgrade(&former.mailbox);
    let mut current = former.for_visit();
    current.request(Demand::fixture(captured.clone(), pixels));
    drop(former);
    current.acquired(Completion {
        owner: departed,
        demand: Completion::fixture_demand(&current, captured.clone()),
        result: Err("departed failure".into()),
    });
    assert!(current.error.is_none());
    assert!(current.failed.is_none());
    current.acquired(Completion {
        owner: Arc::downgrade(&current.mailbox),
        demand: Completion::fixture_demand(&current, captured),
        result: Err("live failure".into()),
    });
    assert_eq!(current.error.as_deref(), Some("live failure"));
}

#[test]
fn a_departed_downstream_capture_cannot_latch_failure_for_the_same_native_request() {
    let source = crate::editor::tests::developed_fixture();
    let area = source.area();
    let matrix = source.camera_to_ap1();
    let frame =
        FrameRaster::full(Composition::new(area, Default::default(), Default::default()).unwrap())
            .unwrap();
    let spec = SourceRequest::new(
        frame,
        crate::processing::InputSettings::Digital,
        matrix,
        None,
    )
    .unwrap();
    let first = Demand::fixture(
        spec.clone(),
        source::Context {
            source,
            reference: None,
            correction: None,
        },
    );
    let mut working = WorkingView::default();
    working.request(first.clone());
    let departed = Completion {
        owner: Arc::downgrade(&working.mailbox),
        demand: working.mailbox.0.lock().unwrap().demand.clone().unwrap(),
        result: Err("old downstream failure".into()),
    };
    let mut latest = first;
    latest.capture = PhotographRequest::Fixture(
        AuthoredPhotograph::new(
            &Default::default(),
            crate::processing::InputSettings::Digital,
            RollSettings {
                exposure: 1.,
                ..Default::default()
            },
            crate::adjustment_layers::AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
            matrix,
            area,
        )
        .unwrap(),
    );
    working.request(latest.clone());
    working.acquired(departed);
    assert!(working.error.is_none());
    assert!(working.failed.is_none());
    let current = Completion {
        owner: Arc::downgrade(&working.mailbox),
        demand: working.mailbox.0.lock().unwrap().demand.clone().unwrap(),
        result: Err("current downstream failure".into()),
    };
    working.acquired(current);
    assert_eq!(working.error.as_deref(), Some("current downstream failure"));
    working.request(latest);
    assert!(
        working.acquisition().is_none(),
        "unchanged failed capture must not spin"
    );
}

#[test]
fn repeated_view_capture_does_not_wake_a_blocked_acquisition_or_repaint_loop() {
    let input = source::Context {
        reference: None,
        source: crate::editor::tests::developed_fixture(),
        correction: None,
    };
    let mut working = WorkingView::default();
    let demand = Demand::fixture(spec(0.), input);
    working.request(demand.clone());
    let changed = working.mailbox.1.listen();
    working.request(demand.clone());
    let mut changed = Box::pin(changed);
    pollster::block_on(std::future::poll_fn(|cx| {
        assert!(
            std::future::Future::poll(changed.as_mut(), cx).is_pending(),
            "unchanged paint demand must not create another host wake"
        );
        std::task::Poll::Ready(())
    }));
    let mut next = demand;
    next.spec = next.spec.with_frame(spec(10.).frame()).unwrap();
    working.request(next);
    pollster::block_on(changed);
}

#[test]
fn reference_admission_respects_budget_inspection_and_explicit_comparison() {
    let mut editor = super::super::tests::fixture();
    editor
        .document
        .thumbnails
        .set_limits(cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        });
    let frame = spec(0.).frame();
    let view = ViewRequest {
        area: frame.bounds().unwrap(),
        frame: Some(frame),
        output: frame.output,
    };
    let retired = editor
        .document
        .visit
        .canvas
        .grade
        .original
        .start_fixture(view);
    assert!(!editor.admit_view_reference(frame));
    assert_ne!(
        editor.document.visit.canvas.grade.original.continuation(),
        retired
    );
    let idle = editor.document.visit.canvas.grade.original.continuation();
    assert!(!editor.admit_view_reference(frame));
    assert_eq!(
        editor.document.visit.canvas.grade.original.continuation(),
        idle,
        "cancel an admitted reference once"
    );

    editor.document.visit.working_view.compare = true;
    editor.document.source = true;
    assert!(
        !editor.admit_view_reference(frame),
        "hidden positive comparison does not force source reference retention"
    );

    editor.document.source = false;
    assert!(editor.admit_view_reference(frame));

    editor.document.visit.working_view.compare = false;
    editor
        .document
        .thumbnails
        .set_limits(cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 512 << 20,
        });
    assert!(
        !editor.admit_view_reference(frame),
        "spare optional budget does not request original processing"
    );
}
