use antitone::{
    film_emulation::FilmPreset,
    halation::{HalationParameters, HalationSettings},
    image_mtf::ImageMtfParameters,
    pixel_pipeline::{Grain, PixelPipeline},
};

#[test]
fn tuned_defaults_compose_with_the_shared_size_profile_and_unchanged_image_mtf() {
    let defaults = HalationSettings::default();
    defaults.validate().unwrap();
    assert!(!defaults.enabled);
    let film = FilmPreset::ThirtyFiveMm.settings();
    let frame = [3840, 2160];
    assert_eq!(
        PixelPipeline::emulation(film, defaults, frame, 17).unwrap(),
        PixelPipeline::film(film, frame, 17).unwrap()
    );
    let pipeline = PixelPipeline::emulation(
        film,
        HalationSettings {
            enabled: true,
            ..defaults
        },
        frame,
        17,
    )
    .unwrap();
    let mtf = pipeline.stages[0].get::<ImageMtfParameters>().unwrap();
    let halo = pipeline.stages[1].get::<HalationParameters>().unwrap();
    let grain = pipeline.stages[2].get::<Grain>().unwrap();
    assert!((mtf.sigma - 0.864).abs() < 1e-12);
    assert!((halo.sigma - 2.3544).abs() < 1e-12);
    for (actual, expected) in halo
        .fractions()
        .into_iter()
        .zip([27. / 77., 189. / 814., 0.])
    {
        assert!((actual - expected).abs() < 1e-15);
    }
    assert!((grain.parameters.grain_size - 1.188).abs() < 1e-12);
    assert_eq!(grain.parameters.amplitude, 0.065);
    assert_eq!(grain.parameters.colour, 0.5);
    assert_eq!(grain.seed, 17);
}

#[test]
fn halation_runs_independently_and_resolves_from_the_logical_crop() {
    let halo = HalationSettings {
        enabled: true,
        size: 0.125,
        strength: 0.2,
    };
    let full = PixelPipeline::emulation(Default::default(), halo, [9000, 6000], 17).unwrap();
    assert_eq!(full.stages.len(), 1);
    assert_eq!(
        full.stages[0].get::<HalationParameters>().unwrap().sigma,
        7.5
    );
    let crop = PixelPipeline::emulation(Default::default(), halo, [4500, 3000], 17).unwrap();
    assert_eq!(
        crop.stages[0].get::<HalationParameters>().unwrap().sigma,
        3.75
    );
    assert_eq!(
        crop,
        PixelPipeline::emulation(Default::default(), halo, [3000, 4500], 19).unwrap()
    );
    assert!(PixelPipeline::emulation(Default::default(), halo, [0, 6000], 17).is_err());
}

#[test]
fn full_emulation_places_halation_after_completed_mtf_before_grain() {
    let film = FilmPreset::ThirtyFiveMm.settings();
    let halo = HalationSettings {
        enabled: true,
        ..Default::default()
    };
    let pipeline = PixelPipeline::emulation(film, halo, [6000, 4000], 17).unwrap();
    assert_eq!(pipeline.stages.len(), 3);
    assert!(pipeline.stages[0].get::<ImageMtfParameters>().is_some());
    assert!(pipeline.stages[1].get::<HalationParameters>().is_some());
    assert!(pipeline.stages[2].get::<Grain>().is_some());
    let changed_grain = PixelPipeline::emulation(
        antitone::film_emulation::FilmEmulationSettings {
            enabled: true,
            size: 0.065,
            image_sigma: 0.065,
            amount: 0.065,
            ..Default::default()
        },
        halo,
        [6000, 4000],
        17,
    )
    .unwrap();
    assert_eq!(
        pipeline.stages[1], changed_grain.stages[1],
        "grain Size does not resize halation"
    );
    assert_ne!(pipeline.stages[0], changed_grain.stages[0]);
}

#[test]
fn disabled_and_zero_strength_preserve_existing_mtf_grain_pipeline() {
    for film in [
        Default::default(),
        antitone::film_emulation::FilmEmulationSettings {
            enabled: true,
            size: 0.025,
            image_sigma: 0.025,
            amount: 0.040,
            ..Default::default()
        },
        antitone::film_emulation::FilmEmulationSettings {
            enabled: true,
            size: 0.065,
            image_sigma: 0.065,
            amount: 0.065,
            ..Default::default()
        },
    ] {
        let expected = PixelPipeline::film(film, [6000, 4000], 17).unwrap();
        for halo in [
            HalationSettings {
                enabled: false,
                size: 0.2,
                strength: 0.4,
            },
            HalationSettings {
                enabled: true,
                strength: 0.,
                ..Default::default()
            },
        ] {
            assert_eq!(
                expected,
                PixelPipeline::emulation(film, halo, [6000, 4000], 17).unwrap()
            );
        }
    }
}
