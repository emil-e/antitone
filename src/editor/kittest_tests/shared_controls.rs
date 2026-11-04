use super::*;
use crate::editor::history::Direction;

fn control_state(h: &mut Harness<'_, Editor>, disabled: bool) {
    use egui::accesskit::Role;
    for (tab, controls) in [
        (
            "Roll",
            vec![
                (Role::SpinButton, "Gamma"),
                (Role::Unknown, "White balance wheel"),
            ],
        ),
        (
            "Photo",
            vec![(Role::SpinButton, "Exposure"), (Role::Button, "Add global")],
        ),
        (
            "Rendering",
            vec![
                (Role::ComboBox, "LUT"),
                (Role::ComboBox, "Negative"),
                (Role::ComboBox, "Paper"),
                (Role::ComboBox, "Target"),
            ],
        ),
    ] {
        stage(h, tab);
        if tab == "Roll" {
            let input_stage = h
                .query_all_by_role(Role::ComboBox)
                .find(|node| node.accesskit_node().value().as_deref() == Some("Negative"))
                .expect("Negative input-stage dropdown");
            input_stage.scroll_to_me();
            h.try_run_realtime().unwrap();
            assert_eq!(
                h.query_all_by_role(Role::ComboBox)
                    .find(|node| node.accesskit_node().value().as_deref() == Some("Negative"))
                    .expect("Negative input-stage dropdown")
                    .accesskit_node()
                    .is_disabled(),
                disabled,
                "Input stage"
            );
        }
        for (role, label) in controls {
            let node = if role == Role::Unknown {
                h.get_by_label(label)
            } else {
                h.get_by_role_and_label(role, label)
            };
            node.scroll_to_me();
            h.try_run_realtime().unwrap();
            let node = if role == Role::Unknown {
                h.get_by_label(label)
            } else {
                h.get_by_role_and_label(role, label)
            };
            assert_eq!(
                node.accesskit_node().is_disabled(),
                disabled,
                "{tab}: {label}"
            );
        }
    }
}

fn mask_artwork(editor: &Editor) -> usize {
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
        editor.paint_mask(ui, editor.canvas_frame.layout.unwrap().0);
    });
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter(|shape| !matches!(shape.shape, egui::Shape::Noop))
        .count()
}

fn click(h: &mut Harness<'_, Editor>, label: &str) {
    let role = egui::accesskit::Role::Button;
    h.get_by_role_and_label(role, label).scroll_to_me();
    h.try_run_realtime().unwrap();
    assert!(
        !h.get_by_role_and_label(role, label)
            .accesskit_node()
            .is_disabled(),
        "{label}"
    );
    h.get_by_role_and_label(role, label).click();
    h.step();
    h.step();
}

#[test]
fn digital_texture_controls_wait_for_photo_tools_and_resume_after_retirement() {
    editor_harness(|mut h, _| {
        h.update(|editor| {
            editor.document.input = InputSettings::Digital;
            editor.sync_document();
        });
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.busy && editor.document.visit.canvas.ready.is_some()
        });
        for (name, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut h, size, width);
            stage(&mut h, "Rendering");
            let role = egui::accesskit::Role::CheckBox;
            h.get_by_role_and_label(role, "Enable").scroll_to_me();
            h.try_run_realtime().unwrap();
            assert!(
                !h.get_by_role_and_label(role, "Enable")
                    .accesskit_node()
                    .is_disabled()
            );
            let enabled = h.read(|editor| editor.document.grain.enabled);
            h.get_by_role_and_label(role, "Enable").click();
            h.step();
            h.read(|editor| assert_eq!(editor.document.grain.enabled, !enabled));
            for tool in [
                "Crop & straighten",
                "Dust repair",
                "Add linear",
                "Add radial",
            ] {
                stage(&mut h, "Photo");
                click(&mut h, tool);
                let before = h.read(|editor| editor.document.project.clone());
                stage(&mut h, "Rendering");
                h.get_by_role_and_label(role, "Enable").scroll_to_me();
                h.try_run_realtime().unwrap();
                assert!(
                    h.get_by_role_and_label(role, "Enable")
                        .accesskit_node()
                        .is_disabled(),
                    "{tool} {name}"
                );
                save_capture(
                    &h.render().unwrap(),
                    &format!(
                        "shared-rendering-{name}-{}",
                        tool.replace(' ', "-").to_lowercase()
                    ),
                );
                click(
                    &mut h,
                    if tool == "Dust repair" {
                        "Done"
                    } else {
                        "Cancel"
                    },
                );
                h.read(|editor| {
                    assert!(!editor.tool_interacting());
                    assert_eq!(editor.document.project, before);
                });
                assert!(
                    !h.get_by_role_and_label(role, "Enable")
                        .accesskit_node()
                        .is_disabled()
                );
            }
        }
    });
}

#[test]
fn tools_exclude_shared_grading_and_contextual_artwork_but_keep_their_own_controls_and_replacements()
 {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Photo");
        click(&mut h, "Add linear");
        let image = h.read(|editor| editor.canvas_frame.layout.unwrap().0.image);
        drag(
            &mut h,
            image.min + image.size() * 0.25,
            image.min + image.size() * 0.75,
        );
        h.read(|editor| assert!(!editor.tool_interacting()));
        control_state(&mut h, false);
        for tool in [
            "Crop & straighten",
            "Dust repair",
            "Auto-crop roll",
            "Analyze",
            "Pick Dmin (film base)",
            "Pick neutral for gamma balance",
            "Pick neutral for white balance",
            "Add linear",
            "Add radial",
            "Flat-field",
        ] {
            stage(&mut h, "Photo");
            adjustment_layers::show_layers(&mut h);
            h.read(|editor| {
                assert!(
                    mask_artwork(editor) > 0,
                    "{tool}: ordinary selected gradient must paint"
                )
            });
            let before = h.read(|editor| editor.document.project.clone());
            let history = h.read(|editor| editor.history_label(Direction::Undo).map(str::to_owned));
            match tool {
                "Flat-field" => {
                    h.enter(|editor, cx| {
                        editor.begin_flat_edit(fixture("bare-light.dng"), None, cx)
                    });
                    h.step();
                }
                "Auto-crop roll"
                | "Analyze"
                | "Pick Dmin (film base)"
                | "Pick neutral for gamma balance" => {
                    stage(&mut h, "Roll");
                    click(&mut h, tool);
                }
                _ => click(&mut h, tool),
            }
            wait_for(&mut h, |editor| !editor.busy);
            h.read(|editor| assert!(editor.has_hosted_tool(), "{tool}: {}", editor.status));
            control_state(&mut h, true);
            stage(&mut h, "Photo");
            for replacement in ["Crop & straighten", "Dust repair", "Add radial"] {
                h.get_by_role_and_label(egui::accesskit::Role::Button, replacement)
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                assert!(
                    !h.get_by_role_and_label(egui::accesskit::Role::Button, replacement)
                        .accesskit_node()
                        .is_disabled(),
                    "{tool}: can replace with {replacement}"
                );
            }
            h.update(|editor| {
                editor
                    .with_current_tool(|_, editor| {
                        assert_eq!(
                            mask_artwork(editor),
                            0,
                            "{tool}: moved-out owner excludes contextual artwork"
                        )
                    })
                    .unwrap();
            });
            click(
                &mut h,
                if tool == "Dust repair" {
                    "Done"
                } else {
                    "Cancel"
                },
            );
            wait_for(&mut h, |editor| !editor.busy);
            h.read(|editor| {
                assert!(!editor.tool_interacting(), "{tool}");
                assert_eq!(editor.document.project, before, "{tool}");
                assert_eq!(
                    editor.history_label(Direction::Undo),
                    history.as_deref(),
                    "{tool}"
                );
                assert!(
                    mask_artwork(editor) > 0,
                    "{tool}: unchanged contextual artwork returns"
                );
            });
        }
        control_state(&mut h, false);
        stage(&mut h, "Photo");
        workflow::enter_numeric(&mut h, "Exposure", "0.625");
        h.read(|editor| {
            assert_eq!(
                editor.document.visit.adjustment_layer_settings.exposure,
                0.625
            )
        });
    });
}
