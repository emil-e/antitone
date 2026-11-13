use crate::Error;
use crate::geometry::TextureMapping;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::renderer::GpuRenderer;
use crate::rendering::OutputTarget;
use glam::{DMat3, Vec4};
use std::sync::Arc;
use wgpu::util::DeviceExt;
mod calculation;
mod proxy;
pub(crate) use proxy::GpuProxy;
#[cfg(test)]
mod film_tests;
#[cfg(test)]
mod look_tests;
#[cfg(test)]
pub(crate) mod readback;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Presentation {
    columns: [Vec4; 3],
    size: [u32; 4],
    uv_x: [f32; 4],
    uv_y: [f32; 4],
}

struct ImageProcessing {
    renderer: Arc<GpuRenderer>,
    target: OutputTarget,
    source: Arc<ProcessingSource>,
    working: wgpu::Buffer,
    aces_group: std::sync::Mutex<Option<crate::renderer::RenderBinding>>,
    camera: std::sync::Mutex<Option<crate::gpu::evaluation::Value>>,
    last_calculation: std::sync::Mutex<Option<crate::gpu::evaluation::Value>>,
}

pub struct GpuImage {
    device_limits: wgpu::Limits,
    look_check: std::sync::Mutex<Option<Arc<crate::gpu::lut3d::Check>>>,
    look_projections: std::sync::Mutex<Vec<(TextureMapping, Arc<crate::gpu::lut3d::Check>)>>,
    paint_mapping: std::sync::Mutex<Option<TextureMapping>>,
    processing: Option<ImageProcessing>,
    pub linear: wgpu::Texture,
    display: wgpu::RenderPipeline,
    display_group: wgpu::BindGroup,
    presentation: Presentation,
    visible_group: std::sync::Mutex<Option<(TextureMapping, wgpu::BindGroup)>>,
}

impl GpuImage {
    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn device_limits(&self) -> &wgpu::Limits {
        &self.device_limits
    }

    pub async fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [u32; 2],
        pixels: &[[u16; 4]],
        camera_to_ap1: DMat3,
        geometry: Option<SourceGeometry>,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        let renderer = GpuRenderer::new(device, queue).await?;
        Self::new_shared(
            &renderer,
            device,
            size,
            pixels,
            camera_to_ap1,
            geometry,
            target,
        )
        .await
    }
    pub async fn new_shared(
        renderer: &Arc<GpuRenderer>,
        device: &wgpu::Device,
        size: [u32; 2],
        pixels: &[[u16; 4]],
        camera_to_ap1: DMat3,
        geometry: Option<SourceGeometry>,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        if size[0].checked_mul(size[1]).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::Gpu(
                "source dimensions do not match the GPU image".into(),
            ));
        }
        let source = ProcessingSource::new(device, pixels, camera_to_ap1, geometry)?;
        Self::create(
            device,
            size[0],
            size[1],
            Some((renderer.clone(), Arc::new(source))),
            renderer.display.clone(),
            target,
        )
        .await
    }
    pub async fn new_continuous_shared(
        renderer: &Arc<GpuRenderer>,
        device: &wgpu::Device,
        size: [u32; 2],
        pixels: &[[f32; 4]],
        camera_to_ap1: DMat3,
        geometry: Option<SourceGeometry>,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        if size[0].checked_mul(size[1]).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::Gpu(
                "source dimensions do not match the GPU image".into(),
            ));
        }
        let source = ProcessingSource::new_continuous(device, pixels, camera_to_ap1, geometry)?;
        Self::create(
            device,
            size[0],
            size[1],
            Some((renderer.clone(), Arc::new(source))),
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
        source: Option<(Arc<GpuRenderer>, Arc<ProcessingSource>)>,
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
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let processing = source
            .map(|(renderer, source)| -> Result<_, Error> {
                let working = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("processed AP1 preview"),
                    size: u64::from(count) * 16,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                Ok(ImageProcessing {
                    renderer,
                    target,
                    source,
                    working,
                    aces_group: std::sync::Mutex::new(None),
                    camera: std::sync::Mutex::new(None),
                    last_calculation: std::sync::Mutex::new(None),
                })
            })
            .transpose()?;
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
            device_limits: device.limits(),
            look_check: std::sync::Mutex::new(None),
            look_projections: std::sync::Mutex::new(Vec::new()),
            paint_mapping: std::sync::Mutex::new(None),
            processing,
            linear,
            display,
            display_group,
            presentation,
            visible_group: std::sync::Mutex::new(None),
        })
    }

    pub fn processing_byte_len(&self) -> (u64, u64) {
        self.processing.as_ref().map_or((0, 0), |p| {
            (
                p.source.byte_len(),
                p.working.size()
                    + p.aces_group
                        .lock()
                        .unwrap()
                        .as_ref()
                        .map_or(0, |binding| binding.reference_bytes()),
            )
        })
    }

    #[cfg(feature = "editor")]
    pub(crate) fn look_resources(&self) -> Vec<Arc<crate::gpu::lut3d::Lut>> {
        self.processing
            .as_ref()
            .map_or_else(Vec::new, |processing| {
                processing
                    .aces_group
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map_or_else(Vec::new, |binding| binding.look_resources())
            })
    }

    /// Shared by viewer and scopes so either callback can prepare the whole proxy first.
    pub fn prepare(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        parameters: ProcessingParameters,
    ) -> bool {
        let root = self
            .inspection(parameters, None)
            .expect("processable source inspection");
        self.prepare_calculation(device, encoder, &root, &Default::default())
            .expect("processable image")
    }

    #[cfg(test)]
    pub(crate) fn require_processing(&self, input: &AuthoredPhotograph) -> Result<(), Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no source buffer".into()))?;
        for asset in input.luts().assets() {
            crate::gpu::lut3d::Lut::require_limits(&self.device_limits, asset.lut())?;
        }
        crate::gpu::execution::raster::resident_plan(&processing.source, input)?;
        Ok(())
    }

    /// Prepare an immutable adjustment-layer input; invalid inputs leave the prior preparation intact.
    #[cfg(test)]
    pub(crate) fn prepare_processing(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &AuthoredPhotograph,
    ) -> Result<bool, Error> {
        let root = self.legacy_calculation(input)?;
        self.prepare_calculation(device, encoder, &root, input.luts())
    }

    pub fn encode(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        parameters: ProcessingParameters,
    ) {
        let root = self
            .inspection(parameters, None)
            .expect("processable source inspection");
        self.encode_calculation(device, encoder, &root, &Default::default())
            .expect("processable image");
    }

    #[cfg(test)]
    pub(crate) fn encode_processing(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        input: &AuthoredPhotograph,
    ) -> Result<(), Error> {
        let root = self.legacy_calculation(input)?;
        self.encode_calculation(device, encoder, &root, input.luts())
    }

    pub(crate) fn set_look_check(&self, check: Option<Arc<crate::gpu::lut3d::Check>>) {
        *self.look_check.lock().unwrap() = check;
    }

    pub fn begin_look_check(&self, ready: impl FnOnce() + Send + 'static) {
        if let Some(check) = self.look_check.lock().unwrap().as_ref() {
            check.begin(ready);
        }
    }

    pub(crate) fn begin_look_check_tracked(
        &self,
        submissions: &crate::gpu::submission::SubmissionGate,
        ready: impl FnOnce() + Send + 'static,
    ) {
        let pending = submissions.track();
        self.begin_look_check(move || {
            ready();
            drop(pending);
        });
    }

    pub fn look_result(&self) -> Option<Result<(), Error>> {
        self.look_check
            .lock()
            .unwrap()
            .as_ref()
            .map_or(Some(Ok(())), |check| check.result())
    }

    pub(crate) fn look_result_for(&self, mapping: TextureMapping) -> Option<Result<(), Error>> {
        if self.processing.is_none() || self.look_check.lock().unwrap().is_none() {
            return self.look_result();
        }
        self.look_projections
            .lock()
            .unwrap()
            .iter()
            .find(|(region, _)| *region == mapping)
            .and_then(|(_, check)| check.result())
    }

    pub(crate) fn look_failure(&self, id: crate::looks::LookId) -> Option<Error> {
        if let Some(processing) = &self.processing
            && processing
                .aces_group
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|binding| binding.look_id())
                != Some(id)
        {
            return None;
        }
        if self.processing.is_none() {
            return self.look_result().and_then(Result::err);
        }
        self.look_result_for((*self.paint_mapping.lock().unwrap())?)
            .and_then(Result::err)
    }

    #[cfg(test)]
    pub(crate) fn prepare_rendering(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input: &AuthoredPhotograph,
        mapping: TextureMapping,
        context: Option<egui::Context>,
    ) -> Result<(), Error> {
        let root = self.legacy_calculation(input)?;
        self.prepare_rendering_calculation(
            device,
            queue,
            encoder,
            (&root, input.luts()),
            mapping,
            context,
        )
    }

    pub fn prepare_rendering_calculation(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        (root, luts): (&crate::gpu::evaluation::Value, &crate::looks::LutChain),
        mapping: TextureMapping,
        context: Option<egui::Context>,
    ) -> Result<(), Error> {
        if luts.is_empty() {
            self.prepare_calculation(device, encoder, root, luts)?;
            return Ok(());
        }
        self.prepare_looked(device, queue, (root, luts), mapping, context)
    }

    fn prepare_looked(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        (root, luts): (&crate::gpu::evaluation::Value, &crate::looks::LutChain),
        mapping: TextureMapping,
        context: Option<egui::Context>,
    ) -> Result<(), Error> {
        let processing = self
            .processing
            .as_ref()
            .ok_or_else(|| Error::Gpu("rendered image has no processing source".into()))?;
        let mut encoder = device.create_command_encoder(&Default::default());
        if self.prepare_calculation(device, &mut encoder, root, luts)? {
            processing
                .renderer
                .submissions
                .submit(queue, [encoder.finish()]);
        }
        // Source proxies include film margins and other undrawn pixels. Admit only the
        // consumer's composed footprint; failed texels remain marked in the shared image.
        let mut projections = self.look_projections.lock().unwrap();
        if let Some(index) = projections
            .iter()
            .position(|(region, _)| *region == mapping)
        {
            let current = projections.remove(index);
            projections.push(current);
        } else {
            let mut encoder = device.create_command_encoder(&Default::default());
            let check = processing.renderer.check_look_projection(
                device,
                &mut encoder,
                &self.linear,
                mapping,
            )?;
            processing
                .renderer
                .submissions
                .submit(queue, [encoder.finish()]);
            let pending = processing.renderer.submissions.track();
            check.begin(move || {
                drop(pending);
                if let Some(context) = context {
                    context.request_repaint();
                }
            });
            if projections.len() == 4 {
                projections.remove(0);
            }
            projections.push((mapping, check));
        }
        drop(projections);
        device
            .poll(wgpu::PollType::Poll)
            .map_err(|e| Error::Gpu(e.to_string()))?;
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
        crate::gpu::submission::SubmissionGate,
        egui::Context,
    )>,
    image: Arc<GpuImage>,
    calculation: Option<crate::gpu::evaluation::Value>,
    luts: crate::looks::LutChain,
    context: Option<egui::Context>,
    uv: TextureMapping,
    group: std::sync::Mutex<Option<wgpu::BindGroup>>,
}

impl ImagePaint {
    #[cfg(feature = "editor")]
    pub(crate) fn rendered(
        view: Arc<crate::gpu::view::RenderedView>,
        uv: TextureMapping,
        submissions: crate::gpu::submission::SubmissionGate,
        context: egui::Context,
    ) -> Self {
        Self {
            image: view.image.clone(),
            calculation: None,
            luts: Default::default(),
            context: Some(context.clone()),
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
        let calculation = parameters
            .map(|parameters| image.inspection(parameters, None))
            .transpose()
            .expect("admitted source-view transform");
        Self::with_calculation(image, calculation, Default::default(), uv)
            .expect("admitted source-view storage")
    }

    pub fn with_calculation(
        image: Arc<GpuImage>,
        calculation: Option<crate::gpu::evaluation::Value>,
        luts: crate::looks::LutChain,
        uv: impl Into<TextureMapping>,
    ) -> Result<Self, Error> {
        if let Some(root) = &calculation {
            image.require_calculation(root, &luts)?;
        } else if !luts.is_empty() {
            return Err(Error::InvalidParameter(
                "look rendering requires a calculation",
            ));
        }
        Ok(Self {
            #[cfg(feature = "editor")]
            rendered: None,
            image,
            calculation,
            luts,
            context: None,
            uv: uv.into(),
            group: std::sync::Mutex::new(None),
        })
    }

    #[cfg(test)]
    pub(crate) fn with_processing(
        image: Arc<GpuImage>,
        processing: Option<AuthoredPhotograph>,
        uv: impl Into<TextureMapping>,
    ) -> Result<Self, Error> {
        let calculation = processing
            .as_ref()
            .map(|input| image.legacy_calculation(input))
            .transpose()?;
        let luts = processing
            .as_ref()
            .map(|input| input.luts().clone())
            .unwrap_or_default();
        Self::with_calculation(image, calculation, luts, uv)
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn texture_mapping(&self) -> TextureMapping {
        self.uv
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn view_frame(&self) -> Option<crate::frame_raster::FrameRaster> {
        self.rendered.as_ref().map(|(view, _, _)| view.spec.frame())
    }

    #[cfg(feature = "editor")]
    pub(crate) fn displayed(
        &self,
    ) -> Option<(Arc<crate::gpu::view::RenderedView>, TextureMapping)> {
        self.rendered
            .as_ref()
            .map(|(view, _, _)| (view.clone(), self.uv))
    }

    pub fn with_context(mut self, context: egui::Context) -> Self {
        self.context = Some(context);
        self
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
        *self.image.paint_mapping.lock().unwrap() = Some(self.uv);
        #[cfg(feature = "editor")]
        if let Some((view, submissions, context)) = &self.rendered {
            view.ensure_submitted(device, _queue, submissions, context);
        }
        if let Some(root) = &self.calculation {
            self.image
                .prepare_rendering_calculation(
                    device,
                    _queue,
                    encoder,
                    (root, &self.luts),
                    self.uv,
                    self.context.clone(),
                )
                .expect("validated rendering resources");
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
        if matches!(self.image.look_result_for(self.uv), Some(Err(_))) {
            return;
        }
        match &*self.group.lock().unwrap() {
            Some(group) => self.image.paint_group(pass, group),
            None => self.image.paint(pass),
        }
    }
}
