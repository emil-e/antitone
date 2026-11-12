use super::super::tests::{add_ready_fixture, app_frame, fixture};
use super::super::ui::test_support::{key, pointer};
use super::super::*;

#[test]
fn save_feedback_is_owned_by_the_document_write_not_its_callers_waiter() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("saved.antitone");
    let hold = crate::background::testing::pause();
    crate::application::test_support::with_application(fixture(), |app| {
        app.enter(|editor, cx| {
            drop(editor.start_save(
                save::Write {
                    document: editor.document.capture(),
                    path: path.clone(),
                    policy: crate::project::Save::New,
                },
                cx,
            ));
            assert!(editor.document.write.pending() && editor.interaction_blocked());
            assert_eq!(editor.activities.view().unwrap().title, "Saving project…");
            editor.status = "Unrelated status".into();
            assert_eq!(editor.activities.view().unwrap().title, "Saving project…");
        });
        app.pump();
        assert!(!path.exists());
        drop(hold);
        app.run_until(Duration::from_secs(10), |editor| {
            !editor.document.write.pending()
        });
        app.read(|editor| {
            assert!(editor.activities.view().is_none() && !editor.interaction_blocked());
            assert!(!editor.document.dirty());
        });
        assert!(path.is_file());
    });
}

#[test]

fn unavailable_export_services_and_analysis_device_leave_no_working_feedback() {
    for export in [true, false] {
        let mut editor = fixture();
        add_ready_fixture(&mut editor);
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
            app.run_until(Duration::from_secs(5), |e| !e.interaction_blocked());
            app.read(|editor| {
                assert!(!editor.document.write.pending());
                assert_eq!(editor.document.state.project, before);
            });
            app.pump();
            app.read(|editor| {
                assert!(editor.activities.view().is_none() && !editor.interaction_blocked())
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
            let before = editor.document.state.project.clone();
            editor.export(
                "output.exr".into(),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
                cx,
            );
            assert!(editor.document.export.operation.is_some());
            editor.status = "Unrelated preview status".into();
            assert_eq!(editor.activities.view().unwrap().title, "Exporting…");
            before
        });
        app.run_until(Duration::from_secs(5), |editor| {
            editor.document.export.operation.is_none()
        });
        app.read(|editor| {
            assert!(editor.activities.view().is_none() && !editor.interaction_blocked());
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::Export.title()
            );
            assert_eq!(
                editor.dialogs.message().unwrap().details,
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
                let reply = super::super::analysis::testing::fitting(editor);
                editor.begin_analysis(cx);
                editor.activities.elapse(Duration::from_secs(1));
                let progress = editor
                    .document
                    .analysis_operation()
                    .unwrap()
                    .activity
                    .as_ref()
                    .unwrap()
                    .clone();
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
                assert!(editor.interaction_blocked() && editor.analysis_fitting());
                assert!(
                    editor
                        .document
                        .analysis_operation()
                        .unwrap()
                        .cancelled
                        .is_cancelled()
                );
                progress.progress(1, 2);
                let view = editor.activities.view().unwrap();
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
                assert!(!editor.interaction_blocked() && editor.activities.view().is_none());
                assert_eq!(editor.status, "Analysis cancelled");
            });
        });
    }
}

#[test]
fn retired_analysis_progress_and_cancel_actions_cannot_touch_the_new_task() {
    crate::application::test_support::with_application(fixture(), |app| {
        let (mut reply, progress, cancelled, action) = app.enter(|editor, cx| {
            add_ready_fixture(editor);
            let reply = super::super::analysis::testing::fitting(editor);
            editor.begin_analysis(cx);
            let progress = editor
                .document
                .analysis_operation()
                .unwrap()
                .activity
                .as_ref()
                .unwrap()
                .clone();
            let cancelled = editor
                .document
                .analysis_operation()
                .unwrap()
                .cancelled
                .clone();
            let action = editor.activities.view().unwrap();
            (reply, progress, cancelled, action)
        });
        super::super::analysis::testing::request(app, &mut reply);
        app.update(Editor::end_tool);
        app.pump();
        let (_regions, receive) = futures_channel::oneshot::channel();
        app.enter(|editor, cx| {
            editor.document.spatial_reply = Some(receive);
            editor.review_crop_proposals(editor.document.project.order.clone(), "roll", cx);
            assert!(!editor.analysis_fitting());
            assert!(cancelled.is_cancelled());
            let before = editor.activities.view().unwrap().measured;
            action.request_cancel();
            progress.progress(1, 2);
            assert!(
                editor
                    .document
                    .spatial_preparing
                    .as_ref()
                    .unwrap()
                    .visible()
            );
            assert_eq!(editor.activities.view().unwrap().measured, before);
            assert!(editor.interaction_blocked() && editor.document.auto_crop.active());
        });
        app.pump();
        assert!(reply.result.send(Ok(None)).is_err());
        app.read(|editor| assert!(!editor.status.contains("Analysis cancelled")));
    });
}

#[test]
fn feedback_prominence_is_delayed_without_changing_document_availability() {
    let editor = fixture();
    editor.activities.freeze_time();
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.open_project("project.json".into(), cx);
            assert!(editor.interaction_blocked() && editor.files.opening());
            let context = egui::Context::default();
            app_frame(editor, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_none()
            );
            editor.activities.elapse(Duration::from_secs(1));
            app_frame(editor, &context, vec![]);
            app_frame(editor, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_some()
            );
            assert!(editor.interaction_blocked() && editor.files.opening());
        });
    });
}

#[test]
fn export_blocks_the_workspace_before_its_progress_modal_becomes_prominent() {
    let mut editor = fixture();
    editor.activities.freeze_time();
    add_ready_fixture(&mut editor);
    crate::application::test_support::with_application(editor, |app| {
        app.enter(|editor, cx| {
            editor.export(
                "delayed.exr".into(),
                OutputFormat::Exr(ExrPrecision::Full),
                Overwrite::Refuse,
                cx,
            );
            assert!(editor.workspace_input.is_blocked());
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
            editor.activities.elapse(Duration::from_secs(1));
            app_frame(editor, &context, vec![]);
            app_frame(editor, &context, vec![]);
            assert!(
                context
                    .memory(|memory| memory.area_rect(egui::Id::new("working-document")))
                    .is_some()
            );
            assert!(editor.interaction_blocked() && editor.document.export.operation.is_some());
        });
    });
}

#[test]
fn matching_success_retires_source_and_measurement_activity() {
    for operation in [0, 2] {
        let mut editor = fixture();
        add_ready_fixture(&mut editor);
        if operation == 2 {
            editor = crate::editor::tests::with_document_services(editor);
        }
        let hold = crate::background::testing::pause();
        crate::application::test_support::with_application(editor, |app| {
            if operation == 2 {
                app.enter(|editor, cx| editor.begin_test_picker(PickerCase::Dmin, cx));
                app.pump();
            }
            let mut source_completion = None;
            app.enter(|editor, access| match operation {
                0 => {
                    let ready = editor.document.visit.canvas.ready.take().unwrap();
                    editor.load("current.ARW".into(), access);
                    assert!(editor.document.visit.source.pending());
                    source_completion = Some((editor.document.visit.view, ready));
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
                    editor.set_test_pick_region(Some(Coverage::rectangle(Rect {
                        x: area.x + 8,
                        y: area.y + 8,
                        width: area.width - 16,
                        height: area.height - 16,
                    })));
                    editor.apply_pick(access);
                    assert!(!editor.picking());
                }
            });
            if operation == 2 {
                app.pump();
            }
            assert!(app.read(|editor| editor.activities.view().is_some()));
            if let Some((view, ready)) = source_completion {
                app.update(|e| e.source_result(view, Ok(Box::new(ready))));
            }
            app.pump();
            drop(hold);
            if operation == 2 {
                app.run_until(Duration::from_secs(5), |editor| {
                    !editor.interaction_blocked()
                });
                assert_eq!(
                    app.read(|editor| editor.history_label(crate::editor::history::Direction::Undo)),
                    Some("Calibration pick"),
                    "{}",
                    app.read(|editor| editor.status.clone())
                );
            }
            assert!(
                app.read(
                    |editor| editor.activities.view().is_none() && !editor.interaction_blocked()
                )
            );
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
        assert!(app.read(|e| e.activities.view().is_some() && e.interaction_blocked()));
        app.run_until(std::time::Duration::from_secs(5), |e| {
            !e.interaction_blocked()
        });
        app.read(|e| {
            assert!(e.activities.view().is_none());
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
            let export = editor
                .activities
                .start(ActivityOptions::new("Exporting…").modal());
            app_frame(editor, &context, vec![]);
            assert_eq!(
                editor.document.visit.canvas.last_layout.unwrap().0.canvas,
                canvas
            );
            editor.begin_analysis(cx);
            assert!(
                editor.document.analysis_operation().is_none(),
                "blocking work rejects tool activation"
            );
            app_frame(editor, &context, vec![]);
            assert_eq!(
                editor.document.visit.canvas.last_layout.unwrap().0.canvas,
                canvas
            );
            drop(export);
            let _save = editor
                .activities
                .start(ActivityOptions::new("Saving project…").modal());
            editor.activities.elapse(Duration::from_secs(1));
            editor.report_failure(FailureCase::Save, "previous failure".into());
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
            assert!(editor.dialogs.is_open() && editor.activities.view().is_some());
        });
    });
}
