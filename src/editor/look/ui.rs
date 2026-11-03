use super::*;
use crate::looks::LookAsset;
use std::collections::HashSet;

fn loaded_lut_labels(assets: &[Arc<LookAsset>]) -> Vec<String> {
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

impl Editor {
    pub(in crate::editor) fn material_switch_dialog(
        &mut self,
        context: &egui::Context,
        escape_available: bool,
        cx: &UiContext<'_, Self>,
    ) {
        let Some(proposal) = &self.document.material_switch else {
            return;
        };
        let explanation = if let Some(negative) = proposal.selection.negative() {
            format!(
                "{} with {} is not supported. Switch the paper to {}?",
                negative.name(),
                proposal.previous.paper().name(),
                proposal.selection.paper().name()
            )
        } else {
            format!(
                "{} with {} is not supported. Set Negative to None to use this paper?",
                proposal
                    .previous
                    .negative()
                    .expect("switch clears a selected negative")
                    .name(),
                proposal.selection.paper().name()
            )
        };
        let choice = presentation::DecisionDialog::new(
            "Switch Film & paper?",
            &explanation,
            "Switch materials",
        )
        .show(
            context,
            egui::Id::new("switch-film-and-paper"),
            escape_available,
        );
        match choice {
            Some(presentation::DialogChoice::Confirm) => self.resolve_material_switch(true, cx),
            Some(presentation::DialogChoice::Cancel) => self.resolve_material_switch(false, cx),
            _ => {}
        }
    }
    pub(in crate::editor) fn look_rendering_error(&self) -> Option<Error> {
        if self.document.source || self.picking() || self.document.analysis_review().is_some() {
            return None;
        }
        let id = self.document.state.project.output.luts().id()?;
        if let Some(view) = self
            .document
            .visit
            .working_view
            .displayed
            .as_ref()
            .filter(|view| view.look_id() == Some(id))
        {
            return view.image.look_failure(id);
        }
        self.selected_view()?
            .1
            .image(self.document.target)
            .ok()?
            .look_failure(id)
    }

    fn lut_controls_allowed(&self, slot: Slot) -> bool {
        self.look_request_allowed(slot)
            && self.document.analysis_review().is_none()
            && !self.masking()
    }

    pub(in crate::editor) fn look_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        self.imported_lut_controls(ui, cx);
        self.film_and_paper_controls(ui, cx);
    }

    fn imported_lut_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let labels = loaded_lut_labels(&self.look.loaded);
        let accepted = self.document.project.output.lut.as_ref();
        let name = accepted.map_or_else(
            || "None".to_owned(),
            |asset| {
                self.look
                    .loaded
                    .iter()
                    .position(|entry| entry.lut().id() == asset.lut().id())
                    .map_or_else(|| asset.name().to_owned(), |i| labels[i].clone())
            },
        );
        let changed = accepted.is_some();
        let mut choice = None;
        let mut clear = false;
        let mut import = false;
        ui.add_enabled_ui(self.lut_controls_allowed(Slot::Lut), |ui| {
            let (_, action) = presentation::section(
                ui,
                "rendering-lut",
                "LUT",
                presentation::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    presentation::inspector_dropdown(ui, "creative-lut", "LUT", &name, |ui| {
                        if ui.selectable_label(!changed, "None").clicked() && changed {
                            clear = true;
                        }
                        for (asset, label) in self.look.loaded.iter().zip(&labels) {
                            let selected =
                                self.document.look.is_none()
                                    && self.document.project.output.lut.as_ref().is_some_and(
                                        |current| current.lut().id() == asset.lut().id(),
                                    );
                            if ui
                                .push_id(asset.lut().id(), |ui| {
                                    ui.selectable_label(selected, label)
                                })
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

    fn film_and_paper_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let selection = self.material_selection();
        let negative = selection.and_then(MaterialSelection::negative);
        let paper = selection.map(MaterialSelection::paper);
        let current_definition = self
            .document
            .look
            .as_ref()
            .is_some_and(|op| op.selection.is_some())
            || selection.is_some_and(|s| {
                self.document
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .is_some_and(|asset| asset.origin() == &s.origin())
            });
        let name = self
            .document
            .project
            .output
            .film_and_paper
            .as_ref()
            .map_or("None", |asset| asset.name())
            .to_owned();
        let changed = self.document.project.output.film_and_paper.is_some();
        let mut negative_choice = None;
        let mut paper_choice = None;
        let mut clear = false;
        ui.add_enabled_ui(self.lut_controls_allowed(Slot::FilmAndPaper), |ui| {
            let (_, action) = presentation::section(
                ui,
                "film-and-paper",
                "Film & paper",
                presentation::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    presentation::labelled_inspector_dropdown(
                        ui,
                        "negative-stock",
                        "Negative",
                        negative.map_or("None", |stock| stock.name()),
                        |ui| {
                            if ui.selectable_label(negative.is_none(), "None").clicked()
                                && negative.is_some()
                            {
                                negative_choice = Some(None);
                            }
                            for stock in material::Negative::all() {
                                let selected = negative == Some(stock) && current_definition;
                                let response = ui.selectable_label(selected, stock.name());
                                let response = if paper.is_some_and(|p| !p.supports_negative(stock))
                                {
                                    response.on_hover_text(
                                        "Select this negative to review the required paper change.",
                                    )
                                } else {
                                    response
                                };
                                if response.clicked() && !selected {
                                    negative_choice = Some(Some(stock));
                                    ui.close();
                                }
                            }
                        },
                    );
                    presentation::labelled_inspector_dropdown(
                        ui,
                        "print-paper",
                        "Paper",
                        paper.map_or("None", |medium| medium.name()),
                        |ui| {
                            let none = ui
                                .add_enabled_ui(negative.is_none(), |ui| {
                                    ui.selectable_label(paper.is_none(), "None")
                                })
                                .inner;
                            if none
                                .on_disabled_hover_text("Choose no negative to bypass the paper.")
                                .clicked()
                                && paper.is_some()
                            {
                                paper_choice = Some(None);
                            }
                            for medium in material::Paper::all() {
                                let selected = paper == Some(medium) && current_definition;
                                let response = ui.selectable_label(selected, medium.name());
                                let response =
                                    if negative.is_some_and(|n| !medium.supports_negative(n)) {
                                        response.on_hover_text(
                                            "Select this paper to review setting Negative to None.",
                                        )
                                    } else {
                                        response
                                    };
                                if response.clicked() && !selected {
                                    paper_choice = Some(Some(medium));
                                    ui.close();
                                }
                            }
                        },
                    );
                },
            );
            clear = action.reset;
        });
        if clear {
            self.clear_film_and_paper();
        } else if let Some(negative) = negative_choice {
            self.select_negative(negative, cx);
        } else if let Some(paper) = paper_choice {
            self.select_paper(paper, cx);
        }
    }
}

#[cfg(test)]
mod tests;
