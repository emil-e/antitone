use super::icons::{self, Icon};
use egui::{Color32, Response, Ui, Vec2};

#[path = "editor_ui_cards.rs"]
mod cards;
pub(super) use cards::ContactSheetLayout;

#[path = "editor_ui_hierarchy.rs"]
mod hierarchy;
pub(super) use hierarchy::{
    SectionHeader, collect_expansion, restore_expansion, section, stage_tabs,
};

pub(super) const CONTROL_HEIGHT: f32 = 28.;
pub(super) const VIEWER_BAR_HEIGHT: f32 = 40.;
pub(super) const VIEWER_ITEM_GAP: f32 = 4.;
const HEADER_HEIGHT: f32 = 32.;
const INSCRIPTION: Color32 = Color32::from_rgb(223, 170, 96);

pub(super) fn configure_branding(ui: &mut Ui) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = Color32::from_rgb(34, 36, 38);
    visuals.window_fill = visuals.panel_fill;
    visuals.selection.stroke.color = INSCRIPTION;
    visuals.selection.bg_fill = INSCRIPTION.gamma_multiply(0.25);
    visuals.hyperlink_color = INSCRIPTION;
    ui.ctx().set_theme(egui::Theme::Dark);
    ui.ctx().set_visuals(visuals.clone());
    *ui.visuals_mut() = visuals;
}

pub(super) fn app_header<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Panel::top("header")
        .exact_size(HEADER_HEIGHT)
        .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(12, 2)))
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                wordmark(ui);
                contents(ui)
            })
            .inner
        })
        .inner
}

fn wordmark(ui: &mut Ui) -> Response {
    let id = egui::Id::new("antitone-wordmark-texture");
    let cached = ui
        .ctx()
        .data_mut(|data| data.get_temp::<egui::TextureHandle>(id));
    let texture = cached.unwrap_or_else(|| {
        let image = image::load_from_memory_with_format(
            include_bytes!("../assets/branding/wordmark-header.png"),
            image::ImageFormat::Png,
        )
        .expect("valid embedded wordmark PNG")
        .into_rgba8();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            image.as_raw(),
        );
        let texture =
            ui.ctx()
                .load_texture("Antitone wordmark", image, egui::TextureOptions::LINEAR);
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, texture.clone()));
        texture
    });
    // The outlined 16-point inscription is rasterized at 4×, independent of UI fonts.
    let size = texture.size_vec2() / 4.;
    let response = ui.add(egui::Image::from_texture((texture.id(), size)));
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, response.enabled(), "Antitone")
    });
    response
}

pub(super) fn active_tool_fill(ui: &Ui) -> Color32 {
    ui.visuals().selection.bg_fill.linear_multiply(0.15)
}

/// One label and a right-aligned caller-owned control group; no additional ID scope.
pub(super) fn labelled_row<R>(
    ui: &mut Ui,
    label: &str,
    help: Option<&str>,
    controls: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    icons::row(ui, |ui| {
        let label = ui.label(label);
        if let Some(help) = help {
            label.on_hover_text(help);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), controls)
            .inner
    })
}

pub(super) fn reset_button(ui: &mut Ui, enabled: bool, name: &str) -> Response {
    let name = format!("Reset {name}");
    icons::button(ui, Icon::Reset, enabled, &name, &name)
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

const TOOL_ICON_SIZE: f32 = 16.;
const TOOL_FRAME_MARGIN: i8 = 6;
const TOOL_ACTION_SPACE: f32 = 8.;

/// Fixed stage-level home for immediate image commands and canvas-tool launchers.
pub(super) struct ToolShelf;

impl ToolShelf {
    pub(super) fn show<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
        ui.push_id("tool-shelf", |ui| {
            ui.add_space(6.);
            ui.spacing_mut().item_spacing.x = 4.;
            let result = icons::row(ui, contents).inner;
            ui.add_space(6.);
            ui.separator();
            result
        })
        .inner
    }
}

#[derive(Clone, Copy)]
pub(super) struct ToolButton<'a> {
    icon: Icon,
    name: &'a str,
    help: &'a str,
    enabled: bool,
    active: bool,
    changed: bool,
}

impl<'a> ToolButton<'a> {
    pub(super) fn command(icon: Icon, name: &'a str, help: &'a str) -> Self {
        Self {
            icon,
            name,
            help,
            enabled: true,
            active: false,
            changed: false,
        }
    }

    pub(super) fn tool(icon: Icon, name: &'a str, help: &'a str, active: bool) -> Self {
        Self {
            active,
            ..Self::command(icon, name, help)
        }
    }

    pub(super) fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub(super) fn changed(mut self, changed: bool) -> Self {
        self.changed = changed;
        self
    }

    pub(super) fn show(self, ui: &mut Ui) -> Response {
        let response = icons::button(ui, self.icon, self.enabled, self.name, self.help);
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
pub(super) struct SessionToolGroup<'a> {
    pub(super) identity: ToolIdentity<'a>,
    pub(super) busy: bool,
    pub(super) done_enabled: bool,
}

impl SessionToolGroup<'_> {
    fn done(self) -> TextAction<'static> {
        TextAction::new("Done")
            .enabled(self.done_enabled)
            .primary()
            .toolbar(0.)
            .help("Finish the active tool (Escape).")
    }

    fn content_width(self, ui: &Ui) -> f32 {
        tool_identity_width(ui, self.identity)
            + self.done().width(ui)
            + 2. * ui.spacing().item_spacing.x
            + TOOL_ACTION_SPACE
    }

    pub(super) fn width(self, ui: &Ui) -> f32 {
        self.content_width(ui) + 2. * f32::from(TOOL_FRAME_MARGIN)
    }

    pub(super) fn show(self, ui: &mut Ui) -> egui::InnerResponse<Response> {
        let width = self.content_width(ui);
        tool_group_frame(ui, width, |ui| {
            paint_tool_identity(ui, self.identity, self.busy);
            ui.add_space(TOOL_ACTION_SPACE);
            self.done().show(ui)
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct ToolIdentity<'a> {
    pub(super) icon: Icon,
    pub(super) title: &'a str,
}

impl ToolIdentity<'_> {
    fn icon(self, ui: &mut Ui, busy: bool) {
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(TOOL_ICON_SIZE), egui::Sense::hover());
        if busy {
            ui.put(rect, egui::Spinner::new().size(TOOL_ICON_SIZE));
        } else {
            self.icon.paint(ui, rect, ui.visuals().selection.stroke);
        }
    }

    fn viewer_title(self) -> egui::RichText {
        egui::RichText::new(self.title)
            .font(egui::FontId::proportional(12.))
            .strong()
    }

    /// Inline inspector framing; the caller supplies header actions and body state.
    pub(super) fn inspector<H, R>(
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
pub(super) struct ToolGroup<'a> {
    pub(super) identity: ToolIdentity<'a>,
    pub(super) busy: bool,
    pub(super) cancel_enabled: bool,
    pub(super) apply_enabled: bool,
}

pub(super) struct ToolActions {
    pub(super) cancel: Response,
    pub(super) apply: Response,
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

fn paint_tool_identity(ui: &mut Ui, identity: ToolIdentity<'_>, busy: bool) {
    identity.icon(ui, busy);
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
    fn cancel(self) -> TextAction<'static> {
        TextAction::new("Cancel")
            .enabled(self.cancel_enabled)
            .toolbar(0.)
            .help("Cancel the active tool (Escape).")
    }

    fn apply(self) -> TextAction<'static> {
        TextAction::new("Apply")
            .enabled(self.apply_enabled)
            .primary()
            .toolbar(0.)
            .help("Apply the active tool (Enter).")
    }

    fn content_width(self, ui: &Ui) -> f32 {
        tool_identity_width(ui, self.identity)
            + self.cancel().width(ui)
            + self.apply().width(ui)
            + 3. * ui.spacing().item_spacing.x
            + TOOL_ACTION_SPACE
    }

    /// Complete single-line reservation, including frame margins, without widget IDs.
    pub(super) fn width(self, ui: &Ui) -> f32 {
        self.content_width(ui) + 2. * f32::from(TOOL_FRAME_MARGIN)
    }

    pub(super) fn show(self, ui: &mut Ui) -> egui::InnerResponse<ToolActions> {
        let width = self.content_width(ui);
        tool_group_frame(ui, width, |ui| {
            paint_tool_identity(ui, self.identity, self.busy);
            ui.add_space(TOOL_ACTION_SPACE);
            ToolActions {
                cancel: self.cancel().show(ui),
                apply: self.apply().show(ui),
            }
        })
    }
}

#[cfg(test)]
#[path = "editor_ui_tests.rs"]
mod tests;
