//! One log-chroma centering vote per photograph, at the already fitted gamma.
use super::*;
use crate::color::{ap1_to_cones, opponent_coordinates};
use glam::DVec2;

const WB_SAMPLES_PER_FRAME: usize = 1024;
const CONE_FLOOR: f64 = 1e-8;

pub(super) fn estimate(
    picture: &SampleBatch,
    negative: NegativeInput,
    camera_to_ap1: DMat3,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<(Balance, Vec<usize>)>, Error> {
    let ap1_to_cones = ap1_to_cones();
    let to_cones = ap1_to_cones * camera_to_ap1;
    let white = (ap1_to_cones * DVec3::ONE).ln();
    let mut chroma = vec![Vec::new(); picture.groups.len()];
    if !picture.read(model, negative, cancelled, |frame, _, positive| {
        let cones = to_cones * positive;
        if !cones.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        if cones.min_element() > CONE_FLOOR {
            chroma[frame].push(opponent_coordinates(cones.ln() - white));
        }
        Ok(())
    })? {
        return Ok(None);
    }

    let mut warmth = Vec::new();
    let mut tint = Vec::new();
    let mut counts = Vec::with_capacity(chroma.len());
    for frame in &chroma {
        if cancelled() {
            return Ok(None);
        }
        let count = frame.len().min(WB_SAMPLES_PER_FRAME);
        counts.push(count);
        if count == 0 {
            continue;
        }
        let sum: DVec2 = (0..count).map(|i| centre_sample(frame, count, i)).sum();
        // In log-cone opponents, photographic WB adds its two coordinates.
        let correction = -sum / count as f64;
        warmth.push(correction.x);
        tint.push(correction.y);
    }
    if warmth.is_empty() {
        return Err(Error::UnusableSample(
            "no usable picture chroma samples for roll white balance",
        ));
    }

    // Keep the frame votes unbounded; only the installed roll correction uses control limits.
    let wb = Balance {
        warmth: quantile(&mut warmth, 0.5).clamp(-1., 1.),
        tint: quantile(&mut tint, 0.5).clamp(-1., 1.),
    };
    Ok(Some((wb, counts)))
}

#[cfg(feature = "editor")]
pub(crate) fn semantic_refit(
    proposal: &mut Proposal,
    frames: &[FrameSamples<'_>],
    matrix: DMat3,
    model: &mut impl ForwardModel,
    cancelled: &mut impl FnMut() -> bool,
    mut classify: impl FnMut(
        usize,
        &[usize],
        &mut dyn FnMut() -> bool,
    ) -> Result<Option<Vec<bool>>, Error>,
) -> Result<bool, Error> {
    let negative = proposal
        .input
        .negative_input()
        .expect("anchor proposal contains calibration");
    let picture = SampleBatch::new(frames.iter().map(|frame| frame.picture));
    let to_cones = ap1_to_cones() * matrix;
    let white = (ap1_to_cones() * DVec3::ONE).ln();
    let mut chroma = vec![Vec::new(); frames.len()];
    if !picture.read(model, negative, cancelled, |frame, index, positive| {
        let cones = to_cones * positive;
        if !cones.is_finite() {
            return Err(Error::NumericalOverflow);
        }
        if cones.min_element() > CONE_FLOOR {
            chroma[frame].push((index, opponent_coordinates(cones.ln() - white)));
        }
        Ok(())
    })? {
        return Ok(false);
    }

    let mut votes = Vec::new();
    let mut counts = Vec::new();
    for (frame, sites) in chroma.iter().enumerate() {
        if cancelled() {
            return Ok(false);
        }
        let count = sites.len().min(WB_SAMPLES_PER_FRAME);
        let candidates: Vec<_> = (0..count).map(|i| centre_sample(sites, count, i)).collect();
        let indices: Vec<_> = candidates.iter().map(|&(i, _)| i).collect();
        let Some(kept) = classify(frame, &indices, cancelled)? else {
            return Ok(false);
        };
        if kept.len() != count {
            return Err(Error::InvalidParameter("semantic WB sample count changed"));
        }
        let mut n = 0;
        let mut sum = DVec2::ZERO;
        for ((_, value), keep) in candidates.into_iter().zip(kept) {
            if keep {
                n += 1;
                sum += value;
            }
        }
        votes.push((n >= 256).then(|| -sum / n as f64));
        counts.push((count, n));
    }
    let mut warmth: Vec<_> = votes.iter().flatten().map(|v| v.x).collect();
    let mut tint: Vec<_> = votes.iter().flatten().map(|v| v.y).collect();
    if warmth.len() < frames.len().div_ceil(2) {
        log::info!(
            "Analyze semantic WB abstained: {} of {} photos could vote",
            warmth.len(),
            frames.len()
        );
        return Ok(!cancelled());
    }
    let wb = Balance {
        warmth: quantile(&mut warmth, 0.5).clamp(-1., 1.),
        tint: quantile(&mut tint, 0.5).clamp(-1., 1.),
    };
    let Some((exposure, exposure_counts)) =
        meter(&picture, negative, matrix, wb, model, cancelled)?
    else {
        return Ok(false);
    };
    for (stats, ((eligible, n), exposure_samples)) in proposal
        .frames
        .iter_mut()
        .zip(counts.into_iter().zip(exposure_counts))
    {
        stats.balance_eligible = eligible;
        stats.balance_samples = n;
        stats.exposure_samples = exposure_samples;
    }
    proposal.roll = RollSettings { wb, exposure };
    Ok(!cancelled())
}

#[cfg(test)]
mod tests;
