use super::*;

#[test]
fn linear_distance_transform_preserves_every_reference_distance_bit() {
    assert!(exact_distance_squares((1 << 26) + 1, (1 << 26) + 1));
    assert!(!exact_distance_squares((1 << 26) + 2, (1 << 26) + 1));
    assert!(exact_distance_squares(94_906_266, 1));
    assert!(!exact_distance_squares(94_906_267, 1));
    assert!(!exact_distance_squares(usize::MAX, usize::MAX));
    // Exercise fallback dispatch without allocating an enormous raster: the
    // scalar helper depends only on the supplied indices, not a stored height.
    let distant = [true, false];
    assert_eq!(
        distance_to_unknown(&distant, 94_906_267, 1, &|| false),
        distance_to_unknown_scalar(&distant, 94_906_267, &|| false)
    );
    let check = |known: &[bool], width, height| {
        let exact = distance_to_unknown(known, width, height, &|| false).unwrap();
        let reference = distance_to_unknown_scalar(known, width, &|| false).unwrap();
        assert_eq!(
            exact.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            reference.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            "{width}x{height}"
        );
    };
    for bits in 0..1u32 << 12 {
        let known = (0..12).map(|i| bits & (1 << i) == 0).collect::<Vec<_>>();
        check(&known, 4, 3);
    }
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

#[test]
fn repair_set_keeps_stable_independent_repairs() {
    let mut set = RepairSet::default();
    let a = SupportMask::new([10, 10], [3, 3], vec![true; 9]).unwrap();
    let b = SupportMask::new([20, 20], [2, 2], vec![true; 4]).unwrap();
    let first = set
        .insert(a.clone(), RepairOrigin::Manual, DonorChoice::Automatic)
        .unwrap();
    let second = set
        .insert(
            b.clone(),
            RepairOrigin::Detected,
            DonorChoice::Offset([-3, 2]),
        )
        .unwrap();
    assert_eq!(set.selected(), Some(second));
    let unchanged = set.repairs()[1].clone();
    set.set_enabled(first, false).unwrap();
    set.set_donor(first, DonorChoice::Offset([4, -2])).unwrap();
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
    assert!(set.select(Some(first)).is_err());
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
    let donor = DonorChoice::Offset([-25, 0]);
    let (left, left_match) = heal_ranked(&a, size, &mask, donor).unwrap();
    let (right, right_match) = heal_ranked(&b, size, &mask, donor).unwrap();
    assert_eq!(left_match, right_match);
    assert_eq!(left_match[0].target, [32, 32, 33, 33]);
    assert_eq!(left_match[0].source, [7, 32]);
    assert_eq!(left_match[0].paired_context_pixels, 976);
    assert_eq!(
        left[48 * 96 + 48].map(f32::to_bits),
        [0x3d37_8035, 0xbc2a_64c3, 0x3f8e_a4a8]
    );
    assert!(
        left.iter()
            .zip(&right)
            .all(|(a, b)| a.iter().zip(b).all(|(a, b)| a.to_bits() == b.to_bits()))
    );
    assert!(outside_bits_equal(&left, &a, &mask));
    assert!(
        left.iter()
            .zip(&mask)
            .filter(|(_, hidden)| **hidden)
            .all(|(pixel, _)| pixel[1] < 0. && pixel[2] > 1.)
    );
}

#[test]
fn nearby_masks_are_one_independent_repair_group() {
    let size = [96, 96];
    let image = gradient(size);
    let mut mask = vec![false; image.len()];
    for y in 46..49 {
        for x in 46..49 {
            mask[y * 96 + x] = true;
            mask[y * 96 + x + 4] = true;
        }
    }
    let (result, donors) = heal_ranked(&image, size, &mask, DonorChoice::Offset([-25, 0])).unwrap();
    assert_eq!(donors.len(), 1);
    assert!(outside_bits_equal(&result, &image, &mask));
    for (result, expected) in result
        .iter()
        .zip(&image)
        .zip(&mask)
        .filter_map(|((result, expected), hidden)| hidden.then_some((result, expected)))
    {
        for channel in 0..3 {
            assert!((result[channel] - expected[channel]).abs() < 4e-7);
        }
    }
}

#[test]
fn automatic_ranked_match_preserves_a_crossing_wire() {
    let size = [96, 96];
    let mut image = Vec::with_capacity(96 * 96);
    for y in 0..96 {
        for x in 0..96 {
            let mut value = 0.04 + y as f32 * 0.0001;
            if (47..50).contains(&x) {
                value += 0.08;
            }
            image.push([value; 3]);
        }
    }
    let mask = circle(size, [48, 48], 4);
    let (result, donors) = heal_ranked(&image, size, &mask, DonorChoice::Automatic).unwrap();
    assert_eq!(donors.len(), 1);
    assert_eq!(donors[0].target, [34, 34, 29, 29]);
    assert_eq!(donors[0].source, [34, 23]);
    assert!(outside_bits_equal(&result, &image, &mask));
    for (result, expected) in result
        .iter()
        .zip(&image)
        .zip(&mask)
        .filter_map(|((result, expected), hidden)| hidden.then_some((result, expected)))
    {
        for channel in 0..3 {
            assert!((result[channel] - expected[channel]).abs() < 4e-7);
        }
    }
}

#[test]
fn automatic_evaluation_matches_direct_healer() {
    let size = [96, 96];
    let mut image = gradient(size);
    for x in 0..size[0] {
        image[(48 * size[0] + x) as usize] = [0.7, 0.6, 0.5];
    }
    let bits = circle([13, 13], [6, 6], 6);
    let support = SupportMask::new([42, 42], [13, 13], bits.clone()).unwrap();
    for y in 0..13 {
        for x in 0..13 {
            if bits[(y * 13 + x) as usize] {
                image[((y + 42) * size[0] + x + 42) as usize] = [-100., 40., 80.];
            }
        }
    }
    let repair = Repair {
        id: RepairId(8),
        enabled: true,
        origin: RepairOrigin::Detected,
        support,
        donor: DonorChoice::Automatic,
    };
    let evaluated = evaluate(
        &repair,
        SourceRegion {
            x: 0,
            y: 0,
            width: size[0],
            height: size[1],
        },
        |[x, y]| Ok(image[(y * size[0] + x) as usize]),
    )
    .unwrap();
    let mut full_mask = vec![false; image.len()];
    for y in 0..13 {
        for x in 0..13 {
            full_mask[((y + 42) * size[0] + x + 42) as usize] = bits[(y * 13 + x) as usize];
        }
    }
    let (full, donors) = heal_ranked(&image, size, &full_mask, DonorChoice::Automatic).unwrap();
    assert_eq!(evaluated.donors, donors);
    for (index, base) in image.iter().enumerate() {
        let point = [index as u32 % size[0], index as u32 / size[0]];
        assert_eq!(evaluated.pixel(point, *base), full[index]);
    }
}

#[test]
fn independently_evaluated_repairs_compose_in_either_order() {
    let size = [96, 96];
    let image = gradient(size);
    let make = |id, origin, donor| Repair {
        id: RepairId(id),
        enabled: true,
        origin: RepairOrigin::Manual,
        support: SupportMask::new(origin, [7, 7], circle([7, 7], [3, 3], 3)).unwrap(),
        donor: DonorChoice::Offset(donor),
    };
    let active = SourceRegion {
        x: 0,
        y: 0,
        width: size[0],
        height: size[1],
    };
    let base = |[x, y]: [u32; 2]| -> Result<Ap1, Error> { Ok(image[(y * size[0] + x) as usize]) };
    let first = evaluate(&make(1, [20, 40], [0, -20]), active, base).unwrap();
    let second = evaluate(&make(2, [70, 40], [0, -20]), active, base).unwrap();
    let changed_second = evaluate(&make(2, [70, 40], [0, 20]), active, base).unwrap();
    let first_again = evaluate(&make(1, [20, 40], [0, -20]), active, base).unwrap();
    assert_eq!(first, first_again);
    assert_ne!(second, changed_second);
    for (index, original) in image.iter().enumerate() {
        let point = [index as u32 % size[0], index as u32 / size[0]];
        let forward = second.pixel(point, first.pixel(point, *original));
        let reverse = first.pixel(point, second.pixel(point, *original));
        assert_eq!(forward, reverse);
        if !first.support().contains(point) && !second.support().contains(point) {
            assert_eq!(forward.map(f32::to_bits), original.map(f32::to_bits));
        }
        if !changed_second.support().contains(point) {
            assert_eq!(
                first.pixel(point, changed_second.pixel(point, *original)),
                first.pixel(point, *original)
            );
        }
    }
}

#[test]
fn bounded_evaluation_uses_base_ap1_and_ignores_ui_provenance() {
    use std::cell::Cell;
    let support = SupportMask::new([500, 500], [13, 13], circle([13, 13], [6, 6], 6)).unwrap();
    let make = |id, origin| Repair {
        id: RepairId(id),
        enabled: true,
        origin,
        support: support.clone(),
        donor: DonorChoice::Offset([-25, 0]),
    };
    let active = SourceRegion {
        x: 100,
        y: 200,
        width: 1_000,
        height: 800,
    };
    let samples = Cell::new(0usize);
    let pixel = |[x, y]: [u32; 2]| -> Result<Ap1, Error> {
        samples.set(samples.get() + 1);
        Ok([
            0.04 + x as f32 * 0.0001,
            -0.02 + y as f32 * 0.0002,
            1.1 + x as f32 * 0.0003,
        ])
    };
    let manual = evaluate(&make(1, RepairOrigin::Manual), active, pixel).unwrap();
    assert!(samples.get() < (active.width * active.height) as usize / 100);
    let detected = evaluate(
        &make(2, RepairOrigin::Detected),
        active,
        |[x, y]: [u32; 2]| -> Result<Ap1, Error> {
            Ok([
                0.04 + x as f32 * 0.0001,
                -0.02 + y as f32 * 0.0002,
                1.1 + x as f32 * 0.0003,
            ])
        },
    )
    .unwrap();
    assert_eq!(manual.donors, detected.donors);
    assert_eq!(manual.values, detected.values);
    assert_eq!(manual.donors[0].target, [490, 490, 33, 33]);
    assert_eq!(manual.donors[0].source, [465, 490]);
    let base = [9., 8., 7.];
    assert_eq!(manual.pixel([499, 506], base), base);
    let repaired = manual.pixel([506, 506], base);
    let expected = [0.0906, 0.0812, 1.2518];
    for channel in 0..3 {
        assert!((repaired[channel] - expected[channel]).abs() < 4e-7);
    }
}

fn assert_exhaustive_search(image: &[Ap1], size: [u32; 2], mask: &[bool]) {
    let fast = heal_ranked(image, size, mask, DonorChoice::Automatic);
    let reference = heal_ranked_with_search(
        image,
        size,
        mask,
        DonorChoice::Automatic,
        DonorSearch::Exhaustive,
    );
    match (fast, reference) {
        (Ok((pixels, donors)), Ok((expected, expected_donors))) => {
            assert_eq!(donors, expected_donors);
            for (a, b) in donors.iter().zip(&expected_donors) {
                assert_eq!(a.matching_score.to_bits(), b.matching_score.to_bits());
                assert_eq!(a.context_mse.to_bits(), b.context_mse.to_bits());
            }
            assert!(
                pixels
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| a.map(f32::to_bits) == b.map(f32::to_bits))
            );
            assert!(outside_bits_equal(&pixels, image, mask));
        }
        (Err(a), Err(b)) => assert_eq!(a.to_string(), b.to_string()),
        (a, b) => panic!("bounded/exhaustive disagreement: {a:?} vs {b:?}"),
    }
}

#[test]
fn bounded_search_matches_exhaustive_flat_structure_noise_hdr_and_disconnected_support() {
    let size = [64, 60];
    let one = circle(size, [32, 30], 3);
    let far = circle(size, [19, 20], 2)
        .into_iter()
        .zip(circle(size, [42, 38], 2))
        .map(|(a, b)| a || b)
        .collect::<Vec<_>>();
    let close = circle(size, [29, 30], 2)
        .into_iter()
        .zip(circle(size, [36, 30], 2))
        .map(|(a, b)| a || b)
        .collect::<Vec<_>>();
    let mut seed = 0x1f73459bu32;
    for kind in 0..6 {
        let image = (0..size[0] * size[1])
            .map(|i| {
                let (x, y) = (i % size[0], i / size[0]);
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32;
                match kind {
                    0 => [0.1, -0.2, 1.3],
                    1 => [0.04 + x as f32 * 0.001, -0.02 + y as f32 * 0.001, 1.1],
                    2 => {
                        let v = if (30..33).contains(&x) { 0.3 } else { 0.04 };
                        [v, v * 0.7, 1. + v]
                    }
                    3 => [noise * 0.2 - 0.04, noise * 0.7 - 0.1, 1. + noise],
                    4 => {
                        let v = ((x / 4 + y / 4) % 2) as f32 * 0.1;
                        [v, -0.2 + v, 1.3 + v]
                    }
                    _ => [1e24 + x as f32 * 1e20, -2e24 + y as f32 * 1e20, 3e24],
                }
            })
            .collect::<Vec<_>>();
        for mask in [&one, &far, &close] {
            assert_exhaustive_search(&image, size, mask);
        }
        let changed = image
            .iter()
            .zip(&one)
            .map(|(p, masked)| if *masked { [-1e25, 1e25, 0.] } else { *p })
            .collect::<Vec<_>>();
        assert_exhaustive_search(&changed, size, &one);
        assert_eq!(
            heal_ranked(&image, size, &one, DonorChoice::Automatic).unwrap(),
            heal_ranked(&changed, size, &one, DonorChoice::Automatic).unwrap()
        );
    }
}

#[test]
fn donor_selection_keeps_the_exact_margin_distance_score_and_coordinate_policy() {
    let target = [20, 20];
    let margin = competitive_score(1.);
    let just_above = f64::from_bits(margin.to_bits() + 1);
    assert_eq!(
        select_donor(&[(1., 5, 5), (1.005, 20, 18), (1.02, 20, 19)], target),
        Some((1.005, 20, 18))
    );
    assert_eq!(
        select_donor(
            &[(1., 5, 5), (margin, 20, 19), (just_above, 20, 20)],
            target
        ),
        Some((margin, 20, 19))
    );
    assert_eq!(
        select_donor(&[(1.005, 20, 19), (1., 20, 21)], target),
        Some((1., 20, 21))
    );
    assert_eq!(
        select_donor(&[(1., 20, 21), (1., 20, 19), (1., 19, 20)], target),
        Some((1., 19, 20))
    );
    assert_eq!(
        select_donor(&[(1., 20, 21), (1., 20, 19)], target),
        Some((1., 20, 19))
    );
}

#[test]
fn zero_best_pruning_preserves_nearer_epsilon_and_equal_distance_score_ties() {
    let target = [20, 20];
    let near = (8e-13, 19, 20);
    let winner = select_donor(&[near, (0., 20, 0), (1.01e-12, 20, 20)], target).unwrap();
    assert_eq!(winner, near);
    assert!(!can_beat_zero_winner(winner, [18, 20], target));
    assert!(can_beat_zero_winner(winner, [19, 20], target));
    let perfect = (0., 20, 19);
    assert_eq!(
        select_donor(&[near, (0., 20, 0), perfect], target),
        Some(perfect)
    );
    assert!(!can_beat_zero_winner(perfect, [21, 20], target));
    assert!(can_beat_zero_winner(perfect, [20, 19], target));
}

#[test]
fn score_cutoff_never_discards_a_candidate_on_or_below_its_limit() {
    let width = 48;
    let height = 24;
    let image = gradient([width as u32, height as u32]);
    let gradients = sobel(&gaussian_blur(&image, width, height), width, height);
    let mask = vec![false; width * height];
    let gradient_known = vec![true; width * height];
    let known = (0..63).map(|i| i % 7 != 2).collect::<Vec<_>>();
    let valid = (0..63).map(|i| i % 5 != 3 && known[i]).collect::<Vec<_>>();
    let weights = known
        .iter()
        .enumerate()
        .map(|(i, known)| {
            if *known {
                0.15 + (i % 13) as f64 * 0.05
            } else {
                0.
            }
        })
        .collect::<Vec<_>>();
    let context = MatchContext {
        image: &image,
        mask: &mask,
        gradients: &gradients,
        gradient_known: &gradient_known,
        width,
        target: [4, 4],
        source: [25, 8],
        size: [9, 7],
        known: &known,
        valid_target: &valid,
        weights: &weights,
        center: [0.04, -0.02, 1.1],
        contrast: 0.3,
        gradient_scale: 0.1,
    };
    let mut partial_mask = mask.clone();
    let mut partial_gradient = gradient_known.clone();
    for i in 0..63 {
        let source = (8 + i / 9) * width + 25 + i % 9;
        partial_mask[source] = i % 11 == 0;
        partial_gradient[source] = i % 13 != 0;
    }
    for context in [
        context,
        MatchContext {
            mask: &partial_mask,
            gradient_known: &partial_gradient,
            ..context
        },
    ] {
        let full = matching_score(context).unwrap();
        assert!(full > 0. && full.is_finite());
        let totals = MatchWeights::new(&weights, &valid);
        let mut pruned = false;
        for limit in [
            0.,
            full * 0.5,
            f64::from_bits(full.to_bits() - 1),
            full,
            f64::from_bits(full.to_bits() + 1),
            competitive_score(full),
            f64::INFINITY,
        ] {
            match matching_score_bounded(context, totals, Some(limit)).unwrap() {
                Some(score) => assert_eq!(score.to_bits(), full.to_bits()),
                None => {
                    assert!(full > limit);
                    pruned = true;
                }
            }
            if limit >= full {
                assert_eq!(
                    matching_score_bounded(context, totals, Some(limit))
                        .unwrap()
                        .unwrap()
                        .to_bits(),
                    full.to_bits()
                );
            }
        }
        assert!(pruned);
    }
    let bad = MatchContext {
        gradient_known: &vec![false; width * height],
        ..context
    };
    assert!(matching_score(bad).is_err());
}

#[test]
fn prepared_donor_eligibility_matches_scalar_required_pixels_including_sparse_bounds() {
    let (width, height, local_width, local_height) = (18, 14, 5, 4);
    for mask_kind in 0..5 {
        let mask = (0..width * height)
            .map(|i| match mask_kind {
                0 => false,
                1 => true,
                2 => i % 23 == 0,
                3 => (7..10).contains(&(i % width)) && (5..8).contains(&(i / width)),
                _ => i == 0 || i == width * height - 1,
            })
            .collect::<Vec<_>>();
        for required_kind in 0..5 {
            let required = (0..local_width * local_height)
                .map(|i| match required_kind {
                    0 => false,
                    1 => true,
                    2 => i == 0 || i == local_width * local_height - 1,
                    3 => i % 2 == 0,
                    _ => i == local_width + 2,
                })
                .collect::<Vec<_>>();
            let prepared = DonorEligibility::new(
                &mask,
                width,
                selected_bounds(&mask, width),
                local_width,
                &required,
            );
            for y in 0..=height - local_height {
                for x in 0..=width - local_width {
                    assert_eq!(
                        prepared.clean(x, y, DonorSearch::Bounded),
                        prepared.clean(x, y, DonorSearch::Exhaustive),
                        "mask={mask_kind} required={required_kind} source={x},{y}"
                    );
                }
            }
        }
    }
}

#[test]
fn bounded_and_exhaustive_search_reject_the_same_ineligible_and_invalid_inputs() {
    let size = [32, 32];
    let image = gradient(size);
    for mask in [
        circle(size, [16, 16], 12),
        circle(size, [2, 2], 2),
        vec![true; image.len()],
    ] {
        assert_exhaustive_search(&image, size, &mask);
    }
    let mut invalid = image;
    invalid[0][0] = f32::NAN;
    assert_exhaustive_search(&invalid, size, &circle(size, [16, 16], 2));
}

#[test]
fn empty_is_identity_and_invalid_masks_or_donors_reject() {
    let size = [32, 32];
    let image = gradient(size);
    let empty = vec![false; image.len()];
    let (same, donors) = heal_ranked(&image, size, &empty, DonorChoice::Automatic).unwrap();
    assert_eq!(same, image);
    assert!(donors.is_empty());
    assert!(
        heal_ranked(
            &image,
            size,
            &vec![true; image.len()],
            DonorChoice::Automatic
        )
        .is_err()
    );
    let mask = circle(size, [16, 16], 3);
    for donor in [DonorChoice::Offset([0, 0]), DonorChoice::Offset([1_000, 0])] {
        assert!(heal_ranked(&image, size, &mask, donor).is_err());
    }
}
