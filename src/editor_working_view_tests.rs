use super::*;
use crate::geometry::{Composition, FramePoint};

#[test]
fn physical_view_demand_revision_includes_source_authority_and_requires_accepted_presentation_for_idle()
 {
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
    let first = working.revision();
    assert!(!working.observe(7, frame, processing.clone(), OutputTarget::Srgb));
    assert_eq!(working.revision(), first);
    assert!(working.observe(8, frame, processing, OutputTarget::Srgb));
    assert!(working.revision() > first);
    working.changed_at = Some(std::time::Instant::now() - Duration::from_secs(1));
    assert!(
        !working.idle(),
        "elapsed time without accepted physical presentation is not idle readiness"
    );
    working.clear();
    assert_eq!(working.revision(), None);
}
#[test]
fn physical_view_underlay_reuses_navigation_support_but_retires_on_basis_change_clear_and_drop() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
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
        let roll = RollSettings::new_grade();
        let first = ViewSourceSpec::new(7, frame, cal, roll, scan.camera_to_ap1, None).unwrap();
        for retirement in ["basis", "clear", "drop"] {
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
            let owner = Arc::downgrade(&underlay);
            let mut working = WorkingView::default();
            let mailbox = working.mailbox.clone();
            mailbox.0.lock().unwrap().underlay = Some(underlay);
            let (send, receive) = mpsc::channel();
            let context = egui::Context::default();
            working.request(first.clone(), &send, &context);
            let mut navigated = first.clone();
            navigated.frame.min.0[0] += 10.;
            navigated.frame.max.0[0] += 10.;
            working.request(navigated, &send, &context);
            assert!(working.underlay.is_some());
            assert!(owner.upgrade().is_some());
            assert_eq!(receive.try_iter().count(), 1);
            match retirement {
                "basis" => working.request(
                    ViewSourceSpec::new(
                        7,
                        frame,
                        cal,
                        RollSettings { gamma: 0.7, ..roll },
                        scan.camera_to_ap1,
                        None,
                    )
                    .unwrap(),
                    &send,
                    &context,
                ),
                "clear" => working.clear(),
                _ => drop(working),
            }
            assert!(mailbox.0.lock().unwrap().underlay.is_none());
            assert!(
                owner.upgrade().is_none(),
                "{retirement}: a queued mailbox must not retain retired source support"
            );
        }
    });
}
fn spec(roll: RollSettings, offset: f64) -> ViewSourceSpec {
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
        roll,
        glam::DMat3::IDENTITY,
        None,
    )
    .unwrap()
}
#[test]
fn physical_view_mailbox_coalesces_navigation_without_restarting_for_grade_edits() {
    let (send, receive) = mpsc::channel();
    let context = egui::Context::default();
    let mut working = WorkingView::default();
    let roll = RollSettings::new_grade();
    let first = spec(roll, 0.);
    for n in 0..40 {
        let changed = spec(
            RollSettings {
                exposure: f64::from(n) / 10.,
                wb: crate::color::Balance {
                    warmth: f64::from(n) / 100.,
                    tint: -0.1,
                },
                ..roll
            },
            0.,
        );
        assert!(changed == first);
        working.request(changed, &send, &context);
    }
    assert_eq!(receive.try_iter().count(), 1);
    for n in 1..40 {
        working.request(spec(roll, f64::from(n)), &send, &context);
    }
    assert_eq!(
        receive.try_iter().count(),
        0,
        "one admitted wake, not one per view/edit"
    );
    let last = spec(roll, 39.);
    assert!(working.mailbox.0.lock().unwrap().wanted.as_ref() == Some(&last));
    {
        let mut mailbox = working.mailbox.0.lock().unwrap();
        mailbox.awake = false;
        mailbox.completed = Some((first, Err("obsolete failure".into())));
    }
    working.request(last.clone(), &send, &context);
    assert!(working.error.is_none());
    assert!(working.failed.is_none());
    assert_eq!(receive.try_iter().count(), 1);
    {
        let mut mailbox = working.mailbox.0.lock().unwrap();
        mailbox.awake = false;
        mailbox.completed = Some((last.clone(), Err("current failure".into())));
    }
    for _ in 0..5 {
        working.request(last.clone(), &send, &context);
    }
    assert_eq!(working.error.as_deref(), Some("current failure"));
    assert_eq!(receive.try_iter().count(), 0);
    working.clear();
    assert!(working.mailbox.0.lock().unwrap().wanted.is_none());
    assert!(working.error.is_none());
}
#[test]
fn physical_view_reference_respects_shared_zero_optional_budget_but_explicit_comparison_is_mandatory()
 {
    let (mut editor, _, _) = super::super::tests::fixture();
    editor.thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 0,
    });
    let frame = spec(RollSettings::new_grade(), 0.).frame;
    let view = ViewRequest {
        area: frame.bounds().unwrap(),
        frame: Some(frame),
        output: frame.output,
    };
    editor
        .original
        .generation
        .store(23, std::sync::atomic::Ordering::Relaxed);
    editor.original.refining = Some((23, view));
    assert!(!editor.admit_view_reference(frame));
    assert_eq!(editor.original.revision(), 24);
    assert!(!editor.admit_view_reference(frame));
    assert_eq!(
        editor.original.revision(),
        24,
        "cancel an admitted reference once"
    );
    editor.working.compare = true;
    editor.source = true;
    assert!(
        !editor.admit_view_reference(frame),
        "hidden positive comparison does not force source reference retention"
    );
    editor.source = false;
    assert!(editor.admit_view_reference(frame));
    editor.working.compare = false;
    editor.thumbnails.set_limits(cache_limits::CacheLimits {
        cpu_bytes: 0,
        gpu_bytes: 512 << 20,
    });
    assert!(
        !editor.admit_view_reference(frame),
        "spare optional budget does not request original processing"
    );
}
