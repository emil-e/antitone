use super::*;

#[test]
fn created_linear_and_radial_locals_survive_normal_save_open_source_return_and_gpu_exports() {
    workflow::calibrated_workflow(|mut harness, _, directory| {
        stage(&mut harness, "Roll");
        workflow::enter_numeric(&mut harness, "Gamma", "0.6");
        harness.read(|editor| assert_eq!(editor.roll.gamma, 0.6));
        stage(&mut harness, "Photo");
        let selected = harness.read(|editor| editor.current_source.unwrap());
        for (radial, exposure) in [(false, "0.35"), (true, "-0.125")] {
            arm(&mut harness, radial);
            let geometry = harness.read(|editor| editor.last_layout.unwrap().0);
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
        harness.update(|editor| {
            let area = editor.document.visit.ready.as_ref().unwrap().area;
            editor.crop = Some(Crop::from_region(area).straightened(area, 0.11).unwrap());
            editor.photo_orientation.turn_clockwise();
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
        assert_eq!(snapshot.locals().len(), 2);
        assert_eq!(snapshot.locals()[0].photo.exposure, 0.35);
        assert_eq!(snapshot.locals()[1].photo.exposure, -0.125);
        assert!(matches!(
            snapshot.locals()[0].mask,
            GradientMask::Linear { .. }
        ));
        assert!(matches!(
            snapshot.locals()[1].mask,
            GradientMask::Radial { .. }
        ));
        let path = directory.path().join("local-tools.json");
        harness.update(|editor| editor.queue_save(path.clone(), crate::project::Save::New, None));
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| assert!(!editor.document.dirty(), "{}", editor.status));
        let saved = harness.read(|editor| editor.document.project.clone());
        assert_eq!(crate::project::Project::load(&path).unwrap(), saved);
        harness.update(|editor| editor.start_document(Preparation::Open(path)));
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.document_pending.is_none()
        });
        harness.read(|editor| assert_eq!(editor.document.project, saved, "{}", editor.status));
        harness.get_by_label("working-photo.dng").click();
        harness.step();
        wait_for(&mut harness, |editor| !editor.busy);
        harness.read(|editor| assert_eq!(editor.current_source, Some(selected)));
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
                editor.adjustment_target(),
                super::super::super::adjustments::Target::Global
            )
        });
        harness.read(|editor| assert_eq!(editor.document.project, saved));
        workflow::wait_settled(&mut harness);
        let size = harness.read(|editor| {
            Composition::with_crop(
                editor.document.visit.ready.as_ref().unwrap().area,
                editor.roll_orientation,
                editor.photo_orientation,
                editor.crop,
            )
            .unwrap()
            .size()
        });
        for (name, format) in [
            ("locals-srgb.tif", OutputFormat::Tiff(OutputTarget::Srgb)),
            ("locals-p3.tif", OutputFormat::Tiff(OutputTarget::DisplayP3)),
            ("locals.exr", OutputFormat::Exr(ExrPrecision::Full)),
        ] {
            let destination = directory.path().join(name);
            harness.update(|editor| {
                editor.export(
                    destination.clone(),
                    format,
                    crate::export::Overwrite::Refuse,
                )
            });
            harness.read(|editor| {
                assert!(
                    editor.export_operation.pending.is_some(),
                    "{}",
                    editor.status
                )
            });
            wait_for(&mut harness, |editor| {
                !editor.busy && editor.export_operation.pending.is_none()
            });
            harness.read(|editor| assert!(destination.exists(), "{}", editor.status));
            match format {
                OutputFormat::Tiff(_) => {
                    let mut image =
                        tiff::decoder::Decoder::new(std::fs::File::open(destination).unwrap())
                            .unwrap();
                    assert_eq!(image.dimensions().unwrap(), (size[0], size[1]));
                    assert!(
                        image
                            .get_tag_ascii_string(tiff::tags::Tag::ImageDescription)
                            .unwrap()
                            .contains(crate::local_adjustments::CONTRACT)
                    );
                }
                OutputFormat::Exr(_) => {
                    let image = exr::prelude::read_first_flat_layer_from_file(destination).unwrap();
                    assert_eq!(
                        (image.layer_data.size.0, image.layer_data.size.1),
                        (size[0] as usize, size[1] as usize)
                    );
                    assert_eq!(
                        image
                            .layer_data
                            .attributes
                            .other
                            .get(&exr::prelude::Text::from(
                                "antitoneLocalAdjustmentProcessing"
                            )),
                        Some(&exr::prelude::AttributeValue::Text(
                            exr::prelude::Text::from(crate::local_adjustments::CONTRACT)
                        ))
                    );
                }
            }
            harness.read(|editor| assert_eq!(editor.document.project, saved));
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
