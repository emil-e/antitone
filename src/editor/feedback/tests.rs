use super::super::presentation::tests::{key, pointer};
use super::super::tests::{add_ready_fixture, app_frame, fixture};
use super::*;

#[test]
fn save_failure_survives_status_updates_and_acknowledgement_preserves_the_pending_decision() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.sync_document();
            let before = editor.document.state.project.clone();
            editor.save_project(
                "project.json".into(),
                crate::project::Save::New,
                Some(Transition::Close),
                cx,
            );
            assert!(editor.busy && editor.document_pending.is_some());
            let id = editor.document_pending.unwrap();
            editor.save_result(id, Err("permission denied".into()), cx);
            assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Save);
            assert_eq!(editor.transition, Some(Transition::Close));
            assert_eq!(editor.document.state.project, before);
            assert!(editor.document.dirty());
            assert!(editor.document_failure.is_none());
            editor.status = "Unrelated progress or success".into();
            editor.save_result(id, Err("stale error".into()), cx);
            assert_eq!(
                editor.failure.as_ref().unwrap().details,
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
            app_frame(editor, &context, key(egui::Key::ArrowRight));
            assert!(editor.document_pending.is_none());
            app_frame(editor, &context, key(egui::Key::Enter));
            assert!(editor.failure.is_none());
            assert_eq!(editor.transition, Some(Transition::Close));
            assert!(!editor.allow_close);
            assert_eq!(editor.document.state.project, before);
            let labels = app_frame(editor, &context, vec![]);
            assert!(labels.iter().any(|(label, _)| label == "Save changes"));
            assert!(editor.document_pending.is_none());
        });
    });
}

#[test]
fn unavailable_export_services_report_failure_and_keep_accepted_state() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
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
        assert!(app.read(|e| e.document.export_operation.pending.is_some()));
        app.run_until(Duration::from_secs(5), |e| {
            e.document.export_operation.pending.is_none()
        });
        app.read(|editor| {
            assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
            assert_eq!(
                editor.failure.as_ref().unwrap().details,
                "Export services unavailable"
            );
            assert!(
                !editor.busy
                    && editor.document_pending.is_none()
                    && editor.document_failure.is_none()
            );
            assert!(editor.document.flat_edit.is_none());
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
    editor.sync_document();
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
        editor.failure.as_ref().unwrap().kind,
        FailureKind::FlatReference
    );
    assert!(!editor.busy && editor.document_pending.is_none() && editor.document_failure.is_none());
    assert!(editor.document.flat_edit.is_none() && editor.active_pick().is_none());
    assert_eq!(editor.document.visit.view, visit);
    assert_eq!(
        editor.document.visit.canvas.ready.as_ref().unwrap().path,
        path
    );
    assert!(!editor.document.source);
    assert_eq!(editor.document.state.project, before);
    editor.document_prepared(request, Err("late completion".into()));
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "Reference services unavailable"
    );
}

#[test]
fn matching_save_success_resolves_the_failure_but_starting_a_retry_does_not() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.report_failure(FailureKind::Save, "destination unavailable".into());
            editor.save_project(
                "project.json".into(),
                crate::project::Save::New,
                Some(Transition::Close),
                cx,
            );
            assert!(editor.busy && editor.document_pending.is_some());
            assert!(editor.failure.is_some());
            let id = editor.document_pending.unwrap();
            editor.save_result(id, Ok(Box::new(editor.document.capture())), cx);
            assert!(editor.failure.is_none());
            assert!(editor.allow_close);
            assert_eq!(editor.application_quit_state(), host::QuitState::Approved);
        });
    });
}

#[test]
fn stale_and_unrelated_results_do_not_retire_classified_correction_failures() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            let before = editor.document.state.project.clone();
            editor.start_document(
                Preparation::Correction(
                    editor.document.capture(),
                    editor.document.visit.current_source,
                ),
                cx,
            );
            assert!(editor.busy && editor.document_pending.is_some());
            let id = editor.document_pending.unwrap();
            editor.document_prepared(id + 1, Err("stale correction".into()));
            assert!(editor.failure.is_none() && editor.busy);
            editor.document_prepared(id, Err("unsupported GPU range".into()));
            assert_eq!(
                editor.failure.as_ref().unwrap().kind,
                FailureKind::FlatCorrection
            );
            assert_eq!(editor.document.state.project, before);
            assert!(!editor.busy);
            editor.export_result(editor.id, Ok("unrelated-success.tiff".into()));
            assert_eq!(
                editor.failure.as_ref().unwrap().details,
                "unsupported GPU range"
            );
            editor.start_document(
                Preparation::Correction(
                    editor.document.capture(),
                    editor.document.visit.current_source,
                ),
                cx,
            );
            assert!(editor.busy && editor.document_pending.is_some());
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
            assert!(editor.failure.is_none());
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn rejected_support_preserves_region_and_grade_until_matching_resolution() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let reference = editor.document.visit.canvas.ready.as_ref().unwrap().clone();
    crate::editor::tests::paused_reference_fixture(&mut editor, reference, None);
    editor.set_pick_region(Some(
        Coverage::new(
            [[20.25, 20.], [40.25, 20.], [40.25, 40.], [20.25, 40.]]
                .map(crate::geometry::SourcePoint),
        )
        .unwrap(),
    ));
    editor.sync_document();
    let before = editor.document.state.project.clone();
    let region = editor.pick_region();
    editor.apply_flat_edit(&UiContext::render_only());
    let failure = editor.failure.as_ref().unwrap();
    assert_eq!(failure.kind, FailureKind::FlatSupport);
    assert_eq!(failure.presentation().severity, Severity::Warning);
    assert_eq!(editor.pick_region(), region);
    assert_eq!(editor.document.state.project, before);
    assert!(!editor.busy && editor.document_pending.is_none());
    assert!(
        editor.document.flat_edit.as_ref().unwrap().task.is_none()
            && editor.document_pending.is_none()
    );
    editor.resolve_failure(FailureKind::Export);
    assert!(editor.failure.is_some());
    editor.resolve_failure(FailureKind::FlatCorrection);
    assert!(editor.failure.is_none());
}

#[test]
fn export_failure_is_persistent_stale_results_cannot_replace_it_and_matching_success_resolves_it() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            add_ready_fixture(editor);
            editor.sync_document();
            let before = editor.document.state.project.clone();
            editor.export(
                "output.exr".into(),
                OutputFormat::Exr(ExrPrecision::Half),
                Overwrite::Refuse,
                cx,
            );
            assert!(editor.document.export_operation.pending.is_some());
            let request = editor.document.export_operation.pending.unwrap();
            editor.export_result(request + 1, Err("stale".into()));
            assert!(editor.failure.is_none() && editor.busy);
            editor.export_result(request, Err("choose full-float EXR".into()));
            assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
            assert!(!editor.busy);
            editor.status = "Other status".into();
            editor.export_result(request + 1, Err("another stale error".into()));
            assert_eq!(
                editor.failure.as_ref().unwrap().details,
                "choose full-float EXR"
            );
            editor.export(
                "output.exr".into(),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
                cx,
            );
            assert!(editor.document.export_operation.pending.is_some());
            assert!(editor.failure.is_some());
            let pending = editor.document.export_operation.pending;
            editor.export_result(request, Ok("old-output.exr".into()));
            assert!(editor.busy && editor.failure.is_some());
            assert_eq!(editor.document.export_operation.pending, pending);
            editor.export_result(
                editor.document.export_operation.pending.unwrap(),
                Ok("output.exr".into()),
            );
            assert!(editor.failure.is_none());
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn feedback_blocks_background_file_actions_and_dismissal_only_retires_the_message() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let before = editor.document.state.project.clone();
    editor.report_failure(FailureKind::Export, "disk full".into());
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
    assert!(editor.failure.is_some());
    assert!(
        editor.document_pending.is_none() && editor.document.export_operation.pending.is_none()
    );
    let point = labels
        .iter()
        .find(|(label, _)| label == "OK")
        .unwrap()
        .1
        .center();
    for pressed in [true, false] {
        app_frame(&mut editor, &context, pointer(point, pressed));
    }
    assert!(editor.failure.is_none());
    assert_eq!(editor.document.state.project, before);
    assert!(
        editor.document_pending.is_none() && editor.document.export_operation.pending.is_none()
    );
}
