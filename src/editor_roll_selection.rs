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
    pub(super) fn roll_edit_available(&self) -> bool {
        !self.busy && self.roll_navigation_available()
    }

    fn roll_navigation_available(&self) -> bool {
        (!self.busy || self.ready.is_none() && self.selection_timing.is_some())
            && self.fullscreen.is_none()
            && self.pick.is_none()
            && self.flat_edit.is_none()
            && self.crop_edit.is_none()
            && self.failure.is_none()
            && self.document_pending.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && self.resume_transition.is_none()
            && self.overwrite.is_none()
    }

    pub(super) fn select_roll_photo(&mut self, index: usize, modifiers: egui::Modifiers) {
        if !self.roll_navigation_available() || self.removal_pending.is_some() {
            return;
        }
        let Some(&id) = self.document.project.order.get(index) else {
            return;
        };
        let order = &self.document.project.order;
        self.roll_selection.reconcile(order, self.current_source);
        let active = if modifiers.shift {
            let anchor = self.roll_selection.anchor.unwrap_or(id);
            let start = order
                .iter()
                .position(|candidate| *candidate == anchor)
                .unwrap_or(index);
            self.roll_selection.ids = order[start.min(index)..=start.max(index)].to_vec();
            anchor
        } else if modifiers.command {
            if self.roll_selection.contains(id) {
                if self.roll_selection.ids.len() == 1 {
                    return;
                }
                self.roll_selection.ids.retain(|candidate| *candidate != id);
                if self.current_source == Some(id) {
                    // Prefer a selected successor, then the final selected predecessor.
                    order[index..]
                        .iter()
                        .find(|id| self.roll_selection.contains(**id))
                        .or_else(|| {
                            order[..index]
                                .iter()
                                .rev()
                                .find(|id| self.roll_selection.contains(**id))
                        })
                        .copied()
                        .expect("remaining selection")
                } else {
                    self.current_source.expect("active selected photo")
                }
            } else {
                self.roll_selection.ids.push(id);
                self.current_source.unwrap_or(id)
            }
        } else {
            if !self.roll_selection.contains(id) {
                self.roll_selection.only(id);
            }
            self.roll_selection.anchor = Some(id);
            id
        };
        let retry = active == id && self.ready.is_none();
        if self.current_source != Some(active) || retry {
            self.selected = self
                .document
                .project
                .order
                .iter()
                .position(|id| *id == active)
                .expect("ordered active photo");
            self.load(
                self.document
                    .source_path(active)
                    .expect("active photo locator"),
            );
        }
    }

    pub(super) fn select_active_photo_only(&mut self) {
        if !self.roll_edit_available() || self.removal_pending.is_some() {
            return;
        }
        if let Some(id) = self
            .current_source
            .filter(|id| self.document.project.order.contains(id))
        {
            self.roll_selection.only(id);
        }
    }

    pub(super) fn select_active_shortcut(&mut self, context: &egui::Context) {
        if self.roll_edit_available()
            && self.removal_pending.is_none()
            && !egui::Popup::is_any_open(context)
            && context.input_mut(|input| {
                input.consume_key(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::A,
                )
            })
        {
            self.select_active_photo_only();
            context.request_repaint();
        }
    }

    pub(super) fn request_removal(&mut self) {
        if !self.roll_edit_available() || self.removal_pending.is_some() {
            return;
        }
        self.roll_selection
            .reconcile(&self.document.project.order, self.current_source);
        let ids: Vec<_> = self
            .document
            .project
            .order
            .iter()
            .copied()
            .filter(|id| self.roll_selection.contains(*id))
            .collect();
        if !ids.is_empty() {
            self.removal_pending = Some(ids);
        }
    }

    fn remove_selected_photos(&mut self) {
        if !self.roll_edit_available() {
            return;
        }
        let Some(ids) = self.removal_pending.take() else {
            return;
        };
        let index = self.selected;
        self.sync_document();
        for id in ids {
            self.document
                .project
                .remove_from_filmstrip(id)
                .expect("confirmed ordered source");
        }
        self.thumbnails.cancel();
        self.roll_selection = RollSelection::default();
        if !self.document.project.order.is_empty() {
            self.selected = index.min(self.document.project.order.len() - 1);
            let id = self.document.project.order[self.selected];
            self.roll_selection.only(id);
            self.load(
                self.document
                    .source_path(id)
                    .expect("remaining photo locator"),
            );
        } else {
            self.original.invalidate();
            self.working.clear();
            self.id = self.request_id();
            self.source_epoch
                .store(self.id, std::sync::atomic::Ordering::Relaxed);
            self.analysis_area_pending = None;
            self.thumbnails.release_selected();
            self.thumbnails.refresh_hidden_memory();
            self.current_source = None;
            self.selected = 0;
            self.ready = None;
            self.pending_preview = None;
            self.selection_timing = None;
            self.last_layout = None;
            self.viewport.fit();
            self.photo = PhotoSettings::default();
            self.photo_orientation = PhotoOrientation::default();
            self.crop = None;
            self.status = "No photos in this roll. Add RAWs to continue.".into();
            if self.commands.send(Command::Unload).is_err() {
                self.status = "RAW worker stopped".into();
            }
        }
    }

    pub(super) fn removal_dialog(&mut self, context: &egui::Context, escape_available: bool) {
        let Some(ids) = &self.removal_pending else {
            return;
        };
        let title = if ids.len() == 1 {
            "Remove 1 photo from project?".into()
        } else {
            format!("Remove {} photos from project?", ids.len())
        };
        let choice = presentation::DecisionDialog::new(
            &title,
            "The photos will be removed from this roll. Your RAW files are not changed. This cannot be undone.",
            "Remove photos",
        ).warning().show(context, egui::Id::new("remove-roll-photos"), escape_available);
        match choice {
            Some(presentation::DialogChoice::Cancel) => self.removal_pending = None,
            Some(presentation::DialogChoice::Confirm) => self.remove_selected_photos(),
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "editor_roll_selection_tests.rs"]
mod tests;
