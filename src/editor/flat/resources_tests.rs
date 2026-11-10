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
        editor.document.capture(),
        editor.document.visit.working_view.device().cloned(),
    );
    let (started, observed) = oneshot::channel();
    let (release, resume) = oneshot::channel();
    let resources = Resources::Preparing(
        async move {
            let _ = started.send(());
            resume.await.expect("release correction preparation");
            preparation.acquire().await
        }
        .boxed()
        .shared(),
    );
    editor.document.correction = resources.clone();
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
    let services = editor.export_services.take().unwrap();
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
            context.correction.as_ref().unwrap(),
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
    let services = editor.export_services.take().unwrap();
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
            context.correction.as_ref().unwrap(),
            &correction
        ));
    });
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn import_does_not_await_or_cancel_the_live_correction_observer() {
    use crate::application::test_support::with_application;
    use std::{cell::Cell, rc::Rc, time::Duration};
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let added = directory.path().join("added.dng");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/roll-0.dng"),
        &added,
    )
    .unwrap();
    let (resources, _started, release) = hold_preparation(&mut editor);
    let completed = Rc::new(Cell::new(false));
    with_application(editor, |app| {
        let observed = completed.clone();
        app.enter(|editor, cx| {
            // A live presentation waiter on the real, gated correction dependency.
            // Its marker isolates continuation lifetime from rendering behaviour.
            editor.document.correction_restore = cx.spawn(move |cx| async move {
                let correction = resources.acquire().await.unwrap().unwrap();
                cx.update(|editor| {
                    assert_eq!(
                        editor.document.project.active_flat_field().unwrap().map,
                        correction.settings.map
                    );
                    editor.document.correction_restore = None;
                    observed.set(true);
                });
            });
            editor.add_raws(vec![added], cx);
        });
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.importing.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.sources.len(), 3);
            assert!(editor.document.correction.pending());
            assert!(editor.document.correction_restore.is_some());
            assert!(!completed.get());
        });
        release.send(()).unwrap();
        app.run_until(Duration::from_secs(15), |_| completed.get());
        app.read(|editor| assert!(editor.document.correction.prepared().is_some()));
    });
}
