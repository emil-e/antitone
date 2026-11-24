//! Photographic stages, grading mathematics and graph composition.
#[cfg(feature = "project")]
mod acescct;
mod contrast;
#[cfg(feature = "gpu")]
pub mod graph;
pub mod stages;
mod tonal_ranges;

pub(crate) use tonal_ranges::MAX_EV as MAX_TONAL_EV;
#[cfg(feature = "gpu")]
pub(crate) use tonal_ranges::{WIDTH as TONAL_WIDTH, encoded_grey};

#[cfg(feature = "project")]
pub(crate) use acescct::{decode, encode};

use crate::{Error, adjustment_layers::AdjustmentLayerSettings, color::oklab};
use glam::DVec3;
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
