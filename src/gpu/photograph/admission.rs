//! Validate captured photographic controls and resource layouts before execution.

use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::SourceRegion,
    gpu::{grading::Grading, interpretation::InputTransform, parameters::ProcessingParameters},
    processing::{InputSettings, RollSettings},
};
use glam::DMat3;

pub fn prepare(
    limits: &wgpu::Limits,
    input: InputSettings,
    roll: RollSettings,
    layers: &AdjustmentLayerSnapshot,
    matrix: DMat3,
    area: SourceRegion,
) -> Result<ProcessingParameters, Error> {
    let area = area.validate()?;
    InputTransform::new(input, matrix)?;
    ProcessingParameters::interpret(input, RollSettings::default(), matrix)?;
    Grading::validate(limits, roll, layers, area)?;
    ProcessingParameters::interpret(input, roll, matrix)
}

#[cfg(feature = "project")]
impl crate::image_recipe::ImageRecipe {
    pub(crate) fn validate(
        &self,
        limits: &wgpu::Limits,
    ) -> Result<crate::gpu::parameters::ProcessingParameters, Error> {
        let program = InputTransform::from_recipe(&self.neutral)?;
        prepare(
            limits,
            program.input,
            self.roll,
            &self.grade,
            program.matrix,
            self.neutral.source_development.active_area().into(),
        )
    }
}
