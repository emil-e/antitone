//! Admit the photographic stages and their numerical bounds before execution.

use super::assembly::{Component, definition};
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::SourceRegion,
    gpu::{grading::Grading, interpretation::InputTransform, parameters::ProcessingParameters},
    pixel_pipeline::PixelPipeline,
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
    let mut prepared = None;
    let mut bound = None;
    let steps = definition(&PixelPipeline::default());

    for step in steps {
        match step.component {
            Component::Camera => {
                InputTransform::new(input, matrix)?;
            }
            Component::Interpretation => {}
            Component::Working => {
                // Interpretation and working conversion execute before Roll exposure.
                // Admit that boundary independently of the legacy combined parameter block.
                ProcessingParameters::interpret(input, RollSettings::default(), matrix)?;
                let (parameters, peak) = ProcessingParameters::interpret_with_neutral_bound(
                    input,
                    roll,
                    matrix,
                    neutral_bound,
                )?;
                prepared = Some(parameters);
                bound = Some(peak);
            }
            Component::Repairs => {}
            Component::Roll => {
                Grading::validate(limits, roll, layers, area)?;
            }
            Component::Layers => {
                Grading::admit_layers(
                    layers,
                    bound.ok_or(Error::InvalidParameter("grading lacks an input bound"))?,
                )?;
            }
            Component::Effects(_) => {}
        }
    }
    prepared.ok_or(Error::InvalidParameter(
        "photographic definition lacks source preparation",
    ))
}
