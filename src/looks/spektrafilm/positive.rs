//! Reference-negative paper entry with native-tone exposure anchoring.
mod reference;
use super::{data, develop_print_morph, neutral_exposure};
use crate::{
    Error,
    looks::{failure, require},
};
use glam::DVec3;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Photometry {
    pub dyes: Vec<DVec3>,
    pub weights: Vec<f64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Driver {
    pub input_luminance: DVec3,
    pub photometry: Photometry,
    reference: reference::Reference,
}

impl Driver {
    pub fn validate(&self) -> Result<(), Error> {
        require(
            self.input_luminance.is_finite()
                && self.input_luminance.min_element() > 0.
                && (self.input_luminance.element_sum() - 1.).abs() < 1e-10
                && self.photometry.dyes.len() >= 3
                && self.photometry.dyes.len() == self.photometry.weights.len()
                && self.photometry.dyes.iter().all(|v| v.is_finite())
                && self
                    .photometry
                    .weights
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.)
                && self.photometry.weights.iter().sum::<f64>() > 0.,
            "invalid brightness-anchored paper reference data",
        )?;
        self.reference.validate(self.input_luminance)
    }

    /// Original spectral viewed Y; base density is folded into the positive weights.
    pub fn luminance(&self, paper: &data::Paper, h: DVec3) -> f64 {
        let density = develop_print_morph(paper, h);
        self.photometry
            .dyes
            .iter()
            .zip(&self.photometry.weights)
            .map(|(dye, weight)| weight * (-std::f64::consts::LN_10 * dye.dot(density)).exp())
            .sum()
    }

    fn luminance_and_slope(&self, paper: &data::Paper, h: DVec3) -> (f64, f64) {
        let density = develop_print_morph(paper, h);
        let slope = DVec3::from_array(std::array::from_fn(|c| {
            let k = paper.log_exposure.partition_point(|&x| x <= h[c]);
            if k == 0 || k == paper.log_exposure.len() {
                0.
            } else {
                (paper.density_curves[k][c] - paper.density_curves[k - 1][c])
                    / (paper.log_exposure[k] - paper.log_exposure[k - 1])
            }
        }));
        self.photometry
            .dyes
            .iter()
            .zip(&self.photometry.weights)
            .fold((0., 0.), |(y, dy), (dye, weight)| {
                let value = weight * (-std::f64::consts::LN_10 * dye.dot(density)).exp();
                (
                    y + value,
                    dy - std::f64::consts::LN_10 * value * dye.dot(slope),
                )
            })
    }

    pub fn expose(
        &self,
        rgb: DVec3,
        paper: &data::Paper,
        mapping: &data::Mapping,
    ) -> Result<DVec3, Error> {
        if rgb.x == rgb.y && rgb.y == rgb.z {
            // Preserve the entire neutral input law, including signed and HDR values.
            return Ok(neutral_exposure(paper, rgb.x, mapping));
        }
        let brightness = self.input_luminance.dot(rgb);
        let neutral = neutral_exposure(paper, brightness, mapping);
        let first = paper.log_exposure[0];
        let last = *paper.log_exposure.last().expect("validated paper curve");
        // At either constant material endpoint the target is independent of colour.
        if neutral.max_element() <= first || neutral.min_element() >= last {
            return Ok(neutral);
        }
        let h = neutral + self.reference.colour_difference(rgb, brightness)?;
        let target = self.luminance(paper, neutral);
        let mut low = first - h.max_element();
        let mut high = last - h.min_element();
        let mut offset = 0.;
        for _ in 0..64 {
            let at = h + DVec3::splat(offset);
            let (y, slope) = self.luminance_and_slope(paper, at);
            let error = y - target;
            if error.abs() <= 2. * f64::EPSILON * target {
                return Ok(at);
            }
            if error > 0. {
                low = offset;
            } else {
                high = offset;
            }
            if high - low < 2e-13 {
                let result = h + DVec3::splat((low + high) * 0.5);
                require(
                    (self.luminance(paper, result) - target).abs() < 1e-11,
                    "paper brightness solve did not converge",
                )?;
                return Ok(result);
            }
            let next = offset - error / slope;
            offset = if next.is_finite() && next > low && next < high {
                next
            } else {
                (low + high) * 0.5
            };
        }
        Err(failure("paper brightness solve did not converge"))
    }
}

#[cfg(test)]
mod tests;
