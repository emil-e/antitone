use super::workflow::enter_numeric;
use super::*;

#[test]
fn numeric_text_stays_private_and_opening_crop_can_retire_it_without_corrupting_the_project() {
    for complete in [false, true] {
        setup(|mut h| {
            let original = h.read(|editor| editor.document.project.clone());
            let source = h.read(|editor| editor.document.visit.current_source.unwrap());
            let number = egui::accesskit::Role::SpinButton;
            h.get_by_role_and_label(number, "Exposure").scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(number, "Exposure").click();
            h.step();
            h.event(egui::Event::Text("1.25".into()));
            h.step();
            h.read(|editor| {
                assert_eq!(editor.document.project, original);
                assert_eq!(
                    editor.photo_settings().exposure,
                    0.,
                    "numeric text is private until completion"
                );
            });
            if complete {
                h.key_press(egui::Key::Enter);
                h.step();
                h.read(|editor| assert_eq!(editor.photo_settings().exposure, 1.25));
            }
            h.get_by_label("Crop & straighten").scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_label("Crop & straighten").click();
            h.try_run_realtime().unwrap();
            let accepted = h.read(|editor| {
                assert!(editor.document.crop_edit().is_some(), "{}", editor.status);
                let mut expected = original.clone();
                let exposure = editor.photo_settings().exposure;
                if complete {
                    assert_eq!(exposure, 1.25);
                } else {
                    assert!(exposure == 0. || exposure == 1.25);
                }
                expected
                    .source_mut(source)
                    .unwrap()
                    .adjustment_layers
                    .as_mut()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo
                    .exposure = exposure;
                assert_eq!(
                    editor.document.project, expected,
                    "no unrelated state changes on tool activation"
                );
                editor.document.project.clone()
            });
            h.get_by_label("Cancel").click();
            h.try_run_realtime().unwrap();
            h.read(|editor| assert_eq!(editor.document.project, accepted));
            if accepted != original {
                shortcut(&mut h, false);
                h.read(|editor| assert_eq!(editor.document.project, original));
            }
        });
    }
}

#[test]
fn a_wheel_previews_and_accepts_from_a_completed_numeric_baseline_with_separate_undo() {
    setup(|mut h| {
        let original = h.read(|editor| editor.document.project.clone());
        enter_numeric(&mut h, "Warmth", "0.25");
        let first_accepted = h.read(|editor| {
            assert_eq!(
                editor.photo_settings().wb,
                Balance {
                    warmth: 0.25,
                    tint: 0.
                }
            );
            editor.document.project.clone()
        });
        h.get_by_label("White balance wheel").scroll_to_me();
        h.try_run_realtime().unwrap();
        let start = h.get_by_label("White balance wheel").rect().center();
        let end = start + egui::vec2(12., -7.);
        h.input_mut().events.extend([
            egui::Event::PointerMoved(start),
            egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerMoved(end),
        ]);
        h.step();
        h.step();
        h.read(|editor| {
            assert_eq!(editor.document.project, first_accepted);
            assert_ne!(
                editor.photo_settings().wb,
                Balance {
                    warmth: 0.25,
                    tint: 0.
                },
                "the wheel must actually preview a change"
            );
        });
        h.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.read(|editor| assert_ne!(editor.document.project, first_accepted));
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, first_accepted));
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, original));
    });
}

#[test]
fn dragging_a_numeric_value_keeps_one_edit_through_a_pause_and_release() {
    setup(|mut h| {
        let original = h.read(|editor| editor.document.project.clone());
        let numeric = egui::accesskit::Role::SpinButton;
        h.get_by_role_and_label(numeric, "Exposure").scroll_to_me();
        h.try_run_realtime().unwrap();
        let start = h.get_by_role_and_label(numeric, "Exposure").rect().center();
        h.event(egui::Event::PointerMoved(start));
        h.event(egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        let middle = start + egui::vec2(18., 0.);
        h.event(egui::Event::PointerMoved(middle));
        h.step();
        let preview = h.read(|editor| {
            assert_ne!(
                editor.photo_settings().exposure,
                0.,
                "the numeric drag actually changes the preview"
            );
            assert_eq!(editor.document.project, original);
            assert!(editor.editing());
            assert!(!editor.file_available());
            editor.photo_settings().exposure
        });
        for _ in 0..3 {
            h.step();
        }
        h.read(|editor| {
            assert_eq!(editor.photo_settings().exposure, preview);
            assert_eq!(editor.document.project, original);
            assert!(
                editor.editing(),
                "stationary pointer still owns the same edit"
            );
        });
        let end = middle + egui::vec2(12., 0.);
        h.event(egui::Event::PointerMoved(end));
        h.step();
        let preview = h.read(|editor| {
            assert!(editor.photo_settings().exposure > preview);
            assert_eq!(editor.document.project, original);
            editor.photo_settings().exposure
        });
        h.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        let accepted = h.read(|editor| {
            assert_eq!(editor.photo_settings().exposure, preview);
            assert!(
                !editor.editing(),
                "release completes the numeric-owned edit"
            );
            assert!(editor.file_available());
            editor.document.project.clone()
        });
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, original));
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.project, accepted));
    });
}

#[test]
fn a_precision_track_preserves_a_completed_numeric_baseline_and_has_its_own_undo() {
    setup(|mut h| {
        let original = h.read(|editor| editor.document.project.clone());
        enter_numeric(&mut h, "Exposure", "1.25");
        let first_accepted = h.read(|editor| {
            assert_eq!(editor.photo_settings().exposure, 1.25);
            editor.document.project.clone()
        });
        let track = h
            .get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
            .rect();
        let start = track.center();
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::SHIFT));
        h.event(egui::Event::PointerMoved(start));
        h.event(egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::SHIFT,
        });
        h.step();
        h.read(|editor| {
            assert_eq!(
                editor.photo_settings().exposure,
                1.25,
                "a precision press holds the baseline"
            )
        });
        let end = start + egui::vec2(20., 0.);
        h.event(egui::Event::PointerMoved(end));
        h.step();
        h.read(|editor| {
            assert!(
                editor.photo_settings().exposure > 1.25,
                "rightward fine travel increases the baseline"
            );
            assert_eq!(editor.document.project, first_accepted);
        });
        h.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::SHIFT,
        });
        h.step();
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.step();
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, first_accepted));
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, original));
    });
}

#[test]
fn document_fields_accept_each_value_before_starting_the_next_in_both_paint_orders() {
    for (first, text, second, next) in [
        ("Gamma", "1.1", "Exposure", "0.75"),
        ("Exposure", "0.75", "Gamma", "1.1"),
    ] {
        setup(|mut h| {
            stage(&mut h, "Roll");
            let role = egui::accesskit::Role::SpinButton;
            if h.query_by_role_and_label(role, "Exposure").is_none() {
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
                    .scroll_to_me();
                h.try_run_realtime().unwrap();
                h.get_by_role_and_label(egui::accesskit::Role::Button, "Exposure")
                    .click();
                h.try_run_realtime().unwrap();
            }
            let original = h.read(|editor| editor.document.project.clone());
            h.get_by_role_and_label(role, first).scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(role, first).click();
            h.step();
            h.event(egui::Event::Text(text.into()));
            h.step();
            h.read(|editor| assert_eq!(editor.document.project, original));
            h.get_by_role_and_label(role, second).scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(role, second).click();
            h.step();
            let first_accepted = h.read(|editor| {
                if first == "Gamma" {
                    assert_eq!(editor.document.project.input.gamma(), 1.1);
                    assert_eq!(
                        editor.document.project.roll.exposure,
                        original.roll.exposure
                    );
                } else {
                    assert_eq!(editor.document.project.roll.exposure, 0.75);
                    assert_eq!(editor.document.project.input, original.input);
                }
                assert_eq!(editor.document.project.sources, original.sources);
                editor.document.project.clone()
            });
            h.event(egui::Event::Text(next.into()));
            h.step();
            h.read(|editor| assert_eq!(editor.document.project, first_accepted));
            h.key_press(egui::Key::Enter);
            h.step();
            h.read(|editor| {
                assert_eq!(editor.document.project.input.gamma(), 1.1);
                assert_eq!(editor.document.project.roll.exposure, 0.75);
                assert_eq!(editor.document.project.sources, original.sources);
            });
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.document.project, first_accepted));
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.document.project, original));
        });
    }
}

#[test]
fn numeric_focus_transfer_accepts_each_field_with_independent_undo_in_both_paint_orders() {
    for (first, text, second, next) in [
        ("Exposure", "1.25", "Contrast", "1.4"),
        ("Contrast", "1.4", "Exposure", "1.25"),
    ] {
        setup(|mut h| {
            let original = h.read(|editor| editor.document.project.clone());
            let role = egui::accesskit::Role::SpinButton;
            h.get_by_role_and_label(role, first).scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(role, first).click();
            h.step();
            h.event(egui::Event::Text(text.into()));
            h.step();
            h.read(|editor| assert_eq!(editor.document.project, original));
            h.get_by_role_and_label(role, second).scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(role, second).click();
            h.step();
            let first_accepted = h.read(|editor| {
                let id = editor.document.visit.current_source.unwrap();
                let values = editor
                    .document
                    .project
                    .source(id)
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo;
                if first == "Exposure" {
                    assert_eq!(values.exposure, 1.25);
                    assert_eq!(values.contrast, 1.);
                } else {
                    assert_eq!(values.contrast, 1.4);
                    assert_eq!(values.exposure, 0.);
                }
                editor.document.project.clone()
            });
            h.event(egui::Event::Text(next.into()));
            h.step();
            h.read(|editor| assert_eq!(editor.document.project, first_accepted));
            h.key_press(egui::Key::Enter);
            h.step();
            h.read(|editor| {
                assert_eq!(editor.photo_settings().exposure, 1.25);
                assert_eq!(editor.photo_settings().contrast, 1.4);
                assert_ne!(editor.document.project, first_accepted);
            });
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.document.project, first_accepted));
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.document.project, original));
        });
    }
}

#[test]
fn a_track_drag_after_numeric_completion_has_separate_preview_acceptance_and_undo() {
    for (label, fraction) in [("Contrast", 0.8), ("Exposure", 0.8), ("Exposure", 0.5)] {
        setup(|mut h| {
            let original = h.read(|editor| editor.document.project.clone());
            let role = egui::accesskit::Role::SpinButton;
            h.get_by_role_and_label(role, "Exposure").scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_role_and_label(role, "Exposure").click();
            h.step();
            enter_numeric(&mut h, "Exposure", "1.25");
            h.get_by_role_and_label(egui::accesskit::Role::Slider, label)
                .scroll_to_me();
            h.try_run_realtime().unwrap();
            let track = h
                .get_by_role_and_label(egui::accesskit::Role::Slider, label)
                .rect();
            let point = egui::pos2(track.left() + track.width() * fraction, track.center().y);
            h.event(egui::Event::PointerMoved(point));
            h.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            let first_accepted = h.read(|editor| {
                let id = editor.document.visit.current_source.unwrap();
                let values = editor
                    .document
                    .project
                    .source(id)
                    .unwrap()
                    .adjustment_layers
                    .as_ref()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo;
                assert_eq!(values.exposure, 1.25);
                assert_eq!(values.contrast, 1.);
                assert!(editor.editing());
                assert_ne!(
                    editor.photo_settings(),
                    values,
                    "the admitted track must supply a distinct preview"
                );
                editor.document.project.clone()
            });
            h.event(egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            h.read(|editor| {
                assert!(!editor.editing());
                assert_ne!(editor.document.project, first_accepted);
            });
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.document.project, first_accepted));
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.document.project, original));
        });
    }
}

fn setup<R>(test: impl FnOnce(Harness<'_, Editor>) -> R) -> R {
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| {
            editor.add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")], cx)
        });
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|editor| {
            assert!(
                editor.document.visit.canvas.ready.is_some(),
                "{}",
                editor.status
            )
        });
        h.update(|e| {
            e.set_dmin(Some(DVec3::splat(0.2)));
            e.set_dmax(Some(DVec3::splat(0.005)));
            e.document.source = false;
        });
        stage(&mut h, "Photo");
        test(h)
    })
}

fn shortcut(h: &mut Harness<'_, Editor>, redo: bool) {
    h.key_press_modifiers(
        if redo {
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::COMMAND
        },
        egui::Key::Z,
    );
    h.step();
    wait_for(h, |e| !e.interaction_blocked());
}

fn native_action(h: &mut Harness<'_, Editor>, action: crate::platform::menu::Action) {
    h.update(|editor| host::App::menu_action(editor, action));
    h.step();
    wait_for(h, |editor| !editor.interaction_blocked());
}

#[test]
fn native_history_actions_route_to_text_first_and_then_restore_the_accepted_photo() {
    use crate::platform::menu::Action;
    setup(|mut h| {
        h.update(host::App::native_menu_installed);
        let original = h.read(|editor| editor.document.state.project.clone());
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        h.step();
        h.event(egui::Event::Text("1.25".into()));
        h.step();
        assert!(h.ctx.text_edit_focused());
        h.read(|editor| assert_eq!(editor.application_menu_state().label(Action::Undo), "Undo"));
        native_action(&mut h, Action::Undo);
        assert!(h.ctx.text_edit_focused());
        h.read(|editor| assert_eq!(editor.document.state.project, original));
        h.key_press(egui::Key::Enter);
        h.try_run_realtime().unwrap();
        h.read(|editor| {
            assert_eq!(
                editor.photo_settings().exposure,
                0.,
                "native Undo must change the pending text, not merely leave the project alone"
            )
        });

        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        h.step();
        h.event(egui::Event::Text("1.5".into()));
        h.step();
        native_action(&mut h, Action::Undo);
        native_action(&mut h, Action::Redo);
        h.key_press(egui::Key::Enter);
        h.try_run_realtime().unwrap();
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 1.5));
        let accepted = h.read(|editor| editor.document.state.project.clone());
        h.read(|editor| {
            assert_eq!(
                editor.application_menu_state().label(Action::Undo),
                "Undo Photo settings"
            )
        });
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.state.project, original));
        h.read(|editor| {
            assert_eq!(
                editor.application_menu_state().label(Action::Redo),
                "Redo Photo settings"
            )
        });
        native_action(&mut h, Action::Redo);
        h.read(|editor| assert_eq!(editor.document.state.project, accepted));
        h.update(|editor| editor.begin_crop());
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert!(editor.document.crop_edit().is_none()));
        h.read(|editor| assert_eq!(editor.document.state.project, original));
    });
}

#[test]
fn native_history_actions_cannot_interrupt_a_held_slider_or_an_open_popup() {
    use crate::platform::menu::Action;
    setup(|mut h| {
        h.update(host::App::native_menu_installed);
        enter_numeric(&mut h, "Exposure", "0.75");
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        let rect = h
            .get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
            .rect();
        let start = rect.center();
        let end = start + egui::vec2(rect.width() * 0.2, 0.);
        h.hover_at(start);
        h.event(egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        h.hover_at(end);
        h.step();
        let held = h.read(|editor| editor.document.state.project.clone());
        h.read(|editor| assert_ne!(editor.photo_settings().exposure, 0.75));
        h.read(|editor| assert!(!editor.application_menu_state().enabled(Action::Undo)));
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.state.project, held));
        h.read(|editor| assert!(editor.document_pending.is_none()));
        h.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.75));

        stage(&mut h, "Rendering");
        h.get_by(|node| {
            node.role() == egui::accesskit::Role::ComboBox
                && node.value().as_deref() == Some("Display P3")
        })
        .click();
        h.step();
        assert!(egui::Popup::is_any_open(&h.ctx));
        let before_popup_undo = h.read(|editor| editor.document.state.project.clone());
        h.read(|editor| assert!(!editor.application_menu_state().enabled(Action::Undo)));
        native_action(&mut h, Action::Undo);
        assert!(egui::Popup::is_any_open(&h.ctx));
        h.read(|editor| assert_eq!(editor.document.state.project, before_popup_undo));
        h.read(|editor| assert!(editor.document_pending.is_none()));
        h.key_press(egui::Key::Escape);
        h.step();
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.));
    });
}

#[test]
fn native_history_restores_accepted_creative_looks_and_shares_the_numeric_lut() {
    use crate::{
        looks::{LookAsset, LookOrigin, Lut3D},
        platform::menu::Action,
    };
    setup(|mut h| {
        h.update(host::App::native_menu_installed);
        let mut nodes = Vec::new();
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    nodes.push([r as f32, g as f32, b as f32]);
                }
            }
        }
        let chosen = Arc::new(
            LookAsset::new(
                "History look".into(),
                LookOrigin::Imported,
                Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], nodes).unwrap()),
            )
            .unwrap(),
        );
        let original = h.read(|editor| editor.document.state.project.clone());
        h.enter(|editor, cx| editor.select_look(look::Source::Loaded(chosen.clone()), cx));
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|editor| assert!(!editor.dialogs.is_open(), "{}", editor.status));
        let accepted = h.read(|editor| editor.document.state.project.clone());
        assert!(Arc::ptr_eq(
            accepted.output.lut.as_ref().unwrap().lut(),
            chosen.lut()
        ));
        h.read(|editor| {
            assert_eq!(
                editor.application_menu_state().label(Action::Undo),
                "Undo LUT"
            )
        });
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.state.project, original));
        native_action(&mut h, Action::Redo);
        h.read(|editor| assert_eq!(editor.document.state.project, accepted));
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                editor
                    .document
                    .state
                    .project
                    .output
                    .lut
                    .as_ref()
                    .unwrap()
                    .lut(),
                chosen.lut()
            ))
        });
        h.update(|editor| editor.clear_look());
        h.step();
        h.read(|editor| assert!(editor.document.state.project.output.lut.is_none()));
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.state.project, accepted));
        h.render().unwrap();
    });
}

#[test]
fn history_idle_navigation_and_cancelled_input_preserve_repair_bearing_versions() {
    setup(|mut h| {
        h.update(|e| {
            let source = e.document.visit.current_source.unwrap();
            let repairs = e.dust_repairs_mut(source);
            let support = crate::dust_repair::SupportMask::new(
                [128, 128],
                [64, 64],
                (0..4096).map(|index| index % 2 == 0).collect(),
            )
            .unwrap();
            let repair = repairs
                .insert(support, crate::dust_repair::RepairOrigin::Manual, [84, 0])
                .unwrap();
            repairs.set_enabled(repair, false).unwrap();
        });
        let original = h.read(|editor| editor.document.state.project.clone());
        let original_undo =
            h.read(|editor| editor.history_label(super::super::history::Direction::Undo));
        for _ in 0..3 {
            h.step();
        }
        stage(&mut h, "Roll");
        stage(&mut h, "Photo");
        h.render().unwrap();
        h.read(|editor| {
            assert_eq!(editor.document.project, original);
            assert_eq!(
                editor.history_label(super::super::history::Direction::Undo),
                original_undo
            );
        });

        h.get_by_label("Crop & straighten").click();
        h.step();
        h.read(|editor| assert!(editor.document.crop_edit().is_some()));
        for _ in 0..3 {
            h.step();
        }
        h.get_by_label("Cancel").click();
        h.step();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        h.step();
        h.event(egui::Event::Text("2.5".into()));
        h.step();
        h.key_press(egui::Key::Escape);
        h.step();
        h.try_run_realtime().unwrap();
        h.read(|editor| assert_eq!(editor.document.state.project, original));
        h.read(|editor| {
            assert_eq!(
                editor.history_label(super::super::history::Direction::Undo),
                original_undo
            )
        });

        enter_numeric(&mut h, "Exposure", "0.75");
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.75));
        h.read(|editor| {
            assert_eq!(
                editor.history_label(super::super::history::Direction::Undo),
                Some("Photo settings")
            )
        });
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.state.project, original));
    });
}

#[test]
fn history_slider_drag_is_one_edit_and_redo_restores_the_exact_accepted_grade() {
    setup(|mut h| {
        let before = h.read(|editor| editor.document.state.project.clone());
        h.get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        let rect = h
            .get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
            .rect();
        let start = rect.center();
        h.hover_at(start);
        h.event(egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        for fraction in [0.1, 0.2, 0.3] {
            h.hover_at(start + egui::vec2(rect.width() * fraction, 0.));
            h.step();
        }
        let end = start + egui::vec2(rect.width() * 0.3, 0.);
        h.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        let after = h.read(|editor| editor.document.state.project.clone());
        assert_ne!(after, before);
        shortcut(&mut h, false);
        h.read(|editor| {
            assert_eq!(
                editor.document.state.project, before,
                "one undo must reverse the entire drag"
            )
        });
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.state.project, after));
        h.render().unwrap();
    });
}

#[test]
fn completed_numeric_and_orientation_edits_have_independent_undo() {
    setup(|mut h| {
        let initial = h.read(|editor| editor.document.state.project.clone());
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        h.step();
        h.event(egui::Event::Text("1.25".into()));
        h.step();
        h.read(|editor| {
            assert_eq!(
                editor.photo_settings().exposure,
                0.,
                "text is still pending"
            )
        });
        h.key_press(egui::Key::Enter);
        h.step();
        wait_for(&mut h, |editor| !editor.editing());
        h.get_by_label("Turn right").click();
        h.step();
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 1.25));
        h.read(|editor| assert_ne!(editor.photo_orientation(), PhotoOrientation::default()));
        shortcut(&mut h, false);
        h.read(|editor| {
            assert_eq!(
                editor.photo_settings().exposure,
                1.25,
                "undo rotation must retain the committed numeric edit"
            )
        });
        h.read(|editor| assert_eq!(editor.photo_orientation(), PhotoOrientation::default()));
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.state.project, initial));
    });
}

#[test]
fn completed_numeric_edits_and_section_resets_have_independent_undo() {
    setup(|mut h| {
        enter_numeric(&mut h, "Exposure", "0.5");
        {
            h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .click();
            h.step();
            h.event(egui::Event::Text("1.25".into()));
            h.step();
            h.key_press(egui::Key::Enter);
            h.step();
            wait_for(&mut h, |editor| !editor.editing());
            let reset = h
                .get_all_by_role_and_label(egui::accesskit::Role::Button, "Reset Exposure")
                .last()
                .unwrap();
            reset.click();
            h.step();
            h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.));
            shortcut(&mut h, false);
            h.read(|editor| {
                assert_eq!(
                    editor.photo_settings().exposure,
                    1.25,
                    "reset undo must retain the preceding numeric commit"
                )
            });
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.5));
        }
    });
}

#[test]
fn history_confirmed_multi_removal_can_restore_an_empty_roll_without_resetting_its_basis() {
    setup(|mut h| {
        h.update(|editor| {
            let id = editor.document.state.project.order[0];
            let source = editor.document.state.project.source_mut(id).unwrap();
            source.adjustment_layers.as_mut().unwrap().layers[0]
                .adjustment
                .photo
                .exposure = 0.75;
            source
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new([100, 100], [2, 2], vec![true; 4])
                        .unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    [64, 0],
                )
                .unwrap();
        });
        let before = h.read(|editor| editor.document.state.project.clone());
        h.get_by_label("roll-1.dng")
            .click_modifiers(egui::Modifiers::COMMAND);
        h.step();
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.key_press(egui::Key::Delete);
        h.step();
        h.read(|editor| assert!(editor.dialogs.is_open()));
        h.get_by_label("Remove photos").click();
        h.step();
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|editor| assert!(editor.document.state.project.order.is_empty()));
        h.read(|editor| assert!(editor.document.state.project.sources.is_empty()));
        h.read(|editor| assert_eq!(editor.document.state.project.development(), None));
        let removed = h.read(|editor| editor.document.state.project.clone());
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.state.project, before));
        h.read(|editor| assert!(editor.document.visit.canvas.ready.is_some()));
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.state.project, removed));
        h.read(|editor| assert!(editor.document.visit.canvas.ready.is_none()));
        h.render().unwrap();
    });
}

#[test]
fn history_numeric_commit_navigation_menu_and_crop_draft_respect_input_ownership() {
    setup(|mut h| {
        let edited_photo = h.read(|editor| editor.document.visit.current_source);
        enter_numeric(&mut h, "Exposure", "7.123456789012345");
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 7.123456789012345));
        let accepted = h.read(|editor| editor.document.state.project.clone());
        h.get_by_label("roll-1.dng").click();
        h.step();
        wait_for(&mut h, |e| !e.interaction_blocked());
        h.read(|editor| assert_ne!(editor.document.visit.current_source, edited_photo));
        h.read(|editor| {
            assert_eq!(
                editor
                    .application_menu_state()
                    .label(crate::platform::menu::Action::Undo),
                "Undo Photo settings",
                "the native menu names the discarded grade"
            )
        });
        native_action(&mut h, crate::platform::menu::Action::Undo);
        h.read(|editor| assert_eq!(editor.document.visit.current_source, edited_photo));
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.));
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.state.project, accepted));
        h.get_by_label("Crop & straighten").click();
        h.step();
        h.read(|editor| assert!(editor.document.crop_edit().is_some()));
        shortcut(&mut h, false);
        h.read(|editor| assert!(editor.document.crop_edit().is_none()));
        h.read(|editor| assert_eq!(editor.photo_settings().exposure, 0.));
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.state.project, accepted));
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        h.step();
        h.event(egui::Event::Text("2.5".into()));
        h.step();
        shortcut(&mut h, false);
        h.read(|editor| assert!(editor.document_pending.is_none(), "numeric text owns Cmd-Z"));
        h.read(|editor| assert_eq!(editor.document.state.project, accepted));
        h.render().unwrap();
    });
}
