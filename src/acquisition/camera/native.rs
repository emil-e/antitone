//! Developed camera rectangles on the original grid.

use super::ProcessingSource;
use crate::{
    Error,
    evaluation::{CellRegion, Domain, Grid, ImageFormat, RetainedKey, Source, Value, source},
    scan::{CameraIdentity, DevelopedSource},
};
use std::sync::Arc;

struct Native {
    developed: Arc<DevelopedSource>,
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
        self.developed.camera_image()?.read_codes(region.source()?)
    }

    fn identity(&self) -> Option<RetainedKey> {
        Some(RetainedKey::new(NativeIdentity(self.developed.identity())))
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
        let width = region.width as usize;
        let mut values = vec![[0.; 4]; width * region.height as usize];
        self.developed
            .camera_image()?
            .copy_samples(region, &mut values, width)?;
        ProcessingSource::validate_continuous(&values)?;
        Ok(values)
    }
}

/// Retain the original camera samples, before photographic correction or interpretation.
pub fn camera_source(developed: &Arc<DevelopedSource>) -> Result<Value, Error> {
    let bounds = developed.area().try_into()?;
    let format = if developed.camera_image()?.is_continuous() {
        ImageFormat::Rgba32Float
    } else {
        ImageFormat::Rgba16Uint
    };
    Ok(source(Arc::new(Native {
        developed: developed.clone(),
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
            let source = Arc::new(
                DevelopedSource::from_raw(
                    RawFile::open_uncorrected(file.path()).unwrap(),
                    Some(wb),
                )
                .unwrap(),
            );
            let lease = Arc::downgrade(&source);
            let bounds: CellRegion = source.area().try_into().unwrap();
            let input = camera_source(&source).unwrap().source().unwrap();
            drop(source);
            let source = lease
                .upgrade()
                .expect("camera value retains the native lease");
            assert_eq!(input.storage_format(), expected_format);
            assert_eq!(input.domain().bounds, Some(bounds));
            assert_eq!(input.domain().grid, Grid::axis_aligned([0.; 2], [1.; 2]));

            let developed = source.camera_image().unwrap();
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
