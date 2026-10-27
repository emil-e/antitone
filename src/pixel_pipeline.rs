//! Ordered pixel stages and their spatial dependencies. Stages do not inspect one another.
use crate::{
    Error, film_emulation::FilmEmulationSettings, grain::GrainParameters,
    processing_support::Support,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PixelStage {
    Gaussian {
        sigma: f64,
    },
    Grain {
        parameters: GrainParameters,
        seed: u64,
    },
}

impl PixelStage {
    pub fn support(self, pitch: [f64; 2]) -> Support {
        match self {
            Self::Gaussian { sigma } => {
                crate::image_mtf::ImageMtfParameters { sigma }.support(pitch)
            }
            Self::Grain { parameters, .. } => parameters.support(pitch),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PixelPipeline {
    pub stages: Vec<PixelStage>,
}

impl PixelPipeline {
    pub fn film(settings: FilmEmulationSettings, height: u32, seed: u64) -> Result<Self, Error> {
        let Some(parameters) = settings.parameters(height)? else {
            return Ok(Self::default());
        };
        let stages = vec![
            PixelStage::Gaussian {
                sigma: parameters.image_mtf_sigma,
            },
            PixelStage::Grain {
                parameters: parameters.grain(),
                seed,
            },
        ];
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
}
