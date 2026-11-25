//! Signed photographic tone and colour in AP1.

use super::{MAX_TONAL_EV, contrast, tonal_ranges};
use crate::color::roll_wb_matrix;
use crate::processing::{RollSettings, exposure_multiplier};
use crate::{Error, adjustment_layers::AdjustmentLayerSettings, color::oklab};
use glam::{DMat3, DVec3};
use std::ops::RangeInclusive;

pub(crate) const CONTRAST_RANGE: RangeInclusive<f64> = 0.4..=2.;

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
            (
                photo.contrast,
                *CONTRAST_RANGE.start(),
                *CONTRAST_RANGE.end(),
                "contrast must be within 0.4..2",
            ),
            (photo.pivot, 0.01, 1., "pivot must be within 0.01..1"),
            (
                photo.contrast_colour_mix,
                0.,
                1.,
                "tone colour preservation must be within 0..1",
            ),
            (
                photo.shadows,
                -MAX_TONAL_EV,
                MAX_TONAL_EV,
                "shadows must be within -3..3 EV",
            ),
            (
                photo.highlights,
                -MAX_TONAL_EV,
                MAX_TONAL_EV,
                "highlights must be within -3..3 EV",
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
        let ev = if self.photo.shadows == 0. && self.photo.highlights == 0. {
            0.
        } else {
            tonal_ranges::offset(rgb, self.photo.shadows, self.photo.highlights)
        };
        let mut toned = if ev == 0. {
            rgb
        } else {
            finite(rgb * ev.exp2())?
        };
        if self.photo.contrast != 1. {
            toned = finite(
                toned.map(|v| contrast::tone(v, self.photo.contrast, self.photo.pivot, self.qp)),
            )?;
        }
        let preserve = if ev != 0. || self.photo.contrast != 1. {
            self.photo.contrast_colour_mix
        } else {
            0.
        };
        if preserve == 0. && self.photo.saturation == 1. && self.photo.vibrance == 0.
            || rgb.x == rgb.y && rgb.y == rgb.z
        {
            return Ok(toned);
        }
        // Preserve and colourfulness share roots: an intermediate RGB round trip
        // loses precision near zero cone responses in signed working colours.
        finite(oklab::grade(
            rgb,
            toned,
            preserve,
            self.photo.saturation,
            self.photo.vibrance,
        ))
    }
}

/// Shared photographic balance/exposure on an already interpreted working positive.
#[derive(Debug, Clone)]
pub struct RollPipeline {
    transform: DMat3,
}

impl RollPipeline {
    pub fn new(roll: RollSettings) -> Result<Self, Error> {
        let transform = roll_wb_matrix(roll.wb)? * exposure_multiplier(roll.exposure)?;
        if !transform.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(Self { transform })
    }

    pub fn apply(&self, rgb: DVec3) -> Result<DVec3, Error> {
        let output = self.transform * rgb;
        if !output.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        Ok(output)
    }
}
