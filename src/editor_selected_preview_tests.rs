use super::*;

async fn gpu() -> (wgpu::Device, wgpu::Queue) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    adapter.request_device(&Default::default()).await.unwrap()
}
fn proxy() -> Arc<Proxy> {
    Arc::new(Proxy {
        width: 3,
        height: 2,
        pixels: vec![[8000, 6000, 4000, 0]; 6],
        matrix: glam::DMat3::IDENTITY,
    })
}
fn add_source(editor: &mut Editor, path: PathBuf) -> SourceId {
    std::fs::write(&path, b"cache fixture, not a RAW").unwrap();
    let development = editor.document.project.development.clone().unwrap();
    let id = editor
        .document
        .register(
            Source {
                id: SourceId(0),
                path: path.clone(),
                development: SourceDevelopment {
                    dimensions: [900, 600],
                    active_area: [12, 7, 864, 576],
                    camera_model: "test-camera".into(),
                    camera_to_ap1: glam::DMat3::IDENTITY.to_cols_array_2d(),
                    input_floor: crate::processing::CAMERA_INPUT_FLOOR,
                },
                reference_only: false,
                photo: PhotoSettings::default(),
                local_adjustments: None,
                orientation: Default::default(),
                crop: None,
            },
            development.clone(),
        )
        .unwrap();
    editor
        .thumbnails
        .bind_sources(editor.document.cache().clone());
    editor.thumbnails.seed(
        editor.document.project.source(id).unwrap(),
        &development,
        FileKey::new(&path),
        proxy(),
    );
    id
}
fn frame(editor: &mut Editor, context: &egui::Context) -> egui::FullOutput {
    context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 800.),
            )),
            ..Default::default()
        },
        |ui| host::App::ui(editor, ui),
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
fn pending_preview_paints_current_composition_but_cannot_supply_authoritative_operations() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, _send, commands) = super::super::tests::fixture();
        editor.thumbnails =
            Thumbnails::with_disk(device.clone(), queue.clone(), Arc::default(), None);
        let path = directory.path().join("selected.ARW");
        let id = add_source(&mut editor, path.clone());
        editor
            .document
            .project
            .source_mut(id)
            .unwrap()
            .orientation
            .turn_clockwise();
        editor.document.project.source_mut(id).unwrap().crop = Some(Crop::from_region(Rect {
            x: 120,
            y: 70,
            width: 600,
            height: 400,
        }));
        editor.sync_document();
        let before = editor.document.project.clone();
        editor.load(path);
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
        assert!(editor.ready.is_none() && editor.pending_preview.is_some() && editor.busy);
        assert!(editor.scope_input().is_err());
        editor.begin_pick(Pick::Gamma);
        editor.begin_crop();
        editor.export(
            "unused.tiff".into(),
            OutputFormat::Tiff(OutputTarget::Srgb),
            Overwrite::Refuse,
        );
        assert!(editor.pick.is_none() && editor.crop_edit.is_none());
        assert!(commands.try_recv().is_err());
        let context = egui::Context::default();
        let mut renderer = egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Rgba16Float,
            Default::default(),
        );
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            editor.target = target;
            for source in [false, true] {
                editor.source = source;
                editor.source_exposure = 2.;
                let output = frame(&mut editor, &context);
                let cached_caption = output.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Text(t) if t.galley.job.text.contains(" · Cached preview") && t.pos.y >= editor.last_layout.unwrap().0.canvas.bottom()));
                assert!(editor.ready.is_none() && editor.pending_preview.is_some());
                assert_eq!(editor.last_layout.unwrap().0.composition.size(), [400, 600]);
                draw(&device, &queue, &context, &mut renderer, output);
                assert!(cached_caption);
                let expected = if source {
                    editor.source_parameters(2.).unwrap()
                } else {
                    editor
                        .positive_parameters(
                            editor.calibration().unwrap(),
                            editor.roll,
                            editor.photo,
                        )
                        .unwrap()
                };
                let image = editor
                    .selected_view()
                    .unwrap()
                    .1
                    .image(editor.target)
                    .unwrap();
                let mut encoder = device.create_command_encoder(&Default::default());
                assert!(!image.prepare(&device, &mut encoder, expected));
                assert!(
                    commands.try_recv().is_err(),
                    "pending preview scheduled authoritative work"
                );
            }
        }
        editor.target = OutputTarget::DisplayP3;
        editor.sync_document();
        assert_eq!(editor.document.project, before);
    });
}

#[test]
fn cached_selected_images_do_not_alias_filmstrip_processing() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, _, _commands) = super::super::tests::fixture();
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        editor.thumbnails =
            Thumbnails::with_disk(device.clone(), queue.clone(), renderers.clone(), None);
        let path = directory.path().join("selected.ARW");
        let id = add_source(&mut editor, path.clone());
        let source = editor.document.project.source(id).unwrap();
        let image = proxy()
            .image(
                &renderers.get(&device, &queue).unwrap(),
                &device,
                OutputTarget::DisplayP3,
                &source.development,
                None,
            )
            .await
            .unwrap();
        editor.load(path);
        let selected = editor
            .selected_view()
            .unwrap()
            .1
            .image(editor.target)
            .unwrap();
        assert!(!Arc::ptr_eq(&image, &selected));
        let mut encoder = device.create_command_encoder(&Default::default());
        let source = editor.source_parameters(2.).unwrap();
        let positive = editor
            .positive_parameters(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        assert!(selected.prepare(&device, &mut encoder, source));
        assert!(image.prepare(&device, &mut encoder, positive));
        assert!(!selected.prepare(&device, &mut encoder, source));
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    });
}

#[test]
fn pending_replacements_ignore_old_ready_and_clear_on_failure_or_document_work() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, messages, commands) = super::super::tests::fixture();
        editor.thumbnails = Thumbnails::with_disk(device, queue, Arc::default(), None);
        let a = directory.path().join("a.ARW");
        let b = directory.path().join("b.ARW");
        add_source(&mut editor, a.clone());
        let b_id = add_source(&mut editor, b.clone());
        editor.load(a);
        let first = editor.id;
        editor.load(b.clone());
        let second = editor.id;
        messages
            .send(Message::Ready(first, Err("obsolete".into())))
            .unwrap();
        editor.receive();
        assert_eq!(editor.current_source, Some(b_id));
        assert!(editor.busy && editor.pending_preview.is_some() && editor.ready.is_none());
        messages
            .send(Message::Ready(second, Err("current load failed".into())))
            .unwrap();
        editor.receive();
        assert!(
            !editor.busy && editor.pending_preview.is_none() && editor.selection_timing.is_none()
        );
        assert_eq!(editor.status, "current load failed");
        editor.load(b);
        assert!(editor.pending_preview.is_some());
        editor.start_document(DocumentWork::New);
        assert!(editor.pending_preview.is_none() && editor.selection_timing.is_none());
        assert_eq!(commands.try_iter().count(), 4);
    });
}

#[test]
fn unavailable_changed_or_incompatible_cached_sources_keep_the_loading_placeholder() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, _, commands) = super::super::tests::fixture();
        editor.thumbnails = Thumbnails::with_disk(device, queue, Arc::default(), None);
        let path = directory.path().join("selected.ARW");
        let id = add_source(&mut editor, path.clone());
        editor.load(path.clone());
        assert!(editor.pending_preview.is_some());
        std::fs::write(&path, b"intentionally changed file stats").unwrap();
        editor.validate_pending_preview();
        assert!(editor.pending_preview.is_none());
        editor.load(path.clone());
        assert!(editor.pending_preview.is_none() && editor.ready.is_none() && editor.busy);
        let development = editor.document.project.development.clone().unwrap();
        editor.thumbnails.seed(
            editor.document.project.source(id).unwrap(),
            &development,
            FileKey::new(&path),
            proxy(),
        );
        editor
            .document
            .project
            .development
            .as_mut()
            .unwrap()
            .fixed_wb[0] *= 1.01;
        editor.load(path.clone());
        assert!(editor.pending_preview.is_none());
        std::fs::remove_file(&path).unwrap();
        editor.load(path);
        assert!(editor.pending_preview.is_none());
        assert_eq!(commands.try_iter().count(), 4);
    });
}

#[test]
fn correction_is_preview_owned_and_change_invalidates_only_pending_display() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, _, commands) = super::super::tests::fixture();
        editor.thumbnails = Thumbnails::with_disk(device.clone(), queue, Arc::default(), None);
        let path = directory.path().join("corrected.ARW");
        let id = add_source(&mut editor, path.clone());
        editor.sync_document();
        editor.document.project.flat_field = Some(crate::project::FlatFieldSettings::new(
            editor.document.project.source(id).unwrap(),
            Arc::new(
                crate::flat_field::FlatField::from_nodes(
                    [12, 7, 864, 576],
                    [2, 2],
                    vec![DVec3::splat(1.1); 4],
                )
                .unwrap(),
            ),
            editor.document.project.development.as_ref().unwrap(),
        ));
        editor.correction = Correction::prepare(&device, &editor.document, None).unwrap();
        let before = editor.document.project.clone();
        editor.load(path);
        let proxy = &editor.pending_preview.as_ref().unwrap().proxy;
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            assert_eq!(
                proxy.image(target).unwrap().source_revision(),
                0,
                "correction belongs to the immutable initial source, not a later write"
            );
        }
        assert_eq!(editor.document.project, before);
        editor.correction = None;
        editor.validate_pending_preview();
        assert!(editor.pending_preview.is_none() && editor.busy && editor.ready.is_none());
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
        assert!(commands.try_recv().is_err());
    });
}

#[test]
fn authoritative_ready_replaces_pending_and_active_proxy_survives_optional_pressure() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, messages, _commands) = super::super::tests::fixture();
        editor.thumbnails = Thumbnails::with_disk(device, queue, Arc::default(), None);
        let path = directory.path().join("selected.ARW");
        let id = add_source(&mut editor, path.clone());
        editor.load(path.clone());
        editor
            .thumbnails
            .apply_budgets(memory::CacheBudgets::for_memory(
                48 << 30,
                memory::Pressure::Critical,
            ));
        assert_eq!(editor.thumbnails.sources.proxy_count(), 1);
        editor.validate_pending_preview();
        assert!(editor.pending_preview.is_some());
        let ready = Ready {
            thumbnail: None,
            path,
            area: editor.pending_preview.as_ref().unwrap().area(),
            analysis_support: None,
            floor: editor.floor,
            proxy: editor.pending_preview.as_ref().unwrap().proxy.clone(),
            record: editor.document.project.source(id).unwrap().clone(),
            development: editor.document.project.development.clone().unwrap(),
        };
        messages
            .send(Message::Ready(editor.id, Ok(Box::new(ready))))
            .unwrap();
        editor.receive();
        assert!(editor.ready.is_some() && editor.pending_preview.is_none() && !editor.busy);
        assert!(!editor.selected_view().unwrap().2);
        editor.source = false;
        assert_eq!(editor.scope_input().err(), Some("Waiting for view"));
        editor.last_layout = Some((
            editor.viewport.layout(
                egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800., 600.)),
                editor.ready.as_ref().unwrap().area,
                1.,
            ),
            1.,
        ));
        assert!(editor.scope_input().is_ok());
    });
}

#[test]
fn local_snapshots_reach_pending_previews_and_cached_thumbnails_without_rebuilding_sources() {
    use crate::{
        geometry::SourcePoint,
        local_adjustments::{GradientMask, LocalAdjustment},
        project::LocalAdjustments,
    };
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, _, commands) = super::super::tests::fixture();
        let renderers = Arc::new(crate::renderer::RendererCache::default());
        editor.thumbnails =
            Thumbnails::with_disk(device.clone(), queue.clone(), renderers.clone(), None);
        editor.grades = Grades::new(device.clone());
        let path = directory.path().join("local.ARW");
        let id = add_source(&mut editor, path.clone());
        let source_key = SourceKey::new(
            editor.document.project.source(id).unwrap(),
            editor.document.project.development.as_ref().unwrap(),
            FileKey::new(&path),
        );
        let retained = editor
            .thumbnails
            .sources
            .get_proxy(&source_key.store_key_at(&path).unwrap())
            .unwrap();
        let mut locals = LocalAdjustments::default();
        locals
            .add(
                "Linear".into(),
                LocalAdjustment {
                    mask: GradientMask::Linear {
                        start: SourcePoint([12., 7.]),
                        end: SourcePoint([876., 583.]),
                    },
                    photo: PhotoSettings {
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
            .local_adjustments = Some(locals);
        editor.load(path.clone());
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
        assert!(editor.pending_preview.is_some() && editor.busy && editor.ready.is_none());
        let captured = editor
            .positive_processing(editor.calibration().unwrap(), editor.roll, editor.photo)
            .unwrap();
        let context = egui::Context::default();
        let mut renderer = egui_wgpu::Renderer::new(
            &device,
            wgpu::TextureFormat::Rgba16Float,
            Default::default(),
        );
        for target in [OutputTarget::Srgb, OutputTarget::DisplayP3] {
            editor.target = target;
            let source = editor.document.project.source(id).unwrap();
            let thumbnail = retained
                .image(
                    &renderers.get(&device, &queue).unwrap(),
                    &device,
                    target,
                    &source.development,
                    None,
                )
                .await
                .unwrap();
            let key = ImageKey {
                source: source_key.clone(),
                target,
                field: None,
            };
            editor
                .thumbnails
                .cache
                .insert(key.clone(), Ok(thumbnail.clone()));
            let selected = editor
                .selected_view()
                .unwrap()
                .1
                .image(editor.target)
                .unwrap();
            assert!(!Arc::ptr_eq(&selected, &thumbnail));
            let source_revision = selected.source_revision();
            let thumbnail_revision = thumbnail.source_revision();
            for source_mode in [false, true] {
                editor.source = source_mode;
                editor.source_exposure = 2.;
                draw(
                    &device,
                    &queue,
                    &context,
                    &mut renderer,
                    frame(&mut editor, &context),
                );
                let positive = editor
                    .thumbnail_parameters(editor.document.project.source(id).unwrap())
                    .unwrap();
                assert_eq!(positive.signature(), captured.signature());
                let expected = if source_mode {
                    editor.source_parameters(2.).unwrap().into()
                } else {
                    positive.clone()
                };
                let mut encoder = device.create_command_encoder(&Default::default());
                assert!(
                    !selected
                        .prepare_processing(&device, &mut encoder, &expected)
                        .unwrap(),
                    "selected callback must use the full current input"
                );
                assert!(
                    !thumbnail
                        .prepare_processing(&device, &mut encoder, &positive)
                        .unwrap(),
                    "filmstrip callback must apply locals independently of source inspection"
                );
                queue.submit([encoder.finish()]);
                assert!(
                    commands.try_recv().is_err(),
                    "cached preview must not schedule full-source work"
                );
            }
            editor.source = false;
            editor
                .document
                .project
                .source_mut(id)
                .unwrap()
                .local_adjustments
                .as_mut()
                .unwrap()
                .adjustments[0]
                .adjustment
                .invert = true;
            draw(
                &device,
                &queue,
                &context,
                &mut renderer,
                frame(&mut editor, &context),
            );
            let edited = editor
                .thumbnail_parameters(editor.document.project.source(id).unwrap())
                .unwrap();
            assert_ne!(edited.signature(), captured.signature());
            let mut encoder = device.create_command_encoder(&Default::default());
            assert!(
                !selected
                    .prepare_processing(&device, &mut encoder, &edited)
                    .unwrap()
            );
            assert!(
                !thumbnail
                    .prepare_processing(&device, &mut encoder, &edited)
                    .unwrap()
            );
            queue.submit([encoder.finish()]);
            assert_eq!(selected.source_revision(), source_revision);
            assert_eq!(thumbnail.source_revision(), thumbnail_revision);
            assert!(Arc::ptr_eq(
                editor.thumbnails.cache.get(&key).unwrap().as_ref().unwrap(),
                &thumbnail
            ));
            assert!(Arc::ptr_eq(
                &editor
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
                .local_adjustments
                .as_mut()
                .unwrap()
                .adjustments[0]
                .adjustment
                .invert = false;
        }
        assert!(
            editor.document.validate().is_err(),
            "normal Open/Save remains guarded"
        );
    });
}

#[test]
fn roll_sampling_load_does_not_admit_pending_working_preview() {
    pollster::block_on(async {
        let (device, queue) = gpu().await;
        let directory = tempfile::tempdir().unwrap();
        let (mut editor, _, commands) = super::super::tests::fixture();
        editor.thumbnails = Thumbnails::with_disk(device, queue, Arc::default(), None);
        let path = directory.path().join("reference.ARW");
        let id = add_source(&mut editor, path);
        editor.pick = Some(Pick::Dmin);
        editor.pick_roll_source(id);
        assert!(editor.pending_preview.is_none() && editor.selection_timing.is_none());
        assert!(editor.ready.is_none() && editor.busy && editor.pick == Some(Pick::Dmin));
        assert!(matches!(commands.try_recv().unwrap(), Command::Load(..)));
    });
}
