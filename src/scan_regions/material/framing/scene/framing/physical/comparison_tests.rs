use super::super::tests::fixture;
use super::tests::{reference, template};
use super::*;

#[test]
fn fixed_frame_profiles_match_the_known_step_and_side_measurement() {
    let (capture, evidence, scene) = fixture([64, 48], [8., 56., 6., 42.], None, false);
    let prior = template(capture.size, [8., 56., 6., 42.]);
    let baseline = Areas::default();
    let target = || Target {
        capture: &capture,
        evidence: &evidence,
        baseline: &baseline,
        lines: std::array::from_fn(|_| None),
    };
    let roles = scene.roles(&capture);
    let reference = reference(4. / 3.);
    let material = evaluate_with(
        &scene,
        target(),
        &roles,
        &reference,
        Some(prior.clone()),
        || Ok(()),
        Mode::Material,
    )
    .unwrap();
    let joint = evaluate_with(
        &scene,
        target(),
        &roles,
        &reference,
        Some(prior),
        || Ok(()),
        Mode::Joint,
    )
    .unwrap();
    let data = comparison::inspect(
        &scene,
        target(),
        &roles,
        &material,
        &joint,
        &reference,
        || Ok(()),
    )
    .unwrap();
    assert_eq!(data["material_frame"]["valid_as_proposed_anchor_set"], true);
    let detail = &data["material_frame"]["detail"];
    for side in 0..4 {
        for station in detail["sides"][side]["directional"]["stations"]
            .as_array()
            .unwrap()
        {
            let index = station["index"].as_u64().unwrap() as usize;
            assert!(
                (station["contrast"].as_f64().unwrap() - (20000f64 / 6000.).log2()).abs() < 1e-12
            );
            assert_eq!(
                station["contrast"],
                detail["normal_profiles"][side]["stations"][index]["gradients"][3]["contrast"]
            );
        }
    }
}
