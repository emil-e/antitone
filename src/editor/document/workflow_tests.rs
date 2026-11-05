use super::*;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
#[cfg(feature = "ocio-reference")]
mod rebase_exposure;
use crate::{
    application::test_support::with_application,
    editor::{Preparation, document::Document, source, source_loading, tests::fixture},
};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::AtomicUsize, mpsc},
    time::Duration,
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn editor(scheduler: source_loading::SourceLoader) -> Editor {
    let mut editor = fixture();
    editor.document.install(Document::default());
    editor.set_dmin(None);
    editor.set_dmax(None);
    editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
    editor.sync_document();
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
    editor.source_services = Some(source::Services {
        developments: scheduler,
        renderers: Arc::default(),
        device,
        queue,
    });
    editor
}

#[test]
fn new_work_admission_withdraws_queued_feature_acceptance_after_the_picker_finishes() {
    use crate::editor::{Balance, adjustment_layers::Target, calibration, picking, white_balance};
    use std::{cell::Cell, rc::Rc};
    #[derive(Clone, Copy)]
    enum Admission {
        Publish,
        Import,
        OtherPicker,
    }
    for layer_pick in [false, true] {
        for admission in [
            Admission::Publish,
            Admission::Import,
            Admission::OtherPicker,
        ] {
            let import = matches!(admission, Admission::Import);
            let publish = matches!(admission, Admission::Publish);
            let replacement = if layer_pick {
                crate::editor::Pick::Gamma
            } else {
                crate::editor::Pick::Wb
            };
            let directory = tempfile::tempdir().unwrap();
            let added = directory.path().join("added.dng");
            std::fs::copy(path("patches.dng"), &added).unwrap();
            let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
            editor.sync_document();
            let source = editor.document.visit.current_source.unwrap();
            let Target::Layer(layer) = editor.adjustment_layer_target() else {
                panic!("fixture layer");
            };
            let target = (source, Target::Layer(layer));
            let undo = editor
                .history_label(crate::editor::history::Direction::Undo)
                .map(str::to_owned);
            let before = if layer_pick {
                editor.document.visit.adjustment_layer_settings.wb
            } else {
                editor.balance()
            };
            let value = Balance {
                warmth: 0.2,
                tint: -0.1,
            };
            assert_ne!(value, before);
            let published = Rc::new(Cell::new(false));
            with_application(editor, |app| {
                let reading = app.enter(|editor, cx| {
                    let display = if layer_pick {
                        white_balance::LAYER
                    } else {
                        calibration::GAMMA
                    };
                    let reading = picking::pick(editor, display, move |_, _| Ok(value), cx.clone());
                    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
                    editor.set_test_pick_region(Some(crate::coverage::Coverage::rectangle(area)));
                    editor.hosted_action(crate::editor::tool::Action::Primary);
                    reading
                });
                app.run_until(Duration::from_secs(5), |editor| {
                    !editor.test_picker_active()
                });
                let resumed = published.clone();
                app.enter(|editor, cx| {
                    // The real picker has completed; deliberately defer its feature's consumption.
                    let action = cx
                        .spawn(move |cx| async move {
                            let Some(value) = reading.await else {
                                return;
                            };
                            resumed.set(true);
                            cx.update(|editor| {
                                if layer_pick {
                                    white_balance::set_layer(editor, value, target).unwrap();
                                } else {
                                    calibration::set_gamma_balance(editor, value).unwrap();
                                }
                            });
                        })
                        .unwrap();
                    if layer_pick {
                        editor.document.visit.white_balance_action = Some(action);
                    } else {
                        editor.document.visit.calibration_action = Some(action);
                    }
                    if import {
                        editor.add_raws(vec![added], cx);
                        assert!(editor.document_pending.is_some());
                    } else if !publish {
                        editor.begin_pick(replacement, cx);
                    }
                });
                if import {
                    app.run_until(Duration::from_secs(15), |editor| {
                        !editor.interaction_blocked() && editor.document_pending.is_none()
                    });
                } else if publish {
                    app.run_until(Duration::from_secs(5), |_| published.get());
                } else {
                    app.pump();
                }
                assert_eq!(published.get(), publish);
                app.read(|editor| {
                    assert_eq!(editor.document.visit.current_source, Some(source));
                    let actual = if layer_pick {
                        editor.document.visit.adjustment_layer_settings.wb
                    } else {
                        editor.balance()
                    };
                    assert_eq!(actual, if publish { value } else { before });
                    if !import && !publish {
                        assert_eq!(editor.test_pick(), Some(replacement.display()));
                        assert!(!editor.interaction_blocked());
                    }
                    assert_eq!(
                        editor.document.project.order.len(),
                        if import { 2 } else { 1 }
                    );
                    assert_eq!(
                        editor.history_label(crate::editor::history::Direction::Undo),
                        if import {
                            Some("Import photos")
                        } else if publish {
                            Some("Calibration pick")
                        } else {
                            undo.as_deref()
                        }
                    );
                });
            });
        }
    }
}

#[test]
fn accepted_open_seeds_a_fresh_scope_cache_from_the_prepared_source() {
    let scheduler = source_loading::SourceLoader::new();
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("opened.antitone");
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, cx| e.add_raws(vec![path("patches.dng")], cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        let (snapshot, previous) =
            app.read(|e| (e.document.capture(), Arc::downgrade(&e.document.cache)));
        snapshot.save(&project, crate::project::Save::New).unwrap();
        app.enter(|e, cx| e.start_document(Preparation::Open(project), cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(previous.upgrade().is_none());
            assert!(!previous.ptr_eq(&Arc::downgrade(&e.document.cache)));
            let ready = e
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .expect("opened source ready");
            let key = crate::source::SourceKey::for_load(
                &ready.path,
                ready.file.clone(),
                &ready.record.development,
            );
            assert!(Arc::ptr_eq(
                &e.document.cache.get(&key).unwrap(),
                &ready.developed
            ));
            assert!(Arc::ptr_eq(
                &e.document.cache.get_proxy(&key).unwrap(),
                &ready.thumbnail.as_ref().unwrap().1,
            ));
        });
        drop(snapshot);
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn import_history_correction_and_new_run_without_backend_or_painting() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.add_raws(vec![path("patches.dng")], access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        let (accepted, selected, visit) = app.read(|e| {
            assert!(e.document.visit.canvas.ready.is_some(), "{}", e.status);
            assert_eq!(e.document.state.project.order.len(), 1);
            assert!(e.document.dirty());
            (
                e.document.state.project.clone(),
                e.document.visit.current_source,
                e.document.visit.view,
            )
        });
        app.enter(|e, access| {
            e.start_document(
                Preparation::Correction(e.document.capture(), selected),
                access,
            );
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.state.project, accepted);
            assert_eq!(e.document.visit.view, visit);
        });
        app.enter(|e, access| {
            e.restore_history(crate::editor::history::Direction::Undo, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(
                e.document.state.project.order.is_empty(),
                "status={} undo={:?}",
                e.status,
                e.history_label(crate::editor::history::Direction::Undo)
            );
            assert!(e.document.visit.canvas.ready.is_none());
        });
        app.enter(|e, access| {
            e.restore_history(crate::editor::history::Direction::Redo, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.state.project, accepted);
            assert!(e.document.visit.canvas.ready.is_some());
        });
        let cache = app.read(|e| Arc::downgrade(&e.document.cache));
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.state.project, crate::project::Project::default());
            assert!(!e.document.dirty());
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(
                e.history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
        });
        assert!(cache.upgrade().is_none());
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn input_stage_switch_redevelops_every_source_for_the_target_stage() {
    use crate::processing::InputSettings;

    let directory = tempfile::tempdir().unwrap();
    let original =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"))
            .unwrap();
    let red = directory.path().join("red-neutral.dng");
    std::fs::write(&red, super::super::tests::dng_with_red_as_shot(&original)).unwrap();
    let red_as_shot = crate::editor::document::read_source(&red, None)
        .unwrap()
        .development
        .decode
        .fixed_wb;
    assert_ne!(red_as_shot, crate::project::UNITY_WB);

    let assert_stage_development = |e: &Editor, digital: bool| {
        let wb = if digital {
            None
        } else {
            Some(crate::project::UNITY_WB)
        };
        for source in &e.document.state.project.sources {
            let expected = crate::editor::document::read_source(
                &e.document.source_path(source.id).unwrap(),
                wb,
            )
            .unwrap();
            assert_eq!(
                source.development,
                expected.development,
                "source {} did not use the {} stage WB",
                source.path.display(),
                if digital { "Digital" } else { "Negative" }
            );
        }
    };

    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, cx| e.add_raws(vec![path("patches.dng"), red.clone()], cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(e.document.input.is_negative(), "{}", e.status);
            assert_eq!(e.document.state.project.sources.len(), 2);
            assert_stage_development(e, false);
        });
        app.enter(|e, cx| e.request_input(InputSettings::Digital, cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.input, InputSettings::Digital, "{}", e.status);
            assert_stage_development(e, true);
        });
        app.enter(|e, cx| e.request_input(e.negative_input(), cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(e.document.input.is_negative(), "{}", e.status);
            assert_stage_development(e, false);
            assert_eq!(
                e.document.state.project.scan_basis(),
                Some(&crate::project::ScanBasis::from_development(
                    &e.document.state.project.sources[0].development
                ))
            );
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.input, InputSettings::Digital, "{}", e.status);
            assert_stage_development(e, true);
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(e.document.input.is_negative(), "{}", e.status);
            assert_stage_development(e, false);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn input_switch_clears_live_correction_atomically_and_history_restores_activation() {
    use crate::{flat_field::FlatField, processing::InputSettings, project::FlatFieldSettings};

    let scheduler = source_loading::SourceLoader::new();

    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, cx| e.add_raws(vec![path("patches.dng")], cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.enter(|e, cx| {
            assert!(e.document.visit.canvas.ready.is_some(), "{}", e.status);
            e.set_dmin(Some(glam::DVec3::splat(0.6)));
            e.set_dmax(Some(glam::DVec3::splat(0.005)));
            e.document.source = false;
            e.sync_document();
            let mut candidate = e.document.capture();
            let source = candidate
                .project
                .source(e.document.visit.current_source.unwrap())
                .unwrap();
            let map = Arc::new(
                FlatField::from_nodes(
                    source.development.active_area,
                    [2, 2],
                    vec![glam::DVec3::splat(2.); 4],
                )
                .unwrap(),
            );
            let field = FlatFieldSettings::new(source, map);
            candidate.project.set_flat_field(Some(field));
            e.start_document(
                Preparation::Correction(candidate, e.document.visit.current_source),
                cx,
            );
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        let (negative, scan, expected_pixels) = app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            let services = e.source_services.as_ref().unwrap();
            let ready = e.document.visit.canvas.ready.as_ref().unwrap();
            let (uncorrected, _) = crate::editor::prepare_source(
                &services.renderers,
                &services.device,
                &services.queue,
                ready.path.clone(),
                Some(ready.record.development.decode.fixed_wb),
                Some(&ready.record),
                None,
            )
            .unwrap();
            (
                e.document.input,
                e.document.state.project.scan.clone(),
                digital_proxy_pixels(e, &uncorrected, None),
            )
        });
        let mut cleared_scan = scan.clone().unwrap();
        cleared_scan.flat_field = None;
        // The fixture really exercises an active GPU correction, not just saved settings.
        let corrected_pixels = app.read(|e| {
            digital_proxy_pixels(
                e,
                e.document.visit.canvas.ready.as_ref().unwrap(),
                e.document
                    .correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
            )
        });
        assert!(
            corrected_pixels
                .iter()
                .zip(&expected_pixels)
                .any(|(a, b)| (a[0] - b[0]).abs() > 0.01)
        );
        app.enter(|e, cx| {
            e.request_input(InputSettings::Digital, cx);
            assert_eq!(e.document.input, negative);
            assert_eq!(e.document.input_switch.take(), Some(InputSettings::Digital));
            e.apply_input(InputSettings::Digital, cx);
            assert!(e.interaction_blocked() && e.document.correction.is_some());
            assert_eq!(
                e.document.input, negative,
                "old interpretation stays accepted until preparation completes"
            );
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.input, InputSettings::Digital, "{}", e.status);
            assert!(e.document.correction.is_none() && !e.document.source);
            assert!(
                e.source_context()
                    .unwrap()
                    .scan()
                    .unwrap()
                    .flat_field()
                    .is_none()
            );
            assert_eq!(e.document.state.project.scan, Some(cleared_scan.clone()));
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Input stage")
            );
            let actual = digital_proxy_pixels(
                e,
                e.document.visit.canvas.ready.as_ref().unwrap(),
                e.document
                    .correction
                    .as_ref()
                    .map(|correction| correction.gpu.clone()),
            );
            assert_eq!(actual.len(), expected_pixels.len());
            for (a, b) in actual.iter().zip(&expected_pixels) {
                for channel in 0..3 {
                    assert!((a[channel] - b[channel]).abs() < 3e-5, "{a:?} vs {b:?}");
                }
            }
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.input, negative, "{}", e.status);
            assert!(e.document.correction.is_some() && !e.document.source);
            assert_eq!(e.document.state.project.scan, scan);
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.enter(|e, cx| {
            assert_eq!(e.document.input, InputSettings::Digital, "{}", e.status);
            assert!(e.document.correction.is_none());
            e.request_input(e.negative_input(), cx);
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(
                e.document.input.is_negative()
                    && e.document.source
                    && e.document.correction.is_none(),
                "{}",
                e.status
            );
            assert_eq!(e.document.state.project.scan, Some(cleared_scan));
        });
    });
    scheduler.close_and_wait().unwrap();

    fn digital_proxy_pixels(
        e: &Editor,
        ready: &crate::editor::Ready,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
    ) -> Vec<[f32; 4]> {
        use crate::gpu::view::RenderedView;
        use crate::gpu::view::ViewKernels;
        use crate::gpu::view::request::SourceRequest;
        let services = e.source_services.as_ref().unwrap();
        let (device, queue) = (&services.device, &services.queue);
        let composition =
            crate::geometry::Composition::new(ready.area, Default::default(), Default::default())
                .unwrap();
        let mut frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
        frame.output = ready.proxy.size();
        let matrix = ready.proxy.matrix();
        let spec = SourceRequest::new(
            frame,
            InputSettings::Digital,
            matrix,
            field.as_ref().map(|field| field.field().clone()),
        )
        .unwrap();
        let source = ready
            .proxy
            .view_source(
                queue,
                &crate::gpu::submission::SubmissionGate::default(),
                pollster::block_on(ViewKernels::new(device)).unwrap(),
                &spec.processing_fixture().with_flat_field(field),
                spec,
            )
            .unwrap();
        let parameters = crate::gpu::test_inputs::AuthoredPhotograph::from(Arc::new(
            crate::gpu::test_inputs::AuthoredPhotograph::new(
                &device.limits(),
                InputSettings::Digital,
                Default::default(),
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                matrix,
                ready.area,
            )
            .unwrap(),
        ));
        let view = RenderedView::new(device, source, &parameters, e.document.target, None).unwrap();
        view.ensure_submitted(device, queue, &e.submissions, &egui::Context::default());
        crate::viewer::readback::image_pixels(device, queue, &view.image)
    }
}

#[test]
fn open_prepares_first_source_before_promotion_and_failed_open_preserves_owner_history_and_pixels()
{
    let scheduler = source_loading::SourceLoader::new();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("candidate.antitone");
    let mut candidate = Document::default();
    candidate.import(&[path("patches.dng")]).unwrap();
    candidate
        .save(&destination, crate::project::Save::New)
        .unwrap();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, access| {
            e.start_document(Preparation::Open(destination.clone()), access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        let (snapshot, visit, native, store) = app.update(|e| {
            let before = e.history_snapshot();
            e.document.roll.exposure = 0.25;
            e.record_edit(before, "Exposure", None);
            (
                e.document.state.project.clone(),
                e.document.visit.view,
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .clone(),
                e.document.cache.clone(),
            )
        });
        app.enter(|e, access| {
            e.start_document(
                Preparation::Open(directory.path().join("missing.antitone")),
                access,
            );
        });
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert_eq!(e.document.state.project, snapshot);
            assert_eq!(e.document.visit.view, visit);
            assert!(Arc::ptr_eq(
                &e.document.visit.canvas.ready.as_ref().unwrap().developed,
                &native
            ));
            assert!(Arc::ptr_eq(&store, &e.document.cache));
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Exposure")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn queued_supersession_retires_old_continuation_and_stale_progress_cannot_touch_new_work() {
    let scheduler = source_loading::SourceLoader::new();
    let e = editor(scheduler.clone());
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        app.enter(|e, access| {
            e.add_raws(vec![path("patches.dng")], access);
        });
        app.pump();
        let old = app.read(|e| e.document_pending.unwrap());
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        let current = app.read(|e| e.document_pending.unwrap());
        app.pump();
        app.update(|e| {
            e.document_prepared(old, Err("stale".into()));
        });
        app.read(|e| {
            assert_eq!(e.document_pending, Some(current));
            assert!(e.activities.view().unwrap().measured.is_none());
            assert!(e.interaction_blocked());
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(e.document.state.project.sources.is_empty());
            assert!(e.document.visit.canvas.ready.is_none());
            assert_eq!(e.status, "Project ready");
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn pending_native_candidate_keeps_accepted_state_and_admission_after_logical_retirement() {
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            observed.fetch_add(1, Ordering::Relaxed);
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        Ok(())
    });
    let e = editor(scheduler.clone());
    with_application(e, |app| {
        let accepted = app.read(|e| e.document.state.project.clone());
        app.enter(|e, access| {
            e.add_raws(vec![path("patches.dng")], access);
        });
        app.run_until(Duration::from_secs(10), |_| scheduler.waiter_count() > 0);
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        app.read(|e| {
            assert_eq!(e.document.state.project, accepted);
            assert!(e.document.visit.canvas.ready.is_none() && e.interaction_blocked());
        });
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.pump();
        assert_eq!(scheduler.waiter_count(), 0);
        assert!(app.read(|e| e.interaction_blocked()));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        assert_eq!(count.load(Ordering::Relaxed), 1);
        app.read(|e| {
            assert!(
                e.document.state.project.sources.is_empty()
                    && e.document.visit.canvas.ready.is_none()
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}
