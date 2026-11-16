//! Fixed-exposure colour-negative readout and black-anchored reproduction-code calibration.
use super::{Variant, matrix, require};
use crate::{Error, looks::spektrafilm};
use glam::DVec3;
use std::sync::{OnceLock, atomic::AtomicBool};

struct BandSample {
    dyes: DVec3,
    weight: DVec3,
}

pub(super) struct Entry {
    formation: spektrafilm::Formation,
    base: DVec3,
    bands: Vec<BandSample>,
    balance: DVec3,
    origin: f64,
}

impl Entry {
    pub(super) fn new(
        stock: spektrafilm::Negative,
        variant: Variant,
        cancel: &AtomicBool,
    ) -> Result<Self, Error> {
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
        let mut entry = Self {
            formation,
            base,
            bands,
            balance: DVec3::ONE,
            origin: variant.code_offset(),
        };
        let gray = entry.optical_density(DVec3::splat(0.18));
        require(
            gray.is_finite() && gray.min_element() > 0.,
            "invalid negative scanner grey reference",
        )?;
        // Define the neutral density axis without equalizing stocks' mean grey OD.
        entry.balance = DVec3::splat(gray.element_sum() / 3.) / gray;
        Ok(entry)
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

    fn raw_codes(&self, optical_density: DVec3) -> DVec3 {
        // One A-calibrated density-space characterization; B changes only its source origin.
        let colour = matrix([
            [
                0.9749188050172526,
                0.03164970403459537,
                -0.006568509051847915,
            ],
            [
                -0.10026808253832398,
                1.0523759610528092,
                0.04789212148551488,
            ],
            [0.14823225898420794, 0.016360899739573016, 0.835406841276219],
        ]) * (optical_density * self.balance);
        DVec3::splat(self.origin)
            + 515. * colour * DVec3::new(1.0389265051082284, 1., 1.0318751992283606)
    }

    pub(super) fn coordinates(&self, rgb: DVec3) -> Result<DVec3, Error> {
        let codes = self.raw_codes(self.optical_density(rgb));
        require(codes.is_finite(), "nonfinite negative scanner readout")?;
        // Numeric code-domain continuation, separate from the stock characteristic.
        Ok(codes.map(|code| {
            if code < 0. {
                0.
            } else if code > 1015. {
                1023. - 64. / (code - 1007.)
            } else {
                code
            }
        }))
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
