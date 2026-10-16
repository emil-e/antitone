use super::*;
use crate::{
    geometry::SourcePoint,
    local_adjustments::{GradientMask, LocalAdjustment},
    project::{AdjustmentId, LocalAdjustments},
};

pub(super) fn locals(editor: &Editor) -> &LocalAdjustments {
    editor
        .document
        .project
        .source(editor.current_source.unwrap())
        .unwrap()
        .local_adjustments
        .as_ref()
        .unwrap()
}

fn close_drawer(harness: &mut Harness<'_, Editor>) {
    if harness.state().adjustments.drawer_open() {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Adjustments")
            .click();
        harness.step();
        harness.try_run_realtime().unwrap();
    }
}

pub(super) fn open_drawer(harness: &mut Harness<'_, Editor>) {
    harness.try_run_realtime().unwrap();
    if !harness.state().adjustments.drawer_open() {
        harness
            .get_by_role_and_label(egui::accesskit::Role::Button, "Adjustments")
            .click();
        harness.step();
        harness.try_run_realtime().unwrap();
    }
}

fn rename_selected(harness: &mut Harness<'_, Editor>) {
    open_drawer(harness);
    let super::super::adjustments::Target::Local(id) = harness.state().adjustment_target() else {
        panic!("select a local before renaming");
    };
    let name = locals(harness.state())
        .adjustments
        .iter()
        .find(|record| record.id == id)
        .unwrap()
        .name
        .clone();
    harness.get_by_label(&name).scroll_to_me();
    harness.try_run_realtime().unwrap();
    // Separate the double-click from the preceding selection click, not a triple-click.
    harness.run_steps(40);
    let point = harness.get_by_label(&name).rect().center();
    harness.hover_at(point);
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

fn delete_selected(harness: &mut Harness<'_, Editor>) {
    harness.key_press(egui::Key::Delete);
    harness.step();
    harness.try_run_realtime().unwrap();
}

fn click(harness: &mut Harness<'_, Editor>, name: &str) {
    if [
        "Sky",
        "Foreground",
        "Mask overlay",
        "Add radial",
        "Add linear",
    ]
    .contains(&name)
    {
        open_drawer(harness);
    }
    harness.try_run_realtime().unwrap();
    harness
        .get_all_by_label(name)
        .next()
        .unwrap()
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_all_by_label(name)
        .find(|node| !node.accesskit_node().is_disabled())
        .unwrap_or_else(|| panic!("no enabled {name}"))
        .click();
    harness.step();
    harness.try_run_realtime().unwrap();
}

fn pending_exposure(harness: &mut Harness<'_, Editor>, text: &str) {
    let role = egui::accesskit::Role::SpinButton;
    harness
        .get_by_role_and_label(role, "Exposure")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_role_and_label(role, "Exposure").click();
    harness.step();
    harness.event(egui::Event::Text(text.into()));
    harness.step();
    assert!(harness.ctx.text_edit_focused());
}

pub(super) fn install_locals(editor: &mut Editor) -> [AdjustmentId; 2] {
    let area = editor.ready.as_ref().unwrap().area;
    let adjustment = LocalAdjustment {
        mask: GradientMask::Linear {
            start: SourcePoint([area.x as f64, area.y as f64]),
            end: SourcePoint([(area.x + area.width) as f64, (area.y + area.height) as f64]),
        },
        photo: PhotoSettings::default(),
        enabled: true,
        invert: false,
        strength: 1.,
    };
    let mut records = LocalAdjustments::default();
    let ids = [
        records.add("Sky".into(), adjustment).unwrap(),
        records
            .add(
                "Foreground".into(),
                LocalAdjustment {
                    mask: GradientMask::Radial {
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
            .unwrap(),
    ];
    let id = editor.current_source.unwrap();
    editor
        .document
        .project
        .source_mut(id)
        .unwrap()
        .local_adjustments = Some(records);
    editor
        .validate_source_candidate(editor.document.project.source(id).unwrap())
        .unwrap();
    ids
}

#[test]
fn local_target_widgets_commit_outgoing_numbers_and_keep_global_and_section_resets_separate() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    let ids = install_locals(harness.state_mut());
    harness
        .state_mut()
        .preferences
        .sections
        .insert("photo-adjustments".into(), false);
    harness.step();
    let global = harness.state().photo;
    open_drawer(&mut harness);
    for (size, width) in [
        (egui::vec2(1100., 800.), 315.),
        (egui::vec2(800., 600.), 270.),
    ] {
        set_layout(&mut harness, size, width);
        open_drawer(&mut harness);
        assert_eq!(harness.query_all_by_label("Global").count(), 0);
        assert_eq!(harness.query_all_by_label("Editing: Global").count(), 0);
        assert!(
            (harness.get_by_label("Sky").rect().width()
                - harness.get_by_label("Foreground").rect().width())
            .abs()
                < 1.
        );
        click(&mut harness, "Sky");
        assert_eq!(
            harness.get_by_label("Sky").accesskit_node().is_selected(),
            Some(true)
        );
        assert_eq!(
            harness.state().adjustment_target(),
            super::super::adjustments::Target::Local(ids[0])
        );
        assert!(
            harness
                .get_by_label("Pick neutral…")
                .accesskit_node()
                .is_disabled(),
            "local controls must not invoke Global neutral fitting"
        );
        pending_exposure(&mut harness, "1.125");
        harness.try_run_realtime().unwrap();
        harness.get_by_label("Editing: Sky");
        let before = locals(harness.state()).adjustments[0]
            .adjustment
            .photo
            .exposure;
        assert_ne!(
            before, 1.125,
            "pending text must not change accepted processing"
        );
        click(&mut harness, "Foreground");
        assert_eq!(
            locals(harness.state()).adjustments[0]
                .adjustment
                .photo
                .exposure,
            1.125
        );
        assert_eq!(
            locals(harness.state()).adjustments[1]
                .adjustment
                .photo
                .exposure,
            0.
        );
        workflow::enter_numeric(&mut harness, "Exposure", "-0.75");
        assert_eq!(harness.state().photo, global);
        assert_eq!(
            locals(harness.state()).adjustments[1]
                .adjustment
                .photo
                .exposure,
            -0.75
        );
        click(&mut harness, "Reset Exposure");
        assert_eq!(
            locals(harness.state()).adjustments[1]
                .adjustment
                .photo
                .exposure,
            0.
        );
        assert_eq!(
            locals(harness.state()).adjustments[0]
                .adjustment
                .photo
                .exposure,
            1.125
        );
        click(&mut harness, "Enable Foreground");
        assert!(!locals(harness.state()).adjustments[1].adjustment.enabled);
        workflow::enter_numeric(&mut harness, "Exposure", "0.25");
        assert_eq!(
            locals(harness.state()).adjustments[1]
                .adjustment
                .photo
                .exposure,
            0.25
        );
        assert!(!locals(harness.state()).adjustments[1].adjustment.enabled);
        assert!(
            !harness
                .get_by_label("Disabled · changes have no effect")
                .accesskit_node()
                .is_disabled()
        );
        workflow::enter_numeric(&mut harness, "Strength", "0.375");
        assert_eq!(
            locals(harness.state()).adjustments[1].adjustment.strength,
            0.375
        );
        assert_eq!(
            locals(harness.state()).adjustments[0].adjustment.strength,
            1.
        );
        click(&mut harness, "Invert mask");
        assert!(locals(harness.state()).adjustments[1].adjustment.invert);
        click(&mut harness, "Invert mask");
        click(&mut harness, "Reset Strength");
        assert_eq!(
            locals(harness.state()).adjustments[1].adjustment.strength,
            1.
        );
        let scopes_visible = harness.state().scopes.visible;
        if scopes_visible {
            click(&mut harness, "Scopes");
        }
        harness
            .get_by_role_and_label(egui::accesskit::Role::Label, "Adjustments")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        workflow::wait_settled(&mut harness);
        let png = harness.render().unwrap();
        save_capture(
            &png,
            if width == 315. {
                "local-target-normal-offscreen"
            } else {
                "local-target-narrow-offscreen"
            },
        );
        if scopes_visible {
            click(&mut harness, "Scopes");
        }
        click(&mut harness, "Enable Foreground");
        click(&mut harness, "Reset Exposure");
        click(&mut harness, "Sky");
        click(&mut harness, "Reset Exposure");
        close_drawer(&mut harness);
        assert_eq!(harness.state().photo, global);
        assert_eq!(
            harness.state().adjustment_target(),
            super::super::adjustments::Target::Global
        );
        workflow::wait_settled(&mut harness);
        harness.render().unwrap();
    }
}

#[test]
fn drawer_close_commits_outgoing_local_then_routes_global_and_reopens_the_same_local() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    let ids = install_locals(harness.state_mut());
    harness.step();
    assert_eq!(harness.query_all_by_label("Add linear").count(), 0);
    let before = harness.state().document.project.clone();
    open_drawer(&mut harness);
    assert_eq!(harness.state().document.project, before);
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Local(ids[0])
    );
    assert_eq!(harness.query_all_by_label("Global").count(), 0);
    assert_eq!(harness.query_all_by_label("Editing: Global").count(), 0);
    assert_eq!(harness.query_all_by_label("More…").count(), 0);
    assert_eq!(harness.query_all_by_label("Mask").count(), 0);
    click(&mut harness, "Foreground");
    for name in ["Add linear", "Add radial"] {
        let icon = harness
            .get_by_role_and_label(egui::accesskit::Role::Button, name)
            .rect();
        assert!(icon.width() <= presentation::CONTROL_HEIGHT + 1.);
        assert!(icon.bottom() < harness.get_by_label("Sky").rect().top());
    }
    harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Strength");
    harness.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Feather");
    pending_exposure(&mut harness, "0.125");
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Adjustments")
        .click();
    harness.step();
    harness.try_run_realtime().unwrap();
    assert!(!harness.state().adjustments.drawer_open());
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Global
    );
    assert!(!harness.state().mask_handles_available());
    assert!(harness.state().mask_edit.is_none());
    assert_eq!(
        locals(harness.state()).adjustments[1]
            .adjustment
            .photo
            .exposure,
        0.125
    );
    assert_eq!(harness.query_all_by_label("Add radial").count(), 0);
    assert_eq!(harness.query_all_by_label("Editing: Foreground").count(), 0);
    assert_eq!(harness.query_all_by_label("Editing: Global").count(), 0);
    save_capture(&harness.render().unwrap(), "adjustments-closed-with-locals");
    workflow::enter_numeric(&mut harness, "Exposure", "0.625");
    assert_eq!(harness.state().photo.exposure, 0.625);
    let accepted = harness.state().document.project.clone();
    open_drawer(&mut harness);
    assert_eq!(harness.state().document.project, accepted);
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Local(ids[1])
    );
    assert_eq!(
        locals(harness.state()).adjustments[1]
            .adjustment
            .photo
            .exposure,
        0.125
    );
}

#[test]
fn empty_open_tool_keeps_grading_visible_but_read_only_and_close_reenables_it() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        set_layout(&mut harness, size, width);
        let before = harness.state().document.project.clone();
        open_drawer(&mut harness);
        assert_eq!(harness.query_all_by_label("Global").count(), 0);
        assert_eq!(harness.query_all_by_label("Editing: Global").count(), 0);
        for label in ["Exposure", "Warmth", "Contrast", "Saturation"] {
            assert!(
                harness
                    .get_by_role_and_label(egui::accesskit::Role::SpinButton, label)
                    .accesskit_node()
                    .is_disabled(),
                "empty-drawer {label} must remain visible but read-only"
            );
        }
        harness.get_by_label("Add a gradient to start a local adjustment.");
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        let disabled = harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .rect()
            .center();
        masks::pointer(&mut harness, disabled, true);
        masks::pointer(&mut harness, disabled, false);
        harness.event(egui::Event::Text("1.25".into()));
        harness.key_press(egui::Key::Enter);
        harness.step();
        assert!(!harness.ctx.text_edit_focused());
        assert_eq!(harness.state().document.project, before);
        save_capture(
            &harness.render().unwrap(),
            &format!("adjustments-empty-{name}"),
        );
        click(&mut harness, "Add radial");
        assert!(harness.state().mask_edit.is_some());
        close_drawer(&mut harness);
        assert!(harness.state().mask_edit.is_none());
        assert_eq!(
            harness.state().adjustment_target(),
            super::super::adjustments::Target::Global
        );
        assert_eq!(harness.state().document.project, before);
        assert!(!harness.state().mask_handles_available());
        assert!(
            !harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_disabled()
        );
        assert_eq!(harness.query_all_by_label("Editing: Global").count(), 0);
        save_capture(
            &harness.render().unwrap(),
            &format!("adjustments-closed-{name}"),
        );
    }
}

#[test]
fn row_drag_escape_and_outside_release_never_change_order_or_canvas_geometry() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    install_locals(harness.state_mut());
    harness.step();
    if harness.state().scopes.visible {
        click(&mut harness, "Scopes");
    }
    for (size, width) in [
        (egui::vec2(1100., 800.), 315.),
        (egui::vec2(800., 600.), 270.),
    ] {
        set_layout(&mut harness, size, width);
        click(&mut harness, "Sky");
        harness.get_by_label("Foreground").scroll_to_me();
        harness.try_run_realtime().unwrap();
        let before = harness.state().document.project.clone();
        let start = harness.get_by_label("Sky").rect().center();
        let end = harness.get_by_label("Foreground").rect().center() + egui::vec2(0., 8.);
        masks::hold_drag(&mut harness, start, end);
        assert!(harness.state().adjustments.dragging());
        harness.key_press(egui::Key::Delete);
        harness.step();
        assert!(!harness.state().adjustments.confirming());
        assert!(harness.state().removal_pending.is_none());
        harness.key_press(egui::Key::Escape);
        harness.step();
        masks::pointer(&mut harness, end, false);
        assert!(!harness.state().adjustments.dragging());
        assert_eq!(harness.state().document.project, before);
        let start = harness.get_by_label("Sky").rect().center();
        let outside = harness.state().last_layout.unwrap().0.image.center();
        drag(&mut harness, start, outside);
        assert!(!harness.state().adjustments.dragging());
        assert!(harness.state().mask_edit.is_none());
        assert_eq!(harness.state().document.project, before);
    }
}

#[test]
fn delete_only_targets_active_locals_and_respects_text_popup_fullscreen_and_dust() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    install_locals(harness.state_mut());
    harness.step();
    open_drawer(&mut harness);
    let before = harness.state().document.project.clone();
    delete_selected(&mut harness);
    assert!(harness.state().adjustments.confirming());
    assert!(harness.state().removal_pending.is_none());
    click(&mut harness, "Cancel");
    click(&mut harness, "Sky");
    pending_exposure(&mut harness, "0.125");
    harness.key_press(egui::Key::Delete);
    harness.step();
    assert!(!harness.state().adjustments.confirming() && harness.state().removal_pending.is_none());
    assert_eq!(harness.state().document.project, before);
    assert!(harness.ctx.text_edit_focused());
    assert!(!egui::DragAndDrop::has_any_payload(&harness.ctx));
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert_eq!(
        locals(harness.state()).adjustments[0]
            .adjustment
            .photo
            .exposure,
        0.,
        "numeric Escape must not accept pending exposure"
    );
    click(&mut harness, "Viewer help");
    delete_selected(&mut harness);
    assert!(!harness.state().adjustments.confirming());
    harness.key_press(egui::Key::Escape);
    harness.step();
    let context = harness.ctx.clone();
    harness.state_mut().toggle_fullscreen(&context);
    harness.step();
    delete_selected(&mut harness);
    assert!(!harness.state().adjustments.confirming());
    harness.state_mut().toggle_fullscreen(&context);
    host::App::photo_fullscreen_changed(harness.state_mut(), false);
    harness.step();
    click(&mut harness, "Dust repair");
    assert!(!harness.state().adjustments.drawer_open());
    delete_selected(&mut harness);
    assert!(!harness.state().adjustments.confirming() && harness.state().removal_pending.is_none());
    click(&mut harness, "Done");
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Global
    );
    assert!(!harness.state().adjustment_owns_delete());
    open_drawer(&mut harness);
    delete_selected(&mut harness);
    assert!(harness.state().adjustments.confirming());
    click(&mut harness, "Cancel");
    assert_eq!(harness.state().document.project, before);
}

#[test]
fn inline_rename_commits_before_selecting_another_row() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    let ids = install_locals(harness.state_mut());
    harness.step();
    click(&mut harness, "Sky");
    rename_selected(&mut harness);
    harness.event(egui::Event::Text(" renamed".into()));
    harness.step();
    click(&mut harness, "Foreground");
    assert!(
        locals(harness.state()).adjustments[0]
            .name
            .contains("renamed")
    );
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Local(ids[1])
    );
}

#[test]
fn target_indicator_only_pins_when_the_selected_row_scrolls_away() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    install_locals(harness.state_mut());
    harness.step();
    if harness.state().scopes.visible {
        click(&mut harness, "Scopes");
    }
    for (name, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        set_layout(&mut harness, size, width);
        click(&mut harness, "Sky");
        assert_eq!(harness.query_all_by_label("Editing: Sky").count(), 0);
        save_capture(
            &harness.render().unwrap(),
            &format!("local-target-{name}-offscreen"),
        );
        click(&mut harness, "Foreground");
        save_capture(
            &harness.render().unwrap(),
            &format!("adjustment-drawer-radial-{name}"),
        );
        click(&mut harness, "Sky");
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Density")
            .scroll_to_me();
        harness.try_run_realtime().unwrap();
        harness.get_by_label("Editing: Sky");
    }
}

#[test]
fn creation_launcher_commits_outgoing_local_numbers_before_arming_without_consuming_an_id() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    install_locals(harness.state_mut());
    harness.step();
    click(&mut harness, "Sky");
    pending_exposure(&mut harness, "0.125");
    click(&mut harness, "Add radial");
    assert_eq!(
        locals(harness.state()).adjustments[0]
            .adjustment
            .photo
            .exposure,
        0.125
    );
    assert_eq!(locals(harness.state()).next_id, 3);
    assert!(harness.state().mask_edit.is_some());
    let before = harness.state().document.project.clone();
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert_eq!(harness.state().document.project, before);
    assert!(harness.state().mask_edit.is_none());
}

#[test]
fn local_list_widgets_preserve_ids_processing_metadata_and_confirm_deletion() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    let ids = install_locals(harness.state_mut());
    harness.step();
    click(&mut harness, "Sky");
    let id = harness.state().current_source.unwrap();
    let accepted = harness
        .state()
        .positive_processing(
            harness.state().calibration().unwrap(),
            harness.state().roll,
            harness.state().photo,
        )
        .unwrap();
    rename_selected(&mut harness);
    let entry = harness.get_by_role(egui::accesskit::Role::TextInput);
    entry.focus();
    harness.step();
    harness.event(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.event(egui::Event::Text("Upper sky".into()));
    harness.step();
    harness.key_press(egui::Key::Enter);
    harness.step();
    harness.try_run_realtime().unwrap();
    assert_eq!(locals(harness.state()).adjustments[0].name, "Upper sky");
    rename_selected(&mut harness);
    harness.event(egui::Event::Text(" discarded".into()));
    harness.step();
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert_eq!(locals(harness.state()).adjustments[0].name, "Upper sky");
    let renamed = harness
        .state()
        .positive_processing(
            harness.state().calibration().unwrap(),
            harness.state().roll,
            harness.state().photo,
        )
        .unwrap();
    assert!(std::ptr::eq(
        accepted.grade().unwrap(),
        renamed.grade().unwrap()
    ));
    if harness.state().scopes.visible {
        click(&mut harness, "Scopes");
    }
    harness.get_by_label("Foreground").scroll_to_me();
    harness.try_run_realtime().unwrap();
    let start = harness.get_by_label("Upper sky").rect().center();
    let target = harness.get_by_label("Foreground").rect();
    drag(
        &mut harness,
        start,
        egui::pos2(target.center().x, target.bottom() - 3.),
    );
    harness.try_run_realtime().unwrap();
    assert_eq!(
        locals(harness.state())
            .adjustments
            .iter()
            .map(|r| r.id)
            .collect::<Vec<_>>(),
        vec![ids[1], ids[0]]
    );
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Local(ids[0])
    );
    let before = harness.state().document.project.clone();
    delete_selected(&mut harness);
    assert!(harness.state().adjustments.confirming());
    assert!(!harness.state().roll_edit_available());
    assert!(!harness.state().fullscreen_available());
    harness.get_by_label("Cancel").click();
    harness.step();
    assert_eq!(harness.state().document.project, before);
    delete_selected(&mut harness);
    harness
        .state_mut()
        .document
        .project
        .source_mut(id)
        .unwrap()
        .local_adjustments
        .as_mut()
        .unwrap()
        .adjustments[1]
        .name = "Changed after confirmation".into();
    let changed = harness.state().document.project.clone();
    harness.get_by_label("Delete adjustment").click();
    harness.step();
    assert_eq!(
        harness.state().document.project,
        changed,
        "a stale decision cannot delete a changed record"
    );
    assert!(!harness.state().adjustments.confirming());
    delete_selected(&mut harness);
    harness.get_by_label("Delete adjustment").click();
    harness.step();
    assert_eq!(locals(harness.state()).adjustments.len(), 1);
    assert_eq!(locals(harness.state()).adjustments[0].id, ids[1]);
    assert_eq!(locals(harness.state()).next_id, 3);
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Local(ids[1])
    );
    assert_eq!(harness.state().current_source, Some(id));
    delete_selected(&mut harness);
    harness.get_by_label("Delete adjustment").click();
    harness.step();
    harness.try_run_realtime().unwrap();
    assert!(locals(harness.state()).adjustments.is_empty());
    assert_eq!(locals(harness.state()).next_id, 3);
    assert!(harness.state().adjustments.drawer_open());
    assert!(
        harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .accesskit_node()
            .is_disabled()
    );
    close_drawer(&mut harness);
    assert!(
        !harness
            .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
            .accesskit_node()
            .is_disabled()
    );
    harness
        .state()
        .validate_document_candidate(&harness.state().document)
        .unwrap();
}

#[test]
fn changing_sources_cannot_commit_pending_local_input_into_another_photos_global_grade() {
    let (mut harness, _, _directory) = workflow::calibrated_workflow();
    install_locals(harness.state_mut());
    harness.step();
    let original = harness.state().current_source.unwrap();
    click(&mut harness, "Sky");
    pending_exposure(&mut harness, "1.875");
    click(&mut harness, "patches.dng");
    wait_for(&mut harness, |editor| !editor.busy);
    assert_ne!(harness.state().current_source, Some(original));
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::adjustments::Target::Global
    );
    assert_eq!(harness.state().photo, PhotoSettings::default());
    let accepted = harness.state().document.project.source(original).unwrap();
    assert_eq!(accepted.photo.exposure, 0.5);
    assert_eq!(
        accepted.local_adjustments.as_ref().unwrap().adjustments[0]
            .adjustment
            .photo
            .exposure,
        0.,
        "source replacement cancels the hidden local text rather than applying it to another target"
    );
    workflow::enter_numeric(&mut harness, "Exposure", "-0.125");
    assert_eq!(harness.state().photo.exposure, -0.125);
    let accepted = harness.state().document.project.source(original).unwrap();
    assert_eq!(accepted.photo.exposure, 0.5);
    assert_eq!(
        accepted.local_adjustments.as_ref().unwrap().adjustments[0]
            .adjustment
            .photo
            .exposure,
        0.
    );
}
