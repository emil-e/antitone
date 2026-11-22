use crate::Error;
use crate::geometry::TextureMapping;
use crate::gpu::allocation::DeviceAllocationExt;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::gpu::{drt, evaluation, lut3d::Looks, parameters::ProcessingParameters};
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
pub(crate) mod readback;

struct ImageProcessing {
    evaluation: evaluation::Context,
    looks: Arc<Looks>,
    transform: drt::Target,
    source: Arc<ProcessingSource>,
    working: wgpu::Buffer,
    recording: std::sync::Mutex<Option<crate::gpu::recorded_image::Texture>>,
    camera: std::sync::Mutex<Option<crate::gpu::evaluation::Value>>,
    last_calculation: std::sync::Mutex<Option<crate::gpu::evaluation::Value>>,
}

pub struct GpuImage {
    device_limits: wgpu::Limits,
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
        let evaluation = evaluation::Context::new(device, Default::default());
        let looks = Arc::new(Looks::new(device));
        let targets = drt::Targets::new(device, queue).await?;
        crate::gpu::photograph::prewarm_viewer(&evaluation, &targets)?;
        Self::new_shared(
            evaluation,
            looks,
            targets.get(target),
            size,
            pixels,
            camera_to_ap1,
            geometry,
        )
        .await
    }
    /// Create a resident camera image using shared resources from the same GPU device.
    pub async fn new_shared(
        evaluation: evaluation::Context,
        looks: Arc<Looks>,
        transform: &drt::Target,
        size: [u32; 2],
        pixels: &[[u16; 4]],
        camera_to_ap1: DMat3,
        geometry: Option<SourceGeometry>,
    ) -> Result<Self, Error> {
        if size[0].checked_mul(size[1]).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::Gpu(
                "source dimensions do not match the GPU image".into(),
            ));
        }
        let source = ProcessingSource::new(evaluation.device(), pixels, camera_to_ap1, geometry)?;
        Self::from_camera(evaluation, looks, transform, Arc::new(source), size).await
    }

    /// Continuous-camera counterpart to [`Self::new_shared`].
    pub async fn new_continuous_shared(
        evaluation: evaluation::Context,
        looks: Arc<Looks>,
        transform: &drt::Target,
        size: [u32; 2],
        pixels: &[[f32; 4]],
        camera_to_ap1: DMat3,
        geometry: Option<SourceGeometry>,
    ) -> Result<Self, Error> {
        if size[0].checked_mul(size[1]).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::Gpu(
                "source dimensions do not match the GPU image".into(),
            ));
        }
        let source =
            ProcessingSource::new_continuous(evaluation.device(), pixels, camera_to_ap1, geometry)?;
        Self::from_camera(evaluation, looks, transform, Arc::new(source), size).await
    }

    async fn from_camera(
        evaluation: evaluation::Context,
        looks: Arc<Looks>,
        transform: &drt::Target,
        source: Arc<ProcessingSource>,
        size: [u32; 2],
    ) -> Result<Self, Error> {
        let device = evaluation.device();
        let [width, height] = size;
        let mut image =
            Self::new_rendered(device, width, height, transform.output_target()).await?;
        let working = device.try_create_buffer(&wgpu::BufferDescriptor {
            label: Some("processed AP1 preview"),
            size: u64::from(width) * u64::from(height) * 16,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })?;
        image.processing = Some(ImageProcessing {
            evaluation,
            looks,
            transform: transform.clone(),
            source,
            working,
            recording: std::sync::Mutex::new(None),
            camera: std::sync::Mutex::new(None),
            last_calculation: std::sync::Mutex::new(None),
        });
        Ok(image)
    }

    pub async fn new_rendered(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        target: OutputTarget,
    ) -> Result<Self, Error> {
        width
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
        if let Some(error) = scope.pop().await {
            return Err(Error::Gpu(error.to_string()));
        }
        Ok(Self {
            device_limits: device.limits(),
            processing: None,
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
                    + p.recording.lock().unwrap().as_ref().map_or(0, |binding| {
                        binding.byte_len()
                            + crate::gpu::lut3d::applied_luts(binding.endpoint())
                                .iter()
                                .map(|lut| lut.byte_len())
                                .sum::<u64>()
                    }),
            )
        })
    }

    #[cfg(feature = "editor")]
    pub(crate) fn look_resources(&self) -> Vec<Arc<crate::gpu::lut3d::Lut>> {
        self.processing
            .as_ref()
            .map_or_else(Vec::new, |processing| {
                processing
                    .recording
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map_or_else(Vec::new, |binding| {
                        crate::gpu::lut3d::applied_luts(binding.endpoint())
                    })
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
    rendered: Option<(
        Arc<crate::gpu::view::RenderedView>,
        crate::gpu::submission::Queue,
        egui::Context,
    )>,
    image: Arc<GpuImage>,
    calculation: Option<crate::gpu::evaluation::Value>,
    luts: crate::looks::LutChain,
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
            draw,
            rendered: Some((view, queue, context)),
        }
    }

    pub fn with_calculation(
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
            image,
            calculation,
            luts,
            draw,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_processing(
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
        Self::with_calculation(presentation, image, calculation, luts, uv)
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn texture_mapping(&self) -> TextureMapping {
        self.draw.mapping()
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
            .map(|(view, _, _)| (view.clone(), self.draw.mapping()))
    }
}

impl ImagePaint {
    #[doc(hidden)]
    pub fn prepare_frame(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        #[cfg(feature = "editor")]
        if let Some((view, queue, context)) = &self.rendered {
            view.ensure_submitted(device, queue, context);
        }
        if let Some(root) = &self.calculation {
            self.image
                .prepare_calculation(device, encoder, root, &self.luts)
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
        self.draw.paint(pass);
    }
}
