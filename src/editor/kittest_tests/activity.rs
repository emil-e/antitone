use super::*;

fn held_activity(editor: &mut Editor, options: ActivityOptions) -> Activity {
    let activity = editor.activities.start(options);
    editor.activities.elapse(Duration::from_secs(1));
    activity
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
            !editor.interaction_blocked() && editor.document_pending.is_none()
        });
        harness.read(|editor| assert!(editor.document.visit.canvas.ready.is_some()));
        harness.update(|editor| editor.sync_document());
        let before = harness.read(|editor| editor.document.state.project.clone());
        for (layout, size) in [
            ("normal", egui::vec2(1100., 800.)),
            ("narrow", egui::vec2(800., 600.)),
        ] {
            harness.set_size(size);
            for (name, title) in [
                ("import", "Importing RAWs…"),
                ("open", "Opening project…"),
                ("import-preparing", "Importing RAWs…"),
                ("open-preparing", "Opening project…"),
                ("save", "Saving project…"),
                ("correction", "Applying correction…"),
            ] {
                let activity = harness
                    .update(|editor| held_activity(editor, ActivityOptions::new(title).modal()));
                if name == "import" || name == "open" {
                    activity.progress(3, 12);
                }
                render_case(&mut harness, &format!("{name}-{layout}"));
                assert!(
                    harness
                        .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                        .accesskit_node()
                        .is_disabled()
                );
                harness.key_press(egui::Key::Escape);
                harness.step();
                harness.read(|editor| assert_eq!(editor.activities.view().unwrap().title, title));
                drop(activity);
                harness.step();
            }
            {
                let activity = harness.update(|editor| {
                    held_activity(
                        editor,
                        ActivityOptions::new("Exporting…").modal().cancellable(
                            "Cancel export",
                            "Cancelling export…",
                            crate::cancellation::Cancellation::default(),
                        ),
                    )
                });
                render_case(&mut harness, &format!("export-{layout}"));
                let progress = harness
                    .get_by_role_and_label(egui::accesskit::Role::ProgressIndicator, "Exporting…")
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
                let old_view = harness.read(|editor| editor.document.visit.canvas.viewport);
                let geometry = harness.read(|editor| editor.active_canvas().last_layout.unwrap().0);
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
                harness.read(|editor| assert_eq!(editor.document.visit.canvas.viewport, old_view));
                harness.key_press(egui::Key::Escape);
                harness.step();
                harness.read(|editor| assert!(editor.activities.blocks_workspace()));
                drop(activity);
                harness.step();
                stage(&mut harness, "Roll");
            }
            for (name, title) in [
                ("raw", "Loading photo…"),
                ("reference", "Preparing reference…"),
                ("measurement", "Measuring Dmin…"),
            ] {
                let activity = harness.update(|editor| {
                    if name == "reference" {
                        editor.document.set_flat_edit(Some(flat::FlatEdit {
                            services: None,
                            view: Default::default(),
                            region: None,
                            task: None,
                        }));
                    } else if name == "measurement" {
                        super::super::picking::preview(editor, PickerCase::Dmin.display());
                    }
                    let options = ActivityOptions::new(title);
                    held_activity(
                        editor,
                        if name == "raw" {
                            options
                        } else if name == "measurement" {
                            options.modal().cancellable(
                                "Cancel measurement",
                                "Cancelling measurement…",
                                crate::cancellation::Cancellation::default(),
                            )
                        } else {
                            options.modal()
                        },
                    )
                });
                if name == "measurement" {
                    harness.update(|editor| {
                        editor.set_test_pick_region(Some(Coverage::rectangle(Rect {
                            x: 32,
                            y: 32,
                            width: 64,
                            height: 64,
                        })))
                    });
                }
                let ready = if name == "raw" || name == "reference" {
                    harness.update(|editor| editor.document.visit.canvas.ready.take())
                } else {
                    None
                };
                render_case(&mut harness, &format!("{name}-{layout}"));
                if let Some(ready) = ready {
                    harness.update(|editor| editor.document.visit.canvas.ready = Some(ready));
                }
                harness.update(|editor| editor.end_tool());
                drop(activity);
                harness.step();
            }
            let admission = crate::background::testing::pause();
            harness.enter(|editor, cx| editor.request_analysis(cx));
            harness.update(|editor| editor.activities.elapse(Duration::from_secs(1)));
            render_case(&mut harness, &format!("regions-modal-{layout}"));
            harness.get_by_label("Cancel region preparation").click();
            harness.step();
            wait_for(&mut harness, |editor| {
                editor.document.spatial_preparing.is_none() && !editor.interaction_blocked()
            });
            drop(admission);
            // Presentation fixture; real worker acknowledgement has separate composed tests.
            let activity = harness.update(|editor| {
                let mut operation = super::super::analysis::Operation::new(
                    super::super::analysis::Phase::Preparing,
                );
                let activity = held_activity(
                    editor,
                    ActivityOptions::new("Preparing analysis areas…")
                        .modal()
                        .cancellable(
                            "Cancel analysis",
                            "Cancelling analysis…",
                            operation.cancelled.clone(),
                        ),
                );
                operation.activity = Some(activity.reporter());
                editor.document.set_analysis_operation(Some(operation));
                activity
            });
            activity.progress(3, 12);
            render_case(&mut harness, &format!("analysis-sampling-{layout}"));
            harness
                .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
                .click();
            harness.step();
            harness.read(|editor| {
                assert_eq!(
                    editor.tab, 0,
                    "analysis work blocks unrelated stage navigation"
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
                editor.document.analysis_operation_mut().unwrap().phase =
                    super::super::analysis::Phase::Fitting
            });
            activity.status("Analyzing roll…");
            activity.progress(12, 12);
            render_case(&mut harness, &format!("analysis-fitting-{layout}"));
            harness.get_by_label("Cancel analysis").click();
            harness.step();
            render_case(&mut harness, &format!("analysis-cancelling-{layout}"));
            assert!(harness.query_by_label("Cancel analysis").is_none());
            harness.update(|editor| {
                assert!(
                    editor
                        .document
                        .analysis_operation()
                        .unwrap()
                        .cancelled
                        .is_cancelled()
                );
                editor.document.set_analysis_operation(None);
                editor.status = "Analysis cancelled".into();
            });
            drop(activity);
            render_case(&mut harness, &format!("analysis-cancelled-{layout}"));
            harness.read(|editor| assert!(editor.activities.view().is_none()));
            harness.read(|editor| assert_eq!(editor.status, "Analysis cancelled"));
            stage(&mut harness, "Roll");
            render_case(&mut harness, &format!("background-{layout}"));
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            harness.read(|editor| assert!(editor.activities.view().is_none()));
        }
    });
}
