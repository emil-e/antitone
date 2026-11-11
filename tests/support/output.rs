//! Test-side operation authoring; the production file writer receives only the resulting node.
#![allow(dead_code)]
pub use antitone::gpu_export::Format;
use antitone::{
    Error,
    adjustment_layers::AdjustmentLayerSnapshot,
    dust_repair::Repair,
    export::Overwrite,
    geometry::Composition,
    gpu::{
        evaluation::{self, Reference, Value},
        flat::GpuFlatField,
        grading::Grading,
        photograph::PhotographGraph,
        submission::SubmissionGate,
    },
    pixel_pipeline::PixelPipeline,
    processing::{InputSettings, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputReference,
    scan::{ImageSource, Scan},
};
use std::{path::Path, sync::Arc};

pub struct Request<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub input: InputSettings,
    pub roll: RollSettings,
    pub grade: AdjustmentLayerSnapshot,
    pub repairs: &'a [Repair],
    pub composition: Composition,
    pub format: Format,
    pub luts: antitone::looks::LutChain,
    pub pipeline: PixelPipeline,
}
impl Request<'_> {
    pub fn calculation(
        self,
        renderer: &GpuRenderer,
        device: &wgpu::Device,
    ) -> Result<antitone::gpu_export::Request, Error> {
        if self.scan.area != self.composition.area()
            || self.scan.flat_field() != self.field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu("authored source/geometry mismatch".into()));
        }
        let photograph = PhotographGraph::original_image(
            &device.limits(),
            evaluation::camera_source(self.scan)?,
            self.field,
            ImageSource::new(self.scan, self.input)?.gpu_program()?,
            Grading::new(&device.limits(), self.roll, self.grade, self.scan.area)?,
            self.repairs,
            &self.pipeline,
        )?;
        let reference = match self.format {
            Format::Tiff(target) => OutputReference::DisplayReferred(target),
            Format::Exr(_) => OutputReference::SceneReferred,
        };
        let definition: Value = Reference::capture(device, renderer, reference, &self.luts)?
            .compose(photograph, renderer)?;
        Ok(antitone::gpu_export::Request {
            definition,
            composition: self.composition,
            format: self.format,
        })
    }
}

pub fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
) -> Result<(), Error> {
    antitone::gpu_export::write(
        renderer,
        device,
        gpu,
        request.calculation(renderer, device)?,
        path,
        overwrite,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn write_checked(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
    check: impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    antitone::gpu_export::write_checked(
        renderer,
        device,
        gpu,
        request.calculation(renderer, device)?,
        path,
        overwrite,
        check,
    )
}
