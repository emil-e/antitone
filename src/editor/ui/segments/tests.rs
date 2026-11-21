use super::*;
use crate::editor::ui::test_support::{frame, named_rect, pointer};
use egui::Context;

#[test]
fn segmented_layouts_keep_one_selection_within_a_shared_frame() {
    for layout in [SegmentLayout::Compact, SegmentLayout::Fill] {
        let context = Context::default();
        let mut selected = 0;
        let mut render = |enabled, events| {
            frame(&context, 200., enabled, events, |ui| {
                let selector = Segments {
                    choices: &[
                        (0, Icon::Vectorscope, "First"),
                        (1, Icon::Waveform, "Second"),
                    ],
                    height: 24.,
                    layout,
                };
                let width = selector.width(ui);
                let rect = selector.show(ui, &mut selected).rect;
                assert_eq!(rect.size(), egui::vec2(width, selector.height));
                (
                    rect,
                    ui.visuals()
                        .panel_fill
                        .lerp_to_gamma(ui.visuals().text_color(), 0.20),
                )
            })
        };
        let filled = |output: &egui::FullOutput, color, label| {
            let center = named_rect(output, label).center();
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Rect(rect)
                if rect.fill == color && rect.rect.contains(center))
            })
        };

        let ((group, color), initial) = render(true, vec![]);
        let first = named_rect(&initial, "First");
        let second = named_rect(&initial, "Second");
        assert!(group.contains_rect(first.union(second)));
        let rendered_name = initial.shapes.iter().any(|shape| {
            matches!(
                &shape.shape, egui::Shape::Text(text) if text.galley.job.text == "First"
            )
        });
        assert_eq!(rendered_name, matches!(layout, SegmentLayout::Fill));
        assert_eq!(first.right(), second.left());
        assert!(filled(&initial, color, "First"));
        assert!(!filled(&initial, color, "Second"));

        for _ in 0..2 {
            render(true, pointer(second.center(), true));
            render(true, pointer(second.center(), false));
            let (_, output) = render(true, vec![egui::Event::PointerGone]);
            assert!(!filled(&output, color, "First"));
            assert!(filled(&output, color, "Second"));
        }

        render(false, pointer(first.center(), true));
        render(false, pointer(first.center(), false));
        drop(render);
        assert_eq!(selected, 1, "a disabled choice cannot change the selection");
    }
}
