use super::*;
use crate::adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerSnapshot};
use crate::application::Application;
use crate::source_loader;
use crate::{raw::Rect, source_store::SourceStore};
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

// Gated to macOS: LibRaw holds an open RAW with write sharing denied on Windows, so the test cannot
// change the source beneath the captured measurement.
#[cfg(target_os = "macos")]
#[test]
fn measurement_uses_the_captured_source_and_updates_history_without_receiving_or_painting() {
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
        let scheduler = source_loader::SourceLoader::new();
        editor.source_services = Some(crate::editor::source::Services {
            developments: scheduler.clone(),
            renderers,
            device: device.clone(),
            queue: queue.clone(),
        });
        editor.current_source = Some(
            editor
                .document
                .register_with_development(ready.record.clone(), ready.development().clone())
                .unwrap(),
        );
        editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
        editor.set_dmin(Some(expected * 1.05));
        editor.set_dmax(Some(dark));
        editor.document.visit.ready = Some(ready);
        editor.sync_document();
        editor.document.cache().set_budget(0);
        drop(scan);
        let hold = crate::background::testing::pause();
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
                assert_eq!(editor.dmin(), Some(expected), "{}", editor.status);
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
        let (release_next, resume_next) = mpsc::channel();
        let resume_next = Mutex::new(resume_next);
        let scheduler = source_loader::SourceLoader::new_observed(move |_, phase, _| {
            if matches!(phase, source_loader::NativePhase::UnpackBegin) {
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
                    editor.load(fixture("patches.dng"), cx);
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
fn cancelling_a_running_measurement_retains_its_source_until_the_computation_returns() {
    let scan = Scan::open(&fixture("patches.dng"), None).unwrap();
    let source = scan.source().clone();
    let lease = Arc::downgrade(&source);
    let input = Input {
        source,
        correction: None,
        kind: Pick::Dmin,
        region: Coverage::rectangle(scan.area),
        input: crate::processing::InputSettings::default(),
        roll: RollSettings::default(),
        upstream: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
        layer: None,
    };
    drop(scan);
    let state = std::cell::RefCell::new(Some(()));
    let (mut application, wakes) = application();
    let access = application.access(&state);
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (finished, finishes) = mpsc::channel();
    let task = access
        .spawn(move |_| async move {
            crate::background::run(move || {
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
    let sources = source_loader::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loader::NativePhase::UnpackBegin) {
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
            source_loader::Request {
                path: file.path().to_owned(),
                wb: Some([1.; 4]),
                expected: None,
                key: None,
            },
            source_loader::Priority::Foreground,
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
        let input = Input {
            source,
            correction: None,
            kind: Pick::Dmin,
            region: Coverage::rectangle(region),
            input: crate::processing::InputSettings::default(),
            roll: RollSettings::default(),
            upstream: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            layer: None,
        };
        crate::background::run(move || input.measure().map_err(|error| error.to_string()))
            .await
            .map_err(|_| "worker panicked".to_owned())?
    });
    let unrelated = executor.spawn(async move { crate::background::run(|| 42).await });
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
            input: calibration.into(),
            roll: RollSettings::default(),
            upstream: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            layer: None,
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
            ..RollSettings::default()
        };
        let photo = AdjustmentLayerSettings {
            exposure: 0.25,
            ..AdjustmentLayerSettings::default()
        };
        let input = Input {
            source: source.clone(),
            correction: None,
            kind,
            region: middle,
            input: calibration.into(),
            roll,
            upstream: AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            layer: (kind == Pick::Wb).then(|| AdjustmentLayer::global(photo)),
        };
        let mut reference = Scan::from_source(source.clone());
        let expected = match kind {
            Pick::Gamma => reference.roll_neutral_coverage(middle, &input.input),
            Pick::RollWb => reference.white_balance_coverage(
                middle,
                &input.input,
                RollSettings {
                    wb: Balance::default(),
                    ..roll
                },
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                None,
            ),
            _ => reference.white_balance_coverage(
                middle,
                &input.input,
                roll,
                AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                Some(AdjustmentLayer::global(photo)),
            ),
        }
        .unwrap();
        let Measurement::Balance(value) = input.measure().unwrap() else {
            panic!("expected balance")
        };
        assert_eq!(value, expected);
    }
}

#[test]
fn layer_white_balance_pick_captures_only_preceding_layers_and_fits_a_replacement() {
    use crate::{adjustment_layers::AdjustmentLayer, project::AdjustmentLayers};
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.input = crate::processing::InputSettings::Digital;
    editor.roll = RollSettings::default();
    let source = editor.current_source.unwrap();
    let upstream = Balance {
        warmth: 0.2,
        tint: -0.1,
    };
    let mut layers = AdjustmentLayers::with_global(AdjustmentLayerSettings {
        wb: upstream,
        exposure: 1.,
        ..AdjustmentLayerSettings::default()
    });
    let selected = layers
        .add(
            "Selected".into(),
            AdjustmentLayer::global(AdjustmentLayerSettings {
                wb: Balance {
                    warmth: 0.4,
                    tint: 0.3,
                },
                exposure: 0.5,
                ..AdjustmentLayerSettings::default()
            }),
        )
        .unwrap();
    layers
        .add(
            "Downstream".into(),
            AdjustmentLayer::global(AdjustmentLayerSettings {
                wb: Balance {
                    warmth: -0.7,
                    tint: 0.6,
                },
                ..AdjustmentLayerSettings::default()
            }),
        )
        .unwrap();
    editor
        .document
        .project
        .source_mut(source)
        .unwrap()
        .adjustment_layers = Some(layers);
    editor.refresh_adjustment_layer_selection();
    editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(selected));
    let ready = editor.document.visit.ready.as_ref().unwrap();
    let region = Coverage::rectangle(Rect {
        x: ready.area.x + ready.area.width / 3,
        y: ready.area.y + ready.area.height / 3,
        width: ready.area.width / 3,
        height: ready.area.height / 3,
    });
    let captured = editor.measurement_input(Pick::Wb, region).unwrap();
    let mut scan = Scan::from_source(ready.developed.clone());
    let baseline = scan
        .white_balance_coverage(
            region,
            &editor.input,
            editor.roll,
            AdjustmentLayerSnapshot::new(vec![]).unwrap(),
            None,
        )
        .unwrap();
    // Bradford gains add in log coordinates; the selected replacement cancels the upstream WB.
    let expected = Balance {
        warmth: baseline.warmth - upstream.warmth,
        tint: baseline.tint - upstream.tint,
    };
    editor
        .edit_adjustment_layers(|layers| {
            layers.layers[0].adjustment.photo.wb = Balance::default();
            Ok(())
        })
        .unwrap();
    let Measurement::Balance(actual) = captured.measure().unwrap() else {
        panic!("expected a neutral correction");
    };
    assert!((actual.warmth - expected.warmth).abs() < 1e-10);
    assert!((actual.tint - expected.tint).abs() < 1e-10);
}

#[test]
fn masked_and_partial_layer_wb_picks_neutralize_the_captured_patch_before_own_creative_controls() {
    use crate::{
        adjustment_layers::AdjustmentLayerCoverage, geometry::SourcePoint,
        project::AdjustmentLayers,
    };
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    editor.input = InputSettings::Digital;
    editor.roll = RollSettings::default();
    let source = editor.current_source.unwrap();
    let ready = editor.document.visit.ready.as_ref().unwrap();
    let developed = ready.developed.clone();
    let patch = Rect {
        x: ready.area.x + ready.area.width / 3,
        y: ready.area.y + ready.area.height / 3,
        width: ready.area.width / 3,
        height: ready.area.height / 3,
    };
    let center = [
        f64::from(patch.x) + f64::from(patch.width) / 2.,
        f64::from(patch.y) + f64::from(patch.height) / 2.,
    ];
    for (mask, invert, strength) in [
        (AdjustmentLayerCoverage::Global {}, false, 0.45),
        (
            AdjustmentLayerCoverage::Linear {
                start: SourcePoint([f64::from(patch.x), center[1]]),
                end: SourcePoint([f64::from(patch.x + patch.width), center[1]]),
            },
            true,
            0.7,
        ),
        (
            AdjustmentLayerCoverage::Radial {
                center: SourcePoint(center),
                radii: [f64::from(patch.width), f64::from(patch.height)],
                angle: 0.3,
                feather: 1.,
            },
            false,
            0.65,
        ),
    ] {
        let selected = AdjustmentLayer {
            mask,
            invert,
            strength,
            enabled: true,
            photo: AdjustmentLayerSettings {
                exposure: 0.5,
                wb: Balance {
                    warmth: 0.8,
                    tint: -0.7,
                },
                contrast: 1.5,
                saturation: 1.8,
                ..Default::default()
            },
        };
        let mut layers = AdjustmentLayers::default();
        let id = layers.add("Selected".into(), selected).unwrap();
        layers
            .add(
                "Finishing".into(),
                AdjustmentLayer::global(AdjustmentLayerSettings {
                    wb: Balance {
                        warmth: -0.5,
                        tint: 0.4,
                    },
                    ..Default::default()
                }),
            )
            .unwrap();
        editor
            .document
            .project
            .source_mut(source)
            .unwrap()
            .adjustment_layers = Some(layers);
        editor.refresh_adjustment_layer_selection();
        editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(id));
        let Measurement::Balance(balance) = editor
            .measurement_input(Pick::Wb, Coverage::rectangle(patch))
            .unwrap()
            .measure()
            .unwrap()
        else {
            panic!("expected WB");
        };
        let mut measured = selected;
        measured.photo.wb = balance;
        measured.photo.contrast = 1.;
        measured.photo.saturation = 1.;
        let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
            InputSettings::Digital,
            editor.roll,
            AdjustmentLayerSnapshot::new(vec![measured]).unwrap(),
            developed.camera_to_ap1(),
        )
        .unwrap();
        let scan = Scan::from_source(developed.clone());
        let mut rgb = DVec3::ZERO;
        for y in patch.y..patch.y + patch.height {
            for x in patch.x..patch.x + patch.width {
                rgb += pipeline
                    .apply(scan.pixel(x, y).unwrap(), [x, y])
                    .unwrap()
                    .rgb;
            }
        }
        rgb /= f64::from(patch.width * patch.height);
        assert!(
            (rgb - DVec3::splat(rgb.y)).abs().max_element() < 1e-8,
            "{mask:?}, {balance:?}: {rgb:?}"
        );
    }
}

#[test]
fn a_completed_wb_pick_cannot_write_to_a_different_selected_layer() {
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let source = editor.current_source.unwrap();
    let first = editor
        .document
        .project
        .source(source)
        .unwrap()
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers[0]
        .id;
    let second = editor
        .document
        .project
        .source_mut(source)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .add(
            "Other".into(),
            AdjustmentLayer::global(AdjustmentLayerSettings {
                exposure: 0.5,
                ..Default::default()
            }),
        )
        .unwrap();
    editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(first));
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let task = cx
                .spawn(|_| async { std::future::pending::<()>().await })
                .unwrap();
            let view = editor.document.visit.view;
            editor.document.visit.measurement = Some(Pending {
                kind: Pick::Wb,
                target: Some((source, first)),
                _task: task,
            });
            editor.pick = Some(Pick::Wb);
            editor.busy = true;
            editor.select_adjustment_layer(crate::editor::adjustment_layers::Target::Layer(second));
            let before = editor.document.project.clone();
            let settings = editor.adjustment_layer_settings;
            editor.measurement_result(
                view,
                Pick::Wb,
                Ok(Measurement::Balance(Balance {
                    warmth: 0.3,
                    tint: 0.2,
                })),
                cx,
            );
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.adjustment_layer_settings, settings);
            assert!(editor.document.visit.measurement.is_none());
            assert!(!editor.busy);
            assert!(editor.pick.is_none());
        });
    });
}
