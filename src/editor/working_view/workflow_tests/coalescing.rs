use super::*;
use crate::gpu::input::ProcessingInput;
use crate::gpu::view::input::ViewInput;
use crate::gpu::view::request::SourceRequest;

#[test]
fn running_photographic_capture_progresses_with_matching_gamma_and_wb_before_latest_demand() {
    let (e, pixels, base, scheduler) = fixture();
    let scan = pixels.scan().unwrap();
    let calibration = scan
        .processing_calibration(Calibration {
            dmin: DVec3::splat(0.8),
            dmax: DVec3::splat(0.2),
            input_floor: 1. / 65535.,
        })
        .unwrap();
    let capture = |gamma: f64, wb: f64| {
        let mut settings = crate::processing::InputSettings::from(calibration);
        let crate::processing::InputSettings::Negative { balance, .. } = &mut settings else {
            panic!("negative fixture");
        };
        balance.warmth = gamma;
        let processing = ProcessingInput::new(
            &wgpu::Limits::default(),
            settings,
            RollSettings {
                wb: crate::color::Balance {
                    warmth: wb,
                    tint: 0.,
                },
                exposure: 0.,
            },
            AdjustmentLayerSnapshot::new(Vec::new()).unwrap(),
            scan.camera_to_ap1,
            scan.area,
        )
        .unwrap();
        let spec = SourceRequest::for_processing(base.frame(), &processing)
            .unwrap()
            .with_identity(crate::gpu::source::identity::Identity::camera(
                pixels.source.identity(),
            ));
        Demand {
            spec,
            pixels: pixels.clone(),
            capture: ViewInput::Processing(processing),
        }
    };
    let first = capture(0.02, 0.15);
    let last = capture(-0.03, -0.2);
    assert!(first.spec.same_source(&last.spec));
    assert!(!first.same_calculation(&last));
    assert!(
        first.processing(&Default::default()).unwrap()
            != last.processing(&Default::default()).unwrap()
    );
    let submissions = e.submissions.clone();
    let services = e.source_services.as_ref().unwrap().clone();
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    with_application(e, |app| {
        app.enter(|e, cx| e.request_working(capture(0., 0.), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let previous_native = app.read(|e| e.document.visit.working_view.source().unwrap().clone());
        let baseline = capture(0., 0.04);
        app.enter(|e, cx| e.request_working(baseline.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document
                .visit
                .working_view
                .frame
                .as_ref()
                .is_some_and(|frame| frame.request.capture == baseline.capture)
                && e.document.visit.working.task.is_none()
        });
        app.read(|e| {
            assert!(
                previous_native.same_pixels_as(e.document.visit.working_view.source().unwrap()),
                "a downstream WB calculation must reuse its native value"
            )
        });
        app.update(|e| {
            e.document
                .visit
                .working_view
                .render(
                    &baseline.processing(&Default::default()).unwrap(),
                    OutputTarget::Srgb,
                    &egui::Context::default(),
                )
                .unwrap()
        });
        let baseline_signature = baseline.processing(&Default::default()).unwrap();
        submissions.observe_submissions(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }));
        app.enter(|e, cx| e.request_working(first.clone(), cx));
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        app.update(|e| {
            e.document.visit.working_view.render(&first.processing(&Default::default()).unwrap(),OutputTarget::Srgb,&egui::Context::default()).unwrap();
            assert!(e.document.visit.working_view.rendered.as_ref().unwrap().processing()==baseline_signature,
                "starting a new gamma calculation must not revert an already captured downstream WB edit");
        });
        for i in 0..24 {
            app.enter(|e, cx| {
                e.request_working(
                    capture(-0.01 - i as f64 * 0.0005, -0.05 - i as f64 * 0.001),
                    cx,
                )
            });
            app.pump();
        }
        app.enter(|e, cx| e.request_working(last.clone(), cx));
        app.pump();
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "superseding controls must not start overlapping image jobs"
        );
        let mut advances = 0;
        loop {
            release.send(()).unwrap();
            advances += 1;
            loop {
                app.pump();
                if starts.try_recv().is_ok() {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            let progressed = app.read(|e| {
                e.document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .is_some_and(|frame| frame.request.same_calculation(&first))
            });
            if progressed {
                break;
            }
            assert!(
                advances < 8,
                "the first captured frame must be presented before subsequent captures"
            );
        }
        app.update(|e| {
            let working = &mut e.document.visit.working_view;
            assert!(
                !working.support_ready(),
                "progress is not a claim that the latest demand is ready"
            );
            assert!(
                working
                    .frame
                    .as_ref()
                    .unwrap()
                    .processing(&Default::default())
                    .unwrap()
                    == first.processing(&Default::default()).unwrap()
            );
            working
                .render(
                    &last.processing(&Default::default()).unwrap(),
                    OutputTarget::Srgb,
                    &egui::Context::default(),
                )
                .unwrap();
            let rendered = working.rendered.as_ref().unwrap();
            assert!(
                rendered.processing() == first.processing(&Default::default()).unwrap(),
                "old gamma must never take newer compensating WB"
            );
            let frame = working.frame.as_ref().unwrap();
            assert!(
                frame
                    .source
                    .definition
                    .same_definition(&first.calculation(&frame.source).unwrap().native)
            );
            assert!(rendered.uses_processed(&frame.processed[0]));
            assert!(rendered.processing() != last.processing(&Default::default()).unwrap());
        });
        // The next capture is already submitted and parked. Colour rendering uses the same
        // production submission gate, without the source-worker fixture barrier.
        submissions.observe_submissions(Arc::new(|| {}));
        let rendered = app.read(|e| e.document.visit.working_view.rendered.clone().unwrap());
        rendered.ensure_submitted(
            &services.device,
            &services.queue,
            &submissions,
            &egui::Context::default(),
        );
        let observed = count.clone();
        submissions.observe_submissions(Arc::new(move || {
            observed.fetch_add(1, Ordering::Relaxed);
        }));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.update(|e| {
            let working = &mut e.document.visit.working_view;
            working
                .render(
                    &last.processing(&Default::default()).unwrap(),
                    OutputTarget::Srgb,
                    &egui::Context::default(),
                )
                .unwrap();
            let frame = working.frame.as_ref().unwrap();
            assert!(frame.request.same_calculation(&last));
            assert!(
                frame
                    .source
                    .definition
                    .same_definition(&last.calculation(&frame.source).unwrap().native)
            );
            assert!(
                working.rendered.as_ref().unwrap().processing()
                    == last.processing(&Default::default()).unwrap()
            );
        });
        assert_eq!(
            count.load(Ordering::Relaxed),
            2 * advances,
            "only the running and latest captures should execute; compare with observed capture work, not fixed tiling"
        );
    });
    scheduler.close_and_wait().unwrap();
}
