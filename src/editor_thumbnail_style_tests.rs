fn cards(output: &egui::FullOutput) -> Vec<&egui::epaint::RectShape> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape
                && rect.corner_radius == egui::CornerRadius::same(4)
                && rect.rect.width() >= 64.
                && rect.stroke.width > 0.
            {
                Some(rect)
            } else {
                None
            }
        })
        .collect()
}

#[test]
fn card_frames_and_captions_fit_compact_and_normal_strips_without_changing_preferences() {
    let (mut editor, _, _commands) = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.document = super::tests::fixture();
    editor.current_source = Some(editor.document.project.order[0]);
    editor.busy = true;
    let before = editor.document.project.clone();
    for height in [52., 80., 140., 240.] {
        editor.preferences.filmstrip_height = height;
        for size in [egui::vec2(700., 600.), egui::vec2(1100., 800.)] {
            let context = egui::Context::default();
            for _ in 0..3 {
                let mut output = context.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| editor.filmstrip(ui),
                );
                output.textures_delta.clear();
                let cards = cards(&output);
                assert!(!cards.is_empty());
                let panel = egui::containers::panel::PanelState::load(
                    &context,
                    egui::Id::new("thumbnail-filmstrip"),
                )
                .unwrap()
                .outer_rect;
                for card in &cards {
                    assert!(
                        card.rect.top() >= panel.top() && card.rect.bottom() <= panel.bottom(),
                        "{height}pt / {size:?} / panel={panel:?} / card={:?}",
                        card.rect
                    );
                }
                assert_eq!(
                    cards[0].stroke.color,
                    context
                        .style_of(egui::Theme::Dark)
                        .visuals
                        .selection
                        .stroke
                        .color
                );
                assert_eq!(cards[0].stroke.width, 1.5);
                assert_eq!(cards[0].fill, egui::Color32::from_gray(48));
                let captions: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape
                            && text.galley.job.sections[0].format.font_id.size == 11.
                        {
                            Some(text)
                        } else {
                            None
                        }
                    })
                    .collect();
                assert_eq!(captions.len(), cards.len());
                for (caption, card) in captions.iter().zip(cards.iter()) {
                    let bounds = caption.galley.rect.translate(caption.pos.to_vec2());
                    assert_eq!(caption.galley.rows.len(), 1);
                    assert!(card.rect.expand(0.5).contains_rect(bounds));
                    assert!((bounds.center().x - card.rect.center().x).abs() < 0.5);
                }
                assert_eq!(editor.preferences.filmstrip_height, height);
                assert_eq!(editor.document.project, before);
            }
        }
    }
}

#[test]
fn hover_is_neutral_and_distinct_from_selection_without_changing_cell_geometry() {
    let (mut editor, _, _commands) = crate::editor::tests::fixture();
    crate::editor::tests::add_ready_fixture(&mut editor);
    editor.document = super::tests::fixture();
    editor.current_source = Some(editor.document.project.order[0]);
    editor.busy = false;
    let context = egui::Context::default();
    let mut frame = |pointer: Option<egui::Pos2>| {
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 800.),
                )),
                events: pointer.into_iter().map(egui::Event::PointerMoved).collect(),
                ..Default::default()
            },
            |ui| editor.filmstrip(ui),
        );
        output.textures_delta.clear();
        output
    };
    let idle = frame(None);
    let point = cards(&idle)[1].rect.center();
    let hover = frame(Some(point));
    let idle = cards(&idle);
    let hover = cards(&hover);
    assert_eq!(idle[1].rect, hover[1].rect);
    assert_eq!(hover[1].fill, egui::Color32::from_gray(28));
    assert_eq!(hover[1].stroke.color, egui::Color32::from_gray(65));
    assert_ne!(hover[1].stroke, hover[0].stroke);
    assert_eq!(hover[0].fill, egui::Color32::from_gray(48));
}
