use super::super::color_tests::coloured_roll;
use super::*;

#[test]
fn fractional_one_dimensional_smooth_ramps_do_not_become_directional_edges() {
    for base in [0.5, 20000., 131070.] {
        let (mut captures, scene, prior) = coloured_roll([base; 3], false);
        let capture = &mut captures[3];
        for phase in [0., 0.25, 0.5, 0.75] {
            for slope in [-0.002, 0.002] {
                for logarithmic in [false, true] {
                    for y in 0..48 {
                        for x in 0..64 {
                            let position = x as f64 + 0.5 - 56. - phase;
                            let transmission = if logarithmic {
                                (slope * position).exp2()
                            } else {
                                1. + slope * position
                            };
                            capture.rgb[y * 64 + x] = [(base as f64 * transmission) as f32; 3];
                        }
                    }
                    let frame = geometry(capture.size, [8., 56. + phase, 6., 42.]);
                    let side = directional::measure(
                        capture,
                        &scene.roles(capture),
                        &frame,
                        1,
                        prior.exterior[1],
                    );
                    let report = side.directional.as_ref().unwrap();
                    assert!(
                        report.stations.len() >= 12,
                        "control must have available profiles"
                    );
                    assert!(
                        !side.measured(),
                        "base={base}, phase={phase}, slope={slope}, log={logarithmic}: false edge"
                    );
                    assert!(
                        !side.contradiction,
                        "base={base}, phase={phase}, slope={slope}, log={logarithmic}: false contradiction"
                    );
                }
            }
        }
    }
}

#[test]
fn fractional_one_dimensional_localized_steps_remain_measurable() {
    for base in [0.5, 20000., 131070.] {
        let (mut captures, scene, prior) = coloured_roll([base; 3], false);
        let capture = &mut captures[3];
        for y in 0..48 {
            for x in 0..64 {
                capture.rgb[y * 64 + x] = [if x < 56 { base * 0.4 } else { base }; 3];
            }
        }
        let side = directional::measure(
            capture,
            &scene.roles(capture),
            &geometry(capture.size, [8., 56., 6., 42.]),
            1,
            prior.exterior[1],
        );
        let report = side.directional.as_ref().unwrap();
        assert!(side.measured(), "base={base}: localized step was lost");
        assert!(!side.contradiction);
        assert!(
            report
                .stations
                .iter()
                .all(|s| s.noise == 0. && s.tangent == 0.)
        );
        assert!(report.aggregate_z.is_finite());
    }
}
