//! Positive-entry formation into RGB-ordered native density/code coordinates.
use super::{Variant, require};
use crate::Error;
use glam::DVec3;

#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub matrix: [[f64; 3]; 3],
    pub codes_per_stop: f64,
    pub black_code: f64,
    pub knee: f64,
    pub toe: f64,
    pub channel_offsets: [f64; 3],
    pub entry_width: f64,
}
impl Entry {
    pub(super) fn calibrated(variant: Variant) -> Self {
        // Supplied negative-mode recentering uses 450 for A and 512 for B.
        // Translate the A-calibrated formation before its upper code limit.
        let origin = match variant {
            Variant::A => 450.,
            Variant::B => 512.,
        };
        Self {
            matrix: [
                [
                    1.2931615073725646,
                    -0.2443091742997006,
                    -0.048852333072864115,
                ],
                [
                    -0.27344530677611845,
                    1.28732324207888,
                    -0.013877935302761787,
                ],
                [
                    -0.044913617986590146,
                    -0.08435370450480469,
                    1.129267322491395,
                ],
            ],
            codes_per_stop: 67.75451223731619,
            black_code: 206.3977579662653 + (origin - 450.),
            knee: 0.02665880423285846,
            toe: 2.5,
            channel_offsets: [0., 15.816032029689438, 15.3959029772098],
            entry_width: 8.,
        }
    }
    pub(crate) fn code(&self, signal: f64, channel: usize) -> f64 {
        let base = self.black_code + self.channel_offsets[channel];
        if signal <= 0. {
            return base;
        }
        let z = self.toe * (signal.ln() - self.knee.ln());
        let softplus = z.max(0.) + (-z.abs()).exp().ln_1p();
        let code = base + self.codes_per_stop / (self.toe * std::f64::consts::LN_2) * softplus;
        let start = 1023. - self.entry_width;
        if code >= start {
            1023. - self.entry_width / (1. + (code - start) / self.entry_width)
        } else {
            code
        }
    }
}

impl Entry {
    pub(crate) fn coordinates(&self, rgb: DVec3) -> Result<DVec3, Error> {
        let value = super::matrix(self.matrix) * rgb;
        require(value.is_finite(), "nonfinite Frontier formation")?;
        let signal = super::gamut(value).max(DVec3::ZERO);
        Ok(DVec3::new(
            self.code(signal.x, 1),
            self.code(signal.y, 0),
            self.code(signal.z, 2),
        ))
    }
}
