use super::*;
use crate::flat_field::FlatField;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatFieldSettings {
    pub enabled: bool,
    pub reference: SourceId,
    pub map: Arc<FlatField>,
}

impl FlatFieldSettings {
    pub fn new(reference: &Source, map: Arc<FlatField>) -> Self {
        Self {
            enabled: true,
            reference: reference.id,
            map,
        }
    }

    pub(super) fn validate(&self, project: &Project) -> Result<()> {
        self.map.validate()?;
        project.source(self.reference)?;
        Ok(())
    }
}
