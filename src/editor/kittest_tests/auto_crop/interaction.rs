use super::*;

#[test]
fn batch_owns_its_global_drawer_blocks_grading_and_is_replaced_by_crop_at_both_layouts() {
    for (size, width) in [
        (egui::vec2(1100., 800.), 315.),
        (egui::vec2(800., 600.), 270.),
    ] {
        setup(|mut h, _directory| {
            set_layout(&mut h, size, width);
            stage(&mut h, "Roll");
            h.get_by_role_and_label(egui::accesskit::Role::Slider, "Gamma")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            assert!(
                !h.get_by_role_and_label(egui::accesskit::Role::Slider, "Gamma")
                    .accesskit_node()
                    .is_disabled()
            );

            click_action(&mut h, "Auto-crop roll");
            overlay::wait_context(&mut h);
            let before = h.read(|editor| editor.document.state.project.clone());
            h.read(|editor| {
                assert!(
                    editor
                        .current_tool()
                        .unwrap()
                        .is::<super::super::super::auto_crop::AutoCropTool>()
                )
            });
            h.get_by_role_and_label(egui::accesskit::Role::Slider, "Gamma")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            assert!(
                h.get_by_role_and_label(egui::accesskit::Role::Slider, "Gamma")
                    .accesskit_node()
                    .is_disabled()
            );

            for tab in ["Photo", "Rendering", "Roll"] {
                stage(&mut h, tab);
                h.get_by_role_and_label(egui::accesskit::Role::Slider, "Margin")
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                assert!(
                    !h.get_by_role_and_label(egui::accesskit::Role::Slider, "Margin")
                        .accesskit_node()
                        .is_disabled()
                );
            }
            stage(&mut h, "Photo");
            h.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            assert!(
                h.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
                    .accesskit_node()
                    .is_disabled()
            );
            click_action(&mut h, "Crop & straighten");
            h.read(|editor| {
                assert!(!editor.document.auto_crop.active());
                assert_eq!(editor.document.state.project, before);
            });

            h.read(|editor| assert!(editor.cropping()));
            h.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            assert!(
                h.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
                    .accesskit_node()
                    .is_disabled()
            );
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                .click();
            h.step();
            h.read(|editor| assert_eq!(editor.document.state.project, before));
        });
    }
}

#[test]
fn preparation_keeps_footer_cancel_available_and_apply_waits_for_ready_proposals() {
    for cancel in [true, false] {
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            setup(|mut h, _| {
                set_layout(&mut h, size, width);
                stage(&mut h, "Roll");
                let (before, metadata, history) = h.read(|editor| {
                    (
                        editor.document.state.project.clone(),
                        editor.document.spatial.ready(),
                        editor.history_label(crate::editor::history::Direction::Undo),
                    )
                });
                // Hold region publication while the real tool continuation waits for it.
                let reply = h.update(|editor| {
                    editor.document.spatial = crate::spatial_metadata::Store::default();
                    let (reply, receive) = futures_channel::oneshot::channel();
                    editor.document.spatial_reply = Some(receive);
                    reply
                });
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Auto-crop roll")
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Auto-crop roll")
                    .click();
                h.step();
                h.step();
                h.read(|editor| {
                    assert!(editor.busy && editor.document.spatial_preparing.is_some());
                    assert!(
                        editor.document.spatial_reply.is_none(),
                        "producer must capture the held reply rather than reuse ready metadata"
                    );
                    assert!(
                        editor.document.auto_crop.active()
                            && !editor.document.auto_crop.reviewing()
                    );
                });
                let identity = h.get_by_label("Active tool: Auto-crop · Working").rect();
                assert!(h.ctx.content_rect().contains_rect(identity));
                assert!(
                    !h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                        .accesskit_node()
                        .is_disabled()
                );
                assert!(
                    h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                        .accesskit_node()
                        .is_disabled()
                );
                save_capture(
                    &h.render().unwrap(),
                    &format!("auto-crop-preparing-footer-{width}-offscreen"),
                );
                if cancel {
                    h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
                        .click();
                    h.step();
                    h.read(|editor| {
                        assert!(!editor.document.auto_crop.active());
                        assert!(
                            editor.document.spatial_preparing.is_some(),
                            "shared preparation outlives its consumer"
                        );
                        assert_eq!(editor.document.state.project, before);
                    });
                }
                reply.send(Ok(Some((*metadata).clone()))).unwrap();
                wait_for(&mut h, |editor| {
                    editor.document.spatial_preparing.is_none()
                        && (cancel || editor.document.auto_crop.reviewing())
                });
                h.read(|editor| assert_eq!(editor.document.state.project, before));
                if cancel {
                    h.read(|editor| {
                        assert!(!editor.document.auto_crop.active());
                        assert_eq!(
                            editor.history_label(crate::editor::history::Direction::Undo),
                            history
                        );
                    });
                } else {
                    assert!(
                        !h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                            .accesskit_node()
                            .is_disabled()
                    );
                    h.get_by_role_and_label(egui::accesskit::Role::Button, "Apply")
                        .click();
                    h.step();
                    h.read(|editor| {
                        assert!(!editor.document.auto_crop.active());
                        assert_ne!(
                            editor.document.state.project, before,
                            "the prepared proposals must be accepted"
                        );
                        assert_eq!(
                            editor.history_label(crate::editor::history::Direction::Undo),
                            Some("Auto-crop")
                        );
                    });
                }
            });
        }
    }
}

#[test]
fn grading_and_filmstrip_previews_stay_independent_of_batch_acceptance() {
    for accept in [false, true] {
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            setup(|mut h, _| {
                set_layout(&mut h, size, width);
                stage(&mut h, "Roll");
                let original = h.read(|e| e.document.state.project.clone());
                workflow::enter_numeric(&mut h, "Gamma", "0.73");
                stage(&mut h, "Photo");
                workflow::enter_numeric(&mut h, "Exposure", "1.25");
                let graded = h.read(|e| e.document.state.project.clone());
                assert_ne!(
                    graded, original,
                    "fixture must have independently accepted grading"
                );
                stage(&mut h, "Roll");
                click_action(&mut h, "Auto-crop roll");
                overlay::wait_context(&mut h);
                let regions = h.read(|e| e.document.spatial.ready());
                h.read(|e| {
                    assert!(Arc::ptr_eq(&regions, &e.document.spatial.ready()));
                    assert_eq!(e.document.visit.adjustment_layer_settings.exposure, 1.25);
                    for id in &original.order {
                        assert_eq!(
                            graded.source(*id).unwrap().crop,
                            original.source(*id).unwrap().crop
                        );
                    }
                    assert!(
                        !e.application_menu_state()
                            .enabled(crate::platform::menu::Action::Save)
                    );
                    assert!(
                        !e.application_menu_state()
                            .enabled(crate::platform::menu::Action::Export)
                    );
                });
                workflow::enter_numeric(&mut h, "Margin", "2");
                h.step();
                h.read(|e| {
                    let preview = e.document.thumbnails.preview();
                    assert_eq!(preview.compositions.len(), original.order.len());
                    for (id, framing) in &preview.compositions {
                        assert_eq!(framing.crop(), margin::expected_crop(&h, *id, 2.));
                        assert_eq!(
                            e.document.project.source(*id).unwrap().crop,
                            graded.source(*id).unwrap().crop
                        );
                    }
                });
                h.get_by_role_and_label(
                    egui::accesskit::Role::Button,
                    if accept { "Apply" } else { "Cancel" },
                )
                .click();
                h.step();
                h.step();
                h.read(|e| {
                    assert!(!e.has_hosted_tool());
                    assert!(
                        e.document.thumbnails.preview().compositions.is_empty(),
                        "the next filmstrip pass uses ordinary inputs after its draft caller ends"
                    );
                    assert_eq!(e.document.project.input, graded.input);
                    assert_eq!(e.document.project.roll, graded.roll);
                    for id in &graded.order {
                        assert_eq!(
                            e.document.project.source(*id).unwrap().adjustment_layers,
                            graded.source(*id).unwrap().adjustment_layers
                        );
                    }
                    if accept {
                        assert!(
                            graded.order.iter().any(|id| e
                                .document
                                .project
                                .source(*id)
                                .unwrap()
                                .crop
                                != graded.source(*id).unwrap().crop)
                        );
                        assert_eq!(
                            e.history_label(crate::editor::history::Direction::Undo),
                            Some("Auto-crop")
                        );
                    } else {
                        assert_eq!(e.document.state.project, graded);
                    }
                });
                if accept {
                    h.update(|e| host::App::menu_action(e, crate::platform::menu::Action::Undo));
                    h.step();
                    wait_for(&mut h, |e| !e.busy && e.document_pending.is_none());
                    h.read(|e| assert_eq!(e.document.state.project, graded, "one history action undoes only crop acceptance, not independent grading"));
                }
            });
        }
    }
}

#[test]
fn a_noop_history_request_does_not_retire_an_unfinished_batch() {
    setup(|mut h, _| {
        h.update(|e| e.document.history = Default::default());
        stage(&mut h, "Roll");
        click_action(&mut h, "Auto-crop roll");
        overlay::wait_context(&mut h);
        workflow::enter_numeric(&mut h, "Margin", "2");
        let before = h.read(|e| e.document.state.project.clone());
        h.enter(|e, cx| e.restore_history(crate::editor::history::Direction::Undo, cx));
        h.step();
        h.read(|e| {
            assert!(
                e.current_tool()
                    .unwrap()
                    .is::<crate::editor::auto_crop::AutoCropTool>()
            );
            assert_eq!(e.document.state.project, before);
            assert_eq!(e.auto_crop_margin, 2.);
        });
    });
}
