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
        (!self.busy
            || self.document.visit.ready.is_none()
                && self.document.visit.selection_timing.is_some())
            && self.fullscreen.is_none()
            && self.document.picking.kind.is_none()
            && self.document.flat_edit.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.failure.is_none()
            && self.document_pending.is_none()
            && self.transition.is_none()
            && self.project_overwrite.is_none()
            && !self.allow_close
            && !self.export_modal()
            && !self.document.visit.adjustment_layers.confirming()
    }

    pub(super) fn select_roll_photo(
        &mut self,
        index: usize,
        modifiers: egui::Modifiers,
        cx: &UiContext<'_, Self>,
    ) {
        if !self.roll_navigation_available() || self.document.removal_pending.is_some() {
            return;
        }
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
        let retry = active == id && self.document.visit.ready.is_none();
        if self.document.visit.current_source != Some(active) || retry {
            self.document.selected = self
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
                cx,
            );
        }
    }

    pub(super) fn select_active_photo_only(&mut self) {
        if !self.roll_edit_available() || self.document.removal_pending.is_some() {
            return;
        }
        if let Some(id) = self
            .document
            .visit
            .current_source
            .filter(|id| self.document.state.project.order.contains(id))
        {
            self.document.roll_selection.only(id);
        }
    }

    pub(super) fn select_active_shortcut(&mut self, context: &egui::Context) {
        if self.roll_edit_available()
            && self.document.removal_pending.is_none()
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
        if !self.roll_edit_available()
            || self.document.auto_crop_edit.is_some()
            || self.document.removal_pending.is_some()
        {
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
        if !ids.is_empty() {
            self.document.removal_pending = Some(ids);
        }
    }

    fn remove_selected_photos(&mut self, cx: &UiContext<'_, Self>) {
        if !self.roll_edit_available() {
            return;
        }
        let Some(ids) = self.document.removal_pending.take() else {
            return;
        };
        let index = self.document.selected;
        let active_removed = self
            .document
            .visit
            .current_source
            .is_some_and(|id| ids.contains(&id));
        let before = self.history_snapshot();
        for id in ids {
            self.document
                .state
                .project
                .remove_from_filmstrip(id)
                .expect("confirmed ordered source");
        }
        if active_removed {
            self.document.visit.current_source = None;
        }
        self.record_edit(before, "Remove photos", None);
        self.document.thumbnails.cancel();
        self.document.roll_selection = RollSelection::default();
        if !self.document.state.project.order.is_empty() {
            self.document.selected = index.min(self.document.state.project.order.len() - 1);
            let id = self.document.state.project.order[self.document.selected];
            self.document.roll_selection.only(id);
            self.load(
                self.document
                    .source_path(id)
                    .expect("remaining photo locator"),
                cx,
            );
        } else {
            self.invalidate_original();
            self.document.visit.working_view.clear();
            self.id = self.request_id();
            self.replace_photo_scope(self.id);
            self.source_epoch
                .store(self.id, std::sync::atomic::Ordering::Relaxed);
            self.document.visit.support = Default::default();
            self.document.thumbnails.release_selected();
            self.document.thumbnails.refresh_hidden_memory();
            self.document.visit.current_source = None;
            self.document.selected = 0;
            self.document.visit.ready = None;
            self.document.visit.pending_preview = None;
            self.document.visit.selection_timing = None;
            self.document.visit.last_layout = None;
            self.document.visit.viewport.fit();
            self.document.visit.adjustment_layer_settings = AdjustmentLayerSettings::default();
            self.document.visit.photo_orientation = PhotoOrientation::default();
            self.document.visit.crop = None;
            self.status = "No photos in this roll. Add scans to continue.".into();
        }
    }

    pub(super) fn removal_dialog(
        &mut self,
        context: &egui::Context,
        escape_available: bool,
        cx: &UiContext<'_, Self>,
    ) {
        let Some(ids) = &self.document.removal_pending else {
            return;
        };
        let title = if ids.len() == 1 {
            "Remove 1 photo from project?".into()
        } else {
            format!("Remove {} photos from project?", ids.len())
        };
        let choice = presentation::DecisionDialog::new(
            &title,
            "The photos will be removed from this roll. Their project data will be discarded. Your RAW files are not changed.",
            "Remove photos",
        )
        .warning()
        .show(
            context,
            egui::Id::new("remove-roll-photos"),
            escape_available,
        );
        match choice {
            Some(presentation::DialogChoice::Cancel) => self.document.removal_pending = None,
            Some(presentation::DialogChoice::Confirm) => self.remove_selected_photos(cx),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
