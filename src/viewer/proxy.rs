//! Immutable camera-proxy input; presentation products own their complete endpoints.
use super::*;
use crate::gpu::source::ProcessingSource;
use crate::gpu::source::SourceGeometry;
#[cfg(test)]
use crate::gpu::test_inputs::AuthoredPhotograph;
use crate::renderer::GpuRenderer;
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct GpuProxy(Arc<ProxyState>);

struct ProxyState {
    renderer: Arc<GpuRenderer>,
    source: Arc<ProcessingSource>,
    #[cfg(feature = "editor")]
    identity: Option<crate::gpu::source::identity::Identity>,
    size: [u32; 2],
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
        Ok(Self::from_source(renderer, size, source))
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
        Ok(Self::from_source(renderer, size, source))
    }

    fn from_source(renderer: Arc<GpuRenderer>, size: [u32; 2], source: ProcessingSource) -> Self {
        Self(Arc::new(ProxyState {
            renderer,
            source: Arc::new(source),
            #[cfg(feature = "editor")]
            identity: None,
            size,
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
        queue: &crate::gpu::submission::Queue,
        kernels: Arc<crate::gpu::view::ViewKernels>,
        input: &AuthoredPhotograph,
        spec: crate::gpu::view::request::SourceRequest,
    ) -> Result<Arc<crate::gpu::view::ViewImage>, Error> {
        crate::gpu::view::ViewImage::from_processing(
            self.0.renderer.evaluation().device(),
            queue,
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
        kernels: Arc<crate::gpu::view::ViewKernels>,
        request: &crate::gpu::photograph::request::PhotographRequest,
        field: Option<Arc<crate::gpu::flat::GpuFlatField>>,
        spec: crate::gpu::view::request::SourceRequest,
        target: OutputTarget,
    ) -> Result<crate::gpu::view::rendering::Request, Error> {
        crate::gpu::view::rendering::Request::camera_proxy(
            self.0.renderer.clone(),
            kernels,
            &self.0.source,
            field,
            request,
            spec.capture(self.0.identity.clone().unwrap_or_else(|| {
                crate::gpu::source::identity::Identity::Resident(Arc::downgrade(&self.0.source))
            }))?,
            target,
        )
    }

    #[cfg(test)]
    pub(crate) fn image(&self, target: OutputTarget) -> Result<Arc<GpuImage>, Error> {
        Ok(Arc::new(pollster::block_on(GpuImage::from_camera(
            self.0.renderer.evaluation().clone(),
            self.0.renderer.looks().clone(),
            self.0.renderer.drt(target),
            self.0.source.clone(),
            self.0.size,
        ))?))
    }

    pub(crate) fn byte_len(&self) -> u64 {
        self.0.source.byte_len()
    }
}
