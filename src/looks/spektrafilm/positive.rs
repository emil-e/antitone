//! Reference-negative paper entry with calibrated neutral exposure.
mod reference;
use super::{data, neutral_exposure};
use crate::{Error, looks::require};
use glam::DVec3;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Driver {
    pub input_luminance: DVec3,
    reference: reference::Reference,
}

impl Driver {
    pub fn validate(&self) -> Result<(), Error> {
        require(
            self.input_luminance.is_finite()
                && self.input_luminance.min_element() > 0.
                && (self.input_luminance.element_sum() - 1.).abs() < 1e-10,
            "invalid paper reference luminance",
        )?;
        self.reference.validate(self.input_luminance)
    }

    pub fn expose(
        &self,
        rgb: DVec3,
        paper: &data::Paper,
        mapping: &data::Mapping,
    ) -> Result<DVec3, Error> {
        if rgb.x == rgb.y && rgb.y == rgb.z {
            // Preserve the entire neutral input law, including signed and HDR values.
            return Ok(neutral_exposure(paper, rgb.x, mapping));
        }
        let brightness = self.input_luminance.dot(rgb);
        let neutral = neutral_exposure(paper, brightness, mapping);
        Ok(neutral + self.reference.colour_difference(rgb, brightness)?)
    }
}

#[cfg(test)]
mod tests;
