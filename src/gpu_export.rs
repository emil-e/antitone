//! One bounded original-sample engine for TIFF and scene-linear EXR.
use crate::export_timing::timed;
#[cfg(all(test, target_os = "macos"))]
use crate::gpu::ProcessingKernel;
use crate::{
    Error,
    export::{self, ExrPrecision, ImageSize, Overwrite, ProcessingMetadata},
    frame_raster::FrameRaster,
    geometry::{Composition, SourceRegion},
    gpu::{GpuFlatField, GpuPhotoGrade, ProcessingSource, SourceGeometry, SubmissionGate},
    local_adjustments::PhotoGradeSnapshot,
    processing::{Calibration, RollSettings},
    renderer::GpuRenderer,
    rendering::OutputTarget,
    scan::Scan,
};
use glam::DVec3;
use std::{path::Path, sync::Arc};

#[path = "gpu_export_area.rs"]
mod area;
#[cfg(feature = "project")]
#[path = "gpu_export_job.rs"]
pub(crate) mod job;

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
#[cfg(all(test, target_os = "macos"))]
#[path = "gpu_export_tests.rs"]
mod tests;

const TILE: [u32; 2] = export::BLOCK;

#[derive(Clone, Copy, Debug)]
pub enum Format {
    Tiff(OutputTarget),
    Exr(ExrPrecision),
}

/// Accepted state only. The scan leases the authoritative immutable development and correction.
pub struct Request<'a> {
    pub scan: &'a Scan,
    pub field: Option<Arc<GpuFlatField>>,
    pub calibration: Calibration,
    pub roll: RollSettings,
    pub grade: PhotoGradeSnapshot,
    pub repairs: &'a [Arc<crate::dust_repair::EvaluatedRepair>],
    pub composition: Composition,
    pub format: Format,
    pub look: Option<Arc<crate::looks::LookAsset>>,
}

/// One bounded dispatch/readback outstanding; no CPU photographic rendering fallback.
/// The shared export kernel is lazy for library callers and prewarmed by the editor.
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
    metadata: ProcessingMetadata,
    destination: (&Path, Overwrite),
    build: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<DVec3>, Error>,
    before_publish: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    encode_ordered_checked(
        composition,
        format,
        metadata,
        destination,
        default_order(composition),
        build,
        before_publish,
    )
}

fn encode_ordered_checked(
    composition: Composition,
    format: Format,
    metadata: ProcessingMetadata,
    (path, overwrite): (&Path, Overwrite),
    order: export::BlockOrder,
    mut build: impl FnMut([u32; 2], [u32; 2]) -> Result<Vec<DVec3>, Error>,
    before_publish: impl FnOnce() -> Result<(), Error>,
) -> Result<(), Error> {
    let [width, height] = composition.size();
    let size = ImageSize { width, height };
    match format {
        Format::Tiff(target) => export::tiff_encoded_checked(
            (path, overwrite),
            size,
            target,
            metadata,
            order,
            |origin, extent| {
                let linear = build(origin, extent)?;
                timed!(TargetEncode, {
                    let linear = timed!(
                        TargetLayout,
                        linear.iter().map(|v| v.as_vec3()).collect::<Vec<_>>()
                    );
                    target.encode_u16(&linear)
                })
            },
            before_publish,
        ),
        Format::Exr(precision) => export::exr_blocks_checked(
            (path, overwrite),
            size,
            precision,
            metadata,
            build,
            before_publish,
        ),
    }
}
