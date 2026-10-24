use std::time::{Duration, Instant};
use std::{cell::RefCell, sync::Arc};

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use super::{
    SubmissionGate, fullscreen::PhotoFullscreen, latency::Trace, macos, menu, termination,
};
use crate::application::{Application, UiContext};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuitState {
    /// No accepted close continuation; a native request must be declined.
    Running,
    /// The queued close, user decision or its save continuation is still live.
    Pending,
    /// Accepted state may be closed. A clean document alone does not grant this.
    Approved,
}

pub trait App: Sized {
    /// The same gate used by every background submitter on this host's queue.
    fn submission_gate(&self) -> SubmissionGate;
    fn ui(&mut self, ui: &mut egui::Ui, access: &UiContext<'_, Self>);
    fn menu_state(&self) -> Option<menu::State> {
        None
    }
    fn menu_action(&mut self, _action: menu::Action) {}
    fn menu_paste_requested(&self) -> bool {
        false
    }
    /// Leading text actions become input before this frame's focus-changing commands.
    fn menu_input(&mut self, _clipboard: Option<String>) -> Vec<egui::Event> {
        Vec::new()
    }
    fn native_menu_installed(&mut self) {}
    /// Opt in to native termination negotiation; approval must include dirty protection.
    fn quit_state(&self) -> Option<QuitState> {
        None
    }
    fn request_quit(&mut self) {}
    /// Requested source and processing/view generations, not accepted GPU output.
    /// Queried only by opt-in latency tracing.
    fn processing_revision(&self) -> Option<(u64, u64)> {
        None
    }
    /// Photo-only display coverage, independent of macOS's native fullscreen Space.
    fn photo_fullscreen(&self) -> bool {
        false
    }
    /// Delivered after window presentation is changed; UI restoration uses the next frame's bounds.
    fn photo_fullscreen_changed(&mut self, _active: bool) {}
    fn on_exit(&mut self) {}
}

pub fn run<A: App>(
    title: &str,
    create: impl for<'scope> FnOnce(&wgpu::Device, &wgpu::Queue, UiContext<'scope, A>) -> Result<A>,
) -> Result<u64> {
    let event_loop = EventLoop::<HostEvent>::with_user_event().build()?;
    let context = egui::Context::default();
    context.set_embed_viewports(true);
    let proxy = event_loop.create_proxy();
    let menu_proxy = proxy.clone();
    let quit_proxy = proxy.clone();
    muda::MenuEvent::set_event_handler(Some(move |event: muda::MenuEvent| {
        let _ = menu_proxy.send_event(HostEvent::Menu(event.id));
    }));
    context.set_request_repaint_callback(move |request| {
        if request.viewport_id == egui::ViewportId::ROOT
            && let Some(when) = Instant::now().checked_add(request.delay)
        {
            let _ = proxy.send_event(HostEvent::Repaint(when));
        }
    });
    let task_proxy = quit_proxy.clone();
    let repaint = context.clone();
    // This owner outlives the borrowing executor and every callback/task.
    let app = RefCell::new(None);
    let application = Application::new(
        move || {
            let _ = task_proxy.send_event(HostEvent::ExecutorWake);
        },
        move || repaint.request_repaint(),
    );
    let mut host = Host {
        title: title.to_owned(),
        create: Some(create),
        context,
        proxy: quit_proxy,
        application,
        app: &app,
        state: None,
        finishing: None,
        next_repaint: None,
        error: None,
    };
    let outcome = event_loop.run_app(&mut host);
    host.application.close();
    let trace_outcome = if let Some(state) = &mut host.state {
        state.cleanup()
    } else {
        Ok(())
    };
    outcome?;
    if let Some(error) = host.error.take() {
        return Err(error);
    }
    trace_outcome?;
    let presented = host.state.as_ref().map_or(0, |state| state.gpu.presented);
    let approved = host.finishing.is_some()
        || host
            .state
            .as_ref()
            .is_some_and(|state| state.app.read(App::quit_state) == Some(QuitState::Approved));
    let termination = host
        .state
        .as_mut()
        .and_then(|state| state.termination.take())
        .or_else(|| host.finishing.take());
    // An affirmative AppKit reply can exit the process without returning.
    // Release the application/window/GPU owners before completing that request.
    drop(host);
    if let Some(termination) = termination {
        termination.reply(approved);
    }
    Ok(presented)
}

enum HostEvent {
    ExecutorWake,
    Repaint(Instant),
    Menu(muda::MenuId),
    Quit,
}

struct Host<'scope, A, F> {
    title: String,
    create: Option<F>,
    context: egui::Context,
    proxy: EventLoopProxy<HostEvent>,
    application: Application<'scope>,
    app: &'scope RefCell<Option<A>>,
    state: Option<State<'scope, A>>,
    finishing: Option<termination::Guard>,
    next_repaint: Option<Instant>,
    error: Option<Box<dyn std::error::Error>>,
}

impl<A, F> Drop for Host<'_, A, F> {
    fn drop(&mut self) {
        // Cover unwinding and partially initialized hosts as well as normal exit.
        self.application.close();
        drop(self.app.borrow_mut().take());
    }
}

impl<'scope, A: App, F: FnOnce(&wgpu::Device, &wgpu::Queue, UiContext<'scope, A>) -> Result<A>>
    Host<'scope, A, F>
{
    fn fail(&mut self, event_loop: &ActiveEventLoop, error: Box<dyn std::error::Error>) {
        log::error!(error = error.to_string().as_str(); "host_failed");
        if let Some(guard) = self
            .state
            .as_ref()
            .and_then(|state| state.termination.as_ref())
            .or(self.finishing.as_ref())
        {
            guard.reply(false);
        }
        self.error = Some(error);
        event_loop.exit();
    }

    fn complete_native_quit(&mut self, event_loop: &ActiveEventLoop) {
        let mut state = self.state.take().expect("approved native host state");
        let termination = state
            .termination
            .take()
            .expect("active native termination guard");
        self.application.close();
        let outcome = state.cleanup();
        drop(self.app.borrow_mut().take());
        drop(state);
        match outcome {
            Ok(()) => {
                termination.approve_after_cleanup();
                self.finishing = Some(termination);
            }
            Err(error) => {
                termination.reply(false);
                self.fail(event_loop, error);
            }
        }
    }
}

impl<'scope, A: App, F: FnOnce(&wgpu::Device, &wgpu::Queue, UiContext<'scope, A>) -> Result<A>>
    ApplicationHandler<HostEvent> for Host<'scope, A, F>
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let _native_unwind = termination::cancel_on_unwind();
        if self.state.is_some() || self.finishing.is_some() {
            return;
        }
        let mut initialize = || -> Result<State<'scope, A>> {
            let window = Arc::new(
                event_loop.create_window(
                    Window::default_attributes()
                        .with_title(&self.title)
                        .with_inner_size(winit::dpi::LogicalSize::new(720.0, 320.0)),
                )?,
            );
            let gpu = pollster::block_on(Gpu::new(window.clone()))?;
            let access = self.application.access(self.app);
            let prepared = (|| -> Result<_> {
                let app = self.create.take().expect("initialize once")(
                    &gpu.device,
                    &gpu.queue,
                    access.clone(),
                )?;
                *self.app.borrow_mut() = Some(app);
                let termination = if access.read(App::quit_state).is_some() {
                    let proxy = self.proxy.clone();
                    Some(termination::Guard::install(move || {
                        proxy.send_event(HostEvent::Quit).is_ok()
                    })?)
                } else {
                    None
                };
                let menus = access
                    .read(App::menu_state)
                    .map(|state| menu::native::NativeMenu::new(&state))
                    .transpose()?;
                if menus.is_some() {
                    access.with_mut(App::native_menu_installed);
                }
                Ok((termination, menus))
            })();
            let (termination, menus) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    self.application.close();
                    drop(self.app.borrow_mut().take());
                    return Err(error);
                }
            };
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
                app: access,
                input,
                viewport,
                fullscreen: PhotoFullscreen::default(),
                menus,
                termination,
            })
        };
        match initialize() {
            Ok(state) => self.state = Some(state),
            Err(error) => self.fail(event_loop, error),
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: HostEvent) {
        let _native_unwind = termination::cancel_on_unwind();
        match event {
            HostEvent::ExecutorWake => self.application.drive(),
            HostEvent::Repaint(when) => {
                self.next_repaint = Some(self.next_repaint.map_or(when, |old| old.min(when)));
            }
            HostEvent::Menu(id) => {
                if let Some(state) = &mut self.state
                    && let Some(action) = state.menus.as_ref().and_then(|menus| menus.action(&id))
                {
                    state.reveal();
                    state.app.with_mut(|app| app.menu_action(action));
                }
            }
            HostEvent::Quit => {
                if let Some(state) = &mut self.state
                    && state
                        .termination
                        .as_ref()
                        .is_some_and(|guard| guard.delivered())
                {
                    state.reveal();
                    state.app.with_mut(App::request_quit);
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let _native_unwind = termination::cancel_on_unwind();
        let Some(state) = &mut self.state else {
            return;
        };
        if event_loop.exiting() || id != state.gpu.window.id() {
            return;
        }
        let redrawing = matches!(event, WindowEvent::RedrawRequested);
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::ModifiersChanged(_)
                | WindowEvent::Ime(_)
                | WindowEvent::CursorMoved { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::Touch(_)
                | WindowEvent::PinchGesture { .. }
                | WindowEvent::PanGesture { .. }
                | WindowEvent::DoubleTapGesture { .. }
                | WindowEvent::RotationGesture { .. }
        ) && let Some(trace) = &mut state.gpu.trace
        {
            trace.input();
        }
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
                log::debug!(
                    occluded,
                    visible:? = state.gpu.window.is_visible(),
                    minimized:? = state.gpu.window.is_minimized(),
                    focused = state.gpu.window.has_focus();
                    "window_occlusion"
                );
                Ok(())
            }
            WindowEvent::Focused(focused) => {
                state.fullscreen.focus_changed(focused);
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
        } else if self.state.as_ref().is_some_and(State::native_quit_approved) {
            self.complete_native_quit(event_loop);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let _native_unwind = termination::cancel_on_unwind();
        let mut wake = self.next_repaint;
        if wake.is_some_and(|when| when <= Instant::now()) {
            if let Some(state) = &self.state {
                state.gpu.window.request_redraw();
            }
            self.next_repaint = None;
            wake = None;
        }
        if let Some(state) = &self.state {
            let pending = state.app.read(App::submission_gate).pending();
            if (pending || state.gpu.trace.as_ref().is_some_and(|t| t.pending()))
                && let Err(error) = state.gpu.device.poll(wgpu::PollType::Poll)
            {
                self.fail(event_loop, error.into());
                return;
            }
            if pending {
                let poll = Instant::now() + Duration::from_millis(8);
                wake = Some(wake.map_or(poll, |repaint| repaint.min(poll)));
            }
            if let Some(trace) = &state.gpu.trace {
                trace.flush();
                if let Some(poll) = trace.poll_deadline() {
                    wake = Some(wake.map_or(poll, |repaint| repaint.min(poll)));
                }
            }
        }
        event_loop.set_control_flow(wake.map_or(ControlFlow::Wait, ControlFlow::WaitUntil));
    }
}

struct State<'scope, A> {
    gpu: Gpu,
    app: UiContext<'scope, A>,
    input: egui_winit::State,
    viewport: egui::ViewportInfo,
    fullscreen: PhotoFullscreen,
    menus: Option<menu::native::NativeMenu>,
    termination: Option<termination::Guard>,
}

impl<A: App> State<'_, A> {
    fn native_quit_approved(&self) -> bool {
        self.termination
            .as_ref()
            .is_some_and(|guard| guard.awaiting_decision())
            && self.app.read(App::quit_state) == Some(QuitState::Approved)
    }

    fn cleanup(&mut self) -> Result<()> {
        let restoration = self.fullscreen.update(&self.gpu.window, false);
        self.app.with_mut(App::on_exit);
        restoration?;
        self.gpu.drain_trace()
    }

    fn reveal(&self) {
        // Menus and Dock Quit remain usable with the sole window minimized;
        // decisions and application panels must not open invisibly.
        if self.gpu.window.is_minimized() == Some(true) {
            self.gpu.window.set_minimized(false);
        }
        self.gpu.window.focus_window();
        self.gpu.window.request_redraw();
    }

    fn draw(&mut self, context: &egui::Context, event_loop: &ActiveEventLoop) -> Result<()> {
        let mut timing = self.gpu.trace.as_mut().map(Trace::begin);
        let size = self.gpu.window.inner_size();
        let frame = if size.width == 0 || size.height == 0 {
            None
        } else {
            if self.gpu.needs_configure
                || size.width != self.gpu.config.width
                || size.height != self.gpu.config.height
            {
                self.gpu.configure(self.app.read(App::submission_gate))?;
            }
            if let Some(timing) = &mut timing {
                timing.acquire_start = Some(Instant::now());
            }
            let acquired = self.gpu.surface.get_current_texture();
            if let Some(timing) = &mut timing {
                timing.acquire_end = Some(Instant::now());
                timing.acquire_status = match &acquired {
                    wgpu::CurrentSurfaceTexture::Success(_) => "success",
                    wgpu::CurrentSurfaceTexture::Suboptimal(_) => "suboptimal",
                    wgpu::CurrentSurfaceTexture::Outdated => "outdated",
                    wgpu::CurrentSurfaceTexture::Lost => "lost",
                    wgpu::CurrentSurfaceTexture::Timeout => "timeout",
                    wgpu::CurrentSurfaceTexture::Occluded => "occluded",
                    wgpu::CurrentSurfaceTexture::Validation => "validation",
                };
            }
            match acquired {
                wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
                wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    self.gpu.needs_configure = true;
                    Some(frame)
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    self.gpu.configure(self.app.read(App::submission_gate))?;
                    self.gpu.window.request_redraw();
                    None
                }
                wgpu::CurrentSurfaceTexture::Lost => {
                    self.gpu.surface = self.gpu.instance.create_surface(self.gpu.window.clone())?;
                    self.gpu.configure(self.app.read(App::submission_gate))?;
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
        let clipboard = self
            .app
            .read(App::menu_paste_requested)
            .then(|| self.input.clipboard_text())
            .flatten();
        let menu_input = self.app.with_mut(|app| app.menu_input(clipboard));
        let mut raw = self.input.take_egui_input(&self.gpu.window);
        raw.events.splice(0..0, menu_input);
        raw.viewports
            .insert(egui::ViewportId::ROOT, self.viewport.clone());
        if let Some(timing) = &mut timing {
            timing.ui_start = Some(Instant::now());
        }
        let mut output = context.run_ui(raw, |ui| self.app.with_mut(|app| app.ui(ui, &self.app)));
        if let (Some(menus), Some(state)) = (&mut self.menus, self.app.read(App::menu_state)) {
            menus.update(&state);
        }
        if let Some(timing) = &mut timing {
            timing.ui_end = Some(Instant::now());
        }
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
            if let Some(timing) = &mut timing {
                timing.encode_start = Some(Instant::now());
            }
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
            let commands = commands.into_iter().chain([encoder.finish()]);
            if let (Some(trace), Some(timing)) = (&mut self.gpu.trace, &mut timing) {
                trace.submitting(timing);
                self.app.read(App::submission_gate).submit_with_completion(
                    &self.gpu.queue,
                    commands,
                    trace.completion(timing),
                );
                timing.submit_end = Some(Instant::now());
            } else {
                self.app
                    .read(App::submission_gate)
                    .submit(&self.gpu.queue, commands);
            }
            self.gpu.window.pre_present_notify();
            self.gpu.queue.present(frame);
            if let Some(timing) = &mut timing {
                timing.present_end = Some(Instant::now());
            }
            self.gpu.presented += 1;
        }
        if let (Some(trace), Some(timing)) = (&mut self.gpu.trace, timing) {
            trace.finish(
                timing,
                self.app.read(App::processing_revision),
                &self.gpu.device,
            );
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
        if let Some(active) = self
            .fullscreen
            .update(&self.gpu.window, self.app.read(App::photo_fullscreen))?
        {
            self.app
                .with_mut(|app| app.photo_fullscreen_changed(active));
        }
        if let Some(termination) = &self.termination
            && termination.awaiting_decision()
        {
            match self.app.read(App::quit_state) {
                Some(QuitState::Pending | QuitState::Approved) => {}
                Some(QuitState::Running) | None => termination.reply(false),
            }
        }
        if self.viewport.close_requested() && !self.native_quit_approved() {
            if let Some(guard) = &self.termination {
                guard.reply(false);
            }
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
    trace: Option<Trace>,
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
            trace: Trace::from_env(),
        };
        // No application or background submitters exist during initial configuration.
        gpu.configure(SubmissionGate::default())?;
        log::info!(
            adapter:? = gpu.adapter.get_info(),
            format:? = gpu.config.format;
            "gpu_initialized"
        );

        Ok(gpu)
    }

    fn drain_trace(&self) -> Result<()> {
        if let Some(trace) = &self.trace {
            if trace.pending() {
                self.device.poll(wgpu::PollType::Wait {
                    submission_index: None,
                    timeout: Some(Duration::from_secs(10)),
                })?;
            }
            trace.summary();
        }
        Ok(())
    }

    fn configure(&mut self, submissions: SubmissionGate) -> Result<()> {
        let trace_start = self.trace.as_ref().map(|_| Instant::now());
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
        log::debug!(
            visible:? = self.window.is_visible(),
            minimized:? = self.window.is_minimized(),
            focused = self.window.has_focus();
            "window_configured"
        );

        submissions.configure(|| self.surface.configure(&self.device, &self.config));
        macos::configure_sdr(&self.surface)?;
        self.needs_configure = false;
        if let (Some(trace), Some(start)) = (&self.trace, trace_start) {
            trace.configured(start, self.config.width, self.config.height);
        }
        Ok(())
    }
}
