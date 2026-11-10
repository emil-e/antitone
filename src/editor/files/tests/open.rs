use super::*;
use crate::{
    editor::{
        export::{Draft, Kind, Outcome},
        source_loading,
    },
    project::Save,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, mpsc},
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn editor(loader: source_loading::SourceLoader, limits: wgpu::Limits) -> Editor {
    let (device, queue) = pollster::block_on(async {
        crate::gpu::instance()
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            })
            .await
            .unwrap()
    });
    let mut editor = Editor::new(&device, &queue);
    editor.source_services.as_mut().unwrap().developments = loader.clone();
    editor.developments = Some(loader);
    editor
}

fn saved(directory: &Path) -> Document {
    let mut document = Document::default();
    let paths: Vec<_> = ["first.dng", "second.dng"]
        .into_iter()
        .map(|name| {
            let destination = directory.join(name);
            std::fs::copy(path("patches.dng"), &destination).unwrap();
            destination
        })
        .collect();
    document.import(&paths).unwrap();
    document.project.roll.exposure = 0.375;
    document.project.set_dmin(Some([0.7; 3]));
    document.project.set_dmax(Some([0.01; 3]));
    document.project.order.reverse();
    document
        .save(&directory.join("grade.antitone"), Save::New)
        .unwrap()
}

#[test]
fn all_missing_photos_open_with_intact_metadata_and_a_clean_history() {
    let directory = tempfile::tempdir().unwrap();
    let saved = saved(directory.path());
    for source in &saved.project.sources {
        std::fs::remove_file(&source.path).unwrap();
    }
    let loader = source_loading::SourceLoader::new();
    let mut editor = editor(loader.clone(), Default::default());
    let before = editor.history_snapshot();
    editor.document.project.roll.exposure = 1.;
    editor.record_edit(before, "Old exposure", None);
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.open_project(saved.path.clone().unwrap(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.files.opening() && !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.state.project, saved.project);
            assert_eq!(editor.document.path, saved.path);
            assert_eq!(
                editor.document.visit.current_source,
                saved.project.order.first().copied()
            );
            assert!(!editor.document.dirty());
            assert!(editor.history_label(history::Direction::Undo).is_none());
            assert!(editor.history_label(history::Direction::Redo).is_none());
            assert!(editor.document.visit.canvas.ready.is_none());
            assert!(!editor.interaction_blocked());
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn unreadable_malformed_and_unsupported_projects_preserve_the_current_document_and_view() {
    let directory = tempfile::tempdir().unwrap();
    let malformed = directory.path().join("malformed.antitone");
    std::fs::write(&malformed, b"{").unwrap();
    let unsupported = directory.path().join("unsupported.antitone");
    let mut editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let before = editor.history_snapshot();
    editor.document.project.roll.exposure = 0.625;
    editor.record_edit(before, "Old exposure", None);
    editor.document.visit.canvas.viewport.zoom = Some(1.375);
    let mut invalid = serde_json::to_value(&editor.document.project).unwrap();
    invalid["schema"] = serde_json::json!(u32::MAX);
    std::fs::write(&unsupported, serde_json::to_vec(&invalid).unwrap()).unwrap();
    let accepted = editor.document.state.project.clone();
    let visit = editor.document.visit.view;
    let native = editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .developed
        .clone();
    with_application(editor, |app| {
        for path in [
            directory.path().join("absent.antitone"),
            malformed,
            unsupported,
        ] {
            app.enter(|editor, cx| editor.open_project(path, cx));
            app.run_until(Duration::from_secs(10), |editor| !editor.files.opening());
            app.read(|editor| {
                assert_eq!(editor.document.state.project, accepted);
                assert_eq!(editor.document.visit.view, visit);
                assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(1.375));
                assert!(Arc::ptr_eq(
                    &editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .developed,
                    &native
                ));
                assert_eq!(
                    editor.history_label(history::Direction::Undo),
                    Some("Old exposure")
                );
                assert!(editor.document.dirty());
                assert!(!editor.interaction_blocked());
            });
        }
    });
}

struct Release(Option<mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(send) = self.0.take() {
            let _ = send.send(());
        }
    }
}

#[test]
fn startup_and_menu_open_accept_before_raw_work_and_retire_it_on_new() {
    for startup in [false, true] {
        for retire in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let saved = saved(directory.path());
            let (entered, starts) = mpsc::channel();
            let (send, resume) = mpsc::channel();
            let resume = Mutex::new(resume);
            let release = Release(Some(send));
            let loader = source_loading::SourceLoader::new_observed(move |_, phase, _| {
                if matches!(phase, source_loading::NativePhase::UnpackBegin) {
                    entered.send(()).unwrap();
                    resume
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(30))
                        .unwrap();
                }
                Ok(())
            });
            with_application(editor(loader.clone(), Default::default()), |app| {
                app.enter(|editor, cx| {
                    let path = saved.path.clone().unwrap();
                    if startup {
                        editor.open_initial(path, cx);
                    } else {
                        editor.guard_transition(Transition::Open(path), cx);
                    }
                });
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                loop {
                    app.pump();
                    if starts.try_recv().is_ok() {
                        break;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "selected RAW did not reach the observed native boundary"
                    );
                    std::thread::sleep(Duration::from_millis(1));
                }
                let visit = app.read(|editor| {
                    assert_eq!(editor.document.state.project, saved.project);
                    assert!(!editor.document.dirty() && !editor.files.opening());
                    assert!(editor.document.visit.source.pending());
                    assert!(editor.document.visit.canvas.ready.is_none());
                    assert!(!editor.interaction_blocked());
                    editor.document.visit.view
                });
                let accepted = app.update(|editor| {
                    if retire {
                        editor.new_project();
                    } else {
                        let mut edit = editor.edit_project("Exposure while loading").unwrap();
                        edit.update(|project| project.roll.exposure = 0.625);
                        editor.commit_project(edit).unwrap();
                    }
                    editor.document.state.project.clone()
                });
                drop(release);
                if retire {
                    loader.close_and_wait().unwrap();
                    app.pump();
                }
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.visit.source.pending()
                });
                app.read(|editor| {
                    assert_eq!(editor.document.state.project, accepted);
                    if retire {
                        assert_ne!(editor.document.visit.view, visit);
                        assert!(editor.document.visit.canvas.ready.is_none());
                        assert!(editor.document.project.sources.is_empty());
                    } else {
                        assert_eq!(editor.document.visit.view, visit);
                        assert!(
                            editor.document.visit.canvas.ready.is_some(),
                            "{}",
                            editor.status
                        );
                        assert_eq!(editor.document.project.roll.exposure, 0.625);
                        assert_eq!(
                            editor.history_label(history::Direction::Undo),
                            Some("Exposure while loading")
                        );
                    }
                });
            });
            if !retire {
                loader.close_and_wait().unwrap();
            }
        }
    }
}

#[test]
fn a_bad_first_photo_does_not_roll_back_open_or_prevent_loading_another_photo() {
    for failure in ["missing", "corrupt", "incompatible"] {
        let directory = tempfile::tempdir().unwrap();
        let mut saved = saved(directory.path());
        let first = saved.project.order[0];
        let first_path = saved.source_path(first).unwrap();
        match failure {
            "missing" => std::fs::remove_file(first_path).unwrap(),
            "corrupt" => std::fs::write(first_path, b"not a RAW photograph").unwrap(),
            "incompatible" => {
                saved
                    .project
                    .source_mut(first)
                    .unwrap()
                    .development
                    .dimensions[0] += 1;
                saved
                    .project
                    .save(saved.path.as_ref().unwrap(), Save::Replace)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        let loader = source_loading::SourceLoader::new();
        with_application(editor(loader.clone(), Default::default()), |app| {
            app.enter(|editor, cx| editor.open_project(saved.path.clone().unwrap(), cx));
            app.run_until(Duration::from_secs(15), |editor| {
                !editor.files.opening() && !editor.document.visit.source.pending()
            });
            app.read(|editor| {
                assert_eq!(editor.document.state.project, saved.project);
                assert_eq!(editor.document.visit.current_source, Some(first));
                assert!(editor.document.visit.canvas.ready.is_none());
            });
            let second = saved.project.order[1];
            app.enter(|editor, cx| editor.load(saved.source_path(second).unwrap(), cx));
            app.run_until(Duration::from_secs(15), |editor| {
                !editor.document.visit.source.pending()
            });
            app.read(|editor| {
                assert_eq!(editor.document.visit.current_source, Some(second));
                assert!(
                    editor.document.visit.canvas.ready.is_some(),
                    "{}",
                    editor.status
                );
                assert_eq!(editor.document.state.project, saved.project);
                assert!(!editor.document.dirty());
            });
        });
        loader.close_and_wait().unwrap();
    }
}

#[test]
fn correction_resource_failure_keeps_the_enabled_map_visible_and_saveable() {
    let directory = tempfile::tempdir().unwrap();
    let mut saved = saved(directory.path());
    let reference = saved.project.order[1];
    let record = saved.project.source(reference).unwrap().clone();
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            record.development.active_area,
            [256, 256],
            vec![glam::DVec3::new(1.1, 0.9, 1.05); 256 * 256],
        )
        .unwrap(),
    );
    let settings = crate::project::FlatFieldSettings::new(record.path.clone(), map);
    saved
        .project
        .set_flat_field(Some(settings.clone()))
        .unwrap();
    saved.project.remove_from_filmstrip(reference).unwrap();
    saved = saved
        .save(saved.path.as_ref().unwrap(), Save::Replace)
        .unwrap();
    std::fs::remove_file(&record.path).unwrap();
    let loader = source_loading::SourceLoader::new();
    let editor = editor(
        loader.clone(),
        wgpu::Limits {
            // The active 240×176 RGBA source fits; the 256×256 RGB correction grid does not.
            max_storage_buffer_binding_size: 720_896,
            ..Default::default()
        },
    );
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.open_project(saved.path.clone().unwrap(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.files.opening()
                && !editor.document.correction.pending()
                && !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.state.project, saved.project);
            assert_eq!(editor.document.project.active_flat_field(), Some(&settings));
            assert!(editor.document.correction.prepared().is_none());
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "{}",
                editor.status
            );
            assert!(
                editor.document.source,
                "resource failure falls back to the existing source view"
            );
            assert!(
                editor
                    .document
                    .correction
                    .error()
                    .unwrap()
                    .contains("flat-field grid exceeds GPU limits")
            );
            assert!(!editor.document.dirty());
        });
        let (source, request, services, failure) = app.enter(|editor, _| {
            editor.status = "An unrelated operation finished".into();
            let mut draft = Draft::new(OutputTarget::DisplayP3, directory.path().to_owned());
            draft.kind = Kind::Exr;
            (
                editor.capture_source_context().unwrap(),
                editor.capture_export(&draft).unwrap(),
                editor.export_services.clone().unwrap(),
                editor.document.correction.error().unwrap().to_owned(),
            )
        });
        let source_error = pollster::block_on(source)
            .err()
            .expect("correction acquisition must fail");
        assert_eq!(source_error, failure);
        let summary = pollster::block_on(services.export_batch(
            request,
            crate::cancellation::Cancellation::default(),
            |_, _| {},
        ));
        assert_eq!((summary.exported(), summary.failed()), (0, 1));
        let Outcome::Failed { path, error } = &summary.outcomes[0] else {
            panic!("expected preparation failure")
        };
        assert_eq!(error, &failure);
        assert!(!path.exists());
        let context = egui::Context::default();
        let mut labels = Vec::new();
        for _ in 0..3 {
            labels = ui_frame(app, &context, vec![]);
        }
        assert!(
            labels
                .iter()
                .any(|(label, _)| label.contains("flat-field grid exceeds GPU limits"))
        );
        let destination = directory.path().join("still-corrected.antitone");
        app.enter(|editor, cx| editor.save_project(destination.clone(), Save::New, None, cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.files.pending() && editor.document_pending.is_none()
        });
        let written = crate::project::Project::load(&destination).unwrap();
        assert_eq!(written.active_flat_field(), Some(&settings));
        assert_eq!(written.sources, saved.project.sources);
        app.read(|editor| assert!(!editor.document.dirty()));
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn queued_open_cannot_replace_new_and_closed_execution_keeps_the_current_document() {
    let directory = tempfile::tempdir().unwrap();
    let saved = saved(directory.path());
    let hold = crate::background::testing::pause();
    with_application(Editor::state(), |app| {
        app.enter(|editor, cx| {
            editor.open_project(saved.path.clone().unwrap(), cx);
            assert!(editor.files.pending() && !editor.files.closing());
        });
        app.pump();
        app.update(Editor::new_project);
        drop(hold);
        pollster::block_on(crate::background::run(|_| {})).unwrap();
        app.pump();
        app.read(|editor| {
            assert_eq!(*editor.document.project, crate::project::Project::default());
            assert!(editor.document.path.is_none());
            assert!(!editor.files.pending() && editor.activities.view().is_none());
        });
        app.close_execution();
        let accepted = app.read(|editor| editor.document.state.project.clone());
        app.enter(|editor, cx| editor.open_project(saved.path.clone().unwrap(), cx));
        app.read(|editor| {
            assert_eq!(editor.document.state.project, accepted);
            assert_eq!(editor.status, "Project ready");
            assert!(!editor.files.pending() && !editor.interaction_blocked());
            assert!(editor.activities.view().is_none());
        });
    });
}

#[test]
fn new_needs_neither_source_services_nor_a_gpu_and_retires_unfinished_edits() {
    let mut editor = Editor::state();
    editor.document.project.roll.exposure = 1.;
    let mut edit = editor.edit_project("Retired exposure").unwrap();
    edit.update(|project| project.roll.exposure = 2.);
    editor.new_project();
    assert!(editor.source_services.is_none());
    assert!(editor.document.visit.working_view.device().is_none());
    assert_eq!(*editor.document.project, crate::project::Project::default());
    assert!(!editor.document.dirty());
    assert!(!editor.editing() && !editor.interaction_blocked());
    assert!(editor.commit_project(edit).is_err());
}
