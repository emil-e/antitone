//! Signed AP1 photographic grade; ACEScct is a temporary contrast coordinate.
use crate::{Error, adjustment_layers::AdjustmentLayerSettings, color::AP1_TO_XYZ};
use glam::DVec3;

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

pub(crate) fn decode(q: f64) -> f64 {
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

fn mix(
    a: impl FnOnce() -> Result<DVec3, Error>,
    b: impl FnOnce() -> Result<DVec3, Error>,
    t: f64,
) -> Result<DVec3, Error> {
    if t == 0. {
        return a();
    }
    if t == 1. {
        return b();
    }
    let a = a()?;
    let b = b()?;
    if a == b {
        Ok(a)
    } else {
        finite((1. - t) * a + t * b)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Grade {
    photo: AdjustmentLayerSettings,
    pub(crate) qp: f64,
    pub(crate) q0: f64,
    pub(crate) black: f64,
}

impl Grade {
    pub fn new(photo: AdjustmentLayerSettings) -> Result<Self, Error> {
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

    pub fn apply(&self, rgb: DVec3) -> Result<DVec3, Error> {
        finite(rgb)?;
        let rgb = if self.photo.contrast == 1. {
            rgb
        } else {
            mix(
                || finite(rgb.map(|v| self.tone(v))),
                || finite(DVec3::splat(self.black) + self.gain((AP1_TO_XYZ * rgb).y) * rgb),
                self.photo.contrast_colour_mix,
            )?
        };
        if self.photo.saturation == 1. || rgb.x == rgb.y && rgb.y == rgb.z {
            return Ok(rgb);
        }
        mix(
            || {
                let grey = DVec3::splat((AP1_TO_XYZ * rgb).y);
                finite(grey + self.photo.saturation * (rgb - grey))
            },
            || {
                let maximum = DVec3::splat(rgb.max_element());
                finite(maximum + self.photo.saturation * (rgb - maximum))
            },
            self.photo.saturation_density,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_endpoints_skip_the_unselected_branch() {
        let rgb = DVec3::new(4., -0.5, 0.25);
        assert_eq!(
            mix(|| Ok(rgb), || panic!("unselected endpoint"), 0.).unwrap(),
            rgb
        );
        assert_eq!(
            mix(|| panic!("unselected endpoint"), || Ok(rgb), 1.).unwrap(),
            rgb
        );
    }

    #[test]
    fn interior_blends_mix_both_endpoints_and_propagate_errors() {
        let a = DVec3::new(-1., 0., 2.);
        let b = DVec3::new(3., 4., 6.);
        assert_eq!(
            mix(|| Ok(a), || Ok(b), 0.25).unwrap(),
            DVec3::new(0., 1., 3.)
        );
        assert_eq!(
            mix(|| Err(Error::NumericalOverflow), || Ok(b), 0.5),
            Err(Error::NumericalOverflow)
        );
        assert_eq!(
            mix(|| Ok(a), || Err(Error::NumericalOverflow), 0.5),
            Err(Error::NumericalOverflow)
        );
    }
}
