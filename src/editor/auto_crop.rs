use super::*;
use crate::{auto_crop as framing, project::SourceDevelopment, scan_regions::PictureGeometry};
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
    geometry: Option<PictureGeometry>,
    constraints: Vec<[f64; 3]>,
    spatial: Option<Arc<crate::spatial_metadata::Metadata>>,
}

pub(super) struct Draft {
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
        self.order == editor.document.state.project.order
            && self
                .entries
                .iter()
                .find_map(|entry| entry.spatial.as_ref())
                .is_none_or(|metadata| {
                    metadata.groups.iter().all(|group| {
                        group.sources.first().is_some_and(|row| {
                            editor
                                .document
                                .state
                                .project
                                .scan_basis()
                                .is_some_and(|basis| {
                                    basis.require_source(&row.input.development).is_ok()
                                })
                        }) && group.sources.iter().all(|row| {
                            editor
                                .document
                                .project
                                .source(row.input.source)
                                .is_ok_and(|source| source.development == row.input.development)
                        })
                    })
                })
    }
}

impl Entry {
    fn current(&self, editor: &Editor) -> bool {
        let Ok(source) = editor.document.state.project.source(self.source) else {
            return false;
        };
        editor
            .document
            .source_path(self.source)
            .is_ok_and(|path| editor.document.file_key(&path) == self.file)
            && source.crop == self.before
            && (editor.document.visit.current_source != Some(self.source)
                || editor.document.visit.crop == self.before)
            && source.development == self.development
            && if let Some(metadata) = &self.spatial {
                Arc::ptr_eq(metadata, &editor.document.spatial.ready())
            } else {
                editor
                    .document
                    .project
                    .analysis_regions
                    .iter()
                    .find(|r| r.source == self.source)
                    .is_some_and(|r| {
                        !r.areas.picture_edited
                            && r.areas.picture_observed == [true; 4]
                            && r.areas.picture_weak == [false; 4]
                            && r.areas.picture_geometry.as_ref() == self.geometry.as_ref()
                            && r.areas.picture_constraints == self.constraints
                    })
            }
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
        self.document.auto_crop_edit.is_none()
            && !self.busy
            && self.document.visit.ready.is_some()
            && self.roll_edit_available()
            && !self.document.visit.dust.cleanup
            && self.document.analysis_review.is_none()
            && self.document.removal_pending.is_none()
    }

    pub(super) fn auto_crop_valid(&self) -> bool {
        self.document.auto_crop_edit.as_ref().is_some_and(|d| {
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
            && self.active_pick().is_none()
            && self.document.visit.crop_edit.is_none()
            && self.document.flat_edit.is_none()
            && !self.document.visit.dust.cleanup
            && self.auto_crop_valid()
    }

    pub(super) fn auto_crop_action(&mut self, ui: &mut egui::Ui, target: Target, label: &str) {
        let available = if target == Target::Photo {
            !self.busy
                && self.document.visit.crop_edit.is_some()
                && self.document.visit.ready.is_some()
        } else {
            self.auto_crop_entry_available()
        };
        if presentation::TextAction::new(label)
            .enabled(available)
            .show(ui)
            .clicked()
        {
            self.document.auto_crop_requested = Some(target);
        }
    }

    pub(super) fn auto_crop_controls(&mut self, ui: &mut egui::Ui) {
        if !self.document.input.capabilities().auto_crop {
            return;
        }
        let Some(edit) = &mut self.document.auto_crop_edit else {
            return;
        };
        if std::mem::take(&mut edit.reveal_controls) {
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
        let edit = self.document.auto_crop_edit.as_ref().unwrap();
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
        if let Some(edit) = &mut self.document.auto_crop_edit {
            edit.show_outline = show;
        }
        if margin != self.auto_crop_margin {
            self.adjust_auto_crop_margin(margin);
        }
    }

    fn record_crop_problem(&mut self, source: SourceId, reason: String) {
        let Some(edit) = &mut self.document.auto_crop_edit else {
            return;
        };
        if let Some((_, old)) = edit.problems.iter_mut().find(|(id, _)| *id == source) {
            *old = reason;
        } else {
            edit.problems.push((source, reason));
        }
    }

    pub(super) fn auto_crop_batch_warning(&self) -> Option<&'static str> {
        let edit = self.document.auto_crop_edit.as_ref()?;
        if edit.entries.is_empty() {
            Some("No crop proposals are available for this batch")
        } else if !self.auto_crop_valid() {
            Some("Crop batch is out of date; cancel and start again")
        } else {
            None
        }
    }

    pub(super) fn auto_crop_problem(&self, id: SourceId) -> Option<&str> {
        let edit = self.document.auto_crop_edit.as_ref()?;
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
                && self.document.visit.current_source == Some(id)
                && !self.loaded_crop_source_current(id, &entry.file)
            {
                return Some("Loaded photo no longer matches the crop proposal");
            }
        }
        problem.map(|(_, reason)| reason.as_str())
    }

    fn loaded_crop_source_current(&self, id: SourceId, file: &thumbnails::FileKey) -> bool {
        if self.document.visit.current_source != Some(id) {
            return true;
        }
        self.document.visit.ready.as_ref().is_some_and(|r| {
            self.document.state.project.source(id).is_ok_and(|s| {
                r.record.development == s.development
                    && r.area == Rect::from(s.development.active_area)
                    && &r.file == file
                    && self
                        .document
                        .source_path(id)
                        .is_ok_and(|p| document::same_path(&r.path, &p))
            })
        })
    }

    fn crop_proposal(&self, id: SourceId) -> Result<Entry, Error> {
        let source = self.document.state.project.source(id)?;
        let path = self.document.source_path(id)?;
        let file = self.document.file_key(&path);
        if !self.loaded_crop_source_current(id, &file) {
            return Err(Error::UnusableSample(
                "loaded source changed; reopen the photo",
            ));
        }
        let active = source.development.active_area.into();
        let metadata = self.document.spatial.ready();
        let regions = metadata
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == id);
        let (proposal, geometry, constraints, spatial) = if let Some(regions) = regions {
            if regions.input.development != source.development
                || file.revision.clone() != Some(regions.input.revision.clone())
            {
                return Err(Error::UnusableSample(
                    "shared picture regions are out of date",
                ));
            }
            (
                framing::propose_spatial(regions)?,
                None,
                vec![],
                Some(metadata.clone()),
            )
        } else {
            let areas = &self
                .document
                .project
                .analysis_regions
                .iter()
                .find(|r| r.source == id)
                .ok_or(Error::UnusableSample(
                    "no retained frame evidence or prepared picture regions",
                ))?
                .areas;
            (
                framing::propose(active, areas)?,
                areas.picture_geometry.clone(),
                areas.picture_constraints.clone(),
                None,
            )
        };
        let crop = framing::inward_margin(proposal, active, self.auto_crop_margin)?;
        Ok(Entry {
            source: id,
            proposal,
            before: source.crop,
            draft: accepted(crop, active),
            file,
            development: source.development.clone(),
            geometry,
            constraints,
            spatial,
        })
    }

    fn auto_crop_photo(&mut self) {
        if self.busy || self.fullscreen.is_some() || self.document.visit.crop_edit.is_none() {
            return;
        }
        let Some(id) = self.document.visit.current_source else {
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
                self.document.visit.crop_edit.as_mut().unwrap().auto_problem = None;
                self.status = "Auto-crop proposed".into();
            }
            Err(error) => {
                let problem = format!("Automatic crop unavailable: {}", reason(error));
                self.status = format!("Crop unchanged: {problem}");
                self.document.visit.crop_edit.as_mut().unwrap().auto_problem = Some(problem);
            }
        }
    }

    pub(super) fn request_auto_crop(&mut self, target: Target, cx: &UiContext<'_, Self>) {
        let available = if target == Target::Photo {
            !self.busy
                && self.document.visit.crop_edit.is_some()
                && self.document.visit.ready.is_some()
        } else {
            self.auto_crop_entry_available()
        };
        if !available {
            return;
        }
        let sources = self.auto_crop_source_ids(target);
        self.request_spatial(spatial::Consumer::AutoCrop { target, sources }, cx);
    }

    fn auto_crop_source_ids(&self, target: Target) -> Vec<SourceId> {
        self.document
            .state
            .project
            .order
            .iter()
            .copied()
            .filter(|id| match target {
                Target::Photo => Some(*id) == self.document.visit.current_source,
                Target::Roll => true,
                Target::Selected => self.document.roll_selection.contains(*id),
            })
            .collect()
    }

    #[cfg(test)]
    pub(super) fn auto_crop(&mut self, target: Target) {
        self.auto_crop_sources(target, self.auto_crop_source_ids(target));
    }

    pub(super) fn auto_crop_sources(&mut self, target: Target, ids: Vec<SourceId>) {
        if target == Target::Photo {
            self.auto_crop_photo();
            return;
        }
        if !self.auto_crop_entry_available() {
            return;
        }
        self.sync_document();
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
            self.finish_adjustment_layer_interaction();
            self.document.auto_crop_edit = Some(Draft {
                order: self.document.state.project.order.clone(),
                target,
                attempted,
                summary: String::new(),
                entries,
                problems,
                origin: self.document.visit.current_source,
                viewport: self.document.visit.viewport,
                source_view: self.document.source,
                reveal_controls: true,
                show_outline: true,
                drag: None,
            });
            self.document.visit.viewport.fit();
            self.document.visit.last_layout = None;
            self.invalidate_original();
        }
        self.status = format!("Auto-crop: {count} proposed, {skipped} skipped");
        if let Some(edit) = &mut self.document.auto_crop_edit {
            edit.summary = self.status.clone();
        }
    }

    /// An explicit presentation-only override. The accepted document and current crop stay untouched.
    pub(super) fn auto_crop_preview(&self, id: SourceId) -> Option<Option<Crop>> {
        let edit = self.document.auto_crop_edit.as_ref()?;
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
        let edit = self.document.auto_crop_edit.as_ref().unwrap();
        let mut failed_source = None;
        let proposals = edit
            .entries
            .iter()
            .map(|e| {
                failed_source = Some(e.source);
                if !self.loaded_crop_source_current(e.source, &e.file) {
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
                    .document
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
        let Some(edit) = &self.document.auto_crop_edit else {
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
                let ready = self.document.capture().ready_spatial_metadata();
                if edit.entries.iter().any(|entry| {
                    entry
                        .spatial
                        .as_ref()
                        .is_some_and(|metadata| **metadata != ready)
                }) {
                    return Err(Error::UnusableSample(
                        "shared region dependencies changed since the proposal",
                    ));
                }
                let mut candidate = self.document.state.project.clone();
                for entry in &edit.entries {
                    failed_source = Some(entry.source);
                    if !self.loaded_crop_source_current(entry.source, &entry.file) {
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
                    let before = self.history_snapshot();
                    self.document.state.project = candidate;
                    if let Some(id) = self.document.visit.current_source {
                        self.document.visit.crop = self
                            .document
                            .project
                            .source(id)
                            .expect("active source")
                            .crop;
                    }
                    self.record_edit(before, "Auto-crop", None);
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
        let edit = self.document.auto_crop_edit.take().unwrap();
        self.document.source = edit.source_view;
        if !apply && self.document.visit.current_source == edit.origin {
            self.document.visit.viewport = edit.viewport;
        } else {
            self.document.visit.viewport.fit();
        }
        self.document.visit.last_layout = None;
        self.invalidate_original();
    }
}

#[cfg(test)]
mod tests;
