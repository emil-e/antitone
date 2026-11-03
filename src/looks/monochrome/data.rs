use glam::DVec3;
use serde::Deserialize;
use std::{io::Read, sync::OnceLock};

pub(super) const COMPONENTS: &[u8] = include_bytes!("../../../assets/monochrome/components.json.z");
pub(super) const SPECTRAL: &[u8] = include_bytes!("../../../assets/monochrome/spectral.f32.z");
pub(super) const INDEX: &[u8] = include_bytes!("../../../assets/monochrome/index.json");
pub(super) const PROVENANCE: &[u8] = include_bytes!("../../../assets/monochrome/provenance.json");

#[derive(Deserialize)]
pub(super) struct Label {
    pub id: String,
    pub name: String,
    pub citation: String,
}
#[derive(Deserialize)]
pub(super) struct Labels {
    pub negative: Vec<Label>,
    pub paper: Vec<Label>,
}
pub(super) fn labels() -> &'static Labels {
    static VALUE: OnceLock<Labels> = OnceLock::new();
    VALUE.get_or_init(|| serde_json::from_slice(INDEX).expect("embedded B&W index"))
}

#[derive(Deserialize)]
pub(super) struct Identity {
    pub id: String,
}
#[derive(Deserialize)]
pub(super) struct Exposure {
    pub min: f64,
    pub max: f64,
    pub density: Vec<f64>,
}
impl Exposure {
    pub fn sample(&self, log_h: f64) -> f64 {
        interpolate(log_h, self.min, self.max, &self.density)
    }
    fn validate(&self) {
        assert!(self.min.is_finite() && self.max.is_finite() && self.min < self.max);
        assert!(self.density.len() >= 2 && self.density.iter().all(|v| v.is_finite()));
        assert!(self.density.windows(2).all(|v| v[0] <= v[1]));
    }
}
#[derive(Deserialize)]
pub(super) struct Positive {
    pub gray_log_h: f64,
    pub gray_ap1: f64,
    pub exposure_compression: f64,
}
#[derive(Deserialize)]
pub(super) struct Paper {
    pub material: Identity,
    pub exposure: Exposure,
    pub positive: Positive,
}
#[derive(Deserialize)]
pub(super) struct Film {
    pub material: Identity,
    pub exposure: Exposure,
    pub matrix: [[f64; 3]; 3],
    pub gray_log_h: f64,
    pub raw_size: usize,
    pub raw_offset: usize,
}
#[derive(Deserialize)]
pub(super) struct Readout {
    pub min: f64,
    pub max: f64,
    pub encoded: Vec<DVec3>,
}
#[derive(Deserialize)]
pub(super) struct Catalog {
    pub negative: Vec<Film>,
    pub paper: Vec<Paper>,
    pub readout: Readout,
    #[serde(skip)]
    pub raw: Vec<f64>,
}
fn decompress(bytes: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    flate2::read::ZlibDecoder::new(bytes)
        .read_to_end(&mut decoded)
        .expect("embedded B&W component decompression");
    decoded
}
pub(super) fn catalog() -> &'static Catalog {
    static VALUE: OnceLock<Catalog> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut value: Catalog = serde_json::from_slice(&decompress(COMPONENTS))
            .expect("embedded B&W component structure");
        let raw = decompress(SPECTRAL);
        assert!(raw.len().is_multiple_of(4));
        value.raw = raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| f32::from_le_bytes(*v) as f64)
            .collect();
        assert!(value.raw.iter().all(|v| v.is_finite() && *v >= 0.));
        assert_eq!(value.negative.len(), labels().negative.len());
        assert_eq!(value.paper.len(), labels().paper.len());
        let mut offset = 0;
        for (film, label) in value.negative.iter().zip(&labels().negative) {
            assert_eq!(film.material.id, label.id);
            film.exposure.validate();
            assert!(film.gray_log_h.is_finite());
            assert!(film.matrix.iter().flatten().all(|v| v.is_finite()));
            assert!(film.raw_size >= 2);
            assert_eq!(film.raw_offset, offset);
            offset += film.raw_size * film.raw_size;
        }
        assert_eq!(offset, value.raw.len());
        for (paper, label) in value.paper.iter().zip(&labels().paper) {
            assert_eq!(paper.material.id, label.id);
            paper.exposure.validate();
            let p = &paper.positive;
            assert!(p.gray_log_h.is_finite() && p.gray_ap1.is_finite() && p.gray_ap1 > 0.);
            assert!(p.exposure_compression.is_finite() && p.exposure_compression > 0.);
        }
        let r = &value.readout;
        assert!(r.min.is_finite() && r.max.is_finite() && r.min < r.max);
        assert!(r.encoded.len() >= 2 && r.encoded.iter().all(|v| v.is_finite()));
        value
    })
}

pub(super) fn interpolate<T>(value: f64, min: f64, max: f64, samples: &[T]) -> T
where
    T: Copy + std::ops::Mul<f64, Output = T> + std::ops::Add<Output = T>,
{
    let position = ((value - min) / (max - min)).clamp(0., 1.) * (samples.len() - 1) as f64;
    let index = (position as usize).min(samples.len() - 2);
    let fraction = position - index as f64;
    samples[index] * (1. - fraction) + samples[index + 1] * fraction
}
