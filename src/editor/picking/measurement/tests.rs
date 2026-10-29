use super::*;
use crate::editor::{Pick, Scan};
use crate::source as source_loading;
use crate::{raw::Rect, source::SourceCache};
use std::sync::Arc;
use std::{
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

// Gated to macOS: LibRaw holds an open RAW with write sharing denied on Windows, so the test cannot
// change the source beneath the captured measurement.
#[cfg(target_os = "macos")]
#[test]
fn the_awaiting_calibration_feature_uses_its_captured_source_without_painting() {
    pollster::block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("working.dng");
        std::fs::copy(fixture("illuminated-patches.dng"), &path).unwrap();
        let instance = crate::gpu::instance();
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
        let scheduler = source_loading::SourceLoader::new();
        editor.source_services = Some(crate::editor::source::Services {
            developments: scheduler.clone(),
            renderers,
            device: device.clone(),
            queue: queue.clone(),
        });
        editor.document.visit.current_source = Some(
            editor
                .document
                .register_with_development(ready.record.clone(), ready.development().clone())
                .unwrap(),
        );
        editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
        editor.set_dmin(Some(expected * 1.05));
        editor.set_dmax(Some(dark));
        editor.document.visit.canvas.ready = Some(ready);
        editor.sync_document();
        editor.document.cache.set_budget(0);
        drop(scan);
        let hold = crate::background::testing::pause();
        crate::application::test_support::with_application(editor, |app| {
            let tasks = app.access();
            app.enter(|editor, cx| editor.begin_pick(Pick::Dmin, cx));
            app.pump();
            app.update(|editor| {
                editor.set_pick_region(Some(region));
                editor.apply_pick(&tasks);
                assert!(editor.measurement_pending() && editor.busy);
                editor.document.visit.canvas.ready = None;
            });
            std::fs::write(&path, b"replaced after the operation captured its input").unwrap();
            assert!(lease.upgrade().is_some());
            app.pump();
            assert!(app.read(|editor| editor.measurement_pending()));
            assert_eq!(
                app.read(|editor| editor.document.cache.charges().retained),
                0
            );
            drop(hold);
            app.run_until(Duration::from_secs(5), |editor| !editor.busy);
            app.read(|editor| {
                assert_eq!(editor.dmin(), Some(expected), "{}", editor.status);
                assert!(editor.active_pick().is_none() && !editor.measurement_pending());
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
    let source = Arc::downgrade(
        &editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .developed,
    );
    let before = editor.document.state.project.clone();
    crate::application::test_support::with_application(editor, |app| {
        let tasks = app.access();
        app.update(|editor| editor.begin_pick(Pick::Dmin, &tasks));
        app.pump();
        app.update(|editor| {
            editor.set_pick_region(Some(Coverage::rectangle(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
            editor.apply_pick(&tasks);
            assert!(editor.busy && editor.measurement_pending());
        });
        app.enter(|editor, cx| {
            editor.finish_pick(cx);
            editor.document.visit.canvas.ready = None;
        });
        app.pump();
        app.read(|editor| {
            assert!(!editor.measurement_pending() && !editor.busy);
            assert_eq!(editor.document.state.project, before);
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
            .canvas
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
    let expected_error = capture_scan(&editor)
        .unwrap()
        .reference_coverage(region)
        .expect_err("coverage outside the sensor must be rejected")
        .to_string();
    let before = editor.document.state.project.clone();
    let undo = editor
        .history_label(crate::editor::history::Direction::Undo)
        .map(str::to_owned);
    crate::application::test_support::with_application(editor, |app| {
        let tasks = app.access();
        app.enter(|editor, cx| editor.begin_pick(Pick::Dmin, cx));
        app.pump();
        app.update(|editor| {
            editor.set_pick_region(Some(region));
            editor.apply_pick(&tasks);
            assert!(editor.busy && editor.measurement_pending());
        });
        app.run_until(Duration::from_secs(5), |editor| !editor.busy);
        app.read(|editor| {
            assert_eq!(editor.status, expected_error);
            assert!(editor.active_pick().is_none() && !editor.measurement_pending());
            assert!(editor.tasks.view().is_none());
            assert_eq!(editor.document.state.project, before);
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
        let source = Arc::downgrade(
            &editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .developed,
        );
        let (release_next, resume_next) = mpsc::channel();
        let resume_next = Mutex::new(resume_next);
        let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
            if matches!(phase, source_loading::NativePhase::UnpackBegin) {
                resume_next
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
            Ok(())
        });
        editor.source_services.as_mut().unwrap().developments = scheduler.clone();
        let hold = crate::background::testing::pause();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, access| editor.begin_pick(Pick::Dmin, access));
            app.pump();
            app.enter(|editor, access| {
                editor.set_pick_region(Some(Coverage::rectangle(Rect {
                    x: 12,
                    y: 7,
                    width: 32,
                    height: 32,
                })));
                editor.apply_pick(access);
                assert!(editor.measurement_pending());
            });
            app.pump();
            let (status, activity) = app.enter(|editor, cx| {
                if replace {
                    editor.load(fixture("patches.dng"), cx);
                    assert!(editor.document.visit.source.pending());
                } else {
                    editor.finish_pick(cx);
                    editor.document.visit.canvas.ready = None;
                }
                assert!(!editor.measurement_pending());
                assert_eq!(editor.busy, replace);
                (
                    editor.status.clone(),
                    editor.tasks.view().map(|view| view.title),
                )
            });
            drop(hold);
            let deadline = Instant::now() + Duration::from_secs(5);
            while source.upgrade().is_some() {
                app.pump();
                assert!(
                    Instant::now() < deadline,
                    "retired measurement kept its input"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            app.read(|editor| {
                assert_eq!(editor.busy, replace);
                assert_eq!(editor.status, status);
                assert_eq!(editor.tasks.view().map(|view| view.title), activity);
            });
            if replace {
                release_next.send(()).unwrap();
                app.run_until(Duration::from_secs(5), |editor| !editor.busy);
            }
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn cancelling_a_running_measurement_retains_its_source_until_physical_cleanup() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.document.cache.set_budget(0);
    let source = Arc::downgrade(
        &editor
            .document
            .visit
            .canvas
            .ready
            .as_ref()
            .unwrap()
            .developed,
    );
    let before = editor.document.state.project.clone();
    let gate = testing::Gate::install(&mut editor);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| editor.begin_pick(Pick::Dmin, cx));
        app.pump();
        app.enter(|editor, cx| {
            editor.set_pick_region(Some(Coverage::rectangle(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
            )));
            editor.apply_pick(cx);
        });
        let call = gate.next(app);
        app.enter(|editor, cx| {
            editor.finish_pick(cx);
            editor.document.visit.canvas.ready = None;
        });
        app.pump();
        assert!(call.cancelled.is_cancelled());
        assert!(
            source.upgrade().is_some(),
            "the physical worker still owns its captured source"
        );
        app.read(|editor| {
            assert!(!editor.measurement_pending() && !editor.busy);
            assert!(editor.active_pick().is_none());
            assert_eq!(editor.document.state.project, before);
        });
        call.run();
        let deadline = Instant::now() + Duration::from_secs(5);
        while source.upgrade().is_some() {
            app.pump();
            assert!(
                Instant::now() < deadline,
                "retired measurement kept its input"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.read(|editor| {
            assert_eq!(editor.document.state.project, before);
            assert!(!editor.measurement_pending() && !editor.busy);
        });
    });
}

#[test]
fn native_acquisition_wait_does_not_occupy_blocking_job_capacity() {
    let file = crate::raw::fixture::bayer_dng();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let sources = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        Ok(())
    });
    let store = Arc::new(SourceCache::new(0));
    let ticket = sources
        .submit(
            source_loading::Request {
                path: file.path().to_owned(),
                wb: Some([1.; 4]),
                expected: None,
                key: None,
            },
            source_loading::Priority::Foreground,
        )
        .unwrap();
    starts.recv_timeout(Duration::from_secs(5)).unwrap();
    let executor = async_executor::LocalExecutor::new();
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
        let mut image = Scan::from_source(source);
        crate::background::run(move |_| {
            image
                .reference_coverage(Coverage::rectangle(region))
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| "worker panicked".to_owned())?
    });
    let unrelated = executor.spawn(async move { crate::background::run(|_| 42).await });
    let value = futures_lite::future::block_on(executor.run(async {
        assert_eq!(unrelated.await.unwrap(), 42);
        release.send(()).unwrap();
        measurement.await.unwrap()
    }));
    assert!(value.is_finite() && value.min_element() > 0.);
    assert_eq!(store.charges().retained, 0);
}
