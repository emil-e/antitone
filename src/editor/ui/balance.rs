use super::{
    ControlResponse, ParameterRow, SectionTool,
    icons::Icon,
    interaction::{control_active, record_control_interaction},
};
use crate::color::{Balance, ap1_to_cones, opponent, opponent_coordinates};
use egui::{Color32, Response, Ui};
use glam::{DMat2, DMat3, DVec2, DVec3};

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

pub(in crate::editor) fn balance_pick_tool(
    can_pick: bool,
    subject: &'static str,
) -> SectionTool<'static> {
    SectionTool {
        icon: Icon::Pick,
        enabled: can_pick,
        name: subject,
        help: "Set balance from a neutral patch.",
    }
}

fn wheel_size(available: f32) -> f32 {
    (available * 0.22).clamp(60., 76.)
}

#[derive(Clone, Copy, Default)]
struct WheelDrag {
    /// Pointer position of the last applied motion; only new travel becomes a delta, so an
    /// unchanged pointer in a repeated layout pass is not applied twice.
    pointer: egui::Pos2,
}

/// Hue ring with a position marker; always dragged relatively so radius only affects reach.
fn wheel(ui: &mut Ui, kind: ColorControl, value: &mut Balance, size: f32) -> Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(size), egui::Sense::click_and_drag());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Image,
            response.enabled(),
            match kind {
                ColorControl::GammaBalance => "Gamma balance wheel",
                ColorControl::WhiteBalance => "White balance wheel",
            },
        )
    });
    let center = rect.center();
    let radius = size * 0.5 - 2.;

    let colors = [
        Color32::RED,
        Color32::MAGENTA,
        Color32::BLUE,
        Color32::CYAN,
        Color32::GREEN,
        Color32::YELLOW,
    ];

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
            });
        }

        if let Some(state) = &drag
            && (response.dragged() || response.drag_stopped())
        {
            let delta = (pointer - state.pointer) / radius;
            if delta != egui::Vec2::ZERO {
                let delta = DVec2::new(f64::from(delta.x), -f64::from(delta.y));
                *value =
                    kind.coordinates(kind.position(*value) + delta * if shift { 0.1 } else { 1. });
            }
        }

        if pointer_down
            && response.dragged()
            && let Some(state) = &mut drag
        {
            state.pointer = pointer;
        }
    }

    if pointer_down && let Some(drag) = drag {
        ui.ctx().data_mut(|data| data.insert_temp(drag_id, drag));
    }

    if response.double_clicked() {
        *value = Balance::default();
    }

    let position = kind.position(*value);
    ui.painter().circle_filled(
        center + egui::vec2(position.x as f32, -position.y as f32) * radius,
        4.,
        ui.visuals().text_color(),
    );
    record_control_interaction(ui, &response);
    response.on_hover_text(
        "Drag to shift the balance; hold Shift for finer motion. Double-click resets.",
    )
}

/// Two numeric axes and a relative colour wheel, composed with ordinary egui layout.
pub(in crate::editor) struct BalanceControl {
    id: egui::Id,
    kind: ColorControl,
}

impl BalanceControl {
    pub fn new(id: impl egui::AsId, kind: ColorControl) -> Self {
        Self {
            id: egui::Id::new(id),
            kind,
        }
    }

    pub fn show(self, ui: &mut Ui, value: &mut Balance) -> [ControlResponse; 3] {
        ui.push_id(self.id, |ui| {
            ui.horizontal(|ui| {
                let size = wheel_size(ui.available_width());
                let column = (ui.available_width() - size - ui.spacing().item_spacing.x).max(120.);
                let axes = ui
                    .vertical(|ui| {
                        ui.set_width(column);
                        let warmth = ParameterRow::new("warmth", "Warmth", -1.0..=1.0)
                            .default_value(0.)
                            .show(ui, &mut value.warmth);
                        let tint = ParameterRow::new("tint", "Tint", -1.0..=1.0)
                            .default_value(0.)
                            .show(ui, &mut value.tint);
                        [warmth.edit, tint.edit]
                    })
                    .inner;
                let before = *value;
                let response = wheel(ui, self.kind, value, size);
                let wheel = ControlResponse {
                    id: response.id,
                    active: control_active(ui, &response),
                    enabled: response.enabled(),
                    cancelled: false,
                    changed: *value != before,
                };
                [axes[0], axes[1], wheel]
            })
            .inner
        })
        .inner
    }
}

#[cfg(test)]
mod tests;
