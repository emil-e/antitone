//! File-export adapter for codec-independent original-sample execution.
#[cfg(test)]
pub(super) use super::authored_area as write;
use super::*;
use crate::gpu::execution::{self, SampleEncoding};

pub(super) fn write_calculation(
    context: &evaluation::Context,
    gpu: &crate::gpu::submission::Queue,
    request: Request,
    destination: (&Path, Overwrite),
    check: impl FnMut() -> Result<(), Error>,
) -> Result<execution::Stats, Error> {
    let Request {
        definition,
        composition,
        format,
    } = request;
    let encoding = match format {
        Format::Tiff(target) => SampleEncoding::Rgb16(target),
        Format::Exr(_) => SampleEncoding::LinearFloat,
    };
    execution::execute(
        context,
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
