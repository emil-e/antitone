//! Fit fixed dimensions to existing native observations; residuals do not establish identity.
use super::*;

#[derive(Serialize)]
struct Observation {
    side: usize,
    angle: f64,
    points: Vec<([f64; 2], f64)>,
}

#[derive(Serialize)]
struct Fit {
    geometry: PictureGeometry,
    measured: [bool; 4],
    observations: Vec<Observation>,
    side_rms: [Option<f64>; 4],
    rms: f64,
    angle_interval: [f64; 2],
}

#[derive(Default, Serialize)]
pub(super) struct Report {
    pub(super) proposal: Option<Choice>,
    fit: Option<Fit>,
    error: Option<String>,
    failed_anchors: Vec<usize>,
}

fn observations(target: &Target<'_>) -> Result<Vec<Observation>, String> {
    let frame = observed_geometry(target.baseline).ok_or("no observed native geometry")?;
    let mask = frame.sides.map(|value| value.is_some());
    if mask.iter().filter(|&&v| v).count() < 3 {
        return Err("fewer than three pre-existing measured native sides".into());
    }
    let mut result = vec![];
    for (side, &measured) in mask.iter().enumerate() {
        if !measured {
            continue;
        }
        let line = target.lines[side]
            .as_ref()
            .ok_or("selected native line unavailable")?;
        if line.angle_limit_reached || !line.angle_degrees.is_finite() {
            return Err("native angle unavailable".into());
        }
        let along_axis = usize::from(side < 2);
        let mut points = vec![];
        for [lo, hi] in line.spans() {
            if !lo.is_finite() || !hi.is_finite() || hi <= lo {
                continue;
            }
            // Equal total weight per side. Within a span these weights integrate
            // straight-line quadratic residuals without filling observed gaps.
            for (fraction, weight) in [(lo, 1.), ((lo + hi) * 0.5, 4.), (hi, 1.)] {
                let along = f64::from(target.capture.size[along_axis] - 1) * fraction + 0.5;
                let cross = line.cross(side, target.capture.size, along);
                let point = if side < 2 {
                    [cross, along]
                } else {
                    [along, cross]
                };
                if point
                    .iter()
                    .zip(target.capture.size)
                    .all(|(&p, n)| p.is_finite() && p >= 0.5 && p <= f64::from(n) - 0.5)
                {
                    points.push((point, weight * (hi - lo) / 6.));
                }
            }
        }
        let total = points.iter().map(|(_, weight)| weight).sum::<f64>();
        if total <= 0. {
            return Err("native side has no available observed span".into());
        }
        for (_, weight) in &mut points {
            *weight /= total;
        }
        result.push(Observation {
            side,
            angle: line.physical_angle(side),
            points,
        });
    }
    Ok(result)
}

fn at_pose(
    capture: &Capture,
    source_size: [f64; 2],
    data: &[Observation],
    pose: f64,
) -> (f64, PictureGeometry, [Option<f64>; 4]) {
    let scale = [
        f64::from(capture.area.width) / f64::from(capture.size[0]),
        f64::from(capture.area.height) / f64::from(capture.size[1]),
    ];
    let scales = model::axis_scales(pose, scale);
    let extent: [f64; 2] = std::array::from_fn(|axis| source_size[axis] / scales[axis]);
    let mut center = [0.; 2];
    let mut count = [0; 2];
    for observation in data {
        let axis = usize::from(observation.side >= 2);
        let sign = if observation.side.is_multiple_of(2) {
            -1.
        } else {
            1.
        };
        center[axis] += observation
            .points
            .iter()
            .map(|(p, weight)| weight * project(*p, pose)[axis])
            .sum::<f64>()
            - sign * extent[axis] * 0.5;
        count[axis] += 1;
    }
    for axis in 0..2 {
        center[axis] /= count[axis] as f64;
    }
    let frame = PictureGeometry {
        size: capture.size,
        angle: pose,
        sides: std::array::from_fn(|side| {
            Some(
                center[usize::from(side >= 2)]
                    + if side.is_multiple_of(2) { -0.5 } else { 0.5 }
                        * extent[usize::from(side >= 2)],
            )
        }),
    };
    let mut residual = [None; 4];
    let mut cost = 0.;
    for observation in data {
        let axis = usize::from(observation.side >= 2);
        let squared = observation
            .points
            .iter()
            .map(|(p, weight)| {
                weight * (project(*p, pose)[axis] - frame.sides[observation.side].unwrap()).powi(2)
            })
            .sum::<f64>();
        residual[observation.side] = Some(squared.sqrt());
        cost += squared;
    }
    (cost, frame, residual)
}

fn solve(capture: &Capture, source_size: [f64; 2], data: Vec<Observation>) -> Fit {
    let low = data.iter().map(|v| v.angle).fold(f64::INFINITY, f64::min);
    let high = data
        .iter()
        .map(|v| v.angle)
        .fold(f64::NEG_INFINITY, f64::max);
    let (mut left, mut right) = (low, high);
    let ratio = (5_f64.sqrt() - 1.) * 0.5;
    let mut x = right - ratio * (right - left);
    let mut y = left + ratio * (right - left);
    let mut fx = at_pose(capture, source_size, &data, x).0;
    let mut fy = at_pose(capture, source_size, &data, y).0;
    // Native directions bound this local pose fit; it is not a new discovery grid
    // or a calibrated angular confidence interval.
    for _ in 0..48 {
        if fx <= fy {
            right = y;
            y = x;
            fy = fx;
            x = right - ratio * (right - left);
            fx = at_pose(capture, source_size, &data, x).0;
        } else {
            left = x;
            x = y;
            fx = fy;
            y = left + ratio * (right - left);
            fy = at_pose(capture, source_size, &data, y).0;
        }
    }
    let pose = [low, high, x, y]
        .into_iter()
        .min_by(|a, b| {
            at_pose(capture, source_size, &data, *a)
                .0
                .total_cmp(&at_pose(capture, source_size, &data, *b).0)
        })
        .unwrap();
    let (cost, geometry, side_rms) = at_pose(capture, source_size, &data, pose);
    Fit {
        geometry,
        measured: std::array::from_fn(|side| data.iter().any(|v| v.side == side)),
        rms: (cost / data.len() as f64).sqrt(),
        observations: data,
        side_rms,
        angle_interval: [low, high],
    }
}

pub(super) fn evaluate(
    scene: &Scene,
    target: &Target<'_>,
    roles: &[Role],
    calibration: &Calibration,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Report, String> {
    checkpoint()?;
    let mut report = Report::default();
    let data = match observations(target) {
        Ok(data) => data,
        Err(error) => {
            report.error = Some(error);
            return Ok(report);
        }
    };
    let fit = solve(target.capture, calibration.source_size, data);
    let validation = validation_template(target.capture, calibration, &fit.geometry);
    let light = light::ExteriorLight::new(target.capture, roles);
    match eligible_with(
        scene,
        target.capture,
        roles,
        &fit.geometry,
        validation.as_ref(),
        &light,
        Mode::Joint,
    ) {
        Ok((crop, sides)) => {
            report.failed_anchors = (0..4)
                .filter(|&side| fit.measured[side] && !sides[side].measured())
                .collect();
            if report.failed_anchors.is_empty() {
                report.proposal = Some(make_choice(
                    fit.geometry.clone(),
                    crop,
                    &sides,
                    fit.measured,
                    None,
                    "rigid_joint_native",
                    target.capture,
                ));
            } else {
                report.error = Some("jointly fitted observations did not all requalify".into());
            }
        }
        Err(error) => report.error = Some(error.into()),
    }
    report.fit = Some(fit);
    checkpoint()?;
    Ok(report)
}

#[cfg(test)]
mod tests;
