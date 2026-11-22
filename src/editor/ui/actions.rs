use super::{
    CONTROL_HEIGHT, instrument_readout,
    theme::{DESTRUCTIVE_FILL, instrument_font},
};
use egui::{Response, Ui, Vec2};
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Emphasis {
    Neutral,
    Primary,
    Destructive,
}

/// Text-button presentation only; callers retain availability and action dispatch.
#[derive(Clone, Copy)]
pub(in crate::editor) struct TextAction<'a> {
    label: &'a str,
    name: Option<&'a str>,
    help: Option<&'a str>,
    enabled: bool,
    emphasis: Emphasis,
    readout: bool,
    min_size: Vec2,
    horizontal_padding: Option<f32>,
}

impl<'a> TextAction<'a> {
    pub(in crate::editor) fn new(label: &'a str) -> Self {
        Self {
            label,
            name: None,
            help: None,
            enabled: true,
            emphasis: Emphasis::Neutral,
            readout: false,
            min_size: Vec2::ZERO,
            horizontal_padding: None,
        }
    }

    pub(in crate::editor) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(in crate::editor) fn primary(mut self) -> Self {
        self.emphasis = Emphasis::Primary;
        self
    }

    pub(super) fn destructive(mut self) -> Self {
        self.emphasis = Emphasis::Destructive;
        self
    }

    pub(in crate::editor) fn readout(mut self) -> Self {
        self.readout = true;
        self
    }

    /// Natural actions keep egui sizing; toolbar actions share the control height.
    pub(in crate::editor) fn toolbar(self, min_width: f32) -> Self {
        self.minimum_size(min_width, CONTROL_HEIGHT)
    }

    pub(super) fn minimum_size(mut self, width: f32, height: f32) -> Self {
        self.min_size = egui::vec2(width, height);
        self
    }

    pub(super) fn horizontal_padding(mut self, padding: f32) -> Self {
        self.horizontal_padding = Some(padding);
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
        let button = egui::Button::new(text).min_size(self.min_size);
        match self.emphasis {
            Emphasis::Neutral => button,
            Emphasis::Primary => button.fill(ui.visuals().selection.bg_fill),
            Emphasis::Destructive => button.fill(DESTRUCTIVE_FILL),
        }
    }

    /// Single-line reservation without allocating a widget or advancing caller IDs.
    pub(in crate::editor) fn width(&self, ui: &Ui) -> f32 {
        let padded_style = self.padded_style(ui);
        let style = padded_style.as_ref().unwrap_or(ui.style()).button_style(
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
        (text.size().x + margin).max(self.min_size.x)
    }

    fn padded_style(&self, ui: &Ui) -> Option<Arc<egui::Style>> {
        let padding = self.horizontal_padding?;
        let mut style = ui.style().clone();
        Arc::make_mut(&mut style).spacing.button_padding.x = padding;
        Some(style)
    }

    pub(in crate::editor) fn show(self, ui: &mut Ui) -> Response {
        let padded_style = self.padded_style(ui);
        let previous_style = padded_style.map(|style| {
            let previous = ui.style().clone();
            ui.set_style(style);
            previous
        });
        let response = ui.add_enabled(self.enabled, self.button(ui));
        if let Some(style) = previous_style {
            ui.set_style(style);
        }
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
