use super::*;

#[derive(Default)]
pub(super) struct RollSelection {
    ids: Vec<SourceId>,
    anchor: Option<SourceId>,
}

impl RollSelection {
    fn only(&mut self, id: SourceId) {
        self.ids = vec![id];
        self.anchor = Some(id);
    }

    pub(super) fn contains(&self, id: SourceId) -> bool {
        self.ids.contains(&id)
    }

    pub(super) fn reconcile(&mut self, order: &[SourceId], active: Option<SourceId>) {
        self.ids.retain(|id| order.contains(id));
        if let Some(active) = active.filter(|id| order.contains(id)) {
            if !self.contains(active) {
                self.only(active);
            }
        } else {
            self.ids.clear();
        }
        if self.anchor.is_some_and(|id| !order.contains(&id)) {
            self.anchor = active;
        }
    }
}

impl Editor {
    pub(super) fn select_roll_photo(
        &mut self,
        index: usize,
        modifiers: egui::Modifiers,
        cx: &UiContext<'_, Self>,
    ) {
        let Some(&id) = self.document.state.project.order.get(index) else {
            return;
        };
        let order = &self.document.state.project.order;
        self.document
            .roll_selection
            .reconcile(order, self.document.visit.current_source);
        let active = if modifiers.shift {
            let anchor = self.document.roll_selection.anchor.unwrap_or(id);
            let start = order
                .iter()
                .position(|candidate| *candidate == anchor)
                .unwrap_or(index);
            self.document.roll_selection.ids = order[start.min(index)..=start.max(index)].to_vec();
            anchor
        } else if modifiers.command {
            if self.document.roll_selection.contains(id) {
                if self.document.roll_selection.ids.len() == 1 {
                    return;
                }
                self.document
                    .roll_selection
                    .ids
                    .retain(|candidate| *candidate != id);
                if self.document.visit.current_source == Some(id) {
                    // Prefer a selected successor, then the final selected predecessor.
                    order[index..]
                        .iter()
                        .find(|id| self.document.roll_selection.contains(**id))
                        .or_else(|| {
                            order[..index]
                                .iter()
                                .rev()
                                .find(|id| self.document.roll_selection.contains(**id))
                        })
                        .copied()
                        .expect("remaining selection")
                } else {
                    self.document
                        .visit
                        .current_source
                        .expect("active selected photo")
                }
            } else {
                self.document.roll_selection.ids.push(id);
                self.document.visit.current_source.unwrap_or(id)
            }
        } else {
            if !self.document.roll_selection.contains(id) {
                self.document.roll_selection.only(id);
            }
            self.document.roll_selection.anchor = Some(id);
            id
        };
        self.browse_photo(active, cx);
    }

    pub(super) fn select_active_photo_only(&mut self) {
        if let Some(id) = self
            .document
            .visit
            .current_source
            .filter(|id| self.document.state.project.order.contains(id))
        {
            self.document.roll_selection.only(id);
        }
    }

    pub(super) fn request_removal(&mut self, cx: &UiContext<'_, Self>) {
        if self.editing() {
            return;
        }
        self.document.roll_selection.reconcile(
            &self.document.state.project.order,
            self.document.visit.current_source,
        );
        let ids: Vec<_> = self
            .document
            .project
            .order
            .iter()
            .copied()
            .filter(|id| self.document.roll_selection.contains(*id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let title = if ids.len() == 1 {
            "Remove 1 photo from project?".into()
        } else {
            format!("Remove {} photos from project?", ids.len())
        };
        let response = self.dialogs.show(move |context, id, escape| {
            ui::DecisionDialog::new(&title,
                "The photos will be removed from this roll. Their project data will be discarded. Your RAW files are not changed.",
                "Remove photos").warning().show(context, id, escape)
        });
        self.document.dialog_action = cx.spawn(move |cx| async move {
            if response.await == Ok(ui::DialogChoice::Confirm) {
                cx.update(|editor| editor.remove_selected_photos(ids, &cx));
            }
        });
    }

    fn remove_selected_photos(&mut self, ids: Vec<SourceId>, cx: &UiContext<'_, Self>) {
        if ids
            .iter()
            .any(|id| !self.document.project.order.contains(id))
        {
            self.status = "Photo selection changed; removal cancelled".into();
            return;
        }
        let mut draft = match self.edit_project("Remove photos") {
            Ok(edit) => edit,
            Err(error) => {
                self.status = error.to_string();
                return;
            }
        };
        let index = self.document.active_index().unwrap_or(0);
        draft.update(|project| {
            for id in ids {
                project
                    .remove_from_filmstrip(id)
                    .expect("confirmed ordered source");
            }
        });
        if let Err(error) = self.commit_project(draft) {
            self.status = error.to_string();
            return;
        }
        let scope = &mut self.document;
        scope
            .sampling_inputs
            .retain(&scope.state, &scope.initialization);
        self.document.roll_selection = RollSelection::default();
        if !self.document.state.project.order.is_empty() {
            let index = index.min(self.document.state.project.order.len() - 1);
            let id = self.document.state.project.order[index];
            self.document.roll_selection.only(id);
            self.browse_photo(id, cx);
        } else {
            self.activate_photo(None, cx);
        }
    }
}

#[cfg(test)]
mod tests;
