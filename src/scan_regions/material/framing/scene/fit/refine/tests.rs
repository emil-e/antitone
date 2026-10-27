use super::super::tests::{fixture, shape_reference as reference};
use super::*;

fn profiles(edge: f64, blur: f64) -> Vec<[f64; SAMPLES]> {
    (0..STATIONS)
        .map(|_| {
            std::array::from_fn(|i| {
                let x = i as f64 - f64::from(RADIUS + BAND) + 0.5;
                14. - 2. / (1. + (-(x - edge) / blur).exp())
            })
        })
        .collect()
}

#[test]
fn bounded_step_centres_remain_integer_samples_even_when_analog_edges_are_blurred() {
    for offset in [-8, -2, 0, 5, 9] {
        for blur in [0.1, 1., 2.] {
            for width in [2, 4] {
                assert!(
                    (locate(&profiles(f64::from(offset), blur), width).unwrap() - offset).abs()
                        <= 1
                );
            }
        }
    }
}

#[test]
fn constant_ramp_reverse_step_and_search_cap_do_not_move_an_anchor() {
    assert!(locate(&vec![[12.; SAMPLES]; STATIONS], 2).is_none());
    let ramp = vec![std::array::from_fn(|i| 12. + i as f64 * 0.01); STATIONS];
    assert!(locate(&ramp, 2).is_none());
    let reverse: Vec<_> = profiles(0., 1.)
        .into_iter()
        .map(|row| row.map(|v| -v))
        .collect();
    assert!(locate(&reverse, 2).is_none());
    assert!(locate(&profiles(15., 0.1), 2).is_none());
}

#[test]
fn incoherent_fine_noise_does_not_turn_coarse_association_into_a_position_change() {
    let mut state = 17u32;
    let noise: Vec<_> = (0..STATIONS)
        .map(|_| {
            std::array::from_fn(|_| {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                12. + (f64::from(state) / f64::from(u32::MAX) - 0.5) * 0.2
            })
        })
        .collect();
    assert!(locate(&noise, 2).is_none() || locate(&noise, 4).is_none());
}

pub(super) fn choice(frame: PictureGeometry, inferred: [bool; 4], active: SourceRegion) -> Choice {
    let support = std::array::from_fn(|_| Support {
        pairs: (0..STATIONS)
            .map(|i| Pair {
                along: 0.25 + 0.5 * (i as f64 + 0.5) / STATIONS as f64,
                density: [1.; 3],
                base_outside: true,
            })
            .collect(),
        ..Support::default()
    });
    Choice {
        method: "authored_associated_frame",
        crop: visible_crop(active, &frame).unwrap(),
        physical: frame,
        measured: inferred.map(|v| !v),
        inferred,
        support,
        pooled: std::array::from_fn(|_| None),
        inferred_band_raster: None,
        source_clipped: false,
        outside_base_score: 0.,
        signal_score: 0.,
    }
}

pub(super) fn previous() -> Constraints {
    Constraints::default()
}

pub(super) fn fine_picture(bounds: [f64; 4]) -> Vec<[u16; 3]> {
    (0..192)
        .flat_map(|y| {
            (0..256).map(move |x| {
                if x as f64 + 0.5 >= bounds[0] * 4.
                    && (x as f64 + 0.5) < bounds[1] * 4.
                    && y as f64 + 0.5 >= bounds[2] * 4.
                    && (y as f64 + 0.5) < bounds[3] * 4.
                {
                    [6000; 3]
                } else {
                    [20000; 3]
                }
            })
        })
        .collect()
}

#[test]
fn measured_anchors_refine_then_only_the_missing_side_is_recompleted_without_ratio_feedback() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    let reference = reference(Some(4. / 3.));
    let frame = PictureGeometry {
        size: capture.size,
        angle: 0.,
        sides: [Some(8.5), Some(55.166666666666664), Some(6.5), Some(41.5)],
    };
    let original = choice(frame, [false, true, false, false], capture.area);
    let fine = fine_picture([8., 56., 6., 42.]);
    let result = run(
        &capture,
        &scene.roles(&capture),
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference,
        &previous(),
        &Areas::default(),
    )
    .unwrap();
    assert!(result.applied);
    for (side, expected) in [8., 56., 6., 42.].into_iter().enumerate() {
        assert!((result.physical.sides[side].unwrap() - expected).abs() < 1e-9);
    }
    assert!(!result.edges.iter().any(|row| row.side == 1));
    assert_eq!(reference.ratio, Some(4. / 3.));
    assert_eq!(reference.donors, vec![100, 101, 102]);
    assert!(
        result
            .edges
            .iter()
            .map(|row| row.rgb_queries)
            .sum::<usize>()
            <= 3 * STATIONS * SAMPLES * 3
    );
}

#[test]
fn four_measurements_are_never_forced_to_match_the_roll_ratio() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    let frame = PictureGeometry {
        size: capture.size,
        angle: 0.,
        sides: [Some(8.5), Some(55.5), Some(6.5), Some(41.5)],
    };
    let original = choice(frame, [false; 4], capture.area);
    let fine = fine_picture([8., 56., 6., 42.]);
    let result = run(
        &capture,
        &scene.roles(&capture),
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference(Some(2.7)),
        &previous(),
        &Areas::default(),
    )
    .unwrap();
    assert!(result.applied);
    assert_eq!(
        result.physical.sides,
        [Some(8.), Some(56.), Some(6.), Some(42.)]
    );
}

#[test]
fn unknown_material_preserves_coarse_geometry() {
    let (capture, _, _) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    let original = choice(
        PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: [Some(8.5), Some(55.5), Some(6.5), Some(41.5)],
        },
        [false; 4],
        capture.area,
    );
    let fine = fine_picture([8., 56., 6., 42.]);
    let result = run(
        &capture,
        &vec![Role::Unknown; capture.rgb.len()],
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference(None),
        &previous(),
        &Areas::default(),
    )
    .unwrap();
    assert!(!result.applied);
    assert_eq!(result.crop, original.crop);
    assert!(result.edges.iter().all(|edge| edge.offset_fine.is_none()));
}

#[test]
fn anisotropic_scaling_moves_along_the_picture_normal_not_a_folded_or_nominal_ratio() {
    let frame = PictureGeometry {
        size: [64, 48],
        angle: 3.,
        sides: [Some(8.), Some(56.), Some(6.), Some(42.)],
    };
    let (normal, scale) = direction(&frame, 0, [256, 144]);
    assert!((normal[0].hypot(normal[1]) - 1.).abs() < 1e-12);
    assert!((scale - (normal[0] * 4.).hypot(normal[1] * 3.)).abs() < 1e-12);
}
