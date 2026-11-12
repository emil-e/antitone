use super::{
    Balance, DVec3, Editor, InputSettings, UiContext, measurement,
    picking::{self, Display, View},
    ui,
};

mod query;

#[cfg(test)]
mod tests;

pub(super) const DMIN: Display = Display::sample("Dmin", View::Source);
pub(super) const DMAX: Display = Display::sample("Dmax", View::Source);
pub(super) const GAMMA: Display = Display::sample("Gamma neutral", View::Working);

pub(super) fn reference_control(
    ui: &mut egui::Ui,
    id: &str,
    label: &str,
    rgb: Option<DVec3>,
    enabled: bool,
    meaning: &str,
) -> bool {
    ui.push_id(id, |ui| {
        let pick = ui::labelled_row_at(
            ui,
            ui::INSPECTOR_ROW_HEIGHT,
            label,
            Some(meaning),
            |ui, _| {
                let name = format!("Pick {label}");
                ui::icons::button_at(
                    ui,
                    ui::Icon::Pick,
                    enabled,
                    &name,
                    &name,
                    ui::INSPECTOR_ROW_HEIGHT,
                )
                .clicked()
            },
        )
        .inner;

        if let Some(rgb) = rgb {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(format!(
                        "R {:.6}   G {:.6}   B {:.6}",
                        rgb.x, rgb.y, rgb.z
                    ))
                    .small(),
                )
                .truncate(),
            );
        } else {
            ui.weak("Not measured");
        }
        ui.add_space(ui::PARAMETER_GAP);
        pick
    })
    .inner
}

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
            editor
                .positive_recipe(
                    editor.input(),
                    editor.document.project_snapshot().roll,
                    editor.photo_settings(),
                )
                .map_err(|error| error.to_string())?,
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
    change_input(editor, |candidate| {
        let InputSettings::Negative { dmin, .. } = candidate else {
            return Err("Reference picks require a scan input".into());
        };
        *dmin = Some(value);
        Ok(())
    })?;
    editor.status = "Dmin measured".into();
    Ok(())
}

pub(super) fn set_dmax(editor: &mut Editor, value: DVec3) -> Result<(), String> {
    change_input(editor, |candidate| {
        let InputSettings::Negative { dmax, .. } = candidate else {
            return Err("Reference picks require a scan input".into());
        };
        *dmax = Some(value);
        Ok(())
    })?;
    editor.status = "Dmax measured".into();
    Ok(())
}

pub(super) fn set_gamma_balance(editor: &mut Editor, value: Balance) -> Result<(), String> {
    change_input(editor, |candidate| {
        let InputSettings::Negative { balance, .. } = candidate else {
            return Err("Gamma balance requires a scan input".into());
        };
        *balance = value;
        Ok(())
    })
    .map_err(|error| format!("Pick rejected: {error}"))?;
    editor.status = "Gamma neutral correction applied".into();
    Ok(())
}

fn change_input(
    editor: &mut Editor,
    change: impl FnOnce(&mut InputSettings) -> Result<(), String>,
) -> Result<(), String> {
    let mut draft = editor
        .edit_project("Calibration pick")
        .map_err(|error| error.to_string())?;
    let mut candidate = editor.document.project.input;
    change(&mut candidate)?;
    if let Some(calibration) = candidate.calibration() {
        calibration.validate().map_err(|error| error.to_string())?;
        editor
            .validate_processing(
                candidate,
                editor.document.project.roll,
                editor.photo_settings(),
            )
            .map_err(|error| error.to_string())?;
    }
    let became_ready = !editor.document.project.input.ready() && candidate.ready();
    draft.update(|project| project.input = candidate);
    editor
        .commit_project(draft)
        .map_err(|error| error.to_string())?;
    if became_ready {
        editor.document.source = false;
    }
    Ok(())
}
