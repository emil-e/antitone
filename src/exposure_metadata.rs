//! Optional resolved camera-reference values, detached from accepted project/dirty state.
use crate::{
    exposure_reference::ReferenceExposure,
    project::{Project, SourceDevelopment, SourceId},
    source_revision::FileRevision,
};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[cfg(test)]
#[path = "exposure_metadata_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub source: SourceId,
    pub development: SourceDevelopment,
    pub revision: FileRevision,
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
                    && entry.development.validate().is_ok()
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
        if !self.valid() || project.input.is_negative() {
            return Self::default();
        }
        Self {
            version: VERSION,
            entries: self
                .entries
                .iter()
                .filter(|entry| {
                    project.order.contains(&entry.source)
                        && project
                            .source(entry.source)
                            .is_ok_and(|source| source.development == entry.development)
                })
                .cloned()
                .collect(),
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
