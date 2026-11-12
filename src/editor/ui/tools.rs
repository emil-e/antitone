use super::{
    CONTROL_HEIGHT, INSPECTOR_ROW_HEIGHT, INSPECTOR_SECTION_GAP, TextAction, feedback,
    icons::{self, Icon},
    instrument_label,
};
use egui::{Color32, Response, Ui, Vec2};

pub(in crate::editor) fn active_tool_fill(ui: &Ui) -> Color32 {
    ui.visuals().selection.bg_fill.linear_multiply(0.15)
}

const TOOL_ICON_SIZE: f32 = 16.;
const TOOL_FRAME_MARGIN: i8 = 6;
const TOOL_ACTION_SPACE: f32 = 8.;

/// Fixed stage-level home for immediate image commands and canvas-tool launchers.
pub(in crate::editor) struct ToolShelf;

impl ToolShelf {
    pub(in crate::editor) fn show<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
        ui.push_id("tool-shelf", |ui| {
            // A little air between the stage tabs and their tools; the first section's own
            // separator supplies the separation below.
            ui.add_space(INSPECTOR_SECTION_GAP);
            ui.spacing_mut().item_spacing.x = 4.;
            ui.spacing_mut().interact_size.y = INSPECTOR_ROW_HEIGHT;
            icons::row_at(ui, INSPECTOR_ROW_HEIGHT, contents).inner
        })
        .inner
    }
}

#[derive(Clone, Copy)]
pub(in crate::editor) struct ToolButton<'a> {
    icon: Icon,
    name: &'a str,
    help: &'a str,
    enabled: bool,
    active: bool,
    changed: bool,
}

impl<'a> ToolButton<'a> {
    pub(in crate::editor) fn command(icon: Icon, name: &'a str, help: &'a str) -> Self {
        Self {
            icon,
            name,
            help,
            enabled: true,
            active: false,
            changed: false,
        }
    }

    pub(in crate::editor) fn tool(icon: Icon, name: &'a str, help: &'a str, active: bool) -> Self {
        Self {
            active,
            ..Self::command(icon, name, help)
        }
    }

    pub(in crate::editor) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(in crate::editor) fn changed(mut self, changed: bool) -> Self {
        self.changed = changed;
        self
    }

    pub(in crate::editor) fn show(self, ui: &mut Ui) -> Response {
        // Shelves inherit the inspector row height so the row they were given actually fits.
        let response = icons::button_at(
            ui,
            self.icon,
            self.enabled,
            self.name,
            self.help,
            ui.spacing().interact_size.y,
        );
        if self.active {
            ui.painter().rect_stroke(
                response.rect.shrink(1.),
                4.,
                egui::Stroke::new(1., ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        } else if self.changed {
            ui.painter().circle_filled(
                response.rect.right_top() + egui::vec2(-4., 4.),
                2.5,
                ui.visuals().selection.stroke.color,
            );
        }
        if self.active || self.changed {
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_selected(self.active);
                if self.changed {
                    node.set_description("Modified");
                }
            });
        }
        response
    }
}

#[derive(Clone, Copy)]
pub(in crate::editor) struct ToolIdentity<'a> {
    pub(in crate::editor) icon: Icon,
    pub(in crate::editor) title: &'a str,
}

impl ToolIdentity<'_> {
    fn icon(self, ui: &mut Ui, busy: bool) {
        self.icon_warned(ui, busy, None);
    }

    fn icon_warned(self, ui: &mut Ui, busy: bool, warning: Option<&str>) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::splat(TOOL_ICON_SIZE), egui::Sense::hover());
        if busy {
            ui.put(rect, egui::Spinner::new().size(TOOL_ICON_SIZE));
        } else if let Some(reason) = warning {
            feedback::warning_icon(ui, response, "Tool warning", reason);
        } else {
            self.icon.paint(ui, rect, ui.visuals().selection.stroke);
        }
    }

    fn viewer_title(self) -> egui::RichText {
        instrument_label(self.title, 12.).strong()
    }

    /// Inline inspector framing; the caller supplies header actions and body state.
    pub(in crate::editor) fn inspector<H, R>(
        self,
        ui: &mut Ui,
        header: impl FnOnce(&mut Ui) -> H,
        contents: impl FnOnce(&mut Ui, H) -> R,
    ) -> R {
        egui::Frame::NONE
            .fill(active_tool_fill(ui))
            .inner_margin(TOOL_FRAME_MARGIN)
            .show(ui, |ui| {
                let header = icons::row(ui, |ui| {
                    self.icon(ui, false);
                    ui.strong(self.title);
                    header(ui)
                })
                .inner;
                contents(ui, header)
            })
            .inner
    }
}

/// Viewer transaction presentation; availability and dispatch remain caller-owned.
#[derive(Clone, Copy)]
pub(in crate::editor) struct ToolGroup<'a> {
    pub(in crate::editor) identity: ToolIdentity<'a>,
    pub(in crate::editor) busy: bool,
    pub(in crate::editor) cancel_enabled: bool,
    pub(in crate::editor) apply_enabled: bool,
}

pub(in crate::editor) struct RoleActions {
    pub(in crate::editor) dismiss: Response,
    pub(in crate::editor) primary: Option<Response>,
}

fn tool_identity_width(ui: &Ui, identity: ToolIdentity<'_>) -> f32 {
    egui::WidgetText::into_galley(
        identity.viewer_title().into(),
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    )
    .size()
    .x + TOOL_ICON_SIZE
}

fn paint_tool_identity_warned(
    ui: &mut Ui,
    identity: ToolIdentity<'_>,
    busy: bool,
    warning: Option<&str>,
) {
    identity.icon_warned(ui, busy, warning);
    let label = ui.add(egui::Label::new(identity.viewer_title()));
    label.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Label,
            label.enabled(),
            format!(
                "Active tool: {}{}",
                identity.title,
                if busy { " · Working" } else { "" }
            ),
        )
    });
}

fn tool_group_frame<R>(
    ui: &mut Ui,
    width: f32,
    contents: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::Frame::NONE
        .fill(active_tool_fill(ui))
        .inner_margin(egui::Margin::symmetric(TOOL_FRAME_MARGIN, 0))
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(width, CONTROL_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                contents,
            )
            .inner
        })
}

impl ToolGroup<'_> {
    fn apply(self, label: &'static str) -> TextAction<'static> {
        TextAction::new(label)
            .enabled(self.apply_enabled)
            .primary()
            .toolbar(0.)
            .help("Apply (Enter).")
    }

    pub(in crate::editor) fn width_actions(
        self,
        ui: &Ui,
        primary: Option<&'static str>,
        dismiss: &'static str,
    ) -> f32 {
        tool_identity_width(ui, self.identity)
            + TextAction::new(dismiss).toolbar(0.).width(ui)
            + primary.map_or(0., |label| self.apply(label).width(ui))
            + (if primary.is_some() { 3. } else { 2. }) * ui.spacing().item_spacing.x
            + TOOL_ACTION_SPACE
            + 2. * f32::from(TOOL_FRAME_MARGIN)
    }

    pub(in crate::editor) fn show_actions(
        self,
        ui: &mut Ui,
        primary: Option<&'static str>,
        dismiss: &'static str,
        warning: Option<&str>,
    ) -> egui::InnerResponse<RoleActions> {
        let width = self.width_actions(ui, primary, dismiss) - 2. * f32::from(TOOL_FRAME_MARGIN);
        tool_group_frame(ui, width, |ui| {
            paint_tool_identity_warned(ui, self.identity, self.busy, warning);
            ui.add_space(TOOL_ACTION_SPACE);
            let dismiss = TextAction::new(dismiss)
                .enabled(self.cancel_enabled)
                .toolbar(0.)
                .help("Escape");
            let dismiss = if primary.is_none() {
                dismiss.primary()
            } else {
                dismiss
            };
            RoleActions {
                dismiss: dismiss.show(ui),
                primary: primary.map(|label| self.apply(label).show(ui)),
            }
        })
    }
}

#[cfg(test)]
mod tests;
