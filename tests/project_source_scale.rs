#![cfg(feature = "project")]

use antitone::{
    processing::{CAMERA_INPUT_FLOOR, InputSettings},
    project::*,
};

fn digital(wb: [f64; 4], gain: f64) -> Project {
    let mut project = Project {
        input: InputSettings::Digital,
        ..Default::default()
    };
    project
        .add_source(Source {
            id: SourceId(0),
            path: "source.arw".into(),
            reference_only: false,
            development: SourceDevelopment {
                decode: Development::new("0.22.0".into(), wb),
                lens_correction: None,
                dng_development: None,
                dimensions: [64; 2],
                active_area: [0, 0, 64, 64],
                camera_model: "authored".into(),
                camera_to_ap1: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                input_floor: CAMERA_INPUT_FLOOR * gain,
            },
            adjustment_layers: None,
            repairs: Default::default(),
            orientation: Default::default(),
            crop: None,
        })
        .unwrap();
    project
}

#[test]
fn native_source_precision_tracks_green_relative_wb_without_altering_grades() {
    for (wb, gain) in [
        ([4., 1., 2., 1.], 4.),
        ([8., 2., 4., 2.], 4.),
        ([1., 1., 0.5, 3.5], 3.5),
    ] {
        let project = digital(wb, gain);
        project.validate().unwrap();
        project.require_runtime().unwrap();
        assert_eq!(
            project.sources[0].development.decode.contract,
            DEVELOPMENT_CONTRACT
        );
        assert_eq!(project.roll.exposure, 0.);
        assert!(
            project.sources[0]
                .grade_snapshot()
                .unwrap()
                .layers()
                .is_empty()
        );
        let loaded: Project =
            serde_json::from_str(&serde_json::to_string(&project).unwrap()).unwrap();
        assert_eq!(loaded, project);
        loaded.require_runtime().unwrap();
        let mut wrong_floor = loaded.clone();
        wrong_floor.sources[0].development.input_floor = CAMERA_INPUT_FLOOR;
        assert!(wrong_floor.validate().is_err());
        let mut corrected = loaded;
        corrected.sources[0].development.lens_correction = Some("a".repeat(64));
        corrected.validate().unwrap();
        corrected.sources[0].development.input_floor = CAMERA_INPUT_FLOOR;
        assert!(corrected.validate().is_err());
    }
}

#[test]
fn green_max_wb_retains_the_native_quantum_and_manual_grades() {
    for wb in [[1.; 4], [0.5, 1., 0.25, 1.], [1., 2., 1.5, 2.]] {
        let mut project = digital(wb, 1.);
        assert_eq!(
            project.sources[0].development.decode.contract,
            DEVELOPMENT_CONTRACT
        );
        project.roll.exposure = -0.5;
        project.sources[0].adjustment_layers = Some(AdjustmentLayers::with_global(
            antitone::adjustment_layers::AdjustmentLayerSettings {
                exposure: 1.25,
                ..Default::default()
            },
        ));
        project.require_runtime().unwrap();
        assert_eq!(project.roll.exposure, -0.5);
        assert_eq!(
            project.sources[0].grade_snapshot().unwrap().layers()[0]
                .photo
                .exposure,
            1.25
        );
    }
}
