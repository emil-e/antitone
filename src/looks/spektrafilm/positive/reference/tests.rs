use super::*;

fn diagonal() -> Reference {
    Reference {
        to_records: DMat3::IDENTITY.to_cols_array_2d(),
        flux0: DVec3::ONE,
        samples: [DVec3::X, DVec3::Y, DVec3::Z]
            .into_iter()
            .enumerate()
            .map(|(c, response)| Sample {
                response,
                log_prior: [0.2_f64, 0.3, 0.5][c].ln(),
                receiver: response * [2., 3., 4.][c],
            })
            .collect(),
        prepared: OnceLock::new(),
    }
}

#[test]
fn separate_measuring_bands_fix_the_unique_spectrum_and_receiver_flux() {
    let r = diagonal();
    r.validate(DVec3::new(0.2, 0.7, 0.1)).unwrap();
    for q in [
        DVec3::new(0.2, 0.3, 0.5),
        DVec3::new(0.999, 0.0003, 0.0007),
        DVec3::splat(1. / 3.),
    ] {
        let (theta, at) = r.solve(q, [0, 1], 2).unwrap();
        assert!((at.mean - q).abs().max_element() < 1e-13);
        for (i, s) in r.samples.iter().enumerate() {
            let p =
                (s.log_prior + theta.dot(Reference::features(s, [0, 1])) - at.log_partition).exp();
            assert!((p - q[i]).abs() < 1e-13);
        }
    }
    for records in [DVec3::new(0.2, 0.7, 0.3), DVec3::new(40., 0.08, 0.3)] {
        let target = r.target(records).0;
        let expected = DVec3::new(2., 3., 4.).map(f64::log10) - target;
        assert!((r.field(records).unwrap() - expected).abs().max_element() < 1e-12);
    }
}

#[test]
fn low_positive_receiver_tangent_retains_displacement_in_one_and_multiple_records() {
    let r = diagonal();
    for records in [DVec3::new(0., 0.2, 0.4), DVec3::new(0.003, 0., 0.02)] {
        let anchor = records.max(DVec3::splat(BREAK));
        let (target, slope) = r.target(anchor);
        let expected = DVec3::new(2., 3., 4.).map(f64::log10)
            - target
            - (matrix(CDD_TO_CID).inverse() * (slope * (records - anchor))) / GAIN;
        let observed = r.field(records).unwrap();
        assert!((observed - expected).abs().max_element() < 1e-12);
        assert!((observed - r.field(anchor).unwrap()).abs().max_element() > 0.01);
    }
}

#[test]
fn reference_record_projection_preserves_linear_y_and_neutral_ray_inside_identity() {
    let r = diagonal();
    let w = DVec3::new(0.2, 0.7, 0.1);
    for input in [
        DVec3::new(-0.1, 0.2, 0.05),
        DVec3::new(0.8, -0.02, 0.2),
        DVec3::new(-100., 1., 1024.),
    ] {
        let y = w.dot(input);
        assert!(y > 0.);
        let projected = r.project(input, y);
        assert!(projected.min_element() >= -1e-12);
        assert!((w.dot(projected) - y).abs() < 1e-12);
        assert!(
            (projected - DVec3::splat(y))
                .cross(input - DVec3::splat(y))
                .length()
                < 1e-10
        );
        assert!(
            (r.project(input * 4., y * 4.) - projected * 4.)
                .abs()
                .max_element()
                < 1e-12
        );
    }
    let input = DVec3::new(0.3, 0.1, 0.05);
    assert_eq!(r.project(input, w.dot(input)), input);
    assert_eq!(
        r.colour_difference(DVec3::new(1., -0.5, 0.), 0.).unwrap(),
        DVec3::ZERO
    );
    assert_eq!(
        r.colour_difference(DVec3::new(1., -0.5, 0.), -0.1).unwrap(),
        DVec3::ZERO
    );
}

#[test]
fn common_flux_scale_moves_only_common_exposure_and_coordinate_choice_is_not_colour() {
    let r = diagonal();
    let q = DVec3::splat(1. / 3.);
    let (a, aa) = r.solve(q, [0, 1], 2).unwrap();
    let (b, bb) = r.solve(q, [1, 2], 0).unwrap();
    for s in &r.samples {
        let p = (s.log_prior + a.dot(Reference::features(s, [0, 1])) - aa.log_partition).exp();
        let other = (s.log_prior + b.dot(Reference::features(s, [1, 2])) - bb.log_partition).exp();
        assert!((p - other).abs() < 1e-13);
    }
    let records = DVec3::new(0.2, 0.7, 0.3);
    let h = r.field(records).unwrap();
    let mut scaled = diagonal();
    scaled.flux0 *= 4.;
    assert!(
        (scaled.field(records).unwrap() - h - DVec3::splat(4_f64.log10()))
            .abs()
            .max_element()
            < 1e-12
    );
}

#[test]
fn invalid_spectra_matrices_and_incomplete_measurement_cones_reject() {
    let w = DVec3::splat(1. / 3.);
    let mut r = diagonal();
    r.to_records = [[0.; 3]; 3];
    assert!(r.validate(w).is_err());
    let mut r = diagonal();
    r.flux0.x = 0.;
    assert!(r.validate(w).is_err());
    let mut r = diagonal();
    r.samples[0].log_prior = 0.;
    assert!(r.validate(w).is_err());
    let mut r = diagonal();
    r.samples[0].receiver.x = -1.;
    assert!(r.validate(w).is_err());
    let mut r = diagonal();
    r.samples[0].response = DVec3::new(0.5, 0.5, 0.);
    assert!(r.validate(w).is_err());
}
