use super::*;
mod normalization;

/// Search fixtures can supply a previous accepted donor; only the final result
/// is passed to the deterministic healer.
fn suggest_offset(
    repair: &Repair,
    active: SourceRegion,
    base: &mut impl FnMut([u32; 2]) -> Result<Ap1, Error>,
    deadline: Instant,
    threshold: f64,
    execution: &mut Execution<'_>,
) -> Result<Option<[i32; 2]>, Error> {
    Ok(
        match search_donor(
            &repair.support,
            Some(repair.offset),
            active,
            base,
            deadline,
            threshold,
            execution,
        )? {
            SearchOutcome::Candidate(offset) => Some(offset),
            SearchOutcome::Cancelled => None,
            SearchOutcome::Impossible => panic!("fixture has a valid previous donor"),
        },
    )
}

#[test]
fn linear_distance_transform_preserves_every_reference_distance_bit() {
    assert!(exact_distance_squares((1 << 26) + 1, (1 << 26) + 1));
    assert!(!exact_distance_squares((1 << 26) + 2, (1 << 26) + 1));
    assert!(exact_distance_squares(94_906_266, 1));
    assert!(!exact_distance_squares(94_906_267, 1));
    assert!(!exact_distance_squares(usize::MAX, usize::MAX));
    let distant = [true, false];
    assert_eq!(
        distance_to_unknown(&distant, 94_906_267, 1, &|| false),
        distance_to_unknown_scalar(&distant, 94_906_267, &|| false)
    );
    let check = |known: &[bool], width, height| {
        let exact = distance_to_unknown(known, width, height, &|| false).unwrap();
        let reference = distance_to_unknown_scalar(known, width, &|| false).unwrap();
        assert_eq!(exact, reference, "{width}x{height}");
    };
    for (width, height) in [(1usize, 2048usize), (2048, 1), (257, 7), (63, 41), (64, 64)] {
        for mode in 0..5 {
            let known = (0..width * height)
                .map(|i| match mode {
                    0 => true,
                    1 => false,
                    2 => i != width * height / 2,
                    3 => (i % width + i / width) % 3 != 0,
                    _ => (i.wrapping_mul(2654435761usize).rotate_left(7) & 15) != 0,
                })
                .collect::<Vec<_>>();
            check(&known, width, height);
        }
    }
}

fn circle(size: [u32; 2], center: [i32; 2], radius: i32) -> Vec<bool> {
    let mut mask = vec![false; (size[0] * size[1]) as usize];
    for y in 0..size[1] as i32 {
        for x in 0..size[0] as i32 {
            mask[(y as u32 * size[0] + x as u32) as usize] =
                (x - center[0]).pow(2) + (y - center[1]).pow(2) <= radius.pow(2);
        }
    }
    mask
}

fn gradient(size: [u32; 2]) -> Vec<Ap1> {
    (0..size[1])
        .flat_map(|y| {
            (0..size[0]).map(move |x| {
                [
                    0.04 + x as f32 * 0.0001,
                    -0.02 + y as f32 * 0.0002,
                    1.1 + x as f32 * 0.0003,
                ]
            })
        })
        .collect()
}

fn outside_bits_equal(a: &[Ap1], b: &[Ap1], mask: &[bool]) -> bool {
    a.iter()
        .zip(b)
        .zip(mask)
        .all(|((a, b), hidden)| *hidden || a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()))
}

fn active(size: [u32; 2]) -> SourceRegion {
    SourceRegion {
        x: 0,
        y: 0,
        width: size[0],
        height: size[1],
    }
}

#[test]
fn repair_set_keeps_stable_independent_repairs() {
    let mut set = RepairSet::default();
    let a = SupportMask::new([10, 10], [3, 3], vec![true; 9]).unwrap();
    let b = SupportMask::new([20, 20], [2, 2], vec![true; 4]).unwrap();
    let first = set.insert(a.clone(), RepairOrigin::Manual, [7, 0]).unwrap();
    let second = set
        .insert(b.clone(), RepairOrigin::Detected, [-3, 2])
        .unwrap();
    assert_eq!(set.repairs()[1].id, second);
    let unchanged = set.repairs()[1].clone();
    set.set_enabled(first, false).unwrap();
    set.set_offset(first, [4, -2]).unwrap();
    assert_eq!(set.repairs()[1], unchanged);
    assert!(
        set.set_support(
            first,
            SupportMask::new([19, 19], [3, 3], vec![true; 9]).unwrap()
        )
        .is_err()
    );
    set.set_support(first, a).unwrap();
    assert_eq!(set.remove(first).unwrap().id, first);
    assert_eq!(set.repairs(), &[unchanged]);
    assert!(set.remove(first).is_err());
}

#[test]
fn nearest_offset_is_valid_and_shared_by_disconnected_components() {
    let size = [400, 400];
    let active = active(size);
    let bits = circle(size, [40, 40], 4)
        .into_iter()
        .zip(circle(size, [140, 140], 3))
        .map(|(a, b)| a || b)
        .collect::<Vec<_>>();
    let support = SupportMask::new([0, 0], size, bits).unwrap();
    let offset = nearest_offset(&support, active).expect("a donor margin exists");
    assert!(validate_offset(offset, &support, active).is_ok());
    // One translation serves both components.
    let target = donor_target(&support, active).unwrap();
    let source = offset_target(&support, active, offset).unwrap();
    assert_eq!(source.width, target.width);
    assert_eq!(source.height, target.height);
    assert_ne!(offset, [0, 0]);
}

#[test]
fn offset_validation_rejects_outside_and_support_contact() {
    let size = [96, 96];
    let active = active(size);
    let mask = circle(size, [48, 48], 5);
    let support = SupportMask::new([0, 0], size, mask).unwrap();
    assert!(validate_offset([0, 0], &support, active).is_err());
    assert!(validate_offset([10_000, 0], &support, active).is_err());
    let offset = nearest_offset(&support, active).unwrap();
    assert_ne!(offset, [0, 0]);
    assert!(validate_offset(offset, &support, active).is_ok());
}

#[test]
fn masked_values_never_affect_fixed_donor_and_known_bits_stay_exact() {
    let size = [96, 96];
    let mut a = gradient(size);
    a[0] = [-0.0, 0.0, -0.0];
    let mut b = a.clone();
    let mask = circle(size, [48, 48], 6);
    for (index, hidden) in mask.iter().enumerate() {
        if *hidden {
            a[index] = [-100.; 3];
            b[index] = [100.; 3];
        }
    }
    let offset = [-25, 0];
    let (left, left_match) = heal_ranked(&a, size, &mask, offset).unwrap();
    let (right, right_match) = heal_ranked(&b, size, &mask, offset).unwrap();
    assert_eq!(
        left, right,
        "hidden source values must not leak into reconstructed pixels"
    );
    assert_eq!(left_match, right_match);
    assert!(outside_bits_equal(&left, &a, &mask));
    assert!(outside_bits_equal(&right, &b, &mask));
}

#[test]
fn stored_donor_reconstructs_an_analytic_gradient_through_nearby_masks() {
    let size = [96, 96];
    let image = gradient(size);
    let mut mask = vec![false; image.len()];
    for y in 46..49 {
        for x in 46..49 {
            mask[y * 96 + x] = true;
            mask[y * 96 + x + 4] = true;
        }
    }
    let (result, donors) = heal_ranked(&image, size, &mask, [-25, 0]).unwrap();
    assert_eq!(donors.len(), 1);
    assert!(outside_bits_equal(&result, &image, &mask));
    for (actual, expected) in result
        .iter()
        .zip(&image)
        .zip(&mask)
        .filter_map(|((actual, expected), hidden)| hidden.then_some((actual, expected)))
    {
        for channel in 0..3 {
            assert!((actual[channel] - expected[channel]).abs() < 4e-7);
        }
    }
}

#[test]
fn search_then_stored_donor_preserves_a_crossing_wire() {
    let size = [96, 96];
    let image = (0..96)
        .flat_map(|y| {
            (0..96).map(move |x| {
                let value =
                    0.04 + y as f32 * 0.0001 + if (47..50).contains(&x) { 0.08 } else { 0. };
                [value; 3]
            })
        })
        .collect::<Vec<_>>();
    let mask = circle(size, [48, 48], 4);
    let support = SupportMask::new([0, 0], size, mask.clone()).unwrap();
    let mut repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        offset: initial_offset(&support, active(size)).unwrap(),
        support,
    };
    repair.offset = suggest_offset(
        &repair,
        active(size),
        &mut |[x, y]| Ok(image[(y * 96 + x) as usize]),
        Instant::now() + Duration::from_secs(3),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap()
    .unwrap();
    let (result, _) = heal_ranked(&image, size, &mask, repair.offset).unwrap();
    assert!(outside_bits_equal(&result, &image, &mask));
    for (actual, expected) in result
        .iter()
        .zip(&image)
        .zip(&mask)
        .filter_map(|((actual, expected), hidden)| hidden.then_some((actual, expected)))
    {
        for channel in 0..3 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 4e-7,
                "{:?}: {} != {}",
                repair.offset,
                actual[channel],
                expected[channel]
            );
        }
    }
}

#[test]
fn a_wide_stroke_can_follow_a_slanted_wire_instead_of_copying_plain_background() {
    let size = [512, 512];
    let image = (0..size[1])
        .flat_map(|y| {
            (0..size[0]).map(move |x| {
                let wire = (4 * (x as i32 - 240) - (y as i32 - 256)).abs() <= 3;
                let noise = ((x * 73 + y * 29 + x * y * 11) % 97) as f32 / 97. * 0.001;
                [if wire { 0.02 + noise } else { 0.12 + noise }; 3]
            })
        })
        .collect::<Vec<_>>();
    let support = SupportMask::new([176, 246], [129, 21], vec![true; 129 * 21]).unwrap();
    let result = search_donor(
        &support,
        None,
        active(size),
        &mut |[x, y]| Ok(image[(y * size[0] + x) as usize]),
        Instant::now() + Duration::from_secs(3),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap();
    let SearchOutcome::Candidate(offset) = result else {
        panic!("wire fixture has clean placements")
    };
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support,
        offset,
    };
    let healed = evaluate(&repair, active(size), |[x, y]| {
        Ok(image[(y * size[0] + x) as usize])
    })
    .unwrap();
    for dy in [-8, -4, 0, 4, 8] {
        let point = [(240 + dy / 4) as u32, (256 + dy) as u32];
        assert!(
            healed.pixel(point, [0.; 3])[0] < 0.04,
            "{offset:?} lost the wire at {point:?}"
        );
    }
    assert!(
        healed.pixel([220, 256], [0.; 3])[0] > 0.1,
        "wire preservation must not darken the background"
    );
}

#[test]
fn search_reaches_a_matching_texture_beyond_the_immediate_margin() {
    let size = [256, 192];
    let mut image = gradient(size);
    for y in 80..=112 {
        for x in 48..=80 {
            let n = ((x * 73 + y * 29 + x * y * 11) % 97) as f32 / 97.;
            let pixel = [0.1 + n * 0.1, 0.2 - n * 0.05, 1.1 + n * 0.2];
            image[(y * size[0] + x) as usize] = pixel;
            image[(y * size[0] + x + 64) as usize] = pixel;
        }
    }
    let mask = circle(size, [64, 96], 5);
    let support = SupportMask::new([0, 0], size, mask.clone()).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        offset: initial_offset(&support, active(size)).unwrap(),
        support,
    };
    let selected = suggest_offset(
        &repair,
        active(size),
        &mut |[x, y]| Ok(image[(y * size[0] + x) as usize]),
        Instant::now() + Duration::from_secs(3),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap()
    .unwrap();
    let (result, matches) = heal_ranked(&image, size, &mask, selected).unwrap();
    assert!(matches[0].matching_score < 1e-8, "selected {selected:?}");
    for (actual, expected) in result
        .iter()
        .zip(&image)
        .zip(&mask)
        .filter_map(|((actual, expected), hidden)| hidden.then_some((actual, expected)))
    {
        for channel in 0..3 {
            assert!((actual[channel] - expected[channel]).abs() < 4e-7);
        }
    }
}

#[test]
fn comparable_context_matches_keep_local_texture_instead_of_a_distant_false_minimum() {
    let size = [256, 192];
    let mut image = gradient(size);
    for y in 80..=112 {
        for x in 48..=80 {
            let n = ((x * 73 + y * 29 + x * y * 11) % 97) as f32 / 97.;
            let pixel = [0.1 + n * 0.1; 3];
            image[(y * size[0] + x) as usize] = pixel;
            image[(y * size[0] + x + 32) as usize] = pixel.map(|v| v + 0.0102);
            image[(y * size[0] + x + 128) as usize] = pixel.map(|v| v + 0.01);
        }
    }
    let mask = circle(size, [64, 96], 5);
    // The distant crop has slightly better context but unrelated hidden content.
    // A constant tonal offset in the local donor is removed by reconstruction.
    for y in 91..=101 {
        for x in 59..=69 {
            if mask[(y * size[0] + x) as usize] {
                image[(y * size[0] + x + 128) as usize] = [2.; 3];
            }
        }
    }
    let support = SupportMask::new([0, 0], size, mask.clone()).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        offset: initial_offset(&support, active(size)).unwrap(),
        support,
    };
    let selected = suggest_offset(
        &repair,
        active(size),
        &mut |[x, y]| Ok(image[(y * size[0] + x) as usize]),
        Instant::now() + Duration::from_secs(3),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap()
    .unwrap();
    let (result, _) = heal_ranked(&image, size, &mask, selected).unwrap();
    let error = result
        .iter()
        .zip(&image)
        .zip(&mask)
        .filter_map(|((a, b), hidden)| hidden.then_some((a, b)))
        .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
        .fold(0., f32::max);
    assert!(error < 1e-6, "{selected:?}: reconstruction error {error}");
}

#[test]
fn expired_search_can_reuse_a_valid_previous_donor_without_reading_pixels() {
    let size = [96, 96];
    let support = SupportMask::new([0, 0], size, circle(size, [48, 48], 4)).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        offset: [30, 0],
        support,
    };
    assert_eq!(
        suggest_offset(
            &repair,
            active(size),
            &mut |_| panic!("expired search read pixels"),
            Instant::now(),
            SEARCH_ACCEPTABLE_SCORE,
            &mut Execution::default()
        )
        .unwrap(),
        Some([30, 0])
    );
}

#[test]
fn search_owns_its_last_resort_candidate_and_reports_impossible_or_cancelled_distinctly() {
    let area = active([96, 96]);
    let support = SupportMask::new([0, 0], [96, 96], circle([96, 96], [48, 48], 5)).unwrap();
    let result = search_donor(
        &support,
        None,
        area,
        &mut |_| panic!("expired search read pixels"),
        Instant::now(),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap();
    let SearchOutcome::Candidate(offset) = result else {
        panic!("feasible search returned {result:?}")
    };
    validate_offset(offset, &support, area).unwrap();
    let impossible = SupportMask::new([0, 0], [96, 96], vec![true; 96 * 96]).unwrap();
    assert_eq!(
        search_donor(
            &impossible,
            None,
            area,
            &mut |_| panic!("impossible search read pixels"),
            Instant::now(),
            SEARCH_ACCEPTABLE_SCORE,
            &mut Execution::default()
        )
        .unwrap(),
        SearchOutcome::Impossible
    );
    assert_eq!(
        search_donor(
            &support,
            None,
            area,
            &mut |_| panic!("cancelled search read pixels"),
            Instant::now(),
            SEARCH_ACCEPTABLE_SCORE,
            &mut Execution {
                cancelled: &|| true
            }
        )
        .unwrap(),
        SearchOutcome::Cancelled
    );
}

#[test]
fn quality_threshold_is_not_an_admission_threshold() {
    let size = [96, 96];
    let image = gradient(size);
    let support = SupportMask::new([0, 0], size, circle(size, [48, 48], 5)).unwrap();
    let result = search_donor(
        &support,
        None,
        active(size),
        &mut |[x, y]| Ok(image[(y * size[0] + x) as usize]),
        Instant::now() + Duration::from_secs(3),
        f64::NEG_INFINITY,
        &mut Execution::default(),
    )
    .unwrap();
    let SearchOutcome::Candidate(offset) = result else {
        panic!("no acceptable score should still produce a candidate")
    };
    validate_offset(offset, &support, active(size)).unwrap();
}

#[test]
fn healing_is_deterministic_for_a_stored_offset() {
    let size = [96, 96];
    let image = gradient(size);
    let mask = circle(size, [48, 48], 6);
    let offset = [-25, 0];
    let first = heal_ranked(&image, size, &mask, offset).unwrap();
    let second = heal_ranked(&image, size, &mask, offset).unwrap();
    assert_eq!(first, second);
}

#[test]
fn evaluate_uses_the_stored_offset_and_places_the_donor_once() {
    let size = [128, 128];
    let image = gradient(size);
    let active = active(size);
    let support = SupportMask::new([40, 40], [20, 20], vec![true; 400]).unwrap();
    let offset = nearest_offset(&support, active).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: support.clone(),
        offset,
    };
    let evaluated = evaluate(&repair, active, |[x, y]| {
        Ok(image[y as usize * size[0] as usize + x as usize])
    })
    .unwrap();
    assert_eq!(evaluated.donors.len(), 1);
    let target = donor_target(&support, active).unwrap();
    let donor = &evaluated.donors[0];
    assert_eq!(donor.target[0], target.x);
    assert_eq!(donor.target[1], target.y);
    assert_eq!(
        donor.source,
        [
            (target.x as i32 + offset[0]) as u32,
            (target.y as i32 + offset[1]) as u32
        ]
    );
    let again = evaluate(&repair, active, |[x, y]| {
        Ok(image[y as usize * size[0] as usize + x as usize])
    })
    .unwrap();
    assert_eq!(evaluated, again);
}

#[test]
fn empty_is_identity_and_invalid_masks_or_offsets_reject() {
    let size = [32, 32];
    let image = gradient(size);
    let empty = vec![false; image.len()];
    let (same, donors) = heal_ranked(&image, size, &empty, [4, 0]).unwrap();
    assert_eq!(same, image);
    assert!(donors.is_empty());
    assert!(heal_ranked(&image, size, &vec![true; image.len()], [4, 0]).is_err());
    let mask = circle(size, [16, 16], 3);
    assert!(heal_ranked(&image, size, &mask, [1_000, 0]).is_err());
}

#[test]
fn suggest_offset_returns_a_valid_translation_and_respects_its_deadline() {
    let size = [128, 128];
    let active = active(size);
    let image = vec![[0.1, 0.2, 0.3]; size[0] as usize * size[1] as usize];
    let mask = circle(size, [64, 64], 5);
    let support = SupportMask::new([0, 0], size, mask).unwrap();
    let offset = nearest_offset(&support, active).unwrap();
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support,
        offset,
    };
    let reads = std::cell::Cell::new(0usize);
    let mut base = |[x, y]: [u32; 2]| {
        reads.set(reads.get() + 1);
        Ok(image[y as usize * size[0] as usize + x as usize])
    };
    let suggested = suggest_offset(
        &repair,
        active,
        &mut base,
        Instant::now() + SEARCH_BUDGET,
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap()
    .expect("a valid translation exists");
    assert!(validate_offset(suggested, &repair.support, active).is_ok());
    let target = donor_target(&repair.support, active).unwrap();
    assert!(
        reads.get() < 3 * (target.width * target.height) as usize,
        "an acceptable first candidate must not read the rest of the search"
    );

    // A deadline that is already due still returns the nearest valid translation.
    let elapsed = suggest_offset(
        &repair,
        active,
        &mut base,
        Instant::now(),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(elapsed, repair.offset);

    let never = Execution {
        cancelled: &|| true,
    };
    assert!(
        suggest_offset(&repair, active, &mut base, Instant::now(), 0., &mut {
            never
        })
        .unwrap()
        .is_none()
    );
}
