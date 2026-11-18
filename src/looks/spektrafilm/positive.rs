//! Fixed printing of the calibrated paper-only reference negative.
mod reference;
use super::data;
use crate::{
    Error,
    looks::{failure, require},
};
use glam::DVec3;

pub(super) use reference::DEFINITION;

pub(super) struct Driver {
    reference: &'static reference::Reference,
    pair: &'static data::Pair,
    print_log_h: DVec3,
}

impl Driver {
    pub fn new(paper: &'static data::Paper) -> Result<Self, Error> {
        let reference = reference::reference()?;
        let pair = data::catalog()
            .pairs
            .iter()
            .find(|p| p.negative == reference.definition.negative && p.paper == paper.id)
            .ok_or_else(|| failure("missing paper-reference printing calibration"))?;
        let gray = crate::looks::characteristic::reference().gray;
        let print_log_h = paper.positive_gray_log_h - reference.expose(DVec3::splat(gray), pair);
        require(
            print_log_h.is_finite(),
            "invalid paper-reference printing balance",
        )?;
        Ok(Self {
            reference,
            pair,
            print_log_h,
        })
    }

    pub fn expose(&self, rgb: DVec3) -> Result<DVec3, Error> {
        let result = self.print_log_h + self.reference.expose(rgb, self.pair);
        require(result.is_finite(), "nonfinite paper-reference exposure")?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
