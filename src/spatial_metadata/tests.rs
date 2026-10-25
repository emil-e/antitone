use super::*;
use crate::project::{Save, Source};
use tempfile::TempDir;

fn region(bounds: [f64; 4]) -> Region {
    Region::from_picture(
        &PictureGeometry {
            size: [128, 96],
            angle: 0.,
            sides: bounds.map(Some),
        },
        [13, 29, 1280, 1920].into(),
        [true; 4],
    )
    .unwrap()
}

fn fixture() -> (TempDir, Project, Metadata, Vec<Input>) {
    let dir = tempfile::tempdir().unwrap();
    let mut project = Project::default();
    let mut sources = vec![];
    for i in 0..3 {
        let path = dir.path().join(format!("source-{i}.raw"));
        std::fs::write(&path, [i as u8; 16]).unwrap();
        let development = SourceDevelopment {
            decode: Development::new("test-backend".into(), [1.; 4]),
            lens_correction: None,
            dng_development: None,
            dimensions: [1300, 1960],
            active_area: [13, 29, 1280, 1920],
            camera_model: "fixture".into(),
            camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            input_floor: crate::processing::CAMERA_INPUT_FLOOR,
        };
        let id = project
            .add_source(Source {
                id: SourceId(0),
                path: path.clone(),
                development: development.clone(),
                reference_only: false,
                photo: Default::default(),
                repairs: Default::default(),
                local_adjustments: None,
                orientation: Default::default(),
                crop: None,
            })
            .unwrap();
        let observation = region([16., 112., 12., 84.]);
        sources.push(SourceRegions {
            input: Input {
                source: id,
                revision: FileRevision::read(&path).unwrap(),
                development,
                correction: None,
                code_scale: 1.,
            },
            observation: Some(observation.clone()),
            picture: Some(observation),
            film: None,
            holder_opening: None,
            material: None,
        });
    }
    let inputs = sources.iter().map(|s| s.input.clone()).collect();
    let common_size = CommonSize::fit(&sources);
    let metadata = Metadata {
        version: VERSION,
        groups: vec![Group {
            development: project.scan_basis().unwrap().development.decode.clone(),
            code_scale: 1.,
            sources,
            common_size,
        }],
    };
    metadata.validate().unwrap();
    (dir, project, metadata, inputs)
}

#[test]
fn every_source_can_be_clipped_while_different_observations_establish_common_axes() {
    let (_dir, project, mut metadata, inputs) = fixture();
    let group = &mut metadata.groups[0];
    for (source, side) in group.sources.iter_mut().zip([2, 0, 3]) {
        let observation = source.observation.as_mut().unwrap();
        observation.boundaries[side] = Boundary::CameraClipped;
        source.picture = Some(observation.clone());
        assert!(observation.physical_quad().is_none());
    }
    group.common_size = CommonSize::fit(&group.sources);
    assert_eq!(group.common_size.extent, [Some(960.), Some(1440.)]);
    assert_eq!(group.common_size.observations[0].len(), 2);
    assert_eq!(group.common_size.observations[1].len(), 1);
    metadata.validate().unwrap();
    assert_eq!(metadata.matching(&project, &inputs), metadata);
    for source in &metadata.groups[0].sources {
        let crop = source
            .picture
            .as_ref()
            .unwrap()
            .visible_crop(source.input.development.active_area.into(), &[])
            .unwrap();
        crop.region(source.input.development.active_area.into())
            .unwrap();
    }
}

#[test]
fn all_tops_clipped_leave_height_unknown_without_discarding_visible_picture_or_width() {
    let (_dir, _, mut metadata, _) = fixture();
    let group = &mut metadata.groups[0];
    for source in &mut group.sources {
        source.observation.as_mut().unwrap().boundaries[2] = Boundary::CameraClipped;
        source.picture = source.observation.clone();
    }
    group.common_size = CommonSize::fit(&group.sources);
    assert_eq!(group.common_size.extent, [Some(960.), None]);
    assert!(group.common_size.observations[1].is_empty());
    metadata.validate().unwrap();
    let picture = metadata.groups[0].sources[0].picture.as_ref().unwrap();
    let crop = picture
        .visible_crop([13, 29, 1280, 1920].into(), &[])
        .unwrap();
    assert_eq!(
        crop.region([13, 29, 1280, 1920].into()).unwrap(),
        SourceRegion::from([173, 29, 960, 1680])
    );
    let mut unknown = picture.clone();
    unknown.boundaries[2] = Boundary::Unknown;
    assert!(
        unknown
            .visible_crop([13, 29, 1280, 1920].into(), &[])
            .is_err(),
        "unknown is not proof of camera clipping"
    );
}

#[test]
fn common_size_is_a_median_of_measurements_not_a_consumer_or_inference_feedback_loop() {
    let (_dir, _, mut metadata, _) = fixture();
    let group = &mut metadata.groups[0];
    for (source, right) in group.sources.iter_mut().zip([110., 112., 116.]) {
        source.observation = Some(region([16., right, 12., 84.]));
        let mut inferred = region([10., 120., 8., 88.]);
        inferred.boundaries[1] = Boundary::Inferred {
            plane: inferred.boundaries[1].plane().unwrap(),
        };
        source.picture = Some(inferred);
    }
    group.common_size = CommonSize::fit(&group.sources);
    assert_eq!(group.common_size.extent, [Some(960.), Some(1440.)]);
    group.sources.reverse();
    assert_eq!(CommonSize::fit(&group.sources), group.common_size);
    metadata.validate().unwrap();
    metadata.groups[0].sources[0].observation = metadata.groups[0].sources[0].picture.clone();
    assert!(metadata.validate().is_err());
}

#[test]
fn source_plane_conversion_preserves_rotated_unequal_scale_extents_and_nonzero_origin() {
    let active = SourceRegion::from([13, 29, 1280, 1920]);
    for angle in [-1.25_f64, 0.375, 1.] {
        let frame = PictureGeometry {
            size: [128, 96],
            angle,
            sides: [16., 112., 12., 84.].map(Some),
        };
        let region = Region::from_picture(&frame, active, [true; 4]).unwrap();
        let quad = region.physical_quad().unwrap();
        for (a, b) in quad.iter().zip(frame.frame(active).unwrap()) {
            assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 1e-8);
        }
        let (s, c) = angle.to_radians().sin_cos();
        assert!(
            (region.measured_extent(active, 0).unwrap() - 96. * (10. * c).hypot(20. * s)).abs()
                < 1e-8
        );
        assert!(
            (region.measured_extent(active, 1).unwrap() - 72. * (10. * s).hypot(20. * c)).abs()
                < 1e-8
        );
        let crop = region.visible_crop(active, &[]).unwrap();
        crop.region(active).unwrap();
        for point in crop.coverage().unwrap().corners() {
            for [a, b, c] in region.boundaries.iter().filter_map(Boundary::plane) {
                assert!(a * point.0[0] + b * point.0[1] >= c - 1e-7);
            }
        }
    }
}

#[test]
fn rotated_unequal_scale_opposite_only_observations_keep_both_transformed_source_axes() {
    let active = SourceRegion::from([13, 29, 1280, 1920]);
    for angle in [-3.25_f64, 4.75] {
        let (s, c) = angle.to_radians().sin_cos();
        for axis in 0..2 {
            let frame = PictureGeometry {
                size: [128, 96],
                angle,
                sides: std::array::from_fn(|side| {
                    (usize::from(side >= 2) == axis).then_some([16., 112., 12., 84.][side])
                }),
            };
            let observed = Region::from_picture(&frame, active, [true; 4]).unwrap();
            assert!(observed.physical_quad().is_none());
            let expected = if axis == 0 {
                96. * (10. * c).hypot(20. * s)
            } else {
                72. * (10. * s).hypot(20. * c)
            };
            assert!((observed.measured_extent(active, axis).unwrap() - expected).abs() < 1e-8);
            assert!(observed.measured_extent(active, 1 - axis).is_none());
        }
    }
}

#[test]
fn invalid_or_crossed_geometry_never_becomes_available_coverage() {
    for plane in [
        [0., 0., 1.],
        [1., 0., f64::NAN],
        [-1., 0., -1.],
        [2., 0., 1.],
    ] {
        let mut bad = region([16., 112., 12., 84.]);
        bad.boundaries[0] = Boundary::Measured { plane };
        assert!(bad.validate().is_err());
    }
    let mut crossed = region([16., 112., 12., 84.]);
    crossed.boundaries[0] = Boundary::Measured {
        plane: [1., 0., 1200.],
    };
    assert!(crossed.validate().is_err());
    assert!(
        crossed
            .visible_crop([13, 29, 1280, 1920].into(), &[])
            .is_err()
    );
    let valid = region([16., 112., 12., 84.]);
    assert!(
        valid
            .visible_crop([13, 29, 1280, 1920].into(), &[[1., 0., 2000.]])
            .is_err()
    );
}

#[test]
fn freshness_uses_sources_and_measurement_basis_not_grade_order_or_accepted_crop() {
    let (_dir, mut project, metadata, mut inputs) = fixture();
    project.order.reverse();
    project.source_mut(inputs[0].source).unwrap().photo.exposure = 1.;
    project.source_mut(inputs[0].source).unwrap().crop =
        Some(Crop::from_region([173, 269, 960, 1440].into()));
    inputs.reverse();
    assert_eq!(metadata.matching(&project, &inputs), metadata);
    inputs[0].correction = Some([1; 32]);
    assert!(metadata.matching(&project, &inputs).groups.is_empty());
    inputs[0].correction = None;
    let path = project.source(inputs[0].source).unwrap().path.clone();
    std::fs::write(&path, b"replaced input with different length").unwrap();
    inputs[0].revision = FileRevision::read(&path).unwrap();
    assert!(metadata.matching(&project, &inputs).groups.is_empty());
}

#[test]
fn code_basis_and_measured_visibility_cannot_be_forged_into_reusable_dimensions() {
    let (_dir, project, metadata, mut inputs) = fixture();
    inputs[0].code_scale = 2.;
    assert!(metadata.matching(&project, &inputs).groups.is_empty());
    let mut bad = metadata.clone();
    bad.groups[0].code_scale = 2.;
    assert!(bad.validate().is_err());
    let mut outside = metadata;
    outside.groups[0].sources[0]
        .observation
        .as_mut()
        .unwrap()
        .boundaries[0] = Boundary::Measured {
        plane: [1., 0., -100.],
    };
    outside.groups[0].common_size = CommonSize::fit(&outside.groups[0].sources);
    assert!(
        outside.validate().is_err(),
        "an off-camera line cannot be a measured dimension donor"
    );
}

#[test]
fn source_conversion_roundoff_does_not_remove_whole_pixels_or_swallow_real_insets() {
    let active = SourceRegion::from([13, 29, 1280, 1920]);
    let exact = region([16., 112., 12., 84.]);
    assert_eq!(exact.visible_crop(active, &[]).unwrap().size, [960, 1440]);
    let mut shifted = exact;
    let Boundary::Measured { plane } = &mut shifted.boundaries[0] else {
        unreachable!()
    };
    plane[2] += 1e-6;
    let crop = shifted.visible_crop(active, &[]).unwrap();
    assert_eq!(
        crop.region(active).unwrap(),
        SourceRegion::from([174, 269, 959, 1440])
    );
}

#[test]
fn crop_uses_nominal_picture_even_when_film_holder_and_material_cover_less() {
    let (_dir, _, mut metadata, _) = fixture();
    let source = &mut metadata.groups[0].sources[0];
    source.film = Some(region([40., 70., 10., 60.]));
    source.holder_opening = Some(region([32., 96., 24., 72.]));
    source.material = Some(crate::scan_regions::Material {
        size: [128, 96],
        xs: vec![64],
        ys: vec![48],
        labels: "L".into(),
    });
    let nominal = crate::auto_crop::propose_spatial(source).unwrap();
    assert_eq!(
        nominal
            .region(source.input.development.active_area.into())
            .unwrap(),
        SourceRegion::from([173, 269, 960, 1440])
    );
    assert_eq!(
        source.picture_sampling(10.).unwrap(),
        [[183., 279.], [1123., 279.], [1123., 1699.], [183., 1699.]]
    );
    assert_eq!(source.observation, source.picture);
}

#[test]
fn changed_group_does_not_discard_an_independent_group_and_ambiguous_inputs_are_unavailable() {
    let (_dir, project, mut metadata, mut inputs) = fixture();
    let mut other = metadata.groups[0].clone();
    other.sources = vec![other.sources.remove(2)];
    metadata.groups[0].sources.truncate(2);
    metadata.groups[0].common_size = CommonSize::fit(&metadata.groups[0].sources);
    other.common_size = CommonSize::fit(&other.sources);
    metadata.groups.push(other);
    inputs[0].correction = Some([2; 32]);
    assert_eq!(
        metadata.matching(&project, &inputs).groups,
        vec![metadata.groups[1].clone()]
    );
    inputs.push(inputs[1].clone());
    assert!(metadata.matching(&project, &inputs).groups.is_empty());
}

#[test]
fn store_rejects_retired_and_foreign_requests_and_save_snapshots_do_not_replace_later_results() {
    let (_dir, project, metadata, inputs) = fixture();
    let accepted = project.clone();
    let mut store = Store::default();
    let old = store.begin().unwrap();
    assert!(
        store.snapshot(&project, &inputs).groups.is_empty(),
        "pending work is not saveable output"
    );
    let current = store.begin().unwrap();
    assert!(
        !store
            .publish(old, metadata.clone(), &project, &inputs)
            .unwrap()
    );
    assert!(
        store
            .publish(current, metadata.clone(), &project, &inputs)
            .unwrap()
    );
    let saved = store.snapshot(&project, &inputs);
    let mut later = metadata.clone();
    later.groups[0].sources[0].picture = None;
    let request = store.begin().unwrap();
    assert!(
        store
            .publish(request, later.clone(), &project, &inputs)
            .unwrap()
    );
    assert_eq!(saved, metadata);
    assert_eq!(store.snapshot(&project, &inputs), later);
    assert_eq!(
        project, accepted,
        "derived publication never touches accepted state or its equality"
    );
    let foreign = store.begin().unwrap();
    let mut another = Store::default();
    another.begin().unwrap();
    assert!(
        !another
            .publish(foreign, metadata.clone(), &project, &inputs)
            .unwrap()
    );
    let cancelled = store.begin().unwrap();
    store.cancel();
    assert!(
        !store
            .publish(cancelled, metadata.clone(), &project, &inputs)
            .unwrap()
    );
    let retired = store.begin().unwrap();
    store.close();
    assert!(!store.publish(retired, metadata, &project, &inputs).unwrap());
    assert!(store.begin().is_none());
    assert_eq!(
        saved.groups.len(),
        1,
        "retiring publication does not invalidate captured write data"
    );
}

#[test]
fn project_roundtrip_separates_accepted_state_and_reusable_metadata_without_waiting_for_work() {
    let (dir, project, metadata, inputs) = fixture();
    let accepted = project.clone();
    let path = dir.path().join("saved.antitone");
    project
        .save_with_spatial_metadata(&path, Save::New, &metadata)
        .unwrap();
    let (loaded, restored) = Project::load_with_spatial_metadata(&path).unwrap();
    assert_eq!(loaded, accepted);
    assert!(loaded.spatial_metadata.is_none());
    assert_eq!(restored.matching(&loaded, &inputs), metadata);
    assert_eq!(project, accepted);
    assert!(
        project
            .save_with_spatial_metadata(&path, Save::New, &Metadata::default())
            .is_err()
    );
    assert_eq!(
        Project::load_with_spatial_metadata(&path).unwrap().1,
        metadata,
        "failed no-clobber save damaged existing metadata"
    );
    project
        .save(&dir.path().join("legacy.antitone"), Save::New)
        .unwrap();
    let (legacy, empty) =
        Project::load_with_spatial_metadata(&dir.path().join("legacy.antitone")).unwrap();
    assert_eq!(legacy, project);
    assert!(empty.groups.is_empty());
}

#[test]
fn invalid_or_unsupported_derived_payload_is_discardable_but_accepted_unknown_fields_are_not() {
    let (dir, project, metadata, _) = fixture();
    let path = dir.path().join("saved.antitone");
    project
        .save_with_spatial_metadata(&path, Save::New, &metadata)
        .unwrap();
    let original: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for mut payload in [
        serde_json::json!({"version":999,"groups":[]}),
        {
            let mut old = serde_json::to_value(&metadata).unwrap();
            old["version"] = serde_json::json!(2);
            old
        },
        serde_json::json!({"unexpected":true}),
        serde_json::to_value(&metadata).unwrap(),
    ] {
        if payload["version"] == VERSION {
            payload["groups"][0]["sources"][0]["observation"]["boundaries"][0]["plane"] =
                serde_json::json!([0., 0., 1.]);
        }
        let mut file = original.clone();
        file["spatial_metadata"] = payload;
        std::fs::write(&path, serde_json::to_vec(&file).unwrap()).unwrap();
        let (loaded, metadata) = Project::load_with_spatial_metadata(&path).unwrap();
        assert_eq!(loaded, project);
        assert!(metadata.groups.is_empty());
    }
    let mut bad = original;
    bad["unknown_accepted_setting"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(Project::load_with_spatial_metadata(&path).is_err());
}
