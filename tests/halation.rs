use antitone::{grain::GrainPreset, halation::HalationSettings};

#[test]
fn tuned_defaults_resolve_independent_halo_and_film_lengths() {
    let defaults = HalationSettings::default();
    defaults.validate().unwrap();
    let frame = [3840, 2160];
    assert!(defaults.parameters(frame).unwrap().is_none());
    let film = GrainPreset::ThirtyFiveMm
        .settings()
        .parameters(frame)
        .unwrap()
        .unwrap();
    let halo = HalationSettings {
        enabled: true,
        ..defaults
    }
    .parameters(frame)
    .unwrap()
    .unwrap();
    assert!((film.image_mtf_sigma - 0.864).abs() < 1e-12);
    assert!((halo.sigma - 2.3544).abs() < 1e-12);
    for (actual, expected) in halo
        .fractions()
        .into_iter()
        .zip([27. / 77., 189. / 814., 0.])
    {
        assert!((actual - expected).abs() < 1e-15);
    }
    let grain = film;
    assert!((grain.grain_size - 1.188).abs() < 1e-12);
    assert_eq!(grain.amplitude, 0.065);
    assert_eq!(grain.colour, 0.5);
}

#[test]
fn halation_resolves_from_the_logical_crop_independently_of_film_settings() {
    let halo = HalationSettings {
        enabled: true,
        size: 0.125,
        strength: 0.2,
    };
    let full = halo.parameters([9000, 6000]).unwrap().unwrap();
    let crop = halo.parameters([4500, 3000]).unwrap().unwrap();
    assert_eq!(full.sigma, 7.5);
    assert_eq!(crop.sigma, 3.75);
    assert_eq!(Some(crop), halo.parameters([3000, 4500]).unwrap());
    assert!(halo.parameters([0, 6000]).is_err());
}

#[test]
fn disabled_and_zero_strength_omit_halation() {
    for settings in [
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
        assert!(settings.parameters([6000, 4000]).unwrap().is_none());
    }
}
