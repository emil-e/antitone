use super::*;
use crate::flat_field::{CONTRACT, FlatField};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatFieldSettings {
    pub contract: String,
    pub enabled: bool,
    pub reference: SourceId,
    pub development: SourceDevelopment,
    pub map: Arc<FlatField>,
}

impl FlatFieldSettings {
    pub fn new(reference: &Source, map: Arc<FlatField>) -> Self {
        Self {
            contract: CONTRACT.into(),
            enabled: true,
            reference: reference.id,
            development: reference.development.clone(),
            map,
        }
    }

    pub fn require_source(&self, development: &SourceDevelopment) -> Result<()> {
        require(
            development.same_processing(&self.development),
            "flat-field source dimensions/development differ from the reference",
        )
    }

    pub(super) fn require_photos(&self, project: &Project) -> Result<()> {
        for id in &project.order {
            self.require_source(&project.source(*id)?.development)?;
        }
        Ok(())
    }

    pub(super) fn validate(&self, project: &Project) -> Result<()> {
        require(self.contract == CONTRACT, "unsupported flat-field contract")?;
        self.map.validate()?;
        self.development.validate()?;
        let reference = project.source(self.reference)?;
        require(
            reference.development.same_processing(&self.development),
            "flat-field reference identity differs from its accepted map",
        )?;
        require(
            contains(self.development.active_area, self.map.area()),
            "flat-field support is outside the reference active area",
        )?;
        if self.enabled && project.input.capabilities().flat_field {
            self.require_photos(project)?;
            if let Some(negative) = project.input.negative_input() {
                let calibration = self
                    .development
                    .processing_calibration(negative.calibration, Some(&self.map))?;
                crate::processing::NegativeInput {
                    calibration,
                    ..negative
                }
                .parameters()?;
            }
        }
        Ok(())
    }
}
