use super::super::presentation::tests::{key, pointer};
use super::super::tests::{add_ready_fixture, app_frame, fixture};
use super::*;

#[test]
fn save_failure_survives_status_updates_and_acknowledgement_preserves_the_pending_decision() {
    let (mut editor, send, receiver) = fixture();
    editor.sync_document();
    let before = editor.document.project.clone();
    editor.queue_save(
        "project.json".into(),
        crate::project::Save::New,
        Some(Transition::Close),
    );
    receiver.recv().unwrap();
    let id = editor.document_pending.unwrap();
    send.send(Message::Saved(id, Err("permission denied".into())))
        .unwrap();
    editor.receive();
    assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Save);
    assert_eq!(editor.transition, Some(Transition::Close));
    assert_eq!(editor.document.project, before);
    assert!(editor.document.dirty());
    assert!(editor.document_failure.is_none());
    editor.status = "Unrelated progress or success".into();
    send.send(Message::Saved(id, Err("stale error".into())))
        .unwrap();
    editor.receive();
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "permission denied"
    );
    let context = egui::Context::default();
    app_frame(&mut editor, &context, vec![]);
    let labels = app_frame(&mut editor, &context, vec![]);
    assert!(
        labels
            .iter()
            .any(|(label, _)| label == "Could not save project")
    );
    assert!(!labels.iter().any(|(label, _)| label == "Save changes"));
    app_frame(&mut editor, &context, key(egui::Key::ArrowRight));
    assert!(receiver.try_recv().is_err());
    app_frame(&mut editor, &context, key(egui::Key::Enter));
    assert!(editor.failure.is_none());
    assert_eq!(editor.transition, Some(Transition::Close));
    assert!(!editor.allow_close);
    assert_eq!(editor.document.project, before);
    let labels = app_frame(&mut editor, &context, vec![]);
    assert!(labels.iter().any(|(label, _)| label == "Save changes"));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn stopped_worker_reports_only_targeted_operations_and_keeps_accepted_state() {
    for kind in [
        FailureKind::FlatReference,
        FailureKind::FlatCorrection,
        FailureKind::Save,
        FailureKind::Export,
    ] {
        let (mut editor, _send, receiver) = fixture();
        add_ready_fixture(&mut editor);
        editor.sync_document();
        let before = editor.document.project.clone();
        let source = editor.current_source;
        let view = (editor.viewport, editor.source);
        drop(receiver);
        match kind {
            FailureKind::FlatReference => {
                editor.begin_flat_edit("missing-reference.ARW".into(), None)
            }
            FailureKind::FlatCorrection => {
                editor.start_document(DocumentWork::Correction(editor.document.clone(), source))
            }
            FailureKind::Save => editor.queue_save(
                "project.json".into(),
                crate::project::Save::New,
                Some(Transition::New),
            ),
            FailureKind::Export => editor.export(
                "output.exr".into(),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
            ),
            _ => unreachable!(),
        }
        assert_eq!(editor.failure.as_ref().unwrap().kind, kind);
        assert_eq!(
            editor.failure.as_ref().unwrap().details,
            "RAW worker stopped"
        );
        assert!(
            !editor.busy && editor.document_pending.is_none() && editor.document_failure.is_none()
        );
        assert!(editor.flat_edit.is_none());
        assert_eq!(editor.document.project, before);
        assert_eq!(editor.current_source, source);
        assert_eq!(editor.source, view.1);
        assert_eq!(editor.viewport.zoom, view.0.zoom);
        if kind == FailureKind::Save {
            assert_eq!(editor.transition, Some(Transition::New));
        }
    }
}

#[test]
fn stopped_worker_during_flat_acceptance_restores_working_context_and_ignores_late_completion() {
    let (mut editor, send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.source = false;
    editor.sync_document();
    let before = editor.document.project.clone();
    let id = editor.id;
    let path = editor.ready.as_ref().unwrap().path.clone();
    editor.begin_flat_edit("reference.ARW".into(), None);
    receiver.recv().unwrap();
    let request = editor.id;
    let (mut reference, _, _) = fixture();
    add_ready_fixture(&mut reference);
    send.send(Message::FlatPreview(
        request,
        Ok(Box::new(FlatPreview {
            ready: reference.ready.take().unwrap(),
            region: Some(Rect {
                x: 20,
                y: 20,
                width: 100,
                height: 80,
            }),
        })),
    ))
    .unwrap();
    editor.receive();
    drop(receiver);
    editor.apply_flat_edit();
    assert_eq!(
        editor.failure.as_ref().unwrap().kind,
        FailureKind::FlatCorrection
    );
    assert!(!editor.busy && editor.document_pending.is_none() && editor.document_failure.is_none());
    assert!(editor.flat_edit.is_none() && editor.pick.is_none());
    assert_eq!(editor.id, id);
    assert_eq!(editor.ready.as_ref().unwrap().path, path);
    assert!(!editor.source);
    assert_eq!(editor.document.project, before);
    send.send(Message::Prepared(request, Err("late completion".into())))
        .unwrap();
    editor.receive();
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "RAW worker stopped"
    );
}

#[test]
fn matching_save_success_resolves_the_failure_but_starting_a_retry_does_not() {
    let (mut editor, send, receiver) = fixture();
    editor.report_failure(FailureKind::Save, "destination unavailable".into());
    editor.queue_save(
        "project.json".into(),
        crate::project::Save::New,
        Some(Transition::Close),
    );
    receiver.recv().unwrap();
    assert!(editor.failure.is_some());
    let id = editor.document_pending.unwrap();
    send.send(Message::Saved(id, Ok(Box::new(editor.document.clone()))))
        .unwrap();
    editor.receive();
    assert!(editor.failure.is_none());
    assert_eq!(editor.resume_transition, Some(Transition::Close));
    assert!(!editor.allow_close);
}

#[test]
fn correction_failures_are_classified_from_work_and_stale_or_unrelated_results_do_not_retire_them()
{
    let (mut editor, send, receiver) = fixture();
    let before = editor.document.project.clone();
    editor.start_document(DocumentWork::Correction(
        editor.document.clone(),
        editor.current_source,
    ));
    receiver.recv().unwrap();
    let id = editor.document_pending.unwrap();
    send.send(Message::Prepared(id + 1, Err("stale correction".into())))
        .unwrap();
    editor.receive();
    assert!(editor.failure.is_none() && editor.busy);
    send.send(Message::Prepared(id, Err("unsupported GPU range".into())))
        .unwrap();
    editor.receive();
    assert_eq!(
        editor.failure.as_ref().unwrap().kind,
        FailureKind::FlatCorrection
    );
    assert_eq!(editor.document.project, before);
    assert!(!editor.busy);
    send.send(Message::Exported(
        editor.id,
        Ok("unrelated-success.tiff".into()),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "unsupported GPU range"
    );
    editor.start_document(DocumentWork::Correction(
        editor.document.clone(),
        editor.current_source,
    ));
    receiver.recv().unwrap();
    let id = editor.document_pending.unwrap();
    send.send(Message::Prepared(
        id,
        Ok(Box::new(Prepared {
            document: editor.document.clone(),
            ready: None,
            selected: editor.current_source,
            reuse_ready: true,
            correction: None,
            preserve_view: true,
        })),
    ))
    .unwrap();
    editor.receive();
    assert!(editor.failure.is_none());
    assert_eq!(editor.document.project, before);
}

#[test]
fn rejected_support_warns_without_changing_region_or_processing_and_successful_acceptance_resolves_it()
 {
    let (mut editor, _send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.pick = Some(Pick::Flat);
    editor.region = Some(
        Coverage::new(
            [[20.25, 20.], [40.25, 20.], [40.25, 40.], [20.25, 40.]]
                .map(crate::geometry::SourcePoint),
        )
        .unwrap(),
    );
    editor.sync_document();
    let before = editor.document.project.clone();
    let region = editor.region;
    editor.apply_flat_edit();
    let failure = editor.failure.as_ref().unwrap();
    assert_eq!(failure.kind, FailureKind::FlatSupport);
    assert_eq!(failure.presentation().severity, Severity::Warning);
    assert_eq!(editor.region, region);
    assert_eq!(editor.document.project, before);
    assert!(!editor.busy && editor.document_pending.is_none());
    assert!(receiver.try_recv().is_err());
    editor.resolve_failure(FailureKind::Export);
    assert!(editor.failure.is_some());
    editor.resolve_failure(FailureKind::FlatCorrection);
    assert!(editor.failure.is_none());
}

#[test]
fn export_failure_is_persistent_stale_results_cannot_replace_it_and_matching_success_resolves_it() {
    let (mut editor, send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let before = editor.document.project.clone();
    editor.export(
        "output.exr".into(),
        OutputFormat::Exr(ExrPrecision::Half),
        Overwrite::Refuse,
    );
    receiver.recv().unwrap();
    send.send(Message::Exported(editor.id + 1, Err("stale".into())))
        .unwrap();
    editor.receive();
    assert!(editor.failure.is_none() && editor.busy);
    send.send(Message::Exported(
        editor.id,
        Err("choose full-float EXR".into()),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
    assert!(!editor.busy);
    editor.status = "Other status".into();
    send.send(Message::Exported(
        editor.id + 1,
        Err("another stale error".into()),
    ))
    .unwrap();
    editor.receive();
    assert_eq!(
        editor.failure.as_ref().unwrap().details,
        "choose full-float EXR"
    );
    editor.export(
        "output.exr".into(),
        OutputFormat::Exr(ExrPrecision::Full),
        Overwrite::Refuse,
    );
    receiver.recv().unwrap();
    assert!(editor.failure.is_some());
    send.send(Message::Exported(editor.id, Ok("output.exr".into())))
        .unwrap();
    editor.receive();
    assert!(editor.failure.is_none());
    assert_eq!(editor.document.project, before);
}

#[test]
fn feedback_blocks_background_file_actions_and_dismissal_only_retires_the_message() {
    let (mut editor, _send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let before = editor.document.project.clone();
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
    assert!(receiver.try_recv().is_err());
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
    assert_eq!(editor.document.project, before);
    assert!(receiver.try_recv().is_err());
}
