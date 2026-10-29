//! Original-camera operators. Existing photographic, repair, grain and look shaders own the math.
use super::*;
use crate::{
    geometry::SourceRegion as Rect,
    gpu::{GpuFlatField, ProcessingInput},
};

struct Native {
    scan: crate::scan::Scan,
}
impl Source for Native {
    fn domain(&self) -> Domain {
        Domain {
            bounds: self.scan.area,
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }
    fn read(&self, region: Rect) -> Result<Vec<[f32; 4]>, Error> {
        let values = self.scan.tile_region(
            self.scan.area,
            region.x - self.scan.area.x,
            region.y - self.scan.area.y,
            region.width,
            region.height,
        )?;
        crate::gpu::ProcessingSource::validate_continuous(&values)?;
        Ok(values)
    }
}

/// Capture native camera-code acquisition, including the consumer's recorded exposure context.
pub fn camera_source(scan: &crate::scan::Scan) -> Result<Arc<ImageNode>, Error> {
    let mut snapshot =
        crate::scan::Scan::with_reference(scan.source().clone(), scan.reference_exposure.clone());
    snapshot.set_flat_field(scan.flat_field().cloned())?;
    Ok(source(Arc::new(Native { scan: snapshot })))
}

pub(crate) fn original_chain(
    device: &wgpu::Device,
    renderer: &crate::renderer::GpuRenderer,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    input: &ProcessingInput,
    reference: crate::rendering::OutputReference,
    stages: &crate::pixel_pipeline::PixelPipeline,
) -> Result<Arc<ImageNode>, Error> {
    if scan.flat_field() != field.as_ref().map(|f| f.field()) {
        return Err(Error::Gpu("CPU and GPU flat-field resources differ".into()));
    }
    let matrix = if let Some(pipeline) = input.pipeline() {
        if !pipeline.input_program().matches_native(scan)? {
            return Err(Error::Gpu(
                "original input differs from captured camera source".into(),
            ));
        }
        pipeline.input_program().matrix
    } else {
        scan.camera_to_ap1
    };
    let node = camera_source(scan)?;
    Recipe::with_stages(input, stages.clone()).originals(
        device,
        renderer,
        (node, field, matrix),
        reference,
    )
}
