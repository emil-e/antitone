//! Described image requests become view products through common evaluation.
use super::*;
use crate::gpu::evaluation::{Context, Evaluation, Image, ImageLayout, Value};

#[cfg(test)]
mod tests;

pub(crate) struct ImageRequest {
    pub definition: Value,
    pub spec: SourceRequest,
}

#[derive(Clone)]
pub(crate) struct ViewImage {
    pub source: Arc<ViewSource>,
    pub processed: Arc<processing::ProcessedView>,
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
        processing::ProcessedView::request(context, self.definition.clone())
    }

    pub(crate) fn recorded(
        self,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        image: crate::gpu::evaluation::RecordedImage,
    ) -> Result<ViewImage, Error> {
        let source = Arc::new(acquisition::AcquiredViewRaster {
            spec: self.spec,
            renderer,
            pixels: image.pixels.clone(),
            definition: self.definition.clone(),
        });
        let plan = source.spec.plan.as_ref().clone();
        let stages = image.commands.map(|commands| {
            crate::gpu::pixel_pipeline::Prepared::recorded(
                crate::gpu::raster::Raster::planned(&plan),
                commands,
            )
        });
        let submitted = stages.is_none();
        let processed = Arc::new(processing::ProcessedView {
            source: source.clone(),
            definition: self.definition,
            plan,
            pixels: image.pixels,
            stages,
            submitted: Mutex::new(submitted),
        });
        Ok(ViewImage {
            source: ViewSource::from_acquired(kernels, source),
            processed,
        })
    }

    pub(crate) fn produced(
        self,
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        image: Image<ImageLayout>,
    ) -> Result<ViewImage, Error> {
        let source = Arc::new(acquisition::AcquiredViewRaster {
            spec: self.spec,
            renderer,
            pixels: image.clone().try_into()?,
            definition: self.definition.clone(),
        });
        let processed = processing::ProcessedView::produced(
            source.clone(),
            self.definition,
            source.spec.plan.as_ref().clone(),
            image,
        )?;
        Ok(ViewImage {
            source: ViewSource::from_acquired(kernels, source),
            processed,
        })
    }
}

pub(crate) struct Preparation {
    images: Vec<ImageRequest>,
    work: Vec<crate::gpu::evaluation::Preparation>,
    renderer: Arc<crate::renderer::GpuRenderer>,
    kernels: Arc<ViewKernels>,
}

impl Preparation {
    pub(crate) fn new(
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        images: Vec<ImageRequest>,
    ) -> Result<Self, Error> {
        let evaluations = images
            .iter()
            .map(|image| image.evaluation(renderer.evaluation()))
            .collect::<Result<Vec<_>, _>>()?;
        let work = renderer.evaluation().prepare_images(evaluations)?;
        Ok(Self {
            images,
            work,
            renderer,
            kernels,
        })
    }

    pub(crate) fn photograph(
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        capture: &PhotographRequest,
        spec: SourceRequest,
    ) -> Result<Self, Error> {
        let specs = [spec.clone(), spec.underlay()?];
        let images = specs
            .into_iter()
            .map(|spec| {
                photograph(
                    &renderer.evaluation().device().limits(),
                    scan,
                    field.clone(),
                    capture,
                    spec,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(renderer, kernels, images)
    }

    pub(crate) fn can_record(&self) -> bool {
        self.work.iter().all(|work| work.can_record())
    }

    pub(crate) fn record(self) -> Result<Vec<ViewImage>, Error> {
        self.images
            .into_iter()
            .zip(self.work)
            .map(|(description, work)| {
                let image = work.record(self.renderer.evaluation())?;
                description.recorded(self.renderer.clone(), self.kernels.clone(), image)
            })
            .collect()
    }

    pub(crate) async fn produce(
        self,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<ViewImage>>, Error> {
        let values = self
            .renderer
            .evaluation()
            .produce_images(queue, submissions, self.work)?;
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
            Err(error) => return Err(Error::Gpu(error.to_string())),
        };
        self.images
            .into_iter()
            .zip(values)
            .map(|(description, image)| {
                description.produced(self.renderer.clone(), self.kernels.clone(), image)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some)
    }
}

/// One actual view request may expose a current-image provisional raster while production runs.
pub(crate) struct Request {
    ready: Mutex<Option<Vec<ViewImage>>>,
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

    pub(crate) fn photograph(
        renderer: Arc<crate::renderer::GpuRenderer>,
        kernels: Arc<ViewKernels>,
        scan: &crate::scan::Scan,
        field: Option<Arc<GpuFlatField>>,
        capture: &PhotographRequest,
        frame: FrameRaster,
        previous: Option<&SourceRequest>,
    ) -> Result<Self, Error> {
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
        let prepared = Preparation::photograph(
            renderer.clone(),
            kernels.clone(),
            scan,
            field.clone(),
            capture,
            spec.clone(),
        )?;
        if prepared.can_record() {
            return Self::new(prepared);
        }
        let provisional = previous
            .filter(|previous| {
                previous.same_source(&spec)
                    && previous.frame().composition == spec.frame().composition
                    && previous.frame() != spec.frame()
            })
            .map(|previous| {
                Preparation::photograph(renderer, kernels, scan, field, capture, previous.clone())
            })
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

    pub(crate) fn ready(&self) -> Option<Vec<ViewImage>> {
        self.ready.lock().unwrap().clone()
    }

    pub(crate) async fn produce(
        &self,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<ViewImage>>, Error> {
        let pending = self.pending.lock().unwrap().take();
        let Some(pending) = pending else {
            return Ok(self.ready());
        };
        let result = pending.produce(queue, submissions, cancelled).await?;
        if let Some(images) = &result {
            *self.ready.lock().unwrap() = Some(images.clone());
        }
        Ok(result)
    }
}

/// Photographic authoring selects an image endpoint; viewing executes the resulting Value.
pub(crate) fn photograph(
    limits: &wgpu::Limits,
    scan: &crate::scan::Scan,
    field: Option<Arc<GpuFlatField>>,
    capture: &PhotographRequest,
    mut spec: SourceRequest,
) -> Result<ImageRequest, Error> {
    let (definition, plan) = match capture {
        PhotographRequest::Recipe(recipe) | PhotographRequest::BeforeLooks(recipe) => {
            let (photograph, plan) = crate::gpu::photograph::PhotographGraph::preview(
                limits,
                scan,
                field,
                recipe,
                spec.frame(),
                None,
            )?;
            (photograph.processed, plan)
        }
        #[cfg(test)]
        PhotographRequest::Fixture(input) if input.is_positive() => {
            let (photograph, plan) = crate::gpu::photograph::PhotographGraph::preview_with_input(
                limits,
                crate::gpu::evaluation::camera_source(scan)?,
                field,
                input,
                spec.frame(),
                None,
            )?;
            (photograph.processed, plan)
        }
        _ => {
            let source = capture
                .source_graphs(scan, field, std::slice::from_ref(&spec))?
                .remove(0);
            capture.filtered(limits, &spec, source)?
        }
    };
    spec.plan = Arc::new(plan);
    ImageRequest::new(definition, spec)
}
