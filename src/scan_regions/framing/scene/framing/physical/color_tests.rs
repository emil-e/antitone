use super::super::tests::fixture;
use super::tests::{geometry, run};
use super::*;

// Deliberately varied sensor RGB, not literal stock profiles or an accepted-colour list.
pub(super) const BASES: [[u16; 3]; 6] = [
    [50000, 50000, 50000],
    [45000, 20000, 11000],
    [35000, 14000, 35000],
    [12000, 35000, 16000],
    [21000, 27000, 43000],
    [31000, 17000, 23000],
];

pub(super) fn coloured_roll(base: [u16; 3], blank_target: bool) -> (Vec<Capture>, Scene, Template) {
    let mut captures = vec![];
    for index in 0..4 {
        let (mut capture, _, _) = fixture([64, 48], [8., 56., 6., 42.], None, true);
        for y in 0..48 {
            for x in 0..64 {
                let picture = (8..56).contains(&x) && (6..42).contains(&y);
                capture.rgb[y * 64 + x] = if picture && !(blank_target && index == 3) {
                    base.map(|value| (f64::from(value) * 0.4).round() as u16)
                } else {
                    base
                };
            }
        }
        captures.push(capture);
    }
    let model = RollModel::fit(&captures).unwrap();
    let scene = Scene::fit(&captures, &model);
    let roles: Vec<_> = captures
        .iter()
        .map(|capture| scene.roles(capture))
        .collect();
    let frames: Vec<_> = (0..3)
        .map(|index| (index, geometry(captures[index].size, [8., 56., 6., 42.])))
        .collect();
    let template = Template::fit(&scene, &captures, &roles, &frames).unwrap();
    (captures, scene, template)
}

#[test]
fn roll_learned_base_colour_does_not_impose_orange_neutral_or_catalogue_hues() {
    for base in BASES {
        let (captures, scene, template) = coloured_roll(base, false);
        assert!(!template.donors.contains(&3));
        for exterior in template.exterior {
            let exterior = exterior.expect("all donor sides expose authored base");
            assert!(exterior.samples > 0);
            for (channel, value) in base.into_iter().enumerate() {
                assert!((exterior.mean[channel] - f64::from(value).log2()).abs() < 1e-12);
            }
        }
        let capture = &captures[3];
        let baseline = Areas {
            picture_geometry: Some(geometry(capture.size, [8., 56., 6., 42.])),
            picture_observed: [true; 4],
            ..Areas::default()
        };
        let (_, evidence, _) = fixture(capture.size, [8., 56., 6., 42.], None, true);
        let report = run(capture, &evidence, &scene, &baseline, template, 4. / 3.);
        let proposal = report.proposal.unwrap();
        assert_eq!(
            proposal.physical.sides,
            baseline.picture_geometry.unwrap().sides
        );
        assert_eq!(proposal.measured, [true; 4]);
    }
}

#[test]
fn matching_base_colour_without_picture_transitions_does_not_identify_a_frame() {
    for base in BASES {
        let (captures, scene, template) = coloured_roll(base, true);
        let capture = &captures[3];
        let (_, evidence, _) = fixture(capture.size, [8., 56., 6., 42.], None, true);
        let report = run(
            capture,
            &evidence,
            &scene,
            &Areas::default(),
            template,
            4. / 3.,
        );
        assert!(report.proposal.is_none());
    }
}
