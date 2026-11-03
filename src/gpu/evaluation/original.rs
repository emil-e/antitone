//! Original-camera operators. Existing photographic, repair, grain and look shaders own the math.

use super::*;
use crate::gpu::{GpuFlatField, ProcessingInput};

struct Native {
    scan: crate::scan::Scan,
    bounds: CellRegion,
    format: ImageFormat,
}

#[derive(PartialEq)]
struct NativeIdentity(crate::scan::CameraIdentity);
impl std::fmt::Debug for NativeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("developed camera samples")
    }
}

impl Source for Native {
    fn storage_format(&self) -> ImageFormat {
        self.format
    }

    fn read_codes(&self, region: CellRegion) -> Result<Vec<[u16; 4]>, Error> {
        if !self.bounds.contains_region(region) {
            return Err(Error::InvalidParameter(
                "camera storage outside acquired image",
            ));
        }
        let region = region.source()?;
        let image = self.scan.source().camera_image()?;
        let mut pixels = Vec::with_capacity(region.width as usize * region.height as usize);
        for y in region.y..region.bottom() {
            for x in region.x..region.right() {
                pixels.push(image.pixel_codes(x, y)?);
            }
        }
        Ok(pixels)
    }

    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(NativeIdentity(
            self.scan.source().identity(),
        )))
    }

    fn domain(&self) -> Domain {
        Domain {
            bounds: Some(self.bounds),
            grid: Grid::axis_aligned([0.; 2], [1.; 2]),
            meaning: Meaning::CameraCodes,
        }
    }

    fn read(&self, region: CellRegion) -> Result<Vec<[f32; 4]>, Error> {
        if !self.bounds.contains_region(region) {
            return Err(Error::InvalidParameter(
                "camera storage outside acquired image",
            ));
        }
        let region = region.source()?;
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
    let bounds = scan.area.try_into()?;
    let mut snapshot =
        crate::scan::Scan::with_reference(scan.source().clone(), scan.reference_exposure.clone());
    snapshot.set_flat_field(scan.flat_field().cloned())?;
    let format = if snapshot.source().camera_image()?.is_continuous() {
        ImageFormat::Rgba32Float
    } else {
        ImageFormat::Rgba16Uint
    };
    Ok(source(Arc::new(Native {
        scan: snapshot,
        bounds,
        format,
    })))
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
    let matrix = if let Some(program) = input.source_program() {
        if !program.matches_native(scan)? {
            return Err(Error::Gpu(
                "original input differs from captured camera source".into(),
            ));
        }
        program.matrix
    } else {
        scan.camera_to_ap1
    };
    let node = camera_source(scan)?;
    let photograph =
        Photograph::new_with_stages(&device.limits(), node, field, input, matrix, stages)?;
    Reference::capture(device, renderer, reference, input.look())?
        .compose(photograph.processed, renderer)
}
