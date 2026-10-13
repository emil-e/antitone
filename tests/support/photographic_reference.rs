//! Independent f64 endpoint and blend oracle for signed-AP1 Photo grading.
use antitone::{Error, color::AP1_TO_XYZ};
use glam::DVec3;

pub type Result<T> = std::result::Result<T, Error>;
const TOE: f64 = 0.0078125;
const SLOPE: f64 = 10.5402377416545;
const OFFSET: f64 = 0.0729055341958355;
const CUT: f64 = 0.155251141552511;
const LOG_SCALE: f64 = 17.52 * std::f64::consts::LN_2;

pub fn y(rgb: DVec3) -> f64 {
    (AP1_TO_XYZ * rgb).y
}

pub fn encode(x: f64) -> f64 {
    if x <= TOE {
        SLOPE * x + OFFSET
    } else {
        (x.log2() + 9.72) / 17.52
    }
}

// Deliberately no ACEScct interchange high-end clipping: this is a temporary
// photographic coordinate, with overflow rejected by the public operators.
pub fn decode(q: f64) -> f64 {
    if q <= CUT {
        (q - OFFSET) / SLOPE
    } else {
        (q * 17.52 - 9.72).exp2()
    }
}

fn finite(rgb: DVec3) -> Result<DVec3> {
    if rgb.is_finite() {
        Ok(rgb)
    } else {
        Err(Error::NumericalOverflow)
    }
}

fn parameter(value: f64, low: f64, high: f64) -> Result<()> {
    if value.is_finite() && (low..=high).contains(&value) {
        Ok(())
    } else {
        Err(Error::InvalidParameter("outside declared reference range"))
    }
}

pub fn tone(x: f64, amount: f64, pivot: f64) -> f64 {
    if amount == 1. || x == pivot {
        x
    } else {
        let qp = encode(pivot);
        decode(qp + amount * (encode(x) - qp))
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Contrast {
    Rgb,
    BlackOffsetRatios,
}

pub fn contrast(rgb: DVec3, amount: f64, pivot: f64, model: Contrast) -> Result<DVec3> {
    finite(rgb)?;
    parameter(amount, 0.4, 1.6)?;
    parameter(pivot, 0.01, 1.)?;
    if amount == 1. {
        return Ok(rgb);
    }
    match model {
        Contrast::Rgb => finite(rgb.map(|v| tone(v, amount, pivot))),
        Contrast::BlackOffsetRatios => {
            let luminance = y(rgb);
            let qp = encode(pivot);
            let q0 = qp + amount * (OFFSET - qp);
            let black = decode(q0);
            let delta = amount
                * if luminance <= TOE {
                    SLOPE * luminance
                } else {
                    encode(luminance) - OFFSET
                };
            // Divided difference (f(Y)-f(0))/Y, evaluated without a near-zero
            // epsilon, gain cap or chromatic-pixel fallback.
            let gain = if luminance == 0. {
                if q0 <= CUT {
                    amount
                } else {
                    black * LOG_SCALE * amount * SLOPE
                }
            } else if q0 <= CUT && q0 + delta <= CUT {
                if luminance <= TOE {
                    amount
                } else {
                    delta / (SLOPE * luminance)
                }
            } else if q0 > CUT && q0 + delta > CUT {
                black * (delta * LOG_SCALE).exp_m1() / luminance
            } else {
                // Stable integral across the continuous decode toe; rounded
                // constants must not become a gain jump at vanishing Y.
                let to_cut = CUT - q0;
                if q0 <= CUT {
                    (to_cut / SLOPE + decode(CUT) * ((delta - to_cut) * LOG_SCALE).exp_m1())
                        / luminance
                } else {
                    (black * (to_cut * LOG_SCALE).exp_m1() + (delta - to_cut) / SLOPE) / luminance
                }
            };
            finite(DVec3::splat(black) + gain * rgb)
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Saturation {
    ConstantY,
    Maximum,
}

pub fn saturation(rgb: DVec3, amount: f64, model: Saturation) -> Result<DVec3> {
    finite(rgb)?;
    parameter(amount, 0., 2.)?;
    if amount == 1. || rgb.x == rgb.y && rgb.y == rgb.z {
        return Ok(rgb);
    }
    let out = match model {
        Saturation::ConstantY => {
            let grey = DVec3::splat(y(rgb));
            grey + amount * (rgb - grey)
        }
        Saturation::Maximum => {
            let anchor = DVec3::splat(rgb.max_element());
            anchor + amount * (rgb - anchor)
        }
    };
    finite(out)
}

pub fn blend(a: DVec3, b: DVec3, t: f64) -> Result<DVec3> {
    finite(a)?;
    finite(b)?;
    parameter(t, 0., 1.)?;
    if t == 0. || a == b {
        Ok(a)
    } else if t == 1. {
        Ok(b)
    } else {
        finite((1. - t) * a + t * b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: DVec3, b: DVec3) {
        assert!(
            (a - b).abs().max_element() <= 2e-12 * (1. + b.abs().max_element()),
            "{a:?} != {b:?}"
        );
    }

    fn inputs() -> Vec<DVec3> {
        let mut out = vec![DVec3::new(1., -0.4, 0.02), DVec3::new(-1., -0.2, -0.01)];
        let w = AP1_TO_XYZ.transpose().y_axis;
        for l in [-1., -1e-12, 0., 1e-12, 0.0078125, 0.18, 1., 16., 1e10] {
            out.push(DVec3::splat(l));
            out.push(DVec3::new(l + 0.02, l - 0.02 * w.x / w.y, l));
        }
        out
    }

    #[test]
    fn identity_is_exact_for_endpoints_and_blends() {
        for rgb in inputs() {
            for t in [0., 0.25, 0.5, 0.75, 1.] {
                assert_eq!(
                    blend(
                        contrast(rgb, 1., 0.18, Contrast::Rgb).unwrap(),
                        contrast(rgb, 1., 0.18, Contrast::BlackOffsetRatios).unwrap(),
                        t
                    )
                    .unwrap(),
                    rgb
                );
                assert_eq!(
                    blend(
                        saturation(rgb, 1., Saturation::ConstantY).unwrap(),
                        saturation(rgb, 1., Saturation::Maximum).unwrap(),
                        t
                    )
                    .unwrap(),
                    rgb
                );
            }
        }
    }

    #[test]
    fn encoding_roundtrips_signed_to_hdr_without_high_end_clip() {
        for rgb in inputs() {
            near(rgb.map(|v| decode(encode(v))), rgb);
        }
        assert!(decode(encode(65504. * 4.)) > 65504.);
    }

    #[test]
    fn common_neutral_response_pivot_and_monotonicity() {
        for amount in [0.4, 0.5, 0.8, 1., 1.2, 1.5, 1.6] {
            for pivot in [0.01, 0.18, 1.] {
                let mut previous = f64::NEG_INFINITY;
                for x in [-1., -1e-12, 0., 1e-12, TOE, 0.18, 1., 16.] {
                    let expected = DVec3::splat(tone(x, amount, pivot));
                    near(
                        contrast(DVec3::splat(x), amount, pivot, Contrast::BlackOffsetRatios)
                            .unwrap(),
                        expected,
                    );
                    near(
                        contrast(DVec3::splat(x), amount, pivot, Contrast::Rgb).unwrap(),
                        expected,
                    );
                    assert!(expected.x > previous);
                    previous = expected.x;
                }
                near(
                    contrast(
                        DVec3::splat(pivot),
                        amount,
                        pivot,
                        Contrast::BlackOffsetRatios,
                    )
                    .unwrap(),
                    DVec3::splat(pivot),
                );
            }
        }
        assert!((tone(0., 1.2, 0.18) + 0.00646442474348078).abs() < 1e-14);
    }

    #[test]
    fn ratio_proxy_tracks_y_and_preserves_black_relative_chromatic_direction() {
        for rgb in inputs() {
            for amount in [0.4, 0.5, 0.8, 1.2, 1.5, 1.6] {
                let out = contrast(rgb, amount, 0.18, Contrast::BlackOffsetRatios).unwrap();
                let shifted = out - DVec3::splat(tone(0., amount, 0.18));
                let scale = rgb.abs().max_element().max(1.);
                near(
                    (shifted / shifted.abs().max_element().max(1.)).cross(rgb / scale),
                    DVec3::ZERO,
                );
                assert!((y(out) - tone(y(rgb), amount, 0.18)).abs() < 1e-11 * (1. + y(out).abs()));
            }
        }
    }

    #[test]
    fn saturation_invariants_and_signed_continuity() {
        for rgb in inputs() {
            for amount in [0.25, 0.7, 1.3, 2.] {
                let cy = saturation(rgb, amount, Saturation::ConstantY).unwrap();
                assert!((y(cy) - y(rgb)).abs() < 2e-12 * (1. + rgb.abs().max_element()));
                let mx = saturation(rgb, amount, Saturation::Maximum).unwrap();
                let scale = rgb.abs().max_element().max(1.);
                near(
                    (mx - DVec3::splat(y(mx))) / scale,
                    amount * (rgb - DVec3::splat(y(rgb))) / scale,
                );
            }
        }
        for model in [Saturation::ConstantY, Saturation::Maximum] {
            for x in [0., 0.2, -0.2] {
                let a = saturation(DVec3::new(x - 1e-10, 0.2, -0.2), 0.7, model).unwrap();
                let b = saturation(DVec3::new(x + 1e-10, 0.2, -0.2), 0.7, model).unwrap();
                assert!((a - b).abs().max_element() < 1e-6);
            }
        }
    }

    #[test]
    fn blends_are_independent_endpoint_output_interpolation() {
        let rgb = DVec3::new(0.22, 0.18, 0.14);
        for t in [0., 0.25, 0.5, 0.75, 1.] {
            let a = saturation(rgb, 1.3, Saturation::ConstantY).unwrap();
            let b = saturation(rgb, 1.3, Saturation::Maximum).unwrap();
            near(blend(a, b, t).unwrap(), (1. - t) * a + t * b);
        }
    }

    #[test]
    fn contrast_is_continuous_at_zero_y_and_encoding_boundaries() {
        let base = DVec3::new(0.2, -0.1, 0.01);
        let zero_y = base - DVec3::splat(y(base));
        for amount in [0.4, 0.5, 0.75, 0.8, 1.2, 1.5, 1.6] {
            for pivot in [0.01, 0.18, 1.] {
                let a = contrast(
                    zero_y - DVec3::splat(1e-11),
                    amount,
                    pivot,
                    Contrast::BlackOffsetRatios,
                )
                .unwrap();
                let b = contrast(
                    zero_y + DVec3::splat(1e-11),
                    amount,
                    pivot,
                    Contrast::BlackOffsetRatios,
                )
                .unwrap();
                assert!((a - b).abs().max_element() < 1e-8);
                let boundary = decode(encode(pivot) + (CUT - encode(pivot)) / amount);
                for x in [TOE, boundary] {
                    let epsilon = 1e-11 * (1. + x.abs());
                    for model in [Contrast::Rgb, Contrast::BlackOffsetRatios] {
                        let a = contrast(DVec3::splat(x - epsilon), amount, pivot, model).unwrap();
                        let b = contrast(DVec3::splat(x + epsilon), amount, pivot, model).unwrap();
                        assert!((a - b).abs().max_element() < 1e-8);
                    }
                }
            }
        }
    }

    #[test]
    fn desaturation_uses_each_endpoint_anchor() {
        let rgb = DVec3::new(0.3, 0.18, 0.08);
        near(
            saturation(rgb, 0., Saturation::ConstantY).unwrap(),
            DVec3::splat(y(rgb)),
        );
        near(
            saturation(rgb, 0., Saturation::Maximum).unwrap(),
            DVec3::splat(rgb.max_element()),
        );
    }

    #[test]
    fn invalid_values_and_overflow_reject_without_clamps() {
        for amount in [f64::NAN, f64::INFINITY, -1., 3.] {
            assert!(contrast(DVec3::ONE, amount, 0.18, Contrast::Rgb).is_err());
            assert!(saturation(DVec3::ONE, amount, Saturation::Maximum).is_err());
            assert!(blend(DVec3::ZERO, DVec3::ONE, amount).is_err());
        }
        assert!(contrast(DVec3::splat(f64::MAX), 1.5, 0.18, Contrast::Rgb).is_err());
        assert!(saturation(DVec3::splat(f64::NAN), 1., Saturation::ConstantY).is_err());
    }
}
