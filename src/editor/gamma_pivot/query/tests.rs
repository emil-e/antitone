use super::*;
use crate::{
    geometry::SourcePoint,
    processing::{Calibration, Response},
};

#[test]
fn evidence_plan_is_bounded_deterministic_and_covers_the_selected_shape() {
    let area = SourceRegion {
        x: 10,
        y: 20,
        width: 16000,
        height: 12000,
    };
    let small = Coverage::new(
        [[10.25, 20.5], [12.5, 20.5], [12.5, 22.25], [10.25, 22.25]].map(SourcePoint),
    )
    .unwrap();
    let exact = plan(small, area).unwrap();
    assert_eq!(exact, small.pixels(area).unwrap().collect::<Vec<_>>());
    assert_eq!(exact[0], ([10, 20], 0.375));
    assert!((exact.iter().map(|(_, w)| w).sum::<f64>() - small.area()).abs() < 1e-12);

    let rotated = Coverage::new(
        [
            [5000., 100.],
            [15000., 2100.],
            [13000., 10100.],
            [3000., 8100.],
        ]
        .map(SourcePoint),
    )
    .unwrap();
    let samples = plan(rotated, area).unwrap();
    assert_eq!(samples, plan(rotated, area).unwrap());
    assert!(samples.len() <= LIMIT as usize);
    assert!(
        samples.len() > 3000,
        "large selection must not collapse to one virtual pixel"
    );
    assert!(
        samples
            .iter()
            .all(|(point, weight)| *weight > 0. && rotated.weight(*point) > 0.)
    );
    assert!((samples.iter().map(|(_, w)| w).sum::<f64>() / rotated.area() - 1.).abs() < 1e-12);
    assert!(samples.iter().any(|(p, _)| p[0] < 6000));
    assert!(samples.iter().any(|(p, _)| p[0] > 12000));
    let mut corners = rotated.corners();
    corners.reverse();
    let reversed = Coverage::new(corners).unwrap();
    assert_eq!(samples.len(), plan(reversed, area).unwrap().len());
}

fn negative(gamma: f64) -> NegativeInput {
    NegativeInput {
        calibration: Calibration {
            dmin: DVec3::splat(0.8),
            dmax: DVec3::splat(0.2),
            input_floor: 1e-6,
        },
        response: Response {
            toe: 1.,
            shoulder: 0.,
        },
        gamma,
        balance: Balance::default(),
    }
}

#[test]
fn frozen_samples_are_individually_inverted_and_arbitrary_colour_is_preserved() {
    let file = crate::raw::fixture::bayer_dng_with_rgb([16000, 20000, 24000]);
    let editor = crate::editor::tests::authored_fixture(&[file.path().to_str().unwrap()]);
    let services = editor.source_services.as_ref().unwrap();
    let pixels = vec![[0.2, 0.4, 0.5, 0.], [0.6, 0.3, 0.25, 0.]];
    let evidence = Evidence {
        camera: evaluation::source(Arc::new(Pixels {
            pixels: pixels.clone(),
            meaning: Meaning::Camera,
        })),
        weights: vec![1., 3.],
    };
    // With gamma=1 and no toe/shoulder: P=(0.8/S-1)/3. This is an
    // analytic calibration case, not another implementation of the general response.
    let expected =
        (DVec3::new(1., 1. / 3., 0.2) + DVec3::new(1. / 9., 5. / 9., 11. / 15.) * 3.) / 4.;
    let mean = evidence.mean(services, negative(1.), &|| false).unwrap();
    assert!(
        (mean - expected).abs().max_element() < 2e-6,
        "{mean:?} != {expected:?}"
    );
    let averaged_input = (DVec3::new(0.2, 0.4, 0.5) + DVec3::new(0.6, 0.3, 0.25) * 3.) / 4.;
    let wrong = (DVec3::splat(0.8) / averaged_input - DVec3::ONE) / 3.;
    assert!((mean - wrong).abs().max_element() > 0.1);
    let wb = Balance {
        warmth: 0.12,
        tint: -0.08,
    };
    let compensated = evidence
        .compensate(
            services,
            negative(1.),
            negative(0.7),
            DMat3::IDENTITY,
            wb,
            &|| false,
        )
        .unwrap();
    assert_ne!(compensated, wb);
    let next = evidence.mean(services, negative(0.7), &|| false).unwrap();
    let held = crate::color::wb_matrix(wb).unwrap() * mean;
    let changed = crate::color::wb_matrix(compensated).unwrap() * next;
    assert!((chromaticity(held).unwrap() - chromaticity(changed).unwrap()).length() < 1e-12);
    assert!(
        (held.max_element() - changed.max_element()).abs() > 0.01,
        "brightness is not pinned"
    );
    assert_eq!(
        evidence
            .compensate(
                services,
                negative(1.),
                negative(1.),
                DMat3::IDENTITY,
                wb,
                &|| false
            )
            .unwrap(),
        wb
    );
    assert!(matches!(
        evidence.mean(services, negative(1.), &|| true),
        Err(Error::Cancelled)
    ));
    let limit = Balance {
        warmth: (compensated.warmth - wb.warmth).signum(),
        tint: (compensated.tint - wb.tint).signum(),
    };
    assert!(matches!(
        evidence.compensate(
            services,
            negative(1.),
            negative(0.7),
            DMat3::IDENTITY,
            limit,
            &|| false
        ),
        Err(Error::UnusableSample(
            "gamma pivot compensation exceeds the Roll WB range"
        ))
    ));

    let scan = crate::editor::measurement::capture_scan(&editor).unwrap();
    let captured =
        Evidence::capture(services, &scan, Coverage::rectangle(scan.area), &|| false).unwrap();
    let native = captured.mean(services, negative(1.), &|| false).unwrap();
    let supplied = DVec3::new(16000., 20000., 24000.) / 65535.;
    let expected = (DVec3::splat(0.8) / supplied - DVec3::ONE) / 3.;
    assert!(
        (native - expected).abs().max_element() < 2e-5,
        "capture must use source code units, not normalized-light codes: {native:?} != {expected:?}"
    );

    let extremes = Evidence {
        camera: evaluation::source(Arc::new(Pixels {
            pixels: vec![[0.001, 0.9, 0.4, 0.]],
            meaning: Meaning::Camera,
        })),
        weights: vec![1.],
    };
    let extreme_mean = extremes.mean(services, negative(1.), &|| false).unwrap();
    assert!(extreme_mean.x > 100. && extreme_mean.y < 0.);

    let floored = Evidence {
        camera: evaluation::source(Arc::new(Pixels {
            pixels: vec![[0., 0.2, 0.3, 0.]],
            meaning: Meaning::Camera,
        })),
        weights: vec![1.],
    };
    assert!(matches!(
        floored.mean(services, negative(1.), &|| false),
        Err(Error::UnusableSample(
            "gamma pivot reaches the source precision floor"
        ))
    ));
}

#[test]
fn actual_nonuniform_source_coverage_matches_individually_transformed_originals() {
    let editor = crate::editor::tests::authored_fixture(&["patches.dng"]);
    let scan = crate::editor::measurement::capture_scan(&editor).unwrap();
    let services = editor.source_services.as_ref().unwrap();
    let coverage = Coverage::new(
        [
            [190.25, 70.75],
            [227.75, 79.25],
            [219.5, 110.5],
            [181.8, 99.3],
        ]
        .map(SourcePoint),
    )
    .unwrap();
    let evidence = Evidence::capture(services, &scan, coverage, &|| false).unwrap();
    let actual = evidence.mean(services, negative(1.), &|| false).unwrap();
    let mut sum = DVec3::ZERO;
    let mut camera_sum = DVec3::ZERO;
    let mut weight = 0.;
    for (point, area) in coverage.pixels(scan.area).unwrap() {
        let camera = scan.pixel(point[0], point[1]).unwrap();
        // The gamma-one calibrated response is the rational function used in the
        // analytic case above; source reads include the actual demosaiced ramps.
        sum += ((DVec3::splat(0.8) / camera - DVec3::ONE) / 3.) * area;
        camera_sum += camera * area;
        weight += area;
    }
    let expected = sum / weight;
    assert!(
        (actual - expected).abs().max_element() < 2e-5,
        "{actual:?} != {expected:?}"
    );
    let wrong = (DVec3::splat(0.8) / (camera_sum / weight) - DVec3::ONE) / 3.;
    assert!(
        (actual - wrong).abs().max_element() > 0.001,
        "fixture must distinguish averaging before inversion"
    );
}

#[test]
fn undefined_colour_rejects_without_clipping_signed_or_hdr_working_values() {
    assert!(chromaticity(DVec3::ZERO).is_err());
    assert!(chromaticity(DVec3::splat(-1.)).is_err());
    assert!(chromaticity(DVec3::splat(f64::NAN)).is_err());
    assert!(chromaticity(DVec3::new(-0.01, 2., 3.)).is_ok());
    assert!(
        (chromaticity(DVec3::new(-0.01, 2., 3.)).unwrap()
            - chromaticity(DVec3::new(-0.01, 2., 3.) * 8.).unwrap())
        .length()
            < 1e-14
    );
}
