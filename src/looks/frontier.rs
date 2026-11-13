//! Frontier source model and provenance, resolved to the common LUT during preparation.
use super::{LookAsset, LookOrigin, authoring as baking, failure, require};
use crate::{Error, photographic};
use glam::{DMat3, DVec3};
use std::sync::{Arc, OnceLock, atomic::AtomicBool};

#[cfg(feature = "ocio-reference")]
pub mod authoring;
mod density;
mod formation;
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
    fn id(self) -> &'static str {
        match self {
            Self::A => "frontier_a",
            Self::B => "frontier_b",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub variant: Variant,
}
impl Selection {
    pub fn name(self) -> String {
        self.variant.name().into()
    }
    pub fn origin(self) -> LookOrigin {
        let bytes = match self.variant {
            Variant::A => include_bytes!("../../assets/frontier/density-a.f32.z").as_slice(),
            Variant::B => include_bytes!("../../assets/frontier/density-b.f32.z").as_slice(),
        };
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
        LookOrigin::Renderer {
            renderer: self.variant.id().into(),
            revision: blake3::hash(bytes).to_hex().to_string(),
            generator: generator.clone(),
            attribution: concat!(
                "Smoothed density/AP1 response derived from Fujifilm Frontier350/370 parameter data. ",
                "Antitone forward density toe; A calibrated against Fuji TypeII, shared relative formation with source-mode code origins. ",
                "Not a factory scanner or physical-print calibration."
            ).into(),
        }
    }
    pub fn from_origin(origin: &LookOrigin) -> Option<Self> {
        let LookOrigin::Renderer { renderer, .. } = origin else {
            return None;
        };
        Some(Self {
            variant: Variant::all().find(|v| v.id() == renderer)?,
        })
    }
    pub fn prepare(self, cancel: &AtomicBool) -> Result<LookAsset, Error> {
        Model::new(self)?.bake(baking::DEFAULT_GRID, cancel)
    }
}

/// Authoring source only: projects and pixel consumers use its resolved LUT samples.
pub struct Model {
    selection: Selection,
    entry: formation::Entry,
    response: Arc<density::Response>,
}
impl Model {
    pub fn new(selection: Selection) -> Result<Self, Error> {
        Ok(Self {
            selection,
            entry: formation::Entry::calibrated(selection.variant),
            response: density::builtin(selection.variant)?,
        })
    }
    pub fn ap1(&self, rgb: DVec3) -> Result<DVec3, Error> {
        require(rgb.is_finite(), "nonfinite Frontier input")?;
        let output = self.response.sample(self.entry.coordinates(rgb)?);
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
