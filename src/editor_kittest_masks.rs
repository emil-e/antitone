use super::*;
use crate::local_adjustments::GradientMask;
#[path = "editor_kittest_radial_masks.rs"]
mod radial;
#[path = "editor_kittest_local_workflows.rs"]
mod workflows;

fn click(harness: &mut Harness<'_, Editor>, label: &str) {
    if ["Sky", "Mask overlay"].contains(&label) {
        adjustments::open_drawer(harness);
    }
    harness.get_by_label(label).scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label(label).click();
    harness.step();
    harness.try_run_realtime().unwrap();
}

fn arm(harness: &mut Harness<'_, Editor>, radial: bool) {
    adjustments::open_drawer(harness);
    click(harness, if radial { "Add radial" } else { "Add linear" });
}

fn accepted_mask(editor: &Editor) -> GradientMask {
    editor
        .document
        .project
        .source(editor.current_source.unwrap())
        .unwrap()
        .local_adjustments
        .as_ref()
        .unwrap()
        .adjustments[0]
        .adjustment
        .mask
}

pub(super) fn pointer(harness: &mut Harness<'_, Editor>, point: egui::Pos2, pressed: bool) {
    harness.hover_at(point);
    harness.event(egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();
}

pub(super) fn hold_drag(harness: &mut Harness<'_, Editor>, start: egui::Pos2, end: egui::Pos2) {
    pointer(harness, start, true);
    harness.hover_at(end);
    harness.step();
}

fn endpoint(editor: &Editor) -> egui::Pos2 {
    let GradientMask::Linear { end, .. } = accepted_mask(editor) else {
        unreachable!()
    };
    let layout = editor.last_layout.unwrap().0;
    layout.screen_frame_point(layout.composition.source_to_frame(end))
}

#[test]
fn drawing_accepts_on_release_and_exposure_is_immediately_available_without_a_mask_session() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        let before = harness.read(|editor| editor.document.project.clone());
        arm(&mut harness, false);
        let image = harness.read(|editor| editor.last_layout.unwrap().0.image);
        pointer(&mut harness, image.center(), true);
        pointer(&mut harness, image.center(), false);
        harness.read(|editor| {
            assert!(
                editor.document.visit.mask_edit.is_some(),
                "a rejected zero-size creation remains retryable"
            )
        });
        harness.read(|editor| assert_eq!(editor.document.project, before));
        hold_drag(
            &mut harness,
            image.center() - egui::vec2(80., 40.),
            image.center() + egui::vec2(80., 40.),
        );
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness
            .read(|editor| assert!(editor.document.visit.mask_edit.as_ref().unwrap().dragging()));
        harness.key_press(egui::Key::Escape);
        harness.step();
        pointer(&mut harness, image.center() + egui::vec2(80., 40.), false);
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness.read(|editor| assert!(editor.document.visit.mask_edit.is_none()));
        arm(&mut harness, false);
        let image = harness.read(|editor| editor.last_layout.unwrap().0.image);
        drag(
            &mut harness,
            image.center() - egui::vec2(80., 40.),
            image.center() + egui::vec2(80., 40.),
        );
        harness.read(|editor| {
            assert!(
                editor.document.visit.mask_edit.is_none(),
                "{}",
                editor.status
            )
        });
        harness.try_run_realtime().unwrap();
        let locals = harness.read(adjustments::locals);
        assert_eq!(locals.next_id, 2);
        assert_eq!(locals.adjustments.len(), 1);
        assert_eq!(
            locals.adjustments[0].adjustment.photo,
            PhotoSettings::default()
        );
        harness.read(|editor| {
            assert_eq!(
                editor.adjustment_target(),
                super::super::adjustments::Target::Local(locals.adjustments[0].id)
            )
        });
        assert!(
            !harness
                .get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                .accesskit_node()
                .is_disabled()
        );
        workflow::enter_numeric(&mut harness, "Exposure", "0.25");
        harness.read(|editor| {
            assert_eq!(
                adjustments::locals(editor).adjustments[0]
                    .adjustment
                    .photo
                    .exposure,
                0.25
            )
        });
        harness.read(|editor| {
            editor
                .validate_document_candidate(&editor.document)
                .unwrap()
        });
    });
}

#[test]
fn selected_linear_handles_preview_commit_and_cancel_individual_gestures_through_rotated_crops() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        harness.update(|editor| {
            let area = editor.document.visit.ready.as_ref().unwrap().area;
            editor.roll_orientation.turn_clockwise();
            editor.photo_orientation.flips.horizontal = true;
            editor.crop = Some(Crop::from_region(area).straightened(area, 0.13).unwrap());
            editor.sync_document();
        });
        harness.step();
        arm(&mut harness, false);
        let image = harness.read(|editor| editor.last_layout.unwrap().0.image);
        drag(
            &mut harness,
            image.center() - egui::vec2(30., 50.),
            image.center() + egui::vec2(30., 50.),
        );
        workflow::enter_numeric(&mut harness, "Exposure", "0.25");
        for (name, size, width) in [
            ("normal", egui::vec2(1100., 800.), 315.),
            ("narrow", egui::vec2(800., 600.), 270.),
        ] {
            set_layout(&mut harness, size, width);
            workflow::wait_settled(&mut harness);
            let before = harness.read(|editor| editor.document.project.clone());
            let (input, snapshot, layout, start) = harness.read(|editor| {
                let input = editor
                    .positive_processing(editor.input, editor.roll, editor.photo)
                    .unwrap();
                let snapshot = input.grade().unwrap().snapshot().clone();
                let layout = editor.last_layout.unwrap().0;
                let start = endpoint(editor);
                (input, snapshot, layout, start)
            });
            let end = egui::pos2(layout.canvas.right() - 8., layout.canvas.bottom() - 12.);
            hold_drag(&mut harness, start, end);
            harness.read(|editor| {
                assert!(editor.document.visit.mask_edit.as_ref().unwrap().dragging())
            });
            harness.read(|editor| assert_eq!(editor.document.project, before));
            let (preview, preview_end, original_end) = harness.read(|editor| {
                let preview = editor
                    .view_processing(editor.input, editor.roll, editor.photo)
                    .unwrap();
                assert_ne!(preview.signature(), input.signature());
                assert_eq!(input.grade().unwrap().snapshot(), &snapshot);
                let GradientMask::Linear {
                    end: preview_end, ..
                } = preview.grade().unwrap().snapshot().locals()[0].mask
                else {
                    unreachable!()
                };
                let GradientMask::Linear {
                    end: original_end, ..
                } = accepted_mask(editor)
                else {
                    unreachable!()
                };
                (preview, preview_end, original_end)
            });
            let pressed = layout
                .composition
                .frame_to_source(layout.frame_point(start));
            let released = layout.composition.frame_to_source(layout.frame_point(end));
            let expected = crate::geometry::SourcePoint(
                (glam::DVec2::from_array(original_end.0) + glam::DVec2::from_array(released.0)
                    - glam::DVec2::from_array(pressed.0))
                .to_array(),
            );
            assert!(
                (glam::DVec2::from_array(preview_end.0) - glam::DVec2::from_array(expected.0))
                    .length()
                    < 1e-8
            );
            pointer(&mut harness, end, false);
            harness.read(|editor| assert!(editor.document.visit.mask_edit.is_none()));
            harness.read(|editor| {
                assert_eq!(
                    accepted_mask(editor),
                    preview.grade().unwrap().snapshot().locals()[0].mask
                )
            });
            harness.read(|editor| assert_eq!(adjustments::locals(editor).next_id, 2));
            harness.read(|editor| {
                assert_eq!(
                    adjustments::locals(editor).adjustments[0]
                        .adjustment
                        .photo
                        .exposure,
                    0.25
                )
            });
            let accepted = harness.read(|editor| editor.document.project.clone());
            let start = harness.read(endpoint);
            hold_drag(&mut harness, start, start - egui::vec2(20., 20.));
            harness.key_press(egui::Key::Escape);
            harness.step();
            pointer(&mut harness, start - egui::vec2(20., 20.), false);
            harness.read(|editor| assert_eq!(editor.document.project, accepted));
            let layout = harness.read(|editor| editor.last_layout.unwrap().0);
            let outside = layout.canvas.min + egui::vec2(8., 20.);
            drag(&mut harness, outside, outside + egui::vec2(5., 70.));
            harness.read(|editor| assert!(editor.document.visit.mask_edit.is_none()));
            harness.read(|editor| {
                assert_eq!(
                    editor.document.project, accepted,
                    "outside drags never redraw an existing mask"
                )
            });
            click(&mut harness, "Mask overlay");
            workflow::wait_settled(&mut harness);
            save_capture(
                &harness.render().unwrap(),
                &format!("local-linear-direct-{name}-offscreen"),
            );
            click(&mut harness, "Mask overlay");
        }
    });
}

#[test]
fn creation_popup_escape_and_dust_keep_canvas_ownership_unambiguous() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        let before = harness.read(|editor| editor.document.project.clone());
        arm(&mut harness, false);
        click(&mut harness, "Viewer help");
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.read(|editor| {
            assert!(
                editor.document.visit.mask_edit.is_some(),
                "popup owns the first Escape"
            )
        });
        harness.key_press(egui::Key::Escape);
        harness.step();
        harness.read(|editor| assert!(editor.document.visit.mask_edit.is_none()));
        harness.read(|editor| assert_eq!(editor.document.project, before));
        click(&mut harness, "Dust repair");
        harness.read(|editor| assert!(editor.document.dust.cleanup));
        assert!(
            harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Adjustments")
                .accesskit_node()
                .is_disabled()
        );
        harness.read(|editor| assert!(!editor.mask_handles_available()));
        click(&mut harness, "Done");
        harness.read(|editor| assert!(!editor.document.dust.cleanup));
        assert!(
            !harness
                .get_by_role_and_label(egui::accesskit::Role::Button, "Adjustments")
                .accesskit_node()
                .is_disabled()
        );
    });
}

#[test]
fn armed_creation_suspends_outside_photo_and_resumes_without_consuming_an_id() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for radial in [false, true] {
            let before = h.read(|editor| editor.document.project.clone());
            arm(&mut h, radial);
            for name in ["Roll", "Rendering"] {
                stage(&mut h, name);
                if name == "Rendering" {
                    assert!(
                        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Look")
                            .accesskit_node()
                            .is_disabled()
                    );
                }
                let image = h.read(|editor| editor.last_layout.unwrap().0.image);
                drag(
                    &mut h,
                    image.center() - egui::vec2(30., 30.),
                    image.center() + egui::vec2(30., 30.),
                );
                h.read(|editor| assert_eq!(editor.document.project, before));
                h.read(|editor| {
                    let edit = editor.document.visit.mask_edit.as_ref().unwrap();
                    assert!(edit.creating() && !edit.dragging());
                    assert!(editor.mask_preview_source().is_none());
                });
            }
            stage(&mut h, "Photo");
            let image = h.read(|editor| editor.last_layout.unwrap().0.image);
            drag(
                &mut h,
                image.center() - egui::vec2(30., 30.),
                image.center() + egui::vec2(30., 30.),
            );
            h.read(|editor| {
                assert!(
                    editor.document.visit.mask_edit.is_none(),
                    "{}",
                    editor.status
                )
            });
            let locals = h.read(adjustments::locals);
            assert_eq!(locals.adjustments.len(), if radial { 2 } else { 1 });
            assert_eq!(locals.next_id, if radial { 3 } else { 2 });
        }
    });
}

#[test]
fn captured_masks_block_fit_zoom_and_middle_pan_until_release() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for radial in [false, true] {
            arm(&mut h, radial);
            let image = h.read(|editor| editor.last_layout.unwrap().0.image);
            drag(
                &mut h,
                image.center() - egui::vec2(30., 30.),
                image.center() + egui::vec2(30., 30.),
            );
            let layout = h.read(|editor| editor.last_layout.unwrap().0);
            h.update(|editor| {
                editor
                    .viewport
                    .zoom_at(layout, 1., 2., layout.canvas.center())
            });
            h.step();
            let layout = h.read(|editor| editor.last_layout.unwrap().0);
            let mask = h.read(|editor| {
                adjustments::locals(editor)
                    .adjustments
                    .last()
                    .unwrap()
                    .adjustment
                    .mask
            });
            let point = match mask {
                GradientMask::Linear { end, .. } => end,
                GradientMask::Radial { center, .. } => center,
            };
            let start = layout.screen_frame_point(layout.composition.source_to_frame(point));
            let end = start + egui::vec2(20., 15.);
            let viewport = h.read(|editor| editor.viewport);
            hold_drag(&mut h, start, end);
            h.read(|editor| assert!(editor.document.visit.mask_edit.as_ref().unwrap().dragging()));
            h.key_press(egui::Key::Z);
            h.step();
            h.event(egui::Event::Zoom(1.25));
            h.step();
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: end,
                    button: egui::PointerButton::Middle,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                h.hover_at(end + egui::vec2(10., 5.));
                h.step();
            }
            h.read(|editor| assert_eq!(editor.viewport, viewport));
            h.read(|editor| assert_eq!(editor.last_layout.unwrap().0.image, layout.image));
            let preview = h.read(|editor| {
                editor
                    .mask_preview_source()
                    .unwrap()
                    .local_adjustments
                    .clone()
            });
            pointer(&mut h, end + egui::vec2(10., 5.), false);
            h.read(|editor| {
                assert!(
                    editor.document.visit.mask_edit.is_none(),
                    "{}",
                    editor.status
                )
            });
            h.read(|editor| {
                assert_eq!(
                    editor
                        .document
                        .project
                        .source(editor.current_source.unwrap())
                        .unwrap()
                        .local_adjustments,
                    preview
                )
            });
            h.key_press(egui::Key::Z);
            h.step();
            h.read(|editor| assert!(editor.viewport.zoom.is_none()));
        }
    });
}

#[test]
fn fullscreen_suspension_cannot_accept_an_unseen_mask_release() {
    workflow::calibrated_workflow(|mut harness, _, _directory| {
        arm(&mut harness, false);
        let image = harness.read(|editor| editor.last_layout.unwrap().0.image);
        let start = image.center() - egui::vec2(40., 40.);
        let end = image.center() + egui::vec2(40., 40.);
        hold_drag(&mut harness, start, end);
        let before = harness.read(|editor| editor.document.project.clone());
        harness.key_press(egui::Key::F);
        harness.step();
        harness.read(|editor| assert!(editor.fullscreen.is_none()));
        let context = harness.ctx.clone();
        harness.update(|editor| editor.toggle_fullscreen(&context));
        harness.step();
        harness.read(|editor| assert!(editor.fullscreen.is_some()));
        pointer(&mut harness, end + egui::vec2(80., 50.), false);
        harness.read(|editor| assert_eq!(editor.document.project, before));
        harness.key_press(egui::Key::F);
        harness.step();
        harness.update(|editor| host::App::photo_fullscreen_changed(editor, false));
        harness.step();
        harness.read(|editor| assert!(editor.fullscreen.is_none()));
        harness.read(|editor| assert!(editor.document.visit.mask_edit.is_none()));
        harness.read(|editor| assert_eq!(editor.document.project, before));
    });
}
