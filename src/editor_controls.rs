use super::{
    icons::{self, Icon},
    presentation::{
        INSPECTOR_ROW_HEIGHT, PARAMETER_GAP, PARAMETER_NUMERIC_WIDTH, PARAMETER_SLIDER_HEIGHT,
        PARAMETER_UNIT_WIDTH, SectionHeader, SectionTool, ToolIdentity, labelled_row_at, section,
    },
};
use crate::adjustment_layers::{DEFAULT_COLOUR_MIX, DEFAULT_PIVOT};
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
    FilmAmount,
    FilmSize,
    FilmColour,
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
            Self::ColourMix => "Preserve",
            Self::Saturation => "Saturation",
            Self::Density => "Density",
            Self::CropMargin => "Margin",
            Self::Strength => "Strength",
            Self::Feather => "Feather",
            Self::FilmAmount => "Amount",
            Self::FilmSize => "Size",
            Self::FilmColour => "Colour",
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
            Self::FilmAmount => {
                crate::film_emulation::MIN_AMOUNT..=crate::film_emulation::MAX_AMOUNT
            }
            Self::FilmSize => crate::film_emulation::MIN_SIZE..=crate::film_emulation::MAX_SIZE,
            Self::FilmColour => {
                crate::film_emulation::MIN_COLOUR..=crate::film_emulation::MAX_COLOUR
            }
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
            Self::FilmAmount => crate::film_emulation::DEFAULT_AMOUNT,
            Self::FilmSize => crate::film_emulation::DEFAULT_SIZE,
            Self::FilmColour => crate::film_emulation::DEFAULT_COLOUR,
            _ => 0.,
        }
    }

    fn step(self) -> f64 {
        match self {
            Self::Exposure => 0.05,
            Self::Shoulder => 0.0001,
            Self::Pivot => 0.001,
            Self::FilmSize => 0.001,
            Self::FilmAmount => 0.001,
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

/// Frame-local activity, independent of the history group retained for a completed drag.
pub(super) fn values_are_being_adjusted(context: &egui::Context) -> bool {
    context.data(|data| data.get_temp::<u64>(egui::Id::new("value-adjustment-frame")))
        == Some(context.cumulative_frame_nr())
}

fn record_control_interaction(ui: &Ui, response: &Response) {
    if response.enabled()
        && (response.is_pointer_button_down_on() || response.dragged() || response.drag_stopped())
    {
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::new("project-control-gesture"), response.id);
        });
    }
    let keyboard = response.has_focus()
        && (ui.ctx().text_edit_focused()
            || ui.input(|input| {
                [
                    egui::Key::ArrowLeft,
                    egui::Key::ArrowRight,
                    egui::Key::ArrowUp,
                    egui::Key::ArrowDown,
                ]
                .into_iter()
                .any(|key| input.key_down(key))
            }));
    if response.enabled()
        && (response.is_pointer_button_down_on() || response.dragged() || keyboard)
    {
        let frame = ui.ctx().cumulative_frame_nr();
        ui.ctx()
            .data_mut(|data| data.insert_temp(egui::Id::new("value-adjustment-frame"), frame));
    }
}

fn parameter_help(kind: Parameter) -> &'static str {
    match kind {
        Parameter::Contrast => "Adjust contrast around Pivot; 1 leaves contrast unchanged.",
        Parameter::Pivot => "Set the brightness that contrast pivots around.",
        Parameter::ColourMix => "Reduce colour shifts caused by contrast.",
        Parameter::Saturation => "0 makes the image grey; 1 leaves saturation unchanged.",
        Parameter::Density => "Higher values darken colours more as saturation increases.",
        Parameter::CropMargin => {
            "Trim each edge by a percentage of the detected frame's shorter side."
        }
        Parameter::FilmAmount => "Relative grain strength at original resolution; 0 removes grain.",
        Parameter::FilmSize => "Grain scale as a percentage of the source frame height.",
        Parameter::FilmColour => {
            "0 is monochrome; 0.5 matches the reference colour grain; 1 increases chroma."
        }
        Parameter::Strength => "Scale the adjustment from no effect to full strength.",
        Parameter::Feather => "Soften the oval's edge; 0 is hard, 1 fades across the full radius.",
        Parameter::Shoulder => "Highlight roll-off; 0 turns the shoulder off.",
        _ => "",
    }
}

fn parameter_unit(kind: Parameter) -> Option<&'static str> {
    match kind {
        Parameter::Exposure => Some("EV"),
        Parameter::CropMargin | Parameter::FilmSize => Some("%"),
        _ => None,
    }
}

/// The fixed name column shared by every parameter row, so tracks line up across sections.
fn parameter_name(ui: &mut Ui, kind: Parameter) -> Response {
    let label = super::presentation::name_label(ui, kind.label());
    if let Some(help) = (!parameter_help(kind).is_empty()).then_some(parameter_help(kind)) {
        label.clone().on_hover_text(help);
    }
    label
}

fn reset_parameter(ui: &Ui, kind: Parameter, value: &mut f64) {
    let balance = ui
        .ctx()
        .data(|data| data.get_temp::<ColorControl>(egui::Id::new("reset-balance-scope")))
        .unwrap_or(ColorControl::WhiteBalance);
    if !defer_reset(ui, Reset::Parameter(kind, balance)) {
        *value = kind.default();
    }
}

/// The name is a hover-only label, so reading the pointer avoids making every name a tab stop.
fn name_reset_gesture(ui: &Ui, label: &Response) -> bool {
    ui.input(|input| {
        let over_name = input
            .pointer
            .interact_pos()
            .is_some_and(|position| label.rect.contains(position));
        over_name
            && (input
                .pointer
                .button_double_clicked(egui::PointerButton::Primary)
                || (input.modifiers.alt && input.pointer.any_click()))
    })
}

/// The track: absolute drag normally, slow relative travel when the press began with Shift.
fn track(ui: &mut Ui, kind: Parameter, value: &mut f64, width: f32, name: egui::Id) -> Response {
    let step = kind.step()
        * if ui.input(|input| input.modifiers.shift) {
            0.1
        } else {
            1.
        };
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = width;
        ui.spacing_mut().interact_size.y = PARAMETER_SLIDER_HEIGHT;
        let drag_id = ui.make_persistent_id("fine-drag");
        let mut fine = ui
            .ctx()
            .data_mut(|data| data.remove_temp::<FineDrag>(drag_id));
        let shift = ui.input(|input| input.modifiers.shift);
        let range = kind.slider_range();
        let before = *value;
        let mut shown = before;
        let response = ui
            .add(
                egui::Slider::new(&mut shown, range.clone())
                    .clamping(egui::SliderClamping::Edits)
                    .show_value(false)
                    .logarithmic(matches!(kind, Parameter::Pivot))
                    .smart_aim(false)
                    .step_by(step),
            )
            .on_hover_text(if matches!(kind, Parameter::Exposure) {
                "Slider: ±3 EV; type a value for up to ±16 EV. Shift-drag for finer steps."
            } else {
                "Shift-drag for finer steps."
            });
        let pointer_down = ui.is_enabled() && response.is_pointer_button_down_on();
        if fine.is_some() && !pointer_down && !response.drag_stopped() {
            // A gesture interrupted by disabling or hiding the row must not hold the value
            // back to its pre-drag snapshot when the row reappears.
            fine = None;
        }
        if response.drag_started() || (shift && pointer_down && fine.is_none()) {
            // The press itself holds the accepted value, so precision never jumps.
            fine = Some(FineDrag {
                start: before,
                travel: 0.,
                fine: shift,
            });
        }
        match fine.as_mut() {
            Some(state) if state.fine => {
                if response.dragged() {
                    state.travel += response.drag_delta().x;
                }
                // One tenth of a full-width linear drag; logarithmic tracks scale the value,
                // so their per-pixel travel differs at either end.
                let span = range.end() - range.start();
                let per_pixel = span / f64::from(width);
                *value = (state.start + f64::from(state.travel) * per_pixel * 0.1)
                    .clamp(*range.start(), *range.end());
            }
            _ => *value = shown,
        }
        if response.drag_stopped() {
            fine = None;
        }
        if pointer_down && let Some(state) = fine {
            ui.ctx().data_mut(|data| data.insert_temp(drag_id, state));
        }
        response
    })
    .inner
    .labelled_by(name)
}

fn numeric_field(ui: &mut Ui, kind: Parameter, value: &mut f64, height: f32) -> Response {
    let previous = *value;
    let decimals = match kind {
        Parameter::Shoulder => 4,
        Parameter::Pivot => 3,
        Parameter::FilmAmount | Parameter::FilmSize => 3,
        _ => 2,
    };
    let numeric = ui
        .push_id("numeric", |ui| {
            ui.add_sized(
                [PARAMETER_NUMERIC_WIDTH, height],
                egui::DragValue::new(value)
                    .speed(match kind {
                        Parameter::Shoulder => 0.0001,
                        Parameter::Pivot => 0.001,
                        Parameter::FilmAmount | Parameter::FilmSize => 0.001,
                        _ => 0.01,
                    })
                    .range(kind.range())
                    .clamp_existing_to_range(false)
                    .min_decimals(2)
                    .max_decimals(decimals)
                    .update_while_editing(false),
            )
        })
        .inner;
    if numeric_cancelled(ui, &numeric) {
        *value = previous;
    } else if numeric.lost_focus() {
        // Focus alone rewrites the stored value with its formatted text; keep the accepted
        // value when the only thing that changed is that display rounding.
        let factor = 10f64.powi(decimals as i32);
        let displayed = (previous * factor).round() / factor;
        if *value == displayed && previous != displayed {
            *value = previous;
        }
    }
    numeric
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
        let unit = parameter_unit(kind);
        let (slider, numeric, label) = icons::row_at(ui, INSPECTOR_ROW_HEIGHT, |ui| {
            let label = parameter_name(ui, kind);
            if name_reset_gesture(ui, &label) {
                reset_parameter(ui, kind, value);
            }
            // Reserve one unit column for every row, including rows without a unit, so the
            // numeric fields and slider ends align across parameters.
            let reserved =
                PARAMETER_NUMERIC_WIDTH + 3. * ui.spacing().item_spacing.x + PARAMETER_UNIT_WIDTH;
            let width = (ui.available_width() - reserved).max(48.);
            let slider = track(ui, kind, value, width, label.id);
            let numeric = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_sized(
                        [PARAMETER_UNIT_WIDTH, INSPECTOR_ROW_HEIGHT - 2.],
                        egui::Label::new(unit.unwrap_or("")),
                    );
                    let numeric = numeric_field(ui, kind, value, INSPECTOR_ROW_HEIGHT - 2.)
                        .labelled_by(label.id);
                    numeric.clone().context_menu(|ui| {
                        if ui.button("Reset to default").clicked() {
                            reset_parameter(ui, kind, value);
                            ui.close();
                        }
                    });
                    numeric
                })
                .inner;
            (slider, numeric, label)
        })
        .inner;

        record_control_interaction(ui, &slider);
        record_control_interaction(ui, &numeric);
        ui.add_space(PARAMETER_GAP);
        (slider, numeric, label)
    })
    .inner
}

pub(super) fn photographic_exposure(
    ui: &mut Ui,
    photo: &mut crate::adjustment_layers::AdjustmentLayerSettings,
) {
    let (_, action) = section(
        ui,
        "photo-exposure",
        "Exposure",
        SectionHeader {
            summary: &format!("{:+.2} EV", photo.exposure),
            changed: photo.exposure != 0.,
            reset: Some(photo.exposure != 0.),
            tool: None,
            default_open: Some(true),
        },
        |ui| {
            parameter(ui, Parameter::Exposure, &mut photo.exposure);
        },
    );
    if action.reset && !defer_reset(ui, Reset::Exposure) {
        photo.exposure = 0.;
    }
}

pub(super) fn photographic_colour(
    ui: &mut Ui,
    photo: &mut crate::adjustment_layers::AdjustmentLayerSettings,
    can_pick: bool,
) -> bool {
    let pick = white_balance(ui, "photo-white-balance", &mut photo.wb, can_pick);
    let defaults = crate::adjustment_layers::AdjustmentLayerSettings::default();
    let changed = photo.contrast != defaults.contrast
        || photo.pivot != defaults.pivot
        || photo.contrast_colour_mix != defaults.contrast_colour_mix;
    let (_, action) = section(
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
            tool: None,
            default_open: Some(true),
        },
        |ui| {
            parameter(ui, Parameter::Contrast, &mut photo.contrast);
            parameter(ui, Parameter::Pivot, &mut photo.pivot);
            parameter(ui, Parameter::ColourMix, &mut photo.contrast_colour_mix);
        },
    );
    if action.reset && !defer_reset(ui, Reset::Contrast) {
        photo.contrast = defaults.contrast;
        photo.pivot = defaults.pivot;
        photo.contrast_colour_mix = defaults.contrast_colour_mix;
    }
    let changed = photo.saturation != defaults.saturation
        || photo.saturation_density != defaults.saturation_density;
    let (_, action) = section(
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
            tool: None,
            default_open: Some(true),
        },
        |ui| {
            parameter(ui, Parameter::Saturation, &mut photo.saturation);
            parameter(ui, Parameter::Density, &mut photo.saturation_density);
        },
    );
    if action.reset && !defer_reset(ui, Reset::Saturation) {
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
        let pick = labelled_row_at(ui, INSPECTOR_ROW_HEIGHT, label, Some(meaning), |ui, _| {
            let name = format!("Pick {label}");
            icons::button_at(ui, Icon::Pick, enabled, &name, &name, INSPECTOR_ROW_HEIGHT).clicked()
        })
        .inner;

        if let Some(rgb) = rgb {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "R {:.6}   G {:.6}   B {:.6}",
                        rgb.x, rgb.y, rgb.z
                    ))
                    .small(),
                )
                .truncate(),
            );
        } else {
            ui.weak("Not measured");
        }
        ui.add_space(PARAMETER_GAP);
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

/// Shared balance body: a compact hue wheel with the two axis controls beside it.
///
/// The wheel is a correction surface, not a measurement device; its radius follows the panel
/// width and the axis sliders keep the fine, single-axis work.
pub(super) fn balance_body(ui: &mut Ui, kind: ColorControl, value: &mut Balance) {
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::new("reset-balance-scope"), kind));
    ui.horizontal(|ui| {
        let size = wheel_size(ui.available_width());
        // Reserve exactly one inter-item gap for the wheel so the row never widens the panel.
        let column = (ui.available_width() - size - ui.spacing().item_spacing.x).max(120.);
        ui.vertical(|ui| {
            ui.set_width(column);
            parameter(ui, Parameter::Warmth, &mut value.warmth);
            parameter(ui, Parameter::Tint, &mut value.tint);
        });
        wheel(ui, kind, value, size);
    });
    ui.ctx()
        .data_mut(|data| data.remove::<ColorControl>(egui::Id::new("reset-balance-scope")));
}

pub(super) fn balance_pick_tool(can_pick: bool, subject: &'static str) -> SectionTool<'static> {
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

pub fn white_balance(ui: &mut Ui, id: &str, value: &mut Balance, can_pick: bool) -> bool {
    let changed = *value != Balance::default();
    let (_, action) = section(
        ui,
        id,
        "White balance",
        SectionHeader {
            summary: &format!("Warmth {:+.2} · Tint {:+.2}", value.warmth, value.tint),
            changed,
            reset: Some(changed),
            tool: Some(balance_pick_tool(
                can_pick,
                "Pick neutral for white balance",
            )),
            default_open: Some(true),
        },
        |ui| balance_body(ui, ColorControl::WhiteBalance, value),
    );

    if action.reset && !defer_reset(ui, Reset::Balance(ColorControl::WhiteBalance)) {
        *value = Balance::default();
    }

    action.tool
}

/// A track drag that started with Shift held; travel is applied relatively at one-tenth rate.
#[derive(Clone, Copy, Default)]
struct FineDrag {
    start: f64,
    travel: f32,
    fine: bool,
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

    if response.double_clicked() && !defer_reset(ui, Reset::Balance(kind)) {
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

#[cfg(test)]
#[path = "editor_controls_tests.rs"]
mod tests;
