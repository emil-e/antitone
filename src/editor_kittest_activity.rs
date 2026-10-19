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
    let (mut harness, _capture) = editor_harness();
    harness.state_mut().add_raws(vec![fixture("patches.dng")]);
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    assert!(harness.state().ready.is_some());
    harness.state_mut().sync_document();
    let before = harness.state().document.project.clone();
    for (layout, size) in [
        ("normal", egui::vec2(1100., 800.)),
        ("narrow", egui::vec2(800., 600.)),
    ] {
        harness.set_size(size);
        let id = harness.state().id;
        for (name, task) in [
            ("import", Task::Import(id)),
            ("open", Task::Open(id)),
            ("import-preparing", Task::Import(id)),
            ("open-preparing", Task::Open(id)),
            ("save", Task::Save(id)),
            ("correction", Task::Correction(id)),
        ] {
            held_activity(harness.state_mut(), task);
            if name == "import" || name == "open" {
                harness.state_mut().tasks.report(
                    task.request(),
                    TaskProgress::Counted {
                        completed: 3,
                        total: 12,
                    },
                );
            }
            render_case(&mut harness, &format!("{name}-{layout}"));
            assert!(
                harness
                    .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                    .accesskit_node()
                    .is_disabled()
            );
            let title = harness.state().tasks.view().unwrap().title;
            harness.event(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            assert_eq!(harness.state().tasks.view().unwrap().title, title);
            harness.state_mut().tasks.finish(task.request());
            harness.state_mut().document_pending = None;
            harness.state_mut().busy = false;
            harness.step();
        }
        {
            held_activity(harness.state_mut(), Task::Export(id));
            render_case(&mut harness, &format!("export-{layout}"));
            let progress = harness
                .get_by_role_and_label(egui::accesskit::Role::ProgressIndicator, "Exporting…")
                .rect();
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(progress));
            assert!(
                !harness
                    .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                    .accesskit_node()
                    .is_disabled()
            );
            harness
                .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                .click();
            harness.step();
            assert_eq!(harness.state().tab, 1);
            let old_view = harness.state().viewport;
            let geometry = harness.state().last_layout.unwrap().0;
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
            assert_ne!(harness.state().viewport, old_view);
            harness.state_mut().tasks.finish(TaskRequest::Export(id));
            harness.state_mut().busy = false;
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
            held_activity(harness.state_mut(), task);
            harness.state_mut().pick = pick;
            if name == "measurement" {
                harness.state_mut().region = Some(Coverage::rectangle(Rect {
                    x: 32,
                    y: 32,
                    width: 64,
                    height: 64,
                }));
            }
            let ready = if name == "raw" || name == "reference" {
                harness.state_mut().ready.take()
            } else {
                None
            };
            render_case(&mut harness, &format!("{name}-{layout}"));
            if let Some(ready) = ready {
                harness.state_mut().ready = Some(ready);
            }
            harness.state_mut().pick = None;
            harness.state_mut().region = None;
            harness.state_mut().tasks.finish(task.request());
            harness.state_mut().busy = false;
            harness.step();
        }
        let source = harness.state().current_source.unwrap();
        let pending = super::super::analysis::Pending {
            id: 9000,
            request: super::super::analysis::Request {
                loaded: harness.state().id,
                revision: harness
                    .state()
                    .analysis_generation
                    .load(std::sync::atomic::Ordering::Relaxed),
                document: harness.state().document.clone(),
                selected: source,
                local_support: false,
                film: None,
                picture: None,
                areas: None,
            },
        };
        harness.state_mut().analysis_pending = Some(pending);
        harness.state_mut().tasks.begin(Task::Analyze(9000));
        harness.state_mut().tasks.report(
            TaskRequest::Analyze(9000),
            TaskProgress::Counted {
                completed: 3,
                total: 12,
            },
        );
        harness.state_mut().busy = true;
        render_case(&mut harness, &format!("analysis-sampling-{layout}"));
        harness
            .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
            .click();
        harness.step();
        render_case(&mut harness, &format!("analysis-other-stage-{layout}"));
        assert!(
            !harness
                .get_by_label("Cancel analysis")
                .accesskit_node()
                .is_disabled()
        );
        harness.state_mut().tasks.report(
            TaskRequest::Analyze(9000),
            TaskProgress::Counted {
                completed: 12,
                total: 12,
            },
        );
        render_case(&mut harness, &format!("analysis-fitting-{layout}"));
        harness.get_by_label("Cancel analysis").click();
        harness.step();
        render_case(&mut harness, &format!("analysis-cancelling-{layout}"));
        assert!(harness.query_by_label("Cancel analysis").is_none());
        harness.state_mut().analysis_result(9000, Ok(None));
        render_case(&mut harness, &format!("analysis-cancelled-{layout}"));
        assert!(harness.state().tasks.view().is_none());
        assert_eq!(harness.state().status, "Analysis cancelled");
        stage(&mut harness, "Roll");
        render_case(&mut harness, &format!("background-{layout}"));
        assert_eq!(harness.state().document.project, before);
        assert!(harness.state().tasks.view().is_none());
    }
}
