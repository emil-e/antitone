use super::*;
use crate::processing::InputSettings;
use std::{future::Future, pin::Pin};

pub(super) type Interaction =
    for<'scope> fn(UiContext<'scope, Editor>) -> Pin<Box<dyn Future<Output = ()> + 'scope>>;

impl Editor {
    /// Retain an otherwise-unowned interactive action until it finishes or its context retires.
    pub(super) fn start_interaction<'scope, F, Fut>(
        &mut self,
        cx: &UiContext<'scope, Self>,
        work: F,
    ) -> Result<(), String>
    where
        F: FnOnce(UiContext<'scope, Editor>) -> Fut + 'scope,
        Fut: Future<Output = ()> + 'scope,
    {
        if !self.prepare_tool() {
            return Ok(());
        }
        self.document.visit.interaction_action = Some(cx.spawn(work).ok_or("Application stopped")?);
        Ok(())
    }
}

impl Editor {
    pub(super) fn finish_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.document.analysis_review().is_some() {
            self.status = "Analysis review cancelled; areas and grade unchanged".into();
            self.end_analysis_review(cx);
        } else if self.document.visit.picker.display().is_some() {
            self.end_tool();
        }
    }

    pub(super) fn apply_pick(&mut self, cx: &UiContext<'_, Self>) {
        if self.current_tool().is_some() {
            self.dispatch_tool(tool::Action::Primary, cx);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) struct ControlSettings {
    pub input: InputSettings,
    pub roll: RollSettings,
    pub photo: AdjustmentLayerSettings,
    pub grain: GrainSettings,
    pub halation: crate::halation::HalationSettings,
}

impl ControlSettings {
    pub(super) fn capture(editor: &Editor) -> Self {
        Self {
            input: editor.document.input,
            roll: editor.document.roll,
            photo: editor.document.visit.adjustment_layer_settings,
            grain: editor.document.grain,
            halation: editor.document.halation,
        }
    }

    pub(super) fn apply(self, editor: &mut Editor) {
        editor.document.input = self.input;
        editor.document.roll = self.roll;
        editor.document.visit.adjustment_layer_settings = self.photo;
        editor.document.grain = self.grain;
        editor.document.halation = self.halation;
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

pub(super) enum PhotoAction {
    TurnLeft,
    TurnRight,
    FlipHorizontal,
    FlipVertical,
    Crop,
    Dust,
}

pub(super) enum Command {
    ForPhoto(SourceId, Box<Command>),
    Reset(Scope, controls::Reset),
    InvertAdjustmentLayer(crate::project::AdjustmentLayerId, bool),
    OrderInput {
        start: Option<crate::project::AdjustmentLayerId>,
        drop: Option<(adjustment_layers::OrderKey, usize)>,
        finish: bool,
    },
    Photo(PhotoAction),
    Roll(analysis::Orientation),
    Adjustment(adjustment_layers::Action),

    CreateMask(bool),
    Pick(Interaction),
    GammaPivot,
    Analyze,
    Input(InputSettings),
    ResetRepairs(SourceId),
    Correction(Option<bool>),
    FlatEdit(PathBuf, Option<Rect>),
}

impl Editor {
    pub(super) fn apply_photo_action(&mut self, action: PhotoAction, cx: &UiContext<'_, Self>) {
        match action {
            PhotoAction::Crop => {
                if self.cropping() {
                    self.dispatch_tool(tool::Action::Dismiss, cx);
                } else {
                    self.begin_crop();
                }
            }
            PhotoAction::Dust => {
                if self.dusting() {
                    self.dispatch_tool(tool::Action::Dismiss, cx);
                } else {
                    self.begin_dust_cleanup();
                }
            }
            _ => {
                let result = self.edit_photo_geometry("Photo orientation", |orientation, _| {
                    match action {
                        PhotoAction::TurnLeft => orientation.turn_counterclockwise(),
                        PhotoAction::TurnRight => orientation.turn_clockwise(),
                        PhotoAction::FlipHorizontal => orientation.flips.horizontal ^= true,
                        PhotoAction::FlipVertical => orientation.flips.vertical ^= true,
                        _ => unreachable!(),
                    }
                    Ok(())
                });
                if let Err(error) = result {
                    self.status = format!("Photo orientation rejected: {error}");
                }
            }
        }
    }

    pub(super) fn edit_photo_geometry(
        &mut self,
        label: &'static str,
        edit: impl FnOnce(&mut PhotoOrientation, &mut Option<Crop>) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let mut orientation = self.document.visit.photo_orientation;
        let mut crop = self.document.visit.crop;
        edit(&mut orientation, &mut crop)?;
        if let Some(candidate) = crop {
            let area = self
                .document
                .visit
                .canvas
                .ready
                .as_ref()
                .map(|ready| ready.area)
                .or_else(|| {
                    let source = self
                        .document
                        .state
                        .project
                        .source(self.document.visit.current_source?)
                        .ok()?;
                    Some(source.development.active_area.into())
                })
                .ok_or(Error::InvalidParameter(
                    "Photo geometry source is unavailable",
                ))?;
            candidate.region(area)?;
        }
        if orientation == self.document.visit.photo_orientation && crop == self.document.visit.crop
        {
            return Ok(());
        }
        let before = self.history_snapshot();
        self.document.visit.photo_orientation = orientation;
        self.document.visit.crop = crop;
        self.document.visit.canvas.viewport.fit();
        self.invalidate_original();
        self.record_edit(before, label, self.document.visit.current_source);
        Ok(())
    }

    pub(super) fn accept_crop(&mut self, draft: Crop) -> Result<(), Error> {
        let area = self
            .selected_area()
            .ok_or(Error::InvalidParameter("Crop source is unavailable"))?;
        self.edit_photo_geometry("Crop", |_, crop| {
            *crop = (draft != Crop::from_region(area)).then_some(draft);
            Ok(())
        })?;
        self.document.visit.canvas.viewport.fit();
        Ok(())
    }

    pub(super) fn validate_control_edit(
        &self,
        previous: ControlSettings,
        candidate: ControlSettings,
    ) -> Result<(), Error> {
        candidate.grain.validate()?;
        candidate.halation.validate()?;
        if candidate == previous || !candidate.input.ready() {
            return Ok(());
        }
        self.validate_processing(candidate.input, candidate.roll, candidate.photo)
    }

    // Numeric widgets must resolve focus loss and pass admission before a separate
    // command takes its snapshot or changes the controls' availability.
    pub(super) fn control_command(&mut self, command: Command, cx: &UiContext<'_, Self>) {
        if self.history_controls.is_some() || self.document.gamma_pivot.pending() {
            let photo_bound = matches!(
                &command,
                Command::Photo(_)
                    | Command::Pick(_)
                    | Command::CreateMask(_)
                    | Command::Adjustment(_)
                    | Command::InvertAdjustmentLayer(..)
                    | Command::OrderInput { .. }
            );
            let command = if photo_bound && let Some(source) = self.document.visit.current_source {
                Command::ForPhoto(source, Box::new(command))
            } else {
                command
            };
            self.control_commands.push(command);
        } else {
            self.apply_control_command(command, cx);
        }
    }

    fn reset_control(&mut self, scope: Scope, reset: controls::Reset, cx: &UiContext<'_, Self>) {
        let before = self.history_snapshot();
        let result = match scope {
            Scope::Roll => {
                let previous = ControlSettings::capture(self);
                let mut candidate = previous;
                reset_roll(&mut candidate.input, &mut candidate.roll, reset);
                self.accept_manual_controls(previous, candidate, true, None, cx)
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
                        controls::Parameter::FilmAmount => candidate.grain.amount = kind.default(),
                        controls::Parameter::FilmSize => {
                            candidate.grain = candidate.grain.resized(kind.default())
                        }
                        controls::Parameter::FilmColour => candidate.grain.colour = kind.default(),
                        controls::Parameter::HalationStrength => {
                            candidate.halation.strength = kind.default()
                        }
                        controls::Parameter::HalationSize => {
                            candidate.halation.size = kind.default()
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
            Command::ForPhoto(source, command) => {
                if self.document.visit.current_source == Some(source) {
                    self.apply_control_command(*command, cx);
                } else {
                    self.status = "Action cancelled because the selected photo changed".into();
                }
            }
            Command::Reset(scope, reset) => self.reset_control(scope, reset, cx),
            Command::GammaPivot => self.toggle_gamma_pivot(cx),
            Command::Analyze => self.request_analysis(cx),
            Command::Input(input) => self.request_input(input, cx),
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
            Command::Photo(action) => self.apply_photo_action(action, cx),
            Command::Roll(action) => self.apply_roll_orientation(action),
            Command::Adjustment(action) => self.apply_adjustment_layer_action(action),

            Command::CreateMask(radial) => {
                if radial {
                    self.begin_radial_mask(None);
                } else {
                    self.begin_linear_mask(None);
                }
            }
            Command::Pick(start) => {
                if let Err(error) = self.start_interaction(cx, start) {
                    self.status = error;
                }
            }
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
            Command::FlatEdit(path, region) => flat::FlatTool::open(self, path, region, cx),
        }
    }
}
