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
        assert!((screen_point(layout, a) - start).length() < 0.001);
        assert!((screen_point(layout, b) - end).length() < 0.001);
        assert!(matches!(
            handle_at(Some(mask), layout, start),
            Handle::Start
        ));
        assert!(matches!(handle_at(Some(mask), layout, end), Handle::End));
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
        let layout = editor.document.visit.last_layout.unwrap().0;
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
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
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
            input.signature(),
            editor
                .positive_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings
                )
                .unwrap()
                .signature()
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
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        editor.begin_linear_mask(Some(id));
        editor.document.visit.mask_edit.as_mut().unwrap().mask =
            Some(AdjustmentLayerCoverage::Linear {
                start: SourcePoint([122., 75.]),
                end: SourcePoint([140., 88.]),
            });
        let candidate = editor
            .document
            .visit
            .mask_edit
            .as_ref()
            .unwrap()
            .candidate()
            .unwrap()
            .0;
        editor.validate_source_candidate(&candidate).unwrap();
        editor.document.visit.mask_edit.as_mut().unwrap().preview = Some(candidate);
        assert!(editor.mask_preview_source().is_some());
        editor.finish_adjustment_layer_interaction();
        assert!(editor.document.visit.mask_edit.is_none());
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
            input.signature(),
            editor
                .view_processing(
                    editor.document.input,
                    editor.document.roll,
                    editor.document.visit.adjustment_layer_settings
                )
                .unwrap()
                .signature()
        );
        let context = egui::Context::default();
        let layout = editor.document.visit.last_layout.unwrap().0;
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
fn creation_promotes_one_accepted_id_and_gesture_cancel_keeps_navigation() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let before = editor.document.state.project.clone();
        let mut viewport = editor.document.visit.viewport;
        viewport.zoom = Some(3.);
        editor.document.visit.working_view.compare = true;
        editor.begin_linear_mask(None);
        assert!(!editor.document.visit.working_view.compare);
        assert!(editor.document.visit.mask_edit.is_some());
        assert!(!editor.document.visit.mask_edit.as_ref().unwrap().valid());
        editor.document.visit.mask_edit.as_mut().unwrap().mask =
            Some(AdjustmentLayerCoverage::Linear {
                start: SourcePoint([121., 74.]),
                end: SourcePoint([141., 90.]),
            });
        let (_, first) = editor
            .document
            .visit
            .mask_edit
            .as_ref()
            .unwrap()
            .candidate()
            .unwrap();
        let (_, repeated) = editor
            .document
            .visit
            .mask_edit
            .as_ref()
            .unwrap()
            .candidate()
            .unwrap();
        assert_eq!(first, repeated);
        assert_eq!(editor.document.state.project, before);
        editor.document.visit.viewport.zoom = Some(3.);
        editor.finish_mask(false);
        assert!(editor.document.visit.mask_edit.is_none());
        assert_eq!(editor.document.visit.viewport, viewport);
        assert!(editor.document.visit.working_view.compare);
        assert_eq!(editor.document.state.project, before);
        editor.begin_linear_mask(None);
        editor.document.visit.mask_edit.as_mut().unwrap().mask =
            Some(AdjustmentLayerCoverage::Linear {
                start: SourcePoint([121., 74.]),
                end: SourcePoint([141., 90.]),
            });
        editor.finish_mask(true);
        assert!(
            editor.document.visit.mask_edit.is_none(),
            "{}",
            editor.status
        );
        let source = editor
            .document
            .project
            .source(editor.document.visit.current_source.unwrap())
            .unwrap();
        let layers = source.adjustment_layers.as_ref().unwrap();
        let prior = before
            .source(source.id)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap();
        assert_eq!(layers.next_id, prior.next_id + 1);
        assert_eq!(layers.layers.len(), prior.layers.len() + 1);
        assert_eq!(layers.layers.last().unwrap().id, first);
        assert_eq!(
            editor.adjustment_layer_target(),
            adjustment_layers::Target::Layer(first)
        );
        editor
            .validate_document_candidate(&editor.document)
            .unwrap();
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
                editor.document.input,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .unwrap();
        let snapshot = input.pipeline().unwrap().snapshot().clone();
        editor.begin_linear_mask(Some(id));
        assert!(editor.document.visit.mask_edit.is_some());
        editor.begin_crop();
        editor.preview_picker(Pick::Wb);
        editor.request_analysis(&UiContext::render_only());
        assert!(
            editor.document.visit.crop_edit.is_none()
                && editor.active_pick().is_none()
                && editor.document.analysis_pending.is_none()
        );
        assert!(!editor.roll_edit_available());
        let mask = AdjustmentLayerCoverage::Linear {
            start: SourcePoint([100., 70.]),
            end: SourcePoint([149., 99.]),
        };
        editor.document.visit.mask_edit.as_mut().unwrap().mask = Some(mask);
        editor.finish_mask(true);
        assert!(editor.document.visit.mask_edit.is_none());
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
        assert_eq!(input.pipeline().unwrap().snapshot(), &snapshot);
        let before = editor.document.state.project.clone();
        editor.begin_linear_mask(Some(id));
        editor.document.visit.mask_edit.as_mut().unwrap().mask =
            Some(AdjustmentLayerCoverage::Linear {
                start: SourcePoint([0., 0.]),
                end: SourcePoint([0., 0.]),
            });
        editor.finish_mask(true);
        assert!(
            editor.document.visit.mask_edit.is_none(),
            "rejected refinements return to the accepted selected mask"
        );
        assert_eq!(editor.document.state.project, before);
        editor.finish_mask(false);
        assert_eq!(editor.document.state.project, before);
        editor.document.visit.working_view.compare = true;
        editor.begin_linear_mask(Some(id));
        assert!(!editor.document.visit.working_view.compare);
        editor.document.roll.exposure += 0.1;
        editor.finish_mask(true);
        assert!(
            editor.document.visit.mask_edit.is_none(),
            "changed processing cannot accept a stale mask draft"
        );
        assert!(
            editor.document.visit.working_view.compare,
            "stale rejection restores draft-owned comparison mode"
        );
        assert_eq!(editor.document.state.project, before);
    });
}
