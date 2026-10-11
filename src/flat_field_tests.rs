use super::*;

#[cfg(feature = "raw")]
#[test]
fn production_rectangle_proposal_handles_different_openings_dust_and_ambiguity() {
    for (w, h, left, top, right, bottom) in [
        (101, 61, 3, 2, 97, 58),
        (81, 79, 14, 9, 67, 68),
        (65, 49, 5, 3, 61, 45),
    ] {
        let mut values: Vec<_> = (0..h)
            .flat_map(|y| {
                (0..w).map(move |x| {
                    if x < left || x >= right || y < top || y >= bottom {
                        0.01
                    } else {
                        1. - 0.2 * ((x as f64 - w as f64 / 2.) / (w as f64 / 2.)).powi(2)
                            - 0.2 * ((y as f64 - h as f64 / 2.) / (h as f64 / 2.)).powi(2)
                    }
                })
            })
            .collect();
        values[(top + 3) * w + left + 3] = 0.1;
        assert_eq!(
            propose_support(&values, [w, h], 1.),
            Some([left + 1, top + 1, right - 1, bottom - 1])
        );
    }
    let (w, h) = (81, 61);
    let round: Vec<_> = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                if (x as f64 - 40.).hypot(y as f64 - 30.) < 24. {
                    1.
                } else {
                    0.01
                }
            })
        })
        .collect();
    assert_eq!(propose_support(&round, [w, h], 1.), None);
    assert_eq!(propose_support(&vec![0.; w * h], [w, h], 1.), None);
}
#[test]
fn bilinear_source_coordinates_and_identity_outside_support_are_explicit() {
    let field = FlatField::from_nodes(
        [100, 200, 80, 40],
        [2, 2],
        vec![
            DVec3::ONE,
            DVec3::splat(2.),
            DVec3::splat(3.),
            DVec3::splat(4.),
        ],
    )
    .unwrap();
    assert_eq!(field.gain([100., 200.]).unwrap(), DVec3::ONE);
    assert_eq!(field.gain([140., 220.]).unwrap(), DVec3::splat(2.5));
    for p in [[99.5, 200.5], [180., 220.], [140., 240.], [0., 0.]] {
        assert_eq!(field.gain(p).unwrap(), DVec3::ONE);
    }
    assert!(field.gain([f64::NAN, 0.]).is_err());
    assert_eq!(field.bounds(), (DVec3::ONE, DVec3::splat(4.)));
}
#[test]
fn malformed_geometry_and_unrepresentable_nodes_are_rejected() {
    for area in [[0, 0, 0, 12], [u32::MAX, 0, 2, 2], [0, u32::MAX, 2, 2]] {
        assert!(FlatField::from_nodes(area, [2, 2], vec![DVec3::ONE; 4]).is_err());
    }
    for size in [[1, 2], [257, 2], [2, 0]] {
        assert!(FlatField::from_nodes([0, 0, 80, 60], size, vec![]).is_err());
    }
    for bad in [0., -1., f64::NAN, f64::INFINITY, 1e-100, 1e100] {
        assert!(FlatField::from_nodes([0, 0, 80, 60], [2, 2], vec![DVec3::splat(bad); 4]).is_err());
    }
}
#[test]
fn global_tint_is_neutralized_in_ap1_while_center_luminance_is_preserved() {
    let camera = DVec3::new(0.11, 0.09, 0.13);
    let matrix = DMat3::from_diagonal(DVec3::new(1.2, 0.8, 1.4));
    let field = FlatField::estimate(
        [0, 0, 800, 600],
        [21, 17],
        &vec![camera; 21 * 17],
        matrix,
        camera,
        [400., 300.],
    )
    .unwrap();
    let y = (AP1_TO_XYZ * (matrix * camera)).y;
    for point in [[400., 300.], [0.5, 0.5], [799.5, 599.5]] {
        let corrected = matrix * (camera * field.gain(point).unwrap());
        assert!((corrected - DVec3::splat(y)).abs().max_element() < 2e-12);
    }
    let neutral_camera = matrix.inverse() * DVec3::splat(y);
    assert_ne!(neutral_camera.x, neutral_camera.y);
    assert_eq!(field.nodes().len(), 21 * 17);
    assert_eq!(field.area(), [0, 0, 800, 600]);
    assert_eq!(field.size(), [21, 17]);
}
#[test]
fn smooth_nonradial_color_shading_is_removed_including_boundaries() {
    let [w, h] = [81, 61];
    let area = [50, 70, 1600, 1200];
    let light = |x: f64, y: f64| {
        DVec3::new(
            0.12 * (-0.18 * x * x - 0.10 * y * y + 0.03 * x * y).exp(),
            0.1 * (-0.10 * x * x - 0.16 * y * y + 0.025 * x).exp(),
            0.14 * (-0.15 * x * x - 0.08 * y * y - 0.03 * y).exp(),
        )
    };
    let means: Vec<_> = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                light(
                    (2. * (x as f64 + 0.5) / w as f64) - 1.,
                    (2. * (y as f64 + 0.5) / h as f64) - 1.,
                )
            })
        })
        .collect();
    let center = light(0., 0.);
    let y = (AP1_TO_XYZ * center).y;
    let field =
        FlatField::estimate(area, [w, h], &means, DMat3::IDENTITY, center, [850., 670.]).unwrap();
    assert!(
        (center * field.gain([850., 670.]).unwrap() - DVec3::splat(y))
            .abs()
            .max_element()
            < 2e-12
    );
    for point in [
        [50.5, 70.5],
        [1649.5, 70.5],
        [50.5, 1269.5],
        [1649.5, 1269.5],
        [553., 899.],
    ] {
        let source = light(
            2. * (point[0] - 50.) / 1600. - 1.,
            2. * (point[1] - 70.) / 1200. - 1.,
        );
        let corrected = source * field.gain(point).unwrap();
        assert!(
            (corrected / DVec3::splat(y) - DVec3::ONE)
                .abs()
                .max_element()
                < 0.0004,
            "point={point:?} corrected={corrected:?}"
        );
    }
}
#[test]
fn isolated_dust_does_not_become_a_local_compensation_spike() {
    let size = [41, 31];
    let center = DVec3::new(0.12, 0.1, 0.14);
    let clean = vec![center; 41 * 31];
    let mut dusty = clean.clone();
    dusty[12 * 41 + 9] *= 0.2;
    let a = FlatField::estimate(
        [0, 0, 820, 620],
        size,
        &clean,
        DMat3::IDENTITY,
        center,
        [410., 310.],
    )
    .unwrap();
    let b = FlatField::estimate(
        [0, 0, 820, 620],
        size,
        &dusty,
        DMat3::IDENTITY,
        center,
        [410., 310.],
    )
    .unwrap();
    for (ga, gb) in a.nodes().iter().zip(b.nodes()) {
        assert!((*ga - *gb).abs().max_element() < 1e-10);
    }
}
#[test]
fn pixel_grid_noise_is_attenuated_instead_of_becoming_gain_texture() {
    let (w, h) = (61, 41);
    let center = DVec3::splat(0.1);
    let means: Vec<_> = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                center
                    * if (x + y) % 2 == 0 {
                        0.002_f64.exp()
                    } else {
                        (-0.002_f64).exp()
                    }
            })
        })
        .collect();
    let field = FlatField::estimate(
        [0, 0, 1220, 820],
        [w, h],
        &means,
        DMat3::IDENTITY,
        center,
        [610., 410.],
    )
    .unwrap();
    let rms = (field
        .nodes()
        .iter()
        .map(|g| (*g - DVec3::ONE).length_squared())
        .sum::<f64>()
        / (3 * w * h) as f64)
        .sqrt();
    assert!(rms < 0.00015, "gain noise rms={rms}");
}
#[test]
fn corner_trend_is_not_mistaken_for_dust() {
    let (w, h) = (61, 41);
    let values: Vec<_> = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                0.001 * x as f64
                    - 0.002 * y as f64
                    - 0.00008 * (x * x) as f64
                    - 0.0001 * (y * y) as f64
                    + 0.0002 * (x * y) as f64
            })
        })
        .collect();
    let smoothed = smooth_surface(&values, w, h).unwrap();
    for (a, b) in values.iter().zip(smoothed) {
        assert!((a - b).abs() < 1e-10);
    }
}
#[test]
fn corrected_camera_precision_bound_uses_maximum_gain_without_resampling_calibration() {
    use crate::processing::Calibration;
    let nodes = vec![
        DVec3::new(0.8, 1.3, 0.9),
        DVec3::new(1.4, 0.9, 1.1),
        DVec3::ONE,
        DVec3::new(1.1, 1.2, 0.7),
    ];
    let field = FlatField::from_nodes([100, 200, 80, 40], [2, 2], nodes.clone()).unwrap();
    let base = 1. / 65535.;
    let expected = nodes
        .iter()
        .map(|g| g.max_element() / 65535.)
        .fold(base, f64::max);
    assert_eq!(field.precision_floor().unwrap(), expected);
    let calibration = Calibration {
        dmin: DVec3::splat(0.2),
        dmax: DVec3::splat(0.01),
        input_floor: base,
    };
    let corrected = field.processing_calibration(calibration).unwrap();
    assert_eq!(corrected.dmin, calibration.dmin);
    assert_eq!(corrected.dmax, calibration.dmax);
    assert_eq!(corrected.input_floor, expected);
    assert!(field.processing_calibration(corrected).is_err());
    let unusable = Calibration {
        dmax: DVec3::splat(base),
        ..calibration
    };
    assert!(field.processing_calibration(unusable).is_err());
    let identity = FlatField::from_nodes([100, 200, 80, 40], [2, 2], vec![DVec3::ONE; 4]).unwrap();
    assert_eq!(
        identity.processing_calibration(calibration).unwrap(),
        calibration
    );
    let attenuating =
        FlatField::from_nodes([100, 200, 80, 40], [2, 2], vec![DVec3::splat(0.8); 4]).unwrap();
    assert_eq!(attenuating.precision_floor().unwrap(), base);
}
#[test]
fn unusable_reference_inputs_do_not_produce_a_field() {
    let size = [7, 7];
    let center = DVec3::splat(0.1);
    let means = vec![center; 49];
    let generate = |data: &[DVec3], matrix: DMat3, c: DVec3, point: [f64; 2]| {
        FlatField::estimate([0, 0, 700, 700], size, data, matrix, c, point)
    };
    for bad in [0., -0.1, 1.1, f64::NAN] {
        let mut data = means.clone();
        data[10].x = bad;
        assert!(generate(&data, DMat3::IDENTITY, center, [350., 350.]).is_err());
    }
    assert!(generate(&means, DMat3::ZERO, center, [350., 350.]).is_err());
    assert!(generate(&means, DMat3::IDENTITY, center, [750., 350.]).is_err());
    assert!(generate(&means, DMat3::IDENTITY, DVec3::ZERO, [350., 350.]).is_err());
    assert!(generate(&means[..48], DMat3::IDENTITY, center, [350., 350.]).is_err());
}
