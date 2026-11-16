//! A requested image, its physical result and any commands still awaiting submission.

use super::*;
use crate::geometry::Composition;
use crate::gpu::evaluation::{self as e, BufferImage, Recorded, RecordedImage, Value};
use crate::gpu::source::SourceGeometry;

// Active evaluation storage is independent of optional generated-image retention.
const VIEW_SCRATCH_BYTES: u64 = 1280 << 20;

pub struct ViewImage {
    pub spec: SourceRequest,
    pub renderer: Arc<crate::renderer::GpuRenderer>,
    pub(super) kernels: Arc<ViewKernels>,
    pub definition: Value,
    pub region: e::CellRegion,
    pub pixels: BufferImage,
    pub commands: Option<Recorded>,
    pub submitted: Mutex<bool>,
    #[cfg(test)]
    pub(super) argument: Option<(SourceRequest, e::SuppliedImage)>,
}

impl ViewImage {
    /// Navigation positives process the already-sampled camera raster, not the originals.
    #[allow(clippy::too_many_arguments)]
    pub fn camera_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        proxy: &crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        request: &crate::gpu::photograph::request::PhotographRequest,
        target: crate::rendering::OutputTarget,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Self>, Error> {
        request.require_proxy(proxy)?;
        let spec = spec.capture(crate::gpu::source::identity::Identity::proxy(proxy))?;
        let mut frame = FrameRaster::full(Composition::new(
            spec.area,
            Default::default(),
            Default::default(),
        )?)?;
        frame.output = proxy.dimensions();
        if proxy.width.max(proxy.height) > crate::scan::CameraProxy::MAX_EDGE
            || proxy.pixels.len() as u64 != u64::from(proxy.width) * u64::from(proxy.height)
            || spec.frame() != frame
        {
            return Err(Error::Gpu(
                "camera proxy does not match thumbnail source raster".into(),
            ));
        }
        let area = spec.area;
        let spec = spec.with_sampling(crate::acquisition::Sampling::CameraCells {
            region: area,
            size: proxy.dimensions(),
        })?;
        let camera = ProcessingSource::new_continuous(
            device,
            &proxy.pixels,
            proxy.matrix,
            Some(SourceGeometry::new(
                [spec.area.x, spec.area.y, spec.area.width, spec.area.height],
                proxy.dimensions(),
            )?),
        )?;
        let (root, spec) = request.sampled_output(&renderer, &camera, field, spec, target)?;
        Self::on_worker(
            renderer,
            kernels,
            spec,
            root,
            (queue, submissions),
            cancelled,
        )
    }

    pub fn processing_preview(
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        source: Arc<ProcessingSource>,
        capture: &PhotographRequest,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        target: OutputTarget,
    ) -> Result<Arc<Self>, Error> {
        let spec = Self::proxy_request(&source, spec)?;
        let (root, spec) = capture.sampled_output(&renderer, &source, field, spec, target)?;
        Self::record(renderer, kernels, spec, root)
    }

    pub(super) fn proxy_request(
        source: &Arc<ProcessingSource>,
        spec: SourceRequest,
    ) -> Result<SourceRequest, Error> {
        let spec = if spec.identity.captured() {
            spec
        } else {
            spec.capture(crate::gpu::source::identity::Identity::Resident(
                Arc::downgrade(source),
            ))?
        };
        let geometry = source
            .geometry()
            .ok_or_else(|| Error::Gpu("preview source has no geometry".into()))?;
        if geometry.region() != [spec.area.x, spec.area.y, spec.area.width, spec.area.height].into()
        {
            return Err(Error::Gpu(
                "preview source geometry differs from the view frame".into(),
            ));
        }
        let area = spec.area;
        spec.with_sampling(crate::acquisition::Sampling::CameraProxy {
            region: area,
            size: geometry.size(),
        })
    }

    pub(crate) fn kernels(&self) -> &Arc<ViewKernels> {
        &self.kernels
    }

    pub(crate) fn evaluation(context: &e::Context, root: Value) -> Result<e::Evaluation, Error> {
        let domain = root.domain();
        let bounds = domain.finite()?;
        context.request(
            root,
            e::Region {
                bounds,
                grid: domain.grid,
            },
            e::Budget {
                tile: [bounds.width, bounds.height],
                scratch_bytes: VIEW_SCRATCH_BYTES,
            },
        )
    }

    pub(crate) fn from_recorded(
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        spec: SourceRequest,
        definition: Value,
        region: e::CellRegion,
        image: RecordedImage,
    ) -> Result<Arc<Self>, Error> {
        let image = Arc::new(Self {
            renderer,
            kernels,
            spec,
            definition,
            region,
            pixels: image.pixels,
            submitted: Mutex::new(image.commands.is_none()),
            commands: image.commands,
            #[cfg(test)]
            argument: None,
        });
        image.supplied()?;
        Ok(image)
    }

    pub(crate) fn supplied(&self) -> Result<e::SuppliedImage, Error> {
        e::SuppliedImage::from_buffer(self.definition.clone(), self.region, self.pixels.clone())
    }

    pub(crate) fn record(
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        spec: SourceRequest,
        definition: Value,
    ) -> Result<Arc<Self>, Error> {
        let work = renderer
            .evaluation()
            .prepare_image(Self::evaluation(renderer.evaluation(), definition.clone())?)?;
        let image = work.record(renderer.evaluation())?;
        let region = definition.domain().finite()?;
        Self::from_recorded(renderer, kernels, spec, definition, region, image)
    }

    /// Existing workers fulfil missing production inline; painting only calls `record`.
    pub(crate) fn on_worker(
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        spec: SourceRequest,
        definition: Value,
        gpu: (&wgpu::Queue, &SubmissionGate),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Self>, Error> {
        let work = renderer
            .evaluation()
            .prepare_image(Self::evaluation(renderer.evaluation(), definition.clone())?)?;
        let image = if work.can_record() {
            work.record(renderer.evaluation())?
        } else {
            RecordedImage {
                pixels: renderer
                    .evaluation()
                    .produce_prepared(gpu.0, gpu.1, work, cancelled)?
                    .try_into()?,
                commands: None,
            }
        };
        let region = definition.domain().finite()?;
        Self::from_recorded(renderer, kernels, spec, definition, region, image)
    }

    pub(crate) fn byte_len(&self) -> u64 {
        let mut allocations = e::Allocations::default();
        allocations.buffered(&self.pixels);
        for camera in
            crate::gpu::photograph::PhotographGraph::resident_camera_buffers(&self.definition)
        {
            allocations.buffer(&camera);
        }
        if let Some(commands) = &self.commands {
            commands.allocations(&mut allocations);
        }
        allocations.bytes()
    }

    pub fn encode(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if let Some(commands) = &self.commands {
            commands
                .encode_result(device, encoder, &self.pixels)
                .expect("validated image recording");
        }
    }
}
