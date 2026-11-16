//! Frontier source model and provenance, resolved to the common LUT during preparation.
use super::{LookAsset, LookOrigin, authoring as baking, failure, require, spektrafilm};
use crate::{Error, photographic};
use glam::{DMat3, DVec3};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};

#[cfg(feature = "ocio-reference")]
pub mod authoring;
mod density;
mod formation;
mod negative;
#[cfg(test)]
mod tests;

fn matrix(rows: [[f64; 3]; 3]) -> DMat3 {
    DMat3::from_cols_array_2d(&rows).transpose()
}

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
    pub(super) fn code_offset(self) -> f64 {
        // Supplied negative-mode recentering: A450, B512.
        let origin = match self {
            Self::A => 450.,
            Self::B => 512.,
        };
        origin - 450.
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
            |negative| format!("{} · {}", negative.name(), self.variant.name()),
        )
    }
    fn response_source(self) -> density::Source {
        match self.negative {
            Some(_) => density::Source::Negative(self.variant),
            None => density::Source::Native(self.variant),
        }
    }
    pub fn origin(self) -> LookOrigin {
        static GENERATOR: OnceLock<String> = OnceLock::new();
        let generator = GENERATOR.get_or_init(|| {
            let mut h = blake3::Hasher::new();
            for source in [
                include_bytes!("frontier.rs").as_slice(),
                include_bytes!("frontier/formation.rs").as_slice(),
                include_bytes!("frontier/density.rs").as_slice(),
                include_bytes!("authoring.rs").as_slice(),
                include_bytes!("sampling.rs").as_slice(),
            ] {
                h.update(source);
            }
            h.finalize().to_hex().to_string()
        });
        let revision = self.response_source().revision();
        let generator = self.negative.map_or_else(
            || generator.clone(),
            |stock| {
                let mut hash = blake3::Hasher::new();
                hash.update(generator.as_bytes());
                hash.update(spektrafilm::negative_fingerprint(stock).as_bytes());
                hash.update(include_bytes!("frontier/negative.rs"));
                hash.update(include_bytes!("../../assets/frontier/sensor.json"));
                hash.finalize().to_hex().to_string()
            },
        );
        let attribution = if self.negative.is_some() {
            format!(
                "Smoothed density/AP1 response derived from Fujifilm Frontier350/370 parameter data. Fixed-exposure colour negative, SP3000-inspired spectral readout, shared zero-origin density calibration and authored rendering gamma; source-mode B origin. Not factory scanner or physical-print calibration. {}",
                spektrafilm::ATTRIBUTION
            )
        } else {
            concat!(
                "Smoothed density/AP1 response derived from Fujifilm Frontier350/370 parameter data. ",
                "Antitone forward density toe; A calibrated against Fuji TypeII, shared relative formation with source-mode code origins. ",
                "Not a factory scanner or physical-print calibration."
            ).into()
        };
        LookOrigin::Renderer {
            renderer: self.variant.id().into(),
            negative: self.negative.map(|stock| stock.id().into()),
            revision: revision.into(),
            generator,
            attribution,
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
    Positive(formation::Entry),
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
            Some(stock) => Input::Negative(negative::Entry::new(stock, selection.variant, cancel)?),
            None => Input::Positive(formation::Entry::calibrated(selection.variant)),
        };
        Ok(Self {
            selection,
            input,
            response: selection.response_source().load()?,
        })
    }
    pub fn ap1(&self, rgb: DVec3) -> Result<DVec3, Error> {
        require(rgb.is_finite(), "nonfinite Frontier input")?;
        let coordinates = match &self.input {
            Input::Positive(entry) => entry.coordinates(rgb)?,
            Input::Negative(entry) => entry.coordinates(rgb)?,
        };
        let output = self.response.sample(coordinates);
        require(output.is_finite(), "nonfinite Frontier AP1 output")?;
        Ok(output)
    }
    pub fn bake(&self, size: u32, cancel: &AtomicBool) -> Result<LookAsset, Error> {
        baking::bake(
            self.selection.name(),
            self.selection.origin(),
            size,
            cancel,
            &|rgb| Ok(self.ap1(rgb)?.map(photographic::encode)),
        )
    }
}

fn gamut(mut value: DVec3) -> DVec3 {
    let a = value.max_element();
    if a != 0. {
        for c in 0..3 {
            let distance = (a - value[c]) / a.abs();
            if distance > 0.9 {
                let t = 0.9999 - 0.9;
                let inverse = t / (distance - 0.9);
                let compressed = 0.9 + t / (1. + inverse.powf(1.2)).powf(1. / 1.2);
                value[c] = a - a.abs() * compressed;
            }
        }
    }
    value
}
