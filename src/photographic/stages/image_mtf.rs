//! Gaussian image-response model and its input neighbourhood.
use crate::processing_support::Support;

#[cfg(all(test, feature = "gpu"))]
pub(crate) mod shader_fixture;
#[cfg(all(test, feature = "editor"))]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageMtfParameters {
    pub sigma: f64,
}

impl ImageMtfParameters {
    #[cfg(feature = "gpu")]
    pub(crate) fn compose(
        self,
        input: crate::gpu::evaluation::Value,
    ) -> Result<crate::gpu::evaluation::Value, crate::Error> {
        crate::gpu::gaussian::blur(input, self.sigma)
    }

    pub fn support(self, pitch: [f64; 2]) -> Support {
        Support(crate::gaussian::support(self.sigma, pitch))
    }
}
