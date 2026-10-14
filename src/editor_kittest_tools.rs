use super::*;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/raw")
        .join(name)
}

fn calibrated_harness() -> Harness<'static, Editor> {
    let (mut harness, _) = editor_harness();
    harness.state_mut().add_raws(vec![fixture("patches.dng")]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    let editor = harness.state_mut();
    editor.dmin = Some(DVec3::splat(3000. / 4031.));
    editor.dmax = Some(DVec3::splat(128. / 4031.));
    editor.roll.gamma = 0.75;
    editor.photo.exposure = 0.5;
    editor.photo.wb = Balance {
        warmth: 0.1,
        tint: -0.05,
    };
    editor.source = false;
    editor.sync_document();
    harness.step();
    harness
}

fn stage(harness: &mut Harness<'_, Editor>, name: &str) {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Tab, name)
        .click();
    harness.step();
    harness.try_run_realtime().unwrap();
}

fn click_patch(harness: &mut Harness<'_, Editor>, point: egui::Pos2) {
    let at = harness.state().last_layout.unwrap().0.screen_point(point);
    harness.hover_at(at);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.step();
    assert!(harness.state().region.is_some());
}

fn drag(harness: &mut Harness<'_, Editor>, start: egui::Pos2, end: egui::Pos2) {
    harness.hover_at(start);
    harness.event(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    harness.hover_at(end);
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: end,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

fn capture_tool(harness: &mut Harness<'_, Editor>, name: &str, size: egui::Vec2) {
    for label in ["Apply", "Cancel"] {
        let rect = harness
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .rect();
        assert!(rect.left() >= 0. && rect.right() <= size.x);
        assert!(rect.top() >= 0. && rect.bottom() <= size.y);
    }
    let timing = Arc::new(SelectionTiming::new(harness.state().id, Instant::now()));
    harness.state_mut().selection_timing = Some(timing.clone());
    harness.step();
    let png = harness.render().unwrap();
    assert_eq!(png.dimensions(), (size.x as u32, size.y as u32));
    assert!(timing.ready_was_painted());
    save_capture(&png, name);
}

fn crop_draft(harness: &mut Harness<'_, Editor>) -> Crop {
    harness.get_by_label("Crop…").scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("Crop…").click();
    harness.step();
    assert!(harness.state().crop_edit.is_some());
    harness.step();
    harness.get_by_label("Angle").click();
    harness.step();
    harness.event(egui::Event::Text("2.3".into()));
    harness.event(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let layout = harness.state().crop_edit.as_ref().unwrap().layout.unwrap();
    drag(
        harness,
        layout.image.left_top(),
        layout.image.left_top() + egui::vec2(30., 20.),
    );
    let draft = harness.state().crop_edit.as_ref().unwrap().draft;
    assert!((draft.angle.abs().to_degrees() - 2.3).abs() < 1e-6);
    assert_ne!(
        draft,
        Crop::from_region(harness.state().ready.as_ref().unwrap().area)
    );
    draft
}

#[test]
fn viewer_tools_use_widget_drafts_cancel_and_apply_at_both_layouts() {
    for (layout, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        let mut harness = calibrated_harness();
        set_layout(&mut harness, size, width);
        harness.get_by_label("Scopes").click();
        harness.step();
        stage(&mut harness, "Photo");
        let before = harness.state().document.project.clone();
        crop_draft(&mut harness);
        assert_eq!(harness.state().document.project, before);
        capture_tool(&mut harness, &format!("crop-{layout}-offscreen"), size);
        harness.get_by_label("Cancel").click();
        harness.step();
        assert!(harness.state().crop_edit.is_none());
        assert_eq!(harness.state().document.project, before);
        let draft = crop_draft(&mut harness);
        harness.get_by_label("Apply").click();
        harness.step();
        assert!(harness.state().crop_edit.is_none());
        assert_eq!(harness.state().crop, Some(draft));
        let mut expected = before;
        expected
            .source_mut(harness.state().current_source.unwrap())
            .unwrap()
            .crop = Some(draft);
        assert_eq!(harness.state().document.project, expected);

        harness.get_by_label("Scopes").click();
        harness.step();
        for (name, tab, label, kind) in [
            ("dmin", "Roll", "Pick Dmin (film base)", Pick::Dmin),
            ("wb", "Photo", "Pick neutral…", Pick::Wb),
        ] {
            stage(&mut harness, tab);
            let before = harness.state().document.project.clone();
            harness.get_by_label(label).scroll_to_me();
            harness.try_run_realtime().unwrap();
            harness.get_by_label(label).click();
            harness.step();
            assert_eq!(harness.state().pick, Some(kind));
            assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
            harness.get_by_label("Apply").click();
            harness.step();
            assert!(!harness.state().busy);
            assert_eq!(harness.state().document.project, before);
            click_patch(&mut harness, egui::pos2(96., 32.));
            capture_tool(&mut harness, &format!("{name}-{layout}-offscreen"), size);
            harness.get_by_label("Cancel").click();
            harness.step();
            assert!(harness.state().pick.is_none());
            assert_eq!(harness.state().document.project, before);
            harness.get_by_label(label).click();
            harness.step();
            click_patch(
                &mut harness,
                egui::pos2(if kind == Pick::Dmin { 32. } else { 96. }, 32.),
            );
            harness.get_by_label("Apply").click();
            harness.step();
            assert!(harness.state().busy);
            wait_for(&mut harness, |e| !e.busy);
            assert!(harness.state().pick.is_none(), "{}", harness.state().status);
            let mut expected = before;
            if kind == Pick::Dmin {
                let measured = harness.state().dmin.unwrap();
                assert!(
                    (measured - DVec3::splat(3000. / 4031.)).abs().max_element() <= 2. / 65535.
                );
                expected.dmin = Some(measured.to_array());
            } else {
                assert_ne!(
                    harness.state().photo.wb,
                    Balance {
                        warmth: 0.1,
                        tint: -0.05
                    }
                );
                expected
                    .source_mut(harness.state().current_source.unwrap())
                    .unwrap()
                    .photo
                    .wb = harness.state().photo.wb;
            }
            assert_eq!(harness.state().document.project, expected);
        }
        assert!(harness.state().preference_store.is_none());
    }
}

#[test]
fn picker_drags_keep_press_origin_for_resize_move_and_new_selection() {
    for oriented in [false, true] {
        for (initial, start, end, expected) in [
            (
                Some(Rect {
                    x: 64,
                    y: 48,
                    width: 96,
                    height: 80,
                }),
                egui::pos2(64., 48.),
                egui::pos2(96., 80.),
                Rect {
                    x: 96,
                    y: 80,
                    width: 64,
                    height: 48,
                },
            ),
            (
                Some(Rect {
                    x: 64,
                    y: 48,
                    width: 96,
                    height: 80,
                }),
                egui::pos2(112., 88.),
                egui::pos2(144., 120.),
                Rect {
                    x: 96,
                    y: 80,
                    width: 96,
                    height: 80,
                },
            ),
            (
                None,
                egui::pos2(48., 48.),
                egui::pos2(176., 144.),
                Rect {
                    x: 48,
                    y: 48,
                    width: 128,
                    height: 96,
                },
            ),
        ] {
            let mut harness = calibrated_harness();
            if oriented {
                let editor = harness.state_mut();
                editor.roll_orientation.turn_clockwise();
                editor.photo_orientation.flips.horizontal = true;
                editor.sync_document();
                harness.step();
            }
            let before = harness.state().document.project.clone();
            harness.get_by_label("Pick Dmin (film base)").scroll_to_me();
            harness.try_run_realtime().unwrap();
            harness.get_by_label("Pick Dmin (film base)").click();
            harness.step();
            harness.state_mut().region = initial.map(Coverage::rectangle);
            harness.step();
            let geometry = harness.state().last_layout.unwrap().0;
            let start = geometry.screen_point(start);
            let end = geometry.screen_point(end);
            assert!(start.distance(end) > 8.);
            harness.hover_at(start);
            harness.event(egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            // No intermediate motion: the first recognized drag is already far from the press.
            harness.hover_at(end);
            harness.step();
            assert_eq!(
                harness
                    .state()
                    .region
                    .unwrap()
                    .exact_rectangle(harness.state().ready.as_ref().unwrap().area),
                Some(expected),
                "oriented={oriented}, initial={initial:?}"
            );
            let selection = harness.state().region;
            harness.event(egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            assert!(harness.state().pick_drag.is_none());
            assert_eq!(harness.state().region, selection);
            assert_eq!(harness.state().document.project, before);
            harness.get_by_label("Cancel").click();
            harness.step();
            assert!(harness.state().pick.is_none());
            assert_eq!(harness.state().document.project, before);
        }
    }
}

#[test]
fn flat_reference_support_fits_both_layouts_and_widgets_cancel_or_apply() {
    let (mut harness, _) = editor_harness();
    harness
        .state_mut()
        .add_raws(vec![fixture("illuminated-patches.dng")]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    harness.get_by_label("Scopes").click();
    harness.step();
    let before = harness.state().document.project.clone();
    let selected = harness.state().current_source;
    // The native file chooser is outside this test: supply its resulting path as fixture setup.
    harness
        .state_mut()
        .begin_flat_edit(fixture("bare-light.dng"), None);
    wait_for(&mut harness, |e| !e.busy);
    for (layout, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        set_layout(&mut harness, size, width);
        harness.get_by_label("Fit").click();
        harness.step();
        assert_eq!(harness.state().pick, Some(Pick::Flat));
        assert!(harness.state().flat_edit.is_some());
        wait_for(&mut harness, |e| {
            assert!(e.refinement_error.is_none(), "{}", e.status);
            e.settled.is_some()
        });
        let geometry = harness.state().last_layout.unwrap().0;
        let support = harness.state().region.expect("proposed support");
        assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
        assert!(
            geometry
                .canvas
                .expand(0.1)
                .contains_rect(geometry.screen_coverage(support))
        );
        assert_eq!(harness.state().document.project, before);
        capture_tool(&mut harness, &format!("flat-{layout}-offscreen"), size);
    }
    harness.get_by_label("Cancel").click();
    harness.step();
    assert!(harness.state().flat_edit.is_none());
    assert_eq!(harness.state().document.project, before);
    assert_eq!(harness.state().current_source, selected);
    harness
        .state_mut()
        .begin_flat_edit(fixture("bare-light.dng"), None);
    wait_for(&mut harness, |e| !e.busy);
    harness.get_by_label("Apply").click();
    harness.step();
    assert!(harness.state().busy);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().correction.is_some(),
        "{}",
        harness.state().status
    );
    assert!(harness.state().flat_edit.is_none());
    assert_eq!(harness.state().current_source, selected);
    assert_eq!(harness.state().document.project.order, before.order);
    assert_eq!(harness.state().document.project.dmin, before.dmin);
    assert_eq!(harness.state().document.project.dmax, before.dmax);
    assert!(harness.state().preference_store.is_none());
}
