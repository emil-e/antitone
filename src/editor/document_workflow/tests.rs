use super::*;
use std::sync::atomic::Ordering;
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
fn accepted_open_seeds_a_fresh_scope_cache_from_the_prepared_source() {
    let scheduler = source_loading::SourceLoader::new();
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("opened.antitone");
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, cx| e.add_raws(vec![path("patches.dng")], cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        let (snapshot, previous) = app.read(|e| (e.document.capture(), e.document.cache.clone()));
        snapshot.save(&project, crate::project::Save::New).unwrap();
        app.enter(|e, cx| e.start_document(Preparation::Open(project), cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(previous.is_closed());
            assert!(!Arc::ptr_eq(&previous, &e.document.cache));
            let ready = e
                .document
                .visit
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
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        let (accepted, selected, visit) = app.read(|e| {
            assert!(e.document.visit.ready.is_some(), "{}", e.status);
            assert_eq!(e.document.project.order.len(), 1);
            assert!(e.document.dirty());
            (
                e.document.project.clone(),
                e.current_source,
                e.document.visit.view,
            )
        });
        app.enter(|e, access| {
            e.start_document(
                Preparation::Correction(e.document.capture(), selected),
                access,
            );
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert_eq!(e.document.visit.view, visit);
        });
        app.enter(|e, access| {
            e.restore_history(crate::editor::history::Direction::Undo, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(
                e.document.project.order.is_empty(),
                "status={} undo={:?}",
                e.status,
                e.history_label(crate::editor::history::Direction::Undo)
            );
            assert!(e.document.visit.ready.is_none());
        });
        app.enter(|e, access| {
            e.restore_history(crate::editor::history::Direction::Redo, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert!(e.document.visit.ready.is_some());
        });
        let cache = app.read(|e| e.document.cache.clone());
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, crate::project::Project::default());
            assert!(!e.document.dirty());
            assert!(e.document.visit.ready.is_none());
            assert!(
                e.history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
        });
        assert!(cache.is_closed());
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
        for source in &e.document.project.sources {
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
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(e.input.is_negative(), "{}", e.status);
            assert_eq!(e.document.project.sources.len(), 2);
            assert_stage_development(e, false);
        });
        app.enter(|e, cx| e.request_input(InputSettings::Digital, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.input, InputSettings::Digital, "{}", e.status);
            assert_stage_development(e, true);
        });
        app.enter(|e, cx| e.request_input(e.negative_input(), cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(e.input.is_negative(), "{}", e.status);
            assert_stage_development(e, false);
            assert_eq!(
                e.document.project.scan_basis(),
                Some(&crate::project::ScanBasis::from_development(
                    &e.document.project.sources[0].development
                ))
            );
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.input, InputSettings::Digital, "{}", e.status);
            assert_stage_development(e, true);
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(e.input.is_negative(), "{}", e.status);
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
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.enter(|e, cx| {
            assert!(e.document.visit.ready.is_some(), "{}", e.status);
            e.set_dmin(Some(glam::DVec3::splat(0.6)));
            e.set_dmax(Some(glam::DVec3::splat(0.005)));
            e.source = false;
            e.sync_document();
            let mut candidate = e.document.capture();
            let source = candidate.project.source(e.current_source.unwrap()).unwrap();
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
            e.start_document(Preparation::Correction(candidate, e.current_source), cx);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        let (negative, scan, expected_pixels) = app.read(|e| {
            assert!(e.correction.is_some(), "{}", e.status);
            let services = e.source_services.as_ref().unwrap();
            let ready = e.document.visit.ready.as_ref().unwrap();
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
                e.input,
                e.document.project.scan.clone(),
                digital_proxy_pixels(e, &uncorrected),
            )
        });
        let mut cleared_scan = scan.clone().unwrap();
        cleared_scan.flat_field = None;
        // The fixture really exercises an active GPU correction, not just saved settings.
        let corrected_pixels =
            app.read(|e| digital_proxy_pixels(e, e.document.visit.ready.as_ref().unwrap()));
        assert!(
            corrected_pixels
                .iter()
                .zip(&expected_pixels)
                .any(|(a, b)| (a[0] - b[0]).abs() > 0.01)
        );
        app.enter(|e, cx| {
            e.request_input(InputSettings::Digital, cx);
            assert_eq!(e.input, negative);
            assert_eq!(e.input_switch.take(), Some(InputSettings::Digital));
            e.apply_input(InputSettings::Digital, cx);
            assert!(e.busy && e.correction.is_some());
            assert_eq!(
                e.input, negative,
                "old interpretation stays accepted until preparation completes"
            );
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.input, InputSettings::Digital, "{}", e.status);
            assert!(e.correction.is_none() && !e.source);
            assert!(
                e.source_context()
                    .unwrap()
                    .scan()
                    .unwrap()
                    .flat_field()
                    .is_none()
            );
            assert_eq!(e.document.project.scan, Some(cleared_scan.clone()));
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Input stage")
            );
            let actual = digital_proxy_pixels(e, e.document.visit.ready.as_ref().unwrap());
            assert_eq!(actual.len(), expected_pixels.len());
            for (a, b) in actual.iter().zip(&expected_pixels) {
                for channel in 0..3 {
                    assert!((a[channel] - b[channel]).abs() < 3e-5, "{a:?} vs {b:?}");
                }
            }
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.input, negative, "{}", e.status);
            assert!(e.correction.is_some() && !e.source);
            assert_eq!(e.document.project.scan, scan);
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.enter(|e, cx| {
            assert_eq!(e.input, InputSettings::Digital, "{}", e.status);
            assert!(e.correction.is_none());
            e.request_input(e.negative_input(), cx);
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(
                e.input.is_negative() && e.source && e.correction.is_none(),
                "{}",
                e.status
            );
            assert_eq!(e.document.project.scan, Some(cleared_scan));
        });
    });
    scheduler.close_and_wait().unwrap();

    fn digital_proxy_pixels(e: &Editor, ready: &crate::editor::Ready) -> Vec<[f32; 4]> {
        use crate::gpu::view::{RenderedView, ViewKernels, ViewSourceSpec};
        let services = e.source_services.as_ref().unwrap();
        let (device, queue) = (&services.device, &services.queue);
        let composition =
            crate::geometry::Composition::new(ready.area, Default::default(), Default::default())
                .unwrap();
        let mut frame = crate::frame_raster::FrameRaster::full(composition).unwrap();
        frame.output = ready.proxy.size();
        let matrix = ready.proxy.matrix();
        let spec = ViewSourceSpec::new(e.id, frame, InputSettings::Digital, matrix, None).unwrap();
        let source = ready
            .proxy
            .view_source(
                queue,
                pollster::block_on(ViewKernels::new(device)).unwrap(),
                spec,
            )
            .unwrap();
        let parameters = crate::gpu::ProcessingInput::from(Arc::new(
            crate::gpu::GpuPipeline::new(
                device,
                InputSettings::Digital,
                Default::default(),
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).unwrap(),
                matrix,
                ready.area,
            )
            .unwrap(),
        ));
        let view = RenderedView::new(device, source, &parameters, e.target, None).unwrap();
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
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        let (snapshot, visit, native, store) = app.update(|e| {
            let before = e.history_snapshot();
            e.roll.exposure = 0.25;
            e.record_edit(before, "Exposure", None);
            (
                e.document.project.clone(),
                e.document.visit.view,
                e.document.visit.ready.as_ref().unwrap().developed.clone(),
                e.document.cache.clone(),
            )
        });
        app.enter(|e, access| {
            e.start_document(
                Preparation::Open(directory.path().join("missing.antitone")),
                access,
            );
        });
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(e.document.project, snapshot);
            assert_eq!(e.document.visit.view, visit);
            assert!(Arc::ptr_eq(
                &e.document.visit.ready.as_ref().unwrap().developed,
                &native
            ));
            assert!(!store.is_closed());
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
            e.document_progress(old, 5, 10);
            e.document_prepared(old, Err("stale".into()));
        });
        app.read(|e| {
            assert_eq!(e.document_pending, Some(current));
            assert!(e.tasks.view().unwrap().measured.is_none());
            assert!(e.busy);
        });
        drop(hold);
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(e.document.project.sources.is_empty());
            assert!(e.document.visit.ready.is_none());
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
        let accepted = app.read(|e| e.document.project.clone());
        app.enter(|e, access| {
            e.add_raws(vec![path("patches.dng")], access);
        });
        app.run_until(Duration::from_secs(10), |_| scheduler.waiter_count() > 0);
        starts.recv_timeout(Duration::from_secs(10)).unwrap();
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert!(e.document.visit.ready.is_none() && e.busy);
        });
        app.enter(|e, access| {
            e.start_document(Preparation::New, access);
        });
        app.pump();
        assert_eq!(scheduler.waiter_count(), 0);
        assert!(app.read(|e| e.busy));
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        assert_eq!(count.load(Ordering::Relaxed), 1);
        app.read(|e| {
            assert!(e.document.project.sources.is_empty() && e.document.visit.ready.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}
