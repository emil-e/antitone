//! Frontier source model and provenance, resolved to the common LUT during preparation.
use super::{LookAsset, LookOrigin, authoring as baking, failure, require, spektrafilm};
use crate::Error;
use glam::DVec3;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

#[cfg(feature = "ocio-reference")]
pub mod authoring;
mod density;
mod negative;
mod positive;
mod readout;
#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    A,
    B,
}
impl Variant {
    pub fn all() -> impl Iterator<Item = Self> {
        [Self::A, Self::B].into_iter()
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::A => "Frontier A",
            Self::B => "Frontier B",
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::A => "frontier_a",
            Self::B => "frontier_b",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub negative: Option<spektrafilm::Negative>,
    pub variant: Variant,
}
impl Selection {
    pub fn name(self) -> String {
        self.negative.map_or_else(
            || self.variant.name().into(),
            |stock| format!("{} · {}", stock.name(), self.variant.name()),
        )
    }
    pub fn origin(self) -> LookOrigin {
        static COMMON: OnceLock<String> = OnceLock::new();
        let common = COMMON.get_or_init(|| {
            let mut hash = blake3::Hasher::new();
            for source in [
                include_bytes!("frontier.rs").as_slice(),
                include_bytes!("frontier/readout.rs").as_slice(),
                readout::DEFINITION,
                include_bytes!("frontier/density.rs").as_slice(),
                include_bytes!("authoring.rs").as_slice(),
                include_bytes!("sampling.rs").as_slice(),
            ] {
                hash.update(source);
            }
            hash.finalize().to_hex().to_string()
        });
        let mut hash = blake3::Hasher::new();
        hash.update(common.as_bytes());
        let input = match self.negative {
            Some(stock) => {
                for source in [
                    include_bytes!("frontier/negative.rs").as_slice(),
                    negative::DEFINITION,
                    include_bytes!("../../assets/frontier/sensor.json").as_slice(),
                    spektrafilm::negative_fingerprint(stock).as_bytes(),
                ] {
                    hash.update(source);
                }
                "Nominal colour-negative formation, spectral scanner OD and diagonal neutral readout"
            }
            None => {
                for source in [
                    include_bytes!("frontier/positive.rs").as_slice(),
                    positive::DEFINITION,
                    include_bytes!("characteristic.rs").as_slice(),
                    super::characteristic::DEFINITION,
                ] {
                    hash.update(source);
                }
                "Already-positive AP1, fixed linear Rec.709/D65 basis and neutral-calibrated soft-log readout"
            }
        };
        LookOrigin::Renderer {
            renderer: self.variant.id().into(),
            negative: self.negative.map(|stock| stock.id().into()),
            revision: density::revision(self.variant).into(),
            generator: hash.finalize().to_hex().to_string(),
            attribution: format!(
                "Smoothed AP1 response derived from Fujifilm Frontier350/370 parameter data. {input}; native A/B response with shared grey anchors. Artistic rendering calibration, not a measured scanner or physical-print profile. {}",
                spektrafilm::ATTRIBUTION
            ),
        }
    }
    pub fn from_origin(origin: &LookOrigin) -> Option<Self> {
        let LookOrigin::Renderer {
            renderer, negative, ..
        } = origin
        else {
            return None;
        };
        Some(Self {
            negative: match negative {
                Some(id) => Some(spektrafilm::Negative::from_id(id)?),
                None => None,
            },
            variant: Variant::all().find(|v| v.id() == renderer)?,
        })
    }
    pub fn prepare(self, cancel: &AtomicBool) -> Result<LookAsset, Error> {
        Model::new(self, cancel)?.bake(baking::DEFAULT_GRID, cancel)
    }
}

enum Input {
    Positive(positive::Entry),
    Negative(negative::Entry),
}

/// Authoring source only: projects and pixel consumers use its resolved LUT samples.
pub struct Model {
    selection: Selection,
    input: Input,
    response: Arc<density::Response>,
}
impl Model {
    pub fn new(selection: Selection, cancel: &AtomicBool) -> Result<Self, Error> {
        if cancel.load(Ordering::Relaxed) {
            return Err(failure("look preparation cancelled"));
        }
        let input = match selection.negative {
            None => Input::Positive(positive::Entry::new(selection.variant)?),
            Some(stock) => Input::Negative(negative::Entry::new(stock, selection.variant, cancel)?),
        };
        Ok(Self {
            selection,
            input,
            response: density::load(selection.variant)?,
        })
    }
    pub fn ap1(&self, rgb: DVec3) -> Result<DVec3, Error> {
        require(rgb.is_finite(), "nonfinite Frontier input")?;
        let codes = match &self.input {
            Input::Positive(entry) => entry.coordinates(rgb)?,
            Input::Negative(entry) => entry.coordinates(rgb)?,
        };
        let output = self.response.sample(codes);
        require(output.is_finite(), "nonfinite Frontier AP1 output")?;
        Ok(output)
    }
    pub fn bake(&self, size: u32, cancel: &AtomicBool) -> Result<LookAsset, Error> {
        baking::bake(
            self.selection.name(),
            self.selection.origin(),
            size,
            cancel,
            &|rgb| {
                Ok(self
                    .ap1(rgb)?
                    .map(crate::photographic::stages::look::encode))
            },
        )
    }
}
