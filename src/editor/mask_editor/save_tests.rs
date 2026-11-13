use super::*;
use crate::application::test_support::with_application;
use std::{path::PathBuf, time::Duration};

fn relative_editor() -> (Editor, tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let old = directory.path().join("old");
    let new = directory.path().join("new");
    std::fs::create_dir_all(old.join("scans")).unwrap();
    std::fs::create_dir_all(&new).unwrap();
    let photo = old.join("scans/frame.dng");
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        &photo,
    )
    .unwrap();
    let mut editor = crate::editor::tests::authored_fixture(&[photo.to_str().unwrap()]);
    editor.load_input(
        crate::processing::Calibration {
            dmin: DVec3::splat(0.2),
            dmax: DVec3::splat(0.005),
        }
        .into(),
    );
    editor.document.source = false;
    editor.tab = 1;
    editor.document.path = Some(old.join("grade.antitone"));
    let source = editor.document.visit.current_source.unwrap();
    editor.document.project.source_mut(source).unwrap().path = "scans/frame.dng".into();
    editor.document.capture_source_files().unwrap();
    (editor, directory, new.join("grade.antitone"))
}

fn geometry(editor: &Editor, radial: bool) -> AdjustmentLayerCoverage {
    let area = editor.selected_area().unwrap();
    let point = |x: f64, y: f64| {
        SourcePoint([
            f64::from(area.x) + f64::from(area.width) * x,
            f64::from(area.y) + f64::from(area.height) * y,
        ])
    };
    if radial {
        AdjustmentLayerCoverage::Radial {
            center: point(0.5, 0.5),
            radii: [f64::from(area.width) * 0.2, f64::from(area.height) * 0.2],
            angle: 0.,
            feather: DEFAULT_RADIAL_FEATHER,
        }
    } else {
        AdjustmentLayerCoverage::Linear {
            start: point(0.2, 0.3),
            end: point(0.7, 0.8),
        }
    }
}

#[test]
fn save_as_preserves_private_mask_intent_and_release_keeps_rebased_paths_and_later_grades() {
    for creating in [false, true] {
        for radial in [false, true] {
            let (mut editor, _directory, destination) = relative_editor();
            let source = editor.document.visit.current_source.unwrap();
            let id = editor
                .document
                .project
                .source(source)
                .unwrap()
                .adjustment_layers
                .as_ref()
                .unwrap()
                .layers[0]
                .id;
            let mask = geometry(&editor, radial);
            if !creating {
                editor
                    .edit_adjustment_layers(|layers| {
                        layers.layers[0].adjustment.mask = mask;
                        Ok(())
                    })
                    .unwrap();
            }
            if radial {
                editor.begin_radial_mask((!creating).then_some(id));
            } else {
                editor.begin_linear_mask((!creating).then_some(id));
            }
            let mask = if let AdjustmentLayerCoverage::Linear { start, end } = mask {
                AdjustmentLayerCoverage::Linear {
                    start,
                    end: SourcePoint([end.0[0] + 2., end.0[1]]),
                }
            } else if let AdjustmentLayerCoverage::Radial {
                center,
                radii,
                angle,
                ..
            } = mask
            {
                AdjustmentLayerCoverage::Radial {
                    center,
                    radii,
                    angle,
                    feather: 0.4,
                }
            } else {
                unreachable!()
            };
            if creating {
                editor.mask_creation().unwrap().borrow_mut().mask = Some(mask);
            } else {
                editor.document.visit.masks.gesture_mut().unwrap().mask = Some(mask);
            }
            let accepted = editor.document.state.project.clone();
            let locator = editor.document.source_path(source).unwrap();
            let file = editor.document.file_key(&locator);
            let visit = editor.document.visit.view;
            with_application(editor, |app| {
                app.enter(|editor, cx| {
                    editor.save_project(destination.clone(), crate::project::Save::New, None, cx)
                });
                assert!(app.read(|editor| editor.document.write.pending()));
                app.run_until(Duration::from_secs(15), |editor| {
                    !editor.document.write.pending()
                });
                let saved = crate::project::Project::load(&destination).unwrap();
                app.enter(|editor, _| {
                    assert!(editor.masking());
                    assert_eq!(editor.document.visit.view, visit);
                    assert!(!editor.document.dirty());
                    assert_ne!(
                        editor.document.project.source(source).unwrap().path,
                        accepted.source(source).unwrap().path
                    );
                    assert_eq!(
                        editor
                            .document
                            .file_key(&editor.document.source_path(source).unwrap()),
                        file
                    );
                    assert_eq!(
                        editor
                            .document
                            .source_path(source)
                            .unwrap()
                            .canonicalize()
                            .unwrap(),
                        locator.canonicalize().unwrap()
                    );
                    assert_eq!(
                        saved.source(source).unwrap().adjustment_layers,
                        accepted.source(source).unwrap().adjustment_layers
                    );
                    let mut edit = editor.edit_photo("Later grade").unwrap();
                    edit.update(|project| {
                        let layer = &mut project
                            .source_mut(source)
                            .unwrap()
                            .adjustment_layers
                            .as_mut()
                            .unwrap()
                            .layers[0];
                        layer.adjustment.photo.exposure = 0.75;
                        layer.name = "Later name".into();
                    });
                    editor.commit_project(edit).unwrap();
                    if creating {
                        editor
                            .with_current_tool(|tool, editor| {
                                (tool as &mut dyn std::any::Any)
                                    .downcast_mut::<CreationTool>()
                                    .unwrap()
                                    .finish(editor);
                            })
                            .unwrap();
                    } else {
                        assert!(editor.mask_preview_source().is_some());
                        editor.finish_mask(true);
                    }
                    assert!(!editor.masking(), "{}", editor.status);
                    assert!(editor.document.dirty());
                    let current = editor.document.project.source(source).unwrap();
                    assert_eq!(current.path, saved.source(source).unwrap().path);
                    let layers = current.adjustment_layers.as_ref().unwrap();
                    assert_eq!(layers.layers[0].adjustment.photo.exposure, 0.75);
                    assert_eq!(layers.layers[0].name, "Later name");
                    assert_eq!(layers.layers.last().unwrap().adjustment.mask, mask);
                    assert_eq!(layers.layers.len(), if creating { 2 } else { 1 });
                });
            });
        }
    }
}

#[test]
fn creation_allocates_against_current_layers_after_history_without_reinstalling_a_snapshot() {
    pollster::block_on(async {
        let (mut editor, _, _) = processing::tests::fixture().await;
        let source = editor.document.visit.current_source.unwrap();
        editor
            .edit_adjustment_layers(|layers| {
                layers.add(
                    "Restored layer".into(),
                    AdjustmentLayer::global(AdjustmentLayerSettings::default()),
                )?;
                Ok(())
            })
            .unwrap();
        editor.restore_history(history::Direction::Undo, &UiContext::render_only());
        editor.begin_linear_mask(None);
        editor.mask_creation().unwrap().borrow_mut().mask = Some(geometry(&editor, false));
        editor.restore_history(history::Direction::Redo, &UiContext::render_only());
        let before = editor
            .document
            .project
            .source(source)
            .unwrap()
            .adjustment_layers
            .clone()
            .unwrap();
        editor
            .with_current_tool(|tool, editor| {
                (tool as &mut dyn std::any::Any)
                    .downcast_mut::<CreationTool>()
                    .unwrap()
                    .finish(editor);
            })
            .unwrap();
        let layers = editor
            .document
            .project
            .source(source)
            .unwrap()
            .adjustment_layers
            .as_ref()
            .unwrap();
        assert_eq!(layers.next_id, before.next_id + 1);
        assert_eq!(layers.layers.len(), before.layers.len() + 1);
        let added = layers
            .layers
            .iter()
            .find(|layer| layer.id == AdjustmentLayerId(before.next_id))
            .unwrap();
        assert!(matches!(
            added.adjustment.mask,
            AdjustmentLayerCoverage::Linear { .. }
        ));
        for layer in &before.layers {
            assert_eq!(
                layers.layers.iter().find(|current| current.id == layer.id),
                Some(layer)
            );
        }
    });
}
