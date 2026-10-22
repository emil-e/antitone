use super::*;

fn capture(rgb: Vec<[u16; 3]>) -> Capture {
    let evidence = Evidence {
        lines: crate::scan_regions::lines::SIDES
            .into_iter()
            .map(|s| (s.into(), vec![]))
            .collect(),
        segments: crate::scan_regions::lines::SIDES
            .into_iter()
            .map(|s| (s.into(), vec![]))
            .collect(),
    };
    Capture::new([12, 7, 320, 240].into(), [32, 24], rgb, evidence).unwrap()
}

fn roll() -> Vec<Capture> {
    (0..4)
        .map(|n| {
            capture(
                (0..24)
                    .flat_map(|y| {
                        (0..32).map(move |x| {
                            if !(2..30).contains(&x) || !(2..22).contains(&y) {
                                [100; 3]
                            } else if (13..26).contains(&x) {
                                [6000 + n * 500; 3]
                            } else {
                                [20000; 3]
                            }
                        })
                    })
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn holder_is_frozen_across_photos_instead_of_requiring_each_photo_to_be_dark() {
    let mut captures = roll();
    for y in 0..24 {
        for x in 0..32 {
            if x < 2 || y < 2 {
                captures[0].rgb[y * 32 + x] = [25000; 3];
            }
        }
    }
    let model = RollModel::fit(&captures).unwrap();
    let scene = Scene::fit(&captures, &model);
    assert!(scene.holder[0]);
    assert_eq!(scene.roles(&captures[0])[0], Role::Holder);
    assert!(!scene.holder[12 * 32 + 18]);
}

#[test]
fn a_transient_dark_scene_connected_to_the_border_does_not_move_the_holder() {
    let mut captures = roll();
    for y in 6..18 {
        for x in 0..12 {
            captures[0].rgb[y * 32 + x] = [100; 3];
        }
    }
    let model = RollModel::fit(&captures).unwrap();
    let scene = Scene::fit(&captures, &model);
    assert!(scene.holder[12 * 32]);
    assert!(!scene.holder[12 * 32 + 8]);
}

#[test]
fn no_observed_holder_does_not_invent_one_or_prevent_a_base_estimate() {
    let captures: Vec<_> = (0..4).map(|_| capture(vec![[20000; 3]; 32 * 24])).collect();
    let model = RollModel::fit(&captures).unwrap();
    let scene = Scene::fit(&captures, &model);
    assert!(scene.holder.iter().all(|&v| !v));
    assert!(scene.base.is_some());
}

#[test]
fn base_attenuation_and_ambiguous_or_clipped_colours_are_not_default_verified_film() {
    let captures = roll();
    let model = RollModel::fit(&captures).unwrap();
    let scene = Scene::fit(&captures, &model);
    let mut probe = capture(vec![[20000; 3]; 32 * 24]);
    let at = |x: usize| 12 * 32 + x;
    probe.rgb[at(5)] = [6000; 3];
    probe.rgb[at(6)] = [40000; 3];
    probe.rgb[at(7)] = [40000, 2000, 20000];
    probe.rgb[at(8)] = [65535; 3];
    let roles = scene.roles(&probe);
    assert_eq!(roles[at(4)], Role::BaseCompatible);
    assert_eq!(roles[at(5)], Role::AttenuatedCompatible);
    assert_eq!(roles[at(6)], Role::BrighterThanBase);
    assert_eq!(roles[at(7)], Role::Unknown);
    assert_eq!(roles[at(8)], Role::Unknown);
}
