//! Sampling policy and acquisition needs derived from complete photographic intent.

use super::ImageRecipe;
use crate::Error;

#[cfg(feature = "gpu")]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Quality {
    Originals,
    #[cfg(feature = "editor")]
    PhysicalView,
    CameraProxy,
}

#[cfg(feature = "gpu")]
impl Quality {
    pub fn includes_repairs(self) -> bool {
        self != Self::CameraProxy
    }
}

/// Source-relative operation support needed before camera resources are acquired.
pub(crate) fn sampling_stages(
    recipe: &ImageRecipe,
) -> Result<crate::pixel_pipeline::PixelPipeline, Error> {
    crate::pixel_pipeline::PixelPipeline::emulation(
        recipe.grain,
        recipe.halation,
        recipe.composition.crop().size,
        recipe.neutral.source.0,
    )
}
