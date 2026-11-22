//! Gaussian image-response model and its input neighbourhood.
use crate::processing_support::Support;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageMtfParameters {
    pub sigma: f64,
}

impl ImageMtfParameters {
    pub fn support(self, pitch: [f64; 2]) -> Support {
        Support(crate::gaussian::support(self.sigma, pitch))
    }
}
