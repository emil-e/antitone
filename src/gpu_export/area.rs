//! File-export adapter for codec-independent original-sample execution.
use super::*;
use crate::{
    gpu::execution::{self, SampleEncoding, WorkBudget},
    rendering::OutputReference,
};

#[cfg(all(test, target_os = "macos"))]
use crate::gpu::execution::Unit;

pub(super) fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    destination: (&Path, Overwrite),
    check: impl FnMut() -> Result<(), Error>,
) -> Result<execution::Stats, Error> {
    let composition = request.composition;
    let format = request.format;
    let (output, encoding) = match format {
        Format::Tiff(target) => (
            OutputReference::DisplayReferred(target),
            SampleEncoding::Rgb16,
        ),
        Format::Exr(_) => (OutputReference::SceneReferred, SampleEncoding::LinearFloat),
    };
    let stages = crate::pixel_pipeline::PixelPipeline::film(
        request.film_emulation,
        request.composition.crop().size,
        request.film_seed,
    )?;
    execution::execute(
        renderer,
        device,
        gpu,
        execution::Request {
            scan: request.scan,
            field: request.field,
            input: request.input,
            roll: request.roll,
            grade: request.grade,
            repairs: request.repairs,
            composition,
            stages,
            output,
            look: request.look,
            encoding,
            budget: WorkBudget::for_device(device),
        },
        |render, complete| encode_with_checked(composition, format, destination, render, complete),
        check,
    )
}

#[cfg(all(test, target_os = "macos"))]
mod look_lifetime_tests;
#[cfg(all(test, target_os = "macos"))]
mod repair_payload_tests;
#[cfg(all(test, target_os = "macos", feature = "ocio-reference"))]
mod stage_oracle_tests;
