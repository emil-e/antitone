use super::*;
mod preparation;
use crate::gpu::evaluation::{ImageBacking, ImageLayout, camera_source};
use std::sync::{atomic::AtomicU64, mpsc};

#[test]
fn superseded_layouts_retire_without_restarting_photo_owned_camera_preparation() {
    for warm in [false, true] {
        let (e, pixels, base, scheduler) = fixture();
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
        assert!(first.same_source(&latest));
        assert!(
            first != latest,
            "the control must actually replace the sampling layout"
        );
        let submissions = e.submissions.clone();
        let services = e.source_services.as_ref().unwrap();
        let renderer = services
            .renderers
            .get(&services.device, &services.queue)
            .unwrap();
        let scope = renderer.evaluation().materializations();
        let (started, starts) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let resume = Mutex::new(resume);
        let peak = Arc::new(AtomicU64::new(0));
        let observed = peak.clone();
        let observed_renderer = renderer.clone();
        with_application(e, |app| {
            if warm {
                app.enter(|e, cx| request_working(e, base.clone(), pixels.clone(), cx));
                app.run_until(Duration::from_secs(15), |e| {
                    e.document.visit.working_view.support_ready()
                });
            }
            let camera_before = scope
                .find_ready::<ImageLayout>(&key, |layout| layout.backing == ImageBacking::Texture);
            assert_eq!(camera_before.is_some(), warm);
            submissions.observe_submissions(Arc::new(move || {
                observed.fetch_max(
                    observed_renderer.evaluation().active_input_bytes(),
                    Ordering::Relaxed,
                );
                started.send(()).unwrap();
                resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(15))
                    .unwrap();
            }));
            app.enter(|e, cx| request_working(e, first.clone(), pixels.clone(), cx));
            let deadline = std::time::Instant::now() + Duration::from_secs(15);
            loop {
                app.pump();
                if starts.try_recv().is_ok() {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "first acquisition never reached a real submission"
                );
                std::thread::yield_now();
            }
            assert_eq!(
                peak.load(Ordering::Relaxed),
                bytes,
                "the gate must observe an admitted camera input, not unrelated setup"
            );
            app.read(|e| {
                assert!(
                    e.document
                        .visit
                        .working_view
                        .mailbox
                        .0
                        .lock()
                        .unwrap()
                        .demand
                        .as_ref()
                        .unwrap()
                        .spec
                        == first
                )
            });
            app.enter(|e, cx| request_working(e, latest.clone(), pixels.clone(), cx));
            app.pump();
            release.send(()).unwrap();
            loop {
                app.pump();
                app.read(|e|assert!(e.document.visit.working_view.frame.as_ref().is_none_or(|frame|frame.source.spec!=first),"an obsolete layout must not delay the newest view by publishing as intermediate progress"));
                if starts.try_recv().is_ok() {
                    release.send(()).unwrap();
                }
                if app.read(|e| {
                    e.document.visit.working_view.task.is_none()
                        && e.document.visit.working_view.support_ready()
                }) {
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "latest layout did not complete"
                );
                std::thread::yield_now();
            }
            app.read(|e| {
                assert!(
                    e.document
                        .visit
                        .working_view
                        .frame
                        .as_ref()
                        .unwrap()
                        .request
                        .spec
                        == latest
                )
            });
            assert_eq!(renderer.evaluation().active_input_bytes(), bytes);
            assert!(
                peak.load(Ordering::Relaxed) <= bytes,
                "replacing a viewport must not admit duplicate camera uploads"
            );
            let camera_after = scope
                .find_ready::<ImageLayout>(&key, |layout| layout.backing == ImageBacking::Texture)
                .unwrap();
            if let Some(before) = camera_before {
                assert!(Arc::ptr_eq(&before.image, &camera_after.image));
            }
        });
        scheduler.close_and_wait().unwrap();
    }
}
