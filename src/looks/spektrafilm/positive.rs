//! Reference-negative printing with fixed per-medium exposure and colour balance.
mod reference;
use crate::{Error, looks::require};
use glam::DVec3;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Driver {
    pub input_luminance: DVec3,
    reference: reference::Reference,
    print_log_h: DVec3,
}

impl Driver {
    pub fn validate(&self) -> Result<(), Error> {
        require(
            self.input_luminance.is_finite()
                && self.input_luminance.min_element() > 0.
                && (self.input_luminance.element_sum() - 1.).abs() < 1e-10
                && self.print_log_h.is_finite(),
            "invalid paper reference calibration",
        )?;
        self.reference.validate(self.input_luminance)
    }

    pub fn expose(&self, rgb: DVec3) -> Result<DVec3, Error> {
        let brightness = self.input_luminance.dot(rgb);
        Ok(self.print_log_h + self.reference.expose(rgb, brightness)?)
    }
}

#[cfg(test)]
mod tests;
