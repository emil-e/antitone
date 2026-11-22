use super::{Editor, InputSettings, UiContext, measurement, ui::ControlResponse};
use crate::{Error, color::Balance, coverage::Coverage, geometry::SourceRegion};
use futures_util::{
    FutureExt,
    future::{LocalBoxFuture, Shared},
};
use std::sync::Arc;

mod query;
#[cfg(test)]
mod tests;

struct Reference {
    camera_to_ap1: glam::DMat3,
    evidence: query::Evidence,
}

type Pair = (InputSettings, Balance);
type ReferenceWork = Shared<LocalBoxFuture<'static, Result<Arc<Reference>, String>>>;

struct Gesture {
    id: egui::Id,
    initial: Pair,
    accepted: Pair,
    reference: ReferenceWork,
}

/// The roll owns the toggle and in-flight edit; reference work belongs to one gesture.
#[derive(Default)]
pub(super) struct State {
    bypassed: bool,
    gesture: Option<Gesture>,
    pending: Option<Pending>,
    #[cfg(test)]
    pub(super) edit_gate: Option<measurement::testing::Gate>,
}

struct Pending {
    input: InputSettings,
    response: ControlResponse,
    _task: async_executor::Task<()>,
}

impl State {
    pub(super) fn requested_input(&self) -> Option<InputSettings> {
        self.pending.as_ref().map(|pending| pending.input)
    }

    #[cfg(test)]
    pub(super) fn calculating(&self) -> bool {
        self.pending.is_some()
    }

    pub(super) fn enabled(&self) -> bool {
        !self.bypassed
    }

    fn gesture(&self, response: ControlResponse) -> Option<&Gesture> {
        self.gesture
            .as_ref()
            .filter(|gesture| gesture.id == response.id)
    }
}

pub(super) fn same_gamma(a: InputSettings, b: InputSettings) -> bool {
    a.gamma() == b.gamma() && a.balance() == b.balance()
}

fn basis_input(mut input: InputSettings) -> InputSettings {
    if let InputSettings::Negative { gamma, balance, .. } = &mut input {
        *gamma = crate::processing::DEFAULT_GAMMA;
        *balance = Balance::default();
    }
    input
}

impl Editor {
    fn gamma_pivot_available(&self, input: InputSettings) -> bool {
        self.document.visit.current_source.is_some()
            && input
                .calibration()
                .is_some_and(|calibration| calibration.validate().is_ok())
    }

    pub(super) fn gamma_pivot_tool(&self, input: InputSettings) -> super::ui::SectionTool<'static> {
        let available = self.tool_start_available() && self.gamma_pivot_available(input);
        let enabled = self.document.gamma_pivot.enabled();
        super::ui::SectionTool {
            icon: super::ui::icons::Icon::Pin {
                active: available && enabled,
                filled: available && enabled,
            },
            enabled: available,
            name: "Hold midtone colour",
            help: if !available {
                "Select a calibrated negative to hold midtone colour."
            } else if enabled {
                "Turn off Roll white-balance compensation for gamma edits."
            } else {
                "Hold this photo’s midtone colour while adjusting gamma."
            },
        }
    }

    pub(super) fn toggle_gamma_pivot(&mut self) {
        let state = &mut self.document.gamma_pivot;
        state.bypassed = !state.bypassed;
        if let Some(gesture) = state.gesture.take() {
            self.document.history.finish_control(gesture.id);
        }
        if let Some(pending) = &mut state.pending {
            pending.response.active = false;
        }
        self.status = if state.enabled() {
            "Gamma midtone lock on"
        } else {
            "Gamma midtone lock off"
        }
        .into();
    }

    pub(super) fn gamma_compensation_required(
        &self,
        starting: InputSettings,
        proposed: InputSettings,
    ) -> bool {
        !same_gamma(starting, proposed)
            && basis_input(starting) == basis_input(proposed)
            && self.gamma_pivot_available(starting)
            && self.document.gamma_pivot.enabled()
    }

    pub(super) fn gamma_input_pending(&self, response: ControlResponse) -> bool {
        self.document
            .gamma_pivot
            .pending
            .as_ref()
            .is_some_and(|pending| pending.response.id == response.id)
    }

    pub(super) fn gamma_control_response(&mut self, response: ControlResponse) {
        let project = &self.document.state.project;
        let state = &mut self.document.gamma_pivot;
        if state.gesture(response).is_some_and(|gesture| {
            gesture.accepted != (project.input, project.roll.wb)
                || response.started
                || response.cancelled
                || (!response.active && !response.changed)
        }) {
            state.gesture = None;
        }
        if let Some(pending) = &mut state.pending
            && pending.response.id == response.id
        {
            if response.cancelled && pending.response.active {
                state.pending = None;
            } else if response.started || !response.active {
                pending.response.active = false;
            }
        }
    }

    pub(super) fn gamma_control_initial(&self, response: ControlResponse) -> Pair {
        if let Some(gesture) = self.document.gamma_pivot.gesture(response) {
            return gesture.initial;
        }
        let initial = self.control_initial(response, false);
        (initial.input, initial.roll.wb)
    }

    pub(super) fn clear_gamma_proposal(&mut self) {
        self.document.gamma_pivot.pending = None;
    }

    pub(super) fn gamma_value_accepted(&mut self, response: ControlResponse) {
        let state = &mut self.document.gamma_pivot;
        if let Some(gesture) = &mut state.gesture
            && gesture.id == response.id
        {
            if response.active {
                let project = &self.document.state.project;
                gesture.accepted = (project.input, project.roll.wb);
            } else {
                state.gesture = None;
            }
        }
    }

    pub(super) fn compensate_gamma(
        &mut self,
        initial: Pair,
        candidate: InputSettings,
        response: ControlResponse,
        cx: &UiContext<'_, Self>,
    ) {
        let state = &mut self.document.gamma_pivot;
        let continuing = state.gesture(response).is_some();
        if let Some(pending) = &mut state.pending
            && continuing
            && pending.input == candidate
            && pending.response.id == response.id
        {
            pending.response = response;
            if !response.active {
                state.gesture = None;
            }
            return;
        }
        state.pending = None;
        let retained = state
            .gesture
            .take()
            .filter(|gesture| gesture.id == response.id);
        let reference = match retained
            .map(|gesture| Ok(gesture.reference))
            .unwrap_or_else(|| capture_reference(self, initial.0))
        {
            Ok(reference) => reference,
            Err(error) => {
                self.status = format!("Edit rejected: {error}");
                return;
            }
        };
        let (starting_input, starting_wb) = initial;
        let expected = (
            self.document.state.project.input,
            self.document.state.project.roll.wb,
        );
        if response.active {
            self.document.gamma_pivot.gesture = Some(Gesture {
                id: response.id,
                initial,
                accepted: expected,
                reference: reference.clone(),
            });
        }
        let gpu = measurement::capture_gpu(self);
        #[cfg(test)]
        let gate = self.document.gamma_pivot.edit_gate.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let result = async {
                let captured = reference.await.map_err(|error| {
                    format!("{error}. Turn off Hold midtone colour to edit without compensation")
                })?;
                crate::background::run(move |cancelled| {
                    #[cfg(test)]
                    if let Some(gate) = &gate
                        && !gate.is_after_work()
                    {
                        gate.wait(cancelled.clone()).map_err(Error::Gpu)?;
                    }
                    let gpu = measurement::query_gpu(gpu.as_ref())?;
                    let result = captured.evidence.compensate(
                        gpu,
                        starting_input
                            .negative_input()
                            .expect("calibrated gamma gesture"),
                        candidate
                            .negative_input()
                            .expect("calibrated gamma proposal"),
                        captured.camera_to_ap1,
                        starting_wb,
                        &|| cancelled.is_cancelled(),
                    );
                    #[cfg(test)]
                    if let Some(gate) = &gate
                        && gate.is_after_work()
                    {
                        gate.wait(cancelled).map_err(Error::Gpu)?;
                    }
                    result
                })
                .await
                .map_err(|_| "Gamma compensation worker panicked".to_owned())?
                .map_err(|error| error.to_string())
            }
            .await;
            cx.update(|editor| {
                let state = &mut editor.document.gamma_pivot;
                let pending = state.pending.take().expect("live gamma proposal");
                let project = &editor.document.state.project;
                if (project.input, project.roll.wb) != expected {
                    state.gesture = None;
                    return;
                }
                let accepted = result.and_then(|wb| {
                    let mut edit = editor
                        .edit_project("Gamma & balance")
                        .map_err(|error| error.to_string())?;
                    edit.update(|project| {
                        project.input = candidate;
                        project.roll.wb = wb;
                    });
                    Editor::validate_roll_values(&edit.preview().snapshot().unwrap())
                        .map_err(|error| error.to_string())?;
                    editor.commit_control(edit, pending.response);
                    editor.gamma_value_accepted(pending.response);
                    Ok(())
                });
                if let Err(error) = accepted {
                    editor.status = format!("Edit rejected: {error}");
                    editor.document.history.finish_control(pending.response.id);
                }
            });
        }) else {
            self.status = "Application stopped".into();
            return;
        };
        self.document.gamma_pivot.pending = Some(Pending {
            input: candidate,
            response,
            _task: task,
        });
    }
}

fn capture_reference(editor: &Editor, input: InputSettings) -> Result<ReferenceWork, String> {
    let coverage = automatic_reference(editor)?;
    let image = measurement::capture_input(editor)?;
    let gpu = measurement::capture_gpu(editor);
    let negative = input.negative_input().expect("calibrated gamma reference");
    Ok(async move {
        let (image, field) = image.await?;
        crate::background::run(move |cancelled| {
            let gpu = measurement::query_gpu(gpu.as_ref())?;
            let evidence =
                query::Evidence::capture_midtones(gpu, &image, field, coverage, negative, &|| {
                    cancelled.is_cancelled()
                })?;
            Ok::<_, Error>(Arc::new(Reference {
                camera_to_ap1: image.camera_to_ap1,
                evidence,
            }))
        })
        .await
        .map_err(|_| "Gamma reference worker panicked".to_owned())?
        .map_err(|error| error.to_string())
    }
    .boxed_local()
    .shared())
}

fn automatic_reference(editor: &Editor) -> Result<Coverage, String> {
    let project = &editor.document.state.project;
    let id = editor
        .document
        .visit
        .current_source
        .ok_or("No selected photo")?;
    let source = project.source(id).map_err(|error| error.to_string())?;
    let active: SourceRegion = source.development.active_area.into();
    if let Some(crop) = source.crop {
        return crop.coverage().map_err(|error| error.to_string());
    }
    if let Some(picture) = project
        .analysis_regions
        .iter()
        .find(|entry| entry.source == id)
        .and_then(|entry| entry.areas.picture)
        .and_then(|quad| crate::scan_regions::Areas::coverage(quad).ok())
        .filter(|coverage| coverage.bounds(active).is_ok())
    {
        return Ok(picture);
    }

    // Without picture bounds, avoid the immediate scan border without starting detection.
    let [x, y] = [active.width / 20, active.height / 20];
    Ok(Coverage::rectangle(SourceRegion {
        x: active.x + x,
        y: active.y + y,
        width: active.width - 2 * x,
        height: active.height - 2 * y,
    }))
}
