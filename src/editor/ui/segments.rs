use super::Icon;
use egui::{Response, Stroke, Ui, Vec2};

#[derive(Clone, Copy)]
pub(in crate::editor) enum SegmentLayout {
    Compact,
    Fill,
}

/// One mutually exclusive selection. Compact segments use their names as tooltips;
/// fill-width segments show those names beside their icons.
pub(in crate::editor) struct Segments<'a, T> {
    pub choices: &'a [(T, Icon, &'a str)],
    pub height: f32,
    pub layout: SegmentLayout,
}

const INSET: f32 = 2.;
const ICON_SIZE: f32 = 16.;
const LABEL_GAP: f32 = 5.;

impl<T: Copy + PartialEq> Segments<'_, T> {
    pub fn width(&self, ui: &Ui) -> f32 {
        match self.layout {
            SegmentLayout::Compact => self.choices.len() as f32 * self.height + 2. * INSET,
            SegmentLayout::Fill => ui.available_width(),
        }
    }

    pub fn show(&self, ui: &mut Ui, current: &mut T) -> Response {
        let (track, mut group) = ui.allocate_exact_size(
            egui::vec2(self.width(ui), self.height),
            egui::Sense::hover(),
        );
        let visual = ui.visuals();
        ui.painter().rect(
            track,
            4.,
            visual.panel_fill,
            visual.widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let content = track.shrink(INSET);
        let previous = *current;
        let count = self.choices.len();
        for (index, &(value, icon, name)) in self.choices.iter().enumerate() {
            let edge = |index| content.left() + content.width() * index as f32 / count as f32;
            let rect = egui::Rect::from_min_max(
                egui::pos2(edge(index), content.top()),
                egui::pos2(edge(index + 1), content.bottom()),
            );
            let response = ui.interact(rect, group.id.with(index), egui::Sense::click());
            let selected = value == previous;
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::RadioButton,
                    response.enabled(),
                    selected,
                    name,
                )
            });
            if response.clicked() {
                *current = value;
            }

            let state = ui.style().interact(&response);
            if selected || response.hovered() || response.has_focus() {
                let fill = if selected {
                    visual.panel_fill.lerp_to_gamma(visual.text_color(), 0.20)
                } else {
                    state.weak_bg_fill
                };
                let stroke = if response.has_focus() {
                    state.bg_stroke
                } else {
                    Stroke::NONE
                };
                ui.painter()
                    .rect(rect, 3., fill, stroke, egui::StrokeKind::Inside);
            }

            let icon_stroke = Stroke::new(
                1.3,
                if selected {
                    visual.selection.stroke.color
                } else {
                    state.fg_stroke.color
                },
            );
            match self.layout {
                SegmentLayout::Compact => {
                    icon.paint(ui, rect, icon_stroke);
                    response.on_hover_text(name);
                }
                SegmentLayout::Fill => {
                    let label = egui::WidgetText::from(name).into_galley(
                        ui,
                        Some(egui::TextWrapMode::Truncate),
                        (rect.width() - ICON_SIZE - LABEL_GAP - 4.).max(0.),
                        egui::TextStyle::Button,
                    );
                    let width = ICON_SIZE + LABEL_GAP + label.size().x;
                    let left = rect.center().x - width * 0.5;
                    let icon_center = egui::pos2(left + ICON_SIZE * 0.5, rect.center().y);
                    icon.paint(
                        ui,
                        egui::Rect::from_center_size(icon_center, Vec2::splat(ICON_SIZE)),
                        icon_stroke,
                    );
                    let text_pos = egui::pos2(
                        left + ICON_SIZE + LABEL_GAP,
                        rect.center().y - label.size().y * 0.5,
                    );
                    let truncated = label.elided;
                    ui.painter().galley_with_override_text_color(
                        text_pos,
                        label,
                        state.fg_stroke.color,
                    );
                    if truncated {
                        response.on_hover_text(name);
                    }
                }
            }
        }
        if *current != previous {
            group.mark_changed();
            ui.ctx().request_repaint();
        }
        group
    }
}

#[cfg(test)]
mod tests;
