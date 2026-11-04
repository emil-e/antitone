use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::{halation::HalationSettings, pixel_pipeline::PixelPipeline};

fn base() -> AuthoredPhotograph {
    AuthoredPhotograph::from(
        ProcessingParameters::interpret(
            crate::processing::InputSettings::Digital,
            Default::default(),
            glam::DMat3::IDENTITY,
        )
        .unwrap(),
    )
}

fn capture(halo: HalationSettings, frame: [u32; 2]) -> AuthoredPhotograph {
    base()
        .with_pixel_pipeline(PixelPipeline::emulation(Default::default(), halo, frame, 0).unwrap())
}

#[test]
fn active_halation_invalidates_photographic_content_and_tracks_crop_size() {
    let clean = base();
    let settings = HalationSettings {
        enabled: true,
        ..Default::default()
    };
    let halo = capture(settings, [6000, 4000]);
    assert_ne!(clean, halo);
    assert!(!clean.same_photographic(&halo));
    assert!(
        halo.pixel_pipeline()
            .stages
            .iter()
            .all(|stage| stage.get::<crate::pixel_pipeline::Grain>().is_none())
    );
    for changed in [
        capture(settings, [3000, 2000]),
        capture(
            HalationSettings {
                strength: 0.4,
                ..settings
            },
            [6000, 4000],
        ),
        capture(
            HalationSettings {
                size: 0.2,
                ..settings
            },
            [6000, 4000],
        ),
    ] {
        assert!(!halo.same_photographic(&changed));
    }
}

#[test]
fn disabled_custom_halation_and_zero_strength_do_not_change_content_identity() {
    let clean = base();
    for halo in [
        HalationSettings {
            enabled: false,
            strength: 0.4,
            size: 0.2,
        },
        HalationSettings {
            enabled: true,
            strength: 0.,
            ..Default::default()
        },
    ] {
        assert_eq!(clean, capture(halo, [3000, 2000]));
    }
}

#[test]
fn source_inspection_never_adopts_halation() {
    let source = AuthoredPhotograph::from(ProcessingParameters::source());
    let pipeline = PixelPipeline::emulation(
        Default::default(),
        HalationSettings {
            enabled: true,
            ..Default::default()
        },
        [3000, 2000],
        0,
    )
    .unwrap();
    assert!(!pipeline.is_empty(), "the attempted effect must be active");
    let candidate = source.clone().with_pixel_pipeline(pipeline);
    assert_eq!(source, candidate);
    assert!(candidate.pixel_pipeline().is_empty());
}
