use super::super::presentation::tests::pointer;
use super::super::tests::{add_ready_fixture, app_frame, fixture};
use super::*;
use crate::editor::*;

#[test]
fn save_failure_survives_status_updates_and_acknowledgement_preserves_the_pending_decision() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let before = editor.document.state.project.clone();
            editor.save_project(
                "project.json".into(),
                crate::project::Save::New,
                Some(Transition::Close),
                cx,
            );
            assert!(editor.interaction_blocked() && editor.document_pending.is_some());
            let id = editor.document_pending.unwrap();
            editor.save_result(id, Err("permission denied".into()), cx);
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::Save.title()
            );
            assert!(editor.files.closing());
            assert_eq!(editor.document.state.project, before);
            assert!(editor.document.dirty());
            editor.status = "Unrelated progress or success".into();
            editor.save_result(id, Err("stale error".into()), cx);
            assert_eq!(
                editor.dialogs.message().unwrap().details,
                "permission denied"
            );
            let context = egui::Context::default();
            app_frame(editor, &context, vec![]);
            let labels = app_frame(editor, &context, vec![]);
            assert!(
                labels
                    .iter()
                    .any(|(label, _)| label == "Could not save project")
            );
            assert!(!labels.iter().any(|(label, _)| label == "Save changes"));
            editor.dialogs.answer_for_test("OK");
            assert!(!editor.allow_close);
            assert_eq!(editor.document.state.project, before);
        });
        app.pump();
        app.update(|editor| {
            assert!(editor.dialogs.is_open() && editor.files.closing());
            editor.dialogs.answer_for_test("Cancel");
        });
        app.pump();
        app.read(|editor| assert!(!editor.allow_close && !editor.files.pending()));
    });
}

#[test]
fn unavailable_export_services_report_failure_and_keep_accepted_state() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let before = editor.document.state.project.clone();
    let source = editor.document.visit.current_source;
    let view = (
        editor.document.visit.canvas.viewport,
        editor.document.source,
    );
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|e, cx| {
            e.export(
                "output.exr".into(),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
                cx,
            )
        });
        assert!(app.read(|e| e.document.export.operation.is_some()));
        app.run_until(Duration::from_secs(5), |e| {
            e.document.export.operation.is_none()
        });
        app.read(|editor| {
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::Export.title()
            );
            assert_eq!(
                editor.dialogs.message().unwrap().details,
                "Export services unavailable"
            );
            assert!(!editor.interaction_blocked() && editor.document_pending.is_none());
            assert!(editor.document.flat_edit().is_none());
            assert_eq!(editor.document.state.project, before);
            assert_eq!(editor.document.visit.current_source, source);
            assert_eq!(editor.document.source, view.1);
            assert_eq!(editor.document.visit.canvas.viewport.zoom, view.0.zoom);
        });
    });
}

#[test]
fn unavailable_reference_preparation_restores_working_context_and_preserves_failure() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.document.source = false;
    let before = editor.document.state.project.clone();
    let visit = editor.document.visit.view;
    let path = editor
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .path
        .clone();
    let request = editor.id;
    let mut editor = crate::editor::tests::complete_reference_operation(
        editor,
        Duration::from_secs(5),
        |e, cx| e.begin_flat_edit("missing-reference.ARW".into(), None, cx),
    );
    assert_eq!(
        editor.dialogs.message().unwrap().title,
        FailureCase::FlatReference.title()
    );
    assert!(!editor.interaction_blocked() && editor.document_pending.is_none());
    assert!(editor.document.flat_edit().is_none() && editor.test_pick().is_none());
    assert_eq!(editor.document.visit.view, visit);
    assert_eq!(
        editor.document.visit.canvas.ready.as_ref().unwrap().path,
        path
    );
    assert!(!editor.document.source);
    assert_eq!(editor.document.state.project, before);
    editor.document_prepared(request, Err("late completion".into()));
    assert_eq!(
        editor.dialogs.message().unwrap().details,
        "Reference services unavailable"
    );
}

#[test]
fn successful_save_keeps_the_unread_notice_and_waits_before_closing() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.report_failure(FailureCase::Save, "destination unavailable".into());
            editor.save_project(
                "project.json".into(),
                crate::project::Save::New,
                Some(Transition::Close),
                cx,
            );
            assert!(editor.interaction_blocked() && editor.document_pending.is_some());
            assert!(editor.dialogs.is_open());
            let id = editor.document_pending.unwrap();
            editor.save_result(id, Ok(Box::new(editor.document.capture())), cx);
            assert!(editor.dialogs.is_open());
            assert!(!editor.allow_close);
        });
        app.pump();
        app.read(|editor| assert!(!editor.allow_close && editor.dialogs.is_open()));
        app.update(|editor| editor.dialogs.answer_for_test("OK"));
        app.pump();
        app.read(|editor| {
            assert!(
                editor.allow_close && editor.application_quit_state() == host::QuitState::Approved
            )
        });
    });
}

#[test]
fn stale_results_and_successful_correction_do_not_dismiss_an_unread_failure() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let before = editor.document.state.project.clone();
            editor.start_correction(editor.document.capture(), cx);
            assert!(editor.interaction_blocked() && editor.document_pending.is_some());
            let id = editor.document_pending.unwrap();
            editor.document_prepared(id + 1, Err("stale correction".into()));
            assert!(!editor.dialogs.is_open() && editor.interaction_blocked());
            editor.document_prepared(id, Err("unsupported GPU range".into()));
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::FlatCorrection.title()
            );
            assert_eq!(editor.document.state.project, before);
            assert!(editor.document_pending.is_none());
            editor.clear_look();
            assert_eq!(
                editor.dialogs.message().unwrap().details,
                "unsupported GPU range"
            );
            editor.start_correction(editor.document.capture(), cx);
            assert!(editor.interaction_blocked() && editor.document_pending.is_some());
            let id = editor.document_pending.unwrap();
            editor.document_prepared(
                id,
                Ok(Box::new(Prepared {
                    document: editor.document.capture(),
                    ready: editor.document.visit.canvas.ready.clone(),
                    selected: editor.document.visit.current_source,
                    reuse_ready: true,
                    correction: None,
                    preserve_view: true,
                })),
            );
            assert!(editor.dialogs.is_open());
            assert_eq!(editor.document.state.project, before);
            editor.dialogs.answer_for_test("OK");
            assert!(!editor.dialogs.is_open());
        });
    });
}

#[test]
fn rejected_support_and_accepted_state_remain_visible_until_acknowledgement() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let reference = editor.document.visit.canvas.ready.as_ref().unwrap().clone();
    crate::editor::tests::paused_reference_fixture(&mut editor, reference, None);
    editor.set_test_pick_region(Some(
        Coverage::new(
            [[20.25, 20.], [40.25, 20.], [40.25, 40.], [20.25, 40.]]
                .map(crate::geometry::SourcePoint),
        )
        .unwrap(),
    ));
    let before = editor.document.state.project.clone();
    let region = editor.test_pick_region();
    editor.apply_flat_edit(&UiContext::render_only());
    let failure = editor.dialogs.message().unwrap();
    assert_eq!(failure.title, FailureCase::FlatSupport.title());
    assert_eq!(failure.presentation().severity, Severity::Warning);
    assert_eq!(editor.test_pick_region(), region);
    assert_eq!(editor.document.state.project, before);
    assert!(!editor.interaction_blocked() && editor.document_pending.is_none());
    assert!(
        editor.document.flat_edit().unwrap().task.is_none() && editor.document_pending.is_none()
    );
    editor.clear_look();
    assert!(editor.dialogs.is_open());
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.start_correction(editor.document.capture(), cx);
            assert!(
                editor.dialogs.is_open(),
                "starting a retry must retain the error"
            );
            let id = editor.document_pending.unwrap();
            editor.document_prepared(
                id,
                Ok(Box::new(Prepared {
                    document: editor.document.capture(),
                    ready: editor.document.visit.canvas.ready.clone(),
                    selected: editor.document.visit.current_source,
                    reuse_ready: true,
                    correction: None,
                    preserve_view: true,
                })),
            );
            assert!(editor.dialogs.is_open());
            assert_eq!(editor.document.project, before);
            editor.dialogs.answer_for_test("OK");
            assert!(!editor.dialogs.is_open());
        });
    });
}

#[test]
fn feedback_blocks_background_file_actions_and_dismissal_only_retires_the_message() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let before = editor.document.state.project.clone();
    assert!(editor.document.dirty());
    editor.report_failure(FailureCase::Export, "disk full".into());
    let context = egui::Context::default();
    app_frame(&mut editor, &context, vec![]);
    let labels = app_frame(&mut editor, &context, vec![]);
    app_frame(
        &mut editor,
        &context,
        vec![egui::Event::Key {
            key: egui::Key::N,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    for pressed in [true, false] {
        app_frame(
            &mut editor,
            &context,
            pointer(egui::pos2(30., 15.), pressed),
        );
    }
    assert!(editor.dialogs.is_open());
    assert!(!editor.files.pending());
    assert!(editor.document_pending.is_none() && editor.document.export.operation.is_none());
    let point = labels
        .iter()
        .find(|(label, _)| label == "OK")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        app_frame(&mut editor, &context, pointer(point, pressed));
    }
    assert!(!editor.dialogs.is_open());
    assert_eq!(editor.document.state.project, before);
    assert!(editor.document_pending.is_none() && editor.document.export.operation.is_none());
    assert!(!editor.files.pending());
    // The same command is a meaningful positive control once acknowledgement removes the blocker.
    crate::application::test_support::with_application(editor, |app| {
        app.update(|editor| {
            app_frame(
                editor,
                &context,
                vec![egui::Event::Key {
                    key: egui::Key::N,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        });
        app.enter(|editor, cx| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1100., 800.),
                    )),
                    events: vec![egui::Event::Key {
                        key: egui::Key::N,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    }],
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| host::App::ui(editor, ui, cx));
                },
            );
            output.textures_delta.clear();
        });
        app.pump();
        app.read(|editor| {
            assert!(editor.files.pending() && editor.dialogs.is_open());
            assert_eq!(editor.document.project, before);
        });
    });
}
