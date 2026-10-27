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
    editor_harness_with_thumbnails(true, |mut harness, _| {
        harness.enter(|editor, cx| editor.add_raws(paths.clone(), cx));
        wait_for(&mut harness, |e| !e.busy);
        harness.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        harness.get_by_label("Apply").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| assert!(editor.correction.is_some(), "{}", editor.status));
        let ids = harness.read(|editor| editor.document.project.order.clone());
        let project_path = directory.path().join("roll.antitone.json");
        harness.update(|e| {
            e.set_dmin(Some(DVec3::new(0.2, 0.15, 0.1)));
            e.set_dmax(Some(DVec3::new(0.008, 0.005, 0.002)));
            e.document
                .project
                .source_mut(ids[0])
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap()
                .layers[0]
                .adjustment
                .photo
                .exposure = 0.123456789012345;
            e.adjustment_layer_settings.exposure = 0.123456789012345;
            e.sync_document();
            *e.document = e
                .document
                .save(&project_path, crate::project::Save::New)
                .unwrap();
        });
        let before = harness.read(|editor| editor.document.project.clone());
        let saved_bytes = std::fs::read(&project_path).unwrap();
        harness.get_by_label("photo-2.dng").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness
            .get_by_label("photo-0.dng")
            .click_modifiers(egui::Modifiers::COMMAND);
        harness.step();
        harness.read(|editor| assert_eq!(editor.current_source, Some(ids[2])));
        for (name, active) in [("photo-0.dng", ids[0]), ("photo-2.dng", ids[2])] {
            harness.get_by_label(name).click();
            harness.step();
            wait_for(&mut harness, |e| !e.busy);
            harness.read(|editor| assert_eq!(editor.current_source, Some(active)));
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
            let request = harness.read(|editor| editor.id);
            let viewport = harness.read(|editor| editor.viewport);
            harness.key_press_modifiers(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::A,
            );
            harness.step();
            harness.read(|editor| assert_eq!(editor.current_source, Some(ids[2])));
            harness.read(|editor| {
                assert_eq!(editor.id, request, "shortcut does not dispatch a reload")
            });
            harness.read(|editor| assert_eq!(editor.viewport, viewport));
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
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness.read(|editor| assert!(!editor.document.dirty()));
        assert_eq!(
            harness
                .get_by_label("photo-0.dng")
                .accesskit_node()
                .toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        assert!(
            harness
                .get_by_label("photo-2.dng")
                .accesskit_node()
                .description()
                .is_some_and(|description| description.contains("Active photo")),
            "active identity must remain accessible alongside readiness feedback"
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
        harness.read(|editor| assert!(editor.removal_pending.is_some()));
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness.step();
        save_capture(&harness.render().unwrap(), "roll-remove-confirm-offscreen");
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.read(|editor| assert!(editor.removal_pending.is_none()));
        harness.read(|editor| assert!(!editor.document.dirty()));
        request_removal(&mut harness, "photo-2.dng");
        harness.get_by_label("Remove photos").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        let mut expected = before.clone();
        expected.order = vec![ids[1], ids[3], ids[4]];
        expected
            .sources
            .retain(|source| source.id != ids[0] && source.id != ids[2]);
        expected
            .analysis_regions
            .retain(|entry| entry.source != ids[0] && entry.source != ids[2]);
        harness.read(|editor| assert_eq!(editor.document.project, expected));
        harness.read(|editor| assert_eq!(editor.current_source, Some(ids[4])));
        harness.read(|editor| assert_eq!(editor.selected, 2));
        harness.read(|editor| assert!(editor.document.dirty()));
        assert_eq!(std::fs::read(&project_path).unwrap(), saved_bytes);
        save_existing_project(&mut harness);
        harness.get_by_label("photo-1.dng").click();
        harness.step();
        wait_for(&mut harness, |e| !e.busy);
        harness
            .get_by_label("photo-4.dng")
            .click_modifiers(egui::Modifiers::SHIFT);
        harness.step();
        harness.read(|editor| assert_eq!(editor.current_source, Some(ids[1])));
        for name in ["photo-1.dng", "photo-3.dng", "photo-4.dng"] {
            assert_eq!(
                harness.get_by_label(name).accesskit_node().toggled(),
                Some(egui::accesskit::Toggled::True)
            );
        }
        harness.update(|editor| {
            editor.thumbnails.set_limits(cache_limits::CacheLimits {
                cpu_bytes: 0,
                gpu_bytes: 0,
            })
        });
        harness.read(|editor| assert!(editor.document.cache.protected_bytes() > 0));
        request_removal(&mut harness, "photo-1.dng");
        harness.get_by_label("Remove photos").click();
        harness.step();
        expected.order.clear();
        let reference = before.flat_field().unwrap().reference;
        expected.sources.retain(|source| source.id == reference);
        expected.analysis_regions.clear();
        harness.read(|editor| assert_eq!(editor.document.project, expected));
        harness.read(|editor| {
            assert!(editor.document.visit.ready.is_none() && editor.current_source.is_none())
        });
        harness.read(|editor| assert!(editor.correction.is_some()));
        wait_for(&mut harness, |e| {
            let charges = e.document.cache.charges();
            charges.retained == 0 && charges.leased_only == 0
        });
        harness.read(|editor| assert_eq!(editor.document.cache.protected_bytes(), 0));
        save_capture(&harness.render().unwrap(), "roll-empty-offscreen");
        save_existing_project(&mut harness);
        native_file_action(&mut harness, crate::platform::menu::Action::New);
        wait_for(&mut harness, |e| !e.busy);
        open_project(&mut harness, project_path);
        wait_for(&mut harness, |e| !e.busy);
        harness.read(|editor| assert_eq!(editor.document.project, expected));
        harness.read(|editor| assert!(editor.correction.is_some()));
        harness.read(|editor| {
            assert!(editor.document.visit.ready.is_none() && editor.current_source.is_none())
        });
        harness.enter(|editor, cx| editor.add_raws(vec![paths[0].clone()], cx));
        wait_for(&mut harness, |e| !e.busy);
        let new_id = harness.read(|editor| editor.current_source.unwrap());
        assert_ne!(new_id, ids[0]);
        harness.read(|editor| {
            assert_eq!(editor.document.project.order, vec![new_id]);
            let source = editor.document.project.source(new_id).unwrap();
            assert_eq!(source.path, paths[0]);
            assert_eq!(
                source.adjustment_layers.as_ref().unwrap().layers[0]
                    .adjustment
                    .photo
                    .exposure,
                0.
            );
            assert!(source.repairs.repairs().is_empty());
            assert_eq!(editor.document.project.sources.len(), 2);
            assert!(
                editor
                    .document
                    .project
                    .source(reference)
                    .unwrap()
                    .reference_only
            );
            assert_eq!(editor.adjustment_layer_settings.exposure, 0.);
        });
        harness.read(|editor| assert!(editor.document.visit.ready.is_some()));
        for (path, bytes) in paths.iter().zip(originals) {
            assert_eq!(
                std::fs::read(path).unwrap(),
                bytes,
                "RAW files are untouched"
            );
        }
    });
}

#[test]
fn removal_keyboard_and_context_menu_respect_text_popups_and_drafts() {
    let directory = tempfile::tempdir().unwrap();
    let other = directory.path().join("other.dng");
    std::fs::copy(fixture("patches.dng"), &other).unwrap();
    editor_harness(|mut harness, _| {
        harness.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng"), other], cx));
        wait_for(&mut harness, |e| !e.busy);
        harness
            .get_by_label("other.dng")
            .click_modifiers(egui::Modifiers::COMMAND);
        harness.step();
        harness.update(|e| {
            e.set_dmin(Some(DVec3::new(0.2, 0.15, 0.1)));
            e.set_dmax(Some(DVec3::new(0.008, 0.005, 0.002)));
        });
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
        harness.read(|editor| assert!(editor.removal_pending.is_none()));
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
        let before = harness.read(|editor| editor.document.project.clone());
        harness.key_press(egui::Key::Backspace);
        harness.step();
        harness.read(|editor| assert!(editor.removal_pending.is_some()));
        let pending = harness.read(|editor| editor.removal_pending.clone());
        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::A,
        );
        harness.step();
        harness.read(|editor| assert_eq!(editor.removal_pending, pending));
        assert_eq!(
            harness.get_by_label("other.dng").accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        harness.key_press(egui::Key::Enter);
        harness.step();
        harness.read(|editor| {
            assert!(
                editor.removal_pending.is_none(),
                "Cancel is the safe initial action"
            )
        });
        harness.read(|editor| assert_eq!(editor.document.project, before));
        // The menu bar is native, so the menu tables carry what a File menu offers.
        let offers_removal = |entries: &[crate::platform::menu::Entry]| {
            entries.iter().any(|entry| matches!(
                entry,
                crate::platform::menu::Entry::Action(action) if action.label().contains("Remove")
            ))
        };
        assert!(
            [
                crate::platform::menu::MENUS,
                crate::platform::menu::WINDOWS_MENUS
            ]
            .iter()
            .all(|menus| menus.iter().all(|(_, entries)| !offers_removal(entries))),
            "removal is not a File-menu action"
        );
        harness.key_press(egui::Key::Delete);
        harness.step();
        harness.read(|editor| {
            assert!(
                editor.removal_pending.is_some(),
                "Delete reopens removal after a cancelled one"
            )
        });
        let reopened = harness.read(|editor| editor.removal_pending.clone());
        harness.key_press_modifiers(
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
            egui::Key::A,
        );
        harness.step();
        harness.read(|editor| {
            assert_eq!(
                editor.removal_pending, reopened,
                "the dialog owns the shortcut"
            )
        });
        assert_eq!(
            harness.get_by_label("other.dng").accesskit_node().toggled(),
            Some(egui::accesskit::Toggled::True)
        );
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.read(|editor| assert!(editor.removal_pending.is_none(), "Escape cancels"));
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
        harness.read(|editor| assert!(editor.removal_pending.is_some()));
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.update(|editor| editor.begin_crop());
        harness.step();
        harness.key_press(egui::Key::Delete);
        harness.step();
        harness.read(|editor| assert!(editor.document.visit.crop_edit.is_some()));
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
        harness.read(|editor| assert!(editor.removal_pending.is_none()));
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.set_size(egui::vec2(640., 560.));
        harness.step();
        request_removal(&mut harness, "patches.dng");
        harness.step();
        save_capture(&harness.render().unwrap(), "roll-remove-narrow-offscreen");
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert_eq!(editor.document.project, before));
    });
}

#[test]
fn switching_back_uses_source_crop_for_pending_and_ready_views() {
    let directory = tempfile::tempdir().unwrap();
    let paths: Vec<_> = ["roll-0.dng", "roll-1.dng"]
        .iter()
        .map(|name| {
            let path = directory.path().join(name);
            std::fs::copy(fixture(name), &path).unwrap();
            path
        })
        .collect();
    editor_harness_with_thumbnails(true, |mut h, _| {
        h.enter(|editor, cx| editor.add_raws(paths, cx));
        wait_for(&mut h, |e| !e.busy);
        let first = h.read(|e| e.current_source.unwrap());
        let crop = crate::crop::Crop::from_region(crate::raw::Rect {
            x: 88,
            y: 148,
            width: 320,
            height: 200,
        })
        .straightened(
            crate::raw::Rect {
                x: 0,
                y: 0,
                width: 496,
                height: 496,
            },
            0.1,
        )
        .unwrap();
        h.update(|e| {
            e.set_dmin(Some(DVec3::splat(0.8)));
            e.set_dmax(Some(DVec3::splat(0.02)));
            e.crop = Some(crop);
            e.sync_document();
        });
        h.step();
        h.get_by_label("roll-1.dng").click();
        h.step();
        wait_for(&mut h, |e| !e.busy);
        h.get_by_label("roll-0.dng").click();
        h.step();
        h.read(|e| {
            assert_eq!(e.crop, Some(crop));
            assert_eq!(e.document.project.source(first).unwrap().crop, Some(crop));
            assert!(e.pending_preview.is_some());
            assert_eq!(e.pending_preview.as_ref().unwrap().composition.crop(), crop);
        });
        wait_for(&mut h, |e| !e.busy);
        h.read(|e| {
            assert_eq!(
                e.preview_rendered.as_ref().unwrap().2.composition.crop(),
                crop
            );
        });
    });
}
