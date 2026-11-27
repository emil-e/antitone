use glam::{DMat3, DVec3, Vec3};

use crate::{Error, photographic::stages::display_rendering::validate_channel};

#[cfg(all(feature = "gpu", any(feature = "raw", feature = "ui")))]
pub(crate) mod execution;
#[cfg(all(feature = "gpu", feature = "raw"))]
pub(crate) mod rgb16;

pub use crate::color::OutputTarget;

impl OutputTarget {
    pub fn parameters(self) -> [f64; 9] {
        // Pinned target definitions; reference builds independently check the native parameters.
        match self {
            Self::Srgb => [100., 0.64, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290],
            Self::DisplayP3 => [100., 0.68, 0.32, 0.265, 0.69, 0.15, 0.06, 0.3127, 0.3290],
            Self::Rec2020_500 | Self::Rec2020_1000 | Self::Rec2020_2000 | Self::Rec2020_4000 => [
                f64::from(self.peak_nits()),
                0.708,
                0.292,
                0.170,
                0.797,
                0.131,
                0.046,
                0.3127,
                0.3290,
            ],
        }
    }

    /// ACES display-linear units are 100 cd/m² per unit, not fractions of peak.
    pub fn linear_peak(self) -> f64 {
        f64::from(self.peak_nits()) / 100.
    }

    pub fn rgb_to_xyz(self) -> DMat3 {
        let p = self.parameters();
        let xyz = |x: f64, y: f64| DVec3::new(x / y, 1.0, (1.0 - x - y) / y);
        let primaries = DMat3::from_cols(xyz(p[1], p[2]), xyz(p[3], p[4]), xyz(p[5], p[6]));
        primaries * DMat3::from_diagonal(primaries.inverse() * xyz(p[7], p[8]))
    }

    pub fn linear_to_srgb(self) -> DMat3 {
        if self == Self::Srgb {
            return DMat3::IDENTITY;
        }
        let source_white = self.rgb_to_xyz() * DVec3::ONE;
        let destination = Self::Srgb.rgb_to_xyz();
        let destination_white = destination * DVec3::ONE;
        let bradford = crate::color::BRADFORD;
        destination.inverse()
            * bradford.inverse()
            * DMat3::from_diagonal((bradford * destination_white) / (bradford * source_white))
            * bradford
            * self.rgb_to_xyz()
    }

    pub fn encode(self, linear: f64) -> Result<f64, Error> {
        validate_channel(linear, self.linear_peak())?;
        if self.is_hdr() {
            return Ok(pq_encode(linear / PQ_PEAK_LINEAR));
        }
        Ok(if linear <= 0.0031308 {
            12.92 * linear
        } else {
            1.055 * linear.powf(1.0 / 2.4) - 0.055
        })
    }

    pub fn decode(self, encoded: f64) -> Result<f64, Error> {
        validate_channel(encoded, 1.)?;
        if self.is_hdr() {
            return Ok(pq_decode(encoded) * PQ_PEAK_LINEAR);
        }
        Ok(if encoded <= 0.04045 {
            encoded / 12.92
        } else {
            ((encoded + 0.055) / 1.055).powf(2.4)
        })
    }

    pub fn encode_u16(self, linear: &[Vec3]) -> Result<Vec<[u16; 3]>, Error> {
        linear
            .iter()
            .map(|pixel| {
                let mut encoded = [0; 3];
                for (channel, value) in pixel.to_array().into_iter().enumerate() {
                    encoded[channel] = (self.encode(f64::from(value))? * 65535.0).round() as u16;
                }
                Ok(encoded)
            })
            .collect()
    }
}

// ST 2084's 10000 nit reference ceiling in ACES display-linear units.
const PQ_PEAK_LINEAR: f64 = 100.;
const M1: f64 = 2610. / 16384.;
const M2: f64 = 2523. / 32.;
const C1: f64 = 3424. / 4096.;
const C2: f64 = 2413. / 128.;
const C3: f64 = 2392. / 128.;

fn pq_encode(luminance_fraction: f64) -> f64 {
    let power = luminance_fraction.powf(M1);
    ((C1 + C2 * power) / (1. + C3 * power)).powf(M2)
}

// Decode to a fraction of 10000 nits, including for an ICC profile shared by all HDR peaks.
pub(crate) fn pq_decode(encoded: f64) -> f64 {
    let power = encoded.powf(1. / M2);
    ((power - C1).max(0.) / (C2 - C3 * power)).powf(1. / M1)
}

#[cfg(feature = "ocio-reference")]
pub use crate::photographic::stages::display_rendering::{
    CpuRenderer, ocio_version, reference_parameters,
};
