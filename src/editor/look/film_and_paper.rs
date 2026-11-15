use super::*;

#[derive(Default)]
pub(in crate::editor) struct State {
    pub pending: Option<Operation>,
    pub confirmation: Option<async_executor::Task<()>>,
}

impl Editor {
    pub(super) fn material_selection(&self) -> Option<MaterialSelection> {
        self.document
            .film_and_paper
            .pending
            .as_ref()
            .and_then(|op| op.selection)
            .or_else(|| {
                self.document
                    .state
                    .project
                    .output
                    .film_and_paper
                    .as_ref()
                    .and_then(|asset| MaterialSelection::from_origin(asset.origin()))
            })
    }
    pub(super) fn select_negative(
        &mut self,
        negative: Option<material::Negative>,
        cx: &UiContext<'_, Self>,
    ) {
        if !self.look_request_allowed() {
            return;
        }
        let current = self.material_selection();
        let paper = current
            .map(MaterialSelection::paper)
            .or_else(|| negative.map(material::Negative::default_paper));
        if let Some(selection) = paper.and_then(|paper| MaterialSelection::new(negative, paper)) {
            self.select_look(Source::Photochemical(selection), cx);
        } else if let (Some(previous), Some(negative)) = (current, negative) {
            self.request_material_switch(
                previous,
                MaterialSelection::new(Some(negative), negative.default_paper())
                    .expect("negative's default paper is supported"),
                cx,
            );
        }
    }
    #[cfg(test)]
    pub(super) fn photochemical_selection(&self) -> Option<Selection> {
        match self.material_selection() {
            Some(MaterialSelection::Colour(selection)) => Some(selection),
            _ => None,
        }
    }
    pub(super) fn select_paper(
        &mut self,
        paper: Option<material::Paper>,
        cx: &UiContext<'_, Self>,
    ) {
        if !self.look_request_allowed() {
            return;
        }
        let current = self.material_selection();
        let negative = current.and_then(MaterialSelection::negative);
        match paper {
            Some(paper) => {
                if let Some(selection) = MaterialSelection::new(negative, paper) {
                    self.select_look(Source::Photochemical(selection), cx);
                } else if let Some(previous) = current {
                    self.request_material_switch(
                        previous,
                        MaterialSelection::new(None, paper)
                            .expect("paper-only treatment is supported"),
                        cx,
                    );
                }
            }
            None if current.is_some() && negative.is_none() => self.clear_slot(Slot::FilmAndPaper),
            _ => {}
        }
    }

    fn request_material_switch(
        &mut self,
        previous: MaterialSelection,
        selection: MaterialSelection,
        cx: &UiContext<'_, Self>,
    ) {
        let explanation = if let Some(negative) = selection.negative() {
            format!(
                "{} with {} is not supported. Switch the paper to {}?",
                negative.name(),
                previous.paper().name(),
                selection.paper().name()
            )
        } else {
            format!(
                "{} with {} is not supported. Set Negative to None to use this paper?",
                previous
                    .negative()
                    .expect("switch clears a selected negative")
                    .name(),
                selection.paper().name()
            )
        };
        let response = self.dialogs.show(move |context, id, escape| {
            ui::DecisionDialog::new("Switch Film & paper?", &explanation, "Switch materials")
                .show(context, id, escape)
        });
        self.document.film_and_paper.confirmation = cx.spawn(move |cx| async move {
            if response.await == Ok(ui::DialogChoice::Confirm) {
                cx.update(|editor| editor.prepare_look(Source::Photochemical(selection), &cx));
            }
        });
    }
    pub(super) fn film_and_paper_controls(&mut self, ui: &mut egui::Ui, cx: &UiContext<'_, Self>) {
        let selection = self.material_selection();
        let negative = selection.and_then(MaterialSelection::negative);
        let paper = selection.map(MaterialSelection::paper);
        let current_definition = self
            .document
            .film_and_paper
            .pending
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
        ui.add_enabled_ui(self.lut_controls_allowed(), |ui| {
            let (_, action) = ui::section(
                ui,
                "film-and-paper",
                "Film & paper",
                ui::SectionHeader {
                    summary: &name,
                    changed,
                    reset: Some(changed),
                    tool: None,
                    default_open: Some(true),
                },
                |ui| {
                    ui::labelled_inspector_dropdown(
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
                    ui::labelled_inspector_dropdown(
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
