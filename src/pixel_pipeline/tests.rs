use super::*;

#[derive(Debug, PartialEq)]
struct Independent {
    margin: u32,
}

impl EffectDefinition for Independent {
    #[cfg(feature = "gpu")]
    fn compose(
        &self,
        input: crate::gpu::evaluation::Value,
    ) -> Result<crate::gpu::evaluation::Value, Error> {
        Ok(input)
    }
}

#[test]
fn independently_defined_effects_have_value_identity() {
    let a = PixelStage::new(Independent { margin: 3 });
    let b = PixelStage::new(Independent { margin: 3 });
    assert_eq!(a, b);
    assert_ne!(a, PixelStage::new(Independent { margin: 4 }));
    assert_ne!(
        a,
        PixelStage::new(crate::image_mtf::ImageMtfParameters { sigma: 0.75 })
    );
}

#[test]
fn zero_grain_is_absent_from_the_recipe_while_image_mtf_remains() {
    let settings = GrainSettings {
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
        PixelPipeline::film(GrainSettings::default(), [6000, 4000], 17)
            .unwrap()
            .is_empty()
    );
}
