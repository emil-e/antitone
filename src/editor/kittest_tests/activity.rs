use super::*;

fn held_activity(editor: &mut Editor, task: Task) {
    editor.tasks.begin(task);
    editor.tasks.elapse(Duration::from_secs(1));
    editor.document_pending = match task.request() {
        TaskRequest::Document(id) => Some(id),
        _ => None,
    };
    editor.busy = true;
}

fn render_case(harness: &mut Harness<'_, Editor>, name: &str) {
    harness.hover_at(egui::pos2(-50., -50.));
    for _ in 0..3 {
        harness.step();
    }
    // Inspect a legible animation phase rather than a nearly closed spinner arc.
    for _ in 0..180 {
        if harness.ctx.input(|input| input.time.sin().abs()) > 0.7 {
            break;
        }
        harness.step();
    }
    let png = harness.render().unwrap();
    save_capture(&png, &format!("working-{name}-offscreen"));
}

#[test]
fn working_feedback_gallery_preserves_available_interactions_and_exposes_every_style() {
    editor_harness(|mut harness, _capture| {
        harness.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| assert!(editor.document.visit.ready.is_some()));
        harness.update(|editor| editor.sync_document());
        let before = harness.read(|editor| editor.document.project.clone());
        for (layout, size) in [
            ("normal", egui::vec2(1100., 800.)),
            ("narrow", egui::vec2(800., 600.)),
        ] {
            harness.set_size(size);
            let id = harness.read(|editor| editor.id);
            for (name, task) in [
                ("import", Task::Import(id)),
                ("open", Task::Open(id)),
                ("import-preparing", Task::Import(id)),
                ("open-preparing", Task::Open(id)),
                ("save", Task::Save(id)),
                ("correction", Task::Correction(id)),
            ] {
                harness.update(|editor| held_activity(editor, task));
                if name == "import" || name == "open" {
                    harness.update(|editor| {
                        editor.tasks.report(
                            task.request(),
                            TaskProgress::Counted {
                                completed: 3,
                                total: 12,
                            },
                        )
                    });
                }
                render_case(&mut harness, &format!("{name}-{layout}"));
                assert!(
                    harness
                        .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                        .accesskit_node()
                        .is_disabled()
                );
                let title = harness.read(|editor| editor.tasks.view().unwrap().title);
                harness.event(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
                harness.step();
                harness.read(|editor| assert_eq!(editor.tasks.view().unwrap().title, title));
                harness.update(|editor| editor.tasks.finish(task.request()));
                harness.update(|editor| editor.document_pending = None);
                harness.update(|editor| editor.busy = false);
                harness.step();
            }
            {
                harness.update(|editor| held_activity(editor, Task::Export(id)));
                render_case(&mut harness, &format!("export-{layout}"));
                let title = harness.read(|editor| editor.tasks.view().unwrap().title);
                let progress = harness
                    .get_by_role_and_label(egui::accesskit::Role::ProgressIndicator, &title)
                    .rect();
                assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(progress));
                assert!(
                    harness
                        .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                        .accesskit_node()
                        .is_disabled()
                );
                assert!(
                    !harness
                        .get_by_label("Cancel export")
                        .accesskit_node()
                        .is_disabled()
                );
                let old_tab = harness.read(|editor| editor.tab);
                harness
                    .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                    .click();
                harness.step();
                harness.read(|editor| assert_eq!(editor.tab, old_tab));
                let old_view = harness.read(|editor| editor.viewport);
                let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
                let point = geometry.image.center_bottom() - egui::vec2(0., 10.);
                harness.hover_at(point);
                for pressed in [true, false] {
                    harness.event(egui::Event::PointerButton {
                        pos: point,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    });
                    harness.step();
                }
                harness.read(|editor| assert_eq!(editor.viewport, old_view));
                harness.key_press(egui::Key::Escape);
                harness.step();
                harness.read(|editor| assert!(editor.tasks.blocks_workspace()));
                harness.update(|editor| editor.tasks.finish(TaskRequest::Export(id)));
                harness.update(|editor| editor.busy = false);
                harness.step();
                stage(&mut harness, "Roll");
            }
            for (name, task, pick) in [
                ("raw", Task::Photo(id), None),
                ("reference", Task::Reference(id), Some(Pick::Flat)),
                (
                    "measurement",
                    Task::Measurement(id, Pick::Dmin),
                    Some(Pick::Dmin),
                ),
            ] {
                harness.update(|editor| held_activity(editor, task));
                harness.update(|editor| editor.pick = pick);
                if name == "measurement" {
                    harness.update(|editor| {
                        editor.region = Some(Coverage::rectangle(Rect {
                            x: 32,
                            y: 32,
                            width: 64,
                            height: 64,
                        }))
                    });
                }
                let ready = if name == "raw" || name == "reference" {
                    harness.update(|editor| editor.document.visit.ready.take())
                } else {
                    None
                };
                render_case(&mut harness, &format!("{name}-{layout}"));
                if let Some(ready) = ready {
                    harness.update(|editor| editor.document.visit.ready = Some(ready));
                }
                harness.update(|editor| editor.pick = None);
                harness.update(|editor| editor.region = None);
                harness.update(|editor| editor.tasks.finish(task.request()));
                harness.update(|editor| editor.busy = false);
                harness.step();
            }
            let admission = crate::background::testing::pause();
            harness.enter(|editor, cx| editor.request_analysis(cx));
            harness.update(|editor| editor.tasks.elapse(Duration::from_secs(1)));
            render_case(&mut harness, &format!("regions-modal-{layout}"));
            harness.get_by_label("Cancel region preparation").click();
            harness.step();
            wait_for(&mut harness, |editor| {
                editor.document.spatial_preparing.is_none() && !editor.busy
            });
            drop(admission);
            let source = harness.read(|editor| editor.current_source.unwrap());
            let pending = harness.read(|editor| {
                super::super::analysis_workflow::Pending::new(
                    9000,
                    super::super::analysis::Request {
                        loaded: editor.id,
                        revision: editor
                            .document
                            .analysis_generation
                            .load(std::sync::atomic::Ordering::Relaxed),
                        document: editor.document.capture(),
                        selected: source,
                        local_support: false,
                        film: None,
                        picture: None,
                        areas: None,
                    },
                )
            });
            harness.update(|editor| editor.document.analysis_preparing = Some(pending));
            harness.update(|editor| {
                editor.tasks.begin(Task::AnalysisAreas(9000));
                editor.tasks.elapse(Duration::from_secs(1));
            });
            harness.update(|editor| {
                editor.tasks.report(
                    TaskRequest::Analyze(9000),
                    TaskProgress::Counted {
                        completed: 3,
                        total: 12,
                    },
                )
            });
            harness.update(|editor| editor.busy = true);
            render_case(&mut harness, &format!("analysis-sampling-{layout}"));
            let previous_stage = harness.read(|editor| editor.tab);
            harness
                .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                .click();
            harness.step();
            harness.read(|editor| {
                assert_eq!(
                    editor.tab, previous_stage,
                    "modal preparation intercepts background stage changes"
                )
            });
            render_case(&mut harness, &format!("analysis-modal-background-{layout}"));
            assert!(
                !harness
                    .get_by_label("Cancel analysis")
                    .accesskit_node()
                    .is_disabled()
            );
            harness.update(|editor| {
                editor.document.analysis_pending = editor.document.analysis_preparing.take();
                editor.tasks.begin(Task::Analyze(9000));
                editor.tasks.elapse(Duration::from_secs(1));
                editor.tasks.report(
                    TaskRequest::Analyze(9000),
                    TaskProgress::Counted {
                        completed: 12,
                        total: 12,
                    },
                );
            });
            render_case(&mut harness, &format!("analysis-fitting-{layout}"));
            harness.get_by_label("Cancel analysis").click();
            harness.step();
            render_case(&mut harness, &format!("analysis-cancelling-{layout}"));
            assert!(harness.query_by_label("Cancel analysis").is_none());
            harness.enter(|editor, cx| editor.analysis_result(9000, Ok(None), cx));
            render_case(&mut harness, &format!("analysis-cancelled-{layout}"));
            harness.read(|editor| assert!(editor.tasks.view().is_none()));
            harness.read(|editor| assert_eq!(editor.status, "Analysis cancelled"));
            stage(&mut harness, "Roll");
            render_case(&mut harness, &format!("background-{layout}"));
            harness.read(|editor| assert_eq!(editor.document.project, before));
            harness.read(|editor| assert!(editor.tasks.view().is_none()));
        }
    });
}
