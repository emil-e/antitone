use glam::{DMat3, DVec3, Vec3};

use crate::{Error, photographic::stages::display_rendering::validate_channel};

pub use crate::color::OutputTarget;

impl OutputTarget {
    pub fn parameters(self) -> [f64; 9] {
        // Pinned target definitions; reference builds independently check the native parameters.
        match self {
            Self::Srgb => [100., 0.64, 0.33, 0.30, 0.60, 0.15, 0.06, 0.3127, 0.3290],
            Self::DisplayP3 => [100., 0.68, 0.32, 0.265, 0.69, 0.15, 0.06, 0.3127, 0.3290],
        }
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
        validate_channel(linear)?;
        Ok(match self {
            Self::Srgb | Self::DisplayP3 if linear <= 0.0031308 => 12.92 * linear,
            Self::Srgb | Self::DisplayP3 => 1.055 * linear.powf(1.0 / 2.4) - 0.055,
        })
    }

    pub fn decode(self, encoded: f64) -> Result<f64, Error> {
        validate_channel(encoded)?;
        Ok(match self {
            Self::Srgb | Self::DisplayP3 if encoded <= 0.04045 => encoded / 12.92,
            Self::Srgb | Self::DisplayP3 => ((encoded + 0.055) / 1.055).powf(2.4),
        })
    }

    pub fn encode_u16(self, linear: &[Vec3]) -> Result<Vec<[u16; 3]>, Error> {
        self.encode_u16_pixels(linear.iter().copied())
    }

    pub(crate) fn encode_u16_pixels(
        self,
        linear: impl IntoIterator<Item = Vec3>,
    ) -> Result<Vec<[u16; 3]>, Error> {
        linear
            .into_iter()
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

#[cfg(feature = "ocio-reference")]
pub use crate::photographic::stages::display_rendering::{
    CpuRenderer, ocio_version, reference_parameters,
};
