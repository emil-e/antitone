//! Convex APD-action spectral selection and adapter-local reference-record mapping.
use crate::{
    Error,
    looks::{failure, require},
};
use glam::{DMat2, DMat3, DVec2, DVec3};
use serde::Deserialize;
use std::sync::OnceLock;

const BREAK: f64 = 1. / 128.;
const GAIN: DVec3 = DVec3::new(1., 0.92, 0.95);
const CID: [f64; 11] = [
    -0.19, 0.01, 0.028, 0.054, 0.095, 0.145, 0.220, 0.300, 0.400, 0.500, 0.600,
];
const LOG_EXPOSURE: [f64; 11] = [
    -6.,
    -2.721718645,
    -2.521718645,
    -2.321718645,
    -2.121718645,
    -1.921718645,
    -1.721718645,
    -1.521718645,
    -1.321718645,
    -1.121718645,
    -0.926545676714876,
];
const CDD_TO_CID: [[f64; 3]; 3] = [
    [0.75573, 0.22197, 0.02230],
    [0.05901, 0.96928, -0.02829],
    [0.16134, 0.07406, 0.76460],
];

fn matrix(rows: [[f64; 3]; 3]) -> DMat3 {
    DMat3::from_cols_array_2d(&rows).transpose()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Sample {
    response: DVec3,
    log_prior: f64,
    receiver: DVec3,
}

struct Prepared {
    to_records: DMat3,
    white: DVec3,
    inverse_cdd: DMat3,
    prior_response: DVec3,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reference {
    to_records: [[f64; 3]; 3],
    flux0: DVec3,
    samples: Vec<Sample>,
    #[serde(skip)]
    prepared: OnceLock<Prepared>,
}

struct Moments {
    log_partition: f64,
    mean: DVec3,
    covariance: DMat2,
}

impl Reference {
    pub fn validate(&self, luminance: DVec3) -> Result<(), Error> {
        let m = matrix(self.to_records);
        require(
            m.is_finite() && m.determinant().abs() > 1e-10,
            "invalid reference exposure matrix",
        )?;
        require(
            (m * DVec3::ONE).min_element() > 0.
                && (m.inverse().transpose() * luminance).min_element() > 0.
                && self.flux0.is_finite()
                && self.flux0.min_element() > 0.,
            "invalid reference photometric basis",
        )?;
        require(
            self.samples.len() >= 3
                && self.samples.iter().all(|s| {
                    s.response.is_finite()
                        && s.response.min_element() >= 0.
                        && (s.response.element_sum() - 1.).abs() < 1e-12
                        && s.log_prior.is_finite()
                        && s.log_prior.exp() > 0.
                        && s.receiver.is_finite()
                        && s.receiver.min_element() >= 0.
                }),
            "invalid reference spectral sample",
        )?;
        let mass = self.samples.iter().map(|s| s.log_prior.exp()).sum::<f64>();
        let receiver = self.samples.iter().map(|s| s.receiver).sum::<DVec3>();
        require(
            (mass - 1.).abs() < 1e-12 && receiver.min_element() > 0.,
            "invalid reference prior or receiver",
        )?;
        // Pure-channel columns establish the full positive measurement cone.
        for axis in [DVec3::X, DVec3::Y, DVec3::Z] {
            require(
                self.samples.iter().any(|s| s.response == axis),
                "incomplete reference measurement cone",
            )?;
        }
        let p = self.prepared();
        require(
            p.prior_response.is_finite() && p.prior_response.min_element() > 0.,
            "invalid prepared reference prior",
        )
    }

    fn prepared(&self) -> &Prepared {
        self.prepared.get_or_init(|| {
            let to_records = matrix(self.to_records);
            Prepared {
                to_records,
                white: to_records * DVec3::ONE,
                inverse_cdd: matrix(CDD_TO_CID).inverse(),
                prior_response: self
                    .samples
                    .iter()
                    .map(|s| s.response * s.log_prior.exp())
                    .sum(),
            }
        })
    }

    fn features(sample: &Sample, kept: [usize; 2]) -> DVec2 {
        DVec2::new(sample.response[kept[0]], sample.response[kept[1]])
    }

    fn moments(&self, theta: DVec2, kept: [usize; 2]) -> Moments {
        let mut maximum = f64::NEG_INFINITY;
        for s in &self.samples {
            let value =
                s.log_prior + (theta.x * s.response[kept[0]] + theta.y * s.response[kept[1]]);
            maximum = maximum.max(value);
        }
        let (mut mass, mut x, mut y, mut z) = (0., 0., 0., 0.);
        let (mut xx, mut xy, mut yy) = (0., 0., 0.);
        // Scalar moments avoid constructing a matrix for every wavelength in
        // every Newton iteration, including unoptimized interactive builds.
        for s in &self.samples {
            let a = s.response[kept[0]];
            let b = s.response[kept[1]];
            let p = (s.log_prior + (theta.x * a + theta.y * b) - maximum).exp();
            mass += p;
            x += s.response.x * p;
            y += s.response.y * p;
            z += s.response.z * p;
            xx += a * a * p;
            xy += a * b * p;
            yy += b * b * p;
        }
        let inverse_mass = mass.recip();
        let mean = DVec3::new(x * inverse_mass, y * inverse_mass, z * inverse_mass);
        let a = mean[kept[0]];
        let b = mean[kept[1]];
        let cross = xy * inverse_mass - a * b;
        Moments {
            log_partition: maximum + mass.ln(),
            mean,
            covariance: DMat2::from_cols(
                DVec2::new(xx * inverse_mass - a * a, cross),
                DVec2::new(cross, yy * inverse_mass - b * b),
            ),
        }
    }

    fn solve(&self, q: DVec3, kept: [usize; 2], omitted: usize) -> Result<(DVec2, Moments), Error> {
        let prior = self.prepared().prior_response;
        let target = DVec2::new(q[kept[0]], q[kept[1]]);
        let mut theta = DVec2::from_array(
            kept.map(|c| (q[c] / q[omitted]).ln() - (prior[c] / prior[omitted]).ln()),
        );
        let mut at = self.moments(theta, kept);
        for _ in 0..64 {
            let residual = (at.mean / q).map(f64::ln).abs().max_element();
            if residual < 2e-13 {
                return Ok((theta, at));
            }
            require(
                at.covariance.is_finite() && at.covariance.determinant() > 0.,
                "singular reference moment covariance",
            )?;
            let gradient = DVec2::new(at.mean[kept[0]], at.mean[kept[1]]) - target;
            let step = at.covariance.inverse() * gradient;
            require(step.is_finite(), "nonfinite reference moment step")?;
            let objective = at.log_partition - theta.dot(target);
            let directional = gradient.dot(step);
            let mut scale = 1.;
            let mut accepted = None;
            for _ in 0..32 {
                let candidate = theta - step * scale;
                let next = self.moments(candidate, kept);
                let value = next.log_partition - candidate.dot(target);
                // Merit-function roundoff allowance never relaxes the flux residual.
                if value
                    <= objective - 1e-4 * scale * directional
                        + 8. * f64::EPSILON * objective.abs().max(1.)
                {
                    accepted = Some((candidate, next));
                    break;
                }
                scale *= 0.5;
            }
            let next = accepted
                .ok_or_else(|| failure("reference moment solve did not decrease objective"))?;
            theta = next.0;
            at = next.1;
        }
        Err(failure("reference moment solve did not converge"))
    }

    fn target(&self, records: DVec3) -> (DVec3, DVec3) {
        let mut cid = DVec3::ZERO;
        let mut slope = DVec3::ZERO;
        for c in 0..3 {
            let log = records[c].log10();
            if log > LOG_EXPOSURE[10] {
                cid[c] = 0.55 * (log + 0.7 / 0.55 - 0.18_f64.log10());
                slope[c] = 0.55;
            } else {
                let k = LOG_EXPOSURE
                    .partition_point(|&v| v <= log)
                    .saturating_sub(1)
                    .min(9);
                slope[c] = (CID[k + 1] - CID[k]) / (LOG_EXPOSURE[k + 1] - LOG_EXPOSURE[k]);
                cid[c] = CID[k] + (log - LOG_EXPOSURE[k]) * slope[c];
            }
            slope[c] /= records[c] * std::f64::consts::LN_10;
        }
        (self.prepared().inverse_cdd * cid / GAIN, slope)
    }

    fn field(&self, records: DVec3) -> Result<DVec3, Error> {
        let anchor = records.max(DVec3::splat(BREAK));
        let (target, slope) = self.target(anchor);
        let log_flux = self.flux0.map(f64::ln) - target * std::f64::consts::LN_10;
        let maximum = log_flux.max_element();
        let flux = (log_flux - DVec3::splat(maximum)).map(f64::exp);
        let q = flux / flux.element_sum();
        let log_total = maximum + flux.element_sum().ln();
        let omitted = if q.x >= q.y && q.x >= q.z {
            0
        } else if q.y >= q.z {
            1
        } else {
            2
        };
        let kept = match omitted {
            0 => [1, 2],
            1 => [0, 2],
            _ => [0, 1],
        };
        let (theta, at) = self.solve(q, kept, omitted)?;
        let mut receiver = DVec3::ZERO;
        let mut cross = [DVec3::ZERO; 2];
        for s in &self.samples {
            let feature = Self::features(s, kept);
            let p = (s.log_prior + theta.dot(feature) - at.log_partition).exp();
            receiver += s.receiver * p;
            cross[0] += s.receiver * (p * feature.x);
            cross[1] += s.receiver * (p * feature.y);
        }
        // The displacement retains the existing low-positive receiver tangent;
        // the positive query anchor is not returned as a clipped colour.
        let delta_log_flux = -std::f64::consts::LN_10
            * (self.prepared().inverse_cdd * (slope * (records - anchor)))
            / GAIN;
        let delta_total = q.dot(delta_log_flux);
        let delta_q = DVec2::from_array(kept.map(|c| q[c] * (delta_log_flux[c] - delta_total)));
        let delta_theta = at.covariance.inverse() * delta_q;
        let extension = DVec3::splat(delta_total)
            + (cross[0] / receiver - DVec3::splat(at.mean[kept[0]])) * delta_theta.x
            + (cross[1] / receiver - DVec3::splat(at.mean[kept[1]])) * delta_theta.y;
        let result =
            (DVec3::splat(log_total) + receiver.map(f64::ln) + extension) / std::f64::consts::LN_10;
        require(result.is_finite(), "nonfinite reference receiver exposure")?;
        Ok(result)
    }

    fn project(&self, rgb: DVec3, brightness: f64) -> DVec3 {
        let r = self.prepared().to_records * rgb;
        if r.min_element() >= 0. {
            return r;
        }
        let neutral = self.prepared().white * brightness;
        let distance = ((neutral - r) / neutral).max_element();
        neutral + (r - neutral) / distance
    }

    pub fn colour_difference(&self, rgb: DVec3, brightness: f64) -> Result<DVec3, Error> {
        if brightness <= 0. {
            return Ok(DVec3::ZERO);
        }
        let records = self.project(rgb, brightness);
        let neutral = self.prepared().white * brightness;
        Ok(self.field(records)? - self.field(neutral)?)
    }
}

#[cfg(test)]
mod tests;
