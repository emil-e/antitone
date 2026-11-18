use super::ui::ControlResponse;
use super::{
    Editor, InputSettings, UiContext, measurement,
    picking::{self, Display, View},
};
use crate::{Error, color::Balance, image_recipe::NeutralRecipe, processing::NegativeInput};
use std::sync::Arc;

mod query;
#[cfg(test)]
mod tests;

pub(super) const DISPLAY: Display = Display {
    area: true,
    ..Display::sample("Gamma colour pivot", View::Working)
};

struct Anchor {
    recipe: NeutralRecipe,
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
    fn pivot_recipe(&self, source: crate::project::SourceId) -> Result<NeutralRecipe, Error> {
        Ok(self
            .positive_recipe_for_source(
                self.document.state.project.input,
                self.document.project.roll,
                self.document.project.source(source)?,
                self.photo_settings(),
            )?
            .neutral
            .clone())
    }

    fn pivot_compatible(&self, anchor: &Anchor) -> bool {
        self.document.project.order.contains(&anchor.recipe.source)
            && self
                .document
                .project
                .source(anchor.recipe.source)
                .is_ok_and(|source| {
                    self.document.development_for(source, self.input())
                        == anchor.recipe.source_development
                        && basis_input(self.document.state.project.input)
                            == basis_input(anchor.recipe.input)
                        && self
                            .document
                            .correction
                            .prepared()
                            .map(|correction| &correction.settings.map)
                            == anchor.recipe.correction.as_ref()
                })
    }

    pub(super) fn observe_gamma_pivot(&mut self) {
        if self
            .document
            .gamma_pivot
            .anchor
            .as_ref()
            .is_some_and(|anchor| !self.pivot_compatible(anchor))
        {
            self.document.gamma_pivot = State::default();
            self.status = "Gamma pivot released because its measurement basis changed".into();
        }
    }

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
            && self
                .document
                .gamma_pivot
                .anchor
                .as_ref()
                .is_some_and(|anchor| self.pivot_compatible(anchor))
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
            .expect("compatible gamma pivot");
        let (starting_input, starting_wb) = initial;
        let expected = (
            self.document.state.project.input,
            self.document.state.project.roll.wb,
        );
        let services = self.source_services.clone();
        #[cfg(test)]
        let gate = self.document.gamma_pivot.edit_gate.clone();
        let Some(task) = cx.spawn(move |cx| async move {
            let captured = anchor.clone();
            let result = crate::background::run(move |cancelled| {
                #[cfg(test)]
                if let Some(gate) = &gate
                    && !gate.is_after_work()
                {
                    gate.wait(cancelled.clone()).map_err(Error::Gpu)?;
                }
                let services = measurement::query_services(services.as_ref())?;
                let negative = |input: InputSettings| -> Result<NegativeInput, Error> {
                    let mut recipe = captured.recipe.clone();
                    recipe.input = input;
                    recipe
                        .input
                        .negative_input()
                        .ok_or(Error::UnusableSample("gamma pivot basis changed"))
                };
                let result = captured.evidence.compensate(
                    services,
                    negative(starting_input)?,
                    negative(candidate)?,
                    captured.recipe.processing_transform()?.1,
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
                if !editor.pivot_compatible(&anchor) {
                    editor.observe_gamma_pivot();
                    return;
                }
                if !same_gamma(editor.document.state.project.input, expected.0)
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
            editor
                .pivot_recipe(source)
                .map_err(|error| error.to_string())?,
            editor.source_services.clone(),
        ))
    })
    .await
    else {
        return;
    };
    let (image, source, recipe, services) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let captured = recipe.clone();
    let worker_cx = cx.clone();
    let Some(evidence) = measurement::run(&cx, DISPLAY.label, async move {
        let image = image.await?;
        measurement::query(&worker_cx, move |cancelled| {
            let services = measurement::query_services(services.as_ref())?;
            let evidence =
                query::Evidence::capture(services, &image, region, &|| cancelled.is_cancelled())?;
            let negative = captured
                .input
                .negative_input()
                .ok_or(Error::UnusableSample("gamma pivot basis changed"))?;
            evidence.compensate(
                services,
                negative,
                negative,
                image.camera_to_ap1,
                Balance::default(),
                &|| cancelled.is_cancelled(),
            )?;
            Ok(evidence)
        })
        .await
    })
    .await
    else {
        return;
    };
    cx.update(|editor| {
        let anchor = Arc::new(Anchor { recipe, evidence });
        if editor.document.visit.current_source == Some(source) {
            if editor.pivot_compatible(&anchor) {
                editor.document.gamma_pivot = State {
                    anchor: Some(anchor),
                    ..Default::default()
                };
                editor.status = "Gamma pivot set".into();
            } else {
                editor.status = "gamma pivot basis changed".into();
            }
        }
    });
}
