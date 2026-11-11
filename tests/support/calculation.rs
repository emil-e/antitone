//! Authored operation arguments applied to a consumer's camera cells, not a second image recipe.
#![allow(dead_code)] // Integration executables exercise different subsets of these consumers.

use antitone::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    frame_raster::FrameRaster,
    geometry::{SourceRegion, TextureMapping},
    gpu::{
        evaluation::Value, grading::Grading, interpretation::InputTransform,
        parameters::ProcessingParameters,
    },
    gpu_scopes::{GpuScopes, ScopePaint},
    processing::{InputSettings, RollSettings},
    scopes::{Counts, Settings},
    viewer::{GpuImage, ImagePaint},
};
use glam::DMat3;
use std::sync::Arc;

pub type Calculation = Arc<dyn Fn(&GpuImage) -> Result<Value, Error> + Send + Sync>;

pub fn photograph(
    limits: &wgpu::Limits,
    input: impl Into<InputSettings>,
    roll: RollSettings,
    layers: AdjustmentLayerSnapshot,
    matrix: DMat3,
    area: SourceRegion,
) -> Result<Calculation, Error> {
    photograph_with_field(limits, input, roll, layers, matrix, area, None)
}

pub fn photograph_with_field(
    limits: &wgpu::Limits,
    input: impl Into<InputSettings>,
    roll: RollSettings,
    layers: AdjustmentLayerSnapshot,
    matrix: DMat3,
    area: SourceRegion,
    field: Option<Arc<antitone::gpu::flat::GpuFlatField>>,
) -> Result<Calculation, Error> {
    let program = InputTransform::new(input, matrix)?;
    let grading = Grading::new(limits, roll, layers, area)?;
    Ok(Arc::new(move |image| {
        image.photograph(program, grading.clone(), field.clone(), &Default::default())
    }))
}

pub fn inspection(parameters: ProcessingParameters) -> Calculation {
    inspection_with_field(parameters, None)
}

pub fn inspection_with_field(
    parameters: ProcessingParameters,
    field: Option<Arc<antitone::gpu::flat::GpuFlatField>>,
) -> Calculation {
    Arc::new(move |image| image.inspection(parameters, field.clone()))
}

pub trait ImageCalculations {
    fn prepare_authored(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        calculation: &Calculation,
    ) -> Result<bool, Error>;
    fn encode_authored(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        calculation: &Calculation,
    ) -> Result<(), Error>;
}

impl ImageCalculations for GpuImage {
    fn prepare_authored(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        calculation: &Calculation,
    ) -> Result<bool, Error> {
        self.prepare_calculation(device, encoder, &calculation(self)?, &Default::default())
    }
    fn encode_authored(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        calculation: &Calculation,
    ) -> Result<(), Error> {
        self.encode_calculation(device, encoder, &calculation(self)?, &Default::default())
    }
}

pub fn image_paint(
    image: Arc<GpuImage>,
    calculation: Option<Calculation>,
    uv: impl Into<TextureMapping>,
) -> Result<ImagePaint, Error> {
    let root = calculation
        .map(|calculation| calculation(&image))
        .transpose()?;
    ImagePaint::with_calculation(image, root, Default::default(), uv)
}

pub fn scope_paint(
    gpu: Arc<GpuScopes>,
    image: Arc<GpuImage>,
    calculation: Calculation,
    frame: FrameRaster,
    settings: Settings,
    context: egui::Context,
) -> Result<ScopePaint, Error> {
    let root = calculation(&image)?;
    ScopePaint::with_calculation(
        gpu,
        image,
        (root, Default::default()),
        frame,
        settings,
        context,
    )
}

pub trait ScopeCalculations {
    fn status_authored(
        &self,
        image: &Arc<GpuImage>,
        calculation: &Calculation,
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<Counts, String>>;
}

impl ScopeCalculations for GpuScopes {
    fn status_authored(
        &self,
        image: &Arc<GpuImage>,
        calculation: &Calculation,
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<Counts, String>> {
        self.status_calculation(
            image,
            (&calculation(image).ok()?, &Default::default()),
            frame,
            settings,
        )
    }
}
