use super::{
    Editor, InputSettings, UiContext,
    editing::ControlSettings,
    measurement,
    picking::{self, Display, View},
};
use crate::{Error, color::Balance, image_recipe::NeutralRecipe, processing::NegativeInput};
use async_executor::Task;
use std::{rc::Rc, sync::Arc};

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

/// The document retains accepted evidence independently of the photo visit and picker.
#[derive(Default)]
pub(super) struct State {
    anchor: Option<Arc<Anchor>>,
    edit: Option<Edit>,
    #[cfg(test)]
    pub(super) edit_gate: Option<measurement::testing::Gate>,
}

struct Target {
    anchor: Arc<Anchor>,
    input: InputSettings,
    wb: Balance,
    group: Option<egui::Id>,
}

struct Edit {
    target: Rc<Target>,
    requested: InputSettings,
    task: Option<Task<()>>,
    released: bool,
}

impl State {
    pub(super) fn locked(&self) -> bool {
        self.anchor.is_some()
    }

    pub(super) fn pending(&self) -> bool {
        self.edit.as_ref().is_some_and(|edit| edit.task.is_some())
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
                self.document.input,
                self.document.roll,
                self.document.project.source(source)?,
                self.document.visit.adjustment_layer_settings,
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
                        && basis_input(self.document.input) == basis_input(anchor.recipe.input)
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
            self.document.history.finish_group();
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
        if self.document.visit.picker.display() == Some(DISPLAY) {
            self.finish_pick(cx);
        } else if self.document.gamma_pivot.anchor.take().is_some() {
            self.finish_gamma_edit();
            self.status = "Gamma pivot released".into();
        } else {
            if let Err(error) = self.start_interaction(cx, pick) {
                self.status = error;
            }
        }
    }

    pub(super) fn gamma_pivot_input(&self) -> InputSettings {
        self.document
            .gamma_pivot
            .edit
            .as_ref()
            .map_or(self.document.input, |edit| edit.requested)
    }

    pub(super) fn finish_gamma_edit(&mut self) {
        self.document.gamma_pivot.edit = None;
        self.document.history.finish_group();
    }

    pub(super) fn gamma_gesture_frame(&mut self, down: bool) {
        if let Some(edit) = &mut self.document.gamma_pivot.edit {
            edit.released |= !down;
            if edit.released && edit.task.is_none() {
                self.finish_gamma_edit();
            }
        }
    }

    /// All manual controls, including deferred resets, enter here before history or rendering.
    pub(super) fn accept_manual_controls(
        &mut self,
        previous: ControlSettings,
        candidate: ControlSettings,
        gamma_edited: bool,
        group: Option<egui::Id>,
        cx: &UiContext<'_, Self>,
    ) -> Result<(), Error> {
        if !gamma_edited {
            self.validate_control_edit(previous, candidate)?;
            candidate.apply(self);
            if previous.roll.wb != candidate.roll.wb {
                self.finish_gamma_edit();
            }
            self.observe_gamma_pivot();
            return Ok(());
        }
        let Some(anchor) = self.document.gamma_pivot.anchor.clone() else {
            self.validate_control_edit(previous, candidate)?;
            candidate.apply(self);
            return Ok(());
        };
        if !self.pivot_compatible(&anchor)
            || basis_input(previous.input) != basis_input(candidate.input)
        {
            self.validate_control_edit(previous, candidate)?;
            candidate.apply(self);
            self.observe_gamma_pivot();
            return Ok(());
        }
        if same_gamma(previous.input, candidate.input) {
            if let Some(edit) = &mut self.document.gamma_pivot.edit
                && edit.requested != candidate.input
            {
                edit.task = None;
                edit.requested = candidate.input;
            }
            if previous.roll.wb != candidate.roll.wb {
                self.finish_gamma_edit();
            }
            self.validate_control_edit(previous, candidate)?;
            candidate.apply(self);
            return Ok(());
        }
        // Keep accepted controls coherent while the shared f32 forward model runs off-thread.
        previous.apply(self);
        if self
            .document
            .gamma_pivot
            .edit
            .as_ref()
            .is_some_and(|edit| edit.requested == candidate.input && edit.task.is_some())
        {
            return Ok(());
        }
        let extend = group.is_some()
            && self
                .document
                .gamma_pivot
                .edit
                .as_ref()
                .is_some_and(|edit| edit.target.group == group && !edit.released);
        if !extend {
            self.finish_gamma_edit();
            self.document.gamma_pivot.edit = Some(Edit {
                target: Rc::new(Target {
                    anchor: anchor.clone(),
                    input: previous.input,
                    wb: previous.roll.wb,
                    group,
                }),
                requested: candidate.input,
                task: None,
                released: group.is_none(),
            });
        }
        let edit = self
            .document
            .gamma_pivot
            .edit
            .as_mut()
            .expect("manual gamma edit");
        edit.requested = candidate.input;
        edit.task = None;
        let target = edit.target.clone();
        let services = self.source_services.clone();
        #[cfg(test)]
        let gate = self.document.gamma_pivot.edit_gate.clone();
        let to = candidate.input;
        let task = cx.spawn(move |cx| async move {
            let anchor = target.anchor.clone();
            let from = target.input;
            let wb = target.wb;
            let result = crate::background::run(move |cancelled| {
                #[cfg(test)]
                if let Some(gate) = &gate
                    && !gate.is_after_work()
                {
                    gate.wait(cancelled.clone()).map_err(Error::Gpu)?;
                }
                let services = measurement::query_services(services.as_ref())?;
                let negative = |input: InputSettings| -> Result<NegativeInput, Error> {
                    let mut recipe = anchor.recipe.clone();
                    recipe.input = input;
                    recipe
                        .processing_input()?
                        .negative_input()
                        .ok_or(Error::UnusableSample("gamma pivot basis changed"))
                };
                let result = anchor.evidence.compensate(
                    services,
                    negative(from)?,
                    negative(to)?,
                    glam::DMat3::from_cols_array_2d(
                        &anchor.recipe.source_development.camera_to_ap1,
                    ),
                    wb,
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
            cx.update(|editor| editor.accept_gamma_result(&target, candidate, result));
        });
        self.document.gamma_pivot.edit.as_mut().unwrap().task = task;
        Ok(())
    }

    fn accept_gamma_result(
        &mut self,
        target: &Rc<Target>,
        mut candidate: ControlSettings,
        result: Result<Balance, String>,
    ) {
        if !self
            .document
            .gamma_pivot
            .edit
            .as_ref()
            .is_some_and(|edit| Rc::ptr_eq(&edit.target, target))
        {
            return;
        }
        if !self.pivot_compatible(&target.anchor) {
            self.observe_gamma_pivot();
            self.finish_gamma_edit();
            return;
        }
        // Browsing and independent downstream edits may have advanced while the worker ran.
        // Only the proposed gamma and its compensating WB belong to this acceptance.
        let input = candidate.input;
        candidate = ControlSettings::capture(self);
        candidate.input = input;
        let result = result.and_then(|wb| {
            candidate.roll.wb = wb;
            self.validate_control_edit(ControlSettings::capture(self), candidate)
                .map_err(|error| error.to_string())?;
            let before = self.history_snapshot();
            candidate.apply(self);
            self.record_gamma_edit(before, target.group);
            Ok(())
        });
        let edit = self.document.gamma_pivot.edit.as_mut().unwrap();
        edit.task = None;
        if let Err(error) = result {
            edit.requested = self.document.input;
            self.status = format!("Edit rejected: {error}");
        }
        if edit.released {
            self.finish_gamma_edit();
        }
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
                .processing_input()?
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
                editor.finish_gamma_edit();
                editor.document.gamma_pivot.anchor = Some(anchor);
                editor.status = "Gamma pivot set".into();
            } else {
                editor.status = "gamma pivot basis changed".into();
            }
        }
    });
}
