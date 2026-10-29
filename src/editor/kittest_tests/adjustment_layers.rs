use super::*;
use crate::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
    geometry::SourcePoint,
    project::{AdjustmentLayerId, AdjustmentLayers},
};

pub(super) fn layers(editor: &Editor) -> AdjustmentLayers {
    editor
        .document
        .project
        .source(editor.document.visit.current_source.unwrap())
        .unwrap()
        .adjustment_layers
        .clone()
        .unwrap_or_default()
}

fn header_label(harness: &Harness<'_, Editor>) -> String {
    harness
        .query_all_by_role(egui::accesskit::Role::Button)
        .filter_map(|node| node.accesskit_node().label())
        .find(|label| label == "Layers" || label.starts_with("Layers: "))
        .expect("Layers section header")
}

pub(super) fn show_layers(harness: &mut Harness<'_, Editor>) {
    harness.try_run_realtime().unwrap();
    if harness.read(|editor| editor.tab != 1) {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Tab, "Photo")
            .click();
        harness.step();
        harness.try_run_realtime().unwrap();
    }
    if harness.query_all_by_label("Add global").count() == 0 {
        harness.get_by_label(&header_label(harness)).scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label(&header_label(harness)).click();
        harness.step();
        harness.try_run_realtime().unwrap();
    }
}

pub(super) fn install_layers(editor: &mut Editor) -> [AdjustmentLayerId; 2] {
    let source = editor.document.visit.current_source.unwrap();
    let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
    let mut layers = AdjustmentLayers::default();
    let adjustment = AdjustmentLayer {
        mask: AdjustmentLayerCoverage::Linear {
            start: SourcePoint([area.x as f64, area.y as f64]),
            end: SourcePoint([(area.x + area.width) as f64, (area.y + area.height) as f64]),
        },
        photo: AdjustmentLayerSettings::default(),
        enabled: true,
        invert: false,
        strength: 1.,
    };
    let sky = layers.add("Sky".into(), adjustment).unwrap();
    let ground = layers
        .add(
            "Foreground".into(),
            AdjustmentLayer {
                mask: AdjustmentLayerCoverage::Radial {
                    center: SourcePoint([
                        area.x as f64 + area.width as f64 * 0.5,
                        area.y as f64 + area.height as f64 * 0.5,
                    ]),
                    radii: [area.width as f64 * 0.35, area.height as f64 * 0.25],
                    angle: 0.3,
                    feather: 0.8,
                },
                ..adjustment
            },
        )
        .unwrap();
    editor
        .document
        .project
        .source_mut(source)
        .unwrap()
        .adjustment_layers = Some(layers);
    editor.document.visit.adjustment_layer_settings =
        editor.selected_adjustment_layer_settings().unwrap();
    editor.select_adjustment_layer(super::super::adjustment_layers::Target::Layer(sky));
    editor
        .validate_source_candidate(editor.document.state.project.source(source).unwrap())
        .unwrap();
    [sky, ground]
}

fn click(harness: &mut Harness<'_, Editor>, name: &str) {
    let name = if name == "Layers" {
        header_label(harness)
    } else {
        name.into()
    };
    harness.get_by_label(&name).scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label(&name).click();
    harness.step();
    harness.try_run_realtime().unwrap();
}

fn pending_exposure(harness: &mut Harness<'_, Editor>, text: &str) {
    let input = harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure");
    input.scroll_to_me();
    harness.try_run_realtime().unwrap();
    let input = harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure");
    input.click();
    harness.step();
    harness.event(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.event(egui::Event::Text(text.into()));
    harness.step();
}

fn delete_selected(harness: &mut Harness<'_, Editor>) {
    let name = harness.read(|editor| {
        let super::super::adjustment_layers::Target::Layer(id) = editor.adjustment_layer_target()
        else {
            panic!("select a layer");
        };
        layers(editor)
            .layers
            .iter()
            .find(|record| record.id == id)
            .unwrap()
            .name
            .clone()
    });
    click(harness, &name);
    harness.event(egui::Event::Key {
        key: egui::Key::Delete,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.try_run_realtime().unwrap();
    click(harness, "Delete layer");
}

#[test]
fn default_global_is_an_ordinary_layer_and_new_layers_target_the_shared_controls() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        show_layers(&mut harness);
        harness.read(|editor| {
            let layers = layers(editor);
            assert_eq!(layers.layers.len(), 1);
            assert_eq!(
                layers.layers[0].adjustment.mask,
                AdjustmentLayerCoverage::Global {}
            );
        });
        harness.get_by_label("Layers: Global");
        workflow::enter_numeric(&mut harness, "Exposure", "0.5");
        click(&mut harness, "Add global");
        harness.get_by_label("Layers: Global 2");
        workflow::enter_numeric(&mut harness, "Exposure", "-0.25");
        click(&mut harness, "Global");
        harness.read(|editor| {
            assert_eq!(
                editor.document.visit.adjustment_layer_settings.exposure,
                0.5
            );
            let layers = layers(editor);
            assert_eq!(layers.layers[0].adjustment.photo.exposure, 0.5);
            assert_eq!(layers.layers[1].adjustment.photo.exposure, -0.25);
        });
        click(&mut harness, "Enable Global");
        harness.read(|editor| assert!(!layers(editor).layers[0].adjustment.enabled));
        delete_selected(&mut harness);
        harness.read(|editor| {
            let layers = layers(editor);
            assert_eq!(layers.layers.len(), 1);
            assert_eq!(layers.layers[0].name, "Global 2");
            assert_eq!(
                editor.document.visit.adjustment_layer_settings.exposure,
                -0.25
            );
        });
    });
}

#[test]
fn selecting_layers_commits_pending_numbers_and_resets_only_the_selected_layer() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        for (size, width) in [
            (egui::vec2(1100., 800.), 315.),
            (egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            click(&mut harness, "Sky");
            pending_exposure(&mut harness, "1.125");
            harness.read(|editor| {
                assert_ne!(layers(editor).layers[0].adjustment.photo.exposure, 1.125)
            });
            click(&mut harness, "Foreground");
            harness.read(|editor| {
                assert_eq!(layers(editor).layers[0].adjustment.photo.exposure, 1.125)
            });
            harness.get_by_label("Layers: Foreground");
            workflow::enter_numeric(&mut harness, "Exposure", "-0.75");
            click(&mut harness, "Reset Exposure");
            harness.read(|editor| {
                let layers = layers(editor);
                assert_eq!(layers.layers[1].adjustment.photo.exposure, 0.);
                assert_eq!(layers.layers[0].adjustment.photo.exposure, 1.125);
            });
            pending_exposure(&mut harness, "0.25");
            click(&mut harness, "Enable Foreground");
            for name in ["Exposure", "Strength", "Feather"] {
                assert!(
                    harness
                        .get_by_role_and_label(egui::accesskit::Role::SpinButton, name)
                        .accesskit_node()
                        .is_disabled()
                );
            }
            assert!(
                harness
                    .get_by_label("Invert mask")
                    .accesskit_node()
                    .is_disabled()
            );
            assert!(
                !harness
                    .get_by_label("Enable Foreground")
                    .accesskit_node()
                    .is_disabled()
            );
            harness.read(|editor| {
                let layer = layers(editor).layers[1].adjustment;
                assert!(!layer.enabled);
                assert_eq!(layer.photo.exposure, 0.25);
                assert_eq!(layers(editor).layers[0].adjustment.photo.exposure, 1.125);
            });
            click(&mut harness, "Enable Foreground");
            assert!(
                !harness
                    .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                    .accesskit_node()
                    .is_disabled()
            );
            click(&mut harness, "Sky");
            click(&mut harness, "Reset Exposure");
            harness.get_by_label(&header_label(&harness)).scroll_to_me();
            harness.try_run_realtime().unwrap();
            workflow::wait_settled(&mut harness);
            let capture = harness.render().unwrap();
            save_capture(
                &capture,
                if width == 315. {
                    "adjustment-layers-normal"
                } else {
                    "adjustment-layers-narrow"
                },
            );
        }
    });
}

#[test]
fn visual_order_is_bottom_to_top_and_creation_inserts_above_the_selected_layer() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        let ids = harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        click(&mut harness, "Sky");
        click(&mut harness, "Add global");
        harness.read(|editor| {
            let layers = layers(editor);
            assert_eq!(
                layers
                    .layers
                    .iter()
                    .map(|record| record.id)
                    .collect::<Vec<_>>(),
                vec![ids[0], AdjustmentLayerId(3), ids[1]]
            );
        });
        harness.get_by_label("Foreground").scroll_to_me();
        harness.try_run_realtime().unwrap();
        assert!(
            harness.get_by_label("Foreground").rect().center().y
                < harness.get_by_label("Global 3").rect().center().y
        );
        assert!(
            harness.get_by_label("Global 3").rect().center().y
                < harness.get_by_label("Sky").rect().center().y
        );
        let from = harness.get_by_label("Global 3").rect().center();
        let row = harness.get_by_label("Foreground").rect();
        let to = egui::pos2(row.center().x, row.top() + 2.);
        harness.hover_at(from);
        harness.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        harness.hover_at(to);
        harness.step();
        harness.event(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        harness.try_run_realtime().unwrap();
        harness.read(|editor| {
            let layers = layers(editor);
            assert_eq!(
                layers
                    .layers
                    .iter()
                    .map(|record| record.id)
                    .collect::<Vec<_>>(),
                vec![ids[0], ids[1], AdjustmentLayerId(3)]
            );
            assert_eq!(
                editor.adjustment_layer_target(),
                super::super::adjustment_layers::Target::Layer(AdjustmentLayerId(3))
            );
        });
    });
}

#[test]
fn zero_strength_greys_out_ineffective_controls_but_can_be_restored() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        click(&mut harness, "Foreground");
        let settings = harness.read(|editor| editor.document.visit.adjustment_layer_settings);
        workflow::enter_numeric(&mut harness, "Strength", "0");
        harness.try_run_realtime().unwrap();
        for name in ["Exposure", "Contrast", "Saturation", "Feather"] {
            assert!(
                harness
                    .get_by_role_and_label(egui::accesskit::Role::SpinButton, name)
                    .accesskit_node()
                    .is_disabled()
            );
        }
        assert!(
            harness
                .get_by_label("Invert mask")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            !harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Strength")
                .accesskit_node()
                .is_disabled()
        );
        harness.read(|editor| {
            let layer = layers(editor).layers[1].adjustment;
            assert!(layer.enabled);
            assert_eq!(layer.strength, 0.);
            assert_eq!(layer.photo, settings);
        });
        workflow::enter_numeric(&mut harness, "Strength", "0.5");
        harness.try_run_realtime().unwrap();
        assert!(
            !harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_disabled()
        );
        harness.read(|editor| {
            let layer = layers(editor).layers[1].adjustment;
            assert_eq!(layer.strength, 0.5);
            assert_eq!(layer.photo, settings);
        });
    });
}

#[test]
fn an_empty_stack_disables_grading_and_can_be_repopulated_without_a_privileged_base() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        show_layers(&mut harness);
        delete_selected(&mut harness);
        harness.read(|editor| {
            assert!(layers(editor).layers.is_empty());
            assert_eq!(
                editor.adjustment_layer_target(),
                super::super::adjustment_layers::Target::None
            );
        });
        harness.get_by_label("Layers");
        assert!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_disabled()
        );
        click(&mut harness, "Add global");
        harness.read(|editor| {
            let layers = layers(editor);
            assert_eq!(layers.layers.len(), 1);
            assert_eq!(layers.layers[0].id, AdjustmentLayerId(2));
        });
        assert!(
            !harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_disabled()
        );
    });
}

#[test]
fn collapsing_layers_does_not_retarget_controls_or_hide_the_target_indicator() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        let ids = harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        click(&mut harness, "Sky");
        click(&mut harness, "Layers");
        assert_eq!(harness.query_all_by_label("Bottom to top").count(), 0);
        harness.get_by_label("Layers: Sky");
        workflow::enter_numeric(&mut harness, "Exposure", "0.75");
        harness.read(|editor| {
            assert_eq!(
                editor.adjustment_layer_target(),
                super::super::adjustment_layers::Target::Layer(ids[0])
            );
            assert_eq!(layers(editor).layers[0].adjustment.photo.exposure, 0.75);
            assert_eq!(layers(editor).layers[1].adjustment.photo.exposure, 0.);
        });
    });
}

fn begin_rename(harness: &mut Harness<'_, Editor>, name: &str) {
    click(harness, name);
    // Separate the selection click from the double-click gesture.
    harness.run_steps(40);
    let point = harness.get_by_label(name).rect().center();
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
        }
    }
    harness.try_run_realtime().unwrap();
    assert!(harness.ctx.text_edit_focused());
}

#[test]
fn inline_rename_commits_before_selection_and_escape_preserves_the_accepted_name() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        let ids = harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        begin_rename(&mut harness, "Sky");
        harness.event(egui::Event::Text(" renamed".into()));
        harness.step();
        click(&mut harness, "Foreground");
        let accepted = harness.read(|editor| layers(editor).layers[0].name.clone());
        assert!(accepted.contains("renamed"));
        harness.read(|editor| {
            assert_eq!(
                editor.adjustment_layer_target(),
                super::super::adjustment_layers::Target::Layer(ids[1])
            )
        });
        begin_rename(&mut harness, &accepted);
        harness.event(egui::Event::Text(" discarded".into()));
        harness.step();
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.read(|editor| assert_eq!(layers(editor).layers[0].name, accepted));
    });
}

#[test]
fn row_drag_escape_and_outside_release_preserve_order_and_canvas_state() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        for escape in [false, true] {
            click(&mut harness, "Sky");
            let before = harness.read(|editor| {
                (
                    editor.document.state.project.clone(),
                    editor.document.visit.canvas.viewport,
                )
            });
            let from = harness.get_by_label("Sky").rect().center();
            let outside = harness.read(|editor| {
                editor
                    .document
                    .visit
                    .canvas
                    .last_layout
                    .unwrap()
                    .0
                    .canvas
                    .center()
            });
            harness.hover_at(from);
            harness.event(egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            harness.hover_at(outside);
            harness.step();
            if escape {
                harness.key_press(egui::Key::Escape);
                harness.step();
            }
            harness.event(egui::Event::PointerButton {
                pos: outside,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            harness.try_run_realtime().unwrap();
            harness.read(|editor| {
                assert_eq!(
                    (
                        editor.document.state.project.clone(),
                        editor.document.visit.canvas.viewport
                    ),
                    before
                );
                assert!(!editor.document.visit.adjustment_layers.dragging());
                assert!(editor.document.visit.mask_edit.is_none());
            });
        }
    });
}

#[test]
fn deletion_confirmation_can_cancel_and_text_input_owns_delete() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        click(&mut harness, "Sky");
        let before = harness.read(|editor| editor.document.state.project.clone());
        harness.key_press(egui::Key::Delete);
        harness.step();
        harness.get_by_label("Delete layer");
        click(&mut harness, "Cancel");
        harness.read(|editor| assert_eq!(editor.document.state.project, before));
        pending_exposure(&mut harness, "1.25");
        harness.key_press(egui::Key::Delete);
        harness.step();
        harness.read(|editor| assert!(!editor.document.visit.adjustment_layers.confirming()));
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.read(|editor| assert_eq!(editor.document.state.project, before));
    });
}

#[test]
fn source_switch_cancels_hidden_text_without_applying_it_to_another_photo() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        click(&mut harness, "Sky");
        let (outgoing, outgoing_grade, other, other_grade) = harness.read(|editor| {
            let outgoing = editor.document.visit.current_source.unwrap();
            let other = editor
                .document
                .project
                .order
                .iter()
                .copied()
                .find(|id| {
                    editor
                        .document
                        .project
                        .source(*id)
                        .unwrap()
                        .path
                        .file_name()
                        .unwrap()
                        == "patches.dng"
                })
                .unwrap();
            (
                outgoing,
                editor
                    .document
                    .project
                    .source(outgoing)
                    .unwrap()
                    .grade_snapshot()
                    .unwrap(),
                other,
                editor
                    .document
                    .project
                    .source(other)
                    .unwrap()
                    .grade_snapshot()
                    .unwrap(),
            )
        });
        pending_exposure(&mut harness, "0.875");
        harness.get_by_label("patches.dng").click();
        harness.step();
        wait_for(&mut harness, |editor| {
            editor.document.visit.current_source == Some(other) && !editor.busy
        });
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .source(outgoing)
                    .unwrap()
                    .grade_snapshot()
                    .unwrap(),
                outgoing_grade
            );
            assert_eq!(
                editor
                    .document
                    .project
                    .source(other)
                    .unwrap()
                    .grade_snapshot()
                    .unwrap(),
                other_grade
            );
            assert_eq!(
                editor.document.visit.adjustment_layer_settings,
                editor.selected_adjustment_layer_settings().unwrap()
            );
        });
    });
}

#[test]
fn stale_deletion_confirmation_cannot_remove_a_changed_layer() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        click(&mut harness, "Sky");
        harness.key_press(egui::Key::Delete);
        harness.step();
        harness.get_by_label("Delete layer");
        // A captured confirmation must not authorize a later accepted record.
        harness.update(|editor| {
            editor
                .edit_adjustment_layers(|layers| {
                    layers.layers[0].name = "Changed while confirming".into();
                    Ok(())
                })
                .unwrap()
        });
        let changed = harness.read(|editor| editor.document.state.project.clone());
        click(&mut harness, "Delete layer");
        harness.read(|editor| {
            assert_eq!(editor.document.state.project, changed);
            assert!(!editor.document.visit.adjustment_layers.confirming());
            assert!(editor.status.contains("deletion cancelled"));
        });
    });
}

#[test]
fn white_balance_picker_targets_masked_partial_strength_layers() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(|editor| {
            install_layers(editor);
            editor
                .edit_adjustment_layers(|layers| {
                    for record in &mut layers.layers {
                        record.adjustment.strength = 0.4;
                        record.adjustment.invert = true;
                    }
                    Ok(())
                })
                .unwrap();
        });
        harness.step();
        show_layers(&mut harness);
        for label in ["Sky", "Foreground"] {
            click(&mut harness, label);
            let picker = harness.get_by_label("Pick neutral for white balance");
            picker.scroll_to_me();
            harness.try_run_realtime().unwrap();
            assert!(
                !harness
                    .get_by_label("Pick neutral for white balance")
                    .accesskit_node()
                    .is_disabled()
            );
        }
    });
}

fn mask_artwork(editor: &Editor) -> usize {
    let context = egui::Context::default();
    let mut output = context.run_ui(Default::default(), |ui| {
        editor.paint_mask(ui, editor.active_canvas().last_layout.unwrap().0);
    });
    output.textures_delta.clear();
    output
        .shapes
        .iter()
        .filter(|shape| !matches!(shape.shape, egui::Shape::Noop))
        .count()
}

fn mask_center(editor: &Editor) -> egui::Pos2 {
    let source = editor
        .document
        .project
        .source(editor.document.visit.current_source.unwrap())
        .unwrap();
    let super::super::adjustment_layers::Target::Layer(id) = editor.adjustment_layer_target()
    else {
        panic!("selected layer");
    };
    let mask = source
        .adjustment_layers
        .as_ref()
        .unwrap()
        .layers
        .iter()
        .find(|record| record.id == id)
        .unwrap()
        .adjustment
        .mask;
    let point = match mask {
        AdjustmentLayerCoverage::Linear { start, end } => {
            SourcePoint([(start.0[0] + end.0[0]) / 2., (start.0[1] + end.0[1]) / 2.])
        }
        AdjustmentLayerCoverage::Radial { center, .. } => center,
        _ => panic!("gradient"),
    };
    let layout = editor.active_canvas().last_layout.unwrap().0;
    layout.screen_frame_point(layout.composition.source_to_frame(point))
}

#[test]
fn collapsed_layers_hide_all_mask_artwork_and_do_not_intercept_navigation() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        for name in ["Sky", "Foreground"] {
            click(&mut harness, name);
            if !harness.read(|editor| editor.document.visit.adjustment_layers.show_coverage) {
                click(&mut harness, "Mask overlay");
            }
            harness.read(|editor| {
                assert!(mask_artwork(editor) > 0);
                let point = mask_center(editor);
                let layout = editor.active_canvas().last_layout.unwrap().0;
                assert!(layout.canvas.contains(point));
                assert!(editor.mask_pointer_interest(layout, point));
            });
            let before = harness.read(|editor| editor.document.state.project.clone());
            click(&mut harness, "Layers");
            let point =
                harness.read(|editor| {
                    assert_eq!(mask_artwork(editor), 0);
                    assert!(editor.document.visit.adjustment_layers.show_coverage);
                    let point = mask_center(editor);
                    assert!(!editor.mask_pointer_interest(
                        editor.active_canvas().last_layout.unwrap().0,
                        point
                    ));
                    point
                });
            super::masks::hold_drag(&mut harness, point, point + egui::vec2(18., 12.));
            super::masks::pointer(&mut harness, point + egui::vec2(18., 12.), false);
            harness.read(|editor| {
                assert!(editor.document.visit.mask_edit.is_none());
                assert_eq!(editor.document.state.project, before);
            });
            click(&mut harness, "Layers");
            harness.read(|editor| {
                assert!(mask_artwork(editor) > 0);
                assert!(editor.document.visit.adjustment_layers.show_coverage);
            });
        }
    });
}

#[test]
fn value_adjustments_temporarily_hide_masks_and_restore_the_coverage_preference() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(install_layers);
        harness.step();
        show_layers(&mut harness);
        for name in ["Sky", "Foreground"] {
            click(&mut harness, name);
            if !harness.read(|editor| editor.document.visit.adjustment_layers.show_coverage) {
                click(&mut harness, "Mask overlay");
            }
            let slider = harness.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure");
            slider.scroll_to_me();
            harness.try_run_realtime().unwrap();
            let point = harness
                .get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
                .rect()
                .center();
            let before =
                harness.read(|editor| editor.document.visit.adjustment_layer_settings.exposure);
            harness.read(|editor| assert!(mask_artwork(editor) > 0));
            harness.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
            harness.hover_at(point);
            harness.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::SHIFT,
            });
            harness.step();
            harness.run_steps(3);
            harness.read(|editor| {
                assert_eq!(
                    editor.document.visit.adjustment_layer_settings.exposure,
                    before
                );
                assert_eq!(mask_artwork(editor), 0);
                assert!(editor.document.visit.adjustment_layers.show_coverage);
            });
            let end = point + egui::vec2(18., 0.);
            harness.hover_at(end);
            harness.step();
            harness.read(|editor| {
                assert_ne!(
                    editor.document.visit.adjustment_layer_settings.exposure,
                    before
                );
                assert_eq!(mask_artwork(editor), 0);
            });
            harness.event(egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::SHIFT,
            });
            harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            harness.step();
            harness.try_run_realtime().unwrap();
            harness.read(|editor| assert!(mask_artwork(editor) > 0));

            pending_exposure(&mut harness, "0.375");
            assert!(harness.ctx.text_edit_focused());
            harness.read(|editor| assert_eq!(mask_artwork(editor), 0));
            harness.event(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            harness.try_run_realtime().unwrap();
            harness.read(|editor| {
                assert_eq!(
                    editor.document.visit.adjustment_layer_settings.exposure,
                    0.375
                );
                assert!(mask_artwork(editor) > 0);
            });

            harness.get_by_label("White balance wheel").scroll_to_me();
            harness.try_run_realtime().unwrap();
            let point = harness.get_by_label("White balance wheel").rect().center();
            super::masks::hold_drag(&mut harness, point, point + egui::vec2(2., -1.));
            harness.read(|editor| assert_eq!(mask_artwork(editor), 0));
            super::masks::pointer(&mut harness, point + egui::vec2(2., -1.), false);
            harness.try_run_realtime().unwrap();
            harness.read(|editor| {
                assert!(mask_artwork(editor) > 0);
                assert!(editor.document.visit.adjustment_layers.show_coverage);
            });
        }
    });
}

#[test]
fn collapsed_layers_keep_active_gradient_creation_visible_until_release() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        for name in ["Add linear", "Add radial"] {
            show_layers(&mut harness);
            let before = harness.read(|editor| layers(editor).layers.len());
            click(&mut harness, name);
            click(&mut harness, "Layers");
            let start = harness.read(|editor| {
                editor
                    .document
                    .visit
                    .canvas
                    .last_layout
                    .unwrap()
                    .0
                    .image
                    .center()
            });
            let end = start + egui::vec2(30., 24.);
            super::masks::hold_drag(&mut harness, start, end);
            harness.read(|editor| {
                assert!(editor.document.visit.mask_edit.is_some());
                assert!(mask_artwork(editor) > 0);
            });
            super::masks::pointer(&mut harness, end, false);
            harness.try_run_realtime().unwrap();
            harness.read(|editor| {
                assert!(editor.document.visit.mask_edit.is_none());
                assert_eq!(layers(editor).layers.len(), before + 1);
                assert_eq!(mask_artwork(editor), 0);
            });
        }
    });
}
