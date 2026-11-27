use crate::{
    Error, auto_crop as framing,
    crop::Crop,
    editor::document::Document,
    project::{Project, SourceDevelopment, SourceId},
    raw::Rect,
    source::{FileKey, Initialization},
    spatial_metadata::{Input, Metadata},
};
use std::{cell::OnceCell, collections::HashSet, sync::Arc};

pub(super) struct LoadedPhoto<'a> {
    pub(super) file: &'a FileKey,
    pub(super) development: &'a SourceDevelopment,
    pub(super) area: Rect,
}

pub(super) struct SelectedPhoto<'a> {
    pub(super) id: SourceId,
    pub(super) effective_crop: Option<Crop>,
    pub(super) loaded: Option<LoadedPhoto<'a>>,
}

pub(super) struct Inputs<'a> {
    pub(super) document: &'a Document,
    pub(super) selected: Option<SelectedPhoto<'a>>,
    spatial: Arc<Metadata>,
    initialization: &'a Initialization,
    spatial_inputs: OnceCell<Vec<Input>>,
}

impl<'a> Inputs<'a> {
    pub(super) fn new(
        document: &'a Document,
        selected: Option<SelectedPhoto<'a>>,
        spatial: Arc<Metadata>,
        initialization: &'a Initialization,
    ) -> Self {
        Self {
            document,
            selected,
            spatial,
            initialization,
            spatial_inputs: OnceCell::new(),
        }
    }

    fn spatial_matches(&self, metadata: &Metadata) -> bool {
        let inputs = self.spatial_inputs.get_or_init(|| {
            let mut document = self.document.clone();
            document.capture_initialization(self.initialization);
            document.spatial_inputs()
        });
        metadata.matching(&self.document.project, inputs) == *metadata
    }

    pub(super) fn loaded_source_matches(&self, id: SourceId, file: &FileKey) -> bool {
        let loaded = self
            .selected
            .as_ref()
            .filter(|photo| photo.id == id)
            .and_then(|photo| photo.loaded.as_ref());
        loaded.is_none_or(|loaded| {
            self.document.project.source(id).is_ok_and(|source| {
                loaded.development == &source.development
                    && loaded.area == Rect::from(source.development.active_area)
                    && loaded.file == file
            })
        })
    }

    fn file(&self, id: SourceId) -> Result<FileKey, Error> {
        let path = self.document.source_path(id)?;
        Ok(self.document.file_key(&path))
    }
}

#[derive(Clone)]
pub(super) struct Entry {
    pub(super) source: SourceId,
    pub(super) proposal: Crop,
    pub(super) draft: Option<Crop>,
    pub(super) file: FileKey,
    pub(super) development: SourceDevelopment,
    before: Option<Crop>,
}

impl Entry {
    pub(super) fn propose(inputs: &Inputs<'_>, id: SourceId, margin: f64) -> Result<Self, Error> {
        let source = inputs.document.project.source(id)?;
        let file = inputs.file(id)?;
        if !inputs.loaded_source_matches(id, &file) {
            return Err(Error::UnusableSample(
                "loaded source changed; reopen the photo",
            ));
        }

        let regions = inputs
            .spatial
            .groups
            .iter()
            .flat_map(|group| &group.sources)
            .find(|row| row.input.source == id)
            .ok_or(Error::UnusableSample("no prepared picture regions"))?;
        if regions.input.development != source.development
            || file.revision.as_ref() != Some(&regions.input.revision)
        {
            return Err(Error::UnusableSample(
                "shared picture regions are out of date",
            ));
        }

        let active = source.development.active_area.into();
        let proposal = framing::propose_spatial(regions)?;
        let crop = framing::inward_margin(proposal, active, margin)?;
        Ok(Self {
            source: id,
            proposal,
            before: source.crop,
            draft: accepted(crop, active),
            file,
            development: source.development.clone(),
        })
    }

    pub(super) fn current(&self, inputs: &Inputs<'_>) -> bool {
        let project = &inputs.document.project;
        let Ok(source) = project.source(self.source) else {
            return false;
        };
        inputs.file(self.source).is_ok_and(|file| file == self.file)
            && source.crop == self.before
            && inputs
                .selected
                .as_ref()
                .is_none_or(|photo| photo.id != self.source || photo.effective_crop == self.before)
            && source.development == self.development
    }

    pub(super) fn crop(&self) -> Crop {
        self.draft
            .unwrap_or_else(|| Crop::from_region(self.development.active_area.into()))
    }
}

#[derive(Clone)]
pub(in crate::editor) struct Draft {
    spatial: Arc<Metadata>,
    roll_photos: HashSet<SourceId>,
    pub(super) scope: &'static str,
    pub(super) attempted: usize,
    pub(super) summary: String,
    pub(super) entries: Vec<Entry>,
    problems: Vec<(SourceId, String)>,
    pub(super) reveal_controls: bool,
    pub(super) show_outline: bool,
}

impl Draft {
    pub(super) fn capture(
        inputs: &Inputs<'_>,
        photos: Vec<SourceId>,
        scope: &'static str,
        margin: f64,
    ) -> Self {
        let attempted = photos.len();
        let mut entries = Vec::new();
        let mut problems = Vec::new();
        for id in photos {
            match Entry::propose(inputs, id, margin) {
                Ok(entry) => entries.push(entry),
                Err(error) => problems.push((id, format!("No crop proposal: {}", reason(error)))),
            }
        }
        let summary = format!(
            "Auto-crop: {} proposed, {} skipped",
            entries.len(),
            problems.len()
        );
        Self {
            spatial: inputs.spatial.clone(),
            roll_photos: inputs.document.project.order.iter().copied().collect(),
            scope,
            attempted,
            summary,
            entries,
            problems,
            reveal_controls: true,
            show_outline: true,
        }
    }

    pub(super) fn belongs_to(&self, inputs: &Inputs<'_>) -> bool {
        let order = &inputs.document.project.order;
        self.roll_photos.len() == order.len()
            && order.iter().all(|id| self.roll_photos.contains(id))
            && Arc::ptr_eq(&self.spatial, &inputs.spatial)
            && inputs.spatial_matches(&self.spatial)
    }

    pub(super) fn valid(&self, inputs: &Inputs<'_>) -> bool {
        self.belongs_to(inputs)
            && !self.entries.is_empty()
            && self.entries.iter().all(|entry| {
                entry.current(inputs) && !self.problems.iter().any(|(id, _)| *id == entry.source)
            })
    }

    pub(super) fn candidate(&self, inputs: &Inputs<'_>) -> Result<Arc<Project>, Rejection> {
        if !self.valid(inputs) {
            return Err(Rejection::batch(Error::UnusableSample(
                "document, crops or measurements changed",
            )));
        }

        let mut candidate = inputs.document.project.clone();
        for entry in &self.entries {
            let reject = |error| Rejection::photo(entry.source, error);
            if !inputs.loaded_source_matches(entry.source, &entry.file) {
                return Err(reject(Error::UnusableSample(
                    "a source changed since the proposal",
                )));
            }
            if let Some(crop) = entry.draft {
                crop.region(entry.development.active_area.into())
                    .map_err(reject)?;
            }
            candidate.source_mut(entry.source).map_err(reject)?.crop = entry.draft;
        }
        Ok(candidate.snapshot())
    }

    pub(super) fn problem(
        &self,
        inputs: &Inputs<'_>,
        id: SourceId,
        check_loaded: bool,
    ) -> Option<String> {
        let entry = self.entries.iter().find(|entry| entry.source == id);
        let problem = self.problems.iter().find(|(source, _)| *source == id);
        if (entry.is_some() || problem.is_some()) && !self.belongs_to(inputs) {
            return Some("Crop proposals are out of date".into());
        }
        if let Some(entry) = entry {
            if !entry.current(inputs) {
                return Some("Crop proposal is out of date".into());
            }
            if check_loaded && !inputs.loaded_source_matches(id, &entry.file) {
                return Some("Loaded photo no longer matches the crop proposal".into());
            }
        }
        problem.map(|(_, reason)| reason.clone())
    }

    pub(super) fn record_problem(&mut self, source: SourceId, reason: String) {
        if let Some((_, old)) = self.problems.iter_mut().find(|(id, _)| *id == source) {
            *old = reason;
        } else {
            self.problems.push((source, reason));
        }
    }

    pub(super) fn margin_entry(&self, inputs: &Inputs<'_>) -> Option<&Entry> {
        let id = inputs.selected.as_ref()?.id;
        self.entries
            .iter()
            .find(|entry| entry.source == id)
            .filter(|entry| entry.current(inputs) && inputs.loaded_source_matches(id, &entry.file))
    }
}

#[derive(Debug)]
pub(super) struct Rejection {
    pub(super) source: Option<SourceId>,
    pub(super) error: Error,
}

impl Rejection {
    pub(super) fn batch(error: Error) -> Self {
        Self {
            source: None,
            error,
        }
    }

    fn photo(source: SourceId, error: Error) -> Self {
        Self {
            source: Some(source),
            error,
        }
    }
}

pub(super) fn accepted(crop: Crop, active: Rect) -> Option<Crop> {
    (crop != Crop::from_region(active)).then_some(crop)
}

pub(super) fn reason(error: Error) -> String {
    match error {
        Error::UnusableSample(s) => s.to_owned(),
        error => error.to_string(),
    }
}

#[cfg(test)]
mod tests;
