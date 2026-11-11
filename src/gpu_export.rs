//! One bounded original-sample engine for TIFF and scene-linear EXR.
use crate::export_timing::timed;
#[cfg(test)]
use crate::frame_raster::FrameRaster;
use crate::{
    Error,
    export::{self, ExrPrecision, ImageSize, Overwrite},
    geometry::Composition,
    gpu::submission::SubmissionGate,
    renderer::GpuRenderer,
    rendering::OutputTarget,
};
#[cfg(test)]
use crate::{
    adjustment_layers::AdjustmentLayerSnapshot,
    processing::{InputSettings, RollSettings},
    scan::Scan,
};
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
use crate::gpu::{execution::OutputBlock, rgb16};

#[cfg(all(test, target_os = "macos", feature = "ocio-reference"))]
mod common_exr_tests;
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

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Tiff(OutputTarget),
    Exr(ExrPrecision),
}

/// An authored original-cell calculation in its chosen linear output representation.
pub struct Request {
    pub definition: crate::gpu::evaluation::Value,
    pub composition: Composition,
    pub format: Format,
}

/// Render, reconstruct and encode an already captured calculation.
pub fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: Request,
    path: &Path,
    overwrite: Overwrite,
) -> Result<(), Error> {
    write_checked(renderer, device, gpu, request, path, overwrite, || Ok(()))
}

/// Execute with cooperative stop checks before bounded work and destination publication.
#[allow(clippy::too_many_arguments)]
pub fn write_checked(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: Request,
    path: &Path,
    overwrite: Overwrite,
    check: impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    area::write_calculation(
        renderer,
        device,
        gpu,
        request.definition,
        request.composition,
        request.format,
        (path, overwrite),
        check,
    )
    .map(|_| ())
}

#[cfg(test)]
fn authored_area(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: authored::Request<'_>,
    destination: (&Path, Overwrite),
    check: impl FnMut() -> Result<(), Error>,
) -> Result<crate::gpu::execution::Stats, Error> {
    let request = request.calculation(renderer, device)?;
    area::write_calculation(
        renderer,
        device,
        gpu,
        request.definition,
        request.composition,
        request.format,
        destination,
        check,
    )
}

fn default_order(composition: Composition) -> export::BlockOrder {
    if composition.is_exact() {
        export::BlockOrder::RowMajor
    } else {
        export::BlockOrder::Spatial2x2
    }
}

fn encode_with_checked(
    composition: Composition,
    format: Format,
    destination: (&Path, Overwrite),
    build: impl FnMut([u32; 2], [u32; 2]) -> Result<OutputBlock, Error>,
    before_publish: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    encode_ordered_checked(
        composition,
        format,
        destination,
        default_order(composition),
        build,
        before_publish,
    )
}

fn encode_ordered_checked(
    composition: Composition,
    format: Format,
    (path, overwrite): (&Path, Overwrite),
    order: export::BlockOrder,
    mut build: impl FnMut([u32; 2], [u32; 2]) -> Result<OutputBlock, Error>,
    before_publish: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    let [width, height] = composition.size();
    let size = ImageSize { width, height };
    match format {
        Format::Tiff(target) => export::tiff_encoded_checked(
            (path, overwrite),
            size,
            target,
            order,
            |origin, extent| match build(origin, extent)? {
                OutputBlock::Rgb16 {
                    target: encoded_target,
                    words,
                } if encoded_target == target => {
                    timed!(WorkingPack, rgb16::decode(&words))
                }
                #[cfg(test)]
                OutputBlock::Linear(pixels) => encode_readback(target, pixels),
                _ => Err(Error::Gpu("unexpected RGB16 output representation".into())),
            },
            before_publish,
        ),
        Format::Exr(precision) => export::exr_blocks_checked(
            (path, overwrite),
            size,
            precision,
            |origin, extent| match build(origin, extent)? {
                OutputBlock::Linear(pixels) => working_readback(pixels),
                _ => Err(Error::Gpu(
                    "expected scene-linear output representation".into(),
                )),
            },
            before_publish,
        ),
    }
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

fn working_readback(pixels: Vec<[f32; 4]>) -> Result<Vec<DVec3>, Error> {
    timed!(
        WorkingPack,
        pixels
            .iter()
            .map(|p| readback_rgb(p).map(|rgb| rgb.as_dvec3()))
            .collect()
    )
}

#[cfg(all(test, target_os = "macos"))]
mod packing_tests;
#[cfg(all(test, target_os = "macos"))]
mod rgb16_publication_tests;
