use super::super::presentation::tests::{key, pointer};
use super::super::tests::{add_ready_fixture, app_frame, fixture};
use super::super::*;
use super::*;

#[test]
fn typed_lifecycle_scopes_ids_and_rejects_stale_progress_and_cleanup() {
    let mut tasks = TaskActivity::default();
    tasks.begin(Task::Import(1));
    tasks.report(
        TaskRequest::Document(1),
        TaskProgress::Counted {
            completed: 1,
            total: 4,
        },
    );
    assert_eq!(
        tasks.view().unwrap().measured,
        Some((0.25, "1 of 4".into()))
    );
    tasks.report(TaskRequest::Document(0), TaskProgress::Indeterminate);
    tasks.finish(TaskRequest::Export(1));
    assert_eq!(
        tasks.view().unwrap().measured,
        Some((0.25, "1 of 4".into()))
    );
    tasks.begin(Task::Photo(1));
    tasks.finish(TaskRequest::Document(1));
    tasks.report(
        TaskRequest::Document(1),
        TaskProgress::Counted {
            completed: 2,
            total: 4,
        },
    );
    assert_eq!(tasks.view().unwrap().title, "Loading photo…");
    assert!(tasks.view().unwrap().measured.is_none());
    tasks.finish(TaskRequest::Photo(1));
    tasks.finish(TaskRequest::Photo(1));
    assert!(tasks.view().is_none());
    tasks.begin(Task::Measurement(1, Pick::Dmin));
    tasks.finish(TaskRequest::Measurement(1, Pick::Dmax));
    assert!(tasks.view().is_some());
    tasks.finish(TaskRequest::Measurement(1, Pick::Dmin));
    assert!(tasks.view().is_none());
}

#[test]
fn progress_and_cancellation_states_keep_unknown_duration_honest() {
    let mut tasks = TaskActivity::default();
    for task in [Task::Import(1), Task::Open(1), Task::Analyze(1)] {
        tasks.begin(task);
        tasks.report(
            task.request(),
            TaskProgress::Counted {
                completed: 1,
                total: 4,
            },
        );
        assert!(tasks.view().unwrap().measured.is_some());
        tasks.report(task.request(), TaskProgress::Indeterminate);
        assert!(tasks.view().unwrap().measured.is_none());
        for (completed, total) in [(4, 4), (0, 0)] {
            tasks.report(task.request(), TaskProgress::Counted { completed, total });
            assert!(tasks.view().unwrap().measured.is_none());
        }
        assert!(tasks.view().is_some());
    }
    tasks.report(
        TaskRequest::Analyze(1),
        TaskProgress::Counted {
            completed: 1,
            total: 4,
        },
    );
    let action = tasks.view().unwrap().cancel.unwrap();
    assert_eq!(action.request, TaskRequest::Analyze(1));
    tasks.cancelling(TaskRequest::Analyze(0));
    assert!(tasks.view().unwrap().cancel.is_some());
    tasks.cancelling(action.request);
    tasks.report(
        action.request,
        TaskProgress::Counted {
            completed: 2,
            total: 4,
        },
    );
    let view = tasks.view().unwrap();
    assert_eq!(view.title, "Cancelling analysis…");
    assert!(view.cancel.is_none() && view.measured.is_none());
    tasks.finish(action.request);
    assert!(tasks.view().is_none());
    for task in [
        Task::New(1),
        Task::Save(1),
        Task::Correction(1),
        Task::Photo(1),
        Task::Reference(1),
        Task::Measurement(1, Pick::Dmin),
        Task::Export(1),
    ] {
        tasks.begin(task);
        tasks.report(
            task.request(),
            TaskProgress::Counted {
                completed: 1,
                total: 4,
            },
        );
        tasks.cancelling(task.request());
        let view = tasks.view().unwrap();
        assert!(view.measured.is_none() && view.cancel.is_none());
        assert!(!view.title.starts_with("Cancelling"));
        tasks.finish(task.request());
        assert!(tasks.view().is_none());
    }
}

#[test]
fn document_activity_is_typed_and_only_the_matching_terminal_reply_retires_it() {
    for save in [false, true] {
        let (mut editor, send, receiver) = fixture();
        editor.sync_document();
        let before = editor.document.project.clone();
        if save {
            editor.queue_save(
                "project.json".into(),
                crate::project::Save::New,
                Some(Transition::Close),
            );
        } else {
            editor.add_raws(vec!["one.ARW".into(), "two.ARW".into()]);
        }
        receiver.recv().unwrap();
        let id = editor.document_pending.unwrap();
        let title = editor.tasks.view().unwrap().title;
        editor.status = "Unrelated status".into();
        send.send(Message::Prepared(id + 1, Err("stale".into())))
            .unwrap();
        editor.receive();
        assert_eq!(editor.tasks.view().unwrap().title, title);
        assert!(editor.busy);
        assert_eq!(editor.document.project, before);
        if save {
            send.send(Message::Saved(id, Err("cannot save".into())))
                .unwrap();
        } else {
            send.send(Message::Prepared(id, Err("cannot import".into())))
                .unwrap();
        }
        editor.receive();
        assert!(editor.tasks.view().is_none() && !editor.busy);
        assert_eq!(editor.document.project, before);
    }
}

#[test]
fn file_progress_is_request_owned_and_returns_to_indeterminate_before_promotion() {
    for open in [false, true] {
        let (mut editor, send, receiver) = fixture();
        editor.sync_document();
        let before = editor.document.project.clone();
        if open {
            editor.start_document(DocumentWork::Open("project.json".into()));
        } else {
            editor.add_raws(vec!["one.ARW".into(), "two.ARW".into()]);
        }
        receiver.recv().unwrap();
        let id = editor.document_pending.unwrap();
        send.send(Message::DocumentProgress(id + 1, 1, 2)).unwrap();
        editor.receive();
        assert!(editor.tasks.view().unwrap().measured.is_none());
        send.send(Message::DocumentProgress(id, 1, 2)).unwrap();
        editor.receive();
        assert_eq!(
            editor.tasks.view().unwrap().measured,
            Some((0.5, "1 of 2".into()))
        );
        assert_eq!(editor.document.project, before);
        send.send(Message::DocumentProgress(id, 2, 2)).unwrap();
        editor.receive();
        assert!(editor.tasks.view().unwrap().measured.is_none());
        assert!(editor.busy && editor.document_pending == Some(id));
        send.send(Message::Prepared(id, Err("preparation failed".into())))
            .unwrap();
        send.send(Message::DocumentProgress(id, 1, 2)).unwrap();
        editor.receive();
        assert!(editor.tasks.view().is_none() && !editor.busy);
        assert_eq!(editor.document.project, before);
    }
    let (mut editor, send, receiver) = fixture();
    editor.queue_save("project.json".into(), crate::project::Save::New, None);
    receiver.recv().unwrap();
    send.send(Message::DocumentProgress(
        editor.document_pending.unwrap(),
        1,
        2,
    ))
    .unwrap();
    editor.receive();
    assert!(editor.tasks.view().unwrap().measured.is_none());
}

#[test]
fn send_failures_leave_no_working_feedback() {
    for operation in 0..6 {
        let (mut editor, _send, receiver) = fixture();
        add_ready_fixture(&mut editor);
        drop(receiver);
        match operation {
            0 => editor.add_raws(vec!["one.ARW".into()]),
            1 => editor.load("one.ARW".into()),
            2 => editor.begin_flat_edit("reference.ARW".into(), None),
            3 => editor.queue_save("project.json".into(), crate::project::Save::New, None),
            4 => editor.export(
                "output.tiff".into(),
                OutputFormat::Tiff(OutputTarget::Srgb),
                Overwrite::Refuse,
            ),
            _ => editor.begin_analysis(),
        }
        assert!(editor.tasks.view().is_none() && !editor.busy);
    }
}

#[test]
fn export_activity_preserves_identity_and_closes_before_failure_feedback() {
    let (mut editor, send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.sync_document();
    let before = editor.document.project.clone();
    editor.export(
        "output.exr".into(),
        OutputFormat::Exr(ExrPrecision::Full),
        Overwrite::Refuse,
    );
    receiver.recv().unwrap();
    let id = editor.id;
    editor.status = "Settled preview failed: unrelated".into();
    assert_eq!(editor.tasks.view().unwrap().title, "Exporting…");
    send.send(Message::Exported(id + 1, Err("stale".into())))
        .unwrap();
    editor.receive();
    assert!(editor.tasks.view().is_some());
    send.send(Message::Exported(id, Err("destination unavailable".into())))
        .unwrap();
    editor.receive();
    assert!(editor.tasks.view().is_none());
    assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
    assert_eq!(editor.document.project, before);
}

#[test]
fn analysis_cancellation_is_reachable_from_every_stage_and_stays_pending_until_reply() {
    for tab in 0..3 {
        let (mut editor, send, receiver) = fixture();
        add_ready_fixture(&mut editor);
        editor.begin_analysis();
        let Command::Analyze(id, request) = receiver.recv().unwrap() else {
            panic!()
        };
        editor.tab = tab;
        let context = egui::Context::default();
        app_frame(&mut editor, &context, vec![]);
        let labels = app_frame(&mut editor, &context, vec![]);
        let cancel = labels
            .iter()
            .find(|(label, _)| label == "Cancel analysis")
            .unwrap()
            .1
            .center();
        for pressed in [true, false] {
            app_frame(&mut editor, &context, pointer(cancel, pressed));
        }
        assert!(editor.busy && editor.analysis_pending.is_some());
        assert_ne!(
            editor
                .analysis_generation
                .load(std::sync::atomic::Ordering::Relaxed),
            request.revision
        );
        send.send(Message::AnalysisProgress(id, 1, 2)).unwrap();
        editor.receive();
        let view = editor.tasks.view().unwrap();
        assert_eq!(view.title, "Cancelling analysis…");
        assert!(view.cancel.is_none() && view.measured.is_none());
        app_frame(&mut editor, &context, key(egui::Key::Escape));
        assert!(editor.analysis_pending.is_some());
        send.send(Message::Analyzed(id, Ok(None))).unwrap();
        editor.receive();
        assert!(!editor.busy && editor.tasks.view().is_none());
        assert_eq!(editor.status, "Analysis cancelled");
    }
}

#[test]
fn superseded_analysis_updates_and_cancel_actions_cannot_touch_the_new_task() {
    let (mut editor, send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    editor.begin_analysis();
    let Command::Analyze(id, request) = receiver.recv().unwrap() else {
        panic!()
    };
    let action = editor.tasks.view().unwrap().cancel.unwrap();
    editor.load("next.ARW".into());
    assert!(editor.analysis_pending.is_none());
    assert_ne!(
        editor
            .analysis_generation
            .load(std::sync::atomic::Ordering::Relaxed),
        request.revision
    );
    let revision = editor
        .analysis_generation
        .load(std::sync::atomic::Ordering::Relaxed);
    editor.cancel_task(action.request);
    assert_eq!(
        editor
            .analysis_generation
            .load(std::sync::atomic::Ordering::Relaxed),
        revision
    );
    send.send(Message::AnalysisProgress(id, 1, 2)).unwrap();
    send.send(Message::Analyzed(id, Ok(None))).unwrap();
    editor.receive();
    assert_eq!(editor.tasks.view().unwrap().title, "Loading photo…");
    assert!(editor.busy);
}

#[test]
fn feedback_prominence_is_delayed_without_changing_document_availability() {
    let (mut editor, _send, receiver) = fixture();
    editor.start_document(DocumentWork::Open("project.json".into()));
    receiver.recv().unwrap();
    let context = egui::Context::default();
    app_frame(&mut editor, &context, vec![]);
    assert!(
        context
            .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
            .is_none()
    );
    editor.tasks.elapse(Duration::from_secs(1));
    app_frame(&mut editor, &context, vec![]);
    app_frame(&mut editor, &context, vec![]);
    assert!(
        context
            .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
            .is_some()
    );
    assert!(editor.busy && editor.document_pending.is_some());
}

#[test]
fn matching_success_retires_source_save_measurement_and_export_activity() {
    for operation in 0..4 {
        let (mut editor, send, receiver) = fixture();
        add_ready_fixture(&mut editor);
        match operation {
            0 => {
                let ready = editor.ready.take().unwrap();
                editor.load("current.ARW".into());
                receiver.recv().unwrap();
                send.send(Message::Ready(editor.id, Ok(Box::new(ready))))
                    .unwrap();
            }
            1 => {
                editor.queue_save("project.json".into(), crate::project::Save::New, None);
                receiver.recv().unwrap();
                send.send(Message::Saved(
                    editor.document_pending.unwrap(),
                    Ok(Box::new(editor.document.clone())),
                ))
                .unwrap();
            }
            2 => {
                editor.begin_pick(Pick::Dmin);
                editor.region = Some(Coverage::rectangle(Rect {
                    x: 12,
                    y: 7,
                    width: 30,
                    height: 20,
                }));
                editor.apply_pick();
                let Command::Pick(id, kind, ..) = receiver.recv().unwrap() else {
                    panic!()
                };
                send.send(Message::Picked(
                    id,
                    kind,
                    Ok(Measurement::Reference(editor.dmin.unwrap())),
                ))
                .unwrap();
            }
            _ => {
                editor.export(
                    "output.exr".into(),
                    OutputFormat::Exr(ExrPrecision::Full),
                    Overwrite::Refuse,
                );
                receiver.recv().unwrap();
                send.send(Message::Exported(editor.id, Ok("output.exr".into())))
                    .unwrap();
            }
        }
        assert!(editor.tasks.view().is_some());
        editor.receive();
        assert!(editor.tasks.view().is_none() && !editor.busy);
    }
}

#[test]
fn reference_failure_retires_only_the_matched_preparation_and_restores_the_view() {
    let (mut editor, send, receiver) = fixture();
    add_ready_fixture(&mut editor);
    let before = (
        editor.id,
        editor.current_source,
        editor.viewport,
        editor.source,
    );
    editor.begin_flat_edit("reference.ARW".into(), None);
    receiver.recv().unwrap();
    let id = editor.id;
    send.send(Message::FlatPreview(id + 1, Err("stale".into())))
        .unwrap();
    editor.receive();
    assert!(editor.tasks.view().is_some() && editor.busy);
    send.send(Message::FlatPreview(id, Err("cannot prepare".into())))
        .unwrap();
    editor.receive();
    assert!(editor.tasks.view().is_none() && !editor.busy);
    assert_eq!(
        (
            editor.id,
            editor.current_source,
            editor.viewport,
            editor.source
        ),
        before
    );
}

#[test]
fn activity_keeps_viewer_geometry_stable_and_never_covers_an_acknowledged_failure() {
    let (mut editor, _send, _receiver) = fixture();
    add_ready_fixture(&mut editor);
    let context = egui::Context::default();
    app_frame(&mut editor, &context, vec![]);
    app_frame(&mut editor, &context, vec![]);
    let canvas = editor.last_layout.unwrap().0.canvas;
    editor.tasks.begin(Task::Export(editor.id));
    editor.busy = true;
    app_frame(&mut editor, &context, vec![]);
    assert_eq!(editor.last_layout.unwrap().0.canvas, canvas);
    editor.begin_analysis();
    app_frame(&mut editor, &context, vec![]);
    assert_eq!(editor.last_layout.unwrap().0.canvas, canvas);
    editor.analysis_pending = None;
    editor.tasks.begin(Task::Save(editor.id));
    editor.tasks.elapse(Duration::from_secs(1));
    editor.report_failure(FailureKind::Save, "previous failure".into());
    app_frame(&mut editor, &context, vec![]);
    let labels = app_frame(&mut editor, &context, vec![]);
    assert!(
        labels
            .iter()
            .any(|(label, _)| label == "Could not save project")
    );
    assert!(
        context
            .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
            .is_none()
    );
    assert!(editor.failure.is_some() && editor.tasks.view().is_some());
}
