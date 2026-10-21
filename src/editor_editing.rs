use super::*;

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

fn reset_roll(roll: &mut RollSettings, reset: controls::Reset) {
    use controls::{ColorControl as C, Parameter as P, Reset as R};
    match reset {
        R::Parameter(kind, balance) => {
            let value = kind.default();
            match kind {
                P::Gamma => roll.gamma = value,
                P::Exposure => roll.exposure = value,
                P::Toe => {
                    if let crate::processing::Response::Film { toe, .. } = &mut roll.response {
                        *toe = value;
                    }
                }
                P::Shoulder => {
                    if let crate::processing::Response::Film { shoulder, .. } = &mut roll.response {
                        *shoulder = value;
                    }
                }
                P::Warmth | P::Tint => {
                    let balance = match balance {
                        C::GammaBalance => &mut roll.balance,
                        C::WhiteBalance => &mut roll.wb,
                    };
                    match kind {
                        P::Warmth => balance.warmth = value,
                        _ => balance.tint = value,
                    }
                }
                _ => {}
            }
        }
        R::Exposure => roll.exposure = 0.,
        R::Balance(C::GammaBalance) => roll.balance = Balance::default(),
        R::Balance(C::WhiteBalance) => roll.wb = Balance::default(),
        R::GammaAndBalance => {
            roll.gamma = crate::processing::DEFAULT_GAMMA;
            roll.balance = Balance::default();
        }
        R::Response => roll.response = crate::processing::Response::film(),
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
    // Numeric widgets must resolve focus loss and pass admission before a separate
    // command takes its snapshot or changes the controls' availability.
    pub(super) fn control_command(&mut self, command: Command) {
        if self.history_controls.is_some() {
            self.control_commands.push(command);
        } else {
            self.apply_control_command(command);
        }
    }

    fn reset_control(&mut self, scope: Scope, reset: controls::Reset) {
        let before = self.history_snapshot();
        let result = match scope {
            Scope::Roll => {
                let mut roll = self.roll;
                reset_roll(&mut roll, reset);
                self.calibration()
                    .map_or(Ok(()), |cal| {
                        self.validate_processing(cal, roll, self.photo)
                    })
                    .map(|()| self.roll = roll)
            }
            Scope::Photo(source, target) if Some(source) == self.current_source => match target {
                adjustments::Target::Global => {
                    let mut photo = self.photo;
                    reset_photo(&mut photo, reset);
                    self.calibration()
                        .map_or(Ok(()), |cal| {
                            self.validate_processing(cal, self.roll, photo)
                        })
                        .map(|()| self.photo = photo)
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

    pub(super) fn apply_control_command(&mut self, command: Command) {
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
            Command::Pick(kind) => self.begin_pick(kind),
            Command::ResetRepairs(source) => self.reset_dust_repairs(source),
            Command::Correction(enabled) => {
                let mut candidate = self.document.capture();
                if let Some(enabled) = enabled {
                    candidate
                        .project
                        .flat_field
                        .as_mut()
                        .expect("accepted correction")
                        .enabled = enabled;
                } else {
                    candidate.project.flat_field = None;
                }
                self.start_document(Preparation::Correction(candidate, self.current_source));
            }
            Command::FlatEdit(path, region) => self.begin_flat_edit(path, region),
        }
    }
}
