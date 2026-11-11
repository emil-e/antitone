use super::*;
use std::{
    future::Future,
    sync::atomic::AtomicUsize,
    task::{Context, Wake, Waker},
};

#[derive(Default)]
struct Wakes(AtomicUsize);

impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn viewport_replacement_keeps_an_inflight_camera_preparation_alive() {
    preparation_survives_view_change(false);
}

#[test]
fn retiring_the_view_does_not_retire_photo_preparation_or_its_completed_pixels() {
    preparation_survives_view_change(true);
}

#[test]
fn source_and_visit_replacement_retire_preparation_without_leaking_submitted_inputs() {
    for replace_visit in [false, true] {
        let (editor, pixels, base, scheduler) = fixture();
        let old_source = Arc::downgrade(&pixels.source);
        let original = camera_source(&pixels.scan().unwrap()).unwrap();
        let old_key = original.identity().unwrap().clone();
        drop(original);
        let authored = crate::dng_fixture::Fixture {
            side: 128,
            ramp: true,
            ..Default::default()
        }
        .write();
        let scan = Scan::open(authored.path(), None).unwrap();
        let composition =
            Composition::new(scan.area, Default::default(), Default::default()).unwrap();
        let next = SourceRequest::inspection_fixture(
            FrameRaster::new(
                composition,
                FramePoint([0., 0.]),
                FramePoint(composition.size().map(f64::from)),
                [64, 48],
            )
            .unwrap(),
            scan.camera_to_ap1,
            None,
        )
        .unwrap();
        let input = source::Context {
            reference: None,
            source: scan.source().clone(),
            correction: None,
        };
        let original = camera_source(&scan).unwrap();
        let new_key = original.identity().unwrap().clone();
        assert_ne!(
            old_key, new_key,
            "replacement must have distinct camera mathematics"
        );
        let bytes = u64::from(scan.area.width)
            * u64::from(scan.area.height)
            * original.source().unwrap().storage_format().pixel_bytes();
        let services = editor.source_services.as_ref().unwrap().clone();
        let renderer = services
            .renderers
            .get(&services.device, &services.queue)
            .unwrap();
        let scope = renderer.evaluation().materializations();
        scope.set_retention_enabled(false);
        let (started, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let first = std::sync::atomic::AtomicBool::new(true);
        editor.submissions.observe_submissions(Arc::new(move || {
            if first.swap(false, std::sync::atomic::Ordering::Relaxed) {
                started.send(()).unwrap();
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(15))
                    .unwrap();
            }
        }));
        with_application(editor, |app| {
            app.enter(|editor, cx| request_working(editor, base, pixels, cx));
            let ready = app.read(|editor| editor.document.visit.camera.ready().unwrap());
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                app.pump();
                if starts.try_recv().is_ok() {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            assert!(!ready.finished());
            let mut waiting = Box::pin(ready.wait());
            let wakes = Arc::new(Wakes::default());
            let waker = Waker::from(wakes.clone());
            let mut task = Context::from_waker(&waker);
            assert!(waiting.as_mut().poll(&mut task).is_pending());
            assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
            app.enter(|editor, cx| {
                if replace_visit {
                    editor.replace_photo_scope(8);
                    editor
                        .document
                        .visit
                        .camera
                        .pressure(memory::Pressure::Normal);
                }
                request_working(editor, next, input, cx);
            });
            assert!(
                wakes.0.load(Ordering::Relaxed) > 0,
                "retirement must deliver the registered wake before cleanup is released"
            );
            assert!(
                !pollster::block_on(futures_lite::future::poll_once(waiting.as_mut()))
                    .expect("retired preparation left its waiter pending")
                    .unwrap(),
                "retired preparation must wake and release weak waiters before physical cleanup"
            );
            assert!(
                old_source.upgrade().is_some(),
                "submitted work must still own the source after logical retirement"
            );
            release.send(()).unwrap();
            app.run_until(Duration::from_secs(15), |editor| {
                editor.document.visit.working_view.support_ready()
            });
            loop {
                app.pump();
                if old_source.upgrade().is_none() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "retired physical work leaked its source"
                );
                std::thread::yield_now();
            }
            assert!(
                scope
                    .find_ready::<ImageLayout>(&old_key, |_| true)
                    .is_none()
            );
            assert!(
                scope
                    .find_ready::<ImageLayout>(&new_key, |layout| layout.backing
                        == ImageBacking::Texture)
                    .is_some()
            );
            assert_eq!(renderer.evaluation().active_input_bytes(), bytes);
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn pressure_releases_a_waiting_view_to_bounded_execution_and_retires_only_preparation() {
    let (editor, pixels, base, scheduler) = fixture();
    let services = editor.source_services.as_ref().unwrap().clone();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    renderer
        .evaluation()
        .materializations()
        .set_retention_enabled(false);
    let (started, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let first = std::sync::atomic::AtomicBool::new(true);
    editor.submissions.observe_submissions(Arc::new(move || {
        if first.swap(false, std::sync::atomic::Ordering::Relaxed) {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
    }));
    with_application(editor, |app| {
        app.enter(|editor, cx| request_working(editor, base, pixels, cx));
        let ready = app.read(|editor| editor.document.visit.camera.ready().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!ready.finished());
        assert!(renderer.evaluation().active_input_bytes() > 0);
        let wakes = Arc::new(Wakes::default());
        let waker = Waker::from(wakes.clone());
        let mut task = Context::from_waker(&waker);
        let mut waiting = Box::pin(ready.wait());
        assert!(waiting.as_mut().poll(&mut task).is_pending());
        assert_eq!(wakes.0.load(Ordering::Relaxed), 0);
        app.update(|editor| {
            editor
                .document
                .visit
                .camera
                .pressure(memory::Pressure::Warning)
        });
        assert!(
            wakes.0.load(Ordering::Relaxed) > 0,
            "pressure must deliver the wake of an already parked view"
        );
        assert!(
            pollster::block_on(futures_lite::future::poll_once(waiting.as_mut()))
                .expect("pressure left its view waiter pending")
                .unwrap(),
            "pressure must wake the view rather than withdraw its publication rights"
        );
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        assert_eq!(renderer.evaluation().active_input_bytes(), 0);
        app.read(|editor| assert!(editor.document.visit.working_view.error.is_none()));
    });
    scheduler.close_and_wait().unwrap();
}

fn preparation_survives_view_change(retire_view: bool) {
    let (editor, pixels, base, scheduler) = fixture();
    let scan = pixels.scan().unwrap();
    let original = camera_source(&scan).unwrap();
    let key = original.identity().unwrap().clone();
    let bytes = u64::from(scan.area.width)
        * u64::from(scan.area.height)
        * original.source().unwrap().storage_format().pixel_bytes();
    let make = |low, high| {
        base.clone()
            .with_frame(
                FrameRaster::new(
                    base.frame().composition,
                    FramePoint(low),
                    FramePoint(high),
                    [64, 48],
                )
                .unwrap(),
            )
            .unwrap()
    };
    let first = make([16., 16.], [112., 88.]);
    let latest = make([24., 20.], [120., 92.]);
    let services = editor.source_services.as_ref().unwrap().clone();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let scope = renderer.evaluation().materializations();
    scope.set_retention_enabled(false);
    let gate = editor.submissions.clone();
    let (started, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let first_submission = std::sync::atomic::AtomicBool::new(true);
    gate.observe_submissions(Arc::new(move || {
        if first_submission.swap(false, std::sync::atomic::Ordering::Relaxed) {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
    }));
    with_application(editor, |app| {
        app.enter(|editor, cx| request_working(editor, first, pixels.clone(), cx));
        let ready = app.read(|editor| editor.document.visit.camera.ready().unwrap());
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "camera preparation did not submit"
            );
            std::thread::yield_now();
        }
        assert_eq!(renderer.evaluation().active_input_bytes(), bytes);
        assert!(!ready.finished(), "the upload must genuinely be in flight");
        assert!(pollster::block_on(futures_lite::future::poll_once(ready.wait())).is_none());
        let before = if retire_view {
            app.update(|editor| editor.document.visit.working_view.clear());
            app.read(|editor| {
                assert!(editor.document.visit.working_view.task.is_none());
                assert!(editor.document.visit.working_view.acquisition().is_none());
            });
            release.send(()).unwrap();
            app.run_until(Duration::from_secs(15), |_| ready.finished());
            assert!(pollster::block_on(ready.wait()).unwrap());
            Some(
                scope
                    .find_ready::<ImageLayout>(&key, |layout| {
                        layout.backing == ImageBacking::Texture
                    })
                    .unwrap(),
            )
        } else {
            None
        };
        app.enter(|editor, cx| request_working(editor, latest.clone(), pixels, cx));
        let after = app.read(|editor| editor.document.visit.camera.ready().unwrap());
        assert!(
            ready.same_preparation(&after),
            "view changes must not replace photo preparation"
        );
        if !retire_view {
            assert!(!ready.finished());
            release.send(()).unwrap();
        }
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        app.read(|editor| {
            assert!(
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .request
                    .spec
                    == latest
            );
        });
        let after = scope
            .find_ready::<ImageLayout>(&key, |layout| layout.backing == ImageBacking::Texture)
            .unwrap();
        if let Some(before) = before {
            assert!(
                Arc::ptr_eq(&before.image, &after.image),
                "the photo's completed allocation must remain discoverable across view reacquisition at zero optional retention"
            );
        }
        assert_eq!(renderer.evaluation().active_input_bytes(), bytes);
    });
    scheduler.close_and_wait().unwrap();
}
