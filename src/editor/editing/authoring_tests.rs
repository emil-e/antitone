use super::*;
use crate::adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage};
use crate::geometry::SourcePoint;

fn change(editor: &mut Editor, write: impl FnOnce(&mut crate::project::Project)) {
    editor.roll_control(
        ui::ControlResponse::action(egui::Id::new("authoring"), true),
        &UiContext::render_only(),
        write,
    );
}

fn author_supported_controls(editor: &mut Editor) {
    change(editor, |project| project.roll.exposure = 1.25);
    assert_eq!(
        editor.document.project.roll.exposure, 1.25,
        "{}",
        editor.status
    );
    change(editor, |project| {
        if let InputSettings::Negative {
            gamma, response, ..
        } = &mut project.input
        {
            *gamma = 0.9;
            response.toe = 1.1;
        }
    });
    assert_eq!(editor.input().gamma(), 0.9, "{}", editor.status);
    let balance = Balance {
        warmth: 0.15,
        tint: -0.08,
    };
    white_balance::set_roll(editor, balance).unwrap();
    assert_eq!(editor.document.project.roll.wb, balance);

    editor.photo_control(
        ui::ControlResponse::action(egui::Id::new("layer-authoring"), true),
        |layer| layer.photo.exposure = 0.75,
    );
    assert_eq!(editor.photo_settings().exposure, 0.75, "{}", editor.status);
    editor
        .edit_adjustment_layers(|layers| {
            layers.add(
                "Independent".into(),
                AdjustmentLayer::global(Default::default()),
            )?;
            Ok(())
        })
        .unwrap();
    let target = (
        editor.document.visit.current_source.unwrap(),
        editor.adjustment_layer_target(),
    );
    white_balance::set_layer(editor, balance, target).unwrap();
    assert_eq!(editor.photo_settings().wb, balance);

    let accepted = editor.document.state.project.clone();
    change(editor, |project| project.roll.exposure = f64::INFINITY);
    assert_eq!(editor.document.state.project, accepted);
    change(editor, |project| {
        if let InputSettings::Negative { gamma, .. } = &mut project.input {
            *gamma = 0.;
        }
    });
    assert_eq!(editor.document.state.project, accepted);
    editor.photo_control(
        ui::ControlResponse::action(egui::Id::new("invalid-layer"), true),
        |layer| layer.photo.contrast = f64::NAN,
    );
    assert_eq!(editor.document.state.project, accepted);
}

#[test]
fn authoring_supported_controls_does_not_require_complete_or_compatible_calibration() {
    pollster::block_on(async {
        for missing in [true, false] {
            let (mut editor, _, _) = processing::tests::fixture().await;
            if let InputSettings::Negative { dmin, dmax, .. } = &mut editor.document.project.input {
                *dmax = if missing { None } else { *dmin };
            }
            author_supported_controls(&mut editor);
            assert!(
                editor
                    .positive_processing(
                        editor.input(),
                        editor.document.project.roll,
                        editor.photo_settings()
                    )
                    .is_err()
            );
            let accepted = editor.document.state.project.clone();
            editor.restore_history(history::Direction::Undo, &UiContext::render_only());
            assert_ne!(editor.document.state.project, accepted);
            editor.restore_history(history::Direction::Redo, &UiContext::render_only());
            assert_eq!(editor.document.state.project, accepted);
        }
    });
}

#[test]
fn authoring_ignores_unrenderable_layers_and_can_remove_them() {
    pollster::block_on(async {
        for offscreen in [true, false] {
            let (mut editor, device, _) = processing::tests::fixture().await;
            let current = editor.document.visit.current_source.unwrap();
            let source = if offscreen {
                let mut other = editor.document.project.source(current).unwrap().clone();
                other.id = SourceId(0);
                other.path = "offscreen-authoring.ARW".into();
                editor.document.project.add_source(other).unwrap()
            } else {
                current
            };
            let layer = editor
                .document
                .project
                .source_mut(source)
                .unwrap()
                .adjustment_layers
                .as_mut()
                .unwrap()
                .add(
                    "Unrenderable".into(),
                    AdjustmentLayer {
                        mask: AdjustmentLayerCoverage::Radial {
                            center: SourcePoint([121., 74.]),
                            radii: [f64::MIN_POSITIVE, 1.],
                            angle: 0.,
                            feather: 0.5,
                        },
                        ..AdjustmentLayer::global(AdjustmentLayerSettings {
                            exposure: 1.,
                            ..Default::default()
                        })
                    },
                )
                .unwrap();
            assert!(
                processing::validate_source(
                    Some(&device),
                    editor.input(),
                    editor.document.project.roll,
                    editor.document.project.source(source).unwrap()
                )
                .is_err()
            );
            author_supported_controls(&mut editor);
            if !offscreen {
                assert!(
                    editor
                        .positive_processing(
                            editor.input(),
                            editor.document.project.roll,
                            editor.photo_settings()
                        )
                        .is_err()
                );
                editor
                    .edit_adjustment_layers(|layers| layers.remove(layer))
                    .unwrap();
            }
            editor
                .positive_processing(
                    editor.input(),
                    editor.document.project.roll,
                    editor.photo_settings(),
                )
                .unwrap();
        }
    });
}
