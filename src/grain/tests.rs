use super::*;
use crate::film_emulation::{DEFAULT_SIZE, FilmEmulationSettings, MAX_SIZE, MIN_SIZE};

#[test]
fn pixel_integration_attenuates_intrinsic_green_contrast() {
    // Gaussian PSD integrated against the Gaussian aperture, divided by point-field
    // green power. Values are independent of the production covariance evaluator.
    for (scale, expected) in [
        (0.3, 0.16881786211226144),
        (1., 0.5848674913191082),
        (2., 0.823751868185381),
        (3.5, 0.9309745498835328),
    ] {
        let model = GrainParameters {
            grain_size: scale,
            amplitude: 0.055,
            colour: 0.5,
        };
        assert!((model.channel_variance(1.)[1] - expected).abs() < 1e-12);
        assert!(model.channel_variance(4.)[1] < expected);
        assert_eq!(
            model.channel_variance(0.25),
            model.channel_variance(1.),
            "magnification retains original-pixel integration"
        );
    }
}

#[test]
fn fixed_frame_footprint_covariance_is_independent_of_native_resolution() {
    for size in [MIN_SIZE, DEFAULT_SIZE, MAX_SIZE] {
        for colour in [0., 0.5, 1.] {
            for output in [128, 512, 2048] {
                let mut reference = None;
                for native in [2048, 3000, 4096, 5500, 8192] {
                    let model = FilmEmulationSettings {
                        enabled: true,
                        size,
                        colour,
                        ..Default::default()
                    }
                    .parameters([native; 2])
                    .unwrap()
                    .unwrap();
                    let covariance = model.covariance(f64::from(native) / f64::from(output));
                    if let Some(expected) = reference {
                        let expected: [[f64; 3]; 3] = expected;
                        for r in 0..3 {
                            for c in 0..3 {
                                assert!(
                                    (covariance[r][c] - expected[r][c]).abs()
                                        < 1e-12 * expected[r][c].abs().max(1e-12),
                                    "size={size}, colour={colour}, output={output}, native={native}, ({r},{c})"
                                );
                            }
                        }
                    } else {
                        reference = Some(covariance);
                    }
                }
            }
        }
    }
}
