use super::{
    Balance, DVec3, Editor, InputSettings, UiContext, measurement,
    picking::{self, Display, View},
};

mod query;

#[cfg(test)]
mod tests;

pub(super) const DMIN: Display = Display::sample("Dmin", View::Source);
pub(super) const DMAX: Display = Display::sample("Dmax", View::Source);
pub(super) const GAMMA: Display = Display::sample("Gamma neutral", View::Working);

pub(super) async fn pick_dmin(cx: UiContext<'_, Editor>) {
    let captured = cx.read(|editor| {
        Ok::<_, String>((
            measurement::capture_input(editor)?,
            editor.source_services.clone(),
        ))
    });
    let (image, services) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(region) = picking::pick(&cx, DMIN).await else {
        return;
    };
    let Some(value) = measurement::run(&cx, DMIN.label, async move {
        let mut image = image.await?;
        measurement::query(&worker_cx, move |cancelled| {
            let services = measurement::query_services(services.as_ref())?;
            query::reference(services, &mut image, region, &|| cancelled.is_cancelled())
        })
        .await
    })
    .await
    else {
        return;
    };
    cx.update(|editor| {
        if let Err(error) = set_dmin(editor, value) {
            editor.status = error;
        }
    });
}

pub(super) async fn pick_dmax(cx: UiContext<'_, Editor>) {
    let captured = cx.read(|editor| {
        Ok::<_, String>((
            measurement::capture_input(editor)?,
            editor.source_services.clone(),
        ))
    });
    let (image, services) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(region) = picking::pick(&cx, DMAX).await else {
        return;
    };
    let Some(value) = measurement::run(&cx, DMAX.label, async move {
        let mut image = image.await?;
        measurement::query(&worker_cx, move |cancelled| {
            let services = measurement::query_services(services.as_ref())?;
            query::reference(services, &mut image, region, &|| cancelled.is_cancelled())
        })
        .await
    })
    .await
    else {
        return;
    };
    cx.update(|editor| {
        if let Err(error) = set_dmax(editor, value) {
            editor.status = error;
        }
    });
}

pub(super) async fn pick_gamma(cx: UiContext<'_, Editor>) {
    let captured = cx.read(|editor| {
        Ok::<_, String>((
            measurement::capture_input(editor)?,
            editor.source_services.clone(),
            editor.input(),
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
    let Some(region) = picking::pick(&cx, GAMMA).await else {
        return;
    };
    let Some(value) = measurement::run(&cx, GAMMA.label, async move {
        let mut image = image.await?;
        measurement::query(&worker_cx, move |cancelled| {
            let services = measurement::query_services(services.as_ref())?;
            query::gamma(services, &mut image, region, input, &|| {
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
        if let Err(error) = set_gamma_balance(editor, value) {
            editor.status = error;
        }
    });
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
