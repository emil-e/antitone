//! One immutable camera proxy, with only the active target's mutable output retained.
use super::*;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct GpuProxy(Arc<ProxyState>);

struct ProxyState {
    device: wgpu::Device,
    renderer: Arc<GpuRenderer>,
    source: Arc<ProcessingSource>,
    #[cfg(feature = "editor")]
    identity: Option<crate::gpu::source::identity::Identity>,
    size: [u32; 2],
    active: std::sync::Mutex<Option<(OutputTarget, Arc<GpuImage>)>>,
}

impl GpuProxy {
    #[cfg(test)]
    pub(crate) fn new(
        renderer: Arc<GpuRenderer>,
        device: &wgpu::Device,
        size: [u32; 2],
        pixels: &[[u16; 4]],
        matrix: DMat3,
        geometry: SourceGeometry,
    ) -> Result<Self, Error> {
        if size[0].checked_mul(size[1]).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::InvalidParameter(
                "source dimensions do not match the GPU image",
            ));
        }
        let source = ProcessingSource::new(device, pixels, matrix, Some(geometry))?;
        Ok(Self::from_source(renderer, device, size, source))
    }

    pub(crate) fn new_continuous(
        renderer: Arc<GpuRenderer>,
        device: &wgpu::Device,
        size: [u32; 2],
        pixels: &[[f32; 4]],
        matrix: DMat3,
        geometry: SourceGeometry,
    ) -> Result<Self, Error> {
        if size[0].checked_mul(size[1]).map(u64::from) != Some(pixels.len() as u64) {
            return Err(Error::InvalidParameter(
                "source dimensions do not match the GPU image",
            ));
        }
        let source = ProcessingSource::new_continuous(device, pixels, matrix, Some(geometry))?;
        Ok(Self::from_source(renderer, device, size, source))
    }

    fn from_source(
        renderer: Arc<GpuRenderer>,
        device: &wgpu::Device,
        size: [u32; 2],
        source: ProcessingSource,
    ) -> Self {
        Self(Arc::new(ProxyState {
            device: device.clone(),
            renderer,
            source: Arc::new(source),
            #[cfg(feature = "editor")]
            identity: None,
            size,
            active: Default::default(),
        }))
    }

    #[cfg(feature = "editor")]
    pub(crate) fn with_identity(
        mut self,
        identity: crate::gpu::source::identity::Identity,
    ) -> Result<Self, Error> {
        let state = Arc::get_mut(&mut self.0).ok_or(Error::InvalidParameter(
            "proxy identity must be captured before sharing",
        ))?;
        state.identity = Some(identity);
        Ok(self)
    }

    #[cfg(feature = "editor")]
    pub(crate) fn same_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0.source, &other.0.source)
    }

    pub(crate) fn size(&self) -> [u32; 2] {
        self.0.size
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn matrix(&self) -> DMat3 {
        self.0.source.camera_matrix()
    }

    #[cfg(all(test, feature = "editor"))]
    pub(crate) fn view_source(
        &self,
        queue: &wgpu::Queue,
        submissions: &crate::gpu::submission::SubmissionGate,
        kernels: Arc<crate::gpu::view::ViewKernels>,
        input: &AuthoredPhotograph,
        spec: crate::gpu::view::request::SourceRequest,
    ) -> Result<Arc<crate::gpu::view::ViewSource>, Error> {
        crate::gpu::view::ViewSource::from_processing(
            &self.0.device,
            queue,
            submissions,
            self.0.renderer.clone(),
            kernels,
            self.0.source.clone(),
            input,
            spec.with_identity(self.0.identity.clone().unwrap_or_else(|| {
                crate::gpu::source::identity::Identity::Resident(Arc::downgrade(&self.0.source))
            })),
        )
    }

    #[cfg(feature = "editor")]
    pub(crate) fn preview(
        &self,
        queue: &wgpu::Queue,
        submissions: &crate::gpu::submission::SubmissionGate,
        kernels: Arc<crate::gpu::view::ViewKernels>,
        request: &crate::gpu::photograph::request::PhotographRequest,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        spec: crate::gpu::view::request::SourceRequest,
    ) -> Result<
        (
            Arc<crate::gpu::view::ViewSource>,
            Arc<crate::gpu::view::processing::ProcessedView>,
        ),
        Error,
    > {
        crate::gpu::view::ViewSource::processing_preview(
            &self.0.device,
            queue,
            submissions,
            self.0.renderer.clone(),
            kernels,
            self.0.source.clone(),
            request,
            field,
            spec.capture(self.0.identity.clone().unwrap_or_else(|| {
                crate::gpu::source::identity::Identity::Resident(Arc::downgrade(&self.0.source))
            }))?,
        )
    }

    #[cfg(test)]
    pub(crate) fn image(&self, target: OutputTarget) -> Result<Arc<GpuImage>, Error> {
        let mut active = self.0.active.lock().unwrap();
        if let Some((current, image)) = &*active
            && *current == target
        {
            return Ok(image.clone());
        }
        let image = Arc::new(pollster::block_on(GpuImage::create(
            &self.0.device,
            self.0.size[0],
            self.0.size[1],
            Some((self.0.renderer.clone(), self.0.source.clone())),
            self.0.renderer.display.clone(),
            target,
        ))?);
        *active = Some((target, image.clone()));
        Ok(image)
    }

    pub(crate) fn release_output(&self) {
        self.0.active.lock().unwrap().take();
    }

    pub(crate) fn byte_len(&self) -> u64 {
        let active = self.0.active.lock().unwrap();
        self.0.source.byte_len()
            + active.as_ref().map_or(0, |(_, image)| {
                image.processing_byte_len().1
                    + u64::from(self.0.size[0]) * u64::from(self.0.size[1]) * 16
            })
    }

    #[cfg(all(test, feature = "ui-tests"))]
    pub(crate) fn output_retained(&self) -> bool {
        self.0.active.lock().unwrap().is_some()
    }
}
