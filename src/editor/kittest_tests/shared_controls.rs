use super::*;
use crate::editor::history::Direction;

fn control_state(h: &mut Harness<'_, Editor>, tool_active: bool) {
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
            let orientation = h.read(|editor| editor.document.project.orientation);
            assert!(
                !orientation.is_identity(),
                "fixture must make Reset meaningful"
            );
            let before = h.read(|editor| editor.document.project.clone());
            let history = h.read(|editor| editor.history_label(Direction::Undo));
            for label in [
                "Turn left",
                "Turn right",
                "Flip horizontal",
                "Flip vertical",
                "Reset scan orientation",
            ] {
                let command = h.get_by_role_and_label(Role::Button, label);
                assert_eq!(
                    command.accesskit_node().is_disabled(),
                    tool_active,
                    "Roll shelf: {label}"
                );
                if tool_active {
                    command.click();
                    h.step();
                    h.step();
                    h.read(|editor| {
                        assert_eq!(
                            editor.document.project, before,
                            "blocked {label} must not change accepted settings"
                        );
                        assert_eq!(
                            editor.history_label(Direction::Undo),
                            history,
                            "blocked {label} must not record history"
                        );
                    });
                }
            }
            h.get_by_role_and_label(Role::RadioButton, "Digital")
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            assert_eq!(
                h.get_by_role_and_label(Role::RadioButton, "Digital")
                    .accesskit_node()
                    .is_disabled(),
                tool_active,
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
            assert!(!node.accesskit_node().is_disabled(), "{tab}: {label}");
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
fn calibration_swatches_keep_the_measured_rgb_readouts() {
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_opened_photo(&mut h);
        let role = egui::accesskit::Role::Button;
        scroll_to_control(&mut h, role, "Pick Dmax");
        assert_eq!(h.get_all_by_label("Not measured").count(), 2);

        h.update(|editor| {
            editor.load_input(
                crate::processing::Calibration {
                    dmin: DVec3::new(0.5, 0.125, 0.03125),
                    dmax: DVec3::new(0.03125, 0.015625, 0.0078125),
                }
                .into(),
            );
        });
        h.event(egui::Event::PointerGone);
        // The inspector reads accepted values in the next paint, independent of the view worker.
        h.step();
        assert!(h.query_by_label("Not measured").is_none());

        let png = h.render().unwrap();
        let density = h.ctx.pixels_per_point();
        for (picker, readout, colour) in [
            (
                "Pick Dmin (film base)",
                "R 0.500000   G 0.125000   B 0.031250",
                [188, 99, 49],
            ),
            (
                "Pick Dmax",
                "R 0.031250   G 0.015625   B 0.007812",
                [69, 48, 32],
            ),
        ] {
            h.get_by_label(readout);
            let button = h.get_by_role_and_label(role, picker).rect();
            let y = (button.center().y * density) as u32;
            let right = (button.left() * density) as u32;
            assert!(
                (0..right).any(|x| png.get_pixel(x, y).0[..3] == colour),
                "missing {picker} swatch"
            );
        }
        save_capture(&png, "calibration-swatches");
    });
}

#[test]
fn digital_texture_controls_remain_live_during_photo_tools_and_survive_retirement() {
    editor_harness(|mut h, _| {
        h.update(|editor| {
            editor.document.project.input = InputSettings::Digital;
        });
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.canvas.ready.is_some()
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
            let enabled = h.read(|editor| editor.document.project.output.grain.enabled);
            h.get_by_role_and_label(role, "Enable").click();
            h.step();
            h.read(|editor| assert_eq!(editor.document.project.output.grain.enabled, !enabled));
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
                    !h.get_by_role_and_label(role, "Enable")
                        .accesskit_node()
                        .is_disabled(),
                    "{tool} {name}"
                );
                h.get_by_role_and_label(role, "Enable").click();
                h.step();
                let graded = h.read(|editor| {
                    assert!(editor.has_hosted_tool());
                    assert_ne!(
                        editor.document.project.output.grain.enabled,
                        before.output.grain.enabled
                    );
                    editor.document.project.clone()
                });
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
                    assert_eq!(editor.document.project, graded);
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
fn tools_allow_shared_grading_and_replace_mask_artwork_without_resuming_it_implicitly() {
    tools::calibrated_harness(|mut h| {
        stage(&mut h, "Roll");
        click(&mut h, "Turn left");
        h.read(|editor| assert!(!editor.document.project.orientation.is_identity()));
        stage(&mut h, "Photo");
        click(&mut h, "Add linear");
        let image = h.read(|editor| editor.canvas_frame.layout.unwrap().0.image);
        drag(
            &mut h,
            image.min + image.size() * 0.25,
            image.min + image.size() * 0.75,
        );
        h.read(|editor| assert!(editor.masking() && !editor.creating_mask()));
        control_state(&mut h, true);
        for tool in [
            "Crop & straighten",
            "Dust repair",
            "Auto-crop roll",
            "Analyze",
            "Pick Dmin (film base)",
            calibration::GAMMA_PICK_LABEL,
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
                    "{tool}: active mask tool must paint"
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
                | calibration::GAMMA_PICK_LABEL => {
                    stage(&mut h, "Roll");
                    click(&mut h, tool);
                }
                _ => click(&mut h, tool),
            }
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.read(|editor| assert!(editor.has_hosted_tool(), "{tool}: {}", editor.status));
            control_state(&mut h, true);
            stage(&mut h, "Roll");
            for replacement in ["Analyze", "Auto-crop roll"] {
                assert!(
                    !h.get_by_role_and_label(egui::accesskit::Role::Button, replacement)
                        .accesskit_node()
                        .is_disabled(),
                    "{tool}: can replace with {replacement}"
                );
            }
            if matches!(tool, "Dust repair" | "Auto-crop roll" | "Flat-field") {
                save_capture(
                    &h.render().unwrap(),
                    &format!(
                        "shared-roll-toolbar-{}",
                        tool.replace(' ', "-").to_lowercase()
                    ),
                );
            }
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
                            "{tool}: replacement owns its own canvas artwork"
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
            wait_for(&mut h, |editor| !editor.interaction_blocked());
            h.read(|editor| {
                assert!(!editor.tool_interacting(), "{tool}");
                assert_eq!(editor.document.project, before, "{tool}");
                assert_eq!(
                    editor.history_label(Direction::Undo),
                    history.as_deref(),
                    "{tool}"
                );
                assert_eq!(
                    mask_artwork(editor),
                    0,
                    "{tool}: retiring a replacement does not reopen mask editing"
                );
            });
            adjustment_layers::show_layers(&mut h);
            let name = h.read(|editor| {
                adjustment_layers::layers(editor)
                    .layers
                    .last()
                    .unwrap()
                    .name
                    .clone()
            });
            click(&mut h, &format!("Edit {name}"));
            h.read(|editor| {
                assert!(
                    mask_artwork(editor) > 0,
                    "explicit Edit restores the unchanged mask"
                )
            });
        }
        click(&mut h, "Done");
        control_state(&mut h, false);
        stage(&mut h, "Roll");
        click(&mut h, "Reset scan orientation");
        h.read(|editor| assert!(editor.document.project.orientation.is_identity()));
        stage(&mut h, "Photo");
        workflow::enter_numeric(&mut h, "Exposure", "0.625");
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.625));
    });
}
