use super::{
    CONTROL_HEIGHT, INSPECTOR_BODY_GAP, INSPECTOR_HEADER_HEIGHT, INSPECTOR_ROW_HEIGHT,
    INSPECTOR_SECTION_GAP,
    icons::{self, Icon},
    reset_button_at,
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
        egui::vec2(ui.available_width(), INSPECTOR_HEADER_HEIGHT),
        egui::Sense::hover(),
    );
    let tabs = [
        (
            "Roll",
            "Shared scanning setup, calibration and negative inversion.",
        ),
        (
            "Photo",
            "Composition and photographic grade for this image.",
        ),
        ("Rendering", "Output colour space for preview and TIFF."),
    ];
    let size = egui::TextStyle::Button.resolve(ui.style()).size;
    let texts: Vec<_> = tabs
        .iter()
        .map(|(label, _)| {
            egui::WidgetText::from(
                super::instrument_label(label, size).extra_letter_spacing(size * 0.02),
            )
            .into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            )
        })
        .collect();
    let breathing_room =
        (bar.width() - 24. - texts.iter().map(|text| text.size().x).sum::<f32>()) / 3.;
    let mut left = bar.left();
    for (index, ((label, help), text)) in tabs.into_iter().zip(texts).enumerate() {
        let rect = egui::Rect::from_min_size(
            egui::pos2(left, bar.top()),
            egui::vec2(text.size().x + breathing_room, bar.height()),
        );
        left = rect.right() + 12.;
        if index != 0 {
            Icon::ChevronRight.paint(
                ui,
                egui::Rect::from_center_size(
                    rect.left_center() - egui::vec2(6., 0.),
                    egui::vec2(10., 10.),
                ),
                ui.visuals().widgets.noninteractive.fg_stroke,
            );
        }
        let response = ui.interact(
            rect,
            ui.make_persistent_id(("inspector-tab", index)),
            egui::Sense::click(),
        );
        if response.clicked() {
            *selected = index;
        }
        let active = *selected == index;
        let visuals = ui.style().interact(&response);
        if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
            ui.painter().rect(
                rect,
                4.,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        let color = if active {
            ui.visuals().strong_text_color()
        } else {
            visuals.fg_stroke.color
        };
        ui.painter().galley_with_override_text_color(
            rect.center() - text.size() * 0.5,
            text,
            color,
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::SelectableLabel,
                response.enabled(),
                active,
                label,
            )
        });
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_role(egui::accesskit::Role::Tab);
            node.set_selected(active);
        });
        if active {
            ui.painter().line_segment(
                [rect.left_bottom(), rect.right_bottom()],
                egui::Stroke::new(2., ui.visuals().selection.stroke.color),
            );
        }
        response.on_hover_text(help);
    }
}

/// Single-select row with pointer capture for reordering and an independent enable toggle.
pub fn draggable_icon_row(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    selected: bool,
    enabled: Option<&mut bool>,
) -> (egui::Response, Option<egui::Response>) {
    let (rect, allocated) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), CONTROL_HEIGHT),
        egui::Sense::hover(),
    );
    let toggle_width = if enabled.is_some() {
        CONTROL_HEIGHT
    } else {
        0.
    };
    let select_rect = egui::Rect::from_min_max(rect.min, rect.max - egui::vec2(toggle_width, 0.));
    let response = ui.interact(
        select_rect,
        allocated.id.with("select"),
        egui::Sense::click_and_drag(),
    );
    let visuals = ui.style().interact_selectable(&response, selected);
    ui.painter().rect(
        rect,
        visuals.corner_radius,
        visuals.weak_bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    icon.paint(
        ui,
        egui::Rect::from_center_size(
            rect.left_center() + egui::vec2(14., 0.),
            egui::vec2(16., 16.),
        ),
        visuals.fg_stroke,
    );
    let text = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        (select_rect.width() - 38.).max(1.),
        egui::TextStyle::Button,
    );
    ui.painter().galley_with_override_text_color(
        egui::pos2(rect.left() + 30., rect.center().y - text.size().y * 0.5),
        text,
        visuals.fg_stroke.color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            response.enabled(),
            selected,
            label,
        )
    });
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_selected(selected));
    let toggle = enabled.map(|enabled| {
        let response = ui.put(
            egui::Rect::from_min_max(egui::pos2(select_rect.right(), rect.top()), rect.max),
            egui::Checkbox::without_text(enabled),
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Checkbox,
                response.enabled(),
                *enabled,
                format!("Enable {label}"),
            )
        });
        response
    });
    (response.on_hover_text(label), toggle)
}

/// A section-level tool that acts on the section's subject rather than on one value.
#[derive(Clone, Copy)]
pub struct SectionTool<'a> {
    pub icon: Icon,
    pub enabled: bool,
    pub name: &'a str,
    pub help: &'a str,
}

pub struct SectionHeader<'a> {
    pub summary: &'a str,
    pub changed: bool,
    pub reset: Option<bool>,
    /// Trailing header action, right of the reset, drawn only for inspector sections.
    pub tool: Option<SectionTool<'a>>,
    /// None keeps transactional/pinned controls visible without a disclosure.
    pub default_open: Option<bool>,
}

/// Header actions a section reports back; contents keep their own result.
#[derive(Clone, Copy, Default)]
pub struct SectionAction {
    pub reset: bool,
    pub tool: bool,
}

pub fn section<R>(
    ui: &mut Ui,
    id: &str,
    title: &str,
    header: SectionHeader<'_>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Option<R>, SectionAction) {
    section_impl(
        ui,
        id,
        title,
        header,
        SectionPresentation::Inspector,
        contents,
    )
}

/// Compact dialog disclosure; expansion remains session state, not inspector preferences.
pub fn dialog_disclosure<R>(
    ui: &mut Ui,
    id: &str,
    title: &str,
    contents: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    section_impl(
        ui,
        id,
        title,
        SectionHeader {
            summary: "",
            changed: false,
            reset: None,
            tool: None,
            default_open: Some(false),
        },
        SectionPresentation::Dialog,
        contents,
    )
    .0
}

/// A section disclosure with caller-owned visibility and room for trailing header controls.
pub fn section_disclosure(ui: &mut Ui, title: &str, open: &mut bool, width: f32) -> egui::Response {
    let response = section_heading(
        ui,
        title,
        Some(*open),
        false,
        egui::vec2(width, CONTROL_HEIGHT),
        true,
    );
    if response.clicked() {
        *open = !*open;
    }
    ui.ctx()
        .accesskit_node_builder(response.id, |node| node.set_expanded(*open));
    response
}

fn section_heading(
    ui: &mut Ui,
    title: &str,
    open: Option<bool>,
    changed: bool,
    size: egui::Vec2,
    inspector: bool,
) -> egui::Response {
    let text_color = if inspector {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    let text = ui.painter().layout_no_wrap(
        title.into(),
        if inspector {
            egui::TextStyle::Button
        } else {
            egui::TextStyle::Body
        }
        .resolve(ui.style()),
        text_color,
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let response = ui.interact(
        rect,
        ui.make_persistent_id("disclosure"),
        if open.is_some() {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    if let Some(open) = open {
        if response.hovered() || response.has_focus() || response.is_pointer_button_down_on() {
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
            if inspector {
                ui.style().interact(&response).fg_stroke
            } else {
                egui::Stroke::new(1.5, text_color)
            },
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
    let left = rect.left() + if open.is_some() { 24. } else { 0. };
    let mut clip = rect;
    clip.max.x -= if changed { 12. } else { 0. };
    ui.painter().with_clip_rect(clip).galley(
        egui::pos2(left, rect.center().y - text.size().y * 0.5),
        text,
        text_color,
    );
    if changed {
        ui.painter().circle_filled(
            rect.right_center() - egui::vec2(5., 0.),
            2.5,
            ui.visuals().selection.stroke.color,
        );
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_description("Modified");
        });
    }
    response
}

const DIALOG_DISCLOSURE_HEIGHT: f32 = 22.;

#[derive(Clone, Copy)]
enum SectionPresentation {
    Inspector,
    Dialog,
}

fn section_impl<R>(
    ui: &mut Ui,
    id: &str,
    title: &str,
    header: SectionHeader<'_>,
    presentation: SectionPresentation,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (Option<R>, SectionAction) {
    let SectionHeader {
        summary,
        changed,
        reset: reset_enabled,
        tool,
        default_open,
    } = header;
    let inspector = matches!(presentation, SectionPresentation::Inspector);
    ui.push_id(id, |ui| {
        if inspector {
            ui.add_space(INSPECTOR_SECTION_GAP);
            ui.separator();
        }
        let state_id = ui.make_persistent_id("expanded");
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            state_id,
            default_open.unwrap_or(true),
        );
        if inspector
            && default_open.is_some()
            && let Some(open) = ui.ctx().data_mut(|data| {
                data.get_temp::<SectionExpansion>(egui::Id::new("inspector-expansion"))
                    .and_then(|sections| sections.0.get(id).copied())
            })
        {
            state.set_open(open);
        }
        let open = default_open.is_none() || state.is_open();
        let mut action = SectionAction::default();
        let draw_header = |ui: &mut Ui| {
            let text_color = if inspector {
                ui.visuals().strong_text_color()
            } else {
                ui.visuals().text_color()
            };
            let text = ui.painter().layout_no_wrap(
                title.into(),
                if inspector {
                    egui::TextStyle::Button
                } else {
                    egui::TextStyle::Body
                }
                .resolve(ui.style()),
                text_color,
            );
            let row_height = if inspector {
                INSPECTOR_ROW_HEIGHT
            } else {
                CONTROL_HEIGHT
            };
            let tools = usize::from(reset_enabled.is_some()) + usize::from(tool.is_some());
            let reset_width = tools as f32 * (row_height + ui.spacing().item_spacing.x);
            let width = if inspector {
                ui.available_width() - reset_width
            } else {
                (text.size().x + 28.).min(ui.available_width())
            }
            .max(1.);
            let height = if inspector {
                INSPECTOR_ROW_HEIGHT
            } else {
                DIALOG_DISCLOSURE_HEIGHT
            };
            let response = section_heading(
                ui,
                title,
                default_open.map(|_| open),
                changed,
                egui::vec2(width, height),
                inspector,
            );

            if let Some(enabled) = reset_enabled {
                action.reset |= reset_button_at(ui, enabled, title, row_height).clicked();
            }
            if let Some(tool) = tool {
                action.tool |= icons::button_at(
                    ui,
                    tool.icon,
                    tool.enabled && inspector,
                    tool.name,
                    tool.help,
                    row_height,
                )
                .clicked();
            }
            response
        };
        let header = if inspector {
            icons::row(ui, draw_header).inner
        } else {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), DIALOG_DISCLOSURE_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                draw_header,
            )
            .inner
        };
        if header.clicked() {
            state.toggle(ui);
            if !inspector {
                header.request_focus();
            }
        }
        state.store(ui.ctx());
        if inspector && default_open.is_some() {
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
            ui.add_space(INSPECTOR_BODY_GAP);
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
        (body, action)
    })
    .inner
}
