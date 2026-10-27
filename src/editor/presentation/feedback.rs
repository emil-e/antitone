use super::{Icon, TextAction, dialogs, hierarchy};
use egui::{Context, Id, Ui};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "The shared family supports information; this migration wires warnings and errors only."
        )
    )]
    Information,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum StatusLevel {
    Activity,
    Warning,
    Error,
}

/// Supplied contextual conditions; controllers own their scope and retirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextualStatus {
    pub level: StatusLevel,
    pub reason: String,
}

impl ContextualStatus {
    pub fn new(level: StatusLevel, reason: impl Into<String>) -> Self {
        Self {
            level,
            reason: reason.into(),
        }
    }

    pub fn combine(self, other: Self) -> Self {
        let level = self.level.max(other.level);
        let reason = if self.reason == other.reason {
            self.reason
        } else if other.level > self.level {
            format!("{}\n{}", other.reason, self.reason)
        } else {
            format!("{}\n{}", self.reason, other.reason)
        };
        Self { level, reason }
    }
}

pub(super) fn status_icon(
    ui: &Ui,
    response: egui::Response,
    name: &str,
    level: StatusLevel,
    reason: &str,
) -> egui::Response {
    let rect = response.rect;
    match level {
        StatusLevel::Activity => Icon::Pending.paint(
            ui,
            rect,
            egui::Stroke::new(1.5, ui.visuals().weak_text_color()),
        ),
        StatusLevel::Warning => {
            Icon::Warning.paint(ui, rect, egui::Stroke::new(1.5, ui.visuals().warn_fg_color))
        }
        StatusLevel::Error => Icon::Error.paint(
            ui,
            rect,
            egui::Stroke::new(1.5, ui.visuals().error_fg_color),
        ),
    }
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Image, response.enabled(), name)
    });
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_description(reason));
    response.on_hover_text(reason)
}

pub(super) fn warning_icon(
    ui: &Ui,
    response: egui::Response,
    name: &str,
    reason: &str,
) -> egui::Response {
    status_icon(ui, response, name, StatusLevel::Warning, reason)
}

pub(in crate::editor) fn notice_heading(ui: &mut Ui, severity: Severity, title: &str) {
    let (label, icon, color) = match severity {
        Severity::Information => (
            "Information",
            Icon::Information,
            ui.visuals().selection.stroke.color,
        ),
        Severity::Warning => ("Warning", Icon::Warning, ui.visuals().warn_fg_color),
        Severity::Error => ("Error", Icon::Error, ui.visuals().error_fg_color),
    };
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(20.), egui::Sense::hover());
        icon.paint(ui, rect, egui::Stroke::new(1.5, color));
        ui.label(egui::RichText::new(label).strong().color(color));
    });
    ui.heading(egui::RichText::new(title).strong());
}

/// Supplied message presentation only; acknowledgement and resolution belong to the caller.
pub struct FeedbackDialog<'a> {
    pub severity: Severity,
    pub title: &'a str,
    pub explanation: &'a str,
    pub details: &'a str,
}

impl FeedbackDialog<'_> {
    pub fn show(self, context: &Context, id: Id, escape_available: bool) -> bool {
        let response = dialogs::modal(context, id, |ui| {
            notice_heading(ui, self.severity, self.title);
            ui.label(self.explanation);
            hierarchy::dialog_disclosure(ui, "feedback-details", "Details", |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("feedback-details-scroll")
                    .max_height((context.content_rect().height() * 0.25).min(160.))
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(self.details).wrap().selectable(true));
                    });
            });
            ui.add_space(4.);
            let acknowledge = ui
                .horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        TextAction::new("OK").primary().show(ui)
                    })
                    .inner
                })
                .inner;
            dialogs::focus_default(ui, &acknowledge);
            acknowledge.clicked()
        });
        response.inner || dialogs::escape_requested(&response, escape_available)
    }
}

#[cfg(test)]
mod tests;
