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

/// Accepted evidence is document-owned; unfinished manual values belong to their edit.
#[derive(Default)]
pub(super) struct State {
    anchor: Option<Arc<Anchor>>,
    #[cfg(test)]
    pub(super) edit_gate: Option<measurement::testing::Gate>,
}

impl State {
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
                    source.development == anchor.recipe.source_development
                        && basis_input(self.document.state.project.input)
                            == basis_input(anchor.recipe.input)
                        && self
                            .document
                            .correction
                            .as_ref()
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
            self.cancel_control_edit();
            self.status = "Gamma pivot released because its measurement basis changed".into();
        }
    }

    pub(super) fn gamma_pivot_tool(
        &self,
        enabled: bool,
    ) -> super::presentation::SectionTool<'static> {
        let choosing = self.document.visit.picker.display() == Some(DISPLAY);
        let locked = self.document.gamma_pivot.locked();
        super::presentation::SectionTool {
            icon: super::icons::Icon::Pin {
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
        self.cancel_control_edit();
        if self.document.visit.picker.display() == Some(DISPLAY) {
            self.finish_pick(cx);
        } else if self.document.gamma_pivot.anchor.take().is_some() {
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

    pub(super) fn compensate_control_edit(&mut self, cx: &UiContext<'_, Self>) {
        let anchor = self
            .document
            .gamma_pivot
            .anchor
            .clone()
            .expect("compatible gamma pivot");
        let edit = self
            .document
            .controls_edit
            .as_mut()
            .expect("owned document edit");
        edit.calculation = None;
        edit.values.rename("Gamma & balance");
        let preview = edit.values.preview();
        let starting = edit.values.initial();
        let candidate = edit.gamma_request.expect("requested gamma");
        let services = self.source_services.clone();
        #[cfg(test)]
        let gate = self.document.gamma_pivot.edit_gate.clone();
        let task = cx.spawn(move |cx| async move {
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
                    negative(starting.input)?,
                    negative(candidate)?,
                    glam::DMat3::from_cols_array_2d(
                        &captured.recipe.source_development.camera_to_ap1,
                    ),
                    starting.roll.wb,
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
                if !preview.active() || !editor.document.project_preview.same_edit(&preview) {
                    return;
                }
                if !editor.pivot_compatible(&anchor) {
                    editor.observe_gamma_pivot();
                    return;
                }
                let result = result.and_then(|wb| {
                    crate::color::wb_matrix(wb).map_err(|error| error.to_string())?;
                    let mut edit = editor
                        .document
                        .controls_edit
                        .take()
                        .expect("live edit owner");
                    if edit.gamma_request != Some(candidate) {
                        editor.document.controls_edit = Some(edit);
                        return Ok(());
                    }
                    edit.values.update(|project| {
                        project.input = candidate;
                        project.roll.wb = wb;
                    });
                    editor
                        .validate_roll_values(&edit.values.preview().snapshot().unwrap())
                        .map_err(|error| error.to_string())?;
                    edit.gamma_request = None;
                    edit.calculation = None;
                    editor.document.controls_edit = Some(edit);
                    Ok(())
                });
                if let Err(error) = result {
                    editor.document.controls_edit = None;
                    editor.status = format!("Edit rejected: {error}");
                } else {
                    editor.finish_document_controls();
                }
            });
        });
        self.document.controls_edit.as_mut().unwrap().calculation = task;
    }
}

pub(super) async fn pick(cx: UiContext<'_, Editor>) {
    let captured = cx.read(|editor| {
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
    });
    let (image, source, recipe, services) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let captured = recipe.clone();
    let worker_cx = cx.clone();
    let Some(region) = picking::pick(&cx, DISPLAY).await else {
        return;
    };
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
                editor.cancel_control_edit();
                editor.document.gamma_pivot.anchor = Some(anchor);
                editor.status = "Gamma pivot set".into();
            } else {
                editor.status = "gamma pivot basis changed".into();
            }
        }
    });
}
