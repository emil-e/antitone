//! Pointwise exposure offsets for Shadows and Highlights.

use super::contrast;
use crate::color::AP1_TO_XYZ;
use glam::DVec3;

pub(crate) const MAX_EV: f64 = 3.;
pub(crate) const WIDTH: f64 = 5.;

pub(crate) fn encoded_grey() -> f64 {
    contrast::encode(0.18)
}

pub(super) fn offset(rgb: DVec3, shadows: f64, highlights: f64) -> f64 {
    let luminance = AP1_TO_XYZ.row(1).dot(rgb);
    let stops = contrast::SCALE * (contrast::encode(luminance.max(0.)) - encoded_grey());
    let amount = if stops < 0. { shadows } else { highlights };
    // Map each half-trace to [0, 1) so the cubic eases into its tail without a join.
    let t = stops.abs() / stops.hypot(WIDTH);
    amount * (t * t * (3. - 2. * t))
}
