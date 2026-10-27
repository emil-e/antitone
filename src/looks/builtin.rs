//! Frozen material evaluation for native LUT generation; no source fitting or image-dependent grade.
use super::{LookOrigin, failure, require};
use crate::{color, photographic};
use glam::{DMat3, DVec3};
use serde::Deserialize;
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

/// Grid for newly resolved built-ins; embedded and imported assets keep their own size.
/// Use 32k + 1 nodes per axis to retain the frozen gray anchor at 19/32 of the domain.
pub const DEFAULT_GRID: u32 = 129;
pub const GENERATOR: &str = "p3rec50-native-gpu-v1";
pub const REFERENCE: [f64; 9] = [
    100., 0.694, 0.306, 0.2175, 0.7435, 0.1405, 0.053, 0.3127, 0.3290,
];
pub const DOMAIN_MIN: f64 = -0.5;

pub fn domain_max() -> f64 {
    DOMAIN_MIN + (photographic::encode(0.18) - DOMAIN_MIN) * (64. / 38.)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    PortraEndura,
    ProImageII,
    Kodak2383,
    IlfordFilter2,
}

impl Builtin {
    pub const ALL: [Self; 4] = [
        Self::PortraEndura,
        Self::ProImageII,
        Self::Kodak2383,
        Self::IlfordFilter2,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::PortraEndura => "kodak-portra-endura",
            Self::ProImageII => "kodak-pro-image-ii",
            Self::Kodak2383 => "kodak-2383",
            Self::IlfordFilter2 => "ilford-multigrade-rc-deluxe-filter-2",
        }
    }

    pub fn name(self) -> &'static str {
        &self.metadata().name
    }

    pub fn source_product(self) -> &'static str {
        &self.metadata().source_product
    }

    fn metadata(self) -> &'static Metadata {
        static CATALOG: OnceLock<[Metadata; 4]> = OnceLock::new();
        let entries = CATALOG.get_or_init(|| {
            Self::ALL.map(|builtin| {
                serde_json::from_str(builtin.catalog_json())
                    .expect("validated built-in look metadata")
            })
        });
        &entries[Self::ALL
            .into_iter()
            .position(|builtin| builtin == self)
            .unwrap()]
    }

    fn catalog_json(self) -> &'static str {
        match self {
            Self::PortraEndura => include_str!("../../assets/looks/kodak-portra-endura/look.json"),
            Self::ProImageII => include_str!("../../assets/looks/kodak-pro-image-ii/look.json"),
            Self::Kodak2383 => include_str!("../../assets/looks/kodak-2383/look.json"),
            Self::IlfordFilter2 => {
                include_str!("../../assets/looks/ilford-multigrade-rc-deluxe-filter-2/look.json")
            }
        }
    }

    fn json(self) -> &'static str {
        match self {
            Self::PortraEndura => include_str!("../../assets/looks/kodak-portra-endura/model.json"),
            Self::ProImageII => include_str!("../../assets/looks/kodak-pro-image-ii/model.json"),
            Self::Kodak2383 => include_str!("../../assets/looks/kodak-2383/model.json"),
            Self::IlfordFilter2 => {
                include_str!("../../assets/looks/ilford-multigrade-rc-deluxe-filter-2/model.json")
            }
        }
    }

    pub fn origin(self) -> LookOrigin {
        let mut hash = blake3::Hasher::new();
        hash.update(self.json().as_bytes());
        hash.update(SPECTRAL.as_bytes());
        hash.update(include_bytes!("../../assets/looks/common/authoring.json"));
        LookOrigin::Builtin {
            id: self.id().into(),
            version: 1,
            generator: format!("{GENERATOR}:{}", generation_fingerprint()),
            definition: hash.finalize().to_hex().to_string(),
        }
    }
}

#[derive(Deserialize)]
struct Metadata {
    name: String,
    source_product: String,
}

pub fn generation_fingerprint() -> String {
    let mut hash = blake3::Hasher::new();
    for bytes in [
        include_bytes!("builtin.rs").as_slice(),
        include_bytes!("../renderer/look_reference.rs").as_slice(),
        include_bytes!("../renderer.rs").as_slice(),
        include_bytes!("../../assets/ocio/looks-reference/forward.glsl").as_slice(),
        include_bytes!("../../assets/ocio/looks-reference/inverse.glsl").as_slice(),
        include_bytes!("../../assets/ocio/looks-reference/reach.f32").as_slice(),
        include_bytes!("../../assets/ocio/looks-reference/cusp.f32").as_slice(),
    ] {
        hash.update(bytes);
    }
    hash.finalize().to_hex().to_string()
}

const SPECTRAL: &str = include_str!("../../assets/looks/common/spectral.json");
#[derive(Deserialize)]
#[serde(tag = "family", deny_unknown_fields)]
enum Definition {
    #[serde(rename = "spectral-density-table")]
    Spectral {
        schema_version: u32,
        log_exposure: Vec<f64>,
        dye_density: Vec<[f64; 3]>,
        effective_coefficients: Vec<[f64; 3]>,
        separate_base: Option<()>,
    },
    #[serde(rename = "neutral-density-analytic")]
    Neutral {
        schema_version: u32,
        log_exposure_range: [f64; 2],
        density_parameters: [f64; 5],
    },
}
#[derive(Deserialize)]
struct Spectral {
    schema_version: u32,
    wavelengths: Vec<f64>,
    observer: Vec<[f64; 3]>,
    illumination: Vec<f64>,
}
fn xyz(x: f64, y: f64) -> DVec3 {
    DVec3::new(x / y, 1., (1. - x - y) / y)
}
fn adaptation(from: DVec3, to: DVec3) -> DMat3 {
    let bradford = color::BRADFORD;
    bradford.inverse() * DMat3::from_diagonal((bradford * to) / (bradford * from)) * bradford
}
fn paper_gray() -> f64 {
    // OCIO 2.6.0 neutral operating point only; no scalar output encoding.
    // BSD-3-Clause, Copyright Contributors to the OpenColorIO Project;
    // retained notice: assets/ocio/LICENSE-OpenColorIO.txt.
    let g = 1.15;
    let toe = 0.04;
    let hit = 128.;
    let gray = 0.18;
    let display_gray = 10.013 / 100.;
    let m1 = 0.5 * (1. + f64::sqrt(1. + 4. * toe));
    let u = f64::powf((hit / m1) / (hit / m1 + 1.), g);
    let m = m1 / u;
    let gip = 0.5 * (display_gray + f64::sqrt(display_gray * (display_gray + 4. * toe)));
    let power = f64::powf(gip / m, 1. / g);
    let gipp2 = -(m1 * power) / (power - 1.);
    let w2 = gray / gipp2;
    let s_relative = w2 * m1;
    let u2 = f64::powf((hit / m1) / (hit / m1 + w2), g);
    let m2 = m1 / u2;
    let f = m2 * f64::powf(gray / (gray + s_relative), g);
    f * f / (f + toe)
}
enum Material {
    Spectral {
        range: [f64; 2],
        step: f64,
        coefficients: Vec<DVec3>,
        dyes: Vec<DVec3>,
        weights: Vec<DVec3>,
    },
    Neutral {
        range: [f64; 2],
        parameters: [f64; 5],
    },
}

impl Material {
    fn range(&self) -> [f64; 2] {
        match self {
            Self::Spectral { range, .. } | Self::Neutral { range, .. } => *range,
        }
    }

    fn attenuate(&self, h: DVec3) -> DVec3 {
        match self {
            Self::Neutral {
                range,
                parameters: p,
            } => {
                let x = h.x.clamp(range[0], range[1]);
                let z = -p[2] * (x - p[3]);
                let density = p[0] + (p[1] - p[0]) * (-p[4] * z.exp().ln_1p()).exp();
                DVec3::splat(10f64.powf(-density))
            }
            Self::Spectral {
                range,
                step,
                coefficients,
                dyes,
                weights,
            } => {
                let mut density = DVec3::ZERO;
                for c in 0..3 {
                    let at = ((h[c] - range[0]) / step).clamp(0., 255.);
                    let low = (at as usize).min(254);
                    let f = at - low as f64;
                    density[c] = coefficients[low][c] * (1. - f) + coefficients[low + 1][c] * f;
                }
                let mut rgb = DVec3::ZERO;
                for (dye, weight) in dyes.iter().zip(weights) {
                    rgb += *weight * 10f64.powf(-dye.dot(density));
                }
                rgb
            }
        }
    }

    fn calibrate(&self, gray: f64) -> Result<DVec3, crate::Error> {
        let [low, high] = self.range();
        let mut offset = DVec3::splat((low + high) * 0.5);
        if matches!(self, Self::Neutral { .. }) {
            let (mut lo, mut hi) = (low, high);
            require(
                gray <= self.attenuate(DVec3::splat(lo)).x
                    && gray >= self.attenuate(DVec3::splat(hi)).x,
                "gray outside neutral material response",
            )?;
            for _ in 0..100 {
                let mid = (lo + hi) * 0.5;
                if self.attenuate(DVec3::splat(mid)).x > gray {
                    lo = mid
                } else {
                    hi = mid
                }
            }
            offset = DVec3::splat((lo + hi) * 0.5);
        } else {
            for _ in 0..30 {
                let value = self.attenuate(offset);
                let error = value - DVec3::splat(gray);
                if error.abs().max_element() < 1e-12 {
                    break;
                }
                let columns: [DVec3; 3] = std::array::from_fn(|c| {
                    let mut h = offset;
                    h[c] += 1e-5;
                    (self.attenuate(h) - value) * 1e5
                });
                let jacobian = DMat3::from_cols(columns[0], columns[1], columns[2]);
                let mut delta = jacobian.inverse() * error;
                require(delta.is_finite(), "singular material gray solve")?;
                if delta.abs().max_element() > 0.25 {
                    delta *= 0.25 / delta.abs().max_element();
                }
                offset -= delta;
            }
        }
        require(
            (self.attenuate(offset) - DVec3::splat(gray))
                .abs()
                .max_element()
                <= 1e-9,
            "material gray solve failed",
        )?;
        Ok(offset)
    }
}

pub struct Model {
    material: Material,
    offset: DVec3,
    to_reference: DMat3,
    reference_y: DVec3,
}

impl Model {
    pub fn new(builtin: Builtin) -> Result<Self, crate::Error> {
        let definition: Definition = serde_json::from_str(builtin.json()).map_err(failure)?;
        let material = match definition {
            Definition::Neutral {
                schema_version,
                log_exposure_range,
                density_parameters,
            } => {
                require(
                    schema_version == 1
                        && log_exposure_range[0] < log_exposure_range[1]
                        && density_parameters.iter().all(|p| p.is_finite()),
                    "invalid frozen neutral definition",
                )?;
                Material::Neutral {
                    range: log_exposure_range,
                    parameters: density_parameters,
                }
            }
            Definition::Spectral {
                schema_version,
                log_exposure,
                dye_density,
                effective_coefficients,
                separate_base,
            } => {
                require(
                    separate_base.is_none(),
                    "separate base spectra are not supported by this model family",
                )?;
                let common: Spectral = serde_json::from_str(SPECTRAL).map_err(failure)?;
                require(
                    schema_version == 1
                        && common.schema_version == 1
                        && log_exposure.len() == 256
                        && dye_density.len() == 61
                        && effective_coefficients.len() == 256
                        && common.observer.len() == 61
                        && common.illumination.len() == 61
                        && common.wavelengths.len() == 61,
                    "invalid frozen spectral dimensions",
                )?;
                require(
                    log_exposure.iter().all(|v| v.is_finite())
                        && dye_density
                            .iter()
                            .flatten()
                            .chain(effective_coefficients.iter().flatten())
                            .all(|v| v.is_finite() && *v >= 0.),
                    "invalid frozen spectral values",
                )?;
                let mut white = DVec3::ZERO;
                let mut norm = 0.;
                for (observer, illumination) in common.observer.iter().zip(&common.illumination) {
                    white += DVec3::from_array(*observer) * illumination;
                    norm += illumination * observer[1];
                }
                white /= norm;
                let to_ap1 =
                    color::AP1_TO_XYZ.inverse() * adaptation(white, color::AP1_TO_XYZ * DVec3::ONE);
                let weights = common
                    .observer
                    .iter()
                    .zip(&common.illumination)
                    .map(|(o, s)| to_ap1 * DVec3::from_array(*o) * (s / norm))
                    .collect();
                Material::Spectral {
                    range: [log_exposure[0], log_exposure[255]],
                    step: log_exposure[1] - log_exposure[0],
                    coefficients: effective_coefficients
                        .into_iter()
                        .map(DVec3::from_array)
                        .collect(),
                    dyes: dye_density.into_iter().map(DVec3::from_array).collect(),
                    weights,
                }
            }
        };
        // Validate the 18% gray solve before calibrating to the paper's neutral operating point.
        material.calibrate(0.18)?;
        let offset = material.calibrate(paper_gray())?;
        let p = REFERENCE;
        let m = DMat3::from_cols(xyz(p[1], p[2]), xyz(p[3], p[4]), xyz(p[5], p[6]));
        let reference = m * DMat3::from_diagonal(m.inverse() * xyz(p[7], p[8]));
        let to_reference = reference.inverse()
            * adaptation(color::AP1_TO_XYZ * DVec3::ONE, xyz(p[7], p[8]))
            * color::AP1_TO_XYZ;
        Ok(Self {
            material,
            offset,
            to_reference,
            reference_y: DVec3::new(reference.x_axis.y, reference.y_axis.y, reference.z_axis.y),
        })
    }

    pub fn raw(&self, input: DVec3) -> DVec3 {
        let factor = -std::f64::consts::LOG10_2 * 0.6;
        let stops = if matches!(self.material, Material::Neutral { .. }) {
            DVec3::splat(
                17.52
                    * (photographic::encode(color::AP1_TO_XYZ.row(1).dot(input))
                        - photographic::encode(0.18)),
            )
        } else {
            input.map(|c| 17.52 * (photographic::encode(c) - photographic::encode(0.18)))
        };
        self.material.attenuate(self.offset + stops * factor)
    }

    pub fn mapped(&self, input: DVec3) -> Result<DVec3, crate::Error> {
        let mut value = self.to_reference * self.raw(input);
        let y = self.reference_y.dot(value);
        require(
            value.is_finite() && y > 0. && y < 1.,
            "material reference luminance outside (0,1)",
        )?;
        let radius = value
            .to_array()
            .into_iter()
            .map(|c| {
                if c >= y {
                    (c - y) / (1. - y)
                } else {
                    (y - c) / y
                }
            })
            .fold(0., f64::max);
        let scale = if radius > 0.9 {
            (0.9 + 0.1 * (radius - 0.9) / (radius - 0.9 + 0.1)) / radius
        } else {
            1.
        };
        value = DVec3::splat(y) + (value - DVec3::splat(y)) * scale;
        require(
            value.is_finite() && value.min_element() >= 0. && value.max_element() <= 1.,
            "material reference boundary mapping failed",
        )?;
        Ok(value)
    }

    pub fn mapped_nodes(
        &self,
        size: u32,
        cancel: &AtomicBool,
    ) -> Result<Vec<[f32; 4]>, crate::Error> {
        require(
            (2..=super::MAX_GRID_SIZE).contains(&size),
            "invalid generated LUT dimensions",
        )?;
        let mut values = Vec::with_capacity((size as usize).pow(3));
        let span = domain_max() - DOMAIN_MIN;
        for b in 0..size {
            for g in 0..size {
                require(
                    !cancel.load(Ordering::Relaxed),
                    "look preparation cancelled",
                )?;
                for r in 0..size {
                    let input = DVec3::from_array([r, g, b].map(|v| {
                        photographic::decode(DOMAIN_MIN + span * f64::from(v) / f64::from(size - 1))
                    }));
                    let mapped = self.mapped(input)?;
                    values.push([mapped.x as f32, mapped.y as f32, mapped.z as f32, 1.]);
                }
            }
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_grid_keeps_the_frozen_gray_anchor_on_a_node() {
        assert!((2..=super::super::MAX_GRID_SIZE).contains(&DEFAULT_GRID));
        assert_eq!((DEFAULT_GRID - 1) % 32, 0);
    }

    #[test]
    fn builtin_names_and_attribution_come_from_the_catalog() {
        for (builtin, name, source) in [
            (
                Builtin::PortraEndura,
                "Portrait Paper",
                "KODAK PROFESSIONAL PORTRA ENDURA Paper",
            ),
            (
                Builtin::ProImageII,
                "Colour Paper",
                "KODAK PROFESSIONAL PRO IMAGE II Paper",
            ),
            (
                Builtin::Kodak2383,
                "Cinema Print",
                "KODAK VISION Color Print Film 2383 / 3383",
            ),
            (
                Builtin::IlfordFilter2,
                "Monochrome Paper",
                "ILFORD MULTIGRADE RC DELUXE (new generation), filter 2",
            ),
        ] {
            let catalog: serde_json::Value = serde_json::from_str(builtin.catalog_json()).unwrap();
            assert_eq!(catalog["id"], builtin.id());
            assert_eq!(catalog["name"], name);
            assert_eq!(catalog["source_product"], source);
            assert_eq!(builtin.name(), name);
            assert_eq!(builtin.source_product(), source);
        }
    }

    #[test]
    fn selected_models_keep_gray_and_monochrome_luminance_first() {
        for builtin in Builtin::ALL {
            let model = Model::new(builtin).unwrap();
            assert!(
                (model.raw(DVec3::splat(0.18)) - DVec3::splat(paper_gray()))
                    .abs()
                    .max_element()
                    < 1e-9
            );
            if builtin == Builtin::IlfordFilter2 {
                let input = DVec3::new(0.7, 0.05, 0.01);
                let y = color::AP1_TO_XYZ.row(1).dot(input);
                assert!(
                    (model.raw(input) - model.raw(DVec3::splat(y)))
                        .abs()
                        .max_element()
                        < 1e-14
                );
            }
            assert!(model.mapped(DVec3::splat(0.18)).unwrap().is_finite());
        }
    }
}
