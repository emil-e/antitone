use super::*;
use crate::{
    project::Development,
    spatial_metadata::{Boundary, CommonSize, Group, Input, Region, SourceRegions},
};

fn native_observation(
    scene: &Scene,
    capture: &Capture,
    roles: &[Role],
    baseline: &Areas,
    selected: [Option<Line>; 4],
    checkpoint: &impl Fn() -> Result<(), String>,
) -> Result<(Option<PictureGeometry>, [Option<Line>; 4]), String> {
    if let Some(frame) = physical::observe_native(scene, capture, roles, baseline) {
        return Ok((Some(frame), selected));
    }
    // A pooled position prior can suppress a legitimate partial family when captures
    // are translated differently. Reuse bounded existing native families, not a new
    // pixel/pose search. Every supplied side still needs completed-span qualification.
    for family in capture.group.families.iter().take(16) {
        checkpoint()?;
        let lines: [Option<Line>; 4] =
            std::array::from_fn(|side| capture.selected_line(family, side).cloned());
        let angles: Vec<_> = lines
            .iter()
            .enumerate()
            .filter_map(|(side, line)| line.as_ref().map(|line| line.physical_angle(side)))
            .collect();
        if angles.is_empty() {
            continue;
        }
        let angle = angles.iter().sum::<f64>() / angles.len() as f64;
        let frame = PictureGeometry {
            size: capture.size,
            angle,
            sides: std::array::from_fn(|side| {
                lines[side]
                    .as_ref()
                    .and_then(|line| physical::rigid::native_bound(line, side, capture.size, angle))
            }),
        };
        let areas = Areas {
            picture_observed: frame.sides.map(|side| side.is_some()),
            picture_geometry: Some(frame),
            ..Areas::default()
        };
        if let Some(frame) = physical::observe_native(scene, capture, roles, &areas) {
            return Ok((Some(frame), lines));
        }
    }
    Ok((None, selected))
}

fn film_limits(capture: &Capture, roles: &[Role]) -> Option<Region> {
    let mut region = None;
    for side in 0..4 {
        let mut candidates: Vec<_> = capture.group.lines[side].iter().collect();
        candidates.sort_by(|a, b| {
            (b.gradient_ratio * b.fraction()).total_cmp(&(a.gradient_ratio * a.fraction()))
        });
        for line in candidates {
            let angle = line.physical_angle(side);
            let Some(bound) = physical::rigid::native_bound(line, side, capture.size, angle) else {
                continue;
            };
            let camera = [
                [0., 0.],
                [f64::from(capture.size[0]), 0.],
                capture.size.map(f64::from),
                [0., f64::from(capture.size[1])],
            ]
            .map(|p| project(p, angle));
            let mut frame = PictureGeometry {
                size: capture.size,
                angle,
                sides: std::array::from_fn(|s| {
                    let values = camera.iter().map(|p| p[usize::from(s >= 2)]);
                    Some(if s.is_multiple_of(2) {
                        values.fold(f64::INFINITY, f64::min)
                    } else {
                        values.fold(f64::NEG_INFINITY, f64::max)
                    })
                }),
            };
            frame.sides[side] = Some(bound);
            if measure(capture, roles, &frame, side, line).kind() != Kind::FilmLimit {
                continue;
            }
            frame.sides = std::array::from_fn(|s| (s == side).then_some(bound));
            let Ok(measured) = Region::from_picture(&frame, capture.area, [true; 4]) else {
                continue;
            };
            let output = region.get_or_insert_with(|| Region {
                angle: measured.angle,
                skew: measured.skew,
                boundaries: std::array::from_fn(|_| Boundary::Unknown),
            });
            output.boundaries[side] = measured.boundaries[side].clone();
            break;
        }
    }
    region.filter(|region| region.validate().is_ok())
}

pub(in super::super::super) fn run(
    captures: &[Capture],
    evidence: &[Evidence],
    inputs: Vec<Input>,
    development: Development,
    code_scale: f64,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Group, String> {
    checkpoint()?;
    let model = RollModel::fit(captures).map_err(|e| e.to_string())?;
    let scene = Scene::fit(captures, &model);
    let prepared = Prepared::new(&scene, captures);
    let mut baselines = vec![];
    let mut source_lines = vec![];
    let mut sources = vec![];
    for (index, (capture, input)) in captures.iter().zip(inputs).enumerate() {
        checkpoint()?;
        let mut baseline = model
            .areas(index, capture, capture.size, &capture.rgb)
            .map_err(|e| e.to_string())?;
        let (native, lines) = native_observation(
            &scene,
            capture,
            &prepared.roles[index],
            &baseline,
            selected_lines(&model, index, capture),
            &checkpoint,
        )?;
        let observation = native
            .as_ref()
            .map(|frame| {
                let mut region = Region::from_picture(
                    frame,
                    capture.area,
                    frame.sides.map(|side| side.is_some()),
                )?;
                for (side, censored) in
                    physical::camera_censoring(capture, &prepared.roles[index], frame)
                        .into_iter()
                        .enumerate()
                {
                    if censored {
                        region.boundaries[side] = Boundary::CameraClipped;
                    }
                }
                Ok::<_, Error>(region)
            })
            .transpose()
            .map_err(|e| e.to_string())?;
        if let Some(frame) = native {
            baseline.picture_observed = frame.sides.map(|side| side.is_some());
            baseline.picture_weak = [false; 4];
            baseline.picture_geometry = Some(frame);
        }
        let holder = PictureGeometry {
            size: capture.size,
            angle: 0.,
            sides: holder_limits(&scene, 0.),
        };
        let holder_opening = holder
            .sides
            .iter()
            .any(Option::is_some)
            .then(|| Region::from_picture(&holder, capture.area, [false; 4]))
            .transpose()
            .map_err(|e| e.to_string())?;
        sources.push(SourceRegions {
            input,
            picture: observation.clone(),
            observation,
            film: film_limits(capture, &prepared.roles[index]),
            holder_opening,
            material: baseline.material.clone(),
        });
        baselines.push(baseline);
        source_lines.push(lines);
    }
    let common_size = CommonSize::fit(&sources);
    if let [Some(width), Some(height)] = common_size.extent {
        let calibration = physical::rigid::group_model(captures, [width, height], &checkpoint)?;
        for (index, capture) in captures.iter().enumerate() {
            checkpoint()?;
            let result = physical::rigid::run_group(
                &scene,
                Target {
                    capture,
                    evidence: &evidence[index],
                    baseline: &baselines[index],
                    lines: source_lines[index].clone(),
                },
                &prepared.roles[index],
                &calibration,
                &checkpoint,
            )?;
            // Failure to locate a complete frame need not erase independently qualified
            // camera-limited coverage. A distinct competing-frame ambiguity still abstains.
            let visible_only = if result.ambiguous() {
                None
            } else {
                sources[index].picture.clone().filter(|region| {
                    region
                        .boundaries
                        .iter()
                        .any(|boundary| matches!(boundary, Boundary::CameraClipped))
                })
            };
            sources[index].picture = result
                .proposal
                .map(|choice| Region::from_picture(&choice.physical, capture.area, choice.measured))
                .transpose()
                .map_err(|e| e.to_string())?
                .or(visible_only);
        }
    }
    checkpoint()?;
    Ok(Group {
        development,
        code_scale,
        sources,
        common_size,
    })
}
