//! Signed AP1 photographic grade and ACEScct look-interchange helpers.
#[cfg(feature = "project")]
mod acescct;
mod contrast;

#[cfg(feature = "project")]
pub(crate) use acescct::{decode, encode};

use crate::{
    Error,
    adjustment_layers::AdjustmentLayerSettings,
    color::{AP1_TO_XYZ, oklab},
};
use glam::DVec3;

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
        Ok(Self {
            photo,
            qp: contrast::encode(photo.pivot),
        })
    }

    pub fn apply(&self, rgb: DVec3) -> Result<DVec3, Error> {
        finite(rgb)?;
        let rgb = if self.photo.contrast == 1. {
            rgb
        } else {
            let contrasted = finite(
                rgb.map(|v| contrast::tone(v, self.photo.contrast, self.photo.pivot, self.qp)),
            )?;
            if self.photo.contrast_colour_mix == 0. || rgb.x == rgb.y && rgb.y == rgb.z {
                contrasted
            } else {
                finite(oklab::preserve(
                    rgb,
                    contrasted,
                    self.photo.contrast_colour_mix,
                ))?
            }
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
