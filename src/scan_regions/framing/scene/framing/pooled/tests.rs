//! Shared material-comparator/refinement evidence, not the retired Coherent detector.
use super::super::measurement::Pair;
use super::*;

fn support(values: impl Fn(usize) -> f64) -> Support {
    let pairs: Vec<_> = (0..64)
        .map(|index| Pair {
            along: (index as f64 + 0.5) / 64.,
            density: [values(index); 3],
            base_outside: true,
        })
        .collect();
    Support {
        stations: 64,
        paired: 64,
        positive: pairs.iter().filter(|pair| pair.density[0] > 0.).count(),
        base_outside: 0,
        pairs,
        ..Support::default()
    }
}

#[test]
fn coherent_magnitude_can_establish_an_edge_despite_many_tiny_reverse_signs() {
    let samples = support(|index| if index % 8 < 5 { 0.1 } else { -0.001 });
    assert_eq!(samples.kind(), Kind::Unresolved);
    assert!(pooled(&samples).unwrap().lower_bound > 0.);
}

#[test]
fn clear_bottom_segment_does_not_claim_the_upper_edge_was_observed() {
    let samples = support(|index| {
        if index >= 56 {
            0.2 + (index % 2) as f64 * 0.001
        } else if index % 2 == 0 {
            0.02
        } else {
            -0.02
        }
    });
    let pooled = pooled(&samples).unwrap();
    assert!(pooled.observed_span[0] > 0.875);
    assert_eq!(pooled.pairs, 8);
}

#[test]
fn observed_film_cut_segment_cannot_be_overridden_by_a_picture_like_segment() {
    let mut samples = support(|_| 0.2);
    samples.pairs.truncate(58);
    samples.paired = 58;
    samples.positive = 58;
    samples.film_limit = 6;
    assert_eq!(samples.kind(), Kind::Picture);
    assert!(pooled(&samples).is_none());
}

#[test]
fn adding_global_support_does_not_hide_the_same_strong_segment_from_ranking() {
    let local = support(|index| {
        if index >= 56 {
            0.2 + (index % 2) as f64 * 0.001
        } else if index % 2 == 0 {
            0.02
        } else {
            -0.02
        }
    });
    let global = support(|index| {
        if index >= 56 {
            0.2 + (index % 2) as f64 * 0.001
        } else {
            0.1
        }
    });
    let local = pooled(&local).unwrap();
    let global = pooled(&global).unwrap();
    assert!(local.rank_span.is_some());
    assert_eq!(local.rank_signal, global.rank_signal);
    assert_eq!(local.rank_span, global.rank_span);
}

#[test]
fn zero_mean_noise_or_reverse_polarity_does_not_gain_pooled_picture_support() {
    assert!(pooled(&support(|index| if index % 2 == 0 { 0.01 } else { -0.01 })).is_none());
    assert!(pooled(&support(|_| -0.1)).is_none());
}
