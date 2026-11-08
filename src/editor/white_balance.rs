use super::{
    AdjustmentLayerSettings, Balance, Editor, InputSettings, RollSettings, SourceId, UiContext,
    adjustment_layers::Target,
    measurement,
    picking::{self, Display, View},
};
use crate::{
    Error,
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerSnapshot},
    coverage::Coverage,
    scan::Scan,
};

mod query;

#[cfg(test)]
mod tests;

pub(super) const ROLL: Display = Display::sample("Roll WB neutral", View::Working);
pub(super) const LAYER: Display = Display {
    lock_layer_selection: true,
    ..Display::sample("WB neutral", View::Working)
};

pub(super) struct RollInput {
    pub input: InputSettings,
    pub roll: RollSettings,
}

impl RollInput {
    pub fn capture(editor: &Editor) -> Self {
        Self {
            input: editor.input(),
            roll: RollSettings {
                wb: Balance::default(),
                ..editor.document.project.roll
            },
        }
    }

    pub fn measure(
        self,
        image: &mut Scan,
        region: Coverage,
        services: &super::source::Services,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Balance, Error> {
        query::roll(services, image, region, self, cancelled)
    }
}

pub(super) struct LayerInput {
    pub input: InputSettings,
    pub roll: RollSettings,
    pub upstream: AdjustmentLayerSnapshot,
    pub layer: AdjustmentLayer,
}

impl LayerInput {
    pub fn capture(editor: &Editor) -> Result<Self, String> {
        let source = editor
            .document
            .visit
            .current_source
            .and_then(|id| editor.document.project.source(id).ok())
            .ok_or("No source for WB")?;
        let Target::Layer(id) = editor.adjustment_layer_target() else {
            return Err("Select an adjustment layer for WB".into());
        };
        let records = &source
            .adjustment_layers
            .as_ref()
            .ok_or("No adjustment layers for WB")?
            .layers;
        let index = records
            .iter()
            .position(|record| record.id == id)
            .ok_or("Unknown WB layer")?;
        let upstream = AdjustmentLayerSnapshot::new(
            records[..index]
                .iter()
                .map(|record| record.adjustment)
                .collect(),
        )
        .map_err(|error| error.to_string())?;
        let mut layer = records[index].adjustment;
        layer.photo = editor.photo_settings();
        Ok(Self {
            input: editor.input(),
            roll: editor.document.project.roll,
            upstream,
            layer,
        })
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
            RollInput::capture(editor),
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
            input.measure(&mut image, region, services, &|| cancelled.is_cancelled())
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
