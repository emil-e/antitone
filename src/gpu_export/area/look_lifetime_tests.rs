use super::*;
use crate::processing::Calibration;

#[test]
fn cold_job_retains_one_look_across_source_sizes_and_releases_it_on_success_or_stop() {
    let (device, queue, renderer) = super::super::tests::context();
    let gate = SubmissionGate::default();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let area = scan.area;
    let composition = Composition::with_crop(
        area,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [
                f64::from(area.x) + f64::from(area.width) / 2.,
                f64::from(area.y) + f64::from(area.height) / 2.,
            ],
            size: [area.width - 64, area.height - 64],
            angle: 0.17,
        }),
    )
    .unwrap();
    let calibration = Calibration {
        dmin: DVec3::splat(3000. / 4031.),
        dmax: DVec3::splat(128. / 4031.),
        input_floor: scan.input_floor,
    };
    let grade =
        AdjustmentLayerSnapshot::new(vec![crate::adjustment_layers::AdjustmentLayer::global(
            Default::default(),
        )])
        .unwrap();
    let look = Arc::new(
        crate::looks::LookAsset::new(
            "Lifetime fixture".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(
                crate::looks::Lut3D::new(
                    2,
                    [-2.; 3],
                    [2.; 3],
                    (0..8)
                        .map(|i| {
                            [
                                -2. + 4. * (i % 2) as f32,
                                -2. + 4. * ((i / 2) % 2) as f32,
                                -2. + 4. * (i / 4) as f32,
                            ]
                        })
                        .collect(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    );
    let request = |look| Request {
        film_emulation: Default::default(),
        film_seed: 17,
        scan: &scan,
        field: None,
        input: calibration.into(),
        roll: RollSettings::default(),
        grade: grade.clone(),
        repairs: &[],
        composition,
        format: Format::Tiff(OutputTarget::DisplayP3),
        look,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output.tif");
    let capture = |look| {
        let (result, pixels) = crate::export_timing::pixels::capture(|| {
            write(
                &renderer,
                &device,
                (&queue, &gate),
                request(look),
                (&path, Overwrite::Refuse),
                || Ok(()),
            )
        });
        assert!(pixels.is_some());
        assert!(!path.exists());
        result.unwrap()
    };
    let plain = capture(None);
    assert_eq!(plain.look_payload_bytes, 0);
    assert_eq!(renderer.look_uploads(), 0);
    let cold = capture(Some(look.clone()));
    assert!(
        cold.source_allocations > 1,
        "exercise source-dimension transitions, not a single binding"
    );
    assert!(cold.look_payload_bytes > 0);
    assert_eq!(
        renderer.look_uploads(),
        1,
        "one actual LUT allocation for the whole cold job"
    );
    assert_eq!(
        renderer.live_look_resources(),
        0,
        "successful job must not leave an optional renderer-owned strong reference"
    );
    let retained = renderer.look_resource(&device, &look).unwrap();
    let warm = renderer.look_uploads();
    capture(Some(look.clone()));
    assert_eq!(
        renderer.look_uploads(),
        warm,
        "reuse another live consumer's immutable resource"
    );
    drop(retained);
    assert_eq!(renderer.live_look_resources(), 0);
    std::fs::write(&path, b"preserve existing destination").unwrap();
    for after in [0, 2] {
        let mut checks = 0;
        let result = write(
            &renderer,
            &device,
            (&queue, &gate),
            request(Some(look.clone())),
            (&path, Overwrite::Confirmed),
            || {
                checks += 1;
                if checks > after {
                    Err(Error::Export("cooperative stop".into()))
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"preserve existing destination"
        );
        assert_eq!(
            std::fs::read_dir(directory.path()).unwrap().count(),
            1,
            "temporary output is retired on stop"
        );
        assert_eq!(
            renderer.live_look_resources(),
            0,
            "job-only look ownership must unwind on stop"
        );
    }
}
