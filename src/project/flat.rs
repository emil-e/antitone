use super::*;
use crate::flat_field::FlatField;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatFieldSettings {
    pub enabled: bool,
    /// Descriptive provenance, independent of the project's photographs.
    pub reference_name: String,
    pub map: Arc<FlatField>,
}

impl FlatFieldSettings {
    pub fn new(reference: impl AsRef<Path>, map: Arc<FlatField>) -> Self {
        Self {
            enabled: true,
            reference_name: reference
                .as_ref()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            map,
        }
    }

    pub(super) fn validate(&self) -> Result<()> {
        self.map.validate()
    }
}
