//! File-export adapter for codec-independent original-sample execution.
use super::*;
use crate::gpu::submission::SubmissionGate;
use crate::{
    gpu::execution::{self, SampleEncoding},
    rendering::OutputReference,
};

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
    let output = match format {
        Format::Tiff(target) => OutputReference::DisplayReferred(target),
        Format::Exr(_) => OutputReference::SceneReferred,
    };

    if request.scan.area != composition.area()
        || request.scan.flat_field() != request.field.as_ref().map(|field| field.field())
    {
        return Err(Error::Gpu("processing source/recipe mismatch".into()));
    }
    let photograph = crate::gpu::photograph::PhotographGraph::original(
        &device.limits(),
        crate::gpu::evaluation::camera_source(request.scan)?,
        request.field,
        crate::gpu::interpretation::InputTransform::for_scan(request.scan, request.input)?,
        crate::gpu::grading::Grading::new(
            &device.limits(),
            request.roll,
            request.grade,
            request.scan.area,
        )?,
        crate::gpu::repairs::Snapshot::logical(request.repairs.to_vec())?,
        &request.pipeline,
    )?;
    let definition =
        crate::gpu::evaluation::Reference::capture(device, renderer, output, &request.luts)?
            .compose(photograph.processed, renderer)?;
    write_calculation(
        renderer,
        device,
        gpu,
        definition,
        composition,
        format,
        destination,
        check,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn write_calculation(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    definition: Arc<crate::gpu::evaluation::ImageNode>,
    composition: Composition,
    format: Format,
    destination: (&Path, Overwrite),
    check: impl FnMut() -> Result<(), Error>,
) -> Result<execution::Stats, Error> {
    let encoding = match format {
        Format::Tiff(target) => SampleEncoding::Rgb16(target),
        Format::Exr(_) => SampleEncoding::LinearFloat,
    };
    execution::execute(
        renderer,
        device,
        gpu,
        execution::Request {
            definition,
            composition,
            encoding,
        },
        |render, complete| encode_with_checked(composition, format, destination, render, complete),
        check,
    )
}

#[cfg(all(test, target_os = "macos"))]
mod look_lifetime_tests;
#[cfg(all(test, target_os = "macos"))]
mod repair_upload_tests;
#[cfg(all(test, target_os = "macos", feature = "ocio-reference"))]
mod stage_oracle_tests;
