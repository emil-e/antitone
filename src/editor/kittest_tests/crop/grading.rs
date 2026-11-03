use super::*;

#[test]
fn crop_keeps_its_draft_while_roll_and_rendering_controls_accept_edits() {
    editor_harness(|mut h, _| {
        h.update(|editor| {
            editor.document.input = crate::processing::InputSettings::Digital;
            editor.sync_document();
        });
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document.visit.canvas.ready.is_some()
        });
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
            .click();
        h.step();
        let layout = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
        drag(
            &mut h,
            layout.image.min,
            layout.image.min + egui::vec2(25., 18.),
        );
        let draft = h.read(|editor| editor.document.crop_edit().unwrap().draft);
        assert_ne!(draft, layout.composition.crop());
        stage(&mut h, "Roll");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
            .click();
        h.step();
        h.try_run_realtime().unwrap();
        workflow::enter_numeric(&mut h, "Exposure", "0.5");
        h.read(|editor| assert_eq!(editor.document.roll.exposure, 0.5));
        stage(&mut h, "Rendering");
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        let was_enabled = h.read(|editor| editor.document.grain.enabled);
        h.get_by_role_and_label(egui::accesskit::Role::CheckBox, "Enable")
            .click();
        h.step();
        h.read(|editor| assert_eq!(editor.document.grain.enabled, !was_enabled));
        let role = egui::accesskit::Role::ComboBox;
        h.get_by_role_and_label(role, "Target").scroll_to_me();
        h.try_run_realtime().unwrap();
        select_target(&mut h, OutputTarget::Srgb);
        let paper = crate::looks::spektrafilm::Paper::default();
        h.get_by_role_and_label(role, "Paper").scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(role, "Paper").click();
        h.step();
        h.step();
        h.get_by_label(paper.name()).scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_label(paper.name()).click();
        h.step();
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document.look.is_none()
        });
        h.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .unwrap()
                    .origin(),
                &crate::looks::spektrafilm::Selection {
                    negative: None,
                    paper
                }
                .origin()
            );
            assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
            assert!(editor.document.visit.crop.is_none());
        });
        let before_switch = h.read(|editor| editor.document.state.project.clone());
        let negative =
            crate::looks::monochrome::Negative::from_id("ilford_fp4_plus_ilfotec_hc_8m").unwrap();
        for confirm in [false, true] {
            h.get_by_role_and_label(role, "Negative").scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(role, "Negative").click();
            h.step();
            h.get_by_label(negative.name()).scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_label(negative.name()).click();
            h.step();
            h.read(|editor| assert!(editor.document.material_switch.is_some()));
            if confirm {
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Switch materials")
                    .click();
            } else {
                h.key_press(egui::Key::Escape);
            }
            h.step();
            wait_for(&mut h, |editor| {
                editor.document.material_switch.is_none() && editor.document.look.is_none()
            });
            h.read(|editor| {
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert!(editor.document.visit.crop.is_none());
                if confirm {
                    let asset = editor
                        .document
                        .project
                        .output
                        .film_and_paper
                        .as_ref()
                        .unwrap();
                    assert_eq!(
                        crate::looks::monochrome::Selection::from_origin(asset.origin()),
                        Some(crate::looks::monochrome::Selection {
                            negative: Some(negative),
                            paper: Default::default(),
                        })
                    );
                } else {
                    assert_eq!(editor.document.state.project, before_switch);
                }
            });
        }
        let accepted = h.read(|editor| editor.document.state.project.clone());
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Cancel")
            .click();
        h.step();
        h.read(|editor| {
            assert!(!editor.cropping());
            assert_eq!(editor.document.state.project, accepted);
        });
    });
}

#[test]
fn live_grading_survives_crop_apply_cancel_and_tool_replacement() {
    for exit in ["Apply", "Cancel", "Dust repair", "Add linear"] {
        editor_harness(|mut h, capture| {
            h.update(|editor| {
                editor.document.input = crate::processing::InputSettings::Digital;
                editor.sync_document();
            });
            h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
            wait_for(&mut h, |editor| {
                !editor.busy && editor.document.visit.canvas.ready.is_some()
            });
            stage(&mut h, "Photo");
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
                .click();
            h.step();
            wait_for(&mut h, |editor| {
                editor
                    .document
                    .crop_edit()
                    .is_some_and(|edit| edit.layout.is_some())
            });
            let initial = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            drag(
                &mut h,
                initial.image.min,
                initial.image.min + egui::vec2(25., 18.),
            );
            let draft = h.read(|editor| editor.document.crop_edit().unwrap().draft);
            assert_ne!(
                draft,
                initial.composition.crop(),
                "fixture must have an actual unaccepted edit"
            );
            adjustment_layers::show_layers(&mut h);
            h.event(egui::Event::PointerGone);
            h.step();
            h.render().unwrap();
            let layout = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            let sample = layout.screen_point(egui::pos2(96., 160.));
            assert!(layout.image.shrink(12.).contains(sample));
            assert!(layout.canvas.shrink(12.).contains(sample));
            let pixel = |output: &FloatCapture| {
                assert!(
                    !output.pixels.is_empty(),
                    "actual GPU callback must have painted"
                );
                output.pixels[sample.y.floor() as usize * output.size[0] as usize
                    + sample.x.floor() as usize]
            };
            let before = pixel(&capture.borrow());
            let exposure = h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure");
            exposure.scroll_to_me();
            h.try_run_realtime().unwrap();
            assert!(
                !h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                    .accesskit_node()
                    .is_disabled()
            );
            workflow::enter_numeric(&mut h, "Exposure", "0.75");
            h.try_run_realtime().unwrap();
            h.read(|editor| {
                assert_eq!(
                    editor.document.visit.adjustment_layer_settings.exposure,
                    0.75
                );
                assert_eq!(editor.document.crop_edit().unwrap().draft, draft);
                assert!(editor.document.visit.crop.is_none());
                assert_eq!(
                    editor.document.crop_edit().unwrap().layout.unwrap().image,
                    layout.image
                );
            });
            h.event(egui::Event::PointerGone);
            h.step();
            h.render().unwrap();
            assert_ne!(
                pixel(&capture.borrow()),
                before,
                "changing the accepted grade must affect the painted Crop image"
            );
            let mut expected = h.read(|editor| editor.document.state.project.clone());
            let source = h.read(|editor| editor.document.visit.current_source.unwrap());
            if exit == "Apply" {
                expected.source_mut(source).unwrap().crop = Some(draft);
            }
            h.get_by_role_and_label(egui::accesskit::Role::Button, exit)
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            let action = h.get_by_role_and_label(egui::accesskit::Role::Button, exit);
            assert!(!action.accesskit_node().is_disabled());
            action.click();
            h.step();
            wait_for(&mut h, |editor| !editor.busy && !editor.cropping());
            h.read(|editor| {
                assert_eq!(editor.document.state.project, expected);
                assert_eq!(
                    editor.document.visit.adjustment_layer_settings.exposure,
                    0.75
                );
                if exit == "Dust repair" {
                    assert!(editor.dusting());
                }
                if exit == "Add linear" {
                    assert!(editor.creating_mask());
                }
            });
            if exit == "Dust repair" {
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Done")
                    .click();
                h.step();
            } else if exit == "Add linear" {
                h.key_press(egui::Key::Escape);
                h.step();
            }
            if exit == "Apply" {
                h.read(|editor| {
                    assert_eq!(
                        editor.history_label(super::super::super::history::Direction::Undo),
                        Some("Crop")
                    )
                });
                h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
                h.step();
                wait_for(&mut h, |editor| {
                    !editor.busy && editor.document_pending.is_none()
                });
                h.read(|editor| {
                    assert!(editor.document.visit.crop.is_none());
                    assert_eq!(
                        editor.document.visit.adjustment_layer_settings.exposure,
                        0.75
                    );
                });
            }
            h.read(|editor| {
                assert_eq!(
                    editor.history_label(super::super::super::history::Direction::Undo),
                    Some("Photo settings")
                )
            });
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
            h.step();
            wait_for(&mut h, |editor| {
                !editor.busy && editor.document_pending.is_none()
            });
            h.read(|editor| {
                assert_eq!(editor.document.visit.adjustment_layer_settings.exposure, 0.)
            });
        });
    }
}
