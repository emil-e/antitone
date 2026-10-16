use super::*;

#[test]
fn created_linear_and_radial_locals_survive_normal_save_open_source_return_and_gpu_exports() {
    let (mut harness, _, directory) = workflow::calibrated_workflow();
    stage(&mut harness, "Roll");
    workflow::enter_numeric(&mut harness, "Gamma", "0.6");
    assert_eq!(harness.state().roll.gamma, 0.6);
    stage(&mut harness, "Photo");
    let selected = harness.state().current_source.unwrap();
    for (radial, exposure) in [(false, "0.35"), (true, "-0.125")] {
        arm(&mut harness, radial);
        let geometry = harness.state().last_layout.unwrap().0;
        drag(
            &mut harness,
            geometry.image.min + egui::vec2(30., 40.),
            geometry.image.max - egui::vec2(30., 40.),
        );
        assert!(
            harness.state().mask_edit.is_none(),
            "{}",
            harness.state().status
        );
        workflow::enter_numeric(&mut harness, "Exposure", exposure);
    }
    {
        let editor = harness.state_mut();
        let area = editor.ready.as_ref().unwrap().area;
        editor.crop = Some(Crop::from_region(area).straightened(area, 0.11).unwrap());
        editor.photo_orientation.turn_clockwise();
        editor.sync_document();
    }
    harness.step();
    workflow::wait_settled(&mut harness);
    let snapshot = harness
        .state()
        .document
        .project
        .source(selected)
        .unwrap()
        .grade_snapshot()
        .unwrap();
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
    harness
        .state_mut()
        .queue_save(path.clone(), crate::project::Save::New, None);
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    assert!(
        !harness.state().document.dirty(),
        "{}",
        harness.state().status
    );
    let saved = harness.state().document.project.clone();
    assert_eq!(crate::project::Project::load(&path).unwrap(), saved);
    harness.state_mut().start_document(DocumentWork::Open(path));
    wait_for(&mut harness, |editor| {
        !editor.busy && editor.document_pending.is_none()
    });
    assert_eq!(
        harness.state().document.project,
        saved,
        "{}",
        harness.state().status
    );
    harness.get_by_label("working-photo.dng").click();
    harness.step();
    wait_for(&mut harness, |editor| !editor.busy);
    assert_eq!(harness.state().current_source, Some(selected));
    assert_eq!(
        harness
            .state()
            .document
            .project
            .source(selected)
            .unwrap()
            .grade_snapshot()
            .unwrap(),
        snapshot
    );
    harness.get_by_label("patches.dng").click();
    harness.step();
    wait_for(&mut harness, |editor| !editor.busy);
    harness.get_by_label("working-photo.dng").click();
    harness.step();
    wait_for(&mut harness, |editor| !editor.busy);
    assert_eq!(
        harness.state().adjustment_target(),
        super::super::super::adjustments::Target::Global
    );
    assert_eq!(harness.state().document.project, saved);
    workflow::wait_settled(&mut harness);
    let editor = harness.state();
    let size = Composition::with_crop(
        editor.ready.as_ref().unwrap().area,
        editor.roll_orientation,
        editor.photo_orientation,
        editor.crop,
    )
    .unwrap()
    .size();
    for (name, format) in [
        ("locals-srgb.tif", OutputFormat::Tiff(OutputTarget::Srgb)),
        ("locals-p3.tif", OutputFormat::Tiff(OutputTarget::DisplayP3)),
        ("locals.exr", OutputFormat::Exr(ExrPrecision::Full)),
    ] {
        let destination = directory.path().join(name);
        harness.state_mut().export(
            destination.clone(),
            format,
            crate::export::Overwrite::Refuse,
        );
        assert!(
            harness.state().export_pending.is_some(),
            "{}",
            harness.state().status
        );
        wait_for(&mut harness, |editor| {
            !editor.busy && editor.export_pending.is_none()
        });
        assert!(destination.exists(), "{}", harness.state().status);
        match format {
            OutputFormat::Tiff(_) => {
                let mut image =
                    tiff::decoder::Decoder::new(std::fs::File::open(destination).unwrap()).unwrap();
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
        assert_eq!(harness.state().document.project, saved);
        assert_eq!(
            harness
                .state()
                .document
                .project
                .source(selected)
                .unwrap()
                .grade_snapshot()
                .unwrap(),
            snapshot
        );
    }
}
