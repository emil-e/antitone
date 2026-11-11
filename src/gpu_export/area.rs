//! File-export adapter for codec-independent original-sample execution.
#[cfg(test)]
pub(super) use super::authored_area as write;
use super::*;
use crate::gpu::execution::{self, SampleEncoding};
use crate::gpu::submission::SubmissionGate;

#[allow(clippy::too_many_arguments)]
pub(super) fn write_calculation(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    definition: crate::gpu::evaluation::Value,
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
mod repair_read_tests;
#[cfg(all(test, target_os = "macos", feature = "ocio-reference"))]
mod stage_oracle_tests;
