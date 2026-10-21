use super::*;
use crate::application::Application;
use crate::editor::{development, thumbnails};
use crate::{blocking_jobs::BlockingJobs, raw::Rect, source_store::SourceStore};
use std::{
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn application<'scope>() -> (Application<'scope>, mpsc::Receiver<()>) {
    let (wake, wakes) = mpsc::channel();
    (
        Application::new(
            move || {
                let _ = wake.send(());
            },
            || {},
        ),
        wakes,
    )
}

fn drive_until(
    application: &mut Application<'_>,
    wakes: &mpsc::Receiver<()>,
    done: impl Fn() -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        wakes
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
        application.drive();
    }
}

#[test]
fn measurement_uses_the_captured_source_and_updates_history_without_receiving_or_painting() {
    pollster::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("working.dng");
        std::fs::copy(fixture("illuminated-patches.dng"), &path).unwrap();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let renderers = Arc::new(crate::renderer::RendererCache::new(Default::default()));
        let (ready, mut scan) = crate::editor::prepare_source(
            &renderers,
            &device,
            &queue,
            path.clone(),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let region = Coverage::rectangle(Rect {
            x: 16,
            y: 16,
            width: 32,
            height: 32,
        });
        let expected = scan.reference_coverage(region).unwrap();
        let dark = scan
            .reference(Rect {
                x: 80,
                y: 16,
                width: 32,
                height: 32,
            })
            .unwrap();
        let lease = Arc::downgrade(scan.source());
        let mut editor = Editor::state();
        let scheduler = development::Scheduler::new();
        editor.source_services = Some(crate::editor::source::Services {
            developments: scheduler.clone(),
            jobs: editor.workflow_jobs.clone(),
            renderers,
            device: device.clone(),
            queue: queue.clone(),
        });
        editor.current_source = Some(
            editor
                .document
                .register(ready.record.clone(), ready.development.clone())
                .unwrap(),
        );
        editor.floor = crate::processing::CAMERA_INPUT_FLOOR;
        editor.dmin = Some(expected * 1.05);
        editor.dmax = Some(dark);
        editor.document.visit.ready = Some(ready);
        editor.sync_document();
        editor.document.cache().set_budget(0);
        drop(scan);
        let hold = crate::editor::tests::hold_blocking_work(&editor.workflow_jobs);
        crate::application::test_support::with_application(editor, |app| {
            let tasks = app.access();
            app.enter(|editor, cx| {
                editor.begin_pick(Pick::Dmin, cx);
                editor.region = Some(region);
                editor.start_measurement(Pick::Dmin, region, &tasks);
                assert!(editor.document.visit.measurement.is_some() && editor.busy);
                editor.document.visit.ready = None;
            });
            std::fs::write(&path, b"replaced after the operation captured its input").unwrap();
            assert!(lease.upgrade().is_some());
            app.pump();
            assert!(app.read(|editor| editor.document.visit.measurement.is_some()));
            assert_eq!(
                app.read(|editor| editor.document.cache().charges().retained),
                0
            );
            drop(hold);
            app.run_until(Duration::from_secs(5), |editor| !editor.busy);
            app.read(|editor| {
                assert_eq!(editor.dmin, Some(expected), "{}", editor.status);
                assert!(editor.pick.is_none() && editor.document.visit.measurement.is_none());
                assert_eq!(
                    editor.history_label(crate::editor::history::Direction::Undo),
                    Some("Calibration pick")
                );
            });
            assert!(
                lease.upgrade().is_none(),
                "the completed operation must release its source"
            );
        });
        scheduler.close_and_wait().unwrap();
    });
}

#[test]
fn cancelling_an_unpolled_measurement_releases_its_input_without_admission() {
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    let source = Arc::downgrade(&editor.document.visit.ready.as_ref().unwrap().developed);
    let before = editor.document.project.clone();
    editor.begin_pick(Pick::Dmin, &crate::application::UiContext::render_only());
    editor.region = Some(Coverage::rectangle(
        editor.document.visit.ready.as_ref().unwrap().area,
    ));
    crate::application::test_support::with_application(editor, |app| {
        let tasks = app.access();
        app.update(|editor| {
            editor.start_measurement(Pick::Dmin, editor.region.unwrap(), &tasks);
            assert!(editor.busy && editor.document.visit.measurement.is_some());
        });
        app.enter(|editor, cx| {
            editor.finish_pick(cx);
            editor.document.visit.ready = None;
        });
        app.pump();
        app.read(|editor| {
            assert!(editor.document.visit.measurement.is_none() && !editor.busy);
            assert!(editor.workflow_jobs.idle());
            assert_eq!(editor.document.project, before);
        });
        assert!(source.upgrade().is_none());
    });
}

#[test]
fn service_rejection_retires_the_pick_without_changing_processing_or_history() {
    let mut editor = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.sync_document();
    let area = Scan::from_source(
        editor
            .document
            .visit
            .ready
            .as_ref()
            .unwrap()
            .developed
            .clone(),
    )
    .area;
    let region = Coverage::rectangle(Rect {
        x: area.x + area.width + 16,
        y: area.y + area.height + 16,
        width: 4,
        height: 4,
    });
    let expected_error = editor
        .measurement_input(Pick::Dmin, region)
        .unwrap()
        .measure()
        .err()
        .expect("coverage outside the sensor must be rejected")
        .to_string();
    let before = editor.document.project.clone();
    let undo = editor
        .history_label(crate::editor::history::Direction::Undo)
        .map(str::to_owned);
    crate::application::test_support::with_application(editor, |app| {
        let tasks = app.access();
        app.enter(|editor, cx| {
            editor.begin_pick(Pick::Dmin, cx);
            editor.region = Some(region);
            editor.start_measurement(Pick::Dmin, region, &tasks);
            assert!(editor.busy && editor.document.visit.measurement.is_some());
        });
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert_eq!(editor.status, expected_error);
            assert!(editor.pick.is_none() && editor.document.visit.measurement.is_none());
            assert!(editor.tasks.view().is_none());
            assert_eq!(editor.document.project, before);
            assert_eq!(
                editor.history_label(crate::editor::history::Direction::Undo),
                undo.as_deref()
            );
        });
    });
}

#[test]
fn cancelling_or_replacing_a_queued_measurement_does_not_finish_new_work() {
    for replace in [false, true] {
        let mut editor =
            crate::editor::tests::with_document_services(crate::editor::tests::fixture());
        crate::editor::tests::add_ready_fixture(&mut editor);
        let source = Arc::downgrade(&editor.document.visit.ready.as_ref().unwrap().developed);
        let hold = crate::editor::tests::hold_blocking_work(&editor.workflow_jobs);
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, access| {
                editor.begin_pick(Pick::Dmin, access);
                editor.region = Some(Coverage::rectangle(Rect {
                    x: 12,
                    y: 7,
                    width: 32,
                    height: 32,
                }));
                editor.start_measurement(Pick::Dmin, editor.region.unwrap(), access);
                assert!(editor.document.visit.measurement.is_some());
            });
            app.pump();
            let (status, activity) = app.enter(|editor, cx| {
                if replace {
                    editor.load("next.dng".into(), cx);
                    assert!(editor.document.visit.source.pending());
                } else {
                    editor.finish_pick(cx);
                    editor.document.visit.ready = None;
                }
                assert!(editor.document.visit.measurement.is_none());
                assert_eq!(editor.busy, replace);
                (
                    editor.status.clone(),
                    editor.tasks.view().map(|view| view.title),
                )
            });
            app.run_until(Duration::from_secs(5), |_| source.upgrade().is_none());
            drop(hold);
            app.read(|editor| {
                assert_eq!(editor.busy, replace);
                assert_eq!(editor.status, status);
                assert_eq!(editor.tasks.view().map(|view| view.title), activity);
            });
        });
    }
}

#[test]
fn cancelling_a_running_measurement_retains_its_source_until_the_computation_returns() {
    let scan = Scan::open(&fixture("patches.dng"), None).unwrap();
    let source = scan.source().clone();
    let lease = Arc::downgrade(&source);
    let input = Input {
        source,
        correction: None,
        kind: Pick::Dmin,
        region: Coverage::rectangle(scan.area),
        calibration: None,
        roll: RollSettings::new_grade(),
        photo: PhotoSettings::default(),
    };
    drop(scan);
    let state = std::cell::RefCell::new(Some(()));
    let (mut application, wakes) = application();
    let access = application.access(&state);
    let jobs = BlockingJobs::new(1);
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (finished, finishes) = mpsc::channel();
    let task = access
        .spawn(move |_| async move {
            jobs.run(move || {
                entered.send(()).unwrap();
                resume.recv().unwrap();
                let result = input.measure();
                finished.send(()).unwrap();
                result
            })
            .await
        })
        .unwrap();
    application.drive();
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(task);
    application.drive();
    assert!(lease.upgrade().is_some());
    release.send(()).unwrap();
    finishes.recv_timeout(Duration::from_secs(5)).unwrap();
    drive_until(&mut application, &wakes, || lease.upgrade().is_none());
}

#[test]
fn native_acquisition_wait_does_not_occupy_blocking_job_capacity() {
    let file = crate::raw::fixture::bayer_dng();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let sources = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(())
    });
    let store = Arc::new(SourceStore::new(0));
    let ticket = sources
        .submit(
            development::Request {
                path: file.path().to_owned(),
                wb: Some([1.; 4]),
                expected: None,
                basis: None,
                file: thumbnails::FileKey::new(file.path()),
                key: None,
                store: Some(store.clone()),
                parent: None,
                retain: false,
            },
            development::Priority::Foreground,
        )
        .unwrap();
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let executor = async_executor::LocalExecutor::new();
    let jobs = BlockingJobs::new(1);
    let measurement_jobs = jobs.clone();
    let measurement = executor.spawn(async move {
        let source = ticket.await.map_err(|error| error.to_string())?;
        let scan = Scan::from_source(source.clone());
        // The SDK marks repeated/interpolated border samples ineligible. Admission
        // is the subject here, so measure an ordinary interior rather than borders.
        let area = scan.area;
        let region = Rect {
            x: area.x + 8,
            y: area.y + 8,
            width: area.width - 16,
            height: area.height - 16,
        };
        let input = Input {
            source,
            correction: None,
            kind: Pick::Dmin,
            region: Coverage::rectangle(region),
            calibration: None,
            roll: RollSettings::new_grade(),
            photo: PhotoSettings::default(),
        };
        measurement_jobs
            .run(move || input.measure().map_err(|error| error.to_string()))
            .await
            .map_err(|_| "worker panicked".to_owned())?
    });
    let unrelated = executor.spawn(async move { jobs.run(|| 42).await });
    let value = futures_lite::future::block_on(executor.run(async {
        assert_eq!(unrelated.await.unwrap(), 42);
        release.send(()).unwrap();
        measurement.await.unwrap()
    }));
    let Measurement::Reference(value) = value else {
        panic!("expected reference")
    };
    assert!(value.is_finite() && value.min_element() > 0.);
    assert_eq!(store.charges().retained, 0);
}

#[test]
fn measurement_captures_correction_and_all_five_kinds_keep_their_declared_inputs() {
    let mut scan = Scan::open(&fixture("illuminated-patches.dng"), None).unwrap();
    let region = Coverage::rectangle(Rect {
        x: 16,
        y: 16,
        width: 32,
        height: 32,
    });
    let base = scan.reference_coverage(region).unwrap();
    let dark = scan
        .reference(Rect {
            x: 80,
            y: 16,
            width: 32,
            height: 32,
        })
        .unwrap();
    let area = scan.area;
    let correction = Arc::new(
        FlatField::from_nodes(
            [area.x, area.y, area.width, area.height],
            [2, 2],
            vec![DVec3::splat(1.1); 4],
        )
        .unwrap(),
    );
    let source = scan.source().clone();
    let calibration = Calibration {
        dmin: base,
        dmax: dark,
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    };
    for kind in [Pick::Dmin, Pick::Dmax] {
        let input = Input {
            source: source.clone(),
            correction: Some(correction.clone()),
            kind,
            region,
            calibration: Some(calibration),
            roll: RollSettings::new_grade(),
            photo: PhotoSettings::default(),
        };
        let Measurement::Reference(value) = input.measure().unwrap() else {
            panic!("expected reference")
        };
        assert!((value - base * 1.1).abs().max_element() < 1e-12);
    }
    // The middle patch is distinct from both calibration endpoints.
    let middle = Coverage::rectangle(Rect {
        x: 144,
        y: 16,
        width: 32,
        height: 32,
    });
    for kind in [Pick::Gamma, Pick::RollWb, Pick::Wb] {
        let roll = RollSettings {
            wb: Balance {
                warmth: 0.1,
                tint: -0.05,
            },
            ..RollSettings::new_grade()
        };
        let photo = PhotoSettings {
            exposure: 0.25,
            ..PhotoSettings::default()
        };
        let input = Input {
            source: source.clone(),
            correction: None,
            kind,
            region: middle,
            calibration: Some(calibration),
            roll,
            photo,
        };
        let mut reference = Scan::from_source(source.clone());
        let expected = match kind {
            Pick::Gamma => reference.roll_neutral_coverage(middle, calibration, roll),
            Pick::RollWb => reference.photo_neutral_coverage(
                middle,
                calibration,
                RollSettings {
                    wb: Balance::default(),
                    ..roll
                },
                PhotoSettings::default(),
            ),
            _ => reference.photo_neutral_coverage(middle, calibration, roll, photo),
        }
        .unwrap();
        let Measurement::Balance(value) = input.measure().unwrap() else {
            panic!("expected balance")
        };
        assert_eq!(value, expected);
    }
}
