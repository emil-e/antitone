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
    pub state: Document,
    pub cache: Arc<crate::source::SourceCache>,
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
    pub pick_return: Option<super::PickReturn>,
    pub pick_view: Option<super::PickView>,
    pub analysis_generation: Arc<std::sync::atomic::AtomicU64>,
}

impl DocumentScope {
    pub fn capture(&self) -> Document {
        let mut document = self.state.clone();
        document.spatial_metadata = self.spatial.ready();
        document
    }

    pub fn accept_save(&mut self, saved: Document) {
        self.history
            .rebase(self.state.path.as_deref(), saved.path.as_deref());
        self.state.accept_save(saved);
    }

    pub fn install(&mut self, document: Document) {
        self.state = document;
    }

    pub fn cache_ready(&self, ready: &Ready) {
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
        Self {
            spatial: crate::spatial_metadata::Store::restored((*state.spatial_metadata).clone()),
            spatial_preparing: None,
            spatial_consumer: None,
            state,
            cache: Arc::new(crate::source::SourceCache::new(0)),
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
            pick_return: None,
            pick_view: None,
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

impl Drop for DocumentScope {
    fn drop(&mut self) {
        self.spatial.close();
        drop(self.spatial_preparing.take());
        drop(self.spatial_consumer.take());
        drop(self.look.take());
        drop(self.embedded_preview.take());
        drop(std::mem::take(&mut self.thumbnail_work));
        drop(std::mem::take(&mut self.dust));
        drop(self.analysis_preparing.take());
        drop(self.analysis_pending.take());
        drop(self.analysis_review.take());
        drop(std::mem::take(&mut self.visit));
        drop(std::mem::take(&mut self.write));
        self.cache.close();
    }
}
