//! Developed camera rectangles on the original grid.

use super::ProcessingSource;
use crate::{
    Error,
    evaluation::{CellRegion, Domain, Grid, ImageFormat, RetainedKey, Source, Value, source},
    scan::{CameraIdentity, Scan},
};
use std::sync::Arc;

struct Native {
    scan: Scan,
    bounds: CellRegion,
    format: ImageFormat,
}

#[derive(PartialEq, Hash)]
struct NativeIdentity(CameraIdentity);
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
        ProcessingSource::validate_continuous(&values)?;
        Ok(values)
    }
}

/// Capture native camera-code acquisition, including the consumer's recorded exposure context.
pub fn camera_source(scan: &Scan) -> Result<Value, Error> {
    let bounds = scan.area.try_into()?;
    let mut snapshot = Scan::with_reference(scan.source().clone(), scan.reference_exposure.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::{RawFile, fixture};

    #[test]
    fn native_rectangles_keep_codes_or_continuous_headroom() {
        let file = fixture::bayer_dng_with_rgb([64000; 3]);
        let region = CellRegion::new([31, 32], [2, 2]);
        for (wb, expected_format) in [
            ([1.; 4], ImageFormat::Rgba16Uint),
            ([4., 1., 2., 1.], ImageFormat::Rgba32Float),
        ] {
            let scan =
                Scan::from_raw(RawFile::open_uncorrected(file.path()).unwrap(), Some(wb)).unwrap();
            let input = camera_source(&scan).unwrap().source().unwrap();
            assert_eq!(input.storage_format(), expected_format);
            let bounds: CellRegion = scan.area.try_into().unwrap();
            assert_eq!(input.domain().bounds, Some(bounds));
            assert_eq!(input.domain().grid, Grid::axis_aligned([0.; 2], [1.; 2]));

            let developed = scan.source().camera_image().unwrap();
            let cells = (32..34).flat_map(|y| (31..33).map(move |x| (x, y)));
            match expected_format {
                ImageFormat::Rgba16Uint => {
                    let expected: Vec<_> = cells
                        .map(|(x, y)| developed.pixel_codes(x, y).unwrap())
                        .collect();
                    assert_eq!(input.read_codes(region).unwrap(), expected);
                }
                ImageFormat::Rgba32Float => {
                    let expected: Vec<_> = cells
                        .map(|(x, y)| developed.pixel_samples(x, y).unwrap())
                        .collect();
                    assert!(expected.iter().all(|pixel| pixel[0] > 65535.));
                    assert_eq!(input.read(region).unwrap(), expected);
                    assert!(input.read_codes(region).is_err());
                }
            }
            let outside = CellRegion::new([bounds.right() - 1, region.y], [2, 1]);
            assert!(input.read(outside).is_err());
            assert!(input.read_codes(outside).is_err());
        }
    }
}
