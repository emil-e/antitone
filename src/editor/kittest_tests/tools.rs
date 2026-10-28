use super::*;

pub(super) fn calibrated_harness<R>(test: impl FnOnce(Harness<'_, Editor>) -> R) -> R {
    calibrated_harness_with_thumbnails(false, test)
}

pub(super) fn calibrated_harness_with_thumbnails<R>(
    thumbnails: bool,
    test: impl FnOnce(Harness<'_, Editor>) -> R,
) -> R {
    editor_harness_with_thumbnails(thumbnails, |mut harness, _| {
        harness.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        harness.update(|editor| {
            editor.set_dmin(Some(DVec3::splat(3000. / 4031.)));
            editor.set_dmax(Some(DVec3::splat(128. / 4031.)));
            editor.set_gamma(0.75);
            editor.document.visit.adjustment_layer_settings.exposure = 0.5;
            editor.document.visit.adjustment_layer_settings.wb = Balance {
                warmth: 0.1,
                tint: -0.05,
            };
            editor.document.source = false;
            editor.sync_document();
        });
        harness.step();
        test(harness)
    })
}

fn click_patch(harness: &mut Harness<'_, Editor>, point: egui::Pos2) {
    let at = harness.read(|editor| {
        editor
            .document
            .visit
            .last_layout
            .unwrap()
            .0
            .screen_point(point)
    });
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
    harness.read(|editor| assert!(editor.document.picking.region.is_some()));
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
    harness.update(|editor| editor.document.visit.selection_timing = Some(timing.clone()));
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
fn precision_tools_paint_completed_originals_in_native_and_straightened_frames() {
    for angle in [0., 0.1] {
        editor_harness(|mut h, capture| {
            h.enter(|e, cx| e.add_raws(vec![fixture("patches.dng")], cx));
            wait_for(&mut h, |e| !e.busy && e.document.visit.ready.is_some());
            h.update(|e| {
                e.set_dmin(Some(DVec3::splat(3000. / 4031.)));
                e.set_dmax(Some(DVec3::splat(128. / 4031.)));
                e.document.source = false;
                // Cancel the documented +1 EV source-display baseline for the scalar oracle.
                e.source_exposure = -1.;
                e.document.visit.photo_orientation.turn_clockwise();
                e.document.roll_orientation.horizontal = true;
                e.document.visit.crop = Some(Crop {
                    center: [128., 96.],
                    size: [220, 140],
                    angle,
                });
                e.sync_document();
                // Make the coarse proxy deliberately distinguishable from real originals.
                // The original service still reads the unchanged authored RAW.
                let services = e.source_services.as_ref().unwrap();
                let ready = e.document.visit.ready.as_mut().unwrap();
                let renderer = services
                    .renderers
                    .get(&services.device, &services.queue)
                    .unwrap();
                ready.proxy = crate::viewer::GpuProxy::new_continuous(
                    renderer,
                    &services.device,
                    [2, 2],
                    &[[0.; 4]; 4],
                    ready.proxy.matrix(),
                    None,
                    crate::gpu::SourceGeometry::new(ready.record.development.active_area, [2, 2])
                        .unwrap(),
                )
                .unwrap();
            });
            let (tab, label, kind) = if angle == 0. {
                ("Roll", "Pick Dmin (film base)", Pick::Dmin)
            } else {
                ("Photo", "Pick neutral for white balance", Pick::Wb)
            };
            stage(&mut h, tab);
            h.get_by_label(label).scroll_to_me();
            h.try_run_realtime().unwrap();
            h.get_by_label(label).click();
            h.step();
            h.read(|e| assert_eq!(e.document.picking.kind, Some(kind)));
            wait_for(&mut h, |e| {
                e.document.visit.grade.original.settled.is_some()
            });
            h.event(egui::Event::PointerGone);
            h.step();
            h.render().unwrap();
            let (layout, target, matrix, pipeline) = h.read(|e| {
                let (_, view, _) = e.document.visit.grade.original.settled.as_ref().unwrap();
                assert_eq!(view.frame.is_some(), angle != 0.);
                let matrix = e.document.visit.ready.as_ref().unwrap().proxy.matrix();
                (
                    e.document.visit.last_layout.unwrap().0,
                    e.document.target,
                    matrix,
                    (!e.document.source).then(|| {
                        e.adjustment_layer_snapshot(
                            e.document
                                .state
                                .project
                                .source(e.document.visit.current_source.unwrap())
                                .unwrap(),
                            e.document.visit.adjustment_layer_settings,
                        )
                        .and_then(|layers| {
                            crate::adjustment_layers::AdjustmentLayerPipeline::new(
                                e.document.input,
                                e.document.roll,
                                layers,
                                matrix,
                            )
                        })
                        .unwrap()
                    }),
                )
            });
            let cpu = crate::rendering::CpuRenderer::new(target).unwrap();
            let pixels = capture.borrow();
            for (point, code) in [([96., 32.], 128.), ([160., 96.], 3200.)] {
                let position = layout.screen_point(egui::pos2(point[0], point[1]));
                assert!(
                    layout.image.shrink(4.).contains(position) && layout.canvas.contains(position)
                );
                let camera = DVec3::splat(code / 4031.);
                let working = match &pipeline {
                    Some(pipeline) => pipeline.apply(camera, point.map(|v| v as u32)).unwrap().rgb,
                    None => matrix * camera,
                };
                let linear =
                    target.linear_to_srgb() * cpu.render(&[working]).unwrap()[0].as_dvec3();
                let actual = pixels.pixels[position.y.floor() as usize * pixels.size[0] as usize
                    + position.x.floor() as usize];
                for c in 0..3 {
                    let expected = if linear[c] <= 0.0031308 {
                        linear[c] * 12.92
                    } else {
                        1.055 * linear[c].powf(1. / 2.4) - 0.055
                    };
                    assert!(
                        (f64::from(actual[c]) - expected).abs() < 0.004,
                        "angle={angle}, point={point:?}: {actual:?} vs {expected}"
                    );
                }
            }
            drop(pixels);
            h.get_by_label("Cancel").click();
            h.step();
            h.read(|e| {
                assert!(
                    e.document.visit.grade.original.settled.is_none(),
                    "departed tool must not retain its image"
                )
            });
        });
    }
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
            let before = harness.read(|editor| editor.document.state.project.clone());
            crop_draft(&mut harness);
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            capture_tool(&mut harness, &format!("crop-{layout}-offscreen"), size);
            harness.get_by_label("Cancel").click();
            harness.step();
            harness.read(|editor| assert!(editor.document.visit.crop_edit.is_none()));
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            let draft = crop_draft(&mut harness);
            harness.get_by_label("Apply").click();
            harness.step();
            harness.read(|editor| assert!(editor.document.visit.crop_edit.is_none()));
            harness.read(|editor| assert_eq!(editor.document.visit.crop, Some(draft)));
            let mut expected = before;
            harness.read(|editor| {
                expected
                    .source_mut(editor.document.visit.current_source.unwrap())
                    .unwrap()
                    .crop = Some(draft)
            });
            harness.read(|editor| assert_eq!(editor.document.state.project, expected));

            harness.get_by_label("Scopes").click();
            harness.step();
            for (name, tab, label, kind) in [
                ("dmin", "Roll", "Pick Dmin (film base)", Pick::Dmin),
                ("wb", "Photo", "Pick neutral for white balance", Pick::Wb),
            ] {
                stage(&mut harness, tab);
                let before = harness.read(|editor| editor.document.state.project.clone());
                harness.get_by_label(label).scroll_to_me();
                harness.try_run_realtime().unwrap();
                harness.get_by_label(label).click();
                harness.step();
                harness.read(|editor| assert_eq!(editor.document.picking.kind, Some(kind)));
                assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
                harness.get_by_label("Apply").click();
                harness.step();
                harness.read(|editor| assert!(!editor.busy));
                harness.read(|editor| assert_eq!(editor.document.state.project, before));
                click_patch(&mut harness, egui::pos2(96., 32.));
                capture_tool(&mut harness, &format!("{name}-{layout}-offscreen"), size);
                harness.get_by_label("Cancel").click();
                harness.step();
                harness.read(|editor| assert!(editor.document.picking.kind.is_none()));
                harness.read(|editor| assert_eq!(editor.document.state.project, before));
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
                harness.read(|editor| {
                    assert!(editor.document.picking.kind.is_none(), "{}", editor.status)
                });
                let mut expected = before;
                if kind == Pick::Dmin {
                    let measured = harness.read(|editor| editor.dmin().unwrap());
                    assert!(
                        (measured - DVec3::splat(3000. / 4031.)).abs().max_element() <= 2. / 65535.
                    );
                    expected.set_dmin(Some(measured.to_array()));
                } else {
                    harness.read(|editor| {
                        assert_ne!(
                            editor.document.visit.adjustment_layer_settings.wb,
                            Balance {
                                warmth: 0.1,
                                tint: -0.05
                            }
                        )
                    });
                    harness.read(|editor| {
                        expected
                            .source_mut(editor.document.visit.current_source.unwrap())
                            .unwrap()
                            .adjustment_layers
                            .as_mut()
                            .unwrap()
                            .layers[0]
                            .adjustment
                            .photo
                            .wb = editor.document.visit.adjustment_layer_settings.wb
                    });
                }
                harness.read(|editor| assert_eq!(editor.document.state.project, expected));
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
                        editor.document.roll_orientation.turn_clockwise();
                        editor.document.visit.photo_orientation.flips.horizontal = true;
                        editor.sync_document();
                    });
                    harness.step();
                }
                let before = harness.read(|editor| editor.document.state.project.clone());
                harness.get_by_label("Pick Dmin (film base)").scroll_to_me();
                harness.try_run_realtime().unwrap();
                harness.get_by_label("Pick Dmin (film base)").click();
                harness.step();
                harness.update(|editor| {
                    editor.document.picking.region = initial.map(Coverage::rectangle)
                });
                harness.step();
                let geometry = harness.read(|editor| editor.document.visit.last_layout.unwrap().0);
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
                            .document
                            .picking
                            .region
                            .unwrap()
                            .exact_rectangle(editor.document.visit.ready.as_ref().unwrap().area),
                        Some(expected),
                        "oriented={oriented}, initial={initial:?}"
                    )
                });
                let selection = harness.read(|editor| editor.document.picking.region);
                harness.event(egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                });
                harness.step();
                harness.read(|editor| assert!(editor.document.visit.pick_drag.is_none()));
                harness.read(|editor| assert_eq!(editor.document.picking.region, selection));
                harness.read(|editor| assert_eq!(editor.document.state.project, before));
                harness.get_by_label("Cancel").click();
                harness.step();
                harness.read(|editor| assert!(editor.document.picking.kind.is_none()));
                harness.read(|editor| assert_eq!(editor.document.state.project, before));
            });
        }
    }
}

#[test]
fn flat_reference_support_fits_both_layouts_and_widgets_cancel_or_apply() {
    editor_harness(|mut harness, _| {
        harness.enter(|editor, cx| editor.add_raws(vec![fixture("illuminated-patches.dng")], cx));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.visit.ready.is_some(), "{}", editor.status));
        harness.get_by_label("Scopes").click();
        harness.step();
        let before = harness.read(|editor| editor.document.state.project.clone());
        let selected = harness.read(|editor| editor.document.visit.current_source);
        // The native file chooser is outside this test: supply its resulting path as fixture setup.
        harness.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        for (layout, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            harness.get_by_label("Fit").click();
            harness.step();
            harness.read(|editor| assert_eq!(editor.document.picking.kind, Some(Pick::Flat)));
            harness.read(|editor| assert!(editor.document.flat_edit.is_some()));
            wait_for(&mut harness, |e| {
                assert!(
                    e.document.visit.grade.original.refinement_error.is_none(),
                    "{}",
                    e.status
                );
                e.document.visit.grade.original.settled.is_some()
            });
            let geometry = harness.read(|editor| editor.document.visit.last_layout.unwrap().0);
            let support =
                harness.read(|editor| editor.document.picking.region.expect("proposed support"));
            assert!(geometry.canvas.expand(0.1).contains_rect(geometry.image));
            assert!(
                geometry
                    .canvas
                    .expand(0.1)
                    .contains_rect(geometry.screen_coverage(support))
            );
            harness.read(|editor| assert_eq!(editor.document.state.project, before));
            capture_tool(&mut harness, &format!("flat-{layout}-offscreen"), size);
        }
        harness.get_by_label("Cancel").click();
        harness.step();
        harness.read(|editor| assert!(editor.document.flat_edit.is_none()));
        harness.read(|editor| assert_eq!(editor.document.state.project, before));
        harness.read(|editor| assert_eq!(editor.document.visit.current_source, selected));
        harness.enter(|editor, cx| editor.begin_flat_edit(fixture("bare-light.dng"), None, cx));
        wait_for(&mut harness, |e| !e.busy);
        harness.get_by_label("Apply").click();
        harness.step();
        harness.read(|editor| assert!(editor.busy));
        wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
        harness.read(|editor| assert!(editor.document.correction.is_some(), "{}", editor.status));
        harness.read(|editor| assert!(editor.document.flat_edit.is_none()));
        harness.read(|editor| assert_eq!(editor.document.visit.current_source, selected));
        harness.read(|editor| assert_eq!(editor.document.state.project.order, before.order));
        harness.read(|editor| assert_eq!(editor.document.state.project.dmin(), before.dmin()));
        harness.read(|editor| assert_eq!(editor.document.state.project.dmax(), before.dmax()));
        harness.read(|editor| assert!(editor.preference_store.is_none()));
    });
}
