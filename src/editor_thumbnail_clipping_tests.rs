use super::*;

const WIDTH: f32 = 768.;

fn frame(editor: &mut Editor, context: &egui::Context, time: f64, scroll: f32) -> egui::FullOutput {
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(WIDTH, 800.),
            )),
            time: Some(time),
            events: vec![
                egui::Event::PointerMoved(egui::pos2(256., 750.)),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    phase: egui::TouchPhase::Move,
                    delta: egui::vec2(scroll, 0.),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        },
        |ui| editor.filmstrip(ui),
    );
    output.textures_delta.clear();
    output
}

fn names(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Text(text) = &shape.shape
                && text.galley.job.text.starts_with("photo-")
            {
                Some((text.galley.job.text.clone(), text.pos))
            } else {
                None
            }
        })
        .collect()
}

fn callbacks(output: &egui::FullOutput) -> Vec<(egui::Rect, egui::Rect)> {
    output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Callback(callback) = &shape.shape {
                Some((callback.rect, shape.clip_rect))
            } else {
                None
            }
        })
        .collect()
}

fn paint(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    context: &egui::Context,
    renderer: &mut egui_wgpu::Renderer,
    output: egui::FullOutput,
) -> Vec<[u16; 4]> {
    let pixels_per_point = output.pixels_per_point;
    let width = (WIDTH * pixels_per_point) as u32;
    let height = (800. * pixels_per_point) as u32;
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [width, height],
        pixels_per_point,
    };
    let jobs = context.tessellate(
        output
            .shapes
            .into_iter()
            .filter(|shape| matches!(shape.shape, egui::Shape::Callback(_)))
            .collect(),
        pixels_per_point,
    );
    let mut encoder = device.create_command_encoder(&Default::default());
    let commands = renderer.update_buffers(device, queue, &mut encoder, &jobs, &screen);
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("thumbnail clipping regression"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
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
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("thumbnail clipping pixels"),
        size: u64::from(width) * u64::from(height) * 8,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 8),
                rows_per_image: Some(height),
            },
        },
        target.size(),
    );
    queue.submit(commands.into_iter().chain([encoder.finish()]));
    let (send, receive) = mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        send.send(result).unwrap()
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receive.recv().unwrap().unwrap();
    let pixels = bytemuck::cast_slice(&buffer.get_mapped_range(..).unwrap()).to_vec();
    buffer.unmap();
    pixels
}

#[test]
fn scrolled_thumbnail_edges_match_crops_of_the_unclipped_gpu_image() {
    pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance.request_adapter(&Default::default()).await.unwrap();
        let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
        let shared = crate::renderer::GpuRenderer::new(&device, &queue)
            .await
            .unwrap();
        // Odd dimensions avoid nearest-texel ties when comparing clipped and full viewports.
        let pixels = (0..17)
            .flat_map(|y| {
                (0..25).map(move |x| {
                    [
                        12000 + x * 900,
                        15000 + y * 1000,
                        4000 + x * 500 + y * 120,
                        0,
                    ]
                })
            })
            .collect::<Vec<[u16; 4]>>();
        let image = Arc::new(
            GpuImage::new_shared(
                &shared,
                &device,
                25,
                17,
                &pixels,
                glam::DMat3::IDENTITY,
                OutputTarget::DisplayP3,
            )
            .await
            .unwrap(),
        );
        for height in [80., 140., 210.] {
            for scale in [1., 2.] {
                let (mut editor, _, _commands) = crate::editor::tests::fixture();
                editor.document = super::tests::fixture();
                editor.dmin = None;
                editor.dmax = None;
                editor.busy = true;
                editor.preferences.filmstrip_height = height;
                let before = editor.document.project.clone();
                for id in editor.document.project.order.clone() {
                    editor
                        .thumbnails
                        .cache
                        .insert(super::tests::key(&editor.document, id), Ok(image.clone()));
                }
                let context = egui::Context::default();
                context.set_pixels_per_point(scale);
                let mut renderer = egui_wgpu::Renderer::new(
                    &device,
                    wgpu::TextureFormat::Rgba16Float,
                    Default::default(),
                );
                for step in 0..3 {
                    frame(&mut editor, &context, f64::from(step) / 60., 0.);
                }
                let output = frame(&mut editor, &context, 3. / 60., 0.);
                let references = callbacks(&output);
                let reference_rect = references[0].0;
                let reference_name = names(&output)[0].1;
                let reference = paint(&device, &queue, &context, &mut renderer, output);
                let mut output = frame(
                    &mut editor,
                    &context,
                    4. / 60.,
                    -reference_rect.width() * 0.8,
                );
                for step in 5..40 {
                    output = frame(&mut editor, &context, f64::from(step) / 60., 0.);
                }
                let labels = names(&output);
                let draws = callbacks(&output);
                let actual = paint(&device, &queue, &context, &mut renderer, output);
                let width = (WIDTH * scale) as usize;
                let y = (reference_rect.center().y * scale).floor() as usize;
                let mut edges = [false; 2];
                for ((name, position), (draw, clip)) in labels.iter().zip(draws.iter()) {
                    let full =
                        reference_rect.translate(egui::vec2(position.x - reference_name.x, 0.));
                    let visible = full.intersect(*clip);
                    if visible.width() >= full.width() {
                        continue;
                    }
                    let edge = if full.left() < clip.left() { 0 } else { 1 };
                    edges[edge] = true;
                    let pixel_left = (full.left() * scale).round() as isize;
                    let pixel_width = (full.right() * scale).round() as isize - pixel_left;
                    let reference_rect = references
                        .iter()
                        .map(|(rect, _)| *rect)
                        .find(|rect| {
                            rect.width() == full.width()
                                && (rect.right() * scale).round() as isize
                                    - (rect.left() * scale).round() as isize
                                    == pixel_width
                        })
                        .expect("a full thumbnail with the same pixel-rounded width");
                    let shift = pixel_left - (reference_rect.left() * scale).round() as isize;
                    let start = (visible.left() * scale).ceil() as usize + 1;
                    let end = (visible.right() * scale).floor() as usize - 1;
                    for x in start..end {
                        let expected_x = (x as isize - shift) as usize;
                        assert_eq!(
                            actual[y * width + x],
                            reference[y * width + expected_x],
                            "{height}pt / {scale}x / {name} / draw={draw:?} / full={full:?} / reference={reference_rect:?} / label={position:?} / reference label={reference_name:?} / shift={shift} / x={x}"
                        );
                    }
                }
                assert_eq!(
                    edges,
                    [true, true],
                    "must exercise both scroll edges at {height}pt / {scale}x"
                );
                assert_eq!(editor.document.project, before);
                assert_eq!(editor.preferences.filmstrip_height, height);
            }
        }
    });
}
