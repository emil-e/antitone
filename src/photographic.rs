//! Signed AP1 photographic grade; ACEScct is a temporary contrast coordinate.
use crate::{Error, color::AP1_TO_XYZ, processing::PhotoSettings};
use glam::DVec3;

pub const CONTRACT: &str = "ap1-acescct-rgb-black-relative-contrast-y-max-saturation-blends-v1";

const TOE: f64 = 0.0078125;
const SLOPE: f64 = 10.5402377416545;
const OFFSET: f64 = 0.0729055341958355;
const CUT: f64 = 0.155251141552511;
const LOG_SCALE: f64 = 17.52 * std::f64::consts::LN_2;

pub(crate) fn encode(x: f64) -> f64 {
    if x <= TOE {
        SLOPE * x + OFFSET
    } else {
        (x.log2() + 9.72) / 17.52
    }
}
fn decode(q: f64) -> f64 {
    // No ACEScct interchange clip: working RGB stays signed/HDR.
    if q <= CUT {
        (q - OFFSET) / SLOPE
    } else {
        (q * 17.52 - 9.72).exp2()
    }
}
fn finite(rgb: DVec3) -> Result<DVec3, Error> {
    if rgb.is_finite() {
        Ok(rgb)
    } else {
        Err(Error::NumericalOverflow)
    }
}
fn mix(a: DVec3, b: DVec3, t: f64) -> DVec3 {
    if t == 0. || a == b {
        a
    } else if t == 1. {
        b
    } else {
        (1. - t) * a + t * b
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Grade {
    photo: PhotoSettings,
    pub(crate) qp: f64,
    pub(crate) q0: f64,
    pub(crate) black: f64,
}
impl Grade {
    pub fn new(photo: PhotoSettings) -> Result<Self, Error> {
        for (value, low, high, name) in [
            (photo.contrast, 0.4, 1.6, "contrast must be within 0.4..1.6"),
            (photo.pivot, 0.01, 1., "pivot must be within 0.01..1"),
            (
                photo.contrast_colour_mix,
                0.,
                1.,
                "contrast colour preservation must be within 0..1",
            ),
            (photo.saturation, 0., 2., "saturation must be within 0..2"),
            (
                photo.saturation_density,
                0.,
                1.,
                "saturation density must be within 0..1",
            ),
        ] {
            if !value.is_finite() || !(low..=high).contains(&value) {
                return Err(Error::InvalidParameter(name));
            }
        }
        let qp = encode(photo.pivot);
        let q0 = qp + photo.contrast * (OFFSET - qp);
        Ok(Self {
            photo,
            qp,
            q0,
            black: decode(q0),
        })
    }
    fn tone(&self, x: f64) -> f64 {
        if x == self.photo.pivot {
            x
        } else {
            decode(self.qp + self.photo.contrast * (encode(x) - self.qp))
        }
    }
    fn gain(&self, y: f64) -> f64 {
        let a = self.photo.contrast;
        let delta = a * if y <= TOE {
            SLOPE * y
        } else {
            encode(y) - OFFSET
        };
        // Stable (f(Y)-f(0))/Y, including the analytic zero-Y derivative.
        if y == 0. {
            if self.q0 <= CUT {
                a
            } else {
                self.black * LOG_SCALE * a * SLOPE
            }
        } else if self.q0 <= CUT && self.q0 + delta <= CUT {
            if y <= TOE { a } else { delta / (SLOPE * y) }
        } else if self.q0 > CUT && self.q0 + delta > CUT {
            self.black * (delta * LOG_SCALE).exp_m1() / y
        } else {
            // Integrate across the continuous toe. Subtracting decoded values
            // turns cancellation/rounded spec constants into a zero-Y gain jump.
            let to_cut = CUT - self.q0;
            if self.q0 <= CUT {
                (to_cut / SLOPE + decode(CUT) * ((delta - to_cut) * LOG_SCALE).exp_m1()) / y
            } else {
                (self.black * (to_cut * LOG_SCALE).exp_m1() + (delta - to_cut) / SLOPE) / y
            }
        }
    }
    #[cfg(feature = "gpu")]
    pub(crate) fn intermediate_bound(&self, input: f64) -> f64 {
        let mut peak = input;
        let mut output = input;
        if self.photo.contrast != 1. {
            let a = self.photo.contrast;
            // The secant gain is bounded by the largest derivative on [-B,B].
            // Derivative extrema occur at the linear toe or the positive endpoint;
            // a decode-branch crossing is continuous and introduces no new maximum.
            let derivative = |x: f64| {
                let q = self.qp + a * (encode(x) - self.qp);
                let e = if x <= TOE {
                    SLOPE
                } else {
                    1. / (LOG_SCALE * x)
                };
                let d = if q <= CUT {
                    1. / SLOPE
                } else {
                    LOG_SCALE * decode(q)
                };
                a * e * d
            };
            let gain = a.max(derivative(input.min(TOE))).max(derivative(input));
            output = self
                .tone(-input)
                .abs()
                .max(self.tone(input).abs())
                .max(self.black.abs() + gain * input);
            peak = peak.max(SLOPE * input * a.max(1.)).max(output);
        }
        if self.photo.saturation != 1. {
            // Include chroma subtraction, scaled chroma and grey addition, not
            // only the final blended output. AP1 Y is a positive unit-sum row.
            peak = peak
                .max(2. * output)
                .max((1. + 2. * self.photo.saturation) * output);
        }
        peak
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn weighted_bounds(&self, input: f64, strength: f64) -> (f64, f64, f64) {
        let mut peak = input;
        let mut output = input;
        let mut sensitivity = 1.;
        if self.photo.contrast != 1. {
            let end = 1. + strength * (self.photo.contrast - 1.);
            let low = end.min(1.);
            let high = end.max(1.);
            let curve = |amount| Self {
                photo: PhotoSettings {
                    contrast: amount,
                    ..self.photo
                },
                qp: self.qp,
                q0: self.qp + amount * (OFFSET - self.qp),
                black: decode(self.qp + amount * (OFFSET - self.qp)),
            };
            let mut gain = high;
            let mut black: f64 = 0.;
            for amount in [low, high] {
                let grade = curve(amount);
                black = black.max(grade.black.abs());
                output = output
                    .max(grade.tone(-input).abs())
                    .max(grade.tone(input).abs());
            }
            // For each amount, derivative extrema in x are at the toe or B.
            // In the exponential decode branch, derivative versus amount is
            // proportional to a*exp(k*a): include its interior stationary point.
            for x in [input.min(TOE), input] {
                let k = LOG_SCALE * (encode(x) - self.qp);
                let stationary = if k < 0. { -1. / k } else { high };
                for amount in [low, high, stationary.clamp(low, high)] {
                    let grade = curve(amount);
                    let q = grade.qp + amount * (encode(x) - grade.qp);
                    let e = if x <= TOE {
                        SLOPE
                    } else {
                        1. / (LOG_SCALE * x)
                    };
                    let d = if q <= CUT {
                        1. / SLOPE
                    } else {
                        LOG_SCALE * decode(q)
                    };
                    gain = gain.max(amount * e * d);
                }
            }
            output = output.max(black + gain * input);
            peak = peak.max(SLOPE * input * high).max(output);
            // Bound f'' on each encode/decode branch. In the positive curved
            // branch f'' is proportional to x^(a-2), decreasing for a <= 1.6.
            let q_toe = [low, high]
                .map(|a| self.qp + a * (encode(TOE) - self.qp))
                .into_iter()
                .fold(f64::NEG_INFINITY, f64::max);
            let curved_toe = (q_toe * 17.52 - 9.72).exp2();
            let curvature = (high * high * SLOPE * SLOPE * LOG_SCALE * LOG_SCALE * curved_toe)
                .max(high / (SLOPE * LOG_SCALE * TOE * TOE))
                .max(high * (low - 1.).abs().max((high - 1.).abs()) * curved_toe / (TOE * TOE));
            // g'(Y) = integral_0^1 t*f''(tY) dt. For increased contrast,
            // f'' is zero below the toe, negative in log-encode/linear-decode,
            // then positive in log-encode/exponential-decode. The latter two
            // contributions oppose rather than add; keep the narrow crossing
            // interval instead of applying its peak curvature to all of [-B,B].
            let gain_slope = if low >= 1. {
                let cut =
                    (self.qp + (CUT - self.qp) / high) * LOG_SCALE - 9.72 * std::f64::consts::LN_2;
                let width = (cut - TOE.ln())
                    .max(0.)
                    .min((input / TOE).ln().max(0.))
                    .min(0.5);
                let logarithmic =
                    high / (SLOPE * LOG_SCALE * TOE * TOE) * width * (-2. * width).exp();
                let curved = high * (high - 1.) * curved_toe / (2. * TOE * TOE);
                // Rounded ACEScct constants can put a tiny exponential-decode
                // interval below the encode toe, including arbitrarily near 1.
                let start = ((CUT - curve(low).q0) / (low * SLOPE)).clamp(0., TOE);
                let end = input.min(TOE);
                let toe_fraction = if end > start {
                    (end - start) * (end + start) / (end * end)
                } else {
                    0.
                };
                let linear_curved =
                    high * high * SLOPE * SLOPE * LOG_SCALE * LOG_SCALE * curved_toe * toe_fraction
                        / 2.;
                logarithmic.max(curved) + linear_curved
            } else {
                curvature / 2.
            };
            // AP1 Y has positive unit-sum weights; use the infinity norm.
            sensitivity = gain + self.photo.contrast_colour_mix * input * gain_slope;
        }
        if self.photo.saturation != 1. {
            let saturation = (1. + strength * (self.photo.saturation - 1.)).max(1.);
            peak = peak.max((1. + 2. * saturation) * output);
            let gain = (2. * saturation - 1.).max(1.);
            output *= gain;
            sensitivity *= gain;
        }
        (peak, output, sensitivity)
    }

    pub fn apply(&self, rgb: DVec3) -> Result<DVec3, Error> {
        finite(rgb)?;
        let rgb = if self.photo.contrast == 1. {
            rgb
        } else {
            let coupled = finite(rgb.map(|v| self.tone(v)))?;
            let proxy = finite(DVec3::splat(self.black) + self.gain((AP1_TO_XYZ * rgb).y) * rgb)?;
            finite(mix(coupled, proxy, self.photo.contrast_colour_mix))?
        };
        if self.photo.saturation == 1. || rgb.x == rgb.y && rgb.y == rgb.z {
            return Ok(rgb);
        }
        let grey = DVec3::splat((AP1_TO_XYZ * rgb).y);
        let maximum = DVec3::splat(rgb.max_element());
        let constant_y = finite(grey + self.photo.saturation * (rgb - grey))?;
        let hsv = finite(maximum + self.photo.saturation * (rgb - maximum))?;
        finite(mix(constant_y, hsv, self.photo.saturation_density))
    }
}

#[cfg(all(test, feature = "gpu"))]
mod bounds_tests {
    use super::*;

    #[test]
    fn increased_contrast_bounds_cover_signed_jacobians_at_toe_and_decode_crossings() {
        let weights = AP1_TO_XYZ.row(1);
        for contrast in [1. + 1e-12, 1.0001, 1.01, 1.6] {
            for pivot in [0.01, 0.18, 1.] {
                for input in [0.001, 0.008, 1., 20000.] {
                    let photo = PhotoSettings {
                        contrast,
                        pivot,
                        contrast_colour_mix: 1.,
                        ..Default::default()
                    };
                    let (_, _, bound) = Grade::new(photo).unwrap().weighted_bounds(input, 0.6);
                    for i in 1..=20 {
                        let amount = 1. + 0.6 * i as f64 / 20. * (contrast - 1.);
                        let grade = Grade::new(PhotoSettings {
                            contrast: amount,
                            ..photo
                        })
                        .unwrap();
                        let cut = decode(grade.qp + (CUT - grade.qp) / amount);
                        for y in [
                            0.,
                            TOE * (1. - 1e-12),
                            TOE,
                            TOE * (1. + 1e-12),
                            cut * (1. - 1e-12),
                            cut,
                            cut * (1. + 1e-12),
                            input,
                        ] {
                            if y > input {
                                continue;
                            }
                            let gain = grade.gain(y);
                            let delta = amount
                                * if y <= TOE {
                                    SLOPE * y
                                } else {
                                    encode(y) - OFFSET
                                };
                            let q = grade.q0 + delta;
                            let derivative = amount
                                * if y <= TOE {
                                    SLOPE
                                } else {
                                    1. / (LOG_SCALE * y)
                                }
                                * if q <= CUT {
                                    1. / SLOPE
                                } else {
                                    LOG_SCALE * decode(q)
                                };
                            let gain_slope = if y == 0. || (y <= TOE && q <= CUT) {
                                0.
                            } else {
                                (derivative - gain) / y
                            };
                            for red in [-0.8 * input, 0.8 * input] {
                                let rgb = DVec3::new(red, (y - weights.x * red) / weights.y, 0.);
                                if rgb.abs().max_element() > input {
                                    continue;
                                }
                                for row in 0..3 {
                                    let mut jacobian = gain_slope * rgb[row] * weights;
                                    jacobian[row] += gain;
                                    assert!(
                                        jacobian.abs().element_sum() <= bound * (1. + 1e-10),
                                        "contrast={amount}, pivot={pivot}, B={input}, Y={y}: {jacobian:?} exceeds {bound}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
