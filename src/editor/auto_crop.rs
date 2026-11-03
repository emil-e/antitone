use super::*;
use crate::{auto_crop as framing, project::SourceDevelopment, scan_regions::PictureGeometry};
mod interaction;
mod operation;
mod overlay;
pub(super) use interaction::AutoCropTool;
#[cfg(test)]
pub(super) use interaction::State;
pub(super) use operation::propose_photo_crop;

pub(super) enum Action {
    Batch {
        photos: Vec<SourceId>,
        scope: &'static str,
    },
}

#[derive(Clone)]
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

#[derive(Clone)]
pub(super) struct Draft {
    order: Vec<SourceId>,
    scope: &'static str,
    attempted: usize,
    summary: String,
    entries: Vec<Entry>,
    problems: Vec<(SourceId, String)>,
    pub(super) reveal_controls: bool,
    pub(super) show_outline: bool,
    drag: Option<overlay::MarginDrag>,
}

impl Draft {
    fn valid(&self, editor: &Editor) -> bool {
        self.belongs_to(editor)
            && !self.entries.is_empty()
            && self.entries.iter().all(|entry| {
                entry.current(editor) && !self.problems.iter().any(|(id, _)| *id == entry.source)
            })
    }

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

fn finish_photo_proposal(state: &mut CropEdit, proposal: Result<Crop, Error>) -> String {
    match proposal {
        Ok(crop) => {
            state.install_proposal(crop);
            state.auto_problem = None;
            "Auto-crop proposed".into()
        }
        Err(error) => {
            let problem = format!("Automatic crop unavailable: {}", reason(error));
            let status = format!("Crop unchanged: {problem}");
            state.auto_problem = Some(problem);
            status
        }
    }
}

fn reason(error: Error) -> String {
    match error {
        Error::UnusableSample(s) => s.to_owned(),
        error => error.to_string(),
    }
}

impl Editor {
    pub(super) fn auto_crop_entry_available(&self) -> bool {
        self.tool_start_available()
            && self.document.visit.canvas.ready.is_some()
            && !self.document.project.order.is_empty()
    }

    fn loaded_crop_source_current(&self, id: SourceId, file: &thumbnails::FileKey) -> bool {
        if self.document.visit.current_source != Some(id) {
            return true;
        }
        self.document.visit.canvas.ready.as_ref().is_some_and(|r| {
            self.document.state.project.source(id).is_ok_and(|s| {
                r.record.development == s.development
                    && r.area == Rect::from(s.development.active_area)
                    // The captured key already contains the source locator and revision.
                    && &r.file == file
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

    pub(super) fn photo_crop_proposal(&self, photo: SourceId) -> Result<Crop, Error> {
        let entry = self.crop_proposal(photo)?;
        Ok(entry
            .draft
            .unwrap_or(Crop::from_region(entry.development.active_area.into())))
    }

    #[cfg(test)]
    fn auto_crop_photo(&mut self, photo: SourceId) {
        if self.busy
            || self.fullscreen.is_some()
            || !self.cropping()
            || self.document.visit.current_source != Some(photo)
        {
            return;
        }
        self.sync_document();
        let result = self.photo_crop_proposal(photo);
        let status = finish_photo_proposal(&mut self.document.crop_edit_mut().unwrap(), result);
        self.status = status;
    }

    fn capture_crop_review(&mut self, photos: Vec<SourceId>, scope: &'static str) -> Option<Draft> {
        if photos.is_empty() || !self.auto_crop_entry_available() {
            return None;
        }
        self.sync_document();
        let attempted = photos.len();
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for id in photos {
            match self.crop_proposal(id) {
                Ok(entry) => entries.push(entry),
                Err(error) => problems.push((id, format!("No crop proposal: {}", reason(error)))),
            }
        }
        self.status = format!(
            "Auto-crop: {} proposed, {} skipped",
            entries.len(),
            problems.len()
        );
        Some(Draft {
            order: self.document.state.project.order.clone(),
            scope,
            attempted,
            summary: self.status.clone(),
            entries,
            problems,
            reveal_controls: true,
            show_outline: true,
            drag: None,
        })
    }

    #[cfg(test)]
    pub(super) fn auto_crop_preview(&self, id: SourceId) -> Option<Option<Crop>> {
        let state = self.document.auto_crop.current()?;
        let state = state.borrow();
        let edit = state.as_ref()?;
        if !edit.belongs_to(self) {
            return None;
        }
        edit.entries
            .iter()
            .find(|entry| entry.source == id)
            .filter(|entry| entry.current(self))
            .map(|entry| entry.draft)
    }

    #[cfg(test)]
    pub(super) fn auto_crop_problem(&self, id: SourceId) -> Option<String> {
        let state = self.document.auto_crop.current()?;
        state.borrow().as_ref()?.problem(self, id)
    }

    fn commit_auto_crop(&mut self, edit: &Draft) -> Result<(), (Option<SourceId>, Error)> {
        let mut failed_source = None;
        let candidate = (|| -> Result<_, Error> {
            if !edit.valid(self) {
                return Err(Error::UnusableSample(
                    "document, crops or measurements changed",
                ));
            }
            self.sync_document();
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
        })()
        .map_err(|error| (failed_source, error))?;
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
        self.document.visit.canvas.viewport.fit();
        self.status = format!("Auto-crop applied to {} photos", edit.entries.len());
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn begin_auto_crop_review(&mut self, photos: Vec<SourceId>, scope: &'static str) {
        if !self.prepare_tool() {
            return;
        }
        if let Some(review) = self.capture_crop_review(photos, scope) {
            AutoCropTool::new(Some(review)).install(self);
        }
    }

    #[cfg(test)]
    pub(super) fn finish_auto_crop(&mut self, apply: bool) {
        if self
            .current_tool()
            .is_some_and(|tool| tool.is::<AutoCropTool>())
        {
            self.hosted_action(if apply {
                tool::Action::Primary
            } else {
                tool::Action::Dismiss
            });
        }
    }

    #[cfg(test)]
    pub(super) fn auto_crop_valid(&self) -> bool {
        self.document
            .auto_crop
            .current()
            .is_some_and(|state| state.borrow().as_ref().is_some_and(|edit| edit.valid(self)))
    }

    #[cfg(test)]
    pub(super) fn adjust_auto_crop_margin(&mut self, margin: f64) {
        if let Some(state) = self.document.auto_crop.current()
            && let Some(edit) = state.borrow_mut().as_mut()
        {
            edit.adjust_margin(self, margin);
        }
    }
}

#[cfg(test)]
mod tests;
