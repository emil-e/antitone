use super::*;

#[test]
fn stable_coloured_texture_outranks_nearer_gray_but_changing_channel_proportions() {
    let size = [160, 96];
    let stable = DVec3::new(2., 0.6, 0.3);
    let rgb: Vec<_> = (0..size[0] * size[1])
        .map(|i| {
            let x = (i % size[0]) as f64;
            let y = (i / size[0]) as f64;
            let direction = if x < 80. {
                stable
            } else {
                DVec3::new(
                    1. + 0.25 * (x * 0.4).sin(),
                    1. + 0.25 * (y * 0.23).cos(),
                    1. + 0.25 * ((x + y) * 0.31).sin(),
                )
            };
            direction * (0.3 + 0.08 * (x * 0.18).sin() * (y * 0.12).cos())
        })
        .collect();
    let selected = select(&rgb, &vec![true; rgb.len()], size, 1., &mut || false)
        .unwrap()
        .unwrap();
    assert!(selected.indices.len() >= 15);
    assert!(selected.indices.iter().all(|i| i % size[0] < 80 - RADIUS));
    assert!((selected.mean / selected.mean.y - stable / stable.y).length() < 1e-10);
}

#[test]
fn picture_population_sets_the_lowest_rank_quota_and_threshold_ties_are_retained() {
    let mut scores = vec![0.; 2500];
    let mut eligible = vec![false; scores.len()];
    for (i, score) in [4., 1., 2., 3., 9.].into_iter().enumerate() {
        scores[100 + i] = score;
        eligible[100 + i] = true;
    }
    assert_eq!(ranked_indices(&scores, &eligible), Some(vec![101, 102]));
    scores[102] = 1.;
    scores[103] = 1.;
    assert_eq!(
        ranked_indices(&scores, &eligible),
        Some(vec![101, 102, 103])
    );
    assert_eq!(ranked_indices(&scores, &vec![false; scores.len()]), None);
}

#[test]
fn hdr_positive_variation_remains_evidence_instead_of_becoming_capture_clipping() {
    let size = [64, 64];
    let rgb: Vec<_> = (0..size[0] * size[1])
        .map(|i| DVec3::splat(32. + (i % size[0]) as f64))
        .collect();
    let selected = select(&rgb, &vec![true; rgb.len()], size, 1., &mut || false)
        .unwrap()
        .unwrap();
    assert!(!selected.indices.is_empty());
    assert!(selected.mean.min_element() > 1.);
}

#[test]
fn gaussian_derivative_matches_direct_two_dimensional_kernel() {
    let size = [23, 19];
    let values: Vec<_> = (0..size[0] * size[1])
        .map(|i| ((i * 17 % 137) as f64 / 137.).sin())
        .collect();
    let actual = gradient(&values, size);
    for y in 0..size[1] {
        for x in 0..size[0] {
            let (mut dx, mut dy) = (0., 0.);
            for oy in -2isize..=2 {
                for ox in -2isize..=2 {
                    let sx = (x as isize + ox).clamp(0, size[0] as isize - 1) as usize;
                    let sy = (y as isize + oy).clamp(0, size[1] as isize - 1) as usize;
                    let g =
                        (-((ox * ox + oy * oy) as f64) / 0.5).exp() / (std::f64::consts::PI * 0.25);
                    dx -= ox as f64 * g * values[sy * size[0] + sx];
                    dy -= oy as f64 * g * values[sy * size[0] + sx];
                }
            }
            assert!((actual[y * size[0] + x] - dx.hypot(dy)).abs() < 1e-12);
        }
    }
}

#[test]
fn stability_recovers_a_cast_direction_but_does_not_certify_neutral_material() {
    let size = [96, 80];
    for direction in [DVec3::new(1.2, 1., 0.8), DVec3::new(2., 0.6, 0.3)] {
        let rgb: Vec<_> = (0..size[0] * size[1])
            .map(|i| direction * (0.18 + 0.06 * (i as f64 * 0.07).sin()))
            .collect();
        let mut valid = vec![true; rgb.len()];
        let invalid = [48, 40];
        valid[invalid[1] * size[0] + invalid[0]] = false;
        for gain in [1., 4.] {
            let selection = select(&rgb, &valid, size, gain, &mut || false)
                .unwrap()
                .unwrap();
            assert!(!selection.indices.is_empty());
            assert!((selection.mean / selection.mean.y - direction / direction.y).length() < 1e-10);
            for i in selection.indices {
                let (x, y) = (i % size[0], i / size[0]);
                assert!(x >= 8 && x < size[0] - 8 && y >= 8 && y < size[1] - 8);
                assert!(x.abs_diff(invalid[0]) > 8 || y.abs_diff(invalid[1]) > 8);
            }
        }
    }
}

#[test]
fn flat_dark_and_invalid_evidence_abstains_while_cancellation_returns_no_result() {
    let size = [64, 48];
    let rgb = vec![DVec3::splat(0.2); size[0] * size[1]];
    assert!(select(&rgb, &vec![true; rgb.len()], size, 1., &mut || false).is_err());
    assert!(select(&rgb, &vec![false; rgb.len()], size, 1., &mut || false).is_err());
    assert!(
        select(&rgb, &vec![true; rgb.len()], size, 1., &mut || true)
            .unwrap()
            .is_none()
    );
    let dark: Vec<_> = (0..rgb.len())
        .map(|i| DVec3::splat(0.005 + 0.004 * ((i % size[0]) as f64 * 0.3).sin()))
        .collect();
    let channel: Vec<_> = dark.iter().map(|p| p.x).collect();
    let contrast = gradient(&box_mean(&channel, size), size);
    assert!((RADIUS..size[0] - RADIUS).any(|x| contrast[size[0] * 24 + x] > 1e-4));
    assert!(select(&dark, &vec![true; rgb.len()], size, 1., &mut || false).is_err());
    assert!(
        select(&dark, &vec![true; rgb.len()], size, 4., &mut || false)
            .unwrap()
            .is_some()
    );
}
