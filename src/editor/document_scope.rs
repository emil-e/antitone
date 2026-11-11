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
    pub async fn acquire(
        &self,
        source: SourceId,
        physical: &crate::source::Input,
        provider: &dyn crate::source::SourceProvider,
        cancelled: &crate::cancellation::Cancellation,
    ) -> Result<Option<crate::source::CameraSamples>, crate::Error> {
        let cached = self
            .entries
            .iter()
            .find(|entry| entry.source == source)
            .map(|entry| (entry.key.clone(), entry.input.clone()));
        physical.camera_samples(provider, cached, cancelled).await
    }

    pub fn contains(&self, source: SourceId, physical: &crate::source::Input) -> bool {
        physical
            .cached_key()
            .is_some_and(|key| self.get(source, &key).is_some())
    }

    pub fn record(&mut self, source: SourceId, camera: &crate::source::CameraSamples) {
        self.insert(source, camera.key().clone(), camera.pixels());
    }

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

    pub fn retain(&mut self, document: &Document, initialization: &crate::source::Initialization) {
        let matches = |e: &SamplingEntry| {
            document
                .source_input(e.source, initialization)
                .ok()
                .and_then(|input| input.cached_key())
                .is_some_and(|key| key == e.key)
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
    pub importing: Option<async_executor::Task<()>>,
    pub input_change: Option<async_executor::Task<()>>,
    pub correction: super::flat::Resources,
    pub correction_restore: Option<async_executor::Task<()>>,
    pub flat_correction: Option<async_executor::Task<()>>,
    pub hosted_tool: super::tool::Slot,
    pub state: Document,
    pub cache: Arc<crate::source::SourceCache>,
    pub initialization: Arc<crate::source::Initialization>,
    pub spatial: crate::spatial_metadata::Store,
    pub sampling_inputs: SamplingInputs,
    pub source_previews: super::viewer_ui::SourcePreviews,
    pub spatial_preparing: Option<super::spatial::Pending>,
    #[cfg(test)]
    pub spatial_reply: Option<
        futures_channel::oneshot::Receiver<
            Result<Option<crate::spatial_metadata::Metadata>, String>,
        >,
    >,
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
            .map(|project| self.state.project.updated(project))
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
        document.capture_initialization(&self.initialization);
        document
    }

    pub fn source_input(&self, id: SourceId) -> Result<crate::source::Input, crate::Error> {
        self.state.source_input(id, &self.initialization)
    }

    pub fn development_for(
        &self,
        source: &crate::project::Source,
        input: crate::processing::InputSettings,
    ) -> crate::development_metadata::Development {
        let origin = self
            .state
            .exposure_origin(source.id)
            .unwrap_or_else(|| source.path.clone());
        self.initialization.developments.request(
            source,
            &origin,
            input,
            &self.state.development_metadata,
        )
    }

    pub fn image_recipe(
        &self,
        id: SourceId,
    ) -> Result<crate::image_recipe::ImageRecipe, crate::Error> {
        let source = self.state.project.source(id)?;
        let mut recipe = self
            .state
            .project
            .image_recipe_with_reference(source, self.exposure_value(source))?;
        recipe.neutral.source_development = self.development_for(source, self.state.project.input);
        Ok(recipe)
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
        self.sampling_inputs.retain(&document, &self.initialization);
        self.initialization
            .exposures
            .restore(&document.exposure_metadata, |id| {
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
            .and_then(|origin| self.initialization.exposures.get(source.id, &origin))
            .or_else(|| self.state.reference_exposure(source).cloned())
    }

    pub fn source_preview(&self, source: &crate::project::Source) -> crate::raw::SourcePreview {
        let Ok(path) = self.state.source_path(source.id) else {
            return Default::default();
        };
        let development = self.development_for(source, self.state.project.input);
        let Ok(development) = development.resolved() else {
            return Default::default();
        };
        let key = SourceKey::for_load(&path, self.state.file_key(&path), development);
        self.development_for(source, self.state.project.input)
            .source_preview()
            .unwrap_or_else(|| self.source_previews.get(&key))
    }

    pub fn cache_ready(&mut self, ready: &Ready) {
        if let (Some(id), Some(reference)) = (self.state.find_source(&ready.path), &ready.reference)
        {
            let _ = self
                .state
                .exposure_slot(&self.initialization, id)
                .set(reference.clone());
        }
        if let Ok(source) = self.state.project.source(ready.record.id) {
            self.development_for(source, self.state.project.input)
                .observe_preview(
                    ready
                        .developed
                        .metadata()
                        .source_preview(ready.developed.white_balance()),
                );
        }
        self.state.capture_initialization(&self.initialization);
        let key = crate::source::SourceKey::for_load(
            &ready.path,
            ready.file.clone(),
            &ready.record.development,
        );
        self.source_previews.insert(
            key.clone(),
            ready
                .developed
                .metadata()
                .source_preview(ready.developed.white_balance()),
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
}

impl From<Document> for DocumentScope {
    fn from(state: Document) -> Self {
        let cache = Arc::new(crate::source::SourceCache::new(0));
        let initialization = Arc::new(crate::source::Initialization::new(
            crate::exposure_metadata::Store::restored(&state.exposure_metadata, |id| {
                state.exposure_origin(id)
            }),
            crate::development_metadata::Store::restored(
                &state.development_metadata,
                &state.project,
                |id| state.exposure_origin(id),
            ),
        ));
        Self {
            thumbnails: super::Thumbnails::for_sources(cache.clone()),
            document_preview: Default::default(),
            project_preview: Default::default(),

            controls_edit: None,
            correction: Default::default(),
            correction_restore: None,
            flat_correction: None,
            hosted_tool: Default::default(),
            selected: 0,
            roll_selection: super::RollSelection::default(),

            export: Default::default(),
            importing: None,
            input_change: None,
            gamma_pivot: Default::default(),

            source: true,
            #[cfg(test)]
            auto_crop: Default::default(),
            spatial: crate::spatial_metadata::Store::restored((*state.spatial_metadata).clone()),
            spatial_preparing: None,
            sampling_inputs: Default::default(),
            source_previews: Default::default(),
            #[cfg(test)]
            spatial_reply: None,
            state,
            cache,
            initialization,
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
