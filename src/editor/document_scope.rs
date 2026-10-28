use super::{Ready, document::Document, history::History, photo_scope::PhotoScope};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod spatial_tests;

/// Accepted document authority, cross-photo tool context and the dependent visit.
/// Captured documents are data snapshots, not histories or continuation owners.
pub(super) struct DocumentScope {
    pub thumbnails: super::Thumbnails,
    pub grades: super::Grades,
    pub input: super::InputSettings,
    pub input_switch: Option<super::InputSettings>,
    pub roll: super::RollSettings,
    pub film_emulation: super::FilmEmulationSettings,
    pub roll_orientation: super::RollOrientation,
    pub auto_crop_requested: Option<super::auto_crop::Target>,
    pub target: super::OutputTarget,
    pub source: bool,
    pub picking: super::picking::Session,
    pub selected: usize,
    pub roll_selection: super::RollSelection,
    pub removal_pending: Option<Vec<super::SourceId>>,
    pub export_operation: super::export_workflow::State,
    pub export_dialog: Option<super::export_batch::Draft>,
    pub export_summary: Option<super::export_batch::Summary>,
    pub correction: Option<Arc<super::Correction>>,
    pub flat_edit: Option<super::FlatEdit>,
    pub state: Document,
    pub cache: Arc<crate::source::SourceCache>,
    pub exposures: Arc<crate::exposure_metadata::Store>,
    pub spatial: crate::spatial_metadata::Store,
    pub spatial_preparing: Option<super::spatial::Pending>,
    pub spatial_consumer: Option<super::spatial::Consumer>,
    pub history: History,
    pub visit: PhotoScope,
    pub embedded_preview: Option<super::thumbnails::EmbeddedWork>,
    pub thumbnail_work: super::thumbnails::ThumbnailWork,
    pub write: super::save::State,
    pub look: Option<super::look_ui::Pending>,
    pub dust: super::dust::State,
    pub auto_crop_edit: Option<super::auto_crop::Draft>,
    pub analysis_preparing: Option<super::analysis_workflow::Pending>,
    pub analysis_pending: Option<super::analysis_workflow::Pending>,
    pub analysis_review: Option<super::analysis_review::Review>,
    pub analysis_generation: Arc<std::sync::atomic::AtomicU64>,
}

impl DocumentScope {
    pub fn replacement(&self, state: Document) -> Self {
        let mut next: Self = state.into();
        next.thumbnails = self.thumbnails.for_document(next.cache.clone());
        next.grades = self.grades.for_document();
        next.visit.working_view = self.visit.working_view.for_visit();
        next
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

    pub fn install(&mut self, document: Document) {
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
        crate::scan::positive_matrix(
            &input,
            glam::DMat3::from_cols_array_2d(&source.development.camera_to_ap1),
            self.exposure_value(source).as_ref(),
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
        match (self.visit.ready.as_ref(), ready) {
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
        let input = state.project.input;
        let cache = Arc::new(crate::source::SourceCache::new(0));
        let exposures = Arc::new(crate::exposure_metadata::Store::restored(
            &state.exposure_metadata,
            |id| state.exposure_origin(id),
        ));
        Self {
            thumbnails: super::Thumbnails::for_sources(cache.clone()),
            grades: super::Grades::default(),
            correction: None,
            flat_edit: None,
            selected: 0,
            roll_selection: super::RollSelection::default(),
            removal_pending: None,
            export_operation: Default::default(),
            export_dialog: None,
            export_summary: None,
            picking: Default::default(),
            input,
            input_switch: None,
            roll: super::RollSettings::default(),
            film_emulation: super::FilmEmulationSettings::default(),
            roll_orientation: super::RollOrientation::default(),
            target: super::OutputTarget::DisplayP3,
            source: true,
            auto_crop_requested: None,
            spatial: crate::spatial_metadata::Store::restored((*state.spatial_metadata).clone()),
            spatial_preparing: None,
            spatial_consumer: None,
            state,
            cache,
            exposures,
            history: History::default(),
            visit: PhotoScope::default(),
            embedded_preview: None,
            thumbnail_work: Default::default(),
            write: super::save::State::default(),
            look: None,
            dust: super::dust::State::default(),
            auto_crop_edit: None,
            analysis_preparing: None,
            analysis_pending: None,
            analysis_review: None,
            analysis_generation: Arc::default(),
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
