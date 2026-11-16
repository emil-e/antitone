//! ACEScc contrast coordinate relative to encoded zero, with a signed tangent extension.
//! Positive branches follow https://docs.acescentral.com/encodings/acescc/.

pub(super) const FOOT: f64 = 1. / 32768.;
const SCALE: f64 = 17.52;
const LOG_SCALE: f64 = SCALE * std::f64::consts::LN_2;
const SLOPE: f64 = 1. / (FOOT * LOG_SCALE);
const CUT: f64 = 1. / SCALE;

pub(super) fn encode(x: f64) -> f64 {
    if x <= 0. {
        SLOPE * x
    } else if x < FOOT {
        (x / FOOT).ln_1p() / LOG_SCALE
    } else {
        (x.log2() + 16.) / SCALE
    }
}

fn decode(q: f64) -> f64 {
    if q <= 0. {
        q / SLOPE
    } else if q < CUT {
        FOOT * (q * LOG_SCALE).exp_m1()
    } else {
        (q * SCALE - 16.).exp2()
    }
}

pub(super) fn tone(x: f64, amount: f64, pivot: f64, qp: f64) -> f64 {
    if x == pivot {
        x
    } else {
        decode(qp + amount * (encode(x) - qp))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_coordinate_roundtrips_without_an_interchange_ceiling() {
        for x in [-1e10, -1., -1e-15, 0., 1e-15, FOOT / 2., FOOT, 0.18, 1e10] {
            let actual = decode(encode(x));
            assert!((actual - x).abs() <= 2e-14 * x.abs(), "{x}: {actual}");
        }
    }

    #[test]
    fn negative_tangent_matches_the_positive_foot_at_zero() {
        let h = 1e-12;
        assert_eq!(encode(0.), 0.);
        let left = -encode(-h) / h;
        let right = encode(h) / h;
        assert!((left / right - 1.).abs() < 2e-8);
        let left = -decode(-h) / h;
        let right = decode(h) / h;
        assert!((left / right - 1.).abs() < 2e-8);
    }
}
