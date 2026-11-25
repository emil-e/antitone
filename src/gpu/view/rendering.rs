//! Described image requests become view products through common evaluation.
use super::*;
use crate::gpu::evaluation::{
    Context, Evaluation, ImageDelivery, PendingImages, RecordedImage, Value,
};

#[cfg(test)]
mod tests;

pub(crate) struct Intent<'a> {
    pub capture: &'a PhotographRequest,
    pub frame: FrameRaster,
    pub looks: &'a crate::photographic::stages::look::Looks,
    pub transform: &'a crate::photographic::stages::display_rendering::Target,
}

pub(crate) struct ImageRequest {
    pub definition: Value,
    pub spec: SourceRequest,
}

impl ImageRequest {
    pub(crate) fn new(definition: Value, spec: SourceRequest) -> Result<Self, Error> {
        let bounds = definition.domain().finite()?;
        if [bounds.width, bounds.height] != spec.frame().output {
            return Err(Error::InvalidParameter(
                "view image differs from requested output size",
            ));
        }
        Ok(Self { definition, spec })
    }

    pub(crate) fn evaluation(&self, context: &Context) -> Result<Evaluation, Error> {
        ViewImage::evaluation(context, self.definition.clone())
    }

    pub(crate) fn recorded(
        self,
        context: Context,
        kernels: Arc<ViewKernels>,
        image: crate::gpu::evaluation::RecordedImage,
    ) -> Result<Arc<ViewImage>, Error> {
        let region = self.definition.domain().finite()?;
        ViewImage::from_recorded(context, kernels, self.spec, self.definition, region, image)
    }
}

fn view_images(
    context: &Context,
    kernels: &Arc<ViewKernels>,
    requests: Vec<ImageRequest>,
    images: Vec<RecordedImage>,
) -> Result<Vec<Arc<ViewImage>>, Error> {
    requests
        .into_iter()
        .zip(images)
        .map(|(request, image)| request.recorded(context.clone(), kernels.clone(), image))
        .collect()
}

struct PendingView {
    job: PendingImages,
    requests: Vec<ImageRequest>,
    context: Context,
    kernels: Arc<ViewKernels>,
}

/// One actual view request may expose a current-image provisional raster while production runs.
pub(crate) struct Request {
    ready: Mutex<Option<Vec<Arc<ViewImage>>>>,
    pending: Mutex<Option<PendingView>>,
}

impl Request {
    fn new(
        context: Context,
        kernels: Arc<ViewKernels>,
        requests: Vec<ImageRequest>,
    ) -> Result<Self, Error> {
        let evaluations = requests
            .iter()
            .map(|request| request.evaluation(&context))
            .collect::<Result<Vec<_>, _>>()?;
        let (ready, pending) = match context.request_images(evaluations)? {
            ImageDelivery::Drawable(images) => (
                Some(view_images(&context, &kernels, requests, images)?),
                None,
            ),
            ImageDelivery::Pending(job) => (
                None,
                Some(PendingView {
                    job,
                    requests,
                    context,
                    kernels,
                }),
            ),
        };
        Ok(Self {
            ready: Mutex::new(ready),
            pending: Mutex::new(pending),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn camera_proxy(
        context: Context,
        looks: &crate::photographic::stages::look::Looks,
        transform: &crate::photographic::stages::display_rendering::Target,
        kernels: Arc<ViewKernels>,
        source: &Arc<ProcessingSource>,
        field: Option<Arc<GpuFlatField>>,
        capture: &PhotographRequest,
        spec: SourceRequest,
    ) -> Result<Self, Error> {
        let spec = ViewImage::proxy_request(source, capture, spec)?;
        let (definition, spec) = capture.sampled_output(
            &context.device().limits(),
            source,
            field,
            spec,
            looks,
            transform,
        )?;
        Self::new(context, kernels, vec![ImageRequest::new(definition, spec)?])
    }

    pub(crate) fn photograph(
        context: Context,
        kernels: Arc<ViewKernels>,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        intent: Intent<'_>,
        previous: Option<&SourceRequest>,
    ) -> Result<Self, Error> {
        let Intent {
            capture,
            frame,
            looks,
            transform,
        } = intent;
        #[cfg(test)]
        let spec = if matches!(capture, PhotographRequest::Fixture(_)) {
            capture.source_request(frame)?
        } else {
            SourceRequest::for_frame(frame)?
        };
        #[cfg(not(test))]
        let spec = SourceRequest::for_frame(frame)?;
        let spec = spec.with_identity(crate::gpu::source::identity::Identity::camera(
            scan.source().identity(),
        ));
        let limits = context.device().limits();
        let request_view = |spec: SourceRequest| {
            let underlay = SourceRequest::for_frame(underlay_frame(spec.frame())?)?
                .with_identity(spec.identity.clone());
            let images = [spec, underlay]
                .into_iter()
                .map(|spec| {
                    photograph(
                        &limits,
                        scan,
                        field.clone(),
                        capture,
                        spec,
                        looks,
                        transform,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            Self::new(context.clone(), kernels.clone(), images)
        };
        let mut request = request_view(spec.clone())?;
        if request.pending.get_mut().unwrap().is_some()
            && let Some(previous) = previous.filter(|previous| {
                previous.same_source(&spec)
                    && previous.frame().composition == spec.frame().composition
                    && previous.frame() != spec.frame()
            })
        {
            // A provisional view is useful only if it can draw without waiting.
            let provisional = request_view(previous.clone())?;
            request.ready = provisional.ready;
        }
        Ok(request)
    }

    pub(crate) fn ready(&self) -> Option<Vec<Arc<ViewImage>>> {
        self.ready.lock().unwrap().clone()
    }

    pub(crate) async fn produce(
        &self,
        queue: &crate::gpu::submission::Queue,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<ViewImage>>>, Error> {
        let pending = self.pending.lock().unwrap().take();
        let Some(PendingView {
            job,
            requests,
            context,
            kernels,
        }) = pending
        else {
            return Ok(self.ready());
        };
        let images = match job.finish(queue, cancelled).await {
            Ok(images) => view_images(&context, &kernels, requests, images)?,
            Err(Error::Cancelled) => return Ok(None),
            Err(error) => return Err(error),
        };
        *self.ready.lock().unwrap() = Some(images.clone());
        Ok(Some(images))
    }
}

pub(crate) fn photograph(
    limits: &wgpu::Limits,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    capture: &PhotographRequest,
    spec: SourceRequest,
    looks: &crate::photographic::stages::look::Looks,
    transform: &crate::photographic::stages::display_rendering::Target,
) -> Result<ImageRequest, Error> {
    let (definition, spec) = capture.preview_output(limits, scan, field, spec, looks, transform)?;
    ImageRequest::new(definition, spec)
}
