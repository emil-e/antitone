//! Conservative search acceleration; exact scores and final ranking remain with the healer.
use super::*;
use std::collections::HashMap;
#[cfg(test)]
mod tests;

#[cfg_attr(not(feature = "editor"), allow(dead_code))]
#[derive(Clone, Copy)]
pub(crate) struct ScreenInput<'a> {
    pub features: &'a [u32],
    /// Floor and ceiling weight units, packed into two nine-bit fields.
    pub weights: &'a [u32],
    pub candidates: &'a [[u32; 2]],
    pub width: u32,
    pub target: [u32; 2],
    pub local_width: u32,
    pub factors: [f32; 2],
    pub ceiling: f32,
    pub paired_weights: bool,
    pub upper_totals: [u32; 2],
}

pub(crate) trait Screener {
    /// Return false to use the exact CPU search without screening.
    fn admit(&self, frame: usize, context: usize, candidates: usize) -> bool;
    /// Returns no screening result when cancelled.
    fn screen(
        &mut self,
        input: ScreenInput<'_>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Option<Vec<[u64; 5]>>, Error>;
}

pub(crate) struct Execution<'a> {
    pub screener: Option<&'a mut dyn Screener>,
    pub cancelled: &'a dyn Fn() -> bool,
}

fn never_cancel() -> bool {
    false
}

impl Default for Execution<'_> {
    fn default() -> Self {
        Self {
            screener: None,
            cancelled: &never_cancel,
        }
    }
}

pub(super) struct Filter {
    x0: usize,
    y0: usize,
    nx: usize,
    bounds: Vec<f64>,
    pub scores: HashMap<[usize; 2], f64>,
    pub best: f64,
}

impl Filter {
    pub fn allows(&self, x: usize, y: usize, best: f64) -> bool {
        self.bounds[(y - self.y0) * self.nx + x - self.x0] <= competitive_score(best)
    }
}

// Valid colors are bins of the same computed binary64 features as exact scoring.
// For gradients, the real quotient lies between (q-1)/1024 and (q+2)/1024:
// quotient rounding at these bounded magnitudes is far smaller than one bin.
// A three-bin gap therefore bounds even the reference's subtract-then-divide path.
fn quantize(value: f64) -> i16 {
    let q = (value * 1024.).floor();
    if q.is_finite() && (-32767. ..=32767.).contains(&q) {
        q as i16
    } else {
        i16::MIN
    }
}

// Invalid/out-of-range screening coordinates contribute nothing. Photographic
// values are never quantized or clipped in exact scoring or reconstruction.
fn pack(c: MatchContext<'_>, cancelled: &dyn Fn() -> bool) -> Option<Vec<u32>> {
    let mut data = vec![0; c.image.len() * 5];
    for (i, pixel) in c.image.iter().enumerate() {
        if i % 1024 == 0 && cancelled() {
            return None;
        }
        for channel in 0..9 {
            let value = if channel < 3 {
                (f64::from(pixel[channel]) - c.center[channel]) / c.contrast
            } else if c.gradient_scale.is_finite() && c.gradient_scale > 0. {
                f64::from(c.gradients[i][channel - 3]) / c.gradient_scale
            } else {
                f64::NAN
            };
            data[i * 5 + channel / 2] |= u32::from(quantize(value) as u16) << (16 * (channel % 2));
        }
        data[i * 5 + 4] |= ((!c.mask[i]) as u32 | ((c.gradient_known[i] as u32) << 1)) << 16;
    }
    Some(data)
}

fn paired_coverage_needed(paired: [f64; 2], totals: MatchWeights) -> bool {
    paired[0] < totals.color * 0.875 || paired[1] < totals.gradient * 0.875
}

fn pack_weights(weights: &[f64]) -> Vec<u32> {
    weights
        .iter()
        .map(|w| (w * 256.).floor() as u32 | (((w * 256.).ceil() as u32) << 9))
        .collect()
}

fn lower_score(sum: [u64; 5], totals: MatchWeights) -> f64 {
    // Floored weights cannot exceed exact weights; paired weight sums cannot
    // exceed full target totals under their same-order positive reductions.
    // Paired sums of ceiling units / 256 are also upper bounds: every ceiling
    // dominates its input weight and all dyadic partial sums are exact (<2^30
    // integer units). Monotonic binary64 addition preserves that dominance.
    // Q^2 * weight units = 2^28. For <=4M cells, 24M positive binary64
    // additions lose <3e-9 relatively. 2^-24 also covers normalization,
    // products, integer conversion and the final score calculation.
    let units = (1u64 << 28) as f64;
    let color = totals.color.min(sum[3] as f64 / 256.);
    let gradient = totals.gradient.min(sum[4] as f64 / 256.);
    (0.75 * sum[0] as f64 / ((color * 3.) * units)
        + 0.25 * sum[1] as f64 / ((gradient * 6.) * units))
        * (1. - 2f64.powi(-24))
}

pub(super) fn prepare(
    mut c: MatchContext<'_>,
    totals: MatchWeights,
    eligibility: &DonorEligibility<'_>,
    range: [usize; 4],
    search: DonorSearch,
    execution: &mut Execution<'_>,
) -> Result<Option<Filter>, Error> {
    let [x0, y0, xend, yend] = range;
    let nx = xend - x0 + 1;
    let count = nx * (yend - y0 + 1);
    let Some(screen) = execution.screener.as_deref_mut() else {
        return Ok(None);
    };
    // Measurements favour the existing CPU search for tiny contexts. The proof's
    // reduction bound and transient admission are independent of UI mask limits.
    if !search.is_bounded()
        || c.known.len() < 1024
        || c.known.len() > 4_000_000
        || c.image.len() > 4_000_000
        || !c.contrast.is_finite()
        || c.contrast <= 0.
        || !totals.color.is_finite()
        || totals.color <= 0.
        || !totals.gradient.is_finite()
        || totals.gradient <= 0.
        || c.weights
            .iter()
            .zip(c.known)
            .any(|(w, known)| !w.is_finite() || !(0. ..=1.).contains(w) || (!known && *w != 0.))
        || !screen.admit(c.image.len(), c.known.len(), count)
    {
        return Ok(None);
    }

    let mut eligible = Vec::new();
    for y in y0..=yend {
        if (execution.cancelled)() {
            return Ok(None);
        }
        for x in x0..=xend {
            if eligibility.clean(x, y, search) {
                eligible.push([x, y]);
            }
        }
    }

    let mut nearest = eligible.clone();
    nearest.sort_by_key(|p| {
        (
            ((p[0] as i64 - c.target[0] as i64).pow(2) + (p[1] as i64 - c.target[1] as i64).pow(2)),
            p[1],
            p[0],
        )
    });
    let mut seeds = nearest.into_iter().take(64).collect::<Vec<_>>();
    for i in 0..64 {
        if !eligible.is_empty() {
            seeds.push(eligible[i * eligible.len() / 64]);
        }
    }
    if let Some(last) = eligible.last() {
        seeds.push(*last);
    }
    seeds.sort_unstable();
    seeds.dedup();

    let mut best = f64::INFINITY;
    let mut scores = HashMap::new();
    for source in seeds {
        if (execution.cancelled)() {
            return Ok(None);
        }
        c.source = source;
        if let Ok(Some(score)) = matching_score_bounded(c, totals, Some(competitive_score(best))) {
            if !score.is_finite() {
                return Ok(None);
            }
            best = best.min(score);
            scores.insert(source, score);
        }
    }

    let mut bounds = vec![f64::NAN; count]; // Ineligible, including when best is infinity.
    for p in &eligible {
        bounds[(p[1] - y0) * nx + p[0] - x0] = 0.;
    }
    if best == 0. || eligible.is_empty() {
        return Ok(Some(Filter {
            x0,
            y0,
            nx,
            bounds,
            scores,
            best,
        }));
    }

    let Some(features) = pack(c, execution.cancelled) else {
        return Ok(None);
    };
    let weights = pack_weights(c.weights);
    let positions = eligible
        .iter()
        .map(|p| [p[0] as u32, p[1] as u32])
        .collect::<Vec<_>>();
    // Missing paired context can make full-target denominators much too loose.
    // Pay for per-candidate coverage only when the best seed exposes that loss;
    // choosing either upper bound changes work, never eligibility or final scores.
    let best_seed = scores
        .iter()
        .min_by(|a, b| a.1.total_cmp(b.1).then_with(|| a.0.cmp(b.0)))
        .map(|(p, _)| *p)
        .unwrap_or(eligible[0]);
    let mut paired = [0.; 2];
    let mut upper_totals = [0u32; 2];
    for (i, &weight) in c.weights.iter().enumerate() {
        let source = (best_seed[1] + i / c.size[0]) * c.width + best_seed[0] + i % c.size[0];
        if c.known[i] && !c.mask[source] {
            paired[0] += weight;
        }
        if c.valid_target[i] && c.gradient_known[source] {
            paired[1] += weight;
        }
        upper_totals[0] += weights[i] >> 9;
        if c.valid_target[i] {
            upper_totals[1] += weights[i] >> 9;
        }
    }

    let paired_weights = paired_coverage_needed(paired, totals);
    let units = (1u64 << 28) as f64;
    let Some(sums) = screen.screen(
        ScreenInput {
            features: &features,
            weights: &weights,
            candidates: &positions,
            width: c.width as u32,
            target: c.target.map(|v| v as u32),
            local_width: c.size[0] as u32,
            factors: [
                (0.75 / (totals.color * 3. * units)) as f32,
                (0.25 / (totals.gradient * 6. * units)) as f32,
            ],
            ceiling: competitive_score(best) as f32,
            paired_weights,
            upper_totals,
        },
        execution.cancelled,
    )?
    else {
        return Ok(None);
    };
    if sums.len() != eligible.len() {
        return Err(Error::Gpu("repair screening result length differs".into()));
    }

    for (p, sum) in eligible.iter().zip(sums) {
        if sum[2] > c.known.len() as u64
            || sum[0] > sum[2] * 3 * 4095u64.pow(2) * 256
            || sum[1] > sum[2] * 6 * 4095u64.pow(2) * 256
            || sum[3] > c.known.len() as u64 * 256
            || sum[4] > c.known.len() as u64 * 256
            || (sum[3] == 0 && sum[0] != 0)
            || (sum[4] == 0 && sum[1] != 0)
        {
            return Err(Error::Gpu("invalid repair screening bound".into()));
        }
        let insufficient = sum[3] as f64 / 256. < (totals.color * 0.5).max(1e-12)
            || sum[4] as f64 / 256. < (totals.gradient * 0.5).max(1e-12);
        bounds[(p[1] - y0) * nx + p[0] - x0] = if insufficient {
            f64::NAN
        } else {
            lower_score(sum, totals)
        };
    }
    Ok(Some(Filter {
        x0,
        y0,
        nx,
        bounds,
        scores,
        best,
    }))
}
