//! Checks managed macOS Metal presentation against generated Core Image transfer references.
//! Requires transfer artifacts in `.cache/probes` and a desktop for the resize/window route.
//! Gates f32/half and UI readbacks; noninteractive execution also requires submitted visible frames.
//! Shows a chart (`--interactive` keeps it open); does not establish physical-monitor matching.
use std::sync::Arc;

#[cfg(target_os = "macos")]
use antitone::platform::host;
use glam::{DMat3, DVec3, Vec4};
use wgpu::util::DeviceExt;

type ProbeResult<T> = Result<T, Box<dyn std::error::Error>>;

fn main() -> ProbeResult<()> {
    #[cfg(not(target_os = "macos"))]
    return Err("managed presentation is currently implemented for macOS only".into());
    #[cfg(target_os = "macos")]
    {
        let interactive = std::env::args().any(|v| v == "--interactive");
        let submission_stress = std::env::args().any(|v| v == "--submission-stress");
        let result = Arc::new(std::sync::Mutex::new(None));
        let report = result.clone();
        let presented = host::run(
            "Antitone — managed presentation probe",
            move |device, queue| {
                pollster::block_on(validate_transfer(
                    device,
                    queue,
                    wgpu::TextureFormat::Rgba32Float,
                    2e-5,
                ))?;
                pollster::block_on(validate_transfer(
                    device,
                    queue,
                    wgpu::TextureFormat::Rgba16Float,
                    0.001,
                ))?;
                pollster::block_on(validate_ui(device, queue))?;
                let submissions = antitone::platform::SubmissionGate::default();
                let worker = submission_stress
                    .then(|| SubmissionWorker::new(device, queue, submissions.clone(), report));
                Ok(Probe {
                    chart: Arc::new(Chart::new(
                        device,
                        &chart_patches(),
                        wgpu::TextureFormat::Rgba16Float,
                    )),
                    frame: 0,
                    submissions,
                    worker,
                    last_size: None,
                    resizes: 0,
                    interactive,
                })
            },
        )?;
        if submission_stress {
            let (submitted, resizes) = result
                .lock()
                .unwrap()
                .take()
                .ok_or("submission worker did not report completion")??;
            if submitted == 0 {
                return Err("submission worker did no GPU work".into());
            }
            if resizes < 2 {
                return Err("submission stress observed fewer than two native resizes".into());
            }
            println!("presentation background_submissions={submitted} native_resizes={resizes}");
        }
        println!("presentation submitted_frames={presented}");
        if presented == 0 && !interactive {
            return Err("window remained occluded; offscreen checks passed but onscreen presentation was not exercised".into());
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
type SubmissionReport = Arc<std::sync::Mutex<Option<Result<(u64, u32), String>>>>;

#[cfg(target_os = "macos")]
struct SubmissionWorker {
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: std::thread::JoinHandle<Result<u64, String>>,
    report: SubmissionReport,
}

#[cfg(target_os = "macos")]
impl SubmissionWorker {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: antitone::platform::SubmissionGate,
        report: SubmissionReport,
    ) -> Self {
        let device = device.clone();
        let queue = queue.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = std::thread::spawn(move || {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("surface configure submission stress"),
                size: 4096,
                usage: wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let mut count = 0;
            while !stopping.load(std::sync::atomic::Ordering::Acquire) {
                let mut encoder = device.create_command_encoder(&Default::default());
                encoder.clear_buffer(&buffer, 0, None);
                let index = submissions.submit(&queue, [encoder.finish()]);
                device
                    .poll(wgpu::PollType::Wait {
                        submission_index: Some(index),
                        timeout: None,
                    })
                    .map_err(|error| error.to_string())?;
                count += 1;
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Ok(count)
        });
        Self {
            stop,
            thread,
            report,
        }
    }

    fn finish(self, resizes: u32) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        let outcome = self
            .thread
            .join()
            .map_err(|_| "submission worker panicked".to_owned())
            .and_then(|outcome| outcome);
        *self.report.lock().unwrap() = Some(outcome.map(|submitted| (submitted, resizes)));
    }
}

struct Probe {
    chart: Arc<Chart>,
    submissions: antitone::platform::SubmissionGate,
    #[cfg(target_os = "macos")]
    worker: Option<SubmissionWorker>,
    last_size: Option<egui::Vec2>,
    resizes: u32,
    frame: usize,
    interactive: bool,
}
#[cfg(target_os = "macos")]
impl host::App for Probe {
    fn submission_gate(&self) -> &antitone::platform::SubmissionGate {
        &self.submissions
    }

    fn on_exit(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.finish(self.resizes);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui) {
        self.frame += 1;
        if let Some(size) = ui.input(|input| input.viewport().inner_rect.map(|rect| rect.size())) {
            if self.last_size.is_some_and(|previous| previous != size) {
                self.resizes += 1;
            }
            self.last_size = Some(size);
        }
        ui.heading("Managed wide-gamut SDR probe");
        ui.label(
            "sRGB white / gray / sRGB red / P3 red / sRGB green / P3 green / sRGB blue / P3 blue",
        );
        ui.label("RGBA16Float • extended sRGB • EDR disabled • no application monitor transform");
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), 160.0),
            egui::Sense::hover(),
        );
        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            Paint(self.chart.clone()),
        ));
        ui.label("Layer configuration and transfer/readback are tested. Physical monitor matching is not yet proven.");
        if !self.interactive {
            if self.worker.is_some() {
                if self.frame == 1 {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                if self.frame < 64 {
                    ui.ctx()
                        .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                            760.,
                            360. + (self.frame % 2) as f32,
                        )));
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(4));
                } else {
                    println!("presentation submission_stress UI passes={}", self.frame);
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return;
            }
            if self.frame == 2 {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(640.0, 340.0)));
            }
            if self.frame == 4 {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(760.0, 360.0)));
            }
            if self.frame >= 12 {
                println!(
                    "presentation UI passes={} (initialization and two requested resizes)",
                    self.frame
                );
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ui.ctx().request_repaint();
        }
    }
}

fn chart_patches() -> Vec<Vec4> {
    let p3_to_srgb = DMat3::from_cols(
        DVec3::new(1.22494017628056, -0.04205695470969, -0.01963755459033),
        DVec3::new(-0.22494017628056, 1.04205695470969, -0.07863604555063),
        DVec3::new(0.0, 0.0, 1.09827360014097),
    );
    [
        DVec3::ONE,
        DVec3::splat(0.18),
        DVec3::X,
        p3_to_srgb * DVec3::X,
        DVec3::Y,
        p3_to_srgb * DVec3::Y,
        DVec3::Z,
        p3_to_srgb * DVec3::Z,
    ]
    .map(|v| v.as_vec3().extend(1.0))
    .to_vec()
}

struct Chart {
    pipeline: wgpu::RenderPipeline,
    group: wgpu::BindGroup,
}
impl Chart {
    fn new(device: &wgpu::Device, patches: &[Vec4], format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("extended sRGB presentation"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/presentation.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("managed float presentation"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("linear extended sRGB patches"),
            contents: bytemuck::cast_slice(patches),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("presentation patches"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        Self { pipeline, group }
    }
}
struct Paint(Arc<Chart>);
impl egui_wgpu::CallbackTrait for Paint {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        pass.set_pipeline(&self.0.pipeline);
        pass.set_bind_group(0, &self.0.group, &[]);
        pass.draw(0..3, 0..1);
    }
}

fn target(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("presentation reference target"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

async fn validate_transfer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    tolerance: f32,
) -> ProbeResult<()> {
    let input = std::fs::read(".cache/probes/presentation-linear-srgb.f32")?;
    let reference = std::fs::read(".cache/probes/presentation-encoded-srgb-reference.f32")?;
    if input.is_empty() || input.len() % 16 != 0 || reference.len() != input.len() {
        return Err(
            "invalid native presentation reference; run scripts/probe-presentation-reference.sh"
                .into(),
        );
    }
    let patches: Vec<Vec4> = input
        .as_chunks::<16>()
        .0
        .iter()
        .map(|bytes| bytemuck::pod_read_unaligned(bytes))
        .collect();
    let chart = Chart::new(device, &patches, format);
    let texture = target(device, patches.len() as u32, 1, format);
    let mut encoder = device.create_command_encoder(&Default::default());
    let view = texture.create_view(&Default::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("transfer probe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&chart.pipeline);
        pass.set_bind_group(0, &chart.group, &[]);
        pass.draw(0..3, 0..1);
    }
    let actual = readback(device, queue, &texture, encoder).await?;
    let mut maximum = 0.0f32;
    for (i, expected) in reference.as_chunks::<4>().0.iter().enumerate() {
        let actual = actual[i / 4][i % 4];
        let expected = f32::from_le_bytes(*expected);
        maximum = maximum.max((actual - expected).abs());
        if !actual.is_finite() || !expected.is_finite() || (actual - expected).abs() > tolerance {
            return Err(format!("presentation encoding mismatch: {actual} != {expected}").into());
        }
    }
    println!(
        "GPU_{format:?}_vs_CoreImage_encoding_max_error={maximum:.8e} pixels={} (negative and above-one values included)",
        patches.len()
    );
    Ok(())
}

async fn validate_ui(device: &wgpu::Device, queue: &wgpu::Queue) -> ProbeResult<()> {
    let context = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
    let mut output = context.run_ui(
        egui::RawInput {
            screen_rect: Some(rect),
            ..Default::default()
        },
        |ui| {
            ui.painter()
                .rect_filled(rect, 0.0, egui::Color32::from_rgb(64, 128, 192));
        },
    );
    let jobs = context.tessellate(output.shapes, output.pixels_per_point);
    let texture = target(device, 64, 64, wgpu::TextureFormat::Rgba16Float);
    let mut renderer = egui_wgpu::Renderer::new(
        device,
        texture.format(),
        egui_wgpu::RendererOptions {
            dithering: false,
            ..Default::default()
        },
    );
    for (id, deltas) in output.textures_delta.set.drain() {
        for delta in deltas {
            renderer.update_texture(device, queue, id, &delta);
        }
    }
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [64, 64],
        pixels_per_point: output.pixels_per_point,
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    let commands = renderer.update_buffers(device, queue, &mut encoder, &jobs, &screen);
    let view = texture.create_view(&Default::default());
    {
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI encoding probe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
    }
    queue.submit(commands);
    let pixels = readback(device, queue, &texture, encoder).await?;
    let actual = pixels[32 * 64 + 32];
    let expected = Vec4::new(64.0 / 255.0, 128.0 / 255.0, 192.0 / 255.0, 1.0);
    for c in 0..4 {
        let quantized = half::f16::from_f32(expected[c]).to_f32();
        if actual[c] != quantized {
            return Err(format!(
                "egui encoded UI mismatch channel={c}: {} != {quantized}",
                actual[c]
            )
            .into());
        }
    }
    for id in output.textures_delta.free.drain() {
        renderer.free_texture(&id);
    }
    println!("egui_UI_encoding=encoded_sRGB_once RGBA16Float_readback={actual:?}");
    Ok(())
}

async fn readback(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mut encoder: wgpu::CommandEncoder,
) -> ProbeResult<Vec<Vec4>> {
    let component_bytes = match texture.format() {
        wgpu::TextureFormat::Rgba16Float => 2,
        wgpu::TextureFormat::Rgba32Float => 4,
        _ => return Err("unsupported probe readback format".into()),
    };
    let pixel_bytes = component_bytes * 4;
    let row_bytes = (texture.width() * pixel_bytes).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("presentation readback"),
        size: u64::from(row_bytes) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row_bytes),
                rows_per_image: Some(texture.height()),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        sender.send(r).unwrap();
    });
    device.poll(wgpu::PollType::wait_indefinitely())?;
    receiver.recv()??;
    let mapped = buffer.slice(..).get_mapped_range()?;
    let mut result = Vec::with_capacity((texture.width() * texture.height()) as usize);
    for y in 0..texture.height() as usize {
        let row = &mapped[y * row_bytes as usize
            ..y * row_bytes as usize + texture.width() as usize * pixel_bytes as usize];
        for pixel in row.chunks_exact(pixel_bytes as usize) {
            result.push(Vec4::from_array(std::array::from_fn(|c| {
                if component_bytes == 2 {
                    half::f16::from_bits(u16::from_le_bytes([pixel[c * 2], pixel[c * 2 + 1]]))
                        .to_f32()
                } else {
                    f32::from_le_bytes([
                        pixel[c * 4],
                        pixel[c * 4 + 1],
                        pixel[c * 4 + 2],
                        pixel[c * 4 + 3],
                    ])
                }
            })));
        }
    }
    Ok(result)
}
