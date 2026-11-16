//! Forward reference-negative toe, shared by scanner-code and paper entry.
use serde::Deserialize;
use std::sync::OnceLock;

pub(super) const DEFINITION: &[u8] = include_bytes!("../../assets/looks/reference-negative.json");

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Characteristic {
    pub gamma: f64,
    pub toe: f64,
    pub knee: f64,
    pub gray: f64,
}

pub(super) fn reference() -> &'static Characteristic {
    static VALUE: OnceLock<Characteristic> = OnceLock::new();
    VALUE.get_or_init(|| {
        let value: Characteristic = serde_json::from_slice(DEFINITION).expect("reference curve");
        assert!(value.gamma.is_finite() && value.gamma > 0.);
        assert!(value.toe.is_finite() && value.toe > 1.);
        assert!(value.knee.is_finite() && value.knee > 0.);
        assert!(value.gray.is_finite() && value.gray > 0.);
        value
    })
}

impl Characteristic {
    /// Log-exposure coordinate, zero at black and asymptotically ln(signal/knee).
    /// Nonpositive signals share the black endpoint; upstream AP1 is unchanged.
    pub fn coordinate(&self, signal: f64) -> f64 {
        if signal <= 0. {
            return 0.;
        }
        let z = self.toe * (signal.ln() - self.knee.ln());
        let softplus = z.max(0.) + (-z.abs()).exp().ln_1p();
        softplus / self.toe
    }

    /// Base-10 optical-density increment above the reference base.
    pub fn density(&self, signal: f64) -> f64 {
        self.coordinate(signal) * (self.gamma / std::f64::consts::LN_10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::processing::{Calibration, Inversion, NegativeInput, Response};
    use glam::DVec3;

    #[test]
    fn forward_curve_inverts_the_photographic_no_shoulder_response() {
        let curve = reference();
        let base = 0.8;
        let white = base * (-curve.density(1.) * std::f64::consts::LN_10).exp();
        let inversion = Inversion::new(NegativeInput {
            calibration: Calibration {
                dmin: DVec3::splat(base),
                dmax: DVec3::splat(white),
            },
            response: Response {
                toe: curve.toe,
                shoulder: 0.,
            },
            gamma: curve.gamma,
            balance: Default::default(),
        })
        .unwrap();
        for stops in -24..=24 {
            let positive = curve.gray * 2_f64.powf(stops as f64 / 2.);
            let transmission = base * 10_f64.powf(-curve.density(positive));
            let actual = inversion.apply(DVec3::splat(transmission)).unwrap().rgb.x;
            assert!((actual - positive).abs() < 2e-10 * positive.max(1.));
        }
    }

    #[test]
    fn black_toe_and_straight_line_have_independent_analytic_controls() {
        let c = reference();
        assert_eq!(c.density(0.), 0.);
        assert_eq!(c.density(-1.), 0.);
        assert!((c.density(c.knee) - c.gamma / c.toe * 2_f64.log10()).abs() < 1e-15);
        let a = c.density(c.knee * 1e6);
        let b = c.density(c.knee * 1e7);
        assert!((b - a - c.gamma).abs() < 2e-15);
        let values = [-1., 0., 1e-100, 1e-12, c.knee, c.gray, 1., 1024., 1e100];
        let density = values.map(|x| c.density(x));
        assert!(density.iter().all(|x| x.is_finite()));
        assert!(density.windows(2).all(|v| v[0] <= v[1]));
    }
}
