use egui::{Color32, Id, Rect, Response, Ui, Vec2};
use std::sync::Arc;

/// Accepted contact-sheet geometry; callers retain the tray, scrolling and virtualization.
#[derive(Clone, Copy)]
pub struct ContactSheetLayout {
    size: Vec2,
    padding: f32,
    image_height: f32,
    caption_height: f32,
    caption_gap: f32,
}

impl ContactSheetLayout {
    pub fn new(available_height: f32, compact: bool) -> Self {
        let padding = if compact { 3. } else { 5. };
        let caption_height = if compact { 14. } else { 18. };
        let caption_gap = if compact { 2. } else { 4. };
        let image_height =
            (available_height - 6. - 2. * padding - caption_height - caption_gap).max(8.);
        let image_width = (image_height * 1.5 + 2. * padding).max(80.);
        Self {
            size: egui::vec2(
                image_width,
                image_height + 2. * padding + caption_height + caption_gap,
            ),
            padding,
            image_height,
            caption_height,
            caption_gap,
        }
    }

    pub fn size(self) -> Vec2 {
        self.size
    }

    pub fn pitch(self) -> f32 {
        self.size.x + 12.
    }

    /// Paint the frame/well without allocating layout or introducing an ID scope.
    pub fn begin(
        self,
        ui: &mut Ui,
        origin: egui::Pos2,
        id: Id,
        selected: bool,
        active: bool,
        enabled: bool,
    ) -> ContactSheetCard {
        let cell = Rect::from_min_size(origin, self.size);
        let response = ui.add_enabled(enabled, |ui: &mut Ui| {
            ui.interact(cell, id, egui::Sense::click())
        });
        let hovered = response.hovered();
        let fill = Color32::from_gray(if active {
            48
        } else if selected {
            30
        } else if hovered {
            28
        } else {
            24
        });
        let stroke = if selected || response.has_focus() {
            egui::Stroke::new(1.5, ui.visuals().selection.stroke.color)
        } else {
            egui::Stroke::new(1., Color32::from_gray(if hovered { 65 } else { 42 }))
        };
        let text_color = Color32::from_gray(if selected {
            225
        } else if hovered {
            195
        } else {
            165
        });
        ui.painter()
            .rect(cell, 4., fill, stroke, egui::StrokeKind::Inside);
        let image_rect = Rect::from_min_size(
            cell.min + Vec2::splat(self.padding),
            egui::vec2(self.size.x - 2. * self.padding, self.image_height),
        );
        ui.painter()
            .rect_filled(image_rect, 0., Color32::from_gray(14));
        let caption_rect = Rect::from_min_size(
            egui::pos2(image_rect.left(), image_rect.bottom() + self.caption_gap),
            egui::vec2(image_rect.width(), self.caption_height),
        );
        ContactSheetCard {
            response,
            image_rect,
            caption_rect,
            text_color,
            selected,
            active,
        }
    }
}

/// Paint caller-owned image content in `image_rect`, then the badge/placeholder and caption.
pub struct ContactSheetCard {
    pub response: Response,
    pub image_rect: Rect,
    caption_rect: Rect,
    text_color: Color32,
    selected: bool,
    active: bool,
}

impl ContactSheetCard {
    fn placeholder(&self, ui: &Ui, text: &str) {
        ui.painter().text(
            self.image_rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            egui::FontId::proportional((self.image_rect.height() / 1.2).min(16.)),
            self.text_color,
        );
    }

    pub fn loading(&self, ui: &Ui) {
        self.placeholder(ui, "…");
    }

    pub fn error(&self, ui: &Ui) {
        self.placeholder(ui, "!");
    }

    pub fn camera_badge(&self, ui: &Ui) {
        let image_rect = self.image_rect;
        let painter = ui
            .painter()
            .with_clip_rect(image_rect.intersect(ui.clip_rect()));
        let font = egui::FontId::proportional(((image_rect.height() - 2.) / 1.2).min(10.));
        let galley = painter.layout_no_wrap(
            if image_rect.height() < 24. {
                "Camera"
            } else {
                "Camera preview"
            }
            .into(),
            font,
            Color32::WHITE,
        );
        let size = (galley.size() + egui::vec2(4., 2.)).min(image_rect.size());
        let label = Rect::from_min_size(image_rect.left_bottom() - egui::vec2(0., size.y), size);
        painter.rect_filled(label, 2., Color32::from_black_alpha(190));
        painter.galley(label.center() - galley.size() * 0.5, galley, Color32::WHITE);
    }

    pub fn caption(&self, ui: &Ui, name: &str) {
        self.response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                self.response.enabled(),
                self.selected,
                name,
            )
        });
        if self.active {
            ui.ctx().accesskit_node_builder(self.response.id, |node| {
                node.set_description("Active photo")
            });
        }
        let galley = caption_name(ui, name, self.caption_rect.width(), self.text_color);
        ui.painter()
            .with_clip_rect(self.caption_rect.intersect(ui.clip_rect()))
            .galley(
                self.caption_rect.center() - galley.size() * 0.5,
                galley,
                self.text_color,
            );
    }
}

fn caption_name(ui: &Ui, name: &str, width: f32, color: Color32) -> Arc<egui::Galley> {
    let layout = |text: String| {
        ui.painter()
            .layout_no_wrap(text, egui::FontId::proportional(11.), color)
    };
    let full = layout(name.to_owned());
    if full.size().x <= width {
        return full;
    }
    let characters: Vec<char> = name.chars().collect();
    let mut lower = 0;
    let mut upper = characters.len();
    let mut fitted = layout("…".into());
    while lower + 1 < upper {
        let keep = (lower + upper) / 2;
        // Prioritize the trailing frame identifier and extension at compact strip heights.
        let suffix = keep.min(9).max(keep / 2);
        let prefix = keep - suffix;
        let text = characters[..prefix]
            .iter()
            .copied()
            .chain(std::iter::once('…'))
            .chain(characters[characters.len() - suffix..].iter().copied())
            .collect();
        let candidate = layout(text);
        if candidate.size().x <= width {
            lower = keep;
            fitted = candidate;
        } else {
            upper = keep;
        }
    }
    fitted
}

#[cfg(test)]
#[path = "editor_ui_card_tests.rs"]
mod tests;
