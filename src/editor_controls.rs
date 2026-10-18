use super::{
    icons::{self, Icon},
    presentation::{SectionHeader, ToolIdentity, labelled_row, reset_button, section},
};
use crate::{
    color::{Balance, ap1_to_cones, opponent, opponent_coordinates},
    processing::{
        DEFAULT_COLOUR_MIX, DEFAULT_GAMMA, DEFAULT_PIVOT, DEFAULT_TOE, MAX_EXPOSURE, MAX_GAMMA,
        MAX_SHOULDER, MAX_TOE, MIN_GAMMA, MIN_TOE,
    },
};
use egui::{Color32, Response, Ui};
use glam::{DMat2, DMat3, DVec2, DVec3};
use std::ops::RangeInclusive;

pub(super) fn active_tool<R>(
    ui: &mut Ui,
    icon: Icon,
    title: &str,
    reset: Option<bool>,
    contents: impl FnOnce(&mut Ui, bool) -> R,
) -> R {
    ToolIdentity { icon, title }.inspector(
        ui,
        |ui| {
            reset.is_some_and(|enabled| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    icons::button(
                        ui,
                        Icon::Reset,
                        enabled,
                        "Reset draft",
                        "Reset crop and angle.",
                    )
                    .clicked()
                })
                .inner
            })
        },
        contents,
    )
}

pub(super) const DEFAULT_RADIAL_FEATHER: f64 = 0.8;

#[derive(Clone, Copy)]
pub enum Parameter {
    Gamma,
    Toe,
    Shoulder,
    Exposure,
    Warmth,
    Tint,
    Contrast,
    Pivot,
    ColourMix,
    Saturation,
    Density,
    CropMargin,
    Strength,
    Feather,
}

#[derive(Clone, Copy)]
pub(super) enum Reset {
    Parameter(Parameter, ColorControl),
    Exposure,
    Contrast,
    Saturation,
    Balance(ColorControl),
    GammaAndBalance,
    Response,
    Target,
}

// The composed editor collects reset intent until numeric focus loss has been
// admitted. Standalone controls and tool-draft values retain immediate resets.
pub(super) fn defer_reset(ui: &Ui, reset: Reset) -> bool {
    if matches!(reset, Reset::Parameter(Parameter::CropMargin, _)) {
        return false;
    }
    let active = ui.ctx().data(|data| {
        data.get_temp::<bool>(egui::Id::new("defer-control-resets"))
            .unwrap_or(false)
    });
    if active {
        ui.ctx().data_mut(|data| {
            data.get_temp_mut_or_default::<Vec<Reset>>(egui::Id::new("control-resets"))
                .push(reset)
        });
    }
    active
}

impl Parameter {
    fn label(self) -> &'static str {
        match self {
            Self::Gamma => "Gamma",
            Self::Toe => "Toe",
            Self::Shoulder => "Shoulder",
            Self::Exposure => "Exposure",
            Self::Warmth => "Warmth",
            Self::Tint => "Tint",
            Self::Contrast => "Contrast",
            Self::Pivot => "Pivot",
            Self::ColourMix => "Colour preservation",
            Self::Saturation => "Saturation",
            Self::Density => "Density",
            Self::CropMargin => "Inward margin",
            Self::Strength => "Strength",
            Self::Feather => "Feather",
        }
    }

    fn range(self) -> RangeInclusive<f64> {
        match self {
            Self::Gamma => MIN_GAMMA..=MAX_GAMMA,
            Self::Toe => MIN_TOE..=MAX_TOE,
            Self::Shoulder => 0.0..=MAX_SHOULDER,
            Self::Exposure => -MAX_EXPOSURE..=MAX_EXPOSURE,
            Self::Contrast => 0.4..=1.6,
            Self::Pivot => 0.01..=1.,
            Self::Saturation => 0.0..=2.,
            Self::ColourMix | Self::Density | Self::Strength | Self::Feather => 0.0..=1.,
            Self::CropMargin => 0.0..=crate::auto_crop::MAX_MARGIN_PERCENT,
            _ => -1.0..=1.0,
        }
    }

    fn slider_range(self) -> RangeInclusive<f64> {
        if matches!(self, Self::Exposure) {
            -3.0..=3.0
        } else {
            self.range()
        }
    }

    pub(super) fn default(self) -> f64 {
        match self {
            Self::Gamma => DEFAULT_GAMMA,
            Self::Toe => DEFAULT_TOE,
            Self::Contrast | Self::Saturation | Self::Strength => 1.,
            Self::Pivot => DEFAULT_PIVOT,
            Self::Feather => DEFAULT_RADIAL_FEATHER,
            Self::ColourMix | Self::Density => DEFAULT_COLOUR_MIX,
            Self::CropMargin => crate::auto_crop::DEFAULT_MARGIN_PERCENT,
            _ => 0.,
        }
    }

    fn step(self) -> f64 {
        match self {
            Self::Exposure => 0.05,
            Self::Shoulder => 0.0001,
            Self::Pivot => 0.001,
            _ => 0.01,
        }
    }
}

/// egui 0.36 retains cancelled text for a second lost-focus pass; reject both writes.
pub(super) fn numeric_cancelled(ui: &Ui, response: &Response) -> bool {
    let id = response.id.with("cancelled-numeric-edit");
    let cancelled = ui
        .ctx()
        .data_mut(|data| data.remove_temp::<bool>(id))
        .unwrap_or(false);
    let cancelling = ui.input(|input| input.key_pressed(egui::Key::Escape))
        && (response.lost_focus() || ui.memory(|memory| memory.had_focus_last_frame(response.id)));
    if cancelling {
        ui.ctx().data_mut(|data| data.insert_temp(id, true));
    }
    cancelled || cancelling
}

fn history_gesture(ui: &Ui, response: &Response) {
    if response.enabled()
        && (response.is_pointer_button_down_on() || response.dragged() || response.drag_stopped())
    {
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("project-control-gesture"), response.id);
        });
    }
}

pub fn parameter(ui: &mut Ui, kind: Parameter, value: &mut f64) -> (Response, Response, Response) {
    // DragValue can commit on focus loss while disabled; render a read-only snapshot.
    let mut disabled_value = *value;
    let value = if ui.is_enabled() {
        value
    } else {
        &mut disabled_value
    };

    ui.push_id(kind.label(), |ui| {
        let help = match kind {
            Parameter::Contrast => "Adjust contrast around Pivot; 1 leaves contrast unchanged.",
            Parameter::Pivot => "Set the brightness that contrast pivots around.",
            Parameter::ColourMix => "Reduce colour shifts caused by contrast.",
            Parameter::Saturation => "0 makes the image grey; 1 leaves saturation unchanged.",
            Parameter::Density => "Higher values darken colours more as saturation increases.",
            Parameter::CropMargin => {
                "Trim each edge by a percentage of the detected frame's shorter side."
            }
            Parameter::Strength => "Scale the adjustment from no effect to full strength.",
            Parameter::Feather => {
                "Soften the oval's edge; 0 is hard, 1 fades across the full radius."
            }
            _ => "",
        };

        let (numeric, reset, label) = labelled_row(
            ui,
            kind.label(),
            (!help.is_empty()).then_some(help),
            |ui, label| {
                let reset = reset_button(ui, *value != kind.default(), kind.label());
                if reset.clicked() {
                    let balance = ui
                        .ctx()
                        .data(|data| {
                            data.get_temp::<ColorControl>(egui::Id::new("reset-balance-scope"))
                        })
                        .unwrap_or(ColorControl::WhiteBalance);
                    if !defer_reset(ui, Reset::Parameter(kind, balance)) {
                        *value = kind.default();
                    }
                }

                match kind {
                    Parameter::Exposure => {
                        ui.label("EV");
                    }
                    Parameter::CropMargin => {
                        ui.label("%");
                    }
                    _ => {}
                }

                let previous = *value;
                let numeric = ui
                    .push_id("numeric", |ui| {
                        ui.add_sized(
                            [85., ui.spacing().interact_size.y],
                            egui::DragValue::new(value)
                                .speed(match kind {
                                    Parameter::Shoulder => 0.0001,
                                    Parameter::Pivot => 0.001,
                                    _ => 0.01,
                                })
                                .range(kind.range())
                                .clamp_existing_to_range(false)
                                .min_decimals(2)
                                .max_decimals(6)
                                .update_while_editing(false),
                        )
                    })
                    .inner;
                if numeric_cancelled(ui, &numeric) {
                    *value = previous;
                }
                (numeric.labelled_by(label), reset, label)
            },
        )
        .inner;

        let step = kind.step()
            * if ui.input(|input| input.modifiers.shift) {
                0.1
            } else {
                1.
            };
        let slider = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = ui.available_width();
                ui.add(
                    egui::Slider::new(value, kind.slider_range())
                        .clamping(egui::SliderClamping::Edits)
                        .show_value(false)
                        .logarithmic(matches!(kind, Parameter::Pivot))
                        .smart_aim(false)
                        .step_by(step),
                )
                .on_hover_text(if matches!(kind, Parameter::Exposure) {
                    "Slider: ±3 EV; type a value for up to ±16 EV."
                } else {
                    "Hold Shift for finer steps."
                })
            })
            .inner
            .labelled_by(label);
        history_gesture(ui, &slider);
        history_gesture(ui, &numeric);
        ui.add_space(6.);
        (slider, numeric, reset)
    })
    .inner
}

pub(super) fn photographic_exposure(ui: &mut Ui, photo: &mut crate::processing::PhotoSettings) {
    let (_, reset) = section(
        ui,
        "photo-exposure",
        "Exposure",
        SectionHeader {
            summary: &format!("{:+.2} EV", photo.exposure),
            changed: photo.exposure != 0.,
            reset: Some(photo.exposure != 0.),
            default_open: Some(true),
        },
        |ui| {
            parameter(ui, Parameter::Exposure, &mut photo.exposure);
        },
    );
    if reset && !defer_reset(ui, Reset::Exposure) {
        photo.exposure = 0.;
    }
}

pub(super) fn photographic_colour(
    ui: &mut Ui,
    photo: &mut crate::processing::PhotoSettings,
    can_pick: bool,
) -> bool {
    let pick = color(ui, ColorControl::WhiteBalance, &mut photo.wb, can_pick);
    let defaults = crate::processing::PhotoSettings::default();
    let changed = photo.contrast != defaults.contrast
        || photo.pivot != defaults.pivot
        || photo.contrast_colour_mix != defaults.contrast_colour_mix;
    let (_, reset) = section(
        ui,
        "photo-contrast",
        "Contrast",
        SectionHeader {
            summary: &format!(
                "{:.2} · Pivot {:.3} · Preserve {:.2}",
                photo.contrast, photo.pivot, photo.contrast_colour_mix
            ),
            changed,
            reset: Some(changed),
            default_open: Some(true),
        },
        |ui| {
            parameter(ui, Parameter::Contrast, &mut photo.contrast);
            parameter(ui, Parameter::Pivot, &mut photo.pivot);
            ui.small("Colour behavior");
            parameter(ui, Parameter::ColourMix, &mut photo.contrast_colour_mix);
        },
    );
    if reset && !defer_reset(ui, Reset::Contrast) {
        photo.contrast = defaults.contrast;
        photo.pivot = defaults.pivot;
        photo.contrast_colour_mix = defaults.contrast_colour_mix;
    }
    let changed = photo.saturation != defaults.saturation
        || photo.saturation_density != defaults.saturation_density;
    let (_, reset) = section(
        ui,
        "photo-saturation",
        "Saturation",
        SectionHeader {
            summary: &format!(
                "{:.2} · Density {:.2}",
                photo.saturation, photo.saturation_density
            ),
            changed,
            reset: Some(changed),
            default_open: Some(true),
        },
        |ui| {
            parameter(ui, Parameter::Saturation, &mut photo.saturation);
            ui.small("Colour behavior");
            parameter(ui, Parameter::Density, &mut photo.saturation_density);
        },
    );
    if reset && !defer_reset(ui, Reset::Saturation) {
        photo.saturation = defaults.saturation;
        photo.saturation_density = defaults.saturation_density;
    }
    pick
}

pub fn calibration(
    ui: &mut Ui,
    id: &str,
    label: &str,
    rgb: Option<DVec3>,
    enabled: bool,
    meaning: &str,
) -> bool {
    ui.push_id(id, |ui| {
        let pick = labelled_row(ui, label, Some(meaning), |ui, _| {
            let name = format!("Pick {label}");
            icons::button(ui, Icon::Pick, enabled, &name, &name).clicked()
        })
        .inner;

        if let Some(rgb) = rgb {
            ui.small(format!("R {:.6}   G {:.6}   B {:.6}", rgb.x, rgb.y, rgb.z));
        } else {
            ui.weak("Not measured");
        }
        ui.add_space(6.);
        pick
    })
    .inner
}

#[derive(Clone, Copy)]
pub enum ColorControl {
    GammaBalance,
    WhiteBalance,
}

// Resolve orientation: blue at 3 o'clock, yellow at 9; RGB/CMY proceed clockwise.
fn resolve_orientation() -> DMat2 {
    let c = 3.0_f64.sqrt() / 2.;
    DMat2::from_cols(DVec2::new(-c, 0.5), DVec2::new(0.5, c))
}

fn hue_direction(sector: f64) -> DVec2 {
    let angle = std::f64::consts::PI / 6. + sector * std::f64::consts::PI / 3.;
    resolve_orientation() * DVec2::new(angle.cos(), angle.sin())
}

impl ColorControl {
    fn matrix(self) -> DMat2 {
        let coordinate_to_rgb = match self {
            Self::GammaBalance => DMat2::IDENTITY,
            Self::WhiteBalance => {
                // Differential AP1 neutral-white response to log Bradford-cone gains.
                let cones = ap1_to_cones();
                let response = cones.inverse() * DMat3::from_diagonal(cones * DVec3::ONE);
                DMat2::from_cols(
                    opponent_coordinates(response * opponent(1., 0.)),
                    opponent_coordinates(response * opponent(0., 1.)),
                )
            }
        };
        resolve_orientation() * coordinate_to_rgb
    }

    fn scale(self) -> f64 {
        let matrix = self.matrix();
        [DVec2::new(1., 1.), DVec2::new(1., -1.)]
            .map(|direction| (matrix * direction).length())
            .into_iter()
            .fold(0., f64::max)
    }

    pub fn position(self, value: Balance) -> DVec2 {
        self.matrix() * DVec2::new(value.warmth, value.tint) / self.scale()
    }

    pub fn coordinates(self, position: DVec2) -> Balance {
        let coordinates = (self.matrix().inverse() * position * self.scale())
            .clamp(DVec2::splat(-1.), DVec2::ONE);
        Balance {
            warmth: coordinates.x,
            tint: coordinates.y,
        }
    }
}

pub fn color(ui: &mut Ui, kind: ColorControl, value: &mut Balance, can_pick: bool) -> bool {
    let changed = *value != Balance::default();
    let (pick, reset) = match kind {
        ColorControl::GammaBalance => {
            ui.push_id("Gamma balance", |ui| {
                let reset = icons::row(ui, |ui| {
                    ui.strong("Balance");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        reset_button(ui, changed, "Gamma balance").clicked()
                    })
                    .inner
                })
                .inner;
                (Some(color_inputs(ui, kind, value, can_pick)), reset)
            })
            .inner
        }
        ColorControl::WhiteBalance => {
            return white_balance(ui, "photo-white-balance", value, can_pick);
        }
    };

    if reset && !defer_reset(ui, Reset::Balance(kind)) {
        *value = Balance::default();
    }

    pick.unwrap_or(false)
}

pub fn white_balance(ui: &mut Ui, id: &str, value: &mut Balance, can_pick: bool) -> bool {
    let changed = *value != Balance::default();
    let (pick, reset) = section(
        ui,
        id,
        "White balance",
        SectionHeader {
            summary: &format!("Warmth {:+.2} · Tint {:+.2}", value.warmth, value.tint),
            changed,
            reset: Some(changed),
            default_open: Some(true),
        },
        |ui| color_inputs(ui, ColorControl::WhiteBalance, value, can_pick),
    );

    if reset && !defer_reset(ui, Reset::Balance(ColorControl::WhiteBalance)) {
        *value = Balance::default();
    }

    pick.unwrap_or(false)
}

fn color_inputs(ui: &mut Ui, kind: ColorControl, value: &mut Balance, can_pick: bool) -> bool {
    ui.horizontal(|ui| {
        ui.add_space((ui.available_width() - 140.).max(0.) / 2.);
        wheel(ui, kind, value);
    });

    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new("reset-balance-scope"), kind));
    parameter(ui, Parameter::Warmth, &mut value.warmth);
    parameter(ui, Parameter::Tint, &mut value.tint);
    ui.ctx()
        .data_mut(|data| data.remove::<ColorControl>(egui::Id::new("reset-balance-scope")));

    icons::row(ui, |ui| {
        ui.label("Neutral reference");
        icons::button(
            ui,
            Icon::Pick,
            can_pick,
            "Pick neutral…",
            "Set balance from a neutral patch.",
        )
        .clicked()
    })
    .inner
}

#[derive(Clone, Copy, Default)]
struct WheelDrag {
    pointer: egui::Pos2,
    relative: bool,
}

fn wheel(ui: &mut Ui, kind: ColorControl, value: &mut Balance) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(140., 140.), egui::Sense::click_and_drag());
    let center = rect.center();
    let radius = 52.;

    let colors = [
        Color32::RED,
        Color32::MAGENTA,
        Color32::BLUE,
        Color32::CYAN,
        Color32::GREEN,
        Color32::YELLOW,
    ];
    let names = ["R", "M", "B", "C", "G", "Y"];

    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(center, ui.visuals().extreme_bg_color);
    for index in 0..=72 {
        let sector = index as f32 / 12.;
        let direction = hue_direction(f64::from(sector));
        let direction = egui::vec2(direction.x as f32, -direction.y as f32);

        let left = colors[index / 12 % 6].to_array();
        let right = colors[(index / 12 + 1) % 6].to_array();
        let fraction = sector.fract();
        let interpolate_channel = |channel: usize| {
            (f32::from(left[channel]) * (1. - fraction) + f32::from(right[channel]) * fraction)
                .round() as u8
        };
        let mut color = Color32::from_rgb(
            interpolate_channel(0),
            interpolate_channel(1),
            interpolate_channel(2),
        );
        if !ui.is_enabled() {
            color = color.gamma_multiply(0.3);
        }

        mesh.colored_vertex(center + direction * radius, color);
        if index > 0 {
            mesh.add_triangle(0, index as u32, index as u32 + 1);
        }

        if index < 72 && index % 12 == 0 {
            ui.painter().text(
                center + direction * (radius + 10.),
                egui::Align2::CENTER_CENTER,
                names[index / 12],
                egui::FontId::proportional(10.),
                ui.visuals().weak_text_color(),
            );
        }
    }

    ui.painter().add(egui::Shape::mesh(mesh));
    ui.painter().circle_stroke(
        center,
        radius,
        egui::Stroke::new(1., ui.visuals().weak_text_color()),
    );

    let drag_id = response.id.with("wheel-drag");
    let mut drag = ui
        .ctx()
        .data_mut(|data| data.remove_temp::<WheelDrag>(drag_id));
    let pointer_down = response.enabled() && response.is_pointer_button_down_on();

    if response.enabled()
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let (shift, pressed, origin) = ui.input(|input| {
            (
                input.modifiers.shift,
                input.pointer.any_pressed(),
                input.pointer.press_origin(),
            )
        });

        if pointer_down && (pressed || drag.is_none()) {
            drag = Some(WheelDrag {
                pointer: origin.unwrap_or(pointer),
                relative: shift,
            });
        }
        if pointer_down {
            // Keep motion relative after using precision so releasing Shift cannot snap to the pointer.
            drag.as_mut().unwrap().relative |= shift;
        }

        if let Some(state) = &drag
            && state.relative
        {
            if response.dragged() || response.drag_stopped() {
                let delta = (pointer - state.pointer) / radius;
                if delta != egui::Vec2::ZERO {
                    let delta = DVec2::new(f64::from(delta.x), -f64::from(delta.y));
                    *value = kind
                        .coordinates(kind.position(*value) + delta * if shift { 0.1 } else { 1. });
                }
            }
        } else if (response.dragged() || response.clicked()) && !shift {
            let position = (pointer - center) / radius;
            *value = kind.coordinates(DVec2::new(f64::from(position.x), -f64::from(position.y)));
        }

        if pointer_down && response.dragged() {
            drag.as_mut().unwrap().pointer = pointer;
        }
    }

    if pointer_down && let Some(drag) = drag {
        ui.ctx().data_mut(|data| data.insert_temp(drag_id, drag));
    }

    let position = kind.position(*value);
    ui.painter().circle_filled(
        center + egui::vec2(position.x as f32, -position.y as f32) * radius,
        4.,
        ui.visuals().text_color(),
    );
    history_gesture(ui, &response);
    response.on_hover_text("Shift-drag for finer adjustment.")
}

#[cfg(test)]
#[path = "editor_controls_tests.rs"]
mod tests;
