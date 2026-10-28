use super::{
    Balance, Calibration, Coverage, DVec3, Editor, Error, InputSettings, Pick, RollSettings, Scan,
    SourceId, Task, TaskRequest,
};
use crate::{flat_field::FlatField, scan::DevelopedSource};
use std::sync::Arc;

#[cfg(test)]
mod tests;

pub(super) enum Measurement {
    Reference(DVec3),
    Balance(Balance),
}

pub(super) struct Input {
    pub source: Arc<DevelopedSource>,
    pub reference: Option<crate::exposure_reference::ReferenceExposure>,
    pub correction: Option<Arc<FlatField>>,
    pub kind: Pick,
    pub region: Coverage,
    pub input: InputSettings,
    pub roll: RollSettings,
    pub upstream: crate::adjustment_layers::AdjustmentLayerSnapshot,
    pub layer: Option<crate::adjustment_layers::AdjustmentLayer>,
}

impl Input {
    pub fn measure(self) -> Result<Measurement, Error> {
        let mut scan = Scan::with_reference(self.source, self.reference);
        scan.set_flat_field(self.correction)?;
        match self.kind {
            Pick::Dmin | Pick::Dmax => scan
                .reference_coverage(self.region)
                .map(Measurement::Reference),
            Pick::Gamma => scan
                .roll_neutral_coverage(self.region, &self.input)
                .map(Measurement::Balance),
            Pick::RollWb => scan
                .white_balance_coverage(
                    self.region,
                    &self.input,
                    RollSettings {
                        wb: Balance::default(),
                        ..self.roll
                    },
                    crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![])?,
                    None,
                )
                .map(Measurement::Balance),
            Pick::Wb => scan
                .white_balance_coverage(
                    self.region,
                    &self.input,
                    self.roll,
                    self.upstream,
                    Some(
                        self.layer
                            .ok_or(Error::InvalidParameter("select an adjustment layer for WB"))?,
                    ),
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
    pub(super) target: Option<(SourceId, crate::project::AdjustmentLayerId)>,
    pub _task: async_executor::Task<()>,
}

impl Editor {
    pub(super) fn measurement_input(&self, kind: Pick, region: Coverage) -> Option<Input> {
        Some(Input {
            source: self.document.visit.ready.as_ref()?.developed.clone(),
            reference: self.document.visit.ready.as_ref()?.reference.clone(),
            correction: self
                .document
                .correction
                .as_ref()
                .map(|correction| correction.settings.map.clone()),
            kind,
            region,
            input: self.input(),
            roll: self.document.roll,
            upstream: if kind == Pick::Wb {
                let source = self
                    .document
                    .project
                    .source(self.document.visit.current_source?)
                    .ok()?;
                let crate::editor::adjustment_layers::Target::Layer(id) =
                    self.adjustment_layer_target()
                else {
                    return None;
                };
                let records = &source.adjustment_layers.as_ref()?.layers;
                let index = records.iter().position(|record| record.id == id)?;
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(
                    records[..index]
                        .iter()
                        .map(|record| record.adjustment)
                        .collect(),
                )
                .ok()?
            } else {
                crate::adjustment_layers::AdjustmentLayerSnapshot::new(vec![]).ok()?
            },
            layer: if kind == Pick::Wb {
                let source = self
                    .document
                    .project
                    .source(self.document.visit.current_source?)
                    .ok()?;
                let crate::editor::adjustment_layers::Target::Layer(id) =
                    self.adjustment_layer_target()
                else {
                    return None;
                };
                let mut layer = source
                    .adjustment_layers
                    .as_ref()?
                    .layers
                    .iter()
                    .find(|record| record.id == id)?
                    .adjustment;
                layer.photo = self.document.visit.adjustment_layer_settings;
                Some(layer)
            } else {
                None
            },
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
        self.document.thumbnails.cancel();
        let Some(input) = self.measurement_input(kind, region) else {
            self.status = "No developed source for measurement".into();
            self.finish_pick(cx);
            return;
        };
        let view = self.document.visit.view;
        let Some(task) = cx.spawn(move |cx| async move {
            let result = crate::background::run(move || input.measure())
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
        let target = match (
            kind,
            self.document.visit.current_source,
            self.adjustment_layer_target(),
        ) {
            (Pick::Wb, Some(source), crate::editor::adjustment_layers::Target::Layer(layer)) => {
                Some((source, layer))
            }
            _ => None,
        };
        self.document.visit.measurement = Some(Pending {
            kind,
            target,
            _task: task,
        });
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
            .and_then(|pending| pending.target)
            .is_some_and(|(source, layer)| {
                self.document.visit.current_source != Some(source)
                    || self.adjustment_layer_target()
                        != crate::editor::adjustment_layers::Target::Layer(layer)
            })
        {
            self.cancel_measurement();
            self.status = "WB target changed; measurement discarded".into();
            self.finish_pick(cx);
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
        let photo = (kind == Pick::Wb)
            .then_some(self.document.visit.current_source)
            .flatten();
        match result {
            Ok(Measurement::Reference(value)) => {
                let InputSettings::Negative {
                    dmin,
                    dmax,
                    input_floor,
                    response,
                    gamma,
                    balance,
                } = self.document.input
                else {
                    self.status = "Reference picks require a scan input".into();
                    self.finish_pick(cx);
                    return;
                };
                let other = if kind == Pick::Dmin { dmax } else { dmin };
                let Some(other) = other else {
                    set_reference(&mut self.document.input, kind, value);
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
                if let Err(error) = calibration.validate().and_then(|_| {
                    self.validate_processing(
                        candidate,
                        self.document.roll,
                        self.document.visit.adjustment_layer_settings,
                    )
                }) {
                    self.status = error.to_string();
                    self.finish_pick(cx);
                    return;
                }
                self.document.input = candidate;
                self.status = format!("{kind:?} measured");
            }
            Ok(Measurement::Balance(balance)) => {
                let (mut roll, mut photo) = (
                    self.document.roll,
                    self.document.visit.adjustment_layer_settings,
                );
                let mut input = self.document.input;
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
                if input.ready()
                    && let Err(error) = self.validate_processing(input, roll, photo)
                {
                    self.status = format!("Pick rejected: {error}");
                    self.finish_pick(cx);
                    return;
                }
                self.document.input = input;
                self.document.roll = roll;
                self.document.visit.adjustment_layer_settings = photo;
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
