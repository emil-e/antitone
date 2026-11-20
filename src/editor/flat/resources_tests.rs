use super::*;
use crate::editor::export::{Draft, Kind, Scope};
use futures_channel::oneshot;
use futures_lite::future::{or, poll_once};

fn fixture() -> Editor {
    let mut editor = crate::editor::tests::authored_fixture(&["roll-0.dng", "roll-1.dng"]);
    editor.set_dmin(Some(DVec3::splat(0.8)));
    editor.set_dmax(Some(DVec3::splat(0.02)));
    let id = editor.document.project.order[0];
    let source = editor.document.project.source(id).unwrap();
    let map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            source.development.active_area,
            [2, 2],
            vec![DVec3::new(1.1, 0.9, 1.05); 4],
        )
        .unwrap(),
    );
    let settings = FlatFieldSettings::new(source.path.clone(), map);
    editor
        .document
        .project
        .set_flat_field(Some(settings))
        .unwrap();
    for id in editor.document.project.order.clone() {
        let source = editor.document.project.source_mut(id).unwrap();
        source.crop = Some(crate::crop::Crop {
            center: [200., 200.],
            size: [16, 16],
            angle: 0.,
        });
    }
    editor
}

fn hold_preparation(
    editor: &mut Editor,
) -> (Resources, oneshot::Receiver<()>, oneshot::Sender<()>) {
    let preparation = Resources::prepare(
        editor.document.project.active_flat_field().cloned(),
        editor.document.visit.working_view.device().cloned(),
    );
    let (started, observed) = oneshot::channel();
    let (release, resume) = oneshot::channel();
    let resources = preparation.after(async move {
        let _ = started.send(());
        resume.await.expect("release correction preparation");
    });
    editor.document.correction = resources.clone().into();
    (resources, observed, release)
}

fn draft(folder: &std::path::Path) -> Draft {
    let mut draft = Draft::new(OutputTarget::DisplayP3, folder.to_owned());
    draft.scope = Scope::All;
    draft.kind = Kind::Exr;
    draft
}

#[test]
fn export_and_source_acquisition_await_the_captured_correction_without_live_ui() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let (resources, _started, release) = hold_preparation(&mut editor);
    let request = editor.capture_export(&draft(directory.path())).unwrap();
    let mut source = Box::pin(editor.capture_source_context().unwrap());
    let services = editor.output_services.take().unwrap();
    let settings = editor.document.project.active_flat_field().unwrap().clone();
    // Readers retain their captured resource dependency, not the live editor or a UI waiter.
    drop(editor);
    pollster::block_on(async {
        assert!(poll_once(&mut source).await.is_none());
        let mut export =
            Box::pin(services.export_batch(request, Cancellation::default(), |_, _| {}));
        assert!(poll_once(&mut export).await.is_none());
        release.send(()).unwrap();
        let summary = export.await;
        assert_eq!((summary.exported(), summary.failed()), (2, 0));
        let context = source.await.unwrap();
        let correction = resources.acquire().await.unwrap().unwrap();
        assert!(Arc::ptr_eq(
            context.correction.prepared().unwrap(),
            &correction
        ));
        assert_eq!(context.scan().unwrap().flat_field(), Some(&settings.map));
    });
    for name in ["roll-0.exr", "roll-1.exr"] {
        let image =
            exr::prelude::read_first_flat_layer_from_file(directory.path().join(name)).unwrap();
        assert_eq!([image.layer_data.size.0, image.layer_data.size.1], [16, 16]);
    }
}

#[test]
fn cancelling_export_while_it_awaits_correction_preserves_another_consumers_dependency() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let (resources, started, release) = hold_preparation(&mut editor);
    let request = editor.capture_export(&draft(directory.path())).unwrap();
    let source = editor.capture_source_context().unwrap();
    let services = editor.output_services.take().unwrap();
    drop(editor);
    pollster::block_on(async {
        let cancelled = Cancellation::default();
        let mut export = Box::pin(services.export_batch(request, cancelled.clone(), |_, _| {}));
        or(
            async {
                let _ = export.as_mut().await;
                panic!("export finished before correction preparation was released");
            },
            async { started.await.unwrap() },
        )
        .await;
        cancelled.cancel();
        let summary = poll_once(&mut export)
            .await
            .expect("cancellation must not wait for the held correction");
        assert!(summary.stopped);
        assert!(summary.outcomes.is_empty());
        assert!(resources.pending());
        release.send(()).unwrap();
        let context = source.await.unwrap();
        let correction = resources.acquire().await.unwrap().unwrap();
        assert!(Arc::ptr_eq(
            context.correction.prepared().unwrap(),
            &correction
        ));
    });
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn undo_restores_values_while_an_independent_reader_keeps_departed_preparation() {
    use crate::application::test_support::with_application;
    use crate::editor::history::Direction;
    use std::time::Duration;

    let mut editor = fixture();
    let before = editor.document.project.clone();
    let (resources, _started, release) = hold_preparation(&mut editor);
    let mut source = Box::pin(editor.capture_source_context().unwrap());
    assert!(pollster::block_on(poll_once(&mut source)).is_none());
    let mut changed = editor.document.capture();
    let settings = changed.project.flat_field_mut().unwrap();
    settings.map = Arc::new(
        crate::flat_field::FlatField::from_nodes(
            settings.map.area(),
            [2, 2],
            vec![DVec3::splat(1.2); 4],
        )
        .unwrap(),
    );
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.start_correction(changed, cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.interaction_blocked()
        });
        app.read(|editor| {
            assert_ne!(editor.document.project, before);
            assert!(editor.correction().prepared().is_some());
        });
        assert!(resources.pending());
        assert!(pollster::block_on(poll_once(&mut source)).is_none());
        app.enter(|editor, cx| editor.restore_history(Direction::Undo, cx));
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.history_label(Direction::Redo), Some("Correction"));
            assert!(!editor.interaction_blocked());
        });
        assert!(
            resources.pending(),
            "history does not need the departed reader's map"
        );
        release.send(()).unwrap();
        let context = pollster::block_on(source).unwrap();
        let prepared = pollster::block_on(resources.acquire()).unwrap().unwrap();
        assert!(Arc::ptr_eq(
            context.correction.prepared().unwrap(),
            &prepared
        ));
        assert_eq!(
            context.scan().unwrap().flat_field(),
            before.active_flat_field().map(|settings| &settings.map)
        );
        app.read(|editor| assert_eq!(editor.document.project, before));
    });
}

#[test]
fn import_does_not_await_or_cancel_the_live_correction_observer() {
    use crate::application::test_support::with_application;
    use std::time::Duration;
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let added = directory.path().join("added.dng");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/roll-0.dng"),
        &added,
    )
    .unwrap();
    let (resources, _started, release) = hold_preparation(&mut editor);
    with_application(editor, |app| {
        app.enter(|editor, cx| {
            let _ = editor.correction();
            editor.document.correction.observe(cx);
            editor.add_raws(vec![added], cx);
        });
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.importing.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.sources.len(), 3);
            assert!(editor.correction().pending());
            assert!(resources.pending());
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.correction().pending()
        });
        let prepared = pollster::block_on(resources.acquire()).unwrap().unwrap();
        app.read(|editor| {
            assert!(Arc::ptr_eq(
                editor.correction().prepared().unwrap(),
                &prepared,
            ))
        });
    });
}
