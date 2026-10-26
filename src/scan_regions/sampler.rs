//! Bounded camera-code geometry: gradient observations as line and segment hypotheses per side.
//! Strength and polarity are not material identity; nothing here develops, repairs or grades.
use super::lines::{Evidence, Line, SIDES, Surface};
use std::collections::BTreeMap;

const PI: f64 = std::f64::consts::PI;
/// Smallest resolvable 16-bit code step, the floor for gradient-relative comparisons.
const FLOOR: f64 = 1. / 65535.;

/// Positional nearest-rank quantile, matching a k-th-element selection over the observed values.
fn quantile(values: &mut [f32], q: f64) -> f32 {
    let k = (q * (values.len() - 1) as f64) as usize;
    values.select_nth_unstable_by(k, f32::total_cmp);
    values[k]
}

/// One side's observation geometry: which axis is crossed, where along it to look, and how densely.
#[derive(Clone, Copy)]
struct Pass {
    vertical: bool,
    span: [f64; 2],
    count: u32,
}

/// Fills `out` with the gradient sampled along a slanted line and returns the observed span.
/// All four bilinear corners must have measured gradients, so the derivative border is excluded.
fn gradient_along(
    out: &mut Vec<f32>,
    gradient: &[f32],
    size: [u32; 2],
    pass: Pass,
    position: f64,
    angle: f64,
) -> Option<[f64; 2]> {
    let Pass {
        vertical,
        span,
        count,
    } = pass;
    let [w, h] = size;
    let across = f64::from(if vertical { h } else { w });
    let center = (across - 1.) / 2.;
    let slope = (angle * PI / 180.).tan();
    let (width, height) = (f64::from(w), f64::from(h));
    out.clear();
    let mut observed = [0.; 2];
    for sample in 0..count {
        let fraction = span[0] + (span[1] - span[0]) * f64::from(sample) / f64::from(count - 1);
        let along = (across - 1.) * fraction;
        let cross = position + slope * (along - center);
        let (px, py) = if vertical {
            (cross, along)
        } else {
            (along, cross)
        };
        if px < 1. || py < 1. || px >= width - 2. || py >= height - 2. {
            continue;
        }
        let x = px.floor();
        let y = py.floor();
        let (dx, dy) = (px - x, py - y);
        if out.is_empty() {
            observed[0] = fraction;
        }
        observed[1] = fraction;
        let i = y as usize * w as usize + x as usize;
        // Bilinear weights are doubles and the observation narrows once, as the native observer did.
        let left = f64::from(gradient[i]) * (1. - dx) + f64::from(gradient[i + 1]) * dx;
        let right = f64::from(gradient[i + w as usize]) * (1. - dx)
            + f64::from(gradient[i + w as usize + 1]) * dx;
        out.push((left * (1. - dy) + right * dy) as f32);
    }
    (!out.is_empty()).then_some(observed)
}

/// Median camera code on the surface beyond a candidate line, sampled at whole pixels.
fn surface_along(
    rgb: &[[u16; 3]],
    size: [u32; 2],
    pass: Pass,
    position: f64,
    angle: f64,
) -> Option<[f64; 3]> {
    let Pass {
        vertical,
        span,
        count,
    } = pass;
    let [w, h] = size;
    let across = f64::from(if vertical { h } else { w });
    let center = (across - 1.) / 2.;
    let slope = (angle * PI / 180.).tan();
    let mut channels: [Vec<f32>; 3] = std::array::from_fn(|_| Vec::with_capacity(count as usize));
    for sample in 0..count {
        let fraction = span[0] + (span[1] - span[0]) * f64::from(sample) / f64::from(count - 1);
        let along = (across - 1.) * fraction;
        let cross = position + slope * (along - center);
        let (x, y) = if vertical {
            (cross.round(), along.round())
        } else {
            (along.round(), cross.round())
        };
        if x < 0. || y < 0. || x >= f64::from(w) || y >= f64::from(h) {
            continue;
        }
        let pixel = rgb[y as usize * w as usize + x as usize];
        for (channel, value) in channels.iter_mut().zip(pixel) {
            channel.push(f32::from(value) / 65535.);
        }
    }
    if channels[0].len() < (count / 2) as usize {
        return None;
    }
    Some(std::array::from_fn(|c| {
        f64::from(quantile(&mut channels[c], 0.5))
    }))
}

/// Camera-code change across a candidate line along its normal, and how consistently it repeats.
#[derive(Clone, Copy)]
struct Transition {
    mean_delta: [f64; 3],
    lower_quartile_projection: f64,
}

fn normal_transition(
    rgb: &[[u16; 3]],
    size: [u32; 2],
    pass: Pass,
    position: f64,
    angle: f64,
) -> Option<Transition> {
    let Pass {
        vertical,
        span,
        count,
    } = pass;
    let [w, h] = size;
    let slope = (angle * PI / 180.).tan();
    let scale = (1. + slope * slope).sqrt();
    let nx = if vertical { 1. } else { -slope } / scale;
    let ny = if vertical { -slope } else { 1. } / scale;
    let across = f64::from(if vertical { h } else { w });
    let sample = |px: f64, py: f64, channel: usize| {
        let x = px.floor();
        let y = py.floor();
        let (dx, dy) = (px - x, py - y);
        let i = y as usize * w as usize + x as usize;
        let code = |offset: usize| f64::from(rgb[i + offset][channel]);
        ((code(0) * (1. - dx) + code(1) * dx) * (1. - dy)
            + (code(w as usize) * (1. - dx) + code(w as usize + 1) * dx) * dy)
            / 65535.
    };
    let mut deltas = vec![];
    let mut mean = [0.; 3];
    for k in 0..count {
        let along =
            (across - 1.) * (span[0] + (span[1] - span[0]) * f64::from(k) / f64::from(count - 1));
        let cross = position + slope * (along - (across - 1.) / 2.);
        let (x, y) = if vertical {
            (cross, along)
        } else {
            (along, cross)
        };
        if x - nx.abs() < 0.
            || y - ny.abs() < 0.
            || x + nx.abs() >= f64::from(w - 1)
            || y + ny.abs() >= f64::from(h - 1)
        {
            continue;
        }
        let mut delta = [0.; 3];
        for channel in 0..3 {
            delta[channel] = sample(x + nx, y + ny, channel) - sample(x - nx, y - ny, channel);
            mean[channel] += delta[channel];
        }
        deltas.push(delta);
    }
    if deltas.len() < (count / 2) as usize {
        return None;
    }
    let mut magnitude = 0.;
    for value in &mut mean {
        *value /= deltas.len() as f64;
        magnitude += *value * *value;
    }
    magnitude = magnitude.sqrt();
    let mut projections: Vec<f32> = deltas
        .iter()
        .map(|delta| {
            let dot = (0..3).map(|c| delta[c] * mean[c]).sum::<f64>();
            (if magnitude > 0. { dot / magnitude } else { 0. }) as f32
        })
        .collect();
    Some(Transition {
        mean_delta: mean,
        lower_quartile_projection: f64::from(quantile(&mut projections, 0.25)),
    })
}

/// Two sides face each other when both repeat a decisive change in opposite colour directions.
fn opposed(a: Option<Transition>, b: Option<Transition>) -> bool {
    let (Some(a), Some(b)) = (a, b) else {
        return false;
    };
    if a.lower_quartile_projection <= 0. || b.lower_quartile_projection <= 0. {
        return false;
    }
    (0..3)
        .map(|c| a.mean_delta[c] * b.mean_delta[c])
        .sum::<f64>()
        < 0.
}

/// One side's candidate search: the positional window and which hypotheses to keep.
#[derive(Clone, Copy)]
struct Search<'a> {
    pass: Pass,
    first: i32,
    end: i32,
    maximum: usize,
    partial: bool,
    angles: &'a [f64],
    preserve_opposed: bool,
}

/// Ranked line hypotheses for one side within a positional window.
fn oriented_lines(
    gradient: &[f32],
    rgb: &[[u16; 3]],
    size: [u32; 2],
    search: &Search,
) -> Vec<Line> {
    let Search {
        pass,
        first,
        end,
        maximum,
        partial,
        angles: requested_angles,
        preserve_opposed,
    } = *search;
    let Pass {
        vertical,
        count: samples,
        ..
    } = pass;
    let mut scratch = Vec::with_capacity(samples as usize);
    let range = if vertical { size[0] } else { size[1] };
    let default: Vec<f64> = (-24..=24).map(|step| f64::from(step) * 0.25).collect();
    let angles = if requested_angles.is_empty() {
        &default
    } else {
        requested_angles
    };
    // A small-angle continuity baseline, not an implementation of LSD/EDLines or a format prior.
    let mut candidates: Vec<(f64, f64, f64)> = vec![];
    for position in first..end {
        let mut best: (f64, f64, f64) = (0., 0., 0.);
        for &angle in angles {
            if gradient_along(
                &mut scratch,
                gradient,
                size,
                pass,
                f64::from(position),
                angle,
            )
            .is_none_or(|_| scratch.len() < (samples / 2) as usize)
            {
                continue;
            }
            let visible = scratch.len() as f64 / f64::from(samples);
            let score = f64::from(quantile(&mut scratch, 0.25)) * visible;
            if score > best.0 {
                best = (score, angle, f64::from(position));
            }
        }
        if best.0 > 0. {
            candidates.push(best);
        }
    }
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut result: Vec<Line> = vec![];
    let mut accepted: Vec<f64> = vec![];
    let mut transitions: Vec<Option<Transition>> = vec![];
    for (score, angle, position) in candidates {
        let transition = normal_transition(rgb, size, pass, position, angle);
        let blocked = accepted.iter().enumerate().any(|(i, &accepted)| {
            let distance = (position - accepted).abs();
            distance < 6.
                && !(preserve_opposed && distance >= 2. && opposed(transition, transitions[i]))
        });
        if blocked {
            continue;
        }
        accepted.push(position);
        transitions.push(transition);
        let mut local = 0.;
        let mut shoulders = 0;
        for offset in [-6., 6.] {
            if gradient_along(&mut scratch, gradient, size, pass, position + offset, angle)
                .is_some_and(|_| scratch.len() >= (samples / 2) as usize)
            {
                local += f64::from(quantile(&mut scratch, 0.25));
                shoulders += 1;
            }
        }
        let outward = if first < (range / 2) as i32 { -6. } else { 6. };
        let observed = gradient_along(&mut scratch, gradient, size, pass, position, angle)
            .unwrap_or(pass.span);
        result.push(Line {
            midpoint_pixel: position,
            angle_degrees: angle,
            lower_quartile_gradient_score: score,
            local_gradient_background: (shoulders > 0).then(|| local / f64::from(shoulders)),
            angle_limit_reached: angle.abs() >= 6.,
            outside_surface: surface_along(rgb, size, pass, position + outward, angle)
                .map(|median_camera_rgb| Surface { median_camera_rgb }),
            span_fraction: observed,
            span_intervals: None,
            partial_span: partial,
            gradient_ratio: 0.,
            rank: 0,
        });
        if accepted.len() == maximum {
            break;
        }
    }
    result
}

/// Angles agreed by decisive lines on opposing sides, limiting the segment search to a pose.
fn segment_pose_guides(lines: &[Vec<Line>; 4]) -> Vec<f64> {
    let mut poses = vec![];
    for (side, rows) in lines.iter().enumerate() {
        for line in rows {
            let Some(background) = line.local_gradient_background else {
                continue;
            };
            if line.angle_limit_reached {
                continue;
            }
            if line.lower_quartile_gradient_score / background.max(FLOOR) >= 4. {
                poses.push((side, line.angle_degrees * if side < 2 { -1. } else { 1. }));
            }
        }
    }
    let mut guides = vec![];
    for (i, &(side, angle)) in poses.iter().enumerate() {
        for &(other, other_angle) in &poses[i + 1..] {
            if side != other && (angle - other_angle).abs() <= 1.5 {
                guides.push((angle + other_angle) / 2.);
            }
        }
    }
    guides.sort_by(f64::total_cmp);
    guides.dedup();
    guides
}

/// Narrow-window line hypotheses along an agreed pose, admitting opposed pairs of parallel lines.
fn segment_lines(
    gradient: &[f32],
    rgb: &[[u16; 3]],
    size: [u32; 2],
    axis: Pass,
    first: i32,
    end: i32,
    poses: &[f64],
) -> Vec<Line> {
    if poses.is_empty() {
        return vec![];
    }
    let angles: Vec<f64> = poses
        .iter()
        .map(|angle| if axis.vertical { -angle } else { *angle })
        .collect();
    let mut result = vec![];
    for window in 0..8 {
        let start = if window == 7 {
            0.70
        } else {
            0.05 + f64::from(window) * 0.10
        };
        result.extend(oriented_lines(
            gradient,
            rgb,
            size,
            &Search {
                pass: Pass {
                    span: [start, start + 0.25],
                    count: 48,
                    ..axis
                },
                first,
                end,
                maximum: 3,
                partial: true,
                angles: &angles,
                preserve_opposed: true,
            },
        ));
    }
    result
}

/// Camera-code gradient energy along each axis, excluding the derivative border.
fn gradients(rgb: &[[u16; 3]], size: [u32; 2]) -> (Vec<f32>, Vec<f32>) {
    let [w, h] = size;
    let mut gx = vec![0f32; (w * h) as usize];
    let mut gy = vec![0f32; (w * h) as usize];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = (y * w + x) as usize;
            let (left, right) = (rgb[i - 1], rgb[i + 1]);
            let (above, below) = (rgb[i - w as usize], rgb[i + w as usize]);
            let (mut dx, mut dy) = (0., 0.);
            for ((&left, right), (&above, below)) in
                left.iter().zip(right).zip(above.iter().zip(below))
            {
                let horizontal = (f64::from(right) - f64::from(left)) / 65535.;
                let vertical = (f64::from(below) - f64::from(above)) / 65535.;
                dx += horizontal * horizontal;
                dy += vertical * vertical;
            }
            gx[i] = dx.sqrt() as f32;
            gy[i] = dy.sqrt() as f32;
        }
    }
    (gx, gy)
}

/// Observes one bounded camera-code raster: up to five lines and three segments per side.
pub(super) fn observe(size: [u32; 2], rgb: &[[u16; 3]]) -> Evidence {
    let [w, h] = size;
    let (gx, gy) = gradients(rgb, size);
    let lines_pass = Pass {
        vertical: true,
        span: [0.1, 0.9],
        count: 96,
    };
    let rows_pass = Pass {
        vertical: false,
        ..lines_pass
    };
    let side = |gradient: &[f32], pass: Pass, first: i32, end: i32| {
        oriented_lines(
            gradient,
            rgb,
            size,
            &Search {
                pass,
                first,
                end,
                maximum: 5,
                partial: false,
                angles: &[],
                preserve_opposed: true,
            },
        )
    };
    let lines = [
        side(&gx, lines_pass, 1, (w / 4) as i32),
        side(&gx, lines_pass, (w * 3 / 4) as i32, w as i32 - 1),
        side(&gy, rows_pass, 1, (h / 4) as i32),
        side(&gy, rows_pass, (h * 3 / 4) as i32, h as i32 - 1),
    ];
    let poses = segment_pose_guides(&lines);
    let segments = [
        segment_lines(&gx, rgb, size, lines_pass, 1, (w / 4) as i32, &poses),
        segment_lines(
            &gx,
            rgb,
            size,
            lines_pass,
            (w * 3 / 4) as i32,
            w as i32 - 1,
            &poses,
        ),
        segment_lines(&gy, rgb, size, rows_pass, 1, (h / 4) as i32, &poses),
        segment_lines(
            &gy,
            rgb,
            size,
            rows_pass,
            (h * 3 / 4) as i32,
            h as i32 - 1,
            &poses,
        ),
    ];
    let sides = |rows: [Vec<Line>; 4]| -> BTreeMap<String, Vec<Line>> {
        SIDES
            .iter()
            .copied()
            .zip(rows)
            .map(|(side, lines)| (side.to_owned(), lines))
            .collect()
    };
    Evidence {
        lines: sides(lines),
        segments: sides(segments),
    }
}
