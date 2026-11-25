//! Render an authored original-sample calculation and write the prepared pixels.
use crate::export_timing::timed;
#[cfg(test)]
use crate::frame_raster::FrameRaster;
use crate::{
    Error,
    export::{
        self, ImageSize, Layout, Overwrite, Plan, RenderTarget, Samples, SceneEncoding, pack_rgb8,
    },
    geometry::Composition,
};
#[cfg(test)]
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    processing::{InputSettings, RollSettings},
    render_fixture::RenderFixture,
    scan::Scan,
};
#[cfg(test)]
use crate::{export::ExrPrecision, rendering::OutputTarget};
#[cfg(test)]
use authored::Format;
use glam::{DVec3, Vec3};
use std::{path::Path, sync::Arc};

mod area;
#[cfg(test)]
#[path = "../tests/support/output.rs"]
pub(crate) mod authored;
#[cfg(feature = "project")]
pub(crate) mod job;
#[cfg(all(test, target_os = "macos", feature = "editor"))]
mod native_resources;
use crate::{evaluation, rendering::execution::OutputBlock, rendering::rgb16};

#[cfg(all(test, target_os = "macos"))]
pub(crate) mod observation_clock;
#[cfg(all(test, target_os = "macos", feature = "editor"))]
mod pipeline_tests;
#[cfg(all(test, target_os = "macos"))]
mod production_tests;
#[cfg(all(test, target_os = "macos", feature = "editor"))]
mod resource_tests;
#[cfg(test)]
pub(crate) mod tests;

/// The authored calculation must produce the linear RGB of the plan's render target:
/// target-linear for display output, or working AP1 for scene-linear output.
pub struct Request {
    pub definition: evaluation::Value,
    pub composition: Composition,
    pub plan: Plan,
}

/// Render, reconstruct and encode an already captured calculation.
pub fn write(
    context: &evaluation::Context,
    gpu: &crate::gpu::submission::Queue,
    request: Request,
    path: &Path,
    overwrite: Overwrite,
) -> Result<(), Error> {
    write_checked(context, gpu, request, path, overwrite, || Ok(()))
}

/// Execute with cooperative stop checks before bounded work and destination publication.
pub fn write_checked(
    context: &evaluation::Context,
    gpu: &crate::gpu::submission::Queue,
    request: Request,
    path: &Path,
    overwrite: Overwrite,
    check: impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    area::write_calculation(context, gpu, request, (path, overwrite), check).map(|_| ())
}

#[cfg(test)]
fn authored_area(
    renderer: &RenderFixture,
    device: &wgpu::Device,
    gpu: &crate::gpu::submission::Queue,
    request: authored::Request<'_>,
    destination: (&Path, Overwrite),
    check: impl FnMut() -> Result<(), Error>,
) -> Result<crate::rendering::execution::Stats, Error> {
    let request = request.calculation(renderer, device)?;
    area::write_calculation(renderer.evaluation(), gpu, request, destination, check)
}

const TIFF_BLOCK: [u32; 2] = [512; 2];

fn block_layout(composition: Composition, plan: Plan) -> (export::BlockOrder, [u32; 2]) {
    use export::BlockOrder::{RowMajor, Spatial2x2};

    match plan.format().layout() {
        Layout::Rgb16 => {
            let order = if composition.is_exact() {
                RowMajor
            } else {
                Spatial2x2
            };
            (order, TIFF_BLOCK)
        }
        Layout::Rgb8 | Layout::Float(_) => (RowMajor, export::BLOCK),
    }
}

fn encode_with_checked(
    composition: Composition,
    plan: Plan,
    destination: (&Path, Overwrite),
    mut build: impl FnMut([u32; 2], [u32; 2]) -> Result<OutputBlock, Error>,
    before_publish: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    let [width, height] = composition.size();
    let layout = plan.format().layout();
    let scene = match layout {
        Layout::Float(precision) => Some(SceneEncoding::new(precision)),
        Layout::Rgb8 | Layout::Rgb16 => None,
    };
    plan.write(
        destination,
        ImageSize { width, height },
        block_layout(composition, plan),
        |origin, extent| {
            let block = build(origin, extent)?;
            match (plan.target(), layout, block) {
                (
                    RenderTarget::Display(target),
                    Layout::Rgb16,
                    OutputBlock::Rgb16 {
                        target: encoded_target,
                        words,
                    },
                ) if encoded_target == target => {
                    timed!(WorkingPack, rgb16::decode(&words)).map(Samples::Rgb16)
                }
                (RenderTarget::SceneLinearAp0, Layout::Float(_), OutputBlock::Linear(pixels)) => {
                    let scene = scene.as_ref().expect("scene encoding");
                    timed!(WorkingPack, scene.pack(pixels.iter().map(linear_readback)))
                }
                (RenderTarget::Display(target), Layout::Rgb8, OutputBlock::Linear(pixels)) => {
                    timed!(
                        TargetEncode,
                        pack_rgb8(target, pixels.iter().map(linear_readback))
                    )
                    .map(Samples::Rgb8)
                }
                #[cfg(test)]
                (RenderTarget::Display(target), Layout::Rgb16, OutputBlock::Linear(pixels)) => {
                    encode_readback(target, pixels).map(Samples::Rgb16)
                }
                _ => Err(Error::Gpu("unexpected output representation".into())),
            }
        },
        before_publish,
    )
}

fn readback_rgb(pixel: &[f32; 4]) -> Result<Vec3, Error> {
    if pixel[3] < 0. || pixel.iter().any(|v| !v.is_finite()) {
        return Err(Error::Gpu(format!(
            "export reconstruction invalid contributing input/support marker={}",
            pixel[3]
        )));
    }
    Ok(Vec3::new(pixel[0], pixel[1], pixel[2]))
}

#[cfg(test)]
fn encode_readback(target: OutputTarget, pixels: Vec<[f32; 4]>) -> Result<Vec<[u16; 3]>, Error> {
    // Give invalid GPU contribution markers diagnostic priority over target-range failures.
    timed!(WorkingPack, {
        for pixel in &pixels {
            readback_rgb(pixel)?;
        }
        Ok::<_, Error>(())
    })?;
    timed!(
        TargetEncode,
        target.encode_u16_pixels(pixels.into_iter().map(|p| Vec3::new(p[0], p[1], p[2])))
    )
}

fn linear_readback(pixel: &[f32; 4]) -> Result<DVec3, Error> {
    readback_rgb(pixel).map(|rgb| rgb.as_dvec3())
}

#[cfg(all(test, target_os = "macos"))]
mod packing_tests;
#[cfg(all(test, target_os = "macos"))]
mod rgb16_publication_tests;
