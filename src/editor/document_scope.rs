use super::{Ready, document::Document, history::History, photo_scope::PhotoScope};
use crate::{project::SourceId, scan::AnalysisInput, source::SourceKey};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod spatial_tests;

#[derive(Clone)]
struct SamplingEntry {
    source: SourceId,
    key: SourceKey,
    input: Arc<AnalysisInput>,
}

/// One retained original-camera raster per source, keyed by its captured development.
#[derive(Clone, Default)]
pub struct SamplingInputs {
    entries: Arc<Vec<SamplingEntry>>,
}

impl SamplingInputs {
    pub fn get(&self, source: SourceId, key: &SourceKey) -> Option<Arc<AnalysisInput>> {
        self.entries
            .iter()
            .find(|e| e.source == source && &e.key == key)
            .map(|e| e.input.clone())
    }

    pub fn insert(&mut self, source: SourceId, key: SourceKey, input: Arc<AnalysisInput>) {
        let entries = Arc::make_mut(&mut self.entries);
        entries.retain(|e| e.source != source);
        entries.push(SamplingEntry { source, key, input });
    }

    pub fn retain(&mut self, document: &Document) {
        let matches = |e: &SamplingEntry| {
            document
                .project
                .source(e.source)
                .is_ok_and(|source| !source.reference_only)
                && document.source_key(e.source).is_ok_and(|key| key == e.key)
        };
        if self.entries.iter().all(&matches) {
            return;
        }
        Arc::make_mut(&mut self.entries).retain(matches);
    }

    /// Owned camera payloads, once per allocation; lookup/Arc/allocator overhead is excluded.
    pub fn retained_bytes(&self) -> u64 {
        let mut seen = std::collections::HashSet::new();
        self.entries
            .iter()
            .filter(|e| seen.insert(Arc::as_ptr(&e.input)))
            .map(|e| e.input.retained_bytes())
            .sum()
    }
}

#[cfg(test)]
mod sampling_tests;

/// Accepted document authority, cross-photo tool context and the dependent visit.
/// Captured documents are data snapshots, not histories or continuation owners.
pub(super) struct DocumentScope {
    pub thumbnails: super::Thumbnails,
    pub document_preview: super::editing::Preview<Document>,
    pub project_preview: super::editing::Preview<crate::project::Project>,
    pub controls_edit: Option<super::editing::ControlEdit>,
    pub gamma_pivot: super::gamma_pivot::State,

    #[cfg(test)]
    pub auto_crop: super::auto_crop::State,
    pub source: bool,
    pub selected: usize,
    pub roll_selection: super::RollSelection,

    pub export: super::export::State,
    pub correction: Option<Arc<super::Correction>>,
    pub flat_correction: Option<async_executor::Task<()>>,
    pub hosted_tool: super::tool::Slot,
    pub state: Document,
    pub cache: Arc<crate::source::SourceCache>,
    pub exposures: Arc<crate::exposure_metadata::Store>,
    pub spatial: crate::spatial_metadata::Store,
    pub sampling_inputs: SamplingInputs,
    pub spatial_preparing: Option<super::spatial::Pending>,
    #[cfg(test)]
    pub spatial_reply: Option<
        futures_channel::oneshot::Receiver<
            Result<Option<crate::spatial_metadata::Metadata>, String>,
        >,
    >,
    pub spatial_resumed: Arc<event_listener::Event>,
    pub history: History,
    pub visit: PhotoScope,
    pub embedded_preview: Option<super::thumbnails::EmbeddedWork>,
    pub thumbnail_work: super::thumbnails::ThumbnailWork,
    pub write: super::save::State,
    pub look: Option<super::look::Operation>,
    pub dialog_action: Option<async_executor::Task<()>>,
    pub dust: super::dust::State,
    #[cfg(test)]
    pub analysis_preparation_work:
        Option<super::analysis::testing::Work<super::analysis::Approved>>,
    #[cfg(test)]
    pub analysis_fit_work: Option<super::analysis::testing::Work<Box<super::analysis::Outcome>>>,
}

impl DocumentScope {
    pub fn replacement(&self, state: Document) -> Self {
        let mut next: Self = state.into();
        next.thumbnails = self.thumbnails.for_document(next.cache.clone());
        next.visit.working_view = self.visit.working_view.for_visit();
        next
    }

    pub(super) fn project_snapshot(&self) -> super::editing::ProjectVersion {
        self.visit
            .project_preview
            .snapshot()
            .or_else(|| self.project_preview.snapshot())
            .map(Into::into)
            .or_else(|| {
                self.document_preview
                    .snapshot()
                    .map(|document| document.project.clone())
            })
            .unwrap_or_else(|| self.state.project.clone())
    }

    pub(super) fn target(&self) -> super::OutputTarget {
        match self.project_snapshot().output.tiff {
            crate::project::TiffTarget::Srgb => super::OutputTarget::Srgb,
            crate::project::TiffTarget::DisplayP3 => super::OutputTarget::DisplayP3,
        }
    }

    #[cfg(test)]
    pub(super) fn set_target(&mut self, target: super::OutputTarget) {
        self.state.project.output.tiff = match target {
            super::OutputTarget::Srgb => crate::project::TiffTarget::Srgb,
            super::OutputTarget::DisplayP3 => crate::project::TiffTarget::DisplayP3,
        };
    }

    pub(super) fn effective_document(&self) -> Document {
        let mut document = self.capture();
        document.project = self.project_snapshot();
        document
    }

    pub fn capture(&self) -> Document {
        let mut document = self.state.clone();
        document.spatial_metadata = self.spatial.ready();
        document.capture_exposure(&self.exposures);
        document
    }

    pub fn accept_save(&mut self, saved: Document) {
        self.history
            .rebase(self.state.path.as_deref(), saved.path.as_deref());
        self.state.accept_save(saved);
    }

    pub(super) fn retire_edits(&mut self) {
        self.document_preview = Default::default();
        self.project_preview = Default::default();
        self.visit.project_preview = Default::default();
        self.controls_edit = None;

        self.visit.controls_edit = None;
    }

    pub fn install(&mut self, document: Document) {
        self.retire_edits();
        self.flat_correction = None;
        self.sampling_inputs.retain(&document);
        self.exposures.restore(&document.exposure_metadata, |id| {
            document.exposure_origin(id)
        });
        self.state = document;
    }

    pub(super) fn exposure_value(
        &self,
        source: &crate::project::Source,
    ) -> Option<crate::exposure_reference::ReferenceExposure> {
        self.state
            .exposure_origin(source.id)
            .and_then(|origin| self.exposures.get(source.id, &origin))
            .or_else(|| self.state.reference_exposure(source).cloned())
    }

    pub(super) fn positive_matrix(
        &self,
        source: &crate::project::Source,
        input: crate::processing::InputSettings,
    ) -> Result<glam::DMat3, crate::Error> {
        input.positive_matrix(
            glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1),
            self.exposure_value(source)
                .as_ref()
                .map(|reference| reference.ev()),
        )
    }

    pub fn cache_ready(&mut self, ready: &Ready) {
        if let (Some(id), Some(reference)) = (self.state.find_source(&ready.path), &ready.reference)
        {
            let _ = self
                .state
                .exposure_slot(&self.exposures, id)
                .set(reference.clone());
        }
        self.state.capture_exposure(&self.exposures);
        let key = crate::source::SourceKey::for_load(
            &ready.path,
            ready.file.clone(),
            &ready.record.development,
        );
        if let Err(error) = self
            .cache
            .publish_source(key.clone(), ready.developed.clone())
        {
            log::warn!(error = error.to_string().as_str(); "source_cache_publication_failed");
        }
        if let Some((_, proxy)) = &ready.thumbnail
            && let Err(error) = self.cache.publish_proxy(key, proxy.clone())
        {
            log::warn!(error = error.to_string().as_str(); "source_proxy_publication_failed");
        }
    }

    pub fn same_visit(&self, ready: Option<&Ready>, reuse_ready: bool) -> bool {
        if reuse_ready {
            return true;
        }
        match (self.visit.canvas.ready.as_ref(), ready) {
            (Some(current), Some(candidate)) => {
                current.path == candidate.path
                    && current.file == candidate.file
                    && current.record.development == candidate.record.development
            }
            (None, Some(candidate)) => self.visit.source.matches(candidate),
            (None, None) => true,
            _ => false,
        }
    }
}

impl From<Document> for DocumentScope {
    fn from(state: Document) -> Self {
        let cache = Arc::new(crate::source::SourceCache::new(0));
        let exposures = Arc::new(crate::exposure_metadata::Store::restored(
            &state.exposure_metadata,
            |id| state.exposure_origin(id),
        ));
        Self {
            thumbnails: super::Thumbnails::for_sources(cache.clone()),
            document_preview: Default::default(),
            project_preview: Default::default(),

            controls_edit: None,
            correction: None,
            flat_correction: None,
            hosted_tool: Default::default(),
            selected: 0,
            roll_selection: super::RollSelection::default(),

            export: Default::default(),
            gamma_pivot: Default::default(),

            source: true,
            #[cfg(test)]
            auto_crop: Default::default(),
            spatial: crate::spatial_metadata::Store::restored((*state.spatial_metadata).clone()),
            spatial_preparing: None,
            sampling_inputs: Default::default(),
            #[cfg(test)]
            spatial_reply: None,
            spatial_resumed: Arc::default(),
            state,
            cache,
            exposures,
            history: History::default(),
            visit: PhotoScope::default(),
            embedded_preview: None,
            thumbnail_work: Default::default(),
            write: super::save::State::default(),
            look: None,
            dialog_action: None,
            dust: super::dust::State::default(),
            #[cfg(test)]
            analysis_preparation_work: None,
            #[cfg(test)]
            analysis_fit_work: None,
        }
    }
}

impl Deref for DocumentScope {
    type Target = Document;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for DocumentScope {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}
