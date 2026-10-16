use crate::{
    Error,
    geometry::TextureMapping,
    gpu::{
        GpuFlatField, ProcessingInput, ProcessingParameters, ProcessingSignature, ProcessingSource,
        SourceGeometry,
    },
    renderer::GpuRenderer,
    rendering::OutputTarget,
};
use glam::{DMat3, Vec4};
use std::sync::Arc;
use wgpu::util::DeviceExt;
#[path = "viewer_proxy.rs"]
mod proxy;
pub(crate) use proxy::GpuProxy;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Presentation {
    columns: [Vec4; 3],
    size: [u32; 4],
    uv_x: [f32; 4],
    uv_y: [f32; 4],
}

enum ImageSource {
    Owned(Box<ProcessingSource>),
    Shared(Arc<ProcessingSource>),
}

impl std::ops::Deref for ImageSource {
    type Target = ProcessingSource;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(source) => source,
            Self::Shared(source) => source,
        }
    }
}

impl ImageSource {
    fn mutable(&mut self) -> Result<&mut ProcessingSource, Error> {
        match self {
            Self::Owned(source) => Ok(source),
            Self::Shared(_) => Err(Error::InvalidParameter("shared proxy source is immutable")),
        }
    }

    fn write(&self, queue: &wgpu::Queue, pixels: &[[u16; 4]]) -> Result<(), Error> {
        match self {
            Self::Owned(source) => source.write(queue, pixels),
            Self::Shared(_) => Err(Error::InvalidParameter("shared proxy source is immutable")),
        }
    }
}

struct ImageProcessing {
    renderer: Arc<GpuRenderer>,
    target: OutputTarget,
    source: ImageSource,
    working: wgpu::Buffer,
    aces_group: wgpu::BindGroup,
}

pub struct GpuImage {
    last_parameters: std::sync::Mutex<Option<ProcessingSignature>>,
    source_revision: std::sync::atomic::AtomicU64,
    processing: Option<ImageProcessing>,
    pub linear: wgpu::Texture,
    display: wgpu::RenderPipeline,
    display_group: wgpu::BindGroup,
    presentation: Presentation,
    visible_group: std::sync::Mutex<Option<(TextureMapping, wgpu::BindGroup)>>,
}

impl GpuImage {
    pub async fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
        pixels: &[[u16; 4]],
        camera_to_ap1: DMat3,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        let renderer = GpuRenderer::new(device, queue).await?;
        Self::new_shared(
            &renderer,
            device,
            width,
            height,
            pixels,
            camera_to_ap1,
            target,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn new_shared(
        renderer: &Arc<GpuRenderer>,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        pixels: &[[u16; 4]],
        camera_to_ap1: DMat3,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        if width.checked_mul(height).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::Gpu(
                "source dimensions do not match the GPU image".into(),
            ));
        }
        let source = ProcessingSource::new(device, pixels, camera_to_ap1)?;
        Self::create(
            device,
            width,
            height,
            Some((renderer.clone(), ImageSource::Owned(Box::new(source)))),
            renderer.display.clone(),
            target,
        )
        .await
    }

    pub async fn new_rendered(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        width: u32,
        height: u32,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        Self::create(
            device,
            width,
            height,
            None,
            crate::renderer::display_pipeline(device),
            target,
        )
        .await
    }

    pub async fn new_rendered_shared(
        renderer: &GpuRenderer,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        Self::create(
            device,
            width,
            height,
            None,
            renderer.display.clone(),
            target,
        )
        .await
    }

    async fn create(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        source: Option<(Arc<GpuRenderer>, ImageSource)>,
        display: wgpu::RenderPipeline,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        let count = width
            .checked_mul(height)
            .filter(|n| *n > 0)
            .ok_or(Error::InvalidParameter("invalid GPU image dimensions"))?;
        if width > device.limits().max_texture_dimension_2d
            || height > device.limits().max_texture_dimension_2d
        {
            return Err(Error::Gpu("preview image exceeds GPU limits".into()));
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let processing = source.map(|(renderer, source)| {
            let working = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("processed AP1 preview"),
                size: u64::from(count) * 16,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            });
            let size = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("image dimensions"),
                contents: bytemuck::cast_slice(&[width, height, 0, 0]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let resources = renderer.target(target);
            let aces_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ACES image resources"),
                layout: &resources.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: working.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&resources.tables[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&resources.tables[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&resources.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: size.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: resources.hues.as_entire_binding(),
                    },
                ],
            });
            ImageProcessing {
                renderer,
                target,
                source,
                working,
                aces_group,
            }
        });
        let linear = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("linear target RGB, before presentation"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let matrix = target.linear_to_srgb();
        let presentation = Presentation {
            columns: [matrix.x_axis, matrix.y_axis, matrix.z_axis].map(|v| v.as_vec3().extend(0.)),
            size: [width, height, 0, 0],
            uv_x: [0., 0., 1., 0.],
            uv_y: [0., 1., 0., 0.],
        };
        let display_group = make_presentation_group(device, &display, &linear, presentation);
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            last_parameters: std::sync::Mutex::new(None),
            source_revision: std::sync::atomic::AtomicU64::new(0),
            processing,
            linear,
            display,
            display_group,
            presentation,
            visible_group: std::sync::Mutex::new(None),
        })
    }

    pub fn processing_byte_len(&self) -> (u64, u64) {
        self.processing
            .as_ref()
            .map_or((0, 0), |p| (p.source.byte_len(), p.working.size()))
    }

    pub fn set_geometry(&mut self, geometry: SourceGeometry) -> Result<(), Error> {
        let processing = self
            .processing
            .as_mut()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        processing.source.mutable()?.set_geometry(geometry)?;
        self.source_revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.last_parameters.lock().unwrap() = None;
        Ok(())
    }

    pub fn set_flat_field(
        &mut self,
        field: Option<Arc<GpuFlatField>>,
        geometry: SourceGeometry,
    ) -> Result<(), Error> {
        let processing = self
            .processing
            .as_mut()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        processing
            .source
            .mutable()?
            .set_flat_field(field, geometry)?;
        self.source_revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.last_parameters.lock().unwrap() = None;
        Ok(())
    }

    pub fn write_source(&self, queue: &wgpu::Queue, pixels: &[[u16; 4]]) -> Result<(), Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        processing.source.write(queue, pixels)?;
        self.source_revision
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.last_parameters.lock().unwrap() = None;
        Ok(())
    }

    pub fn source_revision(&self) -> u64 {
        self.source_revision
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Shared by viewer and scopes so either callback can prepare the whole proxy first.
    pub fn prepare(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        parameters: ProcessingParameters,
    ) -> bool {
        self.prepare_processing(device, encoder, &parameters.into())
            .expect("processable image")
    }

    pub fn require_processing(&self, input: &ProcessingInput) -> Result<(), Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        input.require_source(&processing.source)
    }

    /// Prepare an immutable global/local input; invalid inputs leave the prior preparation intact.
    pub fn prepare_processing(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &ProcessingInput,
    ) -> Result<bool, Error> {
        self.require_processing(input)?;
        let signature = input.signature();
        let mut previous = self.last_parameters.lock().unwrap();
        if previous.as_ref() == Some(&signature) {
            return Ok(false);
        }
        self.encode_processing_inner(device, encoder, input)?;
        *previous = Some(signature);
        Ok(true)
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        parameters: ProcessingParameters,
    ) {
        self.encode_processing(device, encoder, &parameters.into())
            .expect("processable image");
    }

    pub fn encode_processing(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &ProcessingInput,
    ) -> Result<(), Error> {
        self.require_processing(input)?;
        let mut previous = self.last_parameters.lock().unwrap();
        self.encode_processing_inner(device, encoder, input)?;
        // An explicit dispatch must not leave a cached preparation describing another output.
        *previous = None;
        Ok(())
    }

    fn encode_processing_inner(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &ProcessingInput,
    ) -> Result<(), Error> {
        let processing = self
            .processing
            .as_ref()
            .expect("validated processing source");
        match input.grade() {
            Some(grade) => processing.renderer.kernel.encode_grade(
                device,
                encoder,
                &processing.source,
                &processing.working,
                grade,
            )?,
            None => processing.renderer.kernel.encode(
                device,
                encoder,
                &processing.source,
                &processing.working,
                input.parameters(),
            ),
        }
        #[cfg(feature = "editor")]
        if !input.repairs.0.is_empty() {
            let geometry = processing.source.geometry().ok_or_else(|| {
                Error::Gpu("repair rendering needs original source geometry".into())
            })?;
            if geometry.region().width == geometry.size()[0]
                && geometry.region().height == geometry.size()[1]
            {
                processing.renderer.repairs.original(
                    device,
                    encoder,
                    &processing.renderer.kernel,
                    &processing.working,
                    input,
                    geometry,
                );
            }
        }
        let view = self.linear.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ACES target rendering"),
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
        pass.set_pipeline(&processing.renderer.target(processing.target).pipeline);
        pass.set_bind_group(0, &processing.aces_group, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }

    pub fn presentation_group(
        &self,
        device: &wgpu::Device,
        uv: impl Into<TextureMapping>,
    ) -> wgpu::BindGroup {
        let uv = uv.into();
        let mut cached = self.visible_group.lock().unwrap();
        if let Some((previous, group)) = &*cached
            && *previous == uv
        {
            return group.clone();
        }
        let group = make_presentation_group(
            device,
            &self.display,
            &self.linear,
            Presentation {
                uv_x: [uv.origin[0], uv.origin[1], uv.x_axis[0], uv.x_axis[1]],
                uv_y: [uv.y_axis[0], uv.y_axis[1], 0., 0.],
                ..self.presentation
            },
        );
        *cached = Some((uv, group.clone()));
        group
    }

    pub fn paint_group(&self, pass: &mut wgpu::RenderPass<'static>, group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.display);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>) {
        self.paint_group(pass, &self.display_group);
    }
}

fn make_presentation_group(
    device: &wgpu::Device,
    display: &wgpu::RenderPipeline,
    linear: &wgpu::Texture,
    parameters: Presentation,
) -> wgpu::BindGroup {
    let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("presentation basis, dimensions and mapping"),
        contents: bytemuck::bytes_of(&parameters),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let view = linear.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("managed image presentation"),
        layout: &display.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

pub struct ImagePaint {
    #[cfg(feature = "editor")]
    rendered: Option<(
        Arc<crate::gpu::view::RenderedView>,
        crate::gpu::SubmissionGate,
        egui::Context,
    )>,
    image: Arc<GpuImage>,
    processing: Option<ProcessingInput>,
    uv: TextureMapping,
    group: std::sync::Mutex<Option<wgpu::BindGroup>>,
}

impl ImagePaint {
    #[cfg(feature = "editor")]
    pub(crate) fn rendered(
        view: Arc<crate::gpu::view::RenderedView>,
        uv: TextureMapping,
        submissions: crate::gpu::SubmissionGate,
        context: egui::Context,
    ) -> Self {
        Self {
            image: view.image.clone(),
            processing: None,
            uv,
            group: std::sync::Mutex::new(None),
            rendered: Some((view, submissions, context)),
        }
    }

    pub fn new(
        image: Arc<GpuImage>,
        parameters: Option<ProcessingParameters>,
        uv: impl Into<TextureMapping>,
    ) -> Self {
        Self {
            #[cfg(feature = "editor")]
            rendered: None,
            image,
            processing: parameters.map(Into::into),
            uv: uv.into(),
            group: std::sync::Mutex::new(None),
        }
    }

    pub fn with_processing(
        image: Arc<GpuImage>,
        processing: Option<ProcessingInput>,
        uv: impl Into<TextureMapping>,
    ) -> Result<Self, Error> {
        if let Some(input) = &processing {
            image.require_processing(input)?;
        }
        Ok(Self {
            #[cfg(feature = "editor")]
            rendered: None,
            image,
            processing,
            uv: uv.into(),
            group: std::sync::Mutex::new(None),
        })
    }
}

impl egui_wgpu::CallbackTrait for ImagePaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        _resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        #[cfg(feature = "editor")]
        if let Some((view, submissions, context)) = &self.rendered {
            view.ensure_submitted(device, _queue, submissions, context);
        }
        if let Some(input) = &self.processing {
            self.image
                .prepare_processing(device, encoder, input)
                .expect("validated processing source");
        }
        if self.uv != TextureMapping::IDENTITY {
            *self.group.lock().unwrap() = Some(self.image.presentation_group(device, self.uv));
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        match &*self.group.lock().unwrap() {
            Some(group) => self.image.paint_group(pass, group),
            None => self.image.paint(pass),
        }
    }
}
