//! Magnitude/noise evidence for newly measured geometry, not a relaxed sign-vote cutoff.
use super::*;

#[derive(Clone, Serialize)]
pub(super) struct Pooled {
    pub(super) method: &'static str,
    pub(super) pairs: usize,
    pub(super) blocks: usize,
    pub(super) observed_span: [f64; 2],
    pub(super) mean_density: [f64; 3],
    pub(super) standard_error: f64,
    pub(super) lower_bound: f64,
    pub(super) outside_base_fraction: f64,
    pub(super) rank_signal: f64,
    pub(super) rank_span: Option<[f64; 2]>,
}

fn evidence(
    values: &[[f64; 3]],
    pairs: &[&Pair],
    sigma: f64,
    method: &'static str,
) -> Option<Pooled> {
    if values.len() < 2 || pairs.is_empty() {
        return None;
    }
    let mean = std::array::from_fn::<_, 3, _>(|channel| {
        values.iter().map(|v| v[channel]).sum::<f64>() / values.len() as f64
    });
    if mean.iter().any(|&value| !value.is_finite() || value < 0.) {
        return None;
    }
    let scalar = mean.iter().sum::<f64>() / 3.;
    let variance = values
        .iter()
        .map(|v| {
            let delta = v.iter().sum::<f64>() / 3. - scalar;
            delta * delta
        })
        .sum::<f64>()
        / (values.len() - 1) as f64;
    let error = (variance / values.len() as f64).sqrt();
    let lower = scalar - sigma * error;
    if !lower.is_finite() || lower <= 0. {
        return None;
    }
    Some(Pooled {
        method,
        pairs: pairs.len(),
        blocks: if method == "spatial_blocks" {
            values.len()
        } else {
            1
        },
        observed_span: [
            pairs
                .iter()
                .map(|pair| pair.along)
                .fold(f64::INFINITY, f64::min),
            pairs
                .iter()
                .map(|pair| pair.along)
                .fold(f64::NEG_INFINITY, f64::max),
        ],
        mean_density: mean,
        standard_error: error,
        lower_bound: lower,
        outside_base_fraction: pairs.iter().filter(|pair| pair.base_outside).count() as f64
            / pairs.len() as f64,
        rank_signal: 0.,
        rank_span: None,
    })
}

pub(super) fn role_conflict(support: &Support) -> bool {
    support.contradicted()
        || support.film_limit >= 6
        || support.holder_limit >= 6
        || matches!(support.kind(), Kind::FilmLimit | Kind::HolderLimit)
}

/// Spatial blocks reduce the influence of adjacent correlated samples. Bounds are exploratory
/// signal tests, not calibrated physical-edge probabilities or a requirement on every channel vote.
pub(super) fn pooled(support: &Support) -> Option<Pooled> {
    // A supported non-picture segment cannot be outvoted by a positive image-like segment.
    if role_conflict(support) {
        return None;
    }
    let mut blocks: [Vec<&Pair>; 8] = std::array::from_fn(|_| vec![]);
    for pair in &support.pairs {
        blocks[((pair.along * 8.).floor() as usize).min(7)].push(pair);
    }
    let means: Vec<_> = blocks
        .iter()
        .filter(|block| block.len() >= 2)
        .map(|block| {
            std::array::from_fn(|channel| {
                block.iter().map(|pair| pair.density[channel]).sum::<f64>() / block.len() as f64
            })
        })
        .collect();
    let all: Vec<_> = support.pairs.iter().collect();
    let global = if all.len() >= 12 && means.len() >= 4 {
        evidence(&means, &all, 3., "spatial_blocks")
    } else {
        None
    };
    let mut best: Option<Pooled> = None;
    for block in &blocks {
        if block.len() < 6 {
            continue;
        }
        let values: Vec<_> = block.iter().map(|pair| pair.density).collect();
        if let Some(result) = evidence(&values, block, 4., "visible_segment")
            && best
                .as_ref()
                .is_none_or(|best| result.lower_bound > best.lower_bound)
        {
            best = Some(result);
        }
    }
    // Admission may use global pooling; ranking always uses the same fixed-window statistic.
    // Adding a valid global observation must not hide a stronger segment already observed.
    let rank_signal = best.as_ref().map_or(0., |segment| segment.lower_bound);
    let rank_span = best.as_ref().map(|segment| segment.observed_span);
    let mut admitted = global.or(best)?;
    admitted.rank_signal = rank_signal;
    admitted.rank_span = rank_span;
    Some(admitted)
}

#[cfg(test)]
mod tests;
