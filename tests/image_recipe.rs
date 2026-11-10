#![cfg(feature = "project")]
use antitone::{
    color::Balance,
    dust_repair::{RepairOrigin, SupportMask},
    flat_field::FlatField,
    processing::{DEFAULT_GAMMA, InputSettings, Response},
    project::{Development, FlatFieldSettings, Project, Source, SourceDevelopment, SourceId},
};
use std::sync::Arc;

fn project() -> (Project, SourceId) {
    let mut project = Project {
        input: InputSettings::Negative {
            dmin: Some(glam::DVec3::splat(0.8)),
            dmax: Some(glam::DVec3::splat(0.02)),
            response: Response::film(),
            gamma: DEFAULT_GAMMA,
            balance: Balance::default(),
        },
        ..Default::default()
    };
    let id = project
        .add_source(Source {
            id: SourceId(0),
            path: "negative.ARW".into(),
            development: SourceDevelopment {
                decode: Development::new("test".into(), [1.; 4]),
                dimensions: [900, 600],
                active_area: [12, 12, 864, 576],
                camera_model: "test".into(),
                camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
            },

            orientation: Default::default(),
            crop: None,
            repairs: Default::default(),
            adjustment_layers: Some(antitone::project::AdjustmentLayers::with_global(
                Default::default(),
            )),
        })
        .unwrap();
    (project, id)
}

#[test]
fn uncalibrated_intent_can_be_captured_without_claiming_a_positive_is_ready() {
    let (mut project, id) = project();
    project.input = InputSettings::default();
    assert!(!project.input.ready());
    let recipe = project.image_recipe(id).unwrap();
    assert_eq!(recipe.neutral.input, project.input);
    assert!(recipe.neutral.input.calibration().is_none());

    let captured = recipe.neutral.clone();
    project.input = InputSettings::Digital;
    assert_ne!(captured, project.image_recipe(id).unwrap().neutral);
    assert!(
        !captured.input.ready(),
        "later calibration cannot change captured intent"
    );
}

#[test]
fn accepted_recipes_capture_grades_composition_and_repair_metadata() {
    let (mut project, id) = project();
    let empty = project.image_recipe(id).unwrap();
    let repair = project
        .source_mut(id)
        .unwrap()
        .repairs
        .insert(
            SupportMask::new([120, 100], [1, 1], vec![true]).unwrap(),
            RepairOrigin::Manual,
            [64, 0],
        )
        .unwrap();
    let repaired = project.image_recipe(id).unwrap();
    assert_ne!(empty, repaired);
    assert!(empty.neutral == repaired.neutral);
    project
        .source_mut(id)
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = 1.;
    project.roll.exposure = 0.5;
    project.orientation.turn_clockwise();
    let graded = project.image_recipe(id).unwrap();
    assert_ne!(repaired, graded);
    assert_eq!(repaired.neutral, graded.neutral);
    assert_eq!(repaired.repairs, graded.repairs);
    assert_eq!(
        repaired.grade.layers()[0].photo.exposure,
        0.,
        "captured recipe is immutable"
    );
    let original_gamma = project.input;
    if let InputSettings::Negative { gamma, .. } = &mut project.input {
        *gamma = 0.7;
    }
    assert_ne!(graded.neutral, project.image_recipe(id).unwrap().neutral);
    project.input = original_gamma;
    let mut metadata = serde_json::to_value(&project).unwrap();
    metadata["sources"][0]["repairs"]["repairs"][0]["id"] = 4.into();
    metadata["sources"][0]["repairs"]["repairs"][0]["origin"] = "detected".into();
    metadata["sources"][0]["repairs"]["last_id"] = 8.into();
    let metadata: Project = serde_json::from_value(metadata).unwrap();
    let captured_metadata = metadata.image_recipe(id).unwrap();
    assert_ne!(graded, captured_metadata);
    assert_eq!(captured_metadata.repairs[0].id.0, 4);
    assert_eq!(captured_metadata.repairs[0].origin, RepairOrigin::Detected);
    project
        .source_mut(id)
        .unwrap()
        .repairs
        .set_enabled(repair, false)
        .unwrap();
    assert!(project.image_recipe(id).unwrap().repairs.is_empty());
    project.source_mut(id).unwrap().repairs.clear();
    assert!(project.image_recipe(id).unwrap().repairs.is_empty());
}

#[test]
fn recipe_capture_uses_the_requested_photo_and_deleted_photos_leave_only_correction_provenance() {
    let (mut project, first) = project();
    let mut second = project.source(first).unwrap().clone();
    second.path = "other.ARW".into();
    second.adjustment_layers.as_mut().unwrap().layers[0]
        .adjustment
        .photo
        .exposure = 2.;
    let second = project.add_source(second).unwrap();
    let map = Arc::new(
        FlatField::from_nodes([12, 12, 864, 576], [2, 2], vec![glam::DVec3::ONE; 4]).unwrap(),
    );
    let field = FlatFieldSettings::new(project.source(second).unwrap().path.clone(), map);
    project.set_flat_field(Some(field.clone())).unwrap();
    assert!(project.order.contains(&first) && project.order.contains(&second));
    let a = project.image_recipe(first).unwrap();
    let b = project.image_recipe(second).unwrap();
    assert_ne!(a.neutral, b.neutral);
    assert_eq!(a.grade.layers()[0].photo.exposure, 0.);
    assert_eq!(b.grade.layers()[0].photo.exposure, 2.);
    project.remove_from_filmstrip(second).unwrap();
    assert!(project.image_recipe(second).is_err());
    assert_eq!(project.flat_field(), Some(&field));
    project.validate().unwrap();
}

#[test]
fn halation_is_captured_without_repair_invalidation_or_grain_enablement() {
    let (mut project, id) = project();
    let clean = project.image_recipe(id).unwrap();
    project.output.halation = antitone::halation::HalationSettings {
        enabled: true,
        size: 0.125,
        strength: 0.3,
    };
    let captured = project.image_recipe(id).unwrap();
    assert_ne!(clean, captured);
    assert_eq!(clean.neutral, captured.neutral);
    assert!(!captured.grain.enabled);
    assert!(
        captured
            .halation
            .parameters(captured.composition.crop().size)
            .unwrap()
            .is_some()
    );
    project.output.halation.strength = 0.4;
    assert_eq!(
        captured.halation.strength, 0.3,
        "captured recipe stays immutable"
    );
    assert_ne!(captured, project.image_recipe(id).unwrap());
    project.output.halation.enabled = false;
    assert_ne!(clean, project.image_recipe(id).unwrap());
    assert!(!project.image_recipe(id).unwrap().halation.enabled);
}

#[test]
fn grain_is_captured_immutably_without_changing_neutral_repair_identity() {
    let (mut project, id) = project();
    let clean = project.image_recipe(id).unwrap();
    let film = antitone::grain::GrainPreset::ThirtyFiveMm.settings();
    project.output.grain = film;
    let captured = project.image_recipe(id).unwrap();
    assert_eq!(captured.grain, film);
    assert_ne!(clean, captured);
    assert_eq!(clean.neutral, captured.neutral);
    project.output.grain.image_sigma *= 1.5;
    assert_ne!(
        captured,
        project.image_recipe(id).unwrap(),
        "independently authored image sigma is rendering identity"
    );
    project.output.grain.image_sigma = film.image_sigma;
    project.output.grain.amount *= 2.;
    let edited = project.image_recipe(id).unwrap();
    assert_ne!(captured, edited);
    assert_eq!(captured.grain, film);
    assert_eq!(captured.neutral.source, id);
    project.output.grain.enabled = false;
    assert!(captured.grain.enabled);
}

#[test]
fn emulation_resolves_from_the_captured_crop_not_its_source_bounds_or_orientation() {
    use antitone::grain::GrainSettings;
    let (mut project, id) = project();
    project.output.grain = GrainSettings {
        enabled: true,
        size: 0.040,
        image_sigma: 0.040,
        ..Default::default()
    };
    let full = project.image_recipe(id).unwrap();
    project.source_mut(id).unwrap().crop = Some(antitone::crop::Crop {
        center: [444., 300.],
        size: [432, 288],
        angle: 0.21,
    });
    let cropped = project.image_recipe(id).unwrap();
    assert_eq!(
        full.neutral, cropped.neutral,
        "crop scale does not alter captured input interpretation"
    );
    let check = |recipe: &antitone::image_recipe::ImageRecipe, expected: f64| {
        let parameters = recipe
            .grain
            .parameters(recipe.composition.crop().size)
            .unwrap()
            .unwrap();
        assert!((parameters.image_mtf_sigma - expected).abs() < 1e-12);
        assert!((parameters.grain_size - expected * 1.375).abs() < 1e-12);
    };
    check(&full, 0.2304);
    check(&cropped, 0.1152);
    project.orientation.turn_clockwise();
    check(&project.image_recipe(id).unwrap(), 0.1152);
    project.source_mut(id).unwrap().development.active_area[3] = 588;
    check(&project.image_recipe(id).unwrap(), 0.1152);
    project.source_mut(id).unwrap().crop = None;
    check(&cropped, 0.1152); // An in-flight recipe retains its accepted crop.
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
            [64, 0],
        )
        .unwrap();
    let none = project.image_recipe(id).unwrap();
    let lut = Arc::new(Lut3D::new(2, [0.; 3], [1.; 3], vec![[0.4, 0.3, 0.2]; 8]).unwrap());
    project.output.lut = Some(Arc::new(
        LookAsset::new("First".into(), LookOrigin::Imported, lut.clone()).unwrap(),
    ));
    let looked = project.image_recipe(id).unwrap();
    assert_ne!(none, looked);
    assert_eq!(none.neutral, looked.neutral);
    assert_eq!(none.repairs, looked.repairs);
    assert!(looked.retained_bytes() >= none.retained_bytes() + lut.byte_len());
    project.output.lut = Some(Arc::new(
        LookAsset::new("Renamed".into(), LookOrigin::Imported, lut.clone()).unwrap(),
    ));
    assert_eq!(looked, project.image_recipe(id).unwrap());
    project.output.lut = None;
    assert_eq!(none, project.image_recipe(id).unwrap());
    assert_eq!(looked.luts.lut.as_ref().unwrap().lut().id(), lut.id());
}
