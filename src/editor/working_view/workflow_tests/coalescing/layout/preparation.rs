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
fn viewport_replacement_retires_old_publication_without_dropping_submitted_work() {
    preparation_survives_view_change(false);
}

#[test]
fn retiring_a_view_wakes_waiters_and_a_new_view_can_use_common_materializations() {
    preparation_survives_view_change(true);
}

#[test]
fn source_and_visit_replacement_retire_old_work_without_pinning_its_source_in_the_cache() {
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
        assert_ne!(old_key, new_key);
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
            if first.swap(false, Ordering::Relaxed) {
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
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                app.pump();
                if starts.try_recv().is_ok() {
                    break;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            let mailbox = app.read(|editor| editor.document.visit.working_view.mailbox.clone());
            let mut waiting = Box::pin(mailbox.1.listen());
            let wakes = Arc::new(Wakes::default());
            let waker = Waker::from(wakes.clone());
            let mut task = Context::from_waker(&waker);
            assert!(waiting.as_mut().poll(&mut task).is_pending());
            app.enter(|editor, cx| {
                if replace_visit {
                    editor.replace_photo_scope(8);
                }
                request_working(editor, next.clone(), input.clone(), cx);
            });
            assert!(
                wakes.0.load(Ordering::Relaxed) > 0,
                "retirement must wake logical waiters before physical cleanup is released"
            );
            assert!(waiting.as_mut().poll(&mut task).is_ready());
            assert!(
                old_source.upgrade().is_some(),
                "submitted work keeps its captured source through cleanup"
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
                    "retired work leaked its source"
                );
                std::thread::yield_now();
            }
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
                        .pixels
                        .source
                        .identity()
                        == input.source.identity()
                )
            });
            assert!(
                scope
                    .find_ready::<ImageLayout>(&old_key, |_| true)
                    .is_none()
            );
            assert_eq!(renderer.evaluation().retained_bytes(), 0);
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn pressure_retires_optional_ownership_without_withdrawing_pending_frame_publication() {
    let (editor, pixels, base, scheduler) = fixture();
    let services = editor.source_services.as_ref().unwrap().clone();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    let scope = renderer.evaluation().materializations();
    scope.set_test_pressure(memory::Pressure::Normal);
    let (started, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let first = std::sync::atomic::AtomicBool::new(true);
    editor.submissions.observe_submissions(Arc::new(move || {
        if first.swap(false, Ordering::Relaxed) {
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
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        scope.set_test_pressure(memory::Pressure::Warning);
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
        app.read(|editor| {
            assert!(!editor.document.visit.working_view.support_ready());
            assert!(editor.document.visit.working_view.task.is_some());
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        assert_eq!(renderer.evaluation().retained_bytes(), 0);
        app.read(|editor| assert!(editor.document.visit.working_view.error.is_none()));
    });
    scheduler.close_and_wait().unwrap();
}

fn preparation_survives_view_change(retire_view: bool) {
    let (editor, pixels, base, scheduler) = fixture();
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
    scope.set_test_pressure(memory::Pressure::Normal);
    let (started, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let first_submission = std::sync::atomic::AtomicBool::new(true);
    editor.submissions.observe_submissions(Arc::new(move || {
        if first_submission.swap(false, Ordering::Relaxed) {
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
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "production did not submit"
            );
            std::thread::yield_now();
        }
        if retire_view {
            let mailbox = app.read(|editor| editor.document.visit.working_view.mailbox.clone());
            let mut waiting = Box::pin(mailbox.1.listen());
            let wakes = Arc::new(Wakes::default());
            let waker = Waker::from(wakes.clone());
            let mut task = Context::from_waker(&waker);
            assert!(waiting.as_mut().poll(&mut task).is_pending());
            app.update(|editor| editor.document.visit.working_view.clear());
            assert!(wakes.0.load(Ordering::Relaxed) > 0);
            assert!(waiting.as_mut().poll(&mut task).is_ready());
            app.read(|editor| {
                assert!(editor.document.visit.working_view.task.is_none());
                assert!(editor.document.visit.working_view.acquisition().is_none());
            });
        }
        app.enter(|editor, cx| request_working(editor, latest.clone(), pixels, cx));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.visit.working_view.support_ready()
        });
        app.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .visit
                    .working_view
                    .frame
                    .as_ref()
                    .unwrap()
                    .request
                    .frame,
                latest.frame()
            )
        });
        assert!(
            renderer.evaluation().retained_bytes()
                <= renderer.evaluation().cache_policy().generated_bytes
        );
    });
    scheduler.close_and_wait().unwrap();
}
