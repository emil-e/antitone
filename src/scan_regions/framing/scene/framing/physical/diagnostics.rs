//! Passive normal profiles, fixed geometries and offsets; never detection decisions.
use super::*;
use serde_json::{Value, json};

fn material(exterior: Option<Exterior>, rgb: [u16; 3]) -> &'static str {
    match exterior.map(|model| model.classify(rgb)) {
        Some(Material::Base) => "Base",
        Some(Material::Photo) => "Photo",
        Some(Material::Other) => "Other",
        None => "UnavailableReference",
    }
}

fn strip(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    side: usize,
    point: [f64; 2],
    sign: f64,
    exterior: Option<Exterior>,
) -> Vec<Value> {
    let normal = perimeter::normal(frame.angle, side);
    let mut samples = vec![];
    for distance in [0.5, 1.5, 2.5] {
        let p = std::array::from_fn(|axis| point[axis] + sign * distance * normal[axis]);
        let value = role_at(capture, roles, p);
        samples.push(json!({
            "distance": distance,
            "point": p,
            "sample": value,
            "material": value.map(|(_, rgb)| material(exterior, rgb)),
            "inside_companions": in_hypothesis(p, frame, Some(side)),
        }));
        let Some((role, _)) = value else {
            break;
        };
        if !matches!(role, Role::BaseCompatible | Role::AttenuatedCompatible)
            || (sign > 0. && !in_hypothesis(p, frame, Some(side)))
        {
            break;
        }
    }
    samples
}

fn side(
    capture: &Capture,
    roles: &[Role],
    frame: &PictureGeometry,
    index: usize,
    exterior: Option<Exterior>,
) -> Value {
    let measured = Mode::Joint.measure(capture, roles, frame, index, exterior);
    let profiles: Vec<_> = (0..64)
        .map(|station| {
            let p = perimeter::point(frame, index, (station as f64 + 0.5) / 64.);
            let normal = perimeter::normal(frame.angle, index);
            let context: Vec<_> = (3..9)
                .map(|step| {
                    let distance = step as f64 + 0.5;
                    let point = std::array::from_fn(|axis| p[axis] - distance * normal[axis]);
                    let sample = role_at(capture, roles, point);
                    json!({
                        "distance": distance,
                        "point": point,
                        "sample": sample,
                        "material": sample.map(|(_, rgb)| material(exterior, rgb)),
                    })
                })
                .collect();
            json!({
                "station": station,
                "point": p,
                "point_available": role_at(capture, roles, p).is_some(),
                "farther_outward_context_only": context,
                "inside": strip(capture, roles, frame, index, p, 1., exterior),
                "outside": strip(capture, roles, frame, index, p, -1., exterior),
            })
        })
        .collect();
    json!({"side": index, "reference": exterior, "evidence": measured, "profiles": profiles})
}

pub(in super::super) fn inspect_joint(
    scene: &Scene,
    target: Target<'_>,
    roles: &[Role],
    report: &Report,
) -> Value {
    let bootstrap = scene.base.as_ref().map(|base| Exterior {
        mean: base.mean,
        spread: base.spread,
        samples: 0,
    });
    let exterior: [Option<Exterior>; 4] = std::array::from_fn(|side| {
        report
            .template
            .as_ref()
            .and_then(|t| t.exterior[side])
            .or(bootstrap)
    });
    let baseline = observed_geometry(target.baseline);
    let pose = baseline.as_ref().map_or_else(
        || report.template.as_ref().map_or(0., |t| t.pose.center),
        |frame| frame.angle,
    );
    let mut frames = vec![];
    if let Some(frame) = &baseline {
        frames.push(("baseline", frame.clone()));
    }
    let mut options = None;
    if let Some(template) = &report.template {
        let pose = if template.pose.contains(pose) {
            pose
        } else {
            template.pose.center
        };
        frames.push(("prior", template.frame(pose)));
        options = Some(candidates_with(
            target.capture,
            roles,
            &target,
            template,
            pose,
            Mode::Joint,
        ));
    }
    let light = light::ExteriorLight::new(target.capture, roles);
    let mut details = vec![];
    for (name, frame) in frames {
        if !frame.sides.iter().all(Option::is_some) {
            continue;
        }
        let mut profiles = vec![];
        let mut offsets = vec![];
        for (index, reference) in exterior.into_iter().enumerate() {
            profiles.push(side(target.capture, roles, &frame, index, reference));
            let mut samples = vec![];
            for offset in [-8., -4., -2., -1., 0., 1., 2., 4., 8.] {
                let mut trial = frame.clone();
                trial.sides[index] = trial.sides[index].map(|bound| bound + offset);
                let evidence = Mode::Joint.measure(target.capture, roles, &trial, index, reference);
                samples.push(json!({"offset": offset, "evidence": evidence}));
            }
            offsets.push(json!({"side": index, "samples": samples}));
        }
        details.push(json!({
            "name": name,
            "geometry": frame,
            "template_compatible": report.template.as_ref().map(|template| template.compatible(&frame)),
            "known_exterior_light_intersects": light.intersects(&frame),
            "sides": profiles,
            "coarse_offset_evidence": offsets,
        }));
    }
    json!({
        "frames": details,
        "candidate_options": options,
        "baseline_crop_result": auto_crop::propose(target.capture.area, target.baseline).map_err(|error| error.to_string()),
        "limits": "Passive fixed-geometry/offset remeasurement, not proposals or precision refinement. Quiet/unavailable exterior is not a physical contradiction.",
    })
}

#[test]
fn passive_profiles_distinguish_source_unavailable_from_observed_base() {
    let (capture, evidence, scene) =
        super::super::tests::fixture([64, 48], [0., 56., 6., 42.], None, true);
    let baseline = Areas {
        picture_geometry: Some(super::tests::geometry(capture.size, [0., 56., 6., 42.])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    let report = super::tests::run(
        &capture,
        &evidence,
        &scene,
        &baseline,
        super::tests::template(capture.size, [0., 56., 6., 42.]),
        56. / 36.,
    );
    let data = inspect_joint(
        &scene,
        Target {
            capture: &capture,
            evidence: &evidence,
            baseline: &baseline,
            lines: [None, None, None, None],
        },
        &scene.roles(&capture),
        &report,
    );
    let profiles = &data["frames"][0]["sides"];
    assert!(profiles[0]["profiles"][0]["outside"][0]["sample"].is_null());
    assert_eq!(profiles[1]["profiles"][0]["outside"][0]["material"], "Base");
}
