use super::workflow::enter_numeric;
use super::*;

fn setup<R>(test: impl FnOnce(Harness<'_, Editor>) -> R) -> R {
    editor_harness(|mut h, _| {
        h.enter(|editor, cx| {
            editor.add_raws(vec![fixture("roll-0.dng"), fixture("roll-1.dng")], cx)
        });
        wait_for(&mut h, |e| !e.busy);
        h.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        h.update(|e| {
            e.set_dmin(Some(DVec3::splat(0.2)));
            e.set_dmax(Some(DVec3::splat(0.005)));
            e.source = false;
            e.sync_document();
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
    wait_for(h, |e| !e.busy);
}

fn native_action(h: &mut Harness<'_, Editor>, action: crate::platform::menu::Action) {
    h.update(|editor| host::App::menu_action(editor, action));
    h.step();
    wait_for(h, |editor| !editor.busy);
}

#[test]
fn native_history_actions_route_to_text_first_and_then_restore_the_accepted_photo() {
    use crate::platform::menu::Action;
    setup(|mut h| {
        h.update(host::App::native_menu_installed);
        let original = h.read(|editor| editor.document.project.clone());
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
        h.read(|editor| assert_eq!(editor.document.project, original));
        h.key_press(egui::Key::Enter);
        h.try_run_realtime().unwrap();
        h.read(|editor| {
            assert_eq!(
                editor.photo.exposure, 0.,
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
        h.read(|editor| assert_eq!(editor.photo.exposure, 1.5));
        let accepted = h.read(|editor| editor.document.project.clone());
        h.read(|editor| {
            assert_eq!(
                editor.application_menu_state().label(Action::Undo),
                "Undo Photo settings"
            )
        });
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.project, original));
        h.read(|editor| {
            assert_eq!(
                editor.application_menu_state().label(Action::Redo),
                "Redo Photo settings"
            )
        });
        native_action(&mut h, Action::Redo);
        h.read(|editor| assert_eq!(editor.document.project, accepted));
        h.update(|editor| editor.begin_crop());
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert!(editor.document.visit.crop_edit.is_some()));
        h.read(|editor| assert_eq!(editor.document.project, accepted));
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
        let held = h.read(|editor| editor.document.project.clone());
        h.read(|editor| assert_ne!(editor.photo.exposure, 0.75));
        h.read(|editor| assert!(!editor.application_menu_state().enabled(Action::Undo)));
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.project, held));
        h.read(|editor| assert!(editor.document_pending.is_none()));
        h.event(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.step();
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.photo.exposure, 0.75));

        stage(&mut h, "Rendering");
        h.get_by(|node| {
            node.role() == egui::accesskit::Role::ComboBox
                && node.value().as_deref() == Some("Display P3")
        })
        .click();
        h.step();
        assert!(egui::Popup::is_any_open(&h.ctx));
        let before_popup_undo = h.read(|editor| editor.document.project.clone());
        h.read(|editor| assert!(!editor.application_menu_state().enabled(Action::Undo)));
        native_action(&mut h, Action::Undo);
        assert!(egui::Popup::is_any_open(&h.ctx));
        h.read(|editor| assert_eq!(editor.document.project, before_popup_undo));
        h.read(|editor| assert!(editor.document_pending.is_none()));
        h.key_press(egui::Key::Escape);
        h.step();
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.photo.exposure, 0.));
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
        let original = h.read(|editor| editor.document.project.clone());
        h.enter(|editor, cx| editor.begin_look(look_ui::Source::Loaded(chosen.clone()), cx));
        wait_for(&mut h, |e| !e.busy);
        h.read(|editor| assert!(editor.failure.is_none(), "{}", editor.status));
        let accepted = h.read(|editor| editor.document.project.clone());
        assert!(Arc::ptr_eq(
            accepted.output.look.as_ref().unwrap().lut(),
            chosen.lut()
        ));
        h.read(|editor| {
            assert_eq!(
                editor.application_menu_state().label(Action::Undo),
                "Undo Creative look"
            )
        });
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.project, original));
        native_action(&mut h, Action::Redo);
        h.read(|editor| assert_eq!(editor.document.project, accepted));
        h.read(|editor| {
            assert!(Arc::ptr_eq(
                editor.document.project.output.look.as_ref().unwrap().lut(),
                chosen.lut()
            ))
        });
        h.update(|editor| editor.clear_look());
        h.step();
        h.read(|editor| assert!(editor.document.project.output.look.is_none()));
        native_action(&mut h, Action::Undo);
        h.read(|editor| assert_eq!(editor.document.project, accepted));
        h.render().unwrap();
    });
}

#[test]
fn history_idle_navigation_and_cancelled_input_do_not_clone_repair_bearing_projects() {
    setup(|mut h| {
        h.update(|e| {
            let source = e.current_source.unwrap();
            let repairs = e.dust_repairs_mut(source);
            let support = crate::dust_repair::SupportMask::new(
                [128, 128],
                [64, 64],
                (0..4096).map(|index| index % 2 == 0).collect(),
            )
            .unwrap();
            let repair = repairs
                .insert(
                    support,
                    crate::dust_repair::RepairOrigin::Manual,
                    crate::dust_repair::DonorChoice::Automatic,
                )
                .unwrap();
            repairs.set_enabled(repair, false).unwrap();
            e.history_snapshot_clones = 0;
        });
        let original = h.read(|editor| editor.document.project.clone());
        for _ in 0..3 {
            h.step();
        }
        stage(&mut h, "Roll");
        stage(&mut h, "Photo");
        h.render().unwrap();
        h.read(|editor| {
            assert_eq!(
                editor.history_snapshot_clones, 0,
                "idle and navigation passes must not copy the project"
            )
        });

        h.get_by_label("Crop & straighten").click();
        h.step();
        h.read(|editor| assert!(editor.document.visit.crop_edit.is_some()));
        h.update(|editor| editor.history_snapshot_clones = 0);
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
        h.read(|editor| assert_eq!(editor.document.project, original));
        h.read(|editor| {
            assert_eq!(
                editor.history_snapshot_clones, 0,
                "draft and cancelled numeric passes must not copy the project"
            )
        });

        enter_numeric(&mut h, "Exposure", "0.75");
        h.read(|editor| assert_eq!(editor.photo.exposure, 0.75));
        h.read(|editor| {
            assert_eq!(
                editor.history_snapshot_clones, 2,
                "a scalar edit retains one before/after pair"
            )
        });
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, original));
    });
}

#[test]
fn history_slider_drag_is_one_edit_and_redo_restores_the_exact_accepted_grade() {
    setup(|mut h| {
        let before = h.read(|editor| editor.document.project.clone());
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
        let after = h.read(|editor| editor.document.project.clone());
        assert_ne!(after, before);
        shortcut(&mut h, false);
        h.read(|editor| {
            assert_eq!(
                editor.document.project, before,
                "one undo must reverse the entire drag"
            )
        });
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.project, after));
        h.render().unwrap();
    });
}

#[test]
fn history_focus_loss_commits_numeric_text_separately_from_an_orientation_command() {
    setup(|mut h| {
        let initial = h.read(|editor| editor.document.project.clone());
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .click();
        h.step();
        h.event(egui::Event::Text("1.25".into()));
        h.step();
        h.read(|editor| assert_eq!(editor.photo.exposure, 0., "text is still pending"));
        h.get_by_label("Turn right").click();
        h.step();
        h.read(|editor| assert_eq!(editor.photo.exposure, 1.25));
        h.read(|editor| assert_ne!(editor.photo_orientation, PhotoOrientation::default()));
        shortcut(&mut h, false);
        h.read(|editor| {
            assert_eq!(
                editor.photo.exposure, 1.25,
                "undo rotation must retain the committed numeric edit"
            )
        });
        h.read(|editor| assert_eq!(editor.photo_orientation, PhotoOrientation::default()));
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, initial));
    });
}

#[test]
fn history_focus_loss_numeric_commit_is_separate_from_section_resets() {
    setup(|mut h| {
        enter_numeric(&mut h, "Exposure", "0.5");
        {
            h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .click();
            h.step();
            h.event(egui::Event::Text("1.25".into()));
            h.step();
            let reset = h
                .get_all_by_role_and_label(egui::accesskit::Role::Button, "Reset Exposure")
                .last()
                .unwrap();
            reset.click();
            h.step();
            h.read(|editor| assert_eq!(editor.photo.exposure, 0.));
            shortcut(&mut h, false);
            h.read(|editor| {
                assert_eq!(
                    editor.photo.exposure, 1.25,
                    "reset undo must retain the preceding numeric commit"
                )
            });
            shortcut(&mut h, false);
            h.read(|editor| assert_eq!(editor.photo.exposure, 0.5));
        }
    });
}

#[test]
fn history_confirmed_multi_removal_can_restore_an_empty_roll_without_resetting_its_basis() {
    setup(|mut h| {
        h.update(|editor| {
            let id = editor.document.project.order[0];
            let source = editor.document.project.source_mut(id).unwrap();
            source.photo.exposure = 0.75;
            editor.photo.exposure = 0.75;
            source
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new([100, 100], [2, 2], vec![true; 4])
                        .unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    crate::dust_repair::DonorChoice::Automatic,
                )
                .unwrap();
        });
        let before = h.read(|editor| editor.document.project.clone());
        h.get_by_label("roll-1.dng")
            .click_modifiers(egui::Modifiers::COMMAND);
        h.step();
        wait_for(&mut h, |e| !e.busy);
        h.key_press(egui::Key::Delete);
        h.step();
        h.read(|editor| assert!(editor.removal_pending.is_some()));
        h.get_by_label("Remove photos").click();
        h.step();
        wait_for(&mut h, |e| !e.busy);
        h.read(|editor| assert!(editor.document.project.order.is_empty()));
        h.read(|editor| assert!(editor.document.project.sources.is_empty()));
        h.read(|editor| assert_eq!(editor.document.project.development(), None));
        let removed = h.read(|editor| editor.document.project.clone());
        shortcut(&mut h, false);
        h.read(|editor| assert_eq!(editor.document.project, before));
        h.read(|editor| assert!(editor.document.visit.ready.is_some()));
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.project, removed));
        h.read(|editor| assert!(editor.document.visit.ready.is_none()));
        h.render().unwrap();
    });
}

#[test]
fn history_numeric_commit_navigation_menu_and_crop_draft_respect_input_ownership() {
    setup(|mut h| {
        let edited_photo = h.read(|editor| editor.current_source);
        enter_numeric(&mut h, "Exposure", "7.123456789012345");
        h.read(|editor| assert_eq!(editor.photo.exposure, 7.123456789012345));
        let accepted = h.read(|editor| editor.document.project.clone());
        h.get_by_label("roll-1.dng").click();
        h.step();
        wait_for(&mut h, |e| !e.busy);
        h.read(|editor| assert_ne!(editor.current_source, edited_photo));
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
        h.read(|editor| assert_eq!(editor.current_source, edited_photo));
        h.read(|editor| assert_eq!(editor.photo.exposure, 0.));
        shortcut(&mut h, true);
        h.read(|editor| assert_eq!(editor.document.project, accepted));
        h.get_by_label("Crop & straighten").click();
        h.step();
        h.read(|editor| assert!(editor.document.visit.crop_edit.is_some()));
        shortcut(&mut h, false);
        h.read(|editor| assert!(editor.document.visit.crop_edit.is_some()));
        h.read(|editor| assert_eq!(editor.document.project, accepted));
        h.get_by_label("Cancel").click();
        h.step();
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
        h.read(|editor| assert_eq!(editor.document.project, accepted));
        h.render().unwrap();
    });
}
