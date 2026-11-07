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

pub(super) enum Pending {
    Edit {
        before: Box<Project>,
        label: &'static str,
        photo: Option<SourceId>,
    },
    Reset,
    Restore {
        direction: Direction,
        cleanup: Option<bool>,
    },
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

    fn complete(&mut self, direction: Direction) {
        self.finish_group();
        let (from, to) = match direction {
            Direction::Undo => (&mut self.undo, &mut self.redo),
            Direction::Redo => (&mut self.redo, &mut self.undo),
        };
        to.push(
            from.pop()
                .expect("accepted history request retains its entry"),
        );
        self.group = None;
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
        let Some(entry) = self.document.history.entry(direction) else {
            return;
        };
        let project = match direction {
            Direction::Undo => entry.before.clone(),
            Direction::Redo => entry.after.clone(),
        };
        let selected = entry
            .photo
            .filter(|id| project.order.contains(id))
            .or(self
                .document
                .visit
                .current_source
                .filter(|id| project.order.contains(id)))
            .or_else(|| {
                project
                    .order
                    .get(
                        self.document
                            .selected
                            .min(project.order.len().saturating_sub(1)),
                    )
                    .copied()
            });
        let mut candidate = self.document.capture();
        candidate.project = project;
        self.start_document(Preparation::History(candidate, selected, direction), cx);
    }

    pub(super) fn history_prepared(&mut self, pending: Option<Pending>) {
        match pending {
            Some(Pending::Reset) => self.document.history = History::default(),
            Some(Pending::Edit {
                before,
                label,
                photo,
            }) => self.record_edit(*before, label, photo),
            Some(Pending::Restore { direction, cleanup }) => {
                let label = self
                    .document
                    .history
                    .entry(direction)
                    .expect("pending history entry")
                    .label;
                self.document.history.complete(direction);
                self.document.visit.working_view.clear();
                self.document.source = !self.document.project.input.ready();
                if let Some(show_masks) = cleanup
                    && !self.document.source
                    && self.document.visit.canvas.ready.is_some()
                {
                    self.restore_dust(show_masks);
                } else if self.dusting() {
                    self.end_tool();
                }
                self.status = format!("{} {label}", direction.verb());
            }
            None => {}
        }
    }

    pub(super) fn history_failed(&mut self, pending: Option<Pending>) {
        if let Some(Pending::Restore {
            cleanup: Some(show_masks),
            ..
        }) = pending
        {
            self.restore_dust(show_masks);
        }
    }

    pub(super) fn history_label(&self, direction: Direction) -> Option<&'static str> {
        self.document
            .history
            .entry(direction)
            .map(|entry| entry.label)
    }
}

#[cfg(test)]
mod tests;
