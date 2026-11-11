//! Bounded source acquisition at neutral-footprint and supplied-camera sampling boundaries.

#[cfg(test)]
use crate::scan::Scan;
use crate::{
    Error,
    frame_raster::FrameRaster,
    geometry::Composition,
    gpu::{
        flat::GpuFlatField,
        source::{ProcessingSource, SourceGeometry},
        submission::SubmissionGate,
    },
};
use std::sync::Arc;

use super::storage::float_buffer;
use crate::gpu::view::processing::ProcessedView;
use crate::gpu::view::request::SourceRequest;

pub struct AcquiredViewRaster {
    pub spec: SourceRequest,
    pub renderer: Arc<crate::renderer::GpuRenderer>,
    pub pixels: crate::gpu::evaluation::BufferImage,
    pub definition: crate::gpu::evaluation::Value,
}

impl AcquiredViewRaster {
    pub fn supplied(&self) -> Result<crate::gpu::evaluation::SuppliedImage, Error> {
        let region = match &self.pixels {
            crate::gpu::evaluation::BufferImage::Produced(value) => value.descriptor.region,
            crate::gpu::evaluation::BufferImage::Submitted(_) => self.spec.input_region()?,
        };
        crate::gpu::evaluation::SuppliedImage::from_buffer(
            self.definition.clone(),
            region,
            self.pixels.clone(),
        )
    }

    /// Navigation positives process the already-sampled camera raster, not the originals.
    #[allow(clippy::too_many_arguments)]
    pub fn camera_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        proxy: &crate::scan::CameraProxy,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
        request: &crate::gpu::photograph::request::PhotographRequest,
    ) -> Result<(Arc<Self>, Arc<ProcessedView>), Error> {
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
        let graph = request.sampled_graph(&device.limits(), &camera, field, spec)?;
        let spec = graph.request;
        let definition = graph.baseline;
        let geometry = camera.geometry().unwrap();
        let native = crate::gpu::pixel_pipeline::Prepared::from_composition(
            device,
            crate::gpu::raster::Raster::region(geometry.region(), geometry.size()),
            definition.clone(),
            renderer.evaluation.clone(),
        )?;
        let pixels = crate::gpu::evaluation::BufferImage::recorded(
            &definition,
            definition.domain().finite()?,
            float_buffer(device, proxy.dimensions())?,
        )?;
        let source = Arc::new(Self {
            pixels,
            spec,
            definition,
            renderer,
        });
        let processed = ProcessedView::record(device, source.clone(), graph.processed, graph.plan)?;
        Self::submit_preview(device, queue, submissions, source, native, processed)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn processing_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        source: Arc<ProcessingSource>,
        request: &crate::gpu::photograph::request::PhotographRequest,
        field: Option<Arc<GpuFlatField>>,
        spec: SourceRequest,
    ) -> Result<(Arc<Self>, Arc<ProcessedView>), Error> {
        let spec = Self::proxy_request(&source, spec)?;
        let graph = request.sampled_graph(&device.limits(), &source, field, spec)?;
        let (acquired, native) =
            Self::record_processing(device, renderer, graph.request, graph.baseline)?;
        let processed =
            ProcessedView::record(device, acquired.clone(), graph.processed, graph.plan)?;
        Self::submit_preview(device, queue, submissions, acquired, native, processed)
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub fn from_processing(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        source: Arc<ProcessingSource>,
        input: &crate::gpu::test_inputs::AuthoredPhotograph,
        spec: SourceRequest,
    ) -> Result<Arc<Self>, Error> {
        let spec = Self::proxy_request(&source, spec)?;
        let (spec, definition) =
            crate::gpu::photograph::request::PhotographRequest::Fixture(input.clone())
                .sampled_prefix(&source, input.field.clone(), spec)?;
        let (acquired, native) = Self::record_processing(device, renderer, spec, definition)?;
        let mut encoder = device.create_command_encoder(&Default::default());
        native.encode_from(device, &mut encoder, &source.input, &acquired.pixels)?;
        submissions.submit(queue, [encoder.finish()]);
        native.submitted();
        acquired
            .pixels
            .submitted(&acquired.renderer.evaluation.materializations());
        Ok(acquired)
    }

    fn proxy_request(
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

    #[allow(clippy::too_many_arguments)]
    pub fn process_from_retained(
        self: &Arc<Self>,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        definition: crate::gpu::evaluation::Value,
        root: crate::gpu::evaluation::Value,
        plan: crate::acquisition::Plan,
        previous: Option<&Arc<ProcessedView>>,
    ) -> Result<(Arc<Self>, Arc<ProcessedView>), Error> {
        let region = root
            .region_for(&definition, root.domain().finite()?)?
            .ok_or(Error::InvalidParameter("preview has no neutral dependency"))?;
        if self.supplied()?.covers(&definition, region) {
            let processed =
                ProcessedView::prepare_calculation(device, self.clone(), root, plan, previous)?;
            return Ok((self.clone(), processed));
        }
        let (source, native) =
            Self::record_processing(device, self.renderer.clone(), self.spec.clone(), definition)?;
        let processed = ProcessedView::record(device, source.clone(), root, plan)?;
        Self::submit_preview(device, queue, submissions, source, native, processed)
    }

    fn submit_preview(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        source: Arc<Self>,
        native: crate::gpu::pixel_pipeline::Prepared,
        mut processed: ProcessedView,
    ) -> Result<(Arc<Self>, Arc<ProcessedView>), Error> {
        let mut encoder = device.create_command_encoder(&Default::default());
        native.encode_result(device, &mut encoder, &source.pixels)?;
        processed.encode(device, &mut encoder);
        submissions.submit(queue, [encoder.finish()]);
        native.submitted();
        processed.stages.as_ref().unwrap().submitted();
        let scope = source.renderer.evaluation.materializations();
        source.pixels.submitted(&scope);
        processed.pixels.submitted(&scope);
        processed.stages = None;
        *processed.submitted.get_mut().unwrap() = true;
        Ok((source, Arc::new(processed)))
    }

    fn record_processing(
        device: &wgpu::Device,
        renderer: Arc<crate::renderer::GpuRenderer>,
        spec: SourceRequest,
        root: crate::gpu::evaluation::Value,
    ) -> Result<(Arc<Self>, crate::gpu::pixel_pipeline::Prepared), Error> {
        let raster = crate::gpu::raster::Raster::frame(spec.input_frame()?);
        let prepared = crate::gpu::pixel_pipeline::Prepared::record_composition(
            device,
            raster,
            root.clone(),
            spec.input_region()?,
            renderer.evaluation.clone(),
            device.limits().max_buffer_size.saturating_mul(4),
            Vec::new(),
        )?;
        let pixels = crate::gpu::evaluation::BufferImage::recorded(
            &root,
            spec.input_region()?,
            float_buffer(device, spec.input_frame()?.output)?,
        )?;
        Ok((
            Arc::new(Self {
                spec,
                renderer,
                definition: root,
                pixels,
            }),
            prepared,
        ))
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub async fn acquire_many(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: Vec<SourceRequest>,
        scope: &crate::gpu::evaluation::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        if cancelled() {
            return Ok(None);
        }
        renderer.evaluation.require_device(device)?;
        let identity = crate::gpu::source::identity::Identity::camera(scan.source().identity());
        let specs: Vec<_> = specs
            .into_iter()
            .map(|spec| spec.capture(identity.clone()))
            .collect::<Result<_, _>>()?;
        let spec = specs
            .first()
            .ok_or_else(|| Error::Gpu("empty physical-view acquisition".into()))?;
        scan.validate_area(spec.area)?;
        if scan.area != spec.area
            || (if spec.is_inspection() {
                scan.camera_to_ap1 != spec.matrix()
            } else {
                !spec
                    .program()
                    .is_some_and(|program| program.matches_native(scan).unwrap_or(false))
            })
            || scan.flat_field() != spec.field.as_ref()
            || spec.field.as_ref() != field.as_ref().map(|f| f.field())
            || specs.iter().any(|s| {
                !s.same_source(spec)
                    || s.plan.sampling() != crate::acquisition::Sampling::NeutralFootprints
            })
        {
            return Err(Error::Gpu(
                "physical-view source authority differs from originals".into(),
            ));
        }
        let roots = Self::source_graphs(scan, field, &specs)?;
        let requests = roots
            .iter()
            .cloned()
            .zip(&specs)
            .map(|(root, spec)| {
                let domain = root.domain();
                let region = spec.input_region()?;
                renderer.evaluation.request(
                    root,
                    crate::gpu::evaluation::Region {
                        bounds: region,
                        grid: domain.grid,
                    },
                    crate::gpu::evaluation::Budget {
                        tile: [region.width, region.height],
                        scratch_bytes: 512 << 20,
                    },
                )
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Self::materialize_many(
            queue,
            submissions,
            renderer,
            specs,
            requests,
            scope,
            cancelled,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn materialize_many(
        queue: &wgpu::Queue,
        submissions: &SubmissionGate,
        renderer: Arc<crate::renderer::GpuRenderer>,
        specs: Vec<SourceRequest>,
        requests: Vec<crate::gpu::evaluation::Evaluation>,
        scope: &crate::gpu::evaluation::Materializations,
        cancelled: impl Fn() -> bool,
    ) -> Result<Option<Vec<Arc<Self>>>, Error> {
        if specs.len() != requests.len() {
            return Err(Error::InvalidParameter("batch omitted a requested image"));
        }
        let roots: Vec<_> = requests
            .iter()
            .map(|request| request.image().clone())
            .collect();
        let futures = renderer
            .evaluation
            .materialize_many(queue, submissions, scope, requests)?;
        // Demand retirement drops only these waiters, not independent consumers of shared work.
        let mut pending = Box::pin(futures_util::future::try_join_all(futures));
        let result = futures_util::future::poll_fn(|cx| {
            if cancelled() {
                return std::task::Poll::Ready(None);
            }
            std::future::Future::poll(pending.as_mut(), cx).map(Some)
        })
        .await;
        let Some(result) = result else {
            return Ok(None);
        };
        let images = match result {
            Ok(images) => images,
            Err(error) if matches!(error.as_ref(), Error::Cancelled) => return Ok(None),
            Err(error) => return Err(Error::Gpu(error.to_string())),
        };
        if cancelled() {
            return Ok(None);
        }
        Ok(Some(
            specs
                .into_iter()
                .zip(images)
                .zip(roots)
                .map(|((spec, image), definition)| {
                    Ok(Arc::new(Self {
                        spec,
                        definition,
                        pixels: image.try_into()?,
                        renderer: renderer.clone(),
                    }))
                })
                .collect::<Result<Vec<_>, Error>>()?,
        ))
    }

    #[cfg(test)]
    pub fn source_graphs(
        scan: &Scan,
        field: Option<Arc<GpuFlatField>>,
        specs: &[SourceRequest],
    ) -> Result<Vec<crate::gpu::evaluation::Value>, Error> {
        use crate::gpu::evaluation as e;
        if specs.is_empty() {
            return Err(Error::InvalidParameter("empty physical-view acquisition"));
        }
        let source = e::camera_source(scan)?;
        specs
            .iter()
            .map(|spec| {
                let input = spec.processing_fixture();
                crate::gpu::photograph::NeutralInput {
                    interpretation: input.source_transform().map_or(
                        crate::gpu::interpretation::SourceInterpretation::Inspection(
                            scan.camera_to_ap1,
                        ),
                        |program| {
                            crate::gpu::interpretation::SourceInterpretation::Photograph(*program)
                        },
                    ),
                    field: field.clone(),
                    repairs: input.repairs.clone(),
                    replacements: input.replacements.clone(),
                }
                .original_views(source.clone(), [spec.plan.output()])?
                .pop()
                .ok_or(Error::InvalidParameter("missing requested fixture prefix"))
            })
            .collect()
    }
}
