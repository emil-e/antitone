use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{source_loading, tests::authored_fixture},
};
use std::{
    path::Path,
    sync::{Mutex, mpsc},
    time::Duration,
};

fn copy_photo(directory: &Path, name: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        &path,
    )
    .unwrap();
    path
}

fn wait(app: &mut TestApplication<'_, Editor>) {
    app.run_until(Duration::from_secs(15), |e| e.document.importing.is_none());
}

#[test]
fn metadata_import_needs_no_services_and_display_failure_stays_with_the_accepted_photo() {
    let directory = tempfile::tempdir().unwrap();
    let photo = copy_photo(directory.path(), "photo.dng");
    with_application(Editor::state(), |app| {
        app.enter(|e, cx| e.add_raws(vec![photo.clone()], cx));
        wait(app);
        app.run_until(Duration::from_secs(15), |e| {
            !e.document.visit.source.pending()
        });
        app.read(|e| {
            let id = e.document.project.order[0];
            assert_eq!(e.document.project.sources.len(), 1);
            assert_eq!(e.document.source_path(id).unwrap(), photo);
            assert_eq!(e.document.visit.current_source, Some(id));
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(e.document.dirty());
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Import photos")
            );
            assert!(!e.editing() && !e.interaction_blocked());
            let saved = e
                .document
                .capture()
                .save(
                    &directory.path().join("grade.antitone"),
                    crate::project::Save::New,
                )
                .unwrap();
            assert_eq!(
                Document::open(saved.path.as_deref().unwrap())
                    .unwrap()
                    .project,
                saved.project
            );
        });
    });
}

#[test]
fn duplicate_paths_are_no_ops_even_when_the_original_file_is_now_missing() {
    let directory = tempfile::tempdir().unwrap();
    let photo = copy_photo(directory.path(), "photo.dng");
    let e = authored_fixture(&[photo.to_str().unwrap()]);
    let before = e.document.project.clone();
    let key = e.document.file_key(&photo);
    let view = e.document.visit.view;
    let pixels = e
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .developed
        .clone();
    std::fs::remove_file(&photo).unwrap();
    with_application(e, |app| {
        app.enter(|e, cx| {
            e.status = "Existing status".into();
            e.add_raws(vec![photo.clone(), photo.clone()], cx);
        });
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert_eq!(e.document.file_key(&photo), key);
            assert_eq!(e.document.visit.view, view);
            assert!(Arc::ptr_eq(
                &e.document.visit.canvas.ready.as_ref().unwrap().developed,
                &pixels
            ));
            assert_eq!(e.status, "Existing status");
            assert!(e.dialogs.message().is_none());
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| {
            assert!(
                e.document.project.sources.is_empty(),
                "duplicate import added an undo entry"
            )
        });
    });
}

#[test]
fn a_bad_addition_is_atomic_and_a_good_addition_keeps_the_current_visit() {
    let directory = tempfile::tempdir().unwrap();
    let first = copy_photo(directory.path(), "first.dng");
    let second = copy_photo(directory.path(), "second.dng");
    let bad = directory.path().join("bad.dng");
    std::fs::write(&bad, b"not a RAW file").unwrap();
    let mut e = authored_fixture(&[first.to_str().unwrap()]);
    e.document.visit.canvas.viewport.zoom = Some(1.375);
    let before = e.document.project.clone();
    let view = e.document.visit.view;
    let viewport = e.document.visit.canvas.viewport;
    let pixels = e
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .developed
        .clone();
    let selected = e.document.visit.current_source;
    with_application(e, |app| {
        app.enter(|e, cx| e.add_raws(vec![second.clone(), bad], cx));
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert_eq!(e.document.visit.view, view);
            assert!(Arc::ptr_eq(
                &e.document.visit.canvas.ready.as_ref().unwrap().developed,
                &pixels
            ));
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Import photos")
            );
        });
        app.enter(|e, cx| e.add_raws(vec![second.clone(), second], cx));
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project.order.len(), 2);
            assert_eq!(
                e.document.project.source(before.order[0]).unwrap(),
                before.source(before.order[0]).unwrap()
            );
            assert_eq!(e.document.visit.current_source, selected);
            assert_eq!(e.document.visit.view, view);
            assert_eq!(e.document.visit.canvas.viewport, viewport);
            assert!(Arc::ptr_eq(
                &e.document.visit.canvas.ready.as_ref().unwrap().developed,
                &pixels
            ));
        });
        app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.interaction_blocked());
        app.read(|e| assert_eq!(e.document.project, before));
    });
}

#[test]
fn an_unavailable_existing_photo_does_not_veto_a_valid_addition() {
    let directory = tempfile::tempdir().unwrap();
    let first = copy_photo(directory.path(), "first.dng");
    let second = copy_photo(directory.path(), "second.dng");
    let mut document = Document::default();
    document.import(std::slice::from_ref(&first)).unwrap();
    let id = document.project.order[0];
    std::fs::remove_file(first).unwrap();
    let mut e = Editor::state();
    e.document = document.into();
    e.document.visit.current_source = Some(id);
    let view = e.document.visit.view;
    with_application(e, |app| {
        app.enter(|e, cx| e.add_raws(vec![second], cx));
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project.order.len(), 2);
            assert_eq!(e.document.visit.current_source, Some(id));
            assert_eq!(e.document.visit.view, view);
            assert!(!e.document.visit.source.pending());
        });
    });
}

#[test]
fn adding_photos_does_not_retire_a_selected_native_load() {
    let directory = tempfile::tempdir().unwrap();
    let first = copy_photo(directory.path(), "first.dng");
    let second = copy_photo(directory.path(), "second.dng");
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let held = first.clone();
    let loader = source_loading::SourceLoader::new_observed(move |path, phase, _| {
        if path == held && matches!(phase, source_loading::NativePhase::UnpackBegin) {
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
        }
        Ok(())
    });
    let mut e = authored_fixture(&[]);
    e.source_services.as_mut().unwrap().developments = loader.clone();
    e.developments = Some(loader.clone());
    e.document.import(std::slice::from_ref(&first)).unwrap();
    with_application(e, |app| {
        app.enter(|e, cx| e.load(first, cx));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native load did not reach its barrier"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let (view, selected) =
            app.read(|e| (e.document.visit.view, e.document.visit.current_source));
        app.enter(|e, cx| e.add_raws(vec![second], cx));
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project.order.len(), 2);
            assert_eq!(e.document.visit.view, view);
            assert_eq!(e.document.visit.current_source, selected);
            assert!(e.document.visit.source.pending());
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(!e.interaction_blocked());
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |e| {
            !e.document.visit.source.pending()
        });
        app.read(|e| {
            assert!(e.document.visit.canvas.ready.is_some(), "{}", e.status);
            assert_eq!(e.document.visit.view, view);
            assert_eq!(e.document.visit.current_source, selected);
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn document_retirement_and_closed_execution_drop_the_import_owner() {
    for poll_first in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let photo = copy_photo(directory.path(), "queued.dng");
        let hold = crate::background::testing::pause();
        with_application(Editor::state(), |app| {
            app.enter(|e, cx| e.add_raws(vec![photo], cx));
            app.read(|e| {
                assert!(e.editing() && e.interaction_blocked());
                assert!(e.document.project.sources.is_empty());
            });
            if poll_first {
                app.pump();
            }
            app.enter(|e, _| e.new_project());
            app.pump();
            app.read(|e| {
                assert!(!e.editing() && !e.interaction_blocked());
                assert!(e.activities.view().is_none());
            });
            drop(hold);
            app.pump();
            app.read(|e| assert!(e.document.project.sources.is_empty()));
        });
    }
    let directory = tempfile::tempdir().unwrap();
    let photo = copy_photo(directory.path(), "closed.dng");
    with_application(Editor::state(), |app| {
        app.close_execution();
        app.enter(|e, cx| e.add_raws(vec![photo], cx));
        app.read(|e| {
            assert!(e.document.importing.is_none());
            assert!(!e.editing() && e.activities.view().is_none());
            assert!(e.document.project.sources.is_empty());
            assert!(e.dialogs.message().is_none());
        });
    });
}

#[test]
fn loading_completion_before_import_publication_keeps_matching_viewing_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let first = copy_photo(directory.path(), "first.dng");
    let raw = std::fs::read(&first).unwrap();
    std::fs::write(&first, crate::editor::tests::dng_with_red_as_shot(&raw)).unwrap();
    let added = copy_photo(directory.path(), "added.dng");
    let mut e = authored_fixture(&[first.to_str().unwrap()]);
    let ready = e.document.visit.canvas.ready.take().unwrap();
    let id = e.document.visit.current_source.unwrap();
    let view = e.document.visit.view;
    let white = crate::raw::SourceWhiteBalance::AsShot;
    let expected = ready
        .developed
        .metadata()
        .source_preview(ready.developed.white_balance())
        .transform(white);
    assert_ne!(
        expected,
        glam::DMat3::IDENTITY,
        "fixture must distinguish lost viewing metadata"
    );
    let saved = e
        .document
        .capture()
        .save(
            &directory.path().join("opened.antitone"),
            crate::project::Save::New,
        )
        .unwrap();
    // Metadata Open has no runtime viewing transforms until source completion.
    e.document = e
        .document
        .replacement(Document::open(saved.path.as_deref().unwrap()).unwrap());
    e.document.visit.current_source = Some(id);
    e.document.visit.view = view;
    e.source_white_balance = white;
    assert_eq!(
        e.document
            .source_preview(e.document.project.source(id).unwrap())
            .transform(white),
        glam::DMat3::IDENTITY
    );
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        app.enter(|e, cx| e.add_raws(vec![added], cx));
        app.pump();
        // Deliver a genuinely acquired source through its production completion boundary,
        // after Import captured its private document and before it can publish.
        app.enter(|e, _| e.source_result(view, Ok(Box::new(ready))));
        app.read(|e| {
            assert_eq!(
                e.document
                    .source_preview(e.document.project.source(id).unwrap())
                    .transform(white),
                expected
            );
            assert!(e.document.importing.is_some());
        });
        drop(hold);
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project.sources.len(), 2);
            assert_eq!(
                e.document
                    .source_preview(e.document.project.source(id).unwrap())
                    .transform(white),
                expected
            );
            let canvas = e.source_parameters(0.).unwrap();
            let card = e
                .source_parameters_for(e.document.project.source(id).unwrap(), 0.)
                .unwrap();
            assert_eq!(bytemuck::bytes_of(&canvas), bytemuck::bytes_of(&card));
        });
        app.enter(|e, _| {
            let mut changed = Document::open(saved.path.as_deref().unwrap()).unwrap();
            changed.project.input = crate::processing::InputSettings::Digital;
            changed
                .project
                .source_mut(id)
                .unwrap()
                .development
                .camera_to_ap1[0][0] *= 1.1;
            changed.project.validate().unwrap();
            e.document.install(changed);
            assert_eq!(
                e.document
                    .source_preview(e.document.project.source(id).unwrap())
                    .transform(white),
                glam::DMat3::IDENTITY,
                "a changed development must not reuse viewing facts from another basis"
            );
            let original = Document::open(saved.path.as_deref().unwrap()).unwrap();
            e.document.install(original.clone());
            assert_eq!(
                e.document
                    .source_preview(e.document.project.source(id).unwrap())
                    .transform(white),
                expected,
                "runtime viewing facts survive metadata publications independently of snapshots"
            );
            e.document = e.document.replacement(original);
            assert_eq!(
                e.document
                    .source_preview(e.document.project.source(id).unwrap())
                    .transform(white),
                glam::DMat3::IDENTITY,
                "a new document owns its own runtime viewing facts"
            );
        });
    });
}

#[test]
fn import_acquisition_does_not_displace_another_edit_and_holds_authority_until_publication() {
    let directory = tempfile::tempdir().unwrap();
    let photo = copy_photo(directory.path(), "photo.dng");
    let mut e = authored_fixture(&[]);
    let edit = e.edit_project("Other edit").unwrap();
    let hold = crate::background::testing::pause();
    with_application(e, |app| {
        app.enter(|e, cx| e.add_raws(vec![photo.clone()], cx));
        app.read(|e| {
            assert!(e.document.importing.is_none());
            assert!(e.document.project.sources.is_empty());
        });
        drop(edit);
        app.enter(|e, cx| {
            e.add_raws(vec![photo], cx);
            assert!(e.edit_project("Competing edit").is_err());
            assert!(e.edit_document("Competing document").is_err());
        });
        app.pump();
        app.read(|e| assert!(e.document.project.sources.is_empty()));
        drop(hold);
        wait(app);
        app.read(|e| assert_eq!(e.document.project.sources.len(), 1));
    });
}
