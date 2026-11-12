use super::theme::INSCRIPTION;
use super::*;
use egui::{Context, Pos2, Rect};

#[test]
fn shared_components_inherit_interaction_colours_without_tinting_photographic_surfaces() {
    let mut tool_fills = Vec::new();
    for accent in [INSCRIPTION, egui::Color32::from_rgb(71, 143, 207)] {
        let context = Context::default();
        super::configure_fonts(&context);
        let mut rectangles = None;
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(600., 320.))),
                ..Default::default()
            },
            |ui| {
                configure_branding(ui);
                assert_eq!(ui.visuals().selection.stroke.color, INSCRIPTION);
                assert_eq!(ui.visuals().hyperlink_color, INSCRIPTION);
                // A second accent detects components that hard-code the current brand colour.
                ui.visuals_mut().selection.stroke.color = accent;
                ui.visuals_mut().selection.bg_fill = accent.gamma_multiply(0.25);
                let action = TextAction::new("Apply").primary().show(ui);
                let mut selected = 1;
                super::stage_tabs(ui, &mut selected);
                let tool_body = super::ToolIdentity {
                    icon: super::Icon::Pick,
                    title: "Dmin",
                }
                .inspector(ui, |_| (), |ui, ()| ui.label("Tool details"));
                let layout = super::ContactSheetLayout::new(100., false);
                let (id, cell) = ui.allocate_space(layout.size());
                let card = layout.begin(ui, cell.min, id, true, false, true);
                rectangles = Some((action.rect, card.response.rect, tool_body.rect));
            },
        );
        output.textures_delta.clear();
        let (action, card, tool_body) = rectangles.unwrap();
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.rect == action && rect.fill == accent.gamma_multiply(0.25))));
        let tool_fill = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.rect.contains_rect(tool_body)
                        && rect.fill != egui::Color32::TRANSPARENT =>
                {
                    Some(rect)
                }
                _ => None,
            })
            .min_by(|a, b| a.rect.area().total_cmp(&b.rect.area()))
            .unwrap()
            .fill;
        tool_fills.push(tool_fill);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::LineSegment { stroke, .. } if stroke.color == accent)));
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.rect == card && rect.stroke.color == accent
                && rect.fill.r() == rect.fill.g() && rect.fill.g() == rect.fill.b())));
    }
    assert_ne!(
        tool_fills[0], tool_fills[1],
        "tool emphasis must follow the supplied accent"
    );
}
