use super::*;
#[derive(Debug, PartialEq)]
struct Independent {
    margin: u32,
}
impl Layer for Independent {
    fn support(&self, _: [f64; 2]) -> Support {
        Support([self.margin, 0])
    }
    #[cfg(all(feature = "gpu", feature = "render"))]
    fn compose(
        &self,
        input: Arc<crate::gpu::pull::Layer>,
    ) -> Result<Arc<crate::gpu::pull::Layer>, Error> {
        Ok(input)
    }
}
#[test]
fn independently_defined_layers_have_value_identity_and_declare_their_own_support() {
    let a = PixelStage::new(Independent { margin: 3 });
    let b = PixelStage::new(Independent { margin: 3 });
    assert_eq!(a, b);
    assert_ne!(a, PixelStage::new(Independent { margin: 4 }));
    assert_ne!(
        a,
        PixelStage::new(crate::image_mtf::ImageMtfParameters { sigma: 0.75 })
    );
    let pipeline = PixelPipeline { stages: vec![a, b] };
    assert_eq!(pipeline.support([1.; 2]).unwrap(), Support([6, 0]));
}
#[test]
fn zero_grain_is_absent_from_the_recipe_while_image_mtf_remains() {
    let settings = FilmEmulationSettings {
        enabled: true,
        amount: 0.,
        ..Default::default()
    };
    let pipeline = PixelPipeline::film(settings, [6000, 4000], 17).unwrap();
    assert!(pipeline.stages.iter().any(|stage| {
        stage
            .get::<crate::image_mtf::ImageMtfParameters>()
            .is_some()
    }));
    assert!(!pipeline.stages.iter().any(|s| s.get::<Grain>().is_some()));
    assert!(
        PixelPipeline::film(FilmEmulationSettings::default(), [6000, 4000], 17)
            .unwrap()
            .is_empty()
    );
}
