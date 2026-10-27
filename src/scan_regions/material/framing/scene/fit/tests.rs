//! Geometry, donor admission and sampling support shared by the research detector.
use super::*;

pub(super) fn line(bound: f64, strong: bool) -> Line {
    Line {
        midpoint_pixel: bound - 0.5,
        angle_degrees: 0.,
        lower_quartile_gradient_score: if strong { 1. } else { 0.001 },
        local_gradient_background: Some(0.01),
        angle_limit_reached: false,
        outside_surface: None,
        span_fraction: [0.1, 0.9],
        span_intervals: None,
        partial_span: false,
        gradient_ratio: 0.,
        rank: 0,
    }
}

pub(super) fn fixture(
    size: [u32; 2],
    picture: [f64; 4],
    film_right: Option<f64>,
    strong: bool,
) -> (Capture, Evidence, Scene) {
    let [w, h] = size;
    let rgb = (0..h)
        .flat_map(|y| {
            (0..w).map(move |x| {
                if film_right.is_some_and(|right| f64::from(x) >= right) {
                    [40000; 3]
                } else if f64::from(x) >= picture[0]
                    && f64::from(x) < picture[1]
                    && f64::from(y) >= picture[2]
                    && f64::from(y) < picture[3]
                {
                    [6000; 3]
                } else {
                    [20000; 3]
                }
            })
        })
        .collect();
    let evidence = Evidence {
        lines: crate::scan_regions::lines::SIDES
            .into_iter()
            .enumerate()
            .map(|(side, name)| (name.into(), vec![line(picture[side], strong)]))
            .collect(),
        segments: crate::scan_regions::lines::SIDES
            .into_iter()
            .map(|name| (name.into(), vec![]))
            .collect(),
    };
    let capture =
        Capture::new([11, 17, w * 10, h * 10].into(), size, rgb, evidence.clone()).unwrap();
    let scene = Scene {
        size,
        holder: vec![false; (w * h) as usize],
        base: Some(Base {
            mean: [20000f64.log2(); 3],
            spread: [0.125; 3],
        }),
    };
    (capture, evidence, scene)
}

pub(super) fn shape_reference(ratio: Option<f64>) -> Reference {
    Reference {
        ratio,
        interval: ratio.map(|ratio| [ratio * 0.998, ratio * 1.002]),
        eligible: vec![100, 101, 102],
        donors: vec![100, 101, 102],
        rejected: vec![],
        error: None,
        shape: ratio.map(|ratio| super::super::super::shape::Shape {
            ratio,
            interval: [ratio * 0.998, ratio * 1.002],
            donors: vec![100, 101, 102],
            rejected: vec![],
        }),
    }
}

fn frame(size: [u32; 2], sides: [Option<f64>; 4]) -> PictureGeometry {
    PictureGeometry {
        size,
        angle: 0.,
        sides,
    }
}

#[test]
fn thin_rebate_measurement_stops_before_holder_instead_of_jumping_across_it() {
    let (mut capture, _, mut scene) = fixture([64, 48], [12., 60., 0., 48.], None, false);
    for y in 0..48 {
        for x in 0..11 {
            scene.holder[y * 64 + x] = true;
            capture.rgb[y * 64 + x] = [100; 3];
        }
    }
    let frame = frame(capture.size, [Some(12.), None, None, None]);
    let support = measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        0,
        &line(12., false),
    );
    assert_eq!(support.kind(), Kind::Picture);
    assert!(!support.contradicted());
    assert!(support.paired >= 12);
}

#[test]
fn a_near_source_edge_uses_the_available_thin_outer_band() {
    let (capture, _, scene) = fixture([64, 48], [0., 64., 0., 47.], None, false);
    let frame = frame(capture.size, [None, None, None, Some(47.)]);
    let support = measure(
        &capture,
        &scene.roles(&capture),
        &frame,
        3,
        &line(47., false),
    );
    assert_eq!(support.kind(), Kind::Picture);
    assert!(support.paired >= 12);
}

#[test]
fn missing_outside_is_unknown_not_a_measured_source_boundary() {
    let (capture, _, scene) = fixture([64, 48], [8., 56., 6., 48.], None, false);
    let support = measure(
        &capture,
        &scene.roles(&capture),
        &frame(capture.size, [None, None, None, Some(48.)]),
        3,
        &line(48., false),
    );
    assert_eq!(support.kind(), Kind::Unresolved);
    assert!(!support.contradicted());
}

#[test]
fn source_clipping_moves_only_visible_geometry_and_keeps_active_origin() {
    let active: SourceRegion = [11, 17, 1000, 500].into();
    let physical = frame([100, 50], [Some(-20.), Some(80.), Some(-10.), Some(45.)]);
    let crop = visible_crop(active, &physical).unwrap();
    assert_eq!(crop.size, [800, 450]);
    assert_eq!(crop.center, [411., 242.]);
    assert_eq!(physical.sides[0], Some(-20.));
    let disjoint = frame([100, 50], [Some(110.), Some(130.), Some(0.), Some(50.)]);
    assert!(visible_crop(active, &disjoint).is_none());
}

#[test]
fn rotated_visible_crop_stays_inside_source_and_original_hypothesis_with_unequal_scales() {
    let active: SourceRegion = [13, 29, 1000, 500].into();
    let mut physical = frame([100, 100], [Some(-5.), Some(90.), Some(-10.), Some(90.)]);
    physical.angle = 3.;
    let crop = visible_crop(active, &physical).unwrap();
    crop.region(active).unwrap();
    for corner in crop.coverage().unwrap().corners() {
        let raster = [(corner.0[0] - 13.) / 10., (corner.0[1] - 29.) / 5.];
        assert!(in_hypothesis(raster, &physical, None));
    }
}

#[test]
fn own_base_pixels_cannot_create_a_repeated_separator_without_peer_support() {
    let prepared = Prepared {
        roles: vec![
            vec![Role::BaseCompatible; 4],
            vec![Role::AttenuatedCompatible; 4],
            vec![Role::AttenuatedCompatible; 4],
        ],
        base_votes: vec![1; 4],
    };
    assert_eq!(prepared.persistent(0), vec![false; 4]);
}

#[test]
fn reference_uses_other_measured_full_frames_not_the_target_or_source_window() {
    let mut captures = vec![];
    for _ in 0..3 {
        captures.push(fixture([64, 48], [8., 56., 6., 42.], None, true).0);
    }
    captures.push(fixture([64, 48], [8., 56., 12., 36.], None, true).0);
    let model = RollModel::fit(&captures).unwrap();
    let scene = Scene::fit(&captures, &model);
    let prepared = Prepared::new(&scene, &captures);
    let reference = reference(&scene, &captures, &model, &prepared, 3, || Ok(())).unwrap();
    assert_eq!(reference.donors, vec![0, 1, 2]);
    assert_eq!(reference.eligible, reference.donors);
    assert!((reference.ratio.unwrap() - 4. / 3.).abs() < 1e-10);
}

#[test]
fn weak_or_unobserved_sides_do_not_become_complete_donor_geometry() {
    let complete = Areas {
        picture_geometry: Some(frame([64, 48], [Some(8.), Some(56.), Some(6.), Some(42.)])),
        picture_observed: [true; 4],
        ..Areas::default()
    };
    assert!(
        observed_geometry(&complete)
            .unwrap()
            .sides
            .iter()
            .all(Option::is_some)
    );
    for side in 0..4 {
        let mut weak = complete.clone();
        weak.picture_weak[side] = true;
        assert!(observed_geometry(&weak).unwrap().sides[side].is_none());
        let mut missing = complete.clone();
        missing.picture_observed[side] = false;
        assert!(observed_geometry(&missing).unwrap().sides[side].is_none());
    }
}

#[test]
fn peer_supported_base_separator_excludes_a_merged_frame_from_donor_geometry() {
    let mut captures = vec![];
    for _ in 0..4 {
        let (mut capture, _, _) = fixture([96, 64], [20., 88., 6., 58.], None, false);
        for y in 6..58 {
            for x in 0..16 {
                capture.rgb[y * 96 + x] = [6000; 3];
            }
        }
        captures.push(capture);
    }
    let (_, _, scene) = fixture([96, 64], [20., 88., 6., 58.], None, false);
    let prepared = Prepared::new(&scene, &captures);
    let bars = separators(
        &captures[0],
        &prepared.roles[0],
        &prepared.persistent(0),
        0.,
    );
    assert!(
        bars.iter()
            .any(|bar| bar.axis == 0 && bar.low >= 16. && bar.high <= 20.)
    );
    let merged = frame([96, 64], [Some(0.), Some(88.), Some(6.), Some(58.)]);
    let target = frame([96, 64], [Some(20.), Some(88.), Some(6.), Some(58.)]);
    assert!(crosses_separator(&merged, [None; 4], &bars));
    assert!(!crosses_separator(&target, [None; 4], &bars));
}
