//! Calibrated exposure records and effective dyes for the paper-only negative.
use super::super::{data, matrix};
use crate::{Error, looks::require};
use glam::{DMat3, DVec3};
use serde::Deserialize;
use std::sync::OnceLock;

pub(in super::super) const DEFINITION: &[u8] =
    include_bytes!("../../../../assets/looks/paper-reference.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Definition {
    schema: u32,
    pub negative: String,
    exposure_rows: [[f64; 3]; 3],
    dye_rows: [[f64; 3]; 3],
    density_gain: f64,
    pub base_density_offset: f64,
}

impl Definition {
    fn validate(&self) -> Result<(), Error> {
        require(
            self.schema == 1
                && !self.negative.is_empty()
                && self.density_gain.is_finite()
                && self.density_gain > 0.
                && self.base_density_offset.is_finite()
                && self.base_density_offset >= 0.
                && 10_f64.powf(self.base_density_offset).is_finite(),
            "invalid paper-reference definition",
        )?;
        for rows in [self.exposure_rows, self.dye_rows] {
            let m = matrix(rows);
            require(
                m.is_finite()
                    && m.determinant().abs() > 1e-10
                    && (m * DVec3::ONE - DVec3::ONE).abs().max_element() < 1e-12,
                "invalid neutral-preserving paper-reference matrix",
            )?;
        }
        Ok(())
    }
}

fn definition() -> &'static Definition {
    static VALUE: OnceLock<Definition> = OnceLock::new();
    VALUE.get_or_init(|| {
        let value: Definition = serde_json::from_slice(DEFINITION).expect("paper reference");
        value.validate().expect("valid paper reference");
        value
    })
}

pub(super) struct Sample {
    pub index: usize,
    pub base: f64,
    pub dyes: DVec3,
}

pub(super) struct Reference {
    pub definition: &'static Definition,
    exposure: DMat3,
    dye_basis: DMat3,
    lamp_scale: f64,
    pub samples: Vec<Sample>,
}

pub(super) fn reference() -> Result<&'static Reference, Error> {
    static VALUE: OnceLock<Result<Reference, Error>> = OnceLock::new();
    VALUE
        .get_or_init(|| Reference::new(data::catalog()))
        .as_ref()
        .map_err(Clone::clone)
}

impl Reference {
    fn new(catalog: &data::Catalog) -> Result<Self, Error> {
        let definition = definition();
        let film = catalog
            .negative
            .iter()
            .find(|f| f.id == definition.negative)
            .ok_or_else(|| crate::looks::failure("missing paper-reference dye basis"))?;
        let peaks = film
            .dyes
            .iter()
            .zip(&film.support)
            .filter(|(_, supported)| **supported)
            .fold(DVec3::ZERO, |peak, (dyes, _)| peak.max(*dyes));
        require(
            peaks.min_element() > 0.,
            "invalid paper-reference dye peaks",
        )?;
        let samples = film
            .support
            .iter()
            .enumerate()
            .filter(|(_, supported)| **supported)
            .map(|(index, _)| Sample {
                index,
                base: film.base[index] + definition.base_density_offset,
                dyes: film.dyes[index] / peaks,
            })
            .collect::<Vec<_>>();
        require(
            !samples.is_empty(),
            "empty paper-reference spectral support",
        )?;
        Ok(Self {
            definition,
            exposure: matrix(definition.exposure_rows),
            dye_basis: matrix(definition.dye_rows),
            lamp_scale: 10_f64.powf(definition.base_density_offset),
            samples,
        })
    }

    pub fn amounts(&self, rgb: DVec3) -> DVec3 {
        let records = self.exposure * rgb;
        let curve = crate::looks::characteristic::reference();
        // Coefficients are peak-OD contributions in an effective signed basis,
        // not concentrations. Total spectral density determines transmission.
        self.dye_basis * records.map(|x| curve.density(x)) * self.definition.density_gain
    }

    pub fn expose(&self, rgb: DVec3, pair: &data::Pair) -> DVec3 {
        let amounts = self.amounts(rgb);
        let mut raw = DVec3::ZERO;
        for sample in &self.samples {
            let density = sample.base + sample.dyes.dot(amounts);
            raw += pair.weights[sample.index] * self.lamp_scale * 10_f64.powf(-density);
        }
        // Fixed neutral filtration and reciprocal lamp scaling preserve paper
        // dose before preflash and both upstream epsilon additions.
        super::super::print_log_exposure(raw, pair)
    }
}

#[cfg(test)]
mod tests;
