use super::*;

#[test]
fn explicit_edit_is_only_on_masked_rows_and_survives_collapse_and_grading() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        h.update(|editor| {
            let ids = adjustment_layers::install_layers(editor);
            let source = editor.document.visit.current_source.unwrap();
            let area = editor
                .document
                .project
                .source(source)
                .unwrap()
                .development
                .active_area;
            let layers = editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap();
            layers.layers[0].adjustment.mask = AdjustmentLayerCoverage::Linear {
                start: crate::geometry::SourcePoint([
                    f64::from(area[0]) + f64::from(area[2]) * 0.3,
                    f64::from(area[1]) + f64::from(area[3]) * 0.3,
                ]),
                end: crate::geometry::SourcePoint([
                    f64::from(area[0]) + f64::from(area[2]) * 0.7,
                    f64::from(area[1]) + f64::from(area[3]) * 0.7,
                ]),
            };
            layers
                .add(
                    "Whole photo".into(),
                    crate::adjustment_layers::AdjustmentLayer::global(
                        AdjustmentLayerSettings::default(),
                    ),
                )
                .unwrap();
            editor.select_adjustment_layer(super::super::super::adjustment_layers::Target::Layer(
                ids[0],
            ));
        });
        h.step();
        adjustment_layers::show_layers(&mut h);
        assert!(
            h.query_by_role_and_label(egui::accesskit::Role::Button, "Edit Whole photo")
                .is_none()
        );
        assert!(
            !h.get_by_role_and_label(egui::accesskit::Role::Button, "Edit Foreground")
                .accesskit_node()
                .is_disabled()
        );
        h.get_by_role_and_label(egui::accesskit::Role::Button, "Edit Sky")
            .scroll_to_me();
        h.try_run_realtime().unwrap();
        save_capture(&h.render().unwrap(), "mask-edit-layer-actions");
        adjustment_layers::click(&mut h, "Edit Sky");
        h.read(|editor| assert!(editor.masking() && !editor.creating_mask()));
        adjustment_layers::click(&mut h, "Layers");
        workflow::enter_numeric(&mut h, "Exposure", "0.35");
        h.read(|editor| {
            assert!(editor.masking());
            assert_eq!(editor.photo_settings().exposure, 0.35);
        });
        save_capture(&h.render().unwrap(), "mask-edit-collapsed-grading");
        let before = h.read(|editor| editor.document.project.clone());
        let start = h.read(endpoint);
        drag(&mut h, start, start + egui::vec2(20., -15.));
        h.read(|editor| {
            assert!(editor.masking() && !editor.mask_dragging());
            assert_eq!(editor.photo_settings().exposure, 0.35);
            assert_ne!(editor.document.project, before);
        });
        let accepted = h.read(|editor| editor.document.project.clone());
        click(&mut h, "Done");
        h.read(|editor| {
            assert!(!editor.masking());
            assert_eq!(editor.document.project, accepted);
        });
    });
}

#[test]
fn row_icon_actions_preserve_selection_and_show_active_and_bypassed_states() {
    for (layout, size, width) in [
        ("normal", egui::vec2(1100., 800.), 315.),
        ("narrow", egui::vec2(800., 600.), 270.),
    ] {
        workflow::calibrated_workflow(|mut h, _, _directory| {
            set_layout(&mut h, size, width);
            h.update(|editor| {
                adjustment_layers::install_layers(editor);
                let source = editor.document.visit.current_source.unwrap();
                editor
                    .document
                    .project
                    .source_mut(source)
                    .unwrap()
                    .adjustment_layers
                    .as_mut()
                    .unwrap()
                    .add(
                        "Whole photo".into(),
                        crate::adjustment_layers::AdjustmentLayer::global(
                            AdjustmentLayerSettings::default(),
                        ),
                    )
                    .unwrap();
            });
            h.step();
            adjustment_layers::show_layers(&mut h);
            adjustment_layers::click(&mut h, "Sky");
            let before_rect = h.get_by_label("Sky").rect();
            adjustment_layers::click(&mut h, "Edit Sky");
            let owner = h.read(|editor| Rc::downgrade(&editor.mask_edit().unwrap()));
            assert_eq!(
                h.get_by_label("Sky").rect(),
                before_rect,
                "activation does not scroll or resize rows"
            );
            assert!(
                h.query_by_label("Linear gradient").is_none(),
                "refinement has no inspector drawer"
            );
            assert_eq!(
                h.get_by_label("Edit Sky").accesskit_node().is_selected(),
                Some(true)
            );
            adjustment_layers::click(&mut h, "Enable Foreground");
            h.read(|editor| {
                assert!(Rc::ptr_eq(
                    &owner.upgrade().unwrap(),
                    &editor.mask_edit().unwrap()
                ));
                assert!(
                    !adjustment_layers::layers(editor).layers[1]
                        .adjustment
                        .enabled
                );
                assert_eq!(
                    editor.photo_settings(),
                    adjustment_layers::layers(editor).layers[0].adjustment.photo
                );
            });
            assert_eq!(
                h.get_by_label("Sky").accesskit_node().is_selected(),
                Some(true)
            );
            assert_eq!(
                h.get_by_label("Enable Foreground")
                    .accesskit_node()
                    .toggled(),
                Some(egui::accesskit::Toggled::False)
            );
            let global = h.get_by_label("Enable Whole photo").rect();
            let power = h.get_by_label("Enable Sky").rect();
            let edit = h.get_by_label("Edit Sky").rect();
            assert_eq!(global.x_range(), power.x_range());
            assert!(edit.right() <= power.left());
            assert!(h.query_by_label("Edit Whole photo").is_none());
            h.event(egui::Event::PointerGone);
            h.step();
            save_capture(&h.render().unwrap(), &format!("mask-row-icons-{layout}"));
            let accepted = h.read(|editor| editor.document.project.clone());
            let history =
                h.read(|editor| editor.history_label(crate::editor::history::Direction::Undo));
            adjustment_layers::click(&mut h, "Edit Sky");
            h.read(|editor| {
                assert!(!editor.masking());
                assert_eq!(editor.document.project, accepted);
                assert_eq!(
                    editor.history_label(crate::editor::history::Direction::Undo),
                    history
                );
            });
            assert!(owner.upgrade().is_none());
            adjustment_layers::click(&mut h, "Edit Foreground");
            h.read(|editor| assert!(editor.masking()));
            assert!(
                h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                    .accesskit_node()
                    .is_disabled()
            );
            adjustment_layers::click(&mut h, "Enable Foreground");
            assert!(
                !h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "Exposure")
                    .accesskit_node()
                    .is_disabled()
            );
            click(&mut h, "Done");
        });
    }
}

#[test]
fn a_new_mask_remains_in_the_same_editor_for_grading_and_refinement() {
    workflow::calibrated_workflow(|mut h, _, _directory| {
        for radial in [false, true] {
            arm(&mut h, radial);
            let owner = h.read(|editor| Rc::downgrade(&editor.mask_edit().unwrap()));
            let image = h.read(|editor| editor.active_canvas().last_layout.unwrap().0.image);
            let before = h.read(adjustment_layers::layers);
            drag(
                &mut h,
                image.center() - egui::vec2(35., 30.),
                image.center() + egui::vec2(35., 30.),
            );
            h.read(|editor| {
                assert!(editor.masking() && !editor.creating_mask() && !editor.mask_dragging());
                assert!(Rc::ptr_eq(
                    &owner.upgrade().unwrap(),
                    &editor.mask_edit().unwrap()
                ));
                assert_eq!(
                    adjustment_layers::layers(editor).layers.len(),
                    before.layers.len() + 1
                );
            });
            workflow::enter_numeric(&mut h, "Exposure", "0.25");
            let accepted = h.read(|editor| {
                assert!(editor.masking());
                assert_eq!(editor.photo_settings().exposure, 0.25);
                editor.document.project.clone()
            });
            click(&mut h, "Done");
            h.read(|editor| assert_eq!(editor.document.project, accepted));
            assert!(
                owner.upgrade().is_none(),
                "Done retires the same tool that created the layer"
            );
        }
    });
}
