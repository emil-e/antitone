use super::pooled::Pooled;
use super::*;

/// Physical sides and their provenance stay distinct from the source-visible crop.
#[derive(Clone, Serialize)]
pub(super) struct Choice {
    pub(super) method: &'static str,
    pub(super) physical: PictureGeometry,
    pub(super) measured: [bool; 4],
    pub(super) inferred: [bool; 4],
    pub(super) crop: Crop,
    pub(super) support: [Support; 4],
    pub(super) pooled: [Option<Pooled>; 4],
    pub(super) inferred_band_raster: Option<[f64; 2]>,
    pub(super) source_clipped: bool,
    pub(super) outside_base_score: f64,
    pub(super) signal_score: f64,
}
