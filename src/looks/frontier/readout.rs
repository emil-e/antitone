//! Shared native-bank grey placement and numeric code-domain continuation.
use super::{Variant, failure, require};
use crate::Error;
use glam::DVec3;
use serde::Deserialize;
use std::sync::OnceLock;

pub(super) const DEFINITION: &[u8] = include_bytes!("../../../assets/frontier/readout.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Banks<T> {
    #[serde(rename = "A")]
    a: T,
    #[serde(rename = "B")]
    b: T,
}
impl<T> Banks<T> {
    pub(super) fn get(&self, variant: Variant) -> &T {
        match variant {
            Variant::A => &self.a,
            Variant::B => &self.b,
        }
    }
}

pub(super) struct Readout {
    pub(super) scene_grey: f64,
    pub(super) gains: DVec3,
    pub(super) grey_codes: DVec3,
}
impl Readout {
    pub(super) fn get(variant: Variant) -> Result<&'static Self, Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Definition {
            scene_grey: f64,
            #[serde(rename = "gains_RGB")]
            gains: DVec3,
            native_grey_codes: Banks<DVec3>,
        }
        static VALUE: OnceLock<Result<Banks<Readout>, Error>> = OnceLock::new();
        let banks = VALUE
            .get_or_init(|| {
                let d: Definition = serde_json::from_slice(DEFINITION).map_err(failure)?;
                require(
                    d.scene_grey.is_finite()
                        && d.scene_grey > 0.
                        && d.gains.is_finite()
                        && d.gains.min_element() > 0.,
                    "invalid Frontier readout reference",
                )?;
                let bank = |variant| -> Result<Readout, Error> {
                    let grey_codes = *d.native_grey_codes.get(variant);
                    require(
                        grey_codes.is_finite()
                            && grey_codes.min_element() >= 0.
                            && grey_codes.max_element() <= 1015.,
                        "invalid native Frontier grey coordinate",
                    )?;
                    Ok(Readout {
                        scene_grey: d.scene_grey,
                        gains: d.gains,
                        grey_codes,
                    })
                };
                Ok(Banks {
                    a: bank(Variant::A)?,
                    b: bank(Variant::B)?,
                })
            })
            .as_ref()
            .map_err(Clone::clone)?;
        Ok(banks.get(variant))
    }
}

#[cfg(test)]
mod tests;

pub(super) fn coordinates(raw: DVec3) -> Result<DVec3, Error> {
    require(raw.is_finite(), "nonfinite Frontier readout")?;
    Ok(raw.map(|q| {
        if q < 0. {
            0.
        } else if q > 1015. {
            1023. - 64. / (q - 1007.)
        } else {
            q
        }
    }))
}
