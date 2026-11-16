//! Black-anchored generic positive entry into RGB-ordered native codes.
use super::{Variant, require};
use crate::{
    Error,
    looks::characteristic::{self, Characteristic},
};
use glam::DVec3;

#[derive(Clone, Debug)]
pub(super) struct Entry {
    matrix: [[f64; 3]; 3],
    codes_per_stop: f64,
    curve: Characteristic,
    channel_gains: [f64; 3],
    origin: f64,
}
impl Entry {
    pub(super) fn calibrated(variant: Variant) -> Self {
        Self {
            matrix: [
                [
                    1.282289562775313,
                    -0.24594354514113376,
                    -0.03634601763417918,
                ],
                [
                    -0.2653420950705695,
                    1.2556186905489233,
                    0.009723404521646214,
                ],
                [
                    -0.07093156157153224,
                    -0.07084708062746652,
                    1.1417786421989988,
                ],
            ],
            codes_per_stop: 73.78531879357067,
            curve: *characteristic::reference(),
            channel_gains: [1.0328716504309126, 1., 1.0760157025406742],
            origin: variant.code_offset(),
        }
    }
    fn raw_code(&self, signal: f64) -> f64 {
        self.origin + self.codes_per_stop / std::f64::consts::LN_2 * self.curve.coordinate(signal)
    }
    fn code(&self, signal: f64) -> f64 {
        let code = self.raw_code(signal);
        if code >= 1015. {
            1023. - 64. / (code - 1007.)
        } else {
            code
        }
    }
    pub(super) fn coordinates(&self, rgb: DVec3) -> Result<DVec3, Error> {
        // Row sums are one; placing the matrix before gains preserves the
        // neutral trajectory established by the exposure/white-balance fit.
        let value = super::matrix(self.matrix) * rgb;
        require(value.is_finite(), "nonfinite Frontier formation")?;
        let signal = super::gamut(value).max(DVec3::ZERO) * DVec3::from_array(self.channel_gains);
        require(signal.is_finite(), "nonfinite Frontier proxy exposure")?;
        Ok(signal.map(|value| self.code(value)))
    }
}

#[cfg(test)]
mod tests;
