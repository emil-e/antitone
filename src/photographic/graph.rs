//! Photographic image boundaries and their contracted camera-cell implementations.

use crate::Error;
use crate::gpu::evaluation::*;
#[cfg(any(test, feature = "editor"))]
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::{Mapping, ProcessingSource};
#[cfg(feature = "editor")]
use crate::photographic::stages::flat_field::operation::CameraCorrection;
use crate::photographic::stages::flat_field::{GpuFlatField, operation::apply as camera_image};
use crate::photographic::stages::grading::Grading;
use crate::reconstruction::reconstruct_cells;
use std::sync::Arc;

pub(crate) mod admission;
#[cfg(test)]
pub(crate) mod authoring;
#[cfg(all(test, feature = "editor"))]
pub(crate) mod authoring_examples;
#[cfg(feature = "editor")]
pub mod camera_response;
pub(crate) mod composition;
#[cfg(all(
    feature = "raw",
    feature = "project",
    any(feature = "editor", feature = "export")
))]
#[cfg(all(test, feature = "editor"))]
mod neutral_readout;
pub(crate) mod operations;
#[cfg(all(test, feature = "editor"))]
mod output_tests;
#[cfg(any(test, feature = "editor"))]
pub(crate) mod sampling;
#[cfg(all(test, feature = "editor"))]
pub(crate) mod test_effects;
#[cfg(feature = "render")]
pub use composition::display_output;
pub use composition::original_image;
#[cfg(feature = "render")]
pub use prewarm::prewarm_viewer;
#[cfg(all(test, feature = "editor"))]
mod provenance_tests;
#[cfg(feature = "editor")]
pub(crate) mod request;
#[cfg(all(feature = "ui", feature = "render"))]
pub(crate) mod resident;
#[cfg(all(test, feature = "editor"))]
mod tests;
#[cfg(feature = "editor")]
pub(crate) mod view_source;
use crate::photographic::stages::interpretation::operation::{Interpretation, NativeIdentity};
use operations::{ResidentCamera, validate_camera_range};
#[cfg(feature = "render")]
mod prewarm;
#[cfg(test)]
pub mod test_support;

#[cfg(feature = "editor")]
pub(crate) fn resident_camera_buffers(image: &Value) -> Vec<wgpu::Buffer> {
    let mut buffers = Vec::new();
    for node in image.ancestors() {
        if let Some(camera) =
            (node.operation().as_ref() as &dyn std::any::Any).downcast_ref::<ResidentCamera>()
            && !buffers.contains(&&camera.pixels)
        {
            buffers.push(&camera.pixels);
        }
    }
    buffers.into_iter().cloned().collect()
}

#[cfg(feature = "editor")]
pub(crate) fn input_transform(
    image: &Value,
) -> Result<Option<crate::photographic::stages::interpretation::InputTransform>, Error> {
    let mut captured = None;
    for node in image.ancestors() {
        if let Some(native) = node.parameters::<NativeIdentity>()
            && let Interpretation::Program(program) = native.0
        {
            if captured.is_some_and(|prior| prior != program) {
                return Err(Error::InvalidParameter(
                    "calculation mixes source interpretations",
                ));
            }
            captured = Some(program);
        }
    }
    Ok(captured)
}

/// Individually selected original cells, retaining their physical correction coordinates.
#[cfg(feature = "editor")]
pub(crate) fn camera_samples(
    source: Value,
    field: Option<Arc<GpuFlatField>>,
    positions: &[[u32; 2]],
) -> Result<Value, Error> {
    camera_patches(source, field, positions, [1, 1])
}

#[cfg(feature = "editor")]
fn camera_patches(
    source: Value,
    field: Option<Arc<GpuFlatField>>,
    origins: &[[u32; 2]],
    patch: [u32; 2],
) -> Result<Value, Error> {
    let height = u32::try_from(origins.len())
        .ok()
        .and_then(|count| count.checked_mul(patch[1]))
        .filter(|height| *height > 0)
        .ok_or(Error::InvalidParameter("invalid camera footprint batch"))?;

    if source.domain().bounds != Some(CellRegion::new([0, 0], [patch[0], height]))
        || source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2])
    {
        return Err(Error::InvalidParameter(
            "camera footprints require an original-cell index grid",
        ));
    }

    validate_camera_range(glam::DMat3::IDENTITY, field.as_deref(), true)?;
    crate::photographic::stages::flat_field::gathered(source, field.as_deref(), origins, patch)
}

/// Corrected original camera light, excluding interpretation and recorded positive exposure.
pub fn camera_light(source: Value, field: Option<Arc<GpuFlatField>>) -> Result<Value, Error> {
    if source.domain().grid != Grid::axis_aligned([0.; 2], [1.; 2]) {
        return Err(Error::InvalidParameter(
            "camera-light conversion requires original camera cells",
        ));
    }

    validate_camera_range(glam::DMat3::IDENTITY, field.as_deref(), true)?;
    Ok(camera_image(source, field, Mapping::Originals))
}

#[cfg(feature = "editor")]
fn view_domain(frame: crate::frame_raster::FrameRaster, coverage: u32) -> Domain {
    Domain {
        bounds: Some(CellRegion::new([0, 0], frame.output)),
        grid: Grid {
            coverage,
            ..Grid::from_frame(frame)
        },
    }
}

#[cfg(feature = "editor")]
fn resident_camera_argument(image: &Value) -> Result<(&Value, Mapping, bool), Error> {
    let mut argument: Option<(&crate::gpu::evaluation::value::Value, Mapping, bool)> = None;
    for current in image.ancestors() {
        if let Some(camera) =
            (current.operation().as_ref() as &dyn std::any::Any).downcast_ref::<CameraCorrection>()
            && let Some(input) = current.unary_input()
            && let Some(resident) =
                (input.operation().as_ref() as &dyn std::any::Any).downcast_ref::<ResidentCamera>()
        {
            if argument.is_some_and(|(prior, _, _)| !prior.same_definition(input)) {
                return Err(Error::InvalidParameter(
                    "calculation mixes resident camera arguments",
                ));
            }
            argument = Some((input, camera.mapping, resident.continuous));
        }
    }
    let Some((input, mapping, continuous)) = argument else {
        return Err(Error::InvalidParameter(
            "calculation has no resident camera argument",
        ));
    };
    Ok((input, mapping, continuous))
}
