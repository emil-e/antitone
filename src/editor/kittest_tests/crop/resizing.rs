use super::*;

#[test]
fn resize_modifiers_and_continuous_camera_compose_with_navigation_and_apply() {
    editor_harness(|mut h, capture| {
        h.update(|editor| {
            editor.document.project.input = crate::processing::InputSettings::Digital;
        });
        h.enter(|editor, cx| editor.add_raws(vec![fixture("patches.dng")], cx));
        wait_for(&mut h, |editor| {
            !editor.interaction_blocked() && editor.document.visit.canvas.ready.is_some()
        });
        stage(&mut h, "Photo");
        for modifiers in [
            egui::Modifiers::NONE,
            egui::Modifiers::ALT,
            egui::Modifiers::SHIFT,
            egui::Modifiers {
                alt: true,
                shift: true,
                ..Default::default()
            },
        ] {
            h.get_by_role_and_label(egui::accesskit::Role::Button, "Crop & straighten")
                .click();
            h.step();
            h.try_run_realtime().unwrap();
            let fitted = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            assert!(fitted.image.center().distance(fitted.canvas.center()) < 0.01);
            let accepted = h.read(|editor| editor.document.state.project.clone());
            h.hover_at(fitted.canvas.center());
            h.event(egui::Event::Zoom(1.02));
            h.step();
            let zoomed = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            assert!(
                zoomed.scale > fitted.scale * 1.01,
                "zoom input must reach the crop viewport"
            );
            let pan_start = fitted.canvas.center();
            let pan_end = pan_start + egui::vec2(5., -4.);
            h.hover_at(pan_start);
            h.event(egui::Event::PointerButton {
                pos: pan_start,
                button: egui::PointerButton::Middle,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            h.hover_at(pan_end);
            h.step();
            h.event(egui::Event::PointerButton {
                pos: pan_end,
                button: egui::PointerButton::Middle,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            let before = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            let offset = before.image.center() - before.canvas.center();
            assert!(
                offset.length() > 2.,
                "pan input must reach the crop viewport"
            );
            let start = before.image.max;
            assert!(
                before.canvas.contains(start),
                "the grabbed handle must be reachable"
            );
            let end = start - egui::vec2(45., 16.);
            h.event(egui::Event::ModifiersChanged(modifiers));
            h.hover_at(start);
            h.event(egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            });
            h.step();
            h.hover_at(end);
            h.step();
            let draft = h.read(|editor| editor.document.crop_edit().unwrap().draft);
            assert_ne!(draft.size, before.composition.crop().size);
            // Geometry stages the next paint; hold the pointer while the live camera catches up.
            h.step();
            let after = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            assert_eq!(after.scale, before.scale, "resizing must not zoom");
            if !modifiers.shift {
                let error = after.image.max - end;
                assert!(error.x.abs() <= before.scale * 0.26 + 0.01);
                assert!(error.y.abs() <= before.scale * 0.26 + 0.01);
            }
            assert!((after.image.center() - after.canvas.center() - offset).length() < 0.01);
            if modifiers.alt {
                for axis in 0..2 {
                    assert!(
                        (draft.center[axis] - before.composition.crop().center[axis]).abs() <= 0.51
                    );
                }
            }
            let size = after.composition.size().map(f64::from);
            let initial = before.composition.size().map(f64::from);
            let ratio_error = (size[0] - size[1] * initial[0] / initial[1]).abs();
            if modifiers.shift {
                assert!(ratio_error <= 1.5);
            } else {
                assert!(ratio_error > 2., "free-resize control must change aspect");
            }
            h.step();
            h.read(|editor| {
                assert_eq!(
                    editor.document.crop_edit().unwrap().draft,
                    draft,
                    "holding must not drift"
                );
                assert_eq!(editor.document.state.project, accepted);
            });
            h.event(egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            });
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.step();
            h.step();
            let released = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            assert!(released.image.min.distance(after.image.min) < 0.01);
            assert!(released.image.max.distance(after.image.max) < 0.01);
            if std::env::var_os("ANTITONE_UI_CAPTURE").is_some() {
                h.event(egui::Event::PointerGone);
                h.step();
                let png = h.render().unwrap();
                save_capture(
                    &png,
                    &format!(
                        "crop-resize-alt-{}-shift-{}",
                        modifiers.alt, modifiers.shift
                    ),
                );
            }
            if modifiers.alt && modifiers.shift {
                h.update(|editor| editor.document.crop_edit_mut().unwrap().guides = false);
                assert_crop_patch_pixels(&mut h, &capture);
            }
            h.get_by_label("Apply").click();
            h.step();
            h.read(|editor| {
                assert_eq!(editor.photo_crop(), Some(draft));
                assert!(editor.document.crop_edit().is_none());
            });
        }
    });
}
