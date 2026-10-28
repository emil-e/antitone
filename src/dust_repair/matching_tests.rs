use super::*;

fn noise(x: u32, y: u32) -> f32 {
    let mut value = x.wrapping_mul(0x9e37_79b9) ^ y.wrapping_mul(0x85eb_ca6b);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^= value >> 16;
    value as f32 / u32::MAX as f32 - 0.5
}

fn wire_image(size: u32, slope: f64, contrast: f32) -> Vec<Ap1> {
    let center = f64::from(size) / 2.;
    (0..size)
        .flat_map(|y| {
            (0..size).map(move |x| {
                let distance = (f64::from(y) - center - slope * (f64::from(x) - center))
                    / (1. + slope * slope).sqrt();
                let wire = (-0.5 * (distance / 0.8).powi(2)).exp() as f32;
                [0.2 + noise(x, y) * 0.02 - contrast * wire; 3]
            })
        })
        .collect()
}

fn circle(center: u32, radius: u32) -> SupportMask {
    let side = 2 * radius + 1;
    let bits = (0..side * side)
        .map(|index| {
            let x = i64::from(index % side) - i64::from(radius);
            let y = i64::from(index / side) - i64::from(radius);
            x * x + y * y <= i64::from(radius).pow(2)
        })
        .collect();
    SupportMask::new([center - radius; 2], [side; 2], bits).unwrap()
}

fn area(size: u32) -> SourceRegion {
    SourceRegion {
        x: 0,
        y: 0,
        width: size,
        height: size,
    }
}

fn targets(support: &SupportMask, size: u32, image: &[Ap1]) -> Vec<SearchTarget> {
    support
        .grouped_supports()
        .iter()
        .map(|group| {
            build_search_target(
                support,
                group,
                area(size),
                &mut |[x, y]| Ok(image[(y * size + x) as usize]),
                &mut Execution::default(),
            )
            .unwrap()
            .unwrap()
        })
        .collect()
}

#[test]
fn coherent_wire_alignment_outranks_a_grain_match_to_plain_sky() {
    let size = 512;
    let image = wire_image(size, 3. / 11., 0.02);
    let support = circle(size / 2, 40);
    let targets = targets(&support, size, &image);
    let matcher = matching::Matcher::new(&targets, &|| false).unwrap();
    let near = NearSupport::new(&support);
    // The first translation runs exactly along the analytic wire. The other
    // supplies clean sky; its grain happened to win under raw context matching.
    let aligned = [99, 27];
    let sky = [59, -58];
    for offset in [aligned, sky] {
        validate_offset(offset, &support, area(size)).unwrap();
    }
    let score = |offset| {
        matcher
            .score(
                &support,
                &near,
                offset,
                &mut |[x, y]| Ok(image[(y * size + x) as usize]),
                &mut Execution::default(),
            )
            .unwrap()
            .unwrap()
    };
    assert!(
        score(aligned) < score(sky),
        "background grain must not outweigh the crossing line"
    );
}

#[test]
fn a_wide_mask_finds_a_shallow_line_outside_the_cardinal_and_diagonal_probes() {
    let size = 512;
    let image = wire_image(size, 1. / 13., 0.08);
    let support = circle(size / 2, 64);
    // A clean matching translation exists independently of the search schedule.
    validate_offset([143, 11], &support, area(size)).unwrap();
    let result = search_donor(
        &support,
        None,
        area(size),
        &mut |[x, y]| Ok(image[(y * size + x) as usize]),
        Instant::now() + Duration::from_secs(3),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap();
    let SearchOutcome::Candidate([dx, dy]) = result else {
        panic!("{result:?}")
    };
    // Half a pixel of normal displacement retains the thin Gaussian line;
    // any distance along the line is acceptable. No particular donor is fixed.
    let displacement = (f64::from(dy) - f64::from(dx) / 13.).abs() / (1_f64 + 1. / 169.).sqrt();
    assert!(
        displacement < 0.5,
        "[{dx},{dy}] misses the wire by {displacement} pixels"
    );
}

#[test]
fn fine_texture_is_not_accepted_as_matching_blurred_background() {
    let size = 192;
    let mut image = vec![[0.2, 0.3, 1.1]; (size * size) as usize];
    for y in 80..=112 {
        for x in 48..=80 {
            let sign = if (x + y) % 2 == 0 { 1. } else { -1. };
            let value = [0.2 + sign * 0.02, 0.3 - sign * 0.01, 1.1 + sign * 0.02];
            image[(y * size + x) as usize] = value;
            image[(y * size + x + 64) as usize] = value;
        }
    }
    let support = SupportMask::new(
        [59, 91],
        [11; 2],
        (0..121)
            .map(|i| {
                let x = i % 11 - 5_i32;
                let y = i / 11 - 5_i32;
                x * x + y * y <= 25
            })
            .collect(),
    )
    .unwrap();
    // This clean previous donor has the right average colour, but no texture.
    let previous = [-32, 0];
    validate_offset(previous, &support, area(size)).unwrap();
    let result = search_donor(
        &support,
        Some(previous),
        area(size),
        &mut |[x, y]| Ok(image[(y * size + x) as usize]),
        Instant::now() + Duration::from_secs(3),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution::default(),
    )
    .unwrap();
    let SearchOutcome::Candidate(offset) = result else {
        panic!("{result:?}")
    };
    let repair = Repair {
        id: RepairId(1),
        enabled: true,
        origin: RepairOrigin::Manual,
        support,
        offset,
    };
    let healed = evaluate(&repair, area(size), |[x, y]| {
        Ok(image[(y * size + x) as usize])
    })
    .unwrap();
    for ([x, y], value) in healed.pixels() {
        for (actual, expected) in value.into_iter().zip(image[(y * size + x) as usize]) {
            assert!(
                (actual - expected).abs() < 1e-6,
                "{offset:?} replaced texture with background"
            );
        }
    }
}

#[test]
fn matching_cannot_read_marked_values_through_the_smoothing_footprint() {
    let size = 192;
    let image = wire_image(size, 3. / 11., 0.08);
    let support = circle(size / 2, 12);
    let targets = targets(&support, size, &image);
    let matcher = matching::Matcher::new(&targets, &|| false).unwrap();
    let near = NearSupport::new(&support);
    let offset = [27, 0];
    validate_offset(offset, &support, area(size)).unwrap();
    let score = matcher
        .score(
            &support,
            &near,
            offset,
            &mut |[x, y]| Ok(image[(y * size + x) as usize]),
            &mut Execution::default(),
        )
        .unwrap()
        .unwrap();
    let changed = matcher
        .score(
            &support,
            &near,
            offset,
            &mut |[x, y]| {
                Ok(if support.contains([x, y]) {
                    [-7., 3., 100.]
                } else {
                    image[(y * size + x) as usize]
                })
            },
            &mut Execution::default(),
        )
        .unwrap()
        .unwrap();
    assert!(score.is_finite());
    assert_eq!(score.to_bits(), changed.to_bits());
}

#[test]
fn cancellation_during_search_pixel_acquisition_does_not_publish_a_partial_match() {
    let support = circle(48, 5);
    let cancelled = std::cell::Cell::new(false);
    let reads = std::cell::Cell::new(0);
    let result = search_donor(
        &support,
        None,
        area(96),
        &mut |_| {
            reads.set(reads.get() + 1);
            if reads.get() == 50 {
                cancelled.set(true);
            }
            Ok([0.2; 3])
        },
        Instant::now() + Duration::from_secs(3),
        SEARCH_ACCEPTABLE_SCORE,
        &mut Execution {
            cancelled: &|| cancelled.get(),
        },
    )
    .unwrap();
    assert_eq!(result, SearchOutcome::Cancelled);
    assert!(
        reads.get() >= 50,
        "the cancellation must occur after acquisition begins"
    );
}
