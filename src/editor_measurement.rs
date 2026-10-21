use super::{
    Balance, Calibration, Coverage, DVec3, Editor, Error, PhotoSettings, Pick, RollSettings, Scan,
    Task, TaskRequest,
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
    pub calibration: Option<Calibration>,
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
                .roll_neutral_coverage(
                    self.region,
                    self.calibration
                        .ok_or(Error::UnusableSample("both film references are required"))?,
                    self.roll,
                )
                .map(Measurement::Balance),
            Pick::RollWb => scan
                .photo_neutral_coverage(
                    self.region,
                    self.calibration
                        .ok_or(Error::UnusableSample("both film references are required"))?,
                    RollSettings {
                        wb: Balance::default(),
                        ..self.roll
                    },
                    PhotoSettings::default(),
                )
                .map(Measurement::Balance),
            Pick::Wb => scan
                .photo_neutral_coverage(
                    self.region,
                    self.calibration
                        .ok_or(Error::UnusableSample("both film references are required"))?,
                    self.roll,
                    self.photo,
                )
                .map(Measurement::Balance),
            Pick::Flat | Pick::Analysis => Err(Error::InvalidParameter(
                "area acceptance uses a separate transaction",
            )),
        }
    }
}

pub(super) struct Pending {
    pub kind: Pick,
    pub request: Option<Input>,
    pub _task: Option<async_executor::Task<()>>,
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
            calibration: self.calibration(),
            roll: self.roll,
            photo: self.photo,
        })
    }

    pub(super) fn queue_measurement(&mut self, kind: Pick, region: Coverage) {
        if self.document.visit.measurement.is_some() {
            return;
        }
        let Some(input) = self.measurement_input(kind, region) else {
            self.status = "No developed source for measurement".into();
            self.finish_pick();
            return;
        };
        self.document.visit.measurement = Some(Pending {
            kind,
            request: Some(input),
            _task: None,
        });
        self.tasks
            .begin(Task::Measurement(self.document.visit.view, kind));
        self.busy = true;
        self.status = "Measuring full-resolution patch…".into();
    }

    pub(super) fn start_measurement_workflow(
        &mut self,
        access: &crate::application::UiAccess<'_, Self>,
    ) {
        let Some(pending) = &mut self.document.visit.measurement else {
            return;
        };
        let Some(input) = pending.request.take() else {
            return;
        };
        let kind = pending.kind;
        let blocking = self.workflow_jobs.clone();
        let view = self.document.visit.view;
        let target = access.clone();
        let Some(task) = access.spawn(async move {
            let result = blocking
                .run(move || input.measure())
                .await
                .map_err(|_| "Measurement worker panicked".to_owned())
                .and_then(|result| result.map_err(|error| error.to_string()));
            target.update(|editor| {
                editor.measurement_result(view, kind, result);
                editor.start_source_workflow(&target);
            });
        }) else {
            self.status = "Application stopped".into();
            self.finish_pick();
            return;
        };
        self.document.visit.measurement.as_mut().unwrap()._task = Some(task);
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
                let other = if kind == Pick::Dmin {
                    self.dmax
                } else {
                    self.dmin
                };
                let valid = other
                    .map(|other| {
                        let (dmin, dmax) = if kind == Pick::Dmin {
                            (value, other)
                        } else {
                            (other, value)
                        };
                        Calibration {
                            dmin,
                            dmax,
                            input_floor: self.floor,
                        }
                        .validate()
                        .and_then(|cal| {
                            self.validate_processing(cal, self.roll, self.photo)
                                .map(|_| cal)
                        })
                    })
                    .transpose();
                if let Err(error) = valid {
                    self.status = error.to_string();
                    self.finish_pick();
                    return;
                }
                if kind == Pick::Dmin {
                    self.dmin = Some(value);
                } else {
                    self.dmax = Some(value);
                }
                self.status = format!("{kind:?} measured");
            }
            Ok(Measurement::Balance(balance)) => {
                let mut roll = self.roll;
                let mut photo = self.photo;
                match kind {
                    Pick::Gamma => roll.balance = balance,
                    Pick::RollWb => roll.wb = balance,
                    _ => photo.wb = balance,
                }
                if let Some(cal) = self.calibration()
                    && let Err(error) = self.validate_processing(cal, roll, photo)
                {
                    self.status = format!("Pick rejected: {error}");
                    self.finish_pick();
                    return;
                }
                self.roll = roll;
                self.photo = photo;
                self.status = format!("{kind:?} neutral correction applied");
            }
            Err(error) => self.status = error,
        }
        self.record_edit(before, "Calibration pick", photo);
        self.finish_pick();
    }
}
