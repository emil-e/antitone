use super::*;
use crate::flat_field::{CONTRACT, FlatField};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlatFieldSettings {
    pub contract: String,
    pub enabled: bool,
    pub reference: SourceId,
    pub fingerprint: Fingerprint,
    pub development: SourceDevelopment,
    pub roll_development: Development,
    pub map: Arc<FlatField>,
}
impl FlatFieldSettings {
    pub fn new(reference: &Source, map: Arc<FlatField>, roll_development: &Development) -> Self {
        Self {
            contract: CONTRACT.into(),
            enabled: true,
            reference: reference.id,
            fingerprint: reference.fingerprint.clone(),
            development: reference.development.clone(),
            roll_development: roll_development.clone(),
            map,
        }
    }
    pub fn require_source(&self, development: &SourceDevelopment) -> Result<()> {
        require(
            *development == self.development,
            "flat-field source dimensions/development differ from the reference",
        )
    }
    pub(super) fn validate(&self, project: &Project) -> Result<()> {
        require(self.contract == CONTRACT, "unsupported flat-field contract")?;
        self.map.validate()?;
        self.roll_development.validate()?;
        require(
            project.development.as_ref() == Some(&self.roll_development),
            "flat-field roll development or WB differs",
        )?;
        self.development.validate()?;
        self.fingerprint.validate()?;
        let reference = project.source(self.reference)?;
        require(
            reference.fingerprint == self.fingerprint && reference.development == self.development,
            "flat-field reference identity differs from its accepted map",
        )?;
        require(
            contains(self.development.active_area, self.map.area()),
            "flat-field support is outside the reference active area",
        )?;
        require(
            project.development.is_some(),
            "flat-field requires RAW development identity",
        )?;
        if self.enabled {
            for id in &project.order {
                self.require_source(&project.source(*id)?.development)?;
            }
            if let Some(calibration) = project.calibration()? {
                let matrix = DMat3::from_cols_array_2d(&self.development.camera_to_ap0);
                Inversion::new(
                    self.map.processing_calibration(matrix, calibration)?,
                    project.roll,
                )?;
            }
        }
        Ok(())
    }
}
