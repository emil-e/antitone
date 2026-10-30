use super::{
    AdjustmentLayerSettings, Balance, Editor, InputSettings, RollSettings, UiContext,
    adjustment_layers::Target,
    picking::{Display, View, measurement},
};
use crate::{
    Error,
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerSnapshot},
    coverage::Coverage,
    scan::Scan,
};
use async_executor::Task;

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
                ..editor.document.roll
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
        layer.photo = editor.document.visit.adjustment_layer_settings;
        Ok(Self {
            input: editor.input(),
            roll: editor.document.roll,
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

pub(super) fn pick_roll<'scope>(
    editor: &mut Editor,
    cx: &UiContext<'scope, Editor>,
) -> Result<Task<()>, String> {
    let mut image = measurement::capture_scan(editor)?;
    let services = editor.source_services.clone();
    let input = RollInput::capture(editor);
    cx.spawn(move |cx| async move {
        let Some(value) = measurement::pick(
            ROLL,
            move |region, cancelled| {
                let services = measurement::query_services(services.as_ref())?;
                input.measure(&mut image, region, services, &|| cancelled.is_cancelled())
            },
            cx.clone(),
        )
        .await
        else {
            return;
        };
        cx.update(|editor| {
            if let Err(error) = set_roll(editor, value) {
                editor.status = error;
            }
        });
    })
    .ok_or_else(|| "Application stopped".into())
}

pub(super) fn pick_layer<'scope>(
    editor: &mut Editor,
    cx: &UiContext<'scope, Editor>,
) -> Result<Task<()>, String> {
    let mut image = measurement::capture_scan(editor)?;
    let services = editor.source_services.clone();
    let input = LayerInput::capture(editor)?;
    let target = (
        editor.document.visit.current_source,
        editor.adjustment_layer_target(),
    );
    cx.spawn(move |cx| async move {
        // Selection can change before this first poll opens and locks the picker.
        if cx.read(|editor| {
            (
                editor.document.visit.current_source,
                editor.adjustment_layer_target(),
            )
        }) != target
        {
            return;
        }
        let Some(value) = measurement::pick(
            LAYER,
            move |region, cancelled| {
                let services = measurement::query_services(services.as_ref())?;
                input.measure(&mut image, region, services, &|| cancelled.is_cancelled())
            },
            cx.clone(),
        )
        .await
        else {
            return;
        };
        cx.update(|editor| {
            if let Err(error) = set_layer(editor, value) {
                editor.status = error;
            }
        });
    })
    .ok_or_else(|| "Application stopped".into())
}

pub(super) fn set_roll(editor: &mut Editor, value: Balance) -> Result<(), String> {
    let candidate = RollSettings {
        wb: value,
        ..editor.document.roll
    };
    validate(
        editor,
        candidate,
        editor.document.visit.adjustment_layer_settings,
    )?;
    let before = editor.history_snapshot();
    editor.document.roll = candidate;
    editor.record_edit(before, "Calibration pick", None);
    editor.status = "RollWb neutral correction applied".into();
    Ok(())
}

pub(super) fn set_layer(editor: &mut Editor, value: Balance) -> Result<(), String> {
    let candidate = AdjustmentLayerSettings {
        wb: value,
        ..editor.document.visit.adjustment_layer_settings
    };
    validate(editor, editor.document.roll, candidate)?;
    let before = editor.history_snapshot();
    editor.document.visit.adjustment_layer_settings = candidate;
    editor.record_edit(
        before,
        "Calibration pick",
        editor.document.visit.current_source,
    );
    editor.status = "Wb neutral correction applied".into();
    Ok(())
}

fn validate(
    editor: &Editor,
    roll: RollSettings,
    photo: AdjustmentLayerSettings,
) -> Result<(), String> {
    if editor.document.input.ready() {
        editor
            .validate_processing(editor.document.input, roll, photo)
            .map_err(|error| format!("Pick rejected: {error}"))?;
    }
    Ok(())
}
