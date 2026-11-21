use crate::Error;
use crate::geometry::TextureMapping;
use crate::gpu::allocation::DeviceAllocationExt;
use crate::gpu::parameters::ProcessingParameters;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::renderer::GpuRenderer;
use crate::rendering::OutputTarget;
use glam::DMat3;
use std::sync::Arc;
mod calculation;
mod presentation;
mod proxy;
pub use presentation::{Draw, Presentation};
pub(crate) use proxy::GpuProxy;
#[cfg(test)]
mod film_tests;
#[cfg(test)]
mod look_tests;
#[cfg(test)]
pub(crate) mod readback;

struct ImageProcessing {
    renderer: Arc<GpuRenderer>,
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
    target: OutputTarget,
    bindings: std::sync::Mutex<presentation::Bindings>,
}

impl GpuImage {
    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn device_limits(&self) -> &wgpu::Limits {
        &self.device_limits
    }

    pub async fn new(
        device: &wgpu::Device,
        queue: &crate::gpu::submission::Queue,
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
            target,
        )
        .await
    }

    pub async fn new_rendered(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        Self::create(device, width, height, None, target).await
    }

    async fn create(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        source: Option<(Arc<GpuRenderer>, Arc<ProcessingSource>)>,
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
        let linear = device.try_create_texture(&wgpu::TextureDescriptor {
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
        })?;
        let processing = source
            .map(|(renderer, source)| -> Result<_, Error> {
                let working = device.try_create_buffer(&wgpu::BufferDescriptor {
                    label: Some("processed AP1 preview"),
                    size: u64::from(count) * 16,
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })?;
                Ok(ImageProcessing {
                    renderer,
                    source,
                    working,
                    aces_group: std::sync::Mutex::new(None),
                    camera: std::sync::Mutex::new(None),
                    last_calculation: std::sync::Mutex::new(None),
                })
            })
            .transpose()?;
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
            target,
            bindings: Default::default(),
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
                        .map_or(0, |binding| binding.retained_bytes()),
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
            .and_then(|prefix| self.output(prefix, &Default::default()))
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
        submissions: &crate::gpu::submission::Queue,
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
        queue: &crate::gpu::submission::Queue,
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
        queue: &crate::gpu::submission::Queue,
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
        queue: &crate::gpu::submission::Queue,
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
            queue.submit([encoder.finish()]);
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
            let check = processing.renderer.looks.check_projection(
                device,
                &mut encoder,
                &self.linear,
                mapping,
            )?;
            queue.submit([encoder.finish()]);
            let pending = queue.track();
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

    fn size(&self) -> [u32; 2] {
        [self.linear.width(), self.linear.height()]
    }

    fn presentation_draw(
        &self,
        presentation: &Presentation,
        mapping: TextureMapping,
    ) -> Result<Draw, Error> {
        self.bindings
            .lock()
            .unwrap()
            .prepare(presentation, &self.linear, self.target, mapping)
    }
}

pub struct ImagePaint {
    #[cfg(feature = "editor")]
    rendered: Option<(Arc<crate::gpu::view::RenderedView>, egui::Context)>,
    queue: crate::gpu::submission::Queue,
    image: Arc<GpuImage>,
    calculation: Option<crate::gpu::evaluation::Value>,
    luts: crate::looks::LutChain,
    context: Option<egui::Context>,
    draw: Draw,
}

impl ImagePaint {
    #[cfg(feature = "editor")]
    pub(crate) fn rendered(
        presentation: &Presentation,
        view: Arc<crate::gpu::view::RenderedView>,
        uv: TextureMapping,
        queue: crate::gpu::submission::Queue,
        context: egui::Context,
    ) -> Result<Self, Error> {
        let draw = view.image.presentation_draw(presentation, uv)?;
        Ok(Self::from_rendered(view, draw, queue, context))
    }

    #[cfg(feature = "editor")]
    pub(crate) fn retained(
        presentation: &Presentation,
        view: Arc<crate::gpu::view::RenderedView>,
        queue: crate::gpu::submission::Queue,
        context: egui::Context,
    ) -> Result<Self, Error> {
        let previous = view.image.bindings.lock().unwrap().retained();
        let draw = match previous {
            Some(draw) => draw,
            None => view
                .image
                .presentation_draw(presentation, TextureMapping::IDENTITY)?,
        };
        Ok(Self::from_rendered(view, draw, queue, context))
    }

    #[cfg(feature = "editor")]
    fn from_rendered(
        view: Arc<crate::gpu::view::RenderedView>,
        draw: Draw,
        queue: crate::gpu::submission::Queue,
        context: egui::Context,
    ) -> Self {
        Self {
            image: view.image.clone(),
            calculation: None,
            luts: Default::default(),
            context: None,
            draw,
            queue,
            rendered: Some((view, context)),
        }
    }

    pub fn with_calculation(
        queue: crate::gpu::submission::Queue,
        presentation: &Presentation,
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
        let uv = uv.into();
        let draw = image.presentation_draw(presentation, uv)?;
        Ok(Self {
            #[cfg(feature = "editor")]
            rendered: None,
            queue,
            image,
            calculation,
            luts,
            context: None,
            draw,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_processing(
        queue: crate::gpu::submission::Queue,
        presentation: &Presentation,
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
        Self::with_calculation(queue, presentation, image, calculation, luts, uv)
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn texture_mapping(&self) -> TextureMapping {
        self.draw.mapping()
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn view_frame(&self) -> Option<crate::frame_raster::FrameRaster> {
        self.rendered.as_ref().map(|(view, _)| view.spec.frame())
    }

    #[cfg(feature = "editor")]
    pub(crate) fn displayed(
        &self,
    ) -> Option<(Arc<crate::gpu::view::RenderedView>, TextureMapping)> {
        self.rendered
            .as_ref()
            .map(|(view, _)| (view.clone(), self.draw.mapping()))
    }

    pub fn with_context(mut self, context: egui::Context) -> Self {
        self.context = Some(context);
        self
    }
}

impl ImagePaint {
    #[doc(hidden)]
    pub fn prepare_frame(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        let mapping = self.draw.mapping();
        *self.image.paint_mapping.lock().unwrap() = Some(mapping);
        #[cfg(feature = "editor")]
        if let Some((view, context)) = &self.rendered {
            view.ensure_submitted(device, &self.queue, context);
        }
        if let Some(root) = &self.calculation {
            self.image
                .prepare_rendering_calculation(
                    device,
                    &self.queue,
                    encoder,
                    (root, &self.luts),
                    mapping,
                    self.context.clone(),
                )
                .expect("validated rendering resources");
        }
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
        self.prepare_frame(device, encoder);
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        _resources: &egui_wgpu::CallbackResources,
    ) {
        if matches!(
            self.image.look_result_for(self.draw.mapping()),
            Some(Err(_))
        ) {
            return;
        }
        self.draw.paint(pass);
    }
}
