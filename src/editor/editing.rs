use super::picking::Display;
use super::*;
use crate::processing::InputSettings;

impl Pick {
    pub(super) fn display(self) -> Display {
        match self {
            Self::Dmin => calibration::DMIN,
            Self::Dmax => calibration::DMAX,
            Self::Gamma => calibration::GAMMA,
            Self::RollWb => white_balance::ROLL,
            Self::Wb => white_balance::LAYER,
            Self::Flat => flat::DISPLAY,
            Self::Analysis => analysis::AREA_DISPLAY,
        }
    }
}

impl Editor {
    pub(super) fn finish_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.document.analysis_review.is_some() {
            self.status = "Analysis review cancelled; areas and grade unchanged".into();
            self.end_analysis_review(cx);
        } else if self.document.flat_edit.is_some() {
            self.finish_flat_edit();
        } else {
            self.cancel_picker();
        }
    }

    pub(super) fn prepare_pick(&mut self, cx: &UiContext<'_, Self>) -> bool {
        self.finish_dust_cleanup();
        if self.active_pick().is_some() {
            self.finish_pick(cx);
        }
        if self.busy
            || self.document.visit.crop_edit.is_some()
            || self.document.auto_crop_edit.is_some()
            || self.document.visit.mask_edit.is_some()
        {
            return false;
        }
        true
    }

    pub(super) fn begin_pick(&mut self, kind: Pick, cx: &UiContext<'_, Self>) {
        if !self.prepare_pick(cx) {
            return;
        }
        if kind == Pick::Analysis {
            self.show_picker(kind.display());
            if self.document.analysis_review.is_some() {
                self.refresh_review_region();
            } else {
                self.request_analysis_support(cx);
            }
            return;
        }
        let result = match kind {
            Pick::Dmin => calibration::pick_dmin(self, cx),
            Pick::Dmax => calibration::pick_dmax(self, cx),
            Pick::Gamma => calibration::pick_gamma(self, cx),
            Pick::RollWb => white_balance::pick_roll(self, cx),
            Pick::Wb => white_balance::pick_layer(self, cx),
            Pick::Flat | Pick::Analysis => return,
        };
        match result {
            Ok(task) => {
                self.document
                    .visit
                    .pick_actions
                    .retain(|task| !task.is_finished());
                self.document.visit.pick_actions.push(task);
            }
            Err(error) => {
                if self.active_pick().is_some() {
                    self.cancel_picker();
                }
                self.status = error;
            }
        }
    }

    pub(super) fn apply_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.submit_picker() {
            return;
        }
        if self.document.analysis_review.is_some() {
            self.confirm_analysis_review(cx);
        } else if self.document.flat_edit.is_some() {
            self.apply_flat_edit(cx);
        } else if self.active_pick() == Some(analysis::AREA_DISPLAY) {
            self.apply_analysis_area(cx);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct ControlSettings {
    pub input: InputSettings,
    pub roll: RollSettings,
    pub photo: AdjustmentLayerSettings,
    pub film_emulation: FilmEmulationSettings,
}

impl ControlSettings {
    pub(super) fn capture(editor: &Editor) -> Self {
        Self {
            input: editor.document.input,
            roll: editor.document.roll,
            photo: editor.document.visit.adjustment_layer_settings,
            film_emulation: editor.document.film_emulation,
        }
    }

    pub(super) fn apply(self, editor: &mut Editor) {
        editor.document.input = self.input;
        editor.document.roll = self.roll;
        editor.document.visit.adjustment_layer_settings = self.photo;
        editor.document.film_emulation = self.film_emulation;
    }
}

fn reset_photo(photo: &mut AdjustmentLayerSettings, reset: controls::Reset) {
    use controls::{Parameter as P, Reset as R};
    let defaults = AdjustmentLayerSettings::default();
    match reset {
        R::Parameter(kind, _) => {
            let value = kind.default();
            match kind {
                P::Exposure => photo.exposure = value,
                P::Warmth => photo.wb.warmth = value,
                P::Tint => photo.wb.tint = value,
                P::Contrast => photo.contrast = value,
                P::Pivot => photo.pivot = value,
                P::ColourMix => photo.contrast_colour_mix = value,
                P::Saturation => photo.saturation = value,
                P::Density => photo.saturation_density = value,
                _ => {}
            }
        }
        R::Exposure => photo.exposure = defaults.exposure,
        R::Balance(_) => photo.wb = defaults.wb,
        R::Contrast => {
            photo.contrast = defaults.contrast;
            photo.pivot = defaults.pivot;
            photo.contrast_colour_mix = defaults.contrast_colour_mix;
        }
        R::Saturation => {
            photo.saturation = defaults.saturation;
            photo.saturation_density = defaults.saturation_density;
        }
        _ => {}
    }
}

fn reset_roll(input: &mut InputSettings, roll: &mut RollSettings, reset: controls::Reset) {
    use controls::{ColorControl as C, Parameter as P, Reset as R};
    // Shared grade (both stages).
    match reset {
        R::Parameter(P::Exposure, _) => roll.exposure = P::Exposure.default(),
        R::Parameter(P::Warmth, C::WhiteBalance) => roll.wb.warmth = P::Warmth.default(),
        R::Parameter(P::Tint, C::WhiteBalance) => roll.wb.tint = P::Tint.default(),
        R::Exposure => roll.exposure = 0.,
        R::Balance(C::WhiteBalance) => roll.wb = Balance::default(),
        _ => {}
    }
    // Negative interpretation (absent for Digital).
    let InputSettings::Negative {
        response,
        gamma,
        balance,
        ..
    } = input
    else {
        return;
    };
    match reset {
        R::Parameter(kind, bal) => {
            let value = kind.default();
            match kind {
                P::Gamma => *gamma = value,
                P::Toe => response.toe = value,
                P::Shoulder => response.shoulder = value,
                P::Warmth | P::Tint if matches!(bal, C::GammaBalance) => match kind {
                    P::Warmth => balance.warmth = value,
                    _ => balance.tint = value,
                },
                _ => {}
            }
        }
        R::Balance(C::GammaBalance) => *balance = Balance::default(),
        R::GammaAndBalance => {
            *gamma = crate::processing::DEFAULT_GAMMA;
            *balance = Balance::default();
        }
        R::Response => *response = crate::processing::Response::film(),
        _ => {}
    }
}

#[derive(Clone, Copy)]
pub(super) enum Scope {
    Roll,
    Photo(SourceId, adjustment_layers::Target),
    Rendering,
}

pub(super) enum Command {
    Reset(Scope, controls::Reset),
    InvertAdjustmentLayer(crate::project::AdjustmentLayerId, bool),
    OrderInput {
        start: Option<crate::project::AdjustmentLayerId>,
        drop: Option<(adjustment_layers::OrderKey, usize)>,
        finish: bool,
    },
    Photo(crop_editor::ToolAction),
    Roll(analysis::Orientation),
    Adjustment(adjustment_layers::Action),

    CreateMask(bool),
    Pick(Pick),
    ResetRepairs(SourceId),
    Correction(Option<bool>),
    FlatEdit(PathBuf, Option<Rect>),
}

impl Editor {
    pub(super) fn validate_control_edit(
        &self,
        previous: ControlSettings,
        candidate: ControlSettings,
    ) -> Result<(), Error> {
        candidate.film_emulation.validate()?;
        if candidate == previous || !candidate.input.ready() {
            return Ok(());
        }
        self.validate_processing(candidate.input, candidate.roll, candidate.photo)
    }

    // Numeric widgets must resolve focus loss and pass admission before a separate
    // command takes its snapshot or changes the controls' availability.
    pub(super) fn control_command(&mut self, command: Command, cx: &UiContext<'_, Self>) {
        if self.history_controls.is_some() {
            self.control_commands.push(command);
        } else {
            self.apply_control_command(command, cx);
        }
    }

    fn reset_control(&mut self, scope: Scope, reset: controls::Reset) {
        let before = self.history_snapshot();
        let result = match scope {
            Scope::Roll => {
                let previous = ControlSettings::capture(self);
                let mut candidate = previous;
                reset_roll(&mut candidate.input, &mut candidate.roll, reset);
                self.validate_control_edit(previous, candidate)
                    .map(|()| candidate.apply(self))
            }
            Scope::Photo(source, target) if Some(source) == self.document.visit.current_source => {
                match target {
                    adjustment_layers::Target::None => {
                        Err(Error::InvalidParameter("no selected adjustment layer"))
                    }
                    adjustment_layers::Target::Layer(id) => {
                        self.edit_adjustment_layers_named("Reset settings", |layers| {
                            let record = layers
                                .layers
                                .iter_mut()
                                .find(|record| record.id == id)
                                .ok_or(Error::InvalidParameter("unknown adjustment layer"))?;
                            match reset {
                            controls::Reset::Parameter(controls::Parameter::Strength, _) => {
                                record.adjustment.strength = 1.
                            }
                            controls::Reset::Parameter(controls::Parameter::Feather, _) => {
                                if let crate::adjustment_layers::AdjustmentLayerCoverage::Radial {
                                    feather,
                                    ..
                                } = &mut record.adjustment.mask
                                {
                                    *feather = controls::DEFAULT_RADIAL_FEATHER;
                                }
                            }
                            _ => reset_photo(&mut record.adjustment.photo, reset),
                        }
                            Ok(())
                        })
                    }
                }
            }
            Scope::Photo(..) => Err(Error::InvalidParameter("reset source changed")),
            Scope::Rendering => {
                if let controls::Reset::Parameter(kind, _) = reset {
                    let previous = ControlSettings::capture(self);
                    let mut candidate = previous;
                    match kind {
                        controls::Parameter::FilmAmount => {
                            candidate.film_emulation.amount = kind.default()
                        }
                        controls::Parameter::FilmSize => {
                            candidate.film_emulation.size = kind.default()
                        }
                        controls::Parameter::FilmColour => {
                            candidate.film_emulation.colour = kind.default()
                        }
                        _ => {}
                    }
                    self.validate_control_edit(previous, candidate)
                        .map(|()| candidate.apply(self))
                } else {
                    self.document.target = OutputTarget::DisplayP3;
                    Ok(())
                }
            }
        };
        if let Err(error) = result {
            self.status = format!("Reset rejected: {error}");
        }
        // Layer admission already records its complete candidate.
        if !matches!(scope, Scope::Photo(_, adjustment_layers::Target::Layer(_))) {
            self.record_edit(
                before,
                "Reset settings",
                match scope {
                    Scope::Photo(id, _) => Some(id),
                    _ => None,
                },
            );
        }
    }

    pub(super) fn apply_control_command(&mut self, command: Command, cx: &UiContext<'_, Self>) {
        match command {
            Command::Reset(scope, reset) => self.reset_control(scope, reset),
            Command::InvertAdjustmentLayer(id, invert) => {
                if let Err(error) = self.edit_adjustment_layers(|layers| {
                    let record = layers
                        .layers
                        .iter_mut()
                        .find(|record| record.id == id)
                        .ok_or(Error::InvalidParameter("unknown adjustment layer"))?;
                    record.adjustment.invert = invert;
                    Ok(())
                }) {
                    self.status = format!("Adjustment edit rejected: {error}");
                }
            }
            Command::OrderInput {
                start,
                drop,
                finish,
            } => self.adjustment_layer_order_input(start, drop, finish),
            Command::Photo(action) => self.apply_photo_tool_action(action),
            Command::Roll(action) => self.apply_roll_orientation(action),
            Command::Adjustment(action) => self.apply_adjustment_layer_action(action),

            Command::CreateMask(radial) => {
                if radial {
                    self.begin_radial_mask(None);
                } else {
                    self.begin_linear_mask(None);
                }
            }
            Command::Pick(kind) => self.begin_pick(kind, cx),
            Command::ResetRepairs(source) => self.reset_dust_repairs(source),
            Command::Correction(enabled) => {
                let mut candidate = self.document.capture();
                if let Some(enabled) = enabled {
                    candidate
                        .project
                        .flat_field_mut()
                        .expect("accepted correction")
                        .enabled = enabled;
                } else {
                    candidate.project.set_flat_field(None);
                }
                self.start_document(
                    Preparation::Correction(candidate, self.document.visit.current_source),
                    cx,
                );
            }
            Command::FlatEdit(path, region) => self.begin_flat_edit(path, region, cx),
        }
    }
}
