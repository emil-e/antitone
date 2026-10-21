//! Fixed-frame and shortlist diagnostics; results never enter proposal selection.
use super::*;
use serde_json::{Value, json};

fn side_summary(side: &Side) -> Value {
    let directional=side.directional.as_ref().map(|d|json!({
        "measured":d.measured,"score":d.aggregate_z,"blocks":d.blocks,
        "supporting_blocks":d.supporting_blocks,"stations":d.stations.len(),
        "contradictory_stations":d.contradictory_stations,
        "continuation_bins":d.continuation_bins,"structure_bins":d.structure_bins,"reversed_bins":d.reversed_bins,
    }));
    json!({"measured":side.measured(),"contradiction":side.contradiction,"directional":directional,"support":side.support,"pooled":side.pooled})
}

fn distance(a: &PictureGeometry, b: &PictureGeometry) -> f64 {
    if (a.angle - b.angle).abs() > 1e-9 {
        return f64::INFINITY;
    }
    a.sides
        .iter()
        .zip(b.sides)
        .map(|(a, b)| match (a, b) {
            (Some(a), Some(b)) => (*a - b).abs(),
            _ => f64::INFINITY,
        })
        .fold(0., f64::max)
}

fn assess(
    scene: &Scene,
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    template: &Template,
    proposed: [bool; 4],
) -> (Value, [Side; 4]) {
    let light = light::ExteriorLight::new(capture, roles);
    let evaluated = eligible_with(
        scene,
        capture,
        roles,
        frame,
        Some(template),
        &light,
        Mode::Joint,
    );
    let (eligibility, crop, sides) = match evaluated {
        Ok((crop, sides)) => (None, Some(crop), sides),
        Err(reason) => (
            Some(reason),
            None,
            std::array::from_fn(|i| {
                Mode::Joint.measure(capture, roles, frame, i, template.exterior[i])
            }),
        ),
    };
    let measured = sides.each_ref().map(|s| s.measured());
    let accepted = eligibility.is_none() && (0..4).all(|i| !proposed[i] || measured[i]);
    (
        json!({"geometry":frame,"proposed":proposed,"measured":measured,"valid_as_proposed_anchor_set":accepted,"eligibility_error":eligibility,"crop_if_eligible":crop,"sides":sides.iter().map(side_summary).collect::<Vec<_>>()}),
        sides,
    )
}

fn detail(capture: &Capture, roles: &[Role], sides: &[Side; 4], frame: &PictureGeometry) -> Value {
    json!({"sides":sides,"normal_profiles":(0..4).map(|side|directional::inspection::profiles(capture,roles,frame,side)).collect::<Vec<_>>()})
}

pub(in super::super) fn inspect(
    scene: &Scene,
    target: Target<'_>,
    roles: &[Role],
    material: &Report,
    joint: &Report,
    reference: &Reference,
    checkpoint: impl Fn() -> Result<(), String>,
) -> Result<Value, String> {
    let (Some(choice), Some(template)) = (&material.proposal, &joint.template) else {
        return Ok(json!({"unavailable":"material proposal or joint template absent"}));
    };
    checkpoint()?;
    let frame = &choice.physical;
    let (mut old, sides) = assess(
        scene,
        target.capture,
        roles,
        frame,
        template,
        choice.measured,
    );
    old["detail"] = detail(target.capture, roles, &sides, frame);
    let mut pose = target
        .baseline
        .picture_geometry
        .as_ref()
        .map_or(template.pose.center, |f| f.angle);
    if !template.pose.contains(pose) {
        pose = template.pose.center;
    }
    let prior = template.frame(pose);
    let (mut prior_evidence, prior_sides) =
        assess(scene, target.capture, roles, &prior, template, [true; 4]);
    prior_evidence["detail"] = detail(target.capture, roles, &prior_sides, &prior);
    let locations = ranked_locations(target.capture, roles, &target, template, pose, Mode::Joint);
    let positions:Vec<_>=locations.iter().enumerate().map(|(side,values)|{
        json!({"side":side,"locations":values,"material_bound_present":values.iter().any(|(bound,_)|(*bound-frame.sides[side].unwrap()).abs()<1e-9)})
    }).collect();
    let mut old_scores = vec![];
    for (i, proposed) in choice.measured.into_iter().enumerate() {
        if proposed {
            old_scores.push(
                locations[i]
                    .iter()
                    .find(|(bound, _)| (*bound - frame.sides[i].unwrap()).abs() < 1e-9)
                    .map(|(_, score)| *score),
            );
        }
    }
    let cached_material_rank = old_scores
        .iter()
        .copied()
        .collect::<Option<Vec<_>>>()
        .map(|scores| scores.iter().sum::<f64>() - template.displacement(frame));
    let (phases, search) = joint::hypotheses(
        &locations,
        template,
        reference,
        target.capture.area,
        pose,
        &checkpoint,
    )?;
    let stored = joint
        .joint_search
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| e.to_string())?;
    let replayed = serde_json::to_value(&search).map_err(|e| e.to_string())?;
    if let Some(stored) = &stored {
        assert_eq!(stored, &replayed, "passive shortlist reconstruction");
    }
    let mut shortlists = vec![];
    let mut best_right: Option<(f64, PictureGeometry, [bool; 4])> = None;
    for (phase, candidates) in phases.into_iter().enumerate() {
        let mut rows = vec![];
        let cutoff = candidates.last().map(|c| c.rank);
        for (index, candidate) in candidates.into_iter().enumerate() {
            checkpoint()?;
            let (mut value, sides) = assess(
                scene,
                target.capture,
                roles,
                &candidate.frame,
                template,
                candidate.proposed,
            );
            value["index"] = json!(index);
            value["cached_rank"] = json!(candidate.rank);
            value["distance_from_material"] = json!(distance(&candidate.frame, frame));
            value["exact_material_hypothesis"] = json!(
                distance(&candidate.frame, frame) < 1e-9 && candidate.proposed == choice.measured
            );
            if candidate.proposed[1] {
                let score = sides[1]
                    .directional
                    .as_ref()
                    .map_or(f64::NEG_INFINITY, |d| d.aggregate_z);
                if best_right.as_ref().is_none_or(|(best, _, _)| score > *best) {
                    best_right = Some((score, candidate.frame.clone(), candidate.proposed));
                }
            }
            rows.push(value);
        }
        shortlists.push(
            json!({"proposed_count":if phase==0 {4}else{3},"cutoff_rank":cutoff,"hypotheses":rows}),
        );
    }
    let best_right = best_right.map(|(score, frame, proposed)| {
        let (mut value, sides) = assess(scene, target.capture, roles, &frame, template, proposed);
        value["right_score"] = json!(score);
        value["detail"] = detail(target.capture, roles, &sides, &frame);
        value
    });
    checkpoint()?;
    Ok(
        json!({"material_frame":old,"prior_frame":prior_evidence,"candidate_locations":positions,"material_cached_rank":cached_material_rank,"original_shortlist_was_run":stored.is_some(),"search":replayed,"shortlists":shortlists,"best_shortlisted_right":best_right,"limits":"Passive remeasurement only. Fixed material frame is a comparator, not physical ground truth. No candidate positions, score/geometry gates, rankings or proposals changed. Full profile and shortlist evidence must not be used to tune this consumed photograph."}),
    )
}
