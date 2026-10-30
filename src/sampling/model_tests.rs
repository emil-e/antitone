use super::*;

#[test]
fn gamma_model_requires_anchor_information_even_inside_the_forward_model_resolution() {
    let negative = NegativeInput::from(crate::processing::Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.005),
        input_floor: crate::processing::CAMERA_INPUT_FLOOR,
    });
    let matrix = DMat3::from_diagonal(DVec3::new(0.8, 1.1, 0.9));
    let fit = GammaNeutral::new(negative, matrix).unwrap();
    let floor = 16. * f64::from(f32::EPSILON);

    // A neutral result alone is insufficient: a constant model provides no gamma information.
    assert!(matches!(
        fit.fit(|_| Ok(fit.target * 0.2), floor),
        Err(Error::UnusableSample(_))
    ));

    let truth = Balance {
        warmth: 0.2,
        tint: -0.1,
    };
    let actual = fit
        .fit(
            |candidate| {
                assert_eq!(candidate.calibration, negative.calibration);
                assert_eq!(candidate.gamma, negative.gamma);
                assert_eq!(candidate.response, negative.response);
                Ok(fit.target
                    * 0.2
                    * DVec3::new(
                        (candidate.balance.warmth - truth.warmth).exp(),
                        1.,
                        (candidate.balance.tint - truth.tint).exp(),
                    ))
            },
            floor,
        )
        .unwrap();
    assert!((actual.warmth - truth.warmth).abs() < 1e-8);
    assert!((actual.tint - truth.tint).abs() < 1e-8);
}
