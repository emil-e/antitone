use super::super::{lines::Group, material::Selection, rectangles};
use super::*;

fn fixture(faint: bool, fuzz: bool) -> (Capture, Selection) {
    let size = [192, 128];
    let mut rgb = vec![[10500; 3]; 192 * 128];
    for y in 0..128 {
        for x in 0..192 {
            if faint && (12..180).contains(&x) && (10..118).contains(&y) {
                rgb[y * 192 + x] = [10000; 3];
            }
            if fuzz && y >= 127 - (x / 7) % 3 {
                rgb[y * 192 + x] = [10; 3];
            }
        }
    }
    let material = Material {
        size,
        xs: (2..192).step_by(4).collect(),
        ys: (2..128).step_by(4).collect(),
        labels: "F".repeat(48 * 32),
    };
    (
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
        },
        Selection {
            material,
            upper: Some([100.; 3]),
            family: None,
            supported: [false; 4],
        },
    )
}

fn fit(c: &Capture, s: Selection) -> Areas {
    rectangles::fit(c, s, c.size, &c.rgb).unwrap()
}

fn corners(notch: bool, missing: bool) -> (Capture, Selection) {
    let (mut c, mut s) = fixture(false, false);
    for y in 0..128 {
        for x in 0..192 {
            if !(30..162).contains(&x)
                || y >= 126
                || ((30..34).contains(&x)
                    || (158..162).contains(&x)
                    || notch && (94..98).contains(&x))
                    && y >= 113
            {
                c.rgb[y * 192 + x] = [10; 3];
            }
            if missing && y >= 126 && (34..158).contains(&x) {
                c.rgb[y * 192 + x] = [10500; 3];
            }
        }
    }
    s.material.labels = s
        .material
        .ys
        .iter()
        .flat_map(|_| {
            s.material
                .xs
                .iter()
                .map(|&x| if !(34..158).contains(&x) { 'H' } else { 'F' })
        })
        .collect();
    (c, s)
}

#[test]
fn one_capture_cannot_supply_peer_material_evidence_and_wrong_roll_targets_reject() {
    let (mut c, _) = fixture(false, true);
    let single = RollModel::fit_refs(&[&c]).unwrap();
    let a = single.areas(0, &c, c.size, &c.rgb).unwrap();
    assert!(a.opening.is_none() && a.picture.is_none());
    let model = RollModel::fit_refs(&[&c, &c]).unwrap();
    assert!(model.areas(usize::MAX, &c, c.size, &c.rgb).is_err());
    c.rgb.fill([1234; 3]);
    assert!(model.areas(0, &c, c.size, &c.rgb).is_err());
}

#[test]
fn finite_but_overflowing_half_planes_reject_instead_of_admitting_nan_comparisons() {
    let (c, _) = fixture(false, false);
    let a = Areas {
        picture_constraints: vec![[1e308, 1., 1.]],
        ..Areas::default()
    };
    assert!(a.validate(c.area).is_err());
}

#[test]
fn contradictory_saved_provenance_is_rejected_before_presentation() {
    let (c, _) = fixture(false, false);
    let a = Areas {
        picture_observed: [true; 4],
        picture_weak: [true; 4],
        ..Areas::default()
    };
    assert!(a.validate(c.area).is_err());
}

#[test]
fn lateral_evidence_disambiguates_bottom_corners_without_inventing_observations() {
    let (c, s) = corners(false, false);
    let a = fit(&c, s);
    assert!(a.opening_observed[0] && a.opening_observed[1] && a.opening_observed[3]);
    assert!(inside(c.source_point([96., 120.]), &a.opening.unwrap()));
    assert!(!inside(c.source_point([96., 127.]), &a.opening.unwrap()));
    let (c, s) = corners(false, true);
    let a = fit(&c, s);
    assert!(a.opening_observed[0] && a.opening_observed[1]);
    assert!(!a.opening_observed[3]);
}

#[test]
fn retained_interior_material_contradictions_still_veto_bottom_measurement() {
    let (c, s) = corners(true, false);
    assert!(!fit(&c, s).opening_observed[3]);
}

#[test]
fn fine_terminal_tips_are_excluded_without_relabelling_the_material_grid() {
    let (c, s) = fixture(false, true);
    let material = s.material.clone();
    let a = fit(&c, s);
    assert!(a.opening_observed[3]);
    assert_eq!(a.material, Some(material));
    for x in 0..192 {
        for y in 125..128 {
            if c.rgb[y * 192 + x] == [10; 3] {
                assert!(!inside(
                    c.source_point([x as f64 + 0.5, y as f64 + 0.5]),
                    &a.opening.unwrap()
                ));
            }
        }
    }
}

#[test]
fn censored_or_deep_unsupported_dark_scenes_are_not_observed_opening_sides() {
    for width in [30, 100] {
        let (mut c, s) = fixture(false, false);
        for y in 0..128 {
            for x in 0..width {
                c.rgb[y * 192 + x] = [10; 3];
            }
        }
        assert!(!fit(&c, s).opening_observed[0]);
    }
}

#[test]
fn faint_picture_border_retains_faint_content_and_freezing_membership_is_not_a_value_filter() {
    let (mut c, s) = fixture(true, false);
    let a = fit(&c, s);
    let q = a.picture.unwrap();
    for p in [[13., 11.], [179., 117.], [96., 64.]] {
        assert!(inside(c.source_point(p), &q));
    }
    for p in [[8., 8.], [184., 122.]] {
        assert!(!inside(c.source_point(p), &q));
    }
    let footprint = SourceRegion {
        x: 12 + 74 * 4,
        y: 20 + 54 * 4,
        width: 8,
        height: 8,
    };
    assert!(a.contains_footprint(footprint, c.area, true));
    for y in 50..60 {
        for x in 70..80 {
            c.rgb[y * 192 + x] = [0, 65535, 10500];
        }
    }
    assert!(a.contains_footprint(footprint, c.area, true));
    assert!(!a.picture_observed.iter().any(|v| *v));
}

#[test]
fn absent_frame_transitions_remain_statistical_coverage_not_observed_physical_sides() {
    let (c, s) = fixture(false, false);
    let a = fit(&c, s);
    assert!(a.ready());
    assert_eq!(a.opening_observed, [false; 4]);
    assert_eq!(a.picture_observed, [false; 4]);
}
