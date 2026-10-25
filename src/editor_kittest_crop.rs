use super::*;
use glam::DMat3;

fn assert_crop_patch_pixels(harness: &mut Harness<'_, Editor>, capture: &RefCell<FloatCapture>) {
    harness.event(egui::Event::PointerGone);
    harness.step();
    harness.render().unwrap();
    let (layout, pipeline, target) = harness.read(|editor| {
        let ready = editor.document.visit.ready.as_ref().unwrap();
        (
            editor
                .document
                .visit
                .crop_edit
                .as_ref()
                .unwrap()
                .layout
                .unwrap(),
            crate::processing::Pipeline::new(
                editor.input,
                editor.roll,
                editor.photo,
                DMat3::from_cols_array_2d(&ready.record.development.camera_to_ap1),
            )
            .unwrap(),
            editor.target,
        )
    });
    let cpu = crate::rendering::CpuRenderer::new(target).unwrap();
    let output = capture.borrow();
    let mut checked = 0;
    // Authored uniform tile centres, independent of the proxy/composition implementation.
    for (point, codes) in [
        ([32., 32.], [3000., 3000., 3000.]),
        ([96., 32.], [128., 128., 128.]),
        ([160., 32.], [1000., 1000., 1000.]),
        ([224., 32.], [2400., 1200., 600.]),
        ([32., 96.], [1200., 2400., 600.]),
        ([96., 96.], [600., 1200., 2400.]),
        ([160., 96.], [3200., 3200., 3200.]),
        ([224., 96.], [600., 1000., 1800.]),
        ([32., 160.], [256., 256., 256.]),
        ([96., 160.], [512., 512., 512.]),
        ([160., 160.], [1024., 1024., 1024.]),
        ([224., 160.], [2048., 2048., 2048.]),
    ] {
        let position = layout.screen_point(egui::pos2(point[0], point[1]));
        // Outside the frame is intentionally shaded; leave artwork and clipped pixels out.
        if !layout.image.shrink(12.).contains(position)
            || !layout.canvas.shrink(12.).contains(position)
        {
            continue;
        }
        let camera = DVec3::from_array(codes) / 4031.;
        let working = pipeline.apply(camera).unwrap().rgb;
        let linear = target.linear_to_srgb() * cpu.render(&[working]).unwrap()[0].as_dvec3();
        let actual = output.pixels
            [position.y.floor() as usize * output.size[0] as usize + position.x.floor() as usize];
        for c in 0..3 {
            let value = linear[c].abs();
            let expected = linear[c].signum()
                * if value <= 0.0031308 {
                    value * 12.92
                } else {
                    1.055 * value.powf(1. / 2.4) - 0.055
                };
            assert!(
                (f64::from(actual[c]) - expected).abs() < 0.004,
                "source={point:?}, screen={position:?}, actual={actual:?}, expected={expected}, linear={linear:?}, frame={:?}",
                layout.composition,
            );
        }
        checked += 1;
    }
    assert!(
        checked >= 2,
        "fixture must expose multiple distinguishable patches, got {checked}"
    );
}

#[test]
fn crop_preview_keeps_original_coordinate_repairs_through_rotation_and_resize() {
    let directory = tempfile::tempdir().unwrap();
    let mut bytes = std::fs::read(fixture("patches.dng")).unwrap();
    assert_eq!(
        bytes.len(),
        98_950,
        "declared single-strip authored fixture"
    );
    let strip = bytes.len() - 256 * 192 * 2;
    // A tiny dark sensor defect inside the independently declared uniform 3200-code plateau.
    for y in 94..98 {
        for x in 158..162 {
            let at = strip + (y * 256 + x) * 2;
            assert_eq!(
                u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()),
                3264
            );
            bytes[at..at + 2].copy_from_slice(&192_u16.to_le_bytes());
        }
    }
    let damaged = directory.path().join("damaged.dng");
    std::fs::write(&damaged, bytes).unwrap();
    editor_harness(|mut h, capture| {
        h.enter(|e, cx| e.add_raws(vec![damaged], cx));
        wait_for(&mut h, |e| !e.busy && e.document.visit.ready.is_some());
        h.update(|e| {
            e.input = crate::processing::InputSettings::Digital;
            e.source = false;
            e.photo.exposure = 0.75;
            e.photo_orientation.turn_clockwise();
            e.roll_orientation.horizontal = true;
            e.sync_document();
        });
        wait_for(&mut h, |e| e.working.idle());
        h.event(egui::Event::PointerGone);
        h.step();
        h.render().unwrap();
        let damaged_value = sampled_patch(&mut h, &capture, false);
        h.update(|e| {
            let id = e.current_source.unwrap();
            e.document
                .project
                .source_mut(id)
                .unwrap()
                .repairs
                .insert(
                    crate::dust_repair::SupportMask::new([155, 91], [10, 10], vec![true; 100])
                        .unwrap(),
                    crate::dust_repair::RepairOrigin::Manual,
                    crate::dust_repair::DonorChoice::Offset([0, 64]),
                )
                .unwrap();
        });
        // This fixture installs an accepted instruction; creation gestures have their own cases.
        h.enter(|e, cx| e.schedule_dust(e.dust_signature().unwrap(), cx));
        h.step();
        h.read(|e| {
            assert!(!e.source && e.document.pick_return.is_none() && e.flat_edit.is_none());
            assert!(e.dust_signature().is_some());
            assert_eq!(
                e.dust_repairs(e.current_source.unwrap())
                    .unwrap()
                    .repairs()
                    .len(),
                1
            );
        });
        wait_for(&mut h, |e| {
            e.dust_signature()
                .is_some_and(|s| e.dust_evaluations(s).len() == 1)
                && e.working.idle()
        });
        let healthy = sampled_patch(&mut h, &capture, true);
        assert!(
            (damaged_value - healthy).abs() > 0.05,
            "fixture defect must be visible before repair"
        );
        stage(&mut h, "Photo");
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
            .click();
        h.step();
        h.try_run_realtime().unwrap();
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Angle")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        h.get_by_label("Thirds guides").click();
        h.step();
        for (angle, size) in [
            (0., egui::vec2(1100., 800.)),
            (0.19, egui::vec2(800., 600.)),
        ] {
            h.update(|e| {
                let draft = &mut e.document.visit.crop_edit.as_mut().unwrap().draft;
                draft.size = [190, 120];
                draft.angle = angle;
            });
            h.set_size(size);
            h.event(egui::Event::PointerGone);
            h.step();
            wait_for(&mut h, |e| {
                e.document
                    .visit
                    .crop_edit
                    .as_ref()
                    .unwrap()
                    .layout
                    .is_some()
                    && !e.working.updating
            });
            sampled_patch(&mut h, &capture, true);
        }
    });

    fn sampled_patch(
        h: &mut Harness<'_, Editor>,
        capture: &RefCell<FloatCapture>,
        healed: bool,
    ) -> f64 {
        // Rendering updates capture after the callback has submitted the selected RenderedView.
        h.render().unwrap();
        let (position, pipeline, target) = h.read(|e| {
            let layout = e
                .document
                .visit
                .crop_edit
                .as_ref()
                .and_then(|edit| edit.layout)
                .unwrap_or_else(|| e.last_layout.unwrap().0);
            let position = layout.screen_point(egui::pos2(160., 96.));
            assert!(
                layout.image.shrink(8.).contains(position)
                    && layout.canvas.shrink(8.).contains(position)
            );
            let ready = e.document.visit.ready.as_ref().unwrap();
            (
                position,
                crate::processing::Pipeline::new(e.input, e.roll, e.photo, ready.proxy.matrix())
                    .unwrap(),
                e.target,
            )
        });
        let output = capture.borrow();
        let actual = output.pixels
            [position.y.floor() as usize * output.size[0] as usize + position.x.floor() as usize];
        if healed {
            let working = pipeline.apply(DVec3::splat(3200. / 4031.)).unwrap().rgb;
            let cpu = crate::rendering::CpuRenderer::new(target).unwrap();
            let linear = target.linear_to_srgb() * cpu.render(&[working]).unwrap()[0].as_dvec3();
            for c in 0..3 {
                let expected = if linear[c] <= 0.0031308 {
                    linear[c] * 12.92
                } else {
                    1.055 * linear[c].powf(1. / 2.4) - 0.055
                };
                assert!(
                    (f64::from(actual[c]) - expected).abs() < 0.004,
                    "{actual:?} vs {expected}"
                );
            }
        }
        f64::from(actual[0])
    }
}

#[test]
fn crop_pixels_follow_straightening_orientation_resize_and_clipped_navigation() {
    for (turns, reflected, digital) in [
        (crate::geometry::QuarterTurns::Zero, false, true),
        (crate::geometry::QuarterTurns::One, true, true),
        (crate::geometry::QuarterTurns::One, true, false),
    ] {
        editor_harness(|mut harness, capture| {
            harness.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
            wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
            harness.update(|editor| {
                assert!(editor.document.visit.ready.is_some(), "{}", editor.status);
                if digital {
                    editor.input = crate::processing::InputSettings::Digital;
                } else {
                    editor.set_dmin(Some(DVec3::splat(3000. / 4031.)));
                    editor.set_dmax(Some(DVec3::splat(128. / 4031.)));
                    editor.set_gamma(0.75);
                }
                editor.source = false;
                editor.roll.exposure = 0.75;
                editor.photo.wb = Balance {
                    warmth: 0.1,
                    tint: -0.05,
                };
                editor.photo_orientation.turns = turns;
                editor.roll_orientation.horizontal = reflected;
                editor.sync_document();
            });
            stage(&mut harness, "Photo");
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
                .click();
            harness.step();
            harness.try_run_realtime().unwrap();
            harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Angle")
                .scroll_to_me();
            harness.try_run_realtime().unwrap();
            harness.get_by_label("Thirds guides").click();
            harness.step();
            assert_crop_patch_pixels(&mut harness, &capture);

            harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Angle")
                .click();
            harness.step();
            assert!(harness.ctx.text_edit_focused());
            harness.event(egui::Event::Text("23".into()));
            harness.event(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            harness.step();
            harness.read(|editor| {
                assert!(
                    (editor
                        .document
                        .visit
                        .crop_edit
                        .as_ref()
                        .unwrap()
                        .draft
                        .angle
                        .abs()
                        .to_degrees()
                        - 23.)
                        .abs()
                        < 1e-6
                );
            });
            assert_crop_patch_pixels(&mut harness, &capture);

            let before = harness.read(|editor| {
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
                &mut harness,
                before.image.min,
                before.image.min + egui::vec2(20., 15.),
            );
            harness.read(|editor| {
                assert_ne!(
                    editor.document.visit.crop_edit.as_ref().unwrap().draft,
                    before.composition.crop()
                )
            });
            assert_crop_patch_pixels(&mut harness, &capture);

            // Explicit zoom/pan is viewport fixture setup; the rendered callback must clip,
            // not squeeze the whole texture into the visible portion of its rectangle.
            harness.update(|editor| {
                let layout = editor
                    .document
                    .visit
                    .crop_edit
                    .as_ref()
                    .unwrap()
                    .layout
                    .unwrap();
                let density = 1.;
                editor
                    .viewport
                    .zoom_at(layout, density, layout.scale * 1.8, layout.image.center());
            });
            harness.step();
            harness.update(|editor| {
                let layout = editor
                    .document
                    .visit
                    .crop_edit
                    .as_ref()
                    .unwrap()
                    .layout
                    .unwrap();
                editor.viewport.pan(layout, 1., egui::vec2(65., -35.));
            });
            harness.step();
            assert_crop_patch_pixels(&mut harness, &capture);
            harness.get_by_label("Cancel").click();
            harness.step();
            harness.read(|editor| assert!(editor.crop.is_none()));
        });
    }
}
