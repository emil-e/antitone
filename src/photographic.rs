//! Signed AP1 photographic grade and ACEScct look-interchange helpers.
#[cfg(feature = "project")]
mod acescct;
mod contrast;

#[cfg(feature = "project")]
pub(crate) use acescct::{decode, encode};

use crate::{Error, adjustment_layers::AdjustmentLayerSettings, color::oklab};
use glam::DVec3;

fn finite(rgb: DVec3) -> Result<DVec3, Error> {
    if rgb.is_finite() {
        Ok(rgb)
    } else {
        Err(Error::NumericalOverflow)
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
            (photo.vibrance, -1., 1., "vibrance must be within -1..1"),
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
        let contrasted = if self.photo.contrast == 1. {
            rgb
        } else {
            finite(rgb.map(|v| contrast::tone(v, self.photo.contrast, self.photo.pivot, self.qp)))?
        };
        let preserve = if self.photo.contrast == 1. {
            0.
        } else {
            self.photo.contrast_colour_mix
        };
        if preserve == 0. && self.photo.saturation == 1. && self.photo.vibrance == 0.
            || rgb.x == rgb.y && rgb.y == rgb.z
        {
            return Ok(contrasted);
        }
        // Preserve and colourfulness share roots: an intermediate RGB round trip
        // loses precision near zero cone responses in signed working colours.
        finite(oklab::grade(
            rgb,
            contrasted,
            preserve,
            self.photo.saturation,
            self.photo.vibrance,
        ))
    }
}
