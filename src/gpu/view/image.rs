//! A requested image, its physical result and any commands still awaiting submission.

use super::*;
use crate::geometry::Composition;
use crate::gpu::evaluation::{self as e, BufferImage, Recorded, RecordedImage, Value};
use crate::gpu::source::SourceGeometry;

pub struct ViewImage {
    pub spec: SourceRequest,
    pub(crate) context: e::Context,
    #[cfg(test)]
    pub(crate) fixture_display: Option<super::fixtures::FixtureDisplay>,
    pub(super) kernels: Arc<ViewKernels>,
    pub definition: Value,
    pub region: e::CellRegion,
    pub pixels: BufferImage,
    pub(super) pending: Mutex<Option<Recorded>>,
    #[cfg(test)]
    pub(super) argument: Option<(SourceRequest, e::SuppliedImage)>,
}

impl ViewImage {
    /// Navigation positives process the already-sampled camera raster, not the originals.
    #[allow(clippy::too_many_arguments)]
    pub fn camera_preview(
        context: e::Context,
        queue: &crate::gpu::submission::Queue,
        looks: &crate::gpu::lut3d::Looks,
        transform: &crate::gpu::drt::Target,
        kernels: Arc<ViewKernels>,
        proxy: &crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        request: &PhotographRequest,
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
        let spec = request.plan_sampling(
            spec,
            crate::acquisition::Sampling::CameraCells {
                region: area,
                size: proxy.dimensions(),
            },
        )?;
        let device = context.device();
        let camera = ProcessingSource::new_continuous(
            device,
            &proxy.pixels,
            proxy.matrix,
            Some(SourceGeometry::new(
                [spec.area.x, spec.area.y, spec.area.width, spec.area.height],
                proxy.dimensions(),
            )?),
        )?;
        let (root, spec) =
            request.sampled_output(&device.limits(), &camera, field, spec, looks, transform)?;
        Self::on_worker(context, kernels, spec, root, queue, cancelled)
    }

    pub(super) fn proxy_request(
        source: &Arc<ProcessingSource>,
        capture: &PhotographRequest,
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
        capture.plan_sampling(
            spec,
            crate::acquisition::Sampling::CameraProxy {
                region: area,
                size: geometry.size(),
            },
        )
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
            e::Tiling {
                tile: [bounds.width, bounds.height],
            },
        )
    }

    pub(crate) fn from_recorded(
        context: e::Context,
        kernels: Arc<ViewKernels>,
        spec: SourceRequest,
        definition: Value,
        region: e::CellRegion,
        image: RecordedImage,
    ) -> Result<Arc<Self>, Error> {
        let image = Arc::new(Self {
            context,
            #[cfg(test)]
            fixture_display: None,
            kernels,
            spec,
            definition,
            region,
            pixels: image.pixels,
            pending: Mutex::new(image.commands),
            #[cfg(test)]
            argument: None,
        });
        image.supplied()?;
        Ok(image)
    }

    pub(crate) fn supplied(&self) -> Result<e::SuppliedImage, Error> {
        e::SuppliedImage::from_buffer(self.definition.clone(), self.region, self.pixels.clone())
    }

    /// Existing workers can fulfil missing production without queueing another worker.
    pub(crate) fn on_worker(
        context: e::Context,
        kernels: Arc<ViewKernels>,
        spec: SourceRequest,
        definition: Value,
        gpu: &crate::gpu::submission::Queue,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Arc<Self>, Error> {
        let image = context.request_image_on_worker(
            Self::evaluation(&context, definition.clone())?,
            gpu,
            cancelled,
        )?;
        let region = definition.domain().finite()?;
        Self::from_recorded(context, kernels, spec, definition, region, image)
    }

    pub(crate) fn byte_len(&self) -> u64 {
        let mut allocations = e::Allocations::default();
        allocations.buffer(&self.pixels);
        for camera in
            crate::photographic::graph::PhotographGraph::resident_camera_buffers(&self.definition)
        {
            allocations.buffer(&camera);
        }
        if let Some(commands) = self.recording().as_ref() {
            commands.allocations(&mut allocations);
        }
        allocations.bytes()
    }

    pub(crate) fn recording(&self) -> std::sync::MutexGuard<'_, Option<Recorded>> {
        self.pending.lock().unwrap()
    }

    #[cfg(test)]
    pub(crate) fn encode(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder) {
        if let Some(commands) = self.recording().as_ref() {
            commands
                .encode_result(device, encoder, &self.pixels)
                .expect("validated image recording");
        }
    }
}
