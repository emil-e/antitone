use super::*;

fn center_pixel(path: &Path) -> glam::DVec3 {
    let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
    let size = image.layer_data.size;
    let index = size.0 * (size.1 / 2) + size.0 / 2;
    glam::DVec3::from_array(["R", "G", "B"].map(|name| {
        let channel = image
            .layer_data
            .channel_data
            .list
            .iter()
            .find(|channel| channel.name.to_string() == name)
            .unwrap();
        let exr::prelude::FlatSamples::F32(values) = &channel.sample_data else {
            panic!("full-float EXR expected");
        };
        values[index] as f64
    }))
}

#[test]
fn cold_batch_pixels_match_each_sources_accepted_grade_not_active_or_later_settings() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let project = editor.document.state.project.clone();
    let wb = project.development().as_ref().unwrap().fixed_wb;
    let calibration = project.calibration().unwrap().unwrap();
    let mut scans = Vec::new();
    let mut grades = Vec::new();
    for id in &project.order {
        // Independent source loading and scalar processing: no export request,
        // active visit or production GPU output supplies the expected pixels.
        let scan =
            crate::scan::Scan::open(&editor.document.source_path(*id).unwrap(), Some(wb)).unwrap();
        grades.push(project.source(*id).unwrap().grade_snapshot().unwrap());
        scans.push(scan);
    }
    let scalar = |source: usize, grade: usize| {
        let scan = &scans[source];
        let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
            scan.processing_calibration(calibration).unwrap(),
            project.roll,
            grades[grade].clone(),
            scan.camera_to_ap1,
        )
        .unwrap();
        crate::color::ap1_to_ap0()
            * pipeline
                .apply(scan.pixel(200, 200).unwrap(), [200, 200])
                .unwrap()
                .rgb
    };
    let expected = [scalar(0, 0), scalar(1, 1)];
    for wrong in [scalar(0, 1), scalar(1, 0)] {
        assert!(
            (expected[1] - wrong).abs().max_element() > 0.01,
            "fixture must distinguish both wrong-source and wrong-grade leakage"
        );
    }
    let mut settings = draft(directory.path());
    settings.precision = ExrPrecision::Full;
    let request = editor.batch_input(&settings).unwrap();
    editor.document.roll.exposure += 1.;
    editor.document.visit.adjustment_layer_settings.exposure += 2.;
    editor.sync_document();
    editor
        .document
        .project
        .source_mut(project.order[1])
        .unwrap()
        .adjustment_layers
        .as_mut()
        .unwrap()
        .layers[0]
        .adjustment
        .photo
        .exposure = -2.;
    let services = editor.export_services.take().unwrap();
    drop(editor);
    let summary = pollster::block_on(services.batch(request, Cancellation::default(), |_, _| {}));
    assert_eq!((summary.exported(), summary.failed()), (2, 0));
    for (index, name) in ["roll-0.exr", "roll-1.exr"].into_iter().enumerate() {
        let actual = center_pixel(&directory.path().join(name));
        for channel in 0..3 {
            let reference = expected[index][channel];
            assert!(
                (actual[channel] - reference).abs()
                    <= crate::working_precision::allowance(reference),
                "{name} channel={channel} actual={} scalar={reference}",
                actual[channel]
            );
        }
    }
}
