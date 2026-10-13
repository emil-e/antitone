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
