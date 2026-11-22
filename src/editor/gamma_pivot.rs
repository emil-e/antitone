use super::ui::ControlResponse;
use super::{
    Editor, InputSettings, UiContext, measurement,
    picking::{self, Display},
};
use crate::{Error, color::Balance, processing::NegativeInput};
use std::sync::Arc;

mod query;
#[cfg(test)]
mod tests;

pub(super) const DISPLAY: Display = Display {
    area: true,
    ..Display::sample("Gamma colour pivot")
};

struct Anchor {
    camera_to_ap1: glam::DMat3,
    evidence: query::Evidence,
}

/// The document owns pivot evidence and its independent calculation.
#[derive(Default)]
pub(super) struct State {
    anchor: Option<Arc<Anchor>>,
    pending: Option<Pending>,
    #[cfg(test)]
    pub(super) edit_gate: Option<measurement::testing::Gate>,
}

struct Pending {
    input: InputSettings,
    initial: (InputSettings, Balance),
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

    pub(super) fn locked(&self) -> bool {
        self.anchor.is_some()
    }
}

pub(super) fn same_gamma(a: InputSettings, b: InputSettings) -> bool {
    match (a.negative_input(), b.negative_input()) {
        (Some(a), Some(b)) => a.gamma == b.gamma && a.balance == b.balance,
        _ => a == b,
    }
}

fn basis_input(mut input: InputSettings) -> InputSettings {
    if let InputSettings::Negative { gamma, balance, .. } = &mut input {
        *gamma = crate::processing::DEFAULT_GAMMA;
        *balance = Balance::default();
    }
    input
}

impl Editor {
    pub(super) fn gamma_pivot_tool(&self, enabled: bool) -> super::ui::SectionTool<'static> {
        let choosing = self.document.visit.picker.display() == Some(DISPLAY);
        let locked = self.document.gamma_pivot.locked();
        super::ui::SectionTool {
            icon: super::ui::icons::Icon::Pin {
                active: choosing || locked,
                filled: locked && !choosing,
            },
            enabled: enabled || choosing || locked,
            name: "Gamma colour pivot",
            help: if choosing {
                "Cancel pivot selection."
            } else if locked {
                "Release the colour pivot."
            } else {
                "Pick a colour to hold while adjusting gamma."
            },
        }
    }

    pub(super) fn toggle_gamma_pivot(&mut self, cx: &UiContext<'_, Self>) {
        if self.document.visit.picker.display() == Some(DISPLAY) {
            self.finish_pick(cx);
        } else if self.document.gamma_pivot.locked() {
            self.document.gamma_pivot = State::default();
            self.status = "Gamma pivot released".into();
        } else if let Err(error) = self.start_interaction(cx, pick) {
            self.status = error;
        }
    }

    pub(super) fn gamma_compensation_required(
        &self,
        starting: InputSettings,
        proposed: InputSettings,
    ) -> bool {
        !same_gamma(starting, proposed)
            && basis_input(starting) == basis_input(proposed)
            && self.document.gamma_pivot.locked()
    }

    pub(super) fn gamma_input_pending(&self, response: ControlResponse) -> bool {
        self.document
            .gamma_pivot
            .pending
            .as_ref()
            .is_some_and(|pending| pending.response.id == response.id)
    }

    pub(super) fn gamma_control_response(&mut self, response: ControlResponse) {
        if let Some(pending) = &mut self.document.gamma_pivot.pending
            && pending.response.id == response.id
        {
            if response.cancelled {
                self.clear_gamma_proposal();
            } else {
                pending.response.active = response.active;
            }
        }
    }

    pub(super) fn gamma_control_initial(
        &self,
        response: ControlResponse,
    ) -> (InputSettings, Balance) {
        self.document
            .gamma_pivot
            .pending
            .as_ref()
            .filter(|pending| pending.response.id == response.id && !response.started)
            .map(|pending| pending.initial)
            .unwrap_or_else(|| {
                let initial = self.control_initial(response, false);
                (initial.input, initial.roll.wb)
            })
    }

    pub(super) fn clear_gamma_proposal(&mut self) {
        self.document.gamma_pivot.pending = None;
    }

    pub(super) fn compensate_gamma(
        &mut self,
        initial: (InputSettings, Balance),
        candidate: InputSettings,
        response: ControlResponse,
        cx: &UiContext<'_, Self>,
    ) {
        if let Some(pending) = &mut self.document.gamma_pivot.pending
            && pending.input == candidate
            && pending.response.id == response.id
        {
            pending.response = response;
            return;
        }
        self.clear_gamma_proposal();
        let anchor = self
            .document
            .gamma_pivot
            .anchor
            .clone()
            .expect("locked gamma pivot");
        let (starting_input, starting_wb) = initial;
        let expected = (
            self.document.state.project.input,
            self.document.state.project.roll.wb,
        );
        let gpu = measurement::capture_gpu(self);
        #[cfg(test)]
        let gate = self.document.gamma_pivot.edit_gate.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let captured = anchor;
            let result = crate::background::run(move |cancelled| {
                #[cfg(test)]
                if let Some(gate) = &gate
                    && !gate.is_after_work()
                {
                    gate.wait(cancelled.clone()).map_err(Error::Gpu)?;
                }
                let gpu = measurement::query_gpu(gpu.as_ref())?;
                let negative = |input: InputSettings| -> Result<NegativeInput, Error> {
                    input
                        .negative_input()
                        .ok_or(Error::UnusableSample("gamma pivot requires negative input"))
                };
                let result = captured.evidence.compensate(
                    gpu,
                    negative(starting_input)?,
                    negative(candidate)?,
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
            .map_err(|_| "Gamma pivot worker panicked".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            cx.update(|editor| {
                let pending = editor
                    .document
                    .gamma_pivot
                    .pending
                    .take()
                    .expect("live gamma proposal");
                if editor.document.state.project.input != expected.0
                    || editor.document.state.project.roll.wb != expected.1
                {
                    return;
                }
                let accepted = result.and_then(|wb| {
                    crate::color::wb_matrix(wb).map_err(|error| error.to_string())?;
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
            initial,
            response,
            _task: task,
        });
    }
}

pub(super) async fn pick(cx: UiContext<'_, Editor>) {
    let Some((region, captured)) = picking::pick_with(&cx, DISPLAY, |editor| {
        let source = editor
            .document
            .visit
            .current_source
            .ok_or("No selected source")?;
        Ok::<_, String>((
            measurement::capture_input(editor)?,
            source,
            editor.document.state.project.input,
            measurement::capture_gpu(editor),
        ))
    })
    .await
    else {
        return;
    };
    let (image, source, input, gpu) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(anchor) = measurement::run(&cx, DISPLAY.label, async move {
        let (image, field) = image.await?;
        measurement::query(&worker_cx, move |cancelled| {
            let gpu = measurement::query_gpu(gpu.as_ref())?;
            let evidence =
                query::Evidence::capture(gpu, &image, field, region, &|| cancelled.is_cancelled())?;
            let negative = input
                .negative_input()
                .ok_or(Error::UnusableSample("gamma pivot requires negative input"))?;
            evidence.compensate(
                gpu,
                negative,
                negative,
                image.camera_to_ap1,
                Balance::default(),
                &|| cancelled.is_cancelled(),
            )?;
            Ok(Anchor {
                camera_to_ap1: image.camera_to_ap1,
                evidence,
            })
        })
        .await
    })
    .await
    else {
        return;
    };
    cx.update(|editor| {
        if editor.document.visit.current_source == Some(source) {
            editor.document.gamma_pivot = State {
                anchor: Some(Arc::new(anchor)),
                ..Default::default()
            };
            editor.status = "Gamma pivot set".into();
        }
    });
}
