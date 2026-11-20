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

#[cfg(test)]
mod tests;
