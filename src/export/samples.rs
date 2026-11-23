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
    Rgb8,
    Rgb16,
    Float(FloatPrecision),
}

pub(crate) enum Samples {
    Rgb8(Vec<[u8; 3]>),
    Rgb16(Vec<[u16; 3]>),
    F16(Vec<[f16; 3]>),
    F32(Vec<[f32; 3]>),
}

#[cfg(any(test, all(feature = "gpu", feature = "raw")))]
pub(crate) fn pack_rgb8(
    target: crate::rendering::OutputTarget,
    pixels: impl IntoIterator<Item = Result<DVec3, Error>>,
) -> Result<Vec<[u8; 3]>, Error> {
    pixels
        .into_iter()
        .map(|pixel| {
            let [r, g, b] = pixel?.to_array();
            let encode =
                |value| -> Result<u8, Error> { Ok((target.encode(value)? * 255.).round() as u8) };
            Ok([encode(r)?, encode(g)?, encode(b)?])
        })
        .collect()
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

#[cfg(test)]
#[test]
fn rgb8_applies_target_transfer_before_rounding() {
    for target in [
        crate::rendering::OutputTarget::Srgb,
        crate::rendering::OutputTarget::DisplayP3,
    ] {
        let samples = pack_rgb8(
            target,
            [
                Ok(DVec3::new(0.001, 0.18, 1.)),
                Ok(DVec3::new(0., 0.5, 0.002)),
            ],
        )
        .unwrap();
        assert_eq!(samples, [[3, 118, 255], [0, 188, 7]]);
        assert!(pack_rgb8(target, [Ok(DVec3::splat(1.1))]).is_err());
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
