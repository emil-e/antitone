use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Preparation, development,
        document::Document,
        source,
        tests::{fixture, hold_blocking_work},
    },
};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicUsize, mpsc},
    time::Duration,
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn editor(scheduler: development::Scheduler) -> Editor {
    let mut editor = fixture();
    editor.document.install(Document::default());
    editor.set_dmin(None);
    editor.set_dmax(None);
    editor.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
    editor.sync_document();
    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
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
        jobs: editor.workflow_jobs.clone(),
        renderers: Arc::default(),
        device,
        queue,
    });
    editor
}

#[test]
fn import_history_correction_and_new_run_without_backend_or_painting() {
    let scheduler = development::Scheduler::new();
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
        let cache = app.read(|e| e.document.cache().clone());
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
fn input_switch_rebuilds_live_correction_atomically_and_history_restores_activation() {
    use crate::{
        flat_field::FlatField, gpu::ProcessingParameters, processing::InputSettings,
        project::FlatFieldSettings,
    };
    let scheduler = development::Scheduler::new();
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
            assert_eq!(e.document.project.scan, scan);
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
                e.input.is_negative() && e.source && e.correction.is_some(),
                "{}",
                e.status
            );
            assert_eq!(e.document.project.scan, scan);
        });
    });
    scheduler.close_and_wait().unwrap();

    fn digital_proxy_pixels(e: &Editor, ready: &crate::editor::Ready) -> Vec<[f32; 4]> {
        use crate::{
            gpu::view::{RenderedView, ViewKernels, ViewSourceSpec},
            local_adjustments::PhotoGradeSnapshot,
        };
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
        let parameters = ProcessingParameters::interpret(
            InputSettings::Digital,
            Default::default(),
            Default::default(),
            matrix,
        )
        .unwrap()
        .into();
        let view = RenderedView::new(
            device,
            source,
            InputSettings::Digital,
            Default::default(),
            PhotoGradeSnapshot::new(Default::default(), vec![]).unwrap(),
            &parameters,
            e.target,
            None,
        )
        .unwrap();
        view.ensure_submitted(device, queue, &e.submissions, &egui::Context::default());
        crate::viewer::readback::image_pixels(device, queue, &view.image)
    }
}

#[test]
fn open_prepares_first_source_before_promotion_and_failed_open_preserves_owner_history_and_pixels()
{
    let scheduler = development::Scheduler::new();
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
                e.document.cache().clone(),
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
    let scheduler = development::Scheduler::new();
    let e = editor(scheduler.clone());
    let hold = hold_blocking_work(&e.workflow_jobs);
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
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin) {
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
