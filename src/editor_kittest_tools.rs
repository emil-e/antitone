use super::*;

pub(super) fn calibrated_harness<R>(test: impl FnOnce(Harness<'_, Editor>) -> R) -> R {
    calibrated_harness_with_thumbnails(false, test)
}

pub(super) fn calibrated_harness_with_thumbnails<R>(
    thumbnails: bool,
    test: impl FnOnce(Harness<'_, Editor>) -> R,
) -> R {
    editor_harness_with_thumbnails(thumbnails, |mut harness, _| {
        harness.update(|editor| editor.add_raws(vec![fixture("patches.dng")]));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        harness.update(|editor| {
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
        });
        harness.step();
        test(harness)
    })
}

fn click_patch(harness: &mut Harness<'_, Editor>, point: egui::Pos2) {
    let at = harness.read(|editor| editor.last_layout.unwrap().0.screen_point(point));
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
    harness.read(|editor| assert!(editor.region.is_some()));
}

fn capture_tool(harness: &mut Harness<'_, Editor>, name: &str, size: egui::Vec2) {
    for label in ["Apply", "Cancel"] {
        let rect = harness
            .get_by_role_and_label(egui::accesskit::Role::Button, label)
            .rect();
        assert!(rect.left() >= 0. && rect.right() <= size.x);
        assert!(rect.top() >= 0. && rect.bottom() <= size.y);
    }
    let timing = harness.read(|editor| Arc::new(SelectionTiming::new(editor.id, Instant::now())));
    harness.update(|editor| editor.selection_timing = Some(timing.clone()));
    harness.step();
    let png = harness.render().unwrap();
    assert_eq!(png.dimensions(), (size.x as u32, size.y as u32));
    assert!(timing.ready_was_painted());
    save_capture(&png, name);
}

pub(super) fn crop_draft(harness: &mut Harness<'_, Editor>) -> Crop {
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
        .click();
    harness.step();
    harness.read(|editor| assert!(editor.document.visit.crop_edit.is_some()));
    harness.step();
    harness
        .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Angle")
        .scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness
        .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Angle")
        .click();
    harness.step();
    assert!(harness.ctx.text_edit_focused());
    harness.event(egui::Event::Text("2.3".into()));
    harness.event(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
    let layout = harness.read(|editor| {
        editor
            .document
            .visit
            .crop_edit
            .as_ref()
            .unwrap()
            .layout
            .unwrap()
    });
    drag(
        harness,
        layout.image.left_top(),
        layout.image.left_top() + egui::vec2(30., 20.),
    );
    let draft = harness.read(|editor| editor.document.visit.crop_edit.as_ref().unwrap().draft);
    assert!((draft.angle.abs().to_degrees() - 2.3).abs() < 1e-6);
    harness.read(|editor| {
        assert_ne!(
            draft,
            Crop::from_region(editor.document.visit.ready.as_ref().unwrap().area)
        )
    });
    draft
}

#[test]
fn viewer_tools_use_widget_drafts_cancel_and_apply_at_both_layouts() {
    for (layout, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        calibrated_harness(|mut harness| {
            set_layout(&mut harness, size, width);
            harness.get_by_label("Scopes").click();
            harness.step();
            stage(&mut harness, "Photo");
            let before = harness.read(|editor| editor.document.project.clone());
            crop_draft(&mut harness);
            harness.read(|editor| assert_eq!(editor.document.project, before));
            capture_tool(&mut harness, &format!("crop-{layout}-offscreen"), size);
            harness.get_by_label("Cancel").click();
            harness.step();
            harness.read(|editor| assert!(editor.document.visit.crop_edit.is_none()));
            harness.read(|editor| assert_eq!(editor.document.project, before));
            let draft = crop_draft(&mut harness);
            harness.get_by_label("Apply").click();
            harness.step();
            harness.read(|editor| assert!(editor.document.visit.crop_edit.is_none()));
            harness.read(|editor| assert_eq!(editor.crop, Some(draft)));
            let mut expected = before;
            harness.read(|editor| {
                expected
                    .source_mut(editor.current_source.unwrap())
                    .unwrap()
                    .crop = Some(draft)
            });
            harness.read(|editor| assert_eq!(editor.document.project, expected));

            harness.get_by_label("Scopes").click();
            harness.step();
            for (name, tab, label, kind) in [
                ("dmin", "Roll", "Pick Dmin (film base)", Pick::Dmin),
                ("wb", "Photo", "Pick neutral…", Pick::Wb),
            ] {
                stage(&mut harness, tab);
                let before = harness.read(|editor| editor.document.project.clone());
                harness.get_by_label(label).scroll_to_me();
                harness.try_run_realtime().unwrap();
                harness.get_by_label(label).click();
                harness.step();
                harness.read(|editor| assert_eq!(editor.pick, Some(kind)));
                assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
                harness.get_by_label("Apply").click();
                harness.step();
                harness.read(|editor| assert!(!editor.busy));
                harness.read(|editor| assert_eq!(editor.document.project, before));
                click_patch(&mut harness, egui::pos2(96., 32.));
                capture_tool(&mut harness, &format!("{name}-{layout}-offscreen"), size);
                harness.get_by_label("Cancel").click();
                harness.step();
                harness.read(|editor| assert!(editor.pick.is_none()));
                harness.read(|editor| assert_eq!(editor.document.project, before));
                harness.get_by_label(label).click();
                harness.step();
                click_patch(
                    &mut harness,
                    egui::pos2(if kind == Pick::Dmin { 32. } else { 96. }, 32.),
                );
                harness.get_by_label("Apply").click();
                harness.step();
                harness.read(|editor| assert!(editor.busy));
                wait_for(&mut harness, |e| !e.busy);
                harness.read(|editor| assert!(editor.pick.is_none(), "{}", editor.status));
                let mut expected = before;
                if kind == Pick::Dmin {
                    let measured = harness.read(|editor| editor.dmin.unwrap());
                    assert!(
                        (measured - DVec3::splat(3000. / 4031.)).abs().max_element() <= 2. / 65535.
                    );
                    expected.dmin = Some(measured.to_array());
                } else {
                    harness.read(|editor| {
                        assert_ne!(
                            editor.photo.wb,
                            Balance {
                                warmth: 0.1,
                                tint: -0.05
                            }
                        )
                    });
                    harness.read(|editor| {
                        expected
                            .source_mut(editor.current_source.unwrap())
                            .unwrap()
                            .photo
                            .wb = editor.photo.wb
                    });
                }
                harness.read(|editor| assert_eq!(editor.document.project, expected));
            }
            harness.read(|editor| assert!(editor.preference_store.is_none()));
        });
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
            calibrated_harness(|mut harness| {
                if oriented {
                    harness.update(|editor| {
                        editor.roll_orientation.turn_clockwise();
                        editor.photo_orientation.flips.horizontal = true;
                        editor.sync_document();
                    });
                    harness.step();
                }
                let before = harness.read(|editor| editor.document.project.clone());
                harness.get_by_label("Pick Dmin (film base)").scroll_to_me();
                harness.try_run_realtime().unwrap();
                harness.get_by_label("Pick Dmin (film base)").click();
                harness.step();
                harness.update(|editor| editor.region = initial.map(Coverage::rectangle));
                harness.step();
                let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
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
                harness.read(|editor| {
                    assert_eq!(
                        editor
                            .region
                            .unwrap()
                            .exact_rectangle(editor.document.visit.ready.as_ref().unwrap().area),
                        Some(expected),
                        "oriented={oriented}, initial={initial:?}"
                    )
                });
                let selection = harness.read(|editor| editor.region);
                harness.event(egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                harness.step();
                harness.read(|editor| assert!(editor.pick_drag.is_none()));
                harness.read(|editor| assert_eq!(editor.region, selection));
                harness.read(|editor| assert_eq!(editor.document.project, before));
                harness.get_by_label("Cancel").click();
                harness.step();
                harness.read(|editor| assert!(editor.pick.is_none()));
                harness.read(|editor| assert_eq!(editor.document.project, before));
            });
        }
    }
}

#[test]
fn flat_reference_support_fits_both_layouts_and_widgets_cancel_or_apply() {
    editor_harness(|mut harness, _| {
        harness.update(|editor| editor.add_raws(vec![fixture("illuminated-patches.dng")]));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        harness.get_by_label("Scopes").click();
        harness.step();
        let before = harness.read(|editor| editor.document.project.clone());
        let selected = harness.read(|editor| editor.current_source);
        // The native file chooser is outside this test: supply its resulting path as fixture setup.
        harness.update(|editor| editor.begin_flat_edit(fixture("bare-light.dng"), None));
        wait_for(&mut harness, |e| !e.busy);
        for (layout, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            harness.get_by_label("Fit").click();
            harness.step();
            harness.read(|editor| assert_eq!(editor.pick, Some(Pick::Flat)));
            harness.read(|editor| assert!(editor.flat_edit.is_some()));
            wait_for(&mut harness, |e| {
                assert!(e.original.refinement_error.is_none(), "{}", e.status);
                e.original.settled.is_some()
            });
            let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
            let support = harness.read(|editor| editor.region.expect("proposed support"));
            assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
            assert!(
                geometry
                    .canvas
                    .expand(0.1)
                    .contains_rect(geometry.screen_coverage(support))
            );
            harness.read(|editor| assert_eq!(editor.document.project, before));
            capture_tool(&mut harness, &format!("flat-{layout}-offscreen"), size);
        }
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert!(editor.flat_edit.is_none()));
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness.read(|editor| assert_eq!(editor.current_source, selected));
        harness.update(|editor| editor.begin_flat_edit(fixture("bare-light.dng"), None));
        wait_for(&mut harness, |e| !e.busy);
        harness.get_by_label("Apply").click();
        harness.step();
        harness.read(|editor| assert!(editor.busy));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.correction.is_some(), "{}", editor.status));
        harness.read(|editor| assert!(editor.flat_edit.is_none()));
        harness.read(|editor| assert_eq!(editor.current_source, selected));
        harness.read(|editor| assert_eq!(editor.document.project.order, before.order));
        harness.read(|editor| assert_eq!(editor.document.project.dmin, before.dmin));
        harness.read(|editor| assert_eq!(editor.document.project.dmax, before.dmax));
        harness.read(|editor| assert!(editor.preference_store.is_none()));
    });
}
