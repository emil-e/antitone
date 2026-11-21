use super::*;

#[test]
fn floating_cancel_retires_only_crop_and_keeps_accepted_state() {
    super::super::tools::calibrated_harness(|mut h| {
        stage(&mut h, "Photo");
        super::super::tools::crop_draft(&mut h);
        let project = h.read(|editor| editor.document.state.project.clone());
        key(&mut h, egui::Key::F);
        reveal(&mut h);
        h.ctx.all_styles_mut(|style| {
            style.interaction.tooltip_delay = 0.;
            style.interaction.show_tooltips_only_when_still = false;
        });
        h.hover_at(h.get_by_label("Apply").rect().center());
        h.step();
        h.step();
        assert!(
            crate::editor::tests::ui_labels(h.output().clone())
                .iter()
                .any(|(label, _)| label == "Apply (Enter)."),
            "hover positive control: {:?}",
            crate::editor::tests::ui_labels(h.output().clone())
        );
        h.hover_at(h.get_by_label("Cancel").rect().center());
        h.step();
        h.step();
        assert!(
            !crate::editor::tests::ui_labels(h.output().clone())
                .iter()
                .any(|(label, _)| label == "Escape"),
            "Escape exits fullscreen, not Cancel"
        );
        h.get_by_label("Cancel").click();
        h.step();
        h.read(|editor| {
            assert!(editor.fullscreen_requested());
            assert!(!editor.cropping());
            assert_eq!(editor.document.state.project, project);
        });
    });
}

#[test]
fn pickers_keep_source_preference_and_remain_interactive_fullscreen() {
    for kind in [PickerCase::Gamma, PickerCase::GammaPivot, PickerCase::Wb] {
        super::super::tools::calibrated_harness(|mut h| {
            h.update(|editor| {
                let crop = Crop::from_region(Rect {
                    x: 48,
                    y: 40,
                    width: 160,
                    height: 112,
                });
                editor
                    .accept_crop(Crop {
                        angle: 0.11,
                        ..crop
                    })
                    .unwrap();
                editor.document.source = true;
            });
            picking::open(&mut h, kind);
            workflow::wait_settled(&mut h);
            let before = h.read(|editor| {
                assert!(editor.canvas_inspection());
                assert!(editor.document.source);
                let painted = editor.canvas_frame.displayed.as_ref().unwrap();
                assert!(painted.view.capture.is_inspection());
                (
                    editor.canvas_frame.layout.unwrap().0.composition,
                    painted.view.clone(),
                )
            });
            key(&mut h, egui::Key::F);
            window_size(&mut h, egui::vec2(1600., 1000.));
            h.step();
            h.render().unwrap();
            h.read(|editor| {
                assert!(editor.canvas_inspection());
                assert!(editor.document.source);
                assert_eq!(editor.canvas_frame.layout.unwrap().0.composition, before.0);
                assert!(
                    editor.canvas_frame.displayed.as_ref().unwrap().view.capture
                        == before.1.capture
                );
            });
            let point = h.read(|editor| {
                editor
                    .canvas_frame
                    .layout
                    .unwrap()
                    .0
                    .screen_point(egui::pos2(128., 96.))
            });
            masks::pointer(&mut h, point, true);
            masks::pointer(&mut h, point, false);
            h.read(|editor| assert!(editor.test_pick_region().is_some()));
            reveal(&mut h);
            assert!(!h.get_by_label("Apply").accesskit_node().is_disabled());
        });
    }
}

#[test]
fn crop_handles_and_floating_actions_edit_the_same_draft_fullscreen() {
    for (size, width, suffix) in [
        (egui::vec2(1100., 800.), 315., "normal"),
        (egui::vec2(800., 600.), 270., "narrow"),
    ] {
        super::super::tools::calibrated_harness(|mut h| {
            set_layout(&mut h, size, width);
            native(&mut h, false);
            stage(&mut h, "Photo");
            let initial = super::super::tools::crop_draft(&mut h);
            let project = h.read(|editor| editor.document.state.project.clone());
            key(&mut h, egui::Key::F);
            native(&mut h, true);
            wait_for(&mut h, |editor| editor.canvas_frame.displayed.is_some());
            reveal(&mut h);
            for label in ["Apply", "Cancel", "Fit crop"] {
                let rect = h.get_by_label(label).rect();
                assert!(h.ctx.content_rect().contains_rect(rect), "{label}");
            }
            let layout = h.read(|editor| editor.document.crop_edit().unwrap().layout.unwrap());
            assert_eq!(layout.composition.crop(), initial);
            let corner = layout.screen_point(egui::pos2(
                initial.coverage().unwrap().corners()[0].0[0] as f32,
                initial.coverage().unwrap().corners()[0].0[1] as f32,
            ));
            drag(&mut h, corner, corner + egui::vec2(24., 18.));
            let edited = h.read(|editor| editor.document.crop_edit().unwrap().draft);
            assert_ne!(edited, initial, "fullscreen handles must change the draft");
            h.read(|editor| assert_eq!(editor.document.state.project, project));
            reveal(&mut h);
            save_capture(&h.render().unwrap(), &format!("fullscreen-crop-{suffix}"));
            key(&mut h, egui::Key::Escape);
            h.update(|editor| editor.fullscreen_changed(false));
            native(&mut h, false);
            h.step();
            h.read(|editor| {
                assert!(editor.fullscreen.is_none());
                assert_eq!(editor.document.crop_edit().unwrap().draft, edited);
                assert_eq!(editor.document.state.project, project);
            });
            key(&mut h, egui::Key::F);
            native(&mut h, true);
            h.step();
            reveal(&mut h);
            h.get_by_label("Apply").click();
            h.step();
            h.read(|editor| {
                assert!(
                    editor.fullscreen.is_some(),
                    "accepting a tool does not exit fullscreen"
                );
                assert!(!editor.cropping());
                assert_eq!(editor.photo_crop(), Some(edited));
            });
        });
    }
}

#[test]
fn dmin_picker_keeps_cropped_positive_composition_anchor_and_selection_fullscreen() {
    super::super::tools::calibrated_harness(|mut h| {
        h.update(|editor| {
            let crop = Crop::from_region(Rect {
                x: 48,
                y: 40,
                width: 160,
                height: 112,
            });
            editor
                .accept_crop(Crop {
                    angle: 0.11,
                    ..crop
                })
                .unwrap();
            editor.document.source = false;
        });
        picking::open(&mut h, PickerCase::Dmin);
        native(&mut h, false);
        let before = h.read(|editor| editor.canvas_frame.layout.unwrap().0);
        assert_eq!(
            before.composition.crop(),
            h.read(|editor| editor.photo_crop().unwrap())
        );
        h.update(|editor| {
            let (layout, density) = editor.canvas_frame.layout.unwrap();
            let size = layout.composition.size();
            let scale = (layout.canvas.width() / size[0] as f32)
                .max(layout.canvas.height() / size[1] as f32)
                * 1.5;
            editor.with_tool_viewport(|viewport| viewport.zoom = Some(scale * density));
        });
        h.step();
        h.update(|editor| {
            let (layout, density) = editor.canvas_frame.layout.unwrap();
            editor.with_tool_viewport(|viewport| {
                viewport.pan(layout, density, egui::vec2(80., -20.))
            });
        });
        h.step();
        let before = h.read(|editor| editor.canvas_frame.layout.unwrap().0);
        assert!(before.image.width() > before.canvas.width());
        assert!(before.image.height() > before.canvas.height());
        let anchor = before.source_point(before.canvas.center());
        assert!(
            anchor.distance(before.source_point(before.image.center())) > 1.,
            "fixture must carry an off-centre source anchor"
        );
        key(&mut h, egui::Key::F);
        native(&mut h, true);
        window_size(&mut h, egui::vec2(1600., 1000.));
        h.step();
        let after = h.read(|editor| editor.canvas_frame.layout.unwrap().0);
        assert_eq!(after.composition, before.composition);
        assert_eq!(after.scale, before.scale);
        assert!(after.source_point(after.canvas.center()).distance(anchor) < 1e-3);
        h.read(|editor| {
            assert!(!editor.canvas_inspection());
            assert!(
                !editor.document.source,
                "fullscreen must not change the normal view preference"
            );
        });
        let point = after.screen_point(egui::pos2(128., 96.));
        assert!(after.canvas.contains(point) && after.image.contains(point));
        masks::pointer(&mut h, point, true);
        masks::pointer(&mut h, point, false);
        let selected = h.read(|editor| editor.test_pick_region().unwrap());
        assert!(selected.exact_rectangle(after.area).is_none());
        let screen = after.screen_coverage(selected);
        assert!(screen.center().distance(point) < 1e-3);
        assert!((screen.width() - 32. * after.scale).abs() < 1e-3);
        assert!((screen.height() - 32. * after.scale).abs() < 1e-3);
        let bounds = selected.bounds(after.area).unwrap();
        assert!(bounds.width * bounds.height > 32 * 32);
        assert!(
            (selected
                .pixels(after.area)
                .unwrap()
                .map(|(_, weight)| weight)
                .sum::<f64>()
                - 1024.)
                .abs()
                < 1e-8
        );
        key(&mut h, egui::Key::Escape);
        h.update(|editor| editor.fullscreen_changed(false));
        native(&mut h, false);
        window_size(&mut h, egui::vec2(1100., 800.));
        h.step();
        h.read(|editor| {
            assert_eq!(editor.test_pick_region(), Some(selected));
            assert!(editor.picking());
            assert_eq!(
                editor.canvas_frame.layout.unwrap().0.composition,
                before.composition
            );
        });
    });
}
