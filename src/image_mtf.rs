//! Gaussian image-response model and its input neighbourhood.
use crate::processing_support::Support;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageMtfParameters {
    pub sigma: f64,
}

impl ImageMtfParameters {
    pub fn support(self, pitch: [f64; 2]) -> Support {
        Support(pitch.map(|p| (4. * self.sigma as f32 / p as f32).ceil() as u32))
    }
}
