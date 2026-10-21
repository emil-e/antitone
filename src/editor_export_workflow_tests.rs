use super::*;
use crate::{
    application::test_support::with_application,
    editor::{
        Crop, DVec3, Editor, OutputFormat, OutputTarget, Overwrite, Thumbnails, document::Document,
        tests::complete_document_operation,
    },
};
use std::{
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

fn hold_first_submission(editor: &Editor) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (started, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let armed = AtomicBool::new(true);
    // The fixture has completed source preparation and disables background thumbnails.
    // With no frames or other demand, the next submission belongs to this export.
    editor.submissions.observe_submissions(Arc::new(move || {
        if armed.swap(false, Ordering::AcqRel) {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(30))
                .unwrap();
        }
    }));
    (starts, release)
}

fn fixture() -> Editor {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.thumbnails = Thumbnails::default();
        editor = complete_document_operation(editor, Duration::from_secs(30), |editor, cx| {
            editor.add_raws(
                vec![
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/raw/patches.dng"),
                ],
                cx,
            )
        });
        editor.dmin = Some(DVec3::splat(0.8));
        editor.dmax = Some(DVec3::splat(0.02));
        let area = editor.document.visit.ready.as_ref().unwrap().area;
        editor.crop = Some(Crop {
            center: [
                f64::from(area.x) + f64::from(area.width) / 2.,
                f64::from(area.y) + f64::from(area.height) / 2.,
            ],
            size: [16, 16],
            angle: 0.,
        });
        editor.sync_document();
        editor
    })
}

fn verify_tiff(path: &std::path::Path) {
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
    assert_eq!(decoder.dimensions().unwrap(), (16, 16));
    let tiff::decoder::DecodingResult::U16(pixels) = decoder.read_image().unwrap() else {
        panic!("expected RGB16")
    };
    assert_eq!(pixels.len(), 16 * 16 * 3);
}

#[test]
fn captured_export_completes_without_paint_and_survives_document_retirement() {
    for replace in [false, true] {
        let editor = fixture();
        let accepted = editor.document.project.clone();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("captured.tif");
        with_application(editor, |app| {
            app.enter(|editor, access| {
                editor.export(
                    path.clone(),
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Refuse,
                    access,
                );

                if replace {
                    editor.document = Document::default().into();
                }
            });
            app.run_until(Duration::from_secs(30), |editor| {
                editor.export_operation.pending.is_none()
            });
            app.read(|editor| {
                assert!(
                    !editor.busy && editor.failure.is_none(),
                    "{}",
                    editor.status
                );
                assert!(editor.tasks.view().is_none());
                if replace {
                    assert!(editor.document.project.sources.is_empty());
                } else {
                    assert_eq!(editor.document.project, accepted);
                }
            });
            verify_tiff(&path);
        });
    }
}

#[test]
fn superseded_export_keeps_physical_admission_until_its_submitted_gpu_work_retires() {
    let editor = fixture();
    let (starts, release) = hold_first_submission(&editor);
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old.tif");
    let new = directory.path().join("new.tif");
    with_application(editor, |app| {
        app.enter(|editor, access| {
            editor.export(
                old.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                access,
            );
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(30)).unwrap();
        let (id, cancelled) = app.read(|editor| {
            (
                editor.export_operation.pending.unwrap(),
                editor.export_operation.cancelled.clone().unwrap(),
            )
        });
        app.enter(|editor, access| {
            editor.export(
                new.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                access,
            );
        });
        app.pump();
        assert!(cancelled.load(Ordering::Relaxed));
        app.read(|editor| {
            assert_ne!(editor.export_operation.pending, Some(id));
            assert!(!editor.workflow_jobs.idle());
            assert!(editor.busy);
        });
        assert!(!old.exists() && !new.exists());
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |editor| {
            editor.export_operation.pending.is_none()
        });
        app.read(|editor| {
            assert!(
                !editor.busy && editor.failure.is_none(),
                "{}",
                editor.status
            )
        });
        assert!(!old.exists());
        verify_tiff(&new);
    });
}

#[test]
fn application_retirement_signals_stop_but_retains_the_admitted_export_source_until_cleanup() {
    let editor = fixture();
    let (starts, release) = hold_first_submission(&editor);
    let jobs = editor.workflow_jobs.clone();
    let source = Arc::downgrade(&editor.document.visit.ready.as_ref().unwrap().developed);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retired.tif");
    with_application(editor, |app| {
        app.enter(|editor, access| {
            editor.export(
                path.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                access,
            );
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(30)).unwrap();
        let cancelled = app.read(|editor| editor.export_operation.cancelled.clone().unwrap());
        app.close_execution();
        assert!(cancelled.load(Ordering::Relaxed));
        app.update(|editor| editor.document = Document::default().into());
        assert!(source.upgrade().is_some());
        assert!(!jobs.idle());
    });
    assert!(source.upgrade().is_some());
    assert!(!jobs.idle());
    release.send(()).unwrap();
    let start = Instant::now();
    while !jobs.idle() || source.upgrade().is_some() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "retired export did not release its physical inputs"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn a_stop_after_publication_acknowledges_the_committed_file_without_rewriting_success() {
    let editor = fixture();
    let (starts, release) = hold_first_submission(&editor);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("committed.tif");
    with_application(editor, |app| {
        app.enter(|editor, access| {
            editor.export(
                path.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                access,
            );
        });
        app.pump();
        starts.recv_timeout(Duration::from_secs(30)).unwrap();
        release.send(()).unwrap();
        // The worker can publish while UI continuation delivery remains unpolled.
        let start = Instant::now();
        while !path.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "export did not publish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.read(|editor| {
            assert!(editor.export_operation.pending.is_some());
            editor
                .export_operation
                .cancelled
                .as_ref()
                .unwrap()
                .store(true, Ordering::Relaxed);
        });
        app.run_until(Duration::from_secs(30), |editor| {
            editor.export_operation.pending.is_none()
        });
        app.read(|editor| {
            assert!(
                !editor.busy && editor.failure.is_none(),
                "{}",
                editor.status
            );
            assert!(editor.status.contains(&path.display().to_string()));
        });
        verify_tiff(&path);
    });
}

#[test]
fn captured_service_writes_after_ui_ownership_ends_and_preserves_destinations_on_stop() {
    for stopped in [false, true] {
        let mut editor = fixture();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("service.tif");
        if stopped {
            std::fs::write(&path, b"existing destination").unwrap();
        }
        editor.sync_document();
        let request = editor
            .export_input(
                path.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                if stopped {
                    Overwrite::Confirmed
                } else {
                    Overwrite::Refuse
                },
            )
            .unwrap()
            .unwrap();
        let services = editor.export_services.take().unwrap();
        drop(editor);
        let result =
            pollster::block_on(services.write(request, Arc::new(AtomicBool::new(stopped))));
        if stopped {
            assert!(result.unwrap_err().contains("cancel"));
            assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        } else {
            assert_eq!(result.unwrap(), path);
            verify_tiff(&path);
        }
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
