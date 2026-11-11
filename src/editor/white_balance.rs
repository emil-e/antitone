use super::{
    AdjustmentLayerSettings, Balance, Editor, RollSettings, SourceId, UiContext,
    adjustment_layers::Target,
    measurement,
    picking::{self, Display, View},
};
use crate::{Error, adjustment_layers::AdjustmentLayerSnapshot, coverage::Coverage, scan::Scan};
#[cfg(test)]
use crate::{adjustment_layers::AdjustmentLayer, processing::InputSettings};

mod query;

#[cfg(test)]
mod tests;

pub(super) const ROLL: Display = Display::sample("Roll WB neutral", View::Working);
pub(super) const LAYER: Display = Display {
    lock_layer_selection: true,
    ..Display::sample("WB neutral", View::Working)
};

pub(super) fn roll_input(editor: &Editor) -> Result<crate::image_recipe::ImageRecipe, String> {
    let mut recipe = editor
        .positive_recipe(
            editor.input(),
            editor.document.project_snapshot().roll,
            editor.photo_settings(),
        )
        .map_err(|error| error.to_string())?;
    recipe.roll.wb = Balance::default();
    Ok(recipe)
}

pub(super) struct LayerInput {
    pub recipe: crate::image_recipe::ImageRecipe,
    pub index: usize,
}

impl LayerInput {
    pub fn capture(editor: &Editor) -> Result<Self, String> {
        let recipe = editor
            .positive_recipe(
                editor.input(),
                editor.document.project_snapshot().roll,
                editor.photo_settings(),
            )
            .map_err(|error| error.to_string())?;
        let project = editor.document.project_snapshot();
        let source = project
            .source(recipe.neutral.source)
            .map_err(|error| error.to_string())?;
        let Target::Layer(id) = editor.adjustment_layer_target() else {
            return Err("Select an adjustment layer for WB".into());
        };
        let index = source
            .adjustment_layers
            .as_ref()
            .and_then(|layers| layers.layers.iter().position(|record| record.id == id))
            .ok_or("Unknown WB layer")?;
        Ok(Self { recipe, index })
    }

    pub fn measure(
        self,
        image: &mut Scan,
        region: Coverage,
        services: &super::source::Services,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Balance, Error> {
        query::layer(services, image, region, self, cancelled)
    }
}

pub(super) async fn pick_roll(cx: UiContext<'_, Editor>) {
    let captured = cx.read(|editor| {
        Ok::<_, String>((
            measurement::capture_input(editor)?,
            editor.source_services.clone(),
            roll_input(editor)?,
        ))
    });
    let (image, services, input) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(region) = picking::pick(&cx, ROLL).await else {
        return;
    };
    let Some(value) = measurement::run(&cx, ROLL.label, async move {
        let mut image = image.await?;
        measurement::query(&worker_cx, move |cancelled| {
            let services = measurement::query_services(services.as_ref())?;
            query::roll(services, &mut image, region, input, &|| {
                cancelled.is_cancelled()
            })
        })
        .await
    })
    .await
    else {
        return;
    };
    cx.update(|editor| {
        if let Err(error) = set_roll(editor, value) {
            editor.status = error;
        }
    });
}

pub(super) async fn pick_layer(cx: UiContext<'_, Editor>) {
    let captured = cx.read(|editor| {
        Ok::<_, String>((
            measurement::capture_input(editor)?,
            editor.source_services.clone(),
            LayerInput::capture(editor)?,
            (
                editor
                    .document
                    .visit
                    .current_source
                    .ok_or("No selected source")?,
                editor.adjustment_layer_target(),
            ),
        ))
    });
    let (image, services, input, target) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(region) = picking::pick(&cx, LAYER).await else {
        return;
    };
    let Some(value) = measurement::run(&cx, LAYER.label, async move {
        let mut image = image.await?;
        measurement::query(&worker_cx, move |cancelled| {
            let services = measurement::query_services(services.as_ref())?;
            input.measure(&mut image, region, services, &|| cancelled.is_cancelled())
        })
        .await
    })
    .await
    else {
        return;
    };
    cx.update(|editor| {
        if let Err(error) = set_layer(editor, value, target) {
            editor.status = error;
        }
    });
}

pub(super) fn set_roll(editor: &mut Editor, value: Balance) -> Result<(), String> {
    let mut draft = editor
        .edit_project("Calibration pick")
        .map_err(|error| error.to_string())?;
    let candidate = RollSettings {
        wb: value,
        ..editor.document.project.roll
    };
    validate(editor, candidate, editor.photo_settings())?;
    draft.update(|project| project.roll = candidate);
    editor
        .commit_project(draft)
        .map_err(|error| error.to_string())?;
    editor.status = "RollWb neutral correction applied".into();
    Ok(())
}

pub(super) fn set_layer(
    editor: &mut Editor,
    value: Balance,
    target: (SourceId, Target),
) -> Result<(), String> {
    if editor.document.visit.current_source != Some(target.0) {
        return Err("WB pick source is no longer selected".into());
    }
    let Target::Layer(id) = target.1 else {
        return Err("WB pick has no target layer".into());
    };
    editor
        .edit_adjustment_layers_named("Calibration pick", |layers| {
            let layer = layers
                .layers
                .iter_mut()
                .find(|layer| layer.id == id)
                .ok_or(Error::InvalidParameter(
                    "WB pick target layer no longer exists",
                ))?;
            layer.adjustment.photo.wb = value;
            Ok(())
        })
        .map_err(|error| format!("Pick rejected: {error}"))?;
    editor.status = "Wb neutral correction applied".into();
    Ok(())
}

fn validate(
    editor: &Editor,
    roll: RollSettings,
    photo: AdjustmentLayerSettings,
) -> Result<(), String> {
    crate::color::wb_matrix(roll.wb).map_err(|error| format!("Pick rejected: {error}"))?;
    if editor.document.project.input.ready() {
        editor
            .validate_processing(editor.document.project.input, roll, photo)
            .map_err(|error| format!("Pick rejected: {error}"))?;
    }
    Ok(())
}
