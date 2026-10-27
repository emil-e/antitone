use super::*;
use crate::processing::InputPipeline;
use std::sync::Weak;

/// Resolved working-positive sampling in original source coordinates.
pub struct ImageSource<'a> {
    scan: &'a Scan,
    pipeline: InputPipeline,
    identity: ImageIdentity,
}

/// Output identity retains source provenance without retaining native pixel ownership.
#[derive(Clone)]
pub struct ImageIdentity {
    native: Weak<FrozenRaw>,
    capture: crate::source::FileKey,
    fixed_wb: [f64; 4],
    lens_correction: bool,
    dng_interpretation: Option<String>,
    input: InputSettings,
    matrix: DMat3,
    correction: Option<Arc<FlatField>>,
}

impl PartialEq for ImageIdentity {
    fn eq(&self, other: &Self) -> bool {
        let same_capture = if self.capture.revision.is_some() && other.capture.revision.is_some() {
            self.capture == other.capture
        } else {
            self.native.ptr_eq(&other.native)
        };
        same_capture
            && self.fixed_wb == other.fixed_wb
            && self.lens_correction == other.lens_correction
            && self.dng_interpretation == other.dng_interpretation
            && self.input == other.input
            && self.matrix == other.matrix
            && self.correction == other.correction
    }
}

impl ImageSource<'_> {
    pub fn new(scan: &Scan, input: impl Into<InputSettings>) -> Result<ImageSource<'_>> {
        let input = scan.refined_input(&input.into())?;
        let matrix = scan.positive_matrix(&input)?;
        let pipeline = InputPipeline::new(input, matrix)?;
        let identity = ImageIdentity {
            native: Arc::downgrade(&scan.source.raw),
            capture: scan.source.file.clone(),
            fixed_wb: scan.source.fixed_wb,
            lens_correction: scan.source.metadata().lens_correction_enabled(),
            dng_interpretation: scan.source.metadata().dng_identity(),
            input,
            matrix,
            correction: scan.flat_field.clone(),
        };
        Ok(ImageSource {
            scan,
            pipeline,
            identity,
        })
    }

    pub fn area(&self) -> Rect {
        self.scan.area
    }

    pub fn identity(&self) -> &ImageIdentity {
        &self.identity
    }

    pub fn sample(&self, pixel: [u32; 2]) -> Result<DVec3> {
        Ok(self
            .pipeline
            .apply(self.scan.pixel(pixel[0], pixel[1])?)?
            .rgb)
    }

    pub fn read_tile(&self, region: Rect) -> Result<Vec<DVec3>> {
        self.scan.validate_area(region)?;
        (region.y..region.y + region.height)
            .flat_map(|y| (region.x..region.x + region.width).map(move |x| [x, y]))
            .map(|point| self.sample(point))
            .collect()
    }

    #[cfg(feature = "gpu")]
    pub fn gpu_program(&self) -> Result<crate::gpu::GpuInputProgram> {
        crate::gpu::GpuInputProgram::new(self.identity.input, self.identity.matrix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "project")]
    use crate::exposure_reference::{ReferenceExposure, StartingExposure};

    #[cfg(feature = "project")]
    fn reference(ev: f64) -> ReferenceExposure {
        ReferenceExposure::Matched(StartingExposure {
            ev,
            samples: 1024,
            spread: 0.,
        })
    }

    #[cfg(feature = "project")]
    #[test]
    fn resolved_images_keep_their_normalization_and_identity_after_native_rebinding() {
        let file = crate::raw::fixture::bayer_dng();
        let native = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let first_native = native.source().interpreted(&reference(1.));
        let second_native = native.source().interpreted(&reference(2.));
        let first_scan = Scan::from_source(first_native);
        let second_scan = Scan::from_source(second_native);
        let first = ImageSource::new(&first_scan, InputSettings::Digital).unwrap();
        let before = first.sample([32, 32]).unwrap();
        let identity = first.identity().clone();
        let second = ImageSource::new(&second_scan, InputSettings::Digital).unwrap();
        let after = second.sample([32, 32]).unwrap();
        assert!((after - before * 2.).abs().max_element() < 1e-12);
        assert!(first.identity() != second.identity());
        assert!(first.identity() == &identity);
        assert_eq!(first.sample([32, 32]).unwrap(), before);
        let reloaded = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let reloaded = Scan::from_source(reloaded.source().interpreted(&reference(1.)));
        let image = ImageSource::new(&reloaded, InputSettings::Digital).unwrap();
        assert!(
            image.identity() == first.identity(),
            "reload of matching captured pixels must preserve content identity"
        );
    }

    #[test]
    fn matching_native_reload_preserves_identity_without_persistence() {
        let file = crate::raw::fixture::bayer_dng();
        let first = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let second = Scan::open(file.path(), Some([1.; 4])).unwrap();
        let image = ImageSource::new(&first, InputSettings::Digital).unwrap();
        let reloaded = ImageSource::new(&second, InputSettings::Digital).unwrap();
        assert!(image.identity() == reloaded.identity());
        assert_eq!(
            image.sample([32, 32]).unwrap(),
            reloaded.sample([32, 32]).unwrap()
        );
    }

    #[test]
    fn retaining_an_image_identity_does_not_retain_native_pixels() {
        let file = crate::raw::fixture::bayer_dng();
        let (identity, native) = {
            let scan = Scan::open(file.path(), Some([1.; 4])).unwrap();
            let image = ImageSource::new(&scan, InputSettings::Digital).unwrap();
            (image.identity().clone(), Arc::downgrade(&scan.source.raw))
        };
        assert!(native.upgrade().is_none());
        drop(identity);
    }
}
