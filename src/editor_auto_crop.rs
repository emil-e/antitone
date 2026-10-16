use super::*;
use crate::{auto_crop as framing, project::SourceDevelopment, scan_regions::PictureGeometry};
#[path = "editor_auto_crop_overlay.rs"]
mod overlay;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Target {
    Photo,
    Roll,
    Selected,
}

struct Entry {
    source: SourceId,
    proposal: Crop,
    before: Option<Crop>,
    draft: Option<Crop>,
    file: thumbnails::FileKey,
    development: SourceDevelopment,
    geometry: PictureGeometry,
    constraints: Vec<[f64; 3]>,
}

pub(super) struct Draft {
    document: std::sync::Weak<crate::source_store::SourceStore>,
    development: Option<Development>,
    order: Vec<SourceId>,
    target: Target,
    attempted: usize,
    summary: String,
    entries: Vec<Entry>,
    problems: Vec<(SourceId, String)>,
    origin: Option<SourceId>,
    viewport: Viewport,
    pub(super) source_view: bool,
    pub(super) reveal_controls: bool,
    pub(super) show_outline: bool,
    drag: Option<overlay::MarginDrag>,
}
impl Draft {
    fn belongs_to(&self, editor: &Editor) -> bool {
        self.document
            .upgrade()
            .is_some_and(|s| !s.is_closed() && Arc::ptr_eq(&s, editor.document.cache()))
            && self.development == editor.document.project.development
            && self.order == editor.document.project.order
    }
}
impl Entry {
    fn current(&self, editor: &Editor) -> bool {
        let Ok(source) = editor.document.project.source(self.source) else {
            return false;
        };
        source.crop == self.before
            && (editor.current_source != Some(self.source) || editor.crop == self.before)
            && source.development == self.development
            && editor
                .document
                .project
                .analysis_regions
                .iter()
                .find(|r| r.source == self.source)
                .is_some_and(|r| {
                    !r.areas.picture_edited
                        && r.areas.picture_observed == [true; 4]
                        && r.areas.picture_weak == [false; 4]
                        && r.areas.picture_geometry.as_ref() == Some(&self.geometry)
                        && r.areas.picture_constraints == self.constraints
                })
    }
}
fn accepted(crop: Crop, active: Rect) -> Option<Crop> {
    (crop != Crop::from_region(active)).then_some(crop)
}
fn reason(error: Error) -> String {
    match error {
        Error::UnusableSample(s) => s.to_owned(),
        error => error.to_string(),
    }
}

impl Editor {
    pub(super) fn auto_crop_entry_available(&self) -> bool {
        self.auto_crop_edit.is_none()
            && !self.busy
            && self.ready.is_some()
            && self.roll_edit_available()
            && !self.dust.cleanup
            && self.analysis_review.is_none()
            && self.removal_pending.is_none()
    }
    pub(super) fn auto_crop_valid(&self) -> bool {
        self.auto_crop_edit.as_ref().is_some_and(|d| {
            d.belongs_to(self)
                && !d.entries.is_empty()
                && d.entries
                    .iter()
                    .all(|e| e.current(self) && !d.problems.iter().any(|(id, _)| *id == e.source))
        })
    }
    pub(super) fn auto_crop_editable(&self) -> bool {
        !self.busy
            && self.fullscreen.is_none()
            && self.pick.is_none()
            && self.crop_edit.is_none()
            && self.flat_edit.is_none()
            && !self.dust.cleanup
            && self.auto_crop_valid()
    }
    pub(super) fn auto_crop_action(&mut self, ui: &mut egui::Ui, target: Target, label: &str) {
        let available = if target == Target::Photo {
            !self.busy && self.crop_edit.is_some() && self.ready.is_some()
        } else {
            self.auto_crop_entry_available()
        };
        if presentation::TextAction::new(label)
            .enabled(available)
            .help("Use frame measurements retained by Analyze.")
            .show(ui)
            .clicked()
        {
            self.auto_crop_requested = Some(target);
        }
    }
    pub(super) fn auto_crop_controls(&mut self, ui: &mut egui::Ui) {
        let Some(edit) = &mut self.auto_crop_edit else {
            return;
        };
        if std::mem::take(&mut edit.reveal_controls) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        let edit = self.auto_crop_edit.as_ref().unwrap();
        let scope = match edit.target {
            Target::Photo => unreachable!("photo proposals belong to manual Crop"),
            Target::Roll => format!(
                "Whole roll · {}/{} proposals",
                edit.entries.len(),
                edit.attempted
            ),
            Target::Selected => format!(
                "Entry selection · {}/{} proposals",
                edit.entries.len(),
                edit.attempted
            ),
        };
        let mut show = edit.show_outline;
        let has_proposals = !edit.entries.is_empty();
        let mut margin = self.auto_crop_margin;
        let editable = self.auto_crop_editable();
        presentation::ToolIdentity {
            icon: icons::Icon::Crop,
            title: "Auto-crop",
        }
        .inspector(
            ui,
            |ui| {
                ui.add(egui::Label::new(&scope).truncate())
                    .on_hover_text(&edit.summary);
            },
            |ui, _| {
                if has_proposals {
                    ui.add_enabled_ui(editable, |ui| {
                        controls::parameter(ui, controls::Parameter::CropMargin, &mut margin);
                        ui.checkbox(&mut show, "Show crop outline")
                            .on_hover_text("Hold H to hide the artwork temporarily.");
                    });
                }
            },
        );
        if let Some(edit) = &mut self.auto_crop_edit {
            edit.show_outline = show;
        }
        if margin != self.auto_crop_margin {
            self.adjust_auto_crop_margin(margin);
        }
    }
    fn record_crop_problem(&mut self, source: SourceId, reason: String) {
        let Some(edit) = &mut self.auto_crop_edit else {
            return;
        };
        if let Some((_, old)) = edit.problems.iter_mut().find(|(id, _)| *id == source) {
            *old = reason;
        } else {
            edit.problems.push((source, reason));
        }
    }
    pub(super) fn auto_crop_batch_warning(&self) -> Option<&'static str> {
        let edit = self.auto_crop_edit.as_ref()?;
        if edit.entries.is_empty() {
            Some("No crop proposals are available for this batch")
        } else if !self.auto_crop_valid() {
            Some("Crop batch is out of date; cancel and start again")
        } else {
            None
        }
    }
    pub(super) fn auto_crop_problem(&self, id: SourceId) -> Option<&str> {
        let edit = self.auto_crop_edit.as_ref()?;
        let entry = edit.entries.iter().find(|entry| entry.source == id);
        let problem = edit.problems.iter().find(|(source, _)| *source == id);
        if (entry.is_some() || problem.is_some()) && !edit.belongs_to(self) {
            return Some("Crop proposals are out of date");
        }
        if let Some(entry) = entry {
            if !entry.current(self) {
                return Some("Crop proposal is out of date");
            }
            if !self.busy
                && self.current_source == Some(id)
                && !self.loaded_crop_source_current(id, &entry.file)
            {
                return Some("Loaded photo no longer matches the crop proposal");
            }
        }
        problem.map(|(_, reason)| reason.as_str())
    }
    fn loaded_crop_source_current(&self, id: SourceId, file: &thumbnails::FileKey) -> bool {
        if self.current_source != Some(id) {
            return true;
        }
        self.ready.as_ref().is_some_and(|r| {
            self.document.project.source(id).is_ok_and(|s| {
                r.record.development == s.development
                    && r.area == Rect::from(s.development.active_area)
                    && self.document.project.development.as_ref() == Some(&r.development)
                    && &r.file == file
                    && self
                        .document
                        .source_path(id)
                        .is_ok_and(|p| document::same_path(&r.path, &p))
            })
        })
    }
    fn crop_proposal(&self, id: SourceId) -> Result<Entry, Error> {
        let source = self.document.project.source(id)?;
        let areas = &self
            .document
            .project
            .analysis_regions
            .iter()
            .find(|r| r.source == id)
            .ok_or(Error::UnusableSample(
                "no retained frame evidence; run Analyze roll first",
            ))?
            .areas;
        let path = self.document.source_path(id)?;
        let file = thumbnails::FileKey::new(&path);
        if file.revision.is_none() {
            return Err(Error::UnusableSample("source file is unavailable"));
        }
        if !self.loaded_crop_source_current(id, &file) {
            return Err(Error::UnusableSample(
                "loaded source changed; reopen the photo",
            ));
        }
        let active = source.development.active_area.into();
        let proposal = framing::propose(active, areas)?;
        let crop = framing::inward_margin(proposal, active, self.auto_crop_margin)?;
        if !file.current_at(&path) {
            return Err(Error::UnusableSample("source changed during auto-crop"));
        }
        Ok(Entry {
            source: id,
            proposal,
            before: source.crop,
            draft: accepted(crop, active),
            file,
            development: source.development.clone(),
            geometry: areas.picture_geometry.clone().expect("eligible frame"),
            constraints: areas.picture_constraints.clone(),
        })
    }
    fn auto_crop_photo(&mut self) {
        if self.busy || self.fullscreen.is_some() || self.crop_edit.is_none() {
            return;
        }
        let Some(id) = self.current_source else {
            return;
        };
        self.sync_document();
        match self.crop_proposal(id) {
            Ok(entry) => {
                self.initialize_crop_frame(
                    entry
                        .draft
                        .unwrap_or(Crop::from_region(entry.development.active_area.into())),
                );
                self.crop_edit.as_mut().unwrap().auto_problem = None;
                self.status = "Auto-crop proposed".into();
            }
            Err(error) => {
                let problem = format!("Automatic crop unavailable: {}", reason(error));
                self.status = format!("Crop unchanged: {problem}");
                self.crop_edit.as_mut().unwrap().auto_problem = Some(problem);
            }
        }
    }
    pub(super) fn auto_crop(&mut self, target: Target) {
        if target == Target::Photo {
            self.auto_crop_photo();
            return;
        }
        if !self.auto_crop_entry_available() {
            return;
        }
        self.sync_document();
        let ids: Vec<_> = self
            .document
            .project
            .order
            .iter()
            .copied()
            .filter(|id| match target {
                Target::Photo => Some(*id) == self.current_source,
                Target::Roll => true,
                Target::Selected => self.roll_selection.contains(*id),
            })
            .collect();
        let attempted = ids.len();
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for id in ids {
            let result = self.crop_proposal(id);
            match result {
                Ok(e) => entries.push(e),
                Err(error) => {
                    let reason = reason(error);
                    problems.push((id, format!("No crop proposal: {reason}")));
                }
            }
        }
        let count = entries.len();
        let skipped = problems.len();
        if attempted > 0 {
            self.close_adjustments();
            self.auto_crop_edit = Some(Draft {
                document: Arc::downgrade(self.document.cache()),
                development: self.document.project.development.clone(),
                order: self.document.project.order.clone(),
                target,
                attempted,
                summary: String::new(),
                entries,
                problems,
                origin: self.current_source,
                viewport: self.viewport,
                source_view: self.source,
                reveal_controls: true,
                show_outline: true,
                drag: None,
            });
            self.viewport.fit();
            self.last_layout = None;
            self.original.invalidate();
        }
        self.status = format!("Auto-crop: {count} proposed, {skipped} skipped");
        if let Some(edit) = &mut self.auto_crop_edit {
            edit.summary = self.status.clone();
        }
    }
    /// An explicit presentation-only override. The accepted document and current crop stay untouched.
    pub(super) fn auto_crop_preview(&self, id: SourceId) -> Option<Option<Crop>> {
        let edit = self.auto_crop_edit.as_ref()?;
        if !edit.belongs_to(self) {
            return None;
        }
        edit.entries
            .iter()
            .find(|e| e.source == id)
            .filter(|e| e.current(self))
            .map(|e| e.draft)
    }
    pub(super) fn adjust_auto_crop_margin(&mut self, percent: f64) {
        if !self.auto_crop_editable() {
            return;
        }
        if !percent.is_finite() || !(0.0..=framing::MAX_MARGIN_PERCENT).contains(&percent) {
            self.status = "Auto-crop margin rejected: invalid percentage".into();
            return;
        }
        let edit = self.auto_crop_edit.as_ref().unwrap();
        let mut failed_source = None;
        let proposals = edit
            .entries
            .iter()
            .map(|e| {
                failed_source = Some(e.source);
                let path = self.document.source_path(e.source)?;
                if !e.file.current_at(&path) || !self.loaded_crop_source_current(e.source, &e.file)
                {
                    return Err(Error::UnusableSample(
                        "source changed; Cancel and start Auto-crop again",
                    ));
                }
                let active = e.development.active_area.into();
                framing::inward_margin(e.proposal, active, percent)
                    .map(|crop| accepted(crop, active))
            })
            .collect::<Result<Vec<_>, _>>();
        match proposals {
            Ok(proposals) => {
                for (entry, draft) in self
                    .auto_crop_edit
                    .as_mut()
                    .unwrap()
                    .entries
                    .iter_mut()
                    .zip(proposals)
                {
                    entry.draft = draft;
                }
                self.auto_crop_margin = percent;
                self.status =
                    format!("Auto-crop draft margin: {percent:.2}% · Apply to accept, or Cancel.");
            }
            Err(error) => {
                let reason = reason(error);
                self.record_crop_problem(
                    failed_source.expect("failed proposal retains source"),
                    reason.clone(),
                );
                self.status = format!("Auto-crop draft unchanged: {reason}");
            }
        }
    }
    pub(super) fn finish_auto_crop(&mut self, apply: bool) {
        if apply && self.busy {
            return;
        }
        let Some(edit) = &self.auto_crop_edit else {
            return;
        };
        if apply {
            if edit.entries.is_empty() {
                self.status = "Auto-crop has no proposals to apply".into();
                return;
            }
            let mut failed_source = None;
            let candidate = (|| -> Result<_, Error> {
                if !self.auto_crop_valid() {
                    return Err(Error::UnusableSample(
                        "document, crops or measurements changed",
                    ));
                }
                let mut candidate = self.document.project.clone();
                for entry in &edit.entries {
                    failed_source = Some(entry.source);
                    let path = self.document.source_path(entry.source)?;
                    if !entry.file.current_at(&path)
                        || !self.loaded_crop_source_current(entry.source, &entry.file)
                    {
                        return Err(Error::UnusableSample("a source changed since the proposal"));
                    }
                    if let Some(crop) = entry.draft {
                        crop.region(entry.development.active_area.into())?;
                    }
                    candidate.source_mut(entry.source)?.crop = entry.draft;
                }
                failed_source = None;
                candidate.validate()?;
                Ok(candidate)
            })();
            match candidate {
                Ok(candidate) => {
                    let count = edit.entries.len();
                    self.document.project = candidate;
                    if let Some(id) = self.current_source {
                        self.crop = self
                            .document
                            .project
                            .source(id)
                            .expect("active source")
                            .crop;
                    }
                    self.status = format!("Auto-crop applied to {count} photos");
                }
                Err(error) => {
                    let reason = reason(error);
                    if let Some(id) = failed_source {
                        self.record_crop_problem(id, reason.clone());
                    }
                    self.status =
                        format!("Auto-crop not applied: {reason}. Cancel and start again.");
                    return;
                }
            }
        } else {
            self.status = "Auto-crop cancelled; previous crops unchanged".into();
        }
        let edit = self.auto_crop_edit.take().unwrap();
        self.source = edit.source_view;
        if !apply && self.current_source == edit.origin {
            self.viewport = edit.viewport;
        } else {
            self.viewport.fit();
        }
        self.last_layout = None;
        self.original.invalidate();
    }
}

#[cfg(test)]
#[path = "editor_auto_crop_tests.rs"]
mod tests;
