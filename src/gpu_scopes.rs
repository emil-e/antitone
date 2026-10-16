use crate::{
    Error,
    frame_raster::FrameRaster,
    geometry::TextureMapping,
    gpu::{ProcessingInput, ProcessingParameters, ProcessingSignature},
    scopes::{BINS, Basis, Counts, PLANE_LEN, Plot, Settings},
    viewer::GpuImage,
};
use std::sync::{Arc, Mutex, Weak};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Parameters {
    size: [u32; 4],
    luma_gain: [f32; 4],
    direction_extent: [f32; 4],
    uv_x: [f32; 4],
    uv_y: [f32; 4],
}

#[derive(Clone)]
struct Key {
    #[cfg(feature = "editor")]
    rendered_source: Option<crate::gpu::view::ViewSourceSpec>,
    image: Weak<GpuImage>,
    source_revision: u64,
    processing: ProcessingSignature,
    settings: Settings,
    frame: FrameRaster,
}

fn accumulation_settings(mut settings: Settings) -> Settings {
    if settings.plot == Plot::Luma {
        settings.plot = Plot::Rgb;
    }
    settings
}

impl Key {
    fn new(
        image: &Arc<GpuImage>,
        input: &ProcessingInput,
        frame: FrameRaster,
        settings: Settings,
    ) -> Self {
        Self {
            #[cfg(feature = "editor")]
            rendered_source: None,
            image: Arc::downgrade(image),
            source_revision: image.source_revision(),
            processing: input.signature(),
            settings: accumulation_settings(settings),
            frame,
        }
    }

    #[cfg(feature = "editor")]
    fn rendered(
        view: &crate::gpu::view::RenderedView,
        frame: FrameRaster,
        settings: Settings,
    ) -> Self {
        Self {
            rendered_source: Some(view.spec.clone()),
            image: Arc::downgrade(&view.image),
            source_revision: 0,
            processing: view.processing.clone(),
            settings: accumulation_settings(settings),
            frame,
        }
    }

    fn matches(&self, request: &Self) -> bool {
        self.can_hold(request)
            && self.image.ptr_eq(&request.image)
            && self.source_revision == request.source_revision
            && self.frame == request.frame
            && self.processing == request.processing
    }

    fn can_hold(&self, request: &Self) -> bool {
        let same_source =
            self.image.ptr_eq(&request.image) && self.source_revision == request.source_revision;
        #[cfg(feature = "editor")]
        let same_source = match (&self.rendered_source, &request.rendered_source) {
            (Some(a), Some(b)) => a.same_source(b),
            (None, None) => same_source,
            _ => false,
        };
        same_source
            && self.settings == request.settings
            && self.frame.composition == request.frame.composition
    }
}

struct Prepared {
    key: Key,
    plot: Plot,
    group: Option<wgpu::BindGroup>,
}

#[derive(Default)]
struct Diagnostic {
    revision: u64,
    pending: bool,
    result: Option<Result<Counts, String>>,
}

pub struct GpuScopes {
    reduce: wgpu::ComputePipeline,
    display: wgpu::RenderPipeline,
    pub bins: wgpu::Buffer,
    pub totals: wgpu::Buffer,
    prepared: Mutex<Option<Prepared>>,
    diagnostic: Arc<Mutex<Diagnostic>>,
    reductions: std::sync::atomic::AtomicU64,
}

impl GpuScopes {
    pub async fn new(device: &wgpu::Device) -> Result<Self, Error> {
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let bins = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope density bins"),
            size: (PLANE_LEN * 4 * 4) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let totals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scope sample totals"),
            size: u64::from(BINS + 4) * 4,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let buffer = |binding, visibility, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            count: None,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        };
        let reduce_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scope reduction layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    count: None,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
                buffer(
                    1,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Uniform,
                ),
                buffer(
                    2,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ),
                buffer(
                    3,
                    wgpu::ShaderStages::COMPUTE,
                    wgpu::BufferBindingType::Storage { read_only: false },
                ),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("visible-frame scope reduction"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scopes.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&reduce_layout)],
            immediate_size: 0,
        });
        let reduce = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("visible-frame scope reduction"),
            layout: Some(&layout),
            module: &module,
            entry_point: Some("reduce"),
            compilation_options: Default::default(),
            cache: None,
        });
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scope display layout"),
            entries: &[
                buffer(
                    0,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer(
                    1,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Storage { read_only: true },
                ),
                buffer(
                    2,
                    wgpu::ShaderStages::FRAGMENT,
                    wgpu::BufferBindingType::Uniform,
                ),
            ],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scope density display"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/scope_display.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&draw_layout)],
            immediate_size: 0,
        });
        let display = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scope density display"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba16Float,
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
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            reduce,
            display,
            bins,
            totals,
            prepared: Mutex::new(None),
            diagnostic: Arc::default(),
            reductions: Default::default(),
        })
    }

    /// Input is the complete final-frame linear-target raster, never a regional settled image.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        frame: &wgpu::Texture,
        settings: Settings,
    ) -> Result<wgpu::BindGroup, Error> {
        self.encode_mapped(
            device,
            encoder,
            frame,
            [frame.width(), frame.height()],
            TextureMapping::IDENTITY,
            settings,
        )
    }

    /// Samples the requested composed frame from the rendered whole-active-area proxy.
    pub fn encode_frame(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        frame: FrameRaster,
        settings: Settings,
    ) -> Result<wgpu::BindGroup, Error> {
        let mapping =
            frame
                .composition
                .texture_mapping(frame.min, frame.max, frame.composition.area());
        self.encode_mapped(device, encoder, source, frame.output, mapping, settings)
    }

    fn encode_mapped(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        size: [u32; 2],
        mapping: TextureMapping,
        settings: Settings,
    ) -> Result<wgpu::BindGroup, Error> {
        settings.validate(size)?;
        if source.format() != wgpu::TextureFormat::Rgba32Float {
            return Err(Error::InvalidParameter(
                "scopes require a linear-target float raster",
            ));
        }
        let uniform = Self::uniform(device, size, mapping, settings);
        let view = source.create_view(&Default::default());
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope reduction resources"),
            layout: &self.reduce.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.bins.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: self.totals.as_entire_binding(),
                },
            ],
        });
        encoder.clear_buffer(&self.bins, 0, None);
        encoder.clear_buffer(&self.totals, 0, None);
        {
            let mut pass = encoder.begin_compute_pass(&Default::default());
            pass.set_pipeline(&self.reduce);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(size[0].div_ceil(16), size[1].div_ceil(16), 1);
        }
        self.reductions
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(self.draw_group(device, &uniform))
    }

    fn uniform(
        device: &wgpu::Device,
        size: [u32; 2],
        mapping: TextureMapping,
        settings: Settings,
    ) -> wgpu::Buffer {
        let basis = Basis::new(settings.target);
        let parameters = Parameters {
            size: [
                size[0],
                size[1],
                match settings.plot {
                    Plot::Rgb => 0,
                    Plot::Luma => 1,
                    Plot::Vector => 2,
                },
                0,
            ],
            luma_gain: [
                basis.luma.x as f32,
                basis.luma.y as f32,
                basis.luma.z as f32,
                settings.amplification as f32,
            ],
            direction_extent: [
                basis.blue_direction.x as f32,
                basis.blue_direction.y as f32,
                (basis.extent / settings.amplification as f64) as f32,
                0.,
            ],
            uv_x: [
                mapping.origin[0],
                mapping.origin[1],
                mapping.x_axis[0],
                mapping.x_axis[1],
            ],
            uv_y: [mapping.y_axis[0], mapping.y_axis[1], 0., 0.],
        };
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scope target and plot parameters"),
            contents: bytemuck::bytes_of(&parameters),
            usage: wgpu::BufferUsages::UNIFORM,
        })
    }

    fn draw_group(&self, device: &wgpu::Device, uniform: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scope display resources"),
            layout: &self.display.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bins.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.totals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
            ],
        })
    }

    pub fn reduction_count(&self) -> u64 {
        self.reductions.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn status(
        &self,
        image: &Arc<GpuImage>,
        parameters: ProcessingParameters,
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<Counts, String>> {
        self.status_processing(image, &parameters.into(), frame, settings)
    }

    pub fn status_processing(
        &self,
        image: &Arc<GpuImage>,
        input: &ProcessingInput,
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<Counts, String>> {
        if self
            .prepared
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|prepared| {
                prepared
                    .key
                    .matches(&Key::new(image, input, frame, settings))
            })
        {
            self.diagnostic.lock().unwrap().result.clone()
        } else {
            None
        }
    }

    #[cfg(feature = "editor")]
    pub(crate) fn status_rendered(
        &self,
        view: &crate::gpu::view::RenderedView,
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<Counts, String>> {
        if self
            .prepared
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|p| p.key.matches(&Key::rendered(view, frame, settings)))
        {
            self.diagnostic.lock().unwrap().result.clone()
        } else {
            None
        }
    }

    pub fn invalidate(&self) {
        if self.prepared.lock().unwrap().take().is_some() {
            let mut diagnostic = self.diagnostic.lock().unwrap();
            diagnostic.revision += 1;
            diagnostic.result = None;
        }
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>, group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.display);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn read_diagnostics(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        context: &egui::Context,
    ) {
        let revision = {
            let mut state = self.diagnostic.lock().unwrap();
            state.revision += 1;
            state.pending = true;
            state.result = None;
            state.revision
        };
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("16-byte scope diagnostic readback"),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&self.totals, 0, &staging, 0, 16);
        let buffer = staging.clone();
        let state = self.diagnostic.clone();
        let context = context.clone();
        encoder.map_buffer_on_submit(&staging, wgpu::MapMode::Read, .., move |result| {
            let result = result
                .map_err(|e| format!("Scope diagnostics failed: {e}"))
                .and_then(|()| {
                    let mapped = buffer.get_mapped_range(..).map_err(|e| e.to_string())?;
                    let values: &[u32] = bytemuck::cast_slice(&mapped);
                    let counts = Counts {
                        valid: values[0],
                        invalid: values[1],
                        outside: values[2],
                    };
                    drop(mapped);
                    buffer.unmap();
                    Ok(counts)
                });
            let mut state = state.lock().unwrap();
            state.pending = false;
            if state.revision == revision {
                state.result = Some(result);
            }
            context.request_repaint();
        });
    }
}

pub struct ScopePaint {
    pub gpu: Arc<GpuScopes>,
    image: Arc<GpuImage>,
    processing: Option<ProcessingInput>,
    #[cfg(feature = "editor")]
    rendered: Option<(
        Arc<crate::gpu::view::RenderedView>,
        crate::gpu::SubmissionGate,
    )>,
    pub frame: FrameRaster,
    pub settings: Settings,
    pub context: egui::Context,
    group: Mutex<Option<wgpu::BindGroup>>,
}

impl ScopePaint {
    #[cfg(feature = "editor")]
    pub(crate) fn rendered(
        gpu: Arc<GpuScopes>,
        view: Arc<crate::gpu::view::RenderedView>,
        frame: FrameRaster,
        settings: Settings,
        submissions: crate::gpu::SubmissionGate,
        context: egui::Context,
    ) -> Self {
        Self {
            gpu,
            image: view.image.clone(),
            processing: None,
            rendered: Some((view, submissions)),
            frame,
            settings,
            context,
            group: Mutex::new(None),
        }
    }

    pub fn new(
        gpu: Arc<GpuScopes>,
        image: Arc<GpuImage>,
        parameters: ProcessingParameters,
        frame: FrameRaster,
        settings: Settings,
        context: egui::Context,
    ) -> Self {
        Self {
            gpu,
            image,
            processing: Some(parameters.into()),
            #[cfg(feature = "editor")]
            rendered: None,
            frame,
            settings,
            context,
            group: Mutex::new(None),
        }
    }

    pub fn with_processing(
        gpu: Arc<GpuScopes>,
        image: Arc<GpuImage>,
        processing: ProcessingInput,
        frame: FrameRaster,
        settings: Settings,
        context: egui::Context,
    ) -> Result<Self, Error> {
        image.require_processing(&processing)?;
        Ok(Self {
            gpu,
            image,
            processing: Some(processing),
            #[cfg(feature = "editor")]
            rendered: None,
            frame,
            settings,
            context,
            group: Mutex::new(None),
        })
    }
}

impl egui_wgpu::CallbackTrait for ScopePaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let key;
        let coverage_ready;
        let proxy_mapping = self.frame.composition.texture_mapping(
            self.frame.min,
            self.frame.max,
            self.frame.composition.area(),
        );
        #[cfg(feature = "editor")]
        let rendered = self.rendered.as_ref();
        #[cfg(not(feature = "editor"))]
        let rendered: Option<()> = None;
        #[cfg(feature = "editor")]
        if let Some((view, submissions)) = rendered {
            view.ensure_submitted(device, queue, submissions, &self.context);
            key = Key::rendered(view, self.frame, self.settings);
            let mut accepted = view.spec.frame;
            accepted.output = self.frame.output;
            coverage_ready =
                accepted == self.frame && matches!(self.image.look_result(), Some(Ok(())));
        } else {
            let input = self.processing.as_ref().expect("source-backed scope input");
            self.image
                .prepare_rendering(
                    device,
                    queue,
                    encoder,
                    input,
                    proxy_mapping,
                    Some(self.context.clone()),
                )
                .expect("validated rendering resources");
            key = Key::new(&self.image, input, self.frame, self.settings);
            coverage_ready = matches!(self.image.look_result_for(proxy_mapping), Some(Ok(())));
        }
        #[cfg(not(feature = "editor"))]
        {
            let input = self.processing.as_ref().expect("source-backed scope input");
            self.image
                .prepare_rendering(
                    device,
                    queue,
                    encoder,
                    input,
                    proxy_mapping,
                    Some(self.context.clone()),
                )
                .expect("validated rendering resources");
            key = Key::new(&self.image, input, self.frame, self.settings);
            coverage_ready = matches!(self.image.look_result_for(proxy_mapping), Some(Ok(())));
        }
        let mut cached = self.gpu.prepared.lock().unwrap();
        let matches = cached
            .as_ref()
            .is_some_and(|prepared| prepared.key.matches(&key));
        // UI state supplies the newest request on the completion repaint. Don't enqueue another
        // reduction/readback while the previous one is outstanding, even after hide/source changes.
        let pending = self.gpu.diagnostic.lock().unwrap().pending;
        if !matches && !pending && coverage_ready {
            let encoded = if rendered.is_some() {
                self.gpu.encode_mapped(
                    device,
                    encoder,
                    &self.image.linear,
                    self.frame.output,
                    TextureMapping::IDENTITY,
                    self.settings,
                )
            } else {
                self.gpu.encode_frame(
                    device,
                    encoder,
                    &self.image.linear,
                    self.frame,
                    self.settings,
                )
            };
            match encoded {
                Ok(group) => {
                    *cached = Some(Prepared {
                        key: key.clone(),
                        plot: self.settings.plot,
                        group: Some(group),
                    });
                    self.gpu.read_diagnostics(device, encoder, &self.context);
                }
                Err(error) => {
                    *cached = Some(Prepared {
                        key: key.clone(),
                        plot: self.settings.plot,
                        group: None,
                    });
                    let mut diagnostic = self.gpu.diagnostic.lock().unwrap();
                    diagnostic.revision += 1;
                    diagnostic.result = Some(Err(error.to_string()));
                    self.context.request_repaint();
                }
            }
        }
        *self.group.lock().unwrap() = cached.as_mut().and_then(|prepared| {
            // Keep the previous complete trace during grade/navigation updates.
            // Its bins and normalization stay together; source/composition/graticule changes reject it.
            if !prepared.key.can_hold(&key) {
                return None;
            }
            if prepared.plot != self.settings.plot && prepared.group.is_some() {
                let uniform = GpuScopes::uniform(
                    device,
                    prepared.key.frame.output,
                    TextureMapping::IDENTITY,
                    self.settings,
                );
                prepared.group = Some(self.gpu.draw_group(device, &uniform));
                prepared.plot = self.settings.plot;
            }
            prepared.group.clone()
        });
        drop(cached);
        let pending = self.gpu.diagnostic.lock().unwrap().pending;
        if pending {
            if let Err(error) = device.poll(wgpu::PollType::Poll) {
                let mut diagnostic = self.gpu.diagnostic.lock().unwrap();
                diagnostic.revision += 1;
                diagnostic.pending = false;
                diagnostic.result = Some(Err(format!("Scope GPU polling failed: {error}")));
                self.context.request_repaint();
            } else {
                self.context
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(group) = &*self.group.lock().unwrap() {
            self.gpu.paint(pass, group);
        }
    }
}
