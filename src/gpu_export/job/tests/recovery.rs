use super::*;

#[test]
fn cold_exports_with_a_failed_heal_match_unrepaired_tiff_and_exr_pixels() {
    let scan = Scan::open(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng"),
        Some([1.; 4]),
    )
    .unwrap();
    let (mut project, id) = fixture(&scan);
    let source = project.source_mut(id).unwrap();
    source.crop = None;
    source
        .repairs
        .insert(
            SupportMask::new([scan.area.x, scan.area.y], [3, 3], vec![true; 9]).unwrap(),
            RepairOrigin::Manual,
            [32, 0],
        )
        .unwrap();
    let repaired = project.image_recipe(id).unwrap();
    let mut original = repaired.clone();
    original.repairs.clear();
    let (device, queue, _) = super::super::super::tests::context();
    let renderer = pollster::block_on(crate::render_fixture::RenderFixture::new_with_cache_policy(
        &device,
        &queue,
        crate::gpu::evaluation::CachePolicy { generated_bytes: 0 },
    ))
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    for (name, format) in [
        ("srgb.tif", Format::Tiff(OutputTarget::Srgb)),
        ("p3.tif", Format::Tiff(OutputTarget::DisplayP3)),
        ("scene.exr", Format::Exr(ExrPrecision::Full)),
    ] {
        let expected = directory.path().join(format!("original-{name}"));
        let actual = directory.path().join(format!("failed-{name}"));
        for (path, recipe) in [(&expected, &original), (&actual, &repaired)] {
            write(
                renderer.evaluation(),
                &queue,
                Job {
                    scan: &scan,
                    field: None,
                    recipe,
                    render: fixture_render(format, &renderer),
                },
                (path, Overwrite::Refuse),
                &mut Execution::default(),
                || Ok(()),
            )
            .unwrap();
        }
        match format {
            Format::Tiff(_) => {
                let decode = |path: &Path| {
                    let mut decoder =
                        tiff::decoder::Decoder::new(std::fs::File::open(path).unwrap()).unwrap();
                    assert_eq!(
                        decoder.dimensions().unwrap(),
                        (scan.area.width, scan.area.height)
                    );
                    let tiff::decoder::DecodingResult::U16(pixels) = decoder.read_image().unwrap()
                    else {
                        panic!("expected RGB16 TIFF");
                    };
                    pixels
                };
                assert_eq!(decode(&actual), decode(&expected), "{name}");
            }
            Format::Exr(_) => {
                let decode = |path: &Path| {
                    let image = exr::prelude::read_first_flat_layer_from_file(path).unwrap();
                    assert_eq!(image.layer_data.channel_data.list.len(), 3);
                    image
                        .layer_data
                        .channel_data
                        .list
                        .into_iter()
                        .map(|channel| {
                            let exr::prelude::FlatSamples::F32(pixels) = channel.sample_data else {
                                panic!("expected float32 EXR");
                            };
                            pixels
                        })
                        .collect::<Vec<_>>()
                };
                assert_eq!(decode(&actual), decode(&expected));
            }
        }
    }
    let field = scan
        .flat_field()
        .cloned()
        .map(|map| Arc::new(crate::gpu::flat::GpuFlatField::new(&device, map).unwrap()));
    let reader = crate::gpu::repair_source::Source::from_recipe(
        field,
        renderer.evaluation(),
        &queue,
        &scan,
        &repaired,
    )
    .unwrap();
    assert_eq!(
        reader
            .repair_succeeded(&repaired.repairs[0], &|| false)
            .unwrap(),
        Some(false),
        "the export fixture must actually fail healing"
    );
    assert!(project.source(id).unwrap().repairs.repairs()[0].enabled);
    assert_eq!(renderer.evaluation().retained_bytes(), 0);
}
