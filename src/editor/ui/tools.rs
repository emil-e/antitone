use super::{
    CONTROL_HEIGHT, INSPECTOR_ROW_HEIGHT, TextAction, feedback,
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
const TOOL_TRAILING_SPACE: f32 = 6.;
const TOOL_CONTROL_PADDING: i8 = 8;

/// Fixed stage-level home for immediate image commands and canvas-tool launchers.
pub(in crate::editor) struct ToolShelf;

impl ToolShelf {
    pub(in crate::editor) fn show<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
        let width = ui.available_rect_before_wrap().x_range();
        let shelf = egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(4, 0))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                ui.spacing_mut().interact_size.y = INSPECTOR_ROW_HEIGHT;
                icons::row_at(ui, INSPECTOR_ROW_HEIGHT, contents).inner
            });
        let seam_gap = ui.spacing().item_spacing.y + super::inspector::STAGE_RULE_INSET;
        ui.painter().hline(
            width,
            shelf.response.rect.bottom() + seam_gap,
            ui.visuals().widgets.noninteractive.bg_stroke,
        );
        shelf.inner
    }
}

#[derive(Clone, Copy)]
pub(in crate::editor) struct ToolButton<'a> {
    icon: Icon,
    name: &'a str,
    help: &'a str,
    enabled: bool,
    active: Option<bool>,
    changed: bool,
    emphasized: bool,
}

impl<'a> ToolButton<'a> {
    pub(in crate::editor) fn command(icon: Icon, name: &'a str, help: &'a str) -> Self {
        Self {
            icon,
            name,
            help,
            enabled: true,
            active: None,
            changed: false,
            emphasized: false,
        }
    }

    pub(in crate::editor) fn tool(icon: Icon, name: &'a str, help: &'a str, active: bool) -> Self {
        Self {
            active: Some(active),
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

    pub(in crate::editor) fn emphasized(mut self) -> Self {
        self.emphasized = true;
        self
    }

    pub(in crate::editor) fn show(self, ui: &mut Ui) -> Response {
        let backdrop = (self.emphasized && self.active != Some(true))
            .then(|| ui.painter().add(egui::Shape::Noop));
        // Shelves inherit the inspector row height so the row they were given actually fits.
        let response = ui
            .scope(|ui| {
                if self.active == Some(true) {
                    let color = ui.visuals().selection.stroke.color;
                    let widgets = &mut ui.visuals_mut().widgets;
                    for state in [
                        &mut widgets.inactive,
                        &mut widgets.hovered,
                        &mut widgets.active,
                    ] {
                        state.fg_stroke.color = color;
                    }
                }
                icons::button_at(
                    ui,
                    self.icon,
                    self.enabled,
                    self.name,
                    self.help,
                    ui.spacing().interact_size.y,
                )
            })
            .inner;
        if let Some(backdrop) = backdrop
            && response.enabled()
            && !response.hovered()
            && !response.has_focus()
        {
            ui.painter().set(
                backdrop,
                egui::Shape::rect_filled(response.rect, 4., ui.visuals().selection.bg_fill),
            );
        }
        if self.active == Some(true) {
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
        if self.active.is_some() || self.changed {
            ui.ctx().accesskit_node_builder(response.id, |node| {
                if let Some(active) = self.active {
                    node.set_selected(active);
                    node.set_toggled(if active {
                        egui::accesskit::Toggled::True
                    } else {
                        egui::accesskit::Toggled::False
                    });
                }
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

fn tool_title_width(ui: &Ui, identity: ToolIdentity<'_>) -> f32 {
    egui::WidgetText::into_galley(
        identity.viewer_title().into(),
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    )
    .size()
    .x
}

fn tool_identity_badge(
    ui: &mut Ui,
    identity: ToolIdentity<'_>,
    busy: bool,
    warning: Option<&str>,
    title_slot: f32,
) {
    let label = egui::Frame::NONE
        .fill(active_tool_fill(ui))
        .corner_radius(ui.visuals().widgets.inactive.corner_radius)
        .inner_margin(egui::Margin::symmetric(TOOL_CONTROL_PADDING, 0))
        .show(ui, |ui| {
            let gap = ui.spacing().item_spacing.x.min(title_slot);
            let title_width = title_slot - gap;
            ui.spacing_mut().item_spacing.x = gap;
            ui.set_min_height(CONTROL_HEIGHT);
            identity.icon_warned(ui, busy, warning);
            ui.allocate_ui_with_layout(
                egui::vec2(title_width, CONTROL_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(title_width);
                    ui.add(egui::Label::new(identity.viewer_title()).truncate())
                },
            )
            .inner
        })
        .inner;
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

impl ToolGroup<'_> {
    fn apply(self, label: &'static str) -> TextAction<'static> {
        TextAction::new(label)
            .enabled(self.apply_enabled)
            .primary()
            .toolbar(0.)
            .horizontal_padding(f32::from(TOOL_CONTROL_PADDING))
            .help("Apply (Enter).")
    }

    fn dismiss(self, label: &'static str, has_primary: bool) -> TextAction<'static> {
        let dismiss = TextAction::new(label)
            .enabled(self.cancel_enabled)
            .toolbar(0.)
            .horizontal_padding(f32::from(TOOL_CONTROL_PADDING));
        if has_primary {
            dismiss
        } else {
            dismiss.primary()
        }
    }

    pub(in crate::editor) fn width_actions(
        self,
        ui: &Ui,
        primary: Option<&'static str>,
        dismiss: &'static str,
    ) -> f32 {
        self.minimum_width_actions(ui, primary, dismiss)
            + self.natural_title_slot(ui)
            + TOOL_ACTION_SPACE
            + TOOL_TRAILING_SPACE
    }

    fn natural_title_slot(self, ui: &Ui) -> f32 {
        ui.spacing().item_spacing.x + tool_title_width(ui, self.identity)
    }

    pub(in crate::editor) fn minimum_width_actions(
        self,
        ui: &Ui,
        primary: Option<&'static str>,
        dismiss: &'static str,
    ) -> f32 {
        TOOL_ICON_SIZE
            + self.dismiss(dismiss, primary.is_some()).width(ui)
            + primary.map_or(0., |label| self.apply(label).width(ui))
            + (if primary.is_some() { 2. } else { 1. }) * ui.spacing().item_spacing.x
            + 2. * f32::from(TOOL_CONTROL_PADDING)
    }

    pub(in crate::editor) fn show_actions(
        self,
        ui: &mut Ui,
        primary: Option<&'static str>,
        dismiss: &'static str,
        warning: Option<&str>,
    ) -> egui::InnerResponse<RoleActions> {
        let fixed = self.minimum_width_actions(ui, primary, dismiss);
        let flexible = (ui.available_width() - fixed).max(0.);
        // Elide the title and yield surrounding whitespace before squeezing any control.
        let trailing = flexible.min(TOOL_TRAILING_SPACE);
        let action_space = (flexible - trailing).min(TOOL_ACTION_SPACE);
        let title_slot = self
            .natural_title_slot(ui)
            .min(flexible - trailing - action_space);
        let width = fixed + title_slot + action_space + trailing;
        ui.allocate_ui_with_layout(
            egui::vec2(width, CONTROL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(width);
                tool_identity_badge(ui, self.identity, self.busy, warning, title_slot);
                ui.add_space(action_space);
                RoleActions {
                    dismiss: self.dismiss(dismiss, primary.is_some()).show(ui),
                    primary: primary.map(|label| self.apply(label).show(ui)),
                }
            },
        )
    }
}

#[cfg(test)]
mod tests;
