use super::*;
fn base() -> ProcessingInput {
    ProcessingInput::from(
        ProcessingParameters::interpret(
            crate::processing::InputSettings::Digital,
            Default::default(),
            glam::DMat3::IDENTITY,
        )
        .unwrap(),
    )
}
#[test]
fn active_halation_invalidates_photographic_content_and_tracks_crop_size() {
    let clean = base().with_frame_size([6000, 4000]);
    let halo = clean.clone().with_halation(HalationSettings {
        enabled: true,
        ..Default::default()
    });
    assert_ne!(clean.signature(), halo.signature());
    assert!(!clean.signature().same_photographic(&halo.signature()));
    assert!(!halo.film_emulation().enabled);
    for changed in [
        halo.clone().with_frame_size([3000, 2000]),
        halo.clone().with_halation(HalationSettings {
            enabled: true,
            strength: 0.4,
            ..Default::default()
        }),
        halo.clone().with_halation(HalationSettings {
            enabled: true,
            size: 0.2,
            ..Default::default()
        }),
    ] {
        assert!(!halo.signature().same_photographic(&changed.signature()));
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
        assert_eq!(
            clean.signature(),
            clean
                .clone()
                .with_frame_size([3000, 2000])
                .with_halation(halo)
                .signature()
        );
    }
}
#[test]
fn source_inspection_never_adopts_halation() {
    let source = ProcessingInput::from(ProcessingParameters::source());
    let candidate = source.clone().with_halation(HalationSettings {
        enabled: true,
        ..Default::default()
    });
    assert_eq!(source.signature(), candidate.signature());
    assert!(!candidate.halation().enabled);
}
