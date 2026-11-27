use crate::Error;
use crate::evaluation::Value;
use crate::frame_raster::FrameRaster;
use crate::geometry::TextureMapping;
use crate::looks::{LookId, LutChain};
use crate::photographic::parameters::ProcessingParameters;
#[cfg(test)]
use crate::photographic::test_inputs::AuthoredPhotograph;
use crate::scopes::BINS;
use crate::scopes::Basis;
use crate::scopes::PLANE_LEN;
use crate::scopes::Plot;
use crate::scopes::Settings;
use crate::viewer::GpuImage;
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
    rendered_source: Option<(crate::photographic::graph::view_source::SourceRequest, bool)>,
    image: Weak<GpuImage>,
    calculation: Option<crate::evaluation::value::WeakValue>,
    look: Option<LookId>,
    settings: Settings,
    frame: FrameRaster,
    mapping: TextureMapping,
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
        (root, luts): (&Value, &LutChain),
        frame: FrameRaster,
        settings: Settings,
    ) -> Self {
        let canonical = image
            .prepared_definition()
            .filter(|old| old.same_definition(root))
            .unwrap_or_else(|| root.clone());
        Self {
            #[cfg(feature = "editor")]
            rendered_source: None,
            image: Arc::downgrade(image),
            calculation: Some(canonical.downgrade()),
            look: luts.id(),
            settings: accumulation_settings(settings),
            frame,
            mapping: frame.composition.texture_mapping(
                frame.min,
                frame.max,
                frame.composition.area(),
            ),
        }
    }

    #[cfg(feature = "editor")]
    fn rendered(
        view: &crate::viewer::view::RenderedView,
        frame: FrameRaster,
        mapping: TextureMapping,
        settings: Settings,
    ) -> Self {
        Self {
            rendered_source: Some((view.spec.clone(), view.capture.is_inspection())),
            image: Arc::downgrade(&view.image),
            calculation: None,
            look: None,
            settings: accumulation_settings(settings),
            frame,
            mapping,
        }
    }

    fn matches(&self, request: &Self) -> bool {
        self.can_hold(request)
            && self.image.ptr_eq(&request.image)
            && self.frame == request.frame
            && self.mapping == request.mapping
            && self.look == request.look
            && match (&self.calculation, &request.calculation) {
                (None, None) => true,
                (Some(a), Some(b)) => a
                    .upgrade()
                    .zip(b.upgrade())
                    .is_some_and(|(a, b)| a.same_definition(&b)),
                _ => false,
            }
    }

    fn can_hold(&self, request: &Self) -> bool {
        let same_source = self.image.ptr_eq(&request.image);
        #[cfg(feature = "editor")]
        let same_source = match (&self.rendered_source, &request.rendered_source) {
            (Some((a, a_inspection)), Some((b, b_inspection))) => {
                a_inspection == b_inspection && a.same_source(b)
            }
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
    result: Option<Result<(), String>>,
}

#[derive(Default)]
struct ReductionState {
    prepared: Option<Prepared>,
    // The command's completion callback owns the flight, even when the plot is hidden.
    pending: Weak<()>,
}

impl ReductionState {
    fn finish(&mut self, flight: &Arc<()>, result: Result<(), String>) {
        if !self.pending.ptr_eq(&Arc::downgrade(flight)) {
            return;
        }
        self.pending = Weak::new();
        if let Some(prepared) = &mut self.prepared {
            if result.is_err() {
                prepared.group = None;
            }
            prepared.result = Some(result);
        }
    }
}

pub struct GpuScopes {
    reduce: wgpu::ComputePipeline,
    display: wgpu::RenderPipeline,
    pub bins: wgpu::Buffer,
    pub totals: wgpu::Buffer,
    state: Arc<Mutex<ReductionState>>,
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
            size: u64::from(BINS + 1) * 4,
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
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_scopes/reduce.wgsl").into()),
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
            source: wgpu::ShaderSource::Wgsl(include_str!("gpu_scopes/display.wgsl").into()),
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
            state: Arc::default(),
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
    ) -> Option<Result<(), String>> {
        let root = image
            .inspection(parameters, None)
            .and_then(|prefix| image.output(prefix, &Default::default()))
            .ok()?;
        self.status_calculation(image, (&root, &Default::default()), frame, settings)
    }

    #[cfg(test)]
    pub(crate) fn status_processing(
        &self,
        image: &Arc<GpuImage>,
        input: &AuthoredPhotograph,
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<(), String>> {
        let root = image.legacy_calculation(input).ok()?;
        self.status_calculation(image, (&root, input.luts()), frame, settings)
    }

    pub fn status_calculation(
        &self,
        image: &Arc<GpuImage>,
        calculation: (&Value, &LutChain),
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<(), String>> {
        self.status_for(&Key::new(image, calculation, frame, settings))
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn status_rendered(
        &self,
        view: &crate::viewer::view::RenderedView,
        frame: FrameRaster,
        settings: Settings,
    ) -> Option<Result<(), String>> {
        self.status_mapped_rendered(view, frame, TextureMapping::IDENTITY, settings)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn status_mapped_rendered(
        &self,
        view: &crate::viewer::view::RenderedView,
        frame: FrameRaster,
        mapping: TextureMapping,
        settings: Settings,
    ) -> Option<Result<(), String>> {
        self.status_for(&Key::rendered(view, frame, mapping, settings))
    }

    fn status_for(&self, key: &Key) -> Option<Result<(), String>> {
        self.state
            .lock()
            .unwrap()
            .prepared
            .as_ref()
            .filter(|prepared| prepared.key.matches(key))?
            .result
            .clone()
    }

    pub fn invalidate(&self) {
        self.state.lock().unwrap().prepared = None;
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>, group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.display);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(all(test, feature = "editor"))]
pub(crate) mod testing;

pub struct ScopePaint {
    pub gpu: Arc<GpuScopes>,
    image: Arc<GpuImage>,
    calculation: Option<Value>,
    luts: LutChain,
    #[cfg(feature = "editor")]
    rendered: Option<(
        Arc<crate::viewer::view::RenderedView>,
        crate::gpu::submission::Queue,
    )>,
    pub frame: FrameRaster,
    mapping: TextureMapping,
    pub settings: Settings,
    pub context: egui::Context,
    group: Mutex<Option<wgpu::BindGroup>>,
}

impl ScopePaint {
    #[cfg(feature = "editor")]
    pub(crate) fn rendered(
        gpu: Arc<GpuScopes>,
        view: Arc<crate::viewer::view::RenderedView>,
        frame: FrameRaster,
        settings: Settings,
        queue: crate::gpu::submission::Queue,
        context: egui::Context,
    ) -> Self {
        Self {
            gpu,
            image: view.image.clone(),
            calculation: None,
            luts: Default::default(),
            rendered: Some((view, queue)),
            frame,
            mapping: TextureMapping::IDENTITY,
            settings,
            context,
            group: Mutex::new(None),
        }
    }

    #[cfg(feature = "editor")]
    pub(crate) fn mapped_rendered(
        gpu: Arc<GpuScopes>,
        view: Arc<crate::viewer::view::RenderedView>,
        frame: FrameRaster,
        mapping: TextureMapping,
        settings: Settings,
        queue: crate::gpu::submission::Queue,
        context: egui::Context,
    ) -> Self {
        let mut paint = Self::rendered(gpu, view, frame, settings, queue, context);
        paint.mapping = mapping;
        paint
    }

    pub fn new(
        gpu: Arc<GpuScopes>,
        image: Arc<GpuImage>,
        parameters: ProcessingParameters,
        frame: FrameRaster,
        settings: Settings,
        context: egui::Context,
    ) -> Self {
        let calculation = image
            .inspection(parameters, None)
            .and_then(|prefix| image.output(prefix, &Default::default()))
            .expect("admitted source-view transform");
        Self::with_calculation(
            gpu,
            image,
            (calculation, Default::default()),
            frame,
            settings,
            context,
        )
        .expect("admitted source-view storage")
    }

    pub fn with_calculation(
        gpu: Arc<GpuScopes>,
        image: Arc<GpuImage>,
        (calculation, luts): (Value, LutChain),
        frame: FrameRaster,
        settings: Settings,
        context: egui::Context,
    ) -> Result<Self, Error> {
        image.require_calculation(&calculation, &luts)?;
        Ok(Self {
            gpu,
            image,
            calculation: Some(calculation),
            luts,
            #[cfg(feature = "editor")]
            rendered: None,
            frame,
            mapping: TextureMapping::IDENTITY,
            settings,
            context,
            group: Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_processing(
        gpu: Arc<GpuScopes>,
        image: Arc<GpuImage>,
        processing: AuthoredPhotograph,
        frame: FrameRaster,
        settings: Settings,
        context: egui::Context,
    ) -> Result<Self, Error> {
        let calculation = image.legacy_calculation(&processing)?;
        Self::with_calculation(
            gpu,
            image,
            (calculation, processing.luts().clone()),
            frame,
            settings,
            context,
        )
    }
}

impl ScopePaint {
    #[doc(hidden)]
    pub fn prepare_frame(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        let key;
        #[cfg(feature = "editor")]
        let rendered = self.rendered.as_ref();
        #[cfg(not(feature = "editor"))]
        let rendered: Option<()> = None;
        #[cfg(feature = "editor")]
        if let Some((view, queue)) = rendered {
            view.ensure_submitted(device, queue, &self.context);
            key = Key::rendered(view, self.frame, self.mapping, self.settings);
        } else {
            let root = self
                .calculation
                .as_ref()
                .expect("source-backed scope calculation");
            self.image
                .prepare_calculation(device, encoder, root, &self.luts)
                .expect("validated rendering resources");
            key = Key::new(&self.image, (root, &self.luts), self.frame, self.settings);
        }
        #[cfg(not(feature = "editor"))]
        {
            let root = self
                .calculation
                .as_ref()
                .expect("source-backed scope calculation");
            self.image
                .prepare_calculation(device, encoder, root, &self.luts)
                .expect("validated rendering resources");
            key = Key::new(&self.image, (root, &self.luts), self.frame, self.settings);
        }
        let mut state = self.gpu.state.lock().unwrap();
        let pending = state.pending.upgrade().is_some();
        if !pending && state.prepared.as_ref().is_some_and(|p| p.result.is_none()) {
            // An abandoned encoder cannot leave its bins available for reuse.
            state.prepared = None;
        }
        let matches = state
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared.key.matches(&key));
        // Completion requests the latest UI demand without queueing intermediate reductions.
        if !matches && !pending {
            let encoded = if rendered.is_some() {
                self.gpu.encode_mapped(
                    device,
                    encoder,
                    &self.image.linear,
                    self.frame.output,
                    self.mapping,
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
            let (group, result) = match encoded {
                Ok(group) => {
                    let flight = Arc::new(());
                    state.pending = Arc::downgrade(&flight);
                    let state = Arc::downgrade(&self.gpu.state);
                    let context = self.context.clone();
                    encoder.on_submitted_work_done(move || {
                        if let Some(state) = state.upgrade() {
                            state.lock().unwrap().finish(&flight, Ok(()));
                            context.request_repaint();
                        }
                    });
                    (Some(group), None)
                }
                Err(error) => {
                    self.context.request_repaint();
                    (None, Some(Err(error.to_string())))
                }
            };
            state.prepared = Some(Prepared {
                key: key.clone(),
                plot: self.settings.plot,
                group,
                result,
            });
        }
        *self.group.lock().unwrap() = state.prepared.as_mut().and_then(|prepared| {
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
        let flight = state.pending.upgrade();
        drop(state);
        if let Some(flight) = flight {
            if let Err(error) = device.poll(wgpu::PollType::Poll) {
                self.gpu
                    .state
                    .lock()
                    .unwrap()
                    .finish(&flight, Err(format!("Scope GPU polling failed: {error}")));
                self.context.request_repaint();
            } else {
                self.context
                    .request_repaint_after(std::time::Duration::from_millis(16));
            }
        }
    }
}

impl egui_wgpu::CallbackTrait for ScopePaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        self.prepare_frame(device, encoder);
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
