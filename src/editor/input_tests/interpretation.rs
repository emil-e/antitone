use super::*;

fn acquire(
    scope: &DocumentScope,
    id: SourceId,
    loader: &source_loading::SourceLoader,
) -> Result<crate::source::input::Acquired, Error> {
    let input = scope.source_input(id)?;
    pollster::block_on(input.source(
        loader.provider(Some(scope.cache.clone())).as_ref(),
        source_loading::Priority::Foreground,
    ))
}

#[test]
fn automatic_import_reinterpretation_uses_a_new_basis_scope_without_resetting_the_workspace() {
    let a = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
    let b = crate::raw::fixture::bayer_dng_with_camera(
        [1800, 2200, 2800],
        [1., 0., 0., 0., 0.9, 0., 0., 0., 1.1],
    );
    let mut document = Document::default();
    document.import(&[a.path().into()]).unwrap();
    let selected = document.project.order[0];
    let loader = source_loading::SourceLoader::new();
    with_application(editor_with_loader(loader.clone(), document), |app| {
        app.enter(|editor, cx| editor.load(a.path().into(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        let before = app.update(|editor| {
            editor.document.visit.canvas.viewport.zoom = Some(1.75);
            editor.document.visit.adjustment_layers.show_coverage = true;
            editor.document.capture()
        });
        assert!(before.scan_basis().is_some());
        app.enter(|editor, cx| editor.add_raws(vec![b.path().into()], cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.importing.is_none() && !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.input, InputSettings::Digital);
            assert_eq!(editor.document.project.order.len(), 2);
            assert_eq!(editor.document.visit.current_source, Some(selected));
            assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(1.75));
            assert!(editor.document.visit.adjustment_layers.show_coverage);
            assert!(editor.document.capture().scan_basis().is_none());
            assert_eq!(
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .white_balance(),
                [2., 1., 3., 1.]
            );
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Import photos")
            );
        });
        app.enter(|editor, cx| editor.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before.project);
            assert_eq!(editor.document.capture().scan_basis(), before.scan_basis());
            assert_eq!(editor.document.visit.current_source, Some(selected));
            assert!(editor.document.visit.adjustment_layers.show_coverage);
            assert_eq!(
                editor
                    .document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .white_balance(),
                crate::project::UNITY_WB
            );
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn undo_restores_the_basis_that_belongs_to_calibration_not_the_next_consumed_photo() {
    let a = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
    let b = crate::raw::fixture::bayer_dng_with_camera(
        [1800, 2200, 2800],
        [1., 0., 0., 0., 0.9, 0., 0., 0., 1.1],
    );
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document
        .import(&[a.path().into(), b.path().into()])
        .unwrap();
    let ids = document.project.order.clone();
    let first = document.project.source(ids[0]).unwrap();
    let second = document.project.source(ids[1]).unwrap();
    assert_eq!(
        first.development.camera_model,
        second.development.camera_model
    );
    assert_ne!(
        first.development.camera_to_ap1,
        second.development.camera_to_ap1
    );
    let loader = source_loading::SourceLoader::new();
    let mut editor = Editor::state();
    editor.document = document.into();
    with_application(editor, |app| {
        let digital_b = app
            .read(|editor| acquire(&editor.document, ids[1], &loader))
            .unwrap();
        app.enter(|editor, cx| editor.request_input(editor.negative_input(), cx));
        let negative_a = app
            .read(|editor| acquire(&editor.document, ids[0], &loader))
            .unwrap();
        let basis = app.read(|editor| editor.document.capture().scan_basis().unwrap());
        assert!(basis.require_source(&digital_b.record.development).is_err());
        assert!(basis.require_source(&negative_a.record.development).is_ok());
        app.update(|editor| {
            let mut edit = editor.edit_project("Calibration fixture").unwrap();
            edit.update(|project| {
                project.set_dmin(Some([0.8; 3]));
                project.set_dmax(Some([0.02; 3]));
            });
            editor.commit_project(edit).unwrap();
        });
        let calibrated = app.read(|editor| editor.document.project.clone());
        app.enter(|editor, cx| editor.request_input(InputSettings::Digital, cx));
        app.update(|editor| editor.dialogs.answer_for_test("Switch stage"));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.input_change.is_none()
        });
        app.update(|editor| editor.document.visit.current_source = Some(ids[1]));
        app.enter(|editor, cx| editor.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, calibrated);
            assert_eq!(editor.document.capture().scan_basis(), Some(basis.clone()));
            assert!(
                acquire(&editor.document, ids[1], &loader).is_err(),
                "B must not establish a replacement basis under A's restored anchors"
            );
            assert!(acquire(&editor.document, ids[0], &loader).is_ok());
            let directory = tempfile::tempdir().unwrap();
            let saved = editor
                .document
                .capture()
                .save(&directory.path().join("calibrated.antitone"), Save::New)
                .unwrap();
            let reopened: DocumentScope =
                Document::open(saved.path.as_ref().unwrap()).unwrap().into();
            assert_eq!(reopened.capture().scan_basis(), Some(basis));
            assert!(acquire(&reopened, ids[1], &loader).is_err());
            assert!(acquire(&reopened, ids[0], &loader).is_ok());
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn correction_reference_cannot_replace_an_established_lazy_negative_basis() {
    let raw = crate::raw::fixture::bayer_dng();
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document.import(&[raw.path().into()]).unwrap();
    let loader = source_loading::SourceLoader::new();
    with_application(editor_with_loader(loader.clone(), document), |app| {
        app.enter(|editor, cx| editor.load(raw.path().into(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.enter(|editor, cx| editor.request_input(editor.negative_input(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        let before = app.read(|editor| editor.document.capture());
        assert!(before.project.scan.is_none());
        assert!(before.scan_basis().is_some());
        let wrong =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/bare-light.dng");
        app.enter(|editor, cx| editor.begin_flat_edit(wrong, None, cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor
                .document
                .flat_edit()
                .is_some_and(|edit| edit.view.ready.is_some())
        });
        app.enter(|editor, cx| editor.apply_flat_edit(cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.flat_correction.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before.project);
            assert_eq!(editor.document.capture().scan_basis(), before.scan_basis());
            assert!(editor.document.project.flat_field().is_none());
            assert!(
                editor.status.contains("scanning setup"),
                "{}",
                editor.status
            );
        });
        app.update(|editor| editor.dialogs.answer_for_test("OK"));
        app.enter(|editor, cx| {
            editor.begin_flat_edit(
                raw.path().into(),
                Some(crate::raw::Rect {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 64,
                }),
                cx,
            )
        });
        app.run_until(Duration::from_secs(15), |editor| {
            editor
                .document
                .flat_edit()
                .is_some_and(|edit| edit.view.ready.is_some())
        });
        app.enter(|editor, cx| editor.apply_flat_edit(cx));
        app.run_until(Duration::from_secs(15), |editor| {
            editor.document.flat_correction.is_none()
        });
        app.read(|editor| {
            assert!(
                editor.document.project.flat_field().is_some(),
                "a compatible reference must still apply: {}",
                editor.status
            );
            assert_eq!(editor.document.capture().scan_basis(), before.scan_basis());
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Correction")
            );
        });
    });
    loader.close_and_wait().unwrap();
}

#[test]
fn proxy_only_reinterpretation_retains_source_view_transforms_without_a_photo_visit() {
    let raw = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document.import(&[raw.path().into()]).unwrap();
    let id = document.project.order[0];
    let mut editor = Editor::state();
    editor.document = document.into();
    let loader = source_loading::SourceLoader::new();
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_input(editor.negative_input(), cx));
        let physical = app.read(|editor| editor.document.source_input(id).unwrap());
        pollster::block_on(physical.proxy(
            loader.provider(None).as_ref(),
            source_loading::Priority::Visible,
        ))
        .unwrap();
        let (record, header) =
            crate::source::open_source(raw.path(), Some(crate::project::UNITY_WB)).unwrap();
        let expected = header
            .metadata
            .source_preview(record.development.decode.white_balance)
            .transform(crate::raw::SourceWhiteBalance::AsShot);
        assert_ne!(
            expected,
            glam::DMat3::IDENTITY,
            "the fixture must distinguish missing viewing metadata"
        );
        app.read(|editor| {
            assert!(editor.document.visit.current_source.is_none());
            assert!(editor.document.visit.canvas.ready.is_none());
            let record = editor.document.project.source(id).unwrap();
            assert_eq!(
                editor
                    .document
                    .source_preview(record)
                    .transform(crate::raw::SourceWhiteBalance::AsShot),
                expected
            );
            let directory = tempfile::tempdir().unwrap();
            let saved = editor
                .document
                .capture()
                .save(&directory.path().join("known.antitone"), Save::New)
                .unwrap();
            let cold: DocumentScope = Document::open(saved.path.as_ref().unwrap()).unwrap().into();
            assert!(
                cold.source_input(id)
                    .unwrap()
                    .development()
                    .unwrap()
                    .resolved()
                    .is_ok()
            );
            pollster::block_on(cold.source_input(id).unwrap().proxy(
                loader.provider(None).as_ref(),
                source_loading::Priority::Visible,
            ))
            .unwrap();
            assert_eq!(
                cold.source_preview(cold.project.source(id).unwrap())
                    .transform(crate::raw::SourceWhiteBalance::AsShot),
                expected
            );
        });
    });
    loader.close_and_wait().unwrap();
}
