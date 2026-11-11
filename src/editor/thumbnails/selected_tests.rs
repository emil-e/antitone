use super::*;
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::project::Development;
use crate::viewer::readback::image_pixels;

fn preview_image(editor: &Editor, input: &AuthoredPhotograph) -> Arc<GpuImage> {
    let (_, frame, view) = editor
        .document
        .visit
        .canvas
        .preview_rendered
        .as_ref()
        .expect("shared provisional renderer");
    assert_eq!(view.processing(), input.clone());
    assert_eq!(view.target, editor.document.target());
    assert_eq!(
        frame.composition,
        editor
            .document
            .visit
            .canvas
            .last_layout
            .unwrap()
            .0
            .composition
    );
    assert!(
        !view.in_flight(),
        "draw must submit and complete the provisional view"
    );
    view.image.clone()
}

fn assert_same_pixels(actual: &[[f32; 4]], expected: &[[f32; 4]]) {
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        for channel in 0..4 {
            assert!(
                (actual[channel] - expected[channel]).abs() < 3e-5,
                "pixel {index}, channel {channel}: {actual:?} vs {expected:?}"
            );
        }
    }
}

async fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance = crate::gpu::instance();
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    adapter.request_device(&Default::default()).await.unwrap()
}

fn proxy() -> Arc<Proxy> {
    Arc::new(Proxy {
        identity: None,
        width: 3,
        height: 2,
        pixels: vec![[8000., 6000., 4000., 0.]; 6].into(),
        matrix: glam::DMat3::IDENTITY,
    })
}

fn add_source(editor: &mut Editor, path: PathBuf) -> SourceId {
    std::fs::write(&path, b"cache fixture, not a RAW").unwrap();
    let development = crate::project::Development::new(crate::raw::version(), [1.; 4]);
    let id = editor
        .document
        .register_with_development(
            Source {
                development_mode: crate::project::DevelopmentMode::Unity,
                id: SourceId(0),
                path: path.clone(),
                development: SourceDevelopment {
                    decode: Development::new("test-backend".into(), [1.; 4]),
                    dimensions: [900, 600],
                    active_area: [12, 7, 864, 576],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                },
                adjustment_layers: Some(crate::project::AdjustmentLayers::with_global(
                    AdjustmentLayerSettings::default(),
                )),
                repairs: Default::default(),
                orientation: Default::default(),
                crop: None,
            },
            development.clone(),
        )
        .unwrap();
    editor.document.thumbnails.seed(
        editor.document.state.project.source(id).unwrap(),
        FileKey::new(&path),
        proxy(),
    );
    id
}

fn frame(
    editor: &mut Editor,
    context: &egui::Context,
    cx: &UiContext<'_, Editor>,
) -> egui::FullOutput {
    context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            ..Default::default()
        },
        |ui| editor.draw_ui(ui, cx),
    )
}

fn draw(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    context: &egui::Context,
    renderer: &mut egui_wgpu::Renderer,
    mut output: egui::FullOutput,
) {
    for (id, deltas) in &output.textures_delta.set {
        for delta in deltas {
            renderer.update_texture(device, queue, *id, delta);
        }
    }
    output.textures_delta.clear();
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [1100, 800],
        pixels_per_point: 1.,
    };
    let jobs = context.tessellate(output.shapes, output.pixels_per_point);
    let mut encoder = device.create_command_encoder(&Default::default());
    let commands = renderer.update_buffers(device, queue, &mut encoder, &jobs, &screen);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("pending preview offscreen paint"),
        size: wgpu::Extent3d {
            width: 1100,
            height: 800,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = target.create_view(&Default::default());
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    queue.submit(commands.into_iter().chain([encoder.finish()]));
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
}

#[test]
fn pending_preview_paints_current_composition_and_geometry_tools_use_metadata_not_preview_pixels() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let editor = super::super::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.initialize_viewer(&device, &queue);
                editor.document.thumbnails =
                    (Thumbnails::with_disk(device.clone(), queue.clone(), Arc::default(), None))
                        .with_sources(editor.document.cache.clone());
                let path = directory.path().join("selected.ARW");
                let id = add_source(editor, path.clone());
                editor
                    .document
                    .project
                    .source_mut(id)
                    .unwrap()
                    .orientation
                    .turn_clockwise();
                editor.document.state.project.source_mut(id).unwrap().crop =
                    Some(Crop::from_region(Rect {
                        x: 120,
                        y: 70,
                        width: 600,
                        height: 400,
                    }));
                editor.document.project.roll.exposure = 1.25;
                editor
                    .document
                    .project
                    .source_mut(id)
                    .unwrap()
                    .adjustment_layers
                    .as_mut()
                    .unwrap()
                    .layers[0]
                    .adjustment
                    .photo
                    .exposure = 0.75;
                let before = editor.document.state.project.clone();

                editor.load(path, cx);
                assert!(editor.document.visit.source.pending());
                assert!(editor.document.visit.canvas.ready.is_none());
                assert!(editor.document.visit.pending_preview.is_some());
                assert!(!editor.interaction_blocked());
                assert!(editor.scope_input().is_err());

                editor.begin_crop();
                assert!(
                    editor.cropping(),
                    "accepted source geometry is available before its full pixels"
                );
                assert!(editor.document.visit.canvas.ready.is_none());
                editor.end_tool();
                assert!(!editor.measurement_pending());
                assert!(editor.document.export.operation.is_none());

                let context = egui::Context::default();
                let mut renderer = egui_wgpu::Renderer::new(
                    &device,
                    wgpu::TextureFormat::Rgba16Float,
                    Default::default(),
                );
                for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                    editor.document.set_target(target);
                    for source in [false, true] {
                        editor.document.source = source;
                        editor.source_exposure = 2.;
                        let output = frame(editor, &context, cx);
                        let cached_caption = output.shapes.iter().any(|shape| match &shape.shape {
                            egui::Shape::Text(text) => {
                                text.galley.job.text.contains(" · Cached preview")
                                    && text.pos.y
                                        >= editor
                                            .document
                                            .visit
                                            .canvas
                                            .last_layout
                                            .unwrap()
                                            .0
                                            .canvas
                                            .bottom()
                            }
                            _ => false,
                        });
                        assert!(editor.document.visit.canvas.ready.is_none());
                        assert!(editor.document.visit.pending_preview.is_some());
                        assert_eq!(
                            editor
                                .document
                                .visit
                                .canvas
                                .last_layout
                                .unwrap()
                                .0
                                .composition
                                .size(),
                            [400, 600]
                        );
                        draw(&device, &queue, &context, &mut renderer, output);
                        assert!(cached_caption);

                        let expected = if source {
                            editor.source_parameters(2.).unwrap().into()
                        } else {
                            editor
                                .positive_processing(
                                    editor.document.project.input,
                                    editor.document.project.roll,
                                    editor.photo_settings(),
                                )
                                .unwrap()
                        };
                        let image = preview_image(editor, &expected);
                        let pixels = image_pixels(&device, &queue, &image);
                        assert!(
                            pixels
                                .iter()
                                .all(|pixel| pixel.iter().all(|channel| channel.is_finite())
                                    && pixel[3] > 0.)
                        );
                        assert!(
                            !editor.document.visit.canvas.original.refining()
                                && editor.document.visit.canvas.original.task.is_none()
                                && !editor.measurement_pending()
                                && editor.document.export.operation.is_none(),
                            "pending preview scheduled authoritative work"
                        );
                    }
                }

                editor.document.set_target(OutputTarget::DisplayP3);
                assert_eq!(editor.document.state.project, before);
            });
        });
    });
}

#[test]
fn cached_selected_images_do_not_alias_filmstrip_processing() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let mut editor = super::super::tests::fixture();
        editor.initialize_viewer(&device, &queue);
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        editor.document.thumbnails =
            (Thumbnails::with_disk(device.clone(), queue.clone(), renderers.clone(), None))
                .with_sources(editor.document.cache.clone());
        let path = directory.path().join("selected.ARW");
        let id = add_source(&mut editor, path.clone());
        let source = editor.document.state.project.source(id).unwrap();
        let image = proxy()
            .image(
                &renderers.get(&device, &queue).unwrap(),
                &device,
                OutputTarget::DisplayP3,
                source.development.active_area,
            )
            .await
            .unwrap();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.load(path, cx);
                let selected = editor
                    .selected_view()
                    .unwrap()
                    .1
                    .image(editor.document.target())
                    .unwrap();
                assert!(!Arc::ptr_eq(&image, &selected));

                let mut encoder = device.create_command_encoder(&Default::default());
                let source = editor.source_parameters(2.).unwrap();
                let positive = editor
                    .positive_processing(
                        editor.document.project.input,
                        editor.document.project.roll,
                        editor.photo_settings(),
                    )
                    .unwrap();
                assert!(selected.prepare(&device, &mut encoder, source));
                assert!(
                    image
                        .prepare_processing(&device, &mut encoder, &positive)
                        .unwrap()
                );
                assert!(!selected.prepare(&device, &mut encoder, source));
                queue.submit([encoder.finish()]);
                device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            });
        });
    });
}

#[test]
fn pending_replacements_ignore_old_ready_and_clear_on_failure_or_document_work() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let editor = super::super::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.initialize_viewer(&device, &queue);
                editor.document.thumbnails =
                    (Thumbnails::with_disk(device, queue, Arc::default(), None))
                        .with_sources(editor.document.cache.clone());
                let a = directory.path().join("a.ARW");
                let b = directory.path().join("b.ARW");
                add_source(editor, a.clone());
                let b_id = add_source(editor, b.clone());

                editor.load(a, cx);
                let first = editor.id;
                editor.load(b.clone(), cx);
                let second = editor.id;
                editor.source_result(first, Err("obsolete".into()));
                assert_eq!(editor.document.visit.current_source, Some(b_id));
                assert!(!editor.interaction_blocked());
                assert!(editor.document.visit.pending_preview.is_some());
                assert!(editor.document.visit.canvas.ready.is_none());

                editor.source_result(second, Err("current load failed".into()));
                assert!(!editor.interaction_blocked());
                assert!(editor.document.visit.pending_preview.is_none());
                assert!(editor.document.visit.canvas.selection_timing.is_none());
                assert_eq!(editor.status, "current load failed");

                editor.load(b.clone(), cx);
                assert!(editor.document.visit.pending_preview.is_some());
                assert_eq!(editor.document.visit.current_source, Some(b_id));
                editor.new_project();
                assert!(
                    editor.document.visit.pending_preview.is_none()
                        && editor.document.visit.canvas.selection_timing.is_none()
                );
                assert!(!editor.interaction_blocked());
                assert!(editor.document.project.sources.is_empty());
            });
        });
    });
}

#[test]
fn incompatible_cached_sources_keep_the_loading_placeholder() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let editor = super::super::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.document.thumbnails =
                    (Thumbnails::with_disk(device, queue, Arc::default(), None))
                        .with_sources(editor.document.cache.clone());
                let path = directory.path().join("selected.ARW");
                let id = add_source(editor, path.clone());
                editor.load(path.clone(), cx);
                assert!(editor.document.visit.pending_preview.is_some());
                assert!(editor.document.visit.canvas.ready.is_none());
                assert!(!editor.interaction_blocked());
                editor.document.thumbnails.seed(
                    editor.document.state.project.source(id).unwrap(),
                    editor.document.file_key(&path),
                    proxy(),
                );
                editor
                    .document
                    .project
                    .source_mut(id)
                    .unwrap()
                    .development
                    .dimensions[0] += 1;
                editor.document.project.require_runtime().unwrap();
                editor.load(path.clone(), cx);
                assert!(editor.document.visit.pending_preview.is_none());

                assert_eq!(
                    editor
                        .source_input(editor.document.source_path(id).unwrap())
                        .source
                        .record()
                        .unwrap()
                        .id,
                    id
                );
            });
        });
    });
}

#[test]
fn project_correction_changes_preserve_pending_camera_samples() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let editor = super::super::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.document.thumbnails =
                    (Thumbnails::with_disk(device.clone(), queue, Arc::default(), None))
                        .with_sources(editor.document.cache.clone());
                let path = directory.path().join("corrected.ARW");
                let id = add_source(editor, path.clone());
                let source = editor.document.state.project.source(id).unwrap().clone();
                editor
                    .document
                    .state
                    .project
                    .set_flat_field(Some(crate::project::FlatFieldSettings::new(
                        source.path.clone(),
                        Arc::new(
                            crate::flat_field::FlatField::from_nodes(
                                [12, 7, 864, 576],
                                [2, 2],
                                vec![DVec3::splat(1.1); 4],
                            )
                            .unwrap(),
                        ),
                    )))
                    .unwrap();
                editor.document.correction = Correction::prepare(&device, &editor.document, None)
                    .unwrap()
                    .into();
                let before = editor.document.state.project.clone();
                editor.load(path, cx);
                assert_eq!(editor.document.state.project, before);
                let proxy = editor
                    .document
                    .visit
                    .pending_preview
                    .as_ref()
                    .unwrap()
                    .proxy
                    .clone();
                editor.document.state.project.set_flat_field(None).unwrap();
                editor.document.correction = None.into();
                editor.validate_pending_preview();
                assert!(
                    proxy.same_source(
                        &editor
                            .document
                            .visit
                            .pending_preview
                            .as_ref()
                            .unwrap()
                            .proxy
                    )
                );
                assert!(!editor.interaction_blocked());
                assert!(editor.document.visit.canvas.ready.is_none());
                assert!(editor.document.visit.source.pending());
            });
        });
    });
}

#[test]
fn authoritative_ready_keeps_pending_until_the_working_view_is_displayed() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let editor = super::super::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                editor.initialize_viewer(&device, &queue);
                editor.document.thumbnails =
                    (Thumbnails::with_disk(device, queue, Arc::default(), None))
                        .with_sources(editor.document.cache.clone());
                let path = directory.path().join("selected.ARW");
                let id = add_source(editor, path.clone());
                editor.load(path.clone(), cx);
                editor
                    .document
                    .thumbnails
                    .apply_budgets(memory::CacheBudgets::for_memory(
                        48 << 30,
                        memory::Pressure::Critical,
                    ));
                assert_eq!(editor.document.thumbnails.sources.proxy_count(), 1);
                editor.validate_pending_preview();
                assert!(editor.document.visit.pending_preview.is_some());
                let ready = Ready {
                    reference: None,
                    developed: crate::editor::tests::developed_fixture(),
                    file: thumbnails::FileKey::new(&path),
                    thumbnail: None,
                    path,
                    area: editor
                        .document
                        .visit
                        .pending_preview
                        .as_ref()
                        .unwrap()
                        .area(),
                    analysis_support: None,
                    proxy: editor
                        .document
                        .visit
                        .pending_preview
                        .as_ref()
                        .unwrap()
                        .proxy
                        .clone(),
                    record: editor.document.state.project.source(id).unwrap().clone(),
                };
                editor.source_result(editor.id, Ok(Box::new(ready)));
                assert!(editor.document.visit.canvas.ready.is_some());
                assert!(editor.document.visit.pending_preview.is_some());
                assert!(!editor.interaction_blocked());
                assert!(editor.selected_view().unwrap().2);
                editor.document.source = false;
                editor
                    .view_processing(
                        editor.input(),
                        editor.document.project.roll,
                        editor.photo_settings(),
                    )
                    .unwrap();
                assert_eq!(editor.scope_input().err(), Some("Waiting for view"));
                editor.document.visit.canvas.last_layout = Some((
                    editor.document.visit.canvas.viewport.layout(
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
                        editor.document.visit.canvas.ready.as_ref().unwrap().area,
                        1.,
                    ),
                    1.,
                ));
                assert_eq!(
                    editor.scope_request().err(),
                    Some("Waiting for view"),
                    "readiness and a synthetic layout are not a displayed image"
                );
            });
            let context = egui::Context::default();
            for _ in 0..3 {
                crate::editor::tests::app_frame(app, &context, vec![]);
            }
            app.read(|editor| {
                let scopes::ScopeInput { view, .. } = editor.scope_request().unwrap();
                assert!(Arc::ptr_eq(
                    &view,
                    &editor.canvas_frame.displayed.as_ref().unwrap().view
                ));
            });
        });
    });
}

#[test]
fn local_snapshots_reach_pending_previews_without_rebuilding_camera_sources() {
    use crate::{
        adjustment_layers::{AdjustmentLayer, AdjustmentLayerCoverage},
        geometry::SourcePoint,
        project::AdjustmentLayers,
    };
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let editor = super::super::tests::fixture();
        crate::application::test_support::with_application(editor, |app| {
            app.enter(|editor, cx| {
                let renderers = Arc::new(crate::renderer::RendererCache::default());
                editor.document.thumbnails =
                    (Thumbnails::with_disk(device.clone(), queue.clone(), renderers.clone(), None))
                        .with_sources(editor.document.cache.clone());
                editor.initialize_viewer(&device, &queue);
                let path = directory.path().join("local.ARW");
                let id = add_source(editor, path.clone());

                let source_key = SourceKey::new(
                    editor.document.state.project.source(id).unwrap(),
                    FileKey::new(&path),
                );
                let retained = editor
                    .document
                    .thumbnails
                    .sources
                    .get_proxy(&source_key.store_key_at(&path).unwrap())
                    .unwrap();

                let mut layers = AdjustmentLayers::default();
                layers
                    .add(
                        "Linear".into(),
                        AdjustmentLayer {
                            mask: AdjustmentLayerCoverage::Linear {
                                start: SourcePoint([12., 7.]),
                                end: SourcePoint([876., 583.]),
                            },
                            photo: AdjustmentLayerSettings {
                                exposure: 1.,
                                ..Default::default()
                            },
                            enabled: true,
                            invert: false,
                            strength: 0.7,
                        },
                    )
                    .unwrap();
                editor
                    .document
                    .project
                    .source_mut(id)
                    .unwrap()
                    .adjustment_layers = Some(layers);

                editor.load(path.clone(), cx);
                assert!(editor.document.visit.source.pending());
                assert!(editor.document.visit.pending_preview.is_some());
                assert!(!editor.interaction_blocked());
                assert!(editor.document.visit.canvas.ready.is_none());
                let captured = editor
                    .positive_processing(
                        editor.document.project.input,
                        editor.document.project.roll,
                        editor.photo_settings(),
                    )
                    .unwrap();
                let context = egui::Context::default();
                let mut renderer = egui_wgpu::Renderer::new(
                    &device,
                    wgpu::TextureFormat::Rgba16Float,
                    Default::default(),
                );
                for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
                    editor.document.set_target(target);
                    let selected = editor
                        .selected_view()
                        .unwrap()
                        .1
                        .image(editor.document.target())
                        .unwrap();
                    let mut positive_pixels = None;

                    for source_mode in [false, true] {
                        editor.document.source = source_mode;
                        editor.source_exposure = 2.;
                        draw(
                            &device,
                            &queue,
                            &context,
                            &mut renderer,
                            frame(editor, &context, cx),
                        );
                        let positive = editor
                            .thumbnail_parameters(editor.document.state.project.source(id).unwrap())
                            .unwrap();
                        assert_eq!(positive.clone(), captured.clone());
                        let expected = if source_mode {
                            editor.source_parameters(2.).unwrap().into()
                        } else {
                            positive.clone()
                        };
                        let mut encoder = device.create_command_encoder(&Default::default());
                        selected
                            .prepare_processing(&device, &mut encoder, &expected)
                            .unwrap();
                        queue.submit([encoder.finish()]);
                        let actual =
                            image_pixels(&device, &queue, &preview_image(editor, &expected));
                        assert_same_pixels(&actual, &image_pixels(&device, &queue, &selected));
                        if !source_mode {
                            positive_pixels = Some(actual);
                        }
                        assert!(
                            !editor.document.visit.canvas.original.refining()
                                && editor.document.visit.canvas.original.task.is_none(),
                            "cached preview must not schedule full-source work"
                        );
                    }

                    editor.document.source = false;
                    editor
                        .document
                        .project
                        .source_mut(id)
                        .unwrap()
                        .adjustment_layers
                        .as_mut()
                        .unwrap()
                        .layers[0]
                        .adjustment
                        .invert = true;
                    draw(
                        &device,
                        &queue,
                        &context,
                        &mut renderer,
                        frame(editor, &context, cx),
                    );
                    let edited = editor
                        .thumbnail_parameters(editor.document.state.project.source(id).unwrap())
                        .unwrap();
                    assert_ne!(edited.clone(), captured.clone());
                    let mut encoder = device.create_command_encoder(&Default::default());
                    selected
                        .prepare_processing(&device, &mut encoder, &edited)
                        .unwrap();
                    queue.submit([encoder.finish()]);
                    let actual = image_pixels(&device, &queue, &preview_image(editor, &edited));
                    assert_same_pixels(&actual, &image_pixels(&device, &queue, &selected));
                    assert!(
                        actual
                            .iter()
                            .zip(positive_pixels.as_ref().unwrap())
                            .any(|(a, b)| (0..3)
                                .any(|channel| (a[channel] - b[channel]).abs() > 1e-4)),
                        "inverting the nonneutral local must visibly change the rendered preview"
                    );
                    assert!(Arc::ptr_eq(
                        &editor
                            .document
                            .thumbnails
                            .sources
                            .get_proxy(&source_key.store_key_at(&path).unwrap())
                            .unwrap(),
                        &retained
                    ));
                    editor
                        .document
                        .project
                        .source_mut(id)
                        .unwrap()
                        .adjustment_layers
                        .as_mut()
                        .unwrap()
                        .layers[0]
                        .adjustment
                        .invert = false;
                }
                editor.document.project.require_runtime().unwrap();
            });
        });
    });
}
