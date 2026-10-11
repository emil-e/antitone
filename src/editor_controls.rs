use crate::{
    color::{Balance, ap1_to_cones, opponent, opponent_coordinates},
    processing::{
        DEFAULT_GAMMA, DEFAULT_TOE, MAX_EXPOSURE, MAX_GAMMA, MAX_SHOULDER, MAX_TOE, MIN_GAMMA,
        MIN_TOE,
    },
};
use egui::{Color32, Response, Ui};
use glam::{DMat2, DMat3, DVec2, DVec3};
use std::ops::RangeInclusive;

pub fn section<R>(
    ui: &mut Ui,
    id: &str,
    title: &str,
    reset_enabled: Option<bool>,
    contents: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    ui.push_id(id, |ui| {
        ui.add_space(8.);
        egui::Frame::group(ui.style())
            .inner_margin(10)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let mut reset = false;
                ui.horizontal(|ui| {
                    ui.strong(title);
                    if let Some(enabled) = reset_enabled {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            reset = reset_button(ui, enabled, title).clicked();
                        });
                    }
                });
                ui.add_space(6.);
                (contents(ui), reset)
            })
            .inner
    })
    .inner
}

fn reset_button(ui: &mut Ui, enabled: bool, name: &str) -> Response {
    ui.add_enabled(enabled, egui::Button::new("↺").small())
        .on_hover_text(format!("Reset {name}"))
}

#[derive(Clone, Copy)]
pub enum Parameter {
    Gamma,
    Toe,
    Shoulder,
    Exposure,
    Warmth,
    Tint,
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
        }
    }
    fn range(self) -> RangeInclusive<f64> {
        match self {
            Self::Gamma => MIN_GAMMA..=MAX_GAMMA,
            Self::Toe => MIN_TOE..=MAX_TOE,
            Self::Shoulder => 0.0..=MAX_SHOULDER,
            Self::Exposure => -MAX_EXPOSURE..=MAX_EXPOSURE,
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
    fn default(self) -> f64 {
        match self {
            Self::Gamma => DEFAULT_GAMMA,
            Self::Toe => DEFAULT_TOE,
            _ => 0.,
        }
    }
    fn step(self) -> f64 {
        match self {
            Self::Exposure => 0.05,
            Self::Shoulder => 0.0001,
            _ => 0.01,
        }
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
        let (numeric, reset) = ui.horizontal(|ui| {
            ui.label(kind.label());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let reset = reset_button(ui, *value != kind.default(), kind.label());
                if reset.clicked() {
                    *value = kind.default();
                }
                if matches!(kind, Parameter::Exposure) {
                    ui.label("EV");
                }
                let numeric = ui.push_id("numeric", |ui| {
                    ui.add_sized(
                        [85., ui.spacing().interact_size.y],
                        egui::DragValue::new(value)
                            .speed(if matches!(kind, Parameter::Shoulder) { 0.0001 } else { 0.01 })
                            .range(kind.range())
                            .clamp_existing_to_range(false)
                            .min_decimals(2)
                            .max_decimals(6)
                            .update_while_editing(false),
                    ).on_hover_text(
                        "Click to enter a value; drag to adjust. Shift-drag for fine adjustment. Enter commits; Escape cancels.",
                    )
                }).inner;
                (numeric, reset)
            }).inner
        }).inner;
        let step = kind.step() * if ui.input(|i| i.modifiers.shift) { 0.1 } else { 1. };
        let slider = ui.scope(|ui| {
            ui.spacing_mut().slider_width = ui.available_width();
            ui.add(
                egui::Slider::new(value, kind.slider_range())
                    .clamping(egui::SliderClamping::Edits)
                    .show_value(false)
                    .smart_aim(false)
                    .step_by(step),
            ).on_hover_text(if matches!(kind, Parameter::Exposure) {
                "Slider: ±3 EV; numeric entry: ±16 EV. Arrow keys adjust; Shift uses finer steps."
            } else {
                "Arrow keys adjust the focused slider. Hold Shift for finer steps."
            })
        }).inner;
        ui.add_space(6.);
        (slider, numeric, reset)
    }).inner
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
        let mut pick = false;
        ui.horizontal(|ui| {
            ui.label(label).on_hover_text(meaning);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                pick = ui
                    .add_enabled(enabled, egui::Button::new("Pick…").small())
                    .clicked();
            });
        });
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
            .map(|v| (matrix * v).length())
            .into_iter()
            .fold(0., f64::max)
    }
    pub fn position(self, value: Balance) -> DVec2 {
        self.matrix() * DVec2::new(value.warmth, value.tint) / self.scale()
    }
    pub fn coordinates(self, position: DVec2) -> Balance {
        let value = (self.matrix().inverse() * position * self.scale())
            .clamp(DVec2::splat(-1.), DVec2::ONE);
        Balance {
            warmth: value.x,
            tint: value.y,
        }
    }
    fn tooltip(self) -> &'static str {
        match self {
            Self::GammaBalance => {
                "RGB directions represent relative native-camera log gamma. Image response depends on the negative and its calibration; this is not a saturation or RGB-gain control."
            }
            Self::WhiteBalance => {
                "RGB directions represent the local AP1 neutral-white response to Bradford WB. Warmth/tint remain cone-gain coordinates, not RGB channel gains or physical Kelvin."
            }
        }
    }
}

pub fn color(ui: &mut Ui, kind: ColorControl, value: &mut Balance, can_pick: bool) -> bool {
    let title = match kind {
        ColorControl::GammaBalance => "Gamma balance",
        ColorControl::WhiteBalance => "White balance",
    };
    let (pick, reset) = section(ui, title, title, Some(*value != Balance::default()), |ui| {
        ui.horizontal(|ui| {
            ui.add_space((ui.available_width() - 140.).max(0.) / 2.);
            wheel(ui, kind, value);
        });
        parameter(ui, Parameter::Warmth, &mut value.warmth);
        parameter(ui, Parameter::Tint, &mut value.tint);
        ui.add_enabled(can_pick, egui::Button::new("Pick neutral…"))
            .on_hover_text("Measure one full-resolution patch. The fitted correction replaces these same warmth/tint values.").clicked()
    });
    if reset {
        *value = Balance::default();
    }
    pick
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
        let t = sector.fract();
        let channel =
            |c: usize| (f32::from(left[c]) * (1. - t) + f32::from(right[c]) * t).round() as u8;
        let mut color = Color32::from_rgb(channel(0), channel(1), channel(2));
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
    if (response.dragged() || response.clicked())
        && let Some(pointer) = response.interact_pointer_pos()
    {
        let position = (pointer - center) / radius;
        *value = kind.coordinates(DVec2::new(f64::from(position.x), -f64::from(position.y)));
    }
    let position = kind.position(*value);
    ui.painter().circle_filled(
        center + egui::vec2(position.x as f32, -position.y as f32) * radius,
        4.,
        ui.visuals().text_color(),
    );
    response.on_hover_text(kind.tooltip())
}

#[cfg(test)]
#[path = "editor_controls_tests.rs"]
mod tests;
