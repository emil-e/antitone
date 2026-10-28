use super::*;
use crate::platform::menu::Action;

fn native_action(harness: &mut Harness<'_, Editor>, action: Action) {
    harness.update(|editor| host::App::menu_action(editor, action));
    harness.step();
    harness.try_run_realtime().unwrap();
}

#[test]
fn settings_close_commits_its_number_and_native_select_all_keeps_text_focus() {
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let store = preferences::Store::open(path.clone()).unwrap().0;
        harness.update(|editor| editor.preference_store = Some(store));
        let project = harness.read(|editor| editor.document.state.project.clone());
        harness.update(host::App::native_menu_installed);
        native_action(&mut harness, Action::Settings);
        let role = egui::accesskit::Role::SpinButton;
        harness.get_by_role_and_label(role, "CPU cache").click();
        harness.step();
        harness.event(egui::Event::Text("999".into()));
        harness.step();
        assert!(harness.ctx.text_edit_focused());
        native_action(&mut harness, Action::SelectAll);
        assert!(harness.ctx.text_edit_focused());
        harness.event(egui::Event::Text("37".into()));
        harness.step();
        assert!(!path.exists(), "editing has not committed yet");
        native_action(&mut harness, Action::Close);
        harness.read(|editor| assert!(editor.menus.panel.is_none()));
        harness.read(|editor| assert!(!editor.allow_close));
        harness.read(|editor| {
            assert_eq!(
                editor.document.thumbnails.limits.cpu_bytes,
                37 * cache_limits::MIB
            )
        });
        let saved_limits = preferences::Store::open(path).unwrap().1.cache_limits;
        harness.read(|editor| assert_eq!(saved_limits, Some(editor.document.thumbnails.limits)));
        harness.read(|editor| assert_eq!(editor.document.state.project, project));
    });
}

#[test]
fn native_paste_reaches_the_field_before_enter_commits_it() {
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        native_action(&mut harness, Action::Settings);
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "CPU cache")
            .click();
        harness.step();
        for action in [Action::SelectAll, Action::Paste] {
            harness.update(|editor| host::App::menu_action(editor, action));
        }
        let input = harness.update(|editor| host::App::menu_input(editor, Some("37".into())));
        for event in input {
            harness.event(event);
        }
        harness.key_press(egui::Key::Enter);
        harness.try_run_realtime().unwrap();
        harness.read(|editor| {
            assert_eq!(
                editor.document.thumbnails.limits.cpu_bytes,
                37 * cache_limits::MIB
            )
        });
        harness
            .read(|editor| assert_eq!(editor.menus.panel, Some(presentation::AppPanel::Settings)));
    });
}

#[test]
fn operation_failure_owns_escape_above_settings_without_closing_settings() {
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        native_action(&mut harness, Action::Settings);
        harness.update(|editor| {
            editor.report_failure(FailureKind::Export, "fixture export failure".into())
        });
        harness.try_run_realtime().unwrap();
        assert!(!harness.get_by_label("OK").accesskit_node().is_disabled());
        harness.key_press(egui::Key::Escape);
        harness.try_run_realtime().unwrap();
        harness.read(|editor| assert!(editor.failure.is_none()));
        harness
            .read(|editor| assert_eq!(editor.menus.panel, Some(presentation::AppPanel::Settings)));
    });
}

#[test]
fn settings_escape_cancels_numeric_text_before_dismissing_the_panel() {
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        native_action(&mut harness, Action::Settings);
        let before = harness.read(|editor| editor.document.thumbnails.limits);
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "CPU cache")
            .click();
        harness.step();
        harness.event(egui::Event::Text("999".into()));
        harness.step();
        harness.key_press(egui::Key::Escape);
        harness.try_run_realtime().unwrap();
        harness
            .read(|editor| assert_eq!(editor.menus.panel, Some(presentation::AppPanel::Settings)));
        harness.read(|editor| assert_eq!(editor.document.thumbnails.limits, before));
        harness.key_press(egui::Key::Escape);
        harness.try_run_realtime().unwrap();
        harness.read(|editor| assert!(editor.menus.panel.is_none()));
        harness.read(|editor| assert_eq!(editor.document.thumbnails.limits, before));
    });
}

#[test]
fn application_panels_can_be_closed_at_both_layouts_without_editing_the_document() {
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        let before = harness.read(|editor| editor.document.state.project.clone());
        harness.update(host::App::native_menu_installed);
        for (layout, size) in [
            ("normal", egui::vec2(1100., 800.)),
            ("narrow", egui::vec2(800., 600.)),
        ] {
            harness.set_size(size);
            for (action, name) in [
                (Action::Settings, "settings"),
                (Action::About, "about"),
                (Action::Help, "help"),
            ] {
                native_action(&mut harness, action);
                let close = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Close");
                assert!(!close.accesskit_node().is_disabled());
                assert!(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(close.rect())
                );
                let png = harness.render().unwrap();
                save_capture(&png, &format!("application-{name}-{layout}-offscreen"));
                harness
                    .get_by_role_and_label(egui::accesskit::Role::Button, "Close")
                    .click();
                harness.try_run_realtime().unwrap();
                harness.read(|editor| {
                    assert!(
                        editor.menus.panel.is_none(),
                        "{name} did not close at {layout}"
                    )
                });
                harness.read(|editor| assert_eq!(editor.document.state.project, before));
            }
        }
    });
}

#[test]
fn native_edit_actions_preserve_the_focused_numeric_control() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        let role = egui::accesskit::Role::SpinButton;
        harness
            .get_by_role_and_label(role, "Exposure")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_role_and_label(role, "Exposure").click();
        harness.step();
        harness.event(egui::Event::Text("1.75".into()));
        harness.step();
        assert!(harness.ctx.text_edit_focused());
        harness.read(|editor| {
            assert!(
                editor.application_menu_state().enabled(Action::SelectAll),
                "a focused field keeps the native Edit commands available"
            )
        });
        native_action(&mut harness, Action::SelectAll);
        assert!(
            harness.ctx.text_edit_focused(),
            "a native Edit command does not surrender text focus"
        );
        harness.event(egui::Event::Text("2.25".into()));
        harness.key_press(egui::Key::Enter);
        harness.try_run_realtime().unwrap();
        harness.read(|editor| {
            assert_eq!(
                editor.document.visit.adjustment_layer_settings.exposure,
                2.25
            )
        });

        harness.get_by_role_and_label(role, "Exposure").click();
        harness.step();
        assert!(harness.ctx.text_edit_focused());
        harness.event(egui::Event::Text("3".into()));
        harness.step();
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.read(|editor| {
            assert_eq!(
                editor.document.visit.adjustment_layer_settings.exposure, 2.25,
                "Escape discards the draft"
            )
        });
    });
}

#[test]
fn native_file_menu_keeps_unavailable_commands_discoverable() {
    editor_harness(|mut harness, _| {
        harness.try_run_realtime().unwrap();
        let state = harness.read(|editor| editor.application_menu_state());
        assert!(state.enabled(Action::AddScans), "Add Scans stays reachable");
        assert!(
            !state.enabled(Action::Export),
            "Export stays listed but unavailable without photos"
        );
        assert_eq!(state.label(Action::AddScans), "Add Scans…");
        assert_eq!(state.label(Action::Export), "Export…");
    });
}
