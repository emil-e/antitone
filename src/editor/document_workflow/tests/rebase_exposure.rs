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
            development: source.development.clone(),
            revision: document.file_key(raw.path()).revision.unwrap(),
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
                    let ready = e.document.visit.ready.as_ref().expect("opened source");
                    values.push(ready.developed.reference_exposure.get().unwrap().clone());
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
    let key = scope.source_key(scope.project.source(id).unwrap()).unwrap();
    scope.cache.exposure_memo(&key).set(reference(3.)).unwrap();
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
    let historical = editor.document.project.clone();
    let key = editor
        .document
        .source_key(editor.document.project.source(offscreen).unwrap())
        .unwrap();
    editor
        .document
        .cache
        .exposure_memo(&key)
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
                work: Preparation::History(
                    target.clone(),
                    Some(selected),
                    crate::editor::history::Direction::Undo,
                ),
                source: None,
                correction: None,
                cache: Some(cache),
            },
            device,
            |_, _| {},
        )
    };
    assert!(
        prepare(Arc::new(crate::source::SourceCache::new(0))).is_ok(),
        "unresolved domains remain lazily admitted"
    );
    let removed = editor.document.project.clone();

    editor.input = InputSettings::Digital;
    editor.roll = removed.roll;
    editor.current_source = Some(selected);
    editor.document.history = Default::default();
    editor.record_edit(target.project.clone(), "Remove photo", None);
    with_application(editor, |app| {
        app.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |e| !e.busy);
        app.read(|e| {
            assert_eq!(
                e.document.project, removed,
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
