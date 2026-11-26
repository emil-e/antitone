use super::*;
use crate::processing::InputPipeline;

/// Resolved working-positive sampling in original source coordinates.
pub struct ImageSource<'a> {
    source: &'a DevelopedSource,
    pipeline: InputPipeline,
    identity: ImageIdentity,
}

/// Output identity retains source provenance without retaining native pixel ownership.
#[derive(Clone)]
pub struct ImageIdentity {
    camera: CameraIdentity,
    input: InputSettings,
    matrix: DMat3,
    correction: Option<Arc<FlatField>>,
}

impl PartialEq for ImageIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.camera == other.camera
            && self.input == other.input
            && self.matrix == other.matrix
            && self.correction == other.correction
    }
}

impl ImageSource<'_> {
    pub fn new(
        source: &DevelopedSource,
        input: impl Into<InputSettings>,
        reference_ev: Option<f64>,
        correction: Option<Arc<FlatField>>,
    ) -> Result<ImageSource<'_>> {
        let input = input.into();
        let matrix = input.positive_matrix(source.camera_to_ap1(), reference_ev)?;
        Self::with_transform(source, input, matrix, correction)
    }

    #[cfg(feature = "project")]
    pub fn from_recipe<'a>(
        source: &'a DevelopedSource,
        recipe: &crate::image_recipe::SourceRecipe,
    ) -> Result<ImageSource<'a>> {
        recipe.require_source(source)?;
        let (input, matrix) = recipe.processing_transform()?;
        Self::with_transform(source, input, matrix, recipe.correction.clone())
    }

    fn with_transform(
        source: &DevelopedSource,
        input: InputSettings,
        matrix: DMat3,
        correction: Option<Arc<FlatField>>,
    ) -> Result<ImageSource<'_>> {
        let pipeline = InputPipeline::new(input, matrix)?;
        let identity = ImageIdentity {
            camera: source.identity(),
            input,
            matrix,
            correction,
        };
        Ok(ImageSource {
            source,
            pipeline,
            identity,
        })
    }

    pub fn area(&self) -> Rect {
        self.source.area()
    }

    pub fn identity(&self) -> &ImageIdentity {
        &self.identity
    }

    pub fn sample(&self, pixel: [u32; 2]) -> Result<DVec3> {
        let camera = crate::flat_field::sample_camera_light(
            self.source,
            self.identity.correction.as_deref(),
            pixel,
        )?;
        Ok(self.pipeline.apply(camera)?.rgb)
    }

    pub fn read_tile(&self, region: Rect) -> Result<Vec<DVec3>> {
        self.source.validate_area(region)?;
        (region.y..region.y + region.height)
            .flat_map(|y| (region.x..region.x + region.width).map(move |x| [x, y]))
            .map(|point| self.sample(point))
            .collect()
    }

    #[cfg(feature = "gpu")]
    pub fn gpu_program(
        &self,
    ) -> Result<crate::photographic::stages::interpretation::InputTransform> {
        crate::photographic::stages::interpretation::InputTransform::new(
            self.identity.input,
            self.identity.matrix,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolved_images_keep_their_normalization_and_identity_after_native_rebinding() {
        let file = crate::raw::fixture::bayer_dng();
        let native = DevelopedSource::open(file.path(), Some([1.; 4])).unwrap();
        let first = ImageSource::new(&native, InputSettings::Digital, Some(1.), None).unwrap();
        let before = first.sample([32, 32]).unwrap();
        let identity = first.identity().clone();
        let second = ImageSource::new(&native, InputSettings::Digital, Some(2.), None).unwrap();
        let after = second.sample([32, 32]).unwrap();
        assert!((after - before * 2.).abs().max_element() < 1e-12);
        assert!(first.identity() != second.identity());
        assert!(first.identity() == &identity);
        assert_eq!(first.sample([32, 32]).unwrap(), before);
        let reloaded = DevelopedSource::open(file.path(), Some([1.; 4])).unwrap();
        let image = ImageSource::new(&reloaded, InputSettings::Digital, Some(1.), None).unwrap();
        assert!(
            image.identity() == first.identity(),
            "reload of matching captured pixels must preserve content identity"
        );
    }

    #[test]
    fn matching_native_reload_preserves_identity_without_persistence() {
        let file = crate::raw::fixture::bayer_dng();
        let first = DevelopedSource::open(file.path(), Some([1.; 4])).unwrap();
        let second = DevelopedSource::open(file.path(), Some([1.; 4])).unwrap();
        let image = ImageSource::new(&first, InputSettings::Digital, None, None).unwrap();
        let reloaded = ImageSource::new(&second, InputSettings::Digital, None, None).unwrap();
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
            let scan = DevelopedSource::open(file.path(), Some([1.; 4])).unwrap();
            let image = ImageSource::new(&scan, InputSettings::Digital, None, None).unwrap();
            (image.identity().clone(), Arc::downgrade(&scan.raw))
        };
        assert!(native.upgrade().is_none());
        drop(identity);
    }
}
