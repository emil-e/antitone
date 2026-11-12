use super::{
    ControlResponse, INSPECTOR_ROW_HEIGHT, PARAMETER_GAP, PARAMETER_NUMERIC_WIDTH,
    PARAMETER_SLIDER_HEIGHT, PARAMETER_UNIT_WIDTH, icons,
    interaction::{control_active, record_control_interaction},
    numeric_cancelled,
};
use egui::{Response, Ui};
use std::ops::RangeInclusive;

/// An inspector value row. Identity is independent of the displayed name.
/// Features supply value semantics; spacing and gestures belong to the component.
#[derive(Clone)]
pub(in crate::editor) struct ParameterRow<'a> {
    id: egui::Id,
    label: &'a str,
    range: RangeInclusive<f64>,
    slider_range: Option<RangeInclusive<f64>>,
    default_value: Option<f64>,
    step: f64,
    speed: Option<f64>,
    decimals: usize,
    logarithmic: bool,
    unit: Option<&'a str>,
    help: Option<&'a str>,
    slider_help: Option<&'a str>,
}

pub(in crate::editor) struct ParameterResponse {
    pub slider: Response,
    pub numeric: Response,
    pub label: Response,
    pub edit: ControlResponse,
}

impl<'a> ParameterRow<'a> {
    pub fn new(id: impl egui::AsId, label: &'a str, range: RangeInclusive<f64>) -> Self {
        Self {
            id: egui::Id::new(id),
            label,
            range,
            slider_range: None,
            default_value: None,
            step: 0.01,
            speed: None,
            decimals: 2,
            logarithmic: false,
            unit: None,
            help: None,
            slider_help: None,
        }
    }

    /// Enables name gestures and the numeric context-menu reset.
    pub fn default_value(mut self, value: f64) -> Self {
        self.default_value = Some(value);
        self
    }

    pub fn slider_range(mut self, range: RangeInclusive<f64>) -> Self {
        self.slider_range = Some(range);
        self
    }

    /// Keyboard/track increment and numeric drag speed, unless overridden with `speed`.
    pub fn step(mut self, step: f64) -> Self {
        self.step = step;
        self
    }

    pub fn speed(mut self, speed: f64) -> Self {
        self.speed = Some(speed);
        self
    }

    pub fn decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    pub fn logarithmic(mut self, logarithmic: bool) -> Self {
        self.logarithmic = logarithmic;
        self
    }

    pub fn unit(mut self, unit: &'a str) -> Self {
        self.unit = Some(unit);
        self
    }

    pub fn help(mut self, help: &'a str) -> Self {
        self.help = Some(help);
        self
    }

    pub fn slider_help(mut self, help: &'a str) -> Self {
        self.slider_help = Some(help);
        self
    }

    pub fn show(self, ui: &mut Ui, value: &mut f64) -> ParameterResponse {
        let mut response = parameter_row(ui, &self, value);
        response.slider = response.slider.labelled_by(response.label.id);
        response.numeric = response.numeric.labelled_by(response.label.id);
        response
    }
}

#[cfg(test)]
mod tests;

/// A track drag retains its press value and whether it began with Shift.
#[derive(Clone, Copy, Default)]
struct FineDrag {
    start: f64,
    travel: f32,
    fine: bool,
}

/// The fixed name column shared by every parameter row, so tracks line up across sections.
fn parameter_name(ui: &mut Ui, row: &ParameterRow<'_>) -> Response {
    let label = super::name_label(ui, row.label);
    if let Some(help) = row.help {
        label.clone().on_hover_text(help);
    }
    label
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
fn track(ui: &mut Ui, row: &ParameterRow<'_>, value: &mut f64, width: f32) -> Response {
    let step = row.step
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
        let range = row
            .slider_range
            .clone()
            .unwrap_or_else(|| row.range.clone());
        let before = *value;
        let mut shown = before;
        let response = ui
            .add(
                egui::Slider::new(&mut shown, range.clone())
                    .clamping(egui::SliderClamping::Edits)
                    .show_value(false)
                    .logarithmic(row.logarithmic)
                    .smart_aim(false)
                    .step_by(step),
            )
            .on_hover_text(row.slider_help.unwrap_or("Shift-drag for finer steps."));
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
                // Fine motion must retain typed values beyond the slider's normal travel.
                let bounds = row.range.clone();
                *value = (state.start + f64::from(state.travel) * per_pixel * 0.1)
                    .clamp(*bounds.start(), *bounds.end());
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
}

fn numeric_field(
    ui: &mut Ui,
    row: &ParameterRow<'_>,
    value: &mut f64,
    height: f32,
) -> (Response, bool) {
    let previous = *value;
    let decimals = row.decimals;
    let numeric = ui
        .push_id("numeric", |ui| {
            ui.add_sized(
                [PARAMETER_NUMERIC_WIDTH, height],
                egui::DragValue::new(value)
                    .speed(row.speed.unwrap_or(row.step))
                    .range(row.range.clone())
                    .clamp_existing_to_range(false)
                    .min_decimals(2.min(decimals))
                    .max_decimals(decimals)
                    .update_while_editing(false),
            )
        })
        .inner;
    let cancelled = numeric_cancelled(ui, &numeric);
    if cancelled {
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
    (numeric, cancelled)
}

fn parameter_row(ui: &mut Ui, row: &ParameterRow<'_>, value: &mut f64) -> ParameterResponse {
    ui.push_id(row.id, |ui| {
        let previous = *value;
        let mut reset = false;
        let unit = row.unit;
        let (slider, numeric, label, cancelled, slider_value, numeric_value) =
            icons::row_at(ui, INSPECTOR_ROW_HEIGHT, |ui| {
                let label = parameter_name(ui, row);
                reset = row.default_value.is_some() && name_reset_gesture(ui, &label);
                let reserved = PARAMETER_NUMERIC_WIDTH
                    + 3. * ui.spacing().item_spacing.x
                    + PARAMETER_UNIT_WIDTH;
                let width = (ui.available_width() - reserved).max(48.);
                let mut slider_value = previous;
                let slider = ui
                    .push_id("track", |ui| track(ui, row, &mut slider_value, width))
                    .inner;
                let mut numeric_value = previous;
                let (numeric, cancelled) = ui
                    .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_sized(
                            [PARAMETER_UNIT_WIDTH, INSPECTOR_ROW_HEIGHT - 2.],
                            egui::Label::new(unit.unwrap_or("")),
                        );
                        let (numeric, cancelled) = ui
                            .push_id("number", |ui| {
                                numeric_field(
                                    ui,
                                    row,
                                    &mut numeric_value,
                                    INSPECTOR_ROW_HEIGHT - 2.,
                                )
                            })
                            .inner;
                        if row.default_value.is_some() {
                            numeric.clone().context_menu(|ui| {
                                if ui.button("Reset to default").clicked() {
                                    reset = true;
                                    ui.close();
                                }
                            });
                        }
                        (numeric, cancelled)
                    })
                    .inner;
                (
                    slider,
                    numeric,
                    label,
                    cancelled,
                    slider_value,
                    numeric_value,
                )
            })
            .inner;

        let number = numeric.has_focus()
            || numeric.lost_focus()
            || numeric.is_pointer_button_down_on()
            || numeric.dragged()
            || numeric.drag_stopped()
            || cancelled
            || numeric_value != previous;
        let selected = if number { &numeric } else { &slider };
        let mut response = ControlResponse {
            id: selected.id,
            active: control_active(ui, selected),
            enabled: selected.enabled(),
            cancelled: number && cancelled,
            changed: false,
        };
        if response.enabled && !response.cancelled {
            *value = if number { numeric_value } else { slider_value };
            response.changed = *value != previous
                || (!number
                    && slider.is_pointer_button_down_on()
                    && !ui.input(|input| input.modifiers.shift));
        }
        if reset
            && ui.is_enabled()
            && let Some(default) = row.default_value
        {
            *value = default;
            response = ControlResponse::action(ui.id().with("reset"), true);
        }
        record_control_interaction(ui, &slider);
        record_control_interaction(ui, &numeric);
        ui.add_space(PARAMETER_GAP);
        ParameterResponse {
            slider,
            numeric,
            label,
            edit: response,
        }
    })
    .inner
}
