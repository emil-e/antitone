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
    tasks.begin(Task::Measurement(1, "Dmin"));
    tasks.finish(TaskRequest::Measurement(0));
    assert!(tasks.view().is_some());
    tasks.finish(TaskRequest::Measurement(1));
    assert!(tasks.view().is_none());
}

#[test]
fn progress_and_cancellation_states_keep_unknown_duration_honest() {
    let mut tasks = TaskActivity::default();
    for task in [
        Task::Import(1),
        Task::Open(1),
        Task::Analyze(1),
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
        assert!(tasks.view().unwrap().measured.is_some());
        tasks.report(task.request(), TaskProgress::Indeterminate);
        assert!(tasks.view().unwrap().measured.is_none());
        for (completed, total) in [(4, 4), (0, 0)] {
            tasks.report(task.request(), TaskProgress::Counted { completed, total });
            assert!(tasks.view().unwrap().measured.is_none());
        }
        assert!(tasks.view().is_some());
    }
    tasks.begin(Task::Analyze(1));
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
        Task::Measurement(1, "Dmin"),
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
        let editor = fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.sync_document();
                let before = editor.document.state.project.clone();
                if save {
                    editor.save_project(
                        "project.json".into(),
                        crate::project::Save::New,
                        Some(Transition::Close),
                        cx,
                    );
                } else {
                    editor.add_raws(vec!["one.ARW".into(), "two.ARW".into()], cx);
                }
                assert!(editor.busy && editor.document_pending.is_some());
                let id = editor.document_pending.unwrap();
                let title = editor.tasks.view().unwrap().title;
                editor.status = "Unrelated status".into();
                editor.document_prepared(id + 1, Err("stale".into()));
                assert_eq!(editor.tasks.view().unwrap().title, title);
                assert!(editor.busy);
                assert_eq!(editor.document.state.project, before);
                if save {
                    editor.save_result(id, Err("cannot save".into()), cx);
                } else {
                    editor.document_prepared(id, Err("cannot import".into()));
                }
                assert!(editor.tasks.view().is_none() && !editor.busy);
                assert_eq!(editor.document.state.project, before);
            });
        });
    }
}

#[test]
fn file_progress_is_request_owned_and_returns_to_indeterminate_before_promotion() {
    for open in [false, true] {
        let editor = fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.sync_document();
                let before = editor.document.state.project.clone();
                if open {
                    editor.start_document(Preparation::Open("project.json".into()), cx);
                } else {
                    editor.add_raws(vec!["one.ARW".into(), "two.ARW".into()], cx);
                }
                assert!(editor.busy && editor.document_pending.is_some());
                let id = editor.document_pending.unwrap();
                editor.document_progress(id + 1, 1, 2);
                assert!(editor.tasks.view().unwrap().measured.is_none());
                editor.document_progress(id, 1, 2);
                assert_eq!(
                    editor.tasks.view().unwrap().measured,
                    Some((0.5, "1 of 2".into()))
                );
                assert_eq!(editor.document.state.project, before);
                editor.document_progress(id, 2, 2);
                assert!(editor.tasks.view().unwrap().measured.is_none());
                assert!(editor.busy && editor.document_pending == Some(id));
                editor.document_prepared(id, Err("preparation failed".into()));
                editor.document_progress(id, 1, 2);
                assert!(editor.tasks.view().is_none() && !editor.busy);
                assert_eq!(editor.document.state.project, before);
            });
        });
    }
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.save_project("project.json".into(), crate::project::Save::New, None, cx);
            assert!(editor.busy && editor.document_pending.is_some());
            editor.document_progress(editor.document_pending.unwrap(), 1, 2);
            assert!(editor.tasks.view().unwrap().measured.is_none());
        });
    });
}

#[test]
fn unavailable_export_and_analysis_services_leave_no_working_feedback() {
    for export in [true, false] {
        let mut editor = fixture();
        add_ready_fixture(&mut editor);
        editor.sync_document();
        let before = editor.document.state.project.clone();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, access| {
                if export {
                    editor.export(
                        "output.tiff".into(),
                        OutputFormat::Tiff(OutputTarget::Srgb),
                        Overwrite::Refuse,
                        access,
                    );
                    assert!(editor.document.export.operation.is_some());
                } else {
                    editor.begin_analysis(access);
                }
            });
            app.run_until(Duration::from_secs(5), |e| !e.busy);
            app.read(|editor| {
                assert!(editor.tasks.view().is_none() && !editor.busy);
                assert_eq!(editor.document.state.project, before);
            });
        });
    }
}

#[test]
fn export_activity_closes_before_awaited_failure_feedback() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        let before = app.enter(|editor, cx| {
            add_ready_fixture(editor);
            editor.sync_document();
            let before = editor.document.state.project.clone();
            editor.export(
                "output.exr".into(),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
                cx,
            );
            assert!(editor.document.export.operation.is_some());
            editor.status = "Unrelated preview status".into();
            assert_eq!(editor.tasks.view().unwrap().title, "Exporting…");
            before
        });
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.export.operation.is_none()
        });
        app.read(|editor| {
            assert!(editor.tasks.view().is_none() && !editor.busy);
            assert_eq!(editor.failure.as_ref().unwrap().kind, FailureKind::Export);
            assert_eq!(
                editor.failure.as_ref().unwrap().details,
                "Export services unavailable"
            );
            assert_eq!(editor.document.state.project, before);
        });
    });
}

#[test]
fn analysis_cancellation_is_reachable_from_every_stage_and_stays_pending_until_acknowledgement() {
    for tab in 0..3 {
        crate::application::test_support::with_application(fixture(), |app| {
            let reply = app.enter(|editor, cx| {
                add_ready_fixture(editor);
                let reply = super::super::analysis_workflow::testing::fitting(editor);
                editor.begin_analysis(cx);
                editor.tasks.elapse(Duration::from_secs(1));
                let id = editor.analysis_work_id().unwrap();
                editor.tab = tab;
                let context = egui::Context::default();
                app_frame(editor, &context, vec![]);
                let labels = app_frame(editor, &context, vec![]);
                let cancel = labels
                    .iter()
                    .find(|(label, _)| label == "Cancel analysis")
                    .unwrap()
                    .1
                    .center();
                for pressed in [true, false] {
                    app_frame(editor, &context, pointer(cancel, pressed));
                }
                assert!(editor.busy && editor.analysis_fitting());
                assert!(
                    editor
                        .document
                        .analysis_operation()
                        .unwrap()
                        .cancelled
                        .is_cancelled()
                );
                editor.analysis_progress(id, 1, 2);
                let view = editor.tasks.view().unwrap();
                assert_eq!(view.title, "Cancelling analysis…");
                assert!(view.cancel.is_none() && view.measured.is_none());
                app_frame(editor, &context, key(egui::Key::Escape));
                assert!(editor.analysis_fitting());
                reply
            });
            app.pump();
            reply.result.send(Ok(None)).ok().unwrap();
            app.run_until(Duration::from_secs(10), |editor| {
                editor.document.analysis_operation().is_none()
            });
            app.read(|editor| {
                assert!(!editor.busy && editor.tasks.view().is_none());
                assert_eq!(editor.status, "Analysis cancelled");
            });
        });
    }
}

#[test]
fn retired_analysis_progress_and_cancel_actions_cannot_touch_the_new_task() {
    crate::application::test_support::with_application(fixture(), |app| {
        let (mut reply, id, cancelled, action) = app.enter(|editor, cx| {
            add_ready_fixture(editor);
            let reply = super::super::analysis_workflow::testing::fitting(editor);
            editor.begin_analysis(cx);
            let id = editor.analysis_work_id().unwrap();
            let cancelled = editor
                .document
                .analysis_operation()
                .unwrap()
                .cancelled
                .clone();
            let action = editor.tasks.view().unwrap().cancel.unwrap();
            (reply, id, cancelled, action)
        });
        super::super::analysis_workflow::testing::request(app, &mut reply);
        let (_regions, receive) = futures_channel::oneshot::channel();
        app.enter(|editor, cx| {
            editor.document.spatial_reply = Some(receive);
            editor.review_crop_proposals(editor.document.project.order.clone(), "roll", cx);
            assert!(!editor.analysis_fitting());
            assert!(cancelled.is_cancelled());
            let current =
                TaskRequest::Regions(editor.document.spatial_preparing.as_ref().unwrap().id);
            let progress = editor.tasks.view().unwrap().measured;
            editor.cancel_task(action.request);
            editor.analysis_progress(id, 1, 2);
            assert!(editor.tasks.is_current(current));
            assert_eq!(editor.tasks.view().unwrap().measured, progress);
            assert!(editor.busy && editor.document.auto_crop_operation().is_some());
        });
        app.pump();
        assert!(reply.result.send(Ok(None)).is_err());
        app.read(|editor| assert!(!editor.status.contains("Analysis cancelled")));
    });
}

#[test]
fn feedback_prominence_is_delayed_without_changing_document_availability() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.start_document(Preparation::Open("project.json".into()), cx);
            assert!(editor.busy && editor.document_pending.is_some());
            let context = egui::Context::default();
            app_frame(editor, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_none()
            );
            editor.tasks.elapse(Duration::from_secs(1));
            app_frame(editor, &context, vec![]);
            app_frame(editor, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_some()
            );
            assert!(editor.busy && editor.document_pending.is_some());
        });
    });
}

#[test]
fn export_blocks_the_workspace_before_its_progress_modal_becomes_prominent() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.export(
                "delayed.exr".into(),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
                cx,
            );
            assert!(editor.tasks.blocks_workspace());
            assert!(
                !editor
                    .application_menu_state()
                    .enabled(crate::platform::menu::Action::Settings)
            );
            assert!(!editor.fullscreen_available());
            let context = egui::Context::default();
            app_frame(editor, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_none()
            );
            editor.tasks.elapse(Duration::from_secs(1));
            app_frame(editor, &context, vec![]);
            app_frame(editor, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_some()
            );
            assert!(editor.busy && editor.document.export.operation.is_some());
        });
    });
}

#[test]
fn matching_success_retires_source_save_and_measurement_activity() {
    for operation in 0..3 {
        let mut editor = fixture();
        add_ready_fixture(&mut editor);
        let hold = crate::background::testing::pause();
        crate::application::test_support::with_application(editor, |app| {
            if operation == 2 {
                app.enter(|editor, cx| editor.begin_pick(Pick::Dmin, cx));
                app.pump();
            }
            let mut source_completion = None;
            let mut save_completion = None;
            app.enter(|editor, access| match operation {
                0 => {
                    let ready = editor.document.visit.canvas.ready.take().unwrap();
                    editor.load("current.ARW".into(), access);
                    assert!(editor.document.visit.source.pending());
                    source_completion = Some((editor.document.visit.view, ready));
                }
                1 => {
                    editor.save_project(
                        "project.json".into(),
                        crate::project::Save::New,
                        None,
                        access,
                    );
                    assert!(editor.busy && editor.document_pending.is_some());
                    save_completion =
                        Some((editor.document_pending.unwrap(), editor.document.capture()));
                }
                _ => {
                    let source = editor
                        .document
                        .visit
                        .canvas
                        .ready
                        .as_ref()
                        .unwrap()
                        .developed
                        .clone();
                    let area = Scan::from_source(source).area;
                    // The synthetic SDK source marks its repeated borders ineligible.
                    // This scenario needs a successful pick, not a rejected capture.
                    editor.set_pick_region(Some(Coverage::rectangle(Rect {
                        x: area.x + 8,
                        y: area.y + 8,
                        width: area.width - 16,
                        height: area.height - 16,
                    })));
                    editor.apply_pick(access);
                    assert!(editor.measurement_pending());
                }
            });
            assert!(app.read(|editor| editor.tasks.view().is_some()));
            if let Some((id, saved)) = save_completion {
                app.enter(|e, cx| e.save_result(id, Ok(Box::new(saved)), cx));
            }
            if let Some((view, ready)) = source_completion {
                app.update(|e| e.source_result(view, Ok(Box::new(ready))));
            }
            app.pump();
            drop(hold);
            if operation == 2 {
                app.run_until(Duration::from_secs(5), |editor| !editor.busy);
                assert_eq!(
                    app.read(|editor| editor.history_label(crate::editor::history::Direction::Undo)),
                    Some("Calibration pick"),
                    "{}",
                    app.read(|editor| editor.status.clone())
                );
            }
            assert!(app.read(|editor| editor.tasks.view().is_none() && !editor.busy));
        });
    }
}

#[test]
fn reference_failure_finishes_preparation_and_restores_the_view() {
    let mut editor = fixture();
    add_ready_fixture(&mut editor);
    let previous_visit = editor.document.visit.view;
    let before = (
        editor.document.visit.current_source,
        editor.document.visit.canvas.viewport,
        editor.document.source,
    );
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|e, cx| e.begin_flat_edit("reference.ARW".into(), None, cx));
        assert!(app.read(|e| e.tasks.view().is_some() && e.busy));
        app.run_until(std::time::Duration::from_secs(5), |e| !e.busy);
        app.read(|e| {
            assert!(e.tasks.view().is_none());
            assert_eq!(e.document.visit.view, previous_visit);
            assert_eq!(
                (
                    e.document.visit.current_source,
                    e.document.visit.canvas.viewport,
                    e.document.source
                ),
                before
            );
        });
    });
}

#[test]
fn activity_keeps_viewer_geometry_stable_and_never_covers_an_acknowledged_failure() {
    let editor = fixture();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            add_ready_fixture(editor);
            let context = egui::Context::default();
            app_frame(editor, &context, vec![]);
            app_frame(editor, &context, vec![]);
            let canvas = editor.document.visit.canvas.last_layout.unwrap().0.canvas;
            editor.tasks.begin(Task::Export(editor.id));
            editor.busy = true;
            app_frame(editor, &context, vec![]);
            assert_eq!(
                editor.document.visit.canvas.last_layout.unwrap().0.canvas,
                canvas
            );
            editor.begin_analysis(cx);
            app_frame(editor, &context, vec![]);
            assert_eq!(
                editor.document.visit.canvas.last_layout.unwrap().0.canvas,
                canvas
            );
            editor.document.set_analysis_operation(None);
            editor.tasks.begin(Task::Save(editor.id));
            editor.tasks.elapse(Duration::from_secs(1));
            editor.report_failure(FailureKind::Save, "previous failure".into());
            app_frame(editor, &context, vec![]);
            let labels = app_frame(editor, &context, vec![]);
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
        });
    });
}
