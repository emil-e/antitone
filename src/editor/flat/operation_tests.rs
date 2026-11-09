use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{document::Document, source, source_loading, tests::fixture},
};
use std::sync::{Arc, atomic::Ordering};
use std::{
    path::PathBuf,
    sync::{Mutex, atomic::AtomicUsize},
    time::Duration,
};

fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn editor(scheduler: source_loading::SourceLoader) -> Editor {
    let mut e = fixture();
    e.document = Document::default().into();
    e.set_dmin(None);
    e.set_dmax(None);
    let (device, queue) = pollster::block_on(async {
        let instance = crate::gpu::instance();
        instance
            .request_adapter(&Default::default())
            .await
            .unwrap()
            .request_device(&Default::default())
            .await
            .unwrap()
    });
    e.document.visit.working_view =
        crate::editor::working_view::WorkingView::new(device.clone(), queue.clone());
    e.source_services = Some(source::Services {
        developments: scheduler,
        renderers: Arc::default(),
        device,
        queue,
    });
    e
}

fn begin(app: &TestApplication<'_, Editor>, name: &str) {
    let cx = app.access();
    app.update(|e| e.begin_flat_edit(path(name), None, &cx));
}

fn apply(app: &TestApplication<'_, Editor>) {
    let cx = app.access();
    app.update(|e| e.apply_flat_edit(&cx));
}

fn wait(app: &mut TestApplication<'_, Editor>) {
    app.run_until(Duration::from_secs(20), |e| !e.interaction_blocked());
}

fn import(app: &mut TestApplication<'_, Editor>) {
    app.enter(|e, cx| {
        e.add_raws(vec![path("illuminated-patches.dng")], cx);
    });
    wait(app);
}

#[test]
fn reference_proposal_and_correction_finish_without_backend_or_paint_and_keep_working_context() {
    let reference_reads = Arc::new(AtomicUsize::new(0));
    let observed = reference_reads.clone();
    let scheduler = source_loading::SourceLoader::new_observed(move |file, phase, _| {
        if file == path("bare-light.dng")
            && matches!(phase, source_loading::NativePhase::UnpackBegin)
        {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (before, visit, camera) = app.update(|e| {
            let camera = e.document.visit.canvas.ready.as_ref().unwrap().clone();
            e.document.visit.canvas.viewport.zoom = Some(1.375);
            (e.history_snapshot(), e.document.visit.view, camera)
        });
        begin(app, "bare-light.dng");
        assert!(app.read(|e| e.document.flat_edit().unwrap().task.is_some()));
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.test_pick_region().is_some());
            assert!(
                e.document
                    .flat_edit()
                    .unwrap()
                    .view
                    .source()
                    .unwrap()
                    .correction
                    .is_none()
            );
            assert_eq!(e.document.visit.view, visit);
        });
        apply(app);
        wait(app);
        app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            assert!(e.document.flat_edit().is_none() && e.document_pending.is_none());
            assert_eq!(e.document.state.project.order, before.order);
            assert_eq!(e.document.visit.canvas.viewport.zoom, Some(1.375));
            assert_eq!(
                (
                    e.document.state.project.dmin(),
                    e.document.state.project.dmax()
                ),
                (before.dmin(), before.dmax())
            );
            assert_eq!(
                e.document.visit.canvas.ready.as_ref().unwrap().path,
                path("illuminated-patches.dng")
            );
            assert_eq!(e.document.visit.view, visit);
            let ready = e.document.visit.canvas.ready.as_ref().unwrap();
            assert!(Arc::ptr_eq(&camera.developed, &ready.developed));
            assert!(
                camera.proxy.same_source(&ready.proxy),
                "correction must retain the uploaded camera input"
            );
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Correction")
            );
        });
        assert_eq!(
            reference_reads.load(Ordering::Relaxed),
            1,
            "estimation must consume the captured reference, not reopen it"
        );
        let accepted = app.read(|e| e.document.state.project.clone());
        let current = app.read(|e| {
            e.document
                .visit
                .canvas
                .ready
                .as_ref()
                .unwrap()
                .developed
                .clone()
        });
        begin(app, "missing-reference.dng");
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.state.project, accepted);
            assert!(Arc::ptr_eq(
                &current,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ));
            assert_eq!(
                e.dialogs.message().unwrap().title,
                FailureCase::FlatReference.title()
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn correction_acquisition_rejects_an_existing_edit_without_retiring_it_or_its_reference_tool() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        begin(app, "bare-light.dng");
        wait(app);
        let (before, edit) = app.update(|e| {
            let before = e.document.project.clone();
            (before, e.edit_project("Existing edit").unwrap())
        });
        apply(app);
        app.read(|e| {
            assert!(
                e.document.flat_edit().is_some(),
                "rejected Apply retains the reference"
            );
            assert!(e.document.flat_correction.is_none());
            assert!(edit.preview().active());
            assert!(!e.document.document_preview.active());
            assert_eq!(e.document.project, before);
            assert!(e.history_label(history::Direction::Undo).is_none());
            assert!(!e.dialogs.is_open());
        });
        drop(edit);
        apply(app);
        wait(app);
        app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Correction")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn accepted_correction_outlives_tool_dismissal_before_and_after_its_first_poll() {
    for poll in [false, true] {
        let scheduler = source_loading::SourceLoader::new();
        with_application(editor(scheduler.clone()), |app| {
            import(app);
            begin(app, "bare-light.dng");
            wait(app);
            let before = app.read(|editor| editor.document.project.clone());
            let hold = crate::background::testing::pause();
            apply(app);
            let publication = app.update(|editor| {
                assert!(
                    editor.document.flat_edit().is_none(),
                    "Apply ends selection synchronously"
                );
                assert!(editor.interaction_blocked());
                assert_eq!(editor.document.project, before);
                assert_eq!(editor.document.project_snapshot(), before);
                assert!(editor.document.correction.is_none());
                assert!(editor.edit_project("Overlapping edit").is_err());
                assert_eq!(
                    editor.history_label(history::Direction::Undo),
                    Some("Import photos")
                );
                editor.document.document_preview.clone()
            });
            if poll {
                app.pump();
            }
            app.update(Editor::finish_flat_edit);
            app.read(|editor| {
                assert!(publication.active());
                assert!(editor.interaction_blocked());
                assert_eq!(editor.document.project, before);
            });
            drop(hold);
            wait(app);
            app.read(|editor| {
                assert!(editor.document.correction.is_some(), "{}", editor.status);
                assert!(editor.document.flat_edit().is_none());
                assert_eq!(editor.document.project.order, before.order);
                assert_eq!(editor.document.project.dmin(), before.dmin());
                assert_eq!(
                    editor.history_label(crate::editor::history::Direction::Undo),
                    Some("Correction")
                );
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn retiring_preparation_or_replacing_accepted_work_cannot_publish_into_a_successor_document() {
    for accept in [false, true] {
        let scheduler = source_loading::SourceLoader::new();
        with_application(editor(scheduler.clone()), |app| {
            let before = app.read(|e| e.document.state.project.clone());
            if accept {
                begin(app, "bare-light.dng");
                wait(app);
            }
            let hold = crate::background::testing::pause();
            if accept {
                apply(app);
            } else {
                begin(app, "bare-light.dng");
            }
            let publication = app.read(|e| {
                e.document
                    .document_preview
                    .active()
                    .then(|| e.document.document_preview.clone())
            });
            assert_eq!(publication.is_some(), accept);
            app.pump();
            app.update(Editor::finish_flat_edit);
            app.pump();
            app.read(|e| {
                assert_eq!(e.document.state.project, before);
                assert_eq!(e.interaction_blocked(), accept);
                assert_eq!(e.document.flat_correction.is_some(), accept);
                assert!(e.document.flat_edit().is_none());
            });
            app.enter(|e, cx| {
                e.start_document(crate::editor::Preparation::New, cx);
            });
            let operation = app.read(|e| e.document_pending);
            app.read(|e| {
                assert!(
                    publication
                        .as_ref()
                        .is_none_or(|owner| !e.document.document_preview.same_edit(owner))
                )
            });
            app.read(|e| assert_eq!(e.document_pending, operation));
            drop(hold);
            wait(app);
            app.read(|e| {
                assert!(!e.dialogs.is_open());
                assert!(e.document.state.project.flat_field().is_none());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn failed_document_change_ends_reference_setup_without_displacing_the_working_view() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (before, pixels, visit) = app.update(|e| {
            (
                e.history_snapshot(),
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .clone(),
                e.document.visit.view,
            )
        });
        begin(app, "bare-light.dng");
        wait(app);
        let hold = crate::background::testing::pause();
        apply(app);
        let publication = app.read(|e| e.document.document_preview.clone());
        app.pump();
        app.enter(|e, cx| {
            e.start_document(
                crate::editor::Preparation::Open(path("missing-project.antitone")),
                cx,
            );
        });
        app.read(|e| {
            assert!(e.document.flat_edit().is_none());
            assert!(e.document.visit.canvas.ready.is_some());
            assert!(e.document_pending.is_some());
        });
        app.read(|e| assert!(!e.document.document_preview.same_edit(&publication)));
        drop(hold);
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.document.flat_edit().is_none() && e.document.correction.is_none());
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ));
            assert_eq!(e.document.visit.view, visit);
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Import photos")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn departed_reference_waiter_can_join_running_unpack_without_stale_publication() {
    let (started, starts) = std::sync::mpsc::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let resume = Mutex::new(resume);
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let scheduler = source_loading::SourceLoader::new_observed(move |_, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin)
            && observed.fetch_add(1, Ordering::Relaxed) == 0
        {
            started.send(()).unwrap();
            resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(20))
                .unwrap();
        }
        Ok(())
    });
    with_application(editor(scheduler.clone()), |app| {
        begin(app, "bare-light.dng");
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            app.pump();
            if starts.try_recv().is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "reference source did not reach unpack"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        app.update(Editor::finish_flat_edit);
        app.pump();
        begin(app, "bare-light.dng");
        app.pump();
        assert_eq!(count.load(Ordering::Relaxed), 1);
        release.send(()).unwrap();
        wait(app);
        app.read(|e| {
            assert!(e.document.flat_edit().unwrap().view.ready.is_some());
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(e.document.state.project.flat_field().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn reference_session_keeps_its_prepared_input_when_the_display_visit_retires() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        begin(app, "bare-light.dng");
        wait(app);
        app.update(|e| {
            let visit = e.request_id();
            e.id = visit;
            e.source_epoch.store(visit, Ordering::Relaxed);
            e.replace_photo_scope(visit);
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(e.document.flat_edit().unwrap().view.ready.is_some());
        });
        apply(app);
        assert!(app.read(|e| e.document.flat_correction.is_some()));
        wait(app);
        app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            assert!(e.document.state.project.order.is_empty());
            assert_eq!(e.document.state.project.sources.len(), 1);
            assert!(e.document.state.project.sources[0].reference_only);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn accepted_correction_completes_after_the_display_visit_retires() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (before, pixels, viewport, source) = app.update(|e| {
            e.document.visit.canvas.viewport.zoom = Some(1.375);
            (
                e.history_snapshot(),
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .clone(),
                e.document.visit.canvas.viewport,
                e.document.source,
            )
        });
        begin(app, "bare-light.dng");
        wait(app);
        app.read(|e| {
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ))
        });
        let hold = crate::background::testing::pause();
        apply(app);
        let publication = app.read(|e| e.document.document_preview.clone());
        app.pump();
        app.update(|e| {
            e.id = e.request_id();
            e.source_epoch.store(e.id, Ordering::Relaxed);
            e.replace_photo_scope(e.id);
            e.finish_flat_edit();
            assert!(publication.active());
            assert!(e.interaction_blocked() && !e.has_hosted_tool());
            assert_eq!(e.document.project, before);
        });
        drop(hold);
        wait(app);
        app.read(|e| {
            assert!(e.document.flat_edit().is_none() && e.document_pending.is_none());
            assert!(!e.interaction_blocked() && e.activities.view().is_none());
            assert!(e.document.correction.is_some(), "{}", e.status);
            assert_eq!(e.document.state.project.order, before.order);
            assert_eq!(e.document.state.project.dmin(), before.dmin());
            assert_eq!(e.document.visit.canvas.viewport, viewport);
            assert_eq!(e.document.source, source);
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Correction")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn reference_failure_after_a_visit_change_finishes_its_own_preparation_activity() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (before, pixels, viewport, source) = app.read(|e| {
            (
                e.document.state.project.clone(),
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .clone(),
                e.document.visit.canvas.viewport,
                e.document.source,
            )
        });
        begin(app, "missing-reference.dng");
        app.read(|e| {
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ))
        });
        app.update(|e| {
            e.id = e.request_id();
            e.source_epoch.store(e.id, Ordering::Relaxed);
            e.replace_photo_scope(e.id);
        });
        wait(app);
        app.read(|e| {
            assert!(e.document.flat_edit().is_none() && e.activities.view().is_none());
            assert_eq!(
                e.dialogs.message().unwrap().title,
                FailureCase::FlatReference.title()
            );
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.document.visit.canvas.viewport, viewport);
            assert_eq!(e.document.source, source);
            assert!(e.document.visit.canvas.ready.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn invalid_measured_support_fails_without_accepting_reference_records_or_history() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        let before = app.read(|e| e.document.state.project.clone());
        begin(app, "bare-light.dng");
        wait(app);
        app.update(|e| {
            let area = e
                .document
                .flat_edit()
                .unwrap()
                .view
                .ready
                .as_ref()
                .unwrap()
                .area;
            e.set_test_pick_region(Some(crate::coverage::Coverage::rectangle(
                crate::raw::Rect {
                    x: area.x,
                    y: area.y,
                    width: 2,
                    height: 2,
                },
            )));
        });
        apply(app);
        assert!(
            app.read(|e| e.document.flat_correction.is_some()),
            "integer support reaches the actual estimator"
        );
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.state.project, before);
            assert!(e.document.flat_edit().is_none() && e.document.correction.is_none());
            assert_eq!(
                e.dialogs.message().unwrap().title,
                FailureCase::FlatCorrection.title()
            );
            assert!(
                e.history_label(crate::editor::history::Direction::Undo)
                    .is_none()
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn failed_replacement_preserves_the_previous_correction_and_adds_no_history_entry() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        let original = app.read(|editor| editor.document.project.clone());
        begin(app, "bare-light.dng");
        wait(app);
        apply(app);
        wait(app);
        let (accepted, correction) = app.read(|editor| {
            (
                editor.document.project.clone(),
                editor.document.correction.clone().unwrap(),
            )
        });
        begin(app, "bare-light.dng");
        wait(app);
        app.update(|editor| {
            let area = editor
                .document
                .flat_edit()
                .unwrap()
                .view
                .ready
                .as_ref()
                .unwrap()
                .area;
            editor.set_test_pick_region(Some(crate::coverage::Coverage::rectangle(
                crate::raw::Rect {
                    x: area.x,
                    y: area.y,
                    width: 2,
                    height: 2,
                },
            )));
        });
        apply(app);
        app.read(|editor| {
            assert!(!editor.has_hosted_tool() && editor.document.flat_correction.is_some());
            assert_eq!(editor.document.project, accepted);
            assert!(Arc::ptr_eq(
                editor.document.correction.as_ref().unwrap(),
                &correction
            ));
        });
        wait(app);
        app.read(|editor| {
            assert_eq!(
                editor.dialogs.message().unwrap().title,
                FailureCase::FlatCorrection.title()
            );
            assert_eq!(editor.document.project, accepted);
            assert!(Arc::ptr_eq(
                editor.document.correction.as_ref().unwrap(),
                &correction
            ));
        });
        let context = egui::Context::default();
        frame(app, &context, vec![]);
        frame(app, &context, key(egui::Key::Escape));
        app.read(|editor| {
            assert!(
                !editor.dialogs.is_open(),
                "acknowledge the rejected replacement before Undo"
            )
        });
        app.enter(|editor, cx| editor.restore_history(crate::editor::history::Direction::Undo, cx));
        assert!(
            app.read(|editor| editor.document_pending.is_some()),
            "Undo must actually start"
        );
        wait(app);
        app.read(|editor| {
            assert_eq!(editor.document.project, original);
            assert!(editor.document.correction.is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn missing_reference_services_finish_activity_and_preserve_accepted_state() {
    let mut e = fixture();
    crate::editor::tests::add_ready_fixture(&mut e);
    let pixels = e
        .document
        .visit
        .canvas
        .ready
        .as_ref()
        .unwrap()
        .developed
        .clone();
    with_application(e, |app| {
        let before = app.update(|e| e.history_snapshot());
        begin(app, "bare-light.dng");
        assert!(
            app.read(|e| e.interaction_blocked() && e.document.flat_edit().unwrap().task.is_some())
        );
        wait(app);
        app.read(|e| {
            assert!(!e.interaction_blocked() && e.activities.view().is_none());
            assert!(e.document.flat_edit().is_none());
            assert_eq!(e.document.state.project, before);
            assert_eq!(
                e.dialogs.message().unwrap().title,
                FailureCase::FlatReference.title()
            );
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ));
        });
    });
}

#[test]
fn ending_setup_while_waiting_for_support_leaves_the_working_view_and_allows_a_new_setup() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (before, pixels, viewport, source) = app.update(|e| {
            e.document.visit.canvas.viewport.zoom = Some(1.375);
            (
                e.history_snapshot(),
                e.document
                    .visit
                    .canvas
                    .ready
                    .as_ref()
                    .unwrap()
                    .developed
                    .clone(),
                e.document.visit.canvas.viewport,
                e.document.source,
            )
        });
        begin(app, "bare-light.dng");
        wait(app);
        app.update(|e| {
            assert!(e.document.flat_edit().unwrap().view.ready.is_some());
            assert!(!e.interaction_blocked() && e.document_pending.is_none());
            e.finish_flat_edit();
        });
        app.pump();
        app.read(|e| {
            assert!(
                e.document.flat_edit().is_none()
                    && !e.interaction_blocked()
                    && e.activities.view().is_none()
            );
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.document.visit.canvas.viewport, viewport);
            assert_eq!(e.document.source, source);
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.canvas.ready.as_ref().unwrap().developed
            ));
        });
        begin(app, "bare-light.dng");
        wait(app);
        apply(app);
        wait(app);
        app.read(|e| assert!(e.document.correction.is_some(), "{}", e.status));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retired_setup_cleanup_does_not_end_a_new_reference_interaction() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let before = app.read(|e| e.document.state.project.clone());
        begin(app, "bare-light.dng");
        wait(app);
        app.enter(|e, cx| {
            e.finish_flat_edit();
            e.begin_flat_edit(path("bare-light.dng"), None, cx);
        });
        wait(app);
        app.read(|e| {
            let edit = e
                .document
                .flat_edit()
                .expect("replacement setup remains active");
            assert!(edit.view.ready.is_some());
            assert!(!e.interaction_blocked() && e.document_pending.is_none());
            assert!(e.test_pick_region().is_some());
            assert_eq!(e.document.state.project, before);
        });
        apply(app);
        wait(app);
        app.read(|e| assert!(e.document.correction.is_some(), "{}", e.status));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn correction_uses_the_prepared_reference_even_if_its_file_disappears_before_apply() {
    let directory = tempfile::tempdir().unwrap();
    let reference = directory.path().join("captured-light.dng");
    std::fs::copy(path("bare-light.dng"), &reference).unwrap();
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        app.enter(|e, cx| e.begin_flat_edit(reference.clone(), None, cx));
        wait(app);
        app.read(|e| assert!(e.test_pick_region().is_some()));
        std::fs::remove_file(&reference).unwrap();
        apply(app);
        wait(app);
        app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            let settings = e.document.state.project.flat_field().unwrap();
            let record = e.document.state.project.source(settings.reference).unwrap();
            assert_eq!(record.path, reference);
            assert!(record.reference_only);
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Correction")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn correction_estimation_toggle_and_removal_do_not_load_an_unavailable_selected_photo() {
    let scheduler = source_loading::SourceLoader::new();
    let directory = tempfile::tempdir().unwrap();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (selected, visit) = app.enter(|e, cx| {
            let selected = e.document.visit.current_source.unwrap();
            let missing = directory.path().join("missing-selected.dng");
            e.document.project.source_mut(selected).unwrap().path = missing.clone();
            e.load(missing, cx);
            (selected, e.document.visit.view)
        });
        app.run_until(Duration::from_secs(20), |e| {
            !e.document.visit.source.pending()
        });
        app.read(|e| {
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(e.document.project.flat_field().is_none());
        });
        begin(app, "bare-light.dng");
        wait(app);
        apply(app);
        wait(app);
        let accepted = app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            assert!(e.document.project.flat_field().unwrap().enabled);
            e.document.project.clone()
        });
        for enabled in [false, true] {
            app.enter(|e, cx| e.change_correction(Some(enabled), cx));
            wait(app);
            app.read(|e| {
                assert_eq!(e.document.project.flat_field().unwrap().enabled, enabled);
                assert_eq!(e.document.correction.is_some(), enabled, "{}", e.status);
                assert!(e.document.visit.canvas.ready.is_none());
                assert!(!e.document.visit.source.pending());
                assert_eq!(e.document.visit.current_source, Some(selected));
                assert_eq!(e.document.visit.view, visit);
            });
        }
        app.enter(|e, cx| e.change_correction(None, cx));
        wait(app);
        app.read(|e| {
            let mut cleared = accepted;
            cleared.set_flat_field(None);
            assert_eq!(e.document.project, cleared, "{}", e.status);
            assert!(e.document.correction.is_none());
            assert!(e.document.visit.canvas.ready.is_none());
            assert!(!e.document.visit.source.pending());
            assert_eq!(e.document.visit.current_source, Some(selected));
            assert_eq!(e.document.visit.view, visit);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn each_correction_change_is_one_exact_undo_and_redo() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let mut versions = vec![app.read(|e| e.document.project.clone())];
        begin(app, "bare-light.dng");
        wait(app);
        apply(app);
        wait(app);
        versions.push(app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            e.document.project.clone()
        }));
        for enabled in [Some(false), Some(true), None] {
            app.enter(|e, cx| e.change_correction(enabled, cx));
            wait(app);
            versions.push(app.read(|e| e.document.project.clone()));
        }
        for expected in versions[..versions.len() - 1].iter().rev() {
            app.enter(|e, cx| e.restore_history(history::Direction::Undo, cx));
            wait(app);
            app.read(|e| {
                assert_eq!(&e.document.project, expected, "{}", e.status);
                assert_eq!(
                    e.document.correction.is_some(),
                    expected.active_flat_field().is_some()
                );
            });
        }
        app.read(|e| {
            assert_eq!(
                e.history_label(history::Direction::Undo),
                Some("Import photos")
            )
        });
        for expected in &versions[1..] {
            app.enter(|e, cx| e.restore_history(history::Direction::Redo, cx));
            wait(app);
            app.read(|e| {
                assert_eq!(&e.document.project, expected, "{}", e.status);
                assert_eq!(
                    e.document.correction.is_some(),
                    expected.active_flat_field().is_some()
                );
            });
        }
        app.read(|e| assert!(e.history_label(history::Direction::Redo).is_none()));
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn apply_captures_support_before_the_correction_task_runs() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        begin(app, "bare-light.dng");
        wait(app);
        let chosen = app.enter(|e, cx| {
            let area = e
                .document
                .flat_edit()
                .unwrap()
                .view
                .ready
                .as_ref()
                .unwrap()
                .area;
            let chosen = e.test_pick_region().unwrap().exact_rectangle(area).unwrap();
            e.apply_flat_edit(cx);
            let later = crate::raw::Rect {
                x: chosen.x + 8,
                y: chosen.y + 8,
                width: chosen.width - 16,
                height: chosen.height - 16,
            };
            assert_ne!(chosen, later);
            e.set_test_pick_region(Some(crate::coverage::Coverage::rectangle(later)));
            chosen
        });
        wait(app);
        app.read(|e| {
            let map = &e
                .document
                .correction
                .as_ref()
                .expect("correction accepted")
                .settings
                .map;
            assert_eq!(
                map.area(),
                [chosen.x, chosen.y, chosen.width, chosen.height]
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn correction_after_reference_selection_does_not_require_source_services() {
    let scheduler = source_loading::SourceLoader::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        begin(app, "bare-light.dng");
        wait(app);
        app.update(|e| e.source_services = None);
        apply(app);
        wait(app);
        app.read(|e| {
            assert!(e.document.correction.is_some(), "{}", e.status);
            assert!(e.document.flat_edit().is_none());
            assert_eq!(
                e.history_label(crate::editor::history::Direction::Undo),
                Some("Correction")
            );
        });
    });
    scheduler.close_and_wait().unwrap();
}

fn frame(
    app: &TestApplication<'_, Editor>,
    context: &egui::Context,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    context.enable_accesskit();
    let cx = app.access();
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| app.update(|e| e.draw_ui(ui, &cx)));
        },
    );
    output.textures_delta.clear();
    output
}

fn key(key: egui::Key) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]
}

fn click(app: &TestApplication<'_, Editor>, context: &egui::Context, label: &str) {
    let output = frame(app, context, vec![]);
    let nodes = output.platform_output.accesskit_update.unwrap().nodes;
    let action = &nodes
        .iter()
        .find(|(_, node)| {
            node.role() == egui::accesskit::Role::Button && node.label() == Some(label)
        })
        .unwrap()
        .1;
    assert!(!action.is_disabled());
    let bounds = action.bounds().unwrap();
    let pos = egui::pos2(
        ((bounds.x0 + bounds.x1) * 0.5) as f32,
        ((bounds.y0 + bounds.y1) * 0.5) as f32,
    );
    for pressed in [true, false] {
        frame(
            app,
            context,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
}

#[test]
fn reference_button_and_enter_accept_and_return_without_post_draw_launchers_or_another_paint() {
    for enter in [false, true] {
        let scheduler = source_loading::SourceLoader::new();
        with_application(editor(scheduler.clone()), |app| {
            import(app);
            let context = egui::Context::default();
            for _ in 0..4 {
                frame(app, &context, vec![]);
            }
            let (before, selected, viewport, source) = app.update(|e| {
                e.document.visit.canvas.viewport.zoom = Some(1.375);
                (
                    e.history_snapshot(),
                    e.document.visit.current_source,
                    e.document.visit.canvas.viewport,
                    e.document.source,
                )
            });
            // Supply the chooser's path through the real admitted control action.
            app.enter(|e, cx| FlatTool::open(e, path("bare-light.dng"), None, cx));
            wait(app);
            let region = app.update(|e| {
                let region = e.test_pick_region().expect("proposed reference support");
                e.set_test_pick_region(None);
                region
            });
            let disabled = frame(app, &context, vec![])
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes;
            assert!(
                disabled
                    .iter()
                    .find(|(_, node)| node.label() == Some("Apply"))
                    .unwrap()
                    .1
                    .is_disabled()
            );
            frame(app, &context, key(egui::Key::Enter));
            assert!(!app.read(|e| e.interaction_blocked()));
            app.update(|e| e.set_test_pick_region(Some(region)));
            if enter {
                frame(app, &context, key(egui::Key::Enter));
            } else {
                click(app, &context, "Apply");
            }
            app.read(|e| {
                assert!(e.interaction_blocked() && e.document.flat_correction.is_some());
                assert!(!e.has_hosted_tool() && e.document.flat_edit().is_none());
                assert_eq!(e.document.state.project, before);
            });
            let busy = frame(app, &context, vec![])
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes;
            assert!(
                !busy
                    .iter()
                    .any(|(_, node)| node.role() == egui::accesskit::Role::Button
                        && matches!(node.label(), Some("Cancel" | "Apply"))),
                "accepted work has no remaining tool actions"
            );
            frame(app, &context, key(egui::Key::Escape));
            assert!(app.read(|e| e.document.flat_edit().is_none()
                && e.interaction_blocked()
                && e.document.flat_correction.is_some()));
            // The document operation completes without another UI frame.
            wait(app);
            app.read(|e| {
                assert!(e.document.correction.is_some(), "{}", e.status);
                assert!(e.document.flat_edit().is_none() && e.document_pending.is_none());
                assert_eq!(e.document.visit.current_source, selected);
                assert_eq!(e.document.visit.canvas.viewport, viewport);
                assert_eq!(e.document.source, source);
                assert_eq!(e.document.state.project.order, before.order);
                assert_eq!(
                    (
                        e.document.state.project.dmin(),
                        e.document.state.project.dmax()
                    ),
                    (before.dmin(), before.dmax())
                );
                assert_eq!(
                    e.document.visit.canvas.ready.as_ref().unwrap().path,
                    path("illuminated-patches.dng")
                );
                assert!(
                    e.history_label(crate::editor::history::Direction::Undo)
                        .is_some()
                );
            });
            let accepted = app.read(|e| e.document.state.project.clone());
            begin(app, "bare-light.dng");
            wait(app);
            if enter {
                frame(app, &context, key(egui::Key::Escape));
            } else {
                click(app, &context, "Cancel");
            }
            app.read(|e| {
                assert!(e.document.flat_edit().is_none() && !e.interaction_blocked());
                assert_eq!(e.document.state.project, accepted);
                assert_eq!(e.document.visit.current_source, selected);
                assert_eq!(e.document.visit.canvas.viewport, viewport);
                assert_eq!(e.document.source, source);
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn unpolled_preparation_can_be_retired_and_closed_execution_rejects_new_work() {
    for accept in [false, true] {
        let scheduler = source_loading::SourceLoader::new();
        with_application(editor(scheduler.clone()), |app| {
            let before = app.read(|e| e.document.state.project.clone());
            if accept {
                begin(app, "bare-light.dng");
                wait(app);
            }
            if accept {
                apply(app);
            } else {
                begin(app, "bare-light.dng");
            }
            if accept {
                assert!(
                    app.read(|e| e.document.flat_edit().is_none()
                        && e.document.flat_correction.is_some())
                );
                app.enter(|e, cx| {
                    e.start_document(
                        crate::editor::Preparation::Open(path("missing.antitone")),
                        cx,
                    )
                });
            } else {
                assert!(app.read(|e| e.document.flat_edit().unwrap().task.is_some()));
                app.update(Editor::finish_flat_edit);
            }
            wait(app);
            app.read(|e| {
                assert!(
                    e.document.flat_edit().is_none()
                        && !e.interaction_blocked()
                        && e.document_pending.is_none()
                );
                assert_eq!(e.document.state.project, before);
            });
            if accept {
                begin(app, "bare-light.dng");
                wait(app);
            }
            let status = app.read(|e| e.status.clone());
            app.close_execution();
            if accept {
                apply(app);
            } else {
                begin(app, "bare-light.dng");
            }
            app.read(|e| {
                assert!(
                    e.document.flat_edit().is_none()
                        && !e.interaction_blocked()
                        && e.document_pending.is_none()
                );
                assert_eq!(e.document.state.project, before);
                assert!(!e.editing() && e.document.flat_correction.is_none());
                assert!(
                    !e.dialogs.is_open(),
                    "shutdown must not create an error notification"
                );
                assert_eq!(e.status, status);
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}
