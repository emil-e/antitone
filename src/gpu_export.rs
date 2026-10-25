//! One bounded original-sample engine for TIFF and scene-linear EXR.
use crate::export_timing::timed;
use crate::{
    Error,
    export::{self, ExrPrecision, ImageSize, Overwrite},
    frame_raster::FrameRaster,
    geometry::{Composition, SourceRegion},
    gpu::{GpuFlatField, GpuPhotoGrade, ProcessingSource, SourceGeometry, SubmissionGate},
    local_adjustments::PhotoGradeSnapshot,
    processing::{InputSettings, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
    scan::Scan,
};
use glam::{DVec3, Vec3};
use std::{path::Path, sync::Arc};

#[path = "gpu_export_area.rs"]
mod area;
#[cfg(feature = "project")]
#[path = "gpu_export_job.rs"]
pub(crate) mod job;
#[path = "gpu_rgb16.rs"]
mod rgb16;

#[cfg(all(test, target_os = "macos", feature = "ocio-reference"))]
#[path = "gpu_export_common_exr_tests.rs"]
mod common_exr_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_observation_clock.rs"]
mod observation_clock;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_production_tests.rs"]
mod production_tests;
#[cfg(all(test, target_os = "macos", feature = "editor"))]
#[path = "gpu_export_resource_tests.rs"]
mod resource_tests;
#[cfg(test)]
#[path = "gpu_export_tests.rs"]
mod tests;

const TILE: [u32; 2] = export::BLOCK;

enum OutputBlock {
    Linear(Vec<[f32; 4]>),
    Rgb16 {
        target: OutputTarget,
        words: Vec<[u32; 2]>,
    },
}

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Tiff(OutputTarget),
    Exr(ExrPrecision),
}

/// Accepted state only. The scan leases the authoritative immutable development and correction.
pub struct Request<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub input: InputSettings,
    pub roll: RollSettings,
    pub grade: PhotoGradeSnapshot,
    pub repairs: &'a [Arc<crate::dust_repair::EvaluatedRepair>],
    pub composition: Composition,
    pub format: Format,
    pub look: Option<Arc<crate::looks::LookAsset>>,
}

/// One bounded dispatch/readback outstanding; no CPU photographic rendering fallback.
/// Uses the renderer's original-processing kernel, initialized with the shared renderer.
pub fn write(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
) -> Result<(), Error> {
    write_checked(renderer, device, gpu, request, path, overwrite, || Ok(()))
}

// Diagnostics and captured workers share the same bounded stop/publication checks.
fn write_checked(
    renderer: &GpuRenderer,
    device: &wgpu::Device,
    gpu: (&wgpu::Queue, &SubmissionGate),
    request: Request<'_>,
    path: &Path,
    overwrite: Overwrite,
    check: impl FnMut() -> Result<(), Error>,
) -> Result<(), Error> {
    area::write(renderer, device, gpu, request, (path, overwrite), check).map(|_| ())
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
#[path = "gpu_export_packing_tests.rs"]
mod packing_tests;
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_rgb16_publication_tests.rs"]
mod rgb16_publication_tests;
