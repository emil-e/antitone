use super::*;
use crate::{
    geometry::{Composition, Flips, PhotoOrientation, QuarterTurns, RollOrientation},
    scan_regions::PictureGeometry,
};

const ACTIVE: SourceRegion = SourceRegion {
    x: 12,
    y: 7,
    width: 8639,
    height: 5757,
};

fn evidence(angle: f64) -> Areas {
    Areas {
        picture_geometry: Some(PictureGeometry {
            size: [768, 512],
            angle,
            sides: [Some(80.3), Some(682.7), Some(80.2), Some(430.8)],
        }),
        picture_observed: [true; 4],
        ..Default::default()
    }
}

fn inside(p: [f64; 2], q: Quad) -> bool {
    (0..4).all(|i| {
        let a = q[i];
        let b = q[(i + 1) % 4];
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -1e-6
    })
}

#[test]
fn margin_is_equal_distance_per_edge_not_uniform_aspect_scaling() {
    let base = Crop {
        center: [2500., 2000.],
        size: [1800, 1200],
        angle: 0.,
    };
    let crop = inward_margin(base, ACTIVE, 1.).unwrap();
    assert_eq!(crop.size, [1776, 1176]);
    assert_eq!(crop.center, base.center);
    assert_eq!(crop.angle, base.angle);
    assert_eq!(inward_margin(base, ACTIVE, 0.).unwrap(), base);
}

#[test]
fn margins_are_inward_monotonic_source_anchored_and_restore_without_accumulation() {
    for angle in [-0.3, 0., 0.27] {
        let base = Crop {
            center: [2500.5, 2000.5],
            size: [1801, 1201],
            angle,
        };
        let mut previous = base.size;
        for percent in [
            0.,
            0.01,
            0.25,
            DEFAULT_MARGIN_PERCENT,
            1.,
            MAX_MARGIN_PERCENT,
        ] {
            let crop = inward_margin(base, ACTIVE, percent).unwrap();
            assert_eq!(crop.center, base.center);
            assert_eq!(crop.angle, base.angle);
            assert!(crop.size.iter().zip(previous).all(|(&n, old)| n <= old));
            let q = base.coverage().unwrap().corners().map(|p| p.0);
            assert!(
                crop.coverage()
                    .unwrap()
                    .corners()
                    .iter()
                    .all(|p| inside(p.0, q))
            );
            previous = crop.size;
        }
        let first = inward_margin(base, ACTIVE, 0.5).unwrap();
        assert_ne!(inward_margin(base, ACTIVE, 2.).unwrap(), first);
        assert_eq!(inward_margin(base, ACTIVE, 0.5).unwrap(), first);
        assert_eq!(inward_margin(base, ACTIVE, 0.).unwrap(), base);
    }
}

#[test]
fn margin_preserves_integer_edges_and_never_empties_tiny_frames_or_accepts_invalid_values() {
    for size in [[1, 1], [2, 2], [3, 2], [3, 3], [1, 1000]] {
        let base = Crop::from_region(SourceRegion {
            x: 2000,
            y: 2000,
            width: size[0],
            height: size[1],
        });
        for angle in [0., 0.4] {
            let base = Crop { angle, ..base };
            let crop = inward_margin(base, ACTIVE, MAX_MARGIN_PERCENT).unwrap();
            assert!(!crop.size.contains(&0));
            assert_eq!(crop.center, base.center);
            crop.region(ACTIVE).unwrap();
            if angle == 0. {
                assert!(
                    crop.coverage()
                        .unwrap()
                        .corners()
                        .iter()
                        .flat_map(|p| p.0)
                        .all(|v| v.fract() == 0.)
                );
            }
            for percent in [-0.01, MAX_MARGIN_PERCENT + 0.01, f64::NAN, f64::INFINITY] {
                assert!(inward_margin(base, ACTIVE, percent).is_err());
            }
        }
    }
}

#[test]
fn mapped_frame_retains_angle_origin_and_content_through_every_turn_and_reflection() {
    for angle in [-5_f64, -0.75, 0., 0.125, 4.] {
        let areas = evidence(angle);
        let crop = propose(ACTIVE, &areas).unwrap();
        let quad = areas
            .picture_geometry
            .as_ref()
            .unwrap()
            .frame(ACTIVE)
            .unwrap();
        assert!(
            crop.coverage()
                .unwrap()
                .corners()
                .iter()
                .all(|p| inside(p.0, quad))
        );
        let expected_angle = (angle.to_radians().sin() * f64::from(ACTIVE.height) / 512.)
            .atan2(angle.to_radians().cos() * f64::from(ACTIVE.width) / 768.);
        assert!((crop.angle - expected_angle).abs() < 1e-12);
        let expected_area = Areas::coverage(quad).unwrap().area();
        assert!(1. - crop.coverage().unwrap().area() / expected_area < 0.001);
        for rt in [
            QuarterTurns::Zero,
            QuarterTurns::One,
            QuarterTurns::Two,
            QuarterTurns::Three,
        ] {
            for pt in [
                QuarterTurns::Zero,
                QuarterTurns::One,
                QuarterTurns::Two,
                QuarterTurns::Three,
            ] {
                for horizontal in [false, true] {
                    for vertical in [false, true] {
                        let roll = RollOrientation {
                            turns: rt,
                            horizontal,
                            vertical,
                        };
                        let photo = PhotoOrientation {
                            turns: pt,
                            flips: Flips {
                                horizontal: vertical,
                                vertical: horizontal,
                            },
                        };
                        let composition =
                            Composition::with_crop(ACTIVE, roll, photo, Some(crop)).unwrap();
                        for p in crop.coverage().unwrap().corners() {
                            let back = composition.frame_to_source(composition.source_to_frame(p));
                            assert!(
                                (back.0[0] - p.0[0]).abs() < 1e-8
                                    && (back.0[1] - p.0[1]).abs() < 1e-8
                            );
                        }
                        assert_eq!(composition.crop(), crop);
                    }
                }
            }
        }
    }
}

#[test]
fn sampling_opening_picture_and_authored_opening_do_not_change_framing() {
    let mut areas = evidence(0.75);
    let crop = propose(ACTIVE, &areas).unwrap();
    let inner = SourceRegion {
        x: 2000,
        y: 1500,
        width: 3000,
        height: 2000,
    };
    areas.edit_opening(crate::coverage::Coverage::rectangle(inner));
    areas.picture = areas.opening;
    assert_eq!(propose(ACTIVE, &areas).unwrap(), crop);
    areas.edit_picture(crate::coverage::Coverage::rectangle(inner));
    assert!(propose(ACTIVE, &areas).is_err());
}

#[test]
fn abstains_for_weak_missing_invalid_off_source_and_nonrectangular_evidence() {
    let baseline = evidence(0.25);
    for side in 0..4 {
        let mut a = baseline.clone();
        a.picture_observed[side] = false;
        a.picture_weak[side] = true;
        assert!(propose(ACTIVE, &a).is_err());
        a.picture_weak[side] = false;
        a.picture_geometry.as_mut().unwrap().sides[side] = None;
        assert!(propose(ACTIVE, &a).is_err());
    }
    let mut a = baseline.clone();
    a.picture_geometry = None;
    assert!(propose(ACTIVE, &a).is_err());
    for value in [f64::NAN, f64::INFINITY, -3., 700.] {
        let mut a = evidence(0.);
        a.picture_geometry.as_mut().unwrap().sides[0] = Some(value);
        assert!(propose(ACTIVE, &a).is_err());
    }
    let mut a = baseline.clone();
    a.picture_geometry.as_mut().unwrap().angle = 7.;
    assert!(propose(ACTIVE, &a).is_err());
    let mut a = baseline;
    a.picture_constraints = vec![[1., 0., 2000.]];
    assert!(propose(ACTIVE, &a).is_err());
    assert!(propose(ACTIVE, &Areas::default()).is_err());
}

#[test]
fn edge_fit_residual_is_measured_in_capture_pixels_not_source_pixels() {
    for active in [
        ACTIVE,
        SourceRegion {
            x: 31,
            y: 19,
            width: 1536,
            height: 1024,
        },
    ] {
        let mut areas = evidence(0.);
        let left = areas
            .picture_geometry
            .as_ref()
            .unwrap()
            .frame(active)
            .unwrap()[0][0];
        let scale = f64::from(active.width) / 768.;
        areas.picture_constraints = vec![[1., 0., left + (MAX_EDGE_RESIDUAL - 0.01) * scale]];
        assert!(propose(active, &areas).is_ok());
        areas.picture_constraints[0][2] = left + (MAX_EDGE_RESIDUAL + 0.01) * scale;
        assert!(propose(active, &areas).is_err());
    }
}

#[test]
fn zero_angle_uses_inward_integer_edges_without_constraining_or_moving_measured_frame() {
    let crop = propose(ACTIVE, &evidence(0.)).unwrap();
    let region = crop.region(ACTIVE).unwrap();
    assert_eq!(region.x, (12. + 80.3 * 8639. / 768_f64).ceil() as u32);
    assert_eq!(region.y, (7. + 80.2 * 5757. / 512_f64).ceil() as u32);
    assert_eq!(
        region.x + region.width,
        (12. + 682.7 * 8639. / 768_f64).floor() as u32
    );
    let mut a = evidence(0.);
    a.picture_geometry.as_mut().unwrap().sides = [Some(0.), Some(768.), Some(0.), Some(512.)];
    assert_eq!(propose(ACTIVE, &a).unwrap(), Crop::from_region(ACTIVE));
    a.picture_geometry.as_mut().unwrap().angle = 0.125;
    assert!(
        propose(ACTIVE, &a).is_err(),
        "do not silently shrink an off-source frame"
    );
}

#[test]
fn nonuniform_raster_mapping_is_inscribed_not_assumed_orthogonal() {
    let active = SourceRegion {
        x: 23,
        y: 41,
        width: 1000,
        height: 700,
    };
    let areas = evidence(3.);
    let crop = propose(active, &areas).unwrap();
    let quad = areas
        .picture_geometry
        .as_ref()
        .unwrap()
        .frame(active)
        .unwrap();
    assert!(
        crop.coverage()
            .unwrap()
            .corners()
            .iter()
            .all(|p| inside(p.0, quad))
    );
    assert!((crop.angle.to_degrees() - 3.).abs() > 0.1);
}
