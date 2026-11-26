//! Test-side operation authoring; the production file writer receives only the resulting node.
#![allow(dead_code)]
use antitone::photographic::graph::{display_output, original_image};
use antitone::{
    Error,
    acquisition::camera::camera_source,
    adjustment_layers::AdjustmentLayerSnapshot,
    dust_repair::Repair,
    evaluation::Value,
    export::{FileFormat, FloatPrecision, Overwrite, Plan, RenderTarget},
    geometry::Composition,
    photographic::stages::flat_field::GpuFlatField,
    photographic::stages::grading::Grading,
    processing::{InputSettings, RollSettings},
    rendering::OutputTarget,
    scan::{DevelopedSource, ImageSource},
};
use std::{path::Path, sync::Arc};

// Presets for tests that compare reopened outputs, not a production routing type.
#[derive(Clone, Copy, Debug)]
pub enum Format {
    Tiff(OutputTarget),
    Exr(FloatPrecision),
}

impl Format {
    pub fn plan(self) -> Plan {
        let (target, file) = match self {
            Self::Tiff(target) => (RenderTarget::Display(target), FileFormat::Tiff),
            Self::Exr(precision) => (RenderTarget::SceneLinearAp0, FileFormat::Exr(precision)),
        };
        Plan::new(target, file).unwrap()
    }
}

pub type Calculation = dyn Fn(Value) -> Result<Value, Error> + Send + Sync;

pub struct Request<'a> {
    pub source: &'a Arc<DevelopedSource>,
    pub reference_ev: Option<f64>,
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
        if self.source.area() != self.composition.area() {
            return Err(Error::Gpu("authored source/geometry mismatch".into()));
        }
        let program =
            ImageSource::new(self.source, self.input, self.reference_ev, None)?.gpu_program()?;
        let photograph = original_image(
            &device.limits(),
            camera_source(self.source)?,
            self.field,
            program,
            Grading::new(&device.limits(), self.roll, self.grade, self.source.area())?,
            self.repairs,
        )?;
        let photograph = match self.calculation {
            Some(compose) => compose(photograph)?,
            None => photograph,
        };
        let plan = self.format.plan();
        let definition = match plan.target() {
            RenderTarget::Display(target) => display_output(
                photograph,
                &self.luts,
                renderer.looks(),
                renderer.drt(target),
            )?,
            RenderTarget::SceneLinearAp0 => photograph,
        };
        Ok(antitone::gpu_export::Request {
            definition,
            composition: self.composition,
            plan,
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
