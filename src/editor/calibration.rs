use super::{
    Balance, DVec3, Editor, InputSettings, UiContext,
    picking::{Display, View, measurement},
};
use async_executor::Task;

mod query;

#[cfg(test)]
mod tests;

pub(super) const DMIN: Display = Display::sample("Dmin", View::Source);
pub(super) const DMAX: Display = Display::sample("Dmax", View::Source);
pub(super) const GAMMA: Display = Display::sample("Gamma neutral", View::Working);

pub(super) fn pick_dmin<'scope>(
    editor: &mut Editor,
    cx: &UiContext<'scope, Editor>,
) -> Result<Task<()>, String> {
    let mut image = measurement::capture_scan(editor)?;
    let services = editor.source_services.clone();
    cx.spawn(move |cx| async move {
        let Some(value) = measurement::pick(
            DMIN,
            move |region, cancelled| {
                let services = measurement::query_services(services.as_ref())?;
                query::reference(services, &mut image, region, &|| cancelled.is_cancelled())
            },
            cx.clone(),
        )
        .await
        else {
            return;
        };
        cx.update(|editor| {
            if let Err(error) = set_dmin(editor, value) {
                editor.status = error;
            }
        });
    })
    .ok_or_else(|| "Application stopped".into())
}

pub(super) fn pick_dmax<'scope>(
    editor: &mut Editor,
    cx: &UiContext<'scope, Editor>,
) -> Result<Task<()>, String> {
    let mut image = measurement::capture_scan(editor)?;
    let services = editor.source_services.clone();
    cx.spawn(move |cx| async move {
        let Some(value) = measurement::pick(
            DMAX,
            move |region, cancelled| {
                let services = measurement::query_services(services.as_ref())?;
                query::reference(services, &mut image, region, &|| cancelled.is_cancelled())
            },
            cx.clone(),
        )
        .await
        else {
            return;
        };
        cx.update(|editor| {
            if let Err(error) = set_dmax(editor, value) {
                editor.status = error;
            }
        });
    })
    .ok_or_else(|| "Application stopped".into())
}

pub(super) fn pick_gamma<'scope>(
    editor: &mut Editor,
    cx: &UiContext<'scope, Editor>,
) -> Result<Task<()>, String> {
    let mut image = measurement::capture_scan(editor)?;
    let services = editor.source_services.clone();
    let input = editor.input();
    cx.spawn(move |cx| async move {
        let Some(value) = measurement::pick(
            GAMMA,
            move |region, cancelled| {
                let services = measurement::query_services(services.as_ref())?;
                query::gamma(services, &mut image, region, input, &|| {
                    cancelled.is_cancelled()
                })
            },
            cx.clone(),
        )
        .await
        else {
            return;
        };
        cx.update(|editor| {
            if let Err(error) = set_gamma_balance(editor, value) {
                editor.status = error;
            }
        });
    })
    .ok_or_else(|| "Application stopped".into())
}

pub(super) fn set_dmin(editor: &mut Editor, value: DVec3) -> Result<(), String> {
    let mut candidate = editor.document.input;
    let InputSettings::Negative { dmin, .. } = &mut candidate else {
        return Err("Reference picks require a scan input".into());
    };
    *dmin = Some(value);
    change_input(editor, candidate)?;
    editor.status = "Dmin measured".into();
    Ok(())
}

pub(super) fn set_dmax(editor: &mut Editor, value: DVec3) -> Result<(), String> {
    let mut candidate = editor.document.input;
    let InputSettings::Negative { dmax, .. } = &mut candidate else {
        return Err("Reference picks require a scan input".into());
    };
    *dmax = Some(value);
    change_input(editor, candidate)?;
    editor.status = "Dmax measured".into();
    Ok(())
}

pub(super) fn set_gamma_balance(editor: &mut Editor, value: Balance) -> Result<(), String> {
    let mut candidate = editor.document.input;
    let InputSettings::Negative { balance, .. } = &mut candidate else {
        return Err("Gamma balance requires a scan input".into());
    };
    *balance = value;
    change_input(editor, candidate).map_err(|error| format!("Pick rejected: {error}"))?;
    editor.status = "Gamma neutral correction applied".into();
    Ok(())
}

fn change_input(editor: &mut Editor, candidate: InputSettings) -> Result<(), String> {
    if let Some(calibration) = candidate.calibration() {
        calibration.validate().map_err(|error| error.to_string())?;
        editor
            .validate_processing(
                candidate,
                editor.document.roll,
                editor.document.visit.adjustment_layer_settings,
            )
            .map_err(|error| error.to_string())?;
    }
    let became_ready = !editor.document.input.ready() && candidate.ready();
    let before = editor.history_snapshot();
    editor.document.input = candidate;
    if became_ready {
        editor.document.source = false;
    }
    editor.record_edit(before, "Calibration pick", None);
    Ok(())
}
