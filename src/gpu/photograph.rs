//! Ordered photographic definition and component admission, independent of execution.

use super::{GpuInputProgram, Grading, ProcessingParameters};
use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    geometry::SourceRegion,
    pixel_pipeline::PixelPipeline,
    processing::{InputSettings, RollSettings},
};
use glam::{DMat3, DVec3};

/// Shared camera-component resources. Image assembly and common evaluation own execution.
#[derive(Clone)]
pub struct CameraResources {
    pub(super) identity: wgpu::Buffer,
    pub(super) empty_adjustment_layers: wgpu::Buffer,
    pub(super) empty_adjustment_layer_context: wgpu::Buffer,
}

impl CameraResources {
    pub fn new(device: &wgpu::Device) -> Result<Self, Error> {
        Ok(Self {
            identity: super::flat::identity_nodes(device),
            empty_adjustment_layers: super::grading::empty_buffer(device),
            empty_adjustment_layer_context: super::grading::context_buffer(device, 0, [0; 4])?,
        })
    }

    /// The exact module text the pipelines compile, exposed for device-free validation.
    pub(crate) fn module_shader() -> String {
        [
            Self::kernel_shader(),
            include_str!("../shaders/footprint.wgsl").to_owned(),
            include_str!("../shaders/repairs.wgsl").to_owned(),
        ]
        .join("\n")
    }

    /// One implementation serves the viewport, export and analysis: the near-base ratio response
    /// and the f32 colour operators.
    pub(crate) fn kernel_shader() -> String {
        let common = [
            include_str!("../shaders/camera_conversion.wgsl"),
            include_str!("../shaders/flat_field.wgsl"),
            include_str!("../shaders/film_response.wgsl"),
            include_str!("../shaders/film_inversion_export.wgsl"),
            include_str!("../shaders/photographic.wgsl"),
            include_str!("../shaders/adjustment_layers.wgsl"),
            include_str!("../shaders/photographic_colour.wgsl"),
            include_str!("../shaders/photographic_export.wgsl"),
        ]
        .join("\n");
        [common.as_str(), include_str!("../shaders/processing.wgsl")].join("\n")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    Codes,
    Camera,
    Native,
    Unrepaired,
    Neutral,
    Roll,
    Graded,
    Processed,
}

pub(crate) enum Component {
    Camera,
    Interpretation,
    Working,
    Repairs,
    Roll,
    Layers,
    Effects(PixelPipeline),
}

impl Component {
    pub(crate) fn camera_run(&self) -> bool {
        matches!(
            self,
            Self::Camera
                | Self::Interpretation
                | Self::Working
                | Self::Repairs
                | Self::Roll
                | Self::Layers
        ) || matches!(self, Self::Effects(effects) if effects.is_empty())
    }
}

pub(crate) struct Step {
    pub boundary: Option<Boundary>,
    pub component: Component,
}

pub(crate) fn definition(stages: &PixelPipeline) -> Vec<Step> {
    vec![
        Step {
            boundary: Some(Boundary::Camera),
            component: Component::Camera,
        },
        Step {
            boundary: Some(Boundary::Native),
            component: Component::Interpretation,
        },
        Step {
            boundary: Some(Boundary::Unrepaired),
            component: Component::Working,
        },
        Step {
            boundary: Some(Boundary::Neutral),
            component: Component::Repairs,
        },
        Step {
            boundary: Some(Boundary::Roll),
            component: Component::Roll,
        },
        Step {
            boundary: Some(Boundary::Graded),
            component: Component::Layers,
        },
        Step {
            boundary: Some(Boundary::Processed),
            component: Component::Effects(stages.clone()),
        },
    ]
}

pub(crate) fn prepare(
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
                GpuInputProgram::new(input, matrix)?;
            }
            Component::Interpretation => {}
            Component::Working => {
                // This component owns the canonical folded camera implementation, including
                // exposure before inversion's representability boundary.
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
