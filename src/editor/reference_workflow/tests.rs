use super::*;
use crate::{
    application::test_support::{TestApplication, with_application},
    editor::{development, document::Document, source, tests::fixture},
};
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

fn editor(scheduler: development::Scheduler) -> Editor {
    let mut e = fixture();
    e.document = Document::default().into();
    e.set_dmin(None);
    e.set_dmax(None);
    e.set_floor(crate::processing::CAMERA_INPUT_FLOOR);
    e.sync_document();
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
    app.run_until(Duration::from_secs(20), |e| !e.busy);
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
    let scheduler = development::Scheduler::new_observed(move |file, phase, _| {
        if file == path("bare-light.dng") && matches!(phase, development::NativePhase::UnpackBegin)
        {
            observed.fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    });
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (before, visit) = app.update(|e| {
            assert!(e.document.visit.ready.is_some(), "{}", e.status);
            e.viewport.zoom = Some(1.375);
            (e.history_snapshot(), e.document.visit.view)
        });
        begin(app, "bare-light.dng");
        assert!(app.read(|e| e.flat_edit.as_ref().unwrap().task.is_some()));
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.region.is_some());
            assert!(e.source_context().unwrap().correction.is_none());
            assert_ne!(e.document.visit.view, visit);
        });
        apply(app);
        wait(app);
        app.read(|e| {
            assert!(e.correction.is_some(), "{}", e.status);
            assert!(e.flat_edit.is_none() && e.document_pending.is_none());
            assert_eq!(e.document.project.order, before.order);
            assert_eq!(e.viewport.zoom, Some(1.375));
            assert_eq!(
                (e.document.project.dmin(), e.document.project.dmax()),
                (before.dmin(), before.dmax())
            );
            assert_eq!(
                e.document.visit.ready.as_ref().unwrap().path,
                path("illuminated-patches.dng")
            );
            assert_ne!(e.document.visit.view, visit);
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
        let accepted = app.read(|e| e.document.project.clone());
        let current = app.read(|e| e.document.visit.ready.as_ref().unwrap().developed.clone());
        begin(app, "missing-reference.dng");
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project, accepted);
            assert!(Arc::ptr_eq(
                &current,
                &e.document.visit.ready.as_ref().unwrap().developed
            ));
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::FlatReference);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn retiring_queued_reference_stages_restores_context_and_cannot_finish_a_newer_operation() {
    for accept in [false, true] {
        let scheduler = development::Scheduler::new();
        with_application(editor(scheduler.clone()), |app| {
            let before = app.read(|e| e.document.project.clone());
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
            let old = app.read(|e| e.id);
            app.pump();
            app.update(Editor::finish_flat_edit);
            app.pump();
            app.read(|e| {
                assert_eq!(e.document.project, before);
                assert!(!e.busy && e.document_pending.is_none());
                assert!(e.flat_edit.is_none());
            });
            app.enter(|e, cx| {
                e.start_document(crate::editor::Preparation::New, cx);
            });
            let operation = app.read(|e| e.document_pending);
            app.update(|e| {
                e.reference_previewed(old, Err("retired".into()));
                e.document_prepared(old, Err("retired".into()));
            });
            app.read(|e| assert_eq!(e.document_pending, operation));
            drop(hold);
            wait(app);
            app.read(|e| {
                assert!(e.failure.is_none());
                assert!(e.document.project.flat_field().is_none());
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn failed_document_supersession_retires_reference_acceptance_but_preserves_its_return_context() {
    let scheduler = development::Scheduler::new();
    with_application(editor(scheduler.clone()), |app| {
        import(app);
        let (before, pixels, visit) = app.update(|e| {
            (
                e.history_snapshot(),
                e.document.visit.ready.as_ref().unwrap().developed.clone(),
                e.document.visit.view,
            )
        });
        begin(app, "bare-light.dng");
        wait(app);
        let hold = crate::background::testing::pause();
        apply(app);
        let old = app.read(|e| e.id);
        app.pump();
        app.enter(|e, cx| {
            e.start_document(
                crate::editor::Preparation::Open(path("missing-project.antitone")),
                cx,
            );
        });
        let operation = app.read(|e| {
            let session = e.flat_edit.as_ref().unwrap();
            assert!(session.task.is_none() && session.working.is_some());
            e.document_pending.unwrap()
        });
        assert_ne!(old, operation);
        app.update(|e| e.document_prepared(old, Err("retired reference".into())));
        drop(hold);
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.flat_edit.is_none() && e.correction.is_none());
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.ready.as_ref().unwrap().developed
            ));
            assert_ne!(e.document.visit.view, visit);
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
    let scheduler = development::Scheduler::new_observed(move |_, phase, _| {
        if matches!(phase, development::NativePhase::UnpackBegin)
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
        begin(app, "bare-light.dng");
        app.pump();
        assert_eq!(count.load(Ordering::Relaxed), 1);
        release.send(()).unwrap();
        wait(app);
        app.read(|e| {
            assert!(e.document.visit.ready.is_some());
            assert!(e.document.project.flat_field().is_none());
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn reference_session_keeps_its_prepared_input_when_the_display_visit_retires() {
    let scheduler = development::Scheduler::new();
    with_application(editor(scheduler.clone()), |app| {
        begin(app, "bare-light.dng");
        wait(app);
        app.update(|e| {
            let visit = e.request_id();
            e.replace_photo_scope(visit);
            assert!(e.document.visit.ready.is_none());
            assert!(e.flat_edit.as_ref().unwrap().reference.is_some());
        });
        apply(app);
        assert!(app.read(|e| e.document_pending.is_some()));
        wait(app);
        app.read(|e| {
            assert!(e.correction.is_some(), "{}", e.status);
            assert!(e.document.project.order.is_empty());
            assert_eq!(e.document.project.sources.len(), 1);
            assert!(e.document.project.sources[0].reference_only);
        });
    });
    scheduler.close_and_wait().unwrap();
}

#[test]
fn invalid_measured_support_fails_without_accepting_reference_records_or_history() {
    let scheduler = development::Scheduler::new();
    with_application(editor(scheduler.clone()), |app| {
        let before = app.read(|e| e.document.project.clone());
        begin(app, "bare-light.dng");
        wait(app);
        app.update(|e| {
            let area = e.document.visit.ready.as_ref().unwrap().area;
            e.region = Some(crate::coverage::Coverage::rectangle(crate::raw::Rect {
                x: area.x,
                y: area.y,
                width: 2,
                height: 2,
            }));
        });
        apply(app);
        assert!(
            app.read(|e| e.document_pending.is_some()),
            "integer support reaches the actual estimator"
        );
        wait(app);
        app.read(|e| {
            assert_eq!(e.document.project, before);
            assert!(e.flat_edit.is_none() && e.correction.is_none());
            assert_eq!(
                e.failure.as_ref().unwrap().kind,
                FailureKind::FlatCorrection
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
fn missing_reference_services_finish_activity_and_preserve_accepted_state() {
    let mut e = fixture();
    crate::editor::tests::add_ready_fixture(&mut e);
    let pixels = e.document.visit.ready.as_ref().unwrap().developed.clone();
    with_application(e, |app| {
        let before = app.update(|e| e.history_snapshot());
        begin(app, "bare-light.dng");
        assert!(app.read(|e| e.busy && e.flat_edit.as_ref().unwrap().task.is_some()));
        wait(app);
        app.read(|e| {
            assert!(!e.busy && e.tasks.view().is_none());
            assert!(e.flat_edit.is_none());
            assert_eq!(e.document.project, before);
            assert_eq!(e.failure.as_ref().unwrap().kind, FailureKind::FlatReference);
            assert!(Arc::ptr_eq(
                &pixels,
                &e.document.visit.ready.as_ref().unwrap().developed
            ));
        });
    });
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
        let scheduler = development::Scheduler::new();
        with_application(editor(scheduler.clone()), |app| {
            import(app);
            let context = egui::Context::default();
            for _ in 0..4 {
                frame(app, &context, vec![]);
            }
            let (before, selected, viewport, source) = app.update(|e| {
                e.viewport.zoom = Some(1.375);
                (e.history_snapshot(), e.current_source, e.viewport, e.source)
            });
            // Supply the chooser's path through the real admitted control action.
            app.enter(|e, cx| {
                e.control_command(
                    super::super::editing::Command::FlatEdit(path("bare-light.dng"), None),
                    cx,
                )
            });
            wait(app);
            let region = app.update(|e| e.region.take().expect("proposed reference support"));
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
            assert!(!app.read(|e| e.busy));
            app.update(|e| e.region = Some(region));
            if enter {
                frame(app, &context, key(egui::Key::Enter));
            } else {
                click(app, &context, "Apply");
            }
            app.read(|e| {
                assert!(e.busy && e.document_pending.is_some());
                assert!(e.flat_edit.as_ref().unwrap().task.is_some());
                assert_eq!(e.document.project, before);
            });
            let busy = frame(app, &context, vec![])
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes;
            for label in ["Cancel", "Apply"] {
                assert!(
                    busy.iter()
                        .find(|(_, node)| node.label() == Some(label))
                        .unwrap()
                        .1
                        .is_disabled()
                );
            }
            frame(app, &context, key(egui::Key::Escape));
            assert!(app.read(|e| e.flat_edit.is_some() && e.busy));
            // The first executor turn after Apply completes all service stages
            // and the return transaction without invoking UI or a starter list.
            wait(app);
            app.read(|e| {
                assert!(e.correction.is_some(), "{}", e.status);
                assert!(e.flat_edit.is_none() && e.document_pending.is_none());
                assert_eq!(e.current_source, selected);
                assert_eq!(e.viewport, viewport);
                assert_eq!(e.source, source);
                assert_eq!(e.document.project.order, before.order);
                assert_eq!(
                    (e.document.project.dmin(), e.document.project.dmax()),
                    (before.dmin(), before.dmax())
                );
                assert_eq!(
                    e.document.visit.ready.as_ref().unwrap().path,
                    path("illuminated-patches.dng")
                );
                assert!(
                    e.history_label(crate::editor::history::Direction::Undo)
                        .is_some()
                );
            });
            let accepted = app.read(|e| e.document.project.clone());
            begin(app, "bare-light.dng");
            wait(app);
            if enter {
                frame(app, &context, key(egui::Key::Escape));
            } else {
                click(app, &context, "Cancel");
            }
            app.read(|e| {
                assert!(e.flat_edit.is_none() && !e.busy);
                assert_eq!(e.document.project, accepted);
                assert_eq!(e.current_source, selected);
                assert_eq!(e.viewport, viewport);
                assert_eq!(e.source, source);
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}

#[test]
fn unpolled_reference_stages_and_closed_execution_never_publish_a_correction() {
    for accept in [false, true] {
        let scheduler = development::Scheduler::new();
        with_application(editor(scheduler.clone()), |app| {
            let before = app.read(|e| e.document.project.clone());
            if accept {
                begin(app, "bare-light.dng");
                wait(app);
            }
            if accept {
                apply(app);
            } else {
                begin(app, "bare-light.dng");
            }
            assert!(app.read(|e| e.flat_edit.as_ref().unwrap().task.is_some()));
            app.update(Editor::finish_flat_edit);
            app.pump();
            app.read(|e| {
                assert!(e.flat_edit.is_none() && !e.busy && e.document_pending.is_none());
                assert_eq!(e.document.project, before);
            });
            if accept {
                begin(app, "bare-light.dng");
                wait(app);
            }
            app.close_execution();
            if accept {
                apply(app);
            } else {
                begin(app, "bare-light.dng");
            }
            app.read(|e| {
                assert!(e.flat_edit.is_none() && !e.busy && e.document_pending.is_none());
                assert_eq!(e.document.project, before);
                assert_eq!(
                    e.failure.as_ref().unwrap().kind,
                    if accept {
                        FailureKind::FlatCorrection
                    } else {
                        FailureKind::FlatReference
                    }
                );
            });
        });
        scheduler.close_and_wait().unwrap();
    }
}
