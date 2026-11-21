use super::*;
mod preparation;
use crate::gpu::evaluation::{ImageLayout, camera_source};
use std::sync::mpsc;

#[test]
fn superseded_layouts_cannot_publish_and_cached_inputs_keep_visit_independent_ownership() {
    for warm in [false, true] {
        let (editor, pixels, base, scheduler) = fixture();
        let original = camera_source(&pixels.scan().unwrap()).unwrap();
        let key = original.identity().unwrap().clone();
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
        assert!(first.same_source(&latest));
        assert!(first != latest);
        let submissions = editor.submissions().clone();
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
        with_application(editor, |app| {
            if warm {
                app.enter(|editor, cx| request_working(editor, base.clone(), pixels.clone(), cx));
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
            }
            let before: Vec<_> = scope
                .ready::<ImageLayout>(&key)
                .into_iter()
                .map(|image| Arc::downgrade(&image.image))
                .collect();
            assert_eq!(!before.is_empty(), warm);
            if !warm {
                submissions.observe_submissions(Arc::new(move || {
                    if first_submission.swap(false, Ordering::Relaxed) {
                        started.send(()).unwrap();
                        resume
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(15))
                            .unwrap();
                    }
                }));
            }
            app.enter(|editor, cx| request_working(editor, first.clone(), pixels.clone(), cx));
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                app.pump();
                if starts.try_recv().is_ok()
                    || warm && app.read(|editor| editor.document.visit.working_view.support_ready())
                {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "first acquisition never submitted actual work"
                );
                std::thread::yield_now();
            }
            app.read(|editor| {
                assert_eq!(
                    editor
                        .document
                        .visit
                        .working_view
                        .mailbox
                        .0
                        .lock()
                        .unwrap()
                        .demand
                        .as_ref()
                        .unwrap()
                        .frame,
                    first.frame()
                )
            });
            app.enter(|editor, cx| request_working(editor, latest.clone(), pixels.clone(), cx));
            app.pump();
            if !warm {
                release.send(()).unwrap();
            }
            loop {
                app.pump();
                if !warm {
                    app.read(|editor| {
                        assert!(
                            editor
                                .document
                                .visit
                                .working_view
                                .frame
                                .as_ref()
                                .is_none_or(|frame| frame.source.spec != first),
                            "obsolete sampling cannot publish as intermediate progress"
                        )
                    });
                }
                if app.read(|editor| {
                    editor.document.visit.working_view.task.is_none()
                        && editor.document.visit.working_view.support_ready()
                }) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "latest layout did not complete"
                );
                std::thread::yield_now();
            }
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
            let after = scope.ready::<ImageLayout>(&key);
            if warm {
                assert!(
                    before.iter().any(|previous| after
                        .iter()
                        .any(|image| previous.ptr_eq(&Arc::downgrade(&image.image)))),
                    "view replacement does not withdraw ordinary cache ownership"
                );
            }
            assert!(
                renderer.evaluation().retained_bytes()
                    <= renderer.evaluation().cache_policy().generated_bytes
            );
        });
        scheduler.close_and_wait().unwrap();
    }
}
