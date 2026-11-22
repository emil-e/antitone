//! Pixel layouts and terminal conversion, independent of file encoding.
use crate::{Error, color::ap1_to_ap0};
use glam::{DMat3, DVec3};
use half::f16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FloatPrecision {
    Half,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layout {
    Rgb16,
    Float(FloatPrecision),
}

pub(crate) enum Samples {
    Rgb16(Vec<[u16; 3]>),
    F16(Vec<[f16; 3]>),
    F32(Vec<[f32; 3]>),
}

/// Scene-linear export uses AP0/D60. Keep the matrix calculation in f64 so finite
/// reconstructed f32 inputs can always be converted before narrowing to storage.
pub(crate) struct SceneEncoding {
    conversion: DMat3,
    precision: FloatPrecision,
}

impl SceneEncoding {
    pub(crate) fn new(precision: FloatPrecision) -> Self {
        Self {
            conversion: ap1_to_ap0(),
            precision,
        }
    }

    pub(crate) fn pack(
        &self,
        pixels: impl IntoIterator<Item = Result<DVec3, Error>>,
    ) -> Result<Samples, Error> {
        let pixels = pixels
            .into_iter()
            .map(|pixel| pixel.map(|rgb| self.conversion * rgb));
        match self.precision {
            FloatPrecision::Half => {
                pack(pixels, f16::MAX.to_f64(), f16::from_f64).map(Samples::F16)
            }
            FloatPrecision::Full => {
                pack(pixels, f64::from(f32::MAX), |v| v as f32).map(Samples::F32)
            }
        }
    }
}

fn pack<T>(
    pixels: impl Iterator<Item = Result<DVec3, Error>>,
    limit: f64,
    narrow: impl Fn(f64) -> T,
) -> Result<Vec<[T; 3]>, Error> {
    pixels
        .map(|pixel| {
            let channels = pixel?.to_array();
            if let Some(channel) = channels.iter().position(|v| !v.is_finite()) {
                return Err(Error::NonFinitePixel { channel });
            }
            Ok(channels.map(|v| narrow(v.clamp(-limit, limit))))
        })
        .collect()
}
