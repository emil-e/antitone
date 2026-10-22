use super::*;

#[derive(Serialize)]
pub(super) struct Donor {
    index: usize,
    geometry: Option<PictureGeometry>,
    source_size: Option<[f64; 2]>,
    rejection: Option<&'static str>,
}

pub(super) fn check_donor(
    scene: &Scene,
    capture: &Capture,
    roles: &[Role],
    areas: &Areas,
    template: &Template,
) -> Result<[f64; 2], &'static str> {
    let frame = observed_geometry(areas)
        .filter(|frame| frame.sides.iter().all(Option::is_some))
        .ok_or("incomplete_or_weak_donor")?;
    let quad = frame.frame(capture.area).ok_or("invalid_donor_geometry")?;
    if quad.iter().any(|p| {
        p[0] < f64::from(capture.area.x)
            || p[1] < f64::from(capture.area.y)
            || p[0] > f64::from(capture.area.x + capture.area.width)
            || p[1] > f64::from(capture.area.y + capture.area.height)
    }) {
        return Err("camera_clipped_donor");
    }
    let light = light::ExteriorLight::new(capture, roles);
    let (_, sides) = eligible_with(
        scene,
        capture,
        roles,
        &frame,
        Some(template),
        &light,
        Mode::Joint,
    )?;
    if !sides.iter().all(Side::measured) {
        return Err("not_four_current_measurements");
    }
    Ok([
        (quad[1][0] - quad[0][0]).hypot(quad[1][1] - quad[0][1]),
        (quad[3][0] - quad[0][0]).hypot(quad[3][1] - quad[0][1]),
    ])
}

fn remap(ids: &mut [usize], peers: &[usize]) {
    for id in ids {
        *id = peers[*id];
    }
}

fn remap_reference(reference: &mut Reference, peers: &[usize]) {
    remap(&mut reference.eligible, peers);
    remap(&mut reference.donors, peers);
    remap(&mut reference.rejected, peers);
    if let Some(shape) = &mut reference.shape {
        remap(&mut shape.donors, peers);
        remap(&mut shape.rejected, peers);
    }
}

#[cfg(test)]
pub(super) fn fit(
    captures: &[Capture],
    excluded_targets: &[usize],
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<CalibrationReport, String> {
    if excluded_targets.is_empty() {
        return Err("rigid calibration needs declared excluded targets".into());
    }
    fit_impl(captures, excluded_targets, checkpoint)
}

pub(super) fn fit_group(
    captures: &[Capture],
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<CalibrationReport, String> {
    fit_impl(captures, &[], checkpoint)
}

fn fit_impl(
    captures: &[Capture],
    excluded_targets: &[usize],
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<CalibrationReport, String> {
    let mut excluded = excluded_targets.to_vec();
    excluded.sort_unstable();
    excluded.dedup();
    if excluded.iter().any(|&id| id >= captures.len()) {
        return Err("rigid calibration needs declared excluded targets".into());
    }
    let peer_ids: Vec<_> = (0..captures.len())
        .filter(|id| !excluded.contains(id))
        .collect();
    if peer_ids.len() < 3 {
        return Ok(CalibrationReport {
            model: None,
            reference: Reference {
                ratio: None,
                interval: None,
                shape: None,
                eligible: vec![],
                donors: vec![],
                rejected: vec![],
                error: Some("fewer than three independent captures".into()),
            },
            donors: vec![],
            unavailable: Some("insufficient_complete_peers"),
        });
    }
    checkpoint()?;
    // Exclusion precedes even native family selection and pooled appearance. Filtering
    // only the final donor rows would still let target cues influence the learned size.
    let peers: Vec<_> = peer_ids.iter().map(|&id| &captures[id]).collect();
    let model = RollModel::fit_refs(&peers).map_err(|error| error.to_string())?;
    let scene = Scene::fit_refs(&peers, &model);
    let prepared = Prepared::new_refs(&scene, &peers);
    let mut reference = reference_refs(&scene, &peers, &model, &prepared, None, &checkpoint)?;
    let mut frames = vec![];
    for &id in &reference.donors {
        checkpoint()?;
        let capture = peers[id];
        let areas = model
            .areas(id, capture, capture.size, &capture.rgb)
            .map_err(|error| error.to_string())?;
        if let Some(frame) =
            observed_geometry(&areas).filter(|frame| frame.sides.iter().all(Option::is_some))
        {
            frames.push((id, frame));
        }
    }
    let template = Template::fit_refs(&scene, &peers, &prepared.roles, &frames);
    remap_reference(&mut reference, &peer_ids);
    let Some(mut template) = template else {
        return Ok(CalibrationReport {
            model: None,
            reference,
            donors: vec![],
            unavailable: Some("insufficient_complete_peers"),
        });
    };
    let mut donors = vec![];
    for &local_id in &template.donors {
        checkpoint()?;
        let index = peer_ids[local_id];
        if excluded.contains(&index) {
            return Err("excluded target reached rigid donor admission".into());
        }
        let capture = peers[local_id];
        let areas = model
            .areas(local_id, capture, capture.size, &capture.rgb)
            .map_err(|error| error.to_string())?;
        let size = check_donor(
            &scene,
            capture,
            &prepared.roles[local_id],
            &areas,
            &template,
        );
        donors.push(Donor {
            index,
            geometry: observed_geometry(&areas),
            source_size: size.as_ref().ok().copied(),
            rejection: size.err(),
        });
    }
    let accepted: Vec<_> = donors
        .iter()
        .filter(|donor| donor.rejection.is_none())
        .collect();
    if accepted.len() < 3 {
        return Ok(CalibrationReport {
            model: None,
            reference,
            donors,
            unavailable: Some("insufficient_current_four_size_donors"),
        });
    }
    let source_size = std::array::from_fn(|axis| {
        median(
            accepted
                .iter()
                .map(|donor| donor.source_size.unwrap()[axis]),
        )
    });
    remap(&mut template.donors, &peer_ids);
    remap(&mut template.rejected, &peer_ids);
    let model = Calibration {
        source_size,
        donors: accepted.iter().map(|donor| donor.index).collect(),
        excluded_targets: excluded,
        template: Some(template),
    };
    Ok(CalibrationReport {
        model: Some(model),
        reference,
        donors,
        unavailable: None,
    })
}
