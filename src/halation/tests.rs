use super::*;
use reference::apply;

#[test]
fn impulse_matches_authored_channel_fractions_and_discrete_taps() {
    // sigma makes the unnormalized Gaussian taps exactly 2^(-i^2), truncated at +/-4.
    let sigma = 1. / (2. * 2_f64.ln()).sqrt();
    let normalization: f64 = 1. + 2. * (0.5 + 1. / 16. + 1. / 512. + 1. / 65536.);
    let center_weight = 1. / normalization.powi(2);
    let mut input = vec![[0.; 4]; 121];
    input[60] = [1., 2., 4., 13.];
    let output = apply(&input, [11, 11], [sigma; 2], 0.2, 0);
    for (c, alpha) in [1. / 6., 14. / 139., 0.].into_iter().enumerate() {
        let peak = input[60][c];
        assert!((output[60][c] - peak * (1. - alpha + alpha * center_weight)).abs() < 1e-14);
        assert!((output[61][c] - peak * alpha * center_weight * 0.5).abs() < 1e-14);
        assert_eq!(output[65][c], 0.);
        assert!((output.iter().map(|p| p[c]).sum::<f64>() - peak).abs() < 1e-14);
    }
    assert_eq!(output[60][3], 13.);
}

#[test]
fn constant_signed_fields_and_zero_strength_are_identity() {
    for pixel in [[0.; 4], [0.18, 2., 64., 7.], [-2., -0.1, 4., 1.]] {
        let input = vec![pixel; 35];
        for strength in [0., 0.2, 2.] {
            let output = apply(&input, [7, 5], [1.3, 0.6], strength, 0);
            for p in output {
                for c in 0..4 {
                    assert!((p[c] - pixel[c]).abs() < 1e-13);
                }
            }
        }
    }
}

#[test]
fn negative_light_is_retained_but_cannot_seed_a_halo() {
    let mut input = vec![[0.; 4]; 81];
    input[40] = [-4., -2., -1., 3.];
    assert_eq!(apply(&input, [9, 9], [1.; 2], 0.2, 0), input);
    input[39] = [8., 4., 2., 0.];
    let output = apply(&input, [9, 9], [1.; 2], 0.2, 0);
    for c in 0..2 {
        assert!(output[40][c] > input[40][c]);
        assert!(output[40][c] < 0.);
    }
    for (actual, original) in output.iter().zip(&input) {
        assert_eq!(actual[2], original[2], "blue light is not redistributed");
    }
}

#[test]
fn scale_and_support_use_declared_raster_pitch() {
    let parameters = HalationParameters {
        sigma: 7.5,
        strength: 0.2,
    };
    assert_eq!(parameters.support([1.; 2]), Support([30; 2]));
    assert_eq!(parameters.support([2., 4.]), Support([15, 8]));
    assert_eq!(
        HalationParameters {
            strength: 0.,
            ..parameters
        }
        .support([1.; 2]),
        Support::NONE
    );
    for (actual, expected) in parameters
        .fractions()
        .into_iter()
        .zip([1. / 6., 14. / 139., 0.])
    {
        assert!((actual - expected).abs() < 1e-16);
    }
}

#[test]
fn rejects_nonfinite_or_negative_model_parameters() {
    for (sigma, strength) in [
        (0., 0.2),
        (-1., 0.2),
        (f64::INFINITY, 0.2),
        (1., -0.1),
        (1., f64::NAN),
    ] {
        assert!(HalationParameters { sigma, strength }.validate().is_err());
    }
}
