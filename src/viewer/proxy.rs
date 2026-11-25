//! Immutable camera-proxy input; presentation products own their complete endpoints.
use super::*;
use crate::acquisition::camera::ProcessingSource;
use crate::acquisition::camera::SourceGeometry;
#[cfg(test)]
use crate::photographic::test_inputs::AuthoredPhotograph;
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct GpuProxy(Arc<ProxyState>);

struct ProxyState {
    evaluation: evaluation::Context,
    looks: Arc<Looks>,
    targets: Arc<drt::Targets>,
    source: Arc<ProcessingSource>,
    #[cfg(feature = "editor")]
    identity: Option<crate::acquisition::camera::identity::Identity>,
    size: [u32; 2],
}

impl GpuProxy {
    #[cfg(test)]
    pub(crate) fn new(
        evaluation: evaluation::Context,
        looks: Arc<Looks>,
        targets: Arc<drt::Targets>,
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
        let source = ProcessingSource::new(evaluation.device(), pixels, matrix, Some(geometry))?;
        Ok(Self::from_source(evaluation, looks, targets, size, source))
    }

    pub(crate) fn new_continuous(
        evaluation: evaluation::Context,
        looks: Arc<Looks>,
        targets: Arc<drt::Targets>,
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
        let source =
            ProcessingSource::new_continuous(evaluation.device(), pixels, matrix, Some(geometry))?;
        Ok(Self::from_source(evaluation, looks, targets, size, source))
    }

    fn from_source(
        evaluation: evaluation::Context,
        looks: Arc<Looks>,
        targets: Arc<drt::Targets>,
        size: [u32; 2],
        source: ProcessingSource,
    ) -> Self {
        Self(Arc::new(ProxyState {
            evaluation,
            looks,
            targets,
            source: Arc::new(source),
            #[cfg(feature = "editor")]
            identity: None,
            size,
        }))
    }

    #[cfg(feature = "editor")]
    pub(crate) fn with_identity(
        mut self,
        identity: crate::acquisition::camera::identity::Identity,
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
        kernels: Arc<crate::viewer::view::ViewKernels>,
        input: &AuthoredPhotograph,
        spec: crate::photographic::graph::view_source::SourceRequest,
    ) -> Result<Arc<crate::viewer::view::ViewImage>, Error> {
        crate::viewer::view::ViewImage::from_processing_with_display(
            self.0.evaluation.clone(),
            (self.0.looks.clone(), self.0.targets.clone()),
            queue,
            kernels,
            self.0.source.clone(),
            input,
            spec.with_identity(self.0.identity.clone().unwrap_or_else(|| {
                crate::acquisition::camera::identity::Identity::Resident(Arc::downgrade(
                    &self.0.source,
                ))
            })),
        )
    }

    #[cfg(feature = "editor")]
    pub(crate) fn preview(
        &self,
        kernels: Arc<crate::viewer::view::ViewKernels>,
        request: &crate::photographic::graph::request::PhotographRequest,
        field: Option<Arc<crate::photographic::stages::flat_field::GpuFlatField>>,
        spec: crate::photographic::graph::view_source::SourceRequest,
        target: OutputTarget,
    ) -> Result<crate::viewer::view::rendering::Request, Error> {
        crate::viewer::view::rendering::Request::camera_proxy(
            self.0.evaluation.clone(),
            &self.0.looks,
            self.0.targets.get(target),
            kernels,
            &self.0.source,
            field,
            request,
            spec.capture(self.0.identity.clone().unwrap_or_else(|| {
                crate::acquisition::camera::identity::Identity::Resident(Arc::downgrade(
                    &self.0.source,
                ))
            }))?,
        )
    }

    #[cfg(test)]
    pub(crate) fn image(&self, target: OutputTarget) -> Result<Arc<GpuImage>, Error> {
        Ok(Arc::new(pollster::block_on(GpuImage::from_camera(
            self.0.evaluation.clone(),
            self.0.looks.clone(),
            self.0.targets.get(target),
            self.0.source.clone(),
            self.0.size,
        ))?))
    }

    pub(crate) fn byte_len(&self) -> u64 {
        self.0.source.byte_len()
    }
}
