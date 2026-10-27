use super::*;

fn donors(ratio: f64) -> Vec<Donor> {
    (0..5).map(|source| Donor { source, ratio }).collect()
}
fn geometry() -> PictureGeometry {
    PictureGeometry {
        size: [100, 100],
        angle: 0.,
        sides: [Some(10.), Some(90.), Some(20.), Some(80.)],
    }
}

#[test]
fn continuous_ordered_shapes_cover_formats_without_a_whitelist_or_reciprocal_folding() {
    // Example rectangles exercise the range, not literal camera aperture specifications.
    for (width, height) in [
        (36., 24.),
        (18., 24.),
        (65., 24.),
        (56., 41.5),
        (56., 56.),
        (69., 56.),
        (76., 56.),
        (84., 56.),
        (112., 56.),
        (168., 56.),
        (137., 43.),
    ] {
        let expected = width / height;
        let shape = fit(&donors(expected), 99).unwrap();
        assert!((shape.ratio - expected).abs() < 1e-12);
        if width < height {
            assert!(shape.ratio < 1.);
        }
    }
}

#[test]
fn own_target_never_votes_and_source_order_does_not_change_shape() {
    let mut input = donors(1.5);
    input.push(Donor {
        source: 99,
        ratio: 3.,
    });
    let shape = fit(&input, 99).unwrap();
    assert!((shape.ratio - 1.5).abs() < 1e-12);
    assert!(!shape.donors.contains(&99));
    input.reverse();
    assert_eq!(fit(&input, 99).unwrap().ratio, shape.ratio);
    let only_self = vec![Donor {
        source: 99,
        ratio: 1.5,
    }];
    assert!(fit(&only_self, 99).is_err());
}

#[test]
fn isolated_bad_donors_are_rejected_but_conflicting_roll_measurements_do_not_create_shapes() {
    let mut input = donors(1.5);
    input.push(Donor {
        source: 8,
        ratio: 2.7,
    });
    let shape = fit(&input, 99).unwrap();
    assert_eq!(shape.rejected, vec![8]);
    let input: Vec<_> = (0..6)
        .map(|source| Donor {
            source,
            ratio: if source < 3 { 1.5 } else { 0.75 },
        })
        .collect();
    assert!(fit(&input, 99).is_err());
    let mut duplicate = donors(1.5);
    duplicate.push(duplicate[0]);
    assert!(fit(&duplicate, 99).is_err());
}

#[test]
fn source_scaling_and_nonzero_origins_do_not_turn_raster_shape_into_roll_shape() {
    let active = [12, 23, 2000, 1000].into();
    assert!((ratio(&geometry(), active).unwrap() - 8. / 3.).abs() < 1e-12);
    let shape = fit(&donors(8. / 3.), 99).unwrap();
    for side in 0..4 {
        let mut hidden = geometry();
        let expected = hidden.sides[side].unwrap();
        hidden.sides[side] = None;
        let draft = complete(&hidden, active, &shape).unwrap();
        assert!((draft.geometry.sides[side].unwrap() - expected).abs() < 1e-10);
        assert_eq!(
            hidden.sides[side], None,
            "target measurements remain absent"
        );
        assert!(!draft.measured[side]);
        assert!(draft.inferred[side]);
        assert_eq!(draft.inferred.iter().filter(|&&v| v).count(), 1);
    }
}

#[test]
fn angle_and_ordered_ratio_are_preserved_without_changing_observed_anchors() {
    let active = [19, 31, 2000, 1000].into();
    let mut full = geometry();
    full.angle = 3.;
    let r = ratio(&full, active).unwrap();
    let shape = fit(&donors(r), 99).unwrap();
    for side in 0..4 {
        let mut hidden = full.clone();
        hidden.sides[side] = None;
        let draft = complete(&hidden, active, &shape).unwrap();
        assert_eq!(draft.geometry.angle, 3.);
        for other in 0..4 {
            if other != side {
                assert_eq!(draft.geometry.sides[other], full.sides[other]);
            }
        }
        assert!((draft.geometry.sides[side].unwrap() - full.sides[side].unwrap()).abs() < 1e-10);
    }
}

#[test]
fn draft_adapter_reuses_crop_geometry_but_does_not_clamp_an_offsource_prediction() {
    let active: SourceRegion = [12, 23, 2000, 1000].into();
    let mut hidden = geometry();
    hidden.sides[0] = None;
    let draft = complete(&hidden, active, &fit(&donors(8. / 3.), 99).unwrap()).unwrap();
    let crop =
        crate::auto_crop::inferred_draft(active, draft.geometry.frame(active).unwrap()).unwrap();
    assert_eq!(crop.size, [1600, 600]);
    assert_eq!(crop.center, [1012., 523.]);
    let outside = complete(&hidden, active, &fit(&donors(4.), 99).unwrap()).unwrap();
    assert!(
        crate::auto_crop::inferred_draft(active, outside.geometry.frame(active).unwrap()).is_err()
    );
    assert!(hidden.sides[0].is_none());
}

#[test]
fn missing_position_anchors_invalid_inputs_and_reversed_extents_abstain() {
    let shape = fit(&donors(1.5), 99).unwrap();
    let active = [0, 0, 1000, 1000].into();
    assert!(complete(&geometry(), active, &shape).is_err());
    let mut missing = geometry();
    missing.sides[0] = None;
    missing.sides[1] = None;
    assert!(complete(&missing, active, &shape).is_err());
    missing = geometry();
    missing.sides[0] = None;
    missing.sides[3] = Some(10.);
    assert!(complete(&missing, active, &shape).is_err());
    let invalid = [Donor {
        source: 1,
        ratio: f64::NAN,
    }];
    assert!(fit(&invalid, 99).is_err());
}
