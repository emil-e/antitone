use super::*;

#[derive(Default)]
pub(in crate::editor) struct State {
    pub pending: Option<Operation>,
}

impl Editor {
    pub(in crate::editor) fn clear_look(&mut self) {
        self.clear_slot(Slot::Lut);
    }
    pub(in crate::editor) fn remember_imported_look(&mut self) {
        let Some(asset) = self
            .document
            .project
            .output
            .lut
            .as_ref()
            .filter(|asset| matches!(asset.origin(), LookOrigin::Imported))
        else {
            return;
        };
        if let Some(existing) = self
            .look
            .loaded
            .iter_mut()
            .find(|entry| entry.id() == asset.id())
        {
            *existing = asset.clone();
        } else {
            self.look.loaded.push(asset.clone());
        }
    }
    pub(super) fn imported_lut_controls(
        &mut self,
        ui: &mut egui::Ui,
        cx: &UiContext<'_, Self>,
        lead: ui::SectionLead,
    ) {
        let labels = loaded_lut_labels(&self.look.loaded);
        let accepted = self.document.project.output.lut.as_ref();
        let name = accepted.map_or_else(
            || "None".to_owned(),
            |asset| {
                self.look
                    .loaded
                    .iter()
                    .position(|entry| entry.id() == asset.id())
                    .map_or_else(|| asset.name().to_owned(), |i| labels[i].clone())
            },
        );
        let changed = accepted.is_some();
        let mut choice = None;
        let mut clear = false;
        let mut import = false;
        ui.add_enabled_ui(self.lut_controls_allowed(), |ui| {
            let (_, action) = ui::section_with_lead(
                ui,
                "rendering-lut",
                "LUT",
                ui::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                lead,
                |ui| {
                    ui::inspector_dropdown(ui, "creative-lut", "LUT", &name, |ui| {
                        if ui.selectable_label(!changed, "None").clicked() && changed {
                            clear = true;
                        }
                        for (asset, label) in self.look.loaded.iter().zip(&labels) {
                            let selected = self.document.lut.pending.is_none()
                                && self
                                    .document
                                    .project
                                    .output
                                    .lut
                                    .as_ref()
                                    .is_some_and(|current| current.id() == asset.id());
                            if ui
                                .push_id(asset.id(), |ui| ui.selectable_label(selected, label))
                                .inner
                                .clicked()
                                && !selected
                            {
                                choice = Some(Source::Loaded(asset.clone()));
                            }
                        }
                        ui.separator();
                        ui.small("ACEScct / AP1 (D60) input and output");
                        ui.small("Applied before Film & paper");
                        if ui.selectable_label(false, "Load LUT…").clicked() {
                            import = true;
                            ui.close();
                        }
                    });
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
                    .set_title("Load ACEScct / AP1 LUT")
                    .add_filter("3D LUT", &["cube"])
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

pub(super) fn loaded_lut_labels(assets: &[Arc<LookAsset>]) -> Vec<String> {
    let mut used: HashSet<String> = ["None".into(), "Load LUT…".into()].into_iter().collect();
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
