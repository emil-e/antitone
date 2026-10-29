use super::*;
use crate::{
    exposure_metadata::{Entry, Metadata},
    exposure_reference::{ReferenceExposure, StartingExposure, UnavailableReference},
    processing::InputSettings,
};

fn digital(paths: &[PathBuf]) -> Document {
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document.import(paths).unwrap();
    document
}

fn reference(ev: f64) -> ReferenceExposure {
    ReferenceExposure::Matched(StartingExposure {
        ev,
        samples: 1024,
        spread: 0.,
    })
}

fn range_stack(document: &mut Document, id: crate::project::SourceId) {
    document.project.roll.exposure = 16.;
    let mut layers = crate::project::AdjustmentLayers::default();
    for exposure in [16., 16., 16., 16., 16., 16., 4.] {
        layers
            .add(
                "Range control".into(),
                crate::adjustment_layers::AdjustmentLayer::global(
                    crate::adjustment_layers::AdjustmentLayerSettings {
                        exposure,
                        ..Default::default()
                    },
                ),
            )
            .unwrap();
    }
    document.project.source_mut(id).unwrap().adjustment_layers = Some(layers);
}

#[test]
fn opening_without_saved_exposure_is_independent_of_the_previous_documents_reference() {
    let raw = crate::exposure_fixture::neutral_dng(1.3);
    let directory = tempfile::tempdir().unwrap();
    let plain = directory.path().join("plain.antitone");
    let forced = directory.path().join("forced.antitone");
    let mut document = digital(&[raw.path().to_owned()]);
    document.save(&plain, crate::project::Save::New).unwrap();
    let source = &document.project.sources[0];
    let unavailable = ReferenceExposure::Unavailable(UnavailableReference::NoUsablePreview);
    document.exposure_metadata = Arc::new(Metadata {
        entries: vec![Entry {
            source: source.id,

            reference: unavailable.clone(),
        }],
        ..Default::default()
    });
    document.save(&forced, crate::project::Save::New).unwrap();

    let open = |paths: &[PathBuf]| {
        let loader = source_loading::SourceLoader::new();
        let mut values = Vec::new();
        let mut leases = Vec::new();
        with_application(editor(loader.clone()), |app| {
            for path in paths {
                app.enter(|e, cx| e.start_document(Preparation::Open(path.clone()), cx));
                app.run_until(Duration::from_secs(15), |e| !e.busy);
                app.read(|e| {
                    let ready = e
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .expect("opened source");
                    values.push(ready.reference.as_ref().unwrap().clone());
                    leases.push(ready.developed.clone());
                });
            }
        });
        loader.close_and_wait().unwrap();
        values
    };
    let cold = open(std::slice::from_ref(&plain));
    assert!(matches!(&cold[0], ReferenceExposure::Matched(_)));
    assert!((cold[0].ev() - 1.3).abs() < 0.03);
    let warm = open(&[forced, plain]);
    assert_eq!(warm[0], unavailable);
    assert_eq!(warm[1], cold[0]);
}

#[test]
fn reopening_keeps_the_recorded_baseline_when_legacy_fingerprints_differ() {
    let raw = crate::exposure_fixture::neutral_dng(1.3);
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("recorded.antitone");
    let saved_again = directory.path().join("saved-again.antitone");
    let mut document = digital(&[raw.path().to_owned()]);
    let id = document.project.order[0];
    let baseline = reference(2.75);
    document.exposure_metadata = Arc::new(Metadata {
        entries: vec![Entry {
            source: id,
            reference: baseline.clone(),
        }],
        ..Default::default()
    });
    document.save(&project, crate::project::Save::New).unwrap();

    // Older projects carried decoder/file checks alongside the value. Keep the
    // real source compatible, but make both saved estimator fingerprints differ.
    let mut encoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&project).unwrap()).unwrap();
    let mut previous_development = document.project.source(id).unwrap().development.clone();
    previous_development.decode.backend_version = "previous developer version".into();
    encoded["exposure_metadata"]["entries"][0]["development"] =
        serde_json::to_value(previous_development).unwrap();
    encoded["exposure_metadata"]["entries"][0]["revision"] =
        serde_json::to_value(crate::source::FileRevision::read(raw.path()).unwrap()).unwrap();
    std::fs::write(&project, serde_json::to_vec(&encoded).unwrap()).unwrap();
    std::fs::File::open(raw.path())
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
        )
        .unwrap();

    let loader = source_loading::SourceLoader::new();
    with_application(editor(loader.clone()), |app| {
        app.enter(|e, cx| e.start_document(Preparation::Open(project), cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .expect("opened photo")
                    .reference
                    .as_ref(),
                Some(&baseline)
            );
            assert_eq!(e.document.state.project.roll.exposure, 0.);
            e.document
                .capture()
                .save(&saved_again, crate::project::Save::New)
                .unwrap();
        });
    });
    let restored = crate::project::Project::load(&saved_again).unwrap();
    assert_eq!(
        restored.exposure_metadata.unwrap().entries[0].reference,
        baseline
    );
    loader.close_and_wait().unwrap();
}

#[test]
fn relative_save_as_keeps_normalized_range_admission() {
    let raw = crate::exposure_fixture::neutral_dng(1.3);
    let destination = tempfile::tempdir().unwrap();
    let old = raw.path().parent().unwrap().join("original.antitone");
    let new = destination.path().join("relocated.antitone");
    let mut document = digital(&[raw.path().to_owned()]);
    let id = document.project.order[0];
    document.path = Some(old.clone());
    document.project.source_mut(id).unwrap().path = raw.path().file_name().unwrap().into();
    document.capture_source_files().unwrap();
    range_stack(&mut document, id);
    document.validate_with_device(None).unwrap();
    let scope: crate::editor::document_scope::DocumentScope = document.into();

    scope
        .exposure_slot(&scope.exposures, id)
        .set(reference(3.))
        .unwrap();
    let snapshot = scope.capture();
    assert!(
        snapshot.validate_with_device(None).is_err(),
        "known amplification makes this otherwise admitted stack unsafe"
    );
    assert!(snapshot.save(&old, crate::project::Save::New).is_err());
    assert!(snapshot.save(&new, crate::project::Save::New).is_err());
    assert!(!old.exists() && !new.exists());
}

#[test]
fn history_restoration_checks_known_offscreen_normalization_in_the_target_project() {
    let first = crate::exposure_fixture::neutral_dng(0.3);
    let second = crate::exposure_fixture::neutral_dng(1.3);
    let loader = source_loading::SourceLoader::new();
    let mut editor = editor(loader.clone());
    let mut document = digital(&[first.path().to_owned(), second.path().to_owned()]);
    let selected = document.project.order[0];
    let offscreen = document.project.order[1];
    range_stack(&mut document, offscreen);
    document.validate_with_device(None).unwrap();
    editor.document.install(document);
    let historical = editor.document.state.project.clone();

    editor
        .document
        .exposure_slot(&editor.document.exposures, offscreen)
        .set(reference(3.))
        .unwrap();
    editor
        .document
        .project
        .remove_from_filmstrip(offscreen)
        .unwrap();
    let mut target = editor.document.capture();
    assert!(target.exposure_metadata.entries.is_empty());
    target.project = historical;
    let device = &editor.source_services.as_ref().unwrap().device;
    let prepare = |cache| {
        super::super::super::document_services::candidate(
            crate::editor::DocumentRequest {
                exposures: None,
                work: Preparation::History(
                    target.clone(),
                    Some(selected),
                    crate::editor::history::Direction::Undo,
                ),
                source: None,
                correction: None,
                sources: Some(
                    editor
                        .source_services
                        .as_ref()
                        .unwrap()
                        .provider(Some(cache)),
                ),
            },
            device,
            |_, _| {},
        )
    };
    assert!(
        prepare(Arc::new(crate::source::SourceCache::new(0))).is_ok(),
        "unresolved domains remain lazily admitted"
    );
    let removed = editor.document.state.project.clone();

    editor.document.input = InputSettings::Digital;
    editor.document.roll = removed.roll;
    editor.document.visit.current_source = Some(selected);
    editor.document.history = Default::default();
    editor.record_edit(target.project.clone(), "Remove photo", None);
    with_application(editor, |app| {
        app.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(
                e.document.state.project, removed,
                "known offscreen normalization must reject the unsafe restored project"
            );
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Remove photo")
            );
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn undoing_an_import_then_adding_another_photo_does_not_reuse_the_removed_baseline() {
    let first = crate::exposure_fixture::neutral_dng(0.3);
    let removed = crate::exposure_fixture::neutral_dng(1.3);
    let replacement = crate::exposure_fixture::neutral_dng(2.3);
    let loader = source_loading::SourceLoader::new();
    let mut editor = editor(loader.clone());
    editor.document.input = InputSettings::Digital;
    editor.document.state.project.input = InputSettings::Digital;
    with_application(editor, |app| {
        for path in [first.path(), removed.path()] {
            app.enter(|e, cx| e.add_raws(vec![path.to_owned()], cx));
            app.run_until(Duration::from_secs(15), |e| !e.busy);
        }
        let removed_id = app.read(|e| e.document.state.project.order[1]);
        app.enter(|e, cx| e.load_source(removed.path().to_owned(), true, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(
                (e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .reference
                    .as_ref()
                    .unwrap()
                    .ev()
                    - 1.3)
                    .abs()
                    < 0.03
            )
        });
        app.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| assert_eq!(e.document.state.project.order.len(), 1));
        app.enter(|e, cx| e.add_raws(vec![replacement.path().to_owned()], cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(
                e.document.state.project.order[1], removed_id,
                "exercise actual numeric ID reuse"
            )
        });
        app.enter(|e, cx| e.load_source(replacement.path().to_owned(), true, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert!(
                (e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .reference
                    .as_ref()
                    .unwrap()
                    .ev()
                    - 2.3)
                    .abs()
                    < 0.03
            );
            assert!(
                e.history_label(crate::editor::history::Direction::Redo)
                    .is_none()
            );
        });
    });
    loader.close_and_wait().unwrap();
}
