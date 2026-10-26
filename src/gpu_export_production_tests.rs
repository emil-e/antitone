use super::*;
use crate::processing::Calibration;

#[cfg(feature = "editor")]
#[test]
fn editor_background_prewarm_initializes_the_shared_renderer() {
    let (device, queue, _) = super::tests::context();
    let cache = Arc::new(crate::renderer::RendererCache::new(
        SubmissionGate::default(),
    ));
    cache.prewarm(device.clone(), queue.clone());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !cache.is_initialized() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        cache.is_initialized(),
        "background prewarm must initialize processing without a foreground consumer"
    );
    let renderer = cache.get(&device, &queue).unwrap();
    assert!(Arc::ptr_eq(&renderer, &cache.get(&device, &queue).unwrap()));
}

#[test]
fn renderer_exports_exact_rotated_tiff_and_both_exr_precisions_and_stops_safely() {
    let (device, queue, renderer) = super::tests::context();
    let gate = SubmissionGate::default();
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        None,
    )
    .unwrap();
    let a = scan.area;
    let exact = Composition::new(a, Default::default(), Default::default()).unwrap();
    let rotated = Composition::with_crop(
        a,
        Default::default(),
        Default::default(),
        Some(crate::crop::Crop {
            center: [
                f64::from(a.x) + f64::from(a.width) / 2.,
                f64::from(a.y) + f64::from(a.height) / 2.,
            ],
            size: [a.width - 96, a.height - 96],
            angle: 0.13,
        }),
    )
    .unwrap();
    let request = |composition, format| Request {
        scan: &scan,
        field: None,
        input: Calibration {
            dmin: DVec3::splat(0.8),
            dmax: DVec3::splat(0.02),
            input_floor: scan.input_floor,
        }
        .into(),
        roll: RollSettings::default(),
        grade: AdjustmentLayerSnapshot::new(vec![
            crate::adjustment_layers::AdjustmentLayer::global(Default::default()),
        ])
        .unwrap(),
        repairs: &[],
        composition,
        format,
        look: None,
    };
    let directory = tempfile::tempdir().unwrap();
    let keep = directory.path().join("keep.tif");
    std::fs::write(&keep, b"existing destination").unwrap();
    assert!(
        write_checked(
            &renderer,
            &device,
            (&queue, &gate),
            request(rotated, Format::Tiff(OutputTarget::Srgb)),
            &keep,
            Overwrite::Confirmed,
            || Err(Error::Export("stop before preparation".into()))
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&keep).unwrap(), b"existing destination");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);

    for (name, composition) in [("exact", exact), ("rotated", rotated)] {
        for (suffix, format) in [
            ("srgb.tif", Format::Tiff(OutputTarget::Srgb)),
            ("p3.tif", Format::Tiff(OutputTarget::DisplayP3)),
            ("half.exr", Format::Exr(ExrPrecision::Half)),
            ("full.exr", Format::Exr(ExrPrecision::Full)),
        ] {
            let path = directory.path().join(format!("{name}-{suffix}"));
            write(
                &renderer,
                &device,
                (&queue, &gate),
                request(composition, format),
                &path,
                Overwrite::Refuse,
            )
            .unwrap();
            assert!(path.is_file());
        }
    }
    assert_eq!(std::fs::read(&keep).unwrap(), b"existing destination");

    let look = Arc::new(
        crate::looks::LookAsset::new(
            "EXR bypass witness".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(
                crate::looks::Lut3D::new(
                    2,
                    [-2.; 3],
                    [2.; 3],
                    (0..8)
                        .map(|i| {
                            [
                                -1.8 + 4. * (i % 2) as f32,
                                -1.8 + 4. * ((i / 2) % 2) as f32,
                                -1.8 + 4. * (i / 4) as f32,
                            ]
                        })
                        .collect(),
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    );
    // Neither pixels nor EXR metadata may inherit TIFF's look. Compare decoded fields:
    // the encoder's HashMap attribute order can change without changing their contents.
    for (name, precision) in [("half", ExrPrecision::Half), ("full", ExrPrecision::Full)] {
        let path = directory.path().join(format!("with-look-{name}.exr"));
        let mut input = request(rotated, Format::Exr(precision));
        input.look = Some(look.clone());
        write(
            &renderer,
            &device,
            (&queue, &gate),
            input,
            &path,
            Overwrite::Refuse,
        )
        .unwrap();
        let actual = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
        let expected = exr::prelude::read_first_flat_layer_from_file(
            directory.path().join(format!("rotated-{name}.exr")),
        )
        .unwrap();
        assert_eq!(actual.layer_data.size, expected.layer_data.size);
        assert_eq!(actual.attributes, expected.attributes);
        assert!(actual.layer_data.attributes == expected.layer_data.attributes);
        assert!(
            actual.layer_data.channel_data == expected.layer_data.channel_data,
            "EXR look bypass must preserve samples exactly, {precision:?}"
        );
    }
}
