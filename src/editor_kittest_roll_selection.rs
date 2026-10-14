use super::*;

fn request_removal(harness: &mut Harness<'_, Editor>, photo: &str) {
    harness.get_by_label(photo).click_secondary();
    harness.step();
    harness.step();
    harness.get_by_label("Remove selected photos…").click();
    harness.step();
}

#[test]
fn roll_selection_removal_empty_reopen_and_reimport_preserve_accepted_state() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..5)
        .map(|index| {
            let path = directory.path().join(format!("photo-{index}.dng"));
            std::fs::copy(fixture("illuminated-patches.dng"), &path).unwrap();
            path
        })
        .collect();
    let originals: Vec<_> = paths
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect();
    let (mut harness, _) = editor_harness_with_thumbnails(true);
    harness.state_mut().add_raws(paths.clone());
    wait_for(&mut harness, |e| !e.busy);
    harness
        .state_mut()
        .begin_flat_edit(fixture("bare-light.dng"), None);
    wait_for(&mut harness, |e| !e.busy);
    harness.get_by_label("Apply").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    assert!(
        harness.state().correction.is_some(),
        "{}",
        harness.state().status
    );
    let ids = harness.state().document.project.order.clone();
    let project_path = directory.path().join("roll.antitone.json");
    {
        let e = harness.state_mut();
        e.dmin = Some(DVec3::new(0.2, 0.15, 0.1));
        e.dmax = Some(DVec3::new(0.008, 0.005, 0.002));
        e.document
            .project
            .source_mut(ids[0])
            .unwrap()
            .photo
            .exposure = 0.123456789012345;
        e.photo.exposure = 0.123456789012345;
        e.sync_document();
        e.document = e
            .document
            .save(&project_path, crate::project::Save::New)
            .unwrap();
    }
    let before = harness.state().document.project.clone();
    let saved_bytes = std::fs::read(&project_path).unwrap();
    harness.get_by_label("photo-2.dng").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    harness
        .get_by_label("photo-0.dng")
        .click_modifiers(egui::Modifiers::COMMAND);
    harness.step();
    assert_eq!(harness.state().current_source, Some(ids[2]));
    for (name, active) in [("photo-0.dng", ids[0]), ("photo-2.dng", ids[2])] {
        harness.get_by_label(name).click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        assert_eq!(harness.state().current_source, Some(active));
        for selected in ["photo-0.dng", "photo-2.dng"] {
            assert_eq!(
                harness.get_by_label(selected).accesskit_node().toggled(),
                Some(egui::accesskit::Toggled::True)
            );
        }
    }
    for focused_card in [true, false] {
        if focused_card {
            harness.get_by_label("photo-0.dng").focus();
            harness.step();
        } else {
            harness.ctx.memory_mut(|memory| {
                if let Some(id) = memory.focused() {
                    memory.surrender_focus(id);
                }
            });
        }
        let request = harness.state().id;
        let viewport = harness.state().viewport;
        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::A,
        );
        harness.step();
        assert_eq!(harness.state().current_source, Some(ids[2]));
        assert_eq!(
            harness.state().id,
            request,
            "shortcut does not dispatch a reload"
        );
        assert_eq!(harness.state().viewport, viewport);
        assert_eq!(
            harness
                .get_by_label("photo-0.dng")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::False)
        );
        assert_eq!(
            harness
                .get_by_label("photo-2.dng")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        harness
            .get_by_label("photo-0.dng")
            .click_modifiers(egui::Modifiers::COMMAND);
        harness.step();
    }
    assert_eq!(harness.state().document.project, before);
    assert!(!harness.state().document.dirty());
    assert_eq!(
        harness
            .get_by_label("photo-0.dng")
            .accesskit_node()
            .toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    assert_eq!(
        harness
            .get_by_label("photo-2.dng")
            .accesskit_node()
            .description()
            .as_deref(),
        Some("Active photo")
    );
    save_capture(&harness.render().unwrap(), "roll-multi-selected-offscreen");
    harness.set_size(egui::vec2(640., 560.));
    harness.step();
    harness.step();
    save_capture(
        &harness.render().unwrap(),
        "roll-multi-selected-narrow-offscreen",
    );
    harness.set_size(egui::vec2(1100., 800.));
    harness.step();
    harness.step();
    request_removal(&mut harness, "photo-2.dng");
    assert!(harness.state().removal_pending.is_some());
    assert_eq!(harness.state().document.project, before);
    harness.step();
    save_capture(&harness.render().unwrap(), "roll-remove-confirm-offscreen");
    harness.key_press(egui::Key::Escape);
    harness.step();
    assert!(harness.state().removal_pending.is_none());
    assert!(!harness.state().document.dirty());
    request_removal(&mut harness, "photo-2.dng");
    harness.get_by_label("Remove photos").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    let mut expected = before.clone();
    expected.order = vec![ids[1], ids[3], ids[4]];
    assert_eq!(harness.state().document.project, expected);
    assert_eq!(harness.state().current_source, Some(ids[4]));
    assert_eq!(harness.state().selected, 2);
    assert!(harness.state().document.dirty());
    assert_eq!(std::fs::read(&project_path).unwrap(), saved_bytes);
    save_existing_project(&mut harness);
    harness.get_by_label("photo-1.dng").click();
    harness.step();
    wait_for(&mut harness, |e| !e.busy);
    harness
        .get_by_label("photo-4.dng")
        .click_modifiers(egui::Modifiers::SHIFT);
    harness.step();
    assert_eq!(harness.state().current_source, Some(ids[1]));
    for name in ["photo-1.dng", "photo-3.dng", "photo-4.dng"] {
        assert_eq!(
            harness.get_by_label(name).accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
    }
    harness
        .state_mut()
        .thumbnails
        .set_limits(cache_limits::CacheLimits {
            cpu_bytes: 0,
            gpu_bytes: 0,
        });
    assert!(harness.state().document.cache().protected_bytes() > 0);
    request_removal(&mut harness, "photo-1.dng");
    harness.get_by_label("Remove photos").click();
    harness.step();
    expected.order.clear();
    assert_eq!(harness.state().document.project, expected);
    assert!(harness.state().ready.is_none() && harness.state().current_source.is_none());
    assert!(harness.state().correction.is_some());
    wait_for(&mut harness, |e| {
        let charges = e.document.cache().charges();
        charges.retained == 0 && charges.leased_only == 0
    });
    assert_eq!(harness.state().document.cache().protected_bytes(), 0);
    save_capture(&harness.render().unwrap(), "roll-empty-offscreen");
    save_existing_project(&mut harness);
    file_menu(&mut harness, "New");
    wait_for(&mut harness, |e| !e.busy);
    open_project(&mut harness, project_path);
    wait_for(&mut harness, |e| !e.busy);
    assert_eq!(harness.state().document.project, expected);
    assert!(harness.state().correction.is_some());
    assert!(harness.state().ready.is_none() && harness.state().current_source.is_none());
    harness.state_mut().add_raws(vec![paths[0].clone()]);
    wait_for(&mut harness, |e| !e.busy);
    expected.order = vec![ids[0]];
    assert_eq!(harness.state().document.project, expected);
    assert_eq!(harness.state().photo.exposure, 0.123456789012345);
    assert_eq!(harness.state().current_source, Some(ids[0]));
    assert!(harness.state().ready.is_some());
    for (path, bytes) in paths.iter().zip(originals) {
        assert_eq!(
            std::fs::read(path).unwrap(),
            bytes,
            "RAW files are untouched"
        );
    }
}

#[test]
fn removal_keyboard_and_context_menu_respect_text_popups_and_drafts() {
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other.dng");
    std::fs::copy(fixture("patches.dng"), &other).unwrap();
    let (mut harness, _) = editor_harness();
    harness
        .state_mut()
        .add_raws(vec![fixture("patches.dng"), other]);
    wait_for(&mut harness, |e| !e.busy);
    harness
        .get_by_label("other.dng")
        .click_modifiers(egui::Modifiers::COMMAND);
    harness.step();
    {
        let e = harness.state_mut();
        e.dmin = Some(DVec3::new(0.2, 0.15, 0.1));
        e.dmax = Some(DVec3::new(0.008, 0.005, 0.002));
    }
    stage(&mut harness, "Photo");
    let role = egui::accesskit::Role::SpinButton;
    harness
        .get_by_role_and_label(role, "Exposure")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_role_and_label(role, "Exposure").click();
    harness.step();
    assert!(harness.ctx.text_edit_focused());
    harness.key_press(egui::Key::Backspace);
    harness.step();
    assert!(harness.state().removal_pending.is_none());
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::A,
    );
    harness.step();
    assert_eq!(
        harness.get_by_label("other.dng").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True),
        "text editing owns the shortcut"
    );
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.get_by_label("patches.dng").focus();
    harness.step();
    assert!(harness.get_by_label("patches.dng").is_focused());
    let before = harness.state().document.project.clone();
    harness.key_press(egui::Key::Backspace);
    harness.step();
    assert!(harness.state().removal_pending.is_some());
    let pending = harness.state().removal_pending.clone();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::A,
    );
    harness.step();
    assert_eq!(harness.state().removal_pending, pending);
    assert_eq!(
        harness.get_by_label("other.dng").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True)
    );
    harness.key_press(egui::Key::Enter);
    harness.step();
    assert!(
        harness.state().removal_pending.is_none(),
        "Cancel is the safe initial action"
    );
    assert_eq!(harness.state().document.project, before);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "File")
        .click();
    harness.step();
    harness.step();
    assert!(
        harness.query_by_label("Remove selected photos…").is_none(),
        "removal is not a File-menu action"
    );
    harness.key_press(egui::Key::Delete);
    harness.step();
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::A,
    );
    harness.step();
    assert!(harness.state().removal_pending.is_none());
    assert_eq!(
        harness.get_by_label("other.dng").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True),
        "popup owns the shortcut"
    );
    harness.key_press(egui::Key::Escape);
    harness.step();
    harness.get_by_label("patches.dng").click_secondary();
    harness.step();
    harness.step();
    assert!(
        !harness
            .get_by_label("Remove selected photos…")
            .accesskit_node()
            .is_disabled()
    );
    save_capture(&harness.render().unwrap(), "roll-remove-context-offscreen");
    harness.get_by_label("Remove selected photos…").click();
    harness.step();
    assert!(harness.state().removal_pending.is_some());
    harness.get_by_label("Cancel").click();
    harness.step();
    harness.state_mut().begin_crop();
    harness.step();
    harness.key_press(egui::Key::Delete);
    harness.step();
    assert!(harness.state().crop_edit.is_some());
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        egui::Key::A,
    );
    harness.step();
    assert_eq!(
        harness.get_by_label("other.dng").accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::True),
        "crop draft retains selection"
    );
    assert!(harness.state().removal_pending.is_none());
    assert_eq!(harness.state().document.project, before);
    harness.get_by_label("Cancel").click();
    harness.step();
    harness.set_size(egui::vec2(640., 560.));
    harness.step();
    request_removal(&mut harness, "patches.dng");
    harness.step();
    save_capture(&harness.render().unwrap(), "roll-remove-narrow-offscreen");
    harness.get_by_label("Cancel").click();
    harness.step();
    assert_eq!(harness.state().document.project, before);
}
