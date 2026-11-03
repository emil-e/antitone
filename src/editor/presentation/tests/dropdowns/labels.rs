use super::*;
use crate::editor::presentation::labelled_inspector_dropdown;

#[test]
fn selectors_inside_a_group_keep_visible_labels_and_accessible_values() {
    for name in ["Negative", "Paper", "LUT"] {
        let context = Context::default();
        let render = || {
            frame(&context, 270., true, vec![], |ui| {
                ui.vertical(|ui| {
                    labelled_inspector_dropdown(ui, "choice", name, "Selected material", |ui| {
                        ui.label("Alternative")
                    })
                })
                .inner
            })
        };
        render();
        let (response, output) = render();
        let labels = output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(text) if text.galley.job.text == name => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 1);
        assert!(labels[0].pos.y < response.response.rect.top());
        let node = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, n)| n.role() == egui::accesskit::Role::ComboBox)
            .unwrap()
            .1;
        assert_eq!(node.label(), Some(name));
        assert_eq!(node.value(), Some("Selected material"));
    }
}
