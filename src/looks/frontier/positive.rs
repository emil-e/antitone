//! Fixed linear-Rec.709 input convention for already-positive AP1.
use super::{
    Variant, failure,
    readout::{self, Banks, Readout},
    require,
};
use crate::{Error, looks::characteristic};
use glam::{DMat3, DVec3};
use serde::Deserialize;
use std::sync::OnceLock;

pub(super) const DEFINITION: &[u8] = include_bytes!("../../../assets/frontier/positive.json");

// Standard primaries, Bradford D60->D65; unit-neutral rows remove derivation roundoff.
const AP1_TO_LINEAR_REC709: DMat3 = DMat3::from_cols(
    DVec3::new(
        1.7050509926579833,
        -0.1302564175070437,
        -0.02400335680461801,
    ),
    DVec3::new(
        -0.6217921206570046,
        1.1408047365754017,
        -0.12896897606497054,
    ),
    DVec3::new(
        -0.0832588720009786,
        -0.010548319068358037,
        1.1529723328695884,
    ),
);

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Compression {
    knee: f64,
    limit: f64,
}
impl Compression {
    fn apply(self, value: DVec3) -> DVec3 {
        let anchor = value.max_element();
        // All-nonpositive drives share the characteristic's black endpoint.
        if anchor <= 0. {
            return value;
        }
        let distance = (anchor - value.min_element()) / anchor;
        if distance <= self.knee {
            return value;
        }
        let excess = distance - self.knee;
        let span = self.limit - self.knee;
        let mapped = self.knee + span * excess / span.hypot(excess);
        DVec3::splat(anchor) + (value - DVec3::splat(anchor)) * (mapped / distance)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    codes_per_log: f64,
    neutral_rows: Banks<DVec3>,
    compression: Compression,
}
impl Definition {
    fn get() -> Result<&'static Self, Error> {
        static VALUE: OnceLock<Result<Definition, Error>> = OnceLock::new();
        VALUE
            .get_or_init(|| {
                let d: Self = serde_json::from_slice(DEFINITION).map_err(failure)?;
                require(
                    d.codes_per_log.is_finite()
                        && d.codes_per_log > 0.
                        && d.compression.knee.is_finite()
                        && d.compression.limit.is_finite()
                        && d.compression.knee > 0.
                        && d.compression.knee < d.compression.limit
                        && d.compression.limit < 1.,
                    "invalid positive Frontier encoding",
                )?;
                require(
                    Variant::all().all(|v| {
                        let rows = d.neutral_rows.get(v);
                        rows.is_finite() && rows.min_element() > 0.
                    }),
                    "invalid positive Frontier neutral slopes",
                )?;
                Ok(d)
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

pub(super) struct Entry {
    definition: &'static Definition,
    readout: &'static Readout,
    neutral: DVec3,
    grey_coordinate: f64,
}
impl Entry {
    pub(super) fn new(variant: Variant) -> Result<Self, Error> {
        let readout = Readout::get(variant)?;
        let definition = Definition::get()?;
        Ok(Self {
            definition,
            readout,
            neutral: *definition.neutral_rows.get(variant),
            grey_coordinate: characteristic::reference().coordinate(readout.scene_grey),
        })
    }
    pub(super) fn coordinates(&self, rgb: DVec3) -> Result<DVec3, Error> {
        let mixed = AP1_TO_LINEAR_REC709 * rgb;
        require(mixed.is_finite(), "nonfinite positive Frontier drive")?;
        let shaped = self.definition.compression.apply(mixed);
        let coordinate = shaped.map(|x| characteristic::reference().coordinate(x));
        let delta = self.neutral * (coordinate - DVec3::splat(self.grey_coordinate));
        readout::coordinates(
            self.readout.grey_codes + self.definition.codes_per_log * self.readout.gains * delta,
        )
    }
}

#[cfg(test)]
mod tests;
