use super::*;

#[test]
fn gradient_layers_then_global_finish_survive_save_open_source_return_and_gpu_exports() {
    workflow::calibrated_workflow(|mut harness, _, directory| {
        stage(&mut harness, "Roll");
        workflow::enter_numeric(&mut harness, "Gamma", "0.6");
        harness.read(|editor| assert_eq!(editor.gamma(), 0.6));
        stage(&mut harness, "Photo");
        let selected = harness.read(|editor| editor.document.visit.current_source.unwrap());
        for (radial, exposure) in [(false, "0.35"), (true, "-0.125")] {
            arm(&mut harness, radial);
            let geometry = harness.read(|editor| editor.active_canvas().last_layout.unwrap().0);
            drag(
                &mut harness,
                geometry.image.min + egui::vec2(30., 40.),
                geometry.image.max - egui::vec2(30., 40.),
            );
            harness.read(|editor| {
                assert!(
                    editor.document.visit.mask_edit.is_none(),
                    "{}",
                    editor.status
                )
            });
            workflow::enter_numeric(&mut harness, "Exposure", exposure);
        }
        click(&mut harness, "Add global");
        workflow::enter_numeric(&mut harness, "Contrast", "0.75");
        harness.update(|editor| {
            let area = editor.document.visit.canvas.ready.as_ref().unwrap().area;
            editor.document.visit.crop =
                Some(Crop::from_region(area).straightened(area, 0.11).unwrap());
            editor.document.visit.photo_orientation.turn_clockwise();
            editor.sync_document();
        });
        harness.step();
        workflow::wait_settled(&mut harness);
        let snapshot = harness.read(|editor| {
            editor
                .document
                .project
                .source(selected)
                .unwrap()
                .grade_snapshot()
                .unwrap()
        });
        assert_eq!(snapshot.layers().len(), 4);
        assert_eq!(
            snapshot.layers()[3].mask,
            AdjustmentLayerCoverage::Global {}
        );
        assert_eq!(snapshot.layers()[3].photo.contrast, 0.75);
        assert_eq!(snapshot.layers()[1].photo.exposure, 0.35);
        assert_eq!(snapshot.layers()[2].photo.exposure, -0.125);
        assert!(matches!(
            snapshot.layers()[1].mask,
            AdjustmentLayerCoverage::Linear { .. }
        ));
        assert!(matches!(
            snapshot.layers()[2].mask,
            AdjustmentLayerCoverage::Radial { .. }
        ));
        let path = directory.path().join("local-tools.json");
        harness.enter(|editor, cx| {
            editor.save_project(path.clone(), crate::project::Save::New, None, cx)
        });
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| assert!(!editor.document.dirty(), "{}", editor.status));
        let saved = harness.read(|editor| editor.document.state.project.clone());
        assert_eq!(crate::project::Project::load(&path).unwrap(), saved);
        harness.enter(|editor, cx| editor.start_document(Preparation::Open(path), cx));
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness
            .read(|editor| assert_eq!(editor.document.state.project, saved, "{}", editor.status));
        harness.get_by_label("working-photo.dng").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.busy);
        harness.read(|editor| assert_eq!(editor.document.visit.current_source, Some(selected)));
        harness.read(|editor| {
            assert_eq!(
                editor
                    .document
                    .project
                    .source(selected)
                    .unwrap()
                    .grade_snapshot()
                    .unwrap(),
                snapshot
            )
        });
        harness.get_by_label("patches.dng").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.busy);
        harness.get_by_label("working-photo.dng").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.busy);
        harness.read(|editor| {
            assert_eq!(
                editor.adjustment_layer_target(),
                super::super::super::adjustment_layers::Target::Layer(
                    crate::project::AdjustmentLayerId(4)
                )
            )
        });
        harness.read(|editor| assert_eq!(editor.document.state.project, saved));
        workflow::wait_settled(&mut harness);
        let size = harness.read(|editor| {
            Composition::with_crop(
                editor.document.visit.canvas.ready.as_ref().unwrap().area,
                editor.document.roll_orientation,
                editor.document.visit.photo_orientation,
                editor.document.visit.crop,
            )
            .unwrap()
            .size()
        });
        for (name, format) in [
            ("layers-srgb.tif", OutputFormat::Tiff(OutputTarget::Srgb)),
            ("layers-p3.tif", OutputFormat::Tiff(OutputTarget::DisplayP3)),
            ("layers.exr", OutputFormat::Exr(ExrPrecision::Full)),
        ] {
            let destination = directory.path().join(name);
            harness.enter(|editor, cx| {
                editor.export(
                    destination.clone(),
                    format,
                    crate::export::Overwrite::Refuse,
                    cx,
                )
            });
            harness.read(|editor| {
                assert!(
                    editor.document.export_operation.pending.is_some(),
                    "{}",
                    editor.status
                )
            });
            wait_for(&mut harness, |editor| {
                !editor.busy && editor.document.export_operation.pending.is_none()
            });
            harness.read(|editor| assert!(destination.exists(), "{}", editor.status));
            match format {
                OutputFormat::Tiff(_) => {
                    let mut image =
                        tiff::decoder::Decoder::new(std::fs::File::open(destination).unwrap())
                            .unwrap();
                    assert_eq!(image.dimensions().unwrap(), (size[0], size[1]));
                }
                OutputFormat::Exr(_) => {
                    let image = exr::prelude::read_first_flat_layer_from_file(destination).unwrap();
                    assert_eq!(
                        (image.layer_data.size.0, image.layer_data.size.1),
                        (size[0] as usize, size[1] as usize)
                    );
                }
            }
            harness.read(|editor| assert_eq!(editor.document.state.project, saved));
            harness.read(|editor| {
                assert_eq!(
                    editor
                        .document
                        .project
                        .source(selected)
                        .unwrap()
                        .grade_snapshot()
                        .unwrap(),
                    snapshot
                )
            });
        }
    });
}
