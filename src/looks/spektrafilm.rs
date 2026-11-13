//! Native nonspatial spektrafilm evaluation and on-demand final LUT construction.
//!
//! Algorithm ports from spektrafilm by Andrea Volpato, GPLv3:
//! https://github.com/andreavolpato/spektrafilm, revision below.
//! See assets/spektrafilm/LICENSE-code.txt. Evaluated profiles/tables are
//! CC BY-SA 4.0; see LICENSE-data.txt and CHANGELOG.txt in that directory.
//! The positive-to-paper adapter and application/LUT plumbing are Antitone additions.
use super::{LookAsset, LookOrigin, failure, require};
use crate::{Error, photographic};
use glam::{DMat3, DVec3};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
mod data;
mod grid;
mod positive;
use super::lookup::cubic_2d;
use grid::{CubicGrid, bilinear};

pub const REVISION: &str = "3bb2c2d2801ff68b92019cf1dbcbb133d60832bc";
pub const ATTRIBUTION: &str = "Derived from spektrafilm by Andrea Volpato, https://github.com/andreavolpato/spektrafilm. Profile/LUT data licensed CC BY-SA 4.0, https://creativecommons.org/licenses/by-sa/4.0/. Modified by Antitone contributors: nonspatial factorization, neutral calibration and inverse authoring DRT.";
const EPSILON: f64 = 1e-10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Negative(usize);
impl Negative {
    pub fn all() -> impl ExactSizeIterator<Item = Self> {
        (0..data::labels().negative.len()).map(Self)
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::all().find(|n| n.id() == id)
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
        Self::all().find(|p| p.id() == id)
    }
    pub fn id(self) -> &'static str {
        &data::labels().paper[self.0].id
    }
    pub fn name(self) -> &'static str {
        &data::labels().paper[self.0].name
    }
}
impl Default for Paper {
    fn default() -> Self {
        Self::from_id("kodak_portra_endura").expect("canonical spektrafilm paper")
    }
}

/// An enabled treatment always has a paper. A missing negative means paper-only;
/// the enclosing optional LookAsset supplies the complete bypass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub negative: Option<Negative>,
    pub paper: Paper,
}
impl Selection {
    pub fn name(self) -> String {
        self.negative.map_or_else(
            || self.paper.name().to_owned(),
            |n| format!("{} · {}", n.name(), self.paper.name()),
        )
    }
    pub fn origin(self) -> LookOrigin {
        LookOrigin::Photochemical {
            negative: self.negative.map(|v| v.id().into()),
            paper: self.paper.id().into(),
            revision: REVISION.into(),
            generator: fingerprint().into(),
            attribution: ATTRIBUTION.into(),
        }
    }
    /// Historic numeric assets still load if their source IDs are no longer in the catalogue.
    /// They are not regenerated on open or reclassified as current data.
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
            include_bytes!("spektrafilm.rs").as_slice(),
            include_bytes!("authoring.rs").as_slice(),
            include_bytes!("sampling.rs").as_slice(),
            include_bytes!("spektrafilm/data.rs").as_slice(),
            include_bytes!("spektrafilm/grid.rs").as_slice(),
            include_bytes!("spektrafilm/positive.rs").as_slice(),
            include_bytes!("spektrafilm/positive/reference.rs").as_slice(),
            include_bytes!("lookup.rs").as_slice(),
        ] {
            hash.update(bytes);
        }
        for (name, bytes) in data::FILES {
            hash.update(name.as_bytes());
            hash.update(bytes);
        }
        format!("spektrafilm-native-v1:{}", hash.finalize().to_hex())
    })
}

fn matrix(rows: [[f64; 3]; 3]) -> DMat3 {
    DMat3::from_cols_array_2d(&rows).transpose()
}

/// Source: model/density_curves.py. Endpoints extend constantly, as upstream.
fn interpolate_exposure_to_density(
    h: DVec3,
    axis: &[f64],
    curves: &[DVec3],
    gamma: DVec3,
) -> DVec3 {
    DVec3::from_array(std::array::from_fn(|c| {
        let x = h[c] * gamma[c];
        let k = axis.partition_point(|&v| v <= x);
        if k == 0 {
            curves[0][c]
        } else if k == axis.len() {
            curves[k - 1][c]
        } else {
            let f = (x - axis[k - 1]) / (axis[k] - axis[k - 1]);
            curves[k - 1][c] + (curves[k][c] - curves[k - 1][c]) * f
        }
    }))
}

fn develop(film: &data::Film, h: DVec3) -> DVec3 {
    let initial =
        interpolate_exposure_to_density(h, &film.log_exposure, &film.density_curves, film.gamma);
    // model/couplers.py uses donor rows and receiver columns.
    let corrected = compute_exposure_correction_dir_couplers(h, initial, matrix(film.dir_matrix));
    interpolate_exposure_to_density(
        corrected,
        &film.log_exposure,
        &film.pre_dir_curves,
        film.gamma,
    )
}
fn compute_exposure_correction_dir_couplers(h: DVec3, density: DVec3, couplers: DMat3) -> DVec3 {
    h - couplers.transpose() * density
}
fn develop_print_morph(paper: &data::Paper, h: DVec3) -> DVec3 {
    // Curve morphing is evaluated upstream at build time; its effective graph is retained.
    interpolate_exposure_to_density(h, &paper.log_exposure, &paper.density_curves, DVec3::ONE)
}

struct Filming {
    profile: &'static data::Film,
    tc: Vec<DVec3>,
    printing: CubicGrid,
}
impl Filming {
    fn new(negative: Negative, paper: Paper, cancel: &AtomicBool) -> Result<Self, Error> {
        let c = data::catalog();
        let film = &c.negative[negative.0];
        let spectra = data::spectra()?;
        let n = c.spectra_size;
        let w = c.wavelengths;
        let mut raw = Vec::with_capacity(n * n);
        for (i, light) in spectra.chunks_exact(w).enumerate() {
            if i % n == 0 && cancel.load(Ordering::Relaxed) {
                return Err(failure("look preparation cancelled"));
            }
            raw.push(
                light
                    .iter()
                    .zip(&film.sensitivity)
                    .fold(DVec3::ZERO, |sum, (value, s)| sum + *s * (*value)),
            );
        }
        let coords = data::coordinates(&film.illuminant)?;
        let tc = coords.into_iter().map(|p| bilinear(&raw, n, p)).collect();
        let pair = c
            .pairs
            .iter()
            .find(|p| p.negative == film.id && p.paper == paper.id())
            .ok_or_else(|| failure("missing printing calibration"))?;
        let maximum = film
            .density_curves
            .iter()
            .copied()
            .fold(DVec3::ZERO, DVec3::max);
        // Pair-specific spectra are projected on demand, never installed as a catalogue of cubes.
        let size = 33;
        let mut values = Vec::with_capacity(size * size * size);
        for r in 0..size {
            if cancel.load(Ordering::Relaxed) {
                return Err(failure("look preparation cancelled"));
            }
            for g in 0..size {
                for b in 0..size {
                    let d = DVec3::new(r as f64, g as f64, b as f64) / (size - 1) as f64 * maximum;
                    values.push(film_cmy_to_print_log_raw(film, pair, d));
                }
            }
        }
        Ok(Self {
            profile: film,
            tc,
            printing: CubicGrid {
                size,
                min: DVec3::ZERO,
                max: maximum,
                values,
            },
        })
    }
    fn expose(&self, rgb: DVec3) -> DVec3 {
        let xyz = matrix(self.profile.matrix) * rgb;
        let brightness = xyz.element_sum();
        let denominator = brightness.max(EPSILON);
        let x = xyz.x / denominator;
        let y = xyz.y / denominator;
        let tc = [
            ((1. - x) * (1. - x)).clamp(0., 1.),
            (y / (1. - x).max(EPSILON)).clamp(0., 1.),
        ];
        let raw = cubic_2d(&self.tc, data::catalog().spectra_size, tc) * brightness;
        raw.map(|v| (v.max(0.) + EPSILON).log10())
    }
}

/// Source: PrintingStage._film_cmy_to_print_log_raw and expose, spatial terms disabled.
fn film_cmy_to_print_log_raw(film: &data::Film, pair: &data::Pair, d: DVec3) -> DVec3 {
    let mut raw = DVec3::ZERO;
    for (i, weights) in pair.weights.iter().enumerate() {
        if film.support[i] {
            let density = film.dyes[i].dot(d) + film.base[i];
            raw += *weights * 10_f64.powf(-density);
        }
    }
    raw += pair.preflash;
    // Preserve upstream's two epsilon additions, including at the print-exposure boundary.
    ((raw.max(DVec3::ZERO) + DVec3::splat(EPSILON)) * pair.print_exposure + DVec3::splat(EPSILON))
        .map(f64::log10)
}

// Shared scalar tone convention, including the signed/low-light tangent.
// Colour enters only through the spectral reference, not independent RGB stops.
fn neutral_exposure(paper: &data::Paper, brightness: f64, mapping: &data::Mapping) -> DVec3 {
    let stops = 17.52 * (photographic::encode(brightness) - photographic::encode(mapping.gray_ap1));
    paper.positive_gray_log_h
        - DVec3::splat(stops * mapping.exposure_compression * std::f64::consts::LOG10_2)
}

pub struct Model {
    selection: Selection,
    film: Option<Filming>,
    paper: &'static data::Paper,
    scanner: Arc<CubicGrid>,
}
impl Model {
    pub fn new(selection: Selection, cancel: &AtomicBool) -> Result<Self, Error> {
        let paper = &data::catalog().paper[selection.paper.0];
        Ok(Self {
            selection,
            film: selection
                .negative
                .map(|f| Filming::new(f, selection.paper, cancel))
                .transpose()?,
            paper,
            scanner: data::scanner(paper)?,
        })
    }
    fn paper_exposure(&self, rgb: DVec3) -> Result<DVec3, Error> {
        match &self.film {
            Some(film) => Ok(film
                .printing
                .sample(develop(film.profile, film.expose(rgb)))),
            None => {
                self.paper
                    .positive_driver
                    .expose(rgb, self.paper, &data::catalog().paper_mapping)
            }
        }
    }
    pub fn encoded(&self, rgb: DVec3) -> Result<DVec3, Error> {
        require(rgb.is_finite(), "nonfinite spektrafilm input")?;
        let density = develop_print_morph(self.paper, self.paper_exposure(rgb)?);
        let value = self.scanner.sample(density);
        require(value.is_finite(), "nonfinite spektrafilm output")?;
        Ok(value)
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
