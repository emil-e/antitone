//! Validated, source-evaluated data. No upstream interpreter is loaded by the app.
use super::grid::CubicGrid;
use crate::{
    Error,
    looks::{failure, require},
};
use flate2::read::ZlibDecoder;
use glam::DVec3;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{Arc, OnceLock},
};

include!("../../../assets/spektrafilm/index.rs");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub id: String,
    pub name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Labels {
    pub revision: String,
    pub negative: Vec<Entry>,
    pub paper: Vec<Entry>,
}

pub(super) fn labels() -> &'static Labels {
    static LABELS: OnceLock<Labels> = OnceLock::new();
    LABELS.get_or_init(|| {
        serde_json::from_str(include_str!("../../../assets/spektrafilm/selection.json"))
            .expect("embedded spektrafilm labels")
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Film {
    pub id: String,
    pub name: String,
    pub illuminant: String,
    pub matrix: [[f64; 3]; 3],
    pub sensitivity: Vec<DVec3>,
    pub log_exposure: Vec<f64>,
    pub density_curves: Vec<DVec3>,
    #[serde(rename = "pre_DIR_curves")]
    pub pre_dir_curves: Vec<DVec3>,
    #[serde(rename = "DIR_matrix")]
    pub dir_matrix: [[f64; 3]; 3],
    pub gamma: DVec3,
    pub dyes: Vec<DVec3>,
    pub base: Vec<f64>,
    pub support: Vec<bool>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Paper {
    pub id: String,
    pub name: String,
    pub log_exposure: Vec<f64>,
    pub density_curves: Vec<DVec3>,
    pub scanner_min: DVec3,
    pub scanner_max: DVec3,
    pub scanner_file: String,
    #[serde(rename = "positive_gray_logH")]
    pub positive_gray_log_h: DVec3,
    #[serde(skip)]
    pub scanner: OnceLock<Result<Arc<CubicGrid>, Error>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Pair {
    pub negative: String,
    pub paper: String,
    pub weights: Vec<DVec3>,
    pub preflash: DVec3,
    pub print_exposure: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Mapping {
    pub exposure_compression: f64,
    #[serde(rename = "gray_AP1")]
    pub gray_ap1: f64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Catalog {
    pub schema: u32,
    pub revision: String,
    pub spectra_size: usize,
    pub wavelengths: usize,
    pub scanner_size: usize,
    pub negative: Vec<Film>,
    pub paper: Vec<Paper>,
    pub pairs: Vec<Pair>,
    pub remap: BTreeMap<String, String>,
    pub authoring_reference: [f64; 9],
    #[serde(rename = "print_gray_Y")]
    pub print_gray_y: f64,
    pub paper_mapping: Mapping,
    pub source_profile_sha256: BTreeMap<String, String>,
}

pub(super) fn decompress(name: &str) -> Result<Vec<u8>, Error> {
    let bytes = FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, bytes)| *bytes)
        .ok_or_else(|| failure(format!("missing spektrafilm data: {name}")))?;
    let mut result = Vec::new();
    ZlibDecoder::new(bytes)
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut result)
        .map_err(failure)?;
    require(
        result.len() <= 32 * 1024 * 1024,
        "spektrafilm component exceeds data bound",
    )?;
    Ok(result)
}

fn validate_curve(axis: &[f64], curves: &[DVec3]) -> Result<(), Error> {
    require(
        axis.len() >= 2
            && axis.len() == curves.len()
            && axis.iter().all(|v| v.is_finite())
            && axis.windows(2).all(|v| v[1] > v[0])
            && curves.iter().all(|v| v.is_finite()),
        "invalid spektrafilm characteristic curve",
    )
}

pub(super) fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| load_catalog().expect("validated embedded spektrafilm catalogue"))
}

fn load_catalog() -> Result<Catalog, Error> {
    let value: Catalog = serde_json::from_slice(&decompress("catalog.json.z")?).map_err(failure)?;
    require(
        value.schema == 1
            && value.revision == super::REVISION
            && value.wavelengths == 81
            && (4..=513).contains(&value.spectra_size)
            && (4..=129).contains(&value.scanner_size)
            && value.authoring_reference == crate::looks::authoring::REFERENCE
            && value.print_gray_y.is_finite()
            && value.print_gray_y > 0.
            && value.paper_mapping.exposure_compression > 0.
            && value.paper_mapping.gray_ap1 > 0.
            && !value.source_profile_sha256.is_empty(),
        "unsupported spektrafilm catalogue/reference",
    )?;
    let labels = labels();
    require(
        labels.revision == value.revision
            && labels
                .negative
                .iter()
                .map(|v| (&v.id, &v.name))
                .eq(value.negative.iter().map(|v| (&v.id, &v.name)))
            && labels
                .paper
                .iter()
                .map(|v| (&v.id, &v.name))
                .eq(value.paper.iter().map(|v| (&v.id, &v.name))),
        "spektrafilm label/data mismatch",
    )?;
    let w = value.wavelengths;
    for f in &value.negative {
        validate_curve(&f.log_exposure, &f.density_curves)?;
        validate_curve(&f.log_exposure, &f.pre_dir_curves)?;
        require(
            f.dyes.len() == w
                && f.base.len() == w
                && f.sensitivity.len() == w
                && f.support.len() == w
                && f.dyes.iter().chain(&f.sensitivity).all(|v| v.is_finite())
                && f.base.iter().all(|v| v.is_finite())
                && f.gamma.is_finite()
                && f.gamma.min_element() > 0.
                && f.matrix
                    .iter()
                    .chain(&f.dir_matrix)
                    .flatten()
                    .all(|v| v.is_finite())
                && value.remap.contains_key(&f.illuminant),
            "invalid spektrafilm negative data",
        )?;
    }
    for p in &value.paper {
        validate_curve(&p.log_exposure, &p.density_curves)?;
        require(
            p.scanner_min.is_finite()
                && p.scanner_max.is_finite()
                && (p.scanner_max - p.scanner_min).min_element() > 0.
                && p.positive_gray_log_h.is_finite(),
            "invalid spektrafilm paper data",
        )?;
    }
    for pair in &value.pairs {
        require(
            pair.weights.len() == w
                && pair.weights.iter().all(|v| v.is_finite())
                && pair.preflash.is_finite()
                && pair.print_exposure.is_finite()
                && pair.print_exposure > 0.
                && value.negative.iter().any(|f| f.id == pair.negative)
                && value.paper.iter().any(|p| p.id == pair.paper),
            "invalid spektrafilm printing calibration",
        )?;
    }
    require(
        value.pairs.len() == value.negative.len() * value.paper.len(),
        "incomplete spektrafilm pair catalogue",
    )?;
    Ok(value)
}

pub(super) fn spectra() -> Result<&'static [f64], Error> {
    static SPECTRA: OnceLock<Result<Vec<f64>, Error>> = OnceLock::new();
    SPECTRA
        .get_or_init(|| {
            let bytes = decompress("irradiance_xy_tc.f16.z")?;
            let c = catalog();
            require(
                bytes.len() == c.spectra_size * c.spectra_size * c.wavelengths * 2,
                "invalid Hanatos spectral family shape",
            )?;
            let values = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| f64::from(half::f16::from_le_bytes([b[0], b[1]])))
                .collect::<Vec<_>>();
            require(
                values.iter().all(|v| v.is_finite()),
                "nonfinite Hanatos spectral family",
            )?;
            Ok(values)
        })
        .as_ref()
        .map(Vec::as_slice)
        .map_err(Clone::clone)
}

pub(super) fn coordinates(illuminant: &str) -> Result<Vec<[f64; 2]>, Error> {
    let name = &catalog().remap[illuminant];
    let bytes = decompress(name)?;
    require(
        bytes.len() == catalog().spectra_size.pow(2) * 16,
        "invalid input-gamut coordinate count",
    )?;
    let values = bytes
        .as_chunks::<16>()
        .0
        .iter()
        .map(|b| {
            [
                f64::from_le_bytes(b[..8].try_into().unwrap()),
                f64::from_le_bytes(b[8..].try_into().unwrap()),
            ]
        })
        .collect::<Vec<_>>();
    require(
        values
            .iter()
            .flatten()
            .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
        "invalid input-gamut coordinates",
    )?;
    Ok(values)
}

pub(super) fn scanner(paper: &Paper) -> Result<Arc<CubicGrid>, Error> {
    paper
        .scanner
        .get_or_init(|| {
            let bytes = decompress(&paper.scanner_file)?;
            let size = catalog().scanner_size;
            require(
                bytes.len() == size.pow(3) * 12,
                "invalid spektrafilm scanner table length",
            )?;
            let values = bytes
                .as_chunks::<12>()
                .0
                .iter()
                .map(|b| {
                    DVec3::from_array(std::array::from_fn(|c| {
                        f64::from(f32::from_le_bytes(b[c * 4..c * 4 + 4].try_into().unwrap()))
                    }))
                })
                .collect::<Vec<_>>();
            require(
                values.iter().all(|v| v.is_finite()),
                "nonfinite spektrafilm scanner table",
            )?;
            Ok(Arc::new(CubicGrid {
                size,
                min: paper.scanner_min,
                max: paper.scanner_max,
                values,
            }))
        })
        .clone()
}
