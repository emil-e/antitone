use super::*;
use crate::geometry::{Composition, FramePoint};

#[test]
fn demand_revision_tracks_source_authority_and_idle_requires_presentation() {
    let frame = FrameRaster::full(
        Composition::new(
            [0, 0, 32, 24].into(),
            Default::default(),
            Default::default(),
        )
        .unwrap(),
    )
    .unwrap();
    let processing = ProcessingInput::from(ProcessingParameters::source()).signature();

    let mut working = WorkingView::default();
    assert_eq!(working.revision(), None);

    assert!(working.observe(7, frame, processing.clone(), OutputTarget::Srgb));
    let initial_revision = working.revision();
    assert!(!working.observe(7, frame, processing.clone(), OutputTarget::Srgb));
    assert_eq!(working.revision(), initial_revision);

    assert!(working.observe(8, frame, processing, OutputTarget::Srgb));
    assert!(working.revision() > initial_revision);

    working.changed_at = Some(std::time::Instant::now() - Duration::from_secs(1));
    assert!(
        !working.idle(),
        "elapsed time without accepted physical presentation is not idle readiness"
    );

    working.clear();
    assert_eq!(working.revision(), None);
}

#[test]
fn navigation_reuses_underlay_until_basis_change_clear_or_drop() {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderer = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        let kernels = ViewKernels::new(&device).await.unwrap();
        let gate = crate::gpu::SubmissionGate::default();

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
        let first = ViewSourceSpec::new(7, frame, cal, scan.camera_to_ap1, None)
            .unwrap()
            .with_identity(crate::gpu::execution::source::Identity::camera(
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
            mailbox.0.lock().unwrap().underlay = Some(underlay);
            working.request(first.clone(), input.clone());
            let mut navigated = first.clone();
            let mut moved_frame = navigated.frame();
            moved_frame.min.0[0] += 10.;
            moved_frame.max.0[0] += 10.;
            navigated = navigated.with_frame(moved_frame).unwrap();
            working.request(navigated, input.clone());

            assert!(working.underlay.is_some());
            assert!(weak_underlay.upgrade().is_some());

            match retirement {
                "basis" => working.request(
                    ViewSourceSpec::new(
                        7,
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
                ),
                "support" => working.request(
                    first
                        .clone()
                        .with_pipeline(crate::pixel_pipeline::PixelPipeline {
                            stages: vec![crate::pixel_pipeline::PixelStage::new(
                                crate::image_mtf::ImageMtfParameters { sigma: 2. },
                            )],
                        })
                        .unwrap(),
                    input.clone(),
                ),
                "clear" => working.clear(),
                _ => drop(working),
            }

            assert!(mailbox.0.lock().unwrap().underlay.is_none());
            assert!(
                weak_underlay.upgrade().is_none(),
                "{retirement}: a queued mailbox must not retain retired source support"
            );
        }
    });
}

fn spec(offset: f64) -> ViewSourceSpec {
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
    ViewSourceSpec::new(
        7,
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
    let identity = crate::gpu::execution::source::Identity::camera(input.source.identity());
    let first = spec(0.).with_identity(identity.clone());
    working.request(first.clone(), input.clone());
    for n in 1..40 {
        working.request(
            spec(f64::from(n)).with_identity(identity.clone()),
            input.clone(),
        );
    }
    let last = spec(39.).with_identity(identity);
    assert!(working.mailbox.0.lock().unwrap().wanted.as_ref() == Some(&last));

    working.acquired(Completion {
        spec: first,
        result: Err("obsolete failure".into()),
    });
    working.request(last.clone(), input.clone());
    assert!(working.error.is_none());
    assert!(working.failed.is_none());
    working.acquired(Completion {
        spec: last.clone(),
        result: Err("current failure".into()),
    });
    for _ in 0..5 {
        working.request(last.clone(), input.clone());
    }
    assert_eq!(working.error.as_deref(), Some("current failure"));
    assert!(
        working.acquisition(7).is_none(),
        "unchanged failure does not spin retries"
    );

    working.clear();
    assert!(working.mailbox.0.lock().unwrap().wanted.is_none());
    assert!(working.error.is_none());
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

    editor
        .document
        .visit
        .grade
        .original
        .generation
        .store(23, std::sync::atomic::Ordering::Relaxed);
    editor.document.visit.grade.original.refining = Some((23, view));
    assert!(!editor.admit_view_reference(frame));
    assert_eq!(editor.document.visit.grade.original.revision(), 24);

    assert!(!editor.admit_view_reference(frame));
    assert_eq!(
        editor.document.visit.grade.original.revision(),
        24,
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
