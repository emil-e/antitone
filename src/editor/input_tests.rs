use super::*;

mod interpretation;
use crate::{
    application::test_support::with_application,
    project::{CameraBasis, FlatFieldSettings, Save, ScanBasis, ScanSetup},
};
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng")
}

#[test]
fn missing_photos_do_not_gate_input_intent_save_or_undo_without_services_or_gpu() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["first.dng", "second.dng"]
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(fixture(), &path).unwrap();
            path
        })
        .into();
    let mut document = Document::default();
    document.import(&paths).unwrap();
    document.project.roll.exposure = 0.375;
    let selected = document.project.order[1];
    let source = document.project.source_mut(selected).unwrap();
    let [x, y, width, height] = source.development.active_area;
    source.crop = Some(crate::crop::Crop {
        center: [f64::from(x + width / 2), f64::from(y + height / 2)],
        size: [16, 16],
        angle: 0.,
    });
    document = document
        .save(&directory.path().join("before.antitone"), Save::New)
        .unwrap();
    let before = document.project.clone();
    for path in &paths {
        std::fs::remove_file(path).unwrap();
    }
    let mut editor = Editor::state();
    editor.document = document.into();
    editor.document.visit.current_source = Some(selected);
    editor.document.visit.canvas.viewport.zoom = Some(2.25);
    assert!(editor.source_services.is_none());
    assert!(editor.document.visit.working_view.device().is_none());
    with_application(editor, |app| {
        app.enter(|editor, cx| editor.request_input(InputSettings::Digital, cx));
        app.read(|editor| {
            assert_eq!(editor.document.project.input, InputSettings::Digital);
            assert_eq!(editor.document.project.sources, before.sources);
            assert_eq!(editor.document.project.roll, before.roll);
            assert_eq!(editor.document.visit.current_source, Some(selected));
            assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(2.25));
            assert!(editor.document.project.scan.is_none());
            assert!(editor.document_pending.is_none());
            assert!(!editor.editing());
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Input stage")
            );
            let saved = editor
                .document
                .capture()
                .save(&directory.path().join("intent.antitone"), Save::New)
                .unwrap();
            let opened = Document::open(saved.path.as_ref().unwrap()).unwrap();
            assert_eq!(opened.project, saved.project);
            assert!(!opened.dirty());
            assert!(
                opened
                    .image_recipe(selected)
                    .unwrap()
                    .neutral
                    .source_development
                    .resolved()
                    .is_err()
            );
        });
        app.run_until(Duration::from_secs(5), |editor| {
            !editor.document.visit.source.pending()
        });
        app.enter(|editor, cx| editor.restore_history(history::Direction::Undo, cx));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document_pending.is_none() && !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert_eq!(editor.document.visit.current_source, Some(selected));
            assert_eq!(editor.document.visit.canvas.viewport.zoom, Some(2.25));
            assert!(!editor.document.dirty());
            assert!(editor.history_label(history::Direction::Undo).is_none());
        });
        app.enter(|editor, cx| editor.restore_history(history::Direction::Redo, cx));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document_pending.is_none() && !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.input, InputSettings::Digital);
            assert_eq!(editor.document.project.sources, before.sources);
            assert_eq!(editor.document.visit.current_source, Some(selected));
        });
    });
}

fn map_only_editor() -> Editor {
    let mut editor = Editor::state();
    editor.document.project.scan = Some(ScanSetup {
        basis: ScanBasis {
            camera: CameraBasis {
                camera_model: "map-only fixture".into(),
                camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
            },
        },
        flat_field: Some(FlatFieldSettings::new(
            "reference.dng",
            Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [0, 0, 16, 16],
                    [2, 2],
                    vec![DVec3::splat(1.1); 4],
                )
                .unwrap(),
            ),
        )),
    });
    assert!(!editor.document.project.input.has_stage_settings());
    editor
}

#[test]
fn map_only_switch_owns_one_edit_across_confirmation_and_cancel_is_lossless() {
    with_application(map_only_editor(), |app| {
        let before = app.read(|editor| editor.document.project.clone());
        app.enter(|editor, cx| editor.request_input(InputSettings::Digital, cx));
        app.read(|editor| {
            assert!(editor.dialogs.is_open());
            assert_eq!(editor.document.project, before);
            assert!(editor.editing());
            assert!(editor.history_label(history::Direction::Undo).is_none());
        });
        app.update(|editor| {
            assert!(editor.edit_document("Competing edit").is_err());
            editor.dialogs.answer_for_test("Cancel");
        });
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.input_change.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project, before);
            assert!(!editor.editing());
            assert!(editor.history_label(history::Direction::Undo).is_none());
        });
        app.enter(|editor, cx| editor.request_input(InputSettings::Digital, cx));
        app.update(|editor| editor.dialogs.answer_for_test("Switch stage"));
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.input_change.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.document.project.input, InputSettings::Digital);
            assert!(editor.document.project.flat_field().is_none());
            assert!(editor.document_pending.is_none());
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Input stage")
            );
        });
    });
}

#[test]
fn replacing_the_document_retires_the_input_continuation_but_not_its_unanswered_dialog() {
    with_application(map_only_editor(), |app| {
        app.enter(|editor, cx| editor.request_input(InputSettings::Digital, cx));
        app.update(|editor| editor.new_project());
        let replacement = app.read(|editor| editor.document.project.clone());
        app.update(|editor| editor.dialogs.answer_for_test("Switch stage"));
        app.pump();
        app.read(|editor| {
            assert_eq!(editor.document.project, replacement);
            assert!(editor.document.project.input.is_negative());
            assert!(!editor.editing());
            assert!(editor.history_label(history::Direction::Undo).is_none());
        });
    });
}

#[test]
fn acquisition_shares_lazy_wb_and_persists_it_without_dirtying_or_redefaulting() {
    let raw = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
    let changed = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [4, 1, 2]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("photo.dng");
    std::fs::copy(raw.path(), &path).unwrap();
    let mut document = Document::default();
    document.import(std::slice::from_ref(&path)).unwrap();
    document.project.input = InputSettings::Digital;
    document.project.scan = None;
    document = document
        .save(&directory.path().join("intent.antitone"), Save::New)
        .unwrap();
    let mut scope: DocumentScope = document.into();
    let id = scope.project.order[0];
    let before = scope.project.clone();
    let first = scope.source_input(id).unwrap();
    let second = scope.source_input(id).unwrap();
    assert!(first.development().unwrap().resolved().is_err());
    assert!(second.development().unwrap().resolved().is_err());
    let starts = Arc::new(AtomicUsize::new(0));
    let observed = starts.clone();
    let loader = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            observed.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    });
    let provider = loader.provider(Some(scope.cache.clone()));
    let acquired =
        pollster::block_on(first.source(provider.as_ref(), source_loading::Priority::Foreground))
            .unwrap();
    let scan = acquired.prepared.scan();
    assert_eq!(scan.white_balance, [2., 1., 3., 1.]);
    assert_eq!(
        second
            .development()
            .unwrap()
            .resolved()
            .unwrap()
            .decode
            .white_balance,
        scan.white_balance
    );
    let proxy =
        pollster::block_on(second.proxy(provider.as_ref(), source_loading::Priority::Visible))
            .unwrap();
    assert_eq!(proxy.matrix, scan.camera_to_ap1);
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "the live source lease must satisfy proxy demand"
    );
    let point = [
        scan.area.x + scan.area.width / 2,
        scan.area.y + scan.area.height / 2,
    ];
    let pixel = scan.pixel(point[0], point[1]).unwrap();
    assert!(pixel.min_element() > 0.);
    scope.state.capture_initialization(&scope.initialization);
    assert_eq!(scope.project, before);
    assert!(
        !scope.dirty(),
        "recording initialization is not a user edit"
    );
    std::fs::remove_file(&path).unwrap();
    let saved = scope
        .capture()
        .save(&directory.path().join("resolved.antitone"), Save::New)
        .unwrap();
    let reopened = Document::open(saved.path.as_ref().unwrap()).unwrap();
    assert!(!reopened.dirty());
    assert_eq!(
        reopened
            .image_recipe(id)
            .unwrap()
            .neutral
            .source_development
            .resolved()
            .unwrap()
            .decode
            .white_balance,
        [2., 1., 3., 1.]
    );
    std::fs::copy(changed.path(), &path).unwrap();
    let changed_default = Scan::open(&path, None).unwrap();
    assert_ne!(
        changed_default.pixel(point[0], point[1]).unwrap(),
        pixel,
        "the replacement fixture must expose as-shot WB drift"
    );
    let reopened: DocumentScope = reopened.into();
    let cold = source_loading::SourceLoader::new();
    let physical = reopened.source_input(id).unwrap();
    let acquired = pollster::block_on(physical.source(
        cold.provider(None).as_ref(),
        source_loading::Priority::Foreground,
    ))
    .unwrap();
    assert_eq!(acquired.prepared.scan().white_balance, [2., 1., 3., 1.]);
    assert_eq!(
        acquired.prepared.scan().pixel(point[0], point[1]).unwrap(),
        pixel
    );
    cold.close_and_wait().unwrap();
    loader.close_and_wait().unwrap();
}

fn editor_with_loader(loader: source_loading::SourceLoader, document: Document) -> Editor {
    let (device, queue, _) = crate::gpu::test_support::render_context();
    let mut editor = Editor::new(&device, &queue);
    editor.source_services.as_mut().unwrap().developments = loader.clone();
    editor.developments = Some(loader);
    editor.document = editor.document.replacement(document);
    assert!(editor.document.visit.working_view.device().is_some());
    editor
}

#[test]
fn input_accepts_while_old_native_work_is_held_and_only_the_new_wb_can_present() {
    use std::sync::{Mutex, atomic::AtomicBool, mpsc};
    let raw = crate::raw::fixture::bayer_dng_with_as_shot([1800, 2200, 2800], [2, 1, 3]);
    let mut document = Document::default();
    document.import(&[raw.path().into()]).unwrap();
    let (entered, starts) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let resume = Mutex::new(resume);
    let visits = Arc::new(AtomicUsize::new(0));
    let observed = visits.clone();
    let held = Arc::new(AtomicBool::new(false));
    let in_native = held.clone();
    let loader = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin)
            && observed.fetch_add(1, Ordering::SeqCst) == 0
        {
            in_native.store(true, Ordering::SeqCst);
            entered.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(15))
                .unwrap();
            in_native.store(false, Ordering::SeqCst);
        }
        Ok(())
    });
    with_application(editor_with_loader(loader.clone(), document), |app| {
        app.enter(|editor, cx| editor.load(raw.path().into(), cx));
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "native input did not reach its barrier"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        let old_view = app.read(|editor| editor.document.visit.view);
        app.enter(|editor, cx| editor.request_input(InputSettings::Digital, cx));
        let accepted_while_held = app.read(|editor| {
            held.load(Ordering::SeqCst)
                && editor.document.project.input == InputSettings::Digital
                && editor.document_pending.is_none()
                && editor.document.visit.view != old_view
                && editor.history_label(history::Direction::Undo) == Some("Input stage")
        });
        release.send(()).unwrap();
        assert!(
            accepted_while_held,
            "intent publication must not wait for old RAW cleanup"
        );
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            let ready = editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .expect("new interpretation");
            assert_eq!(ready.developed.white_balance(), [2., 1., 3., 1.]);
            assert_eq!(
                ready.record.development_mode,
                crate::project::DevelopmentMode::AsShot
            );
            assert_eq!(editor.document.project.input, InputSettings::Digital);
        });
    });
    loader.close_and_wait().unwrap();
    assert!(!held.load(Ordering::SeqCst));
    assert_eq!(
        visits.load(Ordering::SeqCst),
        2,
        "distinct WB inputs require distinct native development"
    );
}

#[test]
fn missing_corrupt_and_incompatible_photos_fail_locally_after_a_successful_switch() {
    let good = crate::raw::fixture::bayer_dng();
    let different = crate::raw::fixture::bayer_dng_with_camera(
        [1800, 2200, 2800],
        [1., 0., 0., 0., 0.9, 0., 0., 0., 1.1],
    );
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["good.dng", "missing.dng", "corrupt.dng", "incompatible.dng"]
        .map(|name| directory.path().join(name))
        .into();
    for path in &paths[..3] {
        std::fs::copy(good.path(), path).unwrap();
    }
    std::fs::copy(different.path(), &paths[3]).unwrap();
    let mut document = Document::default();
    document.project.input = InputSettings::Digital;
    document.import(&paths).unwrap();
    let accepted_sources = document.project.sources.clone();
    std::fs::remove_file(&paths[1]).unwrap();
    std::fs::write(&paths[2], b"not a RAW file").unwrap();
    let loader = source_loading::SourceLoader::new();
    with_application(editor_with_loader(loader.clone(), document), |app| {
        app.enter(|editor, cx| editor.request_input(editor.negative_input(), cx));
        app.read(|editor| {
            assert!(editor.document.project.input.is_negative());
            assert_eq!(editor.document.project.sources, accepted_sources);
            assert!(editor.document_pending.is_none());
        });
        app.enter(|editor, cx| editor.load(paths[0].clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        let pixels = app.read(|editor| {
            editor
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .developed
                .clone()
        });
        for path in &paths[1..] {
            app.enter(|editor, cx| editor.load(path.clone(), cx));
            app.run_until(Duration::from_secs(15), |editor| {
                !editor.document.visit.source.pending()
            });
            app.read(|editor| {
                assert!(
                    editor.document.visit.canvas.ready.is_none(),
                    "{}",
                    path.display()
                );
                assert!(
                    editor
                        .status
                        .contains(path.file_name().unwrap().to_str().unwrap()),
                    "{}",
                    editor.status
                );
                assert!(editor.document.project.input.is_negative());
                assert_eq!(editor.document.project.sources, accepted_sources);
                assert_eq!(
                    editor.document.visit.current_source,
                    editor.document.find_source(path)
                );
            });
        }
        app.enter(|editor, cx| editor.load(paths[0].clone(), cx));
        app.run_until(Duration::from_secs(15), |editor| {
            !editor.document.visit.source.pending()
        });
        app.read(|editor| {
            let ready = editor.document.visit.canvas.ready.as_ref().unwrap();
            assert!(
                Arc::ptr_eq(&pixels, &ready.developed),
                "compatible live cache inputs remain reusable after other photos fail"
            );
            assert_eq!(ready.developed.white_balance(), crate::project::UNITY_WB);
            assert_eq!(
                editor.history_label(history::Direction::Undo),
                Some("Input stage")
            );
        });
    });
    loader.close_and_wait().unwrap();
}
