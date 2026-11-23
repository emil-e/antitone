//! File-export adapter for codec-independent original-sample execution.
#[cfg(test)]
pub(super) use super::authored_area as write;
use super::*;
use crate::gpu::execution::{self, SampleEncoding};

pub(super) fn sample_encoding(plan: Plan) -> SampleEncoding {
    match (plan.target(), plan.format().layout()) {
        (RenderTarget::Display(target), Layout::Rgb16) => SampleEncoding::Rgb16(target),
        (RenderTarget::Display(_), Layout::Rgb8)
        | (RenderTarget::SceneLinearAp0, Layout::Float(_)) => SampleEncoding::LinearFloat,
        _ => unreachable!("the export plan admits only supported target and layout pairs"),
    }
}

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
        plan,
    } = request;
    let layout = block_layout(composition, plan);
    execution::execute_blocks(
        context,
        gpu,
        execution::Request {
            definition,
            composition,
            encoding: sample_encoding(plan),
        },
        layout.1,
        |render, complete| encode_with_checked(composition, plan, destination, render, complete),
        check,
    )
}

#[cfg(all(test, target_os = "macos"))]
mod look_lifetime_tests;
#[cfg(all(test, target_os = "macos"))]
mod repair_read_tests;
#[cfg(all(test, target_os = "macos", feature = "ocio-reference"))]
mod stage_oracle_tests;
