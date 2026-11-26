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

#[cfg(feature = "ocio-reference")]
#[test]
fn digital_batch_reuses_ready_exposure_and_resolves_an_unvisited_photo_with_frozen_edits() {
    use crate::editor::tests::{authored_fixture, complete_document_operation};
    use crate::processing::InputSettings;

    let files = [
        crate::exposure_fixture::neutral_dng(0.7),
        crate::exposure_fixture::neutral_dng(1.3),
    ];
    let mut editor = authored_fixture(&[]);
    editor.document.project.input = InputSettings::Digital;
    editor = complete_document_operation(editor, Duration::from_secs(30), |editor, cx| {
        editor.add_raws(
            files.iter().map(|file| file.path().to_owned()).collect(),
            cx,
        );
    });
    let ids = editor.document.project.order.clone();
    let first = editor
        .document
        .exposure_slot(&editor.document.initialization, ids[0]);
    let second = editor
        .document
        .exposure_slot(&editor.document.initialization, ids[1]);
    let first_ev = first
        .get()
        .expect("selected source has resolved exposure")
        .ev();
    assert!((first_ev - 0.7).abs() < 0.06);
    assert!(
        second.get().is_none(),
        "second photo must require cold acquisition"
    );

    let directory = tempfile::tempdir().unwrap();
    let mut settings = draft(directory.path());
    settings.precision = ExrPrecision::Full;
    let request = editor.capture_export(&settings).unwrap();
    assert!(matches!(request.photos[0].input, Input::Ready(_)));
    assert!(matches!(request.photos[1].input, Input::Acquire(_)));
    let paths: Vec<_> = request
        .photos
        .iter()
        .map(|photo| photo.path.clone())
        .collect();
    editor.document.project.roll.exposure = 3.;
    let services = editor.output_services.take().unwrap();
    drop(editor);
    let summary =
        pollster::block_on(services.export_batch(request, Cancellation::default(), |_, _| {}));
    assert_eq!((summary.exported(), summary.failed()), (2, 0));
    assert_eq!(first.get().unwrap().ev(), first_ev);
    assert!((second.get().unwrap().ev() - 1.3).abs() < 0.06);
    for (path, ev) in paths.iter().zip([0.7_f64, 1.3]) {
        let expected = 6000. / 65535. * ev.exp2();
        assert!((center_pixel(path) / expected - 1.).abs().max_element() < 0.05);
    }
}

#[test]
fn cold_batch_pixels_match_each_sources_accepted_grade_not_active_or_later_settings() {
    let mut editor = fixture();
    let directory = tempfile::tempdir().unwrap();
    let project = editor.document.state.project.clone();
    let wb = project.acquisition_wb();
    let calibration = project.calibration().unwrap().unwrap();
    let mut scans = Vec::new();
    let mut grades = Vec::new();
    for id in &project.order {
        // Independent source loading and scalar processing: no export request,
        // active visit or production GPU output supplies the expected pixels.
        let scan =
            crate::scan::DevelopedSource::open(&editor.document.source_path(*id).unwrap(), wb)
                .unwrap();
        grades.push(project.source(*id).unwrap().grade_snapshot().unwrap());
        scans.push(scan);
    }
    let scalar = |source: usize, grade: usize| {
        let scan = &scans[source];
        let pipeline = crate::adjustment_layers::AdjustmentLayerPipeline::new(
            (calibration).validate().unwrap(),
            project.roll,
            grades[grade].clone(),
            scan.camera_to_ap1(),
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
    let request = editor.capture_export(&settings).unwrap();
    editor.document.project.roll.exposure += 1.;
    editor.photo_settings_mut().exposure += 2.;

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
    let services = editor.output_services.take().unwrap();
    drop(editor);
    let summary =
        pollster::block_on(services.export_batch(request, Cancellation::default(), |_, _| {}));
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
