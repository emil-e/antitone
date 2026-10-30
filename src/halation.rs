//! Constant-field-preserving, threshold-free redistribution of positive AP1 light.
use crate::{Error, processing_support::Support};
mod settings;
pub use settings::*;

/// AP1 colour balance tuned with Medium image MTF; not spectral layer sensitivities.
pub const CHANNEL_RATIO: [f64; 3] = [1., 0.56, 0.];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HalationParameters {
    /// Gaussian sigma in original-source pixels, resolved from the accepted crop.
    pub sigma: f64,
    /// Unnormalized red coefficient k; the redistributed fraction is k / (1 + k).
    pub strength: f64,
}

impl HalationParameters {
    pub fn validate(self) -> Result<(), Error> {
        if !self.sigma.is_finite()
            || self.sigma <= 0.
            || !self.strength.is_finite()
            || self.strength < 0.
        {
            return Err(Error::InvalidParameter("invalid halation parameters"));
        }
        Ok(())
    }

    pub fn fractions(self) -> [f64; 3] {
        CHANNEL_RATIO.map(|ratio| {
            let k = self.strength * ratio;
            k / (1. + k)
        })
    }

    pub fn support(self, pitch: [f64; 2]) -> Support {
        if self.strength == 0. {
            Support::NONE
        } else {
            // Match the actual f32 kernel pitch, not a rounded affine-vector length.
            Support(pitch.map(|p| (4. * (self.sigma as f32 / p as f32)).ceil() as u32))
        }
    }
}

impl crate::pixel_pipeline::EffectDefinition for HalationParameters {
    fn view_pitch(&self) -> Option<f64> {
        (self.strength > 0.).then_some(self.sigma / crate::pixel_pipeline::GAUSSIAN_VIEW_CELLS)
    }

    fn support(&self, pitch: [f64; 2]) -> Support {
        Self::support(*self, pitch)
    }

    #[cfg(feature = "gpu")]
    fn compose(
        &self,
        input: std::sync::Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<std::sync::Arc<crate::gpu::evaluation::ImageNode>, Error> {
        crate::gpu::evaluation::halation(input, *self)
    }
}

#[cfg(all(
    test,
    target_os = "macos",
    feature = "editor",
    feature = "ocio-reference"
))]
mod export_tests;

#[cfg(test)]
pub(crate) mod reference;
#[cfg(test)]
mod tests;
