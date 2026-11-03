//! Document-owned original sampling inputs, independent of saved regions and full-image retention.
use super::{Document, Error, SourceId};
use crate::{scan::AnalysisInput, source::SourceKey};
use std::sync::Arc;

#[derive(Clone)]
struct Entry {
    source: SourceId,
    key: SourceKey,
    input: Arc<AnalysisInput>,
}

#[derive(Clone, Default)]
pub(super) struct Inputs {
    entries: Arc<Vec<Entry>>,
}

fn key(document: &Document, source: SourceId) -> Result<SourceKey, Error> {
    let record = document.project.source(source)?;
    let path = document.source_path(source)?;
    Ok(SourceKey::for_load(
        &path,
        document.file_key(&path),
        &record.development,
    ))
}

impl Inputs {
    pub fn get(
        &self,
        document: &Document,
        source: SourceId,
    ) -> Result<Option<Arc<AnalysisInput>>, Error> {
        let key = key(document, source)?;
        Ok(self
            .entries
            .iter()
            .find(|e| e.source == source && e.key == key)
            .map(|e| e.input.clone()))
    }

    pub fn insert(
        &mut self,
        document: &Document,
        source: SourceId,
        input: Arc<AnalysisInput>,
    ) -> Result<(), Error> {
        let key = key(document, source)?;
        let entries = Arc::make_mut(&mut self.entries);
        entries.retain(|e| e.source != source);
        entries.push(Entry { source, key, input });
        Ok(())
    }

    pub fn retain(&mut self, document: &Document) {
        let matches = |e: &Entry| {
            document
                .project
                .source(e.source)
                .is_ok_and(|source| !source.reference_only)
                && key(document, e.source).is_ok_and(|key| key == e.key)
        };
        if self.entries.iter().all(&matches) {
            return;
        }
        Arc::make_mut(&mut self.entries).retain(matches);
    }

    /// Owned patch payloads, once per allocation; lookup/Arc/allocator overhead is excluded.
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
mod tests;
