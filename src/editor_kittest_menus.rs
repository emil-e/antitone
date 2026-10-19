use super::*;
use crate::platform::menu::Action;

fn native_action(harness: &mut Harness<'_, Editor>, action: Action) {
    host::App::menu_action(harness.state_mut(), action);
    harness.step();
    harness.try_run_realtime().unwrap();
}

#[test]
fn settings_close_commits_its_number_and_native_select_all_keeps_text_focus() {
    let (mut harness, _) = editor_harness();
    harness.try_run_realtime().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("preferences.json");
    harness.state_mut().preference_store = Some(preferences::Store::open(path.clone()).unwrap().0);
    let project = harness.state().document.project.clone();
    host::App::native_menu_installed(harness.state_mut());
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
    assert!(harness.state().menus.panel.is_none());
    assert!(!harness.state().allow_close);
    assert_eq!(
        harness.state().thumbnails.limits.cpu_bytes,
        37 * cache_limits::MIB
    );
    assert_eq!(
        preferences::Store::open(path).unwrap().1.cache_limits,
        Some(harness.state().thumbnails.limits)
    );
    assert_eq!(harness.state().document.project, project);
}

#[test]
fn native_paste_reaches_the_field_before_enter_commits_it() {
    let (mut harness, _) = editor_harness();
    harness.try_run_realtime().unwrap();
    native_action(&mut harness, Action::Settings);
    harness
        .get_by_role_and_label(egui::accesskit::Role::SpinButton, "CPU cache")
        .click();
    harness.step();
    for action in [Action::SelectAll, Action::Paste] {
        host::App::menu_action(harness.state_mut(), action);
    }
    let input = host::App::menu_input(harness.state_mut(), Some("37".into()));
    for event in input {
        harness.event(event);
    }
    harness.key_press(egui::Key::Enter);
    harness.try_run_realtime().unwrap();
    assert_eq!(
        harness.state().thumbnails.limits.cpu_bytes,
        37 * cache_limits::MIB
    );
    assert_eq!(
        harness.state().menus.panel,
        Some(presentation::AppPanel::Settings)
    );
}

#[test]
fn operation_failure_owns_escape_above_settings_without_closing_settings() {
    let (mut harness, _) = editor_harness();
    harness.try_run_realtime().unwrap();
    native_action(&mut harness, Action::Settings);
    harness
        .state_mut()
        .report_failure(FailureKind::Export, "fixture export failure".into());
    harness.try_run_realtime().unwrap();
    assert!(!harness.get_by_label("OK").accesskit_node().is_disabled());
    harness.key_press(egui::Key::Escape);
    harness.try_run_realtime().unwrap();
    assert!(harness.state().failure.is_none());
    assert_eq!(
        harness.state().menus.panel,
        Some(presentation::AppPanel::Settings)
    );
}

#[test]
fn settings_escape_cancels_numeric_text_before_dismissing_the_panel() {
    let (mut harness, _) = editor_harness();
    harness.try_run_realtime().unwrap();
    native_action(&mut harness, Action::Settings);
    let before = harness.state().thumbnails.limits;
    harness
        .get_by_role_and_label(egui::accesskit::Role::SpinButton, "CPU cache")
        .click();
    harness.step();
    harness.event(egui::Event::Text("999".into()));
    harness.step();
    harness.key_press(egui::Key::Escape);
    harness.try_run_realtime().unwrap();
    assert_eq!(
        harness.state().menus.panel,
        Some(presentation::AppPanel::Settings)
    );
    assert_eq!(harness.state().thumbnails.limits, before);
    harness.key_press(egui::Key::Escape);
    harness.try_run_realtime().unwrap();
    assert!(harness.state().menus.panel.is_none());
    assert_eq!(harness.state().thumbnails.limits, before);
}

#[test]
fn application_panels_can_be_closed_at_both_layouts_without_editing_the_document() {
    let (mut harness, _) = editor_harness();
    harness.try_run_realtime().unwrap();
    let before = harness.state().document.project.clone();
    host::App::native_menu_installed(harness.state_mut());
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
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(close.rect()));
            let png = harness.render().unwrap();
            save_capture(&png, &format!("application-{name}-{layout}-offscreen"));
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Close")
                .click();
            harness.try_run_realtime().unwrap();
            assert!(
                harness.state().menus.panel.is_none(),
                "{name} did not close at {layout}"
            );
            assert_eq!(harness.state().document.project, before);
        }
    }
}

#[test]
fn fallback_edit_menu_preserves_the_focused_numeric_control() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
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
    harness.get_by_label("Edit").click();
    harness.step();
    harness.step();
    assert!(
        !harness
            .get_by_label("Select All")
            .accesskit_node()
            .is_disabled()
    );
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(!egui::Popup::is_any_open(&harness.ctx));
    assert!(harness.ctx.text_edit_focused());
    assert_eq!(harness.state().photo.exposure, 0.5);
    harness.get_by_label("Edit").click();
    harness.step();
    harness.step();
    harness.get_by_label("Select All").click();
    harness.step();
    harness.step();
    assert!(harness.ctx.text_edit_focused());
    harness.event(egui::Event::Text("2.25".into()));
    harness.key_press(egui::Key::Enter);
    harness.try_run_realtime().unwrap();
    assert_eq!(harness.state().photo.exposure, 2.25);
}

#[test]
fn fallback_file_menu_keeps_unavailable_commands_discoverable() {
    let (mut harness, _) = editor_harness();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("File").click();
    harness.step();
    harness.step();
    assert!(
        !harness
            .get_by_label("Add Scans…")
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label("Export").click();
    harness.step();
    harness.step();
    assert!(harness.get_by_label("TIFF…").accesskit_node().is_disabled());
}
