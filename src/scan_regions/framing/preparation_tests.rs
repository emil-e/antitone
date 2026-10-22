//! Supplied native candidates exercise preparation/qualification, not blind line discovery.
use super::*;
use crate::{
    project::{SourceDevelopment, SourceId},
    spatial_metadata::{Boundary, FileRevision},
};

fn inputs() -> (tempfile::TempDir, Vec<Input>) {
    let directory = tempfile::tempdir().unwrap();
    let inputs = (1..=3)
        .map(|id| {
            let path = directory.path().join(format!("{id}.raw"));
            std::fs::write(&path, [id as u8; 16]).unwrap();
            Input {
                source: SourceId(id),
                revision: FileRevision::read(&path).unwrap(),
                correction: None,
                code_scale: 1.,
                development: SourceDevelopment {
                    lens_correction: None,
                    dng_development: None,
                    dimensions: [1300, 1000],
                    active_area: [11, 17, 1280, 960],
                    camera_model: "fixture".into(),
                    camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
            }
        })
        .collect();
    (directory, inputs)
}

fn frames(inputs: &[Input], kind: &str) -> Vec<FrameInput> {
    inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let (bounds, missing) = match kind {
                "top_clipped" | "top_heterogeneous" => ([16., 112., -8., 84.], Some(2)),
                "mixed_clipped" => match index {
                    0 => ([16., 112., -8., 64.], Some(2)),
                    1 => ([-8., 88., 12., 84.], Some(0)),
                    _ => ([16., 112., 32., 104.], Some(3)),
                },
                "top_unknown" | "edge_strip" => ([16., 112., 12., 84.], Some(2)),
                "opposite_member" if index == 0 => ([16., 112., -8., 104.], None),
                _ => ([16., 112., 12., 84.], None),
            };
            let cut = kind.starts_with("cut") && (kind != "cut_member" || index == 0);
            let (mut capture, mut evidence, _) =
                super::tests::fixture([128, 96], bounds, if cut { Some(100.) } else { None }, true);
            if let Some(side) = missing {
                evidence
                    .lines
                    .get_mut(crate::scan_regions::lines::SIDES[side])
                    .unwrap()
                    .clear();
            }
            if kind == "opposite_member" && index == 0 {
                evidence.lines.get_mut("top").unwrap().clear();
                evidence.lines.get_mut("bottom").unwrap().clear();
            }
            if kind == "blank" {
                capture.rgb.fill([20000; 3]);
            }
            if kind == "edge_strip" {
                for y in 0..3 {
                    for x in 16..112 {
                        capture.rgb[y * 128 + x] = [6000; 3];
                    }
                }
            }
            if kind == "top_heterogeneous" {
                for y in 0..6 {
                    for x in 16..112 {
                        capture.rgb[y * 128 + x] = if (20..32).contains(&x)
                            || (50..62).contains(&x)
                            || (82..94).contains(&x)
                        {
                            [6000; 3]
                        } else {
                            [20000; 3]
                        };
                    }
                }
            }
            if kind == "cut" || kind == "cut_member" && index == 0 {
                evidence
                    .lines
                    .get_mut("right")
                    .unwrap()
                    .push(super::tests::line(100., true));
            }
            FrameInput::new(input.clone(), capture.size, capture.rgb, evidence).unwrap()
        })
        .collect()
}

fn run(frames: Vec<FrameInput>) -> Metadata {
    prepare_spatial(
        frames,
        Development::new("test-backend".into(), [1.; 4]),
        1.,
        || Ok(()),
    )
    .unwrap()
}

#[test]
fn shared_preparation_produces_common_geometry_and_independent_consumer_insets() {
    let (_dir, inputs) = inputs();
    let metadata = run(frames(&inputs, "complete"));
    let group = &metadata.groups[0];
    assert!((group.common_size.extent[0].unwrap() - 960.).abs() < 1e-8);
    assert!((group.common_size.extent[1].unwrap() - 720.).abs() < 1e-8);
    assert_eq!(group.sources.len(), 3);
    for source in &group.sources {
        assert!(
            source
                .observation
                .as_ref()
                .unwrap()
                .boundaries
                .iter()
                .all(Boundary::is_measured)
        );
        let before = source.clone();
        let crop = crate::auto_crop::propose_spatial(source).unwrap();
        assert_eq!(crop.size, [960, 720]);
        let sampling = source.picture_sampling(10.).unwrap();
        assert!(
            (crate::scan_regions::Areas::coverage(sampling)
                .unwrap()
                .area()
                - 940. * 700.)
                .abs()
                < 1e-6
        );
        let margin =
            crate::auto_crop::inward_margin(crop, source.input.development.active_area.into(), 0.5)
                .unwrap();
        assert!(margin.size[0] < crop.size[0] && margin.size[1] < crop.size[1]);
        assert_eq!(
            crate::auto_crop::inward_margin(crop, source.input.development.active_area.into(), 0.)
                .unwrap(),
            crop
        );
        assert_eq!(*source, before);
        assert!(source.picture_sampling(f64::NAN).is_err());
        assert!(source.picture_sampling(360.).is_err());
    }
}

#[test]
fn all_clipped_photos_can_supply_both_dimensions_and_use_native_joint_fit_without_complete_peers() {
    let (_dir, inputs) = inputs();
    let metadata = run(frames(&inputs, "mixed_clipped"));
    let group = &metadata.groups[0];
    assert!((group.common_size.extent[0].unwrap() - 960.).abs() < 1e-8);
    assert!((group.common_size.extent[1].unwrap() - 720.).abs() < 1e-8);
    for source in &group.sources {
        assert!(
            source
                .observation
                .as_ref()
                .unwrap()
                .physical_quad()
                .is_none()
        );
        let picture = source.picture.as_ref().unwrap();
        let q = picture.physical_quad().unwrap();
        assert!(((q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]) - 960.).abs() < 1e-6);
        assert!(((q[3][0] - q[0][0]).hypot(q[3][1] - q[0][1]) - 720.).abs() < 1e-6);
        crate::auto_crop::propose_spatial(source)
            .unwrap()
            .region(source.input.development.active_area.into())
            .unwrap();
    }
}

#[test]
fn common_height_can_stay_unknown_while_camera_limited_visible_picture_remains_usable() {
    let (_dir, inputs) = inputs();
    let metadata = run(frames(&inputs, "top_clipped"));
    assert!((metadata.groups[0].common_size.extent[0].unwrap() - 960.).abs() < 1e-8);
    assert_eq!(metadata.groups[0].common_size.extent[1], None);
    for source in &metadata.groups[0].sources {
        let picture = source.picture.as_ref().unwrap();
        assert!(matches!(picture.boundaries[2], Boundary::CameraClipped));
        assert!(picture.physical_quad().is_none());
        assert_eq!(
            crate::auto_crop::propose_spatial(source)
                .unwrap()
                .region(source.input.development.active_area.into())
                .unwrap(),
            SourceRegion::from([171, 17, 960, 840])
        );
    }
    let unknown = run(frames(&inputs, "top_unknown"));
    for source in &unknown.groups[0].sources {
        assert!(matches!(
            source.picture.as_ref().unwrap().boundaries[2],
            Boundary::Unknown
        ));
        assert!(crate::auto_crop::propose_spatial(source).is_err());
    }
}

#[test]
fn a_three_pixel_camera_strip_separated_by_base_does_not_certify_picture_continuation() {
    let (_dir, inputs) = inputs();
    let metadata = run(frames(&inputs, "edge_strip"));
    for source in &metadata.groups[0].sources {
        assert!(matches!(
            source.picture.as_ref().unwrap().boundaries[2],
            Boundary::Unknown
        ));
        assert!(crate::auto_crop::propose_spatial(source).is_err());
    }
}

#[test]
fn camera_continuation_does_not_require_uniform_density_along_the_available_picture_edge() {
    let (_dir, inputs) = inputs();
    let metadata = run(frames(&inputs, "top_heterogeneous"));
    for source in &metadata.groups[0].sources {
        assert!(matches!(
            source.picture.as_ref().unwrap().boundaries[2],
            Boundary::CameraClipped
        ));
        assert!(crate::auto_crop::propose_spatial(source).is_ok());
    }
}

#[test]
fn blank_or_unreferenced_cut_claims_cannot_establish_a_frame_from_one_bright_edge() {
    let (_dir, inputs) = inputs();
    for kind in ["blank", "cut"] {
        let metadata = run(frames(&inputs, kind));
        assert!(
            metadata.groups[0]
                .sources
                .iter()
                .all(|source| source.observation.is_none() && source.picture.is_none()),
            "{kind}: {metadata:#?}"
        );
        assert_eq!(metadata.groups[0].common_size.extent, [None, None]);
        assert!(
            metadata.groups[0]
                .sources
                .iter()
                .all(|source| source.film.is_none()),
            "a cut without an independent base reference is unresolved, not a certified film limit"
        );
    }
}

#[test]
fn unreferenced_cut_preserves_credible_height_but_cannot_invent_missing_width() {
    let (_dir, inputs) = inputs();
    let metadata = run(frames(&inputs, "cut_unobserved"));
    assert_eq!(metadata.groups[0].common_size.extent, [None, Some(720.)]);
    for source in &metadata.groups[0].sources {
        let observation = source.observation.as_ref().unwrap();
        assert!(
            observation.boundaries[2..]
                .iter()
                .all(Boundary::is_measured)
        );
        assert!(!observation.boundaries[1].is_measured());
        assert!(source.film.is_none());
        assert!(
            crate::auto_crop::propose_spatial(source).is_err(),
            "no width model or right picture edge exists"
        );
    }
}

#[test]
fn learning_common_dimensions_does_not_erase_an_opposite_only_camera_limited_member() {
    let (_dir, inputs) = inputs();
    let metadata = run(frames(&inputs, "opposite_member"));
    assert!(
        metadata.groups[0]
            .common_size
            .extent
            .iter()
            .all(Option::is_some)
    );
    let source = &metadata.groups[0].sources[0];
    let observed = source.observation.as_ref().unwrap();
    assert!(observed.boundaries[..2].iter().all(Boundary::is_measured));
    assert!(
        observed.boundaries[2..]
            .iter()
            .all(|b| matches!(b, Boundary::CameraClipped))
    );
    let crop = crate::auto_crop::propose_spatial(source)
        .expect("visible-only geometry remains useful when the full frame cannot be located");
    assert_eq!(
        crop.region(source.input.development.active_area.into())
            .unwrap(),
        SourceRegion::from([171, 17, 960, 960])
    );
    assert!(source.picture.as_ref().unwrap().physical_quad().is_none());
}

#[test]
fn known_dimensions_recover_nominal_picture_without_measuring_the_cut_as_its_edge() {
    let (_dir, inputs) = inputs();
    // Two complete sources supply20000-code base;40000-code shine-through occurs
    // only on the cut source, so brightness is not circularly learned as film base.
    let metadata = run(frames(&inputs, "cut_member"));
    let cut = &metadata.groups[0].sources[0];
    let picture = cut
        .picture
        .as_ref()
        .expect("nominal frame survives unavailable film");
    assert!(matches!(picture.boundaries[1], Boundary::Inferred { .. }));
    assert!(!cut.observation.as_ref().unwrap().boundaries[1].is_measured());
    assert_eq!(
        crate::auto_crop::propose_spatial(cut)
            .unwrap()
            .region(cut.input.development.active_area.into())
            .unwrap(),
        SourceRegion::from([171, 137, 960, 720])
    );
    assert!(!metadata.groups[0].common_size.observations[0].contains(&cut.input.source));
    let film = cut.film.as_ref().expect("supported film-limit candidate");
    assert!(film.boundaries[1].is_measured());
    let [a, b, c] = film.boundaries[1].plane().unwrap();
    assert!((a * 1011. + b * 497. - c).abs() < 1e-8);
    assert!(film.physical_quad().is_none());
    assert!(
        metadata.groups[0].sources[1..]
            .iter()
            .all(|source| source.picture.is_some())
    );
}

#[test]
fn preparation_is_source_keyed_and_cancelled_work_has_no_partial_result() {
    let (_dir, inputs) = inputs();
    let normal = run(frames(&inputs, "complete"));
    let mut reversed = frames(&inputs, "complete");
    reversed.reverse();
    assert_eq!(run(reversed), normal);
    for stop in [1, 8] {
        let calls = std::cell::Cell::new(0);
        let result = prepare_spatial(
            frames(&inputs, "complete"),
            Development::new("test-backend".into(), [1.; 4]),
            1.,
            || {
                calls.set(calls.get() + 1);
                if calls.get() >= stop {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(result,Err(reason) if reason=="cancelled"));
    }
}
