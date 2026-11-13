//! Scalar B&W film formation and fixed-grade printing, through ordinary LUT assets.
//!
//! Manufacturer characteristic curves supply total density; silver transmission
//! and paper reflection are neutral approximations. Shared Hanatos reconstruction
//! and nonspatial readout use pinned Spektrafilm code/data, not colour-stock
//! sensitivities, dyes or DIR. See assets/papers/README.md for provenance.
use super::{LookAsset, LookOrigin, require};
use crate::{Error, color, photographic};
use glam::{DMat3, DVec3};
use std::sync::{OnceLock, atomic::AtomicBool};
mod data;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Negative(usize);
impl Negative {
    pub fn all() -> impl ExactSizeIterator<Item = Self> {
        (0..data::labels().negative.len()).map(Self)
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::all().find(|v| v.id() == id)
    }
    pub fn id(self) -> &'static str {
        &data::labels().negative[self.0].id
    }
    pub fn name(self) -> &'static str {
        &data::labels().negative[self.0].name
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paper(usize);
impl Paper {
    pub fn all() -> impl ExactSizeIterator<Item = Self> {
        (0..data::labels().paper.len()).map(Self)
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::all().find(|v| v.id() == id)
    }
    pub fn id(self) -> &'static str {
        &data::labels().paper[self.0].id
    }
    pub fn name(self) -> &'static str {
        &data::labels().paper[self.0].name
    }
    pub fn origin(self) -> LookOrigin {
        Selection::from(self).origin()
    }
    pub fn from_origin(origin: &LookOrigin) -> Option<Self> {
        Selection::from_origin(origin)
            .filter(|v| v.negative.is_none())
            .map(|v| v.paper)
    }
}
impl Default for Paper {
    fn default() -> Self {
        Self::from_id("ilford_multigrade_rc_deluxe_filter_2").expect("canonical B&W paper")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub negative: Option<Negative>,
    pub paper: Paper,
}
impl From<Paper> for Selection {
    fn from(paper: Paper) -> Self {
        Self {
            negative: None,
            paper,
        }
    }
}
impl Selection {
    pub fn name(self) -> String {
        self.negative.map_or_else(
            || self.paper.name().to_owned(),
            |negative| format!("{} · {}", negative.name(), self.paper.name()),
        )
    }
    pub fn origin(self) -> LookOrigin {
        let mut citation = data::labels().paper[self.paper.0].citation.clone();
        if let Some(negative) = self.negative {
            citation.push_str("; ");
            citation.push_str(&data::labels().negative[negative.0].citation);
            citation.push_str(". Estimated scene spectra use the shared Hanatos family distributed by Spektrafilm (CC BY-SA 4.0; assets/spektrafilm/LICENSE-data.txt)");
        }
        LookOrigin::Photochemical {
            negative: self.negative.map(|v| v.id().into()),
            paper: self.paper.id().into(),
            revision: super::spektrafilm::REVISION.into(),
            generator: fingerprint().into(),
            attribution: format!(
                "Antitone reconstruction from {citation}. Neutral-silver model, not manufacturer-authored profiles. Nonspatial algorithms use spektrafilm by Andrea Volpato, https://github.com/andreavolpato/spektrafilm (GPLv3; assets/spektrafilm/LICENSE-code.txt). Own characteristic/sensitivity data; no colour-stock profile."
            ),
        }
    }
    pub fn from_origin(origin: &LookOrigin) -> Option<Self> {
        let LookOrigin::Photochemical {
            negative, paper, ..
        } = origin
        else {
            return None;
        };
        Some(Self {
            negative: match negative {
                Some(id) => Some(Negative::from_id(id)?),
                None => None,
            },
            paper: Paper::from_id(paper)?,
        })
    }
}
fn fingerprint() -> &'static str {
    static VALUE: OnceLock<String> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut hash = blake3::Hasher::new();
        for bytes in [
            include_bytes!("monochrome.rs").as_slice(),
            include_bytes!("monochrome/data.rs").as_slice(),
            include_bytes!("lookup.rs").as_slice(),
            include_bytes!("sampling.rs").as_slice(),
            include_bytes!("authoring.rs").as_slice(),
            data::COMPONENTS,
            data::SPECTRAL,
            data::INDEX,
            data::PROVENANCE,
        ] {
            hash.update(bytes);
        }
        format!("monochrome-v2:{}", hash.finalize().to_hex())
    })
}

struct Filming {
    profile: &'static data::Film,
    raw: &'static [f64],
    gray_density: f64,
}
impl Filming {
    fn new(negative: Negative) -> Self {
        let catalog = data::catalog();
        let profile = &catalog.negative[negative.0];
        let raw = &catalog.raw[profile.raw_offset..profile.raw_offset + profile.raw_size.pow(2)];
        let mut result = Self {
            profile,
            raw,
            gray_density: 0.,
        };
        result.gray_density = profile.exposure.sample(result.exposure(DVec3::splat(0.18)));
        result
    }
    fn raw(&self, rgb: DVec3) -> f64 {
        let xyz = DMat3::from_cols_array_2d(&self.profile.matrix).transpose() * rgb;
        let brightness = xyz.element_sum();
        let x = xyz.x / brightness.max(1e-10);
        let y = xyz.y / brightness.max(1e-10);
        let tc = [
            ((1. - x) * (1. - x)).clamp(0., 1.),
            (y / (1. - x).max(1e-10)).clamp(0., 1.),
        ];
        super::lookup::cubic_2d(self.raw, self.profile.raw_size, tc) * brightness
    }
    fn exposure(&self, rgb: DVec3) -> f64 {
        self.profile.gray_log_h + (self.raw(rgb).max(0.) + 1e-10).log10()
    }
}

pub struct Model {
    selection: Selection,
    paper: &'static data::Paper,
    film: Option<Filming>,
}
impl Model {
    pub fn new(selection: impl Into<Selection>) -> Self {
        let selection = selection.into();
        Self {
            selection,
            paper: &data::catalog().paper[selection.paper.0],
            film: selection.negative.map(Filming::new),
        }
    }
    fn exposure(&self, rgb: DVec3) -> f64 {
        if let Some(film) = &self.film {
            let density = film.profile.exposure.sample(film.exposure(rgb));
            // A neutral negative attenuates the fixed printing spectrum equally
            // at every wavelength. Gray calibration changes only printing time.
            self.paper.positive.gray_log_h + film.gray_density - density
        } else {
            // Already-positive input: luminance precedes the nonlinear adapter.
            let p = &self.paper.positive;
            let y = color::AP1_TO_XYZ.row(1).dot(rgb);
            let stops = 17.52 * (photographic::encode(y) - photographic::encode(p.gray_ap1));
            p.gray_log_h - stops * (p.exposure_compression * std::f64::consts::LOG10_2)
        }
    }
    fn density(&self, log_h: f64) -> f64 {
        self.paper.exposure.sample(log_h)
    }
    pub fn encoded(&self, rgb: DVec3) -> Result<DVec3, Error> {
        require(rgb.is_finite(), "nonfinite B&W input")?;
        let density = self.density(self.exposure(rgb));
        let r = &data::catalog().readout;
        let encoded = data::interpolate(density, r.min, r.max, &r.encoded);
        require(encoded.is_finite(), "nonfinite B&W output")?;
        Ok(encoded)
    }
    pub fn bake(&self, size: u32, cancel: &AtomicBool) -> Result<LookAsset, Error> {
        super::authoring::bake(
            self.selection.name(),
            self.selection.origin(),
            size,
            cancel,
            &|rgb| self.encoded(rgb),
        )
    }
}
#[cfg(test)]
mod tests;
