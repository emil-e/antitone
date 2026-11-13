use super::*;

#[test]
fn linear_gestures_inverse_map_composition_and_allow_off_image_geometry() {
    let area = Rect {
        x: 120,
        y: 73,
        width: 2300,
        height: 1900,
    };
    for rotated in [false, true] {
        let mut roll = RollOrientation::default();
        let mut photo = PhotoOrientation::default();
        if rotated {
            roll.turn_clockwise();
            photo.flips.horizontal = true;
        }
        let crop = Crop::from_region(area).straightened(area, 0.13).unwrap();
        let composition = Composition::with_crop(area, roll, photo, Some(crop)).unwrap();
        let layout = Viewport::default().layout_composition(
            egui::Rect::from_min_size(egui::pos2(20., 30.), egui::vec2(800., 600.)),
            composition,
            1.,
        );
        let start = egui::pos2(layout.image.left() - 40., layout.image.top() - 35.);
        let end = layout.image.center();
        let drag = LinearDrag {
            handle: Handle::New,
            point: source_point(layout, start),
            mask: None,
            layout,
        };
        let mask = drag.update(end);
        let AdjustmentLayerCoverage::Linear { start: a, end: b } = mask else {
            unreachable!()
        };
        assert!((screen_point(layout, a) - end).length() < 0.001);
        assert!((screen_point(layout, b) - start).length() < 0.001);
        assert_eq!(mask.coverage(source_point(layout, start)).unwrap(), 1.);
        assert_eq!(mask.coverage(source_point(layout, end)).unwrap(), 0.);
        assert!(matches!(handle_at(Some(mask), layout, start), Handle::End));
        assert!(matches!(handle_at(Some(mask), layout, end), Handle::Start));
        let middle = start + (end - start) * 0.5;
        assert!(matches!(
            handle_at(Some(mask), layout, middle),
            Handle::Body
        ));
        let move_drag = LinearDrag {
            handle: Handle::Body,
            point: source_point(layout, middle),
            mask: Some(mask),
            layout,
        };
        let moved = move_drag.update(middle + egui::vec2(35., -20.));
        let AdjustmentLayerCoverage::Linear { start, end } = moved else {
            unreachable!()
        };
        let before = DVec2::from_array(b.0) - DVec2::from_array(a.0);
        let after = DVec2::from_array(end.0) - DVec2::from_array(start.0);
        assert!(
            (before - after).length() < 1e-9,
            "translation must preserve width and angle"
        );
    }
}

#[test]
fn linear_paint_reverses_effective_endpoints_and_arrow_with_invert_and_coverage_is_ui_only() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.tab = 1;
        let id = editor
            .document
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .id;
        editor.select_adjustment_layer(adjustment_layers::Target::Layer(id));
        let layout = editor.document.visit.canvas.last_layout.unwrap().0;
        let context = egui::Context::default();
        for invert in [false, true] {
            editor
                .edit_adjustment_layers(|layers| {
                    layers.layers[0].adjustment.invert = invert;
                    Ok(())
                })
                .unwrap();
            let mut output = context.run_ui(Default::default(), |ui| editor.paint_mask(ui, layout));
            output.textures_delta.clear();
            let circles = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Circle(circle) if circle.radius == 4. => Some(circle.fill),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                circles,
                if invert {
                    vec![egui::Color32::WHITE, egui::Color32::BLACK]
                } else {
                    vec![egui::Color32::BLACK, egui::Color32::WHITE]
                }
            );
            let arrow = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Path(path) if path.points.len() == 3 => Some(&path.points),
                    _ => None,
                })
                .unwrap();
            let AdjustmentLayerCoverage::Linear { start, end } =
                editor.selected_mask().unwrap().mask
            else {
                unreachable!()
            };
            let a = screen_point(layout, start);
            let b = screen_point(layout, end);
            let expected_tip =
                a + (b - a) * 0.5 + (b - a).normalized() * if invert { -7. } else { 7. };
            assert_eq!(arrow[0], expected_tip);
        }
        let input = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        editor.document.visit.adjustment_layers.show_coverage = true;
        let mut output = context.run_ui(Default::default(), |ui| editor.paint_mask(ui, layout));
        output.textures_delta.clear();
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Mesh(_)))
        );
        assert_eq!(
            input.clone(),
            editor
                .positive_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings()
                )
                .unwrap()
                .clone()
        );
    });
}

#[test]
fn cancelling_gradient_interaction_discards_preview_without_resetting_the_selected_layer() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        editor.tab = 1;
        let id = editor
            .document
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .layers[0]
            .id;
        editor.select_adjustment_layer(adjustment_layers::Target::Layer(id));
        editor.document.visit.adjustment_layers.show_coverage = true;
        editor.document.visit.working_view.compare = true;
        let before = editor.document.state.project.clone();
        let input = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        editor.begin_linear_mask(Some(id));
        editor.document.visit.masks.gesture_mut().unwrap().mask =
            Some(AdjustmentLayerCoverage::Linear {
                start: SourcePoint([122., 75.]),
                end: SourcePoint([140., 88.]),
            });
        assert!(editor.mask_preview_source().is_some());
        editor.finish_adjustment_layer_interaction();
        assert!(editor.document.visit.masks.gesture().is_none());
        assert!(editor.document.visit.working_view.compare);
        assert_eq!(
            editor.adjustment_layer_target(),
            adjustment_layers::Target::Layer(id)
        );
        assert!(editor.mask_handles_available());
        assert!(editor.mask_help().is_some());
        assert!(editor.mask_preview_source().is_none());
        assert_eq!(editor.document.state.project, before);
        assert_eq!(
            input.clone(),
            editor
                .view_processing(
                    editor.document.project.input,
                    editor.document.project.roll,
                    editor.photo_settings()
                )
                .unwrap()
                .clone()
        );
        let context = egui::Context::default();
        let layout = editor.document.visit.canvas.last_layout.unwrap().0;
        let mut output = context.run_ui(Default::default(), |ui| editor.paint_mask(ui, layout));
        output.textures_delta.clear();
        assert!(
            !output.shapes.is_empty(),
            "the selected gradient retains its handles after an individual gesture ends"
        );
    });
}

#[test]
fn inverse_mapping_keeps_subpixel_precision_at_large_source_origins() {
    let area = Rect {
        x: 16_770_000,
        y: 16_770_000,
        width: 128,
        height: 96,
    };
    let composition = Composition::new(
        area,
        RollOrientation::default(),
        PhotoOrientation::default(),
    )
    .unwrap();
    let layout = Viewport::default().layout_composition(
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
        composition,
        1.,
    );
    let point = SourcePoint([area.x as f64 + 8.125, area.y as f64 + 12.375]);
    assert_eq!(source_point(layout, screen_point(layout, point)), point);
}

#[test]
fn creation_draft_keeps_ids_navigation_and_comparison_until_accepted() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let before = editor.document.state.project.clone();
        editor.document.visit.working_view.compare = true;
        editor.begin_linear_mask(None);
        assert!(editor.creating_mask());
        assert!(editor.document.visit.masks.gesture().is_none());
        assert!(editor.current_tool().unwrap().is::<CreationTool>());
        let draft = editor.mask_creation().unwrap();
        assert!(!draft.borrow().valid());
        draft.borrow_mut().mask = Some(AdjustmentLayerCoverage::Linear {
            start: SourcePoint([121., 74.]),
            end: SourcePoint([141., 90.]),
        });
        assert!(draft.borrow().valid());
        assert_eq!(editor.document.state.project, before);
        assert!(editor.document.visit.working_view.compare);
        editor.document.visit.canvas.viewport.zoom = Some(3.);
        let viewport = editor.document.visit.canvas.viewport;
        let retired = Rc::downgrade(&draft);
        drop(draft);
        editor.end_tool();
        assert!(retired.upgrade().is_none());
        assert!(!editor.masking());
        assert_eq!(editor.document.visit.canvas.viewport, viewport);
        assert!(editor.document.visit.working_view.compare);
        assert_eq!(editor.document.state.project, before);
    });
}

#[test]
fn accepted_mask_edit_retains_ids_numeric_controls_and_immutable_snapshots() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let source = editor.document.visit.current_source.unwrap();
        let records = editor
            .document
            .project
            .source(source)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap()
            .clone();
        let id = records.layers[0].id;
        let input = editor
            .positive_processing(
                editor.document.project.input,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .unwrap();
        let snapshot = input.grade().unwrap().snapshot().clone();
        editor.begin_linear_mask(Some(id));
        assert!(editor.document.visit.masks.gesture().is_some());
        assert!(editor.roll_navigation_available());
        let mask = AdjustmentLayerCoverage::Linear {
            start: SourcePoint([100., 70.]),
            end: SourcePoint([149., 99.]),
        };
        editor.document.visit.masks.gesture_mut().unwrap().mask = Some(mask);
        editor.finish_mask(true);
        assert!(editor.document.visit.masks.gesture().is_none());
        let accepted = editor
            .document
            .project
            .source(source)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap();
        assert_eq!(accepted.next_id, records.next_id);
        assert_eq!(accepted.layers[0].id, id);
        assert_eq!(
            accepted.layers[0].adjustment.photo,
            records.layers[0].adjustment.photo
        );
        assert_eq!(accepted.layers[0].adjustment.mask, mask);
        assert_eq!(
            editor.adjustment_layer_target(),
            adjustment_layers::Target::Layer(id)
        );
        assert_eq!(input.grade().unwrap().snapshot(), &snapshot);
        let before = editor.document.state.project.clone();
        editor.begin_linear_mask(Some(id));
        editor.document.visit.masks.gesture_mut().unwrap().mask =
            Some(AdjustmentLayerCoverage::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([0., 0.]),
            });
        editor.finish_mask(true);
        assert!(
            editor.document.visit.masks.gesture().is_none(),
            "rejected refinements return to the accepted selected mask"
        );
        assert_eq!(editor.document.state.project, before);
        editor.finish_mask(false);
        assert_eq!(editor.document.state.project, before);
        editor.document.visit.working_view.compare = true;
        editor.begin_linear_mask(Some(id));
        assert!(editor.document.visit.working_view.compare);
        assert!(!editor.comparing_original());
        let changed_mask = AdjustmentLayerCoverage::Linear {
            start: SourcePoint([100., 70.]),
            end: SourcePoint([145., 95.]),
        };
        editor.document.visit.masks.gesture_mut().unwrap().mask = Some(changed_mask);
        editor.document.project.roll.exposure += 0.1;
        let mut expected = editor.document.project.clone();
        expected
            .source_mut(source)
            .unwrap()
            .adjustment_layers
            .as_mut()
            .unwrap()
            .layers[0]
            .adjustment
            .mask = changed_mask;
        assert!(editor.mask_preview_source().is_some());
        editor.finish_mask(true);
        assert!(editor.document.visit.masks.gesture().is_none());
        assert!(
            editor.document.visit.working_view.compare,
            "refinement preserves the ordinary comparison preference"
        );
        assert_eq!(editor.document.state.project, expected);
    });
}
