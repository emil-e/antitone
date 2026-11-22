//! Described image requests become view products through common evaluation.
use super::*;
use crate::gpu::evaluation::{Context, Evaluation, Image, ImageLayout, Value};

#[cfg(test)]
mod tests;

pub(crate) struct Intent<'a> {
    pub capture: &'a PhotographRequest,
    pub frame: FrameRaster,
    pub looks: &'a crate::gpu::lut3d::Looks,
    pub transform: &'a crate::gpu::drt::Target,
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

    pub(crate) fn produced(
        self,
        context: Context,
        kernels: Arc<ViewKernels>,
        image: Image<ImageLayout>,
    ) -> Result<Arc<ViewImage>, Error> {
        self.recorded(
            context,
            kernels,
            crate::gpu::evaluation::RecordedImage {
                pixels: image.try_into()?,
                commands: None,
            },
        )
    }
}

pub(crate) struct Preparation {
    images: Vec<ImageRequest>,
    work: Vec<crate::gpu::evaluation::Preparation>,
    context: Context,
    kernels: Arc<ViewKernels>,
}

impl Preparation {
    pub(crate) fn new(
        context: Context,
        kernels: Arc<ViewKernels>,
        images: Vec<ImageRequest>,
    ) -> Result<Self, Error> {
        let evaluations = images
            .iter()
            .map(|image| image.evaluation(&context))
            .collect::<Result<Vec<_>, _>>()?;
        let work = context.prepare_images(evaluations)?;
        Ok(Self {
            images,
            work,
            context,
            kernels,
        })
    }

    pub(crate) fn can_record(&self) -> bool {
        self.work.iter().all(|work| work.can_record())
    }

    pub(crate) fn record(self) -> Result<Vec<Arc<ViewImage>>, Error> {
        self.images
            .into_iter()
            .zip(self.work)
            .map(|(description, work)| {
                let image = work.record(&self.context)?;
                description.recorded(self.context.clone(), self.kernels.clone(), image)
            })
            .collect()
    }

    pub(crate) async fn produce(
        self,
        queue: &crate::gpu::submission::Queue,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<ViewImage>>>, Error> {
        let values = self.context.produce_images(queue, self.work)?;
        let mut values = Box::pin(futures_util::future::try_join_all(values));
        let result = futures_util::future::poll_fn(|cx| {
            if cancelled() {
                return std::task::Poll::Ready(None);
            }
            std::future::Future::poll(values.as_mut(), cx).map(Some)
        })
        .await;
        let Some(result) = result else {
            return Ok(None);
        };
        let values = match result {
            Ok(values) => values,
            Err(error) if matches!(error.as_ref(), Error::Cancelled) => return Ok(None),
            Err(error) => return Err((*error).clone()),
        };
        self.images
            .into_iter()
            .zip(values)
            .map(|(description, image)| {
                description.produced(self.context.clone(), self.kernels.clone(), image)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

/// One actual view request may expose a current-image provisional raster while production runs.
pub(crate) struct Request {
    ready: Mutex<Option<Vec<Arc<ViewImage>>>>,
    pending: Mutex<Option<Preparation>>,
}

impl Request {
    pub(crate) fn new(prepared: Preparation) -> Result<Self, Error> {
        if prepared.can_record() {
            Ok(Self {
                ready: Mutex::new(Some(prepared.record()?)),
                pending: Mutex::new(None),
            })
        } else {
            Ok(Self {
                ready: Mutex::new(None),
                pending: Mutex::new(Some(prepared)),
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn camera_proxy(
        context: Context,
        looks: &crate::gpu::lut3d::Looks,
        transform: &crate::gpu::drt::Target,
        kernels: Arc<ViewKernels>,
        source: &Arc<ProcessingSource>,
        field: Option<Arc<GpuFlatField>>,
        capture: &PhotographRequest,
        spec: SourceRequest,
    ) -> Result<Self, Error> {
        let spec = ViewImage::proxy_request(source, spec)?;
        let (definition, spec) = capture.sampled_output(
            &context.device().limits(),
            source,
            field,
            spec,
            looks,
            transform,
        )?;
        Self::new(Preparation::new(
            context,
            kernels,
            vec![ImageRequest::new(definition, spec)?],
        )?)
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
        let prepare = |spec: SourceRequest| {
            let underlay = spec.underlay()?;
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
            Preparation::new(context.clone(), kernels.clone(), images)
        };
        let prepared = prepare(spec.clone())?;
        if prepared.can_record() {
            return Self::new(prepared);
        }
        let provisional = previous
            .filter(|previous| {
                previous.same_source(&spec)
                    && previous.frame().composition == spec.frame().composition
                    && previous.frame() != spec.frame()
            })
            .map(|previous| prepare(previous.clone()))
            .transpose()?;
        let ready = match provisional {
            Some(provisional) if provisional.can_record() => Some(provisional.record()?),
            _ => None,
        };
        Ok(Self {
            ready: Mutex::new(ready),
            pending: Mutex::new(Some(prepared)),
        })
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
        let Some(pending) = pending else {
            return Ok(self.ready());
        };
        let result = pending.produce(queue, cancelled).await?;
        if let Some(images) = &result {
            *self.ready.lock().unwrap() = Some(images.clone());
        }
        Ok(result)
    }
}

pub(crate) fn photograph(
    limits: &wgpu::Limits,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    capture: &PhotographRequest,
    spec: SourceRequest,
    looks: &crate::gpu::lut3d::Looks,
    transform: &crate::gpu::drt::Target,
) -> Result<ImageRequest, Error> {
    let (definition, spec) = capture.preview_output(limits, scan, field, spec, looks, transform)?;
    ImageRequest::new(definition, spec)
}
