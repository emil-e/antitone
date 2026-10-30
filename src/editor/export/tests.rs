use super::batch::Photo;
use super::*;
mod output;
mod services;

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

impl Editor {
    #[cfg(test)]
    pub(in crate::editor) fn export_input(
        &self,
        path: PathBuf,
        format: OutputFormat,
        overwrite: Overwrite,
    ) -> Result<Option<PhotoRequest>, Error> {
        if self.dust_search_pending() {
            return Err(Error::InvalidParameter(
                "wait for donor selection before exporting",
            ));
        }
        let Some(ready) = &self.document.visit.canvas.ready else {
            return Ok(None);
        };
        if !self.document.input.ready() {
            return Ok(None);
        }
        self.validate_document_candidate(&self.document)?;
        let id = self
            .document
            .visit
            .current_source
            .ok_or_else(|| Error::Project("no accepted source for export".into()))?;
        let recipe = self.document.image_recipe(id)?;
        debug_assert_eq!(recipe.composition.area(), ready.area);
        Ok(Some(PhotoRequest {
            input: self.source_context().expect("ready export source"),
            path,
            format,
            recipe,
            overwrite,
        }))
    }

    #[cfg(test)]
    pub(in crate::editor) fn export(
        &mut self,
        path: PathBuf,
        format: OutputFormat,
        overwrite: Overwrite,
        cx: &UiContext<'_, Self>,
    ) {
        if self.document.auto_crop_edit().is_some() {
            self.status = "Apply or cancel auto-crop before exporting".into();
            return;
        }
        if self
            .document
            .path
            .as_ref()
            .is_some_and(|project| super::document::same_path(project, &path))
        {
            self.status = "Export destination is the current project document".into();
            self.report_failure(FailureKind::Export, self.status.clone());
            return;
        }
        self.sync_document();
        let request = match self.export_input(path, format, overwrite) {
            Ok(Some(request)) => request,
            Ok(None) => return,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let mut protected = self
            .document
            .project
            .sources
            .iter()
            .filter_map(|s| self.document.source_path(s.id).ok())
            .collect::<Vec<_>>();
        protected.extend(self.document.path.clone());
        if let Err(error) = batch::require_safe(&request.path, &protected) {
            self.status = error.to_string();
            self.report_failure(FailureKind::Export, self.status.clone());
            return;
        }
        self.export_photos(
            batch::Request {
                photos: vec![Photo {
                    input: Input::Ready(request.input),
                    recipe: request.recipe,
                    path: request.path,
                }],
                format,
                collision: if matches!(overwrite, Overwrite::Confirmed) {
                    Collision::Replace
                } else {
                    Collision::Refuse
                },
                protected,
            },
            cx,
        );
    }
}

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

fn wait_for_submission(
    app: &mut crate::application::test_support::TestApplication<'_, Editor>,
    starts: &mpsc::Receiver<()>,
) {
    let start = Instant::now();
    loop {
        // A batch has multiple service awaits before GPU work. Drive those
        // continuations while waiting for the actual submitted-work gate.
        app.pump();
        if starts.try_recv().is_ok() {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "export did not submit GPU work"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn fixture() -> Editor {
    pollster::block_on(async {
        let instance = crate::gpu::instance();
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let mut editor = Editor::new(&device, &queue);
        editor.document.thumbnails =
            (Thumbnails::default()).with_sources(editor.document.cache.clone());
        editor = complete_document_operation(editor, Duration::from_secs(30), |editor, cx| {
            editor.add_raws(
                vec![
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures/raw/patches.dng"),
                ],
                cx,
            )
        });
        editor.set_dmin(Some(DVec3::splat(0.8)));
        editor.set_dmax(Some(DVec3::splat(0.02)));
        let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
        editor.document.visit.crop = Some(Crop {
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
fn captured_export_completes_without_paint_without_rereading_live_document_data() {
    for replace_data in [false, true] {
        let editor = fixture();
        let accepted = editor.document.state.project.clone();
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

                if replace_data {
                    editor.document.install(Document::default());
                }
            });
            app.run_until(Duration::from_secs(30), |editor| {
                editor.document.export.operation.is_none()
            });
            app.read(|editor| {
                assert!(
                    !editor.busy && editor.failure.is_none(),
                    "{}",
                    editor.status
                );
                assert!(editor.tasks.view().is_none());
                if replace_data {
                    assert!(editor.document.state.project.sources.is_empty());
                } else {
                    assert_eq!(editor.document.state.project, accepted);
                }
            });
            verify_tiff(&path);
        });
    }
}

#[test]
fn replacing_the_export_owner_discards_old_output_and_publishes_the_captured_replacement() {
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
        wait_for_submission(app, &starts);
        let (id, cancelled) = app.read(|editor| {
            (
                editor.document.export.operation.as_ref().unwrap().activity,
                editor
                    .document
                    .export
                    .operation
                    .as_ref()
                    .unwrap()
                    .cancelled
                    .clone(),
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
        assert!(cancelled.is_cancelled());
        app.update(|editor| {
            let current = editor.document.export.operation.as_ref().unwrap().activity;
            assert_ne!(current, id);
            editor.cancel_task(TaskRequest::Export(id));
            editor.tasks.report(
                TaskRequest::Export(id),
                TaskProgress::Counted {
                    completed: 9,
                    total: 10,
                },
            );
            assert!(
                !editor
                    .document
                    .export
                    .operation
                    .as_ref()
                    .unwrap()
                    .cancelled
                    .is_cancelled()
            );
            assert!(editor.tasks.is_current(TaskRequest::Export(current)) && editor.busy);
        });
        assert!(!old.exists());
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |editor| {
            editor.document.export.operation.is_none()
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
fn application_retirement_signals_stop_but_keeps_the_export_source_until_cleanup() {
    let editor = fixture();
    let (starts, release) = hold_first_submission(&editor);
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
        wait_for_submission(app, &starts);
        let cancelled = app.read(|editor| {
            editor
                .document
                .export
                .operation
                .as_ref()
                .unwrap()
                .cancelled
                .clone()
        });
        app.close_execution();
        assert!(cancelled.is_cancelled());
        app.update(|editor| editor.document = Document::default().into());
        assert!(source.upgrade().is_some());
    });
    assert!(source.upgrade().is_some());
    release.send(()).unwrap();
    let start = Instant::now();
    while source.upgrade().is_some() {
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
        wait_for_submission(app, &starts);
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
            assert!(editor.document.export.operation.is_some());
            editor
                .document
                .export
                .operation
                .as_ref()
                .unwrap()
                .cancelled
                .cancel();
        });
        app.run_until(Duration::from_secs(30), |editor| {
            editor.document.export.operation.is_none()
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
fn running_cancel_waits_for_acknowledgement_and_preserves_the_destination() {
    let editor = fixture();
    let accepted = editor.document.state.project.clone();
    let (starts, release) = hold_first_submission(&editor);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.tif");
    std::fs::write(&path, b"existing destination").unwrap();
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.export(
                path.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Confirmed,
                cx,
            )
        });
        wait_for_submission(app, &starts);
        app.update(|editor| {
            let activity = editor.document.export.operation.as_ref().unwrap().activity;
            editor.cancel_task(TaskRequest::Export(activity));
        });
        app.read(|editor| {
            let operation = editor.document.export.operation.as_ref().unwrap();
            assert!(operation.cancelled.is_cancelled());
            assert!(editor.busy && editor.tasks.view().unwrap().cancel.is_none());
            assert_eq!(editor.document.state.project, accepted);
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(30), |editor| {
            editor.document.export.operation.is_none()
        });
        app.read(|editor| {
            assert!(!editor.busy && editor.tasks.view().is_none() && editor.failure.is_none());
            assert!(editor.status.contains("cancel"));
            assert_eq!(editor.document.state.project, accepted);
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"existing destination");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    });
}

#[test]
fn failed_export_survives_status_updates_until_a_successful_retry() {
    let editor = fixture();
    let accepted = editor.document.state.project.clone();
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("missing");
    let path = folder.join("retry.tif");
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.export(
                path.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                cx,
            )
        });
        app.run_until(Duration::from_secs(30), |editor| {
            editor.document.export.operation.is_none()
        });
        let details = app.read(|editor| {
            assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
            assert!(!editor.busy && editor.tasks.view().is_none());
            editor.failure.as_ref().unwrap().details.clone()
        });
        assert!(!path.exists());
        app.update(|editor| editor.status = "Unrelated status".into());
        std::fs::create_dir(&folder).unwrap();
        let paused = crate::background::testing::pause();
        app.enter(|editor, cx| {
            editor.export(
                path.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                cx,
            )
        });
        app.pump();
        app.read(|editor| {
            assert!(editor.busy && editor.document.export.operation.is_some());
            assert_eq!(editor.failure.as_ref().unwrap().details, details);
        });
        drop(paused);
        app.run_until(Duration::from_secs(30), |editor| {
            editor.document.export.operation.is_none()
        });
        app.read(|editor| {
            assert!(editor.failure.is_none() && !editor.busy && editor.tasks.view().is_none());
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(
                editor.document.export.summary.as_ref().unwrap().exported(),
                1
            );
        });
        verify_tiff(&path);
    });
}

#[test]
fn document_retirement_releases_queued_export_inputs_before_dispatch() {
    for polled in [false, true] {
        let editor = fixture();
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
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("queued.tif");
        let paused = crate::background::testing::pause();
        with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.export(
                    path.clone(),
                    OutputFormat::Tiff(OutputTarget::Srgb),
                    Overwrite::Refuse,
                    cx,
                )
            });
            if polled {
                app.pump();
            }
            let cancelled = app.read(|editor| {
                editor
                    .document
                    .export
                    .operation
                    .as_ref()
                    .unwrap()
                    .cancelled
                    .clone()
            });
            app.update(|editor| editor.document = editor.document.replacement(Document::default()));
            assert!(cancelled.is_cancelled());
            app.pump();
            assert!(
                source.upgrade().is_none(),
                "retired queued work retained its source"
            );
            drop(paused);
            app.pump();
            assert!(!path.exists());
        });
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }
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

#[test]
fn document_retirement_cancels_export_while_the_physical_call_keeps_its_inputs() {
    let editor = fixture();
    let (starts, release) = hold_first_submission(&editor);
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
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("retired-document.tif");
    with_application(editor, |app| {
        app.enter(|editor, access| {
            editor.export(
                path.clone(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
                access,
            )
        });
        wait_for_submission(app, &starts);
        let stopped = app.read(|editor| {
            editor
                .document
                .export
                .operation
                .as_ref()
                .unwrap()
                .cancelled
                .clone()
        });
        app.update(|editor| editor.document = editor.document.replacement(Document::default()));
        assert!(stopped.is_cancelled());
        assert!(
            source.upgrade().is_some(),
            "running export lost its captured native input"
        );
        app.pump();
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while source.upgrade().is_some() {
            app.pump();
            assert!(
                Instant::now() < deadline,
                "retired export did not release its physical input"
            );
            std::thread::yield_now();
        }
        app.read(|editor| {
            assert!(editor.document.export.operation.is_none());
            assert!(
                editor.document.export.summary.is_none(),
                "retired work updated the replacement document"
            );
        });
    });
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}
