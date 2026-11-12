use super::{CONTROL_HEIGHT, instrument_readout, theme::instrument_font};
use egui::{Response, Ui, Vec2};

/// Text-button presentation only; callers retain availability and action dispatch.
#[derive(Clone, Copy)]
pub(in crate::editor) struct TextAction<'a> {
    label: &'a str,
    name: Option<&'a str>,
    help: Option<&'a str>,
    enabled: bool,
    primary: bool,
    readout: bool,
    toolbar_width: Option<f32>,
}

impl<'a> TextAction<'a> {
    pub(in crate::editor) fn new(label: &'a str) -> Self {
        Self {
            label,
            name: None,
            help: None,
            enabled: true,
            primary: false,
            readout: false,
            toolbar_width: None,
        }
    }

    pub(in crate::editor) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(in crate::editor) fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub(in crate::editor) fn readout(mut self) -> Self {
        self.readout = true;
        self
    }

    /// Natural actions keep egui sizing; toolbar actions share the control height.
    pub(in crate::editor) fn toolbar(mut self, min_width: f32) -> Self {
        self.toolbar_width = Some(min_width);
        self
    }

    pub(in crate::editor) fn name(mut self, name: &'a str) -> Self {
        self.name = Some(name);
        self
    }

    pub(in crate::editor) fn help(mut self, help: &'a str) -> Self {
        self.help = Some(help);
        self
    }

    fn min_size(&self) -> Vec2 {
        self.toolbar_width
            .map_or(Vec2::ZERO, |width| egui::vec2(width, CONTROL_HEIGHT))
    }

    fn button(&self, ui: &Ui) -> egui::Button<'a> {
        let text: egui::WidgetText = if self.readout {
            let style = ui.style().button_style(
                &egui::widget_style::Classes::default(),
                egui::widget_style::WidgetState::Inactive,
            );
            instrument_readout(self.label, style.text_style.font_id.size).into()
        } else {
            self.label.into()
        };
        let mut button = egui::Button::new(text).min_size(self.min_size());
        if self.primary {
            button = button.fill(ui.visuals().selection.bg_fill);
        }
        button
    }

    /// Single-line reservation without allocating a widget or advancing caller IDs.
    pub(in crate::editor) fn width(&self, ui: &Ui) -> f32 {
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
            if self.readout {
                instrument_font(style.text_style.font_id.size)
            } else {
                style.text_style.font_id
            },
            style.text_style.color,
        );
        (text.size().x + margin).max(self.min_size().x)
    }

    pub(in crate::editor) fn show(self, ui: &mut Ui) -> Response {
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
mod tests;
