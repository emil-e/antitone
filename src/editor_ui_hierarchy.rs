use super::{
    CONTROL_HEIGHT,
    icons::{self, Icon},
    reset_button,
};
use egui::Ui;

#[derive(Clone, Default)]
struct SectionExpansion(std::collections::BTreeMap<String, bool>);

pub fn restore_expansion(ctx: &egui::Context, sections: std::collections::BTreeMap<String, bool>) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("inspector-expansion"),
            SectionExpansion(sections),
        )
    });
}

pub fn collect_expansion(ctx: &egui::Context) -> std::collections::BTreeMap<String, bool> {
    ctx.data_mut(|data| data.remove_temp::<SectionExpansion>(egui::Id::new("inspector-expansion")))
        .unwrap_or_default()
        .0
}

pub fn stage_tabs(ui: &mut Ui, selected: &mut usize) {
    // One allocation avoids fractional button widths growing the sidebar and changing Fit.
    let (bar, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), CONTROL_HEIGHT),
        egui::Sense::hover(),
    );
    let width = (bar.width() - 24.) / 3.;
    for (index, (label, help)) in [
        ("Roll", "Shared scanning setup, calibration and negative inversion."),
        ("Photo", "Composition and photographic grade for this image."),
        ("Rendering", "Shared rendered appearance: matching preview, thumbnails, scopes and TIFF target; not the monitor profile. EXR bypasses it."),
    ].into_iter().enumerate() {
        let rect = egui::Rect::from_min_size(bar.min + egui::vec2(index as f32 * (width + 12.), 0.), egui::vec2(width, bar.height()));
        if index != 0 {
            Icon::ChevronRight.paint(ui, egui::Rect::from_center_size(rect.left_center() - egui::vec2(6., 0.), egui::vec2(10., 10.)), ui.visuals().widgets.noninteractive.fg_stroke);
        }
        let response = ui.interact(rect, ui.make_persistent_id(("inspector-tab", index)), egui::Sense::click());
        if response.clicked() { *selected = index; }
        let active = *selected == index;
        let visuals = ui.style().interact(&response);
        if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
            ui.painter().rect(rect, 4., visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
        }
        let color = if active { ui.visuals().strong_text_color() } else { visuals.fg_stroke.color };
        let text = ui.painter().layout_no_wrap(label.into(), egui::TextStyle::Button.resolve(ui.style()), color);
        ui.painter().galley(rect.center() - text.size() * 0.5, text, color);
        response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, response.enabled(), active, label));
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_role(egui::accesskit::Role::Tab);
            node.set_selected(active);
        });
        if active {
            ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(2., ui.visuals().selection.stroke.color));
        }
        response.on_hover_text(help);
    }
}

pub struct SectionHeader<'a> {
    pub summary: &'a str,
    pub changed: bool,
    pub reset: Option<bool>,
    /// None keeps transactional/pinned controls visible without a disclosure.
    pub default_open: Option<bool>,
}

pub fn section<R>(
    ui: &mut Ui,
    id: &str,
    title: &str,
    header: SectionHeader<'_>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Option<R>, bool) {
    let SectionHeader {
        summary,
        changed,
        reset: reset_enabled,
        default_open,
    } = header;
    ui.push_id(id, |ui| {
        ui.add_space(8.);
        ui.separator();
        let state_id = ui.make_persistent_id("expanded");
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            state_id,
            default_open.unwrap_or(true),
        );
        if default_open.is_some()
            && let Some(open) = ui.ctx().data_mut(|data| {
                data.get_temp::<SectionExpansion>(egui::Id::new("inspector-expansion"))
                    .and_then(|sections| sections.0.get(id).copied())
            })
        {
            state.set_open(open);
        }
        let open = default_open.is_none() || state.is_open();
        let mut reset = false;
        let header = icons::row(ui, |ui| {
            let reset_width = if reset_enabled.is_some() {
                CONTROL_HEIGHT + ui.spacing().item_spacing.x
            } else {
                0.
            };
            let width = (ui.available_width() - reset_width).max(1.);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(width, CONTROL_HEIGHT), egui::Sense::hover());
            let response = ui.interact(
                rect,
                ui.make_persistent_id("disclosure"),
                if default_open.is_some() {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                },
            );
            if default_open.is_some() {
                if response.hovered()
                    || response.has_focus()
                    || response.is_pointer_button_down_on()
                {
                    let visuals = ui.style().interact(&response);
                    ui.painter().rect(
                        rect,
                        4.,
                        visuals.weak_bg_fill,
                        visuals.bg_stroke,
                        egui::StrokeKind::Inside,
                    );
                }
                let icon = if open {
                    Icon::ChevronDown
                } else {
                    Icon::ChevronRight
                };
                icon.paint(
                    ui,
                    egui::Rect::from_center_size(
                        rect.left_center() + egui::vec2(10., 0.),
                        egui::vec2(16., 16.),
                    ),
                    ui.style().interact(&response).fg_stroke,
                );
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::CollapsingHeader,
                        response.enabled(),
                        title,
                    )
                });
            } else {
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, response.enabled(), title)
                });
            }
            let left = rect.left() + if default_open.is_some() { 24. } else { 0. };
            let marker_width = if changed { 12. } else { 0. };
            let text = ui.painter().layout_no_wrap(
                title.into(),
                egui::TextStyle::Button.resolve(ui.style()),
                ui.visuals().strong_text_color(),
            );
            let mut clip = rect;
            clip.max.x -= marker_width;
            ui.painter().with_clip_rect(clip).galley(
                egui::pos2(left, rect.center().y - text.size().y * 0.5),
                text,
                ui.visuals().strong_text_color(),
            );
            if changed {
                ui.painter().circle_filled(
                    rect.right_center() - egui::vec2(5., 0.),
                    2.5,
                    ui.visuals().selection.stroke.color,
                );
            }
            if let Some(enabled) = reset_enabled {
                reset = reset_button(ui, enabled, title).clicked();
            }
            response
        })
        .inner;
        if header.clicked() {
            state.toggle(ui);
        }
        state.store(ui.ctx());
        if default_open.is_some() {
            ui.ctx().data_mut(|data| {
                let key = egui::Id::new("inspector-expansion");
                if data.get_temp::<SectionExpansion>(key).is_some() {
                    data.get_temp_mut_or_default::<SectionExpansion>(key)
                        .0
                        .insert(id.into(), state.is_open());
                }
            });
        }
        let open = default_open.is_none() || state.is_open();
        if default_open.is_some() {
            ui.ctx()
                .accesskit_node_builder(header.id, |node| node.set_expanded(open));
        }
        let body = if open {
            ui.add_space(6.);
            // A disclosure press steals numeric focus before its release toggles the section.
            // Disabled parameter snapshots absorb that pending commit without editing the grade.
            Some(
                ui.add_enabled_ui(!header.is_pointer_button_down_on(), contents)
                    .inner,
            )
        } else {
            if !summary.is_empty() {
                let summary = if changed {
                    format!("Adjusted · {summary}")
                } else {
                    summary.into()
                };
                ui.add(egui::Label::new(egui::RichText::new(&summary).small().weak()).truncate())
                    .on_hover_text(summary);
            }
            None
        };
        (body, reset)
    })
    .inner
}
