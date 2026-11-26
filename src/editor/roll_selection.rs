use super::*;

pub(super) fn take_navigation_step(context: &egui::Context) -> Option<isize> {
    let step = context.input_mut(|input| {
        let previous = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
        let next = input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
        if previous {
            Some(-1)
        } else if next {
            Some(1)
        } else {
            None
        }
    });
    if step.is_some() {
        // egui queued spatial focus navigation before these presses were consumed.
        context.memory_mut(|memory| memory.move_focus(egui::FocusDirection::None));
    }
    step
}

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
        let order = self.document.displayed_order().into_owned();
        let Some(&id) = order.get(index) else {
            return;
        };
        self.document
            .roll_selection
            .reconcile(&order, self.document.visit.current_source);
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

    pub(super) fn browse_adjacent_roll_photo(&mut self, step: isize, cx: &UiContext<'_, Self>) {
        let order = self.document.displayed_order();
        let active = order
            .iter()
            .position(|id| Some(*id) == self.document.visit.current_source)
            .unwrap_or(0);
        if let Some(index) = active
            .checked_add_signed(step)
            .filter(|&index| index < order.len())
        {
            self.select_roll_photo(index, egui::Modifiers::NONE, cx);
        }
    }

    pub(super) fn select_active_shortcut(&mut self, context: &egui::Context) {
        if context.input_mut(|input| {
            input.consume_key(
                egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                egui::Key::A,
            )
        }) {
            self.select_active_photo_only();
            context.request_repaint();
        }
    }

    pub(super) fn roll_navigation_shortcut(
        &mut self,
        context: &egui::Context,
        cx: &UiContext<'_, Self>,
    ) {
        if self.workspace_input.shortcuts_allowed(context)
            && let Some(step) = take_navigation_step(context)
        {
            self.browse_adjacent_roll_photo(step, cx);
        }
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
                "Remove photos").destructive().show(context, id, escape)
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
        let index = self
            .document
            .displayed_order()
            .iter()
            .position(|id| Some(*id) == self.document.visit.current_source)
            .unwrap_or(0);
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
        let order = self.document.displayed_order();
        if let Some(&id) = order.get(index.min(order.len().saturating_sub(1))) {
            self.document.roll_selection.only(id);
            self.browse_photo(id, cx);
        } else {
            self.activate_photo(None, cx);
        }
    }
}

#[cfg(test)]
mod tests;
