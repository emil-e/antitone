use super::*;
use crate::processing::InputSettings;

#[derive(Clone, Copy, PartialEq)]
pub(super) struct ControlSettings {
    pub input: InputSettings,
    pub roll: RollSettings,
    pub photo: PhotoSettings,
}

impl ControlSettings {
    pub(super) fn capture(editor: &Editor) -> Self {
        Self {
            input: editor.input,
            roll: editor.roll,
            photo: editor.photo,
        }
    }

    pub(super) fn apply(self, editor: &mut Editor) {
        editor.input = self.input;
        editor.roll = self.roll;
        editor.photo = self.photo;
    }
}

fn reset_photo(photo: &mut PhotoSettings, reset: controls::Reset) {
    use controls::{Parameter as P, Reset as R};
    let defaults = PhotoSettings::default();
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
    Photo(SourceId, adjustments::Target),
    Rendering,
}

pub(super) enum Command {
    Reset(Scope, controls::Reset),
    InvertLocal(crate::project::AdjustmentId, bool),
    OrderInput {
        start: Option<crate::project::AdjustmentId>,
        drop: Option<(adjustments::OrderKey, usize)>,
        finish: bool,
    },
    Photo(crop_editor::ToolAction),
    Roll(analysis::Orientation),
    Adjustment(adjustments::Action),
    ToggleAdjustments,
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
        if candidate == previous || !candidate.input.ready() {
            return Ok(());
        }
        if candidate.input != previous.input || candidate.roll != previous.roll {
            self.validate_processing(candidate.input, candidate.roll, candidate.photo)
        } else {
            self.dust_validate_processing(candidate.input, candidate.roll, candidate.photo)?;
            self.positive_processing(candidate.input, candidate.roll, candidate.photo)
                .map(|_| ())
        }
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
            Scope::Photo(source, target) if Some(source) == self.current_source => match target {
                adjustments::Target::Global => {
                    let previous = ControlSettings::capture(self);
                    let mut candidate = previous;
                    reset_photo(&mut candidate.photo, reset);
                    self.validate_control_edit(previous, candidate)
                        .map(|()| candidate.apply(self))
                }
                adjustments::Target::Local(id) => self.edit_adjustments(|locals| {
                    let record = locals
                        .adjustments
                        .iter_mut()
                        .find(|record| record.id == id)
                        .ok_or(Error::InvalidParameter("unknown local adjustment"))?;
                    match reset {
                        controls::Reset::Parameter(controls::Parameter::Strength, _) => {
                            record.adjustment.strength = 1.
                        }
                        controls::Reset::Parameter(controls::Parameter::Feather, _) => {
                            if let crate::local_adjustments::GradientMask::Radial {
                                feather, ..
                            } = &mut record.adjustment.mask
                            {
                                *feather = controls::DEFAULT_RADIAL_FEATHER;
                            }
                        }
                        _ => reset_photo(&mut record.adjustment.photo, reset),
                    }
                    Ok(())
                }),
            },
            Scope::Photo(..) => Err(Error::InvalidParameter("reset source changed")),
            Scope::Rendering => {
                self.target = OutputTarget::DisplayP3;
                Ok(())
            }
        };
        if let Err(error) = result {
            self.status = format!("Reset rejected: {error}");
        }
        // Local admission already records its complete candidate.
        if !matches!(scope, Scope::Photo(_, adjustments::Target::Local(_))) {
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
            Command::InvertLocal(id, invert) => {
                if let Err(error) = self.edit_adjustments(|locals| {
                    let record = locals
                        .adjustments
                        .iter_mut()
                        .find(|record| record.id == id)
                        .ok_or(Error::InvalidParameter("unknown local adjustment"))?;
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
            } => self.adjustment_order_input(start, drop, finish),
            Command::Photo(action) => self.apply_photo_tool_action(action),
            Command::Roll(action) => self.apply_roll_orientation(action),
            Command::Adjustment(action) => self.apply_adjustment_action(action),
            Command::ToggleAdjustments => {
                if self.document.visit.adjustments.drawer_open() {
                    self.close_adjustments();
                } else {
                    self.open_adjustments();
                }
            }
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
                self.start_document(Preparation::Correction(candidate, self.current_source), cx);
            }
            Command::FlatEdit(path, region) => self.begin_flat_edit(path, region, cx),
        }
    }
}
