use super::editing::ProjectVersion as Project;
use super::*;

const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Undo,
    Redo,
}

impl Direction {
    fn verb(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
        }
    }
}

struct Entry {
    before: Project,
    after: Project,
    label: &'static str,
    photo: Option<SourceId>,
}

#[derive(Default)]
pub(super) struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    group: Option<egui::Id>,
    // A gesture with no net change must preserve the redo branch it started from.
    group_redo: Option<Vec<Entry>>,
}

struct Restored {
    before: Project,
    label: &'static str,
    photo: Option<SourceId>,
}

impl History {
    fn entry(&self, direction: Direction) -> Option<&Entry> {
        match direction {
            Direction::Undo => self.undo.last(),
            Direction::Redo => self.redo.last(),
        }
    }

    fn record(
        &mut self,
        before: Project,
        after: Project,
        label: &'static str,
        photo: Option<SourceId>,
        group: Option<egui::Id>,
    ) {
        if before == after {
            return;
        }
        if self.group != group || group.is_none() {
            self.finish_group();
            if group.is_some() {
                self.group_redo = Some(std::mem::take(&mut self.redo));
            }
        }
        self.redo.clear();
        if group.is_some()
            && self.group == group
            && let Some(entry) = self.undo.last_mut()
            && entry.after == before
            && entry.photo == photo
        {
            entry.after = after;
        } else {
            self.undo.push(Entry {
                before,
                after,
                label,
                photo,
            });
            if group.is_none() && self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
        self.group = group;
    }

    pub(super) fn finish_group(&mut self) {
        if self.group.take().is_some() {
            if self
                .undo
                .last()
                .is_some_and(|entry| entry.before == entry.after)
            {
                self.undo.pop();
                self.redo = self.group_redo.take().unwrap_or_default();
            }
            self.group_redo = None;
            if self.undo.len() > HISTORY_LIMIT {
                self.undo.remove(0);
            }
        }
    }

    fn restore(&mut self, accepted: &mut Project, direction: Direction) -> Option<Restored> {
        self.finish_group();
        let (from, to) = match direction {
            Direction::Undo => (&mut self.undo, &mut self.redo),
            Direction::Redo => (&mut self.redo, &mut self.undo),
        };
        let entry = from.pop()?;
        let target = match direction {
            Direction::Undo => entry.before.clone(),
            Direction::Redo => entry.after.clone(),
        };
        let restored = Restored {
            before: std::mem::replace(accepted, target),
            label: entry.label,
            photo: entry.photo,
        };
        to.push(entry);
        Some(restored)
    }

    pub(super) fn rebase(&mut self, old: Option<&std::path::Path>, new: Option<&std::path::Path>) {
        for entry in self
            .undo
            .iter_mut()
            .chain(&mut self.redo)
            .chain(self.group_redo.iter_mut().flatten())
        {
            document::rebase_project(&mut entry.before, old, new);
            document::rebase_project(&mut entry.after, old, new);
        }
    }
}

impl Editor {
    #[cfg(test)]
    pub(super) fn history_snapshot(&mut self) -> Project {
        self.document.state.project.clone()
    }

    pub(super) fn record_edit(
        &mut self,
        before: Project,
        label: &'static str,
        photo: Option<SourceId>,
    ) {
        if before == self.document.state.project {
            return;
        }
        let after = self.document.state.project.clone();
        self.document
            .history
            .record(before, after, label, photo, None);
    }

    pub(super) fn history_available(&self) -> bool {
        !self.interaction_blocked()
            && !self.editing()
            && !self.controls_pending()
            && !self.dialogs.is_open()
            && self.menus.panel.is_none()
            && self.document.look.is_none()
            && self.document_pending.is_none()
            && !self.export_modal()
            && !self.files.pending()
            && !self.allow_close
            && !self.document.visit.adjustment_layers.dragging()
            && !self.dust_history_draft()
    }

    pub(super) fn restore_history(&mut self, direction: Direction, cx: &UiContext<'_, Self>) {
        if !self.history_available() {
            return;
        }
        self.document.history.finish_group();
        if self.document.history.entry(direction).is_none() {
            return;
        }
        let cleanup = self.dusting().then(|| self.dust_outlines());
        self.document.visit.interaction_action = None;
        self.cancel_active_tool();
        self.document.visit.masks.cancel();

        let Restored {
            before,
            label,
            photo,
        } = {
            let document = &mut self.document;
            document
                .history
                .restore(&mut document.state.project, direction)
                .expect("history entry selected in this transaction")
        };
        self.document
            .sampling_inputs
            .retain(&self.document.state, &self.document.initialization);
        let selected = photo
            .filter(|id| self.document.project.order.contains(id))
            .or(self
                .document
                .visit
                .current_source
                .filter(|id| self.document.project.order.contains(id)))
            .or_else(|| {
                self.document
                    .project
                    .order
                    .get(
                        self.document
                            .selected
                            .min(self.document.project.order.len().saturating_sub(1)),
                    )
                    .copied()
            });
        let next = selected.and_then(|id| self.document.source_input(id).ok());
        let stage_changed = before.input.is_negative() != self.document.project.input.is_negative();
        let same_visit = !stage_changed
            && self.document.visit.current_source == selected
            && match (&next, &self.document.visit.canvas.ready) {
                (Some(input), Some(ready)) => {
                    input.matches(&ready.path, &ready.file, &ready.record)
                }
                (Some(input), None) => self
                    .document
                    .visit
                    .source
                    .input
                    .as_ref()
                    .is_some_and(|current| current.same_request(input)),
                (None, _) => true,
            };
        let correction_changed =
            before.active_flat_field() != self.document.project.active_flat_field();
        if !same_visit {
            self.id = self.request_id();
            self.source_epoch
                .store(self.id, std::sync::atomic::Ordering::Relaxed);
            if self.document.visit.current_source == selected {
                self.reinterpret_photo(self.id);
            } else {
                self.replace_photo_scope(self.id);
            }
        }
        if stage_changed {
            self.cancel_spatial();
            self.document.thumbnails.cancel();
        }
        self.document.visit.current_source = selected;
        self.document
            .roll_selection
            .reconcile(&self.document.state.project.order, selected);
        self.document.selected = selected
            .and_then(|id| {
                self.document
                    .project
                    .order
                    .iter()
                    .position(|candidate| *candidate == id)
            })
            .unwrap_or(0);
        self.refresh_adjustment_layer_selection();
        self.remember_imported_look();
        self.invalidate_original();
        self.document.source = !self.document.project.input.ready();
        if correction_changed {
            self.document.correction_restore = None;
            self.document.correction = Default::default();
            self.restore_correction(cx);
        }
        if let Some(show_masks) = cleanup
            && !self.document.source
            && self.document.visit.canvas.ready.is_some()
        {
            self.restore_dust(show_masks);
        }
        self.observe_gamma_pivot();
        if !same_visit && let Some(input) = next {
            self.start_source_acquisition(input.path().to_owned(), cx);
        }
        self.acquire_working(cx);
        self.status = format!("{} {label}", direction.verb());
    }

    pub(super) fn history_label(&self, direction: Direction) -> Option<&'static str> {
        self.document
            .history
            .entry(direction)
            .map(|entry| entry.label)
    }
}

#[cfg(test)]
mod acquisition_tests;
#[cfg(test)]
mod tests;
