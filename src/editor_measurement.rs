use super::{
    Balance, Calibration, Coverage, DVec3, Editor, Error, InputSettings, PhotoSettings, Pick,
    RollSettings, Scan, Task, TaskRequest,
};
use crate::{flat_field::FlatField, scan::DevelopedSource};
use std::sync::Arc;

#[cfg(test)]
#[path = "editor_measurement_tests.rs"]
mod tests;

pub(super) enum Measurement {
    Reference(DVec3),
    Balance(Balance),
}

pub(super) struct Input {
    pub source: Arc<DevelopedSource>,
    pub correction: Option<Arc<FlatField>>,
    pub kind: Pick,
    pub region: Coverage,
    pub input: InputSettings,
    pub roll: RollSettings,
    pub photo: PhotoSettings,
}

impl Input {
    pub fn measure(self) -> Result<Measurement, Error> {
        let mut scan = Scan::from_source(self.source);
        scan.set_flat_field(self.correction)?;
        match self.kind {
            Pick::Dmin | Pick::Dmax => scan
                .reference_coverage(self.region)
                .map(Measurement::Reference),
            Pick::Gamma => scan
                .roll_neutral_coverage(self.region, &self.input)
                .map(Measurement::Balance),
            Pick::RollWb => scan
                .photo_neutral_coverage(
                    self.region,
                    &self.input,
                    RollSettings {
                        wb: Balance::default(),
                        ..self.roll
                    },
                    PhotoSettings::default(),
                )
                .map(Measurement::Balance),
            Pick::Wb => scan
                .photo_neutral_coverage(self.region, &self.input, self.roll, self.photo)
                .map(Measurement::Balance),
            Pick::Flat | Pick::Analysis => Err(Error::InvalidParameter(
                "area acceptance uses a separate transaction",
            )),
        }
    }
}

pub(super) struct Pending {
    pub kind: Pick,
    pub _task: async_executor::Task<()>,
}

impl Editor {
    pub(super) fn measurement_input(&self, kind: Pick, region: Coverage) -> Option<Input> {
        Some(Input {
            source: self.document.visit.ready.as_ref()?.developed.clone(),
            correction: self
                .correction
                .as_ref()
                .map(|correction| correction.settings.map.clone()),
            kind,
            region,
            input: self.input(),
            roll: self.roll,
            photo: self.photo,
        })
    }

    pub(super) fn start_measurement(
        &mut self,
        kind: Pick,
        region: Coverage,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        if self.document.visit.measurement.is_some() {
            return;
        }
        self.thumbnails.cancel();
        let Some(input) = self.measurement_input(kind, region) else {
            self.status = "No developed source for measurement".into();
            self.finish_pick(cx);
            return;
        };
        let blocking = self.workflow_jobs.clone();
        let view = self.document.visit.view;
        let Some(task) = cx.spawn(move |cx| async move {
            let result = blocking
                .run(move || input.measure())
                .await
                .map_err(|_| "Measurement worker panicked".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            cx.update(|editor| {
                editor.measurement_result(view, kind, result, &cx);
            });
        }) else {
            self.status = "Application stopped".into();
            self.finish_pick(cx);
            return;
        };
        self.document.visit.measurement = Some(Pending { kind, _task: task });
        self.tasks.begin(Task::Measurement(view, kind));
        self.busy = true;
        self.status = "Measuring full-resolution patch…".into();
    }

    pub(super) fn cancel_measurement(&mut self) {
        if let Some(pending) = self.document.visit.measurement.take() {
            self.tasks.finish(TaskRequest::Measurement(
                self.document.visit.view,
                pending.kind,
            ));
            self.busy = false;
        }
    }

    pub(super) fn measurement_result(
        &mut self,
        view: u64,
        kind: Pick,
        result: Result<Measurement, String>,
        cx: &crate::application::UiContext<'_, Self>,
    ) {
        if view != self.document.visit.view || self.document_pending.is_some() {
            return;
        }
        if self
            .document
            .visit
            .measurement
            .as_ref()
            .is_some_and(|pending| self.document.visit.view == view && pending.kind == kind)
        {
            self.document.visit.measurement = None;
        }
        self.tasks.finish(TaskRequest::Measurement(view, kind));
        self.busy = false;
        let before = self.history_snapshot();
        let photo = (kind == Pick::Wb).then_some(self.current_source).flatten();
        match result {
            Ok(Measurement::Reference(value)) => {
                let InputSettings::Negative {
                    dmin,
                    dmax,
                    input_floor,
                    response,
                    gamma,
                    balance,
                } = self.input
                else {
                    self.status = "Reference picks require a scan input".into();
                    self.finish_pick(cx);
                    return;
                };
                let other = if kind == Pick::Dmin { dmax } else { dmin };
                let Some(other) = other else {
                    set_reference(&mut self.input, kind, value);
                    self.status = format!("{kind:?} measured");
                    self.record_edit(before, "Calibration pick", photo);
                    self.finish_pick(cx);
                    return;
                };
                let (next_dmin, next_dmax) = if kind == Pick::Dmin {
                    (value, other)
                } else {
                    (other, value)
                };
                let candidate = InputSettings::Negative {
                    dmin: Some(next_dmin),
                    dmax: Some(next_dmax),
                    input_floor,
                    response,
                    gamma,
                    balance,
                };
                let calibration = Calibration {
                    dmin: next_dmin,
                    dmax: next_dmax,
                    input_floor,
                };
                if let Err(error) = calibration
                    .validate()
                    .and_then(|_| self.validate_processing(candidate, self.roll, self.photo))
                {
                    self.status = error.to_string();
                    self.finish_pick(cx);
                    return;
                }
                self.input = candidate;
                self.status = format!("{kind:?} measured");
            }
            Ok(Measurement::Balance(balance)) => {
                let (mut roll, mut photo) = (self.roll, self.photo);
                let mut input = self.input;
                match kind {
                    Pick::Gamma => {
                        if let InputSettings::Negative {
                            balance: gamma_balance,
                            ..
                        } = &mut input
                        {
                            *gamma_balance = balance;
                        }
                    }
                    Pick::RollWb => roll.wb = balance,
                    _ => photo.wb = balance,
                }
                if input.calibration().is_some()
                    && let Err(error) = self.validate_processing(input, roll, photo)
                {
                    self.status = format!("Pick rejected: {error}");
                    self.finish_pick(cx);
                    return;
                }
                self.input = input;
                self.roll = roll;
                self.photo = photo;
                self.status = format!("{kind:?} neutral correction applied");
            }
            Err(error) => self.status = error,
        }
        self.record_edit(before, "Calibration pick", photo);
        self.finish_pick(cx);
    }
}

fn set_reference(input: &mut InputSettings, kind: Pick, value: DVec3) {
    if let InputSettings::Negative { dmin, dmax, .. } = input {
        match kind {
            Pick::Dmin => *dmin = Some(value),
            Pick::Dmax => *dmax = Some(value),
            _ => {}
        }
    }
}
