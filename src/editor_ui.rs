use egui::{Color32, Response, Ui, Vec2};

pub(super) const CONTROL_HEIGHT: f32 = 28.;
pub(super) const VIEWER_BAR_HEIGHT: f32 = 40.;
pub(super) const VIEWER_ITEM_GAP: f32 = 4.;

pub(super) fn active_tool_fill(ui: &Ui) -> Color32 {
    ui.visuals().selection.bg_fill.linear_multiply(0.15)
}

/// Text-button presentation only; callers retain availability and action dispatch.
#[derive(Clone, Copy)]
pub(super) struct TextAction<'a> {
    label: &'a str,
    name: Option<&'a str>,
    help: Option<&'a str>,
    enabled: bool,
    primary: bool,
    toolbar_width: Option<f32>,
}

impl<'a> TextAction<'a> {
    pub(super) fn new(label: &'a str) -> Self {
        Self {
            label,
            name: None,
            help: None,
            enabled: true,
            primary: false,
            toolbar_width: None,
        }
    }

    pub(super) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(super) fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// Natural actions keep egui sizing; toolbar actions share the control height.
    pub(super) fn toolbar(mut self, min_width: f32) -> Self {
        self.toolbar_width = Some(min_width);
        self
    }

    pub(super) fn name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    pub(super) fn help(mut self, help: &'a str) -> Self {
        self.help = Some(help);
        self
    }

    fn min_size(&self) -> Vec2 {
        self.toolbar_width
            .map_or(Vec2::ZERO, |width| egui::vec2(width, CONTROL_HEIGHT))
    }

    fn button(&self, ui: &Ui) -> egui::Button<'a> {
        let mut button = egui::Button::new(self.label).min_size(self.min_size());
        if self.primary {
            button = button.fill(ui.visuals().selection.bg_fill);
        }
        button
    }

    /// Single-line reservation without allocating a widget or advancing caller IDs.
    pub(super) fn width(&self, ui: &Ui) -> f32 {
        let style = ui.style().button_style(
            &egui::widget_style::Classes::default(),
            egui::widget_style::WidgetState::Inactive,
        );
        // Button resolves its font and complete frame margins through button_style.
        let margin = if ui.visuals().button_frame {
            style.frame.total_margin().sum().x
        } else {
            0.
        };
        let text = ui.painter().layout_no_wrap(
            self.label.into(),
            style.text_style.font_id,
            style.text_style.color,
        );
        (text.size().x + margin).max(self.min_size().x)
    }

    pub(super) fn show(self, ui: &mut Ui) -> Response {
        let response = ui.add_enabled(self.enabled, self.button(ui));
        if let Some(name) = self.name {
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, response.enabled(), name)
            });
        }
        if let Some(help) = self.help {
            response.on_hover_text(help)
        } else {
            response
        }
    }
}

#[cfg(test)]
#[path = "editor_ui_tests.rs"]
mod tests;
