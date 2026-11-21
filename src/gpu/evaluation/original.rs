//! Original-camera operators. Existing photographic, repair, grain and look shaders own the math.

use super::*;

struct Native {
    scan: crate::scan::Scan,
    bounds: CellRegion,
    format: ImageFormat,
}

#[derive(PartialEq, Hash)]
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
        crate::gpu::source::ProcessingSource::validate_continuous(&values)?;
        Ok(values)
    }
}

/// Capture native camera-code acquisition, including the consumer's recorded exposure context.
pub fn camera_source(scan: &crate::scan::Scan) -> Result<Value, Error> {
    let bounds = scan.area.try_into()?;
    let mut snapshot =
        crate::scan::Scan::with_reference(scan.source().clone(), scan.reference_exposure.clone());
    snapshot.set_flat_field(scan.flat_field().cloned());
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
