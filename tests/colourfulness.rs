use antitone::{
    adjustment_layers::{AdjustmentLayer, AdjustmentLayerSettings, AdjustmentLayerStack},
    geometry::SourcePoint,
    photographic::Grade,
};
use glam::DVec3;
#[path = "support/photographic_reference.rs"]
mod reference;

fn apply(rgb: DVec3, saturation: f64, vibrance: f64) -> DVec3 {
    Grade::new(AdjustmentLayerSettings {
        saturation,
        vibrance,
        ..Default::default()
    })
    .unwrap()
    .apply(rgb)
    .unwrap()
}

fn near(a: DVec3, b: DVec3) {
    assert!(
        (a - b).abs().max_element() < 3e-11 * (1. + b.abs().max_element()),
        "{a:?} != {b:?}"
    );
}

#[test]
fn saturation_scales_oklab_opponents_and_not_lightness() {
    for rgb in [
        DVec3::new(0.22, 0.18, 0.14),
        DVec3::new(0.001, 0.03, 0.2),
        DVec3::new(4., -0.02, 0.5),
        DVec3::new(-0.2, -0.01, -0.4),
    ] {
        let before = reference::to_oklab(rgb);
        for saturation in [0., 0.25, 0.7, 1., 1.3, 2.] {
            near(
                reference::to_oklab(apply(rgb, saturation, 0.)),
                DVec3::new(before.x, saturation * before.y, saturation * before.z),
            );
        }
        for vibrance in [-1., -0.5, 0., 0.5, 1.] {
            near(
                reference::to_oklab(apply(rgb, 0., vibrance)),
                DVec3::new(before.x, 0., 0.),
            );
        }
    }
}

#[test]
fn vibrance_is_an_independent_hue_neutral_preference_for_muted_colours() {
    for hue in [0_f64, 0.7, 2., 3.5, 5.] {
        let lab = |c| DVec3::new(0.6, c * hue.cos(), c * hue.sin());
        for vibrance in [-1., -0.5, 0.5, 1.] {
            let mut previous_chroma = -1.;
            let mut previous_gain = if vibrance > 0. { 2. } else { 0. };
            for i in 1..=1000 {
                // Keep cone roots away from zero: this checks the chroma mapping,
                // not cube-root conditioning for extreme imaginary colours.
                let chroma = 0.3 * f64::from(i) / 1000.;
                let rgb = reference::from_oklab(lab(chroma));
                let before = reference::to_oklab(rgb);
                let after = reference::to_oklab(apply(rgb, 1., vibrance));
                let out_chroma = after.y.hypot(after.z);
                let gain = out_chroma / chroma;
                assert!((after.x - before.x).abs() < 1e-11);
                assert!(
                    (after.y * before.z - after.z * before.y).abs() < 1e-11,
                    "hue={hue}, v={vibrance}, C={chroma}: {before:?} -> {after:?}, rgb={rgb:?}"
                );
                assert!(
                    out_chroma > previous_chroma,
                    "chroma mapping must not fold over"
                );
                if vibrance > 0. {
                    assert!(gain > 1. && gain < previous_gain);
                } else {
                    assert!(gain < 1. && gain > previous_gain);
                }
                previous_chroma = out_chroma;
                previous_gain = gain;
            }
        }
    }
    let rgb = reference::from_oklab(DVec3::new(0.6, 0.03, -0.02));
    let layer = AdjustmentLayer::global(AdjustmentLayerSettings {
        vibrance: 1.,
        ..Default::default()
    });
    let out = AdjustmentLayerStack::new(vec![layer])
        .unwrap()
        .apply_rgb(rgb, SourcePoint([0.5, 0.5]))
        .unwrap();
    near(out, apply(rgb, 1., 1.));
    assert!(
        (out - rgb).abs().max_element() > 0.01,
        "standalone Vibrance must not be culled"
    );
}

#[test]
fn colourfulness_is_exposure_homogeneous_and_keeps_signed_neutrals_exact() {
    for saturation in [0., 0.7, 1., 2.] {
        for vibrance in [-1., 0., 1.] {
            for x in [-16., -0.18, -1e-20, -0., 0., 1e-20, 0.18, 16.] {
                let rgb = DVec3::splat(x);
                assert_eq!(apply(rgb, saturation, vibrance), rgb);
            }
            for rgb in [
                DVec3::new(0.22, 0.18, 0.14),
                DVec3::new(4., -0.02, 0.5),
                DVec3::new(-0.2, -0.01, -0.4),
            ] {
                let baseline = apply(rgb, saturation, vibrance);
                for ev in [-30_f64, -10., 10., 30.] {
                    let gain = ev.exp2();
                    near(apply(gain * rgb, saturation, vibrance) / gain, baseline);
                }
            }
        }
    }
}

#[test]
fn smooth_rgb_gradients_have_matching_slopes_across_neutral_and_channel_ties() {
    for g in [-4_f64, -0.18, 0.001, 0.18, 4.] {
        for centre in [DVec3::splat(g), DVec3::new(g, g, 0.5 * g)] {
            for direction in [
                DVec3::new(1., 0., -1.),
                DVec3::new(1., -1., 0.),
                DVec3::new(0., 1., -1.),
            ] {
                let h = g.abs() * 1e-5;
                for saturation in [0., 0.7, 1., 2.] {
                    for vibrance in [-1., 0., 1.] {
                        let middle = apply(centre, saturation, vibrance);
                        let left = apply(centre - h * direction, saturation, vibrance);
                        let right = apply(centre + h * direction, saturation, vibrance);
                        let slope_difference = (right + left - 2. * middle) / h;
                        assert!(
                            slope_difference.abs().max_element() < 0.003,
                            "cusp: {centre:?}, {direction:?}, s={saturation}, v={vibrance}: {slope_difference:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn vibrance_domain_includes_both_directions_and_rejects_nonfinite_values() {
    for vibrance in [-1., 0., 1.] {
        assert!(
            AdjustmentLayerSettings {
                vibrance,
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }
    for vibrance in [-1.000001, 1.000001, f64::NAN, f64::INFINITY] {
        assert!(
            AdjustmentLayerSettings {
                vibrance,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
