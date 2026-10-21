use super::*;
use crate::project::Project;

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

#[derive(Clone, Copy, PartialEq)]
struct ControlState {
    roll: RollSettings,
    photo: PhotoSettings,
    target: OutputTarget,
    source: Option<SourceId>,
}

impl ControlState {
    fn capture(editor: &Editor) -> Self {
        Self {
            roll: editor.roll,
            photo: editor.photo,
            target: editor.target,
            source: editor.current_source,
        }
    }

    fn restore(self, project: &mut Project) {
        project.roll = self.roll;
        project.processing_model = self.roll.response.processing_model();
        project.output.tiff = match self.target {
            OutputTarget::Srgb => crate::project::TiffTarget::Srgb,
            OutputTarget::DisplayP3 => crate::project::TiffTarget::DisplayP3,
        };
        if let Some(source) = self.source {
            project
                .source_mut(source)
                .expect("control source belongs to document")
                .photo = self.photo;
        }
    }
}

pub(super) struct Controls {
    state: ControlState,
    before: Option<Project>,
    label: &'static str,
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
    fn clone_history_project(&mut self) -> Project {
        #[cfg(test)]
        {
            self.history_snapshot_clones += 1;
        }
        self.document.project.clone()
    }

    pub(super) fn history_snapshot(&mut self) -> Project {
        self.sync_document();
        self.clone_history_project()
    }

    pub(super) fn begin_control_history(&mut self, label: &'static str) {
        self.sync_document();
        self.history_controls = Some(Controls {
            state: ControlState::capture(self),
            before: None,
            label,
        });
    }

    pub(super) fn record_edit(
        &mut self,
        before: Project,
        label: &'static str,
        photo: Option<SourceId>,
    ) {
        self.sync_document();
        if before == self.document.project {
            return;
        }
        if let Some(controls) = &mut self.history_controls {
            controls.label = label;
            // Local controls replace accepted records during the pass. Keep their
            // first before-project; scalar mirrors can be compared without cloning.
            controls.before.get_or_insert(before);
            return;
        }
        let after = self.clone_history_project();
        self.document
            .history
            .record(before, after, label, photo, None);
    }

    pub(super) fn history_available(&self) -> bool {
        !self.busy
            && self.failure.is_none()
            && self.menus.panel.is_none()
            && self.document.look.is_none()
            && self.document_pending.is_none()
            && self.pick.is_none()
            && self.flat_edit.is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.auto_crop_edit.is_none()
            && self.document.visit.mask_edit.is_none()
            && self.document.analysis_review.is_none()
            && self.document.analysis_preparing.is_none()
            && self.document.analysis_pending.is_none()
            && self.overwrite.is_none()
            && self.transition.is_none()
            && self.after_save.is_none()
            && !self.allow_close
            && self.project_overwrite.is_none()
            && self.removal_pending.is_none()
            && !self.document.visit.adjustments.confirming()
            && !self.document.visit.adjustments.dragging()
            && !self.document.dust.history_draft()
    }

    pub(super) fn restore_history(&mut self, direction: Direction, cx: &UiContext<'_, Self>) {
        if !self.history_available() {
            return;
        }
        self.sync_document();
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
            .or(self.current_source.filter(|id| project.order.contains(id)))
            .or_else(|| {
                project
                    .order
                    .get(self.selected.min(project.order.len().saturating_sub(1)))
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
                self.working.clear();
                self.source = self.calibration().is_none();
                self.document.dust.cleanup =
                    cleanup.is_some() && !self.source && self.document.visit.ready.is_some();
                if let Some(show_masks) = cleanup {
                    self.document.dust.show_masks = show_masks;
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
            self.document.dust.cleanup = true;
            self.document.dust.show_masks = show_masks;
        }
    }

    pub(super) fn history_label(&self, direction: Direction) -> Option<&'static str> {
        self.document
            .history
            .entry(direction)
            .map(|entry| entry.label)
    }

    pub(super) fn record_controls(&mut self, photo: Option<SourceId>, ui: &egui::Ui) {
        let controls = self.history_controls.take().expect("control transaction");
        let changed = controls.before.is_some() || controls.state != ControlState::capture(self);
        let group = ui.ctx().data_mut(|data| {
            let id = egui::Id::new("project-control-gesture");
            let group = data.get_temp::<egui::Id>(id);
            data.remove::<egui::Id>(id);
            group
        });
        if changed {
            let mut before = controls
                .before
                .unwrap_or_else(|| self.clone_history_project());
            controls.state.restore(&mut before);
            self.sync_document();
            if before != self.document.project {
                let after = self.clone_history_project();
                self.document
                    .history
                    .record(before, after, controls.label, photo, group);
            }
        }
        if !ui.input(|input| input.pointer.any_down()) {
            self.document.history.finish_group();
        }
    }
}

#[cfg(test)]
#[path = "editor_history_tests.rs"]
mod tests;
