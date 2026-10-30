//! Ordered effect definitions. Recipe assembly selects effects; evaluation executes their nodes.

use crate::{Error, film_emulation::FilmEmulationSettings, processing_support::Support};
use std::{any::Any, fmt, sync::Arc};

// A cost/accuracy choice for view acquisition, not a native kernel parameter. Integral
// destination subdivisions further constrain the actual working pitch.
pub(crate) const GAUSSIAN_VIEW_CELLS: f64 = 0.6;

#[cfg(all(test, feature = "editor"))]
pub(crate) mod authoring_examples;

#[cfg(all(test, feature = "editor"))]
pub(crate) mod test_effects;

#[cfg(any(test, feature = "editor"))]
pub(crate) mod sampling;

#[cfg(test)]
mod tests;

/// Captured logical effect settings, independent of a particular input or consumer.
/// Composition applies the definition to an image node by constructing operation/result nodes.
pub trait EffectDefinition: Any + fmt::Debug + Send + Sync {
    fn support(&self, pitch: [f64; 2]) -> Support;

    /// Preferred maximum source units per cell for approximate spatial processing.
    /// Acquisition may bound refinement; this does not change native effect mathematics.
    fn view_pitch(&self) -> Option<f64> {
        None
    }

    /// Can consume an integrated destination footprint without upstream neighbours.
    fn integrates_footprint(&self) -> bool {
        false
    }

    #[cfg(feature = "gpu")]
    fn compose(
        &self,
        input: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error>;
}

/// Value identity keeps separately assembled equal recipes compatible without effect-specific keys.
#[derive(Clone)]
pub struct PixelStage {
    value: Arc<dyn EffectDefinition>,
    equals: fn(&dyn EffectDefinition, &dyn EffectDefinition) -> bool,
}

impl PixelStage {
    pub fn new<T: EffectDefinition + PartialEq>(value: T) -> Self {
        Self {
            value: Arc::new(value),
            equals: |a, b| {
                (a as &dyn Any).downcast_ref::<T>() == (b as &dyn Any).downcast_ref::<T>()
            },
        }
    }

    pub fn get<T: EffectDefinition>(&self) -> Option<&T> {
        (self.value.as_ref() as &dyn Any).downcast_ref()
    }

    pub fn support(&self, pitch: [f64; 2]) -> Support {
        self.value.support(pitch)
    }

    #[cfg(any(test, feature = "editor"))]
    pub(crate) fn view_pitch(&self) -> Option<f64> {
        self.value.view_pitch()
    }

    #[cfg(feature = "editor")]
    pub(crate) fn integrates_footprint(&self) -> bool {
        self.value.integrates_footprint()
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn compose(
        &self,
        input: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
        self.value.compose(input)
    }
}

impl fmt::Debug for PixelStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}

impl PartialEq for PixelStage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.value, &other.value)
            || (self.equals)(self.value.as_ref(), other.value.as_ref())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grain {
    pub parameters: crate::grain::GrainParameters,
    pub seed: u64,
}

impl EffectDefinition for Grain {
    fn integrates_footprint(&self) -> bool {
        true
    }

    fn support(&self, pitch: [f64; 2]) -> Support {
        self.parameters.support(pitch)
    }

    #[cfg(feature = "gpu")]
    fn compose(
        &self,
        input: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
        crate::gpu::evaluation::grain(input, self.parameters, self.seed)
    }
}

impl EffectDefinition for crate::image_mtf::ImageMtfParameters {
    fn view_pitch(&self) -> Option<f64> {
        Some(self.sigma / GAUSSIAN_VIEW_CELLS)
    }

    fn support(&self, pitch: [f64; 2]) -> Support {
        Self::support(*self, pitch)
    }

    #[cfg(feature = "gpu")]
    fn compose(
        &self,
        input: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
        crate::gpu::evaluation::gaussian(input, self.sigma)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PixelPipeline {
    pub stages: Vec<PixelStage>,
}

impl PixelPipeline {
    pub fn film(
        settings: FilmEmulationSettings,
        frame_size: [u32; 2],
        seed: u64,
    ) -> Result<Self, Error> {
        Self::emulation(settings, Default::default(), frame_size, seed)
    }

    pub fn emulation(
        film: FilmEmulationSettings,
        halation: crate::halation::HalationSettings,
        frame_size: [u32; 2],
        seed: u64,
    ) -> Result<Self, Error> {
        let film = film.parameters(frame_size)?;
        let mut stages = Vec::new();
        if let Some(parameters) = film {
            stages.push(PixelStage::new(crate::image_mtf::ImageMtfParameters {
                sigma: parameters.image_mtf_sigma,
            }));
        }
        if let Some(parameters) = halation.parameters(frame_size)? {
            stages.push(PixelStage::new(parameters));
        }
        if let Some(parameters) = film.filter(|p| p.amplitude > 0.) {
            stages.push(PixelStage::new(Grain {
                parameters: parameters.grain(),
                seed,
            }));
        }
        Ok(Self { stages })
    }

    pub fn support(&self, pitch: [f64; 2]) -> Result<Support, Error> {
        if pitch.into_iter().any(|p| !p.is_finite() || p <= 0.) {
            return Err(Error::InvalidParameter("invalid processing raster pitch"));
        }
        self.stages
            .iter()
            .try_fold(Support::NONE, |support, stage| {
                support.then(stage.support(pitch))
            })
    }

    #[cfg(feature = "editor")]
    pub(crate) fn footprint_suffix(&self) -> usize {
        self.stages
            .iter()
            .rposition(|stage| !stage.integrates_footprint())
            .map_or(0, |index| index + 1)
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    #[cfg(feature = "gpu")]
    pub(crate) fn compose(
        &self,
        mut input: Arc<crate::gpu::evaluation::ImageNode>,
    ) -> Result<Arc<crate::gpu::evaluation::ImageNode>, Error> {
        for stage in &self.stages {
            let domain = input.domain;
            input = stage.compose(input)?;
            if !domain.same_grid(input.domain) || domain.meaning != input.domain.meaning {
                return Err(Error::InvalidParameter(
                    "effect changes its declared grid or sample meaning",
                ));
            }
        }
        Ok(input)
    }
}
