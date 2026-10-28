//! Ordered layer definitions. Recipe assembly selects effects; execution consumes their contracts.
use crate::{Error, film_emulation::FilmEmulationSettings, processing_support::Support};
use std::{any::Any, fmt, sync::Arc};
#[cfg(all(test, feature = "editor"))]
pub(crate) mod test_layer;
#[cfg(test)]
mod tests;

/// A captured image operation, independent of the consumer requesting its pixels.
pub trait Layer: Any + fmt::Debug + Send + Sync {
    fn support(&self, pitch: [f64; 2]) -> Support;

    #[cfg(all(feature = "gpu", feature = "render"))]
    fn compose(
        &self,
        input: Arc<crate::gpu::pull::Layer>,
    ) -> Result<Arc<crate::gpu::pull::Layer>, Error>;
}

/// Value identity keeps separately assembled equal recipes compatible without effect-specific keys.
#[derive(Clone)]
pub struct PixelStage {
    value: Arc<dyn Layer>,
    equals: fn(&dyn Layer, &dyn Layer) -> bool,
}
impl PixelStage {
    pub fn new<T: Layer + PartialEq>(value: T) -> Self {
        Self {
            value: Arc::new(value),
            equals: |a, b| {
                (a as &dyn Any).downcast_ref::<T>() == (b as &dyn Any).downcast_ref::<T>()
            },
        }
    }
    pub fn get<T: Layer>(&self) -> Option<&T> {
        (self.value.as_ref() as &dyn Any).downcast_ref()
    }
    pub fn support(&self, pitch: [f64; 2]) -> Support {
        self.value.support(pitch)
    }
    #[cfg(all(feature = "gpu", feature = "render"))]
    pub(crate) fn compose(
        &self,
        input: Arc<crate::gpu::pull::Layer>,
    ) -> Result<Arc<crate::gpu::pull::Layer>, Error> {
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
impl Layer for Grain {
    fn support(&self, pitch: [f64; 2]) -> Support {
        self.parameters.support(pitch)
    }
    #[cfg(all(feature = "gpu", feature = "render"))]
    fn compose(
        &self,
        input: Arc<crate::gpu::pull::Layer>,
    ) -> Result<Arc<crate::gpu::pull::Layer>, Error> {
        crate::gpu::pull::grain(input, self.parameters, self.seed)
    }
}
impl Layer for crate::image_mtf::ImageMtfParameters {
    fn support(&self, pitch: [f64; 2]) -> Support {
        Self::support(*self, pitch)
    }
    #[cfg(all(feature = "gpu", feature = "render"))]
    fn compose(
        &self,
        input: Arc<crate::gpu::pull::Layer>,
    ) -> Result<Arc<crate::gpu::pull::Layer>, Error> {
        crate::gpu::pull::gaussian(input, self.sigma)
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
        let Some(parameters) = settings.parameters(frame_size)? else {
            return Ok(Self::default());
        };
        let mut stages = vec![PixelStage::new(crate::image_mtf::ImageMtfParameters {
            sigma: parameters.image_mtf_sigma,
        })];
        if parameters.amplitude > 0. {
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
    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }
    #[cfg(all(feature = "gpu", feature = "render"))]
    pub(crate) fn compose(
        &self,
        mut input: Arc<crate::gpu::pull::Layer>,
    ) -> Result<Arc<crate::gpu::pull::Layer>, Error> {
        for stage in &self.stages {
            let domain = input.domain;
            input = stage.compose(input)?;
            if !domain.same_grid(input.domain) || domain.meaning != input.domain.meaning {
                return Err(Error::InvalidParameter(
                    "pixel layer changes its declared grid or sample meaning",
                ));
            }
        }
        Ok(input)
    }
}
