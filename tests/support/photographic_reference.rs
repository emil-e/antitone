//! Scalar mathematical checks for signed AP1 grading, not a photographic-output oracle.
use antitone::{
    Error,
    color::{AP1_TO_XYZ, BRADFORD},
};
use glam::{DMat3, DVec3};

pub type Result<T> = std::result::Result<T, Error>;
const FOOT: f64 = 1. / 32768.;
const ZERO: f64 = (9.72 - 16.) / 17.52;
const SLOPE: f64 = 32768. / (17.52 * std::f64::consts::LN_2);

// Published ACEScc positive branches, extended by their tangent below zero.
// https://docs.acescentral.com/encodings/acescc/
pub fn encode(x: f64) -> f64 {
    if x <= 0. {
        ZERO + SLOPE * x
    } else if x < FOOT {
        ((1. / 65536. + x / 2.).log2() + 9.72) / 17.52
    } else {
        (x.log2() + 9.72) / 17.52
    }
}

pub fn decode(q: f64) -> f64 {
    if q <= ZERO {
        (q - ZERO) / SLOPE
    } else if q < (9.72 - 15.) / 17.52 {
        2. * ((q * 17.52 - 9.72).exp2() - 1. / 65536.)
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

// Full XYZ/OKLab conversions keep the test calculation distinct from production's
// fused AP1 cone matrix and lightness-projected root difference.
// Matrices: https://www.w3.org/TR/css-color-4/#color-conversion-code
fn adaptation() -> DMat3 {
    let source = BRADFORD * (AP1_TO_XYZ * DVec3::ONE);
    let target = BRADFORD * DVec3::new(0.3127 / 0.3290, 1., 0.3583 / 0.3290);
    BRADFORD.inverse() * DMat3::from_diagonal(target / source) * BRADFORD
}

fn xyz_to_lms() -> DMat3 {
    DMat3::from_cols_array(&[
        0.819022442647055,
        0.0329836558550745,
        0.04817719382823899,
        0.3619062604571286,
        0.9292868602921457,
        0.2642395276821537,
        -0.12887378595068213,
        0.03614466586101669,
        0.6335478283072317,
    ])
}

fn roots_to_lab() -> DMat3 {
    DMat3::from_cols_array(&[
        0.21045426824930336,
        1.977998539071675,
        0.025904030547901084,
        0.7936177747759865,
        -2.4285922502018127,
        0.7827717270900503,
        -0.00407204302528986,
        0.4505937111301374,
        -0.8086757576379514,
    ])
}

pub fn to_oklab(rgb: DVec3) -> DVec3 {
    let xyz = adaptation() * (AP1_TO_XYZ * rgb);
    roots_to_lab() * (xyz_to_lms() * xyz).map(f64::cbrt)
}

pub fn from_oklab(lab: DVec3) -> DVec3 {
    let roots = roots_to_lab().inverse() * lab;
    let xyz = xyz_to_lms().inverse() * (roots * roots * roots);
    AP1_TO_XYZ.inverse() * (adaptation().inverse() * xyz)
}

#[derive(Clone, Copy, Debug)]
pub enum Contrast {
    Rgb,
    Oklab,
}

pub fn contrast(rgb: DVec3, amount: f64, pivot: f64, model: Contrast) -> Result<DVec3> {
    contrast_mix(
        rgb,
        amount,
        pivot,
        match model {
            Contrast::Rgb => 0.,
            Contrast::Oklab => 1.,
        },
    )
}

pub fn contrast_mix(rgb: DVec3, amount: f64, pivot: f64, preserve: f64) -> Result<DVec3> {
    finite(rgb)?;
    parameter(amount, 0.4, 2.)?;
    parameter(pivot, 0.01, 1.)?;
    parameter(preserve, 0., 1.)?;
    if amount == 1. {
        return Ok(rgb);
    }
    let contrasted = finite(rgb.map(|v| tone(v, amount, pivot)))?;
    if preserve == 0. || rgb.x == rgb.y && rgb.y == rgb.z {
        return Ok(contrasted);
    }
    let original = to_oklab(rgb);
    let graded = to_oklab(contrasted);
    finite(from_oklab(DVec3::new(
        graded.x,
        (1. - preserve) * graded.y + preserve * original.y,
        (1. - preserve) * graded.z + preserve * original.z,
    )))
}

pub fn saturation(rgb: DVec3, amount: f64, vibrance: f64) -> Result<DVec3> {
    finite(rgb)?;
    parameter(amount, 0., 2.)?;
    parameter(vibrance, -1., 1.)?;
    if amount == 1. && vibrance == 0. || rgb.x == rgb.y && rgb.y == rgb.z {
        return Ok(rgb);
    }
    let mut lab = to_oklab(rgb);
    let relative = lab.x * lab.x / 16.;
    let denominator = relative + lab.y * lab.y + lab.z * lab.z;
    let weight = if denominator == 0. {
        0.
    } else {
        relative / denominator
    };
    let gain = amount * (1. + vibrance * weight);
    lab.y *= gain;
    lab.z *= gain;
    finite(from_oklab(lab))
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
                        contrast(rgb, 1., 0.18, Contrast::Oklab).unwrap(),
                        t
                    )
                    .unwrap(),
                    rgb
                );
                assert_eq!(saturation(rgb, 1., 0.).unwrap(), rgb);
            }
        }
    }

    #[test]
    fn saturation_reference_preserves_oklab_lightness_and_hue() {
        for rgb in inputs() {
            let before = to_oklab(rgb);
            for amount in [0., 0.7, 1.3, 2.] {
                let after = to_oklab(saturation(rgb, amount, 0.).unwrap());
                near(
                    after,
                    DVec3::new(before.x, amount * before.y, amount * before.z),
                );
            }
        }
    }

    #[test]
    fn invalid_values_and_overflow_reject_without_clamps() {
        for amount in [f64::NAN, f64::INFINITY, -1., 3.] {
            assert!(contrast(DVec3::ONE, amount, 0.18, Contrast::Rgb).is_err());
            assert!(saturation(DVec3::ONE, amount, 0.).is_err());
            assert!(blend(DVec3::ZERO, DVec3::ONE, amount).is_err());
        }
        assert!(contrast(DVec3::splat(f64::MAX), 1.5, 0.18, Contrast::Rgb).is_err());
        assert!(saturation(DVec3::splat(f64::NAN), 1., 0.).is_err());
    }

    #[test]
    fn oklab_d65_white_and_neutrals_have_zero_opponents() {
        for x in [-1., 0., 0.18, 1., 16.] {
            let rgb = DVec3::splat(x);
            let lab = to_oklab(rgb);
            assert!((lab - DVec3::new(x.cbrt(), 0., 0.)).abs().max_element() < 2e-14);
            assert!((from_oklab(lab) - rgb).abs().max_element() < 1e-13);
        }
    }

    #[test]
    fn css_reference_srgb_red_fixture_matches_xyz_conversion() {
        // Fixture evaluated from CSS Color 4's reference conversion, including its
        // 64-bit matrix update; older rounded direct-sRGB matrices differ here.
        let xyz = DVec3::new(0.4123907992659595, 0.21263900587151036, 0.01933081871559185);
        let actual = roots_to_lab() * (xyz_to_lms() * xyz).map(f64::cbrt);
        let expected = DVec3::new(0.6279553645, 0.2248630710, 0.1258462751);
        assert!((actual - expected).abs().max_element() < 1e-10);
    }

    #[test]
    fn acescc_positive_codes_and_signed_extension_roundtrip() {
        assert_eq!(encode(0.), ZERO);
        assert!((encode(FOOT) - (9.72 - 15.) / 17.52).abs() < 1e-16);
        for x in [-1e10, -1., -1e-12, 0., 1e-12, FOOT, 0.18, 1., 1e10] {
            assert!((decode(encode(x)) - x).abs() < 2e-14 * (1. + x.abs()));
        }
    }

    #[test]
    fn contrast_endpoints_share_the_neutral_curve() {
        for x in [-1., 0., FOOT, 0.18, 16.] {
            for amount in [0.4, 1., 2.] {
                let rgb = DVec3::splat(x);
                assert_eq!(
                    contrast(rgb, amount, 0.18, Contrast::Rgb).unwrap(),
                    contrast(rgb, amount, 0.18, Contrast::Oklab).unwrap()
                );
            }
        }
    }
}
