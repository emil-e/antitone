//! Admit the photographic stages and their numerical bounds before execution.

use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::SourceRegion,
    gpu::{grading::Grading, interpretation::InputTransform, parameters::ProcessingParameters},
    processing::{InputSettings, RollSettings},
};
use glam::{DMat3, DVec3};

pub fn prepare(
    limits: &wgpu::Limits,
    input: InputSettings,
    roll: RollSettings,
    layers: &AdjustmentLayerSnapshot,
    matrix: DMat3,
    area: SourceRegion,
    neutral_bound: DVec3,
) -> Result<ProcessingParameters, Error> {
    if !neutral_bound.is_finite() || neutral_bound.min_element() < 0. {
        return Err(Error::Gpu("invalid neutral repair bound".into()));
    }
    let area = area.validate()?;
    InputTransform::new(input, matrix)?;
    // Interpretation and camera conversion precede Roll exposure, so admit the
    // ungraded boundary as well as the combined bound used by downstream grading.
    ProcessingParameters::interpret(input, RollSettings::default(), matrix)?;
    let (parameters, bound) =
        ProcessingParameters::interpret_with_neutral_bound(input, roll, matrix, neutral_bound)?;
    Grading::validate(limits, roll, layers, area)?;
    Grading::admit_layers(layers, bound)?;
    Ok(parameters)
}
