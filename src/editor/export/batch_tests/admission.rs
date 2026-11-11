use super::*;
use crate::editor::{document::Document, source, source_loading, working_view::WorkingView};
use std::sync::Mutex;

fn cold_editor(limits: wgpu::Limits) -> (Editor, Arc<Mutex<Vec<PathBuf>>>) {
    let instance = crate::gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: limits,
        ..Default::default()
    }))
    .unwrap();
    let reads = Arc::new(Mutex::new(Vec::new()));
    let observed = reads.clone();
    let loader = source_loading::SourceLoader::new_observed(move |path, phase, _| {
        if matches!(phase, source_loading::NativePhase::UnpackBegin) {
            observed.lock().unwrap().push(path.to_owned());
        }
        Ok(())
    });
    let mut document = Document::default();
    document
        .import(&["roll-0.dng", "roll-1.dng"].map(|name| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/raw")
                .join(name)
        }))
        .unwrap();
    document.project.set_dmin(Some([0.8; 3]));
    document.project.set_dmax(Some([0.02; 3]));
    for source in document.project.sources.iter_mut() {
        source.crop = Some(crate::crop::Crop {
            center: [200., 200.],
            size: [16, 16],
            angle: 0.,
        });
    }
    let mut editor = Editor::new(&device, &queue);
    editor.document = document.into();
    let selected = editor.document.project.order[1];
    editor.document.visit.current_source = Some(selected);
    editor
        .document
        .roll_selection
        .reconcile(&editor.document.project.order.clone(), Some(selected));
    editor.document.visit.working_view = WorkingView::new(device.clone(), queue.clone());
    let renderers = Arc::new(crate::renderer::RendererCache::default());
    let sources = source::Services {
        device: device.clone(),
        queue: queue.clone(),
        developments: loader,
        renderers: renderers.clone(),
    };
    editor.source_services = Some(sources.clone());
    editor.export_services = Some(Services::new(
        renderers,
        device,
        queue,
        crate::platform::SubmissionGate::default(),
        sources,
    ));
    (editor, reads)
}

fn run_export(
    app: &mut crate::application::test_support::TestApplication<'_, Editor>,
    draft: &Draft,
) {
    app.enter(|editor, cx| {
        let request = editor.capture_export(draft).unwrap();
        assert!(
            request
                .photos
                .iter()
                .all(|photo| matches!(photo.input, Input::Acquire(_))),
            "metadata-only fixture must exercise captured source acquisition"
        );
        editor.export_photos(request, cx);
    });
    app.run_until(Duration::from_secs(30), |editor| {
        editor.document.export.operation.is_none()
    });
}

fn export(
    app: &mut crate::application::test_support::TestApplication<'_, Editor>,
    draft: &Draft,
) -> Summary {
    run_export(app, draft);
    let summary = app.read(|editor| editor.document.export.summary.clone().unwrap());
    app.update(|editor| editor.dialogs.answer_for_test("OK"));
    summary
}

#[test]
fn selected_cold_export_ignores_unselected_render_failure_and_all_scope_reports_it_locally() {
    for missing in [false, true] {
        let (mut editor, reads) = cold_editor(Default::default());
        let directory = tempfile::tempdir().unwrap();
        let bad = editor.document.project.order[0];
        let good = editor.document.project.order[1];
        let mut layers = crate::project::AdjustmentLayers::default();
        layers
            .add(
                "Unrepresentable mask".into(),
                crate::adjustment_layers::AdjustmentLayer {
                    mask: crate::adjustment_layers::AdjustmentLayerCoverage::Radial {
                        center: crate::geometry::SourcePoint([200., 200.]),
                        radii: [f64::MIN_POSITIVE, 1.],
                        angle: 0.,
                        feather: 0.5,
                    },
                    photo: AdjustmentLayerSettings {
                        exposure: 1.,
                        ..Default::default()
                    },
                    enabled: true,
                    invert: false,
                    strength: 1.,
                },
            )
            .unwrap();
        editor
            .document
            .project
            .source_mut(bad)
            .unwrap()
            .adjustment_layers = Some(layers);
        editor.document.project.require_runtime().unwrap();
        assert!(
            crate::editor::tests::photograph(
                &editor.document.capture(),
                bad,
                &editor.source_services.as_ref().unwrap().device.limits()
            )
            .is_err()
        );
        if missing {
            editor.document.project.source_mut(bad).unwrap().path =
                directory.path().join("unavailable.dng");
        }
        let expected_read = editor.document.source_path(good).unwrap();
        let before = editor.document.project.clone();
        let selected = editor.document.visit.current_source;
        let mut selected_draft = draft(directory.path());
        selected_draft.scope = Scope::Selected;
        let all_folder = directory.path().join("all");
        std::fs::create_dir(&all_folder).unwrap();
        let bad_path = all_folder.join(if missing {
            "unavailable.exr"
        } else {
            "roll-0.exr"
        });
        std::fs::write(&bad_path, b"existing failed-photo output").unwrap();
        with_application(editor, |app| {
            let summary = export(app, &selected_draft);
            assert_eq!(
                (summary.exported(), summary.failed(), summary.total),
                (1, 0, 1)
            );
            assert!(!summary.stopped);
            assert_eq!(*reads.lock().unwrap(), vec![expected_read]);
            assert_eq!(exr_size(&directory.path().join("roll-1.exr")), [16, 16]);
            let mut all_draft = draft(&all_folder);
            all_draft.collision = Collision::Replace;
            let all = export(app, &all_draft);
            assert_eq!((all.exported(), all.failed(), all.unattempted()), (1, 1, 0));
            assert!(
                !all.stopped,
                "a photo error must not become a shared-infrastructure failure"
            );
            assert!(matches!(&all.outcomes[0], Outcome::Failed { path, .. } if path == &bad_path));
            assert_eq!(
                std::fs::read(&bad_path).unwrap(),
                b"existing failed-photo output"
            );
            assert_eq!(exr_size(&all_folder.join("roll-1.exr")), [16, 16]);
            app.read(|editor| {
                assert_eq!(editor.document.project, before);
                assert_eq!(editor.document.visit.current_source, selected);
                assert!(editor.document.visit.canvas.ready.is_none());
            });
        });
    }
}

fn look(size: u32) -> Arc<crate::looks::LookAsset> {
    Arc::new(
        crate::looks::LookAsset::new(
            "Capacity witness".into(),
            crate::looks::LookOrigin::Imported,
            Arc::new(
                crate::looks::Lut3D::new(
                    size,
                    [0.; 3],
                    [1.; 3],
                    vec![[0.5; 3]; (size as usize).pow(3)],
                )
                .unwrap(),
            ),
        )
        .unwrap(),
    )
}

#[test]
fn exr_bypasses_actual_lut_capacity_while_tiff_preserves_failed_destination_and_can_retry() {
    let (editor, _) = cold_editor(wgpu::Limits {
        max_storage_buffer_binding_size: 8 * 1024 * 1024,
        ..Default::default()
    });
    let directory = tempfile::tempdir().unwrap();
    let large = look(81);
    let services = editor.source_services.as_ref().unwrap();
    let renderer = services
        .renderers
        .get(&services.device, &services.queue)
        .unwrap();
    assert!(
        renderer.look_resource(&services.device, &large).is_err(),
        "the LUT must actually exceed this device's resource capacity"
    );
    with_application(editor, |app| {
        for precision in [ExrPrecision::Half, ExrPrecision::Full] {
            let mut outputs = Vec::new();
            for with_look in [false, true] {
                let folder = directory.path().join(format!("{precision:?}-{with_look}"));
                std::fs::create_dir(&folder).unwrap();
                app.update(|editor| {
                    editor.document.project.output.lut = with_look.then(|| large.clone())
                });
                let mut settings = draft(&folder);
                settings.scope = Scope::Selected;
                settings.precision = precision;
                let summary = export(app, &settings);
                assert_eq!((summary.exported(), summary.failed()), (1, 0));
                outputs.push(
                    exr::prelude::read_first_flat_layer_from_file(folder.join("roll-1.exr"))
                        .unwrap(),
                );
            }
            let [without, with]: [_; 2] = outputs.try_into().ok().unwrap();
            assert_eq!(with.layer_data.size, without.layer_data.size);
            assert_eq!(with.attributes, without.attributes);
            assert!(with.layer_data.attributes == without.layer_data.attributes);
            assert!(
                with.layer_data.channel_data == without.layer_data.channel_data,
                "EXR bypass must preserve decoded samples exactly, {precision:?}"
            );
        }
        app.update(|editor| editor.document.project.output.lut = Some(large));
        let folder = directory.path().join("tiff");
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join("roll-1.tif");
        std::fs::write(&path, b"preserve failed TIFF destination").unwrap();
        let mut settings = draft(&folder);
        settings.scope = Scope::Selected;
        settings.kind = Kind::Tiff;
        settings.collision = Collision::Replace;
        run_export(app, &settings);
        app.update(|editor| {
            let failure = editor.dialogs.message().unwrap();
            assert_eq!(failure.title, FailureCase::Export.title());
            assert!(failure.details.contains("limits"), "{}", failure.details);
            assert!(
                editor.document.export.summary.is_none(),
                "a single-photo failure uses its acknowledged notice"
            );
            editor.dialogs.answer_for_test("OK");
        });
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"preserve failed TIFF destination"
        );
        assert_eq!(
            std::fs::read_dir(&folder).unwrap().count(),
            1,
            "no leftover temporary output"
        );
        app.update(|editor| editor.document.project.output.lut = Some(look(3)));
        let retried = export(app, &settings);
        assert_eq!((retried.exported(), retried.failed()), (1, 0));
        let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(decoder.dimensions().unwrap(), (16, 16));
        assert!(matches!(
            decoder.read_image().unwrap(),
            tiff::decoder::DecodingResult::U16(_)
        ));
    });
}
