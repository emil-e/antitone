use super::*;
mod layout;
use crate::gpu::photograph::request::PhotographRequest;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::view::request::SourceRequest;

fn capture(pixels: &source::Context, base: &SourceRequest, offset: f64, wb: f64) -> Demand {
    let scan = pixels.scan().unwrap();
    let calibration = (Calibration {
        dmin: DVec3::splat(0.8),
        dmax: DVec3::splat(0.2),
    })
    .validate()
    .unwrap();
    let mut settings = crate::processing::InputSettings::from(calibration);
    let crate::processing::InputSettings::Negative { gamma, .. } = &mut settings else {
        panic!("negative fixture");
    };
    *gamma = 0.6 + offset;
    let processing = AuthoredPhotograph::new(
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
        rendering: OnceLock::new(),
        target: OutputTarget::Srgb,
        frame: spec.frame(),
        pixels: pixels.clone(),
        capture: PhotographRequest::Fixture(processing),
    }
}

fn assert_coherent(editor: &mut Editor, wanted: &Demand) {
    let working = &mut editor.document.visit.working_view;
    working.render(&egui::Context::default()).unwrap();
    let frame = working.frame.as_ref().unwrap();
    assert!(frame.request.same_calculation(wanted));
    assert!(
        frame
            .source
            .definition
            .same_definition(&requested_image(wanted, &frame.source, frame).unwrap())
    );
    let rendered = working.rendered.as_ref().unwrap();
    assert!(
        rendered.processing() == wanted.processing(&Default::default()).unwrap(),
        "a captured gamma must not acquire another capture's compensating WB"
    );
    assert!(rendered.uses_image(&frame.source));
}

#[test]
fn ready_inputs_preserve_coherent_gamma_and_wb_snapshots_without_requiring_worker_submission() {
    let (editor, pixels, base, scheduler) = fixture();
    let services = editor.source_services.as_ref().unwrap().clone();
    with_application(editor, |app| {
        let initial = capture(&pixels, &base, 0., 0.);
        app.enter(|editor, cx| editor.request_working(initial, cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        complete_frame(
            &app.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone()
            }),
            &services,
        );
        for (gamma, wb) in [(0.02, 0.15), (-0.01, -0.07), (-0.03, -0.2)] {
            let wanted = capture(&pixels, &base, gamma, wb);
            app.enter(|editor, cx| editor.request_working(wanted.clone(), cx));
            app.run_until(Duration::from_secs(15), |editor| {
                editor.document.visit.working_view.support_ready()
            });
            app.update(|editor| assert_coherent(editor, &wanted));
            complete_frame(
                &app.read(|editor| {
                    editor
                        .document
                        .visit
                        .working_view
                        .frame
                        .as_ref()
                        .unwrap()
                        .clone()
                }),
                &services,
            );
        }
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn running_capture_coalesces_superseded_controls_and_finishes_with_the_latest_coherent_snapshot() {
    let (editor, pixels, base, scheduler) = fixture();
    let first = capture(&pixels, &base, 0.02, 0.15);
    let last = capture(&pixels, &base, -0.03, -0.2);
    assert!(first.same_source(&last));
    assert!(!first.same_calculation(&last));
    let submissions = editor.submissions().clone();
    let services = editor.source_services.as_ref().unwrap().clone();
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    submissions.observe_submissions(Arc::new(move || {
        if observed.fetch_add(1, Ordering::Relaxed) == 0 {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
    }));
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_working(first.clone(), cx));
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        for index in 0..24 {
            app.enter(|editor, cx| {
                editor.request_working(
                    capture(
                        &pixels,
                        &base,
                        -0.01 - index as f64 * 0.0005,
                        -0.05 - index as f64 * 0.001,
                    ),
                    cx,
                )
            });
            app.pump();
        }
        app.enter(|editor, cx| editor.request_working(last.clone(), cx));
        app.pump();
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "superseding controls must not start overlapping image jobs"
        );
        release.send(()).unwrap();
        loop {
            app.pump();
            app.update(|editor| {
                if let Some(frame) = &editor.document.visit.working_view.frame {
                    let active = frame.request.clone();
                    assert!(
                        active.same_calculation(&first) || active.same_calculation(&last),
                        "superseded intermediate controls must not become completed captures"
                    );
                    assert_coherent(editor, &active);
                }
            });
            if app.read(|editor| editor.document.visit.working_view.support_ready()) {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        app.update(|editor| assert_coherent(editor, &last));
        complete_frame(
            &app.read(|editor| {
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .clone()
            }),
            &services,
        );
    });
    scheduler.close_and_wait().unwrap();
}
