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
        evaluation::Value,
        flat::GpuFlatField,
        grading::Grading,
        photograph::{PhotographGraph, display_output},
        source::camera_source,
    },
    processing::{InputSettings, RollSettings},
    scan::{ImageSource, Scan},
};
use std::{path::Path, sync::Arc};

pub type Calculation = dyn Fn(Value) -> Result<Value, Error> + Send + Sync;

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
    pub calculation: Option<Arc<Calculation>>,
}
impl Request<'_> {
    pub fn calculation(
        self,
        renderer: &crate::render_fixture::RenderFixture,
        device: &wgpu::Device,
    ) -> Result<antitone::gpu_export::Request, Error> {
        if self.scan.area != self.composition.area()
            || self.scan.flat_field() != self.field.as_ref().map(|field| field.field())
        {
            return Err(Error::Gpu("authored source/geometry mismatch".into()));
        }
        let photograph = PhotographGraph::original_image(
            &device.limits(),
            camera_source(self.scan)?,
            self.field,
            ImageSource::new(self.scan, self.input)?.gpu_program()?,
            Grading::new(&device.limits(), self.roll, self.grade, self.scan.area)?,
            self.repairs,
        )?;
        let photograph = match self.calculation {
            Some(compose) => compose(photograph)?,
            None => photograph,
        };
        let definition = match self.format {
            Format::Tiff(target) => display_output(
                photograph,
                &self.luts,
                renderer.looks(),
                renderer.drt(target),
            )?,
            Format::Exr(_) => photograph,
        };
        Ok(antitone::gpu_export::Request {
            definition,
            composition: self.composition,
            format: self.format,
        })
    }
}

pub fn write(
    renderer: &crate::render_fixture::RenderFixture,
    device: &wgpu::Device,
    gpu: &antitone::gpu::submission::Queue,
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
) -> Result<(), Error> {
    antitone::gpu_export::write(
        renderer.evaluation(),
        gpu,
        request.calculation(renderer, device)?,
        path,
        overwrite,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn write_checked(
    renderer: &crate::render_fixture::RenderFixture,
    device: &wgpu::Device,
    gpu: &antitone::gpu::submission::Queue,
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
    check: impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    antitone::gpu_export::write_checked(
        renderer.evaluation(),
        gpu,
        request.calculation(renderer, device)?,
        path,
        overwrite,
        check,
    )
}
