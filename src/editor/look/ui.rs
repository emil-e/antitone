use super::*;
use crate::looks::{LookAsset, LookOrigin, builtin::Builtin};
use std::collections::HashSet;

fn builtin_attribution(builtin: Builtin) -> String {
    let source = builtin.source_product();
    format!(
        "Independent creative interpretation derived from published {source} data.\nNot affiliated with or endorsed by the manufacturer."
    )
}
fn builtin_for_asset(asset: &LookAsset) -> Option<Builtin> {
    let LookOrigin::Builtin { id, .. } = asset.origin() else {
        return None;
    };
    Builtin::ALL.into_iter().find(|builtin| builtin.id() == id)
}
fn asset_label(asset: &LookAsset) -> &str {
    match builtin_for_asset(asset) {
        Some(builtin) => builtin.name(),
        None => asset.name(),
    }
}

fn loaded_lut_labels(assets: &[Arc<LookAsset>]) -> Vec<String> {
    let mut used: HashSet<String> = Builtin::ALL
        .into_iter()
        .map(|builtin| builtin.name().to_owned())
        .chain(["None".into(), "Load LUT…".into()])
        .collect();
    assets
        .iter()
        .map(|asset| {
            let mut name = asset.name().to_owned();
            let mut suffix = 2;
            while !used.insert(name.clone()) {
                name = format!("{} ({suffix})", asset.name());
                suffix += 1;
            }
            name
        })
        .collect()
}

impl Editor {
    pub(in crate::editor) fn look_rendering_error(&self) -> Option<Error> {
        if self.document.source
            || self.active_pick().is_some()
            || self.document.flat_edit().is_some()
            || self.document.analysis_review().is_some()
        {
            return None;
        }
        let id = self.document.state.project.output.look.as_ref()?.lut().id();
        if let Some(view) = self
            .document
            .visit
            .working_view
            .displayed
            .as_ref()
            .filter(|view| view.processing.look_id() == Some(id))
        {
            return view.image.look_failure(id);
        }
        self.selected_view()?
            .1
            .image(self.document.target)
            .ok()?
            .look_failure(id)
    }

    pub(in crate::editor) fn look_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let allowed = (!self.busy || self.document.look.is_some())
            && self.document.crop_edit().is_none()
            && self.document.auto_crop_edit().is_none()
            && self.document.flat_edit().is_none()
            && self.document.analysis_review().is_none()
            && self.document.mask_edit().is_none();
        let loaded_labels = loaded_lut_labels(&self.look.loaded);
        let name = self
            .document
            .state
            .project
            .output
            .look
            .as_ref()
            .map_or_else(
                || "None".into(),
                |asset| {
                    if matches!(asset.origin(), LookOrigin::Imported)
                        && let Some(index) = self
                            .look
                            .loaded
                            .iter()
                            .position(|entry| entry.lut().id() == asset.lut().id())
                    {
                        loaded_labels[index].clone()
                    } else {
                        asset_label(asset).to_owned()
                    }
                },
            );
        let source = self
            .document
            .project
            .output
            .look
            .as_deref()
            .and_then(builtin_for_asset);
        let changed = self.document.state.project.output.look.is_some();
        let mut choice = None;
        let mut clear = false;
        let mut import = false;
        ui.add_enabled_ui(allowed, |ui| {
            let (_, action) = presentation::section(
                ui,
                "rendering-look",
                "Look",
                presentation::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    let response = presentation::inspector_dropdown(
                        ui,
                        "creative-look",
                        "Look",
                        &name,
                        |ui| {
                            if ui.selectable_label(!changed, "None").clicked() {
                                clear = true;
                            }
                            for builtin in Builtin::ALL {
                                let selected = self
                                    .document
                                    .state
                                    .project
                                    .output
                                    .look
                                    .as_ref()
                                    .is_some_and(|asset| asset.origin() == &builtin.origin());
                                if ui
                                    .selectable_label(selected, builtin.name())
                                    .on_hover_text(builtin_attribution(builtin))
                                    .clicked()
                                    && !selected
                                {
                                    choice = Some(Source::Builtin(builtin));
                                }
                            }
                            if !self.look.loaded.is_empty() {
                                ui.separator();
                                ui.small("Loaded LUTs");
                                for (asset, name) in self.look.loaded.iter().zip(&loaded_labels) {
                                    let selected = self
                                        .document
                                        .state
                                        .project
                                        .output
                                        .look
                                        .as_ref()
                                        .is_some_and(|current| {
                                            matches!(current.origin(), LookOrigin::Imported)
                                                && current.lut().id() == asset.lut().id()
                                        });
                                    if ui
                                        .push_id(asset.lut().id(), |ui| {
                                            ui.selectable_label(selected, name)
                                        })
                                        .inner
                                        .clicked()
                                        && !selected
                                    {
                                        choice = Some(Source::Loaded(asset.clone()));
                                    }
                                }
                            }
                            ui.separator();
                            ui.small("ACEScct / AP1 (D60) input and output");
                            ui.small("No output-rendering or colour-conversion LUTs");
                            if ui.selectable_label(false, "Load LUT…").clicked() {
                                import = true;
                                ui.close();
                            }
                        },
                    );
                    if let Some(builtin) = source {
                        response
                            .response
                            .on_hover_text(builtin_attribution(builtin));
                    }
                },
            );
            clear |= action.reset;
        });
        if clear {
            self.clear_look();
        }
        if import
            && let Some(path) = crate::platform::menu::with_native_dialog(|| {
                rfd::FileDialog::new()
                    .set_title("Load scene-referred ACEScct / AP1 LUT")
                    .add_filter("3D look LUT", &["cube"])
                    .pick_file()
            })
        {
            choice = Some(Source::Imported(path));
        }
        if let Some(source) = choice {
            self.select_look(source, cx);
        }
    }
}

#[cfg(test)]
mod tests;
