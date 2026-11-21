//! Nominal colour-negative formation and mean-preserving spectral scanner readout.
use super::{
    Variant, failure,
    readout::{self, Banks, Readout},
    require,
};
use crate::{Error, looks::spektrafilm};
use glam::DVec3;
use serde::Deserialize;
use std::sync::{OnceLock, atomic::AtomicBool};

pub(super) const DEFINITION: &[u8] = include_bytes!("../../../assets/frontier/negative.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    #[serde(rename = "codes_per_OD")]
    codes_per_od: f64,
    neutral_rows: Banks<DVec3>,
}
impl Definition {
    fn get() -> Result<&'static Self, Error> {
        static VALUE: OnceLock<Result<Definition, Error>> = OnceLock::new();
        VALUE
            .get_or_init(|| {
                let d: Self = serde_json::from_slice(DEFINITION).map_err(failure)?;
                require(
                    d.codes_per_od.is_finite()
                        && d.codes_per_od > 0.
                        && Variant::all().all(|v| {
                            let rows = d.neutral_rows.get(v);
                            rows.is_finite() && rows.min_element() > 0.
                        }),
                    "invalid stock Frontier readout slopes",
                )?;
                Ok(d)
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

struct BandSample {
    dyes: DVec3,
    weight: DVec3,
}

struct Scanner {
    formation: spektrafilm::Formation,
    base: DVec3,
    bands: Vec<BandSample>,
}

pub(super) struct Entry {
    scanner: Scanner,
    readout: &'static Readout,
    definition: &'static Definition,
    neutral: DVec3,
    grey: DVec3,
    balance: DVec3,
}
impl Entry {
    pub(super) fn new(
        stock: spektrafilm::Negative,
        variant: Variant,
        cancel: &AtomicBool,
    ) -> Result<Self, Error> {
        let readout = Readout::get(variant)?;
        let definition = Definition::get()?;
        let scanner = Scanner::new(stock, cancel)?;
        let grey = scanner.optical_density(DVec3::splat(readout.scene_grey));
        require(
            grey.is_finite() && grey.min_element() > 0.,
            "invalid negative scanner grey reference",
        )?;
        let balance = DVec3::splat(grey.element_sum() / 3.) / grey;
        Ok(Self {
            scanner,
            readout,
            definition,
            grey,
            balance,
            neutral: *definition.neutral_rows.get(variant),
        })
    }

    fn raw_codes(&self, optical_density: DVec3) -> DVec3 {
        let centred = (optical_density - self.grey) * self.balance;
        self.readout.grey_codes
            + self.definition.codes_per_od * (self.neutral * centred) * self.readout.gains
    }

    pub(super) fn coordinates(&self, rgb: DVec3) -> Result<DVec3, Error> {
        readout::coordinates(self.raw_codes(self.scanner.optical_density(rgb)))
    }
}

impl Scanner {
    fn new(stock: spektrafilm::Negative, cancel: &AtomicBool) -> Result<Self, Error> {
        let formation = spektrafilm::Formation::new(stock, cancel)?;
        let base = formation.density(DVec3::ZERO);
        let sensor = sensor()?;
        let mut bands = Vec::new();
        let mut norm = DVec3::ZERO;
        for (index, dyes, base_spectrum) in formation.spectrum() {
            let weight = sensor[index] * 10_f64.powf(-dyes.dot(base) - base_spectrum);
            norm += weight;
            bands.push(BandSample { dyes, weight });
        }
        require(
            norm.is_finite() && norm.min_element() > 0.,
            "unsupported negative/scanner bands",
        )?;
        for sample in &mut bands {
            sample.weight /= norm;
        }
        Ok(Self {
            formation,
            base,
            bands,
        })
    }

    fn attenuation(&self, density: DVec3) -> DVec3 {
        let delta = density - self.base;
        if delta.abs().max_element() < 0.001 {
            let change = self.bands.iter().fold(DVec3::ZERO, |sum, sample| {
                sum + sample.weight * (-std::f64::consts::LN_10 * sample.dyes.dot(delta)).exp_m1()
            });
            -change.map(f64::ln_1p)
        } else {
            -self
                .bands
                .iter()
                .fold(DVec3::ZERO, |sum, sample| {
                    sum + sample.weight * 10_f64.powf(-sample.dyes.dot(delta))
                })
                .map(f64::ln)
        }
    }

    fn optical_density(&self, rgb: DVec3) -> DVec3 {
        self.attenuation(self.formation.density(rgb)) / std::f64::consts::LN_10
    }
}

fn sensor() -> Result<&'static [DVec3], Error> {
    #[derive(serde::Deserialize)]
    struct Sensor {
        schema: u32,
        #[serde(rename = "wavelength_RGB")]
        bands: Vec<[f64; 4]>,
    }
    static VALUE: OnceLock<Result<Vec<DVec3>, Error>> = OnceLock::new();
    VALUE
        .get_or_init(|| {
            let data: Sensor =
                serde_json::from_str(include_str!("../../../assets/frontier/sensor.json"))
                    .map_err(super::failure)?;
            require(
                data.schema == 1 && data.bands.len() == 81,
                "invalid scanner band dimensions",
            )?;
            require(
                data.bands.iter().enumerate().all(|(i, row)| {
                    row[0] == 380. + 5. * i as f64
                        && row[1..].iter().all(|v| v.is_finite() && *v >= 0.)
                }),
                "invalid scanner spectrum",
            )?;
            Ok(data
                .bands
                .into_iter()
                .map(|row| DVec3::new(row[1], row[2], row[3]))
                .collect())
        })
        .as_ref()
        .map(Vec::as_slice)
        .map_err(Clone::clone)
}

#[cfg(test)]
mod tests;
