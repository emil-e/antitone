use super::super::{lines::Group, opening::Shared, rectangles::project};
use super::*;

fn capture(scene: usize, angle: f64) -> Capture {
    let size = [192, 128];
    let mut rgb = vec![[10500; 3]; 192 * 128];
    for y in 0..128 {
        for x in 0..192 {
            let [u, v] = project([x as f64 + 0.5 - 96., y as f64 + 0.5 - 64.], angle);
            let [u, v] = [u + 96., v + 64.];
            let dx = (u - 96.).abs() - 58.;
            let dy = (v - 64.).abs() - 42.;
            let outside = dx.max(0.).hypot(dy.max(0.)) > 12. || v >= 117. - ((x / 7) % 3) as f64;
            if outside {
                rgb[y * 192 + x] = [10; 3];
            } else if scene == 0 && u < 65. {
                // A dark scene adjoining the left boundary is not a new holder position.
                rgb[y * 192 + x] = [10; 3];
            } else if (60. ..135.).contains(&u) && (32. ..96.).contains(&v) {
                rgb[y * 192 + x] = [5000 + scene as u16 * 1000; 3];
            }
        }
    }
    Capture {
        area: SourceRegion {
            x: 12,
            y: 20,
            width: 768,
            height: 512,
        },
        size,
        rgb,
        group: Group {
            lines: Default::default(),
            families: vec![],
        },
    }
}

#[test]
fn pooled_opening_ignores_one_dark_scene_and_excludes_rounded_corners_and_terminal_fuzz() {
    for angle in [0., 1.25, -1.25] {
        let captures: Vec<_> = (0..3).map(|n| capture(n, angle)).collect();
        let model = RollModel::fit(&captures).unwrap();
        let mut previous = None;
        for (i, c) in captures.iter().enumerate() {
            let a = model.areas(i, c, c.size, &c.rgb).unwrap();
            assert!(
                a.opening_shared && !a.opening_exception,
                "{angle} {i}: {a:?}"
            );
            assert_eq!(a.opening_observed, [false; 4]);
            let opening = a.opening.unwrap();
            if let Some(previous) = previous {
                assert_eq!(opening, previous);
            }
            previous = Some(opening);
            assert!(inside(c.source_point([96., 64.]), &opening));
            for y in 0..128 {
                for x in 0..192 {
                    if captures[1].rgb[y * 192 + x] == [10; 3] {
                        assert!(
                            !inside(c.source_point([x as f64 + 0.5, y as f64 + 0.5]), &opening),
                            "holder admitted at {x},{y}, angle {angle}"
                        );
                    }
                }
            }
            assert!(a.picture.is_some());
            if i == 0 {
                assert_eq!(c.rgb[64 * 192 + 45], [10; 3]);
                assert!(
                    a.contains_footprint(
                        SourceRegion {
                            x: c.area.x + 45 * 4,
                            y: c.area.y + 64 * 4,
                            width: 8,
                            height: 8
                        },
                        c.area,
                        false
                    ),
                    "a nonrecurring dense scene is not a holder exclusion"
                );
            }
            a.validate(c.area).unwrap();
        }
        let reversed: Vec<_> = captures.iter().rev().collect();
        let other = RollModel::fit_refs(&reversed).unwrap();
        assert_eq!(
            other
                .areas(0, reversed[0], reversed[0].size, &reversed[0].rgb)
                .unwrap()
                .opening,
            previous
        );
    }
}

#[test]
fn ragged_individual_edges_can_strengthen_a_common_interior_without_perfect_local_rails() {
    let captures: Vec<_> = (0..12)
        .map(|i| {
            let mut c = capture(1 + i % 3, 0.);
            for y in 24..104 {
                for x in 0..26 + 3 * ((y / 4 + i) % 4) {
                    c.rgb[y * 192 + x] = [10; 3];
                }
            }
            c
        })
        .collect();
    let model = RollModel::fit(&captures).unwrap();
    let mut areas: Vec<_> = captures
        .iter()
        .enumerate()
        .map(|(i, c)| model.areas(i, c, c.size, &c.rgb).unwrap())
        .collect();
    assert!(
        model.opening.is_some(),
        "ragged local rails vetoed the shared model"
    );
    assert!(
        areas.iter().all(|a| a.opening_shared),
        "ragged local rails must not veto corroborated exterior: {:?}",
        areas
            .iter()
            .enumerate()
            .filter(|(_, a)| !a.opening_shared)
            .map(|(i, a)| (i, a.opening))
            .collect::<Vec<_>>()
    );
    model.refine_areas(&mut areas).unwrap();
    for (c, a) in captures.iter().zip(&areas) {
        for y in 0..128 {
            for x in 0..192 {
                if c.rgb[y * 192 + x] == [10; 3] {
                    assert!(!inside(
                        c.source_point([x as f64 + 0.5, y as f64 + 0.5]),
                        &a.opening.unwrap()
                    ));
                }
            }
        }
    }
}
fn moved_capture(dx: usize) -> Capture {
    let mut c = capture(1, 0.);
    let original = c.rgb.clone();
    c.rgb.fill([10; 3]);
    for y in 0..128 {
        for x in dx..192 {
            c.rgb[y * 192 + x] = original[y * 192 + x - dx];
        }
    }
    c
}
#[test]
fn paired_motion_is_a_photo_exception_not_a_scene_edge_or_a_shared_redefinition() {
    let captures = [
        capture(1, 0.),
        capture(2, 0.),
        capture(3, 0.),
        moved_capture(20),
    ];
    let model = RollModel::fit(&captures).unwrap();
    let c = &captures[3];
    let a = model.areas(3, c, c.size, &c.rgb).unwrap();
    assert!(a.opening_shared && a.opening_exception);
    let q = a.opening.unwrap();
    for y in 0..128 {
        for x in 0..192 {
            if c.rgb[y * 192 + x] == [10; 3] {
                assert!(
                    !inside(c.source_point([x as f64 + 0.5, y as f64 + 0.5]), &q),
                    "moved holder at {x},{y} admitted"
                );
            }
        }
    }
    let a = capture(1, 0.);
    let b = moved_capture(20);
    assert!(
        Shared::fit(&[&a, &b], Some([100.; 3])).is_none(),
        "two incompatible captures cannot establish a common aperture"
    );
}
#[test]
fn unsupported_scene_darkness_and_single_captures_do_not_define_a_shared_holder() {
    let mut c = capture(1, 0.);
    c.rgb.fill([10500; 3]);
    for y in 32..96 {
        for x in 60..135 {
            c.rgb[y * 192 + x] = [10; 3];
        }
    }
    assert!(Shared::fit(&[&c], Some([100.; 3])).is_none());
    assert!(Shared::fit(&[&c, &c], Some([100.; 3])).is_none());
    let a = RollModel::fit_refs(&[&c, &c])
        .unwrap()
        .areas(0, &c, c.size, &c.rgb)
        .unwrap();
    assert!(!a.opening_shared);
}

#[test]
fn independently_measured_inward_departures_tighten_shared_support_without_expanding_it() {
    let c = capture(1, 0.);
    let shared = Shared::fit(&[&c, &c], Some([100.; 3])).unwrap();
    let (base, _) = shared.constrain(
        c.size,
        [[0., 0.], [192., 0.], [192., 128.], [0., 128.]],
        [false; 4],
        [None; 4],
    );
    let local = [[40., 4.], [150., 4.], [150., 124.], [40., 124.]];
    let (exception, changed) =
        shared.constrain(c.size, local, [true, true, false, false], [None; 4]);
    assert!(changed);
    let exception = exception.unwrap();
    assert!(
        exception
            .iter()
            .all(|&p| inside(p, &base.unwrap()) && inside(p, &local))
    );
    assert!(exception.iter().all(|p| p[0] >= 40. && p[0] <= 150.));
    let (same, changed) = shared.constrain(c.size, local, [false; 4], [None; 4]);
    assert!(!changed);
    assert_eq!(same, base);
    let (missing, changed) = shared.constrain(
        c.size,
        [[0., 0.], [8., 0.], [8., 128.], [0., 128.]],
        [true; 4],
        [None; 4],
    );
    assert!(
        changed && missing.is_none(),
        "an empty intersection must not expand back to a local fallback"
    );
}

#[test]
fn fine_measurements_strengthen_one_shared_interior_but_preserve_authored_and_exception_areas() {
    let captures = [
        capture(1, 0.),
        capture(2, 0.),
        capture(3, 0.),
        moved_capture(20),
    ];
    let model = RollModel::fit(&captures).unwrap();
    let mut areas: Vec<_> = captures
        .iter()
        .enumerate()
        .map(|(i, c)| model.areas(i, c, c.size, &c.rgb).unwrap())
        .collect();
    assert!(areas[3].opening_exception);
    let exceptional = areas[3].clone();
    let mut q = areas[1].opening.unwrap();
    q[0][0] += 4.;
    q[3][0] += 4.;
    q = q.map(|p| {
        let centre = [396., 276.];
        let p = project([p[0] - centre[0], p[1] - centre[1]], -0.75);
        [p[0] + centre[0], p[1] + centre[1]]
    });
    areas[1].opening = Some(q);
    let interiors: Vec<_> = areas[..3].iter().map(|a| a.opening.unwrap()).collect();
    model.refine_areas(&mut areas).unwrap();
    assert_eq!(areas[0].opening, areas[1].opening);
    assert_eq!(areas[0].opening, areas[2].opening);
    for p in areas[0].opening.unwrap() {
        assert!(interiors.iter().all(|q| inside(p, q)));
    }
    assert_eq!(areas[3], exceptional);
    areas[1].edit_opening(Coverage::rectangle(captures[1].area));
    let authored = areas[1].clone();
    model.refine_areas(&mut areas).unwrap();
    assert_eq!(areas[1], authored);
    assert!(model.refine_areas(&mut areas[..3]).is_err());
}
#[test]
fn opening_changes_refit_automatic_picture_coverage_without_moving_frame_evidence_or_authored_edits()
 {
    let active = SourceRegion {
        x: 12,
        y: 20,
        width: 768,
        height: 512,
    };
    let mut a = Areas {
        picture_geometry: Some(PictureGeometry {
            size: [192, 128],
            angle: 0.,
            sides: [Some(30.), Some(162.), None, None],
        }),
        picture_observed: [true, true, false, false],
        ..Areas::default()
    };
    a.edit_opening(Coverage::rectangle(active));
    a.refit_picture(active);
    let original = a.picture.unwrap();
    a.edit_opening(Coverage::rectangle(SourceRegion {
        x: 12 + 20 * 4,
        y: 20 + 15 * 4,
        width: 152 * 4,
        height: 90 * 4,
    }));
    a.refit_picture(active);
    let smaller = a.picture.unwrap();
    assert_eq!(smaller[0][0], original[0][0]);
    assert_eq!(smaller[1][0], original[1][0]);
    assert!(smaller[0][1] > original[0][1] && smaller[2][1] < original[2][1]);
    a.validate(active).unwrap();
    a.edit_picture(Areas::coverage(original).unwrap());
    a.edit_opening(Coverage::rectangle(SourceRegion {
        x: 12 + 35 * 4,
        y: 20 + 20 * 4,
        width: 120 * 4,
        height: 80 * 4,
    }));
    a.refit_picture(active);
    assert_eq!(a.picture, Some(original));
    assert!(a.picture_geometry.is_none());
}
