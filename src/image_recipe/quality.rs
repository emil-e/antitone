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
    #[cfg(feature = "editor")]
    Measurement,
}

#[cfg(feature = "gpu")]
impl Quality {
    pub fn includes_repairs(self) -> bool {
        match self {
            Self::Originals => true,
            #[cfg(feature = "editor")]
            Self::PhysicalView => true,
            _ => false,
        }
    }
}

/// Source-relative operation support needed before camera resources are acquired.
pub(crate) fn sampling_stages(
    recipe: &ImageRecipe,
) -> Result<crate::pixel_pipeline::PixelPipeline, Error> {
    let pipeline = crate::pixel_pipeline::PixelPipeline::emulation(
        recipe.grain,
        recipe.halation,
        recipe.composition.crop().size,
        recipe.neutral.source.0,
    )?;
    #[cfg(all(test, feature = "editor"))]
    let pipeline = testing::append(pipeline);
    Ok(pipeline)
}

#[cfg(all(test, feature = "editor"))]
pub(crate) mod testing {
    use crate::pixel_pipeline::{PixelPipeline, PixelStage};
    use std::cell::RefCell;

    thread_local! {
        static SUFFIX: RefCell<Option<PixelStage>> = const { RefCell::new(None) };
    }

    pub(super) fn append(mut pipeline: PixelPipeline) -> PixelPipeline {
        SUFFIX.with(|slot| pipeline.stages.extend(slot.borrow().clone()));
        pipeline
    }

    pub(crate) fn with_suffix<T>(stage: PixelStage, author: impl FnOnce() -> T) -> T {
        struct Restore(Option<PixelStage>);
        impl Drop for Restore {
            fn drop(&mut self) {
                SUFFIX.with(|slot| slot.replace(self.0.take()));
            }
        }
        let previous = SUFFIX.with(|slot| slot.replace(Some(stage)));
        let _restore = Restore(previous);
        author()
    }
}
