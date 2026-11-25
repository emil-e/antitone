use super::*;
use crate::dust_repair::{donor_target, nearest_offset, test_support::*};

#[test]
fn healing_extreme_signed_context_remains_finite_without_range_preflight() {
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        offset: [21, 0],
        support: SupportMask::new([60, 60], [5, 5], vec![true; 25]).unwrap(),
    };
    let evaluated = evaluate(&repair, active([128, 128]), |point| {
        let value = if point[0] < 64 { 8e37 } else { -8e37 };
        Ok([value, -value, value])
    })
    .unwrap();
    assert!(
        evaluated
            .pixels()
            .all(|(_, rgb)| rgb.iter().all(|value| value.is_finite()))
    );
    assert!(
        evaluated
            .pixels()
            .any(|(_, rgb)| rgb.iter().any(|value| value.abs() > 1.))
    );
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
