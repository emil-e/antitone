use super::{
    Balance, DVec3, Editor, InputSettings, UiContext, measurement,
    picking::{self, Display},
    ui, white_balance,
};

mod query;

#[cfg(test)]
mod tests;

pub(super) const DMIN: Display = Display::sample("Dmin");
pub(super) const DMAX: Display = Display::sample("Dmax");
pub(super) const GAMMA: Display = Display {
    help: Some(
        "Click a neutral patch, or drag a rectangle and choose Apply, to set gamma balance and Roll WB.",
    ),
    ..Display::sample("Roll balance")
};
pub(super) const GAMMA_PICK_LABEL: &str = "Balance roll from neutral patch";

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
                let pick = ui::icons::button_at(
                    ui,
                    ui::Icon::Pick,
                    enabled,
                    &name,
                    &name,
                    ui::INSPECTOR_ROW_HEIGHT,
                )
                .clicked();
                if let Some(rgb) = rgb {
                    let height = ui::INSPECTOR_ROW_HEIGHT * (2. / 3.);
                    let response = egui::color_picker::show_color(
                        ui,
                        reference_swatch(rgb),
                        egui::vec2(2. * height, height),
                    );
                    ui.painter().rect_stroke(
                        response.rect,
                        0.,
                        ui.visuals().widgets.noninteractive.bg_stroke,
                        egui::StrokeKind::Inside,
                    );
                    response.on_hover_text("Approximate camera RGB preview.");
                }
                pick
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

fn reference_swatch(rgb: DVec3) -> egui::Color32 {
    // Peak-channel Reinhard with one stop of preview gain. A common scale preserves
    // camera-channel ratios; there is no source profile attached to a roll anchor.
    let display = (rgb / (0.5 + rgb.max_element())).as_vec3();
    egui::Rgba::from_rgb(display.x, display.y, display.z).into()
}

pub(super) async fn pick_dmin(cx: UiContext<'_, Editor>) {
    let Some((region, captured)) = picking::pick_with(&cx, DMIN, |editor| {
        Ok::<_, String>((
            editor.capture_source_context()?,
            measurement::capture_gpu(editor),
        ))
    })
    .await
    else {
        return;
    };
    let (source, gpu) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(value) = measurement::run(&cx, DMIN.label, async move {
        let context = source.await?;
        let field = context
            .correction
            .gpu()
            .map_err(|error| error.to_string())?;
        measurement::query(&worker_cx, move |cancelled| {
            let gpu = measurement::query_gpu(gpu.as_ref())?;
            query::reference(gpu, &context.source, field, region, &|| {
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
        if let Err(error) = set_dmin(editor, value) {
            editor.status = error;
        }
    });
}

pub(super) async fn pick_dmax(cx: UiContext<'_, Editor>) {
    let Some((region, captured)) = picking::pick_with(&cx, DMAX, |editor| {
        Ok::<_, String>((
            editor.capture_source_context()?,
            measurement::capture_gpu(editor),
        ))
    })
    .await
    else {
        return;
    };
    let (source, gpu) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(value) = measurement::run(&cx, DMAX.label, async move {
        let context = source.await?;
        let field = context
            .correction
            .gpu()
            .map_err(|error| error.to_string())?;
        measurement::query(&worker_cx, move |cancelled| {
            let gpu = measurement::query_gpu(gpu.as_ref())?;
            query::reference(gpu, &context.source, field, region, &|| {
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
        if let Err(error) = set_dmax(editor, value) {
            editor.status = error;
        }
    });
}

pub(super) async fn pick_gamma(cx: UiContext<'_, Editor>) {
    let Some((region, captured)) = picking::pick_with(&cx, GAMMA, |editor| {
        if editor.input().calibration().is_none() {
            return Err("Measure Dmin and Dmax before balancing the roll".into());
        }
        Ok::<_, String>((
            editor.capture_source_context()?,
            measurement::capture_gpu(editor),
            white_balance::roll_input(editor)?,
        ))
    })
    .await
    else {
        return;
    };
    let (source, gpu, input) = match captured {
        Ok(input) => input,
        Err(error) => {
            cx.update(|editor| editor.status = error);
            return;
        }
    };
    let worker_cx = cx.clone();
    let Some(fit) = measurement::run(&cx, GAMMA.label, async move {
        let context = source.await?;
        let field = context
            .correction
            .gpu()
            .map_err(|error| error.to_string())?;
        measurement::query(&worker_cx, move |cancelled| {
            let gpu = measurement::query_gpu(gpu.as_ref())?;
            query::roll_balance(gpu, &context.source, field, region, input, &|| {
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
        if let Err(error) = apply_roll_balance(editor, fit) {
            editor.status = format!("Pick rejected: {error}");
        }
    });
}

fn apply_roll_balance(editor: &mut Editor, fit: query::RollBalancePick) -> Result<(), String> {
    let mut input = editor.input();
    let InputSettings::Negative { balance, .. } = &mut input else {
        return Err("Roll balance requires a negative input".into());
    };
    *balance = fit.gamma;
    let mut edit = editor
        .edit_project("Balance roll from patch")
        .map_err(|error| error.to_string())?;
    edit.update(|project| {
        project.input = input;
        project.roll.wb = fit.wb;
    });
    editor
        .commit_project(edit)
        .map_err(|error| error.to_string())?;
    editor.status = match fit.inferred_dmax {
        Some(rgb) => format!(
            "Gamma and Roll WB set · inferred Dmax: R {:.6} G {:.6} B {:.6}",
            rgb.x, rgb.y, rgb.z,
        ),
        None => "Gamma and Roll WB set".into(),
    };
    Ok(())
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

#[cfg(test)]
pub(super) fn set_gamma_balance(editor: &mut Editor, value: Balance) -> Result<(), String> {
    value.validate().map_err(|error| error.to_string())?;
    change_input(editor, |candidate| {
        let InputSettings::Negative { balance, .. } = candidate else {
            return Err("Gamma balance requires a scan input".into());
        };
        *balance = value;
        Ok(())
    })
    .map_err(|error| format!("Pick rejected: {error}"))?;
    editor.status = "Gamma span set from patch".into();
    Ok(())
}

fn change_input(
    editor: &mut Editor,
    change: impl FnOnce(&mut InputSettings) -> Result<(), String>,
) -> Result<(), String> {
    let mut draft = editor
        .edit_project("Calibration pick")
        .map_err(|error| error.to_string())?;
    let was_calibrated = editor.calibration().is_some();
    let mut candidate = editor.document.project.input;
    change(&mut candidate)?;
    draft.update(|project| project.input = candidate);
    editor
        .commit_project(draft)
        .map_err(|error| error.to_string())?;

    if !was_calibrated && editor.calibration().is_some() {
        editor.document.source = false;
    }
    Ok(())
}
