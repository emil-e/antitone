use super::super::tests::{fixture, shape_reference as reference};
use super::tests::{choice, fine_picture, previous};
use super::*;

#[test]
fn newly_inferred_extent_cannot_cross_known_film_cut_even_if_local_anchors_improve() {
    let (capture, _, scene) = fixture([64, 48], [8., 55., 6., 42.], Some(55.), false);
    let original = choice(
        PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: [Some(8.5), Some(55.166666666666664), Some(6.5), Some(41.5)],
        },
        [false, true, false, false],
        capture.area,
    );
    let fine = fine_picture([8., 56., 6., 42.]);
    let result = run(
        &capture,
        &scene.roles(&capture),
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference(Some(4. / 3.)),
        &previous(),
        &Areas::default(),
    )
    .unwrap();
    assert_eq!(result.status, "retained_after_geometry_check");
    assert!(!result.applied);
    assert_eq!(result.crop, original.crop);
}

#[test]
fn a_camera_clipped_inferred_side_is_not_queried_as_a_measured_anchor() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., -5., 42.], None, false);
    let original = choice(
        PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: [Some(8.5), Some(55.5), Some(-4.520833333333336), Some(41.5)],
        },
        [false, false, true, false],
        capture.area,
    );
    let fine = fine_picture([8., 56., -5., 42.]);
    let result = run(
        &capture,
        &scene.roles(&capture),
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference(Some(48. / 47.)),
        &previous(),
        &Areas::default(),
    )
    .unwrap();
    assert!(result.applied);
    assert!(!result.edges.iter().any(|row| row.side == 2));
    assert!((result.physical.sides[2].unwrap() + 5.).abs() < 1e-9);
    result.crop.region(capture.area).unwrap();
}

#[test]
fn one_ambiguous_anchor_keeps_total_missing_side_uncertainty_unresolved() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    let original = choice(
        PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: [Some(8.5), Some(55.166666666666664), Some(6.5), Some(41.5)],
        },
        [false, true, false, false],
        capture.area,
    );
    let mut fine = fine_picture([8., 56., 6., 42.]);
    fine[..40 * 256].fill([6000; 3]);
    let result = run(
        &capture,
        &scene.roles(&capture),
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference(Some(4. / 3.)),
        &previous(),
        &Areas::default(),
    )
    .unwrap();
    assert!(result.applied);
    assert!(result.uncertainty_unresolved);
    assert!(result.inferred_envelope_raster.is_none());
    assert_eq!(result.physical.sides[2], original.physical.sides[2]);
}

#[test]
fn established_four_edges_skip_all_fine_queries_even_when_another_step_is_available() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    let original = choice(
        PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: [Some(8.5), Some(55.5), Some(6.5), Some(41.5)],
        },
        [false; 4],
        capture.area,
    );
    let baseline = Areas {
        picture_observed: [true; 4],
        ..Areas::default()
    };
    assert!(!needed(&original, &baseline));
    let fine = fine_picture([8., 56., 6., 42.]);
    let result = run(
        &capture,
        &scene.roles(&capture),
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference(Some(4. / 3.)),
        &previous(),
        &baseline,
    )
    .unwrap();
    assert!(!result.applied);
    assert_eq!(result.crop, original.crop);
    assert!(result.edges.iter().all(|edge| edge.rgb_queries == 0));
}

#[test]
fn mixed_evidence_refines_only_weakly_acquired_anchors_and_preserves_the_strong_one() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    let original = choice(
        PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: [Some(8.5), Some(55.166666666666664), Some(6.5), Some(41.5)],
        },
        [false, true, false, false],
        capture.area,
    );
    let baseline = Areas {
        picture_observed: [true, false, false, false],
        ..Areas::default()
    };
    assert!(needed(&original, &baseline));
    let fine = fine_picture([8., 56., 6., 42.]);
    let result = run(
        &capture,
        &scene.roles(&capture),
        Fine {
            size: [256, 192],
            rgb: &fine,
        },
        &original,
        &reference(Some(4. / 3.)),
        &previous(),
        &baseline,
    )
    .unwrap();
    assert!(result.applied);
    assert_eq!(result.physical.sides[0], original.physical.sides[0]);
    assert_eq!(result.edges[0].status, "retained_strong_anchor");
    assert_eq!(result.edges[0].rgb_queries, 0);
    assert!(
        result
            .edges
            .iter()
            .map(|edge| edge.rgb_queries)
            .sum::<usize>()
            <= 2 * STATIONS * SAMPLES * 3
    );
}

#[test]
fn camera_clipped_probe_window_does_not_relocate_an_observed_edge_to_the_source_boundary() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., 1., 42.], None, false);
    let original = choice(
        PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: [Some(8.5), Some(55.5), Some(1.5), Some(41.5)],
        },
        [false; 4],
        capture.area,
    );
    let fine = fine_picture([8., 56., 1., 42.]);
    let result = run(
        &capture,
        &scene.roles(&capture),
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
    assert_eq!(result.physical.sides[2], original.physical.sides[2]);
    assert!(
        result
            .edges
            .iter()
            .find(|edge| edge.side == 2)
            .unwrap()
            .offset_fine
            .is_none()
    );
}
