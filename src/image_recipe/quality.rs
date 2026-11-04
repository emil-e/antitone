//! Sampling policy and acquisition needs derived from complete photographic intent.
//! CPU repair planning uses the same support even when GPU execution is unavailable.

use super::ImageRecipe;
use crate::{
    Error,
    acquisition::{Plan, Request, Sampling, SourceSupport},
    frame_raster::FrameRaster,
};

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

pub(crate) fn original_plan(recipe: &ImageRecipe) -> Result<Plan, Error> {
    let stages = sampling_stages(recipe)?;
    Plan::new(Request {
        output: FrameRaster::full(recipe.composition)?,
        stages: &stages,
        sampling: Sampling::Originals {
            reconstruction: SourceSupport([0; 2]),
        },
    })
}
