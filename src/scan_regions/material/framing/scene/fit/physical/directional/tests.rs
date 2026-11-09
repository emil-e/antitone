use super::*;

#[test]
fn log_resolution_bounds_the_float_rounding_interval_including_subnormal_and_hdr_light() {
    for value in [
        f32::from_bits(1),
        f32::MIN_POSITIVE,
        0.5,
        1.,
        65535.,
        131070.,
        f32::MAX,
    ] {
        let bound = log_resolution([value; 3]);
        assert!(bound.is_finite() && bound > 0.);
        let v = f64::from(value);
        let lower = f64::from(value.next_down());
        let upper = if value.next_up().is_finite() {
            f64::from(value.next_up())
        } else {
            v + (v - lower)
        };
        for midpoint in [(v + lower) * 0.5, (v + upper) * 0.5] {
            // The oracle uses logarithms, independently of the production rational bound.
            let error = (midpoint.log2() - v.log2()).abs();
            let rounding = 8. * f64::EPSILON * v.log2().abs().max(1.);
            assert!(error <= bound + rounding, "{value}: {error} > {bound}");
        }
    }
}
