//! Recorded per-photo exposure baselines, separate from editable grading controls.
use crate::{
    exposure_reference::ReferenceExposure,
    project::{Project, SourceId},
};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[cfg(test)]
mod tests;

// Older entries include estimator fingerprints; a recorded baseline follows the photo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub source: SourceId,
    pub reference: ReferenceExposure,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub version: u32,
    pub entries: Vec<Entry>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            version: VERSION,
            entries: Vec::new(),
        }
    }
}

impl Metadata {
    pub fn valid(&self) -> bool {
        let mut ids = std::collections::HashSet::new();
        self.version == VERSION
            && self.entries.iter().all(|entry| {
                entry.source.0 != 0
                    && ids.insert(entry.source)
                    && crate::processing::exposure_multiplier(entry.reference.ev()).is_ok()
                    && match &entry.reference {
                        ReferenceExposure::Matched(value) => {
                            value.samples >= 128 && value.spread.is_finite() && value.spread >= 0.
                        }
                        ReferenceExposure::Unavailable(_) => true,
                    }
            })
    }

    pub fn for_project(&self, project: &Project) -> Self {
        if !self.valid() {
            return Self::default();
        }
        Self {
            version: VERSION,
            entries: self
                .entries
                .iter()
                .filter(|entry| project.source(entry.source).is_ok())
                .cloned()
                .collect(),
        }
    }
}

#[cfg(all(feature = "raw", any(feature = "editor", test)))]
type Slot = std::sync::Arc<std::sync::OnceLock<ReferenceExposure>>;

/// Document-owned baseline initialization, independent of optional pixel retention.
#[cfg(all(feature = "raw", any(feature = "editor", test)))]
#[derive(Default)]
pub(crate) struct Store {
    values: std::sync::Mutex<Vec<(SourceId, std::path::PathBuf, Slot)>>,
}

#[cfg(all(feature = "raw", any(feature = "editor", test)))]
impl Store {
    pub fn restored(
        metadata: &Metadata,
        origin: impl Fn(SourceId) -> Option<std::path::PathBuf>,
    ) -> Self {
        let store = Self::default();
        store.restore(metadata, origin);
        store
    }

    pub fn restore(
        &self,
        metadata: &Metadata,
        origin: impl Fn(SourceId) -> Option<std::path::PathBuf>,
    ) {
        if metadata.valid() {
            for entry in &metadata.entries {
                if let Some(origin) = origin(entry.source) {
                    let _ = self
                        .memo(entry.source, &origin)
                        .set(entry.reference.clone());
                }
            }
        }
    }

    // History can reuse a numeric SourceId for a different imported photo. The
    // locator separates those runtime slots; saved values still restore by photo ID.
    pub fn memo(&self, source: SourceId, origin: &std::path::Path) -> Slot {
        let mut values = self.values.lock().unwrap();
        if let Some((_, _, value)) = values
            .iter()
            .find(|(id, path, _)| *id == source && path == origin)
        {
            return value.clone();
        }
        let value = std::sync::Arc::new(std::sync::OnceLock::new());
        values.push((source, origin.to_owned(), value.clone()));
        value
    }

    pub fn get(&self, source: SourceId, origin: &std::path::Path) -> Option<ReferenceExposure> {
        self.values
            .lock()
            .unwrap()
            .iter()
            .find(|(id, path, _)| *id == source && path == origin)
            .and_then(|(_, _, value)| value.get().cloned())
    }

    pub fn snapshot(
        &self,
        project: &Project,
        origin: impl Fn(SourceId) -> Option<std::path::PathBuf>,
    ) -> Metadata {
        Metadata {
            entries: self
                .values
                .lock()
                .unwrap()
                .iter()
                .filter_map(|(id, path, value)| {
                    if origin(*id).as_ref() != Some(path) {
                        return None;
                    }
                    Some(Entry {
                        source: *id,
                        reference: value.get()?.clone(),
                    })
                })
                .filter(|entry| project.source(entry.source).is_ok())
                .collect(),
            ..Metadata::default()
        }
    }
}

pub(crate) fn read_cached<'de, D: serde::Deserializer<'de>>(
    reader: D,
) -> Result<Option<Metadata>, D::Error> {
    let value = serde_json::Value::deserialize(reader)?;
    Ok(serde_json::from_value::<Metadata>(value)
        .ok()
        .filter(Metadata::valid))
}

pub(crate) fn empty_or_invalid(value: &Option<Metadata>) -> bool {
    value
        .as_ref()
        .is_none_or(|metadata| metadata.entries.is_empty() || !metadata.valid())
}
