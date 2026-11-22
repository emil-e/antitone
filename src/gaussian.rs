//! Raster support for the shared four-sigma Gaussian kernel.

pub(crate) const RADIUS_SIGMAS: f32 = 4.;

pub(crate) fn support(sigma: f64, pitch: [f64; 2]) -> [u32; 2] {
    pitch.map(|pitch| (RADIUS_SIGMAS * sigma as f32 / pitch as f32).ceil() as u32)
}
