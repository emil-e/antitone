use super::*;
use egui_kittest::{
    Harness, TestRenderer,
    kittest::{NodeT, Queryable},
};
use std::{cell::RefCell, rc::Rc, time::Instant};

#[test]
fn parameter_reset_click_respects_disabled_state() {
    let mut harness = Harness::builder().with_max_steps(12).build_ui_state(
        |ui, state: &mut (f64, bool)| {
            ui.add_enabled_ui(state.1, |ui| {
                controls::parameter(ui, controls::Parameter::Contrast, &mut state.0);
            });
        },
        (1.4, true),
    );
    harness.get_by_label("Reset Contrast").click();
    harness.run();
    assert_eq!(harness.state().0, 1.);
    *harness.state_mut() = (1.4, false);
    harness.run();
    assert!(
        harness
            .get_by_label("Reset Contrast")
            .accesskit_node()
            .is_disabled()
    );
    harness.get_by_label("Reset Contrast").click();
    harness.run();
    assert_eq!(harness.state().0, 1.4);
}

#[derive(Default)]
struct FloatCapture {
    size: [u32; 2],
    pixels: Vec<[f32; 4]>,
}

// Kittest's built-in readback assumes RGBA8. Retain the host's RGBA16Float path;
// only the optional inspection PNG is quantized/clipped, not the numeric readback.
struct FloatRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    submissions: crate::platform::SubmissionGate,
    renderer: egui_wgpu::Renderer,
    capture: Rc<RefCell<FloatCapture>>,
}
impl FloatRenderer {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: crate::platform::SubmissionGate,
        capture: Rc<RefCell<FloatCapture>>,
    ) -> Self {
        Self {
            device: device.clone(),
            queue: queue.clone(),
            submissions,
            capture,
            renderer: egui_wgpu::Renderer::new(
                device,
                wgpu::TextureFormat::Rgba16Float,
                egui_wgpu::RendererOptions {
                    dithering: false,
                    ..Default::default()
                },
            ),
        }
    }
}
impl TestRenderer for FloatRenderer {
    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        for (id, updates) in delta.set.drain() {
            for update in updates {
                self.renderer
                    .update_texture(&self.device, &self.queue, id, &update);
            }
        }
        for id in delta.free.drain() {
            self.renderer.free_texture(&id);
        }
    }
    fn render(
        &mut self,
        ctx: &egui::Context,
        output: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        let size = ctx.content_rect().size() * output.pixels_per_point;
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.x.round() as u32, size.y.round() as u32],
            pixels_per_point: output.pixels_per_point,
        };
        let [width, height] = screen.size_in_pixels;
        let jobs = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kittest offscreen RGBA16Float"),
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
        let view = texture.create_view(&Default::default());
        let row = (width * 8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kittest half-float readback"),
            size: u64::from(row) * u64::from(height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let commands =
            self.renderer
                .update_buffers(&self.device, &self.queue, &mut encoder, &jobs, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("kittest production callbacks and egui"),
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
            self.renderer
                .render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            texture.size(),
        );
        self.submissions
            .submit(&self.queue, commands.into_iter().chain([encoder.finish()]));
        let (send, receive) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(10)),
            })
            .map_err(|e| e.to_string())?;
        receive
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .map_err(|e| e.to_string())?;
        let mut pixels = Vec::with_capacity((width * height) as usize);
        let mut png = image::RgbaImage::new(width, height);
        for (y, bytes) in mapped.chunks_exact(row as usize).enumerate() {
            for (x, code) in bytes[..(width * 8) as usize]
                .as_chunks::<8>()
                .0
                .iter()
                .enumerate()
            {
                let pixel: [f32; 4] = std::array::from_fn(|c| {
                    half::f16::from_le_bytes(code[c * 2..c * 2 + 2].try_into().unwrap()).to_f32()
                });
                if pixel.iter().any(|v| !v.is_finite()) {
                    return Err("nonfinite offscreen pixel".into());
                }
                pixels.push(pixel);
                png.put_pixel(
                    x as u32,
                    y as u32,
                    image::Rgba(pixel.map(|v| (v.clamp(0., 1.) * 255.).round() as u8)),
                );
            }
        }
        drop(mapped);
        buffer.unmap();
        *self.capture.borrow_mut() = FloatCapture {
            size: [width, height],
            pixels,
        };
        Ok(png)
    }
}

fn wait_for(harness: &mut Harness<'_, Editor>, done: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(harness.state()) {
        assert!(
            Instant::now() < deadline,
            "Editor wait timed out: {}",
            harness.state().status
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    harness.step();
}

#[test]
fn dmin_picker_cancels_or_applies_measurement_and_paints_source() {
    let (device, queue) = pollster::block_on(async {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = instance
            .request_adapter(&Default::default())
            .await
            .expect("composed UI test requires Metal");
        adapter.request_device(&Default::default()).await.unwrap()
    });
    let mut editor = Editor::new(&device, &queue, None);
    editor.thumbnails = Thumbnails::default();
    let capture = Rc::new(RefCell::new(FloatCapture::default()));
    let renderer = FloatRenderer::new(
        &device,
        &queue,
        host::App::submission_gate(&editor).clone(),
        capture.clone(),
    );
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1100., 800.))
        .with_max_steps(240)
        .with_step_dt(1. / 60.)
        .renderer(renderer)
        .build_ui_state(|ui, editor| host::App::ui(editor, ui), editor);
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/raw/patches.dng");
    harness.state_mut().add_raws(vec![fixture.clone()]);
    wait_for(&mut harness, |e| !e.busy && e.document_pending.is_none());
    assert!(
        harness.state().ready.is_some(),
        "{}",
        harness.state().status
    );
    assert!(harness.state().dmin.is_none());
    let project = harness.state().document.project.clone();
    harness.get_by_label("Pick Dmin (film base)").scroll_to_me();
    harness.try_run_realtime().unwrap();
    harness.get_by_label("Pick Dmin (film base)").click();
    harness.step();
    assert_eq!(harness.state().pick, Some(Pick::Dmin));
    assert!(harness.get_by_label("Apply").accesskit_node().is_disabled());
    harness.get_by_label("Cancel").click();
    harness.step();
    assert!(harness.state().pick.is_none());
    assert_eq!(harness.state().document.project, project);
    harness.get_by_label("Pick Dmin (film base)").click();
    harness.step();
    assert_eq!(harness.state().pick, Some(Pick::Dmin));
    let point = harness
        .state()
        .last_layout
        .unwrap()
        .0
        .screen_point(egui::pos2(32., 32.));
    harness.hover_at(point);
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.step();
    assert!(harness.state().region.is_some());
    harness.get_by_label("Apply").click();
    harness.step();
    assert!(harness.state().busy);
    wait_for(&mut harness, |e| !e.busy);
    assert!(harness.state().pick.is_none());
    assert_eq!(harness.state().status, "Dmin measured");
    let measured = harness.state().dmin.unwrap();
    assert!((measured - DVec3::splat(3000. / 4031.)).abs().max_element() <= 2. / 65535.);
    assert_eq!(harness.state().roll, project.roll);
    let mut expected_project = project;
    expected_project.dmin = Some(measured.to_array());
    assert_eq!(harness.state().document.project, expected_project);
    assert!(harness.state().preference_store.is_none());
    let timing = Arc::new(SelectionTiming::new(harness.state().id, Instant::now()));
    harness.state_mut().selection_timing = Some(timing.clone());
    harness.step();
    let png = harness.render().unwrap();
    assert!(
        timing.ready_was_painted(),
        "production selected-image callback did not paint"
    );
    let geometry = harness.state().last_layout.unwrap().0;
    let output = capture.borrow();
    assert_eq!(png.dimensions(), (output.size[0], output.size[1]));
    assert!(harness.state().source);
    assert_eq!(harness.state().source_exposure, 0.);
    let metadata = crate::raw::RawFile::open(&fixture).unwrap().metadata;
    let target = harness.state().target;
    let cpu = crate::rendering::CpuRenderer::new(target).unwrap();
    for (source, code) in [(egui::pos2(32., 32.), 3000.), (egui::pos2(96., 32.), 128.)] {
        let position = geometry.screen_point(source);
        let pixel = output.pixels
            [position.y.floor() as usize * output.size[0] as usize + position.x.floor() as usize];
        // Source inspection bypasses inversion, not ACES or managed presentation encoding.
        let working = metadata.camera_to_ap1().unwrap() * DVec3::splat(code / 4031.);
        let linear = target.linear_to_srgb() * cpu.render(&[working]).unwrap()[0].as_dvec3();
        for c in 0..3 {
            let value = linear[c].abs();
            let expected = linear[c].signum()
                * if value <= 0.0031308 {
                    value * 12.92
                } else {
                    1.055 * value.powf(1. / 2.4) - 0.055
                };
            assert!(
                (f64::from(pixel[c]) - expected).abs() <= 0.001,
                "{pixel:?} vs {expected}"
            );
        }
    }
    if let Some(directory) = std::env::var_os("ANTITONE_UI_CAPTURE") {
        let directory = PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        png.save(directory.join("authored-dmin-offscreen.png"))
            .unwrap();
    }
}
