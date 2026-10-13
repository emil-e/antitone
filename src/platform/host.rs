use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use super::{SubmissionGate, macos};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

pub trait App {
    /// The same gate used by every background submitter on this host's queue.
    fn submission_gate(&self) -> &SubmissionGate;
    fn ui(&mut self, ui: &mut egui::Ui);
    fn on_exit(&mut self) {}
}

pub fn run<A: App + 'static>(
    title: &str,
    create: impl FnOnce(&wgpu::Device, &wgpu::Queue) -> Result<A> + 'static,
) -> Result<u64> {
    let event_loop = EventLoop::<Repaint>::with_user_event().build()?;
    let context = egui::Context::default();
    context.set_embed_viewports(true);
    let proxy = event_loop.create_proxy();
    context.set_request_repaint_callback(move |request| {
        if request.viewport_id == egui::ViewportId::ROOT
            && let Some(when) = Instant::now().checked_add(request.delay)
        {
            let _ = proxy.send_event(Repaint(when));
        }
    });
    let mut host = Host {
        title: title.to_owned(),
        create: Some(create),
        context,
        state: None,
        next_repaint: None,
        error: None,
    };
    let outcome = event_loop.run_app(&mut host);
    if let Some(state) = &mut host.state {
        state.app.on_exit();
    }
    outcome?;
    if let Some(error) = host.error {
        return Err(error);
    }
    Ok(host.state.as_ref().map_or(0, |state| state.gpu.presented))
}

struct Repaint(Instant);
struct Host<A, F> {
    title: String,
    create: Option<F>,
    context: egui::Context,
    state: Option<State<A>>,
    next_repaint: Option<Instant>,
    error: Option<Box<dyn std::error::Error>>,
}

impl<A: App, F: FnOnce(&wgpu::Device, &wgpu::Queue) -> Result<A>> Host<A, F> {
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn std::error::Error>) {
        self.error = Some(error);
        event_loop.exit();
    }
}

impl<A: App, F: FnOnce(&wgpu::Device, &wgpu::Queue) -> Result<A>> ApplicationHandler<Repaint>
    for Host<A, F>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
            return;
        }
        let mut initialize = || -> Result<State<A>> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title(&self.title)
                        .with_inner_size(winit::dpi::LogicalSize::new(720.0, 320.0)),
                )?,
            );
            let gpu = pollster::block_on(Gpu::new(window.clone()))?;
            let app = self.create.take().expect("initialize once")(&gpu.device, &gpu.queue)?;
            let input = egui_winit::State::new(
                self.context.clone(),
                egui::ViewportId::ROOT,
                &window,
                Some(window.scale_factor() as f32),
                window.theme(),
                Some(gpu.device.limits().max_texture_dimension_2d as usize),
            );
            let mut viewport = egui::ViewportInfo::default();
            egui_winit::update_viewport_info(&mut viewport, &self.context, &window, true);
            window.request_redraw();
            Ok(State {
                gpu,
                app,
                input,
                viewport,
            })
        };
        match initialize() {
            Ok(state) => self.state = Some(state),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, Repaint(when): Repaint) {
        self.next_repaint = Some(self.next_repaint.map_or(when, |old| old.min(when)));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(state) = &mut self.state else {
            return;
        };
        if event_loop.exiting() || id != state.gpu.window.id() {
            return;
        }
        let redrawing = matches!(event, WindowEvent::RedrawRequested);
        let response = state.input.on_window_event(&state.gpu.window, &event);
        let outcome = match event {
            WindowEvent::CloseRequested => {
                state.viewport.events.push(egui::ViewportEvent::Close);
                state.gpu.window.request_redraw();
                Ok(())
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                state.gpu.needs_configure = true;
                state.gpu.window.request_redraw();
                Ok(())
            }
            WindowEvent::Occluded(occluded) => {
                println!(
                    "presentation window occluded={occluded} visible={:?} minimized={:?} focused={}",
                    state.gpu.window.is_visible(),
                    state.gpu.window.is_minimized(),
                    state.gpu.window.has_focus()
                );
                Ok(())
            }
            WindowEvent::RedrawRequested => state.draw(&self.context, event_loop),
            _ => Ok(()),
        };
        if response.repaint && !redrawing {
            state.gpu.window.request_redraw();
        }
        if let Err(error) = outcome {
            self.fail(event_loop, error);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(when) = self.next_repaint {
            if when <= Instant::now() {
                if let Some(state) = &self.state {
                    state.gpu.window.request_redraw();
                }
                self.next_repaint = None;
                event_loop.set_control_flow(ControlFlow::Wait);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(when));
            }
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

struct State<A> {
    gpu: Gpu,
    app: A,
    input: egui_winit::State,
    viewport: egui::ViewportInfo,
}

impl<A: App> State<A> {
    fn draw(&mut self, context: &egui::Context, event_loop: &ActiveEventLoop) -> Result<()> {
        let size = self.gpu.window.inner_size();
        let frame = if size.width == 0 || size.height == 0 {
            None
        } else {
            if self.gpu.needs_configure
                || size.width != self.gpu.config.width
                || size.height != self.gpu.config.height
            {
                self.gpu.configure(self.app.submission_gate())?;
            }
            match self.gpu.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    self.gpu.needs_configure = true;
                    Some(frame)
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    self.gpu.configure(self.app.submission_gate())?;
                    self.gpu.window.request_redraw();
                    None
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    self.gpu.surface = self.gpu.instance.create_surface(self.gpu.window.clone())?;
                    self.gpu.configure(self.app.submission_gate())?;
                    self.gpu.window.request_redraw();
                    None
                }
                wgpu::CurrentSurfaceTexture::Timeout => {
                    context.request_repaint_after(Duration::from_millis(16));
                    None
                }
                wgpu::CurrentSurfaceTexture::Occluded => None,
                wgpu::CurrentSurfaceTexture::Validation => {
                    return Err("surface acquisition validation failed".into());
                }
            }
        };
        // Input and close/cancel commands still run when a drawable is unavailable.
        egui_winit::update_viewport_info(&mut self.viewport, context, &self.gpu.window, false);
        let mut raw = self.input.take_egui_input(&self.gpu.window);
        raw.viewports
            .insert(egui::ViewportId::ROOT, self.viewport.clone());
        let mut output = context.run_ui(raw, |ui| self.app.ui(ui));
        self.input.handle_platform_output_with_event_loop(
            &self.gpu.window,
            event_loop,
            output.platform_output,
        );
        let jobs = context.tessellate(output.shapes, output.pixels_per_point);
        for (id, deltas) in output.textures_delta.set.drain() {
            for delta in deltas {
                self.gpu
                    .renderer
                    .update_texture(&self.gpu.device, &self.gpu.queue, id, &delta);
            }
        }
        if let Some(frame) = frame {
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [self.gpu.config.width, self.gpu.config.height],
                pixels_per_point: output.pixels_per_point,
            };
            let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
            let commands = self.gpu.renderer.update_buffers(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                &jobs,
                &screen,
            );
            let view = frame.texture.create_view(&Default::default());
            {
                let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("managed image and egui UI"),
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
                self.gpu
                    .renderer
                    .render(&mut pass.forget_lifetime(), &jobs, &screen);
            }
            self.app.submission_gate().submit(
                &self.gpu.queue,
                commands.into_iter().chain([encoder.finish()]),
            );
            self.gpu.window.pre_present_notify();
            self.gpu.queue.present(frame);
            self.gpu.presented += 1;
        }
        for id in output.textures_delta.free.drain() {
            self.gpu.renderer.free_texture(&id);
        }
        if let Some(root) = output.viewport_output.get(&egui::ViewportId::ROOT) {
            let mut actions = Vec::new();
            egui_winit::process_viewport_commands(
                context,
                &mut self.viewport,
                root.commands.clone(),
                &self.gpu.window,
                &mut actions,
            );
            if root.commands.contains(&egui::ViewportCommand::CancelClose) {
                self.viewport
                    .events
                    .retain(|e| *e != egui::ViewportEvent::Close);
            }
            for action in actions {
                let event = match action {
                    egui_winit::ActionRequested::Cut => egui::Event::Cut,
                    egui_winit::ActionRequested::Copy => egui::Event::Copy,
                    egui_winit::ActionRequested::Paste => {
                        let Some(text) = self.input.clipboard_text() else {
                            continue;
                        };
                        egui::Event::Paste(text)
                    }
                    egui_winit::ActionRequested::Screenshot(_) => {
                        return Err("host screenshots are not implemented".into());
                    }
                };
                self.input.egui_input_mut().events.push(event);
                self.gpu.window.request_redraw();
            }
        }
        if self.viewport.close_requested() {
            event_loop.exit();
        }
        self.viewport.events.clear();
        Ok(())
    }
}

struct Gpu {
    surface: wgpu::Surface<'static>,
    window: Arc<Window>,
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    renderer: egui_wgpu::Renderer,
    needs_configure: bool,
    presented: u64,
}

impl Gpu {
    async fn new(window: Arc<Window>) -> Result<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let surface = instance.create_surface(window.clone())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                compatible_surface: Some(&surface),
                ..Default::default()
            })
            .await?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_limits: super::device_limits(adapter.limits()),
                ..Default::default()
            })
            .await?;
        let size = window.inner_size();
        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or("surface configuration unavailable")?;
        config.format = wgpu::TextureFormat::Rgba16Float;
        config.view_formats = vec![config.format];
        let renderer = egui_wgpu::Renderer::new(
            &device,
            config.format,
            egui_wgpu::RendererOptions {
                dithering: false,
                ..Default::default()
            },
        );
        let mut gpu = Self {
            surface,
            window,
            instance,
            adapter,
            device,
            queue,
            config,
            renderer,
            needs_configure: true,
            presented: 0,
        };
        // No application or background submitters exist during initial configuration.
        gpu.configure(&SubmissionGate::default())?;
        println!(
            "presentation adapter={:?} target={:?}",
            gpu.adapter.get_info(),
            gpu.config.format
        );
        Ok(gpu)
    }

    fn configure(&mut self, submissions: &SubmissionGate) -> Result<()> {
        if !self
            .surface
            .get_capabilities(&self.adapter)
            .formats
            .contains(&self.config.format)
        {
            return Err(
                "RGBA16Float surface unavailable; narrower presentation is not substituted".into(),
            );
        }
        let size = self.window.inner_size();
        if size.width == 0 || size.height == 0 {
            return Ok(());
        }
        self.config.width = size.width;
        self.config.height = size.height;
        println!(
            "presentation native_window visible={:?} minimized={:?} focused={}",
            self.window.is_visible(),
            self.window.is_minimized(),
            self.window.has_focus()
        );
        submissions.configure(|| self.surface.configure(&self.device, &self.config));
        macos::configure_sdr(&self.surface)?;
        self.needs_configure = false;
        Ok(())
    }
}
