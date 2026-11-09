//! Local colour stability in native positives, not a certification of neutral material.
//! The Gaussian-gradient GI formulation follows Qian et al.'s executable reference.
use crate::Error;
use glam::DVec3;

const RADIUS: usize = 8;

pub(super) struct Selection {
    pub indices: Vec<usize>,
    pub mean: DVec3,
}

fn ranked_indices(scores: &[f64], eligible: &[bool]) -> Option<Vec<usize>> {
    let mut ranked: Vec<_> = scores
        .iter()
        .zip(eligible)
        .filter_map(|(score, ok)| ok.then_some(*score))
        .collect();
    if ranked.is_empty() {
        return None;
    }
    ranked.sort_unstable_by(f64::total_cmp);
    let quota = ((scores.len() as f64 * 0.001).floor() as usize)
        .max(1)
        .min(ranked.len());
    let threshold = ranked[quota - 1];
    Some(
        (0..scores.len())
            .filter(|&i| eligible[i] && scores[i] <= threshold)
            .collect(),
    )
}

fn box_mean(values: &[f64], [width, height]: [usize; 2]) -> Vec<f64> {
    let mut horizontal = vec![0.; values.len()];
    let mut output = vec![0.; values.len()];
    // Periodic support is only an implementation of the rolling sum: the complete
    // eight-cell dependency border is excluded from every selected pixel below.
    for y in 0..height {
        let mut sum: f64 = (0..7)
            .map(|j| values[y * width + (width + j - 3) % width])
            .sum();
        for x in 0..width {
            horizontal[y * width + x] = sum / 7.;
            sum +=
                values[y * width + (x + 4) % width] - values[y * width + (width + x - 3) % width];
        }
    }
    for x in 0..width {
        let mut sum: f64 = (0..7)
            .map(|j| horizontal[((height + j - 3) % height) * width + x])
            .sum();
        for y in 0..height {
            output[y * width + x] = sum / 7.;
            sum += horizontal[((y + 4) % height) * width + x]
                - horizontal[((height + y - 3) % height) * width + x];
        }
    }
    output
}

fn filter(
    values: &[f64],
    [width, height]: [usize; 2],
    kernel: &[f64; 5],
    vertical: bool,
) -> Vec<f64> {
    let mut output = vec![0.; values.len()];
    for y in 0..height {
        for x in 0..width {
            for (j, weight) in kernel.iter().enumerate() {
                let sx = if vertical {
                    x
                } else {
                    (x as isize + j as isize - 2).clamp(0, width as isize - 1) as usize
                };
                let sy = if vertical {
                    (y as isize + j as isize - 2).clamp(0, height as isize - 1) as usize
                } else {
                    y
                };
                output[y * width + x] += weight * values[sy * width + sx];
            }
        }
    }
    output
}

fn gradient(values: &[f64], size: [usize; 2]) -> Vec<f64> {
    let gaussian = std::array::from_fn(|j| {
        let x = j as f64 - 2.;
        (-x * x / 0.5).exp()
    });
    let derivative =
        std::array::from_fn(|j| -(j as f64 - 2.) * gaussian[j] / (std::f64::consts::PI * 0.25));
    let dx = filter(
        &filter(values, size, &derivative, false),
        size,
        &gaussian,
        true,
    );
    let dy = filter(
        &filter(values, size, &gaussian, false),
        size,
        &derivative,
        true,
    );
    dx.iter().zip(dy).map(|(x, y)| x.hypot(y)).collect()
}

pub(super) fn select(
    rgb: &[DVec3],
    valid: &[bool],
    size: [usize; 2],
    gain: f64,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Selection>, Error> {
    select_admitted(rgb, valid, valid, size, gain, cancelled)
}

pub(super) fn select_admitted(
    rgb: &[DVec3],
    valid: &[bool],
    admitted: &[bool],
    size: [usize; 2],
    gain: f64,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<Option<Selection>, Error> {
    let [width, height] = size;
    if cancelled() {
        return Ok(None);
    }
    if width <= 2 * RADIUS || height <= 2 * RADIUS {
        return Err(Error::UnusableSample(
            "picture is too small for GI neighbourhoods",
        ));
    }
    let mut integral = vec![0usize; (width + 1) * (height + 1)];
    for y in 0..height {
        if cancelled() {
            return Ok(None);
        }
        for x in 0..width {
            let i = y * width + x;
            let bad = !valid[i] || !rgb[i].is_finite() || rgb[i].min_element() <= 1e-8;
            integral[(y + 1) * (width + 1) + x + 1] = usize::from(bad)
                + integral[y * (width + 1) + x + 1]
                + integral[(y + 1) * (width + 1) + x]
                - integral[y * (width + 1) + x];
        }
    }
    let channels: [Vec<f64>; 3] = std::array::from_fn(|c| {
        rgb.iter()
            .zip(valid)
            .map(|(p, ok)| {
                if *ok && p.is_finite() && p.min_element() > 1e-8 {
                    p[c] * gain
                } else {
                    0.
                }
            })
            .collect()
    });
    let smooth = channels.each_ref().map(|v| box_mean(v, size));
    if cancelled() {
        return Ok(None);
    }
    let gradients = smooth.each_ref().map(|v| gradient(v, size));
    let logs: [Vec<f64>; 2] = [0, 2].map(|c| {
        (0..rgb.len())
            .map(|i| {
                let total = smooth[0][i] + smooth[1][i] + smooth[2][i];
                if smooth[c][i] > 0. && total > 0. {
                    (smooth[c][i] / total).ln()
                } else {
                    0.
                }
            })
            .collect()
    });
    if cancelled() {
        return Ok(None);
    }
    let derivatives = logs.each_ref().map(|v| gradient(v, size));
    let mut scores: Vec<_> = derivatives[0]
        .iter()
        .zip(&derivatives[1])
        .map(|(a, b)| a.hypot(*b))
        .collect();
    let mut eligible = vec![false; rgb.len()];
    for y in RADIUS..height - RADIUS {
        if cancelled() {
            return Ok(None);
        }
        for x in RADIUS..width - RADIUS {
            let i = y * width + x;
            let (x0, x1, y0, y1) = (x - RADIUS, x + RADIUS + 1, y - RADIUS, y + RADIUS + 1);
            let bad = integral[y1 * (width + 1) + x1] + integral[y0 * (width + 1) + x0]
                - integral[y0 * (width + 1) + x1]
                - integral[y1 * (width + 1) + x0];
            eligible[i] = bad == 0
                && rgb[i].element_sum() * gain > 0.0315
                && (0..3).any(|c| gradients[c][i] > 1e-4)
                && scores[i].is_finite();
        }
    }
    if !eligible.iter().any(|v| *v) {
        return Err(Error::UnusableSample("no valid, non-flat GI evidence"));
    }
    let maximum = scores
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(0., f64::max);
    for (score, ok) in scores.iter_mut().zip(&eligible) {
        if !ok {
            *score = maximum;
        }
    }
    let scores = box_mean(&scores, size);
    if cancelled() {
        return Ok(None);
    }
    // Admission restricts retained centres, not the native-positive neighbourhood
    // used to calculate their GI scores. Apply it before selecting the rank quota.
    for (eligible, admitted) in eligible.iter_mut().zip(admitted) {
        *eligible &= *admitted;
    }
    let indices = ranked_indices(&scores, &eligible)
        .ok_or(Error::UnusableSample("no admitted GI evidence"))?;
    let mean = indices.iter().map(|&i| rgb[i] / indices.len() as f64).sum();
    Ok((!cancelled()).then_some(Selection { indices, mean }))
}

#[cfg(test)]
mod tests;
