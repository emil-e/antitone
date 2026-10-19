#![cfg(feature = "project")]
use antitone::{
    dust_repair::{DonorChoice, RepairOrigin, SupportMask},
    processing::CAMERA_INPUT_FLOOR,
    project::{Development, Project, Source, SourceDevelopment, SourceId},
};

fn project() -> (Project, SourceId) {
    let mut project = Project {
        development: Some(Development::new("test".into(), [1.; 4])),
        dmin: Some([0.8; 3]),
        dmax: Some([0.02; 3]),
        input_floor: Some(CAMERA_INPUT_FLOOR),
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "negative.ARW".into(),
            reference_only: false,
            development: SourceDevelopment {
                lens_correction: None,
                dng_development: None,
                dimensions: [900, 600],
                active_area: [12, 12, 864, 576],
                camera_model: "test".into(),
                camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                input_floor: CAMERA_INPUT_FLOOR,
            },
            photo: Default::default(),
            orientation: Default::default(),
            crop: None,
            repairs: Default::default(),
            local_adjustments: None,
        })
        .unwrap();
    (project, id)
}

#[test]
fn accepted_recipes_separate_neutral_identity_from_grades_composition_and_repair_metadata() {
    let (mut project, id) = project();
    let empty = project.image_recipe(id).unwrap();
    let repair = project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([120, 100], [1, 1], vec![true]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Automatic,
        )
        .unwrap();
    let repaired = project.image_recipe(id).unwrap();
    assert_ne!(empty, repaired);
    assert!(empty.neutral == repaired.neutral);
    project.source_mut(id).unwrap().photo.exposure = 1.;
    project.roll.exposure = 0.5;
    project.orientation.turn_clockwise();
    let graded = project.image_recipe(id).unwrap();
    assert_ne!(repaired, graded);
    assert!(repaired.same_neutral_content(&graded));
    assert_eq!(
        repaired.grade.global().exposure,
        0.,
        "captured recipe is immutable"
    );
    project.roll.gamma = 0.7;
    assert!(!graded.same_neutral_content(&project.image_recipe(id).unwrap()));
    project.roll.gamma = graded.roll.gamma;
    let mut metadata = serde_json::to_value(&project).unwrap();
    metadata["sources"][0]["repairs"]["repairs"][0]["id"] = 4.into();
    metadata["sources"][0]["repairs"]["repairs"][0]["origin"] = "detected".into();
    metadata["sources"][0]["repairs"]["last_id"] = 8.into();
    let metadata: Project = serde_json::from_value(metadata).unwrap();
    assert_eq!(graded, metadata.image_recipe(id).unwrap());
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .set_enabled(repair, false)
        .unwrap();
    assert!(project.image_recipe(id).unwrap().repairs().is_empty());
    project.source_mut(id).unwrap().repairs.clear();
    assert!(project.image_recipe(id).unwrap().repairs().is_empty());
}

#[test]
fn recipe_capture_uses_the_requested_source_even_when_it_is_not_in_the_filmstrip() {
    let (mut project, first) = project();
    let mut second = project.source(first).unwrap().clone();
    second.path = "other.ARW".into();
    second.photo.exposure = 2.;
    let second = project.add_source(second).unwrap();
    project.remove_from_filmstrip(second).unwrap();
    let a = project.image_recipe(first).unwrap();
    let b = project.image_recipe(second).unwrap();
    assert_ne!(a.neutral, b.neutral);
    assert_eq!(a.grade.global().exposure, 0.);
    assert_eq!(b.grade.global().exposure, 2.);
}

#[test]
fn accepted_look_identity_is_numeric_immutable_and_downstream_of_neutral_repairs() {
    use antitone::looks::{LookAsset, LookOrigin, Lut3D};
    use std::sync::Arc;
    let (mut project, id) = project();
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([120, 100], [1, 1], vec![true]).unwrap(),
            RepairOrigin::Manual,
            DonorChoice::Automatic,
        )
        .unwrap();
    let none = project.image_recipe(id).unwrap();
    let lut = Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    project.output.look = Some(Arc::new(
        LookAsset::new("First".into(), LookOrigin::Imported, lut.clone()).unwrap(),
    ));
    let looked = project.image_recipe(id).unwrap();
    assert_ne!(none, looked);
    assert!(none.same_neutral_content(&looked));
    assert!(looked.retained_bytes() >= none.retained_bytes() + lut.byte_len());
    project.output.look = Some(Arc::new(
        LookAsset::new("Renamed".into(), LookOrigin::Imported, lut.clone()).unwrap(),
    ));
    assert_eq!(looked, project.image_recipe(id).unwrap());
    project.output.look = None;
    assert_eq!(none, project.image_recipe(id).unwrap());
    assert_eq!(looked.look.as_ref().unwrap().lut().id(), lut.id());
}
