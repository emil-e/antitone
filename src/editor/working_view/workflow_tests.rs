use super::*;
use crate::{
    application::test_support::with_application,
    geometry::{Composition, FramePoint},
};
use std::sync::atomic::Ordering;

fn fixture() -> (
    Editor,
    source::Context,
    ViewSourceSpec,
    source_loading::SourceLoader,
) {
    let mut e = crate::editor::tests::fixture();
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
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
    let spec = ViewSourceSpec::inspection(e.id, frame, scan.camera_to_ap1, None)
        .unwrap()
        .with_identity(crate::gpu::execution::source::Identity::camera(
            scan.source().identity(),
        ));
    let input = source::Context {
        reference: None,
        source: scan.source().clone(),
        correction: None,
    };
    let scheduler = source_loading::SourceLoader::new();
    e.source_services = Some(source::Services {
        device: device.clone(),
        queue: queue.clone(),
        developments: scheduler.clone(),
        renderers: Arc::default(),
    });
    e.document.visit.working_view = WorkingView::new(device, queue.clone());
    e.source_epoch.store(e.id, Ordering::Relaxed);
    (e, input, spec, scheduler)
}

fn positive_spec(
    authority: u64,
    frame: FrameRaster,
    input: &source::Context,
    sigma: f64,
) -> ViewSourceSpec {
    let scan = input.scan().unwrap();
    ViewSourceSpec::new(
        authority,
        frame,
        crate::processing::InputSettings::Digital,
        scan.positive_matrix(&crate::processing::InputSettings::Digital)
            .unwrap(),
        None,
    )
    .unwrap()
    .with_pipeline(crate::pixel_pipeline::PixelPipeline {
        stages: vec![crate::pixel_pipeline::PixelStage::new(
            crate::image_mtf::ImageMtfParameters { sigma },
        )],
    })
    .unwrap()
    .with_identity(crate::gpu::execution::source::Identity::camera(
        input.source.identity(),
    ))
}

#[test]
fn parked_support_growth_rejects_stale_layouts_and_smaller_demand_reuses_acquired_neighbours() {
    let (e, input, inspection, scheduler) = fixture();
    let mut frame = inspection.frame();
    frame.output = std::array::from_fn(|i| (frame.max.0[i] - frame.min.0[i]).ceil() as u32);
    let small = positive_spec(e.id, frame, &input, 0.25);
    let queued = positive_spec(e.id, frame, &input, 0.5);
    let latest = positive_spec(e.id, frame, &input, 0.75);
    assert!(
        !small.compatible_processing(&latest),
        "fixture must actually increase required raster support"
    );
    assert!(
        small.same_source(&latest),
        "support is not upstream content identity"
    );
    with_application(e, |app| {
        app.enter(|e, access| e.request_working(small.clone(), input.clone(), access));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let old = app.read(|e| {
            e.document
                .visit
                .working_view
                .source
                .as_ref()
                .unwrap()
                .clone()
        });
        let hold = crate::background::testing::pause();
        app.enter(|e, access| e.request_working(queued.clone(), input.clone(), access));
        app.pump();
        app.enter(|e, access| e.request_working(latest.clone(), input.clone(), access));
        app.pump();
        app.update(|e| {
            assert!(!e.document.visit.working_view.support_ready());
            assert!(
                e.document.visit.working_view.underlay.is_none(),
                "old underlay lacks the new neighbourhood"
            );
            assert!(e.document.visit.working.task.is_some());
            e.document.visit.working_view.acquired(Completion {
                spec: queued.clone(),
                result: Ok(old.clone()),
            });
            assert!(
                e.document.visit.working_view.error.is_none(),
                "obsolete completion cannot fail current demand"
            );
            // Publication must validate actual coverage as well as the completion ticket.
            e.document.visit.working_view.acquired(Completion {
                spec: latest.clone(),
                result: Ok(old.clone()),
            });
            assert!(!e.document.visit.working_view.support_ready());
            assert!(e.document.visit.working_view.error.is_some());
            assert!(Arc::ptr_eq(
                &old,
                e.document.visit.working_view.source.as_ref().unwrap()
            ));
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let acquired = app.read(|e| {
            assert!(e.document.visit.working_view.error.is_none());
            assert!(
                e.document
                    .visit
                    .working_view
                    .source
                    .as_ref()
                    .unwrap()
                    .spec
                    .satisfies(&latest)
            );
            assert!(
                e.document
                    .visit
                    .working_view
                    .underlay
                    .as_ref()
                    .unwrap()
                    .spec
                    .compatible_processing(&latest)
            );
            e.document
                .visit
                .working_view
                .source
                .as_ref()
                .unwrap()
                .clone()
        });
        app.enter(|e, access| e.request_working(small.clone(), input, access));
        app.pump();
        app.read(|e| {
            assert!(e.document.visit.working_view.support_ready());
            assert!(
                e.document.visit.working.task.is_none(),
                "smaller support does not reacquire an already sufficient input"
            );
            assert!(Arc::ptr_eq(
                &acquired,
                e.document.visit.working_view.source.as_ref().unwrap()
            ));
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn support_growth_during_submission_cancels_obsolete_units_before_publishing_latest_support() {
    use crate::{
        acquisition::{FootprintBatch, Plan, Request, Sampling},
        pixel_pipeline::{PixelPipeline, PixelStage},
    };
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
        correction: None,
    };
    let small = positive_spec(e.id, frame, &input, 0.25);
    let latest = positive_spec(e.id, frame, &input, 0.75);
    let count_work = |spec: &ViewSourceSpec, sigma| {
        let stages = PixelPipeline {
            stages: vec![PixelStage::new(crate::image_mtf::ImageMtfParameters {
                sigma,
            })],
        };
        let plans = [spec.frame(), spec.underlay().unwrap().frame()]
            .into_iter()
            .map(|output| {
                Arc::new(
                    Plan::new(Request {
                        output,
                        stages: &stages,
                        sampling: Sampling::NeutralFootprints,
                    })
                    .unwrap(),
                )
            })
            .collect();
        FootprintBatch::new(plans, Default::default())
            .unwrap()
            .units()
            .map(|unit| unit.unwrap().submissions.len())
            .sum::<usize>()
    };
    assert!(
        count_work(&small, 0.25) > 1,
        "fixture must leave obsolete work after the parked submission"
    );
    let expected = 1 + count_work(&latest, 0.75);
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    e.submissions.observe_submissions(Arc::new(move || {
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
        app.enter(|e, access| e.request_working(small.clone(), input.clone(), access));
        app.pump();
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        app.enter(|e, access| e.request_working(latest.clone(), input, access));
        app.pump();
        app.read(|e| {
            assert!(!e.document.visit.working_view.support_ready());
            let state = e.document.visit.working_view.mailbox.0.lock().unwrap();
            assert!(
                state.neutral.as_ref().is_none_or(|cache| cache.is_empty())
                    && state.underlay.is_none(),
                "partially acquired buffers must remain private"
            );
            assert!(
                e.document.visit.working.task.is_some(),
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
                    .source
                    .as_ref()
                    .unwrap()
                    .spec
                    .satisfies(&latest)
            );
            assert!(
                e.document
                    .visit
                    .working_view
                    .underlay
                    .as_ref()
                    .unwrap()
                    .spec
                    .compatible_processing(&latest)
            );
            assert!(e.document.visit.working.task.is_none());
        });
    });
    // The planner's geometry is protected by independent coverage oracles; this separately
    // checks that execution follows its declared work without completing superseded units.
    assert_eq!(count.load(Ordering::Relaxed), expected);
    scheduler.close_and_wait().unwrap();
}

#[test]
fn current_ticket_reuses_captured_support_and_rejects_the_previous_ticket_completion() {
    let (e, input, spec, scheduler) = fixture();
    with_application(e, |app| {
        app.enter(|e, cx| e.request_working(spec.clone(), input.clone(), cx));
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let mut current = spec.clone();
        current.authority += 1;
        app.update(|e| {
            e.document
                .visit
                .working_view
                .request(current.clone(), input);
            assert!(e.document.visit.working_view.support_ready());
            assert!(
                e.document
                    .visit
                    .working_view
                    .acquisition(current.authority)
                    .is_none(),
                "new rights do not reacquire equal content"
            );
            assert_eq!(
                e.document
                    .visit
                    .working_view
                    .source
                    .as_ref()
                    .unwrap()
                    .spec
                    .authority,
                current.authority
            );
            e.document.visit.working_view.acquired(Completion {
                spec,
                result: Err("departed owner".into()),
            });
            assert!(
                e.document.visit.working_view.error.is_none(),
                "equal content cannot restore a departed continuation's rights"
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn physical_support_completes_without_backend_or_another_paint() {
    let (e, input, spec, scheduler) = fixture();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.request_working(spec.clone(), input, access);
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            assert!(e.document.visit.working_view.source.as_ref().unwrap().spec == spec);
            assert!(e.document.visit.working_view.underlay.is_some());
            assert!(e.document.visit.working.task.is_none());
            assert!(
                e.document.visit.working_view.displayed.is_none(),
                "colour capture remains frame-bound"
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_navigation_uses_latest_demand_and_reuses_neutral_support_after_grade_output_changes() {
    let (e, input, mut spec, scheduler) = fixture();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        for _ in 0..40 {
            let mut moved_frame = spec.frame();
            moved_frame.min.0[0] += 1.;
            moved_frame.max.0[0] += 1.;
            spec = spec.with_frame(moved_frame).unwrap();
            app.enter(|e, access| {
                e.request_working(spec.clone(), input.clone(), access);
            });
            app.pump();
            app.read(|e| {
                assert!(!e.document.visit.working_view.support_ready());
                assert!(e.document.visit.working.task.is_some());
            });
        }
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| {
            e.document.visit.working_view.support_ready()
        });
        let neutral = app.read(|e| {
            assert!(e.document.visit.working_view.source.as_ref().unwrap().spec == spec);
            e.document.visit.working_view.neutral_source().unwrap()
        });
        app.enter(|e, access| {
            e.document.target = OutputTarget::Srgb;
            e.source_exposure = 1.2;
            let processing =
                ProcessingInput::from(ProcessingParameters::source_inspection(1.2).unwrap());
            e.observe_grade(&processing);
            e.request_working(spec.clone(), input, access);
        });
        app.pump();
        app.read(|e| {
            assert!(
                e.document.visit.working.task.is_none(),
                "output-only demand must not reacquire originals"
            );
            assert!(
                neutral.same_pixels_as(&e.document.visit.working_view.neutral_source().unwrap())
            );
        });
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
        correction: None,
    };
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    e.submissions.observe_submissions(Arc::new(move || {
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
            e.request_working(spec.clone(), input, access);
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        let mut moved_frame = spec.frame();
        moved_frame.min.0[0] += 1.;
        moved_frame.max.0[0] += 1.;
        spec = spec.with_frame(moved_frame).unwrap();
        app.enter(|e, access| {
            e.replace_photo_scope(8);
            e.request_working(spec.clone(), next, access);
        });
        app.pump();
        assert!(source.upgrade().is_some());
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "a running acquisition keeps its source lease"
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
            assert!(e.document.visit.working_view.source.as_ref().unwrap().spec == spec);
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
            e.request_working(spec, input, access);
        });
        app.pump();
        app.update(|e| e.replace_photo_scope(8));
        app.pump();
        assert!(source.upgrade().is_none());
        drop(hold);
        app.pump();
        app.read(|e| {
            assert!(
                e.document.visit.working_view.source.is_none()
                    && e.document.visit.working_view.underlay.is_none()
            );
            assert!(e.document.visit.working.task.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn save_completion_readmits_existing_working_demand_without_another_paint() {
    let (e, input, spec, scheduler) = fixture();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.antitone");
    with_application(e, |app| {
        app.enter(|e, cx| {
            e.save_project(path.clone(), crate::project::Save::New, None, cx);
            e.request_working(spec.clone(), input, cx);
            assert!(e.document_pending.is_some());
            assert!(
                e.document.visit.working.task.is_none(),
                "working-source dispatch is paused"
            );
        });
        app.run_until(Duration::from_secs(15), |e| {
            e.document_pending.is_none() && e.document.visit.working_view.support_ready()
        });
        app.read(|e| {
            assert!(e.document.visit.working_view.source.as_ref().unwrap().spec == spec);
            assert!(e.document.visit.working.task.is_none() && !e.busy);
            assert!(e.failure.is_none(), "{}", e.status);
        });
        assert!(path.is_file());
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn guarded_save_close_does_not_readmit_waiting_working_demand() {
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
            e.request_working(spec, input, cx);
            assert!(e.document.visit.working.task.is_none());
        });
        app.run_until(Duration::from_secs(15), |e| e.allow_close);
        app.pump();
        app.read(|e| {
            assert!(e.document_pending.is_none() && !e.busy);
            assert!(e.document.visit.working.task.is_none());
            assert!(
                e.document.visit.working_view.source.is_none(),
                "close must not start another acquisition"
            );
        });
        assert!(path.is_file());
    });
    scheduler.close_and_wait().unwrap();
}
